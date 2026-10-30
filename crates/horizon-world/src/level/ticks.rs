//! Per-level mutable simulation state: scheduled block and fluid ticks plus the
//! in-flight neighbour-update queue and the random-tick position generator.
use super::{states::BlockId, update::NeighborUpdater};
use crate::fluid::FluidKind;
use horizon_core::{
    position::{BlockPos, ChunkPos},
    ticks::{
        ChunkTicks, LevelTicks, ScheduleResult, ScheduledTick, TickError, TickPriority, TickTypeId,
    },
};

/// Owned by the caller next to its chunk storage. Chunks must be registered
/// with [`Ticks::add_chunk`] before ticks can be scheduled in them, mirroring
/// vanilla's `LevelTicks.allContainers` (scheduling elsewhere is dropped).
pub struct Ticks {
    /// Block ticks keyed by `BlockId` (vanilla Block registry ID).
    pub blocks: LevelTicks,
    /// Fluid ticks keyed by vanilla Fluid registry ID (`FluidKind as u32`).
    pub fluids: LevelTicks,
    sub_tick: i64,
    /// `Level.randValue`, the LCG behind `getBlockRandomPos`.
    pub random_tick_value: i32,
    pub(crate) updater: NeighborUpdater,
    /// `ComparatorBlockEntity.output` per comparator position (block-entity
    /// state owned by the level; persist it with the chunk if desired).
    pub comparator_outputs: std::collections::HashMap<BlockPos, i32>,
    /// `RedstoneTorchBlock.RECENT_TOGGLES` (position, game time).
    pub torch_toggles: Vec<(BlockPos, i64)>,
    /// Moving-piston block entities in ticker order (render and collide
    /// moving blocks from these; see `piston_shape`).
    pub moving_pistons: Vec<(BlockPos, super::pistons::MovingPiston)>,
    /// The pushes this tick's moving-piston block entities asked for: each
    /// piston with the progress it is about to reach, in the order they ticked
    /// (`PistonMovingBlockEntity.tick`'s `moveCollidedEntities` call). The
    /// caller moves entities by these; `Ticks` has no entities of its own.
    pub piston_pushes: Vec<(BlockPos, super::pistons::MovingPiston, f32)>,
    pub(crate) next_block_entity_id: u64,
    /// `ServerLevel.blockEvents`, run by `Behaviours::run_block_events`.
    pub block_events: Vec<super::pistons::BlockEventData>,
    /// `ServerLevel.isHandlingTick` (true while scheduled ticks run).
    pub(crate) handling_tick: bool,
    /// Worldgen shape updates collect the ticks they schedule here, for the
    /// proto chunks of their region, instead of scheduling them.
    pub(crate) capture: Option<Vec<CapturedTick>>,
}
/// A tick scheduled while `Ticks::capture` is set.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CapturedTick {
    pub fluid: bool,
    /// `BlockId` for block ticks, `FluidKind` for fluid ticks.
    pub kind: u32,
    pub pos: BlockPos,
    pub delay: i32,
}
impl Default for Ticks {
    fn default() -> Self {
        Self::new(0)
    }
}
impl Ticks {
    /// `random_tick_seed` seeds `Level.randValue` (vanilla draws it from a
    /// thread-local random at level construction).
    pub fn new(random_tick_seed: i32) -> Self {
        Self {
            blocks: LevelTicks::default(),
            fluids: LevelTicks::default(),
            sub_tick: 0,
            random_tick_value: random_tick_seed,
            updater: NeighborUpdater::default(),
            comparator_outputs: Default::default(),
            torch_toggles: Vec::new(),
            moving_pistons: Vec::new(),
            piston_pushes: Vec::new(),
            next_block_entity_id: 0,
            block_events: Vec::new(),
            handling_tick: false,
            capture: None,
        }
    }
    /// Register a loaded chunk's saved block and fluid ticks.
    pub fn add_chunk(
        &mut self,
        pos: ChunkPos,
        blocks: ChunkTicks,
        fluids: ChunkTicks,
    ) -> Result<(), TickError> {
        self.blocks.add_chunk(pos, blocks)?;
        self.fluids.add_chunk(pos, fluids)
    }
    /// A moving-piston block entity loaded with its chunk, as a new ticker.
    pub fn load_moving_piston(&mut self, pos: BlockPos, mut piston: super::pistons::MovingPiston) {
        self.moving_pistons.retain(|(p, _)| *p != pos);
        self.next_block_entity_id += 1;
        piston.id = self.next_block_entity_id;
        self.moving_pistons.push((pos, piston));
    }
    /// Register a chunk without saved ticks.
    pub fn add_empty_chunk(&mut self, pos: ChunkPos) -> Result<(), TickError> {
        self.add_chunk(pos, ChunkTicks::default(), ChunkTicks::default())
    }
    pub fn remove_chunk(&mut self, pos: ChunkPos) -> (Option<ChunkTicks>, Option<ChunkTicks>) {
        (self.blocks.remove_chunk(pos), self.fluids.remove_chunk(pos))
    }
    /// `ServerLevel.nextSubTickCount`, shared by block and fluid ticks.
    pub fn next_sub_tick(&mut self) -> i64 {
        let value = self.sub_tick;
        self.sub_tick = self.sub_tick.wrapping_add(1);
        value
    }
    pub fn schedule_block(
        &mut self,
        game_time: i64,
        pos: BlockPos,
        block: BlockId,
        delay: i32,
        priority: TickPriority,
    ) -> ScheduleResult {
        if let Some(captured) = &mut self.capture {
            captured.push(CapturedTick {
                fluid: false,
                kind: u32::from(block.0),
                pos,
                delay,
            });
            return ScheduleResult::Added;
        }
        let tick = ScheduledTick {
            kind: TickTypeId(u32::from(block.0)),
            pos,
            trigger_tick: game_time.wrapping_add(i64::from(delay)),
            priority,
            sub_tick_order: self.next_sub_tick(),
        };
        self.blocks.schedule(tick)
    }
    pub fn schedule_fluid(
        &mut self,
        game_time: i64,
        pos: BlockPos,
        fluid: FluidKind,
        delay: i32,
    ) -> ScheduleResult {
        if let Some(captured) = &mut self.capture {
            captured.push(CapturedTick {
                fluid: true,
                kind: fluid as u32,
                pos,
                delay,
            });
            return ScheduleResult::Added;
        }
        let tick = ScheduledTick {
            kind: TickTypeId(fluid as u32),
            pos,
            trigger_tick: game_time.wrapping_add(i64::from(delay)),
            priority: TickPriority::Normal,
            sub_tick_order: self.next_sub_tick(),
        };
        self.fluids.schedule(tick)
    }
    pub fn has_block_tick(&self, pos: BlockPos, block: BlockId) -> bool {
        self.blocks
            .has_scheduled(pos, TickTypeId(u32::from(block.0)))
    }
    pub fn will_block_tick_this_tick(&mut self, pos: BlockPos, block: BlockId) -> bool {
        self.blocks
            .will_tick_this_tick(pos, TickTypeId(u32::from(block.0)))
    }
    /// `Level.getBlockRandomPos`.
    pub fn block_random_pos(&mut self, x: i32, y: i32, z: i32, y_mask: i32) -> BlockPos {
        self.random_tick_value = self
            .random_tick_value
            .wrapping_mul(3)
            .wrapping_add(1_013_904_223);
        let value = self.random_tick_value >> 2;
        BlockPos::new(
            x + (value & 15),
            y + ((value >> 16) & y_mask),
            z + ((value >> 8) & 15),
        )
    }
}
