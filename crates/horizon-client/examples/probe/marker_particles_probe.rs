//! Probe for tools/test_marker_particles_parity.py.
use horizon_client::{
    Result,
    animate_tick::{AnimateContext, AnimateTables, animate_tick, marker_particle_target},
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
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::Arc,
};

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
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let case: Value = serde_json::from_str(&line?)?;
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
