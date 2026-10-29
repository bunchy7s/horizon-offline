//! The points of interest of each resident section (`PoiSection`), so a
//! search visits the few POI blocks of the sections in range instead of
//! reading every block of them.
//!
//! Vanilla keeps its records up to date from `onBlockStateChange`; here a
//! section's list is rebuilt from its blocks after any edit. Resident
//! sections are immutable behind `Arc` and edited through `Arc::make_mut`, so
//! a weak reference tells an edited or replaced section from the one the list
//! was built from (see `random_tick_sections`), without keeping its blocks.
use crate::{chunk::Section, fast_hash::FxHashMap};
use std::sync::{Arc, Mutex, Weak};

/// A section's POIs: the block index in the section (`y << 8 | z << 4 | x`)
/// and its kind, in ascending index order.
pub type SectionPois<T> = Arc<[(u16, T)]>;

struct Cached<T> {
    section: Weak<Section>,
    pois: SectionPois<T>,
}
/// Cached POI lists by section, `(chunk x, section index, chunk z)`.
pub struct PoiSections<T> {
    sections: Mutex<Lists<T>>,
}
struct Lists<T> {
    by_section: FxHashMap<(i32, i32, i32), Cached<T>>,
    /// Size above which lists of sections no longer resident are dropped.
    prune_at: usize,
}
const MIN_PRUNE: usize = 4096;
impl<T> Default for PoiSections<T> {
    fn default() -> Self {
        Self {
            sections: Mutex::new(Lists {
                by_section: FxHashMap::default(),
                prune_at: MIN_PRUNE,
            }),
        }
    }
}
impl<T: Copy> PoiSections<T> {
    /// The POIs of `section`, whose block states `kind` classifies. `kind`
    /// must give the same answer for a state on every call.
    pub fn get(
        &self,
        key: (i32, i32, i32),
        section: &Arc<Section>,
        mut kind: impl FnMut(u32) -> Option<T>,
    ) -> SectionPois<T> {
        let mut lists = self.sections.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = lists.by_section.get(&key)
            && cached.section.as_ptr() == Arc::as_ptr(section)
        {
            return cached.pois.clone();
        }
        let pois: SectionPois<T> = if section.blocks.maybe_has(|v| kind(v).is_some()) {
            let mut states = [0u32; 4096];
            section.unpack(&mut states);
            states
                .iter()
                .enumerate()
                .filter_map(|(i, &state)| kind(state).map(|poi| (i as u16, poi)))
                .collect()
        } else {
            Arc::from([])
        };
        if lists.by_section.len() >= lists.prune_at {
            lists.by_section.retain(|_, c| c.section.strong_count() > 0);
            lists.prune_at = (lists.by_section.len() * 2).max(MIN_PRUNE);
        }
        lists.by_section.insert(
            key,
            Cached {
                section: Arc::downgrade(section),
                pois: pois.clone(),
            },
        );
        pois
    }
}

/// The POI types villagers use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poi {
    /// A job site, by profession index.
    Job(u8),
    Home,
    Meeting,
}
impl Poi {
    /// `PoiType.maxTickets`.
    pub fn max_tickets(self) -> usize {
        match self {
            Poi::Meeting => 32,
            _ => 1,
        }
    }
    /// `PoiType.validRange`.
    pub fn valid_range(self) -> i32 {
        match self {
            Poi::Meeting => 6,
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{Chunk, MIN_Y};

    #[test]
    fn lists_follow_edits_and_replacement() {
        let index = PoiSections::<u8>::default();
        let kind = |v: u32| (v == 5 || v == 6).then_some(v as u8);
        let mut chunk = Arc::new(Chunk::empty(0, 0));
        assert!(index.get((0, 0, 0), &chunk.sections[0], kind).is_empty());
        Arc::make_mut(&mut chunk).set(3, MIN_Y + 2, 1, 6);
        Arc::make_mut(&mut chunk).set(0, MIN_Y, 0, 5);
        let found = index.get((0, 0, 0), &chunk.sections[0], kind);
        assert_eq!(&*found, &[(0, 5), (2 << 8 | 1 << 4 | 3, 6)]);
        // Unchanged: the cached list, not a rescan.
        let again = index.get((0, 0, 0), &chunk.sections[0], |_| panic!("rescanned"));
        assert!(Arc::ptr_eq(&found, &again));
        // An edit in place (no other owner) still invalidates the list.
        Arc::make_mut(&mut chunk).set(0, MIN_Y, 0, 1);
        assert_eq!(
            &*index.get((0, 0, 0), &chunk.sections[0], kind),
            &[(2 << 8 | 1 << 4 | 3, 6)]
        );
        // So does replacing the section, or the chunk.
        Arc::make_mut(&mut chunk).sections[0] = Arc::new(Section::default());
        assert!(index.get((0, 0, 0), &chunk.sections[0], kind).is_empty());
        let mut other = Chunk::empty(0, 0);
        other.set(1, MIN_Y, 1, 5);
        chunk = Arc::new(other);
        assert_eq!(
            &*index.get((0, 0, 0), &chunk.sections[0], kind),
            &[(1 << 4 | 1, 5)]
        );
        // An unused palette entry alone yields no POI.
        Arc::make_mut(&mut chunk).set(1, MIN_Y, 1, 2);
        assert!(index.get((0, 0, 0), &chunk.sections[0], kind).is_empty());
    }

    #[test]
    fn unloaded_sections_are_dropped() {
        let index = PoiSections::<u8>::default();
        let kind = |v: u32| (v == 5).then_some(5);
        let kept = Arc::new(Section::default());
        index.get((0, 0, 0), &kept, kind);
        for i in 0..MIN_PRUNE as i32 {
            index.get((1, i, 0), &Arc::new(Section::default()), kind);
        }
        let lists = index.sections.lock().unwrap();
        assert!(lists.by_section.len() <= 2);
        assert!(lists.by_section.contains_key(&(0, 0, 0)));
    }
}
