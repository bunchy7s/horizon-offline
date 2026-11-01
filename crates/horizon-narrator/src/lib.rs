//! The narrator: vanilla's text-to-speech (`GameNarrator` over
//! `com.mojang.text2speech.Narrator` 1.19.12) and the screen narration state
//! (`ScreenNarrationCollector` and `Screen`'s narration timing, [`screen`]).
//!
//! One narrator per process, as `Minecraft.narrator`: [`init`] opens the
//! platform's speech on its own thread (SAPI on Windows, NSSpeechSynthesizer
//! on macOS, flite on Linux, as text2speech does), and the free functions are
//! `GameNarrator`'s methods. Speaking never blocks the caller: the speech
//! thread takes the calls in order. Until [`init`] (the dedicated server,
//! tests) the narrator is inactive, as text2speech's `Narrator.EMPTY`.
//!
//! The game keeps the narrator's [`Status`] and voice volume in step with its
//! options ([`set_status`], [`set_volume`]); the narrator reads them where
//! `GameNarrator` reads `options.narrator()` and
//! `getFinalSoundSourceVolume(VOICE)`.

pub mod screen;
mod speech;

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

/// `NarratorStatus`, by its `options.txt` id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Status {
    #[default]
    Off,
    All,
    Chat,
    System,
}

impl Status {
    pub const ALL: [Status; 4] = [Status::Off, Status::All, Status::Chat, Status::System];

    /// `byId`: `ByIdMap.continuous` with `OutOfBoundsStrategy.WRAP`.
    pub fn by_id(id: i32) -> Status {
        Self::ALL[id.rem_euclid(4) as usize]
    }
    pub fn id(self) -> i32 {
        self as i32
    }
    /// The status the narrator hotkey moves to (`byId(getId() + 1)`).
    pub fn next(self) -> Status {
        Self::by_id(self.id() + 1)
    }
    /// The translation key of `getName`.
    pub fn key(self) -> &'static str {
        match self {
            Status::Off => "options.narrator.off",
            Status::All => "options.narrator.all",
            Status::Chat => "options.narrator.chat",
            Status::System => "options.narrator.system",
        }
    }
    pub fn should_narrate_chat(self) -> bool {
        matches!(self, Status::All | Status::Chat)
    }
    pub fn should_narrate_system(self) -> bool {
        matches!(self, Status::All | Status::System)
    }
    pub fn should_narrate_system_or_chat(self) -> bool {
        matches!(self, Status::All | Status::System | Status::Chat)
    }
}

/// What `GameNarrator` hands its `Narrator`: the speech thread, or a test's
/// recorder.
pub trait Sink: Send {
    fn say(&mut self, message: &str, interrupt: bool, volume: f32);
    fn clear(&mut self);
    /// `Narrator.active`: false for `Narrator.EMPTY`, which a platform without
    /// speech (or whose speech failed to open) gets.
    fn active(&mut self) -> bool;
}

/// How many narrated messages [`recent`] keeps.
const RECENT: usize = 32;

/// `GameNarrator`, with its options read from [`Status`] and the volume the
/// game sets.
pub struct GameNarrator {
    sink: Option<Box<dyn Sink>>,
    status: Status,
    volume: f32,
    /// The messages the narrator was asked to say, oldest first (vanilla only
    /// logs them in its IDE): the control socket and tests read them.
    recent: VecDeque<String>,
}

impl GameNarrator {
    pub const fn new() -> Self {
        Self {
            sink: None,
            status: Status::Off,
            volume: 1.0,
            recent: VecDeque::new(),
        }
    }
    pub fn with_sink(sink: Box<dyn Sink>) -> Self {
        Self {
            sink: Some(sink),
            ..Self::new()
        }
    }
    pub fn set_sink(&mut self, sink: Box<dyn Sink>) {
        self.sink = Some(sink);
    }
    pub fn set_status(&mut self, status: Status) {
        self.status = status;
    }
    pub fn status(&self) -> Status {
        self.status
    }
    /// `getFinalSoundSourceVolume(SoundSource.VOICE)`: master times voice.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    pub fn say_chat_queued(&mut self, message: &str) {
        if self.status.should_narrate_chat() {
            self.narrate_not_interrupting(message);
        }
    }
    pub fn say_system_chat_queued(&mut self, message: &str) {
        if self.status.should_narrate_system_or_chat() {
            self.narrate_not_interrupting(message);
        }
    }
    pub fn say_system_queued(&mut self, message: &str) {
        if self.status.should_narrate_system() {
            self.narrate_not_interrupting(message);
        }
    }
    fn narrate_not_interrupting(&mut self, message: &str) {
        if !message.is_empty() {
            self.log(message);
            self.narrate(message, false);
        }
    }
    /// `saySystemNow`: what is said now cuts off whatever is being said.
    pub fn say_system_now(&mut self, message: &str) {
        if self.status.should_narrate_system() && !message.is_empty() {
            self.log(message);
            if self.is_active() {
                if let Some(sink) = &mut self.sink {
                    sink.clear();
                }
                self.narrate(message, true);
            }
        }
    }
    fn narrate(&mut self, message: &str, interrupt: bool) {
        let volume = self.volume;
        if let Some(sink) = &mut self.sink {
            sink.say(message, interrupt, volume);
        }
    }
    fn log(&mut self, message: &str) {
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(message.to_owned());
    }

    /// `updateNarratorStatus`, once the option holds `status`: clears, then
    /// says `announcement` (`options.narrator` + " : " + the status's name)
    /// whatever the status, `Off` included. The toast that goes with it is
    /// the caller's ([`toast`]).
    pub fn update_status(&mut self, status: Status, announcement: &str) {
        self.status = status;
        self.clear();
        self.log(announcement);
        self.narrate(announcement, true);
    }

    pub fn is_active(&mut self) -> bool {
        self.sink.as_mut().is_some_and(|sink| sink.active())
    }
    pub fn clear(&mut self) {
        if self.status != Status::Off && self.is_active() {
            if let Some(sink) = &mut self.sink {
                sink.clear();
            }
        }
    }
    pub fn recent(&self) -> Vec<String> {
        self.recent.iter().cloned().collect()
    }
}

impl Default for GameNarrator {
    fn default() -> Self {
        Self::new()
    }
}

/// The toast `updateNarratorStatus` shows (`SystemToastId.NARRATOR_TOGGLE`):
/// translation keys of its title and of its message, if it has one.
pub fn toast(status: Status, active: bool) -> (&'static str, Option<&'static str>) {
    if !active {
        (
            "narrator.toast.disabled",
            Some("options.narrator.notavailable"),
        )
    } else if status == Status::Off {
        ("narrator.toast.disabled", None)
    } else {
        ("narrator.toast.enabled", Some(status.key()))
    }
}

static NARRATOR: Mutex<GameNarrator> = Mutex::new(GameNarrator::new());

fn narrator() -> MutexGuard<'static, GameNarrator> {
    NARRATOR.lock().unwrap_or_else(|e| e.into_inner())
}

/// `new GameNarrator`: opens the platform's speech on its own thread. With
/// `HORIZON_NO_AUDIO` set the narrator is active but silent, so scripted runs
/// still narrate into [`recent`]. Calling it again does nothing.
pub fn init() {
    let mut narrator = narrator();
    if narrator.sink.is_none() {
        narrator.sink = Some(Box::new(speech::Thread::spawn(
            std::env::var_os("HORIZON_NO_AUDIO").is_some(),
        )));
    }
}

/// `destroy`: stops speaking and closes the speech.
pub fn destroy() {
    if let Some(mut sink) = narrator().sink.take() {
        sink.clear();
    }
}

pub fn set_status(status: Status) {
    narrator().set_status(status);
}
pub fn status() -> Status {
    narrator().status()
}
pub fn set_volume(volume: f32) {
    narrator().set_volume(volume);
}
pub fn is_active() -> bool {
    narrator().is_active()
}
pub fn say_chat_queued(message: &str) {
    narrator().say_chat_queued(message);
}
pub fn say_system_chat_queued(message: &str) {
    narrator().say_system_chat_queued(message);
}
pub fn say_system_queued(message: &str) {
    narrator().say_system_queued(message);
}
pub fn say_system_now(message: &str) {
    narrator().say_system_now(message);
}
pub fn update_status(status: Status, announcement: &str) {
    narrator().update_status(status, announcement);
}
pub fn clear() {
    narrator().clear();
}
/// The last messages the narrator was asked to say, oldest first.
pub fn recent() -> Vec<String> {
    narrator().recent()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, PartialEq)]
    enum Call {
        Say(String, bool, f32),
        Clear,
    }

    #[derive(Clone, Default)]
    struct Recorder {
        calls: Arc<Mutex<Vec<Call>>>,
        inactive: bool,
    }
    impl Recorder {
        fn take(&self) -> Vec<Call> {
            std::mem::take(&mut self.calls.lock().unwrap())
        }
    }
    impl Sink for Recorder {
        fn say(&mut self, message: &str, interrupt: bool, volume: f32) {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Say(message.into(), interrupt, volume));
        }
        fn clear(&mut self) {
            self.calls.lock().unwrap().push(Call::Clear);
        }
        fn active(&mut self) -> bool {
            !self.inactive
        }
    }

    fn narrator(status: Status) -> (GameNarrator, Recorder) {
        let recorder = Recorder::default();
        let mut n = GameNarrator::with_sink(Box::new(recorder.clone()));
        n.set_status(status);
        (n, recorder)
    }

    #[test]
    fn statuses_wrap_through_their_ids_as_the_hotkey_cycles_them() {
        assert_eq!(Status::by_id(0), Status::Off);
        assert_eq!(Status::by_id(3), Status::System);
        assert_eq!(Status::by_id(4), Status::Off);
        assert_eq!(Status::by_id(-1), Status::System);
        assert_eq!(Status::Off.next(), Status::All);
        assert_eq!(Status::System.next(), Status::Off);
    }

    #[test]
    fn each_status_narrates_its_own_kinds_of_message() {
        for (status, chat, system, either) in [
            (Status::Off, false, false, false),
            (Status::All, true, true, true),
            (Status::Chat, true, false, true),
            (Status::System, false, true, true),
        ] {
            assert_eq!(status.should_narrate_chat(), chat);
            assert_eq!(status.should_narrate_system(), system);
            assert_eq!(status.should_narrate_system_or_chat(), either);
        }
    }

    #[test]
    fn queued_messages_wait_their_turn_and_empty_ones_are_skipped() {
        let (mut n, r) = narrator(Status::Chat);
        n.set_volume(0.5);
        n.say_chat_queued("<Steve> hi");
        n.say_system_queued("not in chat mode");
        n.say_system_chat_queued("Steve joined the game");
        n.say_chat_queued("");
        assert_eq!(
            r.take(),
            vec![
                Call::Say("<Steve> hi".into(), false, 0.5),
                Call::Say("Steve joined the game".into(), false, 0.5),
            ]
        );
        assert_eq!(n.recent(), ["<Steve> hi", "Steve joined the game"]);
    }

    #[test]
    fn saying_now_clears_and_interrupts_only_for_system_narration() {
        let (mut n, r) = narrator(Status::System);
        n.say_system_now("Options");
        assert_eq!(
            r.take(),
            vec![Call::Clear, Call::Say("Options".into(), true, 1.0)]
        );
        n.set_status(Status::Chat);
        n.say_system_now("Options");
        assert!(r.take().is_empty());
    }

    #[test]
    fn an_inactive_narrator_logs_but_says_nothing() {
        let recorder = Recorder {
            inactive: true,
            ..Default::default()
        };
        let mut n = GameNarrator::with_sink(Box::new(recorder.clone()));
        n.set_status(Status::All);
        n.say_system_now("Options");
        n.clear();
        assert!(recorder.take().is_empty());
        assert_eq!(n.recent(), ["Options"]);
        assert!(!n.is_active());
        assert!(!GameNarrator::new().is_active());
    }

    #[test]
    fn a_status_change_is_announced_even_when_turning_it_off() {
        let (mut n, r) = narrator(Status::All);
        n.update_status(Status::Off, "Narrator : OFF");
        // clear() reads the new status: off, so nothing is cleared first.
        assert_eq!(
            r.take(),
            vec![Call::Say("Narrator : OFF".into(), true, 1.0)]
        );
        n.update_status(Status::All, "Narrator : Narrates All");
        assert_eq!(
            r.take(),
            vec![
                Call::Clear,
                Call::Say("Narrator : Narrates All".into(), true, 1.0)
            ]
        );
    }

    #[test]
    fn the_toggle_toast_names_the_status_or_why_there_is_none() {
        assert_eq!(
            toast(Status::All, false),
            (
                "narrator.toast.disabled",
                Some("options.narrator.notavailable")
            )
        );
        assert_eq!(toast(Status::Off, true), ("narrator.toast.disabled", None));
        assert_eq!(
            toast(Status::Chat, true),
            ("narrator.toast.enabled", Some("options.narrator.chat"))
        );
    }
}
