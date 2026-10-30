//! Behaviour tables built once from the pinned client resources.
use super::{
    Behaviours, GameplayStateSource,
    classes::{Class, ClassSet, Interface},
    states::{BlockId, PropKey, StateTable},
};
use crate::{
    collision::CachedCollisionRegistry,
    compression::invalid,
    fluid::{FluidBlockBindings, FluidKind, FluidState},
    registry::{BlockRegistry, BlockStateId, BlockTableFile},
    worldgen::tags::TagDefinitions,
};
use horizon_core::geometry::{Aabb, Direction, ShapeOp, Vec3, VoxelShape};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::{self, File},
    io,
    path::Path,
    sync::Arc,
};
#[path = "data/cache.rs"]
mod cache;
pub(super) use cache::set_dir as set_cache_dir;

/// Per-state flags read from vanilla accessors over an empty level.
pub mod state_flags {
    pub const SOLID: u32 = 1;
    pub const AIR: u32 = 1 << 1;
    pub const CONDUCTOR: u32 = 1 << 2;
    pub const SIGNAL_SOURCE: u32 = 1 << 3;
    pub const ANALOG_OUTPUT: u32 = 1 << 4;
    pub const COLLISION_EMPTY: u32 = 1 << 5;
    pub const SUFFOCATING: u32 = 1 << 6;
    pub const REPLACEABLE: u32 = 1 << 7;
    pub const RANDOM_TICKS: u32 = 1 << 8;
    pub const SOLID_RENDER: u32 = 1 << 9;
    pub const LIQUID: u32 = 1 << 10;
    pub const BLOCK_ENTITY: u32 = 1 << 11;
    pub const IGNITED_BY_LAVA: u32 = 1 << 12;
    pub const CAN_OCCLUDE: u32 = 1 << 13;
    pub const FULL_COLLISION: u32 = 1 << 14;
    pub const SKYLIGHT_DOWN: u32 = 1 << 15;
    pub const REQUIRES_TOOL: u32 = 1 << 16;
    pub const SHAPE_OCCLUSION: u32 = 1 << 17;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushReaction {
    PushPull,
    Push,
    Popped,
    Immoveable,
    IgnoreEntity,
}

#[derive(Clone, Copy, Debug)]
pub struct StateFacts {
    pub flags: u32,
    /// isFaceSturdy bits: support type (FULL, CENTER, RIGID) * 6 + direction.
    pub sturdy: u32,
    /// getSignal / getDirectSignal over an empty level, 4 bits per direction.
    pub weak: u32,
    pub strong: u32,
    pub fluid: FluidState,
    pub emission: u8,
    pub dampening: u8,
    pub hardness: f32,
    pub push: PushReaction,
    pub instrument: u8,
    /// Index into `Behaviours::shapes`, or -1 for an uncached (dynamic) shape.
    pub shape: i32,
}

#[derive(Clone, Debug)]
pub struct SetType {
    pub name: Box<str>,
    pub can_open_by_hand: bool,
    pub can_open_by_wind_charge: bool,
    pub arrows: bool,
    /// true: MOBS sensitivity (living entities only); false: EVERYTHING.
    pub mobs_only: bool,
    pub door_open: Arc<str>,
    pub door_close: Arc<str>,
    pub trapdoor_open: Arc<str>,
    pub trapdoor_close: Arc<str>,
    pub plate_on: Arc<str>,
    pub plate_off: Arc<str>,
    pub button_on: Arc<str>,
    pub button_off: Arc<str>,
}

#[derive(Clone, Debug)]
pub struct Sounds {
    pub volume: f32,
    pub pitch: f32,
    pub place: Arc<str>,
    pub break_: Arc<str>,
    pub fall: Arc<str>,
}

/// Precomputed queries over one cached collision shape.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShapeInfo {
    /// `Block.isFaceFull(shape, direction)` per direction bit.
    pub full_faces: u8,
    /// `!shape.getFaceShape(direction).isEmpty()` per direction bit.
    pub nonempty_faces: u8,
    /// WallBlock `isCovered(shape.getFaceShape(DOWN), test)` for the post (bit 0)
    /// and the north/east/south/west wall tests (bits 1..=4).
    pub wall_cover: u8,
}

pub(crate) fn analyse_shape(shape: &VoxelShape) -> io::Result<ShapeInfo> {
    let err = |e: horizon_core::geometry::ShapeError| invalid(e.to_string());
    let mut info = ShapeInfo::default();
    let block = VoxelShape::block();
    for direction in Direction::ALL {
        let face = shape.face_shape(direction).map_err(err)?;
        if !block.join_is_not_empty(&face, ShapeOp::XOR).map_err(err)? {
            info.full_faces |= 1 << direction as u8;
        }
        if !face.is_empty() {
            info.nonempty_faces |= 1 << direction as u8;
        }
    }
    let above = shape.face_shape(Direction::Down).map_err(err)?;
    let px = |v: f64| v / 16.0;
    let tests = [
        (7.0, 7.0, 9.0, 9.0),
        (7.0, 0.0, 9.0, 9.0),
        (7.0, 7.0, 16.0, 9.0),
        (7.0, 7.0, 9.0, 16.0),
        (0.0, 7.0, 9.0, 9.0),
    ];
    for (bit, (x0, z0, x1, z1)) in tests.into_iter().enumerate() {
        let test = VoxelShape::from_box(Aabb::new(
            Vec3::new(px(x0), 0.0, px(z0)),
            Vec3::new(px(x1), 1.0, px(z1)),
        ))
        .map_err(err)?;
        if !test
            .join_is_not_empty(&above, ShapeOp::ONLY_FIRST)
            .map_err(err)?
        {
            info.wall_cover |= 1 << bit;
        }
    }
    Ok(info)
}

/// Fluid tags as masks over `FluidKind::mask`.
#[derive(Clone, Copy, Debug, Default)]
pub struct FluidTags {
    pub water: u8,
    pub lava: u8,
    pub supports_frogspawn: u8,
    pub supports_lily_pad: u8,
    pub supports_sugar_cane_adjacently: u8,
    pub bubble_column_can_occupy: u8,
}

/// A set of blocks (a resolved block tag).
#[derive(Clone, Debug, Default)]
pub struct BlockSet(Vec<u64>);
impl BlockSet {
    pub fn with_capacity(blocks: usize) -> Self {
        Self(vec![0; blocks.div_ceil(64)])
    }
    pub fn insert(&mut self, block: BlockId) {
        let i = block.0 as usize;
        if i / 64 >= self.0.len() {
            self.0.resize(i / 64 + 1, 0);
        }
        self.0[i / 64] |= 1 << (i % 64);
    }
    pub fn contains(&self, block: BlockId) -> bool {
        let i = block.0 as usize;
        self.0.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }
}

#[derive(Clone, Debug)]
pub struct BlockData {
    pub class: Class,
    /// Recognised classes of the Java class chain, most derived first; ported
    /// overrides dispatch along it so `super` calls reach the next override.
    pub chain: Vec<Class>,
    pub classes: ClassSet,
    pub interfaces: u16,
    pub set_type: Option<Arc<SetType>>,
    pub gate_open: Option<Arc<str>>,
    pub gate_close: Option<Arc<str>>,
    pub open_sound: Option<Arc<str>>,
    pub close_sound: Option<Arc<str>>,
    pub ticks_to_stay_pressed: i32,
    pub max_weight: i32,
    /// Block-valued configuration: concrete (powder), fruit (stems), potted
    /// content, dead coral, stair base, candle (candle cakes), required nylium.
    pub link: Option<BlockId>,
    /// Second block link: attached stem (stems) or stem (attached stems).
    pub link2: Option<BlockId>,
    pub support_tag: Option<Arc<BlockSet>>,
    pub support_tag2: Option<Arc<BlockSet>>,
    pub feature: Option<Arc<str>>,
    pub grow_probability: f64,
    pub growth_direction: Direction,
    pub schedule_fluid_ticks: bool,
    pub weather: u8,
    pub ignite_odds: i32,
    pub burn_odds: i32,
    pub sounds: Arc<Sounds>,
    pub item: Arc<str>,
    pub dynamic_shape: bool,
}
impl BlockData {
    pub fn is(&self, class: Class) -> bool {
        self.class == class || self.classes.contains(class)
    }
    pub fn implements(&self, interface: Interface) -> bool {
        self.interfaces & interface.mask() != 0
    }
}

/// Item-to-block placement bindings (BlockItem and its subclasses).
#[derive(Clone, Debug)]
pub struct ItemBinding {
    pub block: BlockId,
    pub wall_block: Option<BlockId>,
    /// StandingAndWallBlockItem.attachmentDirection (DOWN for torches/signs, UP for hanging signs).
    pub attachment: Option<Direction>,
    pub kind: ItemKind,
    /// GameMasterBlockItem (command/structure/jigsaw blocks): only placeable
    /// by players that can use game master blocks.
    pub game_master: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Block,
    DoubleHigh,
    StandingAndWall,
    Scaffolding,
    PlaceOnWater,
    HangingSign,
    SolidBucket,
    Other,
}

macro_rules! named_blocks {
    ($($field:ident = $name:literal,)*) => {
        /// Blocks referenced by identity in vanilla code (`Blocks.X`).
        #[derive(Clone, Copy, Debug)]
        pub struct Named { $(pub $field: BlockId,)* }
        impl Named {
            pub(super) fn resolve(table: &StateTable) -> io::Result<Self> {
                Ok(Self { $($field: table.block_by_name(concat!("minecraft:", $name)).ok_or_else(|| invalid(concat!("missing block ", $name)))?,)* })
            }
        }
    };
}
named_blocks! {
    air = "air",
    cave_air = "cave_air",
    void_air = "void_air",
    water = "water",
    lava = "lava",
    stone = "stone",
    cobblestone = "cobblestone",
    obsidian = "obsidian",
    basalt = "basalt",
    soul_soil = "soul_soil",
    blue_ice = "blue_ice",
    ice = "ice",
    frosted_ice = "frosted_ice",
    snow = "snow",
    snow_block = "snow_block",
    powder_snow = "powder_snow",
    dirt = "dirt",
    grass_block = "grass_block",
    mycelium = "mycelium",
    podzol = "podzol",
    farmland = "farmland",
    dirt_path = "dirt_path",
    mud = "mud",
    clay = "clay",
    sponge = "sponge",
    wet_sponge = "wet_sponge",
    fire = "fire",
    soul_fire = "soul_fire",
    redstone_wire = "redstone_wire",
    comparator = "comparator",
    repeater = "repeater",
    redstone_torch = "redstone_torch",
    redstone_wall_torch = "redstone_wall_torch",
    redstone_block = "redstone_block",
    redstone_lamp = "redstone_lamp",
    observer = "observer",
    piston = "piston",
    sticky_piston = "sticky_piston",
    piston_head = "piston_head",
    moving_piston = "moving_piston",
    tnt = "tnt",
    magma_block = "magma_block",
    soul_sand = "soul_sand",
    bubble_column = "bubble_column",
    kelp = "kelp",
    kelp_plant = "kelp_plant",
    seagrass = "seagrass",
    tall_seagrass = "tall_seagrass",
    cactus = "cactus",
    cactus_flower = "cactus_flower",
    sugar_cane = "sugar_cane",
    bamboo = "bamboo",
    bamboo_sapling = "bamboo_sapling",
    scaffolding = "scaffolding",
    melon = "melon",
    pumpkin = "pumpkin",
    carved_pumpkin = "carved_pumpkin",
    jack_o_lantern = "jack_o_lantern",
    barrier = "barrier",
    cake = "cake",
    flower_pot = "flower_pot",
    cauldron = "cauldron",
    water_cauldron = "water_cauldron",
    lava_cauldron = "lava_cauldron",
    powder_snow_cauldron = "powder_snow_cauldron",
    dragon_egg = "dragon_egg",
    tripwire = "tripwire",
    tripwire_hook = "tripwire_hook",
    slime_block = "slime_block",
    honey_block = "honey_block",
    hopper = "hopper",
    note_block = "note_block",
    chorus_plant = "chorus_plant",
    chorus_flower = "chorus_flower",
    end_stone = "end_stone",
    vine = "vine",
    lily_pad = "lily_pad",
    small_dripleaf = "small_dripleaf",
    big_dripleaf = "big_dripleaf",
    big_dripleaf_stem = "big_dripleaf_stem",
    pointed_dripstone = "pointed_dripstone",
    ladder = "ladder",
    sweet_berry_bush = "sweet_berry_bush",
    torchflower = "torchflower",
    pitcher_plant = "pitcher_plant",
    pitcher_crop = "pitcher_crop",
    moss_block = "moss_block",
    short_grass = "short_grass",
    tall_grass = "tall_grass",
    fern = "fern",
    large_fern = "large_fern",
    dead_bush = "dead_bush",
    netherrack = "netherrack",
    soul_campfire = "soul_campfire",
    campfire = "campfire",
    light = "light",
    sculk_sensor = "sculk_sensor",
    calibrated_sculk_sensor = "calibrated_sculk_sensor",
    sculk_shrieker = "sculk_shrieker",
    sculk_catalyst = "sculk_catalyst",
    sculk = "sculk",
    sculk_vein = "sculk_vein",
    lectern = "lectern",
    target = "target",
    daylight_detector = "daylight_detector",
    lever = "lever",
    beehive = "beehive",
    bee_nest = "bee_nest",
    composter = "composter",
    respawn_anchor = "respawn_anchor",
    end_portal_frame = "end_portal_frame",
    crafter = "crafter",
    copper_bulb = "copper_bulb",
    glow_lichen = "glow_lichen",
    hanging_roots = "hanging_roots",
    mangrove_roots = "mangrove_roots",
    big_dripleaf_fallback = "big_dripleaf",
    nether_portal = "nether_portal",
    chain_command_block = "chain_command_block",
    frogspawn = "frogspawn",
    turtle_egg = "turtle_egg",
    sniffer_egg = "sniffer_egg",
    sea_pickle = "sea_pickle",
    coarse_dirt = "coarse_dirt",
    rooted_dirt = "rooted_dirt",
    crimson_nylium = "crimson_nylium",
    warped_nylium = "warped_nylium",
}

macro_rules! tags {
    ($($field:ident = $name:literal,)*) => {
        /// Block tags consulted by ported behaviour.
        #[derive(Clone, Debug, Default)]
        pub struct Tags { $(pub $field: Arc<BlockSet>,)* }
        impl Tags {
            pub(super) fn resolve(all: &HashMap<String, Arc<BlockSet>>) -> Self {
                Self { $($field: all.get(concat!("minecraft:", $name)).cloned().unwrap_or_default(),)* }
            }
        }
    };
}
tags! {
    leaves = "leaves",
    logs = "logs",
    prevents_nearby_leaf_decay = "prevents_nearby_leaf_decay",
    walls = "walls",
    fences = "fences",
    wooden_fences = "wooden_fences",
    fence_gates = "fence_gates",
    stairs = "stairs",
    slabs = "slabs",
    doors = "doors",
    trapdoors = "trapdoors",
    rails = "rails",
    beds = "beds",
    wall_post_override = "wall_post_override",
    replaceable = "replaceable",
    dirt = "dirt",
    sand = "sand",
    ice = "ice",
    snow = "snow",
    infiniburn_overworld = "infiniburn_overworld",
    infiniburn_nether = "infiniburn_nether",
    infiniburn_end = "infiniburn_end",
    soul_fire_base_blocks = "soul_fire_base_blocks",
    washed_away_by_fluids = "washed_away_by_fluids",
    blocks_fluid_flow = "blocks_fluid_flow",
    blocks_motion = "blocks_motion",
    unstable_bottom_center = "unstable_bottom_center",
    cannot_support_snow_layer = "cannot_support_snow_layer",
    support_override_snow_layer = "support_override_snow_layer",
    maintains_farmland = "maintains_farmland",
    supports_vegetation = "supports_vegetation",
    supports_crops = "supports_crops",
    supports_cactus = "supports_cactus",
    supports_sugar_cane = "supports_sugar_cane",
    supports_sugar_cane_adjacently = "supports_sugar_cane_adjacently",
    supports_bamboo = "supports_bamboo",
    supports_cocoa = "supports_cocoa",
    supports_azalea = "supports_azalea",
    supports_dry_vegetation = "supports_dry_vegetation",
    supports_nether_sprouts = "supports_nether_sprouts",
    supports_nether_wart = "supports_nether_wart",
    supports_wither_rose = "supports_wither_rose",
    supports_lily_pad = "supports_lily_pad",
    supports_frogspawn = "supports_frogspawn",
    supports_big_dripleaf = "supports_big_dripleaf",
    supports_small_dripleaf = "supports_small_dripleaf",
    supports_mangrove_propagule = "supports_mangrove_propagule",
    supports_hanging_mangrove_propagule = "supports_hanging_mangrove_propagule",
    supports_chorus_plant = "supports_chorus_plant",
    supports_chorus_flower = "supports_chorus_flower",
    support_override_cactus_flower = "support_override_cactus_flower",
    cannot_support_seagrass = "cannot_support_seagrass",
    cannot_support_kelp = "cannot_support_kelp",
    overrides_mushroom_light_requirement = "overrides_mushroom_light_requirement",
    enables_bubble_column_push_up = "enables_bubble_column_push_up",
    enables_bubble_column_drag_down = "enables_bubble_column_drag_down",
    shulker_boxes = "shulker_boxes",
    pale_oak_logs = "pale_oak_logs",
    convertible_to_mud = "convertible_to_mud",
    grows_crops = "grows_crops",
    candles = "candles",
    candle_cakes = "candle_cakes",
    campfires = "campfires",
    buttons = "buttons",
    pressure_plates = "pressure_plates",
    wool = "wool",
    crops = "crops",
    small_flowers = "small_flowers",
    flowers = "flowers",
    saplings = "saplings",
    anvil = "anvil",
    lanterns = "lanterns",
    climbable = "climbable",
    nylium = "nylium",
    mushroom_grow_block = "mushroom_grow_block",
    guarded_by_piglins = "guarded_by_piglins",
    copper = "copper",
    copper_chests = "copper_chests",
    copper_golem_statues = "copper_golem_statues",
}

/// Property keys interned at load.
macro_rules! props {
    ($($field:ident = $name:literal,)*) => {
        #[derive(Clone, Copy, Debug)]
        pub struct Props { $(pub $field: PropKey,)* }
        impl Props {
            pub(super) fn resolve(table: &mut StateTable) -> Self {
                Self { $($field: table.key($name),)* }
            }
        }
    };
}
props! {
    waterlogged = "waterlogged",
    facing = "facing",
    half = "half",
    shape = "shape",
    open = "open",
    powered = "powered",
    hinge = "hinge",
    in_wall = "in_wall",
    axis = "axis",
    age = "age",
    level = "level",
    layers = "layers",
    snowy = "snowy",
    north = "north",
    south = "south",
    east = "east",
    west = "west",
    up = "up",
    down = "down",
    distance = "distance",
    persistent = "persistent",
    moisture = "moisture",
    face = "face",
    lit = "lit",
    power = "power",
    delay = "delay",
    locked = "locked",
    mode = "mode",
    triggered = "triggered",
    extended = "extended",
    kind = "type",
    part = "part",
    occupied = "occupied",
    hanging = "hanging",
    attached = "attached",
    candles = "candles",
    pickles = "pickles",
    eggs = "eggs",
    hatch = "hatch",
    note = "note",
    instrument = "instrument",
    bites = "bites",
    rotation = "rotation",
    stage = "stage",
    leaves = "leaves",
    bottom = "bottom",
    falling = "falling",
    drag = "drag",
    tilt = "tilt",
    attachment = "attachment",
    vertical_direction = "vertical_direction",
    thickness = "thickness",
    berries = "berries",
    flower_amount = "flower_amount",
    segment_amount = "segment_amount",
    charges = "charges",
    eye = "eye",
    has_bottle_0 = "has_bottle_0",
    disarmed = "disarmed",
    inverted = "inverted",
    enabled = "enabled",
    conditional = "conditional",
    short = "short",
    unstable = "unstable",
    tip = "tip",
    honey_level = "honey_level",
    bloom = "bloom",
    natural = "natural",
    creaking_heart_state = "creaking_heart_state",
    cracked = "cracked",
    has_book = "has_book",
    has_record = "has_record",
    signal_fire = "signal_fire",
    orientation = "orientation",
    crafting = "crafting",
    side_chain = "side_chain",
    slot_0_occupied = "slot_0_occupied",
    slot_1_occupied = "slot_1_occupied",
    slot_2_occupied = "slot_2_occupied",
    slot_3_occupied = "slot_3_occupied",
    slot_4_occupied = "slot_4_occupied",
    slot_5_occupied = "slot_5_occupied",
    copper_golem_pose = "copper_golem_pose",
    sculk_sensor_phase = "sculk_sensor_phase",
    shrieking = "shrieking",
    can_summon = "can_summon",
    potent_sulfur_state = "potent_sulfur_state",
}

fn fail(message: &str) -> io::Error {
    invalid(message)
}

/// From the file's bytes in memory: several times faster than through a
/// reader on gameplay-facts.json (55 MB), which a first start parses.
fn read_json(path: &Path) -> io::Result<Value> {
    serde_json::from_slice(&fs::read(path)?).map_err(|e| invalid(e.to_string()))
}

fn interned(cache: &mut HashMap<String, Arc<str>>, text: &str) -> Arc<str> {
    cache
        .entry(text.to_owned())
        .or_insert_with(|| Arc::from(text))
        .clone()
}

fn set_type(value: &Value, cache: &mut HashMap<String, Arc<str>>) -> Option<SetType> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    Some(SetType {
        name: value.get("name")?.as_str()?.into(),
        can_open_by_hand: value.get("canOpenByHand")?.as_bool()?,
        can_open_by_wind_charge: value.get("canOpenByWindCharge")?.as_bool()?,
        arrows: value.get("canButtonBeActivatedByArrows")?.as_bool()?,
        mobs_only: value.get("pressurePlateSensitivity")?.as_str()? == "MOBS",
        door_open: interned(cache, text("doorOpen")),
        door_close: interned(cache, text("doorClose")),
        trapdoor_open: interned(cache, text("trapdoorOpen")),
        trapdoor_close: interned(cache, text("trapdoorClose")),
        plate_on: interned(cache, text("pressurePlateClickOn")),
        plate_off: interned(cache, text("pressurePlateClickOff")),
        button_on: interned(cache, text("buttonClickOn")),
        button_off: interned(cache, text("buttonClickOff")),
    })
}

/// Build the block-state registry from the resources' `facts.json` (the
/// same source the native client uses), streamed.
pub fn load_registry(root: &Path) -> io::Result<BlockRegistry> {
    BlockRegistry::from_facts_json(File::open(root.join("facts.json"))?)
}

pub(super) fn load(registry: &BlockRegistry, root: &Path) -> io::Result<Behaviours> {
    cached(registry, root, || load_uncached(registry, root))
}

/// [`load`] with `block-behaviours.json` already parsed by the caller.
pub(super) fn load_with_behaviour_json(
    registry: &BlockRegistry,
    root: &Path,
    behaviour: &Value,
) -> io::Result<Behaviours> {
    cached(registry, root, || {
        build_with_behaviour_json(registry, root, behaviour)
    })
}

/// [`load`] with the client's typed gameplay projection. The snapshot is
/// still tried first: on a hit the projection is not needed.
pub(super) fn load_with_gameplay<G: GameplayStateSource>(
    registry: &BlockRegistry,
    root: &Path,
    behaviour: &Value,
    gameplay_states: &[G],
) -> io::Result<Behaviours> {
    cached(registry, root, || {
        build_with_gameplay(registry, root, behaviour, gameplay_states)
    })
}

/// The snapshot when it matches, else `build` and refresh the snapshot.
fn cached(
    registry: &BlockRegistry,
    root: &Path,
    build: impl FnOnce() -> io::Result<Behaviours>,
) -> io::Result<Behaviours> {
    if let Some(behaviours) = cache::load(registry, root) {
        return Ok(behaviours);
    }
    let behaviours = build()?;
    cache::store(registry, root, &behaviours);
    Ok(behaviours)
}

fn load_uncached(registry: &BlockRegistry, root: &Path) -> io::Result<Behaviours> {
    let behaviour = read_json(&root.join("block-behaviours.json"))?;
    build_with_behaviour_json(registry, root, &behaviour)
}

fn build_with_behaviour_json(
    registry: &BlockRegistry,
    root: &Path,
    behaviour: &Value,
) -> io::Result<Behaviours> {
    let gameplay = read_json(&root.join("gameplay-facts.json"))?;
    let gameplay_states = gameplay["states"]
        .as_array()
        .ok_or_else(|| fail("missing gameplay states"))?;
    build_with_gameplay(registry, root, behaviour, gameplay_states)
}

fn build_with_gameplay<G: GameplayStateSource>(
    registry: &BlockRegistry,
    root: &Path,
    behaviour: &Value,
    gameplay_states: &[G],
) -> io::Result<Behaviours> {
    let mut table = StateTable::new(registry)?;
    let props = Props::resolve(&mut table);
    let named = Named::resolve(&table)?;
    let blocks_json = behaviour["blocks"]
        .as_array()
        .ok_or_else(|| fail("missing behaviour blocks"))?;
    let states_json = behaviour["states"]
        .as_array()
        .ok_or_else(|| fail("missing behaviour states"))?;
    if blocks_json.len() != table.block_count()
        || states_json.len() != table.state_count()
        || gameplay_states.len() != table.state_count()
    {
        return Err(fail("behaviour exports do not match the block registry"));
    }
    // Fluid registry order fixes the fluid tick type IDs.
    let fluids: Vec<&str> = behaviour["fluids"]
        .as_array()
        .ok_or_else(|| fail("missing fluid registry"))?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    if fluids
        .iter()
        .copied()
        .ne(FluidKind::ALL.iter().map(|k| k.name()))
    {
        return Err(fail("fluid registry order differs from Horizon's"));
    }

    // Tags.
    let bundle = read_json(&root.join("block-tags.json"))?;
    let mut definitions = TagDefinitions::default();
    for (name, text) in bundle
        .as_object()
        .ok_or_else(|| fail("invalid block tag bundle"))?
    {
        definitions
            .apply_json(
                name,
                "vanilla",
                text.as_str()
                    .ok_or_else(|| fail("invalid block tag text"))?,
            )
            .map_err(|e| invalid(e.to_string()))?;
    }
    let built = definitions
        .build(|name, _| table.block_by_name(name).map(|_| name.to_owned()))
        .map_err(|e| invalid(e.to_string()))?;
    let mut all_tags = HashMap::new();
    for (name, members) in &built.tags {
        let mut set = BlockSet::with_capacity(table.block_count());
        for member in members {
            if let Some(block) = table.block_by_name(member) {
                set.insert(block);
            }
        }
        all_tags.insert(name.clone(), Arc::new(set));
    }
    let tags = Tags::resolve(&all_tags);
    let tag = |name: &str| -> Option<Arc<BlockSet>> {
        all_tags
            .get(name.strip_prefix('#').unwrap_or(name))
            .cloned()
    };

    // Collision shapes.
    let collision_file = BlockTableFile::open(&root.join("cached-collision.nbt"), registry)?;
    let cached = CachedCollisionRegistry::from_table(collision_file.table("collision")?)?;
    let mut shapes: Vec<VoxelShape> = Vec::new();
    let mut shape_ids = Vec::with_capacity(table.state_count());
    for id in 0..table.state_count() {
        let Some(shape) = cached.cached_shape(BlockStateId(id as u32))? else {
            shape_ids.push(-1);
            continue;
        };
        let index = match shapes.iter().position(|s| s.same_identity(shape)) {
            Some(index) => index,
            None => {
                shapes.push(shape.clone());
                shapes.len() - 1
            }
        };
        shape_ids.push(index as i32);
    }

    let shape_info = shapes
        .iter()
        .map(analyse_shape)
        .collect::<io::Result<Vec<_>>>()?;

    // Fluid tags.
    let mut fluid_definitions = TagDefinitions::default();
    let fluid_dir = root.join("data/minecraft/tags/fluid");
    for entry in fs::read_dir(&fluid_dir)? {
        let path = entry?.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        fluid_definitions
            .apply_json(
                &format!("minecraft:{stem}"),
                "vanilla",
                &fs::read_to_string(&path)?,
            )
            .map_err(|e| invalid(e.to_string()))?;
    }
    let fluid_built = fluid_definitions
        .build(|name, _| FluidKind::from_name(name).map(|kind| kind as u8))
        .map_err(|e| invalid(e.to_string()))?;
    let fluid_mask = |name: &str| -> u8 {
        fluid_built
            .tags
            .get(name)
            .into_iter()
            .flatten()
            .fold(0, |mask, kind| mask | 1 << kind)
    };
    let fluid_tags = FluidTags {
        water: fluid_mask("minecraft:water"),
        lava: fluid_mask("minecraft:lava"),
        supports_frogspawn: fluid_mask("minecraft:supports_frogspawn"),
        supports_lily_pad: fluid_mask("minecraft:supports_lily_pad"),
        supports_sugar_cane_adjacently: fluid_mask("minecraft:supports_sugar_cane_adjacently"),
        bubble_column_can_occupy: fluid_mask("minecraft:bubble_column_can_occupy"),
    };

    // Per-state facts.
    let mut facts = Vec::with_capacity(table.state_count());
    for (id, (exported, game)) in states_json.iter().zip(gameplay_states).enumerate() {
        let row = exported
            .as_array()
            .ok_or_else(|| fail("invalid behaviour state row"))?;
        let int = |i: usize| row.get(i).and_then(Value::as_i64).unwrap_or(0);
        let gb = |key: &str| game.flag(key);
        let mut flags = int(1) as u32 & 0x7f;
        for (key, bit) in [
            ("replaceable", state_flags::REPLACEABLE),
            ("random_ticks", state_flags::RANDOM_TICKS),
            ("solid_render", state_flags::SOLID_RENDER),
            ("liquid", state_flags::LIQUID),
            ("block_entity", state_flags::BLOCK_ENTITY),
            ("ignited_by_lava", state_flags::IGNITED_BY_LAVA),
            ("can_occlude", state_flags::CAN_OCCLUDE),
            ("full_collision", state_flags::FULL_COLLISION),
            ("skylight_down", state_flags::SKYLIGHT_DOWN),
            ("requires_tool", state_flags::REQUIRES_TOOL),
            ("shape_occlusion", state_flags::SHAPE_OCCLUSION),
        ] {
            if gb(key) {
                flags |= bit;
            }
        }
        let fluid_kind = FluidKind::ALL
            .get(int(2) as usize)
            .copied()
            .ok_or_else(|| fail("invalid exported fluid"))?;
        let fluid = FluidState::new(
            fluid_kind,
            int(3) as u8,
            row.get(4).and_then(Value::as_bool).unwrap_or(false),
        );
        facts.push(StateFacts {
            flags,
            sturdy: int(0) as u32,
            weak: int(5) as u32,
            strong: int(6) as u32,
            fluid,
            emission: game.unsigned("emission") as u8,
            dampening: game.unsigned("dampening") as u8,
            hardness: game.number("hardness") as f32,
            push: match game.text("push") {
                "PUSH" => PushReaction::Push,
                "POPPED" => PushReaction::Popped,
                "IMMOVEABLE" => PushReaction::Immoveable,
                "IGNORE_ENTITY" => PushReaction::IgnoreEntity,
                _ => PushReaction::PushPull,
            },
            instrument: instrument_index(row.get(8).and_then(Value::as_str).unwrap_or("HARP")),
            shape: shape_ids[id],
        });
    }

    // Per-block data.
    let fire = &behaviour["fire"];
    let odds = |key: &str, name: &str| -> i32 {
        fire.get(key)
            .and_then(|m| m.get(name))
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32
    };
    let mut strings = HashMap::new();
    let mut blocks = Vec::with_capacity(table.block_count());
    for (index, json) in blocks_json.iter().enumerate() {
        let block = BlockId(index as u16);
        let name = json["name"]
            .as_str()
            .ok_or_else(|| fail("block without name"))?;
        if name != &*table.info(block).name {
            return Err(fail("behaviour block order differs from registry"));
        }
        let mut classes = ClassSet::default();
        let mut chain = Vec::new();
        for java in json["class"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(c) = Class::from_java(java) {
                classes.insert(c);
                chain.push(c);
            }
        }
        let class = chain.first().copied().unwrap_or(Class::Block);
        let mut interfaces = 0;
        for java in json["interfaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(i) = Interface::from_java(java) {
                interfaces |= i.mask();
            }
        }
        let fields = &json["fields"];
        let field = |key: &str| fields.get(key);
        let block_field = |key: &str| {
            field(key)
                .and_then(Value::as_str)
                .and_then(|n| table.block_by_name(n))
        };
        let wood = field("type").filter(|t| t.get("setType").is_some());
        let set_type = match wood {
            Some(wood) => set_type(&wood["setType"], &mut strings),
            None => field("type").and_then(|t| set_type(t, &mut strings)),
        }
        .map(Arc::new);
        let text = |value: Option<&Value>, strings: &mut HashMap<String, Arc<str>>| {
            value.and_then(Value::as_str).map(|s| interned(strings, s))
        };
        let gate_open = text(wood.and_then(|w| w.get("fenceGateOpen")), &mut strings);
        let gate_close = text(wood.and_then(|w| w.get("fenceGateClose")), &mut strings);
        let open_sound = text(field("openSound"), &mut strings);
        let close_sound = text(field("closeSound"), &mut strings);
        let link = [
            "concrete",
            "fruit",
            "potted",
            "deadBlock",
            "base",
            "candleBlock",
            "requiredBlock",
            "baseBlock",
        ]
        .into_iter()
        .find_map(block_field)
        .or_else(|| {
            let state = field("blockToGrowOn")?.get("state")?.as_u64()?;
            (state < table.state_count() as u64).then(|| table.block(BlockStateId(state as u32)))
        });
        let link2 = ["attachedStem", "stem"].into_iter().find_map(block_field);
        let support_tag = ["stemSupportBlocks", "supportBlocks"]
            .into_iter()
            .find_map(|k| field(k).and_then(Value::as_str))
            .and_then(tag);
        let support_tag2 = field("fruitSupportBlocks")
            .and_then(Value::as_str)
            .and_then(tag);
        let feature = field("feature")
            .and_then(Value::as_str)
            .or_else(|| {
                field("treeGrower")
                    .and_then(|g| g.get("name"))
                    .and_then(Value::as_str)
            })
            .map(|s| interned(&mut strings, s));
        let growth_direction = match field("growthDirection").and_then(Value::as_str) {
            Some("DOWN") => Direction::Down,
            _ => Direction::Up,
        };
        let default = table.default_state(block);
        let sound = &gameplay_states[default.0 as usize];
        let sounds = Arc::new(Sounds {
            volume: sound.sound_number("volume") as f32,
            pitch: sound.sound_number("pitch") as f32,
            place: interned(&mut strings, sound.sound_id("place")),
            break_: interned(&mut strings, sound.sound_id("break")),
            fall: interned(&mut strings, sound.sound_id("fall")),
        });
        blocks.push(BlockData {
            class,
            chain,
            classes,
            interfaces,
            set_type,
            gate_open,
            gate_close,
            open_sound,
            close_sound,
            ticks_to_stay_pressed: field("ticksToStayPressed")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32,
            max_weight: field("maxWeight").and_then(Value::as_i64).unwrap_or(0) as i32,
            link,
            link2,
            support_tag,
            support_tag2,
            feature,
            grow_probability: field("growPerTickProbability")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            growth_direction,
            schedule_fluid_ticks: field("scheduleFluidTicks")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            // `WeatheringCopper.getAge`: copper golem statues name the field
            // `weatheringState`.
            weather: match field("weatherState")
                .or_else(|| field("weatheringState"))
                .and_then(Value::as_str)
            {
                Some("EXPOSED") => 1,
                Some("WEATHERED") => 2,
                Some("OXIDIZED") => 3,
                _ => 0,
            },
            ignite_odds: odds("igniteOdds", name),
            burn_odds: odds("burnOdds", name),
            sounds,
            item: interned(
                &mut strings,
                json["item"].as_str().unwrap_or("minecraft:air"),
            ),
            dynamic_shape: json["dynamic_shape"].as_bool().unwrap_or(false),
        });
    }

    // Item bindings.
    let mut items = HashMap::new();
    for (item, binding) in behaviour["items"]
        .as_object()
        .ok_or_else(|| fail("missing item bindings"))?
    {
        let fields = &binding["fields"];
        let Some(block) = fields
            .get("block")
            .and_then(Value::as_str)
            .and_then(|n| table.block_by_name(n))
        else {
            continue;
        };
        let classes: Vec<&str> = binding["class"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let has = |name: &str| classes.contains(&name);
        let kind = if has("HangingSignItem") {
            ItemKind::HangingSign
        } else if has("StandingAndWallBlockItem") {
            ItemKind::StandingAndWall
        } else if has("DoubleHighBlockItem") {
            ItemKind::DoubleHigh
        } else if has("ScaffoldingBlockItem") {
            ItemKind::Scaffolding
        } else if has("PlaceOnWaterBlockItem") {
            ItemKind::PlaceOnWater
        } else if has("SolidBucketItem") {
            ItemKind::SolidBucket
        } else if has("BlockItem") {
            ItemKind::Block
        } else {
            ItemKind::Other
        };
        items.insert(
            item.clone(),
            ItemBinding {
                block,
                wall_block: fields
                    .get("wallBlock")
                    .and_then(Value::as_str)
                    .and_then(|n| table.block_by_name(n)),
                attachment: match fields.get("attachmentDirection").and_then(Value::as_str) {
                    Some("UP") => Some(Direction::Up),
                    Some("DOWN") => Some(Direction::Down),
                    _ => None,
                },
                kind,
                game_master: has("GameMasterBlockItem"),
            },
        );
    }

    let liquid_blocks = FluidBlockBindings::new(registry)?;
    let mut dynamic = HashMap::new();
    for (id, fact) in facts.iter().enumerate() {
        if fact.shape < 0 {
            let state = BlockStateId(id as u32);
            if let Some(support) =
                crate::dynamic_support::DynamicSupport::from_state(registry.state(state).unwrap())?
            {
                dynamic.insert(state, support);
            }
        }
    }
    Ok(Behaviours {
        table,
        props,
        named,
        tags,
        all_tags,
        facts,
        shapes,
        shape_info,
        fluid_tags,
        blocks,
        items,
        liquid_blocks,
        dynamic,
        occlusion_cache: Default::default(),
        growth: Default::default(),
    })
}

/// `NoteBlockInstrument` serialized names in declaration order.
pub const INSTRUMENTS: [&str; 27] = [
    "harp",
    "basedrum",
    "snare",
    "hat",
    "bass",
    "flute",
    "bell",
    "guitar",
    "chime",
    "xylophone",
    "iron_xylophone",
    "cow_bell",
    "didgeridoo",
    "bit",
    "banjo",
    "pling",
    "trumpet",
    "trumpet_exposed",
    "trumpet_oxidized",
    "trumpet_weathered",
    "zombie",
    "skeleton",
    "creeper",
    "dragon",
    "wither_skeleton",
    "piglin",
    "custom_head",
];
/// `NoteBlockInstrument.worksAboveNoteBlock` (mob heads and custom heads).
pub fn instrument_works_above(index: u8) -> bool {
    index >= 20
}
fn instrument_index(name: &str) -> u8 {
    INSTRUMENTS
        .iter()
        .position(|n| n.eq_ignore_ascii_case(name))
        .map_or(0, |i| i as u8)
}

impl GameplayStateSource for Value {
    fn flag(&self, name: &str) -> bool {
        self.get(name).and_then(Value::as_bool).unwrap_or(false)
    }
    fn unsigned(&self, name: &str) -> u64 {
        self.get(name).and_then(Value::as_u64).unwrap_or(0)
    }
    fn number(&self, name: &str) -> f64 {
        self.get(name).and_then(Value::as_f64).unwrap_or(0.0)
    }
    fn text(&self, name: &str) -> &str {
        self.get(name).and_then(Value::as_str).unwrap_or("")
    }
    fn sound_number(&self, name: &str) -> f64 {
        self.get("sound")
            .and_then(|s| s.get(name))
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
    }
    fn sound_id(&self, name: &str) -> &str {
        self.get("sound")
            .and_then(|s| s.get(name))
            .and_then(|s| s.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
    }
}
