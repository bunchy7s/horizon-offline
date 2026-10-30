//! Behaviour tests over an in-memory level. They need the prepared client
//! resources (`HORIZON_RESOURCES`, or `.local/horizon-client/<version>` at the
//! repository root) and are skipped when those are absent.
use super::*;
use crate::chunk::BlockState;
use std::{
    collections::{BTreeMap, HashMap},
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
}
impl TestLevel {
    fn new(b: &Behaviours) -> Self {
        Self {
            blocks: HashMap::new(),
            air: b.air(),
            random: LegacyRandom::new(42),
            time: 0,
            day_time: 0,
            events: Vec::new(),
            block_light: HashMap::new(),
            client_side: false,
        }
    }
}
impl LevelAccess for TestLevel {
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
                .collect::<BTreeMap<_, _>>(),
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
fn bucket_source_removal_drains_spread_water() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let at = BlockPos::new(0, 1, 0);
    assert!(w.b().bucket_empty(
        &mut w.level,
        &mut w.ticks,
        at,
        BucketContent::Fluid(crate::fluid::FluidKind::Water),
        "minecraft:item.bucket.empty"
    ));
    for _ in 0..60 {
        w.tick();
    }
    assert_ne!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at),
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
    /// Right-click the north face of `pos` holding `item` in the main hand.
    fn use_item(&mut self, pos: BlockPos, item: &str) -> InteractionResult {
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
        self.f.behaviours.use_item_on(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
            Some(item),
        )
    }
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
    let picked = w.b().bucket_pickup(&mut w.level, &mut w.ticks, at);
    assert_eq!(picked, Some("minecraft:water_bucket"));
    assert_eq!(w.get(at), w.b().air());
    // Flowing water gives nothing.
    let flowing = w.id("water", &[("level", "3")]);
    w.put(at, flowing);
    assert_eq!(w.b().bucket_pickup(&mut w.level, &mut w.ticks, at), None);
    // A waterlogged slab is drained and stays a slab.
    let slab = w.id("oak_slab", &[("waterlogged", "true")]);
    w.put(at, slab);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at),
        Some("minecraft:water_bucket")
    );
    assert_eq!(w.prop(at, "waterlogged"), "false");
    // Powder snow comes up as its bucket.
    let snow = w.id("powder_snow", &[]);
    w.put(at, snow);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at),
        Some("minecraft:powder_snow_bucket")
    );
    // Emptying water into air makes a source; stone refuses.
    let sound = "minecraft:item.bucket.empty";
    let content = BucketContent::Fluid(crate::fluid::FluidKind::Water);
    assert!(
        w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound)
    );
    assert_eq!(w.name(at), "water[level=0]");
    let stone = w.id("stone", &[]);
    w.put(at, stone);
    assert!(
        !w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound)
    );
    // Water fills a waterloggable slab instead of replacing it.
    let dry = w.id("oak_slab", &[]);
    w.put(at, dry);
    assert!(
        w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound)
    );
    assert_eq!(w.prop(at, "waterlogged"), "true");
    // Powder snow only goes into empty space.
    let air = w.b().air();
    w.put(at, air);
    assert!(w.b().bucket_empty(
        &mut w.level,
        &mut w.ticks,
        at,
        BucketContent::Powder,
        "minecraft:item.bucket.empty_powder_snow"
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
