use super::*;

/// `TerrainParticle`: the model's particle sprite and tint source 0's
/// `colorAsTerrainParticle` (#1034: grass blocks crumbled as tinted grass tops).
#[test]
fn terrain_particles_use_the_particle_sprite_and_vanilla_tint_sources() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
    if !root.join("horizon-resources.json").exists() {
        return;
    }
    let assets = Assets::load(&root).unwrap();
    let render = assets.reload(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
    let tables = MeshTables::new(&assets, &render, &facts, visibility).unwrap();
    let biomes = Biomes::load(&root).unwrap();
    let state = |name| assets.registry.default_state(name).unwrap();
    let sprite = |name| {
        let layer = render.model_catalog.particle(state(name)).unwrap();
        let (sprite, _) = render
            .sprite_layers
            .iter()
            .find(|(_, l)| **l == layer)
            .unwrap();
        sprite.clone()
    };
    assert_eq!(sprite("minecraft:grass_block"), "minecraft:block/dirt");
    assert_eq!(
        sprite("minecraft:short_grass"),
        "minecraft:block/short_grass"
    );
    assert_eq!(sprite("minecraft:oak_leaves"), "minecraft:block/oak_leaves");

    let center = Chunk::empty(0, 0);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&center);
    let tint = |name| {
        tables.terrain_particle_tint(
            state(name),
            [5, 70, 9],
            &region,
            &biomes,
            BiomeZoom::new(0),
            2,
        )
    };
    // grassBlock() overrides colorAsTerrainParticle to -1: the dirt stays untinted.
    assert_eq!(tint("minecraft:grass_block"), None);
    assert!(tint("minecraft:short_grass").is_some());
    assert!(tint("minecraft:oak_leaves").is_some());
    assert_eq!(tint("minecraft:spruce_leaves"), Some(0x61_9961));
    // Pink petals' layer 0 is BLANK_LAYER; stone has no tint source.
    assert_eq!(tint("minecraft:pink_petals"), None);
    assert_eq!(tint("minecraft:stone"), None);
}

/// Blocks drawn outside the chunk mesh (moved by a piston) take the chunk
/// mesher's per-layer tints: a grass block's top and side overlay are tinted
/// as in the world, not left gray like its particles.
#[test]
fn block_tints_match_the_chunk_mesh_for_grass_blocks() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
    if !root.join("horizon-resources.json").exists() {
        return;
    }
    let assets = Assets::load(&root).unwrap();
    let render = assets.reload(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
    let tables = MeshTables::new(&assets, &render, &facts, visibility).unwrap();
    let biomes = Biomes::load(&root).unwrap();
    let state = |name| assets.registry.default_state(name).unwrap();
    let center = Chunk::empty(0, 0);
    let mut region = [[None; 3]; 3];
    region[1][1] = Some(&center);
    let tints = |name| {
        tables.block_tints(
            state(name),
            [5, 70, 9],
            &region,
            &biomes,
            BiomeZoom::new(0),
            2,
        )
    };
    let grass = tints("minecraft:grass_block");
    assert!(grass(0).is_some());
    assert_eq!(grass(-1), None);
    assert_eq!(tints("minecraft:spruce_leaves")(0), Some(0x61_9961));
    assert_eq!(tints("minecraft:stone")(0), None);
}
