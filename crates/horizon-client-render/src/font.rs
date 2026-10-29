//! Fonts (FontManager / FontSet): `font/default.json` and every other font a
//! text component names (`minecraft:alt`, `minecraft:uniform`, a pack's own),
//! each a list of providers tried in order: `space`, `bitmap` (sheets in the
//! shared atlas, or rasterized on demand when a pack's sheet is not in it),
//! `unihex` (GNU Unifont) and `ttf` (TrueTypeGlyphProvider, rasterized with
//! ab_glyph). Glyphs drawn at run time go into a reserved atlas region.
//! Vanilla glyph metrics (ascent, oversampling, advances, bold/shadow
//! offsets), styled drawing and StringSplitter-style line wrapping.
use crate::{
    hud::Rect,
    text::{Span, Style, Text, legacy_format},
    zip::Zip,
};
use ab_glyph::{Font as _, ScaleFont as _};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// `FontDescription.DEFAULT`.
pub const DEFAULT: &str = "minecraft:default";

/// A glyph ready to draw: atlas UVs and its box relative to the pen
/// position (GlyphBitmap left/top/right/bottom), in GUI pixels.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub rect: Option<Rect>,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub advance: f32,
    /// Shadow and bold offset (1 for bitmap and TrueType glyphs, 0.5 for unihex).
    pub offset: f32,
}
/// Pixels to copy into the shared atlas before the next HUD draw.
#[derive(Clone, Debug)]
pub struct Patch {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Copy)]
struct HexGlyph {
    /// Rows left-aligned in 32 bits.
    rows: [u32; 16],
    left: u32,
    right: u32,
}
/// A provider's `filter` (`FontOption.Filter`): the options it needs.
#[derive(Clone, Copy, Default)]
struct Filter {
    uniform: Option<bool>,
    jp: Option<bool>,
}
impl Filter {
    fn of(provider: &serde_json::Value) -> Self {
        let f = &provider["filter"];
        Self {
            uniform: f["uniform"].as_bool(),
            jp: f["jp"].as_bool(),
        }
    }
    fn passes(self, uniform: bool, japanese: bool) -> bool {
        self.uniform.is_none_or(|v| v == uniform) && self.jp.is_none_or(|v| v == japanese)
    }
}
/// A bitmap glyph whose sheet is not in the atlas: the cell to copy.
struct SheetCell {
    sheet: Arc<(u32, Vec<u8>)>,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    glyph: Glyph,
}
enum BitmapGlyph {
    Atlas(Glyph),
    Sheet(SheetCell),
}
/// `TrueTypeGlyphProvider`.
struct TrueType {
    font: ab_glyph::FontVec,
    ids: HashMap<char, ab_glyph::GlyphId>,
    /// FT_Set_Pixel_Sizes: `size * oversample` pixels to the em.
    scale: ab_glyph::PxScale,
    oversample: f32,
    /// FT_Set_Transform's delta, in oversampled pixels (y down).
    shift: (f32, f32),
}
enum Provider {
    Space(HashMap<char, f32>),
    Bitmap(Filter, HashMap<char, BitmapGlyph>),
    /// The definition, and its glyphs once read.
    Unihex(Filter, serde_json::Value, Option<HashMap<u32, HexGlyph>>),
    TrueType(Filter, Box<TrueType>),
}
/// One font (`FontSet`): its providers and the glyphs resolved so far.
struct FontSet {
    providers: Vec<Provider>,
    /// Every char drawn looks up here, so the hasher is Fx, not SipHash.
    glyphs: crate::fast_hash::FxHashMap<char, Glyph>,
    /// `glyphsByWidth` for obfuscated text (built on first use).
    by_width: Option<HashMap<u32, Vec<char>>>,
}
/// The atlas region glyphs are rasterized into, filled in rows.
struct Region {
    cursor: (u32, u32),
    row: u32,
    pending: Vec<Patch>,
}
struct Cache {
    /// Every font by id, the default one first (the common lookup).
    sets: Vec<(String, FontSet)>,
    region: Region,
    /// `Font.random`: picks obfuscated glyphs.
    random: horizon_core::random::LegacyRandom,
}
pub struct Font {
    /// The atlas rects of the sheets under `textures/font`.
    sheets: HashMap<String, Rect>,
    missing: Glyph,
    white: Rect,
    root: PathBuf,
    /// Atlas size and the reserved dynamic region (y, height).
    atlas: (u32, u32),
    region: (u32, u32),
    cache: RefCell<Cache>,
    /// Set anew for each font and each `reset_cache` (see `epoch`).
    generation: std::cell::Cell<u64>,
    /// "Force Unicode Font": the `uniform` filter option.
    pub uniform: bool,
    /// "Japanese Glyph Variants": the `jp` filter option.
    pub japanese: bool,
}
fn parse_hex_digit(b: u8) -> Option<u32> {
    (b as char).to_digit(16)
}
/// UnihexProvider.readFromStream over one `.hex` file.
fn parse_hex(text: &[u8], out: &mut HashMap<u32, HexGlyph>) {
    for line in text.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            continue;
        };
        let (code, data) = (&line[..colon], &line[colon + 1..]);
        if !(4..=6).contains(&code.len()) {
            continue;
        }
        let Some(codepoint) = code
            .iter()
            .try_fold(0u32, |a, b| parse_hex_digit(*b).map(|d| a << 4 | d))
        else {
            continue;
        };
        let digits_per_row = match data.len() {
            32 => 2,
            64 => 4,
            96 => 6,
            128 => 8,
            _ => continue,
        };
        let bit_width = digits_per_row as u32 * 4;
        let mut rows = [0u32; 16];
        let mut mask = 0u32;
        let mut ok = true;
        for (r, row) in rows.iter_mut().enumerate() {
            let chunk = &data[r * digits_per_row..(r + 1) * digits_per_row];
            match chunk
                .iter()
                .try_fold(0u32, |a, b| parse_hex_digit(*b).map(|d| a << 4 | d))
            {
                Some(v) => {
                    *row = v << (32 - bit_width);
                    mask |= *row;
                }
                None => ok = false,
            }
        }
        if !ok {
            continue;
        }
        // LineData.calculateWidth.
        let (left, right) = if mask == 0 {
            (0, bit_width)
        } else {
            (mask.leading_zeros(), 31 - mask.trailing_zeros())
        };
        out.insert(codepoint, HexGlyph { rows, left, right });
    }
}
fn read_json(root: &Path, path: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(root.join(path)).ok()?).ok()
}
fn resource_path(id: &str, folder: &str, ext: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("assets/{ns}/{folder}/{}{ext}", path.trim_end_matches(ext))
}
/// Providers of a font definition with references expanded, in order, with
/// their `filter` objects.
fn providers(root: &Path, id: &str, depth: u32, out: &mut Vec<serde_json::Value>) {
    if depth > 8 {
        return;
    }
    let Some(def) = read_json(root, &resource_path(id, "font", ".json")) else {
        return;
    };
    for p in def["providers"].as_array().into_iter().flatten() {
        if p["type"] == "reference" {
            let mut nested = Vec::new();
            providers(root, p["id"].as_str().unwrap_or(""), depth + 1, &mut nested);
            for mut n in nested {
                // A reference's filter applies to everything it includes.
                if let Some(f) = p["filter"].as_object() {
                    let merged = n["filter"]
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .chain(f.clone())
                        .collect();
                    n["filter"] = serde_json::Value::Object(merged);
                }
                out.push(n);
            }
        } else {
            out.push(p.clone());
        }
    }
}
// FontTexture.add nudges UVs inside the glyph to avoid neighboring texels.
fn glyph_rect(x: u32, y: u32, w: u32, h: u32, atlas: (u32, u32)) -> Rect {
    Rect {
        u0: (x as f32 + 0.01) / atlas.0 as f32,
        v0: (y as f32 + 0.01) / atlas.1 as f32,
        u1: ((x + w) as f32 - 0.01) / atlas.0 as f32,
        v1: ((y + h) as f32 - 0.01) / atlas.1 as f32,
        w,
        h,
    }
}
/// The unscaled pixel width of a bitmap cell (`BitmapProvider.getActualGlyphWidth`).
fn used_width(pixels: &[u8], stride: u32, (cx, cy, cw, ch): (u32, u32, u32, u32)) -> u32 {
    (0..cw)
        .rev()
        .find(|&x| {
            (0..ch).any(|y| {
                pixels
                    .get((((cy + y) * stride + cx + x) * 4 + 3) as usize)
                    .is_some_and(|a| *a != 0)
            })
        })
        .map_or(0, |x| x + 1)
}
/// `BitmapProvider.Definition.load`: each character's glyph, from the atlas
/// when the sheet is in it (`pixels` its RGBA), else from the sheet's own
/// pixels, copied into the atlas when first drawn.
fn load_bitmap(
    root: &Path,
    p: &serde_json::Value,
    sheets: &HashMap<String, Rect>,
    pixels: &[u8],
    atlas: (u32, u32),
) -> HashMap<char, BitmapGlyph> {
    let mut out = HashMap::new();
    let file = p["file"].as_str().unwrap_or("");
    let (ns, name) = file.split_once(':').unwrap_or(("minecraft", file));
    let in_atlas = (ns == "minecraft")
        .then(|| sheets.get(name.trim_end_matches(".png")).copied())
        .flatten();
    // The sheet: its size, and its pixels when they are not in the atlas.
    let (sw, sh, sheet) = match in_atlas {
        Some(rect) => {
            let (w, h) = rect.texels(atlas);
            (w, h, None)
        }
        None => match crate::hud::decode(&root.join(resource_path(file, "textures", ""))) {
            Ok((w, h, rgba)) => (w, h, Some(Arc::new((w, rgba)))),
            Err(_) => {
                log::warn!("font sheet {file} could not be read");
                return out;
            }
        },
    };
    let rows: Vec<Vec<char>> = p["chars"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str().map(|s| s.chars().collect()))
        .collect();
    if rows.is_empty() || rows[0].is_empty() {
        return out;
    }
    let cell_w = sw / rows[0].len() as u32;
    let cell_h = sh / rows.len() as u32;
    let height = p["height"].as_f64().unwrap_or(8.0) as f32;
    let ascent = p["ascent"].as_f64().unwrap_or(7.0) as f32;
    let scale = height / cell_h as f32;
    let origin = in_atlas.map_or((0, 0), |rect| {
        (
            (rect.u0 * atlas.0 as f32).round() as u32,
            (rect.v0 * atlas.1 as f32).round() as u32,
        )
    });
    for (r, row) in rows.iter().enumerate() {
        for (c, ch) in row.iter().enumerate() {
            if *ch == '\0' || out.contains_key(ch) {
                continue;
            }
            let (x, y) = (c as u32 * cell_w, r as u32 * cell_h);
            let used = match &sheet {
                Some(s) => used_width(&s.1, s.0, (x, y, cell_w, cell_h)),
                None => used_width(
                    pixels,
                    atlas.0,
                    (origin.0 + x, origin.1 + y, cell_w, cell_h),
                ),
            };
            let top = 7.0 - ascent;
            let glyph = Glyph {
                rect: None,
                left: 0.0,
                top,
                right: cell_w as f32 * scale,
                bottom: top + cell_h as f32 * scale,
                advance: (0.5 + used as f32 * scale).floor() + 1.0,
                offset: 1.0,
            };
            let glyph = match &sheet {
                None => BitmapGlyph::Atlas(Glyph {
                    rect: Some(glyph_rect(
                        origin.0 + x,
                        origin.1 + y,
                        cell_w,
                        cell_h,
                        atlas,
                    )),
                    ..glyph
                }),
                Some(s) => BitmapGlyph::Sheet(SheetCell {
                    sheet: s.clone(),
                    x,
                    y,
                    w: cell_w,
                    h: cell_h,
                    glyph,
                }),
            };
            out.insert(*ch, glyph);
        }
    }
    out
}
/// `TrueTypeGlyphProviderDefinition.load`: a TrueType file (not CFF) under
/// `font/`, its Unicode characters but the `skip` ones.
fn load_true_type(root: &Path, p: &serde_json::Value) -> Option<TrueType> {
    let file = p["file"].as_str()?;
    let data = std::fs::read(root.join(resource_path(file, "font", "")))
        .map_err(|e| log::warn!("font file {file}: {e}"))
        .ok()?;
    // FT_Get_Font_Format must say TrueType: CFF outlines ("OTTO") are refused.
    if data.starts_with(b"OTTO") {
        log::warn!("font file {file} is not in TTF format");
        return None;
    }
    let font = ab_glyph::FontVec::try_from_vec(data)
        .map_err(|e| log::warn!("font file {file}: {e}"))
        .ok()?;
    let size = p["size"].as_f64().unwrap_or(11.0) as f32;
    let oversample = p["oversample"].as_f64().unwrap_or(1.0) as f32;
    let shift = p["shift"].as_array().map_or((0.0, 0.0), |s| {
        let f = |i: usize| s.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        (f(0), f(1))
    });
    let skip: Vec<char> = match &p["skip"] {
        serde_json::Value::String(s) => s.chars().collect(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str())
            .flat_map(str::chars)
            .collect(),
        _ => Vec::new(),
    };
    let ids = font
        .codepoint_ids()
        .filter(|(id, c)| id.0 != 0 && !skip.contains(c))
        .map(|(id, c)| (c, id))
        .collect();
    let pixels_per_em = (size * oversample).round();
    // ab_glyph scales by the font's height; FreeType by its em.
    let units = font.units_per_em().unwrap_or(1000.0);
    let scale = ab_glyph::PxScale::from(pixels_per_em * font.height_unscaled() / units);
    Some(TrueType {
        font,
        ids,
        scale,
        oversample,
        shift: (shift.0 * oversample, shift.1 * oversample),
    })
}
impl TrueType {
    /// The advance in GUI pixels: hinted FreeType advances are whole
    /// oversampled pixels.
    fn advance(&self, id: ab_glyph::GlyphId) -> f32 {
        self.font.as_scaled(self.scale).h_advance(id).round() / self.oversample
    }
    /// `loadGlyph` and its bitmap: the grey coverage as `rrrr` texels
    /// (`text_intensity`), below 0.1 cut (its discard).
    fn glyph(&self, id: ab_glyph::GlyphId) -> (Glyph, Option<(u32, u32, Vec<u8>)>) {
        let advance = self.advance(id);
        let glyph =
            id.with_scale_and_position(self.scale, ab_glyph::point(self.shift.0, self.shift.1));
        let empty = Glyph {
            rect: None,
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            advance,
            offset: 1.0,
        };
        let Some(outline) = self.font.outline_glyph(glyph) else {
            return (empty, None);
        };
        let bounds = outline.px_bounds();
        let (w, h) = (bounds.width() as u32, bounds.height() as u32);
        if w == 0 || h == 0 {
            return (empty, None);
        }
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        outline.draw(|x, y, coverage| {
            let v = if coverage < 0.1 {
                0
            } else {
                (coverage.min(1.0) * 255.0).round() as u8
            };
            let i = ((y * w + x) * 4) as usize;
            if let Some(px) = rgba.get_mut(i..i + 4) {
                px.copy_from_slice(&[v; 4]);
            }
        });
        let os = self.oversample;
        // GlyphBitmap: bitmap_left / bitmap_top over the oversampling, and
        // top = 7 - bearingTop.
        let (left, top) = (bounds.min.x / os, 7.0 + bounds.min.y / os);
        (
            Glyph {
                rect: None,
                left,
                top,
                right: left + w as f32 / os,
                bottom: top + h as f32 / os,
                advance,
                offset: 1.0,
            },
            Some((w, h, rgba)),
        )
    }
}
fn load_hex(root: &Path, p: &serde_json::Value) -> HashMap<u32, HexGlyph> {
    let mut glyphs = HashMap::new();
    let Some(file) = p["hex_file"].as_str() else {
        return glyphs;
    };
    let path = root.join(resource_path(file, "", "").replace("//", "/"));
    let Ok(zip) = Zip::open(&path) else {
        return glyphs;
    };
    let names: Vec<String> = zip
        .names()
        .filter(|n| n.ends_with(".hex"))
        .map(str::to_owned)
        .collect();
    for name in names {
        if let Ok(bytes) = zip.read(&name) {
            parse_hex(&bytes, &mut glyphs);
        }
    }
    for o in p["size_overrides"].as_array().into_iter().flatten() {
        let bound = |k: &str| {
            o[k].as_str()
                .and_then(|s| s.chars().next())
                .map_or(0, u32::from)
        };
        let (left, right) = (
            o["left"].as_u64().unwrap_or(0) as u32,
            o["right"].as_u64().unwrap_or(15) as u32,
        );
        for c in bound("from")..=bound("to") {
            if let Some(g) = glyphs.get_mut(&c) {
                g.left = left;
                g.right = right;
            }
        }
    }
    glyphs
}
/// The ids of every font definition (`font/**/*.json` of each namespace).
fn font_ids(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}/"), out);
            } else if let Some(stem) = name.strip_suffix(".json") {
                out.push(format!("{prefix}{stem}"));
            }
        }
    }
    let mut ids = Vec::new();
    for ns in std::fs::read_dir(root.join("assets"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let ns = ns.file_name().to_string_lossy().into_owned();
        walk(
            &root.join("assets").join(&ns).join("font"),
            &format!("{ns}:"),
            &mut ids,
        );
    }
    ids.sort();
    ids
}
impl Region {
    /// Place `w x h` pixels in the region (rows left to right); their atlas
    /// position, or none when it is full.
    fn place(
        &mut self,
        w: u32,
        h: u32,
        rgba: Vec<u8>,
        atlas: (u32, u32),
        region: (u32, u32),
    ) -> Option<(u32, u32)> {
        let (mut x, mut y) = self.cursor;
        if x + w + 1 > atlas.0 {
            x = 0;
            y += self.row + 1;
            self.row = 0;
        }
        if y + h > region.1 {
            return None;
        }
        let (ax, ay) = (x, region.0 + y);
        self.pending.push(Patch {
            x: ax,
            y: ay,
            width: w,
            height: h,
            rgba,
        });
        self.cursor = (x + w + 1, y);
        self.row = self.row.max(h);
        Some((ax, ay))
    }
}
/// A value no font has had yet (`Font::epoch`).
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
impl Font {
    /// Build from the atlas holding the bitmap font sheets (`sprites`, of
    /// `atlas` size) with `region` the reserved rows.
    pub fn load(
        root: &Path,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
        atlas: (u32, u32),
        region: (u32, u32),
        white: Rect,
    ) -> Self {
        let sheets = sprites
            .iter()
            .filter(|(k, _)| k.starts_with("font/"))
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        let font = Self {
            sheets,
            missing: Glyph {
                rect: None,
                left: 0.0,
                top: 0.0,
                right: 5.0,
                bottom: 8.0,
                advance: 6.0,
                offset: 1.0,
            },
            white,
            root: root.to_owned(),
            atlas,
            region,
            cache: RefCell::new(Cache {
                sets: Vec::new(),
                region: Region {
                    cursor: (0, 0),
                    row: 0,
                    pending: Vec::new(),
                },
                random: horizon_core::random::LegacyRandom::new(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_nanos() as i64),
                ),
            }),
            generation: std::cell::Cell::new(next_generation()),
            uniform: false,
            japanese: false,
        };
        // FontManager.prepare: every font definition of the resources.
        let mut ids = font_ids(root);
        ids.sort_by_key(|id| id != DEFAULT);
        let sets = ids
            .into_iter()
            .map(|id| {
                let set = font.load_set(&id, pixels);
                (id, set)
            })
            .collect();
        font.cache.borrow_mut().sets = sets;
        font
    }
    /// `FontManager`: a font's providers with references expanded.
    fn load_set(&self, id: &str, pixels: &[u8]) -> FontSet {
        let mut all = Vec::new();
        providers(&self.root, id, 0, &mut all);
        let providers = all
            .iter()
            .filter_map(|p| match p["type"].as_str()? {
                "space" => Some(Provider::Space(
                    p["advances"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter_map(|(k, v)| Some((k.chars().next()?, v.as_f64()? as f32)))
                        .collect(),
                )),
                "bitmap" => Some(Provider::Bitmap(
                    Filter::of(p),
                    load_bitmap(&self.root, p, &self.sheets, pixels, self.atlas),
                )),
                "unihex" => Some(Provider::Unihex(Filter::of(p), p.clone(), None)),
                "ttf" => Some(Provider::TrueType(
                    Filter::of(p),
                    Box::new(load_true_type(&self.root, p)?),
                )),
                _ => None,
            })
            .collect();
        FontSet {
            providers,
            glyphs: Default::default(),
            by_width: None,
        }
    }
    /// Forget rasterized glyphs (after changing font options).
    pub fn reset_cache(&self) {
        let mut cache = self.cache.borrow_mut();
        for (_, set) in &mut cache.sets {
            set.glyphs.clear();
            set.by_width = None;
        }
        cache.region.cursor = (0, 0);
        cache.region.row = 0;
        self.generation.set(next_generation());
    }
    /// Changes whenever a glyph may come out different: another font, a
    /// cache reset (glyphs move in the atlas) or other font options. Text laid
    /// out under one epoch can be kept until it changes.
    pub fn epoch(&self) -> u64 {
        self.generation.get() << 2 | u64::from(self.uniform) << 1 | u64::from(self.japanese)
    }
    /// The glyph for a character in the default font (FontSet.getGlyph).
    pub fn glyph(&self, c: char) -> Glyph {
        self.glyph_of(DEFAULT, c)
    }
    /// The glyph for a character drawn in `style`'s font (its id is
    /// already complete, see `text::font_id`).
    pub fn glyph_styled(&self, c: char, style: &Style) -> Glyph {
        self.glyph_of(style.font.as_deref().unwrap_or(DEFAULT), c)
    }
    /// The glyph for a character in the font `id` (`alt` is `minecraft:alt`).
    pub fn glyph_in(&self, id: &str, c: char) -> Glyph {
        if id.contains(':') {
            self.glyph_of(id, c)
        } else {
            self.glyph_of(&format!("minecraft:{id}"), c)
        }
    }
    /// The first active provider's glyph, else the missing glyph.
    fn glyph_of(&self, id: &str, c: char) -> Glyph {
        let mut cache = self.cache.borrow_mut();
        let Cache { sets, region, .. } = &mut *cache;
        // FontManager.getFontSetRaw: a font that doesn't exist draws every
        // character as the missing glyph.
        let Some((_, set)) = sets.iter_mut().find(|(k, _)| k == id) else {
            return self.missing;
        };
        if let Some(g) = set.glyphs.get(&c) {
            return *g;
        }
        let glyph = self.resolve(set, region, c).unwrap_or(self.missing);
        set.glyphs.insert(c, glyph);
        glyph
    }
    fn resolve(&self, set: &mut FontSet, region: &mut Region, c: char) -> Option<Glyph> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        for provider in &mut set.providers {
            match provider {
                Provider::Space(advances) => {
                    if let Some(a) = advances.get(&c) {
                        return Some(Glyph {
                            rect: None,
                            left: 0.0,
                            top: 0.0,
                            right: 0.0,
                            bottom: 0.0,
                            advance: *a,
                            offset: 1.0,
                        });
                    }
                }
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    match glyphs.get(&c) {
                        Some(BitmapGlyph::Atlas(g)) => return Some(*g),
                        Some(BitmapGlyph::Sheet(cell)) => {
                            let (stride, pixels) = (&cell.sheet.0, &cell.sheet.1);
                            let mut rgba = Vec::with_capacity((cell.w * cell.h * 4) as usize);
                            for y in cell.y..cell.y + cell.h {
                                let at = ((y * stride + cell.x) * 4) as usize;
                                rgba.extend_from_slice(&pixels[at..at + (cell.w * 4) as usize]);
                            }
                            let (x, y) =
                                region.place(cell.w, cell.h, rgba, self.atlas, self.region)?;
                            return Some(Glyph {
                                rect: Some(glyph_rect(x, y, cell.w, cell.h, self.atlas)),
                                ..cell.glyph
                            });
                        }
                        None => {}
                    }
                }
                Provider::Unihex(filter, def, glyphs) if filter.passes(uniform, japanese) => {
                    let glyphs = glyphs.get_or_insert_with(|| load_hex(&self.root, def));
                    if let Some(h) = glyphs.get(&u32::from(c)).copied() {
                        return self.rasterize_hex(region, h);
                    }
                }
                Provider::TrueType(filter, ttf) if filter.passes(uniform, japanese) => {
                    if let Some(id) = ttf.ids.get(&c) {
                        let (glyph, bitmap) = ttf.glyph(*id);
                        return match bitmap {
                            Some((w, h, rgba)) => {
                                let (x, y) = region.place(w, h, rgba, self.atlas, self.region)?;
                                Some(Glyph {
                                    rect: Some(glyph_rect(x, y, w, h, self.atlas)),
                                    ..glyph
                                })
                            }
                            None => Some(glyph),
                        };
                    }
                }
                _ => {}
            }
        }
        None
    }
    fn rasterize_hex(&self, region: &mut Region, h: HexGlyph) -> Option<Glyph> {
        let width = h.right.saturating_sub(h.left) + 1;
        let mut rgba = Vec::with_capacity((width * 16 * 4) as usize);
        for row in h.rows {
            for bit in h.left..=h.right {
                let set = bit < 32 && (row >> (31 - bit)) & 1 != 0;
                rgba.extend(if set { [255; 4] } else { [0; 4] });
            }
        }
        let (x, y) = region.place(width, 16, rgba, self.atlas, self.region)?;
        Some(Glyph {
            rect: Some(glyph_rect(x, y, width, 16, self.atlas)),
            left: 0.0,
            top: 0.0,
            right: width as f32 / 2.0,
            bottom: 8.0,
            advance: (width / 2 + 1) as f32,
            offset: 0.5,
        })
    }
    /// `FontSet.getRandomGlyph`: what an obfuscated character draws, a random
    /// glyph of the font as wide as `advance` (rounded up), else the missing
    /// glyph.
    ///
    /// Vanilla picks among every glyph of the font set, Unifont's included.
    /// Those are rasterized into a fixed atlas region on demand, which a
    /// line of obfuscated text would fill in seconds, so the candidates are
    /// the glyphs already in the atlas (the bitmap sheets there and spaces),
    /// or for a font without any (Force Unicode Font, a TrueType font) its
    /// glyphs below U+0250.
    pub fn random_glyph(&self, style: &Style, advance: f32) -> Glyph {
        use horizon_core::random::RandomSource;
        let id = style.font.as_deref().unwrap_or(DEFAULT);
        let width = advance.ceil() as u32;
        let pick = {
            let mut cache = self.cache.borrow_mut();
            let Cache { sets, random, .. } = &mut *cache;
            let Some((_, set)) = sets.iter_mut().find(|(k, _)| k == id) else {
                return self.missing;
            };
            if set.by_width.is_none() {
                set.by_width = Some(self.glyphs_by_width(set));
            }
            set.by_width
                .as_ref()
                .and_then(|m| m.get(&width))
                .and_then(|chars| {
                    let i = random.next_int_bound(chars.len() as i32).ok()?;
                    chars.get(i as usize).copied()
                })
        };
        pick.map_or(self.missing, |c| self.glyph_of(id, c))
    }
    fn glyphs_by_width(&self, set: &mut FontSet) -> HashMap<u32, Vec<char>> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        let mut chars = std::collections::BTreeSet::new();
        let mut in_atlas = false;
        for provider in &set.providers {
            match provider {
                Provider::Space(advances) => chars.extend(advances.keys().copied()),
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    for (c, g) in glyphs {
                        if matches!(g, BitmapGlyph::Atlas(_)) {
                            in_atlas = true;
                            chars.insert(*c);
                        }
                    }
                }
                _ => {}
            }
        }
        if !in_atlas {
            // Nothing in the atlas: the glyphs below U+0250, rasterized as used.
            chars.extend((0..0x250u32).filter_map(char::from_u32));
        }
        let mut out: HashMap<u32, Vec<char>> = HashMap::new();
        for c in chars {
            if let Some(advance) = self.provider_advance(set, c) {
                out.entry(advance.ceil() as u32).or_default().push(c);
            }
        }
        out
    }
    /// A character's advance in `set` (its first provider that has it),
    /// without rasterizing it.
    fn provider_advance(&self, set: &mut FontSet, c: char) -> Option<f32> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        for provider in &mut set.providers {
            match provider {
                Provider::Space(advances) => {
                    if let Some(a) = advances.get(&c) {
                        return Some(*a);
                    }
                }
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    match glyphs.get(&c) {
                        Some(BitmapGlyph::Atlas(g)) => return Some(g.advance),
                        Some(BitmapGlyph::Sheet(cell)) => return Some(cell.glyph.advance),
                        None => {}
                    }
                }
                Provider::Unihex(filter, def, glyphs) if filter.passes(uniform, japanese) => {
                    let glyphs = glyphs.get_or_insert_with(|| load_hex(&self.root, def));
                    if let Some(h) = glyphs.get(&u32::from(c)) {
                        let width = h.right.saturating_sub(h.left) + 1;
                        return Some((width / 2 + 1) as f32);
                    }
                }
                Provider::TrueType(filter, ttf) if filter.passes(uniform, japanese) => {
                    if let Some(id) = ttf.ids.get(&c) {
                        return Some(ttf.advance(*id));
                    }
                }
                _ => {}
            }
        }
        None
    }
    /// Atlas pixels rasterized since the last call.
    pub fn take_patches(&self) -> Vec<Patch> {
        std::mem::take(&mut self.cache.borrow_mut().region.pending)
    }
    pub fn advance(&self, c: char, style: &Style) -> f32 {
        let g = self.glyph_styled(c, style);
        g.advance + if style.is_bold() { g.offset } else { 0.0 }
    }
    /// Font.width of plain text (legacy codes are formatting, not glyphs).
    pub fn width(&self, s: &str) -> f32 {
        if s.contains('§') {
            return self.width_text(&Text::legacy(s));
        }
        crate::bidi::visual_str(s)
            .chars()
            .map(|c| self.glyph(c).advance)
            .sum::<f32>()
            .ceil()
    }
    /// Font.width of a component: `§` codes in the spans are formatting,
    /// not glyphs. Right-to-left text is measured as it is drawn, shaped.
    pub fn width_text(&self, t: &Text) -> f32 {
        let t = t.resolved();
        crate::bidi::visual(&t)
            .chars()
            .map(|(c, s)| self.advance(c, s))
            .sum::<f32>()
            .ceil()
    }
    /// `StringSplitter.splitLines`: each line and whether it continues a
    /// wrapped one. A line breaks at a `\n`, or once a character overflows
    /// `max_width` (never the first visible one) at the last space so far, or
    /// before that character; the `\n` or space at the break is dropped.
    pub fn split_lines(&self, t: &Text, max_width: f32) -> Vec<(Text, bool)> {
        let max_width = max_width.max(1.0);
        let chars: Vec<(char, &Style)> = t.chars().collect();
        let n = chars.len();
        let mut out = Vec::new();
        let (mut start, mut wrapped, mut force_new_line) = (0, false, false);
        loop {
            // LineBreakFinder over what is left.
            let (mut width, mut had_visible, mut last_space) = (0.0f32, false, None);
            let mut split = None;
            for (i, &(c, style)) in chars.iter().enumerate().skip(start) {
                if c == '\n' {
                    split = Some(i);
                    break;
                }
                if c == ' ' {
                    last_space = Some(i);
                }
                let advance = self.advance(c, style);
                width += advance;
                if !had_visible || width <= max_width {
                    had_visible |= advance != 0.0;
                } else {
                    split = Some(last_space.unwrap_or(i));
                    break;
                }
            }
            let Some(at) = split else { break };
            let new_line = chars[at].0 == '\n';
            let skip = usize::from(new_line || chars[at].0 == ' ');
            force_new_line = new_line;
            out.push((t.slice(start, at), wrapped));
            wrapped = !new_line;
            start = at + skip;
        }
        if start < n {
            out.push((t.slice(start, n), wrapped));
        } else if force_new_line {
            out.push((Text::empty(), false));
        }
        out
    }
    /// `Font.split`: `StringSplitter.splitLines(text, width, Style.EMPTY)` over
    /// the text's parts with their `§` codes in. A code styles what follows
    /// (`§r` back to no style while measuring), and the part a line breaks in
    /// goes on in the style at the break. Lines keep their codes for drawing.
    pub fn split_formatted(&self, t: &Text, max_width: f32) -> Vec<Text> {
        let max_width = max_width.max(1.0);
        let mut parts: Vec<(Vec<char>, Style)> = t
            .spans
            .iter()
            .filter(|s| !s.text.is_empty())
            .map(|s| (s.text.chars().collect(), s.style.clone()))
            .collect();
        let mut flat: Vec<char> = parts.iter().flat_map(|p| p.0.iter().copied()).collect();
        let mut out = Vec::new();
        let mut force_new_line = false;
        // One LineBreakFinder per line, over the parts left.
        while let Some((at, split_style)) = self.line_break(&parts, max_width) {
            let tail = flat[at];
            let new_line = tail == '\n';
            let skip = usize::from(new_line || tail == ' ');
            force_new_line = new_line;
            out.push(split_at(&mut parts, at, skip, &split_style));
            flat.drain(..at + skip);
        }
        if !parts.is_empty() {
            let mut rest = Text::default();
            rest.spans = parts
                .into_iter()
                .map(|(text, style)| Span {
                    text: text.into_iter().collect(),
                    style,
                })
                .collect();
            out.push(rest);
        } else if force_new_line {
            out.push(Text::empty());
        }
        out
    }
    /// `StringSplitter.LineBreakFinder` fed by `StringDecomposer.iterateFormatted`:
    /// where the parts break (a character index) and the style there.
    fn line_break(&self, parts: &[(Vec<char>, Style)], max_width: f32) -> Option<(usize, Style)> {
        let (mut width, mut had_visible, mut last_space) = (0.0f32, false, None);
        let mut offset = 0;
        for (contents, part_style) in parts {
            let mut style = part_style.clone();
            let mut i = 0;
            while i < contents.len() {
                let c = contents[i];
                if c == '§' {
                    // A `§` that ends its part ends only that part.
                    let Some(&code) = contents.get(i + 1) else {
                        break;
                    };
                    style = legacy_format(&style, code, &Style::default());
                    i += 2;
                    continue;
                }
                let at = offset + i;
                if c == '\n' {
                    return Some((at, style));
                }
                if c == ' ' {
                    last_space = Some((at, style.clone()));
                }
                let advance = self.advance(c, &style);
                width += advance;
                if had_visible && width > max_width {
                    return Some(last_space.unwrap_or((at, style)));
                }
                had_visible |= advance != 0.0;
                i += 1;
            }
            offset += contents.len();
        }
        None
    }
    /// `ComponentRenderUtils.wrapComponents`: the split lines, wrapped ones
    /// indented by an unstyled space; one empty line for empty text.
    pub fn wrap_components(&self, t: &Text, max_width: i32) -> Vec<Text> {
        let lines: Vec<Text> = self
            .split_lines(t, max_width as f32)
            .into_iter()
            .map(|(line, wrapped)| {
                if wrapped {
                    Text::literal(" ").append(line)
                } else {
                    line
                }
            })
            .collect();
        if lines.is_empty() {
            vec![Text::empty()]
        } else {
            lines
        }
    }
    /// StringSplitter.splitLines: wrap at the last space that fits (or
    /// mid-word when none does); `\n` always breaks. Styles are kept.
    pub fn split(&self, t: &Text, max_width: f32) -> Vec<Text> {
        let chars: Vec<(char, Style)> = t.chars().map(|(c, s)| (c, s.clone())).collect();
        let mut lines = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let mut width = 0.0;
            let mut end = chars.len();
            let mut next = chars.len();
            let mut last_space = None;
            for (i, (c, style)) in chars.iter().enumerate().skip(start) {
                if *c == '\n' {
                    end = i;
                    next = i + 1;
                    break;
                }
                if *c == ' ' {
                    last_space = Some(i);
                }
                width += self.advance(*c, style);
                if width > max_width {
                    match last_space {
                        Some(s) if s > start => {
                            end = s;
                            next = s + 1;
                        }
                        _ => {
                            end = i.max(start + 1);
                            next = end;
                        }
                    }
                    break;
                }
            }
            lines.push(t.slice(start, end));
            start = next;
            if start == chars.len() && chars.last().is_some_and(|c| c.0 == '\n') {
                lines.push(Text::empty());
            }
        }
        if lines.is_empty() {
            lines.push(Text::empty());
        }
        lines
    }
    /// Characters of `s` that fit in `width` (Font.plainSubstrByWidth).
    pub fn fit(&self, s: &str, width: f32) -> usize {
        let mut w = 0.0;
        for (i, c) in s.chars().enumerate() {
            w += self.glyph(c).advance;
            if w > width {
                return i;
            }
        }
        s.chars().count()
    }
    /// Characters of `s` that fit in `width`, measured from the end.
    pub fn fit_reverse(&self, s: &str, width: f32) -> usize {
        let mut w = 0.0;
        for (i, c) in s.chars().rev().enumerate() {
            w += self.glyph(c).advance;
            if w > width {
                return i;
            }
        }
        s.chars().count()
    }
    pub fn white(&self) -> Rect {
        self.white
    }
}

/// `StringSplitter.FlatComponents.splitAt`: the parts before `at` as a line,
/// `skip` characters dropped, and the part it ends in restyled as `style`.
fn split_at(parts: &mut Vec<(Vec<char>, Style)>, at: usize, skip: usize, style: &Style) -> Text {
    let mut line = Text::default();
    let mut push = |text: &[char], style: &Style| {
        line.spans.push(Span {
            text: text.iter().collect(),
            style: style.clone(),
        })
    };
    // Each part done is removed, so the one at hand is always the first.
    let (mut position, mut in_skip) = (at, false);
    while let Some((text, part_style)) = parts.first() {
        let size = text.len();
        if !in_skip {
            if position > size {
                push(text, part_style);
                parts.remove(0);
                position -= size;
                continue;
            }
            if position > 0 {
                push(&text[..position], part_style);
            }
            position += skip;
            in_skip = true;
        }
        if position <= size {
            if position == size {
                parts.remove(0);
            } else {
                parts[0] = (text[position..].to_vec(), style.clone());
            }
            break;
        }
        parts.remove(0);
        position -= size;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unihex_rows_and_widths() {
        let mut out = HashMap::new();
        // 'A' from unifont (8 wide) and a 16-wide glyph with an empty left column.
        let wide = format!("4E00:{}7FFE{}\n", "0000".repeat(4), "0000".repeat(11));
        parse_hex(b"0041:0000000018242442427E424242420000\n", &mut out);
        parse_hex(wide.as_bytes(), &mut out);
        let a = out[&0x41];
        assert_eq!((a.left, a.right), (1, 6));
        assert_eq!(a.rows[4], 0x18 << 24);
        let yi = out[&0x4E00];
        assert_eq!((yi.left, yi.right), (1, 14));
        // Unparseable lines are skipped.
        parse_hex(b"zz:00\n", &mut out);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn glyph_uvs_have_the_native_texel_inset() {
        for (x, y, w, h) in [(120, 48, 8, 8), (0, 0, 1, 16), (4000, 2000, 32, 16)] {
            let r = glyph_rect(x, y, w, h, (4096, 4096));
            assert_eq!((r.w, r.h), (w, h));
            assert!(r.u0 > x as f32 / 4096.0 && r.v0 > y as f32 / 4096.0);
            assert!(r.u1 < (x + w) as f32 / 4096.0 && r.v1 < (y + h) as f32 / 4096.0);
            assert_eq!(r.u0, (x as f32 + 0.01) / 4096.0);
            assert_eq!(r.v1, ((y + h) as f32 - 0.01) / 4096.0);
        }
    }

    /// A resource root with these font definitions and files.
    fn root(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("horizon-font-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, bytes) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        root
    }
    fn white() -> Rect {
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.0,
            v1: 0.0,
            w: 1,
            h: 1,
        }
    }
    fn load(root: &Path) -> Font {
        Font::load(
            root,
            &HashMap::new(),
            &[],
            (4096, 1024),
            (512, 512),
            white(),
        )
    }
    fn png(width: u32, height: u32, opaque: impl Fn(u32, u32) -> bool) -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.extend(if opaque(x, y) { [255; 4] } else { [0; 4] });
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        out
    }

    #[test]
    fn true_type_glyphs_are_rasterized_at_the_provider_size() {
        let def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2,"shift":[0.5,1],"skip":["B"]}]}"#;
        let root = root(
            "ttf",
            &[
                ("assets/minecraft/font/default.json", def),
                (
                    "assets/minecraft/font/mono.ttf",
                    horizon_theme::fonts::GEIST_MONO,
                ),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let a = font.glyph('A');
        let rect = a.rect.expect("A is drawn");
        // Oversampled 2x: the bitmap is twice the glyph's GUI size.
        assert_eq!(rect.w as f32 / 2.0, a.right - a.left);
        assert_eq!(rect.h as f32 / 2.0, a.bottom - a.top);
        // A monospaced font: every advance is the same, in half pixels.
        assert_eq!(a.advance, font.glyph('i').advance);
        assert_eq!((a.advance * 2.0).fract(), 0.0);
        assert!(a.advance > 4.0 && a.advance < 9.0, "{}", a.advance);
        // Capitals sit on the baseline at y = 7, shifted down a pixel.
        assert!((a.bottom - 8.0).abs() <= 0.5, "{}", a.bottom);
        assert_eq!(a.offset, 1.0);
        // The space has an advance and nothing to draw.
        let space = font.glyph(' ');
        assert!(space.rect.is_none() && space.advance > 0.0);
        // Skipped characters fall through to the missing glyph.
        assert_eq!(font.glyph('B').advance, 6.0);
        // Coverage as `rrrr` texels, faint coverage cut.
        let patches = font.take_patches();
        assert_eq!(patches.len(), 2);
        let texels: Vec<&[u8]> = patches.iter().flat_map(|p| p.rgba.chunks(4)).collect();
        assert!(texels.iter().all(|t| t.iter().all(|v| *v == t[0])));
        assert!(texels.iter().all(|t| t[0] == 0 || t[0] >= 25));
        assert!(texels.iter().any(|t| t[0] == 255));
    }

    #[test]
    fn styles_pick_their_font_and_unknown_fonts_draw_missing_glyphs() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let alt = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ba"]}]}"#;
        let root = root(
            "styles",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/font/alt.json", alt),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        // The sheet is not in this atlas: glyphs are copied in when first drawn.
        assert_eq!(font.glyph('a').advance, 3.0);
        assert_eq!(font.glyph('b').advance, 7.0);
        let alt = Style {
            font: Some("minecraft:alt".into()),
            ..Style::default()
        };
        assert_eq!(font.glyph_styled('a', &alt).advance, 7.0);
        assert_eq!(font.glyph_in("alt", 'b').advance, 3.0);
        assert!(font.glyph_styled('a', &alt).rect.is_some());
        // Not in alt (no space provider there), or no such font at all.
        assert_eq!(font.glyph_styled(' ', &alt).advance, 6.0);
        let missing = Style {
            font: Some("minecraft:nope".into()),
            ..Style::default()
        };
        assert_eq!(font.glyph_styled(' ', &missing).advance, 6.0);
        assert_eq!(font.advance('a', &alt), 7.0);
        let text = Text::literal("ab").with_style(&alt);
        assert_eq!(font.width_text(&text), 10.0);
        assert_eq!(font.random_glyph(&alt, 7.0).advance, 7.0);
        assert!(!font.take_patches().is_empty());
    }

    /// StringDecomposer.iterateFormatted: a `§` that ends a part stops that
    /// part only; the next part is still measured and can break.
    #[test]
    fn a_section_sign_ending_a_part_still_breaks_the_next() {
        let sheet = png(16, 8, |x, _| x < 2);
        let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "section-sign",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let mut text = Text::default();
        text.spans = ["a§", "a a"]
            .map(|part| Span {
                text: part.into(),
                style: Style::default(),
            })
            .into();
        // 3 + 3 + 4 + 3 is past 12: the line breaks at the space.
        let lines: Vec<String> = font
            .split_formatted(&text, 12.0)
            .iter()
            .map(Text::plain)
            .collect();
        assert_eq!(lines, ["a§a", "a"]);
    }

    /// A renderer rebuilt over the same font (the loading screen's handoff)
    /// makes its atlas texture from the textures alone, so the glyphs it had
    /// rasterized into the previous texture must be placed into the new one.
    #[test]
    fn a_rebuilt_atlas_gets_its_glyphs_again() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let default = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "rebuild",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let first = font.glyph('a');
        assert_eq!(font.take_patches().len(), 1);
        font.reset_cache();
        // Nothing is left over: drawing the character again places its pixels.
        assert!(font.take_patches().is_empty());
        assert_eq!(font.glyph('a').advance, first.advance);
        assert_eq!(font.take_patches().len(), 1);
    }

    #[test]
    fn provider_filters_follow_the_font_options() {
        let sheet = png(8, 8, |x, _| x < 4);
        let default = br#"{"providers":[{"type":"reference","id":"minecraft:include/sheet","filter":{"uniform":false}},{"type":"space","advances":{"a":2}}]}"#;
        let sheet_def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/s.png","ascent":7,"chars":["a"]}]}"#;
        let root = root(
            "filters",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/font/include/sheet.json", sheet_def),
                ("assets/minecraft/textures/font/s.png", &sheet),
            ],
        );
        let mut font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(font.glyph('a').advance, 5.0);
        font.uniform = true;
        font.reset_cache();
        assert_eq!(font.glyph('a').advance, 2.0);
    }
}
