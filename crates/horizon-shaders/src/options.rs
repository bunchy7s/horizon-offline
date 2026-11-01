//! Switch options (spec 8.1), as far as program-enable expressions (3.3) need them: which are on
//! by default. A switch is `#define NAME` (on) or `//#define NAME` (off) with nothing after the
//! name but a comment, in any stage or included file of the pack. Packs keep their options in
//! included `.glsl` files (or files of any name a source includes) and test them elsewhere
//! (OQ-13), so the test the spec asks for (`#ifdef NAME` / `#ifndef NAME`) may be in any file
//! of the pack.

use std::collections::{BTreeMap, BTreeSet};

use crate::Pack;

const SOURCES: [&str; 8] = ["vsh", "fsh", "gsh", "csh", "tcs", "tes", "glsl", "inc"];

/// The switch options that are on by default.
pub fn switches_on(pack: &Pack) -> BTreeSet<String> {
    let (mut on, mut off, mut tested) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for text in option_texts(pack) {
        for line in text.lines() {
            let line = line.trim();
            let (commented, rest) = match line.strip_prefix("//") {
                Some(rest) => (true, rest.trim_start()),
                None => (false, line),
            };
            let Some((directive, rest)) = rest
                .strip_prefix('#')
                .map(str::trim_start)
                .and_then(|r| r.split_once(|c: char| c.is_whitespace()))
            else {
                continue;
            };
            let mut words = rest.split_whitespace();
            let Some(name) = words.next() else { continue };
            match directive {
                "ifdef" | "ifndef" if !commented => _ = tested.insert(name.to_owned()),
                "define" if words.next().is_none_or(|w| w.starts_with("//")) => {
                    let set = if commented { &mut off } else { &mut on };
                    set.insert(name.to_owned());
                }
                _ => {}
            }
        }
    }
    // Defined both ways is ambiguous (8.1): it keeps its default only where it is on everywhere.
    on.retain(|name| tested.contains(name) && !off.contains(name));
    on
}

/// The value options and their defaults: `#define NAME VALUE // [A B C]`, a value and the list
/// of the values it may take (8.1).
pub fn value_options(pack: &Pack) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for text in option_texts(pack) {
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix('#') else {
                continue;
            };
            let Some(rest) = rest.trim_start().strip_prefix("define") else {
                continue;
            };
            let Some((code, comment)) = rest.split_once("//") else {
                continue;
            };
            let mut words = code.split_whitespace();
            let (Some(name), Some(value), None) = (words.next(), words.next(), words.next()) else {
                continue;
            };
            if comment.contains('[') && comment.contains(']') {
                out.entry(name.to_owned())
                    .or_insert_with(|| value.to_owned());
            }
        }
    }
    out
}

/// `macros` with the pack's options at their defaults: the switches that are on, and the
/// value options. `shaders.properties` and the program switches test them (3.3, 8.1).
pub fn with_options(pack: &Pack, macros: &crate::Macros) -> crate::Macros {
    let mut out = macros.clone();
    for name in switches_on(pack) {
        out.define(name, "");
    }
    for (name, value) in value_options(pack) {
        out.define(name, value);
    }
    out
}

/// An option the player can change (8.1): a switch (`values` are `false` and `true`), a value
/// option or a `const` with the list of values it may take.
#[derive(Clone, Debug, PartialEq)]
pub struct PackOption {
    pub name: String,
    pub default: String,
    pub values: Vec<String>,
    pub switch: bool,
    /// A switch some file tests; value options always count as tested.
    pub tested: bool,
}

/// A line that declares an option, as [`declaration`] reads it.
struct Declaration<'a> {
    name: &'a str,
    /// The value and its byte offset in the line; `None` for a switch.
    value: Option<(&'a str, usize)>,
    /// Behind `//`: a switch that is off.
    commented: bool,
    /// The values in the comment's `[...]`.
    values: Option<Vec<&'a str>>,
    /// `const bool`: a switch written as a value.
    boolean: bool,
}

/// `#define NAME`, `//#define NAME`, `#define NAME VALUE // [A B]`,
/// `const TYPE NAME = VALUE; // [A B]` or `const bool NAME = true;`.
fn declaration(line: &str) -> Option<Declaration<'_>> {
    let body = line.trim_start();
    let (commented, code) = match body.strip_prefix("//") {
        Some(rest) => (true, rest.trim_start()),
        None => (false, body),
    };
    let offset = |part: &str| part.as_ptr() as usize - line.as_ptr() as usize;
    if let Some(rest) = code.strip_prefix('#') {
        let rest = rest.trim_start().strip_prefix("define")?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let (code, comment) = rest.split_once("//").unwrap_or((rest, ""));
        let mut words = code.split_whitespace();
        let name = words.next()?;
        let value = words.next();
        if words.next().is_some() {
            return None;
        }
        return Some(Declaration {
            name,
            value: value.map(|v| (v, offset(v))),
            commented,
            values: list(comment),
            boolean: false,
        });
    }
    if commented {
        return None;
    }
    // `const float shadowDistance = 128.0; // [64.0 128.0]`, `const bool x = true;`
    let rest = code.strip_prefix("const")?;
    let (decl, comment) = rest.split_once("//").unwrap_or((rest, ""));
    let (left, right) = decl.split_once('=')?;
    let mut words = left.split_whitespace();
    let (Some(ty), Some(name), None) = (words.next(), words.next(), words.next()) else {
        return None;
    };
    let value = right.trim().strip_suffix(';')?.trim_end();
    if value.is_empty() || value.contains(char::is_whitespace) {
        return None;
    }
    let boolean = ty == "bool" && matches!(value, "true" | "false");
    Some(Declaration {
        name,
        value: Some((value, offset(value))),
        commented: false,
        values: if boolean {
            Some(vec!["false", "true"])
        } else {
            Some(list(comment)?)
        },
        boolean,
    })
}

/// The values in a comment's `[...]`.
fn list(comment: &str) -> Option<Vec<&str>> {
    let (_, rest) = comment.split_once('[')?;
    let (inner, _) = rest.split_once(']')?;
    Some(inner.split_whitespace().collect())
}

/// Every option of the pack, by name, at its default.
pub fn options(pack: &Pack) -> BTreeMap<String, PackOption> {
    let mut out: BTreeMap<String, PackOption> = BTreeMap::new();
    let (mut on, mut off, mut tested) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for text in option_texts(pack) {
        // `#ifndef NAME` just before `#define NAME`: an include guard, not an option.
        let mut guard: Option<&str> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            let opened = guard_name(trimmed);
            for directive in ["#ifdef", "#ifndef"] {
                if let Some(name) = trimmed.strip_prefix(directive).map(str::trim) {
                    tested.insert(name.split_whitespace().next().unwrap_or("").to_owned());
                }
            }
            if trimmed.starts_with("#if") || trimmed.starts_with("#elif") {
                tested.extend(defined_names(trimmed));
            }
            let Some(d) = declaration(line) else {
                if opened.is_some() || !(trimmed.is_empty() || trimmed.starts_with("//")) {
                    guard = opened;
                }
                continue;
            };
            if guard.take() == Some(d.name) && d.value.is_none() && !d.commented {
                continue;
            }
            match (d.value, d.values) {
                (None, _) => {
                    let set = if d.commented { &mut off } else { &mut on };
                    set.insert(d.name.to_owned());
                }
                (Some((value, _)), Some(values)) if !d.commented && !values.is_empty() => {
                    // A `const bool` is often the pack's own setting rather than the player's:
                    // it shows where a screen names it, not with `*`.
                    out.entry(d.name.to_owned()).or_insert_with(|| PackOption {
                        name: d.name.to_owned(),
                        default: value.to_owned(),
                        values: values.iter().map(|v| (*v).to_owned()).collect(),
                        switch: d.boolean,
                        tested: !d.boolean,
                    });
                }
                _ => {}
            }
        }
    }
    for name in on.iter().chain(&off) {
        if out.contains_key(name) {
            continue;
        }
        // Defined both ways is ambiguous (8.1): on only where it is on everywhere.
        let default = on.contains(name) && !off.contains(name);
        out.insert(
            name.clone(),
            PackOption {
                name: name.clone(),
                default: default.to_string(),
                values: vec!["false".into(), "true".into()],
                switch: true,
                tested: tested.contains(name),
            },
        );
    }
    out
}

/// The name an `#ifndef NAME` or `#if !defined NAME` line tests.
fn guard_name(line: &str) -> Option<&str> {
    let rest = match line.strip_prefix("#ifndef") {
        Some(rest) => rest,
        None => line
            .strip_prefix("#if")?
            .trim_start()
            .strip_prefix('!')?
            .trim_start()
            .strip_prefix("defined")?,
    };
    let rest = rest.trim_start().trim_start_matches('(').trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// The names `defined NAME` and `defined(NAME)` test in an `#if` line.
fn defined_names(line: &str) -> Vec<String> {
    line.split("defined")
        .skip(1)
        .filter_map(|rest| {
            let rest = rest.trim_start().trim_start_matches('(').trim_start();
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            (end > 0).then(|| rest[..end].to_owned())
        })
        .collect()
}

/// `text` with the options in `chosen` at their chosen values: switches commented out or in,
/// values replaced. Lines keep their place.
pub fn apply(text: &str, chosen: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let end = &line[content.len()..];
        let replaced = declaration(content).and_then(|d| {
            let want = chosen.get(d.name)?;
            match (d.value, d.values) {
                (None, _) => {
                    let on = want == "true";
                    if on != d.commented {
                        return None;
                    }
                    let body = content.trim_start();
                    let indent = &content[..content.len() - body.len()];
                    Some(if on {
                        format!("{indent}{}", body.trim_start_matches('/').trim_start())
                    } else {
                        format!("{indent}//{body}")
                    })
                }
                (Some((value, at)), Some(_)) if !d.commented => Some(format!(
                    "{}{want}{}",
                    &content[..at],
                    &content[at + value.len()..]
                )),
                _ => None,
            }
        });
        out.push_str(replaced.as_deref().unwrap_or(content));
        out.push_str(end);
    }
    out
}

/// The file the player's choices for pack `name` are kept in, beside it in `shaderpacks/`.
pub fn chosen_file(name: &str) -> String {
    format!("{name}.txt")
}

/// `NAME=value` lines; blank lines and `#` comments are skipped.
pub fn parse_chosen(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

pub fn chosen_text(chosen: &BTreeMap<String, String>) -> String {
    chosen.iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}

/// The texts of the pack's sources and of the files they include.
fn option_texts(pack: &Pack) -> Vec<String> {
    let files = pack.all_files();
    let source = |path: &str| SOURCES.contains(&path.rsplit_once('.').map_or("", |(_, e)| e));
    let texts: Vec<(&String, String)> = files
        .iter()
        .filter(|p| source(p))
        .filter_map(|p| Some((p, pack.read_text(p)?)))
        .collect();
    // Files of other names count when a source includes them (`#include "shaders.settings"`).
    let included: BTreeSet<&str> = texts
        .iter()
        .flat_map(|(_, t)| t.lines())
        .filter_map(|l| l.trim().strip_prefix("#include"))
        .map(|r| r.trim().trim_matches(|c| c == '"' || c == '<' || c == '>'))
        .map(|r| r.rsplit('/').next().unwrap_or(r))
        .collect();
    let extra: Vec<(&String, String)> = files
        .iter()
        .filter(|p| !source(p) && included.contains(p.rsplit('/').next().unwrap_or(p)))
        .filter_map(|p| Some((p, pack.read_text(p)?)))
        .collect();
    texts.into_iter().chain(extra).map(|(_, t)| t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;

    #[test]
    fn switches_need_a_test_and_no_value() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/lib/settings.glsl",
                    "  #define AO // ambient occlusion\n//#define BLOOM\n#define LEVEL 2 //[1 2]\n\
                     #define SPLIT\n#define UNTESTED\n",
                ),
                (
                    "shaders/world0/deferred.fsh",
                    "#ifdef AO\n#endif\n#ifndef BLOOM\n#endif\n#ifdef SPLIT\n#endif\n#ifdef LEVEL\n#endif\n",
                ),
                ("shaders/world-1/deferred.fsh", "//#define SPLIT\n"),
                (
                    "shaders/final.fsh",
                    "#include \"shaders.settings\"\n#ifdef GODRAYS\n#endif\n",
                ),
                ("shaders/shaders.settings", "    #define GODRAYS //Toggle\n"),
                ("shaders/notes.settings", "#define NOT_INCLUDED\n#ifdef NOT_INCLUDED\n"),
                ("shaders/shaders.properties", "#define NOT_SOURCE\n#ifdef NOT_SOURCE\n"),
            ]),
        )
        .unwrap();
        assert_eq!(
            switches_on(&pack),
            BTreeSet::from(["AO".to_owned(), "GODRAYS".to_owned()])
        );
    }

    #[test]
    fn options_read_switches_values_and_consts() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/lib/settings.glsl",
                    "#define AO\n//#define BLOOM // glow\n#define LEVEL 2 //[1 2 3]\n\
                     #define FIXED 4\nconst int shadowMapResolution = 2048; // [1024 2048]\n\
                     //#define OFF_VALUE 1 // [1 2]\n",
                ),
                (
                    "shaders/final.fsh",
                    "#ifdef AO\n#endif\n#if defined(BLOOM) && LEVEL > 1\n#endif\n",
                ),
                (
                    "shaders/lib/common.glsl",
                    "#ifndef COMMON_GLSL\n#define COMMON_GLSL\n#endif\n\
                     #if !defined(INCLUDE_SKY)\n\n#define INCLUDE_SKY\n#endif\n",
                ),
            ]),
        )
        .unwrap();
        let options = options(&pack);
        let names: Vec<&str> = options.keys().map(String::as_str).collect();
        assert_eq!(names, ["AO", "BLOOM", "LEVEL", "shadowMapResolution"]);
        assert_eq!(options["AO"].default, "true");
        assert!(options["BLOOM"].switch && options["BLOOM"].tested);
        assert_eq!(options["BLOOM"].default, "false");
        assert_eq!(options["LEVEL"].values, ["1", "2", "3"]);
        assert_eq!(options["shadowMapResolution"].default, "2048");
    }

    #[test]
    fn const_bools_are_switches_screens_name() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[(
                "shaders/lib/settings.glsl",
                "const bool shadowHardwareFiltering = true;\nconst bool generateShadowMipmap = false; // mips\n\
                 const bool notABool = 1;\n",
            )]),
        )
        .unwrap();
        let options = options(&pack);
        let filtering = &options["shadowHardwareFiltering"];
        assert!(filtering.switch && !filtering.tested);
        assert_eq!(filtering.default, "true");
        assert_eq!(options["generateShadowMipmap"].default, "false");
        assert!(!options.contains_key("notABool"));
        let chosen = parse_chosen("shadowHardwareFiltering=false\n");
        assert_eq!(
            apply("const bool shadowHardwareFiltering = true;\n", &chosen),
            "const bool shadowHardwareFiltering = false;\n"
        );
    }

    #[test]
    fn chosen_values_rewrite_their_lines() {
        let chosen = parse_chosen("AO=false\nBLOOM=true\nLEVEL=3\nshadowMapResolution=1024\n");
        let text = "  #define AO // occlusion\r\n//#define BLOOM\n#define LEVEL 2 //[1 2 3]\n\
                    #define LEVEL 9\nconst int shadowMapResolution = 2048; // [1024 2048]\n";
        assert_eq!(
            apply(text, &chosen),
            "  //#define AO // occlusion\r\n#define BLOOM\n#define LEVEL 3 //[1 2 3]\n\
             #define LEVEL 9\nconst int shadowMapResolution = 1024; // [1024 2048]\n"
        );
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                ("shaders/lib/settings.glsl", "#define LEVEL 2 //[1 2 3]\n"),
                ("shaders/shaders.properties", "#define LEVEL 2 //[1 2 3]\n"),
            ]),
        )
        .unwrap()
        .with_chosen(chosen.clone());
        assert_eq!(value_options(&pack)["LEVEL"], "3");
        assert_eq!(
            pack.read_text("shaders.properties").unwrap(),
            "#define LEVEL 2 //[1 2 3]\n"
        );
        assert_eq!(parse_chosen(&chosen_text(&chosen)), chosen);
    }
}
