//! Decoded block sprites kept on disk between launches (M-11): reading and
//! decoding ~1300 small PNGs is most of `Sprites::load`. One file per resource
//! root, valid only while the `textures` trees still have the same paths,
//! sizes and modification times and the format version is the same; anything
//! else (missing, corrupt, stale, unwritable) means the normal decoding path.
use crate::mipmap::Strategy;
use std::{
    collections::HashMap,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 4] = b"HSPR";
/// Bump when `Decoded` or the decoding in `Sprites::decode` changes.
const VERSION: u32 = 1;
const FILE: &str = "horizon-sprites.cache";

/// What decoding one texture gives, before it takes a layer.
#[derive(Clone)]
pub(crate) struct Decoded {
    pub side: u32,
    pub frames: Vec<Vec<u8>>,
    /// The `.mcmeta` `animation` value, as text.
    pub animation: Option<String>,
    pub strategy: Strategy,
    pub alpha_cutoff_bias: f32,
    pub has_transparent: bool,
    pub opaque: bool,
    pub translucent: bool,
}

pub(crate) struct SpriteCache {
    path: PathBuf,
    fingerprint: u64,
    entries: HashMap<String, Decoded>,
    dirty: bool,
}

/// Paths, sizes and modification times of every file under `assets/*/textures`,
/// summed so the directory order does not matter.
fn fingerprint(root: &Path) -> Option<u64> {
    fn walk(dir: &Path, base: &Path, sum: &mut u64) -> Option<()> {
        for entry in fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let meta = entry.metadata().ok()?;
            if meta.is_dir() {
                walk(&entry.path(), base, sum)?;
                continue;
            }
            let mut hasher = DefaultHasher::new();
            entry.path().strip_prefix(base).ok()?.hash(&mut hasher);
            meta.len().hash(&mut hasher);
            meta.modified().ok()?.hash(&mut hasher);
            *sum = sum.wrapping_add(hasher.finish());
        }
        Some(())
    }
    let mut sum = 0;
    for namespace in fs::read_dir(root.join("assets")).ok()? {
        let textures = namespace.ok()?.path().join("textures");
        if textures.is_dir() {
            walk(&textures, root, &mut sum)?;
        }
    }
    Some(sum)
}

impl SpriteCache {
    /// The cache of `root`: its stored entries when still valid, else empty.
    /// `None` when the textures cannot be fingerprinted (no caching then).
    pub fn open(root: &Path) -> Option<Self> {
        let fingerprint = fingerprint(root)?;
        let path = root.join(FILE);
        let entries = fs::read(&path)
            .ok()
            .and_then(|bytes| parse(&bytes, fingerprint))
            .unwrap_or_default();
        Some(Self {
            path,
            fingerprint,
            entries,
            dirty: false,
        })
    }
    pub fn get(&self, name: &str) -> Option<&Decoded> {
        self.entries.get(name)
    }
    pub fn insert(&mut self, name: &str, decoded: Decoded) {
        self.entries.insert(name.to_owned(), decoded);
        self.dirty = true;
    }
    /// Write new entries; a failure only costs the next launch the decoding.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        if fs::write(&temporary, serialize(self.fingerprint, &self.entries)).is_ok()
            && fs::rename(&temporary, &self.path).is_err()
        {
            let _ = fs::remove_file(&temporary);
        }
    }
}

fn serialize(fingerprint: u64, entries: &HashMap<String, Decoded>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&fingerprint.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (name, d) in entries {
        put_bytes(&mut out, name.as_bytes());
        out.extend_from_slice(&d.side.to_le_bytes());
        out.extend_from_slice(&(d.frames.len() as u32).to_le_bytes());
        for frame in &d.frames {
            put_bytes(&mut out, frame);
        }
        put_bytes(&mut out, d.animation.as_deref().unwrap_or("").as_bytes());
        out.push(u8::from(d.animation.is_some()));
        out.push(d.strategy as u8);
        out.extend_from_slice(&d.alpha_cutoff_bias.to_bits().to_le_bytes());
        out.push(
            u8::from(d.has_transparent) | u8::from(d.opaque) << 1 | u8::from(d.translucent) << 2,
        );
    }
    out
}
fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
}
fn parse(bytes: &[u8], fingerprint: u64) -> Option<HashMap<String, Decoded>> {
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    if u64::from_le_bytes(r.take(8)?.try_into().ok()?) != fingerprint {
        return None;
    }
    let count = r.u32()?;
    let mut entries = HashMap::new();
    for _ in 0..count {
        let name = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let side = r.u32()?;
        let frame_count = r.u32()?;
        let mut frames = Vec::new();
        for _ in 0..frame_count {
            frames.push(r.bytes()?.to_vec());
        }
        let animation = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        let has_animation = r.take(1)?[0] != 0;
        let strategy = match r.take(1)?[0] {
            0 => Strategy::Auto,
            1 => Strategy::Mean,
            2 => Strategy::Cutout,
            3 => Strategy::StrictCutout,
            4 => Strategy::DarkCutout,
            _ => return None,
        };
        let alpha_cutoff_bias = f32::from_bits(r.u32()?);
        let flags = r.take(1)?[0];
        let frame_len = (side as usize).checked_mul(side as usize)?.checked_mul(4)?;
        if frames.is_empty() || frames.iter().any(|f| f.len() != frame_len) {
            return None;
        }
        entries.insert(
            name,
            Decoded {
                side,
                frames,
                animation: has_animation.then_some(animation),
                strategy,
                alpha_cutoff_bias,
                has_transparent: flags & 1 != 0,
                opaque: flags & 2 != 0,
                translucent: flags & 4 != 0,
            },
        );
    }
    r.0.is_empty().then_some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(blue: u8, animation: Option<&str>) -> Decoded {
        Decoded {
            side: 2,
            frames: vec![vec![blue; 16], vec![blue + 1; 16]],
            animation: animation.map(str::to_owned),
            strategy: Strategy::Cutout,
            alpha_cutoff_bias: 0.25,
            has_transparent: true,
            opaque: false,
            translucent: true,
        }
    }
    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-sprite-cache-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets/minecraft/textures/block")).unwrap();
        dir
    }
    #[test]
    fn round_trips() {
        let mut entries = HashMap::new();
        entries.insert("a:b".to_owned(), sample(1, Some("{\"frametime\":2}")));
        entries.insert("a:c".to_owned(), sample(9, None));
        let back = parse(&serialize(7, &entries), 7).unwrap();
        for (name, d) in &entries {
            let e = &back[name];
            assert_eq!(
                (d.side, &d.frames, &d.animation),
                (e.side, &e.frames, &e.animation)
            );
            assert_eq!(d.alpha_cutoff_bias, e.alpha_cutoff_bias);
            assert_eq!(
                (d.has_transparent, d.opaque, d.translucent, d.strategy as u8),
                (e.has_transparent, e.opaque, e.translucent, e.strategy as u8)
            );
        }
    }
    #[test]
    fn wrong_fingerprint_truncation_and_garbage_are_misses() {
        let mut entries = HashMap::new();
        entries.insert("a:b".to_owned(), sample(1, None));
        let bytes = serialize(7, &entries);
        assert!(parse(&bytes, 8).is_none());
        for cut in 0..bytes.len() {
            assert!(parse(&bytes[..cut], 7).is_none(), "cut {cut}");
        }
        assert!(parse(b"garbage", 7).is_none());
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(parse(&extra, 7).is_none());
    }
    #[test]
    fn changed_texture_invalidates_and_corrupt_file_is_ignored() {
        let root = scratch("invalidate");
        let png = root.join("assets/minecraft/textures/block/x.png");
        fs::write(&png, b"one").unwrap();
        let mut cache = SpriteCache::open(&root).unwrap();
        cache.insert("minecraft:block/x", sample(1, None));
        cache.save();
        assert!(
            SpriteCache::open(&root)
                .unwrap()
                .get("minecraft:block/x")
                .is_some()
        );
        // Same length, new content and time: still stale.
        std::thread::sleep(std::time::Duration::from_millis(30));
        fs::write(&png, b"two").unwrap();
        assert!(
            SpriteCache::open(&root)
                .unwrap()
                .get("minecraft:block/x")
                .is_none()
        );
        // A new file also invalidates.
        let mut cache = SpriteCache::open(&root).unwrap();
        cache.insert("minecraft:block/x", sample(1, None));
        cache.save();
        fs::write(root.join("assets/minecraft/textures/block/y.png"), b"y").unwrap();
        assert!(
            SpriteCache::open(&root)
                .unwrap()
                .get("minecraft:block/x")
                .is_none()
        );
        // Corrupt cache file: empty, no error.
        fs::write(root.join(FILE), b"HSPR junk").unwrap();
        assert!(
            SpriteCache::open(&root)
                .unwrap()
                .get("minecraft:block/x")
                .is_none()
        );
        let _ = fs::remove_dir_all(&root);
    }
    fn write_png(path: &Path, rgba: [u8; 4]) {
        let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = rgba.iter().copied().cycle().take(16 * 16 * 4).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    fn sprite_pixels(root: &Path) -> Vec<u8> {
        let mut sprites = crate::sprites::Sprites::with_disk_cache(root);
        sprites.insert(root, "minecraft:block/x").unwrap();
        sprites.save_disk_cache();
        sprites.finish();
        sprites.pixels
    }
    #[test]
    fn altered_png_is_not_served_stale_and_cache_matches_decoding() {
        let root = scratch("stale");
        let png = root.join("assets/minecraft/textures/block/x.png");
        write_png(&png, [255, 0, 0, 255]);
        let decoded = sprite_pixels(&root);
        assert!(root.join(FILE).exists());
        assert_eq!(
            sprite_pixels(&root),
            decoded,
            "cache hit differs from decoding"
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        write_png(&png, [0, 255, 0, 255]);
        let fresh = sprite_pixels(&root);
        assert_eq!(&fresh[..4], &[0, 255, 0, 255]);
        // Corrupt and deleted caches fall back to decoding.
        fs::write(root.join(FILE), b"HSPR\x01\0\0\0 junk").unwrap();
        assert_eq!(sprite_pixels(&root), fresh);
        fs::remove_file(root.join(FILE)).unwrap();
        assert_eq!(sprite_pixels(&root), fresh);
        let _ = fs::remove_dir_all(&root);
    }
}
