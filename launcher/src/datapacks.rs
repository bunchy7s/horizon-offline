//! A world's data packs: `<world>/datapacks/`, each a `.zip` or a folder with a `pack.mcmeta`, as
//! the game discovers them (`datapack::discover`). The game turns on every pack it finds there
//! that `<world>/datapacks.json` doesn't list as disabled, when the world opens or on `/reload`,
//! so adding one is copying it in.
use crate::packs::{entry, plain};
use std::fs;
use std::path::{Path, PathBuf};

const SETTINGS: &str = "datapacks.json";

#[derive(Clone, Debug, PartialEq)]
pub struct DataPack {
    /// Its file or folder name in `datapacks/`; the game's id is `file/<name>`.
    pub file: String,
    pub path: PathBuf,
    /// `pack.mcmeta`'s description as plain text.
    pub description: String,
    /// Off when `datapacks.json` lists it as disabled (the game's `/datapack disable`).
    pub enabled: bool,
}

fn settings(world: &Path) -> serde_json::Value {
    fs::read(world.join(SETTINGS))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn disabled(settings: &serde_json::Value) -> Vec<String> {
    settings["disabled"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// The data packs of the world folder `world`, by name.
pub fn list(world: &Path) -> Vec<DataPack> {
    let off = disabled(&settings(world));
    let mut packs: Vec<DataPack> = fs::read_dir(world.join("datapacks"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let file = e.file_name().to_str()?.to_string();
            let zip = path.is_file() && file.to_ascii_lowercase().ends_with(".zip");
            if !(zip || path.is_dir()) {
                return None;
            }
            let meta: serde_json::Value =
                serde_json::from_slice(&entry(&path, "pack.mcmeta")?).ok()?;
            Some(DataPack {
                enabled: !off.contains(&format!("file/{file}")),
                description: plain(&meta["pack"]["description"]),
                file,
                path,
            })
        })
        .collect();
    packs.sort_by_key(|p| p.file.to_lowercase());
    packs
}

/// Whether a zip is a data pack: a `pack.mcmeta` and a `data/` folder.
fn check(file: &Path) -> Result<(), String> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let zip = fs::File::open(file)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .ok_or_else(|| format!("{name} is not a zip file that can be read."))?;
    if zip.index_for_name("pack.mcmeta").is_none() {
        return Err(format!("{name} is not a pack: it has no pack.mcmeta."));
    }
    if !zip.file_names().any(|n| n.starts_with("data/")) {
        return Err(format!(
            "{name} has no data folder: it is a resource pack, not a data pack."
        ));
    }
    Ok(())
}

/// Copies a data pack zip into the world's `datapacks/` under a free name. The game turns it on
/// the next time the world opens (or on `/reload`).
pub fn add(world: &Path, file: &Path) -> Result<DataPack, String> {
    check(file)?;
    let dir = world.join("datapacks");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = file.file_name().ok_or("A pack file has a name")?;
    let stem = Path::new(name)
        .file_stem()
        .unwrap_or(name)
        .to_string_lossy();
    let target = (1..)
        .map(|n| match n {
            1 => dir.join(name),
            n => dir.join(format!("{stem} ({n}).zip")),
        })
        .find(|p| !p.exists())
        .unwrap();
    fs::copy(file, &target).map_err(|e| format!("Could not copy {}: {e}", file.display()))?;
    let file_name = target.file_name().unwrap().to_string_lossy().into_owned();
    // A pack of the same name removed while it was disabled would stay off.
    forget(world, &file_name)?;
    list(world)
        .into_iter()
        .find(|p| p.file == file_name)
        .ok_or_else(|| format!("{} is not a data pack the game reads", target.display()))
}

/// Drops `file/<name>` from both lists of `datapacks.json`, when it is there.
fn forget(world: &Path, file: &str) -> Result<(), String> {
    let mut s = settings(world);
    let id = format!("file/{file}");
    let mut changed = false;
    for key in ["enabled", "disabled"] {
        if let Some(list) = s[key].as_array_mut() {
            let before = list.len();
            list.retain(|v| v.as_str() != Some(id.as_str()));
            changed |= list.len() != before;
        }
    }
    if !changed {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&s).unwrap();
    fs::write(world.join(SETTINGS), text).map_err(|e| format!("{SETTINGS}: {e}"))
}

/// The data pack a command names: its file name, with or without `.zip`.
pub fn find(world: &Path, query: &str) -> Result<DataPack, String> {
    list(world)
        .into_iter()
        .find(|p| p.file == query || p.file.strip_suffix(".zip") == Some(query))
        .ok_or_else(|| {
            format!(
                "No data pack {query} in {}",
                world.join("datapacks").display()
            )
        })
}

/// Moves the data pack to the system's trash and forgets it in `datapacks.json`.
pub fn remove(world: &Path, pack: &DataPack) -> Result<(), String> {
    crate::desktop_seams::to_trash(&pack.path)
        .map_err(|e| format!("Could not move {} to the trash: {e}", pack.file))?;
    forget(world, &pack.file)
}

/// What of a data pack the game reads today (its `DataStore`): `data/<namespace>/<kind>/`.
pub const READ: [&str; 4] = ["function", "function tag", "predicate", "item_modifier"];

/// A data pack's files by kind (`function`, `worldgen`, `item tag`...), with how many each.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coverage {
    /// What Horizon runs.
    pub read: Vec<(String, usize)>,
    /// What it doesn't read yet.
    pub unread: Vec<(String, usize)>,
}

impl Coverage {
    /// "Runs its 12 functions, 1 predicate. Not read yet: 40 worldgen, 3 recipes."
    pub fn summary(&self) -> String {
        let runs = match self.read.is_empty() {
            true => "Nothing in it runs in Horizon yet.".to_string(),
            false => format!("Runs its {}.", counts(&self.read)),
        };
        match self.unread.is_empty() {
            true => runs,
            false => format!("{runs} Not read yet: {}.", self.not_read()),
        }
    }

    /// "1077 worldgen, 653 loot tables, 16 recipes": what Horizon doesn't read yet, the six
    /// biggest kinds and how many other kinds.
    pub fn not_read(&self) -> String {
        let mut biggest = self.unread.clone();
        biggest.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let others = biggest.len().saturating_sub(6);
        biggest.truncate(6);
        match others {
            0 => counts(&biggest),
            1 => format!("{} and 1 other kind", counts(&biggest)),
            n => format!("{} and {n} other kinds", counts(&biggest)),
        }
    }
}

fn counts(list: &[(String, usize)]) -> String {
    list.iter()
        .map(|(k, n)| {
            let k = k.replace('_', " ");
            match *n == 1 || k == "worldgen" {
                true => format!("{n} {k}"),
                false => format!("{n} {k}s"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Counts a data pack zip's files by kind, and splits them into what the game reads and what
/// it doesn't yet.
pub fn coverage(file: &Path) -> Result<Coverage, String> {
    let zip = fs::File::open(file)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .ok_or_else(|| format!("{} is not a zip file that can be read.", file.display()))?;
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for name in zip.file_names().filter(|n| !n.ends_with('/')) {
        let parts: Vec<&str> = name.split('/').collect();
        let kind = match parts.as_slice() {
            ["data", _, "tags", tag, _, ..] => format!("{tag} tag"),
            ["data", _, kind, _, ..] => kind.to_string(),
            _ => continue,
        };
        *kinds.entry(kind).or_default() += 1;
    }
    let (read, unread) = kinds
        .into_iter()
        .partition(|(k, _)| READ.contains(&k.as_str()));
    Ok(Coverage { read, unread })
}

#[cfg(test)]
#[path = "datapacks/tests.rs"]
mod tests;
