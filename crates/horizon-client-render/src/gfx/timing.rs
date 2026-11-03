//! GPU frame time from timestamp queries, read back a few frames late so the
//! CPU never waits for it. Frames whose readback slot is still busy go untimed.
//!
//! Where the GPU can only time pass boundaries (Apple GPUs on Metal), a
//! one-thread compute pass stamps the frame's start, and a render pass over
//! the frame's target that shades one pixel with every write masked stamps
//! its end (Metal leaves the timestamps of a pass that does nothing at zero).
//! A pass that loads the target runs after every pass that draws into it; a
//! compute pass, which depends on nothing, ran alongside them. Metal also
//! resolves timestamps without waiting for the passes that write them, so
//! they are resolved in a later frame, once the GPU has finished this one.
//! Resolved in the frame itself, the end was mostly the one an earlier frame
//! left in the query, and frames read about 0.01 ms.
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

/// Frames in flight at once; on Metal a slot is busy for about five frames.
const SLOTS: usize = 8;
/// Each slot's timestamps have their own queries and resolve range.
const RESOLVE_STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
/// The frame's target and its format.
pub type Target<'a> = (&'a wgpu::TextureView, wgpu::TextureFormat);
const FREE: u8 = 0;
/// Its frame is submitted; with markers, until the GPU has finished it.
const RUNNING: u8 = 1;
/// Its frame is finished: its timestamps can be resolved.
const DONE: u8 = 2;
/// Resolved and on its way back.
const PENDING: u8 = 3;
const MAPPED: u8 = 4;
/// Readback failed; stop timing rather than reuse an invalid buffer.
const FAILED: u8 = 5;

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

pub struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// The slot this frame writes into, when one was free at `begin`.
    current: Option<usize>,
    next: usize,
    period_ns: f32,
    /// Timestamps between commands, or else these passes' boundaries.
    markers: Option<Markers>,
    /// Slots resolved in this frame's commands, read back once it is submitted.
    resolved: Vec<usize>,
}

/// The marker passes: the start's compute pipeline, and the end's shader and
/// pipeline, for the target format it was made for.
struct Markers {
    module: wgpu::ShaderModule,
    start: wgpu::ComputePipeline,
    end: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

/// The start's one thread; the end's triangle over the whole target (the
/// pass's scissor keeps one pixel), shading nothing it writes.
pub(super) const MARKER: &str =
    "@vertex fn cover(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0, 0.0, 1.0);
}
@fragment fn nothing() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
@compute @workgroup_size(1) fn mark() {}";

impl GpuTimer {
    /// The features timing uses that the adapter has (none: no timing).
    pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
        adapter.features()
            & (wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    }
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Frame timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: 2 * SLOTS as u32,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frame timestamp resolve"),
            size: RESOLVE_STRIDE * SLOTS as u64,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Frame timestamp readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            slots,
            current: None,
            next: 0,
            period_ns: queue.get_timestamp_period(),
            markers: (!device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS))
            .then(|| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Frame timestamp"),
                    source: wgpu::ShaderSource::Wgsl(MARKER.into()),
                });
                Markers {
                    start: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("Frame timestamp"),
                        layout: None,
                        module: &module,
                        entry_point: Some("mark"),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                    module,
                    end: None,
                }
            }),
            resolved: Vec::new(),
        })
    }
    pub fn begin(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self
            .slots
            .iter()
            .any(|slot| slot.state.load(Ordering::Acquire) == FAILED)
        {
            self.current = None;
            return;
        }
        if self.markers.is_some() {
            for slot in 0..SLOTS {
                if self.slots[slot].state.load(Ordering::Acquire) == DONE {
                    self.resolve(encoder, slot);
                }
            }
        }
        let slot = self.next;
        self.current = (self.slots[slot].state.load(Ordering::Acquire) == FREE).then_some(slot);
        let Some(slot) = self.current else { return };
        let index = 2 * slot as u32;
        let Some(markers) = &self.markers else {
            encoder.write_timestamp(&self.queries, index);
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Frame timestamp"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
        });
        pass.set_pipeline(&markers.start);
        pass.dispatch_workgroups(1, 1, 1);
    }
    /// After the frame's last pass into `target` (the frame's own, of `format`).
    pub fn end(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        (target, format): Target,
    ) {
        let Some(slot) = self.current else { return };
        let index = 2 * slot as u32 + 1;
        let Some(markers) = &mut self.markers else {
            encoder.write_timestamp(&self.queries, index);
            self.resolve(encoder, slot);
            return;
        };
        if markers.end.as_ref().is_none_or(|(f, _)| *f != format) {
            markers.end = Some((format, end_pipeline(device, &markers.module, format)));
        }
        let (_, pipeline) = markers.end.as_ref().expect("made above");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Frame timestamp"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &self.queries,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(index),
            }),
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_scissor_rect(0, 0, 1, 1);
        pass.draw(0..3, 0..1);
    }
    /// Copy `slot`'s timestamps to its readback buffer.
    fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder, slot: usize) {
        let (first, at) = (2 * slot as u32, RESOLVE_STRIDE * slot as u64);
        encoder.resolve_query_set(&self.queries, first..first + 2, &self.resolve, at);
        encoder.copy_buffer_to_buffer(&self.resolve, at, &self.slots[slot].buffer, 0, 16);
        self.slots[slot].state.store(RUNNING, Ordering::Release);
        self.resolved.push(slot);
    }
    /// After the frame's submission: start reading back what it resolved,
    /// and with markers, wait for the GPU to finish the frame it timed.
    pub fn submitted(&mut self, queue: &wgpu::Queue) {
        for slot in self.resolved.drain(..) {
            let state = self.slots[slot].state.clone();
            state.store(PENDING, Ordering::Release);
            self.slots[slot]
                .buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    state.store(
                        if result.is_ok() { MAPPED } else { FAILED },
                        Ordering::Release,
                    );
                });
        }
        let Some(slot) = self.current.take() else {
            return;
        };
        if self.markers.is_some() {
            let state = self.slots[slot].state.clone();
            state.store(RUNNING, Ordering::Release);
            queue.on_submitted_work_done(move || state.store(DONE, Ordering::Release));
        }
        self.next = (slot + 1) % SLOTS;
    }
    /// GPU milliseconds of frames whose timestamps have arrived, oldest first.
    pub fn collect(&mut self, device: &wgpu::Device, out: &mut Vec<f32>) {
        if device.poll(wgpu::PollType::Poll).is_err() {
            return;
        }
        for i in 0..SLOTS {
            let slot = &self.slots[(self.next + i) % SLOTS];
            if slot.state.load(Ordering::Acquire) != MAPPED {
                continue;
            }
            {
                let Ok(data) = slot.buffer.slice(..).get_mapped_range() else {
                    // Device loss can destroy a buffer after its mapping succeeded.
                    // Unmapping it would turn the failed readback into a panic.
                    slot.state.store(FAILED, Ordering::Release);
                    continue;
                };
                // Read unaligned: the mapping isn't guaranteed to be 8-byte aligned.
                let stamps: [u64; 2] = bytemuck::pod_read_unaligned(&data[..16]);
                // A start marker that ran late (rare on Metal) leaves the frame untimed.
                if let Some(ticks) = stamps[1].checked_sub(stamps[0]) {
                    out.push((ticks as f64 * f64::from(self.period_ns) / 1e6) as f32);
                }
            }
            slot.buffer.unmap();
            slot.state.store(FREE, Ordering::Release);
        }
    }
}

/// The end marker's pipeline for a `format` target, every write masked.
pub(super) fn end_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Frame timestamp"),
        layout: None,
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("cover"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("nothing"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped_timer() -> Option<(wgpu::Device, wgpu::Queue, GpuTimer)> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        // WGL can hang while creating a test instance on Windows (#5238).
        descriptor.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: GpuTimer::features(&adapter),
            ..Default::default()
        }))
        .unwrap();
        let mut timer = GpuTimer::new(&device, &queue).unwrap();
        queue.write_buffer(
            &timer.slots[0].buffer,
            0,
            bytemuck::cast_slice(&[10_u64, 20]),
        );
        queue.submit([]);
        timer.resolved.push(0);
        timer.submitted(&queue);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(timer.slots[0].state.load(Ordering::Acquire), MAPPED);
        Some((device, queue, timer))
    }

    #[test]
    fn successful_readback_releases_slot() {
        let Some((device, _queue, mut timer)) = mapped_timer() else {
            eprintln!("Skipping GPU timer test: no timestamp-capable adapter");
            return;
        };
        let mut samples = Vec::new();
        timer.collect(&device, &mut samples);
        assert_eq!(
            samples,
            vec![(10.0 * f64::from(timer.period_ns) / 1e6) as f32]
        );
        assert_eq!(timer.slots[0].state.load(Ordering::Acquire), FREE);
    }

    #[test]
    fn destroyed_readback_stops_timing_without_unmapping() {
        let Some((device, queue, mut timer)) = mapped_timer() else {
            eprintln!("Skipping GPU timer test: no timestamp-capable adapter");
            return;
        };
        // Reproduce a buffer destroyed after its successful map callback (#6320).
        timer.slots[0].buffer.destroy();
        let mut samples = Vec::new();
        timer.collect(&device, &mut samples);
        assert!(samples.is_empty());
        assert_eq!(timer.slots[0].state.load(Ordering::Acquire), FAILED);
        // Later frames must not resolve or copy into the failed readback buffer.
        let mut encoder = device.create_command_encoder(&Default::default());
        timer.begin(&mut encoder);
        assert!(timer.current.is_none());
        assert!(timer.resolved.is_empty());
        queue.submit([encoder.finish()]);
        timer.submitted(&queue);
        timer.collect(&device, &mut samples);
        assert!(samples.is_empty());
    }
}
