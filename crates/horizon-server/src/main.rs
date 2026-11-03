//! `horizon-server`: host a Horizon server (docs/server.md). It keeps the server's settings in `server.toml`,
//! runs the game headless as the engine, and manages the vanilla files around it.

mod admin;
mod api;
mod backup;
mod cli;
mod config;
mod console;
mod discord;
mod engine;
mod events;
mod http;
mod init;
mod lists;
mod management;
mod management_methods;
mod management_schema;
mod management_tls;
mod mcp;
mod panel;
mod process;
mod query;
mod rcon;
#[cfg(test)]
mod reference;
mod service;
mod settings;
mod signals;
mod supervisor;
mod update;
mod websocket;

use std::{path::PathBuf, process::ExitCode};

/// This release's version: release.yml builds it with `HORIZON_VERSION` from the tag
/// (docs/versioning.md); a build from source says Cargo.toml's.
const VERSION: &str = match option_env!("HORIZON_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

const HELP: &str = "\
horizon-server: host a Horizon (Minecraft Java Edition 26.3) server

Usage: horizon-server [--dir DIR] <command>

  init        Set up a server here: the EULA, the world, the port, who may join (asks, or takes flags:
              --eula --world NAME --seed SEED --port N --whitelist/--no-whitelist --player NAME... --op NAME
              --offline --yes)
  run         Run the server in the foreground; type console commands, Ctrl-C stops it cleanly. It
              restarts the game if it crashes.
  start       Run the server in the background (its output goes to logs/supervisor.log)
  stop, restart
  status [--json]
              Up or not, TPS, MSPT, players, RAM and CPU
  cmd COMMAND...
              Run a console command on the running server and print what it answers
  report [SECONDS]
              Record a performance report for SECONDS (default 60): where the ticks' time goes,
              entities, chunks, network, memory; saved in reports/ (JSON and text, no player data)
  logs [-f] [-n N]
              The last N lines of the log (default 50); -f follows it
  console     Attach to the running server: its log as it happens, TPS, MSPT, players and RAM at the
              top, and a command line for console commands; Ctrl-D detaches
  mcp         An MCP server on stdin and stdout, for an AI agent to run and moderate the server
              (players, kick, ban, whitelist, logs, events, backups, settings, restart...)
  backup [NAME]
              Zip the world and the settings into backups/ (a running server saves first and stays up)
  backups [--json]
              List the backups
  restore BACKUP [--yes]
              Put a backup's world in place (the current one is kept in backups/); a running
              server restarts with it
  service install|uninstall|print [--system]
              Start the server at boot or login: a systemd unit on Linux (--system: for every user,
              with sudo), a launchd agent on macOS, a task at logon on Windows
  panel       A one-time login link for the web panel ([panel] in server.toml; 127.0.0.1:8130 by
              default)
  update [--check [--json]] [--version TAG] [--timeout SECONDS]
              Install the newest release (checked against GitHub's SHA-256), back up the world and
              restart on it; goes back by itself if the game isn't up within the timeout (300 s).
              --archive FILE --version NAME installs an archive you have
  rollback    Back to the release used before the update
  config [--json]
              The settings in server.toml
  config set KEY=VALUE... [--json]
              Change settings (for example world.difficulty=hard server.max_players=10), checked first;
              a running server takes the difficulty, default game mode and whitelist at once, the rest
              at its next start
  import FILE Write server.toml from a vanilla server.properties
  op NAME, deop NAME
  whitelist on|off|list, whitelist add|remove NAME
              Who may join (on the running server, or in the files when it is stopped)
  players [--json]
              Who is online, the operators, the whitelist and the bans
  kick NAME [REASON], ban NAME [REASON], pardon NAME, ban-ip ADDRESS|NAME [REASON], pardon-ip ADDRESS
              Moderate the running server (vanilla's commands, as from its console)

  --dir DIR   The server folder (default: the current folder)

Settings live in server.toml; the server writes server.properties from it before every start.
";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = PathBuf::from(".");
    if let Some(i) = args.iter().position(|a| a == "--dir") {
        if i + 1 >= args.len() {
            eprintln!("horizon-server: --dir needs a folder");
            return ExitCode::from(2);
        }
        dir = PathBuf::from(args.remove(i + 1));
        args.remove(i);
    }
    let result = match args.first().map(String::as_str) {
        None | Some("help" | "-h" | "--help") => {
            print!("{HELP}");
            Ok(())
        }
        Some("version" | "-V" | "--version") => {
            println!("horizon-server {VERSION}");
            Ok(())
        }
        Some("init") => init_answers(&args[1..]).and_then(|a| init::run(&dir, a)),
        Some("run") => supervisor::run(&dir),
        Some("start") => cli::start(&dir),
        Some("stop") => cli::stop(&dir, false),
        Some("restart") => cli::stop(&dir, true),
        Some("status") => cli::status(&dir, args[1..].iter().any(|a| a == "--json")),
        Some("cmd") => cli::command(&dir, &args[1..]),
        Some("report") => cli::report(&dir, &args[1..]),
        Some("logs") => logs(&dir, &args[1..]),
        Some("console") => console::run(&dir),
        Some("mcp") => mcp::serve(&dir),
        Some("panel") => cli::panel(&dir),
        Some("backup") => cli::backup(&dir, args.get(1).map(String::as_str)),
        Some("backups") => cli::backups(&dir, args[1..].iter().any(|a| a == "--json")),
        Some("service") => service::command(&dir, &args[1..]),
        Some("restore") => match args.get(1) {
            Some(name) => cli::restore(&dir, name, args[2..].iter().any(|a| a == "--yes")),
            None => Err("restore needs a backup (`horizon-server backups` lists them)".into()),
        },
        Some("import") => import(&dir, &args[1..]),
        Some("config") => settings::command(&dir, &args[1..]),
        Some("update") => update::command(&dir, &args[1..]),
        Some("rollback") => update::rollback(&dir, &args[1..]),
        Some(command @ ("op" | "deop" | "whitelist")) => lists::run(&dir, command, &args[1..]),
        Some("players") => cli::players(&dir, args[1..].iter().any(|a| a == "--json")),
        Some(command @ ("kick" | "ban" | "pardon" | "ban-ip" | "pardon-ip")) => {
            cli::moderate(&dir, command, &args[1..])
        }
        Some(other) => Err(format!(
            "unknown command {other}; `horizon-server help` lists them"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("horizon-server: {e}");
            ExitCode::FAILURE
        }
    }
}

fn logs(dir: &std::path::Path, args: &[String]) -> Result<(), String> {
    let mut follow = false;
    let mut count = 50;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-f" | "--follow" => follow = true,
            "-n" => {
                count = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or("-n needs a number of lines")?;
            }
            other => return Err(format!("logs: unknown option {other}")),
        }
    }
    cli::logs(dir, follow, count)
}

fn init_answers(args: &[String]) -> Result<init::Answers, String> {
    let mut answers = init::Answers::from_env()?;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--eula" => answers.eula = true,
            "--yes" | "-y" => answers.yes = true,
            "--world" => answers.world = Some(value(arg)?),
            "--seed" => answers.seed = Some(value(arg)?),
            "--port" => {
                answers.port = Some(
                    value(arg)?
                        .parse()
                        .map_err(|_| "--port needs a number from 1 to 65535")?,
                );
            }
            "--whitelist" => answers.whitelist = Some(true),
            "--no-whitelist" => answers.whitelist = Some(false),
            "--player" => answers.players.push(value(arg)?),
            "--op" => answers.op = Some(value(arg)?),
            "--offline" => answers.online = Some(false),
            other => return Err(format!("init: unknown option {other}")),
        }
    }
    Ok(answers)
}

fn import(dir: &std::path::Path, args: &[String]) -> Result<(), String> {
    let [source] = args else {
        return Err("import needs the server.properties to read".into());
    };
    let target = dir.join(config::FILE);
    if target.exists() {
        return Err(format!(
            "{} already exists; move it away first",
            target.display()
        ));
    }
    let cfg = config::Config::import(&config::read_properties(std::path::Path::new(source))?)?;
    std::fs::write(&target, config::render(&cfg)).map_err(|e| e.to_string())?;
    println!("Wrote {}", target.display());
    Ok(())
}
