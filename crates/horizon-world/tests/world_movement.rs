//! The candidate budget bounds a scan, never a movement: a movement big enough
//! to pass the caller's budget is walked in full, as vanilla's uncounted
//! collision walk does (#6124).
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    block_collision_scan::{BlockCollisionWorld, CollisionBlock, scan_cells},
    collision_context::CollisionContext,
    entity_obstruction::EntityIdentity,
    registry::BlockStateId,
    world_movement::{EntityMovementWorld, MovementRequest, resolve_world_movement},
};
use std::io;

/// Open air: every cell is reachable and has no shape, so a scan visits all of
/// them and the element count is the walk it performed.
struct OpenAir {
    cells: usize,
}
impl BlockCollisionWorld for OpenAir {
    type Error = io::Error;
    type Chunk = ();
    fn collision_chunk(&mut self, _: ChunkPos) -> io::Result<Option<()>> {
        Ok(Some(()))
    }
    fn collision_block(&mut self, _: (), _: BlockPos) -> io::Result<CollisionBlock> {
        self.cells += 1;
        Ok(CollisionBlock {
            state: BlockStateId(0),
            large_collision_shape: false,
            moving_piston: false,
            empty_shape: true,
        })
    }
    fn is_suffocating(&mut self, _: (), _: CollisionBlock, _: BlockPos) -> io::Result<bool> {
        unreachable!()
    }
    fn context_shape(
        &mut self,
        _: CollisionBlock,
        _: BlockPos,
        _: &CollisionContext<'_>,
    ) -> io::Result<VoxelShape> {
        unreachable!()
    }
}
impl EntityMovementWorld for OpenAir {
    type Error = io::Error;
    fn entity_collisions(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Vec<VoxelShape>> {
        Ok(Vec::new())
    }
    fn movement_border(&mut self, _: EntityIdentity, _: Aabb) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}

fn player() -> Aabb {
    Aabb::new(Vec3::new(0.5, 64.0, 0.5), Vec3::new(1.1, 65.8, 1.1))
}

fn resolve(movement: Vec3, candidate_budget: usize) -> (Vec3, OpenAir) {
    let mut world = OpenAir { cells: 0 };
    let resolved = resolve_world_movement(
        &mut world,
        MovementRequest {
            entity: EntityIdentity(0),
            context: CollisionContext::empty(false),
            bounds: player(),
            movement,
            max_step: 0.6,
            on_ground: true,
            candidate_budget,
        },
    )
    .expect("a movement is never aborted by the candidate budget");
    (resolved, world)
}

#[test]
fn a_diagonal_sweep_past_the_budget_is_walked_in_full() {
    // 40 blocks each way pads to about 42 x 4 x 42 cells, far past the budget.
    let movement = Vec3::new(40.0, 0.0, 40.0);
    let sweep = scan_cells(player().expand_towards(movement)).expect("a scannable box");
    assert!(sweep > 1_000, "the sweep is {sweep} cells");
    let (resolved, world) = resolve(movement, 10);
    assert_eq!(resolved, movement);
    assert!(
        world.cells > 0,
        "the whole sweep was walked: {} cells",
        world.cells
    );
}

#[test]
fn scan_cells_covers_what_the_scan_visits() {
    // The count includes the corners the cursor skips, so it is an upper bound
    // on the walk that is never short of it by more than the eight corners.
    for movement in [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, -0.5, 2.0),
        Vec3::new(40.0, 3.0, 40.0),
    ] {
        let (_, world) = resolve(movement, 0);
        let sweep = scan_cells(player().expand_towards(movement)).expect("a scannable box");
        if movement.length_squared() == 0.0 {
            // A zero movement is never scanned.
            assert_eq!(world.cells, 0);
        } else {
            assert!(world.cells <= sweep, "movement {movement:?}");
            assert!(sweep - world.cells <= 8, "movement {movement:?}");
        }
    }
}

#[test]
fn ordinary_movements_are_unchanged() {
    // Resolved movement does not depend on the budget once it covers the sweep.
    for movement in [
        Vec3::new(0.2, -0.0784, 0.1),
        Vec3::new(1.0, 0.0, -3.0),
        Vec3::new(0.0, -0.5, 0.0),
    ] {
        assert_eq!(
            resolve(movement, 1_000_000).0,
            resolve(movement, 1).0,
            "movement {movement:?}"
        );
    }
}
