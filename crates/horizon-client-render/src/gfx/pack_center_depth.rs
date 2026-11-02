//! `centerDepthSmooth`'s input: `depthtex0` at the centre of the screen. A depth texture can
//! only be copied whole, so each frame a one-pixel pass loads that texel into a 1x1 `R32Float`
//! target, copied to one of a few small buffers and read back frames later without waiting for
//! the GPU (as `timing.rs` reads its timestamps); `FrameUniforms` smooths the latest value.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const SLOTS: usize = 3;
const FREE: u8 = 0;
/// Copied this frame: mapped once the frame is submitted.
const COPIED: u8 = 1;
const PENDING: u8 = 2;
const MAPPED: u8 = 3;
/// One row of a texture copy, the alignment wgpu asks of `bytes_per_row`.
const ROW: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

const SHADER: &str = "
@group(0) @binding(0) var depth: texture_depth_2d;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs() -> @location(0) vec4<f32> {
    let centre = vec2<i32>(textureDimensions(depth) / 2u);
    return vec4<f32>(textureLoad(depth, centre, 0), 0.0, 0.0, 1.0);
}
";

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

pub struct CenterDepth {
    slots: Vec<Slot>,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The 1x1 target the centre texel is loaded into.
    texel: wgpu::Texture,
    texel_view: wgpu::TextureView,
    /// The bind group of the depth texture it reads, made again when that texture changes.
    group: Option<(wgpu::Texture, wgpu::BindGroup)>,
    /// The depth last read back: GL window depth, 0..1.
    latest: Option<f32>,
}

impl CenterDepth {
    pub fn new(device: &wgpu::Device) -> Self {
        let slots = (0..SLOTS)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Shader pack centre depth"),
                    size: u64::from(ROW),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shader pack centre depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader pack centre depth"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shader pack centre depth"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let format = wgpu::TextureFormat::R32Float;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shader pack centre depth"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let texel = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Shader pack centre depth"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texel_view = texel.create_view(&Default::default());
        Self {
            slots,
            pipeline,
            layout,
            texel,
            texel_view,
            group: None,
            latest: None,
        }
    }

    /// Loads the centre texel of `depth` (a `Depth32Float` texture that can be bound) and
    /// copies it into a free slot; with none free (the GPU is behind), this frame is skipped.
    pub fn copy(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::Texture,
    ) {
        let Some(slot) = self
            .slots
            .iter()
            .find(|s| s.state.load(Ordering::Acquire) == FREE)
        else {
            return;
        };
        if self.group.as_ref().is_none_or(|(t, _)| t != depth) {
            let view = depth.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Shader pack centre depth"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            });
            self.group = Some((depth.clone(), group));
        }
        let Some((_, group)) = &self.group else {
            return;
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shader pack centre depth"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.texel_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            self.texel.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &slot.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW),
                    rows_per_image: Some(1),
                },
            },
            self.texel.size(),
        );
        slot.state.store(COPIED, Ordering::Release);
    }

    /// After the frame's submission: start reading back what it copied.
    pub fn submitted(&mut self) {
        for slot in &self.slots {
            if slot.state.load(Ordering::Acquire) != COPIED {
                continue;
            }
            slot.state.store(PENDING, Ordering::Release);
            let state = slot.state.clone();
            slot.buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let next = if result.is_ok() { MAPPED } else { FREE };
                    state.store(next, Ordering::Release);
                });
        }
    }

    /// The newest depth read back so far, `None` before the first arrives.
    pub fn collect(&mut self, device: &wgpu::Device) -> Option<f32> {
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &self.slots {
            if slot.state.load(Ordering::Acquire) != MAPPED {
                continue;
            }
            let depth = slot
                .buffer
                .slice(..4)
                .get_mapped_range()
                .ok()
                .map(|bytes| bytemuck::pod_read_unaligned::<f32>(&bytes[..4]));
            slot.buffer.unmap();
            slot.state.store(FREE, Ordering::Release);
            if let Some(depth) = depth.filter(|d| d.is_finite()) {
                self.latest = Some(depth.clamp(0.0, 1.0));
            }
        }
        self.latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    /// A depth texture cleared to a known depth comes back as that depth, a frame or more
    /// later, without the frame waiting for it.
    #[test]
    fn the_centre_depth_comes_back() {
        let Some((device, queue)) = test_device() else {
            return;
        };
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 9,
                height: 7,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = depth.create_view(&Default::default());
        let mut center = CenterDepth::new(&device);
        assert_eq!(center.collect(&device), None);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.375),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        center.copy(&device, &mut encoder, &depth);
        queue.submit([encoder.finish()]);
        center.submitted();
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        assert_eq!(center.collect(&device), Some(0.375));
        // Its slot is free again: every slot can be used once more.
        for _ in 0..SLOTS {
            let mut encoder = device.create_command_encoder(&Default::default());
            center.copy(&device, &mut encoder, &depth);
            queue.submit([encoder.finish()]);
            center.submitted();
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        assert_eq!(center.collect(&device), Some(0.375));
    }
}
