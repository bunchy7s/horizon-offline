//! 2D overlay: one atlas holding GUI sprites, the bitmap font, item and block
//! textures, and a batch of textured quads in GUI pixels (vanilla's automatic
//! GUI scale is applied when drawing).
use crate::Result;
use horizon_assets::animation::AnimationMetadata;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use wgpu::util::DeviceExt;

/// A sprite's atlas region, and its size in GUI pixels: the size of the
/// default resources' texture of that name, whatever the resolution of the
/// pack that replaced it (vanilla's GUI code draws in those units: `blit`
/// divides by the texture size it is given, sprites by their `gui.scaling`
/// size). Textures the default resources lack keep their own size.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub w: u32,
    pub h: u32,
}
impl Rect {
    /// The region's size in texels of an atlas this large.
    pub fn texels(&self, (width, height): (u32, u32)) -> (u32, u32) {
        (
            ((self.u1 - self.u0) * width as f32).round() as u32,
            ((self.v1 - self.v0) * height as f32).round() as u32,
        )
    }
}
/// `TextureMetadataSection`'s `blur` and `clamp`: how vanilla samples a
/// standalone texture (`ReloadableTexture.setSampler`: linear rather than
/// nearest filtering, clamp-to-edge rather than repeat addressing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureFlags {
    pub blur: bool,
    pub clamp: bool,
}
impl TextureFlags {
    /// The `texture` section of a `.png.mcmeta` file, if it sets either flag.
    pub fn from_mcmeta(json: &str) -> Option<Self> {
        let meta: serde_json::Value = serde_json::from_str(json).ok()?;
        let section = meta.get("texture")?;
        let flag = |key| section.get(key).and_then(serde_json::Value::as_bool) == Some(true);
        let flags = Self {
            blur: flag("blur"),
            clamp: flag("clamp"),
        };
        (flags != Self::default()).then_some(flags)
    }
}
pub struct HudAtlas {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub sprites: HashMap<String, Rect>,
    /// Sprites with a 0 < alpha < 255 texel (`SpriteContents.transparency().hasTranslucent`),
    /// which puts their particles on vanilla's translucent layer.
    pub translucent: HashSet<String>,
    /// Sprites whose `.png.mcmeta` sets `blur` or `clamp`; the others
    /// sample as vanilla's defaults (nearest, repeat).
    pub flags: HashMap<String, TextureFlags>,
    /// Every animated block or item sprite the atlas packs: its atlas rect
    /// and the mcmeta-driven frame list, re-uploaded into the rect as the
    /// frames turn (vanilla's stitched atlas animates all of these).
    pub sprite_animations: Vec<(Rect, crate::sprites::SpriteAnimation)>,
    /// The default font (bitmap sheets in this atlas, unifont rasterized on demand).
    pub font: crate::font::Font,
    /// `misc/enchanted_glint_item` as its own image: vanilla's `GlintSampler`
    /// repeats it per pixel, which one sprite inside this atlas cannot.
    pub glint: (Vec<u8>, u32, u32),
    /// `misc/enchanted_glint_armor`, the armour layers' `GlintSampler`.
    pub glint_armor: (Vec<u8>, u32, u32),
    white: Rect,
}
/// Atlas rows reserved for glyphs rasterized at run time.
pub const GLYPH_ROWS: u32 = 512;
/// The largest atlas side: the 2D texture limit of the GPUs Horizon runs on
/// (Apple silicon, RADV and Intel allow 16384; NVIDIA 32768).
pub const MAX_ATLAS_SIZE: u32 = 16384;
/// 128x128 atlas slots for map textures. A lobby paints its signs as walls of
/// framed maps, so the pool holds a wall; too few slots re-upload every map
/// every frame. 26.3's `MapTextureManager` keeps a texture per map instead.
pub const MAP_SLOTS: usize = 256;
/// The atlas slot the minimap extra draws into.
pub const MINIMAP_SPRITE: &str = "horizon/minimap";
/// Atlas slots for downloaded player skins (`skins/<sha1>`), 64x64 each.
/// Vanilla registers a texture per skin. Here a skin evicted from its slot
/// is not stored again, and its player falls back to the default skin, so
/// the slots must cover a busy server's player list.
pub const SKIN_SLOTS: usize = 128;
/// Reused by the visible rows of the classic world selector.
pub const WORLD_ICON_SLOTS: usize = 32;
/// Atlas slots for the pack icons on the pack screen, one per icon drawn
/// (`PackSelectionScreen.loadPackIcon` registers a texture per pack): two lists
/// of 36-pixel rows at GUI scale 2 on a 4K screen still fit.
pub const PACK_ICON_SLOTS: usize = 64;
/// A pack icon slot's side: vanilla's own icons are 128x128.
pub const PACK_ICON_SIZE: u32 = 128;
/// Atlas slots for armour trim textures made at run time (`trims`), each the
/// size of the largest trim texture.
pub const TRIM_SLOTS: usize = 32;
pub fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    decode_bytes(&fs::read(path)?)
}
/// A PNG in memory as RGBA pixels.
pub fn decode_bytes(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut bytes = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
    for p in bytes[..info.buffer_size()].chunks_exact(channels) {
        rgba.extend(match channels {
            4 => [p[0], p[1], p[2], p[3]],
            3 => [p[0], p[1], p[2], 255],
            2 => [p[0], p[0], p[0], p[1]],
            _ => [p[0], p[0], p[0], 255],
        });
    }
    Ok((info.width, info.height, rgba))
}
/// A PNG's width and height, from its header alone.
fn png_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut header = [0u8; 24];
    fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    let field = |at: usize| u32::from_be_bytes(header[at..at + 4].try_into().unwrap_or_default());
    (header[..8] == *b"\x89PNG\r\n\x1a\n" && header[12..16] == *b"IHDR")
        .then(|| (field(16), field(20)))
}
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "png") {
            out.push(path);
        }
    }
}
/// A packed atlas image: sprite name, size and RGBA pixels.
type PackedImage = (String, u32, u32, Vec<u8>);
/// A frame of an animated strip at `x, y`, nearest-resampled to the longer
/// side of the frame: array layers are square, as `Sprites::load` keeps them
/// for the world's sprite arrays.
fn strip_frame(pixels: &[u8], stride: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let side = w.max(h);
    let mut out = Vec::with_capacity((side * side * 4) as usize);
    for row in 0..side {
        let source_row = y + row * h / side;
        for column in 0..side {
            let at = ((source_row * stride + x + column * w / side) * 4) as usize;
            out.extend_from_slice(&pixels[at..at + 4]);
        }
    }
    out
}
/// `image` inside a one-texel border copied from its opposite edges, the
/// texels repeat addressing reads past each edge.
fn wrap_border((name, w, h, data): &PackedImage) -> PackedImage {
    let (bw, bh) = (w + 2, h + 2);
    let mut out = Vec::with_capacity((bw * bh * 4) as usize);
    for y in 0..bh {
        let row = (y + h - 1) % h;
        for x in 0..bw {
            let at = ((row * w + (x + w - 1) % w) * 4) as usize;
            out.extend_from_slice(&data[at..at + 4]);
        }
    }
    (name.clone(), bw, bh, out)
}
/// Sprites the atlas definitions (`atlases/*.json`) add beyond the texture
/// files already packed: generated ones (`paletted_permutations` such as the
/// armour trim sprites, `unstitch` regions) and files listed under another id.
fn atlas_sprites(root: &Path, packed: &[PackedImage]) -> Vec<PackedImage> {
    use crate::atlas_sources::{self, SpriteOrigin};
    let have: std::collections::HashSet<&str> = packed.iter().map(|i| i.0.as_str()).collect();
    let textures = atlas_sources::list_textures(root);
    // Each new name with its origins in the atlases' order: the first that
    // loads is the sprite. They load at once (a content pack's atlases list
    // tens of thousands of files), and come out in the order a load of one
    // after another would give.
    let mut names: Vec<(String, Vec<(usize, SpriteOrigin)>)> = Vec::new();
    let mut slots = std::collections::HashMap::new();
    let mut order = 0;
    for (atlas, sources) in atlas_sources::atlases(root) {
        // The other atlases' renamed files (GUI sprites, particles...) are
        // looked up by their texture paths here.
        let item_models = atlas == "minecraft:blocks" || atlas == "minecraft:items";
        for (id, origin) in atlas_sources::list(&sources, &textures) {
            if !item_models && matches!(origin, SpriteOrigin::File(_)) {
                continue;
            }
            let name = id.strip_prefix("minecraft:").unwrap_or(&id).to_owned();
            if have.contains(name.as_str()) {
                continue;
            }
            // A vanilla file is packed under its own name already: only a
            // renamed one is new. Other namespaces' files (content mods) are
            // never in the walked directories.
            if let SpriteOrigin::File(texture) = &origin
                && texture.strip_prefix("minecraft:") == Some(name.as_str())
            {
                continue;
            }
            let slot = *slots.entry(name.clone()).or_insert_with(|| {
                names.push((name, Vec::new()));
                names.len() - 1
            });
            names[slot].1.push((order, origin));
            order += 1;
        }
    }
    let load = |texture: &str, directory: &str| {
        decode(&atlas_sources::texture_path(root, texture, directory)).ok()
    };
    let first = |(name, origins): &(String, Vec<(usize, SpriteOrigin)>)| {
        origins.iter().find_map(|(order, origin)| {
            let image = match origin {
                SpriteOrigin::File(texture) => {
                    let path = texture.strip_prefix("minecraft:").unwrap_or(texture);
                    load(texture, "textures").map(|(w, h, p)| {
                        // Animated strips keep their first frame, like the packed files.
                        if h > w && (path.starts_with("block/") || path.starts_with("item/")) {
                            (w, w, p[..(w * w * 4) as usize].to_vec())
                        } else {
                            (w, h, p)
                        }
                    })
                }
                generated => atlas_sources::generate(generated, &load),
            };
            image.map(|(w, h, pixels)| (*order, (name.clone(), w, h, pixels)))
        })
    };
    let mut out: Vec<_> = crate::par::par_map(&names, first)
        .into_iter()
        .flatten()
        .collect();
    out.sort_unstable_by_key(|(order, _)| *order);
    out.into_iter().map(|(_, image)| image).collect()
}
impl HudAtlas {
    /// A font glyph's atlas cell (8x8 GUI pixels) and advance.
    pub fn glyph(&self, ch: char) -> Option<(Rect, f32)> {
        let g = self.font.glyph(ch);
        g.rect.map(|rect| (rect, g.advance))
    }
    /// A font glyph's atlas cell (none for blank glyphs such as the space) and advance.
    pub fn glyph_cell(&self, ch: char) -> (Option<Rect>, f32) {
        let g = self.font.glyph(ch);
        (g.rect, g.advance)
    }
    pub fn build(root: &Path) -> Result<Self> {
        Self::build_over(root, root)
    }
    /// The atlas of a resource root that layers packs over `defaults` (the
    /// game's own resources), whose textures give the sprites' GUI sizes.
    pub fn build_over(root: &Path, defaults: &Path) -> Result<Self> {
        Self::pack(
            root,
            defaults,
            &[
                "gui",
                "item",
                "block",
                "font",
                "environment",
                "misc",
                "painting",
                "entity",
                "particle",
                "mob_effect",
                "map",
            ],
            true,
        )
    }
    /// Empty slots written at run time: map textures (game/maps.rs), the
    /// minimap, downloaded skins, world and pack icons and the on-screen keyboard's text.
    fn slots(images: &mut Vec<PackedImage>, trim: (u32, u32)) {
        for i in 0..MAP_SLOTS {
            images.push((format!("horizon/map{i}"), 128, 128, vec![0; 128 * 128 * 4]));
        }
        // The minimap extra's image (game/extras.rs).
        images.push((
            MINIMAP_SPRITE.into(),
            crate::minimap::SIZE as u32,
            crate::minimap::SIZE as u32,
            vec![0; crate::minimap::SIZE * crate::minimap::SIZE * 4],
        ));
        // Downloaded skins (game/scoreboard.rs).
        for i in 0..SKIN_SLOTS {
            images.push((format!("horizon/skin{i}"), 64, 64, vec![0; 64 * 64 * 4]));
        }
        for i in 0..TRIM_SLOTS {
            images.push((
                format!("horizon/trim{i}"),
                trim.0,
                trim.1,
                vec![0; (trim.0 * trim.1 * 4) as usize],
            ));
        }
        for i in 0..WORLD_ICON_SLOTS {
            images.push((
                format!("horizon/world-icon{i}"),
                64,
                64,
                vec![0; 64 * 64 * 4],
            ));
        }
        let side = PACK_ICON_SIZE as usize;
        for i in 0..PACK_ICON_SLOTS {
            images.push((
                format!("horizon/pack-icon{i}"),
                PACK_ICON_SIZE,
                PACK_ICON_SIZE,
                vec![0; side * side * 4],
            ));
        }
        // The on-screen keyboard's text and corner masks (ui_theme), rasterized at run time.
        images.push((
            "horizon/ui-text".into(),
            2048,
            1024,
            vec![0; 2048 * 1024 * 4],
        ));
    }
    /// Only the font's metrics, for a game that draws nothing (the
    /// integrated server): text measures as with the full atlas, without
    /// holding its textures. The pixels are dropped once the glyph widths are
    /// read, since nothing uploads them.
    pub fn fonts(root: &Path) -> Result<Self> {
        let mut atlas = Self::pack(root, root, &["font"], false)?;
        atlas.pixels = Vec::new();
        Ok(atlas)
    }
    /// Shelf placements of `images` (tallest first) in the narrowest atlas
    /// width from 4096 up that keeps the atlas, glyph rows included, within
    /// `max` texels a side; and the first row free for glyphs. High-resolution
    /// packs fill a 4096-wide atlas past what GPUs allow (Faithful 64x needs
    /// 32768 rows); vanilla's stitcher fails the reload when its atlases don't
    /// fit, and the packs are then turned off (`Minecraft.clearResourcePacksOnError`).
    fn layout(images: &[PackedImage], max: u32) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        let mut width = 4096.min(max);
        loop {
            let mut placements = Vec::with_capacity(images.len());
            let (mut x, mut y, mut row) = (0u32, 0u32, 0u32);
            for (_, w, h, _) in images {
                if x + w + 1 > width {
                    x = 0;
                    y += row + 1;
                    row = 0;
                }
                placements.push((x, y));
                x += w + 1;
                row = row.max(*h);
            }
            let glyph_region = y + row + 1;
            let wide_enough = images.iter().all(|i| i.1 < width);
            if wide_enough && glyph_region + GLYPH_ROWS <= max {
                return Ok((width, placements, glyph_region));
            }
            if width >= max {
                let texels: u64 = images.iter().map(|i| u64::from(i.1) * u64::from(i.2)).sum();
                return Err(format!(
                    "the textures ({} images, {texels} texels) do not fit a {max}x{max} atlas",
                    images.len()
                )
                .into());
            }
            width = (width * 2).min(max);
        }
    }
    /// Pack the textures under these folders of `textures/`, and for the
    /// `full` atlas the generated sprites and the run-time slots too.
    fn pack(root: &Path, defaults: &Path, folders: &[&str], full: bool) -> Result<Self> {
        let textures = root.join("assets/minecraft/textures");
        let default_textures =
            (defaults != root).then(|| defaults.join("assets/minecraft/textures"));
        let mut files = Vec::new();
        for folder in folders {
            walk(&textures.join(folder), &mut files);
        }
        files.retain(|p| {
            let s = p.to_string_lossy();
            !s.contains("title/background") && !s.contains("realms") && !s.contains("presets")
        });
        type Decoded = (PackedImage, Option<TextureFlags>, Option<(u32, u32)>, bool);
        let decode_one = |path: &PathBuf| -> Option<Decoded> {
            let name = path
                .strip_prefix(&textures)
                .ok()?
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            let (mut w, mut h, pixels) = decode(path).ok()?;
            let mcmeta = fs::read_to_string(path.with_extension("png.mcmeta")).ok();
            let animation = mcmeta
                .as_deref()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                .and_then(|meta| {
                    meta.get("animation")
                        .and_then(|section| AnimationMetadata::from_json(&section.to_string()).ok())
                });
            // An animated strip keeps its first frame: the region the mcmeta's
            // frame size picks (`AnimationMetadataSection.frameSize`), which is
            // the top-left square unless it names other dimensions or the
            // frames run along a row.
            let frames = name.starts_with("block/") || name.starts_with("item/");
            let pixels = if frames {
                let stride = w;
                let [fw, fh] = animation.as_ref().map_or([w.min(h) as i32; 2], |meta| {
                    meta.frame_size(w as i32, h as i32)
                });
                match (u32::try_from(fw), u32::try_from(fh)) {
                    (Ok(fw), Ok(fh))
                        if fw > 0 && fh > 0 && w % fw == 0 && h % fh == 0 && (fw, fh) != (w, h) =>
                    {
                        let side = fw.max(fh);
                        (w, h) = (side, side);
                        strip_frame(&pixels, stride, 0, 0, fw, fh)
                    }
                    _ => pixels,
                }
            } else {
                pixels
            };
            // The default texture's size, when a pack's differs from it.
            let gui_size = default_textures
                .as_ref()
                .and_then(|d| png_size(&d.join(format!("{name}.png"))))
                .map(|(dw, dh)| {
                    let side = dw.min(dh);
                    if frames { (side, side) } else { (dw, dh) }
                })
                .filter(|&size| size != (w, h));
            let flags = mcmeta.as_deref().and_then(TextureFlags::from_mcmeta);
            let animated = frames && animation.is_some();
            let (gw, gh) = gui_size.unwrap_or((w, h));
            (gw <= 1024 && (gh <= 512 || name.starts_with("font/"))).then_some((
                (name, w, h, pixels),
                flags,
                gui_size,
                animated,
            ))
        };
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        let chunk = files.len().div_ceil(threads).max(1);
        let decoded: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = files
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || part.iter().filter_map(decode_one).collect::<Vec<_>>())
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        let mut flags = HashMap::new();
        let mut gui_sizes = HashMap::new();
        let mut animated: Vec<String> = Vec::new();
        let mut images: Vec<PackedImage> = Vec::with_capacity(decoded.len());
        for (image, image_flags, gui_size, is_animated) in decoded {
            if let Some(f) = image_flags {
                flags.insert(image.0.clone(), f);
            }
            if let Some(size) = gui_size {
                gui_sizes.insert(image.0.clone(), size);
            }
            if is_animated {
                animated.push(image.0.clone());
            }
            images.push(image);
        }
        if full {
            images.extend(atlas_sprites(root, &images));
        }
        // The title screen's "JAVA EDITION" banner reads "RUST EDITION".
        if let Some(edition) = images
            .iter_mut()
            .find(|i| i.0 == crate::title_edition::SPRITE)
            && let Some(pixels) =
                crate::title_edition::rust_edition(edition.1, edition.2, &edition.3)
        {
            edition.3 = pixels;
        }
        images.push(("horizon/white".into(), 4, 4, vec![255; 64]));
        if full {
            Self::slots(&mut images, crate::trims::slot_size(root));
        }
        // Blurred sprites are sampled linearly in the atlas (screen.wgsl): a
        // border wrapped from their own edges keeps the filter off their
        // neighbours, as repeat addressing of their own texture would.
        let mut bordered = std::collections::HashSet::new();
        for image in &mut images {
            if flags.get(&image.0).is_some_and(|f: &TextureFlags| f.blur) {
                *image = wrap_border(image);
                bordered.insert(image.0.clone());
            }
        }
        images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        let (width, placements, glyph_region) = Self::layout(&images, MAX_ATLAS_SIZE)?;
        // A multiple of 1024 rather than a power of two: the default resources
        // need 4096x5120 (8192 rows before), Faithful 32x 4096x12288 (16384).
        let height = (glyph_region + GLYPH_ROWS).next_multiple_of(1024);
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut sprites = HashMap::new();
        let mut translucent = HashSet::new();
        for ((name, w, h, data), (px, py)) in images.iter().zip(placements) {
            if data
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] != 0 && p[3] != 255)
            {
                translucent.insert(name.clone());
            }
            for row in 0..*h {
                let src = &data[(row * w * 4) as usize..((row + 1) * w * 4) as usize];
                let dst = (((py + row) * width + px) * 4) as usize;
                pixels[dst..dst + (w * 4) as usize].copy_from_slice(src);
            }
            let b = u32::from(bordered.contains(name));
            let (px, py, w, h) = (px + b, py + b, w - 2 * b, h - 2 * b);
            let (gw, gh) = gui_sizes.get(name).copied().unwrap_or((w, h));
            sprites.insert(
                name.clone(),
                Rect {
                    u0: px as f32 / width as f32,
                    v0: py as f32 / height as f32,
                    u1: (px + w) as f32 / width as f32,
                    v1: (py + h) as f32 / height as f32,
                    w: gw,
                    h: gh,
                },
            );
        }
        // Every animated sprite animates: the packed strip is the raw first
        // frame, while the mcmeta's frame list may start anywhere, so each
        // animation's frame(0) overwrites its atlas rect before drawing.
        let mut loaded = crate::sprites::Sprites::default();
        let mut sprite_animations = Vec::new();
        animated.sort();
        for name in animated {
            let Some(rect) = sprites.get(&name).copied() else {
                continue;
            };
            let layer = loaded.insert(root, &format!("minecraft:{name}"))?;
            let Some(animation) = loaded.animations.iter().find(|a| a.layer == layer).cloned()
            else {
                continue;
            };
            let (tw, th) = rect.texels((width, height));
            let image = animation.resized_pixels(animation.frame(0), tw, th);
            let x = (rect.u0 * width as f32).round() as usize;
            let y = (rect.v0 * height as f32).round() as usize;
            let row_bytes = tw as usize * 4;
            for row in 0..th as usize {
                let dst = ((y + row) * width as usize + x) * 4;
                let src = row * row_bytes;
                pixels[dst..dst + row_bytes].copy_from_slice(&image[src..src + row_bytes]);
            }
            sprite_animations.push((rect, animation));
        }
        let white = sprites["horizon/white"];
        let glint = Self::glint_texture(root, defaults, "enchanted_glint_item");
        let glint_armor = Self::glint_texture(root, defaults, "enchanted_glint_armor");
        let font = crate::font::Font::load(
            root,
            &sprites,
            &pixels,
            (width, height),
            (glyph_region, height - glyph_region),
            white,
        );
        Ok(Self {
            pixels,
            width,
            height,
            sprites,
            translucent,
            flags,
            sprite_animations,
            glint,
            glint_armor,
            font,
            white,
        })
    }
    /// `misc/<name>.png` on its own, or a transparent texel if the resource
    /// pack lacks it.
    fn glint_texture(root: &Path, defaults: &Path, name: &str) -> (Vec<u8>, u32, u32) {
        let relative = format!("assets/minecraft/textures/misc/{name}.png");
        let mut path = root.join(&relative);
        if !path.exists() && defaults != root {
            path = defaults.join(&relative);
        }
        decode(&path).map_or((vec![0; 4], 1, 1), |(w, h, pixels)| (pixels, w, h))
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HudVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
    pub depth: f32,
    /// The `GLINT_TEXTURING` UV into the repeating glint texture, unused
    /// unless `uv.x >= 2`.
    pub glint_uv: [f32; 2],
}
/// The GUI Scale option (0 = auto).
pub static GUI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// One frame of overlay geometry in GUI pixels.
pub struct HudBatch {
    pub vertices: Vec<HudVertex>,
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    /// Glyph pixels to upload into the atlas before drawing.
    pub patches: Vec<crate::font::Patch>,
    /// Vertex ranges with another blend or depth/scissor state, in order.
    pub blends: Vec<(std::ops::Range<u32>, Blend)>,
    /// The textures and time of the `Blend::EndPortal` runs.
    pub end_portal: EndPortalFill,
    /// `options.glintStrength` for the glint pass (`GlintAlpha`).
    pub glint_alpha: f32,
    /// Screen's first vertex: the preceding HUD is part of its blurred background.
    pub blur_at: u32,
}
/// How a run of HUD vertices blends and tests depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blend {
    /// `BlendFunction.TRANSLUCENT`.
    Alpha,
    /// `BlendFunction.INVERT` (`RenderPipelines.CROSSHAIR`, `GUI_INVERT`).
    Invert,
    /// `BlendFunction.ADDITIVE` (`RenderPipelines.GUI_TEXT_HIGHLIGHT`).
    Additive,
    /// Alpha blending with an isolated depth buffer and a pixel scissor.
    Depth([u32; 4]),
    /// `RenderPipelines.VIGNETTE`: the destination darkened by the texture
    /// (`ZERO, ONE_MINUS_SRC_COLOR`).
    Vignette,
    /// `RenderPipelines.END_PORTAL` as a GUI fill: the End portal's layers
    /// projected from the screen (`HudBatch::end_portal` sets the textures).
    EndPortal,
}
/// What a `Blend::EndPortal` fill samples: the atlas rects (u0, v0, width,
/// height) of `end_sky.png` and `end_portal.png`, and the `GameTime` uniform.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EndPortalFill {
    pub sprites: [[f32; 4]; 2],
    pub game_time: f32,
}
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(a) << 24 | u32::from(b) << 16 | u32::from(g) << 8 | u32::from(r)
}
impl HudBatch {
    /// Window.calculateScale: the largest scale keeping >= 320x240 GUI pixels,
    /// capped by the GUI Scale option (0 = auto).
    pub fn new(window: [u32; 2]) -> Self {
        let limit = GUI_SCALE.load(std::sync::atomic::Ordering::Relaxed);
        let mut scale = 1;
        while (limit == 0 || scale < limit)
            && window[0] / (scale + 1) >= 320
            && window[1] / (scale + 1) >= 240
        {
            scale += 1;
        }
        Self {
            vertices: Vec::new(),
            scale: scale as f32,
            width: (window[0] as f32 / scale as f32).ceil(),
            height: (window[1] as f32 / scale as f32).ceil(),
            patches: Vec::new(),
            blends: Vec::new(),
            end_portal: EndPortalFill::default(),
            glint_alpha: 0.75,
            blur_at: 0,
        }
    }
    /// Draw the vertices added since `start` with the inverting blend.
    pub fn invert_since(&mut self, start: usize) {
        self.blend_since(start, Blend::Invert);
    }
    /// Draw the vertices added since `start` with `blend`.
    pub fn blend_since(&mut self, start: usize, blend: Blend) {
        let range = start as u32..self.vertices.len() as u32;
        if range.is_empty() || blend == Blend::Alpha {
            return;
        }
        match self.blends.last_mut() {
            Some((last, b)) if last.end == range.start && *b == blend => last.end = range.end,
            _ => self.blends.push((range, blend)),
        }
    }
    /// `GuiGraphicsExtractor.textHighlight`: with `invert_text` the area's
    /// colours are inverted first (white text on a dark field turns blue on
    /// white), then blue is added.
    pub fn text_highlight(
        &mut self,
        atlas: &HudAtlas,
        (x, y, w, h): (f32, f32, f32, f32),
        invert_text: bool,
    ) {
        if invert_text {
            let start = self.vertices.len();
            self.fill(atlas, x, y, w, h, rgba(255, 255, 255, 255));
            self.invert_since(start);
        }
        let start = self.vertices.len();
        self.fill(atlas, x, y, w, h, rgba(0, 0, 255, 255));
        self.blend_since(start, Blend::Additive);
    }
    /// The vertex ranges in order, each with its blend.
    pub fn runs(&self) -> Vec<(std::ops::Range<u32>, Blend)> {
        let mut out = Vec::new();
        let mut at = 0;
        for (r, blend) in &self.blends {
            if r.start > at {
                out.push((at..r.start, Blend::Alpha));
            }
            out.push((r.clone(), *blend));
            at = r.end;
        }
        let end = self.vertices.len() as u32;
        if end > at {
            out.push((at..end, Blend::Alpha));
        }
        out
    }
    fn runs_in(&self, vertices: std::ops::Range<u32>) -> Vec<(std::ops::Range<u32>, Blend)> {
        let mut runs = self.runs();
        for (range, _) in &mut runs {
            *range = range.start.max(vertices.start)..range.end.min(vertices.end);
        }
        runs.retain(|(range, _)| !range.is_empty());
        runs
    }
    /// `AbstractContainerScreen.init`: center in integer GUI coordinates.
    pub fn centered_origin(&self, width: f32, height: f32) -> (f32, f32) {
        (
            ((self.width - width) / 2.0).trunc(),
            ((self.height - height) / 2.0).trunc(),
        )
    }
    pub fn quad(&mut self, p: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: u32) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: p[i],
                uv: uv[i],
                color,
                depth: 0.0,
                glint_uv: [0.0, 0.0],
            });
        }
    }
    /// A quad whose texture is the `GLINT_TEXTURING` sample at `glint_uv`;
    /// `uv` is the item's atlas UV, which the shader masks by.
    pub fn quad_glint(
        &mut self,
        p: [[f32; 2]; 4],
        uv: [[f32; 2]; 4],
        glint_uv: [[f32; 2]; 4],
        color: u32,
    ) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: p[i],
                uv: uv[i],
                color,
                depth: 0.0,
                glint_uv: glint_uv[i],
            });
        }
    }
    pub fn quad_3d(&mut self, p: [[f32; 3]; 4], uv: [[f32; 2]; 4], color: u32) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: [p[i][0], p[i][1]],
                uv: uv[i],
                color,
                depth: p[i][2],
                glint_uv: [0.0, 0.0],
            });
        }
    }
    /// Keep per-vertex depth for an orthographic GUI model, with +z toward the viewer.
    pub fn depth_since(&mut self, start: usize, clip: [f32; 4]) {
        let vertices = &mut self.vertices[start..];
        if vertices.is_empty() {
            return;
        }
        let min = vertices
            .iter()
            .map(|v| v.depth)
            .fold(f32::INFINITY, f32::min);
        let max = vertices
            .iter()
            .map(|v| v.depth)
            .fold(f32::NEG_INFINITY, f32::max);
        for v in vertices {
            v.depth = (max - v.depth) / (max - min).max(f32::EPSILON);
        }
        let [x0, y0, x1, y1] = clip.map(|v| (v * self.scale).max(0.0) as u32);
        // Separate previews clear depth independently, even if their scissors match.
        self.blends.push((
            start as u32..self.vertices.len() as u32,
            Blend::Depth([x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]),
        ));
    }
    pub fn rect_uv(&mut self, x: f32, y: f32, w: f32, h: f32, r: Rect, color: u32) {
        self.quad(
            [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
            color,
        );
    }
    pub fn sprite(&mut self, atlas: &HudAtlas, name: &str, x: f32, y: f32) {
        if let Some(r) = atlas.sprites.get(name).copied() {
            self.rect_uv(x, y, r.w as f32, r.h as f32, r, u32::MAX);
        }
    }
    pub fn sprite_scaled(
        &mut self,
        atlas: &HudAtlas,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
    ) {
        if let Some(r) = atlas.sprites.get(name).copied() {
            self.rect_uv(x, y, w, h, r, color);
        }
    }
    /// Draw part of a sprite, `fraction` of its width from the left.
    pub fn sprite_part(&mut self, atlas: &HudAtlas, name: &str, x: f32, y: f32, fraction: f32) {
        if let Some(mut r) = atlas.sprites.get(name).copied() {
            let w = r.w as f32 * fraction;
            r.u1 = r.u0 + (r.u1 - r.u0) * fraction;
            self.rect_uv(x, y, w, r.h as f32, r, u32::MAX);
        }
    }
    /// Clip the quads drawn since `start` to `[x0, x1) x [y0, y1)` (the
    /// GUI scissor): axis-aligned quads are cut with their UVs, others are
    /// kept or dropped by their centre.
    /// Scale the vertices pushed since `start` about `(cx, cy)` (a GUI pose
    /// matrix of translate, scale, translate back).
    pub fn scale_since(&mut self, start: usize, cx: f32, cy: f32, sx: f32, sy: f32) {
        for v in &mut self.vertices[start..] {
            v.pos[0] = cx + (v.pos[0] - cx) * sx;
            v.pos[1] = cy + (v.pos[1] - cy) * sy;
        }
    }
    pub fn clip_since(&mut self, start: usize, x0: f32, y0: f32, x1: f32, y1: f32) {
        let quads: Vec<[HudVertex; 6]> = self.vertices[start..].as_chunks::<6>().0.to_vec();
        self.vertices.truncate(start);
        for q in quads {
            let corners = [q[0], q[1], q[2], q[5]];
            let (mut x_lo, mut x_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            let (mut y_lo, mut y_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for v in &corners {
                x_lo = x_lo.min(v.pos[0]);
                x_hi = x_hi.max(v.pos[0]);
                y_lo = y_lo.min(v.pos[1]);
                y_hi = y_hi.max(v.pos[1]);
            }
            // An axis-aligned rectangle, its corners in any order.
            let find = |x: f32, y: f32| corners.iter().find(|v| v.pos == [x, y]).copied();
            let rect = (
                find(x_lo, y_lo),
                find(x_hi, y_lo),
                find(x_hi, y_hi),
                find(x_lo, y_hi),
            );
            let (Some(tl), Some(tr), Some(br), Some(bl)) = rect else {
                let cx = corners.iter().map(|v| v.pos[0]).sum::<f32>() / 4.0;
                let cy = corners.iter().map(|v| v.pos[1]).sum::<f32>() / 4.0;
                if cx >= x0 && cx < x1 && cy >= y0 && cy < y1 {
                    self.vertices.extend_from_slice(&q);
                }
                continue;
            };
            let (cx0, cy0) = (x_lo.max(x0), y_lo.max(y0));
            let (cx1, cy1) = (x_hi.min(x1), y_hi.min(y1));
            if cx0 >= cx1 || cy0 >= cy1 {
                continue;
            }
            let t = |p: f32, lo: f32, hi: f32| if hi == lo { 0.0 } else { (p - lo) / (hi - lo) };
            let uv = |x: f32, y: f32| {
                let (s, r) = (t(x, x_lo, x_hi), t(y, y_lo, y_hi));
                std::array::from_fn(|i| {
                    let top = tl.uv[i] + (tr.uv[i] - tl.uv[i]) * s;
                    let bottom = bl.uv[i] + (br.uv[i] - bl.uv[i]) * s;
                    top + (bottom - top) * r
                })
            };
            self.quad(
                [[cx0, cy0], [cx1, cy0], [cx1, cy1], [cx0, cy1]],
                [uv(cx0, cy0), uv(cx1, cy0), uv(cx1, cy1), uv(cx0, cy1)],
                tl.color,
            );
        }
    }
    /// `blit` of the top-left `w x h` texels of a `tw x th` texture.
    pub fn sprite_region(
        &mut self,
        atlas: &HudAtlas,
        name: &str,
        [x, y, w, h]: [f32; 4],
        tw: f32,
        th: f32,
    ) {
        if let Some(mut r) = atlas.sprites.get(name).copied() {
            r.u1 = r.u0 + (r.u1 - r.u0) * w / tw;
            r.v1 = r.v0 + (r.v1 - r.v0) * h / th;
            self.rect_uv(x, y, w, h, r, u32::MAX);
        }
    }
    pub fn fill(&mut self, atlas: &HudAtlas, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let mut r = atlas.white;
        let (cu, cv) = ((r.u0 + r.u1) / 2.0, (r.v0 + r.v1) / 2.0);
        r.u0 = cu;
        r.u1 = cu;
        r.v0 = cv;
        r.v1 = cv;
        self.rect_uv(x, y, w, h, r, color);
    }
    /// Any four-cornered shape in one colour, corners in order round it.
    pub fn fill_quad(&mut self, atlas: &HudAtlas, corners: [[f32; 2]; 4], color: u32) {
        let w = atlas.white;
        let uv = [(w.u0 + w.u1) / 2.0, (w.v0 + w.v1) / 2.0];
        self.quad(corners, [uv; 4], color);
    }
    /// GuiGraphics.fillGradient: `top` to `bottom`, down the rectangle.
    pub fn fill_gradient(
        &mut self,
        atlas: &HudAtlas,
        (x, y, w, h): (f32, f32, f32, f32),
        top: u32,
        bottom: u32,
    ) {
        let r = atlas.white;
        let uv = [(r.u0 + r.u1) / 2.0, (r.v0 + r.v1) / 2.0];
        let p = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        let c = [top, top, bottom, bottom];
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: p[i],
                uv,
                color: c[i],
                depth: 0.0,
                glint_uv: [0.0, 0.0],
            });
        }
    }
    pub fn text_width(atlas: &HudAtlas, text: &str) -> f32 {
        atlas.font.width(text)
    }
    /// Draw plain text (legacy `§` codes are honoured); returns the advance.
    pub fn text(
        &mut self,
        atlas: &HudAtlas,
        text: &str,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        let t = if text.contains('§') {
            crate::text::Text::legacy(text)
        } else {
            crate::text::Text::literal(text)
        };
        self.text_styled(atlas, &t, x, y, color, shadow)
    }
    /// Font.drawInternal: styled glyphs, shadows first, with bold, italic,
    /// underline and strikethrough. `color` is the default colour and alpha.
    pub fn text_styled(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        self.styled_text(atlas, text, x, y, color, shadow, false)
    }
    /// `text_styled` for text drawn in the world (`TextRenderable.render`
    /// with `flat` false): each quad's `depth` is its z in text units, the
    /// glyphs `Font.SHADOW_DEPTH` in front of their shadow, a bold copy 0.001
    /// in front of its glyph and the lines 0.01 (`BakedSheetGlyph`), so they
    /// do not fight over the same pixels.
    pub fn text_styled_world(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        self.styled_text(atlas, text, x, y, color, shadow, true)
    }
    #[allow(clippy::too_many_arguments)]
    fn styled_text(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
        world: bool,
    ) -> f32 {
        // StringDecomposer.iterateFormatted inside Font.drawInternal:
        // `§` codes inside a span restyle what follows and are not drawn.
        let text = text.resolved();
        // Language.getVisualOrder: shaped and reordered for right-to-left text.
        let text = crate::bidi::visual(&text);
        let font = &atlas.font;
        let alpha = (color >> 24) as u8;
        let mut glyphs = Vec::new();
        let mut cursor = x;
        let mut previous = None;
        for (i, (ch, style)) in text.chars().enumerate() {
            // Font.PreparedTextBuilder.accept: lines start 1 left of a
            // sequence's first character. Language.getVisualOrder makes a
            // sequence of each run of one style, which world text has.
            let first = i == 0 || world && previous != Some(style);
            previous = Some(style);
            // The style's font (`Style.getFont`), the default one unless set.
            let mut g = font.glyph_styled(ch, style);
            // Font.StringRenderOutput.accept: obfuscated characters draw a
            // random glyph of the same width each frame.
            if style.obfuscated == Some(true) && ch != ' ' {
                g = font.random_glyph(style, g.advance);
            }
            let c = style.color.map_or(color, |rgb| {
                rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, alpha)
            });
            let bold = style.is_bold();
            let advance = g.advance + if bold { g.offset } else { 0.0 };
            glyphs.push((
                cursor,
                g,
                c,
                bold,
                style.is_italic(),
                style.underlined == Some(true),
                style.strikethrough == Some(true),
                first,
                advance,
            ));
            cursor += advance;
        }
        self.patches.extend(font.take_patches());
        let white = font.white();
        let passes: &[bool] = if shadow { &[true, false] } else { &[false] };
        // ARGB.scaleRGB(color, 0.25): the shadow's colour.
        let shade = |c: u32| {
            let b = c.to_le_bytes();
            rgba(
                (f32::from(b[0]) * 0.25) as u8,
                (f32::from(b[1]) * 0.25) as u8,
                (f32::from(b[2]) * 0.25) as u8,
                b[3],
            )
        };
        for &is_shadow in passes {
            for &(gx, g, c, bold, italic, underline, strike, first, advance) in &glyphs {
                // In the world a glyph whose shadow is 0 has none (hasShadow).
                let has_shadow = shadow && !(world && shade(c) == 0);
                if is_shadow && !has_shadow {
                    continue;
                }
                let (dx, col) = if is_shadow {
                    (g.offset, shade(c))
                } else {
                    (0.0, c)
                };
                let (px, py) = (gx + dx, y + dx);
                let shear = |v: f32| if italic { 1.0 - 0.25 * v } else { 0.0 };
                let thick = if bold { 0.1 } else { 0.0 };
                let draw = |batch: &mut Self, ox: f32| match g.rect {
                    Some(r) => {
                        let (x0, x1, y0, y1) = (
                            px + ox + g.left,
                            px + ox + g.right,
                            py + g.top,
                            py + g.bottom,
                        );
                        batch.quad(
                            [
                                [x0 + shear(g.top) - thick, y0 - thick],
                                [x1 + shear(g.top) + thick, y0 - thick],
                                [x1 + shear(g.bottom) + thick, y1 + thick],
                                [x0 + shear(g.bottom) - thick, y1 + thick],
                            ],
                            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
                            col,
                        );
                    }
                    None if g.right > 0.0 => {
                        // SpecialGlyphs.MISSING: a 5x8 outline.
                        let (bx, by) = (px + ox, py);
                        for (fx, fy, fw, fh) in [
                            (0.0, 0.0, 5.0, 1.0),
                            (0.0, 7.0, 5.0, 1.0),
                            (0.0, 1.0, 1.0, 6.0),
                            (4.0, 1.0, 1.0, 6.0),
                        ] {
                            batch.rect_uv(bx + fx, by + fy, fw, fh, white, col);
                        }
                    }
                    None => {}
                };
                let base = if has_shadow && !is_shadow { 0.03 } else { 0.0 };
                let lift = |batch: &mut Self, start: usize, z: f32| {
                    if world {
                        for v in &mut batch.vertices[start..] {
                            v.depth = z;
                        }
                    }
                };
                let start = self.vertices.len();
                draw(self, 0.0);
                lift(self, start, base);
                if bold {
                    let start = self.vertices.len();
                    draw(self, g.offset);
                    lift(self, start, base + 0.001);
                }
                let x0 = if first { px - 1.0 } else { px };
                let start = self.vertices.len();
                if strike {
                    self.rect_uv(x0, py + 3.5, px + advance - x0, 1.0, white, col);
                }
                if underline {
                    self.rect_uv(x0, py + 8.0, px + advance - x0, 1.0, white, col);
                }
                lift(self, start, base + 0.01);
            }
        }
        cursor - x
    }
    /// SubmitNodeCollection.submitNameTag's text in its own space (x right,
    /// y down, in GUI pixels): centred at `-Font.width / 2`, drawn as
    /// Font.prepareText draws any text (styles, fonts and glyph boxes; no
    /// shadow), over `background` (RGBA) unless its alpha is 0. Returns how
    /// many vertices at the start of the batch are the background.
    pub fn name_tag(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        y: f32,
        color: u32,
        background: u32,
    ) -> usize {
        let x = -atlas.font.width_text(text) / 2.0;
        let start = self.vertices.len();
        let advance = self.text_styled(atlas, text, x, y, color, false);
        if background >> 24 == 0 {
            return 0;
        }
        // PreparedTextBuilder.markBackground: one pixel round the advances,
        // from a pixel above the line to 9 below its top.
        let glyphs = self.vertices.split_off(start);
        self.fill(atlas, x - 1.0, y - 1.0, advance + 1.0, 10.0, background);
        let behind = self.vertices.len() - start;
        self.vertices.extend(glyphs);
        behind
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HudUniform {
    size: [f32; 4],
    /// `EndPortalFill`: the two sprites, then the game time in x.
    end_portal: [[f32; 4]; 3],
}
impl HudUniform {
    fn new(size: [f32; 4], fill: EndPortalFill) -> Self {
        Self {
            size,
            end_portal: [
                fill.sprites[0],
                fill.sprites[1],
                [fill.game_time, 0.0, 0.0, 0.0],
            ],
        }
    }
}
pub struct HudRenderer {
    pub sprite_animations: Vec<(Rect, crate::sprites::SpriteAnimation, (usize, usize, u32))>,
    /// The shared atlas texture, for run-time glyph uploads.
    pub atlas_texture: Option<wgpu::Texture>,
    pipeline: wgpu::RenderPipeline,
    /// `RenderPipelines.CROSSHAIR`.
    invert_pipeline: wgpu::RenderPipeline,
    additive_pipeline: wgpu::RenderPipeline,
    depth_pipeline: wgpu::RenderPipeline,
    vignette_pipeline: wgpu::RenderPipeline,
    end_portal_pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    buffer: wgpu::Buffer,
    capacity: u64,
}
impl HudRenderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        view: &wgpu::TextureView,
        glint_view: &wgpu::TextureView,
        glint_sampler: &wgpu::Sampler,
    ) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Horizon HUD"),
            source: wgpu::ShaderSource::Wgsl(include_str!("hud.wgsl").into()),
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("HUD"),
            ..Default::default()
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HUD size"),
            contents: bytemuck::bytes_of(&HudUniform::new([1.0; 4], EndPortalFill::default())),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("HUD bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HUD resources"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(glint_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(glint_sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HUD layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let attributes =
            wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Unorm8x4,3=>Float32,4=>Float32x2];
        let create = |label, blend, fragment_entry, depth: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<HudVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: Default::default(),
                depth_stencil: depth.then_some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment_entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = create("HUD", wgpu::BlendState::ALPHA_BLENDING, "fs_main", false);
        let depth_pipeline = create(
            "HUD model",
            wgpu::BlendState::ALPHA_BLENDING,
            "fs_main",
            true,
        );
        // BlendFunction.INVERT: ONE_MINUS_DST_COLOR, ONE_MINUS_SRC_COLOR,
        // ONE, ZERO.
        let invert_pipeline = create(
            "HUD crosshair",
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::OneMinusDst,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
            },
            "fs_crosshair",
            false,
        );
        // BlendFunction.ADDITIVE: ONE, ONE.
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let additive_pipeline = create(
            "HUD text highlight",
            wgpu::BlendState {
                color: add,
                alpha: add,
            },
            "fs_main",
            false,
        );
        // RenderPipelines.VIGNETTE: ZERO, ONE_MINUS_SRC_COLOR, ZERO, ONE.
        let vignette_pipeline = create(
            "HUD vignette",
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
            "fs_main",
            false,
        );
        // RenderPipelines.END_PORTAL: opaque.
        let end_portal_pipeline = create(
            "HUD end portal",
            wgpu::BlendState::REPLACE,
            "fs_end_portal",
            false,
        );
        let capacity = 1 << 16;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("HUD vertices"),
            size: capacity * std::mem::size_of::<HudVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            atlas_texture: None,
            sprite_animations: Vec::new(),
            pipeline,
            invert_pipeline,
            additive_pipeline,
            depth_pipeline,
            vignette_pipeline,
            end_portal_pipeline,
            group,
            uniform,
            buffer,
            capacity,
        })
    }
    pub fn animate_sprites(
        &mut self,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        tick: u64,
    ) {
        let Some(texture) = &self.atlas_texture else {
            return;
        };
        for (rect, animation, previous) in &mut self.sprite_animations {
            // `SpriteContents.AnimationState`: the frame from the mcmeta's
            // list, blended into the next one by the sub-frame's thousandths
            // when the sprite interpolates.
            let blend = animation.blend(tick);
            let state = (blend.frame, blend.next, blend.millis);
            if state == *previous {
                continue;
            }
            let (w, h) = rect.texels((texture.width(), texture.height()));
            let pixels = if blend.millis == 0 {
                animation.resized_pixels(blend.frame, w, h)
            } else {
                let current = animation.resized_pixels(blend.frame, w, h);
                let next = animation.resized_pixels(blend.next, w, h);
                let millis = blend.millis;
                current
                    .iter()
                    .zip(next)
                    .map(|(&a, b)| {
                        ((u32::from(a) * (1000 - millis) + u32::from(b) * millis + 500) / 1000)
                            as u8
                    })
                    .collect()
            };
            uploads.texture(
                encoder,
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: (rect.u0 * texture.width() as f32).round() as u32,
                        y: (rect.v0 * texture.height() as f32).round() as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                w,
                h,
                &pixels,
            );
            *previous = state;
        }
    }
    /// Stage the batch's atlas patches, vertices and uniform: before the
    /// frame's first pass, so the copies share one command buffer.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut crate::gfx::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        batch: &HudBatch,
        size: [u32; 2],
    ) {
        if let Some(texture) = &self.atlas_texture {
            for p in &batch.patches {
                uploads.texture(
                    encoder,
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: p.x,
                            y: p.y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    p.width,
                    p.height,
                    &p.rgba,
                );
            }
        }
        if batch.vertices.is_empty() || size.contains(&0) {
            return;
        }
        let count = batch.vertices.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("HUD vertices"),
                size: self.capacity * std::mem::size_of::<HudVertex>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        uploads.buffer(
            encoder,
            &self.buffer,
            0,
            bytemuck::cast_slice(&batch.vertices),
        );
        uploads.buffer(
            encoder,
            &self.uniform,
            0,
            bytemuck::bytes_of(&HudUniform::new(
                [
                    size[0] as f32,
                    size[1] as f32,
                    batch.scale,
                    batch.glint_alpha,
                ],
                batch.end_portal,
            )),
        );
    }
    /// Draw the batch `prepare` staged.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        batch: &HudBatch,
        size: [u32; 2],
    ) {
        self.draw_range(
            encoder,
            target,
            depth,
            batch,
            size,
            0..batch.vertices.len() as u32,
        );
    }
    /// Draw part of the prepared batch, preserving its blend/depth runs.
    pub fn draw_range(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        batch: &HudBatch,
        size: [u32; 2],
        vertices: std::ops::Range<u32>,
    ) {
        if vertices.is_empty() || size.contains(&0) {
            return;
        }
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })];
        let runs = batch.runs_in(vertices);
        let mut at = 0;
        while at < runs.len() {
            let clip = match runs[at].1 {
                Blend::Depth(clip) => Some(clip),
                _ => None,
            };
            let end = if clip.is_some() {
                at + 1
            } else {
                runs[at..]
                    .iter()
                    .position(|(_, b)| matches!(b, Blend::Depth(_)))
                    .map_or(runs.len(), |n| at + n)
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HUD"),
                color_attachments: &attachments,
                depth_stencil_attachment: clip.map(|_| wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_vertex_buffer(0, self.buffer.slice(..));
            if let Some([x, y, w, h]) = clip {
                let (x, y) = (x.min(size[0]), y.min(size[1]));
                let (w, h) = (w.min(size[0] - x), h.min(size[1] - y));
                if w == 0 || h == 0 {
                    at = end;
                    continue;
                }
                pass.set_scissor_rect(x, y, w, h);
            }
            for (range, blend) in &runs[at..end] {
                pass.set_pipeline(match blend {
                    Blend::Alpha => &self.pipeline,
                    Blend::Invert => &self.invert_pipeline,
                    Blend::Additive => &self.additive_pipeline,
                    Blend::Depth(_) => &self.depth_pipeline,
                    Blend::Vignette => &self.vignette_pipeline,
                    Blend::EndPortal => &self.end_portal_pipeline,
                });
                pass.draw(range.clone(), 0..1);
            }
            at = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a PNG `width x height` whose `frames` stacked rows are solid
    /// colours, then its mcmeta beside it.
    fn write_strip(path: &Path, width: u32, height: u32, frames: &[[u8; 4]], mcmeta: Option<&str>) {
        let file = std::io::BufWriter::new(fs::File::create(path).unwrap());
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for frame in frames {
            let rows = height / frames.len() as u32;
            pixels.extend((0..width * rows).flat_map(|_| *frame));
        }
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
        if let Some(mcmeta) = mcmeta {
            let meta = path.with_extension("png.mcmeta");
            fs::write(meta, mcmeta).unwrap();
        }
    }
    /// A resource root holding one animated block texture: a strip of two
    /// 16x16 frames, red then blue, whose mcmeta starts on the blue one.
    fn animated_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("horizon-atlas-anim-{}", std::process::id()));
        let block = root.join("assets/minecraft/textures/block");
        fs::create_dir_all(&block).unwrap();
        write_strip(
            &block.join("test_anim.png"),
            16,
            32,
            &[[255, 0, 0, 255], [0, 0, 255, 255]],
            Some(r#"{"animation": {"frames": [1, 0]}}"#),
        );
        root
    }
    fn texel_at(atlas: &HudAtlas, rect: Rect, x: u32, y: u32) -> [u8; 4] {
        let px = (rect.u0 * atlas.width as f32).round() as u32 + x;
        let py = (rect.v0 * atlas.height as f32).round() as u32 + y;
        let at = ((py * atlas.width + px) * 4) as usize;
        atlas.pixels[at..at + 4].try_into().unwrap()
    }
    #[test]
    fn animated_block_sprites_register_and_pack_their_first_mcmeta_frame() {
        let root = animated_root();
        let atlas = HudAtlas::build(&root).unwrap();
        let (rect, animation) = atlas
            .sprite_animations
            .iter()
            .find(|(_, animation)| animation.frame_count() == 2)
            .expect("the animated block sprite");
        // The mcmeta's list starts on the strip's second (blue) frame.
        assert_eq!(animation.frame(0), 1);
        assert_eq!(rect.texels((atlas.width, atlas.height)), (16, 16));
        assert_eq!(texel_at(&atlas, *rect, 0, 0), [0, 0, 255, 255]);
        assert_eq!(texel_at(&atlas, *rect, 15, 15), [0, 0, 255, 255]);
    }
    #[test]
    fn a_strip_frames_size_comes_from_its_mcmeta() {
        let root = animated_root();
        let block = root.join("assets/minecraft/textures/block");
        // A row of two frames with the animation's own frame size.
        write_strip(
            &block.join("test_row.png"),
            32,
            16,
            &[[0, 255, 0, 255], [255, 255, 0, 255]],
            Some(r#"{"animation": {"width": 16, "height": 16}}"#),
        );
        let atlas = HudAtlas::build(&root).unwrap();
        let rect = atlas.sprites["block/test_row"];
        // The first frame is the left 16x16, not the whole row.
        assert_eq!(rect.texels((atlas.width, atlas.height)), (16, 16));
        assert_eq!(texel_at(&atlas, rect, 0, 0), [0, 255, 0, 255]);
    }
    /// `count` images of `w x h`; layout reads only their sizes.
    fn sized(count: usize, w: u32, h: u32) -> Vec<PackedImage> {
        (0..count)
            .map(|i| (format!("i{i}"), w, h, Vec::new()))
            .collect()
    }
    fn assert_inside(images: &[PackedImage], width: u32, placements: &[(u32, u32)], free: u32) {
        for ((_, w, h, _), (x, y)) in images.iter().zip(placements) {
            assert!(
                x + w <= width && y + h < free,
                "{x},{y} {w}x{h} outside {width}x{free}"
            );
        }
    }
    fn write_png(root: &Path, name: &str, w: u32, h: u32) {
        let path = root.join(format!("assets/minecraft/textures/{name}.png"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels = vec![255; (w * h * 4) as usize];
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    #[test]
    fn a_pack_texture_keeps_the_default_textures_gui_size() {
        let temp = std::env::temp_dir().join(format!("horizon-gui-size-{}", std::process::id()));
        let (defaults, pack) = (temp.join("defaults"), temp.join("pack"));
        write_png(&defaults, "gui/container/inventory", 256, 256);
        write_png(&defaults, "gui/sprites/widget/button", 200, 20);
        write_png(&defaults, "block/stone", 16, 16);
        write_png(&defaults, "item/compass", 16, 512);
        // Faithful 64x's inventory (past the 512 rows a texture may have),
        // 32x's button and stone, an animated strip, and a texture of its own.
        write_png(&pack, "gui/container/inventory", 1024, 1024);
        write_png(&pack, "gui/sprites/widget/button", 400, 40);
        write_png(&pack, "block/stone", 32, 32);
        write_png(&pack, "item/compass", 32, 1024);
        write_png(&pack, "gui/sprites/pack_only", 30, 10);
        let atlas = HudAtlas::build_over(&pack, &defaults).unwrap();
        let _ = fs::remove_dir_all(&temp);
        let size = (atlas.width, atlas.height);
        for (name, gui, texels) in [
            ("gui/container/inventory", (256, 256), (1024, 1024)),
            ("gui/sprites/widget/button", (200, 20), (400, 40)),
            ("block/stone", (16, 16), (32, 32)),
            ("item/compass", (16, 16), (32, 32)),
            ("gui/sprites/pack_only", (30, 10), (30, 10)),
        ] {
            let r = atlas.sprites[name];
            assert_eq!(((r.w, r.h), r.texels(size)), (gui, texels), "{name}");
        }
    }
    #[test]
    fn the_default_textures_keep_a_4096_wide_atlas() {
        let images = sized(400, 64, 64);
        let (width, placements, free) = HudAtlas::layout(&images, MAX_ATLAS_SIZE).unwrap();
        assert_eq!(width, 4096);
        assert_inside(&images, width, &placements, free);
    }
    #[test]
    fn a_high_resolution_pack_widens_the_atlas_to_fit_the_gpu() {
        // 34 rows of three 1024x512 GUI textures: 17442 rows at 4096 wide,
        // past the 16384 GPUs allow (Faithful 64x needed 32768).
        let images = sized(100, 1024, 512);
        let (width, placements, free) = HudAtlas::layout(&images, MAX_ATLAS_SIZE).unwrap();
        assert_eq!(width, 8192);
        assert!(free + GLYPH_ROWS <= MAX_ATLAS_SIZE);
        assert_inside(&images, width, &placements, free);
    }
    #[test]
    fn textures_that_cannot_fit_are_an_error_not_a_gpu_panic() {
        let error = HudAtlas::layout(&sized(300, 1024, 1024), MAX_ATLAS_SIZE).unwrap_err();
        assert!(error.to_string().contains("do not fit a 16384x16384 atlas"));
        // One image wider than the largest atlas.
        assert!(HudAtlas::layout(&sized(1, 2048, 16), 2048).is_err());
    }

    #[test]
    fn a_wrapped_border_repeats_the_opposite_edges() {
        // 2x2 texels a b / c d, one byte each in red.
        let image = (
            "x".to_owned(),
            2,
            2,
            [1u8, 2, 3, 4]
                .iter()
                .flat_map(|&v| [v, 0, 0, 255])
                .collect(),
        );
        let (_, w, h, data) = wrap_border(&image);
        assert_eq!((w, h), (4, 4));
        let red: Vec<u8> = data.chunks(4).map(|t| t[0]).collect();
        #[rustfmt::skip]
        assert_eq!(red, [
            4, 3, 4, 3,
            2, 1, 2, 1,
            4, 3, 4, 3,
            2, 1, 2, 1,
        ]);
    }

    /// A synthetic default font: two 8x8 glyphs, 3 and 5 pixels wide.
    fn two_glyph_font(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("horizon-fonts-{name}-{}", std::process::id()));
        let font = root.join("assets/minecraft/font");
        let textures = root.join("assets/minecraft/textures/font");
        fs::create_dir_all(&font).unwrap();
        fs::create_dir_all(&textures).unwrap();
        fs::write(
            font.join("default.json"),
            r#"{"providers":[{"type":"bitmap","file":"minecraft:font/test.png","ascent":7,"chars":["ab"]}]}"#,
        )
        .unwrap();
        let mut pixels = vec![0u8; 16 * 8 * 4];
        for y in 0..8 {
            for x in (0..3).chain(8..13) {
                pixels[(y * 16 + x) * 4 + 3] = 255;
            }
        }
        let mut encoder =
            png::Encoder::new(fs::File::create(textures.join("test.png")).unwrap(), 16, 8);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        root
    }
    #[test]
    fn the_fonts_only_atlas_measures_text_like_the_full_one() {
        let root = two_glyph_font("atlas");
        let full = HudAtlas::build(&root).unwrap();
        let fonts = HudAtlas::fonts(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert!(fonts.pixels.is_empty());
        let width = full.font.width("abba");
        assert!(width > 0.0);
        assert_eq!(fonts.font.width("abba"), width);
    }
    #[test]
    fn obfuscated_glyphs_keep_the_width() {
        let root = two_glyph_font("obfuscated");
        let fonts = HudAtlas::fonts(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        let (a, b) = (fonts.font.glyph('a').advance, fonts.font.glyph('b').advance);
        assert_ne!(a, b);
        for _ in 0..20 {
            assert_eq!(fonts.font.random_glyph(&Default::default(), a).advance, a);
            assert_eq!(fonts.font.random_glyph(&Default::default(), b).advance, b);
        }
        // No glyph that wide: FontSet's missing glyph.
        let missing = fonts.font.random_glyph(&Default::default(), 30.0);
        assert!(missing.rect.is_none() && missing.right == 5.0);
    }

    #[test]
    fn name_tags_draw_bold_text_and_tall_glyphs_like_any_text() {
        // `ascii.png`'s 8x8 'a' (4 pixels wide) and `accented.png`'s 9x12
        // 'й' (height 12, ascent 10: the breve above the line), 7 wide.
        let root = std::env::temp_dir().join(format!("horizon-name-tag-{}", std::process::id()));
        let textures = root.join("assets/minecraft/textures/font");
        fs::create_dir_all(root.join("assets/minecraft/font")).unwrap();
        fs::create_dir_all(&textures).unwrap();
        fs::write(
            root.join("assets/minecraft/font/default.json"),
            r#"{"providers":[
                {"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["a"]},
                {"type":"bitmap","file":"minecraft:font/tall.png","height":12,"ascent":10,"chars":["й"]}]}"#,
        )
        .unwrap();
        for (name, (w, h), used) in [("a", (8, 8), 4), ("tall", (9, 12), 7)] {
            let pixels: Vec<u8> = (0..w * h)
                .flat_map(|i| [255, 255, 255, if i % w < used { 255 } else { 0 }])
                .collect();
            let file = fs::File::create(textures.join(format!("{name}.png"))).unwrap();
            let mut encoder = png::Encoder::new(file, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&pixels).unwrap();
        }
        let atlas = HudAtlas::build(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        let text = crate::text::Text::literal("aй").bold();
        let mut batch = HudBatch::new([1, 1]);
        let behind = batch.name_tag(&atlas, &text, 0.0, u32::MAX, 0x3F00_0000);
        // Each quad's box: x0, y0, x1, y1.
        let boxes: Vec<[f32; 4]> = batch
            .vertices
            .as_chunks::<6>()
            .0
            .iter()
            .map(|q| [q[0].pos[0], q[0].pos[1], q[2].pos[0], q[2].pos[1]])
            .collect();
        // Advances 5 and 8, plus 1 each for bold: Font.width 15, centred.
        let x = -7.5;
        // Each glyph twice (bold), the copy a pixel right; 'й' in its 9x12
        // box from 3 pixels above the line, each bold quad 0.1 larger.
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
        let expected = [
            [x - 0.1, -0.1, x + 8.1, 8.1],
            [x + 0.9, -0.1, x + 9.1, 8.1],
            [x + 5.9, -3.1, x + 15.1, 9.1],
            [x + 6.9, -3.1, x + 16.1, 9.1],
        ];
        for (got, want) in boxes[1..].iter().zip(expected) {
            assert!(near(*got, want), "{got:?} != {want:?}");
        }
        assert_eq!(boxes.len(), 1 + expected.len());
        // The background first: a pixel round the advances, 10 pixels tall.
        assert_eq!(behind, 6);
        assert_eq!(boxes[0], [x - 1.0, -1.0, x + 15.0, 9.0]);
    }

    #[test]
    fn effect_icons_are_painted_from_the_mob_effect_directory() {
        use crate::{effects::EffectInstance, hud_layout, hud_overlays, inventory_effects};
        // A synthetic resource tree exercises loading and painting together.
        // No Minecraft installation or GPU is needed for this regression test.
        let root =
            std::env::temp_dir().join(format!("horizon-effect-icons-{}", std::process::id()));
        for (name, size) in [
            ("mob_effect/levitation", 18),
            ("gui/sprites/hud/effect_background", 24),
            ("gui/sprites/container/inventory/effect_background", 32),
        ] {
            let path = root.join(format!("assets/minecraft/textures/{name}.png"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), size, size);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![255; (size * size * 4) as usize])
                .unwrap();
        }
        let atlas = HudAtlas::build(&root).unwrap();
        let effect = EffectInstance::new("levitation", -1, 0);
        let icon = hud_overlays::EffectIcon::from(&effect);
        let draws = hud_overlays::effects(320, false, &[icon]);
        let mut batch = HudBatch::new([320, 240]);
        hud_layout::paint(&mut batch, &atlas, &draws);
        // Both the background and the 18x18 effect icon must reach the batch.
        assert_eq!(batch.vertices.len(), 12);
        let rect = atlas.sprites["mob_effect/levitation"];
        assert_eq!(batch.vertices[6].pos, [298.0, 30.0]);
        assert_eq!(batch.vertices[6].uv, [rect.u0, rect.v0]);
        assert_eq!(batch.vertices[8].uv, [rect.u1, rect.v1]);

        let panels = inventory_effects::panels(
            320,
            [72, 37],
            176,
            std::slice::from_ref(&effect),
            20.0,
            &|_| None,
            &|_| 60,
        );
        batch.vertices.clear();
        let tooltip = inventory_effects::paint(&mut batch, &atlas, &panels, [257, 44]);
        assert_eq!(tooltip.unwrap()[0].0, "levitation");
        assert_eq!(batch.vertices.len(), 24);
        assert_eq!(batch.vertices[18].pos, [257.0, 44.0]);
        assert_eq!(batch.vertices[18].uv, [rect.u0, rect.v0]);
        assert_eq!(batch.vertices[20].uv, [rect.u1, rect.v1]);

        // Wide panels retain the four-pixel border instead of stretching it.
        let panels =
            inventory_effects::panels(480, [72, 37], 176, &[effect], 20.0, &|_| None, &|_| 60);
        batch.vertices.clear();
        assert!(inventory_effects::paint(&mut batch, &atlas, &panels, [0, 0]).is_none());
        assert!(batch.vertices.iter().any(|v| v.color == 0xff808080));
        assert_eq!(batch.vertices[0].pos, [250.0, 37.0]);
        assert_eq!(batch.vertices[1].pos, [254.0, 37.0]);
        let background = atlas.sprites["gui/sprites/container/inventory/effect_background"];
        assert!(
            (batch.vertices[1].uv[0] - background.u0 - (background.u1 - background.u0) / 8.0).abs()
                < 1e-6
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gui_models_keep_separate_depth_runs_between_overlay_draws() {
        let mut batch = HudBatch::new([640, 480]);
        batch.scale = 2.0;
        let p = [
            [0.0, 0.0, 2.0],
            [0.0, 1.0, 2.0],
            [1.0, 1.0, 2.0],
            [1.0, 0.0, 2.0],
        ];
        let uv = [[0.0; 2]; 4];
        batch.quad([[0.0; 2]; 4], uv, u32::MAX);
        for _ in 0..2 {
            let start = batch.vertices.len();
            batch.quad_3d(p, uv, u32::MAX);
            batch.depth_since(start, [1.0, 2.0, 4.0, 6.0]);
        }
        batch.quad([[0.0; 2]; 4], uv, u32::MAX);
        assert_eq!(
            batch.runs(),
            vec![
                (0..6, Blend::Alpha),
                (6..12, Blend::Depth([2, 4, 6, 8])),
                (12..18, Blend::Depth([2, 4, 6, 8])),
                (18..24, Blend::Alpha),
            ]
        );
        assert!(
            batch
                .vertices
                .iter()
                .all(|v| v.depth.is_finite() && (0.0..=1.0).contains(&v.depth))
        );
    }

    #[test]
    fn hud_shader_validates_both_fragment_entries() {
        let module = naga::front::wgsl::parse_str(include_str!("hud.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn inverted_ranges_split_the_draws_in_order() {
        let mut batch = HudBatch::new([854, 480]);
        let quad = |b: &mut HudBatch| b.quad([[0.0; 2]; 4], [[0.0; 2]; 4], 0);
        quad(&mut batch);
        let start = batch.vertices.len();
        quad(&mut batch);
        batch.invert_since(start);
        let start = batch.vertices.len();
        quad(&mut batch);
        batch.invert_since(start);
        batch.invert_since(batch.vertices.len());
        let start = batch.vertices.len();
        quad(&mut batch);
        batch.blend_since(start, Blend::Additive);
        quad(&mut batch);
        assert_eq!(
            batch.blends,
            vec![(6..18, Blend::Invert), (18..24, Blend::Additive)]
        );
        assert_eq!(
            batch.runs(),
            vec![
                (0..6, Blend::Alpha),
                (6..18, Blend::Invert),
                (18..24, Blend::Additive),
                (24..30, Blend::Alpha)
            ]
        );
    }

    #[test]
    fn blur_boundary_splits_blend_runs_without_losing_or_repeating_vertices() {
        let mut batch = HudBatch::new([854, 480]);
        for _ in 0..5 {
            batch.quad([[0.0; 2]; 4], [[0.0; 2]; 4], 0);
        }
        batch.blends = vec![
            (6..18, Blend::Invert),
            (18..24, Blend::Depth([0, 0, 10, 10])),
        ];
        for split in [0, 6, 12, 18, 24, 30] {
            let before = batch.runs_in(0..split);
            let after = batch.runs_in(split..30);
            let actual: Vec<_> = before
                .into_iter()
                .chain(after)
                .flat_map(|(range, blend)| range.map(move |v| (v, blend)))
                .collect();
            let expected: Vec<_> = batch
                .runs()
                .into_iter()
                .flat_map(|(range, blend)| range.map(move |v| (v, blend)))
                .collect();
            assert_eq!(actual, expected, "blur at {split}");
        }
    }
    #[test]
    fn container_origins_use_java_integer_division() {
        // Odd scaled dimensions, odd image sizes and an oversized panel.
        for (width, height) in [(640, 360), (641, 361), (456, 256), (534, 300), (100, 100)] {
            let batch = HudBatch {
                vertices: Vec::new(),
                scale: 3.0,
                width: width as f32,
                height: height as f32,
                patches: Vec::new(),
                blends: Vec::new(),
                end_portal: EndPortalFill::default(),
                glint_alpha: 0.75,
                blur_at: 0,
            };
            for (image_width, image_height) in [(195, 136), (176, 166), (176, 222)] {
                assert_eq!(
                    batch.centered_origin(image_width as f32, image_height as f32),
                    (
                        ((width - image_width) / 2) as f32,
                        ((height - image_height) / 2) as f32
                    )
                );
            }
        }
    }
}
