//! The player's own download of the pinned version from Mojang, for a signed-in account that
//! owns Minecraft: Java Edition (#434), as other launchers get it: the version JSON, the client
//! jar, this system's libraries and the assets (the sounds), each checked against its sha1,
//! in the official launcher's layout in the launcher's data folder. sources.rs then finds them
//! there like any launcher's. Nothing of Mojang's is part of Horizon.
//!
//! Each file is checked against the size and sha1 Mojang lists, and written whole. A download
//! stopped half way (cancelled, no network) keeps the files it finished: the next one fetches
//! only the rest.

use crate::sources::{fetch_checked, fetch_manifest, sha1_file, sha1_hex, write_file, Fetch, Pins};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

const OBJECTS: &str = "https://resources.download.minecraft.net";
/// The index of Mojang's Java runtimes, the one the official launcher reads.
const JAVA_RUNTIMES: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Set to stop the download under way (the window's Cancel); cleared when a preparation starts.
pub static CANCEL: AtomicBool = AtomicBool::new(false);
/// The download under way: files done, of how many (0 when none runs).
static DONE: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);

/// How far the download under way is, files done of how many; none when none runs.
pub fn progress() -> Option<(usize, usize)> {
    let total = TOTAL.load(Relaxed);
    (total > 0).then(|| (DONE.load(Relaxed).min(total), total))
}

const CANCELLED: &str = "Download cancelled. Download again to go on where it stopped.";

/// One file to download, with the size Mojang lists for it when it lists one.
struct File {
    url: String,
    sha1: String,
    size: Option<u64>,
    path: PathBuf,
}

/// Downloads the pinned version into `dir`, `jobs` files at a time; files already there and
/// whole are kept. An asset that fails is a line in the log (the game is then partly silent),
/// anything else an error. `cancel` stops it between files.
pub fn download(
    pins: &Pins,
    dir: &Path,
    os: &str,
    jobs: usize,
    fetch: Fetch,
    cancel: &AtomicBool,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let v = &pins.version;
    let versions = dir.join("versions").join(v);
    let manifest = versions.join(format!("{v}.json"));
    fetch_manifest(pins, &manifest, fetch, log)?;
    let manifest = read_json(&manifest)?;
    let client = &manifest["downloads"]["client"];
    // A version Horizon doesn't pin takes its client's sha1 from its version file, itself
    // checked against the sha1 Mojang's version list gives (`Pins::for_version`).
    let client_sha1 = match pins.client_sha1.as_str() {
        "" => text(client, "sha1")?,
        pinned => pinned.to_string(),
    };
    let mut game = vec![File {
        url: text(client, "url")?,
        sha1: client_sha1,
        size: client["size"].as_u64(),
        path: versions.join(format!("{v}.jar")),
    }];
    let libraries = manifest["libraries"]
        .as_array()
        .ok_or("The version file has no libraries")?;
    for library in libraries.iter().filter(|l| allowed(l, os)) {
        // Before 1.19 a library's natives for this system are a classifier of its own.
        let native = native_classifier(library, os)
            .map(|key| &library["downloads"]["classifiers"][key.as_str()]);
        for artifact in [Some(&library["downloads"]["artifact"]), native]
            .into_iter()
            .flatten()
            .filter(|a| !a.is_null())
        {
            game.push(File {
                url: text(artifact, "url")?,
                sha1: text(artifact, "sha1")?,
                size: artifact["size"].as_u64(),
                path: dir
                    .join("libraries")
                    .join(relative(&text(artifact, "path")?)?),
            });
        }
    }
    let libraries = game.len() - 1;
    // The logging setup the game is started with (`logging.client.argument`): for 1.7 to 1.18,
    // Mojang's fixed one is what keeps Log4Shell out.
    let logging = &manifest["logging"]["client"]["file"];
    if !logging.is_null() {
        game.push(File {
            url: text(logging, "url")?,
            sha1: text(logging, "sha1")?,
            size: logging["size"].as_u64(),
            path: dir
                .join("assets/log_configs")
                .join(relative(&text(logging, "id")?)?),
        });
    }
    log(format!(
        "Downloading Minecraft {v} from Mojang: the game and {libraries} libraries"
    ));
    fetch_all(&game, jobs, fetch, cancel, log)?;
    let index = &manifest["assetIndex"];
    let index = File {
        url: text(index, "url")?,
        sha1: text(index, "sha1")?,
        size: index["size"].as_u64(),
        path: dir
            .join("assets/indexes")
            .join(relative(&format!("{}.json", text(index, "id")?))?),
    };
    let assets = fetch_all(std::slice::from_ref(&index), 1, fetch, cancel, log)
        .and_then(|_| objects(&read_json(&index.path)?, dir))
        .and_then(|objects| {
            log(format!("Downloading the sounds ({} files)", objects.len()));
            // A connection that fails now and then is tried again, and a file that still fails
            // does not stop the others: the next download fetches what is left.
            let retrying = |url: &str| {
                let mut got = fetch(url);
                for _ in 0..2 {
                    if got.is_ok() {
                        break;
                    }
                    got = fetch(url);
                }
                got
            };
            fetch_each(&objects, jobs, &retrying, cancel, log, false)
        });
    match assets {
        Err(e) if e == CANCELLED => return Err(e),
        Err(e) => log(format!("The sounds could not all be downloaded: {e}")),
        Ok(()) => {}
    }
    Ok(())
}

/// Mojang's name for this system in its Java runtime index.
pub(crate) fn java_platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "mac-os-arm64",
        ("macos", "x86_64") => "mac-os",
        ("linux", "x86_64") => "linux",
        ("linux", "x86") => "linux-i386",
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("windows", "x86") => "windows-x86",
        _ => return None,
    })
}

/// Downloads Mojang's Java runtime `component` (the version JSON's `javaVersion.component`, e.g.
/// `java-runtime-epsilon`) into the game directory's `runtime` folder, as the official launcher
/// lays it out: `<component>/<platform>/<component>/`. Each file is checked against its sha1;
/// files already there and whole are kept. These also include javac for older release preparers;
/// current releases can use an already installed runtime without it.
pub fn java_runtime(
    component: &str,
    runtime: &Path,
    jobs: usize,
    fetch: Fetch,
    cancel: &AtomicBool,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let platform = java_platform().ok_or("Mojang has no Java for this system")?;
    let index: Value = serde_json::from_slice(&fetch(JAVA_RUNTIMES)?)
        .map_err(|e| format!("Mojang's Java index: {e}"))?;
    let entry = &index[platform][component][0];
    if entry.is_null() {
        return Err(format!("Mojang has no {component} for {platform}"));
    }
    let manifest = &entry["manifest"];
    let manifest: Value = serde_json::from_slice(&fetch_checked(
        fetch,
        &text(manifest, "url")?,
        &text(manifest, "sha1")?,
    )?)
    .map_err(|e| format!("Mojang's {component} manifest: {e}"))?;
    let files = manifest["files"]
        .as_object()
        .ok_or_else(|| format!("Mojang's {component} manifest lists no files"))?;
    let home = runtime.join(component).join(platform).join(component);
    let mut downloads = Vec::new();
    let mut executables = Vec::new();
    let mut links = Vec::new();
    for (name, file) in files {
        let path = home.join(relative(name)?);
        match file["type"].as_str() {
            Some("file") => {
                let raw = &file["downloads"]["raw"];
                if file["executable"] == true {
                    executables.push(path.clone());
                }
                downloads.push(File {
                    url: text(raw, "url")?,
                    sha1: text(raw, "sha1")?,
                    size: raw["size"].as_u64(),
                    path,
                });
            }
            Some("link") => links.push((path, text(file, "target")?)),
            _ => {}
        }
    }
    let name = entry["version"]["name"].as_str().unwrap_or(component);
    log(format!(
        "Downloading Minecraft's Java {name} from Mojang ({} files)",
        downloads.len()
    ));
    fetch_all(&downloads, jobs, fetch, cancel, log)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in &executables {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        for (path, target) in &links {
            if !link_stays_inside(name_in(&home, path), target) {
                return Err(format!(
                    "Unsafe link {target:?} in Mojang's {component} manifest"
                ));
            }
            let _ = fs::remove_file(path);
            fs::create_dir_all(path.parent().unwrap())
                .and_then(|_| std::os::unix::fs::symlink(target, path))
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (executables, links);
    Ok(home)
}

// Used by the unix link step below; the path-traversal check itself is tested on every
// platform, so these stay compiled and are allowed to look unused elsewhere.
#[cfg_attr(not(unix), allow(dead_code))]
fn name_in<'a>(home: &Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(home).unwrap_or(path)
}

/// Whether a link at `link` (relative to the runtime's home) pointing at `target` stays in the
/// home.
#[cfg_attr(not(unix), allow(dead_code))]
fn link_stays_inside(link: &Path, target: &str) -> bool {
    let mut depth = link.components().count().saturating_sub(1);
    for c in Path::new(target).components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return false,
        }
    }
    true
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v[key]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("The version file has no {key} in {v}"))
}

/// A path from the version file, refused if it could leave the folder.
pub(crate) fn relative(path: &str) -> Result<PathBuf, String> {
    let safe = !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    safe.then(|| PathBuf::from(path))
        .ok_or_else(|| format!("Unsafe path {path:?} in the version file"))
}

/// Whether a library's rules let this system (`osx`, `windows`, `linux`) use it: the last
/// rule that matches decides.
fn allowed(library: &Value, os: &str) -> bool {
    let Some(rules) = library["rules"].as_array() else {
        return true;
    };
    rules
        .iter()
        .rfind(|rule| rule["os"]["name"].as_str().is_none_or(|name| name == os))
        .is_some_and(|rule| rule["action"] == "allow")
}

/// The classifier holding a library's natives for `os` (`natives-osx`), from its `natives` map;
/// `${arch}` is this system's word size, as old versions name Windows' natives.
pub(crate) fn native_classifier(library: &Value, os: &str) -> Option<String> {
    let arch = if cfg!(target_pointer_width = "64") {
        "64"
    } else {
        "32"
    };
    library["natives"][os]
        .as_str()
        .map(|key| key.replace("${arch}", arch))
}

/// The asset index's objects, once each.
fn objects(index: &Value, dir: &Path) -> Result<Vec<File>, String> {
    let objects = index["objects"]
        .as_object()
        .ok_or("The asset index has no objects")?;
    let sizes: std::collections::HashMap<&str, u64> = objects
        .values()
        .filter_map(|o| Some((o["hash"].as_str()?, o["size"].as_u64()?)))
        .collect();
    let mut hashes: Vec<&str> = objects
        .values()
        .filter_map(|o| o["hash"].as_str())
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
        .into_iter()
        .map(|hash| {
            if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("Bad asset hash {hash:?} in the asset index"));
            }
            let path = format!("{}/{hash}", &hash[..2]);
            Ok(File {
                url: format!("{OBJECTS}/{path}"),
                sha1: hash.to_string(),
                size: sizes.get(hash).copied(),
                path: dir.join("assets/objects").join(path),
            })
        })
        .collect()
}

/// One file from Mojang, refused unless it has the size and sha1 Mojang lists.
fn fetch_file(fetch: Fetch, file: &File) -> Result<Vec<u8>, String> {
    let body = fetch(&file.url)?;
    if let Some(size) = file.size.filter(|&size| size != body.len() as u64) {
        return Err(format!(
            "is {} bytes, not the {size} Mojang lists",
            body.len()
        ));
    }
    match sha1_hex(&body) {
        actual if actual == file.sha1 => Ok(body),
        actual => Err(format!("is not the one Horizon pins (sha1 {actual})")),
    }
}

/// Downloads the files not already there, `jobs` at a time, with a line every second; stops at
/// the first error, or between files once `cancel` is set.
fn fetch_all(
    files: &[File],
    jobs: usize,
    fetch: Fetch,
    cancel: &AtomicBool,
    log: &dyn Fn(String),
) -> Result<(), String> {
    fetch_each(files, jobs, fetch, cancel, log, true)
}

/// `fetch_all`, which with `stop_on_error` false goes on past a file that fails and reports how
/// many did, with the first one's error, once the rest are done.
fn fetch_each(
    files: &[File],
    jobs: usize,
    fetch: Fetch,
    cancel: &AtomicBool,
    log: &dyn Fn(String),
    stop_on_error: bool,
) -> Result<(), String> {
    let errors = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed = Mutex::new(None);
    DONE.store(0, Relaxed);
    TOTAL.store(files.len(), Relaxed);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..jobs.clamp(1, files.len().max(1)))
            .map(|_| {
                scope.spawn(|| {
                    while let Some(file) = files.get(next.fetch_add(1, Relaxed)) {
                        if stop_on_error && failed.lock().unwrap().is_some() {
                            break;
                        }
                        if cancel.load(Relaxed) {
                            failed.lock().unwrap().get_or_insert(CANCELLED.to_string());
                            break;
                        }
                        if sha1_file(&file.path).as_ref() != Some(&file.sha1) {
                            let got = fetch_file(fetch, file)
                                .map_err(|e| format!("{} {e}", file.url))
                                .and_then(|body| write_file(&file.path, &body));
                            if let Err(e) = got {
                                errors.fetch_add(1, Relaxed);
                                failed.lock().unwrap().get_or_insert(e);
                            }
                        }
                        DONE.store(done.fetch_add(1, Relaxed) + 1, Relaxed);
                    }
                })
            })
            .collect();
        let mut waited = 0;
        while !workers.iter().all(|w| w.is_finished()) {
            std::thread::sleep(Duration::from_millis(100));
            waited += 1;
            if waited % 10 == 0 {
                log(format!("{} of {} files", done.load(Relaxed), files.len()));
            }
        }
    });
    TOTAL.store(0, Relaxed);
    match failed.into_inner().unwrap() {
        Some(e) if !stop_on_error && e != CANCELLED => Err(format!(
            "{} of {} files failed, the first: {e}",
            errors.load(Relaxed),
            files.len()
        )),
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "mojang_tests.rs"]
mod tests;
