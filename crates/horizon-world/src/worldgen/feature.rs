//! Initial block-writing features. Full feature registries and block behavior
//! remain separate work; world access supplies mutation and update semantics.
use super::{DecodeError, Object, density::identifier, required};
use crate::registry::{BlockRegistry, BlockStateId};
use horizon_core::random::{InvalidBound, RandomSource};
#[derive(Clone, Copy, Debug)]
pub struct FeatureBlocks {
    pub air: BlockStateId,
    pub stone: BlockStateId,
    pub cobblestone: BlockStateId,
    pub end_stone: BlockStateId,
    pub obsidian: BlockStateId,
}
impl FeatureBlocks {
    pub fn from_registry(registry: &BlockRegistry) -> Result<Self, DecodeError> {
        let get = |name| {
            registry
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("unknown feature block {name}")))
        };
        Ok(Self {
            air: get("minecraft:air")?,
            stone: get("minecraft:stone")?,
            cobblestone: get("minecraft:cobblestone")?,
            end_stone: get("minecraft:end_stone")?,
            obsidian: get("minecraft:obsidian")?,
        })
    }
}
pub trait FeatureWorld {
    type Error;
    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, Self::Error>;
    /// Walk the column from `position` by `step` (1 or -1) while `keep`
    /// holds for the block there, reading at most `limit` blocks. Returns the
    /// y it stopped at: the first refused block, or the one after the last
    /// read. `keep` must depend only on the state (it may be skipped for a
    /// run of blocks known to be identical).
    fn scan_column(
        &mut self,
        position: [i32; 3],
        step: i32,
        limit: i32,
        mut keep: impl FnMut(BlockStateId) -> bool,
    ) -> Result<i32, Self::Error>
    where
        Self: Sized,
    {
        let mut p = position;
        for _ in 0..limit {
            if !keep(self.block(p)?) {
                break;
            }
            p[1] = p[1].wrapping_add(step);
        }
        Ok(p[1])
    }
    fn set_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, Self::Error>;
}
#[derive(Debug)]
pub enum FeatureError<E> {
    Random(InvalidBound),
    World(E),
}
impl<E: std::fmt::Display> std::fmt::Display for FeatureError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Random(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for FeatureError<E> {}
impl<E> From<InvalidBound> for FeatureError<E> {
    fn from(e: InvalidBound) -> Self {
        Self::Random(e)
    }
}
#[derive(Clone, Copy, Debug)]
pub enum BasicFeature {
    VoidStartPlatform,
    EndIsland,
    EndPlatform,
}
impl BasicFeature {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        match identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?
        .as_str()
        {
            "minecraft:void_start_platform" => Ok(Self::VoidStartPlatform),
            "minecraft:end_island" => Ok(Self::EndIsland),
            "minecraft:end_platform" => Ok(Self::EndPlatform),
            _ => Err(DecodeError::Field("basic feature type")),
        }
    }
    pub fn place<W: FeatureWorld>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
        blocks: FeatureBlocks,
    ) -> Result<bool, FeatureError<W::Error>> {
        let [x, y, z] = origin;
        match self {
            Self::VoidStartPlatform => {
                let (cx, cz) = (x >> 4, z >> 4);
                if cx.wrapping_abs().max(cz.wrapping_abs()) > 1 {
                    return Ok(true);
                }
                let y = y.wrapping_add(3);
                for z in cz.wrapping_shl(4)..=cz.wrapping_shl(4).wrapping_add(15) {
                    for x in cx.wrapping_shl(4)..=cx.wrapping_shl(4).wrapping_add(15) {
                        if 8_i32
                            .wrapping_sub(x)
                            .wrapping_abs()
                            .max(8_i32.wrapping_sub(z).wrapping_abs())
                            <= 16
                        {
                            let state = if x == 8 && z == 8 {
                                blocks.cobblestone
                            } else {
                                blocks.stone
                            };
                            world
                                .set_block([x, y, z], state, 2)
                                .map_err(FeatureError::World)?;
                        }
                    }
                }
            }
            Self::EndIsland => {
                let mut radius = random.next_int_bound(3)? as f32 + 4.;
                let mut dy = 0_i32;
                while radius > 0.5 {
                    let low = (-radius).floor() as i32;
                    let high = radius.ceil() as i32;
                    for dx in low..=high {
                        for dz in low..=high {
                            if (dx * dx + dz * dz) as f32 <= (radius + 1.) * (radius + 1.) {
                                world
                                    .set_block(
                                        [
                                            x.wrapping_add(dx),
                                            y.wrapping_add(dy),
                                            z.wrapping_add(dz),
                                        ],
                                        blocks.end_stone,
                                        3,
                                    )
                                    .map_err(FeatureError::World)?;
                            }
                        }
                    }
                    radius -= random.next_int_bound(2)? as f32 + 0.5;
                    dy -= 1;
                }
            }
            Self::EndPlatform => {
                for dz in -2..=2 {
                    for dx in -2..=2 {
                        for dy in -1..3 {
                            let p = [x.wrapping_add(dx), y.wrapping_add(dy), z.wrapping_add(dz)];
                            let state = if dy == -1 {
                                blocks.obsidian
                            } else {
                                blocks.air
                            };
                            // Both target blocks have one state in the pinned registry.
                            if world.block(p).map_err(FeatureError::World)? != state {
                                world.set_block(p, state, 3).map_err(FeatureError::World)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(true)
    }
}
