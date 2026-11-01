//! A file dropped on the window (or given to `instance import`): what it is, and an instance
//! made from a shared one.
//!
//! - A `.mrpack` that needs no mod loader (only `minecraft` in its dependencies), such as
//!   [`crate::export`] writes, or a `.zip` laid out as a game folder: a new instance. Its
//!   `overrides/` (and `client-overrides/`) go into the instance's folder; each of its `files`
//!   is downloaded from Modrinth and checked against its SHA-512 (`browser::download`).
//! - A `.mrpack` for a mod loader, or a CurseForge modpack zip (`manifest.json`): a Java
//!   modpack, for the Modpacks import (`modpacks::import`).
//! - A zip with `pack.mcmeta`: a data pack (`data/`) or a resource pack; with `shaders/`, or a
//!   `.hshader`: a shader pack.
//!
//! Only what an instance holds is taken from a shared zip ([`crate::export::FILES`],
//! [`crate::export::FOLDERS`], `saves/`), and every path is checked to stay inside its folder.
use crate::browser::{download, FileRef};
use crate::export::{FILES, FOLDERS};
use crate::instances::{self, Instance};
use crate::packs;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const INDEX: &str = "modrinth.index.json";
/// The `.mrpack` folders copied into the game folder; `server-overrides/` is a server's.
const OVERRIDES: [&str; 2] = ["overrides/", "client-overrides/"];

/// What a dropped file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dropped {
    /// A shared instance: a new instance is made from it.
    Instance,
    /// A Java modpack, for the Modpacks import.
    Modpack,
    /// A resource or shader pack, for the selected instance.
    Pack(packs::Kind),
    /// A data pack: it goes into a world.
    DataPack,
}

fn open(file: &Path) -> Result<zip::ZipArchive<fs::File>, String> {
    fs::File::open(file)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .ok_or_else(|| format!("{} is not a zip file that can be read.", name_of(file)))
}

fn name_of(file: &Path) -> String {
    file.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn read_index(zip: &mut zip::ZipArchive<fs::File>) -> Result<serde_json::Value, String> {
    let mut text = Vec::new();
    zip.by_name(INDEX)
        .map_err(|_| format!("It has no {INDEX}: not a .mrpack."))?
        .take(16 << 20)
        .read_to_end(&mut text)
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&text).map_err(|e| format!("{INDEX}: {e}"))
}

/// What `file` is, from its name and what is inside.
pub fn what(file: &Path) -> Result<Dropped, String> {
    let name = name_of(file);
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("hshader") => return Ok(Dropped::Pack(packs::Kind::Shader)),
        Some("mrpack") => {
            let index = read_index(&mut open(file)?)?;
            let loaders = index["dependencies"]
                .as_object()
                .is_some_and(|d| d.keys().any(|k| k != "minecraft"));
            return Ok(match loaders {
                true => Dropped::Modpack,
                false => Dropped::Instance,
            });
        }
        Some("zip") => {}
        _ => {
            return Err(format!(
                "{name} can't be imported: drop a .mrpack, a .zip or a .hshader."
            ))
        }
    }
    let zip = open(file)?;
    let has = |n: &str| zip.index_for_name(n).is_some();
    let names: Vec<_> = zip
        .file_names()
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let under = |p: &str| names.iter().any(|n| n.starts_with(p));
    if has("manifest.json") && !has("pack.mcmeta") {
        return Ok(Dropped::Modpack);
    }
    if has("pack.mcmeta") {
        return Ok(match under("data/") {
            true => Dropped::DataPack,
            false => Dropped::Pack(packs::Kind::Resource),
        });
    }
    if under("shaders/") {
        return Ok(Dropped::Pack(packs::Kind::Shader));
    }
    if FILES.iter().any(|f| has(f))
        || under("saves/")
        || FOLDERS.iter().any(|f| under(&format!("{f}/")))
    {
        return Ok(Dropped::Instance);
    }
    Err(format!(
        "{name} is neither a pack nor a shared instance: nothing in it that Horizon reads."
    ))
}

/// Where an archive entry goes in the instance's folder, when it is one an instance holds and
/// stays inside it.
fn target(home: &Path, relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    if relative.ends_with('/') || !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let top = relative.split('/').next()?;
    let kept = FILES.contains(&relative)
        || (relative.contains('/') && (FOLDERS.contains(&top) || top == "saves"));
    kept.then(|| home.join(path))
}

/// Makes a new instance from a shared one: `file` is a `.mrpack` without mod loaders or a game
/// folder `.zip`. Named after the pack (or the file), with "(2)" when the name is taken.
pub fn import_instance(file: &Path, progress: &dyn Fn(&str, f32)) -> Result<Instance, String> {
    import_into(file, progress, &|name| instances::create(name, None))
}

/// [`import_instance`] with the instance made by `create`.
pub fn import_into(
    file: &Path,
    progress: &dyn Fn(&str, f32),
    create: &dyn Fn(&str) -> Result<Instance, String>,
) -> Result<Instance, String> {
    if what(file)? != Dropped::Instance {
        return Err(format!("{} is not a shared instance.", name_of(file)));
    }
    let mut zip = open(file)?;
    let mrpack = zip.index_for_name(INDEX).is_some();
    let index = match mrpack {
        true => Some(read_index(&mut zip)?),
        false => None,
    };
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Imported".into());
    let base = index
        .as_ref()
        .and_then(|i| i["name"].as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or(stem);
    let instance = (1..20)
        .map(|n| match n {
            1 => base.clone(),
            n => format!("{base} ({n})"),
        })
        .find_map(|name| create(&name).ok())
        .ok_or_else(|| format!("Could not make an instance called {base}."))?;
    let home = instance.home.clone();
    let filled = fill(&mut zip, index.as_ref(), &home, progress);
    if let Err(e) = filled {
        let _ = fs::remove_dir_all(&home);
        return Err(e);
    }
    Ok(instance)
}

fn fill(
    zip: &mut zip::ZipArchive<fs::File>,
    index: Option<&serde_json::Value>,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<(), String> {
    let count = zip.len();
    for i in 0..count {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        let relative = match index {
            Some(_) => match OVERRIDES.iter().find_map(|o| name.strip_prefix(o)) {
                Some(r) => r.to_string(),
                None => continue,
            },
            None => name.clone(),
        };
        let Some(to) = target(home, &relative) else {
            continue;
        };
        fs::create_dir_all(to.parent().expect("inside home")).map_err(|e| e.to_string())?;
        let mut out = fs::File::create(&to).map_err(|e| format!("{}: {e}", to.display()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("{}: {e}", to.display()))?;
        progress(
            &format!("Copying {relative}"),
            (i + 1) as f32 / count as f32,
        );
    }
    let files = index
        .and_then(|i| i["files"].as_array())
        .cloned()
        .unwrap_or_default();
    for f in files {
        if f["env"]["client"] == "unsupported" {
            continue;
        }
        let relative = f["path"].as_str().unwrap_or_default();
        let to = target(home, relative)
            .ok_or_else(|| format!("{relative} isn't a place an instance keeps files."))?;
        let file = FileRef {
            url: f["downloads"][0].as_str().unwrap_or_default().to_string(),
            name: to.file_name().unwrap().to_string_lossy().into_owned(),
            size: f["fileSize"].as_u64().unwrap_or(0),
            sha512: f["hashes"]["sha512"].as_str().map(str::to_ascii_lowercase),
            sha256: None,
            sha1: f["hashes"]["sha1"].as_str().map(str::to_ascii_lowercase),
            primary: true,
        };
        let folder = to.parent().expect("inside home");
        download::fetch(&file, folder, &|p| progress(&file.name, p))
            .map_err(|e| format!("{relative}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "import/tests.rs"]
mod tests;
