//! Sapling growth, bone meal and templates through live feature placement.
//! Needs the prepared client resources and worldgen bundle (skipped when
//! absent).
use super::*;
use crate::{chunk::BlockState, worldgen::generator::GeneratorData, worldgen::live::LiveFeatures};
use std::{collections::HashMap, path::PathBuf, sync::OnceLock};

struct Fixture {
    registry: &'static BlockRegistry,
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
            // Trees need the feature block-behaviour exports in the worldgen bundle.
            if !root.join("block-behaviours.json").exists()
                || !root.join("worldgen/blocks/block-properties.nbt").exists()
            {
                eprintln!("skipping growth tests: no resources at {}", root.display());
                return None;
            }
            let registry: &'static BlockRegistry =
                Box::leak(Box::new(data::load_registry(&root).expect("registry")));
            let behaviours = Behaviours::load(registry, &root).expect("behaviours");
            let data =
                GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data");
            let features = LiveFeatures::new(&data)
                .expect("features")
                .expect("feature bundle");
            behaviours.set_live_features(Arc::new(features));
            Some(Fixture {
                registry,
                behaviours,
            })
        })
        .as_ref()
}

struct Level {
    blocks: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    events: Vec<LevelEvent>,
    biome: &'static str,
    day: i64,
    block_entities: Vec<(BlockPos, crate::nbt::Compound)>,
}
impl LevelAccess for Level {
    fn day_time(&self) -> i64 {
        self.day
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
        0
    }
    fn emit(&mut self, event: LevelEvent) {
        self.events.push(event);
    }
    fn biome_name(&self, _pos: BlockPos) -> Option<&str> {
        Some(self.biome)
    }
    fn load_block_entity(&mut self, pos: BlockPos, nbt: crate::nbt::Compound) {
        self.block_entities.push((pos, nbt));
    }
}

struct World {
    f: &'static Fixture,
    level: Level,
    ticks: Ticks,
}
impl World {
    fn new(f: &'static Fixture, seed: i64) -> Self {
        let mut ticks = Ticks::new(0);
        for x in -4..4 {
            for z in -4..4 {
                ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
            }
        }
        Self {
            level: Level {
                blocks: HashMap::new(),
                air: f.behaviours.air(),
                random: LegacyRandom::new(seed),
                events: Vec::new(),
                biome: "minecraft:plains",
                day: 6000,
                block_entities: Vec::new(),
            },
            f,
            ticks,
        }
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
    fn floor(&mut self, name: &str, radius: i32) {
        let state = self.id(name, &[]);
        let stone = self.id("stone", &[]);
        for x in -radius..=radius {
            for z in -radius..=radius {
                self.level.blocks.insert(BlockPos::new(x, 63, z), state);
                self.level.blocks.insert(BlockPos::new(x, 62, z), stone);
            }
        }
    }
    fn put(&mut self, pos: BlockPos, state: BlockStateId) {
        self.level.blocks.insert(pos, state);
    }
    fn name(&self, pos: BlockPos) -> String {
        let state = self.level.block_state(pos);
        self.f.registry.state(state).unwrap().name.clone()
    }
    fn bone_meal(&mut self, pos: BlockPos) -> bool {
        self.f
            .behaviours
            .use_bone_meal(&mut self.level, &mut self.ticks, pos, Direction::Up)
    }
    fn count(&self, suffix: &str) -> usize {
        self.level
            .blocks
            .values()
            .filter(|&&s| self.f.registry.state(s).unwrap().name.ends_with(suffix))
            .count()
    }
}

fn grow(sapling: &str, soil: &str, positions: &[(i32, i32)], log: &str) {
    let Some(f) = fixture() else { return };
    let mut grown = 0;
    for seed in 0..6 {
        let mut w = World::new(f, seed);
        w.floor(soil, 12);
        let state = w.id(sapling, &[]);
        for &(x, z) in positions {
            w.put(BlockPos::new(x, 64, z), state);
        }
        let origin = BlockPos::new(0, 64, 0);
        let mut uses = 0;
        while w.name(origin) == format!("minecraft:{sapling}") && uses < 64 {
            assert!(w.bone_meal(origin), "{sapling} is a bone meal target");
            uses += 1;
        }
        if w.count(log) > 0 {
            grown += 1;
            assert_eq!(
                w.name(origin),
                format!("minecraft:{log}"),
                "{sapling} trunk at origin"
            );
        }
    }
    assert!(grown >= 5, "{sapling}: grew {grown} of 6 times");
}

#[test]
fn oak_sapling_grows_with_bone_meal() {
    grow("oak_sapling", "grass_block", &[(0, 0)], "oak_log");
}
#[test]
fn a_tree_with_bees_fills_its_nest() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 3);
    w.floor("grass_block", 12);
    let origin = BlockPos::new(0, 64, 0);
    let placed = f.behaviours.place_feature(
        &mut w.level,
        &mut w.ticks,
        "minecraft:fancy_oak_bees",
        origin,
    );
    assert_eq!(placed, Some(true));
    assert_eq!(w.count("bee_nest"), 1);
    let [(at, nbt)] = &w.level.block_entities[..] else {
        panic!("one block entity: {:?}", w.level.block_entities);
    };
    assert_eq!(w.name(*at), "minecraft:bee_nest");
    let Some(crate::nbt::Tag::List(bees)) = nbt.get(&"bees".into()) else {
        panic!("bees: {nbt:?}");
    };
    assert!(
        (2..=3).contains(&bees.len()),
        "BeehiveDecorator stores 2 or 3 bees"
    );
}
#[test]
fn single_saplings_grow() {
    grow("birch_sapling", "dirt", &[(0, 0)], "birch_log");
    grow("spruce_sapling", "dirt", &[(0, 0)], "spruce_log");
    grow("jungle_sapling", "dirt", &[(0, 0)], "jungle_log");
    grow("acacia_sapling", "dirt", &[(0, 0)], "acacia_log");
    grow("cherry_sapling", "grass_block", &[(0, 0)], "cherry_log");
    grow("poplar_sapling", "grass_block", &[(0, 0)], "poplar_log");
}
#[test]
fn two_by_two_saplings_grow() {
    let square = [(0, 0), (1, 0), (0, 1), (1, 1)];
    grow("dark_oak_sapling", "grass_block", &square, "dark_oak_log");
    grow("pale_oak_sapling", "grass_block", &square, "pale_oak_log");
    grow("spruce_sapling", "podzol", &square, "spruce_log");
    grow("jungle_sapling", "grass_block", &square, "jungle_log");
}
#[test]
fn lone_dark_oak_sapling_cannot_grow() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 1);
    w.floor("grass_block", 4);
    let sapling = w.id("dark_oak_sapling", &[]);
    w.put(BlockPos::new(0, 64, 0), sapling);
    assert!(!w.bone_meal(BlockPos::new(0, 64, 0)));
}
#[test]
fn bone_meal_on_a_client_succeeds_only_where_its_level_can_tell() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 1);
    w.floor("grass_block", 4);
    let sapling = w.id("oak_sapling", &[]);
    w.put(BlockPos::new(0, 64, 0), sapling);
    w.put(BlockPos::new(1, 64, 0), w.id("wheat", &[("age", "0")]));
    w.put(BlockPos::new(2, 64, 0), w.id("wheat", &[("age", "7")]));
    w.put(BlockPos::new(3, 64, 0), w.id("water", &[("level", "0")]));
    let before = w.level.blocks.clone();
    let mut client = |pos| {
        f.behaviours
            .bone_meal_succeeds_on_client(&mut w.level, pos, Direction::Up)
    };
    // SaplingBlock.isValidBonemealTarget needs a ServerLevel: the client passes.
    assert!(!client(BlockPos::new(0, 64, 0)));
    // CropBlock: below its max age.
    assert!(client(BlockPos::new(1, 64, 0)));
    assert!(!client(BlockPos::new(2, 64, 0)));
    // GrassBlock with air above; under water, growWaterPlant above its top face.
    assert!(client(BlockPos::new(-1, 63, 0)));
    assert!(client(BlockPos::new(3, 63, 0)));
    assert!(!client(BlockPos::new(-1, 62, 0)));
    assert_eq!(w.level.blocks, before, "nothing grows on a client");
    assert!(w.level.events.is_empty());
    // The server's growCrop takes the sapling.
    assert!(w.bone_meal(BlockPos::new(0, 64, 0)));
}
/// A mod block answers vanilla's three `BonemealableBlock` questions itself,
/// ahead of the `BonemealableBlock` its template would give it, and the meal
/// is used up with the growth particles and sound as vanilla's are.
#[test]
fn bone_meal_answers_a_mod_blocks_three_questions() {
    /// A level whose bone meal answers are the test's.
    struct ModLevel<'a> {
        inner: &'a mut Level,
        valid: Option<bool>,
        success: bool,
        /// `isBonemealSuccess` was asked, and what it saw.
        rolls: Vec<(BlockPos, BlockStateId)>,
        /// `performBonemeal` ran, and what it saw.
        performed: Vec<(BlockPos, BlockStateId)>,
    }
    impl LevelAccess for ModLevel<'_> {
        fn day_time(&self) -> i64 {
            self.inner.day_time()
        }
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
        fn is_valid_bonemeal_target(&self, _pos: BlockPos, _state: BlockStateId) -> Option<bool> {
            self.valid
        }
        fn is_bonemeal_success(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
            self.rolls.push((pos, state));
            self.success
        }
        fn perform_bonemeal(&mut self, _ticks: &mut Ticks, pos: BlockPos, state: BlockStateId) {
            self.performed.push((pos, state));
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let mut w = World::new(f, 1);
    w.floor("stone", 4);
    let pos = BlockPos::new(0, 64, 0);
    let stone = w.id("stone", &[]);
    let crop = w.id("wheat", &[("age", "0")]);
    // A block nothing in Horizon bonemeals: the level's answers take the meal,
    // the roll grows it, and the caller fires the growth particles and sound.
    w.put(pos, stone);
    let mut level = ModLevel {
        inner: &mut w.level,
        valid: Some(true),
        success: true,
        rolls: Vec::new(),
        performed: Vec::new(),
    };
    assert!(b.use_bone_meal(&mut level, &mut w.ticks, pos, Direction::Up));
    assert_eq!(level.rolls, vec![(pos, stone)]);
    assert_eq!(level.performed, vec![(pos, stone)]);
    assert!(level.inner.events.iter().any(|e| matches!(
        e,
        LevelEvent::LevelEvent { id: 1505, pos: at, data: 15 } if *at == pos
    )));
    // A roll that fails still uses the meal up, without growing it.
    let mut level = ModLevel {
        inner: &mut w.level,
        valid: Some(true),
        success: false,
        rolls: Vec::new(),
        performed: Vec::new(),
    };
    assert!(b.use_bone_meal(&mut level, &mut w.ticks, pos, Direction::Up));
    assert!(level.performed.is_empty());
    // Refused: not a target, no roll, no growth, no meal used.
    let mut level = ModLevel {
        inner: &mut w.level,
        valid: Some(false),
        success: true,
        rolls: Vec::new(),
        performed: Vec::new(),
    };
    assert!(!b.use_bone_meal(&mut level, &mut w.ticks, pos, Direction::Up));
    assert!(level.rolls.is_empty() && level.performed.is_empty());
    // A block whose template bonemeals (a crop) is the mod's instead: the
    // block's own growth doesn't run.
    w.put(pos, crop);
    let mut level = ModLevel {
        inner: &mut w.level,
        valid: Some(true),
        success: true,
        rolls: Vec::new(),
        performed: Vec::new(),
    };
    assert!(b.grow_crop(&mut level, &mut w.ticks, pos));
    assert_eq!(level.performed, vec![(pos, crop)]);
    assert_eq!(level.inner.block_state(pos), crop);
    // Refused by the mod: vanilla's growCrop stops, the crop keeps its meal
    // and doesn't grow.
    let mut level = ModLevel {
        inner: &mut w.level,
        valid: Some(false),
        success: true,
        rolls: Vec::new(),
        performed: Vec::new(),
    };
    assert!(!b.grow_crop(&mut level, &mut w.ticks, pos));
    assert_eq!(level.inner.block_state(pos), crop);
    // Answered nothing, the crop keeps its own bone meal and grows.
    let mut level = ModLevel {
        inner: &mut w.level,
        valid: None,
        success: true,
        rolls: Vec::new(),
        performed: Vec::new(),
    };
    assert!(b.grow_crop(&mut level, &mut w.ticks, pos));
    assert!(level.rolls.is_empty() && level.performed.is_empty());
    assert_ne!(level.inner.block_state(pos), crop, "the crop grew");
    // A client predicts the use with the same answer, where the block itself
    // takes no bone meal on a client.
    w.put(pos, stone);
    let mut level = ModLevel {
        inner: &mut w.level,
        valid: Some(true),
        success: true,
        rolls: Vec::new(),
        performed: Vec::new(),
    };
    assert!(b.bone_meal_succeeds_on_client(&mut level, pos, Direction::Up));
    level.valid = Some(false);
    assert!(!b.bone_meal_succeeds_on_client(&mut level, pos, Direction::Up));
}

#[test]
fn azalea_and_mangrove_grow() {
    let Some(f) = fixture() else { return };
    for (block, soil, log) in [
        ("azalea", "grass_block", "oak_log"),
        ("mangrove_propagule", "mud", "mangrove_log"),
    ] {
        let mut grew = false;
        for seed in 0..6 {
            let mut w = World::new(f, seed);
            w.floor(soil, 12);
            let state = w.id(block, &[]);
            w.put(BlockPos::new(0, 64, 0), state);
            for _ in 0..64 {
                if w.count(log) > 0 {
                    break;
                }
                w.bone_meal(BlockPos::new(0, 64, 0));
            }
            grew |= w.count(log) > 0;
        }
        assert!(grew, "{block} grows into a tree");
    }
}
#[test]
fn random_ticks_advance_sapling_stage_then_grow() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 3);
    w.floor("grass_block", 8);
    let sapling = w.id("oak_sapling", &[]);
    let pos = BlockPos::new(0, 64, 0);
    w.put(pos, sapling);
    for _ in 0..2000 {
        let state = w.level.block_state(pos);
        if !f.behaviours.is_randomly_ticking(state) {
            break;
        }
        f.behaviours.random_tick(&mut w.level, &mut w.ticks, pos);
    }
    assert_eq!(w.name(pos), "minecraft:oak_log");
}
#[test]
fn grass_bone_meal_places_grass_and_flowers() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 7);
    w.floor("grass_block", 8);
    assert!(w.bone_meal(BlockPos::new(0, 63, 0)));
    let plants = w.count("short_grass") + w.count("tall_grass");
    let flowers = w.count("dandelion")
        + w.count("poppy")
        + w.count("cornflower")
        + w.count("oxeye_daisy")
        + w.count("azure_bluet")
        + w.count("_tulip");
    assert!(plants > 10, "grass placed: {plants}");
    assert!(plants + flowers > 15);
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::LevelEvent { id: 1505, .. }))
    );
}
#[test]
fn huge_mushroom_and_fungus_grow() {
    let Some(f) = fixture() else { return };
    let mut grew = (false, false);
    for seed in 0..8 {
        let mut w = World::new(f, seed);
        w.floor("mycelium", 8);
        let mushroom = w.id("red_mushroom", &[]);
        w.put(BlockPos::new(0, 64, 0), mushroom);
        for _ in 0..16 {
            w.bone_meal(BlockPos::new(0, 64, 0));
        }
        grew.0 |= w.count("red_mushroom_block") > 0 && w.count("mushroom_stem") > 0;
        let mut w = World::new(f, seed);
        w.floor("crimson_nylium", 8);
        let fungus = w.id("crimson_fungus", &[]);
        w.put(BlockPos::new(0, 64, 0), fungus);
        for _ in 0..16 {
            if w.count("crimson_stem") > 0 {
                break;
            }
            w.bone_meal(BlockPos::new(0, 64, 0));
        }
        grew.1 |= w.count("crimson_stem") > 0 && w.count("nether_wart_block") > 0;
    }
    assert!(grew.0, "huge red mushroom");
    assert!(grew.1, "huge crimson fungus");
}
#[test]
fn crops_age_with_bone_meal() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 11);
    let farmland = w.id("farmland", &[]);
    let pos = BlockPos::new(0, 64, 0);
    w.put(BlockPos::new(0, 63, 0), farmland);
    w.put(pos, w.id("wheat", &[]));
    let mut uses = 0;
    while w.bone_meal(pos) {
        uses += 1;
    }
    assert!((2..=4).contains(&uses), "wheat matured after {uses} uses");
    assert_eq!(w.level.block_state(pos), w.id("wheat", &[("age", "7")]));
    w.put(pos, w.id("torchflower_crop", &[]));
    assert!(w.bone_meal(pos));
    assert!(w.bone_meal(pos));
    assert_eq!(w.name(pos), "minecraft:torchflower");
    assert!(!w.bone_meal(pos), "torchflowers are not bonemealable");
}
#[test]
fn tall_flowers_and_seagrass() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 5);
    w.floor("dirt", 2);
    let fern = w.id("fern", &[]);
    w.put(BlockPos::new(0, 64, 0), fern);
    assert!(w.bone_meal(BlockPos::new(0, 64, 0)));
    assert_eq!(w.name(BlockPos::new(0, 64, 0)), "minecraft:large_fern");
    assert_eq!(w.name(BlockPos::new(0, 65, 0)), "minecraft:large_fern");
    let lilac = w.id("lilac", &[("half", "lower")]);
    w.put(BlockPos::new(1, 64, 0), lilac);
    assert!(w.bone_meal(BlockPos::new(1, 64, 0)));
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::BlockEvent {
            kind: "pop_block_item",
            ..
        }
    )));
}

#[test]
fn snowy_biomes_freeze_and_snow() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 1);
    w.level.biome = "minecraft:snowy_plains";
    w.floor("grass_block", 2);
    let water = w.id("water", &[]);
    w.put(BlockPos::new(1, 63, 0), water);
    let b = &f.behaviours;
    let grass = b
        .precipitation_info(&w.level, BlockPos::new(0, 70, 0))
        .unwrap();
    assert_eq!(grass.top, BlockPos::new(0, 64, 0));
    assert!(grass.snows_at_top && !grass.freezes_below);
    assert_eq!(grass.kind, PrecipitationKind::Snow);
    let pond = b
        .precipitation_info(&w.level, BlockPos::new(1, 70, 0))
        .unwrap();
    assert!(pond.freezes_below, "a water source at the edge freezes");
    w.level.biome = "minecraft:plains";
    let rain = b
        .precipitation_info(&w.level, BlockPos::new(0, 70, 0))
        .unwrap();
    assert!(!rain.snows_at_top && !rain.freezes_below);
    assert_eq!(rain.kind, PrecipitationKind::Rain);
}

#[test]
fn a_template_with_entities_and_no_blocks_places_them() {
    use crate::{
        nbt::{Compound, JavaString, Tag},
        worldgen::structure::{EntityInfo, Mirror, Rotation, StructureTemplate, save_nbt},
    };
    let Some(f) = fixture() else { return };
    let resources = f
        .behaviours
        .live_features()
        .and_then(|features| features.structure_resources())
        .expect("structure resources");
    let mut pig = Compound::new();
    pig.insert("id".into(), Tag::String("minecraft:pig".into()));
    let pig = EntityInfo {
        pos: [0.5, 0.0, 0.5],
        block_pos: [0, 0, 0],
        nbt: pig,
    };
    // StructureTemplate.placeInWorld places when there are blocks, or entities
    // it does not ignore.
    for (entities, ignore_entities, expected) in [
        (vec![pig.clone()], false, true),
        (vec![pig], true, false),
        (vec![], false, false),
    ] {
        let mut w = World::new(f, 0);
        let tag = save_nbt(
            f.registry,
            &resources.transforms,
            [1, 1, 1],
            Vec::new(),
            &entities,
            horizon_core::version::DATA_VERSION,
        );
        let template = StructureTemplate::from_nbt(&tag, f.registry, &resources.transforms);
        let settings = TemplateSettings {
            mirror: Mirror::None,
            rotation: Rotation::None,
            ignore_entities,
            known_shape: false,
            integrity: 1.0,
            seed: 0,
            flags: flags::UPDATE_CLIENTS,
        };
        let (placed, spawned) = f
            .behaviours
            .place_template(
                &mut w.level,
                &mut w.ticks,
                &template,
                BlockPos::new(3, 64, -2),
                settings,
            )
            .expect("structure resources");
        let what = format!("{} entities, ignored {ignore_entities}", entities.len());
        assert_eq!(placed, expected, "{what}");
        assert_eq!(spawned.len(), usize::from(expected), "{what}");
        if let Some(nbt) = spawned.first() {
            assert_eq!(
                nbt.get(&JavaString::from("id")),
                Some(&Tag::String("minecraft:pig".into()))
            );
            let pos = [3.5, 64.0, -1.5].map(|v: f64| Tag::Double(v.to_bits()));
            assert_eq!(
                nbt.get(&JavaString::from("Pos")),
                Some(&Tag::List(pos.to_vec()))
            );
        }
    }
}
#[test]
fn eyeblossoms_open_at_night_and_wake_neighbours() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 2);
    w.floor("grass_block", 4);
    let closed = w.id("closed_eyeblossom", &[]);
    w.put(BlockPos::new(0, 64, 0), closed);
    w.put(BlockPos::new(2, 64, 1), closed);
    f.behaviours
        .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(0, 64, 0));
    assert_eq!(
        w.name(BlockPos::new(0, 64, 0)),
        "minecraft:closed_eyeblossom",
        "closed by day"
    );
    w.level.day = 13000;
    f.behaviours
        .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(0, 64, 0));
    assert_eq!(w.name(BlockPos::new(0, 64, 0)), "minecraft:open_eyeblossom");
    let block = f.behaviours.block(closed);
    assert!(
        w.ticks.has_block_tick(BlockPos::new(2, 64, 1), block),
        "neighbour scheduled"
    );
    assert!(w.level.events.iter().any(|e| matches!(e, LevelEvent::Sound { sound, .. } if &**sound == "minecraft:block.eyeblossom.open_long")));
}
#[test]
fn turtle_eggs_crack_at_dawn_on_sand() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 3);
    w.floor("sand", 1);
    let egg = w.id("turtle_egg", &[]);
    let pos = BlockPos::new(0, 64, 0);
    w.put(pos, egg);
    w.level.day = 21500;
    f.behaviours.random_tick(&mut w.level, &mut w.ticks, pos);
    assert_eq!(
        w.level.block_state(pos),
        w.id("turtle_egg", &[("hatch", "1")])
    );
    f.behaviours.random_tick(&mut w.level, &mut w.ticks, pos);
    f.behaviours.random_tick(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.name(pos), "minecraft:air", "hatched");
}
#[test]
fn sniffer_eggs_crack_twice_then_hatch_a_snifflet() {
    let Some(f) = fixture() else { return };
    let mut w = World::new(f, 5);
    w.floor("moss_block", 1);
    let egg = w.id("sniffer_egg", &[]);
    let pos = BlockPos::new(0, 64, 0);
    f.behaviours
        .set_block(&mut w.level, &mut w.ticks, pos, egg, flags::UPDATE_ALL);
    let block = f.behaviours.block(egg);
    assert!(
        w.ticks.has_block_tick(pos, block),
        "placing schedules the first stage"
    );
    for stage in 1..=2 {
        f.behaviours
            .scheduled_block_tick(&mut w.level, &mut w.ticks, pos);
        assert_eq!(
            w.level.block_state(pos),
            w.id("sniffer_egg", &[("hatch", &stage.to_string())])
        );
    }
    f.behaviours
        .scheduled_block_tick(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.name(pos), "minecraft:air", "hatched");
    assert!(w.level.events.iter().any(|e| matches!(
        e,
        LevelEvent::BlockEvent {
            kind: "hatch_sniffer",
            ..
        }
    )));
}
#[test]
fn a_non_strict_template_updates_its_shapes_and_neighbours() {
    use crate::worldgen::structure::{BlockInfo, Mirror, Rotation, StructureTemplate, save_nbt};
    let Some(f) = fixture() else { return };
    let resources = f
        .behaviours
        .live_features()
        .and_then(|features| features.structure_resources())
        .expect("structure resources");
    for strict in [false, true] {
        let mut w = World::new(f, 0);
        let fence = w.id("oak_fence", &[]);
        let power = w.id("redstone_block", &[]);
        let block = |pos: [i32; 3], state: BlockStateId| BlockInfo {
            pos,
            state,
            nbt: None,
        };
        let blocks = vec![
            block([0, 0, 0], fence),
            block([1, 0, 0], fence),
            block([2, 0, 0], power),
        ];
        let tag = save_nbt(
            f.registry,
            &resources.transforms,
            [3, 1, 1],
            blocks,
            &[],
            horizon_core::version::DATA_VERSION,
        );
        let template = StructureTemplate::from_nbt(&tag, f.registry, &resources.transforms);
        let lamp = w.id("redstone_lamp", &[("lit", "false")]);
        w.put(BlockPos::new(3, 64, 0), lamp);
        let settings = TemplateSettings {
            mirror: Mirror::None,
            rotation: Rotation::None,
            ignore_entities: true,
            known_shape: strict,
            integrity: 1.0,
            seed: 0,
            // StructureBlockEntity.placeStructure's `2 | (strict ? 816 : 0)`.
            flags: flags::UPDATE_CLIENTS | if strict { 816 } else { 0 },
        };
        let (placed, _) = f
            .behaviours
            .place_template(
                &mut w.level,
                &mut w.ticks,
                &template,
                BlockPos::new(0, 64, 0),
                settings,
            )
            .expect("structure resources");
        assert!(placed);
        // The middle fence: placing the redstone block connects its east side,
        // only the shape pass connects its west side to the fence placed before
        // it.
        let connected = w.id("oak_fence", &[("east", "true"), ("west", "true")]);
        let lit = w.id("redstone_lamp", &[("lit", "true")]);
        assert_eq!(
            w.level.block_state(BlockPos::new(1, 64, 0)) == connected,
            !strict,
            "strict {strict}: the middle fence connects to the fence placed before it"
        );
        assert_eq!(
            w.level.block_state(BlockPos::new(3, 64, 0)) == lit,
            !strict,
            "strict {strict}: the lamp next to the template lights"
        );
    }
}
