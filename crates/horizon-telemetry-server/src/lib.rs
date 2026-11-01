//! The ingest server for opt-in data collection (docs/telemetry.md).
//!
//! - `POST /v1/batch`: a zstd batch from the launcher, checked by `horizon_telemetry::batch`.
//! - `DELETE /v1/installs/<id>`: everything an install sent goes.
//! - `GET /v1/aggregate?days=30`: the anonymous totals the website reads, cached.
//! - `GET /v1/benchmarks/summary`: players' benchmark results grouped by GPU and CPU, groups of at
//!   least 3 installs only (`summary`), cached; the website's Benchmarks page reads it.
//! - `POST /v1/live`: a game's anonymous "I'm playing" ping (`live`, in memory only).
//! - `GET /v1/live`: how many play now, exactly, cached briefly.
//! - `GET /v1/live/history?range=24h|7d|30d|all&series=players|official`: that count, or the
//!   official server's (its status ping), over time, written down once a minute (`history`:
//!   totals only), cached a minute.
//! - `/hub/…`: the hub's votes (`hub`) and bug reports (`intake`).
//! - `GET /healthz`.
//!
//! Abuse limits: a token bucket per IP address (kept in memory for an hour, never stored), a
//! daily item cap per install, and the batch size limits. The live count has its own, larger
//! bucket per address (several players can share one) and a cap on the sessions it holds. Raw
//! rows expire after `retention_days`; the daily totals stay. One small process and one SQLite
//! file: see `README.md` for running it.
use std::collections::HashMap;
use std::hash::{Hash, RandomState};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use horizon_telemetry::batch;

pub mod classes;
pub mod github;
pub mod history;
pub mod hub;
pub mod intake;
pub mod live;
pub mod store;
pub mod summary;

use github::Filer;
use horizon_telemetry::live::{MAX_PING_BYTES, Ping};
use live::Live;
use store::Store;

pub struct Config {
    /// Days raw rows are kept.
    pub retention_days: u32,
    /// Groups in the public totals need at least this many install-days.
    pub min_install_days: u64,
    /// Requests an IP address may make an hour (with a burst of a tenth of it).
    pub requests_per_hour: f64,
    /// Items one install may send a day.
    pub items_per_day: u32,
    /// Behind a proxy (Railway, Fly…): the client's address is the proxy's `X-Real-IP`, else the
    /// last `X-Forwarded-For` entry.
    pub trust_proxy: bool,
    /// The website allowed to read the totals from a browser.
    pub allow_origin: String,
    /// Live count requests (pings and reads) an IP address may make an hour, with a burst of a
    /// tenth of it. A game pings once a minute, so the default lets ten play behind one address.
    pub live_per_hour: f64,
    /// How long a session counts after its last ping.
    pub live_window: Duration,
    /// Sessions the live count holds at most; a new one past that is refused.
    pub live_max_sessions: usize,
    /// Days the live count's minute samples are kept (`history`).
    pub live_minute_days: u32,
    /// Days its hourly totals are kept; at least 31, for the 30-day view. Daily ones stay.
    pub live_hour_days: u32,
    /// The official server, asked for its player count every minute (`history`'s `official`
    /// series); None asks nothing.
    pub official_server: Option<String>,
    /// The hub's votes and reads of their counts an IP address may make an hour.
    pub hub_per_hour: f64,
    /// Bug reports an IP address may send an hour.
    pub hub_posts_per_hour: f64,
    /// Bug reports taken a day, from everyone: each one opens a GitHub issue.
    pub hub_reports_per_day: u32,
    /// Days reports are kept (opened as issues long before).
    pub hub_keep_days: u32,
    /// Opens the website's bug reports as issues; unset, `POST /hub/report` answers 503.
    pub github_token: Option<String>,
    /// Where those issues go, `owner/name`.
    pub github_repo: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            retention_days: 180,
            min_install_days: 5,
            requests_per_hour: 60.0,
            items_per_day: 100,
            trust_proxy: false,
            allow_origin: "https://playhorizon.dev".into(),
            live_per_hour: 600.0,
            live_window: horizon_telemetry::live::WINDOW,
            live_max_sessions: 200_000,
            live_minute_days: 7,
            live_hour_days: 90,
            official_server: Some(horizon_telemetry::server_status::OFFICIAL_STATUS_ADDRESS.into()),
            hub_per_hour: 600.0,
            hub_posts_per_hour: 20.0,
            hub_reports_per_day: 30,
            hub_keep_days: 90,
            github_token: None,
            github_repo: "horizon-rust/horizon".into(),
        }
    }
}

impl Config {
    /// The defaults, with `HORIZON_TELEMETRY_RETENTION_DAYS`, `…_MIN_INSTALL_DAYS`,
    /// `…_REQUESTS_PER_HOUR`, `…_ITEMS_PER_DAY`, `…_TRUST_PROXY` (1), `…_ALLOW_ORIGIN`,
    /// `…_LIVE_PER_HOUR`, `…_LIVE_WINDOW_SECONDS`, `…_LIVE_MAX_SESSIONS`, `…_LIVE_MINUTE_DAYS`,
    /// `…_LIVE_HOUR_DAYS`, `…_OFFICIAL_SERVER` (`off` for none), `…_HUB_PER_HOUR`,
    /// `…_HUB_POSTS_PER_HOUR`, `…_HUB_REPORTS_PER_DAY`, `…_HUB_KEEP_DAYS`, `…_GITHUB_TOKEN` and
    /// `…_GITHUB_REPO`. An empty token counts as unset.
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(format!("HORIZON_TELEMETRY_{k}")).ok();
        let secret = |k: &str| {
            var(k)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let d = Self::default();
        Self {
            retention_days: var("RETENTION_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.retention_days),
            min_install_days: var("MIN_INSTALL_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.min_install_days),
            requests_per_hour: var("REQUESTS_PER_HOUR")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.requests_per_hour),
            items_per_day: var("ITEMS_PER_DAY")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.items_per_day),
            trust_proxy: var("TRUST_PROXY").is_some_and(|v| v == "1"),
            allow_origin: var("ALLOW_ORIGIN").unwrap_or(d.allow_origin),
            live_per_hour: var("LIVE_PER_HOUR")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.live_per_hour),
            live_window: var("LIVE_WINDOW_SECONDS")
                .and_then(|v| v.parse().ok())
                .map_or(d.live_window, Duration::from_secs),
            live_max_sessions: var("LIVE_MAX_SESSIONS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.live_max_sessions),
            live_minute_days: var("LIVE_MINUTE_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.live_minute_days),
            live_hour_days: var("LIVE_HOUR_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.live_hour_days),
            official_server: match var("OFFICIAL_SERVER").as_deref().map(str::trim) {
                Some("" | "off") => None,
                Some(address) => Some(address.to_owned()),
                None => d.official_server,
            },
            hub_per_hour: var("HUB_PER_HOUR")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.hub_per_hour),
            hub_posts_per_hour: var("HUB_POSTS_PER_HOUR")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.hub_posts_per_hour),
            hub_reports_per_day: var("HUB_REPORTS_PER_DAY")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.hub_reports_per_day),
            hub_keep_days: var("HUB_KEEP_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.hub_keep_days),
            github_token: secret("GITHUB_TOKEN"),
            github_repo: var("GITHUB_REPO").unwrap_or(d.github_repo),
        }
    }
}

pub struct App {
    store: Mutex<Store>,
    config: Config,
    /// Tokens left and when they were counted, per client address.
    buckets: Mutex<Buckets>,
    /// The same for the live count's requests.
    live_buckets: Mutex<Buckets>,
    /// Who is playing now (`live`).
    live: Mutex<Live>,
    /// `GET /v1/live`'s answer and when it was made.
    live_cache: Mutex<Option<(Instant, String)>>,
    /// `GET /v1/live/history`'s answers by range, with when they were made.
    history_cache: Mutex<HashMap<(history::Series, history::Range), (Instant, String)>>,
    /// When the server started: the live count is whole only a window later.
    started: Instant,
    /// `/v1/aggregate` answers by `days`, with when they were made.
    cache: Mutex<HashMap<u32, (Instant, String)>>,
    /// `/v1/benchmarks/summary`'s answer and when it was made.
    summary_cache: Mutex<Option<(Instant, String)>>,
    /// Today, in days since 1970 (a test moves it).
    today: Box<dyn Fn() -> u32 + Send + Sync>,
    /// The key the hub's buckets hash addresses with, drawn at start and never stored.
    hub_key: RandomState,
    /// The hub's votes and vote counts, per hashed address.
    hub_buckets: Mutex<hub::HashedBuckets>,
    /// The hub's bug reports, per hashed address.
    hub_post_buckets: Mutex<hub::HashedBuckets>,
    /// `GET /hub/votes`'s answer and when it was made.
    votes_cache: Mutex<Option<(Instant, String)>>,
    /// Opens bug reports as GitHub issues; None without a token.
    filer: Option<Arc<Filer>>,
}

type Buckets = HashMap<IpAddr, (f64, Instant)>;

const CACHE_FOR: Duration = Duration::from_secs(600);

fn status(code: StatusCode, message: &str) -> Response {
    (code, message.to_owned()).into_response()
}

impl App {
    pub fn new(store: Store, config: Config) -> Arc<Self> {
        Self::with_clock(store, config, Box::new(horizon_telemetry::today))
    }

    pub fn with_clock(
        store: Store,
        config: Config,
        today: Box<dyn Fn() -> u32 + Send + Sync>,
    ) -> Arc<Self> {
        let filer = config
            .github_token
            .clone()
            .map(|token| Arc::from(github::github(config.github_repo.clone(), token)));
        Self::with_filer(store, config, today, filer)
    }

    /// [`Self::with_clock`] with the hub's bug reports opened by `filer` (tests give their own).
    pub fn with_filer(
        store: Store,
        config: Config,
        today: Box<dyn Fn() -> u32 + Send + Sync>,
        filer: Option<Arc<Filer>>,
    ) -> Arc<Self> {
        let live = Live::new(config.live_window, config.live_max_sessions);
        Arc::new(Self {
            store: Mutex::new(store),
            config,
            buckets: Mutex::default(),
            live_buckets: Mutex::default(),
            live: Mutex::new(live),
            live_cache: Mutex::default(),
            history_cache: Mutex::default(),
            started: Instant::now(),
            cache: Mutex::default(),
            summary_cache: Mutex::default(),
            today,
            hub_key: RandomState::new(),
            hub_buckets: Mutex::default(),
            hub_post_buckets: Mutex::default(),
            votes_cache: Mutex::default(),
            filer,
        })
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes a token for `ip`; false when it has none left.
    fn allow(&self, ip: IpAddr) -> bool {
        take(&self.buckets, self.config.requests_per_hour, ip)
    }

    /// [`Self::allow`] for the live count's requests.
    fn allow_live(&self, ip: IpAddr) -> bool {
        take(&self.live_buckets, self.config.live_per_hour, ip)
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The address requests are counted under, for the rate limits only: kept in memory, never
    /// stored or logged. Behind a proxy (Railway) the TCP peer is the proxy, so it is the proxy's
    /// `X-Real-IP`, else the last `X-Forwarded-For` entry: the one the proxy added, which a client
    /// can't choose (the first one it can). An IPv6 client counts by its /64.
    fn client(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> IpAddr {
        let header = |name: &str| headers.get(name)?.to_str().ok();
        let forwarded = self
            .config
            .trust_proxy
            .then(|| {
                header("x-real-ip")
                    .and_then(|ip| ip.trim().parse().ok())
                    .or_else(|| {
                        header("x-forwarded-for")?
                            .rsplit(',')
                            .next()?
                            .trim()
                            .parse()
                            .ok()
                    })
            })
            .flatten();
        rate_key(
            forwarded
                .or(peer.map(|p| p.ip()))
                .unwrap_or(IpAddr::from([0, 0, 0, 0])),
        )
    }

    /// The day an install's newest row came in, if it has any left.
    pub fn newest_day(&self, install: &[u8]) -> Option<u32> {
        self.store().newest_day(install).ok().flatten()
    }

    /// Drops the cached public answers (after a deletion or expiry).
    fn forget_cached(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        *self.summary_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Writes down the live count as this minute's (`history`), once the server has run a whole
    /// window: before that, games that play haven't all pinged it yet. True when it did.
    pub fn record_live(&self) -> rusqlite::Result<bool> {
        if self.started.elapsed() < self.config.live_window {
            return Ok(false);
        }
        let count = self.live().count(Instant::now());
        self.record_sample(
            history::Series::Players,
            unix_now(),
            &history::Sample::of(&count),
        )
    }

    /// Asks the official server for its player count (a status ping, blocking, at most 5 seconds
    /// a step) and writes it down as this minute's. False when there is no official server or it
    /// didn't answer: that minute stays empty.
    pub fn record_official(&self) -> rusqlite::Result<bool> {
        let Some(address) = &self.config.official_server else {
            return Ok(false);
        };
        let unix = unix_now();
        match horizon_telemetry::server_status::players(address, Duration::from_secs(5)) {
            Ok(players) => {
                let sample = history::Sample {
                    playing: players.online,
                    ..history::Sample::default()
                };
                self.record_sample(history::Series::Official, unix, &sample)
            }
            Err(_) => Ok(false),
        }
    }

    /// Writes down `sample` as `series`' count of the minute holding `unix` (seconds). False when
    /// that minute has one already.
    pub fn record_sample(
        &self,
        series: history::Series,
        unix: i64,
        sample: &history::Sample,
    ) -> rusqlite::Result<bool> {
        self.store().record_live(series, unix, sample)
    }

    /// Daily upkeep: totals rebuilt, expired rows removed, idle rate-limit entries and live
    /// sessions forgotten, the hub's old reports removed. Reports still waiting for
    /// their issue are [`Self::file_pending`]'s.
    pub fn maintain(&self) -> rusqlite::Result<usize> {
        let oldest = (self.today)().saturating_sub(self.config.retention_days);
        let removed = self.store().expire(oldest)?;
        self.expire_intake()?;
        self.forget_hub_buckets();
        for buckets in [&self.buckets, &self.live_buckets] {
            buckets
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(3600));
        }
        self.live().forget_expired(Instant::now());
        self.store().prune_live(
            unix_now(),
            self.config.live_minute_days,
            self.config.live_hour_days.max(31),
        )?;
        if removed > 0 {
            self.forget_cached();
        }
        Ok(removed)
    }
}

/// Takes a token for `key` from its bucket in `buckets` (`per_hour`, bursts of a tenth of it);
/// false when it has none left.
fn take<K: Hash + Eq>(buckets: &Mutex<HashMap<K, (f64, Instant)>>, per_hour: f64, key: K) -> bool {
    let rate = per_hour / 3600.0;
    let burst = (per_hour / 10.0).max(1.0);
    let mut buckets = buckets.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let (tokens, at) = buckets.entry(key).or_insert((burst, now));
    *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * rate).min(burst);
    *at = now;
    if *tokens < 1.0 {
        return false;
    }
    *tokens -= 1.0;
    true
}

/// The address a client's requests count under: IPv4 (an IPv4-mapped IPv6 address as IPv4 too),
/// or an IPv6 address's /64, which one home or host holds whole, so addresses rotated inside it
/// don't get fresh buckets.
fn rate_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
        v4 => v4,
    }
}

fn peer(req: &Request) -> Option<SocketAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0)
}

async fn receive(State(app): State<Arc<App>>, req: Request) -> Response {
    let ip = app.client(req.headers(), peer(&req));
    if !app.allow(ip) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let zstd = req
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"zstd"));
    if !zstd {
        return status(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "send zstd (Content-Encoding: zstd)",
        );
    }
    let Ok(body) = axum::body::to_bytes(req.into_body(), batch::MAX_COMPRESSED).await else {
        return status(StatusCode::PAYLOAD_TOO_LARGE, "a batch is at most 256 KiB");
    };
    let batch = match batch::decode(&body) {
        Ok(batch) => batch,
        Err(e) => return status(StatusCode::BAD_REQUEST, &e),
    };
    let today = (app.today)();
    let install = store::install_key(&batch.install_id).unwrap_or_default();
    let mut store = app.store();
    let sent = store.count_on(&install, today).unwrap_or(u32::MAX);
    if sent.saturating_add(batch.items.len() as u32) > app.config.items_per_day {
        return status(
            StatusCode::TOO_MANY_REQUESTS,
            "this install's daily limit is reached",
        );
    }
    match store.insert(&batch, today) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("horizon-telemetry-server: could not store a batch: {e}");
            status(StatusCode::SERVICE_UNAVAILABLE, "try again later")
        }
    }
}

async fn forget(State(app): State<Arc<App>>, Path(id): Path<String>, req: Request) -> Response {
    if !app.allow(app.client(req.headers(), peer(&req))) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let Some(install) = store::install_key(&id) else {
        return status(StatusCode::BAD_REQUEST, "not an install ID");
    };
    match app.store().delete_install(&install) {
        Ok(_) => {
            app.forget_cached();
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            eprintln!("horizon-telemetry-server: could not delete an install: {e}");
            status(StatusCode::SERVICE_UNAVAILABLE, "try again later")
        }
    }
}

#[derive(serde::Deserialize)]
struct Window {
    days: Option<u32>,
}

async fn aggregate(State(app): State<Arc<App>>, Query(window): Query<Window>) -> Response {
    let days = window.days.unwrap_or(30).clamp(1, 366);
    let cached = app
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&days)
        .filter(|(at, _)| at.elapsed() < CACHE_FOR)
        .map(|(_, json)| json.clone());
    let json = match cached {
        Some(json) => json,
        None => {
            let to = (app.today)();
            let made =
                app.store()
                    .aggregate(to + 1 - days.min(to + 1), to, app.config.min_install_days);
            match made {
                Ok(value) => {
                    let json = value.to_string();
                    app.cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(days, (Instant::now(), json.clone()));
                    json
                }
                Err(e) => {
                    eprintln!("horizon-telemetry-server: could not build the totals: {e}");
                    return status(StatusCode::SERVICE_UNAVAILABLE, "try again later");
                }
            }
        }
    };
    json_response(&app, json, CACHE_FOR.as_secs())
}

/// `GET /v1/benchmarks/summary`: players' benchmark groups (`summary`), made again at most every
/// ten minutes.
async fn benchmarks(State(app): State<Arc<App>>) -> Response {
    let cached = app
        .summary_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|(at, _)| at.elapsed() < CACHE_FOR)
        .map(|(_, json)| json.clone());
    let json = match cached {
        Some(json) => json,
        None => {
            let oldest = (app.today)().saturating_sub(app.config.retention_days);
            let uploads = match app.store().benchmark_uploads(oldest) {
                Ok(uploads) => uploads,
                Err(e) => {
                    eprintln!("horizon-telemetry-server: could not read the benchmarks: {e}");
                    return status(StatusCode::SERVICE_UNAVAILABLE, "try again later");
                }
            };
            let json = summary::summary(&uploads, summary::MIN_INSTALLS, &utc_now()).to_string();
            *app.summary_cache.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((Instant::now(), json.clone()));
            json
        }
    };
    json_response(&app, json, CACHE_FOR.as_secs())
}

/// Now in seconds since 1970.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Now as `YYYY-MM-DDTHH:MM:SSZ`.
fn utc_now() -> String {
    let secs = unix_now() as u64;
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// `POST /v1/live`: one "I'm playing". 204, or 400 for a bad ping, 413 for one too large, 429
/// over the address's limit, 503 when the table is full.
async fn ping(State(app): State<Arc<App>>, req: Request) -> Response {
    if !app.allow_live(app.client(req.headers(), peer(&req))) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let Ok(body) = axum::body::to_bytes(req.into_body(), MAX_PING_BYTES).await else {
        return status(StatusCode::PAYLOAD_TOO_LARGE, "a ping is at most 512 bytes");
    };
    let ping: Ping = match serde_json::from_slice(&body) {
        Ok(ping) => ping,
        Err(e) => return status(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    if let Err(e) = ping.check() {
        return status(StatusCode::BAD_REQUEST, e);
    }
    match app.live().ping(&ping, Instant::now()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(live::Full) => status(StatusCode::SERVICE_UNAVAILABLE, "try again later"),
    }
}

/// `GET /v1/live`: the exact live count (`horizon_telemetry::live::Count`), made again at
/// most every eighth of the window.
async fn playing(State(app): State<Arc<App>>, req: Request) -> Response {
    if !app.allow_live(app.client(req.headers(), peer(&req))) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let fresh_for = app.config.live_window / 8;
    let json = {
        let mut cache = app.live_cache.lock().unwrap_or_else(|e| e.into_inner());
        match &*cache {
            Some((at, json)) if at.elapsed() < fresh_for => json.clone(),
            _ => {
                let json =
                    serde_json::to_string(&app.live().count(Instant::now())).unwrap_or_default();
                *cache = Some((Instant::now(), json.clone()));
                json
            }
        }
    };
    json_response(&app, json, fresh_for.as_secs())
}

#[derive(serde::Deserialize)]
struct HistoryQuery {
    range: Option<String>,
    series: Option<String>,
}

/// How long a history answer is reused: a new sample comes each minute.
const HISTORY_FOR: Duration = Duration::from_secs(60);

/// `GET /v1/live/history?range=24h|7d|30d|all&series=players|official` (default 24h, players):
/// the live count, or the official server's, over time (`history::History`), made again at most
/// every minute. 400 for another range or series.
async fn live_history(
    State(app): State<Arc<App>>,
    Query(query): Query<HistoryQuery>,
    req: Request,
) -> Response {
    if !app.allow_live(app.client(req.headers(), peer(&req))) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let Some(range) = history::Range::parse(query.range.as_deref().unwrap_or("24h")) else {
        return status(StatusCode::BAD_REQUEST, "range: 24h, 7d, 30d or all");
    };
    let Some(series) = history::Series::parse(query.series.as_deref().unwrap_or("players")) else {
        return status(StatusCode::BAD_REQUEST, "series: players or official");
    };
    let cached = app
        .history_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(series, range))
        .filter(|(at, _)| at.elapsed() < HISTORY_FOR)
        .map(|(_, json)| json.clone());
    let json = match cached {
        Some(json) => json,
        None => match app.store().live_history(series, range, unix_now()) {
            Ok(history) => {
                let json = serde_json::to_string(&history).unwrap_or_default();
                app.history_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert((series, range), (Instant::now(), json.clone()));
                json
            }
            Err(e) => {
                eprintln!("horizon-telemetry-server: could not read the live history: {e}");
                return status(StatusCode::SERVICE_UNAVAILABLE, "try again later");
            }
        },
    };
    json_response(&app, json, HISTORY_FOR.as_secs())
}

fn json_response(app: &App, json: String, max_age: u64) -> Response {
    let origin =
        HeaderValue::from_str(&app.config.allow_origin).unwrap_or(HeaderValue::from_static("*"));
    let cache = HeaderValue::from_str(&format!("public, max-age={max_age}"))
        .unwrap_or(HeaderValue::from_static("no-cache"));
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, cache),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, origin),
        ],
        json,
    )
        .into_response()
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/v1/batch", post(receive))
        .route("/v1/live", post(ping).get(playing))
        .route("/v1/live/history", get(live_history))
        .route("/v1/installs/{id}", delete(forget))
        .route("/v1/aggregate", get(aggregate))
        .route("/v1/benchmarks/summary", get(benchmarks))
        .route("/hub/vote", post(hub::vote).options(hub::preflight))
        .route("/hub/votes", get(hub::votes))
        .route("/hub/report", post(intake::report).options(hub::preflight))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_rates_count_by_64() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        // Rotated inside a /64: one key. Another /64: another key.
        assert_eq!(rate_key(ip("2001:db8::1")), rate_key(ip("2001:db8::99")));
        assert_ne!(rate_key(ip("2001:db8::1")), rate_key(ip("2001:db8:1::1")));
        // IPv4-mapped counts as the IPv4 it is.
        assert_eq!(
            rate_key(ip("::ffff:203.0.113.7")),
            IpAddr::from([203, 0, 113, 7])
        );
        // Plain IPv4 unchanged.
        assert_eq!(rate_key(ip("203.0.113.8")), IpAddr::from([203, 0, 113, 8]));
    }
}
