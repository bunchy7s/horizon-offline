//! What a mod jar holds, read without decompiling: its metadata
//! (`neoforge.mods.toml`, `mods.toml`, `fabric.mod.json`, `mcmod.info`) and a
//! census of its data, assets and classes. Legacy metadata contributes identity
//! only; license and dependency fields are not interpreted.
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct JarInfo {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub license: String,
    /// Required dependencies by mod id (loaders and `minecraft` left out).
    pub depends: Vec<String>,
    pub census: Census,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Census {
    pub blockstates: usize,
    pub item_models: usize,
    pub recipes: usize,
    pub loot_tables: usize,
    pub tags: usize,
    pub worldgen: usize,
    pub structures: usize,
    pub languages: usize,
    pub textures: usize,
    pub sounds: usize,
    pub classes: usize,
    pub mixin_configs: usize,
}

const NOT_MODS: [&str; 5] = ["minecraft", "neoforge", "forge", "fabricloader", "java"];

pub fn read(path: &Path) -> Result<JarInfo, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = |zip: &mut zip::ZipArchive<fs::File>, name: &str| -> Option<String> {
        let mut s = String::new();
        zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let mut info = if let Some(t) = text(&mut zip, "META-INF/neoforge.mods.toml")
        .or_else(|| text(&mut zip, "META-INF/mods.toml"))
    {
        mods_toml(&t)?
    } else if let Some(t) = text(&mut zip, "fabric.mod.json") {
        fabric(&t)?
    } else if let Some(t) = text(&mut zip, "mcmod.info") {
        mcmod_info(&t)?
    } else {
        JarInfo::default()
    };
    // `version = "${file.jarVersion}"` is filled in from the manifest at load.
    if info.version.starts_with("${") {
        info.version = text(&mut zip, "META-INF/MANIFEST.MF")
            .and_then(|m| manifest_value(&m, "Implementation-Version"))
            .unwrap_or_default();
    }
    let names: Vec<String> = zip
        .file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    info.census = census(&names);
    if info.mod_id.is_empty() {
        info.mod_id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    Ok(info)
}

/// A main-section attribute of `META-INF/MANIFEST.MF`.
fn manifest_value(manifest: &str, key: &str) -> Option<String> {
    manifest
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// `[[mods]]` (the first one is the jar's mod), the top-level `license`, and
/// `[[dependencies.<id>]]` with `type = "required"` (or the older
/// `mandatory = true`).
pub fn mods_toml(text: &str) -> Result<JarInfo, String> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| format!("mods.toml: {e}"))?;
    let first = doc
        .get("mods")
        .and_then(|m| m.as_array())
        .and_then(|m| m.first())
        .and_then(|m| m.as_table());
    let get = |t: Option<&toml::Table>, k: &str| {
        t.and_then(|t| t.get(k))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned()
    };
    let mod_id = get(first, "modId");
    let mut depends = Vec::new();
    if let Some(deps) = doc
        .get("dependencies")
        .and_then(|d| d.get(&mod_id))
        .and_then(|d| d.as_array())
    {
        for d in deps.iter().filter_map(|d| d.as_table()) {
            let required = d.get("type").and_then(|t| t.as_str()).map_or_else(
                || d.get("mandatory").and_then(|m| m.as_bool()) == Some(true),
                |t| t.eq_ignore_ascii_case("required"),
            );
            let id = get(Some(d), "modId");
            if required && !NOT_MODS.contains(&id.as_str()) && !id.is_empty() {
                depends.push(id);
            }
        }
    }
    Ok(JarInfo {
        name: get(first, "displayName"),
        version: get(first, "version"),
        license: get(Some(&doc), "license"),
        mod_id,
        depends,
        census: Census::default(),
    })
}

pub fn fabric(text: &str) -> Result<JarInfo, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("fabric.mod.json: {e}"))?;
    let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
    let license = match &v["license"] {
        serde_json::Value::Array(l) => l
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(" OR "),
        l => l.as_str().unwrap_or("").to_owned(),
    };
    let depends = v["depends"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, _)| id.clone())
        .filter(|id| !NOT_MODS.contains(&id.as_str()) && !id.starts_with("fabric"))
        .collect();
    Ok(JarInfo {
        mod_id: s("id"),
        name: s("name"),
        version: s("version"),
        license,
        depends,
        census: Census::default(),
    })
}

/// Read the first mod's identity from legacy Forge metadata.
/// License and dependency fields remain unknown.
fn mcmod_info(text: &str) -> Result<JarInfo, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("mcmod.info: {e}"))?;
    let first = match &value {
        serde_json::Value::Array(mods) => mods.first(),
        serde_json::Value::Object(root) => root
            .get("modList")
            .and_then(serde_json::Value::as_array)
            .and_then(|mods| mods.first()),
        _ => None,
    }
    .and_then(serde_json::Value::as_object);
    let get = |key: &str| {
        first
            .and_then(|mod_info| mod_info.get(key))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    Ok(JarInfo {
        mod_id: get("modid"),
        name: get("name"),
        version: get("version"),
        ..JarInfo::default()
    })
}

pub fn census(names: &[String]) -> Census {
    let mut c = Census::default();
    for n in names {
        let parts: Vec<&str> = n.split('/').collect();
        if n.ends_with(".class") {
            c.classes += 1;
        } else if n.ends_with(".mixins.json") || (parts.len() == 1 && n.contains("mixins")) {
            c.mixin_configs += 1;
        }
        // assets/<ns>/<kind>/... and data/<ns>/<kind>/...
        let (Some(root), Some(kind)) = (parts.first(), parts.get(2)) else {
            continue;
        };
        let json = n.ends_with(".json");
        match (*root, *kind) {
            ("assets", "blockstates") if json => c.blockstates += 1,
            ("assets", "models") if json && parts.get(3) == Some(&"item") => c.item_models += 1,
            ("assets", "lang") => c.languages += 1,
            ("assets", "textures") if n.ends_with(".png") => c.textures += 1,
            ("assets", "sounds.json") => c.sounds += 1,
            ("data", "recipe" | "recipes") if json => c.recipes += 1,
            ("data", "loot_table" | "loot_tables") if json => c.loot_tables += 1,
            ("data", "tags") if json => c.tags += 1,
            ("data", "worldgen") if json => c.worldgen += 1,
            ("data", "structure" | "structures") if n.ends_with(".nbt") => c.structures += 1,
            _ => {}
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_JAR: AtomicU64 = AtomicU64::new(0);

    struct SyntheticJar(PathBuf, PathBuf);

    impl Drop for SyntheticJar {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_dir(&self.1);
        }
    }

    fn synthetic_jar(stem: &str, entries: &[(&str, &str)]) -> SyntheticJar {
        let id = NEXT_TEST_JAR.fetch_add(1, Ordering::Relaxed);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            ".jar-info-test-{}-{}-{id}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join(format!("{stem}.jar"));
        let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, contents) in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(contents.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
        SyntheticJar(path, dir)
    }

    #[test]
    fn reads_neoforge_metadata() {
        let info = mods_toml(
            r#"modLoader = "javafml"
license = "MIT"
[[mods]]
modId = "ruby"
version = "1.2.0"
displayName = "Ruby"
[[dependencies.ruby]]
modId = "neoforge"
type = "required"
[[dependencies.ruby]]
modId = "geckolib"
type = "required"
[[dependencies.ruby]]
modId = "jei"
type = "optional"
[[dependencies.ruby]]
modId = "curios"
mandatory = true
"#,
        )
        .unwrap();
        assert_eq!(
            (
                info.mod_id.as_str(),
                info.version.as_str(),
                info.license.as_str()
            ),
            ("ruby", "1.2.0", "MIT")
        );
        assert_eq!(info.depends, ["geckolib", "curios"]);
    }

    #[test]
    fn reads_fabric_metadata() {
        let info = fabric(
            r#"{"id": "gem", "version": "3", "name": "Gem", "license": ["MIT", "Apache-2.0"],
                "depends": {"fabricloader": "*", "fabric-api": "*", "minecraft": "1.21.1", "cloth-config": "*"}}"#,
        )
        .unwrap();
        assert_eq!(info.license, "MIT OR Apache-2.0");
        assert_eq!(info.depends, ["cloth-config"]);
    }

    #[test]
    fn reads_first_legacy_mod_from_root_array_without_inventing_license_or_dependencies() {
        let jar = synthetic_jar(
            "legacy-array",
            &[(
                "mcmod.info",
                r#"[
                    {"modid":"legacy_array","name":"Legacy Array","version":"1.2.3","requiredMods":["other_mod"]},
                    {"modid":"ignored_second","name":"Second Mod","version":"9.9"}
                ]"#,
            )],
        );

        let info = read(&jar.0).unwrap();

        assert_eq!(
            (
                info.mod_id.as_str(),
                info.name.as_str(),
                info.version.as_str()
            ),
            ("legacy_array", "Legacy Array", "1.2.3")
        );
        assert!(info.license.is_empty());
        assert!(
            info.depends.is_empty(),
            "legacy dependency fields are intentionally not interpreted"
        );
    }

    #[test]
    fn reads_first_legacy_mod_from_object_mod_list_and_keeps_unverified_license_empty() {
        let jar = synthetic_jar(
            "legacy-object",
            &[(
                "mcmod.info",
                r#"{
                    "modListVersion": 2,
                    "modList": [
                        {"modid":"legacy_object","name":"Legacy Object","version":"2.4","license":"unverified-custom"},
                        {"modid":"ignored_second","name":"Second Mod","version":"9.9"}
                    ]
                }"#,
            )],
        );

        let info = read(&jar.0).unwrap();

        assert_eq!(
            (
                info.mod_id.as_str(),
                info.name.as_str(),
                info.version.as_str()
            ),
            ("legacy_object", "Legacy Object", "2.4")
        );
        assert!(info.license.is_empty());
        assert!(info.depends.is_empty());
    }

    #[test]
    fn preserves_neoforge_forge_and_fabric_precedence_over_legacy_metadata() {
        let neoforge = synthetic_jar(
            "neoforge-precedence",
            &[
                (
                    "META-INF/neoforge.mods.toml",
                    "[[mods]]\nmodId = \"neo\"\nversion = \"neo-version\"\ndisplayName = \"NeoForge\"\n",
                ),
                (
                    "META-INF/mods.toml",
                    "[[mods]]\nmodId = \"forge\"\nversion = \"forge-version\"\ndisplayName = \"Forge\"\n",
                ),
                (
                    "fabric.mod.json",
                    r#"{"id":"fabric","name":"Fabric","version":"fabric-version"}"#,
                ),
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","name":"Legacy","version":"legacy-version"}]"#,
                ),
            ],
        );
        let forge = synthetic_jar(
            "forge-precedence",
            &[
                (
                    "META-INF/mods.toml",
                    "[[mods]]\nmodId = \"forge\"\nversion = \"forge-version\"\ndisplayName = \"Forge\"\n",
                ),
                (
                    "fabric.mod.json",
                    r#"{"id":"fabric","name":"Fabric","version":"fabric-version"}"#,
                ),
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","name":"Legacy","version":"legacy-version"}]"#,
                ),
            ],
        );
        let fabric = synthetic_jar(
            "fabric-precedence",
            &[
                (
                    "fabric.mod.json",
                    r#"{"id":"fabric","name":"Fabric","version":"fabric-version"}"#,
                ),
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","name":"Legacy","version":"legacy-version"}]"#,
                ),
            ],
        );

        let neo_info = read(&neoforge.0).unwrap();
        let forge_info = read(&forge.0).unwrap();
        let fabric_info = read(&fabric.0).unwrap();

        assert_eq!(
            (neo_info.mod_id.as_str(), neo_info.version.as_str()),
            ("neo", "neo-version")
        );
        assert_eq!(
            (forge_info.mod_id.as_str(), forge_info.version.as_str()),
            ("forge", "forge-version")
        );
        assert_eq!(
            (fabric_info.mod_id.as_str(), fabric_info.version.as_str()),
            ("fabric", "fabric-version")
        );
    }

    #[test]
    fn malformed_legacy_json_is_an_error_instead_of_filename_fallback() {
        let jar = synthetic_jar("malformed-legacy", &[("mcmod.info", "[{not-json]")]);

        let error = read(&jar.0).unwrap_err();

        assert!(error.starts_with("mcmod.info:"), "{error}");
    }

    #[test]
    fn missing_or_empty_legacy_mod_id_uses_the_jar_filename() {
        let missing = synthetic_jar(
            "missing-id",
            &[(
                "mcmod.info",
                r#"[{"name":"No Identifier","version":"1.0"}]"#,
            )],
        );
        let empty = synthetic_jar(
            "empty-id",
            &[(
                "mcmod.info",
                r#"[{"modid":"","name":"Empty Identifier","version":"1.1"}]"#,
            )],
        );

        let missing_info = read(&missing.0).unwrap();
        let empty_info = read(&empty.0).unwrap();

        assert_eq!(missing_info.mod_id, "missing-id");
        assert_eq!(empty_info.mod_id, "empty-id");
        assert_eq!(missing_info.name, "No Identifier");
        assert_eq!(missing_info.version, "1.0");
        assert_eq!(empty_info.name, "Empty Identifier");
        assert_eq!(empty_info.version, "1.1");
    }

    #[test]
    fn resolves_legacy_version_placeholder_from_manifest() {
        let jar = synthetic_jar(
            "legacy-manifest-version",
            &[
                (
                    "mcmod.info",
                    r#"[{"modid":"legacy","name":"Legacy","version":"${file.jarVersion}"}]"#,
                ),
                (
                    "META-INF/MANIFEST.MF",
                    "Manifest-Version: 1.0\r\nImplementation-Version: 6.2.1\r\n\r\n",
                ),
            ],
        );

        let info = read(&jar.0).unwrap();

        assert_eq!(info.version, "6.2.1");
    }

    #[test]
    fn counts_files_by_kind() {
        let names = [
            "assets/ruby/blockstates/ruby_block.json",
            "assets/ruby/models/item/ruby.json",
            "assets/ruby/models/block/ruby_block.json",
            "assets/ruby/textures/item/ruby.png",
            "assets/ruby/lang/en_us.json",
            "data/ruby/recipe/ruby.json",
            "data/ruby/loot_table/blocks/ruby_block.json",
            "data/minecraft/tags/block/mineable/pickaxe.json",
            "data/ruby/structure/hut.nbt",
            "com/example/Ruby.class",
            "ruby.mixins.json",
        ]
        .map(String::from);
        let c = census(&names);
        assert_eq!(
            (c.blockstates, c.item_models, c.textures, c.languages),
            (1, 1, 1, 1)
        );
        assert_eq!(
            (c.recipes, c.loot_tables, c.tags, c.structures),
            (1, 1, 1, 1)
        );
        assert_eq!((c.classes, c.mixin_configs), (1, 1));
    }
}
