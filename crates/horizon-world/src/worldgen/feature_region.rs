//! Resident pre-light chunks shared by modifier queries and feature mutations.
//! The caller supplies world write policy/effects; storage alone cannot implement
//! neighbor updates, block entities, POIs or lighting.
use super::{
    anchor::WorldGenerationContext,
    biome_chunk::{BiomeRegion, BiomeRegionError},
    feature::FeatureWorld,
    placement::{LayerBlock, PlacementContext, PlacementQueryError},
    terrain::{TerrainBlockStates, TerrainChunk, TerrainError},
};
use crate::{
    fluid::FluidKind,
    heightmap::{HeightmapState, HeightmapType},
    registry::BlockStateId,
    tick_data::TickKind,
};
use horizon_core::{
    position::BlockPos,
    ticks::{
        ProtoChunkTicks, ScheduledTick, TickPriority, TickTypeId, WorldGenTickAccess,
        WorldGenTickContainers,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

struct RegionBlockQueries {
    fluids: super::fluid_predicate::FluidPredicateBindings,
    tags: Arc<BTreeMap<String, Arc<BTreeSet<BlockStateId>>>>,
}

#[derive(Debug)]
pub enum RegionError {
    InvalidChunks,
    InvalidBindings,
    InvalidTickState(BlockStateId),
    MissingChunk([i32; 2]),
    Terrain(TerrainError),
    Biome(BiomeRegionError),
    BiomesNotGenerated([i32; 2]),
}
impl std::fmt::Display for RegionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RegionError {}
impl From<TerrainError> for RegionError {
    fn from(error: TerrainError) -> Self {
        Self::Terrain(error)
    }
}
impl From<RegionError> for PlacementQueryError {
    fn from(error: RegionError) -> Self {
        Self::Failed(error.to_string())
    }
}
pub struct TerrainRegion<'r, B: TerrainBlockStates> {
    /// Ordered by X then Z.
    chunks: Vec<TerrainChunk<'r, B>>,
    /// Each chunk's index in `chunks`: every block access looks one up.
    index: ChunkIndex,
    min_y: i32,
    height: i32,
    generation: WorldGenerationContext,
    sea_level: i32,
    game_time: i64,
    next_tick_order: i64,
    block_queries: Option<RegionBlockQueries>,
    world_seed: Option<i64>,
}
/// Where each chunk of a region sits in its list: a grid over the chunks'
/// bounding box while it is small (the usual square of neighbours), else a
/// map (chunks spread far apart).
enum ChunkIndex {
    Grid {
        origin: (i32, i32),
        columns: i64,
        rows: i64,
        /// Row by row from `origin`; `u32::MAX` where no chunk is.
        cells: Vec<u32>,
    },
    Sparse(HashMap<(i32, i32), u32>),
}
impl ChunkIndex {
    /// The largest grid, in chunk columns.
    const GRID_CELLS: i64 = 1 << 20;
    /// `None` when two chunks share a position.
    fn new(positions: &[(i32, i32)]) -> Option<Self> {
        let x0 = positions.iter().map(|p| p.0).min()?;
        let z0 = positions.iter().map(|p| p.1).min()?;
        let columns = i64::from(positions.iter().map(|p| p.0).max()?) - i64::from(x0) + 1;
        let rows = i64::from(positions.iter().map(|p| p.1).max()?) - i64::from(z0) + 1;
        if columns * rows > Self::GRID_CELLS {
            let mut map = HashMap::with_capacity(positions.len());
            for (i, &p) in positions.iter().enumerate() {
                if map.insert(p, i as u32).is_some() {
                    return None;
                }
            }
            return Some(Self::Sparse(map));
        }
        let mut cells = vec![u32::MAX; (columns * rows) as usize];
        for (i, &(x, z)) in positions.iter().enumerate() {
            let cell = (i64::from(z) - i64::from(z0)) * columns + (i64::from(x) - i64::from(x0));
            let cell = &mut cells[cell as usize];
            if *cell != u32::MAX {
                return None;
            }
            *cell = i as u32;
        }
        Some(Self::Grid {
            origin: (x0, z0),
            columns,
            rows,
            cells,
        })
    }
    #[inline]
    fn get(&self, x: i32, z: i32) -> Option<usize> {
        match self {
            Self::Grid {
                origin,
                columns,
                rows,
                cells,
            } => {
                let (dx, dz) = (
                    i64::from(x) - i64::from(origin.0),
                    i64::from(z) - i64::from(origin.1),
                );
                if !(0..*columns).contains(&dx) || !(0..*rows).contains(&dz) {
                    return None;
                }
                match cells[(dz * columns + dx) as usize] {
                    u32::MAX => None,
                    i => Some(i as usize),
                }
            }
            Self::Sparse(map) => map.get(&(x, z)).map(|&i| i as usize),
        }
    }
}
impl<'r, B: TerrainBlockStates> TerrainRegion<'r, B> {
    pub fn new(
        chunks: Vec<TerrainChunk<'r, B>>,
        generator: WorldGenerationContext,
    ) -> Result<Self, RegionError> {
        if chunks.is_empty() || chunks.len() > 4096 {
            return Err(RegionError::InvalidChunks);
        }
        let first = &chunks[0];
        let (min_y, height, registry) = (first.min_y(), first.height(), first.registry());
        if chunks.iter().any(|c| {
            c.min_y() != min_y || c.height() != height || !std::ptr::eq(c.registry(), registry)
        }) {
            return Err(RegionError::InvalidChunks);
        }
        let mut chunks = chunks;
        chunks.sort_by_key(|c| (c.position().x, c.position().z));
        let positions: Vec<(i32, i32)> = chunks
            .iter()
            .map(|c| (c.position().x, c.position().z))
            .collect();
        let index = ChunkIndex::new(&positions).ok_or(RegionError::InvalidChunks)?;
        Ok(Self {
            chunks,
            index,
            sea_level: generator.sea_level,
            game_time: 0,
            next_tick_order: 0,
            block_queries: None,
            world_seed: None,
            min_y,
            height,
            generation: WorldGenerationContext::with_generator(
                min_y,
                height,
                generator.min_y,
                generator.height,
                generator.sea_level,
            ),
        })
    }
    /// Bind an immutable snapshot of fluid states and resolved block/fluid tags.
    /// Tables are shared with configured features; registry identities must match.
    pub fn with_block_bindings(
        mut self,
        bindings: &super::block_feature::FeatureBlockBindings<'r>,
    ) -> Result<Self, RegionError> {
        if !std::ptr::eq(self.chunks[0].registry(), bindings.registry()) {
            return Err(RegionError::InvalidBindings);
        }
        self.block_queries = Some(RegionBlockQueries {
            fluids: bindings.fluids.clone(),
            tags: bindings.tags.clone(),
        });
        Ok(self)
    }
    /// Supply the world seed (WorldGenLevel.getSeed) for seed-derived features.
    pub fn with_world_seed(mut self, seed: i64) -> Self {
        self.world_seed = Some(seed);
        self
    }
    /// Supply the owning world's current time and region-local next tick order.
    pub fn with_tick_clock(mut self, game_time: i64, next_tick_order: i64) -> Self {
        self.game_time = game_time;
        self.next_tick_order = next_tick_order;
        self
    }
    pub fn tick_clock(&self) -> (i64, i64) {
        (self.game_time, self.next_tick_order)
    }
    /// Raw tick identities are default block-state IDs for block ticks and
    /// `FluidKind` discriminants for fluid ticks, in separate namespaces.
    pub fn schedule_tick(
        &mut self,
        kind: TickKind,
        tick: ScheduledTick,
    ) -> Result<bool, RegionError> {
        WorldGenTickAccess::new(RegionTickContainers { region: self, kind }).schedule(tick)
    }
    fn request_tick(
        &mut self,
        kind: TickKind,
        position: [i32; 3],
        tick_type: TickTypeId,
        delay: i32,
    ) -> Result<(), RegionError> {
        let tick = ScheduledTick {
            kind: tick_type,
            pos: BlockPos::new(position[0], position[1], position[2]),
            trigger_tick: self.game_time.wrapping_add(i64::from(delay)),
            priority: TickPriority::Normal,
            sub_tick_order: self.next_tick_order,
        };
        self.next_tick_order = self.next_tick_order.wrapping_add(1);
        self.schedule_tick(kind, tick)?;
        Ok(())
    }
    /// Release the resident chunks, ordered by X then Z; match them by position.
    pub fn into_chunks(self) -> Vec<TerrainChunk<'r, B>> {
        self.chunks
    }
    pub fn chunks(&self) -> impl Iterator<Item = &TerrainChunk<'r, B>> {
        self.chunks.iter()
    }
    #[inline]
    fn index(&self, x: i32, z: i32) -> Option<usize> {
        self.index.get(x, z)
    }
    #[inline]
    pub fn chunk(&self, x: i32, z: i32) -> Result<&TerrainChunk<'r, B>, RegionError> {
        match self.index(x, z) {
            Some(i) => Ok(&self.chunks[i]),
            None => Err(RegionError::MissingChunk([x, z])),
        }
    }
    pub fn chunk_mut(&mut self, x: i32, z: i32) -> Result<&mut TerrainChunk<'r, B>, RegionError> {
        match self.index(x, z) {
            Some(i) => Ok(&mut self.chunks[i]),
            None => Err(RegionError::MissingChunk([x, z])),
        }
    }
    #[inline]
    pub fn block(&self, [x, y, z]: [i32; 3]) -> Result<BlockStateId, RegionError> {
        Ok(self.chunk(x >> 4, z >> 4)?.block(x, y, z))
    }
    /// `FeatureWorld::scan_column` within one chunk's column.
    pub fn scan_column(
        &self,
        [x, y, z]: [i32; 3],
        step: i32,
        limit: i32,
        keep: impl FnMut(BlockStateId) -> bool,
    ) -> Result<i32, RegionError> {
        Ok(self
            .chunk(x >> 4, z >> 4)?
            .scan_column([x, y, z], step, limit, keep))
    }
    /// Level-style biome lookup checks the selected chunk's generation status
    /// before reading its palette. Raw biome storage remains independently
    /// accessible for generation itself. Retained upgrade status permits reading
    /// old biomes before the new generation reaches the biome stage.
    pub fn biome_at<'b, T: Clone>(
        &self,
        biomes: &'b BiomeRegion<T>,
        position: BlockPos,
    ) -> Result<&'b T, RegionError> {
        self.noise_biome(biomes, biomes.quart_at_block(position))
    }
    pub fn noise_biome<'b, T: Clone>(
        &self,
        biomes: &'b BiomeRegion<T>,
        quart: [i32; 3],
    ) -> Result<&'b T, RegionError> {
        let chunk = self.chunk(quart[0] >> 2, quart[2] >> 2)?;
        if chunk.highest_generated_status() < crate::chunk_status::ChunkStatus::Biomes {
            return Err(RegionError::BiomesNotGenerated([
                quart[0] >> 2,
                quart[2] >> 2,
            ]));
        }
        biomes.noise_biome(quart).map_err(RegionError::Biome)
    }
    /// Storage mutation only. World code must apply its write permissions,
    /// block-entity/POI effects and postprocessing around this operation.
    pub fn set_block_worldgen(
        &mut self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
    ) -> Result<BlockStateId, RegionError> {
        Ok(self
            .chunk_mut(x >> 4, z >> 4)?
            .set_block_worldgen(x, y, z, state)?)
    }
}
struct RegionTickContainers<'a, 'r, B: TerrainBlockStates> {
    region: &'a mut TerrainRegion<'r, B>,
    kind: TickKind,
}
impl<B: TerrainBlockStates> WorldGenTickContainers for RegionTickContainers<'_, '_, B> {
    type Container = ProtoChunkTicks;
    type Error = RegionError;
    fn container_at(&mut self, p: BlockPos) -> Result<&mut ProtoChunkTicks, RegionError> {
        let chunk = self.region.chunk_mut(p.x >> 4, p.z >> 4)?;
        Ok(match self.kind {
            TickKind::Block => chunk.block_ticks_mut(),
            TickKind::Fluid => chunk.fluid_ticks_mut(),
        })
    }
}
impl<B: TerrainBlockStates> PlacementContext for TerrainRegion<'_, B> {
    fn fluid_state(
        &mut self,
        position: [i32; 3],
    ) -> Result<crate::fluid::FluidState, PlacementQueryError> {
        let queries = self
            .block_queries
            .as_ref()
            .ok_or(PlacementQueryError::Unavailable)?;
        let state = self.block(position)?;
        queries
            .fluids
            .state(state)
            .ok_or_else(|| PlacementQueryError::Failed("fluid state outside bound registry".into()))
    }
    fn water_at(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        let queries = self
            .block_queries
            .as_ref()
            .ok_or(PlacementQueryError::Unavailable)?;
        Ok(queries
            .fluids
            .state_matches_tag(self.block(position)?, "minecraft:water"))
    }
    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        let queries = self
            .block_queries
            .as_ref()
            .ok_or(PlacementQueryError::Unavailable)?;
        let state = self.block(position)?;
        Ok(queries
            .tags
            .get(tag)
            .is_some_and(|states| states.contains(&state)))
    }

    fn schedule_block_tick(
        &mut self,
        position: [i32; 3],
        block: BlockStateId,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        let registry = self.chunks[0].registry();
        let state = registry
            .state(block)
            .ok_or(RegionError::InvalidTickState(block))?;
        // Default state IDs provide one identity per block, separate from fluids.
        let block = registry
            .default_state(&state.name)
            .ok_or(RegionError::InvalidTickState(block))?;
        self.request_tick(TickKind::Block, position, TickTypeId(block.0), delay)
            .map_err(Into::into)
    }
    fn schedule_fluid_tick(
        &mut self,
        position: [i32; 3],
        fluid: FluidKind,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.request_tick(TickKind::Fluid, position, TickTypeId(fluid as u32), delay)
            .map_err(Into::into)
    }

    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.sea_level)
    }
    fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
        self.world_seed.ok_or(PlacementQueryError::Unavailable)
    }

    fn mark_postprocessing(&mut self, [x, y, z]: [i32; 3]) -> Result<(), PlacementQueryError> {
        self.chunk_mut(x >> 4, z >> 4)?.mark_postprocessing(x, y, z);
        Ok(())
    }
    fn generation(&self) -> WorldGenerationContext {
        self.generation
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.block(position).map_err(Into::into)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.min_y)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.min_y.wrapping_add(self.height).wrapping_sub(1))
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        let chunk = self.chunk_mut(x >> 4, z >> 4)?;
        if chunk.heightmap(kind).is_none() {
            chunk.prime_heightmaps(&[kind]).map_err(RegionError::from)?;
        }
        chunk
            .heightmap(kind)
            .unwrap()
            .first_available(x & 15, z & 15)
            .map_err(|e| RegionError::from(TerrainError::from(e)).into())
    }
    fn layer_block(&mut self, position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        let chunk = self.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        let name = &chunk
            .registry()
            .state(state)
            .expect("registered chunk state")
            .name;
        Ok(
            if chunk.block_properties(state).is_air()
                || name == "minecraft:water"
                || name == "minecraft:lava"
            {
                LayerBlock::Empty
            } else if name == "minecraft:bedrock" {
                LayerBlock::Bedrock
            } else {
                LayerBlock::Ground
            },
        )
    }
}
/// Gives feature algorithms and placement queries one region. All update flags
/// reach the supplied write function, which owns world mutation policy/effects.
pub struct TerrainFeatureAccess<'a, 'r, B: TerrainBlockStates, F> {
    region: &'a mut TerrainRegion<'r, B>,
    write: F,
    write_zone: Option<super::write_zone::GenerationWriteZone>,
    upgrade_height: UpgradeHeightPolicy,
}
enum UpgradeHeightPolicy {
    Unrestricted,
    Fixed(super::write_zone::UpgradeWriteHeight),
    Center(horizon_core::position::ChunkPos),
}
impl<'a, 'r, B: TerrainBlockStates, F> TerrainFeatureAccess<'a, 'r, B, F> {
    pub fn new(region: &'a mut TerrainRegion<'r, B>, write: F) -> Self {
        Self {
            region,
            write,
            write_zone: None,
            upgrade_height: UpgradeHeightPolicy::Unrestricted,
        }
    }
    /// Apply the horizontal write radius before invoking mutation policy/effects.
    pub fn with_write_zone(mut self, zone: super::write_zone::GenerationWriteZone) -> Self {
        self.write_zone = Some(zone);
        self
    }
    /// Use an explicit height interval. For live generation lifecycle state,
    /// prefer `with_upgrade_center`, which observes changes made by callbacks.
    pub fn with_upgrade_height(mut self, height: super::write_zone::UpgradeWriteHeight) -> Self {
        self.upgrade_height = UpgradeHeightPolicy::Fixed(height);
        self
    }
    /// Read the resident center chunk's upgrade state before each write. The
    /// target chunk's upgrade state does not select the region's write height.
    /// Horizontal write radius is configured independently with `with_write_zone`.
    pub fn with_upgrade_center(
        mut self,
        center: horizon_core::position::ChunkPos,
    ) -> Result<Self, RegionError> {
        self.region.chunk(center.x, center.z)?;
        self.upgrade_height = UpgradeHeightPolicy::Center(center);
        Ok(self)
    }
}
impl<'r, B: TerrainBlockStates, F> TerrainFeatureAccess<'_, 'r, B, F>
where
    F: FnMut(&mut TerrainRegion<'r, B>, [i32; 3], BlockStateId, i32) -> Result<bool, RegionError>,
{
    fn write_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, RegionError> {
        if !self.can_write(position)? {
            return Ok(false);
        }
        (self.write)(self.region, position, state, flags)
    }
}
impl<'r, B: TerrainBlockStates, F> TerrainFeatureAccess<'_, 'r, B, F> {
    /// WorldGenRegion.ensureCanWrite without its diagnostic logging.
    fn can_write(&mut self, position: [i32; 3]) -> Result<bool, RegionError> {
        if self.write_zone.is_some_and(|zone| {
            !zone.contains_block(BlockPos::new(position[0], position[1], position[2]))
        }) {
            return Ok(false);
        }
        let height = match self.upgrade_height {
            UpgradeHeightPolicy::Unrestricted => None,
            UpgradeHeightPolicy::Fixed(height) => Some(height),
            UpgradeHeightPolicy::Center(center) => {
                let chunk = self.region.chunk(center.x, center.z)?;
                chunk.is_upgrading().then(|| {
                    let (min_y, height) = chunk.height_for_generation();
                    super::write_zone::UpgradeWriteHeight::new(min_y, height)
                })
            }
        };
        Ok(height.is_none_or(|height| height.contains(position[1])))
    }
}
impl<'r, B: TerrainBlockStates, F> PlacementContext for TerrainFeatureAccess<'_, 'r, B, F>
where
    F: FnMut(&mut TerrainRegion<'r, B>, [i32; 3], BlockStateId, i32) -> Result<bool, RegionError>,
{
    fn destroy_block_without_entity(
        &mut self,
        position: [i32; 3],
        _drop_resources: bool,
    ) -> Result<bool, PlacementQueryError> {
        // WorldGenRegion destruction is an air replacement through its write
        // policy. It does not evaluate drops, unlike live-level destruction.
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        if chunk.block_properties(state).is_air() {
            return Ok(false);
        }
        let air = chunk
            .registry()
            .default_state("minecraft:air")
            .expect("validated air state");
        self.write_block(position, air, 3).map_err(Into::into)
    }

    fn fluid_state(
        &mut self,
        position: [i32; 3],
    ) -> Result<crate::fluid::FluidState, PlacementQueryError> {
        self.region.fluid_state(position)
    }
    fn ensure_can_write(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.can_write(position).map_err(Into::into)
    }
    fn section_block(
        &mut self,
        position: [i32; 3],
    ) -> Result<Option<BlockStateId>, PlacementQueryError> {
        if position[1] < self.region.min_y
            || position[1] >= self.region.min_y.wrapping_add(self.region.height)
        {
            return Ok(None);
        }
        self.region.block(position).map(Some).map_err(Into::into)
    }
    fn set_section_block(
        &mut self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.region
            .chunk_mut(x >> 4, z >> 4)?
            .set_block_raw(x, y, z, state)
            .map_err(RegionError::from)?;
        Ok(())
    }
    /// ProtoChunk.setBlockState: the section and the stage's heightmaps.
    fn set_chunk_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.region.set_block_worldgen(position, state)?;
        Ok(())
    }
    fn water_at(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.region.water_at(position)
    }
    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        self.region.block_matches_tag(position, tag)
    }

    fn schedule_block_tick(
        &mut self,
        position: [i32; 3],
        block: BlockStateId,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.region.schedule_block_tick(position, block, delay)
    }
    fn schedule_fluid_tick(
        &mut self,
        position: [i32; 3],
        fluid: FluidKind,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.region.schedule_fluid_tick(position, fluid, delay)
    }

    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.sea_level()
    }
    fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
        self.region.world_seed()
    }

    fn mark_postprocessing(&mut self, position: [i32; 3]) -> Result<(), PlacementQueryError> {
        self.region.mark_postprocessing(position)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.region.generation()
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.region.block_state(position)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.min_y()
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.max_y()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        self.region.height(kind, x, z)
    }
    fn layer_block(&mut self, position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        self.region.layer_block(position)
    }
}
impl<'r, B: TerrainBlockStates, F> FeatureWorld for TerrainFeatureAccess<'_, 'r, B, F>
where
    F: FnMut(&mut TerrainRegion<'r, B>, [i32; 3], BlockStateId, i32) -> Result<bool, RegionError>,
{
    type Error = RegionError;
    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        self.region.block(position)
    }
    fn scan_column(
        &mut self,
        position: [i32; 3],
        step: i32,
        limit: i32,
        keep: impl FnMut(BlockStateId) -> bool,
    ) -> Result<i32, Self::Error> {
        self.region.scan_column(position, step, limit, keep)
    }
    fn set_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, Self::Error> {
        self.write_block(position, state, flags)
    }
}
