//! Where packs keep their custom entity models and how a game reads one: the `.jem` files of
//! `emf/cem/` and `optifine/cem/` by model name, each read under [`MAX_BYTES`], parsed, its
//! warnings logged; and the atlas name of a model's texture. Shared by the mobs' and the block
//! entities' models.

use super::model::{EntityModel, MAX_BYTES, parse_jem};
use std::{collections::BTreeMap, fs, path::Path};

/// Every `.jem` of the CEM folders by model name, sorted, each name's files best first:
/// `emf/cem` before `optifine/cem` whichever form either uses, and within one folder the flat
/// `<name>.jem` before `<name>/<name>.jem`.
pub fn jem_files(root: &Path) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for base in ["assets/minecraft/emf/cem", "assets/minecraft/optifine/cem"] {
        let Ok(entries) = fs::read_dir(root.join(base)) else {
            continue;
        };
        let (mut flat, mut nested) = (Vec::new(), Vec::new());
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(model) = name.strip_suffix(".jem") {
                // A directory named `x.jem` isn't a model.
                if e.path().is_file() {
                    flat.push((model.to_owned(), format!("{base}/{name}")));
                }
            } else {
                let path = format!("{base}/{name}/{name}.jem");
                if root.join(&path).is_file() {
                    nested.push((name, path));
                }
            }
        }
        for (model, path) in flat.into_iter().chain(nested) {
            out.entry(model).or_default().push(path);
        }
    }
    out
}

/// The `.jem` at `path`, its warnings logged; None, with a warning, when it can't be read or
/// parsed.
pub fn load_jem(root: &Path, path: &str) -> Option<EntityModel> {
    let Some(text) = read_small(&root.join(path)) else {
        log::warn!("Horizon: custom entity model {path}: unreadable or over {MAX_BYTES} bytes");
        return None;
    };
    let jem = match parse_jem(path, &text, &mut |p: &str| read_small(&root.join(p))) {
        Ok(jem) => jem,
        Err(e) => {
            log::warn!("Horizon: custom entity model {e}");
            return None;
        }
    };
    for warning in &jem.warnings {
        log::warn!("Horizon: custom entity model {warning}");
    }
    if let Some(texture) = &jem.texture {
        if atlas_name(texture).is_none() {
            log::warn!(
                "Horizon: custom entity model {path}: its texture {texture} is outside \
                 assets/minecraft/textures/, so not in the atlas: the vanilla one draws"
            );
        } else if !root.join(texture).is_file() {
            log::warn!(
                "Horizon: custom entity model {path}: its texture {texture} doesn't exist, \
                 so not in the atlas: the vanilla one draws"
            );
        }
    }
    Some(jem)
}

/// A pack file's text, None when it's missing or larger than a model may read: its size is
/// checked before anything is read.
fn read_small(path: &Path) -> Option<String> {
    let len = fs::metadata(path).ok()?.len();
    if len > MAX_BYTES as u64 {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// The atlas name of a texture resource path: `assets/minecraft/textures/entity/x.png` is
/// `entity/x`. Textures outside a `textures/` folder are not in the atlas.
pub fn atlas_name(path: &str) -> Option<String> {
    let rest = path.strip_prefix("assets/minecraft/textures/")?;
    Some(rest.strip_suffix(".png").unwrap_or(rest).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emf_files_win_and_a_flat_file_beats_its_folder() {
        let root = std::env::temp_dir().join(format!("horizon-jem-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for path in [
            // EMF's folder form over OptiFine's flat file.
            "optifine/cem/cow.jem",
            "emf/cem/cow/cow.jem",
            // One folder: the flat file over the folder form.
            "optifine/cem/pig.jem",
            "optifine/cem/pig/pig.jem",
            // A directory named `sheep.jem` is no file: the folder form stands.
            "optifine/cem/sheep.jem/readme.txt",
            "optifine/cem/sheep/sheep.jem",
        ] {
            let path = root.join("assets/minecraft").join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "{}").unwrap();
        }
        let files = jem_files(&root);
        fs::remove_dir_all(&root).unwrap();
        let paths = |paths: &[&str]| {
            paths
                .iter()
                .map(|p| format!("assets/minecraft/{p}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            files,
            BTreeMap::from([
                (
                    "cow".to_owned(),
                    paths(&["emf/cem/cow/cow.jem", "optifine/cem/cow.jem"])
                ),
                (
                    "pig".to_owned(),
                    paths(&["optifine/cem/pig.jem", "optifine/cem/pig/pig.jem"])
                ),
                ("sheep".to_owned(), paths(&["optifine/cem/sheep/sheep.jem"])),
            ])
        );
    }
}
