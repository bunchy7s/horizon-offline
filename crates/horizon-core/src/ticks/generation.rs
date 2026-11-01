//! Generation-stage tick storage and routing, before level tick execution.
use super::{ChunkTicks, SavedTick, ScheduledTick, TickTypeId};
use crate::fast_hash::FxHashSet;
use crate::position::BlockPos;

/// Proto chunks retain the first request for a type/position in insertion order.
/// Newly scheduled delays are zero; loaded delays and priorities are preserved.
#[derive(Clone, Debug, Default)]
pub struct ProtoChunkTicks {
    ticks: Vec<SavedTick>,
    positions: FxHashSet<(TickTypeId, BlockPos)>,
}
impl ProtoChunkTicks {
    pub fn from_saved(ticks: impl IntoIterator<Item = SavedTick>) -> Self {
        let mut result = Self::default();
        for tick in ticks {
            result.schedule_saved(tick);
        }
        result
    }
    fn schedule_saved(&mut self, tick: SavedTick) -> bool {
        if !self.positions.insert((tick.kind, tick.pos)) {
            return false;
        }
        self.ticks.push(tick);
        true
    }
    pub fn schedule(&mut self, tick: ScheduledTick) -> bool {
        self.schedule_saved(SavedTick {
            kind: tick.kind,
            pos: tick.pos,
            delay: 0,
            priority: tick.priority,
        })
    }
    pub fn has_scheduled(&self, pos: BlockPos, kind: TickTypeId) -> bool {
        self.positions.contains(&(kind, pos))
    }
    pub fn count(&self) -> usize {
        self.ticks.len()
    }
    /// Borrow the current saved list. Game time does not affect proto delays.
    pub fn pack(&self, _game_time: i64) -> &[SavedTick] {
        &self.ticks
    }
    /// An independent snapshot, matching scheduledTicks rather than pack's view.
    pub fn scheduled_ticks(&self) -> Vec<SavedTick> {
        self.ticks.clone()
    }
}

pub trait TickContainerAccess {
    fn schedule(&mut self, tick: ScheduledTick) -> bool;
    fn has_scheduled(&self, pos: BlockPos, kind: TickTypeId) -> bool;
}
impl TickContainerAccess for ProtoChunkTicks {
    fn schedule(&mut self, tick: ScheduledTick) -> bool {
        Self::schedule(self, tick)
    }
    fn has_scheduled(&self, pos: BlockPos, kind: TickTypeId) -> bool {
        Self::has_scheduled(self, pos, kind)
    }
}
impl TickContainerAccess for ChunkTicks {
    fn schedule(&mut self, tick: ScheduledTick) -> bool {
        Self::schedule(self, tick)
    }
    fn has_scheduled(&self, pos: BlockPos, kind: TickTypeId) -> bool {
        Self::has_scheduled(self, pos, kind)
    }
}

/// World-owned selection of the receiving chunk/container. This receives the
/// original block position, so lookup policy and its errors remain observable.
pub trait WorldGenTickContainers {
    type Container: TickContainerAccess;
    type Error;
    fn container_at(&mut self, pos: BlockPos) -> Result<&mut Self::Container, Self::Error>;
}

pub struct WorldGenTickAccess<C> {
    containers: C,
}
impl<C: WorldGenTickContainers> WorldGenTickAccess<C> {
    pub fn new(containers: C) -> Self {
        Self { containers }
    }
    pub fn containers(&self) -> &C {
        &self.containers
    }
    pub fn containers_mut(&mut self) -> &mut C {
        &mut self.containers
    }
    pub fn schedule(&mut self, tick: ScheduledTick) -> Result<bool, C::Error> {
        Ok(self.containers.container_at(tick.pos)?.schedule(tick))
    }
    pub fn has_scheduled(&mut self, pos: BlockPos, kind: TickTypeId) -> Result<bool, C::Error> {
        Ok(self.containers.container_at(pos)?.has_scheduled(pos, kind))
    }
    /// Generation access never reports a currently executing tick or a level count.
    pub fn will_tick_this_tick(&self, _pos: BlockPos, _kind: TickTypeId) -> bool {
        false
    }
    pub fn count(&self) -> usize {
        0
    }
}
