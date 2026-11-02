//! World persistence in the vanilla 26.3 layout, readable by both Horizon and
//! the vanilla game:
//! - `level.dat` (`Data`: name, game type, time, spawn, difficulty, version)
//!   plus the saved-data files `data/minecraft/{weather,world_clocks,
//!   world_gen_settings,game_rules}.dat`;
//! - the player in `players/data/<uuid>.dat` (`singleplayer_uuid`);
//! - per dimension `dimensions/minecraft/<name>/region/r.X.Z.mca` (chunks:
//!   paletted sections with light, heightmaps, block entities, scheduled
//!   block/fluid ticks) and `entities/r.X.Z.mca` (entities, `entity_nbt`).
//!
//! Data Horizon does not simulate (structure references, unknown block
//! entities and entities, player fields such as the ender chest or effects,
//! game rules) is kept from the loaded files and written back unchanged.
//! Worlds saved by older Horizon builds (`region/`, `DIM-1/`, `Player` in
//! `level.dat`) are migrated on open.
use crate::{
    Result,
    beehive::BeehiveExt,
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, ColumnLayout, Light, Section, block_strategy},
    mobs::{spawner::SpawnerExt, trial_spawner::TrialSpawnerExt},
    streaming::ChunkGenerator,
};
use horizon_core::{
    paletted::PalettedContainer,
    position::{BlockPos, ChunkPos},
    ticks::{SavedTick, TickPriority, TickTypeId},
};
use horizon_world::attachments::{ATTACHMENTS_KEY, attachment_compound};
use horizon_world::{
    DATA_VERSION,
    chunk::BlockState,
    chunk_status::ChunkStatus,
    compression::Compression,
    heightmap::{Heightmap, HeightmapChunk, HeightmapState, HeightmapType},
    nbt::{self, Compound, JavaString, Limits, NamedTag, Tag},
    region::{ChunkRecord, RegionFile},
    registry::{BlockRegistry, BlockStateId},
    worldgen::terrain::TerrainBlockStates,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

pub const VERSION_NAME: &str = horizon_core::VANILLA_VERSION;

mod gateway;

/// Snapshot live block entities and custom names for a save batch without
/// mutating the world. Each map is visited once, retaining its iteration order
/// within each chunk (including repeated chunk coordinates in the input).
/// An empty batch skips both maps; one chunk uses the direct filter path.
pub fn chunks_with_block_entities<'a, N: std::hash::BuildHasher>(
    chunks: impl IntoIterator<Item = &'a Arc<Chunk>>,
    entities: &HashMap<(i32, i32, i32), crate::containers::BlockEntity>,
    names: &HashMap<(i32, i32, i32), String, N>,
) -> Vec<Chunk> {
    let chunks: Vec<_> = chunks.into_iter().collect();
    if chunks.is_empty() {
        return Vec::new();
    }
    if chunks.len() == 1 {
        let mut c = (**chunks[0]).clone();
        c.block_entities = entities
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, e)| (*p, e.clone()))
            .collect();
        c.block_names = names
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, n)| (*p, n.clone()))
            .collect();
        return vec![c];
    }
    // Index only requested chunks, borrowing their entries so unsaved entities
    // are never cloned. Clone one chunk at a time for inventory cache locality.
    let mut grouped = crate::fast_hash::FxHashMap::<_, (Vec<_>, Vec<_>)>::default();
    grouped.reserve(chunks.len());
    for c in &chunks {
        grouped.entry((c.x, c.z)).or_default();
    }
    for (p, e) in entities {
        if let Some((entries, _)) = grouped.get_mut(&(p.0 >> 4, p.2 >> 4)) {
            entries.push((p, e));
        }
    }
    for (p, n) in names {
        if let Some((_, entries)) = grouped.get_mut(&(p.0 >> 4, p.2 >> 4)) {
            entries.push((p, n));
        }
    }
    chunks
        .into_iter()
        .map(|chunk| {
            let mut c = (**chunk).clone();
            let (entities, names) = &grouped[&(c.x, c.z)];
            c.block_entities = entities.iter().map(|(p, e)| (**p, (*e).clone())).collect();
            c.block_names = names.iter().map(|(p, n)| (**p, (*n).clone())).collect();
            c
        })
        .collect()
}

use horizon_client_core::nbt_value::string;
pub use horizon_client_core::nbt_value::{float, number, text};
/// Insert or replace a key of a compound tag.
pub(crate) fn put(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(map) = tag {
        map.insert(key.into(), value);
    }
}
fn compound_or_empty(tag: Option<Tag>) -> Tag {
    match tag {
        Some(t @ Tag::Compound(_)) => t,
        _ => Tag::Compound(Compound::new()),
    }
}

/// `dimensions/<namespace>/<path>` inside a world folder.
pub fn dimension_dir(world: &Path, dimension: &str) -> PathBuf {
    let (namespace, path) = dimension
        .split_once(':')
        .unwrap_or(("minecraft", dimension));
    world.join("dimensions").join(namespace).join(path)
}
/// `DirectoryLock`: `session.lock` holds "\u{2603}" (UTF-16BE) under an exclusive
/// file lock for as long as the world is open, so two instances cannot share it.
pub struct SessionLock(fs::File);
impl Drop for SessionLock {
    /// `DirectoryLock.close`: release the lock, then close the file. Closing
    /// alone keeps the lock while any copy of the descriptor lives on, such as
    /// the one a process spawned on another thread holds until its `exec`.
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
impl SessionLock {
    pub fn acquire(world: &Path) -> Result<Self> {
        use std::io::Write;
        fs::create_dir_all(world)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(world.join("session.lock"))?;
        file.try_lock()
            .map_err(|_| format!("{} is already open in another instance", world.display()))?;
        file.set_len(0)?;
        file.write_all(&[0x26, 0x03])?;
        file.flush()?;
        Ok(Self(file))
    }
    /// `DirectoryLock.isLocked`: another copy holds the world's `session.lock`.
    /// A world without the file is not locked; one whose file may not be opened
    /// is.
    pub fn is_locked(world: &Path) -> bool {
        let file = match fs::OpenOptions::new()
            .write(true)
            .open(world.join("session.lock"))
        {
            Ok(file) => file,
            Err(e) => return e.kind() == std::io::ErrorKind::PermissionDenied,
        };
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) => true,
            Err(_) => false,
            // Take the probe's own lock back at once, as `SessionLock` does on drop.
            Ok(()) => {
                let _ = file.unlock();
                false
            }
        }
    }
}
/// Bring a world last saved by an older game version to the current format, as
/// 26.3 does when it opens one: the file fixes and level.dat first
/// (`FileFixerUpper`), then the DataFixers upgrades of every chunk, entity chunk,
/// POI chunk, player, saved data, advancements and statistics file. Vanilla runs
/// the latter as it reads each file; Horizon runs them all on opening, which reads
/// the same data, and region records still older in a world whose level.dat is
/// current are upgraded as they are read (`upgrade_record`).
pub fn upgrade_older_world(world: &Path) -> Result<()> {
    upgrade_older_world_with(
        world,
        &horizon_world::datafix::progress::UpgradeProgress::new(),
    )
}
/// [`upgrade_older_world`] reporting to `progress`, which `FileFixerProgressScreen` reads.
pub fn upgrade_older_world_with(
    world: &Path,
    progress: &horizon_world::datafix::progress::UpgradeProgress,
) -> Result<()> {
    let Some(root) = read_dat(&world.join("level.dat")) else {
        return Ok(());
    };
    let version = root.get("Data").map_or(i64::from(DATA_VERSION), |d| {
        number(d.get("DataVersion")).unwrap_or(-1) // saves of 1.8 and before have none
    });
    if version >= i64::from(DATA_VERSION) {
        return Ok(());
    }
    if version < i64::from(horizon_world::datafix::OLDEST_SUPPORTED) {
        return Err(format!(
            "{} was saved with DataVersion {version}; worlds older than DataVersion {} \
             cannot be upgraded yet (tools/upgrade_world.py runs vanilla's own upgrader)",
            world.display(),
            horizon_world::datafix::OLDEST_SUPPORTED
        )
        .into());
    }
    horizon_world::datafix::files::upgrade_world_with(world, progress)
        .map_err(|e| format!("upgrading {}: {e}", world.display()))?;
    Ok(())
}
/// Whether opening the world runs file fixes, which vanilla asks for a backup
/// first (`BackupStatus.FILE_FIXING_REQUIRED`). Saves of 1.8 and before have no
/// DataVersion, read as -1.
pub fn needs_file_fixes(world: &Path) -> bool {
    let Some(root) = read_dat(&world.join("level.dat")) else {
        return false;
    };
    let Some(data) = root.get("Data") else {
        return false;
    };
    let version = number(data.get("DataVersion")).unwrap_or(-1);
    i32::try_from(version).is_ok_and(horizon_world::datafix::files::requires_file_fixing)
}
/// `LevelVersion.parse`: the saved game version's name, this build's when the `Version`
/// compound has no `Name`, and empty without a `Version` (`getWorldVersionName` then
/// shows `selectWorld.versionUnknown`).
fn saved_version_name(version: Option<&Tag>) -> String {
    match version {
        Some(v) => text(v.get("Name")).unwrap_or_else(|| VERSION_NAME.to_string()),
        None => String::new(),
    }
}
/// `LevelSummary.isCompatible` (`DataVersion.isCompatible`: the saved series must be
/// the current `main` one). None when the world opens; otherwise the saved game
/// version's name, empty when it has none (`getWorldVersionName`).
pub fn incompatible_version(world: &Path) -> Option<String> {
    let root = read_dat(&world.join("level.dat"))?;
    let version = root.get("Data")?.get("Version");
    let series = text(version.and_then(|v| v.get("Series"))).unwrap_or_else(|| "main".into());
    if series == "main" {
        return None;
    }
    Some(saved_version_name(version))
}
/// `LevelSummary.backupStatus` DOWNGRADE: the saved game version's name (empty when it
/// has none) when the world's `Version.Id` is newer than this build's `DataVersion`
/// (`LevelVersion.parse`: no `Version` reads as 0, no `Id` as the current one).
pub fn downgraded_from(world: &Path) -> Option<String> {
    let root = read_dat(&world.join("level.dat"))?;
    let version = root.get("Data")?.get("Version");
    let id = version.map_or(0, |v| {
        number(v.get("Id")).unwrap_or_else(|| i64::from(DATA_VERSION))
    });
    if id <= i64::from(DATA_VERSION) {
        return None;
    }
    Some(saved_version_name(version))
}
/// `LevelStorageAccess.checkForLowDiskSpace`: under 64 MiB usable.
pub const LOW_DISK_SPACE: u64 = 67_108_864;
/// `LevelStorageAccess.estimateDiskSpace`: the usable bytes of the disk holding `dir`
/// (`FileStore.getUsableSpace`), `u64::MAX` when it cannot be asked.
pub fn estimate_disk_space(dir: &Path) -> u64 {
    fs4::available_space(dir).unwrap_or(u64::MAX)
}
/// `LevelStorageAccess.checkForLowDiskSpace`.
pub fn low_disk_space(dir: &Path) -> bool {
    estimate_disk_space(dir) < LOW_DISK_SPACE
}
/// `WorldOptions.isOldCustomizedWorld`: the generator settings keep `legacy_custom_options`.
pub fn is_old_customized_world(world: &Path) -> bool {
    world_gen_settings(world).is_some_and(|s| text(s.get("legacy_custom_options")).is_some())
}
/// A dimension's `LevelStem` field read as a resource location (an inline definition is not one).
fn stem_id(tag: Option<&Tag>) -> Option<String> {
    let id = text(tag)?;
    Some(if id.contains(':') {
        id
    } else {
        format!("minecraft:{id}")
    })
}
/// `WorldDimensions.isVanillaLike`: whether a saved dimension is the stable Overworld, Nether or
/// End (`isStableOverworld`, `isStableNether`, `isStableEnd`); any other key never is.
fn dimension_is_vanilla_like(key: &str, dimension: &Tag) -> bool {
    let kind = stem_id(dimension.get("type"));
    let generator = dimension.get("generator");
    let source = generator.and_then(|g| g.get("biome_source"));
    let source_kind = source.and_then(|s| stem_id(s.get("type")));
    // MultiNoiseBiomeSource.stable(preset): a reference to the preset, not its parameters.
    let multi_noise_preset = |preset: &str| {
        source_kind.as_deref() == Some("minecraft:multi_noise")
            && stem_id(source.and_then(|s| s.get("preset"))).as_deref() == Some(preset)
    };
    // NoiseBasedChunkGenerator.stable(settings): the generator uses the settings by key.
    let noise_settings = |settings: &str| {
        generator.is_some_and(|g| stem_id(g.get("type")).as_deref() == Some("minecraft:noise"))
            && stem_id(generator.and_then(|g| g.get("settings"))).as_deref() == Some(settings)
    };
    match key {
        "minecraft:overworld" => {
            matches!(
                kind.as_deref(),
                Some("minecraft:overworld" | "minecraft:overworld_caves")
            ) && (source_kind.as_deref() != Some("minecraft:multi_noise")
                || multi_noise_preset("minecraft:overworld"))
        }
        "minecraft:the_nether" => {
            kind.as_deref() == Some("minecraft:the_nether")
                && noise_settings("minecraft:nether")
                && multi_noise_preset("minecraft:nether")
        }
        "minecraft:the_end" => {
            kind.as_deref() == Some("minecraft:the_end")
                && noise_settings("minecraft:end")
                && source_kind.as_deref() == Some("minecraft:the_end")
        }
        _ => false,
    }
}
/// A data pack's JSON as the NBT `dimension_is_vanilla_like` reads (what a codec gets from
/// either one is the same data).
fn json_tag(value: &serde_json::Value) -> Tag {
    use serde_json::Value;
    match value {
        Value::Null => Tag::compound(std::iter::empty::<(&str, Tag)>()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => n.as_i64().map_or_else(
            || Tag::Double(n.as_f64().unwrap_or_default().to_bits()),
            Tag::Long,
        ),
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(items) => Tag::List(items.iter().map(json_tag).collect()),
        Value::Object(map) => Tag::compound(map.iter().map(|(k, v)| (k.as_str(), json_tag(v)))),
    }
}
/// The level stems the world's enabled data packs define, as `WorldOpenFlows.loadWorldStem`'s
/// `datapackDimensions`.
fn datapack_dimensions(world: &Path) -> Vec<(String, Tag)> {
    let packs = crate::datapack::discover(world);
    let enabled_features = Level::read(world)
        .map(|l| l.enabled_features)
        .unwrap_or_default();
    let enabled = crate::datapack::enabled_ids(world, &packs, &enabled_features);
    let enabled: Vec<&crate::datapack::DataPack> = enabled
        .iter()
        .filter_map(|id| packs.iter().find(|p| &p.id == id))
        .collect();
    crate::datapack::dimension_stems(&enabled)
        .into_iter()
        .map(|(id, json)| (id, json_tag(&json)))
        .collect()
}
/// `data.worldGenSettingsLifecycle() != Lifecycle.stable()` (`WorldDimensions.bake`): the known
/// dimensions (the saved ones and the enabled data packs', a pack's replacing the saved one of
/// its id) lack one of the three builtin ones, or have one that is not the vanilla one.
pub fn world_gen_unstable(world: &Path) -> bool {
    let saved = world_gen_settings(world)
        .as_ref()
        .and_then(|s| s.get("dimensions"))
        .cloned();
    let stems = datapack_dimensions(world);
    if saved.is_none() && stems.is_empty() {
        return false;
    }
    let mut dimensions: std::collections::BTreeMap<String, Tag> = match saved {
        Some(Tag::Compound(saved)) => saved
            .into_iter()
            .map(|(key, dimension)| (key.to_string_lossy(), dimension))
            .collect(),
        _ => std::collections::BTreeMap::new(),
    };
    // `baseDimensions.getOptional(key).or(saved)`.
    dimensions.extend(stems);
    let builtin = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ];
    builtin.iter().any(|key| !dimensions.contains_key(*key))
        || dimensions
            .iter()
            .any(|(key, dimension)| !dimension_is_vanilla_like(key, dimension))
}
/// Move a world saved by an older Horizon build (pre-26.3 layout with a
/// DataVersion that keeps vanilla from upgrading it) into the 26.3 layout.
pub fn migrate_legacy(world: &Path) -> Result<()> {
    for (old, dimension) in [
        ("", "minecraft:overworld"),
        ("DIM-1", "minecraft:the_nether"),
        ("DIM1", "minecraft:the_end"),
    ] {
        let from = if old.is_empty() {
            world.to_owned()
        } else {
            world.join(old)
        };
        let to = dimension_dir(world, dimension);
        for folder in ["region", "entities", "poi"] {
            let (src, dst) = (from.join(folder), to.join(folder));
            if src.is_dir() && !dst.exists() {
                fs::create_dir_all(&to)?;
                fs::rename(&src, &dst)?;
            }
        }
        if !old.is_empty() && from.is_dir() && fs::read_dir(&from)?.next().is_none() {
            let _ = fs::remove_dir(&from);
        }
    }
    Ok(())
}

/// Registry lookups shared by chunk encoding and decoding.
pub struct SaveTables {
    /// Block registry IDs (`BlockId`, scheduled tick types) in registry order.
    block_names: Vec<String>,
    block_ids: HashMap<String, u16>,
    /// Default state of each state's block, to write vanilla's short palette form.
    default_state: Vec<bool>,
    block_entity: Vec<bool>,
    heightmaps: crate::assets::Properties,
    air: u32,
    /// The POI type and its max tickets of each state (`PoiTypes`).
    poi: Vec<Option<(&'static str, i32)>>,
}
/// `PoiTypes.register`: the POI type of a block state and its max tickets.
pub fn poi_type(state: &horizon_world::chunk::BlockState) -> Option<(&'static str, i32)> {
    let name = state.name.strip_prefix("minecraft:")?;
    Some(match name {
        "blast_furnace" => ("minecraft:armorer", 1),
        "smoker" => ("minecraft:butcher", 1),
        "cartography_table" => ("minecraft:cartographer", 1),
        "brewing_stand" => ("minecraft:cleric", 1),
        "composter" => ("minecraft:farmer", 1),
        "barrel" => ("minecraft:fisherman", 1),
        "fletching_table" => ("minecraft:fletcher", 1),
        "cauldron" | "lava_cauldron" | "water_cauldron" | "powder_snow_cauldron" => {
            ("minecraft:leatherworker", 1)
        }
        "lectern" => ("minecraft:librarian", 1),
        "stonecutter" => ("minecraft:mason", 1),
        "loom" => ("minecraft:shepherd", 1),
        "smithing_table" => ("minecraft:toolsmith", 1),
        "grindstone" => ("minecraft:weaponsmith", 1),
        "bell" => ("minecraft:meeting", 32),
        "beehive" => ("minecraft:beehive", 0),
        "bee_nest" => ("minecraft:bee_nest", 0),
        "nether_portal" => ("minecraft:nether_portal", 0),
        "lodestone" => ("minecraft:lodestone", 0),
        "test_instance_block" => ("minecraft:test_instance", 0),
        // PoiTypes.BEDS is the sixteen colours: no straw bed.
        bed if bed.ends_with("_bed") && bed != "straw_bed" => {
            if state.properties.get("part").map(String::as_str) != Some("head") {
                return None;
            }
            ("minecraft:home", 1)
        }
        rod if rod.ends_with("lightning_rod") => ("minecraft:lightning_rod", 0),
        _ => return None,
    })
}
impl SaveTables {
    pub fn new(assets: &crate::assets::Assets) -> Self {
        let registry = assets.registry;
        let mut block_names: Vec<String> = Vec::new();
        let mut default_state = Vec::with_capacity(registry.len());
        for id in 0..registry.len() {
            let state = registry
                .state(BlockStateId(id as u32))
                .expect("registry state");
            if block_names.last() != Some(&state.name) {
                block_names.push(state.name.clone());
            }
            default_state
                .push(registry.default_state(&state.name) == Some(BlockStateId(id as u32)));
        }
        let block_ids = block_names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i as u16))
            .collect();
        Self {
            block_names,
            block_ids,
            default_state,
            block_entity: Vec::new(),
            heightmaps: assets.properties.clone(),
            air: registry.default_state("minecraft:air").map_or(0, |s| s.0),
            poi: (0..registry.len())
                .map(|id| registry.state(BlockStateId(id as u32)).and_then(poi_type))
                .collect(),
        }
    }
    /// Tables with only the heightmaps' state facts, for tests.
    #[cfg(test)]
    pub(crate) fn for_heightmaps(heightmaps: crate::assets::Properties) -> Self {
        Self {
            block_names: Vec::new(),
            block_ids: HashMap::new(),
            default_state: Vec::new(),
            block_entity: Vec::new(),
            heightmaps,
            air: 0,
            poi: Vec::new(),
        }
    }
    /// Which states carry a block entity (`BlockFacts`), to drop entries for
    /// unknown block entities whose block is gone.
    pub fn with_block_entities(mut self, facts: &crate::blocks::BlockFacts) -> Self {
        self.block_entity = facts.states.iter().map(|s| s.block_entity).collect();
        self
    }
}

pub use crate::chunk::{ChunkExtras, RolledLoot, SavedProto};

fn tick_list(ticks: &[SavedTick], name: impl Fn(u32) -> Option<String>) -> Tag {
    Tag::List(
        ticks
            .iter()
            .filter_map(|t| {
                Some(Tag::compound([
                    ("i", string(&name(t.kind.0)?)),
                    ("x", Tag::Int(t.pos.x)),
                    ("y", Tag::Int(t.pos.y)),
                    ("z", Tag::Int(t.pos.z)),
                    ("t", Tag::Int(t.delay)),
                    ("p", Tag::Int(t.priority as i32)),
                ]))
            })
            .collect(),
    )
}
fn read_ticks(
    tag: Option<&Tag>,
    chunk: (i32, i32),
    id: impl Fn(&str) -> Option<u32>,
) -> Vec<SavedTick> {
    let Some(Tag::List(list)) = tag else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|t| {
            let kind = id(&text(t.get("i"))?)?;
            let pos = BlockPos::new(
                number(t.get("x"))? as i32,
                number(t.get("y"))? as i32,
                number(t.get("z"))? as i32,
            );
            // SavedTick.filterTickListForChunk
            if (pos.x >> 4, pos.z >> 4) != chunk {
                return None;
            }
            Some(SavedTick {
                kind: TickTypeId(kind),
                pos,
                delay: number(t.get("t")).unwrap_or(0) as i32,
                priority: TickPriority::by_value(number(t.get("p")).unwrap_or(0) as i32),
            })
        })
        .collect()
}
fn fluid_name(kind: u32) -> Option<String> {
    use horizon_world::fluid::FluidKind;
    let fluid = *FluidKind::ALL.get(kind as usize)?;
    Some(
        match fluid {
            FluidKind::Empty => return None,
            FluidKind::FlowingWater => "minecraft:flowing_water",
            FluidKind::Water => "minecraft:water",
            FluidKind::FlowingLava => "minecraft:flowing_lava",
            FluidKind::Lava => "minecraft:lava",
        }
        .to_owned(),
    )
}
fn fluid_id(name: &str) -> Option<u32> {
    use horizon_world::fluid::FluidKind;
    Some(match name {
        "minecraft:flowing_water" => FluidKind::FlowingWater,
        "minecraft:water" => FluidKind::Water,
        "minecraft:flowing_lava" => FluidKind::FlowingLava,
        "minecraft:lava" => FluidKind::Lava,
        _ => return None,
    } as u32)
}

pub use horizon_items::item_nbt::{
    decode_items, encode_item, encode_items, item_from_nbt, item_to_nbt,
};
/// The block entity type ID for the block it sits in (a chest-like container
/// is a chest, trapped chest or barrel).
fn block_entity_id(e: &crate::containers::BlockEntity, block: &str) -> String {
    match (e, block) {
        (
            crate::containers::BlockEntity::Chest(_),
            "minecraft:trapped_chest" | "minecraft:barrel" | "minecraft:dropper",
        ) => block.to_owned(),
        (crate::containers::BlockEntity::Chest(_), b) if b.ends_with("shulker_box") => {
            "minecraft:shulker_box".to_owned()
        }
        _ => e.id().to_owned(),
    }
}
pub fn encode_block_entity(
    p: (i32, i32, i32),
    e: &crate::containers::BlockEntity,
    block: &str,
) -> Tag {
    use crate::containers::BlockEntity;
    if let BlockEntity::Mod(m) = e {
        let mut map = m.nbt.clone();
        map.insert("id".into(), string(&m.id));
        if m.items.iter().any(Option::is_some) {
            map.insert("Items".into(), encode_items(&m.items));
        }
        for (key, v) in [("x", p.0), ("y", p.1), ("z", p.2)] {
            map.insert(key.into(), Tag::Int(v));
        }
        return Tag::Compound(map);
    }
    let components = match e {
        BlockEntity::Banner(b) => b.components.clone(),
        _ => Compound::new(),
    };
    let mut entries = vec![
        ("id", string(&block_entity_id(e, block))),
        ("x", Tag::Int(p.0)),
        ("y", Tag::Int(p.1)),
        ("z", Tag::Int(p.2)),
        ("keepPacked", Tag::Byte(0)),
        ("components", Tag::Compound(components)),
    ];
    match e {
        BlockEntity::Chest(items) => entries.push(("Items", encode_items(items))),
        BlockEntity::Furnace(f) => {
            entries.push(("Items", encode_items(&f.items)));
            entries.push(("lit_time_remaining", Tag::Int(f.lit_remaining)));
            entries.push(("lit_total_time", Tag::Int(f.lit_total)));
            entries.push(("cooking_time_spent", Tag::Int(f.cooking_timer)));
            entries.push(("cooking_total_time", Tag::Int(f.cooking_total)));
            entries.push(("speed_multiplier", Tag::Float(f.speed.to_bits())));
            let used: Compound = f
                .recipes_used
                .iter()
                .map(|(id, count)| (id.as_str().into(), Tag::Int(*count as i32)))
                .collect();
            entries.push(("RecipesUsed", Tag::Compound(used)));
        }
        BlockEntity::Brewing(b) => {
            entries.push(("Items", encode_items(&b.items)));
            entries.push(("BrewTime", Tag::Int(b.brew_time)));
            entries.push(("total_brew_time", Tag::Int(b.total_brew_time)));
            entries.push(("Fuel", Tag::Int(b.fuel)));
            entries.push(("total_fuel", Tag::Int(b.total_fuel)));
            entries.push(("speed_multiplier", Tag::Float(b.speed.to_bits())));
        }
        BlockEntity::Sign(sign) => entries.extend(sign.encode()),
        BlockEntity::Jukebox(j) => {
            if let Some(record) = j.record() {
                entries.push(("RecordItem", item_to_nbt(record)));
            }
            entries.push(("ticks_since_song_started", Tag::Long(j.ticks)));
        }
        BlockEntity::Beacon(b) => {
            if let Some(p) = &b.primary {
                entries.push(("primary_effect", string(p)));
            }
            if let Some(s) = &b.secondary {
                entries.push(("secondary_effect", string(s)));
            }
            entries.push(("Levels", Tag::Int(b.levels)));
        }
        BlockEntity::Lectern(l) => {
            if let Some(book) = &l.book {
                entries.push(("Book", item_to_nbt(book)));
                entries.push(("Page", Tag::Int(l.page)));
            }
        }
        BlockEntity::Sculk(s) => entries.extend(s.to_nbt()),
        BlockEntity::Shelf(shelf) => {
            entries.push(("Items", encode_items(&shelf.items)));
            entries.push((
                "align_items_to_bottom",
                Tag::Byte(i8::from(shelf.align_to_bottom)),
            ));
        }
        // ChiseledBookShelfBlockEntity.saveAdditional.
        BlockEntity::ChiseledBookshelf(b) => {
            entries.push(("Items", encode_items(&b.items)));
            entries.push(("last_interacted_slot", Tag::Int(b.last_slot)));
        }
        // CampfireBlockEntity.saveAdditional.
        BlockEntity::Campfire(c) => {
            entries.push(("Items", encode_items(&c.items)));
            entries.push(("CookingTimes", Tag::IntArray(c.progress.to_vec())));
            entries.push(("CookingTotalTimes", Tag::IntArray(c.total.to_vec())));
        }
        BlockEntity::Crafter(c) => {
            entries.push(("Items", encode_items(&c.items)));
            entries.push((
                "disabled_slots",
                Tag::IntArray(
                    (0..9)
                        .filter(|&i| c.disabled[i])
                        .map(|i| i as i32)
                        .collect(),
                ),
            ));
            entries.push(("triggered", Tag::Int(i32::from(c.triggered))));
            entries.push(("crafting_ticks_remaining", Tag::Int(c.crafting_ticks)));
        }
        BlockEntity::Banner(b) => {
            if !b.layers.is_empty() {
                entries.push(("patterns", crate::banner::layers_to_nbt(&b.layers)));
            }
            if let Some(name) = &b.name {
                entries.push(("CustomName", string(name)));
            }
        }
        BlockEntity::CreakingHeart(h) => {
            if let Some((_, uuid)) = h.creaking {
                entries.push(("creaking", uuid_tag(uuid)));
            }
        }
        BlockEntity::Skull(s) => {
            if let Some(profile) = &s.profile {
                entries.push(("profile", profile.clone()));
            }
            if let Some(sound) = &s.note_block_sound {
                entries.push(("note_block_sound", string(sound)));
            }
            if let Some(name) = &s.custom_name {
                entries.push(("custom_name", name.clone()));
            }
        }
        // TheEndGatewayBlockEntity.saveAdditional.
        BlockEntity::EndGateway(g) => {
            entries.push(("Age", Tag::Long(g.age)));
            if let Some(exit) = g.exit_portal {
                entries.push(("exit_portal", Tag::IntArray(exit.to_vec())));
            }
            if g.exact_teleport {
                entries.push(("ExactTeleport", Tag::Byte(1)));
            }
        }
        BlockEntity::Spawner(s) => entries.extend(s.save()),
        BlockEntity::TrialSpawner(s) => entries.extend(s.save()),
        BlockEntity::Vault(v) => entries.extend(v.save()),
        BlockEntity::Beehive(h) => entries.extend(h.to_nbt()),
        BlockEntity::CommandBlock(c) => entries.extend(c.save()),
        BlockEntity::StructureBlock(b) => entries.extend(b.save()),
        BlockEntity::JigsawBlock(b) => entries.extend(b.save()),
        BlockEntity::TestBlock(b) => entries.extend(b.save()),
        BlockEntity::TestInstanceBlock(b) => entries.extend(b.save()),
        BlockEntity::Brushable(b) => {
            if let Some(table) = &b.loot_table {
                entries.push(("LootTable", table.clone()));
                if let Some(seed) = &b.loot_seed {
                    entries.push(("LootTableSeed", seed.clone()));
                }
            } else if let Some(item) = &b.item {
                entries.push(("item", item_to_nbt(item)));
            }
        }
        // Encoded above.
        BlockEntity::DaylightDetector | BlockEntity::Mod(_) => {}
        // PotentSulfurBlockEntity.saveAdditional.
        BlockEntity::PotentSulfur(sulfur) => {
            entries.push(("countdown", Tag::Int(sulfur.waiting_countdown)))
        }
    }
    Tag::compound(entries)
}
/// `block_entity_nbt::overlay`, then a container that now holds items loses its
/// loot table (vanilla clears it when the loot is generated into the container).
fn merge_block_entity(original: &Tag, loaded: &Tag, current: &Tag) -> Tag {
    let mut merged = crate::block_entity_nbt::overlay(original, loaded, current);
    let has_items = matches!(current.get("Items"), Some(Tag::List(items)) if !items.is_empty());
    if has_items && let Tag::Compound(map) = &mut merged {
        map.remove(&"LootTable".into());
        map.remove(&"LootTableSeed".into());
    }
    merged
}
/// `BlockDataAccessor.getData` (`BlockEntity.saveWithFullMetadata`): the tag
/// `data get block` shows and `data merge block` changes, without the
/// chunk-only `keepPacked`, and with the fields Horizon keeps outside its
/// model: a container's `lock` (`LockCode`), from the tag it was loaded or set
/// with.
pub fn block_entity_data(
    p: (i32, i32, i32),
    e: &crate::containers::BlockEntity,
    block: &str,
    lock: Option<&Tag>,
) -> Tag {
    let mut tag = encode_block_entity(p, e, block);
    if let Tag::Compound(map) = &mut tag {
        map.remove(&"keepPacked".into());
        if let Some(lock) = lock {
            map.insert("lock".into(), lock.clone());
        }
    }
    tag
}
/// `BannerBlockEntity.loadAdditional` (and the `components` a block entity keeps).
pub fn decode_banner(tag: &Tag) -> crate::containers::Banner {
    crate::containers::Banner {
        layers: crate::banner::layers_from_nbt(tag.get("patterns")),
        name: text(tag.get("CustomName")),
        components: match tag.get("components") {
            Some(Tag::Compound(c)) => c.clone(),
            _ => Compound::new(),
        },
    }
}
/// A loaded container with a loot table and no items yet, rolled as
/// generation rolls it (Horizon does not unpack on first open).
fn roll_loaded_loot(
    chunk: &Chunk,
    biomes: &Biomes,
    tag: &Tag,
    be: &mut ((i32, i32, i32), crate::containers::BlockEntity),
) -> Option<RolledLoot> {
    let crate::containers::BlockEntity::Chest(slots) = &mut be.1 else {
        return None;
    };
    let table = text(tag.get("LootTable"))?;
    if slots.iter().any(Option::is_some) {
        return None;
    }
    let loot = crate::structure_blocks::loaded_loot_data()?;
    let (x, y, z) = be.0;
    let biome = &biomes.get(chunk.biome(x & 15, y, z & 15)).name;
    let seed = crate::structure_blocks::loot_seed(tag);
    Some(crate::structure_blocks::roll_container(
        slots,
        Some(&loot),
        table,
        seed,
        be.0,
        biome,
    ))
}
/// `TheEndGatewayBlockEntity.loadAdditional`.
pub fn decode_end_gateway(tag: &Tag) -> crate::containers::EndGateway {
    crate::containers::EndGateway {
        age: number(tag.get("Age")).unwrap_or(0),
        exit_portal: tag
            .get("exit_portal")
            .and_then(crate::block_entity_nbt::gateway_exit),
        exact_teleport: number(tag.get("ExactTeleport")).unwrap_or(0) != 0,
    }
}
/// A mod's block entity: its compound without `id` and the position, and
/// its `Items` as slots (`ContainerHelper.loadAllItems`; the game sizes them
/// to the block's container when it uses them). `Items` that are not an item
/// list saved exactly so stay the mod's own.
fn decode_mod_block_entity(id: String, tag: &Tag) -> Option<crate::containers::ModBlockEntity> {
    let Tag::Compound(map) = tag else {
        return None;
    };
    let mut nbt = map.clone();
    for key in ["id", "x", "y", "z"] {
        nbt.remove(&key.into());
    }
    // `Slot` is a byte: 256 slots at most.
    let mut items = decode_items(nbt.get(&"Items".into()), 256);
    if nbt.get(&"Items".into()) == Some(&encode_items(&items)) {
        nbt.remove(&"Items".into());
        while items.last().is_some_and(Option::is_none) {
            items.pop();
        }
    } else {
        items.clear();
    }
    Some(
        crate::containers::ModBlockEntityFields {
            id,
            nbt,
            items,
            data: Vec::new(),
        }
        .into(),
    )
}
pub fn decode_block_entity(tag: &Tag) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    use crate::containers::BlockEntity;
    let int = |k: &str| number(tag.get(k)).map(|v| v as i32);
    let id = text(tag.get("id"))?;
    let pos = (int("x")?, int("y")?, int("z")?);
    if id.contains(':') && !id.starts_with("minecraft:") {
        return Some((
            pos,
            BlockEntity::Mod(Box::new(decode_mod_block_entity(id, tag)?)),
        ));
    }
    if id == "minecraft:end_gateway" {
        return Some((pos, BlockEntity::EndGateway(decode_end_gateway(tag))));
    }
    if id == "minecraft:jukebox" {
        let record = tag.get("RecordItem").and_then(item_from_nbt);
        let ticks = number(tag.get("ticks_since_song_started")).unwrap_or(0);
        let playing = record.is_some();
        return Some((
            pos,
            BlockEntity::Jukebox(crate::containers::Jukebox {
                items: [record],
                ticks,
                playing,
            }),
        ));
    }
    if id == "minecraft:creaking_heart" {
        let heart = crate::containers::CreakingHeart {
            creaking: uuid_from_tag(tag.get("creaking")).map(|uuid| (None, uuid)),
            ..Default::default()
        };
        return Some((pos, BlockEntity::CreakingHeart(Box::new(heart))));
    }
    if id == "minecraft:lectern" {
        let lectern = crate::containers::Lectern {
            book: tag.get("Book").and_then(item_from_nbt),
            page: int("Page").unwrap_or(0),
        };
        return Some((pos, BlockEntity::Lectern(lectern)));
    }
    if id == "minecraft:banner" {
        return Some((pos, BlockEntity::Banner(decode_banner(tag))));
    }
    if id == "minecraft:beacon" {
        let beacon = crate::containers::Beacon {
            levels: int("Levels").unwrap_or(0),
            primary: text(tag.get("primary_effect")),
            secondary: text(tag.get("secondary_effect")),
            last_check_y: i32::MIN / 2,
            ..Default::default()
        };
        return Some((pos, BlockEntity::Beacon(Box::new(beacon))));
    }
    // A skull's block entity id is not its block's name (every head block).
    if id == "minecraft:skull" {
        return Some((
            pos,
            BlockEntity::Skull(crate::containers::Skull::new(
                tag.get("profile").cloned(),
                text(tag.get("note_block_sound")),
                tag.get("custom_name")
                    .and_then(crate::block_entity_nbt::component),
            )),
        ));
    }
    // A spawner's block entity id is not its block's name.
    let block = match id.as_str() {
        "minecraft:mob_spawner" => "minecraft:spawner",
        other => other,
    };
    let mut entity = BlockEntity::for_block(block)?;
    match &mut entity {
        BlockEntity::Chest(items) => *items = decode_items(tag.get("Items"), items.len()),
        BlockEntity::Furnace(f) => {
            let items = decode_items(tag.get("Items"), 3);
            f.items = [items[0].clone(), items[1].clone(), items[2].clone()];
            f.lit_remaining = int("lit_time_remaining").unwrap_or(0);
            f.lit_total = int("lit_total_time").unwrap_or(0);
            f.cooking_timer = int("cooking_time_spent").unwrap_or(0);
            f.cooking_total = int("cooking_total_time").unwrap_or(0);
            if let Some(speed) = float(tag.get("speed_multiplier")) {
                f.speed = speed as f32;
            }
            // A save without it owes nothing: the field is newer than some worlds.
            if let Some(Tag::Compound(used)) = tag.get("RecipesUsed") {
                for (id, count) in used {
                    let count = match count {
                        Tag::Int(n) => *n,
                        _ => continue,
                    };
                    // Saves from before the key was namespaced hold both forms of an id.
                    *f.recipes_used
                        .entry(crate::containers::recipe_used_key(&id.to_string_lossy()))
                        .or_insert(0) += count.max(0) as u32;
                }
            }
        }
        BlockEntity::Brewing(b) => {
            let items = decode_items(tag.get("Items"), 5);
            b.items = [
                items[0].clone(),
                items[1].clone(),
                items[2].clone(),
                items[3].clone(),
                items[4].clone(),
            ];
            b.brew_time = int("BrewTime").unwrap_or(0);
            b.total_brew_time = int("total_brew_time").unwrap_or(400);
            if b.brew_time > 0 {
                b.ingredient = b.items[3].as_ref().map(|s| s.item.clone());
            }
            b.fuel = int("Fuel").unwrap_or(0);
            b.total_fuel = int("total_fuel").unwrap_or(20);
            b.speed = match tag.get("speed_multiplier") {
                Some(Tag::Float(f)) => f32::from_bits(*f),
                _ => 1.0,
            };
        }
        BlockEntity::Brushable(b) => {
            b.loot_table = tag.get("LootTable").cloned();
            b.loot_seed = tag.get("LootTableSeed").cloned();
            b.item = if b.loot_table.is_none() {
                tag.get("item").and_then(item_from_nbt)
            } else {
                None
            };
            b.hit_direction = int("hit_direction").filter(|v| (0..=5).contains(v));
        }
        BlockEntity::Sign(sign) => sign.decode(tag),
        BlockEntity::Sculk(s) => s.load(tag),
        BlockEntity::Shelf(shelf) => {
            for (slot, item) in decode_items(tag.get("Items"), 3).into_iter().enumerate() {
                shelf.items[slot] = item;
            }
            shelf.align_to_bottom = int("align_items_to_bottom").unwrap_or(0) != 0;
        }
        // ChiseledBookShelfBlockEntity.loadAdditional.
        BlockEntity::ChiseledBookshelf(b) => {
            for (slot, item) in decode_items(tag.get("Items"), 6).into_iter().enumerate() {
                b.items[slot] = item;
            }
            b.last_slot = int("last_interacted_slot").unwrap_or(-1);
        }
        // CampfireBlockEntity.loadAdditional: missing time arrays stay zeros.
        BlockEntity::Campfire(c) => {
            for (slot, item) in decode_items(tag.get("Items"), 4).into_iter().enumerate() {
                c.items[slot] = item;
            }
            let ints = |key: &str| match tag.get(key) {
                Some(Tag::IntArray(v)) => std::array::from_fn(|i| v.get(i).copied().unwrap_or(0)),
                _ => [0; 4],
            };
            c.progress = ints("CookingTimes");
            c.total = ints("CookingTotalTimes");
        }
        BlockEntity::Crafter(c) => {
            for (slot, item) in decode_items(tag.get("Items"), 9).into_iter().enumerate() {
                c.items[slot] = item;
            }
            if let Some(Tag::IntArray(slots)) = tag.get("disabled_slots") {
                for &slot in slots {
                    if (0..9).contains(&slot) && c.items[slot as usize].is_none() {
                        c.disabled[slot as usize] = true;
                    }
                }
            }
            c.triggered = int("triggered").unwrap_or(0) != 0;
            c.state_triggered = None;
            c.crafting_ticks = int("crafting_ticks_remaining").unwrap_or(0);
        }
        // PotentSulfurBlockEntity.loadAdditional.
        BlockEntity::PotentSulfur(sulfur) => {
            if let Some(countdown) = int("countdown") {
                sulfur.waiting_countdown = countdown;
            }
        }
        BlockEntity::Jukebox(_)
        | BlockEntity::Beacon(_)
        | BlockEntity::Lectern(_)
        | BlockEntity::Banner(_)
        | BlockEntity::CreakingHeart(_)
        | BlockEntity::EndGateway(_)
        | BlockEntity::Skull(_)
        | BlockEntity::DaylightDetector
        | BlockEntity::Mod(_) => {}
        BlockEntity::Spawner(s) => **s = crate::mobs::spawner::Spawner::load(tag),
        BlockEntity::TrialSpawner(s) => **s = crate::mobs::trial_spawner::TrialSpawner::load(tag),
        BlockEntity::Vault(v) => **v = crate::vault::Vault::load(tag),
        BlockEntity::Beehive(h) => **h = crate::beehive::Beehive::from_nbt(tag),
        BlockEntity::CommandBlock(c) => **c = crate::command_block::CommandBlock::load(tag),
        BlockEntity::StructureBlock(b) => **b = crate::structure_block::StructureBlock::load(tag),
        BlockEntity::TestBlock(b) => **b = crate::test_block::TestBlock::load(tag),
        BlockEntity::TestInstanceBlock(b) => b.load(tag),
        // The state is not known here; a saved jigsaw always has its joint.
        BlockEntity::JigsawBlock(b) => {
            **b = crate::jigsaw_block::JigsawBlock::load(
                tag,
                crate::jigsaw_block::JointType::Rollable,
            )
        }
    }
    Some((pos, entity))
}

/// The chunk's points of interest as `PoiManager` saves them: each section
/// with POI blocks, valid, with a record per block at its max tickets (a
/// freshly made world). None when the chunk has none.
pub fn encode_poi(chunk: &Chunk, tables: &SaveTables) -> Option<NamedTag> {
    let kind = |v: u32| tables.poi.get(v as usize).copied().flatten();
    let mut sections = Compound::new();
    let mut states = [0u32; 4096];
    for (si, section) in chunk.sections.iter().enumerate() {
        if !section.blocks.maybe_has(|v| kind(v).is_some()) {
            continue;
        }
        section.unpack(&mut states);
        let y = (chunk.min_y() >> 4) + si as i32;
        let records: Vec<Tag> = states
            .iter()
            .enumerate()
            .filter_map(|(i, &v)| {
                let (id, tickets) = kind(v)?;
                let i = i as i32;
                let pos = [
                    chunk.x * 16 + (i & 15),
                    y * 16 + (i >> 8),
                    chunk.z * 16 + ((i >> 4) & 15),
                ];
                let mut record = vec![("pos", Tag::IntArray(pos.to_vec())), ("type", string(id))];
                // PoiRecord.CODEC: free_tickets defaults to 0 and is then left out.
                if tickets != 0 {
                    record.push(("free_tickets", Tag::Int(tickets)));
                }
                Some(Tag::compound(record))
            })
            .collect();
        if !records.is_empty() {
            sections.insert(
                y.to_string().as_str().into(),
                Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(records))]),
            );
        }
    }
    (!sections.is_empty()).then(|| poi_root(sections))
}
/// A record's `free_tickets` from the POI file the chunk had, while the same
/// POI is still there: a bed or job site a villager claimed stays claimed.
fn keep_claims(saved: &mut NamedTag, old: &NamedTag) {
    let key = |r: &Tag| match (r.get("pos"), text(r.get("type"))) {
        (Some(Tag::IntArray(pos)), Some(kind)) => Some((pos.clone(), kind)),
        _ => None,
    };
    let mut claims = HashMap::new();
    if let Some(Tag::Compound(sections)) = old.tag.get("Sections") {
        for section in sections.values() {
            if let Some(Tag::List(records)) = section.get("Records") {
                for r in records {
                    if let Some(k) = key(r) {
                        claims.insert(k, number(r.get("free_tickets")).unwrap_or(0) as i32);
                    }
                }
            }
        }
    }
    let Tag::Compound(root) = &mut saved.tag else {
        return;
    };
    let Some(Tag::Compound(sections)) = root.get_mut(&"Sections".into()) else {
        return;
    };
    for section in sections.values_mut() {
        let Tag::Compound(section) = section else {
            continue;
        };
        let Some(Tag::List(records)) = section.get_mut(&"Records".into()) else {
            continue;
        };
        for r in records {
            let Some(&tickets) = key(r).and_then(|k| claims.get(&k)) else {
                continue;
            };
            if let Tag::Compound(r) = r {
                if tickets == 0 {
                    r.remove(&"free_tickets".into());
                } else {
                    r.insert("free_tickets".into(), Tag::Int(tickets));
                }
            }
        }
    }
}
fn poi_root(sections: Compound) -> NamedTag {
    NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("Sections", Tag::Compound(sections)),
        ]),
    }
}

pub use crate::chunk::LevelShape;

/// A column's sections as `Heightmap.primeHeightmaps` reads them.
struct HeightmapView<'a> {
    sections: &'a [Arc<Section>],
    /// The minimum Y of `sections[0]`.
    column_min_y: i32,
    tables: &'a SaveTables,
    min_y: i32,
    height: i32,
}
impl HeightmapChunk for HeightmapView<'_> {
    type State = crate::assets::StateFacts;
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.height
    }
    fn highest_section_y(&self) -> i32 {
        let top = self
            .sections
            .iter()
            .rposition(|s| !s.is_uniform(self.tables.air))
            .unwrap_or(0);
        self.column_min_y + top as i32 * 16
    }
    fn block_state(&self, x: i32, y: i32, z: i32) -> Self::State {
        // Chunk.get: no block outside the sections.
        let state = usize::try_from((y - self.column_min_y) >> 4)
            .ok()
            .and_then(|s| self.sections.get(s))
            .map_or(0, |s| s.get(crate::chunk::index(x, y, z)));
        self.tables.heightmaps.heightmap_state(BlockStateId(state))
    }
}
/// Heightmaps kept after world generation (`ChunkStatus.FULL.heightmapsAfter`).
fn heightmaps(chunk: &Chunk, tables: &SaveTables, shape: LevelShape) -> Tag {
    let kinds = [
        HeightmapType::WorldSurface,
        HeightmapType::OceanFloor,
        HeightmapType::MotionBlocking,
        HeightmapType::MotionBlockingNoLeaves,
    ];
    let mut maps: Vec<Heightmap> = kinds
        .iter()
        .filter_map(|k| Heightmap::new(*k, shape.min_y, shape.height).ok())
        .collect();
    let view = HeightmapView {
        sections: &chunk.sections,
        column_min_y: chunk.min_y(),
        tables,
        min_y: shape.min_y,
        height: shape.height,
    };
    let mut refs: Vec<&mut Heightmap> = maps.iter_mut().collect();
    if Heightmap::prime(&view, &mut refs).is_err() {
        return Tag::Compound(Compound::new());
    }
    Tag::compound(maps.iter().map(|m| {
        (
            m.kind().name(),
            Tag::LongArray(m.raw_data().iter().map(|&w| w as i64).collect()),
        )
    }))
}
/// The client heightmaps of a column's sections (from `column_min_y`), in a
/// level from `min_y`, `height` blocks high.
pub fn client_heightmaps_in(
    sections: &[Arc<Section>],
    column_min_y: i32,
    tables: &SaveTables,
    min_y: i32,
    height: i32,
) -> Vec<(i32, Vec<i64>)> {
    let kinds = [
        (1, HeightmapType::WorldSurface),
        (4, HeightmapType::MotionBlocking),
        (5, HeightmapType::MotionBlockingNoLeaves),
    ];
    let mut maps: Vec<(i32, Heightmap)> = kinds
        .iter()
        .filter_map(|(id, k)| Heightmap::new(*k, min_y, height).ok().map(|m| (*id, m)))
        .collect();
    let view = HeightmapView {
        sections,
        column_min_y,
        tables,
        min_y,
        height,
    };
    let mut refs: Vec<&mut Heightmap> = maps.iter_mut().map(|(_, m)| m).collect();
    if Heightmap::prime(&view, &mut refs).is_err() {
        return Vec::new();
    }
    maps.iter()
        .map(|(id, m)| (*id, m.raw_data().iter().map(|&w| w as i64).collect()))
        .collect()
}
/// Vanilla light layer (nibbles, index y<<8|z<<4|x, low nibble first).
fn nibbles(light: &Light, sky: bool) -> Vec<u8> {
    let mut out = vec![0u8; 2048];
    for (i, byte) in out.iter_mut().enumerate() {
        let value = |j: usize| {
            let v = match light {
                Light::Uniform(v) => *v,
                Light::Full(a) => a[j],
            };
            if sky { v >> 4 } else { v & 15 }
        };
        *byte = value(2 * i) | value(2 * i + 1) << 4;
    }
    out
}

pub fn encode(chunk: &Chunk, registry: &BlockRegistry, biomes: &Biomes) -> NamedTag {
    encode_with(chunk, registry, biomes, None, LevelShape::OVERWORLD)
}
/// The chunk NBT vanilla's `SerializableChunkData.write` produces for a full
/// chunk in a level of `shape`.
pub fn encode_with(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
) -> NamedTag {
    let sky_light = shape.sky_light;
    let mut sections = Vec::new();
    let air = tables.map_or(0, |t| t.air);
    // Sky layers below the highest section with light other than 15 are
    // written (even all-zero ones): vanilla derives missing layers from the
    // layer above them.
    let top_sky = chunk.sections.iter().rposition(|s| match &s.light {
        Light::Uniform(v) => v >> 4 != 15,
        Light::Full(a) => a.iter().any(|v| v >> 4 != 15),
    });
    for (si, section) in chunk.sections.iter().enumerate() {
        let y = (chunk.min_y() >> 4) + si as i32;
        if !shape.has_section(y) {
            continue;
        }
        // PalettedContainer.pack: distinct states in index order, indices at
        // the saved width.
        let packed = section.blocks.pack();
        let mut states = vec![(
            "palette",
            Tag::List(
                packed
                    .palette
                    .iter()
                    .map(|&id| {
                        let state = registry.state(BlockStateId(id));
                        match state {
                            // BlockState.CODEC: the block name for default states.
                            Some(s)
                                if tables.is_some_and(|t| {
                                    t.default_state.get(id as usize) == Some(&true)
                                }) =>
                            {
                                string(&s.name)
                            }
                            Some(s) => s.to_nbt(),
                            None => string("minecraft:air"),
                        }
                    })
                    .collect(),
            ),
        )];
        if let Some(data) = &packed.data {
            states.push((
                "data",
                Tag::LongArray(data.iter().map(|&w| w as i64).collect()),
            ));
        }
        let biome_packed = section.biomes.pack();
        let mut biome_tag = vec![(
            "palette",
            Tag::List(
                biome_packed
                    .palette
                    .iter()
                    .map(|&b| string(&biomes.get(b as u16).name))
                    .collect(),
            ),
        )];
        if let Some(data) = &biome_packed.data {
            biome_tag.push((
                "data",
                Tag::LongArray(data.iter().map(|&w| w as i64).collect()),
            ));
        }
        let mut entries = vec![
            ("Y", Tag::Byte(y as i8)),
            ("block_states", Tag::compound(states)),
            ("biomes", Tag::compound(biome_tag)),
        ];
        if chunk.lit {
            let block_lit = match &section.light {
                Light::Uniform(v) => v & 15 != 0,
                Light::Full(a) => a.iter().any(|v| v & 15 != 0),
            };
            if block_lit {
                entries.push(("BlockLight", Tag::ByteArray(nibbles(&section.light, false))));
            }
            if sky_light && top_sky.is_some_and(|top| si <= top + 1) {
                entries.push(("SkyLight", Tag::ByteArray(nibbles(&section.light, true))));
            }
        }
        let _ = air;
        sections.push(Tag::compound(entries));
    }
    let extras = chunk.saved.as_deref();
    let block_name = |p: &(i32, i32, i32)| -> String {
        registry
            .state(chunk.block(p.0 & 15, p.1, p.2 & 15))
            .map_or_else(String::new, |s| s.name.clone())
    };
    let mut block_entities: Vec<Tag> = chunk
        .block_entities
        .iter()
        // A mod block entity a native mod is ticking right now
        // (`tick_mod_block_entity`): its data is out of the world.
        .filter(|(_, e)| !matches!(e, crate::containers::BlockEntity::Mod(m) if m.id.is_empty()))
        .map(|(p, e)| {
            let block = block_name(p);
            let mut current = encode_block_entity(*p, e, &block);
            if let Some((_, name)) = chunk.block_names.iter().find(|(q, _)| q == p)
                && let Tag::Compound(map) = &mut current
            {
                map.insert("CustomName".into(), string(name));
            }
            let source = extras
                .and_then(|x| x.block_entity_sources.iter().find(|(q, _)| q == p))
                .map(|(_, tag)| tag)
                .filter(|tag| {
                    crate::block_entity_nbt::id(tag) == crate::block_entity_nbt::id(&current)
                });
            let merged = match source {
                Some(original) => {
                    let loaded = decode_block_entity(original).map_or_else(
                        || current.clone(),
                        |(q, e)| encode_block_entity(q, &e, &block),
                    );
                    merge_block_entity(original, &loaded, &current)
                }
                None => current,
            };
            let mut saved = crate::block_entity_nbt::canonical(&merged).unwrap_or(merged);
            if let Some((_, attachments)) =
                extras.and_then(|e| e.block_entity_attachments.iter().find(|(pos, _)| pos == p))
                && let Tag::Compound(map) = &mut saved
                && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
            {
                let data = attachments.to_nbt();
                if data.is_empty() {
                    map.remove(&ATTACHMENTS_KEY.into());
                } else {
                    map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
                }
            }
            if let Some(r) = extras.and_then(|x| x.rolled_loot.iter().find(|r| r.pos == *p))
                && let Tag::Compound(map) = &mut saved
                && !r.restore(map)
            {
                // A loaded source still carries the table; kept, it rolls again on load.
                map.remove(&"LootTable".into());
                map.remove(&"LootTableSeed".into());
            }
            saved
        })
        .collect();
    if let Some(extras) = extras {
        for raw in &extras.block_entities {
            let pos = (
                number(raw.get("x")).unwrap_or(0) as i32,
                number(raw.get("y")).unwrap_or(0) as i32,
                number(raw.get("z")).unwrap_or(0) as i32,
            );
            let state = chunk.get(pos.0 & 15, pos.1, pos.2 & 15) as usize;
            let still_there =
                tables.is_none_or(|t| t.block_entity.get(state).copied().unwrap_or(true));
            if still_there && !chunk.block_entities.iter().any(|(p, _)| *p == pos) {
                block_entities
                    .push(crate::block_entity_nbt::canonical(raw).unwrap_or_else(|| raw.clone()));
            }
        }
    }
    let mut root = extras
        .map(|e| Tag::Compound(e.root.clone()))
        .unwrap_or_else(|| Tag::Compound(Compound::new()));
    for (key, value) in [
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("xPos", Tag::Int(chunk.x)),
        ("zPos", Tag::Int(chunk.z)),
        ("yPos", Tag::Int(shape.min_section())),
        ("Status", string("minecraft:full")),
        ("LastUpdate", Tag::Long(extras.map_or(0, |e| e.last_update))),
        ("isLightOn", Tag::Byte(i8::from(chunk.lit))),
        ("sections", Tag::List(sections)),
        ("block_entities", Tag::List(block_entities)),
    ] {
        put(&mut root, key, value);
    }
    if let Some(Tag::Compound(map)) = Some(&mut root) {
        if let Some(extras) = extras
            && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
        {
            let attachments = extras.attachments.to_nbt();
            if attachments.is_empty() {
                map.remove(&ATTACHMENTS_KEY.into());
            } else {
                map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(attachments));
            }
        }
        map.entry("InhabitedTime".into()).or_insert(Tag::Long(0));
        map.entry("structures".into()).or_insert_with(|| {
            Tag::compound([
                ("References", Tag::Compound(Compound::new())),
                ("starts", Tag::Compound(Compound::new())),
            ])
        });
        if !chunk.structure_refs.is_empty()
            && let Some(Tag::Compound(s)) = map.get_mut(&"structures".into())
        {
            let mut refs: std::collections::BTreeMap<&str, Vec<i64>> = Default::default();
            for (id, (x, z)) in &chunk.structure_refs {
                refs.entry(id)
                    .or_default()
                    .push(i64::from(*x as u32) | i64::from(*z) << 32);
            }
            let refs: Compound = refs
                .into_iter()
                .map(|(id, v)| (id.into(), Tag::LongArray(v)))
                .collect();
            s.insert("References".into(), Tag::Compound(refs));
        }
        let starts: Compound = chunk
            .structure_starts
            .iter()
            .filter_map(|start| match start.save() {
                Some(tag) => Some((start.name.as_str().into(), tag)),
                None => {
                    log::warn!(
                        "Horizon: structure start {} not saved: one of its pieces cannot be",
                        start.name
                    );
                    None
                }
            })
            .collect();
        if !starts.is_empty()
            && let Some(Tag::Compound(s)) = map.get_mut(&"structures".into())
        {
            s.insert("starts".into(), Tag::Compound(starts));
        }
        map.entry("PostProcessing".into())
            .or_insert_with(|| Tag::List(vec![Tag::List(Vec::new()); shape.section_count()]));
    }
    if let Some(t) = tables {
        put(&mut root, "Heightmaps", heightmaps(chunk, t, shape));
        let names = |k: u32| t.block_names.get(k as usize).cloned();
        let (blocks, fluids) = extras.map_or((&[][..], &[][..]), |e| {
            (&e.block_ticks[..], &e.fluid_ticks[..])
        });
        put(&mut root, "block_ticks", tick_list(blocks, names));
        put(&mut root, "fluid_ticks", tick_list(fluids, fluid_name));
    }
    NamedTag {
        name: "".into(),
        tag: root,
    }
}
/// Root keys `encode_with` regenerates; everything else is kept as loaded.
const GENERATED_KEYS: [&str; 14] = [
    "DataVersion",
    "xPos",
    "zPos",
    "yPos",
    "Status",
    "LastUpdate",
    "isLightOn",
    "sections",
    "block_entities",
    "Heightmaps",
    "block_ticks",
    "fluid_ticks",
    "entities",
    "CarvingMask",
];
pub fn decode(root: &NamedTag, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
    decode_with(root, registry, biomes, None, ColumnLayout::VANILLA)
}
/// Only full chunks load here; partially generated ones (vanilla's world
/// border of proto chunks) resume generation through `decode_proto`.
pub fn decode_with(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<Chunk> {
    if text(root.tag.get("Status")).is_some_and(|s| s != "minecraft:full" && s != "full") {
        return None;
    }
    decode_column(root, registry, biomes, tables, layout)
}
/// `decode_with` whatever the chunk's status, into a chunk of `layout`
/// (sections outside it are left out).
fn decode_column(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<Chunk> {
    let tag = &root.tag;
    let Tag::Int(x) = tag.get("xPos")? else {
        return None;
    };
    let Tag::Int(z) = tag.get("zPos")? else {
        return None;
    };
    let mut chunk = Chunk::with_layout(*x, *z, layout);
    // structures.References: structure id -> packed start chunk positions.
    if let Some(Tag::Compound(refs)) = tag.get("structures").and_then(|s| s.get("References")) {
        for (id, v) in refs {
            if let Tag::LongArray(starts) = v {
                let id: Arc<str> = Arc::from(id.to_string_lossy());
                for &p in starts {
                    chunk
                        .structure_refs
                        .push((id.clone(), (p as i32, (p >> 32) as i32)));
                }
            }
        }
    }
    let air = registry.default_state("minecraft:air")?.0;
    let mut cache: HashMap<String, u32> = HashMap::new();
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return Some(chunk);
    };
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let Some(si) = layout.section_index(y as i32 * 16) else {
            continue;
        };
        if let Some(states) = section.get("block_states")
            && let Some(Tag::List(palette)) = states.get("palette")
        {
            let palette: Vec<u32> = palette
                .iter()
                .map(|p| {
                    let state = BlockState::from_nbt(p).ok();
                    let key = state
                        .as_ref()
                        .map(|s| format!("{}{:?}", s.name, s.properties))
                        .unwrap_or_default();
                    *cache.entry(key).or_insert_with(|| {
                        state
                            .and_then(|s| {
                                registry.id(&s).or_else(|| {
                                    // Missing properties take the block's defaults.
                                    let default = registry.default_state(&s.name)?;
                                    let mut full = registry.state(default)?.clone();
                                    for (k, v) in &s.properties {
                                        if full.properties.contains_key(k) {
                                            full.properties.insert(k.clone(), v.clone());
                                        }
                                    }
                                    registry.id(&full).or(Some(default))
                                })
                            })
                            .map_or(air, |id| id.0)
                    })
                })
                .collect();
            let target = Arc::make_mut(&mut chunk.sections[si]);
            let data = match states.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            if let Ok(blocks) =
                PalettedContainer::from_packed(block_strategy(), palette, data.as_deref())
            {
                target.blocks = blocks;
            }
        }
        if let Some(b) = section.get("biomes")
            && let Some(Tag::List(palette)) = b.get("palette")
        {
            let ids: Vec<u16> = palette
                .iter()
                .map(|p| match p {
                    Tag::String(s) => biomes.id(&s.to_string_lossy()),
                    _ => 0,
                })
                .collect();
            let data = match b.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            let palette: Vec<u32> = ids.iter().map(|&id| u32::from(id)).collect();
            let target = Arc::make_mut(&mut chunk.sections[si]);
            if let Ok(container) =
                PalettedContainer::from_packed(BIOME_STRATEGY, palette.clone(), data.as_deref())
            {
                target.biomes = container;
            } else if let Some(&first) = palette.first() {
                target.biomes = PalettedContainer::new(BIOME_STRATEGY, first);
            }
        }
    }
    let mut extras = ChunkExtras {
        last_update: number(tag.get("LastUpdate")).unwrap_or(0),
        ..ChunkExtras::default()
    };
    if let Some(Tag::List(entities)) = tag.get("block_entities") {
        for e in entities {
            let int = |k: &str| number(e.get(k)).map(|v| v as i32);
            if let (Some(x), Some(y), Some(z)) = (int("x"), int("y"), int("z")) {
                extras.block_entity_order.push((x, y, z));
            }
            match decode_block_entity(e) {
                Some(mut be) => {
                    if let Ok(Some(data)) = attachment_compound(e.get(ATTACHMENTS_KEY)) {
                        extras.block_entity_attachments.push((
                            be.0,
                            horizon_world::attachments::Attachments::from_nbt(data.clone()),
                        ));
                    }
                    if let Some(name) = text(e.get("CustomName")) {
                        chunk.block_names.push((be.0, name));
                    }
                    if let Some(rolled) = roll_loaded_loot(&chunk, biomes, e, &mut be) {
                        extras.rolled_loot.push(rolled);
                    }
                    extras.block_entity_sources.push((be.0, e.clone()));
                    chunk.block_entities.push(be);
                }
                None => extras.block_entities.push(e.clone()),
            }
        }
    }
    if let Tag::Compound(map) = tag {
        if let Ok(Some(data)) = attachment_compound(map.get(&ATTACHMENTS_KEY.into())) {
            extras.attachments = horizon_world::attachments::Attachments::from_nbt(data.clone());
        }
        extras.root = map
            .iter()
            .filter(|(k, _)| !GENERATED_KEYS.contains(&k.to_string_lossy().as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }
    if let Some(t) = tables {
        extras.block_ticks = read_ticks(tag.get("block_ticks"), (*x, *z), |n| {
            t.block_ids.get(n).map(|&i| u32::from(i))
        });
        extras.fluid_ticks = read_ticks(tag.get("fluid_ticks"), (*x, *z), fluid_id);
    }
    chunk.saved = Some(Arc::new(extras));
    Some(chunk)
}

pub fn encode_proto(
    proto: &SavedProto,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
) -> NamedTag {
    let mut root = encode_with(&proto.column, registry, biomes, tables, shape);
    let status = if proto.decorated {
        ChunkStatus::Features
    } else {
        ChunkStatus::Terrain
    };
    // ProtoChunk.postProcessing: one list per section, packed section offsets.
    let mut marks = vec![Vec::new(); shape.section_count()];
    for &[x, y, z] in &proto.column.postprocess {
        let section = usize::try_from((y - shape.min_y) >> 4).ok();
        if let Some(list) = section.and_then(|s| marks.get_mut(s)) {
            list.push(Tag::Short(
                ((x & 15) | (y & 15) << 4 | (z & 15) << 8) as i16,
            ));
        }
    }
    for (key, value) in [
        ("Status", string(status.name())),
        ("isLightOn", Tag::Byte(0)),
        ("block_entities", Tag::List(proto.block_entities.clone())),
        ("entities", Tag::List(proto.entities.clone())),
        (
            "PostProcessing",
            Tag::List(marks.into_iter().map(Tag::List).collect()),
        ),
    ] {
        put(&mut root.tag, key, value);
    }
    root
}
/// A proto chunk generation can resume: `terrain` (written to by a
/// neighbour's decoration) up to `spawn`. Earlier statuses hold nothing that
/// generating the chunk again doesn't reproduce.
pub fn decode_proto(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<SavedProto> {
    let status = ChunkStatus::parse(&text(root.tag.get("Status"))?).ok()?;
    if !(ChunkStatus::Terrain..ChunkStatus::Full).contains(&status) {
        return None;
    }
    let mut column = decode_column(root, registry, biomes, tables, layout)?;
    column.block_entities.clear();
    let base = number(root.tag.get("yPos")).map_or(column.min_y(), |y| y as i32 * 16);
    if let Some(Tag::List(sections)) = root.tag.get("PostProcessing") {
        for (si, marks) in sections.iter().enumerate() {
            let Tag::List(marks) = marks else {
                continue;
            };
            for mark in marks {
                if let Tag::Short(p) = mark {
                    let p = i32::from(*p);
                    column.postprocess.push([
                        column.x * 16 + (p & 15),
                        base + si as i32 * 16 + ((p >> 4) & 15),
                        column.z * 16 + ((p >> 8) & 15),
                    ]);
                }
            }
        }
    }
    let list = |key: &str| match root.tag.get(key) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    Some(SavedProto {
        column,
        decorated: status >= ChunkStatus::Features,
        block_entities: list("block_entities"),
        entities: list("entities"),
    })
}

/// Read the stored `SkyLight`/`BlockLight` layers of a chunk saved with
/// `isLightOn` (`SerializableChunkData.read` + `ThreadedLevelLightEngine`),
/// so it needs no relighting. A missing sky layer repeats the bottom layer of
/// the section above it (15 above the highest stored one); a missing block
/// layer is dark.
pub fn load_light(tag: &Tag, chunk: &mut Chunk, sky_light: bool) {
    if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
        return;
    }
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return;
    };
    let mut layers: Vec<(Option<&[u8]>, Option<&[u8]>)> = vec![(None, None); chunk.sections.len()];
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let Some(si) = chunk.section_index(y as i32 * 16) else {
            continue;
        };
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
            _ => None,
        };
        layers[si] = (layer("SkyLight"), layer("BlockLight"));
    }
    // Uniform saved layers need no expanded allocation. Both nibbles of
    // each byte must match, not merely the bytes across the layer.
    let uniform_layer = |layer: &[u8]| {
        let first = layer[0];
        (first & 15 == first >> 4 && layer.iter().all(|&v| v == first)).then_some(first & 15)
    };
    let mut above = [if sky_light { 15 } else { 0 }; 256];
    for (si, (sky, block)) in layers.into_iter().enumerate().rev() {
        let sky = sky.filter(|_| sky_light);
        let sky_uniform = sky.map_or_else(
            || above.iter().all(|&v| v == above[0]).then_some(above[0]),
            uniform_layer,
        );
        let block_uniform = block.map_or(Some(0), uniform_layer);
        let section = Arc::make_mut(&mut chunk.sections[si]);
        if let (Some(s), Some(b)) = (sky_uniform, block_uniform) {
            section.light = Light::Uniform(s << 4 | b);
            above.fill(s);
        } else {
            let mut data = Box::new([0u8; 4096]);
            match (sky, block) {
                (Some(sky), Some(block)) => {
                    for ((pair, &s), &b) in
                        data.as_chunks_mut::<2>().0.iter_mut().zip(sky).zip(block)
                    {
                        pair[0] = s << 4 | (b & 15);
                        pair[1] = (s & 0xf0) | (b >> 4);
                    }
                }
                (Some(sky), None) => {
                    for (pair, &s) in data.as_chunks_mut::<2>().0.iter_mut().zip(sky) {
                        pair[0] = s << 4;
                        pair[1] = s & 0xf0;
                    }
                }
                (None, Some(block)) => {
                    for (i, (pair, &b)) in data
                        .as_chunks_mut::<2>()
                        .0
                        .iter_mut()
                        .zip(block)
                        .enumerate()
                    {
                        pair[0] = above[(i * 2) & 255] << 4 | (b & 15);
                        pair[1] = above[(i * 2 + 1) & 255] << 4 | (b >> 4);
                    }
                }
                (None, None) => {
                    for layer in data.as_chunks_mut::<256>().0 {
                        for (v, &s) in layer.iter_mut().zip(&above) {
                            *v = s << 4;
                        }
                    }
                }
            }
            if let Some(sky) = sky {
                for (pair, &s) in above.as_chunks_mut::<2>().0.iter_mut().zip(sky) {
                    pair[0] = s & 15;
                    pair[1] = s >> 4;
                }
            }
            section.light = Light::Full(data);
        }
        section.compact();
    }
    chunk.lit = true;
    chunk.light_loaded = true;
}

type RegionCache = Mutex<RegionFiles>;
/// `RegionFileStorage.MAX_CACHE_SIZE`: the region files of a folder kept
/// open. Each holds a file descriptor: kept for every region ever visited,
/// a server whose players travel runs out of them (`Too many open files`).
const MAX_OPEN_REGIONS: usize = 256;
/// A folder's region files, and the ones known to be missing, by region,
/// with when each was last asked for.
#[derive(Default)]
struct RegionFiles {
    regions: HashMap<(i32, i32), (Option<Arc<Mutex<RegionFile>>>, u64)>,
    clock: u64,
}
impl RegionFiles {
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
    /// The open region files.
    #[cfg(test)]
    fn open(&self) -> usize {
        self.regions.values().filter(|(r, _)| r.is_some()).count()
    }
    /// Close the least recently used regions until there is room for one
    /// more, as `RegionFileStorage.getRegionFile` does. A region a reader or
    /// the writer still holds stays: closing it under them would let a
    /// second handle read a header older than its sectors.
    fn make_room(&mut self) {
        while self.regions.len() >= MAX_OPEN_REGIONS {
            let oldest = self
                .regions
                .iter()
                .filter(|(_, (r, _))| r.as_ref().is_none_or(|r| Arc::strong_count(r) == 1))
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key);
            let Some(key) = oldest else {
                return;
            };
            self.regions.remove(&key);
        }
    }
}
/// The region file at `path` through the cache, which readers and the writer
/// share, so a read never sees a header older than the sectors it points to.
/// `create` makes a missing file (for a write); otherwise a missing file is
/// `None`, until a write creates it.
fn cached_region(
    cache: &RegionCache,
    key: (i32, i32),
    path: &Path,
    create: bool,
) -> Result<Option<Arc<Mutex<RegionFile>>>> {
    let mut cache = cache.lock().map_err(|_| "region cache poisoned")?;
    cache.clock += 1;
    let now = cache.clock;
    if let Some((cached, used)) = cache.regions.get_mut(&key)
        && (cached.is_some() || !create)
    {
        *used = now;
        return Ok(cached.clone());
    }
    cache.make_room();
    if create && let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // A file this process can't write (a read-only world) is still read.
    let region = match RegionFile::open_writable(path, create) {
        Err(e) if !create && e.kind() == std::io::ErrorKind::PermissionDenied => {
            RegionFile::open(path)
        }
        other => other,
    };
    let region = match region {
        Ok(r) => Some(Arc::new(Mutex::new(r))),
        Err(_) if !create => None,
        Err(e) => return Err(e.into()),
    };
    cache.regions.insert(key, (region.clone(), now));
    Ok(region)
}
/// Write `records` into the region files of `folder` in place, as vanilla's
/// `RegionFile.write` does: each chunk takes free sectors and the header
/// then points to it; the region's other chunks are not touched. `None`
/// clears a chunk. The chunks are encoded and compressed before their region
/// is locked for the writes (`compress_records`), and each region is flushed
/// to the disk once.
fn write_region_records(
    root: &Path,
    cache: &RegionCache,
    folder: &str,
    records: &[((i32, i32), &Record)],
) -> Result<()> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let compressed = compress_records(records, timestamp);
    let mut by_region: BTreeMap<(i32, i32), Vec<(ChunkPos, Option<ChunkRecord>)>> = BTreeMap::new();
    for (&((x, z), _), record) in records.iter().zip(compressed) {
        let record = record?;
        by_region
            .entry((x >> 5, z >> 5))
            .or_default()
            .push((ChunkPos::new(x, z), record));
    }
    for ((rx, rz), list) in by_region {
        let path = region_path(root, folder, rx, rz);
        // Clearing chunks of a region that doesn't exist changes nothing.
        let create = list.iter().any(|(_, r)| r.is_some());
        let Some(region) = cached_region(cache, (rx, rz), &path, create)? else {
            continue;
        };
        let mut region = region.lock().map_err(|_| "region file poisoned")?;
        let mut changed = false;
        for (pos, record) in &list {
            match record {
                Some(record) => {
                    region.write(*pos, record)?;
                    changed = true;
                }
                None => changed |= region.clear(*pos, timestamp)?,
            }
        }
        if changed {
            region.sync()?;
        }
    }
    Ok(())
}

/// Fewest chunks each helper of `compress_records` takes: below this a thread
/// costs more to start than it saves.
const CHUNKS_PER_HELPER: usize = 4;
/// Each record encoded and compressed (`None` for a deletion), in the order
/// given. Vanilla's IOWorker does this one chunk after another; here a batch
/// is shared out to scoped threads, which take the next chunk as they finish
/// one. Each chunk's bytes depend only on the chunk, and the writes stay in
/// order, so the files are the same as from one thread. At most half the
/// hardware threads help, leaving the rest to the game and its workers.
fn compress_records(
    records: &[((i32, i32), &Record)],
    timestamp: u32,
) -> Vec<std::io::Result<Option<ChunkRecord>>> {
    let compress = |record: &Record| {
        record
            .tag()
            .map(|tag| ChunkRecord::from_nbt(&tag, Compression::Zlib, timestamp, Limits::saved()))
            .transpose()
    };
    let helpers = std::thread::available_parallelism()
        .map_or(1, |n| n.get().div_ceil(2))
        .min(records.len() / CHUNKS_PER_HELPER);
    if helpers <= 1 {
        return records.iter().map(|(_, r)| compress(r)).collect();
    }
    share_out(records.len(), helpers, |i| compress(records[i].1), || true)
}

/// `job(i)` for every `i` below `count`, in order, shared between this thread
/// and up to `helpers - 1` scoped threads, each taking the next index as it
/// finishes one. A helper the system refuses to start (`start` false, or a
/// container at its limit of threads: EAGAIN) leaves its share to the
/// others: this thread does all of it at worst, and never panics for it, as
/// `thread::scope`'s own `spawn` would, which stopped the region writer.
fn share_out<R: Send>(
    count: usize,
    helpers: usize,
    job: impl Fn(usize) -> R + Sync,
    mut start: impl FnMut() -> bool,
) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let work = || {
        let mut out = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= count {
                return out;
            }
            out.push((i, job(i)));
        }
    };
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let threads: Vec<_> = (1..helpers)
            .filter(|_| start())
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, work).ok())
            .collect();
        let mut done = work();
        for t in threads {
            done.extend(t.join().unwrap_or_else(|e| std::panic::resume_unwind(e)));
        }
        done
    });
    done.sort_unstable_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

fn region_path(root: &Path, folder: &str, rx: i32, rz: i32) -> PathBuf {
    root.join(format!("{folder}/r.{rx}.{rz}.mca"))
}

/// A record waiting for the writer thread.
#[derive(Clone)]
enum Record {
    Tag(Arc<NamedTag>),
    /// A chunk to encode on the writer thread.
    Chunk(Arc<Chunk>, Arc<Encoding>),
    /// A chunk's points of interest, made on the writer thread against the
    /// saved ones (`resolve_poi`).
    Poi(Arc<Chunk>, Arc<SaveTables>),
    Delete,
}
/// What encoding a chunk needs, captured when it is queued.
struct Encoding {
    registry: &'static BlockRegistry,
    biomes: Arc<Biomes>,
    tables: Option<Arc<SaveTables>>,
    shape: LevelShape,
}
impl Record {
    fn tag(&self) -> Option<Arc<NamedTag>> {
        match self {
            Record::Tag(t) => Some(t.clone()),
            Record::Chunk(c, e) => Some(Arc::new(encode_with(
                c,
                e.registry,
                &e.biomes,
                e.tables.as_deref(),
                e.shape,
            ))),
            Record::Poi(c, t) => encode_poi(c, t).map(Arc::new),
            Record::Delete => None,
        }
    }
}
/// A queued POI record as it is written: claims kept from the saved one, and
/// a chunk that lost its last POI gets an empty one, as vanilla trusts a
/// saved section. None when the chunk never had any.
fn resolve_poi(
    root: &Path,
    cache: &RegionCache,
    (x, z): (i32, i32),
    record: &Record,
) -> Option<Record> {
    let Record::Poi(chunk, tables) = record else {
        return Some(record.clone());
    };
    let old = read_region_record(root, cache, "poi", x, z)
        .and_then(|old| {
            old.map(|mut old| {
                upgrade_record(Folder::Poi, &mut old, || None)?;
                Ok(old)
            })
            .transpose()
        })
        .unwrap_or_else(|e| {
            log::warn!("Horizon: could not read the poi record of chunk {x} {z}: {e}");
            None
        });
    let tag = match (encode_poi(chunk, tables), old) {
        (Some(mut tag), Some(old)) => {
            keep_claims(&mut tag, &old);
            tag
        }
        (Some(tag), None) => tag,
        (None, Some(_)) => poi_root(Compound::new()),
        (None, None) => return None,
    };
    Some(Record::Tag(Arc::new(tag)))
}
fn read_region_record(
    root: &Path,
    cache: &RegionCache,
    folder: &str,
    x: i32,
    z: i32,
) -> Result<Option<NamedTag>> {
    let key = (x >> 5, z >> 5);
    let path = region_path(root, folder, key.0, key.1);
    let Some(region) = cached_region(cache, key, &path, false)? else {
        return Ok(None);
    };
    let record = region
        .lock()
        .map_err(|_| "region file poisoned")?
        .read(ChunkPos::new(x, z))?;
    Ok(match record {
        Some(r) => Some(r.nbt(Limits::saved())?),
        None => None,
    })
}
/// The `__context` the chunk fixes read: the dimension and its generator type
/// (`ChunkMap.getChunkDataFixContextTag`).
struct DatafixContext {
    dimension: String,
    generator: Option<String>,
}
impl DatafixContext {
    /// From a dimension folder made by [`dimension_dir`] and the world's
    /// `world_gen_settings.dat`; None for a folder outside a world's `dimensions/`.
    fn of(root: &Path) -> Option<Self> {
        let mut parts = Vec::new();
        let mut at = root;
        while at.file_name()? != "dimensions" {
            parts.push(at.file_name()?.to_str()?);
            at = at.parent()?;
        }
        let world = at.parent()?;
        let namespace = parts.pop()?;
        if parts.is_empty() {
            return None;
        }
        parts.reverse();
        let dimension = format!("{namespace}:{}", parts.join("/"));
        let generator = horizon_world::datafix::files::generators(world)
            .ok()
            .and_then(|g| g.into_iter().find(|(d, _)| *d == dimension))
            .map(|(_, g)| g);
        Some(Self {
            dimension,
            generator,
        })
    }
}
/// `SimpleRegionStorage.upgradeChunkTag`, as `ChunkMap.upgradeChunkTag` (chunks),
/// `EntityStorage` and `SectionStorage.PackedChunk.parse` (POI) call it on every
/// record they read: a world 26.3 has opened keeps the chunks nobody loaded since
/// an older version in that version's format, and `upgrade_older_world` only runs
/// for an older level.dat. A record without a DataVersion counts as -1 (POI: 1945).
/// `context` is needed only for an older chunk; without one it stays as saved.
fn upgrade_record<'a>(
    folder: Folder,
    record: &mut NamedTag,
    context: impl FnOnce() -> Option<&'a DatafixContext>,
) -> Result<()> {
    use horizon_world::datafix::{self, schema::Ty};
    let default = if folder == Folder::Poi { 1945 } else { -1 };
    if number(record.tag.get("DataVersion")).unwrap_or(default) >= i64::from(DATA_VERSION) {
        return Ok(());
    }
    let context = match folder {
        Folder::Region => match context() {
            Some(c) => Some(c),
            None => return Ok(()),
        },
        _ => None,
    };
    let tag = &mut record.tag;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match (folder, context) {
        (Folder::Region, Some(c)) => {
            datafix::update_chunk(tag, &c.dimension, c.generator.as_deref());
        }
        (Folder::Entities, _) => datafix::update_region_tag(Ty::EntityChunk, tag),
        (Folder::Poi, _) => datafix::update_region_tag(Ty::PoiChunk, tag),
        (Folder::Region, None) => {}
    }))
    .map_err(|_| "upgrading an older record failed".into())
}
/// Chunk and entity records waiting for the writer thread, by (entities?,
/// x, z): reads see them before the region files do.
type PendingWrites = Mutex<HashMap<(Folder, i32, i32), (u64, Record)>>;
type WriteBatch = (Folder, Vec<((i32, i32), Record, u64)>);
/// The region folders of a dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Folder {
    Region,
    Entities,
    Poi,
}
impl Folder {
    fn of(name: &str) -> Self {
        match name {
            "entities" => Self::Entities,
            "poi" => Self::Poi,
            _ => Self::Region,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::Entities => "entities",
            Self::Poi => "poi",
        }
    }
}

/// The background thread that encodes chunks and writes them into the region files, in
/// submission order.
struct Writer {
    send: mpsc::Sender<WriteBatch>,
    thread: std::thread::JoinHandle<()>,
}

/// Writes still queued or running, with a condition to wait for none.
#[derive(Default)]
struct Outstanding {
    count: Mutex<usize>,
    error: Mutex<Option<String>>,
    done: Condvar,
}
/// One dimension folder (`dimensions/minecraft/<name>`). Region files are
/// read lazily by worker threads.
pub struct SaveDir {
    pub root: PathBuf,
    regions: Arc<RegionCache>,
    entity_regions: Arc<RegionCache>,
    poi_regions: Arc<RegionCache>,
    pending: Arc<PendingWrites>,
    sequence: AtomicU64,
    writer: Mutex<Option<Writer>>,
    outstanding: Arc<Outstanding>,
    /// Set by `Persistent` before the first load.
    pub tables: OnceLock<Arc<SaveTables>>,
    /// The dimension's skylight and height, from its folder name.
    pub shape: LevelShape,
    /// Which chunks each region file holds, for far terrain (`lod_saved`).
    lod_regions: Mutex<crate::fast_hash::FxHashMap<(i32, i32), LodRegion>>,
    /// Saved chunks decoded for far terrain, shared by its sampling threads
    /// (`lod_surfaces_shared`), oldest first.
    lod_decoded: Mutex<LodDecoded>,
    /// What older chunks are upgraded with, found on the first one read.
    datafix_context: OnceLock<Option<DatafixContext>>,
}
fn is_forced_ticket(ticket: &Tag) -> bool {
    matches!(ticket.get("type"), Some(Tag::String(t)) if *t == JavaString::from("minecraft:forced"))
}
#[derive(Default)]
struct LodDecoded {
    chunks: crate::fast_hash::FxHashMap<(i32, i32), (std::time::Instant, LodSurfaces)>,
    order: std::collections::VecDeque<(i32, i32)>,
}
/// Saved chunks kept decoded for far terrain (each tens of kilobytes), and
/// for how long: edits saved later show after it.
const LOD_DECODED: usize = 1024;
const LOD_DECODED_TTL: std::time::Duration = std::time::Duration::from_secs(30);
/// A region file's saved chunks (bit `x + z * 32`), `None` without the file,
/// as of `checked`.
struct LodRegion {
    checked: std::time::Instant,
    chunks: Option<[u64; 16]>,
}
/// How long far terrain trusts a region file's chunk list: chunks saved after
/// it was read show in the far view this long after.
const LOD_REGION_TTL: std::time::Duration = std::time::Duration::from_secs(10);
impl SaveDir {
    /// This dimension's server-owned mod attachments: `data/horizon/attachments.dat`, Horizon's
    /// own namespace. A file that doesn't read is moved aside to `attachments.dat.corrupt`, with
    /// a warning, and the level opens without them (as vanilla's `SavedDataStorage` starts a
    /// saved data over when its file is broken), so the next save can't overwrite it.
    pub fn load_attachments(&self) -> horizon_world::attachments::Attachments {
        let path = self.attachments_path();
        let data = match try_read_dat(&path) {
            Err(DatIssue::Missing) => return Default::default(),
            Ok(root) => match root.get("data") {
                Some(Tag::Compound(data)) => Ok(data.clone()),
                _ => Err("no data compound".to_owned()),
            },
            Err(DatIssue::Unreadable(why)) => Err(why),
        };
        data.map(horizon_world::attachments::Attachments::from_nbt)
            .unwrap_or_else(|why| {
                let aside = path.with_extension("dat.corrupt");
                log::warn!(
                    "{}: {why}; mod attachments start over, the file is kept as {}",
                    path.display(),
                    aside.display()
                );
                let _ = fs::rename(&path, &aside);
                Default::default()
            })
    }

    /// Save this dimension's server-owned mod attachments; nothing while it has none and never had.
    pub fn save_attachments(
        &self,
        attachments: &horizon_world::attachments::Attachments,
    ) -> Result<()> {
        let path = self.attachments_path();
        let data = attachments.to_nbt();
        if data.is_empty() && !path.exists() {
            return Ok(());
        }
        write_dat(
            &path,
            Tag::compound([
                ("data", Tag::Compound(data)),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
    }

    fn attachments_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("horizon")
            .join("attachments.dat")
    }

    /// The saves of a level of `shape` at `root` (its dimension folder).
    pub fn new(root: PathBuf, shape: LevelShape) -> Self {
        Self {
            root,
            regions: Default::default(),
            entity_regions: Default::default(),
            poi_regions: Default::default(),
            pending: Default::default(),
            sequence: AtomicU64::new(0),
            writer: Mutex::new(None),
            outstanding: Default::default(),
            tables: OnceLock::new(),
            shape,
            lod_regions: Default::default(),
            lod_decoded: Default::default(),
            datafix_context: OnceLock::new(),
        }
    }
    /// 26.3's TicketStorage: persistent tickets live in each dimension's
    /// data/minecraft/chunk_tickets.dat, with ChunkPos.CODEC's [x, z] int array.
    pub fn load_forced_chunks(&self) -> HashSet<(i32, i32)> {
        let Some(data) = read_saved_data(&self.root, "chunk_tickets") else {
            return HashSet::new();
        };
        let Some(Tag::List(tickets)) = data.get("tickets") else {
            return HashSet::new();
        };
        tickets
            .iter()
            .filter(|t| is_forced_ticket(t))
            .filter_map(|t| match t.get("chunk_pos")? {
                Tag::IntArray(pos) if pos.len() == 2 => Some((pos[0], pos[1])),
                _ => None,
            })
            .collect()
    }
    pub fn save_forced_chunks(&self, forced: &HashSet<(i32, i32)>) -> Result<()> {
        if forced.is_empty() && !saved_data_path(&self.root, "chunk_tickets").exists() {
            return Ok(());
        }
        let mut data = compound_or_empty(read_saved_data(&self.root, "chunk_tickets"));
        // Preserve persistent tickets Horizon does not simulate.
        let mut tickets = match data.get("tickets") {
            Some(Tag::List(tickets)) => tickets
                .iter()
                .filter(|t| !is_forced_ticket(t))
                .cloned()
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let mut chunks: Vec<_> = forced.iter().copied().collect();
        chunks.sort_unstable();
        tickets.extend(chunks.into_iter().map(|(x, z)| {
            Tag::compound([
                ("type", Tag::String("minecraft:forced".into())),
                ("level", Tag::Int(31)), // ChunkMap.FORCED_TICKET_LEVEL: ENTITY_TICKING.
                ("chunk_pos", Tag::IntArray(vec![x, z])),
            ])
        }));
        put(&mut data, "tickets", Tag::List(tickets));
        write_saved_data(&self.root, "chunk_tickets", data)
    }
    /// Whether chunk `x z` is in its region file, from a chunk list kept
    /// `LOD_REGION_TTL`. Far terrain asks for every column of the view: it
    /// opened the region file each time, also where there is none, which
    /// held the far view back for minutes.
    fn lod_saved(&self, x: i32, z: i32) -> bool {
        let key = (x >> 5, z >> 5);
        let mut regions = self.lod_regions.lock().unwrap();
        let fresh = regions
            .get(&key)
            .is_some_and(|region| region.checked.elapsed() < LOD_REGION_TTL);
        if !fresh {
            let chunks = RegionFile::open(self.region_path("region", key.0, key.1))
                .ok()
                .map(|region| {
                    let mut bits = [0u64; 16];
                    for (pos, _) in region.chunks() {
                        let index = ((pos.x & 31) + (pos.z & 31) * 32) as usize;
                        bits[index / 64] |= 1 << (index % 64);
                    }
                    bits
                });
            regions.insert(
                key,
                LodRegion {
                    checked: std::time::Instant::now(),
                    chunks,
                },
            );
        }
        let index = ((x & 31) + (z & 31) * 32) as usize;
        regions[&key]
            .chunks
            .is_some_and(|bits| bits[index / 64] & (1 << (index % 64)) != 0)
    }
    fn region_path(&self, folder: &str, rx: i32, rz: i32) -> PathBuf {
        region_path(&self.root, folder, rx, rz)
    }
    /// The chunks of a region file that an older version generated, as
    /// `IOWorker.createOldDataForRegion` finds them (bit `x + z * 32`): saved
    /// before DataVersion 4997, or with `blending_data`.
    pub fn old_chunk_bits(&self, rx: i32, rz: i32) -> [u64; 16] {
        let mut bits = [0u64; 16];
        let Ok(region) = RegionFile::open(self.region_path("region", rx, rz)) else {
            return bits;
        };
        let present: Vec<ChunkPos> = region.chunks().map(|(pos, _)| pos).collect();
        drop(region);
        for pos in present {
            // `IOWorker` reads the record as saved, before any upgrade.
            let Some(root) = self.read_record_raw("region", pos.x, pos.z) else {
                continue;
            };
            let old = match root.tag.get("DataVersion") {
                Some(Tag::Int(version)) => *version < 4997,
                _ => true,
            } || root.tag.get("blending_data").is_some();
            if old {
                let index = ((pos.x & 31) + (pos.z & 31) * 32) as usize;
                bits[index / 64] |= 1 << (index % 64);
            }
        }
        bits
    }
    /// A saved chunk brought to the current version, for the readers of its
    /// fields that `load` does not decode (`blending_data`).
    pub fn raw_chunk(&self, x: i32, z: i32) -> Option<NamedTag> {
        self.read_record("region", x, z)
    }
    /// The references a saved chunk keeps for its structure start `name`
    /// (`StructureStart.getReferences`, saved as `references`): 0 for a chunk
    /// not saved yet, a start it does not have or one without the field.
    pub fn start_references(&self, x: i32, z: i32, name: &str) -> i32 {
        let Some(root) = self.read_record("region", x, z) else {
            return 0;
        };
        match root
            .tag
            .get("structures")
            .and_then(|s| s.get("starts"))
            .and_then(|s| s.get(name))
            .and_then(|s| s.get("references"))
        {
            Some(Tag::Int(n)) => *n,
            _ => 0,
        }
    }
    /// A record as saved: the pending write first, then the region file.
    fn read_record_raw(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        let pending = self
            .pending
            .lock()
            .ok()?
            .get(&(Folder::of(folder), x, z))
            .map(|(_, r)| r.clone());
        if let Some(record) = pending {
            return record.tag().map(|t| (*t).clone());
        }
        let cache = match Folder::of(folder) {
            Folder::Region => &self.regions,
            Folder::Entities => &self.entity_regions,
            Folder::Poi => &self.poi_regions,
        };
        match read_region_record(&self.root, cache, folder, x, z) {
            Ok(record) => record,
            Err(e) => {
                crate::chunk_io::report_load_failure(&self.root, x, z, &e);
                None
            }
        }
    }
    /// A record brought to the current version (`upgrade_record`).
    fn read_record(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        let mut record = self.read_record_raw(folder, x, z)?;
        match self.upgrade_record(Folder::of(folder), &mut record) {
            Ok(()) => Some(record),
            Err(e) => {
                crate::chunk_io::report_load_failure(&self.root, x, z, &e);
                None
            }
        }
    }
    fn upgrade_record(&self, folder: Folder, record: &mut NamedTag) -> Result<()> {
        upgrade_record(folder, record, || {
            self.datafix_context
                .get_or_init(|| DatafixContext::of(&self.root))
                .as_ref()
        })
    }
    pub fn load(&self, x: i32, z: i32, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
        let root = self.read_record("region", x, z)?;
        let tables = self.tables.get().map(|t| &**t);
        let mut chunk = decode_with(&root, registry, biomes, tables, self.shape.column())?;
        load_light(&root.tag, &mut chunk, self.shape.sky_light);
        Some(chunk)
    }
    /// A saved full chunk read for a player's far ring (`lan::far`), as `load` but past the gameplay
    /// region cache, and only where the region file lists it (no file opened where there is none).
    pub fn load_far(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Option<Chunk> {
        let pending = self
            .pending
            .lock()
            .ok()?
            .get(&(Folder::Region, x, z))
            .map(|(_, record)| record.clone());
        let root = if let Some(record) = pending {
            record.tag().map(|tag| (*tag).clone())?
        } else {
            if !self.lod_saved(x, z) {
                return None;
            }
            let mut region = RegionFile::open(self.region_path("region", x >> 5, z >> 5)).ok()?;
            let mut root = region
                .read(ChunkPos::new(x, z))
                .ok()??
                .nbt(Limits::default())
                .ok()?;
            self.upgrade_record(Folder::Region, &mut root).ok()?;
            root
        };
        let tables = self.tables.get().map(|t| &**t);
        let mut chunk = decode_with(&root, registry, biomes, tables, self.shape.column())?;
        load_light(&root.tag, &mut chunk, self.shape.sky_light);
        // Without its saved light (`isLightOn`) it would need the light engine: not one for the far ring.
        chunk.light_loaded.then_some(chunk)
    }
    /// Chunks queued for the writer thread and not yet in their region files.
    pub fn pending_writes(&self) -> usize {
        self.pending.lock().map_or(0, |p| p.len())
    }
    /// A proto chunk saved before it became final (`decode_proto`).
    pub fn load_proto(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Option<SavedProto> {
        let root = self.read_record("region", x, z)?;
        let tables = self.tables.get().map(|t| &**t);
        decode_proto(&root, registry, biomes, tables, self.shape.column())
    }
    /// Queue proto chunks for the writer thread; loads see them meanwhile.
    pub fn save_protos(
        &self,
        protos: &[SavedProto],
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Result<()> {
        let tables = self.tables.get().map(|t| &**t);
        let records = protos
            .iter()
            .map(|p| {
                let tag = encode_proto(p, registry, biomes, tables, self.shape);
                ((p.column.x, p.column.z), Some(tag))
            })
            .collect();
        self.write_records("region", records, false)
    }
    /// Entities saved for a chunk (`entities/`; older chunks may still keep
    /// them in the chunk's own `entities` list).
    pub fn load_entities(&self, x: i32, z: i32) -> Vec<Tag> {
        let mut out = Vec::new();
        if let Some(root) = self.read_record("entities", x, z)
            && let Some(Tag::List(list)) = root.tag.get("Entities")
        {
            out.extend(list.iter().cloned());
        }
        out
    }
    /// Queue `records` for the writer thread; with `wait`, return once
    /// they and every earlier write are on disk.
    fn write_records(
        &self,
        folder: &str,
        records: Vec<((i32, i32), Option<NamedTag>)>,
        wait: bool,
    ) -> Result<()> {
        let records = records
            .into_iter()
            .map(|(k, t)| (k, t.map_or(Record::Delete, |t| Record::Tag(Arc::new(t)))))
            .collect();
        self.queue(Folder::of(folder), records, wait)
    }
    fn queue(&self, folder: Folder, records: Vec<((i32, i32), Record)>, wait: bool) -> Result<()> {
        if !records.is_empty() {
            let mut batch = Vec::with_capacity(records.len());
            {
                let mut pending = self.pending.lock().map_err(|_| "pending writes poisoned")?;
                for (key, record) in records {
                    let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
                    pending.insert((folder, key.0, key.1), (seq, record.clone()));
                    batch.push((key, record, seq));
                }
            }
            *self
                .outstanding
                .count
                .lock()
                .map_err(|_| "writer poisoned")? += 1;
            let mut writer = self.writer.lock().map_err(|_| "writer poisoned")?;
            let writer = writer.get_or_insert_with(|| self.start_writer());
            if writer.send.send((folder, batch)).is_err() {
                *self
                    .outstanding
                    .error
                    .lock()
                    .map_err(|_| "writer poisoned")? = Some("the region writer stopped".into());
                *self
                    .outstanding
                    .count
                    .lock()
                    .map_err(|_| "writer poisoned")? -= 1;
                self.outstanding.done.notify_all();
                return Err("the region writer stopped".into());
            }
        }
        if wait {
            self.flush()?;
        }
        Ok(())
    }
    fn start_writer(&self) -> Writer {
        let (send, receive) = mpsc::channel::<WriteBatch>();
        let root = self.root.clone();
        let (regions, entity_regions, poi_regions) = (
            self.regions.clone(),
            self.entity_regions.clone(),
            self.poi_regions.clone(),
        );
        let pending = self.pending.clone();
        let outstanding = self.outstanding.clone();
        let thread = std::thread::Builder::new()
            .name("horizon-region-writer".into())
            .spawn(move || {
                for (kind, batch) in receive {
                    let cache = match kind {
                        Folder::Region => &regions,
                        Folder::Entities => &entity_regions,
                        Folder::Poi => &poi_regions,
                    };
                    let folder = kind.name();
                    // IOWorker.pendingWrites: a chunk queued again since is
                    // written once, by its newest batch.
                    let newest = |k: &(i32, i32), seq: u64| {
                        pending.lock().map_or(true, |p| {
                            p.get(&(kind, k.0, k.1)).is_none_or(|p| p.0 <= seq)
                        })
                    };
                    let resolved: Vec<_> = batch
                        .iter()
                        .filter(|(k, _, seq)| newest(k, *seq))
                        .filter_map(|(k, r, _)| Some((*k, resolve_poi(&root, cache, *k, r)?)))
                        .collect();
                    let records: Vec<_> = resolved.iter().map(|(k, r)| (*k, r)).collect();
                    let result = write_region_records(&root, cache, folder, &records);
                    if let Err(e) = &result {
                        let message = format!("failed to write {folder} files: {e}");
                        log::warn!("Horizon: {message}");
                        *outstanding.error.lock().unwrap() = Some(message);
                        // `reportChunkSaveFailure` for each chunk of the batch.
                        for (k, _, _) in &batch {
                            crate::chunk_io::report_save_failure(&root, k.0, k.1, e);
                        }
                    }
                    // Keep failed snapshots readable; never report them as durable.
                    if result.is_ok()
                        && let Ok(mut pending) = pending.lock()
                    {
                        for (k, _, seq) in &batch {
                            if pending.get(&(kind, k.0, k.1)).is_some_and(|p| p.0 == *seq) {
                                pending.remove(&(kind, k.0, k.1));
                            }
                        }
                    }
                    if let Ok(mut count) = outstanding.count.lock() {
                        *count -= 1;
                        outstanding.done.notify_all();
                    }
                }
            })
            .expect("spawn region writer");
        Writer { send, thread }
    }
    /// Wait until every queued write is on disk.
    pub fn flush(&self) -> Result<()> {
        let mut count = self
            .outstanding
            .count
            .lock()
            .map_err(|_| "writer poisoned")?;
        while *count > 0 {
            count = self
                .outstanding
                .done
                .wait(count)
                .map_err(|_| "writer poisoned")?;
        }
        self.write_result()
    }
    /// Poll durability without holding up the event loop. Errors remain visible
    /// to every waiter, including a later synchronous shutdown.
    pub fn writes_finished(&self) -> Result<bool> {
        let count = self
            .outstanding
            .count
            .lock()
            .map_err(|_| "writer poisoned")?;
        self.write_result()?;
        Ok(*count == 0)
    }
    fn write_result(&self) -> Result<()> {
        match &*self
            .outstanding
            .error
            .lock()
            .map_err(|_| "writer poisoned")?
        {
            Some(error) => Err(error.clone().into()),
            None => Ok(()),
        }
    }
    /// Write `chunks` into their region files and wait until they are on disk.
    pub fn save(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Result<usize> {
        let tables = self.tables.get().map(|t| &**t);
        let records = chunks
            .iter()
            .map(|c| {
                (
                    (c.x, c.z),
                    Some(encode_with(c, registry, biomes, tables, self.shape)),
                )
            })
            .collect();
        self.write_records("region", records, true)?;
        self.save_poi(chunks, true)?;
        Ok(chunks.len())
    }
    fn save_poi(&self, chunks: &[Arc<Chunk>], wait: bool) -> Result<()> {
        let Some(tables) = self.tables.get() else {
            return Ok(());
        };
        let records = chunks
            .iter()
            .map(|c| ((c.x, c.z), Record::Poi(c.clone(), tables.clone())))
            .collect();
        self.queue(Folder::Poi, records, wait)
    }
    /// `save` on the writer thread: returns before the files are written;
    /// loads see the chunks meanwhile.
    pub fn save_later(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<usize> {
        let encoding = Arc::new(Encoding {
            registry,
            biomes: biomes.clone(),
            tables: self.tables.get().cloned(),
            shape: self.shape,
        });
        let records = chunks
            .iter()
            .map(|c| ((c.x, c.z), Record::Chunk(c.clone(), encoding.clone())))
            .collect();
        self.queue(Folder::Region, records, false)?;
        self.save_poi(chunks, false)?;
        Ok(chunks.len())
    }
    /// Write the entity chunks (`EntityStorage`): an empty list deletes it.
    pub fn save_entities(&self, chunks: Vec<((i32, i32), Vec<Tag>)>) -> Result<()> {
        self.write_entities(chunks, true)
    }
    /// `save_entities` on the writer thread.
    pub fn save_entities_later(&self, chunks: Vec<((i32, i32), Vec<Tag>)>) -> Result<()> {
        self.write_entities(chunks, false)
    }
    fn write_entities(&self, chunks: Vec<((i32, i32), Vec<Tag>)>, wait: bool) -> Result<()> {
        let records = chunks
            .into_iter()
            .map(|((x, z), entities)| {
                let tag = (!entities.is_empty()).then(|| NamedTag {
                    name: "".into(),
                    tag: Tag::compound([
                        ("DataVersion", Tag::Int(DATA_VERSION)),
                        ("Position", Tag::IntArray(vec![x, z])),
                        ("Entities", Tag::List(entities)),
                    ]),
                });
                ((x, z), tag)
            })
            .collect();
        self.write_records("entities", records, wait)
    }
}

impl Drop for SaveDir {
    /// Finish queued writes before the world is let go.
    fn drop(&mut self) {
        if let Some(writer) = self.writer.get_mut().ok().and_then(Option::take) {
            drop(writer.send);
            let _ = writer.thread.join();
        }
    }
}

pub fn read_dat(path: &Path) -> Option<Tag> {
    try_read_dat(path).ok()
}
/// `read_dat`, with why the file did not read.
fn try_read_dat(path: &Path) -> std::result::Result<Tag, DatIssue> {
    let bytes = fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DatIssue::Missing,
        _ => DatIssue::Unreadable(e.to_string()),
    })?;
    let raw = match Compression::Gzip.decode(&bytes, Limits::saved().max_bytes) {
        Ok(raw) => raw,
        Err(_) => bytes,
    };
    nbt::read_named(&raw, Limits::saved())
        .map(|(named, _)| named.tag)
        .map_err(|e| DatIssue::Unreadable(e.to_string()))
}
/// What `LevelStorageAccess.collectIssues` finds wrong with a level.dat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatIssue {
    /// No such file (`FileNotFoundException`).
    Missing,
    /// It exists but does not read as a level: why.
    Unreadable(String),
}
/// `LevelStorageAccess.collectIssues`: why level.dat (level.dat_old with
/// `old`) can't be read as the world's level data, `None` when it can.
pub fn level_dat_issue(dir: &Path, old: bool) -> Option<DatIssue> {
    let file = if old { "level.dat_old" } else { "level.dat" };
    match try_read_dat(&dir.join(file)) {
        Ok(root) if matches!(root.get("Data"), Some(Tag::Compound(_))) => None,
        Ok(_) => Some(DatIssue::Unreadable("Invalid root tag".into())),
        Err(issue) => Some(issue),
    }
}
/// `WorldOpenFlows.openWorldLoadLevelData`: what keeps `world`'s level data from reading, which
/// opens RecoverWorldDataScreen instead of the world. A folder with neither level.dat nor
/// level.dat_old is a world not made yet, not a broken one.
pub fn level_data_needs_recovery(world: &Path) -> Option<DatIssue> {
    match level_dat_issue(world, false)? {
        DatIssue::Missing if !world.join("level.dat_old").exists() => None,
        issue => Some(issue),
    }
}
/// `LevelStorageAccess.restoreLevelDataFromOld` (`Util.safeReplaceOrMoveFile`):
/// level.dat moves aside to `level.dat_corrupted_<time>` and level.dat_old
/// takes its place. Whether it worked. Vanilla passes `noRollback` and reports
/// success even when level.dat_old could not be moved in; here the broken
/// level.dat goes back and the failure is reported.
pub fn restore_level_data_from_old(dir: &Path) -> bool {
    let (current, latest) = (dir.join("level.dat"), dir.join("level.dat_old"));
    let backup = dir.join(format!("level.dat_corrupted_{}", backup_stamp()));
    if current.exists() && fs::rename(&current, &backup).is_err() {
        return false;
    }
    if fs::rename(&latest, &current).is_err() {
        if backup.exists() {
            let _ = fs::rename(&backup, &current);
        }
        return false;
    }
    true
}
/// `FileNameDateFormatter`: the time in the names of backed-up unreadable files.
fn backup_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}
/// `LevelStorageAccess.getUnfixedDataTagWithFallback`, which the dedicated server's `Main` calls:
/// a level.dat that does not read falls back to level.dat_old, which is restored in its place
/// (`restoreLevelDataFromOld`). Whether it did; `false` leaves the world as it was, for the
/// caller to refuse (both files broken, or nothing to fall back to).
pub fn fall_back_to_level_dat_old(world: &Path) -> bool {
    level_dat_issue(world, false).is_some()
        && world.join("level.dat_old").exists()
        && level_dat_issue(world, true).is_none()
        && restore_level_data_from_old(world)
}
/// `LevelStorageSource.readLevelDataTagFixed`: level.dat, else level.dat_old.
fn read_level_dat(dir: &Path) -> Option<Tag> {
    read_dat(&dir.join("level.dat")).or_else(|| read_dat(&dir.join("level.dat_old")))
}
/// `restoreLevelDataFromOld`: an unreadable level.dat is kept as
/// `level.dat_corrupted_<time>` and level.dat_old takes its place, so the next
/// write does not bury either file.
fn restore_level_dat(dir: &Path) {
    let path = dir.join("level.dat");
    if read_dat(&path).is_some() {
        return;
    }
    if path.exists() {
        let _ = fs::rename(
            &path,
            dir.join(format!("level.dat_corrupted_{}", backup_stamp())),
        );
    }
    let old = dir.join("level.dat_old");
    if read_dat(&old).is_some() {
        let _ = fs::rename(old, path);
    }
}
/// `PlayerDataStorage.backup`: a player file that fails to load is copied to
/// `<uuid>_corrupted_<time>.dat` before it is written over.
fn back_up_unreadable_player(path: &Path) {
    if path.is_file()
        && read_dat(path).is_none()
        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
    {
        let _ = fs::copy(
            path,
            path.with_file_name(format!("{stem}_corrupted_{}.dat", backup_stamp())),
        );
    }
}
/// Write a gzip NBT file the way vanilla does (`<name>_new`, keep `_old`).
pub fn write_dat(path: &Path, tag: Tag) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let bytes = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag,
        },
        Limits::saved(),
    )
    .map_err(|e| e.to_string())?;
    let compressed = Compression::Gzip.encode(&bytes)?;
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("data.dat");
    let temporary = path.with_file_name(format!("{file}_new"));
    fs::write(&temporary, compressed)?;
    if path.exists() {
        let old = match file.strip_suffix(".dat") {
            Some(stem) => path.with_file_name(format!("{stem}.dat_old")),
            None => path.with_file_name(format!("{file}_old")),
        };
        // Util.safeReplaceFile: no backup, no replacement.
        fs::rename(path, old).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}
/// `SavedDataStorage`: `{data: ..., DataVersion}` under `data/minecraft/`.
fn saved_data_path(world: &Path, name: &str) -> PathBuf {
    world
        .join("data")
        .join("minecraft")
        .join(format!("{name}.dat"))
}
fn read_saved_data(world: &Path, name: &str) -> Option<Tag> {
    read_dat(&saved_data_path(world, name))?
        .get("data")
        .cloned()
}
/// The world's generator settings: world_gen_settings.dat (26.3) or
/// level.dat's Data.WorldGenSettings.
pub fn world_gen_settings(world: &Path) -> Option<Tag> {
    // A world still to be file-fixed has them in level.dat in an older shape: they are read as
    // the fixes would leave them, so the questions on opening it are the same ones.
    if let Ok(Some(settings)) = horizon_world::datafix::files::current_world_gen_settings(world) {
        return Some(settings);
    }
    read_saved_data(world, "world_gen_settings").or_else(|| {
        read_dat(&world.join("level.dat"))?
            .get("Data")?
            .get("WorldGenSettings")
            .cloned()
    })
}
fn write_saved_data(world: &Path, name: &str, data: Tag) -> Result<()> {
    write_dat(
        &saved_data_path(world, name),
        Tag::compound([("data", data), ("DataVersion", Tag::Int(DATA_VERSION))]),
    )
}
pub use horizon_client_core::uuid::{uuid_from_text, uuid_text};
/// A random (version 4) UUID as vanilla's four-int form.
///
/// Every call gets its own: the generator is seeded once from the clock and
/// the process, then each call takes the next step of one shared counter. A
/// seed from the clock alone repeats within its resolution (a microsecond on
/// macOS): a pack of mobs generated together all had the same UUID, and a
/// vanilla client keeps only one entity per UUID (the rest never moved).
pub fn random_uuid() -> [i32; 4] {
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        AtomicU64::new(t ^ (std::process::id() as u64).rotate_left(32))
    });
    // SplitMix64: distinct steps give distinct outputs.
    let split = || {
        let mut z = next
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            .wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let (a, b) = (split(), split());
    let a = (a & !0xF000) | 0x4000;
    let b = (b & !(0xC000_0000_0000_0000)) | 0x8000_0000_0000_0000;
    [(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]
}
pub(crate) fn uuid_tag(u: [i32; 4]) -> Tag {
    Tag::IntArray(u.to_vec())
}
pub(crate) fn uuid_from_tag(tag: Option<&Tag>) -> Option<[i32; 4]> {
    match tag? {
        Tag::IntArray(v) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
        Tag::String(s) => uuid_from_text(&s.to_string_lossy()),
        _ => None,
    }
}
pub(crate) fn doubles(values: &[f64]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Double(v.to_bits())).collect())
}
pub(crate) fn floats(values: &[f32]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Float(v.to_bits())).collect())
}
pub(crate) fn read_doubles<const N: usize>(tag: Option<&Tag>) -> Option<[f64; N]> {
    match tag? {
        Tag::List(l) if l.len() == N => {
            let mut out = [0.0; N];
            for (o, v) in out.iter_mut().zip(l) {
                *o = float(Some(v))?;
            }
            Some(out)
        }
        _ => None,
    }
}

/// Default game rules of a new 26.3 world (`data/minecraft/game_rules.dat`).
const GAME_RULES: [(&str, i32); 58] = [
    ("spawn_wandering_traders", 1),
    ("block_drops", 1),
    ("reduced_debug_info", 0),
    ("show_death_messages", 1),
    ("spawner_blocks_work", 1),
    ("spawn_monsters", 1),
    ("tnt_explodes", 1),
    ("player_movement_check", 1),
    ("immediate_respawn", 0),
    ("block_explosion_drop_decay", 1),
    ("spread_vines", 1),
    ("max_entity_cramming", 24),
    ("forgive_dead_players", 1),
    ("fall_damage", 1),
    ("send_command_feedback", 1),
    ("global_sound_events", 1),
    ("elytra_movement_check", 1),
    ("fire_spread_radius_around_player", 128),
    ("freeze_damage", 1),
    ("mob_explosion_drop_decay", 1),
    ("natural_health_regeneration", 1),
    ("players_nether_portal_default_delay", 80),
    ("mob_drops", 1),
    ("log_admin_commands", 1),
    ("mob_griefing", 1),
    ("spawn_mobs", 1),
    ("pvp", 1),
    ("spectators_generate_chunks", 1),
    ("max_command_sequence_length", 65536),
    ("players_nether_portal_creative_delay", 0),
    ("players_sleeping_percentage", 100),
    ("advance_weather", 1),
    ("max_block_modifications", 32768),
    ("max_command_forks", 65536),
    ("drowning_damage", 1),
    ("command_block_output", 1),
    ("locator_bar", 1),
    ("show_advancement_messages", 1),
    ("respawn_radius", 10),
    ("raids", 1),
    ("spawn_phantoms", 1),
    ("max_snow_accumulation_height", 1),
    ("limited_crafting", 0),
    ("allow_entering_nether_using_portals", 1),
    ("lava_source_conversion", 0),
    ("tnt_explosion_drop_decay", 0),
    ("universal_anger", 0),
    ("keep_inventory", 0),
    ("spawn_patrols", 1),
    ("random_tick_speed", 3),
    ("fire_damage", 1),
    ("entity_drops", 1),
    ("advance_time", 1),
    ("command_blocks_work", 1),
    ("spawn_wardens", 1),
    ("water_source_conversion", 1),
    ("projectiles_can_break_blocks", 1),
    ("ender_pearls_vanish_on_death", 1),
];
/// Integer-valued rules; the others are booleans stored as bytes.
const INT_RULES: [&str; 11] = [
    "max_snow_accumulation_height",
    "random_tick_speed",
    "max_entity_cramming",
    "fire_spread_radius_around_player",
    "players_nether_portal_default_delay",
    "max_command_sequence_length",
    "players_nether_portal_creative_delay",
    "players_sleeping_percentage",
    "max_block_modifications",
    "max_command_forks",
    "respawn_radius",
];

/// The world presets of 26.3's `worldgen/world_preset` that the create
/// screen offers, in the order of `WorldPresetTags.NORMAL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldPreset {
    Normal,
    Flat,
    LargeBiomes,
    Amplified,
    SingleBiomeSurface,
    /// `minecraft:debug_all_block_states`: a grid of every block state
    /// (`DebugLevelSource`). The create screen does not offer it; a data
    /// pack, `level-type` or a saved world names it.
    Debug,
}
impl WorldPreset {
    /// The presets, in the order the world-type button cycles through them.
    pub const ALL: [Self; 5] = [
        Self::Normal,
        Self::Flat,
        Self::LargeBiomes,
        Self::Amplified,
        Self::SingleBiomeSurface,
    ];
    /// The preset's registry id (`minecraft:normal`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Normal => "minecraft:normal",
            Self::Flat => "minecraft:flat",
            Self::LargeBiomes => "minecraft:large_biomes",
            Self::Amplified => "minecraft:amplified",
            Self::SingleBiomeSurface => "minecraft:single_biome_surface",
            Self::Debug => "minecraft:debug_all_block_states",
        }
    }
    /// The preset an id names, if it is one of them.
    pub fn of_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Debug])
            .find(|preset| preset.id() == id)
    }
    /// The language key the world-type button shows
    /// (`Identifier.toLanguageKey("generator")`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "generator.minecraft.normal",
            Self::Flat => "generator.minecraft.flat",
            Self::LargeBiomes => "generator.minecraft.large_biomes",
            Self::Amplified => "generator.minecraft.amplified",
            Self::SingleBiomeSurface => "generator.minecraft.single_biome_surface",
            Self::Debug => "generator.minecraft.debug_all_block_states",
        }
    }
    /// The preset the world-type button cycles to next.
    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
    /// The preset the world-type button cycles to previously.
    pub fn prev(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
    /// The preset a saved overworld generator is: the noise settings name
    /// the preset, and the single-biome preset shares the overworld's noise
    /// settings, so its fixed biome source tells the two apart.
    pub fn of_generator(generator: &Tag) -> Self {
        if text(generator.get("type")).as_deref() == Some("minecraft:flat") {
            return Self::Flat;
        }
        if text(generator.get("type")).as_deref() == Some("minecraft:debug") {
            return Self::Debug;
        }
        let source = generator.get("biome_source");
        let fixed = text(source.and_then(|s| s.get("type"))).as_deref() == Some("minecraft:fixed");
        match text(generator.get("settings")).as_deref() {
            Some("minecraft:large_biomes") => Self::LargeBiomes,
            Some("minecraft:amplified") => Self::Amplified,
            Some("minecraft:overworld") if fixed => Self::SingleBiomeSurface,
            _ => Self::Normal,
        }
    }
}
/// World and player state stored in `level.dat`, the saved-data files and
/// the player file.
/// The world options CreateWorldScreen sets (`WorldOptions` and the world
/// preset) that `world_gen_settings` keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldGen {
    /// The world preset the world is created with.
    pub preset: WorldPreset,
    pub structures: bool,
    pub bonus_chest: bool,
}
impl WorldGen {
    /// A saved world's choices, from its `world_gen_settings`.
    pub fn read(world: &Path) -> Option<Self> {
        let settings = world_gen_settings(world)?;
        let flag = |key: &str, default: bool| number(settings.get(key)).map_or(default, |v| v != 0);
        let generator = settings
            .get("dimensions")
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"));
        Some(Self {
            preset: generator.map_or(WorldPreset::Normal, WorldPreset::of_generator),
            structures: flag("generate_structures", true),
            bonus_chest: flag("bonus_chest", false),
        })
    }
    /// The bundled world preset a saved world's overworld was generated
    /// with, from its generator: `minecraft:amplified` and
    /// `minecraft:large_biomes` by their noise settings, and
    /// `minecraft:single_biome_surface` by a fixed biome source. Anything
    /// else (and any world without settings) is `minecraft:normal`.
    pub fn preset(world: &Path) -> &'static str {
        let settings = world_gen_settings(world);
        let generator = settings
            .as_ref()
            .and_then(|s| s.get("dimensions"))
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"));
        let noise = text(generator.and_then(|g| g.get("settings")));
        let fixed_biome = generator
            .and_then(|g| g.get("biome_source"))
            .and_then(|b| text(b.get("type")))
            .is_some_and(|t| t == "minecraft:fixed");
        match noise.as_deref() {
            Some("minecraft:amplified") => "minecraft:amplified",
            Some("minecraft:large_biomes") => "minecraft:large_biomes",
            Some("minecraft:overworld") if fixed_biome => "minecraft:single_biome_surface",
            _ => "minecraft:normal",
        }
    }
}
impl Default for WorldGen {
    fn default() -> Self {
        Self {
            preset: WorldPreset::Normal,
            structures: true,
            bonus_chest: false,
        }
    }
}
/// Writes a new world's `world_gen_settings` (seed, options and dimensions).
pub fn write_world_gen_settings(dir: &Path, seed: i64, options: WorldGen) -> Result<()> {
    write_world_gen_settings_with(dir, seed, options, None, None)
}
/// [`write_world_gen_settings`] with a flat overworld's own generator
/// settings (FlatLevelGeneratorSettings, as `flatgen::settings_from_json`
/// reads them) instead of the Superflat preset's classic layers, and with the
/// biome the single-biome preset fixes (`CreateBuffetWorldScreen`'s choice,
/// `minecraft:plains` when none was made).
pub fn write_world_gen_settings_with(
    dir: &Path,
    seed: i64,
    options: WorldGen,
    flat_settings: Option<Tag>,
    single_biome: Option<&str>,
) -> Result<()> {
    fs::create_dir_all(dir)?;
    // worldgen/world_preset/flat.json's overworld: the classic flat layers.
    let layer = |block: &str, height: i32| {
        Tag::compound([("block", string(block)), ("height", Tag::Int(height))])
    };
    let classic = || {
        Tag::compound([
            ("biome", string("minecraft:plains")),
            ("features", Tag::Byte(0)),
            ("lakes", Tag::Byte(0)),
            (
                "layers",
                Tag::List(vec![
                    layer("minecraft:bedrock", 1),
                    layer("minecraft:dirt", 2),
                    layer("minecraft:grass_block", 1),
                ]),
            ),
            (
                "structure_overrides",
                Tag::List(vec![
                    string("minecraft:strongholds"),
                    string("minecraft:villages"),
                ]),
            ),
        ])
    };
    let flat = Tag::compound([
        ("type", string("minecraft:overworld")),
        (
            "generator",
            Tag::compound([
                ("type", string("minecraft:flat")),
                ("settings", flat_settings.unwrap_or_else(classic)),
            ]),
        ),
    ]);
    let dimension = |kind: &str, settings: &str, source: Tag| {
        Tag::compound([
            ("type", string(kind)),
            (
                "generator",
                Tag::compound([
                    ("type", string("minecraft:noise")),
                    ("settings", string(settings)),
                    ("biome_source", source),
                ]),
            ),
        ])
    };
    let preset = |p: &str| {
        Tag::compound([
            ("type", string("minecraft:multi_noise")),
            ("preset", string(p)),
        ])
    };
    // The overworld generator of worldgen/world_preset/<preset>.json: the
    // noise settings and the biome source each preset uses.
    let overworld = match options.preset {
        WorldPreset::Flat => flat,
        WorldPreset::LargeBiomes => dimension(
            "minecraft:overworld",
            "minecraft:large_biomes",
            preset("minecraft:overworld"),
        ),
        WorldPreset::Amplified => dimension(
            "minecraft:overworld",
            "minecraft:amplified",
            preset("minecraft:overworld"),
        ),
        WorldPreset::SingleBiomeSurface => dimension(
            "minecraft:overworld",
            "minecraft:overworld",
            Tag::compound([
                ("type", string("minecraft:fixed")),
                ("biome", string(single_biome.unwrap_or("minecraft:plains"))),
            ]),
        ),
        WorldPreset::Normal => dimension(
            "minecraft:overworld",
            "minecraft:overworld",
            preset("minecraft:overworld"),
        ),
        // worldgen/world_preset/debug_all_block_states.json's overworld.
        WorldPreset::Debug => Tag::compound([
            ("type", string("minecraft:overworld")),
            (
                "generator",
                Tag::compound([("type", string("minecraft:debug"))]),
            ),
        ]),
    };
    write_saved_data(
        dir,
        "world_gen_settings",
        Tag::compound([
            ("seed", Tag::Long(seed)),
            (
                "generate_structures",
                Tag::Byte(i8::from(options.structures)),
            ),
            ("bonus_chest", Tag::Byte(i8::from(options.bonus_chest))),
            (
                "dimensions",
                Tag::compound([
                    ("minecraft:overworld", overworld),
                    (
                        "minecraft:the_nether",
                        dimension(
                            "minecraft:the_nether",
                            "minecraft:nether",
                            preset("minecraft:nether"),
                        ),
                    ),
                    (
                        "minecraft:the_end",
                        dimension(
                            "minecraft:the_end",
                            "minecraft:end",
                            Tag::compound([("type", string("minecraft:the_end"))]),
                        ),
                    ),
                ]),
            ),
        ]),
    )
}
#[derive(Clone, Debug, Default)]
pub struct Level {
    pub name: String,
    pub seed: i64,
    pub creative: bool,
    /// The game mode id (0 survival, 1 creative, 2 adventure, 3 spectator);
    /// `creative` wins when the two disagree.
    pub game_mode: i32,
    /// `previousPlayerGameType`: the mode the player left last (the F3+F4 switcher's
    /// return mode), when it has one.
    pub previous_game_mode: Option<i32>,
    pub time: i64,
    pub day_time: i64,
    /// Where the player respawns when it has no bed or anchor: its forced
    /// Overworld point (`spawnpoint`), or the world spawn.
    pub spawn: Option<[f64; 3]>,
    /// A respawn point resolved through its block when the player respawns
    /// (a bed, a respawn anchor, a forced point outside the Overworld):
    /// dimension, block, yaw and `forced`.
    pub respawn_elsewhere: Option<(String, [i32; 3], f32, bool)>,
    pub position: Option<[f64; 3]>,
    pub rotation: [f32; 2],
    pub health: f32,
    /// LivingEntity `AbsorptionAmount`: the golden hearts left.
    pub absorption: f32,
    pub food: i32,
    pub saturation: f32,
    /// (slot, item, count, damage): 0..36 inventory, 100..103 armor (feet..head), 150 offhand.
    pub inventory: Vec<(u8, String, u32, u32)>,
    /// Extra item components of `inventory` entries, by slot.
    pub inventory_components: HashMap<u8, Box<crate::item_data::StackData>>,
    /// Player.XpSeed: the enchanting table seed.
    pub xp_seed: i32,
    /// ServerPlayer.seenCredits: the end poem has been shown.
    pub seen_credits: bool,
    /// ServerPlayer.postEffects: the `post_effect` chains `/posteffect` set.
    pub post_effects: Vec<String>,
    /// Player.lastDeathLocation: the dimension and block of the last death.
    pub last_death: Option<(String, [i32; 3])>,
    /// Player.active_effects: (id, duration, amplifier, ambient, show_particles).
    pub active_effects: Vec<(String, i32, i32, bool, bool)>,
    /// Player.EnderItems: the ender chest inventory (27 slots).
    pub ender_items: Vec<Option<crate::items::ItemStack>>,
    /// The `recipeBook` compound: open/filter settings and unlocked recipes.
    pub recipe_book: crate::recipe_book::Settings,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub xp_total: i32,
    pub weather: crate::world::Weather,
    /// Player.Dimension (minecraft:overworld / minecraft:the_nether).
    pub dimension: String,
    /// World spawn (`level.dat` `spawn`), distinct from a bed respawn point.
    pub world_spawn: Option<[i32; 3]>,
    /// The world spawn's yaw and pitch (`setworldspawn`'s rotation).
    pub world_spawn_angle: [f32; 2],
    pub difficulty: String,
    pub hardcore: bool,
    /// `difficulty_settings.locked` (LevelData.isDifficultyLocked).
    pub difficulty_locked: bool,
    /// `allowCommands`: read from `level.dat` or chosen on the Create New
    /// World screen; unset is written as off.
    pub allow_commands: Option<bool>,
    /// `DataPackConfig` (Create World's Data Packs and Experiments): the
    /// enabled and disabled pack ids. Unset keeps vanilla's defaults.
    pub data_packs: Option<(Vec<String>, Vec<String>)>,
    /// `WorldDataConfiguration`'s `enabled_features`: the feature flag ids
    /// the world enables, `minecraft:vanilla` included. Empty is vanilla's
    /// default, which the level.dat omits.
    pub enabled_features: Vec<String>,
    /// Game rule values by name without namespace (`keep_inventory`, ...).
    pub game_rules: BTreeMap<String, i32>,
    pub player_uuid: Option<[i32; 4]>,
    pub selected_slot: i32,
    pub motion: [f64; 3],
    pub fall_distance: f64,
    pub on_ground: bool,
    pub food_exhaustion: f32,
    pub food_tick_timer: i32,
    /// Abilities.flying: whether creative/spectator flight is active.
    pub flying: bool,
    /// Abilities.flyingSpeed, when the file has one (vanilla's is 0.05).
    pub fly_speed: Option<f32>,
    /// The vehicle the player rides, by UUID (`ServerPlayer.saveParentVehicle`'s
    /// "RootVehicle"), so the ride comes back on the next login.
    pub root_vehicle: Option<[i32; 4]>,
    /// The player compound as loaded, for fields Horizon does not model
    /// (ender chest, effects, abilities, recipe book, attributes, ...).
    pub player_extra: Option<Tag>,
    /// The player was read from the file level.dat names (`singleplayer_uuid`,
    /// or its `.dat_old`): `PlayerList.loadPlayerData` gives the world's owner
    /// that file, not their own.
    pub singleplayer_file: bool,
}
impl Level {
    /// `PrimaryLevelData.setDifficulty`: the level data holds the world's
    /// difficulty, so the game and the file follow each other and a world
    /// that was set to hard is hard when it is opened again.
    pub fn set_difficulty(&mut self, difficulty: u8) {
        self.difficulty = crate::title::DIFFICULTIES[difficulty as usize % 4].to_owned();
    }

    /// `PrimaryLevelData.getDifficulty`: the id of the level data's
    /// difficulty (peaceful 0 .. hard 3), normal when it is not one of them.
    pub fn difficulty_id(&self) -> u8 {
        crate::title::DIFFICULTIES
            .iter()
            .position(|d| *d == self.difficulty)
            .map_or(2, |i| i as u8)
    }
    /// Initialize the offline single-player identity before saving or dropping items.
    pub fn ensure_player_uuid(&mut self) -> [i32; 4] {
        *self.player_uuid.get_or_insert_with(|| {
            let u = horizon_protocol::crypto::offline_uuid("Player");
            [
                (u >> 96) as u32 as i32,
                (u >> 64) as u32 as i32,
                (u >> 32) as u32 as i32,
                u as u32 as i32,
            ]
        })
    }
    pub fn game_rule(&self, name: &str) -> i32 {
        self.game_rules
            .get(name)
            .copied()
            .or_else(|| GAME_RULES.iter().find(|(n, _)| *n == name).map(|(_, v)| *v))
            .unwrap_or(0)
    }
    /// The saved game mode id.
    pub fn mode(&self) -> i32 {
        match (self.creative, self.game_mode) {
            (true, _) => 1,
            (false, 1) => 0,
            (false, m) => m.clamp(0, 3),
        }
    }
    pub fn read(dir: &Path) -> Option<Self> {
        Self::read_root(dir, read_level_dat(dir)?)
    }
    /// `read` from level.dat alone, without the level.dat_old fallback: what
    /// `LevelStorageSource.readLevelSummary` reads for the world list.
    pub fn read_summary(dir: &Path) -> Option<Self> {
        Self::read_root(dir, read_dat(&dir.join("level.dat"))?)
    }
    fn read_root(dir: &Path, root: Tag) -> Option<Self> {
        let data = root.get("Data")?;
        let n = |t: Option<&Tag>| number(t).unwrap_or(0);
        let mut level = Self {
            name: text(data.get("LevelName")).unwrap_or_else(|| "World".into()),
            creative: n(data.get("GameType")) == 1,
            game_mode: n(data.get("GameType")) as i32,
            time: n(data.get("Time")),
            health: 20.0,
            food: 20,
            saturation: 5.0,
            dimension: "minecraft:overworld".into(),
            difficulty: "normal".into(),
            ..Self::default()
        };
        // 26.3: spawn {pos: [I; x, y, z], dimension, yaw, pitch}; older: SpawnX/Y/Z.
        level.world_spawn = match data.get("spawn").and_then(|s| s.get("pos")) {
            Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
            _ => data.get("SpawnX").map(|_| {
                [
                    n(data.get("SpawnX")) as i32,
                    n(data.get("SpawnY")) as i32,
                    n(data.get("SpawnZ")) as i32,
                ]
            }),
        };
        let angle = |k: &str| match data.get("spawn").and_then(|s| s.get(k)) {
            Some(Tag::Float(b)) => f32::from_bits(*b),
            _ => 0.0,
        };
        level.world_spawn_angle = [angle("yaw"), angle("pitch")];
        if let Some(d) = data.get("difficulty_settings") {
            level.difficulty = text(d.get("difficulty")).unwrap_or_else(|| "normal".into());
            level.hardcore = n(d.get("hardcore")) != 0;
            level.difficulty_locked = n(d.get("locked")) != 0;
        }
        level.allow_commands = Some(n(data.get("allowCommands")) != 0);
        // WorldDataConfiguration.MAP_CODEC's `enabled_features`, flat in the
        // Data compound and omitted when the flags are vanilla's default.
        if let Some(Tag::List(flags)) = data.get("enabled_features") {
            level.enabled_features = flags
                .iter()
                .filter_map(|f| match f {
                    Tag::String(s) => Some(s.to_string_lossy()),
                    _ => None,
                })
                .collect();
        }
        let mut rules: BTreeMap<String, i32> = GAME_RULES
            .iter()
            .map(|(k, v)| ((*k).to_owned(), *v))
            .collect();
        let saved_rules = read_saved_data(dir, "game_rules");
        if let Some(Tag::Compound(map)) = &saved_rules {
            for (k, v) in map {
                let name = k.to_string_lossy();
                let name = name.strip_prefix("minecraft:").unwrap_or(&name).to_owned();
                if let Some(v) = number(Some(v)) {
                    rules.insert(name, v as i32);
                }
            }
        }
        level.game_rules = rules;
        // Seed: world_gen_settings.dat (26.3) or Data.WorldGenSettings.
        level.seed = read_saved_data(dir, "world_gen_settings")
            .and_then(|w| number(w.get("seed")))
            .or_else(|| number(data.get("WorldGenSettings").and_then(|w| w.get("seed"))))
            .unwrap_or(0);
        // Day time: the overworld clock (26.3) or Data.DayTime.
        level.day_time = read_saved_data(dir, "world_clocks")
            .and_then(|c| {
                number(
                    c.get("minecraft:overworld")
                        .and_then(|o| o.get("total_ticks")),
                )
            })
            .or_else(|| number(data.get("DayTime")))
            .unwrap_or(level.time);
        let weather = read_saved_data(dir, "weather");
        let w = |new: &str, old: &str| {
            number(weather.as_ref().and_then(|w| w.get(new)))
                .or_else(|| number(data.get(old)))
                .unwrap_or(0)
        };
        let raining = w("raining", "raining") != 0;
        let thundering = w("thundering", "thundering") != 0;
        level.weather = crate::world::Weather {
            clear_time: w("clear_weather_time", "clearWeatherTime") as i32,
            rain_time: w("rain_time", "rainTime") as i32,
            thunder_time: w("thunder_time", "thunderTime") as i32,
            raining,
            thundering,
            rain_level: if raining { (1.0, 1.0) } else { (0.0, 0.0) },
            thunder_level: if thundering { (1.0, 1.0) } else { (0.0, 0.0) },
        };
        // The player: players/data/<singleplayer_uuid>.dat, or the legacy Data.Player. With no
        // `singleplayer_uuid` (a world from a server, a Realms download) the owner loads their
        // own file, not another player's (`PlayerList.loadPlayerData`): `load_owner` does.
        let uuid = uuid_from_tag(data.get("singleplayer_uuid"));
        let player_dir = dir.join("players").join("data");
        // PlayerDataStorage.load: the player's file, else its .dat_old.
        let owner = uuid.and_then(|u| {
            let name = uuid_text(u);
            read_dat(&player_dir.join(format!("{name}.dat")))
                .or_else(|| read_dat(&player_dir.join(format!("{name}.dat_old"))))
        });
        level.singleplayer_file = owner.is_some();
        let player = owner.or_else(|| data.get("Player").cloned());
        level.player_uuid = uuid_from_tag(player.as_ref().and_then(|p| p.get("UUID"))).or(uuid);
        let spawn = level
            .world_spawn
            .map(|s| [f64::from(s[0]), f64::from(s[1]), f64::from(s[2])]);
        level.spawn = spawn;
        if let Some(p) = &player {
            level.read_player(p);
        }
        level.player_extra = player;
        Some(level)
    }
    pub fn read_player(&mut self, p: &Tag) {
        let n = |k: &str| number(p.get(k));
        let f = |k: &str| float(p.get(k));
        self.position = read_doubles::<3>(p.get("Pos"));
        if let Some(r) = read_doubles::<2>(p.get("Rotation")) {
            self.rotation = [r[0] as f32, r[1] as f32];
        }
        self.motion = read_doubles::<3>(p.get("Motion")).unwrap_or_default();
        self.health = f("Health").map_or(20.0, |v| v as f32);
        self.absorption = f("AbsorptionAmount").map_or(0.0, |v| v as f32);
        self.food = n("foodLevel").map_or(20, |v| v as i32);
        self.saturation = f("foodSaturationLevel").map_or(5.0, |v| v as f32);
        self.food_exhaustion = f("foodExhaustionLevel").unwrap_or(0.0) as f32;
        self.food_tick_timer = n("foodTickTimer").unwrap_or(0) as i32;
        self.xp_level = n("XpLevel").unwrap_or(0) as i32;
        self.xp_progress = f("XpP").unwrap_or(0.0) as f32;
        self.xp_total = n("XpTotal").unwrap_or(0) as i32;
        self.selected_slot = n("SelectedItemSlot").unwrap_or(0) as i32;
        self.fall_distance = f("fall_distance").unwrap_or(0.0);
        self.on_ground = n("OnGround").unwrap_or(0) != 0;
        self.seen_credits = n("seenCredits").unwrap_or(0) != 0;
        self.post_effects = match p.get("post_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|t| match t {
                    Tag::String(s) => Some(s.to_string_lossy()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        self.last_death = p.get("LastDeathLocation").and_then(|d| {
            let Some(Tag::IntArray(pos)) = d.get("pos") else {
                return None;
            };
            Some((
                text(d.get("dimension"))?,
                [*pos.first()?, *pos.get(1)?, *pos.get(2)?],
            ))
        });
        if let Some(mode) = n("playerGameType") {
            self.creative = mode == 1;
            self.game_mode = mode as i32;
        }
        self.previous_game_mode = n("previousPlayerGameType").map(|m| m as i32);
        self.flying = p
            .get("abilities")
            .and_then(|a| number(a.get("flying")))
            .is_some_and(|v| v != 0);
        self.fly_speed = p
            .get("abilities")
            .and_then(|a| float(a.get("flySpeed")))
            .map(|v| v as f32);
        if let Some(d) = text(p.get("Dimension")) {
            self.dimension = d;
        }
        if let Some(respawn) = p.get("respawn")
            && let Some(Tag::IntArray(pos)) = respawn.get("pos")
            && pos.len() == 3
        {
            let dimension =
                text(respawn.get("dimension")).unwrap_or_else(|| "minecraft:overworld".into());
            let forced = number(respawn.get("forced")).is_some_and(|v| v != 0);
            // A forced point in the Overworld (`spawnpoint`) is where the
            // player respawns; anything else (a bed, an anchor) is resolved
            // through its block then.
            if dimension == "minecraft:overworld" && forced {
                self.spawn = Some([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]);
            } else {
                let yaw = match respawn.get("yaw") {
                    Some(Tag::Float(b)) => f32::from_bits(*b),
                    _ => 0.0,
                };
                self.respawn_elsewhere = Some((dimension, [pos[0], pos[1], pos[2]], yaw, forced));
            }
        }
        let mut inventory = Vec::new();
        let mut inventory_components = HashMap::new();
        if let Some(Tag::List(items)) = p.get("Inventory") {
            for it in items {
                let slot = number(it.get("Slot")).unwrap_or(0) as i8 as u8;
                if let Some(stack) = item_from_nbt(it) {
                    let slot = if slot as i8 == -106 { 150 } else { slot };
                    if let Some(extra) = &stack.extra {
                        inventory_components.insert(slot, extra.clone());
                    }
                    // Legacy armor slots 100..103 and offhand -106 (150).
                    let slot = if slot as i8 == -106 { 150 } else { slot };
                    inventory.push((slot, stack.item.to_string(), stack.count, stack.damage));
                }
            }
        }
        if let Some(eq) = p.get("equipment") {
            for (name, slot) in [
                ("feet", 100u8),
                ("legs", 101),
                ("chest", 102),
                ("head", 103),
                ("offhand", 150),
            ] {
                if let Some(stack) = eq.get(name).and_then(item_from_nbt) {
                    inventory.retain(|(s, ..)| *s != slot);
                    if let Some(extra) = &stack.extra {
                        inventory_components.insert(slot, extra.clone());
                    }
                    inventory.push((slot, stack.item.to_string(), stack.count, stack.damage));
                }
            }
        }
        self.inventory = inventory;
        self.ender_items = decode_items(p.get("EnderItems"), 27);
        self.recipe_book = crate::recipe_book::Settings::read(p.get("recipeBook"));
        self.inventory_components = inventory_components;
        self.xp_seed = n("XpSeed").unwrap_or(0) as i32;
        self.active_effects = match p.get("active_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    let id = text(e.get("id"))?;
                    let get = |k: &str| number(e.get(k));
                    Some((
                        id,
                        get("duration").unwrap_or(0) as i32,
                        get("amplifier").unwrap_or(0) as i32,
                        get("ambient").unwrap_or(0) != 0,
                        get("show_particles").is_none_or(|v| v != 0),
                    ))
                })
                .collect(),
            _ => Vec::new(),
        };
        // ServerPlayer.saveParentVehicle: the mount the player rides, by
        // UUID; the vehicle tree stays in the raw compound until it is back.
        self.root_vehicle = p
            .get("RootVehicle")
            .and_then(|r| uuid_from_tag(r.get("Attach")));
    }
    /// The player compound: the loaded one with Horizon's state written over it.
    pub fn player_tag(&self) -> Tag {
        let mut p = compound_or_empty(self.player_extra.clone());
        // Keys rewritten from scratch.
        if let Tag::Compound(map) = &mut p {
            for key in [
                "Inventory",
                "EnderItems",
                "equipment",
                "active_effects",
                "post_effects",
                "Pos",
                "respawn",
                "recipeBook",
                "LastDeathLocation",
                "previousPlayerGameType",
            ] {
                map.remove(&JavaString::from(key));
            }
        }
        let mut inventory = Vec::new();
        let mut equipment = Vec::new();
        for (slot, id, count, damage) in &self.inventory {
            let extra = self.inventory_components.get(slot).cloned();
            let stack = crate::items::ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra,
            };
            match slot {
                0..=35 => {
                    let mut entries = vec![("Slot", Tag::Byte(*slot as i8))];
                    entries.extend(encode_item(&stack));
                    inventory.push(Tag::compound(entries));
                }
                100 => equipment.push(("feet", item_to_nbt(&stack))),
                101 => equipment.push(("legs", item_to_nbt(&stack))),
                102 => equipment.push(("chest", item_to_nbt(&stack))),
                103 => equipment.push(("head", item_to_nbt(&stack))),
                150 => equipment.push(("offhand", item_to_nbt(&stack))),
                _ => {}
            }
        }
        // GameType.updatePlayerAbilities.
        let mode = self.mode();
        let (creative, spectator) = (mode == 1, mode == 3);
        let mut abilities = compound_or_empty(p.get("abilities").cloned());
        for (k, v) in [
            ("invulnerable", creative || spectator),
            ("mayfly", creative || spectator),
            ("instabuild", creative),
            ("mayBuild", mode < 2),
        ] {
            put(&mut abilities, k, Tag::Byte(i8::from(v)));
        }
        if spectator {
            put(&mut abilities, "flying", Tag::Byte(1));
        } else if creative {
            put(&mut abilities, "flying", Tag::Byte(i8::from(self.flying)));
        } else {
            put(&mut abilities, "flying", Tag::Byte(0));
        }
        if let Tag::Compound(map) = &mut abilities {
            map.entry("mayBuild".into()).or_insert(Tag::Byte(1));
            if let Some(speed) = self.fly_speed {
                map.insert("flySpeed".into(), Tag::Float(speed.to_bits()));
            }
            map.entry("flySpeed".into())
                .or_insert(Tag::Float(0.05f32.to_bits()));
            map.entry("walkSpeed".into())
                .or_insert(Tag::Float(0.1f32.to_bits()));
            map.entry("flying".into()).or_insert(Tag::Byte(0));
        }
        let uuid = self.player_uuid.unwrap_or([0, 0, 0, 0]);
        let mut entries: Vec<(&str, Tag)> = vec![
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("UUID", uuid_tag(uuid)),
            ("Rotation", floats(&self.rotation)),
            ("Motion", doubles(&self.motion)),
            ("OnGround", Tag::Byte(i8::from(self.on_ground))),
            ("fall_distance", Tag::Double(self.fall_distance.to_bits())),
            ("Health", Tag::Float(self.health.to_bits())),
            ("AbsorptionAmount", Tag::Float(self.absorption.to_bits())),
            ("foodLevel", Tag::Int(self.food)),
            ("foodSaturationLevel", Tag::Float(self.saturation.to_bits())),
            (
                "foodExhaustionLevel",
                Tag::Float(self.food_exhaustion.to_bits()),
            ),
            ("foodTickTimer", Tag::Int(self.food_tick_timer)),
            ("XpLevel", Tag::Int(self.xp_level)),
            ("XpP", Tag::Float(self.xp_progress.to_bits())),
            ("XpTotal", Tag::Int(self.xp_total)),
            ("SelectedItemSlot", Tag::Int(self.selected_slot)),
            ("XpSeed", Tag::Int(self.xp_seed)),
            (
                "active_effects",
                Tag::List(
                    self.active_effects
                        .iter()
                        .map(|(id, duration, amplifier, ambient, particles)| {
                            Tag::compound([
                                ("id", string(id)),
                                ("duration", Tag::Int(*duration)),
                                ("amplifier", Tag::Byte(*amplifier as i8)),
                                ("ambient", Tag::Byte(i8::from(*ambient))),
                                ("show_particles", Tag::Byte(i8::from(*particles))),
                                ("show_icon", Tag::Byte(i8::from(*particles))),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("playerGameType", Tag::Int(self.mode())),
            ("Inventory", Tag::List(inventory)),
            ("EnderItems", encode_items(&self.ender_items)),
            ("recipeBook", self.recipe_book.write()),
            ("abilities", abilities),
            (
                "Dimension",
                string(if self.dimension.is_empty() {
                    "minecraft:overworld"
                } else {
                    &self.dimension
                }),
            ),
        ];
        // LivingEntity.addAdditionalSaveData omits empty equipment and an empty
        // effect list. PlayerEquipment.isEmpty also counts the selected hotbar
        // item, which the codec never writes: holding one saves `equipment: {}`.
        let holding = self
            .inventory
            .iter()
            .any(|(slot, _, count, _)| i32::from(*slot) == self.selected_slot && *count > 0);
        if holding || !equipment.is_empty() {
            entries.push(("equipment", Tag::compound(equipment)));
        }
        if self.active_effects.is_empty() {
            entries.retain(|(key, _)| *key != "active_effects");
        }
        if let Some(pos) = self.position {
            entries.push(("Pos", doubles(&pos)));
        }
        entries.push(("seenCredits", Tag::Byte(i8::from(self.seen_credits))));
        // `output.store` writes the list even when it is empty.
        entries.push((
            "post_effects",
            Tag::List(self.post_effects.iter().map(|e| string(e)).collect()),
        ));
        if let Some((dimension, pos)) = &self.last_death {
            entries.push((
                "LastDeathLocation",
                Tag::compound([
                    ("dimension", string(dimension)),
                    ("pos", Tag::IntArray(pos.to_vec())),
                ]),
            ));
        }
        // A respawn point other than the world spawn: an anchor elsewhere, or a bed.
        if let Some((dimension, block, yaw, forced)) = &self.respawn_elsewhere {
            let mut respawn = vec![
                ("dimension", string(dimension)),
                ("pos", Tag::IntArray(block.to_vec())),
                ("yaw", Tag::Float(yaw.to_bits())),
                ("pitch", Tag::Float(0f32.to_bits())),
            ];
            if *forced {
                respawn.push(("forced", Tag::Byte(1)));
            }
            entries.push(("respawn", Tag::compound(respawn)));
        } else if let Some(s) = self.spawn {
            let block = [
                s[0].floor() as i32,
                s[1].floor() as i32,
                s[2].floor() as i32,
            ];
            // A point of the player's own in the Overworld other than a bed
            // (`respawn_elsewhere`): `spawnpoint`'s, forced.
            if Some(block) != self.world_spawn {
                entries.push((
                    "respawn",
                    Tag::compound([
                        ("dimension", string("minecraft:overworld")),
                        ("pos", Tag::IntArray(block.to_vec())),
                        ("yaw", Tag::Float(0f32.to_bits())),
                        ("pitch", Tag::Float(0f32.to_bits())),
                        ("forced", Tag::Byte(1)),
                    ]),
                ));
            }
        }
        // ServerPlayer.storeGameTypes: storeNullable, so absent without one.
        if let Some(mode) = self.previous_game_mode {
            entries.push(("previousPlayerGameType", Tag::Int(mode)));
        }
        for (key, value) in entries {
            put(&mut p, key, value);
        }
        if let Tag::Compound(map) = &mut p {
            for (key, value) in [
                ("Air", Tag::Short(300)),
                ("Fire", Tag::Short(-20)),
                ("Invulnerable", Tag::Byte(0)),
                ("PortalCooldown", Tag::Int(0)),
                ("Score", Tag::Int(0)),
                ("SleepTimer", Tag::Short(0)),
                ("XpSeed", Tag::Int(0)),
                ("seenCredits", Tag::Byte(0)),
                ("HurtTime", Tag::Short(0)),
                ("DeathTime", Tag::Short(0)),
                ("FallFlying", Tag::Byte(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        // ServerPlayer.saveParentVehicle: the savers put the whole
        // RootVehicle in the compound; a mount known by UUID alone (older
        // Horizon files) is still named.
        if let Some(vehicle) = self.root_vehicle
            && p.get("RootVehicle").is_none()
        {
            put(
                &mut p,
                "RootVehicle",
                Tag::compound([("Attach", uuid_tag(vehicle))]),
            );
        }
        p
    }
    /// Write every file; a world without a player UUID gets a random one.
    pub fn write(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, true)
    }

    /// Dedicated servers save connected players separately.
    pub fn write_without_player(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, false)
    }

    /// The player's file alone, `players/data/<singleplayer_uuid>.dat`
    /// (`PlayerDataStorage.save`): the one level.dat names.
    pub fn write_player(&mut self, dir: &Path) -> Result<()> {
        let uuid = self.ensure_player_uuid();
        let player_file = dir
            .join("players")
            .join("data")
            .join(format!("{}.dat", uuid_text(uuid)));
        back_up_unreadable_player(&player_file);
        write_dat(&player_file, self.player_tag())
    }

    fn write_internal(&mut self, dir: &Path, write_player: bool) -> Result<()> {
        fs::create_dir_all(dir)?;
        restore_level_dat(dir);
        let existing = read_dat(&dir.join("level.dat")).and_then(|r| r.get("Data").cloned());
        let legacy = existing
            .as_ref()
            .is_some_and(|d| d.get("Player").is_some() || d.get("WorldGenSettings").is_some());
        let mut data = compound_or_empty(existing);
        if let Tag::Compound(map) = &mut data {
            // Pre-26.3 keys now stored elsewhere (a legacy Horizon level.dat).
            for key in [
                "Player",
                "WorldGenSettings",
                "DayTime",
                "SpawnX",
                "SpawnY",
                "SpawnZ",
                "SpawnAngle",
                "clearWeatherTime",
                "rainTime",
                "thunderTime",
                "raining",
                "thundering",
                "Difficulty",
                "hardcore",
                "GameRules",
                "DimensionData",
                "WanderingTraderSpawnChance",
                "WanderingTraderSpawnDelay",
                "WanderingTraderId",
                "BorderCenterX",
                "BorderCenterZ",
                "BorderSize",
                "BorderSafeZone",
                "BorderWarningBlocks",
                "BorderWarningTime",
                "BorderSizeLerpTime",
                "BorderSizeLerpTarget",
                "BorderDamagePerBlock",
                "DifficultyLocked",
                "ScheduledEvents",
                "CustomBossEvents",
                "DragonFight",
            ] {
                map.remove(&JavaString::from(key));
            }
        }
        let uuid = self.ensure_player_uuid();
        // A new world's first spawn point is its world spawn.
        if self.world_spawn.is_none() {
            self.world_spawn = self.spawn.map(|s| {
                [
                    s[0].floor() as i32,
                    s[1].floor() as i32,
                    s[2].floor() as i32,
                ]
            });
        }
        let spawn = self.world_spawn.unwrap_or([0, 64, 0]);
        let mut history: Vec<Tag> = match data.get("version_history") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        if history.last() != Some(&Tag::Int(DATA_VERSION)) {
            history.push(Tag::Int(DATA_VERSION));
        }
        let mut brands: Vec<Tag> = match data.get("ServerBrands") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        if !brands.contains(&string("horizon")) {
            brands.push(string("horizon"));
        }
        let mut difficulty = compound_or_empty(data.get("difficulty_settings").cloned());
        put(
            &mut difficulty,
            "difficulty",
            string(if self.difficulty.is_empty() {
                "normal"
            } else {
                &self.difficulty
            }),
        );
        put(
            &mut difficulty,
            "hardcore",
            Tag::Byte(i8::from(self.hardcore)),
        );
        put(
            &mut difficulty,
            "locked",
            Tag::Byte(i8::from(self.difficulty_locked)),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        for (key, value) in [
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "Version",
                Tag::compound([
                    ("Name", string(VERSION_NAME)),
                    ("Id", Tag::Int(DATA_VERSION)),
                    ("Snapshot", Tag::Byte(0)),
                    ("Series", string("main")),
                ]),
            ),
            ("version_history", Tag::List(history)),
            ("version", Tag::Int(19133)),
            ("LevelName", string(&self.name)),
            ("GameType", Tag::Int(self.mode())),
            ("Time", Tag::Long(self.time)),
            ("LastPlayed", Tag::Long(now)),
            (
                "spawn",
                Tag::compound([
                    ("pos", Tag::IntArray(spawn.to_vec())),
                    ("dimension", string("minecraft:overworld")),
                    ("yaw", Tag::Float(self.world_spawn_angle[0].to_bits())),
                    ("pitch", Tag::Float(self.world_spawn_angle[1].to_bits())),
                ]),
            ),
            ("difficulty_settings", difficulty),
            ("initialized", Tag::Byte(1)),
            ("singleplayer_uuid", uuid_tag(uuid)),
            ("ServerBrands", Tag::List(brands)),
        ] {
            put(&mut data, key, value);
        }
        if let Tag::Compound(map) = &mut data {
            map.entry("allowCommands".into()).or_insert(Tag::Byte(0));
            if let Some(allow) = self.allow_commands {
                map.insert("allowCommands".into(), Tag::Byte(i8::from(allow)));
            }
            map.entry("WasModded".into()).or_insert(Tag::Byte(0));
            // lenientOptionalFieldOf("enabled_features", DEFAULT_FLAGS): the
            // tag is left out when the set is vanilla's default, so a
            // `["minecraft:vanilla"]`-only list does not reach the file.
            let vanilla_only = self.enabled_features == ["minecraft:vanilla"];
            if !self.enabled_features.is_empty() && !vanilla_only {
                map.insert(
                    "enabled_features".into(),
                    Tag::List(self.enabled_features.iter().map(|s| string(s)).collect()),
                );
            }
            map.entry("DataPacks".into()).or_insert_with(|| {
                let (enabled, disabled) = self.data_packs.clone().unwrap_or_else(|| {
                    (
                        vec!["vanilla".into()],
                        [
                            "minecart_improvements",
                            "redstone_experiments",
                            "trade_rebalance",
                        ]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    )
                });
                let list = |ids: Vec<String>| Tag::List(ids.iter().map(|s| string(s)).collect());
                Tag::compound([("Enabled", list(enabled)), ("Disabled", list(disabled))])
            });
        }
        // Saved data: weather, clocks, and (when missing or legacy) world
        // generation settings and game rules.
        write_saved_data(
            dir,
            "weather",
            Tag::compound([
                ("clear_weather_time", Tag::Int(self.weather.clear_time)),
                ("rain_time", Tag::Int(self.weather.rain_time)),
                ("thunder_time", Tag::Int(self.weather.thunder_time)),
                ("raining", Tag::Byte(i8::from(self.weather.raining))),
                ("thundering", Tag::Byte(i8::from(self.weather.thundering))),
            ]),
        )?;
        let mut clocks = compound_or_empty(read_saved_data(dir, "world_clocks"));
        let mut overworld = compound_or_empty(clocks.get("minecraft:overworld").cloned());
        put(&mut overworld, "total_ticks", Tag::Long(self.day_time));
        put(&mut clocks, "minecraft:overworld", overworld);
        if let Tag::Compound(map) = &mut clocks {
            map.entry("minecraft:the_end".into())
                .or_insert_with(|| Tag::compound([("total_ticks", Tag::Long(self.time))]));
        }
        write_saved_data(dir, "world_clocks", clocks)?;
        if legacy || !saved_data_path(dir, "world_gen_settings").exists() {
            write_world_gen_settings(dir, self.seed, WorldGen::default())?;
        }
        if !saved_data_path(dir, "game_rules").exists() || !self.game_rules.is_empty() {
            // Only rules vanilla knows: an unknown key makes it reject the file.
            let mut rules = Tag::Compound(Compound::new());
            for (name, default) in GAME_RULES {
                let value = self.game_rules.get(name).copied().unwrap_or(default);
                let tag = if INT_RULES.contains(&name) {
                    Tag::Int(value)
                } else {
                    Tag::Byte(i8::from(value != 0))
                };
                put(&mut rules, &format!("minecraft:{name}"), tag);
            }
            write_saved_data(dir, "game_rules", rules)?;
        }
        if write_player {
            self.write_player(dir)?;
        }
        write_dat(&dir.join("level.dat"), Tag::compound([("Data", data)]))?;
        Ok(())
    }
}
/// Loads saved chunks before falling back to generation.
pub struct Persistent<G> {
    pub inner: Arc<G>,
    pub save: Arc<SaveDir>,
    pub registry: Arc<crate::assets::Assets>,
    pub biomes: Arc<Biomes>,
}
type LodSurfaces = Arc<[crate::lod::Column; 256]>;
/// The coarsest far tile spacing whose columns read saved chunks: 16 blocks
/// a column, and 4 with Reduced far detail (`LodSampler::set_approximate`).
const SAVED_LOD_STRIDE: i32 = 16;
const SAVED_LOD_STRIDE_REDUCED: i32 = 4;
struct SavedLodSampler<G: ChunkGenerator> {
    generator: Arc<Persistent<G>>,
    inner: Box<dyn crate::streaming::LodSampler>,
    cache: std::collections::VecDeque<((i32, i32), Option<LodSurfaces>)>,
    details: Vec<crate::streaming::LodDetail>,
    /// Reduced far detail: saved chunks only in tiles up to
    /// `SAVED_LOD_STRIDE_REDUCED` blocks a column.
    approximate: bool,
    /// Chunks the generator decorated for far terrain alone, kept from
    /// earlier sessions; saved chunks take precedence over them.
    generated: Arc<crate::lod::cache::DiskCache>,
}
impl<G: ChunkGenerator> SavedLodSampler<G> {
    fn saved_columns(&mut self, key: (i32, i32)) -> Option<LodSurfaces> {
        let cached = match self.cache.iter().position(|(at, _)| *at == key) {
            Some(index) => self.cache.remove(index).unwrap(),
            None => (
                key,
                self.generator.save.lod_surfaces_shared(
                    key.0,
                    key.1,
                    &self.generator.registry,
                    &self.generator.biomes,
                ),
            ),
        };
        let columns = cached.1.clone();
        self.cache.push_front(cached);
        self.cache.truncate(128);
        columns
    }
}
impl<G: ChunkGenerator> crate::streaming::LodSampler for SavedLodSampler<G> {
    fn fork(&self) -> Option<Box<dyn crate::streaming::LodSampler>> {
        Some(Box::new(Self {
            generator: self.generator.clone(),
            inner: self.inner.fork()?,
            cache: Default::default(),
            details: Vec::new(),
            approximate: self.approximate,
            generated: self.generated.clone(),
        }))
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
        self.inner.set_approximate(approximate);
    }
    fn request_detail(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        let mut missing = Vec::new();
        for &key in chunks {
            // Generated chunks from earlier sessions answer only these
            // requests: reading them for every sampled cell of a coarse tile
            // slowed sampling more than it saved.
            if let Some(columns) = self
                .saved_columns(key)
                .or_else(|| self.generated.get(key.0, key.1))
            {
                self.details.push((key, columns));
            } else {
                missing.push(key);
            }
        }
        self.inner.request_detail(&missing, center);
    }
    fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
        let mut details = std::mem::take(&mut self.details);
        for (key, generated) in self.inner.drain_details() {
            // A generation batch also returns neighbouring, unrequested chunks.
            // Their saved builds must take precedence too.
            let columns = self.saved_columns(key).unwrap_or_else(|| {
                self.generated.put(key.0, key.1, &generated);
                generated
            });
            details.push((key, columns));
        }
        details
    }
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<crate::lod::Column> {
        let mut columns = self.inner.grid(x, z, stride, side);
        // Saved chunks show in tiles up to 16 blocks a column. With Reduced far
        // detail only up to 4 (within about a kilometre): coarser tiles keep
        // the generator's terrain, which saved chunks were made from, since
        // decoding up to 256 saved chunks for each of them held the whole far
        // view back for minutes in large saves. Edits that far away show once
        // their tiles refine on approach.
        let limit = if self.approximate {
            SAVED_LOD_STRIDE_REDUCED
        } else {
            SAVED_LOD_STRIDE
        };
        if stride <= limit {
            for (index, column) in columns.iter_mut().enumerate() {
                let (wx, wz) = (
                    x + (index % side) as i32 * stride,
                    z + (index / side) as i32 * stride,
                );
                if let Some(saved) = self.saved_columns((wx >> 4, wz >> 4)) {
                    *column = crate::lod::Column::reduced(
                        &saved,
                        (wx & 15) as usize,
                        (wz & 15) as usize,
                        stride as usize,
                    );
                }
            }
        }
        columns
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface> {
        self.column(x, z).surface
    }
    fn column(&mut self, x: i32, z: i32) -> crate::lod::Column {
        self.reduced_column(x, z, 1)
    }
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> crate::lod::Column {
        if width <= 16
            && let Some(columns) = self.saved_columns((x >> 4, z >> 4))
        {
            return crate::lod::Column::reduced(
                &columns,
                (x & 15) as usize,
                (z & 15) as usize,
                width as usize,
            );
        }
        self.inner.reduced_column(x, z, width)
    }
}

/// Read one palette index without expanding the other entries in the section.
fn lod_palette_index(
    container: &Tag,
    index: usize,
    entries: usize,
    minimum_bits: u32,
) -> Option<usize> {
    let Tag::List(palette) = container.get("palette")? else {
        return None;
    };
    if palette.is_empty() || palette.len() > entries || index >= entries {
        return None;
    }
    if palette.len() == 1 {
        return Some(0);
    }
    let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(minimum_bits);
    let per_word = 64 / bits as usize;
    let Tag::LongArray(data) = container.get("data")? else {
        return None;
    };
    if data.len() != entries.div_ceil(per_word) {
        return None;
    }
    let value = ((data[index / per_word] as u64 >> ((index % per_word) * bits as usize))
        & ((1 << bits) - 1)) as usize;
    (value < palette.len()).then_some(value)
}
fn lod_light_layers(sections: &[(i32, &Tag)], layout: ColumnLayout, sky_light: bool) -> Box<[u8]> {
    let mut layers = vec![(None, None); layout.sections()];
    for &(y, section) in sections {
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(data)) if data.len() == 2048 => Some(&data[..]),
            _ => None,
        };
        layers[(y - (layout.min_y() >> 4)) as usize] = (
            if sky_light { layer("SkyLight") } else { None },
            layer("BlockLight"),
        );
    }
    let mut light = vec![0u8; layout.sections() * 4096].into_boxed_slice();
    let mut above = [if sky_light { 15u8 } else { 0 }; 256];
    let nibble = |data: &[u8], index: usize| (data[index / 2] >> ((index & 1) * 4)) & 15;
    for si in (0..layout.sections()).rev() {
        let (sky, block) = layers[si];
        let section = &mut light[si * 4096..(si + 1) * 4096];
        for (index, value) in section.iter_mut().enumerate() {
            *value = sky.map_or(above[index & 255], |data| nibble(data, index)) << 4
                | block.map_or(0, |data| nibble(data, index));
        }
        // Missing sky layers inherit the bottom layer above, as load_light does.
        for (index, value) in above.iter_mut().enumerate() {
            *value = section[index] >> 4;
        }
    }
    light
}
fn lod_face_light(light: &[u8], x: usize, y: usize, z: usize) -> u16 {
    let index = y * 256 + z * 16 + x;
    let mut sky = light[index] >> 4;
    let mut block = light[index] & 15;
    for (valid, index) in [
        (x > 0, index.wrapping_sub(1)),
        (x < 15, index + 1),
        (z > 0, index.wrapping_sub(16)),
        (z < 15, index + 16),
        (y > 0, index.wrapping_sub(256)),
        (index + 256 < light.len(), index + 256),
    ] {
        if valid {
            sky = sky.max(light[index] >> 4);
            block = block.max(light[index] & 15);
        }
    }
    u16::from(sky) << 4 | u16::from(block) << 12
}
fn decode_lod_surfaces(
    root: &NamedTag,
    assets: &crate::assets::Assets,
    biomes: &Biomes,
    layout: ColumnLayout,
    sky_light: bool,
) -> Option<LodSurfaces> {
    let Tag::List(sections) = root.tag.get("sections")? else {
        return None;
    };
    let water_state = assets.registry.default_state("minecraft:water");
    let mut sections: Vec<_> = sections
        .iter()
        .filter_map(|section| {
            let y = number(section.get("Y"))? as i32;
            layout
                .section_index(y * 16)
                .is_some()
                .then_some((y, section))
        })
        .collect();
    sections.sort_unstable_by_key(|(y, _)| std::cmp::Reverse(*y));
    let mut columns: Box<[crate::lod::Column; 256]> =
        Box::new(std::array::from_fn(|_| crate::lod::Column {
            surface: None,
            spans: crate::lod::Spans::new(),
        }));
    let mut water = [None; 256];
    let light = lod_light_layers(&sections, layout, sky_light);
    let mut states = BTreeMap::new();
    for (section_y, section) in sections {
        if let Some(blocks) = section.get("block_states") {
            let Tag::List(palette) = blocks.get("palette")? else {
                return None;
            };
            // Resolve each state once per palette, rather than parsing NBT for every block.
            let palette: Vec<_> = palette
                .iter()
                .map(|entry| {
                    let descriptor = BlockState::from_nbt(entry).ok()?;
                    let state = *states.entry(descriptor.clone()).or_insert_with(|| {
                        assets
                            .registry
                            .id(&descriptor)
                            .or_else(|| assets.registry.default_state(&descriptor.name))
                    });
                    Some(state.map(|state| {
                        let properties = assets.properties.heightmap_state(state);
                        let fluid = matches!(
                            descriptor.name.as_str(),
                            "minecraft:water" | "minecraft:lava"
                        );
                        // Chorus is thin and non-solid but still far-End detail.
                        let chorus = matches!(
                            descriptor.name.as_str(),
                            "minecraft:chorus_plant" | "minecraft:chorus_flower"
                        );
                        let solid = properties.blocks_motion_in_heightmap();
                        // Kelp and seagrass: the water they hold. Left out, the
                        // water stopped short where they reach the surface.
                        match water_state {
                            Some(water) if !fluid && !solid && properties.has_fluid() => {
                                (water, true, true)
                            }
                            _ => (state, fluid, fluid || solid || chorus),
                        }
                    }))
                })
                .collect::<Option<_>>()?;
            let biome_palette: Vec<_> = section
                .get("biomes")
                .and_then(|container| match container.get("palette")? {
                    Tag::List(palette) => Some(palette),
                    _ => None,
                })
                .into_iter()
                .flatten()
                .map(|entry| match entry {
                    Tag::String(name) => biomes.id(&name.to_string_lossy()),
                    _ => 0,
                })
                .collect();
            for z in 0..16 {
                for x in 0..16 {
                    let cell = z * 16 + x;
                    let column = &mut columns[cell];
                    for y in (0..16).rev() {
                        let index = y * 256 + cell;
                        let palette_index = lod_palette_index(blocks, index, 4096, 4)?;
                        let Some((state, fluid, kept)) = palette[palette_index] else {
                            continue;
                        };
                        if !kept {
                            continue;
                        }
                        let biome = section
                            .get("biomes")
                            .and_then(|container| {
                                lod_palette_index(
                                    container,
                                    (y / 4) * 16 + (z / 4) * 4 + x / 4,
                                    64,
                                    1,
                                )
                            })
                            .and_then(|index| biome_palette.get(index))
                            .copied()
                            .unwrap_or(0);
                        let height = section_y * 16 + y as i32 + 1;
                        if column.surface.is_none() {
                            if fluid {
                                water[cell].get_or_insert((height, state));
                            } else {
                                column.surface = Some(crate::lod::Surface {
                                    height,
                                    state,
                                    water: water[cell],
                                    biome,
                                });
                            }
                        }
                        let light =
                            lod_face_light(&light, x, (height - 1 - layout.min_y()) as usize, z);
                        let min = (height - 1) as i16;
                        let max = height as i16;
                        if let Some(previous) = column.spans.last_mut()
                            && previous.min == max
                            && (previous.state, previous.biome, previous.light)
                                == (state, biome, light)
                        {
                            previous.min = min;
                        } else {
                            column.spans.push(crate::lod::Span {
                                min,
                                max,
                                state,
                                biome,
                                light,
                            });
                        }
                    }
                }
            }
        }
    }
    for column in &mut *columns {
        column.spans.reverse();
    }
    Some(Arc::from(columns))
}

impl SaveDir {
    /// `lod_surfaces`, decoded once for all far sampling threads: each saved
    /// chunk lies under far tiles of several levels, sampled on any thread.
    fn lod_surfaces_shared(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        if let Some((at, columns)) = self.lod_decoded.lock().unwrap().chunks.get(&(x, z))
            && at.elapsed() < LOD_DECODED_TTL
        {
            return Some(columns.clone());
        }
        let columns = self.lod_surfaces(x, z, assets, biomes)?;
        let mut decoded = self.lod_decoded.lock().unwrap();
        let fresh = (std::time::Instant::now(), columns.clone());
        if decoded.chunks.insert((x, z), fresh).is_none() {
            decoded.order.push_back((x, z));
            while decoded.order.len() > LOD_DECODED {
                if let Some(old) = decoded.order.pop_front() {
                    decoded.chunks.remove(&old);
                }
            }
        }
        Some(columns)
    }
    fn lod_surfaces(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        let pending = self
            .pending
            .lock()
            .ok()?
            .get(&(Folder::Region, x, z))
            .map(|(_, record)| record.clone());
        let root = if let Some(record) = pending {
            record.tag().map(|tag| (*tag).clone())?
        } else {
            if !self.lod_saved(x, z) {
                return None;
            }
            // Far-only reads must not populate the gameplay region cache across the whole view.
            let mut region = RegionFile::open(self.region_path("region", x >> 5, z >> 5)).ok()?;
            let mut root = region
                .read(ChunkPos::new(x, z))
                .ok()??
                .nbt(Limits::default())
                .ok()?;
            self.upgrade_record(Folder::Region, &mut root).ok()?;
            root
        };
        // Proto saves may still be missing decoration from their neighbors.
        if text(root.tag.get("Status"))
            .is_some_and(|status| status != "minecraft:full" && status != "full")
        {
            return None;
        }
        decode_lod_surfaces(
            &root,
            assets,
            biomes,
            self.shape.column(),
            self.shape.sky_light,
        )
    }
}

impl<G: ChunkGenerator> ChunkGenerator for Persistent<G> {
    type Worker = G::Worker;
    fn worker(&self) -> G::Worker {
        self.inner.worker()
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn crate::streaming::LodSampler> {
        let inner = ChunkGenerator::lod_sampler(self.inner.clone());
        let generated = Arc::new(crate::lod::cache::DiskCache::new(
            self.save.root.join("data/horizon_far_terrain"),
            crate::lod::cache::build_key(self.registry.registry.len(), self.biomes.list.len()),
        ));
        Box::new(SavedLodSampler {
            generator: self,
            inner,
            cache: Default::default(),
            details: Vec::new(),
            approximate: false,
            generated,
        })
    }
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let mut chunk = self.save.load(x, z, self.registry.registry, &self.biomes)?;
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        chunk.entities = self.save.load_entities(x, z);
        Some(chunk)
    }
    fn lod_surface(&self, worker: &mut G::Worker, x: i32, z: i32) -> Option<crate::lod::Surface> {
        // Resident/visited summaries override this procedural fallback in the LOD cache.
        self.inner.lod_surface(worker, x, z)
    }
    fn prepare_loaded(&self, worker: &mut G::Worker, chunk: &Chunk) {
        self.inner.prepare_loaded(worker, chunk);
    }
    fn idle(&self, worker: &mut G::Worker) {
        self.inner.idle(worker);
    }
    fn proto(&self, worker: &mut G::Worker, x: i32, z: i32) -> crate::streaming::Proto {
        let mut proto = self.inner.proto(worker, x, z);
        if let Some(saved) = self
            .save
            .load_proto(x, z, self.registry.registry, &self.biomes)
        {
            self.inner.restore_proto(&mut proto, saved);
        }
        proto
    }
    fn is_decorated(&self, proto: &crate::streaming::Proto) -> bool {
        self.inner.is_decorated(proto)
    }
    fn light_view(&self, proto: &crate::streaming::Proto) -> Option<Chunk> {
        self.inner.light_view(proto)
    }
    fn non_air_sections(
        &self,
        proto: &crate::streaming::Proto,
    ) -> Option<crate::section_mask::SectionMask> {
        self.inner.non_air_sections(proto)
    }
    fn save_protos(&self, protos: &[&crate::streaming::Proto]) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let saved: Vec<SavedProto> = protos
            .iter()
            .filter_map(|p| self.inner.saved_proto(p))
            .collect();
        if let Err(e) = self
            .save
            .save_protos(&saved, self.registry.registry, &self.biomes)
        {
            log::warn!("Horizon: failed to save proto chunks: {e}");
        }
    }
    fn save_finished(&self, mut chunk: Chunk) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let key = (chunk.x, chunk.z);
        let entities = std::mem::take(&mut chunk.entities);
        let saved = self
            .save
            .save_later(&[Arc::new(chunk)], self.registry.registry, &self.biomes)
            .and_then(|_| self.save.save_entities_later(vec![(key, entities)]));
        if let Err(e) = saved {
            log::warn!("Horizon: failed to save chunk {} {}: {e}", key.0, key.1);
        }
    }
    fn decorates(&self) -> bool {
        self.inner.decorates()
    }
    fn decorates_beside_final(&self) -> bool {
        self.inner.decorates_beside_final()
    }
    fn proto_beside_final(
        &self,
        worker: &mut G::Worker,
        snapshot: &Chunk,
    ) -> Option<crate::streaming::Proto> {
        self.inner.proto_beside_final(worker, snapshot)
    }
    fn final_writes(
        &self,
        snapshot: &Chunk,
        proto: &crate::streaming::Proto,
    ) -> Vec<([i32; 3], u32)> {
        self.inner.final_writes(snapshot, proto)
    }
    fn final_block_entities(
        &self,
        proto: &crate::streaming::Proto,
        writes: &[([i32; 3], u32)],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        self.inner.final_block_entities(proto, writes)
    }
    fn decorate(&self, worker: &mut G::Worker, region: &mut [crate::streaming::Proto; 9]) {
        self.inner.decorate(worker, region);
    }
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        self.inner.finish(proto)
    }
}

#[cfg(test)]
mod tests {
    use crate::mobs::spawner::SpawnDataExt;

    #[test]
    fn a_server_falls_back_to_level_dat_old_and_restores_it() {
        let world = std::env::temp_dir().join(format!("horizon-dat-old-{}", std::process::id()));
        let _ = fs::remove_dir_all(&world);
        fs::create_dir_all(&world).unwrap();
        let level = |name: &str| {
            Tag::compound([(
                "Data",
                Tag::compound([("LevelName", Tag::String(name.into()))]),
            )])
        };
        // Both files broken, or only level.dat_old there: nothing to fall back to.
        fs::write(world.join("level.dat"), b"cut short").unwrap();
        assert!(!fall_back_to_level_dat_old(&world));
        fs::write(world.join("level.dat_old"), b"cut short").unwrap();
        assert!(!fall_back_to_level_dat_old(&world));
        // A good level.dat_old: it takes level.dat's place and the broken file is kept.
        write_dat(&world.join("level.dat_old"), level("old")).unwrap();
        assert!(fall_back_to_level_dat_old(&world));
        assert!(level_dat_issue(&world, false).is_none());
        let kept = fs::read_dir(&world)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("level.dat_corrupted_")
            })
            .count();
        assert_eq!(kept, 1);
        // A readable level.dat is left alone.
        assert!(!fall_back_to_level_dat_old(&world));
        let _ = fs::remove_dir_all(&world);
    }

    #[test]
    fn points_of_interest_save_as_poi_sections() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let assets = crate::assets::Assets::load(&PathBuf::from(root)).expect("assets");
        let tables = SaveTables::new(&assets);
        let state = |name: &str, props: &[(&str, &str)]| {
            let mut s = assets
                .registry
                .state(assets.registry.default_state(name).unwrap())
                .unwrap()
                .clone();
            for (k, v) in props {
                s.properties.insert((*k).into(), (*v).into());
            }
            assets.registry.id(&s).unwrap().0
        };
        let mut chunk = Chunk::empty(2, -1);
        chunk.set(1, 70, 1, state("minecraft:red_bed", &[("part", "head")]));
        chunk.set(1, 70, 2, state("minecraft:red_bed", &[("part", "foot")]));
        chunk.set(5, 71, 5, state("minecraft:bell", &[]));
        chunk.set(6, 71, 5, state("minecraft:stone", &[]));
        chunk.set(9, 70, 9, state("minecraft:straw_bed", &[("part", "head")]));
        assert!(encode_poi(&Chunk::empty(0, 0), &tables).is_none());
        let saved = encode_poi(&chunk, &tables).expect("records");
        let Some(Tag::Compound(sections)) = saved.tag.get("Sections") else {
            panic!("no sections");
        };
        assert_eq!(sections.len(), 1);
        let section = &sections[&"4".into()];
        assert_eq!(section.get("Valid"), Some(&Tag::Byte(1)));
        let Some(Tag::List(records)) = section.get("Records") else {
            panic!("no records");
        };
        let records: Vec<_> = records
            .iter()
            .map(|r| {
                (
                    text(r.get("type")).unwrap(),
                    r.get("pos").cloned(),
                    number(r.get("free_tickets")),
                )
            })
            .collect();
        assert_eq!(
            records,
            [
                (
                    "minecraft:home".into(),
                    Some(Tag::IntArray(vec![33, 70, -15])),
                    Some(1)
                ),
                (
                    "minecraft:meeting".into(),
                    Some(Tag::IntArray(vec![37, 71, -11])),
                    Some(32)
                ),
            ]
        );
    }

    #[test]
    fn a_chunk_that_loses_its_last_poi_saves_empty_sections() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Arc::new(Biomes::load(&root).expect("biomes"));
        let bell = assets.registry.default_state("minecraft:bell").unwrap().0;
        let dir = std::env::temp_dir().join(format!("horizon-poi-save-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let _ = save.tables.set(Arc::new(SaveTables::new(&assets)));
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(5, 71, 5, bell);
        let sections = |save: &SaveDir| match save.read_record("poi", 0, 0)?.tag.get("Sections") {
            Some(Tag::Compound(s)) => Some(s.len()),
            _ => None,
        };
        save.save_later(&[Arc::new(chunk.clone())], assets.registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        assert_eq!(sections(&save), Some(1));
        chunk.set(5, 71, 5, 0);
        save.save_later(&[Arc::new(chunk)], assets.registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        assert_eq!(
            sections(&SaveDir::new(dir.clone(), LevelShape::OVERWORLD)),
            Some(0)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_poi_records_keep_their_claims() {
        let record = |pos: [i32; 3], kind: &str, tickets: Option<i32>| {
            let mut r = vec![("pos", Tag::IntArray(pos.to_vec())), ("type", string(kind))];
            if let Some(t) = tickets {
                r.push(("free_tickets", Tag::Int(t)));
            }
            Tag::compound(r)
        };
        let poi = |records: Vec<Tag>| {
            let mut sections = Compound::new();
            sections.insert(
                "4".into(),
                Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(records))]),
            );
            poi_root(sections)
        };
        let old = poi(vec![
            record([1, 70, 1], "minecraft:home", None),
            record([5, 71, 5], "minecraft:meeting", Some(30)),
        ]);
        let mut saved = poi(vec![
            record([1, 70, 1], "minecraft:home", Some(1)),
            record([5, 71, 5], "minecraft:meeting", Some(32)),
            record([9, 71, 9], "minecraft:home", Some(1)),
        ]);
        keep_claims(&mut saved, &old);
        let expected = poi(vec![
            record([1, 70, 1], "minecraft:home", None),
            record([5, 71, 5], "minecraft:meeting", Some(30)),
            record([9, 71, 9], "minecraft:home", Some(1)),
        ]);
        assert_eq!(saved.tag, expected.tag);
    }

    #[test]
    fn level_attachments_survive_save_and_load() {
        use horizon_world::attachments::{
            AttachmentCodec, AttachmentError, AttachmentType, Attachments,
        };
        let dir =
            std::env::temp_dir().join(format!("horizon-level-attachments-{}", std::process::id()));
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let key = AttachmentType {
            namespace: "test",
            name: "counter",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
                decode: |c: &Compound| match c.get(&"count".into()) {
                    Some(Tag::Int(n)) => Ok(*n),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            copy_on_death: false,
        };
        let mut map = Attachments::default();
        map.set(&key, 7).unwrap();
        save.save_attachments(&map).unwrap();
        assert_eq!(save.load_attachments().get(&key), Ok(7));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chunk_attachments_survive_save_and_load() {
        use horizon_world::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&root).expect("biomes");
        let key = AttachmentType {
            namespace: "test",
            name: "counter",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
                decode: |c: &Compound| match c.get(&"count".into()) {
                    Some(Tag::Int(n)) => Ok(*n),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            copy_on_death: false,
        };
        let mut chunk = Chunk::empty(2, 3);
        let mut extras = ChunkExtras::default();
        extras.attachments.set(&key, 42).unwrap();
        let mut block_data = horizon_world::attachments::Attachments::default();
        block_data.set(&key, 21).unwrap();
        let pos = (33, 64, 50);
        let chest = assets.registry.default_state("minecraft:chest").unwrap();
        chunk.set(1, 64, 2, chest.0);
        chunk
            .block_entities
            .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
        extras.block_entity_attachments.push((pos, block_data));
        chunk.saved = Some(Arc::new(extras));
        let saved = encode(&chunk, assets.registry, &biomes);
        let mut loaded = decode(&saved, assets.registry, &biomes).expect("chunk");
        assert_eq!(
            Arc::make_mut(loaded.saved.as_mut().unwrap())
                .attachments
                .get(&key),
            Ok(42)
        );
        let extras = Arc::make_mut(loaded.saved.as_mut().unwrap());
        assert_eq!(extras.block_entity_attachments[0].0, pos);
        assert_eq!(extras.block_entity_attachments[0].1.get(&key), Ok(21));
    }

    #[test]
    fn unopened_rolled_containers_save_their_loot_table() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&root).expect("biomes");
        let tables = SaveTables::new(&assets);
        let chest = assets.registry.default_state("minecraft:chest").unwrap();
        let mut slots = vec![None; 27];
        slots[3] = Some(crate::items::ItemStack::new("minecraft:bone", 2));
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(1, 64, 2, chest.0);
        chunk.block_entities.push((
            (1, 64, 2),
            crate::containers::BlockEntity::Chest(slots.clone()),
        ));
        let rolled = RolledLoot {
            pos: (1, 64, 2),
            table: "minecraft:chests/simple_dungeon".into(),
            seed: Some(42),
            items: encode_items(&slots),
        };
        chunk.saved = Some(Arc::new(ChunkExtras {
            rolled_loot: vec![rolled],
            ..ChunkExtras::default()
        }));
        let saved_chest = |chunk: &Chunk| -> Tag {
            let saved = encode_with(
                chunk,
                assets.registry,
                &biomes,
                Some(&tables),
                LevelShape::OVERWORLD,
            );
            let Some(Tag::List(list)) = saved.tag.get("block_entities") else {
                panic!("no block entities");
            };
            list[0].clone()
        };
        let unopened = saved_chest(&chunk);
        assert_eq!(
            text(unopened.get("LootTable")).as_deref(),
            Some("minecraft:chests/simple_dungeon")
        );
        assert_eq!(number(unopened.get("LootTableSeed")), Some(42));
        assert!(unopened.get("Items").is_none());
        // Taking an item makes the items the container's own.
        if let crate::containers::BlockEntity::Chest(s) = &mut chunk.block_entities[0].1 {
            s[3] = None;
            s[4] = Some(crate::items::ItemStack::new("minecraft:bone", 1));
        }
        let opened = saved_chest(&chunk);
        assert!(opened.get("LootTable").is_none());
        assert!(opened.get("Items").is_some());
        // Loaded with its table, rolled, then emptied: no table, or it refills on load.
        let mut extras = ChunkExtras::clone(chunk.saved.as_ref().unwrap());
        extras.block_entity_sources = vec![(
            (1, 64, 2),
            Tag::compound([
                ("id", string("minecraft:chest")),
                ("x", Tag::Int(1)),
                ("y", Tag::Int(64)),
                ("z", Tag::Int(2)),
                ("LootTable", string("minecraft:chests/simple_dungeon")),
                ("LootTableSeed", Tag::Long(42)),
            ]),
        )];
        chunk.saved = Some(Arc::new(extras));
        if let crate::containers::BlockEntity::Chest(s) = &mut chunk.block_entities[0].1 {
            s.fill(None);
        }
        let emptied = saved_chest(&chunk);
        assert!(emptied.get("LootTable").is_none());
        assert!(emptied.get("LootTableSeed").is_none());
    }

    #[test]
    fn a_tall_level_saves_and_loads_its_whole_height() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&root).expect("biomes");
        let tables = SaveTables::new(&assets);
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        // To The Max's Overworld: -64..2032, 131 sections.
        let shape = LevelShape::new(-64, 2096, 2096, true).unwrap();
        let mut chunk = Chunk::with_layout(0, 0, shape.column());
        chunk.set(0, -64, 0, stone.0);
        chunk.set(0, 2000, 0, stone.0);
        let saved = encode_with(&chunk, assets.registry, &biomes, Some(&tables), shape);
        assert_eq!(number(saved.tag.get("yPos")), Some(-4));
        let Some(Tag::List(sections)) = saved.tag.get("sections") else {
            panic!("no sections");
        };
        assert_eq!(sections.len(), 131);
        // 12-bit heightmap entries: y 2000's top reads 2001 - -64.
        let Some(Tag::LongArray(surface)) = saved
            .tag
            .get("Heightmaps")
            .and_then(|h| h.get("WORLD_SURFACE"))
        else {
            panic!("no WORLD_SURFACE");
        };
        assert_eq!(surface[0] & 0xfff, 2065);
        let back = decode_with(
            &saved,
            assets.registry,
            &biomes,
            Some(&tables),
            shape.column(),
        )
        .expect("decodes");
        assert_eq!(back.sections.len(), 131);
        assert_eq!(back.get(0, 2000, 0), stone.0);
        assert_eq!(back.get(0, -64, 0), stone.0);
        // Read into the vanilla column, the sections above it are left out.
        let short = decode_with(
            &saved,
            assets.registry,
            &biomes,
            Some(&tables),
            ColumnLayout::VANILLA,
        )
        .expect("decodes");
        assert_eq!(short.sections.len(), SECTIONS);
        assert_eq!(short.get(0, 2000, 0), 0);
        assert_eq!(short.get(0, -64, 0), stone.0);
    }

    #[test]
    fn nether_and_end_chunks_save_with_their_own_height() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&root).expect("biomes");
        let tables = SaveTables::new(&assets);
        let rack = assets
            .registry
            .default_state("minecraft:netherrack")
            .unwrap();
        let mut chunk = Chunk::empty(0, 0);
        for y in 0..128 {
            chunk.set(0, y, 0, rack.0);
        }
        let saved = encode_with(
            &chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            LevelShape::NETHER,
        );
        assert_eq!(number(saved.tag.get("yPos")), Some(0));
        let Some(Tag::List(sections)) = saved.tag.get("sections") else {
            panic!("no sections");
        };
        let ys: Vec<i32> = sections
            .iter()
            .filter_map(|s| number(s.get("Y")).map(|y| y as i32))
            .collect();
        assert_eq!(ys, (0..16).collect::<Vec<_>>());
        let Some(Tag::List(post)) = saved.tag.get("PostProcessing") else {
            panic!("no PostProcessing");
        };
        assert_eq!(post.len(), 16);
        // Heightmap values count from the level's minimum: the column's top
        // block at y=127 reads 128 (vanilla), not 192 (from the Overworld's -64).
        let Some(Tag::LongArray(surface)) = saved
            .tag
            .get("Heightmaps")
            .and_then(|h| h.get("WORLD_SURFACE"))
        else {
            panic!("no WORLD_SURFACE");
        };
        assert_eq!(surface[0] & 0x1ff, 128);
        let overworld = encode_with(
            &chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            LevelShape::OVERWORLD,
        );
        assert_eq!(number(overworld.tag.get("yPos")), Some(-4));
    }

    /// The far ring's reads (`lan::far`): a chunk saved with its light, without the gameplay's region
    /// cache; none for a chunk not saved, or saved without its light.
    #[test]
    fn far_reads_take_saved_lit_chunks_only() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&root).expect("biomes");
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let mut lit = Chunk::empty(4, -7);
        lit.set(1, 70, 2, stone.0);
        let mut dark = lit.clone();
        dark.x = 5;
        lit.lit = true;
        let dir = std::env::temp_dir().join(format!("horizon-far-save-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let records =
            [&lit, &dark].map(|c| ((c.x, c.z), Some(encode(c, assets.registry, &biomes))));
        save.write_records("region", records.into(), true).unwrap();
        let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let far = reopened
            .load_far(4, -7, assets.registry, &biomes)
            .expect("saved with its light");
        assert_eq!(far.block(1, 70, 2), stone);
        assert!(far.lit && far.light_loaded);
        assert!(reopened.load_far(5, -7, assets.registry, &biomes).is_none());
        assert!(reopened.load_far(6, -7, assets.registry, &biomes).is_none());
        assert!(reopened.regions.lock().unwrap().is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saved_far_surface_reads_builds_and_water_without_gameplay_chunks() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&root).expect("biomes");
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let sand = assets.registry.default_state("minecraft:sand").unwrap();
        let dirt = assets.registry.default_state("minecraft:dirt").unwrap();
        let leaves = assets
            .registry
            .default_state("minecraft:oak_leaves")
            .unwrap();
        let end_stone = assets
            .registry
            .default_state("minecraft:end_stone")
            .unwrap();
        let chorus = assets
            .registry
            .default_state("minecraft:chorus_plant")
            .unwrap();
        let flower = assets
            .registry
            .default_state("minecraft:chorus_flower")
            .unwrap();
        let mut chunk = Chunk::empty(-2, 3);
        chunk.set(0, 109, 0, stone.0);
        chunk.set(3, 121, 5, stone.0);
        chunk.set(3, 122, 5, stone.0);
        chunk.set(3, 120, 5, dirt.0);
        chunk.set(3, 119, 5, dirt.0);
        chunk.set(3, 115, 5, stone.0);
        chunk.set(11, 125, 11, stone.0);
        chunk.set(14, 127, 14, stone.0);
        chunk.set(14, 112, 14, stone.0);
        for y in 95..98 {
            chunk.set(2, y, 6, stone.0);
        }
        chunk.lit = true;
        let mut cave_light = Box::new([0x03; 4096]);
        cave_light[crate::chunk::index(12, 125, 11)] = 0xa7;
        Arc::make_mut(&mut chunk.sections[((121 - MIN_Y) >> 4) as usize]).light =
            Light::Full(cave_light);
        chunk.set(7, 90, 9, sand.0);
        chunk.set(7, 98, 9, leaves.0);
        let forest = biomes.id("minecraft:forest");
        chunk.set_biome_cell(7 / 4, (98 - MIN_Y) / 4, 9 / 4, forest);
        chunk.set(4, 54, 4, sand.0);
        for y in 55..64 {
            chunk.set(4, y, 4, water.0);
        }
        // Chorus is thin and non-solid but still part of the far End.
        chunk.set(12, 100, 2, end_stone.0);
        chunk.set(12, 101, 2, chorus.0);
        chunk.set(12, 102, 2, flower.0);
        let mut saved = encode(&chunk, assets.registry, &biomes);
        put(
            &mut saved.tag,
            "block_entities",
            Tag::String("not decoded by far sampling".into()),
        );
        let dir = std::env::temp_dir().join(format!("horizon-lod-save-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        save.write_records("region", vec![((-2, 3), Some(saved))], true)
            .unwrap();
        let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let surfaces = reopened
            .lod_surfaces(-2, 3, &assets, &biomes)
            .expect("saved summary");
        assert_eq!(surfaces[0].surface.unwrap().height, 110);
        assert_eq!(surfaces[0].surface.unwrap().state, stone);
        let narrow_build = surfaces[5 * 16 + 3].surface.unwrap();
        assert_eq!((narrow_build.height, narrow_build.state), (123, stone));
        assert!(
            surfaces[5 * 16 + 2].surface.is_none(),
            "adjacent columns stay distinct"
        );
        let strata = &surfaces[5 * 16 + 3].spans;
        assert_eq!(
            strata
                .iter()
                .map(|span| (span.min, span.max, span.state))
                .collect::<Vec<_>>(),
            vec![(115, 116, stone), (119, 121, dirt), (121, 123, stone)]
        );
        assert!(
            strata.iter().all(|span| span.light == 0x3000),
            "saved cave darkness and block light survive"
        );
        assert_eq!(
            surfaces[11 * 16 + 11].spans[0].light,
            0x70a0,
            "neighbor face light uses separate sky/block channels"
        );
        let boundaries = &surfaces[14 * 16 + 14].spans;
        assert_eq!(boundaries.len(), 2);
        assert!(
            boundaries.iter().all(|span| span.light == 0x30f0),
            "face light crosses section boundaries above and below"
        );
        let across_sections = &surfaces[6 * 16 + 2].spans;
        assert_eq!(across_sections.len(), 1);
        assert_eq!((across_sections[0].min, across_sections[0].max), (95, 98));
        let canopy = surfaces[9 * 16 + 7].surface.unwrap();
        assert_eq!(
            (canopy.height, canopy.state, canopy.biome),
            (99, leaves, forest)
        );
        let underwater = surfaces[4 * 16 + 4].surface.unwrap();
        assert_eq!((underwater.height, underwater.state), (55, sand));
        assert_eq!(underwater.water, Some((64, water)));
        let chorus_column = &surfaces[2 * 16 + 12];
        assert_eq!(
            chorus_column
                .surface
                .map(|surface| (surface.height, surface.state)),
            Some((103, flower))
        );
        assert_eq!(
            chorus_column
                .spans
                .iter()
                .map(|span| (span.min, span.max, span.state))
                .collect::<Vec<_>>(),
            vec![
                (100, 101, end_stone),
                (101, 102, chorus),
                (102, 103, flower)
            ]
        );
        assert!(surfaces[15].surface.is_none(), "saved void stays empty");
        assert!(
            reopened.regions.lock().unwrap().is_empty(),
            "far reads do not grow gameplay region cache"
        );
        let mut proto = encode(&chunk, assets.registry, &biomes);
        put(
            &mut proto.tag,
            "Status",
            string(ChunkStatus::Terrain.name()),
        );
        put(&mut proto.tag, "xPos", Tag::Int(-3));
        save.write_records("region", vec![((-3, 3), Some(proto))], true)
            .unwrap();
        assert!(
            reopened.lod_surfaces(-3, 3, &assets, &biomes).is_none(),
            "unfinished proto saves must not suppress complete decoration"
        );
        let nether = SaveDir::new(dir.clone(), LevelShape::NETHER);
        let dark = nether.lod_surfaces(-2, 3, &assets, &biomes).unwrap();
        assert!(
            dark.iter()
                .flat_map(|c| &c.spans)
                .all(|s| s.light & 0xf0 == 0),
            "no invented skylight in Nether saves"
        );
        struct SavedOnly(Vec<crate::streaming::LodDetail>);
        impl crate::streaming::LodSampler for SavedOnly {
            fn sample(&mut self, _: i32, _: i32) -> Option<crate::lod::Surface> {
                panic!("saved data must win")
            }
            fn request_detail(&mut self, chunks: &[(i32, i32)], _: [i32; 2]) {
                assert!(
                    chunks.is_empty(),
                    "saved builds must not request speculative generation"
                );
            }
            fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
                std::mem::take(&mut self.0)
            }
        }
        let inner = Arc::new(crate::testgen::TestTerrain::new(assets.registry, &biomes).unwrap());
        let generator = Arc::new(Persistent {
            inner,
            save: Arc::new(reopened),
            registry: Arc::new(assets),
            biomes: Arc::new(biomes),
        });
        let mut sampler = SavedLodSampler {
            generator,
            inner: Box::new(SavedOnly(Vec::new())),
            cache: Default::default(),
            details: Vec::new(),
            approximate: false,
            generated: Arc::new(crate::lod::cache::DiskCache::new(dir.join("far"), 1)),
        };
        use crate::streaming::LodSampler;
        sampler.request_detail(&[(-2, 3)], [-32, 48]);
        let details = sampler.drain_details();
        assert_eq!(details.len(), 1);
        assert_eq!(&details[0].1[..], &surfaces[..]);
        sampler.cache.clear();
        let generated: LodSurfaces =
            Arc::new(std::array::from_fn(|_| crate::lod::Column::default()));
        sampler.inner = Box::new(SavedOnly(vec![
            ((-2, 3), generated.clone()),
            ((-9, 9), generated.clone()),
        ]));
        let details = sampler.drain_details();
        assert_eq!(
            &details[0].1[..],
            &surfaces[..],
            "unrequested batch neighbours must preserve saved builds"
        );
        assert!(Arc::ptr_eq(&details[1].1, &generated));
        assert!(
            sampler.generated.get(-2, 3).is_none(),
            "saved builds are not cached as generated"
        );
        assert_eq!(sampler.generated.get(-9, 9).unwrap(), generated);
        // A later session reads the generated chunk back instead of
        // generating it again.
        sampler.request_detail(&[(-9, 9)], [-144, 144]);
        assert_eq!(sampler.drain_details()[0].1, generated);
        sampler.inner =
            crate::streaming::ChunkGenerator::lod_sampler(sampler.generator.inner.clone());
        let grid = sampler.grid(-32, 48, 2, 16);
        for z in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    grid[z * 16 + x],
                    crate::lod::Column::reduced(&surfaces, x * 2, z * 2, 2),
                    "saved builds must override a batched direct grid"
                );
            }
        }
        drop((save, sampler, nether));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saved_far_surface_rejects_invalid_packed_palettes() {
        let mut states = Tag::compound([
            ("palette", Tag::List(vec![Tag::Byte(0), Tag::Byte(1)])),
            ("data", Tag::LongArray(vec![0; 256])),
        ]);
        assert_eq!(lod_palette_index(&states, 4095, 4096, 4), Some(0));
        assert_eq!(lod_palette_index(&states, 4096, 4096, 4), None);
        put(&mut states, "data", Tag::LongArray(vec![15; 256]));
        assert_eq!(lod_palette_index(&states, 0, 4096, 4), None);
        put(&mut states, "data", Tag::LongArray(vec![0; 255]));
        assert_eq!(lod_palette_index(&states, 0, 4096, 4), None);
    }

    #[test]
    fn save_batch_preserves_order_coordinates_and_ownership() {
        use crate::containers::BlockEntity;
        let mut entities = HashMap::new();
        let mut names = HashMap::new();
        for x in [i32::MIN, -17, -16, -1, 0, 15, 16, i32::MAX] {
            for z in [-17, -16, -1, 0, 15, 16] {
                let p = (x, 64, z);
                entities.insert(p, BlockEntity::Chest(vec![None; 27]));
                names.insert(p, format!("container {x},{z}"));
            }
        }
        // Names without a modelled entity must also survive the snapshot.
        names.insert((-2, 80, -3), "orphan name".into());
        let mut source = Chunk::empty(-1, -1);
        source
            .block_entities
            .push(((99, 0, 99), BlockEntity::Chest(vec![])));
        source.block_names.push(((99, 0, 99), "stale".into()));
        source.saved = Some(Arc::new(ChunkExtras::default()));
        let source = Arc::new(source);
        let batch = vec![
            source.clone(),
            Arc::new(Chunk::empty(1, 1)),
            Arc::new(Chunk::empty(-2, 0)),
            source.clone(), // Preserve input order and repeated coordinates.
            Arc::new(Chunk::empty(i32::MIN >> 4, -2)),
            Arc::new(Chunk::empty(i32::MAX >> 4, 1)),
            Arc::new(Chunk::empty(100, 100)),
        ];
        let encode = |entries: &[((i32, i32, i32), BlockEntity)]| {
            entries
                .iter()
                .map(|(p, e)| encode_block_entity(*p, e, "minecraft:chest"))
                .collect::<Vec<_>>()
        };
        for input in [&batch[..0], &batch[..1], &batch[..]] {
            let actual = chunks_with_block_entities(input, &entities, &names);
            assert_eq!(actual.len(), input.len());
            for (c, original) in actual.iter().zip(input) {
                let expected: Vec<_> = entities
                    .iter()
                    .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                    .map(|(p, e)| (*p, e.clone()))
                    .collect();
                let expected_names: Vec<_> = names
                    .iter()
                    .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                    .map(|(p, n)| (*p, n.clone()))
                    .collect();
                assert_eq!((c.x, c.z), (original.x, original.z));
                assert_eq!(encode(&c.block_entities), encode(&expected));
                assert_eq!(c.block_names, expected_names);
                assert!(Arc::ptr_eq(&c.sections[0], &original.sections[0]));
                if let Some(saved) = &original.saved {
                    assert!(Arc::ptr_eq(c.saved.as_ref().unwrap(), saved));
                }
            }
        }
        let snapshot = chunks_with_block_entities(&batch, &entities, &names);
        entities.clear();
        names.clear();
        assert!(!snapshot[0].block_entities.is_empty());
        assert!(!snapshot[0].block_names.is_empty());
        assert_eq!(source.block_names[0].1, "stale");
        assert_eq!(source.block_entities[0].0, (99, 0, 99));
    }

    #[test]
    fn work_shared_out_is_all_done_in_order_when_no_helper_can_start() {
        use super::share_out;
        let squares = |i: usize| i * i;
        let want: Vec<usize> = (0..40).map(squares).collect();
        // Every helper refused (a container at its thread limit): this thread does it all.
        assert_eq!(share_out(40, 4, squares, || false), want);
        // Some refused, some started.
        let mut asked = 0;
        let some = || {
            asked += 1;
            asked % 2 == 0
        };
        assert_eq!(share_out(40, 4, squares, some), want);
        assert_eq!(share_out(40, 4, squares, || true), want);
        assert_eq!(share_out(0, 4, squares, || true), Vec::<usize>::new());
    }

    #[test]
    fn records_compressed_on_helpers_match_one_thread_in_order() {
        use super::{Record, compress_records};
        let records: Vec<((i32, i32), Record)> = (0..53i32)
            .map(|i| {
                let record = if i % 7 == 3 {
                    Record::Delete
                } else {
                    let payload: Vec<i64> = (0..i * 97 % 2000).map(|v| i64::from(v * i)).collect();
                    Record::Tag(Arc::new(NamedTag {
                        name: "".into(),
                        tag: Tag::compound([
                            ("xPos", Tag::Int(i)),
                            ("Data", Tag::LongArray(payload)),
                        ]),
                    }))
                };
                ((i, -i), record)
            })
            .collect();
        let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
        let compressed = compress_records(&refs, 1234);
        assert_eq!(compressed.len(), records.len());
        for ((_, record), got) in records.iter().zip(compressed) {
            let want = record
                .tag()
                .map(|t| ChunkRecord::from_nbt(&t, Compression::Zlib, 1234, Limits::default()))
                .transpose()
                .unwrap();
            let got = got.unwrap();
            assert_eq!(
                got.as_ref().map(|r| (&r.compressed, r.timestamp)),
                want.as_ref().map(|r| (&r.compressed, r.timestamp))
            );
        }
    }

    #[test]
    fn older_worlds_are_upgraded_on_opening() {
        let dir = std::env::temp_dir().join(format!("horizon-dv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let level =
            |v: i32| Tag::compound([("Data", Tag::compound([("DataVersion", Tag::Int(v))]))]);
        let version = |dir: &std::path::Path| {
            super::read_dat(&dir.join("level.dat"))
                .and_then(|r| super::number(r.get("Data")?.get("DataVersion")))
        };
        // 1.12.2 and 1.16.5: upgraded too, every server fix is ported.
        for old in [1343, 2586] {
            super::write_dat(&dir.join("level.dat"), level(old)).unwrap();
            super::upgrade_older_world(&dir).unwrap();
            assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
        }
        // 1.21.1: upgraded to the current version.
        super::write_dat(&dir.join("level.dat"), level(3955)).unwrap();
        super::upgrade_older_world(&dir).unwrap();
        assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
        super::upgrade_older_world(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn region_files_kept_open_stay_under_vanillas_cache_size() {
        let open_fds = || std::fs::read_dir("/dev/fd").map_or(0, |d| d.count());
        let dir = std::env::temp_dir().join(format!("horizon-save-fds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let before = open_fds();
        let held = super::cached_region(
            &save.regions,
            (0, 0),
            &save.region_path("region", 0, 0),
            true,
        )
        .unwrap()
        .unwrap();
        for x in 1..=600 {
            super::cached_region(
                &save.regions,
                (x, 0),
                &save.region_path("region", x, 0),
                true,
            )
            .unwrap()
            .unwrap();
        }
        // Other tests open files at the same time: a margin, far below 600.
        let grown = open_fds().saturating_sub(before);
        assert!(
            grown <= super::MAX_OPEN_REGIONS + 32,
            "{grown} descriptors more"
        );
        let cache = save.regions.lock().unwrap();
        assert!(
            cache.open() <= super::MAX_OPEN_REGIONS,
            "{} open",
            cache.open()
        );
        // The one still held is never closed under its holder.
        assert!(Arc::ptr_eq(
            &cache.regions[&(0, 0)].0.clone().unwrap(),
            &held
        ));
        drop(cache);
        // What was written to regions since closed reads back from the disk.
        let entity = |x: i32| Tag::compound([("x", Tag::Int(x))]);
        save.save_entities_later((0..300).map(|i| ((i * 32, 0), vec![entity(i)])).collect())
            .unwrap();
        save.flush().unwrap();
        assert!(save.entity_regions.lock().unwrap().open() <= super::MAX_OPEN_REGIONS);
        for i in [0, 1, 150, 299] {
            assert_eq!(save.load_entities(i * 32, 0), vec![entity(i)]);
        }
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn write_completion_polls_without_waiting_and_keeps_latest_snapshot() {
        let dir = std::env::temp_dir().join(format!("horizon-save-poll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let lock = super::SessionLock::acquire(&dir).unwrap();
        let path = save.region_path("entities", 0, 0);
        let region = super::cached_region(&save.entity_regions, (0, 0), &path, true)
            .unwrap()
            .unwrap();
        // Hold the actual region lock, rather than relying on disk speed or sleeps.
        let blocked = region.lock().unwrap();
        let entity = |version| Tag::compound([("version", Tag::Int(version))]);
        save.save_entities_later(vec![((1, 2), vec![entity(1)])])
            .unwrap();
        save.save_entities_later(vec![((1, 2), vec![entity(2)])])
            .unwrap();
        assert!(!save.writes_finished().unwrap());
        assert_eq!(save.load_entities(1, 2), vec![entity(2)]);
        assert!(super::SessionLock::acquire(&dir).is_err());
        drop(blocked);
        save.flush().unwrap();
        assert!(save.writes_finished().unwrap());
        let reopened = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        assert_eq!(reopened.load_entities(1, 2), vec![entity(2)]);
        save.save_entities_later(vec![((1, 2), vec![])]).unwrap();
        save.flush().unwrap();
        assert!(
            super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD)
                .load_entities(1, 2)
                .is_empty()
        );
        drop((reopened, save, lock));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_async_writes_are_readable_and_never_reported_durable() {
        let dir = std::env::temp_dir().join(format!("horizon-save-failure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A file where a directory is needed causes a real writer I/O error.
        std::fs::write(dir.join("entities"), b"blocked").unwrap();
        let save = super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        let entity = Tag::compound([("version", Tag::Int(3))]);
        save.save_entities_later(vec![((0, 0), vec![entity.clone()])])
            .unwrap();
        assert!(save.flush().is_err());
        assert!(save.writes_finished().is_err());
        assert_eq!(save.load_entities(0, 0), vec![entity]);
        // The failure stays visible on subsequent barriers too.
        assert!(save.flush().is_err());
        drop(save);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_lock_is_exclusive() {
        let dir = std::env::temp_dir().join(format!("horizon-lock-{}", std::process::id()));
        let first = super::SessionLock::acquire(&dir).expect("first lock");
        assert!(super::SessionLock::acquire(&dir).is_err());
        drop(first);
        // Windows refuses a read through another handle while the file is
        // locked, so the bytes are read once the lock is released.
        assert_eq!(
            std::fs::read(dir.join("session.lock")).unwrap(),
            [0x26, 0x03]
        );
        assert!(super::SessionLock::acquire(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// #749: a copy of the lock's descriptor (a child process spawned on
    /// another thread holds one until it execs) must not keep a released
    /// world locked.
    #[test]
    fn session_lock_releases_while_a_descriptor_copy_lives() {
        let dir = std::env::temp_dir().join(format!("horizon-lock-copy-{}", std::process::id()));
        let lock = super::SessionLock::acquire(&dir).expect("first lock");
        let copy = lock.0.try_clone().unwrap();
        drop(lock);
        assert!(super::SessionLock::acquire(&dir).is_ok());
        drop(copy);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn crafter_block_entity_round_trips() {
        let mut crafter = crate::containers::Crafter::default();
        crafter.items[2] = Some(crate::items::ItemStack::new("minecraft:stick", 5));
        crafter.disabled[4] = true;
        crafter.triggered = true;
        crafter.crafting_ticks = 3;
        let tag = encode_block_entity(
            (7, 8, 9),
            &crate::containers::BlockEntity::Crafter(crafter),
            "minecraft:crafter",
        );
        let (pos, back) = decode_block_entity(&tag).expect("crafter");
        assert_eq!(pos, (7, 8, 9));
        let crate::containers::BlockEntity::Crafter(back) = back else {
            panic!("not a crafter");
        };
        assert_eq!(back.items[2].as_ref().map(|s| s.count), Some(5));
        assert!(back.disabled[4] && !back.disabled[3]);
        assert!(back.triggered);
        assert_eq!(back.crafting_ticks, 3);
        assert_eq!(back.redstone_signal(), 2);
    }
    #[test]
    fn campfire_block_entity_round_trips() {
        let mut c = crate::containers::Campfire::default();
        c.items[1] = Some(crate::items::ItemStack::new("minecraft:beef", 1));
        c.progress[1] = 120;
        c.total[1] = 600;
        let tag = encode_block_entity(
            (3, 64, -2),
            &crate::containers::BlockEntity::Campfire(Box::new(c.clone())),
            "minecraft:soul_campfire",
        );
        // One block entity type for both campfires.
        assert_eq!(text(tag.get("id")).as_deref(), Some("minecraft:campfire"));
        let (pos, back) = decode_block_entity(&tag).expect("campfire");
        assert_eq!(pos, (3, 64, -2));
        let crate::containers::BlockEntity::Campfire(back) = back else {
            panic!("not a campfire");
        };
        assert_eq!(*back, c);
        // The update tag a server's clients get: the items alone.
        let update =
            crate::lan::chunks::entity_tag(&crate::containers::BlockEntity::Campfire(Box::new(c)))
                .expect("update tag");
        assert!(update.get("Items").is_some() && update.get("CookingTimes").is_none());
    }
    /// `LockCode`: a chest's `lock` is not in Horizon's model, so `data get`
    /// takes it from the tag the block entity was loaded (or set) with.
    #[test]
    fn a_locked_chest_keeps_its_lock_through_decode_and_encode() {
        let saved = horizon_world::nbt::parse_snbt(
            r#"{id:"minecraft:chest",x:1,y:2,z:3,lock:{items:"minecraft:gold_ingot"}}"#,
        )
        .unwrap()
        .0;
        let (pos, chest) = decode_block_entity(&saved).expect("chest");
        assert_eq!(chest.slots().len(), 27);
        assert_eq!(pos, (1, 2, 3));
        // The encoded entity has no field to carry the lock...
        let plain = encode_block_entity(pos, &chest, "minecraft:chest");
        assert!(plain.get("lock").is_none());
        // ...so `data get` puts back the one it was loaded with.
        let data = block_entity_data(pos, &chest, "minecraft:chest", saved.get("lock"));
        let lock = horizon_world::nbt::parse_snbt(r#"{items:"minecraft:gold_ingot"}"#)
            .unwrap()
            .0;
        assert_eq!(data.get("lock"), Some(&lock));
    }
    #[test]
    fn spawner_block_entity_round_trips() {
        // A dungeon spawner as vanilla saves it (`id` is mob_spawner, not the block).
        let saved = horizon_world::nbt::parse_snbt(
            r#"{id:"minecraft:mob_spawner",x:67,y:-52,z:-64,Delay:20s,SpawnData:{entity:{id:"minecraft:zombie"}},SpawnPotentials:[]}"#,
        )
        .unwrap()
        .0;
        let (pos, back) = decode_block_entity(&saved).expect("spawner");
        assert_eq!(pos, (67, -52, -64));
        let crate::containers::BlockEntity::Spawner(spawner) = &back else {
            panic!("not a spawner");
        };
        let next = spawner.next_data().and_then(|d| d.id());
        assert_eq!(next.as_deref(), Some("minecraft:zombie"));
        let tag = encode_block_entity(pos, &back, "minecraft:spawner");
        let (_, again) = decode_block_entity(&tag).expect("spawner again");
        let crate::containers::BlockEntity::Spawner(again) = again else {
            panic!("not a spawner");
        };
        assert_eq!(again.update_tag(), spawner.update_tag());
    }
    #[test]
    fn mod_block_entity_round_trips() {
        let saved = horizon_world::nbt::parse_snbt(
            r#"{id:"gtmod:macerator",x:5,y:-60,z:12,energy:1200L,progress:7,Items:[{Slot:0b,id:"minecraft:iron_ore",count:3}]}"#,
        )
        .unwrap()
        .0;
        let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
        assert_eq!(pos, (5, -60, 12));
        let crate::containers::BlockEntity::Mod(m) = &back else {
            panic!("not a mod's");
        };
        assert_eq!(m.id, "gtmod:macerator");
        // The mod sees its own fields only.
        for key in ["id", "x", "y", "z"] {
            assert!(!m.nbt.contains_key(&key.into()), "{key}");
        }
        assert_eq!(m.nbt.get(&"energy".into()), Some(&Tag::Long(1200)));
        // `Items` are its slots, not the mod's compound.
        assert!(!m.nbt.contains_key(&"Items".into()));
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.items[0].as_ref().map(|s| s.count), Some(3));
        assert_eq!(encode_block_entity(pos, &back, "gtmod:macerator"), saved);
        // `Items` that are not an item list stay the mod's, as saved.
        for items in [
            r#"Items:{energy:5}"#,
            r#"Items:[1,2,3]"#,
            r#"Items:[{Slot:0b,id:"minecraft:air",count:1}]"#,
            r#"Items:[{Slot:0b,id:"minecraft:stone",count:1,note:"mine"}]"#,
        ] {
            let saved = horizon_world::nbt::parse_snbt(&format!(
                r#"{{id:"gtmod:cable",x:0,y:0,z:0,{items}}}"#
            ))
            .unwrap()
            .0;
            let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
            let crate::containers::BlockEntity::Mod(m) = &back else {
                panic!("not a mod's");
            };
            assert!(m.items.is_empty(), "{items}");
            assert_eq!(
                encode_block_entity(pos, &back, "gtmod:cable"),
                saved,
                "{items}"
            );
        }
        // Vanilla's types are never a mod's, modelled or not.
        let ender = horizon_world::nbt::parse_snbt(r#"{id:"minecraft:ender_chest",x:0,y:0,z:0}"#)
            .unwrap()
            .0;
        assert!(!matches!(
            decode_block_entity(&ender),
            Some((_, crate::containers::BlockEntity::Mod(_)))
        ));
    }
    #[test]
    fn banner_block_entity_round_trips() {
        let banner = crate::containers::Banner {
            layers: vec![crate::banner::BannerLayer::new("minecraft:creeper", "lime")],
            name: Some("Flag".into()),
            components: [("minecraft:rarity".into(), string("uncommon"))].into(),
        };
        let tag = encode_block_entity(
            (1, 2, 3),
            &crate::containers::BlockEntity::Banner(banner.clone()),
            "minecraft:red_banner",
        );
        let (pos, back) = decode_block_entity(&tag).expect("banner");
        assert_eq!(pos, (1, 2, 3));
        let crate::containers::BlockEntity::Banner(back) = back else {
            panic!("not a banner");
        };
        assert_eq!(back.layers, banner.layers);
        assert_eq!(back.name, banner.name);
        assert_eq!(back.components, banner.components);
    }
    use super::*;
    use crate::chunk::{MIN_Y, SECTIONS};

    #[test]
    fn end_gateway_block_entity_round_trips() {
        use crate::containers::{BlockEntity, EndGateway};
        let plain = BlockEntity::for_block("minecraft:end_gateway").expect("gateway");
        assert!(matches!(&plain, BlockEntity::EndGateway(g) if *g == EndGateway::default()));
        for exit in [None, Some([8, 100, 0])] {
            for exact in [false, true] {
                let gateway = EndGateway {
                    age: 201,
                    exit_portal: exit,
                    exact_teleport: exact,
                };
                let tag = encode_block_entity(
                    (702, 100, 0),
                    &BlockEntity::EndGateway(gateway.clone()),
                    "minecraft:end_gateway",
                );
                assert_eq!(
                    text(tag.get("id")).as_deref(),
                    Some("minecraft:end_gateway")
                );
                assert_eq!(tag.get("Age"), Some(&Tag::Long(201)));
                assert_eq!(
                    tag.get("exit_portal"),
                    exit.map(|p| Tag::IntArray(p.to_vec())).as_ref()
                );
                assert_eq!(
                    tag.get("ExactTeleport"),
                    exact.then_some(Tag::Byte(1)).as_ref()
                );
                let (pos, back) = decode_block_entity(&tag).expect("gateway");
                assert_eq!(pos, (702, 100, 0));
                assert!(matches!(back, BlockEntity::EndGateway(g) if g == gateway));
            }
        }
    }

    #[test]
    fn end_gateway_exit_requires_spawnable_bounds() {
        for (exit, valid) in [
            (vec![-30_000_000, -20_000_000, -30_000_000], true),
            (vec![29_999_999, 19_999_999, 29_999_999], true),
            (vec![30_000_000, 0, 0], false),
            (vec![0, 0, 30_000_000], false),
            (vec![-30_000_001, 0, 0], false),
            (vec![0, 0, -30_000_001], false),
            (vec![0, 20_000_000, 0], false),
            (vec![0, -20_000_001, 0], false),
            (vec![i32::MIN, 0, 0], false),
            (vec![0, 100], false),
        ] {
            let tag = Tag::compound([
                ("id", string("minecraft:end_gateway")),
                ("x", Tag::Int(702)),
                ("y", Tag::Int(100)),
                ("z", Tag::Int(0)),
                ("exit_portal", Tag::IntArray(exit)),
            ]);
            let (_, crate::containers::BlockEntity::EndGateway(g)) =
                decode_block_entity(&tag).unwrap()
            else {
                panic!("gateway");
            };
            assert_eq!(g.exit_portal.is_some(), valid);
            let canonical = crate::block_entity_nbt::canonical(&tag).expect("gateway NBT");
            assert_eq!(canonical.get("exit_portal").is_some(), valid);
        }
        assert_eq!(
            decode_end_gateway(&Tag::Compound(Compound::new())),
            crate::containers::EndGateway::default()
        );
    }

    #[test]
    fn skull_block_entity_keeps_its_profile() {
        // A placed `player_head` as vanilla saves it: the owning player's
        // `profile` survives load, encode and `canonical`.
        let saved = horizon_world::nbt::parse_snbt(
            r#"{id:"minecraft:skull",x:10,y:64,z:20,profile:{name:"Steve",id:[I;1,2,3,4],properties:[{name:"textures",value:"abc"}]},note_block_sound:"minecraft:entity.player.hurt",custom_name:"Trophy"}"#,
        )
        .unwrap()
        .0;
        let (pos, back) = decode_block_entity(&saved).expect("skull");
        assert_eq!(pos, (10, 64, 20));
        let crate::containers::BlockEntity::Skull(skull) = &back else {
            panic!("not a skull");
        };
        assert_eq!(skull.profile, saved.get("profile").cloned());
        assert_eq!(
            skull.note_block_sound.as_deref(),
            Some("minecraft:entity.player.hurt")
        );
        assert_eq!(back.id(), "minecraft:skull");
        let tag = encode_block_entity(pos, &back, "minecraft:player_head");
        assert_eq!(
            crate::block_entity_nbt::id(&tag).as_deref(),
            Some("minecraft:skull")
        );
        assert_eq!(tag.get("profile"), saved.get("profile"));
        let (_, again) = decode_block_entity(&tag).expect("skull again");
        let crate::containers::BlockEntity::Skull(again) = again else {
            panic!("not a skull");
        };
        assert_eq!(again.profile, skull.profile);
        // A profile-less head (skeleton, creeper) round-trips too.
        let plain = encode_block_entity(
            pos,
            &crate::containers::BlockEntity::Skull(crate::containers::Skull::default()),
            "minecraft:creeper_head",
        );
        let (_, decoded) = decode_block_entity(&plain).expect("plain skull");
        assert!(matches!(decoded, crate::containers::BlockEntity::Skull(_)));
        // The heads and skulls have one; the piston's head is no skull.
        use crate::containers::BlockEntity;
        assert!(matches!(
            BlockEntity::for_block("minecraft:player_wall_head"),
            Some(BlockEntity::Skull(_))
        ));
        assert!(BlockEntity::for_block("minecraft:piston_head").is_none());
    }

    #[test]
    fn uuid_text_round_trips() {
        let u = [-158327916, 1572358581, -2125825222, -487948715];
        assert_eq!(uuid_from_text(&uuid_text(u)), Some(u));
        assert_eq!(
            uuid_text([0, 1, 2, 3]),
            "00000000-0000-0001-0000-000200000003"
        );
        let r = random_uuid();
        assert_eq!((r[1] >> 12) & 0xF, 4, "version 4");
    }
    #[test]
    fn random_uuids_made_at_once_differ() {
        let made: std::collections::HashSet<[i32; 4]> =
            (0..10_000).map(|_| random_uuid()).collect();
        assert_eq!(made.len(), 10_000);
    }
    #[test]
    fn creaking_heart_keeps_its_protector_uuid() {
        use crate::containers::{BlockEntity, CreakingHeart};
        let heart = CreakingHeart {
            creaking: Some((Some(7), [1, 2, 3, 4])),
            ..Default::default()
        };
        let tag = encode_block_entity(
            (3, 70, -5),
            &BlockEntity::CreakingHeart(Box::new(heart)),
            "minecraft:creaking_heart",
        );
        assert_eq!(
            tag.get("creaking"),
            Some(&Tag::IntArray(vec![1, 2, 3, 4])),
            "vanilla stores the protector's UUID under `creaking`"
        );
        let (pos, entity) = decode_block_entity(&tag).expect("heart decodes");
        assert_eq!(pos, (3, 70, -5));
        let BlockEntity::CreakingHeart(h) = entity else {
            panic!("a creaking heart");
        };
        // The entity id is resolved again once the creaking is loaded.
        assert_eq!(h.creaking, Some((None, [1, 2, 3, 4])));
        let lonely = encode_block_entity(
            (0, 0, 0),
            &BlockEntity::CreakingHeart(Box::default()),
            "minecraft:creaking_heart",
        );
        assert!(lonely.get("creaking").is_none());
    }
    #[test]
    fn light_nibbles_use_vanilla_order() {
        let mut a = Box::new([0u8; 4096]);
        a[0] = 0xF3; // sky 15, block 3 at x=0
        a[1] = 0x52; // sky 5, block 2 at x=1
        let light = Light::Full(a);
        assert_eq!(nibbles(&light, false)[0], 0x23);
        assert_eq!(nibbles(&light, true)[0], 0x5F);
    }
    #[test]
    fn stored_light_loads_with_vanilla_defaults() {
        let mut sky = vec![0u8; 2048];
        sky[0] = 0x5A; // x=0 -> 10, x=1 -> 5
        let section = |y: i8, entries: Vec<(&str, Tag)>| {
            let mut all = vec![("Y", Tag::Byte(y))];
            all.extend(entries);
            Tag::compound(all)
        };
        let tag = Tag::compound([
            ("isLightOn", Tag::Byte(1)),
            (
                "sections",
                Tag::List(vec![
                    section(
                        -4,
                        vec![
                            ("SkyLight", Tag::ByteArray(sky)),
                            ("BlockLight", Tag::ByteArray(vec![0x21; 2048])),
                        ],
                    ),
                    // No sky layer: repeats the bottom layer of the section above.
                    section(-3, vec![]),
                ]),
            ),
        ]);
        let mut chunk = Chunk::empty(0, 0);
        load_light(&tag, &mut chunk, true);
        assert!(chunk.lit && chunk.light_loaded);
        let at = |c: &Chunk, si: usize, i: usize| c.sections[si].light(i);
        assert_eq!(at(&chunk, 0, 0), 0xA1, "stored sky 10, block 1");
        assert_eq!(at(&chunk, 0, 1), 0x52, "stored sky 5, block 2");
        // Section -3 has no layers; above it (-2..) sky defaults to 15.
        assert_eq!(at(&chunk, 1, 0), 0xF0);
        assert_eq!(at(&chunk, SECTIONS - 1, 5), 0xF0);
        let mut dark = Chunk::empty(0, 0);
        load_light(&tag, &mut dark, false);
        assert_eq!(at(&dark, 1, 0), 0x00, "no skylight dimension");
        let mut unlit = Chunk::empty(0, 0);
        load_light(
            &Tag::compound([("isLightOn", Tag::Byte(0))]),
            &mut unlit,
            true,
        );
        assert!(!unlit.lit && !unlit.light_loaded);
    }
    #[test]
    fn saved_light_matches_reference_for_layers_and_metadata() {
        fn reference(tag: &Tag, chunk: &mut Chunk, sky_light: bool) {
            if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
                return;
            }
            let Some(Tag::List(sections)) = tag.get("sections") else {
                return;
            };
            let mut layers: Vec<(Option<&[u8]>, Option<&[u8]>)> = vec![(None, None); SECTIONS];
            for section in sections {
                let Some(y) = number(section.get("Y")) else {
                    continue;
                };
                let si = y as i32 - (MIN_Y >> 4);
                if !(0..SECTIONS as i32).contains(&si) {
                    continue;
                }
                let layer = |name: &str| match section.get(name) {
                    Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
                    _ => None,
                };
                layers[si as usize] = (layer("SkyLight"), layer("BlockLight"));
            }
            let above = [if sky_light { 15 } else { 0 }; 256];
            crate::light::light_from_layers(chunk, &layers, above, sky_light);
        }

        let section = |y: Tag, sky: Option<Tag>, block: Option<Tag>| {
            let mut entries = vec![("Y", y)];
            if let Some(v) = sky {
                entries.push(("SkyLight", v));
            }
            if let Some(v) = block {
                entries.push(("BlockLight", v));
            }
            Tag::compound(entries)
        };
        let root = |sections| {
            Tag::compound([
                ("isLightOn", Tag::Byte(1)),
                ("sections", Tag::List(sections)),
            ])
        };
        let check = |tag: &Tag| {
            for sky_light in [false, true] {
                let mut expected = Chunk::empty(-3, 5);
                let s = Arc::make_mut(&mut expected.sections[0]);
                s.light = Light::Full(Box::new([0xab; 4096]));
                s.blocks.set(0, 1);
                s.blocks.set(0, 0);
                s.biomes.set(0, 1);
                s.biomes.set(0, 0);
                let mut actual = expected.clone();
                reference(tag, &mut expected, sky_light);
                load_light(tag, &mut actual, sky_light);
                assert_eq!(
                    (actual.lit, actual.light_loaded),
                    (expected.lit, expected.light_loaded)
                );
                for (a, b) in actual.sections.iter().zip(&expected.sections) {
                    assert_eq!(
                        std::mem::discriminant(&a.light),
                        std::mem::discriminant(&b.light)
                    );
                    assert_eq!(
                        format!("{:?}", a.blocks.to_wire()),
                        format!("{:?}", b.blocks.to_wire())
                    );
                    assert_eq!(
                        format!("{:?}", a.biomes.to_wire()),
                        format!("{:?}", b.biomes.to_wire())
                    );
                    for i in 0..4096 {
                        assert_eq!(a.light(i), b.light(i), "index {i}, sky {sky_light}");
                    }
                }
            }
        };
        for tag in [
            Tag::compound(Vec::<(&str, Tag)>::new()),
            Tag::compound([("isLightOn", Tag::Byte(0))]),
            Tag::compound([("isLightOn", Tag::Byte(2))]),
            Tag::compound([("isLightOn", Tag::Int(1)), ("sections", Tag::List(vec![]))]),
            Tag::compound([("isLightOn", Tag::Byte(1)), ("sections", Tag::Int(0))]),
            root(vec![]),
        ] {
            check(&tag);
        }
        // Cover every repeated byte, including 0xF0/0x0F: a uniform byte
        // array need not hold uniform nibbles.
        for byte in 0..=255u8 {
            check(&root(vec![section(
                Tag::Byte(19),
                Some(Tag::ByteArray(vec![byte; 2048])),
                Some(Tag::ByteArray(vec![255 - byte; 2048])),
            )]));
        }
        // Every pair of uniform sky/block nibble values, with missing
        // sections below inheriting the stored bottom row.
        for sky in 0..16u8 {
            for block in 0..16u8 {
                check(&root(vec![section(
                    Tag::Byte(1),
                    Some(Tag::ByteArray(vec![sky * 17; 2048])),
                    Some(Tag::ByteArray(vec![block * 17; 2048])),
                )]));
            }
        }
        for length in [0, 1, 2047, 2048, 2049] {
            for invalid in [
                Tag::Int(1),
                Tag::List(vec![]),
                Tag::ByteArray(vec![0xf0; length]),
            ] {
                check(&root(vec![section(
                    Tag::Byte(-4),
                    Some(invalid.clone()),
                    Some(invalid),
                )]));
            }
        }
        let varied: Vec<u8> = (0..2048)
            .map(|i| (i as u8).wrapping_mul(73).wrapping_add(31))
            .collect();
        check(&root(vec![
            section(Tag::Byte(-5), Some(Tag::ByteArray(varied.clone())), None),
            section(Tag::Byte(20), Some(Tag::ByteArray(varied.clone())), None),
            section(
                Tag::String("19".into()),
                Some(Tag::ByteArray(varied.clone())),
                None,
            ),
            section(Tag::Byte(19), Some(Tag::ByteArray(varied.clone())), None),
            section(Tag::Byte(17), None, Some(Tag::ByteArray(varied.clone()))),
            section(Tag::Byte(-4), Some(Tag::ByteArray(varied.clone())), None),
            // Last duplicate wins, even when its arrays have wrong types.
            section(
                Tag::Byte(-4),
                Some(Tag::Int(15)),
                Some(Tag::ByteArray(vec![7; 2047])),
            ),
            section(Tag::Byte(0), None, Some(Tag::ByteArray(varied))),
        ]));
    }
    /// #2571: the difficulty is a property of the world: the level data
    /// holds it, so a world that was set to hard is hard when it is opened
    /// again, and a world that was downloaded keeps the difficulty it was
    /// made with.
    #[test]
    fn the_level_data_holds_the_difficulty() {
        let dir = std::env::temp_dir().join(format!("horizon-difficulty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut level = Level {
            name: "Test".into(),
            ..Level::default()
        };
        // The world's difficulty is the level data's.
        level.set_difficulty(3);
        assert_eq!(level.difficulty, "hard");
        assert_eq!(level.difficulty_id(), 3);
        // It survives the save, so the world comes back with it.
        level.write(&dir).unwrap();
        let back = Level::read(&dir).unwrap();
        assert_eq!(back.difficulty, "hard");
        assert_eq!(back.difficulty_id(), 3);
        // A world whose difficulty is not one of ours is normal.
        let other = Level {
            difficulty: "extreme".into(),
            ..Level::default()
        };
        assert_eq!(other.difficulty_id(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Dedicated world saves must not overwrite separately managed player data.
    #[test]
    fn dedicated_world_save_preserves_player_file() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-dedicated-player-save-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);

        let mut level = Level {
            name: "Test".into(),
            position: Some([12.5, 70.0, -8.25]),
            inventory: vec![(0, "minecraft:diamond".into(), 7, 0)],
            ..Level::default()
        };

        // Create a real player file using the existing singleplayer writer.
        level.write(&dir).unwrap();
        let uuid = level.player_uuid.unwrap();
        let player_file = dir
            .join("players")
            .join("data")
            .join(format!("{}.dat", uuid_text(uuid)));

        let before = fs::read(&player_file).unwrap();

        // Simulate an incomplete world-level player state.
        level.position = None;
        level.inventory.clear();
        level.day_time = 12345;

        // Dedicated saves must update metadata without touching player data.
        level.write_without_player(&dir).unwrap();

        let after = fs::read(&player_file).unwrap();
        assert_eq!(before, after, "dedicated save overwrote player data");

        // The world metadata must still be persisted.
        let reopened = Level::read(&dir).unwrap();
        assert_eq!(reopened.day_time, 12345);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn level_round_trips_through_the_vanilla_files() {
        let dir = std::env::temp_dir().join(format!("horizon-level-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut level = Level {
            name: "Test".into(),
            seed: -42,
            time: 1234,
            day_time: 25000,
            creative: true,
            position: Some([1.5, 70.0, -3.25]),
            rotation: [90.0, 10.0],
            health: 17.0,
            absorption: 5.0,
            food: 15,
            saturation: 2.5,
            inventory: vec![
                (0, "minecraft:stone".into(), 12, 0),
                (5, "minecraft:iron_sword".into(), 1, 3),
                (103, "minecraft:iron_helmet".into(), 1, 0),
            ],
            xp_level: 3,
            xp_progress: 0.5,
            xp_total: 30,
            dimension: "minecraft:the_nether".into(),
            world_spawn: Some([8, 64, 8]),
            world_spawn_angle: [45.0, -10.0],
            spawn: Some([20.0, 70.0, 20.0]),
            difficulty_locked: true,
            flying: true,
            ..Level::default()
        };
        level.weather.raining = true;
        level.weather.rain_time = 500;
        level.game_rules.insert("keep_inventory".into(), 1);
        level.write(&dir).unwrap();
        let back = Level::read(&dir).unwrap();
        assert_eq!(back.name, "Test");
        assert_eq!(back.seed, -42);
        assert_eq!((back.time, back.day_time), (1234, 25000));
        assert!(back.creative);
        assert!(back.flying);
        assert_eq!(back.position, Some([1.5, 70.0, -3.25]));
        assert_eq!(back.rotation, [90.0, 10.0]);
        assert_eq!((back.health, back.food, back.saturation), (17.0, 15, 2.5));
        assert_eq!(back.absorption, 5.0);
        let mut inv = back.inventory.clone();
        inv.sort();
        assert_eq!(
            inv,
            vec![
                (0, "minecraft:stone".into(), 12, 0),
                (5, "minecraft:iron_sword".into(), 1, 3),
                (103, "minecraft:iron_helmet".into(), 1, 0)
            ]
        );
        assert_eq!(
            (back.xp_level, back.xp_progress, back.xp_total),
            (3, 0.5, 30)
        );
        assert_eq!(back.dimension, "minecraft:the_nether");
        assert_eq!(back.world_spawn, Some([8, 64, 8]));
        assert_eq!(back.world_spawn_angle, [45.0, -10.0]);
        assert_eq!(back.spawn, Some([20.0, 70.0, 20.0]));
        assert!(back.difficulty_locked);
        assert!(back.weather.raining);
        assert_eq!(back.weather.rain_time, 500);
        assert_eq!(back.game_rule("keep_inventory"), 1);
        assert_eq!(back.game_rule("random_tick_speed"), 3);
        // Unknown player data survives another save.
        let mut extra = back.player_extra.clone().unwrap();
        put(
            &mut extra,
            "ShoulderEntityLeft",
            Tag::compound([("id", string("minecraft:parrot"))]),
        );
        let mut ender = vec![None; 27];
        ender[3] = Some(crate::items::ItemStack::new("minecraft:diamond", 2));
        let mut again = Level {
            player_extra: Some(extra),
            ender_items: ender,
            // The live amount, not the one the loaded compound held (#1307).
            absorption: 2.0,
            ..back
        };
        again.write(&dir).unwrap();
        let third = Level::read(&dir).unwrap();
        assert_eq!(third.absorption, 2.0);
        assert!(matches!(
            third
                .player_extra
                .as_ref()
                .and_then(|p| p.get("ShoulderEntityLeft")),
            Some(Tag::Compound(_))
        ));
        // EnderItems round-trip by slot.
        assert_eq!(
            third.ender_items[3].as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:diamond", 2))
        );
        // A dead player's empty inventory replaces the previously loaded items.
        let mut dead = third;
        dead.health = 0.0;
        dead.inventory.clear();
        dead.inventory_components.clear();
        dead.write(&dir).unwrap();
        let dead = Level::read(&dir).unwrap();
        assert_eq!(dead.health, 0.0);
        assert!(dead.inventory.is_empty());
        assert!(dead.inventory_components.is_empty());
        assert!(dead.player_extra.unwrap().get("equipment").is_none());
        assert!(dir.join("data/minecraft/world_gen_settings.dat").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn effects_that_ended_are_not_saved_again() {
        // A player loaded with an effect that has since run out: the loaded
        // compound still holds it, but the save writes only what is active.
        let mut level = Level {
            active_effects: vec![("minecraft:speed".into(), 200, 1, false, true)],
            ..Level::default()
        };
        let loaded = level.player_tag();
        assert!(loaded.get("active_effects").is_some());
        level.player_extra = Some(loaded);
        level.active_effects.clear();
        assert!(level.player_tag().get("active_effects").is_none());
    }

    #[test]
    fn creative_flight_status_persists_in_save_and_resets_for_survival() {
        let mut level = Level {
            creative: true,
            flying: true,
            ..Level::default()
        };
        let tag = level.player_tag();
        let flying_byte = tag
            .get("abilities")
            .and_then(|a| number(a.get("flying")))
            .unwrap_or(0);
        assert_eq!(flying_byte, 1);

        let mut loaded = Level::default();
        loaded.read_player(&tag);
        assert!(loaded.flying);

        // Survival mode resets flying ability to 0 in save.
        level.creative = false;
        level.game_mode = 0;
        let tag_survival = level.player_tag();
        let flying_survival = tag_survival
            .get("abilities")
            .and_then(|a| number(a.get("flying")))
            .unwrap_or(0);
        assert_eq!(flying_survival, 0);
    }

    #[test]
    fn unconsumed_root_vehicle_keeps_the_complete_vanilla_tree() {
        let entity = Tag::compound([
            ("id", Tag::String("minecraft:horse".into())),
            ("UUID", uuid_tag([5, 6, 7, 8])),
            (
                "Passengers",
                Tag::List(vec![Tag::compound([
                    ("id", Tag::String("minecraft:pig".into())),
                    ("UUID", uuid_tag([1, 2, 3, 4])),
                ])]),
            ),
        ]);
        let vehicle = Tag::compound([("Attach", uuid_tag([1, 2, 3, 4])), ("Entity", entity)]);
        let raw = Tag::compound([("RootVehicle", vehicle.clone())]);
        let mut level = Level::default();
        level.read_player(&raw);
        level.player_extra = Some(raw);
        assert_eq!(level.root_vehicle, Some([1, 2, 3, 4]));
        assert_eq!(level.player_tag().get("RootVehicle"), Some(&vehicle));
        // Even saving before the vehicle is restored must not destroy it.
        level.root_vehicle = None;
        assert_eq!(level.player_tag().get("RootVehicle"), Some(&vehicle));
    }

    /// `RecipesUsed` is what a furnace owes in experience, so a save that drops it pays the
    /// player nothing after a reload.
    #[test]
    fn a_furnaces_recipes_used_survive_a_save() {
        use crate::containers::BlockEntity;
        let Some(BlockEntity::Furnace(mut f)) = BlockEntity::for_block("minecraft:furnace") else {
            panic!("a furnace");
        };
        f.recipes_used
            .insert("minecraft:cooked_porkchop".to_owned(), 3);
        let tag = encode_block_entity((1, 64, 2), &BlockEntity::Furnace(f), "minecraft:furnace");
        let Some((_, BlockEntity::Furnace(back))) = decode_block_entity(&tag) else {
            panic!("a furnace back");
        };
        assert_eq!(back.recipes_used.get("minecraft:cooked_porkchop"), Some(&3));
    }
    /// The world's save (Level.write) writes the rules as they are now, and a
    /// world opened again reads them back (#74).
    #[test]
    fn level_save_keeps_changed_rules() {
        use crate::gamerules::{GameRules, Value};
        let dir =
            std::env::temp_dir().join(format!("horizon-gamerules-level-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut level = super::Level::default();
        level.write(&dir).unwrap();
        // Rules changed while playing, then saved and loaded again.
        let mut rules = GameRules::load(&dir);
        assert_eq!(rules, GameRules::default());
        assert!(rules.set("advance_time", Value::Bool(false)));
        assert!(rules.set("advance_weather", Value::Bool(false)));
        assert!(rules.set("random_tick_speed", Value::Int(0)));
        level.game_rules = rules.by_name();
        level.write(&dir).unwrap();
        assert_eq!(GameRules::load(&dir), rules);
        let read = super::Level::read(&dir).unwrap();
        assert_eq!(read.game_rules, rules.by_name());
        assert_eq!(read.game_rule("advance_time"), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A save written before the key was namespaced holds an id twice, once bare: vanilla
    /// refuses such a furnace, so a load merges them under the full id.
    #[test]
    fn a_furnaces_recipes_used_merges_bare_and_full_ids() {
        use crate::containers::BlockEntity;
        let Some(BlockEntity::Furnace(mut f)) = BlockEntity::for_block("minecraft:furnace") else {
            panic!("a furnace");
        };
        f.recipes_used
            .insert("iron_ingot_from_smelting_iron_ore".to_owned(), 2);
        f.recipes_used
            .insert("minecraft:iron_ingot_from_smelting_iron_ore".to_owned(), 3);
        let tag = encode_block_entity((1, 64, 2), &BlockEntity::Furnace(f), "minecraft:furnace");
        let Some((_, BlockEntity::Furnace(back))) = decode_block_entity(&tag) else {
            panic!("a furnace back");
        };
        assert_eq!(back.recipes_used.len(), 1);
        assert_eq!(
            back.recipes_used
                .get("minecraft:iron_ingot_from_smelting_iron_ore"),
            Some(&5)
        );
    }

    /// `WorldDataConfiguration.MAP_CODEC`'s `lenientOptionalFieldOf
    /// ("enabled_features", DEFAULT_FLAGS)`: a picked experiment's list
    /// round-trips, a vanilla-only selection leaves the tag out.
    #[test]
    fn enabled_features_round_trip_and_omit_the_vanilla_default() {
        let dir = gen_world("enabled-features");
        let mut level = Level {
            name: "Features".into(),
            enabled_features: vec![
                "minecraft:vanilla".into(),
                "minecraft:minecart_improvements".into(),
            ],
            ..Level::default()
        };
        level.write(&dir).unwrap();
        let back = Level::read(&dir).unwrap();
        assert_eq!(back.enabled_features, level.enabled_features);
        // The tag is in the file.
        let raw = crate::save::read_level_dat(&dir).unwrap();
        if let Tag::Compound(data) = &raw {
            let data = data.get(&JavaString::from("Data")).unwrap();
            assert!(matches!(data.get("enabled_features"), Some(Tag::List(_))));
        }
        // A vanilla-only selection writes no tag, and reads back as the
        // same vanilla-only list. A fresh folder: write_internal keeps an
        // older file's fields.
        let dir2 = gen_world("enabled-features-vanilla");
        level.enabled_features = vec!["minecraft:vanilla".into()];
        level.write(&dir2).unwrap();
        let raw = crate::save::read_level_dat(&dir2).unwrap();
        if let Tag::Compound(data) = &raw {
            let data = data.get(&JavaString::from("Data")).unwrap();
            assert!(data.get("enabled_features").is_none());
        }
        let back = Level::read(&dir2).unwrap();
        // The read side keeps its own convention: empty is the vanilla
        // default the file omitted.
        assert!(back.enabled_features.is_empty());
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir2);
    }

    fn gen_world(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `WorldOpenFlows.openWorldCheckWorldStemCompatibility`: every world a preset makes is
    /// stable, and a world with a changed dimension, a custom one or a customized generator
    /// is asked about.
    #[test]
    fn world_stem_compatibility_flags_unstable_and_customized_worlds() {
        for preset in [
            WorldPreset::Normal,
            WorldPreset::Flat,
            WorldPreset::LargeBiomes,
            WorldPreset::Amplified,
            WorldPreset::SingleBiomeSurface,
            WorldPreset::Debug,
        ] {
            let dir = gen_world("stem-stable");
            write_world_gen_settings(
                &dir,
                7,
                WorldGen {
                    preset,
                    structures: true,
                    bonus_chest: false,
                },
            )
            .unwrap();
            assert!(!world_gen_unstable(&dir), "{preset:?}");
            assert!(!is_old_customized_world(&dir));
            let _ = fs::remove_dir_all(dir);
        }
        // Without world_gen_settings there is nothing to ask about.
        let dir = gen_world("stem-none");
        assert!(!world_gen_unstable(&dir) && !is_old_customized_world(&dir));
        let _ = fs::remove_dir_all(dir);

        let dir = gen_world("stem-edit");
        write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
        let settings = world_gen_settings(&dir).unwrap();
        let edit = |change: &dyn Fn(&mut Tag)| {
            let mut tag = settings.clone();
            change(&mut tag);
            write_saved_data(&dir, "world_gen_settings", tag).unwrap();
        };
        fn dimension<'a>(tag: &'a mut Tag, key: &str) -> &'a mut Tag {
            tag.get_mut("dimensions").unwrap().get_mut(key).unwrap()
        }
        // A custom biome source in the Nether is not the preset.
        edit(&|tag| {
            let nether = dimension(tag, "minecraft:the_nether");
            let generator = nether.get_mut("generator").unwrap();
            *generator.get_mut("biome_source").unwrap() = Tag::compound([
                ("type", string("minecraft:fixed")),
                ("biome", string("minecraft:plains")),
            ]);
        });
        assert!(world_gen_unstable(&dir));
        // The End on another dimension type.
        edit(&|tag| {
            let end = dimension(tag, "minecraft:the_end");
            *end.get_mut("type").unwrap() = string("minecraft:overworld");
        });
        assert!(world_gen_unstable(&dir));
        // Another noise settings in the Overworld is fine (amplified), in the Nether it is not.
        edit(&|tag| {
            let nether = dimension(tag, "minecraft:the_nether");
            *nether
                .get_mut("generator")
                .unwrap()
                .get_mut("settings")
                .unwrap() = string("minecraft:overworld");
        });
        assert!(world_gen_unstable(&dir));
        // A fourth dimension, and a missing builtin one.
        edit(&|tag| {
            if let Some(Tag::Compound(dimensions)) = tag.get_mut("dimensions") {
                let end = dimensions.get(&"minecraft:the_end".into()).unwrap().clone();
                dimensions.insert("mypack:moon".into(), end);
            }
        });
        assert!(world_gen_unstable(&dir));
        edit(&|tag| {
            if let Some(Tag::Compound(dimensions)) = tag.get_mut("dimensions") {
                dimensions.remove(&"minecraft:the_end".into());
            }
        });
        assert!(world_gen_unstable(&dir));
        // legacy_custom_options marks an old customized world.
        edit(&|tag| {
            if let Tag::Compound(map) = tag {
                map.insert("legacy_custom_options".into(), string("{\"seaLevel\":63}"));
            }
        });
        assert!(is_old_customized_world(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    /// `WorldOpenFlows.loadWorldStem`: the enabled data packs' level stems are the base of
    /// `WorldDimensions.bake`, replacing the saved one of their id.
    #[test]
    fn world_stem_compatibility_reads_data_pack_dimensions() {
        let dir = gen_world("stem-datapack");
        write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
        assert!(!world_gen_unstable(&dir));
        let write = |pack: &str, file: &str, text: &str| {
            let path = dir.join("datapacks").join(pack).join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write("p", "pack.mcmeta", "{}");
        // A pack's dimension of another id is not vanilla-like.
        write(
            "p",
            "data/mypack/dimension/moon.json",
            r#"{"type":"minecraft:the_end","generator":{"type":"minecraft:noise","settings":"minecraft:end","biome_source":{"type":"minecraft:the_end"}}}"#,
        );
        assert!(world_gen_unstable(&dir));
        // Disabled, it counts for nothing.
        fs::write(dir.join("datapacks.json"), r#"{"disabled":["file/p"]}"#).unwrap();
        assert!(!world_gen_unstable(&dir));
        fs::remove_file(dir.join("datapacks.json")).unwrap();
        fs::remove_file(dir.join("datapacks/p/data/mypack/dimension/moon.json")).unwrap();
        assert!(!world_gen_unstable(&dir));
        // A pack's Nether with another biome source replaces the saved vanilla one.
        write(
            "p",
            "data/minecraft/dimension/the_nether.json",
            r#"{"type":"minecraft:the_nether","generator":{"type":"minecraft:noise","settings":"minecraft:nether","biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#,
        );
        assert!(world_gen_unstable(&dir));
        // And a vanilla-like one stays stable.
        write(
            "p",
            "data/minecraft/dimension/the_nether.json",
            r#"{"type":"minecraft:the_nether","generator":{"type":"minecraft:noise","settings":"minecraft:nether","biome_source":{"type":"minecraft:multi_noise","preset":"minecraft:nether"}}}"#,
        );
        assert!(!world_gen_unstable(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    /// A world that still needs its file fixes keeps its generator settings in level.dat; they
    /// are read as the fixes would leave them, so the question is asked on the first open.
    #[test]
    fn world_gen_settings_of_a_world_needing_file_fixes_are_read_in_memory() {
        let dir = gen_world("stem-prefix");
        write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
        let settings = world_gen_settings(&dir).unwrap();
        let customized = {
            let mut tag = settings.clone();
            if let Tag::Compound(map) = &mut tag {
                map.insert("legacy_custom_options".into(), string("{\"seaLevel\":63}"));
            }
            tag
        };
        // 26.3 layout moved back to the old one: settings in level.dat, no saved data file.
        fs::remove_file(saved_data_path(&dir, "world_gen_settings")).unwrap();
        write_dat(
            &dir.join("level.dat"),
            Tag::compound([(
                "Data",
                Tag::compound([
                    ("DataVersion", Tag::Int(4700)),
                    ("WorldGenSettings", customized),
                ]),
            )]),
        )
        .unwrap();
        assert!(needs_file_fixes(&dir));
        let read = world_gen_settings(&dir).expect("settings");
        assert!(read.get("dimensions").is_some());
        assert!(!world_gen_unstable(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn low_disk_space_is_under_64_mib_usable() {
        assert_eq!(LOW_DISK_SPACE, 64 * 1024 * 1024);
        let dir = std::env::temp_dir();
        assert!(estimate_disk_space(&dir) > 0);
        assert_eq!(
            low_disk_space(&dir),
            estimate_disk_space(&dir) < LOW_DISK_SPACE
        );
        // A path that does not exist cannot be asked: vanilla reads that as plenty.
        #[cfg(unix)]
        assert_eq!(
            estimate_disk_space(&dir.join("horizon-no-such-folder/x")),
            u64::MAX
        );
    }

    #[test]
    fn mod_attachments_save_in_horizons_namespace_only_when_some() {
        use horizon_world::attachments::Attachments;
        let root = std::env::temp_dir().join(format!("horizon-attach-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let save = SaveDir::new(root.clone(), LevelShape::OVERWORLD);
        let file = root.join("data/horizon/attachments.dat");
        save.save_attachments(&Attachments::default()).unwrap();
        assert!(
            !root.join("data").exists(),
            "a level without mod data writes nothing"
        );
        let data = Compound::from_iter([("example:count".into(), Tag::Int(3))]);
        save.save_attachments(&Attachments::from_nbt(data.clone()))
            .unwrap();
        assert!(file.is_file());
        assert_eq!(save.load_attachments().to_nbt(), data);
        fs::write(&file, b"not nbt").unwrap();
        assert!(save.load_attachments().to_nbt().is_empty());
        assert_eq!(
            fs::read(root.join("data/horizon/attachments.dat.corrupt")).unwrap(),
            b"not nbt"
        );
        assert!(!file.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn forced_chunks_survive_dimension_restarts_and_removals() {
        let root = std::env::temp_dir().join(format!("horizon-forced-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dimensions = [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ];
        for (i, dim) in dimensions.iter().enumerate() {
            let save = SaveDir::new(dimension_dir(&root, dim), LevelShape::OVERWORLD);
            assert!(save.load_forced_chunks().is_empty());
            let forced = HashSet::from([(i as i32, -17), (-23, i as i32), (-1, -1)]);
            save.save_forced_chunks(&forced).unwrap();
        }
        for (i, dim) in dimensions.iter().enumerate() {
            let path = dimension_dir(&root, dim);
            let save = SaveDir::new(path.clone(), LevelShape::OVERWORLD);
            let mut forced = save.load_forced_chunks();
            assert_eq!(
                forced,
                HashSet::from([(i as i32, -17), (-23, i as i32), (-1, -1)])
            );
            let mut data = read_saved_data(&path, "chunk_tickets").unwrap();
            let other = Tag::compound([
                ("type", Tag::String("minecraft:portal".into())),
                ("level", Tag::Int(30)),
                ("chunk_pos", Tag::IntArray(vec![7, -8])),
                ("ticks_left", Tag::Long(42)),
            ]);
            if let Some(Tag::List(tickets)) = data.get_mut("tickets") {
                assert!(
                    tickets
                        .iter()
                        .all(|t| t.get("level") == Some(&Tag::Int(31)))
                );
                tickets.push(other.clone());
            }
            write_saved_data(&path, "chunk_tickets", data).unwrap();
            forced.remove(&(-1, -1));
            save.save_forced_chunks(&forced).unwrap();
            drop(save);
            let reopened = SaveDir::new(path.clone(), LevelShape::OVERWORLD);
            assert_eq!(reopened.load_forced_chunks(), forced);
            reopened.save_forced_chunks(&HashSet::new()).unwrap();
            assert!(
                SaveDir::new(path.clone(), LevelShape::OVERWORLD)
                    .load_forced_chunks()
                    .is_empty()
            );
            assert_eq!(
                read_saved_data(&path, "chunk_tickets")
                    .unwrap()
                    .get("tickets"),
                Some(&Tag::List(vec![other]))
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// The Debug preset (`worldgen/world_preset/debug_all_block_states.json`): its id names it,
    /// the saved overworld generator is `minecraft:debug` and reads back as it, and the create
    /// screen's button never cycles to it.
    #[test]
    fn the_debug_preset_is_saved_and_read_back() {
        assert_eq!(
            WorldPreset::of_id("minecraft:debug_all_block_states"),
            Some(WorldPreset::Debug)
        );
        assert!(!WorldPreset::ALL.contains(&WorldPreset::Debug));
        let dir = gen_world("debug-preset");
        write_world_gen_settings(
            &dir,
            7,
            WorldGen {
                preset: WorldPreset::Debug,
                structures: true,
                bonus_chest: false,
            },
        )
        .unwrap();
        let settings = world_gen_settings(&dir).unwrap();
        let generator = settings
            .get("dimensions")
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"))
            .unwrap();
        assert_eq!(
            text(generator.get("type")).as_deref(),
            Some("minecraft:debug")
        );
        assert_eq!(WorldGen::read(&dir).unwrap().preset, WorldPreset::Debug);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn world_preset_next_and_prev_cycle_bidirectionally() {
        let first = WorldPreset::Normal;
        assert_eq!(first.next(), WorldPreset::Flat);
        assert_eq!(first.prev(), WorldPreset::SingleBiomeSurface);
        assert_eq!(WorldPreset::SingleBiomeSurface.next(), WorldPreset::Normal);
        for preset in WorldPreset::ALL {
            assert_eq!(preset.next().prev(), preset);
            assert_eq!(preset.prev().next(), preset);
        }
    }
    /// A region record of the dimension folder `save` at chunk (0, 0), written
    /// as an older version left it.
    fn write_old_record(save: &SaveDir, folder: &str, tag: Tag) {
        let record = Record::Tag(Arc::new(NamedTag {
            name: "".into(),
            tag,
        }));
        let cache = match Folder::of(folder) {
            Folder::Region => &save.regions,
            Folder::Entities => &save.entity_regions,
            Folder::Poi => &save.poi_regions,
        };
        super::write_region_records(&save.root, cache, folder, &[((0, 0), &record)]).unwrap();
    }

    #[test]
    fn older_records_of_a_current_world_upgrade_as_they_are_read() {
        let world = std::env::temp_dir().join(format!("horizon-lazy-dv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        let save = SaveDir::new(
            super::dimension_dir(&world, "minecraft:overworld"),
            LevelShape::OVERWORLD,
        );
        // A 1.20.1 chunk (DataVersion 3465): 1.20.3 renamed its grass to short_grass.
        let section = Tag::compound([
            ("Y", Tag::Byte(-4)),
            (
                "block_states",
                Tag::compound([(
                    "palette",
                    Tag::List(vec![Tag::compound([("Name", string("minecraft:grass"))])]),
                )]),
            ),
            (
                "biomes",
                Tag::compound([("palette", Tag::List(vec![string("minecraft:plains")]))]),
            ),
        ]);
        write_old_record(
            &save,
            "region",
            Tag::compound([
                ("DataVersion", Tag::Int(3465)),
                ("xPos", Tag::Int(0)),
                ("zPos", Tag::Int(0)),
                ("yPos", Tag::Int(-4)),
                ("Status", string("minecraft:full")),
                ("sections", Tag::List(vec![section])),
            ]),
        );
        let chunk = save.raw_chunk(0, 0).expect("the chunk reads");
        assert_eq!(
            number(chunk.tag.get("DataVersion")),
            Some(i64::from(DATA_VERSION))
        );
        let Some(Tag::List(sections)) = chunk.tag.get("sections") else {
            panic!("no sections: {:?}", chunk.tag);
        };
        let Some(Tag::List(palette)) = sections[0]
            .get("block_states")
            .and_then(|b| b.get("palette"))
        else {
            panic!("no palette");
        };
        assert_eq!(
            text(palette[0].get("id")).as_deref(),
            Some("minecraft:short_grass")
        );
        // Decoded, it keeps its blocks: read raw, the 26.x decoder finds no `id` in a
        // 1.20 palette (`Name`) and the whole column is air.
        if let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") {
            let root = PathBuf::from(root);
            let assets = crate::assets::Assets::load(&root).expect("assets");
            let biomes = Biomes::load(&root).expect("biomes");
            let grass = assets
                .registry
                .default_state("minecraft:short_grass")
                .unwrap();
            let loaded = save
                .load(0, 0, assets.registry, &biomes)
                .expect("the chunk loads");
            assert_eq!(loaded.get(3, -60, 7), grass.0);
        }
        // A 1.20.1 item entity: 1.20.5 made its stack `count` (an int) from `Count`.
        write_old_record(
            &save,
            "entities",
            Tag::compound([
                ("DataVersion", Tag::Int(3465)),
                ("Position", Tag::IntArray(vec![0, 0])),
                (
                    "Entities",
                    Tag::List(vec![Tag::compound([
                        ("id", string("minecraft:item")),
                        (
                            "Item",
                            Tag::compound([
                                ("id", string("minecraft:stone")),
                                ("Count", Tag::Byte(3)),
                            ]),
                        ),
                    ])]),
                ),
            ]),
        );
        let entities = save.load_entities(0, 0);
        let item = entities[0].get("Item").expect("the item stack");
        assert_eq!(item.get("count"), Some(&Tag::Int(3)));
        assert!(item.get("Count").is_none());
        let _ = std::fs::remove_dir_all(&world);
    }

    #[test]
    fn old_chunks_are_found_in_the_saved_records_and_old_poi_upgrade() {
        let world = std::env::temp_dir().join(format!("horizon-lazy-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        let nether = SaveDir::new(
            super::dimension_dir(&world, "minecraft:the_nether"),
            LevelShape::NETHER,
        );
        // A 1.20.1 Nether chunk: upgraded it has no `blending_data` (only Overworld
        // chunks blend) and the current DataVersion, so only the saved record says
        // it is older than blending (4997), as `IOWorker.isOldChunk` reads it.
        write_old_record(
            &nether,
            "region",
            Tag::compound([
                ("DataVersion", Tag::Int(3465)),
                ("xPos", Tag::Int(0)),
                ("zPos", Tag::Int(0)),
                ("yPos", Tag::Int(0)),
                ("Status", string("minecraft:full")),
                ("sections", Tag::List(Vec::new())),
            ]),
        );
        let upgraded = nether.raw_chunk(0, 0).expect("the chunk reads");
        assert!(upgraded.tag.get("blending_data").is_none());
        assert_ne!(nether.old_chunk_bits(0, 0)[0] & 1, 0);
        // A POI record of before 1.14.4 (no DataVersion, read as 1945): its
        // sections at the root, which 1946 moved under `Sections`.
        let overworld = SaveDir::new(
            super::dimension_dir(&world, "minecraft:overworld"),
            LevelShape::OVERWORLD,
        );
        write_old_record(
            &overworld,
            "poi",
            Tag::compound([(
                "0",
                Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(Vec::new()))]),
            )]),
        );
        let poi = overworld.read_record("poi", 0, 0).expect("the POI reads");
        assert!(poi.tag.get("Sections").and_then(|s| s.get("0")).is_some());
        assert_eq!(
            number(poi.tag.get("DataVersion")),
            Some(i64::from(DATA_VERSION))
        );
        let _ = std::fs::remove_dir_all(&world);
    }

    #[test]
    fn a_chunk_from_before_1_18_is_brought_to_the_current_layout() {
        let world = std::env::temp_dir().join(format!("horizon-lazy-117-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        let save = SaveDir::new(
            super::dimension_dir(&world, "minecraft:overworld"),
            LevelShape::OVERWORLD,
        );
        // A 1.17.1 chunk (DataVersion 2730), its fields under `Level`: section 0 all stone.
        let section = Tag::compound([
            ("Y", Tag::Byte(0)),
            (
                "Palette",
                Tag::List(vec![
                    Tag::compound([("Name", string("minecraft:air"))]),
                    Tag::compound([("Name", string("minecraft:stone"))]),
                ]),
            ),
            (
                "BlockStates",
                Tag::LongArray(vec![0x1111_1111_1111_1111; 256]),
            ),
        ]);
        write_old_record(
            &save,
            "region",
            Tag::compound([
                ("DataVersion", Tag::Int(2730)),
                (
                    "Level",
                    Tag::compound([
                        ("xPos", Tag::Int(0)),
                        ("zPos", Tag::Int(0)),
                        ("Status", string("full")),
                        ("Biomes", Tag::IntArray(vec![1; 1024])),
                        ("Sections", Tag::List(vec![section])),
                    ]),
                ),
            ]),
        );
        let chunk = save.raw_chunk(0, 0).expect("the chunk reads");
        assert!(
            chunk.tag.get("Level").is_none(),
            "still wrapped: {:?}",
            chunk.tag
        );
        assert_eq!(number(chunk.tag.get("xPos")), Some(0));
        // 1.18's height fix: an Overworld chunk older than the new heights blends with them.
        assert!(chunk.tag.get("blending_data").is_some());
        let _ = std::fs::remove_dir_all(&world);
    }
}
