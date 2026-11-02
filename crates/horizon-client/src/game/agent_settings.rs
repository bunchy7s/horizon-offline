//! Agent control as a setting (Horizon Settings > AI Agent): when the control socket is open, who
//! is connected, and the details an agent needs. The socket opens when the game was started for
//! an agent (`--control`, `--control-socket`: horizon-ctl launch, the launcher's MCP `play`) or
//! when the player turned Agent Control on, and never under No AI (`horizon_core::no_ai`).
//! Turning either switch in the game opens or closes it at once.
use super::Game;
use horizon_client::{Result, control};
use std::path::PathBuf;

impl Game<'_> {
    /// The endpoint: the one the command line gave, else the default horizon-ctl looks for.
    fn agent_socket_path(&self) -> PathBuf {
        self.options
            .control
            .clone()
            .unwrap_or_else(control::default_path)
    }

    /// At startup: open the socket if the game was started for an agent or Agent Control is on.
    pub(super) fn start_agent_control(&mut self) -> Result<()> {
        self.agent_control_on = self.settings.extras.agent_control;
        if self.options.benchmark.is_some() && self.options.control.is_none() {
            return Ok(());
        }
        if self.settings.extras.no_ai {
            if self.options.control.is_some() {
                log::warn!(
                    "Horizon: control socket not opened: {}",
                    horizon_core::no_ai::REFUSAL
                );
            }
            return Ok(());
        }
        if let Some(path) = &self.options.control {
            // Asked for on the command line: failing to listen is an error.
            let server = control::Server::start(path)
                .map_err(|e| format!("control socket {}: {e}", path.display()))?;
            log::info!("Horizon: control socket {}", server.path().display());
            self.control = Some(server);
        } else if self.agent_control_on {
            self.open_agent_socket();
        }
        Ok(())
    }

    /// The setting's socket: a failure (another game holds the default endpoint) is logged and
    /// shown on the AI Agent page, not fatal.
    fn open_agent_socket(&mut self) {
        let path = self.agent_socket_path();
        match control::Server::start(&path) {
            Ok(server) => {
                log::info!("Horizon: control socket {}", server.path().display());
                self.control = Some(server);
            }
            Err(e) => log::warn!("Horizon: control socket {}: {e}", path.display()),
        }
    }

    /// After a settings change: No AI closes the socket; Agent Control turned on or off opens or
    /// closes it.
    pub(super) fn sync_agent_control(&mut self) {
        let wanted = self.settings.extras.agent_control;
        let changed = wanted != self.agent_control_on;
        self.agent_control_on = wanted;
        let open = self
            .control
            .as_ref()
            .is_some_and(|c| !c.path().as_os_str().is_empty());
        if self.settings.extras.no_ai || (changed && !wanted) {
            if open {
                log::info!("Horizon: control socket closed");
                self.control = None;
            }
        } else if changed && wanted && self.control.is_none() {
            self.open_agent_socket();
        }
    }

    /// An agent's command arrived.
    pub(super) fn agent_command_seen(&mut self) {
        self.agent_last = Some(std::time::Instant::now());
    }

    /// The AI Agent page's lines: the state, then where agents connect.
    pub(super) fn agent_status_lines(&self) -> Vec<String> {
        let server = self
            .control
            .as_ref()
            .filter(|c| !c.path().as_os_str().is_empty())
            .map(|s| Socket {
                clients: s.clients(),
                name: s
                    .path()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            });
        let mut lines = status_lines(
            server,
            self.agent_last.map(|t| t.elapsed().as_secs()),
            self.options.control.is_some() && !self.agent_control_on,
        );
        if let Some(line) = self.repro.agent_line(self.input_trace.events()) {
            lines.push(line);
        }
        lines
    }

    /// What Copy Connection Details puts on the clipboard: the socket, a horizon-ctl line to try,
    /// and an MCP client's config for this game.
    pub(super) fn agent_details(&self) -> String {
        let socket = self.agent_socket_path();
        let ctl = std::env::current_exe()
            .unwrap_or_default()
            .with_file_name(if cfg!(windows) {
                "horizon-ctl.exe"
            } else {
                "horizon-ctl"
            });
        let config = serde_json::json!({ "mcpServers": { "horizon-game": {
            "command": ctl, "args": ["--socket", socket, "mcp"] } } });
        format!(
            "Horizon agent control (this computer only)\nSocket: {}\nTry it: \"{}\" --socket \"{}\" state\nMCP config:\n{}\n",
            socket.display(),
            ctl.display(),
            socket.display(),
            serde_json::to_string_pretty(&config).unwrap_or_default()
        )
    }
}

/// The control socket as the AI Agent page tells of it.
struct Socket {
    /// Agents connected now.
    clients: usize,
    /// The socket's file name; the full path is too wide for the page (Copy Connection Details
    /// has it).
    name: String,
}

/// The AI Agent page's lines in the selected language: the state, then where agents connect.
/// `last_command` is the seconds since an agent's last command, `session_only` whether the
/// program that started the game opened the socket and the player's switch did not.
fn status_lines(
    server: Option<Socket>,
    last_command: Option<u64>,
    session_only: bool,
) -> Vec<String> {
    use horizon_client::lang::{get, tr};
    let mut lines = vec![match &server {
        Some(s) if s.clients > 0 => {
            let (one, many) = match last_command {
                Some(_) => ("one_ago", "many_ago"),
                None => ("one", "many"),
            };
            let key = format!(
                "horizon.agent.connected.{}",
                if s.clients == 1 { one } else { many }
            );
            let mut args: Vec<&dyn std::fmt::Display> = Vec::new();
            if s.clients != 1 {
                args.push(&s.clients);
            }
            if let Some(ago) = &last_command {
                args.push(ago);
            }
            tr(&key, &args)
        }
        Some(_) => get("horizon.agent.waiting"),
        None => get("horizon.agent.off"),
    }];
    if let Some(s) = &server {
        if session_only {
            lines.push(get("horizon.agent.session_opened"));
        }
        lines.push(tr("horizon.agent.socket", &[&s.name]));
    }
    lines.push(get("horizon.agent.local_only"));
    lines
}

#[cfg(test)]
mod tests {
    use super::{Socket, status_lines};
    use horizon_client::lang::{Language, scoped};

    fn lines(code: &str, clients: usize, ago: Option<u64>, session_only: bool) -> Vec<String> {
        let socket = Socket {
            clients,
            name: "ctl.sock".into(),
        };
        let (lines, missed) = scoped(Language::load(&[], code), || {
            status_lines(Some(socket), ago, session_only)
        });
        assert!(missed.is_empty(), "{code}: {missed:?}");
        lines
    }

    /// The AI Agent page told its state in English in every language (#1927).
    #[test]
    fn agent_page_lines_follow_the_language() {
        assert_eq!(
            lines("de_de", 2, Some(5), false),
            [
                "Verbunden: 2 Agentenverbindungen, letzter Befehl vor 5 s",
                "Socket: ctl.sock (Verbindungsdaten kopieren für den vollständigen Pfad)",
                "Nur lokal: Agenten auf diesem Computer, nie über das Netzwerk.",
            ]
        );
        assert_eq!(
            lines("pl_pl", 1, None, true),
            [
                "Połączono: 1 połączenie agenta",
                "Otwarte na tę sesję przez program, który uruchomił grę.",
                "Gniazdo: ctl.sock (skopiuj dane, aby poznać pełną ścieżkę)",
                "Tylko lokalnie: agenci na tym komputerze, nigdy przez sieć.",
            ]
        );
        let (off, missed) = scoped(Language::load(&[], "fr_fr"), || {
            status_lines(None, None, false)
        });
        assert!(missed.is_empty(), "{missed:?}");
        assert_eq!(
            off,
            [
                "Désactivé : les agents ne peuvent pas joindre ce jeu.",
                "Local uniquement : agents sur cet ordinateur, jamais via le réseau.",
            ]
        );
    }

    /// English keeps the sentences it always had: the count, its plural, the time since the last
    /// command, and the wait for a first agent.
    #[test]
    fn agent_page_lines_in_english() {
        assert_eq!(
            lines("en_us", 2, Some(5), false)[0],
            "Connected: 2 agent connections, last command 5 s ago"
        );
        assert_eq!(
            lines("en_us", 1, None, false)[0],
            "Connected: 1 agent connection"
        );
        assert_eq!(
            lines("en_us", 1, Some(0), false)[0],
            "Connected: 1 agent connection, last command 0 s ago"
        );
        assert_eq!(
            lines("en_us", 3, None, false)[0],
            "Connected: 3 agent connections"
        );
        assert_eq!(lines("en_us", 0, None, true)[0], "Waiting for an agent.");
    }
}
