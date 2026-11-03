//! What players wear, from Horizon's cosmetics service: one batched request
//! for the players in view, answers cached for the service's TTL, item files
//! cached on disk by their hash. The work runs on a thread of its own; the
//! frame only reads the cache, and a service that doesn't answer leaves the
//! last answers in place and is asked again later, less and less often.
//! The last answers are also kept on disk (`worn.json`, beside the item
//! files), so a game started while the service is down still shows what
//! players last wore.
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
        let mut saved = None;
        let worker: Box<dyn FnMut() + Send> = Box::new(move || {
            let saved = saved.get_or_insert_with(|| Saved::load(&cache));
            work(&inner, &transport, &cache, saved);
        });
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

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct ItemRef {
    id: String,
    hash: String,
}

/// The last answers on disk: what each player wore whose items all read.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Saved {
    players: HashMap<String, Vec<ItemRef>>,
}

impl Saved {
    const FILE: &str = "worn.json";

    fn load(cache: &Option<PathBuf>) -> Self {
        cache
            .as_ref()
            .and_then(|d| std::fs::read(d.join(Self::FILE)).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, cache: &Option<PathBuf>) {
        let Some(dir) = cache else { return };
        let (Ok(text), Ok(())) = (serde_json::to_vec(self), std::fs::create_dir_all(dir)) else {
            return;
        };
        let part = dir.join("worn.part");
        if std::fs::write(&part, text).is_ok() {
            let _ = std::fs::rename(&part, dir.join(Self::FILE));
        }
    }
}

/// No service: items are read from memory or the disk cache only.
struct Offline;

impl Transport for Offline {
    fn post(&self, _: &str, _: &[u8]) -> Result<Vec<u8>, String> {
        Err("offline".into())
    }
    fn get(&self, _: &str) -> Result<Vec<u8>, String> {
        Err("offline".into())
    }
}

/// One round: asks for up to `BATCH` pending players, then reads the items
/// they wear (memory, disk, then the service). When the service doesn't
/// answer, players not known yet get what `saved` says they last wore.
fn work(shared: &Shared, transport: &dyn Transport, cache: &Option<PathBuf>, saved: &mut Saved) {
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
            let restored = restore(shared, cache, saved, &batch);
            let mut s = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
            let now = (shared.clock)();
            for (uuid, worn) in restored {
                // Shown until the service answers: asked again with the others.
                s.players.entry(uuid).or_insert(Entry {
                    worn: Some(worn),
                    expires: now,
                    seen: now,
                });
            }
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
    let mut changed = false;
    for (hex, refs) in answer.players {
        let Ok(uuid) = u128::from_str_radix(&hex, 16) else {
            continue;
        };
        let mut items = Vec::new();
        let mut kept = Vec::new();
        for r in &refs {
            match item(shared, transport, cache, r) {
                Ok(Some(i)) => {
                    items.push(i);
                    kept.push(r.clone());
                }
                Ok(None) => {}
                Err(()) => {
                    short.insert(uuid);
                }
            }
        }
        if !short.contains(&uuid) && saved.players.get(&hex) != Some(&kept) {
            changed = true;
            if kept.is_empty() {
                saved.players.remove(&hex);
            } else {
                saved.players.insert(hex, kept);
            }
        }
        if !items.is_empty() {
            worn.insert(uuid, items.into());
        }
    }
    // Answered without anything: wears nothing now.
    for uuid in &batch {
        if !worn.contains_key(uuid) && !short.contains(uuid) {
            changed |= saved.players.remove(&format!("{uuid:032x}")).is_some();
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
    if saved.players.len() > REMEMBERED {
        saved.players.retain(|hex, _| {
            u128::from_str_radix(hex, 16).is_ok_and(|u| s.players.contains_key(&u))
        });
        changed = true;
    }
    drop(s);
    if changed {
        saved.save(cache);
    }
}

/// What the players of `batch` not known yet last wore, by `saved` and the
/// item files on disk.
fn restore(
    shared: &Shared,
    cache: &Option<PathBuf>,
    saved: &Saved,
    batch: &[u128],
) -> Vec<(u128, Worn)> {
    let unknown: Vec<u128> = {
        let s = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
        batch
            .iter()
            .copied()
            .filter(|u| !s.players.contains_key(u))
            .collect()
    };
    let mut restored = Vec::new();
    for uuid in unknown {
        let Some(refs) = saved.players.get(&format!("{uuid:032x}")) else {
            continue;
        };
        let items: Vec<Arc<Item>> = refs
            .iter()
            .filter_map(|r| item(shared, &Offline, cache, r).ok().flatten())
            .collect();
        if !items.is_empty() {
            restored.push((uuid, items.into()));
        }
    }
    restored
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

    pub(crate) fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    pub(crate) fn read(answer: Result<ureq::Response, ureq::Error>) -> Result<Vec<u8>, String> {
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

/// Item files in a folder instead of the service (`HORIZON_COSMETICS_LOCAL`): every player asked
/// for wears the first item of each slot found there, by file name. For artists and the bench.
pub struct Local {
    dir: PathBuf,
}

impl Local {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The first item file of each slot, by file name.
    fn worn(&self) -> Vec<String> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        let mut slots = Vec::new();
        let mut ids = Vec::new();
        for file in files {
            if let Some(item) = std::fs::read(&file).ok().and_then(|b| Item::parse(&b).ok())
                && !slots.contains(&item.slot)
                && file.file_stem().is_some_and(|s| *s == *item.id)
            {
                slots.push(item.slot);
                ids.push(item.id);
            }
        }
        ids
    }
}

impl Transport for Local {
    fn post(&self, _: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        #[derive(serde::Deserialize)]
        struct Asked {
            players: Vec<String>,
        }
        let asked: Asked = serde_json::from_slice(body).map_err(|e| e.to_string())?;
        let refs: Vec<_> = self
            .worn()
            .into_iter()
            .map(|id| serde_json::json!({"id": id, "hash": "local"}))
            .collect();
        let players: serde_json::Map<String, serde_json::Value> = asked
            .players
            .into_iter()
            .map(|p| (p, serde_json::Value::Array(refs.clone())))
            .collect();
        Ok(serde_json::json!({"ttl": 60, "players": players})
            .to_string()
            .into_bytes())
    }

    fn get(&self, path: &str) -> Result<Vec<u8>, String> {
        let id = path
            .trim_start_matches("/v1/items/")
            .split('?')
            .next()
            .unwrap_or_default();
        std::fs::read(self.dir.join(format!("{id}.json"))).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;
