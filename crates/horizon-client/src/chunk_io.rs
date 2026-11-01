//! `IntegratedServer.reportChunkLoadFailure` / `reportChunkSaveFailure` /
//! `sendLowDiskSpaceWarning`: the region storage runs on the server thread and
//! its writer thread, and the toasts are the window's. Each failure is logged,
//! checked for low disk space (`MinecraftServer.warnOnLowDiskSpace`) and queued
//! here; the window drains the queue every frame (`Minecraft.execute`).
//! `SystemToast.addOrUpdate` makes a repeat reset the toast it already shows,
//! so only the newest chunk of each kind is kept.
use crate::{lang, save, toasts::SystemToast};
use std::{path::Path, sync::Mutex};

/// What the window has still to toast.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pending {
    /// `SystemToast.onLowDiskSpace`.
    pub low_disk_space: bool,
    /// `SystemToast.onChunkLoadFailure`: the newest chunk.
    pub load_failure: Option<(i32, i32)>,
    /// `SystemToast.onChunkSaveFailure`: the newest chunk.
    pub save_failure: Option<(i32, i32)>,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    low_disk_space: false,
    load_failure: None,
    save_failure: None,
});

impl Pending {
    /// `reportChunkLoadFailure` after `warnOnLowDiskSpace` found `low_disk_space`.
    fn load_failed(&mut self, x: i32, z: i32, low_disk_space: bool) {
        self.low_disk_space |= low_disk_space;
        self.load_failure = Some((x, z));
    }

    /// `reportChunkSaveFailure` after `warnOnLowDiskSpace` found `low_disk_space`.
    fn save_failed(&mut self, x: i32, z: i32, low_disk_space: bool) {
        self.low_disk_space |= low_disk_space;
        self.save_failure = Some((x, z));
    }
}

fn with(update: impl FnOnce(&mut Pending)) {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    update(&mut pending);
}

/// `MinecraftServer.warnOnLowDiskSpace` for the world folder `dir`.
fn warn_on_low_disk_space(dir: &Path) -> bool {
    let low = save::low_disk_space(dir);
    if low {
        // `MinecraftServer.sendLowDiskSpaceWarning`.
        log::warn!("Low disk space! Might not be able to save the world.");
    }
    low
}

/// `reportChunkLoadFailure` for the chunk at (`x`, `z`) of the dimension in `dir`.
pub fn report_load_failure(dir: &Path, x: i32, z: i32, error: &dyn std::fmt::Display) {
    log::error!("Failed to load chunk {x},{z}: {error}");
    let low = warn_on_low_disk_space(dir);
    with(|p| p.load_failed(x, z, low));
}

/// `reportChunkSaveFailure`.
pub fn report_save_failure(dir: &Path, x: i32, z: i32, error: &dyn std::fmt::Display) {
    log::error!("Failed to save chunk {x},{z}: {error}");
    let low = warn_on_low_disk_space(dir);
    with(|p| p.save_failed(x, z, low));
}

/// `Minecraft.sendLowDiskSpaceWarning`: a server's `ClientboundLowDiskSpaceWarningPacket`
/// (`ClientPacketListener.handleLowDiskSpaceWarning`) toasts like the local report.
pub fn report_low_disk_space() {
    with(|p| p.low_disk_space = true);
}

/// The reports since the last call.
pub fn take() -> Pending {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *pending)
}

/// `ChatFormatting.RED`.
fn red() -> u32 {
    crate::hud::rgba(255, 85, 85, 255)
}

/// `SystemToast.onLowDiskSpace` (`SystemToastId.LOW_DISK_SPACE`: ten seconds).
pub fn low_disk_space_toast() -> SystemToast {
    SystemToast::new(
        lang::tr("chunk.toast.lowDiskSpace", &[]),
        Some(lang::tr("chunk.toast.lowDiskSpace.description", &[])),
    )
    .with_display_time(SystemToast::LONG_DISPLAY_TIME)
}

fn chunk_toast(key: &str, x: i32, z: i32) -> SystemToast {
    // `Component.translationArg(ChunkPos)` is `ChunkPos.toString`.
    let pos = format!("[{x}, {z}]");
    SystemToast::new(
        lang::tr(key, &[&pos]),
        Some(lang::tr("chunk.toast.checkLog", &[])),
    )
    .with_title_color(red())
}

/// `SystemToast.onChunkLoadFailure`.
pub fn load_failure_toast(x: i32, z: i32) -> SystemToast {
    chunk_toast("chunk.toast.loadFailure", x, z)
}

/// `SystemToast.onChunkSaveFailure`.
pub fn save_failure_toast(x: i32, z: i32) -> SystemToast {
    chunk_toast("chunk.toast.saveFailure", x, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests keep off `PENDING` where they can: the save tests run in
    // the same process and report their own chunk failures into it (#4854).
    #[test]
    fn failures_keep_the_newest_chunk_of_each_kind() {
        let mut pending = Pending::default();
        pending.load_failed(1, 2, false);
        pending.load_failed(3, 4, false);
        pending.save_failed(5, 6, false);
        assert_eq!(
            pending,
            Pending {
                low_disk_space: false,
                load_failure: Some((3, 4)),
                save_failure: Some((5, 6)),
            }
        );
        // A later failure on a full disk raises the warning, and one with
        // space left does not lower it.
        pending.save_failed(7, 8, true);
        pending.load_failed(9, 10, false);
        assert_eq!(
            pending,
            Pending {
                low_disk_space: true,
                load_failure: Some((9, 10)),
                save_failure: Some((7, 8)),
            }
        );
    }

    /// The only test that drains `PENDING`; other tests only add to it, so
    /// the flag it raises is still there when it takes it.
    #[test]
    fn a_servers_warning_packet_toasts_low_disk_space() {
        report_low_disk_space();
        assert!(take().low_disk_space);
    }

    #[test]
    fn low_disk_space_toast_stays_ten_seconds() {
        let toast = low_disk_space_toast();
        assert_eq!(toast.display_time(), 10000.0);
        assert_eq!(load_failure_toast(0, 0).display_time(), 5000.0);
    }
}
