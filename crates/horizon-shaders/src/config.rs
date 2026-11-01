//! The configuration packs write into their stage files (spec 1.7, 6.1): which buffers a fragment
//! shader writes (`/* DRAWBUFFERS:0257 */`, `/* RENDERTARGETS: 0,2 */`) and, per colortex buffer,
//! its format (`const int colortex2Format = RGBA16F;`, inside a block comment because the format
//! names are not GLSL), whether and to what it is cleared, and whether it gets mipmaps.
//!
//! Everything is read from live code only: the preprocessor has already dropped the false
//! branches, so a format chosen by an option is the one the option selects. Every stage file of
//! the programs that run is read (OQ-17); a later file overrides an earlier one, with a note when
//! the values differ.

use crate::Preprocessed;

/// colortex buffers (6.1.1).
pub const BUFFERS: usize = 16;

/// A colour format of 6.7, as channels, bits per channel and kind. Packed formats are described
/// by their widest channel: `RGB10_A2` is 4 × 10 bits, `R11F_G11F_B10F` 3 × 11 bits float.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub channels: u8,
    pub bits: u8,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Unorm,
    Snorm,
    Float,
    Int,
    Uint,
}

impl Format {
    /// The default `RGBA` (6.1.3).
    pub const RGBA8: Format = Format {
        channels: 4,
        bits: 8,
        kind: Kind::Unorm,
    };

    /// A format name of 6.7, in any case.
    pub fn parse(name: &str) -> Option<Format> {
        let name = name.trim().to_ascii_uppercase();
        let packed = |channels, bits, kind| {
            Some(Format {
                channels,
                bits,
                kind,
            })
        };
        match name.as_str() {
            "RGBA" => return Some(Self::RGBA8),
            "R3_G3_B2" => return packed(3, 3, Kind::Unorm),
            "RGB5_A1" => return packed(4, 5, Kind::Unorm),
            "RGB10_A2" => return packed(4, 10, Kind::Unorm),
            "RGB10_A2UI" => return packed(4, 10, Kind::Uint),
            "R11F_G11F_B10F" => return packed(3, 11, Kind::Float),
            "RGB9_E5" => return packed(3, 9, Kind::Float),
            "RGBA4" => return packed(4, 4, Kind::Unorm),
            "RGB565" => return packed(3, 6, Kind::Unorm),
            "RGBA2" => return packed(4, 2, Kind::Unorm),
            _ => {}
        }
        let letters = name.bytes().take_while(u8::is_ascii_alphabetic).count();
        let channels = match &name[..letters] {
            "R" => 1,
            "RG" => 2,
            "RGB" => 3,
            "RGBA" => 4,
            _ => return None,
        };
        let rest = &name[letters..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let bits: u8 = rest[..digits].parse().ok()?;
        let kind = match &rest[digits..] {
            "" => Kind::Unorm,
            "_SNORM" => Kind::Snorm,
            "F" => Kind::Float,
            "I" => Kind::Int,
            "UI" => Kind::Uint,
            _ => return None,
        };
        let valid = match kind {
            Kind::Unorm | Kind::Snorm => matches!(bits, 8 | 16),
            Kind::Float => matches!(bits, 16 | 32),
            Kind::Int | Kind::Uint => matches!(bits, 8 | 16 | 32),
        };
        valid.then_some(Format {
            channels,
            bits,
            kind,
        })
    }
}

/// The settings of one colortex buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct Buffer {
    pub format: Format,
    /// Cleared every frame (`<buffer>Clear`, default true).
    pub clear: bool,
    /// `<buffer>ClearColor`, else 6.1.5's default: colortex0 the fog colour, colortex1 white,
    /// the others transparent black.
    pub clear_color: Option<[f32; 4]>,
    /// `<buffer>MipmapEnabled`.
    pub mipmap: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            format: Format::RGBA8,
            clear: true,
            clear_color: None,
            mipmap: false,
        }
    }
}

/// The shadow map's settings (6.3, 7.1), from `const` lines of the stage files.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    /// `shadowMapResolution`: the map's side in pixels.
    pub resolution: u32,
    /// `shadowDistance`: half the side of the area the map covers, in blocks.
    pub distance: f32,
    /// `shadowDistanceRenderMul`: when > 0, geometry beyond `distance × mul` is not drawn.
    pub render_mul: f32,
    /// `voxelDistance`: with `shadow.culling=reversed`, nothing within it is culled.
    pub voxel_distance: f32,
    /// `shadowIntervalSize`: the shadow camera moves in steps of this many blocks (0: none).
    pub interval: f32,
    /// Native packs: the snap's remainder is euclidean, so the centre is within half an interval
    /// of the player at negative coordinates too. `shaders/` packs keep Iris's.
    pub euclidean_snap: bool,
    /// `sunPathRotation`, degrees.
    pub sun_path_rotation: f32,
    /// `shadowMapFov`: a perspective projection of this many degrees instead of orthographic.
    pub fov: Option<f32>,
    /// `shadowNearPlane`, `shadowFarPlane`.
    pub near: f32,
    pub far: f32,
    /// `shadowHardwareFiltering(0|1)`: shadowtex0/1 sampled with depth comparison.
    pub hardware_filtering: [bool; 2],
    /// `shadowcolor0/1`: format, cleared every frame, clear colour (default white).
    pub colors: [Buffer; 2],
}

impl Default for Shadow {
    fn default() -> Self {
        let white = Buffer {
            clear_color: Some([1.0; 4]),
            ..Buffer::default()
        };
        Self {
            resolution: 1024,
            distance: 160.0,
            render_mul: -1.0,
            voxel_distance: 160.0,
            interval: 2.0,
            euclidean_snap: false,
            sun_path_rotation: 0.0,
            fov: None,
            near: 0.05,
            far: 256.0,
            hardware_filtering: [false; 2],
            colors: [white.clone(), white],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub buffers: [Buffer; BUFFERS],
    pub shadow: Shadow,
    /// Conflicting or unreadable settings, as `file:line: message`.
    pub notes: Vec<String>,
    format_set: [bool; BUFFERS],
    /// Each setting read so far and its value, to report a later file changing it.
    seen: Vec<(String, String)>,
    gdepth: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            buffers: std::array::from_fn(|_| Buffer::default()),
            shadow: Shadow::default(),
            notes: Vec::new(),
            format_set: [false; BUFFERS],
            seen: Vec::new(),
            gdepth: false,
        }
    }
}

/// The colortex index of a buffer name or alias (6.1.1).
pub fn buffer_index(name: &str) -> Option<usize> {
    let alias = [
        "gcolor",
        "gdepth",
        "gnormal",
        "composite",
        "gaux1",
        "gaux2",
        "gaux3",
        "gaux4",
    ];
    if let Some(i) = alias.iter().position(|a| *a == name) {
        return Some(i);
    }
    let n: usize = name.strip_prefix("colortex")?.parse().ok()?;
    (n < BUFFERS).then_some(n)
}

impl Config {
    /// Reads one preprocessed stage file.
    pub fn read(&mut self, pp: &Preprocessed) {
        let line = |at: usize| pp.locate(code_line(&pp.code, at));
        for (at, ty, name, value) in constants(&pp.code) {
            let place = line(at);
            self.constant(&place, ty, name, value);
        }
        for (origin, comment) in &pp.comments {
            for (_, ty, name, value) in constants(comment) {
                let file = pp
                    .files
                    .get(origin.file as usize)
                    .map_or("?", String::as_str);
                let place = format!("{file}:{}", origin.line);
                self.constant(&place, ty, name, value);
            }
        }
        if declares(&pp.code, "gdepth") {
            self.gdepth = true;
        }
    }

    /// The settings once every file is read: buffer 1 becomes RGBA32F when a program declares
    /// `gdepth` and the pack gave it no format (6.1.3).
    pub fn finish(mut self) -> Self {
        if self.gdepth && !self.format_set[1] {
            self.buffers[1].format = Format {
                channels: 4,
                bits: 32,
                kind: Kind::Float,
            };
        }
        self
    }

    fn constant(&mut self, place: &str, ty: &str, name: &str, value: &str) {
        if self.shadow_constant(ty, name, value) {
            return;
        }
        let settings = [
            ("Format", "int"),
            ("ClearColor", "vec4"),
            ("Clear", "bool"),
            ("MipmapEnabled", "bool"),
        ];
        let Some((buffer, setting)) = settings.iter().find_map(|(suffix, want)| {
            let buffer = buffer_index(name.strip_suffix(suffix)?)?;
            (ty == *want).then_some((buffer, *suffix))
        }) else {
            return;
        };
        let b = &mut self.buffers[buffer];
        let ok = match setting {
            "Format" => Format::parse(value).map(|f| {
                b.format = f;
                self.format_set[buffer] = true;
            }),
            "ClearColor" => vec4(value).map(|c| b.clear_color = Some(c)),
            "Clear" => boolean(value).map(|v| b.clear = v),
            _ => boolean(value).map(|v| b.mipmap = v),
        };
        if ok.is_none() {
            let problem = match setting {
                "Format" => format!("unknown format `{value}`"),
                "ClearColor" => format!("`{name}` is not a vec4: `{value}`"),
                _ => format!("`{name}` is not a bool: `{value}`"),
            };
            self.notes.push(format!("{place}: {problem}"));
            return;
        }
        // The same setting under another name (an alias) counts as the same.
        let key = format!("{buffer}{setting}");
        match self.seen.iter_mut().find(|(k, _)| *k == key) {
            Some((_, old)) if old != value => {
                self.notes.push(format!(
                    "{place}: `{name}` changed by a later file to `{value}`"
                ));
                *old = value.to_owned();
            }
            Some(_) => {}
            None => self.seen.push((key, value.to_owned())),
        }
    }
}

impl Config {
    /// A shadow setting of 6.3 or 7.1; true when `name` is one (read or not).
    fn shadow_constant(&mut self, ty: &str, name: &str, value: &str) -> bool {
        let s = &mut self.shadow;
        let float = |v: &mut f32| {
            if let Some(n) = number(value) {
                *v = n;
            }
        };
        match name {
            "shadowMapResolution" => {
                if let Some(n) = number(value).filter(|n| *n >= 1.0) {
                    s.resolution = (n as u32).min(16384);
                }
            }
            "shadowDistance" => float(&mut s.distance),
            "shadowDistanceRenderMul" => float(&mut s.render_mul),
            "voxelDistance" => float(&mut s.voxel_distance),
            "shadowIntervalSize" => float(&mut s.interval),
            "sunPathRotation" => float(&mut s.sun_path_rotation),
            "shadowNearPlane" => float(&mut s.near),
            "shadowFarPlane" => float(&mut s.far),
            "shadowMapFov" => s.fov = number(value).filter(|f| *f > 0.0),
            "shadowHardwareFiltering" => {
                if let Some(b) = boolean(value) {
                    s.hardware_filtering = [b; 2];
                }
            }
            "shadowHardwareFiltering0" | "shadowHardwareFiltering1" => {
                if let Some(b) = boolean(value) {
                    s.hardware_filtering[usize::from(name.ends_with('1'))] = b;
                }
            }
            _ => {
                let color = ["shadowcolor0", "shadowcolor1", "shadowcolor"]
                    .iter()
                    .find_map(|b| Some((b, name.strip_prefix(b)?)));
                let Some((buffer, setting)) = color else {
                    return false;
                };
                let b = &mut s.colors[usize::from(buffer.ends_with('1'))];
                match (setting, ty) {
                    ("Format", "int") => {
                        if let Some(f) = Format::parse(value) {
                            b.format = f;
                        }
                    }
                    ("Clear", "bool") => {
                        if let Some(v) = boolean(value) {
                            b.clear = v;
                        }
                    }
                    ("ClearColor", "vec4") => {
                        if let Some(c) = vec4(value) {
                            b.clear_color = Some(c);
                        }
                    }
                    _ => return false,
                }
            }
        }
        true
    }
}

/// The buffers a fragment shader writes, from the last `DRAWBUFFERS` or `RENDERTARGETS` comment
/// of its live code, or `None` without one (then it writes buffers 0 to 7). 6.1.4.
pub fn draw_buffers(pp: &Preprocessed) -> Option<Vec<u32>> {
    let mut found = None;
    for (_, comment) in &pp.comments {
        let text = comment.trim();
        if let Some(digits) = text.strip_prefix("DRAWBUFFERS:") {
            let list: Option<Vec<u32>> = digits.trim().chars().map(|c| c.to_digit(10)).collect();
            found = list.or(found);
        } else if let Some(list) = text.strip_prefix("RENDERTARGETS:") {
            let list: Option<Vec<u32>> = list
                .split(',')
                .map(|n| {
                    n.trim()
                        .parse()
                        .ok()
                        .filter(|&n: &u32| (n as usize) < BUFFERS)
                })
                .collect();
            found = list.or(found);
        }
    }
    found.map(|mut list| {
        list.truncate(8);
        list
    })
}

/// `const <type> <name> = <value>;` anywhere in `text`: (byte offset, type, name, value).
fn constants(text: &str) -> Vec<(usize, &str, &str, &str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("const") {
        let at = from + found;
        from = at + 5;
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let rest = &text[from..];
        let Some(end) = rest.find(';') else { break };
        let decl = &rest[..end];
        let Some((left, value)) = decl.split_once('=') else {
            continue;
        };
        let mut words = left.split_whitespace();
        if let (Some(ty), Some(name), None) = (words.next(), words.next(), words.next()) {
            out.push((at, ty, name, value.trim()));
        }
    }
    out
}

/// Whether `text` declares a uniform sampler called `name`.
fn declares(text: &str, name: &str) -> bool {
    text.split(';').any(|decl| {
        let words: Vec<&str> = decl.split_whitespace().collect();
        words.first() == Some(&"uniform") && words.last() == Some(&name)
    })
}

fn code_line(code: &str, at: usize) -> usize {
    code[..at].matches('\n').count()
}

fn boolean(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn number(text: &str) -> Option<f32> {
    let text = text.trim();
    text.strip_suffix(['f', 'F']).unwrap_or(text).parse().ok()
}

/// `vec4(r, g, b, a)` or `vec4(x)`.
fn vec4(value: &str) -> Option<[f32; 4]> {
    let inner = value
        .strip_prefix("vec4")?
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let parts: Option<Vec<f32>> = inner.split(',').map(number).collect();
    match parts?.as_slice() {
        [x] => Some([*x; 4]),
        [r, g, b, a] => Some([*r, *g, *b, *a]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;
    use crate::{Macros, Pack, preprocess};

    fn stage(text: &str) -> Preprocessed {
        let pack = Pack::from_zip_bytes("t", zip(&[("shaders/composite.fsh", text)])).unwrap();
        preprocess(&pack, "composite.fsh", &Macros::standard())
    }

    #[test]
    fn formats_parse() {
        let f = |channels, bits, kind| {
            Some(Format {
                channels,
                bits,
                kind,
            })
        };
        assert_eq!(Format::parse("rgba16f"), f(4, 16, Kind::Float));
        assert_eq!(Format::parse("R32UI"), f(1, 32, Kind::Uint));
        assert_eq!(Format::parse("RG8_SNORM"), f(2, 8, Kind::Snorm));
        assert_eq!(Format::parse("R11F_G11F_B10F"), f(3, 11, Kind::Float));
        assert_eq!(Format::parse("RGB16"), f(3, 16, Kind::Unorm));
        assert_eq!(Format::parse("RGBA12"), None);
        assert_eq!(Format::parse("R16_SNORMX"), None);
    }

    #[test]
    fn draw_buffers_and_constants_come_from_live_code() {
        let pp = stage(
            "#version 120\n\
             #define HQ\n\
             /*\n\
             const int colortex2Format = RGBA16F;\n\
             const int gaux4Format = R32F;\n\
             */\n\
             #ifdef HQ\n\
             /* RENDERTARGETS: 0,2,11 */\n\
             #else\n\
             /* DRAWBUFFERS:01 */\n\
             const bool colortex5Clear = true;\n\
             #endif\n\
             const bool colortex5Clear = false;\n\
             const vec4 colortex3ClearColor = vec4(0.5, 0.25, 0.0, 1.0f);\n\
             const bool colortex3MipmapEnabled = true;\n\
             const vec4 gnormalClearColor = vec4(1.0);\n\
             uniform sampler2D gdepth;\n\
             void main() {}\n",
        );
        assert_eq!(draw_buffers(&pp), Some(vec![0, 2, 11]));
        let mut config = Config::default();
        config.read(&pp);
        let config = config.finish();
        let b = &config.buffers;
        assert_eq!(b[2].format, Format::parse("RGBA16F").unwrap());
        assert_eq!(b[7].format, Format::parse("R32F").unwrap());
        assert!(!b[5].clear && b[0].clear);
        assert_eq!(b[3].clear_color, Some([0.5, 0.25, 0.0, 1.0]));
        assert!(b[3].mipmap && !b[2].mipmap);
        assert_eq!(b[2].clear_color, Some([1.0; 4]));
        assert_eq!(b[1].format, Format::parse("RGBA32F").unwrap(), "gdepth");
        assert!(config.notes.is_empty(), "{:?}", config.notes);
    }

    #[test]
    fn shadow_settings() {
        let mut c = Config::default();
        c.read(&stage(
            "const int shadowMapResolution = 2048;\nconst float shadowDistance = 128.0f;\n\
             const float sunPathRotation = -40.0;\nconst bool shadowHardwareFiltering = true;\n\
             /* const int shadowcolor1Format = RGBA16F; */\nconst bool shadowcolor0Clear = false;\n",
        ));
        let s = c.finish().shadow;
        assert_eq!(
            (s.resolution, s.distance, s.sun_path_rotation),
            (2048, 128.0, -40.0)
        );
        assert_eq!(
            (s.interval, s.fov, s.hardware_filtering),
            (2.0, None, [true; 2])
        );
        assert!(!s.colors[0].clear && s.colors[1].clear);
        assert_eq!(s.colors[1].format, Format::parse("RGBA16F").unwrap());
        assert_eq!(s.colors[1].clear_color, Some([1.0; 4]));
    }

    #[test]
    fn no_directive_and_conflicts() {
        let pp = stage("void main() {}\n");
        assert_eq!(draw_buffers(&pp), None);
        let a =
            stage("/* const int colortex1Format = RGB8; */\nconst bool colortex1Clear = maybe;\n");
        let b =
            stage("/* const int colortex1Format = RGBA16; const int colortex2Format = XYZ; */\n");
        let mut config = Config::default();
        config.read(&a);
        config.read(&b);
        let notes = config.notes.join("\n");
        assert!(
            notes.contains("composite.fsh:2: `colortex1Clear` is not a bool"),
            "{notes}"
        );
        assert!(
            notes.contains("composite.fsh:1: `colortex1Format` changed"),
            "{notes}"
        );
        assert!(notes.contains("unknown format `XYZ`"), "{notes}");
        assert_eq!(
            config.finish().buffers[1].format,
            Format::parse("RGBA16").unwrap()
        );
    }
}
