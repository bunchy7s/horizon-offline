//! `SpeleothemBlock` growth (pointed dripstone, sulfur spikes) and
//! `PointedDripstoneBlock` fluid transfer into cauldrons and mud.
use super::{
    LevelAccess, Sim,
    classes::Class,
    states::BlockId,
    update::{above, below, rel},
};
use crate::{fluid::FluidKind, registry::BlockStateId};
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn speleothem_tag(&self, state: BlockStateId) -> bool {
        self.b
            .tag("minecraft:speleothems")
            .is_some_and(|tag| self.b.in_set(state, tag))
    }
    fn speleothem_with(&self, state: BlockStateId, direction: Direction) -> bool {
        self.speleothem_tag(state)
            && self.b.table.dir(state, self.b.props.vertical_direction) == Some(direction)
    }
    fn speleothem_is_tip(&self, state: BlockStateId, include_merged: bool) -> bool {
        if !self.speleothem_tag(state) {
            return false;
        }
        let thickness = self.b.table.text(state, self.b.props.thickness);
        thickness == "tip" || (include_merged && thickness == "tip_merge")
    }
    fn is_free_hanging_stalactite(&self, state: BlockStateId) -> bool {
        self.speleothem_with(state, Direction::Down)
            && self.b.table.text(state, self.b.props.thickness) == "tip"
            && !self.b.is_waterlogged(state)
    }
    /// `SpeleothemBlock.findBlockVertical`.
    fn find_block_vertical(
        &self,
        pos: BlockPos,
        up: bool,
        path: impl Fn(&Self, BlockPos, BlockStateId) -> bool,
        target: impl Fn(&Self, BlockStateId) -> bool,
        max_steps: i32,
    ) -> Option<BlockPos> {
        let mut current = pos;
        for _ in 1..max_steps {
            current = if up { above(current) } else { below(current) };
            let state = self.state(current);
            if target(self, state) {
                return Some(current);
            }
            if !self.is_inside_build_height(current.y) || !path(self, current, state) {
                return None;
            }
        }
        None
    }
    fn speleothem_find_tip(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        max: i32,
        include_merged: bool,
    ) -> Option<BlockPos> {
        if self.speleothem_is_tip(state, include_merged) {
            return Some(pos);
        }
        let b = self.b;
        let direction = b
            .table
            .dir(state, b.props.vertical_direction)
            .unwrap_or(Direction::Up);
        let block = b.block(state);
        self.find_block_vertical(
            pos,
            direction == Direction::Up,
            |sim, _, s| {
                sim.b.is_block(s, block)
                    && sim.b.table.dir(s, sim.b.props.vertical_direction) == Some(direction)
            },
            |sim, s| sim.speleothem_is_tip(s, include_merged),
            max,
        )
    }
    /// `SpeleothemBlock.randomTick` (+ `PointedDripstoneBlock.maybeTransferFluid`).
    pub(crate) fn speleothem_random_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        if b.instance_of(state, Class::PointedDripstone) {
            let value = self.random().next_float();
            self.dripstone_transfer_fluid(state, pos, value);
        }
        let start = self.speleothem_with(state, Direction::Down)
            && !b.is_block(self.above_state(pos), b.block(state));
        if self.random().next_float() < 0.011_377_778 && start {
            self.speleothem_grow(state, pos);
        }
    }
    fn speleothem_grow(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let block = b.block(state);
        let data = b.data(state);
        let mut can_grow = data
            .link
            .is_some_and(|grow_on| b.is_block(self.above_state(pos), grow_on));
        if data.is(Class::PointedDripstone) {
            let fluid = b.fluid(self.state(pos.offset(0, 2, 0)));
            can_grow &= fluid.kind() == FluidKind::Water;
        }
        if !can_grow {
            return;
        }
        let max = if data.is(Class::SulfurSpike) { 2 } else { 7 };
        let Some(tip_pos) = self.speleothem_find_tip(state, pos, max, false) else {
            return;
        };
        let tip = self.state(tip_pos);
        if self.is_free_hanging_stalactite(tip) && self.speleothem_can_tip_grow(block, tip, tip_pos)
        {
            if self.random().next_boolean() {
                self.speleothem_grow_to(block, tip_pos, Direction::Down);
            } else {
                self.speleothem_grow_stalagmite_below(block, tip_pos);
            }
        }
    }
    fn speleothem_unmerged_tip(
        &self,
        block: BlockId,
        state: BlockStateId,
        direction: Direction,
    ) -> bool {
        self.speleothem_is_tip(state, false)
            && self.b.table.dir(state, self.b.props.vertical_direction) == Some(direction)
            && self.b.is_block(state, block)
    }
    fn speleothem_can_tip_grow(
        &self,
        block: BlockId,
        tip: BlockStateId,
        tip_pos: BlockPos,
    ) -> bool {
        let direction = self
            .b
            .table
            .dir(tip, self.b.props.vertical_direction)
            .unwrap_or(Direction::Up);
        let grow = self.rel_state(tip_pos, direction);
        if self.b.fluid(grow).kind() != FluidKind::Empty {
            false
        } else {
            self.b.is_air(grow) || self.speleothem_unmerged_tip(block, grow, direction.opposite())
        }
    }
    fn speleothem_create(
        &mut self,
        block: BlockId,
        pos: BlockPos,
        direction: Direction,
        thickness: &str,
    ) {
        let b = self.b;
        let state = b
            .table
            .set_dir(b.default_of(block), b.props.vertical_direction, direction);
        let state = b.table.set_text(state, b.props.thickness, thickness);
        let water = self.fluid_at(pos) == crate::fluid::FluidState::WATER
            || self.fluid_at(pos).kind() == FluidKind::Water;
        self.set_block_and_update(pos, b.table.set_bool(state, b.props.waterlogged, water));
    }
    fn speleothem_grow_to(&mut self, block: BlockId, from: BlockPos, direction: Direction) {
        let b = self.b;
        let target = rel(from, direction);
        let existing = self.state(target);
        if self.speleothem_unmerged_tip(block, existing, direction.opposite()) {
            let (stalactite, stalagmite) =
                if b.table.dir(existing, b.props.vertical_direction) == Some(Direction::Up) {
                    (above(target), target)
                } else {
                    (target, below(target))
                };
            self.speleothem_create(block, stalactite, Direction::Down, "tip_merge");
            self.speleothem_create(block, stalagmite, Direction::Up, "tip_merge");
        } else if b.is_air(existing) || b.is_block(existing, b.named.water) {
            self.speleothem_create(block, target, direction, "tip");
        }
    }
    fn speleothem_grow_stalagmite_below(&mut self, block: BlockId, from: BlockPos) {
        let b = self.b;
        let mut pos = from;
        for _ in 0..10 {
            pos = below(pos);
            let state = self.state(pos);
            if b.fluid(state).kind() != FluidKind::Empty {
                return;
            }
            if self.speleothem_unmerged_tip(block, state, Direction::Up)
                && self.speleothem_can_tip_grow(block, state, pos)
            {
                self.speleothem_grow_to(block, pos, Direction::Up);
                return;
            }
            if self.is_valid_speleothem_placement(block, pos, Direction::Up)
                && !self.is_water_at(below(pos))
            {
                self.speleothem_grow_to(block, below(pos), Direction::Up);
                return;
            }
            if b.data(b.default_of(block)).is(Class::PointedDripstone)
                && !self.dripstone_can_drip_through(state)
            {
                return;
            }
        }
    }
    /// `PointedDripstoneBlock.canDripThrough`.
    fn dripstone_can_drip_through(&self, state: BlockStateId) -> bool {
        let b = self.b;
        if b.is_air(state) {
            return true;
        }
        if b.solid_render(state) || b.fluid(state).kind() != FluidKind::Empty {
            return false;
        }
        // REQUIRED_SPACE_TO_DRIP_THROUGH_NON_SOLID_BLOCK = column(4, 0, 16).
        match b.collision_shape(state) {
            None => true,
            Some(shape) => {
                let column = horizon_core::geometry::VoxelShape::from_box(
                    horizon_core::geometry::Aabb::new(
                        horizon_core::geometry::Vec3::new(0.375, 0.0, 0.375),
                        horizon_core::geometry::Vec3::new(0.625, 1.0, 0.625),
                    ),
                );
                column.is_ok_and(|column| {
                    !column
                        .join_is_not_empty(shape, horizon_core::geometry::ShapeOp::AND)
                        .unwrap_or(true)
                })
            }
        }
    }
    /// `PointedDripstoneBlock.getFluidAboveStalactite`: (position above the
    /// root, fluid, state there).
    fn dripstone_fluid_above(
        &self,
        state: BlockStateId,
        pos: BlockPos,
    ) -> Option<(BlockPos, FluidKind, BlockStateId)> {
        let b = self.b;
        if !self.speleothem_with(state, Direction::Down) {
            return None;
        }
        let block = b.block(state);
        let direction = Direction::Down;
        let root = self.find_block_vertical(
            pos,
            true,
            |sim, _, s| {
                sim.b.is_block(s, block)
                    && sim.b.table.dir(s, sim.b.props.vertical_direction) == Some(direction)
            },
            |sim, s| !sim.b.is_block(s, block),
            11,
        )?;
        let above_pos = above(root);
        let above_state = self.state(above_pos);
        let fluid = if b.is_block(above_state, b.named.mud)
            && self.level.dimension() != super::Dimension::Nether
        {
            FluidKind::Water
        } else {
            b.fluid(above_state).kind()
        };
        Some((above_pos, fluid, above_state))
    }
    fn dripstone_transfer_fluid(&mut self, state: BlockStateId, pos: BlockPos, value: f32) {
        let b = self.b;
        if value > 0.175_781_25 {
            return;
        }
        if !self.speleothem_with(state, Direction::Down)
            || b.is_block(self.above_state(pos), b.block(state))
        {
            return;
        }
        let Some((source_pos, fluid, source_state)) = self.dripstone_fluid_above(state, pos) else {
            return;
        };
        let probability = match fluid {
            FluidKind::Water => 0.175_781_25,
            FluidKind::Lava => 0.058_593_75,
            _ => return,
        };
        if value >= probability {
            return;
        }
        let Some(tip) = self.speleothem_find_tip(state, pos, 11, false) else {
            return;
        };
        if b.is_block(source_state, b.named.mud) && fluid == FluidKind::Water {
            self.set_block_and_update(source_pos, b.default_of(b.named.clay));
            self.game_event(source_pos, "minecraft:block_change");
            self.level_event(1504, tip, 0);
        } else if let Some(cauldron) = self.dripstone_find_cauldron(tip, fluid) {
            self.level_event(1504, tip, 0);
            let delay = 50 + tip.y - cauldron.y;
            let block = b.block(self.state(cauldron));
            self.schedule_tick(cauldron, block, delay);
        }
    }
    fn cauldron_can_receive(&self, state: BlockStateId, fluid: FluidKind) -> bool {
        let b = self.b;
        let data = b.data(state);
        if data.is(Class::Cauldron) {
            true
        } else if data.is(Class::LayeredCauldron) {
            fluid == FluidKind::Water && b.is_block(state, b.named.water_cauldron)
        } else {
            false
        }
    }
    fn dripstone_find_cauldron(&self, tip: BlockPos, fluid: FluidKind) -> Option<BlockPos> {
        self.find_block_vertical(
            tip,
            false,
            |sim, _, s| sim.dripstone_can_drip_through(s),
            |sim, s| sim.cauldron_can_receive(s, fluid),
            11,
        )
    }
    /// `AbstractCauldronBlock.tick`: receive a drip from a stalactite above.
    pub(crate) fn cauldron_tick(&mut self, state: BlockStateId, pos: BlockPos) {
        let b = self.b;
        let Some(tip) = self.find_block_vertical(
            pos,
            true,
            |sim, _, s| sim.dripstone_can_drip_through(s),
            |sim, s| sim.is_free_hanging_stalactite(s),
            11,
        ) else {
            return;
        };
        let tip_state = self.state(tip);
        let fluid = self
            .dripstone_fluid_above(tip_state, tip)
            .map(|(_, f, _)| f)
            .filter(|f| matches!(f, FluidKind::Water | FluidKind::Lava))
            .unwrap_or(FluidKind::Empty);
        if fluid == FluidKind::Empty || !self.cauldron_can_receive(state, fluid) {
            return;
        }
        let data = b.data(state);
        if data.is(Class::Cauldron) {
            let (next, event) = if fluid == FluidKind::Water {
                (b.named.water_cauldron, 1047)
            } else {
                (b.named.lava_cauldron, 1046)
            };
            let to = if fluid == FluidKind::Water { 1 } else { 3 };
            if !self
                .level
                .allow_cauldron_level_change(pos, 0, to, "dripstone")
            {
                return;
            }
            self.set_block_and_update(pos, b.default_of(next));
            self.game_event(pos, "minecraft:block_change");
            self.level_event(event, pos, 0);
        } else if data.is(Class::LayeredCauldron) {
            let level = b.table.int(state, b.props.level);
            if level < 3 {
                if !self
                    .level
                    .allow_cauldron_level_change(pos, level, level + 1, "dripstone")
                {
                    return;
                }
                self.set_block_and_update(pos, b.table.set_int(state, b.props.level, level + 1));
                self.game_event(pos, "minecraft:block_change");
                self.level_event(1047, pos, 0);
            }
        }
    }
}
