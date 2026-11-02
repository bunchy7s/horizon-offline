//! Generation and light throughput of the game's worker pool, and how it
//! scales with the number of workers: the columns around a point go through
//! the whole pipeline (proto chunk, decoration, final column, light) as when a
//! world loads, once per worker count, each on a freshly seeded generator.
//!
//!     cargo run --release -p horizon-client-lib --example gen_light_bench -- \
//!         .local/horizon-client/26.3 [--seed 1234] [--center CX CZ] [--radius 12]
//!         [--threads 1,2,4,8] [--period-ms 2] [--loading] [--repeat 1] [--json]
//!
//! For each run it prints the wall time until every column within the radius
//! (the area Streaming finishes) was lit, columns finished and lit per second, the process's CPU seconds per
//! column, the coordinator's (`Streaming::update`) share, and the workers' CPU
//! time by job kind (`streaming::job_stats`). The shared worker pool is sized
//! once per process (`HORIZON_JOB_THREADS`), so each worker count runs in a
//! process of its own, started by this one. The checksum
//! covers the blocks and light of the columns within the radius: an A/B pair of
//! builds, and every worker count, must print the same one.
//! `--period-ms` is how long the coordinator sleeps between updates (the
//! server thread runs one per 50 ms tick and pumps between ticks); `--loading`
//! orders work ring by ring, as under the level loading screen.
//!
//! `--simulate PROTO,DECORATE,FINISH` (milliseconds) swaps the generator for
//! one whose jobs sleep that long and make empty columns: what is left is the
//! pipeline itself (the coordinator, the in-flight limits, decoration order,
//! neighbour waits). Sleeping workers don't compete for cores, so this shows
//! how the pipeline scales to many workers even on a busy machine.
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::Chunk,
    light::LightTables,
    mesher::MeshTables,
    models,
    spans::thread_cpu,
    streaming::{ChunkGenerator, Proto, Streaming, WorkerContext, job_stats},
    vanillagen::VanillaTerrain,
    world::World,
};
use horizon_world::registry::BlockStateId;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    process::Command,
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

fn process_cpu() -> Duration {
    #[cfg(unix)]
    {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
        Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
    }
    #[cfg(not(unix))]
    Duration::ZERO
}

/// Jobs that take a fixed time and make empty columns (`--simulate`).
struct Simulated {
    proto: Duration,
    decorate: Duration,
    finish: Duration,
}
impl ChunkGenerator for Simulated {
    type Worker = ();
    fn worker(&self) {}
    fn proto(&self, _worker: &mut (), x: i32, z: i32) -> Proto {
        thread::sleep(self.proto);
        Box::new((x, z))
    }
    fn decorates(&self) -> bool {
        true
    }
    fn decorate(&self, _worker: &mut (), _region: &mut [Proto; 9]) {
        thread::sleep(self.decorate);
    }
    fn finish(&self, proto: Proto) -> Chunk {
        thread::sleep(self.finish);
        let (x, z) = *proto.downcast::<(i32, i32)>().expect("simulated proto");
        Chunk::empty(x, z)
    }
    fn light_view(&self, proto: &Proto) -> Option<Chunk> {
        let &(x, z) = proto.downcast_ref::<(i32, i32)>()?;
        Some(Chunk::empty(x, z))
    }
}

struct Run {
    wall: f64,
    columns: usize,
    lit: usize,
    cpu: f64,
    coordinator: f64,
    updates: usize,
    jobs: Vec<(&'static str, u64, u64)>,
    checksum: u64,
    half: f64,
}

#[allow(clippy::too_many_arguments)]
fn run<G: ChunkGenerator>(
    generator: G,
    context: &Arc<WorkerContext>,
    center: (i32, i32),
    radius: i32,
    threads: usize,
    period: Duration,
    loading: bool,
) -> Result<Run, Error> {
    let mut streaming = Streaming::new(Arc::new(generator), context.clone(), threads);
    streaming.mesh = false;
    streaming.center = center;
    streaming.radius = radius;
    streaming.loading = loading;
    let mut world = World::new(BlockStateId(0), 0);
    let view: Vec<(i32, i32)> = (center.0 - radius - 1..=center.0 + radius + 1)
        .flat_map(|x| (center.1 - radius - 1..=center.1 + radius + 1).map(move |z| (x, z)))
        // The columns Streaming finishes (`Ranges::in_main_range` with a
        // margin of 1). `in_view_distance` reaches a few columns past them.
        .filter(|&(x, z)| {
            let (dx, dz) = (i64::from(x - center.0), i64::from(z - center.1));
            dx * dx + dz * dz <= i64::from(radius + 1).pow(2)
        })
        .collect();
    job_stats::enable(true);
    job_stats::take();
    let (cpu0, started) = (process_cpu(), Instant::now());
    let mut coordinator = Duration::ZERO;
    let mut updates = 0;
    let mut half = None;
    let trace = std::env::var_os("GEN_LIGHT_TRACE").is_some();
    let mut next_trace = Duration::from_secs(2);
    loop {
        let t = thread_cpu().unwrap_or_default();
        let _ = streaming.update(&mut world);
        coordinator += thread_cpu().unwrap_or_default().saturating_sub(t);
        updates += 1;
        let lit = view
            .iter()
            .filter(|k| world.chunks.get(k).is_some_and(|c| c.lit))
            .count();
        if half.is_none() && lit * 2 >= view.len() {
            half = Some(started.elapsed().as_secs_f64());
        }
        if lit == view.len() {
            break;
        }
        if trace && started.elapsed() >= next_trace {
            next_trace += Duration::from_secs(2);
            let missing: Vec<_> = view
                .iter()
                .filter(|k| !world.chunks.get(k).is_some_and(|c| c.lit))
                .take(4)
                .collect();
            eprintln!(
                "  {:.0} s: {lit}/{} lit, {} columns, {:?}, waiting e.g. {missing:?}",
                started.elapsed().as_secs_f64(),
                view.len(),
                world.chunks.len(),
                streaming.stages()
            );
        }
        if started.elapsed() > Duration::from_secs(1800) {
            return Err("timed out".into());
        }
        streaming.wait(period);
    }
    let wall = started.elapsed().as_secs_f64();
    let cpu = (process_cpu() - cpu0).as_secs_f64();
    let jobs = job_stats::take();
    job_stats::enable(false);
    let mut hash = DefaultHasher::new();
    let mut ids = Box::new([0u32; 4096]);
    for key in &view {
        let chunk = &world.chunks[key];
        key.hash(&mut hash);
        for section in &chunk.sections {
            section.unpack(&mut ids);
            ids.hash(&mut hash);
            (0..4096)
                .map(|i| section.light.get(i))
                .for_each(|v| v.hash(&mut hash));
        }
    }
    let lit = world.chunks.values().filter(|c| c.lit).count();
    Ok(Run {
        wall,
        columns: world.chunks.len(),
        lit,
        cpu,
        coordinator: coordinator.as_secs_f64(),
        updates,
        jobs,
        checksum: hash.finish(),
        half: half.unwrap_or(wall),
    })
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(1).ok_or("usage: gen_light_bench <resources>")?);
    let seed: i64 = flag(&args, "--seed", 1234);
    let radius: i32 = flag(&args, "--radius", 12);
    let repeat: usize = flag(&args, "--repeat", 1);
    let period = Duration::from_millis(flag(&args, "--period-ms", 2));
    let loading = args.iter().any(|a| a == "--loading");
    let json = args.iter().any(|a| a == "--json");
    let simulate: Option<Vec<f64>> = args
        .iter()
        .position(|a| a == "--simulate")
        .map(|i| -> Result<Vec<f64>, Error> {
            let ms: Vec<f64> = args
                .get(i + 1)
                .ok_or("--simulate PROTO,DECORATE,FINISH")?
                .split(',')
                .map(str::parse)
                .collect::<Result<_, _>>()?;
            (ms.len() == 3)
                .then_some(ms)
                .ok_or_else(|| "--simulate PROTO,DECORATE,FINISH".into())
        })
        .transpose()?;
    let threads: Vec<usize> = flag(&args, "--threads", String::from("1,2,4,8"))
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()?;
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
    // The pool is made once per process with its size: one process per
    // worker count and repeat, each given the same flags but one count.
    if threads.len() > 1 || repeat > 1 {
        let exe = std::env::current_exe()?;
        let mut rest = Vec::new();
        let mut i = 1;
        while i < args.len() {
            if args[i] == "--threads" || args[i] == "--repeat" {
                i += 2;
                continue;
            }
            rest.push(args[i].clone());
            i += 1;
        }
        for _ in 0..repeat.max(1) {
            for &n in &threads {
                let status = Command::new(&exe)
                    .args(&rest)
                    .args(["--threads", &n.to_string()])
                    .env("HORIZON_JOB_THREADS", n.to_string())
                    .status()?;
                if !status.success() {
                    return Err(format!("the run with {n} workers failed: {status}").into());
                }
            }
        }
        return Ok(());
    }
    let n = threads[0];
    horizon_threads::jobs::configure(n);

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
    let terrain = || VanillaTerrain::new(&root.join("worldgen"), assets.registry, &biomes, seed);
    let go = |center, radius, n, loading| -> Result<Run, Error> {
        match &simulate {
            Some(ms) => {
                let ms = |i: usize| Duration::from_secs_f64(ms[i] / 1e3);
                let generator = Simulated {
                    proto: ms(0),
                    decorate: ms(1),
                    finish: ms(2),
                };
                run(generator, &context, center, radius, n, period, loading)
            }
            None => run(terrain()?, &context, center, radius, n, period, loading),
        }
    };
    // Warm the shared generator data and the code away from the measured area.
    go((center.0 + 4000, center.1), 2, n, false)?;
    let pool = horizon_threads::jobs::pool().threads();
    if pool != n {
        eprintln!("the worker pool has {pool} threads, not {n} (HORIZON_JOB_THREADS?)");
    }
    let r = go(center, radius, n, loading)?;
    let busy: f64 = r.jobs.iter().map(|j| j.1 as f64).sum::<f64>() / 1e9;
    println!(
        "workers {n:2}: {:.2} s ({:.2} s to half), {} columns ({} lit), {:.1} columns/s, {:.1} lit/s, \
         {:.1} ms CPU/column, coordinator {:.2} s CPU in {} updates, workers busy {:.0}%, checksum {:016x}",
        r.wall,
        r.half,
        r.columns,
        r.lit,
        r.columns as f64 / r.wall,
        r.lit as f64 / r.wall,
        r.cpu * 1e3 / r.columns.max(1) as f64,
        r.coordinator,
        r.updates,
        busy * 100.0 / (r.wall * n as f64),
        r.checksum,
    );
    let line: Vec<String> = r
        .jobs
        .iter()
        .filter(|j| j.2 > 0)
        .map(|(kind, ns, count)| {
            format!(
                "{kind} {count} x {:.2} ms = {:.2} s",
                *ns as f64 / 1e6 / *count as f64,
                *ns as f64 / 1e9
            )
        })
        .collect();
    println!("    {}", line.join(", "));
    if json {
        println!(
            "BENCH {}",
            serde_json::json!({
                "workers": n, "wall_s": r.wall, "half_s": r.half, "columns": r.columns,
                "lit": r.lit, "cpu_s": r.cpu, "coordinator_s": r.coordinator,
                "updates": r.updates, "checksum": format!("{:016x}", r.checksum),
                "jobs": r.jobs.iter().map(|(k, ns, c)| serde_json::json!({"kind": k, "s": *ns as f64 / 1e9, "count": c})).collect::<Vec<_>>(),
            })
        );
    }
    Ok(())
}
