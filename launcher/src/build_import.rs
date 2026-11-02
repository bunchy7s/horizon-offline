//! Game builds the player imports: a `horizon-client` they already have (one built on their
//! fork's Actions, to try a merged change before the next Nightly), installed as its own
//! version beside the releases. The program alone can't start: it needs a version folder's
//! game files. So an import is a copy of an installed release or Nightly's folder (its
//! research-lock.json, preparer sources and prepared resources) with the picked program in
//! place of that version's own, as a native build is (`native_mods::install`).
//!
//! Nothing signed it: the Versions page says so. Its tag (`imported-<date>-<sha>`) is no
//! release's, so no update replaces it, prunes it or moves the selection off it.

use crate::backend::{self, CLIENT};
use crate::i18n::{tr, trf};
use crate::native_mods::{self, Target};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

const PREFIX: &str = "imported-";
/// What an import was made from, in its folder.
const RECORD: &str = "imported.json";
/// The most a program may take: a game build is ~100 MB.
const MAX_PROGRAM: u64 = 1 << 30;

/// Whether an installed version is an imported build.
pub fn is_imported(version: &str) -> bool {
    version.starts_with(PREFIX)
}

/// An imported build as a player reads it: `Imported · 2026-11-01`, or None for any other version.
pub fn label(version: &str) -> Option<String> {
    let date = version.strip_prefix(PREFIX)?.get(..10)?;
    Some(trf("Imported · {date}", &[("date", date)]))
}

/// What an imported build was made from: its `imported.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct About {
    /// The release or Nightly whose game files it plays with.
    pub base: String,
    /// The file the player picked.
    pub file: String,
}

/// What installed imported build `version` was made from; None for any other version.
pub fn about(version: &str) -> Option<About> {
    if !is_imported(version) {
        return None;
    }
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(backend::version_dir(version).join(RECORD)).ok()?)
            .ok()?;
    let field = |name: &str| json[name].as_str().map(str::to_owned);
    Some(About {
        base: field("base")?,
        file: field("file")?,
    })
}

/// A version Horizon published and signed: a release or a Nightly.
fn official(version: &str) -> bool {
    backend::is_game_tag(version) || crate::nightly::date_of(version).is_some()
}

/// The installed version an import takes its game files from: `requested` (`--version`), else
/// `selected` when Horizon published it, else the newest installed release or Nightly.
fn base_for(
    installed: &[String],
    requested: Option<&str>,
    selected: Option<&str>,
) -> Result<String, String> {
    let usable = |v: &str| official(v) && installed.iter().any(|t| t == v);
    match requested {
        Some(b) if usable(b) => Ok(b.to_owned()),
        Some(b) => Err(format!("{b} is not an installed release or Nightly.")),
        None => {
            selected
                .filter(|s| usable(s))
                .map(str::to_owned)
                .or_else(|| native_mods::newest_installed(installed).cloned())
                .ok_or_else(|| {
                    tr("Install a release or Nightly first: an imported build plays with its game \
                    files.")
                .to_string()
                })
        }
    }
}

/// Installs the game program at `path` (or the one inside an archive there) as a new version on
/// the game files of `requested` (default: [`base_for`]), prepared from another version where it
/// can be, and selects it. Importing the same program again selects the version it made before.
pub fn import(path: &Path, requested: Option<&str>) -> Result<String, String> {
    let file = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let program = read_program(path, &file)?;
    if !is_game(&program) {
        return Err(not_a_game(&file));
    }
    let selected = backend::Settings::load().selected;
    let base = base_for(&backend::installed(), requested, selected.as_deref())?;
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let tag = install_in(&backend::version_dir(""), &base, &program, &file, &date)?;
    if let Some(made) = about(&tag).filter(|a| requested.is_some() && a.base != base) {
        return Err(format!(
            "{tag} is this program already, on the game files of {}: remove it to import it \
             on {base}.",
            made.base
        ));
    }
    backend::reuse_prepared_for(&tag);
    // Read again: the window may have saved other settings during the copy.
    let mut settings = backend::Settings::load();
    settings.selected = Some(tag.clone());
    settings.save();
    Ok(tag)
}

fn not_a_game(file: &str) -> String {
    trf(
        "{file} is not a Horizon game for this computer.",
        &[("file", file)],
    )
}

/// The program at `path`: the file itself when it is named as the game is (`horizon-client`,
/// "horizon-client (1).exe" as a browser saves a second copy), or from a zip or a .tar.gz, its
/// entry named as the game is on this platform (`CLIENT`), in any folder. A GitHub Actions
/// artifact is a zip around the release's own archive: one archive inside is looked into too.
/// The launcher and horizon-ctl carry the mod API mark too: the name tells them apart.
fn read_program(path: &Path, file: &str) -> Result<Vec<u8>, String> {
    let open = || fs::File::open(path).map_err(|e| format!("{file}: {e}"));
    let lower = file.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return from_tar(open()?, file);
    }
    let zip = match zip::ZipArchive::new(open()?) {
        Ok(zip) => zip,
        // A download cut short, say.
        Err(e) if lower.ends_with(".zip") => return Err(format!("{file}: {e}")),
        Err(_) if !lower.starts_with("horizon-client") => return Err(not_a_game(file)),
        Err(_) => return read_at_most(open()?, file),
    };
    from_zip(zip, file, true)
}

fn holds_no_game(file: &str) -> String {
    trf(
        "{file} holds no {program}.",
        &[("file", file), ("program", CLIENT)],
    )
}

/// The game in `zip`, else (`nested`) in the first zip or .tar.gz it holds.
fn from_zip<R: Read + Seek>(
    mut zip: zip::ZipArchive<R>,
    file: &str,
    nested: bool,
) -> Result<Vec<u8>, String> {
    let mut inner = None;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| format!("{file}: {e}"))?;
        let name = entry
            .enclosed_name()
            .and_then(|p| Some(p.file_name()?.to_string_lossy().into_owned()))
            .unwrap_or_default();
        if !entry.is_file() {
            continue;
        }
        if name == CLIENT {
            return read_at_most(entry, file);
        }
        if nested
            && inner.is_none()
            && [".zip", ".tar.gz", ".tgz"]
                .iter()
                .any(|x| name.ends_with(x))
        {
            inner = Some((i, name));
        }
    }
    let Some((i, name)) = inner else {
        return Err(holds_no_game(file));
    };
    let entry = zip.by_index(i).map_err(|e| format!("{file}: {e}"))?;
    let archive = read_at_most(entry, file)?;
    if !name.ends_with(".zip") {
        return from_tar(&archive[..], file);
    }
    let zip = zip::ZipArchive::new(Cursor::new(archive)).map_err(|e| format!("{name}: {e}"))?;
    from_zip(zip, file, false)
}

/// The game in a .tar.gz (the Linux release's archive).
fn from_tar(from: impl Read, file: &str) -> Result<Vec<u8>, String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(from));
    for entry in tar.entries().map_err(|e| format!("{file}: {e}"))? {
        let entry = entry.map_err(|e| format!("{file}: {e}"))?;
        let named = entry
            .path()
            .is_ok_and(|p| p.file_name() == Some(std::ffi::OsStr::new(CLIENT)));
        if named && entry.header().entry_type().is_file() {
            return read_at_most(entry, file);
        }
    }
    Err(holds_no_game(file))
}

/// All of `from`, refused past [`MAX_PROGRAM`] rather than cut off.
fn read_at_most(from: impl Read, file: &str) -> Result<Vec<u8>, String> {
    let mut program = Vec::new();
    from.take(MAX_PROGRAM + 1)
        .read_to_end(&mut program)
        .map_err(|e| format!("{file}: {e}"))?;
    if program.len() as u64 > MAX_PROGRAM {
        return Err(not_a_game(file));
    }
    Ok(program)
}

/// Whether `program` is a Horizon game for this computer: it starts as this platform's programs
/// do (PE on Windows, Mach-O on macOS, ELF elsewhere: a Linux build picked on Windows would
/// only fail to start) and carries the mod API mark every game since 0.2.69 does.
fn is_game(program: &[u8]) -> bool {
    let magic: &[&[u8]] = if cfg!(windows) {
        &[b"MZ"]
    } else if cfg!(target_os = "macos") {
        // 64-bit Mach-O, and a universal binary.
        &[&[0xcf, 0xfa, 0xed, 0xfe], &[0xca, 0xfe, 0xba, 0xbe]]
    } else {
        &[b"\x7fELF"]
    };
    magic.iter().any(|m| program.starts_with(m))
        && horizon_mods::api_level::read_mark(program).is_some()
}

/// Installs `program` in `versions` as a copy of installed version `base` with it in place of
/// `base`'s game, and returns its tag. Copied beside it and renamed when complete, as an
/// install is: an interrupted import is never installed.
fn install_in(
    versions: &Path,
    base: &str,
    program: &[u8],
    file: &str,
    date: &str,
) -> Result<String, String> {
    let sha256: String = Sha256::digest(program)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let short = &sha256[..7];
    // The same program imported on another day is the version made then.
    let made = fs::read_dir(versions)
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (is_imported(&name)
                && name.ends_with(&format!("-{short}"))
                && e.path().join(".installed").exists())
            .then_some(name)
        });
    if let Some(tag) = made {
        return Ok(tag);
    }
    let tag = format!("{PREFIX}{date}-{short}");
    let dir = versions.join(&tag);
    // This call's own, where no version is listed (`backend::installed` reads the folders of
    // `versions` itself): the window and the CLI may import the same program at once.
    static IMPORTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = IMPORTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = versions
        .join(".importing")
        .join(format!("{tag}-{}-{n}", std::process::id()));
    let record = serde_json::json!({ "base": base, "file": file, "sha256": sha256 });
    let made = fs::create_dir_all(&partial)
        .map_err(|e| e.to_string())
        .and_then(|()| native_mods::copy_base(&versions.join(base), &partial, Target::Client))
        .and_then(|()| finish(&partial, &dir, program, &record).map_err(|e| e.to_string()));
    let _ = retried(|| fs::remove_dir_all(&partial).or_else(|_| gone(&partial)));
    if let Err(e) = made {
        return Err(trf("Could not import the build: {error}", &[("error", &e)]));
    }
    Ok(tag)
}

/// Puts `program`, its record and the installed mark into the copied folder `partial`, then
/// moves it into place as version `dir` in one step: no other call ever sees it half made, and a
/// version already there (the same program, imported meanwhile) stays.
fn finish(
    partial: &Path,
    dir: &Path,
    program: &[u8],
    record: &serde_json::Value,
) -> std::io::Result<()> {
    fs::write(partial.join(CLIENT), program)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(partial.join(CLIENT), fs::Permissions::from_mode(0o755))?;
    }
    fs::write(partial.join(RECORD), record.to_string())?;
    fs::write(partial.join(".installed"), "")?;
    retried(|| match fs::rename(partial, dir) {
        Err(_) if dir.join(".installed").exists() => Ok(()),
        Err(e) => {
            // No mark: not a version, but what a removal left while its game still ran.
            let _ = fs::remove_dir_all(dir);
            Err(e)
        }
        done => done,
    })
}

/// `step`, tried for a second: Windows refuses to rename or delete a folder while a scan still
/// holds a file just written in it (`backend::remove` waits the same).
fn retried(mut step: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let mut attempt = 1;
    loop {
        match step() {
            Err(_) if attempt < 5 => std::thread::sleep(std::time::Duration::from_millis(250)),
            done => return done,
        }
        attempt += 1;
    }
}

/// Ok when `path` is gone.
fn gone(path: &Path) -> std::io::Result<()> {
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(_) => Err(std::io::ErrorKind::DirectoryNotEmpty.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("horizon-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// The entries of `dir`, the copies' own `.importing` left out.
    fn listed(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != ".importing")
            .count()
    }

    /// A program as this platform's are, carrying the mod API mark the game's do.
    fn program(body: &[u8]) -> Vec<u8> {
        let magic: &[u8] = if cfg!(windows) {
            b"MZ"
        } else if cfg!(target_os = "macos") {
            &[0xcf, 0xfa, 0xed, 0xfe]
        } else {
            b"\x7fELF"
        };
        [magic, body, &horizon_mods::api_level::MARK[..]].concat()
    }

    #[test]
    fn an_import_is_its_own_version_on_its_bases_game_files() {
        let versions = dir("install");
        let base = versions.join("nightly-2026-11-01-c9d474d");
        fs::create_dir_all(base.join(".local/horizon-client/26.3")).unwrap();
        fs::write(
            base.join(".local/horizon-client/26.3/horizon-resources.json"),
            b"{}",
        )
        .unwrap();
        fs::write(base.join("research-lock.json"), b"lock").unwrap();
        fs::write(base.join(CLIENT), b"signed").unwrap();
        fs::write(base.join(".installed"), b"").unwrap();
        fs::create_dir_all(base.join("Horizon.app")).unwrap();
        let built = program(b"fork build");
        let tag = install_in(
            &versions,
            "nightly-2026-11-01-c9d474d",
            &built,
            CLIENT,
            "2026-11-01",
        )
        .unwrap();
        assert!(
            tag.starts_with("imported-2026-11-01-") && tag.len() == 27,
            "{tag}"
        );
        assert!(is_imported(&tag) && !official(&tag));
        assert_eq!(label(&tag).as_deref(), Some("Imported · 2026-11-01"));
        let dir = versions.join(&tag);
        assert_eq!(fs::read(dir.join(CLIENT)).unwrap(), built);
        assert_eq!(fs::read(base.join(CLIENT)).unwrap(), b"signed");
        assert!(dir.join(".installed").is_file());
        assert!(dir
            .join(".local/horizon-client/26.3/horizon-resources.json")
            .is_file());
        assert_eq!(fs::read(dir.join("research-lock.json")).unwrap(), b"lock");
        // The macOS app is made again from the imported program (game_app.rs).
        assert!(!dir.join("Horizon.app").exists());
        let record: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join(RECORD)).unwrap()).unwrap();
        assert_eq!(record["base"], "nightly-2026-11-01-c9d474d");
        // The same program again is the same version, left as it is.
        let again = install_in(
            &versions,
            "nightly-2026-11-01-c9d474d",
            &built,
            "x",
            "2026-11-02",
        );
        assert_eq!(again.unwrap(), tag);
        assert_eq!(listed(&versions), 2);
        let _ = fs::remove_dir_all(versions);
    }

    #[test]
    fn a_failed_import_leaves_nothing_behind() {
        let versions = dir("failed");
        // A folder from an import that never finished is not that program's version.
        let built = program(b"build");
        let unfinished = install_in(&versions, "v0.2.95", &built, CLIENT, "2026-10-01");
        assert!(unfinished.is_err(), "no base to copy");
        assert_eq!(listed(&versions), 0);
        assert_eq!(listed(&versions.join(".importing")), 0);
        let short: String = Sha256::digest(&built)[..4]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        fs::create_dir_all(versions.join(format!("imported-2026-10-01-{}", &short[..7]))).unwrap();
        assert!(install_in(&versions, "v0.2.95", &built, CLIENT, "2026-10-02").is_err());
        // What a removal left while its game ran (no mark) gives way to the import.
        fs::create_dir_all(versions.join("v0.2.95")).unwrap();
        fs::write(versions.join("v0.2.95/research-lock.json"), b"lock").unwrap();
        let left = versions.join(format!("imported-2026-10-01-{}", &short[..7]));
        fs::write(left.join(CLIENT), b"old").unwrap();
        let tag = install_in(&versions, "v0.2.95", &built, CLIENT, "2026-10-01").unwrap();
        assert_eq!(versions.join(tag), left);
        assert_eq!(fs::read(left.join(CLIENT)).unwrap(), built);
        assert!(left.join(".installed").is_file());
        let _ = fs::remove_dir_all(versions);
    }

    #[test]
    fn only_a_horizon_game_for_this_computer_is_imported() {
        assert!(is_game(&program(b"")));
        // Another program of this platform, without the game's mark.
        assert!(!is_game(&program(b"")[..program(b"").len() - 27]));
        let other = if cfg!(windows) {
            &b"\x7fELF"[..]
        } else {
            &b"MZ"[..]
        };
        assert!(!is_game(
            &[other, &horizon_mods::api_level::MARK[..]].concat()
        ));
        let d = dir("zip");
        let zip_path = d.join("horizon-client-windows-x86_64.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("README.md", options).unwrap();
        std::io::Write::write_all(&mut zip, b"notes").unwrap();
        zip.start_file(format!("horizon/{CLIENT}"), options)
            .unwrap();
        std::io::Write::write_all(&mut zip, &program(b"zipped")).unwrap();
        zip.finish().unwrap();
        assert_eq!(
            read_program(&zip_path, "a.zip").unwrap(),
            program(b"zipped")
        );
        let name = "horizon-client (1)";
        let plain = d.join(name);
        fs::write(&plain, program(b"plain")).unwrap();
        assert_eq!(read_program(&plain, name).unwrap(), program(b"plain"));
        // The launcher carries the mark too.
        let launcher = d.join("horizon-launcher");
        fs::write(&launcher, program(b"launcher")).unwrap();
        assert!(read_program(&launcher, "horizon-launcher").is_err());
        let empty = d.join("empty.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&empty).unwrap());
        zip.start_file("README.md", options).unwrap();
        zip.finish().unwrap();
        assert!(read_program(&empty, "empty.zip").is_err());
        let _ = fs::remove_dir_all(d);
    }

    /// An Actions artifact: a zip around the release's own archive, a zip (Windows, macOS) or a
    /// .tar.gz (Linux) with the game in its top folder.
    #[test]
    fn the_game_is_found_inside_an_actions_artifact() {
        use std::io::Write;
        let d = dir("artifact");
        let options = zip::write::SimpleFileOptions::default();
        let mut inner = zip::ZipWriter::new(Cursor::new(Vec::new()));
        inner
            .start_file(format!("horizon-windows-x64/{CLIENT}"), options)
            .unwrap();
        inner.write_all(&program(b"inner zip")).unwrap();
        let inner = inner.finish().unwrap().into_inner();
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let game = program(b"inner tar");
        let mut header = tar::Header::new_gnu();
        header.set_size(game.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("horizon-linux-x86_64/{CLIENT}"),
            &game[..],
        )
        .unwrap();
        let tar = tar.into_inner().unwrap().finish().unwrap();
        for (archive, bytes, body) in [
            ("horizon-windows-x64.zip", inner, &b"inner zip"[..]),
            (
                "horizon-linux-x86_64.tar.gz",
                tar.clone(),
                &b"inner tar"[..],
            ),
        ] {
            let artifact = d.join("artifact.zip");
            let mut zip = zip::ZipWriter::new(fs::File::create(&artifact).unwrap());
            zip.start_file(archive, options).unwrap();
            zip.write_all(&bytes).unwrap();
            zip.finish().unwrap();
            assert_eq!(
                read_program(&artifact, "artifact.zip").unwrap(),
                program(body)
            );
        }
        let plain_tar = d.join("horizon-linux-x86_64.tar.gz");
        fs::write(&plain_tar, &tar).unwrap();
        let name = "horizon-linux-x86_64.tar.gz";
        assert_eq!(read_program(&plain_tar, name).unwrap(), game);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn the_base_is_the_selected_release_or_the_newest_one() {
        let installed = [
            "nightly-2026-11-01-c9d474d".to_string(),
            "v0.2.95".to_string(),
            "native-c9d474d-0123456789ab".to_string(),
            "imported-2026-10-31-abcdef0".to_string(),
        ];
        let newest = Ok("nightly-2026-11-01-c9d474d".to_string());
        assert_eq!(
            base_for(&installed, None, Some("v0.2.95")),
            Ok("v0.2.95".to_string())
        );
        for selected in [None, Some("imported-2026-10-31-abcdef0"), Some("v0.1.0")] {
            assert_eq!(base_for(&installed, None, selected), newest);
        }
        assert!(base_for(&installed[2..], None, None).is_err());
        // --version: an installed release or Nightly, never another local build.
        let asked = |b| base_for(&installed, Some(b), Some("v0.2.95"));
        assert_eq!(asked("nightly-2026-11-01-c9d474d"), newest);
        for b in [
            "v0.1.0",
            "native-c9d474d-0123456789ab",
            "imported-2026-10-31-abcdef0",
        ] {
            assert!(asked(b).is_err(), "{b}");
        }
    }
}
