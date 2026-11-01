//! The engine: the game itself, run headless (`horizon-client --dedicated`, crates/horizon-client
//! game/dedicated.rs). It reads the vanilla files the supervisor writes, takes console commands on stdin and
//! logs on stdout.

use crate::config::Config;
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

/// The game's program names, most specific first: the server build (the game without sound or
/// controllers, so it needs neither ALSA nor udev on Linux; `horizon-server-linux-x86_64.tar.gz` holds it),
/// else the full game.
const PROGRAMS: [&str; 2] = ["horizon-engine", "horizon-client"];

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// The game program: `engine.path`, else one installed beside `horizon-server` (in the release in use when
/// it was installed by the installers), else one on the PATH.
pub fn program(config: &Config, dir: &Path) -> Result<PathBuf, String> {
    program_in(config, dir, crate::update::Install::find().as_ref())
}

/// [`program`], for the install `install`.
pub fn program_in(
    config: &Config,
    dir: &Path,
    install: Option<&crate::update::Install>,
) -> Result<PathBuf, String> {
    if !config.engine.path.is_empty() {
        let path = dir.join(&config.engine.path);
        return path
            .is_file()
            .then_some(path)
            .ok_or_else(|| format!("engine.path {} is not a file", config.engine.path));
    }
    // Installed: the release `current` points at now, so a switch applies at the next start.
    let beside = install.map(crate::update::Install::programs).or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
    });
    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    PROGRAMS
        .iter()
        .flat_map(|name| {
            beside
                .iter()
                .chain(&path_dirs)
                .map(move |d| d.join(exe(name)))
        })
        .find(|p| p.is_file())
        .ok_or_else(|| {
            "the game program (horizon-client) is neither beside horizon-server nor on the PATH: \
             install it there, or set engine.path in server.toml"
                .into()
        })
}

/// Checks that the game program starts at all: `--help` loads no game files, so it fails only when the
/// system can't load the program, as on a Linux without the ALSA and udev libraries the full game links
/// (the server build doesn't). The reason, with what to install.
pub fn check_starts(program: &Path) -> Result<(), String> {
    let mut command = crate::process::command(program);
    let output = command
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("the game ({}) can't start: {e}", program.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(start_failure(program, output.status.code(), &stderr))
}

/// What to tell the owner when the game program exits at once.
fn start_failure(program: &Path, code: Option<i32>, stderr: &str) -> String {
    let reason = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut text = format!(
        "the game ({}) can't start{}{}",
        program.display(),
        code.map_or(String::new(), |c| format!(" (exit status {c})")),
        if reason.is_empty() {
            String::new()
        } else {
            format!(": {reason}")
        }
    );
    if reason.contains("error while loading shared libraries")
        && (reason.contains("libasound") || reason.contains("libudev"))
    {
        text.push_str(
            "\nThe full game needs ALSA's and udev's libraries (libasound.so.2, libudev.so.1). Install the \
             server build, which needs neither (packaging/server/install.sh picks it on Linux), or the \
             libraries:\n  Debian, Ubuntu: sudo apt install libasound2 libudev1   (Ubuntu 24.04 and later: \
             libasound2t64)\n  Fedora, RHEL: sudo dnf install alsa-lib systemd-libs\n  Arch: sudo pacman -S \
             alsa-lib systemd-libs\n  NixOS: enable programs.nix-ld with alsa-lib and udev in its libraries",
        );
    }
    text
}

/// Where the prepared game files may be, in the order they are tried.
pub fn resource_candidates(config: &Config, dir: &Path, program: Option<&Path>) -> Vec<PathBuf> {
    if !config.engine.resources.is_empty() {
        return vec![dir.join(&config.engine.resources)];
    }
    let version = "26.3";
    let mut out = vec![dir.join("resources")];
    // A release folder or a checkout: .local/horizon-client/<version> beside the program or above it.
    if let Some(program) = program {
        for ancestor in program.ancestors().skip(1).take(4) {
            out.push(ancestor.join(".local/horizon-client").join(version));
        }
    }
    out
}

/// The prepared game files: a folder holding the manifest every `horizon-prepare` run writes
/// (`asset-index.json` comes only with the optional sounds, which a server doesn't need).
pub fn resources(config: &Config, dir: &Path, program: Option<&Path>) -> Result<PathBuf, String> {
    let candidates = resource_candidates(config, dir, program);
    candidates
        .iter()
        .find(|d| d.join("horizon-resources.json").is_file())
        .cloned()
        .ok_or_else(|| {
            let tried: Vec<String> = candidates
                .iter()
                .map(|d| format!("  {}", d.display()))
                .collect();
            format!(
                "no prepared game files. Horizon reads them from your own copy of Minecraft 26.3:\n\
                 prepare them with `horizon-prepare --prepare --server-jar PATH` (or the launcher) and copy \
                 the folder to resources/ here, or set engine.resources in server.toml. Looked in:\n{}",
                tried.join("\n")
            )
        })
}

/// The files `horizon-prepare` derives from the game and the engine reads at start (docs/hosting.md: a folder
/// prepared before `registries.json` and `commands.json` were kept stops with "registries.json not found").
const PREPARED_FILES: [&str; 4] = [
    "facts.json",
    "block-tags.json",
    "registries.json",
    "commands.json",
];

/// Folders of the client's own assets the engine loads fonts and textures from at start.
const PREPARED_DIRS: [&str; 2] = ["assets/minecraft/font", "assets/minecraft/textures"];

/// Checks a prepared folder before the engine starts on it, so a half-copied or stale folder is named
/// instead of surfacing as the engine's `os error 2` at every restart. Checks what the manifest
/// (`horizon-resources.json`, written last by `horizon-prepare`) records: the derived files are there and
/// match the hashes it holds, and the asset folders the engine loads from aren't empty.
pub fn check_prepared(resources: &Path) -> Result<(), String> {
    let manifest = std::fs::read_to_string(resources.join("horizon-resources.json"))
        .map_err(|e| format!("horizon-resources.json: {e}"))
        .and_then(|text| {
            serde_json::from_str::<serde_json::Value>(&text)
                .map_err(|e| format!("horizon-resources.json is not valid JSON ({e})"))
        });
    let mut problems = Vec::new();
    for name in PREPARED_FILES {
        if !resources.join(name).is_file() {
            problems.push(format!("{name} is missing"));
        }
    }
    match &manifest {
        Ok(manifest) => {
            for (name, key) in [
                ("facts.json", "facts_sha256"),
                ("block-tags.json", "block_tags_sha256"),
            ] {
                let Some(expected) = manifest.get(key).and_then(|v| v.as_str()) else {
                    continue;
                };
                if let Ok(actual) = sha256_file(&resources.join(name))
                    && !actual.eq_ignore_ascii_case(expected)
                {
                    problems.push(format!(
                        "{name} is not the file the preparation wrote (checksum differs)"
                    ));
                }
            }
        }
        Err(e) => problems.push(e.clone()),
    }
    for dir in PREPARED_DIRS {
        let empty = std::fs::read_dir(resources.join(dir))
            .map_or(true, |mut entries| entries.next().is_none());
        if empty {
            problems.push(format!("{dir} is missing or empty"));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the prepared game files in {} are incomplete or damaged:\n  {}\nPrepare them again with \
         `horizon-prepare --prepare --server-jar PATH` (or the launcher), or copy the whole prepared folder to \
         the server again.",
        resources.display(),
        problems.join("\n  ")
    ))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Starts the engine in the server folder, with its console and log piped to the supervisor and its status
/// on the control endpoint.
pub fn spawn(
    program: &Path,
    resources: &Path,
    dir: &Path,
    low_memory: bool,
) -> std::io::Result<Child> {
    command(program, resources, dir, low_memory)?.spawn()
}

/// The command `spawn` runs.
fn command(
    program: &Path,
    resources: &Path,
    dir: &Path,
    low_memory: bool,
) -> std::io::Result<Command> {
    // The game runs in the server folder, so a folder given relative to ours (`--dir srv`) would put its
    // control endpoint in `srv/srv/.horizon`, which doesn't exist.
    let dir = std::path::absolute(dir)?;
    let resources = std::path::absolute(resources)?;
    let mut command = Command::new(program);
    command
        .arg("--resources")
        .arg(resources)
        .arg("--control-socket")
        .arg(control_path(&dir))
        .args(["--dedicated", "--nogui"])
        .current_dir(&dir)
        .env("HORIZON_NO_AUDIO", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The terminal's Ctrl-C reaches the supervisor alone, which then stops the game cleanly.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP
            | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
    );
    // `[engine] low_memory`: mimalloc then keeps transparent huge pages off. Mostly empty huge pages cost
    // 60 MB idle and 190 MB at 20 players, and save ~1 % of tick time (docs/hosting.md).
    if low_memory {
        command.env("MIMALLOC_ALLOW_THP", "0");
    }
    Ok(command)
}

/// The game's control endpoint in a server folder (`--control-socket`): TCP on 127.0.0.1 with a token, on every OS.
pub fn control_path(dir: &Path) -> PathBuf {
    dir.join(".horizon/engine.json")
}

/// A connection to the game's control endpoint: one JSON line asked, one answered.
pub struct Control {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Control {
    pub fn connect(dir: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(control_path(dir))?;
        let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not a control endpoint");
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| bad())?;
        let port = v["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(bad)?;
        let token = v["token"].as_str().ok_or_else(bad)?;
        let writer = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        writer.set_read_timeout(Some(Duration::from_secs(30)))?;
        let mut control = Self {
            reader: BufReader::new(writer.try_clone()?),
            writer,
        };
        let reply = control.ask(&format!("auth {token}"))?;
        if reply["ok"] != true {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the game refused the token",
            ));
        }
        Ok(control)
    }

    pub fn ask(&mut self, line: &str) -> std::io::Result<serde_json::Value> {
        writeln!(self.writer, "{line}")?;
        self.writer.flush()?;
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        serde_json::from_str(&reply)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_memory_turns_huge_pages_off_for_the_game() {
        let thp = |low_memory| {
            let dir = std::env::temp_dir();
            let command = command(Path::new("horizon-client"), &dir, &dir, low_memory).unwrap();
            command
                .get_envs()
                .find(|(k, _)| *k == "MIMALLOC_ALLOW_THP")
                .and_then(|(_, v)| v.map(|v| v.to_owned()))
        };
        assert_eq!(thp(true).as_deref(), Some(std::ffi::OsStr::new("0")));
        assert_eq!(thp(false), None);
    }

    #[test]
    fn resources_are_searched_in_order() {
        let mut config = Config::default();
        let dir = Path::new("/srv/mc");
        let program = Path::new("/opt/horizon/target/release/horizon-client");
        let found = resource_candidates(&config, dir, Some(program));
        assert_eq!(found[0], Path::new("/srv/mc/resources"));
        assert!(found.contains(&PathBuf::from("/opt/horizon/.local/horizon-client/26.3")));
        config.engine.resources = "/data/res".into();
        assert_eq!(
            resource_candidates(&config, dir, Some(program)),
            [PathBuf::from("/data/res")]
        );
    }

    #[test]
    fn resources_prepared_without_sounds_are_found() {
        let root = std::env::temp_dir().join(format!("horizon-server-res-{}", std::process::id()));
        let prepared = root.join(".local/horizon-client/26.3");
        std::fs::create_dir_all(&prepared).unwrap();
        let program = root.join("horizon-client");
        let config = Config::default();
        let server = root.join("server");
        assert!(resources(&config, &server, Some(&program)).is_err());
        // What `horizon-prepare` writes on every run; no asset index without `horizon-prepare sounds`.
        std::fs::write(prepared.join("horizon-resources.json"), "{}").unwrap();
        assert_eq!(resources(&config, &server, Some(&program)), Ok(prepared));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A folder as `horizon-prepare` leaves it, as far as `check_prepared` looks.
    fn prepared_folder(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("horizon-server-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in PREPARED_DIRS {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("file"), "x").unwrap();
        }
        for file in PREPARED_FILES {
            std::fs::write(root.join(file), file).unwrap();
        }
        let manifest = serde_json::json!({
            "facts_sha256": sha256_file(&root.join("facts.json")).unwrap(),
            "block_tags_sha256": sha256_file(&root.join("block-tags.json")).unwrap(),
        });
        std::fs::write(root.join("horizon-resources.json"), manifest.to_string()).unwrap();
        root
    }

    #[test]
    fn a_complete_prepared_folder_passes() {
        let root = prepared_folder("complete");
        assert_eq!(check_prepared(&root), Ok(()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_files_and_folders_are_named() {
        let root = prepared_folder("missing");
        std::fs::remove_file(root.join("registries.json")).unwrap();
        std::fs::remove_dir_all(root.join("assets/minecraft/font")).unwrap();
        let text = check_prepared(&root).unwrap_err();
        assert!(text.contains("registries.json is missing"), "{text}");
        assert!(
            text.contains("assets/minecraft/font is missing or empty"),
            "{text}"
        );
        assert!(!text.contains("commands.json"), "{text}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_empty_asset_folder_counts_as_missing() {
        let root = prepared_folder("empty");
        std::fs::remove_file(root.join("assets/minecraft/textures/file")).unwrap();
        let text = check_prepared(&root).unwrap_err();
        assert!(
            text.contains("assets/minecraft/textures is missing or empty"),
            "{text}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_changed_derived_file_is_reported() {
        let root = prepared_folder("changed");
        std::fs::write(root.join("facts.json"), "damaged").unwrap();
        let text = check_prepared(&root).unwrap_err();
        assert!(
            text.contains("facts.json is not the file the preparation wrote"),
            "{text}"
        );
        assert!(!text.contains("block-tags.json"), "{text}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_broken_manifest_is_reported() {
        let root = prepared_folder("manifest");
        std::fs::write(root.join("horizon-resources.json"), "{").unwrap();
        let text = check_prepared(&root).unwrap_err();
        assert!(
            text.contains("horizon-resources.json is not valid JSON"),
            "{text}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_library_says_what_to_install() {
        let program = Path::new("/opt/horizon/horizon-client");
        let text = start_failure(
            program,
            Some(127),
            "/opt/horizon/horizon-client: error while loading shared libraries: libasound.so.2: cannot open \
             shared object file: No such file or directory\n",
        );
        assert!(text.starts_with(
            "the game (/opt/horizon/horizon-client) can't start (exit status 127): /opt/horizon/horizon-client: \
             error while loading shared libraries: libasound.so.2"
        ));
        assert!(text.contains("sudo apt install libasound2 libudev1"));
        // Another library: no ALSA advice (the server build, horizon-engine, links neither).
        let engine = start_failure(
            Path::new("/opt/horizon/horizon-engine"),
            Some(127),
            "/opt/horizon/horizon-engine: error while loading shared libraries: libgcc_s.so.1: cannot open \
             shared object file: No such file or directory\n",
        );
        assert!(!engine.contains("libasound"), "{engine}");
        let other = start_failure(program, Some(1), "something else\n");
        assert_eq!(
            other,
            "the game (/opt/horizon/horizon-client) can't start (exit status 1): something else"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_relative_folder_reaches_the_game_whole() {
        use std::os::unix::fs::PermissionsExt;
        // Relative to this process's folder (the crate's, under cargo test), as `--dir srv` gives it.
        let root = PathBuf::from(format!("reldir-test-{}", std::process::id()));
        let server = root.join("srv");
        std::fs::create_dir_all(server.join(".horizon")).unwrap();
        std::fs::create_dir_all(root.join("res")).unwrap();
        // A game that checks, from its own folder, the folders it was given.
        let game = root.join("game");
        std::fs::write(
            &game,
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in --resources) r=$2; shift;; \
             --control-socket) c=$2; shift;; esac; shift; done\n\
             [ -d \"$r\" ] && [ -d \"$(dirname \"$c\")\" ] && [ -f server.toml ]\n",
        )
        .unwrap();
        std::fs::set_permissions(&game, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(server.join("server.toml"), "").unwrap();
        // ETXTBSY: see `check_written`.
        let mut tries = 0;
        let status = loop {
            match spawn(
                &std::path::absolute(&game).unwrap(),
                &root.join("res"),
                &server,
                false,
            ) {
                Err(e) if e.raw_os_error() == Some(26) && tries < 50 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                child => break child.unwrap().wait(),
            }
        };
        std::fs::remove_dir_all(&root).unwrap();
        let status = status.unwrap();
        assert!(status.success(), "{status}");
    }

    /// `check_starts` on a script this test process just wrote. Another test's spawn can fork while the
    /// script's write handle is still open, and its child keeps that handle until it execs: running the
    /// script meanwhile fails with ETXTBSY (Linux), so that one error is retried.
    #[cfg(unix)]
    fn check_written(program: &Path) -> Result<(), String> {
        for _ in 0..50 {
            match check_starts(program) {
                Err(e) if e.contains("Text file busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                other => return other,
            }
        }
        check_starts(program)
    }

    #[cfg(unix)]
    #[test]
    fn the_check_runs_the_program() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("horizon-server-check-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = |name: &str, body: &str| {
            let path = root.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        assert_eq!(check_written(&script("starts", "exit 0")), Ok(()));
        let fails = script("fails", "echo 'no luck' >&2; exit 3");
        assert_eq!(
            check_written(&fails),
            Err(format!(
                "the game ({}) can't start (exit status 3): no luck",
                fails.display()
            ))
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
