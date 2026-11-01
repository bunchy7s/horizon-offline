//! Which chunks tick, from the chunk tickets of `DistanceManager`.
//!
//! Every player holds two kinds of ticket:
//! - `PLAYER_LOADING` at level 31 (`ENTITY_TICKING`) on every chunk within
//!   the server's view distance (Chebyshev), from `PlayerTicketTracker`;
//! - `PLAYER_SIMULATION` at level `31 - simulationDistance` on its own chunk.
//!
//! A `/forceload`ed chunk holds a `FORCED` ticket at level 31 of both kinds.
//! An entity that came through a portal leaves a `PORTAL` ticket of radius 3
//! (`Entity.placePortalTicket`: level 30 of both kinds, for 300 ticks) on the
//! chunk it arrived in; it keeps that level running with no player in it.
//! Levels rise by one per chunk away from a ticket (`ChunkTracker`) and each
//! chunk takes the lowest. The loading level gives the chunk's
//! `FullChunkStatus` (`ChunkLevel.fullStatus`: 31 entity ticking, 32 block
//! ticking, 33 full) and the simulation level gives what
//! `inEntityTickingRange` (31 or less) and `inBlockTickingRange` (32 or less)
//! answer. Vanilla needs both:
//! - entities tick where both levels are entity ticking (`ServerLevel.tick`,
//!   and `EntityTickList` holds only the entities of entity-ticking chunks);
//! - random ticks run in the simulation tracker's entity-ticking chunks that
//!   have a ticking chunk (`ChunkMap.forEachBlockTickingChunk`);
//! - scheduled ticks, block events and block entities run where both are
//!   block ticking (`isPositionTickingWithEntitiesLoaded`,
//!   `shouldTickBlocksAt`, `LevelChunk.isTicking`).
//!
//! Whoever the players are (the local player, LAN guests, a dedicated
//! server's players), they count the same.
use crate::fast_hash::FxHashMap;
use std::sync::Arc;

/// `ChunkLevel`'s levels.
pub const ENTITY_TICKING: u8 = 31;
pub const BLOCK_TICKING: u8 = 32;
/// What a chunk with no ticket near it has (`FULL` and above count as none).
const NONE: (u8, u8) = (BLOCK_TICKING + 1, BLOCK_TICKING + 1);

/// `TicketType.PORTAL`'s timeout, in ticks.
const PORTAL_TIMEOUT: i32 = 300;
/// `Entity.placePortalTicket`'s radius: the ticket's level is `FULL` (33)
/// minus it.
const PORTAL_RADIUS: i32 = 3;
const PORTAL_LEVEL: i32 = BLOCK_TICKING as i32 + 1 - PORTAL_RADIUS;

/// The tickets of one level: where its players are, its forced chunks, its
/// portal tickets, and the server's view and simulation distances.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub players: Vec<(i32, i32)>,
    pub forced: Vec<(i32, i32)>,
    pub portal: Vec<(i32, i32)>,
    pub view_distance: i32,
    pub simulation_distance: i32,
}

/// Each chunk's (loading, simulation) level, for the chunks where either is
/// block ticking or lower; `None` ticks everywhere (a game with no server and
/// no player: the headless fixtures). With them sits the box
/// `(min_x, min_z, max_x, max_z)` that holds every chunk that can random tick,
/// so a scan of the residents need not ask each one of them.
#[derive(Clone, Default)]
pub struct Levels(Option<Arc<Map>>, Option<(i32, i32, i32, i32)>);

/// The levels by chunk: a grid over the tickets' bounding box while it is
/// small (players near each other, the usual case), a map otherwise. Every
/// block entity, mob and ticking chunk asks each tick, so an index beats a
/// hash; and a player crossing into another chunk rebuilds it all.
enum Map {
    Grid {
        x0: i32,
        z0: i32,
        width: u32,
        depth: u32,
        cells: Box<[(u8, u8)]>,
    },
    Sparse(FxHashMap<(i32, i32), (u8, u8)>),
}

/// The largest grid, in chunks: 1 MiB of levels, a box 724 chunks wide.
const GRID_CELLS: u64 = 1 << 19;

impl Map {
    fn get(&self, x: i32, z: i32) -> (u8, u8) {
        match self {
            Self::Grid {
                x0,
                z0,
                width,
                depth,
                cells,
            } => {
                let (dx, dz) = (x.wrapping_sub(*x0) as u32, z.wrapping_sub(*z0) as u32);
                if dx < *width && dz < *depth {
                    cells[(dz * width + dx) as usize]
                } else {
                    NONE
                }
            }
            Self::Sparse(map) => map.get(&(x, z)).copied().unwrap_or(NONE),
        }
    }
}

impl Levels {
    /// Chunk coordinates.
    fn get(&self, x: i32, z: i32) -> Option<(u8, u8)> {
        Some(self.0.as_ref()?.get(x, z))
    }
    /// `DistanceManager.inEntityTickingRange`, in an entity-ticking chunk:
    /// where entities tick.
    pub fn entity_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z)
            .is_none_or(|(l, s)| l <= ENTITY_TICKING && s <= ENTITY_TICKING)
    }
    /// `inBlockTickingRange` in a block-ticking chunk: where scheduled ticks,
    /// block events and block entities run.
    pub fn block_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z)
            .is_none_or(|(l, s)| l <= BLOCK_TICKING && s <= BLOCK_TICKING)
    }
    /// `inEntityTickingRange` in a block-ticking chunk: where random ticks run.
    pub fn random_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z)
            .is_none_or(|(l, s)| l <= BLOCK_TICKING && s <= ENTITY_TICKING)
    }
    /// The box that holds every chunk `random_ticking` can answer for, or
    /// `None` with no tickets at all, where it ticks everywhere. A box whose
    /// minimum is past its maximum holds nothing: the tickets hold no player,
    /// no forced chunk and no `PORTAL` one, so nothing random ticks.
    pub fn random_tick_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        self.1
    }
    /// `ChunkHolder.getFullStatus` is block ticking or more: the chunk's
    /// ticking future can complete (its saved ticks unpack then).
    pub fn loaded_ticking(&self, x: i32, z: i32) -> bool {
        self.get(x, z).is_none_or(|(l, _)| l <= BLOCK_TICKING)
    }
    /// The same as [`Self::entity_ticking`] at a block position.
    pub fn entity_ticking_at(&self, x: i32, z: i32) -> bool {
        self.entity_ticking(x >> 4, z >> 4)
    }
    /// The same as [`Self::block_ticking`] at a block position.
    pub fn block_ticking_at(&self, x: i32, z: i32) -> bool {
        self.block_ticking(x >> 4, z >> 4)
    }
}

/// The levels, rebuilt only when their sources change (a player crossing
/// into another chunk, a distance or a forced chunk changing).
#[derive(Default)]
pub struct ChunkTickets {
    sources: Option<Sources>,
    levels: Levels,
    /// The `PORTAL` tickets: the chunk and the ticks it has left.
    portal: Vec<((i32, i32), i32)>,
}

impl ChunkTickets {
    /// `None`: tick everywhere.
    pub fn update(&mut self, mut sources: Option<Sources>) {
        if let Some(s) = sources.as_mut() {
            s.players.sort_unstable();
            s.players.dedup();
            s.forced.sort_unstable();
            s.portal = self.portal.iter().map(|&(c, _)| c).collect();
            s.portal.sort_unstable();
        }
        if sources == self.sources {
            return;
        }
        self.levels = match sources.as_ref() {
            Some(s) => {
                let map = Some(Arc::new(compute(s)));
                // Tickets with no random tick of their own still get a box, an
                // empty one: `None` is for a game with no tickets at all.
                Levels(map, Some(random_bounds(s).unwrap_or((1, 1, 0, 0))))
            }
            None => Levels(None, None),
        };
        self.sources = sources;
    }
    pub fn levels(&self) -> Levels {
        self.levels.clone()
    }
    /// `TicketStorage.addTicket` of a `PORTAL` ticket: one already on the
    /// chunk only starts its timeout again.
    pub fn add_portal(&mut self, chunk: (i32, i32)) {
        match self.portal.iter_mut().find(|(c, _)| *c == chunk) {
            Some((_, left)) => *left = PORTAL_TIMEOUT,
            None => self.portal.push((chunk, PORTAL_TIMEOUT)),
        }
    }
    /// `TicketStorage.purgeStaleTickets` for the `PORTAL` tickets, once a
    /// tick: they time out after `PORTAL_TIMEOUT + 1` ticks.
    pub fn tick_portal(&mut self) {
        for (_, left) in &mut self.portal {
            *left -= 1;
        }
        self.portal.retain(|&(_, left)| left >= 0);
    }
    /// A `PORTAL` ticket is held (`shouldKeepDimensionActive`), the chunks
    /// it is on.
    pub fn portal_chunks(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.portal.iter().map(|&(c, _)| c)
    }
}

/// The box that holds every chunk `random_ticking` can answer for: a player's
/// simulation ticket reaches its simulation distance, a forced chunk only ever
/// random ticks in the chunk itself, and a `PORTAL` ticket inside its whole
/// area. Each is a superset of that ticket's random-ticking chunks, so the box
/// never rejects a chunk that random ticks.
fn random_bounds(s: &Sources) -> Option<(i32, i32, i32, i32)> {
    let sim = s.simulation_distance.clamp(0, 32);
    let boxes = s
        .players
        .iter()
        .map(|&p| (p, sim))
        .chain(s.forced.iter().map(|&f| (f, 0)))
        .chain(s.portal.iter().map(|&p| (p, PORTAL_RADIUS)));
    boxes.fold(None, |b, ((x, z), r)| {
        let (lo_x, lo_z, hi_x, hi_z) = (x - r, z - r, x + r, z + r);
        Some(match b {
            None => (lo_x, lo_z, hi_x, hi_z),
            Some((a, b, c, d)) => (lo_x.min(a), lo_z.min(b), hi_x.max(c), hi_z.max(d)),
        })
    })
}

/// How far a player's tickets reach, and the simulation ticket's level.
fn reach(s: &Sources) -> (i32, i32, i32) {
    let view = s.view_distance.clamp(0, 32);
    let simulation = s.simulation_distance.clamp(0, 32);
    // PLAYER_LOADING within the view distance, PLAYER_SIMULATION at the
    // player's chunk (`getPlayerTicketLevel`: never below 0).
    let player_sim = (i32::from(ENTITY_TICKING) - simulation).max(0);
    let reach = (view + 1).max(i32::from(BLOCK_TICKING) - player_sim);
    (view, player_sim, reach)
}

fn compute(s: &Sources) -> Map {
    let (view, player_sim, reach) = reach(s);
    let boxes = s
        .players
        .iter()
        .map(|&p| (p, reach))
        .chain(s.forced.iter().map(|&f| (f, 1)))
        .chain(s.portal.iter().map(|&p| (p, PORTAL_RADIUS)));
    let Some((x0, z0, x1, z1)) = boxes.fold(None, |b, ((x, z), r)| {
        let (lo_x, lo_z, hi_x, hi_z) = (
            i64::from(x) - i64::from(r),
            i64::from(z) - i64::from(r),
            i64::from(x) + i64::from(r),
            i64::from(z) + i64::from(r),
        );
        Some(match b {
            None => (lo_x, lo_z, hi_x, hi_z),
            Some((a, b, c, d)) => (lo_x.min(a), lo_z.min(b), hi_x.max(c), hi_z.max(d)),
        })
    }) else {
        return Map::Sparse(FxHashMap::default());
    };
    let (width, depth) = ((x1 - x0 + 1) as u64, (z1 - z0 + 1) as u64);
    if width * depth > GRID_CELLS {
        return Map::Sparse(compute_sparse(s));
    }
    let (x0, z0, width, depth) = (x0 as i32, z0 as i32, width as u32, depth as u32);
    let mut cells = vec![NONE; (width * depth) as usize].into_boxed_slice();
    let mut lower = |x: i32, z: i32, loading: i32, sim: i32| {
        let i = ((z - z0) as u32 * width + (x - x0) as u32) as usize;
        let e = &mut cells[i];
        e.0 = e.0.min(loading.min(i32::from(NONE.0)) as u8);
        e.1 = e.1.min(sim.min(i32::from(NONE.1)) as u8);
    };
    for &(px, pz) in &s.players {
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let d = dx.abs().max(dz.abs());
                lower(
                    px + dx,
                    pz + dz,
                    i32::from(ENTITY_TICKING) + (d - view).max(0),
                    player_sim + d,
                );
            }
        }
    }
    for &(fx, fz) in &s.forced {
        for dz in -1i32..=1 {
            for dx in -1i32..=1 {
                let level = i32::from(ENTITY_TICKING) + dx.abs().max(dz.abs());
                lower(fx + dx, fz + dz, level, level);
            }
        }
    }
    for &(cx, cz) in &s.portal {
        for dz in -PORTAL_RADIUS..=PORTAL_RADIUS {
            for dx in -PORTAL_RADIUS..=PORTAL_RADIUS {
                let level = PORTAL_LEVEL + dx.abs().max(dz.abs());
                lower(cx + dx, cz + dz, level, level);
            }
        }
    }
    Map::Grid {
        x0,
        z0,
        width,
        depth,
        cells,
    }
}

/// The levels as a map, for tickets spread too far apart for a grid.
fn compute_sparse(s: &Sources) -> FxHashMap<(i32, i32), (u8, u8)> {
    let (view, player_sim, reach) = reach(s);
    let mut levels: FxHashMap<(i32, i32), (u8, u8)> = FxHashMap::default();
    let mut lower = |key, loading: i32, sim: i32| {
        let e = levels.entry(key).or_insert(NONE);
        e.0 = e.0.min(loading.min(i32::from(NONE.0)) as u8);
        e.1 = e.1.min(sim.min(i32::from(NONE.1)) as u8);
    };
    for &(px, pz) in &s.players {
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let d = dx.abs().max(dz.abs());
                lower(
                    (px + dx, pz + dz),
                    i32::from(ENTITY_TICKING) + (d - view).max(0),
                    player_sim + d,
                );
            }
        }
    }
    for &(fx, fz) in &s.forced {
        for dz in -1i32..=1 {
            for dx in -1i32..=1 {
                let level = i32::from(ENTITY_TICKING) + dx.abs().max(dz.abs());
                lower((fx + dx, fz + dz), level, level);
            }
        }
    }
    for &(cx, cz) in &s.portal {
        for dz in -PORTAL_RADIUS..=PORTAL_RADIUS {
            for dx in -PORTAL_RADIUS..=PORTAL_RADIUS {
                let level = PORTAL_LEVEL + dx.abs().max(dz.abs());
                lower((cx + dx, cz + dz), level, level);
            }
        }
    }
    levels.retain(|_, l| *l != NONE);
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(players: &[(i32, i32)], forced: &[(i32, i32)], view: i32, sim: i32) -> Levels {
        let mut t = ChunkTickets::default();
        t.update(Some(Sources {
            players: players.to_vec(),
            forced: forced.to_vec(),
            portal: Vec::new(),
            view_distance: view,
            simulation_distance: sim,
        }));
        t.levels()
    }

    #[test]
    fn simulation_distance_inside_the_view() {
        // view 8, simulation 4: entities and random ticks out to 4 chunks,
        // scheduled ticks out to 5.
        let l = levels(&[(0, 0)], &[], 8, 4);
        assert!(l.entity_ticking(4, -4) && !l.entity_ticking(5, 0));
        assert!(l.random_ticking(-4, 4) && !l.random_ticking(0, 5));
        assert!(l.block_ticking(5, 5) && !l.block_ticking(6, 0));
        assert!(l.loaded_ticking(9, 9) && !l.loaded_ticking(10, 0));
    }

    #[test]
    fn view_distance_inside_the_simulation() {
        // view 4, simulation 10: what isn't loaded at entity ticking (beyond
        // the view distance) runs no entities, and block ticks stop where the
        // chunk is only full (beyond the view distance + 1).
        let l = levels(&[(0, 0)], &[], 4, 10);
        assert!(l.entity_ticking(4, 0) && !l.entity_ticking(5, 0));
        assert!(l.random_ticking(5, 0) && !l.random_ticking(6, 0));
        assert!(l.block_ticking(5, 5) && !l.block_ticking(6, 0));
    }

    #[test]
    fn every_player_and_forced_chunk() {
        let l = levels(&[(0, 0), (40, 0)], &[(-100, 7)], 4, 4);
        assert!(l.entity_ticking(44, 0) && l.entity_ticking(0, -4));
        assert!(!l.entity_ticking(20, 0));
        // A forced chunk entity ticks, its neighbours only block tick.
        assert!(l.entity_ticking(-100, 7) && !l.entity_ticking(-101, 7));
        assert!(l.block_ticking(-101, 8) && !l.block_ticking(-102, 7));
        assert!(!l.random_ticking(-101, 7));
        assert!(l.entity_ticking_at(-1600, 112) && !l.block_ticking_at(-1617, 112));
    }

    #[test]
    fn no_tickets_tick_nothing_and_no_sources_everything() {
        let none = levels(&[], &[], 10, 10);
        assert!(!none.block_ticking(0, 0) && !none.entity_ticking(0, 0));
        let everywhere = ChunkTickets::default().levels();
        assert!(everywhere.random_ticking(1000, -1000));
    }

    #[test]
    fn rebuilt_only_when_sources_change() {
        let mut t = ChunkTickets::default();
        let s = |players: Vec<(i32, i32)>| {
            Some(Sources {
                players,
                forced: Vec::new(),
                portal: Vec::new(),
                view_distance: 2,
                simulation_distance: 2,
            })
        };
        t.update(s(vec![(0, 0), (5, 5)]));
        let first = t.levels();
        t.update(s(vec![(5, 5), (0, 0)]));
        assert!(Arc::ptr_eq(
            first.0.as_ref().unwrap(),
            t.levels().0.as_ref().unwrap()
        ));
        t.update(s(vec![(1, 0), (5, 5)]));
        assert!(t.levels().entity_ticking(3, 0) && !first.entity_ticking(3, 0));
        t.update(None);
        assert!(t.levels().0.is_none());
    }

    /// The grid answers as the map does, everywhere around the tickets.
    #[test]
    fn grid_matches_the_map() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % (2 * n as u64 + 1)) as i32 - n
        };
        for case in 0..200 {
            let spread = [3, 20, 80][case % 3];
            let players: Vec<_> = (0..1 + case % 12)
                .map(|_| (next(spread), next(spread)))
                .collect();
            let forced: Vec<_> = (0..case % 4)
                .map(|_| (next(spread), next(spread)))
                .collect();
            let s = Sources {
                players,
                forced,
                portal: Vec::new(),
                view_distance: 2 + next(12).abs(),
                simulation_distance: 2 + next(12).abs(),
            };
            let Map::Grid { .. } = compute(&s) else {
                panic!("case {case} fell back to the map");
            };
            let (grid, map) = (compute(&s), Map::Sparse(compute_sparse(&s)));
            let r = spread + 50;
            for z in -r..=r {
                for x in -r..=r {
                    assert_eq!(grid.get(x, z), map.get(x, z), "case {case} at ({x}, {z})");
                }
            }
        }
    }

    #[test]
    fn far_apart_tickets_use_the_map() {
        let l = levels(
            &[(0, 0), (100_000, -100_000)],
            &[(i32::MAX - 2, i32::MIN + 2)],
            10,
            10,
        );
        assert!(matches!(**l.0.as_ref().unwrap(), Map::Sparse(_)));
        assert!(l.entity_ticking(100_010, -100_010) && !l.block_ticking(50_000, 0));
        assert!(l.entity_ticking(i32::MAX - 2, i32::MIN + 2));
    }

    /// `tick_blocks` filters its candidates with the bounds box before it asks
    /// `random_ticking`: whatever random ticks is inside it, every ticket's
    /// own area, a `PORTAL` ticket's with no player near it included.
    #[test]
    fn random_tick_bounds_cover_every_random_ticking_chunk() {
        let mut t = ChunkTickets::default();
        // A view distance shorter than the simulation one, two players far
        // apart, a forced chunk, and `PORTAL` tickets that outlive them.
        let sources = || {
            Some(Sources {
                players: vec![(0, 0), (60, 40)],
                forced: vec![(-30, 17)],
                portal: Vec::new(),
                view_distance: 4,
                simulation_distance: 12,
            })
        };
        t.update(sources());
        for portal in [(0, 0), (25, -25), (300, 300), (-80, 95)] {
            t.add_portal(portal);
            t.update(sources());
            let levels = t.levels();
            let (min_x, min_z, max_x, max_z) = levels.random_tick_bounds().unwrap();
            for z in -100..=310 {
                for x in -100..=310 {
                    if levels.random_ticking(x, z) {
                        assert!(
                            x >= min_x && x <= max_x && z >= min_z && z <= max_z,
                            "({x}, {z}) random ticks outside the box"
                        );
                    }
                }
            }
        }
    }

    /// cargo test --release -p horizon-client --lib ticket_levels::tests::levels_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn levels_cost() {
        for players in [1usize, 10, 25, 50] {
            // The benchmark's bots: most near spawn, a fifth flying outwards.
            let at: Vec<_> = (0..players)
                .map(|i| {
                    let a = i as f64 * 2.399;
                    let d = if i % 5 == 0 { 30.0 } else { 3.0 };
                    ((a.cos() * d) as i32, (a.sin() * d) as i32)
                })
                .collect();
            let s = Sources {
                players: at.clone(),
                forced: Vec::new(),
                portal: Vec::new(),
                view_distance: 10,
                simulation_distance: 10,
            };
            let rounds = 200;
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(compute_sparse(std::hint::black_box(&s)));
            }
            let map_build = t.elapsed().as_secs_f64() * 1e3 / rounds as f64;
            let t = std::time::Instant::now();
            for _ in 0..rounds {
                std::hint::black_box(compute(std::hint::black_box(&s)));
            }
            let grid_build = t.elapsed().as_secs_f64() * 1e3 / rounds as f64;
            // A tick's questions: every chunk within 12 of every player.
            let (map, grid) = (Map::Sparse(compute_sparse(&s)), compute(&s));
            let ask = |m: &Map| {
                let t = std::time::Instant::now();
                let mut n = 0u32;
                for _ in 0..rounds {
                    for &(px, pz) in &at {
                        for z in pz - 12..=pz + 12 {
                            for x in px - 12..=px + 12 {
                                n +=
                                    u32::from(std::hint::black_box(m.get(x, z)).0 <= BLOCK_TICKING);
                            }
                        }
                    }
                }
                std::hint::black_box(n);
                t.elapsed().as_secs_f64() * 1e3 / rounds as f64
            };
            println!(
                "{players} players: build {map_build:.3} -> {grid_build:.3} ms, {} lookups {:.3} -> {:.3} ms",
                players * 625,
                ask(&map),
                ask(&grid)
            );
        }
    }
}
