//! Vanilla's menu background blur (`GameRenderer.processBlurEffect`, the
//! `minecraft:blur` post chain): three rounds of a horizontal then a vertical
//! `post/box_blur` pass over the frame drawn so far, `menuBackgroundBlurriness`
//! framebuffer pixels wide, before the screen is drawn over it. Works in the
//! frame's own (gamma) values, as vanilla's main target does, and needs a
//! swapchain that can be copied from.
//!
//! On a software adapter (WARP, llvmpipe) the rounds run at half the frame's
//! width and height with half the radius: the same blur, a quarter of the
//! pixels. At full size they took 250 ms of a CPU per frame at 1440x900, and
//! every blurred menu ran at 3 FPS (#5185).

/// `blur.json`: the passes' directions, ping-ponging between the copied
/// frame and a swap texture; the last one writes the frame.
const PASSES: [[f32; 2]; 6] = [
    [1.0, 0.0],
    [0.0, 1.0],
    [1.0, 0.0],
    [0.0, 1.0],
    [1.0, 0.0],
    [0.0, 1.0],
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    size_dir: [f32; 4],
    radius: [f32; 4],
}

pub struct MenuBlur {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// One per pass, then the half-size copy's.
    uniforms: Vec<wgpu::Buffer>,
    /// The copied frame (`minecraft:main`) and `swap`; at half size, then the
    /// frame copied at full size.
    targets: Vec<(wgpu::Texture, wgpu::TextureView)>,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    /// Blur at half size (`new`'s `software`).
    half: bool,
}
impl MenuBlur {
    /// `format` is the swapchain's; the blur works in its non-sRGB view. `software`:
    /// the adapter renders on the CPU, so the blur runs at half size.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, software: bool) -> Self {
        let format = format.remove_srgb_suffix();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Menu blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("menu_blur.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Menu blur bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Menu blur"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Menu blur pass"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // The post chain's "bilinear": true inputs, clamped like render targets.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Menu blur"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniforms = (0..=PASSES.len())
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Menu blur pass"),
                    size: std::mem::size_of::<Uniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Self {
            layout,
            pipeline,
            sampler,
            uniforms,
            targets: Vec::new(),
            size: [0, 0],
            format,
            half: software,
        }
    }
    /// The size the blur's rounds run at for a frame of `size`.
    fn work_size(&self, size: [u32; 2]) -> [u32; 2] {
        if self.half {
            size.map(|s| s.div_ceil(2))
        } else {
            size
        }
    }
    fn ensure(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.size == size && !self.targets.is_empty() {
            return;
        }
        self.size = size;
        let work = self.work_size(size);
        let sizes = if self.half {
            vec![work, work, size]
        } else {
            vec![work, work]
        };
        self.targets = sizes
            .into_iter()
            .map(|size| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Menu blur target"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                (texture, view)
            })
            .collect();
    }
    /// Give the targets' video memory back: no frame blurs until the next
    /// screen that does, which makes them again.
    pub fn release(&mut self) {
        self.targets.clear();
    }
    /// Blur `frame` (the swapchain texture) in place through `target`, its
    /// non-sRGB view, by `radius` pixels (`menuBackgroundBlurriness`).
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        target: &wgpu::TextureView,
        size: [u32; 2],
        radius: f32,
        mut timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        self.ensure(device, size);
        let work = self.work_size(size);
        let copy = if self.half { 2 } else { 0 };
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            self.targets[copy].0.as_image_copy(),
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        let radius = if self.half { radius / 2.0 } else { radius };
        if self.half {
            // A radius of 0 samples each half-size pixel's 2x2 block at its shared
            // corner: their mean.
            let data = Uniform {
                size_dir: [size[0] as f32, size[1] as f32, 0.0, 0.0],
                radius: [0.0; 4],
            };
            let (source, output) = (&self.targets[2].1, &self.targets[0].1);
            self.pass(
                device,
                queue,
                encoder,
                PASSES.len(),
                data,
                source,
                output,
                None,
            );
        }
        for (i, dir) in PASSES.iter().enumerate() {
            let data = Uniform {
                size_dir: [work[0] as f32, work[1] as f32, dir[0], dir[1]],
                radius: [radius, 0.0, 0.0, 0.0],
            };
            let source = &self.targets[i % 2].1;
            // The last pass writes the frame; from half size it samples up as it blurs.
            let output = if i + 1 == PASSES.len() {
                target
            } else {
                &self.targets[(i + 1) % 2].1
            };
            self.pass(
                device,
                queue,
                encoder,
                i,
                data,
                source,
                output,
                timing.as_deref_mut(),
            );
        }
    }
    /// One `box_blur` pass from `source` into `output` with uniform buffer `slot`.
    #[allow(clippy::too_many_arguments)]
    fn pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        data: Uniform,
        source: &wgpu::TextureView,
        output: &wgpu::TextureView,
        timing: Option<&mut crate::gfx::pack_timing::PassTiming>,
    ) {
        queue.write_buffer(&self.uniforms[slot], 0, bytemuck::bytes_of(&data));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Menu blur pass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniforms[slot].as_entire_binding(),
                },
            ],
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: output,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Menu blur pass"),
            timestamp_writes: timing.and_then(|t| t.writes("Menu blur")),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of hard stripes, blurred, read back.
    fn blurred(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        blur: &mut MenuBlur,
        size: [u32; 2],
    ) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let stripes: Vec<u8> = (0..size[0] * size[1])
            .flat_map(|i| if i % 4 < 2 { [255; 4] } else { [0, 0, 0, 255] })
            .collect();
        let frame = wgpu::util::DeviceExt::create_texture_with_data(
            device,
            queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &stripes,
        );
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(size[0] * size[1] * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        let view = frame.create_view(&Default::default());
        blur.apply(device, queue, &mut encoder, &frame, &view, size, 2.0, None);
        encoder.copy_texture_to_buffer(
            frame.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &out,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size[0] * 4),
                    rows_per_image: None,
                },
            },
            extent,
        );
        queue.submit([encoder.finish()]);
        out.slice(..).map_async(wgpu::MapMode::Read, |_| ());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        out.slice(..).get_mapped_range().unwrap().to_vec()
    }

    /// Released between two screens, the blur makes its targets again and
    /// blurs the same.
    #[test]
    fn blurs_the_same_after_its_targets_were_released() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            return;
        };
        let mut blur = MenuBlur::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb, false);
        let size = [64, 8];
        let first = blurred(&device, &queue, &mut blur, size);
        // The stripes are blurred into greys.
        assert!(first.chunks(4).any(|p| p[0] != 0 && p[0] != 255));
        blur.release();
        assert_eq!(blurred(&device, &queue, &mut blur, size), first);
    }

    /// On a software adapter the blur runs at half size and still blurs the
    /// frame much as at full size: stripes two pixels wide turn to an even grey.
    #[test]
    fn the_half_size_blur_blurs_like_the_full_size_one() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            return;
        };
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let size = [64, 8];
        let full = blurred(
            &device,
            &queue,
            &mut MenuBlur::new(&device, format, false),
            size,
        );
        let half = blurred(
            &device,
            &queue,
            &mut MenuBlur::new(&device, format, true),
            size,
        );
        // Away from the edges, both are within a few levels of the stripes' mean grey.
        for row in 0..size[1] as usize {
            for x in 8..56 {
                let at = (row * size[0] as usize + x) * 4;
                for (name, frame) in [("full", &full), ("half", &half)] {
                    let v = frame[at];
                    assert!((120..=136).contains(&v), "{name} size: {v} at ({x}, {row})");
                }
            }
        }
    }

    #[test]
    fn box_blur_shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("menu_blur.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
