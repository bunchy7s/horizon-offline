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
    /// the player's rustup or compiler-wrapper settings.
    pub fn command(&self, program: &Path) -> Command {
        let mut cmd = Command::new(program);
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mingw = Some(self.mingw_bin()).filter(|bin| bin.is_dir());
        let paths = std::iter::once(self.dir.join("bin"))
            .chain(mingw)
            .chain(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(paths).unwrap_or(path))
            .env("RUSTC", self.dir.join("bin").join(exe("rustc")))
            .env("CARGO_HOME", cargo_home());
        for name in [
            "RUSTUP_TOOLCHAIN",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
            "CARGO_BUILD_RUSTC_WRAPPER",
            "CARGO_TARGET_DIR",
        ] {
            cmd.env_remove(name);
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

/// Where toolchains live: `<launcher data>/rust/`.
pub fn root() -> PathBuf {
    backend::data_dir().join("rust")
}

/// The Cargo home of native builds (downloaded crates, horizon-mixins), never the player's own.
pub fn cargo_home() -> PathBuf {
    backend::data_dir().join("cargo")
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

/// Whether this computer can link what the toolchain compiles: Rust brings its compiler, but uses
/// the system's linker on macOS (the Command Line Tools) and Linux (`cc`). Windows' `windows-gnu`
/// toolchain brings its own, and MinGW-w64 for C and import libraries ([`MINGW`]).
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
        "linux" if !ok("cc", &["--version"]) => Err(
            "Native mods need a C compiler (`cc`) to link the game. Install gcc or clang with your distribution's package manager, then build again."
                .into(),
        ),
        _ => Ok(()),
    }
}

/// Removes every toolchain and the native builds' Cargo home.
pub fn remove() -> Result<(), String> {
    for dir in [root(), cargo_home()] {
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
}
