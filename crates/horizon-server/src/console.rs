//! `horizon-server console` (docs/server.md, section 5): attach to the running server. The recent log, then
//! every new line as it happens (the control API's event stream), a header with the TPS, MSPT, players and
//! RAM refreshed every second, and a command line: each line typed runs as a console command. Ctrl-D
//! detaches and leaves the server running. Plain ANSI, no TUI framework; when stdout isn't a terminal, plain
//! lines without the header.

use crate::api::Client;
use serde_json::{Value, json};
use std::{
    io::{BufRead, IsTerminal, Write},
    path::Path,
    sync::Arc,
    thread,
    time::Duration,
};

/// The log lines shown on attaching.
const RECENT: usize = 30;

pub fn run(dir: &Path) -> Result<(), String> {
    let client = Arc::new(
        Client::open(dir)?.ok_or_else(|| format!("no server is running in {}", dir.display()))?,
    );
    let recent = client.call("GET", "/api/v1/events?since=0&types=log", None)?;
    let lines: Vec<&str> = recent["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["line"].as_str())
        .collect();
    let header = std::io::stdout()
        .is_terminal()
        .then(terminal_rows)
        .flatten();
    if let Some(rows) = header {
        // Line 1 is the header; the log scrolls below it.
        print!("\x1b[2J\x1b[2;{rows}r\x1b[{rows};1H");
    }
    for line in &lines[lines.len().saturating_sub(RECENT)..] {
        println!("{line}");
    }
    println!("Attached: type console commands (`list`, `say hi`, `ban NAME`...); Ctrl-D detaches.");
    let since = recent["next"].as_u64().unwrap_or(0);
    let follower = client.clone();
    thread::spawn(move || {
        let path = format!("/api/v1/events?since={since}&types=log");
        let ended = follower.follow(&path, |event| {
            if let Some(line) = event["line"].as_str() {
                println!("{line}");
            }
        });
        detach(header.is_some());
        match ended {
            Ok(()) => println!("The server stopped."),
            Err(e) => println!("Lost the server: {e}"),
        }
        std::process::exit(0);
    });
    if header.is_some() {
        let status = client.clone();
        thread::spawn(move || {
            loop {
                if let Ok(s) = status.call("GET", "/api/v1/status", None) {
                    // Save the cursor, write line 1, put the cursor back.
                    print!(
                        "\x1b7\x1b[1;1H\x1b[2K\x1b[7m{}\x1b[0m\x1b8",
                        header_line(&s)
                    );
                    let _ = std::io::stdout().flush();
                }
                thread::sleep(Duration::from_secs(1));
            }
        });
    }
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // The game logs what the command says, so it shows in the log above.
        if let Err(e) = client.call("POST", "/api/v1/command", Some(&json!({ "line": line }))) {
            println!("{e}");
        }
    }
    detach(header.is_some());
    println!("Detached; the server keeps running.");
    Ok(())
}

/// Gives the terminal its whole height back.
fn detach(header: bool) {
    if header {
        print!("\x1b[r\x1b[999;1H");
    }
    let _ = std::io::stdout().flush();
}

/// `running · 20.0 TPS · 3.2 ms · 2/20 players · 512 MB`.
pub fn header_line(status: &Value) -> String {
    let sample = &status["sample"];
    let game = &sample["game"];
    let mut parts = vec![format!(
        "Horizon server {} · {}",
        status["version"].as_str().unwrap_or_default(),
        status["phase"].as_str().unwrap_or("?")
    )];
    if let Some(tps) = game["tps"].as_f64() {
        parts.push(format!("{tps:.1} TPS"));
    }
    if let Some(mspt) = game["mspt"]["average"].as_f64() {
        parts.push(format!("{mspt:.1} ms/tick"));
    }
    if let Some(players) = game["players"].as_array() {
        parts.push(format!(
            "{}/{} players",
            players.len(),
            game["max_players"].as_u64().unwrap_or(0)
        ));
    }
    if let Some(rss) = sample["rss_bytes"].as_u64() {
        parts.push(format!("{} MB", rss >> 20));
    }
    parts.join(" · ")
}

/// The terminal's height, for the scroll region under the header.
#[cfg(unix)]
fn terminal_rows() -> Option<u16> {
    // SAFETY: TIOCGWINSZ fills a winsize, which is zeroed plain data.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    (ok && size.ws_row > 2).then_some(size.ws_row)
}

#[cfg(not(unix))]
fn terminal_rows() -> Option<u16> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_says_how_the_server_does() {
        let status = json!({
            "version": "0.1.0", "phase": "running",
            "sample": { "rss_bytes": 536_870_912u64, "game": {
                "tps": 19.96, "mspt": { "average": 3.24 }, "players": [{}, {}], "max_players": 20,
            } },
        });
        assert_eq!(
            header_line(&status),
            "Horizon server 0.1.0 · running · 20.0 TPS · 3.2 ms/tick · 2/20 players · 512 MB"
        );
        assert_eq!(
            header_line(&json!({ "version": "0.1.0", "phase": "restarting", "sample": null })),
            "Horizon server 0.1.0 · restarting"
        );
    }
}
