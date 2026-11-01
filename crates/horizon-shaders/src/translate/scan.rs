//! The first pass over a stage's tokens: what it declares at global scope, which of its
//! functions take samplers, which outputs and built-ins it uses.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;

use naga::ShaderStage;

use super::Error;
use super::rewrite::attribute_value;
use crate::Preprocessed;
use crate::macros::lex;

/// A varying: its name and its declaration with `@` where the name goes (`flat ivec2 @[2];`).
pub(super) struct Var {
    pub(super) name: String,
    pub(super) decl: String,
    /// Of a struct type: the pack's declaration stays as a private global, and the members
    /// travel in varyings of ours (`struct_parts`).
    pub(super) structure: bool,
}

/// `layout(rgba16f) uniform writeonly image2D colorimg4;`
#[derive(Clone)]
pub(super) struct ImageDecl {
    pub(super) name: String,
    pub(super) ty: String,
    pub(super) format: String,
    /// `readonly`, `writeonly` or empty (both).
    pub(super) access: String,
    /// Declared `coherent` or `volatile`: invocations of a dispatch share it as it changes.
    pub(super) coherent: bool,
}

pub(super) struct Stage<'p> {
    pub(super) kind: ShaderStage,
    pub(super) pp: &'p Preprocessed,
    pub(super) tokens: Vec<&'p str>,
    /// Token ranges to blank out (declarations moved into the header).
    pub(super) removed: Vec<std::ops::Range<usize>>,
    /// Name, type with its array suffix, token of the type.
    pub(super) uniforms: Vec<(String, String, usize)>,
    pub(super) samplers: Vec<(String, String)>,
    pub(super) images: Vec<ImageDecl>,
    /// The bindings of the SSBOs (`buffer` blocks), in set 4.
    pub(super) buffers: BTreeSet<u32>,
    pub(super) ins: Vec<Var>,
    pub(super) outs: Vec<Var>,
    /// Vertex inputs, now private globals, and the value our `main` gives each.
    /// Vertex inputs: name, our composite value, type.
    pub(super) attributes: Vec<(String, String, String)>,
    /// A gbuffers-style program: the matrices are the frame's, not the quad's.
    pub(super) gbuffers: bool,
    /// The pack's fragment outputs, declared again with their locations.
    pub(super) frag_outs: Vec<(u32, String)>,
    /// Narrow outputs copied into their vec4 after the pack's `main`.
    pub(super) out_copies: Vec<String>,
    /// The stage calls `inverse`, which ours replaces, and doesn't define its own.
    pub(super) inverse: bool,
    /// The pack's functions that take samplers: which parameters are samplers.
    pub(super) sampler_functions: HashMap<String, Vec<bool>>,
    /// The pack's structs: each member's type and name with its array suffix.
    pub(super) structs: HashMap<String, Vec<(String, String)>>,
    pub(super) outputs: BTreeSet<u32>,
    pub(super) frag_coord: bool,
    /// Tokens written differently, by index.
    pub(super) replaced: HashMap<usize, String>,
    /// Globals whose initializer runs in our `main` (`g = hz_init_g();`), in order.
    pub(super) inits: Vec<String>,
    pub(super) errors: Vec<Error>,
}

/// The format qualifiers of images (GLSL 4.60, 4.4.6.2).
const IMAGE_FORMATS: [&str; 39] = [
    "rgba32f",
    "rgba16f",
    "rg32f",
    "rg16f",
    "r11f_g11f_b10f",
    "r32f",
    "r16f",
    "rgba16",
    "rgb10_a2",
    "rgba8",
    "rg16",
    "rg8",
    "r16",
    "r8",
    "rgba16_snorm",
    "rgba8_snorm",
    "rg16_snorm",
    "rg8_snorm",
    "r16_snorm",
    "r8_snorm",
    "rgba32i",
    "rgba16i",
    "rgba8i",
    "rg32i",
    "rg16i",
    "rg8i",
    "r32i",
    "r16i",
    "r8i",
    "rgba32ui",
    "rgba16ui",
    "rgb10_a2ui",
    "rgba8ui",
    "rg32ui",
    "rg16ui",
    "rg8ui",
    "r32ui",
    "r16ui",
    "r8ui",
];

pub(super) const STORAGE: [&str; 5] = ["uniform", "in", "out", "attribute", "varying"];
pub(super) const QUALIFIERS: [&str; 11] = [
    "flat",
    "smooth",
    "noperspective",
    "centroid",
    "invariant",
    "highp",
    "mediump",
    "lowp",
    "precise",
    "sample",
    "const",
];
/// The compatibility profile's uniform state (with its `Inverse`/`Transpose` forms).
pub(super) const UNIFORM_STATE: [&str; 5] = [
    "gl_ModelViewMatrix",
    "gl_ProjectionMatrix",
    "gl_ModelViewProjectionMatrix",
    "gl_NormalMatrix",
    "gl_TextureMatrix",
];
/// The compatibility profile's vertex inputs.
pub(super) const VERTEX_INPUTS: [&str; 6] = [
    "gl_Vertex",
    "gl_Color",
    "gl_SecondaryColor",
    "gl_Normal",
    "gl_FogCoord",
    "gl_MultiTexCoord",
];
/// Built-in functions naga's constant evaluator lacks: a `const` that calls one is evaluated
/// where the program runs instead (GL drivers fold them; naga fails the program).
pub(super) const NOT_CONSTANT: [&str; 25] = [
    "mix",
    "smoothstep",
    "reflect",
    "refract",
    "faceforward",
    "transpose",
    "determinant",
    "inverse",
    "outerProduct",
    "matrixCompMult",
    "modf",
    "frexp",
    "ldexp",
    "packUnorm2x16",
    "packSnorm2x16",
    "packUnorm4x8",
    "packSnorm4x8",
    "packHalf2x16",
    "unpackUnorm2x16",
    "unpackSnorm2x16",
    "unpackUnorm4x8",
    "unpackSnorm4x8",
    "unpackHalf2x16",
    "bitfieldExtract",
    "bitfieldInsert",
];
/// Words that became keywords after GLSL 1.x and that old packs use as names.
pub(super) const LATER_KEYWORDS: [&str; 17] = [
    "sample",
    "buffer",
    "shared",
    "filter",
    "input",
    "output",
    "common",
    "partition",
    "active",
    "resource",
    "patch",
    "subroutine",
    "coherent",
    "volatile",
    "restrict",
    "readonly",
    "writeonly",
];

/// Vulkan GLSL's texture and sampler types, which naga's frontend reads as types, renamed.
const VULKAN_TYPES: [(&str, &str); 9] = [
    ("sampler", "hz_kw_sampler"),
    ("samplerShadow", "hz_kw_samplerShadow"),
    ("texture1D", "hz_kw_texture1D"),
    ("texture2D", "hz_kw_texture2D"),
    ("texture3D", "hz_kw_texture3D"),
    ("textureCube", "hz_kw_textureCube"),
    ("texture2DArray", "hz_kw_texture2DArray"),
    ("texture2DMS", "hz_kw_texture2DMS"),
    ("textureCubeArray", "hz_kw_textureCubeArray"),
];

pub(super) fn significant(t: &str) -> bool {
    !t.trim().is_empty()
}

/// A fragment output narrower than four components: the vec4 of its kind, and the components
/// that fill it (alpha 1).
fn widened(ty: &str) -> Option<(&'static str, &'static str)> {
    Some(match ty {
        "float" => ("vec4", ", 0.0, 0.0, 1.0"),
        "vec2" => ("vec4", ", 0.0, 1.0"),
        "vec3" => ("vec4", ", 1.0"),
        "int" => ("ivec4", ", 0, 0, 1"),
        "ivec2" => ("ivec4", ", 0, 1"),
        "ivec3" => ("ivec4", ", 1"),
        "uint" => ("uvec4", ", 0u, 0u, 1u"),
        "uvec2" => ("uvec4", ", 0u, 1u"),
        "uvec3" => ("uvec4", ", 1u"),
        _ => return None,
    })
}

/// `sampler2D`, `isampler3D`, `usampler2D`, `sampler2DShadow`...
pub(super) fn is_sampler_type(t: &str) -> bool {
    t.trim_start_matches(['i', 'u']).starts_with("sampler")
}

pub(super) fn is_ident(t: &str) -> bool {
    t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

impl<'p> Stage<'p> {
    pub(super) fn scan(pp: &'p Preprocessed, kind: ShaderStage) -> Self {
        let mut s = Stage {
            kind,
            pp,
            tokens: lex(&pp.code),
            removed: Vec::new(),
            uniforms: Vec::new(),
            samplers: Vec::new(),
            images: Vec::new(),
            buffers: BTreeSet::new(),
            ins: Vec::new(),
            outs: Vec::new(),
            attributes: Vec::new(),
            frag_outs: Vec::new(),
            out_copies: Vec::new(),
            inverse: false,
            sampler_functions: HashMap::new(),
            structs: HashMap::new(),
            outputs: BTreeSet::new(),
            frag_coord: false,
            gbuffers: false,
            replaced: HashMap::new(),
            inits: Vec::new(),
            errors: Vec::new(),
        };
        s.rename_vulkan_types();
        let (mut start, mut depth, mut parens) = (0, 0usize, 0usize);
        let mut own_inverse = false;
        for i in 0..s.tokens.len() {
            match s.tokens[i] {
                "(" => parens += 1,
                ")" => parens = parens.saturating_sub(1),
                "{" if depth == 0 && parens == 0 => {
                    s.global(start, i, true);
                    s.structure(start, i);
                    depth = 1;
                }
                "{" => depth += 1,
                "}" if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        start = i + 1;
                    }
                }
                ";" if depth == 0 && parens == 0 => {
                    s.global(start, i, false);
                    start = i + 1;
                }
                "gl_FragData" | "gl_FragColor" if kind == ShaderStage::Fragment => {
                    let n = if s.tokens[i] == "gl_FragColor" {
                        Some(0)
                    } else {
                        s.index_after(i).map(|(n, _)| n)
                    };
                    match n {
                        Some(n) => _ = s.outputs.insert(n),
                        None => s
                            .errors
                            .push(s.error(i, "`gl_FragData` needs a constant index")),
                    }
                }
                "gl_FragCoord" => s.frag_coord = true,
                "inverse" => {
                    s.inverse = true;
                    // `mat4 inverse(mat4 m)`: a pack for GLSL 1.20, which has none, wrote its own.
                    own_inverse |= depth == 0
                        && s.tokens[..i]
                            .iter()
                            .rfind(|t| significant(t))
                            .is_some_and(|t| is_ident(t));
                }
                "gl_TexCoord" => {
                    if let Some((n, _)) = s.index_after(i) {
                        s.builtin_varying(format!("hz_TexCoord{n}"));
                    }
                }
                "gl_FrontColor" => s.builtin_varying("hz_FrontColor".into()),
                "gl_Color" if kind == ShaderStage::Fragment => {
                    s.builtin_varying("hz_FrontColor".into())
                }
                _ => {}
            }
        }
        s.inverse &= !own_inverse;
        s
    }

    /// Names that desktop GLSL leaves free but Vulkan GLSL takes for its separate texture and
    /// sampler types (`sampler2D sampler`): renamed, unless called (`texture2D(...)`).
    fn rename_vulkan_types(&mut self) {
        for i in 0..self.tokens.len() {
            let Some(&(_, renamed)) = VULKAN_TYPES.iter().find(|(t, _)| *t == self.tokens[i])
            else {
                continue;
            };
            let called = self.tokens[i + 1..]
                .iter()
                .find(|t| significant(t))
                .is_some_and(|&t| t == "(");
            if !called {
                self.tokens[i] = renamed;
            }
        }
    }

    pub(super) fn builtin_varying(&mut self, name: String) {
        let list = if self.kind == ShaderStage::Vertex {
            &mut self.outs
        } else {
            &mut self.ins
        };
        if !list.iter().any(|v| v.name == name) {
            list.push(Var {
                name,
                decl: "vec4 @;".into(),
                structure: false,
            });
        }
    }

    /// `[ n ]` right after token `i`: the number and the index of the `]`.
    pub(super) fn index_after(&self, i: usize) -> Option<(u32, usize)> {
        let mut it = (i + 1..self.tokens.len()).filter(|&j| significant(self.tokens[j]));
        let open = it.next()?;
        let n = it.next()?;
        let close = it.next()?;
        (self.tokens[open] == "[" && self.tokens[close] == "]")
            .then(|| self.tokens[n].parse().ok().map(|n| (n, close)))
            .flatten()
    }

    pub(super) fn error(&self, token: usize, message: &str) -> Error {
        let line: usize = self.tokens[..token.min(self.tokens.len())]
            .iter()
            .map(|t| t.matches('\n').count())
            .sum();
        let o = self.pp.lines.get(line).copied().unwrap_or_default();
        Error {
            file: self
                .pp
                .files
                .get(o.file as usize)
                .cloned()
                .unwrap_or_default(),
            line: o.line,
            message: message.to_owned(),
        }
    }

    /// A statement at global scope, tokens `from..to` (without its `;`, or before a `{`).
    pub(super) fn global(&mut self, from: usize, to: usize, body: bool) {
        let words: Vec<usize> = (from..to)
            .filter(|&i| significant(self.tokens[i]))
            .collect();
        // A prototype: a `(` before any `=`, and no storage qualifier.
        let paren = words
            .iter()
            .position(|&i| matches!(self.tokens[i], "(" | "="));
        let prototype = paren.is_some_and(|p| self.tokens[words[p]] == "(")
            && !words.iter().any(|&i| STORAGE.contains(&self.tokens[i]));
        if body && words.iter().any(|&i| self.tokens[i] == "buffer") {
            return self.buffer_block(&words);
        }
        if body || prototype {
            return self.function(&words);
        }
        let mut k = 0;
        let mut storage = None;
        let mut location = None;
        let mut format = None;
        let mut access = "";
        let mut coherent = false;
        let mut interpolation = String::new();
        while let Some(&i) = words.get(k) {
            let w = self.tokens[i];
            if w == "layout" {
                // layout(location = n, ...), an image's layout(rgba16f)
                let mut j = k + 1;
                while let Some(&t) = words.get(j) {
                    j += 1;
                    match self.tokens[t] {
                        "location" => {
                            location = words
                                .get(j + 1)
                                .and_then(|&n| self.tokens[n].parse::<u32>().ok());
                        }
                        f if IMAGE_FORMATS.contains(&f) => format = Some(f),
                        ")" => break,
                        _ => {}
                    }
                }
                k = j;
                continue;
            }
            if STORAGE.contains(&w) {
                storage = Some(w);
            } else if matches!(w, "readonly" | "writeonly") {
                access = w;
            } else if QUALIFIERS.contains(&w) || matches!(w, "coherent" | "volatile" | "restrict") {
                coherent |= matches!(w, "coherent" | "volatile");
                if matches!(
                    w,
                    "flat" | "smooth" | "noperspective" | "centroid" | "sample"
                ) {
                    write!(interpolation, "{w} ").unwrap();
                }
            } else {
                break;
            }
            k += 1;
        }
        let Some(storage) = storage else {
            return self.initializer(&words, to);
        };
        let Some(&ty_at) = words.get(k) else { return };
        let ty = self.tokens[ty_at].to_owned();
        if words[k..].iter().any(|&i| self.tokens[i] == "{") {
            return; // An interface block: left as the pack wrote it.
        }
        // Declarators: name [array] [= init], separated by commas.
        let mut declarators: Vec<(String, String)> = Vec::new();
        let mut j = k + 1;
        while let Some(&n) = words.get(j) {
            let name = self.tokens[n].to_owned();
            let mut array = String::new();
            j += 1;
            let mut nesting = 0;
            while let Some(&t) = words.get(j) {
                match self.tokens[t] {
                    "," if nesting == 0 => break,
                    "(" | "[" => nesting += 1,
                    ")" | "]" => nesting -= 1,
                    _ => {}
                }
                if self.tokens[t] == "=" && nesting == 0 {
                    // An initializer: uniforms are set by the game, so it is dropped.
                    while words.get(j).is_some_and(|&t| self.tokens[t] != ",") {
                        j += 1;
                    }
                    break;
                }
                array.push_str(self.tokens[t]);
                j += 1;
            }
            j += 1;
            declarators.push((name, array));
        }
        let fragment = self.kind == ShaderStage::Fragment;
        let input = matches!(storage, "in" | "attribute") || (storage == "varying" && fragment);
        self.removed.push(from..to + 1);
        for (name, array) in declarators {
            let decl = format!("{interpolation}{ty} @{array};");
            match storage {
                "uniform" if is_sampler_type(&ty) => self.samplers.push((name, ty.clone())),
                "uniform" if ty.contains("image") => match format {
                    Some(format) => self.images.push(ImageDecl {
                        name,
                        ty: ty.clone(),
                        format: format.to_owned(),
                        access: access.to_owned(),
                        coherent,
                    }),
                    None => self
                        .errors
                        .push(self.error(ty_at, &format!("`{name}`: an image needs a format"))),
                },
                "uniform" => self.uniforms.push((name, format!("{ty}{array}"), ty_at)),
                _ if input && !fragment => {
                    let value = attribute_value(&name, &ty);
                    self.attributes.push((name, value, ty.clone()));
                    self.removed.pop();
                    // A private global of ours now: `in`/`attribute` and qualifiers go.
                    for &w in &words[..k] {
                        self.removed.push(w..w + 1);
                    }
                }
                _ if self.structs.contains_key(&ty) => {
                    self.removed.pop();
                    for &w in &words[..k] {
                        self.removed.push(w..w + 1);
                    }
                    let var = Var {
                        name,
                        decl,
                        structure: true,
                    };
                    if input {
                        self.ins.push(var);
                    } else {
                        self.outs.push(var);
                    }
                }
                _ if input => self.ins.push(Var {
                    name,
                    decl,
                    structure: false,
                }),
                _ if fragment => {
                    let n = location.unwrap_or(self.outputs.len() as u32);
                    self.outputs.insert(n);
                    // wgpu wants every component of the target's format written; GL fills
                    // what a narrower output leaves out. Ours is a vec4, theirs a private global.
                    match widened(&ty).filter(|_| array.is_empty()) {
                        Some((wide, fill)) => {
                            self.frag_outs.push((
                                n,
                                format!(
                                    "layout(location = {n}) out {wide} hz_out{n}; {ty} {name};"
                                ),
                            ));
                            self.out_copies
                                .push(format!("hz_out{n} = {wide}({name}{fill});"));
                        }
                        None => self
                            .frag_outs
                            .push((n, format!("layout(location = {n}) out {ty} {name}{array};"))),
                    }
                }
                _ => self.outs.push(Var {
                    name,
                    decl,
                    structure: false,
                }),
            }
        }
    }

    /// A global `T name = a ? b : c;` or, without `const`, any initialized global: naga only
    /// evaluates constant initializers at global scope (a conditional is not one; GL drivers
    /// also take uniforms and other globals there), so each initializer becomes a function of
    /// ours on the same line (`T a; T hz_init_a() { return T(...); } T b; ...`), called by our
    /// `main` before the pack's, in file order. The return converts as GLSL's initialization
    /// does (`vec2 v = ivec2(...)`).
    pub(super) fn initializer(&mut self, words: &[usize], to: usize) {
        let text = |i: usize| self.tokens[i];
        let Some(eq) = words.iter().position(|&i| text(i) == "=") else {
            return;
        };
        // Declarators, split at top-level commas: (first word, `=` if any, end).
        let mut declarators = Vec::new();
        let (mut begin, mut nesting, mut assign) = (0, 0i32, None);
        for (n, &i) in words.iter().enumerate() {
            match text(i) {
                "(" | "[" | "{" => nesting += 1,
                ")" | "]" | "}" => nesting -= 1,
                "=" if nesting == 0 && assign.is_none() => assign = Some(n),
                "," if nesting == 0 => {
                    declarators.push((begin, assign.take(), n));
                    begin = n + 1;
                }
                _ => {}
            }
        }
        declarators.push((begin, assign, words.len()));
        let constant = words[..eq].iter().any(|&i| text(i) == "const");
        // What naga cannot evaluate at global scope: a conditional, or a call to a built-in its
        // constant evaluator lacks.
        let runtime = |d: &(usize, Option<usize>, usize)| {
            (d.0..d.2).any(|n| {
                text(words[n]) == "?"
                    || (NOT_CONSTANT.contains(&text(words[n]))
                        && words.get(n + 1).is_some_and(|&j| text(j) == "("))
            })
        };
        if constant && !declarators.iter().any(runtime) {
            return;
        }
        // The first declarator's name ends the type.
        let first_end = declarators[0].1.unwrap_or(declarators[0].2);
        let Some(name_at) = words[..first_end].iter().rposition(|&i| is_ident(text(i))) else {
            return;
        };
        // The type, with a `vec3[]`'s count written in.
        let mut ty = String::new();
        for &i in &words[..name_at] {
            match text(i) {
                "const" => {}
                "]" if ty.ends_with('[') => match self.array_length(i) {
                    Some(n) => write!(ty, "{n}]").unwrap(),
                    None => return,
                },
                // `T[6]`: the size and its `]` belong to the type.
                t if ty.is_empty() || t == "[" || ty.contains('[') && !ty.ends_with(']') => {
                    ty.push_str(t)
                }
                // Qualifiers before the type (`highp`...): the type is the last word.
                t => ty = t.to_owned(),
            }
        }
        if ty.is_empty() {
            return;
        }
        let mut replaced = Vec::new();
        let mut names = Vec::new();
        for (n, &(begin, assign, end)) in declarators.iter().enumerate() {
            let name_at = if n == 0 { name_at } else { begin };
            let name = text(words[name_at]).to_owned();
            if !is_ident(&name) {
                return;
            }
            let stop = assign.unwrap_or(end);
            let mut array: String = words[name_at + 1..stop].iter().map(|&i| text(i)).collect();
            if array == "[]" {
                // `T x[] = T[](a, b)`: the count is written in, as the rewrite does for the name.
                match assign.and_then(|a| self.array_length(words[a - 1])) {
                    Some(n) => array = format!("[{n}]"),
                    None => return,
                }
            }
            let next = if n + 1 < declarators.len() {
                format!(" {ty}")
            } else {
                String::new()
            };
            let end_token = words.get(end).copied().unwrap_or(to);
            match assign {
                Some(a) => {
                    // Scalars, vectors and matrices convert; arrays and structs must match.
                    let scalar = array.is_empty()
                        && !ty.contains('[')
                        && (matches!(ty.as_str(), "float" | "int" | "uint" | "bool")
                            || ty.contains("vec")
                            || ty.starts_with("mat"));
                    let (open, close) = if scalar {
                        (format!("{ty}("), ")")
                    } else {
                        (String::new(), "")
                    };
                    replaced.push((
                        words[a],
                        format!("; {ty}{array} hz_init_{name}() {{ return {open}"),
                    ));
                    replaced.push((end_token, format!("{close}; }}{next}")));
                    names.push(name);
                }
                None if n + 1 < declarators.len() => replaced.push((end_token, format!(";{next}"))),
                None => {}
            }
        }
        for &i in &words[..name_at] {
            if text(i) == "const" {
                self.removed.push(i..i + 1);
            }
        }
        self.replaced.extend(replaced);
        self.inits.extend(names);
    }

    /// `layout(std430, binding = n) buffer Name {`: an SSBO, in set 4 at its binding. Its
    /// access qualifiers go, so that every stage binds it alike (read and write).
    fn buffer_block(&mut self, words: &[usize]) {
        let mut binding = None;
        for (k, &i) in words.iter().enumerate() {
            match self.tokens[i] {
                "binding" => {
                    binding = words
                        .get(k + 2)
                        .and_then(|&n| self.tokens[n].parse::<u32>().ok());
                    self.replaced.insert(i, "set = 4, binding".into());
                }
                "readonly" | "writeonly" | "restrict" | "coherent" | "volatile" => {
                    self.removed.push(i..i + 1)
                }
                _ => {}
            }
        }
        match binding {
            Some(b) => _ = self.buffers.insert(b),
            None => {
                let at = words.first().copied().unwrap_or(0);
                self.errors
                    .push(self.error(at, "a buffer block needs a binding"));
            }
        }
    }

    /// `struct Name {` at token `open`, whose words start at `from`: its members, to `}`.
    fn structure(&mut self, from: usize, open: usize) {
        let words: Vec<&str> = self.tokens[from..open]
            .iter()
            .copied()
            .filter(|t| significant(t))
            .collect();
        let [.., "struct", name] = words[..] else {
            return;
        };
        let mut members = Vec::new();
        let mut decl: Vec<&str> = Vec::new();
        for &t in &self.tokens[open + 1..] {
            match t {
                "}" => break,
                ";" => {
                    // `T a, b[2]`, after any precision or other qualifier.
                    let words: Vec<&str> =
                        decl.drain(..).filter(|w| !QUALIFIERS.contains(w)).collect();
                    let Some((ty, rest)) = words.split_first() else {
                        continue;
                    };
                    for d in rest.split(|w| *w == ",") {
                        if let Some((n, array)) = d.split_first() {
                            members.push((ty.to_string(), format!("{n}{}", array.concat())));
                        }
                    }
                }
                t if significant(t) => decl.push(t),
                _ => {}
            }
        }
        self.structs.insert(name.to_owned(), members);
    }

    /// A function definition or prototype: remember which parameters are samplers.
    pub(super) fn function(&mut self, words: &[usize]) {
        let Some(open) = words.iter().position(|&i| self.tokens[i] == "(") else {
            return;
        };
        let Some(&name) = open.checked_sub(1).and_then(|n| words.get(n)) else {
            return;
        };
        let mut params = Vec::new();
        let mut is_sampler = false;
        for &i in &words[open + 1..] {
            match self.tokens[i] {
                "," | ")" => {
                    params.push(is_sampler);
                    is_sampler = false;
                }
                t if is_sampler_type(t) => is_sampler = true,
                _ => {}
            }
        }
        if params.iter().any(|&p| p) {
            self.sampler_functions
                .insert(self.tokens[name].to_owned(), params);
        }
    }
}
