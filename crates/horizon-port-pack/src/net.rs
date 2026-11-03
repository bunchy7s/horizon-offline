//! The agents port-pack's downloads go through: the jars of an imported modpack come from the same
//! places as everything else, so they take the same proxy — and leave the same hosts alone.
//!
//! The launcher hands its own setting over (`set_proxy`, from `proxy.rs`); a run of the
//! `horizon-port-pack` command reads `HTTPS_PROXY`, `HTTP_PROXY` and `ALL_PROXY` instead (`env_for`),
//! taking the one that matches the address's scheme the way the launcher does. Either way a host in
//! `NO_PROXY`, or on this computer, is reached directly: a host that only answers without the proxy
//! would otherwise fail here while the launcher's own downloads worked.
//!
//! Which hosts those are is decided here and nowhere else. The launcher asks `direct`, `env_for`,
//! `host_of`, `scheme_of`, `socks` and `parse` for its own traffic, and keeps its agents here too
//! (`keep`), so the two agree whatever the player set — and there is one copy of the rules to
//! change when they need it.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// What the launcher set, when it did.
static PROXY: Mutex<Option<String>> = Mutex::new(None);

/// Agents kept for reuse, named by the part that asked and the proxy they use: a fresh `Agent`
/// starts a new connection pool, so building one per request costs a handshake per file.
static KEPT: Mutex<Option<HashMap<String, ureq::Agent>>> = Mutex::new(None);

/// The proxy to use from now on: the launcher's setting, or None to fall back to the environment.
/// The agents built so far are dropped when it changes, since they hold the address it replaces.
pub fn set_proxy(spec: Option<String>) {
    let spec = spec.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let mut proxy = lock(&PROXY);
    if *proxy == spec {
        return;
    }
    *proxy = spec;
    drop(proxy);
    *lock(&KEPT) = None;
}

/// The proxy address `url` is fetched through: what the launcher set, else the environment's for
/// its scheme, unless `url` is a host that must be reached directly.
fn spec_for(url: &str) -> Option<String> {
    if direct(url) {
        return None;
    }
    lock(&PROXY)
        .clone()
        .or_else(|| env_for(scheme_of(url).unwrap_or("https")))
}

/// An agent for downloading `url`: through the proxy when one is set, unless `url` is a host that
/// must be reached directly.
pub fn agent(url: &str) -> ureq::Agent {
    keep("port-pack", spec_for(url).as_deref(), |proxy| {
        let builder = ureq::AgentBuilder::new();
        match proxy {
            Some(proxy) => builder.proxy(proxy).build(),
            None => builder.build(),
        }
    })
}

/// The agent `url` is fetched with, built by `build` and then kept for the next request that names
/// the same `slot` and proxy: a fresh `Agent` starts a new connection pool, so building one per
/// request is a handshake per file, and a modpack's jars are fetched one after another.
pub fn keep(
    slot: &str,
    spec: Option<&str>,
    build: impl FnOnce(Option<ureq::Proxy>) -> ureq::Agent,
) -> ureq::Agent {
    let proxy = spec.and_then(parse);
    // An address that isn't a proxy is used as none, and remembered as none.
    let key = format!(
        "{slot} {}",
        if proxy.is_some() {
            spec.unwrap_or_default()
        } else {
            ""
        }
    );
    if let Some(agent) = lock(&KEPT).as_ref().and_then(|kept| kept.get(&key)) {
        return agent.clone();
    }
    let agent = build(proxy);
    lock(&KEPT)
        .get_or_insert_with(HashMap::new)
        .insert(key, agent.clone());
    agent
}

/// `spec` as a proxy, when it is one. An address that can't be used is said once: a download left
/// to go direct would otherwise fail with nothing pointing at the proxy.
pub fn parse(spec: &str) -> Option<ureq::Proxy> {
    if socks(spec) {
        say(&format!(
            "{} is a SOCKS proxy; downloads go without one. Write the http:// address your \
             proxy app listens on as well.",
            redacted(spec)
        ));
        return None;
    }
    match ureq::Proxy::new(spec) {
        Ok(proxy) => Some(proxy),
        Err(e) => {
            say(&format!(
                "The proxy address {} can't be used ({e}); downloads go without one.",
                redacted(spec)
            ));
            None
        }
    }
}

/// `spec` as it may be shown: a user name and password in it become `***`. An address that can't
/// be used is printed to stderr and shown in the launcher, and a proxy's credentials don't belong in
/// either. Everything up to the last `@` is the credentials, so a password with an unescaped `/` or
/// `@` is hidden too.
pub fn redacted(spec: &str) -> String {
    let (scheme, rest) = match spec.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), spec),
    };
    match rest.rfind('@') {
        Some(at) => format!("{scheme}***@{}", &rest[at + 1..]),
        None => spec.to_owned(),
    }
}

/// Whether `spec` names a SOCKS proxy, which these downloads don't go through: `ureq` reads a
/// SOCKS address happily and only fails when it connects through it.
pub fn socks(spec: &str) -> bool {
    matches!(
        spec.split_once("://")
            .map(|(s, _)| s.to_ascii_lowercase())
            .as_deref(),
        Some("socks" | "socks4" | "socks4a" | "socks5" | "socks5h")
    )
}

/// Whether `url` is reached without the proxy: a local address, or a host `NO_PROXY` names. The
/// launcher asks this of its own requests, so both reach the same hosts directly.
pub fn direct(url: &str) -> bool {
    host_of(url).is_some_and(|host| local(host) || excluded(host, &no_proxy()))
}

/// `HTTPS_PROXY` (`HTTP_PROXY` for an http address), then `ALL_PROXY`, in the upper-case spelling
/// first and the lower-case one after, as the tools that write them do.
pub fn env_for(scheme: &str) -> Option<String> {
    let names: &[&str] = if scheme.eq_ignore_ascii_case("http") {
        &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    };
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}

/// `NO_PROXY`'s list: the hosts reached directly whatever the proxy is.
fn no_proxy() -> String {
    ["NO_PROXY", "no_proxy"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_default()
}

/// The host part of a URL (`https://user@host:port/path` is `host`).
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['/', '?', '#']).next()?;
    let rest = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    Some(match rest.rfind(':') {
        // A port follows the last colon; an IPv6 literal keeps the colons inside its brackets.
        Some(at) if !rest[at..].contains(']') => &rest[..at],
        _ => rest,
    })
}

/// `https` in `https://…`, or None for anything that isn't an absolute address.
pub fn scheme_of(url: &str) -> Option<&str> {
    url.split_once("://").map(|(scheme, _)| scheme)
}

/// Whether `host` is this computer: the game's own API and a server on the same machine.
fn local(host: &str) -> bool {
    let host = bare(host);
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "::1"
        || host == "0.0.0.0"
        || is_loopback_v4(&host)
}

/// A dotted IPv4 address in `127.0.0.0/8`; a DNS name that merely starts with `127.` is not one.
fn is_loopback_v4(host: &str) -> bool {
    host.starts_with("127.")
        && host
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `host` is in `NO_PROXY`: `*`, `host`, `.domain`, `*.domain` and `host:port` entries.
fn excluded(host: &str, no_proxy: &str) -> bool {
    let host = bare(host);
    no_proxy
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .any(|entry| {
            if entry == "*" {
                return true;
            }
            let entry = entry.trim_start_matches('*').trim_start_matches('.');
            // A port follows the last colon of a bracketed entry, or of a host with no colon
            // before it; an unbracketed IPv6 literal keeps its colons.
            let entry = match entry.rfind(':') {
                Some(at)
                    if !entry[at..].contains(']')
                        && (entry[..at].ends_with(']') || !entry[..at].contains(':')) =>
                {
                    &entry[..at]
                }
                _ => entry,
            };
            let entry = bare(entry);
            !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
        })
}

/// A host without the brackets an IPv6 literal wears, lower case.
fn bare(host: &str) -> String {
    host.trim().trim_matches(['[', ']']).to_ascii_lowercase()
}

/// Said once, however many downloads follow: a wrong address is one thing to report, not one
/// report per file.
fn say(what: &str) {
    static SAID: AtomicBool = AtomicBool::new(false);
    if !SAID.swap(true, Ordering::Relaxed) {
        eprintln!("{what}");
    }
}

/// The lock, even after a thread panicked while holding it: an address lost to a panic is worse
/// than an old one kept.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the tests below share, since the address and the agents are one of each for the whole
    /// process: they take it in turn.
    fn alone() -> MutexGuard<'static, ()> {
        static TURN: Mutex<()> = Mutex::new(());
        lock(&TURN)
    }

    #[test]
    fn credentials_are_not_shown() {
        assert_eq!(redacted("http://127.0.0.1:7890"), "http://127.0.0.1:7890");
        assert_eq!(redacted("proxy.local:3128"), "proxy.local:3128");
        assert_eq!(
            redacted("http://user:pass@proxy.local:3128"),
            "http://***@proxy.local:3128"
        );
        assert_eq!(
            redacted("socks5://u:p@10.0.0.1:1080"),
            "socks5://***@10.0.0.1:1080"
        );
        assert_eq!(
            redacted("user:pass@proxy.local:3128"),
            "***@proxy.local:3128"
        );
        assert_eq!(redacted("http://u:p/a@ss@proxy:1"), "http://***@proxy:1");
        assert_eq!(redacted("http://user:pass@"), "http://***@");
    }

    #[test]
    fn a_proxy_the_launcher_sets_is_trimmed_and_kept() {
        let _turn = alone();
        set_proxy(Some("  http://127.0.0.1:7890  ".into()));
        assert_eq!(
            PROXY.lock().unwrap().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        set_proxy(Some("   ".into()));
        assert!(PROXY.lock().unwrap().is_none());
    }

    /// An address on this computer is reached directly whatever `NO_PROXY` says: what an import
    /// needs when the launcher's own requests to it go direct.
    #[test]
    fn a_local_address_is_reached_directly() {
        assert!(direct("http://127.0.0.1:8130/api/v1/status"));
        assert!(direct("http://localhost/api/v1/status"));
        assert!(direct("https://[::1]:25565/"));
    }

    #[test]
    fn a_host_is_read_out_of_the_address() {
        assert_eq!(
            host_of("https://cdn.modrinth.com/v2/x.jar"),
            Some("cdn.modrinth.com")
        );
        assert_eq!(
            host_of("https://user:pass@piston-meta.mojang.com:8443/a"),
            Some("piston-meta.mojang.com")
        );
        assert_eq!(
            host_of("http://127.0.0.1:8130/api/v1/status"),
            Some("127.0.0.1")
        );
        assert_eq!(host_of("https://[::1]:25565/"), Some("[::1]"));
        assert_eq!(host_of("https://example.com?x=1"), Some("example.com"));
        assert_eq!(host_of("https://example.com#top"), Some("example.com"));
    }

    #[test]
    fn only_absolute_addresses_have_a_scheme() {
        assert_eq!(scheme_of("https://a.b"), Some("https"));
        assert_eq!(scheme_of("http://a.b"), Some("http"));
        assert_eq!(scheme_of("a.b/c"), None);
    }

    #[test]
    fn this_computer_is_never_proxied() {
        for host in [
            "localhost",
            "LOCALHOST",
            "[::1]",
            "::1",
            "127.0.0.1",
            "127.1.2.3",
        ] {
            assert!(local(host), "{host}");
        }
        for host in [
            "playhorizon.dev",
            "api.minecraftservices.com",
            "10.0.0.1",
            "127.example.com",
            "127.0.0.1.example.com",
        ] {
            assert!(!local(host), "{host}");
        }
    }

    /// What an import with `NO_PROXY=modrinth.com` needs: the launcher's own requests to it go
    /// direct, so the jars fetched for it must too.
    #[test]
    fn no_proxy_lists_hosts_and_their_subdomains() {
        assert!(excluded("anything.at.all", "*"));
        assert!(excluded("cdn.modrinth.com", "modrinth.com"));
        assert!(excluded("api.modrinth.com", "modrinth.com"));
        assert!(excluded("piston-meta.mojang.com", "mojang.com"));
        assert!(excluded(
            "wiki.corp.example",
            " .corp.example , other.host "
        ));
        assert!(excluded("github.com", "github.com:443"));
        assert!(!excluded("notmodrinth.com", "modrinth.com"));
        assert!(!excluded("modrinth.com", " .corp.example , mojang.com "));
        assert!(!excluded("edge.forgecdn.net", "modrinth.com"));
        assert!(!excluded("modrinth.com", ""));
    }

    /// A bare IPv6 entry keeps its colons: cut at the last one it would never match the host in
    /// its brackets, and the download would go through the proxy after all.
    #[test]
    fn no_proxy_keeps_a_bare_ipv6_entry_whole() {
        assert!(excluded("[2001:db8::1]", "2001:db8::1"));
        assert!(excluded("[2001:db8::1]", "[2001:db8::1]"));
        assert!(excluded("[2001:db8::1]", "[2001:db8::1]:8080"));
        assert!(!excluded("[2001:db8::1]", "2001:db8::2"));
    }

    #[test]
    fn an_address_that_cant_be_used_becomes_no_proxy() {
        assert!(parse("http://127.0.0.1:7890").is_some());
        assert!(parse("127.0.0.1:7890").is_some());
        // `ureq` reads a SOCKS address and only fails once it connects through it.
        assert!(socks("socks5://127.0.0.1:7891"));
        assert!(parse("socks5://127.0.0.1:7891").is_none());
        assert!(!socks("http://127.0.0.1:7890"));
    }

    /// The agent a second request gets is the one the first built: one connection pool, not one
    /// handshake per jar.
    #[test]
    fn an_agent_is_kept_for_the_next_request() {
        let _turn = alone();
        set_proxy(None);
        let mut built = 0;
        for _ in 0..3 {
            keep("kept", None, |_| {
                built += 1;
                ureq::AgentBuilder::new().build()
            });
        }
        assert_eq!(built, 1, "three requests, one agent");
        // Another address is another agent, and a new setting drops the ones before it.
        keep("kept", Some("http://127.0.0.1:7890"), |proxy| {
            assert!(proxy.is_some());
            built += 1;
            ureq::AgentBuilder::new().build()
        });
        assert_eq!(built, 2, "a second address needs a second agent");
        set_proxy(Some("http://127.0.0.1:7891".into()));
        keep("kept", Some("http://127.0.0.1:7891"), |_| {
            built += 1;
            ureq::AgentBuilder::new().build()
        });
        assert_eq!(built, 3, "a new setting drops the agents kept before it");
        set_proxy(None);
    }
}
