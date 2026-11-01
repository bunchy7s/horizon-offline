//! Which sections of the game thread's chunks random-tick
//! (`LevelChunkSection.isRandomlyTicking`), memoized per section.
use crate::{
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y, Section},
    fast_hash::FxHashMap,
    section_mask::SectionMask,
};
use horizon_world::{level::RandomTickMask, registry::BlockStateId};
use std::sync::{Arc, Weak};

/// A value snapshot, in ascending section order, with no per-chunk allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sections {
    mask: SectionMask,
    /// The minimum Y of section index 0 (the chunk's).
    base: i32,
}
impl Default for Sections {
    fn default() -> Self {
        Self {
            mask: SectionMask::EMPTY,
            base: MIN_Y,
        }
    }
}
impl Sections {
    /// The minimum Y of the lowest section in the set from block `min_y` up.
    pub fn from(self, min_y: i32) -> Option<i32> {
        let index = ((min_y - self.base) >> 4).max(0) as usize;
        Sections {
            mask: self.mask.from(index),
            ..self
        }
        .next()
    }
}
impl Iterator for Sections {
    type Item = i32;
    fn next(&mut self) -> Option<i32> {
        let index = self.mask.iter().next()?;
        self.mask.remove(index);
        Some(self.base + index as i32 * 16)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.mask.len();
        (len, Some(len))
    }
}
impl ExactSizeIterator for Sections {}

struct Cached {
    chunk: Weak<Chunk>,
    /// The section each bit of `sections` was read from.
    read: Vec<Weak<Section>>,
    sections: Sections,
    /// Each ticking section's [`RandomTickMask`], built at its first draw
    /// and dropped when the section is read again.
    masks: Vec<Option<Arc<RandomTickMask>>>,
}
/// Vanilla keeps a count of randomly ticking blocks and fluids per section,
/// updated by every `setBlockState`, and ticks the sections where it is above
/// zero. Here a section's answer is read from its blocks after any edit:
/// resident chunks and sections are immutable behind `Arc` and edited through
/// `Arc::make_mut`, so a weak reference tells an edited or replaced one from
/// the one read, without retaining block data or forcing a deep copy on the
/// next edit, and prevents address reuse. An unchanged chunk costs one
/// pointer comparison; a changed one, a comparison per section and a read of
/// the sections that changed. Clear this cache whenever the gameplay facts
/// change (including resource reload).
#[derive(Default)]
pub struct RandomTickSections {
    chunks: FxHashMap<(i32, i32), Cached>,
}
impl RandomTickSections {
    pub fn clear(&mut self) {
        self.chunks.clear();
    }
    pub fn remove(&mut self, key: &(i32, i32)) {
        self.chunks.remove(key);
    }
    /// The sections of `chunk` holding a randomly ticking block or fluid.
    pub fn get(&mut self, key: (i32, i32), chunk: &Arc<Chunk>, facts: &BlockFacts) -> Sections {
        self.get_with(key, chunk, |id| facts.get(BlockStateId(id)).random_ticks)
    }
    fn get_with(
        &mut self,
        key: (i32, i32),
        chunk: &Arc<Chunk>,
        mut random: impl FnMut(u32) -> bool,
    ) -> Sections {
        Self::refresh(&mut self.chunks, key, chunk, &mut random).sections
    }
    /// The ticking sections of `chunk` ([`Self::get`]) into `out`, bottom up:
    /// each one's minimum Y and its mask of the blocks a random tick can do
    /// anything for (`candidate`: `Behaviours::random_tick_candidate`), built
    /// at the section's first tick and kept until it changes.
    pub fn masked(
        &mut self,
        key: (i32, i32),
        chunk: &Arc<Chunk>,
        facts: &BlockFacts,
        mut candidate: impl FnMut(u32) -> bool,
        out: &mut Vec<(i32, Arc<RandomTickMask>)>,
    ) {
        out.clear();
        let cached = Self::refresh(&mut self.chunks, key, chunk, &mut |id| {
            facts.get(BlockStateId(id)).random_ticks
        });
        for y in cached.sections {
            let index = ((y - cached.sections.base) >> 4) as usize;
            let mask = cached.masks[index].get_or_insert_with(|| {
                let mut blocks = [0u32; 4096];
                chunk.sections[index].blocks.unpack(&mut blocks);
                Arc::new(RandomTickMask::of(&blocks, &mut candidate))
            });
            out.push((y, mask.clone()));
        }
    }
    fn refresh<'a>(
        chunks: &'a mut FxHashMap<(i32, i32), Cached>,
        key: (i32, i32),
        chunk: &Arc<Chunk>,
        random: &mut impl FnMut(u32) -> bool,
    ) -> &'a mut Cached {
        let cached = chunks.entry(key).or_insert_with(|| Cached {
            chunk: Weak::new(),
            read: Vec::new(),
            sections: Sections::default(),
            masks: Vec::new(),
        });
        if cached.chunk.as_ptr() != Arc::as_ptr(chunk) {
            if cached.read.len() != chunk.sections.len() || cached.sections.base != chunk.min_y() {
                cached.read = chunk.sections.iter().map(|_| Weak::new()).collect();
                cached.masks = vec![None; chunk.sections.len()];
                cached.sections = Sections {
                    mask: SectionMask::EMPTY,
                    base: chunk.min_y(),
                };
            }
            for (i, (section, read)) in chunk.sections.iter().zip(&mut cached.read).enumerate() {
                if read.as_ptr() == Arc::as_ptr(section) {
                    continue;
                }
                // `isRandomlyTicking`: the blocks themselves, not the palette,
                // which keeps the states no block uses any more.
                cached.masks[i] = None;
                if section.blocks.has(&mut *random) {
                    cached.sections.mask.insert(i);
                } else {
                    cached.sections.mask.remove(i);
                }
                *read = Arc::downgrade(section);
            }
            cached.chunk = Arc::downgrade(chunk);
        }
        cached
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::SECTIONS;
    use crate::chunk::Section;

    #[test]
    fn a_tall_chunk_ticks_its_sections_near_the_top() {
        let layout = crate::chunk::ColumnLayout::new(-64, 2096).unwrap();
        let mut cache = RandomTickSections::default();
        let mut chunk = Arc::new(Chunk::with_layout(0, 0, layout));
        Arc::make_mut(&mut chunk).set(0, 2020, 0, 1);
        Arc::make_mut(&mut chunk).set(0, 100, 0, 1);
        let sections = cache.get_with((0, 0), &chunk, |id| id == 1);
        assert_eq!(sections.collect::<Vec<_>>(), [96, 2016]);
        assert_eq!(sections.from(200), Some(2016));
        // A vanilla chunk at the same key is read again.
        let short = Arc::new(Chunk::empty(0, 0));
        assert_eq!(cache.get_with((0, 0), &short, |id| id == 1).len(), 0);
    }
    #[test]
    fn edits_replacements_compaction_and_reload() {
        let mut cache = RandomTickSections::default();
        let mut chunk = Arc::new(Chunk::empty(0, 0));
        assert_eq!(cache.get_with((0, 0), &chunk, |id| id == 1).len(), 0);
        cache.get_with((0, 0), &chunk, |_| panic!("unchanged chunk rescanned"));
        let worker = chunk.clone();
        Arc::make_mut(&mut chunk).set(0, MIN_Y, 0, 1);
        assert_eq!(
            cache
                .get_with((0, 0), &chunk, |id| id == 1)
                .collect::<Vec<_>>(),
            [MIN_Y]
        );
        assert_eq!(worker.get(0, MIN_Y, 0), 0);
        // Once the last ticking block is gone the section stops ticking, as
        // vanilla's count drops to zero, though its palette still lists it.
        Arc::make_mut(&mut chunk).set(0, MIN_Y, 0, 0);
        assert!(chunk.sections[0].blocks.maybe_has(|id| id == 1));
        assert_eq!(cache.get_with((0, 0), &chunk, |id| id == 1).len(), 0);
        // Only the edited section is read again (its palette: air, then 1).
        Arc::make_mut(&mut chunk).set(0, MIN_Y + 16, 0, 1);
        let mut asked = Vec::new();
        let sections = cache.get_with((0, 0), &chunk, |id| {
            asked.push(id);
            id == 1
        });
        assert_eq!(
            (sections.collect::<Vec<_>>(), asked),
            (vec![MIN_Y + 16], vec![0, 1])
        );
        Arc::make_mut(&mut chunk).set(0, MIN_Y + 16, 0, 0);
        Arc::make_mut(&mut Arc::make_mut(&mut chunk).sections[1]).compact();
        assert_eq!(cache.get_with((0, 0), &chunk, |id| id == 1).len(), 0);
        Arc::make_mut(&mut chunk).sections[SECTIONS - 1] = Arc::new(Section::default());
        cache.clear(); // resource reload, with changed facts for the same state IDs
        assert_eq!(cache.get_with((0, 0), &chunk, |id| id == 0).len(), SECTIONS);
        cache.remove(&(0, 0));
        assert!(cache.chunks.is_empty());
    }

    #[test]
    fn replacement_at_same_coordinates_and_early_snapshot() {
        let mut cache = RandomTickSections::default();
        let mut chunk = Arc::new(Chunk::empty(-2, 3));
        let before_scheduled = cache.get_with((-2, 3), &chunk, |id| id == 1);
        Arc::make_mut(&mut chunk).set(0, MIN_Y + 16, 0, 1);
        assert_eq!(before_scheduled.len(), 0);
        assert_eq!(
            cache
                .get_with((-2, 3), &chunk, |id| id == 1)
                .collect::<Vec<_>>(),
            [MIN_Y + 16]
        );
        let weak = Arc::downgrade(&chunk);
        drop(chunk);
        assert!(weak.upgrade().is_none()); // cache does not retain unloaded block data
        let replacement = Arc::new(Chunk::empty(-2, 3));
        assert_eq!(cache.get_with((-2, 3), &replacement, |id| id == 1).len(), 0);
    }

    #[test]
    fn global_palette_and_ascending_order() {
        let mut chunk = Chunk::empty(0, 0);
        for i in [0, 9, SECTIONS - 1] {
            Arc::make_mut(&mut chunk.sections[i]).blocks =
                Section::from_blocks(&std::array::from_fn(|j| (j % 300) as u32));
        }
        let chunk = Arc::new(chunk);
        let mut cache = RandomTickSections::default();
        // Global palettes: the states the blocks hold, not the whole ID range.
        assert_eq!(cache.get_with((0, 0), &chunk, |id| id == 40000).len(), 0);
        cache.clear();
        assert_eq!(
            cache
                .get_with((0, 0), &chunk, |id| id == 299)
                .collect::<Vec<_>>(),
            [MIN_Y, MIN_Y + 9 * 16, MIN_Y + (SECTIONS as i32 - 1) * 16]
        );
        let sections = cache.get_with((0, 0), &chunk, |id| id == 299);
        let top = MIN_Y + (SECTIONS as i32 - 1) * 16;
        let from = |y| sections.from(y);
        assert_eq!(
            [
                from(MIN_Y - 16),
                from(MIN_Y),
                from(MIN_Y + 16),
                from(MIN_Y + 9 * 16)
            ],
            [
                Some(MIN_Y),
                Some(MIN_Y),
                Some(MIN_Y + 9 * 16),
                Some(MIN_Y + 9 * 16)
            ]
        );
        assert_eq!(
            [from(top), from(top + 16), from(top + 1000)],
            [Some(top), None, None]
        );
    }
}

#[cfg(test)]
mod tick_parity {
    use super::*;
    use crate::chunk::SECTIONS;
    use horizon_core::{
        position::{BlockPos, ChunkPos},
        random::{LegacyRandom, RandomSource},
    };
    use horizon_world::level::{LevelAccess, LevelEvent, Ticks, data};
    use std::{cell::RefCell, path::PathBuf};

    #[derive(Clone)]
    struct Level {
        chunks: FxHashMap<(i32, i32), Arc<Chunk>>,
        random: LegacyRandom,
        reads: RefCell<Vec<BlockPos>>,
        events: Vec<LevelEvent>,
        time: i64,
        speed: i32,
    }
    impl LevelAccess for Level {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.reads.borrow_mut().push(pos);
            self.chunks
                .get(&(pos.x >> 4, pos.z >> 4))
                .map_or(BlockStateId(0), |c| c.block(pos.x, pos.y, pos.z))
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            let chunk = self.chunks.get_mut(&(pos.x >> 4, pos.z >> 4))?;
            Some(BlockStateId(
                Arc::make_mut(chunk).set(pos.x, pos.y, pos.z, state.0),
            ))
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.chunks.contains_key(&(pos.x >> 4, pos.z >> 4))
        }
        fn random(&mut self) -> &mut LegacyRandom {
            &mut self.random
        }
        fn game_time(&self) -> i64 {
            self.time
        }
        fn game_rules(&self) -> horizon_world::level::GameRules {
            horizon_world::level::GameRules {
                random_tick_speed: self.speed,
                ..Default::default()
            }
        }
        fn emit(&mut self, event: LevelEvent) {
            self.events.push(event);
        }
    }

    #[test]
    fn actual_random_ticks_match_a_block_scan() {
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                let local = manifest
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION);
                if local.join("gameplay-facts.json").exists() {
                    local
                } else {
                    // The branch worktree lives below the main checkout.
                    manifest
                        .join("../../../../.local/horizon-client")
                        .join(horizon_core::VANILLA_VERSION)
                }
            });
        if !root.join("gameplay-facts.json").exists() {
            eprintln!(
                "skipping resource-backed random-tick parity: {}",
                root.display()
            );
            return;
        }
        let registry = data::load_registry(&root).unwrap();
        let (mut facts, behaviours) = BlockFacts::load_pair(&root, &registry).unwrap();
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let chunk = |x| {
            let mut c = Chunk::empty(x, 0);
            c.lit = true;
            // Every sampled block can run real behaviour and change to dirt.
            for i in [4, 7] {
                Arc::make_mut(&mut c.sections[i]).blocks =
                    crate::chunk::Section::from_blocks(&[grass.0; 4096]);
            }
            Arc::new(c)
        };
        let mut original = Level {
            chunks: [
                ((-1, 0), chunk(-1)),
                ((0, 0), chunk(0)),
                ((1, 0), Arc::new(Chunk::empty(1, 0))),
            ]
            .into_iter()
            .collect(),
            random: LegacyRandom::new(42),
            reads: RefCell::default(),
            events: Vec::new(),
            time: 0,
            speed: 3,
        };
        Arc::make_mut(original.chunks.get_mut(&(1, 0)).unwrap()).lit = true; // precipitation-only chunk
        let mut cached = original.clone();
        // The masked path reads only the drawn blocks that can tick.
        let mut masked = original.clone();
        let mut cache = RandomTickSections::default();
        let mut mask_cache = RandomTickSections::default();
        let (mut a, mut b, mut c) = (Ticks::new(12345), Ticks::new(12345), Ticks::new(12345));
        for x in -1..=1 {
            a.add_empty_chunk(ChunkPos::new(x, 0)).unwrap();
            b.add_empty_chunk(ChunkPos::new(x, 0)).unwrap();
            c.add_empty_chunk(ChunkPos::new(x, 0)).unwrap();
        }
        for step in 0..128 {
            for level in [&mut original, &mut cached, &mut masked] {
                level.time = step;
                level.speed = [0, 1, 3, 9][step as usize % 4];
                level.reads.borrow_mut().clear();
                level.events.clear();
                match step {
                    12 => {
                        level.chunks.remove(&(-1, 0));
                    }
                    13 => {
                        level.chunks.insert((-1, 0), chunk(-1));
                    }
                    30 => {
                        Arc::make_mut(level.chunks.get_mut(&(0, 0)).unwrap()).sections[7] =
                            Arc::new(crate::chunk::Section::default());
                    }
                    40 | 41 => {
                        Arc::make_mut(level.chunks.get_mut(&(0, 0)).unwrap()).lit = step == 41;
                    }
                    70 => {
                        // Every block of a ticking section replaced: its
                        // palette still lists grass, but nothing ticks there.
                        let c = Arc::make_mut(level.chunks.get_mut(&(-1, 0)).unwrap());
                        for i in 0..4096 {
                            c.set(i & 15, MIN_Y + 4 * 16 + (i >> 8), i >> 4 & 15, stone.0);
                        }
                        assert!(c.sections[4].blocks.maybe_has(|id| id == grass.0));
                    }
                    _ => {}
                }
            }
            if step == 12 {
                cache.remove(&(-1, 0));
                mask_cache.remove(&(-1, 0));
            }
            if step == 50 || step == 60 {
                facts.states[grass.0 as usize].random_ticks = step == 60;
                cache.clear();
                mask_cache.clear();
            }
            // A write between ticks.
            if step == 20 {
                for level in [&mut original, &mut cached, &mut masked] {
                    level.set_raw(BlockPos::new(0, MIN_Y + 16, 0), grass);
                }
            }
            let lit = |level: &Level| {
                let mut keys: Vec<_> = level
                    .chunks
                    .iter()
                    .filter(|(_, c)| c.lit)
                    .map(|(&key, _)| key)
                    .collect();
                keys.sort();
                keys
            };
            let random = |id| facts.get(BlockStateId(id)).random_ticks;
            for (x, z) in lit(&original) {
                // Vanilla's rule, asked as each section is reached: any of its
                // 4,096 blocks ticks randomly.
                let reference = |level: &mut Level, from: i32| {
                    let chunk = &level.chunks[&(x, z)];
                    (from..MIN_Y + SECTIONS as i32 * 16)
                        .step_by(16)
                        .find(|&min_y| {
                            (0..16).any(|y| {
                                (0..16).any(|z| (0..16).any(|x| random(chunk.get(x, min_y + y, z))))
                            })
                        })
                };
                behaviours.tick_chunk_random(&mut original, &mut a, ChunkPos::new(x, z), reference);
            }
            for (x, z) in lit(&cached) {
                let memoized = |level: &mut Level, from| {
                    cache.get((x, z), &level.chunks[&(x, z)], &facts).from(from)
                };
                behaviours.tick_chunk_random(&mut cached, &mut b, ChunkPos::new(x, z), memoized);
            }
            let mut sections = Vec::new();
            for (x, z) in lit(&masked) {
                let mut read = None;
                let next = |level: &mut Level, from| {
                    // An edit of the chunk reads its masks again (the game
                    // asks again after any write: the world's revision).
                    let now = level.chunks[&(x, z)].clone();
                    if read.as_ref().is_none_or(|r| !Arc::ptr_eq(r, &now)) {
                        mask_cache.masked(
                            (x, z),
                            &now,
                            &facts,
                            |id| behaviours.random_tick_candidate(BlockStateId(id)),
                            &mut sections,
                        );
                        read = Some(now);
                    }
                    sections
                        .iter()
                        .find(|(y, _)| *y >= from)
                        .map(|(y, mask)| (*y, Some(mask.clone())))
                };
                behaviours.tick_chunk_random_masked(&mut masked, &mut c, ChunkPos::new(x, z), next);
            }
            assert_eq!(original.reads, cached.reads, "ordered reads at tick {step}");
            assert_eq!(
                original.events, masked.events,
                "masked events at tick {step}"
            );
            assert!(masked.reads.borrow().len() <= original.reads.borrow().len());
            assert_eq!(
                a.random_tick_value, c.random_tick_value,
                "masked position RNG at tick {step}"
            );
            assert_eq!(
                original.random.clone().next_long(),
                masked.random.next_long(),
                "masked level RNG at tick {step}"
            );
            assert_eq!(original.events, cached.events, "events at tick {step}");
            assert_eq!(
                a.random_tick_value, b.random_tick_value,
                "position RNG at tick {step}"
            );
            assert_eq!(
                original.random.next_long(),
                cached.random.next_long(),
                "level RNG at tick {step}"
            );
        }
        let dirt = registry.default_state("minecraft:dirt").unwrap();
        assert!(
            original
                .chunks
                .values()
                .any(|c| c.sections[4].blocks.maybe_has(|id| id == dirt.0))
        );
        for (key, c) in &original.chunks {
            for (s, t) in c.sections.iter().zip(&cached.chunks[key].sections) {
                assert_eq!(s.blocks, t.blocks);
            }
            for (s, t) in c.sections.iter().zip(&masked.chunks[key].sections) {
                assert_eq!(s.blocks, t.blocks);
            }
        }
    }
}
