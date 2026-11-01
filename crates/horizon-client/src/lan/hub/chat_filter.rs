//! The chat filter on the server side (docs/chat-filter.md): a player's text, before the server relays
//! it, through `horizon_chat_filter`'s list, mutes and escalation. A refused text reaches nobody; the
//! sender is told in private, and the game logs it and bans when the escalation says so
//! (`HostEvent::ChatFiltered`).
use super::{HostEvent, Hub, chat::now_millis};
use horizon_chat_filter::{Action, Guard};
use horizon_protocol::{
    packets::play::{self, Clientbound as C},
    text::literal,
};
use horizon_world::nbt::{JavaString, Tag};

/// Where a player's text was going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A chat message.
    Chat,
    /// A command's message: `/msg`, `/tell`, `/w`, `/me`, `/say`, `/teammsg`.
    Command,
    Sign,
    Anvil,
    Book,
}

impl Place {
    pub fn name(self) -> &'static str {
        match self {
            Place::Chat => "chat",
            Place::Command => "command",
            Place::Sign => "sign",
            Place::Anvil => "anvil",
            Place::Book => "book",
        }
    }
    /// Chat and commands: a muted player can't use them.
    fn is_chat(self) -> bool {
        matches!(self, Place::Chat | Place::Command)
    }
}

/// What the sender reads when their text was refused (`hit` false: refused because they are muted).
pub fn notice(place: Place, hit: bool, action: Action, now: i64) -> String {
    let minutes = |until: i64| (until - now + 59_999) / 60_000;
    let refused = match place {
        Place::Chat | Place::Command => "That message wasn't sent",
        Place::Sign => "That sign wasn't changed",
        Place::Anvil => "That name wasn't used",
        Place::Book => "That book wasn't signed",
    };
    match (hit, action) {
        (false, Action::Muted { until }) => {
            format!(
                "You're muted for {} more minutes: slurs aren't allowed here.",
                minutes(until)
            )
        }
        (_, Action::Muted { until }) => format!(
            "{refused}: slurs aren't allowed here. You're muted for {} minutes.",
            minutes(until)
        ),
        _ => format!("{refused}: slurs aren't allowed here."),
    }
}

/// The text in red.
fn red(text: &str) -> Tag {
    let mut t = literal(text);
    if let Tag::Compound(map) = &mut t {
        map.insert(
            JavaString::from("color"),
            Tag::String(JavaString::from("red")),
        );
    }
    t
}

impl Hub {
    /// `server.toml`'s `[chat_filter]` (dedicated servers); `None` turns it off.
    pub fn set_chat_filter(&mut self, guard: Option<Guard>) {
        self.chat_filter = guard.map(Box::new);
    }

    pub fn chat_filter_on(&self) -> bool {
        self.chat_filter.is_some()
    }

    /// Whether guest `i` may send `texts` (`place` says where to): true without a filter. A refusal
    /// tells the sender, counts a hit when a text holds a slur, and pushes `ChatFiltered`.
    pub fn chat_allowed(
        &mut self,
        i: usize,
        texts: &[&str],
        place: Place,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        let Some(guard) = self.chat_filter.as_deref_mut() else {
            return true;
        };
        let (uuid, now) = (self.guests[i].uuid, now_millis());
        let hit = texts.iter().any(|t| guard.filter.matches(t));
        let muted = place
            .is_chat()
            .then(|| guard.muted_until(uuid, now))
            .flatten();
        if !hit {
            let Some(until) = muted else {
                return true;
            };
            let text = notice(place, false, Action::Muted { until }, now);
            self.guests[i].send(&C::SystemChat(play::SystemChat {
                content: red(&text),
                overlay: false,
            }));
            return false;
        }
        // An anvil renames at every key typed: refused quietly, on the action bar, with no hit.
        if place == Place::Anvil {
            self.guests[i].send(&C::SystemChat(play::SystemChat {
                content: red(&notice(place, true, Action::Refused, now)),
                overlay: true,
            }));
            return false;
        }
        let name = self.guests[i].name.clone();
        let (count, action) = guard.strike(uuid, &name, now);
        if !matches!(action, Action::Banned { .. }) {
            self.guests[i].send(&C::SystemChat(play::SystemChat {
                content: red(&notice(place, true, action, now)),
                overlay: false,
            }));
        }
        events.push(HostEvent::ChatFiltered {
            sender: uuid,
            name,
            place,
            text: texts.join(" | "),
            count,
            action,
        });
        false
    }

    /// `/unmute <player>`: the name as it was muted, if they were.
    pub fn unmute(&mut self, name: &str) -> Option<String> {
        self.chat_filter.as_deref_mut()?.unmute(name)
    }

    /// The `[chat_filter]` log's path, relative to the server's folder.
    pub fn chat_filter_log(&self) -> Option<&str> {
        self.chat_filter
            .as_deref()
            .map(|g| g.config.log.as_str())
            .filter(|l| !l.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sender_is_told_why() {
        let now = 1_000_000;
        assert_eq!(
            notice(Place::Chat, true, Action::Refused, now),
            "That message wasn't sent: slurs aren't allowed here."
        );
        assert_eq!(
            notice(
                Place::Command,
                true,
                Action::Muted {
                    until: now + 600_000
                },
                now
            ),
            "That message wasn't sent: slurs aren't allowed here. You're muted for 10 minutes."
        );
        assert_eq!(
            notice(
                Place::Chat,
                false,
                Action::Muted {
                    until: now + 61_000
                },
                now
            ),
            "You're muted for 2 more minutes: slurs aren't allowed here."
        );
        assert_eq!(
            notice(Place::Sign, true, Action::Refused, now),
            "That sign wasn't changed: slurs aren't allowed here."
        );
        assert!(Place::Chat.is_chat() && Place::Command.is_chat() && !Place::Sign.is_chat());
    }
}
