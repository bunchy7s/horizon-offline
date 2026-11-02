//! Specialize opaque image parameters to their bound resources. GLSL image parameters do not
//! carry a storage format, while Naga's image types do. This preserves the actual global image
//! type/access and writes no pack files. Unsupported dynamic handles remain compiler errors.
use crate::macros::lex;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
struct Function {
    name: String,
    result: String,
    params: Vec<String>,
    images: Vec<(usize, String)>,
    body: String,
}
struct Site {
    start: usize,
    end: usize,
    function: usize,
    prototype: bool,
}
struct Instance {
    name: String,
    signature: String,
    body: String,
}
struct Rewriter {
    functions: Vec<Function>,
    resources: BTreeSet<String>,
    instances: BTreeMap<(usize, Vec<String>), Instance>,
}
fn word(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn image(s: &str) -> bool {
    s.strip_prefix("uimage")
        .or_else(|| s.strip_prefix("iimage"))
        .or_else(|| s.strip_prefix("image"))
        .is_some_and(|s| {
            matches!(
                s,
                "1D" | "2D"
                    | "3D"
                    | "Cube"
                    | "Buffer"
                    | "1DArray"
                    | "2DArray"
                    | "CubeArray"
                    | "2DMS"
                    | "2DMSArray"
            )
        })
}
fn next(tokens: &[&str], i: usize) -> Option<usize> {
    (i + 1..tokens.len()).find(|&j| !tokens[j].trim().is_empty())
}
fn close(tokens: &[&str], open: usize, left: &str, right: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        if *t == left {
            depth += 1;
        } else if *t == right {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}
fn args(tokens: &[&str], open: usize, end: usize) -> Vec<String> {
    let (mut depth, mut first) = (0, open + 1);
    let mut result = Vec::new();
    for i in open + 1..end {
        match tokens[i] {
            "(" | "[" => depth += 1,
            ")" | "]" => depth -= 1,
            "," if depth == 0 => {
                result.push(tokens[first..i].concat());
                first = i + 1;
            }
            _ => {}
        }
    }
    let last = tokens[first..end].concat();
    if !last.trim().is_empty() {
        result.push(last);
    }
    result
}
impl Rewriter {
    fn calls(&mut self, text: &str) -> Result<String, String> {
        let tokens = lex(text);
        let mut out = String::new();
        let mut i = 0;
        while i < tokens.len() {
            let Some(open) = next(&tokens, i).filter(|&o| tokens[o] == "(") else {
                out.push_str(tokens[i]);
                i += 1;
                continue;
            };
            let Some(end) = close(&tokens, open, "(", ")") else {
                out.push_str(tokens[i]);
                i += 1;
                continue;
            };
            let params = args(&tokens, open, end);
            let candidates: Vec<_> = self
                .functions
                .iter()
                .enumerate()
                .filter(|(_, f)| f.name == tokens[i] && f.params.len() == params.len())
                .map(|(i, _)| i)
                .collect();
            if candidates.is_empty() {
                out.push_str(tokens[i]);
                i += 1;
                continue;
            }
            if candidates.len() != 1 {
                return Err(format!(
                    "image-parameter overload of {} is ambiguous",
                    tokens[i]
                ));
            }
            let id = candidates[0];
            let function = self.functions[id].clone();
            let mut bindings = Vec::new();
            for (k, _) in &function.images {
                let value = lex(&params[*k])
                    .into_iter()
                    .filter(|t| !t.trim().is_empty())
                    .collect::<String>();
                if !word(&value) || !self.resources.contains(&value) {
                    return Err(format!(
                        "image parameter of {} must resolve to a bound image",
                        function.name
                    ));
                }
                bindings.push(value);
            }
            let key = (id, bindings.clone());
            if !self.instances.contains_key(&key) {
                if self.instances.len() >= 256 {
                    return Err("image parameter specialization exceeds 256 variants".into());
                }
                let name = format!("hz_image_call_{}", self.instances.len());
                let params = function
                    .params
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| !function.images.iter().any(|(i, _)| i == k))
                    .map(|(_, p)| p.clone())
                    .collect::<Vec<_>>()
                    .join(",");
                let signature = format!("{} {name}({params})", function.result);
                self.instances.insert(
                    key.clone(),
                    Instance {
                        name,
                        signature,
                        body: String::new(),
                    },
                );
                let substitutions: BTreeMap<_, _> = function
                    .images
                    .iter()
                    .zip(&bindings)
                    .map(|((_, p), b)| (p.as_str(), b.as_str()))
                    .collect();
                let body = lex(&function.body)
                    .iter()
                    .map(|t| substitutions.get(t).copied().unwrap_or(t))
                    .collect::<String>();
                let body = self.calls(&body)?;
                self.instances.get_mut(&key).unwrap().body = body;
            }
            let name = self.instances[&key].name.clone();
            let mut kept = Vec::new();
            for (k, param) in params.iter().enumerate() {
                if !function.images.iter().any(|(i, _)| *i == k) {
                    kept.push(self.calls(param)?);
                }
            }
            let call = format!("{name}({})", kept.join(","));
            out.push_str(&call.replace('\n', " "));
            out.extend(
                tokens[i..=end]
                    .iter()
                    .flat_map(|t| t.chars().filter(|c| *c == '\n')),
            );
            i = end + 1;
        }
        Ok(out)
    }
}

pub(super) fn rewrite(text: &str) -> Result<String, String> {
    let tokens = lex(text);
    let words: Vec<_> = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    let resources = words
        .windows(2)
        .filter(|p| image(tokens[p[0]]) && word(tokens[p[1]]))
        .map(|p| tokens[p[1]].to_owned())
        .collect();
    let mut functions: Vec<Function> = Vec::new();
    let mut sites = Vec::new();
    let mut depth = 0;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "{" => depth += 1,
            "}" => depth -= 1,
            _ => {}
        }
        if depth != 0 || !word(tokens[i]) {
            i += 1;
            continue;
        }
        let Some(open) = next(&tokens, i).filter(|&o| tokens[o] == "(") else {
            i += 1;
            continue;
        };
        let Some(end) = close(&tokens, open, "(", ")") else {
            i += 1;
            continue;
        };
        let Some(after) = next(&tokens, end).filter(|&j| matches!(tokens[j], "{" | ";")) else {
            i += 1;
            continue;
        };
        let params = args(&tokens, open, end);
        let mut images = Vec::new();
        for (k, p) in params.iter().enumerate() {
            let w: Vec<_> = lex(p)
                .into_iter()
                .filter(|t| !t.trim().is_empty())
                .collect();
            if let Some(at) = w.iter().position(|t| image(t))
                && let Some(name) = w.get(at + 1)
            {
                images.push((k, (*name).to_owned()));
            }
        }
        if images.is_empty() {
            i += 1;
            continue;
        }
        let Some(start) = (0..i).rev().find(|&j| !tokens[j].trim().is_empty()) else {
            i += 1;
            continue;
        };
        if !word(tokens[start]) {
            i += 1;
            continue;
        }
        let prototype = tokens[after] == ";";
        let stop = if prototype {
            after
        } else {
            close(&tokens, after, "{", "}").ok_or("unterminated image function")?
        };
        let function = Function {
            name: tokens[i].into(),
            result: tokens[start].into(),
            params,
            images,
            body: if prototype {
                String::new()
            } else {
                tokens[after + 1..stop].concat()
            },
        };
        let id = functions
            .iter()
            .position(|f| f.name == function.name && f.params == function.params)
            .unwrap_or(functions.len());
        if id == functions.len() {
            functions.push(function);
        } else if !prototype {
            functions[id] = function;
        }
        sites.push(Site {
            start,
            end: stop + 1,
            function: id,
            prototype,
        });
        i = stop + 1;
    }
    if sites.is_empty() {
        return Ok(text.into());
    }
    let mut r = Rewriter {
        functions,
        resources,
        instances: BTreeMap::new(),
    };
    let mut chunks = Vec::new();
    let mut first = 0;
    for site in &sites {
        chunks.push(r.calls(&tokens[first..site.start].concat())?);
        first = site.end;
    }
    chunks.push(r.calls(&tokens[first..].concat())?);
    let mut out = String::new();
    for (k, site) in sites.iter().enumerate() {
        out.push_str(&chunks[k]);
        for ((id, _), instance) in &r.instances {
            if *id == site.function {
                let code = if site.prototype {
                    format!("{};", instance.signature)
                } else {
                    format!("{}{{{}}}", instance.signature, instance.body)
                };
                out.push_str(&code.replace('\n', " "));
            }
        }
        out.extend(
            tokens[site.start..site.end]
                .iter()
                .flat_map(|t| t.chars().filter(|c| *c == '\n')),
        );
    }
    out.push_str(chunks.last().unwrap());
    Ok(out)
}
