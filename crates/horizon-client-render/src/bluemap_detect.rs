//! Finding a server's BlueMap without the player naming it
//! (`blueMapAutoDetect`, off by default): the BlueMap worker tries a few
//! likely web addresses (`candidates`), taking only one whose `settings.json`
//! is BlueMap's web-app settings, and the answer is kept per server in
//! `server-terrain/<server>/BlueMap/detected.txt` so a rejoin asks nothing.
//! A `blueMap:` line the player wrote always wins.
use crate::bluemap::Settings;
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

/// The answer kept per server, beside its BlueMap tiles.
pub const FILE: &str = "detected.txt";
/// A server found without a map is asked again after this long.
pub const RETRY_NONE: Duration = Duration::from_secs(24 * 3600);
/// A map found is checked again (its address first) after this long.
pub const RETRY_FOUND: Duration = Duration::from_secs(7 * 24 * 3600);
/// Each probe gets this long, connecting included.
pub const TIMEOUT: Duration = Duration::from_secs(4);
/// Probes start this far apart at least.
pub const GAP: Duration = Duration::from_millis(500);

/// Where a level's BlueMap comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The player's `blueMap:` line, or a map found before.
    Url {
        url: String,
        maps: Vec<(String, String)>,
    },
    /// To be looked for at these addresses, in order.
    Detect(Vec<String>),
}

/// What the server at `address` draws far terrain from, its BlueMap folder
/// being `dir`: `None` when the player named no map and either auto-detect
/// is off or the server was found without one recently.
pub fn source(settings: &Settings, address: &str, dir: &Path, now: SystemTime) -> Option<Source> {
    if let Some(server) = settings.server(address) {
        return Some(Source::Url {
            url: server.url.clone(),
            maps: server.maps.clone(),
        });
    }
    if !settings.auto_detect {
        return None;
    }
    let mut probes = Vec::new();
    // Only an address this server's would be looked for at: a remembered
    // one from elsewhere (a copied folder) names someone else's.
    let ours = candidates(address);
    match remembered(dir, now) {
        Remembered::Found(url) | Remembered::Stale(Some(url)) if !ours.contains(&url) => {}
        Remembered::Found(url) => {
            return Some(Source::Url {
                url,
                maps: Vec::new(),
            });
        }
        Remembered::Nothing => return None,
        Remembered::Stale(Some(url)) => probes.push(url),
        Remembered::Stale(None) => {}
    }
    for candidate in ours {
        if !probes.contains(&candidate) {
            probes.push(candidate);
        }
    }
    (!probes.is_empty()).then_some(Source::Detect(probes))
}

/// The first labels a game server's host name takes beside its site's.
const GAME_LABELS: &[&str] = &["play", "mc", "minecraft", "mine", "join", "server"];

/// Domains whose names belong to many customers (dynamic DNS, server
/// hosts): `map.` of them is someone else's.
const SHARED: &[&str] = &[
    "aternos.me",
    "ddns.net",
    "duckdns.org",
    "hopto.org",
    "minehut.gg",
    "no-ip.org",
    "playit.gg",
    "serveminecraft.net",
    "zapto.org",
];

/// The web addresses a server's BlueMap likely answers at: BlueMap's own
/// web server (port 8100), the server's host, and its `map.` subdomain, also
/// under the parent domain of a `play.`/`mc.` style host (`GAME_LABELS`).
pub fn candidates(address: &str) -> Vec<String> {
    let address = address.trim().to_ascii_lowercase();
    let host = if let Some(rest) = address.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if address.matches(':').count() > 1 {
        &address[..]
    } else {
        address.split(':').next().unwrap_or(&address)
    };
    let host = host.trim_end_matches('.');
    if host.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Ok(ip) = host.parse::<IpAddr>() {
        // Certificates are for names: plain HTTP only.
        let host = match ip {
            IpAddr::V6(_) => format!("[{host}]"),
            IpAddr::V4(_) => host.to_owned(),
        };
        out.push(format!("http://{host}:8100"));
        out.push(format!("http://{host}"));
        return out;
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Vec::new();
    }
    out.push(format!("http://{host}:8100"));
    let labels = host.split('.').count();
    let map = |host: &str| (!host.starts_with("map.")).then(|| format!("map.{host}"));
    if labels >= 2
        && let Some(map) = map(host)
    {
        out.push(format!("https://{map}"));
    }
    out.push(format!("https://{host}"));
    if labels >= 2
        && let Some(map) = map(host)
    {
        out.push(format!("http://{map}"));
    }
    out.push(format!("http://{host}"));
    // `play.example.net` → `map.example.net`, only for a game server's
    // usual names: the parent of `name.hosting.example` is another party's.
    if labels >= 3
        && let Some((first, parent)) = host.split_once('.')
        && GAME_LABELS.contains(&first)
        && !SHARED.contains(&parent)
        && let Some(map) = map(parent)
    {
        out.push(format!("https://{map}"));
        out.push(format!("http://{map}"));
    }
    let mut seen = Vec::new();
    out.retain(|url| {
        let new = !seen.contains(url);
        seen.push(url.clone());
        new
    });
    out
}

/// Whether `json` is a BlueMap web app's `settings.json`: a list of map ids
/// and where their data lies, not any page a web server returns.
pub fn is_web_app_settings(json: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) else {
        return false;
    };
    let maps = v.get("maps").and_then(|m| m.as_array());
    maps.is_some_and(|maps| !maps.is_empty() && maps.iter().all(|m| m.is_string()))
        && v.get("mapDataRoot").is_some_and(|r| r.is_string())
}

/// Whether a BlueMap web app answers at `base`.
pub fn probe(agent: &ureq::Agent, base: &str) -> bool {
    use std::io::Read;
    let Ok(response) = agent
        .get(&format!("{base}/settings.json"))
        .timeout(TIMEOUT)
        .call()
    else {
        return false;
    };
    let mut body = Vec::new();
    response
        .into_reader()
        .take(256 << 10)
        .read_to_end(&mut body)
        .is_ok()
        && is_web_app_settings(&body)
}

/// What an earlier look found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Remembered {
    Found(String),
    /// Nothing, recently.
    Nothing,
    /// Never looked, or long enough ago to look again (first at the map
    /// found then, if any).
    Stale(Option<String>),
}

pub fn remembered(dir: &Path, now: SystemTime) -> Remembered {
    let Ok(text) = std::fs::read_to_string(dir.join(FILE)) else {
        return Remembered::Stale(None);
    };
    let mut words = text.split_whitespace();
    let (kind, at) = (words.next(), words.next().and_then(|t| t.parse().ok()));
    let Some(at) = at.map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs)) else {
        return Remembered::Stale(None);
    };
    // A time ahead of the clock (a clock set wrong, a copied file) is stale.
    let Ok(age) = now.duration_since(at) else {
        return Remembered::Stale(None);
    };
    match (kind, words.next()) {
        (Some("found"), Some(url)) if age < RETRY_FOUND => Remembered::Found(url.to_owned()),
        (Some("found"), Some(url)) => Remembered::Stale(Some(url.to_owned())),
        (Some("none"), _) if age < RETRY_NONE => Remembered::Nothing,
        _ => Remembered::Stale(None),
    }
}

/// Keeps what a look found (`None`: no map).
pub fn remember(dir: &Path, found: Option<&str>, now: SystemTime) {
    let secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let text = match found {
        Some(url) => format!("found {secs} {url}\n"),
        None => format!("none {secs}\n"),
    };
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(FILE), text);
}

/// The servers being looked at now, by BlueMap folder: one look a server at
/// a time, however many levels open meanwhile.
static LOOKING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The right to look for `dir`'s server's map, until dropped.
pub struct Claim(PathBuf);
impl Claim {
    /// `None` while another look at the same server runs.
    pub fn take(dir: &Path) -> Option<Self> {
        let mut looking = LOOKING.lock().unwrap();
        if looking.iter().any(|d| d == dir) {
            return None;
        }
        looking.push(dir.to_owned());
        Some(Self(dir.to_owned()))
    }
}
impl Drop for Claim {
    fn drop(&mut self) {
        LOOKING.lock().unwrap().retain(|d| *d != self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_cover_the_web_server_host_and_map_subdomains() {
        assert_eq!(
            candidates("Play.Example.net:25566"),
            [
                "http://play.example.net:8100",
                "https://map.play.example.net",
                "https://play.example.net",
                "http://map.play.example.net",
                "http://play.example.net",
                "https://map.example.net",
                "http://map.example.net",
            ]
        );
        assert_eq!(
            candidates("example.net"),
            [
                "http://example.net:8100",
                "https://map.example.net",
                "https://example.net",
                "http://map.example.net",
                "http://example.net",
            ]
        );
        // Already the map's host: no `map.map.`.
        assert_eq!(
            candidates("map.example.net"),
            [
                "http://map.example.net:8100",
                "https://map.example.net",
                "http://map.example.net",
            ]
        );
        assert_eq!(
            candidates("192.0.2.7:25565"),
            ["http://192.0.2.7:8100", "http://192.0.2.7"]
        );
        assert_eq!(
            candidates("[2001:db8::7]:25565"),
            ["http://[2001:db8::7]:8100", "http://[2001:db8::7]"]
        );
        assert_eq!(
            candidates("2001:db8::7"),
            ["http://[2001:db8::7]:8100", "http://[2001:db8::7]"]
        );
        assert_eq!(
            candidates("localhost"),
            [
                "http://localhost:8100",
                "https://localhost",
                "http://localhost"
            ]
        );
        // A hosted name's parent is the host's, not the server's.
        assert!(
            !candidates("play.ddns.net")
                .iter()
                .any(|url| url.ends_with("//map.ddns.net"))
        );
        assert!(
            !candidates("name.host.example")
                .iter()
                .any(|url| url.ends_with("//map.host.example"))
        );
        assert!(candidates("").is_empty());
        assert!(candidates("evil.example.net/path?x").is_empty());
    }

    #[test]
    fn only_blue_maps_settings_count() {
        let web_app = br#"{"version":"5.4","useCookies":true,"maps":["world","world_nether"],
            "mapDataRoot":"maps","liveDataRoot":"maps","scripts":[],"styles":[]}"#;
        assert!(is_web_app_settings(web_app));
        assert!(!is_web_app_settings(b"<!doctype html><html></html>"));
        assert!(
            !is_web_app_settings(br#"{"maps":["world"]}"#),
            "no data root"
        );
        assert!(!is_web_app_settings(br#"{"maps":[],"mapDataRoot":"maps"}"#));
        assert!(!is_web_app_settings(
            br#"{"maps":[1],"mapDataRoot":"maps"}"#
        ));
        assert!(!is_web_app_settings(
            br#"{"maps":"world","mapDataRoot":"maps"}"#
        ));
        assert!(!is_web_app_settings(
            br#"[{"maps":["world"],"mapDataRoot":"maps"}]"#
        ));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "horizon-bluemap-detect-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn answers_are_kept_and_looked_at_again_later() {
        let dir = scratch("remember");
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let hour = Duration::from_secs(3600);
        assert_eq!(remembered(&dir, t0), Remembered::Stale(None));
        remember(&dir, None, t0);
        assert_eq!(remembered(&dir, t0 + hour), Remembered::Nothing);
        // Written ahead of the clock: not trusted.
        assert_eq!(remembered(&dir, t0 - hour), Remembered::Stale(None));
        assert_eq!(remembered(&dir, t0 + RETRY_NONE), Remembered::Stale(None));
        remember(&dir, Some("https://map.example.net"), t0);
        assert_eq!(
            remembered(&dir, t0 + 2 * RETRY_NONE),
            Remembered::Found("https://map.example.net".into())
        );
        assert_eq!(
            remembered(&dir, t0 + RETRY_FOUND),
            Remembered::Stale(Some("https://map.example.net".into()))
        );
        std::fs::write(dir.join(FILE), "garbage").unwrap();
        assert_eq!(remembered(&dir, t0), Remembered::Stale(None));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_players_map_wins_and_detection_waits_for_the_setting() {
        let dir = scratch("source");
        let now = SystemTime::now();
        let mut settings = Settings::default();
        let address = "play.example.net";
        assert_eq!(
            source(&settings, address, &dir, now),
            None,
            "off by default"
        );
        settings.auto_detect = true;
        let Some(Source::Detect(probes)) = source(&settings, address, &dir, now) else {
            panic!("looked for");
        };
        assert_eq!(probes, candidates(address));
        remember(&dir, None, now);
        assert_eq!(
            source(&settings, address, &dir, now),
            None,
            "none, recently"
        );
        remember(&dir, Some("https://map.example.net"), now);
        assert_eq!(
            source(&settings, address, &dir, now),
            Some(Source::Url {
                url: "https://map.example.net".into(),
                maps: Vec::new()
            })
        );
        // A remembered map this server's addresses don't include (a copied
        // folder's) is no map.
        remember(&dir, Some("https://tracker.example"), now);
        let Some(Source::Detect(probes)) = source(&settings, address, &dir, now) else {
            panic!("looked for, the remembered one ignored");
        };
        assert_eq!(probes, candidates(address));
        remember(&dir, Some("https://map.example.net"), now);
        // Past a week the found map is tried first, then the rest.
        let later = now + RETRY_FOUND;
        let Some(Source::Detect(probes)) = source(&settings, address, &dir, later) else {
            panic!("looked for again");
        };
        assert_eq!(probes[0], "https://map.example.net");
        assert_eq!(probes.len(), candidates(address).len());
        settings.servers.push(
            Settings::parse_server("play.example.net http://example.net:8123 nether=n").unwrap(),
        );
        assert_eq!(
            source(&settings, address, &dir, now),
            Some(Source::Url {
                url: "http://example.net:8123".into(),
                maps: vec![("minecraft:the_nether".into(), "n".into())]
            })
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_look_a_server_at_a_time() {
        let dir = scratch("claim");
        let claim = Claim::take(&dir).unwrap();
        assert!(Claim::take(&dir).is_none());
        drop(claim);
        assert!(Claim::take(&dir).is_some());
    }
}
