//! Selection cost only (not block simulation). Run with prepared client resources.
//! cargo run --profile fast -p horizon-client --example random_tick_eligibility_bench -- RESOURCES
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y, Section},
    fast_hash::FxHashMap,
    random_tick_sections::RandomTickSections,
};
use horizon_world::registry::BlockStateId;
use std::{hint::black_box, path::Path, sync::Arc, time::Instant};

fn select_baseline(chunks: &FxHashMap<(i32, i32), Arc<Chunk>>, facts: &BlockFacts) -> usize {
    let mut ticking = Vec::new();
    for (&(x, z), chunk) in chunks {
        if x.abs() > 8 || z.abs() > 8 || !chunk.lit {
            continue;
        }
        let sections: Vec<i32> = chunk
            .sections
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.blocks
                    .maybe_has(|id| facts.get(BlockStateId(id)).random_ticks)
            })
            .map(|(i, _)| MIN_Y + i as i32 * 16)
            .collect();
        ticking.push((x, z, sections));
    }
    black_box(ticking).iter().map(|(_, _, s)| s.len()).sum()
}

fn select_cached(
    chunks: &FxHashMap<(i32, i32), Arc<Chunk>>,
    facts: &BlockFacts,
    cache: &mut RandomTickSections,
) -> usize {
    let mut ticking = Vec::new();
    for (&(x, z), chunk) in chunks {
        if x.abs() > 8 || z.abs() > 8 || !chunk.lit {
            continue;
        }
        ticking.push((x, z, cache.get((x, z), chunk, facts)));
    }
    black_box(ticking).iter().map(|(_, _, s)| s.len()).sum()
}

fn main() -> horizon_client::Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or("expected prepared client resource directory")?;
    let start = Instant::now();
    let registry = horizon_world::level::data::load_registry(Path::new(&root))?;
    let facts = BlockFacts::load(Path::new(&root), &registry)?;
    println!(
        "resource_load_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let inactive: Vec<u32> = facts
        .states
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.random_ticks)
        .take(128)
        .map(|(i, _)| i as u32)
        .collect();
    let active = registry.default_state("minecraft:grass_block").unwrap().0;
    assert!(facts.get(BlockStateId(active)).random_ticks);
    for (name, palette) in [("sparse", 1), ("dense", 128)] {
        let start = Instant::now();
        let mut chunks = FxHashMap::default();
        for z in -8..=8 {
            for x in -8..=8 {
                let mut chunk = Chunk::empty(x, z);
                chunk.lit = true;
                for (i, section) in chunk.sections.iter_mut().enumerate() {
                    let mut values = std::array::from_fn(|j| inactive[j % palette]);
                    if i % 3 == 0 {
                        values[4095] = active;
                    }
                    Arc::make_mut(section).blocks = Section::from_blocks(&values);
                }
                chunks.insert((x, z), Arc::new(chunk));
            }
        }
        println!(
            "{name} fixture_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        for (workload, changed) in [
            ("steady", 0),
            ("cold", 0),
            ("reload", 0),
            ("edit_1pct", 3),
            ("edit_all", 289),
        ] {
            let keys: Vec<_> = chunks.keys().copied().collect();
            let mut baseline = Vec::new();
            let mut candidate = Vec::new();
            let mut cache = RandomTickSections::default();
            for sample in 0..8 {
                // Alternate order; first pair warms both paths and is discarded.
                for cached in if sample % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let start = Instant::now();
                    for tick in 0..300 {
                        if cached && workload == "cold" {
                            cache = RandomTickSections::default();
                        }
                        if cached && workload == "reload" {
                            cache.clear();
                        }
                        for key in keys.iter().take(changed) {
                            Arc::make_mut(chunks.get_mut(key).unwrap()).set(
                                0,
                                MIN_Y,
                                0,
                                inactive[tick % 2],
                            );
                        }
                        black_box(if cached {
                            select_cached(&chunks, &facts, &mut cache)
                        } else {
                            select_baseline(&chunks, &facts)
                        });
                    }
                    // Include teardown, and remove weak handles before baseline edits.
                    if cached {
                        cache = RandomTickSections::default();
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
            println!("{name} {workload} baseline_us={baseline:.3?} candidate_us={candidate:.3?}");
        }
    }
    println!("chunk_wrapper_bytes={}", std::mem::size_of::<Chunk>());
    Ok(())
}
