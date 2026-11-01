//! What the game asks of the loop that runs it.
//!
//! A window's game runs inside winit's event loop, and its methods used to take winit's
//! `ActiveEventLoop` only to stop the loop or to say when to wake next. A headless client
//! (#4951) has no display, so it cannot have that loop: it runs its own, and hands the same
//! methods this trait instead. winit's loop implements it, so a window's game is unchanged.

use winit::event_loop::{ActiveEventLoop, ControlFlow};

pub(super) trait EventLoopCtl {
    /// Ends the loop after the current iteration (`ActiveEventLoop::exit`).
    fn exit(&self);
    /// When the loop wakes next (`ActiveEventLoop::set_control_flow`).
    fn set_control_flow(&self, flow: ControlFlow);
}

impl EventLoopCtl for ActiveEventLoop {
    fn exit(&self) {
        ActiveEventLoop::exit(self);
    }
    fn set_control_flow(&self, flow: ControlFlow) {
        ActiveEventLoop::set_control_flow(self, flow);
    }
}
