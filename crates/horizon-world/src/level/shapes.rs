//! `BlockBehaviour.updateShape` for every overriding vanilla class.
//!
//! Dispatch walks the block's class chain; a handler returning `None` defers
//! to the next class (the Java `super` call), the end of the chain returns the
//! state unchanged (`BlockBehaviour.updateShape`).
use super::{
    LevelAccess, Sim,
    classes::Class,
    query::{axis_of, clockwise, counter_clockwise, is_horizontal},
    states::BlockId,
    update::{above, below, rel},
};
use crate::{fluid::FluidKind, registry::BlockStateId};
use horizon_core::{
    geometry::{Axis, Direction},
    position::BlockPos,
    random::RandomSource,
};

/// `SoundEvents.GEYSER_ERUPTION_START`.
const GEYSER_ERUPTION_START: &str = "minecraft:block.potent_sulfur.geyser_eruption";
/// `SoundEvents.GEYSER_CONTINUOUS_START`.
const GEYSER_CONTINUOUS_START: &str = "minecraft:block.potent_sulfur.geyser_continuous_eruption";

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    /// `BlockState.updateShape(level, ticks, pos, direction, neighbourPos, neighbourState, random)`.
    pub fn update_shape(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
    ) -> BlockStateId {
        self.update_shape_at(0, state, pos, direction, neighbour_pos, neighbour)
    }

    pub(crate) fn schedule_water_tick(&mut self, pos: BlockPos) {
        self.schedule_fluid_tick(pos, FluidKind::Water, 5);
    }
    fn waterlogged_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        if self.b.is_waterlogged(state) {
            self.schedule_water_tick(pos);
        }
    }

    fn update_shape_at(
        &mut self,
        depth: usize,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
    ) -> BlockStateId {
        let chain_len = self.b.data(state).chain.len();
        for index in depth..chain_len {
            let class = self.b.data(state).chain[index];
            if let Some(result) = self.update_shape_class(
                class,
                index,
                state,
                pos,
                direction,
                neighbour_pos,
                neighbour,
            ) {
                return result;
            }
        }
        state
    }

    #[allow(clippy::too_many_arguments)]
    fn update_shape_class(
        &mut self,
        class: Class,
        depth: usize,
        state: BlockStateId,
        pos: BlockPos,
        dir: Direction,
        npos: BlockPos,
        nstate: BlockStateId,
    ) -> Option<BlockStateId> {
        let b = self.b;
        let p = b.props;
        let t = &b.tags;
        let n = b.named;
        let table = &b.table;
        let air = b.air();
        let block = b.block(state);
        macro_rules! sup {
            () => {
                self.update_shape_at(depth + 1, state, pos, dir, npos, nstate)
            };
        }
        // `dir == X && !canSurvive ? AIR : super`.
        macro_rules! survive_or_air {
            ($cond:expr) => {{
                if $cond && !self.can_survive(state, pos) {
                    air
                } else {
                    sup!()
                }
            }};
        }
        Some(match class {
            Class::AbstractBed => {
                if dir == self.bed_neighbour_direction(state) {
                    if b.is_block(nstate, block)
                        && table.text(nstate, p.part) != table.text(state, p.part)
                    {
                        table.set_bool(state, p.occupied, table.bool(nstate, p.occupied))
                    } else {
                        air
                    }
                } else {
                    sup!()
                }
            }
            Class::AmethystCluster | Class::ShelfMushroom => {
                if class == Class::AmethystCluster {
                    self.waterlogged_tick(state, pos);
                }
                survive_or_air!(dir == b.facing(state).opposite())
            }
            Class::AttachedStem => {
                if !b
                    .data(state)
                    .link
                    .is_some_and(|fruit| b.is_block(nstate, fruit))
                    && dir == b.facing(state)
                {
                    if let Some(stem) = b.data(state).link2 {
                        return Some(table.set_int(b.default_of(stem), p.age, 7));
                    }
                }
                sup!()
            }
            Class::BambooSapling => {
                if !self.can_survive(state, pos) {
                    air
                } else if dir == Direction::Up && b.is_block(nstate, n.bamboo) {
                    b.default_of(n.bamboo)
                } else {
                    sup!()
                }
            }
            Class::BambooStalk => {
                if !self.can_survive(state, pos) {
                    self.schedule_tick(pos, block, 1);
                }
                if dir == Direction::Up
                    && b.is_block(nstate, n.bamboo)
                    && table.int(nstate, p.age) > table.int(state, p.age)
                {
                    table.cycle(state, p.age)
                } else {
                    sup!()
                }
            }
            Class::Banner
            | Class::BasePressurePlate
            | Class::BaseTorch
            | Class::Cake
            | Class::CandleCake
            | Class::FlowerPot
            | Class::StandingSign => survive_or_air!(dir == Direction::Down),
            Class::Barrier
            | Class::BaseRail
            | Class::Candle
            | Class::Chain
            | Class::Conduit
            | Class::CopperGolemStatue
            | Class::DecoratedPot
            | Class::DriedGhast
            | Class::EnderChest
            | Class::HeavyCore
            | Class::Light
            | Class::LightningRod
            | Class::MangroveRoots
            | Class::SculkSensor
            | Class::SculkShrieker
            | Class::Shelf
            | Class::Sign
            | Class::Slab
            | Class::SmallDripleaf
            | Class::TrapDoor
            | Class::WaterloggedTransparent => {
                self.waterlogged_tick(state, pos);
                sup!()
            }
            Class::BaseCoralPlantType => {
                self.waterlogged_tick(state, pos);
                survive_or_air!(dir == Direction::Down)
            }
            Class::BaseCoralWallFan => {
                self.waterlogged_tick(state, pos);
                if dir.opposite() == b.facing(state) && !self.can_survive(state, pos) {
                    air
                } else {
                    state
                }
            }
            Class::Beehive => sup!(),
            Class::Bell => {
                let attachment = table.text(state, p.attachment);
                let connected = self.bell_connected_direction(state).opposite();
                if connected == dir && !self.can_survive(state, pos) && attachment != "double_wall"
                {
                    return Some(air);
                }
                if axis_of(dir) == axis_of(b.facing(state)) {
                    if attachment == "double_wall" && !b.is_face_sturdy(nstate, dir) {
                        let single = table.set_text(state, p.attachment, "single_wall");
                        return Some(table.set_dir(single, p.facing, dir.opposite()));
                    }
                    if attachment == "single_wall"
                        && connected.opposite() == dir
                        && b.is_face_sturdy(nstate, b.facing(state))
                    {
                        return Some(table.set_text(state, p.attachment, "double_wall"));
                    }
                }
                sup!()
            }
            Class::BigDripleaf => {
                if dir == Direction::Down && !self.can_survive(state, pos) {
                    return Some(air);
                }
                self.waterlogged_tick(state, pos);
                if dir == Direction::Up && b.is_block(nstate, block) {
                    table.with_properties_of(n.big_dripleaf_stem, state)
                } else {
                    sup!()
                }
            }
            Class::BigDripleafStem => {
                if matches!(dir, Direction::Down | Direction::Up) && !self.can_survive(state, pos) {
                    self.schedule_tick(pos, block, 1);
                }
                self.waterlogged_tick(state, pos);
                sup!()
            }
            Class::Brushable => {
                self.schedule_tick(pos, block, 2);
                sup!()
            }
            Class::BubbleColumn => {
                self.schedule_water_tick(pos);
                if !self.can_survive(state, pos)
                    || dir == Direction::Down
                    || (dir == Direction::Up
                        && !b.is_block(nstate, block)
                        && self.bubble_column_can_occupy(nstate))
                {
                    self.schedule_tick(pos, block, 5);
                }
                sup!()
            }
            Class::Cactus | Class::SugarCane => {
                if !self.can_survive(state, pos) {
                    self.schedule_tick(pos, block, 1);
                }
                sup!()
            }
            Class::Campfire => {
                self.waterlogged_tick(state, pos);
                if dir == Direction::Down {
                    let hay = table.block_by_name("minecraft:hay_block");
                    table.set_bool(
                        state,
                        p.signal_fire,
                        hay.is_some_and(|hay| b.is_block(nstate, hay)),
                    )
                } else {
                    sup!()
                }
            }
            Class::Carpet | Class::SnowLayer | Class::Vegetation | Class::Frogspawn => {
                survive_or_air!(true)
            }
            Class::SeaPickle => {
                if !self.can_survive(state, pos) {
                    return Some(air);
                }
                self.waterlogged_tick(state, pos);
                sup!()
            }
            Class::CeilingHangingSign | Class::SporeBlossom => {
                survive_or_air!(dir == Direction::Up)
            }
            Class::Chest => {
                self.waterlogged_tick(state, pos);
                if self.chest_can_connect_to(state, nstate) && is_horizontal(dir) {
                    let neighbour_type = table.text(nstate, p.kind);
                    if table.text(state, p.kind) == "single"
                        && neighbour_type != "single"
                        && b.facing(state) == b.facing(nstate)
                        && self.chest_connected_direction(nstate) == dir.opposite()
                    {
                        let opposite = if neighbour_type == "left" {
                            "right"
                        } else {
                            "left"
                        };
                        return Some(table.set_text(state, p.kind, opposite));
                    }
                } else if self.chest_connected_direction(state) == dir {
                    return Some(table.set_text(state, p.kind, "single"));
                }
                sup!()
            }
            Class::CopperChest => {
                let result = sup!();
                if self.chest_can_connect_to(state, nstate)
                    && table.text(result, p.kind) != "single"
                    && self.chest_connected_direction(result) == dir
                {
                    return Some(table.with_properties_of(b.block(nstate), result));
                }
                result
            }
            Class::ChorusFlower => {
                if dir != Direction::Up && !self.can_survive(state, pos) {
                    self.schedule_tick(pos, block, 1);
                }
                sup!()
            }
            Class::ChorusPlant => {
                if !self.can_survive(state, pos) {
                    self.schedule_tick(pos, block, 1);
                    sup!()
                } else {
                    let connect = b.is_block(nstate, block)
                        || b.is_block(nstate, n.chorus_flower)
                        || (dir == Direction::Down && b.in_set(nstate, &t.supports_chorus_plant));
                    table.set_bool(state, self.direction_prop(dir), connect)
                }
            }
            Class::Cocoa => survive_or_air!(dir == b.facing(state)),
            Class::Comparator => {
                if dir == Direction::Down && !b.is_face_sturdy_rigid(nstate, Direction::Up) {
                    air
                } else {
                    sup!()
                }
            }
            Class::ConcretePowder => {
                if self.concrete_touches_liquid(pos) {
                    b.data(state)
                        .link
                        .map_or(state, |concrete| b.default_of(concrete))
                } else {
                    sup!()
                }
            }
            Class::Coral => {
                if !self.coral_scan_for_water(pos) {
                    let delay = 60 + self.random().next_int_bound(40).unwrap_or(0);
                    self.schedule_tick(pos, block, delay);
                }
                sup!()
            }
            Class::CoralFan | Class::CoralPlant => {
                if dir == Direction::Down && !self.can_survive(state, pos) {
                    return Some(air);
                }
                self.coral_try_schedule_die_tick(state, pos);
                self.waterlogged_tick(state, pos);
                sup!()
            }
            Class::CoralWallFan => {
                if dir.opposite() == b.facing(state) && !self.can_survive(state, pos) {
                    return Some(air);
                }
                self.waterlogged_tick(state, pos);
                self.coral_try_schedule_die_tick(state, pos);
                sup!()
            }
            Class::CreakingHeart => {
                self.schedule_tick(pos, block, 1);
                sup!()
            }
            Class::Door => {
                let lower = table.text(state, p.half) == "lower";
                if dir.normal()[1] == 0 || lower != (dir == Direction::Up) {
                    if lower && dir == Direction::Down && !self.can_survive(state, pos) {
                        air
                    } else {
                        sup!()
                    }
                } else if b.instance_of(nstate, Class::Door)
                    && table.text(nstate, p.half) != table.text(state, p.half)
                {
                    table.set_text(nstate, p.half, table.text(state, p.half))
                } else {
                    air
                }
            }
            Class::DoublePlant => {
                let half = table.text(state, p.half);
                let lower = half == "lower";
                if dir.normal()[1] == 0
                    || lower != (dir == Direction::Up)
                    || (b.is_block(nstate, block) && table.text(nstate, p.half) != half)
                {
                    if lower && dir == Direction::Down && !self.can_survive(state, pos) {
                        air
                    } else {
                        sup!()
                    }
                } else {
                    air
                }
            }
            Class::PitcherCrop => {
                if table.int(state, p.age) >= 3 {
                    sup!()
                } else if self.can_survive(state, pos) {
                    state
                } else {
                    air
                }
            }
            Class::FaceAttachedHorizontalDirectional | Class::Lantern => {
                if class == Class::Lantern {
                    self.waterlogged_tick(state, pos);
                    let connected = if table.bool(state, p.hanging) {
                        Direction::Down
                    } else {
                        Direction::Up
                    };
                    survive_or_air!(connected.opposite() == dir)
                } else {
                    survive_or_air!(self.face_attached_connected(state).opposite() == dir)
                }
            }
            Class::FallingBlock => {
                let delay = if class_is(b, block, Class::DragonEgg) {
                    5
                } else {
                    2
                };
                self.schedule_tick(pos, block, delay);
                sup!()
            }
            Class::Farmland | Class::Path => {
                if dir == Direction::Up && !self.can_survive(state, pos) {
                    self.schedule_tick(pos, block, 1);
                }
                sup!()
            }
            Class::Fence => {
                self.waterlogged_tick(state, pos);
                if is_horizontal(dir) {
                    let sturdy = b.is_face_sturdy(nstate, dir.opposite());
                    table.set_bool(
                        state,
                        self.direction_prop(dir),
                        self.fence_connects_to(state, nstate, sturdy, dir.opposite()),
                    )
                } else {
                    sup!()
                }
            }
            Class::IronBars => {
                self.waterlogged_tick(state, pos);
                if is_horizontal(dir) {
                    let sturdy = b.is_face_sturdy(nstate, dir.opposite());
                    table.set_bool(
                        state,
                        self.direction_prop(dir),
                        self.bars_attach_to(nstate, sturdy),
                    )
                } else {
                    sup!()
                }
            }
            Class::FenceGate => {
                if axis_of(clockwise(b.facing(state))) != axis_of(dir) {
                    sup!()
                } else {
                    let other = self.rel_state(pos, dir.opposite());
                    let in_wall = b.in_set(nstate, &t.walls) || b.in_set(other, &t.walls);
                    table.set_bool(state, p.in_wall, in_wall)
                }
            }
            Class::Fire => {
                if self.can_survive(state, pos) {
                    let age = table.int(state, p.age);
                    self.fire_state_with_age(pos, age)
                } else {
                    air
                }
            }
            Class::SoulFire => {
                if self.can_survive(state, pos) {
                    b.default_of(block)
                } else {
                    air
                }
            }
            Class::GrowingPlantBody => {
                let growth = b.data(state).growth_direction;
                if dir == growth.opposite() && !self.can_survive(state, pos) {
                    self.schedule_tick(pos, block, 1);
                }
                let (head, _) = self.growing_plant_pair(block);
                if dir == growth && !b.is_block(nstate, block) && !b.is_block(nstate, head) {
                    let age = self.random().next_int_bound(25).unwrap_or(0);
                    let head_state = table.set_int(b.default_of(head), p.age, age);
                    return Some(if class_is(b, block, Class::CaveVinesPlant) {
                        table.set_bool(head_state, p.berries, table.bool(state, p.berries))
                    } else {
                        head_state
                    });
                }
                if b.data(state).schedule_fluid_ticks {
                    self.schedule_water_tick(pos);
                }
                sup!()
            }
            Class::GrowingPlantHead => {
                let growth = b.data(state).growth_direction;
                let (_, body) = self.growing_plant_pair(block);
                let to_body = |sim: &Self| {
                    let body_state = b.default_of(body);
                    if class_is(b, block, Class::CaveVines) {
                        sim.b
                            .table
                            .set_bool(body_state, p.berries, table.bool(state, p.berries))
                    } else {
                        body_state
                    }
                };
                if dir == growth.opposite() {
                    if !self.can_survive(state, pos) {
                        self.schedule_tick(pos, block, 1);
                    } else {
                        let next = self.rel_state(pos, growth);
                        if b.is_block(next, block) || b.is_block(next, body) {
                            return Some(to_body(self));
                        }
                    }
                }
                if dir != growth || (!b.is_block(nstate, block) && !b.is_block(nstate, body)) {
                    if b.data(state).schedule_fluid_ticks {
                        self.schedule_water_tick(pos);
                    }
                    sup!()
                } else {
                    to_body(self)
                }
            }
            Class::HangingMoss => {
                let above_state = self.above_state(pos);
                if !(b.multiface_can_attach(above_state, Direction::Up)
                    || b.is_block(above_state, block))
                {
                    self.schedule_tick(pos, block, 1);
                }
                table.set_bool(state, p.tip, !b.is_block(self.below_state(pos), block))
            }
            Class::HangingRoots => {
                if dir == Direction::Up && !self.can_survive(state, pos) {
                    return Some(air);
                }
                self.waterlogged_tick(state, pos);
                sup!()
            }
            Class::HugeMushroom => {
                if b.is_block(nstate, block) {
                    table.set_bool(state, self.direction_prop(dir), false)
                } else {
                    sup!()
                }
            }
            Class::Ladder => {
                if dir.opposite() == b.facing(state) && !self.can_survive(state, pos) {
                    return Some(air);
                }
                self.waterlogged_tick(state, pos);
                sup!()
            }
            Class::Leaves => {
                self.waterlogged_tick(state, pos);
                let distance = self.leaf_distance_at(nstate) + 1;
                if distance != 1 || table.int(state, p.distance) != distance {
                    self.schedule_tick(pos, block, 1);
                }
                state
            }
            Class::Liquid => {
                let fluid = b.fluid(state);
                if fluid.is_source() || b.fluid(nstate).is_source() {
                    let delay = self.fluid_tick_delay(fluid.kind());
                    self.schedule_fluid_tick(pos, fluid.kind(), delay);
                }
                if dir == Direction::Down && self.should_bubble_column_occupy(state) {
                    self.try_schedule_bubble_column(pos, nstate, block);
                }
                sup!()
            }
            Class::MangrovePropagule => {
                self.waterlogged_tick(state, pos);
                survive_or_air!(dir == Direction::Up)
            }
            Class::MossyCarpet => {
                if !self.can_survive(state, pos) {
                    return Some(air);
                }
                let updated = self.mossy_carpet_updated_state(state, pos, false);
                if self.mossy_carpet_has_faces(updated) {
                    updated
                } else {
                    air
                }
            }
            Class::Multiface => {
                self.waterlogged_tick(state, pos);
                if !Direction::ALL
                    .into_iter()
                    .any(|d| self.multiface_has_face(state, d))
                {
                    air
                } else if self.multiface_has_face(state, dir)
                    && !b.multiface_can_attach(nstate, dir)
                {
                    let removed = table.set_bool(state, self.direction_prop(dir), false);
                    if Direction::ALL
                        .into_iter()
                        .any(|d| self.multiface_has_face(removed, d))
                    {
                        removed
                    } else {
                        air
                    }
                } else {
                    state
                }
            }
            Class::PistonHead => survive_or_air!(dir.opposite() == b.facing(state)),
            Class::Note => {
                if dir.normal()[1] != 0 {
                    self.note_block_instrument(pos, state)
                } else {
                    sup!()
                }
            }
            Class::Observer => {
                if b.facing(state) == dir && !table.bool(state, p.powered) {
                    self.observer_start_signal(pos, block);
                }
                sup!()
            }
            Class::RedstoneWallTorch
            | Class::WallTorch
            | Class::TripWireHook
            | Class::WallBanner
            | Class::WallSign => {
                let attached = dir.opposite() == b.facing(state);
                let fails = attached && !self.can_survive(state, pos);
                match class {
                    Class::RedstoneWallTorch | Class::WallTorch => {
                        if fails {
                            air
                        } else {
                            state
                        }
                    }
                    _ => {
                        if fails {
                            air
                        } else {
                            sup!()
                        }
                    }
                }
            }
            Class::RedstoneWire => self.redstone_wire_update_shape(state, pos, dir, npos, nstate),
            Class::Repeater => {
                if dir == Direction::Down && !b.is_face_sturdy_rigid(nstate, Direction::Up) {
                    air
                } else if axis_of(dir) != axis_of(b.facing(state)) {
                    let locked = self.diode_is_locked(pos, state);
                    table.set_bool(state, p.locked, locked)
                } else {
                    sup!()
                }
            }
            Class::Scaffolding => {
                self.waterlogged_tick(state, pos);
                self.schedule_tick(pos, block, 1);
                state
            }
            Class::Seagrass => {
                let result = sup!();
                if !b.is_air(result) {
                    self.schedule_water_tick(pos);
                }
                result
            }
            Class::Snowy => {
                if dir == Direction::Up {
                    table.set_bool(state, p.snowy, b.in_set(nstate, &t.snow))
                } else {
                    sup!()
                }
            }
            Class::Speleothem => {
                self.waterlogged_tick(state, pos);
                if !matches!(dir, Direction::Up | Direction::Down) {
                    return Some(state);
                }
                let tip = table
                    .dir(state, p.vertical_direction)
                    .unwrap_or(Direction::Up);
                if tip == Direction::Down && self.ticks.has_block_tick(pos, block) {
                    return Some(state);
                }
                if dir == tip.opposite() && !self.can_survive(state, pos) {
                    self.schedule_tick(pos, block, if tip == Direction::Down { 2 } else { 1 });
                    state
                } else {
                    let merge = table.text(state, p.thickness) == "tip_merge";
                    let thickness = self.speleothem_thickness(pos, tip, merge);
                    table.set_text(state, p.thickness, thickness)
                }
            }
            Class::Stair => {
                self.waterlogged_tick(state, pos);
                if is_horizontal(dir) {
                    let shape = self.stairs_shape(state, pos);
                    table.set_text(state, p.shape, shape)
                } else {
                    sup!()
                }
            }
            Class::TripWire => {
                if is_horizontal(dir) {
                    let connect = self.tripwire_should_connect_to(nstate, dir);
                    table.set_bool(state, self.direction_prop(dir), connect)
                } else {
                    sup!()
                }
            }
            Class::Vine => {
                if dir == Direction::Down {
                    sup!()
                } else {
                    let updated = self.vine_updated_state(state, pos);
                    if self.vine_count_faces(updated) == 0 {
                        air
                    } else {
                        updated
                    }
                }
            }
            Class::Wall => {
                self.waterlogged_tick(state, pos);
                match dir {
                    Direction::Down => sup!(),
                    Direction::Up => {
                        let sides = [
                            Direction::North,
                            Direction::East,
                            Direction::South,
                            Direction::West,
                        ]
                        .map(|d| table.text(state, self.direction_prop(d)) != "none");
                        self.wall_shape(state, npos, nstate, sides)
                    }
                    _ => {
                        let opposite = dir.opposite();
                        let sides = [
                            Direction::North,
                            Direction::East,
                            Direction::South,
                            Direction::West,
                        ]
                        .map(|d| {
                            if d == dir {
                                self.wall_connects_to(
                                    nstate,
                                    b.is_face_sturdy(nstate, opposite),
                                    opposite,
                                )
                            } else {
                                table.text(state, self.direction_prop(d)) != "none"
                            }
                        });
                        let above_pos = above(pos);
                        let above_state = self.state(above_pos);
                        self.wall_shape(state, above_pos, above_state, sides)
                    }
                }
            }
            Class::WallHangingSign => {
                survive_or_air!(axis_of(dir) == axis_of(clockwise(b.facing(state))))
            }
            Class::PotentSulfur => {
                let next = self.potent_sulfur_state(state, pos);
                // validBlockState: a block that was no geyser starts its
                // block entity's countdown over when it becomes dormant.
                if table.text(next, p.potent_sulfur_state) == "dormant"
                    && !matches!(
                        table.text(state, p.potent_sulfur_state),
                        "erupting" | "dormant"
                    )
                {
                    self.level.reset_potent_sulfur_countdown(pos);
                }
                next
            }
            Class::NetherPortal => {
                let x_axis = table.text(state, p.axis) == "x";
                let update_axis = axis_of(dir);
                let wrong_axis =
                    update_axis != if x_axis { Axis::X } else { Axis::Z } && update_axis != Axis::Y;
                if !wrong_axis
                    && !b.is_block(nstate, block)
                    && !self.find_portal_shape(pos, x_axis).is_complete()
                {
                    air
                } else {
                    sup!()
                }
            }
            _ => return None,
        })
    }

    // ---- family helpers ----

    /// `PotentSulfurBlock.validBlockState` (the geyser block-entity countdown
    /// reset is the caller's).
    pub(crate) fn potent_sulfur_state(&self, state: BlockStateId, pos: BlockPos) -> BlockStateId {
        let b = self.b;
        let key = b.props.potent_sulfur_state;
        if self.fluid_at(above(pos)).kind() != FluidKind::Water {
            return b.table.set_text(state, key, "dry");
        }
        let below_state = self.below_state(pos);
        let fluid = b.fluid(below_state);
        let source_if_fluid = fluid.kind() == FluidKind::Empty || fluid.is_source();
        let tagged = |name: &str| b.tag(name).is_some_and(|tag| b.in_set(below_state, tag));
        if tagged("minecraft:causes_continuous_geyser_eruptions") && source_if_fluid {
            return b.table.set_text(state, key, "continuous");
        }
        if tagged("minecraft:causes_periodic_geyser_eruptions") && source_if_fluid {
            if b.table.text(state, key) == "erupting" {
                state
            } else {
                b.table.set_text(state, key, "dormant")
            }
        } else {
            b.table.set_text(state, key, "wet")
        }
    }

    /// `PotentSulfurBlock.onPlace`: a geyser that starts erupting (or runs
    /// continuously) queues its eruption block event, plays its start sound
    /// and signals the activation.
    pub(crate) fn potent_sulfur_on_place(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let current = b.table.text(state, b.props.potent_sulfur_state);
        if current != "erupting" && current != "continuous" {
            return;
        }
        let sound = if current == "continuous" {
            GEYSER_CONTINUOUS_START
        } else {
            GEYSER_ERUPTION_START
        };
        self.block_event(pos, b.block(state), 0, 0);
        self.sound(pos, &sound.into(), 1.0, 1.0);
        self.game_event_state(pos, "minecraft:block_activate", state);
    }

    fn bed_neighbour_direction(&self, state: BlockStateId) -> Direction {
        let facing = self.b.facing(state);
        if self.b.table.text(state, self.b.props.part) == "foot" {
            facing
        } else {
            facing.opposite()
        }
    }
    pub(crate) fn chest_can_connect_to(&self, state: BlockStateId, other: BlockStateId) -> bool {
        let b = self.b;
        if b.instance_of(state, Class::CopperChest) {
            b.tag("minecraft:copper_chests")
                .is_some_and(|tag| b.in_set(other, tag))
                && b.table.has(other, b.props.kind)
        } else {
            b.block(state) == b.block(other)
        }
    }
    /// `ChestBlock.getConnectedDirection`.
    pub(crate) fn chest_connected_direction(&self, state: BlockStateId) -> Direction {
        let facing = self.b.facing(state);
        if self.b.table.text(state, self.b.props.kind) == "left" {
            clockwise(facing)
        } else {
            counter_clockwise(facing)
        }
    }
    /// `FenceBlock.connectsTo`.
    pub(crate) fn fence_connects_to(
        &self,
        fence: BlockStateId,
        state: BlockStateId,
        face_solid: bool,
        direction: Direction,
    ) -> bool {
        let b = self.b;
        let t = &b.tags;
        let same_fence = b.in_set(state, &t.fences)
            && b.in_set(state, &t.wooden_fences) == b.in_set(fence, &t.wooden_fences);
        let gate = b.instance_of(state, Class::FenceGate)
            && axis_of(b.facing(state)) == axis_of(clockwise(direction));
        (!b.is_exception_for_connection(state) && face_solid) || same_fence || gate
    }
    /// `IronBarsBlock.attachsTo`.
    pub(crate) fn bars_attach_to(&self, state: BlockStateId, face_solid: bool) -> bool {
        let b = self.b;
        (!b.is_exception_for_connection(state) && face_solid)
            || b.instance_of(state, Class::IronBars)
            || b.in_set(state, &b.tags.walls)
    }
    /// `WallBlock.connectsTo`.
    pub(crate) fn wall_connects_to(
        &self,
        state: BlockStateId,
        face_solid: bool,
        direction: Direction,
    ) -> bool {
        let b = self.b;
        let gate = b.instance_of(state, Class::FenceGate)
            && axis_of(b.facing(state)) == axis_of(clockwise(direction));
        b.in_set(state, &b.tags.walls)
            || (!b.is_exception_for_connection(state) && face_solid)
            || b.instance_of(state, Class::IronBars)
            || gate
    }
    /// `WallBlock.updateShape(level, state, topPos, topNeighbour, n, e, s, w)`.
    pub(crate) fn wall_shape(
        &self,
        state: BlockStateId,
        top_pos: BlockPos,
        top: BlockStateId,
        sides: [bool; 4],
    ) -> BlockStateId {
        let b = self.b;
        let table = &b.table;
        let cover = self.shape_info_at(top, top_pos).wall_cover;
        let mut result = state;
        let directions = [
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
        ];
        for (i, direction) in directions.into_iter().enumerate() {
            let value = if !sides[i] {
                "none"
            } else if cover & (1 << (i + 1)) != 0 {
                "tall"
            } else {
                "low"
            };
            result = table.set_text(result, self.direction_prop(direction), value);
        }
        // shouldRaisePost
        let side = |d: Direction| table.text(result, self.direction_prop(d)).to_owned();
        let (north, south, east, west) = (
            side(Direction::North),
            side(Direction::South),
            side(Direction::East),
            side(Direction::West),
        );
        let raise = if b.instance_of(top, Class::Wall) && table.bool(top, b.props.up) {
            true
        } else {
            let (nn, sn, en, wn) = (
                north == "none",
                south == "none",
                east == "none",
                west == "none",
            );
            if (nn && sn && wn && en) || nn != sn || wn != en {
                true
            } else if (north == "tall" && south == "tall") || (east == "tall" && west == "tall") {
                false
            } else {
                b.in_set(top, &b.tags.wall_post_override) || cover & 1 != 0
            }
        };
        table.set_bool(result, b.props.up, raise)
    }
    /// `StairBlock.getStairsShape`.
    pub(crate) fn stairs_shape(&self, state: BlockStateId, pos: BlockPos) -> &'static str {
        let b = self.b;
        let p = b.props;
        let table = &b.table;
        let facing = b.facing(state);
        let half = table.text(state, p.half);
        let behind = self.rel_state(pos, facing);
        if b.instance_of(behind, Class::Stair) && table.text(behind, p.half) == half {
            let behind_facing = b.facing(behind);
            if axis_of(behind_facing) != axis_of(facing)
                && self.stairs_can_take_shape(state, pos, behind_facing.opposite())
            {
                return if behind_facing == counter_clockwise(facing) {
                    "outer_left"
                } else {
                    "outer_right"
                };
            }
        }
        let front = self.rel_state(pos, facing.opposite());
        if b.instance_of(front, Class::Stair) && table.text(front, p.half) == half {
            let front_facing = b.facing(front);
            if axis_of(front_facing) != axis_of(facing)
                && self.stairs_can_take_shape(state, pos, front_facing)
            {
                return if front_facing == counter_clockwise(facing) {
                    "inner_left"
                } else {
                    "inner_right"
                };
            }
        }
        "straight"
    }
    fn stairs_can_take_shape(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> bool {
        let b = self.b;
        let neighbour = self.rel_state(pos, direction);
        !b.instance_of(neighbour, Class::Stair)
            || b.facing(neighbour) != b.facing(state)
            || b.table.text(neighbour, b.props.half) != b.table.text(state, b.props.half)
    }
    /// `TripWireBlock.shouldConnectTo`.
    pub(crate) fn tripwire_should_connect_to(
        &self,
        state: BlockStateId,
        direction: Direction,
    ) -> bool {
        let b = self.b;
        if b.is_block(state, b.named.tripwire_hook) {
            b.facing(state) == direction.opposite()
        } else {
            b.is_block(state, b.named.tripwire)
        }
    }
    /// `LeavesBlock.getDistanceAt`.
    pub(crate) fn leaf_distance_at(&self, state: BlockStateId) -> i32 {
        let b = self.b;
        if b.in_set(state, &b.tags.prevents_nearby_leaf_decay) {
            0
        } else if b.table.has(state, b.props.distance) {
            b.table.int(state, b.props.distance)
        } else {
            7
        }
    }
    /// `ConcretePowderBlock.touchesLiquid`.
    pub(crate) fn concrete_touches_liquid(&self, pos: BlockPos) -> bool {
        let b = self.b;
        let solidifies = |state: BlockStateId| b.is_water_fluid(b.fluid(state).kind());
        let mut test = pos;
        for direction in Direction::ALL {
            let current = self.state(test);
            if direction != Direction::Down || solidifies(current) {
                test = rel(pos, direction);
                let state = self.state(test);
                if solidifies(state) && !b.is_face_sturdy(state, direction.opposite()) {
                    return true;
                }
            }
        }
        false
    }
    pub(crate) fn coral_scan_for_water(&self, pos: BlockPos) -> bool {
        Direction::ALL
            .into_iter()
            .any(|d| self.b.is_water_fluid(self.fluid_at(rel(pos, d)).kind()))
    }
    /// `BaseCoralPlantTypeBlock.scanForWater(state, level, pos)`.
    pub(crate) fn coral_plant_scan_for_water(&self, state: BlockStateId, pos: BlockPos) -> bool {
        if self.b.is_waterlogged(state) {
            return true;
        }
        self.coral_scan_for_water(pos)
    }
    /// `BaseCoralPlantTypeBlock.tryScheduleDieTick`.
    pub(crate) fn coral_try_schedule_die_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        if !self.coral_plant_scan_for_water(state, pos) {
            let delay = 60 + self.random().next_int_bound(40).unwrap_or(0);
            let block = self.b.block(state);
            self.schedule_tick(pos, block, delay);
        }
    }
    /// `BubbleColumnBlock.canOccupy`.
    pub(crate) fn bubble_column_can_occupy(&self, state: BlockStateId) -> bool {
        let b = self.b;
        let fluid = b.fluid(state);
        b.is_block(state, b.named.bubble_column)
            || (b.fluid_in(fluid.kind(), b.fluid_tags.bubble_column_can_occupy)
                && b.instance_of(state, Class::Liquid)
                && fluid.is_source()
                && fluid.amount() >= 8)
    }
    pub(crate) fn should_bubble_column_occupy(&self, state: BlockStateId) -> bool {
        let fluid = self.b.fluid(state);
        self.b
            .fluid_in(fluid.kind(), self.b.fluid_tags.bubble_column_can_occupy)
            && fluid.is_source()
            && fluid.is_full()
    }
    pub(crate) fn try_schedule_bubble_column(
        &mut self,
        pos: BlockPos,
        below_state: BlockStateId,
        block: BlockId,
    ) {
        let t = &self.b.tags;
        if self
            .b
            .in_set(below_state, &t.enables_bubble_column_drag_down)
            || self.b.in_set(below_state, &t.enables_bubble_column_push_up)
        {
            self.schedule_tick(pos, block, 20);
        }
    }
    /// `NoteBlock.setInstrument`.
    pub(crate) fn note_block_instrument(&self, pos: BlockPos, state: BlockStateId) -> BlockStateId {
        let b = self.b;
        let above = b.facts(self.above_state(pos)).instrument;
        let name = if super::data::instrument_works_above(above) {
            above
        } else {
            let below = b.facts(self.below_state(pos)).instrument;
            if super::data::instrument_works_above(below) {
                0
            } else {
                below
            }
        };
        b.table.set_text(
            state,
            b.props.instrument,
            super::data::INSTRUMENTS[name as usize],
        )
    }
    /// `SpeleothemBlock.calculateSpeleothemThickness`.
    pub(crate) fn speleothem_thickness(
        &self,
        pos: BlockPos,
        tip: Direction,
        merge: bool,
    ) -> &'static str {
        let b = self.b;
        let p = b.props;
        let base = tip.opposite();
        let is_with = |state: BlockStateId, direction: Direction| {
            b.tag("minecraft:speleothems")
                .is_some_and(|tag| b.in_set(state, tag))
                && b.table.dir(state, p.vertical_direction) == Some(direction)
        };
        let front = self.rel_state(pos, tip);
        let block = b.block(self.state(pos));
        if is_with(front, base) && b.is_block(front, block) {
            if !merge && b.table.text(front, p.thickness) != "tip_merge" {
                "tip"
            } else {
                "tip_merge"
            }
        } else if !is_with(front, tip) {
            "tip"
        } else {
            let front_thickness = b.table.text(front, p.thickness);
            if front_thickness != "tip" && front_thickness != "tip_merge" {
                let behind = self.rel_state(pos, base);
                if !is_with(behind, tip) {
                    "base"
                } else {
                    "middle"
                }
            } else {
                "frustum"
            }
        }
    }
    /// `FireBlock.getStateWithAge` via `BaseFireBlock.getState`.
    pub(crate) fn fire_state_with_age(&self, pos: BlockPos, age: i32) -> BlockStateId {
        let b = self.b;
        let placement = self.fire_state_for(pos);
        if b.is_block(placement, b.named.fire) {
            b.table.set_int(placement, b.props.age, age)
        } else {
            placement
        }
    }
    /// `BaseFireBlock.getState(level, pos)`.
    pub(crate) fn fire_state_for(&self, pos: BlockPos) -> BlockStateId {
        let b = self.b;
        let below_pos = below(pos);
        let below_state = self.state(below_pos);
        if b.in_set(below_state, &b.tags.soul_fire_base_blocks) {
            return b.default_of(b.named.soul_fire);
        }
        let fire = b.default_of(b.named.fire);
        if !self.can_burn(below_state) && !b.is_face_sturdy(below_state, Direction::Up) {
            let mut result = fire;
            for direction in Direction::ALL {
                if direction == Direction::Down {
                    continue;
                }
                let burn = self.can_burn(self.rel_state(pos, direction));
                result = b
                    .table
                    .set_bool(result, self.direction_prop(direction), burn);
            }
            result
        } else {
            fire
        }
    }
    /// `MossyCarpetBlock.getUpdatedState`.
    pub(crate) fn mossy_carpet_updated_state(
        &self,
        mut state: BlockStateId,
        pos: BlockPos,
        create_sides: bool,
    ) -> BlockStateId {
        let b = self.b;
        let table = &b.table;
        let block = b.block(state);
        let base = table.bool(state, b.props.bottom);
        let create_sides = create_sides || base;
        let mut above_state = None;
        let mut below_state = None;
        for direction in [
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
        ] {
            let key = self.direction_prop(direction);
            let supported = b.multiface_can_attach(self.rel_state(pos, direction), direction);
            let mut side = if supported {
                if create_sides {
                    "low"
                } else {
                    table.text(state, key)
                }
            } else {
                "none"
            };
            if side == "low" {
                let above = *above_state.get_or_insert_with(|| self.above_state(pos));
                if b.is_block(above, block)
                    && table.text(above, key) != "none"
                    && !table.bool(above, b.props.bottom)
                {
                    side = "tall";
                }
                if !base {
                    let below = *below_state.get_or_insert_with(|| self.below_state(pos));
                    if b.is_block(below, block) && table.text(below, key) == "none" {
                        side = "none";
                    }
                }
            }
            let side = side.to_owned();
            state = table.set_text(state, key, &side);
        }
        state
    }
    pub(crate) fn mossy_carpet_has_faces(&self, state: BlockStateId) -> bool {
        let b = self.b;
        table_bool(b, state)
            || [
                Direction::North,
                Direction::East,
                Direction::South,
                Direction::West,
            ]
            .into_iter()
            .any(|d| b.table.text(state, self.direction_prop(d)) != "none")
    }
}

fn table_bool(b: &super::Behaviours, state: BlockStateId) -> bool {
    b.table.bool(state, b.props.bottom)
}
fn class_is(b: &super::Behaviours, block: BlockId, class: Class) -> bool {
    b.block_data(block).is(class)
}
