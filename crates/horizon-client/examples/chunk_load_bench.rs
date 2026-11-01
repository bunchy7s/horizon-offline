//! What a column costs to load, on real terrain: generates and lights the
//! columns around a point with the vanilla generator, as a world being explored
//! does, then meshes every section of the inner columns on this one thread, so a
//! change to generation or meshing shows as time per column and per section.
//!
//!     cargo run --release -p horizon-client --example chunk_load_bench -- \
//!         .local/horizon-client/26.3 [--seed 1234] [--center CX CZ] [--radius 8]
//!         [--threads N] [--rounds 3]
//!
//! The default centre is the village of `tools/perf_bench.py`. The generation
//! figure is wall time with N workers (the game's own pool); the mesh figure is
//! this thread's CPU time in `Mesher::mesh`, the best of `--rounds`, so other
//! work on the machine changes it less. The checksum of the meshes shows that an
//! A/B pair of builds made the same ones.
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::HEIGHT,
    light::LightTables,
    mesher::{MeshTables, Mesher, Region, borrow},
    models,
    spans::thread_cpu,
    streaming::{Streaming, WorkerContext},
    vanillagen::VanillaTerrain,
    world::World,
};
use horizon_core::biome::{BiomeZoom, obfuscate_seed};
use horizon_world::registry::BlockStateId;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

type Error = Box<dyn std::error::Error>;

fn flag<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(default)
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(1).ok_or("usage: chunk_load_bench <resources>")?);
    let seed: i64 = flag(&args, "--seed", 1234);
    let radius: i32 = flag(&args, "--radius", 8);
    let rounds: usize = flag(&args, "--rounds", 3);
    let threads: usize = flag(
        &args,
        "--threads",
        thread::available_parallelism().map_or(2, |n| n.get()),
    );
    let center = args
        .iter()
        .position(|a| a == "--center")
        .map(|i| -> Result<(i32, i32), Error> {
            Ok((
                args.get(i + 1).ok_or("--center CX CZ")?.parse()?,
                args.get(i + 2).ok_or("--center CX CZ")?.parse()?,
            ))
        })
        .transpose()?
        .unwrap_or((-60, 10));

    let assets = Arc::new(Assets::load(&root)?);
    let render = Arc::new(assets.reload(&root)?);
    let facts = BlockFacts::load(&root, assets.registry)?;
    let visibility = models::load_visibility(&root, assets.registry)?;
    let biomes = Arc::new(Biomes::load(&root)?);
    let context = Arc::new(WorkerContext {
        tables: Arc::new(MeshTables::new(&assets, &render, &facts, visibility)?),
        light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
        biomes: biomes.clone(),
        assets: assets.clone(),
        render,
    });
    let terrain = VanillaTerrain::new(&root.join("worldgen"), assets.registry, &biomes, seed)?;

    // Generation and light, on the game's worker pool. Meshing is left out
    // here and measured on its own below.
    let mut streaming = Streaming::new(Arc::new(terrain), context.clone(), threads);
    streaming.mesh = false;
    streaming.center = center;
    streaming.radius = radius;
    let mut world = World::new(BlockStateId(0), 0);
    let started = Instant::now();
    let mut quiet = 0;
    while quiet < 100 {
        let before = world.chunks.len();
        let _ = streaming.update(&mut world);
        thread::sleep(Duration::from_millis(2));
        quiet = if streaming.pending() == 0 && before == world.chunks.len() {
            quiet + 1
        } else {
            0
        };
    }
    let generated = started.elapsed() - Duration::from_millis(2 * 100);
    let columns = world.chunks.len();
    println!(
        "generated and lit {columns} columns in {:.0} ms with {threads} workers ({:.2} ms per column)",
        generated.as_secs_f64() * 1e3,
        generated.as_secs_f64() * 1e3 / columns.max(1) as f64
    );

    // Meshing every section of the columns whose neighbours are all loaded.
    let inner = radius - 1;
    let regions: Vec<(Region, i32)> = (center.0 - inner..=center.0 + inner)
        .flat_map(|x| (center.1 - inner..=center.1 + inner).map(move |z| (x, z)))
        .filter_map(|(x, z)| {
            let region: Region = std::array::from_fn(|dz| {
                std::array::from_fn(|dx| {
                    world
                        .chunks
                        .get(&(x + dx as i32 - 1, z + dz as i32 - 1))
                        .cloned()
                })
            });
            region
                .iter()
                .flatten()
                .all(Option::is_some)
                .then_some(region)
        })
        .flat_map(|region| (0..HEIGHT / 16).map(move |sy| (region.clone(), sy)))
        .collect();
    let mut best = Duration::MAX;
    let (mut vertices, mut checksum) = (0, 0);
    for _ in 0..rounds.max(1) {
        let mut mesher = Mesher {
            nether_shading: false,
            shader_pack_lighting: false,
            zoom: BiomeZoom::new(obfuscate_seed(seed)),
            blend_radius: 2,
            smooth_lighting: true,
            cutout_leaves: true,
            assets: &context.assets,
            render: &context.render,
            tables: &context.tables,
            biomes: &context.biomes,
            visibility: context.tables.visibility.clone(),
        };
        vertices = 0;
        let mut hash = DefaultHasher::new();
        let mut spent = Duration::ZERO;
        for (region, sy) in &regions {
            let started = thread_cpu().ok_or("no thread CPU clock here")?;
            let mesh = mesher.mesh(&borrow(region), *sy);
            spent += thread_cpu().ok_or("no thread CPU clock here")? - started;
            vertices += mesh.solid.len() + mesh.translucent.len();
            bytemuck::cast_slice::<_, u8>(&mesh.solid).hash(&mut hash);
            bytemuck::cast_slice::<_, u8>(&mesh.translucent).hash(&mut hash);
        }
        checksum = hash.finish();
        best = best.min(spent);
    }
    println!(
        "meshed {} sections ({vertices} vertices, checksum {checksum:016x}) in {:.1} ms of CPU ({:.1} us per section)",
        regions.len(),
        best.as_secs_f64() * 1e3,
        best.as_secs_f64() * 1e6 / regions.len().max(1) as f64
    );
    Ok(())
}
