//! Leaf culling through the real mesher: face counts, the opaque layer and the
//! conservative cases (cavities, partial neighbours, missing chunks, column
//! ends). Needs prepared resources, like the other mesher geometry tests.
use super::*;
use crate::{
    assets::{Assets, RenderAssets},
    biome::Biomes,
    blocks::BlockFacts,
    chunk::{Chunk, ColumnLayout},
    graphics::LeafCullingMode,
    models,
};

struct Fixture {
    assets: Assets,
    biomes: Biomes,
    render: RenderAssets,
    visibility: horizon_world::block_render::FaceVisibility,
    tables: MeshTables,
}

fn fixture() -> Fixture {
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
    let visibility = models::load_visibility(&root, assets.registry).unwrap();
    let tables = MeshTables::new(&assets, &render, &facts, visibility.clone()).unwrap();
    Fixture {
        assets,
        biomes,
        render,
        visibility,
        tables,
    }
}

fn state(f: &Fixture, name: &str) -> u32 {
    f.assets.registry.default_state(name).unwrap().0
}

fn leaves_cube(chunk: &mut Chunk, min: i32, max: i32, y: i32, state: u32) {
    for z in min..=max {
        for x in min..=max {
            chunk.set(x, y, z, state);
        }
    }
}

fn leaf_cube(f: &Fixture, min: i32, max: i32, base: i32, name: &str) -> Chunk {
    let mut chunk = Chunk::empty(0, 0);
    let state = state(f, name);
    for y in base..base + (max - min + 1) {
        leaves_cube(&mut chunk, min, max, y, state);
    }
    chunk
}

fn mesh(
    f: &Fixture,
    _chunk: &Chunk,
    region: [[Option<&Chunk>; 3]; 3],
    sy: i32,
    mode: LeafCullingMode,
) -> SectionMesh {
    let mut mesher = Mesher {
        nether_shading: false,
        shader_pack_lighting: false,
        separate_ao: false,
        assets: &f.assets,
        render: &f.render,
        tables: &f.tables,
        biomes: &f.biomes,
        visibility: f.visibility.clone(),
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: false,
        cutout_leaves: true,
        translucent_layers: &[],
        leaf_culling: mode,
    };
    mesher.mesh(&region, sy)
}

fn around(chunk: &Chunk) -> [[Option<&Chunk>; 3]; 3] {
    [[Some(chunk); 3]; 3]
}

/// (opaque, cutout) leaf quads, rejecting a quad whose four vertices disagree.
fn quads(mesh: &SectionMesh) -> (usize, usize) {
    let mut opaque = 0;
    let mut cutout = 0;
    for quad in mesh.solid.as_chunks::<4>().0 {
        let flags = quad
            .iter()
            .map(|v| v.layer & OPAQUE_LAYER != 0)
            .collect::<Vec<_>>();
        assert!(
            flags.iter().all(|flag| *flag == flags[0]),
            "a quad mixed opaque and cutout vertices: {flags:?}"
        );
        if flags[0] {
            opaque += 1;
        } else {
            cutout += 1;
        }
    }
    assert_eq!(mesh.solid.len() % 4, 0);
    (opaque, cutout)
}

const BASE: i32 = MIN_Y + 4;

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn off_draws_every_fancy_leaf_face() {
    let f = fixture();
    let chunk = leaf_cube(&f, 6, 8, BASE, "minecraft:oak_leaves");
    let mesh = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Off);
    // 27 blocks, six quads each: vanilla's fancy leaves keep the inside.
    assert_eq!(quads(&mesh), (0, 162));
    assert_eq!(mesh.translucent.len(), 0);
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn solid_draws_the_fully_enclosed_leaf_opaque() {
    let f = fixture();
    let chunk = leaf_cube(&f, 6, 8, BASE, "minecraft:oak_leaves");
    let solid = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Solid);
    // Only the centre has all six sides covered: its six quads go opaque.
    assert_eq!(quads(&solid), (6, 156));

    let aggressive = mesh(
        &f,
        &chunk,
        around(&chunk),
        0,
        LeafCullingMode::SolidAggressive,
    );
    // Aggressive also takes the six face-centre leaves: seven blocks opaque.
    assert_eq!(quads(&aggressive), (42, 120));
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn aggressive_solidifies_the_next_shell_layer_of_a_five_cube() {
    let f = fixture();
    let chunk = leaf_cube(&f, 5, 9, BASE, "minecraft:oak_leaves");
    let solid = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Solid);
    // The 3x3x3 inside the surface: 27 blocks, six quads each.
    assert_eq!(quads(&solid), (162, 588));
    let aggressive = mesh(
        &f,
        &chunk,
        around(&chunk),
        0,
        LeafCullingMode::SolidAggressive,
    );
    // The 3x3x3 inside plus the 3x3 face centre of each of the six sides.
    assert_eq!(quads(&aggressive), (486, 264));
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn hollow_drops_only_the_faces_two_leaves_share() {
    let f = fixture();
    let chunk = leaf_cube(&f, 6, 8, BASE, "minecraft:oak_leaves");
    let hollow = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Hollow);
    // 162 quads minus the 54 shared pairs' 108 faces: the 3x3x3 shell.
    assert_eq!(quads(&hollow), (0, 54));
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn mixed_species_count_as_one_leaf_volume() {
    let f = fixture();
    let oak = state(&f, "minecraft:oak_leaves");
    let birch = state(&f, "minecraft:birch_leaves");
    let mut chunk = Chunk::empty(0, 0);
    for y in BASE..BASE + 3 {
        for z in 6..=8 {
            for x in 6..=8 {
                chunk.set(x, y, z, if (x + y + z) % 2 == 0 { oak } else { birch });
            }
        }
    }
    let solid = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Solid);
    assert_eq!(quads(&solid), (6, 156));
    let hollow = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Hollow);
    assert_eq!(quads(&hollow), (0, 54));
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn a_cavity_keeps_its_six_faces_and_its_rim_stays_cutout() {
    let f = fixture();
    let mut chunk = leaf_cube(&f, 5, 9, BASE, "minecraft:oak_leaves");
    chunk.set(7, BASE + 2, 7, 0);
    let hollow = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Hollow);
    // Five-cube shell (150) plus the six faces around the one-block cavity.
    assert_eq!(quads(&hollow), (0, 156));
    let solid = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Solid);
    // The six leaves around the cavity are exposed, so they stay cutout; the
    // other 20 inner leaves draw solid. The cavity block itself has no quads.
    assert_eq!(quads(&solid), (120, 624));
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn section_boundary_faces_stay_with_a_loaded_neighbour() {
    let f = fixture();
    let leaf = state(&f, "minecraft:oak_leaves");
    let chunk = leaf_cube(&f, 12, 15, BASE, "minecraft:oak_leaves");
    // A loaded east chunk whose x=0 column continues the canopy.
    let mut east = Chunk::empty(1, 0);
    for y in BASE..BASE + 4 {
        for z in 12..=15 {
            east.set(0, y, z, leaf);
        }
    }
    let mut region = around(&chunk);
    region[1][2] = Some(&east);
    let joined = mesh(&f, &chunk, region, 0, LeafCullingMode::Hollow);
    let mut region = around(&chunk);
    region[1][2] = None;
    let missing = mesh(&f, &chunk, region, 0, LeafCullingMode::Hollow);
    // The east faces of the x=15 leaves are on the section boundary: a loaded
    // neighbour does not let Hollow drop them, and a missing one does not
    // change the mesh either.
    assert_eq!(joined.solid.len(), missing.solid.len());
    let off = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Off);
    assert!(joined.solid.len() < off.solid.len());
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn the_top_of_a_one_section_column_stays_visible() {
    let f = fixture();
    let leaf = state(&f, "minecraft:oak_leaves");
    let layout = ColumnLayout::new(0, 16).unwrap();
    let mut chunk = Chunk::with_layout(0, 0, layout);
    for y in 13..16 {
        leaves_cube(&mut chunk, 5, 7, y, leaf);
    }
    let hollow = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Hollow);
    let solid = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Solid);
    // y=15 is the column's top: the faces toward y=16 are unknown, so they
    // stay, and the y=15 leaves are not enclosed.
    assert!(!hollow.solid.is_empty());
    assert!(solid.solid.iter().any(|v| v.pos[1] == 16.0));
    assert!(
        solid
            .solid
            .iter()
            .filter(|v| v.pos[1] == 16.0)
            .all(|v| v.layer & OPAQUE_LAYER == 0)
    );
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn a_world_without_leaves_meshes_identically_in_every_mode() {
    let f = fixture();
    let mut chunk = Chunk::empty(0, 0);
    for y in BASE..BASE + 3 {
        leaves_cube(&mut chunk, 6, 8, y, state(&f, "minecraft:stone"));
    }
    let off = mesh(&f, &chunk, around(&chunk), 0, LeafCullingMode::Off);
    for mode in LeafCullingMode::ALL {
        let other = mesh(&f, &chunk, around(&chunk), 0, mode);
        assert_eq!(off.solid.len(), other.solid.len(), "{mode:?}");
        assert_eq!(off.translucent.len(), other.translucent.len(), "{mode:?}");
        for (a, b) in off.solid.iter().zip(&other.solid) {
            assert_eq!(
                (a.pos, a.uv, a.layer, a.light, a.color),
                (b.pos, b.uv, b.layer, b.light, b.color),
                "{mode:?}"
            );
        }
        for (a, b) in off.translucent.iter().zip(&other.translucent) {
            assert_eq!(
                (a.pos, a.uv, a.layer, a.light, a.color),
                (b.pos, b.uv, b.layer, b.light, b.color),
                "{mode:?}"
            );
        }
    }
}
