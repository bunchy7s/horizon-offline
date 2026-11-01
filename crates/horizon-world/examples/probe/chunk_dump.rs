//! Whole-chunk dump in the format of the vanilla chunk-generation oracle.
//!
//! chunk_dump <client resource dir> <seed> <dimension> <x0,z0,x1,z1>...
//! With HORIZON_SCAN set, one rectangle lists the structure starts in it.
//! HORIZON_DATAPACKS names a world's datapacks folder of folder packs, each
//! enabled after vanilla in name order (tools/test_datapack_worldgen_parity.py).
//! HORIZON_SAVED_STARTS names a JSON file of saved starts (`[{"id", "chunk":
//! [x, z], "pieces": [...]}]`, jigsaw pieces as a save lists them) that take
//! the place of the starts generation builds in their chunks.
//!
//! Chunks are generated through biomes, terrain and carvers, then decorated in
//! row-major order over the target rectangle grown by one chunk (or in the
//! order HORIZON_ORDER lists), each with its eight neighbours so feature spill
//! persists, exactly as the oracle does.
//! For every target chunk this prints `chunk X Z`, one `q QY NAME...` line per
//! quart layer (16 columns, z-major), one `y Y ID...` line per block row and
//! one `be X Y Z CANONICAL_NBT` line per block entity and one `t block|fluid
//! NAME X Y Z PRIORITY` line per scheduled tick.
use crate::support::common::tag;
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::BlockRegistry,
    worldgen::{
        biome::BiomeId,
        blending::{BlendingData, Direction8, OldChunk, OldWorld, Sides},
        generator::{GeneratorData, OverworldGenerator, ProtoChunk},
        overrides::{PackFile, WorldgenOverrides},
    },
};
use serde_json::Value;
use std::{collections::HashMap, fmt::Write as _, fs, io::Write as _, path::Path, sync::Arc};

type Error = Box<dyn std::error::Error + Send + Sync>;

fn registry(resources: &Path) -> Result<&'static BlockRegistry, Error> {
    let facts: Value = serde_json::from_str(&fs::read_to_string(resources.join("facts.json"))?)?;
    let states = facts["states"].as_array().ok_or("facts states")?;
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(states.iter().map(|v| tag(&v[4])).collect()),
        ),
        ("defaults", tag(&facts["defaults"])),
    ]))?;
    Ok(Box::leak(Box::new(registry)))
}

/// Type-prefixed scalars with sorted compound keys, as the oracle prints them.
fn canonical(tag: &Tag, out: &mut String) {
    match tag {
        Tag::End => {}
        Tag::Byte(v) => write!(out, "b{v}").unwrap(),
        Tag::Short(v) => write!(out, "s{v}").unwrap(),
        Tag::Int(v) => write!(out, "i{v}").unwrap(),
        Tag::Long(v) => write!(out, "l{v}").unwrap(),
        Tag::Float(v) => write!(out, "f{}", *v as i32).unwrap(),
        Tag::Double(v) => write!(out, "d{}", *v as i64).unwrap(),
        Tag::String(s) => {
            let text = s
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            write!(out, "\"{text}\"").unwrap();
        }
        Tag::ByteArray(v) => {
            out.push_str("B[");
            for b in v {
                write!(out, "{},", *b as i8).unwrap();
            }
            out.push(']');
        }
        Tag::IntArray(v) => {
            out.push_str("I[");
            for b in v {
                write!(out, "{b},").unwrap();
            }
            out.push(']');
        }
        Tag::LongArray(v) => {
            out.push_str("L[");
            for b in v {
                write!(out, "{b},").unwrap();
            }
            out.push(']');
        }
        Tag::List(v) => {
            out.push('[');
            for item in v {
                canonical(item, out);
                out.push(',');
            }
            out.push(']');
        }
        Tag::Compound(c) => {
            out.push('{');
            for (k, v) in c {
                write!(out, "{}=", k.to_string_lossy()).unwrap();
                canonical(v, out);
                out.push(',');
            }
            out.push('}');
        }
    }
}

/// HORIZON_DATAPACKS: a world's datapacks folder of folder packs, each enabled
/// after vanilla in name order, as the oracle selects them.
fn datapacks(dir: &Path) -> Result<WorldgenOverrides, Error> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(root, &path, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut packs: Vec<_> = fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("pack.mcmeta").exists())
        .collect();
    packs.sort();
    let mut overrides = WorldgenOverrides::default();
    for pack in packs {
        let id = format!(
            "file/{}",
            pack.file_name().unwrap_or_default().to_string_lossy()
        );
        let mut files = Vec::new();
        walk(&pack, &pack.join("data"), &mut files);
        for file in files {
            let read = || fs::read_to_string(pack.join(&file)).ok();
            if let PackFile::Unreadable = overrides.add_file(&id, &file, read) {
                return Err(format!("{id}: {file}: unreadable").into());
            }
        }
    }
    for warning in overrides.finish(|_| true) {
        eprintln!("data pack worldgen: {warning}");
    }
    Ok(overrides)
}

/// HORIZON_OLD_CHUNKS=SEED:x0,z0,x1,z1: chunks another seed generated that
/// count as saved by an older version (see the oracle).
struct OldChunks {
    keys: std::collections::HashSet<(i32, i32)>,
    chunks: HashMap<(i32, i32), ProtoChunk>,
    data: std::sync::Mutex<HashMap<(i32, i32), std::sync::Arc<BlendingData>>>,
}
struct OldView<'a>(&'a ProtoChunk, ChunkPos);
impl OldChunk for OldView<'_> {
    fn has_surface_heightmap(&self) -> bool {
        false
    }
    fn surface_height(&self, _: i32, _: i32) -> i32 {
        unreachable!("no worldgen heightmap")
    }
    fn is_surface_block(&self, x: i32, y: i32, z: i32) -> bool {
        matches!(
            self.0.block_name(self.1.x * 16 + x, y, self.1.z * 16 + z),
            "minecraft:podzol"
                | "minecraft:gravel"
                | "minecraft:grass_block"
                | "minecraft:stone"
                | "minecraft:coarse_dirt"
                | "minecraft:sand"
                | "minecraft:red_sand"
                | "minecraft:mycelium"
                | "minecraft:snow_block"
                | "minecraft:terracotta"
                | "minecraft:dirt"
        )
    }
    fn is_ground(&self, x: i32, y: i32, z: i32) -> bool {
        // A terrain chunk holds air, the two fluids and full solid blocks.
        !matches!(
            self.0.block_name(self.1.x * 16 + x, y, self.1.z * 16 + z),
            "minecraft:air"
                | "minecraft:cave_air"
                | "minecraft:void_air"
                | "minecraft:water"
                | "minecraft:lava"
        )
    }
    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId {
        self.0
            .biome_id_at(self.1.x * 16 + qx * 4, qy * 4, self.1.z * 16 + qz * 4)
    }
}
impl OldWorld for OldChunks {
    fn is_old_chunk_around(&self, center: ChunkPos, range: i32) -> bool {
        self.keys
            .iter()
            .any(|&(x, z)| (x - center.x).abs() <= range && (z - center.z).abs() <= range)
    }
    fn old_biomes(
        &self,
        x: i32,
        z: i32,
    ) -> Option<Arc<horizon_world::worldgen::biome_chunk::BiomeChunk<BiomeId>>> {
        self.chunks
            .get(&(x, z))
            .map(|c| Arc::new(c.biomes().clone()))
    }
    fn blending(&self, x: i32, z: i32) -> Option<std::sync::Arc<BlendingData>> {
        let chunk = self.chunks.get(&(x, z))?;
        let mut data = self.data.lock().unwrap();
        if let Some(found) = data.get(&(x, z)) {
            return Some(found.clone());
        }
        let mut sides = Sides::default();
        for side in Direction8::ALL {
            let (dx, dz) = side.step();
            if !self.keys.contains(&(x + dx, z + dz)) {
                sides.insert(side);
            }
        }
        let made = BlendingData::unpack(-4, 20, None)?
            .calculated(&OldView(chunk, ChunkPos::new(x, z)), sides);
        let made = std::sync::Arc::new(made);
        data.insert((x, z), made.clone());
        Some(made)
    }
}

pub fn main() -> Result<(), Error> {
    let mut args = crate::args().skip(1);
    let resources = std::path::PathBuf::from(args.next().ok_or("client resource directory")?);
    let seed: i64 = args.next().ok_or("seed")?.parse()?;
    let dimension = format!("minecraft:{}", args.next().ok_or("dimension")?);
    let rects = args
        .map(|rect| {
            let v: Vec<i32> = rect.split(',').map(str::parse).collect::<Result<_, _>>()?;
            let v: [i32; 4] = v.try_into().map_err(|_| "rectangle x0,z0,x1,z1")?;
            Ok(v)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let blocks = registry(&resources)?;
    // HORIZON_LIST_ORDER=x,z,view: print the chunks vanilla decorates around a
    // player standing in chunk x,z (`decoration_order::vanilla_order`) and exit.
    if let Some(at) = std::env::var_os("HORIZON_LIST_ORDER") {
        let v: Vec<i32> = at
            .to_string_lossy()
            .split(',')
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        let [x, z, view]: [i32; 3] = v.try_into().map_err(|_| "HORIZON_LIST_ORDER=x,z,view")?;
        for (x, z) in horizon_world::worldgen::decoration_order::vanilla_order((x, z), view) {
            println!("{x} {z}");
        }
        return Ok(());
    }
    let overrides = match std::env::var_os("HORIZON_DATAPACKS") {
        Some(dir) => datapacks(Path::new(&dir))?,
        None => WorldgenOverrides::default(),
    };
    let data = GeneratorData::load_with(&resources.join("worldgen"), blocks, &overrides)?;
    for warning in data.warnings() {
        eprintln!("data pack worldgen: {warning}");
    }
    let generator =
        OverworldGenerator::for_dimension(data.clone(), seed, "minecraft:normal", &dimension)?;
    let mut worker = generator.worker();
    let old_spec = std::env::var("HORIZON_OLD_CHUNKS").ok();
    let old_rect = match &old_spec {
        Some(spec) => {
            let (_, rect) = spec
                .split_once(':')
                .ok_or("HORIZON_OLD_CHUNKS=SEED:x0,z0,x1,z1")?;
            let v: Vec<i32> = rect.split(',').map(str::parse).collect::<Result<_, _>>()?;
            Some(<[i32; 4]>::try_from(v).map_err(|_| "old rectangle")?)
        }
        None => None,
    };
    // HORIZON_PHASE: terrain (no decoration), structures (pieces only) or full.
    let phase = std::env::var("HORIZON_PHASE").unwrap_or_else(|_| "full".into());
    worker.set_structures_only(phase == "structures");
    if std::env::var_os("HORIZON_SCAN").is_some() {
        let [x0, z0, x1, z1] = rects[0];
        for x in x0..=x1 {
            for z in z0..=z1 {
                for s in worker.structure_starts(ChunkPos::new(x, z)) {
                    if s.chunk == ChunkPos::new(x, z) {
                        if std::env::var_os("HORIZON_PIECES").is_some() {
                            for piece in &s.pieces {
                                let b = piece.bounding_box();
                                println!(
                                    "piece {x} {z} {} {} {} {} {} {} {}",
                                    piece.type_name(),
                                    b.min[0],
                                    b.min[1],
                                    b.min[2],
                                    b.max[0],
                                    b.max[1],
                                    b.max[2]
                                );
                            }
                        }
                        println!(
                            "start {} {x} {z} {} {} {} {} {} {}",
                            s.name,
                            s.bounding_box.min[0],
                            s.bounding_box.min[1],
                            s.bounding_box.min[2],
                            s.bounding_box.max[0],
                            s.bounding_box.max[1],
                            s.bounding_box.max[2]
                        );
                    }
                }
            }
        }
        return Ok(());
    }
    let saved: Vec<Value> = match std::env::var_os("HORIZON_SAVED_STARTS") {
        Some(path) => serde_json::from_str(&fs::read_to_string(path)?)?,
        None => Vec::new(),
    };
    for [x0, z0, x1, z1] in rects {
        // Each rectangle starts from scratch, like the oracle's fresh chunks:
        // structure pieces keep placement state (a shipwreck's adjusted height).
        worker.forget_structure_starts();
        for start in &saved {
            let chunk = ChunkPos::new(
                start["chunk"][0].as_i64().ok_or("saved start chunk")? as i32,
                start["chunk"][1].as_i64().ok_or("saved start chunk")? as i32,
            );
            let pieces = start["pieces"].as_array().ok_or("saved start pieces")?;
            let id = start["id"].as_str().ok_or("saved start id")?;
            worker.use_saved_start(chunk, id, pieces)?;
        }
        let mut chunks: HashMap<(i32, i32), ProtoChunk> = HashMap::new();
        if let (Some(spec), Some([ox0, oz0, ox1, oz1])) = (&old_spec, old_rect) {
            // The old chunks: another seed's terrain, without structures.
            let old_seed: i64 = spec.split(':').next().ok_or("old seed")?.parse()?;
            let old_generator = OverworldGenerator::without_structures(data.clone(), old_seed)?;
            let mut old_worker = old_generator.worker();
            let mut made = HashMap::new();
            for z in oz0..=oz1 {
                for x in ox0..=ox1 {
                    made.insert((x, z), old_worker.generate_proto(ChunkPos::new(x, z))?);
                }
            }
            let keys = made.keys().copied().collect();
            let old = std::sync::Arc::new(OldChunks {
                keys,
                chunks: made,
                data: Default::default(),
            });
            generator.set_old_world(old.clone());
            for &key in old.chunks.keys() {
                // Each rectangle starts from scratch: the probe's own copy.
                let again = old_worker.generate_proto(ChunkPos::new(key.0, key.1))?;
                chunks.insert(key, again);
            }
        }
        for z in z0 - 2..=z1 + 2 {
            for x in x0 - 2..=x1 + 2 {
                if chunks.contains_key(&(x, z)) {
                    continue;
                }
                chunks.insert((x, z), worker.generate_proto(ChunkPos::new(x, z))?);
            }
        }
        let mut failed = 0;
        let margin = i32::from(std::env::var_os("HORIZON_ISOLATED").is_none());
        // HORIZON_ORDER: a file of `x z` lines, the chunks to decorate in that
        // order (the oracle reads the same file); row-major by default.
        let order: Vec<(i32, i32)> = match std::env::var_os("HORIZON_ORDER") {
            Some(path) => fs::read_to_string(path)?
                .lines()
                .filter_map(|l| {
                    let mut t = l.split_whitespace().map(str::parse::<i32>);
                    Some((t.next()?.ok()?, t.next()?.ok()?))
                })
                .collect(),
            None => (z0 - margin..=z1 + margin)
                .flat_map(|z| (x0 - margin..=x1 + margin).map(move |x| (x, z)))
                .collect(),
        };
        for (x, z) in order {
            // An old chunk is not decorated, only written into.
            if old_rect.is_some_and(|[ox0, oz0, ox1, oz1]| {
                (ox0..=ox1).contains(&x) && (oz0..=oz1).contains(&z)
            }) {
                continue;
            }
            let mut region = Vec::with_capacity(9);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    region.push(chunks.remove(&(x + dx, z + dz)).ok_or("missing chunk")?);
                }
            }
            let mut region: [ProtoChunk; 9] = region.try_into().map_err(|_| "region size")?;
            let stats = if phase == "terrain" {
                Default::default()
            } else {
                worker.decorate(&mut region)?
            };
            if stats.failed > 0 || stats.unsupported > 0 {
                failed += 1;
                eprintln!(
                    "chunk {x} {z}: {} failed, {} unsupported {:?}",
                    stats.failed, stats.unsupported, stats.skipped
                );
            }
            for (i, chunk) in region.into_iter().enumerate() {
                chunks.insert((x + i as i32 % 3 - 1, z + i as i32 / 3 - 1), chunk);
            }
        }
        if failed > 0 {
            eprintln!("{failed} chunks had failed or unsupported features");
        }
        let stdout = std::io::stdout();
        let mut w = std::io::BufWriter::new(stdout.lock());
        for z in z0..=z1 {
            for x in x0..=x1 {
                let chunk = &chunks[&(x, z)];
                writeln!(w, "chunk {x} {z}")?;
                let (min, height) = (chunk.terrain.min_y(), chunk.terrain.height());
                for qy in min >> 2..=(min + height - 1) >> 2 {
                    let mut line = format!("q {qy}");
                    for qz in 0..4 {
                        for qx in 0..4 {
                            line.push(' ');
                            line.push_str(chunk.biome_at(x * 16 + qx * 4, qy * 4, z * 16 + qz * 4));
                        }
                    }
                    writeln!(w, "{line}")?;
                }
                for y in min..min + height {
                    let mut line = format!("y {y}");
                    for zz in 0..16 {
                        for xx in 0..16 {
                            write!(line, " {}", chunk.block(x * 16 + xx, y, z * 16 + zz).0)?;
                        }
                    }
                    writeln!(w, "{line}")?;
                }
                let mut entities: Vec<_> = chunk.block_entities.iter().collect();
                entities.sort_by_key(|(p, _)| *p);
                for ([bx, by, bz], nbt) in entities {
                    let mut text = String::new();
                    canonical(&Tag::Compound(nbt.clone()), &mut text);
                    writeln!(w, "be {bx} {by} {bz}\t{text}")?;
                }
                // Scheduled ticks, in scheduling order (the order they run in).
                for t in chunk.terrain.block_ticks().pack(0) {
                    let name = blocks
                        .state(horizon_world::registry::BlockStateId(t.kind.0))
                        .map_or("?", |s| s.name.as_str());
                    let p = t.pos;
                    writeln!(
                        w,
                        "t block {name} {} {} {} {}",
                        p.x, p.y, p.z, t.priority as i32
                    )?;
                }
                for t in chunk.terrain.fluid_ticks().pack(0) {
                    let name = [
                        "minecraft:empty",
                        "minecraft:flowing_water",
                        "minecraft:water",
                        "minecraft:flowing_lava",
                        "minecraft:lava",
                    ]
                    .get(t.kind.0 as usize)
                    .copied()
                    .unwrap_or("?");
                    let p = t.pos;
                    writeln!(
                        w,
                        "t fluid {name} {} {} {} {}",
                        p.x, p.y, p.z, t.priority as i32
                    )?;
                }
                // Postprocessing marks, per section in list order.
                let base = chunk.terrain.min_y() >> 4;
                for (i, marks) in chunk.terrain.postprocessing().iter().enumerate() {
                    for &packed in marks {
                        let p = i32::from(packed);
                        writeln!(
                            w,
                            "p {} {} {}",
                            x * 16 + (p & 15),
                            (base + i as i32) * 16 + ((p >> 4) & 15),
                            z * 16 + ((p >> 8) & 15)
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}
