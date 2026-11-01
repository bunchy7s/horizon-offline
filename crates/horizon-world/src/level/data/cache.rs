//! A disposable, versioned snapshot of resolved behaviour tables.
//!
//! The source resources remain authoritative. Failed reads, unsupported
//! versions, corrupt payloads and write failures always use the normal loader.
//! The snapshot lives in the game's own cache folder ([`set_dir`]), never in the
//! resource folder: that one is shared by worktrees and copied between installed
//! versions by the launcher. Its key includes the running executable, so a build
//! that resolves the tables differently never reads another build's snapshot.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8; 8] = b"HZBEHAV\0";
const VERSION: u32 = 2;
const FILE_NAME: &str = "behaviours-v2.bin";
const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Keep snapshots in `dir` (the client passes `<game folder>/.horizon-cache`).
/// Without it (tests, tools, servers), `Behaviours::load` always uses the source loader.
pub(in crate::level) fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    tags: HashMap<String, Vec<u64>>,
    facts: Vec<Fact>,
    shapes: Vec<Shape>,
    shape_info: Vec<[u8; 3]>,
    fluid_tags: [u8; 6],
    blocks: Vec<Block>,
    items: HashMap<String, Item>,
}

#[derive(Serialize, Deserialize)]
struct Fact {
    flags: u32,
    sturdy: u32,
    weak: u32,
    strong: u32,
    fluid_kind: u8,
    fluid_amount: u8,
    fluid_falling: bool,
    emission: u8,
    dampening: u8,
    hardness: f32,
    push: u8,
    instrument: u8,
    shape: i32,
}

#[derive(Serialize, Deserialize)]
struct Shape {
    coords: [Vec<f64>; 3],
    occupied: Vec<bool>,
    cube: bool,
    coordinate_cubes: [bool; 3],
    min: [i32; 3],
    max: [i32; 3],
    /// Preserve the shared singleton identities used by vanilla shape shortcuts.
    singleton: u8,
}

#[derive(Serialize, Deserialize)]
struct SetTypeSnapshot {
    name: String,
    can_open_by_hand: bool,
    can_open_by_wind_charge: bool,
    arrows: bool,
    mobs_only: bool,
    door_open: String,
    door_close: String,
    trapdoor_open: String,
    trapdoor_close: String,
    plate_on: String,
    plate_off: String,
    button_on: String,
    button_off: String,
}

#[derive(Serialize, Deserialize)]
struct SoundsSnapshot {
    volume: f32,
    pitch: f32,
    place: String,
    break_: String,
    fall: String,
}

#[derive(Serialize, Deserialize)]
struct Block {
    chain: Vec<u16>,
    interfaces: u16,
    set_type: Option<SetTypeSnapshot>,
    gate_open: Option<String>,
    gate_close: Option<String>,
    open_sound: Option<String>,
    close_sound: Option<String>,
    ticks_to_stay_pressed: i32,
    max_weight: i32,
    link: Option<u16>,
    link2: Option<u16>,
    support_tag: Option<Vec<u64>>,
    support_tag2: Option<Vec<u64>>,
    feature: Option<String>,
    grow_probability: f64,
    growth_direction: u8,
    schedule_fluid_ticks: bool,
    weather: u8,
    ignite_odds: i32,
    burn_odds: i32,
    sounds: SoundsSnapshot,
    item: String,
    dynamic_shape: bool,
}

#[derive(Serialize, Deserialize)]
struct Item {
    block: u16,
    wall_block: Option<u16>,
    attachment: Option<u8>,
    kind: u8,
    game_master: bool,
}

fn fingerprint(registry: &BlockRegistry, root: &Path) -> io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    hasher.update(b"horizon resolved behaviours 2");
    // The build that resolved the tables: the version constant alone misses
    // loader changes that keep the snapshot layout.
    let exe = std::env::current_exe()?;
    let built = fs::metadata(&exe)?;
    hash_str(&mut hasher, &exe.to_string_lossy());
    hasher.update(built.len().to_le_bytes());
    let modified = built
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| invalid(e.to_string()))?;
    hasher.update(modified.as_nanos().to_le_bytes());
    hasher.update(sources_digest(root)?);
    // The registry argument may be modified independently of facts.json.
    hasher.update((registry.len() as u64).to_le_bytes());
    let mut seen = std::collections::HashSet::new();
    for index in 0..registry.len() {
        let state = registry
            .state(BlockStateId(index as u32))
            .ok_or_else(|| invalid("missing registry state"))?;
        hash_str(&mut hasher, &state.name);
        let mut props: Vec<_> = state.properties.iter().collect();
        props.sort_by_key(|(key, _)| *key);
        hasher.update((props.len() as u32).to_le_bytes());
        for (key, value) in props {
            hash_str(&mut hasher, key);
            hash_str(&mut hasher, value);
        }
        if seen.insert(state.name.as_str()) {
            let default = registry
                .default_state(&state.name)
                .ok_or_else(|| invalid("missing default state"))?;
            hasher.update(default.0.to_le_bytes());
        }
    }
    Ok(hasher.finalize().into())
}

/// The source files the snapshot is resolved from, as their lengths and
/// modification times, and the hash of their contents.
type Sources = (Vec<(PathBuf, u64, Option<SystemTime>)>, [u8; 32]);
/// Source hashes already computed by this process: the client's tables and
/// the world generator's are loaded from the same folder, and hashing its
/// 70 MB again took a third of a second on a 4-thread laptop. A file whose
/// length or modification time changed is read again.
static SOURCES: Mutex<Vec<Sources>> = Mutex::new(Vec::new());

fn sources_digest(root: &Path) -> io::Result<[u8; 32]> {
    let mut paths: Vec<PathBuf> = [
        "facts.json",
        "block-behaviours.json",
        "gameplay-facts.json",
        "block-tags.json",
        "cached-collision.nbt",
    ]
    .iter()
    .map(|relative| root.join(relative))
    .collect();
    let mut fluids: Vec<PathBuf> = fs::read_dir(root.join("data/minecraft/tags/fluid"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<_>>()?;
    fluids.sort();
    paths.extend(fluids);
    let stamps = paths
        .iter()
        .map(|path| {
            let meta = fs::metadata(path)?;
            Ok((path.clone(), meta.len(), meta.modified().ok()))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let known = SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, digest)) = known.iter().find(|(s, _)| *s == stamps) {
        return Ok(*digest);
    }
    drop(known);
    let mut hasher = Sha256::new();
    for path in &paths {
        hash_file(&mut hasher, root, path)?;
    }
    let digest: [u8; 32] = hasher.finalize().into();
    SOURCES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((stamps, digest));
    Ok(digest)
}

fn hash_str(hasher: &mut Sha256, text: &str) {
    hasher.update((text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
}

fn hash_file(hasher: &mut Sha256, root: &Path, path: &Path) -> io::Result<()> {
    let relative = path
        .strip_prefix(root)
        .map_err(|e| invalid(e.to_string()))?;
    hash_str(hasher, &relative.to_string_lossy());
    let bytes = fs::read(path)?;
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(())
}

fn direction(index: u8) -> io::Result<Direction> {
    Direction::ALL
        .get(index as usize)
        .copied()
        .ok_or_else(|| invalid("cached direction out of range"))
}
fn direction_index(direction: Direction) -> u8 {
    Direction::ALL.iter().position(|d| *d == direction).unwrap() as u8
}
fn class(index: u16) -> io::Result<Class> {
    Class::ALL
        .get(index as usize)
        .copied()
        .ok_or_else(|| invalid("cached block class out of range"))
}
fn kind(index: u8) -> io::Result<ItemKind> {
    Ok(match index {
        0 => ItemKind::Block,
        1 => ItemKind::DoubleHigh,
        2 => ItemKind::StandingAndWall,
        3 => ItemKind::Scaffolding,
        4 => ItemKind::PlaceOnWater,
        5 => ItemKind::HangingSign,
        6 => ItemKind::SolidBucket,
        7 => ItemKind::Other,
        _ => return Err(invalid("cached item kind out of range")),
    })
}
fn kind_index(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::Block => 0,
        ItemKind::DoubleHigh => 1,
        ItemKind::StandingAndWall => 2,
        ItemKind::Scaffolding => 3,
        ItemKind::PlaceOnWater => 4,
        ItemKind::HangingSign => 5,
        ItemKind::SolidBucket => 6,
        ItemKind::Other => 7,
    }
}
fn push(index: u8) -> io::Result<PushReaction> {
    Ok(match index {
        0 => PushReaction::PushPull,
        1 => PushReaction::Push,
        2 => PushReaction::Popped,
        3 => PushReaction::Immoveable,
        4 => PushReaction::IgnoreEntity,
        _ => return Err(invalid("cached push reaction out of range")),
    })
}
fn push_index(push: PushReaction) -> u8 {
    match push {
        PushReaction::PushPull => 0,
        PushReaction::Push => 1,
        PushReaction::Popped => 2,
        PushReaction::Immoveable => 3,
        PushReaction::IgnoreEntity => 4,
    }
}

impl From<&SetType> for SetTypeSnapshot {
    fn from(value: &SetType) -> Self {
        Self {
            name: value.name.to_string(),
            can_open_by_hand: value.can_open_by_hand,
            can_open_by_wind_charge: value.can_open_by_wind_charge,
            arrows: value.arrows,
            mobs_only: value.mobs_only,
            door_open: value.door_open.to_string(),
            door_close: value.door_close.to_string(),
            trapdoor_open: value.trapdoor_open.to_string(),
            trapdoor_close: value.trapdoor_close.to_string(),
            plate_on: value.plate_on.to_string(),
            plate_off: value.plate_off.to_string(),
            button_on: value.button_on.to_string(),
            button_off: value.button_off.to_string(),
        }
    }
}
impl From<SetTypeSnapshot> for SetType {
    fn from(value: SetTypeSnapshot) -> Self {
        Self {
            name: value.name.into(),
            can_open_by_hand: value.can_open_by_hand,
            can_open_by_wind_charge: value.can_open_by_wind_charge,
            arrows: value.arrows,
            mobs_only: value.mobs_only,
            door_open: Arc::from(value.door_open),
            door_close: Arc::from(value.door_close),
            trapdoor_open: Arc::from(value.trapdoor_open),
            trapdoor_close: Arc::from(value.trapdoor_close),
            plate_on: Arc::from(value.plate_on),
            plate_off: Arc::from(value.plate_off),
            button_on: Arc::from(value.button_on),
            button_off: Arc::from(value.button_off),
        }
    }
}
impl From<&Sounds> for SoundsSnapshot {
    fn from(value: &Sounds) -> Self {
        Self {
            volume: value.volume,
            pitch: value.pitch,
            place: value.place.to_string(),
            break_: value.break_.to_string(),
            fall: value.fall.to_string(),
        }
    }
}
impl From<SoundsSnapshot> for Sounds {
    fn from(value: SoundsSnapshot) -> Self {
        Self {
            volume: value.volume,
            pitch: value.pitch,
            place: Arc::from(value.place),
            break_: Arc::from(value.break_),
            fall: Arc::from(value.fall),
        }
    }
}

impl Snapshot {
    fn capture(behaviours: &Behaviours) -> Self {
        let tags = behaviours
            .all_tags
            .iter()
            .map(|(key, value)| (key.clone(), value.0.clone()))
            .collect();
        let facts = behaviours
            .facts
            .iter()
            .map(|fact| Fact {
                flags: fact.flags,
                sturdy: fact.sturdy,
                weak: fact.weak,
                strong: fact.strong,
                fluid_kind: fact.fluid.kind() as u8,
                fluid_amount: fact.fluid.amount(),
                fluid_falling: fact.fluid.falling(),
                emission: fact.emission,
                dampening: fact.dampening,
                hardness: fact.hardness,
                push: push_index(fact.push),
                instrument: fact.instrument,
                shape: fact.shape,
            })
            .collect();
        let shapes = behaviours
            .shapes
            .iter()
            .map(|shape| {
                let sizes = shape.sizes();
                let mut occupied = Vec::with_capacity(sizes.iter().product());
                for x in 0..sizes[0] {
                    for y in 0..sizes[1] {
                        for z in 0..sizes[2] {
                            occupied.push(shape.is_full(x as i32, y as i32, z as i32));
                        }
                    }
                }
                let (min, max) = shape.discrete_bounds();
                Shape {
                    coords: std::array::from_fn(|axis| {
                        shape
                            .coordinates(
                                [
                                    horizon_core::geometry::Axis::X,
                                    horizon_core::geometry::Axis::Y,
                                    horizon_core::geometry::Axis::Z,
                                ][axis],
                            )
                            .to_vec()
                    }),
                    occupied,
                    cube: shape.is_cube(),
                    coordinate_cubes: shape.cube_coordinate_axes(),
                    min,
                    max,
                    singleton: if shape.same_identity(&VoxelShape::block()) {
                        1
                    } else if shape.same_identity(&VoxelShape::empty()) {
                        2
                    } else {
                        0
                    },
                }
            })
            .collect();
        let shape_info = behaviours
            .shape_info
            .iter()
            .map(|info| [info.full_faces, info.nonempty_faces, info.wall_cover])
            .collect();
        let f = behaviours.fluid_tags;
        let fluid_tags = [
            f.water,
            f.lava,
            f.supports_frogspawn,
            f.supports_lily_pad,
            f.supports_sugar_cane_adjacently,
            f.bubble_column_can_occupy,
        ];
        let blocks = behaviours
            .blocks
            .iter()
            .map(|block| Block {
                chain: block.chain.iter().map(|class| *class as u16).collect(),
                interfaces: block.interfaces,
                set_type: block
                    .set_type
                    .as_ref()
                    .map(|value| SetTypeSnapshot::from(value.as_ref())),
                gate_open: block.gate_open.as_ref().map(|v| v.to_string()),
                gate_close: block.gate_close.as_ref().map(|v| v.to_string()),
                open_sound: block.open_sound.as_ref().map(|v| v.to_string()),
                close_sound: block.close_sound.as_ref().map(|v| v.to_string()),
                ticks_to_stay_pressed: block.ticks_to_stay_pressed,
                max_weight: block.max_weight,
                link: block.link.map(|v| v.0),
                link2: block.link2.map(|v| v.0),
                support_tag: block.support_tag.as_ref().map(|v| v.0.clone()),
                support_tag2: block.support_tag2.as_ref().map(|v| v.0.clone()),
                feature: block.feature.as_ref().map(|v| v.to_string()),
                grow_probability: block.grow_probability,
                growth_direction: direction_index(block.growth_direction),
                schedule_fluid_ticks: block.schedule_fluid_ticks,
                weather: block.weather,
                ignite_odds: block.ignite_odds,
                burn_odds: block.burn_odds,
                sounds: SoundsSnapshot::from(block.sounds.as_ref()),
                item: block.item.to_string(),
                dynamic_shape: block.dynamic_shape,
            })
            .collect();
        let items = behaviours
            .items
            .iter()
            .map(|(name, item)| {
                (
                    name.clone(),
                    Item {
                        block: item.block.0,
                        wall_block: item.wall_block.map(|v| v.0),
                        attachment: item.attachment.map(direction_index),
                        kind: kind_index(item.kind),
                        game_master: item.game_master,
                    },
                )
            })
            .collect();
        Self {
            tags,
            facts,
            shapes,
            shape_info,
            fluid_tags,
            blocks,
            items,
        }
    }

    fn restore(self, registry: &BlockRegistry) -> io::Result<Behaviours> {
        let mut table = StateTable::new(registry)?;
        let props = Props::resolve(&mut table);
        let named = Named::resolve(&table)?;
        if self.facts.len() != table.state_count()
            || self.blocks.len() != table.block_count()
            || self.shape_info.len() != self.shapes.len()
        {
            return Err(invalid("cached behaviour table size mismatch"));
        }
        let all_tags: HashMap<String, Arc<BlockSet>> = self
            .tags
            .into_iter()
            .map(|(key, value)| (key, Arc::new(BlockSet(value))))
            .collect();
        let tags = Tags::resolve(&all_tags);
        let shapes = self
            .shapes
            .into_iter()
            .map(|wire| {
                let shape = match wire.singleton {
                    1 => VoxelShape::block(),
                    2 => VoxelShape::empty(),
                    0 => {
                        if wire.cube {
                            let sizes = std::array::from_fn(|axis| {
                                wire.coords[axis].len().saturating_sub(1)
                            });
                            VoxelShape::from_cube(sizes, &wire.occupied)
                                .map_err(|e| invalid(e.to_string()))?
                        } else {
                            VoxelShape::from_mixed_grid(
                                wire.coords,
                                &wire.occupied,
                                wire.coordinate_cubes,
                            )
                            .map_err(|e| invalid(e.to_string()))?
                        }
                    }
                    _ => return Err(invalid("invalid cached shape identity")),
                };
                shape
                    .with_discrete_bounds(wire.min, wire.max)
                    .map_err(|e| invalid(e.to_string()))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let shape_info = self
            .shape_info
            .into_iter()
            .map(|row| ShapeInfo {
                full_faces: row[0],
                nonempty_faces: row[1],
                wall_cover: row[2],
            })
            .collect();
        let facts = self
            .facts
            .into_iter()
            .map(|wire| {
                if wire.shape < -1 || wire.shape >= shapes.len() as i32 {
                    return Err(invalid("cached shape index out of range"));
                }
                let fluid_kind = FluidKind::ALL
                    .get(wire.fluid_kind as usize)
                    .copied()
                    .ok_or_else(|| invalid("cached fluid kind out of range"))?;
                Ok(StateFacts {
                    flags: wire.flags,
                    sturdy: wire.sturdy,
                    weak: wire.weak,
                    strong: wire.strong,
                    fluid: FluidState::new(fluid_kind, wire.fluid_amount, wire.fluid_falling),
                    emission: wire.emission,
                    dampening: wire.dampening,
                    hardness: wire.hardness,
                    push: push(wire.push)?,
                    instrument: wire.instrument,
                    shape: wire.shape,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let f = self.fluid_tags;
        let fluid_tags = FluidTags {
            water: f[0],
            lava: f[1],
            supports_frogspawn: f[2],
            supports_lily_pad: f[3],
            supports_sugar_cane_adjacently: f[4],
            bubble_column_can_occupy: f[5],
        };
        let blocks = self
            .blocks
            .into_iter()
            .map(|wire| {
                let chain = wire
                    .chain
                    .into_iter()
                    .map(class)
                    .collect::<io::Result<Vec<_>>>()?;
                let mut classes = ClassSet::default();
                for class in &chain {
                    classes.insert(*class);
                }
                let class = chain.first().copied().unwrap_or(Class::Block);
                for link in [wire.link, wire.link2].into_iter().flatten() {
                    if link as usize >= table.block_count() {
                        return Err(invalid("cached block link out of range"));
                    }
                }
                Ok(BlockData {
                    class,
                    chain,
                    classes,
                    interfaces: wire.interfaces,
                    set_type: wire.set_type.map(|v| Arc::new(SetType::from(v))),
                    gate_open: wire.gate_open.map(Arc::from),
                    gate_close: wire.gate_close.map(Arc::from),
                    open_sound: wire.open_sound.map(Arc::from),
                    close_sound: wire.close_sound.map(Arc::from),
                    ticks_to_stay_pressed: wire.ticks_to_stay_pressed,
                    max_weight: wire.max_weight,
                    link: wire.link.map(BlockId),
                    link2: wire.link2.map(BlockId),
                    support_tag: wire.support_tag.map(|v| Arc::new(BlockSet(v))),
                    support_tag2: wire.support_tag2.map(|v| Arc::new(BlockSet(v))),
                    feature: wire.feature.map(Arc::from),
                    grow_probability: wire.grow_probability,
                    growth_direction: direction(wire.growth_direction)?,
                    schedule_fluid_ticks: wire.schedule_fluid_ticks,
                    weather: wire.weather,
                    ignite_odds: wire.ignite_odds,
                    burn_odds: wire.burn_odds,
                    sounds: Arc::new(Sounds::from(wire.sounds)),
                    item: Arc::from(wire.item),
                    dynamic_shape: wire.dynamic_shape,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let items = self
            .items
            .into_iter()
            .map(|(name, wire)| {
                if wire.block as usize >= table.block_count()
                    || wire
                        .wall_block
                        .is_some_and(|id| id as usize >= table.block_count())
                {
                    return Err(invalid("cached item block out of range"));
                }
                Ok((
                    name,
                    ItemBinding {
                        block: BlockId(wire.block),
                        wall_block: wire.wall_block.map(BlockId),
                        attachment: wire.attachment.map(direction).transpose()?,
                        kind: kind(wire.kind)?,
                        game_master: wire.game_master,
                    },
                ))
            })
            .collect::<io::Result<HashMap<_, _>>>()?;
        let liquid_blocks = FluidBlockBindings::new(registry)?;
        let mut dynamic = HashMap::new();
        for (id, fact) in facts.iter().enumerate() {
            if fact.shape < 0 {
                let state = BlockStateId(id as u32);
                if let Some(support) = crate::dynamic_support::DynamicSupport::from_state(
                    registry.state(state).unwrap(),
                )? {
                    dynamic.insert(state, support);
                }
            }
        }
        let potted_by_content = super::potted_by_content(&blocks, &table);
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
            potted_by_content,
            liquid_blocks,
            dynamic,
            occlusion_cache: Default::default(),
            growth: Default::default(),
        })
    }
}

pub(super) fn load(registry: &BlockRegistry, root: &Path) -> Option<Behaviours> {
    load_from(DIR.get()?, registry, root)
}

fn load_from(dir: &Path, registry: &BlockRegistry, root: &Path) -> Option<Behaviours> {
    let bytes = fs::read(dir.join(FILE_NAME)).ok()?;
    let fingerprint = fingerprint(registry, root).ok()?;
    // Validate bounds before decoding untrusted cache contents.
    if bytes.len() < 8 + 4 + 32 + 32 || bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    if &bytes[..8] != MAGIC
        || u32::from_le_bytes(bytes[8..12].try_into().ok()?) != VERSION
        || bytes[12..44] != fingerprint
    {
        return None;
    }
    let body = &bytes[76..];
    if Sha256::digest(body).as_slice() != &bytes[44..76] {
        return None;
    }
    let expanded_len = u32::from_le_bytes(body.get(..4)?.try_into().ok()?) as usize;
    if expanded_len > MAX_DECODED_BYTES {
        return None;
    }
    let expanded = lz4_flex::decompress_size_prepended(body).ok()?;
    let snapshot: Snapshot = postcard::from_bytes(&expanded).ok()?;
    snapshot.restore(registry).ok()
}

pub(super) fn store(registry: &BlockRegistry, root: &Path, behaviours: &Behaviours) {
    if let Some(dir) = DIR.get() {
        let _ = write(dir, registry, root, behaviours);
    }
}

fn write(
    dir: &Path,
    registry: &BlockRegistry,
    root: &Path,
    behaviours: &Behaviours,
) -> io::Result<()> {
    let fingerprint = fingerprint(registry, root)?;
    let snapshot = Snapshot::capture(behaviours);
    let serialized = postcard::to_allocvec(&snapshot).map_err(|e| invalid(e.to_string()))?;
    let body = lz4_flex::compress_prepend_size(&serialized);
    if body.len() + 76 > MAX_FILE_BYTES {
        return Err(invalid("behaviour cache too large"));
    }
    let mut bytes = Vec::with_capacity(76 + body.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&fingerprint);
    bytes.extend_from_slice(&Sha256::digest(&body));
    bytes.extend_from_slice(&body);
    fs::create_dir_all(dir)?;
    let path = dir.join(FILE_NAME);
    let tmp = dir.join(format!(
        "{FILE_NAME}.tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct FixtureDir(PathBuf);
    impl Drop for FixtureDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> Option<FixtureDir> {
        let source = std::env::var_os("HORIZON_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
            });
        if !source.join("block-behaviours.json").exists() {
            eprintln!("skipping behaviour cache tests: no client resources");
            return None;
        }
        let dir = std::env::temp_dir().join(format!(
            "horizon-behaviour-cache-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(dir.join("data/minecraft/tags/fluid")).unwrap();
        for file in [
            "facts.json",
            "block-behaviours.json",
            "gameplay-facts.json",
            "block-tags.json",
            "cached-collision.nbt",
        ] {
            fs::copy(source.join(file), dir.join(file)).unwrap();
        }
        for entry in fs::read_dir(source.join("data/minecraft/tags/fluid")).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                fs::copy(
                    entry.path(),
                    dir.join("data/minecraft/tags/fluid")
                        .join(entry.file_name()),
                )
                .unwrap();
            }
        }
        Some(FixtureDir(dir))
    }

    #[test]
    fn resolved_snapshot_roundtrips_all_fields() {
        let Some(root) = fixture() else {
            return;
        };
        let registry = super::super::load_registry(&root.0).unwrap();
        let original = super::super::load_uncached(&registry, &root.0).unwrap();
        let captured = Snapshot::capture(&original);
        let restored = captured.restore(&registry).unwrap();
        assert_eq!(
            serde_json::to_value(Snapshot::capture(&original)).unwrap(),
            serde_json::to_value(Snapshot::capture(&restored)).unwrap(),
            "all resolved fields, shape grids and identities should roundtrip",
        );
    }

    /// A folder's sources are hashed once while they stay as they are; a
    /// file rewritten at the same length is hashed again.
    #[test]
    fn sources_are_hashed_once_until_a_file_changes() {
        let Some(root) = fixture() else {
            return;
        };
        let entries = || {
            SOURCES
                .lock()
                .unwrap()
                .iter()
                .filter(|(stamps, _)| stamps[0].0.starts_with(&root.0))
                .count()
        };
        let first = sources_digest(&root.0).unwrap();
        assert_eq!(sources_digest(&root.0).unwrap(), first);
        assert_eq!(entries(), 1);
        let tags = root.0.join("block-tags.json");
        let mut bytes = fs::read(&tags).unwrap();
        bytes[0] ^= 1;
        fs::write(&tags, bytes).unwrap();
        assert_ne!(sources_digest(&root.0).unwrap(), first);
        assert_eq!(entries(), 2);
    }

    #[test]
    fn cache_invalidates_sources_and_recovers_from_corruption() {
        let Some(root) = fixture() else {
            return;
        };
        // The game's cache folder, apart from the resources.
        let dir = root.0.join("game/.horizon-cache");
        let registry = super::super::load_registry(&root.0).unwrap();
        // What `data::load` does once `set_dir` gave it a folder.
        let load_or_build = || {
            load_from(&dir, &registry, &root.0).unwrap_or_else(|| {
                let built = super::super::load_uncached(&registry, &root.0).unwrap();
                write(&dir, &registry, &root.0, &built).unwrap();
                built
            })
        };
        assert!(load_from(&dir, &registry, &root.0).is_none());
        let initial = load_or_build();
        assert!(dir.join(FILE_NAME).is_file());
        assert!(
            !root.0.join(FILE_NAME).exists(),
            "the resource folder stays untouched"
        );
        let hit = load_from(&dir, &registry, &root.0).expect("cache hit");
        assert_eq!(initial.table.state_count(), hit.table.state_count());
        let tags = root.0.join("block-tags.json");
        fs::OpenOptions::new()
            .append(true)
            .open(&tags)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        assert!(
            load_from(&dir, &registry, &root.0).is_none(),
            "changed resource must invalidate"
        );
        load_or_build();
        assert!(
            load_from(&dir, &registry, &root.0).is_some(),
            "fallback should replace cache"
        );
        let cache_file = dir.join(FILE_NAME);
        let mut bytes = fs::read(&cache_file).unwrap();
        bytes[80] ^= 0x80;
        fs::write(&cache_file, bytes).unwrap();
        assert!(
            load_from(&dir, &registry, &root.0).is_none(),
            "checksum failure must invalidate"
        );
        load_or_build();
        assert!(load_from(&dir, &registry, &root.0).is_some());
        // A snapshot written under another key (another build of the game,
        // other resources) is never trusted.
        let mut other = fs::read(&cache_file).unwrap();
        other[12] ^= 1;
        fs::write(&cache_file, other).unwrap();
        assert!(load_from(&dir, &registry, &root.0).is_none());
    }
}
