//! Players' heads for the Servers page: the face of the skin the game shows for them. Without the skin
//! service a player has vanilla's default skin for their UUID (`DefaultPlayerSkin.get`), read from the
//! player's own prepared game files.
use std::path::Path;

/// `DefaultPlayerSkin.get(UUID)`: the default skin by `UUID.hashCode` (as the game's tab list picks it).
pub fn default_skin(uuid: u128) -> &'static str {
    const SKINS: [&str; 18] = [
        "slim/alex",
        "slim/ari",
        "slim/efe",
        "slim/kai",
        "slim/makena",
        "slim/noor",
        "slim/steve",
        "slim/sunny",
        "slim/zuri",
        "wide/alex",
        "wide/ari",
        "wide/efe",
        "wide/kai",
        "wide/makena",
        "wide/noor",
        "wide/steve",
        "wide/sunny",
        "wide/zuri",
    ];
    let hilo = ((uuid >> 64) as u64) ^ (uuid as u64);
    let hash = ((hilo >> 32) as i32) ^ (hilo as i32);
    SKINS[hash.rem_euclid(SKINS.len() as i32) as usize]
}

/// A UUID written with or without dashes.
pub fn parse_uuid(text: &str) -> Option<u128> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    (hex.len() == 32).then(|| u128::from_str_radix(&hex, 16).ok())?
}

/// The 8 by 8 face, row by row as RGBA, with the hat layer over it: what the tab list draws.
pub fn face(skin_png: &[u8]) -> Option<Vec<[u8; 4]>> {
    let skin = image::load_from_memory_with_format(skin_png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    if skin.width() < 64 || skin.height() < 32 {
        return None;
    }
    let mut out = Vec::with_capacity(64);
    for y in 0..8 {
        for x in 0..8 {
            let base = skin.get_pixel(8 + x, 8 + y).0;
            let hat = skin.get_pixel(40 + x, 8 + y).0;
            out.push(if hat[3] > 0 { hat } else { base });
        }
    }
    Some(out)
}

/// The face of a player with a default skin, from the prepared game files in `resources`.
pub fn default_face(resources: &Path, uuid: &str) -> Option<Vec<[u8; 4]>> {
    let skin = default_skin(parse_uuid(uuid)?);
    let path = resources
        .join("assets/minecraft/textures/entity/player")
        .join(format!("{skin}.png"));
    face(&std::fs::read(path).ok()?)
}

fn profile_pixels(png: &[u8]) -> Option<Vec<[u8; 4]>> {
    // Check dimensions before decoding a downloaded image; skin heads need no large allocation.
    let dimensions =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png)
            .into_dimensions()
            .ok()?;
    matches!(dimensions, (64, 32 | 64))
        .then(|| face(png))
        .flatten()
}

/// A signed-in profile's public skin, cached independently of its authentication tokens.
/// This may do disk/network I/O and must run on a worker. Missing skins use the UUID default;
/// the `bool` says the download failed, so the real skin is worth asking for again later.
pub fn profile_face(
    account: &crate::account::Account,
    cache: &Path,
    resources: &Path,
) -> (Option<Vec<[u8; 4]>>, bool) {
    let failed = std::cell::Cell::new(false);
    let face = profile_face_with(account, cache, resources, &|url| {
        use std::io::Read;
        failed.set(true);
        let response = crate::proxy::agent(
            ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(10)),
            url,
        )
        .build()
        .get(url)
        .call()
        .ok()?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(1024 * 1024)
            .read_to_end(&mut bytes)
            .ok()?;
        failed.set(false);
        Some(bytes)
    });
    (face, failed.get())
}

fn profile_face_with(
    account: &crate::account::Account,
    cache: &Path,
    resources: &Path,
    fetch: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Option<Vec<[u8; 4]>> {
    use sha1::{Digest, Sha1};
    if let Some((url, _)) = &account.skin {
        // Public Minecraft texture URLs only; never send credentials to a skin host.
        let texture = url
            .strip_prefix("https://textures.minecraft.net/texture/")
            .or_else(|| url.strip_prefix("http://textures.minecraft.net/texture/"));
        if texture
            .is_some_and(|hash| !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            let file = cache.join(format!("{:x}.png", Sha1::digest(url.as_bytes())));
            if let Some(face) = std::fs::read(&file)
                .ok()
                .and_then(|png| profile_pixels(&png))
            {
                return Some(face);
            }
            if let Some(png) = fetch(url) {
                if let Some(face) = profile_pixels(&png) {
                    if std::fs::create_dir_all(cache).is_ok() {
                        let _ = std::fs::write(file, png);
                    }
                    return Some(face);
                }
            }
        }
    }
    default_face(resources, &account.uuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_skins_follow_the_uuid_hash() {
        // OfflinePlayer:Alex and OfflinePlayer:Steve, as vanilla's server makes them.
        assert_eq!(
            parse_uuid("36532b5e-c442-3dbb-a24c-c7e55d0f979a"),
            Some(0x36532b5ec4423dbba24cc7e55d0f979a)
        );
        assert_eq!(default_skin(0), "slim/alex");
        assert_eq!(default_skin(1), "slim/ari");
        assert_eq!(default_skin(u128::MAX), "slim/alex");
        assert!(parse_uuid("not-a-uuid").is_none());
    }

    #[test]
    fn the_face_wears_its_hat() {
        let mut skin = image::RgbaImage::new(64, 64);
        skin.put_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
        skin.put_pixel(9, 8, image::Rgba([4, 5, 6, 255]));
        skin.put_pixel(41, 8, image::Rgba([7, 8, 9, 255]));
        let mut png = Vec::new();
        skin.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let face = face(&png).unwrap();
        assert_eq!(face.len(), 64);
        assert_eq!(face[0], [1, 2, 3, 255]);
        assert_eq!(face[1], [7, 8, 9, 255]);
    }

    #[test]
    fn profile_heads_are_cached_by_skin_and_work_offline() {
        let dir = std::env::temp_dir().join(format!(
            "hz-profile-head-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut skin = image::RgbaImage::new(64, 64);
        skin.put_pixel(8, 8, image::Rgba([12, 34, 56, 255]));
        let mut png = Vec::new();
        skin.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut account = crate::account::Account {
            name: "TestPlayer".into(),
            uuid: "00000000000000000000000000000000".into(),
            skin: Some((
                "https://textures.minecraft.net/texture/abc".into(),
                "classic".into(),
            )),
            cape: None,
        };
        let calls = std::cell::Cell::new(0);
        let fetch = |_: &str| {
            calls.set(calls.get() + 1);
            Some(png.clone())
        };
        let head = profile_face_with(&account, &dir, &dir, &fetch).unwrap();
        assert_eq!(head[0], [12, 34, 56, 255]);
        assert_eq!(
            profile_face_with(&account, &dir, &dir, &|_| panic!(
                "cached head must not need the network"
            )),
            Some(head)
        );
        account.skin.as_mut().unwrap().0 = "https://textures.minecraft.net/texture/def".into();
        assert!(profile_face_with(&account, &dir, &dir, &fetch).is_some());
        assert_eq!(calls.get(), 2, "skin changes must not reuse an old image");
        assert!(profile_pixels(&png).is_some());
        let mut oversized = Vec::new();
        image::RgbaImage::new(128, 64)
            .write_to(
                &mut std::io::Cursor::new(&mut oversized),
                image::ImageFormat::Png,
            )
            .unwrap();
        assert!(profile_pixels(&oversized).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unavailable_profile_skin_uses_the_uuid_default() {
        let dir = std::env::temp_dir().join(format!("hz-profile-default-{}", std::process::id()));
        let path = dir.join("assets/minecraft/textures/entity/player/slim/alex.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut skin = image::RgbaImage::new(64, 64);
        skin.put_pixel(8, 8, image::Rgba([3, 5, 7, 255]));
        skin.save(&path).unwrap();
        let account = crate::account::Account {
            name: "TestPlayer".into(),
            uuid: "00000000000000000000000000000000".into(),
            skin: Some((
                "https://textures.minecraft.net/texture/abc".into(),
                "classic".into(),
            )),
            cape: None,
        };
        let head = profile_face_with(&account, &dir.join("cache"), &dir, &|_| None).unwrap();
        assert_eq!(head[0], [3, 5, 7, 255]);
        let mut blocked = account;
        blocked.skin.as_mut().unwrap().0 = "https://example.com/not-a-minecraft-skin".into();
        assert!(
            profile_face_with(&blocked, &dir.join("cache"), &dir, &|_| panic!(
                "must not fetch an arbitrary profile URL"
            ))
            .is_some()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
