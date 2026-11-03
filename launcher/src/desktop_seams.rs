//! The two things the launcher does to the player's desktop that a test run must never do to the
//! real one: moving files to the system's trash, and writing to the clipboard.
//!
//! - `HORIZON_LAUNCHER_TRASH=<folder>`: what would go to the trash is moved into that folder
//!   instead (under a free name).
//! - `HORIZON_LAUNCHER_CLIPBOARD=<file>`: what would be copied is written to that file instead
//!   (text as UTF-8, a picture as its PNG or JPEG bytes).
//!
//! Test runs point both into their scratch folders, beside `HORIZON_HOME` and
//! `HORIZON_LAUNCHER_HOME`, and `HORIZON_LAUNCHER_DESKTOP` too (`shortcut`: where desktop
//! shortcuts go).
use std::path::{Path, PathBuf};

pub const TRASH_ENV: &str = "HORIZON_LAUNCHER_TRASH";
pub const CLIPBOARD_ENV: &str = "HORIZON_LAUNCHER_CLIPBOARD";

/// Moves a file or folder to the system's trash, where the player can still get it back, or
/// into `HORIZON_LAUNCHER_TRASH` when it is set.
pub fn to_trash(path: &Path) -> Result<(), String> {
    match std::env::var_os(TRASH_ENV).filter(|v| !v.is_empty()) {
        Some(dir) => to_folder(path, Path::new(&dir)),
        None => system_trash(path),
    }
}

/// Moves `path` into `dir` under a free name: the trash of a test run.
pub fn to_folder(path: &Path, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = path.file_name().ok_or("Nothing to move")?.to_string_lossy();
    let target = (1..)
        .map(|n| match n {
            1 => dir.join(&*name),
            n => dir.join(format!("{name} {n}")),
        })
        .find(|p| !p.exists())
        .unwrap();
    std::fs::rename(path, &target).map_err(|e| e.to_string())
}

fn system_trash(path: &Path) -> Result<(), String> {
    // On Windows the crate starts COM on the calling thread and panics when that thread already
    // runs it in the other mode (RPC_E_CHANGED_MODE). A worker of GPUI's thread pool can, and a
    // panic there aborts the launcher (#3784); a thread of its own starts without COM.
    on_own_thread(|| {
        #[allow(unused_mut)]
        let mut trash = trash::TrashContext::default();
        // The crate's default on macOS asks Finder through AppleScript, which needs the player
        // to allow the launcher to control Finder first; the file manager call needs nothing.
        #[cfg(target_os = "macos")]
        trash::macos::TrashContextExtMacos::set_delete_method(
            &mut trash,
            trash::macos::DeleteMethod::NsFileManager,
        );
        trash.delete(path).map_err(|e| e.to_string())
    })
}

/// Runs `f` on a new thread and waits for it; a panic there comes back as an error.
fn on_own_thread<T: Send>(f: impl FnOnce() -> Result<T, String> + Send) -> Result<T, String> {
    std::thread::scope(|s| s.spawn(f).join()).unwrap_or_else(|panic| {
        let why = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "it panicked".into());
        Err(why)
    })
}

/// The file copies go to instead of the clipboard, when `HORIZON_LAUNCHER_CLIPBOARD` is set.
pub fn clipboard_file() -> Option<PathBuf> {
    std::env::var_os(CLIPBOARD_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trash_folder_takes_what_would_go_to_the_trash() {
        let root = std::env::temp_dir().join(format!("horizon-seams-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, bin) = (root.join("home"), root.join("trash"));
        std::fs::create_dir_all(home.join("World/region")).unwrap();
        std::fs::write(home.join("World/level.dat"), b"x").unwrap();
        std::fs::write(home.join("shot.png"), b"1").unwrap();
        to_folder(&home.join("World"), &bin).unwrap();
        to_folder(&home.join("shot.png"), &bin).unwrap();
        std::fs::write(home.join("shot.png"), b"2").unwrap();
        to_folder(&home.join("shot.png"), &bin).unwrap();
        assert!(bin.join("World/level.dat").is_file());
        assert_eq!(std::fs::read(bin.join("shot.png")).unwrap(), b"1");
        assert_eq!(std::fs::read(bin.join("shot.png 2")).unwrap(), b"2");
        assert!(std::fs::read_dir(&home).unwrap().next().is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_trash_runs_on_a_thread_of_its_own_and_its_panic_is_an_error() {
        let caller = std::thread::current().id();
        assert_eq!(
            on_own_thread(|| Ok(std::thread::current().id() != caller)),
            Ok(true)
        );
        let failed: Result<(), String> = on_own_thread(|| panic!("Call to CoInitializeEx failed"));
        assert_eq!(failed, Err("Call to CoInitializeEx failed".into()));
    }
}
