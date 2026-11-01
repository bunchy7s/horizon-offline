//! The stages a pack can fill, each a WGSL function with a fixed signature
//! that Horizon calls inside its own passes, and the WGSL put around a
//! pack's files to build a stage: the prelude before (the inputs and the
//! resources), the entry points after.
use crate::inputs::{Frame, MATERIAL_INPUT, POST_INPUT, SKY_INPUT, SURFACE, Struct, WEATHER_INPUT};
use crate::package::Pack;

/// The bind group of a pack's resources; groups 0 to 2 are the renderer's.
pub const GROUP: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    /// The sky's colour along each view ray, before the sun, moon and stars.
    Sky,
    /// The colour of block surfaces (terrain), before the alpha test and fog.
    Material,
    /// The colour of rain and snow streaks.
    Weather,
    /// A full-screen pass over the world's picture, under the hand and HUD.
    Post,
}

/// A resource a stage reads, declared by its prelude.
#[derive(Debug)]
pub struct Resource {
    pub binding: u32,
    pub name: &'static str,
    pub ty: &'static str,
    pub doc: &'static str,
}

const FRAME: Resource = Resource {
    binding: 0,
    name: "frame",
    ty: "var<uniform> frame: Frame",
    doc: "The frame (`Frame`).",
};

/// The shadow map at `binding`.
const fn shadow_map(binding: u32) -> Resource {
    Resource {
        binding,
        name: "shadow_map",
        ty: "var shadow_map: texture_depth_2d",
        doc: "This frame's shadow map: the depth of the solid terrain and entities the light sees, 0 at the light, 1 away from it (and where nothing is), stored at `shadow_position`'s mapping. Valid while `frame.has_shadow` is 1 (the pack asks for it with `[shadow]`), else a 1×1 map that lights everything.",
    }
}

/// The shadow map's comparison sampler at `binding`.
const fn shadow_sampler(binding: u32) -> Resource {
    Resource {
        binding,
        name: "shadow_sampler",
        ty: "var shadow_sampler: sampler_comparison",
        doc: "Depth comparison, bilinear (2×2 percentage-closer filtering): `textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z)` is 1 where the point is lit, 0 in shadow.",
    }
}

/// The shadow map with translucent terrain at `binding`.
const fn shadow_map_translucent(binding: u32) -> Resource {
    Resource {
        binding,
        name: "shadow_map_translucent",
        ty: "var shadow_map_translucent: texture_depth_2d",
        doc: "`shadow_map` with the translucent terrain (water, stained glass, ice) the light passes through drawn in too, read with `shadow_sampler` the same way: a point lit in `shadow_map` but not here lies under such a surface, which `shadow_color` gives the colour of.",
    }
}

/// The colour of the translucent terrain in the shadow map at `binding`.
const fn shadow_color(binding: u32) -> Resource {
    Resource {
        binding,
        name: "shadow_color",
        ty: "var shadow_color: texture_2d<f32>",
        doc: "The texture colour times the vertex colour (rgb: the tint, the face's directional shade and ambient occlusion) of the nearest translucent surface the light passes through at each texel of `shadow_map_translucent`, white where there is none; alpha is 1 on a fluid (water) and the texture's alpha, at most 0.99, on anything else. Read it with `textureLoad` at `shadow_position`'s xy times `textureDimensions(shadow_color)`.",
    }
}

/// A function the material and post preludes give packs.
pub const SHADOW_HELPERS: &str = "\
// The shadow map's distortion toward its centre, of a point at shadow clip `xy`.
fn shadow_distort(xy: vec2<f32>) -> vec2<f32> {
    let k = frame.shadow_distortion;
    return xy / (length(xy) * k + 1.0 - k);
}
// Where the camera-relative point `position` is stored in the shadow map: xy its 0..1 texture
// coordinate (from the top left), z its depth from the light, for `textureSampleCompareLevel`.
fn shadow_position(position: vec3<f32>) -> vec3<f32> {
    let clip = frame.shadow_view_proj * vec4<f32>(position, 1.0);
    let xy = shadow_distort(clip.xy);
    // At most 1, the depth where nothing is: past the map's far end a point stays lit.
    return vec3<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5, min(clip.z, 1.0));
}
";

const POST_RESOURCES: [Resource; 11] = [
    FRAME,
    Resource {
        binding: 1,
        name: "scene_color",
        ty: "var scene_color: texture_2d<f32>",
        doc: "The world's picture before any pass.",
    },
    Resource {
        binding: 2,
        name: "scene_depth",
        ty: "var scene_depth: texture_depth_2d",
        doc: "The world's depth, reversed (1 near, 0 far and sky).",
    },
    Resource {
        binding: 3,
        name: "previous",
        ty: "var previous: texture_2d<f32>",
        doc: "The previous pass's output (`scene_color` for the first pass).",
    },
    Resource {
        binding: 4,
        name: "linear_sampler",
        ty: "var linear_sampler: sampler",
        doc: "Bilinear filtering, clamped to the edges.",
    },
    Resource {
        binding: 5,
        name: "nearest_sampler",
        ty: "var nearest_sampler: sampler",
        doc: "Nearest texel, clamped to the edges.",
    },
    shadow_map(6),
    shadow_sampler(7),
    Resource {
        binding: 8,
        name: "history",
        ty: "var history: texture_2d<f32>",
        doc: "Last frame's input to the last pass (what the pass before it wrote), for temporal passes (TAA, denoising): put such a pass second to last and it reads its own previous output; reproject with `frame.previous_view_projection`. Black on the first frame, after a gap of over half a second, and with a single pass.",
    },
    shadow_map_translucent(9),
    shadow_color(10),
];

/// The material stage's resources: the frame, and the world as it was before translucent
/// terrain drew (for water and glass to see what lies behind them).
const MATERIAL_RESOURCES: [Resource; 11] = [
    FRAME,
    Resource {
        binding: 1,
        name: "reflection",
        ty: "var reflection: texture_2d<f32>",
        doc: "The water mirror: the world as a camera mirrored about the horizontal water plane around the camera (`frame.water_plane`) sees it, at half resolution, mapped like the world's picture, so water samples it at `input.screen`. Solid terrain, cut away below the plane, over the vanilla sky gradient: not the pack's `sky` stage, nor the sun, moon and stars. Drawn only for a pack whose material reads it, while `frame.has_reflection` is 1; else a 1×1 blank.",
    },
    Resource {
        binding: 2,
        name: "reflection_sampler",
        ty: "var reflection_sampler: sampler",
        doc: "Bilinear filtering, clamped to the edges.",
    },
    shadow_map(3),
    shadow_sampler(4),
    shadow_map_translucent(7),
    shadow_color(8),
    Resource {
        binding: 6,
        name: "opaque_depth",
        ty: "var opaque_depth: texture_depth_2d",
        doc: "The world's depth before translucent terrain (reversed, like `scene_depth`), for the translucent pass: `textureLoad(opaque_depth, vec2<i32>(input.screen * frame.resolution), 0)` is what lies behind water or glass. Stale while solid terrain draws.",
    },
    Resource {
        binding: 9,
        name: "block_light_colors",
        ty: "var block_light_colors: texture_3d<f32>",
        doc: "The colour of the block light around the camera, from the `[[lights]]` of the light sources nearby: rgb their colours averaged by how near each is, over 4; a how strongly they reach the cell. Read it through `block_light_color`.",
    },
    Resource {
        binding: 10,
        name: "block_light_grid",
        ty: "var<uniform> block_light_grid: vec4<f32>",
        doc: "Where `block_light_colors` lies: xyz its corner, camera-relative, w the side of one of its cells in blocks.",
    },
    Resource {
        binding: 11,
        name: "opaque_color",
        ty: "var opaque_color: texture_2d<f32>",
        doc: "The world's picture before translucent terrain (sky, solid terrain, entities), the size of `opaque_depth`, for the translucent pass: what a surface refracts. Read it with `textureLoad`. Stale while solid terrain draws; black when the world draws straight to the window (a pack without `[[post]]` passes, at full resolution).",
    },
];

/// Functions the post prelude gives packs.
const POST_HELPERS: &str = "\
// The world's depth at `uv` (reversed: 0 is the sky).
fn depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(scene_depth));
    let texel = vec2<i32>(clamp(uv * size, vec2<f32>(0.0), size - 1.0));
    return textureLoad(scene_depth, texel, 0);
}
// The camera-relative world position, in blocks, of the point at `uv` and `depth`.
fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let view = frame.projection_inverse * clip;
    return (frame.view_inverse * vec4<f32>(view.xyz / view.w, 0.0)).xyz;
}
";

/// What the material entries (the checker's below and the renderer's) share: the
/// surface's normal and tangent from the derivatives of its position and uv.
const MATERIAL_HELPERS: &str = "\
// The colour of the block light at the camera-relative `position` (rgb, from the `[[lights]]`
// of the sources nearby) and how strongly they reach it (a, 0 where none does). Block light
// keeps its own brightness (`input.light.x`): this is its colour only.
fn block_light_color(position: vec3<f32>) -> vec4<f32> {
    let size = vec3<f32>(textureDimensions(block_light_colors));
    let uvw = (position - block_light_grid.xyz) / (block_light_grid.w * size);
    if any(uvw < vec3<f32>(0.0)) || any(uvw > vec3<f32>(1.0)) {
        return vec4<f32>(0.0);
    }
    let c = textureSampleLevel(block_light_colors, reflection_sampler, uvw, 0.0);
    return vec4<f32>(c.rgb * 4.0, c.a);
}
// The point being shaded: its layer and mip level in the LabPBR maps (the entries set them).
var<private> horizon_block_layer: u32;
var<private> horizon_block_lod: f32;
// LabPBR: whether the resource packs ship normal (`_n`) or specular (`_s`) maps beside their
// block textures. Without them `block_normal_map` and `block_specular_map` give the defaults
// below.
fn block_pbr_maps() -> bool {
    return horizon_block_pbr_present();
}
// The surface's LabPBR normal map at `uv` (the sprite's space, as `input.uv`): rg the normal's
// x and y in the texture's space (x along `input.tangent`, y down the texture; 0..1 for
// -1..1), b ambient occlusion (1 none), a height (1 the top). (0.5, 0.5, 1, 1) where a texture
// has none.
fn block_normal_map(uv: vec2<f32>) -> vec4<f32> {
    return horizon_block_pbr(uv, horizon_block_layer, horizon_block_lod, 0u);
}
// `block_normal_map` at mip level `lod` (0 the full-size map).
fn block_normal_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32> {
    return horizon_block_pbr(uv, horizon_block_layer, lod, 0u);
}
// The surface's LabPBR specular map at `uv`: r perceptual smoothness, g F0 up to 229/255 and
// a metal's id above, b porosity or subsurface scattering, a emission (up to 254/255; 1
// none). (0, 0, 0, 0) where a texture has none.
fn block_specular_map(uv: vec2<f32>) -> vec4<f32> {
    return horizon_block_pbr(uv, horizon_block_layer, horizon_block_lod, 1u);
}
// `block_specular_map` at mip level `lod`.
fn block_specular_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32> {
    return horizon_block_pbr(uv, horizon_block_layer, lod, 1u);
}
// The LabPBR maps' size, in texels (they may be smaller than the block's texture).
fn block_pbr_size() -> vec2<f32> {
    return horizon_block_pbr_size();
}
// Block faces are flat: the derivatives of the position give the face's normal, turned
// toward the camera. Far away they lose precision, so a normal within about 3.6 degrees of
// an axis (every full block face) is that axis exactly: no shimmer on distant faces. The
// gentlest slope of flowing water, one level over a block, tilts about 6 degrees.
fn horizon_material_normal(position: vec3<f32>) -> vec3<f32> {
    let n = normalize(cross(dpdx(position), dpdy(position)));
    let a = abs(n);
    let m = max(a.x, max(a.y, a.z));
    let axis = select(select(vec3(0.0, 0.0, sign(n.z)), vec3(0.0, sign(n.y), 0.0), a.y == m),
        vec3(sign(n.x), 0.0, 0.0), a.x == m);
    let flat_n = select(n, axis, m > 0.998);
    return select(flat_n, -flat_n, dot(flat_n, position) > 0.0);
}
// The direction `uv.x` grows along on the surface, and the sign of the one `uv.y` grows
// along against `cross(normal, tangent)`.
fn horizon_material_tangent(position: vec3<f32>, uv: vec2<f32>, normal: vec3<f32>) -> vec4<f32> {
    let dp1 = dpdx(position);
    let dp2 = dpdy(position);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, normal);
    let dp1perp = cross(normal, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    if length(t) < 1e-12 {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    let tangent = normalize(t - normal * dot(normal, t));
    return vec4<f32>(tangent, select(-1.0, 1.0, dot(cross(normal, tangent), b) >= 0.0));
}
";

/// The post stage's entry points, as the renderer builds them: a full-screen
/// triangle, and a fragment that calls the pack's `post` for each pixel.
const POST_ENTRIES: &str = "\
struct HorizonPostOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn horizon_vs_post(@builtin(vertex_index) i: u32) -> HorizonPostOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: HorizonPostOut;
    out.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}
@fragment fn horizon_fs_post(in: HorizonPostOut) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(previous, linear_sampler, in.uv, 0.0);
    return post(PostInput(in.uv, in.position.xy, color, depth_at(in.uv)));
}
";

/// What the renderer gives the material prelude's LabPBR functions (`native_material.rs`); the
/// check and the entries below put a stand-in in its place.
macro_rules! pbr_stub {
    () => {
        "fn horizon_block_pbr(uv: vec2<f32>, layer: u32, lod: f32, map: u32) -> vec4<f32> {
    return select(vec4<f32>(0.5, 0.5, 1.0, 1.0), vec4<f32>(0.0), map == 1u);
}
fn horizon_block_pbr_present() -> bool {
    return false;
}
fn horizon_block_pbr_size() -> vec2<f32> {
    return vec2<f32>(16.0);
}
"
    };
}
pub const PBR_STUB: &str = pbr_stub!();

/// Entry points with the material stage's inputs: the renderer calls
/// `material` from its own terrain shader in the same way.
const MATERIAL_ENTRIES: &str = concat!(
    pbr_stub!(),
    "\
struct HorizonMaterialIn {
    @builtin(position) position: vec4<f32>,
    @location(0) albedo: vec4<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) light: vec2<f32>,
    @location(3) lightmap: vec3<f32>,
    @location(4) world: vec3<f32>,
    @location(5) uv: vec2<f32>,
    @location(6) @interpolate(flat) translucent: u32,
    @location(7) @interpolate(flat) tags: u32,
    @location(8) @interpolate(flat) face: u32,
    @location(9) @interpolate(flat) fluid_quad: u32,
    @location(10) @interpolate(flat) emission: f32,
    @location(11) @interpolate(flat) material: u32,
}
@fragment fn horizon_fs_material(in: HorizonMaterialIn) -> @location(0) vec4<f32> {
    let fluid = select(0u, select(2u, 1u, in.translucent != 0u), in.fluid_quad != 0u);
    let normal = horizon_material_normal(in.world);
    let tangent = horizon_material_tangent(in.world, in.uv, normal);
    let input = MaterialInput(in.albedo, in.tint, in.light, in.lightmap, in.world, in.uv, in.translucent != 0u, in.tags, in.material,
        in.position.xy / frame.resolution, in.face, fluid, in.emission, normal, tangent);
    return material(input).color;
}
",
);

/// Entry points with the sky stage's inputs: the renderer calls `sky` from
/// its own sky shader in the same way.
const SKY_ENTRIES: &str = "\
struct HorizonSkyIn {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
    @location(1) color: vec4<f32>,
}
@fragment fn horizon_fs_sky(in: HorizonSkyIn) -> @location(0) vec4<f32> {
    return sky(SkyInput(normalize(in.direction), in.color));
}
";

/// An entry point with the weather stage's inputs: the renderer calls `weather` from its own
/// weather shader in the same way.
const WEATHER_ENTRIES: &str = "\
struct HorizonWeatherIn {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) light: vec2<f32>,
    @location(2) world: vec3<f32>,
    @location(3) @interpolate(flat) snow: u32,
}
@fragment fn horizon_fs_weather(in: HorizonWeatherIn) -> @location(0) vec4<f32> {
    return weather(WeatherInput(in.color, in.light, in.world, in.snow != 0u));
}
";

impl StageKind {
    pub const ALL: [StageKind; 4] = [
        StageKind::Sky,
        StageKind::Material,
        StageKind::Weather,
        StageKind::Post,
    ];

    /// The manifest's name for the stage (`[sky]`, `[material]`, `[weather]`, `[[post]]`).
    pub fn name(self) -> &'static str {
        match self {
            StageKind::Sky => "sky",
            StageKind::Material => "material",
            StageKind::Weather => "weather",
            StageKind::Post => "post",
        }
    }

    /// The function a pack defines: it has the stage's name.
    pub fn function(self) -> &'static str {
        self.name()
    }

    pub fn input(self) -> &'static Struct {
        match self {
            StageKind::Sky => &SKY_INPUT,
            StageKind::Material => &MATERIAL_INPUT,
            StageKind::Weather => &WEATHER_INPUT,
            StageKind::Post => &POST_INPUT,
        }
    }

    /// The type the function returns.
    pub fn output(self) -> &'static str {
        match self {
            StageKind::Material => SURFACE.name,
            StageKind::Sky | StageKind::Weather | StageKind::Post => "vec4<f32>",
        }
    }

    /// `fn <name>(input: <Input>) -> <Output>`.
    pub fn signature(self) -> String {
        format!(
            "fn {}(input: {}) -> {}",
            self.function(),
            self.input().name,
            self.output()
        )
    }

    pub fn resources(self) -> &'static [Resource] {
        match self {
            StageKind::Post => &POST_RESOURCES,
            StageKind::Sky | StageKind::Weather => std::slice::from_ref(&FRAME),
            StageKind::Material => &MATERIAL_RESOURCES,
        }
    }

    /// Everything the pack's files may use, declared before them.
    pub fn prelude(self) -> String {
        let mut out =
            String::from("// Horizon shader API prelude (docs/api/shaders-reference.md)\n");
        out += &Frame::WGSL.wgsl();
        out += &self.input().wgsl();
        if self == StageKind::Material {
            out += &SURFACE.wgsl();
        }
        for r in self.resources() {
            out += &format!("@group({GROUP}) @binding({}) {};\n", r.binding, r.ty);
        }
        match self {
            StageKind::Post => {
                out += POST_HELPERS;
                out += SHADOW_HELPERS;
            }
            StageKind::Material => {
                out += MATERIAL_HELPERS;
                out += SHADOW_HELPERS;
            }
            StageKind::Sky | StageKind::Weather => {}
        }
        out
    }

    /// The entry points put after the pack's files.
    pub fn entries(self) -> &'static str {
        match self {
            StageKind::Sky => SKY_ENTRIES,
            StageKind::Material => MATERIAL_ENTRIES,
            StageKind::Weather => WEATHER_ENTRIES,
            StageKind::Post => POST_ENTRIES,
        }
    }
}

/// The first binding of a pack's `[[textures]]` in group 3; their samplers follow at
/// `TEXTURE_SAMPLER_BINDING`.
pub const TEXTURE_BINDING: u32 = 16;
pub const TEXTURE_SAMPLER_BINDING: u32 = 24;

/// The declarations of a pack's `[[textures]]`, part of every stage's prelude.
pub fn texture_declarations(textures: &[crate::manifest::PackTexture]) -> String {
    let mut out = String::new();
    for (i, t) in textures.iter().enumerate() {
        out += &format!(
            "@group({GROUP}) @binding({}) var {}: texture_2d<f32>;\n\
             @group({GROUP}) @binding({}) var {}_sampler: sampler;\n",
            TEXTURE_BINDING + i as u32,
            t.id,
            TEXTURE_SAMPLER_BINDING + i as u32,
            t.id
        );
    }
    out
}

/// A stage's WGSL, and where each part of it came from.
#[derive(Debug)]
pub struct Composed {
    pub text: String,
    /// (the file, or `<prelude>` / `<entries>`, its byte range in `text`).
    pub parts: Vec<(String, std::ops::Range<usize>)>,
}

impl Composed {
    /// The file, line and column (1-based) of a byte offset in `text`.
    pub fn locate(&self, offset: usize) -> (&str, usize, usize) {
        let (file, range) = self
            .parts
            .iter()
            .find(|(_, r)| r.contains(&offset))
            .or(self.parts.last())
            .expect("a composed stage has parts");
        let before = &self.text[range.start..offset.clamp(range.start, range.end)];
        let line = before.matches('\n').count() + 1;
        let column = before.len() - before.rfind('\n').map_or(0, |i| i + 1) + 1;
        (file, line, column)
    }
}

/// The binding of a pack's first post target (`[[post]] output`) in group 3, the others after.
pub const POST_TARGET_BINDING: u32 = 32;

/// The declarations of a pack's post targets, part of the post stage's prelude.
pub fn post_target_declarations(manifest: &crate::manifest::Manifest) -> String {
    manifest
        .post_targets()
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            format!(
                "@group({GROUP}) @binding({}) var {name}: texture_2d<f32>;\n",
                POST_TARGET_BINDING + i as u32
            )
        })
        .collect()
}

/// The WGSL of the stage `kind` whose own file is `file`: the prelude, the
/// pack's shared files in order, the file, then the entry points.
pub fn compose(pack: &Pack, kind: StageKind, file: &str) -> Composed {
    let mut composed = Composed {
        text: String::new(),
        parts: Vec::new(),
    };
    let mut push = |name: &str, text: &str| {
        let start = composed.text.len();
        composed.text += text;
        if !text.ends_with('\n') {
            composed.text.push('\n');
        }
        composed
            .parts
            .push((name.to_owned(), start..composed.text.len()));
    };
    let mut prelude = kind.prelude() + &texture_declarations(&pack.manifest.textures);
    if kind == StageKind::Post {
        prelude += &post_target_declarations(&pack.manifest);
    }
    push("<prelude>", &prelude);
    for shared in pack.manifest.shared.iter().filter(|s| *s != file) {
        push(shared, pack.source(shared).unwrap_or_default());
    }
    // The stage's own extra files, between the shared ones and its file.
    let m = &pack.manifest;
    let stage = match kind {
        StageKind::Sky => m.sky.as_ref(),
        StageKind::Material => m.material.as_ref(),
        StageKind::Weather => m.weather.as_ref(),
        StageKind::Post => m.post.iter().find(|s| s.file == file),
    };
    for extra in stage.into_iter().flat_map(|s| &s.files) {
        if extra != file && !m.shared.contains(extra) {
            push(extra, pack.source(extra).unwrap_or_default());
        }
    }
    push(file, pack.source(file).unwrap_or_default());
    push("<entries>", kind.entries());
    composed
}
