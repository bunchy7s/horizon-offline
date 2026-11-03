//! Probe for tools/test_marker_particles_parity.py.
use horizon_client::{
    Result,
    animate_tick::{
        AnimateContext, AnimateTables, animate_tick, campfire_particles, make_campfire_smoke,
        marker_particle_target,
    },
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::Chunk,
    collisions,
    command::args::GameMode,
    items::ItemStack,
    particles::{ParticleEngine, ParticleOptions, ParticleType},
    world::World,
};
use horizon_core::random::RandomSource;
use horizon_world::chunk::BlockState;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::Arc,
};

/// A `minecraft:name[k=v,...]` block state, as the cases write one.
fn parse_state(spec: &str) -> BlockState {
    let (name, properties) = match spec.split_once('[') {
        Some((name, rest)) => (name, rest.trim_end_matches(']')),
        None => (spec, ""),
    };
    BlockState {
        name: name.to_owned(),
        properties: properties
            .split(',')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    }
}
/// A position as the oracle prints it: exact double bits.
fn d(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(".local/horizon-client/26.3");
    let assets = Assets::load(root)?;
    let facts = BlockFacts::load(root, assets.registry)?;
    let biomes = Biomes::load(root)?;
    let collision = collisions::load(root, assets.registry)?;
    let tables = AnimateTables::load(root, assets.registry, &biomes)?;
    let mut world = World::new(facts.air, 0);
    let barrier = assets
        .registry
        .default_state("minecraft:barrier")
        .ok_or("barrier")?;
    for cz in -2..2 {
        for cx in -2..2 {
            world
                .chunks
                .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
            for y in 33..96 {
                for z in 0..16 {
                    for x in 0..16 {
                        world.set(cx * 16 + x, y, cz * 16 + z, barrier)?;
                    }
                }
            }
        }
    }
    let mut engine = ParticleEngine::new(root, &HashMap::new(), &Default::default());
    let mut out = BufWriter::new(File::create(&args[2])?);
    for (index, line) in BufReader::new(File::open(&args[1])?).lines().enumerate() {
        let case: Value = serde_json::from_str(&line?)?;
        let kind = case["kind"].as_str().unwrap_or("marker");
        // The campfire's own particles: the block entity's tick and the smoke a
        // douse adds, both from the block at [0, 64, 0] with the level random
        // seeded by the case.
        if kind == "campfire" || kind == "make" {
            let seed = case["seed"].as_i64().ok_or("seed")?;
            let ctx = AnimateContext {
                world: &world,
                facts: &facts,
                collision: &collision,
                biomes: &biomes,
                registry: assets.registry,
                raining_at: &|_| false,
                height_no_leaves: &|_| 0,
                firefly_bush_sounds: false,
                creaking_active: false,
                sky_darken: 0,
                marker: None,
                model_offset: &|_, _, _| None,
                gateway_particles: &|_, _| 0,
                thundering: false,
                ultrawarm: false,
            };
            let pos = [0, 64, 0];
            // Every particle the level is asked for: the engine records them
            // instead of making them, and the camera sits on the block so the
            // limiter keeps them all.
            engine.clear();
            engine.requested = None;
            engine.created_requests = Some(Vec::new());
            engine.camera = [0.5, 64.5, 0.5];
            engine.seed_randoms(seed, 0, 0);
            if kind == "campfire" {
                let state = assets
                    .registry
                    .resolve(&parse_state(case["state"].as_str().ok_or("state")?))?;
                let slots = case["slots"].as_array().ok_or("slots")?;
                let mut cooking = [false; 4];
                for (slot, held) in slots.iter().take(4).enumerate() {
                    cooking[slot] = held.as_u64() != Some(0);
                }
                let blocks = [(pos, state.0)];
                for _ in 0..case["ticks"].as_u64().ok_or("ticks")? {
                    campfire_particles(&tables, &mut engine, &ctx, blocks.iter(), &|_| cooking);
                }
            } else {
                let signal = case["signal"].as_bool().ok_or("signal")?;
                let smoking = case["smoking"].as_bool().ok_or("smoking")?;
                for _ in 0..case["puffs"].as_u64().ok_or("puffs")? {
                    make_campfire_smoke(&mut engine, pos, signal, smoking);
                }
            }
            writeln!(out, "== {index} {kind}")?;
            for (options, p, v) in engine.created_requests.take().ok_or("requests")? {
                write!(out, "{}", options.ty.name())?;
                for x in p.into_iter().chain(v) {
                    write!(out, " {}", d(x))?;
                }
                writeln!(out)?;
            }
            writeln!(out, "next {}", engine.random().next_int())?;
            continue;
        }
        let mode = GameMode::ALL
            .into_iter()
            .find(|mode| Some(mode.name()) == case["mode"].as_str())
            .ok_or("mode")?;
        let stack = ItemStack::new(
            &format!("minecraft:{}", case["main"].as_str().ok_or("main")?),
            case["count"].as_u64().ok_or("count")? as u32,
        );
        let target = marker_particle_target(mode, Some(&stack));
        writeln!(out, "{}", target.unwrap_or("none"))?;
        // Exercise the real ambient sampling loop in a dry barrier-filled world.
        let options =
            ParticleOptions::block(ParticleType::BlockMarker, "item/barrier".into(), 0xFFFFFF);
        let options = |_| Some(options.clone());
        let ctx = AnimateContext {
            world: &world,
            facts: &facts,
            collision: &collision,
            biomes: &biomes,
            registry: assets.registry,
            raining_at: &|_| false,
            height_no_leaves: &|_| 0,
            firefly_bush_sounds: false,
            creaking_active: false,
            sky_darken: 0,
            marker: target.map(|name| (name, &options as &dyn Fn(_) -> _)),
            model_offset: &|_, _, _| None,
            gateway_particles: &|_, _| 0,
            thundering: false,
            ultrawarm: false,
        };
        engine.seed_randoms(1057, 0, 0);
        engine.requested = Some(Vec::new());
        animate_tick(&tables, &mut engine, &ctx, [0, 64, 0]);
        for (options, p, v) in engine.requested.take().ok_or("requests")? {
            assert_eq!(options.ty, ParticleType::BlockMarker);
            assert_eq!(v, [0.0; 3]);
            writeln!(out, "marker {} {} {}", p[0], p[1], p[2])?;
        }
        writeln!(out, "next {}", engine.random().next_int())?;
    }
    Ok(())
}
