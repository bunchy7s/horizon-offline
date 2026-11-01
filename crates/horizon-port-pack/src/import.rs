//! Step 1, import: a Modrinth `.mrpack`, a CurseForge zip, an instance
//! folder (Prism / MultiMC, or a plain `mods/` folder), or a Crash Assistant
//! `modlist.json` (the list packs such as ATM10 publish) to `pack.json`.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Pack {
    pub name: String,
    /// `mrpack`, `curseforge` or `instance`.
    pub format: String,
    pub minecraft: String,
    /// `neoforge`, `forge`, `fabric` or `quilt`.
    pub loader: String,
    pub loader_version: String,
    pub files: Vec<PackFile>,
    /// Override files by top folder (`config`, `kubejs`, ...).
    pub overrides: BTreeMap<String, usize>,
    /// A CurseForge zip's files by project and file id only: how many its
    /// manifest lists, when the files came from the pack's own mod list.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub curseforge_files: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Where Crash Assistant keeps its list in an instance (and in the
/// overrides of packs that ship it, such as All the Mods 10).
pub const MODLIST: &str = "config/crash_assistant/modlist.json";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PackFile {
    /// Path in the instance (`mods/create-1.21.1-6.0.4.jar`).
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    /// `required`, `optional` or `unsupported`; empty when not stated.
    pub client: String,
    pub server: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<String>,
    /// CurseForge project and file ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curseforge: Option<(u64, u64)>,
    /// The mod id, name and version a mod list states (`modlist.json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The jar on disk (instances only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar: Option<PathBuf>,
}

impl PackFile {
    /// `mods`, `resourcepacks`, `shaderpacks`, `datapacks`...
    pub fn kind(&self) -> &str {
        self.path.split('/').next().unwrap_or("")
    }
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

const LOADERS: [(&str, &str); 4] = [
    ("neoforge", "neoforge"),
    ("forge", "forge"),
    ("fabric-loader", "fabric"),
    ("quilt-loader", "quilt"),
];

pub fn import(path: &Path) -> Result<Pack, String> {
    if path.is_dir() {
        return instance(path);
    }
    if path.extension().is_some_and(|e| e == "json") {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let list: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        return modlist(&list);
    }
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(index) = read_json(&mut zip, "modrinth.index.json")? {
        return Ok(mrpack(&index, &names(&zip)?));
    }
    if let Some(manifest) = read_json(&mut zip, "manifest.json")? {
        let mut pack = curseforge(&manifest, &names(&zip)?);
        // The manifest names files only by CurseForge ids, which need a key
        // to resolve; the pack's own mod list names them and gives their
        // SHA-1s, so Modrinth can answer for most.
        let root = manifest["overrides"].as_str().unwrap_or("overrides");
        if let Some(list) = read_json(&mut zip, &format!("{root}/{MODLIST}"))? {
            pack.curseforge_files = pack.files.len();
            pack.files = modlist(&list)?.files;
        }
        return Ok(pack);
    }
    Err(format!(
        "{}: neither modrinth.index.json nor manifest.json",
        path.display()
    ))
}

fn names(zip: &zip::ZipArchive<fs::File>) -> Result<Vec<String>, String> {
    zip.file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

fn read_json(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Result<Option<Value>, String> {
    let Ok(mut entry) = zip.by_name(name) else {
        return Ok(None);
    };
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|e| format!("{name}: {e}"))?;
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map(Some)
        .map_err(|e| format!("{name}: {e}"))
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// Override files by top folder, below any of `roots`.
fn overrides(names: &[String], roots: &[&str]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for name in names.iter().filter(|n| !n.ends_with('/')) {
        for root in roots {
            if let Some(rest) = name.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
                let top = rest.split('/').next().unwrap_or(rest);
                *out.entry(top.to_owned()).or_default() += 1;
            }
        }
    }
    out
}

pub fn mrpack(index: &Value, names: &[String]) -> Pack {
    let deps = &index["dependencies"];
    let (loader, loader_version) = LOADERS
        .iter()
        .find_map(|(key, name)| Some((name.to_string(), deps.get(*key)?.as_str()?.to_owned())))
        .unwrap_or_default();
    let files = index["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| PackFile {
            path: str_of(f, "path"),
            sha1: f["hashes"]["sha1"].as_str().map(str::to_owned),
            client: str_of(&f["env"], "client"),
            server: str_of(&f["env"], "server"),
            downloads: f["downloads"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d.as_str().map(str::to_owned))
                .collect(),
            ..PackFile::default()
        })
        .collect();
    Pack {
        name: str_of(index, "name"),
        format: "mrpack".into(),
        minecraft: str_of(deps, "minecraft"),
        loader,
        loader_version,
        files,
        overrides: overrides(
            names,
            &["overrides", "client-overrides", "server-overrides"],
        ),
        ..Pack::default()
    }
}

pub fn curseforge(manifest: &Value, names: &[String]) -> Pack {
    let primary = manifest["minecraft"]["modLoaders"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|l| l["primary"].as_bool().unwrap_or(true))
        .map(|l| str_of(l, "id"))
        .unwrap_or_default();
    let (loader, loader_version) = primary.split_once('-').unwrap_or((&primary, ""));
    let files = manifest["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let (p, id) = (f["projectID"].as_u64(), f["fileID"].as_u64());
            PackFile {
                path: format!("mods/curseforge-{}-{}", p.unwrap_or(0), id.unwrap_or(0)),
                // CurseForge states no side; its API needs a key, so names
                // come from the user's installed instance (import that).
                curseforge: p.zip(id),
                ..PackFile::default()
            }
        })
        .collect();
    let root = manifest["overrides"].as_str().unwrap_or("overrides");
    Pack {
        name: str_of(manifest, "name"),
        format: "curseforge".into(),
        minecraft: str_of(&manifest["minecraft"], "version"),
        loader: loader.to_owned(),
        loader_version: loader_version.to_owned(),
        files,
        overrides: overrides(names, &[root]),
        ..Pack::default()
    }
}

/// Crash Assistant's `config/crash_assistant/modlist.json`: jar name → mod
/// id, name, version and Modrinth SHA-1, with the loader as an entry of its
/// own. NeoForge `21.1.x` runs Minecraft 1.21.1.
pub fn modlist(list: &Value) -> Result<Pack, String> {
    let entries = list.as_object().ok_or("modlist.json: not an object")?;
    let mut pack = Pack {
        name: "modlist".into(),
        format: "modlist".into(),
        ..Pack::default()
    };
    for (key, e) in entries {
        let jar = e["jarName"].as_str().unwrap_or(key);
        if jar.ends_with("(modloader)") {
            let version = str_of(e, "version");
            let (loader, v) = version.split_once('-').unwrap_or(("", &version));
            pack.loader = loader.to_owned();
            pack.loader_version = v.to_owned();
            if loader == "neoforge" {
                let mut parts = v.split('.');
                if let (Some(a), Some(b)) = (parts.next(), parts.next()) {
                    pack.minecraft = if b == "0" {
                        format!("1.{a}")
                    } else {
                        format!("1.{a}.{b}")
                    };
                }
            }
            continue;
        }
        let folder = if jar.ends_with(".zip") {
            "datapacks"
        } else {
            "mods"
        };
        let text = |k: &str| e[k].as_str().filter(|v| !v.is_empty()).map(str::to_owned);
        pack.files.push(PackFile {
            path: format!("{folder}/{jar}"),
            sha1: text("modrinthHash"),
            mod_id: text("modId"),
            name: text("name"),
            version: text("version"),
            ..PackFile::default()
        });
    }
    pack.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(pack)
}

/// A Prism / MultiMC instance (`mmc-pack.json`), a game folder, or a
/// folder of jars: the jars on disk, hashed.
fn instance(dir: &Path) -> Result<Pack, String> {
    let mut pack = Pack {
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        format: "instance".into(),
        ..Pack::default()
    };
    if let Ok(text) = fs::read_to_string(dir.join("mmc-pack.json")) {
        let mmc: Value = serde_json::from_str(&text).map_err(|e| format!("mmc-pack.json: {e}"))?;
        for c in mmc["components"].as_array().into_iter().flatten() {
            let version = str_of(c, "version");
            match c["uid"].as_str().unwrap_or("") {
                "net.minecraft" => pack.minecraft = version,
                "net.neoforged" => {
                    (pack.loader, pack.loader_version) = ("neoforge".into(), version)
                }
                "net.minecraftforge" => {
                    (pack.loader, pack.loader_version) = ("forge".into(), version)
                }
                "net.fabricmc.fabric-loader" => {
                    (pack.loader, pack.loader_version) = ("fabric".into(), version)
                }
                "org.quiltmc.quilt-loader" => {
                    (pack.loader, pack.loader_version) = ("quilt".into(), version)
                }
                _ => {}
            }
        }
    }
    let game = game_dir(dir)?;
    let mods = if game.join("mods").is_dir() {
        game.join("mods")
    } else {
        game.clone()
    };
    let mut jars: Vec<PathBuf> = fs::read_dir(&mods)
        .map_err(|e| format!("{}: {e}", mods.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jar"))
        .collect();
    jars.sort();
    for jar in jars {
        let bytes = fs::read(&jar).map_err(|e| format!("{}: {e}", jar.display()))?;
        pack.files.push(PackFile {
            path: format!(
                "mods/{}",
                jar.file_name().unwrap_or_default().to_string_lossy()
            ),
            sha1: Some(sha1_hex(&bytes)),
            jar: Some(jar),
            ..PackFile::default()
        });
    }
    for top in ["config", "kubejs", "defaultconfigs", "datapacks"] {
        let n = count_files(&game.join(top));
        if n > 0 {
            pack.overrides.insert(top.into(), n);
        }
    }
    Ok(pack)
}

/// Resolve the game root for both mod and resource-pack instance imports.
pub(crate) fn game_dir(dir: &Path) -> Result<PathBuf, String> {
    [".minecraft", "minecraft", "."]
        .iter()
        .map(|g| dir.join(g))
        .find(|g| g.join("mods").is_dir() || g.join("resourcepacks").is_dir())
        .or_else(|| dir.read_dir().is_ok().then(|| dir.to_path_buf()))
        .ok_or_else(|| format!("{}: no game folder", dir.display()))
}

fn count_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() { count_files(&p) } else { 1 }
        })
        .sum()
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::Digest;
    sha1::Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_an_mrpack_index() {
        let index = json!({"name": "Small", "dependencies": {"minecraft": "1.21.1", "neoforge": "21.1.233"},
            "files": [{"path": "mods/create.jar", "hashes": {"sha1": "ab"}, "env": {"client": "required", "server": "required"},
                "downloads": ["https://cdn.modrinth.com/create.jar"]},
                {"path": "resourcepacks/look.zip", "hashes": {"sha1": "cd"}}]});
        let names = [
            "overrides/config/a.toml",
            "overrides/config/b/c.toml",
            "client-overrides/kubejs/x.js",
            "overrides/",
        ]
        .map(String::from);
        let p = mrpack(&index, &names);
        assert_eq!(
            (
                p.minecraft.as_str(),
                p.loader.as_str(),
                p.loader_version.as_str()
            ),
            ("1.21.1", "neoforge", "21.1.233")
        );
        assert_eq!(p.files.len(), 2);
        assert_eq!(
            (p.files[0].kind(), p.files[0].file_name()),
            ("mods", "create.jar")
        );
        assert_eq!(p.files[0].sha1.as_deref(), Some("ab"));
        assert_eq!(p.files[1].kind(), "resourcepacks");
        assert_eq!(
            p.overrides,
            BTreeMap::from([("config".into(), 2), ("kubejs".into(), 1)])
        );
    }

    #[test]
    fn reads_a_crash_assistant_mod_list() {
        let list = serde_json::json!({
            "neoforge-21.1.259 (modloader)": {"jarName": "neoforge-21.1.259 (modloader)",
                "modId": "neoforge", "version": "neoforge-21.1.259"},
            "mcw-doors-1.1.2.jar": {"jarName": "mcw-doors-1.1.2.jar", "modId": "mcwdoors",
                "modrinthHash": "0123456789abcdef0123456789abcdef01234567"},
            "sawmill.zip": {"jarName": "sawmill.zip"}
        });
        let pack = modlist(&list).unwrap();
        assert_eq!(
            (
                pack.minecraft.as_str(),
                pack.loader.as_str(),
                pack.loader_version.as_str()
            ),
            ("1.21.1", "neoforge", "21.1.259")
        );
        let paths: Vec<&str> = pack.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["datapacks/sawmill.zip", "mods/mcw-doors-1.1.2.jar"]);
        assert!(pack.files[1].sha1.is_some());
        assert_eq!(pack.files[1].mod_id.as_deref(), Some("mcwdoors"));
    }

    #[test]
    fn reads_a_curseforge_manifest() {
        let manifest = json!({"name": "CF", "overrides": "overrides",
            "minecraft": {"version": "1.21.1", "modLoaders": [{"id": "neoforge-21.1.259", "primary": true}]},
            "files": [{"projectID": 238222, "fileID": 5101366, "required": true}]});
        let p = curseforge(&manifest, &["overrides/kubejs/server_scripts/a.js".into()]);
        assert_eq!(
            (p.loader.as_str(), p.loader_version.as_str()),
            ("neoforge", "21.1.259")
        );
        assert_eq!(p.files[0].curseforge, Some((238222, 5101366)));
        assert_eq!(p.overrides["kubejs"], 1);
    }

    #[test]
    fn a_curseforge_zip_takes_names_from_its_mod_list() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("horizon-cf-zip-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pack.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        let manifest = json!({"name": "Pack", "overrides": "overrides",
            "minecraft": {"version": "1.21.1", "modLoaders": [{"id": "neoforge-21.1.259"}]},
            "files": [{"projectID": 1, "fileID": 2}, {"projectID": 3, "fileID": 4}]});
        zip.start_file("manifest.json", opts).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.start_file(format!("overrides/{MODLIST}"), opts)
            .unwrap();
        let list = json!({"mcw-doors-1.1.2.jar": {"jarName": "mcw-doors-1.1.2.jar",
            "modId": "mcwdoors", "modrinthHash": "0123456789abcdef0123456789abcdef01234567"}});
        // Crash Assistant writes a byte order mark.
        zip.write_all(format!("\u{feff}{list}").as_bytes()).unwrap();
        zip.finish().unwrap();
        let pack = import(&path).unwrap();
        assert_eq!(
            (pack.format.as_str(), pack.name.as_str()),
            ("curseforge", "Pack")
        );
        assert_eq!(pack.curseforge_files, 2);
        assert_eq!(pack.files.len(), 1);
        assert_eq!(pack.files[0].path, "mods/mcw-doors-1.1.2.jar");
        assert_eq!(pack.overrides["config"], 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
