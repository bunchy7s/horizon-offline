//! Exact pre-optimization reference, correctness fixtures, and opt-in benchmark.
use super::*;

fn reference(pad: &Padded, info: &[Info]) -> u64 {
    let mut seen = vec![false; 4096];
    let mut result = 0u64;
    let opaque = |x: i32, y: i32, z: i32| info[pad.blocks[pi(x, y, z)] as usize].solid;
    let mut stack = Vec::new();
    for start in 0..4096usize {
        let (x, y, z) = (
            (start & 15) as i32,
            (start >> 8) as i32,
            ((start >> 4) & 15) as i32,
        );
        if seen[start] || opaque(x, y, z) {
            continue;
        }
        let mut faces = 0u8;
        seen[start] = true;
        stack.push(start);
        while let Some(c) = stack.pop() {
            let (x, y, z) = ((c & 15) as i32, (c >> 8) as i32, ((c >> 4) & 15) as i32);
            if y == 0 {
                faces |= 1 << 0;
            }
            if y == 15 {
                faces |= 1 << 1;
            }
            if z == 0 {
                faces |= 1 << 2;
            }
            if z == 15 {
                faces |= 1 << 3;
            }
            if x == 0 {
                faces |= 1 << 4;
            }
            if x == 15 {
                faces |= 1 << 5;
            }
            for (dx, dy, dz) in [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ] {
                let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                if !(0..16).contains(&nx) || !(0..16).contains(&ny) || !(0..16).contains(&nz) {
                    continue;
                }
                let n = (ny << 8 | nz << 4 | nx) as usize;
                if !seen[n] && !opaque(nx, ny, nz) {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        for a in 0..6 {
            if faces & (1 << a) != 0 {
                for b in 0..6 {
                    if faces & (1 << b) != 0 {
                        result |= 1 << (a * 6 + b);
                    }
                }
            }
        }
    }
    result
}

fn info(solid: bool) -> Info {
    Info {
        solid,
        full: false,
        permeable: false,
        shade: 1.0,
        emission: 0,
        tier: 0,
        emissive: false,
        fluid: FluidState::EMPTY,
        model: false,
        block_entity: false,
        tints: [Tint::None; 2],
        tint_count: 0,
        offset: None,
        legacy_solid: false,
        overlay: false,
        leaves: false,
        pane: false,
        opaque_cube: solid,
        blocks_flow: false,
        ice: false,
        sturdy: 0,
        face: [0; 6],
        cover: [0.0; 6],
    }
}

fn fixture(mut opaque: impl FnMut(i32, i32, i32) -> bool) -> Padded {
    // Multiple opaque and non-opaque state IDs, plus an opaque padding border.
    let mut pad = Padded {
        blocks: vec![1; P * P * P],
        light: Vec::new(),
    };
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                pad.blocks[pi(x, y, z)] = u32::from(opaque(x, y, z)) + 2 * ((x + y + z) as u32 & 1);
            }
        }
    }
    pad
}

fn random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn cases() -> Vec<(&'static str, Padded)> {
    let mut cases = vec![
        ("open", fixture(|_, _, _| false)),
        ("closed", fixture(|_, _, _| true)),
        ("terrain_half", fixture(|_, y, _| y < 8)),
        ("dividing_wall", fixture(|x, _, _| x == 8)),
        (
            "sealed_cavity",
            fixture(|x, y, z| [x, y, z].iter().any(|&v| v == 0 || v == 15)),
        ),
        (
            "disconnected_all_pairs",
            fixture(|x, y, z| {
                !((x == 8 && z == 8)
                    || ((y == 0 || y == 15) && !((7..=9).contains(&x) && (7..=9).contains(&z))))
            }),
        ),
        ("checkerboard", fixture(|x, y, z| (x + y + z) % 2 == 0)),
        (
            "six_face_tunnel",
            fixture(|x, y, z| [x, y, z].iter().filter(|&&v| v == 8).count() < 2),
        ),
        (
            "open_with_cavity",
            fixture(|x, y, z| {
                [x, y, z].iter().all(|&v| (4..=11).contains(&v))
                    && [x, y, z].iter().any(|&v| v == 4 || v == 11)
            }),
        ),
    ];
    for (name, percent) in [
        ("opaque_1pct", 1),
        ("opaque_25pct", 25),
        ("opaque_50pct", 50),
        ("opaque_75pct", 75),
        ("opaque_99pct", 99),
    ] {
        let mut seed = 0x1234_5678_9abc_def0;
        cases.push((name, fixture(|_, _, _| random(&mut seed) % 100 < percent)));
    }
    cases
}

#[test]
fn visibility_matches_reference() {
    let info = [info(false), info(true), info(false), info(true)];
    for (name, pad) in cases() {
        assert_eq!(visibility(&pad, &info), reference(&pad, &info), "{name}");
    }
    // All 256 open/closed octant combinations, including disconnected regions.
    for mask in 0..256u16 {
        let pad = fixture(|x, y, z| mask & (1 << ((x / 8) + 2 * (z / 8) + 4 * (y / 8))) != 0);
        assert_eq!(
            visibility(&pad, &info),
            reference(&pad, &info),
            "octants {mask}"
        );
    }
    // Randomized distributions include percolating regions and isolated cells.
    let mut seed = 0x8765_4321_abcd_ef01;
    for percent in [1, 10, 25, 50, 75, 90, 99] {
        for _ in 0..32 {
            let pad = fixture(|_, _, _| random(&mut seed) % 100 < percent);
            assert_eq!(visibility(&pad, &info), reference(&pad, &info));
        }
    }
}

#[test]
fn row_fill_matches_a_cell_by_cell_fill() {
    let naive = |seeds: u16, open: u16| {
        let mut fill = seeds;
        loop {
            let next = (fill | fill << 1 | fill >> 1) & open;
            if next == fill {
                return fill;
            }
            fill = next;
        }
    };
    let mut state = 0x9E37_79B9u32;
    for _ in 0..200_000 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let open = state as u16;
        let seeds = (state >> 16) as u16 & open;
        assert_eq!(
            fill_row(seeds, open),
            naive(seeds, open),
            "{open:016b} {seeds:016b}"
        );
    }
    // Several seeds in one run, the case a carry-based fill gets wrong.
    assert_eq!(fill_row(0b0110, 0b1111), 0b1111);
    assert_eq!(fill_row(0xC000, 0xF000), 0xF000);
}

#[test]
fn row_fill_follows_open_runs() {
    for open in [0u16, 0xFFFF, 0b0111_0110, 0x8001, 0xF0F0, 0x5555] {
        for bit in 0..16 {
            let seed = 1u16 << bit;
            if open & seed == 0 {
                continue;
            }
            let mut expected = seed;
            for x in (0..bit).rev().take_while(|&x| open & 1 << x != 0) {
                expected |= 1 << x;
            }
            for x in (bit + 1..16).take_while(|&x| open & 1 << x != 0) {
                expected |= 1 << x;
            }
            assert_eq!(fill_row(seed, open), expected, "{open:016b} from {bit}");
        }
    }
}

/// Mazes: one long winding cavity, and many small ones, where the row
/// sweeps are slowest.
#[test]
fn winding_and_scattered_cavities_match_reference() {
    let info = [info(false), info(true), info(false), info(true)];
    let serpentine = fixture(|x, y, z| {
        let lane = z % 4 == 0 && y % 4 == 0;
        let turn = (x == 0 && z % 8 == 4) || (x == 15 && z % 8 == 0);
        let climb = x == 0 && z == 0 && y % 4 != 0;
        !(lane || (turn && y % 4 == 0) || climb)
    });
    assert_eq!(
        visibility(&serpentine, &info),
        reference(&serpentine, &info)
    );
    let pockets = fixture(|x, y, z| !(x % 3 == 1 && y % 3 == 1 && z % 3 == 1));
    assert_eq!(visibility(&pockets, &info), reference(&pockets, &info));
}

#[test]
fn visibility_preserves_every_face_pair() {
    let info = [info(false), info(true), info(false), info(true)];
    // Each subset of six arms is one connected cavity. Its exact flags are
    // the Cartesian product of the touched faces, including self-visibility.
    for mask in 0..64u8 {
        let pad = fixture(|x, y, z| {
            let arm = (x == 8
                && z == 8
                && ((y <= 8 && mask & 1 != 0) || (y >= 8 && mask & 2 != 0)))
                || (x == 8 && y == 8 && ((z <= 8 && mask & 4 != 0) || (z >= 8 && mask & 8 != 0)))
                || (y == 8 && z == 8 && ((x <= 8 && mask & 16 != 0) || (x >= 8 && mask & 32 != 0)));
            !(arm || (x == 8 && y == 8 && z == 8))
        });
        let mut expected = 0;
        for a in 0..6 {
            for b in 0..6 {
                if mask & (1 << a) != 0 && mask & (1 << b) != 0 {
                    expected |= 1 << (6 * a + b);
                }
            }
        }
        assert_eq!(visibility(&pad, &info), expected, "arms {mask}");
        assert_eq!(reference(&pad, &info), expected);
    }
    assert_eq!(visibility(&fixture(|_, _, _| false), &info), ALL_VISIBLE);
    assert_eq!(visibility(&fixture(|_, _, _| true), &info), 0);
    // Bottom and top sheets each touch five faces; a separate vertical tube
    // supplies Down <-> Up. Their union is all flags, with no six-face region.
    let (_, pad) = cases()
        .into_iter()
        .find(|(name, _)| *name == "disconnected_all_pairs")
        .unwrap();
    assert_eq!(visibility(&pad, &info), ALL_VISIBLE);
    assert_eq!(reference(&pad, &info), ALL_VISIBLE);
    // Two disconnected corner cells must never connect opposite faces.
    let pad = fixture(|x, y, z| !((x == 0 && y == 0 && z == 0) || (x == 15 && y == 15 && z == 15)));
    let flags = visibility(&pad, &info);
    assert_eq!(flags, reference(&pad, &info));
    assert_eq!(flags & (1 << 1), 0); // Down -> Up
    assert_ne!(flags & 1, 0); // Down -> Down
}

#[test]
#[ignore = "opt-in optimized microbenchmark; see docs/visibility-benchmark.md"]
fn visibility_benchmark() {
    use std::{hint::black_box, time::Instant};
    const WARMUP: usize = 200;
    const ITERATIONS: usize = 1000;
    const SAMPLES: usize = 11;
    let info = [info(false), info(true), info(false), info(true)];
    println!(
        "case,opaque_cells,before_ns,after_ns,speedup; warmup={WARMUP}, iterations={ITERATIONS}, samples={SAMPLES}"
    );
    for (name, pad) in cases() {
        assert_eq!(visibility(&pad, &info), reference(&pad, &info), "{name}");
        let opaque = (0..4096)
            .filter(|&i| info[pad.blocks[pi(i & 15, i >> 8, (i >> 4) & 15)] as usize].solid)
            .count();
        for _ in 0..WARMUP {
            black_box(reference(black_box(&pad), black_box(&info)));
            black_box(visibility(black_box(&pad), black_box(&info)));
        }
        let measure = |f: fn(&Padded, &[Info]) -> u64| {
            let start = Instant::now();
            for _ in 0..ITERATIONS {
                black_box(f(black_box(&pad), black_box(&info)));
            }
            start.elapsed().as_nanos() as f64 / ITERATIONS as f64
        };
        let mut before = Vec::new();
        let mut after = Vec::new();
        for sample in 0..SAMPLES {
            // Alternate order to reduce drift bias.
            if sample % 2 == 0 {
                before.push(measure(reference));
                after.push(measure(visibility));
            } else {
                after.push(measure(visibility));
                before.push(measure(reference));
            }
        }
        before.sort_by(f64::total_cmp);
        after.sort_by(f64::total_cmp);
        let (before, after) = (before[SAMPLES / 2], after[SAMPLES / 2]);
        println!(
            "{name},{opaque},{before:.1},{after:.1},{:.2}x",
            before / after
        );
    }
}
