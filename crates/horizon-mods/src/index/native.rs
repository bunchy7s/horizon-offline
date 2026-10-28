//! Source-only native mixin bundles. Reading a bundle never executes its code.
use super::{Facts, valid_id};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, path::Path};

pub const MANIFEST: &str = "horizon-native.json";

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
    /// Validated relative paths; only TOML and Rust beneath `mixins/<id>/`.
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Bundle {
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        if zip.len() > 512 {
            return Err("native bundle has too many entries".into());
        }
        let mut files = BTreeMap::new();
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().to_owned();
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
        let prefix = format!("mixins/{}/", manifest.id);
        for name in files.keys().filter(|n| n.as_str() != MANIFEST) {
            if !name.starts_with(&prefix) || !(name.ends_with(".rs") || name.ends_with(".toml")) {
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
    fn rejects_path_traversal_and_non_source_payloads() {
        for (i, name) in [
            "../escape.rs",
            "/absolute.rs",
            "mixins/other/hook.rs",
            "mixins/veinminer/code.dll",
            "mixins\\veinminer\\hook.rs",
        ]
        .iter()
        .enumerate()
        {
            assert!(read(&format!("bad-path-{i}"), &[(name, b"bad")], &mixin()).is_err());
        }
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
}
