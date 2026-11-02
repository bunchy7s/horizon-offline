//! A data-pack biome is in the registry a server sends, with its data, at the
//! place the chunk encoder numbers it by, and a client rebuilds it from that
//! data.
use horizon_client::{
    biome::Biomes,
    chunk::LevelShapes,
    lan::data::{BIOME_REGISTRY, SyncData, client_biomes},
};
use serde_json::Value;
use std::path::PathBuf;

/// Removes the folder when the test ends, passed or not.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn a_pack_biome_is_sent_with_its_data_and_read_back_by_a_client() {
    let root = std::env::temp_dir().join(format!("horizon-sync-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let maps = root.join("assets/minecraft/textures/colormap");
    let biome_dir = root.join("data/minecraft/worldgen/biome");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::create_dir_all(&biome_dir).unwrap();
    std::fs::write(root.join("registries.json"), "{}").unwrap();
    for name in ["grass", "foliage", "dry_foliage"] {
        let file = std::fs::File::create(maps.join(format!("{name}.png"))).unwrap();
        let mut png = png::Encoder::new(file, 1, 1);
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        let mut writer = png.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3]).unwrap();
    }
    for name in ["plains", "river"] {
        std::fs::write(biome_dir.join(format!("{name}.json")), "{}").unwrap();
    }
    let base = Biomes::load(&root).unwrap();
    let pack: Value = serde_json::from_str(r##"{"effects":{"grass_color":"#123456"}}"##).unwrap();
    let world = base.with_packs([("minecraft:tundra_x", &pack)]);
    let sync =
        SyncData::load_with(&root, &horizon_world::loot::PackOverlay::NONE, Some(&world)).unwrap();

    // The encoder numbers a biome by its place in this list: none may be
    // missing, and the pack's is not the first.
    let registry = &sync.dynamic.registries[BIOME_REGISTRY];
    let names: Vec<_> = registry.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["minecraft:plains", "minecraft:river", "minecraft:tundra_x"]
    );
    assert!(world.list.iter().all(|b| names.contains(&b.name.as_str())));

    // A client that knows the vanilla pack still gets the pack biome's data.
    let sent = sync.registry_packets(true, &LevelShapes::VANILLA);
    let biomes = sent.iter().find(|r| r.registry == BIOME_REGISTRY).unwrap();
    assert!(biomes.entries[0].data.is_none(), "a known one is not sent");
    assert!(biomes.entries[2].data.is_some(), "the pack's biome is sent");

    // The client builds its table from what arrived: the server's id for
    // the name, and the pack's colour.
    let received: Vec<_> = biomes
        .entries
        .iter()
        .map(|e| (e.id.clone(), e.data.clone()))
        .collect();
    let client = client_biomes(&base, &received).expect("the pack's biome is new to the client");
    let id = client.id("minecraft:tundra_x");
    assert_eq!(id, world.id("minecraft:tundra_x"));
    assert_eq!(client.get(id).grass, 0x123456);
    assert_ne!(
        client.get(id).grass,
        client.get(client.id("minecraft:plains")).grass
    );

    // Nothing new to a client that knows them all: it keeps its own table.
    let known: Vec<_> = received
        .iter()
        .filter(|(n, _)| base.contains(n))
        .cloned()
        .collect();
    assert!(client_biomes(&base, &known).is_none());
}
