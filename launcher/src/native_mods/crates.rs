//! Native mods' own crates and post-build steps (docs/native-mods.md, "Crates" and
//! "Post-build steps"). horizon-mixins stages a mod's `[[crate]]` as
//! `native-crates/<mod>/<crate>/`, a workspace member, and lists its `[[post_build]]` steps in
//! `native-post-build.json`. A mod's crate may use only what Horizon's own `Cargo.lock` has: the
//! lock file is the allow list, so a build never downloads a crate Horizon doesn't already ship.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where horizon-mixins stages the mods' crates, in the workspace it writes.
pub(super) const DIR: &str = "native-crates";
/// The post-build steps it found, in mod order.
const POST_BUILD: &str = "native-post-build.json";

/// A package of a `Cargo.lock` from a registry (or git): the ones a build would download.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Locked {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: Option<String>,
}

/// The downloaded packages of the lock file `text`; the workspace's own (no `source`) are left
/// out.
pub(super) fn downloaded(text: &str) -> Result<BTreeSet<Locked>, String> {
    let lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
    let packages = lock.get("package").and_then(|p| p.as_array());
    let field =
        |p: &toml::Value, name: &str| p.get(name).and_then(|v| v.as_str()).map(String::from);
    Ok(packages
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(Locked {
                source: field(p, "source")?,
                name: field(p, "name")?,
                version: field(p, "version")?,
                checksum: field(p, "checksum"),
            })
        })
        .collect())
}

/// A crate a mod's crate asks for in its `Cargo.toml`: under its own name or `package`, in any
/// dependency table (dev and build ones too: the lock has them all).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Wanted {
    pub mod_id: String,
    pub name: String,
    pub version: String,
}

/// What every crate of `stage/native-crates/<mod>/<crate>/` asks for.
pub(super) fn wanted(stage: &Path) -> Result<Vec<Wanted>, String> {
    let mut out = Vec::new();
    for (mod_id, manifest) in manifests(&stage.join(DIR))? {
        let text =
            fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let value: toml::Table = text
            .parse()
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let mut tables: Vec<&toml::Value> = Vec::new();
        let kinds = ["dependencies", "dev-dependencies", "build-dependencies"];
        tables.extend(kinds.iter().filter_map(|k| value.get(*k)));
        for target in value
            .get("target")
            .and_then(|t| t.as_table())
            .into_iter()
            .flat_map(|t| t.values())
        {
            tables.extend(kinds.iter().filter_map(|k| target.get(*k)));
        }
        for (key, dep) in tables.iter().filter_map(|t| t.as_table()).flatten() {
            let get = |name: &str| dep.get(name).and_then(|v| v.as_str());
            out.push(Wanted {
                mod_id: mod_id.clone(),
                name: get("package").unwrap_or(key).to_owned(),
                version: dep.as_str().or(get("version")).unwrap_or("*").to_owned(),
            });
        }
    }
    out.sort();
    Ok(out)
}

/// `(mod id, its crate's Cargo.toml)` under `native-crates/`, sorted.
fn manifests(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out = Vec::new();
    let read = |d: &Path| -> Result<Vec<PathBuf>, String> {
        let mut paths: Vec<PathBuf> = fs::read_dir(d)
            .map_err(|e| format!("{}: {e}", d.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        paths.sort();
        Ok(paths)
    };
    for mod_dir in read(dir)? {
        let id = mod_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        // A folder left empty by an earlier build (the stage keeps folders) is no crate.
        for krate in read(&mod_dir)? {
            let manifest = krate.join("Cargo.toml");
            if manifest.is_file() {
                out.push((id.clone(), manifest));
            }
        }
    }
    Ok(out)
}

/// Why a mod's crates can't build with Horizon's lock file `horizon`: a crate they name that it
/// lacks (checked before Cargo, which would look for it online), or a package the updated lock
/// `staged` has that Horizon's lacks or has in another version. None when they fit.
pub(super) fn refused(
    horizon: &BTreeSet<Locked>,
    staged: Option<&BTreeSet<Locked>>,
    wanted: &[Wanted],
) -> Option<String> {
    let why = |mod_id: &str, name: &str, version: &str| {
        format!("{mod_id} needs {name} {version}, which isn't in Horizon's Cargo.lock. A native mod's crate can use only the crates Horizon itself uses, in the versions its Cargo.lock has.")
    };
    if let Some(w) = wanted
        .iter()
        .find(|w| !horizon.iter().any(|l| l.name == w.name))
    {
        return Some(why(&w.mod_id, &w.name, &w.version));
    }
    let added = staged?.difference(horizon).next()?;
    // A crate asked for by name is that mod's; one pulled in by another names every mod with crates.
    let mut mods: Vec<&str> = wanted
        .iter()
        .filter(|w| w.name == added.name)
        .map(|w| w.mod_id.as_str())
        .collect();
    if mods.is_empty() {
        mods = wanted.iter().map(|w| w.mod_id.as_str()).collect();
    }
    mods.dedup();
    Some(why(&mods.join(", "), &added.name, &added.version))
}

/// A Cargo error (`run`'s) said plainly when it is `--locked` refusing to change the lock file:
/// a mod's crate turned on, with a feature, a dependency Horizon's `Cargo.lock` lacks.
pub(super) fn locked(error: String) -> String {
    if !error.contains("--locked was passed") {
        return error;
    }
    format!("A native mod's crate needs a crate that isn't in Horizon's Cargo.lock, through one of its dependencies' features. A native mod's crate can use only the crates Horizon itself uses, with the features that need no others.\n\n{error}")
}

/// A mod's post-build step: a program of one of its crates, run after the game compiles.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub(super) struct PostBuild {
    #[serde(rename = "mod")]
    pub mod_id: String,
    pub package: String,
    pub bin: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// The post-build steps horizon-mixins listed in `stage` (none without the file: an older
/// horizon-mixins, or no mod has one).
pub(super) fn post_builds(stage: &Path) -> Result<Vec<PostBuild>, String> {
    let path = stage.join(POST_BUILD);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let steps: Vec<PostBuild> =
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            match steps
                .iter()
                .find(|s| !horizon_mods::index::valid_id(&s.mod_id))
            {
                Some(bad) => Err(format!(
                    "{}: invalid mod id {:?}",
                    path.display(),
                    bad.mod_id
                )),
                None => Ok(steps),
            }
        }
        // Only a missing file means none: any other error must not skip the steps.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Whether `stage` has any mod's crate: a build with none skips the crate check, even when an
/// earlier build left `native-crates/` behind (the stage keeps folders).
pub(super) fn any(stage: &Path) -> bool {
    manifests(&stage.join(DIR)).is_ok_and(|m| !m.is_empty())
}

/// Step `step`'s program `program` set up to run in the new build's version folder `version`,
/// with the mod's folders in its environment: `HORIZON_NATIVE_DATA_DIR` (what the game reads at
/// run time, in the version folder) and `HORIZON_NATIVE_CACHE_DIR` (kept across builds, under
/// `cache`). Both are created. `HORIZON_NATIVE_BUNDLE_DATA_DIR` is the bundle's own `data/`
/// as staged in `stage` (`native-data/<mod id>/`), which the step may read.
pub(super) fn post_build(
    program: &Path,
    step: &PostBuild,
    version: &Path,
    cache: &Path,
    stage: &Path,
) -> Result<Command, String> {
    let data = version.join("native-data").join(&step.mod_id);
    let cache = cache.join(&step.mod_id);
    for dir in [&data, &cache] {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut cmd = Command::new(program);
    crate::rust_toolchain::low_priority(&mut cmd);
    cmd.args(&step.args)
        .current_dir(version)
        .env("HORIZON_NATIVE_MOD", &step.mod_id)
        .env("HORIZON_NATIVE_VERSION_DIR", version)
        .env("HORIZON_NATIVE_DATA_DIR", &data)
        .env("HORIZON_NATIVE_CACHE_DIR", &cache)
        .env(
            "HORIZON_NATIVE_BUNDLE_DATA_DIR",
            stage.join("native-data").join(&step.mod_id),
        );
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"
version = 4

[[package]]
name = "horizon-client"
version = "0.1.0"

[[package]]
name = "serde_json"
version = "1.0.140"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc"

[[package]]
name = "itoa"
version = "1.0.15"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "def"
"#;

    fn wants(mod_id: &str, name: &str, version: &str) -> Wanted {
        Wanted {
            mod_id: mod_id.into(),
            name: name.into(),
            version: version.into(),
        }
    }

    #[test]
    fn a_mods_crate_may_use_only_what_horizons_lock_has() {
        let horizon = downloaded(LOCK).unwrap();
        assert_eq!(
            horizon.len(),
            2,
            "the workspace's own packages are left out"
        );
        let ok = [wants("bake", "serde_json", "1")];
        assert_eq!(refused(&horizon, None, &ok), None);
        assert_eq!(refused(&horizon, Some(&horizon), &ok), None);
        // Asked for by name, and not in the lock: refused before Cargo looks online.
        let err = refused(&horizon, None, &[wants("bake", "rand", "0.9")]).unwrap();
        assert!(
            err.starts_with("bake needs rand 0.9, which isn't in Horizon's Cargo.lock"),
            "{err}"
        );
        // In the lock, but the update had to change its version.
        let newer = LOCK.replace("1.0.140", "1.0.145");
        let err = refused(&horizon, Some(&downloaded(&newer).unwrap()), &ok).unwrap();
        assert!(err.starts_with("bake needs serde_json 1.0.145,"), "{err}");
        // Pulled in by another crate: every mod with crates is named.
        let added = format!(
            "{LOCK}\n[[package]]\nname = \"ryu\"\nversion = \"1.0.20\"\nsource = \"registry+x\"\n"
        );
        let two = [wants("bake", "serde_json", "1"), wants("zoom", "itoa", "1")];
        let err = refused(&horizon, Some(&downloaded(&added).unwrap()), &two).unwrap();
        assert!(err.starts_with("bake, zoom needs ryu 1.0.20,"), "{err}");
        // A feature that needs another crate: Cargo's --locked refusal, said plainly.
        let cargo = "error: cannot update the lock file /s/Cargo.lock because --locked was passed to prevent this".to_string();
        assert!(locked(cargo.clone()).starts_with("A native mod's crate needs a crate"));
        assert!(locked(cargo).ends_with("to prevent this"));
        assert_eq!(locked("other".into()), "other");
        // Another checksum is another package.
        let other = LOCK.replace("\"abc\"", "\"abd\"");
        assert!(refused(&horizon, Some(&downloaded(&other).unwrap()), &ok).is_some());
    }

    #[test]
    fn every_dependency_table_of_every_mod_crate_is_read() {
        let stage =
            std::env::temp_dir().join(format!("horizon-native-crates-{}", std::process::id()));
        let _ = fs::remove_dir_all(&stage);
        let krate = stage.join(DIR).join("bake").join("engine");
        fs::create_dir_all(&krate).unwrap();
        fs::write(
            krate.join("Cargo.toml"),
            "[package]\nname = \"bake-engine\"\nversion = \"0.1.0\"\n\
             [dependencies]\nserde_json = \"1\"\nhash = { package = \"sha2\", version = \"0.10\" }\n\
             [build-dependencies]\nitoa = { version = \"1\" }\n\
             [target.'cfg(unix)'.dev-dependencies]\nlibc = \"0.2\"\n",
        )
        .unwrap();
        assert_eq!(
            wanted(&stage).unwrap(),
            [
                wants("bake", "itoa", "1"),
                wants("bake", "libc", "0.2"),
                wants("bake", "serde_json", "1"),
                wants("bake", "sha2", "0.10"),
            ]
        );
        fs::create_dir_all(stage.join(DIR).join("gone/engine/src")).unwrap();
        assert_eq!(
            wanted(&stage).unwrap().len(),
            4,
            "an emptied crate folder is skipped"
        );
        assert_eq!(post_builds(&stage).unwrap(), []);
        assert!(any(&stage));
        fs::remove_file(krate.join("Cargo.toml")).unwrap();
        assert!(!any(&stage), "only emptied crate folders left");
    }

    #[test]
    fn post_build_steps_run_in_the_version_folder_with_the_mods_folders() {
        let json = r#"[{"mod":"bake","package":"bake-engine","bin":"bake","args":["--fast"]},
            {"mod":"zoom","package":"zoom-tables","bin":"tables"}]"#;
        let root = std::env::temp_dir().join(format!("horizon-native-post-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(POST_BUILD), json).unwrap();
        let steps = post_builds(&root).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1].args, Vec::<String>::new());
        let version = root.join("versions/native-abc");
        let cmd = post_build(
            Path::new("/target/fast/bake"),
            &steps[0],
            &version,
            &root.join("cache"),
            &root.join("stage"),
        )
        .unwrap();
        assert_eq!(cmd.get_current_dir(), Some(version.as_path()));
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["--fast"]);
        let env = |name: &str| {
            cmd.get_envs()
                .find(|(k, _)| *k == name)
                .and_then(|(_, v)| v)
                .map(PathBuf::from)
        };
        assert_eq!(env("HORIZON_NATIVE_MOD"), Some("bake".into()));
        assert_eq!(env("HORIZON_NATIVE_VERSION_DIR"), Some(version.clone()));
        assert_eq!(
            env("HORIZON_NATIVE_DATA_DIR"),
            Some(version.join("native-data/bake"))
        );
        assert_eq!(
            env("HORIZON_NATIVE_CACHE_DIR"),
            Some(root.join("cache/bake"))
        );
        assert_eq!(
            env("HORIZON_NATIVE_BUNDLE_DATA_DIR"),
            Some(root.join("stage/native-data/bake"))
        );
        assert!(version.join("native-data/bake").is_dir() && root.join("cache/bake").is_dir());
        fs::write(root.join(POST_BUILD), "{}").unwrap();
        assert!(post_builds(&root).is_err());
        // A mod id that would leave its folder is refused.
        let bad = r#"[{"mod":"../x","package":"p","bin":"b"}]"#;
        fs::write(root.join(POST_BUILD), bad).unwrap();
        assert!(post_builds(&root).is_err());
        // A file that can't be read is an error, not "no steps".
        fs::remove_file(root.join(POST_BUILD)).unwrap();
        fs::create_dir(root.join(POST_BUILD)).unwrap();
        assert!(post_builds(&root).is_err());
    }
}
