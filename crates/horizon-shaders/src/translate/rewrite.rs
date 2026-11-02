//! The second pass: the stage's code written again for naga, line for line, and the
//! declarations of ours it needs.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use naga::ShaderStage;

use super::scan::{
    LATER_KEYWORDS, NOT_CONSTANT, Stage, UNIFORM_STATE, VERTEX_INPUTS, is_ident, is_sampler_type,
    significant,
};
use super::{Sampler, split_sampler};

/// `inverse` for each square matrix: the adjugate over the determinant.
const INVERSE: &str = "mat2 hz_inverse(mat2 m) { return mat2(m[1][1], -m[0][1], -m[1][0], m[0][0]) / determinant(m); }\n\
    mat3 hz_inverse(mat3 m) { vec3 r0 = cross(m[1], m[2]); vec3 r1 = cross(m[2], m[0]); vec3 r2 = cross(m[0], m[1]); \
    return transpose(mat3(r0, r1, r2)) / dot(m[0], r0); }\n\
    mat4 hz_inverse(mat4 m) {\
    float a00 = m[0][0], a01 = m[0][1], a02 = m[0][2], a03 = m[0][3], a10 = m[1][0], a11 = m[1][1], a12 = m[1][2], a13 = m[1][3];\
    float a20 = m[2][0], a21 = m[2][1], a22 = m[2][2], a23 = m[2][3], a30 = m[3][0], a31 = m[3][1], a32 = m[3][2], a33 = m[3][3];\
    float b00 = a00 * a11 - a01 * a10, b01 = a00 * a12 - a02 * a10, b02 = a00 * a13 - a03 * a10, b03 = a01 * a12 - a02 * a11;\
    float b04 = a01 * a13 - a03 * a11, b05 = a02 * a13 - a03 * a12, b06 = a20 * a31 - a21 * a30, b07 = a20 * a32 - a22 * a30;\
    float b08 = a20 * a33 - a23 * a30, b09 = a21 * a32 - a22 * a31, b10 = a21 * a33 - a23 * a31, b11 = a22 * a33 - a23 * a32;\
    float det = b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06;\
    return mat4(a11 * b11 - a12 * b10 + a13 * b09, a02 * b10 - a01 * b11 - a03 * b09, a31 * b05 - a32 * b04 + a33 * b03, a22 * b04 - a21 * b05 - a23 * b03,\
    a12 * b08 - a10 * b11 - a13 * b07, a00 * b11 - a02 * b08 + a03 * b07, a32 * b02 - a30 * b05 - a33 * b01, a20 * b05 - a22 * b02 + a23 * b01,\
    a10 * b10 - a11 * b08 + a13 * b06, a01 * b08 - a00 * b10 - a03 * b06, a30 * b04 - a31 * b02 + a33 * b00, a21 * b02 - a20 * b04 - a23 * b00,\
    a11 * b07 - a10 * b09 - a12 * b06, a00 * b09 - a01 * b07 + a02 * b06, a31 * b01 - a30 * b03 - a32 * b00, a20 * b03 - a21 * b01 + a22 * b00) / det; }\n";

impl Stage<'_> {
    /// Declarations of ours this stage uses (after the uniform block, samplers and varyings).
    pub(super) fn prelude(&self) -> String {
        let mut p = String::from(
            "float hz_modf(float x, out float i) { i = trunc(x); return x - i; }\n\
             vec2 hz_modf(vec2 x, out vec2 i) { i = trunc(x); return x - i; }\n\
             vec3 hz_modf(vec3 x, out vec3 i) { i = trunc(x); return x - i; }\n\
             vec4 hz_modf(vec4 x, out vec4 i) { i = trunc(x); return x - i; }\n\
             struct hz_FogParameters { vec4 color; float density; float start; float end; float scale; };\n\
             hz_FogParameters hz_Fog;\n\
             float hz_zero = 0.0;\n",
        );
        if self.inverse {
            p.push_str(INVERSE);
        }
        if self.gather_offsets {
            p.push_str(&gather_offsets());
        }
        // GLSL's scalar `dot` and `normalize` (genType includes `float`); naga's are vector only.
        if self.scalar_dot {
            p.push_str("float dot(float a, float b) { return a * b; }\n");
        }
        if self.scalar_normalize {
            p.push_str("float normalize(float x) { return sign(x); }\n");
        }
        if self.gbuffers {
            // Set by our `main` from the frame's matrices (gbuffers.rs).
            p.push_str(
                "mat4 hz_ModelViewMatrix; mat4 hz_ProjectionMatrix; mat4 hz_ModelViewProjectionMatrix;\n\
                 mat4 hz_ModelViewMatrixInverse; mat4 hz_ProjectionMatrixInverse; mat3 hz_NormalMatrix; mat4 hz_TextureMatrix[8];\n",
            );
        } else {
            p.push_str(
                "const mat4 hz_ModelViewMatrix = mat4(1.0); const mat4 hz_ProjectionMatrix = mat4(2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, -1.0, -1.0, 0.0, 1.0);\n\
                 const mat4 hz_ModelViewProjectionMatrix = hz_ProjectionMatrix; const mat4 hz_ModelViewMatrixInverse = mat4(1.0); const mat3 hz_NormalMatrix = mat3(1.0);\n\
                 const mat4 hz_ProjectionMatrixInverse = mat4(0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.5, 0.5, 0.0, 1.0);\n\
                 const mat4 hz_TextureMatrix[8] = mat4[8](mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0), mat4(1.0));\n",
            );
        }
        if self.kind == ShaderStage::Vertex {
            p.push_str(
                "const vec2 hz_quad[6] = vec2[6](vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));\n\
                 vec4 hz_Vertex; vec4 hz_Color; vec4 hz_SecondaryColor; vec3 hz_Normal; float hz_FogCoord;\n\
                 vec4 hz_MultiTexCoord0; vec4 hz_MultiTexCoord1; vec4 hz_MultiTexCoord2; vec4 hz_MultiTexCoord3;\n\
                 vec4 hz_MultiTexCoord4; vec4 hz_MultiTexCoord5; vec4 hz_MultiTexCoord6; vec4 hz_MultiTexCoord7;\n",
            );
        } else if self.kind == ShaderStage::Fragment {
            for n in &self.outputs {
                if !self.frag_outs.iter().any(|(at, _)| at == n) {
                    writeln!(p, "layout(location = {n}) out vec4 hz_out{n};").unwrap();
                }
            }
            for (_, decl) in &self.frag_outs {
                writeln!(p, "{decl}").unwrap();
            }
            p.push_str("vec4 hz_FragCoord;\n");
        }
        p
    }

    /// The pack's code with the rewrites above; the same number of lines.
    pub(super) fn rewrite(
        &self,
        samplers: &[Sampler],
        uniforms: &BTreeMap<String, String>,
    ) -> String {
        let removed = |i: usize| self.removed.iter().any(|r| r.contains(&i));
        let sampler_ty: HashMap<&str, &str> = samplers
            .iter()
            .map(|s| (s.name.as_str(), s.ty.as_str()))
            .collect();
        let old = self.pp.version.as_ref().is_none_or(|(v, _)| *v < 400);
        let fragment = self.kind == ShaderStage::Fragment;
        let mut out = String::with_capacity(self.pp.code.len() + 1024);
        // Per open bracket: the function called, and the closing text to add.
        let mut calls: Vec<(&str, &str, usize)> = Vec::new();
        // Sampler parameters of the function being written: name → type.
        let mut params: HashMap<String, String> = HashMap::new();
        // Names of samplers and bool uniforms a local variable or parameter of the function hides.
        // Each with where its scope starts and the block depth it ends with.
        let mut hidden: Vec<(&str, usize, i32)> = Vec::new();
        // Depths of the `for` loops whose body is a single statement, ending at its `;`.
        let mut unbraced_for: Vec<i32> = Vec::new();
        let mut depth = 0;
        let mut skip_until = 0;
        // The `;` that closes a statement we rewrite, and what is written in its place.
        let mut statement_close: Option<(usize, &str)> = None;
        // The type the function being written returns, when GLSL converts to it implicitly.
        let mut returns: Option<&str> = None;
        let next = |i: usize| (i + 1..self.tokens.len()).find(|&j| significant(self.tokens[j]));
        let prev = |i: usize| (0..i).rev().find(|&j| significant(self.tokens[j]));
        let structs: Vec<&str> = (self.tokens.iter().enumerate())
            .filter(|&(_, &t)| t == "struct")
            .filter_map(|(i, _)| next(i).map(|j| self.tokens[j]))
            .collect();
        for (i, &t) in self.tokens.iter().enumerate() {
            if i < skip_until {
                continue;
            }
            if let Some((end, close)) = statement_close
                && end == i
            {
                out.push_str(close);
                statement_close = None;
                // This `;` may end the single-statement body of a `for`.
                if calls.is_empty() && unbraced_for.last() == Some(&depth) {
                    while unbraced_for.last() == Some(&depth) {
                        unbraced_for.pop();
                    }
                    hidden.retain(|&(_, _, d)| d <= depth);
                }
                continue;
            }
            if let Some(text) = self.replaced.get(&i) {
                out.push_str(text);
                continue;
            }
            if removed(i) {
                out.extend(t.chars().filter(|&c| c == '\n'));
                if t.contains(' ') && !t.contains('\n') {
                    out.push(' ');
                }
                continue;
            }
            // Dimensions naga reads in reverse (`T x[A][B]`): written reversed, read as GL does.
            if t == "["
                && let Some(groups) = self.type_dimensions(i, &structs)
            {
                for &(open, close) in groups.iter().rev() {
                    out.extend(self.tokens[open..=close].iter().copied());
                }
                for pair in groups.windows(2) {
                    for &gap in &self.tokens[pair[0].1 + 1..pair[1].0] {
                        out.extend(gap.chars().filter(|&c| c == '\n'));
                    }
                }
                skip_until = groups.last().map_or(i, |g| g.1) + 1;
                continue;
            }
            // `sqrt(-1.0)`: GL's NaN at run time, which naga would fold and refuse.
            if t == "sqrt"
                && let Some(open) = next(i).filter(|&j| self.tokens[j] == "(")
                && let Some(minus) = next(open).filter(|&j| self.tokens[j] == "-")
                && let Some(n) = next(minus).filter(|&j| {
                    self.tokens[j].starts_with(|c: char| c.is_ascii_digit() || c == '.')
                })
                && let Some(close) = next(n).filter(|&j| self.tokens[j] == ")")
                && !is_zero_literal(self.tokens[n])
            {
                out.push_str("(hz_zero / hz_zero)");
                skip_until = close + 1;
                continue;
            }
            // `x / 0.0`: GL's infinity at run time, which naga would fold and refuse.
            if t == "/"
                && let Some(j) = next(i).filter(|&j| is_zero_literal(self.tokens[j]))
            {
                out.push_str("/ hz_zero");
                skip_until = j + 1;
                continue;
            }
            // naga's frontend leaves the code after an unconditional `discard` or `return`
            // unreachable but half-built (a module that fails validation): behind an `if` it
            // stays reachable as far as naga knows. The braces keep a pack's `else` with the
            // pack's `if` (`if (c) discard; else ...`). A `return` with a value is guarded only
            // as a statement of its own with code after it in its block: as an `if`'s branch
            // the code after it is reachable already, and naga would end our guard's empty
            // `else` with a `return;` of its own where the function returns a value.
            // GLSL converts a returned value to the function's type (`return 1;` in a `float`
            // function); naga does not, so the conversion is written out: `return float(1);`.
            if matches!(t, "discard" | "return")
                && let Some(end) = self.statement_end(i)
            {
                let value = t == "return" && next(i) != Some(end);
                let guard = !value
                    || (prev(i).is_none_or(|j| matches!(self.tokens[j], ";" | "{" | "}"))
                        && next(end).is_some_and(|j| self.tokens[j] != "}"));
                let wrap = returns.filter(|_| value && !self.top_level_comma(i, end));
                if guard || wrap.is_some() {
                    if guard {
                        out.push_str("{ if (true) ");
                    }
                    out.push_str(t);
                    if let Some(ty) = wrap {
                        write!(out, " {ty}(").unwrap();
                    }
                    statement_close = Some((
                        end,
                        match (wrap.is_some(), guard) {
                            (true, true) => "); }",
                            (true, false) => ");",
                            (false, _) => "; }",
                        },
                    ));
                    continue;
                }
            }
            let called = next(i).is_some_and(|j| self.tokens[j] == "(");
            match t {
                "{" => {
                    if depth == 0 {
                        returns = self.converted_return(i);
                    }
                    depth += 1;
                }
                "}" => {
                    depth -= 1;
                    hidden.retain(|&(_, _, d)| d <= depth);
                    if depth == 0 {
                        params.clear();
                        hidden.clear();
                        returns = None;
                    }
                }
                ";" if calls.is_empty() && unbraced_for.last() == Some(&depth) => {
                    while unbraced_for.last() == Some(&depth) {
                        unbraced_for.pop();
                    }
                    hidden.retain(|&(_, _, d)| d <= depth);
                }
                // A prototype (`float h(bool x);`) ends its parameters' scope.
                ";" if depth == 0 && calls.is_empty() => {
                    hidden.clear();
                    params.clear();
                }
                "(" => {
                    let callee = prev(i).map_or("", |j| self.tokens[j]);
                    let close = if matches!(callee, "shadow2D" | "shadow2DLod" | "shadow2DProj") {
                        ")"
                    } else {
                        ""
                    };
                    calls.push((callee, close, 0));
                }
                ")" => {
                    if let Some((callee, close, _)) = calls.pop() {
                        out.push_str(close);
                        if callee == "for" && next(i).is_some_and(|j| self.tokens[j] != "{") {
                            unbraced_for.push(depth);
                        }
                    }
                }
                "," => {
                    if let Some(c) = calls.last_mut() {
                        c.2 += 1;
                    }
                }
                _ => {}
            }
            // `T x[] = T[](a, b)`: naga gives unsized arrays no size, so the count is written in.
            if t == "["
                && let Some(close) = next(i).filter(|&j| self.tokens[j] == "]")
            {
                if let Some(n) = self.array_length(close) {
                    write!(out, "[{n}]").unwrap();
                    skip_until = close + 1;
                    continue;
                }
            }
            if !is_ident(t) {
                out.push_str(t);
                continue;
            }
            // A local `const` that calls a built-in naga cannot evaluate: an ordinary local.
            if t == "const" && depth > 0 && self.calls_not_constant(i) {
                continue;
            }
            // A parameter's `const` (`const in float x`, which naga's parser refuses): it only
            // promises the function does not write the parameter.
            if t == "const" && depth == 0 && !calls.is_empty() {
                continue;
            }
            // A function definition's sampler parameters: `sampler2D s` → `texture2D s, sampler hz_s_s`.
            if depth == 0 && is_sampler_type(t) && calls.len() == 1 {
                if let Some(n) = next(i).filter(|&n| is_ident(self.tokens[n])) {
                    let name = self.tokens[n];
                    let (texture, sampler) = split_sampler(t);
                    write!(out, "{texture} {name}, {sampler} hz_s_{name}").unwrap();
                    params.insert(name.to_owned(), t.to_owned());
                    skip_until = n + 1;
                    continue;
                }
            }
            // `float specular`: a variable of the same name as a sampler or a sampler parameter,
            // from the end of its initializer to the function's end; `.specular` a member.
            if sampler_ty.contains_key(t)
                || params.contains_key(t)
                || uniforms.get(t).is_some_and(|ty| ty == "bool")
            {
                let before = prev(i).map(|j| self.tokens[j]);
                if before == Some(".") {
                    out.push_str(t);
                    continue;
                }
                let declares = before
                    .is_some_and(|b| is_ident(b) && !matches!(b, "return" | "case" | "else"))
                    || (before == Some(",")
                        && (calls.is_empty() || calls.last().is_some_and(|c| c.0 == "for"))
                        && self.is_next_declarator(i));
                if declares {
                    // A `for` header's variable ends with the loop's block.
                    let ends = if calls.last().is_some_and(|c| c.0 == "for") {
                        depth + 1
                    } else {
                        depth.max(1)
                    };
                    hidden.push((t, self.scope_start(i), ends));
                    out.push_str(t);
                    continue;
                }
            }
            if hidden.iter().any(|&(h, from, _)| h == t && i >= from) {
                out.push_str(t);
                continue;
            }
            // A sampler named like a function (`texture(texture, uv)`) is the function when called.
            let sampler = params
                .get(t)
                .map(String::as_str)
                .or_else(|| sampler_ty.get(t).copied())
                .filter(|_| !called);
            if let Some(ty) = sampler {
                let as_argument = calls.last().is_some_and(|(callee, _, arg)| {
                    self.sampler_functions
                        .get(*callee)
                        .is_some_and(|p| p.get(*arg) == Some(&true))
                }) && prev(i)
                    .is_some_and(|j| matches!(self.tokens[j], "(" | ","));
                if as_argument {
                    write!(out, "{t}, hz_s_{t}").unwrap();
                } else {
                    write!(out, "{ty}({t}, hz_s_{t})").unwrap();
                }
                continue;
            }
            if uniforms.get(t).is_some_and(|ty| ty == "bool") {
                write!(out, "(hz_bool_{t} != 0)").unwrap();
                continue;
            }
            let replaced: Option<String> = match t {
                "main" if depth == 0 && called => Some("hz_main".into()),
                // GLSL's `modf`, which naga's frontend lacks.
                "modf" if called => Some("hz_modf".into()),
                "textureGatherOffsets" if called => Some("hz_textureGatherOffsets".into()),
                // Matrix inverse, which naga writes for neither Metal nor WGSL.
                "inverse" if called && self.inverse => Some("hz_inverse".into()),
                "texture2D" | "texture1D" | "texture3D" | "textureCube" if called => {
                    Some("texture".into())
                }
                "texture2DLod" | "texture3DLod" | "texture2DLodARB" | "textureCubeLod"
                    if called =>
                {
                    Some("textureLod".into())
                }
                // GL_EXT_gpu_shader4's names (Bliss).
                "texelFetch1D" | "texelFetch2D" | "texelFetch3D" if called => {
                    Some("texelFetch".into())
                }
                "texture2DProj" if called => Some("textureProj".into()),
                "texture2DGrad" | "texture2DGradARB" if called => Some("textureGrad".into()),
                "shadow2D" if called => Some("vec4(texture".into()),
                "shadow2DLod" if called => Some("vec4(textureLod".into()),
                "shadow2DProj" if called => Some("vec4(textureProj".into()),
                "ftransform" if called => {
                    // `ftransform()` → the product it stands for; its `()` stays as a no-op call... so skip it.
                    let close = next(i).and_then(next);
                    skip_until = close.map_or(i + 1, |c| c + 1);
                    Some("(hz_ModelViewProjectionMatrix * hz_Vertex)".into())
                }
                "gl_FragColor" => Some("hz_out0".into()),
                "gl_FragData" | "gl_TexCoord" => match self.index_after(i) {
                    Some((n, close)) => {
                        skip_until = close + 1;
                        Some(match t {
                            "gl_FragData" => format!("hz_out{n}"),
                            _ => format!("hz_TexCoord{n}"),
                        })
                    }
                    None => None,
                },
                "gl_Color" if fragment => Some("hz_FrontColor".into()),
                "gl_FragCoord" if !fragment => None,
                "gl_FragCoord" => Some("hz_FragCoord".into()),
                "gl_FrontColor" => Some("hz_FrontColor".into()),
                "gl_FogFragCoord" => Some("hz_FogFragCoord".into()),
                "gl_Fog" => Some("hz_Fog".into()),
                // Before the compatibility inputs, of which `gl_Vertex` is a prefix.
                "gl_VertexID" => Some("gl_VertexIndex".into()),
                "gl_InstanceID" => Some("gl_InstanceIndex".into()),
                "gl_VertexIndex" | "gl_InstanceIndex" => None,
                _ if UNIFORM_STATE.iter().any(|b| t.starts_with(b)) => {
                    Some(format!("hz_{}", &t[3..]))
                }
                _ if !fragment && VERTEX_INPUTS.iter().any(|b| t.starts_with(b)) => {
                    Some(format!("hz_{}", &t[3..]))
                }
                _ if old && LATER_KEYWORDS.contains(&t) => Some(format!("hz_kw_{t}")),
                _ => None,
            };
            out.push_str(replaced.as_deref().unwrap_or(t));
        }
        out
    }
}

impl Stage<'_> {
    /// Whether the declaration after token `i` calls a built-in in `NOT_CONSTANT` before its `;`.
    /// The return type of the function whose body opens at token `brace`, when it is one GLSL
    /// converts returned values to (a scalar or vector of `float`, `int` or `uint`).
    pub(super) fn converted_return(&self, brace: usize) -> Option<&str> {
        let prev = |i: usize| (0..i).rev().find(|&j| significant(self.tokens[j]));
        let close = prev(brace).filter(|&j| self.tokens[j] == ")")?;
        let mut depth = 0usize;
        let open = (0..=close).rev().find(|&j| match self.tokens[j] {
            ")" => {
                depth += 1;
                false
            }
            "(" => {
                depth -= 1;
                depth == 0
            }
            _ => false,
        })?;
        let ty = self.tokens[prev(prev(open)?)?];
        matches!(
            ty,
            "float"
                | "vec2"
                | "vec3"
                | "vec4"
                | "int"
                | "ivec2"
                | "ivec3"
                | "ivec4"
                | "uint"
                | "uvec2"
                | "uvec3"
                | "uvec4"
        )
        .then_some(ty)
    }

    /// Whether tokens `i..end` hold a comma outside brackets (GLSL's comma operator).
    pub(super) fn top_level_comma(&self, i: usize, end: usize) -> bool {
        let mut depth = 0usize;
        self.tokens[i..end].iter().any(|&t| match t {
            "(" | "[" => {
                depth += 1;
                false
            }
            ")" | "]" => {
                depth = depth.saturating_sub(1);
                false
            }
            "," => depth == 0,
            _ => false,
        })
    }

    /// The `;` that ends the statement starting at token `i` (outside brackets).
    pub(super) fn statement_end(&self, i: usize) -> Option<usize> {
        let mut depth = 0usize;
        for j in i + 1..self.tokens.len() {
            match self.tokens[j] {
                "(" | "[" => depth += 1,
                ")" | "]" => depth = depth.saturating_sub(1),
                ";" if depth == 0 => return Some(j),
                "{" | "}" => return None,
                _ => {}
            }
        }
        None
    }

    pub(super) fn calls_not_constant(&self, i: usize) -> bool {
        for (j, &t) in self.tokens.iter().enumerate().skip(i + 1) {
            if matches!(t, ";" | "{" | "}") {
                return false;
            }
            if NOT_CONSTANT.contains(&t)
                && self.tokens[j + 1..].iter().find(|t| significant(t)) == Some(&"(")
            {
                return true;
            }
        }
        false
    }

    /// When the `[` at token `i` begins two or more array dimensions of a type (after a type
    /// name, `float[2][3]`, or after the name a type declares, `float x[2][3]`), each one's
    /// first and last token. naga's frontend nests such dimensions the other way round.
    fn type_dimensions(&self, i: usize, structs: &[&str]) -> Option<Vec<(usize, usize)>> {
        let prev = |k: usize| (0..k).rev().find(|&j| significant(self.tokens[j]));
        let is_type = |t: &str| TYPE_NAMES.contains(&t) || structs.contains(&t);
        let p = prev(i)?;
        let declared = is_ident(self.tokens[p])
            && prev(p).is_some_and(|q| is_type(self.tokens[q]) || self.tokens[q] == "]");
        if !is_type(self.tokens[p]) && !declared {
            return None;
        }
        let mut groups = Vec::new();
        let mut open = i;
        loop {
            let mut level = 0;
            let mut close = None;
            for j in open..self.tokens.len() {
                match self.tokens[j] {
                    "[" => level += 1,
                    "]" => {
                        level -= 1;
                        if level == 0 {
                            close = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let close = close?;
            // `[]` keeps its own handling (the constructor's count).
            if (open + 1..close).all(|j| !significant(self.tokens[j])) {
                return None;
            }
            groups.push((open, close));
            match (close + 1..self.tokens.len()).find(|&j| significant(self.tokens[j])) {
                Some(j) if self.tokens[j] == "[" => open = j,
                _ => break,
            }
        }
        (groups.len() > 1).then_some(groups)
    }

    /// Whether the name at `i`, after a `,`, is another declarator of the statement's type
    /// (`bool a = 1, x = 0;`): the statement starts with a type and a name.
    fn is_next_declarator(&self, i: usize) -> bool {
        let follows = (i + 1..self.tokens.len())
            .find(|&j| significant(self.tokens[j]))
            .map(|j| self.tokens[j]);
        if !matches!(follows, Some("=" | "," | ";" | "[")) {
            return false;
        }
        let mut level = 0i32;
        let mut start = i;
        let mut j = i;
        while j > 0 {
            j -= 1;
            let t = self.tokens[j];
            if !significant(t) {
                continue;
            }
            match t {
                ")" | "]" => level += 1,
                "(" | "[" if level > 0 => level -= 1,
                "(" | "[" | "{" | "}" | ";" if level == 0 => break,
                _ => {}
            }
            start = j;
        }
        is_ident(self.tokens[start])
            && !matches!(self.tokens[start], "return" | "case" | "else" | "do")
            && (start + 1..i)
                .find(|&j| significant(self.tokens[j]))
                .is_some_and(|j| is_ident(self.tokens[j]))
    }

    /// Where the scope of the name declared at token `i` begins: after its initializer (GLSL 4.60
    /// 4.2.2: `float depth = texture(depth, uv).x` still reads the outer `depth`), else after it.
    fn scope_start(&self, i: usize) -> usize {
        let after = (i + 1..self.tokens.len()).find(|&j| significant(self.tokens[j]));
        if after.is_none_or(|j| self.tokens[j] != "=") {
            return i + 1;
        }
        let mut level = 0i32;
        for j in i + 1..self.tokens.len() {
            match self.tokens[j] {
                "(" | "[" | "{" => level += 1,
                ")" | "]" | "}" => level -= 1,
                "," | ";" if level <= 0 => return j,
                _ => {}
            }
        }
        self.tokens.len()
    }

    /// The element count of an unsized array whose `[]` ends at `close`: the arguments of the
    /// constructor that follows (`[] = T[](...)` or `[](...)`).
    pub(super) fn array_length(&self, close: usize) -> Option<usize> {
        let mut j = close + 1;
        let mut seen_eq = false;
        while let Some(&t) = self.tokens.get(j) {
            match t {
                "(" => break,
                "=" if !seen_eq => seen_eq = true,
                ";" | "," | ")" | "{" => return None,
                _ => {}
            }
            j += 1;
        }
        let (mut depth, mut args, mut any) = (0, 1, false);
        for &t in &self.tokens[j + 1..] {
            match t {
                "(" | "[" | "{" => depth += 1,
                ")" if depth == 0 => return any.then_some(args),
                ")" | "]" | "}" => depth -= 1,
                "," if depth == 0 => args += 1,
                _ if significant(t) => any = true,
                _ => {}
            }
        }
        None
    }
}

/// What our vertex `main` gives a vertex input of a composite-style program.
/// GLSL's scalar, vector and matrix type names.
const TYPE_NAMES: [&str; 35] = [
    "float", "int", "uint", "bool", "double", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4",
    "uvec2", "uvec3", "uvec4", "bvec2", "bvec3", "bvec4", "dvec2", "dvec3", "dvec4", "mat2",
    "mat3", "mat4", "mat2x2", "mat2x3", "mat2x4", "mat3x2", "mat3x3", "mat3x4", "mat4x2", "mat4x3",
    "mat4x4", "dmat2", "dmat3", "dmat4",
];

/// `0.0`, `0.`, `.0`, `0.0f`: a floating-point literal zero.
/// `textureGatherOffsets` (GLSL 4.60, 8.9.4), which naga's frontend lacks: texel `i` of the
/// result is texel `i` of the footprint `offsets[i]` texels away. WGSL takes only constant
/// offsets, so each footprint is gathered at `P` moved by its offset over the texture's size,
/// which picks the same four texels; the component, a constant in GLSL, picks one of four
/// gathers.
fn gather_offsets() -> String {
    let mut p = String::new();
    for prefix in ["", "i", "u"] {
        let (tex, smp) = (format!("{prefix}texture2D"), format!("{prefix}sampler2D"));
        let vec = format!("{prefix}vec4");
        write!(
            p,
            "{vec} hz_gather({tex} t, sampler s, vec2 p, int c) {{\n\
             \x20   if (c == 1) return textureGather({smp}(t, s), p, 1);\n\
             \x20   if (c == 2) return textureGather({smp}(t, s), p, 2);\n\
             \x20   if (c == 3) return textureGather({smp}(t, s), p, 3);\n\
             \x20   return textureGather({smp}(t, s), p, 0);\n}}\n\
             {vec} hz_textureGatherOffsets({tex} t, sampler s, vec2 p, ivec2 o[4], int c) {{\n\
             \x20   vec2 px = 1.0 / vec2(textureSize({smp}(t, s), 0));\n\
             \x20   return {vec}(hz_gather(t, s, p + vec2(o[0]) * px, c).x, hz_gather(t, s, p + vec2(o[1]) * px, c).y,\n\
             \x20       hz_gather(t, s, p + vec2(o[2]) * px, c).z, hz_gather(t, s, p + vec2(o[3]) * px, c).w);\n}}\n\
             {vec} hz_textureGatherOffsets({tex} t, sampler s, vec2 p, ivec2 o[4]) {{\n\
             \x20   return hz_textureGatherOffsets(t, s, p, o, 0);\n}}\n"
        )
        .unwrap();
    }
    p.push_str(
        "vec4 hz_textureGatherOffsets(texture2D t, samplerShadow s, vec2 p, float r, ivec2 o[4]) {\n\
         \x20   vec2 px = 1.0 / vec2(textureSize(sampler2DShadow(t, s), 0));\n\
         \x20   return vec4(textureGather(sampler2DShadow(t, s), p + vec2(o[0]) * px, r).x,\n\
         \x20       textureGather(sampler2DShadow(t, s), p + vec2(o[1]) * px, r).y,\n\
         \x20       textureGather(sampler2DShadow(t, s), p + vec2(o[2]) * px, r).z,\n\
         \x20       textureGather(sampler2DShadow(t, s), p + vec2(o[3]) * px, r).w);\n}\n",
    );
    p
}

fn is_zero_literal(t: &str) -> bool {
    let t = t.trim_end_matches(['f', 'F']);
    t.contains('.') && !t.contains(['e', 'E']) && t.chars().all(|c| c == '0' || c == '.')
}

pub(super) fn attribute_value(name: &str, ty: &str) -> String {
    match name {
        "vaPosition" => "vec3(hz_p, 0.0)".into(),
        "vaUV0" => "hz_p".into(),
        "vaColor" => "vec4(1.0)".into(),
        "vaNormal" => "vec3(0.0, 0.0, 1.0)".into(),
        _ => format!("{ty}(0)"),
    }
}
