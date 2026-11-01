//! `DebugLevelSource`: the `minecraft:debug` generator of the Debug world preset,
//! a grid of every block state floating over a layer of barriers.
use crate::{
    chunk::{Chunk, LevelShape},
    streaming::ChunkGenerator,
};
use horizon_world::{nbt::Tag, registry::BlockRegistry};

/// `DebugLevelSource.HEIGHT`: the layer the block states stand in.
pub const HEIGHT: i32 = 70;
/// `DebugLevelSource.BARRIER_HEIGHT`.
pub const BARRIER_HEIGHT: i32 = 60;

/// The debug world's generator: every block state in registry order
/// (`ALL_BLOCKS`) on a square grid, one every other block, at `HEIGHT`.
pub struct DebugTerrain {
    shape: LevelShape,
    biome: u16,
    barrier: u32,
    /// `ALL_BLOCKS.size()`: block states are numbered in that order, so the
    /// state at an index is the state with that id.
    states: u32,
}
impl DebugTerrain {
    /// The generator of a world whose overworld is `minecraft:debug`.
    pub fn from_settings(
        settings: &Tag,
        registry: &BlockRegistry,
        biomes: &crate::biome::Biomes,
        shape: LevelShape,
    ) -> Option<Self> {
        let generator = settings
            .get("dimensions")?
            .get("minecraft:overworld")?
            .get("generator")?;
        if crate::save::text(generator.get("type"))?.trim_start_matches("minecraft:") != "debug" {
            return None;
        }
        Some(Self {
            shape,
            // `FixedBiomeSource(plains)`.
            biome: biomes.id("minecraft:plains"),
            barrier: registry.default_state("minecraft:barrier")?.0,
            states: u32::try_from(registry.len()).ok()?,
        })
    }
    /// `DebugLevelSource.getBlockStateFor`: the state id at a block column
    /// (air, id 0, where the grid has none).
    pub fn state_for(&self, x: i32, z: i32) -> u32 {
        grid_state(self.states, x, z)
    }
}
/// `DebugLevelSource.GRID_WIDTH` and `GRID_HEIGHT` for `count` block states:
/// `Mth.ceil(Mth.sqrt(size))` and `ceil(size / width)`.
fn grid(count: u32) -> (i32, i32) {
    let width = (count as f32).sqrt().ceil() as i32;
    let height = (count as f32 / width as f32).ceil() as i32;
    (width, height)
}
/// `getBlockStateFor(worldX, worldZ)` as a state id.
fn grid_state(count: u32, mut x: i32, mut z: i32) -> u32 {
    let (width, height) = grid(count);
    if x > 0 && z > 0 && x % 2 != 0 && z % 2 != 0 {
        x /= 2;
        z /= 2;
        if x <= width && z <= height {
            let index = (x * width + z).unsigned_abs();
            if index < count {
                return index;
            }
        }
    }
    0
}
impl ChunkGenerator for DebugTerrain {
    type Worker = ();
    fn worker(&self) {}
    /// `DebugLevelSource.buildTerrain` builds nothing; `applyBiomeDecoration`
    /// writes the barriers and the grid.
    fn proto(&self, _: &mut (), cx: i32, cz: i32) -> crate::streaming::Proto {
        let mut chunk = Chunk::with_layout(cx, cz, self.shape.column());
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).biomes = horizon_core::paletted::PalettedContainer::new(
                crate::chunk::BIOME_STRATEGY,
                u32::from(self.biome),
            );
        }
        for z in 0..16 {
            for x in 0..16 {
                chunk.set(x, BARRIER_HEIGHT, z, self.barrier);
                let state = self.state_for(cx * 16 + x, cz * 16 + z);
                if state != 0 {
                    chunk.set(x, HEIGHT, z, state);
                }
            }
        }
        for s in &mut chunk.sections {
            std::sync::Arc::make_mut(s).compact();
        }
        Box::new(chunk)
    }
    /// `spawnOriginalMobs` is empty.
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        *proto.downcast::<Chunk>().expect("debug terrain proto")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `getBlockStateFor`: only odd, positive columns hold a state, in the order
    /// `x / 2 * width + z / 2`, and nothing past the last state.
    #[test]
    fn the_grid_holds_a_state_every_other_block() {
        // 1000 states: a 32 wide grid, 32 high (ceil(1000 / 32) = 32).
        assert_eq!(grid(1000), (32, 32));
        assert_eq!(grid(1), (1, 1));
        assert_eq!(grid(26_000), (162, 161));
        // Even, zero and negative columns are air.
        for (x, z) in [(2, 1), (1, 2), (0, 1), (1, 0), (-1, 1), (1, -1)] {
            assert_eq!(grid_state(1000, x, z), 0, "{x} {z}");
        }
        // (1, 1) is index 0 (air), (3, 5) is 1 * 32 + 2 = 34.
        assert_eq!(grid_state(1000, 1, 1), 0);
        assert_eq!(grid_state(1000, 3, 5), 34);
        // x = 2 * 32 + 1 maps to x / 2 = 32: index 32 * 32 + z / 2, past the 1000 states.
        assert_eq!(grid_state(1000, 65, 1), 0);
        assert_eq!(grid_state(1000, 61, 3), 30 * 32 + 1);
    }
}
