//! Behaviour tests over an in-memory level. They need the prepared client
//! resources (`HORIZON_RESOURCES`, or `.local/horizon-client/<version>` at the
//! repository root) and are skipped when those are absent.
use super::*;
use crate::chunk::BlockState;
use horizon_core::random::RandomSource;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::OnceLock,
};

struct Fixture {
    registry: BlockRegistry,
    behaviours: Behaviours,
}

fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let root = std::env::var_os("HORIZON_RESOURCES")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../.local/horizon-client")
                        .join(horizon_core::VANILLA_VERSION)
                });
            if !root.join("block-behaviours.json").exists() {
                eprintln!("skipping level tests: no resources at {}", root.display());
                return None;
            }
            let registry = data::load_registry(&root).expect("registry");
            let behaviours = Behaviours::load(&registry, &root).expect("behaviours");
            Some(Fixture {
                registry,
                behaviours,
            })
        })
        .as_ref()
}

pub(crate) struct TestLevel {
    blocks: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    pub time: i64,
    pub day_time: i64,
    pub events: Vec<LevelEvent>,
    pub block_light: HashMap<BlockPos, i32>,
    /// `Level.isClientSide`.
    pub client_side: bool,
    /// Positions that refuse fluids, fire and piston moves, as a land claim would.
    pub guarded: HashSet<BlockPos>,
    /// What `adjust_signal` answers at a position: the weak and direct signal,
    /// and the only direction they go in, if any.
    pub signals: HashMap<BlockPos, (i32, i32, Option<Direction>)>,
    /// What `adjust_analog_output` answers at a position, for the states in
    /// `analog_states` (`adjust_has_analog_output`).
    pub analog: HashMap<BlockPos, i32>,
    pub analog_states: HashSet<BlockStateId>,
    /// What `before_fire_odds` answers for a state.
    pub fire_odds: HashMap<BlockStateId, (i32, i32)>,
    /// What `before_smoke_source` answers for a state.
    pub smoke_sources: HashMap<BlockStateId, bool>,
}
impl TestLevel {
    fn new(b: &Behaviours) -> Self {
        Self {
            blocks: HashMap::new(),
            air: b.air(),
            random: LegacyRandom::new(42),
            signals: HashMap::new(),
            analog: HashMap::new(),
            analog_states: HashSet::new(),
            time: 0,
            day_time: 0,
            events: Vec::new(),
            block_light: HashMap::new(),
            client_side: false,
            guarded: HashSet::new(),
            fire_odds: HashMap::new(),
            smoke_sources: HashMap::new(),
        }
    }
}
impl LevelAccess for TestLevel {
    fn adjust_has_analog_output(&self, state: BlockStateId, vanilla: bool) -> bool {
        vanilla || self.analog_states.contains(&state)
    }
    fn adjust_analog_output(
        &self,
        pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
        vanilla: i32,
    ) -> i32 {
        self.analog.get(&pos).copied().unwrap_or(vanilla)
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        _state: BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        match self.signals.get(&pos) {
            Some((_, _, Some(only))) if *only != direction => 0,
            Some((weak, strong, _)) => {
                if direct {
                    *strong
                } else {
                    *weak
                }
            }
            None => vanilla,
        }
    }
    fn is_client_side(&self) -> bool {
        self.client_side
    }
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.blocks.get(&pos).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        Some(self.blocks.insert(pos, state).unwrap_or(self.air))
    }
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.time
    }
    fn day_time(&self) -> i64 {
        self.day_time
    }
    fn block_light(&self, pos: BlockPos) -> i32 {
        self.block_light.get(&pos).copied().unwrap_or(0)
    }
    fn emit(&mut self, event: LevelEvent) {
        self.events.push(event);
    }
    fn allow_spread(&self, _kind: Spread, _from: BlockPos, to: BlockPos) -> bool {
        !self.guarded.contains(&to)
    }
    fn allow_piston_move(
        &self,
        _piston: BlockPos,
        direction: Direction,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) -> bool {
        !push
            .iter()
            .flat_map(|&p| [p, update::rel(p, direction)])
            .chain(destroy.iter().copied())
            .any(|p| self.guarded.contains(&p))
    }
    fn before_fire_odds(&self, state: BlockStateId, odds: &mut Option<(i32, i32)>) {
        if let Some(&answer) = self.fire_odds.get(&state) {
            *odds = Some(answer);
        }
    }
    fn before_smoke_source(&self, state: BlockStateId, smoke: &mut Option<bool>) {
        if let Some(&answer) = self.smoke_sources.get(&state) {
            *smoke = Some(answer);
        }
    }
}

struct World<'f> {
    f: &'f Fixture,
    level: TestLevel,
    ticks: Ticks,
}
impl<'f> World<'f> {
    fn new(f: &'f Fixture) -> Self {
        let mut ticks = Ticks::new(0);
        for x in -4..4 {
            for z in -4..4 {
                ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
            }
        }
        Self {
            level: TestLevel::new(&f.behaviours),
            f,
            ticks,
        }
    }
    fn b(&self) -> &'f Behaviours {
        &self.f.behaviours
    }
    fn id(&self, name: &str, props: &[(&str, &str)]) -> BlockStateId {
        let state = BlockState {
            name: format!("minecraft:{name}"),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        self.f
            .registry
            .resolve(&state)
            .unwrap_or_else(|_| panic!("unknown state {name} {props:?}"))
    }
    fn set(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        self.f.behaviours.set_block(
            &mut self.level,
            &mut self.ticks,
            pos,
            state,
            flags::UPDATE_ALL,
        )
    }
    /// Raw storage write without updates (world generation).
    fn put(&mut self, pos: BlockPos, state: BlockStateId) {
        self.level.blocks.insert(pos, state);
    }
    fn get(&self, pos: BlockPos) -> BlockStateId {
        self.level.block_state(pos)
    }
    fn name(&self, pos: BlockPos) -> String {
        let state = self.f.registry.state(self.get(pos)).unwrap();
        let props: Vec<String> = state
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!(
            "{}[{}]",
            state.name.trim_start_matches("minecraft:"),
            props.join(",")
        )
    }
    fn prop(&self, pos: BlockPos, key: &str) -> String {
        self.f
            .registry
            .state(self.get(pos))
            .unwrap()
            .properties
            .get(key)
            .cloned()
            .unwrap_or_default()
    }
    fn tick(&mut self) {
        self.level.time += 1;
        self.f
            .behaviours
            .tick_scheduled(&mut self.level, &mut self.ticks);
    }
    /// Scheduled ticks, block events and block entities, as `ServerLevel.tick`.
    fn full_tick(&mut self) {
        self.tick();
        self.f
            .behaviours
            .run_block_events(&mut self.level, &mut self.ticks);
        self.f
            .behaviours
            .tick_block_entities(&mut self.level, &mut self.ticks);
    }
    fn floor(&mut self, y: i32, radius: i32, block: &str) {
        let state = self.id(block, &[]);
        for x in -radius..=radius {
            for z in -radius..=radius {
                self.put(BlockPos::new(x, y, z), state);
            }
        }
    }
}

macro_rules! world {
    () => {{
        let Some(f) = fixture() else {
            return;
        };
        World::new(f)
    }};
}

#[test]
fn block_observation_follows_successful_commits_only() {
    use std::cell::RefCell;

    #[derive(Debug, PartialEq)]
    enum Observation {
        Block(BlockPos, BlockStateId, BlockStateId),
        Neighbor,
    }
    struct Observer {
        inner: TestLevel,
        loaded: bool,
        seen: RefCell<Vec<Observation>>,
    }
    impl LevelAccess for Observer {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, _pos: BlockPos) -> bool {
            self.loaded
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn observe_block_changed(&self, pos: BlockPos, old: BlockStateId, new: BlockStateId) {
            assert_eq!(self.block_state(pos), new);
            self.seen
                .borrow_mut()
                .push(Observation::Block(pos, old, new));
        }
        fn observe_neighbor_changed(&self, _: BlockPos, _: BlockStateId, _: BlockId, _: bool) {
            self.seen.borrow_mut().push(Observation::Neighbor);
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let mut level = Observer {
        inner: TestLevel::new(b),
        loaded: true,
        seen: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    let pos = BlockPos::new(0, 80, 0);
    let stone = f.registry.default_state("minecraft:stone").unwrap();
    assert!(b.set_block(&mut level, &mut ticks, pos, stone, flags::UPDATE_ALL));
    let seen = level.seen.borrow();
    assert_eq!(seen[0], Observation::Block(pos, b.air(), stone));
    assert!(seen[1..].contains(&Observation::Neighbor));
    drop(seen);
    level.seen.borrow_mut().clear();
    assert!(!b.set_block(&mut level, &mut ticks, pos, stone, flags::UPDATE_ALL));
    level.loaded = false;
    assert!(!b.set_block(&mut level, &mut ticks, pos, b.air(), flags::UPDATE_ALL));
    level.loaded = true;
    assert!(!b.set_block(
        &mut level,
        &mut ticks,
        BlockPos::new(0, 320, 0),
        stone,
        flags::UPDATE_ALL
    ));
    assert!(level.seen.borrow().is_empty());
    assert!(b.set_block(&mut level, &mut ticks, pos, b.air(), flags::UPDATE_ALL));
    assert_eq!(
        level.seen.borrow()[0],
        Observation::Block(pos, stone, b.air())
    );
}

#[test]
fn neighbor_observation_follows_dispatch_order() {
    use std::cell::RefCell;

    struct Observer {
        inner: TestLevel,
        seen: RefCell<Vec<(BlockPos, BlockStateId, BlockId, bool)>>,
    }
    impl LevelAccess for Observer {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn observe_neighbor_changed(
            &self,
            pos: BlockPos,
            state: BlockStateId,
            changed: BlockId,
            moved: bool,
        ) {
            assert_eq!(self.block_state(pos), state);
            self.seen.borrow_mut().push((pos, state, changed, moved));
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let mut level = Observer {
        inner: TestLevel::new(b),
        seen: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    let changed = b.table.block_by_name("minecraft:stone").unwrap();
    b.update_neighbors_at(&mut level, &mut ticks, BlockPos::new(0, 0, 0), changed);
    let expected = [
        (-1, 0, 0),
        (1, 0, 0),
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
    ];
    let actual = level.seen.borrow();
    assert_eq!(actual.len(), expected.len());
    for (event, (x, y, z)) in actual.iter().zip(expected) {
        assert_eq!(*event, (BlockPos::new(x, y, z), b.air(), changed, false));
    }
    drop(actual);
    b.sim(&mut level, &mut ticks)
        .neighbor_changed(b.air(), BlockPos::new(3, 4, 5), changed, true);
    assert_eq!(
        level.seen.borrow().last(),
        Some(&(BlockPos::new(3, 4, 5), b.air(), changed, true))
    );
}

#[test]
fn random_tick_hook_can_change_the_block_and_skip_its_own_tick() {
    use std::cell::Cell;

    struct Grower {
        inner: TestLevel,
        b: &'static Behaviours,
        /// The state the hook sets, if any, and the `setBlock` flags it sets
        /// it with.
        to: Option<BlockStateId>,
        flags: i32,
        handle: bool,
        /// `neighborChanged` calls the hook's writes made.
        notified: Cell<usize>,
        seen: Vec<(BlockPos, BlockStateId)>,
        /// Moves on with every write, as `World::revision` does.
        revision: u64,
    }
    impl LevelAccess for Grower {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.revision += 1;
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn observe_neighbor_changed(
            &self,
            _pos: BlockPos,
            _state: BlockStateId,
            _changed: BlockId,
            _moved: bool,
        ) {
            self.notified.set(self.notified.get() + 1);
        }
        fn before_random_tick(
            &mut self,
            ticks: &mut Ticks,
            pos: BlockPos,
            state: &mut BlockStateId,
            handled: &mut bool,
        ) {
            assert_eq!(self.block_state(pos), *state);
            self.seen.push((pos, *state));
            // The hook, then the level's part of the contract as `GameLevel`
            // keeps it: re-read the block if the hook wrote.
            let revision = self.revision;
            if let Some(to) = self.to {
                let b = self.b;
                assert!(b.set_block(self, ticks, pos, to, self.flags));
            }
            *handled = self.handle;
            if !*handled && self.revision != revision {
                *state = self.block_state(pos);
            }
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let wheat = f.registry.default_state("minecraft:wheat").unwrap();
    let grown = b.table.set_int(wheat, b.props.age, 3);
    let next = b.table.set_int(wheat, b.props.age, 4);
    let mut level = Grower {
        inner: TestLevel::new(b),
        b,
        to: Some(grown),
        flags: flags::UPDATE_CLIENTS,
        handle: true,
        notified: Cell::new(0),
        seen: Vec::new(),
        revision: 0,
    };
    let mut ticks = Ticks::new(0);
    let stone = f.registry.default_state("minecraft:stone").unwrap();
    let (low, high) = (BlockPos::new(0, 1, 0), BlockPos::new(4, 1, 0));
    level.set_raw(low, stone);
    level.set_raw(high, wheat);
    let draw = |level: &Grower| level.inner.random.clone().next_long();
    let before = draw(&level);
    b.random_tick(&mut level, &mut ticks, low);
    assert!(level.seen.is_empty());
    b.random_tick(&mut level, &mut ticks, high);
    assert_eq!(level.seen, [(high, wheat)]);
    assert_eq!(level.block_state(high), grown);
    // The crop's own tick never ran: it would have drawn from the random.
    assert_eq!(draw(&level), before);
    // Grown with flag 2, as `CropBlock.randomTick` grows a crop, no
    // neighbour hears of it; with `UPDATE_ALL` the six do.
    assert_eq!(level.notified.get(), 0);
    level.flags = flags::UPDATE_ALL;
    level.set_raw(high, wheat);
    b.random_tick(&mut level, &mut ticks, high);
    assert_eq!(level.block_state(high), grown);
    assert_eq!(level.notified.get(), 6);
    level.flags = flags::UPDATE_CLIENTS;
    // Unhandled and unchanged, the crop ticks as before.
    (level.to, level.handle) = (None, false);
    level.set_raw(high, wheat);
    b.random_tick(&mut level, &mut ticks, high);
    assert_eq!(level.seen.len(), 3);
    assert_ne!(draw(&level), before);
    // Changed but unhandled, the crop ticks as the hook left it: it stays at
    // the hook's age or grows from there, never from the age it had.
    level.to = Some(grown);
    let mut grew = false;
    for _ in 0..1000 {
        level.set_raw(high, wheat);
        b.random_tick(&mut level, &mut ticks, high);
        let now = level.block_state(high);
        assert!(now == grown || now == next, "{now:?}");
        grew |= now == next;
    }
    assert!(grew);
    // Changed into a block that doesn't tick randomly, nothing more runs.
    level.to = Some(stone);
    level.set_raw(high, wheat);
    let before = draw(&level);
    b.random_tick(&mut level, &mut ticks, high);
    assert_eq!(level.block_state(high), stone);
    assert_eq!(draw(&level), before);
}

#[test]
fn tick_block_hook_can_change_the_block_and_skip_its_own_tick() {
    use horizon_core::ticks::TickPriority;

    struct Ticked {
        inner: TestLevel,
        b: &'static Behaviours,
        /// The state the hook sets, if any, and the `setBlock` flags it sets
        /// it with.
        to: Option<BlockStateId>,
        flags: i32,
        handle: bool,
        seen: Vec<(BlockPos, BlockStateId)>,
        /// Moves on with every write, as `World::revision` does.
        revision: u64,
    }
    impl LevelAccess for Ticked {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.revision += 1;
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn before_tick_block(
            &mut self,
            ticks: &mut Ticks,
            pos: BlockPos,
            state: &mut BlockStateId,
            handled: &mut bool,
        ) {
            assert_eq!(self.block_state(pos), *state);
            self.seen.push((pos, *state));
            // The hook, then the level's part of the contract as `GameLevel`
            // keeps it: re-read the block if the hook wrote.
            let revision = self.revision;
            if let Some(to) = self.to {
                let b = self.b;
                assert!(b.set_block(self, ticks, pos, to, self.flags));
            }
            *handled = self.handle;
            if !*handled && self.revision != revision {
                *state = self.block_state(pos);
            }
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let target = f.registry.default_state("minecraft:target").unwrap();
    let powered = b.table.set_int(target, b.props.power, 15);
    let to = b.table.set_int(target, b.props.power, 7);
    let mut level = Ticked {
        inner: TestLevel::new(b),
        b,
        to: Some(to),
        // A target's own tick writes with `setBlockAndUpdate`.
        flags: flags::UPDATE_ALL,
        handle: true,
        seen: Vec::new(),
        revision: 0,
    };
    let mut ticks = Ticks::new(0);
    ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    let pos = BlockPos::new(1, 1, 1);
    // The due ticks run as the game runs them, stepped (scheduled_begin,
    // scheduled_step, scheduled_finish), or as tests do, through
    // tick_scheduled.
    let run = |level: &mut Ticked, ticks: &mut Ticks, stepped: bool| {
        if stepped {
            assert!(b.scheduled_begin(level, ticks, false));
            while b.scheduled_step(level, ticks, false) {}
            b.scheduled_finish(ticks, false);
        } else {
            b.tick_scheduled(level, ticks);
        }
    };
    // One tick of the block at `pos` falls due.
    let due = |level: &mut Ticked, ticks: &mut Ticks, stepped: bool| {
        let (time, block) = (level.inner.time, b.block(level.block_state(pos)));
        ticks.schedule_block(time, pos, block, 1, TickPriority::Normal);
        level.inner.time += 1;
        run(level, ticks, stepped);
    };
    let rod = |facing: &str, powered: &str| {
        let lightning_rod = f.registry.default_state("minecraft:lightning_rod").unwrap();
        let mut s = f.registry.state(lightning_rod).unwrap().clone();
        s.properties.insert("facing".into(), facing.into());
        s.properties.insert("powered".into(), powered.into());
        f.registry.resolve(&s).unwrap()
    };
    for stepped in [false, true] {
        level.seen.clear();
        (level.to, level.handle) = (Some(to), true);
        level.set_raw(pos, powered);
        due(&mut level, &mut ticks, stepped);
        assert_eq!(level.seen, [(pos, powered)]);
        assert_eq!(level.block_state(pos), to);
        // Unhandled, the target's own tick turns it off.
        (level.to, level.handle) = (None, false);
        level.set_raw(pos, powered);
        due(&mut level, &mut ticks, stepped);
        assert_eq!(level.block_state(pos), target);
        // A tick whose block was replaced runs nothing.
        let stone = f.registry.default_state("minecraft:stone").unwrap();
        level.set_raw(pos, powered);
        let block = b.block(powered);
        ticks.schedule_block(level.inner.time, pos, block, 1, TickPriority::Normal);
        level.inner.time += 1;
        level.set_raw(pos, stone);
        run(&mut level, &mut ticks, stepped);
        assert_eq!(level.seen.len(), 2);
        // Changed into another block and unhandled, nothing more runs.
        level.to = Some(stone);
        level.set_raw(pos, powered);
        due(&mut level, &mut ticks, stepped);
        assert_eq!(level.seen.len(), 3);
        assert_eq!(level.block_state(pos), stone);
        // Changed but unhandled, the block ticks as the hook left it: the
        // rod's tick unpowers it facing where the hook turned it.
        (level.to, level.handle) = (Some(rod("north", "true")), false);
        level.set_raw(pos, rod("up", "true"));
        due(&mut level, &mut ticks, stepped);
        assert_eq!(level.block_state(pos), rod("north", "false"));
    }
}

#[test]
fn state_table_matches_registry() {
    let Some(f) = fixture() else {
        return;
    };
    let t = &f.behaviours.table;
    assert_eq!(t.state_count(), f.registry.len());
    let stairs = f.registry.default_state("minecraft:oak_stairs").unwrap();
    let key = f.behaviours.props.facing;
    let east = t.with_text(stairs, key, "east").unwrap();
    assert_eq!(f.registry.state(east).unwrap().properties["facing"], "east");
    assert_eq!(t.text(east, f.behaviours.props.half), "bottom");
    assert!(classes::Class::ALL.len() <= 320);
}

#[test]
fn fences_connect_and_disconnect() {
    let mut w = world!();
    let fence = w.id("oak_fence", &[]);
    let a = BlockPos::new(0, 1, 0);
    let c = BlockPos::new(1, 1, 0);
    w.set(a, fence);
    w.set(c, fence);
    assert_eq!(w.prop(a, "east"), "true");
    assert_eq!(w.prop(c, "west"), "true");
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 1, -1), stone);
    assert_eq!(w.prop(a, "north"), "true", "fences connect to sturdy faces");
    let air = w.b().air();
    w.set(c, air);
    assert_eq!(w.prop(a, "east"), "false");
}

#[test]
fn walls_and_panes_connect() {
    let mut w = world!();
    let wall = w.id("cobblestone_wall", &[]);
    let pane = w.id("glass_pane", &[]);
    w.set(BlockPos::new(0, 1, 0), wall);
    w.set(BlockPos::new(0, 1, 1), wall);
    w.set(BlockPos::new(0, 1, 2), wall);
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "north"), "low");
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "south"), "low");
    assert_eq!(
        w.prop(BlockPos::new(0, 1, 1), "up"),
        "false",
        "straight wall drops its post"
    );
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 2, 1), stone);
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "north"), "tall");
    w.set(BlockPos::new(5, 1, 0), pane);
    w.set(BlockPos::new(6, 1, 0), pane);
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "east"), "true");
}

#[test]
fn stairs_form_corners() {
    let mut w = world!();
    let north = w.id("oak_stairs", &[("facing", "north")]);
    let east = w.id("oak_stairs", &[("facing", "east")]);
    w.set(BlockPos::new(0, 1, 0), north);
    w.set(BlockPos::new(0, 1, -1), east);
    // The north-facing stair has an east-facing stair behind it.
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "shape"), "outer_right");
}

#[test]
fn torch_and_door_halves_break_with_support() {
    let mut w = world!();
    w.floor(0, 3, "stone");
    let torch = w.id("torch", &[]);
    w.set(BlockPos::new(1, 1, 1), torch);
    let door = w.id("oak_door", &[("half", "lower")]);
    let upper = w.id("oak_door", &[("half", "upper")]);
    w.set(BlockPos::new(0, 1, 0), door);
    w.set(BlockPos::new(0, 2, 0), upper);
    let air = w.b().air();
    w.set(BlockPos::new(1, 0, 1), air);
    assert_eq!(
        w.get(BlockPos::new(1, 1, 1)),
        air,
        "torch pops without support"
    );
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::DropResources { .. }))
    );
    w.set(BlockPos::new(0, 1, 0), air);
    assert_eq!(
        w.get(BlockPos::new(0, 2, 0)),
        air,
        "upper door half goes with the lower"
    );
}

#[test]
fn snowy_grass_and_chests_pair() {
    let mut w = world!();
    let grass = w.id("grass_block", &[]);
    w.set(BlockPos::new(0, 0, 0), grass);
    let snow = w.id("snow", &[]);
    w.set(BlockPos::new(0, 1, 0), snow);
    assert_eq!(w.prop(BlockPos::new(0, 0, 0), "snowy"), "true");
    // A left chest's partner sits clockwise of its facing; removing the
    // partner makes it single again.
    let left = w.id("chest", &[("facing", "north"), ("type", "left")]);
    let right = w.id("chest", &[("facing", "north"), ("type", "right")]);
    w.set(BlockPos::new(3, 1, 0), left);
    w.set(BlockPos::new(4, 1, 0), right);
    let air = w.b().air();
    w.set(BlockPos::new(4, 1, 0), air);
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "type"), "single");
}

#[test]
fn sand_falls_and_concrete_solidifies() {
    let mut w = world!();
    let sand = w.id("sand", &[]);
    w.set(BlockPos::new(0, 5, 0), sand);
    w.tick();
    w.tick();
    assert_eq!(w.get(BlockPos::new(0, 5, 0)), w.b().air());
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::FallingBlock { state, .. } if *state == sand))
    );
    w.floor(0, 2, "stone");
    let water = w.id("water", &[]);
    w.put(BlockPos::new(1, 1, 0), water);
    let powder = w.id("white_concrete_powder", &[]);
    let landing = w.b().falling_block_landed(
        &mut w.level,
        &mut w.ticks,
        BlockPos::new(0, 1, 0),
        powder,
        false,
    );
    assert_eq!(
        landing,
        falling::Landing::Placed(w.id("white_concrete", &[]))
    );
}

#[test]
fn water_spreads_on_a_floor() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), water);
    for _ in 0..60 {
        w.tick();
    }
    for d in 1..=7 {
        let pos = BlockPos::new(d, 1, 0);
        assert_eq!(
            w.prop(pos, "level"),
            d.to_string(),
            "flow at distance {d}: {}",
            w.name(pos)
        );
    }
    assert_eq!(w.get(BlockPos::new(8, 1, 0)), w.b().air());
    // Diagonals follow Manhattan distance.
    assert_eq!(w.prop(BlockPos::new(2, 1, 2), "level"), "4");
}

#[test]
fn refused_spread_keeps_water_out_of_a_guarded_block() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    w.level.guarded.insert(BlockPos::new(3, 1, 0));
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), water);
    for _ in 0..60 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(2, 1, 0), "level"), "2");
    assert_eq!(w.get(BlockPos::new(3, 1, 0)), w.b().air());
    // Water still reaches (4, 1, 0) around the guarded block.
    assert_eq!(w.prop(BlockPos::new(4, 1, 0), "level"), "6");
}

#[test]
fn bucket_source_removal_drains_spread_water() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let at = BlockPos::new(0, 1, 0);
    assert!(w.b().bucket_empty(
        &mut w.level,
        &mut w.ticks,
        at,
        BucketContent::Fluid(crate::fluid::FluidKind::Water),
        "minecraft:item.bucket.empty",
        false,
        false
    ));
    for _ in 0..60 {
        w.tick();
    }
    assert_ne!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        Some("minecraft:water_bucket")
    );
    for _ in 0..120 {
        w.tick();
    }
    for x in -8..=8 {
        for z in -8..=8 {
            assert_eq!(
                w.get(BlockPos::new(x, 1, z)),
                w.b().air(),
                "water remains at {x}, {z}"
            );
        }
    }
}

#[test]
fn water_falls_and_makes_sources() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 4, 0), water);
    for _ in 0..30 {
        w.tick();
    }
    assert_eq!(
        w.prop(BlockPos::new(0, 3, 0), "level"),
        "8",
        "falling water"
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "level"), "8");
    // Two sources make a third (no holes within slope distance, so the
    // sources spread in every direction).
    let mut w = world!();
    w.floor(0, 12, "stone");
    w.set(BlockPos::new(0, 1, 0), water);
    w.set(BlockPos::new(2, 1, 0), water);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), water);
    // On a small platform the water instead seeks the nearest edges.
    let mut w = world!();
    w.floor(0, 4, "stone");
    w.set(BlockPos::new(0, 1, 0), water);
    w.set(BlockPos::new(2, 1, 0), water);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
    assert_eq!(w.prop(BlockPos::new(-1, 1, 0), "level"), "1");
}

#[test]
fn lava_meets_water() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let lava = w.id("lava", &[]);
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), lava);
    w.set(BlockPos::new(1, 1, 0), water);
    assert_eq!(w.get(BlockPos::new(0, 1, 0)), w.id("obsidian", &[]));
    let mut w = world!();
    w.floor(0, 4, "stone");
    w.set(BlockPos::new(0, 1, 0), lava);
    for _ in 0..40 {
        w.tick();
    }
    w.set(BlockPos::new(1, 2, 0), water);
    for _ in 0..40 {
        w.tick();
    }
    assert_eq!(
        w.get(BlockPos::new(1, 1, 0)),
        w.id("cobblestone", &[]),
        "{}",
        w.name(BlockPos::new(1, 1, 0))
    );
}

#[test]
fn waterlogged_blocks_schedule_water() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let slab = w.id("oak_slab", &[("waterlogged", "true")]);
    w.set(BlockPos::new(0, 1, 0), slab);
    // A neighbour shape update schedules the slab's water tick.
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 1, 1), stone);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "level"), "1");
}

#[test]
fn redstone_lever_wire_and_lamp() {
    let mut w = world!();
    w.floor(0, 20, "stone");
    let wire = w.id("redstone_wire", &[]);
    let lamp = w.id("redstone_lamp", &[]);
    let lever = w.id("lever", &[("face", "floor"), ("facing", "north")]);
    for x in 1..=5 {
        w.set(BlockPos::new(x, 1, 0), wire);
    }
    w.set(BlockPos::new(6, 1, 0), lamp);
    w.set(BlockPos::new(0, 1, 0), lever);
    let lever_on = w
        .b()
        .table
        .set_bool(w.get(BlockPos::new(0, 1, 0)), w.b().props.powered, true);
    w.set(BlockPos::new(0, 1, 0), lever_on);
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "power"), "15");
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "power"), "11");
    assert_eq!(w.prop(BlockPos::new(6, 1, 0), "lit"), "true");
    let lever_off = w.b().table.set_bool(lever_on, w.b().props.powered, false);
    w.set(BlockPos::new(0, 1, 0), lever_off);
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "power"), "0");
    for _ in 0..5 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(6, 1, 0), "lit"), "false");
}

#[test]
fn repeater_delays_and_torch_inverts() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let block = w.id("redstone_block", &[]);
    // A repeater takes its input from the side it faces.
    let repeater = w.id("repeater", &[("facing", "west")]);
    w.set(BlockPos::new(1, 1, 0), repeater);
    w.set(BlockPos::new(0, 1, 0), block);
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "powered"), "false");
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "powered"), "true");
    let torch = w.id("redstone_torch", &[]);
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(5, 1, 0), stone);
    w.set(BlockPos::new(5, 2, 0), torch);
    // A redstone block only weakly powers the stone: the torch stays lit.
    w.set(BlockPos::new(6, 1, 0), block);
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(5, 2, 0), "lit"), "true");
    // A lever on the stone strongly powers it.
    let air = w.b().air();
    w.set(BlockPos::new(6, 1, 0), air);
    let lever = w.id(
        "lever",
        &[("face", "wall"), ("facing", "east"), ("powered", "true")],
    );
    w.set(BlockPos::new(6, 1, 0), lever);
    // LeverBlock.pull notifies the attached block's neighbours.
    let lever_block = w.b().block(lever);
    w.b().update_neighbors_at(
        &mut w.level,
        &mut w.ticks,
        BlockPos::new(5, 1, 0),
        lever_block,
    );
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(5, 2, 0), "lit"), "false");
}

#[test]
fn crops_grow_and_leaves_decay() {
    let mut w = world!();
    let farmland = w.id("farmland", &[("moisture", "7")]);
    let wheat = w.id("wheat", &[]);
    w.put(BlockPos::new(0, 0, 0), farmland);
    w.put(BlockPos::new(0, 1, 0), wheat);
    for _ in 0..400 {
        w.b()
            .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(0, 1, 0));
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "age"), "7");
    let leaves = w.id("oak_leaves", &[("distance", "7")]);
    w.put(BlockPos::new(5, 5, 5), leaves);
    w.b()
        .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(5, 5, 5));
    assert_eq!(w.get(BlockPos::new(5, 5, 5)), w.b().air());
    let log = w.id("oak_log", &[]);
    let leaf = w.id("oak_leaves", &[]);
    w.set(BlockPos::new(9, 5, 5), leaf);
    w.set(BlockPos::new(8, 5, 5), log);
    w.tick();
    assert_eq!(w.prop(BlockPos::new(9, 5, 5), "distance"), "1");
}

impl World<'_> {
    /// Right-click `item` on the `face` of `clicked`, hitting the face centre
    /// (offset vertically by `dy`), looking with `yaw`/`pitch`.
    fn place(
        &mut self,
        item: &str,
        clicked: BlockPos,
        face: Direction,
        yaw: f32,
        pitch: f32,
        dy: f64,
    ) -> PlaceOutcome {
        let [nx, ny, nz] = face.normal();
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(clicked.x) + 0.5 + f64::from(nx) * 0.5,
            f64::from(clicked.y) + 0.5 + f64::from(ny) * 0.5 + dy,
            f64::from(clicked.z) + 0.5 + f64::from(nz) * 0.5,
        );
        let ctx = PlaceContext {
            clicked_pos: clicked,
            face,
            hit,
            inside: false,
            yaw,
            pitch,
            sneaking: false,
            item: format!("minecraft:{item}"),
            can_use_game_master_blocks: false,
        };
        self.f
            .behaviours
            .place_block(&mut self.level, &mut self.ticks, &ctx)
    }
    fn use_block(&mut self, pos: BlockPos) -> InteractionResult {
        let player = PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: true,
            game_master: false,
        };
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z),
        );
        self.f.behaviours.use_without_item(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
        )
    }
    /// Right-click the north face of `pos` with `item` (or an empty hand),
    /// and which of `useItemOn` and `useWithoutItem` answered.
    fn use_branch(&mut self, pos: BlockPos, item: Option<&str>) -> (InteractionResult, UseBranch) {
        let player = PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: false,
            game_master: false,
        };
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z),
        );
        self.f.behaviours.use_item_on_branch(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
            item,
        )
    }
    /// Right-click the north face of `pos` holding `item` in the main hand.
    fn use_item(&mut self, pos: BlockPos, item: &str) -> InteractionResult {
        self.use_item_on(pos, Some(item), 0.5)
    }
    /// Right-click the north face of `pos` with an empty main hand.
    fn use_empty_hand(&mut self, pos: BlockPos) -> InteractionResult {
        self.use_item_on(pos, None, 0.5)
    }
    /// Right-click the north face of `pos` with an empty main hand, hitting
    /// `dy` blocks above the block's floor.
    fn use_empty_hand_at(&mut self, pos: BlockPos, dy: f64) -> InteractionResult {
        self.use_item_on(pos, None, dy)
    }
    fn use_item_on(&mut self, pos: BlockPos, item: Option<&str>, dy: f64) -> InteractionResult {
        let player = PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: true,
            game_master: false,
        };
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + dy,
            f64::from(pos.z),
        );
        self.f.behaviours.use_item_on(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
            item,
        )
    }
}

#[test]
fn use_on_a_block_says_which_branch_answered() {
    // ServerPlayerGameMode.useItemOn: ITEM_USED_ON_BLOCK when useItemOn
    // consumes the click, DEFAULT_BLOCK_USE when useWithoutItem does.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    // A lever has no useItemOn of its own: it flips with any hand.
    let lever = w.id("lever", &[("face", "floor"), ("facing", "north")]);
    for item in [None, Some("minecraft:stick")] {
        w.put(pos, lever);
        let (result, branch) = w.use_branch(pos, item);
        assert!(result.consumes_action(), "{item:?}");
        assert_eq!(branch, UseBranch::WithoutItem, "{item:?}");
    }
    // RespawnAnchorBlock.useItemOn: glowstone charges it.
    let anchor = w.id("respawn_anchor", &[("charges", "0")]);
    w.put(pos, anchor);
    assert_eq!(
        w.use_branch(pos, Some("minecraft:glowstone")),
        (InteractionResult::Success, UseBranch::ItemOn)
    );
    // CandleBlock.useItemOn: an empty hand snuffs a lit candle.
    let candle = w.id("candle", &[("candles", "1"), ("lit", "true")]);
    w.put(pos, candle);
    assert_eq!(
        w.use_branch(pos, None),
        (InteractionResult::Success, UseBranch::ItemOn)
    );
    // SweetBerryBushBlock.useItemOn: bone meal on a growing bush passes on
    // to the item without trying the empty hand.
    let bush = w.id("sweet_berry_bush", &[("age", "0")]);
    w.put(pos, bush);
    assert_eq!(
        w.use_branch(pos, Some("minecraft:bone_meal")),
        (InteractionResult::Pass, UseBranch::ItemOn)
    );
}

#[test]
fn glowstone_charges_a_respawn_anchor_up_to_four() {
    // RespawnAnchorBlock.useItemOn: glowstone on an anchor below 4 charges
    // charges it and the click ends (SUCCESS), so no glowstone is placed (#3034).
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let empty = w.id("respawn_anchor", &[("charges", "0")]);
    w.put(pos, empty);
    for charges in 1..=4 {
        assert_eq!(
            w.use_item(pos, "minecraft:glowstone"),
            InteractionResult::Success
        );
        assert_eq!(w.prop(pos, "charges"), charges.to_string());
    }
    // A full anchor can't be charged: TRY_WITH_EMPTY_HAND, then useWithoutItem
    // (the respawn point), which leaves the charges alone.
    w.use_item(pos, "minecraft:glowstone");
    assert_eq!(w.prop(pos, "charges"), "4");
    // Anything else on an empty anchor passes, so the item is used instead.
    w.put(pos, empty);
    assert_eq!(w.use_item(pos, "minecraft:stone"), InteractionResult::Pass);
    assert_eq!(w.prop(pos, "charges"), "0");
}

#[test]
fn mining_lights_redstone_ore_until_its_random_tick() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    for ore in ["redstone_ore", "deepslate_redstone_ore"] {
        let unlit = w.id(ore, &[("lit", "false")]);
        let lit = w.id(ore, &[("lit", "true")]);
        w.put(pos, unlit);
        w.b().attack_block(&mut w.level, &mut w.ticks, pos);
        assert_eq!(w.get(pos), lit);
        w.b().attack_block(&mut w.level, &mut w.ticks, pos);
        assert_eq!(w.get(pos), lit);
        w.b().random_tick(&mut w.level, &mut w.ticks, pos);
        assert_eq!(w.get(pos), unlit);
    }
    let stone = w.id("stone", &[]);
    w.put(pos, stone);
    w.b().attack_block(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.get(pos), stone);
}

#[test]
fn placement_orients_blocks() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let ground = BlockPos::new(0, 0, 0);
    // Looking north (yaw 180) at the top of the floor.
    w.place("oak_stairs", ground, Direction::Up, 180.0, 30.0, 0.0);
    assert_eq!(
        w.name(BlockPos::new(0, 1, 0)),
        "oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]"
    );
    w.place(
        "oak_log",
        BlockPos::new(1, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    w.place(
        "oak_log",
        BlockPos::new(1, 1, 0),
        Direction::East,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(2, 1, 0), "axis"), "x");
    // Torch against the log's east face becomes a wall torch facing east.
    w.place(
        "torch",
        BlockPos::new(2, 1, 0),
        Direction::East,
        270.0,
        0.0,
        0.0,
    );
    assert_eq!(w.name(BlockPos::new(3, 1, 0)), "wall_torch[facing=east]");
    // Furnaces face the player; observers face where the player looks.
    w.place(
        "furnace",
        BlockPos::new(-2, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-2, 1, 0), "facing"), "north");
    w.place(
        "observer",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 1, 0), "facing"), "south");
}

#[test]
fn placement_merges_slabs_and_stacks() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let ground = BlockPos::new(0, 0, 0);
    w.place("oak_slab", ground, Direction::Up, 0.0, 60.0, 0.0);
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "type"), "bottom");
    w.place(
        "oak_slab",
        BlockPos::new(0, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.5,
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "type"), "double");
    // Clicking the upper half of a side face places a top slab.
    w.place(
        "oak_slab",
        BlockPos::new(0, 1, 0),
        Direction::East,
        0.0,
        0.0,
        0.25,
    );
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "type"), "top");
    w.place(
        "snow",
        BlockPos::new(3, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.place(
        "snow",
        BlockPos::new(3, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.375,
    );
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "layers"), "2");
    w.place(
        "candle",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.place(
        "candle",
        BlockPos::new(-3, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.4,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 1, 0), "candles"), "2");
}

#[test]
fn doors_beds_and_plants_place_both_halves() {
    let mut w = world!();
    w.floor(0, 6, "grass_block");
    let ground = BlockPos::new(0, 0, 0);
    w.place("oak_door", ground, Direction::Up, 180.0, 30.0, 0.0);
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "half"), "lower");
    assert_eq!(w.prop(BlockPos::new(0, 2, 0), "half"), "upper");
    assert_eq!(
        w.use_block(BlockPos::new(0, 1, 0)),
        InteractionResult::Success
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "open"), "true");
    assert_eq!(
        w.prop(BlockPos::new(0, 2, 0), "open"),
        "true",
        "the upper half follows"
    );
    w.place(
        "red_bed",
        BlockPos::new(3, 0, 0),
        Direction::Up,
        180.0,
        30.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "part"), "foot");
    assert_eq!(w.prop(BlockPos::new(3, 1, -1), "part"), "head");
    w.place(
        "sunflower",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        30.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 2, 0), "half"), "upper");
    // Iron doors cannot be opened by hand.
    w.place(
        "iron_door",
        BlockPos::new(5, 0, 0),
        Direction::Up,
        180.0,
        30.0,
        0.0,
    );
    assert_eq!(w.use_block(BlockPos::new(5, 1, 0)), InteractionResult::Pass);
    // A placement into a solid block fails.
    assert_eq!(
        w.place(
            "stone",
            BlockPos::new(0, 0, 0),
            Direction::West,
            0.0,
            0.0,
            0.0
        ),
        PlaceOutcome::Failed
    );
}

#[test]
fn levers_power_wire_through_interaction() {
    let mut w = world!();
    w.floor(0, 8, "stone");
    w.place(
        "lever",
        BlockPos::new(0, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    for x in 1..=3 {
        w.place(
            "redstone",
            BlockPos::new(x, 0, 0),
            Direction::Up,
            0.0,
            60.0,
            0.0,
        );
    }
    assert_eq!(
        w.use_block(BlockPos::new(0, 1, 0)),
        InteractionResult::Success
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "powered"), "true");
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "power"), "13");
    w.use_block(BlockPos::new(0, 1, 0));
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "power"), "0");
    // Buttons release after their pressed time.
    w.place(
        "stone_button",
        BlockPos::new(5, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.use_block(BlockPos::new(5, 1, 0));
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "powered"), "true");
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "powered"), "false");
}

#[test]
fn pistons_push_and_sticky_pistons_pull() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("sticky_piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    let dirt = w.id("dirt", &[]);
    let air = w.b().air();
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), dirt);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "true");
    assert_eq!(
        w.name(BlockPos::new(1, 1, 0)),
        "piston_head[facing=east,short=false,type=sticky]"
    );
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), dirt);
    assert!(w.ticks.moving_pistons.is_empty());
    w.set(BlockPos::new(-1, 1, 0), air);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "false");
    assert_eq!(
        w.get(BlockPos::new(1, 1, 0)),
        dirt,
        "the sticky piston pulled the block back"
    );
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), air);
}

#[test]
fn refused_spread_keeps_fire_off_a_guarded_block() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    w.set(BlockPos::new(0, 0, 0), w.id("netherrack", &[]));
    let planks = w.id("oak_planks", &[]);
    w.set(BlockPos::new(1, 1, 0), planks);
    w.set(BlockPos::new(-1, 1, 0), planks);
    w.level.guarded.insert(BlockPos::new(1, 1, 0));
    w.set(BlockPos::new(0, 1, 0), w.id("fire", &[]));
    for _ in 0..4000 {
        w.tick();
    }
    assert_eq!(
        w.get(BlockPos::new(1, 1, 0)),
        planks,
        "the guarded planks never burn"
    );
    assert_ne!(
        w.get(BlockPos::new(-1, 1, 0)),
        planks,
        "the other planks burned"
    );
}

#[test]
fn refused_piston_move_leaves_the_piston_retracted() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    let dirt = w.id("dirt", &[]);
    w.level.guarded.insert(BlockPos::new(2, 1, 0));
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), dirt);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "false");
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), dirt);
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), w.b().air());
    assert!(w.ticks.moving_pistons.is_empty());
}

#[test]
fn refused_sticky_pull_keeps_the_block_and_sends_no_event() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("sticky_piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    let dirt = w.id("dirt", &[]);
    let air = w.b().air();
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), dirt);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), dirt);
    w.level.guarded.insert(BlockPos::new(2, 1, 0));
    w.level.events.clear();
    w.set(BlockPos::new(-1, 1, 0), air);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "false");
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), air);
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), dirt, "the pull was refused");
    // Clients would replay a retraction event and pull the block anyway.
    let sent = w.level.events.iter().any(
        |e| matches!(e, LevelEvent::BlockEntityEvent { pos, .. } if *pos == BlockPos::new(0, 1, 0)),
    );
    assert!(!sent, "no retraction event for a refused pull");
}

/// TntBlock.prime returns false on a client: neither the entity, the removal,
/// nor its random push may be predicted while replaying a piston update.
#[test]
fn tnt_duper_3856_client_neighbor_does_not_prime_or_remove_tnt() {
    let mut w = world!();
    let f = w.f;
    w.level.client_side = true;
    let pos = BlockPos::new(0, 1, 0);
    let tnt = w.id("tnt", &[]);
    let power = w.id("redstone_block", &[]);
    w.put(pos, tnt);
    w.put(pos.offset(1, 0, 0), power);
    let mut random = w.level.random.clone();
    f.behaviours.handle_neighbor_changed(
        &mut w.level,
        &mut w.ticks,
        pos,
        f.behaviours.block(power),
    );
    assert_eq!(w.get(pos), tnt, "client must wait for server block changes");
    assert!(
        w.level.events.is_empty(),
        "client must not emit TNT effects"
    );
    assert_eq!(w.level.random.next_long(), random.next_long());
}

#[test]
fn tnt_duper_3856_client_placement_does_not_prime_tnt() {
    let mut w = world!();
    w.level.client_side = true;
    let pos = BlockPos::new(0, 1, 0);
    let tnt = w.id("tnt", &[]);
    let power = w.id("redstone_block", &[]);
    w.put(pos.offset(1, 0, 0), power);
    w.set(pos, tnt);
    assert_eq!(w.get(pos), tnt);
    assert!(w.level.events.is_empty());
}

#[test]
fn tnt_duper_3856_server_still_primes_each_powered_tnt() {
    let mut w = world!();
    let tnt = w.id("tnt", &[]);
    let power = w.id("redstone_block", &[]);
    for pos in [BlockPos::new(0, 1, 0), BlockPos::new(0, 1, 3)] {
        w.put(pos.offset(1, 0, 0), power);
        w.set(pos, tnt);
        assert_eq!(w.get(pos), w.b().air());
        assert_eq!(
            w.level
                .events
                .iter()
                .filter(|e| matches!(e, LevelEvent::PrimeTnt { pos: p, .. } if *p == pos))
                .count(),
            1,
        );
    }
}

/// Breaking a wall torch next to a BUD-powered TNT updates the old block while
/// the piston already holds its saved state. This vanilla duplication must
/// continue to work on the server, without a second TNT from the client.
#[test]
fn tnt_duper_3856_piston_keeps_original_and_primes_one_tnt() {
    let Some(f) = fixture() else {
        return;
    };
    for client in [false, true] {
        let mut w = World::new(f);
        w.level.client_side = client;
        let base = BlockPos::new(0, 2, 0);
        let piston = w.id("sticky_piston", &[("facing", "east")]);
        let slime = w.id("slime_block", &[]);
        let tnt = w.id("tnt", &[]);
        let torch = w.id("wall_torch", &[("facing", "north")]);
        let power = w.id("redstone_block", &[]);
        w.put(base, piston);
        w.put(base.offset(1, 0, 0), slime);
        w.put(base.offset(2, 0, 0), slime);
        w.put(base.offset(2, -1, 0), slime);
        w.put(base.offset(2, -1, -1), tnt);
        w.put(base.offset(2, 0, -1), torch);
        // Raw writes model a powered TNT that has not received an update yet.
        w.put(base.offset(2, -1, -2), power);
        w.put(base.offset(-1, 0, 0), power);
        if client {
            assert!(f.behaviours.client_block_event(
                &mut w.level,
                &mut w.ticks,
                base,
                0,
                Direction::East as i32
            ));
        } else {
            f.behaviours.handle_neighbor_changed(
                &mut w.level,
                &mut w.ticks,
                base,
                f.behaviours.block(power),
            );
            f.behaviours.run_block_events(&mut w.level, &mut w.ticks);
        }
        let primed = w
            .level
            .events
            .iter()
            .filter(|e| matches!(e, LevelEvent::PrimeTnt { .. }))
            .count();
        assert_eq!(primed, usize::from(!client), "client={client}");
        for _ in 0..10 {
            w.level.time += 1;
            if client {
                f.behaviours
                    .client_tick_block_entities(&mut w.level, &mut w.ticks);
            } else {
                f.behaviours.tick_block_entities(&mut w.level, &mut w.ticks);
            }
        }
        assert_eq!(
            w.get(base.offset(3, -1, -1)),
            tnt,
            "original must survive, client={client}"
        );
        assert_eq!(
            w.level
                .events
                .iter()
                .filter(|e| matches!(e, LevelEvent::PrimeTnt { .. }))
                .count(),
            usize::from(!client)
        );
    }
}

/// `ServerLevel.runBlockEvents` reports a handled event for the clients
/// (`ClientboundBlockEventPacket`), carrying the block the event named.
#[test]
fn a_handled_block_event_reaches_the_clients() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(-1, 1, 0), block);
    w.level.events.clear();
    w.full_tick();
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::BlockEntityEvent {
            pos,
            state,
            a: 0,
            ..
        } if *pos == BlockPos::new(0, 1, 0) && *state == piston
    )));
}

/// `ClientLevel.blockEvent`: the client re-runs `triggerEvent` on its own
/// level, which builds the moving block entities a piston's `moveBlocks`
/// makes, and ticks them to the placed block.
#[test]
fn a_client_replays_a_pistons_block_event() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("sticky_piston", &[("facing", "east")]);
    let dirt = w.id("dirt", &[]);
    let block = w.id("redstone_block", &[]);
    w.put(BlockPos::new(0, 1, 0), piston);
    w.put(BlockPos::new(1, 1, 0), dirt);
    w.put(BlockPos::new(-1, 1, 0), block);
    w.f.behaviours.client_block_event(
        &mut w.level,
        &mut w.ticks,
        BlockPos::new(0, 1, 0),
        0,
        Direction::East as i32,
    );
    assert!(
        w.ticks
            .moving_pistons
            .iter()
            .any(|(p, _)| *p == BlockPos::new(2, 1, 0))
    );
    for _ in 0..3 {
        w.level.time += 1;
        w.f.behaviours
            .client_tick_block_entities(&mut w.level, &mut w.ticks);
    }
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), dirt);
    assert_eq!(
        w.name(BlockPos::new(1, 1, 0)),
        "piston_head[facing=east,short=false,type=sticky]"
    );
    assert!(w.ticks.moving_pistons.is_empty());
}

/// A client's copy of a move doesn't start the pistons the moved block
/// powers (`checkIfExtend` is server-only): their own block events come from
/// the server.
#[test]
fn a_client_replay_starts_no_other_piston() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    w.level.client_side = true;
    let piston = w.id("piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    let up = w.id("piston", &[("facing", "up")]);
    w.put(BlockPos::new(0, 1, 0), piston);
    w.put(BlockPos::new(1, 1, 0), block);
    w.put(BlockPos::new(2, 2, 0), up);
    w.f.behaviours.client_block_event(
        &mut w.level,
        &mut w.ticks,
        BlockPos::new(0, 1, 0),
        0,
        Direction::East as i32,
    );
    for _ in 0..10 {
        w.level.time += 1;
        w.f.behaviours
            .client_tick_block_entities(&mut w.level, &mut w.ticks);
    }
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), block);
    assert_eq!(w.get(BlockPos::new(2, 2, 0)), up);
    assert!(w.ticks.moving_pistons.is_empty());
    assert!(w.ticks.block_events.is_empty());
}

#[test]
fn pistons_pop_fragile_blocks_and_refuse_obsidian() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    let torch = w.id("torch", &[]);
    let obsidian = w.id("obsidian", &[]);
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), torch);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(
        w.name(BlockPos::new(1, 1, 0)),
        "piston_head[facing=east,short=false,type=normal]"
    );
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::DropResources { state, .. } if *state == torch))
    );
    let mut w = world!();
    w.floor(0, 10, "stone");
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), obsidian);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "false");
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), obsidian);
}

/// `MovingPistonBlock.getDrops`: a broken moving piston drops the block it is
/// moving, and drops nothing at all once its moving-piston block entity is
/// gone.
#[test]
fn a_broken_moving_piston_drops_the_block_it_carries() {
    let mut w = world!();
    let moving = w.id("moving_piston", &[("facing", "east"), ("type", "normal")]);
    let sand = w.id("sand", &[]);
    let carried = BlockPos::new(0, 1, 0);
    let orphan = BlockPos::new(5, 1, 5);
    for pos in [carried, orphan] {
        w.put(pos, moving);
    }
    w.ticks.moving_pistons.push((
        carried,
        MovingPiston {
            moved_state: sand,
            direction: Direction::East,
            extending: true,
            source: false,
            progress: 0.5,
            progress_o: 0.4,
            last_ticked: 0,
            death_ticks: 0,
            id: 0,
        },
    ));
    w.level.events.clear();
    for pos in [carried, orphan] {
        w.b().destroy_block(&mut w.level, &mut w.ticks, pos, true);
    }
    let drops: Vec<(BlockPos, BlockStateId)> = w
        .level
        .events
        .iter()
        .filter_map(|e| match e {
            LevelEvent::DropResources { pos, state } => Some((*pos, *state)),
            _ => None,
        })
        .collect();
    assert_eq!(
        drops,
        vec![(carried, sand)],
        "the moving piston drops the sand it carries; without its block entity there is nothing to drop"
    );
}

#[test]
fn dripstone_fills_cauldrons() {
    let mut w = world!();
    let block = w.id("dripstone_block", &[]);
    let water = w.id("water", &[]);
    let tip = w.id(
        "pointed_dripstone",
        &[("vertical_direction", "down"), ("thickness", "tip")],
    );
    let cauldron = w.id("cauldron", &[]);
    w.put(BlockPos::new(0, 11, 0), water);
    w.put(BlockPos::new(0, 10, 0), block);
    w.put(BlockPos::new(0, 9, 0), tip);
    w.put(BlockPos::new(0, 5, 0), cauldron);
    for _ in 0..100 {
        w.b()
            .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(0, 9, 0));
    }
    for _ in 0..80 {
        w.tick();
    }
    assert_eq!(w.name(BlockPos::new(0, 5, 0)), "water_cauldron[level=1]");
}

#[test]
fn a_level_can_answer_for_a_blocks_signal() {
    let mut w = world!();
    let source = BlockPos::new(0, 1, 0);
    let beside = BlockPos::new(-1, 1, 0);
    let above = BlockPos::new(0, 2, 0);
    let conductor = BlockPos::new(1, 1, 0);
    let past_it = BlockPos::new(2, 1, 0);
    let gold = w.id("gold_block", &[]);
    let stone = w.id("stone", &[]);
    w.set(source, gold);
    w.set(conductor, stone);
    let read = |w: &mut World, pos| {
        w.f.behaviours
            .best_neighbor_signal(&mut w.level, &mut w.ticks, pos)
    };
    assert_eq!(read(&mut w, beside), 0, "gold gives nothing by itself");

    // A weak signal reaches what touches it, and stops at a conductor.
    w.level.signals.insert(source, (7, 0, None));
    assert_eq!(read(&mut w, beside), 7);
    assert_eq!(read(&mut w, past_it), 0);

    // A direct signal passes through the conductor.
    w.level.signals.insert(source, (7, 15, None));
    assert_eq!(read(&mut w, past_it), 15);

    // One side only: `Down` is the direction read by the block above.
    w.level
        .signals
        .insert(source, (9, 0, Some(Direction::Down)));
    assert_eq!(read(&mut w, above), 9);
    assert_eq!(read(&mut w, beside), 0);

    // Vanilla's sources go through the same answer untouched.
    w.level.signals.clear();
    let redstone = w.id("redstone_block", &[]);
    w.set(source, redstone);
    assert_eq!(read(&mut w, beside), 15);
}

#[test]
fn a_level_can_answer_for_a_blocks_comparator_output() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let block = BlockPos::new(0, 1, 0);
    let comparator = BlockPos::new(1, 1, 0);
    let after = BlockPos::new(2, 1, 0);
    // A comparator facing west reads the block west of it.
    let facing_west = w.id("comparator", &[("facing", "west")]);
    w.set(comparator, facing_west);
    let read = |w: &mut World| {
        for _ in 0..4 {
            w.full_tick();
        }
        w.f.behaviours
            .best_neighbor_signal(&mut w.level, &mut w.ticks, after)
    };
    let gold = w.id("gold_block", &[]);
    w.level.analog.insert(block, 9);
    w.set(block, gold);
    assert_eq!(read(&mut w), 0, "gold has no output of its own");

    // Placing a block that says it has one tells the comparator, as Level.setBlock does.
    w.level.analog_states.insert(gold);
    w.set(block, w.id("stone", &[]));
    w.set(block, gold);
    assert_eq!(read(&mut w), 9);

    // An output that changes without the block: updateNeighbourForOutputSignal.
    w.level.analog.insert(block, 4);
    assert_eq!(read(&mut w), 9, "nothing told the comparator yet");
    w.f.behaviours
        .update_neighbour_for_output_signal(&mut w.level, &mut w.ticks, block);
    assert_eq!(read(&mut w), 4);

    // Behind a conductor, and out of range answers are clamped.
    let further = BlockPos::new(-1, 1, 0);
    w.set(block, w.id("stone", &[]));
    w.level.analog.insert(further, 40);
    w.set(further, gold);
    assert_eq!(read(&mut w), 15);
}

#[test]
fn sculk_sensor_phases_and_signal() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let pos = BlockPos::new(0, 1, 0);
    let sensor = w.id("sculk_sensor", &[]);
    w.set(pos, sensor);
    assert!(w.b().sculk_sensor_can_activate(w.get(pos)));
    w.f.behaviours
        .sculk_sensor_activate(&mut w.level, &mut w.ticks, pos, 11);
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "active");
    assert_eq!(w.prop(pos, "power"), "11");
    // The sensor powers the block below it from the top.
    let below = BlockPos::new(0, 0, 0);
    assert_eq!(
        w.f.behaviours
            .best_neighbor_signal(&mut w.level, &mut w.ticks, below),
        11,
        "the block under an active sensor receives its power"
    );
    for _ in 0..29 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "active");
    w.tick();
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "cooldown");
    assert_eq!(w.prop(pos, "power"), "0");
    for _ in 0..10 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "inactive");
}

#[test]
fn calibrated_sensor_is_active_for_ten_ticks() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let pos = BlockPos::new(0, 1, 0);
    let sensor = w.id("calibrated_sculk_sensor", &[("facing", "north")]);
    w.set(pos, sensor);
    w.f.behaviours
        .sculk_sensor_activate(&mut w.level, &mut w.ticks, pos, 4);
    for _ in 0..9 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "active");
    w.tick();
    assert_eq!(w.prop(pos, "sculk_sensor_phase"), "cooldown");
    // The block behind (south) feeds the calibration signal.
    let source = w.id("redstone_block", &[]);
    w.set(BlockPos::new(0, 1, 1), source);
    assert_eq!(
        w.f.behaviours
            .calibrated_back_signal(&mut w.level, &mut w.ticks, pos),
        15
    );
}

#[test]
fn sculk_charge_spreads_veins_and_sculk() {
    let mut w = world!();
    w.floor(0, 12, "stone");
    w.floor(-1, 12, "stone");
    let origin = BlockPos::new(0, 1, 0);
    let catalyst = w.id("sculk_catalyst", &[]);
    w.put(origin, catalyst);
    let mut spreader = sculk::SculkSpreader::default();
    spreader.add_cursors(BlockPos::new(2, 1, 0), 40);
    assert_eq!(spreader.cursors.len(), 1);
    for _ in 0..80 {
        w.f.behaviours.sculk_update_cursors(
            &mut w.level,
            &mut w.ticks,
            &mut spreader,
            origin,
            true,
        );
    }
    let sculk = w.id("sculk", &[]);
    let found = (-12..=12)
        .flat_map(|x| (-12..=12).map(move |z| BlockPos::new(x, 0, z)))
        .filter(|p| w.get(*p) == sculk)
        .count();
    assert!(found > 0, "the charge turned stone into sculk");
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::LevelEvent { id: 3006, .. })),
        "charge particles are reported"
    );
}

#[test]
fn shrieker_responds_when_the_shriek_ends() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let pos = BlockPos::new(0, 1, 0);
    let shrieker = w.id("sculk_shrieker", &[("can_summon", "true")]);
    w.set(pos, shrieker);
    w.f.behaviours
        .sculk_shrieker_shriek(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.prop(pos, "shrieking"), "true");
    for _ in 0..89 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "shrieking"), "true");
    w.tick();
    assert_eq!(w.prop(pos, "shrieking"), "false");
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::BlockEvent {
            kind: "shrieker_respond",
            ..
        }
    )));
}

#[test]
fn catalyst_bloom_lasts_eight_ticks() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let catalyst = w.id("sculk_catalyst", &[]);
    w.set(pos, catalyst);
    w.f.behaviours
        .sculk_catalyst_bloom(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.prop(pos, "bloom"), "true");
    for _ in 0..7 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "bloom"), "true");
    w.tick();
    assert_eq!(w.prop(pos, "bloom"), "false");
}

#[test]
fn placing_and_breaking_report_the_affected_state() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let wool = w.id("white_wool", &[]);
    let placed = w.place(
        "white_wool",
        BlockPos::new(0, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    assert!(matches!(placed, PlaceOutcome::Placed { .. }));
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::GameEvent { event: "minecraft:block_place", state: Some(s), .. } if *s == wool
    )));
    w.level.events.clear();
    w.f.behaviours
        .destroy_block(&mut w.level, &mut w.ticks, BlockPos::new(0, 1, 0), false);
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::GameEvent { event: "minecraft:block_destroy", state: Some(s), .. } if *s == wool
    )));
}

#[test]
fn dispenser_buckets_pick_up_and_empty() {
    let mut w = world!();
    let at = BlockPos::new(0, 10, 0);
    let water = w.id("water", &[("level", "0")]);
    w.put(at, water);
    let picked = w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false);
    assert_eq!(picked, Some("minecraft:water_bucket"));
    assert_eq!(w.get(at), w.b().air());
    // Flowing water gives nothing.
    let flowing = w.id("water", &[("level", "3")]);
    w.put(at, flowing);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        None
    );
    // A waterlogged slab is drained and stays a slab.
    let slab = w.id("oak_slab", &[("waterlogged", "true")]);
    w.put(at, slab);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        Some("minecraft:water_bucket")
    );
    assert_eq!(w.prop(at, "waterlogged"), "false");
    // Powder snow comes up as its bucket.
    let snow = w.id("powder_snow", &[]);
    w.put(at, snow);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        Some("minecraft:powder_snow_bucket")
    );
    // A barrier only a creative player fills or drains.
    let barrier = w.id("barrier", &[]);
    w.put(at, barrier);
    let sound = "minecraft:item.bucket.empty";
    let content = BucketContent::Fluid(crate::fluid::FluidKind::Water);
    assert!(
        !w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound, false, false)
    );
    assert_eq!(w.prop(at, "waterlogged"), "false");
    assert!(
        w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound, false, true)
    );
    assert_eq!(w.prop(at, "waterlogged"), "true");
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, false),
        None
    );
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at, true),
        Some("minecraft:water_bucket")
    );
    assert_eq!(w.prop(at, "waterlogged"), "false");
    // Emptying water into air makes a source; stone refuses.
    let air = w.b().air();
    w.put(at, air);
    assert!(
        w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound, false, false)
    );
    assert_eq!(w.name(at), "water[level=0]");
    let stone = w.id("stone", &[]);
    w.put(at, stone);
    assert!(
        !w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound, false, false)
    );
    // Water fills a waterloggable slab instead of replacing it.
    let dry = w.id("oak_slab", &[]);
    w.put(at, dry);
    assert!(
        w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound, false, false)
    );
    assert_eq!(w.prop(at, "waterlogged"), "true");
    // Sneak-clicking the slab refuses: only air takes a fluid under Shift.
    let dry = w.id("oak_slab", &[]);
    w.put(at, dry);
    assert!(
        !w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound, true, false)
    );
    assert_eq!(w.prop(at, "waterlogged"), "false");
    // Powder snow only goes into empty space.
    let air = w.b().air();
    w.put(at, air);
    assert!(w.b().bucket_empty(
        &mut w.level,
        &mut w.ticks,
        at,
        BucketContent::Powder,
        "minecraft:item.bucket.empty_powder_snow",
        false,
        false
    ));
    assert_eq!(w.name(at), "powder_snow[]");
}

#[test]
fn creaking_heart_follows_its_logs_and_the_night() {
    let mut w = world!();
    let heart_at = BlockPos::new(0, 10, 0);
    let log = |w: &World, axis: &str| w.id("pale_oak_log", &[("axis", axis)]);
    let heart = w.id(
        "creaking_heart",
        &[("axis", "y"), ("creaking_heart_state", "uprooted")],
    );
    let (below, above) = (BlockPos::new(0, 9, 0), BlockPos::new(0, 11, 0));
    w.put(heart_at, heart);
    w.put(below, log(&w, "y"));
    // A log along another axis does not count.
    w.put(above, log(&w, "x"));
    assert!(
        !w.b()
            .creaking_heart_has_logs(&mut w.level, &mut w.ticks, heart_at, heart)
    );
    // Day: a log update wakes the uprooted heart to dormant.
    let up = log(&w, "y");
    w.set(above, up);
    for _ in 0..3 {
        w.tick();
    }
    assert_eq!(w.prop(heart_at, "creaking_heart_state"), "dormant");
    // Night: the same trunk wakes a fresh heart up.
    w.level.day_time = 14000;
    w.put(heart_at, heart);
    w.put(above, w.b().air());
    w.set(above, up);
    for _ in 0..3 {
        w.tick();
    }
    assert_eq!(w.prop(heart_at, "creaking_heart_state"), "awake");
    // Placed without logs it stays uprooted.
    w.put(above, w.b().air());
    w.put(heart_at, heart);
    let air = w.b().air();
    w.set(above, air);
    for _ in 0..3 {
        w.tick();
    }
    assert_eq!(w.prop(heart_at, "creaking_heart_state"), "uprooted");
}

#[test]
fn player_breaks_fire_with_extinguish_not_generic_break_event() {
    for name in ["fire", "soul_fire", "white_wool"] {
        let mut w = world!();
        let pos = BlockPos::new(0, 1, 0);
        let support = w.id("soul_soil", &[]);
        w.put(BlockPos::new(0, 0, 0), support);
        let state = w.id(name, &[]);
        w.put(pos, state);
        w.level.events.clear();
        assert!(
            w.f.behaviours
                .player_destroy_block(&mut w.level, &mut w.ticks, pos, false, true)
        );
        let is_fire = name != "white_wool";
        assert_eq!(
            w.level
                .events
                .iter()
                .any(|e| matches!(e,LevelEvent::LevelEvent {id:1009,pos:p,..} if *p==pos)),
            is_fire,
            "extinguish for {name}"
        );
        assert_eq!(
            w.level
                .events
                .iter()
                .any(|e| matches!(e,LevelEvent::LevelEvent {id:2001,pos:p,..} if *p==pos)),
            !is_fire,
            "generic break for {name}"
        );
        assert_eq!(w.get(pos), w.b().air());
    }
}

#[test]
fn big_dripleaf_tilts_under_an_entity_standing_on_it() {
    let mut w = world!();
    w.floor(0, 2, "clay");
    let pos = BlockPos::new(0, 1, 0);
    let leaf = w.id("big_dripleaf", &[("facing", "north")]);
    w.put(pos, leaf);
    let stand = |w: &mut World, on_ground: bool, feet: f64| {
        w.f.behaviours
            .entity_on_big_dripleaf(&mut w.level, &mut w.ticks, pos, on_ground, feet);
    };
    // canEntityTilt: in the air, or with its feet at or under 0.6875, nothing.
    stand(&mut w, false, 1.9375);
    stand(&mut w, true, 1.6875);
    assert_eq!(w.prop(pos, "tilt"), "none");
    // Standing on the leaf's top (15/16): UNSTABLE, then PARTIAL and FULL
    // 10 ticks apart, and back to NONE 100 ticks later.
    stand(&mut w, true, 1.9375);
    assert_eq!(w.prop(pos, "tilt"), "unstable");
    for _ in 0..10 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "tilt"), "partial");
    for _ in 0..10 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "tilt"), "full");
    // A tilted leaf doesn't restart under the entity.
    stand(&mut w, true, 1.9375);
    assert_eq!(w.prop(pos, "tilt"), "full");
    for _ in 0..100 {
        w.tick();
    }
    assert_eq!(w.prop(pos, "tilt"), "none");
    // A powered leaf doesn't tilt.
    let power = w.id("redstone_block", &[]);
    w.put(BlockPos::new(1, 1, 0), power);
    stand(&mut w, true, 1.9375);
    assert_eq!(w.prop(pos, "tilt"), "none");
}

#[test]
fn using_storage_and_interactable_blocks_succeeds() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("chest", &[("facing", "north"), ("type", "single")]),
        ("trapped_chest", &[("facing", "north"), ("type", "single")]),
        ("barrel", &[("facing", "up")]),
        ("ender_chest", &[("facing", "north")]),
        ("shulker_box", &[("facing", "up")]),
        ("dispenser", &[("facing", "north")]),
        ("dropper", &[("facing", "north")]),
        ("hopper", &[("facing", "down")]),
        ("crafter", &[("orientation", "north_up")]),
        ("furnace", &[("facing", "north")]),
        ("blast_furnace", &[("facing", "north")]),
        ("smoker", &[("facing", "north")]),
        ("brewing_stand", &[]),
        ("beacon", &[]),
        ("decorated_pot", &[]),
        ("jukebox", &[("has_record", "true")]),
    ];
    for &(name, props) in cases {
        w.put(pos, w.id(name, props));
        assert_eq!(
            w.use_block(pos),
            InteractionResult::Success,
            "{name} should succeed (client swing)"
        );
    }
    w.put(pos, w.id("jukebox", &[("has_record", "false")]));
    assert_eq!(
        w.use_block(pos),
        InteractionResult::Pass,
        "an empty jukebox passes"
    );
}

#[test]
fn creative_breaking_piston_head_does_not_drop_piston() {
    for (piston_name, head_type) in [("piston", "normal"), ("sticky_piston", "sticky")] {
        let mut w = world!();
        let base_pos = BlockPos::new(0, 1, 0);
        let head_pos = BlockPos::new(1, 1, 0);
        let base = w.id(piston_name, &[("extended", "true"), ("facing", "east")]);
        let head = w.id(
            "piston_head",
            &[("type", head_type), ("facing", "east"), ("short", "false")],
        );
        w.put(base_pos, base);
        w.put(head_pos, head);
        w.level.events.clear();

        // Breaking the head in creative mode. Destroying the base first makes the
        // head's shape update remove it, so its own removeBlock finds air.
        w.f.behaviours
            .player_destroy_block(&mut w.level, &mut w.ticks, head_pos, true, true);

        // Both head and base must be removed:
        assert_eq!(w.get(head_pos), w.b().air(), "head should become air");
        assert_eq!(w.get(base_pos), w.b().air(), "base should become air");

        // Must NOT drop the base: the head's own removal (its shape update) drops
        // its empty loot, as vanilla's updateOrDestroy does.
        assert!(
            !w.level
                .events
                .iter()
                .any(|e| matches!(e, LevelEvent::DropResources { pos, .. } if *pos == base_pos)),
            "creative mode breaking {head_type} piston head must not drop the base piston"
        );
    }
}

#[test]
fn survival_breaking_piston_head_drops_piston() {
    for (piston_name, head_type) in [("piston", "normal"), ("sticky_piston", "sticky")] {
        let mut w = world!();
        let base_pos = BlockPos::new(0, 1, 0);
        let head_pos = BlockPos::new(1, 1, 0);
        let base = w.id(piston_name, &[("extended", "true"), ("facing", "east")]);
        let head = w.id(
            "piston_head",
            &[("type", head_type), ("facing", "east"), ("short", "false")],
        );
        w.put(base_pos, base);
        w.put(head_pos, head);
        w.level.events.clear();

        // Breaking the head in survival mode:
        assert!(
            w.f.behaviours
                .player_destroy_block(&mut w.level, &mut w.ticks, head_pos, false, true),
            "head should be destroyed"
        );

        // Both head and base must be removed:
        assert_eq!(w.get(head_pos), w.b().air(), "head should become air");
        assert_eq!(w.get(base_pos), w.b().air(), "base should become air");

        // Must drop the base piston at base_pos:
        assert!(
            w.level
                .events
                .iter()
                .any(|e| matches!(e, LevelEvent::DropResources { pos, .. } if *pos == base_pos)),
            "survival mode breaking {head_type} piston head must drop base piston"
        );
    }
}

/// A lever on the back of a piston stays through extend and retract.
#[test]
fn lever_on_piston_back_survives_extend_and_retract() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    for kind in ["piston", "sticky_piston"] {
        let piston = w.id(kind, &[("facing", "east")]);
        let lever = w.id("lever", &[("face", "wall"), ("facing", "west")]);
        w.set(BlockPos::new(0, 1, 0), piston);
        w.set(BlockPos::new(-1, 1, 0), lever);
        let on = w.b().table.set_bool(lever, w.b().props.powered, true);
        w.set(BlockPos::new(-1, 1, 0), on);
        for _ in 0..4 {
            w.full_tick();
        }
        assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "true", "{kind}");
        assert_eq!(
            w.name(BlockPos::new(-1, 1, 0)),
            "lever[face=wall,facing=west,powered=true]",
            "{kind}"
        );
        w.set(BlockPos::new(-1, 1, 0), lever);
        for _ in 0..4 {
            w.full_tick();
        }
        assert_eq!(
            w.prop(BlockPos::new(0, 1, 0), "extended"),
            "false",
            "{kind}"
        );
        assert_eq!(
            w.name(BlockPos::new(-1, 1, 0)),
            "lever[face=wall,facing=west,powered=false]",
            "{kind}"
        );
    }
}

#[test]
fn dust_on_a_downward_piston_survives_extend_and_retract() {
    // PistonBaseBlock.triggerEvent writes the retracting base as a
    // `moving_piston` and still updates the neighbours around it, so the wire
    // above the base is asked whether it can survive. Vanilla reads that block
    // through its block entity, whose collision shape holds the extended base;
    // the exported support facts see no block entity and call every face
    // unsupported, which broke the dust (#4728).
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("piston", &[("facing", "down")]);
    let power = w.id("redstone_block", &[]);
    let wire = w.id("redstone_wire", &[]);
    let air = w.b().air();
    let base = BlockPos::new(0, 3, 0);
    let dust = BlockPos::new(0, 4, 0);
    w.set(base, piston);
    w.set(dust, wire);
    w.set(BlockPos::new(1, 3, 0), power);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(base, "extended"), "true");
    assert!(
        w.name(BlockPos::new(0, 2, 0)).starts_with("piston_head"),
        "the piston pushed its head down"
    );
    assert!(
        w.name(dust).starts_with("redstone_wire"),
        "the dust stays on the extended piston, got {}",
        w.name(dust)
    );
    w.set(BlockPos::new(1, 3, 0), air);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(base, "extended"), "false");
    assert!(
        w.name(dust).starts_with("redstone_wire"),
        "the dust stays through the retract, got {}",
        w.name(dust)
    );
}

#[test]
fn a_candle_on_an_uneaten_cake_makes_a_candle_cake() {
    // CakeBlock.useItemOn: any candle on a cake with no bites turns it into the
    // matching unlit candle cake; a bitten cake does not.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let cake = w.id("cake", &[("bites", "0")]);
    for (item, block) in [
        ("minecraft:candle", "candle_cake"),
        ("minecraft:red_candle", "red_candle_cake"),
    ] {
        w.put(pos, cake);
        assert_eq!(w.use_item(pos, item), InteractionResult::Success);
        assert_eq!(w.get(pos), w.id(block, &[("lit", "false")]));
    }
    let bitten = w.id("cake", &[("bites", "1")]);
    w.put(pos, bitten);
    w.use_item(pos, "minecraft:candle");
    assert_ne!(w.get(pos), w.id("candle_cake", &[("lit", "false")]));
    w.put(pos, cake);
    // Anything else answers TRY_WITH_EMPTY_HAND, and ServerPlayerGameMode
    // then calls useWithoutItem with whatever the main hand holds, so a stone
    // eats a slice of the cake instead of adding a candle to it.
    assert_eq!(
        w.use_item(pos, "minecraft:stone"),
        InteractionResult::Success
    );
    assert_eq!(w.get(pos), bitten);
}

#[test]
fn an_empty_hand_snuffs_a_lit_candle() {
    // CandleBlock.useItemOn: an empty hand on a lit candle (`mayBuild`) runs
    // AbstractCandleBlock.extinguish: unlit, the wick smoke, the extinguish
    // sound, the block-change game event, and nothing drops.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let lit = w.id("candle", &[("lit", "true")]);
    let unlit = w.id("candle", &[("lit", "false")]);
    w.put(pos, lit);
    w.level.events.clear();
    assert_eq!(w.use_empty_hand(pos), InteractionResult::Success);
    assert_eq!(w.get(pos), unlit);
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::Sound { sound, .. } if &**sound == "minecraft:block.candle.extinguish"
        )),
        "the candle extinguish sound"
    );
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::BlockEvent {
                kind: "candle_smoke",
                ..
            }
        )),
        "the wick smoke"
    );
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::GameEvent {
                event: "minecraft:block_change",
                ..
            }
        )),
        "the block-change game event"
    );
    assert!(
        !w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::DropResources { .. })),
        "snuffing drops nothing"
    );
    // A held item, or a candle that is already out, leaves it alone.
    w.put(pos, lit);
    assert_eq!(w.use_item(pos, "minecraft:stick"), InteractionResult::Pass);
    assert_eq!(w.get(pos), lit);
    w.put(pos, unlit);
    assert_eq!(w.use_empty_hand(pos), InteractionResult::Pass);
}

#[test]
fn an_empty_hand_on_a_lit_candle_cake_eats_it_or_snuffs_its_candle() {
    // CandleCakeBlock.useWithoutItem below the candle eats the cake body
    // (CakeBlock.eat on a plain cake), so one bite is gone and the candle with
    // it; on the candle above half a block, CandleCakeBlock.useItemOn snuffs it
    // (`candleHit`) and the cake stays; flint and steel passes, so the cake is
    // not eaten.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let lit = w.id("candle_cake", &[("lit", "true")]);
    w.put(pos, lit);
    assert_eq!(w.use_empty_hand(pos), InteractionResult::Success);
    assert_eq!(w.get(pos), w.id("cake", &[("bites", "1")]));
    w.put(pos, lit);
    w.level.events.clear();
    assert_eq!(w.use_empty_hand_at(pos, 0.75), InteractionResult::Success);
    assert_eq!(w.get(pos), w.id("candle_cake", &[("lit", "false")]));
    assert!(
        w.level.events.iter().any(|e| matches!(
            e,
            LevelEvent::Sound { sound, .. } if &**sound == "minecraft:block.candle.extinguish"
        )),
        "the candle extinguish sound"
    );
    w.put(pos, lit);
    assert_eq!(
        w.use_item(pos, "minecraft:flint_and_steel"),
        InteractionResult::Pass
    );
    assert_eq!(w.get(pos), lit);
}

#[test]
fn flowers_and_saplings_can_be_placed_in_flower_pots() {
    // FlowerPotBlock.useItemOn: placing a plant in an empty pot pots it.
    // Clicking a potted plant with another plant CONSUMEs without change.
    // Clicking a potted plant with an empty hand takes the plant out (useWithoutItem).
    // Clicking an empty pot with empty hand or non-plant item CONSUMEs.
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let empty_pot = w.id("flower_pot", &[]);
    for (item, potted_block) in [
        ("minecraft:poppy", "potted_poppy"),
        ("minecraft:dandelion", "potted_dandelion"),
        ("minecraft:oak_sapling", "potted_oak_sapling"),
        ("minecraft:cactus", "potted_cactus"),
        ("minecraft:red_mushroom", "potted_red_mushroom"),
        ("minecraft:fern", "potted_fern"),
    ] {
        w.put(pos, empty_pot);
        assert_eq!(w.use_item(pos, item), InteractionResult::Success);
        assert_eq!(w.get(pos), w.id(potted_block, &[]));

        // Right-clicking a potted plant with another flower does nothing (CONSUME).
        assert_eq!(
            w.use_item(pos, "minecraft:poppy"),
            InteractionResult::Consume
        );
        assert_eq!(w.get(pos), w.id(potted_block, &[]));

        // Right-clicking a potted plant with empty hand extracts the plant.
        assert_eq!(w.use_block(pos), InteractionResult::Success);
        assert_eq!(w.get(pos), empty_pot);
    }

    // Right-clicking an empty flower pot with an empty hand does nothing (CONSUME, no swing).
    w.put(pos, empty_pot);
    let empty_click = w.use_block(pos);
    assert_eq!(empty_click, InteractionResult::Consume);
    assert!(empty_click.consumes_action());
    assert!(!empty_click.should_swing());
    assert_eq!(w.get(pos), empty_pot);

    // Right-clicking an empty flower pot with a non-plant item does nothing (CONSUME, no swing).
    let non_plant_click = w.use_item(pos, "minecraft:stone");
    assert_eq!(non_plant_click, InteractionResult::Consume);
    assert!(non_plant_click.consumes_action());
    assert!(!non_plant_click.should_swing());
    assert_eq!(w.get(pos), empty_pot);
}

#[test]
fn update_shape_hook_can_answer_for_the_block() {
    use horizon_core::ticks::TickPriority;
    use std::cell::RefCell;

    struct Shaper {
        inner: TestLevel,
        b: &'static Behaviours,
        fence: BlockId,
        /// The state the hook answers for a fence, if any.
        answer: Option<BlockStateId>,
        seen: RefCell<Vec<(BlockPos, Direction)>>,
        /// The `state` each call was given.
        states: RefCell<Vec<BlockStateId>>,
    }
    impl LevelAccess for Shaper {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn emit(&mut self, event: LevelEvent) {
            self.inner.emit(event);
        }
        fn before_update_shape(
            &self,
            ticks: &mut Ticks,
            pos: BlockPos,
            state: BlockStateId,
            direction: Direction,
            neighbour_pos: BlockPos,
            neighbour: BlockStateId,
            new: &mut Option<BlockStateId>,
        ) {
            // A piston landing a block asks before the block is in the world.
            assert_eq!(self.block_state(neighbour_pos), neighbour);
            self.seen.borrow_mut().push((pos, direction));
            self.states.borrow_mut().push(state);
            if let Some(answer) = self.answer
                && self.b.block(state) == self.fence
            {
                *new = Some(answer);
                let time = self.game_time();
                ticks.schedule_block(time, pos, self.fence, 2, TickPriority::Normal);
            }
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let fence = f.registry.default_state("minecraft:oak_fence").unwrap();
    let with = |key: &str| {
        let mut s = f.registry.state(fence).unwrap().clone();
        s.properties.insert(key.into(), "true".into());
        f.registry.resolve(&s).unwrap()
    };
    let stone = f.registry.default_state("minecraft:stone").unwrap();
    let mut level = Shaper {
        inner: TestLevel::new(b),
        b,
        fence: b.block(fence),
        answer: Some(with("west")),
        seen: RefCell::new(Vec::new()),
        states: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    let pos = BlockPos::new(1, 1, 1);
    level.set_raw(pos, fence);
    // Stone placed east of the fence: the hook's state, not the fence's own,
    // and the tick the hook scheduled.
    b.set_block(
        &mut level,
        &mut ticks,
        BlockPos::new(2, 1, 1),
        stone,
        flags::UPDATE_ALL,
    );
    assert!(level.seen.borrow().contains(&(pos, Direction::East)));
    assert_eq!(level.block_state(pos), with("west"));
    assert!(ticks.has_block_tick(pos, b.block(fence)));
    // Fitted to all its neighbours (as post-processing or a piston does), the
    // fence is asked once per direction.
    level.seen.borrow_mut().clear();
    assert_eq!(
        b.update_from_neighbour_shapes(&mut level, &mut ticks, pos, with("west")),
        with("west")
    );
    assert_eq!(level.seen.borrow().len(), 6);
    // Fitted into an empty spot (a piston landing it), every call still got
    // the incoming state first, not the air at `pos`.
    level.seen.borrow_mut().clear();
    level.states.borrow_mut().clear();
    level.set_raw(pos, b.air());
    assert_eq!(
        b.update_from_neighbour_shapes(&mut level, &mut ticks, pos, fence),
        with("west")
    );
    assert_eq!(level.seen.borrow().len(), 6);
    assert_eq!(level.states.borrow()[0], fence);
    // A made-up id is ignored, not trusted.
    level.answer = Some(BlockStateId(u32::MAX));
    level.set_raw(pos, fence);
    b.set_block(
        &mut level,
        &mut ticks,
        BlockPos::new(2, 1, 1),
        stone,
        flags::UPDATE_ALL,
    );
    assert_ne!(level.block_state(pos), BlockStateId(u32::MAX));
    level.answer = Some(with("west"));
    // Without an answer the fence connects as vanilla's does.
    level.answer = None;
    level.set_raw(pos, fence);
    b.set_block(
        &mut level,
        &mut ticks,
        BlockPos::new(1, 1, 2),
        stone,
        flags::UPDATE_ALL,
    );
    assert_eq!(level.block_state(pos), with("south"));
    // Air breaks the fence, with its drops.
    level.answer = Some(b.air());
    level.set_raw(pos, fence);
    b.set_block(
        &mut level,
        &mut ticks,
        BlockPos::new(1, 1, 0),
        stone,
        flags::UPDATE_ALL,
    );
    assert_eq!(level.block_state(pos), b.air());
    assert!(level.inner.events.iter().any(
        |e| matches!(e, LevelEvent::DropResources { pos: p, state } if *p == pos && *state == fence)
    ));
}

#[test]
fn the_level_chooses_the_state_a_block_is_placed_in() {
    use crate::fluid::{FluidKind, FluidState};
    use horizon_core::geometry::Vec3;
    use std::cell::RefCell;

    /// What one call of the hook was asked.
    struct Asked {
        state: BlockStateId,
        pos: BlockPos,
        clicked: BlockPos,
        face: Direction,
        item: String,
        sneaking: bool,
        replace_clicked: bool,
        fluid: FluidState,
    }
    struct Placer {
        inner: TestLevel,
        b: &'static Behaviours,
        /// The block the hook answers for.
        block: BlockId,
        /// What it answers for that block.
        answer: Option<Option<BlockStateId>>,
        asked: RefCell<Vec<Asked>>,
    }
    impl LevelAccess for Placer {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn emit(&mut self, event: LevelEvent) {
            self.inner.emit(event);
        }
        fn before_state_for_placement(
            &self,
            state: BlockStateId,
            context: &PlacementContext,
            fluid: FluidState,
            answer: &mut Option<Option<BlockStateId>>,
        ) {
            self.asked.borrow_mut().push(Asked {
                state,
                pos: context.pos,
                clicked: context.hit_pos,
                face: context.face,
                item: context.item.clone(),
                sneaking: context.sneaking,
                replace_clicked: context.replace_clicked,
                fluid,
            });
            if self.b.block(state) == self.block {
                *answer = self.answer;
            }
        }
    }
    /// One call of the hook, as owned values (the level is mutable again after).
    fn asked(
        level: &Placer,
    ) -> (
        BlockStateId,
        BlockPos,
        BlockPos,
        Direction,
        String,
        bool,
        bool,
        FluidState,
    ) {
        let asked = level.asked.borrow();
        let first = &asked[0];
        (
            first.state,
            first.pos,
            first.clicked,
            first.face,
            first.item.clone(),
            first.sneaking,
            first.replace_clicked,
            first.fluid,
        )
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let stone = f.registry.default_state("minecraft:stone").unwrap();
    let furnace = f.registry.default_state("minecraft:furnace").unwrap();
    let furnace_block = b.block(furnace);
    // The same block facing another way: what a mod block's hook answers.
    let facing = |direction: &str| {
        let mut s = f.registry.state(furnace).unwrap().clone();
        s.properties.insert("facing".into(), direction.into());
        f.registry.resolve(&s).unwrap()
    };
    let mut level = Placer {
        inner: TestLevel::new(b),
        b,
        block: furnace_block,
        answer: Some(Some(facing("east"))),
        asked: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    let ground = BlockPos::new(0, 0, 0);
    level.set_raw(ground, stone);
    // Looking south (yaw 0) at the top of the floor, holding a furnace.
    let mut ctx = PlaceContext {
        clicked_pos: ground,
        face: Direction::Up,
        hit: Vec3::new(0.5, 1.0, 0.5),
        inside: false,
        yaw: 0.0,
        pitch: 0.0,
        sneaking: false,
        item: "minecraft:furnace".into(),
        can_use_game_master_blocks: false,
    };
    // The hook's state, not the one vanilla's furnace placement computes.
    assert_eq!(
        b.state_for_placement(&mut level, &mut ticks, furnace_block, &ctx),
        Some(facing("east"))
    );
    // It was asked with the block it decides for, where the block would go
    // (above the stone), the block clicked, the face clicked, the item in
    // hand, and the fluid of the position it goes in (none: dry stone).
    let (state, pos, clicked, face, item, sneaking, replace_clicked, fluid) = asked(&level);
    assert_eq!(state, furnace);
    assert_eq!(pos, BlockPos::new(0, 1, 0));
    assert_eq!(clicked, ground);
    assert_eq!(face, Direction::Up);
    assert_eq!(item, "minecraft:furnace");
    assert!(!replace_clicked);
    assert!(!sneaking);
    assert_eq!(fluid, FluidState::EMPTY);
    // Clicking a block that can be replaced: the block goes into it, and the
    // fluid already there (water) reaches the hook, which a waterlogged block
    // reads. Sneaking reaches it too, as `isSecondaryUseActive`.
    level.asked.borrow_mut().clear();
    let water = f.registry.default_state("minecraft:water").unwrap();
    level.set_raw(BlockPos::new(0, 1, 0), water);
    ctx.clicked_pos = BlockPos::new(0, 1, 0);
    ctx.sneaking = true;
    assert_eq!(
        b.state_for_placement(&mut level, &mut ticks, furnace_block, &ctx),
        Some(facing("east"))
    );
    let (_, pos, _, _, _, sneaking, replace_clicked, fluid) = asked(&level);
    assert_eq!(pos, BlockPos::new(0, 1, 0));
    assert!(replace_clicked);
    assert!(sneaking);
    assert_eq!(fluid.kind(), FluidKind::Water);
    // Refused (vanilla's `null` return): there is no state, so the placement
    // fails and the stack is kept.
    level.answer = Some(None);
    assert_eq!(
        b.state_for_placement(&mut level, &mut ticks, furnace_block, &ctx),
        None
    );
    // Without an answer the block's own `getStateForPlacement` runs: a furnace
    // placed with yaw 0 faces north, which is its default state.
    level.answer = None;
    assert_eq!(
        b.state_for_placement(&mut level, &mut ticks, furnace_block, &ctx),
        Some(furnace)
    );
    // And a block the hook does not answer for is unaffected.
    let stairs = b.block(f.registry.default_state("minecraft:oak_stairs").unwrap());
    let vanilla = b.state_for_placement(&mut level, &mut ticks, stairs, &ctx);
    level.answer = Some(Some(facing("west")));
    assert_eq!(
        b.state_for_placement(&mut level, &mut ticks, stairs, &ctx),
        vanilla
    );
    // A dispenser places through the same state computation
    // (`ShulkerBoxDispenseBehavior`'s `DirectionalPlaceContext`): it places
    // the hook's state too, with the dispenser's own view of the click.
    level.answer = Some(Some(facing("east")));
    level.asked.borrow_mut().clear();
    let front = BlockPos::new(3, 1, 1);
    let dispenser = PlaceContext {
        clicked_pos: front,
        face: Direction::East,
        hit: Vec3::new(3.0, 1.0, 1.5),
        inside: false,
        yaw: 270.0,
        pitch: 0.0,
        sneaking: false,
        item: "minecraft:furnace".into(),
        can_use_game_master_blocks: false,
    };
    assert!(b.directional_can_place(&mut level, &mut ticks, &dispenser));
    let outcome = b.place_block(&mut level, &mut ticks, &dispenser);
    assert_eq!(
        outcome,
        PlaceOutcome::Placed {
            pos: front,
            state: facing("east")
        }
    );
    assert_eq!(level.block_state(front), facing("east"));
    let (state, pos, _, _, item, _, replace_clicked, _) = asked(&level);
    assert_eq!(state, furnace);
    assert_eq!(pos, front);
    assert_eq!(item, "minecraft:furnace");
    assert!(replace_clicked, "the front block is air: it is replaced");
    // A made-up id is not trusted, nor is another block's state: the block
    // places as vanilla's would.
    level.answer = Some(Some(BlockStateId(u32::MAX)));
    assert_eq!(
        b.state_for_placement(&mut level, &mut ticks, furnace_block, &ctx),
        Some(furnace)
    );
    level.answer = Some(Some(b.default_of(stairs)));
    assert_eq!(
        b.state_for_placement(&mut level, &mut ticks, furnace_block, &ctx),
        Some(furnace)
    );
}

#[test]
fn survive_hooks_answer_for_the_block() {
    use std::cell::Cell;

    struct Rules {
        inner: TestLevel,
        b: &'static Behaviours,
        /// A plant and the one block it may stand on, by `mayPlaceOn`.
        ground: Option<(BlockId, BlockId)>,
        /// A block and its `canSurvive` answer.
        survive: Option<(BlockId, bool)>,
        asked: Cell<usize>,
    }
    impl LevelAccess for Rules {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn emit(&mut self, event: LevelEvent) {
            self.inner.emit(event);
        }
        fn before_can_survive(
            &self,
            _pos: BlockPos,
            state: BlockStateId,
            survives: &mut Option<bool>,
        ) {
            self.asked.set(self.asked.get() + 1);
            if let Some((block, answer)) = self.survive
                && self.b.block(state) == block
            {
                *survives = Some(answer);
            }
        }
        fn before_may_place_on(
            &self,
            state: BlockStateId,
            ground: BlockStateId,
            _ground_pos: BlockPos,
            may: &mut Option<bool>,
        ) {
            if let Some((plant, on)) = self.ground
                && self.b.block(state) == plant
            {
                *may = Some(self.b.block(ground) == on);
            }
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let state = |name: &str| f.registry.default_state(name).unwrap();
    let (dandelion, stone, cobblestone) = (
        state("minecraft:dandelion"),
        state("minecraft:stone"),
        state("minecraft:cobblestone"),
    );
    let mut level = Rules {
        inner: TestLevel::new(b),
        b,
        ground: None,
        survive: None,
        asked: Cell::new(0),
    };
    let mut ticks = Ticks::new(0);
    ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    let ground = BlockPos::new(1, 1, 1);
    let pos = BlockPos::new(1, 2, 1);
    let place = |level: &mut Rules, ticks: &mut Ticks, item: &str| {
        let ctx = PlaceContext {
            clicked_pos: ground,
            face: Direction::Up,
            hit: horizon_core::geometry::Vec3::new(1.5, 2.0, 1.5),
            inside: false,
            yaw: 0.0,
            pitch: 0.0,
            sneaking: false,
            item: item.into(),
            can_use_game_master_blocks: false,
        };
        b.place_block(level, ticks, &ctx)
    };
    level.set_raw(ground, stone);

    // A dandelion doesn't stand on stone in vanilla, so it can't be placed there.
    assert!(!b.can_survive(&mut level, &mut ticks, pos, dandelion));
    assert_eq!(
        place(&mut level, &mut ticks, "minecraft:dandelion"),
        PlaceOutcome::Failed
    );
    // A mod's mayPlaceOn names stone: placed, through the dandelion's own canSurvive.
    level.ground = Some((b.block(dandelion), b.block(stone)));
    assert!(matches!(
        place(&mut level, &mut ticks, "minecraft:dandelion"),
        PlaceOutcome::Placed { pos: p, .. } if p == pos
    ));
    assert_eq!(level.block_state(pos), dandelion);
    // Its ground turns to cobblestone: its shape update asks again and it pops off, with its drops.
    b.set_block(
        &mut level,
        &mut ticks,
        ground,
        cobblestone,
        flags::UPDATE_ALL,
    );
    assert_eq!(level.block_state(pos), b.air());
    assert!(level.inner.events.iter().any(
        |e| matches!(e, LevelEvent::DropResources { pos: p, state } if *p == pos && *state == dandelion)
    ));

    // canSurvive answered for the block itself wins over its own rule, either way.
    level.survive = Some((b.block(dandelion), true));
    assert!(b.can_survive(&mut level, &mut ticks, pos, dandelion));
    level.set_raw(ground, stone);
    level.survive = Some((b.block(stone), false));
    assert_eq!(
        place(&mut level, &mut ticks, "minecraft:stone"),
        PlaceOutcome::Failed
    );
    assert_eq!(level.block_state(pos), b.air());
    // Without an answer the block's own rule runs: stone survives anywhere.
    level.survive = None;
    let asked = level.asked.get();
    assert!(matches!(
        place(&mut level, &mut ticks, "minecraft:stone"),
        PlaceOutcome::Placed { .. }
    ));
    assert!(level.asked.get() > asked);
}

#[test]
fn a_level_can_answer_how_a_block_burns() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let glass = w.id("glass", &[]);
    let planks = w.id("oak_planks", &[]);
    let burn = |w: &mut World, fire: BlockPos| {
        w.set(fire.offset(0, -1, 0), w.id("netherrack", &[]));
        w.set(fire.offset(1, 0, 0), glass);
        w.set(fire.offset(-1, 0, 0), planks);
        w.set(fire, w.id("fire", &[]));
        for _ in 0..4000 {
            w.tick();
        }
    };
    // Vanilla's table: glass never burns, planks do.
    let fire = BlockPos::new(0, 1, 0);
    burn(&mut w, fire);
    assert_eq!(w.get(fire.offset(1, 0, 0)), glass, "glass doesn't burn");
    assert_ne!(w.get(fire.offset(-1, 0, 0)), planks, "planks burn");

    // The level's answer wins over the table, either way.
    w.level.fire_odds.insert(glass, (30, 60));
    w.level.fire_odds.insert(planks, (0, 0));
    let fire = BlockPos::new(0, 1, 6);
    burn(&mut w, fire);
    assert_ne!(w.get(fire.offset(1, 0, 0)), glass, "flammable glass burns");
    assert_eq!(
        w.get(fire.offset(-1, 0, 0)),
        planks,
        "fireproof planks don't"
    );

    // Fire in the air beside a block stays only while the block's ignite
    // odds are above zero, and burn odds of zero never destroy the block.
    let lit_beside = |w: &mut World, at: BlockPos, ticks: usize| {
        w.set(at.offset(1, 0, 0), glass);
        w.set(at, w.id("fire", &[]));
        for _ in 0..ticks {
            w.tick();
        }
        w.get(at)
    };
    let fire = w.id("fire", &[]);
    w.level.fire_odds.clear();
    let at = BlockPos::new(-6, 5, 0);
    assert_ne!(
        w.b().block(lit_beside(&mut w, at, 45)),
        w.b().block(fire),
        "fire beside plain glass goes out"
    );
    w.level.fire_odds.insert(glass, (5, 0));
    let at = BlockPos::new(-6, 5, 6);
    assert_eq!(
        w.b().block(lit_beside(&mut w, at, 45)),
        w.b().block(fire),
        "fire stands beside glass that ignites"
    );
    for _ in 0..4000 {
        w.tick();
    }
    assert_eq!(w.get(at.offset(1, 0, 0)), glass, "and never burns it away");
}

#[test]
fn a_level_can_answer_what_makes_a_campfire_signal() {
    let mut w = world!();
    let (stone, hay) = (w.id("stone", &[]), w.id("hay_block", &[]));
    let ground = BlockPos::new(0, 0, 0);
    let campfire = ground.offset(0, 1, 0);
    let place = |w: &mut World| {
        w.level.blocks.remove(&campfire);
        let ctx = PlaceContext {
            clicked_pos: ground,
            face: Direction::Up,
            hit: horizon_core::geometry::Vec3::new(0.5, 1.0, 0.5),
            inside: false,
            yaw: 0.0,
            pitch: 0.0,
            sneaking: false,
            item: "minecraft:campfire".into(),
            can_use_game_master_blocks: false,
        };
        let b = w.b();
        assert!(matches!(
            b.place_block(&mut w.level, &mut w.ticks, &ctx),
            PlaceOutcome::Placed { .. }
        ));
        w.prop(campfire, "signal_fire")
    };
    w.put(ground, stone);
    assert_eq!(place(&mut w), "false");
    w.put(ground, hay);
    assert_eq!(place(&mut w), "true", "vanilla's hay bale");

    // The level makes stone a smoke source and hay not: placed, and as the ground changes.
    w.level.smoke_sources.insert(stone, true);
    w.level.smoke_sources.insert(hay, false);
    assert_eq!(place(&mut w), "false");
    w.set(ground, stone);
    assert_eq!(w.prop(campfire, "signal_fire"), "true");
    w.set(ground, hay);
    assert_eq!(w.prop(campfire, "signal_fire"), "false");
}
