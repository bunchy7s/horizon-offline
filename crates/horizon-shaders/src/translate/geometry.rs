//! A program with a geometry stage (`shadow.gsh`), which wgpu lacks: its vertex, geometry and
//! fragment stages become one vertex stage and the fragment stage. The game draws its triangles
//! without an index buffer, three vertices each, and one instance per triangle the geometry stage
//! can emit: vertex invocation `v` of instance `t` stands for corner `v % 3` of the stage's
//! triangle `t`. It reads its input triangle's three vertices itself (`GeometryInputs::index`,
//! `pull`), runs the pack's vertex stage on each, then the geometry stage, keeping the corner it
//! stands for; a corner the stage does not emit is degenerate.
//!
//! The geometry stage runs once per output corner; the image and buffer writes it makes as
//! statements run in one of them only (`hz_primary`).

use std::collections::BTreeSet;
use std::fmt::Write;

use super::scan::significant;
use super::{Error, GbuffersInputs, Program, Target};
use crate::Preprocessed;
use crate::macros::lex;

/// How a geometry program's vertex invocations reach their triangle's vertices.
#[derive(Clone, Debug, Default)]
pub struct GeometryInputs {
    /// The game's vertex inputs, as `GbuffersInputs`; their `layout(location = n) in`
    /// declarations become plain globals the pull sets.
    pub inputs: GbuffersInputs,
    /// GLSL statements that set the vertex inputs from vertex `hz_vertex_index` (a `uint`).
    pub pull: String,
    /// The vertex of the `@`th corner of the draw (a `uint` expression, `hz_indices[@]`).
    pub index: String,
}

/// Translates `vertex`, `geometry` and `fragment` (a shadow program): the program, and how many
/// triangles the geometry stage can emit (the instances to draw).
pub fn translate_geometry(
    vertex: &Preprocessed,
    geometry: &Preprocessed,
    fragment: &Preprocessed,
    inputs: &GeometryInputs,
) -> Result<(Program, u32), Vec<Error>> {
    let (combined, triangles) = combine(vertex, geometry, inputs)?;
    let mut gbuffers = inputs.inputs.clone();
    gbuffers.declarations = gbuffers
        .declarations
        .lines()
        .map(|l| match l.find(") in ") {
            Some(at) if l.starts_with("layout(location") => &l[at + 5..],
            _ => l,
        })
        .fold(String::new(), |s, l| s + l + "\n");
    // The pack's vertex stage reads its inputs in our loop, not in `main`.
    gbuffers.body = String::new();
    gbuffers.attributes = Vec::new();
    let program = super::translate(&combined, fragment, Target::Buffers, Some(&gbuffers), true)?;
    Ok((program, triangles))
}

/// A global statement's tokens and what it declares.
struct Statement {
    range: std::ops::Range<usize>,
    kind: Kind,
}

enum Kind {
    /// `in`, `out`, `attribute`, `varying`: storage, names and types (with array suffixes).
    Interface(&'static str, Vec<(String, String)>),
    /// `layout(...) in;` or `out;` without a variable.
    Layout(&'static str),
    Buffer(String),
    Uniform,
    /// Functions, variables, structs: the names it defines.
    Defines(Vec<String>),
}

fn statements(tokens: &[&str]) -> Vec<Statement> {
    let mut out = Vec::new();
    let (mut start, mut parens) = (0, 0usize);
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "(" => parens += 1,
            ")" => parens = parens.saturating_sub(1),
            "{" if parens == 0 => {
                // A function body, a struct or a block: to its matching `}` (and `;`).
                let mut d = 0;
                let mut j = i;
                while j < tokens.len() {
                    match tokens[j] {
                        "{" => d += 1,
                        "}" => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let mut end = j + 1;
                let head: Vec<&str> = tokens[start..i]
                    .iter()
                    .copied()
                    .filter(|t| significant(t))
                    .collect();
                // `struct S { } ;`, `buffer B { } name;`: to the `;`.
                if !head.contains(&"(") || head.contains(&"buffer") {
                    while end < tokens.len() && tokens[end] != ";" {
                        end += 1;
                    }
                    end += 1;
                }
                out.push(classify(tokens, start..end.min(tokens.len())));
                start = end.min(tokens.len());
                i = start;
                continue;
            }
            ";" if parens == 0 => {
                out.push(classify(tokens, start..i + 1));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn classify(tokens: &[&str], range: std::ops::Range<usize>) -> Statement {
    let words: Vec<&str> = tokens[range.clone()]
        .iter()
        .copied()
        .filter(|t| significant(t))
        .collect();
    // A storage word before any `(` but a leading `layout(...)`'s: in a function's parameters
    // `in` and `out` are qualifiers.
    let skip = if words.first() == Some(&"layout") {
        words.iter().position(|w| *w == ")").map_or(0, |p| p + 1)
    } else {
        0
    };
    let head = match words[skip..].iter().position(|w| *w == "(") {
        Some(p) => &words[skip..skip + p],
        None => &words[skip..],
    };
    let storage = ["in", "out", "attribute", "varying"]
        .into_iter()
        .find(|s| head.contains(s));
    let kind = if let Some(p) = words.iter().position(|w| *w == "buffer") {
        Kind::Buffer(words.get(p + 1).map_or(String::new(), |s| s.to_string()))
    } else if words.contains(&"uniform") {
        Kind::Uniform
    } else if let Some(storage) = storage {
        // Skip `layout(...)`, then qualifiers, then the type and the declarators.
        let mut k = 0;
        if words.first() == Some(&"layout") {
            while k < words.len() && words[k] != ")" {
                k += 1;
            }
            k += 1;
        }
        let quals = [
            "in",
            "out",
            "attribute",
            "varying",
            "flat",
            "smooth",
            "noperspective",
            "centroid",
            "invariant",
            "highp",
            "mediump",
            "lowp",
            "const",
        ];
        while k < words.len() && quals.contains(&words[k]) {
            k += 1;
        }
        match words.get(k) {
            None | Some(&";") => Kind::Layout(storage),
            Some(ty) => {
                let mut vars = Vec::new();
                let mut j = k + 1;
                while j < words.len() && words[j] != ";" {
                    let name = words[j].to_owned();
                    let mut array = String::new();
                    j += 1;
                    while j < words.len() && !matches!(words[j], "," | ";") {
                        array.push_str(words[j]);
                        j += 1;
                    }
                    j += 1;
                    vars.push((name, format!("{ty}{array}")));
                }
                Kind::Interface(storage, vars)
            }
        }
    } else {
        let mut names = Vec::new();
        let first = words.iter().position(|w| matches!(*w, "(" | "="));
        if let Some(p) = words.iter().position(|w| *w == "struct") {
            names.extend(words.get(p + 1).map(|s| s.to_string()));
        } else if let Some(p) = first.filter(|&p| words[p] == "(") {
            // A function.
            names.extend(p.checked_sub(1).map(|n| words[n].to_owned()));
        } else {
            // Variables: the last identifier before each declarator's `=` or end.
            let (mut depth, mut last, mut init) = (0, None, false);
            for w in &words {
                match *w {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    "=" if depth == 0 => {
                        names.extend(last.take());
                        init = true;
                    }
                    "," | ";" if depth == 0 => {
                        if !init {
                            names.extend(last.take());
                        }
                        (last, init) = (None, false);
                    }
                    w if depth == 0 && !init && super::scan::is_ident(w) => {
                        last = Some(w.to_owned())
                    }
                    _ => {}
                }
            }
        }
        names.dedup();
        Kind::Defines(names)
    };
    Statement { range, kind }
}

/// The combined vertex stage's source, and the triangles the geometry stage can emit.
fn combine(
    vertex: &Preprocessed,
    geometry: &Preprocessed,
    inputs: &GeometryInputs,
) -> Result<(Preprocessed, u32), Vec<Error>> {
    let error = |message: String| {
        vec![Error {
            file: geometry.files.first().cloned().unwrap_or_default(),
            line: 0,
            message,
        }]
    };
    let gs_tokens = lex(&geometry.code);
    let vs_tokens = lex(&vertex.code);
    let gs = statements(&gs_tokens);
    let vs = statements(&vs_tokens);

    // The geometry stage's interface.
    let mut gs_ins: Vec<(String, String)> = Vec::new();
    let mut gs_outs: Vec<(String, String)> = Vec::new();
    let mut gs_buffers = BTreeSet::new();
    let mut max_vertices = None;
    let mut gs_text: Vec<String> = gs_tokens.iter().map(|t| t.to_string()).collect();
    for s in &gs {
        match &s.kind {
            Kind::Interface("in", vars) => {
                // The input primitive is a triangle: `in vec3 v[]` has three elements.
                let vars: Vec<(String, String)> = vars
                    .iter()
                    .map(|(n, ty)| (n.clone(), ty.replace("[]", "[3]")))
                    .collect();
                gs_ins.extend(vars.iter().cloned());
                // One declaration per name (naga types `T a[3], b[3]`'s second wrong).
                let decls: String = vars
                    .iter()
                    .map(|(n, ty)| {
                        let (base, array) = split_type(ty);
                        format!("{base} {n}{array}; ")
                    })
                    .collect();
                blank(&mut gs_text, s.range.clone());
                gs_text[s.range.start] = decls + &gs_text[s.range.start];
            }
            Kind::Interface("out", vars) => gs_outs.extend(vars.iter().cloned()),
            Kind::Layout(storage) => {
                if *storage == "out" {
                    let words: Vec<&str> = gs_tokens[s.range.clone()]
                        .iter()
                        .copied()
                        .filter(|t| significant(t))
                        .collect();
                    if let Some(p) = words.iter().position(|w| *w == "max_vertices") {
                        max_vertices = words.get(p + 2).and_then(|n| n.parse::<u32>().ok());
                    }
                }
                blank(&mut gs_text, s.range.clone());
            }
            Kind::Buffer(name) => _ = gs_buffers.insert(name.clone()),
            _ => {}
        }
    }
    let Some(max_vertices) = max_vertices else {
        return Err(error("the geometry stage has no `max_vertices`".into()));
    };
    for (i, t) in gs_tokens.iter().enumerate() {
        let prev = (0..i).rev().find(|&j| significant(gs_tokens[j]));
        let next = (i + 1..gs_tokens.len()).find(|&j| significant(gs_tokens[j]));
        let called = next.is_some_and(|j| gs_tokens[j] == "(");
        match *t {
            "EmitVertex" if called => gs_text[i] = "hz_emit".into(),
            "EndPrimitive" if called => gs_text[i] = "hz_end".into(),
            "gl_in" => gs_text[i] = "hz_gl_in".into(),
            "gl_Position" if prev.is_some_and(|j| gs_tokens[j] == ".") => gs_text[i] = "p".into(),
            "main" if called => gs_text[i] = "hz_gs_main".into(),
            _ => {}
        }
    }

    // The vertex stage: its globals renamed, its outputs plain globals, its buffer blocks the
    // geometry stage's.
    let mut defined: BTreeSet<String> = BTreeSet::new();
    let mut vs_outs: Vec<(String, String)> = Vec::new();
    let mut vs_attributes: Vec<(String, String)> = Vec::new();
    let mut vs_text: Vec<String> = vs_tokens.iter().map(|t| t.to_string()).collect();
    for s in &vs {
        match &s.kind {
            Kind::Defines(names) => defined.extend(names.iter().cloned()),
            Kind::Interface("out" | "varying", vars) => {
                // Declared again, renamed, in our head.
                vs_outs.extend(vars.iter().cloned());
                blank(&mut vs_text, s.range.clone());
            }
            Kind::Interface("in" | "attribute", vars) => {
                vs_attributes.extend(vars.iter().cloned());
                strip_storage(&mut vs_text, &vs_tokens, s.range.clone());
            }
            Kind::Buffer(name) if gs_buffers.contains(name) => blank(&mut vs_text, s.range.clone()),
            Kind::Layout(_) => blank(&mut vs_text, s.range.clone()),
            _ => {}
        }
    }
    let renamed: BTreeSet<&str> = defined
        .iter()
        .map(String::as_str)
        .chain(vs_outs.iter().map(|(n, _)| n.as_str()))
        .collect();
    for (i, t) in vs_tokens.iter().enumerate() {
        if vs_text[i] != *t {
            continue;
        }
        let prev = (0..i).rev().find(|&j| significant(vs_tokens[j]));
        if prev.is_some_and(|j| vs_tokens[j] == ".") {
            continue;
        }
        if *t == "gl_Position" {
            vs_text[i] = "hz_vs_Position".into();
        } else if matches!(*t, "gl_VertexID" | "gl_VertexIndex") {
            // The vertex the pack's vertex stage runs for, not this invocation.
            vs_text[i] = "hz_vs_vertex".into();
        } else if matches!(*t, "gl_InstanceID" | "gl_InstanceIndex") {
            // The game draws terrain uninstanced; this invocation's instance is a triangle.
            vs_text[i] = "0".into();
        } else if renamed.contains(t) {
            vs_text[i] = format!("hz_vs_{t}");
        }
    }

    once(&mut gs_text, &gs_tokens);
    once(&mut vs_text, &vs_tokens);

    // Ours: before the geometry stage, its emission; after the vertex stage, `main`.
    let mut head = String::from(
        "void hz_emit(); void hz_end(); struct hz_PerVertex { vec4 p; }; hz_PerVertex hz_gl_in[3];\n\
         int hz_n; int hz_strip; int hz_tri; int hz_want_tri; int hz_want_corner; bool hz_found; bool hz_primary;\n\
         vec4 hz_ring_Position[3]; vec4 hz_res_Position; vec4 hz_vs_Position; int hz_vs_vertex;\n",
    );
    for (name, ty) in &vs_outs {
        let (base, array) = split_type(ty);
        writeln!(head, "{base} hz_vs_{name}{array};").unwrap();
    }
    let mut emit = String::from(
        "void hz_emit() {\n    int hz_s = hz_n % 3;\n    hz_ring_Position[hz_s] = gl_Position;\n",
    );
    let mut take = String::from("        hz_res_Position = hz_ring_Position[hz_r];\n");
    let mut result = String::from("    gl_Position = hz_found ? hz_res_Position : vec4(0.0);\n");
    for (name, ty) in &gs_outs {
        let (base, array) = split_type(ty);
        if !array.is_empty() {
            return Err(error(format!(
                "`{name}`: array outputs of a geometry stage are not supported yet"
            )));
        }
        writeln!(head, "{base} hz_ring_{name}[3]; {base} hz_res_{name};").unwrap();
        writeln!(emit, "    hz_ring_{name}[hz_s] = {name};").unwrap();
        writeln!(take, "        hz_res_{name} = hz_ring_{name}[hz_r];").unwrap();
        writeln!(result, "    {name} = hz_res_{name};").unwrap();
    }
    write!(
        emit,
        "    hz_n++;\n    hz_strip++;\n    if (hz_strip >= 3) {{\n        if (hz_tri == hz_want_tri) {{\n            \
         int hz_k = hz_want_corner;\n            if (((hz_strip - 3) & 1) == 1 && hz_k < 2) {{ hz_k = 1 - hz_k; }}\n            \
         int hz_r = (hz_n - 3 + hz_k) % 3;\n{take}            hz_found = true;\n        }}\n        hz_tri++;\n    }}\n}}\n\
         void hz_end() {{ hz_strip = 0; }}\n"
    )
    .unwrap();
    let body = inputs
        .inputs
        .body
        .replace("gl_VertexIndex", "hz_vertex_index");
    let mut main = String::from(
        "void main() {\n    uint hz_v = uint(gl_VertexIndex);\n    uint hz_first = hz_v / 3u * 3u;\n    \
         hz_want_corner = int(hz_v % 3u);\n    hz_want_tri = int(gl_InstanceIndex);\n    \
         hz_primary = hz_want_tri == 0 && hz_want_corner == 0;\n    \
         for (int hz_i = 0; hz_i < 3; hz_i++) {\n",
    );
    writeln!(
        main,
        "        uint hz_vertex_index = {};",
        inputs.index.replace('@', "hz_first + uint(hz_i)")
    )
    .unwrap();
    main.push_str("        hz_vs_vertex = int(hz_vertex_index);\n");
    main.push_str(&inputs.pull);
    main.push_str(&body);
    for (name, ty) in &vs_attributes {
        let value = inputs
            .inputs
            .attributes
            .iter()
            .find(|a| &a.0 == name)
            .map_or_else(|| format!("{ty}(0)"), |a| format!("{ty}({})", a.1));
        writeln!(main, "        {name} = {value};").unwrap();
    }
    main.push_str("        hz_vs_main();\n");
    for (name, _) in &vs_outs {
        if gs_ins.iter().any(|(n, _)| n == name) {
            writeln!(main, "        {name}[hz_i] = hz_vs_{name};").unwrap();
        }
    }
    main.push_str("        hz_gl_in[hz_i].p = hz_vs_Position;\n    }\n    hz_gs_main();\n");
    main.push_str(&result);
    main.push_str("}\n");

    // The source, lines mapped: ours (no file), the geometry stage, ours, the vertex stage, ours.
    let mut out = Preprocessed {
        version: geometry.version.clone().or(vertex.version.clone()),
        files: geometry.files.clone(),
        ..Default::default()
    };
    let offset = out.files.len() as u16;
    out.files.extend(vertex.files.iter().cloned());
    let mut code = String::new();
    let ours = |code: &mut String, lines: &mut Vec<crate::Origin>, text: &str| {
        for l in text.lines() {
            code.push_str(l);
            code.push('\n');
            lines.push(crate::Origin::default());
        }
    };
    ours(&mut code, &mut out.lines, &head);
    code.push_str(&gs_text.concat());
    code.push('\n');
    out.lines.extend(geometry.lines.iter().copied());
    ours(&mut code, &mut out.lines, &emit);
    code.push_str(&vs_text.concat());
    code.push('\n');
    out.lines.extend(vertex.lines.iter().map(|o| crate::Origin {
        file: o.file + offset,
        line: o.line,
    }));
    ours(&mut code, &mut out.lines, &main);
    out.code = code.trim_end_matches('\n').to_owned();
    out.lines.truncate(out.code.split('\n').count());
    // A strip of n vertices is n - 2 triangles: at most `max_vertices - 2`, whichever of the
    // stage's paths runs.
    let triangles = max_vertices.saturating_sub(2).max(1);
    Ok((out, triangles))
}

/// Functions whose calls write memory other invocations see.
fn writes(name: &str) -> bool {
    name == "imageStore" || name.starts_with("imageAtomic") || name.starts_with("atomic")
}

/// Calls that write images or buffers run for one invocation of a triangle only: as statements,
/// `if (!hz_primary) {} else imageStore(...)`; in an expression the others read the value instead,
/// `(hz_primary ? imageAtomicOr(i, p, v) : imageLoad(i, p).x)`, `(hz_primary ? atomicAdd(m, v) : m)`.
fn once(text: &mut [String], tokens: &[&str]) {
    for (i, &t) in tokens.iter().enumerate() {
        let open = (i + 1..tokens.len()).find(|&j| significant(tokens[j]));
        let Some(open) = open.filter(|&j| tokens[j] == "(") else {
            continue;
        };
        if !writes(t) || text[i] != t {
            continue;
        }
        let prev = (0..i).rev().find(|&j| significant(tokens[j]));
        if prev.is_none_or(|j| matches!(tokens[j], ";" | "{" | "}" | ")" | "else")) {
            // A whole if/else, so that an `else` after the statement stays the pack's.
            text[i] = format!("if (!hz_primary) {{}} else {t}");
            continue;
        }
        // The call's arguments, at its top-level commas.
        let (mut depth, mut args, mut from, mut close) = (0, Vec::new(), open + 1, None);
        for (j, &u) in tokens.iter().enumerate().skip(open) {
            match u {
                "(" | "[" => depth += 1,
                ")" | "]" if depth == 1 && u == ")" => {
                    args.push(tokens[from..j].concat());
                    close = Some(j);
                    break;
                }
                ")" | "]" => depth -= 1,
                "," if depth == 1 => {
                    args.push(tokens[from..j].concat());
                    from = j + 1;
                }
                _ => {}
            }
        }
        let Some(close) = close else { continue };
        let read = if t.starts_with("imageAtomic") && args.len() >= 2 {
            format!("imageLoad({}, {}).x", args[0], args[1])
        } else if t.starts_with("atomic") && !args.is_empty() {
            format!("({})", args[0])
        } else {
            continue;
        };
        text[i] = format!("(hz_primary ? {t}");
        text[close] = format!(") : {read})");
    }
}

/// `vec3[2]` or `vec3` → (`vec3`, `[2]`).
fn split_type(ty: &str) -> (&str, &str) {
    match ty.find('[') {
        Some(at) => (&ty[..at], &ty[at..]),
        None => (ty, ""),
    }
}

/// Blanks a statement, keeping its new lines.
fn blank(text: &mut [String], range: std::ops::Range<usize>) {
    for t in &mut text[range] {
        *t = t.chars().filter(|&c| c == '\n').collect();
    }
}

/// Removes a declaration's `layout(...)`, storage and interpolation words: a plain global.
fn strip_storage(text: &mut [String], tokens: &[&str], range: std::ops::Range<usize>) {
    let mut in_layout = false;
    for i in range {
        match tokens[i] {
            "layout" => in_layout = true,
            ")" if in_layout => {
                in_layout = false;
                text[i] = " ".into();
                continue;
            }
            _ => {}
        }
        if in_layout
            || matches!(
                tokens[i],
                "in" | "out"
                    | "attribute"
                    | "varying"
                    | "flat"
                    | "smooth"
                    | "noperspective"
                    | "centroid"
                    | "invariant"
            )
        {
            text[i] = if tokens[i].contains('\n') {
                "\n".into()
            } else {
                " ".into()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;
    use crate::{Macros, Pack, preprocess};

    #[test]
    fn a_geometry_stage_folds_into_the_vertex_stage() {
        let vsh = "#version 430\nout vec3 vPos;\nout vec4 vColor;\nvoid main() {\n    \
                   vPos = gl_Vertex.xyz;\n    vColor = gl_Color * float(gl_InstanceID + 1);\n    \
                   gl_Position = ftransform();\n}\n";
        let gsh = "#version 430\nlayout(triangles) in;\nlayout(triangle_strip, max_vertices = 4) out;\n\
                   in vec3 vPos[];\nin vec4 vColor[];\nout vec4 gColor;\nvoid main() {\n    \
                   for (int i = 0; i < 3; i++) {\n        gColor = vColor[i];\n        \
                   gl_Position = gl_in[i].gl_Position;\n        EmitVertex();\n    }\n    \
                   gl_Position = vec4(vPos[0], 1.0);\n    EmitVertex();\n    EndPrimitive();\n}\n";
        let fsh = "#version 430\nin vec4 gColor;\nvoid main() {\n    gl_FragData[0] = gColor;\n}\n";
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                ("shaders/shadow.vsh", vsh),
                ("shaders/shadow.gsh", gsh),
                ("shaders/shadow.fsh", fsh),
            ]),
        )
        .unwrap();
        let pp = |file: &str| preprocess(&pack, file, &Macros::standard());
        let inputs = GeometryInputs {
            inputs: GbuffersInputs {
                declarations: "layout(set = 3, binding = 1) readonly buffer HzVertices { uint hz_vertices[]; };\n\
                               layout(location = 0) in vec3 hz_v_pos;"
                    .into(),
                body: "    hz_Vertex = vec4(hz_v_pos, 1.0);\n    hz_Color = vec4(1.0);\n".into(),
                ..Default::default()
            },
            pull: "        hz_v_pos = uintBitsToFloat(uvec3(hz_vertices[hz_vertex_index * 3u], \
                   hz_vertices[hz_vertex_index * 3u + 1u], hz_vertices[hz_vertex_index * 3u + 2u]));\n"
                .into(),
            index: "uint(@)".into(),
        };
        let (program, triangles) = translate_geometry(
            &pp("shadow.vsh"),
            &pp("shadow.gsh"),
            &pp("shadow.fsh"),
            &inputs,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        // A strip of four corners is two triangles.
        assert_eq!(triangles, 2);
        assert_eq!(program.vertex.entry_points.len(), 1);
        assert_eq!(program.fragment.entry_points.len(), 1);
    }

    #[test]
    fn writes_run_once_and_keep_their_else() {
        let code =
            "if (c) imageStore(a, p, v); else imageStore(b, p, v);\nint n = atomicAdd(m, 1);\n";
        let tokens = lex(code);
        let mut text: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
        once(&mut text, &tokens);
        assert_eq!(
            text.concat(),
            "if (c) if (!hz_primary) {} else imageStore(a, p, v); else if (!hz_primary) {} else \
             imageStore(b, p, v);\nint n = (hz_primary ? atomicAdd(m, 1) : (m));\n"
        );
    }
}
