//! Offline skins and capes: a PNG the player picks for an offline name, kept in the launcher's
//! `skins/` folder, with its model (classic arms or slim). The game gets them in
//! `HORIZON_OFFLINE_SKIN`, `HORIZON_OFFLINE_SKIN_MODEL` and `HORIZON_OFFLINE_CAPE` when that
//! name plays, and draws them on the local player (`skins::offline_skin`, `skins::offline_cape`).
//!
//! Skins are the layout every Java Edition skin uses: 64×64, or the older 64×32 (whose left arm
//! and leg are the right ones mirrored). Capes are 64×32 (or the older 22×17).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SKIN_ENV: &str = "HORIZON_OFFLINE_SKIN";
pub const MODEL_ENV: &str = "HORIZON_OFFLINE_SKIN_MODEL";
pub const CAPE_ENV: &str = "HORIZON_OFFLINE_CAPE";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    /// Four-pixel arms ("Steve").
    #[default]
    Classic,
    /// Three-pixel arms ("Alex").
    Slim,
}

impl Model {
    pub fn name(self) -> &'static str {
        match self {
            Model::Classic => "classic",
            Model::Slim => "slim",
        }
    }
}

/// What an offline name wears.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Look {
    #[serde(default)]
    pub model: Model,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skin: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cape: bool,
}

/// Where an offline name's files are: `skins/<name>.png` and `skins/<name>.cape.png`, the name
/// in lowercase (offline names are kept once in any case).
pub fn files(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let key = name.to_ascii_lowercase();
    (
        dir.join(format!("{key}.png")),
        dir.join(format!("{key}.cape.png")),
    )
}

pub fn dir() -> PathBuf {
    crate::backend::data_dir().join("skins")
}

fn decode(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map(|i| i.to_rgba8())
        .map_err(|_| "That isn't a PNG picture.".to_string())
}

/// A skin's picture, if it is one: 64×64 or 64×32.
pub fn read_skin(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let png = decode(bytes)?;
    match png.dimensions() {
        (64, 64) | (64, 32) => Ok(png),
        (w, h) => Err(format!(
            "A skin is 64×64 (or the older 64×32) pixels; this one is {w}×{h}."
        )),
    }
}

/// A cape's picture, if it is one: 64×32 or 22×17.
pub fn read_cape(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let png = decode(bytes)?;
    match png.dimensions() {
        (64, 32) | (22, 17) => Ok(png),
        (w, h) => Err(format!(
            "A cape is 64×32 (or the older 22×17) pixels; this one is {w}×{h}."
        )),
    }
}

/// A part of the skin copied to the front view: from (x, y), its size, to (x, y), mirrored.
type Part = (u32, u32, u32, u32, u32, u32, bool);

/// The front of the player, 16×32 texels: head, body, arms and legs, each with its outer layer
/// over it (64×64 skins have one for every part; 64×32 only for the head).
pub fn front(skin: &image::RgbaImage, model: Model) -> Vec<[u8; 4]> {
    let w = 16usize;
    let mut out = vec![[0u8; 4]; w * 32];
    let modern = skin.height() == 64;
    let arm = if model == Model::Slim { 3 } else { 4 };
    let mut base: Vec<Part> = vec![
        (8, 8, 8, 8, 4, 0, false),            // head
        (20, 20, 8, 12, 4, 8, false),         // body
        (44, 20, arm, 12, 4 - arm, 8, false), // right arm, on the viewer's left
        (4, 20, 4, 12, 4, 20, false),         // right leg
    ];
    let mut outer: Vec<Part> = vec![(40, 8, 8, 8, 4, 0, false)];
    if modern {
        base.extend([
            (36, 52, arm, 12, 12, 8, false),
            (20, 52, 4, 12, 8, 20, false),
        ]);
        outer.extend([
            (20, 36, 8, 12, 4, 8, false),
            (44, 36, arm, 12, 4 - arm, 8, false),
            (52, 52, arm, 12, 12, 8, false),
            (4, 36, 4, 12, 4, 20, false),
            (4, 52, 4, 12, 8, 20, false),
        ]);
    } else {
        base.extend([(44, 20, arm, 12, 12, 8, true), (4, 20, 4, 12, 8, 20, true)]);
    }
    for (layer, parts) in [base, outer].into_iter().enumerate() {
        for (fx, fy, pw, ph, tx, ty, mirrored) in parts {
            for dy in 0..ph {
                for dx in 0..pw {
                    let sx = if mirrored { fx + pw - 1 - dx } else { fx + dx };
                    let c = skin.get_pixel(sx, fy + dy).0;
                    // The base layer is opaque in game; the outer one shows where it is drawn.
                    if layer == 0 || c[3] > 0 {
                        out[(ty + dy) as usize * w + (tx + dx) as usize] = [c[0], c[1], c[2], 255];
                    }
                }
            }
        }
    }
    out
}

/// The cape's outside, 10×16 texels.
pub fn cape_front(cape: &image::RgbaImage) -> Vec<[u8; 4]> {
    let mut out = Vec::with_capacity(160);
    for y in 1..17 {
        for x in 1..11 {
            out.push(cape.get_pixel(x, y).0);
        }
    }
    out
}

/// Keeps `file` as `name`'s skin (or cape), checked first.
pub fn keep(dir: &Path, name: &str, file: &Path, cape: bool) -> Result<(), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    match cape {
        false => read_skin(&bytes).map(|_| ())?,
        true => read_cape(&bytes).map(|_| ())?,
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (skin, cape_file) = files(dir, name);
    let to = if cape { cape_file } else { skin };
    std::fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))
}

/// Drops a forgotten name's files (`Accounts::remove_offline`); missing files are fine.
pub fn forget(dir: &Path, name: &str) {
    let (skin, cape) = files(dir, name);
    let _ = std::fs::remove_file(skin);
    let _ = std::fs::remove_file(cape);
}

/// The variables the game gets for `name`'s look.
pub fn env(dir: &Path, name: &str, look: &Look) -> Vec<(&'static str, String)> {
    let (skin, cape) = files(dir, name);
    let mut out = Vec::new();
    if look.skin && skin.is_file() {
        out.push((SKIN_ENV, skin.to_string_lossy().into_owned()));
        out.push((MODEL_ENV, look.model.name().to_string()));
    }
    if look.cape && cape.is_file() {
        out.push((CAPE_ENV, cape.to_string_lossy().into_owned()));
    }
    out
}

#[cfg(test)]
#[path = "skins/tests.rs"]
mod tests;
