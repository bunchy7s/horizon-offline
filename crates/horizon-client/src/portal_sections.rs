//! Memoized nether-portal palette eligibility per resident chunk.
//!
//! `Game::find_entity_portals` asks the same sections every tick whether their
//! palette may hold a portal state. The answer only changes when a chunk's
//! blocks change, so it is kept per chunk until the weak handle stops matching.
use crate::{chunk::Chunk, fast_hash::FxHashMap, section_mask::SectionMask};
use std::sync::{Arc, Weak};

struct Cached {
    chunk: Weak<Chunk>,
    /// The sections whose palette may hold a portal.
    sections: SectionMask,
}

/// Resident chunks are immutable behind `Arc`; edits use `Arc::make_mut`.
/// A weak reference detects both edits and replacement, without retaining block
/// data or forcing a deep copy on the next edit. It also prevents address reuse.
/// Clear this cache whenever the gameplay facts change (including resource reload).
#[derive(Default)]
pub struct PortalSections {
    chunks: FxHashMap<(i32, i32), Cached>,
}

impl PortalSections {
    pub fn clear(&mut self) {
        self.chunks.clear();
    }
    pub fn remove(&mut self, key: &(i32, i32)) {
        self.chunks.remove(key);
    }
    /// The sections of `chunk` whose palette may hold a portal, by index
    /// (`MIN_Y`'s section is 0). `portal` is only called while
    /// (re)building an entry, so an unchanged chunk scans its palette once.
    pub fn get(
        &mut self,
        key: (i32, i32),
        chunk: &Arc<Chunk>,
        mut portal: impl FnMut(u32) -> bool,
    ) -> SectionMask {
        let cached = self.chunks.entry(key).or_insert_with(|| Cached {
            chunk: Weak::new(),
            sections: SectionMask::EMPTY,
        });
        if cached.chunk.as_ptr() != Arc::as_ptr(chunk) {
            // Preserve maybe_has, including unused entries and global palettes.
            cached.sections = chunk
                .sections
                .iter()
                .enumerate()
                .filter(|(_, section)| section.blocks.maybe_has(&mut portal))
                .map(|(i, _)| i)
                .collect();
            cached.chunk = Arc::downgrade(chunk);
        }
        cached.sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{MIN_Y, SECTIONS, Section};

    #[test]
    fn edits_replacements_compaction_and_reload() {
        let mut cache = PortalSections::default();
        let mut chunk = Arc::new(Chunk::empty(0, 0));
        assert_eq!(cache.get((0, 0), &chunk, |id| id == 7), SectionMask::EMPTY);
        cache.get((0, 0), &chunk, |_| panic!("unchanged chunk rescanned"));
        let worker = chunk.clone();
        Arc::make_mut(&mut chunk).set(0, MIN_Y, 0, 7);
        assert_eq!(
            cache.get((0, 0), &chunk, |id| id == 7),
            SectionMask::single(0)
        );
        assert_eq!(worker.get(0, MIN_Y, 0), 0);
        // Even after the last portal block is removed, its unused palette entry counts.
        Arc::make_mut(&mut chunk).set(0, MIN_Y, 0, 0);
        assert_eq!(
            cache.get((0, 0), &chunk, |id| id == 7),
            SectionMask::single(0)
        );
        Arc::make_mut(&mut Arc::make_mut(&mut chunk).sections[0]).compact();
        assert_eq!(cache.get((0, 0), &chunk, |id| id == 7), SectionMask::EMPTY);
        Arc::make_mut(&mut chunk).sections[SECTIONS - 1] = Arc::new(Section::default());
        cache.clear(); // resource reload, with changed facts for the same state IDs
        assert_eq!(
            cache.get((0, 0), &chunk, |id| id == 0),
            SectionMask::below(SECTIONS),
            "a fresh palette is rescanned for every section"
        );
        cache.remove(&(0, 0));
        assert!(cache.chunks.is_empty());
    }

    #[test]
    fn replacement_at_same_coordinates_and_early_snapshot() {
        let mut cache = PortalSections::default();
        let mut chunk = Arc::new(Chunk::empty(-2, 3));
        let before = cache.get((-2, 3), &chunk, |id| id == 1);
        Arc::make_mut(&mut chunk).set(0, MIN_Y + 16, 0, 1);
        assert_eq!(before, SectionMask::EMPTY);
        assert_eq!(
            cache.get((-2, 3), &chunk, |id| id == 1),
            SectionMask::single(1)
        );
        let weak = Arc::downgrade(&chunk);
        drop(chunk);
        assert!(weak.upgrade().is_none()); // cache does not retain unloaded block data
        let replacement = Arc::new(Chunk::empty(-2, 3));
        assert_eq!(
            cache.get((-2, 3), &replacement, |id| id == 1),
            SectionMask::EMPTY
        );
    }

    #[test]
    fn global_palette_covers_the_whole_id_range() {
        let mut chunk = Chunk::empty(0, 0);
        for i in [0, 9, SECTIONS - 1] {
            Arc::make_mut(&mut chunk.sections[i]).blocks =
                Section::from_blocks(&std::array::from_fn(|j| (j % 300) as u32));
        }
        let chunk = Arc::new(chunk);
        let mut cache = PortalSections::default();
        // Global maybe_has considers the entire ID range, including absent states.
        assert_eq!(
            cache.get((0, 0), &chunk, |id| id == 40000),
            SectionMask::from_iter([0, 9, SECTIONS - 1])
        );
    }
}
