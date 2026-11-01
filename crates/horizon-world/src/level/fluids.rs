//! `FlowingFluid` (water and lava) ticking and spreading, `LiquidBlock`
//! interactions, liquid containers (waterlogging) and bucket pickup.
use super::{
    Behaviours, Dimension, LevelAccess, Sim, Spread,
    classes::{Class, Interface},
    update::{HORIZONTAL, above, below, rel},
};
use crate::{
    fluid::{FluidKind, FluidState},
    registry::BlockStateId,
};
use horizon_core::{
    geometry::{Axis, Direction, ShapeOp, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};

/// `LiquidBlock.POSSIBLE_FLOW_DIRECTIONS`.
const POSSIBLE_FLOW_DIRECTIONS: [Direction; 5] = [
    Direction::Down,
    Direction::South,
    Direction::North,
    Direction::East,
    Direction::West,
];

fn is_water(kind: FluidKind) -> bool {
    matches!(kind, FluidKind::Water | FluidKind::FlowingWater)
}
fn source_of(kind: FluidKind) -> FluidKind {
    if is_water(kind) {
        FluidKind::Water
    } else {
        FluidKind::Lava
    }
}
fn flowing_of(kind: FluidKind) -> FluidKind {
    if is_water(kind) {
        FluidKind::FlowingWater
    } else {
        FluidKind::FlowingLava
    }
}

/// `FlowingFluid.SpreadContext`: per-spread caches keyed by horizontal offset.
/// A spread looks at most `SPREAD_REACH` blocks away on each axis (the slope
/// search's 4 steps after the first), so the caches are flat arrays around
/// the origin: no hashing and no allocation on the hot path. A position
/// outside them is read uncached.
struct SpreadContext {
    origin: BlockPos,
    states: [Option<BlockStateId>; SPREAD_CELLS],
    holes: [Option<bool>; SPREAD_CELLS],
}
const SPREAD_REACH: i32 = 5;
const SPREAD_SIDE: i32 = 2 * SPREAD_REACH + 1;
const SPREAD_CELLS: usize = (SPREAD_SIDE * SPREAD_SIDE) as usize;
impl SpreadContext {
    fn new(origin: BlockPos) -> Self {
        Self {
            origin,
            states: [None; SPREAD_CELLS],
            holes: [None; SPREAD_CELLS],
        }
    }
    fn index(&self, pos: BlockPos) -> Option<usize> {
        let x = pos.x.wrapping_sub(self.origin.x) + SPREAD_REACH;
        let z = pos.z.wrapping_sub(self.origin.z) + SPREAD_REACH;
        ((0..SPREAD_SIDE).contains(&x) && (0..SPREAD_SIDE).contains(&z))
            .then(|| (x * SPREAD_SIDE + z) as usize)
    }
}

impl Behaviours {
    /// `state.getBlock() instanceof LiquidBlockContainer`: a block that keeps
    /// a fluid inside its own state (`SimpleWaterloggedBlock` extends the
    /// interface, so stairs, slabs and every other waterloggable block count).
    pub fn is_liquid_container(&self, state: BlockStateId) -> bool {
        self.implements(state, Interface::LiquidBlockContainer)
            || self.implements(state, Interface::SimpleWaterlogged)
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn fast_lava(&self) -> bool {
        self.level.dimension() == Dimension::Nether
    }
    /// `Fluid.getTickDelay`.
    pub(crate) fn fluid_tick_delay(&self, kind: FluidKind) -> i32 {
        match kind {
            FluidKind::Empty => 0,
            FluidKind::Water | FluidKind::FlowingWater => 5,
            _ => {
                if self.fast_lava() {
                    10
                } else {
                    30
                }
            }
        }
    }
    fn drop_off(&self, kind: FluidKind) -> i32 {
        if is_water(kind) || self.fast_lava() {
            1
        } else {
            2
        }
    }
    fn slope_find_distance(&self, kind: FluidKind) -> i32 {
        if is_water(kind) || self.fast_lava() {
            4
        } else {
            2
        }
    }
    fn can_convert_to_source(&self, kind: FluidKind) -> bool {
        let rules = self.level.game_rules();
        if is_water(kind) {
            rules.water_source_conversion
        } else {
            rules.lava_source_conversion
        }
    }
    fn fluid_height(&self, fluid: FluidState, pos: BlockPos) -> f32 {
        if fluid.kind() == FluidKind::Empty {
            return 0.0;
        }
        if fluid.kind().same_family(self.fluid_at(above(pos)).kind()) {
            1.0
        } else {
            fluid.own_height()
        }
    }
    /// `FluidState.canBeReplacedWith(level, pos, other, direction)`.
    fn can_be_replaced_with(
        &self,
        fluid: FluidState,
        pos: BlockPos,
        other: FluidKind,
        direction: Direction,
    ) -> bool {
        match fluid.kind() {
            FluidKind::Empty => true,
            FluidKind::Water | FluidKind::FlowingWater => {
                direction == Direction::Down && !is_water(other)
            }
            _ => self.fluid_height(fluid, pos) >= 0.444_444_45 && is_water(other),
        }
    }

    // ---- shape occlusion ----

    fn collision_or_empty(&self, state: BlockStateId) -> (i32, &VoxelShape) {
        let index = self.b.facts(state).shape;
        match self.b.collision_shape(state) {
            Some(shape) => (index, shape),
            None => (-1, VoxelShape::empty_ref()),
        }
    }
    /// `FlowingFluid.canPassThroughWall`.
    fn can_pass_through_wall(
        &self,
        direction: Direction,
        source: BlockStateId,
        target: BlockStateId,
    ) -> bool {
        // The shapes are compared by identity and borrowed, never cloned: a
        // clone of the shared block or empty shape is atomic reference counts
        // every thread contends on, and this runs for every step of a spread.
        let (target_index, target_shape) = self.collision_or_empty(target);
        if target_shape.is_block() {
            return false;
        }
        let (source_index, source_shape) = self.collision_or_empty(source);
        if source_shape.is_block() {
            return false;
        }
        if source_shape.is_empty_identity() && target_shape.is_empty_identity() {
            return true;
        }
        let cacheable = source_index >= 0 && target_index >= 0;
        let key = (source_index, target_index, direction as u8);
        if cacheable
            && let Some(result) = self
                .b
                .occlusion_cache
                .read()
                .ok()
                .and_then(|cache| cache.get(&key).copied())
        {
            return result;
        }
        let result = !merged_face_occludes(source_shape, target_shape, direction);
        if cacheable && let Ok(mut cache) = self.b.occlusion_cache.write() {
            cache.insert(key, result);
        }
        result
    }

    // ---- liquid containers ----

    /// `LiquidBlockContainer` membership (`canHoldAnyFluid` first clause).
    pub(super) fn is_liquid_container(&self, state: BlockStateId) -> bool {
        self.b.is_liquid_container(state)
    }
    /// `LiquidBlockContainer.canPlaceLiquid(user, level, pos, state, fluid)`:
    /// `creative` is whether `user` is a creative player (a bucket in creative
    /// mode); a dispenser or a flowing fluid passes none (`false`).
    pub(crate) fn can_place_liquid(
        &self,
        state: BlockStateId,
        fluid: FluidKind,
        creative: bool,
    ) -> bool {
        let b = self.b;
        let data = b.data(state);
        if data.is(Class::Kelp)
            || data.is(Class::KelpPlant)
            || data.is(Class::Seagrass)
            || data.is(Class::TallSeagrass)
        {
            return false;
        }
        // `BarrierBlock.canPlaceLiquid`: only a creative player may waterlog.
        if data.is(Class::Barrier) && !creative {
            return false;
        }
        if data.is(Class::Slab) && b.table.text(state, b.props.kind) == "double" {
            return false;
        }
        fluid == FluidKind::Water
    }
    /// `LiquidBlockContainer.placeLiquid`.
    pub(crate) fn place_liquid(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        fluid: FluidState,
        creative: bool,
    ) -> bool {
        let b = self.b;
        if !self.can_place_liquid(state, fluid.kind(), creative)
            || fluid.kind() != FluidKind::Water
            || !b.table.has(state, b.props.waterlogged)
        {
            return false;
        }
        if b.is_waterlogged(state) {
            return false;
        }
        let mut next = b.table.set_bool(state, b.props.waterlogged, true);
        let data = b.data(state);
        if (data.is(Class::Candle) || data.is(Class::Campfire)) && b.table.bool(state, b.props.lit)
        {
            next = b.table.set_bool(next, b.props.lit, false);
            if data.is(Class::Campfire) {
                self.emit(super::LevelEvent::Sound {
                    pos,
                    sound: "minecraft:block.generic.extinguish_fire".into(),
                    volume: 1.0,
                    pitch: 1.0,
                });
            } else {
                self.emit(super::LevelEvent::Sound {
                    pos,
                    sound: "minecraft:block.candle.extinguish".into(),
                    volume: 1.0,
                    pitch: 1.0,
                });
            }
        }
        self.set_block_and_update(pos, next);
        let delay = self.fluid_tick_delay(fluid.kind());
        self.schedule_fluid_tick(pos, fluid.kind(), delay);
        // DriedGhastBlock.placeLiquid: its own place-in-water sound.
        if data.is(Class::DriedGhast) {
            self.emit(super::LevelEvent::Sound {
                pos,
                sound: "minecraft:block.dried_ghast.place_in_water".into(),
                volume: 1.0,
                pitch: 1.0,
            });
        }
        true
    }
    /// `BucketPickup.pickupBlock`: true when a fluid was taken.
    pub(crate) fn pickup_block(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        creative: bool,
    ) -> bool {
        let b = self.b;
        if b.instance_of(state, Class::Liquid) {
            if b.table.int(state, b.props.level) == 0 {
                self.set_block(pos, b.air(), 11);
                return true;
            }
            return false;
        }
        if b.implements(state, Interface::SimpleWaterlogged)
            && b.table.has(state, b.props.waterlogged)
        {
            // `BarrierBlock.pickupBlock`: only a creative player drains one.
            if b.instance_of(state, Class::Barrier) && !creative {
                return false;
            }
            if b.is_waterlogged(state) {
                self.set_block_and_update(pos, b.table.set_bool(state, b.props.waterlogged, false));
                if !self.can_survive(state, pos) {
                    self.destroy_block(pos, true, 512);
                }
                return true;
            }
            return false;
        }
        if b.is_block(state, b.named.bubble_column) {
            // BubbleColumnBlock.pickupBlock: the column is removed, the bucket is water.
            self.set_block(pos, b.air(), 11);
            return true;
        }
        if b.is_block(state, b.named.powder_snow) {
            // PowderSnowBlock.pickupBlock: the block breaks with its particles and sound.
            self.set_block(pos, b.air(), 11);
            self.level_event(2001, pos, state.0 as i32);
            return true;
        }
        false
    }
    fn can_hold_any_fluid(&self, state: BlockStateId) -> bool {
        self.is_liquid_container(state) || self.b.in_set(state, &self.b.tags.washed_away_by_fluids)
    }
    fn can_hold_specific_fluid(&self, state: BlockStateId, fluid: FluidKind) -> bool {
        if self.is_liquid_container(state) {
            self.can_place_liquid(state, fluid, false)
        } else {
            true
        }
    }
    fn can_hold_fluid(&self, state: BlockStateId, fluid: FluidKind) -> bool {
        self.can_hold_any_fluid(state) && self.can_hold_specific_fluid(state, fluid)
    }

    // ---- FlowingFluid ----

    fn is_source_of(&self, kind: FluidKind, fluid: FluidState) -> bool {
        fluid.kind().same_family(kind) && fluid.is_source()
    }
    /// `FlowingFluid.getNewLiquid`.
    fn new_liquid(&self, kind: FluidKind, pos: BlockPos, state: BlockStateId) -> FluidState {
        let b = self.b;
        let mut highest = 0;
        let mut sources = 0;
        for direction in HORIZONTAL {
            let neighbour = self.rel_state(pos, direction);
            let fluid = b.fluid(neighbour);
            if fluid.kind().same_family(kind)
                && self.can_pass_through_wall(direction, state, neighbour)
            {
                if fluid.is_source() {
                    sources += 1;
                }
                highest = highest.max(i32::from(fluid.amount()));
            }
        }
        if sources >= 2 && self.can_convert_to_source(kind) {
            let below_state = self.below_state(pos);
            if b.is_solid(below_state) || self.is_source_of(kind, b.fluid(below_state)) {
                return FluidState::new(source_of(kind), 8, false);
            }
        }
        let above_state = self.above_state(pos);
        let above_fluid = b.fluid(above_state);
        if above_fluid.kind() != FluidKind::Empty
            && above_fluid.kind().same_family(kind)
            && self.can_pass_through_wall(Direction::Up, state, above_state)
        {
            return FluidState::new(flowing_of(kind), 8, true);
        }
        let amount = highest - self.drop_off(kind);
        if amount <= 0 {
            FluidState::EMPTY
        } else {
            FluidState::new(flowing_of(kind), amount as u8, false)
        }
    }
    fn is_water_hole(&self, kind: FluidKind, top: BlockStateId, bottom: BlockStateId) -> bool {
        if !self.can_pass_through_wall(Direction::Down, top, bottom) {
            return false;
        }
        if self.b.fluid(bottom).kind().same_family(kind) {
            true
        } else {
            self.can_hold_fluid(bottom, flowing_of(kind))
        }
    }
    fn can_maybe_pass_through(
        &self,
        kind: FluidKind,
        source: BlockStateId,
        direction: Direction,
        test: BlockStateId,
        test_fluid: FluidState,
    ) -> bool {
        !self.is_source_of(kind, test_fluid)
            && self.can_hold_any_fluid(test)
            && self.can_pass_through_wall(direction, source, test)
    }
    fn can_pass_through(
        &self,
        kind: FluidKind,
        source: BlockStateId,
        direction: Direction,
        test: BlockStateId,
        test_fluid: FluidState,
    ) -> bool {
        self.can_maybe_pass_through(kind, source, direction, test, test_fluid)
            && self.can_hold_specific_fluid(test, flowing_of(kind))
    }
    fn context_state(&self, context: &mut SpreadContext, pos: BlockPos) -> BlockStateId {
        let Some(i) = context.index(pos) else {
            return self.state(pos);
        };
        if let Some(state) = context.states[i] {
            return state;
        }
        let state = self.state(pos);
        context.states[i] = Some(state);
        state
    }
    fn context_is_hole(&self, kind: FluidKind, context: &mut SpreadContext, pos: BlockPos) -> bool {
        let index = context.index(pos);
        if let Some(hole) = index.and_then(|i| context.holes[i]) {
            return hole;
        }
        let state = self.context_state(context, pos);
        let below_state = self.below_state(pos);
        let hole = self.is_water_hole(kind, state, below_state);
        if let Some(i) = index {
            context.holes[i] = Some(hole);
        }
        hole
    }
    fn slope_distance(
        &self,
        kind: FluidKind,
        pos: BlockPos,
        pass: i32,
        from: Direction,
        state: BlockStateId,
        context: &mut SpreadContext,
    ) -> i32 {
        let mut lowest = 1000;
        for direction in HORIZONTAL {
            if direction == from {
                continue;
            }
            let test_pos = rel(pos, direction);
            let test_state = self.context_state(context, test_pos);
            let test_fluid = self.b.fluid(test_state);
            if self.can_pass_through(kind, state, direction, test_state, test_fluid) {
                if self.context_is_hole(kind, context, test_pos) {
                    return pass;
                }
                if pass < self.slope_find_distance(kind) {
                    let distance = self.slope_distance(
                        kind,
                        test_pos,
                        pass + 1,
                        direction.opposite(),
                        test_state,
                        context,
                    );
                    lowest = lowest.min(distance);
                }
            }
        }
        lowest
    }
    /// `FlowingFluid.getSpread` (EnumMap: iteration in Direction order).
    fn spread_targets(
        &self,
        kind: FluidKind,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Vec<(Direction, FluidState)> {
        let mut lowest = 1000;
        let mut result: Vec<(Direction, FluidState)> = Vec::new();
        let mut context: Option<SpreadContext> = None;
        for direction in HORIZONTAL {
            let test_pos = rel(pos, direction);
            let test_state = self.state(test_pos);
            let test_fluid = self.b.fluid(test_state);
            if !self.can_maybe_pass_through(kind, state, direction, test_state, test_fluid) {
                continue;
            }
            let new_fluid = self.new_liquid(kind, test_pos, test_state);
            if !self.can_hold_specific_fluid(test_state, new_fluid.kind()) {
                continue;
            }
            let context = context.get_or_insert_with(|| SpreadContext::new(pos));
            let distance = if self.context_is_hole(kind, context, test_pos) {
                0
            } else {
                self.slope_distance(kind, test_pos, 1, direction.opposite(), test_state, context)
            };
            if distance < lowest {
                result.clear();
            }
            if distance <= lowest {
                if self.can_be_replaced_with(test_fluid, test_pos, new_fluid.kind(), direction) {
                    result.push((direction, new_fluid));
                }
                lowest = distance;
            }
        }
        result.sort_by_key(|(direction, _)| *direction as u8);
        result
    }
    fn source_neighbour_count(&self, kind: FluidKind, pos: BlockPos) -> i32 {
        HORIZONTAL
            .into_iter()
            .filter(|d| self.is_source_of(kind, self.fluid_at(rel(pos, *d))))
            .count() as i32
    }
    /// `FlowingFluid.spreadTo` (with LavaFluid's water-to-stone override).
    fn spread_to(
        &mut self,
        kind: FluidKind,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        target: FluidState,
    ) {
        let b = self.b;
        if !self
            .level
            .allow_spread(Spread::Fluid, rel(pos, direction.opposite()), pos)
        {
            return;
        }
        if !is_water(kind)
            && direction == Direction::Down
            && b.is_water_fluid(self.fluid_at(pos).kind())
        {
            if b.instance_of(state, Class::Liquid) {
                self.set_block_and_update(pos, b.default_of(b.named.stone));
            }
            self.level_event(1501, pos, 0);
            return;
        }
        if self.is_liquid_container(state) {
            self.place_liquid(pos, state, target, false);
        } else {
            if !b.is_air(state) {
                if is_water(kind) {
                    self.drop_resources(state, pos);
                } else {
                    self.level_event(1501, pos, 0);
                }
            }
            let legacy = b.legacy_block(target);
            self.set_block_and_update(pos, legacy);
        }
    }
    fn spread_to_sides(
        &mut self,
        kind: FluidKind,
        pos: BlockPos,
        fluid: FluidState,
        state: BlockStateId,
    ) {
        let mut neighbour = i32::from(fluid.amount()) - self.drop_off(kind);
        if fluid.falling() {
            neighbour = 7;
        }
        if neighbour > 0 {
            for (direction, new_fluid) in self.spread_targets(kind, pos, state) {
                let neighbour_pos = rel(pos, direction);
                let neighbour_state = self.state(neighbour_pos);
                self.spread_to(kind, neighbour_pos, neighbour_state, direction, new_fluid);
            }
        }
    }
    /// `FlowingFluid.spread`.
    fn spread(&mut self, kind: FluidKind, pos: BlockPos, state: BlockStateId, fluid: FluidState) {
        if fluid.kind() == FluidKind::Empty {
            return;
        }
        let below_pos = below(pos);
        let below_state = self.state(below_pos);
        let below_fluid = self.b.fluid(below_state);
        if self.can_maybe_pass_through(kind, state, Direction::Down, below_state, below_fluid) {
            let new_below = self.new_liquid(kind, below_pos, below_state);
            if self.can_be_replaced_with(below_fluid, below_pos, new_below.kind(), Direction::Down)
                && self.can_hold_specific_fluid(below_state, new_below.kind())
            {
                self.spread_to(kind, below_pos, below_state, Direction::Down, new_below);
                if self.source_neighbour_count(kind, pos) >= 3 {
                    self.spread_to_sides(kind, pos, fluid, state);
                }
                return;
            }
        }
        if fluid.is_source() || !self.is_water_hole(kind, state, below_state) {
            self.spread_to_sides(kind, pos, fluid, state);
        }
    }
    /// `FlowingFluid.tick` for the fluid at `pos`.
    pub(crate) fn fluid_tick(&mut self, pos: BlockPos) {
        let mut state = self.state(pos);
        let mut fluid = self.b.fluid(state);
        let kind = fluid.kind();
        if kind == FluidKind::Empty {
            return;
        }
        if !fluid.is_source() {
            let new_fluid = self.new_liquid(kind, pos, state);
            let delay = self.spread_delay(kind, pos, fluid, new_fluid);
            if new_fluid.kind() == FluidKind::Empty {
                fluid = new_fluid;
                state = self.b.air();
                self.set_block_and_update(pos, state);
            } else if new_fluid != fluid {
                fluid = new_fluid;
                state = self.b.legacy_block(fluid);
                self.set_block_and_update(pos, state);
                self.schedule_fluid_tick(pos, fluid.kind(), delay);
            }
        }
        self.spread(kind, pos, state, fluid);
    }
    fn spread_delay(
        &mut self,
        kind: FluidKind,
        pos: BlockPos,
        old: FluidState,
        new: FluidState,
    ) -> i32 {
        let delay = self.fluid_tick_delay(kind);
        if is_water(kind) {
            return delay;
        }
        if old.kind() != FluidKind::Empty
            && new.kind() != FluidKind::Empty
            && !old.falling()
            && !new.falling()
            && self.fluid_height(new, pos) > self.fluid_height(old, pos)
            && self.random().next_int_bound(4).unwrap_or(0) != 0
        {
            delay * 4
        } else {
            delay
        }
    }

    // ---- LiquidBlock ----

    /// `LiquidBlock.shouldSpreadLiquid`, including lava's conversions.
    pub(crate) fn should_spread_liquid(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        let b = self.b;
        if !b.is_lava_fluid(b.fluid(state).kind()) {
            return true;
        }
        let over_soul_soil = b.is_block(self.below_state(pos), b.named.soul_soil);
        for direction in POSSIBLE_FLOW_DIRECTIONS {
            let neighbour = rel(pos, direction.opposite());
            if b.is_water_fluid(self.fluid_at(neighbour).kind()) {
                let block = if self.fluid_at(pos).is_source() {
                    b.named.obsidian
                } else {
                    b.named.cobblestone
                };
                self.set_block_and_update(pos, b.default_of(block));
                self.level_event(1501, pos, 0);
                return false;
            }
            if over_soul_soil && b.is_block(self.state(neighbour), b.named.blue_ice) {
                self.set_block_and_update(pos, b.default_of(b.named.basalt));
                self.level_event(1501, pos, 0);
                return false;
            }
        }
        true
    }
    /// `LiquidBlock.onPlace` / `neighborChanged`.
    pub(crate) fn liquid_block_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        if self.should_spread_liquid(pos, state) {
            let kind = self.b.fluid(state).kind();
            let delay = self.fluid_tick_delay(kind);
            self.schedule_fluid_tick(pos, kind, delay);
        }
        if self.should_bubble_column_occupy(state) {
            let below_state = self.below_state(pos);
            let block = self.b.block(state);
            self.try_schedule_bubble_column(pos, below_state, block);
        }
    }

    /// `LavaFluid.randomTick`.
    pub(crate) fn lava_random_tick(&mut self, pos: BlockPos) {
        if !self.level.can_spread_fire_around(pos) {
            return;
        }
        let passes = self.random().next_int_bound(3).unwrap_or(0);
        if passes > 0 {
            let mut test = pos;
            for _ in 0..passes {
                let dx = self.random().next_int_bound(3).unwrap_or(0) - 1;
                let dz = self.random().next_int_bound(3).unwrap_or(0) - 1;
                test = test.offset(dx, 1, dz);
                if !self.level.is_loaded(test) {
                    return;
                }
                let state = self.state(test);
                if self.b.is_air(state) {
                    if Direction::ALL
                        .into_iter()
                        .any(|d| self.lava_flammable(rel(test, d)))
                    {
                        let fire = self.fire_state_for(test);
                        self.set_block_and_update(test, fire);
                        return;
                    }
                } else if self
                    .b
                    .tag("minecraft:blocks_lava_fire_spread")
                    .is_some_and(|tag| self.b.in_set(state, tag))
                {
                    return;
                }
            }
        } else {
            for _ in 0..3 {
                let dx = self.random().next_int_bound(3).unwrap_or(0) - 1;
                let dz = self.random().next_int_bound(3).unwrap_or(0) - 1;
                let test = pos.offset(dx, 0, dz);
                if !self.level.is_loaded(test) {
                    return;
                }
                if self.is_empty_block(above(test)) && self.lava_flammable(test) {
                    let fire = self.fire_state_for(test);
                    self.set_block_and_update(above(test), fire);
                }
            }
        }
    }
    fn lava_flammable(&self, pos: BlockPos) -> bool {
        if self.is_inside_build_height(pos.y) && !self.level.is_loaded(pos) {
            return false;
        }
        self.b
            .has_flag(self.state(pos), super::data::state_flags::IGNITED_BY_LAVA)
    }
}

/// `Shapes.mergedFaceOccludes(shape, occluder, direction)`.
pub(crate) fn merged_face_occludes(
    shape: &VoxelShape,
    occluder: &VoxelShape,
    direction: Direction,
) -> bool {
    let block = VoxelShape::block();
    if shape.is_block() || occluder.is_block() {
        return true;
    }
    let positive = matches!(
        direction,
        Direction::Up | Direction::South | Direction::East
    );
    let axis = match direction {
        Direction::West | Direction::East => Axis::X,
        Direction::Down | Direction::Up => Axis::Y,
        _ => Axis::Z,
    };
    let (first, second) = if positive {
        (shape, occluder)
    } else {
        (occluder, shape)
    };
    let max_face = match axis {
        Axis::X => Direction::East,
        Axis::Y => Direction::Up,
        Axis::Z => Direction::South,
    };
    let empty = VoxelShape::empty();
    // SliceShape(first, axis, last layer) / SliceShape(second, axis, 0): the face
    // shapes sample the same boundary layers when the shapes touch the face.
    let first_slice = if !first.is_empty() && (first.max(axis) - 1.0).abs() <= 1.0e-7 {
        first.face_shape(max_face).unwrap_or_else(|_| empty.clone())
    } else {
        empty.clone()
    };
    let second_slice = if !second.is_empty() && second.min(axis).abs() <= 1.0e-7 {
        second
            .face_shape(max_face.opposite())
            .unwrap_or_else(|_| empty.clone())
    } else {
        empty.clone()
    };
    let union = match first_slice.join(&second_slice, ShapeOp::OR) {
        Ok(shape) => shape,
        Err(_) => return false,
    };
    !block
        .join_is_not_empty(&union, ShapeOp::ONLY_FIRST)
        .unwrap_or(true)
}
