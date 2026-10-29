//! Compiled density samplers for Java Edition 26.3.
//!
//! Evaluation primitives and an initial graph compiler; complete terrain
//! generation is pending. Bulk evaluation retains its own floating-point path.
use crate::{
    geometry::Axis,
    noise::{DensityError, DensityVolume, Noise},
};
use std::sync::Arc;

mod beardifier;
pub use beardifier::{
    Beardifier, JigsawJunction, KernelJunction, KernelPiece, RigidPiece, StructureBounds,
    TerrainAdjustment, TerrainKernel,
};
mod cache;
mod world_fields;
pub use world_fields::{
    BlendDensitySampler, ContextBoundSampler, DensityBlender, EmptyBlender, WorldField, WorldFields,
};
mod end_islands;
pub use end_islands::EndIslandSampler;
pub mod graph;
pub use cache::CachingDensitySampler;
mod arithmetic;
mod interpolation;
mod interval;
pub use interpolation::InterpolatedSampler;
pub use interval::{Interval, IntervalError};
mod rounding;
mod selection;
pub use rounding::{RoundMode, RoundSampler};
mod power;
pub use power::{PowerSampler, compile_constant_exponent};
mod slice;
pub use slice::{SliceAxes, SliceSampler};
mod spline;
pub use spline::{CubicSpline, MultipointSpline, SplineError, SplinePoint, SplineSampler};
mod surface;
pub use surface::{FindTopSurfaceSampler, compile_surface_search};
mod terrain_noise;
pub use arithmetic::{BinaryOp, BinarySampler, UnaryOp, UnarySampler};
pub use selection::{
    DensityValue, IntervalSelectSampler, InvalidIntervals, LerpSampler, RangeChoiceSampler,
};
pub use terrain_noise::{
    BlendedFbmSet, BlendedNoise, DistanceMetric, DistanceToPointSampler, ShiftNoiseKind,
    compile_shift_noise, create_blended_fbm,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleError {
    Volume(DensityError),
    ZeroGradientPeriod,
    ExactBufferSizeRequired,
    CacheIdOutOfRange,
    RecursiveCache,
    SurfaceRequiresSingleLayer,
    SurfaceSearchOverflow,
}
impl From<DensityError> for SampleError {
    fn from(value: DensityError) -> Self {
        Self::Volume(value)
    }
}
impl std::fmt::Display for SampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Volume(error) => error.fmt(f),
            Self::SurfaceRequiresSingleLayer => {
                f.write_str("surface search requires a single Y layer")
            }
            Self::SurfaceSearchOverflow => {
                f.write_str("surface search crossed the integer height domain")
            }
            Self::CacheIdOutOfRange => f.write_str("density cache id exceeds 65535"),
            Self::RecursiveCache => f.write_str("density cache reentered its own evaluation"),
            Self::ExactBufferSizeRequired => {
                f.write_str("this density sampler requires exactly a volume-sized output")
            }
            Self::ZeroGradientPeriod => {
                f.write_str("repeating gradient has a zero coordinate period")
            }
        }
    }
}
impl std::error::Error for SampleError {}

/// Worker-owned temporary storage. Samplers can be shared across workers; their
/// scratch buffers cannot. World fields are immutable for an evaluation context.
#[derive(Debug, Default)]
pub struct SamplerContext {
    scratch: Vec<Vec<f32>>,
    caches: Option<Vec<cache::CacheCell>>,
    fields: WorldFields,
    sparse: bool,
    /// Planes of y-fixed slices already sampled (`with_planes`).
    planes: Option<std::collections::HashMap<slice::PlaneKey, Vec<f32>>>,
}
impl SamplerContext {
    fn with_buffer<T>(
        &mut self,
        size: usize,
        sample: impl FnOnce(&mut Self, &mut [f32]) -> Result<T, SampleError>,
    ) -> Result<T, SampleError> {
        let mut buffer = self.scratch.pop().unwrap_or_default();
        buffer.resize(size, 0.0);
        let result = sample(self, &mut buffer);
        self.scratch.push(buffer);
        result
    }
    /// Start an independent evaluation, retaining only temporary scratch buffers.
    /// Cached scalar/volume results and world bindings must not cross chunks or
    /// graphs, whose cache IDs can refer to different functions.
    pub fn reset(&mut self) {
        self.caches = self.caches.as_ref().map(|_| Vec::new());
        self.fields = WorldFields::default();
        if let Some(planes) = &mut self.planes {
            planes.clear();
        }
    }
    /// Release retained storage between worlds or after unusually large work.
    pub fn clear_scratch(&mut self) {
        self.scratch.clear();
    }
}

pub trait DensitySampler: std::fmt::Debug + Send + Sync {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError>;

    /// The slice length is the logical buffer size. Some samplers fill its tail;
    /// others leave it untouched. A short buffer is rejected before any writes.
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        visit(volume, |index, position| {
            buffer[index] = self.sample_value(context, position)?;
            Ok(())
        })
    }
}
fn check_buffer(buffer: &[f32], volume: DensityVolume) -> Result<(), SampleError> {
    if buffer.len() < volume.size() {
        Err(DensityError::BufferTooShort.into())
    } else {
        Ok(())
    }
}
fn check_exact_buffer(buffer: &[f32], volume: DensityVolume) -> Result<(), SampleError> {
    check_buffer(buffer, volume)?;
    if buffer.len() != volume.size() {
        Err(SampleError::ExactBufferSizeRequired)
    } else {
        Ok(())
    }
}
fn visit(
    volume: DensityVolume,
    mut cell: impl FnMut(usize, [i32; 3]) -> Result<(), SampleError>,
) -> Result<(), SampleError> {
    let [sx, sy, sz] = volume.sizes();
    let mut index = 0;
    for z in 0..sz {
        let bz = volume.block(Axis::Z, z).unwrap();
        for x in 0..sx {
            let bx = volume.block(Axis::X, x).unwrap();
            for y in 0..sy {
                cell(index, [bx, volume.block(Axis::Y, y).unwrap(), bz])?;
                index += 1;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct ConstantSampler(pub f32);
impl DensitySampler for ConstantSampler {
    fn sample_value(&self, _: &mut SamplerContext, _: [i32; 3]) -> Result<f32, SampleError> {
        Ok(self.0)
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        buffer.fill(self.0);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TilingMode {
    ClampToEdge,
    Repeat,
    MirroredRepeat,
}

#[derive(Clone, Copy, Debug)]
pub struct GradientSampler {
    axis: Axis,
    tiling: TilingMode,
    from: i32,
    to: i32,
    period: i32,
    value: f32,
    factor: f32,
}
impl GradientSampler {
    pub fn new(
        axis: Axis,
        tiling: TilingMode,
        from: i32,
        to: i32,
        from_value: f32,
        to_value: f32,
    ) -> Self {
        let period = to.wrapping_sub(from);
        Self {
            axis,
            tiling,
            from,
            to,
            period,
            value: from_value,
            factor: (to_value - from_value) / period as f32,
        }
    }
    pub fn compute(&self, coordinate: i32) -> Result<f32, SampleError> {
        let distance = match self.tiling {
            TilingMode::ClampToEdge => coordinate
                .clamp(self.from.min(self.to), self.from.max(self.to))
                .wrapping_sub(self.from),
            TilingMode::Repeat | TilingMode::MirroredRepeat => {
                if self.period == 0 {
                    return Err(SampleError::ZeroGradientPeriod);
                }
                let delta = coordinate.wrapping_sub(self.from);
                // Java floorDiv/floorMod wrap MIN / -1, and round toward -infinity.
                let mut cycle = delta.wrapping_div(self.period);
                let mut remainder = delta.wrapping_rem(self.period);
                if remainder != 0 && (delta ^ self.period) < 0 {
                    cycle = cycle.wrapping_sub(1);
                    remainder = remainder.wrapping_add(self.period);
                }
                if self.tiling == TilingMode::MirroredRepeat && cycle & 1 != 0 {
                    self.period.wrapping_sub(remainder)
                } else {
                    remainder
                }
            }
        };
        Ok(self.value + distance as f32 * self.factor)
    }
}
impl DensitySampler for GradientSampler {
    fn sample_value(&self, _: &mut SamplerContext, position: [i32; 3]) -> Result<f32, SampleError> {
        self.compute(position[self.axis as usize])
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        let [sx, sy, sz] = volume.sizes();
        for coordinate in 0..volume.sizes()[self.axis as usize] {
            let value = self.compute(volume.block(self.axis, coordinate).unwrap())?;
            match self.axis {
                Axis::X => {
                    for z in 0..sz {
                        let start = (coordinate + z * sx) * sy;
                        buffer[start..start + sy].fill(value);
                    }
                }
                Axis::Y => {
                    for z in 0..sz {
                        for x in 0..sx {
                            buffer[(x + z * sx) * sy + coordinate] = value;
                        }
                    }
                }
                Axis::Z => {
                    let start = coordinate * sx * sy;
                    buffer[start..start + sx * sy].fill(value);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct NoiseSampler {
    pub noise: Arc<dyn Noise>,
    pub xz_scale: f64,
    pub y_scale: f64,
}
impl DensitySampler for NoiseSampler {
    fn sample_value(
        &self,
        _: &mut SamplerContext,
        [x, y, z]: [i32; 3],
    ) -> Result<f32, SampleError> {
        Ok(self.noise.get3(
            x as f64 * self.xz_scale,
            y as f64 * self.y_scale,
            z as f64 * self.xz_scale,
        ))
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        buffer.fill(0.0);
        Ok(self
            .noise
            .add_to_volume(buffer, volume, self.xz_scale, self.y_scale, 1.0)?)
    }
}

/// Compiled XZ or XYZ shifted noise. `None` selects vanilla's XZ specialization;
/// it must not add a synthetic zero Y shift (that would change signed zeros).
#[derive(Clone, Debug)]
pub struct ShiftedNoiseSampler {
    pub source: NoiseSampler,
    pub shift_x: Arc<dyn DensitySampler>,
    pub shift_y: Option<Arc<dyn DensitySampler>>,
    pub shift_z: Arc<dyn DensitySampler>,
}
impl DensitySampler for ShiftedNoiseSampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        let x = position[0] as f64 * self.source.xz_scale
            + self.shift_x.sample_value(context, position)? as f64;
        let mut y = position[1] as f64 * self.source.y_scale;
        if let Some(shift) = &self.shift_y {
            y += shift.sample_value(context, position)? as f64;
        }
        let z = position[2] as f64 * self.source.xz_scale
            + self.shift_z.sample_value(context, position)? as f64;
        Ok(self.source.noise.get3(x, y, z))
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        self.shift_x.sample_volume(context, buffer, volume)?;
        if let Some(shift_y) = &self.shift_y {
            context.with_buffer(volume.size(), |context, y| {
                shift_y.sample_volume(context, y, volume)?;
                context.with_buffer(volume.size(), |context, z| {
                    self.shift_z.sample_volume(context, z, volume)?;
                    self.sample_shifted_volume(buffer, volume, Some(y), z)
                })
            })
        } else {
            context.with_buffer(volume.size(), |context, z| {
                self.shift_z.sample_volume(context, z, volume)?;
                self.sample_shifted_volume(buffer, volume, None, z)
            })
        }
    }
}
impl ShiftedNoiseSampler {
    fn sample_shifted_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        shift_y: Option<&[f32]>,
        shift_z: &[f32],
    ) -> Result<(), SampleError> {
        visit(volume, |index, [x, y, z]| {
            let x = x as f64 * self.source.xz_scale + buffer[index] as f64;
            let mut y = y as f64 * self.source.y_scale;
            if let Some(shift) = shift_y {
                y += shift[index] as f64;
            }
            let z = z as f64 * self.source.xz_scale + shift_z[index] as f64;
            buffer[index] = self.source.noise.get3(x, y, z);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests;
