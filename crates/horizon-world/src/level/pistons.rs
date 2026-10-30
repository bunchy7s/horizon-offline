//! Pistons: `PistonBaseBlock` (signal checks, block events, `moveBlocks`),
//! `PistonStructureResolver`, `PistonHeadBlock`, and the
//! `PistonMovingBlockEntity` progress ticking, plus the level's block-event
//! queue (`ServerLevel.blockEvent` / `runBlockEvents`).
//!
//! Entity pushing by moving blocks is the caller's (see `piston_movement`).
use super::{
    Behaviours, LevelAccess, LevelEvent, Sim, Ticks,
    data::PushReaction,
    states::BlockId,
    update::{above, rel},
};
use crate::{
    java_hash_map::{JavaHashMap, JavaKey},
    registry::BlockStateId,
};
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};
use std::cmp::Ordering;

/// `PistonMovingBlockEntity` state, owned by [`Ticks::moving_pistons`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovingPiston {
    pub moved_state: BlockStateId,
    pub direction: Direction,
    pub extending: bool,
    pub source: bool,
    pub progress: f32,
    pub progress_o: f32,
    pub last_ticked: i64,
    /// `PistonMovingBlockEntity.deathTicks`: on a client the entity waits
    /// five ticks past progress 1.0 before placing the block itself.
    pub death_ticks: i32,
    /// Identity of this block entity (a replaced entity is a new ticker).
    pub id: u64,
}
impl MovingPiston {
    /// What collision and rendering need from this block entity.
    pub fn shape_state(&self) -> crate::piston_shape::MovingPistonState {
        crate::piston_shape::MovingPistonState {
            moved_state: self.moved_state,
            direction: self.direction,
            extending: self.extending,
            source: self.source,
            progress: self.progress,
            previous_progress: self.progress_o,
        }
    }
    pub fn movement_direction(&self) -> Direction {
        if self.extending {
            self.direction
        } else {
            self.direction.opposite()
        }
    }
}

/// `BlockEventData`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockEventData {
    pub pos: BlockPos,
    pub block: BlockId,
    pub param_a: i32,
    pub param_b: i32,
}

#[derive(PartialEq, Eq)]
struct PosKey(BlockPos);
impl JavaKey for PosKey {
    fn java_hash(&self) -> i32 {
        (self.0.y.wrapping_add(self.0.z.wrapping_mul(31)))
            .wrapping_mul(31)
            .wrapping_add(self.0.x)
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        // Vec3i implements Comparable: y, then z, then x.
        Some(
            self.0
                .y
                .cmp(&other.0.y)
                .then(self.0.z.cmp(&other.0.z))
                .then(self.0.x.cmp(&other.0.x)),
        )
    }
    fn tie_break(&self, _other: &Self) -> Ordering {
        Ordering::Less
    }
}

struct Resolver {
    piston_pos: BlockPos,
    extending: bool,
    start: BlockPos,
    push_direction: Direction,
    piston_direction: Direction,
    to_push: Vec<BlockPos>,
    to_destroy: Vec<BlockPos>,
}

fn rel_n(pos: BlockPos, direction: Direction, n: i32) -> BlockPos {
    let [x, y, z] = direction.normal();
    pos.offset(x * n, y * n, z * n)
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    // ---- block events ----
    pub(crate) fn block_event(
        &mut self,
        pos: BlockPos,
        block: BlockId,
        param_a: i32,
        param_b: i32,
    ) {
        let event = BlockEventData {
            pos,
            block,
            param_a,
            param_b,
        };
        if !self.ticks.block_events.contains(&event) {
            self.ticks.block_events.push(event);
        }
    }
    /// `BlockState.triggerEvent`.
    fn trigger_event(&mut self, state: BlockStateId, pos: BlockPos, a: i32, b: i32) -> bool {
        let data = self.b.data(state);
        if data.is(super::classes::Class::PistonBase) {
            self.piston_trigger_event(pos, state, a, b)
        } else if data.is(super::classes::Class::Note) {
            self.note_trigger_event(pos, state);
            true
        } else if data.is(super::classes::Class::Bell) {
            // BellBlockEntity.triggerEvent handles event 1; the caller
            // reports handled events (`runBlockEvents` broadcasts them).
            a == 1
        } else if data.is(super::classes::Class::PotentSulfur) {
            // PotentSulfurBlock.triggerEvent: the block entity notes the
            // eruption's game time (the caller's), and the event is handled.
            true
        } else {
            false
        }
    }

    // ---- moving piston block entities ----
    pub(crate) fn moving_piston(&self, pos: BlockPos) -> Option<MovingPiston> {
        self.ticks
            .moving_pistons
            .iter()
            .find(|(p, _)| *p == pos)
            .map(|(_, m)| *m)
    }
    fn set_moving_piston(&mut self, pos: BlockPos, mut piston: MovingPiston) {
        self.ticks.moving_pistons.retain(|(p, _)| *p != pos);
        self.ticks.next_block_entity_id += 1;
        piston.id = self.ticks.next_block_entity_id;
        self.ticks.moving_pistons.push((pos, piston));
    }
    pub(crate) fn remove_moving_piston(&mut self, pos: BlockPos) -> Option<MovingPiston> {
        let index = self
            .ticks
            .moving_pistons
            .iter()
            .position(|(p, _)| *p == pos)?;
        Some(self.ticks.moving_pistons.remove(index).1)
    }
    /// `PistonMovingBlockEntity.finalTick`.
    fn moving_piston_final_tick(&mut self, pos: BlockPos) {
        let Some(piston) = self.moving_piston(pos) else {
            return;
        };
        // finalTick: `progressO < 1.0 || level.isClientSide` — a client
        // finishes its copy even at full progress.
        if piston.progress_o >= 1.0 && !self.level.is_client_side() {
            return;
        }
        self.remove_moving_piston(pos);
        let b = self.b;
        if b.is_block(self.state(pos), b.named.moving_piston) {
            let new = if piston.source {
                b.air()
            } else {
                self.update_from_neighbour_shapes(piston.moved_state, pos)
            };
            self.set_block_and_update(pos, new);
            let block = b.block(new);
            self.neighbor_changed_at(pos, block);
        }
    }
    /// `PistonMovingBlockEntity.tick`. A piston that is still moving reports
    /// the push `moveCollidedEntities` would do at its new progress; the caller
    /// moves the entities, because `Ticks` has none (`piston_pushes`).
    fn moving_piston_tick(&mut self, pos: BlockPos) {
        let Some(mut piston) = self.moving_piston(pos) else {
            return;
        };
        let b = self.b;
        piston.last_ticked = self.level.game_time();
        piston.progress_o = piston.progress;
        if piston.progress_o >= 1.0 {
            if self.level.is_client_side() && piston.death_ticks < 5 {
                // The client's copy waits for the server's state before
                // placing the block itself (deathTicks < 5).
                piston.death_ticks += 1;
                if let Some(entry) = self
                    .ticks
                    .moving_pistons
                    .iter_mut()
                    .find(|(p, _)| *p == pos)
                {
                    entry.1 = piston;
                }
                return;
            }
            self.remove_moving_piston(pos);
            if b.is_block(self.state(pos), b.named.moving_piston) {
                let mut new = self.update_from_neighbour_shapes(piston.moved_state, pos);
                if b.is_air(new) {
                    self.set_block(pos, piston.moved_state, 340);
                    self.update_or_destroy(
                        piston.moved_state,
                        new,
                        pos,
                        3,
                        super::flags::UPDATE_LIMIT,
                    );
                } else {
                    if b.is_waterlogged(new) {
                        new = b.table.set_bool(new, b.props.waterlogged, false);
                    }
                    self.set_block(pos, new, 67);
                    let block = b.block(new);
                    self.neighbor_changed_at(pos, block);
                }
            }
        } else {
            // moveCollidedEntities and moveStuckEntities run with the new
            // progress but read the old one off the entity (their delta),
            // so the push carries the piston as it was before the advance.
            let old = piston;
            let new_progress = (old.progress + 0.5).min(1.0);
            let current = self
                .ticks
                .moving_pistons
                .iter_mut()
                .find(|(p, _)| *p == pos);
            if let Some(entry) = current {
                piston.progress = new_progress;
                entry.1 = piston;
                self.ticks.piston_pushes.push((pos, old, new_progress));
            }
        }
    }

    // ---- PistonBaseBlock ----
    fn is_sticky_piston(&self, state: BlockStateId) -> bool {
        self.b.is_block(state, self.b.named.sticky_piston)
    }
    fn piston_neighbor_signal(&self, pos: BlockPos, push: Direction) -> bool {
        for direction in Direction::ALL {
            if direction != push && self.has_signal(rel(pos, direction), direction) {
                return true;
            }
        }
        if self.has_signal(pos, Direction::Down) {
            return true;
        }
        let up = above(pos);
        Direction::ALL.into_iter().any(|direction| {
            direction != Direction::Down && self.has_signal(rel(up, direction), direction)
        })
    }
    /// `PistonBaseBlock.checkIfExtend`, which `neighborChanged`, `onPlace`
    /// and `setPlacedBy` call only server-side: a client moves its pistons on
    /// the server's block events alone.
    pub(crate) fn piston_check_if_extend(&mut self, pos: BlockPos, state: BlockStateId) {
        if self.level.is_client_side() {
            return;
        }
        let b = self.b;
        let direction = b.facing(state);
        let extend = self.piston_neighbor_signal(pos, direction);
        let extended = b.table.bool(state, b.props.extended);
        let block = b.block(state);
        if extend && !extended {
            if self.piston_resolve(pos, direction, true).is_some() {
                self.block_event(pos, block, 0, direction as i32);
            }
        } else if !extend && extended {
            let pushed_pos = rel_n(pos, direction, 2);
            let pushed = self.state(pushed_pos);
            let mut event = 1;
            if b.is_block(pushed, b.named.moving_piston) && b.facing(pushed) == direction {
                if let Some(piston) = self.moving_piston(pushed_pos) {
                    if piston.extending
                        && (piston.progress_o < 0.5
                            || self.level.game_time() == piston.last_ticked
                            || self.ticks.handling_tick)
                    {
                        event = 2;
                    }
                }
            }
            self.block_event(pos, block, event, direction as i32);
        }
    }
    /// `PistonBaseBlock.isPushable`.
    fn is_pushable(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
        allow_destroyable: bool,
        connection: Direction,
    ) -> bool {
        let b = self.b;
        let (min_y, max_y) = (self.level.min_y(), self.level.max_y() - 1);
        if pos.y < min_y || pos.y > max_y {
            return false;
        }
        if b.is_air(state) {
            return true;
        }
        if direction == Direction::Down && pos.y == min_y {
            return false;
        }
        if direction == Direction::Up && pos.y == max_y {
            return false;
        }
        let is_piston =
            b.is_block(state, b.named.piston) || b.is_block(state, b.named.sticky_piston);
        if !is_piston {
            if b.facts(state).hardness == -1.0 {
                return false;
            }
            match b.facts(state).push {
                PushReaction::Immoveable => return false,
                PushReaction::Popped => return allow_destroyable,
                PushReaction::Push => return direction == connection,
                _ => {}
            }
        } else if b.table.bool(state, b.props.extended) {
            return false;
        }
        !b.has_flag(state, super::data::state_flags::BLOCK_ENTITY)
    }
    fn is_slime_or_honey(&self, state: BlockStateId) -> bool {
        self.b.is_block(state, self.b.named.slime_block)
            || self.b.is_block(state, self.b.named.honey_block)
    }
    fn can_stick(&self, first: BlockStateId, second: BlockStateId) -> bool {
        let n = self.b.named;
        let (honey1, slime1) = (
            self.b.is_block(first, n.honey_block),
            self.b.is_block(first, n.slime_block),
        );
        let (honey2, slime2) = (
            self.b.is_block(second, n.honey_block),
            self.b.is_block(second, n.slime_block),
        );
        if (honey1 && slime2) || (slime1 && honey2) {
            return false;
        }
        self.is_slime_or_honey(first) || self.is_slime_or_honey(second)
    }
    /// `PistonStructureResolver.resolve`; None when the structure cannot move.
    fn piston_resolve(
        &self,
        piston_pos: BlockPos,
        direction: Direction,
        extending: bool,
    ) -> Option<(Vec<BlockPos>, Vec<BlockPos>, Direction)> {
        let mut r = Resolver {
            piston_pos,
            extending,
            start: if extending {
                rel(piston_pos, direction)
            } else {
                rel_n(piston_pos, direction, 2)
            },
            push_direction: if extending {
                direction
            } else {
                direction.opposite()
            },
            piston_direction: direction,
            to_push: Vec::new(),
            to_destroy: Vec::new(),
        };
        let next = self.state(r.start);
        if !self.is_pushable(next, r.start, r.push_direction, false, r.piston_direction) {
            if r.extending && self.b.facts(next).push == PushReaction::Popped {
                r.to_destroy.push(r.start);
                return Some((r.to_push, r.to_destroy, r.push_direction));
            }
            return None;
        }
        let start = r.start;
        let push = r.push_direction;
        if !self.add_block_line(&mut r, start, push) {
            return None;
        }
        let mut i = 0;
        while i < r.to_push.len() {
            let pos = r.to_push[i];
            if self.is_slime_or_honey(self.state(pos)) && !self.add_branching_blocks(&mut r, pos) {
                return None;
            }
            i += 1;
        }
        Some((r.to_push, r.to_destroy, r.push_direction))
    }
    fn add_block_line(&self, r: &mut Resolver, start: BlockPos, direction: Direction) -> bool {
        let mut next = self.state(start);
        if self.b.is_air(next) {
            return true;
        }
        if !self.is_pushable(next, start, r.push_direction, false, direction)
            || start == r.piston_pos
            || r.to_push.contains(&start)
        {
            return true;
        }
        let mut count = 1;
        if count + r.to_push.len() > 12 {
            return false;
        }
        while self.is_slime_or_honey(next) {
            let pos = rel_n(start, r.push_direction.opposite(), count as i32);
            let previous = next;
            next = self.state(pos);
            if self.b.is_air(next)
                || !self.can_stick(previous, next)
                || !self.is_pushable(
                    next,
                    pos,
                    r.push_direction,
                    false,
                    r.push_direction.opposite(),
                )
                || pos == r.piston_pos
            {
                break;
            }
            count += 1;
            if count + r.to_push.len() > 12 {
                return false;
            }
        }
        let mut added = 0;
        for i in (0..count).rev() {
            r.to_push
                .push(rel_n(start, r.push_direction.opposite(), i as i32));
            added += 1;
        }
        let mut i = 1;
        loop {
            let pos = rel_n(start, r.push_direction, i);
            if let Some(collision) = r.to_push.iter().position(|p| *p == pos) {
                self.reorder_at_collision(r, added, collision);
                for j in 0..=collision + added {
                    let block_pos = r.to_push[j];
                    if self.is_slime_or_honey(self.state(block_pos))
                        && !self.add_branching_blocks(r, block_pos)
                    {
                        return false;
                    }
                }
                return true;
            }
            next = self.state(pos);
            if self.b.is_air(next) {
                return true;
            }
            if !self.is_pushable(next, pos, r.push_direction, true, r.push_direction)
                || pos == r.piston_pos
            {
                return false;
            }
            if self.b.facts(next).push == PushReaction::Popped {
                r.to_destroy.push(pos);
                return true;
            }
            if r.to_push.len() >= 12 {
                return false;
            }
            r.to_push.push(pos);
            added += 1;
            i += 1;
        }
    }
    fn reorder_at_collision(&self, r: &mut Resolver, added: usize, collision: usize) {
        let len = r.to_push.len();
        let head: Vec<BlockPos> = r.to_push[..collision].to_vec();
        let last_line: Vec<BlockPos> = r.to_push[len - added..].to_vec();
        let middle: Vec<BlockPos> = r.to_push[collision..len - added].to_vec();
        r.to_push.clear();
        r.to_push.extend(head);
        r.to_push.extend(last_line);
        r.to_push.extend(middle);
    }
    fn add_branching_blocks(&self, r: &mut Resolver, from: BlockPos) -> bool {
        let from_state = self.state(from);
        for direction in Direction::ALL {
            if super::query::axis_of(direction) != super::query::axis_of(r.push_direction) {
                let neighbour = rel(from, direction);
                let neighbour_state = self.state(neighbour);
                if self.can_stick(neighbour_state, from_state)
                    && !self.add_block_line(r, neighbour, direction)
                {
                    return false;
                }
            }
        }
        true
    }
    fn moving_piston_state(&self, direction: Direction, sticky: bool) -> BlockStateId {
        let b = self.b;
        let state = b.table.set_dir(
            b.default_of(b.named.moving_piston),
            b.props.facing,
            direction,
        );
        b.table.set_text(
            state,
            b.props.kind,
            if sticky { "sticky" } else { "normal" },
        )
    }
    /// `PistonBaseBlock.moveBlocks`.
    fn piston_move_blocks(
        &mut self,
        piston_pos: BlockPos,
        direction: Direction,
        extending: bool,
        sticky: bool,
    ) -> bool {
        let b = self.b;
        let arm = rel(piston_pos, direction);
        if !extending && b.is_block(self.state(arm), b.named.piston_head) {
            self.set_block(arm, b.air(), 276);
        }
        let Some((to_push, to_destroy, push_direction)) =
            self.piston_resolve(piston_pos, direction, extending)
        else {
            return false;
        };
        let mut delete_after: JavaHashMap<PosKey, BlockStateId> = JavaHashMap::default();
        let mut shapes = Vec::with_capacity(to_push.len());
        for pos in &to_push {
            let state = self.state(*pos);
            shapes.push(state);
            delete_after.put(PosKey(*pos), state);
        }
        let mut to_update = Vec::with_capacity(to_push.len() + to_destroy.len());
        let move_direction = if extending {
            direction
        } else {
            direction.opposite()
        };
        for pos in to_destroy.iter().rev() {
            let state = self.state(*pos);
            self.drop_resources(state, *pos);
            // The client's copy plays the destroy effect itself (the
            // server sends no 2001 for piston moves).
            let fire = b.is_block(state, b.named.fire) || b.is_block(state, b.named.soul_fire);
            if !fire && self.level.is_client_side() {
                self.level_event(2001, *pos, state.0 as i32);
            }
            self.set_block(*pos, b.air(), 18);
            self.game_event(*pos, "minecraft:block_destroy");
            to_update.push(state);
        }
        for (i, pos) in to_push.iter().enumerate().rev() {
            let state = self.state(*pos);
            let target = rel(*pos, move_direction);
            delete_after.remove(&PosKey(target));
            let moving = b.table.set_dir(
                b.default_of(b.named.moving_piston),
                b.props.facing,
                direction,
            );
            self.set_block(target, moving, 324);
            self.set_moving_piston(
                target,
                MovingPiston {
                    moved_state: shapes[i],
                    direction,
                    extending,
                    source: false,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                    death_ticks: 0,
                    id: 0,
                },
            );
            to_update.push(state);
        }
        if extending {
            let head = b.table.set_text(
                b.table
                    .set_dir(b.default_of(b.named.piston_head), b.props.facing, direction),
                b.props.kind,
                if sticky { "sticky" } else { "normal" },
            );
            let moving = self.moving_piston_state(direction, sticky);
            delete_after.remove(&PosKey(arm));
            self.set_block(arm, moving, 324);
            self.set_moving_piston(
                arm,
                MovingPiston {
                    moved_state: head,
                    direction,
                    extending: true,
                    source: true,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                    death_ticks: 0,
                    id: 0,
                },
            );
        }
        let air = b.air();
        let entries: Vec<(BlockPos, BlockStateId)> =
            delete_after.entries().map(|(k, v)| (k.0, *v)).collect();
        for (pos, _) in &entries {
            self.set_block(*pos, air, 82);
        }
        for (pos, old) in &entries {
            self.update_indirect_neighbour_shapes(*old, *pos, 2, super::flags::UPDATE_LIMIT);
            self.update_neighbour_shapes(air, *pos, 2, super::flags::UPDATE_LIMIT);
            self.update_indirect_neighbour_shapes(air, *pos, 2, super::flags::UPDATE_LIMIT);
        }
        let mut index = 0;
        for pos in to_destroy.iter().rev() {
            let state = to_update[index];
            index += 1;
            self.affect_neighbors_after_removal(state, *pos, false);
            self.update_indirect_neighbour_shapes(state, *pos, 2, super::flags::UPDATE_LIMIT);
            let block = b.block(state);
            self.update_neighbors_at(*pos, block);
        }
        for pos in to_push.iter().rev() {
            let block = b.block(to_update[index]);
            index += 1;
            self.update_neighbors_at(*pos, block);
        }
        if extending {
            self.update_neighbors_at(arm, b.named.piston_head);
        }
        let _ = push_direction;
        true
    }
    /// `PistonBaseBlock.triggerEvent`.
    fn piston_trigger_event(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        a: i32,
        param_b: i32,
    ) -> bool {
        let b = self.b;
        let p = b.props;
        let direction = b.facing(state);
        let extended_state = b.table.set_bool(state, p.extended, true);
        let sticky = self.is_sticky_piston(state);
        // The signal re-check is server-only (`!level.isClientSide`): a
        // client trusts the event as sent.
        if !self.level.is_client_side() {
            let extend = self.piston_neighbor_signal(pos, direction);
            if extend && (a == 1 || a == 2) {
                self.set_block(pos, extended_state, 2);
                return false;
            }
            if !extend && a == 0 {
                return false;
            }
        }
        if a == 0 {
            if !self.piston_move_blocks(pos, direction, true, sticky) {
                return false;
            }
            self.set_block(pos, extended_state, 67);
            let pitch = self.random().next_float() * 0.25 + 0.6;
            self.emit(LevelEvent::Sound {
                pos,
                sound: "minecraft:block.piston.extend".into(),
                volume: 0.5,
                pitch,
            });
            self.game_event(pos, "minecraft:block_activate");
        } else if a == 1 || a == 2 {
            self.moving_piston_final_tick(rel(pos, direction));
            let moving = self.moving_piston_state(direction, sticky);
            self.set_block(pos, moving, 276);
            let piston_default = b.table.set_dir(
                b.default_of(b.block(state)),
                p.facing,
                Direction::ALL[(param_b & 7).min(5) as usize],
            );
            self.set_moving_piston(
                pos,
                MovingPiston {
                    moved_state: piston_default,
                    direction,
                    extending: false,
                    source: true,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                    death_ticks: 0,
                    id: 0,
                },
            );
            self.update_neighbors_at(pos, b.named.moving_piston);
            self.update_neighbour_shapes(moving, pos, 2, super::flags::UPDATE_LIMIT);
            if sticky {
                let two = rel_n(pos, direction, 2);
                let moving_state = self.state(two);
                let mut piston_piece = false;
                if b.is_block(moving_state, b.named.moving_piston) {
                    if let Some(entity) = self.moving_piston(two) {
                        if entity.direction == direction && entity.extending {
                            self.moving_piston_final_tick(two);
                            piston_piece = true;
                        }
                    }
                }
                if !piston_piece {
                    let is_piston = b.is_block(moving_state, b.named.piston)
                        || b.is_block(moving_state, b.named.sticky_piston);
                    if a != 1
                        || b.is_air(moving_state)
                        || !self.is_pushable(
                            moving_state,
                            two,
                            direction.opposite(),
                            false,
                            direction,
                        )
                        || (b.facts(moving_state).push != PushReaction::PushPull && !is_piston)
                    {
                        self.remove_block(rel(pos, direction), false);
                    } else {
                        self.piston_move_blocks(pos, direction, false, sticky);
                    }
                }
            } else {
                self.remove_block(rel(pos, direction), false);
            }
            let pitch = self.random().next_float() * 0.15 + 0.6;
            self.emit(LevelEvent::Sound {
                pos,
                sound: "minecraft:block.piston.contract".into(),
                volume: 0.5,
                pitch,
            });
            self.game_event(pos, "minecraft:block_deactivate");
        }
        true
    }

    // ---- PistonHeadBlock ----
    pub(crate) fn piston_head_fitting_base(&self, head: BlockStateId, base: BlockStateId) -> bool {
        let b = self.b;
        let expected = if b.table.text(head, b.props.kind) == "sticky" {
            b.named.sticky_piston
        } else {
            b.named.piston
        };
        b.is_block(base, expected)
            && b.table.bool(base, b.props.extended)
            && b.facing(base) == b.facing(head)
    }
    pub(crate) fn piston_head_can_survive(&self, head: BlockStateId, pos: BlockPos) -> bool {
        let b = self.b;
        let base = self.rel_state(pos, b.facing(head).opposite());
        self.piston_head_fitting_base(head, base)
            || (b.is_block(base, b.named.moving_piston) && b.facing(base) == b.facing(head))
    }
}

impl Behaviours {
    /// `ServerLevel.runBlockEvents`: run queued block events (piston moves).
    /// Call once per tick after scheduled and random ticks.
    pub fn run_block_events<L: LevelAccess + ?Sized>(&self, level: &mut L, ticks: &mut Ticks) {
        let mut reschedule = Vec::new();
        while !ticks.block_events.is_empty() {
            let event = ticks.block_events.remove(0);
            if !level.is_ticking_chunk(event.pos.chunk()) {
                reschedule.push(event);
                continue;
            }
            let mut sim = self.sim(level, ticks);
            let state = sim.state(event.pos);
            if self.block(state) == event.block
                && sim.trigger_event(state, event.pos, event.param_a, event.param_b)
            {
                // ServerLevel.runBlockEvents broadcasts a handled event to
                // the clients (`ClientboundBlockEventPacket`); their copy of
                // the block re-runs `triggerEvent` (a piston's `moveBlocks`).
                sim.emit(LevelEvent::BlockEntityEvent {
                    pos: event.pos,
                    state,
                    a: event.param_a,
                    b: event.param_b,
                });
            }
        }
        for event in reschedule {
            if !ticks.block_events.contains(&event) {
                ticks.block_events.push(event);
            }
        }
    }
    /// `Level.tickBlockEntities` for moving pistons (the only block entities
    /// whose ticking changes blocks). Entries added during the pass wait for
    /// the next tick, as vanilla's pending tickers do.
    pub fn tick_block_entities<L: LevelAccess + ?Sized>(&self, level: &mut L, ticks: &mut Ticks) {
        let tickers: Vec<(BlockPos, u64)> = ticks
            .moving_pistons
            .iter()
            .map(|(p, m)| (*p, m.id))
            .collect();
        for (pos, id) in tickers {
            if !level.is_ticking_chunk(pos.chunk())
                || !ticks
                    .moving_pistons
                    .iter()
                    .any(|(p, m)| *p == pos && m.id == id)
            {
                continue;
            }
            self.sim(level, ticks).moving_piston_tick(pos);
        }
    }
    /// `ClientLevel.blockEvent`: the client re-runs the block's
    /// `triggerEvent` on its own level (a piston's `moveBlocks` builds the
    /// moving block entities it then animates) instead of waiting for the
    /// server's states. No queue and no ticking-chunk gate: a client runs
    /// the event where the packet lands.
    pub fn client_block_event<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        a: i32,
        b: i32,
    ) -> bool {
        let triggered = {
            let mut sim = self.sim(level, ticks);
            let state = sim.state(pos);
            let triggered = sim.trigger_event(state, pos, a, b);
            if triggered {
                // The client's copy of a handled event reaches the client
                // copy of the block entity (a bell's ring).
                sim.emit(LevelEvent::BlockEntityEvent { pos, state, a, b });
            }
            triggered
        };
        self.client_drain_block_events(level, ticks);
        triggered
    }
    /// `ClientLevel.tickBlockEntities` for moving pistons: the client copy of
    /// [`Behaviours::tick_block_entities`], without the server's
    /// ticking-chunk gate (a client ticks every loaded block entity). One
    /// whose block was replaced is dropped (`TickingBlockEntity.isRemoved`).
    pub fn client_tick_block_entities<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
    ) {
        let tickers: Vec<(BlockPos, u64)> = ticks
            .moving_pistons
            .iter()
            .map(|(p, m)| (*p, m.id))
            .collect();
        for (pos, id) in tickers {
            if !ticks
                .moving_pistons
                .iter()
                .any(|(p, m)| *p == pos && m.id == id)
            {
                continue;
            }
            {
                let mut sim = self.sim(level, ticks);
                let state = sim.state(pos);
                if sim.b.is_block(state, sim.b.named.moving_piston) {
                    sim.moving_piston_tick(pos);
                } else {
                    sim.remove_moving_piston(pos);
                }
            }
            self.client_drain_block_events(level, ticks);
        }
    }
    /// Block events a client-side `triggerEvent` queued
    /// (`PistonBaseBlock.checkIfExtend` in a neighbour update): a client
    /// runs them as they come (`ClientLevel.blockEvent`), not in a tick pass.
    fn client_drain_block_events<L: LevelAccess + ?Sized>(&self, level: &mut L, ticks: &mut Ticks) {
        while let Some(event) = ticks.block_events.first().copied() {
            ticks.block_events.remove(0);
            let mut sim = self.sim(level, ticks);
            let state = sim.state(event.pos);
            if self.block(state) == event.block
                && sim.trigger_event(state, event.pos, event.param_a, event.param_b)
            {
                sim.emit(LevelEvent::BlockEntityEvent {
                    pos: event.pos,
                    state,
                    a: event.param_a,
                    b: event.param_b,
                });
            }
        }
    }
}
