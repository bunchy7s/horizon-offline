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
type WorldViews = (wgpu::TextureView, wgpu::TextureView, wgpu::TextureView);

/// Where a post pass draws.
#[derive(Clone, Copy, PartialEq)]
enum Writes {
    /// One of the chain's two targets.
    Chain(usize),
    /// A pack target (`[[post]] output`).
    Target(usize),
    /// The screen: the last pass.
    Screen,
}

/// A post pass's target, and the chain's picture it reads as `previous` (the world's for
/// none).
#[derive(Clone, Copy)]
struct PassPlan {
    writes: Writes,
    reads: Option<usize>,
}

/// The passes' plan for `outputs` (each pass's `[[post]] output`, by target index): the chain's
/// passes write its targets in turn, each reading what the one before wrote; a pass with an
/// output writes that and reads the chain's picture, which goes on past it.
fn plan(outputs: &[Option<usize>]) -> Vec<PassPlan> {
    let mut current = None;
    outputs
        .iter()
        .enumerate()
        .map(|(i, output)| {
            let reads = current;
            let writes = match output {
                _ if i + 1 == outputs.len() => Writes::Screen,
                Some(t) => Writes::Target(*t),
                None => {
                    let slot = usize::from(current == Some(0));
                    current = Some(slot);
                    Writes::Chain(slot)
                }
            };
            PassPlan { writes, reads }
        })
        .collect()
}

/// A pack target's size: `scale` of the world's `size`, at least a texel.
fn scaled(size: [u32; 2], scale: f32) -> [u32; 2] {
    size.map(|s| ((s as f32 * scale).ceil() as u32).max(1))
}

pub struct NativePost {
    passes: Vec<(String, wgpu::RenderPipeline)>,
    layout: wgpu::BindGroupLayout,
    /// The pack's textures.
    textures: Vec<(wgpu::TextureView, wgpu::Sampler)>,
    frame: wgpu::Buffer,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    /// Two targets of the world's size, written in turn (none with a single pass).
    targets: Option<([u32; 2], [wgpu::TextureView; 2])>,
    /// The frame's shadow map, its comparison sampler (bindings 6 and 7), the translucent map
    /// and its colour (bindings 9 and 10).
    shadow: (
        wgpu::TextureView,
        wgpu::Sampler,
        wgpu::TextureView,
        wgpu::TextureView,
    ),
    /// Last frame's input to the last pass (`history`, binding 8), made with the targets,
    /// and a black 1×1 in its place before then.
    history: Option<wgpu::Texture>,
    history_blank: wgpu::Texture,
    /// One bind group per pass, for these world views and either the valid history or black.
    groups: Option<(WorldViews, bool, Vec<wgpu::BindGroup>)>,
    /// Whether `history` contains the last frame's input to the last pass.
    history_valid: bool,
    /// What each pass writes and reads (`[[post]] output`).
    plan: Vec<PassPlan>,
    /// The pack's targets' scales (`Manifest::post_targets`), and their views for the world's
    /// size.
    scales: Vec<f32>,
    named: Option<([u32; 2], Vec<wgpu::TextureView>)>,
    /// The screen's colour format, the world target's too.
    format: wgpu::TextureFormat,
}

/// The `frame` every stage of a pack reads (group 3, binding 0), written once a frame, and
/// what the material stage reads besides.
pub struct NativeFrame {
    /// The solid terrain's second target, the post stage's `scene_surface`.
    surface: wgpu::Texture,
    pub surface_view: wgpu::TextureView,
    pub(super) buffer: wgpu::Buffer,
    /// Group 3 of the sky and material stages.
    pub layout: wgpu::BindGroupLayout,
    /// The pack's `[[textures]]` and their samplers, bound for every stage.
    pub textures: Vec<(wgpu::TextureView, wgpu::Sampler)>,
    /// The colour of block light around the camera (`[[lights]]`), bindings 9 and 10.
    pub block_light: super::block_light::BlockLight,
    pub group: wgpu::BindGroup,
    /// The world's depth before translucent terrain (the material stage's `opaque_depth`,
    /// binding 6), copied each frame.
    pub opaque_depth: wgpu::Texture,
    /// The world's colour before translucent terrain (`opaque_color`, binding 11), copied
    /// with it; 1×1 while the world draws straight to the window.
    pub opaque_color: wgpu::Texture,
    /// The resource packs' LabPBR maps (bindings 12 and 13: normals, specular), a blank 1x1
    /// without them.
    pbr: [wgpu::TextureView; 2],
    /// The material stage's `reflection` (binding 1): this frame's water mirror, else
    /// `blank`; and its sampler (binding 2).
    reflection: wgpu::TextureView,
    blank: wgpu::TextureView,
    reflection_sampler: wgpu::Sampler,
    /// The shadow map the stages read, and the camera that draws it when the pack asks for one.
    pub shadow_map: super::native_shadow::ShadowMap,
    shadow_camera: Option<horizon_shaders::Shadow>,
    /// Whether the pack's material reads `reflection` (`NativePack::new`): else no mirror
    /// is drawn and `has_reflection` stays 0.
    pub reflects: bool,
    /// Last frame's view-projection, camera and time (`write`).
    last: Option<([[f32; 4]; 4], glam::DVec3, std::time::Instant)>,
    /// The eased eye light (block, sky) and being in water and lava (`write`); NaN until the
    /// first frame sets them.
    eye: [f32; 4],
    /// A post stage must use black history after a long gap until it has rendered a frame.
    history_reset: bool,
    start: std::time::Instant,
    frames: u32,
    /// `frame.wetness`, and when it was last written.
    wetness: (f32, Option<std::time::Instant>),
}

/// A depth texture of `size` the world's depth is copied into.
fn depth_copy(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    copy_target(
        device,
        "Native shader pack opaque depth",
        size,
        wgpu::TextureFormat::Depth32Float,
    )
}

/// A colour texture of `size` and `format` the world's colour is copied into.
fn color_copy(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat) -> wgpu::Texture {
    copy_target(device, "Native shader pack opaque colour", size, format)
}

fn copy_target(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
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
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
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

/// The size of the water mirror for a world of `size`: half of it.
pub fn reflection_size(size: [u32; 2]) -> [u32; 2] {
    size.map(|s| s.div_ceil(2).max(1))
}

/// Seconds between frames past which the last one is forgotten (`NativeFrame::write`).
const MAX_FRAME_GAP: f32 = 0.5;

/// `value` moved toward `target` over `dt` seconds by a half-life in seconds; a NaN `value`
/// (the first frame) starts at `target`.
fn eased(value: f32, target: f32, half_life: f32, dt: f32) -> f32 {
    if value.is_nan() {
        target
    } else {
        target + (value - target) * 0.5f32.powf(dt / half_life)
    }
}

impl NativeFrame {
    /// The frame of a pack with the `[shadow]` settings `shadow`, if any, and the
    /// block light colours `lights`.
    pub fn new(
        device: &wgpu::Device,
        shadow: Option<&horizon_shader_api::manifest::Shadow>,
        textures: Vec<(wgpu::TextureView, wgpu::Sampler)>,
        lights: super::block_light::LightTable,
        pbr: Option<&[wgpu::Texture; 2]>,
    ) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Native shader pack frame"),
            size: size_of::<FrameInputs>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut entries = vec![
            frame_entry(wgpu::ShaderStages::VERTEX_FRAGMENT),
            texture_entry(1, wgpu::TextureSampleType::Float { filterable: true }),
            sampler_entry(2),
            texture_entry(3, wgpu::TextureSampleType::Depth),
            comparison_entry(4),
            texture_entry(6, wgpu::TextureSampleType::Depth),
            texture_entry(7, wgpu::TextureSampleType::Depth),
            texture_entry(8, wgpu::TextureSampleType::Float { filterable: true }),
            texture_entry(11, wgpu::TextureSampleType::Float { filterable: false }),
            pbr_entry(12),
            pbr_entry(13),
        ];
        let block_light_entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        entries.extend(block_light_entries);
        entries.extend(pack_texture_entries(textures.len()));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack frame"),
            entries: &entries,
        });
        let surface = surface_target(device, [1, 1]);
        let surface_view = surface.create_view(&Default::default());
        let opaque_depth = depth_copy(device, [1, 1]);
        let opaque_color = color_copy(device, [1, 1], wgpu::TextureFormat::Rgba8Unorm);
        let blank = copy_target(
            device,
            "Native shader pack reflection blank",
            [1, 1],
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .create_view(&Default::default());
        let reflection_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Native shader pack reflection sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_map = super::native_shadow::ShadowMap::new(device, shadow);
        let block_light = super::block_light::BlockLight::new(device, lights);
        let array_view = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let pbr = match pbr {
            Some(maps) => maps.each_ref().map(array_view),
            None => {
                // Two layers: one would make a 2D texture on the OpenGL backend.
                let blank = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Native shader pack LabPBR (none)"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 2,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                [array_view(&blank), array_view(&blank)]
            }
        };
        let group = Self::group(
            device,
            &layout,
            &buffer,
            (&blank, &reflection_sampler),
            &shadow_map,
            (&opaque_depth, &opaque_color),
            &textures,
            &block_light,
            &pbr,
        );
        Self {
            textures,
            block_light,
            pbr,
            buffer,
            layout,
            group,
            surface,
            surface_view,
            opaque_depth,
            opaque_color,
            reflection: blank.clone(),
            blank,
            reflection_sampler,
            shadow_map,
            shadow_camera: shadow.map(super::native_shadow::camera),
            reflects: false,
            last: None,
            eye: [f32::NAN; 4],
            history_reset: true,
            start: std::time::Instant::now(),
            frames: 0,
            wetness: (0.0, None),
        }
    }

    /// What the shadow camera sees this frame, for the renderer's shadow lists.
    pub fn shadow_view(&self, frame: &Frame) -> Option<super::pack_post::ShadowView> {
        let camera = self.shadow_camera.as_ref()?;
        Some(super::pack_post::ShadowView {
            cull: super::pack_uniforms::shadow_culling(frame, camera),
            light: super::pack_uniforms::shadow_light(frame, camera),
            // The light's box culls: on the ground it reaches further than `distance` along
            // the sun's azimuth while the sun is low.
            distance: None,
            unculled: None,
        })
    }

    fn group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        (reflection, sampler): (&wgpu::TextureView, &wgpu::Sampler),
        shadow: &super::native_shadow::ShadowMap,
        (opaque_depth, opaque_color): (&wgpu::Texture, &wgpu::Texture),
        textures: &[(wgpu::TextureView, wgpu::Sampler)],
        block_light: &super::block_light::BlockLight,
        pbr: &[wgpu::TextureView; 2],
    ) -> wgpu::BindGroup {
        let opaque_depth = opaque_depth.create_view(&Default::default());
        let opaque_color = opaque_color.create_view(&Default::default());
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(reflection),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&shadow.view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&shadow.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&shadow.translucent_view),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::TextureView(&shadow.color_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&opaque_depth),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::TextureView(&opaque_color),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: wgpu::BindingResource::TextureView(&pbr[0]),
            },
            wgpu::BindGroupEntry {
                binding: 13,
                resource: wgpu::BindingResource::TextureView(&pbr[1]),
            },
        ];
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(&block_light.view),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: block_light.grid.as_entire_binding(),
            },
        ]);
        entries.extend(pack_texture_group_entries(textures));
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Native shader pack frame"),
            layout,
            entries: &entries,
        })
    }

    /// Keeps the opaque copies the size of the world's depth `depth` and, where the world's
    /// colour `color` can be copied (the scaled target, not the window's own), its size and
    /// format; the group is made again when they change.
    pub fn fit_opaque(
        &mut self,
        device: &wgpu::Device,
        depth: &wgpu::Texture,
        color: Option<&wgpu::Texture>,
    ) {
        let size = [depth.width(), depth.height()];
        let (color_size, format) = match color {
            Some(c) => ([c.width(), c.height()], c.format()),
            None => ([1, 1], self.opaque_color.format()),
        };
        let color_fits = [self.opaque_color.width(), self.opaque_color.height()] == color_size
            && self.opaque_color.format() == format;
        let depth_fits = [self.opaque_depth.width(), self.opaque_depth.height()] == size;
        if depth_fits && color_fits {
            return;
        }
        if !color_fits {
            self.opaque_color = color_copy(device, color_size, format);
        }
        if !depth_fits {
            self.opaque_depth = depth_copy(device, size);
        }
        self.regroup(device);
    }

    /// Sizes the surface target: the world's while a material stage writes it, else a blank.
    pub fn fit_surface(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if [self.surface.width(), self.surface.height()] != size {
            self.surface = surface_target(device, size);
            self.surface_view = self.surface.create_view(&Default::default());
        }
    }

    /// Binds `view` as this frame's `reflection` (`None`: the blank). The mirror is drawn
    /// before the stages read it.
    pub fn set_reflection(&mut self, device: &wgpu::Device, view: Option<&wgpu::TextureView>) {
        let view = view.unwrap_or(&self.blank);
        if *view != self.reflection {
            self.reflection = view.clone();
            self.regroup(device);
        }
    }

    fn regroup(&mut self, device: &wgpu::Device) {
        self.group = Self::group(
            device,
            &self.layout,
            &self.buffer,
            (&self.reflection, &self.reflection_sampler),
            &self.shadow_map,
            (&self.opaque_depth, &self.opaque_color),
            &self.textures,
            &self.block_light,
            &self.pbr,
        );
    }

    /// Writes this frame's inputs, for a world of `size`.
    pub fn write(&mut self, queue: &wgpu::Queue, frame: &Frame, size: [u32; 2]) {
        let seconds = self.start.elapsed().as_secs_f32() % 3600.0;
        let now = std::time::Instant::now();
        let since = self.wetness.1.map_or(0.0, |t| (now - t).as_secs_f32());
        self.wetness = (wetness(self.wetness.0, frame.pack.rain, since), Some(now));
        let mut inputs = frame_inputs(
            frame,
            size,
            seconds,
            self.frames,
            self.shadow_camera.as_ref(),
        );
        inputs.wetness = self.wetness.0;
        if !self.reflects {
            inputs.has_reflection = 0;
        }
        let view_proj = (Mat4::from_cols_array_2d(&inputs.projection)
            * Mat4::from_cols_array_2d(&inputs.view))
        .to_cols_array_2d();
        inputs.previous_view_projection = view_proj;
        // `write` skips title frames, so a gap means a pause or another world: the frame
        // before says nothing about this one, and the eased values start over.
        match self.last {
            Some((previous, camera, time)) if (now - time).as_secs_f32() < MAX_FRAME_GAP => {
                inputs.previous_view_projection = previous;
                inputs.camera_motion = (frame.camera - camera).as_vec3().to_array();
                inputs.delta_time = (now - time).as_secs_f32();
            }
            _ => {
                self.eye = [f32::NAN; 4];
                self.history_reset = true;
            }
        }
        self.last = Some((view_proj, frame.camera, now));
        // The eye's light and medium eased by half-lives in seconds, from where they are at
        // first.
        let targets = [
            (inputs.eye_light[0], 2.0),
            (inputs.eye_light[1], 2.0),
            (f32::from(u8::from(inputs.eye_in == 1)), 0.25),
            (f32::from(u8::from(inputs.eye_in == 2)), 0.25),
        ];
        let dt = inputs.delta_time;
        for (value, (target, half_life)) in self.eye.iter_mut().zip(targets) {
            *value = eased(*value, target, half_life, dt);
        }
        let [block, sky, water, lava] = self.eye;
        inputs.eye_light_smooth = [block, sky];
        inputs.in_water = water;
        inputs.in_lava = lava;
        self.frames = self.frames.wrapping_add(1);
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&inputs));
    }

    /// Takes the post stage's pending history reset. It stays pending if post rendering is
    /// skipped for a frame, so the first frame it does render still sees black history.
    pub(super) fn take_history_reset(&mut self) -> bool {
        let reset = self.history_reset;
        self.history_reset = false;
        reset
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

/// The solid terrain's surface target of `size`, drawn and read.
fn surface_target(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Native shader pack surface"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: super::native_material::SURFACE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
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

/// A LabPBR map's array, read by the material entries (`native_material.rs`).
fn pbr_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn comparison_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
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
        manifest: &horizon_shader_api::Manifest,
        constants: &[(String, f64)],
        format: wgpu::TextureFormat,
        frame: &NativeFrame,
    ) -> Result<Self, String> {
        let targets = manifest.post_targets();
        let outputs: Vec<Option<usize>> = manifest
            .post
            .iter()
            .map(|p| {
                let name = p.output.as_deref()?;
                targets.iter().position(|t| t.0 == name)
            })
            .collect();
        let plan = plan(&outputs);
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let mut entries = vec![
            frame_entry(wgpu::ShaderStages::FRAGMENT),
            texture_entry(1, float),
            texture_entry(2, wgpu::TextureSampleType::Depth),
            texture_entry(3, float),
            sampler_entry(4),
            sampler_entry(5),
            texture_entry(6, wgpu::TextureSampleType::Depth),
            comparison_entry(7),
            texture_entry(8, float),
            texture_entry(9, wgpu::TextureSampleType::Depth),
            texture_entry(10, float),
            texture_entry(11, wgpu::TextureSampleType::Float { filterable: false }),
        ];
        entries.extend(pack_texture_entries(frame.textures.len()));
        entries
            .extend((0..targets.len() as u32).map(|j| {
                texture_entry(horizon_shader_api::stages::POST_TARGET_BINDING + j, float)
            }));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack post"),
            entries: &entries,
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
                        format: if plan[i].writes == Writes::Screen {
                            format
                        } else {
                            INTERMEDIATE
                        },
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
            textures: frame.textures.clone(),
            frame: frame.buffer.clone(),
            shadow: (
                frame.shadow_map.view.clone(),
                frame.shadow_map.sampler.clone(),
                frame.shadow_map.translucent_view.clone(),
                frame.shadow_map.color_view.clone(),
            ),
            linear: sampler("Native shader pack linear", wgpu::FilterMode::Linear),
            nearest: sampler("Native shader pack nearest", wgpu::FilterMode::Nearest),
            targets: None,
            history: None,
            history_blank: history_target(device, "Native shader pack history blank", [1, 1]),
            groups: None,
            history_valid: false,
            scales: targets.iter().map(|t| t.1).collect(),
            named: None,
            plan,
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
        world: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
        size: [u32; 2],
        history_reset: bool,
    ) {
        if !self.scales.is_empty() && self.named.as_ref().is_none_or(|n| n.0 != size) {
            let views = self
                .scales
                .iter()
                .map(|&scale| {
                    let size = scaled(size, scale);
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Native shader pack target"),
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
                })
                .collect();
            self.named = Some((size, views));
            self.groups = None;
        }
        let chain = self
            .plan
            .iter()
            .any(|p| matches!(p.writes, Writes::Chain(_)));
        if chain && self.targets.as_ref().is_none_or(|t| t.0 != size) {
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
                        // COPY_SRC: the last pass's input is kept as next frame's `history`.
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_SRC,
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
            self.history = Some(history_target(device, "Native shader pack history", size));
            self.history_valid = false;
            self.groups = None;
        }
        let blank_history = history_reset || !self.history_valid;
        if self
            .groups
            .as_ref()
            .is_some_and(|((color, depth, surface), blank, _)| {
                color == world.0
                    && depth == world.1
                    && surface == world.2
                    && *blank == blank_history
            })
        {
            return;
        }
        let history = if blank_history {
            &self.history_blank
        } else {
            self.history
                .as_ref()
                .expect("history exists after a post stage has more than one pass")
        }
        .create_view(&Default::default());
        let blank = self.history_blank.create_view(&Default::default());
        let groups = (0..self.passes.len())
            .map(|i| {
                // The chain's picture so far: the world's before its first pass.
                let previous = match (&self.targets, self.plan[i].reads) {
                    (Some((_, t)), Some(slot)) => &t[slot],
                    _ => world.0,
                };
                let mut entries = vec![
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
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&self.shadow.0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::Sampler(&self.shadow.1),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&history),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::TextureView(&self.shadow.2),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::TextureView(world.2),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: wgpu::BindingResource::TextureView(&self.shadow.3),
                    },
                ];
                entries.extend(pack_texture_group_entries(&self.textures));
                // The pack's targets, a blank one in place of the one this pass writes.
                if let Some((_, views)) = &self.named {
                    entries.extend(views.iter().enumerate().map(|(j, view)| {
                        let view = if self.plan[i].writes == Writes::Target(j) {
                            &blank
                        } else {
                            view
                        };
                        wgpu::BindGroupEntry {
                            binding: horizon_shader_api::stages::POST_TARGET_BINDING + j as u32,
                            resource: wgpu::BindingResource::TextureView(view),
                        }
                    }));
                }
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Native shader pack post"),
                    layout: &self.layout,
                    entries: &entries,
                })
            })
            .collect();
        self.groups = Some((
            (world.0.clone(), world.1.clone(), world.2.clone()),
            blank_history,
            groups,
        ));
    }

    /// Runs the passes over the world target `world` (colour, depth) of `size`, the last onto
    /// `screen`, timed by `timing` when it times this frame.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        world: (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
        screen: &wgpu::TextureView,
        size: [u32; 2],
        history_reset: bool,
        mut timing: Option<&mut super::pack_timing::PassTiming>,
    ) {
        self.prepare(device, world, size, history_reset);
        if let Some(t) = timing.as_deref_mut() {
            t.mark(device, encoder, (world.0, self.format));
        }
        let Some((_, _, groups)) = &self.groups else {
            return;
        };
        for (i, ((label, pipeline), group)) in self.passes.iter().zip(groups).enumerate() {
            let view = match (self.plan[i].writes, &self.targets, &self.named) {
                (Writes::Chain(slot), Some((_, t)), _) => &t[slot],
                (Writes::Target(j), _, Some((_, n))) => &n[j],
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
        // The chain's last picture becomes next frame's `history`, once every pass has read
        // this frame's.
        let before_last = &self.plan[..self.plan.len() - 1];
        if let Some(Writes::Chain(slot)) = before_last
            .iter()
            .rfind(|p| matches!(p.writes, Writes::Chain(_)))
            .map(|p| p.writes)
            && let (Some(history), Some((_, targets))) = (&self.history, &self.targets)
        {
            encoder.copy_texture_to_texture(
                targets[slot].texture().as_image_copy(),
                history.as_image_copy(),
                history.size(),
            );
            self.history_valid = true;
        }
    }
}

/// A target of `size` the post stage's `history` is copied into.
fn history_target(device: &wgpu::Device, label: &str, size: [u32; 2]) -> wgpu::Texture {
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
        format: INTERMEDIATE,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// `wetness` after `dt` seconds of `rain`: halfway there in 30 seconds while it rains
/// harder, in 10 while it dries (the half-lives of the `shaders/` format's `wetness`).
fn wetness(wetness: f32, rain: f32, dt: f32) -> f32 {
    let half_life = if rain > wetness { 30.0 } else { 10.0 };
    rain + (wetness - rain) * 0.5f32.powf(dt / half_life)
}

/// The frame as packs read it (docs/api/shaders-reference.md), from the renderer's `Frame`.
pub fn frame_inputs(
    frame: &Frame,
    size: [u32; 2],
    seconds: f32,
    index: u32,
    shadow: Option<&horizon_shaders::Shadow>,
) -> FrameInputs {
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
        precipitation: pack.biome_precipitation.clamp(0, 2) as u32,
        temperature: pack.temperature,
        downfall: pack.rainfall,
        wetness: pack.rain,
        // Filled by `NativeFrame::write`, which remembers the frames before.
        previous_view_projection: m(Mat4::IDENTITY),
        camera_motion: [0.0; 3],
        delta_time: 0.0,
        eye_light_smooth: [level(pack.eye_light[0]), level(pack.eye_light[1])],
        in_water: f32::from(u8::from(pack.eye_in as u32 == 1)),
        in_lava: f32::from(u8::from(pack.eye_in as u32 == 2)),
        water_plane: frame.water_plane.unwrap_or(0.0),
        has_reflection: u32::from(frame.water_plane.is_some()),
        reflection_resolution: reflection_size(size).map(|s| s as f32),
        mods: pack.mod_params,
        light_direction: match shadow {
            Some(s) => super::pack_uniforms::shadow_light(frame, s),
            None => Vec3::Y,
        }
        .to_array(),
        has_shadow: u32::from(shadow.is_some()),
        shadow_view_proj: m(shadow.map_or(Mat4::IDENTITY, |s| {
            super::pack_uniforms::shadow_culling(frame, s)
        })),
        shadow_distance: shadow.map_or(0.0, |s| s.distance),
        shadow_distortion: shadow.map_or(0.0, |s| {
            super::native_shadow::distortion(s.distance, s.resolution)
        }),
        shadow_resolution: shadow.map_or(0.0, |s| s.resolution as f32),
        shadow_center_texel: shadow.map_or(0.0, |s| {
            let k = super::native_shadow::distortion(s.distance, s.resolution);
            2.0 * s.distance / s.resolution.max(1) as f32 * (1.0 - k)
        }),
    }
}

/// The pack's `[[textures]]` (`check::Image`s), uploaded with samplers as the manifest says.
pub fn pack_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    report: &horizon_shader_api::Report,
) -> Vec<(wgpu::TextureView, wgpu::Sampler)> {
    report
        .textures
        .iter()
        .map(|(t, image)| {
            let texture = wgpu::util::DeviceExt::create_texture_with_data(
                device,
                queue,
                &wgpu::TextureDescriptor {
                    label: Some(&t.id),
                    size: wgpu::Extent3d {
                        width: image.width,
                        height: image.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &image.rgba,
            );
            let filter = if t.linear {
                wgpu::FilterMode::Linear
            } else {
                wgpu::FilterMode::Nearest
            };
            let address = if t.repeat {
                wgpu::AddressMode::Repeat
            } else {
                wgpu::AddressMode::ClampToEdge
            };
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(&t.id),
                address_mode_u: address,
                address_mode_v: address,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            });
            (texture.create_view(&Default::default()), sampler)
        })
        .collect()
}

/// Layout entries of `count` pack textures and their samplers.
pub(super) fn pack_texture_entries(count: usize) -> Vec<wgpu::BindGroupLayoutEntry> {
    use horizon_shader_api::stages::{TEXTURE_BINDING, TEXTURE_SAMPLER_BINDING};
    (0..count as u32)
        .flat_map(|i| {
            [
                wgpu::BindGroupLayoutEntry {
                    binding: TEXTURE_BINDING + i,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: TEXTURE_SAMPLER_BINDING + i,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ]
        })
        .collect()
}

/// Group entries of the pack's textures, as `pack_texture_entries` lays them out.
pub(super) fn pack_texture_group_entries(
    textures: &[(wgpu::TextureView, wgpu::Sampler)],
) -> Vec<wgpu::BindGroupEntry<'_>> {
    use horizon_shader_api::stages::{TEXTURE_BINDING, TEXTURE_SAMPLER_BINDING};
    textures
        .iter()
        .enumerate()
        .flat_map(|(i, (view, sampler))| {
            [
                wgpu::BindGroupEntry {
                    binding: TEXTURE_BINDING + i as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: TEXTURE_SAMPLER_BINDING + i as u32,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_pass_with_an_output_leaves_the_chain_as_it_was() {
        use super::{Writes, plan};
        // Bloom into a target, a pass of the chain, a second target, the picture.
        let p = plan(&[Some(0), None, Some(1), None, None]);
        let writes: Vec<_> = p.iter().map(|p| p.writes).collect();
        assert!(writes[0] == Writes::Target(0));
        assert!(writes[1] == Writes::Chain(0));
        assert!(writes[2] == Writes::Target(1));
        assert!(writes[3] == Writes::Chain(1));
        assert!(writes[4] == Writes::Screen);
        let reads: Vec<_> = p.iter().map(|p| p.reads).collect();
        assert_eq!(reads, [None, None, Some(0), Some(0), Some(1)]);
    }
    #[test]
    fn eased_values_start_at_the_target_and_halve_the_distance() {
        use super::eased;
        assert_eq!(eased(f32::NAN, 0.25, 2.0, 0.0), 0.25);
        assert_eq!(eased(1.0, 0.0, 2.0, 2.0), 0.5);
        assert_eq!(eased(1.0, 0.0, 2.0, 0.0), 1.0);
    }
    use super::*;
    use horizon_shader_api::{MANIFEST, Pack, check};

    #[test]
    fn wetness_rises_slower_than_it_dries() {
        assert_eq!(wetness(0.0, 1.0, 30.0), 0.5);
        assert_eq!(wetness(1.0, 0.0, 10.0), 0.5);
        assert_eq!(wetness(0.25, 0.25, 5.0), 0.25);
    }

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
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
        let frame = NativeFrame::new(&device, None, Vec::new(), Default::default(), None);
        queue.write_buffer(
            &frame.buffer,
            0,
            bytemuck::bytes_of(&FrameInputs::default()),
        );
        let mut post = NativePost::new(
            &device,
            report.stages,
            &report.manifest,
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
        post.run(
            &device,
            &mut encoder,
            (&color, &depth, &frame.surface_view),
            &view,
            [4, 4],
            true,
            None,
        );
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

    /// `history` is black on the first frame, then what the pass before the last wrote the
    /// frame before.
    #[test]
    fn the_last_pass_reads_last_frames_input_as_history() {
        let Some((device, queue)) = device() else {
            return;
        };
        let manifest = "schema = 1
id = \"t\"
name = \"T\"
version = \"1\"
api = \"0.1\"
            [[post]]
file = \"a.wgsl\"
[[post]]
file = \"b.wgsl\"
";
        let a = "fn post(input: PostInput) -> vec4<f32> { return vec4<f32>(input.color.rgb * 0.5, 1.0); }
";
        let b = "fn post(input: PostInput) -> vec4<f32> { return textureLoad(history, vec2<i32>(input.pixel), 0); }
";
        let files = [(MANIFEST, manifest), ("a.wgsl", a), ("b.wgsl", b)]
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let frame = NativeFrame::new(&device, None, Vec::new(), Default::default(), None);
        let mut post = NativePost::new(
            &device,
            report.stages,
            &report.manifest,
            &[],
            format,
            &frame,
        )
        .unwrap();
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let color = texture(&device, format, usage).create_view(&Default::default());
        let depth = texture(&device, wgpu::TextureFormat::Depth32Float, usage)
            .create_view(&Default::default());
        let screen = texture(
            &device,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = screen.create_view(&Default::default());
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT * 4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut frame_pixel = |history_reset| {
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            post.run(
                &device,
                &mut encoder,
                (&color, &depth, &frame.surface_view),
                &view,
                [4, 4],
                history_reset,
                None,
            );
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
            buffer.unmap();
            pixel
        };
        assert_eq!(
            frame_pixel(true)[..3],
            [0, 0, 0],
            "black on the first frame"
        );
        let second = frame_pixel(false);
        assert!(
            second[..3].iter().all(|&v| v.abs_diff(128) <= 1),
            "{second:?}"
        );
        assert_eq!(frame_pixel(true)[..3], [0, 0, 0], "black after a frame gap");
        let fourth = frame_pixel(false);
        assert!(
            fourth[..3].iter().all(|&v| v.abs_diff(128) <= 1),
            "{fourth:?}"
        );
    }
}
