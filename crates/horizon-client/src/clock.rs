//! `DeltaTracker.Timer` and the tick-rate managers of 26.3: how real
//! milliseconds turn into game ticks, partial ticks and real-time deltas, and
//! the `/tick` freeze, step and sprint state.

/// `DeltaTracker.Timer`: float arithmetic on integer milliseconds, exactly as
/// the client does it.
#[derive(Clone, Debug)]
pub struct Timer {
    delta_ticks: f32,
    delta_tick_residual: f32,
    realtime_delta_ticks: f32,
    paused_delta_tick_residual: f32,
    last_ms: i64,
    last_ui_ms: i64,
    ms_per_tick: f32,
    paused: bool,
    frozen: bool,
}

impl Timer {
    pub fn new(ticks_per_second: f32, current_ms: i64) -> Self {
        Self {
            delta_ticks: 0.0,
            delta_tick_residual: 0.0,
            realtime_delta_ticks: 0.0,
            paused_delta_tick_residual: 0.0,
            last_ms: current_ms,
            last_ui_ms: current_ms,
            ms_per_tick: 1000.0 / ticks_per_second,
            paused: false,
            frozen: false,
        }
    }

    /// `advanceGameTime`: the whole ticks to run now. `target_mspt` maps the
    /// default milliseconds per tick to the target in force (the tick-rate
    /// manager's, when the level runs normally). The caller runs at most ten.
    pub fn advance_game_time(
        &mut self,
        current_ms: i64,
        target_mspt: impl FnOnce(f32) -> f32,
    ) -> i32 {
        self.delta_ticks = (current_ms - self.last_ms) as f32 / target_mspt(self.ms_per_tick);
        self.last_ms = current_ms;
        self.delta_tick_residual += self.delta_ticks;
        let ticks = self.delta_tick_residual as i32;
        self.delta_tick_residual -= ticks as f32;
        ticks
    }

    /// `advanceRealTime`.
    pub fn advance_real_time(&mut self, current_ms: i64) {
        self.realtime_delta_ticks = (current_ms - self.last_ui_ms) as f32 / self.ms_per_tick;
        self.last_ui_ms = current_ms;
    }

    /// `updatePauseState`.
    pub fn update_pause_state(&mut self, pause: bool) {
        if pause {
            if !self.paused {
                self.paused_delta_tick_residual = self.delta_tick_residual;
            }
            self.paused = true;
        } else {
            if self.paused {
                self.delta_tick_residual = self.paused_delta_tick_residual;
            }
            self.paused = false;
        }
    }

    /// `updateFrozenState`.
    pub fn update_frozen_state(&mut self, frozen: bool) {
        self.frozen = frozen;
    }

    pub fn game_time_delta_ticks(&self) -> f32 {
        self.delta_ticks
    }

    /// `getGameTimeDeltaPartialTick`.
    pub fn game_time_delta_partial_tick(&self, ignore_frozen_game: bool) -> f32 {
        if !ignore_frozen_game && self.frozen {
            1.0
        } else if self.paused {
            self.paused_delta_tick_residual
        } else {
            self.delta_tick_residual
        }
    }

    /// `getRealtimeDeltaTicks`.
    pub fn realtime_delta_ticks(&self) -> f32 {
        if self.realtime_delta_ticks > 7.0 {
            0.5
        } else {
            self.realtime_delta_ticks
        }
    }
}

const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;
const NANOSECONDS_PER_MILLISECOND: i64 = 1_000_000;

/// `TickRateManager`.
#[derive(Clone, Debug)]
pub struct TickRateManager {
    tickrate: f32,
    nanoseconds_per_tick: i64,
    frozen_ticks_to_run: i32,
    run_game_elements: bool,
    is_frozen: bool,
}

impl Default for TickRateManager {
    fn default() -> Self {
        Self {
            tickrate: 20.0,
            nanoseconds_per_tick: NANOSECONDS_PER_SECOND / 20,
            frozen_ticks_to_run: 0,
            run_game_elements: true,
            is_frozen: false,
        }
    }
}

impl TickRateManager {
    /// `setTickRate` (`Math.max(rate, 1.0F)`, NaN propagates).
    pub fn set_tick_rate(&mut self, rate: f32) {
        self.tickrate = if rate.is_nan() || rate > 1.0 {
            rate
        } else {
            1.0
        };
        self.nanoseconds_per_tick =
            (NANOSECONDS_PER_SECOND as f64 / f64::from(self.tickrate)) as i64;
    }
    pub fn tickrate(&self) -> f32 {
        self.tickrate
    }
    pub fn milliseconds_per_tick(&self) -> f32 {
        self.nanoseconds_per_tick as f32 / NANOSECONDS_PER_MILLISECOND as f32
    }
    pub fn nanoseconds_per_tick(&self) -> i64 {
        self.nanoseconds_per_tick
    }
    pub fn runs_normally(&self) -> bool {
        self.run_game_elements
    }
    pub fn is_stepping_forward(&self) -> bool {
        self.frozen_ticks_to_run > 0
    }
    pub fn set_frozen_ticks_to_run(&mut self, ticks: i32) {
        self.frozen_ticks_to_run = ticks;
    }
    pub fn frozen_ticks_to_run(&self) -> i32 {
        self.frozen_ticks_to_run
    }
    pub fn set_frozen(&mut self, frozen: bool) {
        self.is_frozen = frozen;
    }
    pub fn is_frozen(&self) -> bool {
        self.is_frozen
    }
    /// `tick`: decides whether game elements run this tick.
    pub fn tick(&mut self) {
        self.run_game_elements = !self.is_frozen || self.frozen_ticks_to_run > 0;
        if self.frozen_ticks_to_run > 0 {
            self.frozen_ticks_to_run -= 1;
        }
    }
}

/// What `/tick sprint` reports when it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SprintReport {
    pub ticks_per_second: i32,
    pub milliseconds_per_tick: String,
}

/// `ServerTickRateManager`: the freeze, step and sprint state kept by the
/// server (the broadcasts to clients are the caller's concern).
#[derive(Clone, Debug, Default)]
pub struct ServerTickRateManager {
    pub base: TickRateManager,
    remaining_sprint_ticks: i64,
    sprint_tick_start_time: i64,
    sprint_time_spend: i64,
    scheduled_current_sprint_ticks: i64,
    previous_is_frozen: bool,
}

impl ServerTickRateManager {
    pub fn is_sprinting(&self) -> bool {
        self.scheduled_current_sprint_ticks > 0
    }
    pub fn remaining_sprint_ticks(&self) -> i64 {
        self.remaining_sprint_ticks
    }
    /// `stepGameIfPaused`.
    pub fn step_game_if_paused(&mut self, ticks: i32) -> bool {
        if !self.base.is_frozen() {
            return false;
        }
        self.base.set_frozen_ticks_to_run(ticks);
        true
    }
    /// `stopStepping`.
    pub fn stop_stepping(&mut self) -> bool {
        if self.base.frozen_ticks_to_run() > 0 {
            self.base.set_frozen_ticks_to_run(0);
            true
        } else {
            false
        }
    }
    /// `stopSprinting`.
    pub fn stop_sprinting(&mut self) -> Option<SprintReport> {
        if self.remaining_sprint_ticks > 0 {
            Some(self.finish_tick_sprint())
        } else {
            None
        }
    }
    /// `requestGameToSprint`: true when an earlier sprint was interrupted.
    pub fn request_game_to_sprint(&mut self, time: i32) -> bool {
        let interrupted = self.remaining_sprint_ticks > 0;
        self.sprint_time_spend = 0;
        self.scheduled_current_sprint_ticks = i64::from(time);
        self.remaining_sprint_ticks = i64::from(time);
        self.previous_is_frozen = self.base.is_frozen();
        self.base.set_frozen(false);
        interrupted
    }
    fn finish_tick_sprint(&mut self) -> SprintReport {
        let completed_ticks = self.scheduled_current_sprint_ticks - self.remaining_sprint_ticks;
        let milliseconds_to_complete =
            (self.sprint_time_spend.max(1)) as f64 / NANOSECONDS_PER_MILLISECOND as f64;
        let ticks_per_second = (1000.0 * completed_ticks as f64 / milliseconds_to_complete) as i32;
        let per_tick = if completed_ticks == 0 {
            f64::from(self.base.milliseconds_per_tick())
        } else {
            milliseconds_to_complete / completed_ticks as f64
        };
        self.scheduled_current_sprint_ticks = 0;
        self.sprint_time_spend = 0;
        self.remaining_sprint_ticks = 0;
        self.base.set_frozen(self.previous_is_frozen);
        SprintReport {
            ticks_per_second,
            milliseconds_per_tick: crate::java_text::format_fixed2(per_tick),
        }
    }
    /// `checkShouldSprintThisTick`: whether to run this tick without waiting,
    /// plus the report when the sprint just finished.
    pub fn check_should_sprint_this_tick(&mut self, now_ns: i64) -> (bool, Option<SprintReport>) {
        if !self.base.run_game_elements {
            (false, None)
        } else if self.remaining_sprint_ticks > 0 {
            self.sprint_tick_start_time = now_ns;
            self.remaining_sprint_ticks -= 1;
            (true, None)
        } else {
            (false, Some(self.finish_tick_sprint()))
        }
    }
    /// `endTickWork`.
    pub fn end_tick_work(&mut self, now_ns: i64) {
        self.sprint_time_spend += now_ns - self.sprint_tick_start_time;
    }
    /// `setTickRate`.
    pub fn set_tick_rate(&mut self, rate: f32) {
        self.base.set_tick_rate(rate);
    }
}

/// Ticks one sprint frame may run (rendering carries on between frames).
const SPRINT_PER_FRAME: i64 = 40;
const SAMPLES: usize = 100;

/// A step command's broadcast value and the ticks still owed to a new client.
/// `ServerTickRateManager.updateStepTicks` sends the former immediately;
/// `ServerTickRateManager.updateJoiningPlayer` sends the latter on a join.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepState {
    pub revision: u32,
    pub requested: i32,
    pub remaining: i32,
}

/// The game's clock: `DeltaTracker.Timer` pacing driven by the
/// `ServerTickRateManager` the `/tick` command controls, over a virtual
/// millisecond clock built from the frame times it is fed. The integrated
/// simulation is one loop, so the tick rate paces it directly (the vanilla
/// client timer only ever slows down, because its server ticks separately).
#[derive(Clone, Debug)]
pub struct FixedStep {
    timer: Timer,
    manager: ServerTickRateManager,
    now: std::time::Duration,
    pause: bool,
    sprint_checked: bool,
    sprint_report: Option<SprintReport>,
    tick_times: [i64; SAMPLES],
    aggregated: i64,
    tick_count: i64,
    /// A server's client: its level's tick rate only slows the timer down
    /// (`Minecraft.getTickTargetMillis`), since the server runs the ticks.
    client: bool,
    /// How many times `/tick step` started or stopped a step
    /// (`ServerTickRateManager.updateStepTicks` broadcasts each one).
    step_changes: u32,
    step_requested: i32,
}

impl Default for FixedStep {
    fn default() -> Self {
        Self {
            timer: Timer::new(20.0, 0),
            manager: ServerTickRateManager::default(),
            now: std::time::Duration::ZERO,
            pause: false,
            sprint_checked: false,
            sprint_report: None,
            tick_times: [0; SAMPLES],
            aggregated: 0,
            tick_count: 0,
            client: false,
            step_changes: 0,
            step_requested: 0,
        }
    }
}

impl FixedStep {
    /// The clock of a client connected to a server: the `ClientLevel`'s
    /// `TickRateManager`, set by the server's ticking packets.
    pub fn client() -> Self {
        Self {
            client: true,
            ..Self::default()
        }
    }
    fn now_ms(&self) -> i64 {
        self.now.as_millis() as i64
    }
    fn now_ns(&self) -> i64 {
        self.now.as_nanos() as i64
    }
    /// The length of one tick at the current rate.
    pub fn tick_duration(&self) -> std::time::Duration {
        std::time::Duration::from_nanos(self.manager.base.nanoseconds_per_tick().max(1) as u64)
    }
    /// Feed one frame's elapsed time; returns the ticks to run (at most ten,
    /// or a sprint frame's worth).
    pub fn advance(&mut self, elapsed: std::time::Duration) -> u32 {
        self.now += elapsed;
        let now_ms = self.now_ms();
        if self.manager.is_sprinting() {
            // Keep the timer current so leaving the sprint does not replay it.
            self.timer.advance_game_time(now_ms, |d| d);
            let remaining = self.manager.remaining_sprint_ticks();
            // One extra pass lets the sprint finish and report.
            return (remaining + 1).min(SPRINT_PER_FRAME) as u32;
        }
        let runs = self.manager.base.runs_normally();
        let mspt = self.manager.base.milliseconds_per_tick();
        let client = self.client;
        let ticks = self
            .timer
            .advance_game_time(now_ms, |default| match (runs, client) {
                (true, true) => default.max(mspt),
                (true, false) => mspt,
                (false, _) => default,
            });
        ticks.clamp(0, 10) as u32
    }
    /// Restart pacing from now (after loading, so no backlog replays).
    pub fn reset(&mut self) {
        self.timer = Timer::new(20.0, self.now_ms());
        self.timer.update_pause_state(self.pause);
    }
    /// The partial tick for interpolation (`getGameTimeDeltaPartialTick(false)`).
    pub fn fraction(&self) -> f64 {
        f64::from(self.timer.game_time_delta_partial_tick(false))
    }
    /// The camera entity's partial tick (`Camera.getCameraEntityPartialTicks`
    /// for a player, who is never frozen): `getGameTimeDeltaPartialTick(true)`,
    /// so the view and the hand stay smooth in a frozen level.
    pub fn camera_fraction(&self) -> f64 {
        f64::from(self.timer.game_time_delta_partial_tick(true))
    }
    /// After a frame: whether the game is paused by a screen (the residual
    /// stops accumulating) and whether the level is frozen.
    pub fn end_frame(&mut self, pause: bool) {
        self.pause = pause;
        self.timer.update_pause_state(pause);
        self.timer
            .update_frozen_state(!self.manager.base.runs_normally());
    }
    pub fn paused(&self) -> bool {
        self.pause
    }
    /// `TickRateManager.tick` for one tick, then whether the world runs
    /// (`runsNormally`). Also the sprint check that opens a sprint tick.
    pub fn world_runs(&mut self) -> bool {
        self.manager.base.tick();
        self.sprint_checked = false;
        if self.manager.is_sprinting() {
            let (_, report) = self.manager.check_should_sprint_this_tick(self.now_ns());
            self.sprint_checked = report.is_none();
            if report.is_some() {
                self.sprint_report = report;
            }
        }
        self.manager.base.runs_normally()
    }
    /// The report of a sprint that just finished, once.
    pub fn take_sprint_report(&mut self) -> Option<SprintReport> {
        self.sprint_report.take()
    }
    pub fn rate(&self) -> f32 {
        self.manager.base.tickrate()
    }
    pub fn set_rate(&mut self, rate: f32) {
        self.manager.set_tick_rate(rate);
    }
    pub fn frozen(&self) -> bool {
        self.manager.base.is_frozen()
    }
    /// `runsNormally` as of the last `world_runs`.
    pub fn runs_normally(&self) -> bool {
        self.manager.base.runs_normally()
    }
    pub fn set_frozen(&mut self, frozen: bool) {
        self.manager.base.set_frozen(frozen);
    }
    /// `TickRateManager.setFrozenTicksToRun` (ClientboundTickingStepPacket).
    pub fn set_frozen_ticks_to_run(&mut self, ticks: i32) {
        self.manager.base.set_frozen_ticks_to_run(ticks);
    }
    pub fn sprinting(&self) -> bool {
        self.manager.is_sprinting()
    }
    /// `requestGameToSprint`: true when a sprint was already running.
    pub fn start_sprint(&mut self, ticks: u32) -> bool {
        self.manager
            .request_game_to_sprint(ticks.min(i32::MAX as u32) as i32)
    }
    /// `stopSprinting`: the report when a sprint was running.
    pub fn stop_sprint(&mut self) -> Option<SprintReport> {
        self.manager.stop_sprinting()
    }
    pub fn stepping(&self) -> bool {
        self.manager.base.is_stepping_forward()
    }
    /// `stepGameIfPaused`: only works while frozen.
    pub fn start_step(&mut self, ticks: u32) -> bool {
        let started = self
            .manager
            .step_game_if_paused(ticks.min(i32::MAX as u32) as i32);
        if started {
            self.step_changes += 1;
            self.step_requested = self.manager.base.frozen_ticks_to_run();
        }
        started
    }
    pub fn stop_step(&mut self) -> bool {
        let stopped = self.manager.stop_stepping();
        if stopped {
            self.step_changes += 1;
            self.step_requested = 0;
        }
        stopped
    }
    /// Preserve the command's count until the network pass, even if a world
    /// tick has already consumed it.
    /// New arrivals instead receive the current remaining ticks.
    pub fn step_state(&self) -> StepState {
        StepState {
            revision: self.step_changes,
            requested: self.step_requested,
            remaining: self.manager.base.frozen_ticks_to_run(),
        }
    }
    /// Record how long a tick took (`MinecraftServer.tickServer` bookkeeping
    /// and `endTickWork`).
    pub fn record(&mut self, took: std::time::Duration) {
        let took = took.as_nanos() as i64;
        let index = (self.tick_count % SAMPLES as i64) as usize;
        self.aggregated -= self.tick_times[index];
        self.aggregated += took;
        self.tick_times[index] = took;
        self.tick_count += 1;
        if self.sprint_checked {
            let end = self.now_ns() + took;
            self.manager.end_tick_work(end);
        }
    }
    /// `getAverageTickTimeNanos`.
    pub fn average_nanos(&self) -> u64 {
        (self.aggregated / SAMPLES.min(self.tick_count.max(1) as usize) as i64).max(0) as u64
    }
    /// The sorted `getTickTimesNanos` array (always 100 entries).
    pub fn sorted_samples(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.tick_times.iter().map(|&t| t.max(0) as u64).collect();
        v.sort_unstable();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_carries_fractions_and_caps_nothing_itself() {
        let mut t = Timer::new(20.0, 0);
        assert_eq!(t.advance_game_time(49, |d| d), 0);
        assert_eq!(t.advance_game_time(50, |d| d), 1);
        assert_eq!(t.advance_game_time(1050, |d| d), 20);
    }
    #[test]
    fn pause_restores_the_residual() {
        let mut t = Timer::new(20.0, 0);
        t.advance_game_time(25, |d| d);
        t.update_pause_state(true);
        t.advance_game_time(1000, |d| d);
        assert_eq!(t.game_time_delta_partial_tick(false), 0.5);
        t.update_pause_state(false);
        assert_eq!(t.game_time_delta_partial_tick(false), 0.5);
    }
    #[test]
    fn frozen_step_runs_exactly_the_requested_ticks() {
        let mut m = ServerTickRateManager::default();
        m.base.set_frozen(true);
        m.base.tick();
        assert!(!m.base.runs_normally());
        assert!(m.step_game_if_paused(2));
        for _ in 0..2 {
            m.base.tick();
            assert!(m.base.runs_normally());
        }
        m.base.tick();
        assert!(!m.base.runs_normally());
    }
    #[test]
    fn the_camera_keeps_interpolating_in_a_frozen_level() {
        let mut clock = FixedStep::default();
        clock.set_frozen(true);
        clock.world_runs();
        clock.advance(std::time::Duration::from_millis(25));
        clock.end_frame(false);
        assert_eq!(clock.fraction(), 1.0, "frozen entities stay put");
        assert_eq!(clock.camera_fraction(), 0.5, "the player does not");
    }
    #[test]
    fn only_a_started_or_stopped_step_is_broadcast() {
        let mut clock = FixedStep::default();
        assert!(!clock.start_step(5), "not frozen");
        assert_eq!(clock.step_state(), StepState::default());
        clock.set_frozen(true);
        assert!(clock.start_step(5));
        assert_eq!(
            clock.step_state(),
            StepState {
                revision: 1,
                requested: 5,
                remaining: 5
            }
        );
        clock.world_runs();
        assert_eq!(
            clock.step_state(),
            StepState {
                revision: 1,
                requested: 5,
                remaining: 4
            },
            "consuming a tick preserves the command broadcast"
        );
        assert!(clock.stop_step());
        assert!(!clock.stop_step());
        assert_eq!(
            clock.step_state(),
            StepState {
                revision: 2,
                requested: 0,
                remaining: 0
            }
        );
    }
    #[test]
    fn a_consumed_single_step_still_advances_the_client_once() {
        let mut server = FixedStep::default();
        server.set_frozen(true);
        assert!(server.start_step(1));
        assert!(server.world_runs());
        assert!(!server.world_runs());
        let state = server.step_state();
        assert_eq!(state.remaining, 0, "a late join has no pending ticks");
        let mut client = FixedStep::client();
        client.set_frozen(true);
        client.set_frozen_ticks_to_run(state.requested);
        assert!(
            client.world_runs(),
            "the connected client runs the command's tick"
        );
        assert!(!client.world_runs(), "then remains frozen");
    }
    #[test]
    fn a_servers_tick_rate_only_slows_its_client_down() {
        let quarter = std::time::Duration::from_millis(250);
        let mut client = FixedStep::client();
        client.set_rate(40.0);
        assert_eq!(client.advance(quarter), 5, "still 20 a second");
        let mut slow = FixedStep::client();
        slow.set_rate(10.0);
        assert_eq!(slow.advance(quarter * 2), 5, "10 a second");
        let mut local = FixedStep::default();
        local.set_rate(40.0);
        assert_eq!(
            local.advance(quarter),
            10,
            "the integrated loop runs at the rate"
        );
    }
}
