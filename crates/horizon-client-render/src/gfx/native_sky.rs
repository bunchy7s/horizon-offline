//! The sky stage of Horizon's own shader packs (docs/shaders.md): the pack's `sky` function,
//! called for each pixel of the sky disc with vanilla's colour along its view ray. The
//! pipeline's shader is the pack's stage (`CheckedStage::source`) followed by a copy of the
//! renderer's sky (terrain.wgsl's `Globals`, `linear_fog` and sky section) whose names all
//! take a `horizon_sky_` prefix, so none can meet a pack's (the check refuses `horizon_`
//! names). It reads the renderer's globals (group 0) and the pack's frame (group 3), and
//! draws where vanilla's sky draws; the sun, moon, stars and the dark disc are drawn over it.

use horizon_shader_api::{check::CheckedStage, stages::GROUP};

use super::native_post::{NativeFrame, present_constants};
use super::wgsl_prefix::{TERRAIN, prefixed, section};

const PREFIX: &str = "horizon_sky_";

/// The fragment entry that calls the pack: vanilla's colour in, an opaque colour out.
const PACK_ENTRY: &str = "
@fragment fn horizon_sky_fs_pack(in: horizon_sky_SkyOut) -> @location(0) vec4<f32> {
    let d = horizon_sky_view_ray(in.ndc);
    return vec4(sky(SkyInput(d, vec4(horizon_sky_sky_along_ray(d), 1.0))).rgb, 1.0);
}
";

pub struct NativeSky {
    pipeline: wgpu::RenderPipeline,
    pub label: String,
}

/// terrain.wgsl's sky with every name it declares (and `globals`) prefixed.
fn vanilla_sky() -> String {
    let text = [
        section(TERRAIN, "struct Globals {", "@group(0) @binding(1)"),
        section(TERRAIN, "fn linear_fog(", "fn apply_fog("),
        section(TERRAIN, "struct SkyOut {", "// rendertype_lines"),
    ]
    .concat();
    prefixed(&text, PREFIX)
}

/// The sky shader for the pack's sky stage `source`.
fn shader_source(source: &str) -> String {
    format!(
        "{source}\n// Horizon's sky (terrain.wgsl)\n{}{PACK_ENTRY}",
        vanilla_sky()
    )
}

/// A sky pipeline over `module` whose fragment entry is `fragment`.
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    fragment: &str,
    constants: &[(&str, f64)],
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let options = wgpu::PipelineCompilationOptions {
        constants,
        ..Default::default()
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Native shader pack sky"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("horizon_sky_vs_sky"),
            compilation_options: options.clone(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: options,
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The sky's naga module and the layout of its pipelines: `globals` at group 0, the frame
/// at group 3.
fn build(
    device: &wgpu::Device,
    globals: &wgpu::BindGroupLayout,
    frame: &NativeFrame,
    stage: &CheckedStage,
) -> Result<(naga::Module, wgpu::PipelineLayout), String> {
    let text = shader_source(&stage.source);
    let module = naga::front::wgsl::parse_str(&text)
        .map_err(|e| format!("{}: {}", stage.file, e.emit_to_string(&text)))?;
    let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] = Default::default();
    groups[0] = Some(globals);
    groups[GROUP as usize] = Some(&frame.layout);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Native shader pack sky"),
        bind_group_layouts: &groups,
        ..Default::default()
    });
    Ok((module, layout))
}

impl NativeSky {
    /// The pipeline of the pack's sky `stage` with the option values `constants`, drawing
    /// onto the world's target of `format`; `globals` is the terrain's group 0.
    pub fn new(
        device: &wgpu::Device,
        globals: &wgpu::BindGroupLayout,
        frame: &NativeFrame,
        stage: &CheckedStage,
        constants: &[(String, f64)],
        format: wgpu::TextureFormat,
    ) -> Result<Self, String> {
        let (module, layout) = build(device, globals, frame, stage)?;
        let present = present_constants(&module, constants);
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&stage.label),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let pipeline = pipeline(
            device,
            &layout,
            &module,
            "horizon_sky_fs_pack",
            &present,
            format,
        );
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("{}: {e}", stage.file));
        }
        Ok(Self {
            pipeline,
            label: stage.label.clone(),
        })
    }

    /// Draws the sky in a pass whose group 0 is the terrain's.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, frame: &NativeFrame) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(GROUP, &frame.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_shader_api::{Frame as FrameInputs, MANIFEST, Pack, StageKind, check};

    fn stage(sky: &str) -> CheckedStage {
        let mut manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.1\"\n\
            [sky]\nfile = \"sky.wgsl\"\n"
            .to_owned();
        if sky.contains("gain") {
            manifest += "[[options]]\nid = \"gain\"\nname = \"Gain\"\ntype = \"float\"\n\
                default = 1.0\nmin = 0.0\nmax = 2.0\n";
        }
        let manifest = manifest.as_str();
        let files = [(MANIFEST, manifest), ("sky.wgsl", sky)]
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .into();
        let mut report = check(&Pack::from_files(files).unwrap()).unwrap();
        assert_eq!(report.stages[0].kind, StageKind::Sky);
        report.stages.remove(0)
    }

    #[test]
    fn the_renderer_s_sky_takes_a_prefix_and_validates_with_a_pack() {
        let sky = vanilla_sky();
        for name in [
            "Globals",
            "SkyOut",
            "view_ray",
            "PI",
            "sky_along_ray",
            "linear_fog",
            "fs_sky",
        ] {
            assert!(sky.contains(&format!("{PREFIX}{name}")), "{name}");
        }
        // A pack may use the renderer's names for its own things.
        let own = "override gain: f32;\nconst PI: f32 = 3.0;\nfn view_ray() -> f32 { return PI; }\n\
            fn sky(input: SkyInput) -> vec4<f32> { return input.color * gain * view_ray(); }\n";
        let text = shader_source(&stage(own).source);
        let module = naga::front::wgsl::parse_str(&text).unwrap_or_else(|e| {
            panic!("{}", e.emit_to_string(&text));
        });
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap();
    }

    /// Vanilla's sky and two packs over the same globals: one that returns its input draws
    /// vanilla's pixels, one that inverts them draws the inverse (and stays opaque).
    #[test]
    fn a_pack_gets_vanilla_s_colour_and_draws_opaque() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let mut globals: super::super::Globals = bytemuck::Zeroable::zeroed();
        globals.inverse_view = glam::Mat4::IDENTITY.to_cols_array_2d();
        globals.misc = [0.0, 0.0, 1.0, 1.0];
        globals.fog_color = [0.2, 0.4, 0.6, 1.0];
        globals.sky_color = [0.5, 0.7, 1.0, 1.0];
        globals.fog_ends = [64.0, 0.0, 0.0, 0.0];
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size_of::<super::super::Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&globals_buffer, 0, bytemuck::bytes_of(&globals));
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let frame = NativeFrame::new(&device);
        queue.write_buffer(
            &frame.buffer,
            0,
            bytemuck::bytes_of(&FrameInputs::default()),
        );

        let gain = [("gain".to_owned(), 1.0)];
        let draw = |stage: &CheckedStage, vanilla: bool| -> Vec<u8> {
            let sky =
                NativeSky::new(&device, &globals_layout, &frame, stage, &gain, format).unwrap();
            let pipeline = if vanilla {
                let (module, layout) = build(&device, &globals_layout, &frame, stage).unwrap();
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: None,
                    source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
                });
                pipeline(&device, &layout, &module, "horizon_sky_fs_sky", &[], format)
            } else {
                sky.pipeline.clone()
            };
            let texture = |format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 4,
                        height: 4,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let color = texture(
                format,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let depth = texture(
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            let (color_view, depth_view) = (
                color.create_view(&Default::default()),
                depth.create_view(&Default::default()),
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_bind_group(0, &globals_group, &[]);
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(GROUP, &frame.group, &[]);
                pass.draw(0..3, 0..1);
            }
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT * 4),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                color.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                        rows_per_image: Some(4),
                    },
                },
                color.size(),
            );
            queue.submit([encoder.finish()]);
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
            (0..4)
                .flat_map(|row| {
                    let start = row * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
                    bytes[start..start + 16].to_vec()
                })
                .collect()
        };
        let same = stage("fn sky(input: SkyInput) -> vec4<f32> { return input.color; }\n");
        let vanilla = draw(&same, true);
        assert_eq!(draw(&same, false), vanilla);
        // The top row looks up into the sky disc, the bottom one down into the fog.
        assert_ne!(vanilla[..4], vanilla[vanilla.len() - 4..]);
        let inverse = stage(
            "override gain: f32;\nfn sky(input: SkyInput) -> vec4<f32> { return vec4(vec3(gain) - input.color.rgb, 0.25); }\n",
        );
        let inverted = draw(&inverse, false);
        for (got, want) in inverted.chunks(4).zip(vanilla.chunks(4)) {
            assert_eq!(got[3], 255);
            for i in 0..3 {
                assert!(got[i].abs_diff(255 - want[i]) <= 1, "{got:?} {want:?}");
            }
        }
    }
}
