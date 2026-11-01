//! `PistonMovingBlockEntity.moveCollidedEntities`, `moveStuckEntities` and
//! `fixEntityWithinPistonBase` over immutable geometry.
//!
//! Entity storage, the moving entities themselves and the blocks they collide
//! with belong to the caller: these functions decide *how far* an entity is
//! pushed, which is the part that must match vanilla to the bit. The movement
//! they describe is an `Entity.move(MoverType.PISTON, …)`, so the caller still
//! applies it through its own collision resolution and per-tick piston limit.
use crate::{level::data::PushReaction, piston_shape::MovingPistonState};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
};

/// `PistonMovingBlockEntity.PUSH_OFFSET`.
pub const PUSH_OFFSET: f64 = 0.01;

/// `PistonMovingBlockEntity.getExtendedProgress`: where along its travel the
/// moved block is, in blocks (negative before it starts, 0 when it arrives).
pub fn extended_progress(piston: MovingPistonState, progress: f32) -> f32 {
    if piston.extending {
        progress - 1.0
    } else {
        1.0 - progress
    }
}

/// `Direction.getAxisDirection().getStep()`.
fn step_along(direction: Direction) -> i32 {
    match direction {
        Direction::East | Direction::Up | Direction::South => 1,
        Direction::West | Direction::Down | Direction::North => -1,
    }
}

/// `Direction.getStepX/Y/Z` as a vector.
pub fn step_vector(direction: Direction) -> Vec3 {
    let [x, y, z] = direction.normal();
    Vec3::new(f64::from(x), f64::from(y), f64::from(z))
}

/// `PistonMath.getMovementArea`: the slab an entity's box must reach into to be
/// run over by `amount` of further travel along `direction`.
pub fn movement_area(aabb: Aabb, direction: Direction, amount: f64) -> Aabb {
    let delta = amount * f64::from(step_along(direction));
    let low = delta.min(0.0);
    let high = delta.max(0.0);
    let (min, max) = (aabb.min_position(), aabb.max_position());
    match direction {
        Direction::West => Aabb::new(
            Vec3::new(min.x + low, min.y, min.z),
            Vec3::new(min.x + high, max.y, max.z),
        ),
        Direction::East => Aabb::new(
            Vec3::new(max.x + low, min.y, min.z),
            Vec3::new(max.x + high, max.y, max.z),
        ),
        Direction::Down => Aabb::new(
            Vec3::new(min.x, min.y + low, min.z),
            Vec3::new(max.x, min.y + high, max.z),
        ),
        Direction::Up => Aabb::new(
            Vec3::new(min.x, max.y + low, min.z),
            Vec3::new(max.x, max.y + high, max.z),
        ),
        Direction::North => Aabb::new(
            Vec3::new(min.x, min.y, min.z + low),
            Vec3::new(max.x, max.y, min.z + high),
        ),
        Direction::South => Aabb::new(
            Vec3::new(min.x, min.y, max.z + low),
            Vec3::new(max.x, max.y, max.z + high),
        ),
    }
}

/// `PistonMovingBlockEntity.moveByPositionAndProgress`: a box moved to the
/// block position and on to where the moved block currently is.
pub fn move_by_position_and_progress(pos: BlockPos, aabb: Aabb, piston: MovingPistonState) -> Aabb {
    let current = f64::from(extended_progress(piston, piston.progress));
    let step = step_vector(piston.direction);
    aabb.move_by(Vec3::new(
        f64::from(pos.x) + current * step.x,
        f64::from(pos.y) + current * step.y,
        f64::from(pos.z) + current * step.z,
    ))
}

/// `PistonMovingBlockEntity.getMovement`: how far `entity` stands inside the
/// block it must be pushed out of, measured along `movement`'s axis.
pub fn get_movement(block: Aabb, movement: Direction, entity: Aabb) -> f64 {
    let (bmin, bmax) = (block.min_position(), block.max_position());
    let (emin, emax) = (entity.min_position(), entity.max_position());
    match movement {
        Direction::East => bmax.x - emin.x,
        Direction::West => emax.x - bmin.x,
        Direction::Up => bmax.y - emin.y,
        Direction::Down => emax.y - bmin.y,
        Direction::South => bmax.z - emin.z,
        Direction::North => emax.z - bmin.z,
    }
}

/// `moveCollidedEntities`' inner loop: the furthest any of the moved block's
/// boxes reaches into `entity`, capped at the progress delta, plus
/// [`PUSH_OFFSET`]. `0.0` when the block does not touch the entity.
pub fn collided_amount(
    boxes: &[Aabb],
    pos: BlockPos,
    piston: MovingPistonState,
    new_progress: f32,
    movement: Direction,
    entity: Aabb,
) -> f64 {
    let delta_progress = f64::from(new_progress - piston.progress);
    let mut delta: f64 = 0.0;
    for block in boxes {
        let moving = movement_area(
            move_by_position_and_progress(pos, *block, piston),
            movement,
            delta_progress,
        );
        if moving.intersects(entity) {
            delta = delta.max(get_movement(moving, movement, entity));
            if delta >= delta_progress {
                break;
            }
        }
    }
    if delta <= 0.0 {
        return 0.0;
    }
    delta.min(delta_progress) + PUSH_OFFSET
}

/// The box `level.getEntities` searches for a push: the moved block's own box
/// widened by the slab it is about to sweep.
pub fn movement_search_area(block_box: Aabb, movement: Direction, delta_progress: f64) -> Aabb {
    movement_area(block_box, movement, delta_progress).union_bounds(block_box)
}

/// `moveStuckEntities`: the box of standing entities a moving honey block drags
/// with it. `sticky_top` is the moved state's top collision height.
pub fn sticky_area(pos: BlockPos, piston: MovingPistonState, sticky_top: f64) -> Aabb {
    let area = Aabb::new(
        Vec3::new(0.0, sticky_top, 0.0),
        Vec3::new(1.0, 1.500_001_000_000_000_1, 1.0),
    );
    move_by_position_and_progress(pos, area, piston)
}

/// `PistonMovingBlockEntity.matchesStickyCritera`.
pub fn matches_sticky_criteria(
    area: Aabb,
    reaction: PushReaction,
    on_ground: bool,
    supported_by_piston: bool,
    entity_position: Vec3,
) -> bool {
    if reaction != PushReaction::PushPull || !on_ground {
        return false;
    }
    if supported_by_piston {
        return true;
    }
    let (min, max) = (area.min_position(), area.max_position());
    entity_position.x >= min.x
        && entity_position.x <= max.x
        && entity_position.z >= min.z
        && entity_position.z <= max.z
}

/// The level query intersects the sticky box before applying the predicate.
pub fn sticky_entity_selected(
    area: Aabb,
    bounds: Aabb,
    reaction: PushReaction,
    on_ground: bool,
    supported_by_piston: bool,
    position: Vec3,
) -> bool {
    area.intersects(bounds)
        && matches_sticky_criteria(area, reaction, on_ground, supported_by_piston, position)
}

/// `fixEntityWithinPistonBase`: a retracting source piston drags an entity that
/// is still inside the piston's own block out along the opposite of `movement`.
/// The returned distance is measured along that opposite direction.
pub fn fix_within_piston_base(
    pos: BlockPos,
    entity: Aabb,
    movement: Direction,
    delta_progress: f64,
) -> Option<f64> {
    let block = Aabb::unit_cube(Vec3::from_block(pos));
    if !entity.intersects(block) {
        return None;
    }
    let opposite = movement.opposite();
    let delta = get_movement(block, opposite, entity) + PUSH_OFFSET;
    let intersected = entity.intersect(block);
    let delta_intersected = get_movement(block, opposite, intersected) + PUSH_OFFSET;
    // As deep inside as the block allows: it is stuck in the base itself, so it
    // leaves along with the retraction.
    if (delta - delta_intersected).abs() < PUSH_OFFSET {
        Some(delta.min(delta_progress) + PUSH_OFFSET)
    } else {
        None
    }
}

/// `PistonMovingBlockEntity.getCollisionRelatedBlockState` uses the moved state
/// unless a source piston is retracting, where it shows a piston head instead.
/// The state itself comes from [`PistonBlockBindings::collision_related_state`].
///
/// [`PistonBlockBindings::collision_related_state`]:
///     crate::piston_shape::PistonBlockBindings::collision_related_state
pub fn uses_moved_state(piston: MovingPistonState) -> bool {
    piston.extending || !piston.source
}

/// `moveCollidedEntities`' slime bounce: a slime block sets the velocity along
/// the movement axis to that direction's step, leaving the other axes.
pub fn bounce_velocity(movement: Direction, velocity: Vec3) -> Vec3 {
    let mut velocity = velocity;
    match movement {
        Direction::East => velocity.x = 1.0,
        Direction::West => velocity.x = -1.0,
        Direction::Up => velocity.y = 1.0,
        Direction::Down => velocity.y = -1.0,
        Direction::South => velocity.z = 1.0,
        Direction::North => velocity.z = -1.0,
    }
    velocity
}

/// `Entity.getPistonPushReaction() == PushReaction.IGNORE_ENTITY`.
///
/// `Marker`, `Interaction`, `Display`, `AreaEffectCloud` and
/// `OminousItemSpawner` return `IGNORE_ENTITY`, and an armor stand does when it
/// is a marker. Horizon gives those kinds no pushed body, so a caller that has
/// no box to push never reaches the maths above; this names the predicate for
/// the kinds that do have one.
pub fn ignores_push(marker: bool) -> bool {
    marker
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> Aabb {
        Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0))
    }

    fn moving(direction: Direction, extending: bool, progress: f32) -> MovingPistonState {
        MovingPistonState {
            moved_state: crate::registry::BlockStateId(0),
            direction,
            extending,
            source: false,
            progress,
            previous_progress: progress,
        }
    }

    /// `getMovementArea` puts the slab on the leading face and widens it by the
    /// travel in that direction only.
    #[test]
    fn movement_area_sits_on_the_leading_face() {
        let east = movement_area(unit(), Direction::East, 0.5);
        assert_eq!(east.min_position(), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(east.max_position(), Vec3::new(1.5, 1.0, 1.0));

        let west = movement_area(unit(), Direction::West, 0.5);
        assert_eq!(west.min_position(), Vec3::new(-0.5, 0.0, 0.0));
        assert_eq!(west.max_position(), Vec3::new(0.0, 1.0, 1.0));

        let up = movement_area(unit(), Direction::Up, 0.5);
        assert_eq!(up.min_position(), Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(up.max_position(), Vec3::new(1.0, 1.5, 1.0));

        let north = movement_area(unit(), Direction::North, 0.5);
        assert_eq!(north.min_position(), Vec3::new(0.0, 0.0, -0.5));
        assert_eq!(north.max_position(), Vec3::new(1.0, 1.0, 0.0));
    }

    /// A negative amount sweeps behind the leading face.
    #[test]
    fn movement_area_follows_a_negative_amount() {
        let area = movement_area(unit(), Direction::East, -0.5);
        assert_eq!(area.min_position(), Vec3::new(0.5, 0.0, 0.0));
        assert_eq!(area.max_position(), Vec3::new(1.0, 1.0, 1.0));
    }

    /// The arm of a piston at the origin: the block entity sits in the block it
    /// pushes into, and at progress 0 its shape is still a block back.
    const ARM: BlockPos = BlockPos::new(1, 0, 0);

    /// An entity in the way is pushed the progress delta, plus the push offset;
    /// here it hangs 0.6 into the slab and the delta caps it at 0.5.
    #[test]
    fn a_collided_entity_moves_the_progress_delta() {
        let piston = moving(Direction::East, true, 0.0);
        let entity = Aabb::new(Vec3::new(0.9, 0.0, 0.2), Vec3::new(1.5, 1.8, 0.8));
        let amount = collided_amount(&[unit()], ARM, piston, 0.5, Direction::East, entity);
        assert!((amount - (0.5 + PUSH_OFFSET)).abs() < 1.0e-12, "{amount}");
    }

    /// The delta is not a floor: an entity barely nicking the slab moves by only
    /// what it is owed, plus the offset.
    #[test]
    fn a_grazed_entity_moves_only_what_it_overlaps() {
        let piston = moving(Direction::East, true, 0.0);
        let entity = Aabb::new(Vec3::new(1.4, 0.0, 0.2), Vec3::new(2.0, 1.8, 0.8));
        let amount = collided_amount(&[unit()], ARM, piston, 0.5, Direction::East, entity);
        assert!((amount - (0.1 + PUSH_OFFSET)).abs() < 1.0e-12, "{amount}");
    }

    /// Clear of the block, nothing moves.
    #[test]
    fn an_entity_out_of_the_way_is_left_alone() {
        let piston = moving(Direction::East, true, 0.0);
        let entity = Aabb::new(Vec3::new(3.0, 0.0, 0.2), Vec3::new(3.6, 1.8, 0.8));
        let amount = collided_amount(&[unit()], ARM, piston, 0.5, Direction::East, entity);
        assert_eq!(amount, 0.0);
    }

    /// A retracting base drags an entity stuck inside its own block, limited by
    /// the progress delta.
    #[test]
    fn a_retracting_base_drags_a_stuck_entity() {
        let entity = Aabb::new(Vec3::new(0.75, 0.0, 0.2), Vec3::new(1.35, 1.8, 0.8));
        let amount = fix_within_piston_base(BlockPos::new(0, 0, 0), entity, Direction::West, 0.5)
            .expect("stuck in the base");
        assert!(amount > 0.0 && amount <= 0.5 + PUSH_OFFSET, "{amount}");
    }

    /// An entity outside the base's block is not dragged by it.
    #[test]
    fn a_free_entity_is_not_dragged_by_the_base() {
        let entity = Aabb::new(Vec3::new(2.0, 0.0, 0.2), Vec3::new(2.6, 1.8, 0.8));
        assert_eq!(
            fix_within_piston_base(BlockPos::new(0, 0, 0), entity, Direction::West, 0.5),
            None
        );
    }

    /// Honey drags only entities that stand on it and that blocks may push.
    #[test]
    fn honey_drags_standing_entities() {
        let area = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.5, 1.0));
        let on_top = Vec3::new(0.5, 0.0, 0.5);
        assert!(matches_sticky_criteria(
            area,
            PushReaction::PushPull,
            true,
            false,
            on_top
        ));
        assert!(!matches_sticky_criteria(
            area,
            PushReaction::PushPull,
            false,
            false,
            on_top
        ));
        assert!(!matches_sticky_criteria(
            area,
            PushReaction::Immoveable,
            true,
            false,
            on_top
        ));
        // Beside it: only standing on the piston's own block holds it.
        let beside = Vec3::new(2.0, 0.0, 0.5);
        assert!(!matches_sticky_criteria(
            area,
            PushReaction::PushPull,
            true,
            false,
            beside
        ));
        assert!(matches_sticky_criteria(
            area,
            PushReaction::PushPull,
            true,
            true,
            beside
        ));
    }

    #[test]
    fn honey_selection_uses_entity_position_and_intersection() {
        let area = Aabb::new(Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 1.5, 1.0));
        let position = Vec3::new(0.1, 1.0, 0.5);
        let bounds = Aabb::new(Vec3::new(-0.2, 1.0, 0.2), Vec3::new(0.4, 2.8, 0.8));
        assert!(sticky_entity_selected(
            area,
            bounds,
            PushReaction::PushPull,
            true,
            false,
            position
        ));
        let above = bounds.move_by(Vec3::new(0.0, 3.0, 0.0));
        assert!(!sticky_entity_selected(
            area,
            above,
            PushReaction::PushPull,
            true,
            true,
            position
        ));
    }

    /// The slime bounce rewrites only the movement axis.
    #[test]
    fn a_slime_block_bounces_along_its_axis() {
        let v = Vec3::new(0.2, -0.5, 0.3);
        assert_eq!(
            bounce_velocity(Direction::East, v),
            Vec3::new(1.0, -0.5, 0.3)
        );
        assert_eq!(bounce_velocity(Direction::Up, v), Vec3::new(0.2, 1.0, 0.3));
        assert_eq!(
            bounce_velocity(Direction::North, v),
            Vec3::new(0.2, -0.5, -1.0)
        );
    }

    /// An item sitting on a slime block that a piston is pushing up is inside
    /// the bounce search area and is pushed (`moveCollidedEntities`).
    #[test]
    fn an_item_on_slime_is_caught_by_an_upward_piston() {
        let piston = moving(Direction::Up, true, 0.0);
        let dest = BlockPos::new(0, 2, 0);
        let visual = move_by_position_and_progress(dest, unit(), piston);
        assert_eq!(visual.min_position(), Vec3::new(0.0, 1.0, 0.0));
        let search = movement_search_area(visual, Direction::Up, 0.5);
        let item = Aabb::new(Vec3::new(0.375, 2.0, 0.375), Vec3::new(0.625, 2.25, 0.625));
        assert!(
            search.intersects(item),
            "item sitting on slime must be in the bounce search"
        );
        let amount = collided_amount(&[unit()], dest, piston, 0.5, Direction::Up, item);
        assert!(
            amount > 0.0,
            "item sitting on slime must be pushed, got {amount}"
        );
        assert_eq!(bounce_velocity(Direction::Up, Vec3::ZERO).y, 1.0);
    }

    /// The moved block sits along the piston's own direction, half a block short
    /// at mid-travel and in place once landed.
    #[test]
    fn the_moved_block_sits_along_the_piston_direction() {
        let half = moving(Direction::East, true, 0.5);
        let moved = move_by_position_and_progress(BlockPos::new(2, 3, 4), unit(), half);
        assert_eq!(moved.min_position(), Vec3::new(1.5, 3.0, 4.0));

        let landed = moving(Direction::East, true, 1.0);
        assert_eq!(
            move_by_position_and_progress(BlockPos::new(2, 3, 4), unit(), landed).min_position(),
            Vec3::new(2.0, 3.0, 4.0)
        );

        // Retracting runs the other way: two thirds out at progress 1/3.
        let pulling = moving(Direction::East, false, 0.5);
        assert_eq!(
            move_by_position_and_progress(BlockPos::new(2, 3, 4), unit(), pulling).min_position(),
            Vec3::new(2.5, 3.0, 4.0)
        );
    }

    /// The search area covers the block and the slab in front of it.
    #[test]
    fn the_search_area_covers_block_and_sweep() {
        let area = movement_search_area(unit(), Direction::East, 0.5);
        assert_eq!(area.min_position(), Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(area.max_position(), Vec3::new(1.5, 1.0, 1.0));
    }
}
