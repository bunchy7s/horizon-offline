//! A program's preprocessed GLSL (any version from 110 to 460, compatibility or core) turned
//! into the Vulkan-flavoured GLSL 450 naga reads, then into validated naga modules (spec 1.6,
//! 2.2). The rewrite works on preprocessing tokens and never adds or removes a line of the pack's
//! code, so naga's line numbers map back through the preprocessor to the pack's files.
//!
//! What it does:
//! - Loose uniforms go into one uniform block shared by both stages (`bool` stored as `int`).
//! - Each sampler is split into a texture and a sampler object, combined again where it is used
//!   (`sampler2D(t, s)`), or passed as two arguments to the pack's own functions.
//! - The compatibility built-ins (`gl_Vertex`, `gl_MultiTexCoordN`, `gl_*Matrix`, `ftransform()`,
//!   `gl_FragData`, `gl_FragColor`, `gl_TexCoord`, `texture2D`...) become declarations of ours,
//!   named with the reserved prefix `hz_`; varyings get locations by name across the stages.
//! - The pack's `main` becomes `hz_main`, called by a `main` of ours that sets the inputs and
//!   adapts GL's conventions (spec 5.5): clip y is negated for every target but the window, so
//!   memory rows are GL's rows and `gl_FragCoord` needs no change, and clip z goes from GL's
//!   [-w, w] to [0, w], so stored depth is GL's window depth.
//!
//! Composite-style programs draw one quad from (0,0) to (1,1) with an identity model-view and an
//! orthographic projection onto it, the texture coordinate equal to the position (OQ-6).

use std::collections::BTreeMap;
use std::fmt::Write;

use naga::ShaderStage;
use naga::front::glsl::{Frontend, Options};
use naga::valid::{Capabilities, ValidationFlags, Validator};

use crate::Preprocessed;

mod atomic_vectors;
mod geometry;
mod ordering;
mod rewrite;
mod scan;

pub use geometry::{GeometryInputs, translate_geometry};

use scan::Stage;

/// What a program's stages need from the game.
#[derive(Debug, Default, PartialEq)]
pub struct Interface {
    /// The uniform block at binding 0: its size and each value's name, GLSL type and offset.
    /// Ours come first: `hz_target` (the target's width, height, 1/width, 1/height), and the
    /// fixed-function fog the compatibility `gl_Fog` reads (OQ-28): `hz_fog_color` (vanilla's fog
    /// colour) and `hz_fog` (density, start, end, 1 / (end - start)).
    pub uniform_size: u32,
    pub uniforms: Vec<Uniform>,
    /// Each sampler: texture at `binding`, sampler object at `binding + 1`.
    pub samplers: Vec<Sampler>,
    /// The fragment stage's output locations.
    pub outputs: Vec<u32>,
    /// A gbuffers program reads `DRAW_UNIFORMS`: the block at set 2, binding 0, set per draw.
    pub draw: bool,
    /// Storage images (`colorimg4`...), after the samplers.
    pub images: Vec<Image>,
    /// The SSBOs' bindings in set 4 (`bufferObject.<n>`).
    pub buffers: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub name: String,
    /// `image2D`, `uimage3D`...
    pub ty: String,
    /// The format qualifier (`rgba16f`).
    pub format: String,
    /// `readonly`, `writeonly`, empty for both, or `atomic` for an image an atomic function
    /// uses.
    pub access: String,
    pub binding: u32,
    /// A compute program's copy of image `before` as it was before the dispatch, which its
    /// loads read (see `ordering`).
    pub before: Option<String>,
}

/// A compute program (`deferred4_a.csh`): its module, what it reads and writes, and how many
/// work groups it runs: `workGroups`, or `workGroupsRender` times the target's size over the
/// local size (1, 1 when neither is set).
pub struct ComputeProgram {
    pub module: naga::Module,
    pub interface: Interface,
    pub work_groups: Option<[u32; 3]>,
    pub work_groups_render: [f32; 2],
}

/// The uniforms a gbuffers program reads per draw (5.3's "D"), in a block of their own
/// (`HzDraw`, std140: three ints, a pad, a vec4; 32 bytes) the game sets for each draw.
pub const DRAW_UNIFORMS: [(&str, &str); 4] = [
    ("renderStage", "int"),
    ("entityId", "int"),
    ("blockEntityId", "int"),
    ("entityColor", "vec4"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Uniform {
    pub name: String,
    /// The type the pack declared (`float`, `ivec2`, `mat4`; `bool` is stored as an `int`).
    pub ty: String,
    pub offset: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sampler {
    pub name: String,
    /// The type the pack declared: `sampler2D`, `sampler2DShadow`, `usampler2D`, `sampler3D`...
    pub ty: String,
    pub binding: u32,
}

/// A program that failed, at a pack file and line.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub file: String,
    pub line: u32,
    pub message: String,
}

pub struct Program {
    pub vertex: naga::Module,
    pub fragment: naga::Module,
    pub interface: Interface,
}

/// Where the program draws, which decides the conventions of 5.5.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// Pack buffers: rows stored as GL stores them.
    Buffers,
    /// The window (`final`).
    Window,
}

/// What a gbuffers-style program's vertex stage reads. The game writes it: it knows its vertex
/// formats, the translator only places it.
#[derive(Clone, Debug, Default)]
pub struct GbuffersInputs {
    /// GLSL declarations: the vertex inputs and any bindings of set 1.
    pub declarations: String,
    /// GLSL statements that set `hz_Vertex`, `hz_Color`, `hz_MultiTexCoord0`,
    /// `hz_MultiTexCoord1` and `hz_Normal` (the compatibility inputs), run first in `main`.
    pub body: String,
    /// The value of each attribute a pack may declare (`mc_Entity`, `vaPosition`...), as a GLSL
    /// expression of the declared type's size; an attribute not listed reads zero.
    pub attributes: Vec<(String, String)>,
    /// GLSL statements run last in `main`, with `gl_Position` in wgpu's clip space.
    pub after: String,
    /// The alpha test applied to output 0 (`gl_FragData[0]`, `gl_FragColor`), the loader's
    /// (3.2 `alphaTest.<program>`): `None` keeps every fragment.
    pub alpha_test: Option<AlphaTest>,
}

/// A fragment stays when its output 0's alpha compares to `reference` as `op` says (a GLSL
/// comparison operator, `true` for GL's `ALWAYS`, `false` for `NEVER`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaTest {
    pub op: &'static str,
    pub reference: f32,
}

impl AlphaTest {
    /// `alphaTest.<program>`'s value: `off`, or a GL comparison and a reference.
    pub fn parse(value: &str) -> Option<Option<Self>> {
        let mut words = value.split_whitespace();
        let func = words.next()?;
        if func == "off" {
            return Some(None);
        }
        let op = match func.trim_start_matches("GL_") {
            "NEVER" => "false",
            "LESS" => "<",
            "EQUAL" => "==",
            "LEQUAL" => "<=",
            "GREATER" => ">",
            "NOTEQUAL" => "!=",
            "GEQUAL" => ">=",
            "ALWAYS" => return Some(None),
            _ => return None,
        };
        let reference = words.next()?.parse().ok()?;
        Some(Some(Self { op, reference }))
    }
}

/// Translates a composite-style program (`composite`, `deferred`, `final`...).
pub fn translate_composite(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    target: Target,
) -> Result<Program, Vec<Error>> {
    translate(vertex, fragment, target, None, false)
}

/// Translates a gbuffers-style program (`gbuffers_terrain`...) drawing the game's geometry
/// described by `inputs`, into `target` (the pack's buffers).
pub fn translate_gbuffers(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    target: Target,
    inputs: &GbuffersInputs,
) -> Result<Program, Vec<Error>> {
    translate(vertex, fragment, target, Some(inputs), false)
}

/// Translates a shadow program (`shadow`, `shadow_water`...) drawing the game's geometry
/// described by `inputs` from the shadow light: its model-view and projection matrices are the
/// shadow camera's (5.2).
pub fn translate_shadow(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    inputs: &GbuffersInputs,
) -> Result<Program, Vec<Error>> {
    translate(vertex, fragment, Target::Buffers, Some(inputs), true)
}

/// The matrices of the compatibility profile in a gbuffers-style program, from the frame's.
const GBUFFERS_MATRICES: &str = "    hz_ModelViewMatrix = gbufferModelView;\n    \
     hz_ProjectionMatrix = gbufferProjection;\n    \
     hz_ModelViewProjectionMatrix = gbufferProjection * gbufferModelView;\n    \
     hz_ModelViewMatrixInverse = gbufferModelViewInverse;\n    \
     hz_ProjectionMatrixInverse = gbufferProjectionInverse;\n    \
     hz_NormalMatrix = mat3(gbufferModelView);\n    \
     mat4 hz_light = mat4(vec4(1.0 / 256.0, 0.0, 0.0, 0.0), vec4(0.0, 1.0 / 256.0, 0.0, 0.0), vec4(0.0, 0.0, 1.0, 0.0), vec4(1.0 / 32.0, 1.0 / 32.0, 0.0, 1.0));\n    \
     hz_TextureMatrix = mat4[8](mat4(1.0), hz_light, hz_light, mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0));\n";

fn translate(
    vertex: &Preprocessed,
    fragment: &Preprocessed,
    target: Target,
    gbuffers: Option<&GbuffersInputs>,
    shadow: bool,
) -> Result<Program, Vec<Error>> {
    let matrices = if shadow {
        GBUFFERS_MATRICES.replace("gbuffer", "shadow")
    } else {
        GBUFFERS_MATRICES.to_owned()
    };
    let mut v = Stage::scan(vertex, ShaderStage::Vertex);
    let mut f = Stage::scan(fragment, ShaderStage::Fragment);
    v.gbuffers = gbuffers.is_some();
    f.gbuffers = gbuffers.is_some();
    let mut errors: Vec<Error> = v.errors.drain(..).chain(f.errors.drain(..)).collect();

    // One uniform block and one set of samplers for both stages.
    let mut uniforms: BTreeMap<String, String> = BTreeMap::new();
    if gbuffers.is_some() {
        for name in [
            "gbufferModelView",
            "gbufferModelViewInverse",
            "gbufferProjection",
            "gbufferProjectionInverse",
        ] {
            uniforms.insert(name.to_owned(), "mat4".to_owned());
            if shadow {
                uniforms.insert(name.replace("gbuffer", "shadow"), "mat4".to_owned());
            }
        }
    }
    let mut samplers: BTreeMap<String, String> = BTreeMap::new();
    for stage in [&v, &f] {
        for (name, ty, at) in &stage.uniforms {
            if let Some(old) = uniforms.insert(name.clone(), ty.clone())
                && &old != ty
            {
                errors.push(stage.error(
                    *at,
                    &format!("`{name}` is `{old}` in one stage and `{ty}` in the other"),
                ));
            }
        }
        for (name, ty) in &stage.samplers {
            samplers.insert(name.clone(), ty.clone());
        }
    }
    // `alphaTestRef` is the program's alpha test reference: a constant of the program.
    let alpha_test_ref = gbuffers
        .filter(|_| uniforms.get("alphaTestRef").is_some_and(|t| t == "float"))
        .map(|g| g.alpha_test.map_or(0.0, |t| t.reference));
    if alpha_test_ref.is_some() {
        uniforms.remove("alphaTestRef");
    }
    // Per-draw values move to their own block; another type than the documented one keeps
    // the frame's block (and reads zero).
    let mut draw = false;
    if gbuffers.is_some() {
        for (name, ty) in DRAW_UNIFORMS {
            if uniforms.get(name).is_some_and(|t| t == ty) {
                uniforms.remove(name);
                draw = true;
            }
        }
    }
    let samplers: Vec<Sampler> = samplers
        .into_iter()
        .enumerate()
        .map(|(i, (name, ty))| Sampler {
            name,
            ty,
            binding: 1 + 2 * i as u32,
        })
        .collect();
    let images = bind_images([&v, &f], samplers.len());

    // Varyings by name: the vertex outputs, then fragment inputs the vertex stage lacks. A
    // matrix or array travels as one varying per column or element (`varying_parts`), a
    // struct as its members packed into vectors (`struct_parts`).
    let mut locations: BTreeMap<String, (u32, String)> = BTreeMap::new();
    let mut split: BTreeMap<String, (String, Vec<(String, String)>)> = BTreeMap::new();
    let vars = v.outs.iter().map(|o| (&v, o));
    for (stage, var) in vars.chain(f.ins.iter().map(|i| (&f, i))) {
        if locations.contains_key(&var.name) || split.contains_key(&var.name) {
            continue;
        }
        if var.structure {
            match struct_parts(&var.decl, &stage.structs) {
                Some(parts) => {
                    let mut copies = Vec::new();
                    for (k, (decl, members)) in parts.into_iter().enumerate() {
                        let part = format!("hz_v_{}_{k}", var.name);
                        let next = locations.len() as u32;
                        locations.insert(part.clone(), (next, decl));
                        for (swizzle, access) in members {
                            copies.push((format!("{part}{swizzle}"), access));
                        }
                    }
                    split.insert(var.name.clone(), (String::new(), copies));
                }
                None => errors.push(Error {
                    file: stage.pp.files.first().cloned().unwrap_or_default(),
                    line: 0,
                    message: format!("`{}`: this struct can't be a varying", var.name),
                }),
            }
            continue;
        }
        match varying_parts(&var.decl) {
            Some((global, parts)) => {
                let mut copies = Vec::new();
                for (k, (decl, access)) in parts.into_iter().enumerate() {
                    let part = format!("hz_v_{}_{k}", var.name);
                    let next = locations.len() as u32;
                    locations.insert(part.clone(), (next, decl));
                    copies.push((part, access));
                }
                split.insert(var.name.clone(), (global, copies));
            }
            None => {
                let next = locations.len() as u32;
                locations.insert(var.name.clone(), (next, var.decl.clone()));
            }
        }
    }

    let header = |stage: &Stage| {
        let mut h = resources(&uniforms, &samplers, &images);
        if let Some(reference) = alpha_test_ref {
            writeln!(h, "const float alphaTestRef = {reference:?};").unwrap();
        }
        if draw {
            h.push_str(
                "layout(set = 2, binding = 0) uniform HzDraw { int renderStage; int entityId; \
                 int blockEntityId; int hz_draw_pad; vec4 entityColor; };\n",
            );
        }
        let direction = if stage.kind == ShaderStage::Vertex {
            "out"
        } else {
            "in"
        };
        for (name, (location, decl)) in &locations {
            writeln!(
                h,
                "layout(location = {location}) {direction} {}",
                decl.replace("@", name)
            )
            .unwrap();
        }
        for (name, (global, _)) in split.iter().filter(|(_, (g, _))| !g.is_empty()) {
            writeln!(h, "{}", global.replace("@", name)).unwrap();
        }
        if let (Some(inputs), ShaderStage::Vertex) = (gbuffers, stage.kind) {
            h.push_str(&inputs.declarations);
            h.push('\n');
        }
        h.push_str(&stage.prelude());
        h
    };
    let flip = if target == Target::Buffers { "-" } else { "" };
    let fog =
        "    hz_Fog = hz_FogParameters(hz_fog_color, hz_fog.x, hz_fog.y, hz_fog.z, hz_fog.w);\n";
    let mut vertex_main = String::from("\nvoid main() {\n");
    match gbuffers {
        None => vertex_main.push_str(
            "    vec2 hz_p = hz_quad[gl_VertexIndex];\n    hz_Vertex = vec4(hz_p, 0.0, 1.0);\n    \
             hz_MultiTexCoord0 = hz_Vertex;\n    hz_Color = vec4(1.0);\n    hz_Normal = vec3(0.0, 0.0, 1.0);\n",
        ),
        Some(inputs) => {
            vertex_main.push_str(&matrices);
            vertex_main.push_str(&inputs.body);
        }
    }
    for (name, value, ty) in &v.attributes {
        let value = match gbuffers {
            // Our quad's values for a composite.
            None => value.clone(),
            // The game's, or zero for anything it doesn't provide.
            Some(inputs) => match inputs.attributes.iter().find(|a| &a.0 == name) {
                Some((_, given)) => format!("{ty}({given})"),
                None => format!("{ty}(0)"),
            },
        };
        writeln!(vertex_main, "    {name} = {value};").unwrap();
    }
    vertex_main.push_str(fog);
    for name in &v.inits {
        writeln!(vertex_main, "    {name} = hz_init_{name}();").unwrap();
    }
    vertex_main.push_str("    hz_main();\n");
    // A struct's own declaration is the pack's: only a stage that declares it copies it.
    let declared = |list: &[scan::Var], name: &str, global: &str| {
        !global.is_empty() || list.iter().any(|var| var.name == name)
    };
    for (name, (global, copies)) in &split {
        if !declared(&v.outs, name, global) {
            continue;
        }
        for (part, access) in copies {
            writeln!(vertex_main, "    {part} = {name}{access};").unwrap();
        }
    }
    write!(
        vertex_main,
        "    gl_Position = vec4(gl_Position.x, {flip}gl_Position.y, (gl_Position.z + gl_Position.w) * 0.5, gl_Position.w);\n{}}}\n",
        gbuffers.map_or("", |g| &g.after)
    )
    .unwrap();
    let mut fragment_main = String::from("\nvoid main() {\n");
    if gbuffers.is_some() {
        fragment_main.push_str(&matrices);
    }
    fragment_main.push_str(fog);
    if f.frag_coord {
        fragment_main.push_str(match target {
            Target::Window => "    hz_FragCoord = vec4(gl_FragCoord.x, hz_target.y - gl_FragCoord.y, gl_FragCoord.zw);\n",
            Target::Buffers => "    hz_FragCoord = gl_FragCoord;\n",
        });
    }
    for (name, (global, copies)) in &split {
        if !declared(&f.ins, name, global) {
            continue;
        }
        for (part, access) in copies {
            writeln!(fragment_main, "    {name}{access} = {part};").unwrap();
        }
    }
    for name in &f.inits {
        writeln!(fragment_main, "    {name} = hz_init_{name}();").unwrap();
    }
    fragment_main.push_str("    hz_main();\n");
    for copy in &f.out_copies {
        writeln!(fragment_main, "    {copy}").unwrap();
    }
    // With `out` variables the pack tests alpha itself.
    if let Some(t) = gbuffers.and_then(|g| g.alpha_test)
        && f.outputs.contains(&0)
        && !f.frag_outs.iter().any(|(n, _)| *n == 0)
    {
        match t.op {
            "false" => fragment_main.push_str("    discard;\n"),
            op => writeln!(
                fragment_main,
                "    if (!(hz_out0.a {op} {:?})) {{ discard; }}",
                t.reference
            )
            .unwrap(),
        }
    }
    fragment_main.push_str("}\n");

    let v_text = format!(
        "{}{}{vertex_main}",
        header(&v),
        v.rewrite(&samplers, &uniforms)
    );
    let f_text = format!(
        "{}{}{fragment_main}",
        header(&f),
        f.rewrite(&samplers, &uniforms)
    );
    if !errors.is_empty() {
        return Err(errors);
    }
    let vertex_module = compile(&v, &v_text, header(&v).lines().count());
    let fragment_module = compile(&f, &f_text, header(&f).lines().count());
    let (vertex, fragment) = match (vertex_module, fragment_module) {
        (Ok(v), Ok(f)) => (v, f),
        (a, b) => return Err(a.err().into_iter().chain(b.err()).flatten().collect()),
    };
    let (uniform_size, offsets) = block_layout(&fragment);
    let uniforms = offsets
        .into_iter()
        .filter_map(|(member, offset)| {
            let name = member
                .strip_prefix("hz_bool_")
                .unwrap_or(&member)
                .to_owned();
            let ty = if name.starts_with("hz_") {
                "vec4".to_owned()
            } else {
                uniforms.get(&name)?.clone()
            };
            Some(Uniform { name, ty, offset })
        })
        .collect();
    let outputs = f.outputs.iter().copied().collect();
    let mut images = images;
    mark_atomic(&vertex, &mut images);
    mark_atomic(&fragment, &mut images);
    Ok(Program {
        interface: Interface {
            uniform_size,
            uniforms,
            samplers,
            outputs,
            draw,
            images,
            buffers: v.buffers.union(&f.buffers).copied().collect(),
        },
        vertex,
        fragment,
    })
}

/// Images an atomic function uses (naga gave them atomic access) are bound for atomics.
fn mark_atomic(module: &naga::Module, images: &mut [Image]) {
    for (_, g) in module.global_variables.iter() {
        let naga::TypeInner::Image {
            class: naga::ImageClass::Storage { access, .. },
            ..
        } = module.types[g.ty].inner
        else {
            continue;
        };
        if let Some(b) = &g.binding
            && access.contains(naga::StorageAccess::ATOMIC)
            && let Some(i) = images.iter_mut().find(|i| i.binding == b.binding)
        {
            i.access = "atomic".into();
        }
    }
}

/// The images of `stages`, by name, bound after `samplers` samplers.
fn bind_images(stages: [&Stage; 2], samplers: usize) -> Vec<Image> {
    let mut images: BTreeMap<String, scan::ImageDecl> = BTreeMap::new();
    for stage in stages {
        for i in &stage.images {
            images.entry(i.name.clone()).or_insert_with(|| i.clone());
        }
    }
    images
        .into_values()
        .enumerate()
        .map(|(k, i)| Image {
            name: i.name,
            ty: i.ty,
            format: i.format,
            access: i.access,
            binding: 1 + 2 * samplers as u32 + k as u32,
            before: None,
        })
        .collect()
}

/// `#version 450`, the uniform block at binding 0, each sampler as a texture and a sampler
/// object, and the images.
fn resources(
    uniforms: &BTreeMap<String, String>,
    samplers: &[Sampler],
    images: &[Image],
) -> String {
    let mut h = String::from("#version 450\n");
    h.push_str("layout(set = 0, binding = 0) uniform HzUniforms { vec4 hz_target; vec4 hz_fog_color; vec4 hz_fog;");
    for (name, ty) in uniforms {
        match ty.as_str() {
            "bool" => write!(h, " int hz_bool_{name};"),
            _ => match ty.split_once('[') {
                Some((base, size)) => write!(h, " {base} {name}[{size};"),
                None => write!(h, " {ty} {name};"),
            },
        }
        .unwrap();
    }
    h.push_str(" };\n");
    for s in samplers {
        let (texture, sampler) = split_sampler(&s.ty);
        writeln!(
            h,
            "layout(set = 0, binding = {}) uniform {texture} {}; layout(set = 0, binding = {}) uniform {sampler} hz_s_{};",
            s.binding, s.name, s.binding + 1, s.name
        )
        .unwrap();
    }
    for i in images {
        writeln!(
            h,
            "layout(set = 0, binding = {}, {}) uniform {} {} {};",
            i.binding, i.format, i.access, i.ty, i.name
        )
        .unwrap();
    }
    h
}

/// Translates a compute program (`.csh`).
pub fn translate_compute(compute: &Preprocessed) -> Result<ComputeProgram, Vec<Error>> {
    let split = atomic_vectors::rewrite(compute);
    let compute = split.as_ref().unwrap_or(compute);
    let mut c = Stage::scan(compute, ShaderStage::Compute);
    let mut errors = std::mem::take(&mut c.errors);
    let mut uniforms: BTreeMap<String, String> = BTreeMap::new();
    for (name, ty, _) in &c.uniforms {
        uniforms.insert(name.clone(), ty.clone());
    }
    let samplers: Vec<Sampler> = c
        .samplers
        .iter()
        .map(|(name, ty)| (name, ty))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .enumerate()
        .map(|(i, (name, ty))| Sampler {
            name: name.clone(),
            ty: ty.clone(),
            binding: 1 + 2 * i as u32,
        })
        .collect();
    let mut images = bind_images([&c, &c], samplers.len());
    let code = c.rewrite(&samplers, &uniforms);
    let code = work_group_size(&code).unwrap_or_else(|message| {
        errors.push(Error {
            file: compute.files.first().cloned().unwrap_or_default(),
            line: 0,
            message,
        });
        code
    });
    let code = ordering::barriers_after_atomics(&code).unwrap_or(code);
    let declared: Vec<(String, bool)> = c
        .images
        .iter()
        .map(|i| (i.name.clone(), i.coherent))
        .collect();
    let before = ordering::snapshots(&code, &declared);
    let code = ordering::read_before(&code, &before);
    for name in before {
        let image = images.iter().find(|i| i.name == name).expect("declared");
        let copy = Image {
            name: ordering::before_name(&name),
            ty: image.ty.clone(),
            format: image.format.clone(),
            access: String::new(),
            binding: 1 + 2 * samplers.len() as u32 + images.len() as u32,
            before: Some(name),
        };
        images.push(copy);
    }
    let mut main = String::from("\nvoid main() {\n");
    main.push_str(
        "    hz_Fog = hz_FogParameters(hz_fog_color, hz_fog.x, hz_fog.y, hz_fog.z, hz_fog.w);\n",
    );
    for name in &c.inits {
        writeln!(main, "    {name} = hz_init_{name}();").unwrap();
    }
    main.push_str("    hz_main();\n}\n");
    let header = resources(&uniforms, &samplers, &images) + &c.prelude();
    let text = format!("{header}{code}{main}");
    // `workGroups` is an ivec3, `workGroupsRender` a vec2 (a third number, the depth of a 2D
    // render, is taken as 1); anything else is not understood rather than guessed.
    let mut sizes = |name: &str, counts: &[usize]| {
        let v = constant(&c, name)?;
        if !counts.contains(&v.len()) {
            errors.push(Error {
                file: compute.files.first().cloned().unwrap_or_default(),
                line: 0,
                message: format!("`{name}` must have {counts:?} numbers, not {}", v.len()),
            });
            return None;
        }
        Some(v)
    };
    let work_groups = sizes("workGroups", &[3]).map(|v| [v[0] as u32, v[1] as u32, v[2] as u32]);
    let work_groups_render =
        sizes("workGroupsRender", &[2, 3]).map_or([1.0, 1.0], |v| [v[0] as f32, v[1] as f32]);
    if !errors.is_empty() {
        return Err(errors);
    }
    let module = compile(&c, &text, header.lines().count()).map_err(|e| {
        errors.extend(e);
        std::mem::take(&mut errors)
    })?;
    let (uniform_size, offsets) = block_layout(&module);
    let mut interface = Interface {
        uniform_size,
        uniforms: offsets
            .into_iter()
            .filter_map(|(member, offset)| {
                let name = member
                    .strip_prefix("hz_bool_")
                    .unwrap_or(&member)
                    .to_owned();
                let ty = if name.starts_with("hz_") {
                    "vec4".to_owned()
                } else {
                    uniforms.get(&name)?.clone()
                };
                Some(Uniform { name, ty, offset })
            })
            .collect(),
        samplers,
        outputs: Vec::new(),
        draw: false,
        images,
        buffers: c.buffers.iter().copied().collect(),
    };
    mark_atomic(&module, &mut interface.images);
    Ok(ComputeProgram {
        module,
        interface,
        work_groups,
        work_groups_render,
    })
}

/// `code` with `gl_WorkGroupSize` written as the constant its `layout(local_size_*)` declares:
/// GLSL defines it so, and naga's HLSL writer has no such built-in.
fn work_group_size(code: &str) -> Result<String, String> {
    let tokens = crate::macros::lex(code);
    if !tokens.contains(&"gl_WorkGroupSize") {
        return Ok(code.to_owned());
    }
    let sig: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|t| scan::significant(t))
        .collect();
    // A size is one number, the whole of its `local_size_* = N`; 1 when not declared.
    let size = |axis: &str| match sig.windows(4).find(|w| w[0] == axis && w[1] == "=") {
        None => Ok(1),
        Some(w) => w[2]
            .trim_end_matches(['u', 'U'])
            .parse::<u32>()
            .ok()
            .filter(|_| matches!(w[3], "," | ")"))
            .ok_or_else(|| format!("`{axis}` is not a number gl_WorkGroupSize can take")),
    };
    let value = format!(
        "uvec3({}u, {}u, {}u)",
        size("local_size_x")?,
        size("local_size_y")?,
        size("local_size_z")?
    );
    Ok(tokens
        .iter()
        .map(|&t| {
            if t == "gl_WorkGroupSize" {
                value.as_str()
            } else {
                t
            }
        })
        .collect())
}

/// The numbers of a constant's constructor, `const ivec3 name = ivec3(1, 2, 3);` → 1, 2, 3.
fn constant(stage: &Stage, name: &str) -> Option<Vec<f64>> {
    let at = stage.tokens.iter().position(|&t| t == name)?;
    let rest = &stage.tokens[at + 1..];
    if rest.iter().find(|t| scan::significant(t)) != Some(&"=") {
        return None;
    }
    let end = rest.iter().position(|&t| t == ";")?;
    Some(
        rest[..end]
            .iter()
            .filter_map(|t| t.trim_end_matches(['u', 'U', 'f', 'F']).parse().ok())
            .collect(),
    )
}

/// A matrix or array varying (`flat mat3 @[2];`): wgpu's stage interfaces take only scalars
/// and vectors (WGSL), so it becomes a private global (`mat3 @[2];`) and one varying per
/// column or element, each with the qualifiers and how to reach it (`[1][2]`). `None` for
/// scalars and vectors, and for sizes that are not plain numbers.
fn varying_parts(decl: &str) -> Option<(String, Vec<(String, String)>)> {
    let (before, after) = decl.split_once('@')?;
    let mut words: Vec<&str> = before.split_whitespace().collect();
    let ty = words.pop()?;
    let qualifiers: String = words.iter().map(|w| format!("{w} ")).collect();
    let array = after.trim().trim_end_matches(';').trim();
    let mut sizes = Vec::new();
    let mut rest = array;
    while let Some(r) = rest.strip_prefix('[') {
        let (n, r) = r.split_once(']')?;
        sizes.push(n.trim().parse::<usize>().ok()?);
        rest = r.trim_start();
    }
    if !rest.is_empty() {
        return None;
    }
    // matC or matCxR: C columns of R rows.
    let column = ty.strip_prefix("mat").and_then(|d| {
        let (c, r) = d.split_once('x').unwrap_or((d, d));
        Some((c.parse::<usize>().ok()?, r.parse::<usize>().ok()?))
    });
    if sizes.is_empty() && column.is_none() {
        return None;
    }
    let part_ty = column.map_or(ty.to_owned(), |(_, r)| format!("vec{r}"));
    if let Some((c, _)) = column {
        sizes.push(c);
    }
    let count: usize = sizes.iter().product();
    let parts = (0..count)
        .map(|mut k| {
            let mut access = String::new();
            for &size in sizes.iter().rev() {
                access.insert_str(0, &format!("[{}]", k % size));
                k /= size;
            }
            (format!("{qualifiers}{part_ty} @;"), access)
        })
        .collect();
    Some((format!("{ty} @{array};"), parts))
}

/// A struct varying (`flat Params @;`) as vectors: its scalar, vector and matrix members, through
/// nested structs and arrays, packed four components at a time by base type. Each part: its
/// declaration and, per member, the swizzle of the part and how to reach the member (`.a[1].b`).
/// `None` for a member that can't travel (`bool`, an unknown type, a size that is not a number).
#[allow(clippy::type_complexity)]
fn struct_parts(
    decl: &str,
    structs: &std::collections::HashMap<String, Vec<(String, String)>>,
) -> Option<Vec<(String, Vec<(String, String)>)>> {
    let (before, after) = decl.split_once('@')?;
    let mut words: Vec<&str> = before.split_whitespace().collect();
    let ty = words.pop()?;
    let qualifiers: String = words.iter().map(|w| format!("{w} ")).collect();
    let mut leaves = Vec::new();
    leaves_of(
        ty,
        after.trim().trim_end_matches(';'),
        String::new(),
        structs,
        &mut leaves,
    )?;
    // Slots of one base type and up to four components: (base, used, members (from, size, access)).
    let mut slots: Vec<(&str, usize, Vec<(usize, usize, String)>)> = Vec::new();
    for (base, size, access) in leaves {
        match slots
            .iter_mut()
            .find(|(b, used, _)| *b == base && used + size <= 4)
        {
            Some(slot) => {
                slot.2.push((slot.1, size, access));
                slot.1 += size;
            }
            None => slots.push((base, size, vec![(0, size, access)])),
        }
    }
    Some(
        slots
            .into_iter()
            .map(|(base, used, members)| {
                let ty = match (base, used) {
                    (b, 1) => b.to_owned(),
                    ("float", n) => format!("vec{n}"),
                    ("int", n) => format!("ivec{n}"),
                    (_, n) => format!("uvec{n}"),
                };
                // Integers only travel flat.
                let flat = if base != "float" && !qualifiers.contains("flat") {
                    "flat "
                } else {
                    ""
                };
                let members = members
                    .into_iter()
                    .map(|(from, size, access)| {
                        let swizzle = if used == 1 {
                            String::new()
                        } else {
                            format!(".{}", &"xyzw"[from..from + size])
                        };
                        (swizzle, access)
                    })
                    .collect();
                (format!("{flat}{qualifiers}{ty} @;"), members)
            })
            .collect(),
    )
}

/// The scalars and vectors inside a value of type `ty` with array suffix `array`, reached by
/// `access`: (base type, components, access).
fn leaves_of(
    ty: &str,
    array: &str,
    access: String,
    structs: &std::collections::HashMap<String, Vec<(String, String)>>,
    out: &mut Vec<(&'static str, usize, String)>,
) -> Option<()> {
    if let Some(rest) = array.trim().strip_prefix('[') {
        let (n, rest) = rest.split_once(']')?;
        for k in 0..n.trim().parse::<usize>().ok()? {
            leaves_of(ty, rest, format!("{access}[{k}]"), structs, out)?;
        }
        return Some(());
    }
    if let Some(members) = structs.get(ty) {
        for (member_ty, member) in members {
            let (name, array) = member
                .split_once('[')
                .map_or((member.as_str(), ""), |(n, _)| (n, &member[n.len()..]));
            leaves_of(member_ty, array, format!("{access}.{name}"), structs, out)?;
        }
        return Some(());
    }
    if let Some(dims) = ty.strip_prefix("mat") {
        let (c, r) = dims.split_once('x').unwrap_or((dims, dims));
        let (c, r) = (c.parse::<usize>().ok()?, r.parse::<usize>().ok()?);
        for k in 0..c {
            out.push(("float", r, format!("{access}[{k}]")));
        }
        return Some(());
    }
    let (base, size) = match ty {
        "float" => ("float", 1),
        "int" => ("int", 1),
        "uint" => ("uint", 1),
        _ => {
            let (base, n) = if let Some(n) = ty.strip_prefix("vec") {
                ("float", n)
            } else if let Some(n) = ty.strip_prefix("ivec") {
                ("int", n)
            } else {
                ("uint", ty.strip_prefix("uvec")?)
            };
            (base, n.parse().ok()?)
        }
    };
    out.push((base, size, access));
    Some(())
}

/// `sampler2DShadow` → (`texture2D`, `samplerShadow`); `isampler3D` → (`itexture3D`, `sampler`).
pub(super) fn split_sampler(ty: &str) -> (String, &'static str) {
    let shadow = ty.ends_with("Shadow");
    let base = ty
        .trim_end_matches("Shadow")
        .replacen("sampler", "texture", 1);
    (base, if shadow { "samplerShadow" } else { "sampler" })
}

/// Parses and validates one stage; errors at the pack's files and lines.
fn compile(stage: &Stage, text: &str, header_lines: usize) -> Result<naga::Module, Vec<Error>> {
    // `text` is the source the span is in: the rewrites below keep its lines.
    let at = |text: &str, span: naga::Span, message: String| {
        let start = span.to_range().map_or(0, |r| r.start);
        let line = text[..start.min(text.len())].matches('\n').count();
        match line
            .checked_sub(header_lines)
            .and_then(|l| stage.pp.lines.get(l))
        {
            Some(o) => Error {
                file: stage
                    .pp
                    .files
                    .get(o.file as usize)
                    .cloned()
                    .unwrap_or_default(),
                line: o.line,
                message,
            },
            None => Error {
                file: "<horizon>".into(),
                line: line as u32 + 1,
                message,
            },
        }
    };
    let parse = |text: &str| {
        Frontend::default()
            .parse(&Options::from(stage.kind), text)
            .map_err(|e| {
                e.errors
                    .into_iter()
                    .map(|e| at(text, e.meta, e.kind.to_string()))
                    .collect::<Vec<_>>()
            })
    };
    // naga's frontend refuses a few things GL takes: a scalar's swizzle (`f.x`, GLSL 4.20), a
    // matrix column's component written to (`m[1].y =`) and `min`/`max`/`clamp` of a `uint` and
    // integer literals. Each refusal is rewritten where naga points, and the source parsed again.
    let mut text = std::borrow::Cow::Borrowed(text);
    let mut tries = 0;
    let module = loop {
        tries += 1;
        match Frontend::default().parse(&Options::from(stage.kind), &text) {
            Ok(m) => break m,
            Err(e) => match (e.errors.iter())
                .find_map(|e| scalar_swizzle(&text, e).or_else(|| unsigned_literals(&text, e)))
            {
                Some(fixed) if tries < 256 => text = fixed.into(),
                _ => return parse(&text),
            },
        }
    };
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .subgroup_operations(naga::valid::SubgroupOperationSet::all())
        .subgroup_stages(naga::valid::ShaderStages::all())
        .validate(&module)
        .map_err(|e| {
            let span = e.spans().next().map_or(naga::Span::default(), |(s, _)| *s);
            // The validator nests its reasons: say them all.
            let inner = e.into_inner();
            let mut message = inner.to_string();
            let mut source = std::error::Error::source(&inner);
            while let Some(s) = source {
                write!(message, ": {s}").unwrap();
                source = s.source();
            }
            vec![at(&text, span, message)]
        })?;
    Ok(module)
}

/// `text` without the scalar swizzle naga's frontend refused (`e`, GLSL 4.20's `f.x`, `f.xx`): one
/// component dropped, more a vector constructor of the scalar.
fn scalar_swizzle(text: &str, e: &naga::front::glsl::Error) -> Option<String> {
    if !e
        .kind
        .to_string()
        .starts_with("Can't lookup field on this type")
    {
        return None;
    }
    let r = e.meta.to_range()?;
    let field = text.get(r.clone())?.strip_prefix('.')?;
    let one = |c: char| field.chars().all(|f| f == c);
    let indexed = text[..r.start].trim_end().ends_with(']');
    let component = field.len() == 1 && "xyzwrgbastpq".contains(field);
    if field.is_empty()
        || field.len() > 4
        || !(one('x') || one('r') || one('s') || indexed && component)
    {
        return None;
    }
    let mut out = text.to_owned();
    if field.len() == 1 && indexed {
        // A matrix column's component, written to: `m[1].y` as `m[1][1]`.
        let k = "xyzwrgbastpq".find(field).map(|k| k % 4)?;
        out.replace_range(r, &format!("[{k}]"));
    } else if field.len() == 1 {
        out.replace_range(r, "");
    } else {
        let start = expression_start(text, r.start)?;
        out.replace_range(r, ")");
        out.insert_str(start, &format!("vec{}(", field.len()));
    }
    Some(out)
}

/// `text` with the integer literals of the `min`/`max`/`clamp` call naga found ambiguous (`e`)
/// made unsigned: with a `uint` beside them GL takes the `uint` overload.
fn unsigned_literals(text: &str, e: &naga::front::glsl::Error) -> Option<String> {
    let kind = e.kind.to_string();
    let f = kind
        .strip_prefix("Ambiguous best function for '")?
        .trim_end_matches('\'');
    if !matches!(f, "min" | "max" | "clamp") {
        return None;
    }
    let r = e.meta.to_range()?;
    let call = text.get(r.clone())?;
    let b = call.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'.';
    let mut out = String::with_capacity(call.len() + 8);
    let mut i = 0;
    let mut changed = false;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !word(b[i - 1])) {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push_str(&call[start..i]);
            if i == b.len() || !(word(b[i])) {
                out.push('u');
                changed = true;
            }
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    changed.then(|| format!("{}{out}{}", &text[..r.start], &text[r.end..]))
}

/// Where the operand that ends right before byte `end` begins: a name, a call or an index (with
/// what it is of), or a bracketed expression.
fn expression_start(text: &str, end: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = end;
    while i > 0 && b[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    loop {
        match b.get(i.checked_sub(1)?) {
            Some(b')') | Some(b']') => {
                let mut level = 0i32;
                loop {
                    i = i.checked_sub(1)?;
                    match b[i] {
                        b')' | b']' => level += 1,
                        b'(' | b'[' => {
                            level -= 1;
                            if level == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some(c) if c.is_ascii_alphanumeric() || *c == b'_' || *c == b'.' => i -= 1,
            _ => return Some(i),
        }
        if i == 0 {
            return Some(0);
        }
    }
}

/// The uniform block's size and member offsets, from naga's layout.
fn block_layout(module: &naga::Module) -> (u32, Vec<(String, u32)>) {
    for (_, var) in module.global_variables.iter() {
        let frame = var
            .binding
            .as_ref()
            .is_some_and(|b| b.group == 0 && b.binding == 0);
        if var.space != naga::AddressSpace::Uniform || !frame {
            continue;
        }
        if let naga::TypeInner::Struct { members, span } = &module.types[var.ty].inner {
            let offsets = members
                .iter()
                .filter_map(|m| Some((m.name.clone()?, m.offset)))
                .collect();
            return (*span, offsets);
        }
    }
    (16, vec![("hz_target".into(), 0)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;
    use crate::{Macros, Pack, preprocess};

    fn program(files: &[(&str, &str)], target: Target) -> Result<Program, Vec<Error>> {
        let files: Vec<(String, &str)> = files
            .iter()
            .map(|(p, t)| (format!("shaders/{p}"), *t))
            .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        let m = Macros::standard();
        translate_composite(
            &preprocess(&pack, "composite.vsh", &m),
            &preprocess(&pack, "composite.fsh", &m),
            target,
        )
    }

    const VSH_120: &str = "#version 120\nvarying vec2 texcoord;\nvoid main() {\n    gl_Position = ftransform();\n    \
                           texcoord = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;\n}\n";

    #[test]
    fn compatibility_programs_translate() {
        let fsh = "#version 120\nuniform sampler2D colortex0;\nuniform sampler2D depthtex0;\nuniform float viewWidth;\n\
                   uniform bool hideGUI;\nuniform int frameCounter;\nvarying vec2 texcoord;\n\
                   vec3 tint(sampler2D s, vec2 c) { return texture2D(s, c).rgb; }\nvoid main() {\n    \
                   vec3 c = tint(colortex0, texcoord) * texture2D(depthtex0, texcoord).r;\n    if (hideGUI) c *= 0.5;\n    \
                   /* DRAWBUFFERS:04 */\n    gl_FragData[0] = vec4(c, 1.0);\n    gl_FragData[1] = vec4(frameCounter / viewWidth);\n}\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap();
        let i = &p.interface;
        let samplers: Vec<(&str, u32)> = i
            .samplers
            .iter()
            .map(|s| (s.name.as_str(), s.binding))
            .collect();
        assert_eq!(samplers, [("colortex0", 1), ("depthtex0", 3)]);
        let uniforms: Vec<(&str, &str)> = i
            .uniforms
            .iter()
            .map(|u| (u.name.as_str(), u.ty.as_str()))
            .collect();
        assert_eq!(
            uniforms,
            [
                ("hz_target", "vec4"),
                ("hz_fog_color", "vec4"),
                ("hz_fog", "vec4"),
                ("frameCounter", "int"),
                ("hideGUI", "bool"),
                ("viewWidth", "float")
            ]
        );
        assert_eq!(i.uniforms[0].offset, 0);
        assert!(i.uniform_size >= 28);
        assert_eq!(i.outputs, [0, 1]);
    }

    /// naga drops non-constant global initializers: they run in our `main`, one per declarator,
    /// converting as GLSL's initialization does.
    #[test]
    fn global_initializers_read_uniforms() {
        let fsh = "#version 120\nuniform float rainStrength;\nuniform ivec2 atlasSize;\n\
                   vec2 size = atlasSize;\nvec3 a = vec3(rainStrength), b = a * 2.0, c;\n\
                   const float k = rainStrength > 0.5 ? 1.0 : 0.0;\nfloat w[] = float[](rainStrength, 1.0);\n\
                   void main() { c = b; gl_FragColor = vec4(a + c, size.x * k + w[0]); }\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Window,
        );
        assert!(p.is_ok(), "{:?}", p.err());
    }

    #[test]
    fn core_programs_translate() {
        let vsh = "#version 330 core\nin vec3 vaPosition;\nin vec2 vaUV0;\nuniform mat4 modelViewMatrix, projectionMatrix;\n\
                   out vec2 uv;\nflat out int flatId;\nvoid main() {\n    \
                   gl_Position = projectionMatrix * modelViewMatrix * vec4(vaPosition, 1.0);\n    uv = vaUV0;\n    flatId = 1;\n}\n";
        let fsh = "#version 330 core\nin vec2 uv;\nuniform sampler2D colortex0;\nuniform usampler2D colortex5;\n\
                   layout(location = 2) out vec4 color;\nvoid main() {\n    \
                   if (texture(colortex0, uv).a < 0.1) discard;\n    color = texture(colortex0, uv) + float(texelFetch(colortex5, ivec2(gl_FragCoord.xy), 0).r);\n}\n";
        let p = program(
            &[("composite.vsh", vsh), ("composite.fsh", fsh)],
            Target::Window,
        )
        .unwrap();
        assert_eq!(p.interface.outputs, [2]);
        assert!(
            p.interface
                .uniforms
                .iter()
                .any(|u| u.name == "projectionMatrix" && u.ty == "mat4")
        );
    }

    #[test]
    fn frag_coord_is_set_for_every_target() {
        let fsh = "#version 120\nvoid main() { gl_FragColor = vec4(gl_FragCoord.x); }\n";
        for target in [Target::Buffers, Target::Window] {
            let p = program(
                &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
                target,
            )
            .unwrap();
            let m = &p.fragment;
            let (handle, _) = m
                .global_variables
                .iter()
                .find(|(_, g)| g.name.as_deref() == Some("hz_FragCoord"))
                .unwrap();
            // naga calls our `main` from its entry point: look in every function.
            let functions = m.functions.iter().map(|(_, f)| f);
            let stores = functions.chain([&m.entry_points[0].function]).any(|f| {
                f.body.iter().any(|s| match s {
                    naga::Statement::Store { pointer, .. } => {
                        matches!(f.expressions[*pointer], naga::Expression::GlobalVariable(h) if h == handle)
                    }
                    _ => false,
                })
            });
            assert!(stores, "{target:?}: main does not set hz_FragCoord");
        }
    }

    /// Code after an unconditional `discard` (BSL's `gbuffers_clouds` with its own clouds on)
    /// or `return;` still makes a valid module.
    #[test]
    fn code_after_discard_or_return_translates() {
        let vsh = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
        for fsh in [
            "#version 120\nuniform sampler2D texture;\nvoid main() {\n    discard;\n    vec4 c = texture2D(texture, vec2(0.5));\n    gl_FragData[0] = c;\n}\n",
            "#version 120\nvoid main() {\n    gl_FragData[0] = vec4(1.0);\n    return;\n    gl_FragData[0] = vec4(0.5);\n}\n",
        ] {
            let files = [
                ("shaders/gbuffers_clouds.vsh", vsh),
                ("shaders/gbuffers_clouds.fsh", fsh),
            ];
            let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
            let m = Macros::standard();
            let inputs = GbuffersInputs {
                body: "    hz_Vertex = vec4(0.0, 0.0, 0.0, 1.0);\n".into(),
                ..Default::default()
            };
            translate_gbuffers(
                &preprocess(&pack, "gbuffers_clouds.vsh", &m),
                &preprocess(&pack, "gbuffers_clouds.fsh", &m),
                Target::Buffers,
                &inputs,
            )
            .unwrap_or_else(|e| panic!("{fsh}: {e:?}"));
        }
    }

    #[test]
    fn the_alpha_test_discards_below_its_reference() {
        assert_eq!(
            AlphaTest::parse("GREATER 0.1"),
            Some(Some(AlphaTest {
                op: ">",
                reference: 0.1
            }))
        );
        assert_eq!(AlphaTest::parse("off"), Some(None));
        assert_eq!(AlphaTest::parse("GL_ALWAYS 0"), Some(None));
        assert_eq!(AlphaTest::parse("SOMETIMES 0.5"), None);
        let vsh = "#version 120\nvoid main() { gl_Position = ftransform(); }\n";
        let legacy = "#version 120\nvoid main() { gl_FragData[0] = vec4(0.5); }\n";
        let outs =
            "#version 330\nlayout(location = 0) out vec4 c;\nvoid main() { c = vec4(0.5); }\n";
        for (fsh, kills) in [(legacy, true), (outs, false)] {
            let files = [
                ("shaders/gbuffers_entities.vsh", vsh),
                ("shaders/gbuffers_entities.fsh", fsh),
            ];
            let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
            let m = Macros::standard();
            let inputs = GbuffersInputs {
                body: "    hz_Vertex = vec4(0.0, 0.0, 0.0, 1.0);\n".into(),
                alpha_test: AlphaTest::parse("GREATER 0.1").unwrap(),
                ..Default::default()
            };
            let p = translate_gbuffers(
                &preprocess(&pack, "gbuffers_entities.vsh", &m),
                &preprocess(&pack, "gbuffers_entities.fsh", &m),
                Target::Buffers,
                &inputs,
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
            let functions = p.fragment.functions.iter().map(|(_, f)| f);
            let kill = functions
                .chain([&p.fragment.entry_points[0].function])
                .any(|f| format!("{:?}", f.body).contains("Kill"));
            // With `out` variables the pack tests alpha itself.
            assert_eq!(kill, kills, "{fsh}");
        }
    }

    #[test]
    fn gbuffers_programs_read_the_games_vertices() {
        let vsh = "#version 120\nattribute vec4 mc_Entity;\nattribute vec2 mc_midTexCoord;\nvarying vec2 uv;\nvarying vec2 light;\n\
                   varying vec4 color;\nvarying vec3 normal;\nvarying float id;\nvoid main() {\n    \
                   gl_Position = ftransform();\n    uv = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;\n    \
                   light = (gl_TextureMatrix[1] * gl_MultiTexCoord1).xy;\n    color = gl_Color;\n    \
                   normal = normalize(gl_NormalMatrix * gl_Normal);\n    id = mc_Entity.x + mc_midTexCoord.x;\n}\n";
        let fsh = "#version 120\nuniform sampler2D texture;\nuniform sampler2D lightmap;\nuniform mat4 gbufferModelView;\n\
                   uniform int renderStage;\nvarying vec2 uv;\nvarying vec2 light;\nvarying vec4 color;\nvarying vec3 normal;\nvarying float id;\n\
                   void main() {\n    vec4 c = texture2D(texture, uv) * texture2D(lightmap, light) * color;\n    \
                   /* DRAWBUFFERS:02 */\n    gl_FragData[0] = c * gl_FragCoord.z * float(renderStage);\n    \
                   gl_FragData[1] = vec4(normal * 0.5 + 0.5, id);\n}\n";
        let files: Vec<(String, &str)> =
            [("gbuffers_terrain.vsh", vsh), ("gbuffers_terrain.fsh", fsh)]
                .iter()
                .map(|(p, t)| (format!("shaders/{p}"), *t))
                .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        let m = Macros::standard();
        let game = GbuffersInputs {
            declarations: "layout(location = 0) in vec3 game_pos;\nlayout(location = 1) in uvec2 game_uv;".into(),
            body: "    hz_Vertex = vec4(game_pos, 1.0);\n    hz_MultiTexCoord0 = vec4(vec2(game_uv) / 32768.0, 0.0, 1.0);\n    \
                   hz_MultiTexCoord1 = vec4(240.0, 240.0, 0.0, 1.0);\n    hz_Color = vec4(1.0);\n    hz_Normal = vec3(0.0, 1.0, 0.0);\n"
                .into(),
            attributes: vec![("mc_midTexCoord".into(), "vec2(game_uv) / 32768.0".into())],
            ..Default::default()
        };
        let p = translate_gbuffers(
            &preprocess(&pack, "gbuffers_terrain.vsh", &m),
            &preprocess(&pack, "gbuffers_terrain.fsh", &m),
            Target::Buffers,
            &game,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let names: Vec<&str> = p
            .interface
            .uniforms
            .iter()
            .map(|u| u.name.as_str())
            .collect();
        for name in [
            "gbufferModelView",
            "gbufferProjection",
            "gbufferModelViewInverse",
        ] {
            assert!(names.contains(&name), "{name} missing from {names:?}");
        }
        assert_eq!(p.interface.outputs, vec![0, 1]);
        // renderStage is set per draw, in its own block.
        assert!(p.interface.draw && !names.contains(&"renderStage"));
        let per_draw = p.fragment.global_variables.iter().any(|(_, v)| {
            v.binding
                .as_ref()
                .is_some_and(|b| b.group == 2 && b.binding == 0)
        });
        assert!(per_draw);
        let inputs: Vec<&str> = p.vertex.entry_points[0]
            .function
            .arguments
            .iter()
            .filter_map(|a| a.name.as_deref())
            .collect();
        assert!(
            inputs.contains(&"game_pos") && inputs.contains(&"game_uv"),
            "{inputs:?}"
        );
        // From the light, the compatibility matrices are the shadow camera's.
        let shadow = translate_shadow(
            &preprocess(&pack, "gbuffers_terrain.vsh", &m),
            &preprocess(&pack, "gbuffers_terrain.fsh", &m),
            &game,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let names: Vec<&str> = (shadow.interface.uniforms.iter())
            .map(|u| u.name.as_str())
            .collect();
        for name in ["shadowModelView", "shadowProjection", "gbufferModelView"] {
            assert!(names.contains(&name), "{name} missing from {names:?}");
        }
    }

    #[test]
    fn matrix_and_array_varyings_travel_in_parts() {
        let vsh = "#version 120\nvarying mat3 tbn;\nflat varying ivec2 ids[2];\nvarying mat2x3 m[2];\n\
                   void main() {\n    gl_Position = ftransform();\n    tbn = mat3(2.0);\n    \
                   ids[1] = ivec2(3);\n    m[1][0] = vec3(4.0);\n}\n";
        let fsh = "#version 120\nvarying mat3 tbn;\nflat varying ivec2 ids[2];\nvarying mat2x3 m[2];\n\
                   void main() { gl_FragColor = vec4(tbn[2] + m[1][0], float(ids[1].x)); }\n";
        let p = program(
            &[("composite.vsh", vsh), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let inputs = p.fragment.entry_points[0].function.arguments.len();
        // 3 columns + 2 elements + 2 x 2 columns, and the fragment position.
        assert!(inputs >= 9, "{inputs} inputs");
        assert_eq!(
            varying_parts("flat ivec2 @[2];").unwrap().1,
            [
                ("flat ivec2 @;".to_owned(), "[0]".to_owned()),
                ("flat ivec2 @;".to_owned(), "[1]".to_owned())
            ]
        );
        assert_eq!(varying_parts("mat2x3 @[2];").unwrap().1[3].1, "[1][1]");
        assert!(varying_parts("vec4 @;").is_none());
        assert!(varying_parts("vec4 @[N];").is_none());
    }

    #[test]
    fn sized_array_types_initialize_globals() {
        let fsh = "#version 400\nuniform float k;\nivec3[2] offsets = ivec3[](ivec3(1, 0, 0), ivec3(0, int(k), 0));\n\
                   out vec4 c;\nvoid main() { c = vec4(offsets[1], 1.0); }\n";
        program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn struct_varyings_travel_packed() {
        let lib = "struct Light { vec3 color; float power; int id; mat2 m; float k[2]; };\n";
        let vsh = "#version 400\n#include \"/lib.glsl\"\nflat out Light light;\nvoid main() {\n    \
                   gl_Position = ftransform();\n    light.color = vec3(1.0);\n    light.id = 2;\n}\n";
        let fsh = "#version 400\n#include \"/lib.glsl\"\nflat in Light light;\nout vec4 c;\n\
                   void main() { c = vec4(light.color * light.power * light.m[1].x * light.k[1], float(light.id)); }\n";
        let p = program(
            &[
                ("lib.glsl", lib),
                ("composite.vsh", vsh),
                ("composite.fsh", fsh),
            ],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        // color and power, the matrix's columns, the array, and the int: four slots.
        let inputs = p.fragment.entry_points[0].function.arguments.len();
        assert_eq!(inputs, 4, "{inputs} inputs");
        let structs = std::collections::HashMap::from([(
            "S".to_owned(),
            vec![
                ("vec3".to_owned(), "a".to_owned()),
                ("int".to_owned(), "i".to_owned()),
                ("float".to_owned(), "b[2]".to_owned()),
            ],
        )]);
        assert_eq!(
            struct_parts("S @;", &structs).unwrap(),
            [
                (
                    "vec4 @;".to_owned(),
                    vec![
                        (".xyz".to_owned(), ".a".to_owned()),
                        (".w".to_owned(), ".b[0]".to_owned())
                    ]
                ),
                (
                    "flat int @;".to_owned(),
                    vec![(String::new(), ".i".to_owned())]
                ),
                (
                    "float @;".to_owned(),
                    vec![(String::new(), ".b[1]".to_owned())]
                ),
            ]
        );
        let bools = std::collections::HashMap::from([(
            "B".to_owned(),
            vec![("bool".to_owned(), "b".to_owned())],
        )]);
        assert!(struct_parts("B @;", &bools).is_none());
    }

    #[test]
    fn vulkan_type_names_are_free_names() {
        let fsh = "#version 400\nuniform sampler2D colortex0;\nout vec4 c;\n\
                   vec4 fetch(sampler2D sampler, vec2 p) { return texture(sampler, p); }\n\
                   void main() { float texture2D = 1.0; c = fetch(colortex0, vec2(0.5)) * texture2D; }\n";
        program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn gathers_and_vertex_texture_reads_translate() {
        let vsh = "#version 400\nuniform sampler2D colortex1;\nout vec3 tint;\nvoid main() {\n    \
                   gl_Position = ftransform();\n    tint = texture(colortex1, vec2(0.5)).rgb;\n}\n";
        let fsh = "#version 400\nuniform sampler2D colortex1;\nuniform sampler2D depthtex0;\nin vec3 tint;\nout vec4 c;\n\
                   struct S { float k[2]; };\n\
                   void main() {\n    const S s = S(float[2](1.0, 2.0));\n    \
                   vec4 d = textureGather(depthtex0, vec2(0.5)) + textureGather(colortex1, vec2(0.5), 3);\n    \
                   c = vec4(tint * vec3(d.x > 0.5) * s.k[1], float(ivec2(d.xy) % 2));\n}\n";
        program(
            &[("composite.vsh", vsh), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn fields_and_elements_as_out_arguments_write_spirv() {
        let fsh = "#version 330\nstruct S { float a; vec2 b; };\nvoid f(out float x) { x = 0.5; }\n\
                   void main() { S s; float arr[2]; f(s.a); f(arr[1]); f(s.b.y);\n\
                   gl_FragData[0] = vec4(s.a, arr[1], s.b.y, 1.0); }\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        for module in [&p.vertex, &p.fragment] {
            let info = Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(module)
                .unwrap();
            let entry = &module.entry_points[0];
            let pipeline = naga::back::spv::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
            };
            naga::back::spv::write_vec(module, &info, &Default::default(), Some(&pipeline))
                .unwrap();
        }
    }

    #[test]
    fn a_pack_keeps_its_own_inverse() {
        let fsh = "#version 120\nuniform mat4 gbufferProjection;\n\
                   mat4 inverse(mat4 m) { return m; }\n\
                   void main() { gl_FragData[0] = inverse(gbufferProjection) * vec4(1.0); }\n";
        program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn narrow_outputs_widen_and_inverse_is_ours() {
        let fsh = "#version 400\nlayout(location = 0) out vec3 color;\nlayout(location = 1) out float depth;\n\
                   uniform mat4 gbufferProjection;\nuniform mat3 m3;\n\
                   void main() { color = (inverse(gbufferProjection) * vec4(1.0)).xyz * inverse(m3)[0]; depth = 0.5; }\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let outputs: Vec<_> = p.fragment.entry_points[0]
            .function
            .result
            .iter()
            .flat_map(|r| match &p.fragment.types[r.ty].inner {
                naga::TypeInner::Struct { members, .. } => {
                    members.iter().map(|m| m.ty).collect::<Vec<_>>()
                }
                _ => vec![r.ty],
            })
            .map(|ty| p.fragment.types[ty].inner.clone())
            .collect();
        assert!(
            outputs.iter().all(|t| matches!(
                t,
                naga::TypeInner::Vector {
                    size: naga::VectorSize::Quad,
                    ..
                }
            )),
            "{outputs:?}"
        );
        assert!(p.fragment.functions.iter().all(|(_, f)| {
            f.expressions.iter().all(|(_, e)| {
                !matches!(
                    e,
                    naga::Expression::Math {
                        fun: naga::MathFunction::Inverse,
                        ..
                    }
                )
            })
        }));
    }

    #[test]
    fn buffer_blocks_bind_at_their_binding() {
        let csh = "#version 430\nlayout(local_size_x = 1) in;\n\
                   layout(std430, binding = 2) readonly restrict buffer Lights { uint count; vec4 data[]; } lights;\n\
                   layout(std430, binding = 5) buffer Counts { uint n[]; };\n\
                   void main() { n[0] = 1u; }\n";
        let pack = Pack::from_zip_bytes("P.zip", zip(&[("shaders/deferred_a.csh", csh)])).unwrap();
        let c = translate_compute(&preprocess(&pack, "deferred_a.csh", &Macros::standard()))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(c.interface.buffers, [2, 5]);
        let missing = "#version 430\nlayout(local_size_x = 1) in;\nlayout(std430) buffer B { uint n; };\nvoid main() {}\n";
        let pack =
            Pack::from_zip_bytes("P.zip", zip(&[("shaders/deferred_a.csh", missing)])).unwrap();
        let errors = translate_compute(&preprocess(&pack, "deferred_a.csh", &Macros::standard()))
            .err()
            .unwrap();
        assert!(errors[0].message.contains("needs a binding"), "{errors:?}");
    }

    #[test]
    fn atomics_on_shared_vector_components_compile() {
        let csh = "#version 430\nlayout(local_size_x = 4) in;\nshared ivec4 sums;\nshared uvec2 lights[8];\n\
                   layout(r32i) uniform iimage2D colorimg5;\nvoid main() {\n    \
                   if (gl_LocalInvocationIndex == 0u) sums = ivec4(0);\n    barrier();\n    \
                   atomicAdd(sums.y, 1);\n    atomicMax(lights[gl_LocalInvocationIndex].x, 2u);\n    \
                   sums += ivec4(1);\n    lights[gl_LocalInvocationIndex] += uvec2(1u);\n    \
                   uvec2 l = lights[gl_LocalInvocationIndex];\n    barrier();\n    \
                   imageStore(colorimg5, ivec2(0), ivec4(sums.y + int(lights[1].x + l.y)));\n}\n";
        let pack = Pack::from_zip_bytes("P.zip", zip(&[("shaders/deferred_a.csh", csh)])).unwrap();
        translate_compute(&preprocess(&pack, "deferred_a.csh", &Macros::standard()))
            .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn work_group_size_is_its_layout() {
        let csh = "#version 430\nlayout(local_size_x = 8, local_size_y = 4) in;\n\
                   layout(r32ui) uniform uimage2D colorimg4;\nvoid main() {\n    \
                   imageStore(colorimg4, ivec2(gl_WorkGroupSize.xy), uvec4(gl_WorkGroupSize.z));\n}\n";
        let pack = Pack::from_zip_bytes("P.zip", zip(&[("shaders/deferred_a.csh", csh)])).unwrap();
        let c = translate_compute(&preprocess(&pack, "deferred_a.csh", &Macros::standard()))
            .unwrap_or_else(|e| panic!("{e:?}"));
        let main = &c
            .module
            .functions
            .iter()
            .find(|f| f.1.name.as_deref() == Some("hz_main"))
            .unwrap()
            .1;
        assert!(
            main.expressions
                .iter()
                .any(|(_, e)| matches!(e, naga::Expression::Literal(naga::Literal::U32(8))))
        );
        assert_eq!(
            work_group_size("layout(local_size_x = 8 * 2) in;\nuvec3 s = gl_WorkGroupSize;"),
            Err("`local_size_x` is not a number gl_WorkGroupSize can take".into())
        );
        assert!(
            work_group_size("layout(local_size_x = 16) in;\nuvec3 s = gl_WorkGroupSize;")
                .is_ok_and(|c| c.contains("uvec3(16u, 1u, 1u)"))
        );
    }

    #[test]
    fn compute_programs_translate() {
        let csh = "#version 430\nlayout(local_size_x = 64) in;\nconst ivec3 workGroups = ivec3(2, 1, 1);\n\
                   layout(rgba16f) writeonly uniform image2D colorimg4;\nuniform sampler2D colortex4;\n\
                   uniform float frameTimeCounter;\nshared vec3 sums[64];\nvoid main() {\n    \
                   uint i = gl_LocalInvocationID.x;\n    sums[i] = texelFetch(colortex4, ivec2(i, 0), 0).rgb * frameTimeCounter;\n    \
                   barrier();\n    if (i == 0u) imageStore(colorimg4, ivec2(0), vec4(sums[0] + subgroupAdd(sums[1]), 1.0));\n}\n";
        let pack = Pack::from_zip_bytes("P.zip", zip(&[("shaders/deferred_a.csh", csh)])).unwrap();
        let c = translate_compute(&preprocess(&pack, "deferred_a.csh", &Macros::standard()))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(c.work_groups, Some([2, 1, 1]));
        assert_eq!(c.work_groups_render, [1.0, 1.0]);
        let render = |decl: &str| {
            let csh =
                format!("#version 430\nlayout(local_size_x = 8) in;\n{decl}\nvoid main() {{}}\n");
            let pack =
                Pack::from_zip_bytes("P.zip", zip(&[("shaders/deferred_a.csh", &csh)])).unwrap();
            translate_compute(&preprocess(&pack, "deferred_a.csh", &Macros::standard()))
                .map(|c| c.work_groups_render)
                .map_err(|e| e[0].message.clone())
        };
        assert_eq!(
            render("const vec2 workGroupsRender = vec2(0.5, 0.25);"),
            Ok([0.5, 0.25])
        );
        assert_eq!(
            render("const vec3 workGroupsRender = vec3(0.5, 1.0, 1.0);"),
            Ok([0.5, 1.0])
        );
        assert!(render("const float workGroupsRender = 0.5;").is_err());
        assert_eq!(c.module.entry_points[0].workgroup_size, [64, 1, 1]);
        let i = &c.interface;
        assert_eq!(
            (i.samplers[0].binding, i.images[0].binding),
            (1, 3),
            "{:?}",
            i.images
        );
        assert_eq!(
            (i.images[0].format.as_str(), i.images[0].access.as_str()),
            ("rgba16f", "writeonly")
        );
    }

    #[test]
    fn locals_may_hide_a_sampler() {
        let fsh = "#version 120\nuniform sampler2D specular;\nstruct M { float specular; };\n\
                   float f(float x) { float specular = x * 2.0; return specular; }\n\
                   void main() { M m = M(0.5); gl_FragColor = texture2D(specular, vec2(0.5)) * f(m.specular); }\n";
        program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    }

    #[test]
    fn errors_point_at_the_pack_file_and_line() {
        let fsh =
            "#version 120\n#include \"/lib/bad.glsl\"\nvoid main() { gl_FragColor = vec4(f()); }\n";
        let bad = "// fine\nfloat f() {\n    return undefinedThing;\n}\n";
        let errors = program(
            &[
                ("composite.vsh", VSH_120),
                ("composite.fsh", fsh),
                ("lib/bad.glsl", bad),
            ],
            Target::Buffers,
        )
        .err()
        .unwrap();
        assert_eq!(
            (errors[0].file.as_str(), errors[0].line),
            ("lib/bad.glsl", 3),
            "{errors:?}"
        );
    }

    /// Swizzled scalars, GLSL 4.20's (Kappa's weather map: `noise(p).x`).
    #[test]
    fn scalar_swizzles_translate() {
        let fsh = "#version 430\nout vec4 color;\nfloat noise(vec2 p) { return fract(p.x * 3.1); }\n\
                   void main() {\n    float a = noise(gl_FragCoord.xy * 0.1).x;\n    float s = a.r * 2.0;\n    \
                   vec2 w = s.xx + noise(vec2(a)).ss;\n    color = vec4(w, a, s);\n}\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        );
        assert!(p.is_ok(), "{:?}", p.err());
    }

    /// `x / 0.0` and `sqrt(-1.0)` are GL's infinity and NaN at run time; naga folds them into
    /// constants and refuses them (Mellow, Bloop). `texelFetch2D` is GL_EXT_gpu_shader4's name
    /// (Bliss).
    #[test]
    fn infinities_nans_and_old_fetches_translate() {
        let fsh = "#version 330\nuniform sampler2D noisetex;\nout vec4 color;\nvoid main() {\n    \
                   ivec2 t = ivec2(gl_FragCoord.xy);\n    \
                   color = texelFetch2D(noisetex, t, 0) + vec4(-1.0 / 0.0) + vec4(sqrt(-1.0));\n}\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        );
        assert!(p.is_ok(), "{:?}", p.err());
    }

    /// A matrix varying's column written by component, and `uint` with integer literals in
    /// `max`, `min` and `clamp` (Kappa, Hysteria).
    #[test]
    fn matrix_columns_and_unsigned_overloads_translate() {
        let vsh = "#version 330 compatibility\nout mat2x2 uv;\nvoid main() {\n    \
                   uv = mat2x2(gl_MultiTexCoord0.xy, gl_MultiTexCoord1.xy);\n    \
                   uv[1].y = uv[1].y * 0.5;\n    gl_Position = ftransform();\n}\n";
        let fsh = "#version 330\nin mat2x2 uv;\nout vec4 color;\nvoid main() {\n    uint n = 3u;\n    \
                   uint s = max(n, 1);\n    s = min(s, 32) + uint(clamp(n, 0, 7));\n    \
                   color = vec4(uv[0], uv[1]) / float(s);\n}\n";
        let p = program(
            &[("composite.vsh", vsh), ("composite.fsh", fsh)],
            Target::Buffers,
        );
        assert!(p.is_ok(), "{:?}", p.err());
    }

    /// `const`s that call a built-in naga's constant evaluator lacks (`mix`, `smoothstep`...)
    /// are evaluated where the program runs: a global by our `main`, a local in place (Bliss).
    #[test]
    fn constants_naga_cannot_fold_are_evaluated_at_run_time() {
        let fsh = "#version 120\nvarying vec2 texcoord;\nconst vec3 tint = mix(vec3(0.2), vec3(1.0), 0.25);\n\
                   const float edge = smoothstep(0.0, 1.0, 0.5), mid = 0.5;\n\
                   float shade(float x) {\n    const float k = mix(0.5, 1.0, 0.3);\n    return x * k;\n}\n\
                   void main() { gl_FragColor = vec4(tint * shade(texcoord.x) * edge * mid, 1.0); }\n";
        let p = program(
            &[("composite.vsh", VSH_120), ("composite.fsh", fsh)],
            Target::Buffers,
        );
        assert!(p.is_ok(), "{:?}", p.err());
    }
}
