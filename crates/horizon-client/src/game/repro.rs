//! Repro recordings (docs/agent-control.md): a session's input, tick-stamped
//! state and (single player) a copy of the world, so a bug report can carry a
//! reproducible run. `record-start`/`record-stop` write it, `repro-export`
//! zips it with the logs and crash report, and `record-replay` loads the
//! world and feeds the inputs back, best effort.
//!
//! The recording format (`horizon-repro/1`):
//!
//! ```text
//! recordings/<stamp>-<name>/
//!   session.json          format, mode, world, ticks, counts
//!   inputs.jsonl          one trace event a line (game/input_trace.rs)
//!   state.jsonl           `state_value()` every second, entities every ten
//!   events.jsonl          the agent event log (screens, chat, deaths, items)
//!   world/                single player only: the world when it started
//!   README.txt
//! ```
use super::event_loop::EventLoopCtl;
use super::input_trace::{InputOrigin, TraceEvent, TraceKind};
use super::{Game, Screen};
use horizon_client::{Result, gamepad::Button, keybinds, text::Text, title::world_management};
use horizon_core::geometry::Vec3;
use horizon_telemetry::scrub::Scrubber;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime},
};
use winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseScrollDelta, TouchPhase, WindowEvent},
    window::WindowId,
};

pub(super) const HELP: &[&str] = &[
    "record-start [NAME] [--no-world]  record input, state and the world for a bug repro",
    "record-stop [--export]            stop the recording (and write the repro zip)",
    "record-status                     the running recording and replay, if any",
    "record-list                       the recordings and bundles on disk",
    "record-replay NAME [--stop] [--in-place] [--keep-world] [--timeout S]  replay a recording, best effort",
    "repro-export [NAME] [PATH] [--no-world]  zip a recording with the logs, crash report and status",
];

/// What `session.json` says this is.
const FORMAT: &str = "horizon-repro/1";
/// A keyframe every second: replay's state oracle.
const KEYFRAME_TICKS: u64 = 20;
/// Entities every ten seconds: bigger, so rarer.
const ENTITIES_TICKS: u64 = 200;
/// A recording stops itself at this many input events.
const MAX_INPUT_EVENTS: u64 = 1_000_000;
/// A recording stops itself after this long (two hours).
const MAX_RECORDING_SECS: u64 = 2 * 60 * 60;
/// How long a replay may take by default.
const REPLAY_TIMEOUT_SECS: u64 = 300;
/// Recordings and bundles kept; the oldest beyond these are pruned.
const KEPT_RECORDINGS: usize = 10;

/// What a replay may run. Bundles travel between machines, so a recording is
/// not allowed to run world, network or account commands on someone else's
/// game: only the input and movement lines a repro needs.
const REPLAY_COMMANDS: &[&str] = &[
    "attack",
    "click",
    "click-at",
    "click-button",
    "click-slot",
    "close",
    "drag-at",
    "drop",
    "goto",
    "hold",
    "interact",
    "jump",
    "key",
    "look",
    "look-at",
    "mouse",
    "mouse-move",
    "move",
    "open-inventory",
    "pad",
    "release",
    "respawn",
    "scroll",
    "scroll-at",
    "select",
    "stop",
    "tp",
    "turn",
    "use-item",
    "wait",
];

/// A recording, an export and a replay in flight: the game's repro state.
#[derive(Default)]
pub(super) struct Repro {
    recording: Option<Recording>,
    replay: Option<Replay>,
    /// A replay waits for its world to load.
    pending_replay: Option<PendingReplay>,
    /// Zip files being written off the game thread.
    exports: Vec<Export>,
}

struct Recording {
    name: String,
    dir: PathBuf,
    start_tick: u64,
    started: Instant,
    /// Wall clock, for the crash report's freshness.
    started_at: SystemTime,
    /// `InputTrace`'s count when the recording started: the inputs in this run.
    input_base: u64,
    /// The player and world state when it started: a replay returns the player
    /// to where the recording was.
    start: Value,
    mode: &'static str,
    world: Option<String>,
    /// Joins `inputs.jsonl`'s writer once `InputTrace::stop_file` drops it.
    inputs: Option<thread::JoinHandle<()>>,
    events: Option<Sender<String>>,
    events_joined: Option<thread::JoinHandle<()>>,
    states: Option<Sender<String>>,
    states_joined: Option<thread::JoinHandle<()>>,
    inputs_written: u64,
    events_written: u64,
    states_written: u64,
}

struct Replay {
    name: String,
    /// The recording's first tick, which becomes the replay's first tick.
    base_tick: u64,
    start_tick: u64,
    /// The replay's last tick: the recording's span, not just its last input.
    end_at: u64,
    deadline: Instant,
    events: Vec<TraceEvent>,
    index: usize,
    sent: u64,
    skipped: u64,
    last_progress: u64,
    /// The `saves/` copy it opened, removed when it ends unless kept.
    world_folder: Option<String>,
    keep_world: bool,
}

struct PendingReplay {
    reply: Option<Sender<String>>,
    name: String,
    /// The `saves/` folder to open; empty for `--in-place`.
    folder: String,
    in_place: bool,
    opened: bool,
    keep_world: bool,
    /// When the replay was asked for, not when its world finally loaded.
    deadline: Instant,
    base_tick: u64,
    end_tick: u64,
    start: Option<Value>,
    events: Vec<TraceEvent>,
}

struct Export {
    rx: Receiver<Value>,
    reply: Option<Sender<String>>,
    key: bool,
}

#[derive(Deserialize)]
struct SessionMeta {
    #[serde(default)]
    start_tick: u64,
    #[serde(default)]
    mode: String,
    /// Where the recording stopped: a replay keeps running until the same point.
    /// `None` when the process died before `record-stop`.
    #[serde(default)]
    end_tick: Option<u64>,
    /// The state the recording started from.
    #[serde(default)]
    start: Option<Value>,
}

fn home() -> PathBuf {
    horizon_client::logging::game_home()
}
fn recordings_dir() -> PathBuf {
    home().join("recordings")
}
fn bundles_dir() -> PathBuf {
    home().join("repro")
}
fn saves_dir() -> PathBuf {
    std::env::current_dir()
        .map(|dir| dir.join("saves"))
        .unwrap_or_else(|_| PathBuf::from("saves"))
}
fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}
fn slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_lowercase();
    if slug.is_empty() {
        "session".into()
    } else {
        slug
    }
}

/// One buffered line writer per file, off the game thread.
fn spawn_lines(path: &Path) -> io::Result<(Sender<String>, thread::JoinHandle<()>)> {
    let file = fs::File::create(path)?;
    let (tx, rx) = mpsc::channel::<String>();
    let handle = thread::spawn(move || {
        let mut out = BufWriter::new(file);
        let mut lines = 0u64;
        for line in rx {
            if writeln!(out, "{line}").is_err() {
                break;
            }
            lines += 1;
            // A crash loses at most this many lines.
            if lines.is_multiple_of(64) && out.flush().is_err() {
                break;
            }
        }
        let _ = out.flush();
    });
    Ok((tx, handle))
}

/// Copy a world without its lock or symlinks; the recording's own snapshot.
fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "session.lock" {
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let to = dst.join(&name);
        if kind.is_dir() {
            copy_tree(&path, &to)?;
        } else if kind.is_file() {
            fs::copy(&path, &to)?;
        }
    }
    Ok(())
}

/// The oldest recordings or bundles beyond `max`, leaving `keep` alone.
fn prune_dir(dir: &Path, keep: Option<&Path>, max: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let ours = if path.is_dir() {
                path.join("session.json").is_file()
            } else {
                path.extension().is_some_and(|extension| extension == "zip")
            };
            if !ours || keep.is_some_and(|kept| kept == path.as_path()) {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    items.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in items.into_iter().skip(max) {
        let result = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if let Err(e) = result {
            log::warn!("Horizon: pruning {}: {e}", path.display());
        }
    }
}

fn readme() -> String {
    format!(
        "Horizon repro recording ({FORMAT})\n\
         \n\
         inputs.jsonl   one input event a line: who caused it (user, agent, game),\n\
         \x20              the tick and the key/mouse/pad event or control command.\n\
         state.jsonl    the player and world state every second (tick and full state).\n\
         events.jsonl   what happened: screens, chat, deaths, items, dimensions.\n\
         session.json   the world, mode, game version and start tick.\n\
         world/         single player: the world as it was when the recording started.\n\
         \n\
         Replay it with: horizon-ctl record-replay <this folder's name>\n\
         or (from a zip) horizon-ctl record-replay <the zip's name>.\n\
         Attach the zip from `repro-export` to the bug report; the export scrubs\n\
         the user name, paths, IPs, UUIDs and the world/server names it knows.\n\
         Nothing is uploaded automatically and no network traffic is recorded.\n\
         A replay only runs input commands, never world, network or account ones.\n"
    )
}

fn recording_meta(rec: &Recording, ended: Option<&Instant>, ticks: u64) -> Value {
    json!({
        "format": FORMAT,
        "name": rec.name,
        "created": chrono::Local::now().to_rfc3339(),
        "game_version": env!("CARGO_PKG_VERSION"),
        "mode": rec.mode,
        "world": rec.world,
        "start_tick": rec.start_tick,
        "end_tick": (ended.is_some()).then_some(ticks),
        "seconds": ended.map(|_| rec.started.elapsed().as_secs_f64()),
        "start": rec.start,
        "inputs": rec.inputs_written,
        "events": rec.events_written,
        "state": rec.states_written,
    })
}

/// A keyframe without the absolute recording path (it holds the user name).
fn keyframe_state(mut state: Value) -> Value {
    if let Some(recording) = state
        .pointer_mut("/input/recording")
        .and_then(Value::as_object_mut)
    {
        recording.remove("dir");
    }
    if let Some(replay) = state
        .pointer_mut("/input/replay")
        .and_then(Value::as_object_mut)
    {
        replay.remove("name");
    }
    state
}

impl Game<'_> {
    /// `record-*` and `repro-export`; None for other commands, Some(None) when deferred.
    pub(super) fn repro_command(
        &mut self,
        words: &[&str],
        _line: &str,
        reply: &Sender<String>,
    ) -> Result<Option<Option<Value>>> {
        match words[0] {
            "record-start" => self.record_start_command(&words[1..], Some(reply.clone())),
            "record-stop" => self.record_stop_command(&words[1..], Some(reply.clone())),
            "record-status" => Ok(Some(Some(self.record_status(self.input_trace.events())))),
            "record-list" => Ok(Some(Some(self.record_list()))),
            "record-replay" => self.replay_command(&words[1..], Some(reply.clone())),
            "repro-export" => self.export_command(&words[1..], Some(reply.clone())),
            _ => Ok(None),
        }
    }

    /// Once per loop turn: a waiting replay, the inputs due, and finished exports.
    pub(super) fn poll_repro(&mut self, event_loop: &dyn EventLoopCtl) {
        self.poll_pending_replay();
        self.advance_replay(event_loop);
        self.poll_exports();
    }

    /// Once per tick: the state keyframes. Stops a recording that outgrew its caps.
    pub(super) fn repro_tick(&mut self) {
        let Some((start_tick, started, start_inputs, inputs)) =
            self.repro.recording.as_ref().map(|rec| {
                (
                    rec.start_tick,
                    rec.started,
                    rec.input_base,
                    self.input_trace.events(),
                )
            })
        else {
            return;
        };
        let ticking = self.title.is_none() && self.screen == Screen::Playing;
        let elapsed = self.ticks.saturating_sub(start_tick);
        if ticking && elapsed.is_multiple_of(KEYFRAME_TICKS) {
            let state = self.state_value();
            self.repro.record_state(self.ticks, state);
        }
        if ticking && elapsed.is_multiple_of(ENTITIES_TICKS) {
            let entities = self.entities_value(48.0, None);
            self.repro.record_entities(self.ticks, entities);
        }
        let too_many = inputs.saturating_sub(start_inputs) >= MAX_INPUT_EVENTS;
        let too_long = started.elapsed().as_secs() >= MAX_RECORDING_SECS;
        if too_many || too_long {
            log::warn!("Horizon: the recording reached its limit; stopping it");
            if let Err(e) = self.stop_recording(false, None) {
                log::warn!("Horizon: stopping the recording: {e}");
            }
        }
    }

    /// A native exit: finish the recording and write its bundle, so a crash or a
    /// quit still leaves a repro.
    pub(super) fn repro_on_exit(&mut self) {
        if self.repro.recording.is_none() {
            return;
        }
        if let Err(e) = self.stop_recording(true, None) {
            log::warn!("Horizon: the recording on exit: {e}");
        }
        self.drain_exports(Duration::from_secs(10));
    }

    /// The world is closing (quit to title, another world): a recording cannot
    /// cross it, and a replay feeding the title screen is nonsense.
    pub(super) fn repro_world_closed(&mut self) {
        if self.repro.recording.is_some()
            && let Err(e) = self.stop_recording(false, None)
        {
            log::warn!("Horizon: stopping the recording at the world close: {e}");
        }
        if self.stop_replay() {
            log::info!("Horizon: replay stopped: the world closed");
        }
    }

    /// F3+R: start a recording, or stop the running one and write its bundle.
    pub(super) fn toggle_recording_key(&mut self) {
        if self.repro.replay.is_some() || self.repro.pending_replay.is_some() {
            return;
        }
        if self.repro.recording.is_some() {
            if let Err(e) = self.stop_recording(true, None) {
                self.repro_say(
                    "horizon.repro.error",
                    &[Text::literal(format!("stopping the recording: {e}"))],
                );
            }
        } else if let Err(e) = self.start_recording(None, false, true) {
            self.repro_say("horizon.repro.error", &[Text::literal(format!("{e}"))]);
        }
    }

    /// F3+E: write a bundle now, from the running or the newest recording.
    pub(super) fn export_repro_key(&mut self) {
        if self.repro.recording.is_some() {
            if let Err(e) = self.stop_recording(true, None) {
                self.repro_say(
                    "horizon.repro.error",
                    &[Text::literal(format!("exporting the recording: {e}"))],
                );
            }
            return;
        }
        match newest_recording() {
            Some(dir) => {
                if let Err(e) = self.spawn_export(dir, None, false, None, true) {
                    self.repro_say(
                        "horizon.repro.error",
                        &[Text::literal(format!("exporting the recording: {e}"))],
                    );
                }
            }
            None => self.repro_say("horizon.repro.no_recording", &[]),
        }
    }

    fn repro_say(&mut self, key: &str, args: &[Text]) {
        self.system_message(Text::translate(key, args));
    }

    // ---- start / stop -------------------------------------------------------

    fn record_start_command(
        &mut self,
        args: &[&str],
        reply: Option<Sender<String>>,
    ) -> Result<Option<Option<Value>>> {
        let mut name = None;
        let mut no_world = false;
        for arg in args {
            match *arg {
                "--no-world" => no_world = true,
                other if other.starts_with("--") => {
                    return Err(format!("record-start: unknown flag {other}").into());
                }
                other => name = Some(other.to_owned()),
            }
        }
        let key = reply.is_none();
        let value = self.start_recording(name, no_world, key)?;
        Ok(Some(Some(value)))
    }

    /// Start a recording. The world copy is made here, with the world frozen, so
    /// a later stop never races a half-copied world.
    fn start_recording(
        &mut self,
        name: Option<String>,
        no_world: bool,
        key: bool,
    ) -> Result<Value> {
        if self.repro.recording.is_some() {
            return Err("a recording is already running; record-stop first".into());
        }
        if self.repro.replay.is_some() || self.repro.pending_replay.is_some() {
            return Err("a replay is running; record-replay --stop first".into());
        }
        if self.pending_save.is_some() {
            return Err("a world save is in progress".into());
        }
        // Single player is a client of its own integrated server too: that is
        // still a local world with a folder to copy. The deprecated in-frame
        // mode is not one (CLAUDE.md: never use it).
        let mode = if self.integrated.is_some() {
            "local"
        } else if self.remote.is_some() {
            "remote"
        } else if self.title.is_some() {
            return Err("open a world first".into());
        } else {
            return Err(
                "recording needs the normal client: the in-frame mode is not supported".into(),
            );
        };
        let name = slug(&name.unwrap_or_else(|| "session".into()));
        let dir = recordings_dir().join(format!("{}-{name}", stamp()));
        fs::create_dir_all(&dir)?;
        // Single player's world belongs to the integrated server thread: its
        // save folder is the one to copy, not the client's title-screen save.
        let world_root = if mode == "local" {
            match self.integrated.as_ref() {
                Some(server) => server.call(|game| game.world_root.clone()),
                None if !self.world_root.as_os_str().is_empty() => Some(self.world_root.clone()),
                None => None,
            }
        } else {
            None
        };
        let world = world_root
            .as_ref()
            .and_then(|root| root.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        let include_world = mode == "local" && !no_world;
        if include_world
            && !world_root
                .as_ref()
                .is_some_and(|root| root.join("level.dat").is_file())
        {
            return Err(format!(
                "the world's save folder {} is not on disk yet",
                world_root
                    .as_ref()
                    .map_or_else(|| "(unknown)".into(), |root| root.display().to_string())
            )
            .into());
        }
        let start_step = self.ticks;
        let start_state = keyframe_state(self.state_value());

        // Save, then copy with the world frozen: the integrated server keeps
        // ticking and autosaving otherwise, and a region file would tear.
        if include_world {
            let root = world_root.as_ref().expect("a local world's save folder");
            if let Some(server) = self.integrated.as_ref() {
                server.set_paused(true);
                let saved = server.call(|game| game.save_game().map_err(|e| e.to_string()));
                let copied = copy_tree(root, &dir.join("world"));
                server.set_paused(self.pause);
                match saved {
                    Some(Ok(())) => {}
                    Some(Err(e)) => {
                        return Err(format!("saving the world for the recording: {e}").into());
                    }
                    None => {
                        log::warn!("Horizon: the integrated server did not save for the recording")
                    }
                }
                copied.map_err(|e| format!("copying the world: {e}"))?;
            } else {
                self.save_game_with(true)?;
                copy_tree(root, &dir.join("world"))
                    .map_err(|e| format!("copying the world: {e}"))?;
            }
        }

        let (inputs, inputs_joined) = spawn_lines(&dir.join("inputs.jsonl"))?;
        let (events, events_joined) = spawn_lines(&dir.join("events.jsonl"))?;
        let (states, states_joined) = spawn_lines(&dir.join("state.jsonl"))?;
        self.input_trace.start_file(inputs);
        fs::write(dir.join("README.txt"), readme())?;
        let rec = Recording {
            name: name.clone(),
            dir: dir.clone(),
            start_tick: start_step,
            started: Instant::now(),
            started_at: SystemTime::now(),
            input_base: self.input_trace.events(),
            start: start_state,
            mode,
            world: world.clone(),
            inputs: Some(inputs_joined),
            events: Some(events),
            events_joined: Some(events_joined),
            states: Some(states),
            states_joined: Some(states_joined),
            inputs_written: 0,
            events_written: 0,
            states_written: 0,
        };
        fs::write(
            dir.join("session.json"),
            serde_json::to_vec_pretty(&recording_meta(&rec, None, start_step))?,
        )?;
        self.repro.recording = Some(rec);
        prune_dir(&recordings_dir(), Some(&dir), KEPT_RECORDINGS);
        if key {
            self.repro_say("horizon.repro.started", &[]);
        }
        Ok(json!({
            "ok": true,
            "recording": name,
            "dir": dir.display().to_string(),
            "tick": start_step,
            "world": world,
            "mode": mode,
        }))
    }

    fn record_stop_command(
        &mut self,
        args: &[&str],
        reply: Option<Sender<String>>,
    ) -> Result<Option<Option<Value>>> {
        let mut export = false;
        for arg in args {
            match *arg {
                "--export" => export = true,
                other => return Err(format!("record-stop: unknown argument {other}").into()),
            }
        }
        match self.stop_recording(export, reply)? {
            Some(value) => Ok(Some(Some(value))),
            None => Ok(Some(None)),
        }
    }

    /// Stop the recording. With `export`, its bundle is written off the game
    /// thread: None means the reply comes later.
    fn stop_recording(
        &mut self,
        export: bool,
        reply: Option<Sender<String>>,
    ) -> Result<Option<Value>> {
        let Some(mut rec) = self.repro.recording.take() else {
            return Err("no recording is running".into());
        };
        // A last keyframe after the last input, so a replay's end is covered.
        if self.title.is_none() {
            let state = keyframe_state(self.state_value());
            let entities = self.entities_value(48.0, None);
            let tick = self.ticks;
            if let Some(states) = &rec.states {
                if states
                    .send(json!({ "tick": tick, "kind": "state", "state": state }).to_string())
                    .is_ok()
                {
                    rec.states_written += 1;
                }
                if states
                    .send(
                        json!({ "tick": tick, "kind": "entities", "entities": entities })
                            .to_string(),
                    )
                    .is_ok()
                {
                    rec.states_written += 1;
                }
            }
        }
        rec.inputs_written = self.input_trace.stop_file();
        drop(rec.events.take());
        drop(rec.states.take());
        if let Some(handle) = rec.inputs.take() {
            let _ = handle.join();
        }
        if let Some(handle) = rec.events_joined.take() {
            let _ = handle.join();
        }
        if let Some(handle) = rec.states_joined.take() {
            let _ = handle.join();
        }
        let meta = recording_meta(&rec, Some(&Instant::now()), self.ticks);
        let _ = fs::write(
            rec.dir.join("session.json"),
            serde_json::to_vec_pretty(&meta).unwrap_or_default(),
        );
        let summary = json!({
            "ok": true,
            "recording": rec.name,
            "dir": rec.dir.display().to_string(),
            "seconds": rec.started.elapsed().as_secs_f64(),
            "inputs": rec.inputs_written,
            "events": rec.events_written,
            "state": rec.states_written,
            "mode": rec.mode,
            "world": rec.world,
        });
        if export {
            let key = reply.is_none();
            self.spawn_export(rec.dir.clone(), None, false, reply, key)?;
            Ok(None)
        } else {
            if reply.is_none() {
                self.repro_say("horizon.repro.stopped", &[Text::literal(rec.name.clone())]);
            }
            Ok(Some(summary))
        }
    }

    // ---- status / list -----------------------------------------------------

    fn record_status(&self, inputs: u64) -> Value {
        json!({
            "recording": self.repro.recording.as_ref().map(|r| json!({
                "name": r.name,
                "dir": r.dir.display().to_string(),
                "seconds": r.started.elapsed().as_secs_f64(),
                "start_tick": r.start_tick,
                "ticks": self.ticks.saturating_sub(r.start_tick),
                "inputs": inputs.saturating_sub(r.input_base),
                "events": r.events_written,
                "state": r.states_written,
                "mode": r.mode,
                "world": r.world,
            })),
            "replay": self.repro.replay.as_ref().map(|r| json!({
                "name": r.name,
                "sent": r.sent,
                "skipped": r.skipped,
                "total": r.events.len(),
            })),
        })
    }

    fn record_list(&self) -> Value {
        let mut recordings = Vec::new();
        if let Ok(entries) = fs::read_dir(recordings_dir()) {
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let dir = entry.path();
                let Ok(bytes) = fs::read(dir.join("session.json")) else {
                    continue;
                };
                let Ok(meta) = serde_json::from_slice::<Value>(&bytes) else {
                    continue;
                };
                recordings.push(json!({
                    "name": meta.get("name").cloned().unwrap_or(Value::Null),
                    "dir": dir.display().to_string(),
                    "mode": meta.get("mode").cloned().unwrap_or(Value::Null),
                    "world": meta.get("world").cloned().unwrap_or(Value::Null),
                    "created": meta.get("created").cloned().unwrap_or(Value::Null),
                    "inputs": meta.get("inputs").cloned().unwrap_or(Value::Null),
                    "has_world": dir.join("world").is_dir(),
                }));
            }
        }
        recordings.sort_by(|a, b| b["created"].as_str().cmp(&a["created"].as_str()));
        let mut bundles = Vec::new();
        if let Ok(entries) = fs::read_dir(bundles_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "zip") {
                    bundles.push(json!({
                        "name": path.file_stem().map(|n| n.to_string_lossy().into_owned()),
                        "path": path.display().to_string(),
                    }));
                }
            }
        }
        json!({ "recordings": recordings, "bundles": bundles })
    }

    // ---- export -------------------------------------------------------------

    fn export_command(
        &mut self,
        args: &[&str],
        reply: Option<Sender<String>>,
    ) -> Result<Option<Option<Value>>> {
        let mut name = None;
        let mut path = None;
        let mut no_world = false;
        for arg in args {
            match *arg {
                "--no-world" => no_world = true,
                other if other.starts_with("--") => {
                    return Err(format!("repro-export: unknown flag {other}").into());
                }
                other if path.is_none() && (other.contains('/') || other.ends_with(".zip")) => {
                    path = Some(PathBuf::from(other));
                }
                other if name.is_none() => name = Some(other.to_owned()),
                other => return Err(format!("repro-export: unexpected {other}").into()),
            }
        }
        if self.repro.recording.is_some() {
            // Exporting the run as it stands: stop it first so the files are closed.
            match self.stop_recording(false, None)? {
                Some(_) => {}
                None => return Err("the recording could not be stopped".into()),
            }
        }
        let dir = match name {
            Some(name) => resolve_recording(&name)?,
            None => newest_recording().ok_or("no recording to export yet")?,
        };
        let key = reply.is_none();
        self.spawn_export(dir, path, no_world, reply, key)?;
        Ok(Some(None))
    }

    fn spawn_export(
        &mut self,
        dir: PathBuf,
        dest: Option<PathBuf>,
        no_world: bool,
        reply: Option<Sender<String>>,
        key: bool,
    ) -> Result<()> {
        let status = self.control_status();
        let home = home();
        let dest = dest.unwrap_or_else(|| {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "repro".into());
            bundles_dir().join(format!("{name}.zip"))
        });
        let started_at = self.repro.recording.as_ref().map_or_else(
            || {
                recording_started_at(&dir)
                    .unwrap_or_else(|| SystemTime::now() - Duration::from_secs(MAX_RECORDING_SECS))
            },
            |rec| rec.started_at,
        );
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let (tx, rx) = mpsc::channel();
        let dir_for_thread = dir.clone();
        thread::spawn(move || {
            let value =
                match write_bundle(&dir_for_thread, &dest, &status, &home, no_world, started_at) {
                    Ok(bytes) => json!({
                        "ok": true,
                        "path": dest.display().to_string(),
                        "bytes": bytes,
                    }),
                    Err(e) => json!({ "error": format!("repro export failed: {e}") }),
                };
            let _ = tx.send(value);
        });
        self.repro.exports.push(Export { rx, reply, key });
        Ok(())
    }

    fn poll_exports(&mut self) {
        let mut done = Vec::new();
        for (i, export) in self.repro.exports.iter().enumerate() {
            match export.rx.try_recv() {
                Ok(value) => done.push((i, value)),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => done.push((
                    i,
                    json!({ "error": "the export thread stopped without a result" }),
                )),
            }
        }
        for (i, value) in done.into_iter().rev() {
            let export = self.repro.exports.remove(i);
            if let Some(path) = value.get("path").and_then(Value::as_str) {
                prune_dir(&bundles_dir(), Some(Path::new(path)), KEPT_RECORDINGS);
            }
            if let Some(reply) = export.reply {
                let _ = reply.send(value.to_string());
            } else if export.key || value.get("error").is_some() {
                if let Some(error) = value.get("error").and_then(Value::as_str) {
                    self.repro_say("horizon.repro.error", &[Text::literal(error.to_owned())]);
                } else {
                    let path = value
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    self.repro_say("horizon.repro.bundle", &[Text::literal(path.to_owned())]);
                }
            }
        }
    }

    fn drain_exports(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !self.repro.exports.is_empty() && Instant::now() < deadline {
            self.poll_exports();
            if !self.repro.exports.is_empty() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        self.poll_exports();
    }

    // ---- replay -------------------------------------------------------------

    fn replay_command(
        &mut self,
        args: &[&str],
        reply: Option<Sender<String>>,
    ) -> Result<Option<Option<Value>>> {
        let mut name = None;
        let mut stop = false;
        let mut in_place = false;
        let mut keep_world = false;
        let mut timeout = REPLAY_TIMEOUT_SECS;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "--stop" => stop = true,
                "--in-place" => in_place = true,
                "--keep-world" => keep_world = true,
                "--timeout" => {
                    timeout = args
                        .get(i + 1)
                        .and_then(|t| t.parse().ok())
                        .ok_or("--timeout takes whole seconds")?;
                    i += 1;
                }
                other if other.starts_with("--") => {
                    return Err(format!("record-replay: unknown flag {other}").into());
                }
                other => name = Some(other.to_owned()),
            }
            i += 1;
        }
        if stop {
            let stopped = self.stop_replay();
            if stopped {
                self.events
                    .note(self.ticks, json!({ "event": "replay", "state": "stopped" }));
            }
            return Ok(Some(Some(json!({ "ok": true, "stopped": stopped }))));
        }
        let name = name.ok_or("record-replay NAME: a recording from record-list")?;
        if self.repro.recording.is_some() {
            return Err("a recording is running; record-stop first".into());
        }
        if self.repro.replay.is_some() || self.repro.pending_replay.is_some() {
            return Err("a replay is already running; record-replay --stop first".into());
        }
        let dir = resolve_recording(&name)?;
        let bytes = fs::read(dir.join("session.json"))?;
        let meta: SessionMeta = serde_json::from_slice(&bytes)?;
        if meta.mode == "remote" && !in_place {
            return Err(
                "this recording is a server session: replay it with --in-place in the same world"
                    .into(),
            );
        }
        let events = read_events(&dir.join("inputs.jsonl"))?;
        let base_tick = meta.start_tick;
        // A recording that never stopped cleanly has no end_tick: use its last input.
        let end_tick = meta
            .end_tick
            .unwrap_or_else(|| events.last().map_or(base_tick, |event| event.tick))
            .max(base_tick);
        let deadline = Instant::now() + Duration::from_secs(timeout);
        if in_place {
            if self.title.is_some() {
                return Err("--in-place needs an open world to feed the input into".into());
            }
            let pending = PendingReplay {
                reply,
                name,
                folder: String::new(),
                in_place: true,
                opened: true,
                keep_world: true,
                deadline,
                base_tick,
                end_tick,
                start: meta.start,
                events,
            };
            self.begin_replay(pending);
            return Ok(Some(None));
        }
        let world = dir.join("world");
        if !world.is_dir() {
            return Err(
                "this recording has no world copy (it was made with --no-world or on a server)"
                    .into(),
            );
        }
        let folder = format!("{}-replay-{}", slug(&name), stamp());
        copy_tree(&world, &saves_dir().join(&folder))?;
        if self.title.is_none() {
            self.close_world()?;
        }
        self.repro.pending_replay = Some(PendingReplay {
            reply,
            name,
            folder,
            in_place: false,
            opened: false,
            keep_world,
            deadline,
            base_tick,
            end_tick,
            start: meta.start,
            events,
        });
        Ok(Some(None))
    }

    fn poll_pending_replay(&mut self) {
        let Some((in_place, opened, folder, deadline)) = self
            .repro
            .pending_replay
            .as_ref()
            .map(|p| (p.in_place, p.opened, p.folder.clone(), p.deadline))
        else {
            return;
        };
        if Instant::now() >= deadline {
            let pending = self.repro.pending_replay.take().unwrap();
            self.discard_replay_world(&pending);
            self.reply_error(pending.reply, "the replay's world did not load in time");
            return;
        }
        if !in_place {
            if !opened {
                if self.pending_save.is_some() || self.title.is_none() {
                    return;
                }
                if let Err(e) = self.open_world(&folder, None, None, false, None) {
                    let pending = self.repro.pending_replay.take().unwrap();
                    self.discard_replay_world(&pending);
                    self.reply_error(
                        pending.reply,
                        &format!("could not open the recording's world: {e}"),
                    );
                    return;
                }
                if let Some(pending) = self.repro.pending_replay.as_mut() {
                    pending.opened = true;
                }
            }
            if !self.loaded() || self.pending_entry.is_some() || self.world_loader.is_some() {
                return;
            }
        }
        let pending = self.repro.pending_replay.take().unwrap();
        self.begin_replay(pending);
    }

    fn begin_replay(&mut self, pending: PendingReplay) {
        let total = pending.events.len();
        let name = pending.name.clone();
        // The world copy's player data can be older than the recording: put the
        // player back where it was, facing the same way, and give it a few ticks
        // to land before the input starts.
        if let Some(start) = &pending.start {
            if let Some(position) = start.pointer("/player/position").and_then(Value::as_array)
                && let (Some(x), Some(y), Some(z)) = (
                    position.first().and_then(Value::as_f64),
                    position.get(1).and_then(Value::as_f64),
                    position.get(2).and_then(Value::as_f64),
                )
                && let Some(player) = self.player.as_mut()
            {
                let to = Vec3::new(x, y, z);
                player.position = to;
                player.fall_distance = 0.0;
                self.previous_position = to;
            }
            // A player still walking or falling when the recording started kept
            // its momentum into the bug; keep it here too.
            if let Some(velocity) = start.pointer("/player/velocity").and_then(Value::as_array)
                && let (Some(x), Some(y), Some(z)) = (
                    velocity.first().and_then(Value::as_f64),
                    velocity.get(1).and_then(Value::as_f64),
                    velocity.get(2).and_then(Value::as_f64),
                )
                && let Some(player) = self.player.as_mut()
            {
                player.velocity = Vec3::new(x, y, z);
            }
            if let Some(yaw) = start.pointer("/player/yaw").and_then(Value::as_f64) {
                self.yaw = yaw as f32;
            }
            if let Some(pitch) = start.pointer("/player/pitch").and_then(Value::as_f64) {
                self.pitch = pitch as f32;
            }
        }
        let start_tick = self.ticks.saturating_add(3);
        let end_at = start_tick.saturating_add(pending.end_tick.saturating_sub(pending.base_tick));
        self.input_trace.set_suppress(true);
        self.repro.replay = Some(Replay {
            name: name.clone(),
            base_tick: pending.base_tick,
            start_tick,
            end_at,
            deadline: pending.deadline,
            events: pending.events,
            index: 0,
            sent: 0,
            skipped: 0,
            last_progress: 0,
            world_folder: (!pending.in_place).then_some(pending.folder),
            keep_world: pending.keep_world,
        });
        self.events.note(
            self.ticks,
            json!({ "event": "replay", "state": "started", "name": name }),
        );
        let value = json!({
            "ok": true,
            "replay": name,
            "tick": self.ticks,
            "events": total,
        });
        match pending.reply {
            Some(reply) => {
                let _ = reply.send(value.to_string());
            }
            None => {
                self.repro_say("horizon.repro.replay_started", &[Text::literal(name)]);
            }
        }
    }

    fn advance_replay(&mut self, event_loop: &dyn EventLoopCtl) {
        let mut due = Vec::new();
        let mut timed_out = false;
        {
            let Some(replay) = self.repro.replay.as_mut() else {
                return;
            };
            if Instant::now() >= replay.deadline {
                timed_out = true;
            } else {
                while replay.index < replay.events.len() {
                    let event = &replay.events[replay.index];
                    let at = replay
                        .start_tick
                        .saturating_add(event.tick.saturating_sub(replay.base_tick));
                    if at > self.ticks {
                        break;
                    }
                    due.push(event.clone());
                    replay.index += 1;
                }
            }
        }
        for event in due {
            let outcome = self.replay_event(event, event_loop);
            if let Some(replay) = self.repro.replay.as_mut() {
                if outcome {
                    replay.sent += 1;
                } else {
                    replay.skipped += 1;
                }
            }
        }
        let mut finished = timed_out;
        if let Some(replay) = &self.repro.replay {
            // The recording's span, not just its last input: its trailing wait
            // and deceleration are part of what the bug needed.
            if replay.index >= replay.events.len() && self.ticks >= replay.end_at {
                finished = true;
            }
            if !finished && replay.sent.saturating_sub(replay.last_progress) >= 400 {
                let value = json!({
                    "event": "replay",
                    "state": "progress",
                    "name": replay.name,
                    "sent": replay.sent,
                    "total": replay.events.len(),
                });
                self.repro.note_event(self.ticks, &value);
                self.events.note(self.ticks, value);
                if let Some(replay) = self.repro.replay.as_mut() {
                    replay.last_progress = replay.sent;
                }
            }
        }
        if finished {
            self.finish_replay(timed_out);
        }
    }

    /// One recorded event, fed back. False when it is not replayed.
    fn replay_event(&mut self, event: TraceEvent, event_loop: &dyn EventLoopCtl) -> bool {
        let id = match &self.window {
            Some(window) => window.id(),
            None => WindowId::dummy(),
        };
        let device_id = DeviceId::dummy();
        match event.kind {
            TraceKind::Command { line } => {
                let name = line.split_whitespace().next().unwrap_or("");
                // A recording travels between machines: only input and movement
                // lines may run on someone else's game.
                if !REPLAY_COMMANDS.contains(&name) {
                    log::debug!("Horizon: replay skips `{line}`");
                    return false;
                }
                self.replay_line(&line, event_loop);
                true
            }
            TraceKind::Key { key, action } => {
                // Through the control line, so its release guard runs too.
                self.replay_line(&format!("key {key} {action}"), event_loop);
                true
            }
            TraceKind::Mouse { button, action } => {
                let Some(button) = keybinds::mouse_button(&button) else {
                    return false;
                };
                let state = if action == "down" {
                    ElementState::Pressed
                } else {
                    ElementState::Released
                };
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseInput {
                        device_id,
                        state,
                        button,
                    },
                    InputOrigin::Game,
                );
                true
            }
            TraceKind::Wheel { x, y, lines } => {
                let delta = if lines {
                    MouseScrollDelta::LineDelta(x as f32, y as f32)
                } else {
                    MouseScrollDelta::PixelDelta(PhysicalPosition::new(x, y))
                };
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseWheel {
                        device_id,
                        delta,
                        phase: TouchPhase::Moved,
                    },
                    InputOrigin::Game,
                );
                true
            }
            TraceKind::Cursor { x, y } => {
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::CursorMoved {
                        device_id,
                        position: PhysicalPosition::new(x, y),
                    },
                    InputOrigin::Game,
                );
                true
            }
            TraceKind::Motion { x, y } => {
                self.mouse.on_move(0.0, 0.0, x, y, true, true);
                self.last_input = Instant::now();
                true
            }
            TraceKind::Pad { button, action } => {
                let Some(button) = Button::ALL.into_iter().find(|b| b.name() == button) else {
                    return false;
                };
                self.pad.inject_button(button, action == "down");
                true
            }
        }
    }

    /// Let go of everything a replay (or a person) held.
    fn release_replay_inputs(&mut self) {
        self.keys.clear();
        self.physical_keys.clear();
        self.physical_buttons.clear();
        self.left = false;
        self.right = false;
        self.modifiers = winit::keyboard::ModifiersState::empty();
        self.debug_keys.modifier_down = false;
        self.pad.clear_injected();
    }

    fn finish_replay(&mut self, timed_out: bool) {
        let Some(replay) = self.repro.replay.take() else {
            return;
        };
        self.input_trace.set_suppress(false);
        self.release_replay_inputs();
        if !replay.keep_world
            && let Some(folder) = &replay.world_folder
        {
            let _ = fs::remove_dir_all(saves_dir().join(folder));
        }
        let value = json!({
            "event": "replay",
            "state": if timed_out { "timeout" } else { "done" },
            "name": replay.name,
            "sent": replay.sent,
            "skipped": replay.skipped,
            "total": replay.events.len(),
        });
        self.events.note(self.ticks, value.clone());
        log::info!(
            "Horizon: replay {}: {} of {} events{}",
            replay.name,
            replay.sent,
            replay.events.len(),
            if timed_out { " (timed out)" } else { "" }
        );
    }

    /// Stop a running or waiting replay; whether there was one.
    fn stop_replay(&mut self) -> bool {
        let mut stopped = false;
        if let Some(pending) = self.repro.pending_replay.take() {
            self.discard_replay_world(&pending);
            self.reply_error(pending.reply, "the replay was stopped before it started");
            self.input_trace.set_suppress(false);
            self.release_replay_inputs();
            stopped = true;
        }
        if self.repro.replay.is_some() {
            self.finish_replay(false);
            stopped = true;
        }
        stopped
    }

    fn discard_replay_world(&self, pending: &PendingReplay) {
        if !pending.in_place && !pending.keep_world {
            let _ = fs::remove_dir_all(saves_dir().join(&pending.folder));
        }
    }

    fn reply_error(&mut self, reply: Option<Sender<String>>, error: &str) {
        match reply {
            Some(reply) => {
                let _ = reply.send(json!({ "error": error }).to_string());
            }
            None => self.repro_say("horizon.repro.error", &[Text::literal(error.to_owned())]),
        }
    }
}

impl Repro {
    /// An agent event while recording joins `events.jsonl`.
    pub(super) fn note_event(&mut self, tick: u64, event: &Value) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        let mut event = event.clone();
        event["tick"] = tick.into();
        if let Some(events) = &rec.events
            && events.send(event.to_string()).is_ok()
        {
            rec.events_written += 1;
        }
    }

    /// A `state.jsonl` keyframe: the state at this tick.
    fn record_state(&mut self, tick: u64, state: Value) {
        self.record_keyframe(
            tick,
            json!({ "kind": "state", "state": keyframe_state(state) }),
        );
    }

    /// An `state.jsonl` keyframe of the nearby entities, rarer than the state.
    fn record_entities(&mut self, tick: u64, entities: Value) {
        self.record_keyframe(tick, json!({ "kind": "entities", "entities": entities }));
    }

    fn record_keyframe(&mut self, tick: u64, mut value: Value) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        value["tick"] = tick.into();
        if let Some(states) = &rec.states
            && states.send(value.to_string()).is_ok()
        {
            rec.states_written += 1;
        }
    }

    /// Whether a recording is running (the AI Agent page's button label).
    pub(super) fn recording_active(&self) -> bool {
        self.recording.is_some()
    }

    /// The AI Agent page's line while a recording or replay runs.
    pub(super) fn agent_line(&self, inputs: u64) -> Option<String> {
        if let Some(rec) = &self.recording {
            Some(format!(
                "Recording {}: {} s, {} input events.",
                rec.name,
                rec.started.elapsed().as_secs(),
                inputs.saturating_sub(rec.input_base)
            ))
        } else {
            let replay = self.replay.as_ref()?;
            Some(format!(
                "Replaying {}: {} of {} events.",
                replay.name,
                replay.sent,
                replay.events.len()
            ))
        }
    }

    /// The recording and replay parts of `state`'s and `status`'s `input`.
    pub(super) fn fill_summary(&self, out: &mut Value, inputs: u64) {
        out["recording"] = self.summary(inputs)["recording"].clone();
        out["replay"] = self.summary(inputs)["replay"].clone();
    }

    fn summary(&self, inputs: u64) -> Value {
        json!({
            "recording": self.recording.as_ref().map(|r| json!({
                "name": r.name,
                "dir": r.dir.display().to_string(),
                "seconds": r.started.elapsed().as_secs_f64(),
                "inputs": inputs.saturating_sub(r.input_base),
                "events": r.events_written,
                "state": r.states_written,
                "mode": r.mode,
                "world": r.world,
            })),
            "replay": self.replay.as_ref().map(|r| json!({
                "name": r.name,
                "sent": r.sent,
                "skipped": r.skipped,
                "total": r.events.len(),
            })),
        })
    }
}

/// When a recording on disk started, from its `session.json`.
fn recording_started_at(dir: &Path) -> Option<SystemTime> {
    let text = fs::read_to_string(dir.join("session.json")).ok()?;
    let meta: Value = serde_json::from_str(&text).ok()?;
    let created = meta.get("created")?.as_str()?;
    let time = chrono::DateTime::parse_from_rfc3339(created).ok()?;
    Some(SystemTime::from(time))
}

fn newest_recording() -> Option<PathBuf> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    let entries = fs::read_dir(recordings_dir()).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !path.join("session.json").is_file() {
            continue;
        }
        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, path));
        }
    }
    newest.map(|(_, path)| path)
}

/// A recording by name: an exact folder, the newest folder ending in it, or a
/// bundle (which is unpacked into `recordings/` first).
fn resolve_recording(name: &str) -> Result<PathBuf> {
    let recordings = recordings_dir();
    let exact = recordings.join(name);
    if exact.join("session.json").is_file() {
        return Ok(exact);
    }
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    if let Ok(entries) = fs::read_dir(&recordings) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if !path.is_dir() || !file_name.ends_with(&format!("-{name}")) {
                continue;
            }
            if !path.join("session.json").is_file() {
                continue;
            }
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified())
                && newest.as_ref().is_none_or(|(t, _)| modified > *t)
            {
                newest = Some((modified, path));
            }
        }
    }
    if let Some((_, path)) = newest {
        return Ok(path);
    }
    // An exported bundle: unpack it and take the folder with session.json.
    if let Ok(entries) = fs::read_dir(bundles_dir()) {
        let mut bundle = None;
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            if file_name
                .as_deref()
                .is_some_and(|n| n.contains(name) && n.ends_with(".zip"))
            {
                bundle = Some(path);
            }
        }
        if let Some(bundle) = bundle {
            let dest = recordings.join(format!("replay-{}", stamp()));
            extract_zip(&bundle, &dest)?;
            if dest.join("session.json").is_file() {
                return Ok(dest);
            }
            if let Ok(entries) = fs::read_dir(&dest) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.join("session.json").is_file() {
                        return Ok(path);
                    }
                }
            }
            return Err(format!("the bundle {} has no session.json", bundle.display()).into());
        }
    }
    Err(format!("no recording or bundle named {name}; record-list lists them").into())
}

fn read_events(path: &Path) -> Result<Vec<TraceEvent>> {
    let text = fs::read_to_string(path)?;
    let mut events = Vec::new();
    let mut skipped = 0;
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<TraceEvent>(line) {
            Ok(event) => events.push(event),
            // A killed process leaves a half-written last line; one bad line
            // must not lose a whole crash recording.
            Err(e) => {
                skipped += 1;
                log::warn!(
                    "Horizon: replay: {} line {}: {e}",
                    path.display(),
                    number + 1
                );
            }
        }
    }
    if skipped > 0 {
        log::warn!("Horizon: replay: skipped {skipped} unreadable input line(s)");
    }
    Ok(events)
}

fn extract_zip(path: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    fs::create_dir_all(dest)?;
    // A bundle travels between machines: cap what it may unpack.
    const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
    const MAX_ENTRIES: usize = 200_000;
    let mut total = 0u64;
    for i in 0..archive.len() {
        if i >= MAX_ENTRIES {
            return Err(format!("the bundle has more than {MAX_ENTRIES} entries").into());
        }
        let mut entry = archive.by_index(i)?;
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(name);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = fs::File::create(&out)?;
            total = total.saturating_add(io::copy(&mut entry, &mut file)?);
            if total > MAX_BYTES {
                return Err(format!(
                    "the bundle unpacks to more than {} GiB",
                    MAX_BYTES / (1024 * 1024 * 1024)
                )
                .into());
            }
        }
    }
    Ok(())
}

/// The status the bundle carries: no server address, players, name tags or
/// clipboard (an invite code lives there).
fn bundle_status(status: &Value) -> Value {
    let mut status = status.clone();
    if let Some(obj) = status.as_object_mut() {
        obj.remove("server");
        obj.remove("name_tags");
        obj.remove("sidebar");
        obj.remove("input");
        if let Some(debug) = obj.get_mut("debug").and_then(Value::as_object_mut) {
            debug.remove("clipboard");
            debug.remove("clipboard_text");
        }
    }
    status
}

/// The places and names the bundle's text must not say: the user folder, the
/// world and the server it was played on, and the other players' names.
fn bundle_scrubber(dir: &Path, status: &Value) -> Scrubber {
    let mut terms = vec![dir.display().to_string()];
    for path in ["/server/address", "/server/name", "/input/recording/world"] {
        if let Some(value) = status.pointer(path).and_then(Value::as_str) {
            terms.push(value.to_owned());
        }
    }
    if let Some(players) = status.pointer("/server/players").and_then(Value::as_array) {
        for player in players {
            if let Some(name) = player.get("name").and_then(Value::as_str) {
                terms.push(name.to_owned());
            }
        }
    }
    // A stopped recording's status no longer carries these.
    if let Ok(text) = fs::read_to_string(dir.join("session.json"))
        && let Ok(meta) = serde_json::from_str::<Value>(&text)
    {
        for path in [
            "/world",
            "/start/world",
            "/start/server/name",
            "/start/server/address",
        ] {
            if let Some(value) = meta.pointer(path).and_then(Value::as_str) {
                terms.push(value.to_owned());
            }
        }
    }
    let mut scrubber = Scrubber::new();
    for term in terms {
        scrubber = scrubber.term(term);
    }
    scrubber
}

fn unique_dest(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repro".into());
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "zip".into());
    for index in 1.. {
        let candidate = path.with_file_name(format!("{stem} ({index}).{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

fn write_bundle(
    dir: &Path,
    dest: &Path,
    status: &Value,
    home: &Path,
    no_world: bool,
    started_at: SystemTime,
) -> Result<u64> {
    let prefix = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "recording".into());
    let dest = unique_dest(dest);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_file_name(format!(
        "{}.tmp",
        dest.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repro.zip".into())
    ));
    let scrubber = bundle_scrubber(dir, status);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let result = (|| -> Result<u64> {
        let mut zip = zip::ZipWriter::new(&mut file);
        // The recording's own files: text, so they are scrubbed as they go in.
        for name in [
            "session.json",
            "inputs.jsonl",
            "state.jsonl",
            "events.jsonl",
        ] {
            let path = dir.join(name);
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            add_bytes(
                &mut zip,
                &format!("{prefix}/{name}"),
                scrubber.scrub(&text).as_bytes(),
            )?;
        }
        add_bytes(
            &mut zip,
            &format!("{prefix}/README.txt"),
            readme().as_bytes(),
        )?;
        // The world is binary and stays as it is.
        let world = dir.join("world");
        if !no_world && world.is_dir() {
            world_management::write_tree_skipping(
                &mut zip,
                &world,
                &world,
                &format!("{prefix}/world"),
                &[],
            )?;
        }
        let log = horizon_client::logging::logs_dir(home).join("latest.log");
        if log.is_file() {
            add_text_file(&mut zip, &log, "logs/latest.log", &scrubber)?;
        }
        // Only a crash from this recording is evidence for this repro.
        if let Some(crash) = horizon_client::logging::newest_crash_report(home)
            && crash
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|modified| modified >= started_at - Duration::from_secs(60))
        {
            let name = crash
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "crash.txt".into());
            add_text_file(
                &mut zip,
                &crash,
                &format!("crash-reports/{name}"),
                &scrubber,
            )?;
        }
        add_bytes(
            &mut zip,
            "status.json",
            scrubber
                .scrub(&serde_json::to_string_pretty(&bundle_status(status))?)
                .as_bytes(),
        )?;
        Ok(zip.finish()?.metadata()?.len())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return result;
    }
    fs::rename(&tmp, &dest)?;
    result
}

fn add_text_file<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &Path,
    name: &str,
    scrubber: &Scrubber,
) -> Result<()> {
    let text = String::from_utf8_lossy(&fs::read(path)?).into_owned();
    add_bytes(zip, name, scrubber.scrub(&text).as_bytes())
}

fn add_bytes<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    bytes: &[u8],
) -> Result<()> {
    zip.start_file(
        name,
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    )?;
    zip.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_folder_slugs() {
        assert_eq!(slug("My Bug!"), "my-bug");
        assert_eq!(slug("  "), "session");
        assert_eq!(slug("a/b"), "a-b");
    }

    #[test]
    fn a_world_copy_skips_the_lock_and_symlinks() {
        let root = std::env::temp_dir().join(format!("horizon-repro-test-{}", std::process::id()));
        let src = root.join("src");
        let dst = root.join("dst");
        fs::create_dir_all(src.join("region")).unwrap();
        fs::write(src.join("level.dat"), b"level").unwrap();
        fs::write(src.join("session.lock"), b"lock").unwrap();
        fs::write(src.join("region/world.mca"), b"region").unwrap();
        copy_tree(&src, &dst).unwrap();
        assert!(dst.join("level.dat").is_file());
        assert!(dst.join("region/world.mca").is_file());
        assert!(!dst.join("session.lock").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_trace_event_round_trips_through_an_inputs_line() {
        let event = TraceEvent {
            seq: 3,
            tick: 42,
            ms: 137,
            origin: InputOrigin::User,
            kind: TraceKind::Key {
                key: "key.keyboard.w".into(),
                action: "down".into(),
            },
        };
        let line = serde_json::to_string(&event).unwrap();
        let back: TraceEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(back.seq, 3);
        assert_eq!(back.origin, InputOrigin::User);
        assert!(matches!(back.kind, TraceKind::Key { ref key, .. } if key == "key.keyboard.w"));
    }

    #[test]
    fn every_trace_kind_survives_a_hand_written_line() {
        let lines = [
            r#"{"seq":1,"tick":1,"ms":0,"origin":"user","kind":"key","key":"key.keyboard.w","action":"down"}"#,
            r#"{"seq":2,"tick":2,"ms":10,"origin":"user","kind":"mouse","button":"key.mouse.left","action":"down"}"#,
            r#"{"seq":3,"tick":3,"ms":20,"origin":"user","kind":"wheel","x":0.0,"y":1.0,"lines":true}"#,
            r#"{"seq":4,"tick":4,"ms":30,"origin":"user","kind":"cursor","x":1.5,"y":2.5}"#,
            r#"{"seq":5,"tick":5,"ms":40,"origin":"user","kind":"motion","x":10.0,"y":-2.0}"#,
            r#"{"seq":6,"tick":6,"ms":50,"origin":"agent","kind":"pad","button":"south","action":"down"}"#,
            r#"{"seq":7,"tick":7,"ms":60,"origin":"agent","kind":"command","line":"key w down"}"#,
        ];
        let events: Vec<TraceEvent> = lines
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events.len(), 7);
        // Each kind keeps what the replay needs.
        assert!(matches!(events[0].kind, TraceKind::Key { .. }));
        assert!(matches!(events[1].kind, TraceKind::Mouse { .. }));
        assert!(matches!(
            events[2].kind,
            TraceKind::Wheel { lines: true, .. }
        ));
        assert!(matches!(events[3].kind, TraceKind::Cursor { .. }));
        assert!(matches!(events[4].kind, TraceKind::Motion { .. }));
        assert!(matches!(events[5].kind, TraceKind::Pad { .. }));
        assert!(matches!(events[6].kind, TraceKind::Command { .. }));
    }

    #[test]
    fn only_input_commands_replay() {
        for name in ["key", "mouse", "look", "click-slot", "goto", "pad"] {
            assert!(REPLAY_COMMANDS.contains(&name), "{name}");
        }
        for name in [
            "world-delete",
            "world-rename",
            "connect",
            "chat",
            "command",
            "quit",
        ] {
            assert!(!REPLAY_COMMANDS.contains(&name), "{name}");
        }
    }

    #[test]
    fn the_bundle_status_loses_the_server_and_clipboard() {
        let status = json!({
            "server": { "address": "mc.example.net", "players": [{"name": "Alice"}] },
            "name_tags": ["Alice"],
            "sidebar": "scores",
            "input": { "recording": { "dir": "/home/alice/recordings/x" } },
            "debug": { "clipboard": "invite-code", "fps": 60 },
            "fps": 60,
        });
        let reduced = bundle_status(&status);
        assert!(reduced.get("server").is_none());
        assert!(reduced.get("name_tags").is_none());
        assert!(reduced.get("sidebar").is_none());
        assert!(reduced.get("input").is_none());
        assert!(reduced.pointer("/debug/clipboard").is_none());
        assert_eq!(reduced["fps"], 60);
    }

    #[test]
    fn the_scrubber_hides_the_world_and_the_user() {
        let status = json!({
            "server": { "address": "mc.example.net", "name": "Example" },
            "input": { "recording": { "world": "Secret World" } },
        });
        let scrubber = bundle_scrubber(Path::new("/home/alice/recordings/x"), &status);
        let text = scrubber.scrub("play on Example at mc.example.net in Secret World, /home/alice");
        assert!(!text.contains("Example"), "{text}");
        assert!(!text.contains("mc.example.net"), "{text}");
        assert!(!text.contains("Secret World"), "{text}");
        assert!(!text.contains("/home/alice"), "{text}");
    }

    #[test]
    fn an_export_name_that_is_taken_gets_a_number() {
        let root = std::env::temp_dir().join(format!("horizon-repro-dest-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let first = root.join("repro.zip");
        fs::write(&first, b"x").unwrap();
        assert_eq!(unique_dest(&first), root.join("repro (1).zip"));
        let _ = fs::remove_dir_all(&root);
    }
}
