//! The game-test runner: `GameTestInfo`, `GameTestRunner`, `GameTestTicker`,
//! `MultipleTestTracker`, `ReportGameListener`, `TestCommand.trackAndStartRunner`
//! and the two test instance types (`BlockBasedTestInstance` and
//! `FunctionGameTestInstance`), over the test instance blocks in a level.
//!
//! Retries (`RetryOptions`, `/test run .. <numberOfTimes> <untilFailed>`) and
//! flaky tests (`max_attempts`, `required_successes`) go through
//! `ReportGameListener`'s counters and `GameTestRunner.rerunTest`.
//!
//! Not ported: test sequences (`GameTestSequence`: the built-in tests, only
//! `always_pass`, do not use them), test environments other than the empty
//! `all_of`, and removing the entities of a finished test.
use super::{
    Dim, Game,
    structure_blocks::template_size,
    test_entities::{aabb_of, inflate},
    test_environments::Activation,
    test_instances::TestDef,
};
use horizon_client::{
    containers::BlockEntity,
    lan::data::component_nbt,
    test_block::TestBlockMode,
    test_instance_block::{ErrorMarker, Status, TestInstanceBlock},
    text::{self, Text},
};
use horizon_world::{
    level::flags,
    worldgen::structure::{BoundingBox, Mirror, Rotation, StructureTemplate},
};
use std::time::Instant;

type Pos = (i32, i32, i32);

/// `GameTestAssertException`, `GameTestAssertPosException`,
/// `GameTestTimeoutException` and `UnknownGameTestException`.
#[derive(Clone, Debug)]
pub(super) enum TestError {
    Assert {
        message: Text,
        tick: i32,
    },
    AssertPos {
        message: Text,
        absolute: Pos,
        relative: Pos,
        tick: i32,
    },
    Timeout(Text),
    Unknown(String),
}

fn number(v: i32) -> Text {
    Text::literal(v.to_string())
}

impl TestError {
    /// `getDescription`.
    pub(super) fn description(&self) -> Text {
        match self {
            Self::Assert { message, tick } => {
                Text::translate("test.error.tick", &[message.clone(), number(*tick)])
            }
            Self::AssertPos {
                message,
                absolute,
                relative,
                tick,
            } => Text::translate(
                "test.error.position",
                &[
                    message.clone(),
                    number(absolute.0),
                    number(absolute.1),
                    number(absolute.2),
                    number(relative.0),
                    number(relative.1),
                    number(relative.2),
                    number(*tick),
                ],
            ),
            Self::Timeout(message) => message.clone(),
            Self::Unknown(reason) => {
                Text::translate("test.error.unknown", &[Text::literal(reason.clone())])
            }
        }
    }

    /// `getMessage`: the description's characters (a timeout's own message,
    /// an unknown error's reason).
    fn message(&self) -> String {
        match self {
            Self::Unknown(reason) => reason.clone(),
            other => other.description().plain(),
        }
    }
}

/// `RetryOptions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Retry {
    number_of_tries: i32,
    halt_on_failure: bool,
}

impl Retry {
    pub(super) fn new(number_of_tries: i32, halt_on_failure: bool) -> Self {
        Self {
            number_of_tries,
            halt_on_failure,
        }
    }

    /// `noRetries`.
    pub(super) fn none() -> Self {
        Self::new(1, true)
    }

    /// `unlimitedTries`.
    fn unlimited(&self) -> bool {
        self.number_of_tries < 1
    }

    /// `hasTriesLeft`.
    fn has_tries_left(&self, attempts: i32, successes: i32) -> bool {
        let has_failures = attempts != successes;
        let has_more_attempts = self.unlimited() || attempts < self.number_of_tries;
        has_more_attempts && (!has_failures || !self.halt_on_failure)
    }

    /// `hasRetries`.
    fn has_retries(&self) -> bool {
        self.number_of_tries != 1
    }
}

/// What `runAtTickTime` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// `BlockBasedTestInstance.run`'s `onEachTick` body.
    BlockBasedTick,
}

/// `GameTestInfo`.
pub(super) struct Info {
    id: u32,
    /// `id()`: the test's registry id.
    key: String,
    def: TestDef,
    /// The test instance block (`getTestBlockPos`), once there is one.
    pos: Pos,
    has_pos: bool,
    dim: Dim,
    extra_rotation: Rotation,
    tick_count: i32,
    started: bool,
    placed: bool,
    done: bool,
    error: Option<TestError>,
    run_at: Vec<(i64, Action)>,
    started_at: Option<Instant>,
    run_ms: u128,
    retry: Retry,
    /// The test whose `ReportGameListener` counts this run: its own id, or
    /// the original's for a rerun (`copyReset`).
    origin: u32,
    rerun: bool,
    /// `succeed` removes the entities around the structure; the game does it.
    discard_entities: bool,
}

impl Info {
    pub(super) fn new(
        id: u32,
        key: String,
        def: TestDef,
        pos: Option<Pos>,
        dim: Dim,
        extra_rotation: Rotation,
    ) -> Self {
        Self {
            id,
            key,
            def,
            pos: pos.unwrap_or_default(),
            has_pos: pos.is_some(),
            dim,
            extra_rotation,
            tick_count: 0,
            started: false,
            placed: false,
            done: false,
            error: None,
            run_at: Vec::new(),
            started_at: None,
            run_ms: 0,
            retry: Retry::none(),
            origin: id,
            rerun: false,
            discard_entities: false,
        }
    }

    pub(super) fn with_retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }

    /// `copyReset`: the same test, level and block, as a new run.
    fn copy_reset(&self, id: u32) -> Self {
        let pos = self.has_pos.then_some(self.pos);
        let mut copy = Self::new(
            id,
            self.key.clone(),
            self.def.clone(),
            pos,
            self.dim,
            self.extra_rotation,
        )
        .with_retry(self.retry);
        copy.origin = self.origin;
        copy.rerun = true;
        copy
    }

    /// `getRotation`.
    fn rotation(&self) -> Rotation {
        self.def.rotation.get_rotated(self.extra_rotation)
    }

    /// `assertionException(description)`.
    fn assertion(&self, message: Text) -> TestError {
        TestError::Assert {
            message,
            tick: self.tick_count,
        }
    }

    /// `fail(Component)`.
    fn fail(&mut self, message: Text) {
        self.error = Some(self.assertion(message));
    }

    /// `finish`.
    fn finish(&mut self) {
        if !self.done {
            self.done = true;
            if let Some(at) = self.started_at.take() {
                self.run_ms = at.elapsed().as_millis();
            }
        }
    }

    /// `succeed`: finishes unless a failure is set.
    fn succeed(&mut self) {
        if self.error.is_none() {
            self.finish();
            self.discard_entities = true;
        }
    }

    /// `startExecution(tickDelay)`.
    fn start_execution(&mut self, tick_delay: i32) {
        self.tick_count = -(self.def.setup_ticks + tick_delay + 1);
    }

    /// `onEachTick`: the action on every tick from now to the timeout.
    fn on_each_tick(&mut self, action: Action) {
        for i in i64::from(self.tick_count)..i64::from(self.def.max_ticks) {
            self.run_at.push((i, action));
        }
    }
}

/// One `GameTestBatch`.
pub(super) struct Batch {
    pub(super) index: i32,
    pub(super) environment: String,
    pub(super) infos: Vec<Info>,
}

/// What `GameTestRunner.Builder` is given.
pub(super) struct RunnerOptions {
    halt_on_error: bool,
    batch_size: usize,
    clear_between_batches: bool,
    grid_for_existing: bool,
    grid: Option<Grid>,
}

impl RunnerOptions {
    pub(super) fn new(halt_on_error: bool, grid: Option<Grid>) -> Self {
        Self {
            halt_on_error,
            batch_size: 50,
            clear_between_batches: false,
            grid_for_existing: false,
            grid,
        }
    }

    /// `/test verify`: `batcher(fromGameTestInfo(100))`, the grid for
    /// existing tests too, `haltOnError`, `clearBetweenBatches`.
    pub(super) fn verify(grid: Grid) -> Self {
        Self {
            halt_on_error: true,
            batch_size: 100,
            clear_between_batches: true,
            grid_for_existing: true,
            grid: Some(grid),
        }
    }
}

/// What `MultipleTestTracker` knows of a test.
struct Tracked {
    id: u32,
    required: bool,
    done: bool,
    failed: bool,
}

/// `GameTestBatchFactory.fromGameTestInfo(size)`: one batch per environment
/// (and dimension), `size` to a batch.
fn batch_infos(infos: Vec<Info>, size: usize) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::new();
    let mut counts: Vec<(String, i32)> = Vec::new();
    for info in infos {
        let env = info.def.environment.clone();
        let open = batches
            .iter_mut()
            .find(|b| b.environment == env && b.infos.len() < size);
        if let Some(batch) = open {
            batch.infos.push(info);
            continue;
        }
        let index = match counts.iter_mut().find(|(e, _)| *e == env) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                counts.push((env.clone(), 0));
                0
            }
        };
        batches.push(Batch {
            index,
            environment: env,
            infos: vec![info],
        });
    }
    batches
}

/// `/test verify`'s batches: each test in each rotation 100 times, with
/// `RetryOptions(1, true)`, at the test's block if it has one; the batch
/// index is the rotation's ordinal.
pub(super) fn verify_batches(infos: &[Info], mut next_id: impl FnMut() -> u32) -> Vec<Batch> {
    let mut batches = Vec::new();
    for info in infos {
        for (ordinal, rotation) in Rotation::ALL.into_iter().enumerate() {
            let copies = (0..100)
                .map(|_| {
                    let pos = info.has_pos.then_some(info.pos);
                    Info::new(
                        next_id(),
                        info.key.clone(),
                        info.def.clone(),
                        pos,
                        info.dim,
                        rotation,
                    )
                    .with_retry(Retry::new(1, true))
                })
                .collect();
            batches.push(Batch {
                index: ordinal as i32,
                environment: info.def.environment.clone(),
                infos: copies,
            });
        }
    }
    batches
}

/// A listener's counters, (origin, attempts, successes): its attempts and successes.
fn report_of(reports: &mut Vec<(u32, i32, i32)>, origin: u32) -> (&mut i32, &mut i32) {
    let at = reports
        .iter()
        .position(|r| r.0 == origin)
        .unwrap_or_else(|| {
            reports.push((origin, 0, 0));
            reports.len() - 1
        });
    let r = &mut reports[at];
    (&mut r.1, &mut r.2)
}

/// `GameTestRunner`.
struct Runner {
    batches: Vec<Batch>,
    current: usize,
    /// `currentBatchTracker`: the tests of the batch still to finish.
    batch_pending: usize,
    tracked: Vec<Tracked>,
    halt_on_error: bool,
    /// `GameTestBatchFactory.fromGameTestInfo(maxTestsPerBatch)`, for reruns.
    batch_size: usize,
    /// `clearBetweenBatches`.
    clear_between_batches: bool,
    /// The grid is also `existingStructureSpawner` (`/test verify`): a test
    /// with a block is moved onto it.
    grid_for_existing: bool,
    /// The tests of the batch before: their levels and blocks, for
    /// `clearBetweenBatches`.
    last_batch: Vec<(Dim, Pos)>,
    /// `newStructureSpawner`: where the tests without a block go.
    grid: Option<Grid>,
    /// `ReportGameListener`'s (attempts, successes), by original test.
    reports: Vec<(u32, i32, i32)>,
    /// `scheduledForRerun`.
    scheduled: Vec<Info>,
    /// `stopped`: no batch is running.
    stopped: bool,
}

/// `StructureGridSpawner.DimensionGridState`.
struct DimGrid {
    first: Pos,
    next: Pos,
    /// The z extent of `rowBounds` (an `AABB(BlockPos)` is one block deep).
    row_z: (i32, i32),
    count: i32,
    /// `testsInLastBatch`: their blocks.
    last_batch: Vec<Pos>,
}

impl DimGrid {
    fn new(first: Pos) -> Self {
        Self {
            first,
            next: first,
            row_z: (first.2, first.2 + 1),
            count: 0,
            last_batch: Vec::new(),
        }
    }
}

/// `StructureGridSpawner`: a grid per dimension, made when the first test of
/// that dimension is placed (`firstTestNorthWestCorner.apply(dimension)`).
pub(super) struct Grid {
    /// The column `createTestPositionAround` starts from: the source's.
    around: (i32, i32),
    per_row: i32,
    /// `clearOnBatch`.
    clear_on_batch: bool,
    states: Vec<(Dim, DimGrid)>,
}

impl Grid {
    pub(super) fn new(around: (i32, i32), per_row: i32) -> Self {
        Self {
            around,
            per_row,
            clear_on_batch: false,
            states: Vec::new(),
        }
    }

    /// `clearOnBatch = true`.
    pub(super) fn clearing(mut self) -> Self {
        self.clear_on_batch = true;
        self
    }
}

/// What a runner says to the server, sent on the next tick (a test's
/// messages come from the server's console, not a player's command).
enum Out {
    /// `sendSuccess(.., broadcast)`, `sendFailure` or `sendSystemMessage` of
    /// the server's command source.
    Console { text: Text, broadcast: bool },
    /// `ReportGameListener.say`: every player gets it.
    Players(Text),
}

/// `GameTestTicker.SINGLETON` and the runner it holds, `FailedTestTracker`.
#[derive(Default)]
pub(super) struct GameTests {
    ticker: Vec<Info>,
    runner: Option<Runner>,
    ticking: bool,
    halting: bool,
    next_id: u32,
    /// `FailedTestTracker.LAST_FAILED_TESTS`.
    pub(super) failed: Vec<String>,
    outbox: Vec<Out>,
    /// `GameTestRunner.currentEnvironment`.
    environment: Option<Activation>,
}

impl GameTests {
    /// A test of the runner (placed or still to place) is in `dim`.
    pub(super) fn has_tests_in(&self, dim: Dim) -> bool {
        self.ticker.iter().any(|i| i.dim == dim)
            || self.runner.as_ref().is_some_and(|r| {
                r.batches
                    .iter()
                    .flat_map(|b| &b.infos)
                    .chain(&r.scheduled)
                    .any(|i| i.dim == dim)
            })
    }

    pub(super) fn next_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }
}

impl Game<'_> {
    // ---- GameTestRunner / GameTestTicker ----

    /// `GameTestTicker.clear`: a pass in progress halts at its end.
    pub(super) fn clear_game_tests(&mut self) {
        if self.game_tests.ticking {
            self.game_tests.halting = true;
        } else {
            self.game_tests.ticker.clear();
            if self.game_tests.runner.take().is_some() {
                // GameTestRunner.stop: endCurrentEnvironment.
                self.end_test_environment();
            }
        }
    }

    /// `TestCommand.trackAndStartRunner` for tests already built (batched by
    /// environment, 50 to a batch): the tracker and its listeners, then
    /// `GameTestRunner.start`.
    pub(super) fn track_and_start_runner(
        &mut self,
        infos: Vec<Info>,
        halt_on_error: bool,
        grid: Option<Grid>,
    ) {
        let batches = batch_infos(infos, 50);
        self.track_and_start_batches(batches, RunnerOptions::new(halt_on_error, grid));
    }

    /// `TestCommand.trackAndStartRunner` for batches already made.
    pub(super) fn track_and_start_batches(&mut self, batches: Vec<Batch>, options: RunnerOptions) {
        let tracked = batches
            .iter()
            .flat_map(|b| &b.infos)
            .map(|i| Tracked {
                id: i.id,
                required: i.def.required,
                done: false,
                failed: false,
            })
            .collect();
        self.game_tests.runner = Some(Runner {
            batches,
            current: 0,
            batch_pending: 0,
            tracked,
            halt_on_error: options.halt_on_error,
            batch_size: options.batch_size,
            clear_between_batches: options.clear_between_batches,
            grid_for_existing: options.grid_for_existing,
            last_batch: Vec::new(),
            grid: options.grid,
            reports: Vec::new(),
            scheduled: Vec::new(),
            stopped: false,
        });
        self.run_batch(0);
    }

    /// `GameTestRunner.runBatch`.
    fn run_batch(&mut self, index: usize) {
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        if index >= runner.batches.len() {
            self.end_test_environment();
            self.run_scheduled_reruns();
            return;
        }
        runner.current = index;
        // clearBetweenBatches: the batch before is cleared and its blocks
        // destroyed.
        let cleared = if index > 0 && runner.clear_between_batches {
            std::mem::take(&mut runner.last_batch)
        } else {
            Vec::new()
        };
        // StructureGridSpawner.onBatchStart (clearOnBatch), every dimension.
        let mut grid_cleared: Vec<(Dim, Pos)> = Vec::new();
        if let Some(grid) = runner.grid.as_mut().filter(|g| g.clear_on_batch) {
            for (dim, g) in &mut grid.states {
                grid_cleared.extend(
                    std::mem::take(&mut g.last_batch)
                        .into_iter()
                        .map(|p| (*dim, p)),
                );
                g.next = g.first;
                g.row_z = (g.first.2, g.first.2 + 1);
                g.count = 0;
            }
        }
        for (dim, pos) in cleared {
            self.in_test_level(dim, |g| {
                if let Some(area) = g.test_bounding_box(pos) {
                    g.clear_test_space(&area);
                }
                g.set_test_block_state(pos, "minecraft:air", flags::UPDATE_ALL);
            });
        }
        for (dim, pos) in grid_cleared {
            self.in_test_level(dim, |g| {
                if let Some(area) = g.test_bounding_box(pos) {
                    g.clear_test_space(&area);
                }
            });
        }
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        let Some(batch) = runner.batches.get_mut(index) else {
            return;
        };
        let mut infos = std::mem::take(&mut batch.infos);
        let (environment, batch_number) = (batch.environment.clone(), batch.index);
        // createStructuresForBatch: `StructureSpawner.IN_PLACE`.
        let mut spawned = Vec::new();
        for mut info in infos.drain(..) {
            if self.spawn_test(&mut info) {
                spawned.push(info);
            }
        }
        log::info!(
            "Running test environment '{environment}' batch {batch_number} ({} tests)...",
            spawned.len()
        );
        // endCurrentEnvironment, then TestEnvironmentDefinition.activate.
        self.end_test_environment();
        match self.test_environment(&environment) {
            Some(definition) => {
                let activation = self.activate_test_environment(definition);
                self.game_tests.environment = Some(activation);
            }
            None => log::error!("Missing test environment {environment}"),
        }
        self.game_tests.outbox.push(Out::Console {
            text: Text::translate(
                "commands.test.batch.starting",
                &[Text::literal(environment), number(batch_number)],
            ),
            broadcast: true,
        });
        if let Some(runner) = self.game_tests.runner.as_mut() {
            runner.batch_pending = spawned.len();
            runner.last_batch = spawned.iter().map(|i| (i.dim, i.pos)).collect();
        }
        self.game_tests.ticker.extend(spawned);
    }

    /// `GameTestRunner.endCurrentEnvironment`.
    fn end_test_environment(&mut self) {
        if let Some(activation) = self.game_tests.environment.take() {
            self.tear_down_test_environment(activation);
        }
    }

    /// `GameTestRunner.runScheduledRerunTests`: the reruns are batched
    /// again and run, or the runner stops.
    fn run_scheduled_reruns(&mut self) {
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        if runner.scheduled.is_empty() {
            runner.batches = Vec::new();
            runner.stopped = true;
            return;
        }
        let scheduled = std::mem::take(&mut runner.scheduled);
        log::info!(
            "Starting re-run of tests: {}",
            scheduled
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        runner.batches = batch_infos(scheduled, runner.batch_size);
        runner.stopped = false;
        self.run_batch(0);
    }

    /// `GameTestRunner.rerunTest`.
    fn rerun_test(&mut self, info: &Info) {
        let id = self.game_tests.next_id();
        let copy = info.copy_reset(id);
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        runner.scheduled.push(copy);
        if runner.stopped {
            self.run_scheduled_reruns();
        }
    }

    /// `GameTestServer`'s per-tick call: `MinecraftServer.tickServer`'s
    /// `GameTestTicker.SINGLETON.tick()`, for the tests of the level in place.
    pub(super) fn tick_game_tests(&mut self) {
        self.flush_game_test_output();
        if self.game_tests.runner.is_none() {
            return;
        }
        let dim = self.dim;
        self.game_tests.ticking = true;
        let mut infos = std::mem::take(&mut self.game_tests.ticker);
        for info in infos.iter_mut().filter(|i| i.dim == dim) {
            self.tick_test(info);
        }
        infos.retain(|i| !i.done);
        infos.append(&mut self.game_tests.ticker);
        self.game_tests.ticker = infos;
        self.game_tests.ticking = false;
        if std::mem::take(&mut self.game_tests.halting) {
            self.clear_game_tests();
        }
    }

    // ---- GameTestInfo ----

    /// `GameTestRunner.spawn`: a test with a block is prepared where it is
    /// (`StructureSpawner.IN_PLACE`), one without goes to the next place in
    /// the grid (`StructureGridSpawner.spawnStructure`), or is dropped
    /// (`NOT_SET`).
    fn spawn_test(&mut self, info: &mut Info) -> bool {
        let dim = info.dim;
        // The test's own level is in place while it is prepared.
        match self.in_test_level(dim, |g| g.spawn_test_here(info)) {
            Some(spawned) => spawned,
            None => {
                log::error!("Could not resolve level for dimension: {}", dim.id());
                false
            }
        }
    }

    /// `spawn_test` with the test's level in place.
    fn spawn_test_here(&mut self, info: &mut Info) -> bool {
        let in_place = !self
            .game_tests
            .runner
            .as_ref()
            .is_some_and(|r| r.grid_for_existing);
        if info.has_pos && in_place {
            if !self.prepare_test_structure(info) {
                return false;
            }
            info.start_execution(1);
            self.test_structure_loaded(info);
            return true;
        }
        let Some(mut grid) = self
            .game_tests
            .runner
            .as_mut()
            .and_then(|runner| runner.grid.take())
        else {
            return false;
        };
        // `gridFor`: the dimension's grid starts where the source stands.
        let at = match grid.states.iter().position(|(d, _)| *d == info.dim) {
            Some(at) => at,
            None => {
                let first = self.test_position_in_level(grid.around);
                grid.states.push((info.dim, DimGrid::new(first)));
                grid.states.len() - 1
            }
        };
        let per_row = grid.per_row;
        let state = &mut grid.states[at].1;
        info.pos = state.next;
        info.has_pos = true;
        let prepared = self.prepare_test_structure(info);
        if prepared {
            state.last_batch.push(info.pos);
            info.start_execution(1);
            self.test_structure_loaded(info);
            if let Some(b) = self.test_bounding_box(info.pos) {
                // `getTestBounds()`: the structure's box and its padding.
                let width = b.max[0] - b.min[0] + 1;
                state.row_z = (state.row_z.0.min(b.min[2]), state.row_z.1.max(b.max[2] + 1));
                state.next.0 += width + 5;
                state.count += 1;
                if state.count >= per_row {
                    state.count = 0;
                    state.next.2 += state.row_z.1 - state.row_z.0 + 6;
                    state.next.0 = state.first.0;
                    state.row_z = (state.next.2, state.next.2 + 1);
                }
            }
        }
        if let Some(runner) = self.game_tests.runner.as_mut() {
            runner.grid = Some(grid);
        }
        prepared
    }

    /// Run `f` with `dim`'s level in place (`enter_level`, as a command
    /// whose source is there), then put the level back. None when the level
    /// cannot be entered.
    pub(super) fn in_test_level<R>(
        &mut self,
        dim: Dim,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if dim == self.dim {
            return Some(f(self));
        }
        let back = self.enter_command_level(dim)?;
        let result = f(self);
        if let Err(e) = self.enter_level(None, back) {
            log::error!("could not return to {}: {e}", back.id());
        }
        Some(result)
    }

    /// `level.getChunk(pos)` for the chunks of `area`: forced (the test needs
    /// them kept) and waited for, as vanilla loads them synchronously.
    pub(super) fn load_test_chunks(&mut self, area: &BoundingBox) {
        let missing = |g: &Self| -> bool {
            (area.min[0] >> 4..=area.max[0] >> 4).any(|x| {
                (area.min[2] >> 4..=area.max[2] >> 4).any(|z| !g.world.chunks.contains_key(&(x, z)))
            })
        };
        if !missing(self) {
            return;
        }
        self.force_load_test_chunks(area);
        let deadline = Instant::now() + std::time::Duration::from_secs(120);
        while missing(self) && Instant::now() < deadline {
            let _ = self.streaming.update(&mut self.world);
            self.streaming.wait(std::time::Duration::from_millis(5));
        }
    }

    /// `ReportGameListener.testStructureLoaded`.
    fn test_structure_loaded(&mut self, info: &Info) {
        if let Some(runner) = self.game_tests.runner.as_mut() {
            *report_of(&mut runner.reports, info.origin).0 += 1;
        }
    }

    /// `GameTestInfo.prepareTestStructure`: the test instance block with the
    /// test's data, then `placeStructure`. False when the block is gone.
    fn prepare_test_structure(&mut self, info: &mut Info) -> bool {
        let pos = info.pos;
        self.load_test_chunks(&BoundingBox::new(
            [pos.0, pos.1, pos.2],
            [pos.0, pos.1, pos.2],
        ));
        self.set_test_block_state(pos, "minecraft:test_instance_block", flags::UPDATE_ALL);
        if self.test_instance_at(pos).is_none() {
            self.world.block_entities.insert(
                pos,
                BlockEntity::TestInstanceBlock(Box::<TestInstanceBlock>::default()),
            );
        }
        let size = self
            .template_entry(&info.def.structure)
            .map_or([1, 1, 1], |entry| template_size(&entry.tag));
        if let Some(block) = self.test_instance_mut(pos) {
            block.data.test = Some(info.key.clone());
            block.data.size = size;
            block.data.rotation = info.extra_rotation;
            block.data.ignore_entities = false;
            block.data.status = Status::Cleared;
            block.data.error_message = None;
        }
        self.mark_block_entity_updated(pos);
        // placeStructure
        if let Some(area) = self.test_bounding_box(pos) {
            self.load_test_chunks(&area);
        }
        if !self.place_test_structure(pos) {
            let name = self.test_name(pos).plain();
            info.fail(Text::translate(
                "test.error.structure.failure",
                &[Text::literal(name)],
            ));
        }
        info.placed = true;
        self.encase_test_structure(pos);
        if let Some(bounds) = self.test_bounding_box(pos) {
            self.clear_test_ticks(&bounds);
        }
        true
    }

    /// `getTestBoundingBox`: the structure's box and its padding.
    pub(super) fn test_bounding_box(&self, pos: Pos) -> Option<BoundingBox> {
        let padding = self.test_holder_padding(pos);
        let inner = self.test_structure_box(pos)?;
        Some(BoundingBox::new(
            inner.min.map(|v| v - padding),
            inner.max.map(|v| v + padding),
        ))
    }

    /// `encaseStructure`: barriers around the box, but for the block itself.
    fn encase_test_structure(&mut self, pos: Pos) {
        let block = self
            .pack
            .assets
            .registry
            .default_state("minecraft:test_instance_block");
        for at in self.structure_boundary(pos) {
            if Some(self.world.block(at.0, at.1, at.2)) != block {
                self.set_test_block_state(at, "minecraft:barrier", flags::UPDATE_ALL);
            }
        }
    }

    /// `GameTestInfo.tick`.
    fn tick_test(&mut self, info: &mut Info) {
        if info.done {
            return;
        }
        if !info.placed {
            info.fail(Text::translate("test.error.ticking_without_structure", &[]));
        }
        let has_block = self.test_instance_at(info.pos).is_some();
        if !has_block {
            info.fail(Text::translate("test.error.missing_block_entity", &[]));
        }
        if info.error.is_some() {
            info.finish();
        } else if !self.test_chunks_loaded(info.pos) {
            return;
        }
        self.tick_internal(info);
        if std::mem::take(&mut info.discard_entities)
            && let Some(b) = self.test_structure_box(info.pos)
        {
            // `GameTestInfo.succeed`: `getStructureBounds().inflate(1.0)`.
            self.discard_entities_in(inflate(aabb_of(&b), 1.0));
        }
        if info.done {
            self.test_finished(info);
        }
    }

    /// The chunks of the structure are loaded (`areEntitiesActuallyLoadedAndTicking`).
    fn test_chunks_loaded(&self, pos: Pos) -> bool {
        let Some(b) = self.test_structure_box(pos) else {
            return true;
        };
        (b.min[0] >> 4..=b.max[0] >> 4).all(|cx| {
            (b.min[2] >> 4..=b.max[2] >> 4).all(|cz| self.world.chunks.contains_key(&(cx, cz)))
        })
    }

    /// `GameTestInfo.tickInternal`.
    fn tick_internal(&mut self, info: &mut Info) {
        info.tick_count += 1;
        if info.tick_count < 0 {
            return;
        }
        if !info.started {
            self.start_test(info);
        }
        let now = i64::from(info.tick_count);
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut info.run_at)
            .into_iter()
            .partition(|(time, _)| *time <= now);
        info.run_at = later;
        for (_, action) in due {
            if let Err(error) = self.run_action(info, action) {
                info.error = Some(error);
            }
        }
        if info.tick_count > info.def.max_ticks {
            // No sequences: the test had no result in time.
            info.error = Some(TestError::Timeout(Text::translate(
                "test.error.timeout.no_result",
                &[number(info.def.max_ticks)],
            )));
        }
    }

    /// `GameTestInfo.startTest`.
    fn start_test(&mut self, info: &mut Info) {
        info.started = true;
        info.started_at = Some(Instant::now());
        self.set_test_status(info.pos, Status::Running, None);
        if let Err(error) = self.run_instance(info) {
            info.error = Some(error);
        }
    }

    /// `GameTestInstance.run(helper)`.
    fn run_instance(&mut self, info: &mut Info) -> Result<(), TestError> {
        if info.def.block_based {
            return self.run_block_based(info);
        }
        // FunctionGameTestInstance: the function from the test function
        // registry (`BuiltinTestFunctions`).
        let function = info.def.function.as_deref().unwrap_or_default();
        let function = if function.contains(':') {
            function.to_owned()
        } else {
            format!("minecraft:{function}")
        };
        match function.as_str() {
            // ALWAYS_PASS: GameTestHelper::succeed.
            "minecraft:always_pass" => {
                info.succeed();
                Ok(())
            }
            other => Err(TestError::Unknown(format!(
                "Trying to access missing test function: {other}"
            ))),
        }
    }

    fn run_action(&mut self, info: &mut Info, action: Action) -> Result<(), TestError> {
        match action {
            Action::BlockBasedTick => self.block_based_tick(info),
        }
    }

    // ---- GameTestHelper ----

    /// `absolutePos`.
    fn test_absolute_pos(&self, info: &Info, relative: Pos) -> Pos {
        let origin = self.test_origin(info);
        let moved = [
            origin[0] + relative.0,
            origin[1] + relative.1,
            origin[2] + relative.2,
        ];
        let [x, y, z] = StructureTemplate::transform(moved, Mirror::None, info.rotation(), origin);
        (x, y, z)
    }

    /// `getTestOrigin`: the block entity's start corner.
    fn test_origin(&self, info: &Info) -> [i32; 3] {
        let padding = info.def.padding;
        let rotation = self.test_rotation(info.pos);
        self.test_instance_at(info.pos).map_or([0; 3], |b| {
            b.start_corner([info.pos.0, info.pos.1, info.pos.2], padding, rotation)
        })
    }

    /// `assertionException(pos, description)`.
    fn pos_assertion(&self, info: &Info, relative: Pos, message: Text) -> TestError {
        TestError::AssertPos {
            message,
            absolute: self.test_absolute_pos(info, relative),
            relative,
            tick: info.tick_count,
        }
    }

    /// `getBlockEntity(pos, TestBlockEntity.class)`: the absolute position of
    /// the test block.
    fn test_block_entity(&self, info: &Info, relative: Pos) -> Result<Pos, TestError> {
        let absolute = self.test_absolute_pos(info, relative);
        match self.world.block_entities.get(&absolute) {
            None => Err(self.pos_assertion(
                info,
                relative,
                Text::translate("test.error.missing_block_entity", &[]),
            )),
            Some(BlockEntity::TestBlock(_)) => Ok(absolute),
            Some(other) => Err(self.pos_assertion(
                info,
                relative,
                Text::translate(
                    "test.error.wrong_block_entity",
                    &[Text::literal(other.id().to_owned())],
                ),
            )),
        }
    }

    /// `forEveryBlockInStructure`: the template-relative positions, x then
    /// y then z.
    fn test_relative_positions(&self, info: &Info) -> Vec<Pos> {
        let Some(b) = self.test_structure_box(info.pos) else {
            return Vec::new();
        };
        let size = [
            b.max[0] - b.min[0] + 1,
            b.max[1] - b.min[1] + 1,
            b.max[2] - b.min[2] + 1,
        ];
        let size = match info.rotation() {
            Rotation::Clockwise90 | Rotation::Counterclockwise90 => [size[2], size[1], size[0]],
            _ => size,
        };
        let mut out = Vec::new();
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    out.push((x, y, z));
                }
            }
        }
        out
    }

    /// `BlockBasedTestInstance.findTestBlocks`.
    fn find_test_blocks(&self, info: &Info, mode: TestBlockMode) -> Vec<Pos> {
        let registry = self.pack.assets.registry;
        self.test_relative_positions(info)
            .into_iter()
            .filter(|&relative| {
                let at = self.test_absolute_pos(info, relative);
                registry
                    .state(self.world.block(at.0, at.1, at.2))
                    .is_some_and(|s| {
                        &*s.name == "minecraft:test_block"
                            && s.properties.get("mode").map(String::as_str)
                                == Some(mode.serialized_name())
                    })
            })
            .collect()
    }

    // ---- BlockBasedTestInstance ----

    /// `BlockBasedTestInstance.run`.
    fn run_block_based(&mut self, info: &mut Info) -> Result<(), TestError> {
        // findStartBlock
        let starts = self.find_test_blocks(info, TestBlockMode::Start);
        let missing = |game: &Self, info: &Info, key: &str, mode: TestBlockMode| {
            game.info_failure(
                info,
                Text::translate(key, &[Text::translate(&mode.label_key(), &[])]),
            )
        };
        if starts.is_empty() {
            return Err(missing(
                self,
                info,
                "test_block.error.missing",
                TestBlockMode::Start,
            ));
        }
        if starts.len() != 1 {
            return Err(missing(
                self,
                info,
                "test_block.error.too_many",
                TestBlockMode::Start,
            ));
        }
        let start = self.test_block_entity(info, starts[0])?;
        self.trigger_test_block(start);
        info.on_each_tick(Action::BlockBasedTick);
        Ok(())
    }

    /// `helper.fail(message)`: the exception it throws.
    fn info_failure(&self, info: &Info, message: Text) -> TestError {
        info.assertion(message)
    }

    /// The body of `BlockBasedTestInstance.run`'s `onEachTick`.
    fn block_based_tick(&mut self, info: &mut Info) -> Result<(), TestError> {
        let accepts = self.find_test_blocks(info, TestBlockMode::Accept);
        if accepts.is_empty() {
            return Err(self.info_failure(
                info,
                Text::translate(
                    "test_block.error.missing",
                    &[Text::translate(&TestBlockMode::Accept.label_key(), &[])],
                ),
            ));
        }
        let mut accepted = false;
        for relative in accepts {
            let at = self.test_block_entity(info, relative)?;
            accepted |= self.test_block_triggered(at);
        }
        if accepted {
            info.succeed();
            return Ok(());
        }
        // forAllTriggeredTestBlocks(FAIL, fail with the block's message)
        for relative in self.find_test_blocks(info, TestBlockMode::Fail) {
            let at = self.test_block_entity(info, relative)?;
            if self.test_block_triggered(at) {
                let message = self.test_block_message(at);
                return Err(self.info_failure(info, Text::literal(message)));
            }
        }
        // forAllTriggeredTestBlocks(LOG, trigger), each one reset after
        for relative in self.find_test_blocks(info, TestBlockMode::Log) {
            let at = self.test_block_entity(info, relative)?;
            if self.test_block_triggered(at) {
                self.trigger_test_block(at);
                self.reset_test_block(at);
            }
        }
        Ok(())
    }

    // ---- TestInstanceBlockEntity ----

    /// `setRunning`, `setSuccess` and `setErrorMessage`: the status and the
    /// error message it carries (`withStatus` clears it).
    fn set_test_status(&mut self, pos: Pos, status: Status, error: Option<&Text>) {
        let Some(block) = self.test_instance_mut(pos) else {
            return;
        };
        block.data.status = status;
        block.data.error_message = error.map(|t| component_nbt(&t.component()));
        self.mark_block_entity_updated(pos);
    }

    /// `markError`.
    fn mark_test_error(&mut self, pos: Pos, at: Pos, message: &Text) {
        let Some(block) = self.test_instance_mut(pos) else {
            return;
        };
        block.error_markers.push(ErrorMarker {
            pos: [at.0, at.1, at.2],
            text: component_nbt(&message.component()),
        });
        self.mark_block_entity_updated(pos);
    }

    // ---- listeners ----

    /// The listeners of a finished test, in the order they were added:
    /// `ReportGameListener`, the `TestSummaryDisplayer`, the failure tracker
    /// and the runner's batch listener.
    fn test_finished(&mut self, info: &Info) {
        let passed = info.error.is_none();
        self.report_game_test(info, passed);
        // TestSummaryDisplayer: only the tests the command started (the
        // listener is not passed on to a rerun's copy).
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        let tracked = runner.tracked.iter_mut().find(|t| t.id == info.id);
        let counted = tracked.is_some();
        if let Some(t) = tracked {
            t.done = true;
            t.failed = !passed;
        }
        let all_done = counted && runner.tracked.iter().all(|t| t.done);
        let total = runner.tracked.len();
        let failed_required = runner
            .tracked
            .iter()
            .filter(|t| t.failed && t.required)
            .count();
        let failed_optional = runner
            .tracked
            .iter()
            .filter(|t| t.failed && !t.required)
            .count();
        let halt = runner.halt_on_error;
        if all_done {
            let out = &mut self.game_tests.outbox;
            out.push(Out::Console {
                text: Text::translate("commands.test.summary", &[number(total as i32)])
                    .with_color(text::WHITE),
                broadcast: true,
            });
            if failed_required > 0 {
                out.push(Out::Console {
                    text: Text::translate(
                        "commands.test.summary.failed",
                        &[number(failed_required as i32)],
                    )
                    .with_color(text::RED),
                    broadcast: false,
                });
            } else {
                out.push(Out::Console {
                    text: Text::translate("commands.test.summary.all_required_passed", &[])
                        .with_color(text::GREEN),
                    broadcast: true,
                });
            }
            if failed_optional > 0 {
                out.push(Out::Console {
                    text: Text::translate(
                        "commands.test.summary.optional_failed",
                        &[number(failed_optional as i32)],
                    ),
                    broadcast: false,
                });
            }
        }
        // FailedTestTracker.rememberFailedTest
        if !passed && counted {
            self.game_tests.failed.push(info.key.clone());
        }
        // The runner's batch listener.
        if passed {
            self.remove_test_barriers(info.pos);
        } else if halt {
            self.release_forced_chunks();
            self.clear_game_tests();
            return;
        }
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        runner.batch_pending = runner.batch_pending.saturating_sub(1);
        if runner.batch_pending == 0 {
            let next = runner.current + 1;
            self.release_forced_chunks();
            self.run_batch(next);
        }
    }

    /// `ReportGameListener.testPassed` and `testFailed`.
    fn report_game_test(&mut self, info: &Info, passed: bool) {
        let Some(runner) = self.game_tests.runner.as_mut() else {
            return;
        };
        let (attempts, successes) = report_of(&mut runner.reports, info.origin);
        if passed {
            *successes += 1;
        }
        let (attempts, successes) = (*attempts, *successes);
        let (max_attempts, required) = (info.def.max_attempts, info.def.required_successes);
        let flaky = max_attempts > 1;
        let retry = info.retry;
        let again = if passed {
            if retry.has_retries() {
                let line = retry_line(info, retry, attempts, successes, true);
                self.report_passed(info, &line);
                retry.has_tries_left(attempts, successes)
            } else if !flaky {
                let line = format!(
                    "{} passed! ({}ms / {}gameticks)",
                    info.key, info.run_ms, info.tick_count
                );
                self.report_passed(info, &line);
                false
            } else if successes >= required {
                let line = format!(
                    "{} passed {successes} times of {attempts} attempts.",
                    info.key
                );
                self.report_passed(info, &line);
                false
            } else {
                self.say(
                    text::GREEN,
                    format!(
                        "Flaky test {} succeeded, attempt: {attempts} successes: {successes}",
                        info.key
                    ),
                );
                true
            }
        } else if !flaky {
            self.report_failure(info, None);
            if retry.has_retries() {
                let line = retry_line(info, retry, attempts, successes, false);
                self.say(text::RED, line);
                retry.has_tries_left(attempts, successes)
            } else {
                false
            }
        } else {
            let mut line = format!(
                "Flaky test {} failed, attempt: {attempts}/{max_attempts}",
                info.key
            );
            if required > 1 {
                line.push_str(&format!(", successes: {successes} ({required} required)"));
            }
            self.say(text::YELLOW, line);
            if max_attempts - attempts + successes >= required {
                true
            } else {
                self.report_failure(info, Some((attempts, successes)));
                false
            }
        };
        if again {
            self.rerun_test(info);
        }
    }

    /// `ReportGameListener.reportPassed`.
    fn report_passed(&mut self, info: &Info, line: &str) {
        self.set_test_status(info.pos, Status::Finished, None);
        self.say(text::GREEN, line.to_owned());
    }

    /// `ReportGameListener.reportFailure`: the test's own error, or the
    /// `ExhaustedAttemptsException` of a flaky test that ran out of
    /// attempts (`attempts`, `successes`), whose cause is the test's error.
    fn report_failure(&mut self, info: &Info, exhausted: Option<(i32, i32)>) {
        let Some(error) = &info.error else {
            return;
        };
        let (description, message) = match exhausted {
            None => (error.description(), error.message()),
            Some((attempts, successes)) => {
                let text = format!(
                    "Not enough successes: {successes} out of {attempts} attempts. Required successes: {}. max attempts: {}.",
                    info.def.required_successes, info.def.max_attempts
                );
                // describeError(error) is the cause's message.
                let cause = error.message();
                (
                    Text::literal(cause.clone()),
                    format!("{text} cause: {cause}"),
                )
            }
        };
        self.set_test_status(info.pos, Status::Finished, Some(&description));
        let required = info.def.required;
        let message = format!(
            "{}{} failed! {}",
            if required { "" } else { "(optional) " },
            info.key,
            message
        );
        self.say(if required { text::RED } else { text::YELLOW }, message);
        if let TestError::AssertPos {
            message, absolute, ..
        } = error
        {
            self.mark_test_error(info.pos, *absolute, message);
        }
    }

    /// `ReportGameListener.say`.
    fn say(&mut self, color: u32, text: String) {
        self.game_tests
            .outbox
            .push(Out::Players(Text::literal(text).with_color(color)));
    }

    /// Sends what the runner said: the server's console log (and the
    /// operators, when it broadcasts) and the players' chat.
    fn flush_game_test_output(&mut self) {
        for out in std::mem::take(&mut self.game_tests.outbox) {
            match out {
                Out::Console { text, broadcast } => {
                    let plain = text.plain();
                    if self.options.dedicated.is_some() {
                        super::dedicated::log("INFO", &plain);
                    } else {
                        log::info!("{plain}");
                    }
                    if broadcast {
                        // CommandSourceStack.broadcastToAdmins as the server.
                        self.console_broadcast = Some(true);
                        self.broadcast_to_admins(text);
                        self.console_broadcast = None;
                        if let Some(mut host) = self.lan.take() {
                            self.send_admin_feedback(&mut host, None);
                            self.lan = Some(host);
                        }
                        self.admin_feedback.clear();
                    }
                }
                Out::Players(text) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.system_message(component_nbt(&text.component()));
                    }
                    if self.serving.is_none() && self.options.dedicated.is_none() {
                        self.system_message(text);
                    }
                }
            }
        }
    }
}

/// `ReportGameListener.handleRetry`'s line: the counters, then the test's
/// result.
fn retry_line(info: &Info, retry: Retry, attempts: i32, successes: i32, passed: bool) -> String {
    let mut report = format!(
        "[Run: {attempts:>4}, Ok: {successes:>4}, Fail: {:>4}",
        attempts - successes
    );
    if !retry.unlimited() {
        report.push_str(&format!(", Left: {:>4}", retry.number_of_tries - attempts));
    }
    report.push(']');
    let name = format!(
        "{} {}! {}ms",
        info.key,
        if passed { "passed" } else { "failed" },
        info.run_ms
    );
    format!("{report:<53}{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(block_based: bool, max_ticks: i32, setup_ticks: i32) -> Info {
        Info::new(
            1,
            "minecraft:t".into(),
            TestDef::sample(block_based, max_ticks, setup_ticks),
            Some((0, 64, 0)),
            Dim::Overworld,
            Rotation::None,
        )
    }

    #[test]
    fn a_test_keeps_its_level_awake_until_it_is_done() {
        let mut tests = GameTests::default();
        assert!(!tests.has_tests_in(Dim::Nether));
        let mut nether = info(false, 10, 0);
        nether.dim = Dim::Nether;
        tests.ticker.push(nether);
        assert!(tests.has_tests_in(Dim::Nether));
        assert!(!tests.has_tests_in(Dim::Overworld));
        // A rerun keeps the level of the test it copies.
        assert_eq!(tests.ticker[0].copy_reset(2).dim, Dim::Nether);
    }

    #[test]
    fn a_grid_has_a_state_per_dimension_starting_at_its_own_corner() {
        let mut grid = Grid::new((4, 9), 10).clearing();
        assert!(grid.clear_on_batch);
        grid.states
            .push((Dim::Overworld, DimGrid::new((4, 70, 12))));
        grid.states.push((Dim::Nether, DimGrid::new((4, 32, 12))));
        let nether = &grid.states[1].1;
        assert_eq!(nether.next, (4, 32, 12));
        assert_eq!(nether.row_z, (12, 13));
    }

    #[test]
    fn verify_runs_each_rotation_a_hundred_times_in_its_own_batch() {
        let mut id = 0;
        let batches = verify_batches(&[info(true, 100, 0)], || {
            id += 1;
            id
        });
        assert_eq!(batches.len(), 4);
        for (i, batch) in batches.iter().enumerate() {
            assert_eq!(batch.index, i as i32);
            assert_eq!(batch.infos.len(), 100);
            assert!(
                batch
                    .infos
                    .iter()
                    .all(|t| t.extra_rotation == Rotation::ALL[i])
            );
        }
        assert_eq!(id, 400);
    }

    #[test]
    fn start_execution_waits_for_the_setup_ticks_and_the_delay() {
        // always_pass: setup_ticks 1, delay 1: ticks -2, -1 and 0 follow.
        let mut i = info(false, 1, 1);
        i.start_execution(1);
        assert_eq!(i.tick_count, -3);
    }

    #[test]
    fn on_each_tick_runs_every_tick_up_to_the_timeout() {
        let mut i = info(true, 5, 0);
        i.tick_count = 2;
        i.on_each_tick(Action::BlockBasedTick);
        let times: Vec<i64> = i.run_at.iter().map(|(t, _)| *t).collect();
        assert_eq!(times, vec![2, 3, 4]);
    }

    #[test]
    fn a_failure_is_not_overwritten_by_success_and_stamps_the_tick() {
        let mut i = info(true, 5, 0);
        i.tick_count = 3;
        i.fail(Text::literal("boom"));
        i.succeed();
        assert!(!i.done);
        assert!(matches!(i.error, Some(TestError::Assert { tick: 3, .. })));
        let mut ok = info(true, 5, 0);
        ok.succeed();
        assert!(ok.done && ok.error.is_none());
    }

    #[test]
    fn unknown_errors_report_their_reason() {
        let error = TestError::Unknown("no such function".into());
        assert_eq!(error.message(), "no such function");
    }

    #[test]
    fn retry_options_decide_whether_a_test_runs_again() {
        // noRetries: one try, no rerun.
        assert!(!Retry::none().has_retries());
        assert!(!Retry::none().has_tries_left(1, 1));
        // 3 times, not until failed: runs on through failures.
        let three = Retry::new(3, false);
        assert!(three.has_retries());
        assert!(three.has_tries_left(1, 0));
        assert!(three.has_tries_left(2, 2));
        assert!(!three.has_tries_left(3, 3));
        // Until failed: stops at the first failure, and 0 means unlimited.
        let until = Retry::new(0, true);
        assert!(until.unlimited());
        assert!(until.has_tries_left(50, 50));
        assert!(!until.has_tries_left(50, 49));
    }

    #[test]
    fn a_retry_line_counts_runs_and_pads_to_53() {
        let info = info(true, 10, 0);
        let line = retry_line(&info, Retry::new(5, false), 2, 1, false);
        assert!(line.starts_with("[Run:    2, Ok:    1, Fail:    1, Left:    3]"));
        assert_eq!(line.find("minecraft:t"), Some(53));
        assert!(line.ends_with("minecraft:t failed! 0ms"));
        let unlimited = retry_line(&info, Retry::new(0, true), 1, 1, true);
        assert!(unlimited.starts_with("[Run:    1, Ok:    1, Fail:    0]"));
    }

    #[test]
    fn a_rerun_is_a_copy_that_shares_the_originals_counters() {
        let original = info(true, 10, 0).with_retry(Retry::new(2, false));
        let copy = original.copy_reset(9);
        assert_eq!((copy.id, copy.origin, copy.rerun), (9, 1, true));
        assert_eq!(copy.retry, original.retry);
        assert!(copy.has_pos && copy.pos == original.pos);
        let mut reports = Vec::new();
        *report_of(&mut reports, copy.origin).0 += 1;
        *report_of(&mut reports, original.origin).1 += 1;
        assert_eq!(reports, vec![(1, 1, 1)]);
    }
}
