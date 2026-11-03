//! An instance's resource packs and shader packs, as the game finds and picks them.
//!
//! - Resource packs: `resourcepacks/` in the game folder, each a `.zip` or a folder with a
//!   `pack.mcmeta`. The ones turned on are `options.txt`'s `resourcePacks` list (vanilla's
//!   format, `"file/<name>"`, the bottom first), so the game's Resource Packs screen shows the
//!   same choice. A pack turned on here goes on top, as the screen puts a newly picked one.
//! - Shader packs: `shaderpacks/`, Horizon's own (a folder with `horizon-shaders.toml`, or a
//!   `.hshader`) and those in the `shaders/` format (a `.zip`, or a folder with `shaders/`), as
//!   the game's Shader Packs page lists them (`shader_pack_list`). One at most is on:
//!   `pickedShaderPack` in `horizon-graphics.txt`.
//!
//! Every other line of both files is written back as it was.
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const OPTIONS: &str = "options.txt";
const GRAPHICS: &str = "horizon-graphics.txt";
const SHADER_MANIFEST: &str = "horizon-shaders.toml";
const SHADER_PACKAGE: &str = "hshader";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Resource,
    Shader,
}

impl Kind {
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Resource => "resourcepacks",
            Kind::Shader => "shaderpacks",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    pub kind: Kind,
    /// Its file or folder name in the pack folder: the id commands take.
    pub file: String,
    pub path: PathBuf,
    /// A Horizon shader pack's own name, else the file's.
    pub title: String,
    /// `pack.mcmeta`'s description as plain text; a shader pack's format.
    pub description: String,
    /// `pack.png`, a resource pack's picture.
    pub icon: Option<Vec<u8>>,
    pub enabled: bool,
}

/// The packs of `kind` in the game folder `home`, by name: turning one on or off leaves it where
/// it is.
pub fn list(home: &Path, kind: Kind) -> Vec<Pack> {
    let on = enabled(home, kind);
    let mut packs: Vec<Pack> = fs::read_dir(home.join(kind.folder()))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read(&e.path(), kind, &on))
        .collect();
    packs.sort_by_key(|p| p.title.to_lowercase());
    packs
}

/// The file names turned on, bottom first.
fn enabled(home: &Path, kind: Kind) -> Vec<String> {
    match kind {
        Kind::Resource => option_list(&fs::read_to_string(home.join(OPTIONS)).unwrap_or_default())
            .into_iter()
            .filter_map(|id| id.strip_prefix("file/").map(str::to_string))
            .collect(),
        Kind::Shader => line_value(
            &fs::read_to_string(home.join(GRAPHICS)).unwrap_or_default(),
            "pickedShaderPack",
        )
        .filter(|v| !v.is_empty())
        .into_iter()
        .collect(),
    }
}

fn read(path: &Path, kind: Kind, on: &[String]) -> Option<Pack> {
    let file = path.file_name()?.to_str()?.to_string();
    if file.starts_with('.') {
        return None;
    }
    let ext =
        |x: &str| path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case(x));
    let (title, description, icon) = match kind {
        Kind::Resource => {
            let meta = entry(path, "pack.mcmeta")?;
            let meta: serde_json::Value = serde_json::from_slice(&meta).ok()?;
            (
                file.clone(),
                plain(&meta["pack"]["description"]),
                entry(path, "pack.png"),
            )
        }
        Kind::Shader if path.join(SHADER_MANIFEST).is_file() || ext(SHADER_PACKAGE) => {
            let name = entry(path, SHADER_MANIFEST)
                .and_then(|t| toml::from_slice::<toml::Table>(&t).ok())
                .and_then(|t| t.get("name")?.as_str().map(str::to_string));
            (
                name.unwrap_or_else(|| file.clone()),
                "Horizon shader pack".into(),
                None,
            )
        }
        Kind::Shader if ext("zip") || path.join("shaders").is_dir() => {
            (file.clone(), "shaders/ format".into(), None)
        }
        Kind::Shader => return None,
    };
    Some(Pack {
        kind,
        enabled: on.contains(&file),
        path: path.to_path_buf(),
        file,
        title,
        description,
        icon,
    })
}

/// A file at the root of a pack folder or zip, at most 4 MB.
pub(crate) fn entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    if path.is_dir() {
        fs::File::open(path.join(name))
            .ok()?
            .take(4 << 20)
            .read_to_end(&mut out)
            .ok()?;
    } else {
        let mut zip = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
        let entry = zip.by_name(name).ok()?;
        entry.take(4 << 20).read_to_end(&mut out).ok()?;
    }
    Some(out)
}

/// A text component as plain text: its strings, without `§` formatting codes.
pub(crate) fn plain(component: &serde_json::Value) -> String {
    fn walk(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::String(s) => out.push_str(s),
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            serde_json::Value::Object(o) => {
                if let Some(t) = o.get("text").or_else(|| o.get("translate")) {
                    walk(t, out);
                }
                if let Some(extra) = o.get("extra") {
                    walk(extra, out);
                }
            }
            _ => {}
        }
    }
    let mut raw = String::new();
    walk(component, &mut raw);
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '§' => {
                chars.next();
            }
            '\n' => out.push(' '),
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

/// `key:value` in a `key:value` per line file.
fn line_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .map(str::to_string)
}

/// `resourcePacks:["vanilla","file/x.zip"]` in options.txt.
fn option_list(options: &str) -> Vec<String> {
    line_value(options, "resourcePacks")
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

/// Sets `key:value` in the file, or adds the line; every other line stays.
fn set_line(path: &Path, key: &str, value: &str) -> Result<(), String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(
            |l| match l.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
                Some(_) if !found => {
                    found = true;
                    format!("{key}:{value}")
                }
                _ => l.to_string(),
            },
        )
        .collect();
    if !found {
        out.push(format!("{key}:{value}"));
    }
    let mut text = out.join("\n");
    text.push('\n');
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Turns a pack on or off. A resource pack turned on goes on top; a shader pack turned on
/// replaces the one that was.
pub fn set_enabled(home: &Path, pack: &Pack, on: bool) -> Result<(), String> {
    match pack.kind {
        Kind::Resource => {
            let options = home.join(OPTIONS);
            let text = fs::read_to_string(&options).unwrap_or_default();
            let id = format!("file/{}", pack.file);
            let mut list = option_list(&text);
            list.retain(|i| *i != id);
            if list.is_empty() {
                list.push("vanilla".into());
            }
            if on {
                list.push(id.clone());
            } else {
                // The game's "use it anyway" answer goes with it.
                let mut incompatible: Vec<String> = line_value(&text, "incompatibleResourcePacks")
                    .and_then(|v| serde_json::from_str(&v).ok())
                    .unwrap_or_default();
                if incompatible.contains(&id) {
                    incompatible.retain(|i| *i != id);
                    let value = serde_json::to_string(&incompatible).unwrap();
                    set_line(&options, "incompatibleResourcePacks", &value)?;
                }
            }
            set_line(
                &options,
                "resourcePacks",
                &serde_json::to_string(&list).unwrap(),
            )
        }
        Kind::Shader => {
            let value = if on { pack.file.as_str() } else { "" };
            set_line(&home.join(GRAPHICS), "pickedShaderPack", value)
        }
    }
}

/// The pack a command names: its file name, or its title (any case).
pub fn find(home: &Path, kind: Kind, query: &str) -> Result<Pack, String> {
    let all = list(home, kind);
    all.iter()
        .find(|p| p.file == query)
        .or_else(|| all.iter().find(|p| p.title.eq_ignore_ascii_case(query)))
        .cloned()
        .ok_or_else(|| format!("No pack {query} in {}", home.join(kind.folder()).display()))
}

/// What a dropped or picked file is: a resource pack (a zip with `pack.mcmeta`), a Horizon
/// shader pack (`.hshader`), or a shader pack in the `shaders/` format (a zip with `shaders/`).
pub fn kind_of(file: &Path) -> Result<Kind, String> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some(SHADER_PACKAGE) => return Ok(Kind::Shader),
        Some("zip") => {}
        _ => {
            return Err(format!(
                "{name} is not a pack: packs are .zip or .hshader files."
            ))
        }
    }
    let zip = fs::File::open(file)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .ok_or_else(|| format!("{name} is not a zip file that can be read."))?;
    if zip.index_for_name("pack.mcmeta").is_some() {
        Ok(Kind::Resource)
    } else if zip
        .file_names()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{name} has an entry whose name can't be read: {e}"))?
        .iter()
        .any(|n| n.starts_with("shaders/"))
    {
        Ok(Kind::Shader)
    } else {
        Err(format!(
            "{name} has neither a pack.mcmeta (a resource pack) nor a shaders folder (a shader pack)."
        ))
    }
}

/// Copies a pack file into the right folder of `home`, under a name not taken yet. Returns it,
/// turned off.
pub fn add(home: &Path, file: &Path) -> Result<Pack, String> {
    let kind = kind_of(file)?;
    let dir = home.join(kind.folder());
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = file.file_name().ok_or("A pack file has a name")?;
    let stem = Path::new(name)
        .file_stem()
        .unwrap_or(name)
        .to_string_lossy();
    let ext = Path::new(name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let target = (1..)
        .map(|n| match n {
            1 => dir.join(name),
            n => dir.join(format!("{stem} ({n}){ext}")),
        })
        .find(|p| !p.exists())
        .unwrap();
    fs::copy(file, &target).map_err(|e| format!("Could not copy {}: {e}", file.display()))?;
    read(&target, kind, &[])
        .ok_or_else(|| format!("{} is not a pack the game reads", target.display()))
}

/// Turns the pack off and moves it to the system's trash.
pub fn remove(home: &Path, pack: &Pack) -> Result<(), String> {
    if pack.enabled {
        set_enabled(home, pack, false)?;
    }
    crate::desktop_seams::to_trash(&pack.path)
        .map_err(|e| format!("Could not move {} to the trash: {e}", pack.file))
}

#[cfg(test)]
#[path = "packs/tests.rs"]
mod tests;
