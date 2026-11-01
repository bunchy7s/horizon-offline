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
}
