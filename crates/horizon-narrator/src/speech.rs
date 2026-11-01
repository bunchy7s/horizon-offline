//! `com.mojang.text2speech.Narrator`: the platform's speech, opened and driven
//! on a thread of its own so the game never waits on it.
use std::sync::mpsc::{self, Receiver, Sender};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(test, target_os = "linux"))]
mod linux_split;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

/// A platform's speech (`NarratorWindows`, `NarratorMac`, `NarratorLinux`),
/// used only on the thread that opened it.
pub trait Speech {
    /// `say`: `interrupt` drops what is queued and cuts off what is being said.
    fn say(&mut self, message: &str, interrupt: bool, volume: f32);
    /// `clear`: drops what is queued and stops speaking.
    fn clear(&mut self);
    /// Called every 50 ms on the speech thread, for speech that keeps its own
    /// queue.
    fn idle(&mut self) {}
}

/// `Narrator.getNarrator`: the platform's speech, or why there is none
/// (`Narrator.EMPTY`).
fn open() -> Result<Box<dyn Speech>, String> {
    #[cfg(windows)]
    return windows::Sapi::open().map(|s| Box::new(s) as Box<dyn Speech>);
    #[cfg(target_os = "macos")]
    return mac::Synthesizer::open().map(|s| Box::new(s) as Box<dyn Speech>);
    #[cfg(target_os = "linux")]
    return linux::Flite::open().map(|s| Box::new(s) as Box<dyn Speech>);
    #[allow(unreachable_code)]
    Err(format!("Unsupported platform {}", std::env::consts::OS))
}

/// Speech that is there but makes no sound (`HORIZON_NO_AUDIO`).
struct Silent;
impl Speech for Silent {
    fn say(&mut self, _: &str, _: bool, _: f32) {}
    fn clear(&mut self) {}
}

enum Command {
    Say(String, bool, f32),
    Clear,
}

/// The game's handle on the speech thread.
pub(crate) struct Thread {
    commands: Sender<Command>,
    /// Whether the speech opened, until [`super::Sink::active`] first asks.
    opened: Option<Receiver<bool>>,
    active: bool,
}

impl Thread {
    pub(crate) fn spawn(silent: bool) -> Thread {
        let (commands, receive) = mpsc::channel();
        let (opened, ready) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("Narrator".into())
            .spawn(move || run(silent, &opened, &receive));
        if let Err(e) = spawned {
            log::error!("Error while loading the narrator: {e}");
        }
        Thread {
            commands,
            opened: Some(ready),
            active: false,
        }
    }
}

fn run(silent: bool, opened: &Sender<bool>, commands: &Receiver<Command>) {
    let mut speech: Box<dyn Speech> = if silent {
        Box::new(Silent)
    } else {
        match open() {
            Ok(speech) => speech,
            Err(e) => {
                log::error!("Error while loading the narrator: {e}");
                let _ = opened.send(false);
                return;
            }
        }
    };
    let _ = opened.send(true);
    const IDLE: std::time::Duration = std::time::Duration::from_millis(50);
    let mut last_idle = std::time::Instant::now();
    loop {
        match commands.recv_timeout(IDLE.saturating_sub(last_idle.elapsed())) {
            Ok(Command::Say(message, interrupt, volume)) => speech.say(&message, interrupt, volume),
            Ok(Command::Clear) => speech.clear(),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // `destroy`: the game dropped its handle.
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                speech.clear();
                return;
            }
        }
        // Every 50 ms, however many commands arrive in between.
        if last_idle.elapsed() >= IDLE {
            last_idle = std::time::Instant::now();
            speech.idle();
        }
    }
}

impl super::Sink for Thread {
    fn say(&mut self, message: &str, interrupt: bool, volume: f32) {
        let _ = self
            .commands
            .send(Command::Say(message.to_owned(), interrupt, volume));
    }
    fn clear(&mut self) {
        let _ = self.commands.send(Command::Clear);
    }
    /// Waits for the speech to open the first time it is asked (vanilla opens
    /// it in `Minecraft`'s constructor, before anything asks).
    fn active(&mut self) -> bool {
        if let Some(opened) = self.opened.take() {
            self.active = opened.recv().unwrap_or(false);
        }
        self.active
    }
}
