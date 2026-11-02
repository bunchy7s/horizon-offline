//! What happened in the game, for agents (`events`, `wait-for`; docs/agent-control.md): the
//! state is compared once per event loop turn, after the game ran, so watching never changes
//! what the game does.
use super::Game;
use horizon_client::Result;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

pub(super) const HELP: &[&str] = &[
    "events [--since SEQ]           what happened (screen, chat, death, respawn, item, dimension, world), oldest first",
    "wait-for screen NAME|widget ID|loaded|chat TEXT|event KIND [--timeout S] [--since SEQ]  reply once it holds (default 30 s; chat and event count from SEQ, else from now)",
];

/// Events kept for `events`.
const KEPT: usize = 256;

/// What the last turn saw.
#[derive(PartialEq)]
struct Snapshot {
    screen: &'static str,
    world: Option<String>,
    dimension: String,
    chat: u64,
    counts: BTreeMap<String, i64>,
}

enum Condition {
    Screen(String),
    Widget(String),
    Loaded,
    Chat(String),
    Event(String),
}

struct Wait {
    condition: Condition,
    /// The event sequence number when the wait started.
    since: u64,
    started: Instant,
    timeout: Duration,
    reply: Sender<String>,
}

#[derive(Default)]
pub(super) struct Events {
    /// The sequence number of the newest event (the first is 1).
    seq: u64,
    log: VecDeque<Value>,
    last: Option<Snapshot>,
    waits: Vec<Wait>,
}

impl Events {
    fn push(&mut self, tick: u64, mut event: Value) {
        self.seq += 1;
        event["seq"] = self.seq.into();
        event["tick"] = tick.into();
        if self.log.len() == KEPT {
            self.log.pop_front();
        }
        self.log.push_back(event);
    }
    /// An event from outside the once-a-turn comparison (`user-input`).
    pub(super) fn note(&mut self, tick: u64, event: Value) {
        self.push(tick, event);
    }
    /// Whether a `wait-for` is waiting: its agent is still watching.
    pub(super) fn has_waits(&self) -> bool {
        !self.waits.is_empty()
    }
    fn since(&self, seq: u64) -> impl Iterator<Item = &Value> {
        self.log
            .iter()
            .filter(move |e| e["seq"].as_u64() > Some(seq))
    }
}

/// The events between two snapshots.
fn diff(before: &Snapshot, now: &Snapshot, chat: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    if before.world != now.world {
        if let Some(name) = &before.world {
            out.push(json!({ "event": "world", "state": "closed", "name": name }));
        }
        if let Some(name) = &now.world {
            out.push(json!({ "event": "world", "state": "opened", "name": name }));
        }
    }
    if before.dimension != now.dimension && now.world.is_some() && before.world == now.world {
        out.push(json!({ "event": "dimension", "from": before.dimension, "to": now.dimension }));
    }
    if before.screen != now.screen {
        out.push(json!({ "event": "screen", "from": before.screen, "to": now.screen }));
        if now.screen == "dead" {
            out.push(json!({ "event": "death" }));
        } else if before.screen == "dead" {
            out.push(json!({ "event": "respawn" }));
        }
    }
    // Oldest first; `chat` is newest first.
    for text in chat.iter().rev() {
        out.push(json!({ "event": "chat", "text": text }));
    }
    if before.world == now.world {
        let names = before.counts.keys().chain(now.counts.keys());
        let mut seen = std::collections::BTreeSet::new();
        for item in names.filter(|n| seen.insert(*n)) {
            let delta = now.counts.get(item).unwrap_or(&0) - before.counts.get(item).unwrap_or(&0);
            if delta != 0 {
                out.push(json!({ "event": "item", "item": item, "delta": delta }));
            }
        }
    }
    out
}

/// The condition, the timeout and `--since`.
fn parse_wait(words: &[&str]) -> Result<(Condition, Duration, Option<u64>)> {
    let mut rest = Vec::new();
    let mut timeout = 30.0;
    let mut since = None;
    let mut i = 0;
    while i < words.len() {
        if words[i] == "--timeout" {
            timeout = words
                .get(i + 1)
                .and_then(|t| t.parse::<f64>().ok())
                .ok_or("--timeout takes seconds")?;
            i += 2;
        } else if words[i] == "--since" {
            since = Some(
                words
                    .get(i + 1)
                    .and_then(|t| t.parse::<u64>().ok())
                    .ok_or("--since takes an event seq")?,
            );
            i += 2;
        } else {
            rest.push(words[i]);
            i += 1;
        }
    }
    let usage =
        "wait-for screen NAME|widget ID|loaded|chat TEXT|event KIND [--timeout S] [--since SEQ]";
    let text = rest.get(1..).map(|r| r.join(" ")).unwrap_or_default();
    let need = |text: String| {
        if text.is_empty() {
            Err(usage)
        } else {
            Ok(text)
        }
    };
    let condition = match rest.first().copied() {
        Some("screen") => Condition::Screen(need(text)?),
        Some("widget") => Condition::Widget(need(text)?),
        Some("loaded") => Condition::Loaded,
        Some("chat") => Condition::Chat(need(text)?),
        Some("event") => Condition::Event(need(text)?),
        _ => return Err(usage.into()),
    };
    Ok((
        condition,
        Duration::from_secs_f64(timeout.clamp(0.0, 3600.0)),
        since,
    ))
}

impl Game<'_> {
    fn snapshot(&self) -> Snapshot {
        let world = self.title.is_none().then(|| self.level.name.clone());
        Snapshot {
            screen: self.screen_label(),
            dimension: self.dim.id().to_string(),
            // A remote client counts the server's lines itself.
            chat: self
                .remote
                .as_ref()
                .map_or(self.ui.chat.received, |r| r.chat_received),
            counts: if world.is_some() {
                self.inventory_counts()
            } else {
                BTreeMap::new()
            },
            world,
        }
    }

    pub(super) fn loaded(&self) -> bool {
        self.title.is_none()
            && self.player.is_some()
            && self.streaming.pending() + self.pending_uploads.len() == 0
    }

    /// Once per event loop turn: record what changed, answer the waits that hold.
    pub(super) fn watch_events(&mut self) {
        let now = self.snapshot();
        if let Some(before) = &self.events.last
            && *before != now
        {
            // Joining or leaving a server starts another count.
            let new_chat = now.chat.saturating_sub(before.chat).min(100) as usize;
            // Newest first; the remote client keeps its lines oldest first.
            let lines = match &self.remote {
                Some(r) => r
                    .chat
                    .iter()
                    .rev()
                    .take(new_chat)
                    .map(|(t, _)| t.clone())
                    .collect(),
                None => self.ui.chat.recent_lines(new_chat),
            };
            for event in diff(before, &now, &lines) {
                self.repro.note_event(self.ticks, &event);
                self.events.push(self.ticks, event);
            }
        }
        self.events.last = Some(now);
        if self.events.waits.is_empty() {
            return;
        }
        let waits = std::mem::take(&mut self.events.waits);
        for wait in waits {
            let done = match &wait.condition {
                Condition::Screen(name) => self.screen_label() == name,
                Condition::Widget(id) => self.ui_nodes().iter().any(|n| &n.id == id),
                Condition::Loaded => self.loaded(),
                Condition::Chat(text) => self.events.since(wait.since).any(|e| {
                    e["event"] == "chat" && e["text"].as_str().is_some_and(|t| t.contains(text))
                }),
                Condition::Event(kind) => {
                    self.events.since(wait.since).any(|e| e["event"] == **kind)
                }
            };
            let waited = wait.started.elapsed();
            let reply = if done {
                json!({ "ok": true, "waited_seconds": waited.as_secs_f64(), "screen": self.screen_label() })
            } else if waited >= wait.timeout {
                json!({
                    "error": format!(
                        "timed out after {} s; the screen is {}",
                        wait.timeout.as_secs_f64(),
                        self.screen_label()
                    ),
                    "kind": "timeout",
                })
            } else {
                self.events.waits.push(wait);
                continue;
            };
            let _ = wait.reply.send(reply.to_string());
        }
    }

    /// `events` and `wait-for`; None for other commands, Some(None) when deferred.
    pub(super) fn events_command(
        &mut self,
        words: &[&str],
        reply: &Sender<String>,
    ) -> Result<Option<Option<Value>>> {
        match words[0] {
            "events" => {
                let since = match words.get(1..) {
                    Some(["--since", n]) => n.parse().map_err(|_| "--since takes a number")?,
                    Some([]) | None => 0,
                    _ => return Err("events [--since SEQ]".into()),
                };
                let events: Vec<&Value> = self.events.since(since).collect();
                Ok(Some(Some(
                    json!({ "events": events, "last": self.events.seq }),
                )))
            }
            "wait-for" => {
                let (condition, timeout, since) = parse_wait(&words[1..])?;
                self.events.waits.push(Wait {
                    condition,
                    since: since.unwrap_or(self.events.seq),
                    started: Instant::now(),
                    timeout,
                    reply: reply.clone(),
                });
                // Answered at the end of this turn if it already holds.
                Ok(Some(None))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(screen: &'static str, chat: u64, logs: i64) -> Snapshot {
        Snapshot {
            screen,
            world: Some("w".into()),
            dimension: "minecraft:overworld".into(),
            chat,
            counts: BTreeMap::from([("minecraft:oak_log".to_owned(), logs)]),
        }
    }

    #[test]
    fn diffs_name_what_changed() {
        let events = diff(
            &snap("playing", 3, 1),
            &snap("dead", 5, 0),
            &["newest".into(), "older".into()],
        );
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| e["event"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["screen", "death", "chat", "chat", "item"]);
        assert_eq!(events[2]["text"], "older", "chat oldest first");
        assert_eq!(events[4]["delta"], -1);
        let back = diff(&snap("dead", 5, 0), &snap("playing", 5, 0), &[]);
        assert_eq!(back[1]["event"], "respawn");
        let mut closed = snap("title", 5, 0);
        closed.world = None;
        closed.counts.clear();
        let left = diff(&snap("playing", 5, 4), &closed, &[]);
        assert_eq!(
            left[0],
            json!({ "event": "world", "state": "closed", "name": "w" })
        );
        assert!(
            left.iter().all(|e| e["event"] != "item"),
            "leaving drops no items"
        );
    }

    #[test]
    fn keeps_the_newest_events_numbered() {
        let mut e = Events::default();
        for i in 0..300 {
            e.push(i, json!({ "event": "chat" }));
        }
        assert_eq!(e.log.len(), KEPT);
        assert_eq!(e.since(295).count(), 5);
        assert_eq!(e.since(0).next().unwrap()["seq"], 45);
    }

    #[test]
    fn parses_conditions_and_timeouts() {
        let (c, t, since) =
            parse_wait(&["chat", "hello", "--since", "7", "there", "--timeout", "5"]).unwrap();
        assert!(matches!(c, Condition::Chat(ref s) if s == "hello there"));
        assert_eq!((t, since), (Duration::from_secs(5), Some(7)));
        assert!(matches!(
            parse_wait(&["loaded"]).unwrap().0,
            Condition::Loaded
        ));
        assert!(parse_wait(&["screen"]).is_err());
        assert!(parse_wait(&["soon"]).is_err());
    }
}
