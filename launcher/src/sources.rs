//! Where the player's own Minecraft files are: the folder they chose, the official launcher's,
//! or another launcher's (Prism Launcher, MultiMC, Modrinth App, ATLauncher, GDLauncher), #434.
//! horizon-prepare reads the official launcher's layout and pins the version JSON's own sha1, so
//! files found elsewhere are linked into that layout in the launcher's data folder, with the
//! pinned version JSON from Mojang when the other launcher keeps its own rewrite of it.

use serde_json::Value;
use sha1::{Digest, Sha1};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// What a version's research-lock.json pins about the game files.
pub struct Pins {
    pub version: String,
    pub client_sha1: String,
    /// The version JSON's sha1, then the earlier ones Mojang republished.
    pub manifests: Vec<String>,
}

impl Pins {
    pub fn read(lock: &Value) -> Option<Pins> {
        let game = &lock["minecraft"];
        let text = |key: &str| game[key].as_str().map(String::from);
        let mut manifests = vec![text("manifest_sha1")?];
        if let Some(list) = game["accepted_manifest_sha1"].as_array() {
            manifests.extend(list.iter().filter_map(Value::as_str).map(String::from));
        }
        Some(Pins {
            version: text("version")?,
            client_sha1: text("client_sha1")?,
            manifests,
        })
    }

    /// Any version from Mojang's version list, by its id and its version file's sha1 (the
    /// list's): the client's sha1 then comes from that checked version file.
    pub fn for_version(id: &str, manifest_sha1: &str) -> Pins {
        Pins {
            version: id.to_string(),
            client_sha1: String::new(),
            manifests: vec![manifest_sha1.to_string()],
        }
    }

    fn manifest_accepted(&self, path: &Path) -> bool {
        sha1_file(path).is_some_and(|sha1| self.manifests.contains(&sha1))
    }
}

/// A launcher's copy of the pinned version.
#[derive(Debug, PartialEq)]
pub struct Found {
    /// The folder holding the launcher's `libraries` and `assets`.
    pub base: PathBuf,
    /// The client jar, checked against the pin.
    pub client: PathBuf,
    /// The launcher's version JSON when it is one Horizon pins.
    pub manifest: Option<PathBuf>,
}

impl Found {
    /// Whether `base` is in the official launcher's layout, so horizon-prepare reads it as is.
    fn official_layout(&self, version: &str) -> bool {
        let dir = self.base.join("versions").join(version);
        self.client == dir.join(format!("{version}.jar"))
            && self.manifest == Some(dir.join(format!("{version}.json")))
    }
}

pub struct Folders {
    pub home: PathBuf,
    /// `dirs::data_dir()`: ~/.local/share (XDG_DATA_HOME), ~/Library/Application Support,
    /// %APPDATA%.
    pub data: PathBuf,
    /// `dirs::config_dir()`: ~/.config (XDG_CONFIG_HOME), the same as `data` elsewhere.
    pub config: PathBuf,
}

impl Folders {
    pub fn current() -> Option<Folders> {
        Some(Folders {
            home: dirs::home_dir()?,
            data: dirs::data_dir()?,
            config: dirs::config_dir()?,
        })
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

const OFFICIAL: &str = "the official launcher";

/// Every launcher's default game folder on `os`, the official launcher's first. MultiMC on
/// macOS and Windows has none (it lives where it was unzipped): it is chosen by hand.
pub fn known_folders(os: Os, f: &Folders) -> Vec<(&'static str, PathBuf)> {
    let flatpak = |app: &str, rest: &str| f.home.join(".var/app").join(app).join(rest);
    let mut list = match os {
        Os::Linux => vec![
            (OFFICIAL, f.home.join(".minecraft")),
            (OFFICIAL, flatpak("com.mojang.Minecraft", ".minecraft")),
            ("Prism Launcher", f.data.join("PrismLauncher")),
            (
                "Prism Launcher",
                flatpak("org.prismlauncher.PrismLauncher", "data/PrismLauncher"),
            ),
            ("MultiMC", f.data.join("multimc")),
            ("Modrinth App", f.data.join("ModrinthApp")),
            (
                "Modrinth App",
                flatpak("com.modrinth.ModrinthApp", "data/ModrinthApp"),
            ),
            ("ATLauncher", f.data.join("atlauncher")),
            (
                "ATLauncher",
                flatpak("com.atlauncher.ATLauncher", "data/atlauncher"),
            ),
        ],
        Os::MacOs => vec![
            (OFFICIAL, f.data.join("minecraft")),
            ("Prism Launcher", f.data.join("PrismLauncher")),
            ("MultiMC", f.data.join("MultiMC")),
            ("Modrinth App", f.data.join("ModrinthApp")),
            ("ATLauncher", f.data.join("ATLauncher")),
        ],
        Os::Windows => vec![
            (OFFICIAL, f.data.join(".minecraft")),
            ("Prism Launcher", f.data.join("PrismLauncher")),
            ("Modrinth App", f.data.join("ModrinthApp")),
            ("ATLauncher", f.data.join("ATLauncher")),
        ],
    };
    // Names every system shares: the Modrinth App before its rename, GDLauncher and its
    // previous version (GDLauncher Next).
    list.extend([
        ("Modrinth App", f.data.join("com.modrinth.theseus")),
        ("GDLauncher", f.data.join("gdlauncher_carbon")),
        ("GDLauncher", f.config.join("gdlauncher_next")),
    ]);
    list
}

/// The client jar's path in a Maven repository: `com.mojang:minecraft:26.3:client` is
/// `com/mojang/minecraft/26.3/minecraft-26.3-client.jar`.
pub fn maven_path(name: &str) -> Option<String> {
    let (name, extension) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<&str> = name.split(':').collect();
    let (group, artifact, version) = match parts[..] {
        [group, artifact, version] | [group, artifact, version, _] => (group, artifact, version),
        _ => return None,
    };
    let classifier = parts.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    let unsafe_part = |p: &str| p.is_empty() || p.contains(['/', '\\']) || p.contains("..");
    if parts.iter().any(|p| unsafe_part(p)) || unsafe_part(extension) {
        return None;
    }
    Some(format!(
        "{}/{artifact}/{version}/{artifact}-{version}{classifier}.{extension}",
        group.replace('.', "/")
    ))
}

/// The client jar Prism Launcher's and MultiMC's metadata name (`meta/net.minecraft/<v>.json`,
/// their own format, `mainJar` a Maven name in `libraries`).
fn meta_client_jar(base: &Path, version: &str) -> Option<String> {
    let meta = fs::read_to_string(base.join(format!("meta/net.minecraft/{version}.json"))).ok()?;
    let meta: Value = serde_json::from_str(&meta).ok()?;
    maven_path(meta["mainJar"]["name"].as_str()?).map(|path| format!("libraries/{path}"))
}

/// Where launchers keep the client jar, relative to the folder holding their `libraries`.
fn client_jars(base: &Path, version: &str) -> Vec<String> {
    let v = version;
    let mut jars: Vec<String> = meta_client_jar(base, v).into_iter().collect();
    for jar in [
        // The official launcher; the Modrinth App, in meta/.
        format!("versions/{v}/{v}.jar"),
        // Prism Launcher and MultiMC.
        format!("libraries/com/mojang/minecraft/{v}/minecraft-{v}-client.jar"),
        // ATLauncher and GDLauncher.
        format!("libraries/net/minecraft/client/{v}/client-{v}.jar"),
    ] {
        if !jars.contains(&jar) {
            jars.push(jar);
        }
    }
    jars.extend(loader_jars(base, version));
    jars
}

/// The client jars the Modrinth App keeps for a mod loader's profile: `versions/<v>-<loader>/`,
/// the vanilla jar under the loader's name, when only a Fabric or Quilt profile of the version
/// was ever played (#1783). The jar is checked against the pin like the others.
fn loader_jars(base: &Path, version: &str) -> Vec<String> {
    let prefix = format!("{version}-");
    let mut jars: Vec<String> = fs::read_dir(base.join("versions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(&prefix))
        .map(|name| format!("versions/{name}/{name}.jar"))
        .collect();
    jars.sort();
    jars
}

pub(crate) fn sha1_file(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hash = Sha1::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        match file.read(&mut buffer).ok()? {
            0 => break,
            n => hash.update(&buffer[..n]),
        }
    }
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The Prism or MultiMC launcher root above `dir` (#693). Their instances live in
/// `instances/<name>/` (Prism's game folder is `minecraft`, MultiMC's `.minecraft`) and the
/// shared `libraries` and assets sit in the launcher root above `instances`, two or three
/// folders above what players naturally pick.
fn instance_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|a| a.file_name() == Some(std::ffi::OsStr::new("instances")))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// The launcher whose tree `path` is inside, when its folder name is one Horizon knows, so the
/// not-found message can name it (#693).
fn launcher_of(path: &Path) -> Option<&'static str> {
    path.ancestors()
        .filter_map(Path::file_name)
        .filter_map(|name| name.to_str())
        .find_map(|name| match name {
            "PrismLauncher" => Some("Prism Launcher"),
            "multimc" | "MultiMC" => Some("MultiMC"),
            "ModrinthApp" | "com.modrinth.theseus" => Some("the Modrinth App"),
            "atlauncher" | "ATLauncher" => Some("ATLauncher"),
            "gdlauncher_carbon" | "gdlauncher_next" => Some("GDLauncher"),
            _ => None,
        })
}

/// The pinned version in one launcher folder, or what the folder lacks. The folder holding
/// `libraries` is the folder itself or its `meta` (Modrinth App), `data` (GDLauncher) or
/// `datastore` (GDLauncher Next).
pub fn find(dir: &Path, pins: &Pins) -> Result<Found, String> {
    if !dir.is_dir() {
        return Err("the folder doesn't exist".into());
    }
    let v = &pins.version;
    let mut other = None;
    let bases = [dir.to_path_buf()]
        .into_iter()
        .chain(["meta", "data", "datastore"].map(|sub| dir.join(sub)));
    for base in bases {
        for jar in client_jars(&base, v) {
            let client = base.join(&jar);
            if !client.is_file() {
                continue;
            }
            if sha1_file(&client).as_ref() != Some(&pins.client_sha1) {
                other.get_or_insert(client);
                continue;
            }
            let manifest = base.join(format!("versions/{v}/{v}.json"));
            let manifest = pins.manifest_accepted(&manifest).then_some(manifest);
            return Ok(Found {
                base,
                client,
                manifest,
            });
        }
    }
    Err(match other {
        Some(jar) => format!(
            "{} is not the {v} release Horizon was made for",
            jar.strip_prefix(dir).unwrap_or(&jar).display()
        ),
        None => format!("no Minecraft {v} client jar"),
    })
}

/// The error when no folder has the pinned version: every folder looked in with what it
/// lacks, and what the player can do about it (#419, #693). When the chosen folder is inside a
/// launcher Horizon knows, the advice names it: a Prism or MultiMC instance downloads the game
/// only when it is launched once.
pub fn not_found(version: &str, checked: &[(PathBuf, String)], chosen: &Path) -> String {
    let mut text = format!("Minecraft {version} was not found. Horizon looked in:");
    for (dir, missing) in checked {
        text += &format!("\n  {} ({missing})", dir.display());
    }
    text += "\nTo fix it:";
    text += &match launcher_of(chosen) {
        Some(launcher) => format!(
            "\n  • Launch the {version} instance once in {launcher} so it downloads the game, \
             then prepare again.\n  \
             • Or choose the folder {launcher} keeps Minecraft {version} in (Minecraft folder › \
             Choose… in the launcher, --minecraft on the command line)."
        ),
        None => format!(
            "\n  • Play {version} once in the official Minecraft launcher, or in Prism Launcher, \
             MultiMC, the Modrinth App, ATLauncher or GDLauncher, then prepare again.\n  \
             • Or choose the folder where your launcher keeps Minecraft {version} (Minecraft \
             folder › Choose… in the launcher, --minecraft on the command line)."
        ),
    };
    text
}

/// Fetches a URL's body (from several threads for a download).
pub type Fetch<'a> = &'a (dyn Fn(&str) -> Result<Vec<u8>, String> + Sync);

/// Where the pinned version is.
#[derive(Debug, PartialEq)]
pub enum Resolved {
    /// A folder in the official launcher's layout.
    Folder(PathBuf),
    /// In none of these folders; each with what it lacks.
    NotFound(Vec<(PathBuf, String)>),
}

/// A Minecraft folder in the official launcher's layout holding the pinned version: the
/// chosen folder when it is one, else the files found in the chosen folder, a folder above it
/// (a launcher's instance folder may be chosen, and the launcher root above a Prism or MultiMC
/// `instances` folder is looked in too) or any launcher's default folder, linked into `linked`.
pub fn resolve(
    chosen: &Path,
    pins: &Pins,
    known: &[(&'static str, PathBuf)],
    linked: &Path,
    fetch: Fetch,
    log: &dyn Fn(String),
) -> Result<Resolved, String> {
    let mut checked = Vec::new();
    let mut found = None;
    // The chosen folder, the folders above it (an instance's own folder may be chosen), and,
    // when it is inside a Prism or MultiMC instance tree, the launcher root above `instances`
    // that holds their shared `libraries` (#693).
    let mut dirs: Vec<PathBuf> = chosen.ancestors().take(4).map(Path::to_path_buf).collect();
    if let Some(root) = instance_root(chosen) {
        if !dirs.contains(&root) {
            dirs.push(root);
        }
    }
    for dir in dirs {
        match find(&dir, pins) {
            Ok(f) => {
                found = Some(("the chosen folder", f));
                break;
            }
            Err(missing) if dir.as_path() == chosen => checked.push((dir, missing)),
            Err(_) => {}
        }
    }
    let same = |a: &Path, b: &Path| a == b || fs::canonicalize(a).ok() == fs::canonicalize(b).ok();
    for (name, dir) in known.iter().filter(|(_, d)| d.is_dir() && !same(d, chosen)) {
        if found.is_some() {
            break;
        }
        match find(dir, pins) {
            Ok(f) => found = Some((name, f)),
            Err(missing) => checked.push((dir.clone(), format!("{name}: {missing}"))),
        }
    }
    let Some((name, found)) = found else {
        return Ok(Resolved::NotFound(checked));
    };
    log(format!(
        "Using Minecraft {} from {name} ({})",
        pins.version,
        found.base.display()
    ));
    if found.official_layout(&pins.version) {
        return Ok(Resolved::Folder(found.base));
    }
    link_official_layout(&found, pins, linked, fetch, log)?;
    Ok(Resolved::Folder(linked.to_path_buf()))
}

/// The pinned version JSON at `path`, from piston-meta (by its sha1) unless it is there.
pub(crate) fn fetch_manifest(
    pins: &Pins,
    path: &Path,
    fetch: Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    if pins.manifest_accepted(path) {
        return Ok(());
    }
    let (v, sha1) = (&pins.version, &pins.manifests[0]);
    log(format!("Downloading the {v} version file from Mojang"));
    let url = format!("https://piston-meta.mojang.com/v1/packages/{sha1}/{v}.json");
    let body =
        fetch_checked(fetch, &url, sha1).map_err(|e| format!("Mojang's {v} version file {e}"))?;
    write_file(path, &body)
}

pub(crate) fn sha1_hex(data: &[u8]) -> String {
    Sha1::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A download checked against its sha1; the error says how it differs.
pub(crate) fn fetch_checked(fetch: Fetch, url: &str, sha1: &str) -> Result<Vec<u8>, String> {
    let body = fetch(url)?;
    match sha1_hex(&body) {
        actual if actual == sha1 => Ok(body),
        actual => Err(format!("is not the one Horizon pins (sha1 {actual})")),
    }
}

/// Writes a file whole: a partial one never takes its place.
pub(crate) fn write_file(path: &Path, data: &[u8]) -> Result<(), String> {
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    fs::create_dir_all(path.parent().unwrap()).map_err(error)?;
    let partial = path.with_file_name(format!(
        "{}.partial",
        path.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&partial, data)
        .and_then(|_| fs::rename(&partial, path))
        .map_err(error)
}

/// Links a launcher's files into the official layout in `linked`: the client jar as
/// `versions/<v>/<v>.jar`, its `libraries` and `assets`, and the pinned version JSON, the
/// launcher's own when it keeps it, else Mojang's.
fn link_official_layout(
    found: &Found,
    pins: &Pins,
    linked: &Path,
    fetch: Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let v = &pins.version;
    let versions = linked.join("versions").join(v);
    fs::create_dir_all(&versions).map_err(|e| format!("{}: {e}", versions.display()))?;
    let manifest = versions.join(format!("{v}.json"));
    if let Some(source) = &found.manifest {
        fs::copy(source, &manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    } else {
        fetch_manifest(pins, &manifest, fetch, log)?;
    }
    link(&found.client, &versions.join(format!("{v}.jar")))?;
    for dir in ["libraries", "assets"] {
        let source = found.base.join(dir);
        let target = linked.join(dir);
        if source.is_dir() {
            link(&source, &target)?;
        } else {
            unlink(&target)?;
        }
    }
    Ok(())
}

/// Removes a link left by an earlier preparation; never a real folder's contents.
fn unlink(path: &Path) -> Result<(), String> {
    if fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    // A Windows junction, or a directory symlink, is removed as a directory.
    fs::remove_file(path)
        .or_else(|_| fs::remove_dir(path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// `link` pointing at `target`, replacing an earlier one.
fn link(target: &Path, link: &Path) -> Result<(), String> {
    unlink(link)?;
    make_link(target, link).map_err(|e| format!("{} → {}: {e}", link.display(), target.display()))
}

#[cfg(unix)]
fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Symlinks need developer mode on Windows: a junction for a folder, a hard link for a file (a
/// copy on another volume).
#[cfg(windows)]
fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    if !target.is_dir() {
        return fs::hard_link(target, link).or_else(|_| fs::copy(target, link).map(|_| ()));
    }
    junction(target, link)
}

/// Makes `link` a junction to the folder `target` (no admin rights needed, unlike a symlink). The
/// error carries cmd's own message ("Access is denied", "The system cannot find the path…").
#[cfg(windows)]
pub(crate) fn junction(target: &Path, link: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // Always quoted: cmd reads `&`, `^` or `(` in an unquoted path (a user folder may have
    // them) as its own syntax. A Windows path never holds a quote.
    let quoted = |path: &Path| {
        let mut arg = std::ffi::OsString::from("\"");
        arg.push(path);
        arg.push("\"");
        arg
    };
    let out = crate::process::command("cmd")
        .args(["/C", "mklink", "/J"])
        .raw_arg(quoted(link))
        .raw_arg(quoted(target))
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&out.stderr);
    Err(std::io::Error::other(match why.trim() {
        "" => "mklink /J failed".to_string(),
        why => format!("mklink /J failed: {why}"),
    }))
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
