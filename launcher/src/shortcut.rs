//! A shortcut on the desktop that plays one instance: it runs
//! `horizon-launcher play --instance <id>`, so the game starts without the launcher's window.
//!
//! - Linux: `<name> (Horizon).desktop` on the desktop (executable, and marked trusted with `gio`
//!   where it exists, so GNOME starts it), and the same entry in the applications menu
//!   (`applications/horizon-instance-<id>.desktop`, `desktop::data_home`).
//! - macOS: `<name> (Horizon).app`, a small app whose program is a shell script, with Horizon's
//!   icon. It has no Dock icon of its own (`LSUIElement`): the game's window is the one to see.
//! - Windows: `<name> (Horizon).lnk`, a Shell Link as Microsoft's published [MS-SHLLINK] format
//!   lays it out, to the launcher's `.exe` with Horizon's icon.
//!
//! Each shortcut names its instance's id inside it, so removing finds it after a rename.
//! `HORIZON_LAUNCHER_DESKTOP` points the desktop folder elsewhere, for tests.
//!
//! [MS-SHLLINK]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/
use crate::desktop;
use crate::desktop_seams;
use std::fs;
use std::path::{Path, PathBuf};

pub const DESKTOP_ENV: &str = "HORIZON_LAUNCHER_DESKTOP";
const MAC_ICON: &[u8] = include_bytes!("../../packaging/icons/horizon.icns");
/// The key a Linux shortcut names its instance with.
const INSTANCE_KEY: &str = "X-Horizon-Instance";
/// And a macOS one, in its Info.plist.
const MAC_KEY: &str = "<key>HorizonInstance</key>";

/// The kinds of shortcut, one per system: all of them can be written and tested anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    Linux,
    Mac,
    Windows,
}

impl System {
    pub fn here() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Linux => "desktop",
            Self::Mac => "app",
            Self::Windows => "lnk",
        }
    }
}

/// The desktop folder: `HORIZON_LAUNCHER_DESKTOP`, else the system's.
pub fn desktop_dir() -> Option<PathBuf> {
    std::env::var_os(DESKTOP_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::desktop_dir)
}

/// The shortcut's file name: the instance's name without the characters a file name can't
/// hold on any of the three systems.
pub fn file_name(name: &str, system: System) -> String {
    let clean: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let clean = clean.trim().trim_matches('.');
    let clean = if clean.is_empty() { "Instance" } else { clean };
    format!("{clean} (Horizon).{}", system.extension())
}

/// The id as a file name or bundle id part: letters, digits and `-`.
fn slug(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A Linux `.desktop` entry that plays the instance.
pub fn desktop_entry(exe: &Path, id: &str, name: &str) -> String {
    let field = |s: &str| s.replace(['\n', '\r'], " ");
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Comment=Play {name} in Horizon\n\
         Exec={} play --instance {}\n\
         TryExec={}\n\
         Icon=horizon\n\
         Terminal=false\n\
         Categories=Game;\n\
         StartupWMClass=horizon\n\
         {INSTANCE_KEY}={id}\n",
        desktop::quote(exe),
        desktop::quote(Path::new(id)),
        exe.display(),
        name = field(name),
        id = field(id),
    )
}

/// `'text'` for a POSIX shell.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

pub(crate) fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A macOS app's files, by path inside the `.app`, and whether each is a program.
pub fn mac_app(exe: &Path, id: &str, name: &str) -> Vec<(&'static str, Vec<u8>, bool)> {
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>play</string>
	<key>CFBundleIdentifier</key>
	<string>dev.horizon.instance.{}</string>
	<key>CFBundleName</key>
	<string>{}</string>
	<key>CFBundleIconFile</key>
	<string>horizon</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>LSUIElement</key>
	<true/>
	{MAC_KEY}
	<string>{}</string>
</dict>
</plist>
"#,
        slug(id),
        xml(name),
        xml(id),
    );
    let script = format!(
        "#!/bin/sh\n# Plays the Horizon instance {}.\nexec {} play --instance {}\n",
        name.replace('\n', " "),
        sh_quote(&exe.to_string_lossy()),
        sh_quote(id),
    );
    vec![
        ("Contents/Info.plist", plist.into_bytes(), false),
        ("Contents/MacOS/play", script.into_bytes(), true),
        ("Contents/Resources/horizon.icns", MAC_ICON.to_vec(), false),
    ]
}

/// The arguments of a Windows shortcut: the id in quotes, with its own quotes escaped.
fn windows_args(id: &str) -> String {
    format!("play --instance \"{}\"", id.replace('"', "\\\""))
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// A Windows Shell Link ([MS-SHLLINK]) to `exe` (a Windows path) with `args`: the header, a
/// LinkInfo with the local path (ANSI and Unicode), then the name, working folder, arguments and
/// icon as Unicode strings.
pub fn lnk(exe: &str, args: &str, comment: &str) -> Vec<u8> {
    const HAS_LINK_INFO: u32 = 0x02;
    const HAS_NAME: u32 = 0x04;
    const HAS_WORKING_DIR: u32 = 0x10;
    const HAS_ARGUMENTS: u32 = 0x20;
    const HAS_ICON_LOCATION: u32 = 0x40;
    const IS_UNICODE: u32 = 0x80;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const SW_SHOWNORMAL: u32 = 1;
    let flags =
        HAS_LINK_INFO | HAS_NAME | HAS_WORKING_DIR | HAS_ARGUMENTS | HAS_ICON_LOCATION | IS_UNICODE;
    let mut out = Vec::new();
    out.extend(0x4Cu32.to_le_bytes());
    // CLSID 00021401-0000-0000-C000-000000000046.
    out.extend([
        0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x46,
    ]);
    out.extend(flags.to_le_bytes());
    out.extend(FILE_ATTRIBUTE_NORMAL.to_le_bytes());
    out.extend([0; 24]); // creation, access and write times: unknown
    out.extend(0u32.to_le_bytes()); // file size
    out.extend(0i32.to_le_bytes()); // icon index
    out.extend(SW_SHOWNORMAL.to_le_bytes());
    out.extend([0; 2 + 2 + 4 + 4]); // hot key, reserved

    // LinkInfo: a header with the Unicode offsets (0x24 bytes), a VolumeID for a fixed drive
    // with no label, the path in ANSI (anything else as '?') and in UTF-16, and empty suffixes.
    let header = 0x24u32;
    let volume: Vec<u8> = [0x11u32, 3, 0, 0x10]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .chain([0])
        .collect();
    let ansi: Vec<u8> = exe
        .chars()
        .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
        .chain([0])
        .collect();
    let wide: Vec<u8> = utf16(exe).into_iter().chain([0, 0]).collect();
    let volume_at = header;
    let path_at = volume_at + volume.len() as u32;
    let suffix_at = path_at + ansi.len() as u32;
    let wide_at = suffix_at + 1;
    let wide_suffix_at = wide_at + wide.len() as u32;
    let size = wide_suffix_at + 2;
    for v in [
        size,
        header,
        1, // VolumeIDAndLocalBasePath
        volume_at,
        path_at,
        0, // no network path
        suffix_at,
        wide_at,
        wide_suffix_at,
    ] {
        out.extend(v.to_le_bytes());
    }
    out.extend(volume);
    out.extend(ansi);
    out.push(0);
    out.extend(wide);
    out.extend([0, 0]);

    let folder = exe.rsplit_once('\\').map_or("", |(dir, _)| dir);
    for text in [comment, folder, args, exe] {
        out.extend((text.encode_utf16().count() as u16).to_le_bytes());
        out.extend(utf16(text));
    }
    out.extend(0u32.to_le_bytes()); // no extra data
    out
}

/// Whether the shortcut at `path` plays the instance `id`.
fn plays(path: &Path, id: &str) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "desktop" => fs::read_to_string(path)
            .is_ok_and(|t| t.lines().any(|l| l == format!("{INSTANCE_KEY}={id}"))),
        "app" => fs::read_to_string(path.join("Contents/Info.plist"))
            .is_ok_and(|t| t.contains(&format!("{MAC_KEY}\n\t<string>{}</string>", xml(id)))),
        "lnk" => fs::read(path).is_ok_and(|b| {
            let want = utf16(&windows_args(id));
            b.windows(want.len()).any(|w| w == want)
        }),
        _ => false,
    }
}

/// Writes `bytes` beside `path` and renames it in, so nothing reads half a file.
fn write(path: &Path, bytes: &[u8], program: bool) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(fail)?;
    }
    let temp = path.with_extension("horizon-tmp");
    fs::write(&temp, bytes).map_err(fail)?;
    #[cfg(unix)]
    if program {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o755)).map_err(fail)?;
    }
    #[cfg(not(unix))]
    let _ = program;
    fs::rename(&temp, path).map_err(fail)
}

/// Moves a file or folder out of the way, where the player can get it back.
pub type Trash<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// Makes the shortcut for `system` in `desktop` (and, for Linux, the menu entry under
/// `menu`, an `applications` folder). The instance's shortcut of the same name is rewritten;
/// one under an older name goes to `trash`. Returns what it wrote.
pub fn make_in(
    system: System,
    desktop: &Path,
    menu: Option<&Path>,
    exe: &Path,
    id: &str,
    name: &str,
    trash: Trash,
) -> Result<Vec<PathBuf>, String> {
    let path = desktop.join(file_name(name, system));
    let item = menu.map(|m| m.join(format!("horizon-instance-{}.desktop", slug(id))));
    for old in find_in(desktop, menu, id) {
        if old != path && Some(&old) != item.as_ref() {
            trash(&old)?;
        }
    }
    if path.exists() && !plays(&path, id) {
        return Err(format!(
            "{} is already on the desktop, and isn't this instance's shortcut.",
            path.file_name().unwrap().to_string_lossy()
        ));
    }
    let mut made = vec![path.clone()];
    match system {
        System::Linux => {
            let entry = desktop_entry(exe, id, name);
            write(&path, entry.as_bytes(), true)?;
            if let Some(item) = item {
                write(&item, entry.as_bytes(), false)?;
                made.push(item);
            }
        }
        System::Mac => {
            for (inside, bytes, program) in mac_app(exe, id, name) {
                write(&path.join(inside), &bytes, program)?;
            }
        }
        System::Windows => write(
            &path,
            &lnk(
                &exe.to_string_lossy(),
                &windows_args(id),
                &format!("Play {name} in Horizon"),
            ),
            false,
        )?,
    }
    Ok(made)
}

/// The instance's shortcuts in `desktop` and the menu.
pub fn find_in(desktop: &Path, menu: Option<&Path>, id: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(desktop)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| plays(p, id))
        .collect();
    if let Some(item) = menu
        .map(|m| m.join(format!("horizon-instance-{}.desktop", slug(id))))
        .filter(|p| plays(p, id))
    {
        found.push(item);
    }
    found.sort();
    found
}

/// Moves the instance's shortcuts to `trash`. Returns what it moved.
pub fn remove_in(
    desktop: &Path,
    menu: Option<&Path>,
    id: &str,
    trash: Trash,
) -> Result<Vec<PathBuf>, String> {
    let found = find_in(desktop, menu, id);
    for path in &found {
        trash(path)?;
    }
    Ok(found)
}

fn menu() -> Option<PathBuf> {
    (System::here() == System::Linux)
        .then(desktop::data_home)
        .flatten()
        .map(|d| d.join("applications"))
}

fn desktop_or_err() -> Result<PathBuf, String> {
    desktop_dir().ok_or_else(|| "This computer has no desktop folder.".into())
}

/// Makes the shortcut on this computer's desktop that plays `id`, starting `exe`.
pub fn make(exe: &Path, id: &str, name: &str) -> Result<Vec<PathBuf>, String> {
    let desktop = desktop_or_err()?;
    let made = make_in(
        System::here(),
        &desktop,
        menu().as_deref(),
        &desktop::target(exe),
        id,
        name,
        &desktop_seams::to_trash,
    )?;
    if System::here() == System::Linux {
        // GNOME starts a desktop file only once it is trusted; elsewhere this does nothing.
        let _ = std::process::Command::new("gio")
            .args(["set", "-t", "string"])
            .arg(&made[0])
            .args(["metadata::trusted", "true"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok(made)
}

/// The instance's shortcuts on this computer.
pub fn find(id: &str) -> Vec<PathBuf> {
    desktop_dir()
        .map(|d| find_in(&d, menu().as_deref(), id))
        .unwrap_or_default()
}

/// Moves the instance's shortcuts on this computer to the trash.
pub fn remove(id: &str) -> Result<Vec<PathBuf>, String> {
    remove_in(
        &desktop_or_err()?,
        menu().as_deref(),
        id,
        &desktop_seams::to_trash,
    )
}

#[cfg(test)]
#[path = "shortcut/tests.rs"]
mod tests;
