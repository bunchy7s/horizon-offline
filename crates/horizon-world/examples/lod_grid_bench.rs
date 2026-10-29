//! Far terrain sampling cost (`GeneratorWorker::lod_grid`): milliseconds per
//! far tile at each tile level, as the client's far terrain requests them
//! (16 x 16 columns), with checksums of what they hold.
//!
//! lod_grid_bench <client resource dir> [--seed S] [--tiles N] [--levels 0,2,4,6] [--dump PATH]
//!     [--approximate]
//!
//! `--approximate` samples as Reduced far detail does.
use horizon_world::worldgen::generator::{GeneratorData, OverworldGenerator};
use std::{path::PathBuf, time::Instant};

type Error = Box<dyn std::error::Error + Send + Sync>;

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let resources = PathBuf::from(args.next().ok_or("usage: lod_grid_bench <resources>")?);
    let (mut seed, mut tiles) = (0i64, 25);
    let mut levels = vec![0u8, 2, 4, 6];
    let mut dump: Option<std::fs::File> = None;
    let mut approximate = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => seed = args.next().ok_or("--seed S")?.parse()?,
            "--approximate" => approximate = true,
            "--tiles" => tiles = args.next().ok_or("--tiles N")?.parse()?,
            "--dump" => dump = Some(std::fs::File::create(args.next().ok_or("--dump PATH")?)?),
            "--levels" => {
                levels = args
                    .next()
                    .ok_or("--levels 0,2,4")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?
            }
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let registry = Box::leak(Box::new(horizon_world::level::data::load_registry(
        &resources,
    )?));
    let start = Instant::now();
    let data = GeneratorData::load(&resources.join("worldgen"), registry)?;
    let generator =
        OverworldGenerator::for_dimension(data, seed, "minecraft:normal", "minecraft:overworld")?;
    let mut worker = generator.worker();
    println!(
        "seed {seed}, generator ready in {:.1}s",
        start.elapsed().as_secs_f64()
    );
    // Warm the worker's caches once.
    worker.lod_grid(0, 0, 16, 16, approximate)?;
    for level in levels {
        let size = 16i32 << level;
        let width = size / 16;
        let start = Instant::now();
        let mut spans = 0;
        // Everything the tiles hold: an exact speedup keeps it.
        let mut hash = std::hash::DefaultHasher::new();
        // The terrain's shape alone (surface and water heights): an
        // approximation of materials or biomes keeps this one.
        let mut heights = std::hash::DefaultHasher::new();
        for i in 0..tiles {
            // Tiles around the origin, as a view would request them.
            let (tx, tz) = (i % 5 - 2, i / 5 - 2);
            let grid = worker.lod_grid(
                tx * size + width / 2,
                tz * size + width / 2,
                width,
                16,
                approximate,
            )?;
            spans += grid.iter().map(|(_, s)| s.len()).sum::<usize>();
            std::hash::Hash::hash(&format!("{grid:?}"), &mut hash);
            for (surface, _) in &grid {
                let shape = surface.as_ref().map(|s| (s.height, s.water.map(|w| w.0)));
                std::hash::Hash::hash(&shape, &mut heights);
                if let Some(file) = &mut dump {
                    use std::io::Write;
                    let s = surface.as_ref();
                    writeln!(
                        file,
                        "{level} {:?} {:?} {:?}",
                        s.map(|s| s.height),
                        s.map(|s| s.state.0),
                        s.map(|s| s.biome.0)
                    )?;
                }
            }
        }
        let ms = start.elapsed().as_secs_f64() * 1000. / tiles as f64;
        println!(
            "level {level} ({size} blocks, stride {width}): {ms:.1} ms/tile, {:.1} spans/column, output {:016x}, heights {:016x}",
            spans as f64 / (tiles * 256) as f64,
            std::hash::Hasher::finish(&hash),
            std::hash::Hasher::finish(&heights)
        );
    }
    Ok(())
}
