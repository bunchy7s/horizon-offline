//! Servers hosted from the launcher. Every server is a `horizon-server` folder and the launcher drives
//! `horizon-server` (docs/server.md in the repository) rather than its own copy of the logic: `init`,
//! `start`, `stop`, `config set`, `backup`, `restore`, `update`, `service`, and the control API the running
//! supervisor serves on 127.0.0.1 for the live status, the log and console commands.
//!
//! The programs come from an installed Horizon version, laid out as `install.sh` lays them out
//! (`server-runtime/releases/<tag>`, `current`, `game-files/`), so `horizon-server update` and its
//! rollback work on them unchanged. The game files are the player's own, copied from the version the
//! launcher prepared: nothing of Mojang's is downloaded or shipped.

use crate::backend;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const PROGRAM: &str = if cfg!(windows) {
    "horizon-server.exe"
} else {
    "horizon-server"
};

/// The Minecraft EULA a server owner agrees to.
pub const EULA_URL: &str = "https://aka.ms/MinecraftEULA";

// ----- The list of servers -----------------------------------------------------------------------

/// One server the launcher knows: its name and its folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub dir: PathBuf,
}

fn list_file() -> PathBuf {
    backend::data_dir().join("servers.json")
}

/// The servers, in the order they were added; folders that are gone are left out.
pub fn list() -> Vec<Entry> {
    let entries: Vec<Entry> = fs::read_to_string(list_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    entries.into_iter().filter(|e| e.dir.is_dir()).collect()
}

fn save_list(entries: &[Entry]) -> Result<(), String> {
    let path = list_file();
    fs::create_dir_all(backend::data_dir()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    // A torn write would drop every server from the sidebar: write beside
    // the list, flush it, and rename it into place. Without the flush a
    // power cut can leave a file of the right length full of zeros.
    let partial = path.with_extension("json.tmp");
    let mut file = fs::File::create(&partial).map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&partial, &path).map_err(|e| e.to_string())
}

/// Adds a server folder to the list (once).
pub fn add(entry: Entry) -> Result<(), String> {
    let mut all = list();
    all.retain(|e| e.dir != entry.dir);
    all.push(entry);
    save_list(&all)
}

/// Takes a server off the list; its folder stays.
pub fn forget(dir: &Path) -> Result<(), String> {
    let mut all = list();
    all.retain(|e| e.dir != dir);
    save_list(&all)
}

/// Removes a registered, stopped server, retaining its files in the system trash.
pub fn delete(dir: &Path) -> Result<(), String> {
    if !list().iter().any(|entry| entry.dir == dir) {
        return Err("This server is no longer in the launcher’s list.".into());
    }
    if service_installed(dir) {
        return Err("Turn off running in the background before deleting this server.".into());
    }
    trash_stopped_server(dir, crate::desktop_seams::to_trash)?;
    forget(dir).map_err(|e| {
        format!("The server was moved to the trash, but its list could not be saved: {e}")
    })
}

fn trash_stopped_server(
    dir: &Path,
    trash: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if dir
        .symlink_metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
        || !dir.join("server.toml").is_file()
    {
        return Err("This is not a server folder.".into());
    }
    let state = dir.join(".horizon");
    fs::create_dir_all(&state).map_err(|e| e.to_string())?;
    // The supervisor holds this same lock throughout starting, running and stopping. Taking it
    // proves no supervisor is running, so a stale UI/API status cannot delete a live server.
    let lock = fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "Stop the server before deleting it.".to_string())?;
    // Windows can't move a folder while a file in it is open, the lock included: let it go
    // first. Still safe: a supervisor starting in between opens the lock again, and then the
    // move fails instead of taking a live server.
    #[cfg(windows)]
    drop(lock);
    drop(lock);
    trash(dir).map_err(|e| format!("Could not move the server to the trash: {e}"))
}

/// A folder name for a server name: lower case letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "server".into()
    } else {
        out
    }
}

/// A new folder for a server called `name`, under `servers/` in the data folder.
pub fn new_folder(name: &str) -> PathBuf {
    let root = backend::data_dir().join("servers");
    let base = slug(name);
    let mut dir = root.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{base}-{n}"));
        n += 1;
    }
    dir
}

// ----- The programs: an installed Horizon version, laid out as install.sh does ---------------------

pub fn runtime_root() -> PathBuf {
    backend::data_dir().join("server-runtime")
}

/// `horizon-server` of the release in use, once one is set up.
pub fn program() -> Option<PathBuf> {
    Some(runtime_root().join("current").join(PROGRAM)).filter(|p| p.is_file())
}

/// The release the servers run.
pub fn runtime_release() -> Option<String> {
    let target = fs::canonicalize(runtime_root().join("current")).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

/// Where `horizon-server` finds the game files: the folder install.sh keeps across updates.
pub fn game_files(root: &Path, game_version: &str) -> PathBuf {
    root.join("game-files/horizon-client").join(game_version)
}

/// Whether the servers' game files are there.
pub fn game_files_ready() -> bool {
    game_files(&runtime_root(), "26.3")
        .join("horizon-resources.json")
        .is_file()
}

/// Whether an installed version can host: it ships `horizon-server` and is prepared.
pub fn can_host(tag: &str) -> Result<(), String> {
    if !backend::version_dir(tag).join(PROGRAM).is_file() {
        return Err(format!(
            "Horizon {tag} has no server program. Install a newer version on the Versions page."
        ));
    }
    if !backend::is_prepared(tag) {
        return Err(format!(
            "Horizon {tag} isn't prepared yet: press Prepare on the Play page (it uses your own Minecraft {})",
            backend::game_version(tag)
        ));
    }
    Ok(())
}

/// Sets up the servers' programs from the installed version `tag` (if none are yet) and copies its
/// prepared game files for them. Safe to run again: it only refreshes the game files.
pub fn set_up(tag: &str) -> Result<(), String> {
    can_host(tag)?;
    // The data generator's reports the game reads at run time, as Play checks them.
    backend::need_prepared(tag)?;
    set_up_in(
        &runtime_root(),
        &backend::version_dir(tag),
        tag,
        &backend::game_version(tag),
    )
}

/// [`set_up`] with its folders given (for tests).
pub fn set_up_in(root: &Path, version: &Path, tag: &str, game_version: &str) -> Result<(), String> {
    let releases = root.join("releases");
    fs::create_dir_all(&releases).map_err(|e| format!("{}: {e}", releases.display()))?;
    let release = releases.join(tag);
    if !root.join("current").exists() {
        if !release.join(PROGRAM).is_file() {
            let scratch = releases.join(format!(".{tag}.copying"));
            let _ = fs::remove_dir_all(&scratch);
            copy_release(version, &scratch).map_err(|e| format!("copying {tag}: {e}"))?;
            let _ = fs::remove_dir_all(&release);
            fs::rename(&scratch, &release).map_err(|e| e.to_string())?;
        }
        let local = release.join(".local");
        fs::create_dir_all(&local).map_err(|e| e.to_string())?;
        let link = local.join("horizon-client");
        if link.symlink_metadata().is_err() {
            fs::create_dir_all(root.join("game-files/horizon-client"))
                .map_err(|e| e.to_string())?;
            point(&link, &root.join("game-files/horizon-client")).map_err(|e| e.to_string())?;
        }
        point(&root.join("current"), &release).map_err(|e| e.to_string())?;
    }
    // The game files: a copy (preparing again rewrites files in place), swapped in whole.
    let from = version.join(".local/horizon-client").join(game_version);
    let to = game_files(root, game_version);
    let fresh = to.with_extension("copying");
    let _ = fs::remove_dir_all(&fresh);
    copy_dir(&from, &fresh).map_err(|e| format!("copying the game files: {e}"))?;
    let old = to.with_extension("old");
    let _ = fs::remove_dir_all(&old);
    if to.exists() {
        fs::rename(&to, &old).map_err(|e| e.to_string())?;
    }
    fs::rename(&fresh, &to).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

/// A release folder without the launcher's own files (its prepared copy, install marker and the
/// game's macOS app bundle).
fn copy_release(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".local" || name == ".installed" || name == "Horizon.app" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Makes `link` point at the folder `target`, as install.sh (a symbolic link) and install.ps1 (a
/// junction, which needs no admin rights) do.
fn point(link: &Path, target: &Path) -> std::io::Result<()> {
    let fresh = link.with_extension("switching");
    let _ = fs::remove_file(&fresh);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, &fresh)?;
        fs::rename(&fresh, link)
    }
    #[cfg(windows)]
    {
        let _ = fs::remove_dir(link);
        crate::sources::junction(target, link)
    }
}

// ----- Running horizon-server -------------------------------------------------------------------

/// `horizon-server --dir <dir> <args>`.
pub fn command(dir: &Path, args: &[&str]) -> Result<Command, String> {
    let program = program().ok_or(
        "The server programs aren't set up yet: create a server, or press Prepare on its page.",
    )?;
    let mut cmd = crate::process::command(program);
    cmd.arg("--dir").arg(dir).args(args);
    Ok(cmd)
}

fn supervisor_log(dir: &Path) -> PathBuf {
    dir.join("logs/supervisor.log")
}

fn log_length(dir: &Path) -> u64 {
    fs::metadata(supervisor_log(dir)).map_or(0, |m| m.len())
}

/// Runs one `horizon-server` command to its end: what it printed, or the problem in plain words.
pub fn run(dir: &Path, args: &[&str]) -> Result<String, Problem> {
    let since = log_length(dir);
    let output = command(dir, args)
        .map_err(Problem::plain)?
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Problem::plain(format!("Couldn't run horizon-server: {e}")))?;
    let out = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        Ok(out)
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(Problem::explain(&format!("{out}\n{err}"), dir, since))
    }
}

/// Runs a long `horizon-server` command, passing each line it prints to `line`.
pub fn run_streamed(dir: &Path, args: &[&str], line: &dyn Fn(String)) -> Result<(), Problem> {
    use std::io::{BufRead, BufReader, Read};
    let since = log_length(dir);
    let mut child = command(dir, args)
        .map_err(Problem::plain)?
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Problem::plain(format!("Couldn't run horizon-server: {e}")))?;
    let mut err = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(e) = &mut err {
            let _ = e.read_to_string(&mut text);
        }
        text
    });
    let mut out = String::new();
    if let Some(stdout) = child.stdout.take() {
        // `start` prints a dot every half second on one line: those aren't lines worth showing.
        for l in BufReader::new(stdout).lines().map_while(Result::ok) {
            let shown = l
                .trim_start_matches(['.', ' '])
                .trim_start_matches("Starting");
            let shown = shown.trim_start_matches('.').trim();
            if !shown.is_empty() {
                line(shown.to_string());
            }
            out.push_str(&l);
            out.push('\n');
        }
    }
    let status = child.wait().map_err(|e| Problem::plain(e.to_string()))?;
    let err = errors.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(Problem::explain(&format!("{out}\n{err}"), dir, since))
    }
}

/// What can be done about a problem, with one button.
#[derive(Clone, Debug, PartialEq)]
pub enum Fix {
    /// The port is taken: use this free one.
    UsePort(u16),
    /// The game files are missing: prepare them (Play page) and copy them for the servers.
    Prepare,
    /// The EULA isn't agreed to.
    AgreeEula,
    /// The game's own log says more.
    ShowLog,
}

/// A problem, in plain words.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub message: String,
    pub fix: Option<Fix>,
}

impl Problem {
    pub fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fix: None,
        }
    }

    /// Reads what `horizon-server` (and the game log it points at) said.
    /// `since` is the background server's log length before the command: only what it logged after counts.
    pub fn explain(output: &str, dir: &Path, since: u64) -> Self {
        let log = fs::read(supervisor_log(dir)).unwrap_or_default();
        let new = log.get(since as usize..).unwrap_or_default();
        Self::explain_with(output, &String::from_utf8_lossy(new), dir)
    }

    fn explain_with(output: &str, log: &str, dir: &Path) -> Self {
        let both = format!("{output}\n{log}");
        let has = |s: &str| both.contains(s);
        // Vanilla says FAILED TO BIND TO PORT for any failure while starting; only these are a busy port.
        let busy = [
            "already in use",
            "Address in use",
            "Only one usage of each socket address",
        ];
        if busy.iter().any(|b| both.contains(b)) {
            let port = config(dir)
                .and_then(|c| c["server"]["port"].as_u64())
                .unwrap_or(25565) as u16;
            return Self {
                message: format!(
                    "Port {port} is already used by another program (maybe another server). Use a free \
                     port instead: friends then add it to the address."
                ),
                fix: Some(Fix::UsePort(free_port(port + 1, &[]))),
            };
        }
        if has("no prepared game files") || has("not found next to the resources") {
            return Self {
                message:
                    "The server needs the game files from your own Minecraft. Prepare them: it \
                          takes a few minutes, once."
                        .into(),
                fix: Some(Fix::Prepare),
            };
        }
        if has("EULA") && !has("eula=true") {
            return Self {
                message: "Running a server needs your agreement to the Minecraft EULA.".into(),
                fix: Some(Fix::AgreeEula),
            };
        }
        if has("isn't up after") || has("the server stopped") {
            return Self {
                message: "The game didn't start. Its log below says why.".into(),
                fix: Some(Fix::ShowLog),
            };
        }
        let said = both
            .lines()
            .rev()
            .find_map(|l| l.strip_prefix("horizon-server: "))
            .map(str::to_string)
            .or_else(|| {
                output
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "horizon-server failed without saying why".into());
        Self::plain(capitalised(&said))
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

// ----- Creating a server ------------------------------------------------------------------------

/// The world a new server starts with.
#[derive(Clone, Debug, PartialEq)]
pub enum World {
    /// A new world; an empty seed is a random one.
    New { seed: String },
    /// A copy of one of the player's worlds (its folder).
    Copy(PathBuf),
}

/// What the Create dialog asks; [`NewServer::default`] alone makes a working server.
#[derive(Clone, Debug, PartialEq)]
pub struct NewServer {
    pub name: String,
    pub world: World,
    pub game_mode: String,
    pub difficulty: String,
    pub max_players: u32,
    /// Only listed players (and operators) may join.
    pub friends_only: bool,
    /// Check players' Minecraft accounts: they must sign in with Microsoft.
    pub check_accounts: bool,
    /// Who may join first (whitelisted), and the first of them becomes an operator.
    pub players: Vec<String>,
    pub eula: bool,
}

impl Default for NewServer {
    fn default() -> Self {
        Self {
            name: "My server".into(),
            world: World::New {
                seed: String::new(),
            },
            game_mode: "survival".into(),
            difficulty: "normal".into(),
            max_players: 10,
            friends_only: true,
            check_accounts: false,
            players: Vec::new(),
            eula: false,
        }
    }
}

/// A port nobody listens on, from `from` up, and not one of `taken`.
pub fn free_port(from: u16, taken: &[u16]) -> u16 {
    (from..u16::MAX)
        .find(|p| !taken.contains(p) && std::net::TcpListener::bind(("0.0.0.0", *p)).is_ok())
        .unwrap_or(from)
}

/// The ports the launcher's other servers use: their game port and their web panel.
fn ports_taken() -> Vec<u16> {
    let mut out = Vec::new();
    for e in list() {
        if let Some(c) = config(&e.dir) {
            out.extend(c["server"]["port"].as_u64().map(|p| p as u16));
            out.extend(
                c["panel"]["bind"]
                    .as_str()
                    .and_then(|b| b.rsplit_once(':'))
                    .and_then(|(_, p)| p.parse::<u16>().ok()),
            );
        }
    }
    out
}

/// The player's worlds (the game home's `saves/`), newest first: their name and folder.
pub fn player_worlds() -> Vec<(String, PathBuf)> {
    let saves = backend::game_home().join("saves");
    let mut worlds: Vec<(std::time::SystemTime, String, PathBuf)> = fs::read_dir(saves)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("level.dat").is_file())
        .map(|e| {
            let when = e
                .path()
                .join("level.dat")
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (when, e.file_name().to_string_lossy().into_owned(), e.path())
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.0));
    worlds.into_iter().map(|(_, n, p)| (n, p)).collect()
}

/// The `horizon-server init` arguments for a new server.
pub fn init_args(spec: &NewServer, world: &str, port: u16) -> Vec<String> {
    let mut args: Vec<String> = vec!["init".into(), "--yes".into()];
    if spec.eula {
        args.push("--eula".into());
    }
    args.extend(["--world".into(), world.to_string()]);
    if let World::New { seed } = &spec.world {
        if !seed.trim().is_empty() {
            args.extend(["--seed".into(), seed.trim().to_string()]);
        }
    }
    args.extend(["--port".into(), port.to_string()]);
    args.push(if spec.friends_only {
        "--whitelist".into()
    } else {
        "--no-whitelist".into()
    });
    if !spec.check_accounts {
        args.push("--offline".into());
    }
    for p in &spec.players {
        args.extend(["--player".into(), p.clone()]);
    }
    if let Some(first) = spec.players.first() {
        args.extend(["--op".into(), first.clone()]);
    }
    args
}

/// The `horizon-server config set` arguments that finish a new server.
pub fn settings_args(spec: &NewServer, panel_port: u16) -> Vec<String> {
    vec![
        "config".into(),
        "set".into(),
        format!("world.game_mode={}", spec.game_mode),
        format!("world.difficulty={}", spec.difficulty),
        format!("server.max_players={}", spec.max_players.clamp(1, 1000)),
        format!("server.motd={}", spec.name.trim()),
        format!("panel.bind=127.0.0.1:{panel_port}"),
    ]
}

/// Creates a server: its folder, the world (a copy when asked), `init` and the settings, then adds it to
/// the list. `tag` is the installed version whose programs and game files it uses when none are set up.
pub fn create(spec: &NewServer, tag: Option<&str>, log: &dyn Fn(String)) -> Result<Entry, Problem> {
    if !spec.eula {
        return Err(Problem {
            message: "Running a server needs your agreement to the Minecraft EULA.".into(),
            fix: Some(Fix::AgreeEula),
        });
    }
    if program().is_none() || !game_files_ready() {
        let tag = tag.ok_or_else(|| {
            Problem::plain("Install a Horizon version first (Versions page), then prepare it.")
        })?;
        log(format!("Setting up the server programs from Horizon {tag}"));
        set_up(tag).map_err(|e| {
            if e.contains("prepared") {
                Problem {
                    message: e,
                    fix: Some(Fix::Prepare),
                }
            } else {
                Problem::plain(e)
            }
        })?;
    }
    let name = if spec.name.trim().is_empty() {
        "My server".to_string()
    } else {
        spec.name.trim().to_string()
    };
    let dir = new_folder(&name);
    fs::create_dir_all(&dir).map_err(|e| Problem::plain(e.to_string()))?;
    let world = match &spec.world {
        World::New { .. } => "world".to_string(),
        World::Copy(from) => {
            let world = from
                .file_name()
                .map(|n| slug(&n.to_string_lossy()))
                .unwrap_or_else(|| "world".into());
            log(format!("Copying the world {}", from.display()));
            copy_world(from, &dir.join(&world)).map_err(|e| {
                let _ = fs::remove_dir_all(&dir);
                Problem::plain(format!("Couldn't copy the world: {e}"))
            })?;
            world
        }
    };
    let taken = ports_taken();
    let port = free_port(25565, &taken);
    let panel = free_port(8130, &taken);
    let undo = |p: Problem| {
        let _ = fs::remove_dir_all(&dir);
        p
    };
    let init = init_args(spec, &world, port);
    log("Writing the server's settings".into());
    run(&dir, &init.iter().map(String::as_str).collect::<Vec<_>>()).map_err(undo)?;
    let settings = settings_args(spec, panel);
    run(
        &dir,
        &settings.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .map_err(undo)?;
    let entry = Entry { name, dir };
    add(entry.clone()).map_err(Problem::plain)?;
    Ok(entry)
}

/// A copy of a world folder, without the lock a running game holds.
fn copy_world(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "session.lock" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_world(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

// ----- A server's files --------------------------------------------------------------------------

/// The settings, as `horizon-server config --json` prints them (`None` before `init`, or from a
/// `horizon-server` older than the command).
pub fn config(dir: &Path) -> Option<Value> {
    if !dir.join("server.toml").is_file() {
        return None;
    }
    let out = run(dir, &["config", "--json"]).ok()?;
    serde_json::from_str(out.trim()).ok()
}

/// What `config set` did: the settings applied now, and those waiting for a restart.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Applied {
    #[serde(default)]
    pub applied: Vec<String>,
    #[serde(default)]
    pub restart: Vec<String>,
    #[serde(default)]
    pub running: bool,
}

/// Changes settings through `horizon-server config set` (checked there; nothing is written if one is
/// wrong).
pub fn set_config(dir: &Path, changes: &[(&str, String)]) -> Result<Applied, Problem> {
    let pairs: Vec<String> = changes.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let mut args = vec!["config", "set"];
    args.extend(pairs.iter().map(String::as_str));
    args.push("--json");
    let out = run(dir, &args)?;
    serde_json::from_str(out.trim()).map_err(|e| Problem::plain(e.to_string()))
}

/// A list file's names (`whitelist.json`, `ops.json`, `banned-players.json`), as vanilla writes them.
pub fn names(dir: &Path, file: &str) -> Vec<String> {
    fs::read_to_string(dir.join(file))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e["name"].as_str().map(str::to_string))
        .collect()
}

/// A list file's names with their UUIDs (empty when the file has none).
pub fn entries(dir: &Path, file: &str) -> Vec<(String, String)> {
    fs::read_to_string(dir.join(file))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            let name = e["name"].as_str()?.to_string();
            Some((name, e["uuid"].as_str().unwrap_or_default().to_string()))
        })
        .collect()
}

/// A backup, as `horizon-server backups --json` lists it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Backup {
    pub name: String,
    pub bytes: u64,
    #[serde(default)]
    pub modified: Option<u64>,
}

pub fn backups(dir: &Path) -> Result<Vec<Backup>, Problem> {
    let out = run(dir, &["backups", "--json"])?;
    serde_json::from_str(out.trim()).map_err(|e| Problem::plain(e.to_string()))
}

/// The address friends on the same network type: this computer's LAN address. No packet is sent: the
/// system only picks the interface it would use.
pub fn lan_address() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// Whether the server starts at login: the service definition `horizon-server service print` names exists.
pub fn service_installed(dir: &Path) -> bool {
    if cfg!(windows) {
        let name = format!("horizon-server-{}", service_suffix(dir));
        return crate::process::command("schtasks")
            .args(["/Query", "/TN", &name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    }
    run(dir, &["service", "print"])
        .ok()
        .and_then(|out| {
            out.lines()
                .next()
                .and_then(|l| l.strip_prefix("# "))
                .map(PathBuf::from)
        })
        .is_some_and(|p| p.is_file())
}

/// horizon-server's service name for a folder, after `horizon-server-`.
fn service_suffix(dir: &Path) -> String {
    let folder = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let clean: String = folder
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches('-');
    if clean.is_empty() {
        "server".into()
    } else {
        clean.into()
    }
}

// ----- The running server: its control API -------------------------------------------------------

/// The running supervisor's control API (`.horizon/api.json`: its address and token).
#[derive(Clone, Debug)]
pub struct Api {
    address: String,
    token: String,
}

impl Api {
    /// The API of the server running in `dir`, if one runs.
    pub fn open(dir: &Path) -> Option<Self> {
        let text = fs::read_to_string(dir.join(".horizon/api.json")).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        Some(Self {
            address: v["address"].as_str()?.to_string(),
            token: v["token"].as_str()?.to_string(),
        })
    }

    pub fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        // No proxy here on purpose: the server is on this computer, and this is asked every
        // second (`proxy` never sends a loopback address through one either).
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build();
        let request = agent
            .request(method, &format!("http://{}{path}", self.address))
            .set("Authorization", &format!("Bearer {}", self.token));
        let response = match body {
            Some(b) => request.send_json(b),
            None => request.call(),
        };
        match response {
            Ok(r) => r.into_json().map_err(|e| e.to_string()),
            Err(ureq::Error::Status(_, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                Err(v["error"]
                    .as_str()
                    .unwrap_or("the server refused")
                    .to_string())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn status(&self) -> Option<Status> {
        self.call("GET", "/api/v1/status", None)
            .ok()
            .map(|v| Status::parse(&v))
    }

    /// The log lines after `since`, and the number to ask from next time.
    pub fn logs(&self, since: u64) -> Option<(u64, Vec<String>)> {
        let v = self
            .call("GET", &format!("/api/v1/logs?since={since}"), None)
            .ok()?;
        let lines = v["lines"]
            .as_array()?
            .iter()
            .filter_map(|l| l.as_str().map(str::to_string))
            .collect();
        Some((v["next"].as_u64().unwrap_or(since), lines))
    }

    /// One console command; what the game answered.
    pub fn command(&self, line: &str) -> Result<Vec<String>, String> {
        let v = self.call("POST", "/api/v1/command", Some(json!({ "line": line })))?;
        Ok(v["output"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str().map(str::to_string))
            .collect())
    }

    /// A one-time login link for the web panel.
    pub fn panel_link(&self) -> Result<String, String> {
        let v = self.call("POST", "/api/v1/panel/login", None)?;
        v["url"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "the panel gave no link".into())
    }
}

/// A player online.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    pub name: String,
    pub uuid: String,
    pub ping: Option<u64>,
}

/// What the supervisor reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// starting, running, stopping or restarting (stopped when nothing answers).
    pub phase: String,
    pub uptime: Option<u64>,
    pub crashes: u64,
    pub last_crash: Option<String>,
    pub tps: Option<f64>,
    pub mspt: Option<f64>,
    pub players: Vec<Player>,
    pub max_players: Option<u64>,
    pub ram_bytes: Option<u64>,
    pub cpu_percent: Option<f64>,
    /// Paused because nobody is online (vanilla's pause-when-empty).
    pub paused: bool,
    /// The release in use and a newer one, when the supervisor found one.
    pub release: Option<String>,
    pub update_available: Option<String>,
    /// An update under way: what it is doing now.
    pub update_busy: Option<String>,
    /// How the last update went (its message, and whether it worked).
    pub update_last: Option<(String, bool)>,
}

impl Status {
    pub fn stopped() -> Self {
        Self {
            phase: "stopped".into(),
            ..Self::default()
        }
    }

    pub fn parse(v: &Value) -> Self {
        let game = &v["sample"]["game"];
        let update = &v["update"];
        Self {
            phase: v["phase"].as_str().unwrap_or("stopped").to_string(),
            uptime: v["engine_uptime_seconds"].as_u64(),
            crashes: v["crashes"].as_u64().unwrap_or(0),
            last_crash: v["last_crash"].as_str().map(str::to_string),
            tps: game["tps"].as_f64(),
            mspt: game["mspt"]["average"].as_f64(),
            players: game["players"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| Player {
                    name: p["name"].as_str().unwrap_or_default().to_string(),
                    uuid: p["uuid"].as_str().unwrap_or_default().to_string(),
                    ping: p["ping"].as_u64(),
                })
                .collect(),
            max_players: game["max_players"].as_u64(),
            ram_bytes: v["sample"]["rss_bytes"].as_u64(),
            cpu_percent: v["sample"]["cpu_percent"].as_f64(),
            paused: game["paused"] == true,
            release: update["installed"].as_str().map(str::to_string),
            update_available: update["available"].as_str().map(str::to_string),
            update_busy: update["busy"].as_str().map(str::to_string),
            update_last: update["last"]
                .as_str()
                .map(|m| (m.to_string(), update["ok"] != false)),
        }
    }

    pub fn running(&self) -> bool {
        self.phase != "stopped"
    }
}

/// The status of the server in `dir`: what its API says, or stopped.
pub fn status(dir: &Path) -> Status {
    Api::open(dir)
        .and_then(|api| api.status())
        .unwrap_or_else(Status::stopped)
}

#[cfg(test)]
#[path = "servers/tests.rs"]
mod tests;
