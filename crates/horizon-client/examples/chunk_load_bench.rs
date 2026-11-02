//! What a column costs to load, on real terrain: generates and lights the
//! columns around a point with the vanilla generator, as a world being explored
//! does, then meshes every section of the inner columns on this one thread, so a
//! change to generation or meshing shows as time per column and per section.
//!
//!     cargo run --release -p horizon-client --example chunk_load_bench -- \
//!         .local/horizon-client/26.3 [--seed 1234] [--center CX CZ] [--radius 8]
//!         [--threads N] [--rounds 3] [--mesh-threads N] [--json out.json]
//!
//! The default centre is the village of `tools/perf_bench.py`. The generation
//! figure is wall time with N workers (the game's own pool); the mesh figure is
//! this thread's CPU time in the workers' path (`Mesher::mesh_reusing` with a
//! recycled mesh and scratch, then `sort_solid`), each section's best of
//! `--rounds`, so other work on the machine changes it less; then the wall time
//! of `--mesh-threads` threads meshing them all at once. The checksum of the
//! meshes (vertices and visibility) shows that an A/B pair of builds made the
//! same ones.
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::HEIGHT,
    graphics::LeafCullingMode,
    light::LightTables,
    mesher::{MeshScratch, MeshTables, Mesher, Region, SectionMesh, TerrainVertex, borrow},
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
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
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
    let one = mesh_rounds(&context, seed, &regions, rounds)?;
    let mut sorted = one.per_section.clone();
    sorted.sort_unstable();
    let pct = |p: f64| {
        let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted.get(i).map_or(0.0, |d| d.as_secs_f64() * 1e6)
    };
    let total: Duration = one.per_section.iter().sum();
    let filled: Vec<Duration> = one
        .per_section
        .iter()
        .zip(&one.vertices_per_section)
        .filter(|&(_, &v)| v > 0)
        .map(|(d, _)| *d)
        .collect();
    let filled_total: Duration = filled.iter().sum();
    let bytes = one.vertices * std::mem::size_of::<TerrainVertex>();
    println!(
        "meshed {} sections ({} with geometry, {} vertices, {:.1} MB of vertices, checksum {:016x}) in {:.1} ms of CPU ({:.1} us per section)",
        regions.len(),
        filled.len(),
        one.vertices,
        bytes as f64 / 1e6,
        one.checksum,
        total.as_secs_f64() * 1e3,
        total.as_secs_f64() * 1e6 / regions.len().max(1) as f64
    );
    println!(
        "per section (best of {rounds} rounds, us): p50 {:.1} p90 {:.1} p99 {:.1} max {:.1}; with geometry: mean {:.1}; {:.0} sections/s per core",
        pct(0.5),
        pct(0.9),
        pct(0.99),
        pct(1.0),
        filled_total.as_secs_f64() * 1e6 / filled.len().max(1) as f64,
        regions.len() as f64 / total.as_secs_f64().max(1e-9),
    );
    // Every worker meshing at once: the throughput a world being explored gets,
    // memory bandwidth and the allocator included.
    let mesh_threads: usize = flag(&args, "--mesh-threads", threads);
    let parallel = mesh_parallel(&context, seed, &regions, mesh_threads)?;
    println!(
        "{mesh_threads} threads meshed {} sections in {:.1} ms of wall time: {:.0} sections/s",
        regions.len(),
        parallel.as_secs_f64() * 1e3,
        regions.len() as f64 / parallel.as_secs_f64().max(1e-9)
    );
    if let Some(path) = args
        .iter()
        .position(|a| a == "--json")
        .and_then(|i| args.get(i + 1))
    {
        let json = format!(
            concat!(
                "{{\"sections\":{},\"filled\":{},\"vertices\":{},\"vertex_bytes\":{},",
                "\"checksum\":\"{:016x}\",\"cpu_ms\":{:.3},\"p50_us\":{:.2},\"p90_us\":{:.2},",
                "\"p99_us\":{:.2},\"max_us\":{:.2},\"filled_mean_us\":{:.2},",
                "\"mesh_threads\":{},\"parallel_per_s\":{:.1}}}\n"
            ),
            regions.len(),
            filled.len(),
            one.vertices,
            bytes,
            one.checksum,
            total.as_secs_f64() * 1e3,
            pct(0.5),
            pct(0.9),
            pct(0.99),
            pct(1.0),
            filled_total.as_secs_f64() * 1e6 / filled.len().max(1) as f64,
            mesh_threads,
            regions.len() as f64 / parallel.as_secs_f64().max(1e-9),
        );
        std::fs::write(path, json)?;
    }
    Ok(())
}

fn mesher<'a>(context: &'a WorkerContext, seed: i64) -> Mesher<'a> {
    Mesher {
        nether_shading: false,
        shader_pack_lighting: false,
        zoom: BiomeZoom::new(obfuscate_seed(seed)),
        blend_radius: 2,
        smooth_lighting: true,
        cutout_leaves: true,
        translucent_layers: &[],
        leaf_culling: LeafCullingMode::Off,
        assets: &context.assets,
        render: &context.render,
        tables: &context.tables,
        biomes: &context.biomes,
        visibility: context.tables.visibility.clone(),
    }
}

struct Rounds {
    /// Each section's CPU time, the least of the rounds.
    per_section: Vec<Duration>,
    vertices_per_section: Vec<usize>,
    vertices: usize,
    checksum: u64,
}

/// Meshes every section `rounds` times on this thread the way a worker does:
/// one mesher (its face cache kept), one scratch, the mesh buffers recycled,
/// and the solid quads sorted by side.
fn mesh_rounds(
    context: &WorkerContext,
    seed: i64,
    regions: &[(Region, i32)],
    rounds: usize,
) -> Result<Rounds, Error> {
    let mut mesher = mesher(context, seed);
    let mut scratch = MeshScratch::default();
    let mut recycled = SectionMesh::default();
    let mut per_section = vec![Duration::MAX; regions.len()];
    let mut vertices_per_section = vec![0; regions.len()];
    let mut checksum = 0;
    for _ in 0..rounds.max(1) {
        let mut hash = DefaultHasher::new();
        for (i, (region, sy)) in regions.iter().enumerate() {
            let started = thread_cpu().ok_or("no thread CPU clock here")?;
            let mut mesh = mesher.mesh_reusing(&borrow(region), *sy, recycled, &mut scratch);
            mesh.sort_solid();
            let spent = thread_cpu().ok_or("no thread CPU clock here")? - started;
            per_section[i] = per_section[i].min(spent);
            vertices_per_section[i] = mesh.solid.len() + mesh.translucent.len();
            bytemuck::cast_slice::<_, u8>(&mesh.solid).hash(&mut hash);
            bytemuck::cast_slice::<_, u8>(&mesh.translucent).hash(&mut hash);
            mesh.visibility.hash(&mut hash);
            recycled = mesh;
        }
        checksum = hash.finish();
    }
    let vertices = vertices_per_section.iter().sum();
    Ok(Rounds {
        per_section,
        vertices_per_section,
        vertices,
        checksum,
    })
}

/// Wall time for `threads` workers to mesh every section once between them.
fn mesh_parallel(
    context: &WorkerContext,
    seed: i64,
    regions: &[(Region, i32)],
    threads: usize,
) -> Result<Duration, Error> {
    let next = AtomicUsize::new(0);
    let started = Instant::now();
    thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                let mut mesher = mesher(context, seed);
                let mut scratch = MeshScratch::default();
                let mut recycled = SectionMesh::default();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((region, sy)) = regions.get(i) else {
                        break;
                    };
                    let mut mesh =
                        mesher.mesh_reusing(&borrow(region), *sy, recycled, &mut scratch);
                    mesh.sort_solid();
                    recycled = mesh;
                }
            });
        }
    });
    Ok(started.elapsed())
}
