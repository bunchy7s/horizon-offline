//! Stateful 26.3 underground fluid and pressure selection.
use super::{AquiferField, AquiferSamplers, FluidPicker, FluidStatus};
use crate::registry::{BlockRegistry, BlockStateId};
use horizon_core::{
    density::{SampleError, SamplerContext},
    noise::{DensityError, DensityVolume},
    position::{BlockPos, ChunkPos},
    random::{PositionalRandom, RandomSource},
};
use std::collections::HashMap;

const WAY_BELOW_MIN_Y: i32 = -32512;
const FLOWING_UPDATE_SIMILARITY: f64 = 1.0 - 44.0 / 25.0;
const SURFACE_OFFSETS: [[i32; 2]; 13] = [
    [0, 0],
    [-2, -1],
    [-1, -1],
    [0, -1],
    [1, -1],
    [-3, 0],
    [-2, 0],
    [-1, 0],
    [1, 0],
    [-2, 1],
    [-1, 1],
    [0, 1],
    [1, 1],
];

/// All IDs and block classifications must refer to the same immutable registry.
/// Lava/water tests classify the block, whereas default-lava equality tests the
/// exact state. Air tests include cave air and void air.
pub trait AquiferBlockStates {
    fn air(&self) -> BlockStateId;
    fn lava(&self) -> BlockStateId;
    fn is_air(&self, state: BlockStateId) -> bool;
    fn is_lava(&self, state: BlockStateId) -> bool;
    fn is_water(&self, state: BlockStateId) -> bool;
}
pub struct RegistryAquiferBlockStates<'a> {
    registry: &'a BlockRegistry,
    air: BlockStateId,
    lava: BlockStateId,
}
impl<'a> RegistryAquiferBlockStates<'a> {
    pub fn new(registry: &'a BlockRegistry) -> Result<Self, AquiferError> {
        Ok(Self {
            registry,
            air: registry
                .default_state("minecraft:air")
                .ok_or(AquiferError::MissingBlock)?,
            lava: registry
                .default_state("minecraft:lava")
                .ok_or(AquiferError::MissingBlock)?,
        })
    }
    fn name(&self, state: BlockStateId) -> Option<&str> {
        self.registry.state(state).map(|s| s.name.as_str())
    }
}
impl AquiferBlockStates for RegistryAquiferBlockStates<'_> {
    fn air(&self) -> BlockStateId {
        self.air
    }
    fn lava(&self) -> BlockStateId {
        self.lava
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        matches!(
            self.name(state),
            Some("minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
        )
    }
    fn is_lava(&self, state: BlockStateId) -> bool {
        self.name(state) == Some("minecraft:lava")
    }
    fn is_water(&self, state: BlockStateId) -> bool {
        self.name(state) == Some("minecraft:water")
    }
}
#[derive(Debug)]
pub enum AquiferError {
    Sample(SampleError),
    InvalidGrid,
    OutsideGrid,
    Allocation,
    MissingBlock,
}
impl From<SampleError> for AquiferError {
    fn from(value: SampleError) -> Self {
        Self::Sample(value)
    }
}
impl From<DensityError> for AquiferError {
    fn from(value: DensityError) -> Self {
        Self::Sample(value.into())
    }
}
impl std::fmt::Display for AquiferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sample(e) => e.fmt(f),
            Self::InvalidGrid => f.write_str("aquifer grid is invalid or exceeds 16777216 cells"),
            Self::OutsideGrid => f.write_str("aquifer query lies outside its grid"),
            Self::Allocation => f.write_str("aquifer allocation failed"),
            Self::MissingBlock => f.write_str("aquifer registry lacks default air or lava"),
        }
    }
}
impl std::error::Error for AquiferError {}

pub struct NoiseBasedAquifer<P, F, B> {
    samplers: AquiferSamplers,
    picker: P,
    random: F,
    blocks: B,
    minimum: [i32; 3],
    size_x: i32,
    size_z: i32,
    statuses: Vec<Option<FluidStatus>>,
    locations: Vec<i64>,
    surfaces: HashMap<i64, i32>,
    neighborhood: Option<([i32; 3], [(usize, BlockPos); 12])>,
    skip_above_y: i32,
    schedule: bool,
    cached_col: [i32; 2],
    cached_y_anchor: i32,
    cached_h: [i32; 12],
    cached_py: [i32; 12],
    cached_indices: [usize; 12],
}
impl<P: FluidPicker, F: PositionalRandom, B: AquiferBlockStates> NoiseBasedAquifer<P, F, B> {
    /// The positional factory is the caller's aquifer factory, not the world's
    /// unmodified factory. Every sampler uses the caller's shared context.
    pub fn new(
        samplers: AquiferSamplers,
        picker: P,
        random: F,
        blocks: B,
        volume: DensityVolume,
        context: &mut SamplerContext,
    ) -> Result<Self, AquiferError> {
        let low = volume.minimum();
        let high = volume.maximum();
        let minimum = [
            low[0].wrapping_sub(5) >> 4,
            low[1].wrapping_add(1).div_euclid(12).wrapping_sub(1),
            low[2].wrapping_sub(5) >> 4,
        ];
        let maximum = [
            (high[0].wrapping_sub(5) >> 4).wrapping_add(1),
            high[1].wrapping_add(1).div_euclid(12).wrapping_add(1),
            (high[2].wrapping_sub(5) >> 4).wrapping_add(1),
        ];
        let sizes: [i32; 3] =
            std::array::from_fn(|i| maximum[i].wrapping_sub(minimum[i]).wrapping_add(1));
        let count = sizes
            .into_iter()
            .try_fold(1usize, |n, s| {
                if s > 0 {
                    n.checked_mul(s as usize)
                } else {
                    None
                }
            })
            .filter(|n| *n <= 16_777_216)
            .ok_or(AquiferError::InvalidGrid)?;
        let mut statuses = Vec::new();
        statuses
            .try_reserve_exact(count)
            .map_err(|_| AquiferError::Allocation)?;
        statuses.resize(count, None);
        let mut locations = Vec::new();
        locations
            .try_reserve_exact(count)
            .map_err(|_| AquiferError::Allocation)?;
        locations.resize(count, i64::MAX);
        let mut result = Self {
            samplers,
            picker,
            random,
            blocks,
            minimum,
            size_x: sizes[0],
            size_z: sizes[2],
            statuses,
            locations,
            surfaces: HashMap::new(),
            neighborhood: None,
            skip_above_y: 0,
            schedule: false,
            cached_col: [i32::MIN, i32::MIN],
            cached_y_anchor: i32::MIN,
            cached_h: [0; 12],
            cached_py: [0; 12],
            cached_indices: [0; 12],
        };
        let max_surface = result
            .max_surface_level(
                context,
                [minimum[0].wrapping_shl(4), minimum[2].wrapping_shl(4)],
                [
                    maximum[0].wrapping_shl(4).wrapping_add(9),
                    maximum[2].wrapping_shl(4).wrapping_add(9),
                ],
            )?
            .wrapping_add(8);
        let skip_grid = max_surface.wrapping_add(12).div_euclid(12).wrapping_add(1);
        result.skip_above_y = skip_grid.wrapping_mul(12).wrapping_add(11).wrapping_sub(1);
        Ok(result)
    }
    /// Read-only diagnostic state; enumeration never populates a cache.
    pub fn cached_cells(&self) -> impl Iterator<Item = (i64, Option<FluidStatus>)> + '_ {
        self.locations
            .iter()
            .copied()
            .zip(self.statuses.iter().copied())
    }
    pub fn cached_surfaces(&self) -> impl Iterator<Item = (i64, i32)> + '_ {
        self.surfaces.iter().map(|(key, value)| (*key, *value))
    }
    pub fn should_schedule_fluid_update(&self) -> bool {
        self.schedule
    }
    pub fn skip_sampling_above_y(&self) -> i32 {
        self.skip_above_y
    }
    /// The fill decided a block without this aquifer. Both old decisions
    /// (positive density and air above the skip height) cleared the pending
    /// fluid update, so the caller reproduces that side effect.
    pub fn note_skipped(&mut self) {
        self.schedule = false;
    }
    /// The y above which a non-positive density is the global fluid's air. The
    /// vanilla fluid rules depend only on y; a picker that varies with x/z keeps
    /// the per-block call by answering `None`.
    pub fn air_above_y(&self) -> Option<i32> {
        let center = self.picker.compute([0, i32::MAX, 0]);
        let east = self.picker.compute([i32::MAX, i32::MAX, i32::MAX]);
        let west = self.picker.compute([i32::MIN, i32::MAX, i32::MIN]);
        (center.level == east.level && center.level == west.level)
            .then(|| self.skip_above_y.max(center.level))
    }
    fn sample(
        &self,
        field: AquiferField,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f64, AquiferError> {
        Ok(self.samplers.get(field).sample_value(context, position)? as f64)
    }
    fn surface_level(
        &mut self,
        context: &mut SamplerContext,
        x: i32,
        z: i32,
    ) -> Result<i32, AquiferError> {
        let x = (x >> 2).wrapping_shl(2);
        let z = (z >> 2).wrapping_shl(2);
        let key = ChunkPos::new(x, z).pack();
        if let Some(level) = self.surfaces.get(&key) {
            return Ok(*level);
        }
        let level = self
            .sample(AquiferField::SurfaceLevel, context, [x, 0, z])?
            .floor() as i32;
        self.surfaces.insert(key, level);
        Ok(level)
    }
    fn max_surface_level(
        &mut self,
        context: &mut SamplerContext,
        minimum: [i32; 2],
        maximum: [i32; 2],
    ) -> Result<i32, AquiferError> {
        let low = minimum.map(|v| v >> 2);
        let high = maximum.map(|v| v >> 2);
        let sizes: [i32; 2] = std::array::from_fn(|i| high[i].wrapping_sub(low[i]).wrapping_add(1));
        let volume = DensityVolume::new(
            [sizes[0], 1, sizes[1]],
            [low[0].wrapping_shl(2), 0, low[1].wrapping_shl(2)],
            [4, 1, 4],
        )?;
        if volume.size() > 2_097_152 {
            return Err(AquiferError::InvalidGrid);
        }
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(volume.size())
            .map_err(|_| AquiferError::Allocation)?;
        buffer.resize(volume.size(), 0.);
        self.samplers
            .get(AquiferField::SurfaceLevel)
            .sample_volume(context, &mut buffer, volume)?;
        self.surfaces
            .try_reserve(volume.size())
            .map_err(|_| AquiferError::Allocation)?;
        let mut maximum = i32::MIN;
        let mut index = 0;
        for z in 0..sizes[1] {
            for x in 0..sizes[0] {
                let value = (buffer[index] as f64).floor() as i32;
                index += 1;
                self.surfaces.insert(
                    ChunkPos::new(
                        low[0].wrapping_add(x).wrapping_shl(2),
                        low[1].wrapping_add(z).wrapping_shl(2),
                    )
                    .pack(),
                    value,
                );
                maximum = maximum.max(value);
            }
        }
        Ok(maximum)
    }
    fn grid_index(&self, position: [i32; 3]) -> Result<usize, AquiferError> {
        let relative: [i32; 3] = std::array::from_fn(|i| position[i].wrapping_sub(self.minimum[i]));
        // Vanilla checks the final array index, rather than each coordinate.
        let index = relative[1]
            .wrapping_mul(self.size_z)
            .wrapping_add(relative[2])
            .wrapping_mul(self.size_x)
            .wrapping_add(relative[0]);
        if index < 0 || index as usize >= self.locations.len() {
            return Err(AquiferError::OutsideGrid);
        }
        Ok(index as usize)
    }
    fn status(
        &mut self,
        context: &mut SamplerContext,
        index: usize,
    ) -> Result<FluidStatus, AquiferError> {
        if let Some(status) = self.statuses[index] {
            return Ok(status);
        }
        let pos = BlockPos::unpack(self.locations[index]);
        let status = self.compute_fluid(context, [pos.x, pos.y, pos.z])?;
        self.statuses[index] = Some(status);
        Ok(status)
    }
    fn compute_fluid(
        &mut self,
        context: &mut SamplerContext,
        [x, y, z]: [i32; 3],
    ) -> Result<FluidStatus, AquiferError> {
        let global = self.picker.compute([x, y, z]);
        let upper = y.wrapping_add(12);
        let lower = y.wrapping_sub(12);
        let mut minimum = i32::MAX;
        let mut center_fluid = false;
        for [dx, dz] in SURFACE_OFFSETS {
            let sx = x.wrapping_add(dx.wrapping_mul(16));
            let sz = z.wrapping_add(dz.wrapping_mul(16));
            let surface = self.surface_level(context, sx, sz)?;
            let adjusted = surface.wrapping_add(8);
            let center = dx == 0 && dz == 0;
            if center && lower > adjusted {
                return Ok(global);
            }
            let above = upper > adjusted;
            if above || center {
                let status = self.picker.compute([sx, adjusted, sz]);
                if !self.blocks.is_air(status.at(adjusted, self.blocks.air())) {
                    if center {
                        center_fluid = true;
                    }
                    if above {
                        return Ok(status);
                    }
                }
            }
            minimum = minimum.min(surface);
        }
        let level =
            self.compute_surface_level(context, [x, y, z], global, minimum, center_fluid)?;
        let mut state = global.state;
        if level <= -10
            && level != WAY_BELOW_MIN_Y
            && state != self.blocks.lava()
            && self
                .sample(
                    AquiferField::Lava,
                    context,
                    [x.div_euclid(64), y.div_euclid(40), z.div_euclid(64)],
                )?
                .abs()
                > 0.3
        {
            state = self.blocks.lava();
        }
        Ok(FluidStatus { level, state })
    }
    fn compute_surface_level(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
        global: FluidStatus,
        minimum: i32,
        center_fluid: bool,
    ) -> Result<i32, AquiferError> {
        let [x, y, z] = position;
        let (dry, flooded) = if self.sample(AquiferField::Exclusion, context, position)? > 0. {
            (-1., -1.)
        } else {
            let distance = minimum.wrapping_add(8).wrapping_sub(y);
            let factor = if center_fluid {
                let alpha = (distance as f64 - 0.) / (64. - 0.);
                if alpha < 0. {
                    1.
                } else if alpha > 1. {
                    0.
                } else {
                    1. + alpha * (0. - 1.)
                }
            } else {
                0.
            };
            let value = self
                .sample(AquiferField::Floodedness, context, position)?
                .clamp(-1., 1.);
            let alpha = (factor - 1.) / (0. - 1.);
            let threshold_flooded = -0.3 + alpha * (0.8 - -0.3);
            let threshold_dry = -0.8 + alpha * (0.4 - -0.8);
            (value - threshold_dry, value - threshold_flooded)
        };
        if flooded > 0. {
            return Ok(global.level);
        }
        if dry > 0. {
            let spread = self.samplers.get(AquiferField::Spread).sample_value(
                context,
                [x.div_euclid(16), y.div_euclid(40), z.div_euclid(16)],
            )? * 10.;
            let offset = ((spread as f64 / 3.).floor() as i32).wrapping_mul(3);
            return Ok(minimum.min(
                y.div_euclid(40)
                    .wrapping_mul(40)
                    .wrapping_add(20)
                    .wrapping_add(offset),
            ));
        }
        Ok(WAY_BELOW_MIN_Y)
    }
    fn pressure(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
        memo: &mut f64,
        a: FluidStatus,
        b: FluidStatus,
    ) -> Result<f64, AquiferError> {
        let y = position[1];
        let sa = a.at(y, self.blocks.air());
        let sb = b.at(y, self.blocks.air());
        if (self.blocks.is_lava(sa) && self.blocks.is_water(sb))
            || (self.blocks.is_water(sa) && self.blocks.is_lava(sb))
        {
            return Ok(2.);
        }
        let gap = a.level.wrapping_sub(b.level).wrapping_abs();
        if gap == 0 {
            return Ok(0.);
        }
        let midpoint = 0.5 * a.level.wrapping_add(b.level) as f64;
        let dy = y as f64 + 0.5 - midpoint;
        let edge = gap as f64 / 2. - dy.abs();
        let q = if dy > 0. {
            let n = 0. + edge;
            if n > 0. { n / 1.5 } else { n / 2.5 }
        } else {
            let n = 3. + edge;
            if n > 0. { n / 3. } else { n / 10. }
        };
        let barrier = if q < -2. || q > 2. {
            0.
        } else {
            if memo.is_nan() {
                *memo = self.sample(AquiferField::Barrier, context, position)?;
            }
            *memo
        };
        Ok(2. * (barrier + q))
    }
    pub fn compute_substance(
        &mut self,
        context: &mut SamplerContext,
        [x, y, z]: [i32; 3],
        density: f64,
    ) -> Result<Option<BlockStateId>, AquiferError> {
        if density > 0. {
            self.schedule = false;
            return Ok(None);
        }
        let global = self.picker.compute([x, y, z]);
        if y > self.skip_above_y {
            self.schedule = false;
            return Ok(Some(global.at(y, self.blocks.air())));
        }
        if self.blocks.is_lava(global.at(y, self.blocks.air())) {
            self.schedule = false;
            return Ok(Some(self.blocks.lava()));
        }
        let grid = [
            x.wrapping_sub(5) >> 4,
            y.wrapping_add(1).div_euclid(12),
            z.wrapping_sub(5) >> 4,
        ];
        // Fill in the original cell order, publishing only after checked traversal
        // succeeds. Failed traversal keeps the last complete neighborhood.
        if self.neighborhood.as_ref().map(|(key, _)| *key) != Some(grid) {
            let mut entries = [(0, BlockPos::default()); 12];
            let mut next = 0;
            for dx in 0..=1i32 {
                for dy in -1..=1i32 {
                    for dz in 0..=1i32 {
                        let cell = [
                            grid[0].wrapping_add(dx),
                            grid[1].wrapping_add(dy),
                            grid[2].wrapping_add(dz),
                        ];
                        let index = self.grid_index(cell)?;
                        if self.locations[index] == i64::MAX {
                            let mut random =
                                self.random.at(BlockPos::new(cell[0], cell[1], cell[2]));
                            let px = cell[0]
                                .wrapping_shl(4)
                                .wrapping_add(random.next_int_bound(10).expect("positive bound"));
                            let py = cell[1]
                                .wrapping_mul(12)
                                .wrapping_add(random.next_int_bound(9).expect("positive bound"));
                            let pz = cell[2]
                                .wrapping_shl(4)
                                .wrapping_add(random.next_int_bound(10).expect("positive bound"));
                            self.locations[index] = BlockPos::new(px, py, pz).pack();
                        }
                        entries[next] = (index, BlockPos::unpack(self.locations[index]));
                        next += 1;
                    }
                }
            }
            self.neighborhood = Some((grid, entries));
        }
        if x != self.cached_col[0] || z != self.cached_col[1] || grid[1] != self.cached_y_anchor {
            self.cached_col = [x, z];
            self.cached_y_anchor = grid[1];
            let entries = &self
                .neighborhood
                .as_ref()
                .expect("populated neighborhood")
                .1;
            for i in 0..12 {
                let (index, point) = entries[i];
                let ax = point.x.wrapping_sub(x);
                let az = point.z.wrapping_sub(z);
                self.cached_indices[i] = index;
                self.cached_h[i] = ax.wrapping_mul(ax).wrapping_add(az.wrapping_mul(az));
                self.cached_py[i] = point.y;
            }
        }
        let mut distances = [i32::MAX; 4];
        let mut indices = [0usize; 4];
        for i in 0..12 {
            let ay = self.cached_py[i].wrapping_sub(y);
            let distance = self.cached_h[i].wrapping_add(ay.wrapping_mul(ay));
            let index = self.cached_indices[i];
            // Equal distances insert before existing entries, as in vanilla.
            if distance <= distances[0] {
                distances = [distance, distances[0], distances[1], distances[2]];
                indices = [index, indices[0], indices[1], indices[2]];
            } else if distance <= distances[1] {
                distances[3] = distances[2];
                distances[2] = distances[1];
                distances[1] = distance;
                indices[3] = indices[2];
                indices[2] = indices[1];
                indices[1] = index;
            } else if distance <= distances[2] {
                distances[3] = distances[2];
                distances[2] = distance;
                indices[3] = indices[2];
                indices[2] = index;
            } else if distance <= distances[3] {
                distances[3] = distance;
                indices[3] = index;
            }
        }
        let a = self.status(context, indices[0])?;
        let s01 = similarity(distances[0], distances[1]);
        let candidate = a.at(y, self.blocks.air());
        if s01 <= 0. {
            self.schedule = if s01 >= FLOWING_UPDATE_SIMILARITY {
                a != self.status(context, indices[1])?
            } else {
                false
            };
            return Ok(Some(candidate));
        }
        if self.blocks.is_water(candidate)
            && self.blocks.is_lava(
                self.picker
                    .compute([x, y.wrapping_sub(1), z])
                    .at(y.wrapping_sub(1), self.blocks.air()),
            )
        {
            self.schedule = true;
            return Ok(Some(candidate));
        }
        let mut memo = f64::NAN;
        let b = self.status(context, indices[1])?;
        if density + s01 * self.pressure(context, [x, y, z], &mut memo, a, b)? > 0. {
            self.schedule = false;
            return Ok(None);
        }
        let c = self.status(context, indices[2])?;
        let s02 = similarity(distances[0], distances[2]);
        if s02 > 0.
            && density + s01 * s02 * self.pressure(context, [x, y, z], &mut memo, a, c)? > 0.
        {
            self.schedule = false;
            return Ok(None);
        }
        let s12 = similarity(distances[1], distances[2]);
        if s12 > 0.
            && density + s01 * s12 * self.pressure(context, [x, y, z], &mut memo, b, c)? > 0.
        {
            self.schedule = false;
            return Ok(None);
        }
        self.schedule = a != b
            || (s12 >= FLOWING_UPDATE_SIMILARITY && b != c)
            || (s02 >= FLOWING_UPDATE_SIMILARITY && a != c)
            || (s02 >= FLOWING_UPDATE_SIMILARITY
                && similarity(distances[0], distances[3]) >= FLOWING_UPDATE_SIMILARITY
                && a != self.status(context, indices[3])?);
        Ok(Some(candidate))
    }
}
fn similarity(a: i32, b: i32) -> f64 {
    1. - b.wrapping_sub(a) as f64 / 25.
}
