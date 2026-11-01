//! The dedicated server's chat filter, game side (docs/chat-filter.md): what the hub refused is
//! written to `logs/chat-filter.log` and told to horizon-server's event stream as `chat_filtered`
//! (never as `chat`), a temporary ban goes into banned-players.json, and operators lift mutes with
//! `/unmute <player>` (or `unmute` on the console).
use super::{Game, dedicated, lan::LanHost};
use horizon_chat_filter::Action;
use horizon_client::lan::hub::{HostEvent, chat_filter::Place};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_protocol::text::{literal, translatable};
use horizon_server_files::user_lists::{BanEntry, NameAndId};

/// The operator level `/unmute` needs, as `/ban`'s.
const UNMUTE_LEVEL: u8 = 3;

/// The reason a temporary ban gives at login.
const BAN_REASON: &str = "Hate speech in chat (Horizon chat filter)";

impl Game<'_> {
    /// The hub's events from a filtered text: `ChatFiltered` carried out here, the others logged.
    pub(super) fn chat_filter_events(&mut self, host: &mut LanHost, events: Vec<HostEvent>) {
        for event in events {
            match event {
                HostEvent::ChatFiltered {
                    sender,
                    name,
                    place,
                    text,
                    count,
                    action,
                } => self.chat_filtered(host, sender, &name, place, &text, count, action),
                event => self.lan_log_event(event),
            }
        }
    }

    /// One refused text: the log, the event, and a temporary ban when it came to that.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn chat_filtered(
        &mut self,
        host: &mut LanHost,
        uuid: u128,
        name: &str,
        place: Place,
        text: &str,
        count: u32,
        action: Action,
    ) {
        let now = horizon_chat_filter::now_millis();
        let summary = format!(
            "[chat-filter] Refused {name}'s {} (hit {count}: {})",
            place.name(),
            action.name()
        );
        if self.options.dedicated.is_none() {
            log::info!("{summary}");
            return;
        }
        // The text only goes to the filter's own log, for operators: never to the console or the stream.
        dedicated::log("INFO", &summary);
        if let Some(path) = host.hub.chat_filter_log() {
            let line =
                horizon_chat_filter::log_line(now, name, uuid, place.name(), count, action, text);
            if let Err(e) = horizon_chat_filter::append_log(std::path::Path::new(path), &line) {
                dedicated::log("WARN", &format!("[chat-filter] {path}: {e}"));
            }
        }
        dedicated::events::record(
            "chat_filtered",
            name,
            uuid,
            serde_json::json!({ "place": place.name(), "hit": count, "action": action.name() }),
        );
        let Action::Banned { until } = action else {
            return;
        };
        if let Some(lists) = &self.server_lists {
            lists.lock().unwrap().bans.add(BanEntry::new(
                NameAndId {
                    id: uuid,
                    name: name.to_owned(),
                },
                Some("Server".into()),
                Some(until),
                Some(BAN_REASON.into()),
            ));
        }
        let hours = (until - now + 3_599_999) / 3_600_000;
        dedicated::log(
            "INFO",
            &format!("[chat-filter] Banned {name} for {hours} hours: {BAN_REASON}"),
        );
        let reason = literal(&format!(
            "You're banned for {hours} hours: slurs aren't allowed here."
        ));
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.uuid == uuid)
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            host.hub.kick(conn, reason.clone());
            dedicated::events::record(
                "kick",
                name,
                uuid,
                serde_json::json!({ "reason": "chat filter: temporary ban" }),
            );
        }
    }

    /// The message arguments of a guest's command (`/msg`, `/me`, `/say`, `/teammsg`...) through the
    /// filter: false when it refused them.
    pub(super) fn chat_filter_command(
        &mut self,
        host: &mut LanHost,
        i: usize,
        command: &str,
    ) -> bool {
        if !host.hub.chat_filter_on() {
            return true;
        }
        let permission = host.hub.guests[i].permission;
        let arguments = self.signable_arguments(permission, command);
        if arguments.is_empty() {
            return true;
        }
        let texts: Vec<&str> = arguments.iter().map(|(_, t)| t.as_str()).collect();
        let mut events = Vec::new();
        let allowed = host
            .hub
            .chat_allowed(i, &texts, Place::Command, &mut events);
        self.chat_filter_events(host, events);
        allowed
    }

    /// A guest's text for a sign, an anvil or a book through the filter (`conn`'s guest).
    pub(super) fn chat_filter_text(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        texts: &[&str],
        place: Place,
    ) -> bool {
        if !host.hub.chat_filter_on() {
            return true;
        }
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return true;
        };
        let mut events = Vec::new();
        let allowed = host.hub.chat_allowed(i, texts, place, &mut events);
        self.chat_filter_events(host, events);
        allowed
    }

    /// `/unmute <player>` from a guest: operators of level 3 (`/ban`'s); vanilla's unknown command
    /// for the others.
    pub(super) fn guest_unmute(&mut self, host: &mut LanHost, conn: u32, words: &[&str]) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (permission, by) = (g.permission, g.name.clone());
        let reply = if permission < UNMUTE_LEVEL {
            translatable("command.unknown.command", &[])
        } else if words.len() != 2 {
            literal("/unmute <player>")
        } else {
            literal(&self.unmute(host, words[1], &by))
        };
        host.hub.send_to(
            conn,
            &C::SystemChat(play::SystemChat {
                content: reply,
                overlay: false,
            }),
        );
    }

    /// Lifts `name`'s mute, logged; what to answer.
    pub(super) fn unmute(&mut self, host: &mut LanHost, name: &str, by: &str) -> String {
        match host.hub.unmute(name) {
            Some(name) => {
                if self.options.dedicated.is_some() {
                    dedicated::log("INFO", &format!("[chat-filter] {by} unmuted {name}"));
                }
                format!("Unmuted {name}")
            }
            None => format!("{name} isn't muted"),
        }
    }
}
