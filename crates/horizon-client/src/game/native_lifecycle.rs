//! Read-only lifecycle observations for native mods on authoritative servers.
//! The deprecated in-frame path delivers no lifecycle events. A paused pass
//! observes nothing; a frozen world reports `world_running: false` except during a step.

use std::sync::atomic::{AtomicU64, Ordering};

use super::{Game, execution_role::ExecutionRole};
// Their old home, for mixin code that names them here.
pub(super) use horizon_client::mod_api::{
    NativeAuthority, NativeLifecycleContext, NativeTickId, NativeWorldEpoch, ServerTickPhase,
    WorldPhase,
};

/// Epoch allocation spans `Game` instances (a new server `Game` per
/// launch), so the counter is process-wide.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Internal state for deciding which events can fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenState {
    Opening,
    Ready,
    Closing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpenAttempt {
    authority: NativeAuthority,
    epoch: u64,
    session: OpenState,
    ticks: u64,
    /// A tick started and its end has not fired (or the tick errored).
    tick_open: bool,
    tick_world_running: bool,
}

impl OpenAttempt {
    fn context(&self) -> NativeLifecycleContext {
        NativeLifecycleContext {
            authority: self.authority,
            epoch: NativeWorldEpoch::new(self.epoch),
            tick: NativeTickId::new(self.ticks),
        }
    }
}

/// The lifecycle bookkeeping: the current attempt's epoch, session phase
/// and non-paused tick count. It only decides whether a phase fires; every
/// hook body is empty, so with no native mods nothing changes.
#[derive(Default)]
pub(super) struct NativeLifecycle {
    attempt: Option<OpenAttempt>,
    read_scope: bool,
}

impl NativeLifecycle {
    /// A read is valid only during a tick hook with its current tick context.
    pub(super) fn permits_read(&self, context: &NativeLifecycleContext) -> bool {
        self.read_scope
            && self
                .attempt
                .as_ref()
                .is_some_and(|a| a.session == OpenState::Ready && a.context() == *context)
    }

    /// The open starts: a fresh process-wide epoch, never reused.
    pub(super) fn begin_open(&mut self, authority: NativeAuthority) {
        let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
        self.attempt = Some(OpenAttempt {
            authority,
            epoch,
            session: OpenState::Opening,
            ticks: 0,
            tick_open: false,
            tick_world_running: false,
        });
    }

    pub(super) fn world_open_ready(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Opening {
            return None;
        }
        attempt.session = OpenState::Ready;
        Some(attempt.context())
    }

    /// `WorldOpenFailed`: only while opening; the attempt is invalidated.
    pub(super) fn world_open_failed(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Opening {
            return None;
        }
        let context = attempt.context();
        self.attempt = None;
        Some(context)
    }

    pub(super) fn begin_close(&mut self) {
        if let Some(attempt) = self.attempt.as_mut() {
            attempt.session = OpenState::Closing;
        }
    }

    /// A failure after readiness retires the epoch, including during shutdown.
    pub(super) fn world_close_failed(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_ref()?;
        if attempt.session == OpenState::Opening {
            return None;
        }
        let context = attempt.context();
        self.attempt = None;
        Some(context)
    }

    /// `WorldCloseFinal`: at most once per epoch; the epoch retires.
    pub(super) fn world_close_final(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Closing {
            return None;
        }
        let context = attempt.context();
        self.attempt = None;
        Some(context)
    }

    /// Non-paused ready ticks only; a paused pass observes nothing, does not count.
    pub(super) fn server_tick_start(
        &mut self,
        world_running: bool,
    ) -> Option<(NativeLifecycleContext, ServerTickPhase)> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != OpenState::Ready {
            return None;
        }
        attempt.ticks += 1;
        attempt.tick_open = true;
        attempt.tick_world_running = world_running;
        Some((
            attempt.context(),
            ServerTickPhase::ServerTickStart { world_running },
        ))
    }

    /// Discard the pending end when a tick fails.
    pub(super) fn cancel_server_tick(&mut self) {
        if let Some(attempt) = &mut self.attempt {
            attempt.tick_open = false;
        }
    }

    /// Pairs the last start; an errored tick never reaches it.
    pub(super) fn server_tick_end(
        &mut self,
        paused: bool,
    ) -> Option<(NativeLifecycleContext, ServerTickPhase)> {
        let attempt = self.attempt.as_mut()?;
        if paused || attempt.session != OpenState::Ready || !attempt.tick_open {
            return None;
        }
        attempt.tick_open = false;
        Some((
            attempt.context(),
            ServerTickPhase::ServerTickEnd {
                world_running: attempt.tick_world_running,
            },
        ))
    }
}

impl Game<'_> {
    /// The role is fixed at construction; a mirror `Game` is remote whatever it is.
    fn native_lifecycle_authority(&self) -> NativeAuthority {
        if self.remote.is_some() {
            return NativeAuthority::RemoteClient;
        }
        match self.options.execution_role {
            ExecutionRole::Client => NativeAuthority::LocalAuthoritative,
            ExecutionRole::IntegratedServer => NativeAuthority::IntegratedServer,
            ExecutionRole::DedicatedServer => NativeAuthority::DedicatedServer,
        }
    }

    pub(super) fn native_lifecycle_open_started(&mut self) {
        self.native_lifecycle
            .begin_open(self.native_lifecycle_authority());
    }

    pub(super) fn native_lifecycle_world_ready(&mut self) {
        if let Some(context) = self.native_lifecycle.world_open_ready() {
            self.native_world_lifecycle(&context, WorldPhase::WorldOpenReady, "");
        }
    }

    /// `error_text` is the engine's own error text, unchanged.
    pub(super) fn native_lifecycle_world_failed(&mut self, error_text: &str) {
        // The levels that opened before the failure close first.
        self.native_levels_closing();
        if let Some(context) = self.native_lifecycle.world_open_failed() {
            self.native_world_lifecycle(&context, WorldPhase::WorldOpenFailed, error_text);
        }
    }

    /// Report a server-loop or shutdown failure once and retire its epoch.
    pub(super) fn native_lifecycle_close_failed(&mut self, error_text: &str) {
        self.native_levels_closing();
        if let Some(context) = self.native_lifecycle.world_close_failed() {
            self.native_world_lifecycle(&context, WorldPhase::WorldCloseFailed, error_text);
        }
    }

    /// After the final save, before the lock release. Mirrors never fire.
    pub(super) fn native_lifecycle_world_closed(&mut self) {
        if self.remote.is_some() {
            return;
        }
        // Each level closes after the final save (`LevelEvent.Unload`).
        self.native_levels_closing();
        if let Some(context) = self.native_lifecycle.world_close_final() {
            self.native_world_lifecycle(&context, WorldPhase::WorldCloseFinal, "");
        }
    }

    pub(super) fn native_lifecycle_server_tick_start(&mut self, world_running: bool) {
        if let Some((context, phase)) = self.native_lifecycle.server_tick_start(world_running) {
            self.native_lifecycle.read_scope = true;
            self.native_server_tick(&context, phase);
            self.native_lifecycle.read_scope = false;
        }
    }

    /// An errored tick returned at `result?` before this site.
    pub(super) fn native_lifecycle_server_tick_end(&mut self, paused: bool) {
        if let Some((context, phase)) = self.native_lifecycle.server_tick_end(paused) {
            self.native_lifecycle.read_scope = true;
            self.native_server_tick(&context, phase);
            self.native_lifecycle.read_scope = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_steps_are_observed_without_consuming_them() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        let mut clock = horizon_client::clock::FixedStep::default();
        clock.set_frozen(true);
        clock.set_frozen_ticks_to_run(4);
        let mut observed = 0;
        for _ in 0..6 {
            let running = clock.world_runs();
            let (_, start) = lifecycle.server_tick_start(running).unwrap();
            let (_, end) = lifecycle.server_tick_end(false).unwrap();
            assert_eq!(
                start,
                ServerTickPhase::ServerTickStart {
                    world_running: running
                }
            );
            assert_eq!(
                end,
                ServerTickPhase::ServerTickEnd {
                    world_running: running
                }
            );
            observed += usize::from(running);
        }
        assert_eq!(observed, 4);
    }

    /// A lifecycle mid-open on the server thread.
    fn opening() -> NativeLifecycle {
        let mut lifecycle = NativeLifecycle::default();
        lifecycle.begin_open(NativeAuthority::IntegratedServer);
        lifecycle
    }

    #[test]
    fn block_reads_require_the_current_hook_scope() {
        let mut life = NativeLifecycle::default();
        life.begin_open(NativeAuthority::IntegratedServer);
        let ready = life.world_open_ready().unwrap();
        assert!(!life.permits_read(&ready));
        let (tick, _) = life.server_tick_start(true).unwrap();
        life.read_scope = true;
        assert!(life.permits_read(&tick));
        assert!(!life.permits_read(&ready));
        life.read_scope = false;
        assert!(!life.permits_read(&tick));
        life.server_tick_end(false).unwrap();
        life.begin_close();
        life.world_close_final().unwrap();
        life.begin_open(NativeAuthority::IntegratedServer);
        life.world_open_ready().unwrap();
        life.read_scope = true;
        assert!(!life.permits_read(&tick));
    }

    #[test]
    fn world_ready_fires_once_per_epoch() {
        let mut lifecycle = opening();
        let context = lifecycle.world_open_ready().expect("the first ready fires");
        assert_eq!(context.authority, NativeAuthority::IntegratedServer);
        assert!(context.epoch.get() >= 1, "the epoch is globally allocated");
        assert!(lifecycle.world_open_ready().is_none(), "at most once");
    }

    #[test]
    fn a_failed_open_invalidates_and_a_retry_is_fresh() {
        let mut lifecycle = opening();
        let context = lifecycle.world_open_failed().expect("the failure fires");
        let failed_epoch = context.epoch;
        assert!(lifecycle.world_open_ready().is_none(), "the epoch is gone");
        assert!(lifecycle.server_tick_start(true).is_none());
        lifecycle.begin_open(NativeAuthority::IntegratedServer);
        let context = lifecycle.world_open_ready().expect("the retry opens fresh");
        assert!(context.epoch.get() > failed_epoch.get(), "a fresh epoch");
    }

    #[test]
    fn close_final_fires_once_and_retires_the_epoch() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        lifecycle.begin_close();
        let context = lifecycle.world_close_final().expect("the close fires");
        assert!(context.epoch.get() >= 1);
        assert!(
            lifecycle.world_close_final().is_none(),
            "a repeated close request is a no-op"
        );
        assert!(lifecycle.server_tick_start(true).is_none());
        assert!(lifecycle.world_open_ready().is_none());
    }

    #[test]
    fn a_close_during_opening_still_closes_once() {
        let mut lifecycle = opening();
        lifecycle.begin_close();
        assert!(lifecycle.world_close_final().is_some());
    }

    #[test]
    fn close_failure_retires_only_ready_epochs() {
        let mut lifecycle = opening();
        assert!(lifecycle.world_close_failed().is_none());
        lifecycle.world_open_ready();
        lifecycle.begin_close();
        assert!(lifecycle.world_close_failed().is_some());
        assert!(lifecycle.world_close_failed().is_none());
        assert!(lifecycle.world_close_final().is_none());
    }

    #[test]
    fn lifecycle_delivery_stays_off_the_deprecated_in_frame_path() {
        let game = include_str!("../game.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        for site in [
            "native_lifecycle_open_started()",
            "native_lifecycle_world_ready()",
            "native_lifecycle_world_closed()",
            "native_lifecycle.begin_close()",
        ] {
            assert!(!game.contains(site), "in-frame site remains: {site}");
        }
        let saving = include_str!("saving.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        assert!(!saving.contains("native_lifecycle"));

        let integrated = include_str!("integrated.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        let open = integrated
            .find("self.native_lifecycle_open_started();")
            .unwrap();
        let load = integrated
            .find("self.finish_open_world(opening.world?.finish()?)?;")
            .unwrap();
        assert!(open < load);
    }
}
