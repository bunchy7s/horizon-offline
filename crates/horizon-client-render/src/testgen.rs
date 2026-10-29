//! Simple rolling terrain used only until the vanilla overworld generator is
//! bound (`--test-terrain`). Not vanilla generation.
use crate::{chunk::Chunk, streaming::ChunkGenerator};
use horizon_world::registry::BlockRegistry;

pub struct TestTerrain {
    stone: u32,
    dirt: u32,
    grass: u32,
    water: u32,
    sand: u32,
    bedrock: u32,
    log: u32,
    leaves: u32,
    short_grass: u32,
    plains: u16,
}
impl TestTerrain {
    pub fn new(registry: &BlockRegistry, biomes: &crate::biome::Biomes) -> crate::Result<Self> {
        let id = |name: &str| -> crate::Result<u32> {
            Ok(registry
                .default_state(name)
                .ok_or_else(|| format!("missing {name}"))?
                .0)
        };
        Ok(Self {
            stone: id("minecraft:stone")?,
            dirt: id("minecraft:dirt")?,
            grass: id("minecraft:grass_block")?,
            water: id("minecraft:water")?,
            sand: id("minecraft:sand")?,
            bedrock: id("minecraft:bedrock")?,
            log: id("minecraft:oak_log")?,
            leaves: registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:oak_leaves".into(),
                    properties: [
                        ("distance".to_owned(), "1".to_owned()),
                        ("persistent".to_owned(), "false".to_owned()),
                        ("waterlogged".to_owned(), "false".to_owned()),
                    ]
                    .into(),
                })?
                .0,
            short_grass: id("minecraft:short_grass")?,
            plains: biomes.id("minecraft:plains"),
        })
    }
    fn height(x: i32, z: i32) -> i32 {
        let (x, z) = (f64::from(x), f64::from(z));
        let h = 68.0
            + 10.0 * (x / 47.0).sin() * (z / 61.0).cos()
            + 5.0 * ((x + z) / 23.0).sin()
            + 3.0 * (x / 11.0).cos() * (z / 13.0).sin();
        h as i32
    }
}
fn hash(x: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (z as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}
impl ChunkGenerator for TestTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn lod_surface(&self, _: &mut (), x: i32, z: i32) -> Option<crate::lod::Surface> {
        let h = Self::height(x, z);
        Some(crate::lod::Surface {
            height: h + 1,
            state: horizon_world::registry::BlockStateId(if h <= 63 {
                self.sand
            } else {
                self.grass
            }),
            water: (h < 62).then_some((63, horizon_world::registry::BlockStateId(self.water))),
            biome: self.plains,
        })
    }
    fn proto(&self, w: &mut (), x: i32, z: i32) -> crate::streaming::Proto {
        Box::new(self.generate(w, x, z))
    }
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        let mut chunk = *proto.downcast::<Chunk>().expect("test terrain proto");
        chunk.fresh = true;
        chunk
    }
}
impl TestTerrain {
    fn generate(&self, _: &mut (), cx: i32, cz: i32) -> Chunk {
        let mut chunk = Chunk::empty(cx, cz);
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).biomes = horizon_core::paletted::PalettedContainer::new(
                crate::chunk::BIOME_STRATEGY,
                u32::from(self.plains),
            );
        }
        for z in 0..16 {
            for x in 0..16 {
                let (wx, wz) = (cx * 16 + x, cz * 16 + z);
                let h = Self::height(wx, wz);
                for y in -64..=h.max(62) {
                    let b = if y == -64 {
                        self.bedrock
                    } else if y > h {
                        self.water
                    } else if y < h - 3 {
                        self.stone
                    } else if h <= 63 {
                        self.sand
                    } else if y == h {
                        self.grass
                    } else {
                        self.dirt
                    };
                    chunk.set(x, y, z, b);
                }
                if h > 63 && hash(wx, wz).is_multiple_of(13) {
                    chunk.set(x, h + 1, z, self.short_grass);
                }
            }
        }
        // Trees fully inside the chunk.
        for z in 2..14 {
            for x in 2..14 {
                let (wx, wz) = (cx * 16 + x, cz * 16 + z);
                let h = Self::height(wx, wz);
                if h <= 64 || !hash(wx, wz).is_multiple_of(97) {
                    continue;
                }
                for y in h + 1..h + 6 {
                    chunk.set(x, y, z, self.log);
                }
                for dy in 3..7 {
                    let r = if dy >= 5 { 1 } else { 2 };
                    for dz in -r..=r {
                        for dx in -r..=r {
                            if chunk.get(x + dx, h + dy, z + dz) == 0 {
                                chunk.set(x + dx, h + dy, z + dz, self.leaves);
                            }
                        }
                    }
                }
            }
        }
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).compact();
        }
        chunk
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_surface_agrees_with_generated_land_and_water() {
        let terrain = TestTerrain {
            stone: 1,
            dirt: 2,
            grass: 3,
            water: 4,
            sand: 5,
            bedrock: 6,
            log: 7,
            leaves: 8,
            short_grass: 9,
            plains: 10,
        };
        let mut wet = false;
        for (cx, cz) in [(0, 0), (-7, 3), (15, -12), (-21, -8)] {
            let chunk = terrain.generate(&mut (), cx, cz);
            for x in 0..16 {
                let surface = terrain.lod_surface(&mut (), cx * 16 + x, cz * 16).unwrap();
                assert_eq!(chunk.get(x, surface.height - 1, 0), surface.state.0);
                if let Some((height, state)) = surface.water {
                    wet = true;
                    assert_eq!(chunk.get(x, height - 1, 0), state.0);
                    assert_eq!(chunk.get(x, height, 0), 0);
                }
            }
        }
        assert!(wet, "fixture includes underwater land");
    }
}
