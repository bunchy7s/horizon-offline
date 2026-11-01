//! `.hpack`, a Horizon modpack (docs/api/mod-index.md): registry mods
//! played together as a profile. A zip holding `horizon-modpack.toml`, which
//! names each mod by id, version and SHA-256 (the files themselves come from
//! the registry), and optionally `overrides/`: settings and resource packs
//! copied into the profile's game folder.
use crate::manifest::STORAGE_DIR;
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// The manifest's file name in a `.hpack`.
pub const MANIFEST: &str = "horizon-modpack.toml";
/// The manifest formats this build reads.
pub const FORMAT: u32 = 1;
const OVERRIDES: &str = "overrides/";
/// Bytes per override file and in all, unpacked.
const MAX_OVERRIDE: u64 = 64 << 20;
const MAX_OVERRIDES: u64 = 256 << 20;
/// The settings files an override may replace.
const SETTINGS: [&str; 3] = ["options.txt", "horizon-graphics.txt", "horizon-extras.txt"];

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    /// `[a-z0-9_]{1,64}`: its profile's folder.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The Minecraft version it is for.
    pub game: String,
    #[serde(default)]
    pub mods: Vec<PackMod>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackMod {
    pub id: String,
    pub version: String,
    pub sha256: String,
    /// Turned on in the profile (`horizon-mods.txt`).
    #[serde(default = "on")]
    pub on: bool,
}

fn on() -> bool {
    true
}

#[cfg(feature = "index")]
impl PackMod {
    pub fn requirement(&self) -> crate::index::Requirement {
        crate::index::Requirement {
            id: self.id.clone(),
            version: self.version.clone(),
            sha256: self.sha256.clone(),
        }
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        if m.format != FORMAT {
            return Err(format!(
                "{MANIFEST}: format {}; this Horizon reads {FORMAT}",
                m.format
            ));
        }
        let id_ok = |id: &str| {
            (1..=64).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        };
        if !id_ok(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        for (i, pm) in m.mods.iter().enumerate() {
            let hex = pm.sha256.len() == 64
                && pm
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !id_ok(&pm.id) || !hex || pm.version.is_empty() {
                return Err(format!(
                    "{MANIFEST}: mod {:?} needs an id, a version and a lowercase hex sha256",
                    pm.id
                ));
            }
            if m.mods[..i].iter().any(|other| other.id == pm.id) {
                return Err(format!("{MANIFEST}: mod {} is listed twice", pm.id));
            }
        }
        Ok(m)
    }
}

/// A `.hpack` as read from disk: its manifest and its override files, by
/// path under `overrides/`.
#[derive(Debug)]
pub struct Modpack {
    pub manifest: Manifest,
    pub overrides: Vec<String>,
}

impl Modpack {
    pub fn read(path: &Path) -> Result<Modpack, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut text = String::new();
        zip.by_name(MANIFEST)
            .map_err(|_| format!("{MANIFEST}: missing"))?
            .take(1 << 20)
            .read_to_string(&mut text)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        let manifest = Manifest::parse(&text)?;
        let (mut overrides, mut total) = (Vec::new(), 0u64);
        for i in 0..zip.len() {
            let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            if name == MANIFEST || entry.is_dir() {
                continue;
            }
            let Some(rest) = name.strip_prefix(OVERRIDES) else {
                return Err(format!(
                    "{name}: a modpack holds only {MANIFEST} and overrides/"
                ));
            };
            check_override(rest, &manifest)?;
            if entry.size() > MAX_OVERRIDE {
                return Err(format!("{name} is over {} MiB", MAX_OVERRIDE >> 20));
            }
            total += entry.size();
            overrides.push(rest.to_owned());
        }
        if total > MAX_OVERRIDES {
            return Err(format!("overrides are over {} MiB", MAX_OVERRIDES >> 20));
        }
        overrides.sort();
        Ok(Modpack {
            manifest,
            overrides,
        })
    }

    /// Writes the overrides of the pack at `path` into the game folder
    /// `home`, each checked again.
    pub fn extract_overrides(path: &Path, home: &Path) -> Result<Vec<PathBuf>, String> {
        let pack = Modpack::read(path)?;
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut written = Vec::new();
        for name in &pack.overrides {
            let mut entry = zip
                .by_name(&format!("{OVERRIDES}{name}"))
                .map_err(|e| format!("{name}: {e}"))?;
            let target = home.join(name);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            let mut out =
                std::fs::File::create(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            std::io::copy(&mut (&mut entry).take(MAX_OVERRIDE), &mut out)
                .map_err(|e| format!("{}: {e}", target.display()))?;
            written.push(target);
        }
        Ok(written)
    }
}

/// An override's path under `overrides/`: a settings file, a resource pack
/// zip, or the stored settings of one of the pack's mods. Plain parts only.
fn check_override(path: &str, manifest: &Manifest) -> Result<(), String> {
    let bad = || {
        format!(
            "overrides/{path}: only {}, resourcepacks/<name>.zip and horizon-mods/{STORAGE_DIR}/<a listed mod>/ may be overridden",
            SETTINGS.join(", ")
        )
    };
    let parts: Vec<&str> = path.split('/').collect();
    let plain = |p: &&str| {
        !p.is_empty()
            && !p.starts_with('.')
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-+ ".contains(&b))
    };
    let ok = match parts.as_slice() {
        [file] => SETTINGS.contains(file),
        ["resourcepacks", name] => plain(name) && name.ends_with(".zip"),
        ["horizon-mods", dir, id, rest @ ..] => {
            *dir == STORAGE_DIR
                && manifest.mods.iter().any(|m| m.id == *id)
                && !rest.is_empty()
                && rest.iter().all(plain)
        }
        _ => false,
    };
    if ok { Ok(()) } else { Err(bad()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const PACK: &str = r#"
format = 1
id = "cozy"
name = "Cozy"
version = "1.0.0"
game = "26.3"
[[mods]]
id = "coords_hud"
version = "0.1.0"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
"#;

    fn write_pack(dir: &Path, manifest: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("cozy.hpack");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(MANIFEST, options).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-hpack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_a_pack_and_writes_its_overrides() {
        let dir = scratch("read");
        let storage = format!("overrides/horizon-mods/{STORAGE_DIR}/coords_hud/corner.txt");
        let path = write_pack(
            &dir,
            PACK,
            &[
                ("overrides/options.txt", b"renderDistance:8\n"),
                ("overrides/resourcepacks/Cozy Look.zip", b"zip"),
                (&storage, b"top-left"),
            ],
        );
        let pack = Modpack::read(&path).unwrap();
        assert_eq!(pack.manifest.mods[0].id, "coords_hud");
        assert!(pack.manifest.mods[0].on);
        assert_eq!(pack.overrides.len(), 3);
        let home = dir.join("home");
        Modpack::extract_overrides(&path, &home).unwrap();
        let options = std::fs::read_to_string(home.join("options.txt")).unwrap();
        assert_eq!(options, "renderDistance:8\n");
        let corner = home.join(format!("horizon-mods/{STORAGE_DIR}/coords_hud/corner.txt"));
        assert!(corner.is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_what_a_pack_may_not_hold() {
        let dir = scratch("refuse");
        for name in [
            "mods/evil.jar",
            "overrides/../../.bashrc",
            "overrides/horizon-mods/evil.hmod",
            "overrides/resourcepacks/../options.txt",
            "overrides/servers.dat",
            "overrides/horizon-mods/.horizon-mod-data/other_mod/x",
            "overrides/resourcepacks/.hidden.zip",
        ] {
            let path = write_pack(&dir, PACK, &[(name, b"x")]);
            assert!(Modpack::read(&path).is_err(), "{name} was accepted");
        }
        let twice = format!("{PACK}{}", &PACK[PACK.find("[[mods]]").unwrap()..]);
        assert!(Manifest::parse(&twice).unwrap_err().contains("twice"));
        let newer = PACK.replace("format = 1", "format = 2");
        assert!(Manifest::parse(&newer).unwrap_err().contains("format 2"));
        let short = PACK.replace("0000000000000000", "00");
        assert!(Manifest::parse(&short).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
