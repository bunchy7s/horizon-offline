//! Scheduled block/fluid tick machinery for Java Edition 26.3. Type IDs identify
//! registry-owned types; each block/fluid registry uses its own LevelTicks.
mod generation;
pub use generation::{
    ProtoChunkTicks, TickContainerAccess, WorldGenTickAccess, WorldGenTickContainers,
};
mod heap;
mod index;
use crate::fast_hash::{FxHashMap, FxHashSet};
use crate::position::{BlockPos, ChunkPos};
use heap::Heap;
use index::DeadlineIndex;
use std::{cmp::Ordering, collections::VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TickTypeId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i8)]
pub enum TickPriority {
    ExtremelyHigh = -3,
    VeryHigh = -2,
    High = -1,
    #[default]
    Normal = 0,
    Low = 1,
    VeryLow = 2,
    ExtremelyLow = 3,
}
impl TickPriority {
    pub const fn by_value(value: i32) -> Self {
        match value {
            i32::MIN..=-3 => Self::ExtremelyHigh,
            -2 => Self::VeryHigh,
            -1 => Self::High,
            0 => Self::Normal,
            1 => Self::Low,
            2 => Self::VeryLow,
            3..=i32::MAX => Self::ExtremelyLow,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduledTick {
    pub kind: TickTypeId,
    pub pos: BlockPos,
    pub trigger_tick: i64,
    pub priority: TickPriority,
    pub sub_tick_order: i64,
}
impl ScheduledTick {
    pub fn drain_cmp(self, other: Self) -> Ordering {
        self.trigger_tick
            .cmp(&other.trigger_tick)
            .then_with(|| self.intra_cmp(other))
    }
    pub fn intra_cmp(self, other: Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.sub_tick_order.cmp(&other.sub_tick_order))
    }
    pub fn saved(self, game_time: i64) -> SavedTick {
        SavedTick {
            kind: self.kind,
            pos: self.pos,
            delay: self.trigger_tick.wrapping_sub(game_time) as i32,
            priority: self.priority,
        }
    }
    fn key(self) -> (TickTypeId, BlockPos) {
        (self.kind, self.pos)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedTick {
    pub kind: TickTypeId,
    pub pos: BlockPos,
    pub delay: i32,
    pub priority: TickPriority,
}
impl SavedTick {
    pub fn unpack(self, game_time: i64, sub_tick_order: i64) -> ScheduledTick {
        ScheduledTick {
            kind: self.kind,
            pos: self.pos,
            trigger_tick: game_time.wrapping_add(i64::from(self.delay)),
            priority: self.priority,
            sub_tick_order,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChunkTicks {
    queue: Heap<ScheduledTick>,
    pending: Option<Vec<SavedTick>>,
    positions: FxHashSet<(TickTypeId, BlockPos)>,
}
impl ChunkTicks {
    pub fn from_saved(ticks: Vec<SavedTick>) -> Self {
        Self {
            positions: ticks.iter().map(|t| (t.kind, t.pos)).collect(),
            pending: Some(ticks),
            ..Self::default()
        }
    }
    pub fn peek(&self) -> Option<ScheduledTick> {
        self.queue.peek()
    }
    pub fn poll(&mut self) -> Option<ScheduledTick> {
        let tick = self.queue.pop(ScheduledTick::drain_cmp)?;
        self.positions.remove(&tick.key());
        Some(tick)
    }
    /// First schedule wins for a type/position, irrespective of timing/priority.
    pub fn schedule(&mut self, tick: ScheduledTick) -> bool {
        if !self.positions.insert(tick.key()) {
            return false;
        }
        self.queue.push(tick, ScheduledTick::drain_cmp);
        true
    }
    pub fn has_scheduled(&self, pos: BlockPos, kind: TickTypeId) -> bool {
        self.positions.contains(&(kind, pos))
    }
    pub fn count(&self) -> usize {
        self.queue.values.len() + self.pending.as_ref().map_or(0, Vec::len)
    }
    /// Heap traversal order, as exposed by vanilla getAll, rather than sorted order.
    pub fn queued(&self) -> &[ScheduledTick] {
        &self.queue.values
    }
    pub fn remove_if(&mut self, mut predicate: impl FnMut(ScheduledTick) -> bool) {
        self.queue.remove_if(
            |tick| {
                if !predicate(tick) {
                    return false;
                }
                self.positions.remove(&tick.key());
                true
            },
            ScheduledTick::drain_cmp,
        );
    }
    pub fn pack(&self, game_time: i64) -> Vec<SavedTick> {
        let mut saved = self.pending.clone().unwrap_or_default();
        let mut queued = self.queue.values.clone();
        queued.sort_by_key(|t| t.sub_tick_order);
        saved.extend(queued.into_iter().map(|t| t.saved(game_time)));
        saved
    }
    pub fn unpack(&mut self, game_time: i64) {
        if let Some(pending) = self.pending.take() {
            let mut order = (pending.len() as i32).wrapping_neg() as i64;
            for saved in pending {
                // Vanilla does not deduplicate a saved list during unpack.
                self.queue
                    .push(saved.unpack(game_time, order), ScheduledTick::drain_cmp);
                order = order.wrapping_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tick(kind: u32, x: i32, time: i64, priority: i32, order: i64) -> ScheduledTick {
        ScheduledTick {
            kind: TickTypeId(kind),
            pos: BlockPos::new(x, 64, 0),
            trigger_tick: time,
            priority: TickPriority::by_value(priority),
            sub_tick_order: order,
        }
    }
    fn level() -> LevelTicks {
        let mut level = LevelTicks::default();
        for x in -1..=2 {
            level
                .add_chunk(ChunkPos::new(x, 0), ChunkTicks::default())
                .unwrap();
        }
        level
    }
    #[test]
    fn due_chunks_compete_by_priority_not_trigger_time() {
        let mut level = level();
        for tick in [
            tick(0, 0, 1, 3, 10),
            tick(1, 1, 2, -3, 11),
            tick(2, 16, 3, 0, 12),
        ] {
            assert_eq!(level.schedule(tick), ScheduleResult::Added);
        }
        assert_eq!(
            level.schedule(tick(0, 0, -100, -3, -10)),
            ScheduleResult::Duplicate
        );
        let mut ran = Vec::new();
        level
            .tick(10, 10, |_| true, |_, tick| ran.push(tick.kind.0))
            .unwrap();
        assert_eq!(ran, [2, 0, 1]);
        assert_eq!(level.count(), 0);
    }
    #[test]
    fn callback_scheduling_waits_until_the_next_collection() {
        let mut level = level();
        let original = tick(0, 0, 0, 0, 0);
        level.schedule(original);
        let mut ran = 0;
        level
            .tick(
                0,
                100,
                |_| true,
                |level, tick| {
                    ran += 1;
                    assert!(!level.will_tick_this_tick(tick.pos, tick.kind));
                    assert_eq!(level.schedule(tick), ScheduleResult::Added);
                    assert!(level.has_scheduled(tick.pos, tick.kind));
                },
            )
            .unwrap();
        assert_eq!(ran, 1);
        assert_eq!(level.count(), 1);
        level.tick(0, 100, |_| true, |_, _| ran += 1).unwrap();
        assert_eq!(ran, 2);
        assert_eq!(level.count(), 0);
    }
    #[test]
    fn pending_ticks_keep_list_order_and_java_time_overflow() {
        let a = tick(0, 0, i64::MAX, -9, 10).saved(i64::MIN);
        assert_eq!(a.delay, -1);
        let b = SavedTick {
            delay: 1,
            kind: TickTypeId(1),
            ..a
        };
        let mut chunk = ChunkTicks::from_saved(vec![a, b, a]);
        assert_eq!(chunk.count(), 3);
        assert!(chunk.peek().is_none());
        assert!(!chunk.schedule(tick(0, 0, 100, 0, 0)));
        chunk.unpack(i64::MAX);
        assert_eq!(chunk.count(), 3);
        assert_eq!(chunk.poll().unwrap().trigger_tick, i64::MIN);
        assert_eq!(chunk.poll().unwrap().sub_tick_order, -3);
        assert_eq!(chunk.poll().unwrap().sub_tick_order, -1);
        chunk.unpack(0);
        assert_eq!(chunk.count(), 0);
    }
    #[test]
    fn disabled_and_removed_chunks_retain_their_queue() {
        let mut level = level();
        level.schedule(tick(0, 0, 0, 0, 0));
        level
            .tick(10, 10, |_| false, |_, _| panic!("disabled chunk ran"))
            .unwrap();
        let chunk = level.remove_chunk(ChunkPos::new(0, 0)).unwrap();
        assert_eq!(chunk.count(), 1);
        assert_eq!(
            level.schedule(tick(1, 0, 0, 0, 0)),
            ScheduleResult::MissingChunk
        );
        level.add_chunk(ChunkPos::new(0, 0), chunk).unwrap();
        let mut ran = Vec::new();
        level
            .tick(10, 1, |_| true, |_, tick| ran.push(tick))
            .unwrap();
        assert_eq!(ran, [tick(0, 0, 0, 0, 0)]);
    }
    #[test]
    fn collection_guards_and_membership_survive_area_mutation() {
        let mut level = level();
        level.schedule(tick(0, 0, 0, 0, 0));
        level.schedule(tick(1, 1, 0, 0, 1));
        level.begin_tick(0, 10, |_| true).unwrap();
        assert_eq!(
            level.begin_tick(0, 10, |_| true),
            Err(TickError::AlreadyCollecting)
        );
        assert_eq!(level.finish_tick(), Err(TickError::TicksRemain));
        assert_eq!(level.next_tick().unwrap().unwrap().kind, TickTypeId(0));
        let probe = BlockPos::new(1, 64, 0);
        assert!(level.will_tick_this_tick(probe, TickTypeId(1)));
        level.clear_area(TickArea::new(probe, probe));
        // Preserve vanilla's stale lazily built set until cleanup.
        assert!(level.will_tick_this_tick(probe, TickTypeId(1)));
        assert!(level.next_tick().unwrap().is_none());
        level.finish_tick().unwrap();
        assert!(!level.will_tick_this_tick(probe, TickTypeId(1)));
        assert_eq!(level.next_tick(), Err(TickError::NotCollecting));
    }
    #[test]
    fn copying_active_ticks_preserves_relative_order_and_wraps() {
        let mut level = level();
        level.schedule(tick(0, 0, 0, 0, i64::MAX - 1));
        level.schedule(tick(1, 1, 0, 0, i64::MAX));
        level.begin_tick(0, 10, |_| true).unwrap();
        level.next_tick().unwrap();
        level.copy_area(
            TickArea::new(BlockPos::new(0, 64, 0), BlockPos::new(1, 64, 0)),
            BlockPos::new(16, 0, 0),
        );
        let copied = level.chunk(ChunkPos::new(1, 0)).unwrap().queued();
        assert_eq!(copied.len(), 2);
        assert_eq!(copied[0].sub_tick_order, i64::MIN);
        assert_eq!(copied[1].sub_tick_order, i64::MIN + 1);
        while level.next_tick().unwrap().is_some() {}
        level.finish_tick().unwrap();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickArea {
    pub min: BlockPos,
    pub max: BlockPos,
}
impl TickArea {
    pub fn new(a: BlockPos, b: BlockPos) -> Self {
        Self {
            min: BlockPos::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
            max: BlockPos::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
        }
    }
    pub fn contains(self, pos: BlockPos) -> bool {
        (self.min.x..=self.max.x).contains(&pos.x)
            && (self.min.y..=self.max.y).contains(&pos.y)
            && (self.min.z..=self.max.z).contains(&pos.z)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleResult {
    Added,
    Duplicate,
    MissingChunk,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickError {
    AlreadyCollecting,
    NotCollecting,
    TicksRemain,
    ChunkAlreadyAdded,
    WrongChunk,
}
impl std::fmt::Display for TickError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "scheduled tick invariant: {self:?}")
    }
}
impl std::error::Error for TickError {}

#[derive(Default)]
pub struct LevelTicks {
    chunks: FxHashMap<i64, ChunkTicks>,
    deadlines: DeadlineIndex,
    containers_to_tick: Heap<i64>,
    to_run: VecDeque<ScheduledTick>,
    already_run: Vec<ScheduledTick>,
    to_run_set: FxHashSet<(TickTypeId, BlockPos)>,
    collecting: bool,
}
impl LevelTicks {
    pub fn add_chunk(&mut self, pos: ChunkPos, ticks: ChunkTicks) -> Result<(), TickError> {
        let key = pos.pack();
        if self.chunks.contains_key(&key) {
            return Err(TickError::ChunkAlreadyAdded);
        }
        if ticks.queue.values.iter().any(|t| t.pos.chunk() != pos)
            || ticks
                .pending
                .as_ref()
                .is_some_and(|p| p.iter().any(|t| t.pos.chunk() != pos))
        {
            return Err(TickError::WrongChunk);
        }
        if let Some(tick) = ticks.peek() {
            self.deadlines.put(key, tick.trigger_tick);
        }
        self.chunks.insert(key, ticks);
        Ok(())
    }
    pub fn remove_chunk(&mut self, pos: ChunkPos) -> Option<ChunkTicks> {
        self.deadlines.remove(pos.pack());
        self.chunks.remove(&pos.pack())
    }
    pub fn chunk(&self, pos: ChunkPos) -> Option<&ChunkTicks> {
        self.chunks.get(&pos.pack())
    }
    pub fn unpack_chunk(&mut self, pos: ChunkPos, game_time: i64) {
        if let Some(chunk) = self.chunks.get_mut(&pos.pack()) {
            chunk.unpack(game_time);
            if let Some(tick) = chunk.peek() {
                self.deadlines.put(pos.pack(), tick.trigger_tick);
            }
        }
    }
    pub fn schedule(&mut self, tick: ScheduledTick) -> ScheduleResult {
        let key = tick.pos.chunk().pack();
        let Some(chunk) = self.chunks.get_mut(&key) else {
            return ScheduleResult::MissingChunk;
        };
        if !chunk.schedule(tick) {
            return ScheduleResult::Duplicate;
        }
        if chunk.peek() == Some(tick) {
            self.deadlines.put(key, tick.trigger_tick);
        }
        ScheduleResult::Added
    }
    pub fn has_scheduled(&self, pos: BlockPos, kind: TickTypeId) -> bool {
        self.chunk(pos.chunk())
            .is_some_and(|c| c.has_scheduled(pos, kind))
    }
    pub fn will_tick_this_tick(&mut self, pos: BlockPos, kind: TickTypeId) -> bool {
        if self.to_run_set.is_empty() && !self.to_run.is_empty() {
            self.to_run_set.extend(self.to_run.iter().map(|t| t.key()));
        }
        self.to_run_set.contains(&(kind, pos))
    }
    pub fn count(&self) -> usize {
        self.chunks.values().map(ChunkTicks::count).sum()
    }
    pub fn begin_tick(
        &mut self,
        game_time: i64,
        maximum: i32,
        mut can_tick: impl FnMut(ChunkPos) -> bool,
    ) -> Result<(), TickError> {
        if self.collecting {
            return Err(TickError::AlreadyCollecting);
        }
        self.collecting = true;
        let mut containers = std::mem::take(&mut self.containers_to_tick);
        let chunks = &mut self.chunks;
        let compare = |a: i64, b: i64| {
            chunks[&a]
                .peek()
                .unwrap()
                .intra_cmp(chunks[&b].peek().unwrap())
        };
        self.deadlines.visit(|key, deadline| {
            if *deadline > game_time {
                return false;
            }
            let Some(head) = chunks.get(&key).and_then(ChunkTicks::peek) else {
                return true;
            };
            if head.trigger_tick > game_time {
                *deadline = head.trigger_tick;
                return false;
            }
            if !can_tick(ChunkPos::unpack(key)) {
                return false;
            }
            containers.push(key, compare);
            true
        });
        let maximum = maximum.max(0) as usize;
        while self.to_run.len() < maximum {
            let Some(key) = containers.pop(|a, b| {
                self.chunks[&a]
                    .peek()
                    .unwrap()
                    .intra_cmp(self.chunks[&b].peek().unwrap())
            }) else {
                break;
            };
            let chunk = self.chunks.get_mut(&key).unwrap();
            self.to_run.push_back(chunk.poll().unwrap());
            let competing = containers.peek().and_then(|key| self.chunks[&key].peek());
            let chunk = self.chunks.get_mut(&key).unwrap();
            while self.to_run.len() < maximum {
                let Some(head) = chunk.peek() else {
                    break;
                };
                if head.trigger_tick > game_time
                    || competing.is_some_and(|other| head.intra_cmp(other) == Ordering::Greater)
                {
                    break;
                }
                self.to_run.push_back(chunk.poll().unwrap());
            }
            if let Some(head) = chunk.peek() {
                if head.trigger_tick <= game_time && self.to_run.len() < maximum {
                    containers.push(key, |a, b| {
                        self.chunks[&a]
                            .peek()
                            .unwrap()
                            .intra_cmp(self.chunks[&b].peek().unwrap())
                    });
                } else {
                    self.deadlines.put(key, head.trigger_tick);
                }
            }
        }
        for &key in &containers.values {
            self.deadlines
                .put(key, self.chunks[&key].peek().unwrap().trigger_tick);
        }
        containers.values.clear();
        self.containers_to_tick = containers;
        Ok(())
    }
    /// Pop immediately before invoking block/fluid behavior. The current tick is
    /// already-run, absent from will_tick, and available to area copying.
    pub fn next_tick(&mut self) -> Result<Option<ScheduledTick>, TickError> {
        if !self.collecting {
            return Err(TickError::NotCollecting);
        }
        let Some(tick) = self.to_run.pop_front() else {
            return Ok(None);
        };
        if !self.to_run_set.is_empty() {
            self.to_run_set.remove(&tick.key());
        }
        self.already_run.push(tick);
        Ok(Some(tick))
    }
    pub fn finish_tick(&mut self) -> Result<(), TickError> {
        if !self.collecting {
            return Err(TickError::NotCollecting);
        }
        if !self.to_run.is_empty() {
            return Err(TickError::TicksRemain);
        }
        self.already_run.clear();
        self.to_run_set.clear();
        self.collecting = false;
        Ok(())
    }
    pub fn tick(
        &mut self,
        game_time: i64,
        maximum: i32,
        can_tick: impl FnMut(ChunkPos) -> bool,
        mut run: impl FnMut(&mut Self, ScheduledTick),
    ) -> Result<(), TickError> {
        self.begin_tick(game_time, maximum, can_tick)?;
        while let Some(tick) = self.next_tick()? {
            run(self, tick);
        }
        self.finish_tick()
    }
    fn chunks_in_area(&self, area: TickArea) -> Vec<i64> {
        let min = area.min.chunk();
        let max = area.max.chunk();
        if min.x > max.x || min.z > max.z {
            return Vec::new();
        }
        let width = (i64::from(max.x) - i64::from(min.x) + 1) as u64;
        let depth = (i64::from(max.z) - i64::from(min.z) + 1) as u64;
        // Small edits should probe their grid rather than scan every loaded chunk.
        // Large empty areas should visit loaded chunks rather than the empty grid.
        if width * depth <= self.chunks.len() as u64 {
            let mut keys = Vec::new();
            for x in min.x..=max.x {
                for z in min.z..=max.z {
                    let key = ChunkPos::new(x, z).pack();
                    if self.chunks.contains_key(&key) {
                        keys.push(key);
                    }
                }
            }
            return keys;
        }
        let mut keys: Vec<_> = self
            .chunks
            .keys()
            .copied()
            .filter(|key| {
                let pos = ChunkPos::unpack(*key);
                (min.x..=max.x).contains(&pos.x) && (min.z..=max.z).contains(&pos.z)
            })
            .collect();
        keys.sort_by_key(|key| {
            let p = ChunkPos::unpack(*key);
            (p.x, p.z)
        });
        keys
    }
    pub fn clear_area(&mut self, area: TickArea) {
        for key in self.chunks_in_area(area) {
            let chunk = self.chunks.get_mut(&key).unwrap();
            let old = chunk.peek();
            chunk.remove_if(|tick| area.contains(tick.pos));
            if chunk.peek() != old {
                if let Some(tick) = chunk.peek() {
                    self.deadlines.put(key, tick.trigger_tick);
                } else {
                    self.deadlines.remove(key);
                }
            }
        }
        self.already_run.retain(|tick| !area.contains(tick.pos));
        self.to_run.retain(|tick| !area.contains(tick.pos));
        // Vanilla clearArea leaves its lazily computed membership set intact.
    }
    fn area_ticks(&self, area: TickArea) -> Vec<ScheduledTick> {
        let mut ticks: Vec<_> = self
            .already_run
            .iter()
            .chain(self.to_run.iter())
            .copied()
            .filter(|t| area.contains(t.pos))
            .collect();
        for key in self.chunks_in_area(area) {
            ticks.extend(
                self.chunks[&key]
                    .queued()
                    .iter()
                    .copied()
                    .filter(|t| area.contains(t.pos)),
            );
        }
        ticks
    }
    fn schedule_copy(&mut self, ticks: Vec<ScheduledTick>, offset: BlockPos) {
        let Some(min) = ticks.iter().map(|t| t.sub_tick_order).min() else {
            return;
        };
        let max = ticks.iter().map(|t| t.sub_tick_order).max().unwrap();
        for mut tick in ticks {
            tick.pos = tick.pos.offset(offset.x, offset.y, offset.z);
            tick.sub_tick_order = tick
                .sub_tick_order
                .wrapping_sub(min)
                .wrapping_add(max)
                .wrapping_add(1);
            self.schedule(tick);
        }
    }
    pub fn copy_area(&mut self, area: TickArea, offset: BlockPos) {
        self.schedule_copy(self.area_ticks(area), offset);
    }
}
