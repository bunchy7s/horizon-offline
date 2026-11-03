//! docs/api/shaders-reference.md, generated from the stages and inputs this
//! crate defines (`tests/reference.rs` fails when the file drifts).
use crate::check::{MAX_EXPRESSIONS, MAX_TEXTURE_READS};
use crate::inputs::{Frame, SURFACE, Struct};
use crate::manifest::{API, MAX_GPU_MS, MAX_OPTIONS, MAX_POST_PASSES};
use crate::package::{MAX_TOTAL, MAX_WGSL};
use crate::stages::{GROUP, StageKind};

fn table(s: &Struct, offsets: Option<&[usize]>) -> String {
    let mut out = format!("### `{}`\n\n{}\n\n", s.name, one_line(s.doc));
    out += if offsets.is_some() {
        "| Field | Type | Offset | Meaning |\n| --- | --- | --- | --- |\n"
    } else {
        "| Field | Type | Meaning |\n| --- | --- | --- |\n"
    };
    for (i, f) in s.fields.iter().enumerate() {
        let offset = offsets.map_or(String::new(), |o| format!(" {} |", o[i]));
        out += &format!(
            "| `{}` | `{}` |{offset} {} |\n",
            f.name,
            f.ty,
            one_line(f.doc)
        );
    }
    out + "\n"
}

fn one_line(doc: &str) -> String {
    doc.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn markdown() -> String {
    let mut out = format!(
        "# Shader API reference ({API})\n\n\
         Generated from `crates/horizon-shader-api` (`HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test reference`\n\
         rewrites it; the test fails when it drifts). The API's design and pack format: [shaders.md](shaders.md).\n\n\
         A stage is a function your WGSL defines, with the signature below; Horizon puts the prelude (these structs, the\n\
         resources and helpers) before your files and calls the function from its own pass. Your files declare no\n\
         bindings and no entry points, and no names starting with `horizon_`.\n\n\
         ## Stages\n\n\
         | Stage | In `horizon-shaders.toml` | Your function |\n| --- | --- | --- |\n"
    );
    for kind in StageKind::ALL {
        let key = match kind {
            StageKind::Post => "[[post]]",
            StageKind::Sky => "[sky]",
            StageKind::Material => "[material]",
            StageKind::Vertex => "[vertex]",
            StageKind::Weather => "[weather]",
        };
        out += &format!("| {} | `{key}` | `{}` |\n", kind.name(), kind.signature());
    }
    out += "\n";
    for kind in StageKind::ALL {
        out += &format!("## {}\n\n`{}`\n\n", kind.name(), kind.signature());
        out += &table(kind.input(), None);
        if kind == StageKind::Material {
            out += &table(&SURFACE, None);
        }
        out += &format!(
            "Resources (bind group {GROUP}):\n\n| Binding | Declaration | Meaning |\n| --- | --- | --- |\n"
        );
        for r in kind.resources() {
            out += &format!("| {} | `{}` | {} |\n", r.binding, r.ty, r.doc);
        }
        out += "\n";
        if kind != StageKind::Sky {
            out += "Helpers:\n\n";
        }
        if kind == StageKind::Post {
            out += "- `fn depth_at(uv: vec2<f32>) -> f32`: the world's depth at `uv`.\n\
                    - `fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32>`: the camera-relative position, in blocks, of\n  \
                    the point at `uv` and `depth`.\n\
                    - `fn surface_at(texel: vec2<i32>) -> SurfaceData`: the solid terrain's surface at a pixel, as the \
                    material stage's `set_surface` left it: `present` (false where the sky, an entity, water, glass or a \
                    particle shows instead), `normal` (world axes), `smoothness` and `data`.\n";
        }
        if kind == StageKind::Material {
            out += "- `fn block_texture(uv: vec2<f32>) -> vec4<f32>`: the surface's block texture at `uv`, anywhere on \
                    the sprite, without the tint.\n\
                    - `fn block_texture_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at mip level `lod`.\n\
                    - `fn block_texture_size() -> vec2<f32>`: its size in texels.\n\
                    - `fn block_texture_lod() -> f32`: the mip level `block_texture` reads at this point.\n";
            out += "- `fn block_pbr_maps() -> bool`: whether the resource packs ship LabPBR maps (`_n`, `_s`).\n\
                    - `fn block_normal_map(uv: vec2<f32>) -> vec4<f32>`: the surface's LabPBR normal map at `uv` (rg the \
                    normal along `input.tangent` and down the texture, b ambient occlusion, a height); (0.5, 0.5, 1, 1) \
                    without one.\n\
                    - `fn block_specular_map(uv: vec2<f32>) -> vec4<f32>`: its LabPBR specular map (r smoothness, g F0 or a \
                    metal, b porosity or subsurface, a emission, 1 none); (0, 0, 0, 0) without one.\n\
                    - `fn block_normal_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`, \
                    `fn block_specular_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at the block texture's mip level `lod`.\n\
                    - `fn block_pbr_size() -> vec2<f32>`: the LabPBR maps' size in texels.\n\
                    - `fn set_surface(normal: vec3<f32>, smoothness: f32, data: f32)`: what the post stage's \
                    `surface_at` reads for this point of solid terrain (`data` the pack's own; the face's normal and 0, 0 unset).\n";
        }
        if kind != StageKind::Sky {
            out += "- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is\n  \
                    stored in the shadow map, for `textureSampleCompareLevel`.\n\
                    - `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.\n\n";
        }
        if kind == StageKind::Post {
            out += "A `[[post]]` entry with `output = \"<name>\"` writes a target of the pack's own, at its `scale` of the\n\
                    world's size (1, 0.5 or 0.25), instead of the next pass's `previous`; the prelude of every pass declares\n\
                    `var <name>: texture_2d<f32>` (group 3 from binding 32), which holds last frame's until written again (a\n\
                    pass reads a blank one in place of the target it writes).\n\n";
        }
    }
    out += "## The frame\n\n";
    out += &table(&Frame::WGSL, Some(Frame::OFFSETS));
    out += &format!(
        "## Options\n\n\
         Each `[[options]]` entry sets the WGSL `override` of the same name when the pipelines are built:\n\n\
         | `type` | Keys | The `override` |\n| --- | --- | --- |\n\
         | `bool` | `default` | `override <id>: bool` |\n\
         | `float` | `default`, `min`, `max`, `step` | `override <id>: f32` |\n\
         | `int` | `default`, `min`, `max` | `override <id>: i32` |\n\
         | `choice` | `values`, `default` (one of them) | `override <id>: u32`, the value's index |\n\n\
         ## Limits\n\n\
         - {MAX_POST_PASSES} post passes, {MAX_OPTIONS} options, `budget.gpu_ms` up to {MAX_GPU_MS}.\n\
         - {} KiB per WGSL file, {} MiB per pack.\n\
         - Per stage: {MAX_EXPRESSIONS} naga expressions over all its functions, {MAX_TEXTURE_READS} texture reads (samples,\n  \
         gathers and loads, the prelude's included).\n\n\
         ## The check\n\n\
         A pack runs only after it passes `horizon_shader_api::check` (the game when it loads a pack, the launcher before it\n\
         installs one, the registry's CI): each stage is parsed and validated by naga, checked against its signature and\n\
         the rules above, and translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL) with its options at their\n\
         defaults, minimums and maximums. Every option must be an `override` of its type in some stage; an `override`\n\
         that is no option needs a default.\n",
        MAX_WGSL >> 10,
        MAX_TOTAL >> 20
    );
    out
}
