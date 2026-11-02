//! Source-only native mixin bundles. Reading a bundle never executes its code.
use super::{Facts, valid_id};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, path::Path};

pub const MANIFEST: &str = "horizon-native.json";

/// Why a native mod may not carry `mod.wasm`: WebAssembly mods don't run any more.
pub const WASM_GONE: &str = "mod.wasm: WebAssembly mods don't run any more, port the code to a native mod (docs/native-mods.md)";

/// Whether `path` names a native mod's file: an `.hnmod`, or a `.hmod` holding native code
/// (one file for content and code, docs/native-mods.md).
pub fn is_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e == "hnmod" || e == "hmod")
}

/// Whether the mod file at `path` carries native code ([`MANIFEST`]): every `.hnmod`, and a
/// `.hmod` with code beside or instead of its content.
pub fn holds(path: &Path) -> bool {
    std::fs::File::open(path)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some_and(|mut zip| zip.by_name(MANIFEST).is_ok())
}

/// Whether a `.hmod`'s entry `name` is its content's (`horizon-mod.toml`, `assets/`, `data/`),
/// which [`crate::Content`] reads and checks, not the native build.
fn content(name: &str) -> bool {
    name == crate::manifest::MANIFEST || name.starts_with("assets/") || name.starts_with("data/")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    pub version: String,
    pub game: String,
    /// Exact source revisions tested together, never mutable tags.
    pub horizon: String,
    pub mixins: String,
    /// The version of Horizon's native hook points (`game/native_hooks.rs`,
    /// `NATIVE_HOOKS_API`) a mod that hooks nothing else needs: the launcher
    /// builds it on any Horizon with that version or a later one, and
    /// `horizon` is only the revision it was tested on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<u32>,
}

pub fn revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Manifest {
    pub fn check(&self) -> Result<(), String> {
        if self.schema != 1
            || !valid_id(&self.id)
            || self.version.is_empty()
            || self.game.is_empty()
            || !revision(&self.horizon)
            || !revision(&self.mixins)
            || self.hooks == Some(0)
        {
            return Err(
                "invalid native manifest: schema 1, id, version, game and exact revisions required"
                    .into(),
            );
        }
        Ok(())
    }

    /// Carries the exact native compatibility contract in the index's API field.
    pub fn api(&self) -> String {
        format!("native-1/{}/{}", self.horizon, self.mixins)
    }

    pub fn facts(&self) -> Facts {
        Facts {
            id: Some(self.id.clone()),
            version: Some(self.version.clone()),
            api: self.api(),
            game: Some(self.game.clone()),
            ..Facts::default()
        }
    }
}

pub struct Bundle {
    pub manifest: Manifest,
    /// Validated relative paths: Rust and TOML, plus files under `mixins/<id>/data/` or a
    /// `data/` folder deeper in (a crate's). A `.hmod`'s content files are not among them.
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Whether `rest` (a path inside `mixins/<id>/`) is a data file: under the bundle's `data/`, or
/// under the top-level `data/` of a folder of it (a crate's, `engine/data/`), not any folder
/// named `data` deeper in.
fn in_data(rest: &str) -> bool {
    let parts: Vec<&str> = rest.split('/').collect();
    parts.len() >= 2 && (parts[0] == "data" || (parts.len() >= 3 && parts[1] == "data"))
}

/// Whether `rest` is one of the text files a crate folder usually carries, which a crate may need
/// to compile (`#![doc = include_str!("../README.md")]`): `README*`, `LICENSE*`, `COPYING*`,
/// `NOTICE*` and `.gitignore`, in the bundle's folder or a top-level folder of it.
fn text(rest: &str) -> bool {
    let parts: Vec<&str> = rest.split('/').collect();
    let name = parts[parts.len() - 1];
    parts.len() <= 2
        && (["README", "LICENSE", "LICENCE", "COPYING", "NOTICE"]
            .iter()
            .any(|p| name.starts_with(p))
            || name == ".gitignore")
}

/// Whether a bundle may carry the file `rest` (its path under `mixins/<id>/`): sources and
/// manifests, data, and the text files of a crate folder.
pub fn allowed(rest: &str) -> bool {
    rest.ends_with(".rs") || rest.ends_with(".toml") || in_data(rest) || text(rest)
}

impl Bundle {
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut files = BTreeMap::new();
        let mut content_id = None;
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            if entry.is_dir() {
                continue;
            }
            if entry
                .unix_mode()
                .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
                || name.contains('\\')
                || name
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
                || name.starts_with('/')
                || name.contains(':')
            {
                return Err(format!("invalid native bundle path: {name}"));
            }
            if name == crate::manifest::WASM {
                return Err(WASM_GONE.into());
            }
            if content(&name) {
                if name == crate::manifest::MANIFEST {
                    let mut text = String::new();
                    (&mut entry)
                        .take(1 << 20)
                        .read_to_string(&mut text)
                        .map_err(|e| format!("{name}: {e}"))?;
                    content_id = Some(crate::Manifest::parse(&text)?.id);
                }
                continue;
            }
            if files.len() >= 512 {
                return Err("native bundle has too many entries".into());
            }
            let mut bytes = Vec::new();
            (&mut entry)
                .take(super::MAX_FILE + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            total += bytes.len() as u64;
            if total > super::MAX_FILE {
                return Err("native bundle expands beyond 64 MiB".into());
            }
            if files.insert(name, bytes).is_some() {
                return Err("duplicate native bundle path".into());
            }
        }
        let manifest: Manifest =
            serde_json::from_slice(files.get(MANIFEST).ok_or("missing horizon-native.json")?)
                .map_err(|e| e.to_string())?;
        manifest.check()?;
        if let Some(id) = content_id.filter(|id| *id != manifest.id) {
            return Err(format!(
                "its {} is mod {id}, its {MANIFEST} {}: one file is one mod",
                crate::manifest::MANIFEST,
                manifest.id
            ));
        }
        let prefix = format!("mixins/{}/", manifest.id);
        for name in files.keys().filter(|n| n.as_str() != MANIFEST) {
            let Some(rest) = name.strip_prefix(&prefix) else {
                return Err(format!("unexpected native source: {name}"));
            };
            if !allowed(rest) {
                return Err(format!("unexpected native source: {name}"));
            }
        }
        let source = files
            .get(&format!("{prefix}horizon-mixin.toml"))
            .ok_or("missing horizon-mixin.toml")?;
        let source: toml::Value =
            toml::from_str(std::str::from_utf8(source).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if source
            .get("mod")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            != Some(&manifest.id)
            || source
                .get("mod")
                .and_then(|v| v.get("version"))
                .and_then(|v| v.as_str())
                != Some(&manifest.version)
            || source
                .get("compatibility")
                .and_then(|v| v.get("horizon_revision"))
                .and_then(|v| v.as_str())
                != Some(&manifest.horizon)
        {
            return Err("native manifest and mixin identity/revision differ".into());
        }
        Ok(Self { manifest, files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn manifest() -> Manifest {
        Manifest {
            schema: 1,
            id: "veinminer".into(),
            version: "0.3.0".into(),
            game: "26.3".into(),
            horizon: "a".repeat(40),
            mixins: "b".repeat(40),
            hooks: None,
        }
    }
    fn read(name: &str, extras: &[(&str, &[u8])], mixin: &str) -> Result<Bundle, String> {
        let path =
            std::env::temp_dir().join(format!("native-bundle-{name}-{}.hnmod", std::process::id()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(MANIFEST, options).unwrap();
        zip.write_all(&serde_json::to_vec(&manifest()).unwrap())
            .unwrap();
        zip.start_file("mixins/veinminer/horizon-mixin.toml", options)
            .unwrap();
        zip.write_all(mixin.as_bytes()).unwrap();
        for (name, bytes) in extras {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let result = Bundle::read(&path);
        std::fs::remove_file(path).unwrap();
        result
    }
    fn mixin() -> String {
        format!(
            "[mod]\nid = \"veinminer\"\nversion = \"0.3.0\"\n[compatibility]\nhorizon_revision = \"{}\"\n",
            "a".repeat(40)
        )
    }
    #[test]
    fn source_bundle_carries_exact_compatibility_into_index_facts() {
        let bundle = read(
            "valid",
            &[("mixins/veinminer/hook.rs", b"fn hook() {}")],
            &mixin(),
        )
        .unwrap();
        assert_eq!(bundle.manifest, manifest());
        let facts = bundle.manifest.facts();
        assert_eq!(
            facts.api,
            format!("native-1/{}/{}", "a".repeat(40), "b".repeat(40))
        );
        assert_eq!(facts.id.as_deref(), Some("veinminer"));
        assert!(facts.capabilities.is_empty());
    }
    #[test]
    fn accepts_binary_data_in_the_bundle_data_directory() {
        let bytes = [0, 255, 42];
        let bundle = read(
            "data",
            &[
                ("mixins/veinminer/data/nested/table.bin", &bytes),
                ("mixins/veinminer/engine/data/model.bin", &bytes),
                ("mixins/veinminer/engine/README.md", b"# engine"),
                ("mixins/veinminer/engine/LICENSE-MIT", b"MIT"),
                ("mixins/veinminer/.gitignore", b"target/"),
            ],
            &mixin(),
        )
        .unwrap();
        assert_eq!(
            bundle.files["mixins/veinminer/data/nested/table.bin"],
            bytes
        );
    }
    #[test]
    fn rejects_path_traversal_and_non_source_payloads() {
        for (i, name) in [
            "../escape.rs",
            "/absolute.rs",
            "mixins/other/hook.rs",
            "mixins/veinminer/code.dll",
            "mixins/veinminer/data/../code.dll",
            "mixins/veinminer/database/file.bin",
            "mixins/veinminer/engine/data.bin",
            "mixins/veinminer/engine/src/data/blob.bin",
            "mixins/veinminer/engine/src/README.md",
            "mixins/veinminer/engine/README.dll/x.bin",
            "mixins/veinminer/engine/Cargo.lock",
            "mixins/other/data/file.bin",
            "mixins\\veinminer\\hook.rs",
        ]
        .iter()
        .enumerate()
        {
            assert!(read(&format!("bad-path-{i}"), &[(name, b"bad")], &mixin()).is_err());
        }
    }
    #[test]
    fn one_hmod_carries_content_beside_its_native_code() {
        let toml = r#"id = "veinminer"
name = "Veinminer"
version = "0.3.0"
api = "0.2"
capabilities = ["registries"]
"#;
        let bundle = read(
            "unified",
            &[
                ("horizon-mod.toml", toml.as_bytes()),
                ("assets/veinminer/lang/en_us.json", b"{}"),
                ("data/veinminer/tags/item/ores.json", b"{}"),
                ("mixins/veinminer/hook.rs", b"fn hook() {}"),
            ],
            &mixin(),
        )
        .unwrap();
        let names: Vec<&str> = bundle.files.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                MANIFEST,
                "mixins/veinminer/hook.rs",
                "mixins/veinminer/horizon-mixin.toml"
            ]
        );
        let other = toml.replace(r#""veinminer""#, r#""other""#);
        let e = read(
            "two-ids",
            &[("horizon-mod.toml", other.as_bytes())],
            &mixin(),
        )
        .err()
        .unwrap();
        assert!(e.contains("one file is one mod"), "{e}");
        let e = read("wasm", &[("mod.wasm", b"\0asm")], &mixin())
            .err()
            .unwrap();
        assert_eq!(e, WASM_GONE);
    }
    #[test]
    fn rejects_missing_or_mismatched_mixin_identity_without_panicking() {
        assert!(read("missing-mod", &[], "[unrelated]\nvalue = 1").is_err());
        assert!(
            read(
                "wrong-revision",
                &[],
                &mixin().replace(&"a".repeat(40), &"c".repeat(40))
            )
            .is_err()
        );
        assert!(read("wrong-version", &[], &mixin().replace("0.3.0", "0.2.0")).is_err());
        assert!(
            Manifest {
                horizon: "nightly".into(),
                ..manifest()
            }
            .check()
            .is_err()
        );
    }
    #[test]
    fn hooks_is_optional_and_kept() {
        let hooked = Manifest {
            hooks: Some(1),
            ..manifest()
        };
        assert!(hooked.check().is_ok());
        let text = serde_json::to_string(&hooked).unwrap();
        assert!(text.contains("\"hooks\":1"), "{text}");
        assert_eq!(serde_json::from_str::<Manifest>(&text).unwrap(), hooked);
        let plain = serde_json::to_string(&manifest()).unwrap();
        assert!(!plain.contains("hooks"), "{plain}");
        let zero = Manifest {
            hooks: Some(0),
            ..manifest()
        };
        assert!(zero.check().is_err());
    }
}
