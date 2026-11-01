//! The GLSL preprocessor a program's stage file goes through before translation (spec 1.4,
//! 1.5, 1.6): comments removed, `#include` expanded, conditionals evaluated, macros expanded.
//! Every output line remembers the pack file and line it came from, so errors found later point
//! at the pack's own code.
//!
//! `#include` is expanded where it stands in live code only: an include inside a false `#if`
//! is not read, so it can't fail (OQ-14). Ten levels deep at most, which also stops a file that
//! includes itself. `#version` and `#extension` are taken out of the code and kept apart: the
//! translation writes its own header.

use std::collections::HashMap;

use crate::Macros;
use crate::Pack;
use crate::condition::{Conditionals, split_directive};
use crate::pack::normalize;

const MAX_DEPTH: usize = 10;
/// How many lines one macro call may span.
const MAX_CALL_LINES: usize = 64;

/// Where a line came from: an index into [`Preprocessed::files`] and a line number from 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Origin {
    pub file: u16,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub origin: Origin,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Preprocessed {
    /// The code, one line per entry of `lines`.
    pub code: String,
    pub lines: Vec<Origin>,
    /// The pack paths of the files read, the stage file first.
    pub files: Vec<String>,
    /// `#version` of the stage file: the number and the profile word, if any.
    pub version: Option<(u32, Option<String>)>,
    /// `#extension` lines of live code, as written after `#extension`.
    pub extensions: Vec<String>,
    /// The text of every block comment in live code (`DRAWBUFFERS:`, buffer formats).
    pub comments: Vec<(Origin, String)>,
    pub diagnostics: Vec<Diagnostic>,
    /// Conditions that could not be evaluated and read as false: noted, the program still runs.
    pub warnings: Vec<Diagnostic>,
}

impl Preprocessed {
    /// `file:line` of an output line (from 0), for messages.
    pub fn locate(&self, line: usize) -> String {
        let o = self.lines.get(line).copied().unwrap_or_default();
        let file = self.files.get(o.file as usize).map_or("?", String::as_str);
        format!("{file}:{}", o.line)
    }
}

/// Runs the preprocessor on the stage file `path` of `pack` with `macros` defined.
pub fn preprocess(pack: &Pack, path: &str, macros: &Macros) -> Preprocessed {
    let mut run = Run {
        pack,
        macros: macros.clone(),
        conditionals: Conditionals::default(),
        out: Preprocessed::default(),
        code: Vec::new(),
        texts: HashMap::new(),
    };
    run.file(path, 0, Origin::default());
    if run.conditionals.open() {
        run.diagnose(Origin::default(), "`#if` without `#endif`");
    }
    run.out.code = run.code.join("\n");
    run.out
}

struct Run<'p> {
    pack: &'p Pack,
    macros: Macros,
    conditionals: Conditionals,
    out: Preprocessed,
    code: Vec<String>,
    /// Files already read: a guarded header is included many times.
    texts: HashMap<String, Option<String>>,
}

impl Run<'_> {
    fn file(&mut self, path: &str, depth: usize, from: Origin) {
        let cached = self
            .texts
            .entry(path.to_owned())
            .or_insert_with(|| self.pack.read_text(path));
        let Some(text) = cached.clone() else {
            return self.diagnose(from, &format!("can't find `{path}`"));
        };
        let file = match self.out.files.iter().position(|f| f == path) {
            Some(i) => i,
            None => {
                self.out.files.push(path.to_owned());
                self.out.files.len() - 1
            }
        } as u16;
        let (text, comments) = strip_comments(&text);
        let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        let mut comments = comments.into_iter().peekable();
        let mut n = 0;
        while n < lines.len() {
            let first = n;
            let mut line = lines[n].to_owned();
            while line.ends_with('\\') && n + 1 < lines.len() {
                line.pop();
                n += 1;
                line.push_str(lines[n]);
            }
            n += 1;
            // A macro call's arguments may span lines: they join the line the call starts on.
            if !line.trim_start().starts_with('#') && self.conditionals.active() {
                while n < lines.len()
                    && n - first < MAX_CALL_LINES
                    && !lines[n].trim_start().starts_with('#')
                    && self.macros.unfinished_call(&line)
                {
                    line.push(' ');
                    line.push_str(lines[n]);
                    n += 1;
                }
            }
            let origin = Origin {
                file,
                line: first as u32 + 1,
            };
            self.line(&line, origin, path, depth);
            for k in first + 1..n {
                self.emit(
                    String::new(),
                    Origin {
                        file,
                        line: k as u32 + 1,
                    },
                );
            }
            while let Some((_, comment)) = comments.next_if(|(at, _)| *at < n) {
                if self.conditionals.active() {
                    self.out.comments.push((origin, comment));
                }
            }
        }
    }

    fn line(&mut self, line: &str, origin: Origin, path: &str, depth: usize) {
        let Some(after) = line.trim_start().strip_prefix('#') else {
            let code = if self.conditionals.active() {
                self.macros.define("__LINE__", origin.line.to_string());
                self.macros.expand(line)
            } else {
                String::new()
            };
            return self.emit(code, origin);
        };
        self.emit(String::new(), origin);
        let Some((word, rest)) = split_directive(after) else {
            return self.diagnose(origin, &format!("unknown directive `#{}`", after.trim()));
        };
        match self.conditionals.apply(word, rest, &self.macros) {
            // As Iris loads such a pack: the branch is skipped, the program kept.
            Err(e) if e.ends_with("read as false") => {
                return self.out.warnings.push(Diagnostic { origin, message: e });
            }
            Err(e) => return self.diagnose(origin, &e),
            Ok(true) => return,
            Ok(false) if !self.conditionals.active() => return,
            Ok(false) => {}
        }
        match word {
            "define" => self.define(rest, origin),
            "undef" => self.macros.undef(rest),
            "include" => {
                let target = rest.trim_matches(|c| c == '"' || c == '<' || c == '>');
                match resolve(path, target) {
                    _ if depth + 1 >= MAX_DEPTH => self.diagnose(
                        origin,
                        &format!("`{target}`: includes nest more than {MAX_DEPTH} deep"),
                    ),
                    Some(file) => {
                        // The include line's own (empty) output line stays; the file follows.
                        self.file(&file, depth + 1, origin);
                    }
                    None => self.diagnose(origin, &format!("`{target}` is outside the pack")),
                }
            }
            "version" if depth == 0 && self.out.version.is_none() => {
                let mut words = rest.split_whitespace();
                let number = words.next().and_then(|v| v.parse().ok()).unwrap_or(110);
                self.out.version = Some((number, words.next().map(str::to_owned)));
                self.macros.define("__VERSION__", number.to_string());
            }
            "extension" => self.out.extensions.push(rest.to_owned()),
            "error" => self.diagnose(origin, &format!("#error {rest}")),
            "version" | "pragma" | "line" | "" => {}
            _ => self.diagnose(origin, &format!("unknown directive `#{word}`")),
        }
    }

    /// `#define NAME body` or `#define NAME(a, b) body` (no space before the `(`).
    fn define(&mut self, rest: &str, origin: Origin) {
        let len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let (name, tail) = rest.split_at(len);
        if name.is_empty() {
            return self.diagnose(origin, "`#define` without a name");
        }
        if let Some(params) = tail.strip_prefix('(') {
            let Some((params, body)) = params.split_once(')') else {
                return self.diagnose(origin, &format!("`#define {name}(` without `)`"));
            };
            let params = params
                .split(',')
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect();
            self.macros.define_function(name, params, body.trim());
        } else {
            self.macros.define(name, tail.trim());
        }
    }

    fn emit(&mut self, code: String, origin: Origin) {
        self.code.push(code);
        self.out.lines.push(origin);
    }

    fn diagnose(&mut self, origin: Origin, message: &str) {
        self.out.diagnostics.push(Diagnostic {
            origin,
            message: message.to_owned(),
        });
    }
}

/// An `#include` path: from the `shaders` folder with a leading `/`, else from the including
/// file's folder (spec 1.4). `None` when it climbs out of the pack.
fn resolve(from_file: &str, target: &str) -> Option<String> {
    match target.strip_prefix('/') {
        Some(absolute) => normalize(absolute),
        None => {
            let dir = from_file.rsplit_once('/').map_or("", |(dir, _)| dir);
            normalize(&format!("{dir}/{target}"))
        }
    }
}

/// `text` with comments replaced by spaces (new lines kept, so line numbers hold), and the body of
/// each block comment with the index of the line it starts on.
fn strip_comments(text: &str) -> (String, Vec<(usize, String)>) {
    let mut out = String::with_capacity(text.len());
    let mut comments = Vec::new();
    let (mut rest, mut line) = (text, 0);
    while let Some(at) = rest.find('/') {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        line += before.matches('\n').count();
        if let Some(body) = from.strip_prefix("//") {
            let end = body.find('\n').unwrap_or(body.len());
            rest = &body[end..];
        } else if let Some(body) = from.strip_prefix("/*") {
            let end = body.find("*/").unwrap_or(body.len());
            let comment = &body[..end];
            comments.push((line, comment.to_owned()));
            let newlines = comment.matches('\n').count();
            line += newlines;
            out.push(' ');
            out.extend(std::iter::repeat_n('\n', newlines));
            rest = body.get(end + 2..).unwrap_or("");
        } else {
            out.push('/');
            rest = &from[1..];
        }
    }
    out.push_str(rest);
    (out, comments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;

    fn run(files: &[(&str, &str)]) -> Preprocessed {
        let files: Vec<(String, &str)> = files
            .iter()
            .map(|(p, t)| (format!("shaders/{p}"), *t))
            .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        preprocess(&pack, "program/final.fsh", &Macros::standard())
    }

    fn code_lines(p: &Preprocessed) -> Vec<(&str, String)> {
        p.code
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| (l.trim(), p.locate(i)))
            .collect()
    }

    #[test]
    fn includes_resolve_and_lines_map_back() {
        let p = run(&[
            (
                "program/final.fsh",
                "#version 330 compatibility\n#include \"/lib/a.glsl\"\nvoid main() {}\n",
            ),
            (
                "lib/a.glsl",
                "#ifndef A\n#define A\n#include \"b.glsl\"\nfloat a;\n#endif\n",
            ),
            ("lib/b.glsl", "// b\nfloat b;\n#include \"a.glsl\"\n"),
        ]);
        assert_eq!(p.version, Some((330, Some("compatibility".into()))));
        assert_eq!(
            code_lines(&p),
            [
                ("float b;", "lib/b.glsl:2".into()),
                ("float a;", "lib/a.glsl:4".into()),
                ("void main() {}", "program/final.fsh:3".into()),
            ]
        );
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
    }

    #[test]
    fn missing_and_runaway_includes_are_diagnosed_where_they_stand() {
        let p = run(&[(
            "program/final.fsh",
            "#ifdef NOPE\n#include \"gone.glsl\"\n#endif\n#include \"gone.glsl\"\n#include \"final.fsh\"\n",
        )]);
        let messages: Vec<String> = p
            .diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.origin.line, d.message))
            .collect();
        assert_eq!(messages[0], "4: can't find `program/gone.glsl`");
        assert!(
            messages
                .last()
                .unwrap()
                .ends_with("includes nest more than 10 deep")
        );
        assert!(
            run(&[("program/final.fsh", "#include \"../../x\"")]).diagnostics[0]
                .message
                .contains("outside")
        );
    }

    #[test]
    fn macros_expand_like_the_glsl_preprocessor() {
        let p = run(&[(
            "program/final.fsh",
            "#define SQ(x) ((x) * (x))\n#define CAT(a, b) a ## b\n#define N 4\n#define \\\n  LONG 1\n\
             float v = SQ(N + 1) + CAT(N, 2) + LONG;\nint l = __LINE__;\n#if MC_VERSION > 12100 && defined MC_RENDER_STAGE_SKY\nint ok;\n#else\nint bad;\n#endif\n",
        )]);
        let lines: Vec<&str> = code_lines(&p).into_iter().map(|(l, _)| l).collect();
        assert_eq!(
            lines,
            [
                "float v = ((4 + 1) * (4 + 1)) + N2 + 1;",
                "int l = 7;",
                "int ok;"
            ]
        );
        assert_eq!(p.lines.len(), p.code.split('\n').count());
    }

    #[test]
    fn macro_arguments_span_lines() {
        let p = run(&[(
            "program/final.fsh",
            "#define CLAMP01(x) clamp(x, 0.0, 1.0)\nfloat v = CLAMP01(\n    a * (b\n    + c)\n);\nfloat w;\n",
        )]);
        assert_eq!(
            code_lines(&p),
            [
                (
                    "float v = clamp(a * (b     + c), 0.0, 1.0);",
                    "program/final.fsh:2".into()
                ),
                ("float w;", "program/final.fsh:6".into())
            ]
        );
        assert_eq!(p.lines.len(), p.code.split('\n').count());
    }

    #[test]
    fn comments_extensions_and_errors() {
        let p = run(&[(
            "program/final.fsh",
            "#version 120\n#extension GL_ARB_shader_texture_lod : enable\n/* DRAWBUFFERS:04 */\n#if 0\n/* RENDERTARGETS: 1 */\n#error dead\n#endif\n\
             /* multi\nline */ float x; // gone\n#error live\n#bogus\n",
        )]);
        assert_eq!(p.extensions, ["GL_ARB_shader_texture_lod : enable"]);
        let comments: Vec<&str> = p.comments.iter().map(|(_, c)| c.trim()).collect();
        assert_eq!(comments, ["DRAWBUFFERS:04", "multi\nline"]);
        assert_eq!(code_lines(&p), [("float x;", "program/final.fsh:9".into())]);
        let messages: Vec<&str> = p.diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages, ["#error live", "unknown directive `#bogus`"]);
    }

    /// A condition that can't be evaluated is a warning and reads as false; the code goes on.
    #[test]
    fn bad_conditions_warn_and_read_false() {
        let p = run(&[(
            "program/final.fsh",
            "#if (in(biome, BIOME_SOUL_SAND_VALLEY)\nfloat a;\n#else\nfloat b;\n#endif\nfloat c;\n",
        )]);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        assert_eq!(p.warnings.len(), 1);
        let code: Vec<&str> = code_lines(&p).iter().map(|(c, _)| *c).collect();
        assert_eq!(code, ["float b;", "float c;"]);
    }
}
