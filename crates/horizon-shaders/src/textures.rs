//! Custom textures: `texture.<stage>.<name>` and `texture.noise` in `shaders.properties` (3.4),
//! and Iris's `customTexture.<name>`, a sampler of its own that every program may read. A
//! texture is a PNG of the pack, a raw dump with its type, formats and size, or a resource
//! location the game resolves (`minecraft:textures/...`, `minecraft:dynamic/lightmap_1`).

use crate::{Format, Kind, Pack, Properties};

/// The stage of a `customTexture.<name>`: every program's.
pub const EVERY_STAGE: &str = "*";

/// A texture bound to sampler `name` in the programs of `stage` (`gbuffers`, `deferred`,
/// `composite`...; [`EVERY_STAGE`] for a `customTexture`), or the noise texture (stage and
/// name `noise`).
#[derive(Clone, Debug)]
pub struct CustomTexture {
    pub stage: String,
    pub name: String,
    pub source: TextureSource,
}

#[derive(Clone, Debug)]
pub enum TextureSource {
    Image(Texture),
    /// A resource location for the game to resolve.
    Resource(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dim {
    D1,
    D2,
    D3,
}

#[derive(Clone, Debug)]
pub struct Texture {
    pub dim: Dim,
    /// Width, height and depth (1 for the dimensions the texture lacks).
    pub size: [u32; 3],
    /// The internal format the pack asked for.
    pub format: Format,
    pub texels: Texels,
    /// From the `.mcmeta` beside it: linear filtering, and clamping instead of repeating.
    pub blur: bool,
    pub clamp: bool,
}

/// Texel values, `components` per texel: 1, 2 or 4 (three widen to four, alpha 1).
#[derive(Clone, Debug, PartialEq)]
pub enum Texels {
    Float(Vec<f32>),
    Int(Vec<i32>),
    Uint(Vec<u32>),
}

impl Texture {
    pub fn components(&self) -> usize {
        components(self.format.channels)
    }
}

fn components(channels: u8) -> usize {
    match channels {
        1 => 1,
        2 => 2,
        _ => 4,
    }
}

/// Every `texture.*` and `customTexture.*` entry, read and decoded, and a note for each that
/// failed.
pub fn load_textures(pack: &Pack, properties: &Properties) -> (Vec<CustomTexture>, Vec<String>) {
    let (mut textures, mut notes) = (Vec::new(), Vec::new());
    for e in &properties.entries {
        let (stage, name) = if let Some(name) = e.key.strip_prefix("customTexture.") {
            (EVERY_STAGE, name)
        } else if let Some(rest) = e.key.strip_prefix("texture.") {
            match rest {
                "noise" => ("noise", "noise"),
                _ => match rest.split_once('.') {
                    // `.0` to `.9` after the name only keep keys apart.
                    Some((stage, name)) => (stage, strip_suffix(name)),
                    None => {
                        notes.push(format!("`{}`: no sampler name", e.key));
                        continue;
                    }
                },
            }
        } else {
            continue;
        };
        match source(pack, e.value.trim()) {
            Ok(source) => textures.push(CustomTexture {
                stage: stage.to_owned(),
                name: name.to_owned(),
                source,
            }),
            Err(why) => notes.push(format!("`{}`: {why}", e.key)),
        }
    }
    (textures, notes)
}

/// Decodes the custom textures that name one of the game's textures by its resource location
/// (`customTexture.clouds = minecraft:textures/environment/clouds.png`; Iris reads these from the
/// game's resources, like a PNG of the pack). `read` gives a resource's bytes by namespace and
/// path (resource packs over the game's own). The game's generated textures
/// (`minecraft:dynamic/lightmap_1`, the atlases) stay for the renderer; one that can't be read
/// or decoded is dropped, with a note.
pub fn resolve_resources(
    textures: &mut Vec<CustomTexture>,
    read: &dyn Fn(&str, &str) -> Option<Vec<u8>>,
) -> Vec<String> {
    let mut notes = Vec::new();
    textures.retain_mut(|t| {
        let TextureSource::Resource(location) = &t.source else {
            return true;
        };
        let (namespace, path) = location.split_once(':').unwrap_or(("minecraft", location));
        if path.starts_with("dynamic/") || path.starts_with("textures/atlas/") {
            return true;
        }
        let key = match t.stage.as_str() {
            EVERY_STAGE => format!("customTexture.{}", t.name),
            stage => format!("texture.{stage}.{}", t.name),
        };
        // A resource location's characters (`ResourceLocation`): nothing reaches outside the
        // game's resources.
        let valid = |s: &str, slash: bool| {
            !s.is_empty()
                && s.bytes().all(|b| {
                    b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || b"_-.".contains(&b)
                        || (slash && b == b'/')
                })
                && s.split('/').all(|part| part != ".." && part != ".")
        };
        if !valid(namespace, false) || !valid(path, true) {
            notes.push(format!("`{key}`: `{location}` is not a resource location"));
            return false;
        }
        let Some(bytes) = read(namespace, path) else {
            notes.push(format!("`{key}`: the game has no `{location}`"));
            return false;
        };
        let Some(mut texture) = decode_png(&bytes) else {
            notes.push(format!("`{key}`: `{location}` is not a PNG"));
            return false;
        };
        let meta = read(namespace, &format!("{path}.mcmeta"));
        let text = meta.as_deref().map(String::from_utf8_lossy);
        (texture.blur, texture.clamp) = mcmeta(text.as_deref());
        t.source = TextureSource::Image(texture);
        true
    });
    notes
}

fn strip_suffix(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((base, n)) if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() => base,
        _ => name,
    }
}

fn source(pack: &Pack, value: &str) -> Result<TextureSource, String> {
    let mut words = value.split_whitespace();
    let path = words.next().ok_or("no path")?;
    if path.contains(':') {
        return Ok(TextureSource::Resource(path.to_owned()));
    }
    let path = path.trim_start_matches('/');
    let bytes = pack
        .read(path)
        .ok_or_else(|| format!("can't find `{path}`"))?;
    let (blur, clamp) = mcmeta(pack.read_text(&format!("{path}.mcmeta")).as_deref());
    let rest: Vec<&str> = words.collect();
    let mut texture = if rest.is_empty() {
        decode_png(&bytes).ok_or_else(|| format!("`{path}` is not a PNG"))?
    } else {
        raw(&bytes, &rest)?
    };
    texture.blur = blur;
    texture.clamp = clamp;
    Ok(TextureSource::Image(texture))
}

/// `{"texture": {"blur": true, "clamp": false}}`: both off when missing.
fn mcmeta(text: Option<&str>) -> (bool, bool) {
    let flag = |key: &str| {
        text.and_then(|t| t.split_once(&format!("\"{key}\"")))
            .map(|(_, after)| after.trim_start().trim_start_matches(':').trim_start())
            .is_some_and(|v| v.starts_with("true"))
    };
    (flag("blur"), flag("clamp"))
}

/// A PNG as 8-bit RGBA, rows top first.
pub fn decode_png(bytes: &[u8]) -> Option<Texture> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Packs ship PNGs with wrong chunk checksums (a server's obfuscated pack);
    // the game's other pack textures decode them, as stb_image does.
    decoder.ignore_checksums(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let pixels = &buf[..info.buffer_size()];
    let rgba: Vec<f32> = match info.color_type {
        png::ColorType::Rgba => pixels.iter().map(|&b| b as f32 / 255.0).collect(),
        png::ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .map(|b| b as f32 / 255.0)
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .map(|b| b as f32 / 255.0)
            .collect(),
        png::ColorType::Grayscale => pixels
            .iter()
            .flat_map(|&g| [g, g, g, 255])
            .map(|b| b as f32 / 255.0)
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Texture {
        dim: Dim::D2,
        size: [info.width, info.height, 1],
        format: Format::RGBA8,
        texels: Texels::Float(rgba),
        blur: false,
        clamp: false,
    })
}

/// The largest size a raw texture may give one of its dimensions.
const MAX_SIZE: u32 = 16384;

/// A raw dump: `<type> <internalFormat> <dimensions...> <pixelFormat> <pixelType>`, its rows
/// tightly packed, or padded to four bytes as GL unpacks by default.
fn raw(bytes: &[u8], words: &[&str]) -> Result<Texture, String> {
    let (dim, dims) = match words[0] {
        "TEXTURE_1D" => (Dim::D1, 1),
        "TEXTURE_2D" | "TEXTURE_RECTANGLE" => (Dim::D2, 2),
        "TEXTURE_3D" => (Dim::D3, 3),
        t => return Err(format!("unknown texture type `{t}`")),
    };
    if words.len() != 4 + dims {
        return Err(format!("`{}` takes {dims} sizes", words[0]));
    }
    let format = Format::parse(words[1]).ok_or_else(|| format!("unknown format `{}`", words[1]))?;
    let mut size = [1u32; 3];
    for (k, w) in words[2..2 + dims].iter().enumerate() {
        size[k] = w
            .parse()
            .ok()
            .filter(|&n| n > 0 && n <= MAX_SIZE)
            .ok_or(format!("bad size `{w}`"))?;
    }
    let (pixel_format, pixel_type) = (words[2 + dims], words[3 + dims]);
    let integer = pixel_format.ends_with("_INTEGER");
    let order: &[usize] = match pixel_format.trim_end_matches("_INTEGER") {
        "RED" | "R" | "LUMINANCE" => &[0],
        "GREEN" => &[1],
        "BLUE" => &[2],
        "ALPHA" => &[3],
        "RG" => &[0, 1],
        "RGB" => &[0, 1, 2],
        "BGR" => &[2, 1, 0],
        "RGBA" => &[0, 1, 2, 3],
        "BGRA" => &[2, 1, 0, 3],
        f => return Err(format!("unknown pixel format `{f}`")),
    };
    let (width, read): (usize, fn(&[u8]) -> Value) = match pixel_type {
        "UNSIGNED_BYTE" => (1, |b| Value::U(b[0] as u32, u8::MAX as f32)),
        "BYTE" => (1, |b| Value::I(b[0] as i8 as i32, i8::MAX as f32)),
        "UNSIGNED_SHORT" => (2, |b| {
            Value::U(u16::from_le_bytes([b[0], b[1]]) as u32, u16::MAX as f32)
        }),
        "SHORT" => (2, |b| {
            Value::I(i16::from_le_bytes([b[0], b[1]]) as i32, i16::MAX as f32)
        }),
        "UNSIGNED_INT" => (4, |b| {
            Value::U(
                u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                u32::MAX as f32,
            )
        }),
        "INT" => (4, |b| {
            Value::I(
                i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                i32::MAX as f32,
            )
        }),
        "HALF_FLOAT" => (2, |b| {
            Value::F(half::f16::from_le_bytes([b[0], b[1]]).to_f32())
        }),
        "FLOAT" => (4, |b| {
            Value::F(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }),
        t => return Err(format!("unknown pixel type `{t}`")),
    };
    let texel = order.len() * width;
    let row = size[0] as usize * texel;
    let rows = size[1] as usize * size[2] as usize;
    let padded = row.next_multiple_of(4);
    let wanted = |stride: usize| stride.checked_mul(rows - 1)?.checked_add(row);
    let stride = if padded != row && wanted(padded).is_some_and(|n| bytes.len() >= n) {
        padded
    } else {
        row
    };
    match wanted(stride) {
        Some(n) if bytes.len() >= n => {}
        n => {
            return Err(format!(
                "{} bytes, {} wanted",
                bytes.len(),
                n.map_or_else(|| "more than that".into(), |n| n.to_string())
            ));
        }
    }
    let out = components(format.channels);
    let count = size[0] as usize * rows;
    let mut values = Vec::with_capacity(count * out);
    for r in 0..rows {
        for x in 0..size[0] as usize {
            let at = r * stride + x * texel;
            let mut rgba = [Value::F(0.0), Value::F(0.0), Value::F(0.0), Value::F(1.0)];
            for (k, &c) in order.iter().enumerate() {
                rgba[c] = read(&bytes[at + k * width..]);
            }
            values.extend_from_slice(&rgba[..out]);
        }
    }
    let texels = match format.kind {
        Kind::Int => Texels::Int(values.iter().map(|v| v.int()).collect()),
        Kind::Uint => Texels::Uint(values.iter().map(|v| v.int() as u32).collect()),
        // Integer data fills normalized and float formats normalized, as GL unpacks it.
        _ => Texels::Float(values.iter().map(|v| v.float(!integer)).collect()),
    };
    Ok(Texture {
        dim,
        size,
        format,
        texels,
        blur: false,
        clamp: false,
    })
}

#[derive(Clone, Copy)]
enum Value {
    U(u32, f32),
    I(i32, f32),
    F(f32),
}

impl Value {
    fn float(self, normalize: bool) -> f32 {
        match self {
            Value::U(v, max) if normalize => v as f32 / max,
            Value::I(v, max) if normalize => (v as f32 / max).max(-1.0),
            Value::U(v, _) => v as f32,
            Value::I(v, _) => v as f32,
            Value::F(v) => v,
        }
    }

    fn int(self) -> i32 {
        match self {
            Value::U(v, _) => v as i32,
            Value::I(v, _) => v,
            Value::F(v) => v as i32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Macros;
    use crate::pack::tests::zip;

    fn half(v: f32) -> [u8; 2] {
        half::f16::from_f32(v).to_le_bytes()
    }

    #[test]
    fn a_png_with_a_wrong_header_checksum_decodes() {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255])
                .unwrap();
            writer.finish().unwrap();
        }
        let ihdr = encoded.windows(4).position(|w| w == b"IHDR").unwrap();
        encoded[ihdr + 4 + 13] ^= 0xFF;
        assert!(decode_png(&encoded).is_some());
    }

    #[test]
    fn raw_sizes_are_bounded() {
        let huge = raw(
            &[0; 64],
            &[
                "TEXTURE_3D",
                "RGBA8",
                "4",
                "65536",
                "65536",
                "RGBA",
                "UNSIGNED_BYTE",
            ],
        );
        assert_eq!(huge.err().as_deref(), Some("bad size `65536`"));
        let big = raw(
            &[0; 64],
            &[
                "TEXTURE_3D",
                "RGBA8",
                "4",
                "16384",
                "16384",
                "RGBA",
                "UNSIGNED_BYTE",
            ],
        );
        assert!(big.is_err_and(|e| e.starts_with("64 bytes")));
    }

    /// Iris's `customTexture.<name>`: a raw 3D texture (Shrimple's `texClouds`) under a sampler
    /// name of its own, for every stage; `texture.*` beside it keeps its stage.
    #[test]
    fn custom_textures_are_for_every_stage() {
        // 2x2x2 RGBA8: texel i holds (i, 255 - i, 7, 255), x fastest.
        let raw: Vec<u8> = (0..8u8).flat_map(|i| [i, 255 - i, 7, 255]).collect();
        let files = [("shaders/clouds.dat", raw.as_slice())];
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        let properties = "customTexture.texClouds = clouds.dat TEXTURE_3D RGBA8 2 2 2 RGBA UNSIGNED_BYTE\n\
                          customTexture.gone = gone.png\n\
                          texture.composite.colortex5 = clouds.dat TEXTURE_3D RGBA8 2 2 2 RGBA UNSIGNED_BYTE\n";
        let (textures, notes) =
            load_textures(&pack, &Properties::parse(properties, &Macros::standard()));
        assert_eq!(notes, ["`customTexture.gone`: can't find `gone.png`"]);
        let found: Vec<(&str, &str)> = textures
            .iter()
            .map(|t| (t.stage.as_str(), t.name.as_str()))
            .collect();
        assert_eq!(
            found,
            [(EVERY_STAGE, "texClouds"), ("composite", "colortex5")]
        );
        let TextureSource::Image(clouds) = &textures[0].source else {
            panic!("{:?}", textures[0].source);
        };
        assert_eq!((clouds.dim, clouds.size), (Dim::D3, [2, 2, 2]));
        let expected: Vec<f32> = (0..8)
            .flat_map(|i| [i as f32 / 255.0, (255 - i) as f32 / 255.0, 7.0 / 255.0, 1.0])
            .collect();
        assert_eq!(clouds.texels, Texels::Float(expected));
    }

    #[test]
    fn raw_textures_and_their_stages() {
        // A 2x1x2 RGB16F LUT, an R8 texture with a suffix to keep its key apart, a PNG with
        // its mcmeta, the noise texture and a resource.
        let lut: Vec<u8> = (0..4)
            .flat_map(|i| [half(i as f32), half(0.5), half(-1.0)])
            .flatten()
            .collect();
        let mut png = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png, 1, 1);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&[255, 0, 51])
                .unwrap();
        }
        let properties = "texture.deferred.depthtex0 = lut.dat TEXTURE_3D RGB16F 2 1 2 RGB HALF_FLOAT\n\
                          texture.composite.colortex6.1 = r8.dat TEXTURE_2D R8 2 2 RED UNSIGNED_BYTE\n\
                          texture.gbuffers.gaux1 = sky.png\ntexture.noise = sky.png\n\
                          texture.composite.colortex3 = minecraft:dynamic/lightmap_1\n\
                          texture.composite.gaux2 = gone.png\n";
        let r8 = [0u8, 255, 0, 0, 51, 0, 0, 0];
        let files = [
            ("shaders/lut.dat", lut.as_slice()),
            ("shaders/r8.dat", r8.as_slice()),
            ("shaders/sky.png", png.as_slice()),
            (
                "shaders/sky.png.mcmeta",
                b"{\"texture\": {\"blur\": true, \"clamp\":true}}".as_slice(),
            ),
        ];
        let pack = Pack::from_zip_bytes("P.zip", zip(&files)).unwrap();
        let (textures, notes) =
            load_textures(&pack, &Properties::parse(properties, &Macros::standard()));
        assert_eq!(notes, ["`texture.composite.gaux2`: can't find `gone.png`"]);
        let names: Vec<(&str, &str)> = textures
            .iter()
            .map(|t| (t.stage.as_str(), t.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("deferred", "depthtex0"),
                ("composite", "colortex6"),
                ("gbuffers", "gaux1"),
                ("noise", "noise"),
                ("composite", "colortex3")
            ]
        );
        let image = |i: usize| match &textures[i].source {
            TextureSource::Image(t) => t,
            s => panic!("{s:?}"),
        };
        let lut = image(0);
        assert_eq!(
            (lut.dim, lut.size, lut.components()),
            (Dim::D3, [2, 1, 2], 4)
        );
        assert_eq!(
            lut.texels,
            Texels::Float((0..4).flat_map(|i| [i as f32, 0.5, -1.0, 1.0]).collect())
        );
        // Rows of 2 bytes, padded to 4.
        assert_eq!(image(1).texels, Texels::Float(vec![0.0, 1.0, 0.2, 0.0]));
        let sky = image(2);
        assert!(sky.blur && sky.clamp);
        assert_eq!(sky.texels, Texels::Float(vec![1.0, 0.0, 0.2, 1.0]));
        assert!(
            matches!(&textures[4].source, TextureSource::Resource(r) if r == "minecraft:dynamic/lightmap_1")
        );
    }

    /// A texture may name one of the game's (`minecraft:textures/environment/clouds.png`): it is
    /// read from the game's resources with its `.mcmeta`. The generated lightmap stays for the
    /// renderer; a missing one, or a location reaching outside the resources, is dropped.
    #[test]
    fn game_textures_come_from_the_game_resources() {
        let mut png = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png, 2, 1);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 128])
                .unwrap();
        }
        let properties = "customTexture.clouds = minecraft:textures/environment/clouds.png\n\
                          texture.composite.gaux2 = minecraft:textures/block/stone.png\n\
                          texture.composite.colortex3 = minecraft:dynamic/lightmap_1\n\
                          customTexture.gone = minecraft:textures/none.png\n\
                          customTexture.escape = minecraft:../../secret.png\n";
        let pack = Pack::from_zip_bytes("P.zip", zip(&[("shaders/final.fsh", "")])).unwrap();
        let (mut textures, notes) =
            load_textures(&pack, &Properties::parse(properties, &Macros::standard()));
        assert!(notes.is_empty(), "{notes:?}");
        let read = |namespace: &str, path: &str| match (namespace, path) {
            ("minecraft", "textures/environment/clouds.png" | "textures/block/stone.png") => {
                Some(png.clone())
            }
            ("minecraft", "textures/block/stone.png.mcmeta") => {
                Some(b"{\"texture\": {\"blur\": true}}".to_vec())
            }
            ("minecraft", p) if p.contains("secret") => Some(png.clone()),
            _ => None,
        };
        let notes = resolve_resources(&mut textures, &read);
        let names: Vec<&str> = textures.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["clouds", "gaux2", "colortex3"], "{notes:?}");
        assert_eq!(notes.len(), 2, "{notes:?}");
        for (t, blur) in [(&textures[0], false), (&textures[1], true)] {
            let TextureSource::Image(image) = &t.source else {
                panic!("{} was not read", t.name);
            };
            assert_eq!(image.size, [2, 1, 1]);
            assert_eq!(
                image.texels,
                Texels::Float(vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 128.0 / 255.0])
            );
            assert_eq!((image.blur, image.clamp), (blur, false), "{}", t.name);
        }
        assert!(
            matches!(&textures[2].source, TextureSource::Resource(r) if r == "minecraft:dynamic/lightmap_1")
        );
    }
}
