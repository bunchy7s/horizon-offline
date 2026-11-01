//! Resident chunk membership, with a token for consumers that cache its keys.
use crate::{
    chunk::Chunk,
    fast_hash::{FxHashMap, FxHashSet},
};
use std::{
    collections::{
        VecDeque,
        hash_map::{Iter, ValuesMut},
    },
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

type Key = (i32, i32);
type Columns = FxHashMap<Key, Arc<Chunk>>;
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);
fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// The membership changes one map keeps for the consumers that read them
/// every tick (`TickChunkMembership.synchronize`); older ones are dropped.
const MAX_RECENT_DELTAS: usize = 256;

/// One membership change of one `ChunkMap`, stamped with the revision it
/// brought the map to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkMembershipDelta {
    Added(Key),
    Removed(Key),
    Reset,
}

/// Read access behaves like a map. Structural writes are explicit so edits to
/// blocks or light don't invalidate consumers of the loaded chunk key set.
/// Tokens are process-wide: replacing the entire map also invalidates caches.
pub struct ChunkMap {
    columns: Columns,
    revision: u64,
    /// The oldest revision `recent_deltas` still answers for: the revision of
    /// the change it last dropped, or the revision that cleared them. A
    /// consumer holding an older one rebuilds, it does not replay.
    min_valid_revision: u64,
    /// This map's recent membership changes, in revision order.
    /// Revisions are process-wide: the numbers missing in between belong to
    /// the other maps (a server thread's, another dimension's), which say
    /// nothing about this one's keys.
    recent_deltas: VecDeque<(u64, ChunkMembershipDelta)>,
    /// The resident chunks generation has not placed its animals in yet.
    fresh: FxHashSet<Key>,
}
impl Default for ChunkMap {
    fn default() -> Self {
        let revision = next_revision();
        Self {
            columns: Columns::default(),
            revision,
            min_valid_revision: revision,
            recent_deltas: VecDeque::new(),
            fresh: FxHashSet::default(),
        }
    }
}
impl ChunkMap {
    pub fn membership_revision(&self) -> u64 {
        self.revision
    }
    fn record_delta(&mut self, delta: ChunkMembershipDelta) {
        if self.recent_deltas.len() >= MAX_RECENT_DELTAS
            && let Some((evicted, _)) = self.recent_deltas.pop_front()
        {
            self.min_valid_revision = evicted;
        }
        self.recent_deltas.push_back((self.revision, delta));
    }
    /// This map's membership changes since `last_revision`, one of its own
    /// earlier `membership_revision`s, or `None` when they can no longer be
    /// answered: the consumer's revision predates what the ring kept, the map
    /// was cleared since (a reset rebuilds everything), or the revision is not
    /// this map's, which its own revisions give away: every one of them in the
    /// answered range is the opening revision or a change the ring still
    /// holds, so a revision another map took in between is refused, not
    /// replayed.
    pub fn deltas_since(
        &self,
        last_revision: u64,
    ) -> Option<impl Iterator<Item = (u64, ChunkMembershipDelta)> + '_> {
        if last_revision == 0
            || last_revision < self.min_valid_revision
            || last_revision > self.revision
            || self.recent_deltas.is_empty()
        {
            return None;
        }
        let pos = self
            .recent_deltas
            .partition_point(|(rev, _)| *rev <= last_revision);
        // A revision this map never took: another map's, drawing on the same
        // process-wide counter. Only the opening revision and the ones the
        // ring still holds are this map's.
        let is_own_revision = last_revision == self.min_valid_revision
            || (pos > 0 && self.recent_deltas[pos - 1].0 == last_revision);
        if !is_own_revision {
            return None;
        }
        let slice = self.recent_deltas.range(pos..);
        if slice
            .clone()
            .any(|(_, d)| *d == ChunkMembershipDelta::Reset)
        {
            return None;
        }
        Some(slice.copied())
    }
    pub fn insert(&mut self, key: Key, chunk: Arc<Chunk>) -> Option<Arc<Chunk>> {
        if chunk.fresh {
            self.fresh.insert(key);
        } else {
            self.fresh.remove(&key);
        }
        let previous = self.columns.insert(key, chunk);
        if previous.is_none() {
            self.revision = next_revision();
            self.record_delta(ChunkMembershipDelta::Added(key));
        }
        previous
    }
    pub fn remove(&mut self, key: &Key) -> Option<Arc<Chunk>> {
        self.fresh.remove(key);
        let previous = self.columns.remove(key);
        if previous.is_some() {
            self.revision = next_revision();
            self.record_delta(ChunkMembershipDelta::Removed(*key));
        }
        previous
    }
    pub fn clear(&mut self) {
        if !self.columns.is_empty() {
            self.columns.clear();
            self.fresh.clear();
            self.revision = next_revision();
            self.recent_deltas.clear();
            self.recent_deltas
                .push_back((self.revision, ChunkMembershipDelta::Reset));
            self.min_valid_revision = self.revision;
        }
    }
    pub fn has_fresh(&self) -> bool {
        !self.fresh.is_empty()
    }
    /// The fresh keys in the chunk map's own iteration order, so the spawns
    /// they drive run in the order a scan of the columns gave.
    pub fn fresh_keys_ordered(&self) -> Vec<Key> {
        if self.fresh.is_empty() {
            Vec::new()
        } else if self.fresh.len() == 1 {
            self.fresh.iter().copied().collect()
        } else {
            self.columns
                .keys()
                .filter(|k| self.fresh.contains(k))
                .copied()
                .collect()
        }
    }
    pub fn remove_fresh(&mut self, key: &Key) {
        self.fresh.remove(key);
    }
    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Arc<Chunk>> {
        self.columns.get_mut(key)
    }
    pub fn values_mut(&mut self) -> ValuesMut<'_, Key, Arc<Chunk>> {
        self.columns.values_mut()
    }
}
impl Deref for ChunkMap {
    type Target = Columns;
    fn deref(&self) -> &Columns {
        &self.columns
    }
}
impl<'a> IntoIterator for &'a ChunkMap {
    type Item = (&'a Key, &'a Arc<Chunk>);
    type IntoIter = Iter<'a, Key, Arc<Chunk>>;
    fn into_iter(self) -> Self::IntoIter {
        self.columns.iter()
    }
}
impl crate::light::LightWorld for ChunkMap {
    fn section(&self, cx: i32, s: usize, cz: i32) -> Option<&Arc<crate::chunk::Section>> {
        self.columns.section(cx, s, cz)
    }
    fn set_section_light(&mut self, cx: i32, s: usize, cz: i32, light: crate::chunk::Light) {
        self.columns.set_section_light(cx, s, cz, light);
    }
    fn layout(&self) -> crate::chunk::ColumnLayout {
        self.columns.layout()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn membership_tokens_track_keys_and_map_replacement_only() {
        let mut map = ChunkMap::default();
        let empty = map.membership_revision();
        map.clear();
        map.remove(&(0, 0));
        assert_eq!(empty, map.membership_revision());
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let inserted = map.membership_revision();
        assert_ne!(empty, inserted);
        map.insert((0, 0), chunk.clone());
        Arc::make_mut(map.get_mut(&(0, 0)).unwrap()).lit = true;
        for column in map.values_mut() {
            Arc::make_mut(column).light_loaded = true;
        }
        crate::light::LightWorld::set_section_light(
            &mut map,
            0,
            0,
            0,
            crate::chunk::Light::Uniform(0),
        );
        assert_eq!(inserted, map.membership_revision());
        map.remove(&(0, 0));
        map.insert((1, 0), chunk);
        assert_eq!(map.len(), 1);
        assert_ne!(inserted, map.membership_revision());
        let replaced = map.membership_revision();
        map.clear();
        assert_ne!(replaced, map.membership_revision());
        let cleared = map.membership_revision();
        map = ChunkMap::default();
        assert_ne!(cleared, map.membership_revision());
    }

    /// The deltas' kinds, checking the ring answers in revision order.
    fn kinds(deltas: Vec<(u64, ChunkMembershipDelta)>) -> Vec<ChunkMembershipDelta> {
        let mut last = 0;
        for (rev, _) in &deltas {
            assert!(*rev > last, "the ring answers in revision order");
            last = *rev;
        }
        deltas.into_iter().map(|(_, d)| d).collect()
    }

    /// Revision numbers are process-wide: the ones another map takes in
    /// between, and a payload-only rewrite that takes none at all, say nothing
    /// about this map's keys.
    #[test]
    fn deltas_since_ignores_revisions_other_maps_take() {
        let mut map = ChunkMap::default();
        let mut other = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let seen = map.membership_revision();
        for key in [(1, 0), (2, 0), (3, 0)] {
            other.insert(key, chunk.clone());
        }
        map.insert((0, 0), Arc::new(Chunk::empty(0, 0))); // Payload only.
        map.insert((4, 0), chunk.clone());
        map.remove(&(0, 0));
        assert_eq!(
            kinds(map.deltas_since(seen).unwrap().collect()),
            [
                ChunkMembershipDelta::Added((4, 0)),
                ChunkMembershipDelta::Removed((0, 0)),
            ]
        );
        // The consumer's own revision answers, with nothing new since.
        assert!(
            map.deltas_since(map.membership_revision())
                .unwrap()
                .next()
                .is_none()
        );
        // A revision another map took in between: refused, not replayed.
        assert!(map.deltas_since(other.membership_revision()).is_none());
    }

    /// A map built ahead of the live one, its revisions interleaved with it,
    /// answering for a revision it never took: refused, so the consumer
    /// rebuilds rather than replay another map's changes.
    #[test]
    fn deltas_since_none_for_interleaved_revision_from_another_map() {
        let mut map = ChunkMap::default();
        let mut other = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let seen = map.membership_revision();
        other.insert((1, 0), chunk.clone());
        let foreign = other.membership_revision();
        map.insert((2, 0), chunk.clone());
        assert!(map.deltas_since(foreign).is_none());
        assert_eq!(
            kinds(map.deltas_since(seen).unwrap().collect()),
            [ChunkMembershipDelta::Added((2, 0))]
        );
    }

    /// The ring answers for the changes it kept, and only those.
    #[test]
    fn deltas_since_none_for_a_revision_the_ring_dropped() {
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        let created = map.membership_revision();
        let mut revisions = Vec::new();
        let changes = MAX_RECENT_DELTAS as i32 + 1;
        for i in 0..changes {
            map.insert((i, 0), chunk.clone());
            revisions.push(map.membership_revision());
        }
        let (first, kept) = revisions.split_first().unwrap();
        // The oldest change fell out with the consumer's revision: rebuild.
        assert!(map.deltas_since(created).is_none());
        assert_eq!(map.deltas_since(*first).unwrap().count(), kept.len());
    }

    /// A reset rebuilds whatever the consumer last saw, and the ring answers
    /// for what follows it.
    #[test]
    fn deltas_since_none_across_a_clear() {
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let before = map.membership_revision();
        map.clear();
        assert!(map.deltas_since(before).is_none());
        let cleared = map.membership_revision();
        map.insert((1, 0), chunk);
        assert_eq!(
            kinds(map.deltas_since(cleared).unwrap().collect()),
            [ChunkMembershipDelta::Added((1, 0))]
        );
    }

    #[test]
    fn fresh_chunks_tracking() {
        let mut map = ChunkMap::default();
        assert!(!map.has_fresh());
        let mut chunk = Chunk::empty(1, 2);
        chunk.fresh = true;
        map.insert((1, 2), Arc::new(chunk));
        assert!(map.has_fresh());
        assert_eq!(map.fresh_keys_ordered(), vec![(1, 2)]);

        map.remove_fresh(&(1, 2));
        assert!(!map.has_fresh());

        let mut chunk2 = Chunk::empty(3, 4);
        chunk2.fresh = true;
        map.insert((3, 4), Arc::new(chunk2));
        assert!(map.has_fresh());

        map.remove(&(3, 4));
        assert!(!map.has_fresh());
    }

    #[test]
    fn fresh_keys_ordered_matches_chunk_map_iteration_order() {
        let mut map = ChunkMap::default();
        let keys = [(-5, 10), (12, -3), (0, 0), (7, 8), (-1, -1), (4, 4)];
        for (i, &key) in keys.iter().enumerate() {
            let mut chunk = Chunk::empty(key.0, key.1);
            chunk.fresh = i % 2 == 0;
            map.insert(key, Arc::new(chunk));
        }

        let ordered = map.fresh_keys_ordered();
        let expected: Vec<Key> = map
            .keys()
            .copied()
            .filter(|k| map.fresh.contains(k))
            .collect();
        assert_eq!(ordered, expected);
        assert_eq!(ordered.len(), 3);
    }
}
