//! `Level.setBlock`, `LevelChunk.setBlockState` and the
//! `CollectingNeighborUpdater` that orders neighbour and shape updates.
use super::{LevelAccess, LevelEvent, Sim, classes::Class, flags, states::BlockId};
use crate::{fluid::FluidState, registry::BlockStateId};
use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
    ticks::{ScheduleResult, TickPriority},
};

/// `NeighborUpdater.UPDATE_ORDER`.
pub(crate) const UPDATE_ORDER: [Direction; 6] = [
    Direction::West,
    Direction::East,
    Direction::Down,
    Direction::Up,
    Direction::North,
    Direction::South,
];
/// `BlockBehaviour.UPDATE_SHAPE_ORDER`.
pub(crate) const UPDATE_SHAPE_ORDER: [Direction; 6] = [
    Direction::West,
    Direction::East,
    Direction::North,
    Direction::South,
    Direction::Down,
    Direction::Up,
];
pub(crate) const HORIZONTAL: [Direction; 4] = [
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::West,
];

/// `MinecraftServer.getMaxChainedNeighborUpdates` default.
const MAX_CHAINED_NEIGHBOR_UPDATES: i32 = 1_000_000;

pub(crate) fn rel(pos: BlockPos, direction: Direction) -> BlockPos {
    let [x, y, z] = direction.normal();
    pos.offset(x, y, z)
}
pub(crate) fn above(pos: BlockPos) -> BlockPos {
    pos.offset(0, 1, 0)
}
pub(crate) fn below(pos: BlockPos) -> BlockPos {
    pos.offset(0, -1, 0)
}

#[derive(Clone, Copy, Debug)]
enum Update {
    Shape {
        direction: Direction,
        neighbor_state: BlockStateId,
        pos: BlockPos,
        neighbor_pos: BlockPos,
        flags: i32,
        limit: i32,
    },
    Simple {
        pos: BlockPos,
        block: BlockId,
    },
    Full {
        state: BlockStateId,
        pos: BlockPos,
        block: BlockId,
        moved: bool,
    },
    Multi {
        source: BlockPos,
        block: BlockId,
        skip: Option<Direction>,
        index: usize,
    },
}

enum Step {
    Shape {
        direction: Direction,
        neighbor_state: BlockStateId,
        pos: BlockPos,
        neighbor_pos: BlockPos,
        flags: i32,
        limit: i32,
    },
    Neighbor {
        state: Option<BlockStateId>,
        pos: BlockPos,
        block: BlockId,
        moved: bool,
    },
}

#[derive(Default)]
pub(crate) struct NeighborUpdater {
    stack: Vec<Update>,
    added: Vec<Update>,
    count: i32,
}

impl Update {
    /// `runNext`: the update to execute now and whether more remain.
    fn next(&mut self) -> (Step, bool) {
        match *self {
            Self::Shape {
                direction,
                neighbor_state,
                pos,
                neighbor_pos,
                flags,
                limit,
            } => (
                Step::Shape {
                    direction,
                    neighbor_state,
                    pos,
                    neighbor_pos,
                    flags,
                    limit,
                },
                false,
            ),
            Self::Simple { pos, block } => (
                Step::Neighbor {
                    state: None,
                    pos,
                    block,
                    moved: false,
                },
                false,
            ),
            Self::Full {
                state,
                pos,
                block,
                moved,
            } => (
                Step::Neighbor {
                    state: Some(state),
                    pos,
                    block,
                    moved,
                },
                false,
            ),
            Self::Multi {
                source,
                block,
                skip,
                ref mut index,
            } => {
                let direction = UPDATE_ORDER[*index];
                *index += 1;
                if *index < UPDATE_ORDER.len() && Some(UPDATE_ORDER[*index]) == skip {
                    *index += 1;
                }
                (
                    Step::Neighbor {
                        state: None,
                        pos: rel(source, direction),
                        block,
                        moved: false,
                    },
                    *index < UPDATE_ORDER.len(),
                )
            }
        }
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    pub fn state(&self, pos: BlockPos) -> BlockStateId {
        self.level.block_state(pos)
    }
    pub fn fluid_at(&self, pos: BlockPos) -> FluidState {
        self.b.fluid(self.state(pos))
    }
    pub fn random(&mut self) -> &mut LegacyRandom {
        self.level.random()
    }
    pub fn emit(&mut self, event: LevelEvent) {
        self.level.emit(event);
    }
    /// `TntBlock.prime` without a source: `new PrimedTnt` draws its push angle
    /// from the level random, then the primed sound and `prime_fuse` event.
    pub fn prime_tnt(&mut self, pos: BlockPos) {
        let rot = self.random().next_double() * f64::from(std::f32::consts::TAU);
        self.emit(LevelEvent::PrimeTnt { pos, rot });
        self.sound(
            pos,
            &std::sync::Arc::from("minecraft:entity.tnt.primed"),
            1.0,
            1.0,
        );
        self.game_event(pos, "minecraft:prime_fuse");
    }
    pub fn level_event(&mut self, id: i32, pos: BlockPos, data: i32) {
        self.level.emit(LevelEvent::LevelEvent { id, pos, data });
    }
    pub fn sound(&mut self, pos: BlockPos, sound: &std::sync::Arc<str>, volume: f32, pitch: f32) {
        self.level.emit(LevelEvent::Sound {
            pos,
            sound: sound.clone(),
            volume,
            pitch,
        });
    }
    pub fn game_event(&mut self, pos: BlockPos, event: &'static str) {
        self.level.emit(LevelEvent::GameEvent {
            pos,
            event,
            state: None,
        });
    }
    /// `Level.gameEvent(event, pos, Context.of(state))`.
    pub fn game_event_state(&mut self, pos: BlockPos, event: &'static str, state: BlockStateId) {
        self.level.emit(LevelEvent::GameEvent {
            pos,
            event,
            state: Some(state),
        });
    }
    pub fn block_of(&self, state: BlockStateId) -> BlockId {
        self.b.table.block(state)
    }
    pub fn is_in_valid_bounds(&self, pos: BlockPos) -> bool {
        pos.y >= self.level.min_y()
            && pos.y < self.level.max_y()
            && (-30_000_000..30_000_000).contains(&pos.x)
            && (-30_000_000..30_000_000).contains(&pos.z)
    }

    // ---- scheduled ticks ----
    pub fn schedule_tick(&mut self, pos: BlockPos, block: BlockId, delay: i32) -> ScheduleResult {
        self.schedule_tick_priority(pos, block, delay, TickPriority::Normal)
    }
    pub fn schedule_tick_priority(
        &mut self,
        pos: BlockPos,
        block: BlockId,
        delay: i32,
        priority: TickPriority,
    ) -> ScheduleResult {
        let time = self.level.game_time();
        self.ticks.schedule_block(time, pos, block, delay, priority)
    }
    pub fn schedule_fluid_tick(
        &mut self,
        pos: BlockPos,
        fluid: crate::fluid::FluidKind,
        delay: i32,
    ) -> ScheduleResult {
        let time = self.level.game_time();
        self.ticks.schedule_fluid(time, pos, fluid, delay)
    }

    // ---- Level.setBlock ----
    pub fn set_block(&mut self, pos: BlockPos, state: BlockStateId, flags: i32) -> bool {
        self.set_block_limit(pos, state, flags, flags::UPDATE_LIMIT)
    }
    pub fn set_block_and_update(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        self.set_block(pos, state, flags::UPDATE_ALL)
    }
    pub fn set_block_limit(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        update_flags: i32,
        limit: i32,
    ) -> bool {
        if !self.is_in_valid_bounds(pos) || !self.level.is_loaded(pos) {
            return false;
        }
        let Some(old) = self.chunk_set_block_state(pos, state, update_flags) else {
            return false;
        };
        let new = self.state(pos);
        if new == state {
            if update_flags & flags::UPDATE_NEIGHBORS != 0 {
                let old_block = self.block_of(old);
                self.update_neighbors_at(pos, old_block);
                if self.has_analog_output(state) {
                    let block = self.block_of(state);
                    self.update_neighbour_for_output_signal(pos, block);
                }
            }
            if update_flags & flags::UPDATE_KNOWN_SHAPE == 0 && limit > 0 {
                let neighbour_flags =
                    update_flags & !(flags::UPDATE_NEIGHBORS | flags::UPDATE_SUPPRESS_DROPS);
                self.update_indirect_neighbour_shapes(old, pos, neighbour_flags, limit - 1);
                self.update_neighbour_shapes(state, pos, neighbour_flags, limit - 1);
                self.update_indirect_neighbour_shapes(state, pos, neighbour_flags, limit - 1);
            }
        }
        true
    }
    /// `LevelChunk.setBlockState`: the previous state when the write happened.
    fn chunk_set_block_state(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        update_flags: i32,
    ) -> Option<BlockStateId> {
        let current = self.state(pos);
        if current == state {
            return None;
        }
        let old = self.level.set_raw(pos, state)?;
        if old == state {
            return None;
        }
        self.level.observe_block_changed(pos, old, state);
        let new_block = self.block_of(state);
        let block_changed = self.block_of(old) != new_block;
        if block_changed && self.block_of(old) == self.b.named.moving_piston {
            // PistonMovingBlockEntity.preRemoveSideEffects -> finalTick; the
            // block is already replaced, so only the entity goes away.
            self.remove_moving_piston(pos);
        } else if block_changed
            && self.b.has_flag(old, super::data::state_flags::BLOCK_ENTITY)
            && !self.keeps_block_entity(state, old)
        {
            // BlockEntity.preRemoveSideEffects (unless UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS)
            // and removeBlockEntity, before the neighbours hear of the removal.
            let side_effects = update_flags & flags::UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS == 0;
            let updates = self.level.remove_block_entity(pos, old, side_effects);
            for _ in 0..updates {
                // JukeboxBlockEntity.onSongChanged.
                let block = self.block_of(old);
                self.update_neighbors_at(pos, block);
            }
        }
        let moved = update_flags & flags::UPDATE_MOVE_BY_PISTON != 0;
        if (block_changed
            || self
                .b
                .block_data(new_block)
                .is(super::classes::Class::BaseRail))
            && (update_flags & flags::UPDATE_NEIGHBORS != 0 || moved)
        {
            self.affect_neighbors_after_removal(old, pos, moved);
        }
        if self.block_of(self.state(pos)) != new_block {
            return None;
        }
        if update_flags & flags::UPDATE_SKIP_ON_PLACE == 0 {
            self.on_place(state, pos, old, moved);
        }
        Some(old)
    }
    /// `BlockState.shouldChangedStateKeepBlockEntity(oldState)`: copper chests
    /// and copper golem statues keep theirs across their weathering states.
    fn keeps_block_entity(&self, new: BlockStateId, old: BlockStateId) -> bool {
        let (new, old) = (self.block_of(new), self.block_of(old));
        let tags = &self.b.tags;
        (tags.copper_chests.contains(new) && tags.copper_chests.contains(old))
            || (tags.copper_golem_statues.contains(new) && tags.copper_golem_statues.contains(old))
    }
    /// `Level.removeBlock`.
    pub fn remove_block(&mut self, pos: BlockPos, moved_by_piston: bool) -> bool {
        let fluid = self.fluid_at(pos);
        let legacy = self.b.legacy_block(fluid);
        self.set_block(pos, legacy, 3 | if moved_by_piston { 64 } else { 0 })
    }
    /// `Level.destroyBlock(pos, dropResources, null, limit)`.
    pub fn destroy_block(&mut self, pos: BlockPos, drop: bool, limit: i32) -> bool {
        let state = self.state(pos);
        if self.b.is_air(state) {
            return false;
        }
        // Block.spawnDestroyParticles: fire spawns none, straw beds their own.
        let data = self.b.data(state);
        if data.is(Class::StrawBed) {
            self.level_event(2014, pos, state.0 as i32);
        } else if !(data.is(Class::BaseFire) || data.is(Class::Fire) || data.is(Class::SoulFire)) {
            self.level_event(2001, pos, state.0 as i32);
        }
        if drop {
            self.drop_resources(state, pos);
        }
        let fluid = self.fluid_at(pos);
        let legacy = self.b.legacy_block(fluid);
        let destroyed = self.set_block_limit(pos, legacy, 3, limit);
        if destroyed {
            self.game_event_state(pos, "minecraft:block_destroy", state);
        }
        destroyed
    }
    pub fn drop_resources(&mut self, state: BlockStateId, pos: BlockPos) {
        // `MovingPistonBlock.getDrops`: a moving piston's loot is the loot of
        // the block it carries.
        // shortcut: the carried block's `spawnAfterBreak` experience comes with
        // it (vanilla spawns it for the moving piston, which gives none);
        // upgrade when a drop harness covers a moving piston.
        let Some(state) =
            super::pistons::dropped_state(self.b, &self.ticks.moving_pistons, state, pos)
        else {
            return;
        };
        if self.level.game_rules().block_drops {
            self.emit(LevelEvent::DropResources { pos, state });
        }
    }
    /// `Block.updateOrDestroy`.
    pub fn update_or_destroy(
        &mut self,
        old: BlockStateId,
        new: BlockStateId,
        pos: BlockPos,
        update_flags: i32,
        limit: i32,
    ) {
        if new != old {
            if self.b.is_air(new) {
                self.destroy_block(pos, update_flags & flags::UPDATE_SUPPRESS_DROPS == 0, limit);
            } else {
                self.set_block_limit(
                    pos,
                    new,
                    update_flags & !flags::UPDATE_SUPPRESS_DROPS,
                    limit,
                );
            }
        }
    }
    /// `Block.updateFromNeighbourShapes`.
    pub fn update_from_neighbour_shapes(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
    ) -> BlockStateId {
        let mut new = state;
        for direction in UPDATE_SHAPE_ORDER {
            let neighbour = rel(pos, direction);
            let neighbour_state = self.state(neighbour);
            new = self.update_shape(new, pos, direction, neighbour, neighbour_state);
        }
        new
    }
    /// `BlockStateBase.updateNeighbourShapes`.
    pub fn update_neighbour_shapes(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        update_flags: i32,
        limit: i32,
    ) {
        for direction in UPDATE_SHAPE_ORDER {
            let neighbour = rel(pos, direction);
            self.neighbor_shape_changed(
                direction.opposite(),
                neighbour,
                pos,
                state,
                update_flags,
                limit,
            );
        }
    }

    // ---- neighbour updater ----
    pub fn update_neighbors_at(&mut self, pos: BlockPos, block: BlockId) {
        self.add_and_run(Update::Multi {
            source: pos,
            block,
            skip: None,
            index: 0,
        });
    }
    pub fn update_neighbors_at_except(&mut self, pos: BlockPos, block: BlockId, skip: Direction) {
        let index = usize::from(UPDATE_ORDER[0] == skip);
        self.add_and_run(Update::Multi {
            source: pos,
            block,
            skip: Some(skip),
            index,
        });
    }
    pub fn neighbor_changed_at(&mut self, pos: BlockPos, block: BlockId) {
        self.add_and_run(Update::Simple { pos, block });
    }
    pub fn neighbor_changed_full(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        block: BlockId,
        moved: bool,
    ) {
        self.add_and_run(Update::Full {
            state,
            pos,
            block,
            moved,
        });
    }
    pub fn neighbor_shape_changed(
        &mut self,
        direction: Direction,
        pos: BlockPos,
        neighbor_pos: BlockPos,
        neighbor_state: BlockStateId,
        update_flags: i32,
        limit: i32,
    ) {
        self.add_and_run(Update::Shape {
            direction,
            neighbor_state,
            pos,
            neighbor_pos,
            flags: update_flags,
            limit,
        });
    }
    fn add_and_run(&mut self, update: Update) {
        let updater = &mut self.ticks.updater;
        let running = updater.count > 0;
        let too_many = updater.count >= MAX_CHAINED_NEIGHBOR_UPDATES;
        updater.count += 1;
        if !too_many {
            if running {
                updater.added.push(update);
            } else {
                updater.stack.push(update);
            }
        }
        if !running {
            self.run_updates();
        }
    }
    fn run_updates(&mut self) {
        loop {
            let updater = &mut self.ticks.updater;
            if updater.stack.is_empty() && updater.added.is_empty() {
                break;
            }
            while let Some(update) = updater.added.pop() {
                updater.stack.push(update);
            }
            while let Some(top) = self.ticks.updater.stack.last_mut() {
                let (step, more) = top.next();
                self.execute(step);
                if !more {
                    self.ticks.updater.stack.pop();
                    break;
                }
                if !self.ticks.updater.added.is_empty() {
                    break;
                }
            }
        }
        let updater = &mut self.ticks.updater;
        updater.stack.clear();
        updater.added.clear();
        updater.count = 0;
    }
    fn execute(&mut self, step: Step) {
        match step {
            Step::Shape {
                direction,
                neighbor_state,
                pos,
                neighbor_pos,
                flags: update_flags,
                limit,
            } => {
                let current = self.state(pos);
                if update_flags & flags::UPDATE_SKIP_SHAPE_UPDATE_ON_WIRE == 0
                    || !self.b.table.is(current, self.b.named.redstone_wire)
                {
                    let new =
                        self.update_shape(current, pos, direction, neighbor_pos, neighbor_state);
                    self.update_or_destroy(current, new, pos, update_flags, limit);
                }
            }
            Step::Neighbor {
                state,
                pos,
                block,
                moved,
            } => {
                let state = state.unwrap_or_else(|| self.state(pos));
                self.neighbor_changed(state, pos, block, moved);
            }
        }
    }
    /// `Level.updateNeighbourForOutputSignal`.
    pub fn update_neighbour_for_output_signal(&mut self, pos: BlockPos, block: BlockId) {
        for direction in super::update::HORIZONTAL {
            let mut relative = rel(pos, direction);
            if !self.level.is_loaded(relative) {
                continue;
            }
            let mut state = self.state(relative);
            if self.b.table.is(state, self.b.named.comparator) {
                self.neighbor_changed_full(state, relative, block, false);
            } else if self.is_redstone_conductor(state, relative) {
                relative = rel(relative, direction);
                state = self.state(relative);
                if self.b.table.is(state, self.b.named.comparator) {
                    self.neighbor_changed_full(state, relative, block, false);
                }
            }
        }
    }
}
