//! `NarratorMac`: AppKit's `NSSpeechSynthesizer`, the voice and rate chosen in
//! the system's spoken content settings, with a queue of its own.
//!
//! Vanilla is the synthesizer's delegate: `speechSynthesizer:didFinishSpeaking:`
//! starts the next queued message. AppKit delivers that callback through the
//! run loop of the thread that speaks (the game's main thread in vanilla); the
//! speech thread only turns its run loop over in `idle`, so `idle` also asks the synthesizer every 50 ms whether
//! it still speaks and runs `didFinishSpeaking` once it no longer does.
//! `stopSpeaking` ends the speech too, and vanilla's delegate then fires as
//! well (later, from the run loop), so after an interrupt or a `clear` the
//! next queued message starts at the next `idle`, in the queue's order: an
//! interrupting message is queued behind the ones already waiting while the
//! cut-off speech has not been reported finished.
//!
//! `crashed` is not ported: vanilla stops narrating for good when a message to
//! the synthesizer throws through its Java bridge. These messages raise no
//! Objective-C exception; one would end the game, as objc2 catches them only
//! with its C exception helper.
#![allow(deprecated)] // NSSpeechSynthesizer, deprecated for AVSpeechSynthesizer; vanilla's.

use objc2::rc::{Retained, autoreleasepool};
use objc2::{AnyThread, msg_send};
use objc2_app_kit::NSSpeechSynthesizer;
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop, NSString};

use std::collections::VecDeque;

pub(super) struct Synthesizer {
    synth: Retained<NSSpeechSynthesizer>,
    /// `queue`: the messages waiting for the one being said.
    queue: VecDeque<String>,
    /// `speaking`: a message was started and its `didFinishSpeaking` has not
    /// run yet.
    speaking: bool,
}

impl Synthesizer {
    /// `NSSpeechSynthesizer alloc` then `init`: the default voice.
    pub(super) fn open() -> Result<Synthesizer, String> {
        // SAFETY: `init` takes no arguments and returns the initialised
        // synthesizer, or nil when it could not be created.
        let synth: Option<Retained<NSSpeechSynthesizer>> =
            unsafe { msg_send![NSSpeechSynthesizer::alloc(), init] };
        let synth = synth.ok_or("Failed to create `NSSpeechSynthesizer`")?;
        Ok(Synthesizer {
            synth,
            queue: VecDeque::new(),
            speaking: false,
        })
    }

    /// `startSpeakingString:`, whose answer vanilla ignores.
    fn start_speaking(&self, message: &str) {
        self.synth.startSpeakingString(&NSString::from_str(message));
    }

    /// `speechSynthesizer:didFinishSpeaking:`: the next queued message, or
    /// nothing left to say.
    fn did_finish_speaking(&mut self) {
        match self.queue.pop_front() {
            Some(next) => self.start_speaking(&next),
            None => self.speaking = false,
        }
    }
}

impl super::Speech for Synthesizer {
    /// The volume is set first; interrupting stops what is being said but
    /// keeps the queue; the message waits its turn while one is being said.
    fn say(&mut self, message: &str, interrupt: bool, volume: f32) {
        autoreleasepool(|_| {
            self.synth.setVolume(volume);
            if interrupt {
                self.synth.stopSpeaking();
            }
            if self.speaking {
                self.queue.push_back(message.to_owned());
            } else {
                self.speaking = true;
                self.start_speaking(message);
            }
        });
    }
    /// Empties the queue and stops what is being said.
    fn clear(&mut self) {
        autoreleasepool(|_| {
            self.queue.clear();
            self.synth.stopSpeaking();
        });
    }
    /// The delegate's callback, polled: the speech that was started ended.
    fn idle(&mut self) {
        autoreleasepool(|_| {
            // Lets AppKit deliver what the synthesizer posts to this thread's
            // run loop (vanilla's runs on the game's main thread), without
            // waiting.
            // SAFETY: the current thread's run loop, run in the default mode
            // until a date already past: it handles what is ready and returns.
            unsafe {
                NSRunLoop::currentRunLoop()
                    .runMode_beforeDate(NSDefaultRunLoopMode, &NSDate::distantPast());
            }
            if self.speaking && !self.synth.isSpeaking() {
                self.did_finish_speaking();
            }
        });
    }
}
