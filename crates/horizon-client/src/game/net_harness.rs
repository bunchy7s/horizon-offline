//! Headless multiplayer client for `tools/test_dedicated_server_parity.py`:
//! with `--server` and `HORIZON_NET_REPORT` set, the client joins without a
//! window, waits until it stands in the world with the chunks around it,
//! sends a chat message and `/list` (or the lines of `HORIZON_NET_CHAT`,
//! split by `;`, one at a time; `.delay N` sets Chat Delay instead), collects the chat it is shown, leaves and
//! writes a JSON report (position, chunks, chat lines, whether it signed its
//! chat, whether the server kept it connected throughout). With `HORIZON_NET_QUIET` (lines
//! split by `;`) it then sends them as one batch of a mod's quiet commands
//! (`horizon_protocol::quiet`, `tools/quiet_commands_e2e.py`) and waits two
//! seconds for any chat they bring.
use super::Game;
use horizon_client::Result;
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};

impl Game<'_> {
    pub(super) fn run_net_harness(&mut self, report: &Path) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut next = Instant::now();
        let mut spawned_at: Option<Instant> = None;
        let mut last_chunks = 0;
        let mut quiet_since = Instant::now();
        let mut step = 0;
        let mut step_at = Instant::now();
        let mut chat_at = 0;
        let mut position = None;
        let quiet: Vec<String> = std::env::var("HORIZON_NET_QUIET")
            .map(|v| v.split(';').map(str::to_owned).collect())
            .unwrap_or_default();
        let lines: Vec<String> = std::env::var("HORIZON_NET_CHAT")
            .map(|v| v.split(';').map(str::to_owned).collect())
            .unwrap_or_else(|_| vec!["hello from the horizon client".into(), "/list".into()]);
        let mut sent = 0;
        let mut signed = false;
        let mut chunks = 0;
        let mut error = None;
        while Instant::now() < deadline {
            self.update_world();
            if self.remote.is_none() {
                error = Some(match self.title.as_ref().map(|t| &t.mode) {
                    Some(horizon_client::title::Mode::Multiplayer(
                        horizon_client::multiplayer::Screen::Disconnected { reason, .. },
                    )) => format!("disconnected: {reason}"),
                    _ => "disconnected".to_owned(),
                });
                break;
            }
            if Instant::now() >= next {
                next += Duration::from_millis(50);
                self.tick()?;
                // Gui.tick: ChatListener.tick, which the frame runs in play.
                self.tick_chat_queue();
            }
            let now = Instant::now();
            if let Some(p) = &self.player {
                spawned_at.get_or_insert(now);
                position.get_or_insert([p.position.x, p.position.y, p.position.z]);
            }
            let count = self.world.chunks.len();
            if count != last_chunks {
                last_chunks = count;
                quiet_since = now;
            }
            // Each step waits for what it sent to be answered (its chat line,
            // the /list reply), however busy the machine, up to 15 s.
            let shown = self.remote.as_ref().map_or(0, |r| r.chat.len());
            let answered = shown > chat_at || now - step_at > Duration::from_secs(15);
            match step {
                0 if spawned_at.is_some()
                    && count > 0
                    && now - quiet_since > Duration::from_secs(3) =>
                {
                    chunks = count;
                    signed = self.remote.as_ref().is_some_and(|r| r.secure_chat.signed());
                    self.send_chat(&lines[0]);
                    sent = 1;
                    step = 2;
                    step_at = now;
                    chat_at = shown;
                }
                2 if answered && sent < lines.len() => {
                    match lines[sent].strip_prefix(".delay ") {
                        Some(delay) => self.settings.chat_delay = delay.parse().unwrap_or(0.0),
                        None => self.send_chat(&lines[sent]),
                    }
                    sent += 1;
                    step_at = now;
                    chat_at = shown;
                }
                2 if answered && !quiet.is_empty() => {
                    for data in horizon_protocol::quiet::encode("net_harness", &quiet) {
                        self.send(horizon_protocol::packets::play::Serverbound::CustomPayload(
                            horizon_protocol::packets::configuration::CustomPayload {
                                channel: horizon_protocol::quiet::CHANNEL.into(),
                                data,
                            },
                        ));
                    }
                    step = 4;
                    step_at = now;
                }
                4 if now - step_at > Duration::from_secs(2) => {
                    step = 3;
                    break;
                }
                2 if answered => {
                    step = 3;
                    break;
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if step < 3 && error.is_none() {
            error = Some(format!("timed out at step {step}"));
        }
        let chat: Vec<String> = self
            .remote
            .as_ref()
            .map(|r| r.chat.iter().map(|(t, _)| t.clone()).collect())
            .unwrap_or_default();
        if self.remote.is_some() {
            self.leave_server(None)?;
        }
        std::fs::write(
            report,
            json!({
                "position": position,
                "chunks": chunks,
                "chat": chat,
                "signed": signed,
                "error": error,
            })
            .to_string(),
        )?;
        Ok(())
    }
}
