//! Native mods' own crates and post-build steps (docs/native-mods.md, "Crates" and
//! "Post-build steps"). horizon-mixins stages a mod's `[[crate]]` as
//! `native-crates/<mod>/<crate>/`, a workspace member, and lists its `[[post_build]]` steps in
//! `native-post-build.json`. A mod's crate may use only what Horizon's own `Cargo.lock` has: the
//! lock file is the allow list, so a build never downloads a crate Horizon doesn't already ship.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
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
    let staged_manifests = manifests(&stage.join(DIR))?;
    for (mod_id, manifest) in &staged_manifests {
        let text =
            fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let value: toml::Table = text
            .parse()
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let mut tables: Vec<&toml::Value> = Vec::new();
        // Cargo still reads the underscore spellings before edition 2024.
        let kinds = [
            "dependencies",
            "dev-dependencies",
            "build-dependencies",
            "dev_dependencies",
            "build_dependencies",
        ];
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
            if let Some(path) = dep.get("path") {
                let path = path.as_str().ok_or_else(|| {
                    format!(
                        "{}: dependency {key} has a non-string path",
                        manifest.display()
                    )
                })?;
                if [
                    "git",
                    "registry",
                    "registry-index",
                    "branch",
                    "rev",
                    "tag",
                    "workspace",
                ]
                .iter()
                .any(|source| dep.get(*source).is_some())
                {
                    return Err(format!(
                        "{}: dependency {key} has conflicting source selectors",
                        manifest.display()
                    ));
                }
                let package = match dep.get("package") {
                    Some(value) => value.as_str().ok_or_else(|| {
                        format!(
                            "{}: dependency {key} has a non-string package name",
                            manifest.display()
                        )
                    })?,
                    None => key,
                };
                validate_sibling_path(mod_id, manifest, path, package, &staged_manifests)?;
                continue;
            }
            out.push(Wanted {
                mod_id: mod_id.to_owned(),
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
        let metadata = fs::symlink_metadata(d).map_err(|e| format!("{}: {e}", d.display()))?;
        if is_link_or_junction(&metadata) {
            return Err(format!("{}: linked native crate directory", d.display()));
        }
        if !metadata.is_dir() {
            return Err(format!("{}: not a native crate directory", d.display()));
        }
        let entries = fs::read_dir(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let mut paths = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| format!("{}: {e}", d.display()))?.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if is_link_or_junction(&metadata) {
                return Err(format!("{}: linked native crate entry", path.display()));
            }
            if metadata.is_dir() {
                paths.push(path);
            }
        }
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
            match fs::symlink_metadata(&manifest) {
                Ok(metadata) if is_link_or_junction(&metadata) => {
                    return Err(format!("{}: linked crate manifest", manifest.display()));
                }
                Ok(metadata) if metadata.is_file() => out.push((id.clone(), manifest)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("{}: {error}", manifest.display())),
            }
        }
    }
    Ok(out)
}

/// Only a literal `../<crate-id>` may name another staged crate. Its directory and manifest must
/// be ordinary entries in the same mod; never canonicalize a source path or follow a link.
fn validate_sibling_path(
    mod_id: &str,
    manifest: &Path,
    path: &str,
    package: &str,
    staged_manifests: &[(String, PathBuf)],
) -> Result<(), String> {
    let invalid = || {
        format!(
            "{}: dependency path {path:?} is not a declared sibling crate",
            manifest.display()
        )
    };
    let Some(crate_id) = path.strip_prefix("../") else {
        return Err(invalid());
    };
    if crate_id.is_empty() || crate_id.contains('/') || crate_id.contains('\\') {
        return Err(invalid());
    }
    let mut components = Path::new(path).components();
    if !matches!(components.next(), Some(Component::ParentDir))
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(invalid());
    }

    let crate_dir = manifest.parent().ok_or_else(invalid)?;
    let mod_dir = crate_dir.parent().ok_or_else(invalid)?;
    let sibling = mod_dir.join(crate_id);
    let sibling_metadata = fs::symlink_metadata(&sibling).map_err(|_| invalid())?;
    if is_link_or_junction(&sibling_metadata) || !sibling_metadata.is_dir() {
        return Err(invalid());
    }

    let sibling_manifest = sibling.join("Cargo.toml");
    let manifest_metadata = fs::symlink_metadata(&sibling_manifest).map_err(|_| invalid())?;
    if is_link_or_junction(&manifest_metadata) || !manifest_metadata.is_file() {
        return Err(invalid());
    }
    if !staged_manifests
        .iter()
        .any(|(staged_mod, staged)| staged_mod == mod_id && staged == &sibling_manifest)
    {
        return Err(invalid());
    }

    let text = fs::read_to_string(&sibling_manifest).map_err(|_| invalid())?;
    let value: toml::Table = text.parse().map_err(|_| invalid())?;
    let staged_package = value
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(invalid)?;
    if staged_package != package {
        return Err(invalid());
    }
    Ok(())
}

/// `symlink_metadata` reports links without following them. Windows junctions are reparse points
/// as well, so reject the reparse attribute even if the standard file type is not a symlink.
fn is_link_or_junction(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

/// Why a mod's crates can't build with Horizon's lock file `horizon`: a crate they name that it
/// lacks (checked before Cargo, which would look for it online), or a package the updated lock
/// `staged` has that Horizon's lacks or has in another version. None when they fit.
pub(super) fn refused(
    horizon: &BTreeSet<Locked>,
    staged: Option<&BTreeSet<Locked>>,
    wanted: &[Wanted],
    api_ids: &[&str],
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
        mods = wanted
            .iter()
            .map(|w| w.mod_id.as_str())
            .chain(api_ids.iter().copied())
            .collect();
        mods.sort_unstable();
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
    let dir = stage.join(DIR);
    match fs::symlink_metadata(&dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
        Ok(metadata) if is_link_or_junction(&metadata) || !metadata.is_dir() => true,
        Ok(_) => manifests(&dir).map_or(true, |m| !m.is_empty()),
    }
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_STAGE: AtomicUsize = AtomicUsize::new(0);

    struct StageFixture {
        root: PathBuf,
    }

    impl StageFixture {
        fn new() -> Self {
            let root = loop {
                let root = std::env::temp_dir().join(format!(
                    "horizon-native-sibling-{}-{}",
                    std::process::id(),
                    NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&root) {
                    Ok(()) => break root,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("{}: {error}", root.display()),
                }
            };
            Self { root }
        }

        fn root(&self) -> &Path {
            &self.root
        }

        fn add_crate(&self, mod_id: &str, crate_id: &str, package: &str, body: &str) -> PathBuf {
            let dir = self.root.join(DIR).join(mod_id).join(crate_id);
            write_manifest(&dir, package, body);
            dir
        }
    }

    impl Drop for StageFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_manifest(dir: &Path, package: &str, body: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = {package:?}\nversion = \"0.1.0\"\n{body}"),
        )
        .unwrap();
    }

    fn assert_rejected_path(label: &str, dependency: impl FnOnce(&StageFixture) -> String) {
        let stage = StageFixture::new();
        let entry = dependency(&stage);
        stage.add_crate(
            "bake",
            "engine",
            "bake-engine",
            &format!("[dependencies]\n{entry}\n"),
        );
        assert!(wanted(stage.root()).is_err(), "{label}");
    }

    #[cfg(any(unix, windows))]
    struct TestDirLink(PathBuf);

    #[cfg(any(unix, windows))]
    impl Drop for TestDirLink {
        fn drop(&mut self) {
            #[cfg(unix)]
            let _ = fs::remove_file(&self.0);
            #[cfg(windows)]
            let _ = fs::remove_dir(&self.0);
        }
    }

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
        assert_eq!(refused(&horizon, None, &ok, &[]), None);
        assert_eq!(refused(&horizon, Some(&horizon), &ok, &[]), None);
        // Asked for by name, and not in the lock: refused before Cargo looks online.
        let err = refused(&horizon, None, &[wants("bake", "rand", "0.9")], &[]).unwrap();
        assert!(
            err.starts_with("bake needs rand 0.9, which isn't in Horizon's Cargo.lock"),
            "{err}"
        );
        // In the lock, but the update had to change its version.
        let newer = LOCK.replace("1.0.140", "1.0.145");
        let err = refused(&horizon, Some(&downloaded(&newer).unwrap()), &ok, &[]).unwrap();
        assert!(err.starts_with("bake needs serde_json 1.0.145,"), "{err}");
        // Pulled in by another crate: every mod with crates is named.
        let added = format!(
            "{LOCK}\n[[package]]\nname = \"ryu\"\nversion = \"1.0.20\"\nsource = \"registry+x\"\n"
        );
        let two = [wants("bake", "serde_json", "1"), wants("zoom", "itoa", "1")];
        let err = refused(&horizon, Some(&downloaded(&added).unwrap()), &two, &[]).unwrap();
        assert!(err.starts_with("bake, zoom needs ryu 1.0.20,"), "{err}");
        // API-only bundles are named even when no mixin crate was scanned.
        let err = refused(
            &horizon,
            Some(&downloaded(&added).unwrap()),
            &[],
            &["trait_mod"],
        )
        .unwrap();
        assert!(err.starts_with("trait_mod needs ryu 1.0.20,"), "{err}");
        // A feature that needs another crate: Cargo's --locked refusal, said plainly.
        let cargo = "error: cannot update the lock file /s/Cargo.lock because --locked was passed to prevent this".to_string();
        assert!(locked(cargo.clone()).starts_with("A native mod's crate needs a crate"));
        assert!(locked(cargo).ends_with("to prevent this"));
        assert_eq!(locked("other".into()), "other");
        // Another checksum is another package.
        let other = LOCK.replace("\"abc\"", "\"abd\"");
        assert!(refused(&horizon, Some(&downloaded(&other).unwrap()), &ok, &[]).is_some());
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
             [dev_dependencies]\nryu = \"1\"\n\
             [target.'cfg(unix)'.dev-dependencies]\nlibc = \"0.2\"\n",
        )
        .unwrap();
        assert_eq!(
            wanted(&stage).unwrap(),
            [
                wants("bake", "itoa", "1"),
                wants("bake", "libc", "0.2"),
                wants("bake", "ryu", "1"),
                wants("bake", "serde_json", "1"),
                wants("bake", "sha2", "0.10"),
            ]
        );
        fs::create_dir_all(stage.join(DIR).join("gone/engine/src")).unwrap();
        assert_eq!(
            wanted(&stage).unwrap().len(),
            5,
            "an emptied crate folder is skipped"
        );
        assert_eq!(post_builds(&stage).unwrap(), []);
        assert!(any(&stage));
        fs::remove_file(krate.join("Cargo.toml")).unwrap();
        assert!(!any(&stage), "only emptied crate folders left");
    }

    #[test]
    fn any_is_false_when_native_crate_discovery_is_missing_or_empty() {
        let stage = StageFixture::new();
        assert!(
            !any(stage.root()),
            "a missing native-crates root has no crates"
        );
        fs::create_dir_all(stage.root().join(DIR)).unwrap();
        assert!(
            !any(stage.root()),
            "an empty native-crates root has no crates"
        );
        fs::create_dir_all(stage.root().join(DIR).join("bake")).unwrap();
        assert!(
            !any(stage.root()),
            "an empty mod folder has no crate manifests"
        );
    }

    #[test]
    fn wanted_omits_validated_sibling_paths_across_dependency_tables() {
        let stage = StageFixture::new();
        stage.add_crate(
            "bake",
            "engine",
            "bake-engine",
            "[dependencies]\n\
             same_name = { path = \"../same_name\", features = [\"enabled\"] }\n\
             industrial-sim = { path = \"../industrial_sim\", package = \"gtnh-lv-compressor-eval-industrial-sim\", version = \"0.1\", optional = true, default-features = false, features = [\"adapter\"] }\n\
             gtnh-lv-compressor-eval-industrial-sim = \"1\"\n\
             [dev-dependencies]\n\
             test-support = { path = \"../test_support\", package = \"test-support-package\", features = [\"fixtures\"] }\n\
             [build-dependencies]\n\
             build-helper = { path = \"../build_helper\", package = \"build-helper-package\", default-features = false }\n\
             [target.'cfg(unix)'.dependencies]\n\
             unix-helper = { path = \"../unix_helper\", package = \"unix-helper-package\", features = [\"unix\"] }\n\
             [target.'cfg(windows)'.dev-dependencies]\n\
             windows-test = { path = \"../windows_test\", package = \"windows-test-package\", features = [\"test\"] }\n\
             [target.'cfg(any())'.build-dependencies]\n\
             target-build = { path = \"../target_build\", package = \"target-build-package\" }\n",
        );
        for (crate_id, package) in [
            ("same_name", "same_name"),
            ("industrial_sim", "gtnh-lv-compressor-eval-industrial-sim"),
            ("test_support", "test-support-package"),
            ("build_helper", "build-helper-package"),
            ("unix_helper", "unix-helper-package"),
            ("windows_test", "windows-test-package"),
            ("target_build", "target-build-package"),
        ] {
            stage.add_crate("bake", crate_id, package, "");
        }

        // Same-name and renamed local dependencies in every dependency table are local. Feature
        // flags do not make them registry dependencies. The same bundled package without `path`
        // remains a registry request and must still pass the lock check.
        let wanted = wanted(stage.root()).unwrap();
        assert_eq!(
            wanted,
            [wants("bake", "gtnh-lv-compressor-eval-industrial-sim", "1")]
        );
        let horizon = downloaded(LOCK).unwrap();
        let err = refused(&horizon, None, &wanted, &[]).unwrap();
        assert!(err.starts_with(
            "bake needs gtnh-lv-compressor-eval-industrial-sim 1, which isn't in Horizon's Cargo.lock"
        ));
    }

    #[test]
    fn wanted_rejects_unknown_malformed_or_out_of_mod_sibling_paths() {
        assert_rejected_path("unknown sibling", |_| {
            "unknown = { path = \"../missing\" }".into()
        });
        assert_rejected_path("package alias does not match sibling manifest", |stage| {
            stage.add_crate("bake", "actual", "actual-package", "");
            "local = { path = \"../actual\", package = \"different-package\" }".into()
        });
        assert_rejected_path("absolute path", |stage| {
            let sibling = stage.add_crate("bake", "absolute", "absolute-package", "");
            format!(
                "local = {{ path = {:?}, package = \"absolute-package\" }}",
                sibling.to_string_lossy().to_string()
            )
        });
        assert_rejected_path("path escapes the mod directory", |stage| {
            write_manifest(&stage.root.join("outside"), "outside-package", "");
            "local = { path = \"../../../outside\", package = \"outside-package\" }".into()
        });
        assert_rejected_path("path crosses into another mod", |stage| {
            stage.add_crate("zoom", "foreign", "foreign-package", "");
            "local = { path = \"../../zoom/foreign\", package = \"foreign-package\" }".into()
        });
        assert_rejected_path("path is not a string", |_| "local = { path = 7 }".into());
        assert_rejected_path("sibling Cargo.toml is not a regular file", |stage| {
            let sibling = stage.root.join(DIR).join("bake").join("not_a_crate");
            fs::create_dir_all(sibling.join("Cargo.toml")).unwrap();
            "local = { path = \"../not_a_crate\" }".into()
        });
    }

    #[test]
    fn wanted_rejects_each_conflicting_path_source_selector() {
        let selectors = [
            ("git", "\"https://example.invalid/repo\""),
            ("registry", "\"crates-io\""),
            ("registry-index", "\"https://example.invalid/index\""),
            ("branch", "\"main\""),
            ("rev", "\"0123456789abcdef\""),
            ("tag", "\"v1.0.0\""),
            ("workspace", "true"),
        ];
        let accepted: Vec<&str> = selectors
            .iter()
            .filter_map(|(selector, value)| {
                let stage = StageFixture::new();
                stage.add_crate("bake", "local", "local-package", "");
                let dependency = format!(
                    "local = {{ path = \"../local\", package = \"local-package\", {selector} = {value} }}"
                );
                stage.add_crate(
                    "bake",
                    "engine",
                    "bake-engine",
                    &format!("[dependencies]\n{dependency}\n"),
                );
                wanted(stage.root()).is_ok().then_some(*selector)
            })
            .collect();
        assert!(
            accepted.is_empty(),
            "path dependencies accepted conflicting source selectors: {accepted:?}"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn wanted_rejects_sibling_paths_through_directory_links() {
        let stage = StageFixture::new();
        let outside = stage.root.join("linked-target");
        write_manifest(&outside, "linked-package", "");
        let engine = stage.add_crate("bake", "engine", "bake-engine", "");
        let link = engine.parent().unwrap().join("linked");
        let _link_guard = TestDirLink(link.clone());
        if let Err(error) = create_test_dir_link(&outside, &link) {
            eprintln!("SKIP directory-link sibling test: could not create link: {error}");
            return;
        }
        assert!(
            any(stage.root()),
            "a linked stage entry must reach the crate-validation gate"
        );
        fs::write(
            engine.join("Cargo.toml"),
            "[package]\nname = \"bake-engine\"\nversion = \"0.1.0\"\n\
             [dependencies]\nlocal = { path = \"../linked\", package = \"linked-package\" }\n",
        )
        .unwrap();
        assert!(wanted(stage.root()).is_err());
    }

    #[cfg(unix)]
    fn create_test_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_test_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
        crate::sources::junction(target, link)
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
