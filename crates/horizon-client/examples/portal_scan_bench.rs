//! The `entity_portals` section scan only: `may_hold_portal` over synthetic
//! entity reaches, with the per-tick palette map against the resident cache.
//! Run with prepared client resources:
//! cargo run --profile fast -p horizon-client --example portal_scan_bench -- RESOURCES
use horizon_client::{
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y, SECTIONS, Section},
    fast_hash::FxHashMap,
    portal_sections::PortalSections,
    section_mask::SectionMask,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use horizon_world::{level::Behaviours, registry::BlockStateId};
use std::{hint::black_box, path::Path, sync::Arc, time::Instant};

/// `Game::swept_portal`'s reach (`Entity.checkInsideBlocks`'s swept box).
fn reach(from: Vec3, to: Vec3, width: f32, height: f32) -> Aabb {
    let half = f64::from(width) / 2.0;
    let bounds = Aabb::new(
        Vec3::new(to.x - half, to.y, to.z - half),
        Vec3::new(to.x + half, to.y + f64::from(height), to.z + half),
    )
    .deflate(Vec3::new(1.0e-5, 1.0e-5, 1.0e-5));
    bounds
        .union_bounds(bounds.move_by(from.subtract(to)))
        .inflate(Vec3::new(1.0, 1.0, 1.0))
}

fn sections(reach: Aabb) -> (i64, i32, i32, i32, i32, i32, i32) {
    let (min, max) = (reach.min_position(), reach.max_position());
    let (min, max) = (BlockPos::containing(min), BlockPos::containing(max));
    let top = SECTIONS as i32 - 1;
    let (x0, x1) = (min.x >> 4, max.x >> 4);
    let (z0, z1) = (min.z >> 4, max.z >> 4);
    let (y0, y1) = (
        ((min.y - MIN_Y) >> 4).clamp(0, top),
        ((max.y - MIN_Y) >> 4).clamp(0, top),
    );
    let count = (i64::from(x1) - i64::from(x0) + 1)
        * (i64::from(y1) - i64::from(y0) + 1)
        * (i64::from(z1) - i64::from(z0) + 1);
    (count, x0, x1, y0, y1, z0, z1)
}

/// The original `Game::may_hold_portal`: per-tick `seen`, palette per section.
fn may_hold_portal_baseline(
    reach: Aabb,
    world: &FxHashMap<(i32, i32), Arc<Chunk>>,
    seen: &mut FxHashMap<(i32, i32, i32), bool>,
    behaviours: &Behaviours,
) -> bool {
    let (count, x0, x1, y0, y1, z0, z1) = sections(reach);
    if count > 64 {
        return true;
    }
    (x0..=x1).any(|cx| {
        (z0..=z1).any(|cz| {
            (y0..=y1).any(|sy| {
                *seen.entry((cx, sy, cz)).or_insert_with(|| {
                    world
                        .get(&(cx, cz))
                        .and_then(|c| c.sections.get(sy as usize))
                        .is_some_and(|s| {
                            s.blocks
                                .maybe_has(|v| behaviours.portal_axis(BlockStateId(v)).is_some())
                        })
                })
            })
        })
    })
}

/// The cached `Game::may_hold_portal`: one mask per resident chunk.
fn may_hold_portal_cached(
    reach: Aabb,
    world: &FxHashMap<(i32, i32), Arc<Chunk>>,
    cache: &mut PortalSections,
    behaviours: &Behaviours,
) -> bool {
    let (count, x0, x1, y0, y1, z0, z1) = sections(reach);
    if count > 64 {
        return true;
    }
    let y_mask = SectionMask::below(y1 as usize + 1).from(y0 as usize);
    (x0..=x1).any(|cx| {
        (z0..=z1).any(|cz| {
            world.get(&(cx, cz)).is_some_and(|chunk| {
                let portals = cache.get((cx, cz), chunk, |v| {
                    behaviours.portal_axis(BlockStateId(v)).is_some()
                });
                !(portals & y_mask).is_empty()
            })
        })
    })
}

fn main() -> horizon_client::Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or("expected prepared client resource directory")?;
    let start = Instant::now();
    let registry = horizon_world::level::data::load_registry(Path::new(&root))?;
    let (facts, behaviours) = BlockFacts::load_pair(Path::new(&root), &registry)?;
    println!(
        "resource_load_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let len = registry.len() as u32;
    let portal: Vec<u32> = (0..len)
        .filter(|&id| behaviours.portal_axis(BlockStateId(id)).is_some())
        .collect();
    let inactive_states: Vec<u32> = (0..len)
        .filter(|&id| behaviours.portal_axis(BlockStateId(id)).is_none())
        .collect();
    let _ = &facts;
    let portal_state = *portal
        .first()
        .ok_or("no nether portal block state in the registry")?;
    println!("portal_states={}", portal.len());

    // 500 entities spread over a 17x17 chunk world. Their reaches touch 1-4
    // sections, as an item or a mob standing in or stepping into a portal does.
    let grid = 17;
    let base = -(grid / 2) * 16;
    let entities: Vec<(Vec3, Vec3, f32, f32)> = (0..500)
        .map(|i| {
            let x = base + (i % grid) * 16 + (i % 5);
            let z = base + (i / grid % grid) * 16 + (i % 7);
            let y = 64 + (i % 3) * 16;
            let from = Vec3::new(f64::from(x), f64::from(y), f64::from(z));
            let to = from.add_vector(Vec3::new(0.1, 0.0, 0.07));
            (from, to, 0.25, 0.25)
        })
        .collect();
    let reaches: Vec<Aabb> = entities
        .iter()
        .map(|&(from, to, w, h)| reach(from, to, w, h))
        .collect();

    for (name, palette, portals_every) in [
        ("sparse", 1usize, 0usize),
        ("dense", 128, 0),
        ("global", 300, 0),
        ("dense_portal", 128, 5),
    ] {
        let start = Instant::now();
        let mut world: FxHashMap<(i32, i32), Arc<Chunk>> = FxHashMap::default();
        for czi in 0..grid {
            for cxi in 0..grid {
                let (cx, cz) = (-(grid / 2) + cxi, -(grid / 2) + czi);
                let mut chunk = Chunk::empty(cx, cz);
                for (i, section) in chunk.sections.iter_mut().enumerate() {
                    let values: [u32; 4096] = std::array::from_fn(|j| {
                        if portals_every > 0 && i % portals_every == 0 && j == 0 {
                            portal_state
                        } else {
                            inactive_states[(j * 7 + i) % palette.min(inactive_states.len())]
                        }
                    });
                    Arc::make_mut(section).blocks = Section::from_blocks(&values);
                }
                world.insert((cx, cz), Arc::new(chunk));
            }
        }
        println!(
            "{name} fixture_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        let keys: Vec<_> = world.keys().copied().collect();
        let portal_chunks: Vec<(i32, i32)> = keys
            .iter()
            .copied()
            .filter(|&(cx, cz)| {
                world[&(cx, cz)]
                    .sections
                    .iter()
                    .any(|s| s.blocks.maybe_has(|v| v == portal_state))
            })
            .collect();
        println!("{name} portal_chunks={}", portal_chunks.len());
        // The global palette's maybe_has walks the whole ID range, so rescanning
        // every chunk each tick (cold/reload/edit_all) is intractable; those rows
        // are only meaningful for the palettes whose scans are bounded.
        let workloads: &[(&str, usize)] = if name == "global" {
            &[("steady", 0), ("edit_1pct", 3)]
        } else {
            &[
                ("steady", 0),
                ("cold", 0),
                ("reload", 0),
                ("edit_1pct", 3),
                ("edit_all", 289),
            ]
        };
        for &(workload, changed) in workloads {
            let mut baseline = Vec::new();
            let mut candidate = Vec::new();
            let mut cache = PortalSections::default();
            let (mut base_hits, mut cand_hits) = (0u64, 0u64);
            for sample in 0..8 {
                for cached in if sample % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let start = Instant::now();
                    let mut result = 0u64;
                    for tick in 0..300 {
                        if cached && workload == "cold" {
                            cache = PortalSections::default();
                        }
                        if cached && workload == "reload" {
                            cache.clear();
                        }
                        for key in keys.iter().take(changed) {
                            Arc::make_mut(world.get_mut(key).unwrap()).set(
                                0,
                                MIN_Y,
                                0,
                                inactive_states[tick % inactive_states.len()],
                            );
                        }
                        let mut seen = FxHashMap::default();
                        for reach in &reaches {
                            let hit = if cached {
                                may_hold_portal_cached(*reach, &world, &mut cache, &behaviours)
                            } else {
                                may_hold_portal_baseline(*reach, &world, &mut seen, &behaviours)
                            };
                            result += hit as u64;
                        }
                    }
                    if cached {
                        cache = PortalSections::default();
                        cand_hits += black_box(result);
                    } else {
                        base_hits += black_box(result);
                    }
                    if sample > 0 {
                        let us = start.elapsed().as_secs_f64() * 1e6 / 300.0;
                        if cached {
                            candidate.push(us);
                        } else {
                            baseline.push(us);
                        }
                    }
                }
            }
            assert_eq!(
                base_hits, cand_hits,
                "{name} {workload}: cached may_hold_portal diverged"
            );
            println!("{name} {workload} baseline_us={baseline:.3?} candidate_us={candidate:.3?}");
        }
    }
    println!("chunk_wrapper_bytes={}", std::mem::size_of::<Chunk>());
    Ok(())
}
