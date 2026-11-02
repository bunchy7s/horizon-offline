//! ETF's blinking textures: beside a texture, `<name>_blink.png` (eyes shut) and optionally
//! `<name>_blink2.png` (half shut), which the entity wears for a few ticks now and then. A
//! `<name>_blink.properties` beside them sets `blinkLength` and `blinkFrequency` (in ticks); a
//! random variant blinks with its own (`zombie3_blink.png`).
//!
//! Without a properties file a blink lasts [`LENGTH`] ticks every [`FREQUENCY`]: the values of
//! ETF's documented example, as its docs give no defaults. With a half-shut frame, the first and
//! last ticks of a blink of 3 or more show it.
//!
//! Written from ETF's README ("Blinking textures"); no mod's source was read.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::paths::files_under;
use crate::properties::Properties;
use crate::random_textures::{RandomTextures, atlas_name};

pub const LENGTH: u32 = 5;
pub const FREQUENCY: u32 = 150;

/// The pack's blinking textures, by the atlas name of the texture they replace.
#[derive(Debug, Default)]
pub struct Blinks {
    blinks: HashMap<String, Blink>,
    /// The blink frames of random variants: file and atlas name, for the atlas to pack.
    files: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Blink {
    /// The atlas names of the shut and half-shut frames.
    pub shut: String,
    pub half: Option<String>,
    pub length: u32,
    pub frequency: u32,
}

impl Blinks {
    pub fn scan(root: &Path, random: &RandomTextures) -> Self {
        let mut out = Self::default();
        let Ok(namespaces) = fs::read_dir(root.join("assets")) else {
            return out;
        };
        for ns in namespaces.flatten() {
            let ns = ns.file_name().to_string_lossy().into_owned();
            let prefix = format!("assets/{ns}/");
            for file in files_under(root, &format!("{prefix}textures/entity"), &["_blink.png"]) {
                let stem = &file[..file.len() - "_blink.png".len()];
                if root.join(format!("{stem}.png")).is_file() {
                    let blink =
                        Blink::read(root, stem, |path| atlas_name(&ns, &path[prefix.len()..]));
                    out.blinks
                        .insert(atlas_name(&ns, &stem[prefix.len()..]), blink);
                }
            }
        }
        for (file, name) in random.files() {
            let Some(stem) = file.strip_suffix(".png") else {
                continue;
            };
            if !root.join(format!("{stem}_blink.png")).is_file() {
                continue;
            }
            let frame = |path: &str| format!("{name}{}", &path[stem.len()..]);
            let blink = Blink::read(root, stem, frame);
            if !file.contains("/textures/") {
                out.files
                    .push((format!("{stem}_blink.png"), blink.shut.clone()));
                if let Some(half) = &blink.half {
                    out.files.push((format!("{stem}_blink2.png"), half.clone()));
                }
            }
            out.blinks.insert(name.to_owned(), blink);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.blinks.is_empty()
    }

    pub fn get(&self, texture: &str) -> Option<&Blink> {
        self.blinks.get(texture)
    }

    /// The frames of random variants' blinks: file and atlas name, for the entity atlas to pack
    /// (it walks `textures/` itself).
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(f, n)| (f.as_str(), n.as_str()))
    }
}

impl Blink {
    /// The blink of the texture at `stem` (its path from the root, no `.png`); `name` gives a
    /// frame's atlas name from its path.
    fn read(root: &Path, stem: &str, name: impl Fn(&str) -> String) -> Self {
        let props = fs::read_to_string(root.join(format!("{stem}_blink.properties")))
            .map(|text| Properties::parse(&text))
            .unwrap_or_default();
        let number = |key, default| {
            props
                .get(key)
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        // At least a tick, and short of `u32::MAX` so the frequency can exceed it.
        let length: u32 = number("blinkLength", LENGTH).clamp(1, u32::MAX - 1);
        let half = format!("{stem}_blink2");
        Self {
            shut: name(&format!("{stem}_blink")),
            half: root
                .join(format!("{half}.png"))
                .is_file()
                .then(|| name(&half)),
            length,
            frequency: number("blinkFrequency", FREQUENCY).max(length + 1),
        }
    }

    /// The frame an entity shows `tick` ticks into its life: `None` with its eyes open. `phase`
    /// spreads entities' blinks apart.
    pub fn frame(&self, tick: u32, phase: u32) -> Option<&str> {
        let t = (u64::from(tick) + u64::from(phase)) % u64::from(self.frequency);
        let t = t as u32;
        if t >= self.length {
            return None;
        }
        match &self.half {
            Some(half) if self.length >= 3 && (t == 0 || t == self.length - 1) => Some(half),
            _ => Some(&self.shut),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(name: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, Blinks) {
        let root =
            std::env::temp_dir().join(format!("horizon-blink-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let b = Blinks::scan(&root, &RandomTextures::scan(&root));
        (root, b)
    }

    const ENTITY: &str = "assets/minecraft/textures/entity";
    const RANDOM: &str = "assets/minecraft/optifine/random/entity";

    #[test]
    fn frames_and_timing() {
        let (root, b) = pack(
            "frames",
            &[
                (&format!("{ENTITY}/creeper/creeper.png"), ""),
                (&format!("{ENTITY}/creeper/creeper_blink.png"), ""),
                (&format!("{ENTITY}/creeper/creeper_blink2.png"), ""),
                (
                    &format!("{ENTITY}/creeper/creeper_blink.properties"),
                    "blinkLength=4\nblinkFrequency=100",
                ),
                (&format!("{ENTITY}/pig/pig.png"), ""),
                (&format!("{ENTITY}/pig/pig_blink.png"), ""),
                // No texture to blink over.
                (&format!("{ENTITY}/cow/cow_blink.png"), ""),
            ],
        );
        let creeper = b.get("entity/creeper/creeper").unwrap();
        assert_eq!((creeper.length, creeper.frequency), (4, 100));
        // Ticks 0-3 of every 100 blink: half, shut, shut, half.
        let frames: Vec<_> = (0..5).map(|t| creeper.frame(t, 0)).collect();
        let (half, shut) = (
            Some("entity/creeper/creeper_blink2"),
            Some("entity/creeper/creeper_blink"),
        );
        assert_eq!(frames, [half, shut, shut, half, None]);
        assert_eq!(creeper.frame(101, 0), shut);
        // A phase of 30 moves the blink to ticks 70-73.
        assert_eq!(creeper.frame(70, 30), half);
        assert_eq!(creeper.frame(0, 30), None);
        let pig = b.get("entity/pig/pig").unwrap();
        assert_eq!(
            (pig.length, pig.frequency, pig.half.as_deref()),
            (LENGTH, FREQUENCY, None)
        );
        assert_eq!(pig.frame(0, 0), Some("entity/pig/pig_blink"));
        assert!(b.get("entity/cow/cow").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_variant_blinks_with_its_own_frames() {
        let (root, b) = pack(
            "variant",
            &[
                (&format!("{RANDOM}/zombie/zombie3.png"), ""),
                (&format!("{RANDOM}/zombie/zombie3_blink.png"), ""),
                (
                    &format!("{RANDOM}/zombie/zombie3_blink.properties"),
                    "blinkLength=0\nblinkFrequency=0",
                ),
            ],
        );
        let variant = "minecraft:optifine/random/entity/zombie/zombie3";
        let blink = b.get(variant).unwrap();
        assert_eq!(blink.shut, format!("{variant}_blink"));
        // Out-of-range values: at least a tick, every other tick.
        assert_eq!((blink.length, blink.frequency), (1, 2));
        assert_eq!(
            b.files().collect::<Vec<_>>(),
            [(
                "assets/minecraft/optifine/random/entity/zombie/zombie3_blink.png",
                format!("{variant}_blink").as_str()
            )]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_huge_blink_length_still_opens_the_eyes() {
        let (root, b) = pack(
            "huge",
            &[
                (&format!("{ENTITY}/pig/pig.png"), ""),
                (&format!("{ENTITY}/pig/pig_blink.png"), ""),
                (
                    &format!("{ENTITY}/pig/pig_blink.properties"),
                    "blinkLength=4294967295",
                ),
            ],
        );
        let pig = b.get("entity/pig/pig").unwrap();
        assert_eq!((pig.length, pig.frequency), (u32::MAX - 1, u32::MAX));
        assert_eq!(pig.frame(u32::MAX - 1, 0), None, "the one open tick");
        let _ = fs::remove_dir_all(root);
    }
}
