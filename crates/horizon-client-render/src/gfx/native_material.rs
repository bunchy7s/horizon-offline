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
}
fn horizon_material_vertex(position: vec3<f32>, uv: vec2<f32>, layer: u32, color: vec4<f32>,
    light: vec2<u32>, origin: vec3<f32>) -> HorizonMaterialOut {
    let pos = position + origin;
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
fn horizon_material_color(in: HorizonMaterialOut, translucent: bool) -> vec4<f32> {
    let albedo = horizon_terrain_sprite(in.uv, in.layer & 0x7fffu);
    return material(MaterialInput(albedo, in.tint, in.light, in.lightmap, in.position, in.uv,
        translucent)).color;
}
@fragment fn horizon_material_fs_solid(in: HorizonMaterialOut) -> @location(0) vec4<f32> {
    let color = horizon_material_color(in, false);
    // Bit 15 of the layer marks opaque leaves (forceOpaque): no alpha cutout.
    if color.a < 0.5 && (in.layer & 0x8000u) == 0u { discard; }
    return horizon_terrain_apply_fog(vec4(color.rgb, 1.0), in.spherical, in.cylindrical);
}
@fragment fn horizon_material_fs_translucent(in: HorizonMaterialOut) -> @location(0) vec4<f32> {
    let color = horizon_material_color(in, true);
    if color.a < 0.01 { discard; }
    return horizon_terrain_apply_fog(color, in.spherical, in.cylindrical);
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
}

/// The pack's terrain pipelines: solid (with the cutout test) and translucent, for the
/// renderer's instanced draws and, where they run, its flat ones.
pub struct NativeMaterial {
    pub solid: wgpu::RenderPipeline,
    pub translucent: wgpu::RenderPipeline,
    pub flat: Option<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
}

/// terrain.wgsl's terrain section (its globals, bindings, sampling, lighting and fog), with
/// every name it declares prefixed.
fn vanilla_terrain() -> String {
    prefixed(
        section(TERRAIN, "struct Globals {", "// Sky (SkyRenderer)"),
        PREFIX,
    )
}

/// The terrain shader for the pack's material stage `source`.
fn shader_source(source: &str) -> String {
    format!(
        "{source}\n// Horizon's terrain (terrain.wgsl)\n{}{ENTRIES}",
        vanilla_terrain()
    )
}

/// The material's naga module, its errors in the stage's file.
fn module(stage: &CheckedStage) -> Result<naga::Module, String> {
    let text = shader_source(&stage.source);
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
    ) -> Result<Self, String> {
        let module = module(stage)?;
        let mut constants = present_constants(&module, constants);
        let sprite_names = [
            "horizon_terrain_SPRITE_ARRAYS",
            "horizon_terrain_SPRITE_SPLIT",
        ];
        constants.extend(sprite_names.into_iter().zip(shape.sprites));
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
                    targets: &[Some(wgpu::ColorTargetState {
                        format: shape.format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
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
                return Surface(apply_fog(sprite(lit)));\n}\n";
        let stage = stage(own);
        let module = module(&stage).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&shader_source(&stage.source))));
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
