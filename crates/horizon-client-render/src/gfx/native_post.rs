//! The post stage of Horizon's own shader packs (docs/shaders.md): each `[[post]]` pass is a
//! full-screen pipeline built from its checked naga module (`horizon_shader_api::check`), with
//! the pack's options as pipeline constants. A pass reads bind group 3: the frame, the world's
//! colour and depth, the previous pass's output and two samplers. Passes before the last write
//! half-float targets of the world's size, so values over 1 carry to the next; the last draws
//! onto the screen. Every stage of a pack reads the same `frame`, written once a frame
//! (`NativeFrame`).

use glam::{Mat4, Vec3};
use horizon_shader_api::{Frame as FrameInputs, check::CheckedStage, stages::GROUP};

use super::Frame;

const INTERMEDIATE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The world's colour and depth views a set of bind groups was made for.
type WorldViews = (wgpu::TextureView, wgpu::TextureView);

pub struct NativePost {
    passes: Vec<(String, wgpu::RenderPipeline)>,
    layout: wgpu::BindGroupLayout,
    frame: wgpu::Buffer,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    /// Two targets of the world's size, written in turn (none with a single pass).
    targets: Option<([u32; 2], [wgpu::TextureView; 2])>,
    /// One bind group per pass, for these world views.
    groups: Option<(WorldViews, Vec<wgpu::BindGroup>)>,
    /// The screen's colour format, the world target's too.
    format: wgpu::TextureFormat,
}

/// The `frame` every stage of a pack reads (group 3, binding 0), written once a frame.
pub struct NativeFrame {
    pub(super) buffer: wgpu::Buffer,
    /// Group 3 of the stages that read only the frame (the sky).
    pub layout: wgpu::BindGroupLayout,
    pub group: wgpu::BindGroup,
    start: std::time::Instant,
    frames: u32,
}

/// The uniform `frame` at binding 0, read by `visibility`.
fn frame_entry(visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size_of::<FrameInputs>() as u64),
        },
        count: None,
    }
}

impl NativeFrame {
    pub fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Native shader pack frame"),
            size: size_of::<FrameInputs>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack frame"),
            entries: &[frame_entry(wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Native shader pack frame"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        Self {
            buffer,
            layout,
            group,
            start: std::time::Instant::now(),
            frames: 0,
        }
    }

    /// Writes this frame's inputs, for a world of `size`.
    pub fn write(&mut self, queue: &wgpu::Queue, frame: &Frame, size: [u32; 2]) {
        let seconds = self.start.elapsed().as_secs_f32() % 3600.0;
        let inputs = frame_inputs(frame, size, seconds, self.frames);
        self.frames = self.frames.wrapping_add(1);
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&inputs));
    }
}

/// The option values of `constants` that `module` declares as overrides (wgpu refuses others).
pub fn present_constants<'a>(
    module: &naga::Module,
    constants: &'a [(String, f64)],
) -> Vec<(&'a str, f64)> {
    constants
        .iter()
        .filter(|(id, _)| {
            module
                .overrides
                .iter()
                .any(|(_, o)| o.name.as_ref() == Some(id))
        })
        .map(|(id, v)| (id.as_str(), *v))
        .collect()
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl NativePost {
    /// Pipelines for `stages` (the pack's post passes, in order) with the option values
    /// `constants`, drawing onto a screen of `format`. Errors name the pass wgpu refused.
    pub fn new(
        device: &wgpu::Device,
        stages: Vec<CheckedStage>,
        constants: &[(String, f64)],
        format: wgpu::TextureFormat,
        frame: &NativeFrame,
    ) -> Result<Self, String> {
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack post"),
            entries: &[
                frame_entry(wgpu::ShaderStages::FRAGMENT),
                texture_entry(1, float),
                texture_entry(2, wgpu::TextureSampleType::Depth),
                texture_entry(3, float),
                sampler_entry(4),
                sampler_entry(5),
            ],
        });
        let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] = Default::default();
        groups[GROUP as usize] = Some(&layout);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Native shader pack post"),
            bind_group_layouts: &groups,
            ..Default::default()
        });
        let count = stages.len();
        let mut passes = Vec::with_capacity(count);
        for (i, stage) in stages.into_iter().enumerate() {
            let present = present_constants(&stage.module, constants);
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&stage.label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(stage.module)),
            });
            let options = wgpu::PipelineCompilationOptions {
                constants: &present,
                ..Default::default()
            };
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&stage.label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("horizon_vs_post"),
                    compilation_options: options.clone(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("horizon_fs_post"),
                    compilation_options: options,
                    targets: &[Some(wgpu::ColorTargetState {
                        format: if i + 1 == count { format } else { INTERMEDIATE },
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            if let Some(e) = pollster::block_on(scope.pop()) {
                return Err(format!("{}: {e}", stage.file));
            }
            passes.push((stage.label, pipeline));
        }
        let sampler = |label, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        Ok(Self {
            passes,
            layout,
            frame: frame.buffer.clone(),
            linear: sampler("Native shader pack linear", wgpu::FilterMode::Linear),
            nearest: sampler("Native shader pack nearest", wgpu::FilterMode::Nearest),
            targets: None,
            groups: None,
            format,
        })
    }

    /// The passes' labels, in order.
    pub fn labels(&self) -> Vec<&str> {
        self.passes.iter().map(|(l, _)| l.as_str()).collect()
    }

    fn prepare(
        &mut self,
        device: &wgpu::Device,
        world: (&wgpu::TextureView, &wgpu::TextureView),
        size: [u32; 2],
    ) {
        if self.passes.len() > 1 && self.targets.as_ref().is_none_or(|t| t.0 != size) {
            let target = |label| {
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
                        format: INTERMEDIATE,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            self.targets = Some((
                size,
                [
                    target("Native shader pack A"),
                    target("Native shader pack B"),
                ],
            ));
            self.groups = None;
        }
        if self
            .groups
            .as_ref()
            .is_some_and(|((color, depth), _)| color == world.0 && depth == world.1)
        {
            return;
        }
        let groups = (0..self.passes.len())
            .map(|i| {
                // Pass i reads what pass i - 1 wrote: target (i - 1) % 2, the world for the first.
                let previous = match (&self.targets, i) {
                    (Some((_, t)), 1..) => &t[(i - 1) % 2],
                    _ => world.0,
                };
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Native shader pack post"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.frame.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(world.0),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(world.1),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(previous),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Sampler(&self.linear),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::Sampler(&self.nearest),
                        },
                    ],
                })
            })
            .collect();
        self.groups = Some(((world.0.clone(), world.1.clone()), groups));
    }

    /// Runs the passes over the world target `world` (colour, depth) of `size`, the last onto
    /// `screen`, timed by `timing` when it times this frame.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        world: (&wgpu::TextureView, &wgpu::TextureView),
        screen: &wgpu::TextureView,
        size: [u32; 2],
        mut timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        self.prepare(device, world, size);
        if let Some(t) = timing.as_deref_mut() {
            t.mark(device, encoder, (world.0, self.format));
        }
        let Some((_, groups)) = &self.groups else {
            return;
        };
        for (i, ((label, pipeline), group)) in self.passes.iter().zip(groups).enumerate() {
            let view = match &self.targets {
                Some((_, t)) if i + 1 < self.passes.len() => &t[i % 2],
                _ => screen,
            };
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &attachments,
                timestamp_writes: timing.as_deref_mut().and_then(|t| t.writes(label)),
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(GROUP, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// The frame as packs read it (docs/api/shaders-reference.md), from the renderer's `Frame`.
pub fn frame_inputs(frame: &Frame, size: [u32; 2], seconds: f32, index: u32) -> FrameInputs {
    let aspect = size[0] as f32 / size[1].max(1) as f32;
    let view = crate::camera::view_matrix(frame.yaw, frame.pitch);
    let projection = super::world_projection(frame.fov_degrees, aspect, frame.depth_far)
        * frame.bob
        * frame.spin;
    // A celestial body's direction at `angle` degrees (SkyRenderer: Y -90°, then X by the
    // angle, of straight up): overhead at 0, rising in the east (+x).
    let body = |angle: f32| {
        let a = angle.to_radians();
        Vec3::new(-a.sin(), a.cos(), 0.0)
    };
    let (fog_color, fog_start, fog_end) = match &frame.fog {
        Some(f) => (
            [f.color[0], f.color[1], f.color[2]],
            f.environmental_start.min(f.render_distance_start),
            f.environmental_end.min(f.render_distance_end),
        ),
        None => (frame.sky.fog_color, frame.distance * 0.9, frame.distance),
    };
    let pack = &frame.pack;
    let level = |l: i32| l as f32 / 15.0;
    let m = |m: Mat4| m.to_cols_array_2d();
    FrameInputs {
        view: m(view),
        projection: m(projection),
        view_inverse: m(view.inverse()),
        projection_inverse: m(projection.inverse()),
        camera_position: frame.camera.as_vec3().to_array(),
        time: seconds,
        sun_direction: body(frame.sky.sun_angle).to_array(),
        day_time: pack.day_time.rem_euclid(24000) as f32 / 24000.0,
        moon_direction: body(frame.sky.moon_angle).to_array(),
        game_time: frame.shader_game_time,
        sky_color: frame.sky.sky_color,
        rain: pack.rain,
        fog_color,
        thunder: pack.thunder,
        resolution: [size[0] as f32, size[1] as f32],
        eye_light: [level(pack.eye_light[0]), level(pack.eye_light[1])],
        fog_start,
        fog_end,
        render_distance: frame.distance,
        far: frame.depth_far,
        night_vision: frame.night_vision,
        blindness: pack.blindness,
        darkness: frame.darkness.1.clamp(0.0, 1.0),
        brightness: pack.brightness,
        eye_in: pack.eye_in as u32,
        dimension: if frame.end_sky {
            1
        } else if frame.nether_lighting {
            -1
        } else {
            0
        },
        frame_index: index,
        hide_gui: u32::from(pack.hide_gui),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_shader_api::{MANIFEST, Pack, check};

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    fn texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
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
    }

    /// Two passes: the first doubles the world's colour (over 1, so the half-float target
    /// must keep it) times an option, the second halves it and writes the depth as alpha.
    #[test]
    fn passes_chain_through_half_float_targets_with_options_and_depth() {
        let Some((device, queue)) = device() else {
            return;
        };
        let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.1\"\n\
            [[post]]\nfile = \"a.wgsl\"\n[[post]]\nfile = \"b.wgsl\"\n\
            [[options]]\nid = \"gain\"\nname = \"Gain\"\ntype = \"float\"\ndefault = 0.0\nmin = 0.0\nmax = 2.0\n";
        let a = "override gain: f32;\nfn post(input: PostInput) -> vec4<f32> { return vec4<f32>(input.color.rgb * 2.0 * gain, 1.0); }\n";
        let b = "fn post(input: PostInput) -> vec4<f32> { return vec4<f32>(input.color.rgb * 0.5, input.depth); }\n";
        let files = [(MANIFEST, manifest), ("a.wgsl", a), ("b.wgsl", b)]
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let frame = NativeFrame::new(&device);
        queue.write_buffer(
            &frame.buffer,
            0,
            bytemuck::bytes_of(&FrameInputs::default()),
        );
        let mut post = NativePost::new(
            &device,
            report.stages,
            &[("gain".into(), 1.0)],
            format,
            &frame,
        )
        .unwrap();
        assert_eq!(post.labels(), ["a.wgsl", "b.wgsl"]);

        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let color = texture(&device, format, usage).create_view(&Default::default());
        let depth = texture(&device, wgpu::TextureFormat::Depth32Float, usage)
            .create_view(&Default::default());
        let screen = texture(
            &device,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.8,
                        g: 0.4,
                        b: 0.2,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.5),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        let view = screen.create_view(&Default::default());
        post.run(&device, &mut encoder, (&color, &depth), &view, [4, 4], None);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            screen.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(4),
                },
            },
            screen.size(),
        );
        queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixel = buffer.slice(..).get_mapped_range().unwrap()[..4].to_vec();
        let near = |got: u8, want: u8| got.abs_diff(want) <= 1;
        assert!(
            near(pixel[0], 204) && near(pixel[1], 102) && near(pixel[2], 51) && near(pixel[3], 128),
            "{pixel:?}"
        );
    }
}
