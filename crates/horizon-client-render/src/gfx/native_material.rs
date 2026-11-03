//! The material stage of Horizon's own shader packs (docs/shaders.md): the pack's `material`
//! function gives the lit colour of each point of a block's surface, before the alpha test and
//! fog. Terrain still draws with Horizon's meshes, culling and draw lists; only its pipelines
//! change. Their shader is the pack's stage (`CheckedStage::source`), then a copy of
//! terrain.wgsl's terrain section whose names all take a `horizon_terrain_` prefix
//! (`wgsl_prefix.rs`), then entries that hand the pack what vanilla's `fs_solid` and
//! `fs_translucent` combine: the texture, the vertex tint, the light levels and the lightmap's
//! colour. They read the terrain's group 0, the flat draws' group 1 where those run, and the
//! pack's frame at group 3.

use horizon_shader_api::{check::CheckedStage, stages::GROUP};

use super::native_post::{NativeFrame, present_constants};
use super::wgsl_prefix::{TERRAIN, prefixed, section};

const PREFIX: &str = "horizon_terrain_";

/// The entries around the pack's `material`: vanilla's vertex work without the lightmap
/// product, then the material, the alpha test and fog as `fs_solid` and `fs_translucent` do.
const ENTRIES: &str = "
// The manifest's `[material] fog`: false when the pack fogs its surfaces itself.
override horizon_material_fog: bool = true;
fn horizon_material_fogged(color: vec4<f32>, spherical: f32, cylindrical: f32) -> vec4<f32> {
    if horizon_material_fog {
        return horizon_terrain_apply_fog(color, spherical, cylindrical);
    }
    return color;
}
struct HorizonMaterialOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) tint: vec4<f32>,
    @location(3) light: vec2<f32>,
    @location(4) lightmap: vec3<f32>,
    @location(5) position: vec3<f32>,
    @location(6) spherical: f32,
    @location(7) cylindrical: f32,
    @location(8) @interpolate(flat) face: u32,
    @location(9) @interpolate(flat) fluid_quad: u32,
    @location(10) @interpolate(flat) emission: f32,
    // terrain.wgsl's `emission`, for HDR output's mark.
    @location(11) @interpolate(flat) hdr_code: u32,
}
fn horizon_material_vertex(position: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>, origin: vec3<f32>) -> HorizonMaterialOut {
    let pos = horizon_material_displace(position + origin, uv, layer, color, light);
    var out: HorizonMaterialOut;
    out.clip = horizon_terrain_globals.view_proj * vec4(pos, 1.0);
    out.uv = uv;
    out.layer = layer;
    out.tint = vec4(color.rgb, 1.0);
    let block = max(f32(light.y), horizon_terrain_dynamic_block_light(pos));
    let sky = f32(light.x);
    // 16 per level, 15 levels.
    out.light = min(vec2(block, sky) / 240.0, vec2(1.0));
    out.lightmap = horizon_terrain_sample_lightmap(sky / 16.0, block / 16.0).rgb;
    out.position = pos;
    out.spherical = length(pos);
    out.cylindrical = max(length(pos.xz), abs(pos.y));
    // The colour's alpha byte (mesher.rs's `fluid_face` and `emitting`): a fluid quad's
    // high nibble is 8 + its face (0..5), every other quad keeps 0xF there; the low nibble
    // is 15 minus the face's HDR tier. Every corner of a quad carries the same byte, so the
    // interpolated value rounds back exactly.
    let flags = u32(round(color.a * 255.0));
    let high = flags >> 4u;
    let fluid = high >= 8u && (high & 7u) < 6u;
    out.face = select(0u, high & 7u, fluid);
    out.fluid_quad = u32(fluid);
    // The tier (0 none, 1 low, 2 incandescent, 3 bright, 4 very bright) as how strongly the
    // face shines: a redstone torch 7/15, a torch 14/15, glowstone 1.
    out.hdr_code = 15u - (flags & 15u);
    // Codes above 4 are the same tiers, shining all over rather than in their bright parts.
    let tier = select(out.hdr_code, out.hdr_code - 4u, out.hdr_code > 4u);
    out.emission = select(min(f32(tier) * 7.0 / 15.0, 1.0), 0.0, tier == 0u);
    return out;
}
@vertex fn horizon_material_vs(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>,
    @location(5) origin: vec4<f32>) -> HorizonMaterialOut {
    return horizon_material_vertex(position, horizon_terrain_terrain_uv(uv, origin.w), layer,
        color, light, origin.xyz);
}
@vertex fn horizon_material_vs_flat(
    @builtin(vertex_index) vertex: u32,
    @location(0) position: vec3<f32>, @location(1) uv: vec2<u32>,
    @location(2) layer: u32, @location(3) color: vec4<f32>, @location(4) light: vec2<u32>
) -> HorizonMaterialOut {
    let origin = horizon_terrain_slot_origins[horizon_terrain_block_slots[vertex >> 8u]];
    return horizon_material_vertex(position, horizon_terrain_terrain_uv(uv, origin.w), layer,
        color, light, origin.xyz);
}
// The prelude's `block_normal_map` and `block_specular_map`: the resource packs' LabPBR maps
// in the pack's frame group (3), one array each, a blank 1x1 without them.
@group(3) @binding(12) var horizon_pbr_normals: texture_2d_array<f32>;
@group(3) @binding(13) var horizon_pbr_specular: texture_2d_array<f32>;
fn horizon_block_pbr_present() -> bool {
    return textureDimensions(horizon_pbr_normals).x > 1u;
}
fn horizon_block_pbr_size() -> vec2<f32> {
    return vec2<f32>(textureDimensions(horizon_pbr_normals).xy);
}
fn horizon_block_pbr(uv: vec2<f32>, layer: u32, lod: f32, map: u32) -> vec4<f32> {
    if !horizon_block_pbr_present() || layer >= textureNumLayers(horizon_pbr_normals) {
        return select(vec4<f32>(0.5, 0.5, 1.0, 1.0), vec4<f32>(0.0), map == 1u);
    }
    let s = horizon_terrain_sprite_sampler;
    // The maps may be smaller than the colour: the same footprint is fewer of their texels.
    let scale = f32(textureDimensions(horizon_terrain_sprites).x)
        / f32(textureDimensions(horizon_pbr_normals).x);
    let l = max(lod - log2(scale), 0.0);
    if map == 0u {
        return textureSampleLevel(horizon_pbr_normals, s, uv, i32(layer), l);
    }
    return textureSampleLevel(horizon_pbr_specular, s, uv, i32(layer), l);
}
// The prelude's `block_texture`: a sprite of the terrain's arrays at a mip level.
fn horizon_block_size() -> vec2<f32> {
    return vec2<f32>(textureDimensions(horizon_terrain_sprites).xy);
}
fn horizon_block_sample(uv: vec2<f32>, layer: u32, lod: f32) -> vec4<f32> {
    let s = horizon_terrain_sprite_sampler;
    let l = i32(layer % horizon_terrain_SPRITE_SPLIT);
    switch layer / horizon_terrain_SPRITE_SPLIT {
        case 0u: { return textureSampleLevel(horizon_terrain_sprites, s, uv, l, lod); }
        case 1u: { return textureSampleLevel(horizon_terrain_sprites_1, s, uv, l, lod); }
        case 2u: { return textureSampleLevel(horizon_terrain_sprites_2, s, uv, l, lod); }
        case 3u: { return textureSampleLevel(horizon_terrain_sprites_3, s, uv, l, lod); }
        case 4u: { return textureSampleLevel(horizon_terrain_sprites_4, s, uv, l, lod); }
        case 5u: { return textureSampleLevel(horizon_terrain_sprites_5, s, uv, l, lod); }
        case 6u: { return textureSampleLevel(horizon_terrain_sprites_6, s, uv, l, lod); }
        default: { return textureSampleLevel(horizon_terrain_sprites_7, s, uv, l, lod); }
    }
}
fn horizon_material_color(in: HorizonMaterialOut, translucent: bool) -> vec4<f32> {
    let albedo = horizon_terrain_sprite(in.uv, in.layer & 0x7fffu);
    horizon_block_layer = in.layer & 0x7fffu;
    // The mip level of this point in the texture: its texels, as the sampler would pick.
    let texels = in.uv * horizon_block_size();
    let footprint = max(dot(dpdx(texels), dpdx(texels)), dot(dpdy(texels), dpdy(texels)));
    horizon_block_lod = max(0.5 * log2(max(footprint, 1e-8)), 0.0);
    // Water draws translucent and lava solid, so the pass resolves the fluid kind.
    let fluid = select(0u, select(2u, 1u, translucent), in.fluid_quad != 0u);
    let normal = horizon_material_normal(in.position);
    horizon_material_face_normal = normal;
    horizon_surface = vec4<f32>(0.0);
    let tangent = horizon_material_tangent(in.position, in.uv, normal);
    return material(MaterialInput(albedo, in.tint, in.light, in.lightmap, in.position, in.uv,
        translucent, horizon_sprite_tags(in.layer & 0x7fffu),
        horizon_sprite_material(in.layer & 0x7fffu), in.clip.xy / frame.resolution,
        in.face, fluid, in.emission, normal, tangent,
        horizon_terrain_native_colored_light(in.position))).color;
}
var<private> horizon_material_face_normal: vec3<f32>;
struct HorizonMaterialSolid {
    @location(0) color: vec4<f32>,
    // The surface target's data: `set_surface`'s, then the fragment's depth, which the post
    // stage's `surface_at` compares with the final depth.
    @location(1) surface: vec4<f32>,
}
// What `fs_solid` leaves in the frame's alpha for HDR output, from the sprite before the pack's
// material: the pack decides the colour, Horizon still knows which pixels are light sources.
fn horizon_material_mark(in: HorizonMaterialOut, texel: vec4<f32>) -> f32 {
    if in.hdr_code > 0u {
        let whole = in.hdr_code > 4u;
        let tier = select(in.hdr_code, in.hdr_code - 4u, whole);
        let l = in.layer & 0x7fffu;
        let t = textureLoad(horizon_terrain_emission_thresholds,
            vec2(i32(l % horizon_terrain_SPRITE_SPLIT), i32(l / horizon_terrain_SPRITE_SPLIT)), 0);
        let y = horizon_terrain_brightness(texel.rgb);
        if whole || y >= t.r {
            return horizon_terrain_hdr_mark(tier, y);
        }
    }
    return 1.0;
}
@fragment fn horizon_material_fs_solid(in: HorizonMaterialOut) -> HorizonMaterialSolid {
    let color = horizon_material_color(in, false);
    let texel = horizon_terrain_sprite(in.uv, in.layer & 0x7fffu);
    // Bit 15 of the layer marks opaque leaves (forceOpaque): no alpha cutout.
    if color.a < 0.5 && (in.layer & 0x8000u) == 0u { discard; }
    if horizon_surface.w == 0.0 {
        set_surface(horizon_material_face_normal, 0.0, 0.0);
    }
    var out: HorizonMaterialSolid;
    let mark = horizon_material_mark(in, texel);
    out.color = horizon_material_fogged(vec4(color.rgb, mark), in.spherical, in.cylindrical);
    out.surface = vec4(horizon_surface.xyz, in.clip.z);
    return out;
}
@fragment fn horizon_material_fs_translucent(in: HorizonMaterialOut) -> @location(0) vec4<f32> {
    let color = horizon_material_color(in, true);
    if color.a < 0.01 { discard; }
    return horizon_material_fogged(color, in.spherical, in.cylindrical);
}
";

/// What terrain pipelines are built against: the renderer's layouts and constants.
pub struct TerrainShape<'a> {
    /// The terrain's group 0.
    pub globals: &'a wgpu::BindGroupLayout,
    /// The flat draws' group 1, where they run (`flat.rs`).
    pub flat: Option<&'a wgpu::BindGroupLayout>,
    /// terrain.wgsl's `SPRITE_ARRAYS` and `SPRITE_SPLIT`.
    pub sprites: [f64; 2],
    /// The world target's colour format.
    pub format: wgpu::TextureFormat,
    /// The pack's tags of each tagged sprite layer, as `(layer, bits)`
    /// (`Manifest::sprite_tags`).
    pub sprite_tags: &'a [(u32, u32)],
    /// The pack's material id of each sprite layer with one, as `(layer, id)`
    /// (`Manifest::sprite_materials`).
    pub sprite_materials: &'a [(u32, u32)],
    /// The entities' group 0 (their atlas), for the shadow pass's entity draw.
    pub entities: &'a wgpu::BindGroupLayout,
    /// The weather's groups 0 and 1 (the entities' atlas, the fall clock), for the weather
    /// stage.
    pub weather_groups: [&'a wgpu::BindGroupLayout; 2],
    /// The resource packs' LabPBR maps (normals, specular), when they ship any.
    pub pbr: Option<&'a [wgpu::Texture; 2]>,
}

/// The pack's terrain pipelines: solid (with the cutout test) and translucent, for the
/// renderer's instanced draws and, where they run, its flat ones.
pub struct NativeMaterial {
    pub solid: wgpu::RenderPipeline,
    pub translucent: wgpu::RenderPipeline,
    pub flat: Option<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
    /// The stage calls `set_surface`: its solid pipelines also write the surface target.
    pub writes_surface: bool,
}

/// terrain.wgsl's terrain section (its globals, bindings, sampling, lighting and fog), with
/// every name it declares prefixed.
fn vanilla_terrain() -> String {
    prefixed(
        section(TERRAIN, "struct Globals {", "// Sky (SkyRenderer)"),
        PREFIX,
    )
}

/// The function `name(layer)`, the value of each sprite layer in `table` and 0 for the rest:
/// `horizon_sprite_tags` (`MaterialInput.tags`) and `horizon_sprite_material`
/// (`MaterialInput.material`).
pub(super) fn layer_table(name: &str, table: &[(u32, u32)]) -> String {
    let mut out = format!("fn {name}(layer: u32) -> u32 {{\n    switch layer {{\n");
    let mut table = table.to_vec();
    table.sort_unstable_by_key(|&(layer, value)| (value, layer));
    for group in table.chunk_by(|a, b| a.1 == b.1) {
        let layers: Vec<String> = group.iter().map(|(layer, _)| format!("{layer}u")).collect();
        out += &format!(
            "        case {}: {{ return {}u; }}\n",
            layers.join(", "),
            group[0].1
        );
    }
    out + "        default: { return 0u; }\n    }\n}\n"
}

/// Where a corner of terrain is drawn without a vertex stage.
pub(super) const DISPLACE_VANILLA: &str = "
fn horizon_material_displace(pos: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>) -> vec3<f32> {
    return pos;
}
";

/// The vertex stage's call, with its inputs from terrain's vertex.
pub(super) const DISPLACE_PACK: &str = "
fn horizon_material_displace(pos: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>) -> vec3<f32> {
    let flags = u32(round(color.a * 255.0));
    let high = flags >> 4u;
    let fluid = high >= 8u && (high & 7u) < 6u;
    let lit = min(vec2(f32(light.y), f32(light.x)) / 240.0, vec2(1.0));
    return vertex(VertexInput(pos, uv, horizon_sprite_material(layer & 0x7fffu), lit,
        vec4(color.rgb, 1.0), select(0u, high & 7u, fluid), u32(fluid)));
}
";

/// The surface target (the post stage's `scene_surface`): x the normal (two snorm16 in its
/// bits), y smoothness, z the pack's data, w the fragment's depth.
pub const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// The terrain shader for the pack's material stage `source`, with its sprite `tags` and
/// `materials`, and the vertex stage in its module when `vertex`.
fn shader_source(
    source: &str,
    tags: &[(u32, u32)],
    materials: &[(u32, u32)],
    vertex: bool,
) -> String {
    format!(
        "{source}\n// Horizon's terrain (terrain.wgsl)\n{}{}{}{}{ENTRIES}",
        vanilla_terrain(),
        if vertex {
            DISPLACE_PACK
        } else {
            DISPLACE_VANILLA
        },
        layer_table("horizon_sprite_tags", tags),
        layer_table("horizon_sprite_material", materials)
    )
}

/// The material's naga module, its errors in the stage's file.
fn module(
    stage: &CheckedStage,
    tags: &[(u32, u32)],
    materials: &[(u32, u32)],
    vertex: bool,
) -> Result<naga::Module, String> {
    let text = shader_source(&stage.source, tags, materials, vertex);
    naga::front::wgsl::parse_str(&text)
        .map_err(|e| format!("{}: {}", stage.file, e.emit_to_string(&text)))
}

impl NativeMaterial {
    /// The pipelines of the pack's material `stage` with the option values `constants`.
    pub fn new(
        device: &wgpu::Device,
        shape: &TerrainShape<'_>,
        frame: &NativeFrame,
        stage: &CheckedStage,
        constants: &[(String, f64)],
        fog: bool,
        vertex: bool,
    ) -> Result<Self, String> {
        let writes_surface = horizon_shader_api::stages::calls_set_surface(&stage.source);
        let module = module(stage, shape.sprite_tags, shape.sprite_materials, vertex)?;
        let mut constants = present_constants(&module, constants);
        let sprite_names = [
            "horizon_terrain_SPRITE_ARRAYS",
            "horizon_terrain_SPRITE_SPLIT",
        ];
        constants.extend(sprite_names.into_iter().zip(shape.sprites));
        constants.push(("horizon_material_fog", f64::from(u8::from(fog))));
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&stage.label),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let layout = |flat: Option<&wgpu::BindGroupLayout>| {
            let mut groups: [Option<&wgpu::BindGroupLayout>; GROUP as usize + 1] =
                Default::default();
            groups[0] = Some(shape.globals);
            groups[1] = flat;
            groups[GROUP as usize] = Some(&frame.layout);
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Native shader pack terrain"),
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
        // As the renderer's terrain pipelines (gfx.rs): back faces culled, reversed depth.
        let pipeline = |layout: &wgpu::PipelineLayout,
                        vertex: &str,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                        fragment: &str,
                        blend: Option<wgpu::BlendState>| {
            let options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            let solid_targets = [
                Some(wgpu::ColorTargetState {
                    format: shape.format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: SURFACE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&stage.label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vertex),
                    compilation_options: options.clone(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: options,
                    targets: if fragment == "horizon_material_fs_solid" && writes_surface {
                        &solid_targets
                    } else {
                        &solid_targets[..1]
                    },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let instanced = layout(None);
        let buffers = [Some(vertices.clone()), Some(instances)];
        let solid = pipeline(
            &instanced,
            "horizon_material_vs",
            &buffers,
            "horizon_material_fs_solid",
            None,
        );
        let translucent = pipeline(
            &instanced,
            "horizon_material_vs",
            &buffers,
            "horizon_material_fs_translucent",
            blend,
        );
        let flat = shape.flat.map(|group| {
            let layout = layout(Some(group));
            let buffers = [Some(vertices.clone())];
            let vs = "horizon_material_vs_flat";
            (
                pipeline(&layout, vs, &buffers, "horizon_material_fs_solid", None),
                pipeline(
                    &layout,
                    vs,
                    &buffers,
                    "horizon_material_fs_translucent",
                    blend,
                ),
            )
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(format!("{}: {e}", stage.file));
        }
        Ok(Self {
            solid,
            translucent,
            flat,
            writes_surface,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_shader_api::{MANIFEST, Pack, StageKind, check};

    fn stage(material: &str) -> CheckedStage {
        let manifest = "schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.1\"\n\
            [material]\nfile = \"material.wgsl\"\n\
            [[options]]\nid = \"gain\"\nname = \"Gain\"\ntype = \"float\"\n\
            default = 1.0\nmin = 0.0\nmax = 2.0\n";
        let files = [(MANIFEST, manifest), ("material.wgsl", material)]
            .map(|(n, t)| (n.to_owned(), t.as_bytes().to_vec()))
            .into();
        let mut report = check(&Pack::from_files(files).unwrap()).unwrap();
        assert_eq!(report.stages[0].kind, StageKind::Material);
        report.stages.remove(0)
    }

    #[test]
    fn the_terrain_takes_a_prefix_and_validates_with_a_pack() {
        let terrain = vanilla_terrain();
        for name in [
            "Globals",
            "globals",
            "sprites_7",
            "sprite",
            "SPRITE_SPLIT",
            "block_slots",
            "sample_lightmap",
            "apply_fog",
        ] {
            assert!(terrain.contains(&format!("{PREFIX}{name}")), "{name}");
        }
        assert!(!terrain.contains("fs_sky"), "the sky stays out");
        // A pack may use any name Horizon's terrain uses unprefixed.
        let own = "override gain: f32;\nconst PI: f32 = 3.0;\n\
            fn sprite(c: vec4<f32>) -> vec4<f32> { return c * PI; }\n\
            fn apply_fog(c: vec4<f32>) -> vec4<f32> { return c * gain; }\n\
            fn material(input: MaterialInput) -> Surface {\n    \
                let lit = input.albedo * input.tint * vec4(input.lightmap, 1.0);\n    \
                let t = vec2<i32>(input.screen * frame.resolution);\n    \
                let behind = textureLoad(opaque_color, t, 0) * textureLoad(opaque_depth, t, 0);\n    \
                let mirrored = textureSampleLevel(reflection, reflection_sampler, input.screen, 0.0);\n    \
                let kind = f32(input.face + input.fluid) * input.emission * mirrored.a;\n    \
                let frame_of = vec4(cross(input.normal, input.tangent.xyz) * input.tangent.w, kind);\n    \
                return Surface(apply_fog(sprite(lit)) + (behind + frame_of) * 0.0);\n}\n";
        let stage = stage(own);
        let tags = [(7, 1), (3, 1), (9, 2)];
        let materials = [(5, 1000)];
        assert!(layer_table("t", &tags).contains("case 3u, 7u: { return 1u; }"));
        let module = module(&stage, &tags, &materials, false).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| {
            panic!(
                "{}",
                e.emit_to_string(&shader_source(&stage.source, &tags, &materials, false))
            )
        });
        let entries: Vec<&str> = module
            .entry_points
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        for entry in [
            "horizon_material_vs",
            "horizon_material_vs_flat",
            "horizon_material_fs_solid",
            "horizon_material_fs_translucent",
        ] {
            assert!(entries.contains(&entry), "{entry}");
        }
        drop(info);
        assert_eq!(
            present_constants(&module, &[("gain".into(), 1.5), ("other".into(), 1.0)]),
            [("gain", 1.5)]
        );
    }
}
