//! Collision queries against the same resident chunks used by the renderer.
//! The development scene has no entities, block entities or world border.
//! Entity-dependent rules use the defaults for players and the supported mobs.
use crate::{Result, world::World};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape, border::BorderGeometry},
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
    block_collision_scan::{BlockCollisionWorld, CollisionBlock},
    collision::CachedCollisionRegistry,
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    dynamic_support::DynamicSupportWorld,
    entity_obstruction::EntityIdentity,
    fluid::{FluidKind, FluidState},
    liquid_collision::LiquidCollisionWorld,
    piston_shape::PistonCollisionWorld,
    powder_snow_collision::{PowderSnowEntity, PowderSnowWorld},
    registry::{BlockRegistry, BlockStateId, BlockTableFile},
    world_movement::{
        CollisionCheckRequest, EntityMovementWorld, MovementRequest, no_world_collision,
        resolve_world_movement,
    },
};
use std::{io, path::Path, sync::Arc};

pub fn load<'r>(root: &Path, registry: &'r BlockRegistry) -> Result<BlockCollisionRegistry<'r>> {
    let file = BlockTableFile::open(&root.join("cached-collision.nbt"), registry)?;
    let cached = CachedCollisionRegistry::from_table(file.table("collision")?)?;
    if !cached.has_canonical_identities() {
        return Err("scene collisions require canonical shape identities".into());
    }
    Ok(BlockCollisionRegistry::new(Arc::new(cached))?)
}

/// What powder snow's and liquids' contextual collisions ask of the colliding
/// entity (`PowderSnowBlock` and `LiquidBlock.getCollisionShape`).
#[derive(Clone, Copy, Debug, Default)]
pub struct PowderSnowSubject {
    pub fall_distance: f64,
    /// `FallingBlockEntity`.
    pub falling_block: bool,
    /// A living entity with leather boots on its feet.
    pub leather_boots: bool,
    /// `EntityTypeTags.POWDER_SNOW_WALKABLE_MOBS`.
    pub walkable_type: bool,
    /// A strider: lava sources hold it up (`canStandOnFluid`), on its
    /// `getLiquidCollisionShape`.
    pub lava_walker: bool,
}
/// `Strider.getLiquidCollisionShape`: `Block.column(16.0, 0.0, 8.0)`.
fn strider_liquid_shape() -> VoxelShape {
    VoxelShape::from_cube([1, 2, 1], &[true, false]).expect("a valid half-block column")
}

// Vanilla scopes this direction to Entity.move while a piston pushes it.
thread_local! {
    static PISTON_NOCLIP: std::cell::Cell<Option<horizon_core::geometry::Direction>> = const { std::cell::Cell::new(None) };
}

/// Suppress the moving part of pistons moving in this direction during a push.
pub fn with_piston_noclip<T>(
    direction: horizon_core::geometry::Direction,
    f: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<horizon_core::geometry::Direction>);
    impl Drop for Reset {
        fn drop(&mut self) {
            PISTON_NOCLIP.with(|value| value.set(self.0));
        }
    }
    let previous = PISTON_NOCLIP.with(|value| value.replace(Some(direction)));
    let _reset = Reset(previous);
    f()
}

pub struct SceneCollisions<'w, 'r> {
    pub world: &'w World,
    pub binding: &'w BlockCollisionRegistry<'r>,
    /// The entity colliding, for powder snow; None: no fall, no boots.
    pub subject: Option<PowderSnowSubject>,
}
/// Already selected entity boxes alongside the resident block collision world.
pub struct EntityCollisions<'a, W> {
    pub blocks: W,
    pub entities: &'a [Aabb],
    /// The world border and the moving entity's x and z
    /// (`CollisionGetter.borderCollision`); None: the blocks' answer.
    pub border: Option<(&'a BorderGeometry, [f64; 2])>,
}
impl<W: BlockCollisionWorld> BlockCollisionWorld for EntityCollisions<'_, W> {
    type Error = W::Error;
    type Chunk = W::Chunk;
    fn collision_chunk(
        &mut self,
        p: ChunkPos,
    ) -> std::result::Result<Option<Self::Chunk>, Self::Error> {
        self.blocks.collision_chunk(p)
    }
    fn collision_block(
        &mut self,
        chunk: Self::Chunk,
        p: BlockPos,
    ) -> std::result::Result<CollisionBlock, Self::Error> {
        self.blocks.collision_block(chunk, p)
    }
    fn is_suffocating(
        &mut self,
        chunk: Self::Chunk,
        block: CollisionBlock,
        p: BlockPos,
    ) -> std::result::Result<bool, Self::Error> {
        self.blocks.is_suffocating(chunk, block, p)
    }
    fn may_reach_out(
        &mut self,
        chunk: Self::Chunk,
        section_y: i32,
    ) -> std::result::Result<bool, Self::Error> {
        self.blocks.may_reach_out(chunk, section_y)
    }
    fn context_shape(
        &mut self,
        block: CollisionBlock,
        p: BlockPos,
        context: &CollisionContext<'_>,
    ) -> std::result::Result<VoxelShape, Self::Error> {
        self.blocks.context_shape(block, p, context)
    }
}
impl<W: EntityMovementWorld<Error = io::Error>> EntityMovementWorld for EntityCollisions<'_, W> {
    type Error = io::Error;
    fn entity_collisions(
        &mut self,
        entity: EntityIdentity,
        bounds: Aabb,
    ) -> io::Result<Vec<VoxelShape>> {
        let mut shapes = self.blocks.entity_collisions(entity, bounds)?;
        for &aabb in self.entities {
            // EntityGetter.getEntityCollisions expands its entity query by 1e-7.
            if aabb.intersects(bounds.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7))) {
                shapes.push(VoxelShape::from_box(aabb).map_err(io::Error::other)?);
            }
        }
        Ok(shapes)
    }
    /// `WorldBorder.isInsideCloseToBorder`: an entity inside the border and
    /// near it collides with its walls.
    fn movement_border(
        &mut self,
        entity: EntityIdentity,
        bounds: Aabb,
    ) -> io::Result<Option<VoxelShape>> {
        let Some((border, [x, z])) = self.border else {
            return self.blocks.movement_border(entity, bounds);
        };
        if !border.is_inside_close_to_border(x, z, bounds) {
            return Ok(None);
        }
        border.collision_shape().map(Some).map_err(io::Error::other)
    }
}
impl<'a, W> EntityCollisions<'a, W>
where
    W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
{
    /// `EntityCollisions` for a body the world border also stops: the border
    /// and the body's own x and z (`WorldBorder.isInsideCloseToBorder`).
    pub fn with_border(blocks: W, border: &'a BorderGeometry, at: [f64; 2]) -> Self {
        Self {
            blocks,
            entities: &[],
            border: Some((border, at)),
        }
    }
    /// `SceneCollisions::resolve` that also stops at the entity boxes and,
    /// with a border, at the world border's walls.
    pub fn resolve(
        &mut self,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
    ) -> Result<Vec3> {
        Ok(resolve_world_movement(
            self,
            MovementRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(bounds.min_position().y),
                bounds,
                movement,
                max_step,
                on_ground,
                candidate_budget: 1_000_000,
            },
        )?)
    }
}
impl SceneCollisions<'_, '_> {
    /// Position contexts permit geometric movement probes without inventing a
    /// player, inventory, effects or fluid-standing behavior; a lava walker
    /// collides as the living entity it is, empty-handed.
    pub fn resolve(
        &mut self,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
    ) -> Result<Vec3> {
        self.resolve_with_context(bounds, movement, max_step, on_ground, self.context(bounds))
    }
    /// Resolve movement with the entity's captured collision context. Most
    /// callers only need the position context; entities such as falling blocks
    /// also affect contextual block shapes (for example powder snow).
    pub fn resolve_with_context(
        &mut self,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
        context: CollisionContext<'_>,
    ) -> Result<Vec3> {
        Ok(resolve_world_movement(
            self,
            MovementRequest {
                entity: EntityIdentity(0),
                context,
                bounds,
                movement,
                max_step,
                on_ground,
                candidate_budget: 1_000_000,
            },
        )?)
    }
    fn context(&self, bounds: Aabb) -> CollisionContext<'static> {
        let y = bounds.min_position().y;
        if !self.subject.is_some_and(|s| s.lava_walker) {
            return CollisionContext::position(y);
        }
        CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                y,
                descending: false,
                main_hand: Some(CollisionItem::EMPTY),
            },
            false,
        )
    }
    pub fn unobstructed(&mut self, bounds: Aabb) -> Result<bool> {
        Ok(no_world_collision(
            self,
            CollisionCheckRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(bounds.min_position().y),
                bounds,
                candidate_budget: 1_000_000,
            },
        )?)
    }
}
/// `noCollision(null, box)`: block collisions under the empty collision
/// context, as entity-less queries such as spawn checks use.
pub fn no_collision_empty<W, E>(world: &mut W, bounds: Aabb) -> bool
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    no_collision_in(world, bounds, false)
}
/// `noCollision(null, box, true)`: the same with liquids colliding as
/// blocks (`CollisionContext.emptyWithFluidCollisions`).
pub fn no_collision_or_liquid<W, E>(world: &mut W, bounds: Aabb) -> bool
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    no_collision_in(world, bounds, true)
}
fn no_collision_in<W, E>(world: &mut W, bounds: Aabb, fluids: bool) -> bool
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    no_world_collision(
        world,
        CollisionCheckRequest {
            entity: EntityIdentity(0),
            context: CollisionContext::empty(fluids),
            bounds,
            candidate_budget: 1_000_000,
        },
    )
    .unwrap_or(false)
}
impl<'w> BlockCollisionWorld for SceneCollisions<'w, '_> {
    type Error = io::Error;
    /// The resident column, looked up once per chunk the scan enters.
    type Chunk = &'w crate::chunk::Chunk;
    fn collision_chunk(&mut self, p: ChunkPos) -> io::Result<Option<Self::Chunk>> {
        Ok(self.world.chunks.get(&(p.x, p.z)).map(|chunk| &**chunk))
    }
    fn collision_block(&mut self, chunk: Self::Chunk, p: BlockPos) -> io::Result<CollisionBlock> {
        self.binding.collision_block(chunk.block(p.x, p.y, p.z))
    }
    /// From the section's palette, without reading its blocks; a palette
    /// of the whole registry reads every cell.
    fn may_reach_out(&mut self, chunk: Self::Chunk, section_y: i32) -> io::Result<bool> {
        let blocks = usize::try_from(section_y - (chunk.min_y() >> 4))
            .ok()
            .and_then(|s| chunk.sections.get(s))
            .map(|section| &section.blocks);
        Ok(blocks.is_none_or(|blocks| {
            blocks.is_global() || blocks.maybe_has(|v| self.binding.reaches_out(BlockStateId(v)))
        }))
    }
    fn is_suffocating(
        &mut self,
        _: Self::Chunk,
        _: CollisionBlock,
        _: BlockPos,
    ) -> io::Result<bool> {
        Ok(false)
    }
    fn context_shape(
        &mut self,
        block: CollisionBlock,
        p: BlockPos,
        context: &CollisionContext<'_>,
    ) -> io::Result<VoxelShape> {
        self.binding
            .collision_shape_with_context(self, block.state, p, context)
            .map_err(io::Error::other)
    }
}
impl EntityMovementWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    fn entity_collisions(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Vec<VoxelShape>> {
        Ok(Vec::new())
    }
    fn movement_border(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}
impl DynamicSupportWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    /// `ShulkerBoxBlockEntity.isClosed`: whether the lid is still shut. A
    /// closed box collides as its plain block (`ShulkerBoxBlock.getShape`
    /// asks the block entity, which has grown once the lid starts to open).
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> io::Result<Option<bool>> {
        Ok(self
            .world
            .shulkers
            .get(&p)
            .map(|s| s.status == crate::block_entity_anim::ShulkerStatus::Closed))
    }
    /// `MovingPistonBlock.getCollisionShape`: the block entity's moving shape
    /// (`PistonMovingBlockEntity.getCollisionShape`), outside any push.
    fn moving_piston_shape(&mut self, p: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        let Some(&(_, piston)) = self.world.moving_pistons.iter().find(|(q, _)| *q == p) else {
            return Ok(None);
        };
        let binding = self.binding;
        let Some(pistons) = binding.pistons() else {
            return Ok(None);
        };
        pistons
            .collision_shape(self, p, piston, PISTON_NOCLIP.with(|value| value.get()))
            .map(Some)
            .map_err(io::Error::other)
    }
}
impl PistonCollisionWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    /// `BlockState.getCollisionShape(level, pos)` of the moved state.
    fn collision_shape(&mut self, state: BlockStateId, p: [i32; 3]) -> io::Result<VoxelShape> {
        let binding = self.binding;
        binding
            .collision_shape(self, state, p)
            .map_err(io::Error::other)
    }
}
impl DynamicCollisionWorld for SceneCollisions<'_, '_> {
    /// `ShulkerBoxBlockEntity.getBoundingBox`: the box's collision shape
    /// (`getShape`) is the free shape grown by the lid's progress, which is
    /// what lets an entity sink into an opening box. None: no such box here.
    fn shulker_collision_state(
        &mut self,
        p: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        Ok(self.world.shulkers.get(&p).map(|s| ShulkerCollisionState {
            previous_progress: s.progress_old,
            progress: s.progress,
        }))
    }
}
impl LiquidCollisionWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    /// Only striders stand on fluids.
    fn entity_liquid_shape(&mut self, _: EntityIdentity) -> io::Result<VoxelShape> {
        Ok(if self.subject.is_some_and(|s| s.lava_walker) {
            strider_liquid_shape()
        } else {
            VoxelShape::empty()
        })
    }
    fn fluid(&mut self, p: BlockPos) -> io::Result<FluidState> {
        FluidState::from_block(
            self.binding
                .registry()
                .state(self.world.block(p.x, p.y, p.z))
                .ok_or_else(|| io::Error::other("scene fluid state outside registry"))?,
        )
    }
    /// `Strider.canStandOnFluid`: lava.
    fn entity_can_stand(&mut self, _: EntityIdentity, fluid: FluidState) -> io::Result<bool> {
        Ok(self.subject.is_some_and(|s| s.lava_walker)
            && matches!(fluid.kind(), FluidKind::Lava | FluidKind::FlowingLava))
    }
}
impl PowderSnowWorld for SceneCollisions<'_, '_> {
    type Error = io::Error;
    fn powder_snow_entity(&mut self, _: EntityIdentity) -> io::Result<PowderSnowEntity> {
        let subject = self.subject.unwrap_or_default();
        Ok(PowderSnowEntity {
            fall_distance: subject.fall_distance,
            falling_block: subject.falling_block,
        })
    }
    fn powder_snow_walkable_tag(&mut self, _: EntityIdentity) -> io::Result<bool> {
        Ok(self.subject.is_some_and(|s| s.walkable_type))
    }
    fn feet_are_leather_boots(&mut self, _: EntityIdentity) -> io::Result<bool> {
        Ok(self.subject.is_some_and(|s| s.leather_boots))
    }
}

/// A collision world without blocks: no chunk is resident, so only the
/// world border stops a body.
pub struct EmptyCollisions;

impl BlockCollisionWorld for EmptyCollisions {
    type Error = io::Error;
    type Chunk = ChunkPos;
    fn collision_chunk(&mut self, _: ChunkPos) -> io::Result<Option<ChunkPos>> {
        Ok(None)
    }
    fn collision_block(&mut self, _: ChunkPos, _: BlockPos) -> io::Result<CollisionBlock> {
        Err(io::Error::other("no chunk is resident"))
    }
    fn is_suffocating(&mut self, _: ChunkPos, _: CollisionBlock, _: BlockPos) -> io::Result<bool> {
        Ok(false)
    }
    fn context_shape(
        &mut self,
        _: CollisionBlock,
        _: BlockPos,
        _: &CollisionContext<'_>,
    ) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
}

impl EntityMovementWorld for EmptyCollisions {
    type Error = io::Error;
    fn entity_collisions(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Vec<VoxelShape>> {
        Ok(Vec::new())
    }
    fn movement_border(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}

/// Collision queries over any block lookup (test worlds without chunks):
/// every chunk is present, nothing is suffocating, no entities collide.
pub struct LookupCollisions<'w, 'r> {
    pub block: &'w dyn Fn(i32, i32, i32) -> horizon_world::registry::BlockStateId,
    pub binding: &'w BlockCollisionRegistry<'r>,
}
impl BlockCollisionWorld for LookupCollisions<'_, '_> {
    type Error = io::Error;
    type Chunk = ChunkPos;
    fn collision_chunk(&mut self, p: ChunkPos) -> io::Result<Option<ChunkPos>> {
        Ok(Some(p))
    }
    fn collision_block(&mut self, _: ChunkPos, p: BlockPos) -> io::Result<CollisionBlock> {
        self.binding.collision_block((self.block)(p.x, p.y, p.z))
    }
    fn is_suffocating(&mut self, _: ChunkPos, _: CollisionBlock, _: BlockPos) -> io::Result<bool> {
        Ok(false)
    }
    fn context_shape(
        &mut self,
        block: CollisionBlock,
        p: BlockPos,
        context: &CollisionContext<'_>,
    ) -> io::Result<VoxelShape> {
        self.binding
            .collision_shape_with_context(self, block.state, p, context)
            .map_err(io::Error::other)
    }
}
impl EntityMovementWorld for LookupCollisions<'_, '_> {
    type Error = io::Error;
    fn entity_collisions(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Vec<VoxelShape>> {
        Ok(Vec::new())
    }
    fn movement_border(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}
impl DynamicSupportWorld for LookupCollisions<'_, '_> {
    type Error = io::Error;
    /// No block-entity data: a lookup world has no shulker box to ask, so the
    /// box collides as a shut one (`VoxelShape::block()`).
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> io::Result<Option<bool>> {
        Ok(None)
    }
    fn moving_piston_shape(&mut self, _: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}
impl DynamicCollisionWorld for LookupCollisions<'_, '_> {
    /// No block-entity data: the block table's own shape stands in for the lid
    /// (`ShulkerBoxBlockEntity.getBoundingBox` needs a block entity).
    fn shulker_collision_state(
        &mut self,
        _: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        Ok(None)
    }
}
impl LiquidCollisionWorld for LookupCollisions<'_, '_> {
    type Error = io::Error;
    fn entity_liquid_shape(&mut self, _: EntityIdentity) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
    fn fluid(&mut self, p: BlockPos) -> io::Result<FluidState> {
        FluidState::from_block(
            self.binding
                .registry()
                .state((self.block)(p.x, p.y, p.z))
                .ok_or_else(|| io::Error::other("fluid state outside registry"))?,
        )
    }
    fn entity_can_stand(&mut self, _: EntityIdentity, _: FluidState) -> io::Result<bool> {
        Ok(false)
    }
}
impl PowderSnowWorld for LookupCollisions<'_, '_> {
    type Error = io::Error;
    fn powder_snow_entity(&mut self, _: EntityIdentity) -> io::Result<PowderSnowEntity> {
        Ok(PowderSnowEntity {
            fall_distance: 0.0,
            falling_block: false,
        })
    }
    fn powder_snow_walkable_tag(&mut self, _: EntityIdentity) -> io::Result<bool> {
        Ok(false)
    }
    fn feet_are_leather_boots(&mut self, _: EntityIdentity) -> io::Result<bool> {
        Ok(false)
    }
}

#[cfg(test)]
mod piston_noclip_tests {
    use super::*;
    use horizon_core::geometry::Direction;

    #[test]
    fn piston_noclip_is_scoped_and_restored_after_unwind() {
        assert_eq!(PISTON_NOCLIP.with(|value| value.get()), None);
        with_piston_noclip(Direction::East, || {
            assert_eq!(
                PISTON_NOCLIP.with(|value| value.get()),
                Some(Direction::East)
            );
            let _ =
                std::panic::catch_unwind(|| with_piston_noclip(Direction::West, || panic!("test")));
            assert_eq!(
                PISTON_NOCLIP.with(|value| value.get()),
                Some(Direction::East)
            );
        });
        assert_eq!(PISTON_NOCLIP.with(|value| value.get()), None);
    }
}

#[cfg(test)]
mod border_movement_tests {
    use super::*;

    /// A border 10 wide around the origin: its wall's inner face is at x = 5.
    fn border() -> BorderGeometry {
        BorderGeometry::new([0.0, 0.0], 10.0, 29_999_984).expect("a border")
    }

    /// `WorldBorder.isInsideCloseToBorder` and `WorldBorder.getCollisionShape`:
    /// a body inside close to the border collides with the border's walls.
    #[test]
    fn a_body_next_to_the_border_stops_at_its_wall() {
        // A body 0.6 wide, 0.4 blocks short of the wall, moved east into it.
        let bounds = Aabb::new(Vec3::new(4.0, 0.0, 0.0), Vec3::new(4.6, 1.8, 0.6));
        let border = border();
        let mut collisions = EntityCollisions::with_border(EmptyCollisions, &border, [4.3, 0.3]);
        let moved = collisions
            .resolve(bounds, Vec3::new(0.51, 0.0, 0.0), 0.0, false)
            .expect("the movement resolves");
        // The wall stops the body 0.4 blocks short of the whole move.
        assert!(
            (moved.x - 0.4).abs() < 1.0e-9,
            "the body was not stopped by the border: {moved:?}"
        );
    }

    /// Away from the border, the same body moves the whole way.
    #[test]
    fn a_body_away_from_the_border_moves_whole() {
        let bounds = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.6, 1.8, 0.6));
        let border = border();
        let mut collisions = EntityCollisions::with_border(EmptyCollisions, &border, [0.3, 0.3]);
        let moved = collisions
            .resolve(bounds, Vec3::new(0.51, 0.0, 0.0), 0.0, false)
            .expect("the movement resolves");
        assert!(
            (moved.x - 0.51).abs() < 1.0e-9,
            "the body did not move the whole way: {moved:?}"
        );
    }
}

#[cfg(test)]
mod shulker_collision_tests {
    use super::*;
    use crate::block_entity_anim::{Shulker, ShulkerStatus, progress_delta_aabb};
    use horizon_core::geometry::{Aabb, Axis, Direction, Vec3, VoxelShape};

    /// A shulker box facing up at progress 0.0 (closed): collision is a 1x1x1 block.
    #[test]
    fn shulker_closed_collision_is_full_block() {
        let shulker = Shulker {
            status: ShulkerStatus::Closed,
            progress: 0.0,
            progress_old: 0.0,
            ..Shulker::default()
        };

        let state = ShulkerCollisionState {
            previous_progress: shulker.progress_old,
            progress: shulker.progress,
        };
        let aabb = state.bounding_box(Direction::Up);

        // Closed box: height 1.0
        assert!((aabb.min(Axis::Y) - 0.0).abs() < 1.0e-9);
        assert!((aabb.max(Axis::Y) - 1.0).abs() < 1.0e-9);
        // Full horizontal extent
        assert!((aabb.min(Axis::X) - 0.0).abs() < 1.0e-9);
        assert!((aabb.max(Axis::X) - 1.0).abs() < 1.0e-9);
    }

    /// A shulker box facing up at progress 1.0 (fully open): collision is 1.5 tall.
    #[test]
    fn shulker_open_collision_grows_upward() {
        let shulker = Shulker {
            status: ShulkerStatus::Opened,
            progress: 1.0,
            progress_old: 1.0,
            ..Shulker::default()
        };

        let state = ShulkerCollisionState {
            previous_progress: shulker.progress_old,
            progress: shulker.progress,
        };
        let aabb = state.bounding_box(Direction::Up);

        // Fully open: height 1.5 (lid extends 0.5 above the block)
        assert!((aabb.min(Axis::Y) - 0.0).abs() < 1.0e-9);
        assert!((aabb.max(Axis::Y) - 1.5).abs() < 1.0e-9);
    }

    /// A shulker box halfway open: collision height is 1.25.
    #[test]
    fn shulker_half_open_collision_height() {
        let shulker = Shulker {
            status: ShulkerStatus::Opening,
            progress: 0.5,
            progress_old: 0.5,
            ..Shulker::default()
        };

        let state = ShulkerCollisionState {
            previous_progress: shulker.progress_old,
            progress: shulker.progress,
        };
        let aabb = state.bounding_box(Direction::Up);

        // Half open: height 1.25
        assert!((aabb.min(Axis::Y) - 0.0).abs() < 1.0e-9);
        assert!((aabb.max(Axis::Y) - 1.25).abs() < 1.0e-9);
    }

    /// Vanilla clip quirk: an entity whose feet are *inside* the grown lid box
    /// moving DOWN is not stopped by the box (collision only triggers when
    /// approaching from outside). An entity ABOVE the box moving DOWN is stopped.
    #[test]
    fn shulker_grown_box_allows_clip_when_inside() {
        // Box at progress 1.0: spans y=0..1.5
        let shulker = Shulker {
            status: ShulkerStatus::Opened,
            progress: 1.0,
            progress_old: 1.0,
            ..Shulker::default()
        };
        let state = ShulkerCollisionState {
            previous_progress: shulker.progress_old,
            progress: shulker.progress,
        };
        let aabb = state.bounding_box(Direction::Up);
        let box_shape = VoxelShape::from_box(aabb).expect("shulker box is a valid shape");

        // Entity box whose feet are at y=0.5 (inside the grown box), height 1.8
        let entity = Aabb::new(Vec3::new(0.2, 0.5, 0.2), Vec3::new(0.8, 2.3, 0.8));

        // Moving DOWN from inside the box: collide() should NOT stop it
        // (the entity is already intersecting the box, so the downward movement
        // sees no face to hit from above).
        let down = -0.1;
        let moved_down = box_shape.collide(Axis::Y, entity, down);
        assert!(
            (moved_down - down).abs() < 1.0e-9,
            "entity inside grown box should fall through (clip): moved_down={moved_down}, expected={down}"
        );

        // Entity box whose feet are at y=1.6 (just ABOVE the grown box)
        let entity_above = Aabb::new(Vec3::new(0.2, 1.6, 0.2), Vec3::new(0.8, 3.4, 0.8));

        // Moving DOWN from above: collide() SHOULD stop it at y=1.5
        let moved_down_above = box_shape.collide(Axis::Y, entity_above, -0.2);
        // The box's top is at y=1.5, entity bottom at 1.6, so max down is -0.1
        assert!(
            (moved_down_above - (-0.1)).abs() < 1.0e-9,
            "entity above grown box should be stopped: moved_down_above={moved_down_above}, expected=-0.1"
        );
    }

    /// An entity standing on the lid (y=1.0 when closed, y=1.5 when open)
    /// and pushed up by the lid: the lid's swept AABB pushes it along the facing.
    /// This is tested at the integration level via `push_shulker_entities`;
    /// here we only verify the swept box geometry matches vanilla's
    /// `Shulker.getProgressDeltaAabb`.
    #[test]
    fn shulker_swept_box_matches_vanilla_delta() {
        // From closed (0.0) to progress 0.5: the lid sweeps from y=0..1 (closed box)
        // to y=0..1.5 (half open), so the swept volume in world coords at position
        // [0.5, 0.0, 0.5] (bottom center of block at y=0) is y=1.0..1.5.
        let delta = progress_delta_aabb(1.0, Direction::Up as u8, 0.0, 0.5, [0.5, 0.0, 0.5]);
        assert!((delta[1] - 1.0).abs() < 1.0e-9); // minY
        assert!((delta[4] - 1.5).abs() < 1.0e-9); // maxY

        // From progress 0.5 to 1.0: swept y=1.5..2.0 in world coords
        let delta2 = progress_delta_aabb(1.0, Direction::Up as u8, 0.5, 1.0, [0.5, 0.0, 0.5]);
        assert!((delta2[1] - 1.5).abs() < 1.0e-9);
        assert!((delta2[4] - 2.0).abs() < 1.0e-9);
    }
}
