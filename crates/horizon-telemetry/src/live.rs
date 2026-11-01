//! The anonymous "I'm playing" ping, and the live count it feeds ("N playing now" in the
//! launcher).
//!
//! Unlike everything else in this crate it does not wait for consent: Ludovic decided
//! (2026-10-22) to count every player and tell them, in the launcher's first-run question, its
//! Settings and the privacy page. So a ping holds only what the count needs: a random session ID
//! made when the game starts and kept only in its memory (not the install ID), the Horizon
//! version, the platform and whether a LAN or play-with-friends world is open. The game sends
//! one every [`INTERVAL`]; the server counts a session while its last ping is under
//! [`WINDOW`] old and serves the counts exactly. It writes down only the totals, once a minute,
//! for the history on playhorizon.dev/stats: never a session.
use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How often the game pings while it runs.
pub const INTERVAL: Duration = Duration::from_secs(60);

/// How long a session counts after its last ping: two pings, so one lost on the way doesn't
/// drop it.
pub const WINDOW: Duration = Duration::from_secs(120);

/// The platforms a ping may name; anything else is `other`.
pub const PLATFORMS: [&str; 4] = ["linux", "macos", "windows", "other"];

/// The largest ping body the server reads.
pub const MAX_PING_BYTES: usize = 512;

const MAX_VERSION: usize = 40;

/// One "I'm playing", as it is sent: `{"session":"…","version":"…","platform":"…","lan":false}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ping {
    /// 128 random bits in hex, made when the game starts ([`session_id`]) and never written down.
    pub session: String,
    /// The release (`v2026.10.15`, `0.2.0`) or `dev 0.1.0` for a build from source.
    pub version: String,
    /// One of [`PLATFORMS`].
    pub platform: String,
    /// A world is open to LAN or to friends over the internet.
    pub lan: bool,
}

impl Ping {
    pub fn new(session: &str, version: &str, lan: bool) -> Self {
        Self {
            session: session.to_owned(),
            version: version.to_owned(),
            platform: platform().to_owned(),
            lan,
        }
    }

    /// Why the server refuses this ping, if it does.
    pub fn check(&self) -> Result<(), &'static str> {
        if !crate::consent::is_install_id(&self.session) {
            return Err("session: 32 lowercase hex digits");
        }
        let version_ok = !self.version.is_empty()
            && self.version.len() <= MAX_VERSION
            && self
                .version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b" .+-_".contains(&b));
        if !version_ok {
            return Err("version: at most 40 letters, digits, spaces and .+-_");
        }
        if !PLATFORMS.contains(&self.platform.as_str()) {
            return Err("platform: linux, macos, windows or other");
        }
        Ok(())
    }
}

/// A new session ID: 128 random bits in hex.
pub fn session_id() -> String {
    crate::random_hex(16)
}

/// This machine's entry in [`PLATFORMS`].
pub fn platform() -> &'static str {
    match std::env::consts::OS {
        os @ ("linux" | "macos" | "windows") => os,
        _ => "other",
    }
}

/// The live count, as `GET /v1/live` serves it: exact numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Count {
    /// Sessions that pinged within the window.
    pub playing: u64,
    /// Of those, the ones with a LAN or play-with-friends world open.
    pub lan: u64,
    /// Per version: the most played ones, the rest summed under `other`.
    pub versions: BTreeMap<String, u64>,
    /// Per platform.
    pub platforms: BTreeMap<String, u64>,
}

/// Sends `ping` to `endpoint` (`https://host`, without a trailing slash).
#[cfg(feature = "live")]
pub fn send(endpoint: &str, ping: &Ping) -> Result<(), String> {
    agent()
        .post(&format!("{endpoint}/v1/live"))
        .set("Content-Type", "application/json")
        .send_string(&serde_json::to_string(ping).map_err(|e| e.to_string())?)
        .map(drop)
        .map_err(|e| e.to_string())
}

/// The live count at `endpoint`.
#[cfg(feature = "live")]
pub fn fetch(endpoint: &str) -> Result<Count, String> {
    let body = agent()
        .get(&format!("{endpoint}/v1/live"))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

#[cfg(feature = "live")]
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ping() -> Ping {
        Ping::new(&session_id(), "v2026.10.15", false)
    }

    #[test]
    fn a_ping_holds_only_the_four_fields() {
        let json = serde_json::to_value(Ping::new(&"a".repeat(32), "0.2.0", true)).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["lan", "platform", "session", "version"]);
        assert!(serde_json::to_string(&ping()).unwrap().len() < MAX_PING_BYTES);
    }

    #[test]
    fn session_ids_are_fresh_each_time() {
        let (a, b) = (session_id(), session_id());
        assert_ne!(a, b);
        assert!(ping().check().is_ok());
    }

    #[test]
    fn bad_pings_are_refused() {
        let bad = |f: fn(&mut Ping)| {
            let mut p = ping();
            f(&mut p);
            p.check().is_err()
        };
        assert!(bad(|p| p.session = "ABC".into()));
        assert!(bad(|p| p.session = "G".repeat(32)));
        assert!(bad(|p| p.version = String::new()));
        assert!(bad(|p| p.version = "1".repeat(41)));
        assert!(bad(|p| p.version = "<script>".into()));
        assert!(bad(|p| p.platform = "haiku".into()));
        assert!(!bad(|p| p.version = "dev 0.1.0".into()));
        let extra = r#"{"session":"00000000000000000000000000000000","version":"0.2.0","platform":"linux","lan":false,"name":"Steve"}"#;
        assert!(serde_json::from_str::<Ping>(extra).is_err());
    }
}
