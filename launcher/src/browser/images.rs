//! Project icons and gallery images, fetched at run time from the sources' image servers and
//! kept on disk: nothing of a project's art is in the launcher. A URL is fetched only over HTTPS
//! from Modrinth's or CurseForge's CDN, at most [`MAX_DOWNLOAD`] bytes, decoded (PNG, JPEG, WebP,
//! GIF), scaled down and stored as a PNG the UI draws; the cache is kept under [`MAX_CACHE`] by
//! dropping the oldest files. After [`FRESH`] an image is asked for again with its server's
//! validators (`If-None-Match`, `If-Modified-Since`): unchanged, it costs one small answer.
use sha1::{Digest, Sha1};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Hosts images may come from.
const HOSTS: [&str; 7] = [
    "https://cdn.modrinth.com/",
    // Fabric's and Forge's own icons, for instances that play them (no published logo licence:
    // drawn from their sites, never shipped).
    "https://raw.githubusercontent.com/FabricMC/fabric-loader/",
    "https://files.minecraftforge.net/static/images/",
    "https://media.forgecdn.net/",
    "https://edge.forgecdn.net/",
    // Horizon's registry's icons (its GitHub Pages), and the mirror kept at the address it had
    // before the repositories moved to the horizon-rust organization.
    "https://horizon-rust.github.io/horizon-mods/",
    "https://ludovic111.github.io/horizon-mods/",
];
/// The most an image may weigh before decoding.
pub const MAX_DOWNLOAD: u64 = 8 << 20;
/// The most the cache keeps on disk.
pub const MAX_CACHE: u64 = 128 << 20;
/// Pixels: an icon's side, a gallery image's longest side.
pub const ICON: u32 = 128;
pub const GALLERY: u32 = 960;
/// A list row's icon: 40 points, drawn at twice that on a Retina screen.
pub const ROW: u32 = 80;
/// How long a cached image is used without asking its server whether it changed.
pub const FRESH: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

fn dir(home: &Path) -> PathBuf {
    home.join("mod-browser/images")
}

/// Why `url` is never fetched.
pub fn refuse(url: &str) -> Option<&'static str> {
    if !HOSTS.iter().any(|h| url.starts_with(h)) {
        return Some("not on Modrinth's or CurseForge's image servers over HTTPS");
    }
    if url.len() > 2048 || url.contains(char::is_whitespace) {
        return Some("not a plain URL");
    }
    None
}

/// Where the image of `url` scaled to `size` is kept.
pub fn path(home: &Path, url: &str, size: u32) -> PathBuf {
    let hash: String = Sha1::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    dir(home).join(format!("{hash}-{size}.png"))
}

/// The image if it is cached already.
pub fn cached(home: &Path, url: &str, size: u32) -> Option<PathBuf> {
    let p = path(home, url, size);
    p.is_file().then_some(p)
}

/// Whether the cached image of `url` is older than [`FRESH`]: [`fetch`] asks its server again.
pub fn stale(home: &Path, url: &str, size: u32) -> bool {
    fs::metadata(path(home, url, size))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > FRESH)
}

/// The server's validators of a cached image (its `ETag`, then its `Last-Modified`), beside it.
fn validators(image: &Path) -> PathBuf {
    image.with_extension("tag")
}

/// The image of `url`, no larger than `size` pixels a side: the cached one while fresh, else
/// fetched (with the cached one's validators: an unchanged one is kept), decoded, scaled and
/// stored.
pub fn fetch(home: &Path, url: &str, size: u32) -> Result<PathBuf, String> {
    let target = path(home, url, size);
    let cached = target.is_file();
    if cached && !stale(home, url, size) {
        return Ok(target);
    }
    if let Some(why) = refuse(url) {
        return Err(format!("{url}: {why}"));
    }
    let mut request = super::download::agent(url).get(url);
    if cached {
        let tags = fs::read_to_string(validators(&target)).unwrap_or_default();
        let mut tags = tags.lines();
        if let Some(etag) = tags.next().filter(|t| !t.is_empty()) {
            request = request.set("If-None-Match", etag);
        }
        if let Some(modified) = tags.next().filter(|t| !t.is_empty()) {
            request = request.set("If-Modified-Since", modified);
        }
    }
    let response = request.call().map_err(|e| format!("{url}: {e}"))?;
    if cached && response.status() == 304 {
        // Unchanged: fresh for another while.
        let _ = fs::File::options()
            .write(true)
            .open(&target)
            .and_then(|f| f.set_modified(std::time::SystemTime::now()));
        return Ok(target);
    }
    let tags = format!(
        "{}\n{}\n",
        response.header("ETag").unwrap_or_default(),
        response.header("Last-Modified").unwrap_or_default()
    );
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_DOWNLOAD + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{url}: {e}"))?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err(format!("{url}: larger than {} MB", MAX_DOWNLOAD >> 20));
    }
    let stored = store(home, url, size, &bytes)?;
    let _ = fs::write(validators(&stored), tags);
    Ok(stored)
}

/// Decodes `bytes`, scales them to fit `size` and writes the PNG.
pub fn store(home: &Path, url: &str, size: u32, bytes: &[u8]) -> Result<PathBuf, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 << 20);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| format!("{url}: {e}"))?;
    let scaled = if decoded.width() > size || decoded.height() > size {
        decoded.resize(size, size, image::imageops::FilterType::Triangle)
    } else {
        decoded
    };
    let target = path(home, url, size);
    let folder = dir(home);
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let part = target.with_extension("part");
    scaled
        .save_with_format(&part, image::ImageFormat::Png)
        .map_err(|e| format!("{}: {e}", part.display()))?;
    fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    prune(&folder, MAX_CACHE);
    Ok(target)
}

/// Drops the oldest files until the folder holds at most `max` bytes.
fn prune(folder: &Path, max: u64) {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= max {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// The most a jar's icon may weigh: a mod's own logo, never a texture atlas.
const MAX_JAR_ICON: u64 = 2 << 20;

/// The icon a mod's jar names for itself: Fabric's `fabric.mod.json` and Quilt's `quilt.mod.json`
/// (`"icon"`: a path, or paths by size, the largest taken), Forge's and NeoForge's `mods.toml`
/// (`logoFile`). The path inside the jar.
pub fn jar_icon_name(
    fabric: Option<&str>,
    quilt: Option<&str>,
    toml: Option<&str>,
) -> Option<String> {
    let pick = |icon: &serde_json::Value| -> Option<String> {
        match icon {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(sizes) => sizes
                .iter()
                .filter_map(|(size, path)| Some((size.parse::<u32>().ok()?, path.as_str()?)))
                .max_by_key(|(size, _)| *size)
                .map(|(_, path)| path.to_owned()),
            _ => None,
        }
    };
    let json =
        |text: Option<&str>| -> Option<serde_json::Value> { serde_json::from_str(text?).ok() };
    let named = json(fabric)
        .and_then(|v| pick(&v["icon"]))
        .or_else(|| json(quilt).and_then(|v| pick(&v["quilt_loader"]["metadata"]["icon"])))
        .or_else(|| {
            toml?.lines().find_map(|line| {
                let value = line
                    .trim()
                    .strip_prefix("logoFile")?
                    .trim()
                    .strip_prefix('=')?;
                Some(value.trim().trim_matches('"').trim_matches('\'').to_owned())
            })
        })?;
    let named = named.trim_start_matches('/').to_owned();
    (!named.is_empty() && !named.contains("..")).then_some(named)
}

/// A mod jar's own icon, scaled to fit `side` pixels, as PNG bytes: read from the jar, never
/// fetched. None when the jar names none or it isn't an image.
pub fn jar_icon(jar: &Path, side: u32) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(fs::File::open(jar).ok()?).ok()?;
    let mut read = |name: &str, max: u64| -> Option<Vec<u8>> {
        let file = zip.by_name(name).ok()?;
        let mut bytes = Vec::new();
        file.take(max + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= max).then_some(bytes)
    };
    let text = |b: Option<Vec<u8>>| b.and_then(|b| String::from_utf8(b).ok());
    let fabric = text(read("fabric.mod.json", 1 << 20));
    let quilt = text(read("quilt.mod.json", 1 << 20));
    let toml = text(read("META-INF/neoforge.mods.toml", 1 << 20))
        .or_else(|| text(read("META-INF/mods.toml", 1 << 20)));
    let name = jar_icon_name(fabric.as_deref(), quilt.as_deref(), toml.as_deref())?;
    fit_png(read(&name, MAX_JAR_ICON)?, side, 4096, 64 << 20)
}

/// `bytes` decoded (at most `max_side` pixels a side and `max_alloc` bytes), scaled down to fit
/// `side` pixels, as PNG bytes. None when they aren't an image within those limits.
pub fn fit_png(bytes: Vec<u8>, side: u32, max_side: u32, max_alloc: u64) -> Option<Vec<u8>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    limits.max_alloc = Some(max_alloc);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let scaled = if decoded.width() > side || decoded.height() > side {
        decoded.resize(side, side, image::imageops::FilterType::Triangle)
    } else {
        decoded
    };
    let mut png = Vec::new();
    scaled
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// Vanilla Minecraft's mark: a grass block drawn from the player's own prepared game files
/// (`<resources>/assets/minecraft/textures/block/`), never shipped. Kept beside the cache.
pub fn grass_block(home: &Path, resources: &Path) -> Option<PathBuf> {
    let target = dir(home).join("grass-block.png");
    if target.is_file() {
        return Some(target);
    }
    let textures = resources.join("assets/minecraft/textures/block");
    let load = |name: &str| image::open(textures.join(name)).ok().map(|i| i.to_rgba8());
    let (top, side, overlay) = (
        load("grass_block_top.png")?,
        load("grass_block_side.png")?,
        load("grass_block_side_overlay.png")?,
    );
    let icon = draw_block(&top, &side, &overlay);
    fs::create_dir_all(dir(home)).ok()?;
    icon.save(&target).ok()?;
    Some(target)
}

/// The plains grass colour vanilla's grass block item shows (`GrassColor.getDefaultColor`
/// is the colormap at 0.5, 1.0: `#91BD59`).
const GRASS: [u8; 3] = [0x91, 0xBD, 0x59];

/// An isometric block, 64 pixels a side, from 16x16 faces: the top tinted, the sides with their
/// tinted overlay, the left side shaded as the game shades a side facing away from the light.
fn draw_block(
    top: &image::RgbaImage,
    side: &image::RgbaImage,
    overlay: &image::RgbaImage,
) -> image::RgbaImage {
    let size = 64u32;
    let mut out = image::RgbaImage::new(size, size);
    let at = |img: &image::RgbaImage, u: f32, v: f32| {
        let (w, h) = (img.width().min(16), img.height().min(16));
        let x = ((u * w as f32) as u32).min(w - 1);
        let y = ((v * h as f32) as u32).min(h - 1);
        *img.get_pixel(x, y)
    };
    let tint = |p: image::Rgba<u8>, shade: f32| {
        let mut q = p;
        for c in 0..3 {
            q[c] = (p[c] as f32 * GRASS[c] as f32 / 255. * shade) as u8;
        }
        q
    };
    let shade = |p: image::Rgba<u8>, s: f32| {
        let mut q = p;
        for c in 0..3 {
            q[c] = (p[c] as f32 * s) as u8;
        }
        q
    };
    let side_px = |u: f32, v: f32, s: f32| {
        let o = at(overlay, u, v);
        if o[3] > 0 {
            tint(o, s)
        } else {
            shade(at(side, u, v), s)
        }
    };
    // The cube: the top face a diamond, two sides below it.
    let (cx, h) = (size as f32 / 2., size as f32 / 4.);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = (px - cx) / cx;
            let dy = (py - h) / h;
            let u = (dy + dx) / 2. + 0.5;
            let v = (dy - dx) / 2. + 0.5;
            let pixel = if (0. ..1.).contains(&u) && (0. ..1.).contains(&v) {
                Some(tint(at(top, u, v), 1.0))
            } else if px < cx {
                let u = px / cx;
                let v = (py - h - u * h) / (2. * h);
                (0. ..1.).contains(&v).then(|| side_px(u, v, 0.8))
            } else {
                let u = (px - cx) / cx;
                let v = (py - 2. * h + u * h) / (2. * h);
                (0. ..1.).contains(&v).then(|| side_px(u, v, 0.6))
            };
            if let Some(p) = pixel {
                out.put_pixel(x, y, p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jar_names_its_icon() {
        let fabric = r#"{"id": "lumen", "icon": "assets/lumen/icon.png"}"#;
        assert_eq!(
            jar_icon_name(Some(fabric), None, None).as_deref(),
            Some("assets/lumen/icon.png")
        );
        let sizes = r#"{"icon": {"16": "small.png", "128": "big.png", "64": "mid.png"}}"#;
        assert_eq!(
            jar_icon_name(Some(sizes), None, None).as_deref(),
            Some("big.png")
        );
        let quilt = r#"{"quilt_loader": {"metadata": {"icon": "/q.png"}}}"#;
        assert_eq!(
            jar_icon_name(None, Some(quilt), None).as_deref(),
            Some("q.png")
        );
        let toml = "modLoader=\"javafml\"\n[[mods]]\nmodId=\"x\"\nlogoFile = \"logo.png\"\n";
        assert_eq!(
            jar_icon_name(None, None, Some(toml)).as_deref(),
            Some("logo.png")
        );
        assert_eq!(
            jar_icon_name(Some(r#"{"icon": "../x.png"}"#), None, None),
            None
        );
        assert_eq!(jar_icon_name(Some("{}"), None, None), None);
    }

    #[test]
    fn a_jar_icon_is_scaled_to_the_row() {
        let dir = std::env::temp_dir().join(format!("jar-icon-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("lumen.jar");
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(256, 256, image::Rgba([200, 80, 40, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("fabric.mod.json", options).unwrap();
        std::io::Write::write_all(&mut zip, br#"{"icon": "icon.png"}"#).unwrap();
        zip.start_file("icon.png", options).unwrap();
        std::io::Write::write_all(&mut zip, &png).unwrap();
        zip.finish().unwrap();
        let icon = jar_icon(&jar, ROW).expect("the jar's icon");
        let decoded = image::load_from_memory(&icon).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (ROW, ROW));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_goes_stale_after_a_while() {
        let home = std::env::temp_dir().join(format!("stale-{}", std::process::id()));
        let url = "https://cdn.modrinth.com/data/x/icon.png";
        let img = path(&home, url, ROW);
        fs::create_dir_all(img.parent().unwrap()).unwrap();
        fs::write(&img, b"png").unwrap();
        assert!(!stale(&home, url, ROW));
        let old = std::time::SystemTime::now() - FRESH - std::time::Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&img)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(stale(&home, url, ROW));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn only_the_sources_image_servers() {
        assert!(refuse("https://cdn.modrinth.com/data/AANobbMI/icon.png").is_none());
        assert!(refuse("https://media.forgecdn.net/avatars/1/2.png").is_none());
        assert!(refuse("http://cdn.modrinth.com/data/x/icon.png").is_some());
        assert!(refuse("https://cdn.modrinth.com.evil.example/x.png").is_some());
        assert!(refuse("https://example.com/x.png").is_some());
    }

    #[test]
    fn images_are_scaled_stored_and_pruned() {
        let home = std::env::temp_dir().join(format!("horizon-images-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let big = image::RgbaImage::from_pixel(300, 150, image::Rgba([1, 2, 3, 255]));
        let mut png = Vec::new();
        big.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let url = "https://cdn.modrinth.com/data/x/icon.png";
        let p = store(&home, url, ICON, &png).unwrap();
        assert_eq!(cached(&home, url, ICON), Some(p.clone()));
        assert_eq!(image::open(&p).unwrap().width(), ICON);
        assert!(store(&home, url, ICON, b"not an image").is_err());
        prune(&dir(&home), 0);
        assert!(cached(&home, url, ICON).is_none());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_grass_block_from_faces() {
        let face = |c: [u8; 4]| image::RgbaImage::from_pixel(16, 16, image::Rgba(c));
        let icon = draw_block(
            &face([200, 200, 200, 255]),
            &face([120, 80, 50, 255]),
            &face([0, 0, 0, 0]),
        );
        // The top is tinted grass, the sides dirt shaded, the corners empty.
        assert_eq!(icon.get_pixel(32, 16)[1], (200. * 0xBD as f32 / 255.) as u8);
        assert_eq!(icon.get_pixel(4, 40).0, [96, 64, 40, 255]);
        assert_eq!(icon.get_pixel(60, 40).0, [72, 48, 30, 255]);
        assert_eq!(icon.get_pixel(1, 1)[3], 0);
    }
}
