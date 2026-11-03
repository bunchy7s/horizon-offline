//! Chunk-send batches for separated players, without a window or sockets.
//! cargo run --profile fast -p horizon-client-lib --example chunk_encoding_bench -- RESOURCES
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    assets::Assets,
    chunk::{Chunk, Section},
    lan::chunks::ChunkEncoder,
    save::SaveTables,
    spans::thread_cpu,
    world::World,
};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    hint::black_box,
    path::Path,
    sync::Arc,
    time::Instant,
};

fn bodies(encoder: &ChunkEncoder, columns: &[&Chunk], world: &World) -> Vec<Vec<u8>> {
    columns.iter().map(|c| encoder.body_in(c, world)).collect()
}

fn fixture(
    players: i32,
    side: i32,
    filled: usize,
    stone: u32,
    water: u32,
) -> (World, Vec<(i32, i32)>) {
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut keys = Vec::new();
    for p in 0..players {
        let (cx, cz) = ((p % 10) * 150, (p / 10) * 150);
        for dz in -1..=side {
            for dx in -1..=side {
                let (x, z) = (cx + dx, cz + dz);
                let mut c = Chunk::empty(x, z);
                c.lit = true;
                for sy in 0..filled {
                    let values = std::array::from_fn(|i| {
                        if (i + sy * 3 + x.unsigned_abs() as usize) % 37 < 17 {
                            stone
                        } else {
                            water
                        }
                    });
                    Arc::make_mut(&mut c.sections[sy]).blocks = Section::from_blocks(&values);
                }
                world.chunks.insert((x, z), Arc::new(c));
                if (0..side).contains(&dx) && (0..side).contains(&dz) {
                    keys.push((x, z));
                }
            }
        }
    }
    (world, keys)
}
fn main() -> horizon_client::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(args.get(1).ok_or("expected prepared resources")?);
    let assets = Assets::load(root)?;
    let stone = assets.registry.default_state("minecraft:stone").unwrap().0;
    let water = assets.registry.default_state("minecraft:water").unwrap().0;
    let tables = Arc::new(SaveTables::new(&assets));
    let biomes = horizon_client::biome::Biomes::load(root)?;
    let biome_names: Vec<String> = biomes.list.iter().map(|b| b.name.clone()).collect();
    let sync =
        horizon_client::lan::data::SyncData::load(root, &horizon_world::loot::PackOverlay::NONE)?;
    let biome_registry: Vec<String> = sync
        .dynamic
        .registries
        .get("minecraft:worldgen/biome")
        .ok_or("missing biome registry")?
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let make = || {
        let states = (0..assets.registry.len()).map(|i| {
            let s = assets
                .registry
                .state(horizon_world::registry::BlockStateId(i as u32))
                .unwrap();
            (
                Arc::<str>::from(s.name.as_str()),
                s.name == "minecraft:water"
                    || s.name == "minecraft:lava"
                    || s.properties.get("waterlogged").is_some_and(|v| v == "true"),
            )
        });
        let mut e = ChunkEncoder::new(states, &sync.builtin, &biome_names, &biome_registry);
        e.tables = Some(tables.clone());
        e
    };
    for (players, side, filled) in [
        (1, 1, 8),
        (100, 1, 8),
        (100, 2, 8),
        (100, 4, 8),
        (100, 4, 0),
    ] {
        let (world, keys) = fixture(players, side, filled, stone, water);
        let columns: Vec<_> = keys
            .iter()
            .map(|k| &**world.chunks.get(k).unwrap())
            .collect();
        let reference = make();
        let expected = bodies(&reference, &columns, &world);
        let mut hasher = DefaultHasher::new();
        expected.hash(&mut hasher);
        let hash = hasher.finish();
        let bytes: usize = expected.iter().map(Vec::len).sum();
        if players == 100 && side == 1 {
            for sample in 0..8 {
                let encoder = make();
                let mut h = DefaultHasher::new();
                let mut biome_bytes = 0;
                for c in &columns {
                    let b = encoder.biome_data(c);
                    b.buffer.hash(&mut h);
                    biome_bytes += b.buffer.len();
                }
                let start = Instant::now();
                let cpu = thread_cpu();
                for _ in 0..50 {
                    for c in &columns {
                        black_box(encoder.biome_data(c));
                    }
                }
                let wall_us = start.elapsed().as_secs_f64() * 1e6 / 50.0;
                let cpu_us = thread_cpu()
                    .zip(cpu)
                    .map(|(b, a)| (b - a).as_secs_f64() * 1e6 / 50.0);
                if sample > 0 {
                    println!(
                        "{}",
                        serde_json::json!({"phase":"biomes","players":players,"side":side,"filled":filled,"cold":false,"sample":sample,"cpu_us":cpu_us,"wall_us":wall_us,"bytes":biome_bytes,"hash":format!("{:016x}",h.finish())})
                    );
                }
            }
        }
        for cold in [true, false] {
            let e = make();
            e.warm(&columns);
            assert_eq!(bodies(&e, &columns, &world), expected);
            for sample in 0..8 {
                let encoder = if cold { make() } else { e.detached() };
                if !cold {
                    encoder.warm(&columns);
                    black_box(bodies(&encoder, &columns, &world));
                }
                let wall = Instant::now();
                let cpu = thread_cpu();
                let rounds = if cold { 1 } else { 10 };
                for _ in 0..rounds {
                    if cold {
                        encoder.warm(&columns);
                    }
                    black_box(bodies(&encoder, &columns, &world));
                }
                let cpu_us = thread_cpu()
                    .zip(cpu)
                    .map(|(b, a)| (b - a).as_secs_f64() * 1e6 / rounds as f64);
                let wall_us = wall.elapsed().as_secs_f64() * 1e6 / rounds as f64;
                if sample > 0 {
                    println!(
                        "{}",
                        serde_json::json!({"phase":"columns","players":players,"side":side,"filled":filled,"cold":cold,"sample":sample,"cpu_us":cpu_us,"wall_us":wall_us,"bytes":bytes,"hash":format!("{hash:016x}")})
                    );
                }
            }
        }
    }
    Ok(())
}
