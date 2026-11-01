//! Who is playing now, from the games' "I'm playing" pings (`horizon_telemetry::live`). Kept in
//! memory only: a session ID, when it last pinged, its version, platform and LAN flag. A
//! session counts while its last ping is under the window old and is forgotten after it. No
//! address is kept, and nothing is written to disk, so a restart starts the count again (it
//! is whole again a minute later).
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use horizon_telemetry::live::{Count, PLATFORMS, Ping};

/// Versions listed by name in a count; the rest are summed under `other`.
const VERSIONS_SHOWN: usize = 12;

struct Seen {
    at: Instant,
    version: Box<str>,
    platform: u8,
    lan: bool,
}

pub struct Live {
    sessions: HashMap<u128, Seen>,
    window: Duration,
    max_sessions: usize,
}

/// The table is full and this session is new.
#[derive(Debug, PartialEq, Eq)]
pub struct Full;

impl Live {
    pub fn new(window: Duration, max_sessions: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            window,
            max_sessions,
        }
    }

    /// Records `ping` (already checked) as seen at `now`.
    pub fn ping(&mut self, ping: &Ping, now: Instant) -> Result<(), Full> {
        let Ok(id) = u128::from_str_radix(&ping.session, 16) else {
            return Ok(());
        };
        if !self.sessions.contains_key(&id) && self.sessions.len() >= self.max_sessions {
            self.forget_expired(now);
            if self.sessions.len() >= self.max_sessions {
                return Err(Full);
            }
        }
        let platform = PLATFORMS
            .iter()
            .position(|p| *p == ping.platform)
            .unwrap_or(PLATFORMS.len() - 1) as u8;
        self.sessions.insert(
            id,
            Seen {
                at: now,
                version: ping.version.as_str().into(),
                platform,
                lan: ping.lan,
            },
        );
        Ok(())
    }

    /// Drops the sessions whose last ping is older than the window.
    pub fn forget_expired(&mut self, now: Instant) {
        let window = self.window;
        self.sessions
            .retain(|_, s| now.saturating_duration_since(s.at) < window);
    }

    /// Who is playing at `now`, exactly.
    pub fn count(&mut self, now: Instant) -> Count {
        self.forget_expired(now);
        let mut versions: HashMap<&str, u64> = HashMap::new();
        let mut platforms = [0u64; PLATFORMS.len()];
        let mut lan = 0;
        for s in self.sessions.values() {
            *versions.entry(&s.version).or_default() += 1;
            platforms[usize::from(s.platform)] += 1;
            lan += u64::from(s.lan);
        }
        let mut by_count: Vec<(&str, u64)> = versions.into_iter().collect();
        by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let mut shown = BTreeMap::new();
        let mut other = 0;
        for (i, (version, n)) in by_count.into_iter().enumerate() {
            if i < VERSIONS_SHOWN {
                shown.insert(version.to_owned(), n);
            } else {
                other += n;
            }
        }
        if other > 0 {
            *shown.entry("other".to_owned()).or_default() += other;
        }
        Count {
            playing: self.sessions.len() as u64,
            lan,
            versions: shown,
            platforms: PLATFORMS
                .iter()
                .zip(platforms)
                .filter(|(_, n)| *n > 0)
                .map(|(p, n)| ((*p).to_owned(), n))
                .collect(),
        }
    }

    /// Sessions held now, counted or not yet forgotten.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ping(n: u32, version: &str, platform: &str, lan: bool) -> Ping {
        Ping {
            session: format!("{n:032x}"),
            version: version.into(),
            platform: platform.into(),
            lan,
        }
    }

    #[test]
    fn sessions_count_until_the_window_passes() {
        let t = Instant::now();
        let mut live = Live::new(Duration::from_secs(120), 100);
        live.ping(&ping(1, "0.2.0", "linux", false), t).unwrap();
        live.ping(&ping(2, "0.2.0", "macos", true), t).unwrap();
        // The same session again is still one.
        live.ping(
            &ping(1, "0.2.0", "linux", false),
            t + Duration::from_secs(60),
        )
        .unwrap();
        let count = live.count(t + Duration::from_secs(61));
        assert_eq!((count.playing, count.lan), (2, 1));
        assert_eq!(count.versions, BTreeMap::from([("0.2.0".into(), 2)]));
        assert_eq!(
            count.platforms,
            BTreeMap::from([("linux".into(), 1), ("macos".into(), 1)])
        );
        // Session 2 stopped pinging two minutes ago; session 1 pinged a minute later.
        let count = live.count(t + Duration::from_secs(150));
        assert_eq!((count.playing, count.lan), (1, 0));
        assert_eq!(live.len(), 1);
        assert_eq!(live.count(t + Duration::from_secs(181)), Count::default());
        assert!(live.is_empty());
    }

    #[test]
    fn a_full_table_takes_no_new_session_until_one_expires() {
        let t = Instant::now();
        let mut live = Live::new(Duration::from_secs(120), 2);
        live.ping(&ping(1, "0.2.0", "linux", false), t).unwrap();
        live.ping(&ping(2, "0.2.0", "linux", false), t).unwrap();
        assert_eq!(live.ping(&ping(3, "0.2.0", "linux", false), t), Err(Full));
        // A known session still refreshes.
        live.ping(
            &ping(1, "0.2.0", "linux", false),
            t + Duration::from_secs(100),
        )
        .unwrap();
        let later = t + Duration::from_secs(130);
        live.ping(&ping(3, "0.2.0", "linux", false), later).unwrap();
        assert_eq!(live.count(later).playing, 2);
    }

    #[test]
    fn rare_versions_are_summed_and_counts_exact() {
        let t = Instant::now();
        let mut live = Live::new(Duration::from_secs(120), 1000);
        for n in 0..41 {
            live.ping(&ping(n, "0.2.0", "windows", n % 4 == 0), t)
                .unwrap();
        }
        for n in 0..(VERSIONS_SHOWN as u32 + 3) {
            live.ping(&ping(100 + n, &format!("dev {n}"), "other", false), t)
                .unwrap();
        }
        let count = live.count(t);
        // Exact to the unit: 41 and 56 stay 41 and 56.
        assert_eq!(count.playing, 56);
        assert_eq!(count.lan, 11);
        assert_eq!(count.versions["0.2.0"], 41);
        assert_eq!(count.versions["other"], 4);
        assert_eq!(count.versions.len(), VERSIONS_SHOWN + 1);
        assert_eq!(count.platforms["windows"], 41);
        assert_eq!(count.platforms["other"], 15);
    }
}
