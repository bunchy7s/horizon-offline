//! The entity outline (`LevelRenderer.entityOutlineTarget`): the glowing entities'
//! silhouettes in their team colours, drawn without depth so walls do not hide them
//! (`RenderTypes.outline`), the `entity_outline` post chain over them (`entity_sobel`, then
//! `entity_outline_box_blur` across and up), and `LevelRenderer.blitEntityOutline` onto
//! the frame after the hand.
use super::Vertex;

/// `entityOutlineTarget`'s `GpuFormat.RGBA8_UNORM`.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Targets {
    size: [u32; 2],
    /// `entityOutlineTarget`.
    outline: wgpu::TextureView,
    /// The chain's `swap` target.
    swap: wgpu::TextureView,
    /// What each pass reads: the outline (nearest), swap (bilinear), the outline
    /// (bilinear), swap (nearest).
    groups: [wgpu::BindGroup; 4],
}

pub(super) struct EntityOutline {
    silhouette: wgpu::RenderPipeline,
    /// Sobel, blur across, blur up (into `FORMAT`) and the blended blit (onto the frame).
    passes: [wgpu::RenderPipeline; 4],
    layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    bilinear: wgpu::Sampler,
    vertices: wgpu::Buffer,
    capacity: u64,
    quads: u32,
    targets: Option<Targets>,
}

impl EntityOutline {
    /// `entity` draws the silhouettes with the entity shader's vertex stage and
    /// `fs_outline`; `format` is the frame's.
    pub(super) fn new(
        device: &wgpu::Device,
        entity: (
            &wgpu::ShaderModule,
            &wgpu::PipelineLayout,
            &[Option<wgpu::VertexBufferLayout>],
        ),
        format: wgpu::TextureFormat,
    ) -> Self {
        let (entity_shader, entity_layout, entity_vertex) = entity;
        // RenderPipelines.OUTLINE_NO_CULL: no depth test, the colour replaced.
        let silhouette = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Entity outline silhouettes"),
            layout: Some(entity_layout),
            vertex: wgpu::VertexState {
                module: entity_shader,
                entry_point: Some("vs_entity"),
                compilation_options: Default::default(),
                buffers: entity_vertex,
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: entity_shader,
                entry_point: Some("fs_outline"),
                compilation_options: Default::default(),
                targets: &[Some(FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Entity outline"),
            source: wgpu::ShaderSource::Wgsl(include_str!("entity_outline.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Entity outline bindings"),
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
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Entity outline"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pass = |entry: &str, target: wgpu::ColorTargetState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_quad"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(target)],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // BlendFunction.ENTITY_OUTLINE_BLIT over the colour only (write mask 7).
        let blit = wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            write_mask: wgpu::ColorWrites::COLOR,
        };
        let sampler = |label, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Self {
            silhouette,
            passes: [
                pass("fs_sobel", FORMAT.into()),
                pass("fs_blur_x", FORMAT.into()),
                pass("fs_blur_y", FORMAT.into()),
                pass("fs_blit", blit),
            ],
            layout,
            nearest: sampler("Entity outline nearest", wgpu::FilterMode::Nearest),
            bilinear: sampler("Entity outline bilinear", wgpu::FilterMode::Linear),
            vertices: Self::vertex_buffer(device, 256),
            capacity: 256,
            quads: 0,
            targets: None,
        }
    }

    fn vertex_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Entity outline vertices"),
            size: capacity * std::mem::size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uploads this frame's silhouettes (`LevelRenderer.currentFrameRendersEntityOutline`
    /// when there are any) and sizes the targets like the window.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut super::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        silhouettes: &[Vertex],
        size: [u32; 2],
    ) {
        self.quads = (silhouettes.len() / 4) as u32;
        if self.quads == 0 {
            return;
        }
        let vertices = silhouettes.len() as u64;
        if vertices > self.capacity {
            self.capacity = vertices.next_power_of_two();
            self.vertices = Self::vertex_buffer(device, self.capacity);
        }
        uploads.buffer(
            encoder,
            &self.vertices,
            0,
            bytemuck::cast_slice(silhouettes),
        );
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let texture = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let outline = texture("Entity outline");
        let swap = texture("Entity outline swap");
        let group = |view: &wgpu::TextureView, sampler: &wgpu::Sampler| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Entity outline pass"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        };
        let groups = [
            group(&outline, &self.nearest),
            group(&swap, &self.bilinear),
            group(&outline, &self.bilinear),
            group(&swap, &self.nearest),
        ];
        self.targets = Some(Targets {
            size,
            outline,
            swap,
            groups,
        });
    }

    /// `LevelRenderer.executeOutline`, the post chain and `blitEntityOutline` onto `frame`.
    pub(super) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::TextureView,
        entity_group: &wgpu::BindGroup,
        indices: &wgpu::Buffer,
    ) {
        let Some(t) = self.targets.as_ref().filter(|_| self.quads > 0) else {
            return;
        };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        {
            let mut pass = begin(encoder, "Entity outline", &t.outline, clear);
            pass.set_pipeline(&self.silhouette);
            pass.set_bind_group(0, entity_group, &[]);
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.draw_indexed(0..self.quads * 6, 0, 0..1);
        }
        // post_effect/entity_outline.json; its last pass only copies swap back to the
        // outline target, so the blit reads swap.
        let steps = [
            ("Entity outline sobel", &t.swap, clear),
            ("Entity outline blur across", &t.outline, clear),
            ("Entity outline blur up", &t.swap, clear),
            ("Entity outline blit", frame, wgpu::LoadOp::Load),
        ];
        for (i, (label, view, load)) in steps.into_iter().enumerate() {
            let mut pass = begin(encoder, label, view, load);
            pass.set_pipeline(&self.passes[i]);
            pass.set_bind_group(0, &t.groups[i], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    label: &str,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'e> {
    let attachments = [Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops: wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        },
    })];
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &attachments,
        ..Default::default()
    })
}
