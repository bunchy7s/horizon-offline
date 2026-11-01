//! Vanilla's post-processing GLSL (`#version 330` with `layout(std140)` uniform blocks
//! and `uniform sampler2D`s, Vulkan-style `gl_VertexIndex`) turned into naga modules at
//! run time, from whichever resource root is active, the way Horizon's shader packs turn
//! their GLSL into naga (`horizon-shaders`' translator, which handles the compatibility
//! dialect; the post shaders need only the block and sampler binding rewrite below).
//!
//! `ShaderManager`/`GlslPreprocessor` read `assets/<ns>/shaders/<path>.{vsh,fsh}` and
//! resolve `#include <ns:path>` to `assets/<ns>/shaders/include/<path>`.
//!
//! The rewrite gives every uniform block a binding of bind group 0, shared by both
//! stages, and splits each `uniform sampler2D NameSampler;` into a texture and a sampler
//! object at the next two bindings, `NameSampler` becoming the combined
//! `sampler2D(texture, sampler)` again where it is used. The vertex stage's `main` is
//! wrapped so that clip y is negated: targets keep GL's rows (row 0 is the bottom edge),
//! so `texCoord` reads the texel it would in vanilla.
use naga::ShaderStage;
use naga::front::glsl::{Frontend, Options};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::path::Path;

/// One program, ready for wgpu.
pub struct Program {
    pub vertex: naga::Module,
    pub fragment: naga::Module,
    /// Every uniform block either stage declares, with its binding.
    pub blocks: Vec<(String, u32)>,
    /// Every sampler (`NameSampler`): the texture's binding; the sampler object is the next.
    pub samplers: Vec<(String, u32)>,
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The source with its comments blanked (same length, so offsets stay valid).
fn blank_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                if b[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            for slot in out.iter_mut().take((i + 2).min(b.len())).skip(i) {
                *slot = b' ';
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// `GlslPreprocessor`'s `#include <ns:path>`.
fn read_source(root: &Path, id: &str, ext: &str) -> Result<String, String> {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    let file = root
        .join("assets")
        .join(ns)
        .join("shaders")
        .join(format!("{path}.{ext}"));
    std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))
}

fn include(root: &Path, src: &str, depth: u32) -> Result<String, String> {
    if depth > 8 {
        return Err("#include nested too deep".into());
    }
    let mut out = String::new();
    for line in src.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("#include") {
            let name = rest
                .trim()
                .trim_start_matches(['<', '"'])
                .trim_end_matches(['>', '"'])
                .trim();
            let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
            let file = root
                .join("assets")
                .join(ns)
                .join("shaders")
                .join("include")
                .join(path);
            let text = std::fs::read_to_string(&file)
                .map_err(|e| format!("#include <{name}>: {}: {e}", file.display()))?;
            out.push_str(&include(root, &text, depth + 1)?);
        } else if t.starts_with("#version") || t.starts_with("#extension") {
            // Re-added below, for the translator.
            out.push('\n');
            continue;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    Ok(out)
}

/// Every `uniform` declaration of a source: (start of the declaration including a
/// `layout(...)` before it, end of the type or of the whole `;`, type, name, is a sampler).
fn declarations(src: &str) -> Vec<(usize, usize, String, String, bool)> {
    let plain = blank_comments(src);
    let b = plain.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = plain[at..].find("uniform") {
        let start = at + found;
        at = start + 7;
        if (start > 0 && is_ident(b[start - 1])) || b.get(start + 7).is_some_and(|c| is_ident(*c)) {
            continue;
        }
        let word = |from: usize| -> Option<(usize, usize)> {
            let mut s = from;
            while s < b.len() && b[s].is_ascii_whitespace() {
                s += 1;
            }
            let mut e = s;
            while e < b.len() && is_ident(b[e]) {
                e += 1;
            }
            (e > s).then_some((s, e))
        };
        let Some((ts, te)) = word(start + 7) else {
            continue;
        };
        let ty = plain[ts..te].to_owned();
        // The `layout(...)` right before `uniform`, if any.
        let before = plain[..start].trim_end();
        let decl_start = before
            .ends_with(')')
            .then(|| before.rfind("layout"))
            .flatten()
            .filter(|l| {
                before[*l + 6..].trim_start().starts_with('(') && !before[*l..].contains(';')
            })
            .unwrap_or(start);
        if ty.starts_with("sampler") {
            let Some((ns, ne)) = word(te) else { continue };
            let end = plain[ne..].find(';').map_or(ne, |p| ne + p + 1);
            out.push((decl_start, end, ty, plain[ns..ne].to_owned(), true));
        } else {
            let mut s = te;
            while s < b.len() && b[s].is_ascii_whitespace() {
                s += 1;
            }
            if b.get(s) == Some(&b'{') {
                out.push((decl_start, te, String::new(), ty, false));
            }
        }
    }
    out
}

/// The source with each declaration given its binding from the shared tables.
fn rewrite(src: &str, blocks: &[(String, u32)], samplers: &[(String, u32)]) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for (start, end, ty, name, sampler) in declarations(src) {
        out.push_str(&src[pos..start]);
        pos = end;
        if sampler {
            let tex = samplers.iter().find(|(n, _)| *n == name).map_or(0, |s| s.1);
            let kind = if ty == "sampler2D" { "texture2D" } else { &ty };
            out.push_str(&format!(
                "layout(set = 0, binding = {tex}) uniform {kind} {name}_hz_tex; \
                 layout(set = 0, binding = {}) uniform sampler {name}_hz_smp;\n\
                 #define {name} sampler2D({name}_hz_tex, {name}_hz_smp)\n",
                tex + 1
            ));
        } else {
            let binding = blocks.iter().find(|(n, _)| *n == name).map_or(0, |s| s.1);
            out.push_str(&format!(
                "layout(std140, set = 0, binding = {binding}) uniform {name}"
            ));
        }
    }
    out.push_str(&src[pos..]);
    out
}

/// Replaces the word `main` of `void main(` with `hz_main`.
fn wrap_main(src: &str) -> Result<String, String> {
    let plain = blank_comments(src);
    let mut at = 0;
    while let Some(found) = plain[at..].find("main") {
        let p = at + found;
        at = p + 4;
        let b = plain.as_bytes();
        if (p > 0 && is_ident(b[p - 1])) || b.get(p + 4).is_some_and(|c| is_ident(*c)) {
            continue;
        }
        if plain[..p].trim_end().ends_with("void") && plain[p + 4..].trim_start().starts_with('(') {
            let mut out = String::with_capacity(src.len() + 80);
            out.push_str(&src[..p]);
            out.push_str("hz_main");
            out.push_str(&src[p + 4..]);
            out.push_str("\nvoid main() { hz_main(); gl_Position.y = -gl_Position.y; }\n");
            return Ok(out);
        }
    }
    Err("no main".into())
}

fn parse(stage: ShaderStage, text: &str) -> Result<naga::Module, String> {
    let module = Frontend::default()
        .parse(&Options::from(stage), text)
        .map_err(|e| {
            e.errors
                .iter()
                .map(|e| e.kind.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| format!("{:?}", e.into_inner()))?;
    Ok(module)
}

/// Translates the vertex and fragment shader a pass names (`minecraft:post/invert`).
pub fn translate(root: &Path, vertex: &str, fragment: &str) -> Result<Program, String> {
    let vs = include(root, &read_source(root, vertex, "vsh")?, 0)?;
    let fs = include(root, &read_source(root, fragment, "fsh")?, 0)?;
    // Blocks and samplers are shared by name across the stages, so both agree on a binding.
    let mut blocks: Vec<(String, u32)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for src in [&vs, &fs] {
        for (_, _, _, name, sampler) in declarations(src) {
            if sampler {
                if !names.contains(&name) {
                    names.push(name);
                }
            } else if !blocks.iter().any(|(n, _)| *n == name) {
                blocks.push((name, blocks.len() as u32));
            }
        }
    }
    let first = blocks.len() as u32;
    let samplers: Vec<(String, u32)> = names.into_iter().zip((first..).step_by(2)).collect();
    let vs = wrap_main(&rewrite(&vs, &blocks, &samplers))?;
    let fs = rewrite(&fs, &blocks, &samplers);
    let vertex_module = parse(ShaderStage::Vertex, &format!("#version 450\n{vs}"))
        .map_err(|e| format!("{vertex}: {e}"))?;
    let fragment_module = parse(ShaderStage::Fragment, &format!("#version 450\n{fs}"))
        .map_err(|e| format!("{fragment}: {e}"))?;
    Ok(Program {
        vertex: vertex_module,
        fragment: fragment_module,
        blocks,
        samplers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> Option<std::path::PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)?
            .join(".local/horizon-client/26.3");
        dir.join("assets/minecraft/shaders/post/blit.fsh")
            .exists()
            .then_some(dir)
    }

    #[test]
    fn vanilla_post_shaders_translate() {
        let Some(root) = root() else { return };
        let vs = ["minecraft:core/screenquad", "minecraft:post/rotscale"];
        let fs = [
            "bits",
            "blit",
            "box_blur",
            "color_convolve",
            "entity_outline_box_blur",
            "entity_sobel",
            "invert",
            "spiderclip",
        ];
        for f in fs {
            for v in vs {
                let name = format!("minecraft:post/{f}");
                let p = translate(&root, v, &name);
                // rotscale's outputs only match spiderclip, but each stage translates alone.
                if let Err(e) = &p {
                    panic!("{v} + {name}: {e}");
                }
            }
        }
    }

    #[test]
    fn sampler_and_block_bindings() {
        let src = "layout(std140) uniform A {\n float x;\n};\nuniform sampler2D InSampler; // c\nvoid main(){}\n";
        let d = declarations(src);
        assert_eq!(d.len(), 2);
        let out = rewrite(src, &[("A".into(), 0)], &[("InSampler".into(), 1)]);
        assert!(out.contains("layout(std140, set = 0, binding = 0) uniform A {"));
        assert!(out.contains("binding = 1) uniform texture2D InSampler_hz_tex;"));
        assert!(out.contains("binding = 2) uniform sampler InSampler_hz_smp;"));
        assert!(wrap_main("void main() {}").unwrap().contains("hz_main"));
    }
}
