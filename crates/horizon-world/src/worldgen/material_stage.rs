//! Surface-stage column traversal and pre-lighting chunk writes.
use super::{
    DecodeError,
    anchor::WorldGenerationContext,
    material_condition::{
        MaterialConditions, MaterialDensitySources, MaterialRule, MaterialRuleError,
        MaterialSurfaceQueries,
    },
    material_extensions::{ExtensionError, MaterialColumn, MaterialExtensions},
    terrain::{TerrainBlockStates, TerrainChunk, TerrainError},
};
use crate::{
    heightmap::{HeightmapState, HeightmapType},
    registry::BlockStateId,
};
use horizon_core::{
    density::{DensitySampler, SamplerContext},
    noise::DensityVolume,
    position::BlockPos,
    random::RandomSource,
};
use std::sync::Arc;

pub struct SurfaceInputs<'a, R: RandomSource, S: MaterialSurfaceQueries> {
    pub generation: WorldGenerationContext,
    pub noise_volume: DensityVolume,
    pub root: R::Positional,
    pub surface: S,
    pub preliminary_surface: Arc<dyn DensitySampler>,
    pub density_context: &'a mut SamplerContext,
    /// Inclusive generation write bounds, contained within the chunk.
    pub write_min_y: i32,
    pub write_max_y: i32,
}
pub struct TopMaterialInputs<'a, R: RandomSource, S: MaterialSurfaceQueries> {
    pub generation: WorldGenerationContext,
    pub root: R::Positional,
    pub surface: S,
    pub preliminary_surface: Arc<dyn DensitySampler>,
    pub density_context: &'a mut SamplerContext,
}
#[derive(Debug)]
pub enum SurfaceError {
    Terrain(TerrainError),
    Rule(MaterialRuleError),
    Decode(DecodeError),
    InvalidBounds,
    InvalidVolume,
    IterationLimit,
}
impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SurfaceError {}
impl From<TerrainError> for SurfaceError {
    fn from(e: TerrainError) -> Self {
        Self::Terrain(e)
    }
}
impl From<MaterialRuleError> for SurfaceError {
    fn from(e: MaterialRuleError) -> Self {
        Self::Rule(e)
    }
}
impl From<DecodeError> for SurfaceError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}
impl From<ExtensionError<SurfaceError>> for SurfaceError {
    fn from(e: ExtensionError<SurfaceError>) -> Self {
        match e {
            ExtensionError::Column(e) => e,
            ExtensionError::IterationLimit => Self::IterationLimit,
        }
    }
}

struct Column<'a, 'r, B: TerrainBlockStates> {
    chunk: &'a mut TerrainChunk<'r, B>,
    x: i32,
    z: i32,
    min: i32,
    max: i32,
    /// Resolved block ids for `low..=high`, reused across columns and updated by
    /// `set_block`. The surface walk reads a column twice (top to bottom, then the
    /// stone-depth look-behind); the palette decode is paid once per column.
    cache: &'a mut Vec<BlockStateId>,
    low: i32,
    high: i32,
}
impl<'a, 'r, B: TerrainBlockStates> Column<'a, 'r, B> {
    fn new(
        chunk: &'a mut TerrainChunk<'r, B>,
        x: i32,
        z: i32,
        min: i32,
        max: i32,
        top: i32,
        cache: &'a mut Vec<BlockStateId>,
    ) -> Self {
        let chunk_min = chunk.min_y();
        let chunk_max = chunk_min.wrapping_add(chunk.height()).wrapping_sub(1);
        let high = top.min(chunk_max);
        let low = min.max(chunk_min);
        cache.clear();
        if high >= low {
            cache.reserve((high - low + 1) as usize);
            for y in low..=high {
                cache.push(chunk.block(x, y, z));
            }
        }
        Self {
            chunk,
            x,
            z,
            min,
            max,
            cache,
            low,
            high,
        }
    }
    fn cache_index(&self, y: i32) -> Option<usize> {
        (y >= self.low && y <= self.high).then(|| (y - self.low) as usize)
    }
}
impl<B: TerrainBlockStates> MaterialColumn for Column<'_, '_, B> {
    type Error = SurfaceError;
    fn block(&mut self, y: i32) -> Result<BlockStateId, SurfaceError> {
        Ok(match self.cache_index(y) {
            Some(index) => self.cache[index],
            None => self.chunk.block(self.x, y, self.z),
        })
    }
    fn set_block(&mut self, y: i32, state: BlockStateId) -> Result<(), SurfaceError> {
        if y >= self.min && y <= self.max {
            // Most of the Nether's rule writes netherrack over netherrack.
            if self.block(y)? != state {
                self.chunk.set_block_worldgen(self.x, y, self.z, state)?;
                if let Some(index) = self.cache_index(y) {
                    self.cache[index] = state;
                }
            }
            if self.chunk.block_properties(state).has_fluid() {
                self.chunk.mark_postprocessing(self.x, y, self.z);
            }
        }
        Ok(())
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        self.chunk.block_properties(state).is_air()
    }
    fn same_block(&self, left: BlockStateId, right: BlockStateId) -> bool {
        self.chunk.same_block(left, right)
    }
}
fn first_available<B: TerrainBlockStates>(
    chunk: &mut TerrainChunk<'_, B>,
    x: i32,
    z: i32,
) -> Result<i32, SurfaceError> {
    if chunk.heightmap(HeightmapType::WorldSurfaceWg).is_none() {
        chunk.prime_heightmaps(&[HeightmapType::WorldSurfaceWg])?;
    }
    chunk
        .heightmap(HeightmapType::WorldSurfaceWg)
        .unwrap()
        .first_available(x, z)
        .map_err(TerrainError::from)
        .map_err(SurfaceError::from)
}
fn tick(remaining: &mut usize) -> Result<(), SurfaceError> {
    if *remaining == 0 {
        return Err(SurfaceError::IterationLimit);
    }
    *remaining -= 1;
    Ok(())
}

/// Apply a compiled top-material rule at one position with a worker the caller
/// built once. Carvers restore many cells per pass, so the rule is compiled and
/// the density bindings created once instead of per restored block.
pub(crate) fn top_material_cell<
    R: RandomSource,
    S: MaterialSurfaceQueries,
    B: TerrainBlockStates,
>(
    worker: &mut MaterialConditions<R, S>,
    chunk: &mut TerrainChunk<'_, B>,
    rule: usize,
    position: BlockPos,
    submerged: bool,
) -> Result<Option<BlockStateId>, SurfaceError> {
    let (lx, lz) = (position.x & 15, position.z & 15);
    let gx = first_available(chunk, (lx + 1).min(15), lz)?.wrapping_sub(first_available(
        chunk,
        (lx - 1).max(0),
        lz,
    )?);
    let gz = first_available(chunk, lx, (lz + 1).min(15))?.wrapping_sub(first_available(
        chunk,
        lx,
        (lz - 1).max(0),
    )?);
    worker.update_xz(position.x, position.z, gx, gz);
    worker.update_y(
        1,
        1,
        if submerged {
            position.y.wrapping_add(1)
        } else {
            i32::MIN
        },
        position.y,
    );
    worker.try_apply(rule, position).map_err(SurfaceError::from)
}

/// Evaluate the replacement surface under a carved block. Each call binds a
/// fresh one-cell material context, without restricting possible biomes, and
/// uses one stone block above/below. The caller applies the returned state.
/// Required heightmaps may be primed; block values are not changed here.
/// The caller's density context is retained on successful and failed evaluation.
pub fn top_material<R: RandomSource, S: MaterialSurfaceQueries, B: TerrainBlockStates>(
    chunk: &mut TerrainChunk<'_, B>,
    rule: MaterialRule,
    inputs: TopMaterialInputs<'_, R, S>,
    position: BlockPos,
    submerged: bool,
) -> Result<Option<BlockStateId>, SurfaceError> {
    let volume = DensityVolume::new([1; 3], [position.x, position.y, position.z], [1; 3])
        .map_err(|_| SurfaceError::InvalidVolume)?;
    let mut sources = MaterialDensitySources::new(
        inputs.surface,
        volume,
        inputs.preliminary_surface,
        SamplerContext::default(),
    )?;
    std::mem::swap(sources.sampler_context_mut(), inputs.density_context);
    let mut worker = MaterialConditions::<R, _>::with_possible_biomes(
        inputs.generation,
        inputs.root,
        sources,
        None,
    );
    let result = (|| {
        let rule = worker.add_rule(rule)?;
        top_material_cell(&mut worker, chunk, rule, position, submerged)
    })();
    let (_, mut context) = worker.into_surface().into_parts();
    std::mem::swap(&mut context, inputs.density_context);
    result
}

/// Build one chunk's surface in vanilla X/Z/descending-Y order. Required heightmaps
/// are updated during writes, so later columns observe earlier modifications.
/// Providers, rule roots, extensions and random settings must belong to one world.
/// Failures retain prior chunk writes and fluid postprocessing entries.
pub fn build_surface<R: RandomSource, S: MaterialSurfaceQueries, B: TerrainBlockStates>(
    chunk: &mut TerrainChunk<'_, B>,
    rule: MaterialRule,
    inputs: SurfaceInputs<'_, R, S>,
    extensions: &MaterialExtensions<R::Positional>,
) -> Result<(), SurfaceError>
where
    R::Positional: Clone,
{
    let min = chunk.min_y();
    let max = min + chunk.height() - 1;
    if inputs.write_min_y < min
        || inputs.write_max_y > max
        || inputs.write_min_y > inputs.write_max_y
    {
        return Err(SurfaceError::InvalidBounds);
    }
    let highest = chunk
        .highest_filled_section_index()
        .map_or(min - 1, |i| min + (i as i32) * 16 + 15);
    let sizes = inputs.noise_volume.sizes();
    let origin = inputs.noise_volume.minimum();
    let height = highest.wrapping_sub(origin[1]).wrapping_add(1).max(1);
    let expected = DensityVolume::new([sizes[0] as i32, height, sizes[2] as i32], origin, [1; 3])
        .map_err(|_| SurfaceError::InvalidVolume)?;
    let mut sources = MaterialDensitySources::new(
        inputs.surface,
        expected,
        inputs.preliminary_surface,
        SamplerContext::default(),
    )?;
    std::mem::swap(sources.sampler_context_mut(), inputs.density_context);
    let mut worker = MaterialConditions::<R, _>::new(inputs.generation, inputs.root, sources);
    let result = build_columns(
        chunk,
        rule,
        &mut worker,
        extensions,
        inputs.write_min_y,
        inputs.write_max_y,
    );
    let (_, mut context) = worker.into_surface().into_parts();
    std::mem::swap(&mut context, inputs.density_context);
    result
}
fn build_columns<R: RandomSource, S: MaterialSurfaceQueries, B: TerrainBlockStates>(
    chunk: &mut TerrainChunk<'_, B>,
    rule: MaterialRule,
    worker: &mut MaterialConditions<R, S>,
    extensions: &MaterialExtensions<R::Positional>,
    write_min_y: i32,
    write_max_y: i32,
) -> Result<(), SurfaceError>
where
    R::Positional: Clone,
{
    let min = chunk.min_y();
    let rule = worker.add_rule(rule)?;
    let variants = worker.surface_variants(rule)?;
    let cx = chunk.position().x.wrapping_shl(4);
    let cz = chunk.position().z.wrapping_shl(4);
    // One block-id buffer, refilled per column and kept across columns.
    let mut cache = Vec::new();
    for lx in 0..16 {
        for lz in 0..16 {
            let x = cx.wrapping_add(lx);
            let z = cz.wrapping_add(lz);
            let original_top = first_available(chunk, lx, lz)?;
            let biome = worker
                .biome_at(BlockPos::new(x, original_top, z))?
                .ok_or(MaterialRuleError::MissingBiome)?;
            if biome.as_ref() == "minecraft:eroded_badlands" {
                extensions.eroded_badlands(
                    &mut Column::new(
                        chunk,
                        x,
                        z,
                        write_min_y,
                        write_max_y,
                        original_top,
                        &mut cache,
                    ),
                    x,
                    z,
                    original_top,
                    min,
                )?;
            }
            let top = first_available(chunk, lx, lz)?;
            let gx = first_available(chunk, (lx + 1).min(15), lz)?.wrapping_sub(first_available(
                chunk,
                (lx - 1).max(0),
                lz,
            )?);
            let gz = first_available(chunk, lx, (lz + 1).min(15))?.wrapping_sub(first_available(
                chunk,
                lx,
                (lz - 1).max(0),
            )?);
            worker.update_xz(x, z, gx, gz);
            let mut above = 0i32;
            let mut water = i32::MIN;
            let mut bottom = i32::MAX;
            let mut y = top;
            let mut budget = 1 << 20;
            let mut column = Column::new(chunk, x, z, write_min_y, write_max_y, top, &mut cache);
            while y >= min {
                tick(&mut budget)?;
                let state = column.block(y)?;
                let properties = column.chunk.block_properties(state);
                if properties.is_air() {
                    above = 0;
                    water = i32::MIN;
                } else if properties.has_fluid() {
                    if water == i32::MIN {
                        water = y.wrapping_add(1);
                    }
                } else {
                    if bottom >= y {
                        bottom = -32512; // DimensionType.MIN_Y << 4 for 26.3.
                        let mut below = y.wrapping_sub(1);
                        while below >= min - 1 {
                            tick(&mut budget)?;
                            let state = column.block(below)?;
                            let properties = column.chunk.block_properties(state);
                            if properties.is_air() || properties.has_fluid() {
                                bottom = below.wrapping_add(1);
                                break;
                            }
                            below = below.wrapping_sub(1);
                        }
                    }
                    above = above.wrapping_add(1);
                    worker.update_y(above, y.wrapping_sub(bottom).wrapping_add(1), water, y);
                    if y >= write_min_y && y <= write_max_y {
                        let above = variants.surface && y >= worker.minimum_surface_level()?;
                        let rule = variants.rule(y, above);
                        if let Some(state) = worker.try_apply(rule, BlockPos::new(x, y, z))? {
                            column.set_block(y, state)?;
                        }
                    }
                }
                y = y.wrapping_sub(1);
            }
            if matches!(
                biome.as_ref(),
                "minecraft:frozen_ocean" | "minecraft:deep_frozen_ocean"
            ) {
                let minimum = worker.minimum_surface_level()?;
                extensions.frozen_ocean_with_melt(
                    &mut column,
                    [x, z],
                    original_top,
                    minimum,
                    |position, sea| {
                        worker
                            .melts_frozen_ocean(&biome, position, sea)
                            .map_err(SurfaceError::from)
                    },
                )?;
            }
        }
    }
    Ok(())
}
