//! Tick scheduler membership, invalidated by resident key changes or resets.
use crate::{
    chunk_map::{ChunkMap, ChunkMembershipDelta},
    fast_hash::FxHashSet,
};

type Key = (i32, i32);

#[derive(Default)]
pub struct TickChunkMembership {
    keys: FxHashSet<Key>,
    revision: Option<u64>,
    added: Vec<Key>,
    removed: Vec<Key>,
    /// Loaded chunks with saved ticks that have not started ticking yet
    /// (`LevelChunkTicks.pendingTicks`, unpacked by `startTickingChunk`).
    waiting: FxHashSet<Key>,
}

impl TickChunkMembership {
    /// A scheduler reset must re-register resident chunks even if the world did
    /// not change. Keep this invalidation with the keys so callers cannot forget.
    pub fn clear(&mut self) {
        self.keys.clear();
        self.revision = None;
        self.added.clear();
        self.removed.clear();
        self.waiting.clear();
    }

    /// Returns false in O(1) when resident membership is unchanged. On change,
    /// update the existing set and record only registration/removal deltas.
    pub fn synchronize(&mut self, chunks: &ChunkMap) -> bool {
        let revision = chunks.membership_revision();
        if self.revision == Some(revision) {
            return false;
        }
        self.added.clear();
        self.removed.clear();

        if let Some(deltas) = self.revision.and_then(|rev| chunks.deltas_since(rev)) {
            let mut removed_waiting = FxHashSet::default();
            for (_, delta) in deltas {
                match delta {
                    ChunkMembershipDelta::Added(key) => {
                        if let Some(pos) = self.removed.iter().position(|&k| k == key) {
                            // The chunk is back where it was: the removal it
                            // cancels is no news, and it keeps its saved ticks.
                            self.removed.swap_remove(pos);
                            self.keys.insert(key);
                            if removed_waiting.remove(&key) {
                                self.waiting.insert(key);
                            }
                        } else if self.keys.insert(key) {
                            self.added.push(key);
                        }
                    }
                    ChunkMembershipDelta::Removed(key) => {
                        if let Some(pos) = self.added.iter().position(|&k| k == key) {
                            self.added.swap_remove(pos);
                            self.keys.remove(&key);
                            self.waiting.remove(&key);
                        } else if self.keys.remove(&key) {
                            self.removed.push(key);
                            if self.waiting.remove(&key) {
                                removed_waiting.insert(key);
                            }
                        }
                    }
                    ChunkMembershipDelta::Reset => unreachable!(),
                }
            }
            self.revision = Some(revision);
            return true;
        }

        self.keys.retain(|key| {
            if chunks.contains_key(key) {
                true
            } else {
                self.removed.push(*key);
                self.waiting.remove(key);
                false
            }
        });
        for &key in chunks.keys() {
            if self.keys.insert(key) {
                self.added.push(key);
            }
        }
        self.revision = Some(revision);
        true
    }

    /// Allows registration to borrow the owning Game without cloning its set.
    pub fn take_added(&mut self) -> Vec<Key> {
        std::mem::take(&mut self.added)
    }

    /// Retain scratch capacity after registration.
    pub fn recycle_added(&mut self, mut added: Vec<Key>) {
        added.clear();
        self.added = added;
    }

    pub fn removed(&self) -> &[Key] {
        &self.removed
    }

    /// `key` was registered with saved ticks to unpack once it ticks.
    pub fn wait(&mut self, key: Key) {
        self.waiting.insert(key);
    }

    /// The waiting chunks that `ticking` says now tick, no longer waiting.
    pub fn start_ticking(&mut self, mut ticking: impl FnMut(Key) -> bool) -> Vec<Key> {
        if self.waiting.is_empty() {
            return Vec::new();
        }
        let started: Vec<Key> = self
            .waiting
            .iter()
            .copied()
            .filter(|&k| ticking(k))
            .collect();
        for key in &started {
            self.waiting.remove(key);
        }
        started
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use std::{collections::HashSet, sync::Arc};

    fn compare(cache: &mut TickChunkMembership, old: &mut HashSet<Key>, map: &ChunkMap) {
        let loaded: HashSet<_> = map.keys().copied().collect();
        let expected_added: HashSet<_> = loaded.difference(old).copied().collect();
        let expected_removed: HashSet<_> = old.difference(&loaded).copied().collect();
        if cache.synchronize(map) {
            let added = cache.take_added();
            assert_eq!(
                added.iter().copied().collect::<HashSet<_>>(),
                expected_added
            );
            assert_eq!(
                cache.removed().iter().copied().collect::<HashSet<_>>(),
                expected_removed
            );
            cache.recycle_added(added);
        } else {
            assert!(expected_added.is_empty() && expected_removed.is_empty());
        }
        assert_eq!(cache.keys.iter().copied().collect::<HashSet<_>>(), loaded);
        *old = loaded;
    }

    #[test]
    fn matches_old_deltas_through_load_unload_replacement_and_reset() {
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        let mut cache = TickChunkMembership::default();
        let mut old = HashSet::new();
        compare(&mut cache, &mut old, &map);
        assert!(!cache.synchronize(&map));
        for key in [(-16, -8), (0, 0), (12, -1)] {
            map.insert(key, chunk.clone());
            compare(&mut cache, &mut old, &map);
        }
        map.remove(&(-16, -8));
        map.insert((-15, -8), chunk.clone()); // Same count, different key.
        compare(&mut cache, &mut old, &map);
        map.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(!cache.synchronize(&map)); // Payload replacement needs no registration.
        cache.clear();
        old.clear(); // Scheduler replaced with unchanged resident map.
        compare(&mut cache, &mut old, &map);
        map.clear();
        compare(&mut cache, &mut old, &map);
        map = ChunkMap::default();
        assert!(cache.synchronize(&map)); // Distinct empty world has distinct token.
        map.insert((-2, 4), chunk);
        compare(&mut cache, &mut old, &map);
        cache.clear();
        old.clear();
        map = ChunkMap::default();
        compare(&mut cache, &mut old, &map);
    }

    #[test]
    fn waiting_chunks_start_ticking_once_and_unloaded_ones_stop_waiting() {
        let mut map = ChunkMap::default();
        let mut cache = TickChunkMembership::default();
        for key in [(0, 0), (1, 0), (2, 0)] {
            map.insert(key, Arc::new(Chunk::empty(key.0, key.1)));
            cache.wait(key);
        }
        cache.synchronize(&map);
        assert_eq!(cache.start_ticking(|k| k == (0, 0)), vec![(0, 0)]);
        assert!(cache.start_ticking(|k| k == (0, 0)).is_empty());
        map.remove(&(1, 0));
        cache.synchronize(&map);
        let mut started = cache.start_ticking(|_| true);
        started.sort_unstable();
        assert_eq!(started, vec![(2, 0)]);
    }

    #[test]
    fn scheduler_keeps_saved_ticks_on_stable_membership_and_removes_unloaded_chunks() {
        use horizon_core::{
            position::{BlockPos, ChunkPos},
            ticks::{ChunkTicks, SavedTick, TickPriority, TickTypeId},
        };
        use horizon_world::level::Ticks;

        fn register(ticks: &mut Ticks, key: Key) {
            let pos = ChunkPos::new(key.0, key.1);
            let saved = SavedTick {
                kind: TickTypeId(1),
                pos: BlockPos::new(key.0 * 16, 0, key.1 * 16),
                delay: 5,
                priority: TickPriority::Normal,
            };
            ticks
                .add_chunk(
                    pos,
                    ChunkTicks::from_saved(vec![saved]),
                    ChunkTicks::from_saved(vec![saved]),
                )
                .unwrap();
            ticks.blocks.unpack_chunk(pos, 100);
            ticks.fluids.unpack_chunk(pos, 100);
        }
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        let mut cache = TickChunkMembership::default();
        let mut old_keys = HashSet::new();
        let mut old_ticks = Ticks::default();
        let mut new_ticks = Ticks::default();
        let keys = [(-3, -1), (2, 4), (-2, -1)];
        for step in 0..7 {
            match step {
                0 => {
                    map.insert(keys[0], chunk.clone());
                    map.insert(keys[1], chunk.clone());
                }
                2 => {
                    map.remove(&keys[0]);
                    map.insert(keys[2], chunk.clone());
                }
                3 => {
                    map.insert(keys[1], chunk.clone());
                }
                4 => {
                    // Reset only the scheduler: resident token is unchanged.
                    new_ticks = Ticks::default();
                    old_ticks = Ticks::default();
                    cache.clear();
                    old_keys.clear();
                }
                5 => {
                    map.clear();
                }
                6 => {
                    map = ChunkMap::default();
                    map.insert(keys[0], chunk.clone());
                    new_ticks = Ticks::default();
                    old_ticks = Ticks::default();
                    cache.clear();
                    old_keys.clear();
                }
                _ => {}
            }
            let loaded: HashSet<_> = map.keys().copied().collect();
            for &key in loaded.difference(&old_keys) {
                register(&mut old_ticks, key);
            }
            for &key in old_keys.difference(&loaded) {
                old_ticks.remove_chunk(ChunkPos::new(key.0, key.1));
            }
            old_keys = loaded;
            if cache.synchronize(&map) {
                let added = cache.take_added();
                for &key in &added {
                    register(&mut new_ticks, key);
                }
                for &key in cache.removed() {
                    new_ticks.remove_chunk(ChunkPos::new(key.0, key.1));
                }
                cache.recycle_added(added);
            }
            for key in keys {
                let pos = ChunkPos::new(key.0, key.1);
                for (old, new) in [
                    (&old_ticks.blocks, &new_ticks.blocks),
                    (&old_ticks.fluids, &new_ticks.fluids),
                ] {
                    assert_eq!(
                        old.chunk(pos).map(|c| c.queued()),
                        new.chunk(pos).map(|c| c.queued())
                    );
                    assert_eq!(new.chunk(pos).is_some(), map.contains_key(&key));
                }
            }
        }
    }

    /// A removal the next addition cancels puts the chunk back where it was,
    /// saved ticks included: the delta replay answers as the full scan does.
    #[test]
    fn cancel_removal_between_synchronizations_restores_membership_and_waiting() {
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        let mut cache = TickChunkMembership::default();
        let key = (3, 7);

        map.insert(key, chunk.clone());
        assert!(cache.synchronize(&map));
        assert_eq!(cache.take_added(), vec![key]);
        cache.wait(key);
        assert!(cache.waiting.contains(&key));

        map.remove(&key);
        map.insert(key, chunk);

        assert!(cache.synchronize(&map));
        assert!(cache.take_added().is_empty());
        assert!(cache.removed().is_empty());
        assert!(cache.keys.contains(&key));
        assert!(cache.waiting.contains(&key));
        assert_eq!(cache.start_ticking(|k| k == key), vec![key]);

        // The unload after that one is still news.
        map.remove(&key);
        assert!(cache.synchronize(&map));
        assert_eq!(cache.removed(), &[key]);
        assert!(!cache.keys.contains(&key));
    }
}
