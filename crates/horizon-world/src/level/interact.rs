//! `BlockBehaviour.useWithoutItem` (right-click with an empty hand or an item
//! that does not handle the block itself).
use super::{Behaviours, LevelAccess, LevelEvent, Sim, Ticks, classes::Class, update::below};
use crate::registry::BlockStateId;
use horizon_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use std::sync::Arc;

/// `InteractionResult` as seen by the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionResult {
    Success,
    Consume,
    Pass,
}
impl InteractionResult {
    pub fn consumes_action(self) -> bool {
        !matches!(self, Self::Pass)
    }
}

/// `SelectableSlotContainer.getHitSlot`: the slot of a block with `rows` by
/// `columns` slots on its front face (the way it `facing`) that the click at
/// `hit` on `face` of the block at `pos` lands in, if it is the front face.
pub fn hit_slot(
    facing: Direction,
    face: Direction,
    pos: BlockPos,
    hit: Vec3,
    rows: i32,
    columns: i32,
) -> Option<usize> {
    if facing != face {
        return None;
    }
    // The location relative to the block in front of the face, which only
    // moves the axis the face points along: the others are the block's own.
    let (dx, dy, dz) = (
        hit.x - f64::from(pos.x),
        hit.y - f64::from(pos.y),
        hit.z - f64::from(pos.z),
    );
    let (x, y) = match face {
        Direction::North => ((1.0 - dx) as f32, dy as f32),
        Direction::South => (dx as f32, dy as f32),
        Direction::West => (dz as f32, dy as f32),
        Direction::East => ((1.0 - dz) as f32, dy as f32),
        Direction::Down | Direction::Up => return None,
    };
    // getSection
    let section = |relative: f32, sections: i32| {
        let size = 16.0f32 / sections as f32;
        ((relative * 16.0f32 / size).floor() as i32).clamp(0, sections - 1)
    };
    let row = section(1.0 - y, rows);
    let column = section(x, columns);
    Some((column + row * columns) as usize)
}

/// The interacting player, as far as block behaviour needs it.
#[derive(Clone, Copy, Debug)]
pub struct PlayerContext {
    /// View rotation in degrees.
    pub yaw: f32,
    pub pitch: f32,
    /// `Abilities.mayBuild` (false in adventure mode).
    pub may_build: bool,
    /// `Player.canEat(false)` (hunger below max, or creative).
    pub can_eat: bool,
    /// `Player.canUseGameMasterBlocks` (creative operator).
    pub game_master: bool,
}

/// Player-side effects of an interaction, reported through
/// [`LevelEvent::Player`]; block changes are already applied.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEffect {
    /// Open the block's menu (crafting table, furnace, chest, sign editor, ...).
    OpenMenu,
    /// `CakeBlock.eat`: the player eats a slice (food 2, saturation 0.1).
    EatCake,
    /// Give the item to the player (dropping it when the inventory is full).
    GiveItem(Arc<str>),
    /// Pop the item at the block (composter bone meal).
    DropItem(Arc<str>),
    /// Evaluate the block-interaction loot table and pop the results.
    InteractLoot(&'static str),
    /// Try to sleep in the bed whose head is at the event position.
    Sleep,
    /// Respawn anchor: set the respawn point (or explode outside the Nether).
    RespawnAnchor,
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn player_effect(&mut self, pos: BlockPos, state: BlockStateId, effect: PlayerEffect) {
        self.emit(LevelEvent::Player { pos, state, effect });
    }

    pub(crate) fn use_without_item(
        &mut self,
        pos: BlockPos,
        face: Direction,
        hit: Vec3,
        player: PlayerContext,
    ) -> InteractionResult {
        let b = self.b;
        let p = b.props;
        let table = &b.table;
        let state = self.state(pos);
        let data = b.data(state);
        let block = b.block(state);
        use InteractionResult::*;
        for &class in data.chain.iter() {
            return match class {
                Class::Door => {
                    if !data.set_type.as_ref().is_some_and(|s| s.can_open_by_hand) {
                        return Pass;
                    }
                    let next = table.cycle(state, p.open);
                    self.set_block(pos, next, 10);
                    let open = table.bool(next, p.open);
                    self.door_sound(pos, next, open);
                    self.game_event(
                        pos,
                        if open {
                            "minecraft:block_open"
                        } else {
                            "minecraft:block_close"
                        },
                    );
                    Success
                }
                Class::TrapDoor => {
                    if !data.set_type.as_ref().is_some_and(|s| s.can_open_by_hand) {
                        return Pass;
                    }
                    let next = table.cycle(state, p.open);
                    self.set_block(pos, next, 2);
                    if b.is_waterlogged(next) {
                        self.schedule_water_tick(pos);
                    }
                    self.trapdoor_sound(pos, next, table.bool(next, p.open));
                    Success
                }
                Class::FenceGate => {
                    let next = if table.bool(state, p.open) {
                        table.set_bool(state, p.open, false)
                    } else {
                        let direction = super::place::from_y_rot(f64::from(player.yaw));
                        let mut next = state;
                        if b.facing(state) == direction.opposite() {
                            next = table.set_dir(next, p.facing, direction);
                        }
                        table.set_bool(next, p.open, true)
                    };
                    self.set_block(pos, next, 10);
                    let opens = table.bool(next, p.open);
                    let sound = if opens {
                        data.gate_open.clone()
                    } else {
                        data.gate_close.clone()
                    };
                    let pitch = self.random().next_float() * 0.1 + 0.9;
                    if let Some(sound) = sound {
                        self.sound(pos, &sound, 1.0, pitch);
                    }
                    self.game_event(
                        pos,
                        if opens {
                            "minecraft:block_open"
                        } else {
                            "minecraft:block_close"
                        },
                    );
                    Success
                }
                Class::Button => {
                    if table.bool(state, p.powered) {
                        return Consume;
                    }
                    self.button_press(pos, state);
                    Success
                }
                Class::Lever => {
                    self.lever_pull(pos, state);
                    Success
                }
                Class::Repeater => {
                    if !player.may_build {
                        return Pass;
                    }
                    self.set_block_and_update(pos, table.cycle(state, p.delay));
                    Success
                }
                Class::Comparator => {
                    if !player.may_build {
                        return Pass;
                    }
                    let next = table.cycle(state, p.mode);
                    let pitch = if table.text(next, p.mode) == "subtract" {
                        0.55
                    } else {
                        0.5
                    };
                    self.emit(LevelEvent::Sound {
                        pos,
                        sound: "minecraft:block.comparator.click".into(),
                        volume: 0.3,
                        pitch,
                    });
                    self.set_block(pos, next, 2);
                    if b.is_block(self.state(pos), block) {
                        self.comparator_refresh(pos, next);
                    }
                    Success
                }
                Class::Note => {
                    let next = table.cycle(state, p.note);
                    self.set_block_and_update(pos, next);
                    self.note_play(pos, next);
                    Success
                }
                Class::RedstoneWire => self.wire_use(pos, state, player),
                Class::DaylightDetector => {
                    if !player.may_build {
                        return Pass;
                    }
                    let next = table.cycle(state, p.inverted);
                    self.set_block(pos, next, 2);
                    self.game_event(pos, "minecraft:block_change");
                    self.daylight_detector_update_signal(pos, next);
                    Success
                }
                Class::Cake => {
                    if !player.can_eat {
                        return Pass;
                    }
                    self.player_effect(pos, state, PlayerEffect::EatCake);
                    self.game_event(pos, "minecraft:eat");
                    let bites = table.int(state, p.bites);
                    if bites < 6 {
                        self.set_block_and_update(pos, table.set_int(state, p.bites, bites + 1));
                    } else {
                        self.remove_block(pos, false);
                        self.game_event(pos, "minecraft:block_destroy");
                    }
                    Success
                }
                Class::CandleCake => {
                    if !player.can_eat {
                        return Pass;
                    }
                    self.player_effect(pos, state, PlayerEffect::EatCake);
                    self.game_event(pos, "minecraft:eat");
                    let cake = b.default_of(b.named.cake);
                    self.set_block_and_update(pos, table.set_int(cake, p.bites, 1));
                    self.drop_resources(state, pos);
                    Success
                }
                Class::Composter => {
                    if table.int(state, p.level) != 8 {
                        return Pass;
                    }
                    self.composter_extract_produce(pos, state);
                    Success
                }
                Class::CaveVines | Class::CaveVinesPlant => {
                    if !table.bool(state, p.berries) {
                        return Pass;
                    }
                    self.player_effect(
                        pos,
                        state,
                        PlayerEffect::InteractLoot("minecraft:harvest/cave_vine"),
                    );
                    let pitch = self.random().next_float() * 0.4 + 0.8;
                    self.emit(LevelEvent::Sound {
                        pos,
                        sound: "minecraft:block.cave_vines.pick_berries".into(),
                        volume: 1.0,
                        pitch,
                    });
                    let next = table.set_bool(state, p.berries, false);
                    self.set_block(pos, next, 2);
                    self.game_event(pos, "minecraft:block_change");
                    Success
                }
                Class::SweetBerryBush => {
                    if table.int(state, p.age) <= 1 {
                        return Pass;
                    }
                    self.player_effect(
                        pos,
                        state,
                        PlayerEffect::InteractLoot("minecraft:harvest/sweet_berry_bush"),
                    );
                    let pitch = 0.8 + self.random().next_float() * 0.4;
                    self.emit(LevelEvent::Sound {
                        pos,
                        sound: "minecraft:block.sweet_berry_bush.pick_berries".into(),
                        volume: 1.0,
                        pitch,
                    });
                    self.set_block(pos, table.set_int(state, p.age, 1), 2);
                    self.game_event(pos, "minecraft:block_change");
                    Success
                }
                Class::FlowerPot => {
                    let Some(potted) = data.link else {
                        return Consume;
                    };
                    if b.is_air(b.default_of(potted)) {
                        return Consume;
                    }
                    let item = b.block_data(potted).item.clone();
                    self.player_effect(pos, state, PlayerEffect::GiveItem(item));
                    self.set_block_and_update(pos, b.default_of(b.named.flower_pot));
                    self.game_event(pos, "minecraft:block_change");
                    Success
                }
                Class::DragonEgg => {
                    self.dragon_egg_teleport(pos, state);
                    Success
                }
                Class::AbstractBed => {
                    let part = table.text(state, p.part).to_owned();
                    let facing = b.facing(state);
                    let other_dir = if part == "foot" {
                        facing
                    } else {
                        facing.opposite()
                    };
                    let other = self.state(super::update::rel(pos, other_dir));
                    if b.is_block(other, block) && table.text(other, p.part) != part {
                        let head = if part == "head" {
                            pos
                        } else {
                            super::update::rel(pos, facing)
                        };
                        let head_state = self.state(head);
                        self.player_effect(head, head_state, PlayerEffect::Sleep);
                        Success
                    } else {
                        Consume
                    }
                }
                Class::RespawnAnchor => {
                    if table.int(state, p.charges) == 0 {
                        return Pass;
                    }
                    self.player_effect(pos, state, PlayerEffect::RespawnAnchor);
                    Success
                }
                Class::Light => {
                    if player.game_master {
                        self.set_block(pos, table.cycle(state, p.level), 2);
                        Success
                    } else {
                        Consume
                    }
                }
                Class::ChiseledBookShelf => {
                    // useItemOn and useWithoutItem agree on what a click takes: no
                    // slot under it passes, an empty slot consumes it, a book in
                    // it comes out (the block entity is the game's).
                    let Some(slot) = hit_slot(b.facing(state), face, pos, hit, 2, 3) else {
                        return Pass;
                    };
                    let occupied = [
                        p.slot_0_occupied,
                        p.slot_1_occupied,
                        p.slot_2_occupied,
                        p.slot_3_occupied,
                        p.slot_4_occupied,
                        p.slot_5_occupied,
                    ];
                    if table.bool(state, occupied[slot]) {
                        Success
                    } else {
                        Consume
                    }
                }
                Class::Bell => {
                    if self.bell_hit(pos, state, face, hit.y - f64::from(pos.y)) {
                        Success
                    } else {
                        Pass
                    }
                }
                Class::AbstractFurnace
                | Class::Anvil
                | Class::Barrel
                | Class::Beacon
                | Class::BrewingStand
                | Class::CartographyTable
                | Class::Chest
                | Class::Crafter
                | Class::CraftingTable
                | Class::Dispenser
                | Class::EnchantingTable
                | Class::EnderChest
                | Class::Grindstone
                | Class::Hopper
                | Class::Loom
                | Class::ShulkerBox
                | Class::Sign
                | Class::SmithingTable
                | Class::Stonecutter
                | Class::Lectern => {
                    self.player_effect(pos, state, PlayerEffect::OpenMenu);
                    Success
                }
                Class::Command | Class::Structure | Class::Jigsaw => {
                    if player.game_master {
                        self.player_effect(pos, state, PlayerEffect::OpenMenu);
                        Success
                    } else {
                        Pass
                    }
                }
                _ => continue,
            };
        }
        let _ = below;
        Pass
    }

    /// `RedstoneWireBlock.useWithoutItem`: toggle between dot and cross.
    fn wire_use(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        player: PlayerContext,
    ) -> InteractionResult {
        let b = self.b;
        if !player.may_build {
            return InteractionResult::Pass;
        }
        let connected = |s: BlockStateId, d: Direction| b.table.text(s, b.dir_prop(d)) != "none";
        let horizontal = super::update::HORIZONTAL;
        let cross = horizontal.into_iter().all(|d| connected(state, d));
        let dot = !horizontal.into_iter().any(|d| connected(state, d));
        if !(cross || dot) {
            return InteractionResult::Pass;
        }
        let power = b.table.int(state, b.props.power);
        let base = if cross {
            b.table
                .set_int(b.default_of(b.named.redstone_wire), b.props.power, power)
        } else {
            self.wire_cross_state(power)
        };
        let next = self.wire_connection_state(base, pos);
        if next == state {
            return InteractionResult::Pass;
        }
        self.set_block_and_update(pos, next);
        for direction in horizontal {
            let relative = super::update::rel(pos, direction);
            if connected(state, direction) != connected(next, direction)
                && self.is_redstone_conductor(self.state(relative), relative)
            {
                let block = b.block(next);
                self.update_neighbors_at_except(relative, block, direction.opposite());
            }
        }
        InteractionResult::Success
    }

    /// `ComposterBlock.extractProduce`: the bone meal pops out, the composter
    /// empties and its sound plays.
    fn composter_extract_produce(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        self.player_effect(
            pos,
            state,
            PlayerEffect::DropItem("minecraft:bone_meal".into()),
        );
        self.set_block_and_update(pos, b.table.set_int(state, b.props.level, 0));
        self.game_event(pos, "minecraft:block_change");
        self.emit(LevelEvent::Sound {
            pos,
            sound: "minecraft:block.composter.empty".into(),
            volume: 1.0,
            pitch: 1.0,
        });
    }

    /// `ComposterBlock.addLayer` once its `compostable` provider gave
    /// `layers`: the new state (the old one when no layer is added).
    fn composter_add_layer(&mut self, pos: BlockPos, layers: i32) -> BlockStateId {
        let b = self.b;
        let state = self.state(pos);
        if layers <= 0 {
            return state;
        }
        let level = b.table.int(state, b.props.level);
        let next_level = (level + layers).clamp(0, 7);
        let next = b.table.set_int(state, b.props.level, next_level);
        self.set_block_and_update(pos, next);
        self.game_event_state(pos, "minecraft:block_change", next);
        if next_level == 7 {
            self.schedule_tick(pos, b.block(state), 20);
        }
        next
    }

    /// `DragonEggBlock.teleport`.
    fn dragon_egg_teleport(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        for _ in 0..1000 {
            let dx = self.random().next_int_bound(16).unwrap_or(0)
                - self.random().next_int_bound(16).unwrap_or(0);
            let dy = self.random().next_int_bound(8).unwrap_or(0)
                - self.random().next_int_bound(8).unwrap_or(0);
            let dz = self.random().next_int_bound(16).unwrap_or(0)
                - self.random().next_int_bound(16).unwrap_or(0);
            let test = pos.offset(dx, dy, dz);
            if b.is_air(self.state(test))
                && !b.is_air(self.below_state(test))
                && self.is_inside_build_height(test.y)
            {
                // BlockUtil.packDifferenceInPosition(pos, test, 16, 8, 16)
                let packed = ((dx + 16) << 16) | ((dy + 8) << 8) | (dz + 16);
                self.level_event(2015, pos, packed);
                self.set_block(test, state, 2);
                self.remove_block(pos, false);
                return;
            }
        }
    }
}

impl Behaviours {
    /// `ComposterBlock.addLayer` at `pos` with the layers its item's
    /// `compostable` provider gave, followed by level event 1500 (fill
    /// particles and sound; data 1 when a layer was added) as every caller
    /// does: `useItemOn` and the hopper input container. The new state.
    pub fn composter_insert<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        layers: i32,
    ) -> BlockStateId {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        let next = sim.composter_add_layer(pos, layers);
        sim.level_event(1500, pos, i32::from(next != state));
        next
    }

    /// `ComposterBlock.addLayer` without level event 1500, for a villager
    /// that composts several items and sends the event once at the end
    /// (`WorkAtComposter.compostItems`). The new state.
    pub fn composter_layer<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        layers: i32,
    ) -> BlockStateId {
        self.sim(level, ticks).composter_add_layer(pos, layers)
    }

    /// `ComposterBlock.extractProduce` by a villager (`WorkAtComposter`).
    pub fn composter_extract<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        sim.composter_extract_produce(pos, state);
    }

    /// Level event 1500 (`WorkAtComposter.spawnComposterFillEffects`): data 1
    /// when the composter's state changed.
    pub fn composter_fill_effects<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        changed: bool,
    ) {
        self.sim(level, ticks)
            .level_event(1500, pos, i32::from(changed));
    }

    /// `ComposterBlock.empty`: the hopper output container took the bone meal.
    pub fn composter_empty<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        let next = self.table.set_int(state, self.props.level, 0);
        sim.set_block_and_update(pos, next);
        sim.game_event_state(pos, "minecraft:block_change", next);
    }

    /// `BlockBehaviour.attack` when mining begins: `RedStoneOreBlock.attack`
    /// lights the ore, `DragonEggBlock.attack` teleports the egg,
    /// `NoteBlock.attack` plays the note. True for a note block, whose player
    /// also earns the `play_noteblock` statistic.
    pub fn attack_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> bool {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        let chain = &self.data(state).chain;
        if chain.contains(&Class::DragonEgg) {
            sim.dragon_egg_teleport(pos, state);
        }
        if chain.contains(&Class::Note) {
            sim.note_play(pos, state);
            return true;
        }
        if chain.contains(&Class::RedStoneOre) && !self.table.bool(state, self.props.lit) {
            sim.set_block_and_update(pos, self.table.set_bool(state, self.props.lit, true));
        }
        false
    }

    /// `BlockState.useWithoutItem` for a right-click on `pos` (hit face and
    /// absolute hit location as in `BlockHitResult`).
    pub fn use_without_item<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        face: Direction,
        hit: Vec3,
        player: PlayerContext,
    ) -> InteractionResult {
        self.sim(level, ticks)
            .use_without_item(pos, face, hit, player)
    }
    /// `DoorBlock.setOpen`: a mob opens or closes the door at `pos` (either half), with its sound
    /// and game event. False when it is no door or already so.
    pub fn set_door_open<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        open: bool,
    ) -> bool {
        let state = level.block_state(pos);
        if !self.data(state).is(Class::Door) || self.table.bool(state, self.props.open) == open {
            return false;
        }
        let mut sim = self.sim(level, ticks);
        let next = self.table.set_bool(state, self.props.open, open);
        sim.set_block(pos, next, 10);
        sim.door_sound(pos, next, open);
        sim.game_event(
            pos,
            if open {
                "minecraft:block_open"
            } else {
                "minecraft:block_close"
            },
        );
        true
    }
    /// `BlockState.useItemOn` with `item` in the main hand, then, when the
    /// block asks to try with an empty hand (the default), `useWithoutItem`.
    /// `SweetBerryBushBlock.useItemOn` passes bone meal on a bush that can
    /// still grow on to the item.
    #[allow(clippy::too_many_arguments)]
    pub fn use_item_on<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        face: Direction,
        hit: Vec3,
        player: PlayerContext,
        item: Option<&str>,
    ) -> InteractionResult {
        let state = level.block_state(pos);
        if item == Some("minecraft:bone_meal")
            && self.data(state).chain.contains(&Class::SweetBerryBush)
            && self.table.int(state, self.props.age) < 3
        {
            return InteractionResult::Pass;
        }
        if self.data(state).chain.contains(&Class::CopperGolemStatue) {
            return self.turn_statue(level, ticks, pos, state, item);
        }
        // RespawnAnchorBlock.useItemOn: glowstone (`isRespawnFuel`) charges an
        // anchor below 4 charges (`canBeCharged`) and the click ends there.
        if item == Some("minecraft:glowstone")
            && self.data(state).chain.contains(&Class::RespawnAnchor)
            && self.table.int(state, self.props.charges) < 4
        {
            // RespawnAnchorBlock.charge.
            let charges = self.table.int(state, self.props.charges);
            let next = self.table.set_int(state, self.props.charges, charges + 1);
            let mut sim = self.sim(level, ticks);
            sim.set_block_and_update(pos, next);
            sim.game_event_state(pos, "minecraft:block_change", next);
            sim.sound(
                pos,
                &"minecraft:block.respawn_anchor.charge".into(),
                1.0,
                1.0,
            );
            return InteractionResult::Success;
        }
        // NoteBlock.useItemOn: a mob head on the top face is placed, not tuned
        // (`#minecraft:noteblock_top_instruments`).
        if face == Direction::Up
            && self.data(state).chain.contains(&Class::Note)
            && item.is_some_and(|i| {
                matches!(
                    i,
                    "minecraft:zombie_head"
                        | "minecraft:skeleton_skull"
                        | "minecraft:creeper_head"
                        | "minecraft:dragon_head"
                        | "minecraft:wither_skeleton_skull"
                        | "minecraft:piglin_head"
                        | "minecraft:player_head"
                )
            })
        {
            return InteractionResult::Pass;
        }
        self.use_without_item(level, ticks, pos, face, hit, player)
    }

    /// `CopperGolemStatueBlock.useItemOn` (and its weathering subclass's): anything but an axe
    /// (`#minecraft:axes`, every one named `*_axe`), even an empty hand, turns the statue to its
    /// next pose; honeycomb waxes a statue that still weathers instead.
    fn turn_statue<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        item: Option<&str>,
    ) -> InteractionResult {
        let item = item.unwrap_or("minecraft:air");
        let waxed = self.table.name(state).starts_with("minecraft:waxed_");
        if item.ends_with("_axe") || (item == "minecraft:honeycomb" && !waxed) {
            return InteractionResult::Pass;
        }
        let mut sim = self.sim(level, ticks);
        sim.sound(
            pos,
            &"minecraft:entity.copper_golem_become_statue".into(),
            1.0,
            1.0,
        );
        sim.set_block_and_update(pos, self.table.cycle(state, self.props.copper_golem_pose));
        sim.game_event(pos, "minecraft:block_change");
        InteractionResult::Success
    }
}
