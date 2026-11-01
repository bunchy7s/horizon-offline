//! Object-like macros: the ones packs test in `#if` lines and use inside property values
//! (spec 1.5, 1.5.1). The GLSL preprocessor's function-like macros are not needed here.

use std::collections::BTreeMap;

/// The Iris feature flags (`iris.features.*`) Horizon has.
const FEATURES: [&str; 4] = [
    "COMPUTE_SHADERS",
    "CUSTOM_IMAGES",
    "ENTITY_TRANSLUCENT",
    "SSBO",
];

/// `MC_VERSION` of the version Horizon plays: 26.3 → 260300.
pub const MC_VERSION: u32 = mc_version(horizon_core::version::VERSION_ID);

/// The documented rule, `major * 10000 + minor * 100 + patch` (spec 1.5.1, OQ-1): `1.21` → 12100,
/// `1.7.10` → 10710. Parts that are not digits count as 0.
pub const fn mc_version(id: &str) -> u32 {
    let bytes = id.as_bytes();
    let (mut parts, mut part, mut i) = ([0u32; 3], 0, 0);
    while i < bytes.len() && part < 3 {
        match bytes[i] {
            b'.' => part += 1,
            b @ b'0'..=b'9' => parts[part] = parts[part] * 10 + (b - b'0') as u32,
            _ => break,
        }
        i += 1;
    }
    parts[0] * 10000 + parts[1] * 100 + parts[2]
}

#[derive(Clone, Debug, PartialEq)]
struct Macro {
    /// `None` for an object-like macro; the parameter names of a function-like one.
    params: Option<Vec<String>>,
    body: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Macros(BTreeMap<String, Macro>);

/// `MC_RENDER_STAGE_*` in the order the stages draw (spec 5.4); each is its index (OQ-7).
pub const RENDER_STAGES: [&str; 24] = [
    "NONE",
    "SKY",
    "SUNSET",
    "CUSTOM_SKY",
    "SUN",
    "MOON",
    "STARS",
    "VOID",
    "TERRAIN_SOLID",
    "TERRAIN_CUTOUT_MIPPED",
    "TERRAIN_CUTOUT",
    "ENTITIES",
    "BLOCK_ENTITIES",
    "DESTROY",
    "OUTLINE",
    "DEBUG",
    "HAND_SOLID",
    "TERRAIN_TRANSLUCENT",
    "TRIPWIRE",
    "PARTICLES",
    "CLOUDS",
    "RAIN_SNOW",
    "WORLD_BORDER",
    "HAND_TRANSLUCENT",
];

/// The value of `MC_RENDER_STAGE_<name>` (OQ-7: its index in `RENDER_STAGES`).
pub fn render_stage(name: &str) -> u32 {
    RENDER_STAGES
        .iter()
        .position(|s| *s == name)
        .unwrap_or_else(|| panic!("no render stage {name}")) as u32
}

impl Macros {
    /// The standard macros that need no renderer (spec 1.5.1): `MC_VERSION`, this system's
    /// `MC_OS_*`, the GL and GLSL versions Horizon's translation accepts, the render stages and
    /// the host settings at their defaults. The GPU vendor and family arrive with the renderer.
    pub fn standard() -> Self {
        let mut m = Self::default();
        m.define("MC_VERSION", MC_VERSION.to_string());
        let os = match std::env::consts::OS {
            "windows" => "MC_OS_WINDOWS",
            "macos" => "MC_OS_MAC",
            "linux" => "MC_OS_LINUX",
            _ => "MC_OS_OTHER",
        };
        m.define(os, "");
        m.define("MC_GL_VERSION", "460");
        m.define("MC_GLSL_VERSION", "460");
        for (i, stage) in RENDER_STAGES.iter().enumerate() {
            m.define(format!("MC_RENDER_STAGE_{stage}"), i.to_string());
        }
        m.define("MC_RENDER_QUALITY", "1.0");
        m.define("MC_SHADOW_QUALITY", "1.0");
        m.define("MC_HAND_DEPTH", "0.125");
        m.define("MAX_COLOR_BUFFERS", "16");
        // Packs take Iris's extensions (compute programs, images, custom textures) where this
        // is defined.
        m.define("IS_IRIS", "");
        m
    }

    /// `iris.features.optional`'s features Horizon has, as `IRIS_FEATURE_<name>`; a note for
    /// each feature `iris.features.required` names that it lacks.
    pub fn define_features(&mut self, properties: &crate::Properties) -> Vec<String> {
        let words = |key: &str| {
            properties
                .get(key)
                .map(|v| v.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        for feature in words("iris.features.optional") {
            if FEATURES.contains(&feature.as_str()) {
                self.define(format!("IRIS_FEATURE_{feature}"), "");
            }
        }
        words("iris.features.required")
            .into_iter()
            .filter(|f| !FEATURES.contains(&f.as_str()))
            .map(|f| format!("the pack requires `{f}`, which is not supported yet"))
            .collect()
    }

    pub fn define(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let body = value.into();
        self.0.insert(name.into(), Macro { params: None, body });
    }

    pub fn define_function(&mut self, name: &str, params: Vec<String>, body: &str) {
        let body = body.to_owned();
        self.0.insert(
            name.to_owned(),
            Macro {
                params: Some(params),
                body,
            },
        );
    }

    pub fn undef(&mut self, name: &str) {
        self.0.remove(name);
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// The value of an object-like macro.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .filter(|m| m.params.is_none())
            .map(|m| m.body.as_str())
    }

    /// Whether `text` calls a function-like macro whose arguments go on past its end.
    pub(crate) fn unfinished_call(&self, text: &str) -> bool {
        let tokens = lex(text);
        tokens.iter().enumerate().any(|(i, t)| {
            self.0.get(*t).is_some_and(|m| m.params.is_some())
                && tokens[i + 1..]
                    .iter()
                    .find(|t| !t.trim().is_empty())
                    .is_some_and(|t| *t == "(")
                && arguments(&tokens, i + 1).is_none()
        })
    }

    /// `text` with every macro expanded, repeatedly; a macro is not expanded again inside its own
    /// expansion, so `A` defined as `A + 1` stops. Function-like macros take their arguments
    /// (expanded first, raw beside `##`, which pastes) when a `(` follows their name.
    pub fn expand(&self, text: &str) -> String {
        self.expand_hiding(text, &mut Vec::new())
    }

    fn expand_hiding<'a>(&'a self, text: &str, hidden: &mut Vec<&'a str>) -> String {
        let mut out = String::with_capacity(text.len());
        let tokens = lex(text);
        let mut i = 0;
        while i < tokens.len() {
            let word = tokens[i];
            i += 1;
            let Some((name, m)) = self.0.get_key_value(word) else {
                out.push_str(word);
                continue;
            };
            if hidden.contains(&name.as_str()) {
                out.push_str(word);
                continue;
            }
            let body = match &m.params {
                None => m.body.clone(),
                Some(params) => {
                    let Some((args, next)) = arguments(&tokens, i) else {
                        out.push_str(word);
                        continue;
                    };
                    i = next;
                    self.substitute(params, &args, &m.body, hidden)
                }
            };
            hidden.push(name);
            out.push_str(&self.expand_hiding(&body, hidden));
            hidden.pop();
        }
        out
    }

    /// A function-like macro's body with its parameters replaced.
    fn substitute<'a>(
        &'a self,
        params: &[String],
        args: &[String],
        body: &str,
        hidden: &mut Vec<&'a str>,
    ) -> String {
        let tokens = lex(body);
        let pasted = |at: usize, step: isize| {
            let mut j = at as isize + step;
            while (0..tokens.len() as isize).contains(&j) && tokens[j as usize].trim().is_empty() {
                j += step;
            }
            (0..tokens.len() as isize).contains(&j) && tokens[j as usize] == "##"
        };
        let mut out = String::new();
        for (at, token) in tokens.iter().enumerate() {
            match params.iter().position(|p| p == token) {
                Some(k) => {
                    let arg = args.get(k).map_or("", |a| a.trim());
                    if pasted(at, -1) || pasted(at, 1) {
                        out.push_str(arg);
                    } else {
                        out.push_str(&self.expand_hiding(arg, hidden));
                    }
                }
                None => out.push_str(token),
            }
        }
        // `a ## b` → `ab`.
        let mut joined = String::with_capacity(out.len());
        for (k, part) in out.split("##").enumerate() {
            joined.push_str(if k == 0 { part.trim_end() } else { part.trim() });
        }
        if out.contains("##") { joined } else { out }
    }
}

/// The arguments of a call whose `(` is the next non-blank token from `i`, split at top-level
/// commas, and the index after its `)`. `None` when no `(` follows or it never closes.
fn arguments(tokens: &[&str], mut i: usize) -> Option<(Vec<String>, usize)> {
    while tokens.get(i)?.trim().is_empty() {
        i += 1;
    }
    if tokens[i] != "(" {
        return None;
    }
    let (mut depth, mut args, mut from) = (0, Vec::new(), i + 1);
    for (j, t) in tokens.iter().enumerate().skip(i + 1) {
        match *t {
            "(" => depth += 1,
            ")" if depth == 0 => {
                args.push(tokens[from..j].concat());
                return Some((args, j + 1));
            }
            ")" => depth -= 1,
            "," if depth == 0 => {
                args.push(tokens[from..j].concat());
                from = j + 1;
            }
            _ => {}
        }
    }
    None
}

/// Preprocessing tokens: identifiers, numbers (with their suffixes and exponents), runs of
/// white space, `##`, and single characters. Concatenated they give back `text`.
pub(crate) fn lex(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let start = i;
        i += 1;
        if is_ident_start(c) {
            while i < bytes.len() && is_ident(bytes[i] as char) {
                i += 1;
            }
        } else if c.is_ascii_digit() || (c == '.' && bytes.get(i).is_some_and(u8::is_ascii_digit)) {
            while i < bytes.len() {
                let d = bytes[i] as char;
                let exp_sign = (d == '+' || d == '-') && matches!(bytes[i - 1], b'e' | b'E');
                if !(is_ident(d) || d == '.' || exp_sign) {
                    break;
                }
                i += 1;
            }
        } else if c.is_whitespace() {
            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
        } else if c == '#' && bytes.get(i) == Some(&b'#') {
            i += 1;
        } else {
            while !text.is_char_boundary(i) {
                i += 1;
            }
        }
        out.push(&text[start..i]);
    }
    out
}

pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

pub(crate) fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iris_features_are_defined_or_noted() {
        let properties = crate::Properties::parse(
            "iris.features.optional = COMPUTE_SHADERS NOT_A_FEATURE\n\
             iris.features.required = ENTITY_TRANSLUCENT NOT_A_FEATURE\n",
            &Macros::standard(),
        );
        let mut m = Macros::standard();
        let notes = m.define_features(&properties);
        assert!(m.is_defined("IRIS_FEATURE_COMPUTE_SHADERS"));
        assert!(!m.is_defined("IRIS_FEATURE_NOT_A_FEATURE"));
        assert_eq!(
            notes,
            ["the pack requires `NOT_A_FEATURE`, which is not supported yet"]
        );
    }

    #[test]
    fn versions_follow_the_documented_rule() {
        assert_eq!(mc_version("1.21"), 12100);
        assert_eq!(mc_version("1.21.0"), 12100);
        assert_eq!(mc_version("1.7.10"), 10710);
        assert_eq!(mc_version("26.3"), 260300);
        assert_eq!(MC_VERSION, 260300);
        assert_eq!(Macros::standard().get("MC_VERSION"), Some("260300"));
    }

    #[test]
    fn expansion_replaces_whole_words_and_stops_on_recursion() {
        let mut m = Macros::default();
        m.define("RES", "SIZE");
        m.define("SIZE", "512");
        m.define("LOOP", "LOOP + 1");
        assert_eq!(m.expand("RES RES 0.5"), "512 512 0.5");
        assert_eq!(
            m.expand("RESOLUTION colortex4 LOOP"),
            "RESOLUTION colortex4 LOOP + 1"
        );
        m.define("e5", "x");
        assert_eq!(m.expand("1e5"), "1e5");
    }

    #[test]
    fn function_like_macros_take_arguments_and_paste() {
        let mut m = Macros::default();
        m.define("N", "4");
        m.define_function("SQ", vec!["x".into()], "((x) * (x))");
        m.define_function("CAT", vec!["a".into(), "b".into()], "a ## b");
        m.define_function("PI", vec![], "3.14");
        assert_eq!(m.expand("SQ(N + 1)"), "((4 + 1) * (4 + 1))");
        assert_eq!(m.expand("SQ(min(N, 2))"), "((min(4, 2)) * (min(4, 2)))");
        assert_eq!(m.expand("CAT(N, 2) CAT(col, ortex4)"), "N2 colortex4");
        assert_eq!(m.expand("PI() SQ SQ(1"), "3.14 SQ SQ(1");
        assert_eq!(m.get("SQ"), None);
    }
}
