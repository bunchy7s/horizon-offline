//! FSR 3.1's frame interpolation (AMD FidelityFX SDK 1.1.4, `ffx_frameinterpolation*`,
//! MIT), ported to WGSL (`interpolation.wgsl`): the frame halfway between the previous
//! real frame and this one, from the camera's motion vectors, the optical flow
//! (`optical_flow.rs`) and the depth.
//!
//! The passes and their order are `ffxFrameInterpolationPrepare` then
//! `ffxFrameInterpolationDispatch`'s: dilate the depth and motion vectors and estimate
//! the previous frame's depth; estimate the made frame's depth; splat the game's and the
//! optical flow's vectors to the made frame's pixels (atomic maximums of packed
//! priorities and vectors) and fill the game field's holes from a pyramid; mask what each
//! frame can't see; blend the two frames along both fields; fill what is left from a
//! pyramid of the result. What differs from AMD's, all forced by WGSL or by Horizon:
//! the game's motion vectors are the camera's, from the depth and `Inputs::reproject`,
//! except where `Inputs::object_motion` has an entity's own; textures AMD updates
//! with image atomics are storage buffers; SPD's single-pass pyramids are a pass per
//! level; and there is no HUD-less pass, as Horizon draws its HUD over the made frame.

use glam::Mat4;

/// One real frame's inputs.
pub struct Inputs<'a> {
    /// This frame's colour without the hand and HUD: `display` size, a filterable
    /// float texture (gamma-encoded RGB, the swapchain's values) that can be copied from.
    pub color: &'a wgpu::TextureView,
    /// This frame's depth: `Depth32Float` at `render_size`, reversed (1 at the near
    /// plane, 0 at the far one), finite, jittered by `jitter`.
    pub depth: &'a wgpu::TextureView,
    pub render_size: [u32; 2],
    /// The projection's sub-pixel jitter this frame, in render pixels (FSR's
    /// `jitterOffset`); zero without TAA.
    pub jitter: [f32; 2],
    /// This frame's unjittered clip space to the previous frame's (camera-relative, as
    /// `world_post::reprojection`): the motion vectors are the camera's, from this and
    /// the depth.
    pub reproject: Mat4,
    /// What moved on its own: `motion::FORMAT` at `render_size`, xy its motion vector
    /// (FSR's convention, in UV) where z is 1, used over the camera's.
    pub object_motion: &'a wgpu::TextureView,
    pub near: f32,
    pub far: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    /// The previous frame is unrelated (first frame, resize, teleport): show this one.
    pub reset: bool,
    /// The optical flow from the previous frame's `color` to this one's
    /// (`OpticalFlow::vectors`, `scene_change`).
    pub flow_vectors: &'a wgpu::TextureView,
    pub scene_change: &'a wgpu::Buffer,
}

/// `interpolation.wgsl`'s `Params`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    reproject: [[f32; 4]; 4],
    device_to_view: [f32; 4],
    render_size: [i32; 2],
    display_size: [i32; 2],
    jitter: [f32; 2],
    optical_flow_scale: [f32; 2],
    optical_flow_size: [i32; 2],
    reset: u32,
    tan_half_fov: f32,
    level_size: [i32; 2],
    _pad: [u32; 2],
}

/// The game vector field pyramid's levels (`LoadInpaintedGameFieldMv`) and the
/// inpainting pyramid's (`ComputeInpainting`).
const GAME_LEVELS: u32 = 11;
const INPAINTING_LEVELS: u32 = 10;

struct Pipelines {
    prepare: wgpu::ComputePipeline,
    setup: wgpu::ComputePipeline,
    reconstruct_previous_depth: wgpu::ComputePipeline,
    game_motion_vector_field: wgpu::ComputePipeline,
    game_field_pyramid_first: wgpu::ComputePipeline,
    game_field_pyramid: wgpu::ComputePipeline,
    optical_flow_vector_field: wgpu::ComputePipeline,
    disocclusion_mask: wgpu::ComputePipeline,
    interpolate: wgpu::ComputePipeline,
    inpainting_pyramid_first: wgpu::ComputePipeline,
    inpainting_pyramid: wgpu::ComputePipeline,
    inpainting: wgpu::ComputePipeline,
}

/// A texture with a view of each of its levels (`levels`, for writing one from the one
/// above) and of all of them.
struct Pyramid {
    all: wgpu::TextureView,
    levels: Vec<wgpu::TextureView>,
    sizes: Vec<[u32; 2]>,
}

/// What is sized like the world's render.
struct RenderTargets {
    size: [u32; 2],
    dilated_depth: wgpu::TextureView,
    dilated_mv: wgpu::TextureView,
    rec_prev_depth: wgpu::Buffer,
    rec_interp_depth: wgpu::Buffer,
    game_x: wgpu::Buffer,
    game_y: wgpu::Buffer,
    of_x: wgpu::Buffer,
    of_y: wgpu::Buffer,
    disocclusion: wgpu::TextureView,
    game_pyramid: Pyramid,
}

pub struct Interpolation {
    display: [u32; 2],
    pipelines: Pipelines,
    sampler: wgpu::Sampler,
    /// The passes' constants, and each pyramid level's.
    params: wgpu::Buffer,
    game_level_params: Vec<wgpu::Buffer>,
    inpainting_level_params: Vec<wgpu::Buffer>,
    counters: wgpu::Buffer,
    interpolated: wgpu::TextureView,
    output: wgpu::TextureView,
    inpainting_pyramid: Pyramid,
    /// The last dispatch's colour (`PREVIOUS_INTERPOLATION_SOURCE`), made on the first.
    previous: Option<wgpu::Texture>,
    render: Option<RenderTargets>,
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    levels: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(4),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn uniform(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Frame interpolation constants"),
        size: std::mem::size_of::<Params>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Levels of `base >> (i + 1)` for `i` below `max`, while both sides are 1 or more.
fn pyramid(device: &wgpu::Device, label: &str, base: [u32; 2], max: u32) -> Pyramid {
    let sizes: Vec<[u32; 2]> = (0..max)
        .map(|i| base.map(|v| v >> (i + 1)))
        .take_while(|s| s[0] >= 1 && s[1] >= 1)
        .collect();
    let levels = (sizes.len() as u32).max(1);
    let t = texture(
        device,
        label,
        base.map(|v| (v / 2).max(1)),
        wgpu::TextureFormat::Rgba16Float,
        levels,
    );
    Pyramid {
        all: t.create_view(&Default::default()),
        levels: (0..levels)
            .map(|i| {
                t.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect(),
        sizes,
    }
}

impl RenderTargets {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let view = |t: wgpu::Texture| t.create_view(&Default::default());
        let field = |label| storage(device, label, u64::from(size[0] * size[1]) * 4);
        Self {
            size,
            dilated_depth: view(texture(
                device,
                "Frame interpolation dilated depth",
                size,
                wgpu::TextureFormat::R32Float,
                1,
            )),
            dilated_mv: view(texture(
                device,
                "Frame interpolation dilated motion vectors",
                size,
                wgpu::TextureFormat::Rg32Float,
                1,
            )),
            rec_prev_depth: field("Frame interpolation previous depth"),
            rec_interp_depth: field("Frame interpolation interpolated depth"),
            game_x: field("Frame interpolation game vectors x"),
            game_y: field("Frame interpolation game vectors y"),
            of_x: field("Frame interpolation flow vectors x"),
            of_y: field("Frame interpolation flow vectors y"),
            disocclusion: view(texture(
                device,
                "Frame interpolation disocclusion",
                size,
                wgpu::TextureFormat::Rgba8Unorm,
                1,
            )),
            game_pyramid: pyramid(
                device,
                "Frame interpolation game vector pyramid",
                size,
                GAME_LEVELS,
            ),
        }
    }
}

fn bind(
    device: &wgpu::Device,
    pipeline: &wgpu::ComputePipeline,
    entries: &[(u32, wgpu::BindingResource)],
) -> wgpu::BindGroup {
    let entries: Vec<_> = entries
        .iter()
        .map(|(binding, resource)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: resource.clone(),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Frame interpolation"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    })
}

fn view(v: &wgpu::TextureView) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::TextureView(v)
}

fn whole(b: &wgpu::Buffer) -> wgpu::BindingResource<'_> {
    b.as_entire_binding()
}

/// `setupDeviceDepthToViewSpaceDepthParams` for inverted, finite depth, in blocks
/// (`viewSpaceToMetersFactor` 1).
fn device_to_view(near: f32, far: f32, fov_y: f32, aspect: f32) -> [f32; 4] {
    let (f_min, f_max) = (near.max(far), near.min(far));
    let q = f_max / (f_min - f_max);
    let cot_half_fov = (0.5 * fov_y).cos() / (0.5 * fov_y).sin();
    [-q, q * f_min, aspect / cot_half_fov, 1.0 / cot_half_fov]
}

impl Interpolation {
    /// For frames of `display` size; the render size may change between dispatches.
    pub fn new(device: &wgpu::Device, display: [u32; 2]) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Frame interpolation"),
            source: wgpu::ShaderSource::Wgsl(include_str!("interpolation.wgsl").into()),
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let view = |t: wgpu::Texture| t.create_view(&Default::default());
        Self {
            display,
            pipelines: Pipelines {
                prepare: pipeline("prepare_pass"),
                setup: pipeline("setup_pass"),
                reconstruct_previous_depth: pipeline("reconstruct_previous_depth_pass"),
                game_motion_vector_field: pipeline("game_motion_vector_field_pass"),
                game_field_pyramid_first: pipeline("game_field_pyramid_first_pass"),
                game_field_pyramid: pipeline("game_field_pyramid_pass"),
                optical_flow_vector_field: pipeline("optical_flow_vector_field_pass"),
                disocclusion_mask: pipeline("disocclusion_mask_pass"),
                interpolate: pipeline("interpolate_pass"),
                inpainting_pyramid_first: pipeline("inpainting_pyramid_first_pass"),
                inpainting_pyramid: pipeline("inpainting_pyramid_pass"),
                inpainting: pipeline("inpainting_pass"),
            },
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Frame interpolation"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            params: uniform(device),
            game_level_params: (0..GAME_LEVELS).map(|_| uniform(device)).collect(),
            inpainting_level_params: (0..INPAINTING_LEVELS).map(|_| uniform(device)).collect(),
            counters: storage(device, "Frame interpolation counters", 8),
            interpolated: view(texture(
                device,
                "Frame interpolation",
                display,
                wgpu::TextureFormat::Rgba16Float,
                1,
            )),
            output: view(texture(
                device,
                "Interpolated frame",
                display,
                wgpu::TextureFormat::Rgba16Float,
                1,
            )),
            inpainting_pyramid: pyramid(
                device,
                "Frame interpolation inpainting pyramid",
                display,
                INPAINTING_LEVELS,
            ),
            previous: None,
            render: None,
        }
    }

    /// The frame halfway between the previous dispatch's `color` and this one's, into
    /// `output`. Keeps what it needs of this frame for the next dispatch.
    pub fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &Inputs,
    ) {
        let color = inputs.color.texture();
        let mut reset = inputs.reset;
        if self
            .previous
            .as_ref()
            .is_none_or(|p| p.size() != color.size() || p.format() != color.format())
        {
            self.previous = Some(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Frame interpolation previous frame"),
                size: color.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: color.format(),
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
            reset = true;
        }
        if self
            .render
            .as_ref()
            .is_none_or(|r| r.size != inputs.render_size)
        {
            self.render = Some(RenderTargets::new(device, inputs.render_size));
            reset = true;
        }
        let r = self.render.as_ref().expect("made above");
        let previous_texture = self.previous.as_ref().expect("made above");
        let previous = previous_texture.create_view(&Default::default());
        let display = self.display.map(|v| v as i32);
        let render = inputs.render_size.map(|v| v as i32);
        let flow_size = self
            .display
            .map(|v| v.div_ceil(super::optical_flow::BLOCK_SIZE) as i32);
        let base = Params {
            reproject: inputs.reproject.to_cols_array_2d(),
            device_to_view: device_to_view(
                inputs.near,
                inputs.far,
                inputs.fov_y,
                render[0] as f32 / render[1].max(1) as f32,
            ),
            render_size: render,
            display_size: display,
            jitter: inputs.jitter,
            optical_flow_scale: self.display.map(|v| 1.0 / v as f32),
            optical_flow_size: flow_size,
            reset: u32::from(reset),
            tan_half_fov: (0.5 * inputs.fov_y).tan() * render[0] as f32 / render[1].max(1) as f32,
            level_size: [0; 2],
            _pad: [0; 2],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&base));
        let level_params = |buffers: &[wgpu::Buffer], sizes: &[[u32; 2]]| {
            for (buffer, size) in buffers.iter().zip(sizes) {
                let params = Params {
                    level_size: size.map(|v| v as i32),
                    ..base
                };
                queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
            }
        };
        level_params(&self.game_level_params, &r.game_pyramid.sizes);
        level_params(
            &self.inpainting_level_params,
            &self.inpainting_pyramid.sizes,
        );
        for b in [
            &r.rec_prev_depth,
            &r.rec_interp_depth,
            &r.game_x,
            &r.game_y,
            &r.of_x,
            &r.of_y,
        ] {
            encoder.clear_buffer(b, 0, None);
        }
        if reset {
            encoder.clear_buffer(&self.counters, 0, None);
        }
        let p = &self.pipelines;
        let params = || whole(&self.params);
        let sampler = || wgpu::BindingResource::Sampler(&self.sampler);
        let groups =
            |pipeline, entries: &[(u32, wgpu::BindingResource)]| bind(device, pipeline, entries);
        let prepare = groups(
            &p.prepare,
            &[
                (0, params()),
                (2, view(inputs.depth)),
                (34, view(inputs.object_motion)),
                (5, view(&r.dilated_depth)),
                (6, view(&r.dilated_mv)),
                (9, whole(&r.rec_prev_depth)),
            ],
        );
        let setup = groups(
            &p.setup,
            &[
                (0, params()),
                (22, whole(inputs.scene_change)),
                (23, whole(&self.counters)),
            ],
        );
        let colors = || {
            [
                (3, view(inputs.color)),
                (4, view(&previous)),
                (1, sampler()),
            ]
        };
        let reconstruct = groups(
            &p.reconstruct_previous_depth,
            &[
                (0, params()),
                (7, view(&r.dilated_depth)),
                (8, view(&r.dilated_mv)),
                (10, whole(&r.rec_interp_depth)),
            ],
        );
        let mut entries = vec![
            (0, params()),
            (7, view(&r.dilated_depth)),
            (8, view(&r.dilated_mv)),
            (13, whole(&r.game_x)),
            (14, whole(&r.game_y)),
        ];
        entries.extend(colors());
        let game_field = groups(&p.game_motion_vector_field, &entries);
        let game_pyramid: Vec<_> = (0..r.game_pyramid.sizes.len())
            .map(|i| {
                let mut entries = vec![
                    (0, whole(&self.game_level_params[i])),
                    (26, view(&r.game_pyramid.levels[i])),
                ];
                if i == 0 {
                    entries.extend([(15, whole(&r.game_x)), (16, whole(&r.game_y))]);
                    groups(&p.game_field_pyramid_first, &entries)
                } else {
                    entries.push((25, view(&r.game_pyramid.levels[i - 1])));
                    groups(&p.game_field_pyramid, &entries)
                }
            })
            .collect();
        let mut entries = vec![
            (0, params()),
            (21, view(inputs.flow_vectors)),
            (17, whole(&r.of_x)),
            (18, whole(&r.of_y)),
        ];
        entries.extend(colors());
        let flow_field = groups(&p.optical_flow_vector_field, &entries);
        let game_mv = || {
            [
                (15, whole(&r.game_x)),
                (16, whole(&r.game_y)),
                (27, view(&r.game_pyramid.all)),
            ]
        };
        let mut entries = vec![
            (0, params()),
            (7, view(&r.dilated_depth)),
            (11, whole(&r.rec_prev_depth)),
            (12, whole(&r.rec_interp_depth)),
            (29, view(&r.disocclusion)),
        ];
        entries.extend(game_mv());
        let disocclusion = groups(&p.disocclusion_mask, &entries);
        let mut entries = vec![
            (0, params()),
            (19, whole(&r.of_x)),
            (20, whole(&r.of_y)),
            (24, whole(&self.counters)),
            (30, view(&r.disocclusion)),
            (31, view(&self.interpolated)),
        ];
        entries.extend(game_mv());
        entries.extend(colors());
        let interpolate = groups(&p.interpolate, &entries);
        let inpainting_pyramid: Vec<_> = (0..self.inpainting_pyramid.sizes.len())
            .map(|i| {
                let mut entries = vec![
                    (0, whole(&self.inpainting_level_params[i])),
                    (26, view(&self.inpainting_pyramid.levels[i])),
                ];
                if i == 0 {
                    entries.push((32, view(&self.interpolated)));
                    groups(&p.inpainting_pyramid_first, &entries)
                } else {
                    entries.push((25, view(&self.inpainting_pyramid.levels[i - 1])));
                    groups(&p.inpainting_pyramid, &entries)
                }
            })
            .collect();
        let inpainting = groups(
            &p.inpainting,
            &[
                (0, params()),
                (28, view(&self.inpainting_pyramid.all)),
                (32, view(&self.interpolated)),
                (33, view(&self.output)),
            ],
        );
        let groups_for = |size: [u32; 2]| (size[0].div_ceil(8), size[1].div_ceil(8));
        let (rx, ry) = groups_for(inputs.render_size);
        let (dx, dy) = groups_for(self.display);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Frame interpolation"),
            timestamp_writes: None,
        });
        let mut run = |pipeline: &wgpu::ComputePipeline, group: &wgpu::BindGroup, (x, y)| {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(x, y, 1);
        };
        run(&p.prepare, &prepare, (rx, ry));
        run(&p.setup, &setup, (1, 1));
        if !reset {
            run(&p.reconstruct_previous_depth, &reconstruct, (rx, ry));
            run(&p.game_motion_vector_field, &game_field, (rx, ry));
            for (i, (group, size)) in game_pyramid.iter().zip(&r.game_pyramid.sizes).enumerate() {
                let pipeline = if i == 0 {
                    &p.game_field_pyramid_first
                } else {
                    &p.game_field_pyramid
                };
                run(pipeline, group, groups_for(*size));
            }
            run(
                &p.optical_flow_vector_field,
                &flow_field,
                groups_for(flow_size.map(|v| v as u32)),
            );
            run(&p.disocclusion_mask, &disocclusion, (rx, ry));
        }
        run(&p.interpolate, &interpolate, (dx, dy));
        for (i, (group, size)) in inpainting_pyramid
            .iter()
            .zip(&self.inpainting_pyramid.sizes)
            .enumerate()
        {
            let pipeline = if i == 0 {
                &p.inpainting_pyramid_first
            } else {
                &p.inpainting_pyramid
            };
            run(pipeline, group, groups_for(*size));
        }
        run(&p.inpainting, &inpainting, (dx, dy));
        drop(pass);
        encoder.copy_texture_to_texture(
            color.as_image_copy(),
            previous_texture.as_image_copy(),
            color.size(),
        );
    }

    /// The interpolated frame: `display` size, `Rgba16Float`, sampled; its colour is
    /// encoded as `Inputs::color`'s is.
    pub fn output(&self) -> &wgpu::TextureView {
        &self.output
    }
}

#[cfg(test)]
mod tests {
    use super::super::optical_flow::{OpticalFlow, tests as of};
    use super::*;

    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("interpolation.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }

    fn f16(bits: u16) -> f32 {
        let (sign, exp, frac) = (bits >> 15, (bits >> 10) & 0x1f, bits & 0x3ff);
        let v = match exp {
            0 => f32::from(frac) / 1024.0 * 2f32.powi(-14),
            31 => f32::INFINITY,
            e => (1.0 + f32::from(frac) / 1024.0) * 2f32.powi(i32::from(e) - 15),
        };
        if sign == 1 { -v } else { v }
    }

    /// Runs `frames` (pattern shifts) through the optical flow and the interpolation with
    /// a camera that moves by `reproject` each frame, and returns the mean error of the
    /// last made frame's interior against `expected`, and the last two real frames'.
    /// `object`: every pixel's own motion vector (UV), as if all of it moved on its own.
    fn interpolate(
        seed: u32,
        shifts: &[[i32; 2]],
        reproject: Mat4,
        object: Option<[f32; 2]>,
        expected: [i32; 2],
    ) -> Option<[f32; 3]> {
        let (device, queue) = of::device()?;
        let size = [512, 288];
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = depth.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.01),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        queue.submit([encoder.finish()]);
        let object_motion = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: super::super::motion::FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        if let Some(mv) = object {
            let texel = [mv[0], mv[1], 1.0, 0.0].map(|v| half::f16::from_f32(v).to_bits());
            let texels: Vec<u16> = (0..size[0] * size[1]).flat_map(|_| texel).collect();
            queue.write_texture(
                object_motion.as_image_copy(),
                bytemuck::cast_slice(&texels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size[0] * 8),
                    rows_per_image: None,
                },
                object_motion.size(),
            );
        }
        let object_motion = object_motion.create_view(&Default::default());
        let mut flow = OpticalFlow::new(&device, size);
        let mut interpolation = Interpolation::new(&device, size);
        let frames: Vec<_> = shifts
            .iter()
            .map(|s| {
                let pixels = of::pattern(size, seed, *s);
                // COPY_SRC as the frame generation's world texture has.
                let t = device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                queue.write_texture(
                    t.as_image_copy(),
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size[0] * 4),
                        rows_per_image: None,
                    },
                    t.size(),
                );
                (t.create_view(&Default::default()), pixels)
            })
            .collect();
        for (i, (color, _)) in frames.iter().enumerate() {
            let mut encoder = device.create_command_encoder(&Default::default());
            flow.dispatch(&device, &queue, &mut encoder, color, i == 0);
            interpolation.dispatch(
                &device,
                &queue,
                &mut encoder,
                &Inputs {
                    color,
                    depth: &depth,
                    render_size: size,
                    jitter: [0.0; 2],
                    reproject: if i == 0 { Mat4::IDENTITY } else { reproject },
                    object_motion: &object_motion,
                    near: 0.05,
                    far: 512.0,
                    fov_y: 1.2,
                    reset: i == 0,
                    flow_vectors: flow.vectors(),
                    scene_change: flow.scene_change(),
                },
            );
            queue.submit([encoder.finish()]);
        }
        let out = of::read(&device, &queue, interpolation.output.texture(), 8);
        let made: Vec<f32> = out
            .chunks(2)
            .map(|c| f16(u16::from_le_bytes([c[0], c[1]])))
            .collect();
        let truth = of::pattern(size, seed, expected);
        let n = frames.len();
        let error = |pixels: &dyn Fn(usize) -> f32| {
            let (mut sum, mut count) = (0.0, 0.0);
            for y in 32..size[1] as usize - 32 {
                for x in 32..size[0] as usize - 32 {
                    for c in 0..3 {
                        let i = (y * size[0] as usize + x) * 4 + c;
                        sum += (pixels(i) - f32::from(truth[i]) / 255.0).abs();
                        count += 1.0;
                    }
                }
            }
            sum / count
        };
        Some([
            error(&|i| made[i]),
            error(&|i| f32::from(frames[n - 2].1[i]) / 255.0),
            error(&|i| f32::from(frames[n - 1].1[i]) / 255.0),
        ])
    }

    /// Something moving on its own (the camera still): the optical flow puts the made
    /// frame's blocks halfway where its colours match better than the still camera's
    /// (FSR prefers the game's vectors unless they clearly don't: primaries make it
    /// clear). The first 10 frames after a reset use the game's vectors only.
    #[test]
    fn moves_what_the_optical_flow_follows_halfway() {
        let mut shifts = vec![[0, 0]; 20];
        shifts.push([16, 8]);
        let Some([made, previous, current]) =
            interpolate(100, &shifts, Mat4::IDENTITY, None, [8, 4])
        else {
            return;
        };
        // Where the still camera's two colours happen to be the same primary (a third of
        // the pixels), FSR keeps the game's vectors.
        assert!(
            made * 3.0 < previous.min(current),
            "{made} against {previous} and {current}"
        );
    }

    /// The camera turning: the game's motion vectors put the made frame halfway.
    #[test]
    fn moves_the_view_halfway_with_the_camera() {
        let size = [512.0, 288.0];
        // What is at a pixel now was 16 pixels to its right last frame.
        let reproject = Mat4::from_translation(glam::Vec3::new(32.0 / size[0], 0.0, 0.0));
        let shifts = [[0, 0], [-16, 0], [-32, 0], [-48, 0]];
        let Some([made, previous, current]) = interpolate(1, &shifts, reproject, None, [-40, 0])
        else {
            return;
        };
        assert!(
            made * 4.0 < previous.min(current),
            "{made} against {previous} and {current}"
        );
    }

    /// Something moving on its own with its own motion vectors (the camera still): the
    /// made frame is halfway, as with the camera's.
    #[test]
    fn moves_what_has_its_own_motion_vectors_halfway() {
        let size = [512.0, 288.0];
        let shifts = [[0, 0], [16, 8], [32, 16], [48, 24]];
        let mv = [-16.0 / size[0], -8.0 / size[1]];
        let Some([made, previous, current]) =
            interpolate(1, &shifts, Mat4::IDENTITY, Some(mv), [40, 20])
        else {
            return;
        };
        assert!(
            made * 4.0 < previous.min(current),
            "{made} against {previous} and {current}"
        );
    }
}
