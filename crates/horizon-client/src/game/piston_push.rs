//! `PistonMovingBlockEntity.moveCollidedEntities` and `moveStuckEntities` for
//! the entities the game moves itself: the players and the mobs. The shulker
//! box lid's push (`ShulkerBoxBlockEntity.moveCollidedEntities`) rides along,
//! with `Entity.move(MoverType.SHULKER_BOX, …)` taking the same path.
//!
//! The geometry is in [`horizon_world::piston_push`]; this file supplies the
//! bodies and applies the movement the way `Entity.move(MoverType.PISTON, …)`
//! does: limited to 0.51 blocks per tick per axis (`Entity.limitPistonMovement`),
//! then run through the block collision scan, so a pushed entity stops at a
//! wall instead of passing through it.
//!
use super::{
    Game,
    entity::{Entity, EntityHandle, with_entity, with_entity_mut},
};
use horizon_client::collisions::{EntityCollisions, SceneCollisions, with_piston_noclip};
use horizon_core::geometry::{Aabb, Axis, Direction, ShapeOp, Vec3, border::BorderGeometry};
use horizon_core::position::BlockPos;
use horizon_world::{
    block_collision_scan::BlockCollisionWorld, level::data::PushReaction::IgnoreEntity,
    level::pistons::MovingPiston, piston_movement::PistonMovement, piston_push,
    world_movement::EntityMovementWorld,
};

/// What one piston asks for this tick: where it is, its state, and the progress
/// `moveCollidedEntities` runs at (before the tick stores it).
pub(super) type Push = (BlockPos, MovingPiston, f32);

/// What one shulker box asks for this tick (`moveCollidedEntities`): where it
/// is, its facing as a 3D data value, and the lid box it swept
/// (`Shulker.getProgressDeltaAabb`).
pub(super) type ShulkerPush = ([i32; 3], u8, [f64; 6]);

/// Which entity was pushed, so its storage can be reached after the world
/// borrow ends.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum Target {
    Player,
    Guest(u32),
    Entity(EntityHandle),
}

/// `Entity.getStepHeight` of each body, read before the push.
struct StepHeights {
    player: f32,
    guests: std::collections::HashMap<u32, f32>,
}

/// What the push needs from one entity.
#[derive(Clone, Copy)]
struct Body {
    bounds: Aabb,
    position: Vec3,
    on_ground: bool,
    step_height: f32,
    /// Standing on the piston's own block (`Entity.isSupportedBy`).
    supported: bool,
}

impl Game<'_> {
    /// Push whatever this tick's moving pistons ran into, in the order the
    /// pistons ticked: vanilla pushes from each block entity's own tick.
    pub(super) fn push_piston_entities(&mut self, pushes: Vec<Push>) {
        let mut accumulators = std::collections::HashMap::new();
        for push in pushes {
            self.push_for_piston(push, &mut accumulators);
        }
    }

    /// `ShulkerBoxBlockEntity.moveCollidedEntities` for the bodies this game
    /// moves itself: every entity the sweeping lid box meets moves out along
    /// the box's facing by the box's own size plus 0.01
    /// (`Entity.move(MoverType.SHULKER_BOX, …)`).
    pub(super) fn push_shulker_entities(&mut self, pushes: Vec<ShulkerPush>) {
        let steps = self.push_step_heights();
        for (pos, facing, lid) in pushes {
            self.push_for_shulker(pos, facing, lid, &steps);
        }
    }

    /// One box's push, a body at a time. The move resolves as a piston's does
    /// (`Entity.move` stops at the blocks and the world border), without a
    /// piston's per-tick travel limit or its noclip.
    fn push_for_shulker(&mut self, pos: [i32; 3], facing: u8, lid: [f64; 6], steps: &StepHeights) {
        let direction = Direction::ALL[usize::from(facing) % Direction::ALL.len()];
        let [x, y, z] = direction.normal();
        let movement = Vec3::new(
            (lid[3] - lid[0] + 0.01) * f64::from(x),
            (lid[4] - lid[1] + 0.01) * f64::from(y),
            (lid[5] - lid[2] + 0.01) * f64::from(z),
        );
        let area = Aabb::new(
            Vec3::new(lid[0], lid[1], lid[2]),
            Vec3::new(lid[3], lid[4], lid[5]),
        );
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.push_targets().for_each(|target| {
            let Some(body) = self.push_body(target, at, steps) else {
                return;
            };
            if !body.bounds.intersects(area) {
                return;
            }
            let Ok(moved) = pushed_movement(
                SceneCollisions {
                    world: &self.world,
                    binding: self.collision,
                    subject: None,
                },
                self.border.geometry(),
                [body.position.x, body.position.z],
                body.bounds,
                movement,
                body.step_height,
                body.on_ground,
            ) else {
                return;
            };
            if moved.length_squared() > 0.0 {
                self.apply_push(target, moved);
            }
        });
    }

    fn push_for_piston(
        &mut self,
        (pos, piston, new_progress): Push,
        accumulators: &mut std::collections::HashMap<Target, PistonMovement>,
    ) {
        let delta_progress = f64::from(new_progress - piston.progress);
        if delta_progress <= 0.0 {
            return;
        }
        let Some(bindings) = self.collision.pistons() else {
            return;
        };
        let Ok(state) = bindings.collision_related_state(piston.shape_state()) else {
            return;
        };
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let Ok(shape) = self
            .collision
            .collision_shape(&mut scene, state, [pos.x, pos.y, pos.z])
        else {
            return;
        };
        if shape.is_empty() {
            return;
        }
        let movement = piston.movement_direction();
        let boxes: Vec<Aabb> = shape.boxes().to_vec();
        let facts = self.res.facts();
        let slime = &*facts.get(piston.moved_state).name == "minecraft:slime_block";
        let honey = &*facts.get(piston.moved_state).name == "minecraft:honey_block";
        // `moveStuckEntities` asks the moved state's own top for its box.
        let sticky_top = if honey {
            let mut scene = SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            };
            self.collision
                .collision_shape(&mut scene, piston.moved_state, [pos.x, pos.y, pos.z])
                .ok()
                .and_then(|s| s.bounds())
                .map_or(0.0, |b| b.max_position().y)
        } else {
            0.0
        };

        let steps = self.push_step_heights();
        let search = shape.bounds().map(|bounds| {
            piston_push::movement_search_area(
                piston_push::move_by_position_and_progress(pos, bounds, piston.shape_state()),
                movement,
                delta_progress,
            )
        });
        // `for_each`: a chain folds list by list, where `next` would re-check every
        // link for each entity.
        self.push_targets().for_each(|target| {
            let Some(body) = self.push_body(target, pos, &steps) else {
                return;
            };
            let amount = piston_push::collided_amount(
                &boxes,
                pos,
                piston.shape_state(),
                new_progress,
                movement,
                body.bounds,
            );
            if slime && search.is_some_and(|area| area.intersects(body.bounds)) {
                self.bounce(target, movement);
            }
            if amount > 0.0 {
                let step = piston_push::step_vector(movement);
                let before = body.bounds;
                self.move_pushed(
                    target,
                    before,
                    [body.position.x, body.position.z],
                    body.step_height,
                    body.on_ground,
                    step.scale(amount),
                    movement,
                    accumulators,
                );
                // `fixEntityWithinPistonBase`: a retracting source piston drags
                // an entity that is still inside its own block.
                if !piston.extending
                    && piston.source
                    && let Some(after) = self.push_body(target, pos, &steps)
                    && let Some(fix) = piston_push::fix_within_piston_base(
                        pos,
                        after.bounds,
                        movement,
                        delta_progress,
                    )
                {
                    let back = piston_push::step_vector(movement.opposite());
                    self.move_pushed(
                        target,
                        after.bounds,
                        [after.position.x, after.position.z],
                        after.step_height,
                        after.on_ground,
                        back.scale(fix),
                        movement,
                        accumulators,
                    );
                }
            }
        });
        // Vanilla selects honey passengers after all collided entities have moved.
        if honey && movement.normal()[1] == 0 {
            let area = piston_push::sticky_area(pos, piston.shape_state(), sticky_top);
            for target in self.push_targets() {
                let Some(body) = self.push_body(target, pos, &steps) else {
                    continue;
                };
                if piston_push::sticky_entity_selected(
                    area,
                    body.bounds,
                    horizon_world::level::data::PushReaction::PushPull,
                    body.on_ground,
                    body.supported,
                    body.position,
                ) {
                    self.move_pushed(
                        target,
                        body.bounds,
                        [body.position.x, body.position.z],
                        body.step_height,
                        body.on_ground,
                        piston_push::step_vector(movement).scale(delta_progress),
                        movement,
                        accumulators,
                    );
                }
            }
        }
    }

    /// `Block.pushEntitiesUp(old, new, level, pos)` for the block now at `pos`
    /// (`FarmlandBlock.turnToBaseBlock`, `PathBlock.turnToBaseBlock`): every body
    /// touching the collision that the new state adds over `old` is lifted
    /// onto it with `teleportRelative`, no collision scan.
    pub(super) fn push_entities_up(
        &mut self,
        pos: BlockPos,
        old: horizon_world::registry::BlockStateId,
    ) {
        let new = self.world.block(pos.x, pos.y, pos.z);
        if new == old {
            return;
        }
        let at = [pos.x, pos.y, pos.z];
        let shape = |state| {
            let mut scene = SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            };
            self.collision.collision_shape(&mut scene, state, at).ok()
        };
        let (Some(old_shape), Some(new_shape)) = (shape(old), shape(new)) else {
            return;
        };
        let Ok(added) = old_shape.join_unoptimized(&new_shape, ShapeOp::ONLY_SECOND) else {
            return;
        };
        let added = added.moved(Vec3::new(
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(pos.z),
        ));
        let Some(bounds) = added.bounds() else {
            return;
        };
        let steps = self.push_step_heights();
        for target in self.push_targets() {
            let Some(body) = self.push_body(target, pos, &steps) else {
                continue;
            };
            if !body.bounds.intersects(bounds) {
                continue;
            }
            let dy = added.collide(Axis::Y, body.bounds.move_by(Vec3::new(0.0, 1.0, 0.0)), -1.0);
            self.apply_push(target, Vec3::new(0.0, 1.0 + dy, 0.0));
        }
    }

    /// `Entity.getStepHeight` for each body, read once: the attribute lookup
    /// needs the mutable body, and the push only needs the number.
    fn push_step_heights(&mut self) -> StepHeights {
        let player = self
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:step_height").ok())
            .map_or(0.6, |v| v as f32);
        let mut guests = std::collections::HashMap::new();
        if let Some(host) = self.lan.as_mut() {
            for g in &mut host.hub.guests {
                let step = g
                    .motion
                    .as_mut()
                    .and_then(|m| m.attribute("minecraft:step_height").ok())
                    .map_or(0.6, |v| v as f32);
                guests.insert(g.conn, step);
            }
        }
        StepHeights { player, guests }
    }

    /// The entities a moving block can run into: the window's player, the LAN
    /// guests in the level, then the mobs.
    fn push_targets(&self) -> impl Iterator<Item = Target> + use<> {
        let player = self.player.is_some().then_some(Target::Player);
        // Empty, and so unallocated, unless guests are here.
        let guests: Vec<Target> = self.lan.as_ref().map_or_else(Vec::new, |host| {
            host.hub
                .guests
                .iter()
                .filter(|g| host.hub.here(g) && g.loaded && !g.dead)
                .map(|g| Target::Guest(g.conn))
                .collect()
        });
        player
            .into_iter()
            .chain(guests)
            .chain(self.entity_handles().map(Target::Entity))
    }

    /// What the push needs from one entity, or `None` when it is not pushed:
    /// removed, dead, a spectator, or an armor-stand marker.
    fn push_body(&self, target: Target, pos: BlockPos, steps: &StepHeights) -> Option<Body> {
        match target {
            Target::Player => {
                let p = self.player.as_ref()?;
                if p.spectator {
                    return None;
                }
                Some(Body {
                    bounds: p.bounds(),
                    position: p.position,
                    on_ground: p.ground.on_ground,
                    step_height: steps.player,
                    supported: p.ground.supporting_block == Some(pos),
                })
            }
            Target::Guest(conn) => {
                let host = self.lan.as_ref()?;
                let g = host.hub.guests.iter().find(|g| g.conn == conn)?;
                if g.dead || g.game_mode == 3 {
                    return None;
                }
                let p = g.position;
                Some(Body {
                    bounds: Aabb::new(
                        Vec3::new(p[0] - 0.3, p[1], p[2] - 0.3),
                        Vec3::new(p[0] + 0.3, p[1] + 1.8, p[2] + 0.3),
                    ),
                    position: Vec3::new(p[0], p[1], p[2]),
                    on_ground: g.on_ground,
                    step_height: steps.guests.get(&conn).copied().unwrap_or(0.6),
                    supported: g
                        .motion
                        .as_ref()
                        .is_some_and(|m| m.ground.supporting_block == Some(pos)),
                })
            }
            Target::Entity(handle) => with_entity!(self, handle, entity => {
                if !entity.is_alive() || entity.get_piston_push_reaction() == IgnoreEntity {
                    return None;
                }
                Some(Body {
                    bounds: Entity::bounds(entity),
                    position: entity.position(),
                    on_ground: entity.on_ground(),
                    step_height: entity.get_step_height(),
                    supported: entity.is_supported_by(pos),
                })
            }),
        }
    }

    /// `Entity.move(MoverType.PISTON, movement)` for one pushed entity: the
    /// per-tick piston limit, then the block and world border collision scan.
    fn move_pushed(
        &mut self,
        target: Target,
        before: Aabb,
        at: [f64; 2],
        step_height: f32,
        on_ground: bool,
        movement: Vec3,
        piston_direction: Direction,
        accumulators: &mut std::collections::HashMap<Target, PistonMovement>,
    ) {
        let Some(limited) = self.limit_piston(target, movement, accumulators) else {
            return;
        };
        // `limitPistonMovement` returns zero once the tick's budget is spent.
        if limited == Vec3::ZERO {
            return;
        }
        let resolved = with_piston_noclip(piston_direction, || {
            pushed_movement(
                SceneCollisions {
                    world: &self.world,
                    binding: self.collision,
                    subject: None,
                },
                self.border.geometry(),
                at,
                before,
                limited,
                step_height,
                on_ground,
            )
            .unwrap_or(Vec3::ZERO)
        });
        // Entity.move updates an authoritative living entity's ground flag
        // even for a horizontal piston push. Keeping the old support flag
        // would run Frost Walker while the stand is moving over open water.
        if let Target::Entity(EntityHandle::ArmorStand(i)) = target {
            self.armor_stands[i].on_ground = limited.y < 0.0 && limited.y != resolved.y;
        }
        if resolved.length_squared() <= 1.0e-7
            && limited.length_squared() - resolved.length_squared() >= 1.0e-7
        {
            return;
        }
        self.apply_push(target, resolved);
    }

    /// `Entity.limitPistonMovement`: each axis may travel at most 0.51 blocks
    /// per game tick, and a move touches only its first non-zero axis.
    fn limit_piston(
        &mut self,
        target: Target,
        movement: Vec3,
        accumulators: &mut std::collections::HashMap<Target, PistonMovement>,
    ) -> Option<Vec3> {
        let time = self.world.game_time;
        let limit = |state: &mut PistonMovement| state.limit(movement, || Ok::<i64, ()>(time)).ok();
        match target {
            Target::Player => {
                let p = self.player.as_mut()?;
                limit(&mut p.piston)
            }
            Target::Guest(conn) => {
                let host = self.lan.as_mut()?;
                let g = host.hub.guests.iter_mut().find(|g| g.conn == conn)?;
                limit(&mut g.motion.as_deref_mut()?.piston)
            }
            Target::Entity(EntityHandle::Mob(i)) => limit(&mut self.mobs[i].piston),
            _ => limit(accumulators.entry(target).or_default()),
        }
    }

    fn apply_push(&mut self, target: Target, movement: Vec3) {
        match target {
            Target::Player => {
                if let Some(p) = self.player.as_mut() {
                    p.position = p.position.add_vector(movement);
                }
            }
            Target::Guest(conn) => {
                if let Some(host) = self.lan.as_mut()
                    && let Some(g) = host.hub.guests.iter_mut().find(|g| g.conn == conn)
                {
                    // The body and the hub's copy of its position move together:
                    // `leave_guest` writes the body back over the hub's.
                    for (slot, delta) in g.position.iter_mut().zip(movement.components()) {
                        *slot += delta;
                    }
                    if let Some(m) = g.motion.as_deref_mut() {
                        m.position = m.position.add_vector(movement);
                    }
                }
            }
            Target::Entity(handle) => {
                with_entity_mut!(self, handle, entity => entity.move_by_piston(movement))
            }
        }
    }

    /// `causeBounce`: a slime block sets the velocity along the movement axis.
    fn bounce(&mut self, target: Target, movement: Direction) {
        match target {
            Target::Player => {
                if let Some(p) = self.player.as_mut() {
                    p.velocity = piston_push::bounce_velocity(movement, p.velocity);
                }
            }
            Target::Guest(conn) => {
                if let Some(host) = self.lan.as_mut()
                    && let Some(guest) = host.hub.guests.iter_mut().find(|g| g.conn == conn)
                    && let Some(motion) = guest.motion.as_deref_mut()
                {
                    motion.velocity = piston_push::bounce_velocity(movement, motion.velocity);
                }
            }
            Target::Entity(handle) => {
                with_entity_mut!(self, handle, entity => Entity::piston_slime_bounce(entity, movement))
            }
        }
    }
}

/// `Entity.move(MoverType.PISTON, movement)` for a pushed body: the movement
/// it makes. The world's blocks stop it, and so does the world border:
/// `Entity.collectCollidersIgnoringWorldBorder` adds the border's walls when
/// the body is inside close to the border, as ordinary movement does.
fn pushed_movement<W>(
    blocks: W,
    border: &BorderGeometry,
    at: [f64; 2],
    before: Aabb,
    movement: Vec3,
    step_height: f32,
    on_ground: bool,
) -> horizon_client::Result<Vec3>
where
    W: BlockCollisionWorld<Error = std::io::Error> + EntityMovementWorld<Error = std::io::Error>,
{
    EntityCollisions::with_border(blocks, border, at).resolve(
        before,
        movement,
        step_height,
        on_ground,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::collisions::EmptyCollisions;

    /// A border 10 wide around the origin: its wall's inner face is at x = 5.
    fn border() -> BorderGeometry {
        BorderGeometry::new([0.0, 0.0], 10.0, 29_999_984).expect("a border")
    }

    /// `Entity.collectCollidersIgnoringWorldBorder`: the border's walls stop a
    /// pushed body that is inside close to the border.
    #[test]
    fn a_pushed_body_stops_at_the_world_border() {
        // A body 0.6 wide, 0.4 blocks short of the wall, pushed east into it.
        let before = Aabb::new(Vec3::new(4.0, 0.0, 0.0), Vec3::new(4.6, 1.8, 0.6));
        let border = border();
        let moved = pushed_movement(
            EmptyCollisions,
            &border,
            [4.3, 0.3],
            before,
            Vec3::new(0.51, 0.0, 0.0),
            0.0,
            false,
        )
        .expect("the push resolves");
        // The wall stops the body 0.4 blocks short of the whole push.
        assert!(
            (moved.x - 0.4).abs() < 1.0e-9,
            "the body was not stopped by the border: {moved:?}"
        );
    }

    /// Away from the border, the same push moves the whole way.
    #[test]
    fn a_pushed_body_moves_whole_away_from_the_border() {
        let before = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.6, 1.8, 0.6));
        let border = border();
        let moved = pushed_movement(
            EmptyCollisions,
            &border,
            [0.3, 0.3],
            before,
            Vec3::new(0.51, 0.0, 0.0),
            0.0,
            false,
        )
        .expect("the push resolves");
        assert!(
            (moved.x - 0.51).abs() < 1.0e-9,
            "the body did not move the whole push: {moved:?}"
        );
    }
}
