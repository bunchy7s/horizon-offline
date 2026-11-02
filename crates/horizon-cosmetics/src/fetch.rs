//! What players wear, from Horizon's cosmetics service: one batched request
//! for the players in view, answers cached for the service's TTL, item files
//! cached on disk by their hash. The work runs on a thread of its own; the
//! frame only reads the cache, and a service that doesn't answer leaves the
//! last answers in place and is asked again later, less and less often.
//!
//! Protocol (JSON over HTTPS):
//! - `POST /v1/worn` `{"players": ["<uuid hex>", …]}` (at most `BATCH`) answers
//!   `{"ttl": seconds, "players": {"<uuid hex>": [{"id", "hash"}, …]}}`; a
//!   player who wears nothing is left out.
//! - `GET /v1/items/<id>?hash=<hash>` answers the item file, which never
//!   changes for a hash.

use crate::Item;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Players per request.
pub const BATCH: usize = 64;
/// Players remembered; the least recently seen go first.
const REMEMBERED: usize = 4096;
/// The service's TTL is kept within these.
const TTL: (u64, u64) = (60, 3600);
/// The wait after a failure doubles from the first to the second.
const BACKOFF: (Duration, Duration) = (Duration::from_secs(15), Duration::from_secs(600));

/// What a player wears, in slot order of the service's answer.
pub type Worn = Arc<[Arc<Item>]>;

/// How the service is reached; `Http` in the game, a fake in tests.
pub trait Transport: Send + 'static {
    fn post(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, String>;
    fn get(&self, path: &str) -> Result<Vec<u8>, String>;
}

pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

#[derive(Default)]
struct State {
    players: HashMap<u128, Entry>,
    /// Seen without a fresh answer, not yet asked.
    pending: HashSet<u128>,
    /// Items by `id@hash`: `None` for a file that can't be read or was refused.
    items: HashMap<String, Option<Arc<Item>>>,
    failures: u32,
    retry_at: Option<Instant>,
    stop: bool,
}

struct Entry {
    worn: Option<Worn>,
    expires: Instant,
    seen: Instant,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    clock: Clock,
}

/// The game's handle. Dropping it stops the thread.
pub struct Cosmetics {
    shared: Arc<Shared>,
    worker: Option<Box<dyn FnMut() + Send>>,
}

impl Cosmetics {
    /// Asks `transport` on a thread of its own; item files are kept in
    /// `cache` when given.
    pub fn new(transport: impl Transport, cache: Option<PathBuf>) -> Self {
        let mut c = Self::manual(transport, cache, Arc::new(Instant::now));
        let mut work = c.worker.take().expect("worker");
        let shared = c.shared.clone();
        let _ = std::thread::Builder::new()
            .name("Cosmetics".into())
            .spawn(move || {
                loop {
                    {
                        let state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
                        let state = shared
                            .wake
                            .wait_while(state, |s| !s.stop && !ready(s, (shared.clock)()))
                            .unwrap_or_else(PoisonError::into_inner);
                        if state.stop {
                            return;
                        }
                    }
                    work();
                    // A failure sets retry_at: sleep until then.
                    let wait = {
                        let s = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
                        s.retry_at
                            .map(|t| t.saturating_duration_since((shared.clock)()))
                    };
                    if let Some(wait) = wait {
                        let s = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
                        let _ = shared.wake.wait_timeout_while(s, wait, |s| !s.stop);
                    }
                }
            });
        c
    }

    /// No thread: `work` runs one round where called (tests).
    fn manual(transport: impl Transport, cache: Option<PathBuf>, clock: Clock) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            wake: Condvar::new(),
            clock,
        });
        let inner = shared.clone();
        let worker: Box<dyn FnMut() + Send> = Box::new(move || work(&inner, &transport, &cache));
        Self {
            shared,
            worker: Some(worker),
        }
    }

    /// One frame's reads: lock once, ask `worn` for each player in view.
    pub fn frame(&self) -> Frame<'_> {
        Frame {
            now: (self.shared.clock)(),
            state: self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            wake: &self.shared.wake,
            asked: false,
        }
    }
}

impl Drop for Cosmetics {
    fn drop(&mut self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop = true;
        self.shared.wake.notify_all();
    }
}

pub struct Frame<'a> {
    now: Instant,
    state: std::sync::MutexGuard<'a, State>,
    wake: &'a Condvar,
    asked: bool,
}

impl Frame<'_> {
    /// What `uuid` wears as last known (`None`: nothing, or not known yet).
    /// A player not known, or known for longer than the TTL, is asked for.
    pub fn worn(&mut self, uuid: u128) -> Option<Worn> {
        let now = self.now;
        let state = &mut *self.state;
        match state.players.get_mut(&uuid) {
            Some(entry) => {
                entry.seen = now;
                if entry.expires <= now && state.pending.insert(uuid) {
                    self.asked = true;
                }
                entry.worn.clone()
            }
            None => {
                self.asked |= state.pending.insert(uuid);
                None
            }
        }
    }
}

impl Drop for Frame<'_> {
    fn drop(&mut self) {
        if self.asked {
            self.wake.notify_all();
        }
    }
}

fn ready(s: &State, now: Instant) -> bool {
    !s.pending.is_empty() && s.retry_at.is_none_or(|t| t <= now)
}

#[derive(serde::Deserialize)]
struct Answer {
    ttl: u64,
    players: HashMap<String, Vec<ItemRef>>,
}

#[derive(serde::Deserialize)]
struct ItemRef {
    id: String,
    hash: String,
}

/// One round: asks for up to `BATCH` pending players, then reads the items
/// they wear (memory, disk, then the service).
fn work(shared: &Shared, transport: &dyn Transport, cache: &Option<PathBuf>) {
    let batch: Vec<u128> = {
        let mut s = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !ready(&s, (shared.clock)()) {
            return;
        }
        let batch: Vec<u128> = s.pending.iter().copied().take(BATCH).collect();
        for uuid in &batch {
            s.pending.remove(uuid);
        }
        batch
    };
    let body = serde_json::json!({
        "players": batch.iter().map(|u| format!("{u:032x}")).collect::<Vec<_>>()
    });
    let answer = transport
        .post("/v1/worn", body.to_string().as_bytes())
        .and_then(|b| serde_json::from_slice::<Answer>(&b).map_err(|e| e.to_string()));
    let answer = match answer {
        Ok(a) => a,
        Err(_) => {
            let mut s = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
            s.failures += 1;
            let wait = BACKOFF.0 * 2u32.saturating_pow(s.failures - 1).min(64);
            s.retry_at = Some((shared.clock)() + wait.min(BACKOFF.1));
            s.pending.extend(batch);
            return;
        }
    };
    let mut worn: HashMap<u128, Worn> = HashMap::new();
    // Players with an item the service didn't send: asked again soon.
    let mut short = HashSet::new();
    for (uuid, refs) in answer.players {
        let Ok(uuid) = u128::from_str_radix(&uuid, 16) else {
            continue;
        };
        let mut items = Vec::new();
        for r in &refs {
            match item(shared, transport, cache, r) {
                Ok(Some(i)) => items.push(i),
                Ok(None) => {}
                Err(()) => {
                    short.insert(uuid);
                }
            }
        }
        if !items.is_empty() {
            worn.insert(uuid, items.into());
        }
    }
    let now = (shared.clock)();
    let ttl = Duration::from_secs(answer.ttl.clamp(TTL.0, TTL.1));
    let mut s = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
    s.failures = 0;
    s.retry_at = None;
    for uuid in batch {
        let seen = s.players.get(&uuid).map_or(now, |e| e.seen);
        let ttl = if short.contains(&uuid) {
            BACKOFF.0
        } else {
            ttl
        };
        s.players.insert(
            uuid,
            Entry {
                worn: worn.remove(&uuid),
                expires: now + ttl,
                seen,
            },
        );
    }
    if s.players.len() > REMEMBERED {
        let mut seen: Vec<Instant> = s.players.values().map(|e| e.seen).collect();
        seen.sort_unstable();
        let cut = seen[s.players.len() - REMEMBERED];
        s.players.retain(|_, e| e.seen >= cut);
    }
}

/// An item file: from memory, the disk cache, else the service (then kept
/// on disk). A file that doesn't read is remembered as such (`Ok(None)`);
/// `Err` when the service didn't send it.
fn item(
    shared: &Shared,
    transport: &dyn Transport,
    cache: &Option<PathBuf>,
    r: &ItemRef,
) -> Result<Option<Arc<Item>>, ()> {
    let safe = |t: &str| {
        !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    if !safe(&r.id) || !safe(&r.hash) {
        return Ok(None);
    }
    let key = format!("{}@{}", r.id, r.hash);
    if let Some(known) = shared
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .items
        .get(&key)
    {
        return Ok(known.clone());
    }
    let file = cache
        .as_ref()
        .map(|d| d.join(format!("{}-{}.json", r.id, r.hash)));
    let from_disk = file.as_ref().and_then(|f| std::fs::read(f).ok());
    let bytes = match from_disk {
        Some(b) => Some(b),
        None => {
            let fetched = transport
                .get(&format!("/v1/items/{}?hash={}", r.id, r.hash))
                .map_err(|_| ())?;
            if let Some(f) = &file
                && Item::parse(&fetched).is_ok()
                && let Some(dir) = f.parent()
                && std::fs::create_dir_all(dir).is_ok()
            {
                let part = f.with_extension("part");
                if std::fs::write(&part, &fetched).is_ok() {
                    let _ = std::fs::rename(&part, f);
                }
            }
            Some(fetched)
        }
    };
    let parsed = bytes
        .and_then(|b| Item::parse(&b).ok())
        .filter(|i| i.id == r.id)
        .map(Arc::new);
    shared
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .items
        .insert(key, parsed.clone());
    Ok(parsed)
}

/// The service over HTTPS.
pub struct Http {
    agent: ureq::Agent,
    base: String,
}

/// Answers are small; anything longer is refused.
const MAX_ANSWER: u64 = 1 << 20;

impl Http {
    /// `base` without a trailing slash, e.g. `https://cosmetics.example`.
    pub fn new(base: &str, user_agent: &str) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .user_agent(user_agent)
                .build(),
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    fn read(answer: Result<ureq::Response, ureq::Error>) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let answer = answer.map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("HTTP {code}"),
            ureq::Error::Transport(t) => t.kind().to_string(),
        })?;
        let mut bytes = Vec::new();
        answer
            .into_reader()
            .take(MAX_ANSWER + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_ANSWER {
            return Err("answer too long".into());
        }
        Ok(bytes)
    }
}

impl Transport for Http {
    fn post(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        let url = format!("{}{path}", self.base);
        Self::read(
            self.agent
                .post(&url)
                .set("Content-Type", "application/json")
                .send_bytes(body),
        )
    }

    fn get(&self, path: &str) -> Result<Vec<u8>, String> {
        Self::read(self.agent.get(&format!("{}{path}", self.base)).call())
    }
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;
