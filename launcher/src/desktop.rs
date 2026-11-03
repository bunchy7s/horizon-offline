//! Linux: the launcher puts itself in the applications menu. It writes
//! `applications/horizon-launcher.desktop` (pointing at wherever the binary is now) and Horizon's
//! icon into the `hicolor` theme, under `$XDG_DATA_HOME` (`~/.local/share` by default), so no root
//! is needed; `horizon.desktop` (hidden from menus) gives the game's window its icon on Wayland.
//! A release build writes it when it starts only if there is no entry yet or the one there starts
//! a program that is gone ([`repair`]), so a development or test build never takes over a working
//! entry. Turning the Settings switch on (or `horizon-launcher desktop-entry`) points it at this
//! binary whatever is there ([`install`]); the switch off and `desktop-entry --remove` take it all
//! away again.
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The icon theme sizes, as `packaging/icons/hicolor` holds them (`cargo run -p xtask -- icons`).
const ICONS: [(&str, &[u8]); 9] = [
    (
        "16x16",
        include_bytes!("../../packaging/icons/hicolor/16x16/apps/horizon.png"),
    ),
    (
        "24x24",
        include_bytes!("../../packaging/icons/hicolor/24x24/apps/horizon.png"),
    ),
    (
        "32x32",
        include_bytes!("../../packaging/icons/hicolor/32x32/apps/horizon.png"),
    ),
    (
        "48x48",
        include_bytes!("../../packaging/icons/hicolor/48x48/apps/horizon.png"),
    ),
    (
        "64x64",
        include_bytes!("../../packaging/icons/hicolor/64x64/apps/horizon.png"),
    ),
    (
        "128x128",
        include_bytes!("../../packaging/icons/hicolor/128x128/apps/horizon.png"),
    ),
    (
        "256x256",
        include_bytes!("../../packaging/icons/hicolor/256x256/apps/horizon.png"),
    ),
    (
        "512x512",
        include_bytes!("../../packaging/icons/hicolor/512x512/apps/horizon.png"),
    ),
    (
        "scalable",
        include_bytes!("../../packaging/icons/hicolor/scalable/apps/horizon.svg"),
    ),
];

const LAUNCHER_ENTRY: &str = "horizon-launcher.desktop";
const GAME_ENTRY: &str = "horizon.desktop";
const GAME_ENTRY_TEXT: &str = include_str!("../../packaging/linux/horizon.desktop");

/// `$XDG_DATA_HOME`, or `~/.local/share` when it is unset or not absolute (as the spec says).
pub fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
}

/// Quotes a path for an `Exec` key: inside double quotes, `"`, `` ` ``, `$` and `\` are escaped.
pub(crate) fn quote(path: &Path) -> String {
    let mut out = String::from("\"");
    for c in path.to_string_lossy().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The launcher's entry, starting the binary at `exe`.
pub fn launcher_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Horizon\n\
         GenericName=Game launcher\n\
         Comment=Install, prepare and play Horizon, Minecraft: Java Edition {minecraft} rewritten in Rust\n\
         Exec={}\n\
         TryExec={}\n\
         Icon=horizon\n\
         Terminal=false\n\
         Categories=Game;\n\
         StartupWMClass=horizon-launcher\n",
        quote(exe),
        exe.display(),
        minecraft = horizon_core::VANILLA_VERSION,
    )
}

/// Every file this module writes, with its contents, under `data`.
fn files(data: &Path, exe: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = vec![
        (
            data.join("applications").join(LAUNCHER_ENTRY),
            launcher_entry(exe).into_bytes(),
        ),
        (
            data.join("applications").join(GAME_ENTRY),
            GAME_ENTRY_TEXT.as_bytes().to_vec(),
        ),
    ];
    for (size, bytes) in ICONS {
        let name = if size == "scalable" {
            "horizon.svg"
        } else {
            "horizon.png"
        };
        out.push((
            data.join("icons/hicolor")
                .join(size)
                .join("apps")
                .join(name),
            bytes.to_vec(),
        ));
    }
    out
}

/// Writes whatever is missing or out of date (a moved binary changes the entry). Returns
/// whether anything was written.
pub fn install_in(data: &Path, exe: &Path) -> io::Result<bool> {
    let mut wrote = false;
    for (path, bytes) in files(data, exe) {
        if fs::read(&path).ok().as_deref() == Some(&bytes[..]) {
            continue;
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Written beside and renamed, so a menu never reads half an entry.
        let temp = path.with_extension("horizon-tmp");
        fs::write(&temp, &bytes)?;
        fs::rename(&temp, &path)?;
        wrote = true;
    }
    Ok(wrote)
}

/// Removes every file `install_in` writes (the folders stay: other apps share them).
pub fn remove_in(data: &Path) -> io::Result<()> {
    for (path, _) in files(data, Path::new("")) {
        match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// The program an entry starts: its `TryExec`, else the first word of its `Exec` (unquoted as
/// [`quote`] writes it). None when the entry names neither.
fn entry_target(entry: &str) -> Option<String> {
    let mut in_main = false;
    let (mut exec, mut try_exec) = (None, None);
    for line in entry.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
        } else if let Some((key, value)) = line.split_once('=').filter(|_| in_main) {
            match key.trim() {
                "TryExec" if try_exec.is_none() => try_exec = Some(value.trim()),
                "Exec" if exec.is_none() => exec = Some(value.trim()),
                _ => {}
            }
        }
    }
    if let Some(program) = try_exec.filter(|p| !p.is_empty()) {
        return Some(program.to_string());
    }
    let exec = exec?;
    let program = match exec.strip_prefix('"') {
        Some(rest) => {
            let (mut out, mut chars) = (String::new(), rest.chars());
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => out.extend(chars.next()),
                    c => out.push(c),
                }
            }
            out
        }
        None => exec.split_whitespace().next()?.to_string(),
    };
    (!program.is_empty()).then_some(program)
}

/// Whether an entry's program is there: a path that exists, or a bare name found on `path`
/// (`$PATH`), as menus look `TryExec` up.
fn target_exists(program: &str, path: Option<&OsStr>) -> bool {
    if program.contains('/') {
        return Path::new(program).exists();
    }
    path.is_some_and(|dirs| std::env::split_paths(dirs).any(|dir| dir.join(program).is_file()))
}

/// Whether the launcher may write its entry by itself: there is none under `data`, or the one
/// there starts a program that no longer exists. An entry pointing at another launcher that
/// still works is left alone.
pub fn needs_repair_in(data: &Path, path: Option<&OsStr>) -> bool {
    match fs::read_to_string(data.join("applications").join(LAUNCHER_ENTRY)) {
        Ok(entry) => !entry_target(&entry).is_some_and(|p| target_exists(&p, path)),
        Err(e) => e.kind() == io::ErrorKind::NotFound,
    }
}

/// What the entry starts: a distributor's wrapper (`HORIZON_LAUNCHER_DESKTOP_EXEC`) when one is
/// configured, else the AppImage the launcher runs from (the copy `current_exe` names is inside its
/// mount, gone once the launcher exits), else the binary at `exe`.
pub(crate) fn target(exe: &Path) -> PathBuf {
    target_from(exe, crate::backend::appimage_path().as_deref())
}

fn target_from(exe: &Path, appimage: Option<&Path>) -> PathBuf {
    std::env::var_os("HORIZON_LAUNCHER_DESKTOP_EXEC")
        .filter(|path| !path.is_empty())
        .map_or_else(|| appimage.unwrap_or(exe).to_path_buf(), PathBuf::from)
}

/// Registers the launcher whatever entry is there: for the player turning it on (the Settings
/// switch, `horizon-launcher desktop-entry`).
pub fn install(exe: &Path) -> Result<bool, String> {
    let data = data_home().ok_or("No home folder to put the menu entry in.")?;
    install_in(&data, &target(exe)).map_err(|e| format!("{}: {e}", data.display()))
}

/// At start: registers the launcher only when [`needs_repair_in`] says so. Returns whether it
/// wrote anything.
pub fn repair(exe: &Path) -> Result<bool, String> {
    let data = data_home().ok_or("No home folder to put the menu entry in.")?;
    if !needs_repair_in(&data, std::env::var_os("PATH").as_deref()) {
        return Ok(false);
    }
    install_in(&data, &target(exe)).map_err(|e| format!("{}: {e}", data.display()))
}

pub fn remove() -> Result<(), String> {
    let data = data_home().ok_or("No home folder.")?;
    remove_in(&data).map_err(|e| format!("{}: {e}", data.display()))
}

/// Whether the launcher's entry is there.
pub fn installed() -> bool {
    data_home().is_some_and(|d| d.join("applications").join(LAUNCHER_ENTRY).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_paths_are_quoted() {
        let entry = launcher_entry(Path::new("/home/a b/Horizon \"x\"/horizon-launcher"));
        assert!(entry.contains("Exec=\"/home/a b/Horizon \\\"x\\\"/horizon-launcher\"\n"));
        assert!(entry.contains("TryExec=/home/a b/Horizon \"x\"/horizon-launcher\n"));
        assert!(entry.contains("Icon=horizon\n"));
    }

    #[test]
    fn installs_follows_a_moved_binary_and_removes() {
        let data = std::env::temp_dir().join(format!("horizon-desktop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        assert!(install_in(&data, Path::new("/opt/a/horizon-launcher")).unwrap());
        assert!(!install_in(&data, Path::new("/opt/a/horizon-launcher")).unwrap());
        let png = data.join("icons/hicolor/48x48/apps/horizon.png");
        assert!(fs::read(&png).unwrap().starts_with(b"\x89PNG"));
        assert!(data
            .join("icons/hicolor/scalable/apps/horizon.svg")
            .exists());
        assert!(install_in(&data, Path::new("/opt/b/horizon-launcher")).unwrap());
        let entry = fs::read_to_string(data.join("applications").join(LAUNCHER_ENTRY)).unwrap();
        assert!(entry.contains("Exec=\"/opt/b/horizon-launcher\""));
        remove_in(&data).unwrap();
        assert!(!data.join("applications").join(LAUNCHER_ENTRY).exists());
        assert!(!png.exists());
        remove_in(&data).unwrap();
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn the_target_is_try_exec_else_the_first_word_of_exec() {
        let ours = launcher_entry(Path::new("/opt/a b/horizon \"x\"/horizon-launcher"));
        assert_eq!(
            entry_target(&ours).as_deref(),
            Some("/opt/a b/horizon \"x\"/horizon-launcher")
        );
        let without_try = ours.replace("TryExec=", "X-Old=");
        assert_eq!(
            entry_target(&without_try).as_deref(),
            Some("/opt/a b/horizon \"x\"/horizon-launcher")
        );
        let wrapper = "[Desktop Entry]\nExec=flatpak run dev.horizon.Launcher %U\n";
        assert_eq!(entry_target(wrapper).as_deref(), Some("flatpak"));
        // Only the main group counts: an action's Exec is not the launcher.
        let action = "[Desktop Action play]\nExec=/a\n[Desktop Entry]\nExec=/b --x\n";
        assert_eq!(entry_target(action).as_deref(), Some("/b"));
        assert_eq!(entry_target("[Desktop Entry]\nName=Horizon\n"), None);
        assert_eq!(entry_target("[Desktop Entry]\nExec=\nTryExec=\n"), None);
    }

    /// A build that starts never takes over an entry that works (a test or development build
    /// started on a player's machine, 2026-10-23); it writes one only when there is none or the
    /// program it starts is gone.
    #[test]
    fn a_start_writes_the_entry_only_when_none_works() {
        let root =
            std::env::temp_dir().join(format!("horizon-desktop-repair-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (data, bin) = (root.join("data"), root.join("bin"));
        fs::create_dir_all(&bin).unwrap();
        let installed = bin.join("horizon-launcher");
        fs::write(&installed, b"").unwrap();
        let entry = data.join("applications").join(LAUNCHER_ENTRY);
        let path = Some(bin.as_os_str());

        // No entry: write one.
        assert!(needs_repair_in(&data, path));
        install_in(&data, &installed).unwrap();
        // Ours works: a build elsewhere leaves it alone.
        assert!(!needs_repair_in(&data, path));
        // So does one another launcher wrote, found on PATH or by its path.
        fs::write(&entry, "[Desktop Entry]\nExec=horizon-launcher %U\n").unwrap();
        assert!(!needs_repair_in(&data, path));
        assert!(needs_repair_in(&data, None));
        // The program is gone (a deleted target/ folder, a moved binary): write it again.
        install_in(&data, &installed).unwrap();
        fs::remove_file(&installed).unwrap();
        assert!(needs_repair_in(&data, path));
        // An entry that starts nothing is broken too.
        fs::write(&entry, "[Desktop Entry]\nName=Horizon\n").unwrap();
        assert!(needs_repair_in(&data, path));
        let _ = fs::remove_dir_all(&root);
    }

    /// Inside an AppImage the entry starts the AppImage, not the copy in its mount, which is gone
    /// when the launcher exits.
    #[test]
    fn an_appimage_entry_starts_the_appimage() {
        let exe = Path::new("/tmp/.mount_Horizon123/usr/bin/horizon-launcher");
        let appimage = Path::new("/home/a b/Horizon.AppImage");
        assert_eq!(target_from(exe, Some(appimage)), appimage);
        assert_eq!(target_from(exe, None), exe);
        // Quoted, so the space in the player's folder survives.
        assert!(launcher_entry(&target_from(exe, Some(appimage)))
            .contains("Exec=\"/home/a b/Horizon.AppImage\"\n"));
    }
}
