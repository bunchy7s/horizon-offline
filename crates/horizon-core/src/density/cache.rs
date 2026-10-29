use super::*;
use crate::position::BlockPos;

#[derive(Debug)]
pub(super) struct CacheCell {
    volume: Option<DensityVolume>,
    buffer: Vec<f32>,
    value_key: i64,
    value: f32,
    active: bool,
}
impl Default for CacheCell {
    fn default() -> Self {
        Self {
            volume: None,
            buffer: Vec::new(),
            value_key: 0,
            value: f32::NAN,
            active: false,
        }
    }
}
impl SamplerContext {
    /// Cache cells belong to this evaluation context, never to shared samplers.
    pub fn cached() -> Self {
        Self {
            caches: Some(Vec::new()),
            ..Self::default()
        }
    }
    /// Far previews evaluate only the supplied lattice. Ordinary generation
    /// retains vanilla's dense strided-volume evaluation and rounding.
    pub fn sparse() -> Self {
        Self {
            sparse: true,
            ..Self::cached()
        }
    }
    /// Keeps each y-fixed slice's plane (a function of x and z only), so that
    /// sampling the same x/z lattice again, at other heights or another
    /// vertical step, reuses it. For one evaluation over one graph: far
    /// previews sample a tile coarsely, then again around its surface.
    pub fn with_planes(mut self) -> Self {
        self.planes = Some(Default::default());
        self
    }
    pub fn clear_caches(&mut self) {
        if let Some(caches) = &mut self.caches {
            caches.clear();
        }
    }
    fn prepare_cache(&mut self, id: usize) -> Result<bool, SampleError> {
        let Some(caches) = &mut self.caches else {
            return Ok(false);
        };
        if id >= caches.len() {
            caches.resize_with((id + 16) & !15, CacheCell::default);
        }
        if caches[id].active {
            return Err(SampleError::RecursiveCache);
        }
        Ok(true)
    }
}

/// A compiled acyclic cache node. IDs are allocated by the graph compiler and
/// shared only for the same function. The bounded API accepts IDs 0..=65535.
#[derive(Clone, Debug)]
pub struct CachingDensitySampler {
    id: usize,
    input: Arc<dyn DensitySampler>,
}
impl CachingDensitySampler {
    pub fn new(id: usize, input: Arc<dyn DensitySampler>) -> Result<Self, SampleError> {
        if id > 65535 {
            return Err(SampleError::CacheIdOutOfRange);
        }
        Ok(Self { id, input })
    }
    pub fn id(&self) -> usize {
        self.id
    }
}
impl DensitySampler for CachingDensitySampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        if !context.prepare_cache(self.id)? {
            return self.input.sample_value(context, position);
        }
        let key = BlockPos::new(position[0], position[1], position[2]).pack();
        let cell = &mut context.caches.as_mut().unwrap()[self.id];
        if cell.value_key == key && !cell.value.is_nan() {
            return Ok(cell.value);
        }
        if let Some(volume) = cell.volume {
            if let Some(index) = volume.index_of_block(position) {
                return Ok(cell.buffer[index]);
            }
        }
        cell.active = true;
        let result = self.input.sample_value(context, position);
        let cell = &mut context.caches.as_mut().unwrap()[self.id];
        cell.active = false;
        if let Ok(value) = result {
            cell.value_key = key;
            cell.value = value;
        }
        result
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        if !context.prepare_cache(self.id)? {
            return self.input.sample_volume(context, buffer, volume);
        }
        check_exact_buffer(buffer, volume)?;
        let cell = &mut context.caches.as_mut().unwrap()[self.id];
        if cell.volume == Some(volume) {
            buffer.copy_from_slice(&cell.buffer);
            return Ok(());
        }
        let mut cached = std::mem::take(&mut cell.buffer);
        cached.resize(volume.size(), 0.0);
        // Unwritten values after a failed child are deterministic. Differential
        // error tests use a reference arena with the same zero-initialization.
        cached.fill(0.0);
        cell.volume = Some(volume);
        cell.active = true;
        let result = self.input.sample_volume(context, &mut cached, volume);
        let cell = &mut context.caches.as_mut().unwrap()[self.id];
        cell.active = false;
        cell.buffer = cached;
        result?;
        buffer.copy_from_slice(&cell.buffer);
        Ok(())
    }
}
