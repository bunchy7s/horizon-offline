//! Headless transport for the section-mesh parity gate: builds the requested
//! scenes in real client chunks and writes the vertices `Mesher::mesh` emits in
//! the oracle's canonical form (vanilla ARGB colours and packed light coords).
#![forbid(unsafe_code)]
use horizon_client::{
    assets::{Assets, RenderAssets},
    biome::{BiomeInfo, Biomes, GrassModifier},
    blocks::BlockFacts,
    chunk::{Chunk, Light, MIN_Y, index},
    mesher::{EMISSION_BITS, FLUID_FACE_BITS, MeshTables, Mesher, OPAQUE_LAYER, TerrainVertex},
    models,
};
use horizon_core::biome::BiomeZoom;
use horizon_world::registry::BlockStateId;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn int(value: &Value) -> Result<i32> {
    Ok(i32::try_from(value.as_i64().ok_or("expected integer")?)?)
}
/// Horizon packs RGBA little endian (red in the low byte); vanilla uses ARGB.
fn argb(color: u32) -> i32 {
    let [r, g, b, a] = color.to_le_bytes();
    i32::from_be_bytes([a, r, g, b])
}
/// Horizon keeps sky*16 in the low byte and block*16 above it; vanilla's light
/// coords keep block in the low half-word and sky in the high one.
fn light_coords(light: u16) -> i32 {
    let light = u32::from(light);
    ((light & 0xFF) << 16 | (light >> 8 & 0xFF)) as i32
}
/// Scenes cover the 3x3 chunk region around chunk (0, 0).
fn chunk_index(x: i32, z: i32) -> Result<usize> {
    let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
    if !(-1..=1).contains(&cx) || !(-1..=1).contains(&cz) {
        return Err("scene block outside the 3x3 chunk region".into());
    }
    Ok(((cz + 1) * 3 + cx + 1) as usize)
}
fn scene(
    row: &Value,
    assets: &Assets,
    render: &RenderAssets,
    tables: &MeshTables,
    biomes: &mut Biomes,
    pristine: &[BiomeInfo],
    names: &BTreeMap<u32, &str>,
    elapsed: &mut Duration,
) -> Result<Value> {
    let colors = &row["colors"];
    let color = |k: &str| -> Result<u32> { Ok(int(&colors[k])? as u32 & 0xFF_FFFF) };
    // Scenes without a biome field give every biome the same constant colours;
    // the others use the real biome definitions.
    biomes.list = pristine.to_vec();
    if row.get("biomes").is_none() {
        for biome in &mut biomes.list {
            biome.grass = color("grass")?;
            biome.modifier = GrassModifier::None;
            biome.foliage = color("foliage")?;
            biome.dry_foliage = color("dry_foliage")?;
            biome.water = color("water")?;
        }
    }
    let default = row["light"].as_array().ok_or("missing default light")?;
    let packed = |sky: i32, block: i32| (sky << 4 | block) as u8;
    let default = packed(int(&default[0])?, int(&default[1])?);
    let mut chunks: Vec<Chunk> = (0..9)
        .map(|i| {
            let mut chunk = Chunk::empty(i % 3 - 1, i / 3 - 1);
            for section in &mut chunk.sections {
                Arc::make_mut(section).light = Light::Uniform(default);
            }
            chunk
        })
        .collect();
    for block in row["blocks"].as_array().ok_or("missing blocks")? {
        let (x, y, z) = (int(&block[0])?, int(&block[1])?, int(&block[2])?);
        let i = chunk_index(x, z)?;
        chunks[i].set(x & 15, y, z & 15, u32::try_from(int(&block[3])?)?);
    }
    for cell in row["lights"].as_array().ok_or("missing lights")? {
        let (x, y, z) = (int(&cell[0])?, int(&cell[1])?, int(&cell[2])?);
        let i = chunk_index(x, z)?;
        let s = ((y - MIN_Y) >> 4) as usize;
        let section = Arc::make_mut(&mut chunks[i].sections[s]);
        if let Light::Uniform(v) = section.light {
            section.light = Light::Full(Box::new([v; 4096]));
        }
        if let Light::Full(values) = &mut section.light {
            values[index(x, y, z)] = packed(int(&cell[3])?, int(&cell[4])?);
        }
    }
    let (mut zoom, mut blend_radius) = (BiomeZoom::new(0), 2);
    let smooth_lighting = row["smooth"].as_bool().unwrap_or(true);
    let cutout_leaves = row["cutout_leaves"].as_bool().unwrap_or(true);
    if let Some(field) = row.get("biomes") {
        zoom = BiomeZoom::new(field["seed"].as_i64().ok_or("missing biome seed")?);
        blend_radius = int(&field["radius"])?;
        let default = biomes.id(field["default"].as_str().ok_or("missing default biome")?);
        for chunk in &mut chunks {
            for qy in 0..96 {
                for cell in 0..16 {
                    chunk.set_biome_cell(cell & 3, qy, cell >> 2, default);
                }
            }
        }
        for cell in field["cells"].as_array().ok_or("missing biome cells")? {
            let (qx, qy, qz) = (int(&cell[0])?, int(&cell[1])?, int(&cell[2])?);
            let i = chunk_index(qx * 4, qz * 4)?;
            chunks[i].set_biome_cell(
                qx,
                qy - MIN_Y / 4,
                qz,
                biomes.id(cell[3].as_str().ok_or("biome name")?),
            );
        }
    }
    let region: [[Option<&Chunk>; 3]; 3] =
        std::array::from_fn(|z| std::array::from_fn(|x| Some(&chunks[z * 3 + x])));
    let mut mesher = Mesher {
        nether_shading: row["nether"].as_bool().unwrap_or(false),
        shader_pack_lighting: false,
        assets,
        render,
        tables,
        biomes: &*biomes,
        visibility: tables.visibility.clone(),
        zoom,
        blend_radius,
        smooth_lighting,
        cutout_leaves,
    };
    let section_y = int(&row["section_y"])?;
    let started = Instant::now();
    let mesh = mesher.mesh(&region, (section_y * 16 - MIN_Y) / 16);
    *elapsed += started.elapsed();
    let mut quads = Vec::new();
    for (layer, vertices) in [("opaque", &mesh.solid), ("translucent", &mesh.translucent)] {
        let (vertices, rest) = vertices.as_chunks::<4>();
        if !rest.is_empty() {
            return Err("partial quad".into());
        }
        for quad in vertices {
            let sprite = names
                .get(&u32::from(quad[0].layer & !OPAQUE_LAYER))
                .ok_or("unknown sprite layer")?;
            if quad
                .iter()
                .any(|v: &TerrainVertex| v.layer != quad[0].layer)
            {
                return Err("quad mixes sprite layers".into());
            }
            let vertices: Vec<Value> = quad
                .iter()
                .map(|v| {
                    let ([x, y, z], [u, w]) = (v.pos, v.uv());
                    json!([
                        x,
                        y,
                        z,
                        u,
                        w,
                        argb(v.color | EMISSION_BITS | FLUID_FACE_BITS),
                        light_coords(v.light)
                    ])
                })
                .collect();
            quads.push(json!({"layer": layer, "sprite": sprite, "vertices": vertices}));
        }
    }
    Ok(json!({"name": row["name"], "quads": quads}))
}
pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: section_mesh_fixture RESOURCES (INPUT.jsonl | facts) OUTPUT.jsonl".into(),
        );
    }
    let root = PathBuf::from(&args[0]);
    let assets = Assets::load(&root)?;
    let render = assets.reload(&root)?;
    let facts = BlockFacts::load(&root, assets.registry)?;
    let mut biomes = Biomes::load(&root)?;
    let visibility = models::load_visibility(&root, assets.registry)?;
    let tables = MeshTables::new(&assets, &render, &facts, visibility)?;
    let names: BTreeMap<u32, &str> = render
        .sprite_layers
        .iter()
        .map(|(name, layer)| (*layer, name.as_str()))
        .collect();
    let pristine = biomes.list.clone();
    let mut output = BufWriter::new(fs::File::create(&args[2])?);
    if args[1] == "facts" {
        // The per-state facts the mesher derives, in the oracle's order and positions.
        let positions = [(0, 0), (5, -3), (-17, 31), (123, -456), (9, 2)];
        for id in 0..assets.registry.len() as u32 {
            let state = BlockStateId(id);
            let f = tables.facts(state).ok_or("missing mesh facts")?;
            let offsets: Vec<_> = positions
                .iter()
                .filter_map(|&(x, z)| tables.offset(state, x, z))
                .collect();
            let row = json!({"id": id, "shade": f.shade, "emissive": f.emissive,
                "legacy_solid": f.legacy_solid, "light_permeable": f.light_permeable,
                "solid_render": f.solid_render, "full_collision": f.full_collision,
                "emission": f.emission, "overlay": f.overlay, "tint_sources": f.tint_sources,
                "offsets": offsets});
            serde_json::to_writer(&mut output, &row)?;
            output.write_all(b"\n")?;
        }
        output.flush()?;
        return Ok(());
    }
    let (mut elapsed, mut sections) = (Duration::ZERO, 0u32);
    for line in BufReader::new(fs::File::open(&args[1])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let result = scene(
            &row,
            &assets,
            &render,
            &tables,
            &mut biomes,
            &pristine,
            &names,
            &mut elapsed,
        )?;
        serde_json::to_writer(&mut output, &result)?;
        output.write_all(b"\n")?;
        sections += 1;
    }
    output.flush()?;
    eprintln!(
        "meshed {sections} sections in {:.1} ms ({:.0} us per section)",
        elapsed.as_secs_f64() * 1e3,
        elapsed.as_secs_f64() * 1e6 / f64::from(sections.max(1))
    );
    Ok(())
}
