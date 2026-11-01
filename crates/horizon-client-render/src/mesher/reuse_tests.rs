use super::*;

#[test]
fn padded_reuse_resets_missing_neighbors_and_matches_chunk_reads_across_layouts() {
    let mut pad = Padded::default();
    let mut own = Box::new([u32::MAX; 4096]);
    for min_y in [MIN_Y, -256, 0, 256] {
        let layout = crate::chunk::ColumnLayout::new(min_y, 32).unwrap();
        let base_y = min_y + 16;
        let mut center = Chunk::with_layout(0, 0, layout);
        center.set(0, base_y, 0, 7);
        center.set(15, base_y + 15, 15, 9);
        let mut neighbor = Chunk::with_layout(1, 0, layout);
        neighbor.set(0, base_y + 2, 3, 11);
        let mut region = [[None; 3]; 3];
        region[1][1] = Some(&center);
        region[1][2] = Some(&neighbor);
        pad.fill(&region, 1, &mut own);
        let buffers = (pad.blocks.as_ptr(), pad.light.as_ptr());
        for current in [region, [[None; 3]; 3], region] {
            pad.fill(&current, 1, &mut own);
            for y in -B..16 + B {
                for z in -B..16 + B {
                    for x in -B..16 + B {
                        let chunk = current[((z + 16) >> 4) as usize][((x + 16) >> 4) as usize];
                        // A missing neighbour continues the centre's edge column.
                        let (block, light) = match (chunk, current[1][1]) {
                            (Some(c), _) => (
                                c.get(x & 15, base_y + y, z & 15),
                                c.light(x & 15, base_y + y, z & 15),
                            ),
                            (None, Some(c)) => {
                                let (ex, ez) = (x.clamp(0, 15), z.clamp(0, 15));
                                (c.get(ex, base_y + y, ez), c.light(ex, base_y + y, ez))
                            }
                            (None, None) => (0, 0xF0),
                        };
                        assert_eq!(pad.blocks[pi(x, y, z)], block);
                        assert_eq!(pad.light[pi(x, y, z)], light);
                    }
                }
            }
            assert_eq!(buffers, (pad.blocks.as_ptr(), pad.light.as_ptr()));
        }
    }
}

/// Run explicitly with prepared assets; never silently pass without exercising geometry.
/// With a shader pack lighting the faces, every face of a lone stone block has one colour; the
/// game's face shading (up 1.0, down 0.5, north/south 0.8, east/west 0.6) is left to the pack.
#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn a_shader_pack_lights_the_faces_itself() {
    let root = std::env::var_os("HORIZON_MESH_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client/26.3")
        });
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let biomes = Biomes::load(&root).unwrap();
    let render = assets.reload(&root).unwrap();
    let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
    let tables = MeshTables::new(&assets, &render, &facts, visibility.clone()).unwrap();
    let stone = assets.registry.default_state("minecraft:stone").unwrap().0;
    let mut chunk = Chunk::empty(0, 0);
    chunk.set(8, MIN_Y + 8, 8, stone);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let colours = |shader_pack_lighting| {
        let mut mesher = Mesher {
            nether_shading: false,
            shader_pack_lighting,
            assets: &assets,
            render: &render,
            tables: &tables,
            biomes: &biomes,
            visibility: visibility.clone(),
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: false,
            cutout_leaves: true,
        };
        let mut colours: Vec<u32> = mesher
            .mesh(&region, 0)
            .solid
            .iter()
            .map(|v| v.color)
            .collect();
        colours.sort_unstable();
        colours.dedup();
        colours
    };
    assert_eq!(colours(false).len(), 4);
    assert_eq!(colours(true).len(), 1);
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn reused_mesh_matches_fresh_geometry_across_sections_and_resource_reload() {
    let root = std::env::var_os("HORIZON_MESH_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client/26.3")
        });
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let biomes = Biomes::load(&root).unwrap();
    let state = |name| assets.registry.default_state(name).unwrap().0;
    let names = [
        "minecraft:stone",
        "minecraft:glass",
        "minecraft:grass_block",
        "minecraft:oak_leaves",
        "minecraft:water",
        "minecraft:chest",
    ];
    let mut dense = Chunk::empty(0, 0);
    for z in 0..16 {
        for x in 0..16 {
            dense.set(
                x,
                MIN_Y + 4,
                z,
                state(names[((x + z) as usize) % names.len()]),
            );
        }
    }
    let mut sparse = Chunk::empty(-3, 5);
    sparse.set(0, MIN_Y + 15, 0, state("minecraft:short_grass"));
    sparse.set(15, MIN_Y + 1, 15, state("minecraft:lava"));
    let empty = Chunk::empty(0, 0);
    let mut neighbor = Chunk::empty(1, 0);
    neighbor.set(0, MIN_Y + 4, 0, state("minecraft:stone"));
    let mut custom = Chunk::with_layout(2, -4, crate::chunk::ColumnLayout::new(-256, 64).unwrap());
    custom.set(1, -252, 2, state("minecraft:chest"));
    let mut scratch = MeshScratch::default();
    let mut recycled = SectionMesh::default();
    // The same scratch and output buffers survive a resource reload. They must
    // contain no references into the previous RenderAssets or MeshTables.
    for reload in 0..2 {
        let render = assets.reload(&root).unwrap();
        let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
        let tables = MeshTables::new(&assets, &render, &facts, visibility.clone()).unwrap();
        let mut mesher = Mesher {
            nether_shading: false,
            shader_pack_lighting: false,
            assets: &assets,
            render: &render,
            tables: &tables,
            biomes: &biomes,
            visibility,
            zoom: BiomeZoom::new(42),
            blend_radius: 2,
            smooth_lighting: reload == 0,
            cutout_leaves: reload == 0,
        };
        for (case, (center, east)) in [
            (&empty, None),
            (&dense, Some(&neighbor)),
            (&sparse, None),
            (&empty, None),
            (&dense, None),
            (&custom, None),
        ]
        .into_iter()
        .enumerate()
        {
            let mut region = [[None; 3]; 3];
            region[1][1] = Some(center);
            region[1][2] = east;
            let mut fresh = mesher.mesh(&region, 0);
            recycled = mesher.mesh_reusing(&region, 0, recycled, &mut scratch);
            if reload == 0 && case == 0 {
                assert!(scratch.own.is_none());
                assert_eq!(scratch.padded.blocks.capacity(), 0);
                assert_eq!(scratch.padded.light.capacity(), 0);
                assert_eq!(scratch.visibility.capacity(), 0);
            }
            fresh.sort_solid();
            recycled.sort_solid();
            assert_eq!(
                bytemuck::cast_slice::<TerrainVertex, u8>(&fresh.solid),
                bytemuck::cast_slice::<TerrainVertex, u8>(&recycled.solid)
            );
            assert_eq!(
                bytemuck::cast_slice::<TerrainVertex, u8>(&fresh.translucent),
                bytemuck::cast_slice::<TerrainVertex, u8>(&recycled.translucent)
            );
            assert_eq!(fresh.block_entities, recycled.block_entities);
            assert_eq!(fresh.visibility, recycled.visibility);
            assert_eq!(fresh.sides, recycled.sides);
            assert_eq!(fresh.bounds, recycled.bounds);
            if std::ptr::eq(center, &dense) {
                assert!(!fresh.solid.is_empty());
                assert!(!fresh.translucent.is_empty());
                assert!(!fresh.block_entities.is_empty());
            }
            if std::ptr::eq(center, &custom) {
                assert_eq!(
                    fresh.block_entities,
                    [([33, -252, -62], state("minecraft:chest"))]
                );
            }
        }
    }
}

/// An isolated scratch-stage comparison, not a whole-mesher or FPS benchmark.
/// Output each sample so the caller can retain the observations and spread.
#[test]
#[ignore = "opt-in scratch microbenchmark; run with --nocapture --test-threads=1"]
fn benchmark_padded_reuse() {
    const ITERATIONS: usize = 4096;
    let mut chunk = Chunk::empty(0, 0);
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                chunk.set(x, MIN_Y + y, z, 1 + ((x + y + z) % 7) as u32);
            }
        }
    }
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&chunk);
    let mut pad = Padded::new(&region, 0);
    let mut own = Box::new([0; 4096]);
    let pointers = (pad.blocks.as_ptr(), pad.light.as_ptr(), own.as_ptr());
    let bytes = pad.blocks.capacity() * 4 + pad.light.capacity() + 4096 * 4;
    // Warm both code paths and the process allocator before recording samples.
    for _ in 0..1024 {
        std::hint::black_box(Padded::new(std::hint::black_box(&region), 0));
        pad.fill(std::hint::black_box(&region), 0, &mut own);
        std::hint::black_box(&pad);
    }
    for sample in 0..9 {
        // Alternate execution order to expose, rather than hide, order effects.
        for reused in [sample % 2 == 0, sample % 2 != 0] {
            let start = std::time::Instant::now();
            for _ in 0..ITERATIONS {
                if reused {
                    pad.fill(std::hint::black_box(&region), 0, &mut own);
                    std::hint::black_box(&pad);
                } else {
                    std::hint::black_box(Padded::new(std::hint::black_box(&region), 0));
                }
            }
            println!(
                "{{\"sample\":{sample},\"reused\":{reused},\"iterations\":{ITERATIONS},\"elapsed_ms\":{:.6},\"scratch_bytes\":{bytes}}}",
                start.elapsed().as_secs_f64() * 1000.0
            );
            assert_eq!(
                pointers,
                (pad.blocks.as_ptr(), pad.light.as_ptr(), own.as_ptr())
            );
        }
    }
}
