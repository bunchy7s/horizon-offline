//! Block-state queries shared by the ported behaviour (`BlockStateBase`
//! accessors and `Block` static helpers).
use super::{
    Behaviours, LevelAccess, Sim,
    classes::{Class, Interface},
    data::{BlockSet, state_flags},
    states::{BlockId, PropKey},
    update::{above, below, rel},
};
use crate::{fluid::FluidKind, registry::BlockStateId};
use horizon_core::{geometry::Direction, position::BlockPos};

pub(crate) const fn clockwise(direction: Direction) -> Direction {
    match direction {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        other => other,
    }
}
pub(crate) const fn counter_clockwise(direction: Direction) -> Direction {
    match direction {
        Direction::North => Direction::West,
        Direction::West => Direction::South,
        Direction::South => Direction::East,
        Direction::East => Direction::North,
        other => other,
    }
}
pub(crate) const fn is_horizontal(direction: Direction) -> bool {
    !matches!(direction, Direction::Up | Direction::Down)
}
pub(crate) fn axis_of(direction: Direction) -> horizon_core::geometry::Axis {
    use horizon_core::geometry::Axis;
    match direction {
        Direction::West | Direction::East => Axis::X,
        Direction::Up | Direction::Down => Axis::Y,
        Direction::North | Direction::South => Axis::Z,
    }
}
/// `Direction.from2DDataValue` (S, W, N, E).
pub(crate) fn from_2d(value: i32) -> Direction {
    [
        Direction::South,
        Direction::West,
        Direction::North,
        Direction::East,
    ][value.rem_euclid(4) as usize]
}
pub(crate) fn data_2d(direction: Direction) -> i32 {
    match direction {
        Direction::South => 0,
        Direction::West => 1,
        Direction::North => 2,
        Direction::East => 3,
        _ => -1,
    }
}

impl Behaviours {
    pub(crate) fn in_set(&self, state: BlockStateId, set: &BlockSet) -> bool {
        set.contains(self.table.block(state))
    }
    pub(crate) fn is_block(&self, state: BlockStateId, block: BlockId) -> bool {
        self.table.is(state, block)
    }
    pub(crate) fn instance_of(&self, state: BlockStateId, class: Class) -> bool {
        self.data(state).is(class)
    }
    pub(crate) fn implements(&self, state: BlockStateId, interface: Interface) -> bool {
        self.data(state).implements(interface)
    }
    pub(crate) fn fluid_in(&self, kind: FluidKind, mask: u8) -> bool {
        mask & (1 << kind as u8) != 0
    }
    pub(crate) fn is_water_fluid(&self, kind: FluidKind) -> bool {
        self.fluid_in(kind, self.fluid_tags.water)
    }
    pub(crate) fn is_lava_fluid(&self, kind: FluidKind) -> bool {
        self.fluid_in(kind, self.fluid_tags.lava)
    }
    pub(crate) fn liquid(&self, state: BlockStateId) -> bool {
        self.has_flag(state, state_flags::LIQUID)
    }
    pub(crate) fn solid_render(&self, state: BlockStateId) -> bool {
        self.has_flag(state, state_flags::SOLID_RENDER)
    }
    pub(crate) fn full_collision(&self, state: BlockStateId) -> bool {
        self.has_flag(state, state_flags::FULL_COLLISION)
    }
    /// `Block.isFaceFull(state.getCollisionShape(level, pos), direction)`.
    pub(crate) fn collision_face_full(&self, state: BlockStateId, direction: Direction) -> bool {
        self.shape_info(state).full_faces & (1 << direction as u8) != 0
    }
    /// `MultifaceBlock.canAttachTo(level, direction, neighbourPos, neighbourState)`.
    pub(crate) fn multiface_can_attach(
        &self,
        neighbour: BlockStateId,
        direction: Direction,
    ) -> bool {
        let face = direction.opposite();
        self.is_face_sturdy(neighbour, face) || self.collision_face_full(neighbour, face)
    }
    /// `Block.isExceptionForConnection`.
    pub(crate) fn is_exception_for_connection(&self, state: BlockStateId) -> bool {
        let n = &self.named;
        let block = self.table.block(state);
        self.instance_of(state, Class::Leaves)
            || block == n.barrier
            || block == n.carved_pumpkin
            || block == n.jack_o_lantern
            || block == n.melon
            || block == n.pumpkin
            || self.tags.shulker_boxes.contains(block)
    }
    pub(crate) fn is_waterlogged(&self, state: BlockStateId) -> bool {
        self.table.bool(state, self.props.waterlogged)
    }
    pub(crate) fn facing(&self, state: BlockStateId) -> Direction {
        self.table
            .dir(state, self.props.facing)
            .unwrap_or(Direction::North)
    }
    /// The boolean/enum property named after `direction` (`north`, `up`, ...).
    pub(crate) fn dir_prop(&self, direction: Direction) -> PropKey {
        let p = &self.props;
        match direction {
            Direction::Down => p.down,
            Direction::Up => p.up,
            Direction::North => p.north,
            Direction::South => p.south,
            Direction::West => p.west,
            Direction::East => p.east,
        }
    }
    pub(crate) fn default_of(&self, block: BlockId) -> BlockStateId {
        self.table.default_state(block)
    }
    pub(crate) fn air(&self) -> BlockStateId {
        self.table.default_state(self.named.air)
    }
}

/// Block-entity lookups for dynamic shapes: Horizon's level keeps no shulker
/// box animation state, so boxes read as closed (a full cube).
struct NoBlockEntities;
impl crate::dynamic_support::DynamicSupportWorld for NoBlockEntities {
    type Error = std::convert::Infallible;
    fn shulker_is_closed(&mut self, _position: [i32; 3]) -> Result<Option<bool>, Self::Error> {
        Ok(None)
    }
    fn moving_piston_shape(
        &mut self,
        _position: [i32; 3],
    ) -> Result<Option<horizon_core::geometry::VoxelShape>, Self::Error> {
        Ok(None)
    }
}
impl crate::block_collision::DynamicCollisionWorld for NoBlockEntities {
    fn shulker_collision_state(
        &mut self,
        _position: [i32; 3],
    ) -> Result<Option<crate::block_collision::ShulkerCollisionState>, Self::Error> {
        Ok(None)
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    /// Shape queries over `getCollisionShape(level, pos)`, including the
    /// position-dependent shapes (bamboo, speleothems, shulker boxes).
    pub(crate) fn shape_info_at(
        &self,
        state: BlockStateId,
        pos: BlockPos,
    ) -> super::data::ShapeInfo {
        if self.b.facts(state).shape >= 0 {
            return self.b.shape_info(state);
        }
        match self.b.dynamic.get(&state) {
            Some(support) => support
                .collision_shape(&mut NoBlockEntities, [pos.x, pos.y, pos.z])
                .ok()
                .and_then(|shape| super::data::analyse_shape(&shape).ok())
                .unwrap_or_default(),
            None => super::data::ShapeInfo::default(),
        }
    }
    /// The state whose cached support facts stand in for the block at `pos`.
    /// Vanilla reads a retracting piston's `moving_piston` block through its
    /// block entity, whose collision shape contains the extended base; the
    /// exported facts see no block entity and call every face unsupported,
    /// which made a lever on the piston's back pop off when it retracted.
    fn support_state(&self, pos: BlockPos) -> BlockStateId {
        let state = self.state(pos);
        if self.b.is_block(state, self.b.named.moving_piston) {
            if let Some(piston) = self.moving_piston(pos) {
                // `PistonMovingBlockEntity.getCollisionShape`: only a piston base.
                if piston.source
                    && !piston.extending
                    && (self.b.is_block(piston.moved_state, self.b.named.piston)
                        || self
                            .b
                            .is_block(piston.moved_state, self.b.named.sticky_piston))
                {
                    return self
                        .b
                        .table
                        .set_bool(piston.moved_state, self.b.props.extended, true);
                }
            }
        }
        state
    }
    pub fn sturdy(&self, pos: BlockPos, direction: Direction) -> bool {
        self.b.is_face_sturdy(self.support_state(pos), direction)
    }
    /// `Block.canSupportRigidBlock(level, below)`.
    pub fn can_support_rigid_block(&self, pos: BlockPos) -> bool {
        self.b
            .is_face_sturdy_rigid(self.support_state(pos), Direction::Up)
    }
    /// `Block.canSupportCenter(level, pos, direction)`.
    pub fn can_support_center(&self, pos: BlockPos, direction: Direction) -> bool {
        let state = self.support_state(pos);
        if direction == Direction::Down && self.b.in_set(state, &self.b.tags.unstable_bottom_center)
        {
            return false;
        }
        self.b.is_face_sturdy_center(state, direction)
    }
    pub fn is_empty_block(&self, pos: BlockPos) -> bool {
        self.b.is_air(self.state(pos))
    }
    /// `LevelReader.isWaterAt`: the fluid at `pos` is in the water tag.
    pub fn is_water_at(&self, pos: BlockPos) -> bool {
        self.b.is_water_fluid(self.fluid_at(pos).kind())
    }
    /// `Level.getRawBrightness(pos, darken)`.
    pub fn raw_brightness(&self, pos: BlockPos, darken: i32) -> i32 {
        super::raw_brightness(self.level, pos, darken)
    }
    /// `LevelReader.getMaxLocalRawBrightness(pos)`.
    pub fn max_local_raw_brightness(&self, pos: BlockPos) -> i32 {
        self.raw_brightness(pos, self.level.sky_darken())
    }
    pub fn is_redstone_conductor(&self, state: BlockStateId, _pos: BlockPos) -> bool {
        self.b.has_flag(state, state_flags::CONDUCTOR)
    }
    pub(crate) fn above_state(&self, pos: BlockPos) -> BlockStateId {
        self.state(above(pos))
    }
    pub(crate) fn below_state(&self, pos: BlockPos) -> BlockStateId {
        self.state(below(pos))
    }
    pub(crate) fn rel_state(&self, pos: BlockPos, direction: Direction) -> BlockStateId {
        self.state(rel(pos, direction))
    }
    pub(crate) fn is_inside_build_height(&self, y: i32) -> bool {
        y >= self.level.min_y() && y < self.level.max_y()
    }
}
