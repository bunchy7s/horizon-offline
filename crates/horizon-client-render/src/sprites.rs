//! Shared nearest-sampled sprite array for block and item textures.
use crate::Result;
use horizon_assets::animation::{AnimationClip, AnimationMetadata};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::BufReader,
    path::Path,
    sync::Arc,
};

#[derive(Clone)]
pub struct SpriteAnimation {
    pub layer: u32,
    frames: Arc<Vec<Vec<u8>>>,
    clip: Arc<AnimationClip>,
}
/// What an animated sprite shows: `frame`, mixed `millis` thousandths of the
/// way towards `next` (0 for a single frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameBlend {
    pub frame: usize,
    pub next: usize,
    pub millis: u32,
}
impl SpriteAnimation {
    pub fn frame(&self, tick: u64) -> usize {
        self.clip.sample(tick).frame as usize
    }
    /// `SpriteContents.AnimationState.drawToAtlas`: an animation with
    /// `interpolate` mixes the frame with the next one by
    /// `(int)(subFrame / time * 1000)` thousandths; others show the frame.
    pub fn blend(&self, tick: u64) -> FrameBlend {
        let sample = self.clip.sample(tick);
        let frame = sample.frame as usize;
        let next = sample.next_frame as usize;
        if self.clip.interpolates() && sample.interpolation_millis > 0 && next != frame {
            FrameBlend {
                frame,
                next,
                millis: sample.interpolation_millis,
            }
        } else {
            FrameBlend {
                frame,
                next: frame,
                millis: 0,
            }
        }
    }
    pub fn resized_pixels(&self, frame: usize, width: u32, height: u32) -> Vec<u8> {
        let source = self.pixels(frame);
        let side = (source.len() / 4).isqrt() as u32;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let offset = ((y * side / height * side + x * side / width) * 4) as usize;
                out.extend_from_slice(&source[offset..offset + 4]);
            }
        }
        out
    }
    pub fn pixels(&self, frame: usize) -> &[u8] {
        &self.frames[frame]
    }
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

#[derive(Default)]
pub struct Sprites {
    /// Edge length of every array layer and the layers' current frames, set
    /// by [`Sprites::finish`].
    pub size: u32,
    /// Smallest native layer before array-size resampling.
    pub minimum_size: u32,
    pub pixels: Vec<u8>,
    pub layers: BTreeMap<String, u32>,
    pub animations: Vec<SpriteAnimation>,
    pub opaque: Vec<bool>,
    pub translucent: Vec<bool>,
    /// Each layer's `MipmapGenerator` settings.
    pub mips: Vec<crate::mipmap::Settings>,
    /// Each layer's frames at the texture's own resolution (square), and the
    /// frame shown first.
    native: Vec<(u32, Arc<Vec<Vec<u8>>>, usize)>,
    /// Each layer's LabPBR maps (`_n` normals, `_s` specular) beside its texture: their first
    /// frame at their own resolution (square).
    pbr: Vec<[Option<(u32, Vec<u8>)>; 2]>,
    /// The layers' normal and specular maps at `pbr_size`, layer after layer, set by
    /// [`Sprites::finish`]: empty when no texture has either.
    pub normals: Vec<u8>,
    pub specular: Vec<u8>,
    pub pbr_size: u32,
    /// Still textures by their pixels, their LabPBR maps and mip settings: mods copy vanilla's
    /// textures under their own names, and one layer serves them all (the GPU's array may
    /// hold as few as 2048).
    still: std::collections::HashMap<(Vec<u8>, u64, u32, [u32; 2], [bool; 4]), u32>,
    /// Decoded textures kept on disk between launches ([`Sprites::with_disk_cache`]).
    disk: Option<crate::sprite_cache::SpriteCache>,
    /// Textures [`Sprites::prefetch`] decoded ahead of their insert, by sprite name or
    /// resource path.
    ready: std::collections::HashMap<String, crate::sprite_cache::Decoded>,
}
/// Edge length of vanilla's block textures: the smallest array layer.
pub const SPRITE_SIZE: u32 = 16;
/// The largest sprite array (all layers of the top mip level) when the settings leave it to
/// Horizon: resource packs with bigger textures are sampled down to fit.
pub const DEFAULT_TEXTURE_BUDGET: u64 = 256 << 20;
/// The budget in bytes the next resource load keeps to (`set_texture_budget`).
static TEXTURE_BUDGET: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(DEFAULT_TEXTURE_BUDGET);
/// The block textures' budget from the `textureBudgetMiB` setting (0 for the default), for
/// resources loaded from now on.
pub fn set_texture_budget(mib: u32) {
    let bytes = match mib {
        0 => DEFAULT_TEXTURE_BUDGET,
        mib => u64::from(mib) << 20,
    };
    TEXTURE_BUDGET.store(bytes, std::sync::atomic::Ordering::Relaxed);
}
/// Whether resource loads read the resource packs' LabPBR maps (`set_labpbr_maps`).
static LABPBR_MAPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Whether resource loads from now on read the resource packs' LabPBR maps beside their block
/// textures. Only a native shader pack's material stage reads them, so without one they cost
/// neither loading time nor video memory.
pub fn set_labpbr_maps(on: bool) {
    LABPBR_MAPS.store(on, std::sync::atomic::Ordering::Relaxed);
}
/// Whether resource loads read the LabPBR maps (`set_labpbr_maps`).
pub fn labpbr_maps() -> bool {
    LABPBR_MAPS.load(std::sync::atomic::Ordering::Relaxed)
}
/// Largest array of each LabPBR map, which only a native shader pack reads.
const MAX_MAP_BYTES: u64 = DEFAULT_TEXTURE_BUDGET / 4;
/// Fewer layers than any game has (vanilla's are about 1300): what a map is kept at while
/// loading assumes at least these many share the array.
const LAYERS_AT_LEAST: u64 = 1024;
/// A layer's normal map where its texture has none: flat, unoccluded, at full height.
pub const FLAT_NORMAL: [u8; 4] = [128, 128, 255, 255];
/// A layer's specular map where its texture has none: rough, not metal, not glowing.
pub const NO_SPECULAR: [u8; 4] = [0, 0, 0, 0];
/// The largest power-of-two side from `start` down (at least `SPRITE_SIZE`) at which `layers`
/// square RGBA layers take at most `bytes`.
fn side_within(start: u32, layers: u64, bytes: u64) -> u32 {
    let mut size = start.max(SPRITE_SIZE).next_power_of_two();
    while size > SPRITE_SIZE && layers * u64::from(size) * u64::from(size) * 4 > bytes {
        size /= 2;
    }
    size
}
/// A LabPBR map's square `frame` of `side` averaged down by whole powers of two to at most
/// `max`, channel by channel: they are data, not colour.
fn shrink_map(frame: Vec<u8>, side: u32, max: u32) -> (u32, Vec<u8>) {
    if side <= max || !side.is_power_of_two() || !max.is_power_of_two() {
        return (side, frame);
    }
    let (out_side, f, side) = (max as usize, (side / max) as usize, side as usize);
    let mut out = Vec::with_capacity(out_side * out_side * 4);
    for y in 0..out_side {
        for x in 0..out_side {
            let mut sum = [0u32; 4];
            for dy in 0..f {
                let row = ((y * f + dy) * side + x * f) * 4;
                for texel in frame[row..row + f * 4].as_chunks::<4>().0 {
                    for (s, &v) in sum.iter_mut().zip(texel) {
                        *s += u32::from(v);
                    }
                }
            }
            let n = (f * f) as u32;
            out.extend(sum.map(|s| ((s + n / 2) / n) as u8));
        }
    }
    (max, out)
}
/// Nearest-resample a square RGBA frame to another square size.
pub(crate) fn resample(frame: &[u8], from: u32, to: u32) -> Vec<u8> {
    if from == to {
        return frame.to_vec();
    }
    let mut out = Vec::with_capacity((to * to * 4) as usize);
    for row in 0..to {
        let source_row = row * from / to;
        for column in 0..to {
            let offset = ((source_row * from + column * from / to) * 4) as usize;
            out.extend_from_slice(&frame[offset..offset + 4]);
        }
    }
    out
}
impl Sprites {
    /// Sprites that reuse textures decoded by an earlier launch, kept in a
    /// cache file in `root`.
    pub fn with_disk_cache(root: &Path) -> Self {
        Self {
            disk: crate::sprite_cache::SpriteCache::open(root),
            ..Self::default()
        }
    }
    /// Write what this run decoded to the cache file.
    pub fn save_disk_cache(&mut self) {
        if let Some(disk) = &mut self.disk {
            disk.save();
        }
    }
    /// How many array layers the textures take.
    pub fn count(&self) -> usize {
        self.native.len()
    }
    /// `SpriteContents.computeTransparency(u0, v0, u1, v1)` is NONE: every texel under
    /// the quad's UV box is fully opaque in every frame, so the quad needs no alpha test.
    pub fn region_opaque(&self, layer: u32, uv: &[[f32; 2]; 4]) -> bool {
        if self.opaque[layer as usize] {
            return true;
        }
        let (side, frames, _) = &self.native[layer as usize];
        let side = *side as usize;
        let range = |axis: usize| {
            let lo = uv.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min);
            let hi = uv.iter().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max);
            let clamp = |v: f32| (v.max(0.0) as usize).min(side);
            clamp((lo * side as f32).floor())..clamp((hi * side as f32).ceil())
        };
        let (columns, rows) = (range(0), range(1));
        frames.iter().all(|frame| {
            rows.clone().all(|y| {
                columns
                    .clone()
                    .all(|x| frame[(y * side + x) * 4 + 3] == 255)
            })
        })
    }
    /// Lay every layer out at one size: the largest texture's (resource packs
    /// may ship 32x, 64x... textures, like vanilla's stitched atlas), rounded
    /// up to a power of two and limited to the texture budget (`set_texture_budget`). Smaller
    /// textures are scaled up by whole pixels, so UVs stay per layer.
    pub fn finish(&mut self) {
        self.lay_out(true);
    }
    /// [`Sprites::finish`] for a server, which draws nothing: the layers'
    /// size, but neither their pixels nor the animations' frames.
    pub fn finish_without_pixels(&mut self) {
        self.lay_out(false);
    }
    fn lay_out(&mut self, pixels: bool) {
        // Prefetched textures nothing inserted.
        self.ready = Default::default();
        let layers = self.native.len().max(1) as u64;
        let largest = self.native.iter().map(|n| n.0).max().unwrap_or(SPRITE_SIZE);
        self.minimum_size = self.native.iter().map(|n| n.0).min().unwrap_or(SPRITE_SIZE);
        let budget = TEXTURE_BUDGET.load(std::sync::atomic::Ordering::Relaxed);
        let size = side_within(largest, layers, budget);
        if pixels {
            log::info!(
                "Horizon: block sprites: {layers} layers at {size} px (largest {largest} px, budget {} MiB)",
                budget >> 20
            );
        }
        if !pixels {
            self.pixels = Vec::new();
            self.animations = Vec::new();
            self.pbr = Vec::new();
            self.size = size;
            return;
        }
        let mut resized = Vec::with_capacity(self.native.len());
        self.pixels.clear();
        for (from, frames, initial) in &self.native {
            let frames: Arc<Vec<Vec<u8>>> =
                Arc::new(frames.iter().map(|f| resample(f, *from, size)).collect());
            self.pixels.extend_from_slice(&frames[*initial]);
            resized.push(frames);
        }
        for animation in &mut self.animations {
            animation.frames = resized[animation.layer as usize].clone();
        }
        self.size = size;
        self.normals.clear();
        self.specular.clear();
        let largest_map = self.pbr.iter().flatten().flatten().map(|m| m.0).max();
        self.pbr_size = largest_map.map_or(SPRITE_SIZE, |l| {
            side_within(l.min(size), layers, MAX_MAP_BYTES)
        });
        if largest_map.is_some() {
            let pbr_size = self.pbr_size;
            log::info!("Horizon: LabPBR maps at {pbr_size} px, {layers} layers");
            for maps in std::mem::take(&mut self.pbr) {
                for (map, (out, none)) in maps.into_iter().zip([
                    (&mut self.normals, FLAT_NORMAL),
                    (&mut self.specular, NO_SPECULAR),
                ]) {
                    match map {
                        Some((from, pixels)) => {
                            let (from, pixels) = shrink_map(pixels, from, pbr_size);
                            out.extend(resample(&pixels, from, pbr_size));
                        }
                        None => out.extend(none.repeat((pbr_size * pbr_size) as usize)),
                    }
                }
            }
        }
    }
    /// [`Sprites::insert`] without the missing-texture fallback: a file that doesn't load is an
    /// error, and nothing is added.
    pub fn try_insert(&mut self, root: &Path, name: &str) -> Result<u32> {
        match self.layers.get(name) {
            Some(layer) => Ok(*layer),
            None => self.load(root, name),
        }
    }
    /// The layer of a texture, added on first use. A texture that is missing
    /// or does not load (a resource pack's filter or a broken file) becomes
    /// vanilla's missing texture, as in a stitched atlas.
    pub fn insert(&mut self, root: &Path, name: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(name) {
            return Ok(*layer);
        }
        match self.load(root, name) {
            Ok(layer) => Ok(layer),
            Err(e) => {
                // `MissingTextureAtlasSprite.getLocation()`: vanilla models name the
                // missing texture on purpose (block/air.json's particle), #853.
                if name != "minecraft:missingno" {
                    log::warn!("Horizon: using the missing texture for {name}: {e}");
                }
                // MissingTextureAtlasSprite.generateMissingImage: 16x16,
                // magenta and black quarters.
                let frame: Vec<u8> = (0..SPRITE_SIZE * SPRITE_SIZE)
                    .flat_map(|i| {
                        let (x, y) = (i % SPRITE_SIZE, i / SPRITE_SIZE);
                        if (y < SPRITE_SIZE / 2) ^ (x < SPRITE_SIZE / 2) {
                            [248, 0, 248, 255]
                        } else {
                            [0, 0, 0, 255]
                        }
                    })
                    .collect();
                let layer = u32::try_from(self.native.len())?;
                self.opaque.push(true);
                self.pbr.push([None, None]);
                self.translucent.push(false);
                self.mips.push(crate::mipmap::Settings::default());
                self.layers.insert(name.to_owned(), layer);
                self.native.push((SPRITE_SIZE, Arc::new(vec![frame]), 0));
                Ok(layer)
            }
        }
    }
    /// Decode on every thread the textures about to be inserted: sprite names, or resource
    /// paths outside `textures/` (as [`Sprites::insert_file`] takes them). A loader that adds
    /// many (the block models, an OptiFine pack's tiles) calls this first and then inserts
    /// them in its own order, so the layers are the same as without it. What is already a
    /// layer or in the disk cache is skipped; what fails to decode is left for the insert,
    /// which reports it.
    pub fn prefetch<'a>(&mut self, root: &Path, names: impl IntoIterator<Item = &'a str>) {
        let names: Vec<&str> = names
            .into_iter()
            .filter(|name| {
                !self.layers.contains_key(*name)
                    && !self.ready.contains_key(*name)
                    && self.disk.as_ref().is_none_or(|d| d.get(name).is_none())
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let decoded = crate::par::par_map(&names, |name| {
            if name.contains(':') {
                Self::decode(root, name).ok()
            } else {
                Self::decode_file(&root.join(name), name).ok()
            }
        });
        for (name, decoded) in names.into_iter().zip(decoded) {
            if let Some(decoded) = decoded {
                self.ready.insert(name.to_owned(), decoded);
            }
        }
    }
    fn load(&mut self, root: &Path, name: &str) -> Result<u32> {
        // Custom Item Textures' files are outside `textures/`, which the disk cache's
        // fingerprint covers: like `insert_file`'s, they skip it.
        let cit = name.starts_with("horizon_cit:");
        let disk = self.disk.as_mut().filter(|_| !cit);
        let cached = disk.as_ref().and_then(|d| d.get(name)).cloned();
        let decoded = match cached {
            Some(decoded) => decoded,
            None => {
                let decoded = match self.ready.remove(name) {
                    Some(decoded) => decoded,
                    None => Self::decode(root, name)?,
                };
                if let Some(disk) = disk {
                    disk.insert(name, decoded.clone());
                }
                decoded
            }
        };
        self.register(root, name, decoded)
    }
    /// The layer of a texture outside `textures/` (an OptiFine pack's connected-texture tile),
    /// named by its resource path and added on first use. Unlike [`Sprites::insert`], a texture
    /// that does not load is an error: the caller drops what needed it. Such files bypass the
    /// disk cache, whose fingerprint covers only `textures/`.
    pub fn insert_file(&mut self, root: &Path, resource: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(resource) {
            return Ok(*layer);
        }
        let decoded = match self.ready.remove(resource) {
            Some(decoded) => decoded,
            None => Self::decode_file(&root.join(resource), resource)?,
        };
        self.register(root, resource, decoded)
    }
    /// The tiles of the texture `name` (`ns:path`) cut into a `columns` x `rows` grid, row by
    /// row from the top left: a Fusion connecting texture's sheet. Each tile is its own layer
    /// (named `fusion/<ns>/<path>#<tile>`); a fully transparent one is `None`, as Fusion skips
    /// empty tiles. Tiles are square, as wide as a column. An animated sheet's frames are whole
    /// grids one below the other, or with `per_tile` each cell is a column of its tile's frames.
    /// The sheet's own layer, if models already show it, shows its first tile from then on.
    pub fn insert_tiles(
        &mut self,
        root: &Path,
        name: &str,
        (columns, rows): (u32, u32),
        per_tile: bool,
    ) -> Result<Vec<Option<u32>>> {
        let path = crate::atlas_sources::texture_path(root, name, "textures");
        let metadata = match fs::read_to_string(path.with_extension("png.mcmeta")) {
            Ok(text) => Some(serde_json::from_str::<
                BTreeMap<String, Box<serde_json::value::RawValue>>,
            >(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let animation = metadata
            .as_ref()
            .and_then(|m| m.get("animation"))
            .map(|v| v.get().to_owned());
        // The sheet's `texture` section is each tile's.
        let texture = texture_section(metadata.as_ref());
        let strategy = texture
            .as_ref()
            .and_then(|t| t["mipmap_strategy"].as_str())
            .and_then(crate::mipmap::Strategy::parse)
            .unwrap_or_default();
        let alpha_cutoff_bias = texture
            .as_ref()
            .and_then(|t| t["alpha_cutoff_bias"].as_f64())
            .unwrap_or(0.0) as f32;
        let (width, height, pixels) = read_rgba(&path)?;
        let side = width / columns.max(1);
        // How far down the next frame of a tile is, and the next row of tiles.
        let (frame_step, row_step) = match per_tile {
            true => (side, height / rows.max(1)),
            false => (rows * side, side),
        };
        let frames = match per_tile {
            true => row_step / side.max(1),
            false => height / frame_step.max(1),
        };
        let whole = match per_tile {
            true => height % rows.max(1) == 0 && row_step % side.max(1) == 0,
            false => height % frame_step.max(1) == 0,
        };
        if side == 0 || frames == 0 || width % columns.max(1) != 0 || !whole {
            return Err(format!("{name} isn't a {columns} x {rows} grid of square tiles").into());
        }
        let tiles = (0..columns * rows)
            .map(|tile| {
                let frames: Vec<Vec<u8>> = (0..frames)
                    .map(|f| {
                        let (x, y) = (
                            tile % columns * side,
                            tile / columns * row_step + f * frame_step,
                        );
                        (y..y + side)
                            .flat_map(|row| {
                                let start = ((row * width + x) * 4) as usize;
                                pixels[start..start + side as usize * 4].iter().copied()
                            })
                            .collect()
                    })
                    .collect();
                let alphas = || frames.iter().flat_map(|f| f.iter().skip(3).step_by(4));
                if alphas().all(|&a| a == 0) {
                    return Ok(None);
                }
                let decoded = crate::sprite_cache::Decoded {
                    side,
                    animation: animation.clone().filter(|_| frames.len() > 1),
                    strategy,
                    alpha_cutoff_bias,
                    has_transparent: alphas().any(|&a| a == 0),
                    opaque: alphas().all(|&a| a == 255),
                    translucent: alphas().any(|&a| a != 0 && a != 255),
                    frames,
                };
                let tile_name = format!("fusion/{}#{tile}", name.replace(':', "/"));
                self.register(root, &tile_name, decoded).map(Some)
            })
            .collect::<Result<Vec<_>>>()?;
        // Where Fusion doesn't draw the sheet (items, other models), the lone tile rather than
        // the whole sheet squeezed square, whose size would set every layer's.
        if let (Some(&own), Some(&first)) = (self.layers.get(name), tiles.iter().flatten().next()) {
            self.show_as(own, first);
        }
        Ok(tiles)
    }
    /// Layer `layer` shows what `source` does, and is no longer the layer of its old pixels.
    fn show_as(&mut self, layer: u32, source: u32) {
        self.still.retain(|_, l| *l != layer);
        let (l, s) = (layer as usize, source as usize);
        self.native[l] = self.native[s].clone();
        self.opaque[l] = self.opaque[s];
        self.translucent[l] = self.translucent[s];
        self.mips[l] = self.mips[s];
        self.pbr[l] = self.pbr[s].clone();
        self.animations.retain(|a| a.layer != layer);
        if let Some(animation) = self.animations.iter().find(|a| a.layer == source) {
            let animation = SpriteAnimation {
                layer,
                ..animation.clone()
            };
            self.animations.push(animation);
        }
    }
    /// Read and decode one texture and its `.mcmeta`.
    fn decode(root: &Path, name: &str) -> Result<crate::sprite_cache::Decoded> {
        if !name.contains(':') {
            return Err("invalid sprite identifier".into());
        }
        // `texture_path`, so Custom Item Textures' `horizon_cit:` sprites find their file.
        let path = crate::atlas_sources::texture_path(root, name, "textures");
        Self::decode_file(&path, name)
    }
    fn decode_file(path: &Path, name: &str) -> Result<crate::sprite_cache::Decoded> {
        let metadata = match fs::read_to_string(path.with_extension("png.mcmeta")) {
            Ok(text) => Some(serde_json::from_str::<
                BTreeMap<String, Box<serde_json::value::RawValue>>,
            >(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let animation_text = metadata
            .as_ref()
            .and_then(|v| v.get("animation"))
            .map(|v| v.get().to_owned());
        let animation = animation_text
            .as_deref()
            .map(AnimationMetadata::from_json)
            .transpose()?;
        let (png_width, png_height, pixels) = read_rgba(path)?;
        let (width, height) = if let Some(animation) = &animation {
            let [width, height] =
                animation.frame_size(i32::try_from(png_width)?, i32::try_from(png_height)?);
            (u32::try_from(width)?, u32::try_from(height)?)
        } else {
            (png_width, png_height)
        };
        if width == 0 || height == 0 || png_width % width != 0 || png_height % height != 0 {
            return Err(format!("sprite {name} has invalid frame dimensions").into());
        }
        let columns = png_width / width;
        let count = columns * (png_height / height);
        // Layers are square; a frame of another shape is nearest-resampled to
        // its longer side so UVs (normalized per layer) are unchanged.
        let side = width.max(height);
        let mut frames = Vec::new();
        for index in 0..count {
            let x = index % columns * width;
            let y = index / columns * height;
            let mut frame = Vec::with_capacity((side * side * 4) as usize);
            for row in 0..side {
                let source_row = y + row * height / side;
                for column in 0..side {
                    let offset =
                        ((source_row * png_width + x + column * width / side) * 4) as usize;
                    frame.extend_from_slice(&pixels[offset..offset + 4]);
                }
            }
            frames.push(frame);
        }
        let texture = texture_section(metadata.as_ref());
        Ok(crate::sprite_cache::Decoded {
            side,
            frames,
            animation: animation_text,
            strategy: texture
                .as_ref()
                .and_then(|t| t["mipmap_strategy"].as_str())
                .and_then(crate::mipmap::Strategy::parse)
                .unwrap_or_default(),
            alpha_cutoff_bias: texture
                .as_ref()
                .and_then(|t| t["alpha_cutoff_bias"].as_f64())
                .unwrap_or(0.0) as f32,
            has_transparent: pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0),
            opaque: pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
            translucent: pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] != 0 && p[3] != 255),
        })
    }
    /// Give a decoded texture its layer (or the one an identical still has).
    fn register(
        &mut self,
        root: &Path,
        name: &str,
        decoded: crate::sprite_cache::Decoded,
    ) -> Result<u32> {
        let crate::sprite_cache::Decoded {
            side,
            frames,
            animation: animation_text,
            strategy,
            alpha_cutoff_bias,
            has_transparent,
            opaque,
            translucent,
        } = decoded;
        let animation = animation_text
            .as_deref()
            .map(AnimationMetadata::from_json)
            .transpose()?;
        let count = frames.len() as u32;
        let layer = u32::try_from(self.native.len())?;
        let pbr = match name.split_once(':') {
            Some(_) if labpbr_maps() => {
                let path = crate::atlas_sources::texture_path(root, name, "textures");
                ["_n", "_s"].map(|suffix| pbr_map(&path, suffix, count))
            }
            _ => [None, None],
        };
        let settings = crate::mipmap::Settings {
            native_size: side,
            strategy,
            alpha_cutoff_bias,
            has_transparent,
            // Custom Item Textures' ids are all item textures.
            item: name.starts_with("horizon_cit:")
                || name
                    .split_once(':')
                    .is_some_and(|(_, p)| p.starts_with("item/")),
            dither_cutouts: crate::mipmap::dither_cutouts(),
        };
        let still = (animation.is_none() && frames.len() == 1).then(|| {
            use std::hash::{Hash, Hasher};
            let mut maps = std::collections::hash_map::DefaultHasher::new();
            pbr.hash(&mut maps);
            (
                frames[0].clone(),
                maps.finish(),
                side,
                [
                    settings.strategy as u32,
                    settings.alpha_cutoff_bias.to_bits(),
                ],
                [settings.has_transparent, settings.item, opaque, translucent],
            )
        });
        if let Some(&same) = still.as_ref().and_then(|k| self.still.get(k)) {
            self.layers.insert(name.to_owned(), same);
            return Ok(same);
        }
        if let Some(key) = still {
            self.still.insert(key, layer);
        }
        let frames = Arc::new(frames);
        let mut initial = 0;
        if let Some(clip) = animation
            .as_ref()
            .map(|m| m.clip(count))
            .transpose()?
            .flatten()
        {
            initial = clip.sample(0).frame as usize;
            self.animations.push(SpriteAnimation {
                layer,
                frames: frames.clone(),
                clip: Arc::new(clip),
            });
        }
        self.mips.push(settings);
        self.opaque.push(opaque);
        self.translucent.push(translucent);
        self.pbr.push(pbr);
        self.layers.insert(name.to_owned(), layer);
        self.native.push((side, frames, initial));
        Ok(layer)
    }
}

/// A PNG's width, height and pixels as RGBA.
fn read_rgba(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    decoder.ignore_checksums(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("sprite exceeds decoder limits")?
    ];
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    let mut pixels = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    for pixel in bytes[..info.buffer_size()].chunks_exact(channels) {
        let rgba = match info.color_type {
            png::ColorType::Rgba => [pixel[0], pixel[1], pixel[2], pixel[3]],
            png::ColorType::Rgb => [pixel[0], pixel[1], pixel[2], 255],
            png::ColorType::Grayscale => [pixel[0], pixel[0], pixel[0], 255],
            png::ColorType::GrayscaleAlpha => [pixel[0], pixel[0], pixel[0], pixel[1]],
            _ => return Err("unsupported expanded sprite format".into()),
        };
        pixels.extend(rgba);
    }
    Ok((info.width, info.height, pixels))
}

/// The LabPBR map with `suffix` beside the texture at `path`, whose frames lie in a column of
/// `count`: its first frame, square, no larger than the maps' array allows any game's
/// textures (a 512x pack's maps would otherwise fill the memory while loading).
fn pbr_map(path: &Path, suffix: &str, count: u32) -> Option<(u32, Vec<u8>)> {
    let stem = path.file_stem()?.to_str()?;
    let map = path.with_file_name(format!("{stem}{suffix}.png"));
    let (map_width, map_height, pixels) = read_rgba(&map).ok()?;
    let (w, h) = (map_width, map_height / count.max(1));
    if w == 0 || h == 0 {
        return None;
    }
    let side = w.max(h);
    let mut frame = Vec::with_capacity((side * side * 4) as usize);
    for row in 0..side {
        for column in 0..side {
            let offset = (((row * h / side) * map_width + column * w / side) * 4) as usize;
            frame.extend_from_slice(&pixels[offset..offset + 4]);
        }
    }
    Some(shrink_map(
        frame,
        side,
        side_within(side, LAYERS_AT_LEAST, MAX_MAP_BYTES),
    ))
}

/// SpriteContents: the `.mcmeta` `texture` section (TextureMetadataSection's
/// mipmap strategy and alpha cutoff bias).
fn texture_section(
    metadata: Option<&BTreeMap<String, Box<serde_json::value::RawValue>>>,
) -> Option<serde_json::Value> {
    metadata
        .and_then(|v| v.get("texture"))
        .and_then(|v| serde_json::from_str(v.get()).ok())
}

#[cfg(test)]
pub(crate) fn test_animation(size: u32, layer: u32) -> SpriteAnimation {
    test_clip(size, layer, r#"{"frametime":1,"frames":[0,1]}"#)
}
#[cfg(test)]
pub(crate) fn test_clip(size: u32, layer: u32, metadata: &str) -> SpriteAnimation {
    let frame = |blue: u8| {
        (0..size * size)
            .flat_map(|i| [(i % size) as u8, (i / size) as u8, blue, 255])
            .collect()
    };
    test_frames(layer, vec![frame(0), frame(99)], metadata)
}
#[cfg(test)]
pub(crate) fn test_frames(layer: u32, frames: Vec<Vec<u8>>, metadata: &str) -> SpriteAnimation {
    let metadata = AnimationMetadata::from_json(metadata).unwrap();
    SpriteAnimation {
        layer,
        clip: Arc::new(metadata.clip(frames.len() as u32).unwrap().unwrap()),
        frames: Arc::new(frames),
    }
}

/// How many reduced copies a sprite atlas keeps: the Mipmap Levels option plus the
/// resampling-only levels an upscaled pack needs, within what the atlas holds.
pub(crate) fn sprite_mipmap_limit(requested: u32, size: u32, minimum: u32, available: u32) -> u32 {
    // Only the atlas native minimum determines how many resampling-only mips to skip.
    let upscale = (size / minimum.max(1)).max(1).ilog2();
    requested.saturating_add(upscale).min(available - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_texture_budget_picks_the_largest_side_that_fits() {
        // 1300 layers: vanilla's count, about.
        assert_eq!(side_within(512, 1300, DEFAULT_TEXTURE_BUDGET), 128);
        assert_eq!(side_within(512, 1300, 1536 << 20), 512);
        assert_eq!(side_within(64, 1300, 1536 << 20), 64);
        // Never under vanilla's size, whatever the budget.
        assert_eq!(side_within(512, 1300, 1), SPRITE_SIZE);
    }
    #[test]
    fn a_labpbr_map_shrinks_by_plain_means_and_a_layer_without_one_gets_the_default() {
        // Alpha is data here: it does not weigh the other channels.
        let frame = [
            [200, 100, 0, 255],
            [0, 0, 0, 0],
            [200, 100, 0, 255],
            [0, 0, 0, 0],
        ]
        .concat();
        assert_eq!(shrink_map(frame, 2, 1), (1, vec![100, 50, 0, 128]));
        let mut sprites = Sprites {
            native: vec![
                (SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0),
                (SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0),
            ],
            pbr: vec![
                [Some((16, [9, 8, 7, 6].repeat(16 * 16))), None],
                [None, None],
            ],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!(sprites.pbr_size, 16);
        assert_eq!(sprites.normals[..4], [9, 8, 7, 6]);
        assert_eq!(sprites.normals[16 * 16 * 4..][..4], FLAT_NORMAL);
        assert_eq!(sprites.specular[..4], NO_SPECULAR);
        assert_eq!(sprites.specular.len(), 2 * 16 * 16 * 4);
    }
    #[test]
    fn textures_without_labpbr_maps_keep_none() {
        let mut sprites = Sprites {
            native: vec![(SPRITE_SIZE, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0)],
            pbr: vec![[None, None]],
            ..Default::default()
        };
        sprites.finish();
        assert!(sprites.normals.is_empty() && sprites.specular.is_empty());
    }
    #[test]
    fn mipmap_option_keeps_real_16px_reductions() {
        for levels in 0..=4 {
            assert_eq!(sprite_mipmap_limit(levels, 32, 16, 6), levels + 1);
            assert_eq!(sprite_mipmap_limit(levels, 16, 16, 5), levels);
        }
    }
    #[test]
    fn files_outside_textures_load_by_path() {
        let root = std::env::temp_dir().join(format!("horizon-sprite-file-{}", std::process::id()));
        let tile = "assets/minecraft/optifine/ctm/glass/0.png";
        fs::create_dir_all(root.join(tile).parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(File::create(root.join(tile)).unwrap(), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..256).flat_map(|i| [i as u8, 0, 0, 128]).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        let mut sprites = Sprites::default();
        let layer = sprites.insert_file(&root, tile).unwrap();
        assert_eq!(sprites.insert_file(&root, tile).unwrap(), layer);
        assert!(sprites.translucent[layer as usize]);
        // A missing tile is an error, not the missing texture.
        assert!(
            sprites
                .insert_file(&root, "assets/minecraft/optifine/ctm/glass/1.png")
                .is_err()
        );
        assert_eq!(sprites.count(), 1);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn tile_sheets_cut_into_a_layer_per_tile() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-tiles-{}", std::process::id()));
        let file = root.join("assets/minecraft/textures/block/sheet.png");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        // 3 columns of 2 x 2 tiles over 4 rows of pixels: the pixel at (x, y) has red x, green
        // y; the middle column is transparent, and the top-right tile half see-through.
        let (w, h) = (6u32, 4u32);
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                let alpha = match x / 2 {
                    1 => 0,
                    2 if y < 2 => 128,
                    _ => 255,
                };
                [x as u8, y as u8, 0, alpha]
            })
            .collect();
        let mut encoder = png::Encoder::new(File::create(&file).unwrap(), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        // Red and green of each pixel in a layer's frame.
        let texels = |sprites: &Sprites, layer: u32, frame: usize| -> Vec<(u8, u8)> {
            sprites.native[layer as usize].1[frame]
                .chunks(4)
                .map(|p| (p[0], p[1]))
                .collect()
        };
        let mut sprites = Sprites::default();
        let name = "minecraft:block/sheet";
        let own = sprites.insert(&root, name).unwrap();
        // Still, as a 3 x 2 grid: the empty middle column draws nothing.
        let tiles = sprites.insert_tiles(&root, name, (3, 2), false).unwrap();
        // The sheet's own layer now shows the top-left tile, at a tile's size.
        assert_eq!(texels(&sprites, own, 0), [(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert_eq!(
            tiles.iter().map(Option::is_some).collect::<Vec<_>>(),
            [true, false, true, true, false, true]
        );
        let bottom_left = tiles[3].unwrap();
        assert_eq!(
            texels(&sprites, bottom_left, 0),
            [(0, 2), (1, 2), (0, 3), (1, 3)]
        );
        assert!(sprites.translucent[tiles[2].unwrap() as usize]);
        assert!(sprites.opaque[bottom_left as usize]);
        // Animated, as one row of tiles: two frames, the grid's top and then its bottom half;
        // per tile, each cell is the column of a tile's frames, which here is the same.
        fs::write(file.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        for per_tile in [false, true] {
            let mut sprites = Sprites::default();
            let tiles = sprites.insert_tiles(&root, name, (3, 1), per_tile).unwrap();
            let first = tiles[0].unwrap();
            assert_eq!(texels(&sprites, first, 0), [(0, 0), (1, 0), (0, 1), (1, 1)]);
            assert_eq!(texels(&sprites, first, 1), [(0, 2), (1, 2), (0, 3), (1, 3)]);
            assert_eq!(sprites.animations.len(), 2, "two non-empty tiles");
        }
        // Not a grid of square tiles.
        assert!(sprites.insert_tiles(&root, name, (4, 1), false).is_err());
        assert!(sprites.insert_tiles(&root, name, (3, 3), false).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn custom_item_texture_ids_load_with_their_animation() {
        let root = std::env::temp_dir().join(format!("horizon-sprite-cit-{}", std::process::id()));
        let file = root.join("assets/minecraft/optifine/cit/a.png");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(File::create(&file).unwrap(), 16, 48);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..16 * 48).flat_map(|_| [200, 10, 10, 255]).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        fs::write(file.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        let mut sprites = Sprites::default();
        sprites
            .insert(&root, "horizon_cit:minecraft/optifine/cit/a")
            .unwrap();
        fs::remove_dir_all(&root).unwrap();
        // The missing texture would not animate: this one is the file, in three frames.
        assert_eq!(sprites.animations.len(), 1);
        assert_eq!(sprites.animations[0].frame_count(), 3);
    }
    #[test]
    fn custom_item_textures_skip_the_disk_cache() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-cit-disk-{}", std::process::id()));
        // A realistic root: the cache's fingerprint walks the `textures` trees.
        let vanilla = root.join("assets/minecraft/textures/item/stick.png");
        let file = root.join("assets/minecraft/optifine/cit/a.png");
        let write = |path: &Path, red: u8| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(path).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..256).flat_map(|_| [red, 0, 0, 255]).collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write(&vanilla, 1);
        write(&file, 10);
        let id = "horizon_cit:minecraft/optifine/cit/a";
        let red = |sprites: &Sprites, layer: u32| sprites.native[layer as usize].1[0][0];
        let mut first = Sprites::with_disk_cache(&root);
        let layer = first.insert(&root, id).unwrap();
        assert_eq!(red(&first, layer), 10);
        first.save_disk_cache();
        // The pack's file changes; `textures/` does not, so a cached copy would still match.
        write(&file, 20);
        let mut second = Sprites::with_disk_cache(&root);
        let layer = second.insert(&root, id).unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(red(&second, layer), 20);
    }
    #[test]
    fn custom_item_textures_mip_as_items() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-cit-mip-{}", std::process::id()));
        let write = |rel: &str| {
            let path = root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(path).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            // Any pixels: whether a texture mips as an item is part of the shared-layer key.
            let pixels: Vec<u8> = (0..256)
                .flat_map(|i| [9, 9, 9, if i % 2 == 0 { 255 } else { 0 }])
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write("assets/a/textures/block/x.png");
        write("assets/a/textures/item/x.png");
        write("assets/minecraft/optifine/cit/x.png");
        let mut sprites = Sprites::default();
        let block = sprites.insert(&root, "a:block/x").unwrap();
        let item = sprites.insert(&root, "a:item/x").unwrap();
        let cit = sprites
            .insert(&root, "horizon_cit:minecraft/optifine/cit/x")
            .unwrap();
        fs::remove_dir_all(&root).unwrap();
        // The same pixels share a layer only with the same mipmap settings: the CIT texture's
        // are the item's.
        assert_ne!(block, item);
        assert_eq!(cit, item);
    }
    #[test]
    fn prefetched_textures_take_the_layers_inserting_alone_gives() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-prefetch-{}", std::process::id()));
        let png = |path: &str, side: u32, seed: u8| {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            let mut encoder = png::Encoder::new(File::create(file).unwrap(), side, side);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..side * side)
                .flat_map(|i| {
                    [
                        seed,
                        i as u8,
                        (i >> 8) as u8,
                        if i % 3 == 0 { 0 } else { 255 },
                    ]
                })
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        for i in 0..40u8 {
            png(
                &format!("assets/minecraft/textures/block/b{i}.png"),
                16 << (i % 3),
                i,
            );
        }
        // Two textures alike share a layer; a tile outside `textures/`, a broken file and a
        // missing one take the paths they take without prefetching.
        png("assets/minecraft/textures/block/same.png", 16, 3);
        png("assets/minecraft/optifine/ctm/glass/0.png", 32, 77);
        fs::write(
            root.join("assets/minecraft/textures/block/broken.png"),
            b"no png",
        )
        .unwrap();
        let names: Vec<String> = (0..40)
            .map(|i| format!("minecraft:block/b{}", (i * 7) % 40))
            .chain(["minecraft:block/same", "minecraft:block/broken"].map(String::from))
            .chain(["minecraft:block/missing", "minecraft:block/b3"].map(String::from))
            .collect();
        let tile = "assets/minecraft/optifine/ctm/glass/0.png";
        let load = |prefetch: bool| {
            let mut sprites = Sprites::default();
            if prefetch {
                sprites.prefetch(&root, names.iter().map(String::as_str).chain([tile]));
            }
            let mut layers: Vec<u32> = names
                .iter()
                .map(|name| sprites.insert(&root, name).unwrap())
                .collect();
            layers.push(sprites.insert_file(&root, tile).unwrap());
            sprites.finish();
            (
                layers,
                sprites.layers,
                sprites.pixels,
                sprites.translucent,
                sprites.size,
            )
        };
        let alone = load(false);
        assert_eq!(load(true), alone);
        // The textures alike and the broken one's missing texture: fewer layers than names.
        assert!(alone.1.values().max().unwrap() + 1 < names.len() as u32);
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn identical_still_textures_share_a_layer() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-share-{}", std::process::id()));
        let directory = root.join("assets/a/textures/block");
        fs::create_dir_all(&directory).unwrap();
        let write = |name: &str, red: u8| {
            let mut encoder =
                png::Encoder::new(File::create(directory.join(name)).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..256).flat_map(|i| [red, i as u8, 0, 255]).collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write("one.png", 10);
        write("copy.png", 10);
        write("other.png", 20);
        let mut sprites = Sprites::default();
        let one = sprites.insert(&root, "a:block/one").unwrap();
        let copy = sprites.insert(&root, "a:block/copy").unwrap();
        let other = sprites.insert(&root, "a:block/other").unwrap();
        // The same pixels as an item sprite mip differently: a layer of its own.
        fs::create_dir_all(root.join("assets/a/textures/item")).unwrap();
        fs::copy(
            directory.join("one.png"),
            root.join("assets/a/textures/item/one.png"),
        )
        .unwrap();
        let item = sprites.insert(&root, "a:item/one").unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!((one, copy, other, item), (0, 0, 1, 2));
        assert_eq!((sprites.count(), sprites.layers.len()), (3, 4));
        assert_eq!((sprites.opaque.len(), sprites.mips.len()), (3, 3));
        sprites.finish();
        assert_eq!(sprites.pixels.len(), 3 * 16 * 16 * 4);
    }
    #[test]
    fn a_32px_only_atlas_does_not_add_an_extra_mipmap() {
        let mut sprites = Sprites {
            native: vec![
                (32, Arc::new(vec![vec![255; 32 * 32 * 4]]), 0),
                (32, Arc::new(vec![vec![128; 32 * 32 * 4]]), 0),
            ],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!((sprites.size, sprites.minimum_size), (32, 32));
        for level in 0..=4 {
            assert_eq!(
                sprite_mipmap_limit(level, sprites.size, sprites.minimum_size, 6),
                level
            );
        }
        sprites
            .native
            .push((16, Arc::new(vec![vec![255; 16 * 16 * 4]]), 0));
        sprites.finish();
        assert_eq!((sprites.size, sprites.minimum_size), (32, 16));
        assert_eq!(
            sprite_mipmap_limit(0, sprites.size, sprites.minimum_size, 6),
            1
        );
    }

    #[test]
    fn hud_animation_resampling_uses_the_actual_frame_size() {
        for size in [16, 32, 64] {
            let animation = test_animation(size, 0);
            let resized = animation.resized_pixels(1, 16, 16);
            let last = (15 * size / 16) as u8;
            assert_eq!(&resized[15 * 4..16 * 4], &[last, 0, 99, 255]);
            assert_eq!(&resized[255 * 4..256 * 4], &[last, last, 99, 255]);
        }
    }

    #[test]
    fn native_fire_sprites_register_full_animation_sequences() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root
            .join("assets/minecraft/textures/block/fire_0.png")
            .exists()
        {
            return;
        }
        let mut sprites = Sprites::default();
        for name in ["fire_0", "fire_1", "soul_fire_0", "soul_fire_1"] {
            let layer = sprites
                .insert(&root, &format!("minecraft:block/{name}"))
                .unwrap();
            let animation = sprites
                .animations
                .iter()
                .find(|a| a.layer == layer)
                .expect("fire must be animated");
            assert_ne!(
                animation.pixels(animation.frame(0)),
                animation.pixels(animation.frame(1))
            );
            assert_ne!(
                animation.resized_pixels(animation.frame(0), 16, 16),
                animation.resized_pixels(animation.frame(1), 16, 16)
            );
            assert_eq!(
                animation.pixels(animation.frame(0)).len(),
                (SPRITE_SIZE * SPRITE_SIZE * 4) as usize
            );
        }
    }

    #[test]
    fn native_32_pixel_animation_frames_keep_every_texel() {
        let root = std::env::temp_dir().join(format!(
            "horizon-sprite-detail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = root.join("assets/test/textures");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("flow.png");
        let mut pixels = Vec::new();
        for y in 0..64u8 {
            for x in 0..32u8 {
                pixels.extend([x, y, 0, 255]);
            }
        }
        let mut encoder = png::Encoder::new(File::create(&path).unwrap(), 32, 64);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        fs::write(path.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        let mut sprites = Sprites::default();
        sprites.insert(&root, "test:flow").unwrap();
        sprites.finish();
        let size = sprites.size;
        let initial = sprites.pixels.clone();
        let animation = sprites.animations[0].clone();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            size, 32,
            "half a flowing-water frame must retain 16 texels, not 8"
        );
        assert_eq!(initial, pixels[..32 * 32 * 4]);
        assert_eq!(animation.pixels(animation.frame(1)), &pixels[32 * 32 * 4..]);
    }

    #[test]
    fn animation_boundaries_and_repeated_frames_do_not_depend_on_render_rate() {
        let animation = SpriteAnimation {
            layer: 0,
            frames: Arc::new(vec![vec![], vec![], vec![]]),
            clip: Arc::new(AnimationMetadata::from_json(r#"{"frames":[{"index":2,"time":8},{"index":1,"time":3},{"index":1,"time":2},0]}"#).unwrap().clip(3).unwrap().unwrap()),
        };
        for (tick, frame) in [
            (0, 2),
            (7, 2),
            (8, 1),
            (10, 1),
            (11, 1),
            (12, 1),
            (13, 0),
            (14, 2),
            (29, 2),
            (u64::MAX, 2),
        ] {
            assert_eq!(animation.frame(tick), frame);
        }
    }

    #[test]
    fn interpolated_animations_blend_towards_the_next_frame() {
        let animation = test_clip(16, 0, r#"{"frametime":4,"interpolate":true}"#);
        let blend = |frame, next, millis| FrameBlend {
            frame,
            next,
            millis,
        };
        for (tick, expected) in [
            (0, blend(0, 0, 0)),
            (1, blend(0, 1, 250)),
            (3, blend(0, 1, 750)),
            (4, blend(1, 1, 0)),
            (5, blend(1, 0, 250)),
            (8, blend(0, 0, 0)),
        ] {
            assert_eq!(animation.blend(tick), expected, "tick {tick}");
        }
        let stepped = test_clip(16, 0, r#"{"frametime":4}"#);
        assert_eq!(stepped.blend(1), blend(0, 0, 0));
        assert_eq!(stepped.blend(5), blend(1, 1, 0));
    }
}
