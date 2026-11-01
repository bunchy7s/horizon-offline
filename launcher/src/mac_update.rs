//! Retired app bundles must outlive every process using them. Removing a running macOS
//! executable breaks Keychain credential creation (OSStatus 100002), even though it keeps running.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BINARY: &str = "horizon-launcher";

fn signature_holds(app: &Path) -> bool {
    Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// lsof returns 1 with no output when there are no users of the file. Any warning, failure,
/// signal, or unexpected output means we cannot safely remove it. In particular, the updater's
/// PID alone is insufficient: other windows and MCP processes can run from the same old bundle.
fn definitely_unused(output: io::Result<Output>) -> bool {
    output.is_ok_and(|out| {
        out.status.code() == Some(1) && out.stdout.is_empty() && out.stderr.is_empty()
    })
}

fn unused_by_processes(exe: &Path) -> bool {
    definitely_unused(
        Command::new("/usr/sbin/lsof")
            // -t also suppresses warnings, which could turn an incomplete query into an
            // apparently empty result. Keep warnings enabled and request only PID fields.
            .args(["-nP", "+w", "-F", "p", "--"])
            .arg(exe)
            .output(),
    )
}

fn is_retired(name: &str, app_name: &str) -> bool {
    let prefix = format!(".{app_name}.old-");
    let Some((pid, nonce)) = name.strip_prefix(&prefix).and_then(|s| s.split_once('-')) else {
        return false;
    };
    pid.parse::<u32>()
        .is_ok_and(|number| number != 0 && number.to_string() == pid)
        && nonce.len() == 32
        && nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Only consider a real bundle's executable, never a symlink or an incomplete/unknown folder.
fn retired_executable(app: &Path) -> Option<PathBuf> {
    let mut path = app.to_path_buf();
    for component in ["", "Contents", "MacOS", BINARY] {
        if !component.is_empty() {
            path.push(component);
        }
        let kind = fs::symlink_metadata(&path).ok()?.file_type();
        if kind.is_symlink()
            || (component == BINARY && !kind.is_file())
            || (component != BINARY && !kind.is_dir())
        {
            return None;
        }
    }
    Some(path)
}

/// Best effort: run on startup and after an update. Failed queries and inaccessible files stay
/// for a later start. A restart via exec releases the old mapping before the new process cleans.
pub(crate) fn cleanup(app: &Path) {
    cleanup_with(app, unused_by_processes);
}

fn cleanup_with(app: &Path, mut unused: impl FnMut(&Path) -> bool) {
    let (Some(dir), Some(name)) = (app.parent(), app.file_name().and_then(|s| s.to_str())) else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_str()
            .is_some_and(|s| is_retired(s, name))
        {
            continue;
        }
        let path = entry.path();
        if retired_executable(&path).is_some_and(|exe| unused(&exe)) {
            let _ = fs::remove_dir_all(path);
        }
    }
}

pub(crate) fn replace_bundle(new: &Path, app: &Path) -> Result<(), String> {
    replace_bundle_with(new, app, signature_holds, unused_by_processes)
}

fn replace_bundle_with(
    new: &Path,
    app: &Path,
    mut signature: impl FnMut(&Path) -> bool,
    unused: impl FnMut(&Path) -> bool,
) -> Result<(), String> {
    let dir = app.parent().ok_or("The app has no folder.")?;
    let name = app.file_name().unwrap_or_default().to_string_lossy();
    // A single process may update more than once, with older generations still in use.
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let nonce: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let suffix = format!("{}-{nonce}", std::process::id());
    let staged = dir.join(format!(".{name}.update-{suffix}"));
    let rollback = dir.join(format!(".{name}.rollback-{suffix}"));
    let old = dir.join(format!(".{name}.old-{suffix}"));
    fs::create_dir(&staged).map_err(|e| format!("Could not stage the new app: {e}"))?;
    let copied = Command::new("/usr/bin/ditto")
        .arg(new)
        .arg(&staged)
        .status();
    if !copied.is_ok_and(|s| s.success()) {
        let _ = fs::remove_dir_all(&staged);
        return Err(format!(
            "Could not copy the new app next to {}.",
            app.display()
        ));
    }
    if !signature(&staged) {
        let _ = fs::remove_dir_all(&staged);
        return Err(
            "The downloaded app's signature doesn't hold; the launcher stays as it is.".into(),
        );
    }
    // Startup cleanup in another process must not collect an idle backup while this
    // transaction can still need it. Publish it as retired only after the final check.
    fs::rename(app, &rollback).map_err(|e| {
        let _ = fs::remove_dir_all(&staged);
        format!("Could not move {} aside: {e}", app.display())
    })?;
    if let Err(e) = fs::rename(&staged, app) {
        let _ = fs::rename(&rollback, app);
        let _ = fs::remove_dir_all(&staged);
        return Err(format!(
            "Could not put the new app at {}: {e}",
            app.display()
        ));
    }
    if !signature(app) {
        let failed = dir.join(format!(".{name}.failed-{suffix}"));
        if fs::rename(app, &failed).is_ok() && fs::rename(&rollback, app).is_ok() {
            let _ = fs::remove_dir_all(&failed);
        }
        return Err(format!(
            "The new app's signature doesn't hold at {}; the launcher stays as it was.",
            app.display()
        ));
    }
    // Failure to publish the now-retired bundle only leaves a conservative quarantine;
    // the new installation is verified and the running old executable remains available.
    let _ = fs::rename(&rollback, &old);
    cleanup_with(app, unused);
    Ok(())
}

#[cfg(test)]
mod tests;
