//! `horizon-server init`: the first run (docs/server.md, section 3). It asks about the EULA, the world, the
//! port and who may join, imports an existing `server.properties`, finds the game, and writes `server.toml`.
//! Every question has a flag, so it also runs unattended (Docker, scripts).

use crate::{config, config::Config, engine};
use horizon_server_files::{
    eula,
    java_properties::now_millis,
    user_lists::{OpEntry, OpList, WhiteEntry, WhiteList},
};
use std::{
    io::{BufRead, IsTerminal, Write},
    path::Path,
};

/// The answers given as flags; what is missing is asked, or defaulted with `yes`.
#[derive(Default)]
pub struct Answers {
    pub eula: bool,
    pub world: Option<String>,
    pub seed: Option<String>,
    pub port: Option<u16>,
    pub whitelist: Option<bool>,
    pub players: Vec<String>,
    pub op: Option<String>,
    /// online-mode (true unless set): players' Minecraft accounts are checked.
    pub online: Option<bool>,
    /// Take the default for every question not answered by a flag.
    pub yes: bool,
}

impl Answers {
    /// The answers in the environment, for containers: `HORIZON_EULA=true`, `HORIZON_WORLD`,
    /// `HORIZON_SEED`, `HORIZON_PORT`, `HORIZON_WHITELIST=true|false`, `HORIZON_PLAYERS` (names separated by
    /// commas), `HORIZON_OP` and `HORIZON_ONLINE=true|false`. Flags given on the command line win.
    pub fn from_env() -> Result<Self, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let flag = |name: &str| var(name).map(|v| v.eq_ignore_ascii_case("true"));
        Ok(Self {
            eula: flag("HORIZON_EULA").unwrap_or(false),
            world: var("HORIZON_WORLD"),
            seed: var("HORIZON_SEED"),
            port: var("HORIZON_PORT")
                .map(|p| {
                    p.parse()
                        .map_err(|_| format!("HORIZON_PORT is {p:?}: use a port number"))
                })
                .transpose()?,
            whitelist: flag("HORIZON_WHITELIST"),
            players: var("HORIZON_PLAYERS")
                .map(|p| {
                    p.split([',', ' '])
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            op: var("HORIZON_OP"),
            online: flag("HORIZON_ONLINE"),
            yes: false,
        })
    }
}

struct Prompt {
    interactive: bool,
}

impl Prompt {
    fn ask(&self, question: &str, default: &str) -> String {
        if !self.interactive {
            return default.to_owned();
        }
        if default.is_empty() {
            print!("{question}: ");
        } else {
            print!("{question} [{default}]: ");
        }
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            return default.to_owned();
        }
        match line.trim() {
            "" => default.to_owned(),
            answer => answer.to_owned(),
        }
    }
    fn yes(&self, question: &str, default: bool) -> bool {
        let answer = self.ask(
            &format!("{question} ({})", if default { "Y/n" } else { "y/N" }),
            "",
        );
        match answer.to_ascii_lowercase().as_str() {
            "" => default,
            a => a.starts_with('y'),
        }
    }
}

pub fn run(dir: &Path, answers: Answers) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let toml_path = dir.join(config::FILE);
    if toml_path.exists() {
        return Err(format!(
            "{} already exists: this server is set up. Edit it, or start the server with `horizon-server run`",
            toml_path.display()
        ));
    }
    let prompt = Prompt {
        interactive: !answers.yes && std::io::stdin().is_terminal(),
    };

    // An existing vanilla server folder: its settings become server.toml.
    let properties = dir.join(config::PROPERTIES);
    let mut cfg = if properties.exists() {
        let source = config::read_properties(&properties)?;
        let cfg = Config::import(&source)?;
        let kept = dir.join("server.properties.imported");
        std::fs::copy(&properties, &kept).map_err(|e| e.to_string())?;
        println!(
            "Imported server.properties into server.toml (the original is kept as {}).",
            kept.display()
        );
        cfg
    } else {
        Config::default()
    };

    // 1. The EULA: only a yes writes eula=true.
    let eula_path = dir.join("eula.txt");
    let agreed = eula::agreed(&eula_path).unwrap_or(false);
    if !agreed {
        println!(
            "Minecraft's End User License Agreement applies to running a server: {}",
            eula::EULA_LINK
        );
        let yes = answers.eula || (prompt.interactive && prompt.yes("Do you agree to it?", false));
        if !yes {
            let _ = std::fs::write(&eula_path, eula::file(now_millis(), false));
            return Err(
                "the server can't run until you agree to the EULA: run init again and answer yes, \
                 or pass --eula"
                    .into(),
            );
        }
        std::fs::write(&eula_path, eula::file(now_millis(), true)).map_err(|e| e.to_string())?;
    }

    // 2. The world: an existing one keeps its seed.
    cfg.world.name = answers
        .world
        .unwrap_or_else(|| prompt.ask("World name", &cfg.world.name));
    if dir.join(&cfg.world.name).join("level.dat").exists() {
        println!("Using the existing world in {}.", cfg.world.name);
    } else {
        cfg.world.seed = answers
            .seed
            .unwrap_or_else(|| prompt.ask("Seed (empty for a random one)", &cfg.world.seed));
    }

    // 3. The port, checked free now rather than at the first start.
    cfg.server.port = match answers.port {
        Some(port) => port,
        None => prompt
            .ask("Port", &cfg.server.port.to_string())
            .parse()
            .map_err(|_| "the port must be a number from 1 to 65535".to_owned())?,
    };
    if std::net::TcpListener::bind(("0.0.0.0", cfg.server.port)).is_err() {
        println!(
            "Port {} is in use right now: stop what uses it, or change server.port in server.toml.",
            cfg.server.port
        );
    }

    // 4. Who may join.
    if let Some(online) = answers.online {
        cfg.server.online_mode = online;
    }
    cfg.server.whitelist = answers.whitelist.unwrap_or_else(|| {
        prompt.yes(
            "Only let in players you list (whitelist)?",
            cfg.server.whitelist,
        )
    });
    let mut players = answers.players;
    if players.is_empty() && cfg.server.whitelist {
        players = prompt
            .ask("Players to let in (names, separated by commas)", "")
            .split([',', ' '])
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .collect();
    }
    let op = answers.op.or_else(|| {
        let first = players.first()?;
        prompt
            .yes(&format!("Make {first} an operator?"), true)
            .then(|| first.clone())
    });
    add_players(dir, &players, op.as_deref(), cfg.server.online_mode)?;

    // 5. The game and its files.
    match engine::program(&cfg, dir) {
        Ok(program) => {
            println!("Game: {}", program.display());
            if let Err(e) = engine::check_starts(&program) {
                println!("Warning: {e}");
            }
            match engine::resources(&cfg, dir, Some(&program)) {
                Ok(resources) => {
                    println!("Game files: {}", resources.display());
                    if let Err(e) = engine::check_prepared(&resources) {
                        println!("Warning: {e}");
                    }
                }
                Err(e) => println!("{e}"),
            }
        }
        Err(e) => println!("{e}"),
    }

    std::fs::write(&toml_path, config::render(&cfg)).map_err(|e| e.to_string())?;
    println!(
        "Wrote {}. Start the server with: horizon-server run --dir {}",
        toml_path.display(),
        dir.display()
    );
    if !cfg.server.online_mode {
        println!(
            "Note: online mode is off, so anyone can join under any name: keep the whitelist on, or play on a \
             LAN or a tailnet."
        );
    }
    Ok(())
}

/// Adds players to the whitelist (and one as an operator) in vanilla's files, with the profiles the server
/// will see: offline ones, or in online mode their accounts.
fn add_players(
    dir: &Path,
    players: &[String],
    op: Option<&str>,
    online: bool,
) -> Result<(), String> {
    if !players.is_empty() {
        let mut white = WhiteList::new(dir.join("whitelist.json"));
        let _ = white.load();
        for name in players {
            white.add(WhiteEntry {
                user: crate::lists::account(name, online)?,
            });
        }
        println!("Whitelisted: {}", players.join(", "));
    }
    if let Some(name) = op {
        let mut ops = OpList::new(dir.join("ops.json"));
        let _ = ops.load();
        ops.add(OpEntry {
            user: crate::lists::account(name, online)?,
            level: 4,
            bypasses_player_limit: false,
        });
        println!("Operator: {name}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattended_init_writes_everything() {
        let dir = std::env::temp_dir().join(format!("horizon-server-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let answers = Answers {
            eula: true,
            world: Some("survival".into()),
            seed: Some("12345".into()),
            port: Some(25599),
            whitelist: Some(true),
            players: vec!["Alex".into(), "Steve".into()],
            op: Some("Alex".into()),
            // Offline: no account lookups in a test.
            online: Some(false),
            yes: true,
        };
        run(&dir, answers).unwrap();
        let cfg = Config::load(&dir).unwrap();
        assert_eq!(
            (
                cfg.world.name.as_str(),
                cfg.world.seed.as_str(),
                cfg.server.port
            ),
            ("survival", "12345", 25599)
        );
        assert!(eula::agreed(&dir.join("eula.txt")).unwrap());
        assert!(!cfg.server.online_mode);
        let white = std::fs::read_to_string(dir.join("whitelist.json")).unwrap();
        assert!(
            white.contains("\"Alex\"") && white.contains("\"Steve\""),
            "{white}"
        );
        let ops = std::fs::read_to_string(dir.join("ops.json")).unwrap();
        assert!(
            ops.contains("\"Alex\"") && ops.contains("\"level\": 4"),
            "{ops}"
        );
        // A second init refuses to overwrite the configuration.
        assert!(run(&dir, Answers::default()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_eula_no_server() {
        let dir = std::env::temp_dir().join(format!("horizon-server-eula-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let answers = Answers {
            yes: true,
            ..Answers::default()
        };
        assert!(run(&dir, answers).unwrap_err().contains("EULA"));
        assert!(!dir.join(config::FILE).exists());
        assert!(!eula::agreed(&dir.join("eula.txt")).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
