//! The control API (docs/server.md, section 8): HTTP and JSON on 127.0.0.1, with a random token in
//! `.horizon/api.json` that only the server's own user can read. The CLI manages a running server through
//! it, and so does the web panel. The admin endpoints (players, moderation, text metrics) are in admin.rs.
//!
//! Its routes are [`ROUTES`], listed in docs/api/server-reference.md.

use crate::{
    admin::{self, Action},
    backup,
    config::Config,
    engine,
    http::{self, Request, Response},
    panel, settings,
    update::{self, Install},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::Write,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const ENDPOINT: &str = ".horizon/api.json";
/// The log lines kept for `logs` and the panel's console.
const LOG_LINES: usize = 2000;
/// The samples kept for the panel's graphs: 10 minutes, one a second.
const METRICS: usize = 600;

/// One endpoint of the control API: what the reference (docs/api/server-reference.md) says about it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the reference reads all of it")
)]
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    /// The JSON body or query it takes, "" for none.
    pub takes: &'static str,
    /// The JSON it answers with.
    pub answers: &'static str,
    pub what: &'static str,
}

/// Every endpoint `start` and [`route`] answer; a test checks the two agree.
pub const ROUTES: [Route; 28] = [
    Route {
        method: "GET",
        path: "/api/v1/status",
        takes: "",
        answers: "`phase`, `version`, `pid`, `uptime_seconds`, `engine_pid`, `engine_uptime_seconds`, `crashes`, `last_crash`, `sample` (TPS, MSPT, players, RAM, CPU), `update`",
        what: "How the server does",
    },
    Route {
        method: "GET",
        path: "/api/v1/metrics",
        takes: "`?since=T` (seconds since 1970)",
        answers: "`{\"samples\": [...]}`: one a second, the last 10 minutes",
        what: "The samples taken after T, for graphs",
    },
    Route {
        method: "GET",
        path: "/api/v1/logs",
        takes: "`?since=N`",
        answers: "`{\"next\": N, \"lines\": [...]}`",
        what: "The log lines after the Nth (the last 2000 are kept)",
    },
    Route {
        method: "GET",
        path: "/api/v1/events",
        takes: "`?since=N` (or `Last-Event-ID`), `?types=join,leave,kick,chat,chat_filtered,death,log,phase` (all by default)",
        answers: "with `Accept: text/event-stream`, server-sent events as they happen (`id`, `event`: the type, `data`: JSON); otherwise `{\"next\": N, \"events\": [...]}`",
        what: "Log lines, the supervisor's phase, and the game's joins, leaves, kicks, chat, messages the chat filter refused (`chat_filtered`, without their text) and deaths (the last 2000 kept)",
    },
    Route {
        method: "POST",
        path: "/api/v1/command",
        takes: "`{\"line\": \"<one command>\"}`",
        answers: "the lines the command logged",
        what: "Run a console command on the running game",
    },
    Route {
        method: "POST",
        path: "/api/v1/stop",
        takes: "",
        answers: "`{\"ok\": true}`",
        what: "Stop the game and the supervisor",
    },
    Route {
        method: "POST",
        path: "/api/v1/restart",
        takes: "",
        answers: "`{\"ok\": true}`",
        what: "Restart the game",
    },
    Route {
        method: "GET",
        path: "/api/v1/backups",
        takes: "",
        answers: "`{\"backups\": [{\"name\", \"bytes\"}]}`",
        what: "List the backups",
    },
    Route {
        method: "POST",
        path: "/api/v1/backups",
        takes: "`{\"name\": \"...\"}` (optional)",
        answers: "`{\"name\": \"<file>\"}`",
        what: "Back up now (a running game saves first and stays up)",
    },
    Route {
        method: "DELETE",
        path: "/api/v1/backups",
        takes: "`?name=<file>` (the full name `backups` lists)",
        answers: "`{\"name\": \"<file>\"}`",
        what: "Delete a backup",
    },
    Route {
        method: "GET",
        path: "/api/v1/backups/download",
        takes: "`?name=<file>`",
        answers: "the zip",
        what: "Download a backup",
    },
    Route {
        method: "POST",
        path: "/api/v1/restore",
        takes: "`{\"name\": \"<backup>\"}`",
        answers: "`{\"name\": \"<backup>\"}`",
        what: "Restart the game with that backup's world (the current one is kept in backups/)",
    },
    Route {
        method: "POST",
        path: "/api/v1/update",
        takes: "`{\"tag\": \"...\", \"timeout\": seconds}` (both optional)",
        answers: "`{\"ok\": true}`; `status` says how it went (`update`)",
        what: "Restart on that installed release, or download the newest first; goes back if the game isn't up within the timeout",
    },
    Route {
        method: "GET",
        path: "/api/v1/metrics/prometheus",
        takes: "",
        answers: "the latest sample as Prometheus text: `horizon_up`, `horizon_tps`, `horizon_mspt{stat}`, `horizon_players_online`, `horizon_memory_rss_bytes`...",
        what: "Metrics for a monitoring tool to scrape",
    },
    Route {
        method: "GET",
        path: "/api/v1/players",
        takes: "",
        answers: "`online` (name, uuid, address, ping_ms, op_level), `max_players`, `ops`, `whitelist` (`enabled`, `players`), `bans`, `ip_bans` (with reason, source, created, expires)",
        what: "Who is online, and vanilla's lists",
    },
    Route {
        method: "POST",
        path: "/api/v1/players/kick",
        takes: "`{\"name\", \"reason\"}` (reason optional)",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Kick an online player",
    },
    Route {
        method: "POST",
        path: "/api/v1/players/ban",
        takes: "`{\"name\", \"reason\"}` (reason optional)",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Ban a player (kicked if online)",
    },
    Route {
        method: "POST",
        path: "/api/v1/players/pardon",
        takes: "`{\"name\"}`",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Lift a player's ban",
    },
    Route {
        method: "POST",
        path: "/api/v1/players/ban-ip",
        takes: "`{\"ip\", \"reason\"}`, or `{\"name\"}` of an online player",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Ban an address (its players are kicked)",
    },
    Route {
        method: "POST",
        path: "/api/v1/players/pardon-ip",
        takes: "`{\"ip\"}`",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Lift an address's ban",
    },
    Route {
        method: "POST",
        path: "/api/v1/players/op",
        takes: "`{\"name\"}`",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Make a player an operator (op-permission-level)",
    },
    Route {
        method: "POST",
        path: "/api/v1/players/deop",
        takes: "`{\"name\"}`",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Make a player no longer an operator",
    },
    Route {
        method: "POST",
        path: "/api/v1/whitelist",
        takes: "`{\"enabled\": bool}`",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Turn the whitelist on or off",
    },
    Route {
        method: "POST",
        path: "/api/v1/whitelist/add",
        takes: "`{\"name\"}`",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Whitelist a player",
    },
    Route {
        method: "POST",
        path: "/api/v1/whitelist/remove",
        takes: "`{\"name\"}`",
        answers: "`{\"changed\": bool, \"output\": [...]}`: whether the list (or who is online) changed, and the console's lines",
        what: "Take a player off the whitelist (kicked if enforce-whitelist)",
    },
    Route {
        method: "GET",
        path: "/api/v1/config",
        takes: "",
        answers: "`{\"config\": {...}}`: server.toml's settings by section (`api.token` hidden)",
        what: "The settings",
    },
    Route {
        method: "PATCH",
        path: "/api/v1/config",
        takes: "`{\"changes\": {\"world.difficulty\": \"hard\", ...}}`",
        answers: "`saved`, `applied` (taken by the running game at once), `restart` (from the next start), `running`",
        what: "Change settings as `config set` does; not `engine.*`, `panel.*`, `api.*`, nor RCON, query or management keys",
    },
    Route {
        method: "POST",
        path: "/api/v1/panel/login",
        takes: "",
        answers: "`{\"url\": \"...\"}`",
        what: "A one-time login link for the web panel (with the token only, not from the panel)",
    },
];

/// What the supervisor is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Running,
    Stopping,
    /// Stopping the game to start it again (`restart`), or waiting to after a crash.
    Restarting,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Restarting => "restarting",
        }
    }
}

pub struct State {
    pub phase: Phase,
    pub engine_pid: Option<u32>,
    pub engine_started: Option<Instant>,
    pub crashes: u32,
    pub last_crash: Option<String>,
    /// The latest sample (`sample`): the game's status, RAM and CPU.
    pub sample: Value,
}

/// The log, numbered from the first line since the supervisor started.
#[derive(Default)]
pub struct Log {
    lines: VecDeque<String>,
    first: u64,
}

impl Log {
    pub fn push(&mut self, line: String) {
        if self.lines.len() == LOG_LINES {
            self.lines.pop_front();
            self.first += 1;
        }
        self.lines.push_back(line);
    }
    /// The lines after the `since`th, and the number to ask from next.
    pub fn since(&self, since: u64) -> (u64, Vec<String>) {
        let skip = since.saturating_sub(self.first) as usize;
        let next = self.first + self.lines.len() as u64;
        (next, self.lines.iter().skip(skip).cloned().collect())
    }
}

/// How long a game started on another release has to come up before the switch is undone.
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);

/// Updates and rollbacks (update.rs): what the check found, what is under way, how the last one ended.
#[derive(Default)]
pub struct Updates {
    /// A release newer than the one in use, from the last check.
    pub available: Option<String>,
    /// What is happening now, or nothing.
    pub busy: Option<String>,
    /// How the last update or rollback ended, and whether it went well.
    pub last: Option<String>,
    pub ok: bool,
    /// Counts the updates that ended, so a client can wait for the one it asked for.
    pub finished: u64,
    /// The release to restart the game on, and how long it has to come up.
    pub pending: Option<(String, Duration)>,
}

/// What the supervisor and the API threads share.
pub struct Shared {
    pub dir: PathBuf,
    pub state: Mutex<State>,
    pub log: Mutex<Log>,
    pub stop: AtomicBool,
    pub restart: AtomicBool,
    /// The backup to put in place before the game starts again (`restore`).
    pub restore: Mutex<Option<String>>,
    /// Held while a backup is written: one at a time.
    pub backing_up: Mutex<()>,
    pub started: Instant,
    /// The last samples, oldest first, for the panel's graphs.
    pub metrics: Mutex<VecDeque<Value>>,
    /// The panel's address, login links and sessions.
    pub panel: Mutex<panel::Auth>,
    /// The installer's folder this program runs from (none for a build of your own: no updates).
    pub install: Option<Install>,
    pub update: Mutex<Updates>,
    /// The event stream's events (events.rs), and the streams' wake-up.
    pub events: Mutex<crate::events::Events>,
    pub event_added: std::sync::Condvar,
}

impl Shared {
    pub fn new(dir: &Path) -> Arc<Self> {
        Self::with_install(dir, Install::find())
    }

    pub fn with_install(dir: &Path, install: Option<Install>) -> Arc<Self> {
        Arc::new(Self {
            dir: dir.to_owned(),
            state: Mutex::new(State {
                phase: Phase::Starting,
                engine_pid: None,
                engine_started: None,
                crashes: 0,
                last_crash: None,
                sample: Value::Null,
            }),
            log: Mutex::new(Log::default()),
            stop: AtomicBool::new(false),
            restart: AtomicBool::new(false),
            restore: Mutex::new(None),
            backing_up: Mutex::new(()),
            started: Instant::now(),
            metrics: Mutex::new(VecDeque::new()),
            panel: Mutex::new(panel::Auth::default()),
            install,
            update: Mutex::new(Updates::default()),
            events: Mutex::default(),
            event_added: std::sync::Condvar::new(),
        })
    }

    /// Ends an update or rollback: its outcome, in the log and in `status`.
    pub fn finish_update(&self, ok: bool, message: String) {
        {
            let mut update = self.update.lock().unwrap();
            update.busy = None;
            update.ok = ok;
            update.finished += 1;
            if ok {
                update.available = None;
            }
            update.last = Some(message.clone());
        }
        crate::supervisor::note(self, if ok { "INFO" } else { "ERROR" }, &message);
    }

    /// Asks for a restart on the installed release `tag` (the supervisor switches when the game stops).
    fn switch_to(&self, tag: String, timeout: Duration) {
        self.update.lock().unwrap().busy = Some(format!("Restarting the game on {tag}"));
        self.update.lock().unwrap().pending = Some((tag, timeout));
        self.restart.store(true, Ordering::SeqCst);
    }

    /// `POST /api/v1/update`: switches to `tag`, or downloads the newest release first.
    fn start_update(
        self: &Arc<Self>,
        tag: Option<String>,
        timeout: Duration,
    ) -> Result<(), String> {
        let install = self.install.clone().ok_or(
            "this horizon-server wasn't installed by install.sh or install.ps1: no updates",
        )?;
        {
            let mut update = self.update.lock().unwrap();
            if let Some(busy) = &update.busy {
                return Err(format!("an update is under way: {busy}"));
            }
            update.busy = Some("Looking for the newest release".into());
        }
        if let Some(tag) = tag {
            if !install.release(&tag).is_dir() || tag.contains(['/', '\\']) {
                self.update.lock().unwrap().busy = None;
                return Err(format!(
                    "{tag} isn't installed (`horizon-server update` downloads it)"
                ));
            }
            self.switch_to(tag, timeout);
            return Ok(());
        }
        let shared = self.clone();
        std::thread::Builder::new()
            .name("update".into())
            .spawn(move || {
                let fetched = (|| {
                    let current = install.current().unwrap_or_default();
                    let release = update::release(None)?;
                    if !update::update_available(&release.tag, &current) {
                        return Ok(None);
                    }
                    shared.update.lock().unwrap().busy =
                        Some(format!("Downloading {} ({})", release.tag, release.asset));
                    let archive = update::download(&install, &release)?;
                    let unpacked = update::unpack(&install, &archive, &release.tag);
                    let _ = std::fs::remove_file(&archive);
                    unpacked.map(|_| Some(release.tag))
                })();
                match fetched {
                    Ok(Some(tag)) => shared.switch_to(tag, timeout),
                    Ok(None) => shared.finish_update(true, "Already on the newest release".into()),
                    Err(e) => shared.finish_update(false, format!("Not updated: {e}")),
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Keeps one sample for the graphs.
    pub fn record(&self, sample: Value) {
        let mut metrics = self.metrics.lock().unwrap();
        if metrics.len() == METRICS {
            metrics.pop_front();
        }
        metrics.push_back(sample);
    }
    pub fn set_phase(&self, phase: Phase) {
        self.state.lock().unwrap().phase = phase;
        self.emit("phase", json!({ "phase": phase.name() }));
    }
    fn phase(&self) -> Phase {
        self.state.lock().unwrap().phase
    }

    /// A backup while the server runs: the game saves everything and stops writing while the zip is made.
    pub fn back_up(&self, file_name: &str) -> Result<PathBuf, String> {
        let _one = self.backing_up.lock().unwrap();
        let config = Config::load(&self.dir)?;
        let mut game = match self.phase() {
            Phase::Running => Some(
                engine::Control::connect(&self.dir)
                    .map_err(|e| format!("the game didn't answer: {e}"))?,
            ),
            // Between a crash and the restart nothing writes the world.
            Phase::Restarting => None,
            Phase::Starting | Phase::Stopping => {
                return Err("the game is starting or stopping: try again in a moment".into());
            }
        };
        if let Some(game) = game.as_mut() {
            for command in ["console save-off", "console save-all flush"] {
                game.ask(command)
                    .map_err(|e| format!("the game didn't save: {e}"))?;
            }
        }
        let result = backup::create(&self.dir, &config, file_name);
        if let Some(game) = game.as_mut() {
            let _ = game.ask("console save-on");
        }
        result
    }
}

fn status(shared: &Shared) -> Value {
    let state = shared.state.lock().unwrap();
    json!({
        "phase": state.phase.name(),
        "version": crate::VERSION,
        "pid": std::process::id(),
        "uptime_seconds": shared.started.elapsed().as_secs(),
        "engine_pid": state.engine_pid,
        "engine_uptime_seconds": state.engine_started.map(|t| t.elapsed().as_secs()),
        "crashes": state.crashes,
        "last_crash": state.last_crash,
        "sample": state.sample,
        "update": updates(shared),
    })
}

fn updates(shared: &Shared) -> Value {
    let Some(install) = &shared.install else {
        return Value::Null;
    };
    let update = shared.update.lock().unwrap();
    json!({
        "installed": install.current(),
        "previous": install.previous(),
        "available": update.available,
        "busy": update.busy,
        "last": update.last,
        "ok": update.ok,
        "finished": update.finished,
    })
}

/// A random token: two 64-bit hashes keyed by the OS's random hasher seeds, the time and the process.
pub fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..2u64)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            h.write_u32(std::process::id());
            format!("{:016x}", h.finish())
        })
        .collect()
}

pub fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Writes a file only its owner can read.
pub fn write_private(path: &Path, text: impl AsRef<[u8]>) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_ref())
}

/// Removes the endpoint file when the supervisor ends.
pub struct Endpoint(PathBuf);
impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Starts the API where `[api]` says (a free port of 127.0.0.1 by default) and writes where it is and its
/// token (`[api] token`, or a random one).
pub fn start(shared: Arc<Shared>) -> Result<Endpoint, String> {
    let settings = Config::load(&shared.dir)?.api;
    let listener =
        TcpListener::bind(settings.address()?).map_err(|e| format!("control API: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let token = match settings.token.trim() {
        "" => random_token(),
        token => token.to_owned(),
    };
    let public = !crate::config::private(address.ip());
    if public {
        crate::supervisor::note(
            &shared,
            "WARN",
            &format!(
                "The control API listens on {address}, beyond this computer and the tailnet (api.allow_public): \
                 only its token guards the server"
            ),
        );
    }
    let path = shared.dir.join(ENDPOINT);
    write_private(
        &path,
        json!({ "address": address.to_string(), "token": token, "pid": std::process::id() })
            .to_string(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    http::serve(listener, move |request| {
        // A page in a browser can't send the token, and a rebound DNS name can't pass as this address.
        let authorised = request
            .header("authorization")
            .and_then(|h| h.strip_prefix("Bearer "))
            .is_some_and(|t| same_token(t.trim(), &token));
        if !authorised {
            return Response::error(401, "missing or wrong token (see .horizon/api.json)");
        }
        if !public
            && !request
                .header("host")
                .is_some_and(|h| panel::host_allowed(h, address))
        {
            return Response::error(403, "wrong Host");
        }
        if (request.method.as_str(), request.path.as_str()) == ("POST", "/api/v1/panel/login") {
            return match shared.panel.lock().unwrap().login_link() {
                Some(url) => Response::json(200, &json!({ "url": url })),
                None => Response::error(
                    409,
                    "the panel is off: set [panel] bind in server.toml and restart",
                ),
            };
        }
        route(&shared, request)
    })
    .map_err(|e| e.to_string())?;
    Ok(Endpoint(path))
}

/// The API's routes, for the CLI (with the token) and the panel (with a session).
pub fn route(shared: &Arc<Shared>, request: &Request) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/v1/status") => Response::json(200, &status(shared)),
        ("GET", "/api/v1/metrics") => {
            let since: u64 = request
                .param("since")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let samples: Vec<Value> = shared
                .metrics
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m["t"].as_u64().is_some_and(|t| t > since))
                .cloned()
                .collect();
            Response::json(200, &json!({ "samples": samples }))
        }
        ("GET", "/api/v1/logs") => {
            let since = request
                .param("since")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let (next, lines) = shared.log.lock().unwrap().since(since);
            Response::json(200, &json!({ "next": next, "lines": lines }))
        }
        ("GET", "/api/v1/events") => crate::events::route(shared, request),
        ("POST", "/api/v1/command") => {
            let line = serde_json::from_slice::<Value>(&request.body)
                .ok()
                .and_then(|v| v["line"].as_str().map(str::to_owned));
            let Some(line) = line.filter(|l| !l.trim().is_empty() && !l.contains('\n')) else {
                return Response::error(400, "the body must be {\"line\": \"<one command>\"}");
            };
            if shared.state.lock().unwrap().phase != Phase::Running {
                return Response::error(409, "the game isn't running right now");
            }
            match engine::Control::connect(&shared.dir)
                .and_then(|mut c| c.ask(&format!("console {}", line.trim_start_matches('/'))))
            {
                Ok(reply) => Response::json(200, &reply),
                Err(e) => Response::error(409, &format!("the game didn't answer: {e}")),
            }
        }
        ("POST", "/api/v1/stop") => {
            shared.stop.store(true, Ordering::SeqCst);
            Response::json(200, &json!({ "ok": true }))
        }
        ("POST", "/api/v1/restart") => {
            shared.restart.store(true, Ordering::SeqCst);
            Response::json(200, &json!({ "ok": true }))
        }
        ("GET", "/api/v1/backups") => {
            let list: Vec<Value> = backup::list(&shared.dir)
                .into_iter()
                .map(|b| json!({ "name": b.name, "bytes": b.bytes }))
                .collect();
            Response::json(200, &json!({ "backups": list }))
        }
        ("POST", "/api/v1/backups") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let made = backup::file_name(std::time::SystemTime::now(), body["name"].as_str())
                .and_then(|name| shared.back_up(&name));
            match made {
                Ok(path) => Response::json(
                    200,
                    &json!({ "name": path.file_name().map(|n| n.to_string_lossy()) }),
                ),
                Err(e) => Response::error(409, &e),
            }
        }
        ("POST", "/api/v1/restore") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            match backup::find(&shared.dir, body["name"].as_str().unwrap_or_default()) {
                Ok(name) => {
                    *shared.restore.lock().unwrap() = Some(name.clone());
                    shared.restart.store(true, Ordering::SeqCst);
                    Response::json(200, &json!({ "name": name }))
                }
                Err(e) => Response::error(404, &e),
            }
        }
        ("POST", "/api/v1/update") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let timeout = body["timeout"]
                .as_u64()
                .map_or(UPDATE_TIMEOUT, Duration::from_secs);
            match shared.start_update(body["tag"].as_str().map(str::to_owned), timeout) {
                Ok(()) => Response::json(200, &json!({ "ok": true })),
                Err(e) => Response::error(409, &e),
            }
        }
        ("GET", "/api/v1/metrics/prometheus") => admin::prometheus(shared),
        ("GET", "/api/v1/players") => admin::players(shared),
        ("POST", "/api/v1/players/kick") => admin::act(shared, Action::Kick, &request.body),
        ("POST", "/api/v1/players/ban") => admin::act(shared, Action::Ban, &request.body),
        ("POST", "/api/v1/players/pardon") => admin::act(shared, Action::Pardon, &request.body),
        ("POST", "/api/v1/players/ban-ip") => admin::act(shared, Action::BanIp, &request.body),
        ("POST", "/api/v1/players/pardon-ip") => {
            admin::act(shared, Action::PardonIp, &request.body)
        }
        ("POST", "/api/v1/players/op") => admin::act(shared, Action::Op, &request.body),
        ("POST", "/api/v1/players/deop") => admin::act(shared, Action::Deop, &request.body),
        ("POST", "/api/v1/whitelist") => admin::act(shared, Action::Whitelist, &request.body),
        ("POST", "/api/v1/whitelist/add") => {
            admin::act(shared, Action::WhitelistAdd, &request.body)
        }
        ("POST", "/api/v1/whitelist/remove") => {
            admin::act(shared, Action::WhitelistRemove, &request.body)
        }
        ("DELETE", "/api/v1/backups") => {
            let name = request.param("name").unwrap_or_default();
            if !backup::list(&shared.dir).iter().any(|b| b.name == name) {
                return Response::error(404, "no such backup (`GET /api/v1/backups` lists them)");
            }
            match std::fs::remove_file(shared.dir.join(backup::DIR).join(name)) {
                Ok(()) => {
                    crate::supervisor::note(
                        shared,
                        "INFO",
                        &format!("Control API: deleted the backup {name}"),
                    );
                    Response::json(200, &json!({ "name": name }))
                }
                Err(e) => Response::error(409, &e.to_string()),
            }
        }
        ("GET", "/api/v1/backups/download") => {
            let name = request.param("name").unwrap_or_default();
            if !backup::list(&shared.dir).iter().any(|b| b.name == name) {
                return Response::error(404, "no such backup (`GET /api/v1/backups` lists them)");
            }
            Response::file(shared.dir.join(backup::DIR).join(name), name)
        }
        ("GET", "/api/v1/config") => match settings::shown(&shared.dir) {
            Ok(config) => Response::json(200, &json!({ "config": config })),
            Err(e) => Response::error(409, &e),
        },
        ("PATCH", "/api/v1/config") => patch_config(shared, &request.body),
        (_, path) if ROUTES.iter().any(|r| r.path == path) => Response::error(405, "wrong method"),
        _ => Response::error(404, "no such endpoint"),
    }
}

/// `PATCH /api/v1/config`: `config set` for a program, without the settings that change who can reach the
/// server or what program it runs.
fn patch_config(shared: &Shared, body: &[u8]) -> Response {
    let body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let Some(changes) = body["changes"].as_object().filter(|c| !c.is_empty()) else {
        return Response::error(
            400,
            "the body must be {\"changes\": {\"section.name\": value, ...}}",
        );
    };
    let mut lines = Vec::new();
    for (key, value) in changes {
        if !settings::changeable_remotely(key) {
            return Response::error(403, &format!("{key} can only be changed in server.toml"));
        }
        let text = match value {
            Value::String(s) if !s.contains(['\n', '\r']) => s.clone(),
            Value::Bool(_) | Value::Number(_) => value.to_string(),
            _ => {
                return Response::error(
                    400,
                    &format!("{key}: a value is one line of text, a number or true/false"),
                );
            }
        };
        lines.push(format!("{key}={text}"));
    }
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let live = |line: &str| {
        shared.phase() == Phase::Running
            && engine::Control::connect(&shared.dir)
                .and_then(|mut c| c.ask(&format!("console {line}")))
                .is_ok()
    };
    let running = matches!(shared.phase(), Phase::Running | Phase::Restarting);
    match settings::apply(
        &shared.dir,
        &lines,
        running.then_some(&live as &dyn Fn(&str) -> bool),
    ) {
        Ok(outcome) => {
            crate::supervisor::note(
                shared,
                "INFO",
                &format!("Control API: config set {}", lines.join(" ")),
            );
            Response::json(200, &outcome.to_json())
        }
        Err(e) => Response::error(400, &e),
    }
}

/// The API of the server running in a folder, for the CLI.
pub struct Client {
    address: SocketAddr,
    token: String,
}

impl Client {
    /// The running server's API; `None` when no server runs in `dir`.
    pub fn open(dir: &Path) -> Result<Option<Self>, String> {
        if crate::supervisor::lock(dir).is_ok() {
            return Ok(None);
        }
        // The supervisor writes the endpoint just after taking the lock: give it a moment.
        let path = dir.join(ENDPOINT);
        let deadline = Instant::now() + Duration::from_secs(5);
        let text = loop {
            match std::fs::read_to_string(&path) {
                Ok(text) => break text,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        };
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let address = v["address"]
            .as_str()
            .and_then(|a| a.parse().ok())
            .ok_or_else(|| format!("{}: no address", path.display()))?;
        let token = v["token"].as_str().unwrap_or_default().to_owned();
        Ok(Some(Self { address, token }))
    }

    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        self.call_within(method, path, body, Duration::from_secs(60))
    }

    /// Reads a server-sent event stream (`GET /api/v1/events`), one event at a time, until the server ends
    /// it (`Ok`) or stops answering.
    pub fn follow(&self, path: &str, mut each: impl FnMut(&Value)) -> Result<(), String> {
        use std::io::BufRead;
        let mut stream =
            std::net::TcpStream::connect_timeout(&self.address, Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
        // The stream sends a comment every 15 s: a minute of silence is a server gone.
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .map_err(|e| e.to_string())?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
            self.address, self.token
        )
        .map_err(|e| e.to_string())?;
        let mut lines = std::io::BufReader::new(stream).lines();
        let status = lines.next().and_then(Result::ok).unwrap_or_default();
        if status.split_whitespace().nth(1) != Some("200") {
            return Err(format!("the server answered {status:?}"));
        }
        for line in lines {
            let line = line.map_err(|e| e.to_string())?;
            if let Some(event) = line
                .strip_prefix("data: ")
                .and_then(|d| serde_json::from_str(d).ok())
            {
                each(&event);
            }
        }
        Ok(())
    }

    /// A call that may take long (a backup of a big world).
    pub fn call_within(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let (status, text) = http::request(self.address, &self.token, method, path, body, timeout)
            .map_err(|e| format!("the server didn't answer: {e}"))?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status == 200 {
            Ok(value)
        } else {
            Err(value["error"].as_str().unwrap_or(&text).to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_keeps_the_last_lines_numbered() {
        let mut log = Log::default();
        for i in 0..LOG_LINES + 10 {
            log.push(i.to_string());
        }
        let (next, lines) = log.since(0);
        assert_eq!(next, (LOG_LINES + 10) as u64);
        assert_eq!(lines.len(), LOG_LINES);
        assert_eq!(lines[0], "10");
        let (next2, tail) = log.since(next - 2);
        assert_eq!(
            (next2, tail),
            (
                next,
                vec![(LOG_LINES + 8).to_string(), (LOG_LINES + 9).to_string()]
            )
        );
    }

    #[test]
    fn the_api_wants_its_token() {
        let dir = std::env::temp_dir().join(format!("horizon-server-api-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".horizon")).unwrap();
        std::fs::write(dir.join(crate::config::FILE), crate::config::TEMPLATE).unwrap();
        let shared = Shared::new(&dir);
        let _endpoint = start(shared.clone()).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(ENDPOINT)).unwrap()).unwrap();
        let address: SocketAddr = v["address"].as_str().unwrap().parse().unwrap();
        let token = v["token"].as_str().unwrap();
        let get = |token: &str| {
            http::request(
                address,
                token,
                "GET",
                "/api/v1/status",
                None,
                Duration::from_secs(5),
            )
            .unwrap()
        };
        assert_eq!(get("nope").0, 401);
        let (status, body) = get(token);
        assert_eq!(status, 200);
        let status: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status["phase"], "starting");
        let (code, _) = http::request(
            address,
            token,
            "POST",
            "/api/v1/stop",
            None,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(code, 200);
        assert!(shared.stop.load(Ordering::SeqCst));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(ENDPOINT))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(_endpoint);
        assert!(!dir.join(ENDPOINT).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_token_and_a_port_from_server_toml() {
        let dir =
            std::env::temp_dir().join(format!("horizon-server-api-fixed-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".horizon")).unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let token = "a-token-for-the-discord-bot";
        let toml = crate::config::TEMPLATE
            .replace(
                "[api]\nbind = \"\"",
                &format!("[api]\nbind = \"127.0.0.1:{port}\""),
            )
            .replace("token = \"\"", &format!("token = \"{token}\""));
        std::fs::write(dir.join(crate::config::FILE), toml).unwrap();
        let _endpoint = start(Shared::new(&dir)).unwrap();
        let address: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let get = |token: &str| {
            http::request(
                address,
                token,
                "GET",
                "/api/v1/status",
                None,
                Duration::from_secs(5),
            )
            .unwrap()
            .0
        };
        assert_eq!((get(token), get("a-token-for-the-discord-bo")), (200, 401));
        drop(_endpoint);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_and_backups_over_the_api() {
        let dir =
            std::env::temp_dir().join(format!("horizon-server-api-admin-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        let toml = crate::config::TEMPLATE
            .replace("token = \"\"", "token = \"a-token-for-the-discord-bot\"");
        std::fs::write(dir.join(crate::config::FILE), toml).unwrap();
        std::fs::write(
            dir.join("backups/2026-10-31T12-00-00Z-manual.zip"),
            b"PK\x05\x06zip",
        )
        .unwrap();
        let shared = Shared::with_install(&dir, None);
        shared.set_phase(Phase::Restarting);
        let call = |method: &str, path: &str, query: &str, body: Value| {
            let response = route(
                &shared,
                &Request {
                    method: method.into(),
                    path: path.into(),
                    query: query.into(),
                    headers: Vec::new(),
                    body: body.to_string().into_bytes(),
                },
            );
            (
                response.status,
                serde_json::from_str::<Value>(&response.body).unwrap_or(Value::Null),
            )
        };
        let (status, config) = call("GET", "/api/v1/config", "", Value::Null);
        assert_eq!(
            (status, &config["config"]["api"]["token"]),
            (200, &json!("(set)"))
        );
        assert_eq!(config["config"]["world"]["difficulty"], "easy");
        let (status, outcome) = call(
            "PATCH",
            "/api/v1/config",
            "",
            json!({ "changes": { "world.difficulty": "hard", "server.max_players": 50 } }),
        );
        assert_eq!(status, 200, "{outcome}");
        // The game is down between a crash and its restart: both wait for its start.
        assert_eq!(
            outcome["restart"],
            json!(["server.max_players", "world.difficulty"])
        );
        let saved = crate::config::Config::load(&dir).unwrap();
        assert_eq!(
            (saved.world.difficulty.as_str(), saved.server.max_players),
            ("hard", 50)
        );
        for key in [
            "engine.path",
            "api.allow_public",
            "panel.bind",
            "properties.enable-rcon",
        ] {
            let (status, _) = call(
                "PATCH",
                "/api/v1/config",
                "",
                json!({ "changes": { key: "x" } }),
            );
            assert_eq!(status, 403, "{key}");
        }
        let (status, _) = call(
            "PATCH",
            "/api/v1/config",
            "",
            json!({ "changes": { "world.difficulty": "brutal" } }),
        );
        assert_eq!(status, 400);
        assert_eq!(
            crate::config::Config::load(&dir).unwrap().world.difficulty,
            "hard"
        );

        let name = "2026-10-31T12-00-00Z-manual.zip";
        let response = route(
            &shared,
            &Request {
                method: "GET".into(),
                path: "/api/v1/backups/download".into(),
                query: format!("name={name}"),
                headers: Vec::new(),
                body: Vec::new(),
            },
        );
        assert!(response.stream.is_some());
        assert!(response.headers.contains(&("Content-Length", "7".into())));
        let (status, _) = call(
            "DELETE",
            "/api/v1/backups",
            "name=../server.toml",
            Value::Null,
        );
        assert_eq!(status, 404);
        assert_eq!(
            call(
                "DELETE",
                "/api/v1/backups",
                &format!("name={name}"),
                Value::Null
            )
            .0,
            200
        );
        assert!(backup::list(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
