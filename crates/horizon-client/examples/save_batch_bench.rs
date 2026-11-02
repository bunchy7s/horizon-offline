//! Synthetic village/storage save batches, using the production snapshot and
//! full chunk NBT encoder. Run with `cargo run --profile fast -p horizon-client
//! --example save_batch_bench`. Add `-- --nbt` to also time serialization.
//! Exact NBT equality is always validated. No GPU, compression or disk I/O timed.
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    biome::Biomes,
    chunk::{Chunk, block_strategy},
    containers::BlockEntity,
    items::ItemStack,
    save::{self, ChunkExtras},
};
use horizon_core::paletted::PalettedContainer;
use horizon_world::{
    DATA_VERSION,
    nbt::{self, Limits, Tag},
    registry::BlockRegistry,
};
use std::{collections::HashMap, hint::black_box, sync::Arc, time::Instant};

type Entities = HashMap<(i32, i32, i32), BlockEntity>;
type Names = HashMap<(i32, i32, i32), String>;
const BLOCKS: [&str; 5] = [
    "minecraft:air",
    "minecraft:stone",
    "minecraft:chest",
    "minecraft:furnace",
    "minecraft:oak_sign",
];

// The pre-change with_entities path, with the same Chunk clone and outputs.
fn baseline(chunks: &[Arc<Chunk>], entities: &Entities, names: &Names) -> Vec<Chunk> {
    chunks
        .iter()
        .map(|chunk| {
            let mut c = (**chunk).clone();
            c.block_entities = entities
                .iter()
                .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
                .map(|(p, e)| (*p, e.clone()))
                .collect();
            c.block_names = names
                .iter()
                .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
                .map(|(p, n)| (*p, n.clone()))
                .collect();
            c
        })
        .collect()
}

fn fixture(per_chunk: usize) -> (Vec<Arc<Chunk>>, Entities, Names) {
    let mut chunks = Vec::new();
    let mut entities = Entities::new();
    let mut names = Names::new();
    let stack = ItemStack {
        item: "minecraft:stone".into(),
        count: 64,
        damage: 0,
        extra: horizon_client::item_data::decode_components(Some(&Tag::compound([(
            "minecraft:custom_data",
            Tag::compound([("fixture", Tag::String("save batch".into()))]),
        )])))
        .1,
    };
    let mut template = Chunk::empty(0, 0);
    for s in &mut template.sections[..8] {
        Arc::make_mut(s).blocks = PalettedContainer::new(block_strategy(), 1);
    }
    for i in 0..256 {
        let x = i % 16 - 8;
        let z = i / 16 - 8;
        let mut c = template.clone();
        c.x = x;
        c.z = z;
        let mut extras = ChunkExtras::default();
        extras.root.insert("InhabitedTime".into(), Tag::Long(12000));
        for j in 0..per_chunk {
            let p = (
                x * 16 + (j % 16) as i32,
                64 + (j / 256) as i32,
                z * 16 + ((j / 16) % 16) as i32,
            );
            let (block, mut e) = match j % 4 {
                0 => (4, BlockEntity::for_block(BLOCKS[4]).unwrap()),
                1 => (3, BlockEntity::for_block(BLOCKS[3]).unwrap()),
                _ => (2, BlockEntity::Chest(vec![Some(stack.clone()); 27])),
            };
            if let BlockEntity::Furnace(f) = &mut e {
                f.items[0] = Some(stack.clone());
                f.lit_remaining = 120;
                f.cooking_timer = 45;
            }
            if let BlockEntity::Sign(s) = &mut e {
                s.front.lines[0] = "Village storage".into();
            }
            c.set(p.0 & 15, p.1, p.2 & 15, block);
            if j % 4 == 2 {
                names.insert(p, format!("Storage {x},{z}/{j}"));
            }
            // Exercise the existing NBT overlay and saved list order as well.
            if j % 8 == 2 {
                let mut raw = save::encode_block_entity(p, &e, BLOCKS[block as usize]);
                if let Tag::Compound(map) = &mut raw {
                    map.insert(
                        "LootTable".into(),
                        Tag::String("minecraft:chests/simple_dungeon".into()),
                    );
                }
                extras.block_entity_sources.push((p, raw));
            }
            extras.block_entity_order.push(p);
            entities.insert(p, e);
        }
        // An unmodelled block entity, retained from the original chunk.
        extras.block_entities.push(Tag::compound([
            ("id", Tag::String("minecraft:end_gateway".into())),
            ("x", Tag::Int(x * 16)),
            ("y", Tag::Int(80)),
            ("z", Tag::Int(z * 16)),
            ("Age", Tag::Long(200)),
        ]));
        c.saved = Some(Arc::new(extras));
        chunks.push(Arc::new(c));
    }
    (chunks, entities, names)
}

fn encoding_fixture() -> horizon_client::Result<(BlockRegistry, Biomes)> {
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(BLOCKS.iter().map(|n| Tag::String((*n).into())).collect()),
        ),
        (
            "defaults",
            Tag::compound(
                BLOCKS
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (*n, Tag::Int(i as i32))),
            ),
        ),
    ]))?;
    let temp = std::env::temp_dir().join(format!("horizon-save-batch-{}", std::process::id()));
    let result = (|| {
        let textures = temp.join("assets/minecraft/textures/colormap");
        let biomes = temp.join("data/minecraft/worldgen/biome");
        std::fs::create_dir_all(&textures)?;
        std::fs::create_dir_all(&biomes)?;
        for name in ["grass", "foliage", "dry_foliage"] {
            let mut encoder = png::Encoder::new(
                std::fs::File::create(textures.join(format!("{name}.png")))?,
                1,
                1,
            );
            encoder.set_color(png::ColorType::Rgb);
            encoder.write_header()?.write_image_data(&[0, 128, 0])?;
        }
        std::fs::write(biomes.join("plains.json"), b"{}")?;
        Biomes::load(&temp)
    })();
    std::fs::remove_dir_all(&temp)?;
    Ok((registry, result?))
}

fn serialize(chunks: &[Chunk], registry: &BlockRegistry, biomes: &Biomes) -> Vec<Vec<u8>> {
    chunks
        .iter()
        .map(|c| nbt::write_named(&save::encode(c, registry, biomes), Limits::default()).unwrap())
        .collect()
}

fn measure(mut f: impl FnMut(), repetitions: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..repetitions {
        f();
    }
    start.elapsed().as_secs_f64() * 1e6 / repetitions as f64
}

fn compare(mut before: impl FnMut(), mut after: impl FnMut()) -> (f64, f64, usize) {
    for _ in 0..5 {
        before();
        after();
    }
    let estimate = measure(&mut before, 1).max(measure(&mut after, 1));
    let repetitions = (10_000.0 / estimate.max(0.01)).ceil().clamp(1.0, 10_000.0) as usize;
    let mut old = Vec::new();
    let mut new = Vec::new();
    for i in 0..21 {
        if i % 2 == 0 {
            old.push(measure(&mut before, repetitions));
            new.push(measure(&mut after, repetitions));
        } else {
            new.push(measure(&mut after, repetitions));
            old.push(measure(&mut before, repetitions));
        }
    }
    old.sort_by(f64::total_cmp);
    new.sort_by(f64::total_cmp);
    (old[10], new[10], repetitions)
}

fn main() -> horizon_client::Result<()> {
    let include_nbt = std::env::args().any(|arg| arg == "--nbt");
    let (registry, biomes) = encoding_fixture()?;
    println!(
        "stage,fixture,chunks,world_entities,world_names,saved_entities,bytes,before_us,after_us,speedup,samples,reps"
    );
    for (name, density) in [("village", 12), ("storage", 48)] {
        let (chunks, entities, names) = fixture(density);
        for count in [0, 1, 16, 64, 256] {
            let batch = &chunks[..count];
            let before = baseline(batch, &entities, &names);
            let after = save::chunks_with_block_entities(batch, &entities, &names);
            for (a, b) in before.iter().zip(&after) {
                assert_eq!(a.block_names, b.block_names);
            }
            let encoded = serialize(&before, &registry, &biomes);
            assert_eq!(
                encoded,
                serialize(&after, &registry, &biomes),
                "exact NBT bytes: {name}/{count}"
            );
            let bytes: usize = encoded.iter().map(Vec::len).sum();
            for encode in [false, true] {
                if encode && !include_nbt {
                    continue;
                }
                let run = |optimized| {
                    let snapshots = if optimized {
                        save::chunks_with_block_entities(
                            black_box(batch),
                            black_box(&entities),
                            black_box(&names),
                        )
                    } else {
                        baseline(black_box(batch), black_box(&entities), black_box(&names))
                    };
                    if encode {
                        black_box(serialize(black_box(&snapshots), &registry, &biomes));
                    }
                    black_box(snapshots);
                };
                let (old, new, reps) = compare(|| run(false), || run(true));
                println!(
                    "{},{name},{count},{},{},{},{bytes},{old:.3},{new:.3},{:.3},21,{reps}",
                    if encode { "snapshot+nbt" } else { "snapshot" },
                    entities.len(),
                    names.len(),
                    count * density,
                    old / new
                );
            }
        }
    }
    Ok(())
}
