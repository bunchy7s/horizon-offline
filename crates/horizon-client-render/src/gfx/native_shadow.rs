//! The shadow map of Horizon's own shader packs (`[shadow]`, docs/shaders.md): each frame the
//! solid terrain and entities the light (the sun by day, the moon by night) sees are drawn into a square depth
//! map, which the material and post stages read as `shadow_map`; translucent terrain goes over a
//! copy of it, `shadow_map_translucent`, with its colour in `shadow_color`. The shadow camera is the one
//! `shaders/` packs use (`pack_uniforms::shadow_camera`), so the renderer's shadow visibility and
//! draw lists (`ShadowView`) serve both; the map's texels are distorted toward the camera as the
//! prelude's `shadow_position` undoes.

use horizon_shader_api::{Frame as FrameInputs, manifest::Shadow as Settings, stages::GROUP};
use horizon_shaders::Shadow;

use super::native_material::TerrainShape;
use super::wgsl_prefix::{TERRAIN, prefixed, section};

const PREFIX: &str = "horizon_terrain_";

/// The shadow pass's entries: terrain's vertices through the shadow camera, distorted, and the
/// cutout test (leaves' and plants' holes let the light through).
const ENTRIES: &str = "
struct HorizonShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) tint: vec3<f32>,
    @location(3) @interpolate(flat) fluid: u32,
}
// `color` is the vertex colour: the tint, and the fluid flag in its alpha's high nibble
// (mesher.rs's `fluid_face`).
fn horizon_shadow_vertex(pos: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>)
    -> HorizonShadowOut {
    let clip = frame.shadow_view_proj * vec4(pos, 1.0);
    let xy = shadow_distort(clip.xy);
    var out: HorizonShadowOut;
    out.clip = vec4(xy, clamp(clip.z, 0.0, 1.0), 1.0);
    out.uv = uv;
    out.layer = layer;
    out.tint = color.rgb;
    let high = u32(round(color.a * 255.0)) >> 4u;
    out.fluid = u32(high >= 8u && (high & 7u) < 6u);
    return out;
}
@vertex fn horizon_shadow_vs(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> HorizonShadowOut {
    let atlas = horizon_terrain_terrain_uv(uv, origin.w);
    let pos = horizon_material_displace(position + origin.xyz, atlas, layer, color, light);
    return horizon_shadow_vertex(pos, atlas, layer, color);
}
@vertex fn horizon_shadow_vs_flat(
    @builtin(vertex_index) index: u32,
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>, @location(2) layer: u32,
    @location(3) color: vec4<f32>, @location(4) light: vec2<u32>
) -> HorizonShadowOut {
    let origin = horizon_terrain_slot_origins[horizon_terrain_block_slots[index >> 8u]];
    let atlas = horizon_terrain_terrain_uv(uv, origin.w);
    let pos = horizon_material_displace(position + origin.xyz, atlas, layer, color, light);
    return horizon_shadow_vertex(pos, atlas, layer, color);
}
@fragment fn horizon_shadow_fs(in: HorizonShadowOut) {
    // Bit 15 of the layer marks opaque leaves (forceOpaque): no alpha cutout.
    let alpha = horizon_terrain_sprite(in.uv, in.layer & 0x7fffu).a;
    if alpha < 0.5 && (in.layer & 0x8000u) == 0u && !horizon_shadow_solid(in.layer & 0x7fffu) { discard; }
}
// Translucent terrain: its colour where it is the nearest thing the light passes through;
// alpha 1 marks a fluid, other surfaces keep their alpha below it.
@fragment fn horizon_shadow_fs_translucent(in: HorizonShadowOut) -> @location(0) vec4<f32> {
    let color = horizon_terrain_sprite(in.uv, in.layer & 0x7fffu);
    if color.a < 0.01 { discard; }
    return vec4(color.rgb * in.tint, select(min(color.a, 0.99), 1.0, in.fluid != 0u));
}
";

/// The entity draw of the shadow pass: the renderer's opaque entity quads (`EntityBatch`'s first
/// material) through the shadow camera, with their texture's cutout. The sky's bodies, clouds,
/// beacon beams and breaking cracks share that buffer but are unlit (entity.wgsl's overlays) and
/// cast nothing. A quad's layer holds the overlay in its low 16 bits.
const ENTITY_ENTRIES: &str = "
@group(0) @binding(1) var horizon_atlas: texture_2d<f32>;
@group(0) @binding(2) var horizon_atlas_sampler: sampler;
struct HorizonEntityShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
}
@vertex fn horizon_entity_shadow_vs(@location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>, @location(2) layer: u32, @location(3) color: vec4<f32>)
    -> HorizonEntityShadowOut {
    let clip = frame.shadow_view_proj * vec4(position, 1.0);
    let xy = shadow_distort(clip.xy);
    let overlay = layer & 0xffffu;
    // As entity.wgsl's `unlit`.
    let unlit = overlay == 2u || overlay == 4u || overlay == 6u
        || (overlay >= 9u && overlay <= 12u);
    var out: HorizonEntityShadowOut;
    // Unlit geometry lands outside the clip volume.
    out.clip = select(vec4(xy, clamp(clip.z, 0.0, 1.0), 1.0), vec4(2.0, 2.0, 2.0, 1.0), unlit);
    out.uv = uv;
    out.alpha = color.a;
    return out;
}
@fragment fn horizon_entity_shadow_fs(in: HorizonEntityShadowOut) {
    let alpha = textureSampleLevel(horizon_atlas, horizon_atlas_sampler, in.uv, 0.0).a;
    if alpha * in.alpha < 0.5 { discard; }
}
";

/// `shadow_color`'s format.
const SHADOW_COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The shader pack's shadow camera for `settings`: orthographic over `distance` each side,
/// staying put while the camera walks a `SNAP`-block cell. The map is distorted about its
/// centre, so a centre that moves with the camera, even by a texel, resamples every shadow but
/// the nearest each frame and they shimmer; a still one maps the world to the same texels until
/// the next cell. Its depth starts 256 blocks further toward the light than `shaders/` packs'
/// (the projection's near plane is at `-near`), so the ground over a deep cave still casts its
/// shadow, and ends past the build height and the slant across the map below the camera.
pub fn camera(settings: &Settings) -> Shadow {
    Shadow {
        resolution: settings.resolution,
        distance: settings.distance,
        render_mul: 1.0,
        interval: SNAP,
        euclidean_snap: true,
        near: 256.0,
        far: super::pack_uniforms::SHADOW_EYE + 2.0 * settings.distance + 384.0,
        ..Shadow::default()
    }
}

/// `settings` with the resolution `device` can make.
pub fn fitted(device: &wgpu::Device, settings: &Settings) -> Settings {
    Settings {
        resolution: settings
            .resolution
            .min(device.limits().max_texture_dimension_2d),
        ..settings.clone()
    }
}

/// The cell the shadow camera keeps to, in blocks along the world's axes: the player stays
/// within 7 blocks of the centre (half the cell on each axis), where the map is still over
/// three times as fine as at its edge.
const SNAP: f32 = 8.0;

/// The ground one shadow texel covers right under the camera, in blocks: a 32nd, half a texture
/// pixel, so shadow edges near the player stay as sharp as the blocks casting them.
const CENTRE_TEXEL: f32 = 1.0 / 32.0;

/// How strongly the map is distorted toward its centre (`Frame::shadow_distortion`).
/// `xy / (|xy| k + 1 - k)` magnifies the centre by 1 / (1 - k); an undistorted texel covers
/// 2 distance / resolution blocks, so k is what brings that down to `CENTRE_TEXEL`. None when
/// the map is already that fine; at most 0.95, past which the edge of the map gets too few texels.
pub fn distortion(distance: f32, resolution: u32) -> f32 {
    let texel = 2.0 * distance / resolution.max(1) as f32;
    (1.0 - CENTRE_TEXEL / texel).clamp(0.0, 0.95)
}

/// The shadow map: a square depth map of the pack's resolution, or a 1×1 one (cleared to "far",
/// so everything is lit) for a pack without `[shadow]`; the same with translucent terrain drawn
/// in, and that terrain's colour.
pub struct ShadowMap {
    depth: wgpu::Texture,
    pub view: wgpu::TextureView,
    translucent: wgpu::Texture,
    pub translucent_view: wgpu::TextureView,
    pub color_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// The 1×1 map still holds the zeroes it was made with.
    blank_uncleared: bool,
}

impl ShadowMap {
    pub fn new(device: &wgpu::Device, settings: Option<&Settings>) -> Self {
        let size = settings.map_or(1, |s| s.resolution);
        let texture = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | usage,
                view_formats: &[],
            })
        };
        let depth = texture(
            "Native shader pack shadow map",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::COPY_SRC,
        );
        let translucent = texture(
            "Native shader pack translucent shadow map",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::COPY_DST,
        );
        let color = texture(
            "Native shader pack shadow colour",
            SHADOW_COLOR,
            wgpu::TextureUsages::empty(),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Native shader pack shadow comparison"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            view: depth.create_view(&Default::default()),
            depth,
            translucent_view: translucent.create_view(&Default::default()),
            translucent,
            color_view: color.create_view(&Default::default()),
            sampler,
            blank_uncleared: settings.is_none(),
        }
    }

    /// A depth pass into the map that clears it to "far".
    pub fn begin<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        label: &str,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) -> wgpu::RenderPass<'e> {
        self.depth_pass(encoder, label, wgpu::LoadOp::Clear(1.0), timing)
    }

    /// A depth pass that adds to the map. Entities draw here, after the translucent map was
    /// copied: where one casts, the solid map is the darker and decides.
    pub fn begin_entities<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        label: &str,
    ) -> wgpu::RenderPass<'e> {
        self.depth_pass(encoder, label, wgpu::LoadOp::Load, None)
    }

    fn depth_pass<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        label: &str,
        load: wgpu::LoadOp<f32>,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) -> wgpu::RenderPass<'e> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            timestamp_writes: timing.and_then(|t| t.writes(label)),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.view,
                depth_ops: Some(wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        })
    }

    /// The translucent pass: the solid terrain's map copied into the translucent one, which it
    /// goes on drawing into, and the colour cleared to white (nothing to pass through).
    pub fn begin_translucent<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        label: &str,
        timing: Option<&mut super::pack_timing::PassTiming>,
    ) -> wgpu::RenderPass<'e> {
        encoder.copy_texture_to_texture(
            self.depth.as_image_copy(),
            self.translucent.as_image_copy(),
            self.depth.size(),
        );
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            timestamp_writes: timing.and_then(|t| t.writes(label)),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.translucent_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        })
    }

    /// Clears the 1×1 map of a pack without `[shadow]` once.
    pub fn clear_blank(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if std::mem::take(&mut self.blank_uncleared) {
            drop(self.begin(encoder, "Native shader pack shadow map clear", None));
            drop(self.begin_translucent(encoder, "Native shader pack shadow colour clear", None));
        }
    }
}

/// The pipelines that draw terrain into the shadow map, for the renderer's instanced draws and,
/// where they run, its flat ones, and the frame they read (group 3, the uniform alone: the
/// stages' own group 3 also holds the map being drawn).
pub struct NativeShadow {
    pub solid: wgpu::RenderPipeline,
    pub flat: Option<wgpu::RenderPipeline>,
    /// The translucent terrain's draws into the translucent map and the colour, instanced and
    /// flat.
    pub translucent: wgpu::RenderPipeline,
    pub flat_translucent: Option<wgpu::RenderPipeline>,
    /// The entities' draw, with the entities' group 0 (`ENTITY_ENTRIES`).
    pub entity: wgpu::RenderPipeline,
    pub group: wgpu::BindGroup,
}

/// The entity draw's shader.
fn entity_source() -> String {
    format!(
        "{}@group({GROUP}) @binding(0) var<uniform> frame: Frame;\n{}{ENTITY_ENTRIES}",
        FrameInputs::WGSL.wgsl(),
        horizon_shader_api::stages::SHADOW_HELPERS,
    )
}

/// The shadow pass's entity pipeline: the entities' vertices and group 0, the frame at group 3.
fn entity_pipeline(
    device: &wgpu::Device,
    entities: &wgpu::BindGroupLayout,
    frame_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Native shader pack entity shadow"),
        source: wgpu::ShaderSource::Wgsl(entity_source().into()),
    });
    let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] = Default::default();
    groups[0] = Some(entities);
    groups[GROUP as usize] = Some(frame_layout);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Native shader pack entity shadow"),
        bind_group_layouts: &groups,
        ..Default::default()
    });
    let attributes =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Uint32, 3 => Unorm8x4];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Native shader pack entity shadow"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("horizon_entity_shadow_vs"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<super::Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        // Both sides, as the terrain: thin parts (ears, item quads) cast from either.
        primitive: wgpu::PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: wgpu::DepthBiasState {
                constant: 0,
                slope_scale: 1.5,
                clamp: 0.0,
            },
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("horizon_entity_shadow_fs"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// `horizon_shadow_solid(layer)`: whether the sprite layer's id, out of `materials`
/// (`(layer, id)`), is one of the pack's `solid` ones.
fn shadow_solid(materials: &[(u32, u32)], solid: &[u32]) -> String {
    let layers: Vec<String> = materials
        .iter()
        .filter(|(_, id)| solid.contains(id))
        .map(|(layer, _)| format!("{layer}u"))
        .collect();
    let cases = if layers.is_empty() {
        String::new()
    } else {
        format!("        case {}: {{ return true; }}\n", layers.join(", "))
    };
    format!(
        "fn horizon_shadow_solid(layer: u32) -> bool {{\n    switch layer {{\n{cases}        \
         default: {{ return false; }}\n    }}\n}}\n"
    )
}

/// The shadow pass's shader: the frame, Horizon's terrain, and the entries, with the layers
/// `solid` of `shadow_solid`.
fn shader_source(solid: &str, vertex: Option<&str>, materials: &[(u32, u32)]) -> String {
    let (head, displace) = match vertex {
        Some(stage) => (
            stage.to_owned()
                + &super::native_material::layer_table("horizon_sprite_material", materials),
            super::native_material::DISPLACE_PACK,
        ),
        None => (
            format!(
                "{}@group({GROUP}) @binding(0) var<uniform> frame: Frame;\n{}",
                FrameInputs::WGSL.wgsl(),
                horizon_shader_api::stages::SHADOW_HELPERS,
            ),
            super::native_material::DISPLACE_VANILLA,
        ),
    };
    format!(
        "{head}// Horizon's terrain (terrain.wgsl)\n{}{solid}{displace}{ENTRIES}",
        prefixed(
            section(TERRAIN, "struct Globals {", "// Sky (SkyRenderer)"),
            PREFIX
        )
    )
}

impl NativeShadow {
    pub fn new(
        device: &wgpu::Device,
        shape: &TerrainShape<'_>,
        frame: &super::native_post::NativeFrame,
        solid_materials: &[u32],
        vertex: Option<&horizon_shader_api::check::CheckedStage>,
        constants: &[(String, f64)],
    ) -> Result<Self, String> {
        let text = shader_source(
            &shadow_solid(shape.sprite_materials, solid_materials),
            vertex.map(|s| s.source.as_str()),
            shape.sprite_materials,
        );
        let module = naga::front::wgsl::parse_str(&text)
            .map_err(|e| format!("shadow pass: {}", e.emit_to_string(&text)))?;
        let mut constants = super::native_post::present_constants(&module, constants);
        constants.extend([
            ("horizon_terrain_SPRITE_ARRAYS", shape.sprites[0]),
            ("horizon_terrain_SPRITE_SPLIT", shape.sprites[1]),
        ]);
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Native shader pack shadow"),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Native shader pack shadow frame"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<FrameInputs>() as u64),
                },
                count: None,
            }],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Native shader pack shadow frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.buffer.as_entire_binding(),
            }],
        });
        let layout = |flat: Option<&wgpu::BindGroupLayout>| {
            let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] =
                Default::default();
            groups[0] = Some(shape.globals);
            groups[1] = flat;
            groups[GROUP as usize] = Some(&frame_layout);
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Native shader pack shadow"),
                bind_group_layouts: &groups,
                ..Default::default()
            })
        };
        let instance_attributes = wgpu::vertex_attr_array![5=>Float32x4];
        let vertices = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<super::TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &super::TERRAIN_ATTRIBUTES,
        };
        let instances = wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance_attributes,
        };
        let color_target = [Some(wgpu::ColorTargetState {
            format: SHADOW_COLOR,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = |layout: &wgpu::PipelineLayout,
                        vertex: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        translucent: bool| {
            let options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Native shader pack shadow"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vertex),
                    compilation_options: options.clone(),
                    buffers,
                },
                // Both sides: the quads that face the light are drawn (`facing_runs`), and
                // plants' crossed quads cast from either side.
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 0,
                        slope_scale: 1.5,
                        clamp: 0.0,
                    },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(if translucent {
                        "horizon_shadow_fs_translucent"
                    } else {
                        "horizon_shadow_fs"
                    }),
                    compilation_options: options,
                    targets: if translucent { &color_target } else { &[] },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let instanced = layout(None);
        let solid = pipeline(
            &instanced,
            "horizon_shadow_vs",
            &[Some(vertices.clone()), Some(instances.clone())],
            false,
        );
        let translucent = pipeline(
            &instanced,
            "horizon_shadow_vs",
            &[Some(vertices.clone()), Some(instances)],
            true,
        );
        let flat_layout = shape.flat.map(|group| layout(Some(group)));
        let flat_pipeline = |translucent| {
            flat_layout.as_ref().map(|layout| {
                pipeline(
                    layout,
                    "horizon_shadow_vs_flat",
                    &[Some(vertices.clone())],
                    translucent,
                )
            })
        };
        let flat = flat_pipeline(false);
        let flat_translucent = flat_pipeline(true);
        let entity = entity_pipeline(device, shape.entities, &frame_layout);
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("shadow pass: {e}"));
        }
        Ok(Self {
            solid,
            flat,
            translucent,
            flat_translucent,
            entity,
            group,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shadow_shader_validates() {
        for materials in [&[][..], &[(3, 7), (4, 8), (9, 7)][..]] {
            let text = shader_source(&shadow_solid(materials, &[7]), None, materials);
            let module = naga::front::wgsl::parse_str(&text).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&text)));
        }
    }

    #[test]
    fn only_the_layers_of_solid_ids_lose_their_cutouts() {
        let materials = [(3, 7), (4, 8), (9, 7)];
        assert!(shadow_solid(&materials, &[7]).contains("case 3u, 9u: { return true; }"));
        assert!(!shadow_solid(&materials, &[]).contains("case"));
        assert!(!shadow_solid(&materials, &[5]).contains("case"));
    }

    #[test]
    fn the_entity_shader_validates() {
        let text = entity_source();
        let module = naga::front::wgsl::parse_str(&text).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&text)));
    }

    #[test]
    fn what_the_camera_stands_on_is_inside_the_shadow_cameras_depth() {
        let settings = Settings {
            resolution: 2048,
            distance: 128.0,
            solid_materials: Vec::new(),
        };
        let shadow = camera(&settings);
        let (view, projection) = super::super::pack_uniforms::shadow_camera(
            glam::Vec3::new(0.3, 0.9, 0.0).normalize(),
            glam::DVec3::new(-2064.5, 246.0, -4304.5),
            &shadow,
        );
        // Up to the build height's span below a player flying high, as well as above them.
        for y in [64.0, 0.0, -64.0, -200.0, -384.0] {
            let clip = projection * view * glam::Vec4::new(0.0, y, 0.0, 1.0);
            assert!(clip.is_finite(), "{clip}");
            assert!((-1.0..=1.0).contains(&clip.z), "{clip}");
        }
    }

    #[test]
    fn the_centre_stays_near_the_player_on_both_sides_of_zero() {
        let shadow = camera(&Settings {
            resolution: 2048,
            distance: 128.0,
            solid_materials: Vec::new(),
        });
        let light = glam::Vec3::Y;
        for x in [5.5, -5.5, -2064.5, 2064.5] {
            let camera = glam::DVec3::new(x, 64.0, x);
            let (view, _) = super::super::pack_uniforms::shadow_camera(light, camera, &shadow);
            let centre = view.inverse().w_axis.truncate();
            assert!(centre.x.abs() <= SNAP / 2.0, "{x}: {centre}");
            assert!(centre.z.abs() <= SNAP / 2.0, "{x}: {centre}");
        }
    }

    #[test]
    fn the_distortion_brings_the_centre_texel_to_a_32nd_of_a_block() {
        // 2048 texels over 256 blocks: an 8th of a block each, magnified 4 times.
        assert!((distortion(128.0, 2048) - 0.75).abs() < 1e-6);
        // Already that fine: no distortion.
        assert_eq!(distortion(32.0, 2048), 0.0);
        assert_eq!(distortion(10_000.0, 1024), 0.95);
    }
}
