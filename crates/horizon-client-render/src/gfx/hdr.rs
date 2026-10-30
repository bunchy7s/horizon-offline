//! HDR output (Horizon setting, off by default): the frame renders exactly as
//! without it, into an 8-bit target in vanilla's gamma space, and one pass
//! (`hdr.wgsl`) writes it to an HDR10 or scRGB swapchain at the player's paper
//! white, expanding the world's highlights towards their peak brightness. The
//! world is copied aside before the hand and HUD so only it is expanded.

/// The swapchain HDR output presents to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Output {
    /// HDR10: 10-bit BT.2020 with the PQ curve, the display's own signal.
    Pq,
    /// scRGB: linear 16-bit float, BT.709 primaries, 1.0 = 80 nits.
    ScRgb,
}

impl Output {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pq => "HDR10 (PQ)",
            Self::ScRgb => "scRGB",
        }
    }
    pub fn format(self) -> wgpu::TextureFormat {
        match self {
            Self::Pq => wgpu::TextureFormat::Rgb10a2Unorm,
            Self::ScRgb => wgpu::TextureFormat::Rgba16Float,
        }
    }
    pub fn color_space(self) -> wgpu::SurfaceColorSpace {
        match self {
            Self::Pq => wgpu::SurfaceColorSpace::Bt2100Pq,
            Self::ScRgb => wgpu::SurfaceColorSpace::ExtendedSrgbLinear,
        }
    }
    fn space(self) -> wgpu::SurfaceColorSpaces {
        match self {
            Self::Pq => wgpu::SurfaceColorSpaces::BT2100_PQ,
            Self::ScRgb => wgpu::SurfaceColorSpaces::EXTENDED_SRGB_LINEAR,
        }
    }
}

/// The HDR output `caps` can present: HDR10 where offered, else scRGB.
/// `HORIZON_HDR_OUTPUT=pq` or `scrgb` allows only that one.
pub fn output(caps: &wgpu::SurfaceCapabilities) -> Option<Output> {
    pick(caps, std::env::var("HORIZON_HDR_OUTPUT").ok().as_deref())
}
fn pick(caps: &wgpu::SurfaceCapabilities, only: Option<&str>) -> Option<Output> {
    [(Output::Pq, "pq"), (Output::ScRgb, "scrgb")]
        .into_iter()
        .filter(|(_, name)| only.is_none_or(|o| o == *name))
        .map(|(output, _)| output)
        .find(|o| caps.color_spaces(o.format()).contains(o.space()))
}

struct Targets {
    size: [u32; 2],
    frame: wgpu::Texture,
    world: wgpu::Texture,
    group: wgpu::BindGroup,
}

/// How one tier of light sources (`mesher::hdr_tier`) shines: `share` of the headroom for
/// its brightest pixels, lifted from brightness `start` (none) to `full` (all of it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tier {
    pub share: f32,
    pub start: f32,
    pub full: f32,
}
/// Low, incandescent, bright and very bright.
pub const TIERS: [Tier; 4] = [
    Tier {
        share: 0.2,
        start: 0.08,
        full: 0.09,
    },
    Tier {
        share: 0.5,
        start: 0.14,
        full: 0.38,
    },
    Tier {
        share: 0.6,
        start: 0.14,
        full: 0.38,
    },
    Tier {
        share: 1.0,
        start: 0.14,
        full: 0.38,
    },
];

pub struct Hdr {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    /// Zeroes the frame's alpha once the world is kept (`keep_world`): what draws after,
    /// the hand, HUD and menus, raises it, so HDR output knows where they cover the world.
    clear_alpha: wgpu::RenderPipeline,
    /// Scales the frame's alpha down to 1/255 where the hand drew (`mark_hand`), so HDR
    /// output keeps the hand at the world's brightness.
    hand_mark: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    /// The same pass drawing HDR10 for a screenshot (`capture`), with its own parameters,
    /// so it can differ from the swapchain's in one frame.
    capture_pipeline: wgpu::RenderPipeline,
    capture_params: wgpu::Buffer,
    pub output: Output,
    /// The frame's format: what every other pipeline draws in.
    format: wgpu::TextureFormat,
    targets: Option<Targets>,
    /// `HORIZON_HDR_DEBUG=1`: grey what isn't expanded, tint what is magenta.
    debug: bool,
    /// Paper white and peak brightness, in nits.
    pub paper_white: f32,
    pub peak: f32,
    /// The display's black level, in nits: shadows are lifted onto it (`hdr.wgsl`).
    pub black: f32,
    pub tiers: [Tier; 4],
    /// The HUD and menus' brightness, as a share of paper white (0.25..=1).
    pub ui: f32,
}

impl Hdr {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, output: Output) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon HDR"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../hdr.wgsl").into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Horizon HDR bindings"),
            entries: &[
                texture(0),
                texture(1),
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
            label: Some("Horizon HDR"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline_for = |format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Horizon HDR"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_hdr"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_hdr"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let alpha_pass = |label, blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&device.create_pipeline_layout(&Default::default())),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_hdr"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_clear_alpha"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALPHA,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let clear_alpha = alpha_pass("Horizon HDR UI mask", None);
        // Alpha times the blend constant (1/255): the hand's 1 becomes 1/255, the world's 0
        // stays 0.
        let scale_alpha = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::Constant,
            operation: wgpu::BlendOperation::Add,
        };
        let hand_mark = alpha_pass(
            "Horizon HDR hand mark",
            Some(wgpu::BlendState {
                color: scale_alpha,
                alpha: scale_alpha,
            }),
        );
        let params = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon HDR parameters"),
                size: 80,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            layout,
            pipeline: pipeline_for(output.format()),
            clear_alpha,
            hand_mark,
            params: params(),
            capture_pipeline: pipeline_for(Output::Pq.format()),
            capture_params: params(),
            output,
            format,
            targets: None,
            debug: std::env::var_os("HORIZON_HDR_DEBUG").is_some_and(|v| v == "1"),
            paper_white: 200.0,
            peak: 1000.0,
            black: 0.0,
            tiers: TIERS,
            ui: 1.0,
        }
    }

    /// The 8-bit frame every pass draws into this frame, sized like the window.
    pub fn frame(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let texture = |label, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0].max(1),
                        height: size[1].max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.format,
                    usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            };
            let frame = texture(
                "Horizon HDR frame",
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let world = texture("Horizon HDR world", wgpu::TextureUsages::COPY_DST);
            let view = |t: &wgpu::Texture| t.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Horizon HDR bindings"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view(&frame)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view(&world)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.params.as_entire_binding(),
                    },
                ],
            });
            self.targets = Some(Targets {
                size,
                frame,
                world,
                group,
            });
        }
        self.targets.as_ref().unwrap().frame.clone()
    }

    /// Keeps the world as drawn so far, before the hand and HUD cover it, then zeroes the
    /// frame's alpha: the world's marks live on in the copy, and whatever draws from here
    /// on raises the alpha where it covers the world.
    pub fn keep_world(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(t) = &self.targets else {
            return;
        };
        encoder.copy_texture_to_texture(
            t.frame.as_image_copy(),
            t.world.as_image_copy(),
            t.frame.size(),
        );
        let view = t.frame.create_view(&Default::default());
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon HDR UI mask"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&self.clear_alpha);
        pass.draw(0..3, 0..1);
    }

    /// After the hand: its pixels' alpha goes down to 1/255, below anything the HUD and
    /// menus leave, so HDR output tells the hand (the world's brightness, never lifted)
    /// from the UI (UI Brightness).
    pub fn mark_hand(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(t) = &self.targets else {
            return;
        };
        let view = t.frame.create_view(&Default::default());
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon HDR hand mark"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&self.hand_mark);
        let a = 1.0 / 255.0;
        pass.set_blend_constant(wgpu::Color {
            r: a,
            g: a,
            b: a,
            a,
        });
        pass.draw(0..3, 0..1);
    }

    /// Writes the finished frame to the swapchain image `target`.
    pub fn present(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::Texture,
        world: bool,
    ) {
        let Some(t) = &self.targets else {
            return;
        };
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::cast_slice(&self.params(world, self.output == Output::Pq)),
        );
        Self::draw(encoder, &self.pipeline, &t.group, target);
    }

    /// The frame `present` writes, as HDR10 (BT.2020, PQ, 10 bits a channel) whatever
    /// the swapchain is, in a texture of its own for a screenshot (`Rgb10a2Unorm`).
    pub fn capture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        world: bool,
    ) -> Option<wgpu::Texture> {
        let t = self.targets.as_ref()?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Horizon HDR screenshot"),
            size: t.frame.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Output::Pq.format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Horizon HDR screenshot bindings"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(&t.frame)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(&t.world)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.capture_params.as_entire_binding(),
                },
            ],
        });
        queue.write_buffer(
            &self.capture_params,
            0,
            bytemuck::cast_slice(&self.params(world, true)),
        );
        Self::draw(encoder, &self.capture_pipeline, &group, &texture);
        Some(texture)
    }

    /// hdr.wgsl's `Params`.
    fn params(&self, world: bool, pq: bool) -> [f32; 20] {
        let paper = self.paper_white.max(1.0);
        let mut params = [0.0; 20];
        params[..5].copy_from_slice(&[
            paper / 80.0,
            (self.peak / paper).max(1.0),
            f32::from(u8::from(world)),
            f32::from(u8::from(self.debug)),
            f32::from(u8::from(pq)),
        ]);
        params[5] = self.black.clamp(0.0, paper);
        params[6] = self.ui.clamp(0.25, 1.0);
        for (i, t) in self.tiers.iter().enumerate() {
            params[8 + i] = t.share;
            params[12 + i] = t.start;
            params[16 + i] = t.full;
        }
        params
    }

    fn draw(
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        group: &wgpu::BindGroup,
        target: &wgpu::Texture,
    ) {
        let view = target.create_view(&Default::default());
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Horizon HDR"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::{Output, pick};
    use wgpu::{SurfaceColorSpaces as S, TextureFormat as F};

    #[test]
    fn hdr_shaders_validate() {
        let entity = crate::overlay::wgsl_constants() + include_str!("../entity.wgsl");
        for source in [
            include_str!("../hdr.wgsl"),
            include_str!("../terrain.wgsl"),
            &entity,
        ] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    fn caps(offered: &[(F, S)]) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            format_capabilities: offered
                .iter()
                .map(|&(format, color_spaces)| wgpu::SurfaceFormatCapabilities {
                    format,
                    color_spaces,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn hdr10_first_then_scrgb() {
        let both = caps(&[
            (F::Bgra8UnormSrgb, S::SRGB),
            (F::Rgb10a2Unorm, S::BT2100_PQ | S::BT2100_HLG),
            (F::Rgba16Float, S::EXTENDED_SRGB_LINEAR),
        ]);
        assert_eq!(pick(&both, None), Some(Output::Pq));
        assert_eq!(pick(&both, Some("scrgb")), Some(Output::ScRgb));
        let scrgb = caps(&[(F::Rgba16Float, S::EXTENDED_SRGB_LINEAR)]);
        assert_eq!(pick(&scrgb, None), Some(Output::ScRgb));
        assert_eq!(pick(&scrgb, Some("pq")), None);
        assert_eq!(pick(&caps(&[(F::Bgra8UnormSrgb, S::SRGB)]), None), None);
    }
}
