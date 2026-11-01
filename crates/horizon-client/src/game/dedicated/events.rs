//! What happened on the dedicated server, typed, for horizon-server's event stream (`GET /api/v1/events`,
//! docs/api/server.md): players joining, leaving and kicked, chat, and deaths. The log says the same in
//! vanilla's words; these carry the player's UUID and the parts apart, so a bot needn't parse lines.
//! `server-events SINCE` on the control endpoint answers the events after the SINCE-th.
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Mutex};

/// The events kept for a reader that polls late.
const KEPT: usize = 1000;

struct Events {
    /// The number of the next event (the first is 1).
    next: u64,
    kept: VecDeque<Value>,
}

static EVENTS: Mutex<Events> = Mutex::new(Events {
    next: 1,
    kept: VecDeque::new(),
});

/// Records one event: its kind, the player's name and UUID, and what else it says.
pub(in crate::game) fn record(kind: &str, name: &str, uuid: u128, mut fields: Value) {
    let mut events = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    fields["seq"] = events.next.into();
    fields["type"] = kind.into();
    fields["name"] = name.into();
    fields["uuid"] = horizon_protocol::crypto::uuid_string(uuid).into();
    fields["time"] = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
        .into();
    events.next += 1;
    if events.kept.len() == KEPT {
        events.kept.pop_front();
    }
    events.kept.push_back(fields);
}

/// `server-events SINCE`: the events numbered above `since`, and the number to ask from next.
pub(super) fn since(since: u64) -> Value {
    let events = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    let after: Vec<&Value> = events
        .kept
        .iter()
        .filter(|e| e["seq"].as_u64().is_some_and(|s| s > since))
        .collect();
    json!({ "next": events.next - 1, "events": after })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_numbered_and_read_from_a_number() {
        let first = since(u64::MAX)["next"].as_u64().unwrap();
        record("join", "Kim0", 1, json!({}));
        record("chat", "Kim0", 1, json!({ "message": "hi" }));
        let read = since(first);
        let events = read["events"].as_array().unwrap();
        assert_eq!(read["next"].as_u64(), Some(first + 2));
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["type"], "chat");
        assert_eq!(events[1]["message"], "hi");
        assert_eq!(events[1]["uuid"], "00000000-0000-0000-0000-000000000001");
        assert_eq!(since(first + 1)["events"].as_array().unwrap().len(), 1);
    }
}
