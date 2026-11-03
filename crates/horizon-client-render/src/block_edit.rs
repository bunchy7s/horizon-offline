//! Bulk block edits: runs of blocks up columns written straight into the
//! resident sections, a section unpacked and packed once however many of its
//! blocks change, instead of a `World::set` per block. Nothing round them is
//! told (as `fill ... strict`); the light of each chunk changed is worked out
//! again whole, off the game thread, rather than block by block, and each
//! section changed is remeshed once.
use crate::chunk::index;
use crate::fast_hash::FxHashMap;
use crate::world::World;

/// A run of one column, resolved: blocks `y0..=y1` of column (x, z) set to
/// `state` where filter `only` (if any) passes the block there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub x: i32,
    pub z: i32,
    pub y0: i32,
    pub y1: i32,
    pub state: u32,
    pub only: Option<u16>,
}

/// What an edit changed.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Blocks changed.
    pub changed: u64,
    /// Chunks with a change.
    pub chunks: usize,
    /// Changes of the states `tracked` asked for (block entities, watched
    /// blocks): position, previous and new state.
    pub tracked: Vec<((i32, i32, i32), u32, u32)>,
}

/// Per state answers, worked out once each.
struct Memo(Vec<u8>);
impl Memo {
    #[inline]
    fn get(&mut self, state: u32, ask: impl FnOnce(u32) -> bool) -> bool {
        let i = state as usize;
        if i >= self.0.len() {
            self.0.resize(i + 1, 0);
        }
        if self.0[i] == 0 {
            self.0[i] = 1 + u8::from(ask(state));
        }
        self.0[i] == 2
    }
}

impl World {
    /// Make `runs` in order, each seeing the changes of those before it.
    /// `matches(filter, state)` says whether a filter passes a state, and
    /// `tracked(state)` whether its changes are reported; each is asked once
    /// per state. Runs in unloaded chunks and the parts of runs outside each
    /// chunk's own column (`Chunk::layout`) are left out.
    ///
    /// Unlike `World::set`, this records nothing per block: no
    /// `block_entities.state_changed` (a caller adds or removes block
    /// entities itself, from `Outcome::tracked`), no `watched_writes` (ask
    /// for those states through `tracked`), no `block_log` entry (the chunk
    /// goes in `bulk_log` instead, sent whole), no `path_writes` (mobs'
    /// paths are not recomputed for these blocks) and no `light_edits` (the
    /// chunk and its neighbours are lit again whole, through `dirty_light`).
    pub fn edit_runs(
        &mut self,
        runs: &[Run],
        mut matches: impl FnMut(u16, u32) -> bool,
        mut tracked: impl FnMut(u32) -> bool,
    ) -> Outcome {
        let mut out = Outcome::default();
        // the runs of each chunk, in order, and the chunks in the order they were first named
        let mut by_chunk: FxHashMap<(i32, i32), Vec<u32>> = FxHashMap::default();
        let mut order = Vec::new();
        for (i, r) in runs.iter().enumerate() {
            if r.y1 < r.y0 {
                continue;
            }
            let key = (r.x >> 4, r.z >> 4);
            by_chunk
                .entry(key)
                .or_insert_with(|| {
                    order.push(key);
                    Vec::new()
                })
                .push(i as u32);
        }
        let mut filter_memo: Vec<Memo> = Vec::new();
        let mut tracked_memo = Memo(Vec::new());
        let mut values: Vec<Option<Box<[u32; 4096]>>> = Vec::new();
        let mut changed: Vec<bool> = Vec::new();
        for key in order {
            let Some(chunk) = self.chunks.get(&key) else {
                continue;
            };
            let (min_y, sections) = (chunk.min_y(), chunk.sections.len());
            let max_y = chunk.max_y();
            let list = &by_chunk[&key];
            changed.clear();
            changed.resize(sections, false);
            values.clear();
            values.resize_with(sections, || None);
            let mut any = 0u64;
            for &i in list {
                let r = runs[i as usize];
                let (lx, lz) = (r.x & 15, r.z & 15);
                let (y0, y1) = (r.y0.max(min_y), r.y1.min(max_y - 1));
                let mut y = y0;
                while y <= y1 {
                    let s = ((y - min_y) >> 4) as usize;
                    let top = (min_y + (s as i32 + 1) * 16 - 1).min(y1);
                    let vals = values[s].get_or_insert_with(|| {
                        let mut v = Box::new([0u32; 4096]);
                        chunk.sections[s].unpack(&mut v);
                        v
                    });
                    for yy in y..=top {
                        let at = index(lx, yy, lz);
                        let old = vals[at];
                        if old == r.state {
                            continue;
                        }
                        if let Some(f) = r.only {
                            let f = f as usize;
                            if filter_memo.len() <= f {
                                filter_memo.resize_with(f + 1, || Memo(Vec::new()));
                            }
                            if !filter_memo[f].get(old, |s| matches(f as u16, s)) {
                                continue;
                            }
                        }
                        vals[at] = r.state;
                        changed[s] = true;
                        any += 1;
                        if tracked_memo.get(old, &mut tracked)
                            || tracked_memo.get(r.state, &mut tracked)
                        {
                            out.tracked.push(((r.x, yy, r.z), old, r.state));
                        }
                    }
                    y = top + 1;
                }
            }
            if any == 0 {
                continue;
            }
            out.changed += any;
            out.chunks += 1;
            let chunk =
                std::sync::Arc::make_mut(self.chunks.get_mut(&key).expect("looked up above"));
            for s in 0..sections {
                if changed[s]
                    && let Some(v) = &values[s]
                {
                    chunk.replace_blocks(s, v);
                }
            }
            self.revision = self.revision.wrapping_add(1);
            self.modified.insert(key);
            if self.block_log.is_some() {
                self.bulk_log.insert(key);
            }
            // remeshed once (with the neighbours whose faces and light it touches), lit again whole
            for (s, _) in changed.iter().enumerate().filter(|c| *c.1) {
                for ds in -1..=1 {
                    let sy = s as i32 + ds;
                    if !(0..sections as i32).contains(&sy) {
                        continue;
                    }
                    for dz in -1..=1 {
                        for dx in -1..=1 {
                            self.dirty_sections.insert((key.0 + dx, sy, key.1 + dz));
                        }
                    }
                }
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    self.dirty_light.insert((key.0 + dx, key.1 + dz));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use horizon_world::registry::BlockStateId;
    use std::sync::Arc;

    fn world() -> World {
        let mut w = World::new(BlockStateId(0), 0);
        for cz in -1..=1 {
            for cx in -1..=1 {
                w.chunks.insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
            }
        }
        // stone up to y 63
        for z in -16..32 {
            for x in -16..32 {
                for y in -64..64 {
                    w.set(x, y, z, BlockStateId(1)).unwrap();
                }
            }
        }
        w.dirty_light.clear();
        w.dirty_sections.clear();
        w.block_log = Some(Vec::new());
        w
    }
    fn run(x: i32, z: i32, y0: i32, y1: i32, state: u32, only: Option<u16>) -> Run {
        Run {
            x,
            z,
            y0,
            y1,
            state,
            only,
        }
    }

    #[test]
    fn runs_change_their_blocks_in_order() {
        let mut w = world();
        let runs = [
            run(3, 4, 10, 40, 0, None),
            run(3, 4, 10, 12, 5, None),
            // only where it is air: over the stone, and the three blocks of 5 stay
            run(3, 4, 0, 80, 7, Some(0)),
            run(-1, 20, -70, 400, 9, None),
        ];
        let out = w.edit_runs(&runs, |_, s| s == 0, |_| false);
        assert_eq!(w.block(3, 9, 4).0, 1);
        assert_eq!(w.block(3, 11, 4).0, 5);
        assert_eq!(w.block(3, 13, 4).0, 7);
        assert_eq!(w.block(3, 70, 4).0, 7);
        assert_eq!(w.block(3, 81, 4).0, 0);
        assert_eq!(w.block(3, 0, 4).0, 1);
        assert_eq!(w.block(-1, -64, 20).0, 9);
        assert_eq!(w.block(-1, 319, 20).0, 9);
        assert_eq!(w.block(0, 30, 20).0, 1);
        // 31 dug, 3 set again, the air over them filled (28, and 17 over the stone), the whole of the other column
        assert_eq!(out.changed, 31 + 3 + 45 + 384);
        assert_eq!(out.chunks, 2);
        assert!(w.bulk_log.contains(&(0, 0)) && w.bulk_log.contains(&(-1, 1)));
        assert!(w.dirty_light.contains(&(1, 1)) && w.dirty_light.contains(&(-2, 2)));
        assert!(w.dirty_sections.contains(&(1, 4, 1)));
        // the per-block log is left to the blocks set one at a time
        assert!(w.block_log.as_ref().unwrap().is_empty());
    }

    #[test]
    fn unloaded_chunks_and_unchanged_blocks_are_left_out() {
        let mut w = world();
        let out = w.edit_runs(
            &[run(100, 100, 0, 10, 0, None), run(0, 0, 0, 10, 1, None)],
            |_, _| true,
            |_| false,
        );
        assert_eq!(out.changed, 0);
        assert_eq!(out.chunks, 0);
        assert!(w.bulk_log.is_empty() && w.dirty_light.is_empty());
    }

    #[test]
    fn tracked_states_are_reported() {
        let mut w = world();
        let out = w.edit_runs(&[run(1, 1, 60, 65, 3, None)], |_, _| true, |s| s == 3);
        assert_eq!(out.tracked.len(), 6);
        assert_eq!(out.tracked[0], ((1, 60, 1), 1, 3));
        assert_eq!(out.tracked[5], ((1, 65, 1), 0, 3));
    }

    #[test]
    fn heightmaps_follow_the_edit() {
        let mut w = world();
        let air = |s: u32| s == 0;
        assert_eq!(w.world_surface(5, 5, |s| air(s.0)), 64);
        w.edit_runs(&[run(5, 5, 20, 63, 0, None)], |_, _| true, |_| false);
        assert_eq!(w.world_surface(5, 5, |s| air(s.0)), 20);
        w.edit_runs(&[run(5, 5, 90, 90, 2, None)], |_, _| true, |_| false);
        assert_eq!(w.world_surface(5, 5, |s| air(s.0)), 91);
    }

    #[test]
    fn runs_follow_each_chunks_own_layout() {
        use crate::chunk::ColumnLayout;
        // a data pack's taller dimension: -128..=511
        let tall = ColumnLayout::new(-128, 640).unwrap();
        let mut w = World::new(BlockStateId(0), 0);
        w.chunks
            .insert((0, 0), Arc::new(Chunk::with_layout(0, 0, tall)));
        let out = w.edit_runs(&[run(2, 3, -500, 900, 4, None)], |_, _| true, |_| false);
        assert_eq!(out.changed, 640);
        let chunk = &w.chunks[&(0, 0)];
        assert_eq!(chunk.block(2, -128, 3).0, 4);
        assert_eq!(chunk.block(2, -65, 3).0, 4);
        assert_eq!(chunk.block(2, 320, 3).0, 4);
        assert_eq!(chunk.block(2, 511, 3).0, 4);
        assert!(w.dirty_sections.contains(&(0, 39, 0)));
        assert!(!w.dirty_sections.contains(&(0, 40, 0)));
    }
}
