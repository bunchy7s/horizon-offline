//! Improved transparency: vanilla 26.3's order-independent transparency
//! (`LevelRenderer.executeOit`). The translucent layer is drawn three times
//! against the opaque world's depth, without writing it, into targets the
//! next stage reads (`oit.wgsl` says what each holds), then a full-screen pass
//! composites the sum. With the option off none of this exists and the world
//! draws as before.

use super::oit_features::Features;
use glam::Mat4;

/// The depth bounds' clear: `LevelRenderer.DEPTH_BOUNDS_CLEAR_COLOR`.
const BOUNDS_CLEAR: wgpu::Color = wgpu::Color {
    r: -(f32::MAX as f64),
    g: 0.0,
    b: 0.0,
    a: 0.0,
};
const BOUNDS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const COEFFICIENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The shader's bind group: `terrain.wgsl` holds groups 0 and 1.
const GROUP: u32 = 2;

/// `OitStage`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    DepthBounds,
    Transmittance,
    Accumulate,
}

/// What the device must support: max blending on 32-bit float targets.
pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    adapter.features() & wgpu::Features::FLOAT32_BLENDABLE
}

pub fn supported(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::FLOAT32_BLENDABLE)
}

/// What a stage's pipelines write: `OIT_*_SNIPPET`'s colour targets.
pub(super) fn stage_targets(stage: Stage) -> Vec<Option<wgpu::ColorTargetState>> {
    let blend = |operation| wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation,
    };
    let target = |format, operation| {
        Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState {
                color: blend(operation),
                alpha: blend(operation),
            }),
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let add = wgpu::BlendOperation::Add;
    match stage {
        Stage::DepthBounds => vec![target(BOUNDS_FORMAT, wgpu::BlendOperation::Max)],
        Stage::Transmittance => vec![
            target(COEFFICIENT_FORMAT, add),
            target(COEFFICIENT_FORMAT, add),
        ],
        Stage::Accumulate => vec![target(COEFFICIENT_FORMAT, add)],
    }
}

/// `ProjMat[2][2]` and `ProjMat[3][2]`: a device depth `d` is `y / (d + x)`
/// blocks along the view axis (`oit_linear_depth`).
fn projection_terms(projection: &Mat4) -> [f32; 2] {
    [projection.z_axis.z, projection.w_axis.z]
}

struct Targets {
    size: [u32; 2],
    bounds: wgpu::Texture,
    culled: wgpu::Texture,
    views: Views,
    /// The uniform alone, with the culled bounds, with those and the
    /// coefficients, for the cull with the unculled bounds, and the composite.
    alone: wgpu::BindGroup,
    sampled: wgpu::BindGroup,
    accumulate: wgpu::BindGroup,
    cull: wgpu::BindGroup,
    composite: wgpu::BindGroup,
    /// The depth the terrain's stages test against while a water mask is
    /// drawn (a copy of the world's, plus the masks), and the clouds' own.
    terrain_depth: Option<Depth>,
    cloud_depth: Option<Depth>,
}

struct Depth {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Depth {
    fn new(device: &wgpu::Device, label: &str, size: [u32; 2]) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&Default::default()),
            texture,
        }
    }
}

fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size[0].max(1),
        height: size[1].max(1),
        depth_or_array_layers: 1,
    }
}

/// What the translucent layer's other drawers add to a frame: each draws with
/// a pipeline per stage where it has one.
#[derive(Default)]
pub struct Extras<'a> {
    /// `executeOitWaterMask`: the boats' water patches, depth only.
    pub water_mask: Option<&'a dyn Fn(&mut wgpu::RenderPass<'_>)>,
    /// `CloudRenderer.renderOit`: draws the clouds with the pipeline given.
    pub clouds: Option<&'a dyn Fn(&mut wgpu::RenderPass<'_>, &wgpu::RenderPipeline)>,
    /// "Features, World Border, Weather": draws what the entity, weather and
    /// border shaders put after the opaque world, with `Oit::features`'s
    /// pipelines of the stage.
    pub features: Option<&'a dyn Fn(&mut wgpu::RenderPass<'_>, Stage)>,
}

struct Views {
    bounds: wgpu::TextureView,
    culled: wgpu::TextureView,
    coefficients: [wgpu::TextureView; 2],
    accumulated: wgpu::TextureView,
}

pub struct Oit {
    uniform: wgpu::Buffer,
    /// Layouts of the groups at `GROUP`: the uniform; with the bounds; with the
    /// coefficients; and with the accumulation too.
    layouts: [wgpu::BindGroupLayout; 4],
    /// One per `Stage`.
    terrain: [wgpu::RenderPipeline; 3],
    /// The terrain's flat section draws (`Flat`), where the device has them.
    flat: Option<[wgpu::RenderPipeline; 3]>,
    features: Features,
    clouds: [wgpu::RenderPipeline; 3],
    cull: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    targets: Option<Targets>,
}

impl Oit {
    /// `terrain`: the terrain bind group layout that `vs_terrain` reads,
    /// `flat` the flat draws' second group, `features` the entity, weather
    /// and world border layouts,
    /// `clouds` the cloud pipeline's two, `sprites` the terrain's override
    /// constants, `format` the world target's.
    pub fn new(
        device: &wgpu::Device,
        terrain: &wgpu::BindGroupLayout,
        flat: Option<&wgpu::BindGroupLayout>,
        features: [&wgpu::BindGroupLayout; 3],
        clouds: [&wgpu::BindGroupLayout; 2],
        sprites: [f64; 2],
        format: wgpu::TextureFormat,
    ) -> Self {
        let instance = wgpu::vertex_attr_array![5=>Float32x4];
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<super::TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &super::TERRAIN_ATTRIBUTES,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 16,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance,
            }),
        ];
        let buffers = &buffers[..];
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon improved transparency"),
            source: wgpu::ShaderSource::Wgsl(
                (include_str!("../terrain.wgsl").to_owned()
                    + include_str!("../oit.wgsl")
                    + include_str!("../oit_world.wgsl"))
                .into(),
            ),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Improved transparency"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entry = |binding: u32, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let texture = |binding| {
            entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )
        };
        let params = entry(
            0,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        );
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let layouts = [
            layout("Improved transparency", &[params]),
            layout("Improved transparency bounds", &[params, texture(1)]),
            layout(
                "Improved transparency coefficients",
                &[params, texture(1), texture(2), texture(3)],
            ),
            layout(
                "Improved transparency composite",
                &[params, texture(1), texture(2), texture(3), texture(4)],
            ),
        ];
        let pipeline_layout = |first: Option<&wgpu::BindGroupLayout>, group: usize| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Improved transparency"),
                bind_group_layouts: &[first, None, Some(&layouts[group])],
                ..Default::default()
            })
        };
        let constants = [("SPRITE_ARRAYS", sprites[0]), ("SPRITE_SPLIT", sprites[1])];
        let add = wgpu::BlendOperation::Add;
        let depth = |write| {
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let pipeline = |label,
                        layout: &wgpu::PipelineLayout,
                        vertex: &str,
                        fragment: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        targets: &[Option<wgpu::ColorTargetState>],
                        depth_stencil| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: vertex.starts_with("vs_terrain").then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constants,
                        ..Default::default()
                    },
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let terrain_pipeline = |label, fragment, stage| {
            pipeline(
                label,
                &pipeline_layout(Some(terrain), stage as usize),
                "vs_terrain",
                fragment,
                buffers,
                &stage_targets(stage),
                depth(false),
            )
        };
        let terrain_pipelines = [
            terrain_pipeline("OIT depth bounds", "fs_oit_bounds", Stage::DepthBounds),
            terrain_pipeline(
                "OIT transmittance",
                "fs_oit_transmittance",
                Stage::Transmittance,
            ),
            terrain_pipeline("OIT accumulate", "fs_oit_accumulate", Stage::Accumulate),
        ];
        // The same fragments over `vs_terrain_flat`: one vertex buffer, and
        // the draw table at group 1.
        let flat_pipelines = flat.map(|draw| {
            let flat_buffers = &buffers[..1];
            [
                (
                    "OIT depth bounds (flat)",
                    "fs_oit_bounds",
                    Stage::DepthBounds,
                ),
                (
                    "OIT transmittance (flat)",
                    "fs_oit_transmittance",
                    Stage::Transmittance,
                ),
                (
                    "OIT accumulate (flat)",
                    "fs_oit_accumulate",
                    Stage::Accumulate,
                ),
            ]
            .map(|(label, fragment, stage)| {
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Improved transparency flat terrain"),
                    bind_group_layouts: &[
                        Some(terrain),
                        Some(draw),
                        Some(&layouts[stage as usize]),
                    ],
                    ..Default::default()
                });
                pipeline(
                    label,
                    &layout,
                    "vs_terrain_flat",
                    fragment,
                    flat_buffers,
                    &stage_targets(stage),
                    depth(false),
                )
            })
        });
        let screen = |label, fragment, group, targets: &[_], depth_stencil| {
            pipeline(
                label,
                &pipeline_layout(None, group),
                "vs_sky",
                fragment,
                &[],
                targets,
                depth_stencil,
            )
        };
        let replace = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let cull = screen(
            "OIT cull depth bounds",
            "fs_oit_cull",
            1,
            &[replace(BOUNDS_FORMAT)],
            depth(true),
        );
        // `RenderPipelines.OIT_COMPOSITE`: premultiplied alpha over the world,
        // the nearest see-through depth written.
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: add,
        };
        let composite = screen(
            "OIT composite",
            "fs_oit_composite",
            3,
            &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState {
                    color: premultiplied,
                    alpha: premultiplied,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        // The clouds: entity.wgsl's vertex shader and vertex format, with the
        // stages' fragments.
        let cloud_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon improved transparency clouds"),
            source: wgpu::ShaderSource::Wgsl(
                (super::entity_shader()
                    + include_str!("../oit.wgsl")
                    + include_str!("../oit_clouds.wgsl"))
                .into(),
            ),
        });
        let cloud_attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Uint32,3=>Unorm8x4,4=>Uint8x4,5=>Snorm8x4,6=>Float32x2];
        let cloud_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<super::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &cloud_attributes,
        })];
        let cloud_pipeline = |label, fragment, stage: Stage| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Improved transparency clouds"),
                        bind_group_layouts: &[
                            Some(clouds[0]),
                            Some(clouds[1]),
                            Some(&layouts[stage as usize]),
                        ],
                        ..Default::default()
                    }),
                ),
                vertex: wgpu::VertexState {
                    module: &cloud_shader,
                    entry_point: Some("vs_cloud"),
                    compilation_options: Default::default(),
                    buffers: &cloud_buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                // `OIT_CLOUDS`: only the bounds stage writes the cloud depth,
                // so the later stages keep the nearest face of each cloud.
                depth_stencil: depth(stage == Stage::DepthBounds),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &cloud_shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &stage_targets(stage),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let cloud_pipelines = [
            cloud_pipeline(
                "OIT cloud depth bounds",
                "fs_oit_cloud_bounds",
                Stage::DepthBounds,
            ),
            cloud_pipeline(
                "OIT cloud transmittance",
                "fs_oit_cloud_transmittance",
                Stage::Transmittance,
            ),
            cloud_pipeline(
                "OIT cloud accumulate",
                "fs_oit_cloud_accumulate",
                Stage::Accumulate,
            ),
        ];
        let features = Features::new(device, &layouts, features[0], features[1], features[2]);
        Self {
            uniform,
            layouts,
            terrain: terrain_pipelines,
            flat: flat_pipelines,
            features,
            clouds: cloud_pipelines,
            cull,
            composite,
            targets: None,
        }
    }

    /// The pipeline the terrain's translucent layer draws with in `stage`.
    pub fn terrain(&self, stage: Stage) -> &wgpu::RenderPipeline {
        &self.terrain[stage as usize]
    }

    /// The pipeline the terrain's translucent layer draws with in `stage`
    /// as flat section draws, where they run.
    pub fn flat(&self, stage: Stage) -> Option<&wgpu::RenderPipeline> {
        self.flat.as_ref().map(|f| &f[stage as usize])
    }

    pub fn features(&self) -> &Features {
        &self.features
    }

    /// Sizes the targets to the world's and writes this frame's projection.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        projection: &Mat4,
        wants: [bool; 2],
    ) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::cast_slice(&[projection_terms(projection), [0.0; 2]].concat()),
        );
        let [mask, clouds] = wants;
        if let Some(t) = self.targets.as_mut().filter(|t| t.size == size) {
            if mask && t.terrain_depth.is_none() {
                t.terrain_depth = Some(Depth::new(device, "OIT terrain depth", size));
            }
            if clouds && t.cloud_depth.is_none() {
                t.cloud_depth = Some(Depth::new(device, "OIT cloud depth", size));
            }
            return;
        }
        let make = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let attachment =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let bounds = make(
            "OIT depth bounds",
            BOUNDS_FORMAT,
            attachment | wgpu::TextureUsages::COPY_SRC,
        );
        let culled = make(
            "OIT culled depth bounds",
            BOUNDS_FORMAT,
            attachment | wgpu::TextureUsages::COPY_DST,
        );
        let coefficients =
            [0, 1].map(|_| make("OIT transmittance", COEFFICIENT_FORMAT, attachment));
        let accumulated = make("OIT accumulate", COEFFICIENT_FORMAT, attachment);
        let views = Views {
            bounds: bounds.create_view(&Default::default()),
            culled: culled.create_view(&Default::default()),
            coefficients: coefficients.map(|t| t.create_view(&Default::default())),
            accumulated: accumulated.create_view(&Default::default()),
        };
        let group = |layout: &wgpu::BindGroupLayout, views: &[&wgpu::TextureView]| {
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }];
            entries.extend(views.iter().enumerate().map(|(i, v)| wgpu::BindGroupEntry {
                binding: i as u32 + 1,
                resource: wgpu::BindingResource::TextureView(v),
            }));
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Improved transparency"),
                layout,
                entries: &entries,
            })
        };
        let [c0, c1] = &views.coefficients;
        self.targets = Some(Targets {
            size,
            alone: group(&self.layouts[0], &[]),
            sampled: group(&self.layouts[1], &[&views.culled]),
            accumulate: group(&self.layouts[2], &[&views.culled, c0, c1]),
            cull: group(&self.layouts[1], &[&views.bounds]),
            composite: group(
                &self.layouts[3],
                &[&views.bounds, c0, c1, &views.accumulated],
            ),
            terrain_depth: mask.then(|| Depth::new(device, "OIT terrain depth", size)),
            cloud_depth: clouds.then(|| Depth::new(device, "OIT cloud depth", size)),
            bounds,
            culled,
            views,
        });
    }

    /// `LevelRenderer.executeOit`: `draw` records the terrain's translucent
    /// layer into each stage's pass with that stage's pipeline, `extras` the
    /// water masks and clouds, then the composite goes over `color`. `depth`
    /// is the opaque world's: the stages only read it; the cull and the
    /// composite write it.
    pub fn run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        draw: &dyn Fn(&mut wgpu::RenderPass<'_>, Stage),
        extras: &Extras<'_>,
        mut timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let Some(t) = &self.targets else { return };
        let attachment = |view, load| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
        let load = wgpu::LoadOp::Load;
        let read_only = |view| {
            Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: None,
                stencil_ops: None,
            })
        };
        let water_mask = extras.water_mask.zip(t.terrain_depth.as_ref());
        let cloud_depth = extras.clouds.zip(t.cloud_depth.as_ref());
        if let Some((mask, _)) = water_mask {
            self.execute_water_mask(encoder, t, color, depth, mask, timing.as_deref_mut());
        }
        let terrain_depth = water_mask.map_or(depth, |(_, d)| &d.view);
        let [c0, c1] = &t.views.coefficients;
        let culled = &t.views.culled;
        for (stage, label, group, first, then) in [
            (
                Stage::DepthBounds,
                "OIT depth bounds",
                &t.alone,
                vec![attachment(
                    &t.views.bounds,
                    wgpu::LoadOp::Clear(BOUNDS_CLEAR),
                )],
                // The clouds' bounds join the culled ones.
                vec![attachment(culled, load)],
            ),
            (
                Stage::Transmittance,
                "OIT transmittance",
                &t.sampled,
                vec![attachment(c0, clear), attachment(c1, clear)],
                vec![attachment(c0, load), attachment(c1, load)],
            ),
            (
                Stage::Accumulate,
                "OIT accumulate",
                &t.accumulate,
                vec![attachment(&t.views.accumulated, clear)],
                vec![attachment(&t.views.accumulated, load)],
            ),
        ] {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes(label)),
                    color_attachments: &first,
                    depth_stencil_attachment: read_only(terrain_depth),
                    ..Default::default()
                });
                pass.set_bind_group(GROUP, group, &[]);
                draw(&mut pass, stage);
            }
            if let Some(features) = extras.features {
                // Over the world's own depth, even where a water mask patched
                // the terrain's: the masks only hide water.
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes("OIT features")),
                    color_attachments: &[attachment(
                        match stage {
                            Stage::DepthBounds => &t.views.bounds,
                            Stage::Transmittance => c0,
                            Stage::Accumulate => &t.views.accumulated,
                        },
                        load,
                    )]
                    .into_iter()
                    .chain((stage == Stage::Transmittance).then(|| attachment(c1, load)))
                    .collect::<Vec<_>>(),
                    depth_stencil_attachment: read_only(depth),
                    ..Default::default()
                });
                pass.set_bind_group(GROUP, group, &[]);
                features(&mut pass, stage);
            }
            if stage == Stage::DepthBounds {
                self.execute_depth_bounds_cull(encoder, t, depth, timing.as_deref_mut());
            }
            if let Some((clouds, cloud_depth)) = cloud_depth {
                self.render_clouds(
                    encoder,
                    t,
                    stage,
                    depth,
                    (label, group, &then),
                    (clouds, cloud_depth),
                    timing.as_deref_mut(),
                );
            }
            if stage == Stage::DepthBounds
                && let Some((mask, _)) = water_mask
            {
                self.execute_water_mask(encoder, t, color, depth, mask, timing.as_deref_mut());
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT composite"),
            timestamp_writes: timing.and_then(|t| t.writes("OIT composite")),
            color_attachments: &[attachment(color, wgpu::LoadOp::Load)],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(GROUP, &t.composite, &[]);
        pass.draw(0..3, 0..1);
    }

    /// `CloudRenderer.renderOit` for one stage. The bounds stage first copies
    /// the world's depth (after the cull) and draws into it, so the later
    /// stages, which only read it, keep each pixel's nearest cloud face.
    #[allow(clippy::too_many_arguments)]
    fn render_clouds(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        stage: Stage,
        depth: &wgpu::TextureView,
        (label, group, targets): (
            &str,
            &wgpu::BindGroup,
            &[Option<wgpu::RenderPassColorAttachment<'_>>],
        ),
        (draw, cloud_depth): (
            &dyn Fn(&mut wgpu::RenderPass<'_>, &wgpu::RenderPipeline),
            &Depth,
        ),
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        if stage == Stage::DepthBounds {
            encoder.copy_texture_to_texture(
                depth.texture().as_image_copy(),
                cloud_depth.texture.as_image_copy(),
                extent(t.size),
            );
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            timestamp_writes: timing.and_then(|t| t.writes("OIT clouds")),
            color_attachments: targets,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &cloud_depth.view,
                depth_ops: (stage == Stage::DepthBounds).then_some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(GROUP, group, &[]);
        draw(&mut pass, &self.clouds[stage as usize]);
    }

    /// `LevelRenderer.executeOitWaterMask`: the terrain's depth is the world's
    /// again, with the boats' water patches on top.
    fn execute_water_mask(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        mask: &dyn Fn(&mut wgpu::RenderPass<'_>),
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        let Some(terrain) = &t.terrain_depth else {
            return;
        };
        encoder.copy_texture_to_texture(
            depth.texture().as_image_copy(),
            terrain.texture.as_image_copy(),
            extent(t.size),
        );
        // The mask pipeline writes depth only but has a colour target.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT water mask"),
            timestamp_writes: timing.and_then(|t| t.writes("OIT water mask")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &terrain.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        mask(&mut pass);
    }

    /// `LevelRenderer.executeDepthBoundsCull`: the bounds are copied, then where
    /// the nearest fully opaque see-through fragment is in front of the world,
    /// the far bound stops there and its depth replaces the world's.
    fn execute_depth_bounds_cull(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        encoder.copy_texture_to_texture(
            t.bounds.as_image_copy(),
            t.culled.as_image_copy(),
            wgpu::Extent3d {
                width: t.size[0].max(1),
                height: t.size[1].max(1),
                depth_or_array_layers: 1,
            },
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("OIT cull depth bounds"),
            timestamp_writes: timing.and_then(|t| t.writes("OIT cull depth bounds")),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.views.culled,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.cull);
        pass.set_bind_group(GROUP, &t.cull, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(source: &str, entries: &[&str]) {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        for entry in entries {
            assert!(
                module.entry_points.iter().any(|e| e.name == *entry),
                "{entry}"
            );
        }
    }

    #[test]
    fn the_terrain_shader_validates() {
        validate(
            &(include_str!("../terrain.wgsl").to_owned()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_world.wgsl")),
            &[
                "fs_oit_bounds",
                "fs_oit_transmittance",
                "fs_oit_accumulate",
                "fs_oit_cull",
                "fs_oit_composite",
            ],
        );
    }

    #[test]
    fn the_cloud_shader_validates() {
        validate(
            &(super::super::entity_shader()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_clouds.wgsl")),
            &[
                "fs_oit_cloud_bounds",
                "fs_oit_cloud_transmittance",
                "fs_oit_cloud_accumulate",
            ],
        );
    }

    #[test]
    fn the_feature_shaders_validate() {
        let stages = |name: &str| {
            ["bounds", "transmittance", "accumulate"].map(|s| format!("fs_oit_{name}_{s}"))
        };
        let entity = ["entity", "shadow", "weather", "swirl"]
            .map(stages)
            .concat();
        validate(
            &(super::super::entity_shader()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_features.wgsl")),
            &entity.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let border = stages("border");
        validate(
            &(include_str!("../border.wgsl").to_owned()
                + include_str!("../oit.wgsl")
                + include_str!("../oit_border.wgsl")),
            &border.iter().map(String::as_str).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn a_device_depth_gives_the_distance_along_the_view_axis() {
        // The world's reversed-Z projection: near plane at depth 1.
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
        let [x, y] = projection_terms(&projection);
        for distance in [0.1_f32, 1.0, 7.5, 64.0, 500.0] {
            let clip = projection * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
            let device = clip.z / clip.w;
            let linear = y / (device + x);
            assert!(
                (linear - distance).abs() < distance * 2e-3,
                "{distance}: {linear}"
            );
        }
    }
}
