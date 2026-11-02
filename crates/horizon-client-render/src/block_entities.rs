//! The world's block entities and the order they tick in: vanilla's
//! `Level.blockEntityTickers`, kept beside the block entity map so a tick
//! visits only the block entities that tick, in vanilla's order, instead of
//! scanning the whole map once per kind.
//!
//! The list follows `LevelChunk.updateBlockEntityTicker` and
//! `Level.tickBlockEntities`:
//! - a block entity whose block state has a ticker gets a slot at the end of
//!   the list (through `pendingBlockEntityTickers`, so a slot made during a
//!   tick first ticks on the next one);
//! - replacing the block entity at a position (`setBlockEntity`), or changing
//!   its block state to another state with a ticker, keeps its slot
//!   (`RebindableTickingBlockEntityWrapper.rebind`);
//! - removing it (`removeBlockEntity`), unloading its chunk
//!   (`clearAllBlockEntities`) or changing its state to one without a ticker
//!   (a jukebox losing its record) frees the slot; a later ticker there is
//!   appended again;
//! - a loaded chunk's block entities are added in the iteration order of its
//!   `Object2ObjectOpenHashMap<BlockPos, BlockEntity>`
//!   (`registerAllBlockEntitiesAfterLevelLoad`), filled in the order the chunk
//!   listed them.
//!
//! Which states tick is a table by state id, built by the game from the block
//! registry; whether a slot is made is decided when the slot is promoted at
//! the start of the next tick, from the block state then.
use crate::chunk::block_entity::{BlockEntity, ModContainer, ModEnergyBuffer, ModFluidTanks};
use crate::fast_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;
use std::sync::Arc;

pub type Pos = (i32, i32, i32);

/// What a block state's block entity does each tick (`BlockState.getTicker`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ticking {
    /// No ticker, and the block has no block entity that could have one.
    #[default]
    No,
    /// A ticker.
    Yes,
    /// The block's block entity, but this state has no ticker (a jukebox
    /// without a record): a state change to it frees the slot.
    Idle,
}

#[derive(Default)]
pub struct BlockEntities {
    map: HashMap<Pos, BlockEntity>,
    attachments: HashMap<Pos, horizon_world::attachments::Attachments>,
    /// `Level.blockEntityTickers`: positions and the slot each was given.
    tickers: Arc<Vec<(Pos, u32)>>,
    /// `LevelChunk.tickersInLevel`: the live slot of each ticking position.
    live: FxHashMap<Pos, u32>,
    /// Positions that may need a slot, in the order vanilla would have made
    /// it (`pendingBlockEntityTickers`), each with a token; decided at the
    /// next promotion from the block state then.
    added: Vec<(Pos, u32)>,
    /// The token of each position's pending entry: an entry whose token is
    /// not here any more was cancelled.
    pending: FxHashMap<Pos, u32>,
    next_slot: u32,
    /// Whether a slot was freed since the list was last swept: otherwise
    /// every slot in it is still live, unless its block entity is `out`.
    freed: bool,
    /// Positions whose block entity `take` took and `put` has not brought
    /// back.
    out: FxHashSet<Pos>,
    /// Positions a loaded chunk gave a new block entity over an old one: at
    /// the next tick their slot stays only if their block state ticks
    /// (`updateBlockEntityTicker`: rebind, or remove without a ticker).
    replaced: FxHashSet<Pos>,
    /// `Ticking` by block state id; empty until the game builds it.
    states: Vec<Ticking>,
    /// The containers of mod blocks' block entities, by block state id;
    /// built with `states`.
    mod_containers: FxHashMap<u32, Arc<ModContainer>>,
    /// The fluid tanks of mod blocks' block entities, by block state id;
    /// built with `states`, like the containers.
    mod_fluids: FxHashMap<u32, Arc<ModFluidTanks>>,
    /// The energy buffers of mod blocks' block entities, by block state id;
    /// built with `states`, like the containers.
    mod_energy: FxHashMap<u32, Arc<ModEnergyBuffer>>,
    /// The positions of the mod block entities (`BlockEntity::Mod`) by chunk,
    /// for the renderer: the mesher lists block entities by their template,
    /// which leaves out a mod block whose template has none. One taken out
    /// (`take`) stays listed until `put` or `remove`.
    mods: FxHashMap<(i32, i32), FxHashSet<Pos>>,
    /// The positions of the block entities that draw a bounding box (structure and test
    /// instance blocks, `shows_box`), so a frame looks at them without scanning the map.
    /// Like `mods`, one taken out stays listed until `put` or `remove`.
    boxes: FxHashSet<Pos>,
}

/// A structure or test instance block: the block entities whose box the frame may draw.
fn shows_box(entity: &BlockEntity) -> bool {
    matches!(
        entity,
        BlockEntity::StructureBlock(_) | BlockEntity::TestInstanceBlock(_)
    )
}

fn chunk_of((x, _, z): Pos) -> (i32, i32) {
    (x >> 4, z >> 4)
}

/// Takes `pos` out of its chunk's mod block entity set.
fn forget_mod(mods: &mut FxHashMap<(i32, i32), FxHashSet<Pos>>, pos: Pos) {
    let chunk = chunk_of(pos);
    if let Some(set) = mods.get_mut(&chunk) {
        set.remove(&pos);
        if set.is_empty() {
            mods.remove(&chunk);
        }
    }
}

impl std::ops::Deref for BlockEntities {
    type Target = HashMap<Pos, BlockEntity>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<'a> IntoIterator for &'a BlockEntities {
    type Item = (&'a Pos, &'a BlockEntity);
    type IntoIter = std::collections::hash_map::Iter<'a, Pos, BlockEntity>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

/// `Vec3i.hashCode`.
fn java_hash((x, y, z): Pos) -> i32 {
    y.wrapping_add(z.wrapping_mul(31))
        .wrapping_mul(31)
        .wrapping_add(x)
}
/// `HashCommon.mix`.
fn mix(x: i32) -> usize {
    let h = (x as u32).wrapping_mul(0x9E37_79B9);
    (h ^ (h >> 16)) as usize
}

/// The order `values().forEach` visits a `new Object2ObjectOpenHashMap<>()`
/// keyed by these positions, put in this order (repeats keep their first
/// slot): open addressing with linear probing, a rehash to twice the size
/// past three quarters full, and iteration from the last slot down.
pub fn chunk_map_order(positions: impl IntoIterator<Item = Pos>) -> Vec<Pos> {
    // HashCommon.arraySize(16, .75f), and maxFill of each size.
    let mut slots: Vec<Option<Pos>> = vec![None; 32];
    let max_fill = |n: usize| (n * 3).div_ceil(4).min(n - 1);
    let mut size = 0;
    for p in positions {
        let mask = slots.len() - 1;
        let mut at = mix(java_hash(p)) & mask;
        let mut known = false;
        while let Some(k) = slots[at] {
            if k == p {
                known = true;
                break;
            }
            at = (at + 1) & mask;
        }
        if known {
            continue;
        }
        slots[at] = Some(p);
        // `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`, which
        // puts the keys back from the last slot down.
        size += 1;
        if size > max_fill(slots.len()) {
            let n = ((size + 1) * 4).div_ceil(3).next_power_of_two();
            let old = std::mem::replace(&mut slots, vec![None; n]);
            for k in old.into_iter().rev().flatten() {
                let mut at = mix(java_hash(k)) & (n - 1);
                while slots[at].is_some() {
                    at = (at + 1) & (n - 1);
                }
                slots[at] = Some(k);
            }
        }
    }
    slots.into_iter().rev().flatten().collect()
}

impl BlockEntities {
    /// Whether the ticking table has been built.
    pub fn has_states(&self) -> bool {
        !self.states.is_empty()
    }
    pub fn set_states(&mut self, states: Vec<Ticking>) {
        self.states = states;
    }
    pub fn ticking(&self, state: u32) -> Ticking {
        self.states.get(state as usize).copied().unwrap_or_default()
    }
    pub fn set_mod_containers(&mut self, containers: FxHashMap<u32, Arc<ModContainer>>) {
        self.mod_containers = containers;
    }
    /// The container of a mod block's block entity in block state `state`.
    pub fn mod_container(&self, state: u32) -> Option<&Arc<ModContainer>> {
        self.mod_containers.get(&state)
    }
    pub fn set_mod_fluids(&mut self, fluids: FxHashMap<u32, Arc<ModFluidTanks>>) {
        self.mod_fluids = fluids;
    }
    /// The fluid tanks of a mod block's block entity in block state `state`.
    pub fn mod_fluids(&self, state: u32) -> Option<&Arc<ModFluidTanks>> {
        self.mod_fluids.get(&state)
    }
    pub fn set_mod_energy(&mut self, energy: FxHashMap<u32, Arc<ModEnergyBuffer>>) {
        self.mod_energy = energy;
    }
    /// The energy buffer of a mod block's block entity in block state `state`.
    pub fn mod_energy(&self, state: u32) -> Option<&Arc<ModEnergyBuffer>> {
        self.mod_energy.get(&state)
    }

    fn next(&mut self) -> u32 {
        let n = self.next_slot;
        self.next_slot = n.wrapping_add(1);
        n
    }
    /// A ticker may be made for `pos` now (`updateBlockEntityTicker`).
    fn add(&mut self, pos: Pos) {
        let token = self.next();
        self.pending.insert(pos, token);
        self.added.push((pos, token));
    }
    /// `removeBlockEntityTicker`: no slot, made or pending.
    fn cancel(&mut self, pos: &Pos) {
        self.freed |= self.live.remove(pos).is_some();
        self.pending.remove(pos);
    }

    /// Attachment map for a live block entity.
    pub fn attachments_mut(
        &mut self,
        pos: Pos,
    ) -> Option<&mut horizon_world::attachments::Attachments> {
        if !self.map.contains_key(&pos) && !self.out.contains(&pos) {
            return None;
        }
        Some(self.attachments.entry(pos).or_default())
    }
    /// Attachment maps in a loaded chunk, for a save snapshot.
    pub fn chunk_attachments(
        &self,
        x: i32,
        z: i32,
    ) -> Vec<(Pos, horizon_world::attachments::Attachments)> {
        self.attachments
            .iter()
            .filter(|(p, _)| p.0 >> 4 == x && p.2 >> 4 == z)
            .map(|(p, data)| (*p, data.clone()))
            .collect()
    }
    /// Restore a loaded block entity's attachment map.
    pub fn restore_attachments(&mut self, pos: Pos, data: horizon_world::attachments::Attachments) {
        if self.map.contains_key(&pos) {
            self.attachments.insert(pos, data);
        }
    }
    /// Whether any mod block entity is in the map: until one is, a block change
    /// never looks one up (`LevelAccess::has_mod_block_entity`).
    pub fn has_mods(&self) -> bool {
        !self.mods.is_empty()
    }
    /// The structure and test instance blocks (a frame draws their boxes), in no order.
    pub fn boxes(&self) -> impl Iterator<Item = (&Pos, &BlockEntity)> {
        self.boxes
            .iter()
            .filter_map(|pos| Some((pos, self.map.get(pos)?)))
    }
    /// Keeps `pos` listed in `boxes` exactly while `entity` shows a box.
    fn note_box(&mut self, pos: Pos, entity: Option<&BlockEntity>) {
        if entity.is_some_and(shows_box) {
            self.boxes.insert(pos);
        } else {
            self.boxes.remove(&pos);
        }
    }
    /// The mod block entities within `radius` blocks of `x`, `z` (by chunk, so
    /// far ones are skipped whole; the caller checks each distance).
    pub fn mods_near(&self, x: f64, z: f64, radius: f64) -> impl Iterator<Item = &Pos> {
        let far = move |c: i32, at: f64| {
            let (lo, hi) = (f64::from(c * 16), f64::from(c * 16 + 16));
            at < lo - radius || at > hi + radius
        };
        self.mods
            .iter()
            .filter(move |((cx, cz), _)| !far(*cx, x) && !far(*cz, z))
            .flat_map(|(_, set)| set)
    }
    pub fn get_mut(&mut self, pos: &Pos) -> Option<&mut BlockEntity> {
        self.map.get_mut(pos)
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Pos, &mut BlockEntity)> {
        self.map.iter_mut()
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut BlockEntity> {
        self.map.values_mut()
    }
    /// `LevelChunk.setBlockEntity` (+ `addAndRegisterBlockEntity`): a new
    /// block entity may get a slot; one replacing another keeps its slot.
    /// One of the same type rewrites it in place (a jukebox taking a disc,
    /// `/data merge block`), where vanilla keeps the object, so its
    /// attachments stay; any other replacement starts them over.
    pub fn insert(&mut self, pos: Pos, entity: BlockEntity) -> Option<BlockEntity> {
        let new_is_mod = matches!(entity, BlockEntity::Mod(_));
        let same_type = |old: &BlockEntity| match (old, &entity) {
            (BlockEntity::Mod(old), BlockEntity::Mod(new)) => old.id == new.id,
            (old, new) => std::mem::discriminant(old) == std::mem::discriminant(new),
        };
        if !self.map.get(&pos).is_some_and(same_type) {
            self.attachments.remove(&pos);
        }
        let new_box = shows_box(&entity);
        let old = self.map.insert(pos, entity);
        if old.is_none() {
            self.add(pos);
        }
        if new_box {
            self.boxes.insert(pos);
        } else if old.as_ref().is_some_and(shows_box) {
            self.boxes.remove(&pos);
        }
        if new_is_mod {
            self.mods.entry(chunk_of(pos)).or_default().insert(pos);
        } else if matches!(old, Some(BlockEntity::Mod(_))) {
            forget_mod(&mut self.mods, pos);
        }
        old
    }
    /// The block entity at `pos`, made by `make` when there is none.
    pub fn get_or_insert_with(
        &mut self,
        pos: Pos,
        make: impl FnOnce() -> BlockEntity,
    ) -> &mut BlockEntity {
        let absent = !self.map.contains_key(&pos);
        if absent {
            self.add(pos);
        }
        let entity = self.map.entry(pos).or_insert_with(make);
        if absent && matches!(entity, BlockEntity::Mod(_)) {
            self.mods.entry(chunk_of(pos)).or_default().insert(pos);
        }
        if absent && shows_box(entity) {
            self.boxes.insert(pos);
        }
        entity
    }
    /// `LevelChunk.removeBlockEntity`: the block entity and its slot.
    pub fn remove(&mut self, pos: &Pos) -> Option<BlockEntity> {
        self.attachments.remove(pos);
        self.cancel(pos);
        let removed = self.map.remove(pos);
        if matches!(removed, Some(BlockEntity::Mod(_))) {
            forget_mod(&mut self.mods, *pos);
        }
        self.boxes.remove(pos);
        removed
    }
    /// The block entity at `pos`, out of the map for a while: its slot stays
    /// for `put` to bring it back (a tick working on it).
    pub fn take(&mut self, pos: &Pos) -> Option<BlockEntity> {
        let taken = self.map.remove(pos);
        if taken.is_some() {
            self.out.insert(*pos);
        }
        taken
    }
    /// Brings back what `take` took, in its slot.
    pub fn put(&mut self, pos: Pos, entity: BlockEntity) {
        self.out.remove(&pos);
        if matches!(entity, BlockEntity::Mod(_)) {
            self.mods.entry(chunk_of(pos)).or_default().insert(pos);
        }
        self.note_box(pos, Some(&entity));
        if self.map.insert(pos, entity).is_none()
            && !self.live.contains_key(&pos)
            && !self.pending.contains_key(&pos)
        {
            self.add(pos);
        }
    }
    /// Removes the block entities `keep` refuses, and their slots
    /// (`clearAllBlockEntities` of unloaded chunks).
    pub fn retain(&mut self, mut keep: impl FnMut(&Pos, &mut BlockEntity) -> bool) {
        let (live, pending, freed, attachments) = (
            &mut self.live,
            &mut self.pending,
            &mut self.freed,
            &mut self.attachments,
        );
        let (mods, boxes) = (&mut self.mods, &mut self.boxes);
        self.map.retain(|p, e| {
            let kept = keep(p, e);
            if !kept {
                attachments.remove(p);
                *freed |= live.remove(p).is_some();
                pending.remove(p);
                forget_mod(mods, *p);
                boxes.remove(p);
            }
            kept
        });
    }
    pub fn clear(&mut self) {
        self.attachments.clear();
        self.map.clear();
        self.mods.clear();
        self.boxes.clear();
        self.tickers = Arc::default();
        self.live.clear();
        self.added.clear();
        self.pending.clear();
        self.out.clear();
        self.replaced.clear();
        self.freed = false;
    }
    /// A loaded or generated chunk's block entities, in the order the chunk
    /// lists them; they tick in its block entity map's order
    /// (`registerAllBlockEntitiesAfterLevelLoad`). One replacing an entity
    /// already at its position (`LevelChunk.setBlockEntity`) drops that one;
    /// its slot is decided by the block state at the next tick.
    pub fn load_chunk(&mut self, entities: Vec<(Pos, BlockEntity)>) {
        let order = chunk_map_order(entities.iter().map(|(p, _)| *p));
        for (p, e) in entities {
            self.attachments.remove(&p);
            let new_is_mod = matches!(e, BlockEntity::Mod(_));
            self.note_box(p, Some(&e));
            let old = self.map.insert(p, e);
            if old.is_some() {
                self.replaced.insert(p);
            }
            if new_is_mod {
                self.mods.entry(chunk_of(p)).or_default().insert(p);
            } else if matches!(old, Some(BlockEntity::Mod(_))) {
                forget_mod(&mut self.mods, p);
            }
        }
        for p in order {
            self.add(p);
        }
    }
    /// `LevelChunk.setBlockState` on a block entity's position, from state
    /// `old` to `new`: a ticker made, kept or dropped.
    pub fn state_changed(&mut self, pos: Pos, old: u32, new: u32) {
        if self.states.is_empty() || !self.map.contains_key(&pos) {
            return;
        }
        match self.ticking(new) {
            // A new ticker, unless one was made or is pending (rebound).
            Ticking::Yes => {
                if self.ticking(old) != Ticking::Yes && !self.live.contains_key(&pos) {
                    self.add(pos);
                }
            }
            Ticking::Idle => self.cancel(&pos),
            // Another block: the game removes or replaces the block entity.
            Ticking::No => {}
        }
    }
    /// The start of `Level.tickBlockEntities`: pending slots join the list
    /// and freed ones leave it. `state` reads the block state at a position.
    /// Returns the slots to tick, in order; check each with `is_live` before
    /// ticking it, as the ticks before it can free it.
    pub fn tickers(&mut self, state: impl Fn(Pos) -> u32) -> Arc<Vec<(Pos, u32)>> {
        for (pos, token) in std::mem::take(&mut self.added) {
            if self.pending.get(&pos) != Some(&token) {
                continue;
            }
            self.pending.remove(&pos);
            if self.live.contains_key(&pos)
                || !self.map.contains_key(&pos)
                || self.ticking(state(pos)) != Ticking::Yes
            {
                continue;
            }
            let slot = self.next();
            self.live.insert(pos, slot);
            Arc::make_mut(&mut self.tickers).push((pos, slot));
        }
        for pos in std::mem::take(&mut self.replaced) {
            if self.live.contains_key(&pos) && self.ticking(state(pos)) != Ticking::Yes {
                self.cancel(&pos);
            }
        }
        if !std::mem::take(&mut self.freed) && self.out.is_empty() {
            return Arc::clone(&self.tickers);
        }
        self.out.clear();
        let (live, map) = (&mut self.live, &self.map);
        Arc::make_mut(&mut self.tickers).retain(|(p, slot)| {
            let mine = live.get(p) == Some(slot);
            if mine && !map.contains_key(p) {
                live.remove(p);
            }
            mine && map.contains_key(p)
        });
        Arc::clone(&self.tickers)
    }
    /// Whether a slot from `tickers` still ticks.
    pub fn is_live(&self, (pos, slot): (Pos, u32)) -> bool {
        self.live.get(&pos) == Some(&slot) && self.map.contains_key(&pos)
    }
    /// Frees the slot at `pos` (the block entity stays).
    pub fn forget(&mut self, pos: &Pos) {
        self.cancel(pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hopper() -> BlockEntity {
        BlockEntity::Chest(vec![None; 5])
    }
    /// States: 0 air, 1 ticks, 2 idle.
    fn level() -> BlockEntities {
        let mut b = BlockEntities::default();
        b.set_states(vec![Ticking::No, Ticking::Yes, Ticking::Idle]);
        b
    }
    fn order(b: &mut BlockEntities, states: &HashMap<Pos, u32>) -> Vec<Pos> {
        b.tickers(|p| states.get(&p).copied().unwrap_or(0))
            .iter()
            .map(|(p, _)| *p)
            .collect()
    }

    fn near(b: &BlockEntities, x: f64, z: f64, radius: f64) -> Vec<Pos> {
        let mut found: Vec<Pos> = b.mods_near(x, z, radius).copied().collect();
        found.sort();
        found
    }
    #[test]
    fn mod_block_entities_are_listed_by_chunk_until_they_go() {
        let mut b = level();
        let modded = || BlockEntity::Mod(Box::default());
        b.insert((1, 64, 1), modded());
        b.insert((200, 64, 1), modded());
        b.insert((2, 64, 2), hopper());
        b.get_or_insert_with((3, 64, 3), modded);
        b.load_chunk(vec![((4, 64, 4), modded()), ((5, 64, 5), hopper())]);
        // Only mods, and only chunks within reach.
        assert_eq!(
            near(&b, 0.0, 0.0, 64.0),
            [(1, 64, 1), (3, 64, 3), (4, 64, 4)]
        );
        assert_eq!(near(&b, 200.0, 0.0, 8.0), [(200, 64, 1)]);
        // Taken out for a tick, still listed; removed or replaced, gone.
        let taken = b.take(&(1, 64, 1)).unwrap();
        assert_eq!(near(&b, 0.0, 0.0, 64.0).len(), 3);
        b.put((1, 64, 1), taken);
        b.remove(&(3, 64, 3));
        b.insert((4, 64, 4), hopper());
        assert_eq!(near(&b, 0.0, 0.0, 64.0), [(1, 64, 1)]);
        b.retain(|p, _| p.0 != 1);
        assert!(near(&b, 0.0, 0.0, 64.0).is_empty());
        b.clear();
        assert!(near(&b, 200.0, 0.0, 8.0).is_empty());
    }
    #[test]
    fn attachments_follow_block_entity_lifetime() {
        use horizon_world::attachments::AttachmentType;
        let ty = AttachmentType {
            namespace: "test",
            name: "value",
            default: || 0,
            codec: None,
            copy_on_death: false,
        };
        let p = (1, 64, 2);
        let mut entities = level();
        assert!(entities.attachments_mut(p).is_none());
        entities.insert(p, hopper());
        entities.attachments_mut(p).unwrap().set(&ty, 4).unwrap();
        let held = entities.take(&p).unwrap();
        entities.put(p, held);
        assert_eq!(entities.attachments_mut(p).unwrap().get(&ty), Ok(4));
        entities.insert(p, hopper());
        assert_eq!(entities.attachments_mut(p).unwrap().get(&ty), Ok(4));
        let machine = |id: &str| {
            BlockEntity::Mod(Box::new(
                crate::chunk::block_entity::ModBlockEntityFields {
                    id: id.into(),
                    ..Default::default()
                }
                .into(),
            ))
        };
        entities.insert(p, machine("test:a"));
        assert_eq!(entities.attachments_mut(p).unwrap().get(&ty), Ok(0));
        entities.attachments_mut(p).unwrap().set(&ty, 4).unwrap();
        entities.insert(p, machine("test:a"));
        assert_eq!(entities.attachments_mut(p).unwrap().get(&ty), Ok(4));
        entities.insert(p, machine("test:b"));
        assert_eq!(entities.attachments_mut(p).unwrap().get(&ty), Ok(0));
        entities.attachments_mut(p).unwrap().set(&ty, 4).unwrap();
        entities.remove(&p);
        entities.insert(p, hopper());
        assert_eq!(entities.attachments_mut(p).unwrap().get(&ty), Ok(0));
        entities.attachments_mut(p).unwrap().set(&ty, 5).unwrap();
        entities.retain(|_, _| false);
        entities.insert(p, hopper());
        assert_eq!(entities.attachments_mut(p).unwrap().get(&ty), Ok(0));
    }

    #[test]
    fn slots_follow_vanilla_lifecycle() {
        let mut b = level();
        let mut s = HashMap::new();
        for p in [(0, 0, 0), (1, 0, 0), (2, 0, 0)] {
            s.insert(p, 1);
            b.insert(p, hopper());
        }
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
        // Replacing keeps the slot; removing and placing again appends.
        b.insert((0, 0, 0), hopper());
        b.remove(&(1, 0, 0));
        b.insert((1, 0, 0), hopper());
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (2, 0, 0), (1, 0, 0)]);
        // Take and put keep it; an idle state frees it, ticking again appends.
        let e = b.take(&(0, 0, 0)).unwrap();
        b.put((0, 0, 0), e);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (2, 0, 0), (1, 0, 0)]);
        s.insert((2, 0, 0), 2);
        b.state_changed((2, 0, 0), 1, 2);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0)]);
        s.insert((2, 0, 0), 1);
        b.state_changed((2, 0, 0), 2, 1);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
        // A slot freed during a tick is not live any more.
        let slots = b.tickers(|p| s.get(&p).copied().unwrap_or(0));
        b.retain(|p, _| *p != (1, 0, 0));
        assert!(b.is_live(slots[0]) && !b.is_live(slots[1]));
        // A block entity taken and never brought back loses its slot.
        b.take(&(0, 0, 0));
        assert_eq!(order(&mut b, &s), [(2, 0, 0)]);
        assert_eq!(order(&mut b, &s), [(2, 0, 0)]);
    }

    #[test]
    fn chunk_loads_in_open_hash_map_order() {
        // A few keys: their mixed hashes over 32 slots, read from the top.
        let positions = [
            (0, 64, 0),
            (1, 64, 0),
            (0, 64, 1),
            (15, -64, 15),
            (3, 70, 9),
        ];
        let slot = |p: &Pos| mix(java_hash(*p)) & 31;
        let mut expected = positions.to_vec();
        expected.sort_by_key(|p| std::cmp::Reverse(slot(p)));
        let distinct: std::collections::HashSet<usize> = positions.iter().map(slot).collect();
        assert_eq!(distinct.len(), positions.len());
        assert_eq!(chunk_map_order(positions), expected);
        // Past 24 keys the table doubles; every key stays, once.
        let many: Vec<Pos> = (0..100).map(|i| (i % 16, i / 16, 3)).collect();
        let order = chunk_map_order(many.iter().copied().chain(many.iter().copied()));
        assert_eq!(order.len(), 100);
    }

    /// A chunk loaded over a block entity already there: the new one replaces
    /// it, and keeps the old slot only while its block state ticks.
    #[test]
    fn loaded_chunk_replaces_block_entity_by_state() {
        let mut b = level();
        let mut s = HashMap::new();
        let (p, q) = ((0, 0, 0), (1, 0, 0));
        for pos in [p, q] {
            s.insert(pos, 1);
        }
        // Two chunks, so the order is the order they load in.
        b.load_chunk(vec![(p, hopper())]);
        b.load_chunk(vec![(q, hopper())]);
        assert_eq!(order(&mut b, &s), [p, q]);
        // Replaced where the state ticks: the slot stays in its place.
        b.load_chunk(vec![(p, hopper())]);
        assert_eq!(order(&mut b, &s), [p, q]);
        // Replaced where the state has no ticker: the slot is freed, and the
        // new block entity is the one in the map.
        s.insert(p, 2);
        b.load_chunk(vec![(p, BlockEntity::Chest(vec![None; 27]))]);
        assert_eq!(order(&mut b, &s), [q]);
        assert!(matches!(b.get(&p), Some(BlockEntity::Chest(items)) if items.len() == 27));
        // Ticking again later appends it, as any new ticker does.
        s.insert(p, 1);
        b.state_changed(p, 2, 1);
        assert_eq!(order(&mut b, &s), [q, p]);
    }

    #[test]
    fn boxes_list_exactly_the_structure_and_test_blocks() {
        let test_block = || BlockEntity::TestInstanceBlock(Box::default());
        let listed = |b: &BlockEntities| {
            let mut v: Vec<Pos> = b.boxes().map(|(p, _)| *p).collect();
            v.sort();
            v
        };
        let mut b = level();
        let (p, q, r) = ((0, 0, 0), (1, 0, 0), (2, 0, 0));
        b.insert(p, test_block());
        b.insert(q, hopper());
        assert_eq!(listed(&b), [p]);
        // Replaced by another kind, and back.
        b.insert(p, hopper());
        assert_eq!(listed(&b), []);
        b.insert(q, test_block());
        b.get_or_insert_with(r, test_block);
        assert_eq!(listed(&b), [q, r]);
        // Taken out: not shown until put back; put back as another kind: gone.
        let taken = b.take(&q).unwrap();
        assert_eq!(listed(&b), [r]);
        b.put(q, taken);
        assert_eq!(listed(&b), [q, r]);
        let taken = b.take(&r).unwrap();
        drop(taken);
        b.put(r, hopper());
        assert_eq!(listed(&b), [q]);
        b.remove(&q);
        assert_eq!(listed(&b), []);
        b.load_chunk(vec![(p, test_block()), (q, hopper())]);
        assert_eq!(listed(&b), [p]);
        b.retain(|pos, _| *pos != p);
        assert_eq!(listed(&b), []);
        b.insert(r, test_block());
        b.clear();
        assert!(b.boxes.is_empty());
    }
}
