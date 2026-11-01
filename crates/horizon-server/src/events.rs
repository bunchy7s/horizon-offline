//! The event stream (`GET /api/v1/events`, docs/api/server.md): every log line, the supervisor's phase, and
//! the game's typed events (players joining, leaving and kicked, chat, deaths), numbered in one sequence. A
//! client that asks for `text/event-stream` gets server-sent events as they happen, and resumes after a
//! reconnection from `Last-Event-ID`; any other gets the events after `since` as JSON.

use crate::{
    api::{Phase, Shared},
    engine,
    http::{Request, Response},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::Write,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

/// The events kept for a client that reconnects or polls.
const KEPT: usize = 2000;
/// How often the game's events are fetched.
const POLL: Duration = Duration::from_millis(250);
/// A comment this often keeps proxies from closing a quiet stream, and finds clients that left.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The events, numbered from 1 since the supervisor started.
#[derive(Default)]
pub struct Events {
    kept: VecDeque<Value>,
    last: u64,
}

impl Events {
    /// The events numbered above `since` whose type `wanted` takes, and the newest number.
    pub(crate) fn after(&self, since: u64, wanted: &dyn Fn(&str) -> bool) -> (u64, Vec<Value>) {
        let skip = self
            .kept
            .len()
            .saturating_sub((self.last.saturating_sub(since)) as usize);
        let events = self
            .kept
            .iter()
            .skip(skip)
            .filter(|e| wanted(e["type"].as_str().unwrap_or_default()))
            .cloned()
            .collect();
        (self.last, events)
    }
}

impl Shared {
    /// Adds an event of type `kind` and wakes the streams.
    pub fn emit(&self, kind: &str, mut event: Value) {
        let mut events = self.events.lock().unwrap();
        events.last += 1;
        event["id"] = events.last.into();
        event["type"] = kind.into();
        if events.kept.len() == KEPT {
            events.kept.pop_front();
        }
        events.kept.push_back(event);
        drop(events);
        self.event_added.notify_all();
    }

    /// A log line, for `logs` and as a `log` event.
    pub fn log_line(&self, line: String) {
        self.emit("log", json!({ "line": line }));
        self.log.lock().unwrap().push(line);
    }
}

/// Fetches the game's typed events (`server-events` on its control endpoint) while it runs.
pub fn poll_game(shared: Arc<Shared>) {
    let _ = thread::Builder::new().name("events".into()).spawn(move || {
        let mut control: Option<(u32, engine::Control)> = None;
        let mut since = 0u64;
        loop {
            thread::sleep(POLL);
            let (pid, running) = {
                let state = shared.state.lock().unwrap();
                (state.engine_pid, state.phase == Phase::Running)
            };
            let (Some(pid), true) = (pid, running) else {
                control = None;
                continue;
            };
            // A new game numbers its events from 1 again.
            if control.as_ref().is_none_or(|(p, _)| *p != pid) {
                control = engine::Control::connect(&shared.dir).ok().map(|c| (pid, c));
                since = 0;
            }
            let Some((_, game)) = control.as_mut() else {
                continue;
            };
            let Ok(reply) = game.ask(&format!("server-events {since}")) else {
                control = None;
                continue;
            };
            for event in reply["events"].as_array().into_iter().flatten() {
                let mut event = event.clone();
                since = since.max(event["seq"].as_u64().unwrap_or(since));
                let kind = event["type"].as_str().unwrap_or("game").to_owned();
                if let Some(fields) = event.as_object_mut() {
                    fields.remove("seq");
                }
                shared.emit(&kind, event);
            }
        }
    });
}

/// `GET /api/v1/events[?since=N][&types=join,chat,...]`.
pub fn route(shared: &Arc<Shared>, request: &Request) -> Response {
    let types: Vec<String> = request
        .param("types")
        .map(|t| t.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let wanted = move |kind: &str| types.is_empty() || types.iter().any(|t| t == kind);
    let since = request
        .param("since")
        .or_else(|| request.header("last-event-id"))
        .and_then(|s| s.parse().ok());
    let streamed = request
        .header("accept")
        .is_some_and(|a| a.contains("text/event-stream"));
    if !streamed {
        let (last, events) = shared
            .events
            .lock()
            .unwrap()
            .after(since.unwrap_or(0), &wanted);
        return Response::json(200, &json!({ "next": last, "events": events }));
    }
    // Without a number, the stream starts with the request: what happens while the reply's headers go out
    // is sent.
    let mut cursor = since.unwrap_or_else(|| shared.events.lock().unwrap().last);
    let shared = shared.clone();
    let mut response = Response::text(200, "text/event-stream", "");
    response.stream = Some(Box::new(move |mut stream| {
        let mut quiet_since = Instant::now();
        loop {
            let events = {
                let events = shared.events.lock().unwrap();
                let events = if events.last == cursor {
                    shared
                        .event_added
                        .wait_timeout(events, KEEP_ALIVE)
                        .unwrap()
                        .0
                } else {
                    events
                };
                let (last, after) = events.after(cursor, &wanted);
                cursor = last;
                after
            };
            let mut text = String::new();
            for event in &events {
                text += &format!(
                    "id: {}\nevent: {}\ndata: {event}\n\n",
                    event["id"],
                    event["type"].as_str().unwrap_or_default()
                );
            }
            if text.is_empty() && quiet_since.elapsed() >= KEEP_ALIVE {
                text = ": keep-alive\n\n".into();
            }
            if !text.is_empty() {
                quiet_since = Instant::now();
                if stream
                    .write_all(text.as_bytes())
                    .and_then(|()| stream.flush())
                    .is_err()
                {
                    return;
                }
            }
        }
    }));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_after_a_number_by_type() {
        let mut events = Events::default();
        for i in 0..KEPT + 5 {
            events.last += 1;
            if events.kept.len() == KEPT {
                events.kept.pop_front();
            }
            let kind = if i % 2 == 0 { "log" } else { "chat" };
            events
                .kept
                .push_back(json!({ "id": events.last, "type": kind }));
        }
        let all = |_: &str| true;
        let (last, after) = events.after(events.last - 3, &all);
        assert_eq!(last, (KEPT + 5) as u64);
        assert_eq!(
            after
                .iter()
                .map(|e| e["id"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [last - 2, last - 1, last]
        );
        // Asked from before the oldest kept: everything kept.
        assert_eq!(events.after(0, &all).1.len(), KEPT);
        let chat = |t: &str| t == "chat";
        assert!(
            events
                .after(last - 10, &chat)
                .1
                .iter()
                .all(|e| e["type"] == "chat")
        );
        assert!(events.after(last, &all).1.is_empty());
    }

    #[test]
    fn a_stream_sends_what_happens() {
        use std::io::{BufRead, BufReader};
        let dir =
            std::env::temp_dir().join(format!("horizon-server-events-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shared = Shared::with_install(&dir, None);
        shared.log_line("before".into());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let served = shared.clone();
        crate::http::serve(listener, move |request| route(&served, request)).unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        write!(
            stream,
            "GET /api/v1/events?types=join HTTP/1.1\r\nAccept: text/event-stream\r\n\r\n"
        )
        .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        while line != "\r\n" {
            line.clear();
            reader.read_line(&mut line).unwrap();
        }
        shared.log_line("not a join".into());
        shared.emit("join", json!({ "name": "Kim0" }));
        let mut got = Vec::new();
        for _ in 0..3 {
            line.clear();
            reader.read_line(&mut line).unwrap();
            got.push(line.trim_end().to_owned());
        }
        assert_eq!(got[0], "id: 3");
        assert_eq!(got[1], "event: join");
        assert!(
            got[2].starts_with("data: {") && got[2].contains("\"name\":\"Kim0\""),
            "{got:?}"
        );
        // Polled as JSON: everything after the first.
        let polled = route(
            &shared,
            &Request {
                method: "GET".into(),
                path: "/api/v1/events".into(),
                query: "since=1".into(),
                headers: Vec::new(),
                body: Vec::new(),
            },
        );
        let polled: Value = serde_json::from_str(&polled.body).unwrap();
        assert_eq!(polled["next"], 3);
        assert_eq!(polled["events"][0]["line"], "not a join");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
