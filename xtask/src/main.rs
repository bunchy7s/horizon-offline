//! Repository tasks, run with `cargo run -p xtask -- TASK`.
//!
//! `package BINARY OUTDIR`: a playable Horizon folder with the game binary, horizon-ctl,
//! horizon-prepare and horizon-server (built beside it: cargo build --release -p horizon-client -p horizon-client-lib
//! --bins -p horizon-prepare -p horizon-server), the exporters horizon-prepare compiles, its sources (the launcher compares
//! them to its own), and PLAY.txt. It holds none of Minecraft's files: players prepare those
//! from their own copy. .github/workflows/release.yml makes the release folders with it.
//!
//! `package-server ENGINE OUTDIR`: a server folder (docs/server.md): the server build of the game
//! (`cargo build --release -p horizon-client --no-default-features`, without sound or controllers)
//! as horizon-engine, with horizon-prepare and horizon-server from beside it and the same sources.
//!
//! `tokens`: writes `crates/horizon-theme/tokens.css`, the design tokens of `crates/horizon-theme` as CSS
//! custom properties (a horizon-theme test fails when the file is out of date).
//!
//! `icons`: renders the icon masters in `docs/media/brand` (with `rsvg-convert`, from librsvg)
//! into every size the platforms and the website use, and packs `packaging/icons/horizon.ico`
//! and `horizon.icns`. `docs/media/brand/README.md` lists what goes where.

mod mod_api;

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: cargo run -p xtask -- package BINARY OUTDIR | package-server ENGINE OUTDIR | tokens | icons";

/// What the release folder takes from the repository at HEAD, as `git archive` writes it.
const ARCHIVED: [&str; 5] = [
    "tools",
    "mods/horizon/src/test/java",
    "crates/horizon-prepare",
    "research-lock.json",
    "LICENSE",
];

/// The programs that sit beside the client: horizon-ctl, the command line and MCP server that
/// play the game, horizon-prepare, which prepares the resources from the player's own
/// Minecraft, and horizon-server, which hosts a server with the client as its engine
/// (docs/server.md).
const TOOLS: [&str; 3] = ["horizon-ctl", "horizon-prepare", "horizon-server"];

/// The programs beside the engine in a server folder.
const SERVER_TOOLS: [&str; 2] = ["horizon-prepare", "horizon-server"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [task, binary, out] if task == "package" => package(Path::new(binary), Path::new(out)),
        [task, engine, out] if task == "package-server" => {
            package_server(Path::new(engine), Path::new(out))
        }
        [task] if task == "mod-api" => mod_api::run(false),
        [task, check] if task == "mod-api" && check == "--check" => mod_api::run(true),
        [task] if task == "tokens" => tokens(),
        [task] if task == "icons" => icons(),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn tokens() -> Result<(), String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/horizon-theme/tokens.css");
    fs::write(&path, horizon_theme::css()).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote crates/horizon-theme/tokens.css");
    Ok(())
}

/// The square icon's sizes: Linux's hicolor theme, and the ones the .ico holds.
const HICOLOR: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];
const ICO: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
/// The .icns entries: type and pixel size, macOS's tile drawn on Apple's grid.
const ICNS: [(&[u8; 4], u32); 11] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"icp6", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic13", 256),
    (b"ic14", 512),
];

fn render(svg: &Path, size: (u32, u32)) -> Result<Vec<u8>, String> {
    let output = Command::new("rsvg-convert")
        .arg("-w")
        .arg(size.0.to_string())
        .arg("-h")
        .arg(size.1.to_string())
        .arg(svg)
        .output()
        .map_err(|e| format!("rsvg-convert (librsvg): {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "rsvg-convert {}: {}",
            svg.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn write(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
}

/// An .ico whose images are PNGs (Windows Vista and later read them at every size).
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0, 0, 1, 0];
    out.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, png) in images {
        // A width or height of 256 is written as 0.
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend([side, side, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((png.len() as u32).to_le_bytes());
        out.extend((offset as u32).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images {
        out.extend(png);
    }
    out
}

/// An .icns of PNG entries.
fn icns(entries: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let body: usize = entries.iter().map(|(_, png)| 8 + png.len()).sum();
    let mut out = b"icns".to_vec();
    out.extend(((8 + body) as u32).to_be_bytes());
    for (kind, png) in entries {
        out.extend(*kind);
        out.extend(((8 + png.len()) as u32).to_be_bytes());
        out.extend(png);
    }
    out
}

fn icons() -> Result<(), String> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let brand = repo.join("docs/media/brand");
    let square = brand.join("icon.svg");
    let packaging = repo.join("packaging/icons");
    for size in HICOLOR {
        let png = render(&square, (size, size))?;
        write(
            &packaging.join(format!("hicolor/{size}x{size}/apps/horizon.png")),
            &png,
        )?;
    }
    write(
        &packaging.join("hicolor/scalable/apps/horizon.svg"),
        &fs::read(&square).map_err(|e| format!("{}: {e}", square.display()))?,
    )?;
    let images = ICO
        .iter()
        .map(|&s| render(&square, (s, s)).map(|png| (s, png)))
        .collect::<Result<Vec<_>, _>>()?;
    write(&packaging.join("horizon.ico"), &ico(&images))?;
    let macos = brand.join("icon-macos.svg");
    let entries = ICNS
        .iter()
        .map(|&(kind, s)| render(&macos, (s, s)).map(|png| (kind, png)))
        .collect::<Result<Vec<_>, _>>()?;
    write(&packaging.join("horizon.icns"), &icns(&entries))?;
    // The window icon the game and the launcher set at run time (X11 and Windows title bars).
    write(
        &repo.join("crates/horizon-theme/icons/horizon-64.png"),
        &render(&square, (64, 64))?,
    )?;
    // Avatars, and the website's icons (horizon-site copies docs/media/brand/web/ into its public/).
    let avatar = brand.join("avatar.svg");
    for (svg, size, out) in [
        (
            &avatar,
            500,
            "docs/media/brand/social/github-avatar-500.png",
        ),
        (&avatar, 180, "docs/media/brand/web/apple-touch-icon.png"),
        (&square, 32, "docs/media/brand/web/favicon-32.png"),
        (&square, 1024, "docs/media/brand/icon-1024.png"),
        (&macos, 1024, "docs/media/brand/icon-macos-1024.png"),
    ] {
        write(&repo.join(out), &render(svg, (size, size))?)?;
    }
    write(
        &repo.join("docs/media/brand/web/favicon.svg"),
        &fs::read(&square).map_err(|e| format!("{}: {e}", square.display()))?,
    )?;
    println!("wrote packaging/icons, the window icon, the avatars and the website's icons");
    Ok(())
}

fn package(binary: &Path, out: &Path) -> Result<(), String> {
    let repo = new_folder(out)?;
    copy_into(binary, out)?;
    copy_beside(binary, &TOOLS, out)?;
    copy_exporters(&repo.join("exporters"), &out.join("exporters"))?;
    let play = repo.join("packaging/PLAY.txt");
    copy(&play, &out.join("PLAY.txt"))
}

fn package_server(engine: &Path, out: &Path) -> Result<(), String> {
    let repo = new_folder(out)?;
    copy_exporters(&repo.join("exporters"), &out.join("exporters"))?;
    copy(engine, &out.join(program_name(engine, "horizon-engine")))?;
    copy_beside(engine, &SERVER_TOOLS, out)
}

/// Release builds supply only our compiled classes and their integrity manifest.
fn copy_exporters(from: &Path, to: &Path) -> Result<(), String> {
    if !from.join("manifest.json").is_file() {
        return Err("Missing exporters/manifest.json; run horizon-prepare compile-exporters and copy its bundle to exporters/".into());
    }
    copy_exporter_tree(from, to)
}

fn copy_exporter_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(from).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_exporter_tree(&path, &target)?;
        } else {
            copy(&path, &target)?;
        }
    }
    Ok(())
}

/// `OUT`, emptied, with the sources the release folders take (`ARCHIVED`); the repository's path.
fn new_folder(out: &Path) -> Result<PathBuf, String> {
    let top = git(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &["rev-parse", "--show-toplevel"],
    )?;
    let repo = PathBuf::from(String::from_utf8_lossy(&top).trim());
    match fs::remove_dir_all(out) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {e}", out.display()));
        }
        _ => {}
    }
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut args = vec!["archive", "--format=tar", "HEAD"];
    args.extend(ARCHIVED);
    extract(&git(&repo, &args)?, out)?;
    Ok(repo)
}

/// `name`, with `.exe` when `program` has it.
fn program_name(program: &Path, name: &str) -> String {
    if program.extension().is_some_and(|e| e == "exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Copies `tools`, built beside `program`, into `out`.
fn copy_beside(program: &Path, tools: &[&str], out: &Path) -> Result<(), String> {
    for tool in tools {
        let path = program
            .parent()
            .unwrap_or(Path::new(""))
            .join(program_name(program, tool));
        if !path.is_file() {
            return Err(format!(
                "{} is missing; build it too: cargo build --release -p horizon-client -p horizon-client-lib --bins -p horizon-prepare -p horizon-server",
                path.display()
            ));
        }
        copy_into(&path, out)?;
    }
    Ok(())
}

/// `cp FILE DIR/`.
fn copy_into(file: &Path, dir: &Path) -> Result<(), String> {
    let name = file
        .file_name()
        .ok_or_else(|| format!("{} is not a file", file.display()))?;
    copy(file, &dir.join(name))
}

/// `cp FROM TO` to a new file: the source's mode within the umask, so the programs stay
/// executable.
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let error = |e: std::io::Error| format!("{}: {e}", from.display());
    let data = fs::read(from).map_err(error)?;
    let permissions = fs::metadata(from).map_err(error)?.permissions();
    #[cfg(unix)]
    let mode = std::os::unix::fs::PermissionsExt::mode(&permissions);
    #[cfg(not(unix))]
    let mode = if permissions.readonly() { 0o444 } else { 0o666 };
    create(to, &data, mode)
}

/// A new file holding `data`, created with `mode` within the process umask as tar and cp create
/// theirs (on Windows only the read-only bit counts).
fn create(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    use std::io::Write;
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode & 0o7777);
    options
        .open(path)
        .and_then(|mut file| file.write_all(data))
        .map_err(error)?;
    if cfg!(not(unix)) && mode & 0o222 == 0 {
        let mut permissions = fs::metadata(path).map_err(error)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).map_err(error)?;
    }
    Ok(())
}

/// A NUL-terminated tar header field.
fn field(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())]
}

fn octal(bytes: &[u8]) -> Result<u64, String> {
    let text = std::str::from_utf8(field(bytes)).map_err(|e| e.to_string())?;
    let text = text.trim_matches(|c: char| c == ' ' || c == '\0');
    u64::from_str_radix(if text.is_empty() { "0" } else { text }, 8)
        .map_err(|e| format!("bad tar number {text:?}: {e}"))
}

/// `path` under `out`, refusing anything that would land outside it.
fn inside(out: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path.trim_end_matches('/'));
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!("unsafe path in the archive: {path}"));
    }
    Ok(out.join(relative))
}

/// The `path` record of a pax extended header, if it has one.
fn pax_path(mut records: &[u8]) -> Result<Option<String>, String> {
    let mut path = None;
    while !records.is_empty() {
        let space = records
            .iter()
            .position(|b| *b == b' ')
            .ok_or("bad pax record")?;
        let length: usize = std::str::from_utf8(&records[..space])
            .ok()
            .and_then(|n| n.parse().ok())
            .filter(|n| *n > space && *n <= records.len())
            .ok_or("bad pax record length")?;
        let record = &records[space + 1..length - 1];
        if let Some(value) = record.strip_prefix(b"path=") {
            path = Some(String::from_utf8(value.to_vec()).map_err(|e| e.to_string())?);
        }
        records = &records[length..];
    }
    Ok(path)
}

/// `tar -x -C out` for the ustar/pax archives `git archive` writes: directories and regular
/// files, created with the archive's modes within the process umask as tar creates them for an
/// ordinary user, so executables stay executable.
fn extract(tar: &[u8], out: &Path) -> Result<(), String> {
    let mut pos = 0;
    let mut long_path = None;
    while pos + 512 <= tar.len() {
        let header = &tar[pos..pos + 512];
        if header.iter().all(|b| *b == 0) {
            return Ok(());
        }
        let size = octal(&header[124..136])? as usize;
        let start = pos + 512;
        let data = tar
            .get(start..start + size)
            .ok_or("the archive ends inside an entry")?;
        pos = start + size.div_ceil(512) * 512;
        let path = match long_path.take() {
            Some(path) => path,
            None => {
                let name = String::from_utf8_lossy(field(&header[..100])).into_owned();
                let prefix = String::from_utf8_lossy(field(&header[345..500])).into_owned();
                if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                }
            }
        };
        let mode = octal(&header[100..108])? as u32;
        match header[156] {
            // The global header holds the commit id.
            b'g' => {}
            b'x' => long_path = pax_path(data)?,
            b'5' => {
                let dir = inside(out, &path)?;
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true);
                #[cfg(unix)]
                std::os::unix::fs::DirBuilderExt::mode(&mut builder, mode & 0o7777);
                builder
                    .create(&dir)
                    .map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            b'0' | 0 => {
                let file = inside(out, &path)?;
                if let Some(parent) = file.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                create(&file, data, mode)?;
            }
            kind => {
                return Err(format!(
                    "{path}: unsupported entry type {:?} in the archive",
                    kind as char
                ));
            }
        }
    }
    Err("the archive has no end".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, mode: u32, size: usize, kind: u8) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h
    }

    fn entry(tar: &mut Vec<u8>, name: &str, mode: u32, kind: u8, data: &[u8]) {
        tar.extend(header(name, mode, data.len(), kind));
        tar.extend(data);
        tar.resize(tar.len().div_ceil(512) * 512, 0);
    }

    #[test]
    fn archives_extract_with_their_paths_and_modes() {
        let out = std::env::temp_dir().join(format!("xtask-extract-{}", std::process::id()));
        let _ = fs::remove_dir_all(&out);
        let long = format!("tools/{}/run.sh", "d".repeat(120));
        let pax = format!("path={long}\n");
        let pax = format!("{} {pax}", pax.len() + 4);
        let mut tar = Vec::new();
        entry(
            &mut tar,
            "pax_global_header",
            0o666,
            b'g',
            b"52 comment=abc\n",
        );
        entry(&mut tar, "tools/", 0o775, b'5', b"");
        entry(&mut tar, "tools/a.py", 0o664, b'0', b"print(1)\n");
        entry(&mut tar, "tools/b.sh", 0o775, b'0', b"#!/bin/sh\n");
        entry(&mut tar, "PaxHeaders/x", 0o666, b'x', pax.as_bytes());
        entry(&mut tar, "ignored-name", 0o775, b'0', b"long\n");
        tar.extend([0; 1024]);
        extract(&tar, &out).unwrap();
        assert_eq!(fs::read(out.join("tools/a.py")).unwrap(), b"print(1)\n");
        assert_eq!(fs::read(out.join(&long)).unwrap(), b"long\n");
        assert!(!out.join("ignored-name").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &str| fs::metadata(out.join(p)).unwrap().permissions().mode();
            assert_eq!(mode("tools/a.py") & 0o111, 0);
            assert_eq!(mode("tools/b.sh") & 0o100, 0o100);
            assert_eq!(mode(&long) & 0o100, 0o100);
            assert_eq!(mode("tools") & 0o100, 0o100);
        }
        fs::remove_dir_all(&out).unwrap();
    }

    #[test]
    fn archives_stay_inside_the_folder() {
        let out = Path::new("out");
        assert!(inside(out, "../x").is_err());
        assert!(inside(out, "/etc/x").is_err());
        assert_eq!(inside(out, "tools/").unwrap(), out.join("tools"));
        let mut tar = Vec::new();
        entry(&mut tar, "link", 0o777, b'2', b"");
        tar.extend([0; 1024]);
        assert!(extract(&tar, out).unwrap_err().contains("unsupported"));
    }
}
