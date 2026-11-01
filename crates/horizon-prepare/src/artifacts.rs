//! The player's own game files: the pinned version JSON, client jar and libraries from their
//! official launcher or ATLauncher install, and the server jar they downloaded, each checked against
//! research-lock.json before it is copied to `.local/minecraft/<version>` (tools/lab.py's
//! `prepare`, which the Python preparer ran first), then checked again on every run
//! (tools/test_rust_parity.py's `verify_artifacts`).

use crate::pyjson::{self, Value};
use crate::{digest, read_json, relative_display, write_json, Log};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

const LAUNCHER_HINT: &str =
    "Horizon does not download the game: play {version} once in the launcher you play \
Minecraft Java Edition with so it downloads the game's files, then rerun. Horizon reads the \
official launcher's folder, ATLauncher's and the CurseForge app's; for Prism Launcher, MultiMC, \
the Modrinth App or GDLauncher, prepare from the Horizon launcher, which finds their files. If your launcher keeps \
them in another folder, pass --minecraft-dir or set HORIZON_MINECRAFT_DIR, or name the files with \
--version-json, --client-jar, --libraries-dir and --assets-dir.";
const SERVER_HINT: &str = "Horizon does not download the game: download the {version} server jar yourself (minecraft.net/download/server \
while {version} is the current release, otherwise the downloads.server.url in your launcher's \
versions/{version}/{version}.json), then pass --server-jar PATH or set HORIZON_SERVER_JAR.";

fn hint(text: &str, version: &str) -> String {
    text.replace("{version}", version)
}

/// research-lock.json and the folder the pinned version's files go to.
pub struct Lock {
    pub json: Value,
    pub version: String,
    pub java_major: i64,
    pub directory: PathBuf,
}

impl Lock {
    pub fn read(root: &Path) -> Result<Lock, String> {
        let json = read_json(&root.join("research-lock.json"))?;
        let game = json
            .get("minecraft")
            .ok_or("research-lock.json has no minecraft")?;
        let version = game
            .get("version")
            .and_then(Value::as_str)
            .ok_or("research-lock.json has no minecraft version")?
            .to_string();
        if version.is_empty()
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || version == "."
            || version == ".."
        {
            return Err("Unsafe version identifier in lock file".into());
        }
        let java_major = game
            .get("java_major")
            .and_then(Value::as_i64)
            .ok_or("research-lock.json has no java_major")?;
        let directory = root.join(".local/minecraft").join(&version);
        Ok(Lock {
            json,
            version,
            java_major,
            directory,
        })
    }

    fn pin(&self, key: &str) -> Result<&str, String> {
        self.json
            .get("minecraft")
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("research-lock.json has no minecraft {key}"))
    }

    /// Mojang republishes a version JSON for asset-index changes; every known one is accepted.
    fn manifest_accepted(&self, checksum: &str) -> bool {
        self.pin("manifest_sha1").is_ok_and(|pin| pin == checksum)
            || self.listed("accepted_manifest_sha1", checksum)
    }

    /// The asset indexes of those republished version JSONs: a launcher keeps one
    /// `indexes/<id>.json`, the last it downloaded, whichever version JSON it reads.
    pub(crate) fn asset_index_accepted(&self, checksum: &str) -> bool {
        self.listed("accepted_asset_index_sha1", checksum)
    }

    fn listed(&self, key: &str, checksum: &str) -> bool {
        self.json
            .get("minecraft")
            .and_then(|g| g.get(key))
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(checksum)))
    }
}

fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Python's `Path.expanduser` for the forms players type: `~` and `~/...`.
fn expand_user(path: &Path) -> PathBuf {
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(first)) if first == "~" => home().join(components.as_path()),
        _ => path.to_path_buf(),
    }
}

/// The official launcher's game directory, which holds the player's own copy of the game.
pub fn minecraft_dir(chosen: Option<&Path>) -> PathBuf {
    if let Some(dir) = chosen.filter(|d| !d.as_os_str().is_empty()) {
        return expand_user(dir);
    }
    if let Some(dir) = std::env::var_os("HORIZON_MINECRAFT_DIR").filter(|d| !d.is_empty()) {
        return expand_user(Path::new(&dir));
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/minecraft")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join(".minecraft")
    } else {
        home().join(".minecraft")
    }
}

/// One launcher's copy of the pinned version: its version JSON (Mojang's file, checked by its
/// sha1), the client jar, and the folder holding its `libraries` (at the version JSON's paths)
/// and `assets`.
pub struct GameFiles {
    pub folder: PathBuf,
    pub manifest: PathBuf,
    pub client: PathBuf,
    /// Where the client's libraries are (`folder`'s `libraries`, or `--libraries-dir`).
    pub libraries: PathBuf,
    /// The asset store: `indexes/` and `objects/` (`folder`'s `assets`, or `--assets-dir`).
    pub assets: PathBuf,
    /// Mojang's Java runtimes the launcher downloaded (java::minecraft_runtimes).
    pub runtimes: PathBuf,
    /// The folders looked in, each with its launcher, when none held the version.
    searched: Vec<(&'static str, PathBuf)>,
}

/// Files named one by one (`--version-json`, `--client-jar`, `--libraries-dir`, `--assets-dir`),
/// for launchers whose layout is not one `game_files` knows: each replaces what it names.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub version_json: Option<PathBuf>,
    pub client_jar: Option<PathBuf>,
    pub libraries_dir: Option<PathBuf>,
    pub assets_dir: Option<PathBuf>,
}

impl GameFiles {
    /// These files with what `inputs` names instead.
    pub fn with(mut self, inputs: &Inputs) -> Self {
        let pick = |given: &Option<PathBuf>, found: &mut PathBuf| {
            if let Some(given) = given.as_ref().filter(|p| !p.as_os_str().is_empty()) {
                *found = expand_user(given);
            }
        };
        pick(&inputs.version_json, &mut self.manifest);
        pick(&inputs.client_jar, &mut self.client);
        pick(&inputs.libraries_dir, &mut self.libraries);
        pick(&inputs.assets_dir, &mut self.assets);
        self
    }
    /// What to do when these files are missing, with the folders looked in.
    pub(crate) fn hint(&self, version: &str) -> String {
        let mut text = hint(LAUNCHER_HINT, version);
        if !self.searched.is_empty() {
            text += &format!(" Horizon looked for {version} in:");
            for (launcher, folder) in &self.searched {
                text += &format!("\n  {} ({launcher})", folder.display());
            }
        }
        text
    }
}

/// The layouts horizon-prepare reads in `folder`: the official launcher's, then ATLauncher's,
/// which saves Mojang's version JSON as is in `configs/json/minecraft` and the client jar as
/// the library `net.minecraft:client:<version>`.
fn layouts(folder: &Path, version: &str) -> [GameFiles; 2] {
    let files = |manifest: String, client: String, runtimes: &str| GameFiles {
        folder: folder.to_path_buf(),
        manifest: folder.join(manifest),
        client: folder.join(client),
        libraries: folder.join("libraries"),
        assets: folder.join("assets"),
        runtimes: folder.join(runtimes),
        searched: Vec::new(),
    };
    let v = version;
    [
        files(
            format!("versions/{v}/{v}.json"),
            format!("versions/{v}/{v}.jar"),
            "runtime",
        ),
        files(
            format!("configs/json/minecraft/{v}.json"),
            format!("libraries/net/minecraft/client/{v}/client-{v}.jar"),
            "runtimes/minecraft",
        ),
    ]
}

/// The launchers whose folders are looked in when none is chosen: the official launcher's
/// (`minecraft_dir`), ATLauncher's, then the CurseForge app's `Install` folder, where it runs
/// the official launcher (its default modding folder, `curseforge/minecraft`, is in the home
/// folder on Windows and in Documents elsewhere). ATLauncher keeps its files in its working folder:
/// where its Windows installer puts it, inside the macOS app, and the folders its Linux
/// packages and Flatpak start it in.
fn launcher_folders() -> Vec<(&'static str, PathBuf)> {
    let mut folders = vec![("the official launcher", minecraft_dir(None))];
    // The same folders as the Horizon launcher's (launcher/src/sources.rs `known_folders`).
    let atlauncher: Vec<PathBuf> = if cfg!(target_os = "macos") {
        let app = Path::new("Applications/ATLauncher.app/Contents/Java");
        vec![
            home().join("Library/Application Support/ATLauncher"),
            Path::new("/").join(app),
            home().join(app),
        ]
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(|d| PathBuf::from(d).join("ATLauncher"))
            .into_iter()
            .collect()
    } else {
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|d| !d.is_empty())
            .map_or_else(|| home().join(".local/share"), PathBuf::from);
        let flatpak = home().join(".var/app/com.atlauncher.ATLauncher/data");
        vec![data.join("atlauncher"), flatpak.join("atlauncher"), flatpak]
    };
    folders.extend(atlauncher.into_iter().map(|dir| ("ATLauncher", dir)));
    let curseforge = if cfg!(windows) { "" } else { "Documents" };
    folders.push((
        "the CurseForge app",
        home().join(curseforge).join("curseforge/minecraft/Install"),
    ));
    folders
}

/// The pinned version's files in the chosen folder (`--minecraft-dir`, HORIZON_MINECRAFT_DIR),
/// or the launcher above it when an instance folder (`instances/<name>`) is chosen; with no
/// folder chosen, in the first launcher folder that has them. When none does, the official
/// layout of the first folder is returned, for the errors to name, with the folders looked in.
pub fn game_files(chosen: Option<&Path>, version: &str) -> GameFiles {
    let chosen = chosen
        .filter(|d| !d.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("HORIZON_MINECRAFT_DIR").map(PathBuf::from))
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| expand_user(&d));
    let folders = match chosen {
        Some(dir) => {
            let launcher = dir
                .ancestors()
                .find(|a| a.file_name().is_some_and(|n| n == "instances"))
                .and_then(Path::parent)
                .map(Path::to_path_buf);
            let mut folders = vec![("the folder chosen", dir)];
            folders.extend(launcher.map(|dir| ("the launcher above it", dir)));
            folders
        }
        None => launcher_folders(),
    };
    // A folder with both files first: a version JSON left without its jar is used, for the
    // error to name the jar, only when no folder has both.
    let complete = |files: &GameFiles| files.manifest.is_file() && files.client.is_file();
    let partial = |files: &GameFiles| files.manifest.is_file();
    for found in [complete, partial] {
        let mut all = folders
            .iter()
            .flat_map(|(_, folder)| layouts(folder, version));
        if let Some(files) = all.find(found) {
            return files;
        }
    }
    let [mut files, _] = layouts(&folders[0].1, version);
    files.searched = folders;
    files
}

/// Copy one file from the player's install, checked like a download; an existing verified
/// copy is kept.
pub(crate) fn import_copy(
    source: &Path,
    destination: &Path,
    checksum: &str,
    algorithm: &str,
    hint: &str,
) -> Result<PathBuf, String> {
    crate::files::Import::stage(source, destination, checksum, algorithm, false)
        .map_err(|error| {
            if !source.is_file() {
                format!("Missing {}. {hint}", source.display())
                    .trim()
                    .to_string()
            } else {
                error
            }
        })?
        .commit()
}

/// The pinned version JSON from the launcher's install. The launcher's copy wins over the
/// cached one so the manifest matches the asset index installed alongside it.
pub(crate) fn import_manifest(lock: &Lock, files: &GameFiles) -> Result<Value, String> {
    let version = &lock.version;
    let destination = lock.directory.join("manifest.json");
    let source = &files.manifest;
    if source.is_file() {
        let manifest = read_json(source)?;
        let downloads = manifest.get("downloads");
        let sha1 = |side: &str| {
            downloads
                .and_then(|d| d.get(side))
                .and_then(|d| d.get("sha1"))
                .and_then(Value::as_str)
        };
        if manifest.get("id").and_then(Value::as_str) != Some(version)
            || sha1("client") != Some(lock.pin("client_sha1")?)
            || sha1("server") != Some(lock.pin("server_sha1")?)
        {
            return Err(format!(
                "{} does not describe the pinned {version} release",
                source.display()
            ));
        }
        let checksum = digest(source, "sha1")?;
        if !lock.manifest_accepted(&checksum) {
            // Another launcher's rewrite of Mojang's file (HMCL and PCL add their own fields to
            // the same jars, #1772), or one Mojang republished: Mojang's own copy saved as the
            // imported one is used instead.
            let pinned = lock.pin("manifest_sha1")?;
            if !(destination.exists() && lock.manifest_accepted(&digest(&destination, "sha1")?)) {
                return Err(format!(
                    "{} is not a {version} version file Horizon knows (sha1 {checksum}): your \
                     launcher may have rewritten it, or Mojang republished it. Prepare from the \
                     Horizon launcher, which downloads Mojang's own, or save \
                     https://piston-meta.mojang.com/v1/packages/{pinned}/{version}.json as {} \
                     and prepare again.",
                    source.display(),
                    destination.display()
                ));
            }
        } else if !(destination.exists() && digest(&destination, "sha1")? == checksum) {
            fs::create_dir_all(&lock.directory).map_err(|e| e.to_string())?;
            fs::copy(source, &destination).map_err(|e| e.to_string())?;
        }
    } else if !(destination.exists() && lock.manifest_accepted(&digest(&destination, "sha1")?)) {
        return Err(format!(
            "Missing {}. {}",
            source.display(),
            files.hint(version)
        ));
    }
    let manifest = read_json(&destination)?;
    let java = manifest
        .get("javaVersion")
        .and_then(|j| j.get("majorVersion"))
        .and_then(Value::as_i64);
    if java != Some(lock.java_major) {
        return Err("Manifest Java version does not match lock file".into());
    }
    Ok(manifest)
}

fn import_server(lock: &Lock, server_jar: Option<&Path>) -> Result<PathBuf, String> {
    let version = &lock.version;
    let destination = lock.directory.join("jars/server.jar");
    let checksum = lock.pin("server_sha1")?;
    let hint = hint(SERVER_HINT, version);
    let source = server_jar
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("HORIZON_SERVER_JAR").map(PathBuf::from));
    let Some(source) = source else {
        if destination.exists() && digest(&destination, "sha1")? == checksum {
            return Ok(destination);
        }
        return Err(format!("No {version} server jar. {hint}"));
    };
    import_copy(&expand_user(&source), &destination, checksum, "sha1", &hint)
}

/// The launcher's `os.name` for this machine.
fn rule_system() -> &'static str {
    if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// This machine's architecture and its aliases, as Python's `platform.machine()` names them.
fn rule_architectures() -> Vec<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => vec!["x86_64", "amd64", "x64"],
        "aarch64" => vec!["arm64", "aarch64"],
        "x86" => vec!["i686"],
        other => vec![other],
    }
}

/// The kernel release Python's `platform.release()` reports (only old version JSONs ask).
fn os_release() -> String {
    #[cfg(unix)]
    {
        if let Ok(out) = std::process::Command::new("uname").arg("-r").output() {
            return String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }
    String::new()
}

/// The official launcher's rules: the last matching rule decides, no rules means allowed.
fn library_allowed(library: &Value) -> Result<bool, String> {
    let system = rule_system();
    let architectures = rule_architectures();
    let rules = library
        .get("rules")
        .and_then(Value::as_array)
        .unwrap_or(&[]);
    let mut allowed = rules.is_empty();
    for rule in rules {
        let empty = Value::Object(Vec::new());
        let os = rule.get("os").unwrap_or(&empty);
        if os.get("name").and_then(Value::as_str).unwrap_or(system) != system {
            continue;
        }
        if let Some(arch) = os.get("arch") {
            let pattern = regex::Regex::new(&format!("^(?:{})$", arch.as_str().unwrap_or("")))
                .map_err(|e| e.to_string())?;
            if !architectures.iter().any(|name| pattern.is_match(name)) {
                continue;
            }
        }
        if let Some(version) = os.get("version") {
            let pattern =
                regex::Regex::new(version.as_str().unwrap_or("")).map_err(|e| e.to_string())?;
            if !pattern.is_match(&os_release()) {
                continue;
            }
        }
        if rule.get("features").is_some_and(|f| match f {
            Value::Object(o) => !o.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Null | Value::Bool(false) => false,
            Value::Str(s) => !s.is_empty(),
            _ => true,
        }) {
            continue;
        }
        allowed = rule.get("action").and_then(Value::as_str) == Some("allow");
    }
    Ok(allowed)
}

fn classifier_ends_with(classifier: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        classifier == *name
            || classifier
                .strip_suffix(name)
                .is_some_and(|rest| rest.ends_with('-'))
    })
}

/// Modern manifests list x64 and ARM natives with OS-only rules; keep this machine's. A plain
/// natives-<os> jar is the x64 build when the manifest also lists an ARM variant of it.
fn native_library_matches(library: &Value, names: &[&str]) -> bool {
    let arm = rule_architectures().contains(&"arm64");
    let name = library.get("name").and_then(Value::as_str).unwrap_or("");
    let classifier = name.split(':').nth(3).unwrap_or("");
    if classifier_ends_with(classifier, &["arm64", "aarch64", "aarch_64"]) {
        return arm;
    }
    if classifier_ends_with(classifier, &["x86_64", "x64"]) {
        return !arm;
    }
    if arm && classifier.starts_with("natives-") {
        return !["arm64", "aarch64"]
            .iter()
            .any(|suffix| names.contains(&format!("{name}-{suffix}").as_str()));
    }
    true
}

/// The client libraries the official launcher installs on this OS and architecture: each
/// library's Maven name and its download.
fn client_library_artifacts(manifest: &Value) -> Result<Vec<(&str, &Value)>, String> {
    let libraries = manifest
        .get("libraries")
        .and_then(Value::as_array)
        .ok_or("The version JSON has no libraries")?;
    let names: Vec<&str> = libraries
        .iter()
        .filter_map(|l| l.get("name").and_then(Value::as_str))
        .collect();
    let mut artifacts = Vec::new();
    for library in libraries {
        let Some(artifact) = library.get("downloads").and_then(|d| d.get("artifact")) else {
            continue;
        };
        if library_allowed(library)? && native_library_matches(library, &names) {
            let name = library.get("name").and_then(Value::as_str).unwrap_or("");
            artifacts.push((name, artifact));
        }
    }
    Ok(artifacts)
}

/// Netty's native transports (`io.netty:netty-transport-native-kqueue:4.2.16.Final:osx-aarch_64`,
/// the epoll ones on Linux): the game uses one when it loads and Netty NIO otherwise
/// (`EventLoopGroupHolder.remote` asks `KQueue.isAvailable()`, then `Epoll.isAvailable()`), and
/// Prism Launcher never downloads them (#1487). Preparing imports one when the launcher has it.
fn runs_without(name: &str) -> bool {
    let mut parts = name.split(':');
    parts.next() == Some("io.netty")
        && parts
            .next()
            .is_some_and(|artifact| artifact.starts_with("netty-transport-native-"))
        && parts.nth(1).is_some()
}

/// A relative path with only normal components and forward slashes, from a manifest or bundle.
pub(crate) fn safe_relative(path: &str) -> Option<PathBuf> {
    if path.contains('\\') || path.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.contains(&"..") || (cfg!(windows) && parts.first().is_some_and(|p| p.contains(':'))) {
        return None;
    }
    Some(parts.iter().collect())
}

/// A launcher's copy of the library `name`, whose path in the version JSON is `relative`, under
/// its game folder `minecraft`: at that path, else where Prism Launcher keeps it (#849). Prism's
/// metadata moves a native jar's classifier into its artifact name
/// (`com.mojang:jtracy:1.14.38:natives-windows` is its `com.mojang:jtracy-natives-windows:1.14.38`)
/// and Prism keeps libraries at their name's path, so the same jar, with the same sha1, is in
/// `com/mojang/jtracy-natives-windows/1.14.38/jtracy-natives-windows-1.14.38.jar`. The path the
/// version JSON gives is returned when neither exists, for the error to name.
pub fn launcher_library(minecraft: &Path, name: &str, relative: &Path) -> PathBuf {
    library_in(&minecraft.join("libraries"), name, relative)
}

/// `launcher_library` in a libraries folder of its own.
fn library_in(libraries: &Path, name: &str, relative: &Path) -> PathBuf {
    let official = libraries.join(relative);
    if official.is_file() {
        return official;
    }
    if let [group, artifact, version, classifier] = name.split(':').collect::<Vec<_>>()[..] {
        let artifact = format!("{artifact}-{classifier}");
        let path = format!(
            "{}/{artifact}/{version}/{artifact}-{version}.jar",
            group.replace('.', "/")
        );
        if let Some(prism) = safe_relative(&path)
            .map(|p| libraries.join(p))
            .filter(|p| p.is_file())
        {
            return prism;
        }
    }
    official
}

fn artifact_fields(artifact: &Value) -> Result<(&str, &str), String> {
    let path = artifact.get("path").and_then(Value::as_str);
    let sha1 = artifact.get("sha1").and_then(Value::as_str);
    path.zip(sha1)
        .ok_or_else(|| "A library in the version JSON has no path or sha1".to_string())
}

/// `(sha256, identifier, path)` lines of a server bundle's versions.list or libraries.list.
fn bundle_entries(
    archive: &mut crate::Jar,
    name: &str,
) -> Result<Vec<(String, String, PathBuf, String)>, String> {
    let text = crate::zip_text(archive, name)?;
    let mut entries = Vec::new();
    for line in crate::split_lines(&text) {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [checksum, identifier, relative] = fields[..] else {
            return Err(format!("Malformed line in {name}"));
        };
        let path =
            safe_relative(relative).ok_or_else(|| format!("Unsafe bundled path: {relative}"))?;
        // PurePosixPath's text: empty and `.` parts dropped.
        let posix = relative
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect::<Vec<_>>()
            .join("/");
        entries.push((checksum.to_string(), identifier.to_string(), path, posix));
    }
    Ok(entries)
}

fn extract_verified(
    archive: &mut crate::Jar,
    member: &str,
    destination: &Path,
    checksum: &str,
) -> Result<(), String> {
    let data = crate::zip_bytes(archive, member)?;
    if crate::sha256_hex(&data) != checksum {
        return Err(format!("Bundled artifact checksum mismatch: {member}"));
    }
    fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(destination, data).map_err(|e| format!("{}: {e}", destination.display()))
}

/// Modern server downloads are launchers: the game jar and its libraries are inside.
fn unpack_server(
    server: PathBuf,
    directory: &Path,
    version: &str,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let mut archive = crate::open_zip(&server)?;
    if archive.index_for_name("META-INF/versions.list").is_none() {
        return Ok((server, Vec::new()));
    }
    let versions = bundle_entries(&mut archive, "META-INF/versions.list")?;
    if versions.len() != 1 || versions[0].1 != version {
        return Err("Expected one bundled server matching the pinned version".into());
    }
    let (checksum, _, _, relative) = &versions[0];
    let game = directory.join("jars/server-game.jar");
    extract_verified(
        &mut archive,
        &format!("META-INF/versions/{relative}"),
        &game,
        checksum,
    )?;
    let entries = bundle_entries(&mut archive, "META-INF/libraries.list")?;
    let libraries = crate::pool::map(&entries, |(checksum, _, path, relative)| {
        let destination = directory.join("libraries/server").join(path);
        extract_verified(
            &mut archive.clone(),
            &format!("META-INF/libraries/{relative}"),
            &destination,
            checksum.as_str(),
        )?;
        Ok(destination)
    })?;
    Ok((game, libraries))
}

/// Imports the player's own pinned game files into `.local/minecraft/<version>` and writes the
/// class inventories and prepared.json (tools/lab.py `prepare`). The decompiler lab.py also
/// fetches is a development tool, not something preparing needs; `tools/lab.py prepare` gets it.
pub fn import(
    root: &Path,
    lock: &Lock,
    minecraft: Option<&Path>,
    inputs: &Inputs,
    server_jar: Option<&Path>,
    log: Log,
) -> Result<(), String> {
    let files = game_files(minecraft, &lock.version).with(inputs);
    let manifest = import_manifest(lock, &files)?;
    let client = import_copy(
        &files.client,
        &lock.directory.join("jars/client.jar"),
        lock.pin("client_sha1")?,
        "sha1",
        &files.hint(&lock.version),
    )?;
    let downloads = manifest.get("downloads");
    if downloads.and_then(|d| d.get("client_mappings")).is_some()
        || downloads.and_then(|d| d.get("server_mappings")).is_some()
    {
        return Err(
            "This workflow targets unobfuscated releases. Older releases need a remapping stage."
                .into(),
        );
    }
    let id = manifest
        .get("id")
        .and_then(Value::as_str)
        .ok_or("The version JSON has no id")?;
    // The jar read, which --client-jar may have named outside the Minecraft folder.
    log(&format!(
        "Imported client {id} from {}",
        files.client.display()
    ));
    let server = import_server(lock, server_jar)?;
    let (server, server_libraries) = unpack_server(server, &lock.directory, id)?;
    let mut imports = Vec::new();
    for (name, artifact) in client_library_artifacts(&manifest)? {
        let (path, sha1) = artifact_fields(artifact)?;
        let relative = safe_relative(path).ok_or("Unsafe library path")?;
        let source = library_in(&files.libraries, name, &relative);
        let destination = lock.directory.join("libraries/client").join(&relative);
        if runs_without(name) && !source.is_file() && !destination.exists() {
            log(&format!(
                "No {name} in {}: the game runs without it",
                files.libraries.display()
            ));
            continue;
        }
        imports.push((source, destination, sha1));
    }
    let hint = files.hint(&lock.version);
    let client_libraries = crate::pool::map(&imports, |(source, destination, sha1)| {
        import_copy(source, destination, sha1, "sha1", &hint)
    })?;
    write_import_metadata(
        root,
        lock,
        &client,
        &server,
        &client_libraries,
        &server_libraries,
        log,
    )
}

fn write_import_metadata(
    root: &Path,
    lock: &Lock,
    client: &Path,
    server: &Path,
    client_libraries: &[PathBuf],
    server_libraries: &[PathBuf],
    log: Log,
) -> Result<(), String> {
    let sides = [("client", client), ("server", server)];
    let inventories = crate::pool::map(&sides, |(_, jar)| crate::inventory::inventory(jar))?;
    for ((side, _), data) in sides.into_iter().zip(inventories) {
        let count = |key| data.get(key).cloned().unwrap_or(Value::Null);
        write_json(
            &lock
                .directory
                .join(format!("reports/{side}-inventory.json")),
            &data,
        )?;
        log(&format!(
            "{side}: {} classes, {} methods",
            pyjson::dumps(&count("class_count"), &pyjson::DEFAULT),
            pyjson::dumps(&count("method_count"), &pyjson::DEFAULT)
        ));
    }
    let paths = |list: &[PathBuf]| -> Result<Value, String> {
        Ok(Value::Array(
            list.iter()
                .map(|p| relative_display(p, root).map(Value::Str))
                .collect::<Result<_, _>>()?,
        ))
    };
    write_json(
        &lock.directory.join("prepared.json"),
        &Value::Object(vec![
            ("version".into(), Value::str(&lock.version)),
            (
                "jars".into(),
                Value::Object(vec![
                    ("client".into(), Value::Str(relative_display(client, root)?)),
                    ("server".into(), Value::Str(relative_display(server, root)?)),
                ]),
            ),
            (
                "libraries".into(),
                Value::Object(vec![
                    ("client".into(), paths(client_libraries)?),
                    ("server".into(), paths(server_libraries)?),
                ]),
            ),
        ]),
    )?;
    log(&format!("Artifacts: {}", lock.directory.display()));
    Ok(())
}

/// The checked jars (`client`, `server` = the bundle's inner game jar) and the server
/// libraries, authenticated against the lock and the bundle's own checksums on every run.
pub struct Artifacts {
    pub manifest: Value,
    pub jars: HashMap<&'static str, PathBuf>,
    pub libraries: Vec<PathBuf>,
}

/// Reuses checked imports from this root or another installed version beside it.
pub fn reuse_imported(root: &Path, log: Log) -> Result<bool, String> {
    Ok(reuse_verified(root, log)?.is_some())
}

/// Keep this preparation's verified artifacts and client libraries for the exporters.
pub(crate) fn reuse_verified(
    root: &Path,
    log: Log,
) -> Result<Option<(Artifacts, Vec<PathBuf>)>, String> {
    let root = fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let lock = Lock::read(&root)?;
    let checked = |directory: PathBuf| -> Result<(Artifacts, Vec<PathBuf>), String> {
        let candidate = Lock {
            json: lock.json.clone(),
            version: lock.version.clone(),
            java_major: lock.java_major,
            directory,
        };
        let found = verify(&candidate)?;
        let client = client_libraries(&candidate, &found.manifest)?;
        Ok((found, client))
    };
    if let Ok((found, client_libraries)) = checked(lock.directory.clone()) {
        if [
            "prepared.json",
            "reports/client-inventory.json",
            "reports/server-inventory.json",
        ]
        .iter()
        .any(|path| !lock.directory.join(path).is_file())
        {
            write_import_metadata(
                &root,
                &lock,
                &found.jars["client"],
                &found.jars["server"],
                &client_libraries,
                &found.libraries,
                log,
            )?;
        }
        log(&format!("Using imported Minecraft {}", lock.version));
        return Ok(Some((found, client_libraries)));
    }
    let Some(versions) = root.parent() else {
        return Ok(None);
    };
    // Sibling discovery is intended for the launcher's versions/ folder.
    let mut siblings: Vec<_> = fs::read_dir(versions)
        .map_err(|e| format!("{}: {e}", versions.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path != &root && path.is_dir())
        .collect();
    siblings.sort();
    for sibling in siblings {
        let source = sibling.join(".local/minecraft").join(&lock.version);
        let Ok((found, client)) = checked(source.clone()) else {
            continue;
        };
        let mut files = vec![
            source.join("manifest.json"),
            source.join("jars/client.jar"),
            source.join("jars/server.jar"),
            found.jars["server"].clone(),
        ];
        files.extend(found.libraries);
        files.extend(client);
        for file in files {
            let relative = file.strip_prefix(&source).map_err(|e| e.to_string())?;
            import_copy(
                &file,
                &lock.directory.join(relative),
                &digest(&file, "sha256")?,
                "sha256",
                "",
            )?;
        }
        let found = verify(&lock)?;
        let client_libraries = client_libraries(&lock, &found.manifest)?;
        write_import_metadata(
            &root,
            &lock,
            &found.jars["client"],
            &found.jars["server"],
            &client_libraries,
            &found.libraries,
            log,
        )?;
        log(&format!(
            "Reused imported Minecraft {} from {}",
            lock.version,
            sibling.display()
        ));
        return Ok(Some((found, client_libraries)));
    }
    Ok(None)
}

pub fn verify(lock: &Lock) -> Result<Artifacts, String> {
    let manifest_path = lock.directory.join("manifest.json");
    if !manifest_path.exists() {
        return Err(format!(
            "{} is missing: prepare with --prepare first",
            manifest_path.display()
        ));
    }
    if !lock.manifest_accepted(&digest(&manifest_path, "sha1")?) {
        return Err("Manifest checksum changed; prepare again with --prepare".into());
    }
    let manifest = read_json(&manifest_path)?;
    let id = manifest.get("id").and_then(Value::as_str).unwrap_or("");
    if id != lock.version {
        return Err("Manifest version differs from the lock".into());
    }
    let mut jars = HashMap::new();
    let sides = ["client", "server"];
    let checked = crate::pool::map(&sides, |side| {
        let jar = lock.directory.join(format!("jars/{side}.jar"));
        let pinned = manifest
            .get("downloads")
            .and_then(|d| d.get(side))
            .and_then(|d| d.get("sha1"))
            .and_then(Value::as_str);
        let good = pinned == Some(lock.pin(&format!("{side}_sha1"))?)
            && jar.exists()
            && Some(digest(&jar, "sha1")?.as_str()) == pinned;
        Ok((good, jar))
    })?;
    for (side, (good, jar)) in sides.into_iter().zip(checked) {
        if !good {
            return Err(format!(
                "{side} artifact checksum changed; prepare again with --prepare"
            ));
        }
        jars.insert(side, jar);
    }
    let mut archive = crate::open_zip(&jars["server"])?;
    let versions = bundle_entries(&mut archive, "META-INF/versions.list")?;
    if versions.len() != 1 || versions[0].1 != id {
        return Err("Unexpected server bundle".into());
    }
    let game = lock.directory.join("jars/server-game.jar");
    if !game.exists() || digest(&game, "sha256")? != versions[0].0 {
        return Err("Inner server artifact checksum changed".into());
    }
    jars.insert("server", game);
    let entries = bundle_entries(&mut archive, "META-INF/libraries.list")?;
    let libraries = crate::pool::map(&entries, |(checksum, _, path, relative)| {
        let library = lock.directory.join("libraries/server").join(path);
        if !library.exists() || digest(&library, "sha256")? != *checksum {
            return Err(format!("Dependency checksum changed: {relative}"));
        }
        Ok(library)
    })?;
    Ok(Artifacts {
        manifest,
        jars,
        libraries,
    })
}

/// The client libraries, checked against the version JSON (tools/test_model_baking_parity.py's
/// `client_libraries`); the client-side exporters compile against them. A native transport the
/// player's launcher did not have is left out.
pub fn client_libraries(lock: &Lock, manifest: &Value) -> Result<Vec<PathBuf>, String> {
    let artifacts = client_library_artifacts(manifest)?;
    let libraries = crate::pool::map(&artifacts, |(name, artifact)| {
        let (path, sha1) = artifact_fields(artifact)?;
        let relative = safe_relative(path).ok_or("Unsafe client library path")?;
        let library = lock.directory.join("libraries/client").join(&relative);
        if runs_without(name) && !library.exists() {
            return Ok(None);
        }
        if !library.exists() || digest(&library, "sha1")? != sha1 {
            return Err(format!("Client dependency checksum changed: {path}"));
        }
        Ok(Some(library))
    })?;
    Ok(libraries.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(marker: &[u8]) -> Vec<u8> {
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive
            .start_file("marker", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(marker).unwrap();
        archive.finish().unwrap().into_inner()
    }

    struct Imports {
        directory: PathBuf,
        source: PathBuf,
        target: PathBuf,
    }

    impl Imports {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let directory = std::env::temp_dir()
                .join(format!("horizon-import-reuse-{}-{n}", std::process::id()));
            let source = directory.join("old");
            let target = directory.join("new");
            fs::create_dir_all(&target).unwrap();
            let imported = source.join(".local/minecraft/9.9");
            let write = |path: &str, data: &[u8]| {
                let path = imported.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, data).unwrap();
            };
            let client = jar(b"client");
            let game = jar(b"server game");
            write("jars/client.jar", &client);
            write("jars/server-game.jar", &game);
            write("libraries/server/server.jar", b"server library");
            write("libraries/client/client.jar", b"client library");
            write("reports/private.txt", b"not an imported artifact");
            let mut bundle =
                zip::ZipWriter::new(fs::File::create(imported.join("jars/server.jar")).unwrap());
            let entries = [
                (
                    "META-INF/versions.list",
                    format!("{}\t9.9\tgame.jar\n", crate::sha256_hex(&game)),
                ),
                (
                    "META-INF/libraries.list",
                    format!(
                        "{}\tserver:library:1\tserver.jar\n",
                        crate::sha256_hex(b"server library")
                    ),
                ),
            ];
            for (name, text) in entries {
                bundle
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                bundle.write_all(text.as_bytes()).unwrap();
            }
            for (name, bytes) in [
                ("META-INF/versions/game.jar", game.as_slice()),
                (
                    "META-INF/libraries/server.jar",
                    b"server library".as_slice(),
                ),
            ] {
                bundle
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                bundle.write_all(bytes).unwrap();
            }
            bundle.finish().unwrap();
            let client_sha1 = digest(&imported.join("jars/client.jar"), "sha1").unwrap();
            let server_sha1 = digest(&imported.join("jars/server.jar"), "sha1").unwrap();
            let library_sha1 =
                digest(&imported.join("libraries/client/client.jar"), "sha1").unwrap();
            let manifest = format!(
                "{{\"id\":\"9.9\",\"downloads\":{{\"client\":{{\"sha1\":\"{client_sha1}\"}},\"server\":{{\"sha1\":\"{server_sha1}\"}}}},\"libraries\":[{{\"name\":\"client:library:1\",\"downloads\":{{\"artifact\":{{\"path\":\"client.jar\",\"sha1\":\"{library_sha1}\"}}}}}}]}}"
            );
            write("manifest.json", manifest.as_bytes());
            let manifest_sha1 = digest(&imported.join("manifest.json"), "sha1").unwrap();
            let lock = format!(
                "{{\"minecraft\":{{\"version\":\"9.9\",\"java_major\":25,\"manifest_sha1\":\"{manifest_sha1}\",\"client_sha1\":\"{client_sha1}\",\"server_sha1\":\"{server_sha1}\"}}}}"
            );
            fs::write(target.join("research-lock.json"), lock).unwrap();
            Self {
                directory,
                source,
                target,
            }
        }

        fn imported(&self, root: &Path) -> PathBuf {
            root.join(".local/minecraft/9.9")
        }
    }

    impl Drop for Imports {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn imported_files_are_reused_without_the_official_install() {
        let fixture = Imports::new();
        fs::write(
            fixture.imported(&fixture.source).join("prepared.json"),
            b"stale source metadata",
        )
        .unwrap();
        assert!(reuse_imported(&fixture.target, &|_| {}).unwrap());
        let lock = Lock::read(&fixture.target).unwrap();
        let artifacts = verify(&lock).unwrap();
        assert_eq!(
            client_libraries(&lock, &artifacts.manifest).unwrap().len(),
            1
        );
        assert_eq!(artifacts.libraries.len(), 1);
        let prepared = read_json(&lock.directory.join("prepared.json")).unwrap();
        assert_eq!(prepared.get("version").and_then(Value::as_str), Some("9.9"));
        for side in ["client", "server"] {
            assert_eq!(
                prepared
                    .get("jars")
                    .unwrap()
                    .get(side)
                    .and_then(Value::as_str),
                Some(
                    relative_display(&artifacts.jars[side], &fixture.target)
                        .unwrap()
                        .as_str()
                )
            );
            let inventory = read_json(
                &lock
                    .directory
                    .join(format!("reports/{side}-inventory.json")),
            )
            .unwrap();
            assert_eq!(
                inventory.get("jar_sha256").and_then(Value::as_str),
                Some(digest(&artifacts.jars[side], "sha256").unwrap().as_str())
            );
        }
        assert!(!lock.directory.join("reports/private.txt").exists());
        for side in ["client", "server"] {
            let libraries = prepared
                .get("libraries")
                .unwrap()
                .get(side)
                .unwrap()
                .as_array()
                .unwrap();
            assert_eq!(libraries.len(), 1);
            assert_eq!(
                libraries[0].as_str(),
                Some(
                    relative_display(
                        &lock.directory.join(format!("libraries/{side}/{side}.jar")),
                        &fixture.target
                    )
                    .unwrap()
                    .as_str()
                )
            );
        }
        // The current root's valid imports suffice even after the older copy is corrupted.
        fs::write(
            fixture.imported(&fixture.source).join("jars/client.jar"),
            b"bad",
        )
        .unwrap();
        // Repair metadata omitted by earlier reuse implementations, without a sibling.
        fs::remove_file(lock.directory.join("prepared.json")).unwrap();
        fs::remove_file(lock.directory.join("reports/client-inventory.json")).unwrap();
        assert!(reuse_imported(&fixture.target, &|_| {}).unwrap());
        assert!(read_json(&lock.directory.join("prepared.json")).is_ok());
        assert!(read_json(&lock.directory.join("reports/client-inventory.json")).is_ok());
    }

    #[test]
    fn reuse_rejects_corrupt_imports() {
        for file in [
            "manifest.json",
            "jars/client.jar",
            "jars/server.jar",
            "jars/server-game.jar",
            "libraries/client/client.jar",
            "libraries/server/server.jar",
        ] {
            let fixture = Imports::new();
            fs::write(fixture.imported(&fixture.source).join(file), b"bad").unwrap();
            assert!(!reuse_imported(&fixture.target, &|_| {}).unwrap(), "{file}");
            assert!(!fixture.imported(&fixture.target).exists());
        }
    }

    #[test]
    fn reuse_checks_the_target_pins() {
        let fixture = Imports::new();
        let mut lock = Lock::read(&fixture.target).unwrap();
        let client = lock.pin("client_sha1").unwrap().to_string();
        let text = fs::read_to_string(fixture.target.join("research-lock.json")).unwrap();
        fs::write(
            fixture.target.join("research-lock.json"),
            text.replace(&client, "wrong-client-pin"),
        )
        .unwrap();
        assert!(!reuse_imported(&fixture.target, &|_| {}).unwrap());
        lock.directory = fixture.imported(&fixture.source);
        assert!(verify(&lock).is_ok());
    }

    #[test]
    fn reuse_skips_a_bad_sibling_and_repairs_bad_current_imports() {
        let fixture = Imports::new();
        let bad = fixture.directory.join("a-bad");
        fs::rename(&fixture.source, &bad).unwrap();
        let valid = Imports::new();
        fs::rename(&valid.source, &fixture.source).unwrap();
        fs::write(fixture.imported(&bad).join("jars/client.jar"), b"bad").unwrap();
        let target = fixture.imported(&fixture.target);
        fs::create_dir_all(target.join("jars")).unwrap();
        fs::write(target.join("jars/client.jar"), b"bad").unwrap();
        assert!(reuse_imported(&fixture.target, &|_| {}).unwrap());
        assert_eq!(
            fs::read(target.join("jars/client.jar")).unwrap(),
            fs::read(fixture.imported(&fixture.source).join("jars/client.jar")).unwrap()
        );
    }

    /// A native jar is found where the official launcher keeps it, else under the name Prism
    /// Launcher's metadata gives it (#849); a plain library only at the version JSON's path.
    #[test]
    fn libraries_are_found_where_prism_launcher_keeps_them() {
        let fixture = Imports::new();
        let minecraft = &fixture.directory;
        let official = Path::new("com/mojang/jtracy/1.14.38/jtracy-1.14.38-natives-windows.jar");
        let name = "com.mojang:jtracy:1.14.38:natives-windows";
        let prism = minecraft.join(
            "libraries/com/mojang/jtracy-natives-windows/1.14.38/jtracy-natives-windows-1.14.38.jar",
        );
        assert_eq!(
            launcher_library(minecraft, name, official),
            minecraft.join("libraries").join(official)
        );
        fs::create_dir_all(prism.parent().unwrap()).unwrap();
        fs::write(&prism, b"natives").unwrap();
        assert_eq!(launcher_library(minecraft, name, official), prism);
        let mojang = minecraft.join("libraries").join(official);
        fs::create_dir_all(mojang.parent().unwrap()).unwrap();
        fs::write(&mojang, b"natives").unwrap();
        assert_eq!(launcher_library(minecraft, name, official), mojang);
        let plain = Path::new("com/mojang/jtracy/1.14.38/jtracy-1.14.38.jar");
        assert_eq!(
            launcher_library(minecraft, "com.mojang:jtracy:1.14.38", plain),
            minecraft.join("libraries").join(plain)
        );
        for unsafe_name in ["a:..:1:x", "a:b\\c:1:x"] {
            let path = launcher_library(minecraft, unsafe_name, plain);
            assert_eq!(
                path,
                minecraft.join("libraries").join(plain),
                "{unsafe_name}"
            );
        }
    }

    /// A launcher that rewrites Mojang's version file in the official layout (HMCL, PCL: the same
    /// id and jars, its own fields, #1772) is refused with what to do, and Mojang's own copy saved
    /// as the imported one is then used.
    #[test]
    fn a_rewritten_version_file_falls_back_to_mojangs() {
        let root = std::env::temp_dir().join(format!("horizon-rewritten-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mojang = r#"{"id":"9.9","javaVersion":{"majorVersion":25},"downloads":{"client":{"sha1":"c"},"server":{"sha1":"s"}}}"#;
        let rewritten = r#"{"id":"9.9","javaVersion":{"majorVersion":25},"downloads":{"client":{"sha1":"c"},"server":{"sha1":"s"}},"patches":[],"root":true}"#;
        let minecraft = root.join("minecraft");
        fs::create_dir_all(minecraft.join("versions/9.9")).unwrap();
        fs::write(minecraft.join("versions/9.9/9.9.json"), rewritten).unwrap();
        fs::write(root.join("mojang.json"), mojang).unwrap();
        let pinned = digest(&root.join("mojang.json"), "sha1").unwrap();
        fs::write(
            root.join("research-lock.json"),
            format!(
                r#"{{"minecraft":{{"version":"9.9","java_major":25,"manifest_sha1":"{pinned}","client_sha1":"c","server_sha1":"s"}}}}"#
            ),
        )
        .unwrap();
        let lock = Lock::read(&root).unwrap();
        let destination = lock.directory.join("manifest.json");

        let error = import_manifest(&lock, &game_files(Some(&minecraft), "9.9")).unwrap_err();
        assert!(error.contains("may have rewritten it"), "{error}");
        assert!(
            error.contains(&format!("packages/{pinned}/9.9.json")),
            "{error}"
        );
        assert!(
            error.contains(&destination.display().to_string()),
            "{error}"
        );
        assert!(!destination.exists());

        fs::create_dir_all(&lock.directory).unwrap();
        fs::copy(root.join("mojang.json"), &destination).unwrap();
        let manifest = import_manifest(&lock, &game_files(Some(&minecraft), "9.9")).unwrap();
        assert!(manifest.get("patches").is_none());
        assert_eq!(digest(&destination, "sha1").unwrap(), pinned);
        fs::remove_dir_all(&root).unwrap();
    }

    /// scanfee's Prism Launcher 26.3 install on a Mac (#1487): Prism keeps natives under its own
    /// names and never downloads Netty's native transport, which the game runs without. Preparing
    /// still checks every other library, and the transport whenever the launcher has it.
    #[test]
    fn a_prism_install_without_netty_natives_prepares() {
        let fixture = Imports::new();
        let old = fixture.imported(&fixture.source);
        let prism = fixture.directory.join("PrismLauncher");
        let put = |path: &str, data: &[u8]| {
            let path = prism.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        put(
            "versions/9.9/9.9.jar",
            &fs::read(old.join("jars/client.jar")).unwrap(),
        );
        put("libraries/org/lib/lib/1/lib-1.jar", b"lib");
        put(
            "libraries/com/mojang/jtracy-natives-test/1/jtracy-natives-test-1.jar",
            b"jtracy natives",
        );
        let sha1 = |data: &[u8]| {
            let file = fixture.directory.join("sha1");
            fs::write(&file, data).unwrap();
            digest(&file, "sha1").unwrap()
        };
        let library = |name: &str, path: &str, sha1: &str, rules: &str| {
            format!(
                r#"{{"name":"{name}","downloads":{{"artifact":{{"path":"{path}","sha1":"{sha1}"}}}}{rules}}}"#
            )
        };
        let netty = |arch: &str| {
            library(
                &format!("io.netty:netty-transport-native-kqueue:1:osx-{arch}"),
                &format!("io/netty/netty-transport-native-kqueue/1/netty-transport-native-kqueue-1-osx-{arch}.jar"),
                &sha1(b"kqueue"),
                &format!(r#","rules":[{{"action":"allow","os":{{"name":"{}"}}}}]"#, rule_system()),
            )
        };
        let client_sha1 = digest(&old.join("jars/client.jar"), "sha1").unwrap();
        let server_sha1 = digest(&old.join("jars/server.jar"), "sha1").unwrap();
        let libraries = [
            library(
                "org.lib:lib:1",
                "org/lib/lib/1/lib-1.jar",
                &sha1(b"lib"),
                "",
            ),
            library(
                "com.mojang:jtracy:1:natives-test",
                "com/mojang/jtracy/1/jtracy-1-natives-test.jar",
                &sha1(b"jtracy natives"),
                "",
            ),
            netty("aarch_64"),
            netty("x86_64"),
        ];
        put(
            "versions/9.9/9.9.json",
            format!(
                r#"{{"id":"9.9","javaVersion":{{"majorVersion":25}},"downloads":{{"client":{{"sha1":"{client_sha1}"}},"server":{{"sha1":"{server_sha1}"}}}},"libraries":[{}]}}"#,
                libraries.join(",")
            )
            .as_bytes(),
        );
        let manifest_sha1 = digest(&prism.join("versions/9.9/9.9.json"), "sha1").unwrap();
        fs::write(
            fixture.target.join("research-lock.json"),
            format!(
                r#"{{"minecraft":{{"version":"9.9","java_major":25,"manifest_sha1":"{manifest_sha1}","client_sha1":"{client_sha1}","server_sha1":"{server_sha1}"}}}}"#
            ),
        )
        .unwrap();
        let lock = Lock::read(&fixture.target).unwrap();
        let server = old.join("jars/server.jar");
        let prepare = || {
            import(
                &fixture.target,
                &lock,
                Some(&prism),
                &Inputs::default(),
                Some(&server),
                &|_| {},
            )
        };
        let imported = || {
            let manifest = verify(&lock).unwrap().manifest;
            client_libraries(&lock, &manifest).unwrap().len()
        };

        prepare().unwrap();
        assert_eq!(imported(), 2);
        let prepared = read_json(&lock.directory.join("prepared.json")).unwrap();
        let listed = prepared.get("libraries").and_then(|l| l.get("client"));
        assert_eq!(
            listed.and_then(Value::as_array).map(<[Value]>::len),
            Some(2)
        );
        assert!(reuse_imported(&fixture.target, &|_| {}).unwrap());

        // The launcher's copy is used, and checked, when it has one.
        let manifest = read_json(&prism.join("versions/9.9/9.9.json")).unwrap();
        let (_, transport) = client_library_artifacts(&manifest)
            .unwrap()
            .into_iter()
            .find(|(name, _)| runs_without(name))
            .unwrap();
        let path = transport.get("path").and_then(Value::as_str).unwrap();
        put(&format!("libraries/{path}"), b"not kqueue");
        assert!(prepare().unwrap_err().contains("is not the pinned file"));
        put(&format!("libraries/{path}"), b"kqueue");
        prepare().unwrap();
        assert_eq!(imported(), 3);
        // Other libraries stay required.
        fs::remove_dir_all(lock.directory.join("libraries/client/org")).unwrap();
        fs::remove_file(prism.join("libraries/org/lib/lib/1/lib-1.jar")).unwrap();
        assert!(prepare().unwrap_err().starts_with("Missing "));
    }

    /// ATLauncher keeps Mojang's version JSON in `configs/json/minecraft` and the client jar as
    /// a library; its folder, or one of its instances, prepares like the official launcher's,
    /// with the same pins.
    #[test]
    fn an_atlauncher_install_prepares() {
        let fixture = Imports::new();
        let old = fixture.imported(&fixture.source);
        let atlauncher = fixture.directory.join("ATLauncher");
        let put = |path: &str, data: &[u8]| {
            let path = atlauncher.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        let client = fs::read(old.join("jars/client.jar")).unwrap();
        put("libraries/net/minecraft/client/9.9/client-9.9.jar", &client);
        put("libraries/client.jar", b"client library");
        put("instances/Minecraft99/instance.json", b"{}");
        // A version JSON left without its jar does not hide the launcher's complete copy.
        put("instances/Minecraft99/versions/9.9/9.9.json", b"{}");
        let manifest = fs::read_to_string(old.join("manifest.json")).unwrap();
        let manifest = manifest.replacen('{', r#"{"javaVersion":{"majorVersion":25},"#, 1);
        put("configs/json/minecraft/9.9.json", manifest.as_bytes());
        let pinned = digest(&atlauncher.join("configs/json/minecraft/9.9.json"), "sha1").unwrap();
        let lock_path = fixture.target.join("research-lock.json");
        let text = fs::read_to_string(&lock_path).unwrap();
        let old_sha1 = digest(&old.join("manifest.json"), "sha1").unwrap();
        fs::write(&lock_path, text.replace(&old_sha1, &pinned)).unwrap();
        let lock = Lock::read(&fixture.target).unwrap();
        let server = old.join("jars/server.jar");
        let prepare = |dir: &Path| {
            let _ = fs::remove_dir_all(&lock.directory);
            import(
                &fixture.target,
                &lock,
                Some(dir),
                &Inputs::default(),
                Some(&server),
                &|_| {},
            )
        };

        for chosen in [atlauncher.clone(), atlauncher.join("instances/Minecraft99")] {
            prepare(&chosen).unwrap();
            let artifacts = verify(&lock).unwrap();
            assert_eq!(fs::read(&artifacts.jars["client"]).unwrap(), client);
            assert_eq!(
                client_libraries(&lock, &artifacts.manifest).unwrap().len(),
                1
            );
        }
        let files = game_files(Some(&atlauncher), "9.9");
        assert_eq!(files.runtimes, atlauncher.join("runtimes/minecraft"));

        // The pins gate it as they gate the official layout.
        put(
            "libraries/net/minecraft/client/9.9/client-9.9.jar",
            b"other",
        );
        assert!(prepare(&atlauncher)
            .unwrap_err()
            .contains("is not the pinned file"));
        put("libraries/net/minecraft/client/9.9/client-9.9.jar", &client);
        put(
            "configs/json/minecraft/9.9.json",
            manifest.replace(',', ", ").as_bytes(),
        );
        assert!(prepare(&atlauncher)
            .unwrap_err()
            .contains("may have rewritten it"));
    }

    /// A launcher with a layout of its own names each file (`--version-json`, `--client-jar`,
    /// `--libraries-dir`): they prepare with the same pins, from no Minecraft folder at all.
    #[test]
    fn named_inputs_prepare_without_a_minecraft_folder() {
        let fixture = Imports::new();
        let old = fixture.imported(&fixture.source);
        let own = fixture.directory.join("OwnLauncher");
        let put = |path: &str, data: &[u8]| {
            let path = own.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        let client = fs::read(old.join("jars/client.jar")).unwrap();
        put("instance/minecraft.jar", &client);
        put("shared/libs/client.jar", b"client library");
        let manifest = fs::read_to_string(old.join("manifest.json")).unwrap();
        let manifest = manifest.replacen('{', r#"{"javaVersion":{"majorVersion":25},"#, 1);
        put("meta/9.9.json", manifest.as_bytes());
        let pinned = digest(&own.join("meta/9.9.json"), "sha1").unwrap();
        let lock_path = fixture.target.join("research-lock.json");
        let text = fs::read_to_string(&lock_path).unwrap();
        let old_sha1 = digest(&old.join("manifest.json"), "sha1").unwrap();
        fs::write(&lock_path, text.replace(&old_sha1, &pinned)).unwrap();
        let inputs = Inputs {
            version_json: Some(own.join("meta/9.9.json")),
            client_jar: Some(own.join("instance/minecraft.jar")),
            libraries_dir: Some(own.join("shared/libs")),
            assets_dir: None,
        };
        let lock = Lock::read(&fixture.target).unwrap();
        let server = old.join("jars/server.jar");
        let empty = fixture.directory.join("no-minecraft");
        fs::create_dir_all(&empty).unwrap();
        let prepare = |inputs: &Inputs| {
            let _ = fs::remove_dir_all(&lock.directory);
            import(
                &fixture.target,
                &lock,
                Some(&empty),
                inputs,
                Some(&server),
                &|_| {},
            )
        };

        prepare(&inputs).unwrap();
        let artifacts = verify(&lock).unwrap();
        assert_eq!(fs::read(&artifacts.jars["client"]).unwrap(), client);
        assert_eq!(
            client_libraries(&lock, &artifacts.manifest).unwrap().len(),
            1
        );
        // Without them the empty folder has nothing; a wrong jar fails its pin.
        assert!(prepare(&Inputs::default())
            .unwrap_err()
            .starts_with("Missing "));
        put("instance/minecraft.jar", b"other");
        assert!(prepare(&inputs)
            .unwrap_err()
            .contains("is not the pinned file"));
    }

    /// With no launcher's files in the chosen folder, the error names where Horizon looked and
    /// the launchers it reads.
    #[test]
    fn the_error_names_the_folders_looked_in() {
        let fixture = Imports::new();
        let empty = fixture.directory.join("instances/empty");
        fs::create_dir_all(&empty).unwrap();
        let lock = Lock::read(&fixture.target).unwrap();
        let error = import(
            &fixture.target,
            &lock,
            Some(&empty),
            &Inputs::default(),
            None,
            &|_| {},
        )
        .unwrap_err();
        assert!(error.starts_with(&format!(
            "Missing {}",
            empty.join("versions/9.9/9.9.json").display()
        )));
        assert!(error.contains("Horizon looked for 9.9 in:"), "{error}");
        assert!(error.contains(&format!("\n  {} (the folder chosen)", empty.display())));
        assert!(error.contains(&format!(
            "\n  {} (the launcher above it)",
            fixture.directory.display()
        )));
        assert!(error.contains("ATLauncher"));
    }

    #[test]
    fn every_launcher_horizon_reads_is_looked_in() {
        let launchers: Vec<_> = launcher_folders()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        for name in ["the official launcher", "ATLauncher", "the CurseForge app"] {
            assert!(launchers.contains(&name), "{name}");
        }
    }

    #[test]
    fn runs_without_only_netty_native_transports() {
        assert!(runs_without(
            "io.netty:netty-transport-native-kqueue:4.2.16.Final:osx-aarch_64"
        ));
        assert!(runs_without(
            "io.netty:netty-transport-native-epoll:4.2.16.Final:linux-x86_64"
        ));
        for required in [
            "io.netty:netty-transport-native-unix-common:4.2.16.Final",
            "io.netty:netty-transport-classes-kqueue:4.2.16.Final",
            "org.lwjgl:lwjgl:3.3.3:natives-macos-arm64",
            "com.mojang:jtracy:1.14.38:natives-windows",
        ] {
            assert!(!runs_without(required), "{required}");
        }
    }

    #[test]
    fn natives_follow_the_architecture() {
        let names = ["a:b:1:natives-macos", "a:b:1:natives-macos-arm64"];
        let lib = |name: &str| pyjson::loads(&format!("{{\"name\":\"{name}\"}}")).unwrap();
        let arm = rule_architectures().contains(&"arm64");
        assert_eq!(
            native_library_matches(&lib("a:b:1:natives-macos-arm64"), &names),
            arm
        );
        assert_eq!(
            native_library_matches(&lib("x:y:1:linux-x86_64"), &names),
            !arm
        );
        assert_eq!(
            native_library_matches(&lib("x:y:1:linux-aarch_64"), &names),
            arm
        );
        assert_eq!(
            native_library_matches(&lib("a:b:1:natives-macos"), &names),
            !arm
        );
        assert!(native_library_matches(&lib("a:b:1"), &names));
    }

    #[test]
    fn unsafe_paths_are_refused() {
        assert!(safe_relative("a/../b").is_none());
        assert!(safe_relative("/a").is_none());
        assert!(safe_relative("a\\b").is_none());
        assert_eq!(
            safe_relative("a/b.jar").unwrap(),
            Path::new("a").join("b.jar")
        );
    }
}
