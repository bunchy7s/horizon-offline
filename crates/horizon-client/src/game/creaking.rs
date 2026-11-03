//! The creaking heart block entity in the game: `mobs::creaking_heart`
//! ticks each heart (spawning, letting go of and bleeding for its creaking)
//! and this applies what it reports to the world, the mobs and the speakers.
use super::{Game, creatures::seen};
use horizon_client::{
    Result,
    chunk_map::{ChunkMap, ChunkMembershipDelta},
    containers::{BlockEntity, CreakingHeart},
    mobs::{
        Mob, MobType,
        creaking_heart::{self, HeartEvent, HeartHost},
    },
    particles::{ParticleOptions, ParticleType},
};
use horizon_core::{
    geometry::Vec3,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::registry::BlockStateId;
use std::{collections::HashMap, sync::Arc};

type Pos = (i32, i32, i32);

/// Comparator input of a heart (`getAnalogOutputSignal`).
pub(super) fn heart_signal(world: &horizon_client::world::World, pos: BlockPos) -> Option<i32> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z))? {
        BlockEntity::CreakingHeart(h) => Some(h.output_signal),
        _ => None,
    }
}

/// The game's mobs as a heart sees them, taken out of the game for the tick.
struct Host<'a> {
    mobs: &'a mut Vec<Mob>,
    added: Vec<Mob>,
    uuids: &'a mut HashMap<u32, [i32; 4]>,
    creaking: Option<Arc<MobType>>,
    next_entity: &'a mut u32,
    spawning: bool,
}
impl HeartHost for Host<'_> {
    fn mob(&self, id: u32) -> Option<&Mob> {
        self.mobs
            .iter()
            .chain(self.added.iter())
            .find(|m| m.id == id && !m.removed)
    }
    fn mob_mut(&mut self, id: u32) -> Option<&mut Mob> {
        self.mobs
            .iter_mut()
            .chain(self.added.iter_mut())
            .find(|m| m.id == id && !m.removed)
    }
    fn mob_for_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        self.uuids
            .iter()
            .find(|(id, u)| **u == uuid && self.mob(**id).is_some())
            .map(|(id, _)| *id)
    }
    fn uuid_for(&mut self, id: u32) -> [i32; 4] {
        *self
            .uuids
            .entry(id)
            .or_insert_with(horizon_client::save::random_uuid)
    }
    fn creaking_type(&self) -> Option<Arc<MobType>> {
        self.creaking.clone()
    }
    fn create_mob(&mut self, kind: Arc<MobType>, at: Vec3) -> Mob {
        *self.next_entity += 1;
        // A new entity's random is its own (Mob::new seeds it).
        let mut m = Mob::new(*self.next_entity, kind, at, 0.0);
        let seed = i64::from(m.id);
        horizon_client::mobs::monster_brain::init(&mut m, seed);
        m
    }
    fn add_mob(&mut self, m: Mob) {
        self.added.push(m);
    }
    fn spawning_monsters(&self) -> bool {
        self.spawning
    }
}

impl Game<'_> {
    fn heart_state_name(&self, state: BlockStateId) -> String {
        self.pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("creaking_heart_state").cloned())
            .unwrap_or_default()
    }
    /// Runs `f` on the heart at `key` with the game's mobs and level random,
    /// then applies what the heart reported. `others`: whether `f` reads the
    /// other mobs through the mob world (without, it holds none: building them
    /// costs a pass over every mob).
    fn with_heart(
        &mut self,
        key: Pos,
        heart: &mut CreakingHeart,
        others: bool,
        f: impl FnOnce(
            &mut CreakingHeart,
            &horizon_client::mobs::MobWorld,
            &mut LegacyRandom,
            &mut Host,
            &mut Vec<HeartEvent>,
        ),
    ) -> Result<()> {
        let mut mobs = std::mem::take(&mut self.mobs);
        let mut uuids = std::mem::take(&mut self.persist.uuids);
        let mut next_entity = self.next_entity;
        let mut random = self.world.random.clone();
        let spawning = self.spawning_monsters();
        let creaking = self.res.mob_types().get("minecraft:creaking").cloned();
        let player_uuid = self.level.player_uuid;
        let snapshot: Vec<_> = if others {
            mobs.iter().map(|m| seen(m, player_uuid)).collect()
        } else {
            Vec::new()
        };
        let mut events = Vec::new();
        let added = {
            let w = self.mob_world_base(&snapshot);
            let mut host = Host {
                mobs: &mut mobs,
                added: Vec::new(),
                uuids: &mut uuids,
                creaking,
                next_entity: &mut next_entity,
                spawning,
            };
            f(heart, &w, &mut random, &mut host, &mut events);
            host.added
        };
        mobs.extend(added);
        self.mobs = mobs;
        self.persist.uuids = uuids;
        self.next_entity = next_entity;
        self.world.random = random;
        let mut mob_events = Vec::new();
        for e in events {
            match e {
                HeartEvent::SetBlock(at, state) => self.level_set(at, state)?,
                HeartEvent::Sound(name, at, volume, pitch) => {
                    self.audio
                        .play(&format!("minecraft:{name}"), Some(at), volume, pitch)
                }
                HeartEvent::Trail {
                    at,
                    target,
                    color,
                    duration,
                } => self.particles.add(
                    &ParticleOptions::trail(target, color, duration),
                    at,
                    [0.0; 3],
                ),
                HeartEvent::OutputChanged(at) => self.container_changed((at[0], at[1], at[2])),
                HeartEvent::Mob(event) => mob_events.push(event),
            }
        }
        self.handle_mob_events(mob_events)?;
        let _ = key;
        Ok(())
    }
    /// A creaking bound to the heart at `home` was hurt (`creakingHurt`).
    pub(super) fn creaking_hurt(&mut self, home: [i32; 3], _id: u32, by_player: bool) {
        if !by_player {
            return;
        }
        let key = (home[0], home[1], home[2]);
        let Some(BlockEntity::CreakingHeart(mut heart)) = self.world.block_entities.take(&key)
        else {
            return;
        };
        let _ = self.with_heart(key, &mut heart, true, |h, w, lr, host, out| {
            creaking_heart::creaking_hurt(h, home, w, lr, host, out);
        });
        self.world
            .block_entities
            .put(key, BlockEntity::CreakingHeart(heart));
    }
    /// `Creaking.tearDown`: the crumble particles.
    pub(super) fn creaking_tear_down(&mut self, center: [f64; 3], spread: [f64; 3]) {
        let registry = self.pack.assets.registry;
        let crumble = |this: &mut Self, name: &str, props: &[(&str, &str)], count: i32| {
            let Some(mut state) = registry
                .default_state(name)
                .and_then(|s| registry.state(s).cloned())
            else {
                return;
            };
            for (k, v) in props {
                state.properties.insert((*k).to_owned(), (*v).to_owned());
            }
            let Ok(id) = registry.resolve(&state) else {
                return;
            };
            let Some((sprite, tint)) = this.particle_sprite(id, center.map(|c| c.floor() as i32))
            else {
                return;
            };
            for _ in 0..count {
                let r = &mut this.world.random;
                let p = [
                    center[0] + r.next_gaussian() * spread[0],
                    center[1] + r.next_gaussian() * spread[1],
                    center[2] + r.next_gaussian() * spread[2],
                ];
                this.particles.add(
                    &ParticleOptions::block(ParticleType::BlockCrumble, sprite.clone(), tint),
                    p,
                    [0.0; 3],
                );
            }
        };
        crumble(self, "minecraft:pale_oak_wood", &[], 100);
        crumble(
            self,
            "minecraft:creaking_heart",
            &[("creaking_heart_state", "awake")],
            10,
        );
    }
    /// `CreakingHeartBlock.playerWillDestroy`: the heart's creaking twitches to
    /// death and a natural heart drops experience. `onExplosionHit` does the
    /// same for the creaking (`by_player` false), without the experience.
    pub(super) fn creaking_heart_broken(&mut self, pos: [i32; 3], by_player: bool) {
        let key = (pos[0], pos[1], pos[2]);
        if !matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::CreakingHeart(_))
        ) {
            return;
        }
        let Some(BlockEntity::CreakingHeart(mut heart)) = self.world.block_entities.take(&key)
        else {
            return;
        };
        let _ = self.with_heart(key, &mut heart, false, |h, _, _, host, out| {
            creaking_heart::remove_protector(h, host, true, out);
        });
        let state = self.world.block(key.0, key.1, key.2);
        let natural = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("natural").cloned())
            .is_some_and(|v| v == "true");
        if by_player && natural && !self.creative && !self.spectator() {
            let xp = 20 + self.world.random.next_int_bound(5).unwrap_or(0) as u32;
            self.spawn_orbs(
                [
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]) + 0.5,
                    f64::from(pos[2]) + 0.5,
                ],
                xp,
            );
        }
        self.world
            .block_entities
            .put(key, BlockEntity::CreakingHeart(heart));
    }
    /// `CreakingHeartBlockEntity.serverTick` of the heart at `key`. Its slot
    /// stays while uprooted, so that a heart whose block went away is still
    /// found here and lets go of its protector.
    pub(super) fn tick_creaking_heart(&mut self, key: Pos) -> Result<()> {
        let Some(BlockEntity::CreakingHeart(mut heart)) = self.world.block_entities.take(&key)
        else {
            return Ok(());
        };
        let state = self.world.block(key.0, key.1, key.2);
        if &*self.res.facts().get(state).name != "minecraft:creaking_heart" {
            // The block is gone: preRemoveSideEffects.
            self.world.block_entities.forget(&key);
            self.with_heart(key, &mut heart, false, |h, _, _, host, out| {
                creaking_heart::remove_protector(h, host, false, out);
            })?;
            return Ok(());
        }
        // getTicker: none once uprooted.
        if self.heart_state_name(state) != "uprooted" {
            let pos = [key.0, key.1, key.2];
            let others = creaking_heart::tick_updates(&heart);
            self.with_heart(key, &mut heart, others, |h, w, lr, host, out| {
                creaking_heart::server_tick(h, pos, w, lr, host, out);
            })?;
        }
        self.world
            .block_entities
            .put(key, BlockEntity::CreakingHeart(heart));
        Ok(())
    }
    /// `ServerLevel.isSpawningMonsters`.
    fn spawning_monsters(&self) -> bool {
        self.ui.difficulty != 0
            && self.ui.rules.bool("spawn_mobs")
            && self.ui.rules.bool("spawn_monsters")
    }
    /// Hearts that came with generated or loaded chunks get their block
    /// entity (`newBlockEntity`) the first time their chunk is seen lit.
    /// Only the chunks that arrived since are looked at: a server holds tens
    /// of thousands, and reading each one every tick cost milliseconds.
    pub(super) fn scan_for_hearts(&mut self) {
        if self
            .pack
            .assets
            .registry
            .default_state("minecraft:creaking_heart")
            .is_none()
        {
            return;
        }
        self.creaking.follow_chunks(&self.world.chunks);
        let mut keys: Vec<(i32, i32)> = self
            .creaking
            .unscanned
            .iter()
            .copied()
            .filter(|k| self.world.chunks.get(k).is_some_and(|c| c.lit))
            .collect();
        keys.sort_unstable();
        for key in keys {
            self.creaking.unscanned.remove(&key);
            self.creaking.scanned.insert(key);
            let Some(chunk) = self.world.chunks.get(&key).cloned() else {
                continue;
            };
            if self.creaking.hearts.is_empty() {
                self.creaking.hearts = self
                    .res
                    .facts()
                    .states
                    .iter()
                    .map(|f| &*f.name == "minecraft:creaking_heart")
                    .collect();
            }
            let hearts = &self.creaking.hearts;
            let is_heart = |id: u32| hearts.get(id as usize).copied().unwrap_or(false);
            for (si, section) in chunk.sections.iter().enumerate() {
                // Only a section whose palette holds a heart state can have
                // one: the others are skipped without reading their 4,096 blocks.
                if !section.blocks.maybe_has(is_heart) {
                    continue;
                }
                for i in 0..4096 {
                    let id = section.get(i);
                    if !is_heart(id) {
                        continue;
                    }
                    let (lx, lz, ly) = ((i & 15) as i32, ((i >> 4) & 15) as i32, (i >> 8) as i32);
                    let pos = (
                        key.0 * 16 + lx,
                        self.world.bottom_y() + si as i32 * 16 + ly,
                        key.1 * 16 + lz,
                    );
                    self.world
                        .block_entities
                        .get_or_insert_with(pos, || BlockEntity::CreakingHeart(Box::default()));
                }
            }
        }
    }
}

/// What the creaking code remembers between ticks.
#[derive(Default)]
pub(super) struct CreakingState {
    /// Chunks already scanned for heart blocks.
    scanned: horizon_client::fast_hash::FxHashSet<(i32, i32)>,
    /// Resident chunks not scanned yet (not lit yet, or just arrived).
    unscanned: horizon_client::fast_hash::FxHashSet<(i32, i32)>,
    /// The chunk map's membership revision the two sets follow.
    revision: Option<u64>,
    /// Which block states are creaking hearts, by state id.
    hearts: Vec<bool>,
}

impl CreakingState {
    /// Exchange the chunks scanned for creaking hearts with another level's
    /// (game/levels.rs).
    pub(super) fn swap_level(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.scanned, &mut other.scanned);
        std::mem::swap(&mut self.unscanned, &mut other.unscanned);
        std::mem::swap(&mut self.revision, &mut other.revision);
    }
    /// Bring `scanned` and `unscanned` up to the chunks resident now: a
    /// chunk that left is forgotten (it is scanned again if it comes back
    /// later), one that arrived waits to be scanned. From the map's
    /// membership changes, or from all its keys when they can't be replayed.
    fn follow_chunks(&mut self, chunks: &ChunkMap) {
        let revision = chunks.membership_revision();
        if self.revision == Some(revision) {
            return;
        }
        let deltas = self.revision.and_then(|r| chunks.deltas_since(r));
        match deltas {
            Some(deltas) => {
                for (_, delta) in deltas {
                    match delta {
                        ChunkMembershipDelta::Added(key) => {
                            if chunks.contains_key(&key) && !self.scanned.contains(&key) {
                                self.unscanned.insert(key);
                            }
                        }
                        ChunkMembershipDelta::Removed(key) => {
                            if !chunks.contains_key(&key) {
                                self.scanned.remove(&key);
                                self.unscanned.remove(&key);
                            }
                        }
                        ChunkMembershipDelta::Reset => {}
                    }
                }
            }
            None => {
                self.scanned.retain(|k| chunks.contains_key(k));
                self.unscanned = chunks
                    .keys()
                    .filter(|k| !self.scanned.contains(k))
                    .copied()
                    .collect();
            }
        }
        self.revision = Some(revision);
    }
}

#[cfg(test)]
mod tests {
    use super::CreakingState;
    use horizon_client::{chunk::Chunk, chunk_map::ChunkMap};
    use std::sync::Arc;

    fn add(chunks: &mut ChunkMap, key: (i32, i32)) {
        chunks.insert(key, Arc::new(Chunk::empty(key.0, key.1)));
    }
    fn sorted(set: &horizon_client::fast_hash::FxHashSet<(i32, i32)>) -> Vec<(i32, i32)> {
        let mut keys: Vec<_> = set.iter().copied().collect();
        keys.sort_unstable();
        keys
    }

    /// The heart scan follows the chunks that come and go, as the scan of
    /// every resident chunk each tick did: an arrival waits, a scanned chunk
    /// that stayed is not scanned again, and one that left is forgotten.
    #[test]
    fn the_heart_scan_follows_arrivals_and_departures() {
        let mut chunks = ChunkMap::default();
        let mut state = CreakingState::default();
        add(&mut chunks, (0, 0));
        add(&mut chunks, (1, 0));
        state.follow_chunks(&chunks);
        assert_eq!(sorted(&state.unscanned), [(0, 0), (1, 0)]);
        // Scanned, as scan_for_hearts does once a chunk is lit.
        state.unscanned.remove(&(0, 0));
        state.scanned.insert((0, 0));
        add(&mut chunks, (2, 0));
        chunks.remove(&(1, 0));
        // Gone and back between two ticks: still scanned, as before.
        chunks.remove(&(0, 0));
        add(&mut chunks, (0, 0));
        state.follow_chunks(&chunks);
        assert_eq!(sorted(&state.scanned), [(0, 0)]);
        assert_eq!(sorted(&state.unscanned), [(2, 0)]);
        // Gone over a tick: forgotten, and scanned again when it returns.
        chunks.remove(&(0, 0));
        state.follow_chunks(&chunks);
        assert!(state.scanned.is_empty());
        add(&mut chunks, (0, 0));
        state.follow_chunks(&chunks);
        assert_eq!(sorted(&state.unscanned), [(0, 0), (2, 0)]);
        // A cleared map can't be replayed: rebuilt from its keys.
        chunks.clear();
        add(&mut chunks, (5, 5));
        state.follow_chunks(&chunks);
        assert_eq!(sorted(&state.unscanned), [(5, 5)]);
        assert!(state.scanned.is_empty());
    }
}
