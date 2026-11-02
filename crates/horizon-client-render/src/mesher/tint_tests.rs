//! The one-biome shortcut of `Tints::get` against the full blend. Needs the
//! prepared biome data, like the other mesher tests that load resources.
use super::*;

fn biomes() -> Biomes {
    let root = std::env::var_os("HORIZON_MESH_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client/26.3")
        });
    Biomes::load(&root).unwrap()
}

/// A chunk whose every quart cell holds `biome`.
fn filled(cx: i32, cz: i32, biome: u16) -> Chunk {
    let mut chunk = Chunk::empty(cx, cz);
    for qy in 0..(384 / 4) {
        for qz in 0..4 {
            for qx in 0..4 {
                chunk.set_biome_cell(qx, qy, qz, biome);
            }
        }
    }
    chunk
}

#[test]
#[ignore = "requires prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES or .local/horizon-client/26.3)"]
fn one_biome_shortcut_blends_like_the_full_square() {
    let biomes = biomes();
    let desert = biomes.id("minecraft:desert");
    let (cx, cz) = (3, -2);
    // Grass without and with the dark forest modifier, swamp noise, and
    // biomes whose foliage, dry foliage and water differ from the defaults.
    for name in [
        "minecraft:plains",
        "minecraft:dark_forest",
        "minecraft:swamp",
        "minecraft:mangrove_swamp",
        "minecraft:cherry_grove",
        "minecraft:warm_ocean",
        "minecraft:badlands",
    ] {
        let biome = biomes.id(name);
        let chunks: Vec<Vec<Chunk>> = (0..3)
            .map(|rz| {
                (0..3)
                    .map(|rx| filled(cx + rx - 1, cz + rz - 1, biome))
                    .collect()
            })
            .collect();
        // One desert column in the east neighbour, next to the section's edge,
        // where every blend radius reaches it.
        let mut east = filled(cx + 1, cz, biome);
        for qy in 0..(384 / 4) {
            east.set_biome_cell(0, qy, 2, desert);
        }
        let all = |rz: usize, rx: usize| Some(&chunks[rz][rx]);
        let uniform: [[Option<&Chunk>; 3]; 3] =
            std::array::from_fn(|rz| std::array::from_fn(|rx| all(rz, rx)));
        let mut mixed = uniform;
        mixed[1][2] = Some(&east);
        // A missing neighbour reads as plains.
        let mut missing = uniform;
        missing[0][0] = None;
        for (region, one_biome) in [
            (&uniform, true),
            (&mixed, false),
            (&missing, name == "minecraft:plains"),
        ] {
            for base_y in [MIN_Y, 0, 304] {
                for (radius, seed) in [(0, 0), (1, 42), (2, -7), (5, 1234), (7, i64::MAX)] {
                    let new = || {
                        Tints::new(
                            region,
                            &biomes,
                            BiomeZoom::new(seed),
                            radius,
                            [cx, cz, base_y],
                        )
                    };
                    let mut fast = new();
                    let mut full = new();
                    full.uniform = Some(None);
                    assert_eq!(
                        fast.uniform_biome().is_some(),
                        one_biome,
                        "{name} r{radius} y{base_y}"
                    );
                    for resolver in [
                        Resolver::Grass,
                        Resolver::Foliage,
                        Resolver::Dry,
                        Resolver::Water,
                    ] {
                        for y in 0..16 {
                            for z in 0..16 {
                                for x in 0..16 {
                                    assert_eq!(
                                        fast.get(resolver, x, y, z),
                                        full.get(resolver, x, y, z),
                                        "{name} resolver {} r{radius} at {x} {y} {z} of y{base_y}",
                                        resolver as usize
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
