//! A level taller than vanilla's (To The Max's Overworld: -64..2032, 131
//! sections) streams, lights and meshes to its top, and edits near the top
//! relight and remesh it.
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::{Chunk, ColumnLayout, LevelShape},
    light::LightTables,
    mesher::{MeshTables, Mesher, Region, SectionMesh, borrow},
    models,
    streaming::{ChunkGenerator, Proto, Streaming, WorkerContext},
    world::World,
};
use horizon_core::biome::BiomeZoom;
use horizon_world::registry::BlockStateId;
use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn resources() -> Option<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    root.join("items.json").exists().then_some(root)
}

/// Saved columns without their light: a stone spire to y 1799 in the middle
/// of each, so light jobs and meshes reach far above vanilla's top.
struct Spires {
    stone: u32,
    layout: ColumnLayout,
}
impl ChunkGenerator for Spires {
    type Worker = ();
    fn worker(&self) {}
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        let mut chunk = Chunk::with_layout(x, z, self.layout);
        for y in -64..1800 {
            for (lx, lz) in [(7, 7), (8, 7), (7, 8), (8, 8)] {
                chunk.set(lx, y, lz, self.stone);
            }
        }
        Some(chunk)
    }
    fn proto(&self, _: &mut (), _: i32, _: i32) -> Proto {
        unreachable!("every column is saved")
    }
    fn finish(&self, _: Proto) -> Chunk {
        unreachable!("every column is saved")
    }
}

#[test]
fn a_tall_column_streams_lights_and_meshes_to_its_top() {
    let Some(root) = resources() else {
        return;
    };
    let shape = LevelShape::new(-64, 2096, 2096, true).unwrap();
    let layout = shape.column();
    let assets = Arc::new(Assets::load(&root).unwrap());
    let render = Arc::new(assets.reload(&root).unwrap());
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let visibility = models::load_visibility(&root, assets.registry).unwrap();
    let context = Arc::new(WorkerContext {
        tables: Arc::new(MeshTables::new(&assets, &render, &facts, visibility).unwrap()),
        light: Arc::new(LightTables::load(&root, assets.registry, &facts).unwrap()),
        biomes: Arc::new(Biomes::load(&root).unwrap()),
        assets: assets.clone(),
        render: render.clone(),
    });
    let state = |name: &str| assets.registry.default_state(name).unwrap().0;
    let (stone, torch) = (state("minecraft:stone"), state("minecraft:torch"));
    let mut streaming = Streaming::new(Arc::new(Spires { stone, layout }), context.clone(), 2);
    streaming.radius = 2;
    let mut world = World::new(BlockStateId(0), 0);
    world.shape = shape;
    let mut meshes: HashMap<(i32, i32, i32), SectionMesh> = HashMap::new();
    let settle = |streaming: &mut Streaming,
                  world: &mut World,
                  meshes: &mut HashMap<(i32, i32, i32), SectionMesh>| {
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut quiet = 0;
        while quiet < 40 {
            assert!(Instant::now() < deadline, "streaming never settled");
            let (uploads, _) = streaming.update(world);
            let idle = uploads.is_empty() && streaming.pending() == 0;
            meshes.extend(uploads);
            thread::sleep(Duration::from_millis(5));
            quiet = if idle { quiet + 1 } else { 0 };
        }
    };
    settle(&mut streaming, &mut world, &mut meshes);
    let centre = world.chunks.get(&(0, 0)).expect("centre column loaded");
    assert_eq!(centre.sections.len(), 131);
    assert!(streaming.is_lit(0, 0));
    // Sky light reaches the top and the ground beside the spire; the spire
    // itself is dark.
    assert_eq!(centre.light(0, 2031, 0) >> 4, 15);
    assert_eq!(centre.light(0, 1000, 0) >> 4, 15);
    assert_eq!(centre.light(7, 1000, 7), 0);
    assert!((0..131).all(|sy| meshes.contains_key(&(0, sy, 0))));
    // A torch on the spire's top: it lights the air around it, and the
    // sections it reaches remesh.
    world.set(8, 1800, 8, BlockStateId(torch)).unwrap();
    settle(&mut streaming, &mut world, &mut meshes);
    let centre = world.chunks.get(&(0, 0)).unwrap();
    assert_eq!(centre.light(8, 1800, 8) & 15, 14);
    assert_eq!(centre.light(8, 1803, 8) & 15, 11);
    let mut checked = 0;
    for (&(x, sy, z), mesh) in &meshes {
        if (x, z) != (0, 0) || !(100..=115).contains(&sy) {
            continue;
        }
        let region: Region = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                world
                    .chunks
                    .get(&(x + dx as i32 - 1, z + dz as i32 - 1))
                    .cloned()
            })
        });
        let mut mesher = Mesher {
            nether_shading: false,
            shader_pack_lighting: false,
            zoom: BiomeZoom::new(0),
            blend_radius: 2,
            smooth_lighting: true,
            cutout_leaves: true,
            assets: &context.assets,
            render: &context.render,
            tables: &context.tables,
            biomes: &context.biomes,
            visibility: context.tables.visibility.clone(),
        };
        // The workers sort solid quads by side, as uploads expect.
        let mut fresh = mesher.mesh(&borrow(&region), sy);
        fresh.sort_solid();
        let bytes = |m: &SectionMesh| {
            [
                bytemuck::cast_slice::<_, u8>(&m.solid).to_vec(),
                bytemuck::cast_slice::<_, u8>(&m.translucent).to_vec(),
            ]
        };
        assert!(
            bytes(mesh) == bytes(&fresh) && mesh.visibility == fresh.visibility,
            "section {x} {sy} {z} kept a stale mesh"
        );
        checked += 1;
    }
    assert_eq!(checked, 16);
}
