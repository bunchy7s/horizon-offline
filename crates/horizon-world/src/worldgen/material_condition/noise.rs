//! Named material-noise sources derived from the world's positional factory.
use super::{MaterialRuleError, MaterialSurfaceQueries, codec::identifier};
use crate::{
    registry::BlockStateId,
    worldgen::{DecodeError, decode_noise},
};
use horizon_core::{
    noise::{Noise, NormalNoise},
    position::BlockPos,
    random::PositionalRandom,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Add lazy, shared named noise sources to a material surface provider.
/// Both providers must be configured for the same world seed and random mode.
pub struct MaterialNoiseSources<P: PositionalRandom, S: MaterialSurfaceQueries> {
    root: P,
    surface: S,
    definitions: BTreeMap<String, NormalNoise>,
    samplers: BTreeMap<String, Arc<dyn Noise>>,
}
impl<P: PositionalRandom, S: MaterialSurfaceQueries> MaterialNoiseSources<P, S> {
    pub fn from_json(
        root: P,
        surface: S,
        definitions: &BTreeMap<String, String>,
    ) -> Result<Self, DecodeError> {
        if definitions.len() > 8192 {
            return Err(DecodeError::Field("material noise registry size"));
        }
        let mut noises = BTreeMap::new();
        for (name, json) in definitions {
            if noises
                .insert(identifier(name)?, decode_noise(json)?)
                .is_some()
            {
                return Err(DecodeError::Field("duplicate material noise"));
            }
        }
        Ok(Self {
            root,
            surface,
            definitions: noises,
            samplers: BTreeMap::new(),
        })
    }
}
impl<P: PositionalRandom, S: MaterialSurfaceQueries> MaterialSurfaceQueries
    for MaterialNoiseSources<P, S>
{
    fn bind_density(
        &mut self,
        sampler: Arc<dyn horizon_core::density::DensitySampler>,
        eager: bool,
    ) -> Result<usize, DecodeError> {
        self.surface.bind_density(sampler, eager)
    }
    fn density(&mut self, index: usize, position: BlockPos) -> Result<f32, MaterialRuleError> {
        self.surface.density(index, position)
    }
    fn preliminary_surface(&mut self, position: BlockPos) -> Result<f32, MaterialRuleError> {
        self.surface.preliminary_surface(position)
    }
    fn possible_biomes(&self) -> Option<BTreeSet<String>> {
        self.surface.possible_biomes()
    }
    fn biome(&mut self, position: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
        self.surface.biome(position)
    }
    fn biome_id(&mut self, position: BlockPos) -> Result<Option<u32>, MaterialRuleError> {
        self.surface.biome_id(position)
    }
    fn biome_index(&self, name: &str) -> Option<Option<u32>> {
        self.surface.biome_index(name)
    }
    fn melts_frozen_ocean(
        &mut self,
        biome: &str,
        position: BlockPos,
        sea_level: i32,
    ) -> Result<bool, MaterialRuleError> {
        self.surface.melts_frozen_ocean(biome, position, sea_level)
    }
    fn cold_enough_to_snow(
        &mut self,
        biome: &str,
        position: BlockPos,
        sea_level: i32,
    ) -> Result<bool, MaterialRuleError> {
        self.surface.cold_enough_to_snow(biome, position, sea_level)
    }
    fn surface_depth(&mut self, x: i32, z: i32) -> i32 {
        self.surface.surface_depth(x, z)
    }
    fn surface_secondary(&mut self, x: i32, z: i32) -> f64 {
        self.surface.surface_secondary(x, z)
    }
    fn band(&mut self, position: BlockPos) -> Result<BlockStateId, MaterialRuleError> {
        self.surface.band(position)
    }
    fn bind_noise(&mut self, name: &str) -> Result<Arc<dyn Noise>, DecodeError> {
        let name = identifier(name)?;
        if let Some(noise) = self.samplers.get(&name) {
            return Ok(noise.clone());
        }
        let definition = self
            .definitions
            .get(&name)
            .ok_or(DecodeError::Field("unknown material noise"))?;
        let noise: Arc<dyn Noise> =
            Arc::new(definition.create(&mut self.root.from_hash_of(&name))?);
        self.samplers.insert(name, noise.clone());
        Ok(noise)
    }
}
