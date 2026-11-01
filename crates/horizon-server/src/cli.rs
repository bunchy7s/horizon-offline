//! The commands that manage a server in the background (docs/server.md, section 5): `start`, `stop`,
//! `restart`, `status`, `cmd`, `report` and `logs`. They talk to the running supervisor through the control API.

use crate::{api::Client, backup, signals, supervisor};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// How long `start` waits for the game's `Done`, and `stop` for the server to end.
const START_TIMEOUT: Duration = Duration::from_secs(300);
const STOP_TIMEOUT: Duration = Duration::from_secs(180);

fn running(dir: &Path) -> Result<Client, String> {
    Client::open(dir)?.ok_or_else(|| format!("no server is running in {}", dir.display()))
}

/// `start`: `run` in the background, its output in logs/supervisor.log; returns once the game is up.
pub fn start(dir: &Path) -> Result<(), String> {
    if Client::open(dir)?.is_some() {
        return Err(format!("a server is already running in {}", dir.display()));
    }
    crate::config::Config::load(dir)?;
    // Left by a supervisor that was killed: the new one writes its own.
    let endpoint = dir.join(".horizon/api.json");
    let _ = std::fs::remove_file(&endpoint);
    let logs = dir.join("logs");
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let log_path = logs.join("supervisor.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("{}: {e}", log_path.display()))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    command
        .arg("--dir")
        .arg(dir)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    // Its own process group: closing this terminal doesn't stop the server.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        windows_sys::Win32::System::Threading::DETACHED_PROCESS
            | windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP,
    );
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + START_TIMEOUT;
    print!("Starting");
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            println!();
            return Err(format!(
                "the server stopped ({status}); see {}",
                log_path.display()
            ));
        }
        // Only once the new supervisor holds the folder (its endpoint is there) may we look: probing the
        // lock any earlier could take it from under it.
        if endpoint.exists()
            && let Ok(Some(client)) = Client::open(dir)
            && let Ok(status) = client.call("GET", "/api/v1/status", None)
            && status["phase"] == "running"
        {
            println!(
                "\nThe server is running (pid {}). `horizon-server status` shows how it does, \
                 `horizon-server stop` stops it.",
                child.id()
            );
            return Ok(());
        }
        if Instant::now() > deadline {
            println!();
            return Err(format!(
                "the game isn't up after {} s; it may still be starting: see {}",
                START_TIMEOUT.as_secs(),
                log_path.display()
            ));
        }
        print!(".");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// `stop` and `restart`.
pub fn stop(dir: &Path, restart: bool) -> Result<(), String> {
    let client = running(dir)?;
    if restart {
        let before = client.call("GET", "/api/v1/status", None)?["engine_pid"].as_u64();
        client.call("POST", "/api/v1/restart", None)?;
        print!("Restarting");
        wait_running(&client, before)?;
        println!("\nThe game is running again.");
        return Ok(());
    }
    client.call("POST", "/api/v1/stop", None)?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    while supervisor::lock(dir).is_err() {
        if Instant::now() > deadline {
            return Err("the server is still stopping".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("Stopped.");
    Ok(())
}

/// Waits until the game is up: a new one when `replaced` is the engine's pid before a restart.
fn wait_running(client: &Client, replaced: Option<u64>) -> Result<(), String> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        let status = client.call("GET", "/api/v1/status", None)?;
        if status["phase"] == "running" && status["engine_pid"].as_u64() != replaced {
            return Ok(());
        }
        if status["phase"] == "stopped" || status["phase"] == "stopping" {
            return Err("the server is stopping".into());
        }
        if Instant::now() > deadline {
            return Err(format!(
                "the game isn't up after {} s; `horizon-server logs` shows why",
                START_TIMEOUT.as_secs()
            ));
        }
        if replaced.is_some() {
            print!(".");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// `cmd`: one console command, and what it printed.
pub fn command(dir: &Path, words: &[String]) -> Result<(), String> {
    if words.is_empty() {
        return Err("cmd needs a console command, for example: horizon-server cmd list".into());
    }
    let client = running(dir)?;
    // A game that is starting or restarting takes the command once it is up.
    wait_running(&client, None)?;
    let reply = client.call(
        "POST",
        "/api/v1/command",
        Some(&json!({ "line": words.join(" ") })),
    )?;
    for line in reply["output"].as_array().into_iter().flatten() {
        println!("{}", line.as_str().unwrap_or_default());
    }
    Ok(())
}

/// `players [--json]`: who is online, the operators, the whitelist and the bans; a stopped server's lists
/// are read from its files.
pub fn players(dir: &Path, as_json: bool) -> Result<(), String> {
    let v = match Client::open(dir)? {
        Some(client) => client.call("GET", "/api/v1/players", None)?,
        None => serde_json::from_str(
            &crate::admin::players(&crate::api::Shared::with_install(dir, None)).body,
        )
        .map_err(|e| e.to_string())?,
    };
    if as_json {
        println!("{v}");
        return Ok(());
    }
    let list = |key: &str, describe: &dyn Fn(&Value) -> String| -> String {
        let items: Vec<String> = v[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(describe)
            .collect();
        if items.is_empty() {
            "none".into()
        } else {
            items.join(", ")
        }
    };
    let name = |p: &Value| p["name"].as_str().unwrap_or_default().to_owned();
    let because = |b: &Value| match b["reason"].as_str() {
        Some(reason) => format!(" ({reason})"),
        None => String::new(),
    };
    let online = list("online", &|p| match p["op_level"].as_i64() {
        Some(level) if level > 0 => format!("{} (op {level}, {} ms)", name(p), p["ping_ms"]),
        _ => format!("{} ({} ms)", name(p), p["ping_ms"]),
    });
    match v["max_players"].as_u64() {
        Some(max) => println!(
            "Online ({}/{max}): {online}",
            v["online"].as_array().map_or(0, Vec::len)
        ),
        None => println!("Online: the game isn't running"),
    }
    println!(
        "Operators: {}",
        list("ops", &|o| format!("{} ({})", name(o), o["level"]))
    );
    let white: Vec<String> = v["whitelist"]["players"]
        .as_array()
        .into_iter()
        .flatten()
        .map(name)
        .collect();
    println!(
        "Whitelist ({}): {}",
        if v["whitelist"]["enabled"] == true {
            "on"
        } else {
            "off"
        },
        if white.is_empty() {
            "none".into()
        } else {
            white.join(", ")
        }
    );
    println!(
        "Banned: {}",
        list("bans", &|b| format!("{}{}", name(b), because(b)))
    );
    println!(
        "Banned addresses: {}",
        list("ip_bans", &|b| format!(
            "{}{}",
            b["ip"].as_str().unwrap_or_default(),
            because(b)
        ))
    );
    Ok(())
}

/// `kick NAME [REASON]`, `ban NAME [REASON]`, `pardon NAME`, `ban-ip ADDRESS|NAME [REASON]`, `pardon-ip
/// ADDRESS`: the vanilla command on the running server, through the control API.
pub fn moderate(dir: &Path, command: &str, words: &[String]) -> Result<(), String> {
    let Some(target) = words.first() else {
        return Err(format!(
            "{command} needs a player's name{}",
            if command.ends_with("-ip") {
                " or an address"
            } else {
                ""
            }
        ));
    };
    let mut body = json!({});
    let key = if target.parse::<std::net::IpAddr>().is_ok() {
        "ip"
    } else {
        "name"
    };
    body[key] = json!(target);
    if words.len() > 1 {
        body["reason"] = json!(words[1..].join(" "));
    }
    let client = running(dir)?;
    wait_running(&client, None)?;
    let reply = client.call("POST", &format!("/api/v1/players/{command}"), Some(&body))?;
    for line in reply["output"].as_array().into_iter().flatten() {
        println!("{}", line.as_str().unwrap_or_default());
    }
    Ok(())
}

/// `report [SECONDS]`: the running game records a performance report for that long (60 s by default,
/// at most 600), writes it to `reports/` (a JSON file and a short text summary, nothing about who
/// plays) and this prints the summary.
pub fn report(dir: &Path, words: &[String]) -> Result<(), String> {
    let seconds = match words.first() {
        None => 60,
        Some(word) => word
            .parse::<u64>()
            .ok()
            .filter(|s| (1..=600).contains(s))
            .ok_or("report takes the seconds to record, 1 to 600 (60 by default)")?,
    };
    let client = running(dir)?;
    wait_running(&client, None)?;
    let reports = dir.join("reports");
    let before = report_files(&reports);
    let reply = client.call(
        "POST",
        "/api/v1/command",
        Some(&json!({ "line": format!("horizon-report {seconds}") })),
    )?;
    for line in reply["output"].as_array().into_iter().flatten() {
        println!("{}", line.as_str().unwrap_or_default());
    }
    println!("Recording for {seconds} s...");
    // The report is written a moment after the last recorded tick; a slow server gets a minute more.
    let deadline = Instant::now() + Duration::from_secs(seconds + 60);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        if let Some(text) = report_files(&reports)
            .into_iter()
            .find(|p| !before.contains(p))
        {
            let summary =
                std::fs::read_to_string(&text).map_err(|e| format!("{}: {e}", text.display()))?;
            print!("{summary}");
            println!("Saved: {} (and the .json beside it)", text.display());
            return Ok(());
        }
    }
    Err(format!(
        "no report in {} after {} s: is the game on a release with `horizon-report`?",
        reports.display(),
        seconds + 60
    ))
}

/// The text summaries in `reports/`.
fn report_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "txt")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("perf-"))
        })
        .collect()
}

/// `panel`: a one-time login link for the web panel.
pub fn panel(dir: &Path) -> Result<(), String> {
    let reply = running(dir)?.call("POST", "/api/v1/panel/login", None)?;
    let url = reply["url"].as_str().unwrap_or_default();
    println!(
        "Open this link in your browser (it works once, for 10 minutes):\n  {url}\n\
         From another computer: put a tailnet address in [panel] bind, or forward the port first:\n  \
         ssh -L {port}:{target} <this server>",
        port = url
            .split('/')
            .nth(2)
            .and_then(|h| h.rsplit_once(':'))
            .map_or("8130", |(_, p)| p),
        target = url.split('/').nth(2).unwrap_or("127.0.0.1:8130"),
    );
    Ok(())
}

/// `status [--json]`.
pub fn status(dir: &Path, as_json: bool) -> Result<(), String> {
    let Some(client) = Client::open(dir)? else {
        if as_json {
            println!("{}", json!({ "phase": "stopped" }));
        } else {
            println!("Stopped.");
        }
        return Ok(());
    };
    let status = client.call("GET", "/api/v1/status", None)?;
    if as_json {
        println!("{status}");
    } else {
        print!("{}", describe(&status));
    }
    Ok(())
}

/// The status for people: three short lines.
fn describe(status: &Value) -> String {
    let mut out = String::new();
    let phase = status["phase"].as_str().unwrap_or("?");
    let uptime = status["engine_uptime_seconds"]
        .as_u64()
        .or(status["uptime_seconds"].as_u64())
        .map_or_else(String::new, |s| format!(", up {}", duration(s)));
    let crashes = match status["crashes"].as_u64() {
        Some(0) | None => String::new(),
        Some(n) => format!(
            ", {n} crash{} (last: {})",
            if n == 1 { "" } else { "es" },
            status["last_crash"].as_str().unwrap_or("?")
        ),
    };
    out += &format!("{}{uptime}{crashes}\n", capitalised(phase));
    let sample = &status["sample"];
    let game = &sample["game"];
    if let Some(tps) = game["tps"].as_f64() {
        let mspt = &game["mspt"];
        out += &format!(
            "TPS {tps:.1}, MSPT {:.1} ms average, {:.1} ms P95{}\n",
            mspt["average"].as_f64().unwrap_or(0.0),
            mspt["p95"].as_f64().unwrap_or(0.0),
            if game["paused"] == true {
                " (paused: nobody online)"
            } else {
                ""
            }
        );
        let names: Vec<&str> = game["players"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["name"].as_str())
            .collect();
        out += &format!(
            "{} of {} players{}{}, {} chunks loaded\n",
            names.len(),
            game["max_players"].as_u64().unwrap_or(0),
            if names.is_empty() { "" } else { ": " },
            names.join(", "),
            game["chunks"].as_u64().unwrap_or(0)
        );
    }
    let ram = sample["rss_bytes"]
        .as_f64()
        .map(|b| format!("RAM {:.2} GB", b / 1e9));
    let cpu = sample["cpu_percent"]
        .as_f64()
        .map(|c| format!("CPU {c:.0} % of a core"));
    let usage: Vec<String> = ram.into_iter().chain(cpu).collect();
    if !usage.is_empty() {
        out += &format!("{}\n", usage.join(", "));
    }
    let update = &status["update"];
    if let Some(busy) = update["busy"].as_str() {
        out += &format!("Updating: {busy}\n");
    } else if let Some(tag) = update["available"].as_str() {
        out += &format!(
            "Horizon {tag} is out (this server runs {}): `horizon-server update` installs it\n",
            update["installed"].as_str().unwrap_or("?")
        );
    }
    out
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        3600..86_400 => format!("{} h {:02} min", secs / 3600, secs / 60 % 60),
        _ => format!("{} d {} h", secs / 86_400, secs / 3600 % 24),
    }
}

/// `backup [NAME]`: through the running server (it saves first), or straight from the files.
pub fn backup(dir: &Path, name: Option<&str>) -> Result<(), String> {
    let made = match Client::open(dir)? {
        Some(client) => client.call_within(
            "POST",
            "/api/v1/backups",
            Some(&json!({ "name": name })),
            Duration::from_secs(3600),
        )?["name"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        None => {
            let _lock = supervisor::lock(dir)?;
            let config = crate::config::Config::load(dir)?;
            let file = backup::file_name(std::time::SystemTime::now(), name)?;
            backup::create(dir, &config, &file)?;
            file
        }
    };
    println!("Backed up to {}/{made}", backup::DIR);
    Ok(())
}

/// `backups [--json]`: the backups, newest first.
pub fn backups(dir: &Path, as_json: bool) -> Result<(), String> {
    let all = backup::list(dir);
    if as_json {
        let list: Vec<Value> = all
            .iter()
            .map(|b| {
                let modified = std::fs::metadata(dir.join(backup::DIR).join(&b.name))
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                json!({ "name": b.name, "bytes": b.bytes, "modified": modified })
            })
            .collect();
        println!("{}", Value::Array(list));
        return Ok(());
    }
    if all.is_empty() {
        println!("No backups yet: `horizon-server backup` makes one.");
    }
    for b in all {
        println!("{:>10}  {}", size(b.bytes), b.name);
    }
    Ok(())
}

fn size(bytes: u64) -> String {
    match bytes {
        0..1_000_000 => format!("{} kB", bytes.div_ceil(1000)),
        1_000_000..1_000_000_000 => format!("{:.1} MB", bytes as f64 / 1e6),
        _ => format!("{:.2} GB", bytes as f64 / 1e9),
    }
}

/// `restore NAME [--yes]`: the backup's world in place of the current one (moved aside, not deleted). A
/// running server restarts with it.
pub fn restore(dir: &Path, name: &str, yes: bool) -> Result<(), String> {
    let name = backup::find(dir, name)?;
    if !yes {
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return Err("restore replaces the world: pass --yes to confirm".into());
        }
        print!(
            "Replace the world with {name}? The current world is kept in {}/. [y/N] ",
            backup::DIR
        );
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        if !answer.trim().eq_ignore_ascii_case("y") && !answer.trim().eq_ignore_ascii_case("yes") {
            println!("Nothing changed.");
            return Ok(());
        }
    }
    if let Some(client) = Client::open(dir)? {
        let before = client.call("GET", "/api/v1/status", None)?["engine_pid"].as_u64();
        client.call("POST", "/api/v1/restore", Some(&json!({ "name": name })))?;
        print!("Restoring {name}: restarting the game with it");
        wait_running(&client, before)?;
        println!(
            "\nThe game is running on the restored world; `horizon-server logs` has the details."
        );
        return Ok(());
    }
    let _lock = supervisor::lock(dir)?;
    let config = crate::config::Config::load(dir)?;
    match backup::restore(dir, &config, &name)? {
        Some(old) => println!(
            "Restored {name}; the world it replaced is in {}",
            old.display()
        ),
        None => println!("Restored {name}"),
    }
    Ok(())
}

/// `logs [-f] [-n N]`: the last lines, from the running server or else from logs/latest.log.
pub fn logs(dir: &Path, follow: bool, count: usize) -> Result<(), String> {
    let Some(client) = Client::open(dir)? else {
        if follow {
            return Err("no server is running to follow".into());
        }
        let path = dir.join("logs/latest.log");
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let lines: Vec<&str> = text.lines().collect();
        for line in &lines[lines.len().saturating_sub(count)..] {
            println!("{line}");
        }
        return Ok(());
    };
    let reply = client.call("GET", "/api/v1/logs?since=0", None)?;
    let lines = reply["lines"].as_array().cloned().unwrap_or_default();
    for line in &lines[lines.len().saturating_sub(count)..] {
        println!("{}", line.as_str().unwrap_or_default());
    }
    if !follow {
        return Ok(());
    }
    signals::install();
    let mut next = reply["next"].as_u64().unwrap_or(0);
    while signals::requests() == 0 {
        std::thread::sleep(Duration::from_millis(500));
        let Ok(reply) = client.call("GET", &format!("/api/v1/logs?since={next}"), None) else {
            break;
        };
        for line in reply["lines"].as_array().into_iter().flatten() {
            println!("{}", line.as_str().unwrap_or_default());
        }
        next = reply["next"].as_u64().unwrap_or(next);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reads_well() {
        let status = json!({
            "phase": "running",
            "engine_uptime_seconds": 7380,
            "crashes": 1,
            "last_crash": "signal: 9 (SIGKILL)",
            "sample": {
                "rss_bytes": 1_420_000_000u64,
                "cpu_percent": 12.4,
                "game": {
                    "tps": 20.0,
                    "mspt": { "average": 3.12, "p95": 7.9 },
                    "players": [{ "name": "Alex" }, { "name": "Steve" }],
                    "max_players": 20,
                    "chunks": 1234,
                    "paused": false
                }
            }
        });
        assert_eq!(
            describe(&status),
            "Running, up 2 h 03 min, 1 crash (last: signal: 9 (SIGKILL))\n\
             TPS 20.0, MSPT 3.1 ms average, 7.9 ms P95\n\
             2 of 20 players: Alex, Steve, 1234 chunks loaded\n\
             RAM 1.42 GB, CPU 12 % of a core\n"
        );
        assert_eq!(
            describe(&json!({ "phase": "starting", "uptime_seconds": 4 })),
            "Starting, up 4 s\n"
        );
        assert_eq!(
            describe(&json!({
                "phase": "running",
                "update": { "installed": "v2026.10.18", "available": "v2026.10.19" }
            })),
            "Running\nHorizon v2026.10.19 is out (this server runs v2026.10.18): `horizon-server update` installs it\n"
        );
        assert_eq!(
            describe(
                &json!({ "phase": "restarting", "update": { "busy": "Starting the game on v2" } })
            ),
            "Restarting\nUpdating: Starting the game on v2\n"
        );
    }
}
