//! A portable Rust toolchain of the launcher's own, for native mods (docs/native-mods.md), which
//! compile the game. levzzz5154 (2026-10-27): players combine many native mods per build, so they
//! build on their own computer, and the launcher brings the compiler.
//!
//! It downloads only the official components a build needs (`rustc`, `cargo`, `rust-std`, and on
//! Windows `rust-mingw`, the linker of the `windows-gnu` host, which needs no Visual Studio) from
//! static.rust-lang.org, each pinned by its SHA-256 below, and unpacks them into the launcher's data
//! folder. No rustup, nothing installed on the system or added to the PATH: the launcher calls this
//! cargo with its own `CARGO_HOME`. The pins come from the channel manifest of [`VERSION`]
//! (`https://static.rust-lang.org/dist/channel-rust-<VERSION>.toml`, `[pkg.<name>.target.<host>]`).
//!
//! On Windows it also brings MinGW-w64 ([`MINGW`]): `rust-mingw` only links. The game needs a C
//! compiler (`ring`, `hidapi` and wasmtime build C with `cc`), and `windows-sys`' `raw-dylib`
//! imports need `dlltool`, which runs the GNU assembler; `rust-mingw` has neither.
use crate::backend;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The Rust release native mods build with: the stable release when it was pinned (CI builds the
/// game on stable; horizon-mixins needs 1.95 or newer).
pub const VERSION: &str = "1.99.1";
const DIST: &str = "https://static.rust-lang.org/dist/2026-10-15";

/// One platform's components: (name, SHA-256 of `<name>-<VERSION>-<target>.tar.gz`).
pub struct Host {
    pub target: &'static str,
    components: &'static [(&'static str, &'static str)],
    /// The C toolchain it brings besides Rust's components, unpacked into [`MINGW_DIR`].
    mingw: Option<&'static Mingw>,
}

/// A MinGW-w64 release: one zip, pinned by its SHA-256.
pub struct Mingw {
    pub version: &'static str,
    url: &'static str,
    sha256: &'static str,
}

/// WinLibs' build of GCC and binutils on MinGW-w64 (MSVCRT, the C runtime `rust-mingw` links
/// against; POSIX threads; SEH), the release's `mingw64/` folder without its documentation.
pub const MINGW: Mingw = Mingw {
    version: "GCC 16.2.0, MinGW-w64 14.0.0 (WinLibs r3)",
    url: "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-msvcrt-r3/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64msvcrt-14.0.0-r3.zip",
    sha256: "62af32ea58acdea1318ea5dd8eb7017eaee694dd40b269d52a59c769dcfefb82",
};
/// Where MinGW-w64 goes in the toolchain, as the zip names it.
const MINGW_DIR: &str = "mingw64";

const HOSTS: &[Host] = &[
    Host {
        target: "aarch64-apple-darwin",
        components: &[
            (
                "rustc",
                "cc998c1dc8df006106472357aa87838b2da3f7b25446b98a94cfd58796cb0610",
            ),
            (
                "cargo",
                "b5cc7634f919949c3e5e131123f813416dc47f6b809a5371d12f1f593595ae13",
            ),
            (
                "rust-std",
                "c124006d63885ba1029aca49efd59e93be6b2b171c379318ff94a00ef5eba664",
            ),
        ],
        mingw: None,
    },
    Host {
        target: "x86_64-unknown-linux-gnu",
        components: &[
            (
                "rustc",
                "85e7c0e6f21636f714e2a42966011364a6cc56b6548126d32144d6ba90904966",
            ),
            (
                "cargo",
                "47babc028247e26577f8a1bf390b11f447ad9b7ab93f5c9a0dda078935fc149b",
            ),
            (
                "rust-std",
                "d1290c05994a59c029e562cb75be1bddc6fb11a4c501e1b7b0f61d119c6cbb12",
            ),
        ],
        mingw: None,
    },
    Host {
        target: "x86_64-pc-windows-gnu",
        components: &[
            (
                "rustc",
                "43b17e9a9134ea72562bfbb23728a830f671ea9be1db6a72597a8d240e986b35",
            ),
            (
                "cargo",
                "57e54617be4420dbb967ca8293f5ace7168104ac12efa9dc7c5c3bda112f8b36",
            ),
            (
                "rust-std",
                "6546651238a3ac092b763511be524ca6eda6103e5d47342416923be1b3ce837c",
            ),
            (
                "rust-mingw",
                "30263d7bb4fd0bf63a153afe36d7fc102f929a7d42781651c6b3a03a414ba428",
            ),
        ],
        mingw: Some(&MINGW),
    },
];

/// This computer's host, if the launcher ships a toolchain for it (the platforms it is released
/// for: macOS on Apple silicon, Linux x86-64, Windows x64).
pub fn host() -> Option<&'static Host> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-gnu",
        _ => return None,
    };
    HOSTS.iter().find(|h| h.target == target)
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// An unpacked toolchain.
#[derive(Clone, Debug, PartialEq)]
pub struct Toolchain {
    pub dir: PathBuf,
}

impl Toolchain {
    pub fn cargo(&self) -> PathBuf {
        self.dir.join("bin").join(exe("cargo"))
    }

    /// MinGW-w64's programs (gcc, as, dlltool), when the toolchain brings it (Windows).
    pub fn mingw_bin(&self) -> PathBuf {
        self.dir.join(MINGW_DIR).join("bin")
    }

    /// `program` (this toolchain's cargo, or a tool built with it) set up to use only this
    /// toolchain: its rustc, its MinGW-w64 where it has one, the launcher's Cargo home, and none of
    /// the player's rustup, compiler-wrapper or compiler-flag settings ([`scrubbed`]). On Windows it
    /// opens no console window: cargo is a console program, and the launcher has none to share, so
    /// each build showed an empty one for as long as it ran (#4770).
    pub fn command(&self, program: &Path) -> Command {
        let mut cmd = crate::process::command(program);
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mingw = Some(self.mingw_bin()).filter(|bin| bin.is_dir());
        let paths = std::iter::once(self.dir.join("bin"))
            .chain(mingw)
            .chain(std::env::split_paths(&path));
        if let Some(root) = outside_configs() {
            cmd.current_dir(root);
        }
        cmd.env("PATH", std::env::join_paths(paths).unwrap_or(path))
            .env("RUSTC", self.dir.join("bin").join(exe("rustc")))
            .env("CARGO_HOME", cargo_home());
        // gcc, as, ld — and `cargo install`'s build — put scratch files in the temp dir,
        // which has to be a path they can open too: the build's own, never the player's
        // (#5189: a Cyrillic user name makes the player's unreadable by MinGW-w64).
        let tmp = backend::build_dir().join("tmp");
        if fs::create_dir_all(&tmp).is_ok() {
            cmd.env("TMPDIR", &tmp).env("TMP", &tmp).env("TEMP", &tmp);
        }
        for name in SCRUBBED {
            cmd.env_remove(name);
        }
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(scrubbed) {
                cmd.env_remove(name);
            }
        }
        cmd
    }

    /// `rustc --version`, once the toolchain answers.
    pub fn version(&self) -> Result<String, String> {
        let out = self
            .command(&self.dir.join("bin").join(exe("rustc")))
            .arg("--version")
            .output()
            .map_err(|e| format!("Rust {VERSION} doesn't start: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "Rust {VERSION} doesn't start: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
}

/// Whether the player's environment variable `name` stays out of a build: it would pick another
/// Rust or wrap it (rustup, `RUSTC_WRAPPER`), or change the compiler's flags (`RUSTFLAGS`, a
/// profile, incremental, the build's target or linker: `CARGO_BUILD_*` but the jobs,
/// `CARGO_TARGET_*`),
/// so the build would differ from the release's and recompile everything after any change.
pub fn scrubbed(name: &str) -> bool {
    const PREFIXES: [&str; 3] = ["CARGO_PROFILE_", "CARGO_BUILD_", "CARGO_TARGET_"];
    // Windows' variable names ignore case.
    let name = name.to_ascii_uppercase();
    // The number of jobs changes no flag: a player may still limit it.
    if name == "CARGO_BUILD_JOBS" {
        return false;
    }
    SCRUBBED.contains(&name.as_str()) || PREFIXES.iter().any(|p| name.starts_with(p))
}

/// `ioprio_set`'s "which" for one process.
#[cfg(target_os = "linux")]
const IOPRIO_WHO_PROCESS: libc::c_long = 1;
/// The best-effort I/O class (2) at its lowest level (7), as `ioprio_set` takes it.
#[cfg(target_os = "linux")]
const LOW_IO: libc::c_long = (2 << 13) | 7;

/// Runs `cmd` (a build) below the player's own programs, so the desktop and a game in play stay
/// smooth while it compiles; with the computer otherwise idle it still gets every core. Unix: nice
/// 19, and on Linux the lowest best-effort I/O level (not the idle class, which gets the disk only
/// when nothing else uses it, so a game streaming chunks could stall a build for minutes);
/// Windows: below-normal priority, keeping [`crate::process::hide_console`]'s no-console flag
/// (`creation_flags` sets the whole value, so this sets both).
pub fn low_priority(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: between fork and exec the closure only makes two system calls, which are
        // async-signal-safe, on the child itself; a failure (not allowed) leaves the priority.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, 19);
                #[cfg(target_os = "linux")]
                {
                    libc::syscall(
                        libc::SYS_ioprio_set,
                        IOPRIO_WHO_PROCESS,
                        0 as libc::c_long,
                        LOW_IO,
                    );
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        cmd.creation_flags(crate::process::CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    cmd
}

const SCRUBBED: [&str; 8] = [
    "RUSTUP_TOOLCHAIN",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "CARGO_INCREMENTAL",
];

/// Where commands run so that no player's Cargo configuration reaches them: the filesystem root
/// above the launcher's data (`/` on Unix, whatever mount the data is on; the drive's root on
/// Windows). Cargo reads `.cargo/config.toml` in the folder it starts in and in
/// every folder above it, so from a folder under the player's home it would read
/// `~/.cargo/config.toml` even with another `CARGO_HOME`: its `[profile]`, `[build]` and
/// `[target]` settings (rustflags, linker, incremental) beat both the workspace's profile and
/// the environment, and no command-line switch turns that search off. A build names its workspace
/// with `--manifest-path` instead.
pub fn outside_configs() -> Option<PathBuf> {
    let data = backend::build_dir();
    data.ancestors()
        .last()
        .filter(|root| root.has_root())
        .map(Path::to_path_buf)
}

/// Where toolchains live: `<build dir>/rust/` ([`backend::build_dir`] — a path the tools can
/// read, which the data folder's isn't on Windows with a non-ASCII user name, #5189).
pub fn root() -> PathBuf {
    backend::build_dir().join("rust")
}

/// The Cargo home of native builds (downloaded crates, horizon-mixins), never the player's own.
pub fn cargo_home() -> PathBuf {
    backend::build_dir().join("cargo")
}

fn dir_for(host: &Host) -> PathBuf {
    root().join(format!("{VERSION}-{}", host.target))
}

const COMPLETE: &str = ".installed";

/// The toolchain, when it is installed. A Windows one from before MinGW-w64 isn't: installing
/// again brings it.
pub fn installed() -> Option<Toolchain> {
    let host = host()?;
    let toolchain = Toolchain { dir: dir_for(host) };
    let mingw = host.mingw.is_none() || toolchain.mingw_bin().join(exe("gcc")).is_file();
    (toolchain.dir.join(COMPLETE).is_file() && mingw).then_some(toolchain)
}

/// Downloads, checks and unpacks the pinned toolchain (once); `progress` gets the fraction done.
pub fn install(progress: impl Fn(f32)) -> Result<Toolchain, String> {
    if let Some(done) = installed() {
        return Ok(done);
    }
    let host = host().ok_or_else(|| {
        format!(
            "Native mods need Rust, and the launcher has none for {} {}.",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let dir = dir_for(host);
    let partial = root().join(format!(".{VERSION}-{}.partial", host.target));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let count = (host.components.len() + usize::from(host.mingw.is_some())) as f32;
    for (i, (name, sha256)) in host.components.iter().enumerate() {
        let file = format!("{name}-{VERSION}-{}.tar.gz", host.target);
        let archive = root().join(format!(".{file}"));
        let step = |f: f32| progress((i as f32 + f) / count);
        backend::download(&format!("{DIST}/{file}"), 0, sha256, &archive, &step)
            .map_err(|e| format!("Rust {VERSION} ({name}): {e}"))?;
        let unpacked = unpack(&archive, &partial);
        let _ = fs::remove_file(&archive);
        unpacked.map_err(|e| format!("Rust {VERSION} ({name}): {e}"))?;
    }
    if let Some(mingw) = host.mingw {
        let archive = root().join(".mingw.zip");
        let done = host.components.len() as f32;
        let step = |f: f32| progress((done + f) / count);
        backend::download(mingw.url, 0, mingw.sha256, &archive, &step)
            .map_err(|e| format!("MinGW-w64 ({}): {e}", mingw.version))?;
        let unpacked = unpack_mingw(&archive, &partial);
        let _ = fs::remove_file(&archive);
        unpacked.map_err(|e| format!("MinGW-w64 ({}): {e}", mingw.version))?;
    }
    fs::write(partial.join(COMPLETE), VERSION).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&partial, &dir).map_err(|e| e.to_string())?;
    progress(1.0);
    Ok(Toolchain { dir })
}

/// Unpacks one component archive into `prefix`, as its installer would: an archive holds
/// `<top>/<component>/<files>` per component, and each component's files go under `prefix`. Its
/// installer script, lists and `manifest.in` stay out.
fn unpack(archive: &Path, prefix: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let Some(relative) = component_path(&path) else {
            continue;
        };
        let to = prefix.join(&relative);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry
            .unpack(&to)
            .map_err(|e| format!("{}: {e}", relative.display()))?;
    }
    Ok(())
}

/// Where a file of a component archive goes, relative to the prefix: `rustc-1.99.1-x/rustc/bin/rustc`
/// → `bin/rustc`. None for the archive's own files and anything that would leave the prefix.
fn component_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let parts: Vec<&std::ffi::OsStr> = path
        .components()
        .map(|c| match c {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [_, _, rest @ ..] if !rest.is_empty() && rest != ["manifest.in"] => {
            Some(rest.iter().collect())
        }
        _ => None,
    }
}

/// Unpacks MinGW-w64's zip into `prefix`: its `mingw64/` folder, documentation left out.
fn unpack_mingw(archive: &Path, prefix: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(relative) = entry.enclosed_name().and_then(|p| mingw_path(&p)) else {
            continue;
        };
        let to = prefix.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = fs::File::create(&to).map_err(|e| format!("{}: {e}", relative.display()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("{}: {e}", relative.display()))?;
    }
    Ok(())
}

/// Where a file of MinGW-w64's zip goes, relative to the prefix: `mingw64/bin/gcc.exe` as it is.
/// None for its documentation (`mingw64/share/`, 50 MB of the 925), the folder itself and anything
/// outside `mingw64/`.
fn mingw_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let parts: Vec<&std::ffi::OsStr> = path
        .components()
        .map(|c| match c {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [top, first, ..] if *top == MINGW_DIR && *first != "share" => Some(parts.iter().collect()),
        _ => None,
    }
}

/// Whether this computer has what a build needs besides the toolchain. Rust brings its compiler,
/// but uses the system's linker on macOS (the Command Line Tools) and Linux (`cc`). On Linux the
/// game also builds against the system's sound and controller libraries ([`linux_needs`]).
/// Windows' `windows-gnu` toolchain brings its own linker, and MinGW-w64 for C and import
/// libraries ([`MINGW`]).
pub fn linker() -> Result<(), String> {
    let ok = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    match std::env::consts::OS {
        // `xcode-select -p` only says where they are; running `cc` without them would open
        // Apple's install dialog by surprise.
        "macos" if !ok("xcode-select", &["-p"]) => Err(
            "Native mods need Apple's Command Line Tools to link the game. Run `xcode-select --install` in Terminal, then build again."
                .into(),
        ),
        "linux" => linux_needs(ok, &fs::read_to_string("/etc/os-release").unwrap_or_default()),
        _ => Ok(()),
    }
}

/// What a Linux build needs from the system, checked before anything is downloaded: a C compiler
/// (the linker; `ring` and `hidapi` compile C) with the C library's headers, `pkg-config`, and
/// the development files of ALSA (`alsa-sys`, sound) and libudev (`libudev-sys` and `hidapi`,
/// controllers), which those crates look up with it. Desktop distributions ship the libraries but
/// not these files, and SteamOS has GCC without the C library's headers (`stdint.h` not found in
/// `ring`, #4770), so a build without them failed minutes in. `ok` runs a check; `os` is
/// `/etc/os-release`, which picks the install command.
fn linux_needs(ok: impl Fn(&str, &[&str]) -> bool, os: &str) -> Result<(), String> {
    let cc = ok("cc", &["--version"]);
    // GCC's and Clang's own `stdint.h` include the C library's.
    let headers = [
        "-fsyntax-only",
        "-x",
        "c",
        "-include",
        "stdint.h",
        "/dev/null",
    ];
    let pkg_config = ok("pkg-config", &["--version"]);
    let has = |name: &str| pkg_config && ok("pkg-config", &["--exists", name]);
    let missing: Vec<&str> = [
        (cc, "a C compiler (cc)"),
        (!cc || ok("cc", &headers), "the C library's headers"),
        (pkg_config, "pkg-config"),
        (has("alsa"), "ALSA's development files"),
        (has("libudev"), "libudev's development files"),
    ]
    .into_iter()
    .filter(|(found, _)| !found)
    .map(|(_, what)| what)
    .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let field = |name: &str| {
        os.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
            .map(|v| v.trim_matches('"').to_owned())
            .unwrap_or_default()
    };
    let (id, like) = (field("ID"), field("ID_LIKE"));
    let family = |names: &[&str]| {
        std::iter::once(id.as_str())
            .chain(like.split_whitespace())
            .any(|n| names.contains(&n))
    };
    const APT: &str = "sudo apt install build-essential pkg-config libasound2-dev libudev-dev";
    let command = if family(&["debian", "ubuntu"]) {
        Some(APT)
    } else if family(&["fedora", "rhel"]) {
        Some("sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel systemd-devel")
    } else if family(&["arch"]) && id != "steamos" {
        Some("sudo pacman -S --needed base-devel alsa-lib systemd-libs")
    } else if family(&["suse", "opensuse"]) {
        Some("sudo zypper install gcc pkg-config alsa-devel libudev-devel")
    } else {
        None
    };
    let how = match command {
        _ if id == "steamos" => "SteamOS ships without them and its system is read-only, so native mods can't be built on it yet.".to_owned(),
        Some(command) => format!("Install them in a terminal, then build again:\n{command}"),
        None => format!("Install them with your distribution's package manager, then build again (on Debian, Ubuntu or Mint: {APT})."),
    };
    Err(format!(
        "Native mods need these to build the game on Linux, and this computer lacks them: {}. {how}",
        missing.join(", ")
    ))
}

/// Removes every toolchain, the native builds' Cargo home and their scratch folder.
pub fn remove() -> Result<(), String> {
    for dir in [root(), cargo_home(), backend::build_dir().join("tmp")] {
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn every_host_pins_each_component_once_with_a_sha256() {
        for host in HOSTS {
            let names: Vec<&str> = host.components.iter().map(|c| c.0).collect();
            assert_eq!(
                &names[..3],
                ["rustc", "cargo", "rust-std"],
                "{}",
                host.target
            );
            for (name, sha) in host.components {
                assert!(
                    sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
                    "{name} {}",
                    host.target
                );
            }
        }
        let windows = HOSTS.iter().find(|h| h.target.contains("windows")).unwrap();
        assert!(windows.target.ends_with("-gnu"), "no Visual Studio needed");
        assert!(windows.components.iter().any(|c| c.0 == "rust-mingw"));
        // Only Windows brings a C toolchain; macOS and Linux use the system's.
        for host in HOSTS {
            assert_eq!(host.mingw.is_some(), host.target.contains("windows"));
        }
        let mingw = windows.mingw.unwrap();
        assert!(mingw.sha256.len() == 64 && mingw.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(mingw.url.starts_with("https://") && mingw.url.ends_with(".zip"));
        assert!(
            mingw.url.contains("msvcrt"),
            "the C runtime rust-mingw links against"
        );
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(host().unwrap().target, "aarch64-apple-darwin");
        }
    }

    #[test]
    fn the_players_compiler_settings_stay_out_of_builds() {
        for name in [
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTDOCFLAGS",
            "CARGO_INCREMENTAL",
            "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_PROFILE_RELEASE_CODEGEN_UNITS",
            "CARGO_BUILD_TARGET",
            "CARGO_BUILD_RUSTFLAGS",
            "CARGO_BUILD_RUSTC_WRAPPER",
            "CARGO_TARGET_DIR",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
            "RUSTC_WRAPPER",
            "RUSTUP_TOOLCHAIN",
            "cargo_profile_release_debug",
        ] {
            assert!(scrubbed(name), "{name}");
        }
        for name in [
            "CARGO_BUILD_JOBS",
            "PATH",
            "HOME",
            "CARGO_HOME",
            "CARGO_NET_OFFLINE",
            "CARGO_HTTP_PROXY",
            "HTTPS_PROXY",
            "SSL_CERT_FILE",
        ] {
            assert!(!scrubbed(name), "{name}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn builds_run_at_the_lowest_priority() {
        // Read back from the running child itself: its niceness and its I/O priority.
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let mut child = low_priority(&mut cmd).spawn().unwrap();
        let pid = child.id();
        // The priority is set between fork and exec; wait for the exec.
        std::thread::sleep(std::time::Duration::from_millis(200));
        // SAFETY: plain system calls on a child process of this test.
        let (nice, io) = unsafe {
            let nice = libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t);
            let io = libc::syscall(
                libc::SYS_ioprio_get,
                IOPRIO_WHO_PROCESS,
                pid as libc::c_long,
            );
            (nice, io)
        };
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(nice, 19);
        assert_eq!(io, LOW_IO);
    }

    #[test]
    fn component_files_land_under_the_prefix_and_nothing_else_does() {
        let at = |p: &str| component_path(Path::new(p));
        assert_eq!(
            at("rustc-1.99.1-x/rustc/bin/rustc"),
            Some(PathBuf::from("bin/rustc"))
        );
        assert_eq!(
            at("rust-std-1.99.1-x/rust-std-x/lib/rustlib/x/lib/libstd.rlib"),
            Some(PathBuf::from("lib/rustlib/x/lib/libstd.rlib"))
        );
        for skipped in [
            "rustc-1.99.1-x/install.sh",
            "rustc-1.99.1-x/components",
            "rustc-1.99.1-x/rustc/manifest.in",
            "rustc-1.99.1-x/rustc",
            "rustc-1.99.1-x/../../etc/passwd",
            "/rustc-1.99.1-x/rustc/bin/rustc",
        ] {
            assert_eq!(at(skipped), None, "{skipped}");
        }
    }

    #[test]
    fn a_component_archive_unpacks_as_its_installer_would() {
        let dir = std::env::temp_dir().join(format!("horizon-rust-unpack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("cargo.tar.gz");
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            fs::File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        ));
        for (path, body, mode) in [
            ("cargo-1.99.1-x/install.sh", "#!/bin/sh", 0o755),
            ("cargo-1.99.1-x/cargo/manifest.in", "file:bin/cargo", 0o644),
            ("cargo-1.99.1-x/cargo/bin/cargo", "binary", 0o755),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(mode);
            header.set_cksum();
            tar.append_data(&mut header, path, body.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap().flush().unwrap();
        let prefix = dir.join("prefix");
        unpack(&archive, &prefix).unwrap();
        assert_eq!(
            fs::read_to_string(prefix.join("bin/cargo")).unwrap(),
            "binary"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(prefix.join("bin/cargo"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "cargo stays executable");
        }
        assert!(!prefix.join("install.sh").exists() && !prefix.join("manifest.in").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_command_uses_only_the_launchers_toolchain() {
        let toolchain = Toolchain {
            dir: PathBuf::from("/rust/1.99.1-x"),
        };
        let cmd = toolchain.command(&toolchain.cargo());
        let env: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        assert_eq!(get("RUSTUP_TOOLCHAIN"), Some(None));
        assert_eq!(get("RUSTC_WRAPPER"), Some(None));
        assert_eq!(
            PathBuf::from(get("RUSTC").flatten().unwrap()),
            toolchain.dir.join("bin").join(exe("rustc"))
        );
        assert_eq!(
            std::env::split_paths(&get("PATH").flatten().unwrap()).next(),
            Some(toolchain.dir.join("bin"))
        );
        assert!(get("CARGO_HOME").flatten().unwrap().ends_with("cargo"));
    }

    /// The tools' scratch dir is the build's own (#5189): MinGW-w64's gcc, as and ld can't
    /// open a non-ASCII %TEMP%, like the one under a Cyrillic user name.
    #[test]
    fn the_command_uses_the_builds_own_temp_dir() {
        let toolchain = Toolchain {
            dir: PathBuf::from("/rust/1.99.1-x"),
        };
        let cmd = toolchain.command(&toolchain.cargo());
        let tmp = backend::build_dir().join("tmp");
        for name in ["TMPDIR", "TMP", "TEMP"] {
            let value = cmd
                .get_envs()
                .find(|(k, _)| *k == name)
                .and_then(|(_, v)| v)
                .map(PathBuf::from);
            assert_eq!(value.as_deref(), Some(tmp.as_path()), "{name}");
        }
    }

    #[test]
    fn mingw_files_land_under_mingw64_without_the_documentation() {
        let at = |p: &str| mingw_path(Path::new(p));
        assert_eq!(
            at("mingw64/bin/gcc.exe"),
            Some(PathBuf::from("mingw64/bin/gcc.exe"))
        );
        assert_eq!(
            at("mingw64/libexec/gcc/x86_64-w64-mingw32/16.2.0/cc1.exe"),
            Some(PathBuf::from(
                "mingw64/libexec/gcc/x86_64-w64-mingw32/16.2.0/cc1.exe"
            ))
        );
        for skipped in [
            "mingw64",
            "mingw64/share/doc/gcc.html",
            "other/bin/gcc.exe",
            "mingw64/../evil.exe",
            "/mingw64/bin/gcc.exe",
        ] {
            assert_eq!(at(skipped), None, "{skipped}");
        }
    }

    #[test]
    fn a_mingw_zip_unpacks_its_programs_and_not_its_documentation() {
        let dir = std::env::temp_dir().join(format!("horizon-mingw-unpack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("mingw.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.add_directory("mingw64/bin/", options).unwrap();
        for (path, body) in [
            ("mingw64/bin/gcc.exe", "gcc"),
            ("mingw64/share/doc/readme.txt", "docs"),
            ("licenses/readme.txt", "outside"),
        ] {
            zip.start_file(path, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let prefix = dir.join("prefix");
        unpack_mingw(&archive, &prefix).unwrap();
        assert_eq!(
            fs::read_to_string(prefix.join("mingw64/bin/gcc.exe")).unwrap(),
            "gcc"
        );
        assert!(!prefix.join("mingw64/share").exists() && !prefix.join("licenses").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_command_puts_mingw_after_rust_when_the_toolchain_has_it() {
        let dir = std::env::temp_dir().join(format!("horizon-mingw-path-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let toolchain = Toolchain { dir: dir.clone() };
        fs::create_dir_all(toolchain.mingw_bin()).unwrap();
        let cmd = toolchain.command(&toolchain.cargo());
        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == "PATH")
            .and_then(|(_, v)| v)
            .unwrap();
        let first: Vec<PathBuf> = std::env::split_paths(path).take(2).collect();
        assert_eq!(first, [dir.join("bin"), toolchain.mingw_bin()]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_linux_build_names_what_is_missing_and_the_distributions_command() {
        // A system with `cc` and `pkg-config` but no development files, as Linux Mint ships.
        let no_dev =
            |program: &str, args: &[&str]| program != "pkg-config" || args[0] != "--exists";
        let mint = "NAME=\"Linux Mint\"\nID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";
        let e = linux_needs(no_dev, mint).unwrap_err();
        assert!(
            e.contains("lacks them: ALSA's development files, libudev's development files."),
            "{e}"
        );
        assert!(
            e.ends_with("\nsudo apt install build-essential pkg-config libasound2-dev libudev-dev")
        );
        // Nothing to check through without pkg-config, and nothing at all on SteamOS.
        let nothing = |_: &str, _: &[&str]| false;
        let deck = "ID=steamos\nID_LIKE=arch\n";
        let e = linux_needs(nothing, deck).unwrap_err();
        assert!(e.contains("a C compiler (cc), pkg-config, ALSA's"), "{e}");
        assert!(e.contains("SteamOS") && !e.contains("pacman"), "{e}");
        // SteamOS 3's GCC runs but finds no `stdint.h` (#4770).
        let headless = |program: &str, args: &[&str]| program != "cc" || args == ["--version"];
        let e = linux_needs(headless, deck).unwrap_err();
        assert!(
            e.contains("lacks them: the C library's headers. SteamOS"),
            "{e}"
        );
        let arch = linux_needs(nothing, "ID=endeavouros\nID_LIKE=arch\n").unwrap_err();
        assert!(arch.contains("pacman -S --needed base-devel"), "{arch}");
        let fedora = linux_needs(nothing, "ID=fedora\n").unwrap_err();
        assert!(fedora.contains("dnf install"), "{fedora}");
        let other = linux_needs(nothing, "ID=gentoo\n").unwrap_err();
        assert!(
            other.contains("package manager") && other.contains("apt install"),
            "{other}"
        );
        assert_eq!(linux_needs(|_: &str, _: &[&str]| true, mint), Ok(()));
    }
}
