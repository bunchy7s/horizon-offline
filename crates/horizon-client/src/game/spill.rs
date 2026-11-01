//! Block entities removed by the block behaviour (`LevelChunk.setBlockState`
//! running `BlockEntity.preRemoveSideEffects`) and the resources a destroyed
//! block drops (`Block.dropResources`), drawn from the level random at the
//! moment vanilla draws them. The item entities they make are added to the
//! world as soon as the behaviour returns, within the same tick.
use super::{Game, GameLevel, Screen, dispenser::new_item_motion, storage::is_shulker_box};
use horizon_client::{
    block_entity_item, block_entity_nbt,
    containers::{Banner, BlockEntity},
    furnace::FurnaceCooking,
    item_data,
    items::ItemStack,
    save,
};
use horizon_core::position::BlockPos;
use horizon_core::random::RandomSource;
use horizon_world::{
    level::{
        LevelEvent,
        removal::{self, PreRemove},
    },
    nbt::{Compound, JavaString, Tag},
    registry::BlockStateId,
};
use std::collections::HashMap;

/// What a broken block's entity gives its own dropped item (`copy_components`
/// in the block loot tables): a shulker box's contents, a banner's layers, a
/// container's custom name.
#[derive(Default)]
pub(super) struct Pending {
    pub(super) shulker: HashMap<[i32; 3], Vec<(u8, ItemStack)>>,
    pub(super) banner: HashMap<[i32; 3], Banner>,
    /// The block's item and the custom name.
    pub(super) names: HashMap<[i32; 3], (String, String)>,
    /// A decorated pot's sherds (back, left, right, front).
    pub(super) pot: HashMap<[i32; 3], [Option<ItemStack>; 4]>,
    /// A head's profile, note block sound and name, or a hive's bees: the
    /// dropped item and the component patch it takes.
    pub(super) copied: HashMap<[i32; 3], (String, Compound)>,
}
impl Pending {
    /// Remember what the block entity at `pos` copies into the drops of `block`.
    pub(super) fn record(
        &mut self,
        pos: [i32; 3],
        entity: &BlockEntity,
        block: &str,
        name: Option<String>,
    ) {
        if let BlockEntity::Banner(banner) = entity {
            self.banner.insert(pos, banner.clone());
            return;
        }
        if let Some(name) = name {
            self.names.insert(pos, (block.to_owned(), name));
        }
        // `copy_components` from the block entity: the head tables copy the
        // profile (player heads), note block sound and name, the hive tables
        // the bees.
        let include: &[&str] = match entity {
            BlockEntity::Skull(_) if block.starts_with("minecraft:player_") => &[
                "minecraft:profile",
                "minecraft:note_block_sound",
                "minecraft:custom_name",
            ],
            BlockEntity::Skull(_) => &["minecraft:custom_name"],
            BlockEntity::Beehive(_) => &["minecraft:bees"],
            _ => &[],
        };
        if !include.is_empty() {
            let saved = save::encode_block_entity((pos[0], pos[1], pos[2]), entity, block);
            let mut patch = block_entity_nbt::canonical(&saved)
                .map(|saved| block_entity_item::pick(&saved))
                .unwrap_or_default();
            patch.retain(|k, _| include.contains(&k.to_string_lossy().as_str()));
            if !patch.is_empty() {
                // `Blocks.wallVariant`: a wall head drops the standing item.
                let item = block.replacen("_wall_", "_", 1);
                self.copied.insert(pos, (item, patch));
            }
        }
        if is_shulker_box(block)
            && let BlockEntity::Chest(items) = entity
        {
            let kept = items
                .iter()
                .enumerate()
                .filter_map(|(i, s)| Some((i as u8, s.clone()?)))
                .collect();
            self.shulker.insert(pos, kept);
        }
    }
    /// Copy what was remembered for `pos` into a drop of the block.
    pub(super) fn attach(&mut self, pos: [i32; 3], stack: &mut ItemStack) {
        if is_shulker_box(&stack.item)
            && let Some(contents) = self.shulker.remove(&pos)
            && !contents.is_empty()
        {
            stack.data_mut().container = contents;
        }
        if self
            .names
            .get(&pos)
            .is_some_and(|(item, _)| **item == *stack.item)
            && let Some((_, name)) = self.names.remove(&pos)
        {
            stack.data_mut().custom_name =
                Some(horizon_world::nbt::Tag::String(name.as_str().into()));
        }
        if self
            .copied
            .get(&pos)
            .is_some_and(|(item, _)| **item == *stack.item)
            && let Some((_, copied)) = self.copied.remove(&pos)
        {
            let mut patch = match item_data::encode_components(stack) {
                Some(Tag::Compound(c)) => c,
                _ => Default::default(),
            };
            patch.extend(copied);
            (stack.damage, stack.extra) = item_data::decode_components(Some(&Tag::Compound(patch)));
        }
        if &*stack.item == "minecraft:decorated_pot" {
            // The pot's block entity always copies its decorations, empty or not.
            let sides = self.pot.remove(&pos).unwrap_or_default();
            stack.data_mut().pot_decorations = Some(Box::new(sides));
        }
        if stack.item.ends_with("_banner")
            && let Some(Banner {
                layers,
                name,
                components,
            }) = self.banner.remove(&pos)
        {
            let mut patch = match item_data::encode_components(stack) {
                Some(Tag::Compound(c)) => c,
                _ => Default::default(),
            };
            // The remaining components in the banner loot table's copy_components.
            for key in [
                "minecraft:item_name",
                "minecraft:rarity",
                "minecraft:tooltip_display",
            ] {
                if let Some(value) = components.get(&JavaString::from(key)) {
                    patch.insert(key.into(), value.clone());
                }
            }
            patch.insert(
                "minecraft:banner_patterns".into(),
                horizon_client::banner::layers_to_nbt(&layers),
            );
            if let Some(name) = name {
                patch.insert(
                    "minecraft:custom_name".into(),
                    Tag::String(name.as_str().into()),
                );
            }
            (stack.damage, stack.extra) = item_data::decode_components(Some(&Tag::Compound(patch)));
        }
    }
    /// Remember a decorated pot's sherds from its saved block entity.
    pub(super) fn record_saved(&mut self, pos: [i32; 3], tag: &horizon_world::nbt::Tag) {
        use horizon_world::nbt::Tag;
        if tag.get("id") != Some(&Tag::String("minecraft:decorated_pot".into())) {
            return;
        }
        let sherds = tag.get("sherds");
        let side = |name: &str| match sherds.and_then(|s| s.get(name)) {
            Some(Tag::String(id)) => Some(ItemStack {
                item: std::sync::Arc::from(id.to_string_lossy().as_str()),
                count: 1,
                damage: 0,
                extra: None,
            }),
            Some(t) => horizon_client::save::item_from_nbt(t),
            None => None,
        };
        self.pot.insert(
            pos,
            [side("back"), side("left"), side("right"), side("front")],
        );
    }
    /// The block entity's dynamic drops: a decorated pot's sherds, left,
    /// back, front then right (`DecoratedPotBlock.getDrops`).
    pub(super) fn dynamic(&self, pos: [i32; 3]) -> Vec<ItemStack> {
        self.pot
            .get(&pos)
            .map_or_else(Vec::new, |[back, left, right, front]| {
                [left, back, front, right]
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect()
            })
    }
    pub(super) fn clear(&mut self, pos: [i32; 3]) {
        self.pot.remove(&pos);
        self.shulker.remove(&pos);
        self.banner.remove(&pos);
        self.names.remove(&pos);
        self.copied.remove(&pos);
    }
}

/// An item entity to add: where, its motion (None: `new ItemEntity`'s own
/// random motion) and its pickup delay.
pub(super) struct Spawn {
    pub(super) stack: ItemStack,
    pub(super) pos: [f64; 3],
    pub(super) motion: Option<[f64; 3]>,
    pub(super) delay: u32,
}

#[derive(Default)]
pub(super) struct Spill {
    pub(super) spawns: Vec<Spawn>,
    /// Experience orbs (`ExperienceOrb.award`): where and how much, each
    /// and the merge id it drew from the level random.
    pub(super) orbs: Vec<([f64; 3], u32, i32)>,
    /// Block entities the behaviour removed, for the game's own follow-ups
    /// (open screens, playing songs).
    pub(super) removed: Vec<([i32; 3], BlockEntity)>,
    /// Runs of `spawns` a player's use popped (a picked berry bush), for the
    /// mods' `native_loot_drops`.
    pub(super) used: Vec<UsedDrops>,
}

/// `spawns[start..end]` came from `table` at the block `pos` in `state`.
pub(super) struct UsedDrops {
    start: usize,
    end: usize,
    pos: [i32; 3],
    state: BlockStateId,
    table: &'static str,
}

impl Spill {
    pub(super) fn is_empty(&self) -> bool {
        self.spawns.is_empty()
            && self.orbs.is_empty()
            && self.removed.is_empty()
            && self.used.is_empty()
    }
}

/// Lets `say` change each use's run of `spawns`, newest first so the earlier
/// ranges hold. A run's stacks keep their spots, and a stack `say` adds takes
/// the run's last spot, else the block's top.
fn retell_runs(
    spawns: &mut Vec<Spawn>,
    used: &[UsedDrops],
    mut say: impl FnMut(&UsedDrops, [f64; 3], &mut Vec<ItemStack>),
) {
    for used in used.iter().rev() {
        let run: Vec<Spawn> = spawns.drain(used.start..used.end).collect();
        let mut drops: Vec<ItemStack> = run.iter().map(|s| s.stack.clone()).collect();
        let origin = used.pos.map(|c| f64::from(c) + 0.5);
        say(used, origin, &mut drops);
        let top = [origin[0], origin[1] + 0.5, origin[2]];
        let kept: Vec<Spawn> = drops
            .into_iter()
            .enumerate()
            .map(|(i, stack)| Spawn {
                stack,
                pos: run.get(i).or(run.last()).map_or(top, |s| s.pos),
                motion: run.get(i).and_then(|s| s.motion),
                delay: run.get(i).map_or(10, |s| s.delay),
            })
            .collect();
        spawns.splice(used.start..used.start, kept);
    }
}

/// `Block.popResource`: a spot around the block's centre from the level random.
pub(super) fn pop_position(
    random: &mut impl horizon_core::random::RandomSource,
    pos: [i32; 3],
) -> [f64; 3] {
    let mut offset = || -0.25 + random.next_double() * 0.5;
    // EntityTypes.ITEM is 0.25 high.
    let half_height = f64::from(0.25f32) / 2.0;
    [
        f64::from(pos[0]) + 0.5 + offset(),
        f64::from(pos[1]) + 0.5 + offset() - half_height,
        f64::from(pos[2]) + 0.5 + offset(),
    ]
}

/// The block entity at `pos` Horizon keeps as it was loaded, if any
/// (campfires, decorated pots, chiseled bookshelves, shelves).
pub(super) fn saved_entity(
    world: &horizon_client::world::World,
    pos: [i32; 3],
) -> Option<horizon_world::nbt::Tag> {
    use horizon_world::nbt::Tag;
    let saved = world
        .chunks
        .get(&(pos[0] >> 4, pos[2] >> 4))?
        .saved
        .as_ref()?;
    saved
        .block_entities
        .iter()
        .find(|t| {
            t.get("x") == Some(&Tag::Int(pos[0]))
                && t.get("y") == Some(&Tag::Int(pos[1]))
                && t.get("z") == Some(&Tag::Int(pos[2]))
        })
        .cloned()
}

/// Keep `tag` as the block entity at its position Horizon does not model,
/// replacing the one there.
pub(super) fn put_saved_entity(
    world: &mut horizon_client::world::World,
    pos: [i32; 3],
    tag: horizon_world::nbt::Tag,
) {
    use horizon_world::nbt::Tag;
    let Some(chunk) = world.chunks.get_mut(&(pos[0] >> 4, pos[2] >> 4)) else {
        return;
    };
    let chunk = std::sync::Arc::make_mut(chunk);
    let saved = std::sync::Arc::make_mut(chunk.saved.get_or_insert_default());
    let at = |t: &Tag| {
        t.get("x") == Some(&Tag::Int(pos[0]))
            && t.get("y") == Some(&Tag::Int(pos[1]))
            && t.get("z") == Some(&Tag::Int(pos[2]))
    };
    match saved.block_entities.iter_mut().find(|t| at(t)) {
        Some(old) => *old = tag,
        None => saved.block_entities.push(tag),
    }
}

impl GameLevel<'_> {
    /// `Block.popResource(level, pos, stack)` with the broken block's copied components.
    fn pop_resource(&mut self, pos: [i32; 3], mut stack: ItemStack) {
        self.pending.attach(pos, &mut stack);
        let at = pop_position(&mut self.world.random, pos);
        self.spill.spawns.push(Spawn {
            stack,
            pos: at,
            motion: None,
            delay: 10,
        });
    }
    /// `Block.popResource(level, pos, new ItemStack(block))`.
    pub(super) fn pop_block_item(&mut self, pos: BlockPos, state: BlockStateId) {
        let item = self.facts.get(state).item.clone();
        if &*item != "minecraft:air" {
            self.pop_resource([pos.x, pos.y, pos.z], ItemStack::new(&item, 1));
        }
    }
    /// An item `atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5).offsetRandomXZ(random,
    /// 0.7)` with the default pickup delay (`ComposterBlock.extractProduce`).
    pub(super) fn pop_above(&mut self, pos: BlockPos, item: std::sync::Arc<str>) {
        let at = removal::jukebox_pop_position(&mut self.world.random, pos);
        self.spill.spawns.push(Spawn {
            stack: ItemStack::new(&item, 1),
            pos: at,
            motion: None,
            delay: 10,
        });
    }
    /// `Block.dropFromBlockInteractLootTable` with `Block.popResource` of
    /// every stack.
    pub(super) fn drop_interact_loot(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        table: &'static str,
    ) {
        let drops = self.items.interact_loot(
            table,
            self.registry,
            state,
            &mut self.world.random,
            self.rand_sequences,
            self.seed,
        );
        let start = self.spill.spawns.len();
        for stack in drops {
            if &*stack.item != "minecraft:air" && stack.count > 0 {
                self.pop_resource([pos.x, pos.y, pos.z], stack);
            }
        }
        self.spill.used.push(UsedDrops {
            start,
            end: self.spill.spawns.len(),
            pos: [pos.x, pos.y, pos.z],
            state,
            table,
        });
    }
    /// `Block.dropResources(state, level, pos, blockEntity)`: the block's loot
    /// (from its random sequence, else the level random), with what its block
    /// entity copies into it, popped at once.
    pub(super) fn drop_resources_now(&mut self, pos: BlockPos, state: BlockStateId) {
        let key = (pos.x, pos.y, pos.z);
        let at = [pos.x, pos.y, pos.z];
        if let Some(entity) = self.world.block_entities.get(&key) {
            let entity = entity.clone();
            let name = self.world.block_names.get(&key).cloned();
            let block = self.facts.get(state).name.clone();
            self.pending.record(at, &entity, &block, name);
        } else if let Some(tag) = saved_entity(self.world, [pos.x, pos.y, pos.z]) {
            self.pending.record_saved(at, &tag);
        }
        let dynamic = self.pending.dynamic(at);
        let drops = match self.items.block_loot_sequence(self.registry, state) {
            Some(sequence) => {
                let random = self.rand_sequences.get(&sequence, self.seed);
                self.items
                    .block_drops_with(self.registry, state, None, &dynamic, random)
            }
            None => self.items.block_drops_with(
                self.registry,
                state,
                None,
                &dynamic,
                &mut self.world.random,
            ),
        };
        for stack in drops {
            if &*stack.item != "minecraft:air" && stack.count > 0 {
                self.pop_resource(at, stack);
            }
        }
        self.pending.clear(at);
        // BlockBehaviour.spawnAfterBreak with no tool: ores, sculk and spawners
        // award experience at the block's centre.
        let name = self.facts.get(state).name.clone();
        if let Some(xp) = removal::break_experience(&mut self.world.random, &name)
            && self.rules.block_drops
        {
            let centre = [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 0.5,
                f64::from(pos.z) + 0.5,
            ];
            self.award_orbs(centre, xp);
        }
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes, each drawing the id
    /// `tryMergeToExisting` compares from the level random.
    pub(super) fn award_orbs(&mut self, at: [f64; 3], mut amount: u32) {
        while amount > 0 {
            let value = removal::orb_value(amount);
            amount -= value;
            let merge_id = self.world.random.next_int_bound(40).unwrap_or(0);
            self.spill.orbs.push((at, value, merge_id));
        }
    }
    /// `BlockEntity.preRemoveSideEffects` and `removeBlockEntity` for the block
    /// entity of `old` at `pos`. A block entity Horizon never created reads as
    /// vanilla's fresh one: an empty container still draws a spot per slot.
    pub(super) fn remove_entity(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        side_effects: bool,
    ) -> u8 {
        let key = (pos.x, pos.y, pos.z);
        if self.remove_saved_entity(pos, old, side_effects) {
            return 0;
        }
        // Sculk blocks and creaking hearts notice their removal on their own tick.
        if matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Sculk(_) | BlockEntity::CreakingHeart(_))
        ) {
            return 0;
        }
        let entity = self.world.block_entities.remove(&key);
        self.world.block_names.remove(&key);
        self.world.forget_block_entity_source(key);
        let name = self.facts.get(old).name.clone();
        let kind = match &entity {
            // BaseContainerBlockEntity.preRemoveSideEffects: Containers.dropContents.
            Some(BlockEntity::Mod(m))
                if !m.items.is_empty()
                    && self.world.block_entities.mod_container(old.0).is_some() =>
            {
                PreRemove::Contents {
                    slots: m.items.len(),
                    furnace: false,
                }
            }
            _ => removal::pre_remove(&name),
        };
        let mut neighbour_updates = 0;
        if side_effects {
            match kind {
                PreRemove::Contents { slots, furnace } => {
                    let items = entity.as_ref().map_or(&[][..], BlockEntity::slots);
                    for slot in 0..slots {
                        let stack = items.get(slot).cloned().flatten();
                        self.drop_item_stack(pos, stack);
                    }
                    // AbstractFurnaceBlockEntity.preRemoveSideEffects: then the
                    // experience of what it made, at the block's centre.
                    if furnace && let Some(BlockEntity::Furnace(f)) = &entity {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        for (count, experience) in f.recipe_experience(self.recipes) {
                            let xp = horizon_client::containers::create_experience(
                                count,
                                experience,
                                &mut self.world.random,
                            );
                            self.award_orbs(at, xp);
                        }
                    }
                }
                PreRemove::Campfire => {
                    let items = match &entity {
                        Some(BlockEntity::Campfire(c)) => c.items.clone(),
                        _ => Default::default(),
                    };
                    for stack in items {
                        self.drop_item_stack(pos, stack);
                    }
                }
                PreRemove::Jukebox => {
                    // popOutTheItem: removeTheItem stops a playing song (its
                    // events and a neighbour update), the record pops, and
                    // onSongChanged updates the neighbours again.
                    if let Some(BlockEntity::Jukebox(j)) = &entity
                        && let Some(record) = j.record().cloned()
                    {
                        if j.playing {
                            self.jukebox_stopped(pos, old);
                            neighbour_updates += 1;
                        }
                        let at = removal::jukebox_pop_position(&mut self.world.random, pos);
                        self.spill.spawns.push(Spawn {
                            stack: record,
                            pos: at,
                            motion: None,
                            delay: 10,
                        });
                        neighbour_updates += 1;
                    }
                }
                PreRemove::Lectern => {
                    let props = self.registry.state(old).map(|s| &s.properties);
                    let has_book = props
                        .and_then(|p| p.get("has_book"))
                        .is_some_and(|v| v == "true");
                    let step = match props.and_then(|p| p.get("facing")).map(String::as_str) {
                        Some("north") => [0, -1],
                        Some("south") => [0, 1],
                        Some("west") => [-1, 0],
                        _ => [1, 0],
                    };
                    if has_book
                        && let Some(BlockEntity::Lectern(l)) = &entity
                        && let Some(book) = l.book.clone()
                    {
                        self.spill.spawns.push(Spawn {
                            stack: book,
                            pos: removal::lectern_book_position(pos, step),
                            motion: None,
                            delay: 10,
                        });
                    }
                }
                PreRemove::ShulkerBox
                | PreRemove::SculkShrieker
                | PreRemove::CreakingHeart
                | PreRemove::Beacon
                | PreRemove::Nothing => {}
            }
        }
        // BlockEntity.setRemoved: a jukebox always reports its song stopping,
        // a beacon plays its deactivation.
        match kind {
            PreRemove::Jukebox => self.jukebox_stopped(pos, old),
            PreRemove::Beacon => self.events.push(LevelEvent::Sound {
                pos,
                sound: "minecraft:block.beacon.deactivate".into(),
                volume: 1.0,
                pitch: 1.0,
            }),
            _ => {}
        }
        if let Some(entity) = entity {
            self.spill.removed.push(([pos.x, pos.y, pos.z], entity));
        }
        neighbour_updates
    }
    /// The jukebox's `JUKEBOX_STOP_PLAY` game event and level event 1011.
    fn jukebox_stopped(&mut self, pos: BlockPos, state: BlockStateId) {
        self.events.push(LevelEvent::GameEvent {
            pos,
            event: "minecraft:jukebox_stop_play",
            state: Some(state),
        });
        self.events.push(LevelEvent::LevelEvent {
            id: 1011,
            pos,
            data: 0,
        });
    }
    /// A block entity Horizon keeps as it was loaded (campfires, decorated
    /// pots, chiseled bookshelves, shelves): its items spill as vanilla's do.
    fn remove_saved_entity(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        side_effects: bool,
    ) -> bool {
        use horizon_world::nbt::Tag;
        let chunk_key = (pos.x >> 4, pos.z >> 4);
        let at = |t: &Tag| {
            let int = |k: &str| match t.get(k) {
                Some(Tag::Int(v)) => Some(*v),
                _ => None,
            };
            int("x") == Some(pos.x) && int("y") == Some(pos.y) && int("z") == Some(pos.z)
        };
        let Some(chunk) = self.world.chunks.get_mut(&chunk_key) else {
            return false;
        };
        let Some(saved) = chunk.saved.as_ref() else {
            return false;
        };
        let Some(index) = saved.block_entities.iter().position(at) else {
            return false;
        };
        let chunk = std::sync::Arc::make_mut(chunk);
        let Some(saved) = chunk.saved.as_mut() else {
            return false;
        };
        let tag = std::sync::Arc::make_mut(saved).block_entities.remove(index);
        if !side_effects {
            return true;
        }
        let name = self.facts.get(old).name.clone();
        let items = match removal::pre_remove(&name) {
            PreRemove::Contents { slots, .. } if &*name == "minecraft:decorated_pot" => {
                // Containers.dropContents reads getTheItem, which unpacks the loot table.
                let mut tag = tag;
                super::decorated_pot::unpack_loot(
                    &mut tag,
                    self.loot.as_deref(),
                    &mut self.world.random,
                    self.rand_sequences,
                    self.seed,
                );
                let mut items = vec![None; slots];
                items[0] = tag
                    .get("item")
                    .and_then(horizon_client::save::item_from_nbt);
                items
            }
            PreRemove::Contents { slots, .. } => {
                horizon_client::save::decode_items(tag.get("Items"), slots)
            }
            PreRemove::Campfire => horizon_client::save::decode_items(tag.get("Items"), 4),
            _ => return true,
        };
        for stack in items {
            self.drop_item_stack(pos, stack);
        }
        true
    }
    /// `Containers.dropContents` for a mod block entity that left the world
    /// during its own tick (the block broke itself): what it held spills as
    /// vanilla's `preRemoveSideEffects` would, instead of vanishing with it.
    pub(super) fn drop_lost_contents(&mut self, pos: BlockPos, items: Vec<Option<ItemStack>>) {
        for stack in items {
            self.drop_item_stack(pos, stack);
        }
    }
    /// `Containers.dropItemStack` of one slot (an empty one still draws its spot).
    fn drop_item_stack(&mut self, pos: BlockPos, stack: Option<ItemStack>) {
        let count = stack.as_ref().map_or(0, |s| s.count);
        for piece in removal::drop_item_stack(&mut self.world.random, pos, count) {
            let Some(stack) = &stack else { break };
            self.spill.spawns.push(Spawn {
                stack: ItemStack {
                    count: piece.count,
                    ..stack.clone()
                },
                pos: piece.pos,
                motion: Some(piece.motion),
                delay: 0,
            });
        }
    }
}

impl Game<'_> {
    /// Add the item entities the block behaviour spawned and follow up on the
    /// block entities it removed.
    pub(super) fn flush_spill(&mut self) {
        if self.spill.is_empty() {
            return;
        }
        let mut spill = std::mem::take(&mut self.spill);
        let used = std::mem::take(&mut spill.used);
        let tool = self.held().cloned();
        retell_runs(&mut spill.spawns, &used, |used, origin, drops| {
            let block = Some((used.pos, used.state));
            self.native_use_drops(used.table, origin, block, None, tool.as_ref(), drops);
        });
        for s in spill.spawns {
            let motion = s.motion.unwrap_or_else(new_item_motion);
            self.spawn_item(s.stack, s.pos, motion, s.delay);
        }
        for (at, value, merge_id) in spill.orbs {
            self.award_orb(at, value, merge_id);
        }
        for (pos, entity) in spill.removed {
            let key = (pos[0], pos[1], pos[2]);
            if matches!(entity, BlockEntity::Jukebox(_)) {
                self.jukebox_removed(pos);
            }
            if self.container == Some(key) {
                self.container_opener_event(key, false);
                self.container = None;
                self.screen = Screen::Playing;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{block_entity_item, item_data, mobs::raider, save};
    use horizon_world::nbt::Tag;

    #[test]
    fn a_used_run_is_retold_in_place() {
        let spawn = |item: &str, x: f64| Spawn {
            stack: ItemStack::new(item, 1),
            pos: [x, 0.0, 0.0],
            motion: None,
            delay: 10,
        };
        let mut spawns = vec![
            spawn("minecraft:stone", 0.0),
            spawn("minecraft:sweet_berries", 1.0),
            spawn("minecraft:sweet_berries", 2.0),
            spawn("minecraft:dirt", 3.0),
        ];
        let used = [UsedDrops {
            start: 1,
            end: 3,
            pos: [4, 64, 4],
            state: BlockStateId(0),
            table: "minecraft:harvest/sweet_berry_bush",
        }];
        retell_runs(&mut spawns, &used, |used, origin, drops| {
            assert_eq!(used.table, "minecraft:harvest/sweet_berry_bush");
            assert_eq!(origin, [4.5, 64.5, 4.5]);
            assert_eq!(drops.len(), 2);
            drops.remove(0);
            drops.push(ItemStack::new("minecraft:diamond", 1));
        });
        let items: Vec<(&str, f64)> = spawns.iter().map(|s| (&*s.stack.item, s.pos[0])).collect();
        assert_eq!(
            items,
            [
                ("minecraft:stone", 0.0),
                ("minecraft:sweet_berries", 1.0),
                ("minecraft:diamond", 2.0),
                ("minecraft:dirt", 3.0),
            ]
        );
    }

    #[test]
    fn ominous_banner_survives_placement_save_and_drop() {
        let original = raider::ominous_banner();
        let Tag::Compound(components) = item_data::encode_components(&original).unwrap() else {
            panic!("banner components");
        };
        for block in ["minecraft:white_banner", "minecraft:white_wall_banner"] {
            let pos = (1, 64, 2);
            let fresh = BlockEntity::for_block(block).unwrap();
            let fresh = save::encode_block_entity(pos, &fresh, block);
            let placed = block_entity_item::place(&fresh, &components, false);
            let (_, placed) = save::decode_block_entity(&placed).unwrap();
            let saved = save::encode_block_entity(pos, &placed, block);
            let (_, loaded) = save::decode_block_entity(&saved).unwrap();
            let BlockEntity::Banner(banner) = &loaded else {
                panic!("placed banner");
            };
            assert_eq!(banner.layers.len(), item_data::OMINOUS_LAYERS.len());
            let mut pending = Pending::default();
            pending.record([pos.0, pos.1, pos.2], &loaded, block, None);
            let mut dropped = ItemStack::new("minecraft:white_banner", 1);
            pending.attach([pos.0, pos.1, pos.2], &mut dropped);
            assert!(dropped.data().unwrap().ominous_banner);
            assert!(horizon_client::banner::tooltip_layers(&dropped).is_empty());
            assert_eq!(
                horizon_client::item_tooltip::name_line(&dropped, &|_| None),
                "Ominous Banner"
            );
            assert_eq!(
                horizon_client::item_tooltip::rarity(&dropped, "common"),
                "uncommon"
            );
            assert_eq!(
                item_data::encode_components(&dropped),
                Some(Tag::Compound(components.clone()))
            );
        }
    }
}
