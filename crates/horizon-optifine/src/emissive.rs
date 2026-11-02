//! Emissive textures: a texture's overlay that draws at full brightness, named after it with
//! the pack's emissive suffix (`zombie.png` and `zombie_e.png`).
//!
//! The suffix is `suffix.emissive` in the first of `assets/minecraft/optifine/emissive.properties`,
//! ETF's `assets/minecraft/etf/emissive.properties` and `assets/minecraft/textures/emissive.properties`,
//! and `_e` without one (as ETF reads it). An overlay sits beside its texture, so a random
//! variant's is beside the variant: `optifine/random/entity/zombie/zombie3_e.png` glows over
//! `zombie3`, never over `zombie`.
//!
//! This pairs entity textures, those under `textures/entity/` (mobs' and block entities'), by
//! the entity atlas's names ([`crate::random_textures`]), and through [`Emissive::pair_beside`]
//! random variants and their blink frames wherever they are. A blink frame glows with its own
//! overlay only: the texture's would paint its open eyes over the shut ones. Blocks and items
//! have their own atlas.
//!
//! Written from OptiFine's `emissive.properties` and ETF's `EMISSIVE_GUIDE.md` and
//! `emissive.properties`; no mod's source was read.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::paths::files_under;
use crate::properties::Properties;
use crate::random_textures::{MAX_PROPERTIES, RandomTextures, atlas_name, decode};

/// The pack's emissive overlays, by the atlas name of the texture they glow over.
#[derive(Debug, Default)]
pub struct Emissive {
    pub suffix: String,
    overlays: HashMap<String, String>,
    /// Overlays outside `textures/` (of random variants): file and atlas name.
    files: Vec<(String, String)>,
}

/// Where the suffix is read, first found first.
const PROPERTIES: [&str; 3] = [
    "assets/minecraft/optifine/emissive.properties",
    "assets/minecraft/etf/emissive.properties",
    "assets/minecraft/textures/emissive.properties",
];

impl Emissive {
    pub fn scan(root: &Path, random: &RandomTextures) -> Self {
        let suffix = suffix(root);
        let mut out = Self {
            suffix,
            ..Self::default()
        };
        let Ok(namespaces) = fs::read_dir(root.join("assets")) else {
            return out;
        };
        let png = format!("{}.png", out.suffix);
        for ns in namespaces.flatten() {
            let ns = ns.file_name().to_string_lossy().into_owned();
            let prefix = format!("assets/{ns}/");
            for file in files_under(root, &format!("{prefix}textures/entity"), &[png.as_str()]) {
                let overlay = &file[prefix.len()..file.len() - ".png".len()];
                let base = &overlay[..overlay.len() - out.suffix.len()];
                if root.join(format!("{prefix}{base}.png")).is_file() {
                    out.overlays
                        .insert(atlas_name(&ns, base), atlas_name(&ns, overlay));
                }
            }
        }
        out.pair_beside(root, random.files());
        out
    }

    /// Pairs each of `files` (file from the root, atlas name: random variants, blink frames)
    /// with an overlay beside it, and lists those outside `textures/` for the atlas.
    pub fn pair_beside<'a>(
        &mut self,
        root: &Path,
        files: impl Iterator<Item = (&'a str, &'a str)>,
    ) {
        let png = format!("{}.png", self.suffix);
        for (file, name) in files {
            let Some(stem) = file.strip_suffix(".png") else {
                continue;
            };
            let overlay = format!("{stem}{png}");
            if root.join(&overlay).is_file() {
                let overlay_name = format!("{name}{}", self.suffix);
                self.overlays.insert(name.to_owned(), overlay_name.clone());
                // Those under `textures/` are the atlas's already.
                if !overlay.contains("/textures/") {
                    self.files.push((overlay, overlay_name));
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    /// The atlas name of the overlay that glows over `texture`.
    pub fn overlay(&self, texture: &str) -> Option<&str> {
        self.overlays.get(texture).map(String::as_str)
    }

    /// The overlays of random variants: file (from the resource root) and atlas name, for the
    /// entity atlas to pack (it walks `textures/` itself).
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(f, n)| (f.as_str(), n.as_str()))
    }
}

/// The pack's emissive suffix: `suffix.emissive` from the first of [`PROPERTIES`] that sets it,
/// `_e` without one.
pub fn suffix(root: &Path) -> String {
    // The first file that sets it (read as Java reads properties: Latin-1 when not UTF-8).
    // A suffix is a name's end: no path separators or `..`, so it can't leave the folder.
    PROPERTIES
        .iter()
        .find_map(|p| {
            let path = root.join(p);
            if fs::metadata(&path).ok()?.len() > MAX_PROPERTIES {
                return None;
            }
            let text = decode(fs::read(path).ok()?);
            Properties::parse(&text)
                .get("suffix.emissive")
                .map(str::to_owned)
        })
        .filter(|s| !s.is_empty() && !s.contains(['/', '\\']) && !s.contains(".."))
        .unwrap_or_else(|| "_e".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(name: &str, files: &[&str], suffix: Option<&str>) -> (std::path::PathBuf, Emissive) {
        let root =
            std::env::temp_dir().join(format!("horizon-emissive-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for path in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        if let Some(suffix) = suffix {
            let path = root.join(PROPERTIES[0]);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("suffix.emissive={suffix}\n")).unwrap();
        }
        let e = Emissive::scan(&root, &RandomTextures::scan(&root));
        (root, e)
    }

    const ENTITY: &str = "assets/minecraft/textures/entity";
    const RANDOM: &str = "assets/minecraft/optifine/random/entity";

    #[test]
    fn overlays_beside_their_texture() {
        let (root, e) = pack(
            "beside",
            &[
                &format!("{ENTITY}/zombie/zombie.png"),
                &format!("{ENTITY}/zombie/zombie_e.png"),
                // No base: no overlay.
                &format!("{ENTITY}/zombie/husk_e.png"),
                &format!("{ENTITY}/chest/normal.png"),
                &format!("{ENTITY}/chest/normal_e.png"),
                "assets/mymod/textures/entity/bug/bug.png",
                "assets/mymod/textures/entity/bug/bug_e.png",
                // A variant's overlay glows over the variant only.
                &format!("{RANDOM}/zombie/zombie3.png"),
                &format!("{RANDOM}/zombie/zombie3_e.png"),
                &format!("{RANDOM}/zombie/zombie2.png"),
            ],
            None,
        );
        assert_eq!(e.suffix, "_e");
        assert_eq!(
            e.overlay("entity/zombie/zombie"),
            Some("entity/zombie/zombie_e")
        );
        assert_eq!(e.overlay("entity/zombie/husk"), None);
        assert_eq!(
            e.overlay("entity/chest/normal"),
            Some("entity/chest/normal_e")
        );
        assert_eq!(
            e.overlay("mymod:entity/bug/bug"),
            Some("mymod:entity/bug/bug_e")
        );
        let variant = "minecraft:optifine/random/entity/zombie/zombie3";
        assert_eq!(e.overlay(variant), Some(format!("{variant}_e").as_str()));
        assert_eq!(
            e.overlay("minecraft:optifine/random/entity/zombie/zombie2"),
            None
        );
        assert_eq!(
            e.files().collect::<Vec<_>>(),
            [(
                "assets/minecraft/optifine/random/entity/zombie/zombie3_e.png",
                "minecraft:optifine/random/entity/zombie/zombie3_e"
            )]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_packs_suffix() {
        let (root, e) = pack(
            "suffix",
            &[
                &format!("{ENTITY}/zombie/zombie.png"),
                &format!("{ENTITY}/zombie/zombie_e.png"),
                &format!("{ENTITY}/zombie/zombie_glow.png"),
            ],
            Some("_glow"),
        );
        assert_eq!(
            e.overlay("entity/zombie/zombie"),
            Some("entity/zombie/zombie_glow")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn where_the_suffix_is_read() {
        let suffix = |files: &[(&str, &[u8])]| {
            let root = std::env::temp_dir().join(format!(
                "horizon-emissive-where-{}-{}",
                std::process::id(),
                files.len()
            ));
            let _ = fs::remove_dir_all(&root);
            for (path, bytes) in files {
                let path = root.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, bytes).unwrap();
            }
            let e = Emissive::scan(&root, &RandomTextures::default());
            let _ = fs::remove_dir_all(root);
            e.suffix
        };
        // ETF's folder, when OptiFine's file is missing or doesn't set it.
        assert_eq!(suffix(&[(PROPERTIES[1], b"suffix.emissive=_lit")]), "_lit");
        assert_eq!(
            suffix(&[
                (PROPERTIES[0], b"# trims only"),
                (PROPERTIES[2], b"suffix.emissive=_lit")
            ]),
            "_lit"
        );
        // A Latin-1 comment doesn't hide it.
        assert_eq!(
            suffix(&[
                (PROPERTIES[0], b"# cr\xe9\xe9\nsuffix.emissive=_glow"),
                (PROPERTIES[1], b"x=1"),
                (PROPERTIES[2], b"x=1")
            ]),
            "_glow"
        );
        // Nothing that could leave the texture's folder.
        for bad in [
            &b"suffix.emissive=_a\\\\b"[..],
            b"suffix.emissive=/x",
            b"suffix.emissive=..",
        ] {
            assert_eq!(suffix(&[(PROPERTIES[0], bad)]), "_e");
        }
    }

    #[test]
    fn blink_frames_glow_with_their_own_overlay_only() {
        let (root, mut e) = pack(
            "blink",
            &[
                &format!("{RANDOM}/zombie/zombie3.png"),
                &format!("{RANDOM}/zombie/zombie3_e.png"),
                &format!("{RANDOM}/zombie/zombie3_blink.png"),
                &format!("{RANDOM}/zombie/zombie3_blink_e.png"),
                &format!("{ENTITY}/pig/pig.png"),
                &format!("{ENTITY}/pig/pig_e.png"),
                &format!("{ENTITY}/pig/pig_blink.png"),
            ],
            None,
        );
        let blinks = crate::blink::Blinks::scan(&root, &RandomTextures::scan(&root));
        e.pair_beside(&root, blinks.files());
        let frame = "minecraft:optifine/random/entity/zombie/zombie3_blink";
        assert_eq!(e.overlay(frame), Some(format!("{frame}_e").as_str()));
        assert!(e.files().any(|(_, name)| name == format!("{frame}_e")));
        // No pig_blink_e: the shut eye doesn't take the pig's glow (its open eyes, mostly).
        assert_eq!(e.overlay("entity/pig/pig_blink"), None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn no_overlays_no_entries() {
        let (root, e) = pack("none", &[&format!("{ENTITY}/zombie/zombie.png")], None);
        assert!(e.is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
