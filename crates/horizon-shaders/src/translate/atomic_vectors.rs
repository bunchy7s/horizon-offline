//! Atomic functions on a component of a shared integer vector (`atomicAdd(sums[i], n)`,
//! `atomicMax(lights[k].w, w)`): GLSL allows them, naga has no atomic vectors. Such a vector
//! (or array of vectors) becomes an array of its components, `shared int sums_hz[4]`; a
//! component is an element, a whole vector is read through `hz_get_sums()` and written through
//! `hz_set_sums(v)`. The source keeps its lines.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::scan::{is_ident, significant};
use crate::Preprocessed;
use crate::macros::lex;

const ATOMICS: [&str; 8] = [
    "atomicAdd",
    "atomicMin",
    "atomicMax",
    "atomicAnd",
    "atomicOr",
    "atomicXor",
    "atomicExchange",
    "atomicCompSwap",
];

/// A shared vector rewritten: its scalar and vector types, size, and array length if any.
struct Split {
    scalar: &'static str,
    vector: String,
    size: usize,
    array: Option<String>,
}

/// `pp` with its shared vectors that atomic functions use by component rewritten.
pub(super) fn rewrite(pp: &Preprocessed) -> Option<Preprocessed> {
    if !ATOMICS.iter().any(|a| pp.code.contains(a)) {
        return None;
    }
    let tokens = lex(&pp.code);
    let next = |i: usize| (i + 1..tokens.len()).find(|&j| significant(tokens[j]));
    let prev = |i: usize| (0..i).rev().find(|&j| significant(tokens[j]));
    // `[N]` at `open`: its size's text and the token after `]`.
    let bracket = |open: usize| -> Option<(String, Option<usize>)> {
        let close = (open..tokens.len()).find(|&j| tokens[j] == "]")?;
        let size: String = tokens[open + 1..close].concat();
        Some((size.trim().to_owned(), next(close)))
    };
    // The variables an atomic function takes a component of.
    let mut used = Vec::new();
    for (i, &t) in tokens.iter().enumerate() {
        if ATOMICS.contains(&t)
            && let Some(open) = next(i).filter(|&j| tokens[j] == "(")
            && let Some(name) = next(open).filter(|&j| is_ident(tokens[j]))
        {
            used.push(tokens[name]);
        }
    }
    // Their declarations: `shared ivec4 name;`, `shared ivec4[N] name;`, `shared ivec4 name[N];`.
    let mut splits: BTreeMap<&str, Split> = BTreeMap::new();
    let mut decl_ends = Vec::new();
    for (i, &t) in tokens.iter().enumerate() {
        if t != "shared" {
            continue;
        }
        let Some(ty) = next(i) else { continue };
        let (scalar, size) = match tokens[ty] {
            v if v.starts_with("ivec") => ("int", &v[4..]),
            v if v.starts_with("uvec") => ("uint", &v[4..]),
            _ => continue,
        };
        let Ok(size) = size.parse::<usize>() else {
            continue;
        };
        let mut j = next(ty);
        let mut array = None;
        if let Some((size, after)) = j.filter(|&j| tokens[j] == "[").and_then(bracket) {
            array = Some(size);
            j = after;
        }
        let Some(name_at) = j.filter(|&j| is_ident(tokens[j])) else {
            continue;
        };
        let name = tokens[name_at];
        let mut end = next(name_at);
        if let Some((size, after)) = end.filter(|&e| tokens[e] == "[").and_then(bracket) {
            array = Some(size);
            end = after;
        }
        let Some(end) = end.filter(|&e| tokens[e] == ";") else {
            continue;
        };
        if !used.contains(&name) {
            continue;
        }
        splits.insert(
            name,
            Split {
                scalar,
                vector: tokens[ty].to_owned(),
                size,
                array,
            },
        );
        decl_ends.push((i, end, name));
    }
    if splits.is_empty() {
        return None;
    }
    let mut out: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
    for &(start, end, name) in &decl_ends {
        let s = &splits[name];
        let (v, sc, n) = (&s.vector, s.scalar, s.size);
        let comps = |base: &str| {
            (0..n)
                .map(|c| format!("{base}[{c}]"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let sets = |base: &str| {
            (0..n)
                .map(|c| format!("{base}[{c}] = v[{c}];"))
                .collect::<String>()
        };
        let mut decl = String::new();
        match &s.array {
            None => write!(
                decl,
                "shared {sc} {name}_hz[{n}]; {v} hz_get_{name}() {{ return {v}({}); }} \
                 void hz_set_{name}({v} v) {{ {} }}",
                comps(&format!("{name}_hz")),
                sets(&format!("{name}_hz"))
            ),
            Some(len) => write!(
                decl,
                "shared {sc} {name}_hz[{len}][{n}]; {v} hz_get_{name}(int k) {{ return {v}({}); }} \
                 void hz_set_{name}(int k, {v} v) {{ {} }}",
                comps(&format!("{name}_hz[k]")),
                sets(&format!("{name}_hz[k]"))
            ),
        }
        .unwrap();
        for t in &mut out[start..=end] {
            *t = t.chars().filter(|&c| c == '\n').collect();
        }
        out[start] = decl + &out[start];
    }
    let component = |s: &str| match s {
        "x" | "r" | "s" => Some(0),
        "y" | "g" | "t" => Some(1),
        "z" | "b" | "p" => Some(2),
        "w" | "a" | "q" => Some(3),
        _ => None,
    };
    // Uses: from the name, past `[k]` for an array, then a component, a swizzle, an assignment
    // or the whole vector.
    let mut i = 0;
    while i < tokens.len() {
        let name = tokens[i];
        let Some(s) = splits.get(name) else {
            i += 1;
            continue;
        };
        if decl_ends.iter().any(|&(a, b, _)| (a..=b).contains(&i))
            || prev(i).is_some_and(|p| tokens[p] == ".")
        {
            i += 1;
            continue;
        }
        let mut base = format!("{name}_hz");
        let mut getter_args = String::new();
        let mut last = i;
        if s.array.is_some() {
            // `name[k]`: the index's tokens, written back as they are.
            let Some(open) = next(i).filter(|&j| tokens[j] == "[") else {
                i += 1;
                continue;
            };
            let mut depth = 0;
            let mut close = open;
            for (j, &t) in tokens.iter().enumerate().skip(open) {
                match t {
                    "[" => depth += 1,
                    "]" => {
                        depth -= 1;
                        if depth == 0 {
                            close = j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let index: String = tokens[open + 1..close].concat();
            base = format!("{base}[{index}]");
            // GLSL won't turn a `uint` index into the `int` the getter and setter take.
            getter_args = format!("int({index})");
            last = close;
        }
        let after = next(last);
        let follows = |t: &str| after.is_some_and(|j| tokens[j] == t);
        let assigned = |at: Option<usize>| {
            at.is_some_and(|j| tokens[j] == "=" && next(j).is_none_or(|k| tokens[k] != "="))
        };
        // `name op= value;`: the operator's tokens and the `=`.
        let compound = after.and_then(|op| {
            let first = tokens[op];
            let second = next(op)?;
            match (first, tokens[second]) {
                ("+" | "-" | "*" | "/" | "%" | "&" | "|" | "^", "=") => {
                    Some((first.to_owned(), second))
                }
                ("<" | ">", t) if t == first => {
                    let eq = next(second).filter(|&k| tokens[k] == "=")?;
                    Some((format!("{first}{first}"), eq))
                }
                _ => None,
            }
        });
        let replacement;
        let mut end = last;
        if let Some((op, eq)) = compound {
            // `name op= value;` → `hz_set_name(hz_get_name() op (value));`
            let semi = (eq..tokens.len()).find(|&j| tokens[j] == ";").unwrap_or(eq);
            let args = if getter_args.is_empty() {
                String::new()
            } else {
                format!("{getter_args}, ")
            };
            out[i] = format!("hz_set_{name}({args}hz_get_{name}({getter_args}) {op} (");
            for t in &mut out[i + 1..=eq] {
                *t = t.chars().filter(|&c| c == '\n').collect();
            }
            out[semi] = "));".into();
            i = eq + 1;
            continue;
        } else if follows(".") || (s.array.is_none() && follows("[")) {
            let at = after.and_then(next).unwrap();
            if follows("[") {
                // `name[c]`: an element already.
                replacement = base.clone();
            } else if let Some(c) = component(tokens[at]) {
                replacement = format!("{base}[{c}]");
                end = at;
            } else {
                // A swizzle of several components: read only.
                let get = format!("hz_get_{name}({getter_args})");
                replacement = format!("{get}.{}", tokens[at]);
                end = at;
            }
        } else if assigned(after) {
            // `name = value;` → `hz_set_name(value);`
            let eq = after.unwrap();
            let semi = (eq..tokens.len()).find(|&j| tokens[j] == ";").unwrap_or(eq);
            let args = if getter_args.is_empty() {
                String::new()
            } else {
                format!("{getter_args}, ")
            };
            out[i] = format!("hz_set_{name}({args}");
            for t in &mut out[i + 1..=eq] {
                *t = t.chars().filter(|&c| c == '\n').collect();
            }
            out[semi] = ");".into();
            i = eq + 1;
            continue;
        } else {
            replacement = format!("hz_get_{name}({getter_args})");
        }
        out[i] = replacement;
        for t in &mut out[i + 1..=end] {
            *t = t.chars().filter(|&c| c == '\n').collect();
        }
        i = end + 1;
    }
    Some(Preprocessed {
        code: out.concat(),
        lines: pp.lines.clone(),
        files: pp.files.clone(),
        version: pp.version.clone(),
        extensions: pp.extensions.clone(),
        comments: pp.comments.clone(),
        diagnostics: pp.diagnostics.clone(),
        warnings: pp.warnings.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewritten(code: &str) -> String {
        let pp = Preprocessed {
            code: code.to_owned(),
            lines: Vec::new(),
            files: Vec::new(),
            version: None,
            extensions: Vec::new(),
            comments: Vec::new(),
            diagnostics: Vec::new(),
            warnings: Vec::new(),
        };
        let out = rewrite(&pp).expect("rewritten").code;
        assert_eq!(out.lines().count(), code.lines().count(), "{out}");
        out
    }

    #[test]
    fn components_whole_vectors_and_compound_assignments() {
        let out = rewritten(
            "shared ivec4 sums;\nvoid f(ivec4 v) {\n atomicAdd(sums.y, 1);\n ivec4 a = sums;\n \
             sums = v;\n sums += v;\n sums <<= 1;\n int z = sums.z;\n ivec2 xy = sums.xy;\n}\n",
        );
        assert!(out.contains("shared int sums_hz[4];"));
        assert!(out.contains("atomicAdd(sums_hz[1], 1);"));
        assert!(out.contains("ivec4 a = hz_get_sums();"));
        assert!(out.contains("hz_set_sums( v);"));
        assert!(out.contains("hz_set_sums(hz_get_sums() + ( v));"));
        assert!(out.contains("hz_set_sums(hz_get_sums() << ( 1));"));
        assert!(out.contains("int z = sums_hz[2];"));
        assert!(out.contains("ivec2 xy = hz_get_sums().xy;"));
    }

    #[test]
    fn arrays_of_vectors_with_any_size() {
        let out = rewritten(
            "#define N 4\nshared uvec2 lights[N * 2];\nvoid f(int k) {\n atomicMax(lights[k + 1].x, 3u);\n \
             lights[k] += uvec2(1u);\n uvec2 l = lights[k];\n}\n",
        );
        assert!(out.contains("shared uint lights_hz[N * 2][2];"), "{out}");
        assert!(out.contains("atomicMax(lights_hz[k + 1][0], 3u);"));
        assert!(out.contains("hz_set_lights(int(k), hz_get_lights(int(k)) + ( uvec2(1u)));"));
        assert!(out.contains("uvec2 l = hz_get_lights(int(k));"));
    }

    #[test]
    fn untouched_without_atomics_on_a_component() {
        let pp = |code: &str| Preprocessed {
            code: code.to_owned(),
            lines: Vec::new(),
            files: Vec::new(),
            version: None,
            extensions: Vec::new(),
            comments: Vec::new(),
            diagnostics: Vec::new(),
            warnings: Vec::new(),
        };
        assert!(rewrite(&pp("shared ivec4 v;\nvoid f() { v.x = 1; }\n")).is_none());
        assert!(rewrite(&pp("shared int n;\nvoid f() { atomicAdd(n, 1); }\n")).is_none());
    }
}
