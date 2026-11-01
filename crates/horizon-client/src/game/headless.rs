//! `--headless` (#4951): a client with no display, no window and no renderer.
//!
//! winit's event loop needs a display server even when no window is shown, so a headless game
//! runs this loop instead. Each step is what winit's `about_to_wait` does (`Game::idle`: stop
//! requests, the closing save, world entry, the control socket), and `idle` runs the frame
//! (the client tick, nothing drawn) through the same `RedrawRequested` path a window's game
//! takes. Input comes only from the control socket (`horizon-ctl`), as synthetic window events.

use super::Game;
use super::event_loop::EventLoopCtl;
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use winit::event_loop::ControlFlow;

/// Time between headless frames. The client ticks at 20 Hz whatever the frame rate (a frame
/// runs the ticks its elapsed time owes), so one frame per tick does all the work there is
/// without spending a core on frames nobody sees.
const FRAME: Duration = Duration::from_millis(50);

/// The loop's side of `EventLoopCtl`: what the game asked for during a step.
struct HeadlessLoop {
    exit: Cell<bool>,
    flow: Cell<ControlFlow>,
}

impl EventLoopCtl for HeadlessLoop {
    fn exit(&self) {
        self.exit.set(true);
    }
    fn set_control_flow(&self, flow: ControlFlow) {
        self.flow.set(flow);
    }
}

impl HeadlessLoop {
    /// When the step that started at `started` wants the next one: at the time the game asked
    /// for (`WaitUntil`, as the closing save's polls do), else one frame later.
    fn next_step(&self, started: Instant) -> Instant {
        match self.flow.get() {
            ControlFlow::WaitUntil(at) => at,
            ControlFlow::Poll | ControlFlow::Wait => started + FRAME,
        }
    }
}

impl Game<'_> {
    /// Runs the game until it asks to stop (`horizon-ctl quit`, a signal, an error), then
    /// saves as a window's game does when its loop ends.
    pub(super) fn run_headless(&mut self) {
        let ctl = HeadlessLoop {
            exit: Cell::new(false),
            flow: Cell::new(ControlFlow::Poll),
        };
        self.previous = Instant::now();
        while !ctl.exit.get() {
            let started = Instant::now();
            ctl.flow.set(ControlFlow::Poll);
            self.idle(&ctl);
            if let Some(wait) = ctl
                .next_step(started)
                .checked_duration_since(Instant::now())
            {
                std::thread::sleep(wait);
            }
        }
        self.shut_down();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl(flow: ControlFlow) -> HeadlessLoop {
        HeadlessLoop {
            exit: Cell::new(false),
            flow: Cell::new(flow),
        }
    }

    #[test]
    fn steps_come_a_frame_apart_unless_the_game_asks_for_sooner() {
        let started = Instant::now();
        assert_eq!(ctl(ControlFlow::Poll).next_step(started), started + FRAME);
        let soon = started + Duration::from_millis(4);
        assert_eq!(ctl(ControlFlow::WaitUntil(soon)).next_step(started), soon);
    }

    #[test]
    fn exit_is_remembered_for_the_loop() {
        let c = ctl(ControlFlow::Poll);
        assert!(!c.exit.get());
        EventLoopCtl::exit(&c);
        assert!(c.exit.get());
    }
}
