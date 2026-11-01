//! Profile the fact and behaviour table phase of client startup.
//! Run each mode in a separate process to capture CPU usage and peak RSS.
//! The deferred mode evaluates both phases contiguously for fair loader comparison;
//! actual client startup performs other initialization between these phases.
use horizon_client::{assets::Assets, blocks::BlockFacts};
use horizon_world::level::Behaviours;
use std::{path::Path, time::Instant};

fn resident_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("expected resource directory")?;
    let mode = args
        .next()
        .ok_or("expected baseline, pr607, shared, deferred or deferred-held")?;
    if !["baseline", "pr607", "shared", "deferred", "deferred-held"].contains(&mode.as_str()) {
        return Err("expected baseline, pr607, shared, deferred or deferred-held".into());
    }
    let root = Path::new(&root);
    let total = Instant::now();
    let assets = Assets::load(root)?;
    let started = Instant::now();
    let mut retained_inputs = None;
    let mut subphases = None;
    let (facts, behaviours) = if mode == "baseline" {
        (
            BlockFacts::load(root, assets.registry)?,
            Behaviours::load(assets.registry, root)?,
        )
    } else if mode == "pr607" {
        // Original PR #607 sequence at a01d47f2, retained as a same-build comparator.
        let json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(root.join("block-behaviours.json"))?,
        ))?;
        BlockFacts::load_with_behaviours(root, assets.registry, &json)?
    } else if mode == "shared" {
        BlockFacts::load_pair(root, assets.registry)?
    } else {
        let parse_started = Instant::now();
        let json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(root.join("block-behaviours.json"))?,
        ))?;
        let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
        let facts_started = Instant::now();
        let (facts, gameplay) =
            BlockFacts::load_with_projection(root, assets.registry, &json, true)?;
        let facts_ms = facts_started.elapsed().as_secs_f64() * 1000.0;
        let behaviours_started = Instant::now();
        let behaviours = Behaviours::load_with_gameplay(assets.registry, root, &json, &gameplay)?;
        let behaviours_ms = behaviours_started.elapsed().as_secs_f64() * 1000.0;
        subphases = Some((parse_ms, facts_ms, behaviours_ms));
        if mode == "deferred-held" {
            retained_inputs = Some((json, gameplay));
        }
        (facts, behaviours)
    };
    println!("phase_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
    println!("total_ms={:.3}", total.elapsed().as_secs_f64() * 1000.0);
    println!("states={}", facts.states.len());
    if let Some((parse, facts, behaviours)) = subphases {
        println!("behaviour_json_ms={parse:.3}");
        println!("facts_ms={facts:.3}");
        println!("behaviours_ms={behaviours:.3}");
    }
    if let Some(rss) = resident_kib() {
        println!("post_rss_kib={rss}");
    }
    std::hint::black_box((facts, behaviours, retained_inputs));
    Ok(())
}
