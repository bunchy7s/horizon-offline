//! Scheduled block ticks (`BlockBehaviour.tick`), random ticks
//! (`BlockBehaviour.randomTick`) and the per-tick drivers.
use super::{
    Behaviours, LevelAccess, LevelEvent, Sim, Spread, Ticks,
    classes::Class,
    data::state_flags,
    update::{HORIZONTAL, above, below, rel},
};
use crate::{fluid::FluidKind, registry::BlockStateId};
use horizon_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
    random::RandomSource,
    ticks::TickTypeId,
};
use std::sync::Arc;

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn rand_int(&mut self, bound: i32) -> i32 {
        self.random().next_int_bound(bound).unwrap_or(0)
    }

    /// `BlockState.tick` (scheduled block tick).
    pub fn tick_block(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let p = b.props;
        let block = b.block(state);
        let chain = &b.data(state).chain;
        for &class in chain.iter() {
            match class {
                Class::BambooStalk
                | Class::BigDripleafStem
                | Class::Cactus
                | Class::ChorusFlower
                | Class::ChorusPlant
                | Class::GrowingPlant
                | Class::SugarCane => {
                    if !self.can_survive(state, pos) {
                        self.destroy_block(pos, true, 512);
                    }
                }
                Class::HangingMoss => {
                    let above_state = self.above_state(pos);
                    if !(b.multiface_can_attach(above_state, Direction::Up)
                        || b.is_block(above_state, block))
                    {
                        self.destroy_block(pos, true, 512);
                    }
                }
                Class::BasePressurePlate => {
                    let signal = self.plate_signal_for_state(state);
                    if signal > 0 {
                        self.plate_check_pressed(pos, state, signal);
                    }
                }
                Class::DetectorRail => {
                    if b.table.bool(state, b.props.powered) {
                        self.detector_rail_check_pressed(pos, state);
                    }
                }
                Class::BigDripleaf => self.big_dripleaf_tick(pos, state),
                Class::AbstractCauldron => self.cauldron_tick(state, pos),
                Class::BubbleColumn => {
                    let below_state = self.below_state(pos);
                    self.bubble_column_update(pos, state, below_state);
                }
                Class::Button => self.button_tick(pos, state),
                Class::Diode => self.diode_tick(pos, state),
                Class::Composter => {
                    if b.table.int(state, p.level) == 7 {
                        self.set_block_and_update(pos, b.table.cycle(state, p.level));
                        self.emit(LevelEvent::Sound {
                            pos,
                            sound: "minecraft:block.composter.ready".into(),
                            volume: 1.0,
                            pitch: 1.0,
                        });
                    }
                }
                Class::Coral => {
                    if !self.coral_scan_for_water(pos) {
                        if let Some(dead) = b.data(state).link {
                            self.set_block(pos, b.default_of(dead), 2);
                        }
                    }
                }
                Class::CoralFan | Class::CoralPlant | Class::CoralWallFan => {
                    if !self.coral_plant_scan_for_water(state, pos) {
                        if let Some(dead) = b.data(state).link {
                            let mut next =
                                b.table.set_bool(b.default_of(dead), p.waterlogged, false);
                            if class == Class::CoralWallFan {
                                next = b.table.set_dir(next, p.facing, b.facing(state));
                            }
                            self.set_block(pos, next, 2);
                        }
                    }
                }
                Class::Chest | Class::EnderChest => {
                    // recheckOpen: openerCountChanged broadcasts the lid, a
                    // trapped chest also updates its neighbours on a change,
                    // and the recheck repeats while anyone has it open.
                    if let Some((previous, count)) = self.level.recheck_container_openers(pos) {
                        self.block_event(pos, block, 1, count);
                        if previous != count && b.data(state).is(Class::TrappedChest) {
                            self.update_neighbors_at(pos, block);
                            self.update_neighbors_at(below(pos), block);
                        }
                        if count > 0 {
                            self.schedule_tick(pos, block, 5);
                        }
                    }
                }
                Class::Dispenser | Class::Crafter => {
                    self.emit(LevelEvent::BlockEvent {
                        pos,
                        state,
                        kind: "dispense",
                    });
                }
                // CommandBlock.tick runs commands: the caller does it.
                Class::Command => {
                    self.emit(LevelEvent::BlockEvent {
                        pos,
                        state,
                        kind: "command_block",
                    });
                }
                // TestBlock.tick resets the block entity: the caller owns it.
                Class::TestBlock => {
                    self.emit(LevelEvent::BlockEvent {
                        pos,
                        state,
                        kind: "test_block_tick",
                    });
                }
                Class::FallingBlock => self.falling_block_tick(pos, state),
                Class::Brushable => {
                    // BrushableBlock.tick checks the brush reset first (the
                    // caller keeps the brush counts).
                    self.emit(LevelEvent::BlockEvent {
                        pos,
                        state,
                        kind: "brushable",
                    });
                    if self.is_free(self.below_state(pos)) && pos.y >= self.level.min_y() {
                        self.falling_block_fall(pos, state, None);
                    }
                }
                Class::Farmland => {
                    if !self.can_survive(state, pos) {
                        self.turn_to_base_block(pos, state);
                    }
                }
                Class::Path => self.turn_to_base_block(pos, state),
                Class::Fire => self.fire_tick(pos, state),
                Class::Frogspawn => {
                    if !self.can_survive(state, pos) {
                        self.destroy_block(pos, false, 512);
                    } else {
                        self.emit(LevelEvent::BlockEvent {
                            pos,
                            state,
                            kind: "hatch",
                        });
                        self.destroy_block(pos, false, 512);
                    }
                }
                Class::FrostedIce => self.frosted_ice_tick(pos, state),
                Class::Leaves => {
                    let next = self.leaves_update_distance(state, pos);
                    self.set_block_and_update(pos, next);
                }
                // LecternBlock.tick: the page-turn pulse ends.
                Class::Lectern => {
                    self.set_block_and_update(pos, b.table.set_bool(state, p.powered, false));
                    self.update_neighbors_at(below(pos), block);
                }
                Class::LightningRod => {
                    self.set_block_and_update(pos, b.table.set_bool(state, p.powered, false));
                    let facing = b.facing(state);
                    self.update_neighbors_at(rel(pos, facing.opposite()), block);
                }
                Class::Liquid => {
                    if self.should_bubble_column_occupy(state) {
                        let below_state = self.below_state(pos);
                        let column = b.named.bubble_column;
                        self.bubble_column_update_for(column, pos, state, below_state);
                    }
                }
                Class::Observer => self.observer_tick(pos, state),
                Class::Eyeblossom => self.eyeblossom_tick(state, pos, false),
                Class::CreakingHeart => self.heart_tick(state, pos),
                Class::DriedGhast => self.dried_ghast_tick(state, pos),
                Class::SnifferEgg => self.sniffer_egg_tick(state, pos),
                Class::RedstoneLamp => self.lamp_tick(pos, state),
                Class::RedstoneTorch => self.torch_tick(pos, state),
                Class::Scaffolding => self.scaffolding_tick(pos, state),
                Class::SculkSensor => self.sculk_sensor_tick(pos, state),
                Class::SculkShrieker => self.sculk_shrieker_tick(pos, state),
                Class::SculkCatalyst => self.sculk_catalyst_tick(pos, state),
                Class::Speleothem => {
                    let tip = b.table.dir(state, p.vertical_direction);
                    if tip == Some(Direction::Up) && !self.can_survive(state, pos) {
                        self.destroy_block(pos, true, 512);
                    } else {
                        self.spawn_falling_stalactite(pos, state);
                    }
                }
                Class::Target => {
                    if b.table.int(state, p.power) != 0 {
                        self.set_block_and_update(pos, b.table.set_int(state, p.power, 0));
                    }
                }
                Class::TripWire => self.tripwire_tick(pos),
                Class::TripWireHook => {
                    self.tripwire_hook_calculate(pos, state, false, true, -1, None)
                }
                _ => continue,
            }
            return;
        }
    }

    /// `ServerLevel.tickBlock` once the block at `pos` is the tick's: the
    /// level may handle the tick first, else `BlockState.tick`.
    fn tick_due_block(&mut self, state: BlockStateId, pos: BlockPos) {
        let (mut current, mut handled) = (state, false);
        self.level
            .before_tick_block(self.ticks, pos, &mut current, &mut handled);
        // A block the level changed ticks as it is now, if it is still the
        // tick's block.
        if !handled && (current == state || self.block_of(current) == self.block_of(state)) {
            self.tick_block(current, pos);
        }
    }

    /// `BlockState.randomTick` followed by the fluid's random tick.
    pub fn random_tick_at(&mut self, pos: BlockPos) {
        let state = self.state(pos);
        if self.b.is_randomly_ticking(state) {
            let (mut current, mut handled) = (state, false);
            self.level
                .before_random_tick(self.ticks, pos, &mut current, &mut handled);
            // A block the level changed ticks as it is now, if it still ticks
            // randomly.
            if !handled && (current == state || self.b.is_randomly_ticking(current)) {
                self.random_tick_block(current, pos);
            }
        }
        // Only lava's fluid state ticks randomly. A lava block is ticked twice
        // (LiquidBlock.randomTick, then FluidState.randomTick), as in vanilla.
        let fluid = self.b.fluid(state).kind();
        if matches!(fluid, FluidKind::Lava | FluidKind::FlowingLava) {
            self.lava_random_tick(pos);
        }
    }

    pub fn random_tick_block(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let p = b.props;
        let block = b.block(state);
        let data = b.data(state);
        let chain = &data.chain;
        for class in chain.iter().copied() {
            match class {
                Class::BambooSapling => {
                    if self.rand_int(3) == 0
                        && self.is_empty_block(above(pos))
                        && self.raw_brightness(above(pos), 0) >= 9
                    {
                        self.bamboo_sapling_grow(pos);
                    }
                }
                Class::BambooStalk => {
                    if b.table.int(state, p.stage) == 0
                        && self.rand_int(3) == 0
                        && self.is_empty_block(above(pos))
                        && self.raw_brightness(above(pos), 0) >= 9
                    {
                        let height = self.bamboo_height_below(pos) + 1;
                        if height < 16 {
                            self.bamboo_grow(state, pos, height);
                        }
                    }
                }
                Class::Beetroot | Class::TorchflowerCrop => {
                    if self.rand_int(3) != 0 {
                        self.crop_random_tick(state, pos);
                    }
                }
                Class::Crop => self.crop_random_tick(state, pos),
                Class::BuddingAmethyst => self.budding_amethyst_tick(pos),
                Class::Cactus => self.cactus_random_tick(state, pos),
                Class::ChorusFlower => self.chorus_flower_random_tick(state, pos),
                Class::Cocoa => {
                    if self.rand_int(5) == 0 {
                        let age = b.table.int(state, p.age);
                        if age < 2 {
                            self.set_block(pos, b.table.set_int(state, p.age, age + 1), 2);
                        }
                    }
                }
                Class::Farmland => self.farmland_random_tick(state, pos),
                Class::NetherPortal => {
                    // NetherPortalBlock.randomTick: only the overworld sets
                    // NETHER_PORTAL_SPAWNS_PIGLINS.
                    let rules = self.level.game_rules();
                    let difficulty = self.level.difficulty();
                    if rules.spawn_mobs
                        && rules.spawn_monsters
                        && difficulty != 0
                        && self.level.dimension() == super::Dimension::Overworld
                        && self.rand_int(2000) < difficulty
                    {
                        self.emit(LevelEvent::BlockEvent {
                            pos,
                            state,
                            kind: "portal_spawn",
                        });
                    }
                }
                Class::GrowingPlantHead => {
                    if b.table.int(state, p.age) < 25
                        && self.random().next_double() < data.grow_probability
                    {
                        let growth = rel(pos, data.growth_direction);
                        if self.growing_plant_can_grow_into(state, self.state(growth)) {
                            let next = self.growing_plant_grow_into_state(state);
                            self.set_block_and_update(growth, next);
                        }
                    }
                }
                Class::Ice => {
                    if self.level.block_light(pos) > 11 - i32::from(b.facts(state).dampening) {
                        self.ice_melt(pos, state);
                    }
                }
                Class::Leaves => {
                    if !b.table.bool(state, p.persistent) && b.table.int(state, p.distance) == 7 {
                        self.drop_resources(state, pos);
                        self.remove_block(pos, false);
                    }
                }
                Class::Liquid => {
                    if matches!(
                        b.fluid(state).kind(),
                        FluidKind::Lava | FluidKind::FlowingLava
                    ) {
                        self.lava_random_tick(pos);
                    }
                }
                Class::MangrovePropagule => {
                    if !b.table.bool(state, p.hanging) {
                        if self.rand_int(7) == 0 {
                            self.sapling_advance_tree(pos, state);
                        }
                    } else if b.table.int(state, p.age) != 4 {
                        self.set_block(pos, b.table.cycle(state, p.age), 2);
                    }
                }
                Class::Sapling => {
                    if self.max_local_raw_brightness(above(pos)) >= 9 && self.rand_int(7) == 0 {
                        self.sapling_advance_tree(pos, state);
                    }
                }
                Class::Mushroom => self.mushroom_random_tick(state, pos),
                Class::NetherWart => {
                    let age = b.table.int(state, p.age);
                    if age < 3 && self.rand_int(10) == 0 {
                        self.set_block(pos, b.table.set_int(state, p.age, age + 1), 2);
                    }
                }
                Class::Nylium => {
                    if !self.can_be_nylium(state, pos) {
                        self.set_block_and_update(pos, b.default_of(b.named.netherrack));
                    }
                }
                Class::PitcherCrop => {
                    let speed = self.crop_growth_speed(block, pos);
                    if self.rand_int((25.0 / speed) as i32 + 1) == 0 {
                        self.pitcher_grow(state, pos, 1);
                    }
                }
                Class::Speleothem => self.speleothem_random_tick(state, pos),
                Class::RedStoneOre => {
                    if b.table.bool(state, p.lit) {
                        self.set_block_and_update(pos, b.table.set_bool(state, p.lit, false));
                    }
                }
                Class::SnowLayer => {
                    if self.level.block_light(pos) > 11 {
                        self.drop_resources(state, pos);
                        self.remove_block(pos, false);
                        self.game_event(pos, "minecraft:block_destroy");
                    }
                }
                Class::SpreadingSnowy => self.spreading_snowy_tick(state, pos),
                Class::Stem => self.stem_random_tick(state, pos),
                Class::SugarCane => {
                    if self.is_empty_block(above(pos)) {
                        let mut height = 1;
                        while b.is_block(self.state(pos.offset(0, -height, 0)), block) {
                            height += 1;
                        }
                        if height < 3 {
                            let age = b.table.int(state, p.age);
                            if age == 15 {
                                self.set_block_and_update(above(pos), b.default_of(block));
                                self.set_block(pos, b.table.set_int(state, p.age, 0), 260);
                            } else {
                                self.set_block(pos, b.table.set_int(state, p.age, age + 1), 260);
                            }
                        }
                    }
                }
                Class::SweetBerryBush => {
                    let age = b.table.int(state, p.age);
                    if age < 3 && self.rand_int(5) == 0 && self.raw_brightness(above(pos), 0) >= 9 {
                        let next = b.table.set_int(state, p.age, age + 1);
                        self.set_block(pos, next, 2);
                        self.game_event(pos, "minecraft:block_change");
                    }
                }
                Class::Vine => self.vine_random_tick(state, pos),
                Class::Eyeblossom => self.eyeblossom_tick(state, pos, true),
                Class::FlowerPot => self.potted_eyeblossom_tick(state, pos),
                Class::TurtleEgg => self.turtle_egg_tick(state, pos),
                Class::DriedGhast => self.dried_ghast_random_tick(state, pos),
                Class::WeatheringCopperFull => self.copper_change_over_time(state, pos),
                _ => continue,
            }
            return;
        }
        if data.implements(super::classes::Interface::WeatheringCopper) {
            let upper_door = data.is(Class::Door) && b.table.text(state, p.half) == "upper";
            let right_chest = data.is(Class::Chest) && b.table.text(state, p.kind) == "right";
            if !upper_door && !right_chest {
                self.copper_change_over_time(state, pos);
            }
        }
    }

    // ---- helpers ----

    pub(crate) fn turn_to_base_block(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let base = b.data(state).link.unwrap_or(b.named.dirt);
        // Block.pushEntitiesUp(state, base, level, pos): the game lifts what
        // stands in the new block's added top onto it.
        self.emit(LevelEvent::BlockEvent {
            pos,
            state,
            kind: "push_entities_up",
        });
        self.set_block_and_update(pos, b.default_of(base));
        self.game_event(pos, "minecraft:block_change");
    }
    /// `FallingBlock.isFree`.
    pub(crate) fn is_free(&self, state: BlockStateId) -> bool {
        let b = self.b;
        b.is_air(state)
            || b.tag("minecraft:fire")
                .is_some_and(|tag| b.in_set(state, tag))
            || b.liquid(state)
            || b.is_replaceable(state)
    }
    fn leaves_update_distance(&self, state: BlockStateId, pos: BlockPos) -> BlockStateId {
        let mut distance = 7;
        for direction in Direction::ALL {
            distance = distance.min(self.leaf_distance_at(self.rel_state(pos, direction)) + 1);
            if distance == 1 {
                break;
            }
        }
        self.b.table.set_int(state, self.b.props.distance, distance)
    }
    fn bubble_column_update(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        below_state: BlockStateId,
    ) {
        let column = self.b.block(state);
        self.bubble_column_update_for(column, pos, state, below_state);
    }
    /// `BubbleColumnBlock.updateColumn(column, level, pos, occupyState, belowState)`.
    pub(crate) fn bubble_column_update_for(
        &mut self,
        column: super::states::BlockId,
        pos: BlockPos,
        occupy: BlockStateId,
        below_state: BlockStateId,
    ) {
        let b = self.b;
        if !self.bubble_column_can_occupy(occupy) {
            return;
        }
        let column_state = if b.is_block(below_state, column) {
            below_state
        } else if b.in_set(below_state, &b.tags.enables_bubble_column_push_up) {
            b.table.set_bool(b.default_of(column), b.props.drag, false)
        } else if b.in_set(below_state, &b.tags.enables_bubble_column_drag_down) {
            b.table.set_bool(b.default_of(column), b.props.drag, true)
        } else if b.is_block(occupy, column) {
            b.default_of(b.named.water)
        } else {
            occupy
        };
        self.set_block(pos, column_state, 2);
        let mut up = above(pos);
        while self.bubble_column_can_occupy(self.state(up)) {
            if !self.set_block(up, column_state, 2) {
                return;
            }
            up = above(up);
        }
    }
    fn big_dripleaf_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let (next, delay) = match b.table.text(state, b.props.tilt) {
            _ if self.has_neighbor_signal(pos) => return self.big_dripleaf_reset_tilt(pos, state),
            "unstable" => ("partial", 10),
            "partial" => ("full", 100),
            "full" => return self.big_dripleaf_reset_tilt(pos, state),
            _ => return,
        };
        // setTiltAndScheduleTick: PARTIAL and FULL cause vibrations.
        self.set_block(pos, b.table.set_text(state, b.props.tilt, next), 2);
        self.game_event(pos, "minecraft:block_change");
        let pitch = self.random().next_float() * 0.4 + 0.8;
        self.emit(LevelEvent::Sound {
            pos,
            sound: "minecraft:block.big_dripleaf.tilt_down".into(),
            volume: 1.0,
            pitch,
        });
        self.schedule_tick(pos, b.block(state), delay);
    }
    fn scaffolding_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let distance = self.scaffolding_distance(pos);
        let bottom = self.scaffolding_is_bottom(pos, distance);
        let next = b.table.set_bool(
            b.table.set_int(state, b.props.distance, distance),
            b.props.bottom,
            bottom,
        );
        if distance == 7 {
            if b.table.int(state, b.props.distance) == 7 {
                self.falling_block_fall(pos, next, None);
            } else {
                self.destroy_block(pos, true, 512);
            }
        } else if state != next {
            self.set_block_and_update(pos, next);
        }
    }
    /// `ScaffoldingBlock.isBottom`.
    pub(crate) fn scaffolding_is_bottom(&self, pos: BlockPos, distance: i32) -> bool {
        let b = self.b;
        distance > 0 && !b.is_block(self.below_state(pos), b.named.scaffolding)
    }
    fn fire_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let p = b.props;
        let block = b.block(state);
        let delay = 30 + self.rand_int(10);
        self.schedule_tick(pos, block, delay);
        if !self.level.can_spread_fire_around(pos) {
            return;
        }
        if !self.can_survive(state, pos) {
            self.remove_block(pos, false);
        }
        let below_state = self.below_state(pos);
        let infiniburn = match self.level.dimension() {
            super::Dimension::Overworld => &b.tags.infiniburn_overworld,
            super::Dimension::Nether => &b.tags.infiniburn_nether,
            super::Dimension::End => &b.tags.infiniburn_end,
        };
        let infini = b.in_set(below_state, infiniburn);
        let age = b.table.int(state, p.age);
        let raining = self.level.is_raining();
        if !infini
            && raining
            && self.fire_near_rain(pos)
            && self.random().next_float() < 0.2 + age as f32 * 0.03
        {
            self.remove_block(pos, false);
            return;
        }
        let new_age = 15.min(age + self.rand_int(3) / 2);
        let mut state = state;
        if age != new_age {
            state = b.table.set_int(state, p.age, new_age);
            self.set_block(pos, state, 260);
        }
        if !infini {
            if !self.is_valid_fire_location(pos) {
                if !self.sturdy(below(pos), Direction::Up) || age > 3 {
                    self.remove_block(pos, false);
                }
                return;
            }
            if age == 15 && self.rand_int(4) == 0 && !self.can_burn(self.below_state(pos)) {
                self.remove_block(pos, false);
                return;
            }
        }
        // FireBlock.tick: the INCREASED_FIRE_BURNOUT environment attribute.
        let burnout = self.level.increased_fire_burnout(pos);
        let extra = if burnout { -50 } else { 0 };
        for (direction, chance) in [
            (Direction::East, 300),
            (Direction::West, 300),
            (Direction::Down, 250),
            (Direction::Up, 250),
            (Direction::North, 300),
            (Direction::South, 300),
        ] {
            self.fire_check_burn_out(pos, rel(pos, direction), chance + extra, age);
        }
        for xx in -1..=1 {
            for zz in -1..=1 {
                for yy in -1..=4 {
                    if xx == 0 && yy == 0 && zz == 0 {
                        continue;
                    }
                    let mut rate = 100;
                    if yy > 1 {
                        rate += (yy - 1) * 100;
                    }
                    let test = pos.offset(xx, yy, zz);
                    let ignite = self.fire_ignite_odds_at(test);
                    if ignite > 0 {
                        let mut odds = (ignite + 40 + self.level.difficulty() * 7) / (age + 30);
                        if burnout {
                            odds /= 2;
                        }
                        if odds > 0
                            && self.rand_int(rate) <= odds
                            && (!self.level.is_raining() || !self.fire_near_rain(test))
                        {
                            let spread_age = 15.min(age + self.rand_int(5) / 4);
                            if self.level.allow_spread(Spread::Fire, pos, test) {
                                let next = self.fire_state_with_age(test, spread_age);
                                self.set_block_and_update(test, next);
                            }
                        }
                    }
                }
            }
        }
    }
    /// `FireBlock.getIgniteOdds(level, pos)`: only for air with flammable neighbours.
    fn fire_ignite_odds_at(&self, pos: BlockPos) -> i32 {
        if !self.is_empty_block(pos) {
            return 0;
        }
        Direction::ALL
            .into_iter()
            .map(|d| self.ignite_odds(self.rel_state(pos, d)))
            .max()
            .unwrap_or(0)
    }
    fn fire_near_rain(&self, pos: BlockPos) -> bool {
        self.level.is_raining_at(pos)
            || self.level.is_raining_at(rel(pos, Direction::West))
            || self.level.is_raining_at(rel(pos, Direction::East))
            || self.level.is_raining_at(rel(pos, Direction::North))
            || self.level.is_raining_at(rel(pos, Direction::South))
    }
    /// `FireBlock.checkBurnOut` of the block at `pos` by the fire at `fire`.
    fn fire_check_burn_out(&mut self, fire: BlockPos, pos: BlockPos, chance: i32, age: i32) {
        let b = self.b;
        let odds = self.burn_odds(self.state(pos));
        if self.rand_int(chance) < odds {
            let old = self.state(pos);
            let allowed = self.level.allow_spread(Spread::Fire, fire, pos);
            if self.rand_int(age + 10) < 5 && !self.level.is_raining_at(pos) {
                let new_age = (age + self.rand_int(5) / 4).min(15);
                if allowed {
                    let next = self.fire_state_with_age(pos, new_age);
                    self.set_block_and_update(pos, next);
                }
            } else if allowed {
                self.remove_block(pos, false);
            }
            if allowed && b.instance_of(old, Class::Tnt) {
                self.prime_tnt(pos);
            }
        }
    }
    fn frosted_ice_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let block = b.block(state);
        let fewer = |sim: &Self, at: BlockPos, limit: i32| {
            let mut count = 0;
            for d in Direction::ALL {
                if sim.b.is_block(sim.rel_state(at, d), block) {
                    count += 1;
                    if count >= limit {
                        return false;
                    }
                }
            }
            true
        };
        if self.rand_int(3) == 0 || fewer(self, pos, 4) {
            let brightness = if self.level.dimension() == super::Dimension::End {
                self.level.block_light(pos)
            } else {
                self.max_local_raw_brightness(pos)
            };
            let age = b.table.int(state, b.props.age);
            if brightness > 11 - age - i32::from(b.facts(state).dampening)
                && self.frosted_slightly_melt(pos, state)
            {
                for direction in Direction::ALL {
                    let neighbour_pos = rel(pos, direction);
                    let neighbour = self.state(neighbour_pos);
                    if b.is_block(neighbour, block)
                        && !self.frosted_slightly_melt(neighbour_pos, neighbour)
                    {
                        let delay = self
                            .random()
                            .next_int_between_inclusive(20, 40)
                            .unwrap_or(20);
                        self.schedule_tick(neighbour_pos, block, delay);
                    }
                }
                return;
            }
        }
        let delay = self
            .random()
            .next_int_between_inclusive(20, 40)
            .unwrap_or(20);
        self.schedule_tick(pos, block, delay);
    }
    fn frosted_slightly_melt(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        let b = self.b;
        let age = b.table.int(state, b.props.age);
        if age < 3 {
            self.set_block(pos, b.table.set_int(state, b.props.age, age + 1), 2);
            false
        } else {
            self.ice_melt(pos, state);
            true
        }
    }

    // ---- crops and plants ----

    /// `CropBlock.getGrowthSpeed`.
    pub(crate) fn crop_growth_speed(&self, block: super::states::BlockId, pos: BlockPos) -> f32 {
        let b = self.b;
        let mut speed = 1.0f32;
        let under = below(pos);
        for xx in -1..=1 {
            for zz in -1..=1 {
                let mut block_speed = 0.0f32;
                let state = self.state(under.offset(xx, 0, zz));
                if b.in_set(state, &b.tags.grows_crops) {
                    block_speed = 1.0;
                    if b.table.int(state, b.props.moisture) > 0 {
                        block_speed = 3.0;
                    }
                }
                if xx != 0 || zz != 0 {
                    block_speed /= 4.0;
                }
                speed += block_speed;
            }
        }
        let is = |at: BlockPos| b.is_block(self.state(at), block);
        let (n, s, w, e) = (
            rel(pos, Direction::North),
            rel(pos, Direction::South),
            rel(pos, Direction::West),
            rel(pos, Direction::East),
        );
        let horizontal = is(w) || is(e);
        let vertical = is(n) || is(s);
        let diagonal = is(rel(w, Direction::North))
            || is(rel(e, Direction::North))
            || is(rel(e, Direction::South))
            || is(rel(w, Direction::South));
        if (horizontal && vertical) || diagonal {
            speed /= 2.0;
        }
        speed
    }
    fn crop_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        if self.raw_brightness(pos, 0) < 9 {
            return;
        }
        let age = b.table.int(state, b.props.age);
        let max = self.crop_max_age(state);
        if age < max {
            let speed = self.crop_growth_speed(b.block(state), pos);
            if self.rand_int((25.0 / speed) as i32 + 1) == 0 {
                let next = self.crop_state_for_age(state, age + 1);
                self.set_block(pos, next, 2);
            }
        }
    }
    fn stem_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        if self.raw_brightness(pos, 0) < 9 {
            return;
        }
        let speed = self.crop_growth_speed(b.block(state), pos);
        if self.rand_int((25.0 / speed) as i32 + 1) != 0 {
            return;
        }
        let age = b.table.int(state, b.props.age);
        if age < 7 {
            self.set_block(pos, b.table.set_int(state, b.props.age, age + 1), 2);
            return;
        }
        let direction = HORIZONTAL[self.rand_int(4) as usize];
        let relative = rel(pos, direction);
        let data = b.data(state);
        let below_state = self.below_state(relative);
        if b.is_air(self.state(relative))
            && data
                .support_tag2
                .as_ref()
                .is_some_and(|tag| b.in_set(below_state, tag))
        {
            if let (Some(fruit), Some(stem)) = (data.link, data.link2) {
                self.set_block_and_update(relative, b.default_of(fruit));
                self.set_block_and_update(
                    pos,
                    b.table
                        .set_dir(b.default_of(stem), b.props.facing, direction),
                );
            }
        }
    }
    /// `PitcherCropBlock.grow`.
    pub(crate) fn pitcher_grow(&mut self, state: BlockStateId, pos: BlockPos, increase: i32) {
        let b = self.b;
        let p = b.props;
        let (lower_pos, lower) = if b.table.text(state, p.half) == "upper" {
            let below_pos = below(pos);
            let below_state = self.state(below_pos);
            if !b.is_block(below_state, b.named.pitcher_crop)
                || b.table.text(below_state, p.half) != "lower"
            {
                return;
            }
            (below_pos, below_state)
        } else {
            (pos, state)
        };
        let new_age = b.table.int(lower, p.age) + increase;
        let can_grow = b.table.int(lower, p.age) < 4
            && self.raw_brightness(lower_pos, 0) >= 8
            && self.is_inside_build_height(lower_pos.y + 1)
            && (new_age < 3 || {
                let upper = self.state(above(lower_pos));
                b.is_air(upper) || b.is_block(upper, b.named.pitcher_crop)
            });
        if !can_grow {
            return;
        }
        let grown = b.table.set_int(lower, p.age, new_age);
        self.set_block(lower_pos, grown, 2);
        if new_age >= 3 {
            let upper_pos = above(lower_pos);
            let upper = b.table.set_text(grown, p.half, "upper");
            let upper = b
                .table
                .with_bool(upper, p.waterlogged, self.is_water_at(upper_pos))
                .unwrap_or(upper);
            self.set_block(upper_pos, upper, 3);
        }
    }
    pub(crate) fn growing_plant_can_grow_into(
        &self,
        plant: BlockStateId,
        state: BlockStateId,
    ) -> bool {
        let b = self.b;
        let data = b.data(plant);
        if data.is(Class::Kelp) {
            // KelpBlock.canGrowInto: any water block state.
            b.is_block(state, b.named.water)
        } else {
            // NetherVines.isValidGrowthState / CaveVines: air only.
            b.is_air(state)
        }
    }
    fn growing_plant_grow_into_state(&mut self, state: BlockStateId) -> BlockStateId {
        let b = self.b;
        let next = b.table.cycle(state, b.props.age);
        if b.instance_of(state, Class::CaveVines) {
            let berries = self.random().next_float() < 0.11;
            return b.table.set_bool(next, b.props.berries, berries);
        }
        next
    }
    fn cactus_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let block = b.block(state);
        let above_pos = above(pos);
        if !self.is_empty_block(above_pos) {
            return;
        }
        let mut height = 1;
        let age = b.table.int(state, b.props.age);
        while b.is_block(self.state(pos.offset(0, -height, 0)), block) {
            height += 1;
            if height == 3 && age == 15 {
                return;
            }
        }
        if age == 8 && self.can_survive(b.default_of(block), above_pos) {
            let chance = if height >= 3 { 0.25 } else { 0.1 };
            if self.random().next_double() <= chance {
                self.set_block_and_update(above_pos, b.default_of(b.named.cactus_flower));
            }
        } else if age == 15 && height < 3 {
            self.set_block_and_update(above_pos, b.default_of(block));
            let reset = b.table.set_int(state, b.props.age, 0);
            self.set_block(pos, reset, 260);
            self.neighbor_changed_full(reset, above_pos, block, false);
        }
        if age < 15 {
            self.set_block(pos, b.table.set_int(state, b.props.age, age + 1), 260);
        }
    }
    fn farmland_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let moisture = b.table.int(state, b.props.moisture);
        let mut near_water = false;
        'search: for x in -4..=4 {
            for y in 0..=1 {
                for z in -4..=4 {
                    if b.is_water_fluid(self.fluid_at(pos.offset(x, y, z)).kind()) {
                        near_water = true;
                        break 'search;
                    }
                }
            }
        }
        if !near_water && !self.level.is_raining_at(above(pos)) {
            if moisture > 0 {
                self.set_block(
                    pos,
                    b.table.set_int(state, b.props.moisture, moisture - 1),
                    2,
                );
            } else if !b.in_set(self.above_state(pos), &b.tags.maintains_farmland) {
                self.turn_to_base_block(pos, state);
            }
        } else if moisture < 7 {
            self.set_block(pos, b.table.set_int(state, b.props.moisture, 7), 2);
        }
    }
    /// `SaplingBlock.advanceTree`.
    pub(crate) fn sapling_advance_tree(&mut self, pos: BlockPos, state: BlockStateId) {
        self.advance_tree(pos, state);
    }
    fn mushroom_random_tick(&mut self, state: BlockStateId, mut pos: BlockPos) {
        let b = self.b;
        if self.rand_int(25) != 0 {
            return;
        }
        // canSpread: at most four of this mushroom within a 9x3x9 box.
        let block = b.block(state);
        let mut max = 5;
        for x in -4..=4 {
            for y in -1..=1 {
                for z in -4..=4 {
                    if b.is_block(self.state(pos.offset(x, y, z)), block) {
                        max -= 1;
                        if max <= 0 {
                            return;
                        }
                    }
                }
            }
        }
        let mut offset = self.mushroom_offset(pos);
        for _ in 0..4 {
            if self.is_empty_block(offset) && self.can_survive(state, offset) {
                pos = offset;
            }
            offset = self.mushroom_offset(pos);
        }
        if self.is_empty_block(offset) && self.can_survive(state, offset) {
            self.set_block(offset, state, 2);
        }
    }
    fn mushroom_offset(&mut self, pos: BlockPos) -> BlockPos {
        let x = self.rand_int(3) - 1;
        let y = self.rand_int(2) - self.rand_int(2);
        let z = self.rand_int(3) - 1;
        pos.offset(x, y, z)
    }
    /// `LightEngine.getLightDampeningInto(state, above, UP, above.dampening) < 15`.
    fn top_face_lets_light(&self, state: BlockStateId, above_state: BlockStateId) -> bool {
        let b = self.b;
        let occluding = |s: BlockStateId| {
            b.has_flag(s, state_flags::CAN_OCCLUDE)
                && b.has_flag(s, super::data::state_flags::SHAPE_OCCLUSION)
        };
        let dampening = i32::from(b.facts(above_state).dampening);
        let from_full = occluding(state) && b.collision_face_full(state, Direction::Up);
        let to_full = occluding(above_state) && b.collision_face_full(above_state, Direction::Down);
        let into = if !occluding(state) && !occluding(above_state) {
            dampening
        } else if from_full || to_full {
            16
        } else {
            dampening
        };
        into < 15
    }
    fn can_be_nylium(&self, state: BlockStateId, pos: BlockPos) -> bool {
        self.top_face_lets_light(state, self.above_state(pos))
    }
    fn snowy_can_stay_alive(&self, state: BlockStateId, pos: BlockPos) -> bool {
        let b = self.b;
        let above_state = self.above_state(pos);
        if b.is_block(above_state, b.named.snow) && b.table.int(above_state, b.props.layers) == 1 {
            return true;
        }
        if b.fluid(above_state).is_full() {
            return false;
        }
        self.top_face_lets_light(state, above_state)
    }
    fn spreading_snowy_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let base = b.data(state).link.unwrap_or(b.named.dirt);
        if !self.snowy_can_stay_alive(state, pos) {
            self.set_block_and_update(pos, b.default_of(base));
            return;
        }
        if self.max_local_raw_brightness(above(pos)) < 9 {
            return;
        }
        let default = b.default_of(b.block(state));
        for _ in 0..4 {
            let x = self.rand_int(3) - 1;
            let y = self.rand_int(5) - 3;
            let z = self.rand_int(3) - 1;
            let test = pos.offset(x, y, z);
            if b.is_block(self.state(test), base)
                && self.snowy_can_stay_alive(default, test)
                && !b.is_water_fluid(self.fluid_at(above(test)).kind())
            {
                let snowy = b.in_set(self.above_state(test), &b.tags.snow);
                self.set_block_and_update(test, b.table.set_bool(default, b.props.snowy, snowy));
            }
        }
    }
    fn budding_amethyst_tick(&mut self, pos: BlockPos) {
        let b = self.b;
        if self.rand_int(5) != 0 {
            return;
        }
        let direction = Direction::ALL[self.rand_int(6) as usize];
        let grow = rel(pos, direction);
        let relative = self.state(grow);
        let name = |n: &str| b.table.block_by_name(n);
        let stages = [
            "minecraft:small_amethyst_bud",
            "minecraft:medium_amethyst_bud",
            "minecraft:large_amethyst_bud",
            "minecraft:amethyst_cluster",
        ];
        let fluid = b.fluid(relative);
        let can_cluster_grow =
            b.is_air(relative) || (b.is_block(relative, b.named.water) && fluid.amount() == 8);
        let next = if can_cluster_grow {
            name(stages[0])
        } else {
            let mut found = None;
            for i in 0..3 {
                if name(stages[i]).is_some_and(|s| b.is_block(relative, s))
                    && b.facing(relative) == direction
                {
                    found = name(stages[i + 1]);
                }
            }
            found
        };
        if let Some(next) = next {
            let state = b
                .table
                .set_dir(b.default_of(next), b.props.facing, direction);
            let state = b.table.set_bool(
                state,
                b.props.waterlogged,
                fluid == crate::fluid::FluidState::WATER,
            );
            self.set_block_and_update(grow, state);
        }
    }
    pub(crate) fn bamboo_sapling_grow(&mut self, pos: BlockPos) {
        let b = self.b;
        let bamboo = b.default_of(b.named.bamboo);
        let state = b.table.set_text(bamboo, b.props.leaves, "small");
        self.set_block(above(pos), state, 3);
    }
    pub(crate) fn bamboo_height_below(&self, pos: BlockPos) -> i32 {
        let mut height = 0;
        while height < 16
            && self.b.is_block(
                self.state(pos.offset(0, -(height + 1), 0)),
                self.b.named.bamboo,
            )
        {
            height += 1;
        }
        height
    }
    pub(crate) fn bamboo_height_above(&self, pos: BlockPos) -> i32 {
        let mut height = 0;
        while height < 16
            && self.b.is_block(
                self.state(pos.offset(0, height + 1, 0)),
                self.b.named.bamboo,
            )
        {
            height += 1;
        }
        height
    }
    /// `BambooStalkBlock.growBamboo`.
    pub(crate) fn bamboo_grow(&mut self, state: BlockStateId, pos: BlockPos, height: i32) {
        let b = self.b;
        let p = b.props;
        let below_state = self.below_state(pos);
        let below_two = pos.offset(0, -2, 0);
        let below_two_state = self.state(below_two);
        let mut leaves = "none";
        if height >= 1 {
            if !b.is_block(below_state, b.named.bamboo)
                || b.table.text(below_state, p.leaves) == "none"
            {
                leaves = "small";
            } else if b.is_block(below_state, b.named.bamboo)
                && b.table.text(below_state, p.leaves) != "none"
            {
                leaves = "large";
                if b.is_block(below_two_state, b.named.bamboo) {
                    self.set_block(
                        below(pos),
                        b.table.set_text(below_state, p.leaves, "small"),
                        3,
                    );
                    self.set_block(
                        below_two,
                        b.table.set_text(below_two_state, p.leaves, "none"),
                        3,
                    );
                }
            }
        }
        let age = if b.table.int(state, p.age) != 1 && !b.is_block(below_two_state, b.named.bamboo)
        {
            0
        } else {
            1
        };
        let stage = if (height < 11 || self.random().next_float() >= 0.25) && height != 15 {
            0
        } else {
            1
        };
        let next = b.table.set_int(
            b.table.set_int(
                b.table
                    .set_text(b.default_of(b.named.bamboo), p.leaves, leaves),
                p.age,
                age,
            ),
            p.stage,
            stage,
        );
        let _ = self.bamboo_height_above(pos);
        self.set_block(above(pos), next, 3);
    }
    /// `VineBlock.randomTick`.
    fn vine_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let p = b.props;
        let block = b.block(state);
        // The spread_vines rule, checked before the roll.
        if !self.level.game_rules().spread_vines || self.rand_int(4) != 0 {
            return;
        }
        let direction = Direction::ALL[self.rand_int(6) as usize];
        let above_pos = above(pos);
        let default = b.default_of(block);
        if super::query::is_horizontal(direction)
            && !b.table.bool(state, self.direction_prop(direction))
        {
            if !self.vine_can_spread(pos, block) {
                return;
            }
            let test = rel(pos, direction);
            let edge = self.state(test);
            if b.is_air(edge) {
                let cw = super::query::clockwise(direction);
                let ccw = super::query::counter_clockwise(direction);
                let cw_face = b.table.bool(state, self.direction_prop(cw));
                let ccw_face = b.table.bool(state, self.direction_prop(ccw));
                let cw_test = rel(test, cw);
                let ccw_test = rel(test, ccw);
                if cw_face && b.multiface_can_attach(self.state(cw_test), cw) {
                    self.set_block(
                        test,
                        b.table.set_bool(default, self.direction_prop(cw), true),
                        2,
                    );
                } else if ccw_face && b.multiface_can_attach(self.state(ccw_test), ccw) {
                    self.set_block(
                        test,
                        b.table.set_bool(default, self.direction_prop(ccw), true),
                        2,
                    );
                } else {
                    let opposite = direction.opposite();
                    if cw_face
                        && self.is_empty_block(cw_test)
                        && b.multiface_can_attach(self.rel_state(pos, cw), opposite)
                    {
                        self.set_block(
                            cw_test,
                            b.table
                                .set_bool(default, self.direction_prop(opposite), true),
                            2,
                        );
                    } else if ccw_face
                        && self.is_empty_block(ccw_test)
                        && b.multiface_can_attach(self.rel_state(pos, ccw), opposite)
                    {
                        self.set_block(
                            ccw_test,
                            b.table
                                .set_bool(default, self.direction_prop(opposite), true),
                            2,
                        );
                    } else if self.random().next_float() < 0.05
                        && b.multiface_can_attach(self.state(above(test)), Direction::Up)
                    {
                        self.set_block(test, b.table.set_bool(default, p.up, true), 2);
                    }
                }
            } else if b.multiface_can_attach(edge, direction) {
                self.set_block(
                    pos,
                    b.table
                        .set_bool(state, self.direction_prop(direction), true),
                    2,
                );
            }
            return;
        }
        if direction == Direction::Up && pos.y < self.level.max_y() - 1 {
            if b.multiface_can_attach(self.state(above_pos), Direction::Up) {
                self.set_block(pos, b.table.set_bool(state, p.up, true), 2);
                return;
            }
            if self.is_empty_block(above_pos) {
                if !self.vine_can_spread(pos, block) {
                    return;
                }
                let mut above_state = state;
                for d in HORIZONTAL {
                    if self.random().next_boolean()
                        || !b.multiface_can_attach(self.state(rel(above_pos, d)), d)
                    {
                        above_state = b.table.set_bool(above_state, self.direction_prop(d), false);
                    }
                }
                if HORIZONTAL
                    .into_iter()
                    .any(|d| b.table.bool(above_state, self.direction_prop(d)))
                {
                    self.set_block(above_pos, above_state, 2);
                }
                return;
            }
        }
        if pos.y > self.level.min_y() {
            let below_pos = below(pos);
            let below_state = self.state(below_pos);
            if b.is_air(below_state) || b.is_block(below_state, block) {
                let before = if b.is_air(below_state) {
                    default
                } else {
                    below_state
                };
                let mut after = before;
                for d in HORIZONTAL {
                    if self.random().next_boolean() && b.table.bool(state, self.direction_prop(d)) {
                        after = b.table.set_bool(after, self.direction_prop(d), true);
                    }
                }
                if before != after
                    && HORIZONTAL
                        .into_iter()
                        .any(|d| b.table.bool(after, self.direction_prop(d)))
                {
                    self.set_block(below_pos, after, 2);
                }
            }
        }
    }
    fn vine_can_spread(&self, pos: BlockPos, block: super::states::BlockId) -> bool {
        let mut max = 5;
        for x in -4..=4 {
            for y in -1..=1 {
                for z in -4..=4 {
                    if self.b.is_block(self.state(pos.offset(x, y, z)), block) {
                        max -= 1;
                        if max <= 0 {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
    /// `ChorusFlowerBlock.randomTick`.
    fn chorus_flower_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let p = b.props;
        let block = b.block(state);
        let plant = b.named.chorus_plant;
        let above_pos = above(pos);
        if !self.is_empty_block(above_pos) || above_pos.y > self.level.max_y() - 1 {
            return;
        }
        let age = b.table.int(state, p.age);
        if age >= 5 {
            return;
        }
        let mut grow_up = false;
        let mut pillar_on_support = false;
        let below_state = self.below_state(pos);
        if b.in_set(below_state, &b.tags.supports_chorus_flower) {
            grow_up = true;
        } else if b.is_block(below_state, plant) {
            let mut height = 1;
            for _ in 0..4 {
                let test = self.state(pos.offset(0, -(height + 1), 0));
                if !b.is_block(test, plant) {
                    if b.in_set(test, &b.tags.supports_chorus_flower) {
                        pillar_on_support = true;
                    }
                    break;
                }
                height += 1;
            }
            if height < 2 || height <= self.rand_int(if pillar_on_support { 5 } else { 4 }) {
                grow_up = true;
            }
        } else if b.is_air(below_state) {
            grow_up = true;
        }
        let all_empty = |sim: &Self, at: BlockPos, ignore: Option<Direction>| {
            HORIZONTAL
                .into_iter()
                .all(|d| Some(d) == ignore || sim.is_empty_block(rel(at, d)))
        };
        let plant_state = b.default_of(plant);
        if grow_up && all_empty(self, above_pos, None) && self.is_empty_block(pos.offset(0, 2, 0)) {
            let connected = self.chorus_plant_with_connections(pos, plant_state);
            self.set_block(pos, connected, 2);
            self.set_block(
                above_pos,
                b.table.set_int(b.default_of(block), p.age, age),
                2,
            );
            self.level_event(1033, above_pos, 0);
        } else if age < 4 {
            let mut attempts = self.rand_int(4);
            if pillar_on_support {
                attempts += 1;
            }
            let mut branched = false;
            for _ in 0..attempts {
                let direction = HORIZONTAL[self.rand_int(4) as usize];
                let target = rel(pos, direction);
                if self.is_empty_block(target)
                    && self.is_empty_block(below(target))
                    && all_empty(self, target, Some(direction.opposite()))
                {
                    self.set_block(
                        target,
                        b.table.set_int(b.default_of(block), p.age, age + 1),
                        2,
                    );
                    self.level_event(1033, target, 0);
                    branched = true;
                }
            }
            if branched {
                let connected = self.chorus_plant_with_connections(pos, plant_state);
                self.set_block(pos, connected, 2);
            } else {
                self.set_block(pos, b.table.set_int(state, p.age, 5), 2);
                self.level_event(1034, pos, 0);
            }
        } else {
            self.set_block(pos, b.table.set_int(state, p.age, 5), 2);
            self.level_event(1034, pos, 0);
        }
    }
    /// `ServerLevel.tickPrecipitation` at a column whose top (motion-blocking
    /// heightmap) position and biome answers are supplied by the caller.
    pub(crate) fn precipitation_tick(&mut self, info: Precipitation) {
        let b = self.b;
        let p = b.props;
        let top = info.top;
        let below_pos = below(top);
        if info.freezes_below {
            self.set_block_and_update(below_pos, b.default_of(b.named.ice));
        }
        if !self.level.is_raining() {
            return;
        }
        let max_height = self.level.game_rules().max_snow_accumulation_height;
        if max_height > 0 && info.snows_at_top {
            let state = self.state(top);
            if b.is_block(state, b.named.snow) {
                let layers = b.table.int(state, p.layers);
                if layers < max_height.min(8) {
                    self.set_block_and_update(top, b.table.set_int(state, p.layers, layers + 1));
                }
            } else {
                self.set_block_and_update(top, b.default_of(b.named.snow));
            }
        }
        if info.kind != PrecipitationKind::None {
            let below_state = self.state(below_pos);
            let data = b.data(below_state);
            let chance = match info.kind {
                PrecipitationKind::Rain => 0.05,
                PrecipitationKind::Snow => 0.1,
                PrecipitationKind::None => 0.0,
            };
            if data.is(Class::Cauldron) {
                if self.random().next_float() < chance {
                    let next = if info.kind == PrecipitationKind::Rain {
                        b.named.water_cauldron
                    } else {
                        b.named.powder_snow_cauldron
                    };
                    self.set_block_and_update(below_pos, b.default_of(next));
                    self.game_event(below_pos, "minecraft:block_change");
                }
            } else if data.is(Class::LayeredCauldron) && self.random().next_float() < chance {
                let matches = (info.kind == PrecipitationKind::Rain
                    && b.is_block(below_state, b.named.water_cauldron))
                    || (info.kind == PrecipitationKind::Snow
                        && b.is_block(below_state, b.named.powder_snow_cauldron));
                if b.table.int(below_state, p.level) != 3 && matches {
                    self.set_block_and_update(below_pos, b.table.cycle(below_state, p.level));
                    self.game_event(below_pos, "minecraft:block_change");
                }
            }
        }
    }
    /// `ChangeOverTimeBlock.changeOverTime` for weathering copper.
    fn copper_change_over_time(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.random().next_float() >= 0.056_888_89 {
            return;
        }
        let b = self.b;
        let own = b.data(state).weather;
        let (mut same, mut older) = (0, 0);
        for x in -4i32..=4 {
            for y in -4i32..=4 {
                for z in -4i32..=4 {
                    if x.abs() + y.abs() + z.abs() > 4 || (x == 0 && y == 0 && z == 0) {
                        continue;
                    }
                    let other = self.state(pos.offset(x, y, z));
                    let data = b.data(other);
                    if !data.implements(super::classes::Interface::WeatheringCopper) {
                        continue;
                    }
                    match data.weather.cmp(&own) {
                        std::cmp::Ordering::Less => return,
                        std::cmp::Ordering::Greater => older += 1,
                        std::cmp::Ordering::Equal => same += 1,
                    }
                }
            }
        }
        let chance = (older + 1) as f32 / (older + same + 1) as f32;
        let modifier = if own == 0 { 0.75 } else { 1.0 };
        if self.random().next_float() < chance * chance * modifier {
            if let Some(next) = self.copper_next(state) {
                self.set_block_and_update(pos, next);
            }
        }
    }
    /// `WeatheringCopper.getNext` by registry naming.
    pub(crate) fn copper_next(&self, state: BlockStateId) -> Option<BlockStateId> {
        let b = self.b;
        let name = &*b.table.info(b.block(state)).name;
        let path = name.strip_prefix("minecraft:")?;
        let next = if path == "copper_block" {
            "exposed_copper".to_owned()
        } else if let Some(rest) = path.strip_prefix("exposed_") {
            format!("weathered_{rest}")
        } else if let Some(rest) = path.strip_prefix("weathered_") {
            format!("oxidized_{rest}")
        } else if path.starts_with("oxidized_") || path.starts_with("waxed_") {
            return None;
        } else {
            format!("exposed_{path}")
        };
        let block = b.table.block_by_name(&format!("minecraft:{next}"))?;
        Some(b.table.with_properties_of(block, state))
    }
    /// `WeatheringCopper.getPrevious` (and `getFirst` by repeating it) by
    /// registry naming: the block one oxidation step back.
    fn copper_previous_name(path: &str) -> Option<String> {
        if let Some(rest) = path.strip_prefix("oxidized_") {
            Some(format!("weathered_{rest}"))
        } else if let Some(rest) = path.strip_prefix("weathered_") {
            Some(format!("exposed_{rest}"))
        } else {
            path.strip_prefix("exposed_").map(|rest| {
                if rest == "copper" {
                    "copper_block".to_owned()
                } else {
                    rest.to_owned()
                }
            })
        }
    }
    fn copper_stepped(&self, state: BlockStateId, all_the_way: bool) -> Option<BlockStateId> {
        let b = self.b;
        let name = &*b.table.info(b.block(state)).name;
        let mut path = Self::copper_previous_name(name.strip_prefix("minecraft:")?)?;
        while all_the_way && let Some(earlier) = Self::copper_previous_name(&path) {
            path = earlier;
        }
        let block = b.table.block_by_name(&format!("minecraft:{path}"))?;
        Some(b.table.with_properties_of(block, state))
    }
    /// `LightningBolt.clearCopperOnLightningStrike`: the struck copper goes
    /// back to unoxidized, then 3 to 5 random walks of up to 8 steps each
    /// scrape one step of oxidation off the weathering copper they land on.
    /// A waxed block is left as it is but still starts the walks.
    pub(crate) fn lightning_clear_copper(&mut self, struck: BlockPos) {
        let b = self.b;
        let state = self.state(struck);
        let weathering = b
            .data(state)
            .implements(super::classes::Interface::WeatheringCopper);
        let waxed = b.table.name(state).starts_with("minecraft:waxed_");
        if !(weathering || waxed) {
            return;
        }
        if weathering && let Some(first) = self.copper_stepped(state, true) {
            self.set_block_and_update(struck, first);
        }
        for _ in 0..self.rand_int(3) + 3 {
            let mut at = struck;
            for _ in 0..self.rand_int(8) + 1 {
                // randomStepCleaningCopper: up to 10 random blocks in the
                // 3x3x3 cube around the walk's position.
                let mut next = None;
                for _ in 0..10 {
                    let x = at.x - 1 + self.rand_int(3);
                    let y = at.y - 1 + self.rand_int(3);
                    let z = at.z - 1 + self.rand_int(3);
                    let candidate = BlockPos::new(x, y, z);
                    let state = self.state(candidate);
                    if b.data(state)
                        .implements(super::classes::Interface::WeatheringCopper)
                    {
                        if let Some(previous) = self.copper_stepped(state, false) {
                            self.set_block_and_update(candidate, previous);
                        }
                        self.level_event(3002, candidate, -1);
                        next = Some(candidate);
                        break;
                    }
                }
                match next {
                    Some(p) => at = p,
                    None => break,
                }
            }
        }
    }
}

/// `Biome.Precipitation` at a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrecipitationKind {
    None,
    Rain,
    Snow,
}
/// Biome answers for one precipitation tick (`ServerLevel.tickPrecipitation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Precipitation {
    /// `getHeightmapPos(MOTION_BLOCKING, pos)`.
    pub top: BlockPos,
    /// `biome.shouldFreeze(level, top.below())`.
    pub freezes_below: bool,
    /// `biome.shouldSnow(level, top)`.
    pub snows_at_top: bool,
    /// `biome.getPrecipitationAt(top.below(), seaLevel)`.
    pub kind: PrecipitationKind,
}

impl Behaviours {
    /// Run due scheduled ticks for the level's current game time, in vanilla
    /// order: block ticks then fluid ticks (65536 each at most).
    pub fn tick_scheduled<L: LevelAccess + ?Sized>(&self, level: &mut L, ticks: &mut Ticks) {
        let time = level.game_time();
        ticks.handling_tick = true;
        {
            let level_ref: &L = level;
            if ticks
                .blocks
                .begin_tick(time, 65536, |chunk: ChunkPos| {
                    level_ref.is_ticking_chunk(chunk)
                })
                .is_err()
            {
                ticks.handling_tick = false;
                return;
            }
        }
        while let Ok(Some(tick)) = ticks.blocks.next_tick() {
            let mut sim = self.sim(level, ticks);
            let state = sim.state(tick.pos);
            if u32::from(self.block(state).0) == tick.kind.0 {
                sim.tick_due_block(state, tick.pos);
            }
        }
        let _ = ticks.blocks.finish_tick();
        {
            let level_ref: &L = level;
            if ticks
                .fluids
                .begin_tick(time, 65536, |chunk: ChunkPos| {
                    level_ref.is_ticking_chunk(chunk)
                })
                .is_err()
            {
                ticks.handling_tick = false;
                return;
            }
        }
        while let Ok(Some(tick)) = ticks.fluids.next_tick() {
            let mut sim = self.sim(level, ticks);
            let fluid = self.fluid(sim.state(tick.pos)).kind();
            if TickTypeId(fluid as u32) == tick.kind {
                sim.fluid_tick(tick.pos);
            }
        }
        let _ = ticks.fluids.finish_tick();
        ticks.handling_tick = false;
    }
    /// `tick_scheduled` one tick at a time, for a caller that must act on
    /// each tick's side effects before the next runs (as vanilla runs them
    /// inline): `scheduled_begin`, then `scheduled_step` until it returns
    /// false, then `scheduled_finish`; block ticks first, then fluid ticks.
    /// Returns whether ticks are due (false: nothing to step or finish).
    pub fn scheduled_begin<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        fluids: bool,
    ) -> bool {
        let time = level.game_time();
        ticks.handling_tick = true;
        let level_ref: &L = level;
        let queue = if fluids {
            &mut ticks.fluids
        } else {
            &mut ticks.blocks
        };
        let ok = queue
            .begin_tick(time, 65536, |chunk: ChunkPos| {
                level_ref.is_ticking_chunk(chunk)
            })
            .is_ok();
        if !ok {
            ticks.handling_tick = false;
        }
        ok
    }
    /// Run the next due tick; false when none is left.
    pub fn scheduled_step<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        fluids: bool,
    ) -> bool {
        if fluids {
            let Ok(Some(tick)) = ticks.fluids.next_tick() else {
                return false;
            };
            let mut sim = self.sim(level, ticks);
            let fluid = self.fluid(sim.state(tick.pos)).kind();
            if TickTypeId(fluid as u32) == tick.kind {
                sim.fluid_tick(tick.pos);
            }
        } else {
            let Ok(Some(tick)) = ticks.blocks.next_tick() else {
                return false;
            };
            let mut sim = self.sim(level, ticks);
            let state = sim.state(tick.pos);
            if u32::from(self.block(state).0) == tick.kind.0 {
                sim.tick_due_block(state, tick.pos);
            }
        }
        true
    }
    /// End a `scheduled_begin` phase.
    pub fn scheduled_finish(&self, ticks: &mut Ticks, fluids: bool) {
        let _ = if fluids {
            ticks.fluids.finish_tick()
        } else {
            ticks.blocks.finish_tick()
        };
        ticks.handling_tick = false;
    }
    /// Random-tick one chunk (`ServerLevel.tickChunk` block part). Visits the
    /// sections from the bottom up and draws `randomTickSpeed` positions in
    /// each randomly ticking one (`LevelChunkSection.isRandomlyTicking`: it
    /// holds a randomly ticking block or fluid). `next_ticking(level, min_y)`
    /// is the minimum Y of the lowest such section from `min_y` up, as the
    /// chunk is now: vanilla asks as it reaches each section, so the ticks of
    /// the sections below count.
    pub fn tick_chunk_random<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        chunk: ChunkPos,
        mut next_ticking: impl FnMut(&mut L, i32) -> Option<i32>,
    ) {
        self.tick_chunk_random_masked(level, ticks, chunk, |level, from| {
            next_ticking(level, from).map(|min_y| (min_y, None))
        });
    }
    /// [`Self::tick_chunk_random`], where `next_ticking` may also give the
    /// section's [`RandomTickMask`]: a drawn position outside it is not read,
    /// as its random tick would do nothing. The draws are the same. After a
    /// position that did tick, the section is asked for again (its blocks may
    /// have changed): a mask is only trusted for the blocks as they are.
    pub fn tick_chunk_random_masked<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        chunk: ChunkPos,
        mut next_ticking: impl FnMut(&mut L, i32) -> Option<(i32, Option<Arc<RandomTickMask>>)>,
    ) {
        let speed = level.game_rules().random_tick_speed;
        if speed <= 0 {
            return;
        }
        let (min_x, min_z) = (chunk.x * 16, chunk.z * 16);
        for _ in 0..speed {
            if level.random().next_int_bound(48).unwrap_or(1) == 0 {
                let pos = ticks.block_random_pos(min_x, 0, min_z, 15);
                if let Some(info) = level.precipitation(pos) {
                    self.sim(level, ticks).precipitation_tick(info);
                }
            }
        }
        let mut from = level.min_y();
        while let Some((min_y, mut mask)) =
            next_ticking(level, from).filter(|&(y, _)| y < level.max_y())
        {
            for _ in 0..speed {
                let pos = ticks.block_random_pos(min_x, min_y, min_z, 15);
                if mask.as_ref().is_none_or(|m| m.may_tick(pos)) {
                    self.sim(level, ticks).random_tick_at(pos);
                    // The same section as it is now; one that stopped
                    // ticking still gets the rest of its draws, unfiltered.
                    mask = next_ticking(level, min_y)
                        .filter(|&(y, _)| y == min_y)
                        .and_then(|(_, m)| m);
                }
            }
            from = min_y + 16;
        }
    }
    /// Whether a random tick at a block of `state` can do anything: what
    /// [`Self::tick_chunk_random_masked`]'s masks hold. `random_tick_at`
    /// ticks a randomly ticking block, then lava's fluid.
    pub fn random_tick_candidate(&self, state: BlockStateId) -> bool {
        self.is_randomly_ticking(state)
            || matches!(
                self.fluid(state).kind(),
                FluidKind::Lava | FluidKind::FlowingLava
            )
    }
    /// Random-tick a single position (block then fluid), as `tickChunk` does.
    pub fn random_tick<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        self.sim(level, ticks).random_tick_at(pos);
    }
    /// `LightningBolt.clearCopperOnLightningStrike` for the block at `struck`.
    pub fn lightning_clear_copper<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        struck: BlockPos,
    ) {
        self.sim(level, ticks).lightning_clear_copper(struck);
    }
    /// Run one scheduled block tick at `pos` immediately (no queue check).
    pub fn scheduled_block_tick<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        sim.tick_block(state, pos);
    }
    /// Run one scheduled fluid tick at `pos` immediately.
    pub fn scheduled_fluid_tick<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        self.sim(level, ticks).fluid_tick(pos);
    }
}

/// Which of a section's 4,096 blocks a random tick can do anything for
/// ([`Behaviours::random_tick_candidate`]), one bit each in the section's
/// index order (`y << 8 | z << 4 | x`). Vanilla reads the block at every drawn
/// position; most hold stone, dirt or air, and the read is a cache miss.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RandomTickMask(pub [u64; 64]);
impl RandomTickMask {
    /// The mask of `blocks` (4,096 states in index order).
    pub fn of(blocks: &[u32], mut candidate: impl FnMut(u32) -> bool) -> Self {
        let mut bits = [0u64; 64];
        let mut last = None;
        for (i, &state) in blocks.iter().enumerate().take(4096) {
            let hit = match last {
                Some((s, hit)) if s == state => hit,
                _ => {
                    let hit = candidate(state);
                    last = Some((state, hit));
                    hit
                }
            };
            bits[i >> 6] |= u64::from(hit) << (i & 63);
        }
        Self(bits)
    }
    pub fn may_tick(&self, pos: BlockPos) -> bool {
        let i = ((pos.y & 15) << 8 | (pos.z & 15) << 4 | (pos.x & 15)) as usize;
        self.0[i >> 6] >> (i & 63) & 1 != 0
    }
}
