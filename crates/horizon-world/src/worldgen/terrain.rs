//! Noise-stage chunk filling. Material rules, structures and chunk lifecycle are
//! separate stages; this module does not produce a complete generated chunk.
use super::aquifer::{
    AquiferBlockStates, AquiferError, DisabledAquifer, FluidPicker, NoiseBasedAquifer,
};
use crate::{
    chunk::BlockSection,
    heightmap::{Heightmap, HeightmapChunk, HeightmapError, HeightmapState, HeightmapType},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    density::{DensitySampler, SampleError, SamplerContext},
    noise::DensityVolume,
    position::ChunkPos,
    random::PositionalRandom,
    ticks::ProtoChunkTicks,
};
use std::io;

/// Behavior and resolved tags for every state in the chunk's immutable registry.
pub trait TerrainBlockStates {
    type State: HeightmapState;
    fn heightmap_state(&self, id: BlockStateId) -> Self::State;
}

pub trait TerrainAquifer {
    fn compute_substance(
        &mut self,
        context: &mut SamplerContext,
        position: [i32; 3],
        density: f64,
    ) -> Result<Option<BlockStateId>, AquiferError>;
    fn should_schedule_fluid_update(&self) -> bool;
    /// The y above which every non-positive density is the aquifer's air. The
    /// fill skips those blocks without asking the aquifer; `None` keeps the
    /// per-block call for aquifers whose global fluid can vary with x/z.
    fn air_above_y(&self) -> Option<i32> {
        None
    }
    /// The fill decided this block without the aquifer (positive density or air
    /// above its skip height). Both old paths cleared the pending fluid update.
    fn note_skipped(&mut self) {}
}
impl<P: FluidPicker> TerrainAquifer for DisabledAquifer<P> {
    fn compute_substance(
        &mut self,
        _: &mut SamplerContext,
        position: [i32; 3],
        density: f64,
    ) -> Result<Option<BlockStateId>, AquiferError> {
        Ok(DisabledAquifer::compute_substance(self, position, density))
    }
    fn should_schedule_fluid_update(&self) -> bool {
        false
    }
}
impl<P: FluidPicker, F: PositionalRandom, B: AquiferBlockStates> TerrainAquifer
    for NoiseBasedAquifer<P, F, B>
{
    fn compute_substance(
        &mut self,
        context: &mut SamplerContext,
        position: [i32; 3],
        density: f64,
    ) -> Result<Option<BlockStateId>, AquiferError> {
        NoiseBasedAquifer::compute_substance(self, context, position, density)
    }
    fn should_schedule_fluid_update(&self) -> bool {
        NoiseBasedAquifer::should_schedule_fluid_update(self)
    }
    fn air_above_y(&self) -> Option<i32> {
        NoiseBasedAquifer::air_above_y(self)
    }
    fn note_skipped(&mut self) {
        NoiseBasedAquifer::note_skipped(self);
    }
}

#[derive(Debug)]
pub enum TerrainError {
    Storage(io::Error),
    Heightmap(HeightmapError),
    Sample(SampleError),
    Aquifer(AquiferError),
    MissingAir,
    InvalidState,
    InvalidVolume,
    OutsideBuildHeight,
}
impl std::fmt::Display for TerrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TerrainError {}
impl From<io::Error> for TerrainError {
    fn from(value: io::Error) -> Self {
        Self::Storage(value)
    }
}
impl From<HeightmapError> for TerrainError {
    fn from(value: HeightmapError) -> Self {
        Self::Heightmap(value)
    }
}
impl From<SampleError> for TerrainError {
    fn from(value: SampleError) -> Self {
        Self::Sample(value)
    }
}
impl From<AquiferError> for TerrainError {
    fn from(value: AquiferError) -> Self {
        Self::Aquifer(value)
    }
}

/// Resident registry-backed sections, optional heightmaps and ordered per-section
/// fluid postprocessing offsets for the noise and surface generation stages.
pub struct TerrainChunk<'r, B: TerrainBlockStates> {
    stage: PreLightChunkStage,
    below_zero_retrogen: Option<crate::retrogen::BelowZeroRetrogen>,
    position: ChunkPos,
    blocks: TerrainBlocks<'r, B>,
    heightmaps: [Option<Heightmap>; 6],
    postprocessing: Vec<Vec<u16>>,
    block_ticks: ProtoChunkTicks,
    fluid_ticks: ProtoChunkTicks,
}
/// The persisted generation statuses before lighting initialization. This models
/// status-dependent storage and upgrade cleanup, not scheduling, lighting or
/// save-dirty notifications.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreLightChunkStage {
    #[default]
    Empty,
    StructureStarts,
    StructureReferences,
    Biomes,
    Terrain,
    Features,
}
impl PreLightChunkStage {
    pub const ALL: [Self; 6] = [
        Self::Empty,
        Self::StructureStarts,
        Self::StructureReferences,
        Self::Biomes,
        Self::Terrain,
        Self::Features,
    ];
    pub fn status(self) -> crate::chunk_status::ChunkStatus {
        crate::chunk_status::ChunkStatus::ALL[self as usize]
    }
    pub fn name(self) -> &'static str {
        self.status().name().strip_prefix("minecraft:").unwrap()
    }
    pub fn heightmaps(self) -> &'static [HeightmapType] {
        self.status().heightmaps()
    }
}
struct TerrainBlocks<'r, B: TerrainBlockStates> {
    min_y: i32,
    height: i32,
    sections: Vec<BlockSection<'r>>,
    non_air: Vec<u16>,
    registry: &'r BlockRegistry,
    properties: B,
    air: BlockStateId,
    void_air: BlockStateId,
}
impl<B: TerrainBlockStates> HeightmapChunk for TerrainBlocks<'_, B> {
    type State = B::State;
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.height
    }
    fn highest_section_y(&self) -> i32 {
        self.non_air
            .iter()
            .rposition(|&count| count != 0)
            .map_or(self.min_y, |i| self.min_y + i as i32 * 16)
    }
    fn block_state(&self, x: i32, y: i32, z: i32) -> B::State {
        self.properties.heightmap_state(self.block(x, y, z))
    }
}
impl<B: TerrainBlockStates> TerrainBlocks<'_, B> {
    #[inline]
    fn section_index(&self, y: i32) -> Option<usize> {
        let dy = i64::from(y) - i64::from(self.min_y);
        (dy >= 0 && dy < i64::from(self.height)).then_some((dy >> 4) as usize)
    }
    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        let Some(section) = self.section_index(y) else {
            return self.void_air;
        };
        if self.non_air[section] == 0 {
            return self.air;
        }
        self.sections[section].state(local_index(x, y, z))
    }
}
#[inline]
fn local_index(x: i32, y: i32, z: i32) -> usize {
    (((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize
}

impl<'r, B: TerrainBlockStates> TerrainChunk<'r, B> {
    pub fn new(
        position: ChunkPos,
        min_y: i32,
        height: i32,
        registry: &'r BlockRegistry,
        properties: B,
    ) -> Result<Self, TerrainError> {
        // Share the heightmap's supported range and fail before allocating sections.
        Heightmap::new(HeightmapType::WorldSurfaceWg, min_y, height)?;
        let air = registry
            .default_state("minecraft:air")
            .ok_or(TerrainError::MissingAir)?;
        let void_air = registry
            .default_state("minecraft:void_air")
            .ok_or(TerrainError::MissingAir)?;
        let section_count = (height / 16) as usize;
        let sections = (0..section_count)
            .map(|_| BlockSection::uniform(air, registry))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            stage: PreLightChunkStage::Empty,
            below_zero_retrogen: None,
            position,
            blocks: TerrainBlocks {
                min_y,
                height,
                sections,
                non_air: vec![0; section_count],
                registry,
                properties,
                air,
                void_air,
            },
            heightmaps: std::array::from_fn(|_| None),
            postprocessing: vec![Vec::new(); section_count],
            block_ticks: ProtoChunkTicks::default(),
            fluid_ticks: ProtoChunkTicks::default(),
        })
    }
    pub(crate) fn registry(&self) -> &BlockRegistry {
        self.blocks.registry
    }
    pub fn highest_filled_section_index(&self) -> Option<usize> {
        self.blocks.non_air.iter().rposition(|&count| count != 0)
    }
    pub fn block_properties(&self, state: BlockStateId) -> B::State {
        self.blocks.properties.heightmap_state(state)
    }
    pub fn same_block(&self, left: BlockStateId, right: BlockStateId) -> bool {
        self.blocks
            .registry
            .state(left)
            .zip(self.blocks.registry.state(right))
            .is_some_and(|(left, right)| left.name == right.name)
    }
    pub fn generation_stage(&self) -> PreLightChunkStage {
        self.stage
    }
    /// Retained old terrain may already have completed stages that the new
    /// below-zero generation has not reached. Biome reads use this status.
    pub fn highest_generated_status(&self) -> crate::chunk_status::ChunkStatus {
        self.below_zero_retrogen
            .as_ref()
            .map_or(self.stage.status(), |data| {
                self.stage.status().max(data.target_status())
            })
    }
    /// Changing status alone does not prime or clear any heightmaps. Native
    /// persisted-status assignment also permits earlier statuses. Reaching an
    /// installed upgrade target clears its metadata; save-dirty tracking is separate.
    pub fn set_generation_stage(&mut self, stage: PreLightChunkStage) {
        self.stage = stage;
        if self
            .below_zero_retrogen
            .as_ref()
            .is_some_and(|data| stage.status() >= data.target_status())
        {
            self.below_zero_retrogen = None;
        }
    }
    pub fn below_zero_retrogen(&self) -> Option<&crate::retrogen::BelowZeroRetrogen> {
        self.below_zero_retrogen.as_ref()
    }
    /// Installation alone does not compare the target with the current status.
    /// A subsequent persisted-stage assignment performs completion cleanup.
    pub fn set_below_zero_retrogen(&mut self, data: Option<crate::retrogen::BelowZeroRetrogen>) {
        self.below_zero_retrogen = data;
    }
    pub fn is_upgrading(&self) -> bool {
        self.below_zero_retrogen.is_some()
    }
    /// Generation minimum Y and height. Storage bounds remain unchanged.
    pub fn height_for_generation(&self) -> (i32, i32) {
        if self.is_upgrading() {
            (-64, 64)
        } else {
            (self.min_y(), self.height())
        }
    }
    /// ProtoChunk writes before lighting. The current stage selects heightmaps;
    /// missing maps are primed after existing maps are updated.
    pub fn set_block_worldgen(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        state: BlockStateId,
    ) -> Result<BlockStateId, TerrainError> {
        let Some(section) = self.blocks.section_index(y) else {
            return Ok(self.blocks.void_air);
        };
        if self.blocks.non_air[section] == 0 && state == self.blocks.air {
            return Ok(state);
        }
        let previous = self.set_block_raw(x, y, z, state)?;
        let mut missing = Vec::new();
        for &kind in self.stage.heightmaps() {
            if self.heightmap(kind).is_some() {
                self.update_heightmap(kind, x & 15, y, z & 15, state)?;
            } else {
                missing.push(kind);
            }
        }
        if !missing.is_empty() {
            self.prime_heightmaps(&missing)?;
        }
        Ok(previous)
    }
    pub fn position(&self) -> ChunkPos {
        self.position
    }
    pub fn min_y(&self) -> i32 {
        self.blocks.min_y
    }
    pub fn height(&self) -> i32 {
        self.blocks.height
    }
    pub fn highest_section_y(&self) -> i32 {
        self.blocks.highest_section_y()
    }
    /// ProtoChunk reads collapse an all-air section to its default AIR state.
    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.blocks.block(x, y, z)
    }
    /// `FeatureWorld::scan_column` in this chunk: an all-air section is
    /// tested once, a mixed one read without looking the section up per block.
    pub fn scan_column(
        &self,
        [x, mut y, z]: [i32; 3],
        step: i32,
        mut limit: i32,
        mut keep: impl FnMut(BlockStateId) -> bool,
    ) -> i32 {
        debug_assert!(step == 1 || step == -1);
        let blocks = &self.blocks;
        while limit > 0 {
            let Some(section) = blocks.section_index(y) else {
                if !keep(blocks.void_air) {
                    return y;
                }
                y = y.wrapping_add(step);
                limit -= 1;
                continue;
            };
            // Blocks from y to this section's end in the walk's direction.
            let within = (y - blocks.min_y) & 15;
            let run = if step > 0 { 16 - within } else { within + 1 }.min(limit);
            limit -= run;
            if blocks.non_air[section] == 0 {
                if !keep(blocks.air) {
                    return y;
                }
                y = y.wrapping_add(step * run);
                continue;
            }
            let storage = &blocks.sections[section];
            for _ in 0..run {
                if !keep(storage.state(local_index(x, y, z))) {
                    return y;
                }
                y = y.wrapping_add(step);
            }
        }
        y
    }
    pub fn section(&self, index: usize) -> Option<&BlockSection<'r>> {
        self.blocks.sections.get(index)
    }
    pub fn heightmap(&self, kind: HeightmapType) -> Option<&Heightmap> {
        self.heightmaps[kind as usize].as_ref()
    }
    pub fn block_ticks(&self) -> &ProtoChunkTicks {
        &self.block_ticks
    }
    pub fn block_ticks_mut(&mut self) -> &mut ProtoChunkTicks {
        &mut self.block_ticks
    }
    pub fn fluid_ticks(&self) -> &ProtoChunkTicks {
        &self.fluid_ticks
    }
    pub fn fluid_ticks_mut(&mut self) -> &mut ProtoChunkTicks {
        &mut self.fluid_ticks
    }
    pub fn postprocessing(&self) -> &[Vec<u16>] {
        &self.postprocessing
    }

    pub fn ensure_heightmap(&mut self, kind: HeightmapType) -> &mut Heightmap {
        self.heightmaps[kind as usize].get_or_insert_with(|| {
            Heightmap::new(kind, self.blocks.min_y, self.blocks.height)
                .expect("validated chunk range")
        })
    }
    /// Section write only. The caller chooses which generation-stage maps to update.
    pub fn set_block_raw(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        state: BlockStateId,
    ) -> Result<BlockStateId, TerrainError> {
        let section = self
            .blocks
            .section_index(y)
            .ok_or(TerrainError::OutsideBuildHeight)?;
        let old = self.blocks.sections[section].set(local_index(x, y, z), state)?;
        let old_air = self.blocks.properties.heightmap_state(old).is_air();
        let new_air = self.blocks.properties.heightmap_state(state).is_air();
        if old_air && !new_air {
            self.blocks.non_air[section] += 1;
        } else if !old_air && new_air {
            self.blocks.non_air[section] -= 1;
        }
        Ok(old)
    }
    /// Write a vertical run of one non-air state from `y_top` downward, all
    /// inside the chunk. Returns how many blocks changed from air; section
    /// non-air counts are updated here and the caller owns the heightmaps.
    pub fn fill_solid_run(
        &mut self,
        x: i32,
        y_top: i32,
        z: i32,
        len: i32,
        state: BlockStateId,
    ) -> Result<u16, TerrainError> {
        let air = self.blocks.air;
        let mut added = 0u16;
        let mut top = y_top;
        let mut left = len;
        while left > 0 {
            let section = self
                .blocks
                .section_index(top)
                .ok_or(TerrainError::OutsideBuildHeight)?;
            let local_top = top & 15;
            let take = (local_top + 1).min(left);
            let local_start = local_top - take + 1;
            let now = self.blocks.sections[section].fill_run(
                x & 15,
                local_start,
                z & 15,
                take,
                state,
                air,
            )?;
            self.blocks.non_air[section] = self.blocks.non_air[section].saturating_add(now);
            added = added.saturating_add(now);
            top -= take;
            left -= take;
        }
        Ok(added)
    }
    pub fn update_heightmap(
        &mut self,
        kind: HeightmapType,
        x: i32,
        y: i32,
        z: i32,
        state: BlockStateId,
    ) -> Result<bool, TerrainError> {
        if self.blocks.registry.state(state).is_none() {
            return Err(TerrainError::InvalidState);
        }
        self.ensure_heightmap(kind);
        let state = self.blocks.properties.heightmap_state(state);
        Ok(self.heightmaps[kind as usize]
            .as_mut()
            .unwrap()
            .update(&self.blocks, x, y, z, state)?)
    }
    pub fn prime_heightmaps(&mut self, kinds: &[HeightmapType]) -> Result<(), TerrainError> {
        let mut order = Vec::new();
        for &kind in kinds {
            self.ensure_heightmap(kind);
            if !order.contains(&(kind as usize)) {
                order.push(kind as usize);
            }
        }
        // Split references preserve the supplied set's iteration order.
        let mut maps: Vec<_> = self
            .heightmaps
            .iter_mut()
            .enumerate()
            .filter_map(|(i, map)| {
                order
                    .iter()
                    .position(|&key| key == i)
                    .map(|position| (position, map.as_mut().unwrap()))
            })
            .collect();
        maps.sort_by_key(|(position, _)| *position);
        Heightmap::prime(
            &self.blocks,
            &mut maps.into_iter().map(|(_, map)| map).collect::<Vec<_>>(),
        )?;
        Ok(())
    }
    /// Before replaying a saved chunk's `PostProcessing` lists.
    pub fn clear_postprocessing(&mut self) {
        self.postprocessing.iter_mut().for_each(Vec::clear);
    }
    /// Preserves duplicates and insertion order, as ProtoChunk does.
    pub fn mark_postprocessing(&mut self, x: i32, y: i32, z: i32) {
        if let Some(section) = self.blocks.section_index(y) {
            self.postprocessing[section]
                .push(((x & 15) | ((y & 15) << 4) | ((z & 15) << 8)) as u16);
        }
    }
}

/// Evaluate final density in bulk before writes, then visit Z/X/descending Y.
/// The aquifer and final density must share this already prepared sampler context.
/// Failures retain earlier writes and aquifer/cache state; there is no rollback.
/// Regular gameplay only: developer preliminary-surface markers are not applied.
pub fn fill_noise<B: TerrainBlockStates>(
    chunk: &mut TerrainChunk<'_, B>,
    volume: DensityVolume,
    final_density: &dyn DensitySampler,
    context: &mut SamplerContext,
    aquifer: &mut (impl TerrainAquifer + ?Sized),
    default_block: BlockStateId,
) -> Result<(), TerrainError> {
    let [sx, sy, sz] = volume.sizes();
    let [bx, by, bz] = volume.minimum();
    if sx != 16
        || sz != 16
        || volume.steps() != [1; 3]
        || bx != chunk.position.x.wrapping_shl(4)
        || bz != chunk.position.z.wrapping_shl(4)
        || by < chunk.min_y()
        || i64::from(by) + sy as i64 > i64::from(chunk.min_y()) + i64::from(chunk.height())
        || chunk.blocks.registry.state(default_block).is_none()
    {
        return Err(TerrainError::InvalidVolume);
    }
    chunk.ensure_heightmap(HeightmapType::OceanFloorWg);
    chunk.ensure_heightmap(HeightmapType::WorldSurfaceWg);
    let mut density = vec![0.; volume.size()];
    final_density.sample_volume(context, &mut density, volume)?;
    let air_above = aquifer.air_above_y();
    // A non-air, non-fluid default block may be written as a run without asking
    // the aquifer or touching the heightmap below the run's top.
    let solid_runs = default_block != chunk.blocks.air
        && !chunk
            .blocks
            .properties
            .heightmap_state(default_block)
            .has_fluid();
    const MAPS: [HeightmapType; 2] = [HeightmapType::OceanFloorWg, HeightmapType::WorldSurfaceWg];
    for z in 0..sz {
        let world_z = bz.wrapping_add(z as i32);
        for x in 0..sx {
            let world_x = bx.wrapping_add(x as i32);
            // Heightmap updates at or below first-available - 2 are no-ops; the
            // bound only changes through an update, so skip those calls.
            let mut settled = MAPS.map(|kind| {
                chunk.heightmaps[kind as usize]
                    .as_ref()
                    .expect("ensured heightmap")
                    .first_available(x as i32, z as i32)
                    .map(|first| first.wrapping_sub(2))
            });
            let mut y = sy;
            while y > 0 {
                y -= 1;
                let world_y = by.wrapping_add(y as i32);
                let value = density[volume.index([x, y, z]).expect("valid density coordinates")];
                // A positive-density run is entirely the default block. Write it
                // through the section once instead of per block; the heightmap
                // only changes at the run's top.
                if solid_runs && value > 0.0 {
                    let mut run = 1usize;
                    while run <= y
                        && density[volume
                            .index([x, y - run, z])
                            .expect("valid density coordinates")]
                            > 0.0
                    {
                        run += 1;
                    }
                    aquifer.note_skipped();
                    chunk.fill_solid_run(x as i32, world_y, z as i32, run as i32, default_block)?;
                    for (kind, settled) in MAPS.into_iter().zip(&mut settled) {
                        if matches!(settled, Ok(bound) if world_y <= *bound) {
                            continue;
                        }
                        chunk.update_heightmap(kind, x as i32, world_y, z as i32, default_block)?;
                        *settled = chunk.heightmaps[kind as usize]
                            .as_ref()
                            .expect("ensured heightmap")
                            .first_available(x as i32, z as i32)
                            .map(|first| first.wrapping_sub(2));
                    }
                    y -= run - 1;
                    continue;
                }
                // The aquifer answers `None` for positive density, which the fill
                // writes as the default block, and air above its skip/fluid level.
                // Both are decided here so the per-block call is not made at all,
                // with the same pending-update side effect the call had.
                let solid = value > 0.0;
                if solid {
                    aquifer.note_skipped();
                }
                let state = if solid {
                    default_block
                } else if air_above.is_some_and(|limit| world_y > limit) {
                    aquifer.note_skipped();
                    continue;
                } else {
                    aquifer
                        .compute_substance(context, [world_x, world_y, world_z], f64::from(value))?
                        .unwrap_or(default_block)
                };
                if state == chunk.blocks.air {
                    continue;
                }
                chunk.set_block_raw(x as i32, world_y, z as i32, state)?;
                for (kind, settled) in MAPS.into_iter().zip(&mut settled) {
                    if matches!(settled, Ok(bound) if world_y <= *bound) {
                        continue;
                    }
                    chunk.update_heightmap(kind, x as i32, world_y, z as i32, state)?;
                    *settled = chunk.heightmaps[kind as usize]
                        .as_ref()
                        .expect("ensured heightmap")
                        .first_available(x as i32, z as i32)
                        .map(|first| first.wrapping_sub(2));
                }
                if aquifer.should_schedule_fluid_update()
                    && chunk.blocks.properties.heightmap_state(state).has_fluid()
                {
                    chunk.mark_postprocessing(world_x, world_y, world_z);
                }
            }
        }
    }
    Ok(())
}
