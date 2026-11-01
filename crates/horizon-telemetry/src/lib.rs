//! Opt-in data collection, shared by the launcher, the game and the ingest server.
//!
//! Nothing is collected or sent until the player says yes, kind by kind ([`Consent`]): the
//! game and the launcher only [`outbox::push`] items whose kind is allowed, and the launcher's
//! [`upload::flush`] drops anything the player has since turned off before it opens a
//! connection. Items are small summaries (percentiles and histogram buckets, never raw streams),
//! dated to the day, with no account, world, server or address in them ([`scrub`]); they leave
//! in zstd batches ([`batch`]). `docs/telemetry.md` lists every field.
//!
//! The one exception is [`live`]: the anonymous "I'm playing" ping the game sends while it
//! runs, for the live player count. It holds no install ID and nothing about the player.
//! [`server_status`] asks a Minecraft server how many play on it (the official server's count).
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod batch;
pub mod consent;
pub mod live;
pub mod outbox;
pub mod payload;
pub mod scrub;
pub mod server_status;
pub mod system;
#[cfg(feature = "upload")]
pub mod upload;

pub use consent::{Choices, Consent};
pub use payload::{Item, Kind, Payload, Source};

/// The payload format. New optional fields keep it; a field that changes meaning or goes
/// bumps it, and the server keeps reading the older ones.
pub const SCHEMA: u32 = 1;

/// What players agree to. Bumped when what is collected changes: the launcher then asks again,
/// and until the player answers nothing new is kept or sent. 2: the machine's full hardware
/// (cores, clocks, VRAM, drive, screen, power, form), 2026-10-23.
pub const CONSENT_VERSION: u32 = 2;

/// The ingest server (`crates/horizon-telemetry-server`, on Railway). `HORIZON_TELEMETRY_URL`
/// overrides it (a local server); `HORIZON_TELEMETRY_URL=off` sends nothing at all, no ping
/// either (tests and automated runs). What is sent still depends on the player's switches.
pub const DEFAULT_ENDPOINT: &str = "https://telemetry.playhorizon.dev";

/// Where items are sent, if anywhere.
pub fn endpoint() -> Option<String> {
    resolve(std::env::var("HORIZON_TELEMETRY_URL").ok().as_deref())
}

fn resolve(var: Option<&str>) -> Option<String> {
    match var.map(str::trim) {
        Some("off" | "none" | "0") => None,
        Some(url) if !url.is_empty() => Some(url.trim_end_matches('/').to_owned()),
        _ => Some(DEFAULT_ENDPOINT.to_owned()),
    }
}

/// `telemetry/` in the game folder: the consent, the outbox and the upload state, read by the
/// launcher and the game alike.
pub fn dir(home: &Path) -> PathBuf {
    home.join("telemetry")
}

/// Days since 1970-01-01 (UTC): the only time an item carries.
pub fn today() -> u32 {
    (now_secs() / 86_400) as u32
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `bytes` random bytes in lowercase hex.
pub(crate) fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    // Without the OS's randomness the time still makes names that don't collide in practice.
    if getrandom::fill(&mut buf).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (nanos >> (8 * (i % 16))) as u8;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `bytes` through a temporary file, so a reader never sees half of it.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", random_hex(4)));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
pub(crate) mod test_home {
    use std::path::PathBuf;

    /// An empty folder of its own under the system's temporary folder.
    pub fn new(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-telemetry-{name}-{}", super::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_defaults_to_the_server_and_can_be_moved_or_turned_off() {
        assert_eq!(resolve(None).as_deref(), Some(DEFAULT_ENDPOINT));
        assert_eq!(resolve(Some(" ")).as_deref(), Some(DEFAULT_ENDPOINT));
        assert_eq!(
            resolve(Some("http://127.0.0.1:8080/")).as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(resolve(Some("off")), None);
        assert!(DEFAULT_ENDPOINT.starts_with("https://") && !DEFAULT_ENDPOINT.ends_with('/'));
    }
}
