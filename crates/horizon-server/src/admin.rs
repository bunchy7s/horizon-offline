//! The control API's admin endpoints (docs/api/server.md): who is online and on vanilla's lists, the
//! moderation actions, and the metrics as text for a monitoring tool.
//!
//! An action runs the vanilla console command on the running game (kick, ban, ban-ip, pardon, pardon-ip,
//! op, deop, whitelist), so it does exactly what an operator's command does, kicks included. The reply has
//! the console's lines and `changed`: whether the list file (or, for a kick, who is online) changed.

use crate::{
    api::{Phase, Shared},
    engine,
    http::Response,
};
use horizon_server_files::user_lists::{
    BanList, IpBanList, OpList, UserList, WhiteList, format_date,
};
use serde_json::{Value, json};
use std::{fmt::Write, path::Path, sync::Mutex};

/// One action at a time, so `changed` compares the files around this action only.
static ACTING: Mutex<()> = Mutex::new(());

/// The longest reason taken (vanilla's chat line is 256).
const MAX_REASON: usize = 256;

/// A moderation action: its console command and the file it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Kick,
    Ban,
    Pardon,
    BanIp,
    PardonIp,
    Op,
    Deop,
    WhitelistAdd,
    WhitelistRemove,
    Whitelist,
}

impl Action {
    /// The file whose change says the action did something (none for a kick).
    fn file(self) -> Option<&'static str> {
        match self {
            Self::Kick => None,
            Self::Ban | Self::Pardon => Some("banned-players.json"),
            Self::BanIp | Self::PardonIp => Some("banned-ips.json"),
            Self::Op | Self::Deop => Some("ops.json"),
            Self::WhitelistAdd | Self::WhitelistRemove => Some("whitelist.json"),
            Self::Whitelist => Some("server.properties"),
        }
    }

    /// The console line for a request body, or what is wrong with it.
    pub fn line(self, body: &Value) -> Result<String, String> {
        let name = || {
            let name = body["name"].as_str().unwrap_or_default();
            if (1..=16).contains(&name.chars().count())
                && name.chars().all(|c| !c.is_whitespace() && !c.is_control())
            {
                Ok(name)
            } else {
                Err(
                    "the body needs \"name\": a player's name (1 to 16 characters, no spaces)"
                        .to_owned(),
                )
            }
        };
        let ip = || {
            body["ip"]
                .as_str()
                .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok())
                .ok_or_else(|| "the body needs \"ip\": an IPv4 or IPv6 address".to_owned())
        };
        let reason = || match body["reason"].as_str().map(str::trim) {
            None | Some("") => Ok(String::new()),
            Some(r) if r.chars().count() <= MAX_REASON && !r.chars().any(char::is_control) => {
                Ok(format!(" {r}"))
            }
            Some(_) => Err(format!(
                "\"reason\" is one line of at most {MAX_REASON} characters"
            )),
        };
        Ok(match self {
            Self::Kick => format!("kick {}{}", name()?, reason()?),
            Self::Ban => format!("ban {}{}", name()?, reason()?),
            Self::Pardon => format!("pardon {}", name()?),
            // An online player's name bans the address they play from, as vanilla's /ban-ip does.
            Self::BanIp => format!("ban-ip {}{}", ip().or_else(|_| name())?, reason()?),
            Self::PardonIp => format!("pardon-ip {}", ip()?),
            Self::Op => format!("op {}", name()?),
            Self::Deop => format!("deop {}", name()?),
            Self::WhitelistAdd => format!("whitelist add {}", name()?),
            Self::WhitelistRemove => format!("whitelist remove {}", name()?),
            Self::Whitelist => match body["enabled"].as_bool() {
                Some(true) => "whitelist on".into(),
                Some(false) => "whitelist off".into(),
                None => return Err("the body needs \"enabled\": true or false".into()),
            },
        })
    }
}

fn running(shared: &Shared) -> Result<engine::Control, Response> {
    if shared.state.lock().unwrap().phase != Phase::Running {
        return Err(Response::error(409, "the game isn't running right now"));
    }
    engine::Control::connect(&shared.dir)
        .map_err(|e| Response::error(409, &format!("the game didn't answer: {e}")))
}

/// The online players the game reports (`server-status`).
fn online(game: &mut engine::Control) -> Result<Value, Response> {
    game.ask("server-status")
        .map_err(|e| Response::error(409, &format!("the game didn't answer: {e}")))
}

/// `POST /api/v1/players/<action>` and `/api/v1/whitelist[/...]`.
pub fn act(shared: &Shared, action: Action, body: &[u8]) -> Response {
    let body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let line = match action.line(&body) {
        Ok(line) => line,
        Err(e) => return Response::error(400, &e),
    };
    let mut game = match running(shared) {
        Ok(game) => game,
        Err(response) => return response,
    };
    let _one = ACTING.lock().unwrap_or_else(|e| e.into_inner());
    let read = |file: &str| std::fs::read(shared.dir.join(file)).ok();
    let before = match action.file() {
        Some(file) => read(file),
        None => {
            let status = match online(&mut game) {
                Ok(status) => status,
                Err(response) => return response,
            };
            let name = body["name"].as_str().unwrap_or_default();
            let here = status["players"].as_array().into_iter().flatten().any(|p| {
                p["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            });
            Some(vec![u8::from(here)])
        }
    };
    let reply = match game.ask(&format!("console {line}")) {
        Ok(reply) => reply,
        Err(e) => return Response::error(409, &format!("the game didn't answer: {e}")),
    };
    let changed = match action.file() {
        Some(file) => read(file) != before,
        None => before == Some(vec![1]),
    };
    crate::supervisor::note(shared, "INFO", &format!("Control API: {line}"));
    Response::json(
        200,
        &json!({ "changed": changed, "output": reply["output"] }),
    )
}

/// A list from its file in the server folder; missing or broken reads as empty, as the game would start.
fn load<E: horizon_server_files::user_lists::Entry>(dir: &Path, file: &str) -> UserList<E> {
    let mut list = UserList::new(dir.join(file));
    let _ = list.load();
    list
}

/// `GET /api/v1/players`: who is online, and vanilla's lists.
pub fn players(shared: &Shared) -> Response {
    let dir = &shared.dir;
    let game = match running(shared) {
        Ok(mut game) => online(&mut game).ok(),
        Err(_) => None,
    };
    let ops: OpList = load(dir, "ops.json");
    let ops: Vec<Value> = ops
        .entries()
        .iter()
        .map(|e| json!({ "name": e.user.name, "uuid": uuid(e.user.id), "level": e.level }))
        .collect();
    let op_level = |id: &str| {
        ops.iter()
            .find(|o| o["uuid"] == id)
            .map(|o| o["level"].clone())
    };
    let online: Vec<Value> = game
        .as_ref()
        .and_then(|g| g["players"].as_array())
        .into_iter()
        .flatten()
        .map(|p| {
            let mut p = p.clone();
            p["op_level"] = op_level(p["uuid"].as_str().unwrap_or_default()).unwrap_or(json!(0));
            p
        })
        .collect();
    let white: WhiteList = load(dir, "whitelist.json");
    let bans: BanList = load(dir, "banned-players.json");
    let ip_bans: IpBanList = load(dir, "banned-ips.json");
    let ban = |created: i64, source: &str, expires: Option<i64>, reason: &Option<String>| {
        json!({
            "created": format_date(created),
            "source": source,
            "expires": expires.map(format_date),
            "reason": reason,
        })
    };
    Response::json(
        200,
        &json!({
            "online": online,
            "max_players": game.as_ref().map(|g| g["max_players"].clone()),
            "ops": ops,
            "whitelist": {
                "enabled": whitelist_on(dir),
                "players": white.entries().iter()
                    .map(|e| json!({ "name": e.user.name, "uuid": uuid(e.user.id) }))
                    .collect::<Vec<_>>(),
            },
            "bans": bans.entries().iter().map(|b| {
                let mut v = ban(b.created, &b.source, b.expires, &b.reason);
                v["name"] = json!(b.user.name);
                v["uuid"] = json!(uuid(b.user.id));
                v
            }).collect::<Vec<_>>(),
            "ip_bans": ip_bans.entries().iter().map(|b| {
                let mut v = ban(b.created, &b.source, b.expires, &b.reason);
                v["ip"] = json!(b.user);
                v
            }).collect::<Vec<_>>(),
        }),
    )
}

/// A UUID as the lists and the game write it (8-4-4-4-12 hex digits).
fn uuid(id: u128) -> String {
    let h = format!("{id:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// Whether the whitelist is on: the game writes `white-list` to server.properties when `/whitelist on|off`
/// changes it; server.toml catches up when it stops.
fn whitelist_on(dir: &Path) -> bool {
    std::fs::read(dir.join(crate::config::PROPERTIES))
        .ok()
        .and_then(|b| horizon_server_files::java_properties::Properties::load_file(&b).ok())
        .and_then(|p| p.get("white-list").map(|v| v == "true"))
        .or_else(|| {
            crate::config::Config::load(dir)
                .ok()
                .map(|c| c.server.whitelist)
        })
        .unwrap_or(false)
}

/// `GET /api/v1/metrics/prometheus`: the latest sample in Prometheus's text format, for any monitoring
/// tool that scrapes it.
pub fn prometheus(shared: &Shared) -> Response {
    let (phase, crashes, sample) = {
        let state = shared.state.lock().unwrap();
        (state.phase, state.crashes, state.sample.clone())
    };
    let game = &sample["game"];
    let mut out = String::new();
    let mut gauge = |name: &str, help: &str, values: &[(&str, Option<f64>)]| {
        if values.iter().all(|(_, v)| v.is_none()) {
            return;
        }
        let _ = writeln!(
            out,
            "# HELP horizon_{name} {help}\n# TYPE horizon_{name} gauge"
        );
        for (labels, value) in values {
            if let Some(value) = value {
                let _ = writeln!(out, "horizon_{name}{labels} {value}");
            }
        }
    };
    let f = |v: &Value| v.as_f64();
    gauge(
        "up",
        "1 while the game runs and answers",
        &[(
            "",
            Some(f64::from(u8::from(
                phase == Phase::Running && !game.is_null(),
            ))),
        )],
    );
    gauge(
        "supervisor_uptime_seconds",
        "Seconds since horizon-server started",
        &[("", Some(shared.started.elapsed().as_secs() as f64))],
    );
    gauge(
        "crashes",
        "Game crashes since horizon-server started",
        &[("", Some(f64::from(crashes)))],
    );
    gauge(
        "tps",
        "Ticks per second over the last 5 seconds",
        &[("", f(&game["tps"]))],
    );
    gauge(
        "tps_target",
        "The tick rate the game aims for (/tick rate)",
        &[("", f(&game["target_tps"]))],
    );
    gauge(
        "mspt",
        "Milliseconds per tick: the average and the percentiles /tick query reports",
        &[
            ("{stat=\"average\"}", f(&game["mspt"]["average"])),
            ("{stat=\"p50\"}", f(&game["mspt"]["p50"])),
            ("{stat=\"p95\"}", f(&game["mspt"]["p95"])),
            ("{stat=\"p99\"}", f(&game["mspt"]["p99"])),
        ],
    );
    gauge(
        "players_online",
        "Players online",
        &[("", game["players"].as_array().map(|p| p.len() as f64))],
    );
    gauge(
        "players_max",
        "max-players",
        &[("", f(&game["max_players"]))],
    );
    gauge(
        "chunks_loaded",
        "Chunks loaded",
        &[("", f(&game["chunks"]))],
    );
    gauge(
        "memory_rss_bytes",
        "The game process's resident memory",
        &[("", f(&sample["rss_bytes"]))],
    );
    gauge(
        "cpu_percent",
        "The game process's CPU use (100 = one core)",
        &[("", f(&sample["cpu_percent"]))],
    );
    Response::text(200, "text/plain; version=0.0.4", out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_become_console_lines() {
        let line = |a: Action, body: Value| a.line(&body);
        assert_eq!(
            line(Action::Kick, json!({ "name": "Kim0" })).unwrap(),
            "kick Kim0"
        );
        assert_eq!(
            line(
                Action::Ban,
                json!({ "name": "Lou0", "reason": " cheating " })
            )
            .unwrap(),
            "ban Lou0 cheating"
        );
        assert_eq!(
            line(
                Action::BanIp,
                json!({ "ip": "203.0.113.7", "reason": "spam bots" })
            )
            .unwrap(),
            "ban-ip 203.0.113.7 spam bots"
        );
        assert_eq!(
            line(Action::BanIp, json!({ "name": "Kim0" })).unwrap(),
            "ban-ip Kim0"
        );
        assert_eq!(
            line(Action::PardonIp, json!({ "ip": "::1" })).unwrap(),
            "pardon-ip ::1"
        );
        assert_eq!(
            line(Action::WhitelistAdd, json!({ "name": "Opal" })).unwrap(),
            "whitelist add Opal"
        );
        assert_eq!(
            line(Action::Whitelist, json!({ "enabled": false })).unwrap(),
            "whitelist off"
        );
        // Nothing can add a word or a second line to the command.
        assert!(line(Action::Ban, json!({ "name": "a b" })).is_err());
        assert!(line(Action::Op, json!({ "name": "" })).is_err());
        assert!(line(Action::Op, json!({ "name": "seventeen_letters" })).is_err());
        assert!(line(Action::Kick, json!({ "name": "Kim0", "reason": "a\nstop" })).is_err());
        assert!(line(Action::PardonIp, json!({ "ip": "Kim0" })).is_err());
        assert!(line(Action::Whitelist, json!({})).is_err());
    }

    #[test]
    fn players_and_metrics_without_a_game() {
        let dir = std::env::temp_dir().join(format!("horizon-server-admin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("banned-players.json"),
            r#"[{"uuid":"00000000-0000-0000-0000-000000000001","name":"Lou0","created":"2026-10-31 12:00:00 +0000","source":"Server","expires":"forever","reason":"cheating"}]"#,
        )
        .unwrap();
        std::fs::write(dir.join("server.properties"), "white-list=true\n").unwrap();
        let shared = Shared::with_install(&dir, None);
        let players: Value = serde_json::from_str(&players(&shared).body).unwrap();
        assert_eq!(players["online"], json!([]));
        assert_eq!(players["whitelist"]["enabled"], true);
        assert_eq!(players["bans"][0]["name"], "Lou0");
        assert_eq!(players["bans"][0]["reason"], "cheating");
        assert_eq!(players["bans"][0]["expires"], Value::Null);
        let text = prometheus(&shared).body;
        assert!(
            text.contains("# TYPE horizon_up gauge\nhorizon_up 0\n"),
            "{text}"
        );
        assert!(!text.contains("horizon_tps "), "no game, no TPS: {text}");
        shared.state.lock().unwrap().sample = json!({
            "rss_bytes": 1000, "cpu_percent": 12.5,
            "game": { "tps": 20.0, "mspt": { "average": 3.5, "p50": 3.0, "p95": 6.0, "p99": 9.0 },
                      "players": [{ "name": "Kim0" }], "max_players": 20, "chunks": 441 },
        });
        let text = prometheus(&shared).body;
        for line in [
            "horizon_tps 20\n",
            "horizon_mspt{stat=\"p95\"} 6\n",
            "horizon_players_online 1\n",
            "horizon_memory_rss_bytes 1000\n",
        ] {
            assert!(text.contains(line), "{line} in {text}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
