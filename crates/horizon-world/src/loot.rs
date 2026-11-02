//! Loot tables: `LootTable.fill` for chest-style loot tables (pools,
//! weighted entries, nested tables, item modifiers and conditions, the
//! stack splitter and the slot shuffle) and `LootTable.getRandomItems` for
//! entity loot (the victim, its killer, the damage source and the last
//! player to hurt it), with vanilla's random draw order.
//!
//! Items keep their id, count, damage, enchantments and potion. Exploration
//! maps are kept as found (structure search is not run); conditions that
//! need an entity, tool or damage source fail when the context has none, as
//! they do for containers.
use crate::{lazy_json::LazyJson, nbt::Tag};
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs, io,
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex, Weak},
};

/// Raw files a world's data packs add over the game's own data, by
/// `<kind>/<ns>:<path>` (a `recipe/minecraft:stick` key names the file
/// `data/minecraft/recipe/stick.json`), each id's files in pack order.
/// `datapack::DataStore.files` is the source; the content tables merge these
/// over what they read from the resource root, the way
/// `SimpleJsonResourceReloadListener` and `TagLoader` layer a pack stack.
#[derive(Clone, Copy, Default)]
pub struct PackOverlay<'a> {
    pub files: &'a [(String, Vec<Vec<u8>>)],
}
impl PackOverlay<'static> {
    /// No packs: the tables are the resource root's alone.
    pub const NONE: Self = Self { files: &[] };
}
impl PackOverlay<'_> {
    /// The overlay's files under `<kind>/`, with the prefix stripped.
    pub fn kind(&self, kind: &str) -> impl Iterator<Item = (&str, &Vec<Vec<u8>>)> {
        let prefix = format!("{kind}/");
        self.files
            .iter()
            .filter_map(move |(key, files)| Some((key.strip_prefix(&prefix)?, files)))
    }
    /// The overlay's tag files of one registry, over every namespace: the
    /// keys are `tags/<ns>:<registry>/<path>`, and the yielded id is the
    /// tag id `<ns>:<path>` the game's own tags are keyed by
    /// (`TagLoader`'s ids).
    pub fn tag_kind(&self, registry: &str) -> impl Iterator<Item = (String, &Vec<Vec<u8>>)> {
        let prefix = format!("{registry}/");
        self.files.iter().filter_map(move |(key, files)| {
            let rest = key.strip_prefix("tags/")?;
            let (ns, path) = rest.split_once(':')?;
            let path = path.strip_prefix(&prefix)?;
            Some((format!("{ns}:{path}"), files))
        })
    }
    /// The last pack's file for an id, which is the one
    /// `ResourceManager.listMatchingResources` hands out.
    pub fn top(files: &[Vec<u8>]) -> &[u8] {
        files.last().map(Vec::as_slice).unwrap_or_default()
    }
}

/// A loot function of a mod's own kind: the entry named a `type` in a
/// namespace outside `minecraft:`, and the roll kept it for this call.
/// The kind, the JSON, the context and the stack to change are here.
/// API 133.
#[inline(never)]
pub fn native_loot_function(
    kind: &str,
    data: &Value,
    ctx: &NativeLootContext<'_>,
    stack: &mut LootStack,
) {
    let _ = (kind, data, ctx, stack);
}

/// A loot condition of a mod's own kind: the entry named a `type` in a
/// namespace outside `minecraft:`. The kind, the JSON, the context and
/// whether it holds are here (`holds` starts `false`; a mod that claims
/// the kind sets it). API 133.
#[inline(never)]
pub fn native_loot_condition(
    kind: &str,
    data: &Value,
    ctx: &NativeLootContext<'_>,
    holds: &mut bool,
) {
    let _ = (kind, data, ctx, holds);
}

/// The tool a loot context names: its item id and enchantments.
pub struct NativeLootTool {
    pub item: String,
    pub enchantments: Vec<(String, i32)>,
}

/// What a mod's own loot kinds see of the roll: the entity, the damage,
/// the player who killed it, the tool, the block, the luck, the biome,
/// where the roll happens, and a seed from the roll's random. The tool and
/// block are filled by the items engine (block drops, entity drops,
/// loot_table, interact_loot); the entity, damage and player by the main
/// loot engine; each is None in the other. A table without a mod's kinds
/// draws nothing and stays bit-for-bit the same.
pub struct NativeLootContext<'a> {
    /// The entity the table names (`this`), if any.
    pub entity: Option<&'a LootEntity>,
    /// The damage that killed it, if any.
    pub damage: Option<&'a LootDamage>,
    /// The player whose kill dropped the table (`LootEntity`), if any.
    pub player: Option<&'a LootEntity>,
    /// The tool used, if any. The items engine fills it (all of its
    /// tables: block drops, entity drops, loot_table, interact_loot);
    /// the main loot engine has no tool in its context.
    pub tool: Option<NativeLootTool>,
    /// The block a block-drop table breaks, if any. The items engine
    /// fills it; the main loot engine has no block in its context.
    pub block: Option<&'a str>,
    /// `LootContext.getLuck`.
    pub luck: f32,
    /// The biome the roll is in, if any.
    pub biome: Option<&'a str>,
    /// A seed from the roll's random, for a mod's own draws. Drawn only
    /// when a mod's kind is in the table.
    pub seed: i64,
    /// Where the roll happens: the origin, the block entity and the level,
    /// when the game rolled the table at a place (a broken block's drops,
    /// a mob's death loot); None for the other rolls. Either engine fills it.
    /// API @API:loot_context_place@.
    pub place: Option<Rc<NativeLootPlace>>,
}

/// Where a loot roll happens, as a mod's own loot kinds see it:
/// `LootContextParams.ORIGIN`, `LootContextParams.BLOCK_ENTITY`, and what
/// vanilla's own conditions read of `LootContext.getLevel` (`TimeCheck`:
/// the time; `WeatherCheck`: the rain and the thunder). The game sets it
/// around a roll with [`with_native_loot_place`]. API @API:loot_context_place@.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeLootPlace {
    /// `ORIGIN`: a broken block's centre, a dead mob's position.
    pub origin: Option<[f64; 3]>,
    /// `BLOCK_ENTITY`: the broken block's entity as it was before the
    /// break, saved with its id and position (`saveWithFullMetadata`); a
    /// mod's block entity carries its own data and its `Items`.
    pub block_entity: Option<Tag>,
    /// The level the roll is in.
    pub level: Option<NativeLootLevel>,
}

/// The level a loot roll is in, as a mod's own loot kinds see it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeLootLevel {
    /// The dimension's id (`minecraft:overworld`).
    pub dimension: String,
    /// `Level.getGameTime`.
    pub game_time: i64,
    /// `Level.getDayTime`: it counts on from day to day, so the time of
    /// day is `day_time % 24000` (night from 13000 to 23000).
    pub day_time: i64,
    /// `Level.isRaining`.
    pub raining: bool,
    /// `Level.isThundering`.
    pub thundering: bool,
}

thread_local! {
    static NATIVE_LOOT_PLACE: RefCell<Option<Rc<NativeLootPlace>>> = const { RefCell::new(None) };
}

/// Runs `roll` with `place` as where its loot rolls happen: every
/// [`NativeLootContext`] made inside it, by either engine, carries it. The
/// place before is back when `roll` returns, so these nest.
/// API @API:loot_context_place@.
pub fn with_native_loot_place<T>(place: NativeLootPlace, roll: impl FnOnce() -> T) -> T {
    struct Restore(Option<Rc<NativeLootPlace>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let before = self.0.take();
            NATIVE_LOOT_PLACE.with(|p| *p.borrow_mut() = before);
        }
    }
    let _restore = Restore(NATIVE_LOOT_PLACE.with(|p| p.replace(Some(Rc::new(place)))));
    roll()
}

/// The place [`with_native_loot_place`] set for the rolls now running, if any.
pub fn current_native_loot_place() -> Option<Rc<NativeLootPlace>> {
    NATIVE_LOOT_PLACE.with(|p| p.borrow().clone())
}

/// One generated item stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LootStack {
    pub item: String,
    pub count: i32,
    pub damage: i32,
    pub enchantments: Vec<(String, i32)>,
    pub potion: Option<String>,
    /// Set by exploration_map (the map_id component of a found map).
    pub map: bool,
    /// The search an exploration_map function asked for, left to the game
    /// (it needs the level's structures and maps).
    pub exploration: Option<Box<ExplorationMap>>,
    /// Set by set_ominous_bottle_amplifier.
    pub ominous_amplifier: Option<i32>,
    /// Added by set_stew_effect (the `suspicious_stew_effects` component):
    /// (effect id, duration in ticks).
    pub stew_effects: Vec<(String, i32)>,
    /// Set by set_name: (`custom_name` or `item_name`, the text component).
    pub names: Vec<(String, Value)>,
    /// Set by set_components: (component id, its JSON value; null removes
    /// it), a later patch of the same id replacing an earlier one.
    pub components: Vec<(String, Value)>,
}
/// `ExplorationMapFunction`'s settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorationMap {
    /// A structure tag, without `#`.
    pub destination: String,
    pub decoration: String,
    pub zoom: u8,
    pub search_radius: i32,
    pub skip_existing_chunks: bool,
}
impl ExplorationMap {
    /// The function's fields with their codec defaults (`DEFAULT_DESTINATION`
    /// on_treasure_maps, `DEFAULT_DECORATION` mansion, zoom 2, radius 50,
    /// skipping known structures).
    pub fn from_json(f: &Value) -> Self {
        Self {
            destination: f["destination"]
                .as_str()
                .unwrap_or("#minecraft:on_treasure_maps")
                .trim_start_matches('#')
                .to_owned(),
            decoration: f["decoration"]
                .as_str()
                .unwrap_or("minecraft:mansion")
                .to_owned(),
            zoom: f["zoom"].as_u64().unwrap_or(2) as u8,
            search_radius: f["search_radius"].as_i64().unwrap_or(50) as i32,
            skip_existing_chunks: f["skip_existing_chunks"].as_bool().unwrap_or(true),
        }
    }
}
impl LootStack {
    fn new(item: &str) -> Self {
        Self {
            item: item.to_owned(),
            count: 1,
            damage: 0,
            enchantments: Vec::new(),
            potion: None,
            map: false,
            exploration: None,
            ominous_amplifier: None,
            stew_effects: Vec::new(),
            names: Vec::new(),
            components: Vec::new(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.count <= 0 || self.item == "minecraft:air"
    }
    /// `ItemStack.getCount`: 0 for an empty stack.
    pub fn get_count(&self) -> i32 {
        if self.is_empty() { 0 } else { self.count }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ItemFacts {
    max_stack: i32,
    max_damage: i32,
    enchantable: Option<i32>,
}
#[derive(Debug)]
struct Enchantment {
    name: String,
    supported: Value,
    primary: Option<Value>,
    exclusive: Value,
    max_level: i32,
    weight: i32,
    min_cost: (i32, i32),
    max_cost: (i32, i32),
    /// `slots`: the equipment slot groups the enchantment works in.
    slots: Vec<String>,
    /// `effects` (equipment_drops and others).
    effects: Value,
}
impl Enchantment {
    fn min_cost(&self, level: i32) -> i32 {
        self.min_cost.0 + self.min_cost.1 * (level - 1)
    }
    fn max_cost(&self, level: i32) -> i32 {
        self.max_cost.0 + self.max_cost.1 * (level - 1)
    }
}

/// Loot tables, items, enchantments and the tags they reference.
pub struct LootData {
    tables: Arc<LootTables>,
    modifiers: HashMap<String, Value>,
    predicates: HashMap<String, Value>,
    items: HashMap<String, ItemFacts>,
    /// Enchantments in registry (identifier) order.
    enchantments: Vec<Enchantment>,
    /// Raw tag files by (registry, tag id).
    tags: HashMap<(String, String), Vec<Value>>,
    /// Smelting recipes in recipe id order: (ingredient, result, result count).
    smelting: Vec<(Value, String, i32)>,
}

/// An entity a loot context names (`this`, `attacker`, `direct_attacker`,
/// `attacking_player`): the facts entity predicates read.
#[derive(Clone, Debug, Default)]
pub struct LootEntity {
    /// Entity type id (`minecraft:zombie`).
    pub kind: String,
    /// A `LivingEntity` (has equipment and enchantment levels).
    pub living: bool,
    /// `Entity.isOnFire` (fire-immune entities never are).
    pub on_fire: bool,
    pub baby: bool,
    /// The entity it rides (`getVehicle`).
    pub vehicle: Option<Box<LootEntity>>,
    /// `AbstractCubeMob.getSize`.
    pub cube_size: Option<i32>,
    /// A sheep's colour name and whether it is sheared.
    pub sheep: Option<(String, bool)>,
    /// A raider: whether it is a captain (wears the ominous banner).
    pub captain: Option<bool>,
    /// A frog's variant id (`minecraft:warm`).
    pub frog_variant: Option<String>,
    /// A chicken's variant id (`minecraft:warm`).
    pub chicken_variant: Option<String>,
    /// A charged creeper (`Creeper.isPowered`).
    pub powered: bool,
    /// Worn and held items by `EquipmentSlot` name (mainhand, offhand,
    /// feet, legs, chest, head, body, saddle).
    pub equipment: Vec<(String, LootStack)>,
}
impl LootEntity {
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            living: true,
            ..Self::default()
        }
    }
    pub fn item(&self, slot: &str) -> Option<&LootStack> {
        self.equipment
            .iter()
            .find(|(s, i)| s == slot && !i.is_empty())
            .map(|(_, i)| i)
    }
}
/// A `DamageSource`: its damage type and the entities behind it.
#[derive(Clone, Debug, Default)]
pub struct LootDamage {
    /// Damage type id (`minecraft:player_attack`).
    pub kind: String,
    /// `getEntity`: the attacker (the shooter of a projectile).
    pub source: Option<LootEntity>,
    /// `getDirectEntity`: the projectile, or the attacker itself.
    pub direct: Option<LootEntity>,
}
/// The parameters of `LivingEntity.dropFromLootTable`.
#[derive(Clone, Debug, Default)]
pub struct EntityLootParams {
    pub this: LootEntity,
    pub damage: Option<LootDamage>,
    /// `LAST_DAMAGE_PLAYER`: set when a player killed the entity.
    pub player: Option<LootEntity>,
    /// The killing player's luck.
    pub luck: f32,
}
/// `EquipmentSlot.VALUES` order.
pub const EQUIPMENT_SLOTS: [&str; 8] = [
    "mainhand", "offhand", "feet", "legs", "chest", "head", "body", "saddle",
];
/// `EquipmentSlotGroup.test`.
fn slot_in_group(slot: &str, group: &str) -> bool {
    match group {
        "any" => true,
        "hand" => slot == "mainhand" || slot == "offhand",
        "armor" => matches!(slot, "feet" | "legs" | "chest" | "head" | "body"),
        _ => slot == group,
    }
}
/// The item a recipe's `result` names. `ItemStackTemplate.CODEC` and `TransmuteResult.CODEC`
/// read a stack's fields (`id`, `count`, `components`) or, as their alternative, a bare item id
/// (`Item.CODEC`): `"result": {"id": "mod:item"}` and `"result": "mod:item"` are the same item.
/// None for a stack without an `id` and for anything else, such as the `item` key of older
/// versions.
pub fn recipe_result_id(result: &Value) -> Option<&str> {
    match result {
        Value::String(id) => Some(id),
        Value::Object(fields) => fields.get("id")?.as_str(),
        _ => None,
    }
}
/// `LevelBasedValue.calculate`.
pub fn level_value(v: &Value, level: i32) -> f32 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.) as f32,
        Value::Object(_) => match v["type"].as_str().unwrap_or("") {
            "minecraft:linear" => {
                level_value(&v["base"], level)
                    + level_value(&v["per_level_above_first"], level) * (level - 1) as f32
            }
            "minecraft:constant" => level_value(&v["value"], level),
            "minecraft:clamped" => {
                let value = level_value(&v["value"], level);
                let min = v["min"].as_f64().unwrap_or(0.) as f32;
                let max = v["max"].as_f64().unwrap_or(0.) as f32;
                value.clamp(min, max)
            }
            "minecraft:fraction" => {
                level_value(&v["numerator"], level) / level_value(&v["denominator"], level)
            }
            "minecraft:levels_squared" => {
                (level * level) as f32 + v["added"].as_f64().unwrap_or(0.) as f32
            }
            "minecraft:lookup" => {
                let values = v["values"].as_array().map_or(&[][..], Vec::as_slice);
                match values.get((level - 1).max(0) as usize) {
                    Some(x) if level >= 1 => x.as_f64().unwrap_or(0.) as f32,
                    _ => level_value(&v["fallback"], level),
                }
            }
            _ => 0.,
        },
        _ => 0.,
    }
}
/// `MinMaxBounds.Ints.matches` (a number or `{min, max}`).
fn int_bounds(v: &Value, value: i32) -> bool {
    match v {
        Value::Null => true,
        Value::Number(n) => n.as_i64() == Some(i64::from(value)),
        Value::Object(o) => {
            o.get("min")
                .and_then(Value::as_i64)
                .is_none_or(|m| i64::from(value) >= m)
                && o.get("max")
                    .and_then(Value::as_i64)
                    .is_none_or(|m| i64::from(value) <= m)
        }
        _ => false,
    }
}

/// A JSON number read by an int codec (`Number.intValue`): `3.0`, which
/// 1.21.1 mods write for rolls and counts, is 3.
fn int_value(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

/// `data/<ns>/<kind>/` of every namespace, as `ns:path`: vanilla's, and in
/// a content mods' root theirs (docs/modpacks.md), whose chests name their
/// own loot tables.
/// A kind's overlay files over what the resource root provided: the top
/// pack's file for each id replaces the rest (`SimpleJsonResourceReloadListener`
/// sees one resource per id).
fn overlay_map(overlay: &PackOverlay, kind: &str, out: &mut HashMap<String, Value>) {
    for (id, files) in overlay.kind(kind) {
        let Ok(v) = serde_json::from_slice::<Value>(PackOverlay::top(files)) else {
            continue;
        };
        out.insert(id.to_owned(), v);
    }
}

/// Loot tables by id. Each is kept as its JSON text and read into a [`Value`]
/// the first time it is rolled: of vanilla's ~1,450 tables a game rolls a
/// few, and as `Value`s they took ~7 MB for 0.6 MB of JSON. Every table was
/// read whole when it loaded, so reading it again cannot fail.
#[derive(Debug, Default)]
pub struct LootTables(HashMap<String, LazyJson>);
impl LootTables {
    /// The table `id`.
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.0.get(id).map(LazyJson::get)
    }
    pub fn contains_key(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
/// Each table kept as its JSON, the last of an id winning as in a map.
impl FromIterator<(String, Value)> for LootTables {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(tables: I) -> Self {
        Self(
            tables
                .into_iter()
                .map(|(id, table)| (id, LazyJson::new(&table)))
                .collect(),
        )
    }
}
impl std::ops::Index<&str> for LootTables {
    type Output = Value;
    /// As a map indexes: panics on a missing table.
    fn index(&self, id: &str) -> &Value {
        self.get(id).unwrap_or_else(|| panic!("no loot table {id}"))
    }
}

/// The loot tables of every namespace in `root`'s `data/`, with `overlay`'s
/// over them. Shared while one is alive: `Items` and [`LootData`] of the
/// same root and packs hold one map, not a copy each (7 MB). The key holds
/// the packs' bytes and each root file's size and modification time, so a
/// table changed on disk is read again rather than taken from a held map.
pub fn shared_tables(root: &Path, overlay: &PackOverlay) -> io::Result<Arc<LootTables>> {
    static LOADED: Mutex<Vec<(u64, Weak<LootTables>)>> = Mutex::new(Vec::new());
    let key = {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        root.hash(&mut hasher);
        let data = root.join("data");
        let mut namespaces: Vec<_> = fs::read_dir(&data)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        namespaces.sort();
        for namespace in namespaces {
            hash_files(&namespace.join("loot_table"), &mut hasher);
        }
        overlay
            .kind("loot_table")
            .for_each(|file| file.hash(&mut hasher));
        hasher.finish()
    };
    let held = |loaded: &mut Vec<(u64, Weak<LootTables>)>| {
        loaded.retain(|(_, tables)| tables.strong_count() > 0);
        loaded
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, tables)| tables.upgrade())
    };
    if let Some(tables) = held(&mut LOADED.lock().unwrap_or_else(|e| e.into_inner())) {
        return Ok(tables);
    }
    let mut tables = HashMap::new();
    load_kind(&root.join("data"), "loot_table", &mut tables)?;
    overlay_map(overlay, "loot_table", &mut tables);
    // The mods' hooks run outside the lock.
    let mut each: Vec<_> = tables.into_iter().collect();
    each.sort_by(|a, b| a.0.cmp(&b.0));
    let tables: LootTables = each
        .into_iter()
        .map(|(id, mut table)| {
            native_loot_table_load(&id, &mut table);
            (id, table)
        })
        .collect();
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    // Another thread may have loaded the same tables meanwhile: keep one.
    if let Some(tables) = held(&mut loaded) {
        return Ok(tables);
    }
    let tables = Arc::new(tables);
    loaded.push((key, Arc::downgrade(&tables)));
    Ok(tables)
}

/// A native mod's hook point (docs/native-mods.md), listed in `native-hooks.json` with the game's:
/// the loot table `id` (`minecraft:chests/simple_dungeon`, `minecraft:blocks/stone`,
/// `minecraft:entities/zombie`) as it loads, its JSON after the data packs', before anything
/// rolls it (NeoForge's `LootTableLoadEvent`, Fabric's `LootTableEvents.MODIFY`). Add pools to
/// `table["pools"]` or entries to a pool's `entries`, change or replace it (no pools: it drops
/// nothing; keep its `random_sequence` when replacing it, as a mob's table without one drops
/// nothing). Called once per table and load, in id order, wherever the tables load: the server,
/// and a client too, so make the same change each time, and don't load loot tables from here.
/// Two loads may run at once on two threads. A `/reload` or a world opened with other packs
/// may load them again, or keep the tables already loaded from the same files. API
/// 75.
#[inline(never)]
pub fn native_loot_table_load(id: &str, table: &mut Value) {
    crate::native_load_hooks::loot_table_load(id, table);
}

fn load_kind(data: &Path, kind: &str, out: &mut HashMap<String, Value>) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(data) else {
        return Ok(());
    };
    let mut namespaces: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    namespaces.sort();
    for ns in namespaces {
        load_dir(&data.join(&ns).join(kind), &ns, "", out)?;
    }
    Ok(())
}

/// Every file's path, size and modification time under `dir`, in name order.
fn hash_files(dir: &Path, hasher: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            hash_files(&path, hasher);
        } else {
            path.hash(hasher);
            meta.len().hash(hasher);
            meta.modified().ok().hash(hasher);
        }
    }
}

fn load_dir(
    dir: &Path,
    namespace: &str,
    prefix: &str,
    out: &mut HashMap<String, Value>,
) -> io::Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.is_dir() {
            load_dir(&path, namespace, &format!("{prefix}{stem}/"), out)?;
        } else if path.extension().is_some_and(|e| e == "json") {
            let value = serde_json::from_slice(&fs::read(&path)?)?;
            out.insert(format!("{namespace}:{prefix}{stem}"), value);
        }
    }
    Ok(())
}
fn cost(v: &Value) -> (i32, i32) {
    (
        v["base"].as_i64().unwrap_or(0) as i32,
        v["per_level_above_first"].as_i64().unwrap_or(0) as i32,
    )
}

impl LootData {
    /// Load from a resource root holding `items.json` and `data/`: loot
    /// tables, item modifiers, predicates, tags and recipes of every
    /// namespace, vanilla's enchantments. `overlay` adds a world's data
    /// packs over it.
    pub fn load(root: &Path, overlay: &PackOverlay) -> io::Result<Self> {
        let all = root.join("data");
        let data = all.join("minecraft");
        let tables = shared_tables(root, overlay)?;
        let mut modifiers = HashMap::new();
        load_kind(&all, "item_modifier", &mut modifiers)?;
        overlay_map(overlay, "item_modifier", &mut modifiers);
        let mut predicates = HashMap::new();
        load_kind(&all, "predicate", &mut predicates)?;
        overlay_map(overlay, "predicate", &mut predicates);
        let raw = crate::item_defaults::read(root)?;
        let items = raw
            .iter()
            .map(|(name, c)| {
                (
                    name.clone(),
                    ItemFacts {
                        max_stack: c["minecraft:max_stack_size"].as_i64().unwrap_or(64) as i32,
                        max_damage: c["minecraft:max_damage"].as_i64().unwrap_or(0) as i32,
                        enchantable: c["minecraft:enchantable"]["value"]
                            .as_i64()
                            .map(|v| v as i32),
                    },
                )
            })
            .collect();
        let mut raw_enchantments = HashMap::new();
        load_dir(
            &data.join("enchantment"),
            "minecraft",
            "",
            &mut raw_enchantments,
        )?;
        let mut names: Vec<&String> = raw_enchantments.keys().collect();
        names.sort();
        let enchantments = names
            .into_iter()
            .map(|name| {
                let e = &raw_enchantments[name];
                Enchantment {
                    name: name.clone(),
                    supported: e["supported_items"].clone(),
                    primary: e.get("primary_items").cloned(),
                    exclusive: e.get("exclusive_set").cloned().unwrap_or(Value::Null),
                    max_level: e["max_level"].as_i64().unwrap_or(1) as i32,
                    weight: e["weight"].as_i64().unwrap_or(1) as i32,
                    min_cost: cost(&e["min_cost"]),
                    max_cost: cost(&e["max_cost"]),
                    slots: e["slots"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect(),
                    effects: e["effects"].clone(),
                }
            })
            .collect();
        let mut tags = HashMap::new();
        for registry in [
            "item",
            "enchantment",
            "instrument",
            "entity_type",
            "damage_type",
        ] {
            let mut raw = HashMap::new();
            load_kind(&all, &format!("tags/{registry}"), &mut raw)?;
            for (name, value) in raw {
                let values = value["values"].as_array().cloned().unwrap_or_default();
                tags.insert((registry.to_owned(), name), values);
            }
            // The packs' entries join the game's own, in pack order; a
            // `replace` file drops what the lower sources put in
            // (`TagLoader.load` walks the resource stack the same way).
            for (id, files) in overlay.tag_kind(registry) {
                let slot = tags
                    .entry((registry.to_owned(), id.to_owned()))
                    .or_default();
                for bytes in files {
                    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
                        continue;
                    };
                    if v["replace"].as_bool() == Some(true) {
                        slot.clear();
                    }
                    slot.extend(v["values"].as_array().cloned().unwrap_or_default());
                }
            }
        }
        let mut recipes = HashMap::new();
        load_kind(&all, "recipe", &mut recipes)?;
        overlay_map(overlay, "recipe", &mut recipes);
        let mut ids: Vec<&String> = recipes.keys().collect();
        ids.sort();
        let smelting = ids
            .into_iter()
            .map(|id| &recipes[id])
            .filter(|r| r["type"] == "minecraft:smelting")
            .filter_map(|r| {
                let result = recipe_result_id(&r["result"])?.to_owned();
                let count = r["result"]["count"].as_i64().unwrap_or(1) as i32;
                Some((r["ingredient"].clone(), result, count))
            })
            .collect();
        Ok(Self {
            tables,
            modifiers,
            predicates,
            items,
            enchantments,
            tags,
            smelting,
        })
    }
    pub fn has_table(&self, table: &str) -> bool {
        self.tables.contains_key(table)
    }
    /// The table's `random_sequence` (where `getRandomItems` draws from).
    pub fn random_sequence(&self, table: &str) -> Option<&str> {
        self.tables.get(table)?["random_sequence"].as_str()
    }
    pub fn max_stack(&self, item: &str) -> i32 {
        self.facts(item).max_stack
    }
    pub fn max_damage(&self, item: &str) -> i32 {
        self.facts(item).max_damage
    }
    /// Whether a registry tag (`entity_type`, `damage_type`, `item`...) holds `id`.
    pub fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.tag(registry, tag).iter().any(|m| m == id)
    }
    /// `EnchantmentHelper.getEnchantmentLevel(enchantment, entity)`: the best
    /// level on the items in the enchantment's slots.
    pub fn enchantment_level(&self, enchantment: &str, entity: &LootEntity) -> i32 {
        let Some(e) = self.enchantments.iter().find(|e| e.name == enchantment) else {
            return 0;
        };
        if !entity.living {
            return 0;
        }
        EQUIPMENT_SLOTS
            .iter()
            .filter(|slot| e.slots.iter().any(|g| slot_in_group(slot, g)))
            .filter_map(|slot| entity.item(slot))
            .map(|stack| item_level(stack, enchantment))
            .max()
            .unwrap_or(0)
    }
    /// Whether an item carries an enchantment with `prevent_equipment_drop`.
    pub fn prevents_equipment_drop(&self, stack: &LootStack) -> bool {
        stack.enchantments.iter().any(|(name, _)| {
            self.enchantments.iter().any(|e| {
                &e.name == name && !e.effects["minecraft:prevent_equipment_drop"].is_null()
            })
        })
    }
    /// `EnchantmentHelper.processEquipmentDropChance`: the victim's and the
    /// attacker's `equipment_drops` effects (Looting) applied to `chance`.
    pub fn equipment_drop_chance(&self, params: &EntityLootParams, chance: f32) -> f32 {
        // Enchantment.damageContext: no luck and no killing player.
        let damage_params = EntityLootParams {
            this: params.this.clone(),
            damage: params.damage.clone(),
            player: None,
            luck: 0.0,
        };
        let mut c = Context {
            data: self,
            random: LegacyRandom::new(0),
            biome: None,
            visited: Vec::new(),
            luck: 0.0,
            open_water: None,
            entity: Some(&damage_params),
        };
        let mut chance = chance;
        c.equipment_drops(&params.this, "victim", &mut chance);
        if let Some(attacker) = params
            .damage
            .as_ref()
            .and_then(|d| d.source.as_ref())
            .filter(|a| a.living)
        {
            c.equipment_drops(attacker, "attacker", &mut chance);
        }
        chance
    }
    /// Tag members in tag order (nested tags expanded in place, duplicates
    /// dropped).
    fn tag(&self, registry: &str, name: &str) -> Vec<String> {
        fn visit(data: &LootData, registry: &str, name: &str, out: &mut Vec<String>, depth: u32) {
            if depth > 16 {
                return;
            }
            let Some(values) = data.tags.get(&(registry.to_owned(), name.to_owned())) else {
                return;
            };
            for v in values {
                let id = match v {
                    Value::String(s) => s.as_str(),
                    Value::Object(o) => o.get("id").and_then(Value::as_str).unwrap_or(""),
                    _ => continue,
                };
                match id.strip_prefix('#') {
                    Some(tag) => visit(data, registry, tag, out, depth + 1),
                    None if !out.iter().any(|x| x == id) => out.push(id.to_owned()),
                    None => {}
                }
            }
        }
        let mut out = Vec::new();
        visit(self, registry, name, &mut out, 0);
        out
    }
    /// A HolderSet: "#tag", "id" or a list of ids.
    fn holder_set(&self, registry: &str, v: &Value) -> Vec<String> {
        match v {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => self.tag(registry, tag),
                None => vec![s.clone()],
            },
            Value::Array(a) => a
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    }
    fn item_in(&self, item: &str, set: &Value) -> bool {
        self.holder_set("item", set).iter().any(|i| i == item)
    }
    fn facts(&self, item: &str) -> ItemFacts {
        self.items.get(item).copied().unwrap_or(ItemFacts {
            max_stack: 64,
            ..ItemFacts::default()
        })
    }

    /// LootTable.getRandomItems with the given random (its state is
    /// advanced), luck and, for fishing, whether the hook is in open water.
    pub fn roll(
        &self,
        table: &str,
        random: &mut LegacyRandom,
        luck: f32,
        biome: Option<&str>,
        open_water: Option<bool>,
    ) -> Vec<LootStack> {
        let Some(json) = self.tables.get(table) else {
            return Vec::new();
        };
        let mut c = Context {
            data: self,
            random: random.clone(),
            biome,
            visited: Vec::new(),
            luck,
            open_water,
            entity: None,
        };
        let mut items = Vec::new();
        c.table_raw(table, json, &mut |_, stack| items.push(stack));
        *random = c.random;
        items
    }
    /// LootTable.getRandomItems for an entity context: the generated stacks
    /// after the stack splitter, in order. `random` is the table's random
    /// sequence (or the level random) and is advanced.
    pub fn roll_entity<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
        params: &EntityLootParams,
    ) -> Vec<LootStack> {
        let Some(json) = self.tables.get(table) else {
            return Vec::new();
        };
        let mut c = Context {
            data: self,
            random: random.clone(),
            biome: None,
            visited: Vec::new(),
            luck: params.luck,
            open_water: None,
            entity: Some(params),
        };
        let mut items = Vec::new();
        c.table_raw(table, json, &mut |c, stack| c.data.split(stack, &mut items));
        *random = c.random;
        items
    }
    /// LootTable.getRandomItems with an empty context (a trial spawner's
    /// reward): the stacks after the stack splitter. `random` is the table's
    /// random sequence (or the level random) and is advanced.
    pub fn roll_split<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
    ) -> Vec<LootStack> {
        let Some(json) = self.tables.get(table) else {
            return Vec::new();
        };
        let mut c = Context {
            data: self,
            random: random.clone(),
            biome: None,
            visited: Vec::new(),
            luck: 0.0,
            open_water: None,
            entity: None,
        };
        let mut items = Vec::new();
        c.table_raw(table, json, &mut |c, stack| c.data.split(stack, &mut items));
        *random = c.random;
        items
    }
    /// LootTable.createStackSplitter (every item is enabled).
    fn split(&self, stack: LootStack, items: &mut Vec<LootStack>) {
        let max = self.facts(&stack.item).max_stack.max(1);
        if stack.count < max {
            items.push(stack);
        } else {
            let mut count = stack.count;
            while count > 0 {
                let mut copy = stack.clone();
                copy.count = max.min(count);
                count -= copy.count;
                items.push(copy);
            }
        }
    }
    /// LootTable.fill into an empty container of `slots` slots with
    /// `RandomSource.create(seed)`; `biome` is the container's biome for
    /// location checks (None: no origin, checks fail).
    pub fn fill(
        &self,
        table: &str,
        seed: i64,
        slots: usize,
        biome: Option<&str>,
    ) -> Vec<Option<LootStack>> {
        self.fill_with(table, &mut LegacyRandom::new(seed), slots, biome)
    }
    /// LootTable.fill drawing from `random` (a seeded random or the table's
    /// random sequence), which is advanced.
    pub fn fill_with<R: RandomSource + Clone>(
        &self,
        table: &str,
        random: &mut R,
        slots: usize,
        biome: Option<&str>,
    ) -> Vec<Option<LootStack>> {
        let mut out = vec![None; slots];
        let Some(json) = self.tables.get(table) else {
            return out;
        };
        let mut c = Context {
            data: self,
            random: random.clone(),
            biome,
            visited: Vec::new(),
            luck: 0.0,
            open_water: None,
            entity: None,
        };
        let mut items = Vec::new();
        c.table_raw(table, json, &mut |c, stack| c.data.split(stack, &mut items));
        *random = c.random;
        let mut available: Vec<usize> = (0..slots).collect();
        shuffle(&mut available, random);
        shuffle_and_split(&mut items, available.len(), random);
        for stack in items {
            let Some(slot) = available.pop() else { break };
            out[slot] = (!stack.is_empty()).then_some(stack);
        }
        out
    }
}

/// `MobEffect.isInstantaneous`.
fn is_instantaneous(effect: &str) -> bool {
    matches!(
        effect,
        "minecraft:saturation" | "minecraft:instant_health" | "minecraft:instant_damage"
    )
}
/// The level of one enchantment on a stack (its `enchantments` component).
fn item_level(stack: &LootStack, enchantment: &str) -> i32 {
    stack
        .enchantments
        .iter()
        .find(|(n, _)| n == enchantment)
        .map_or(0, |(_, l)| *l)
}
/// Util.shuffle.
fn shuffle<T>(list: &mut [T], random: &mut impl RandomSource) {
    for i in (2..=list.len()).rev() {
        let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
        list.swap(i - 1, j);
    }
}
/// Mth.nextInt.
fn next_int(random: &mut impl RandomSource, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        random.next_int_bound(max - min + 1).unwrap_or(0) + min
    }
}
/// LootTable.shuffleAndSplitItems.
fn shuffle_and_split(result: &mut Vec<LootStack>, slots: usize, random: &mut impl RandomSource) {
    let mut splittable = Vec::new();
    result.retain(|s| {
        if s.is_empty() {
            false
        } else if s.count > 1 {
            splittable.push(s.clone());
            false
        } else {
            true
        }
    });
    while slots as i64 - result.len() as i64 - splittable.len() as i64 > 0 && !splittable.is_empty()
    {
        let index = next_int(random, 0, splittable.len() as i32 - 1) as usize;
        let mut stack = splittable.remove(index);
        let remove = next_int(random, 1, stack.count / 2).min(stack.count);
        let mut copy = stack.clone();
        copy.count = remove;
        stack.count -= remove;
        if stack.count > 1 && random.next_boolean() {
            splittable.push(stack);
        } else {
            result.push(stack);
        }
        if copy.count > 1 && random.next_boolean() {
            splittable.push(copy);
        } else {
            result.push(copy);
        }
    }
    result.append(&mut splittable);
    shuffle(result, random);
}

struct Context<'a, R> {
    data: &'a LootData,
    random: R,
    biome: Option<&'a str>,
    visited: Vec<String>,
    /// LootContext.getLuck.
    luck: f32,
    /// A fishing hook's `is_open_water` for `entity_properties` (None: no entity).
    open_water: Option<bool>,
    /// The entity, damage source and killer parameters.
    entity: Option<&'a EntityLootParams>,
}
type Sink<'s, R> = dyn FnMut(&mut Context<'_, R>, LootStack) + 's;

impl<R: RandomSource> Context<'_, R> {
    /// A `LootContext.EntityTarget` parameter.
    fn target(&self, name: &str) -> Option<&LootEntity> {
        let p = self.entity?;
        match name {
            "this" => Some(&p.this),
            "attacker" => p.damage.as_ref()?.source.as_ref(),
            "direct_attacker" => p.damage.as_ref()?.direct.as_ref(),
            "attacking_player" => p.player.as_ref(),
            _ => None,
        }
    }
    /// The `ATTACKING_ENTITY` parameter when it is a living entity.
    fn living_attacker(&self) -> Option<&LootEntity> {
        self.target("attacker").filter(|a| a.living)
    }
    /// The `equipment_drops` effects on one entity's equipment
    /// (`runIterationOnEquipment`), for the given `enchanted` target.
    fn equipment_drops(&mut self, holder: &LootEntity, enchanted: &str, chance: &mut f32) {
        let data = self.data;
        for slot in EQUIPMENT_SLOTS {
            let Some(stack) = holder.item(slot) else {
                continue;
            };
            for (name, level) in &stack.enchantments {
                let Some(e) = data.enchantments.iter().find(|e| &e.name == name) else {
                    continue;
                };
                if !e.slots.iter().any(|g| slot_in_group(slot, g)) {
                    continue;
                }
                for effect in e.effects["minecraft:equipment_drops"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if effect["enchanted"].as_str() != Some(enchanted)
                        || effect["affected"].as_str().unwrap_or("victim") != "victim"
                    {
                        continue;
                    }
                    if !effect["requirements"].is_null() && !self.condition(&effect["requirements"])
                    {
                        continue;
                    }
                    let value = level_value(&effect["effect"]["value"], *level);
                    *chance = match effect["effect"]["type"].as_str().unwrap_or("") {
                        "minecraft:add" => *chance + value,
                        "minecraft:multiply" => *chance * value,
                        "minecraft:set" => value,
                        _ => *chance,
                    };
                }
            }
        }
    }
    /// EntityPredicate.matches (false without an entity).
    fn entity_matches(&self, p: &Value, e: Option<&LootEntity>) -> bool {
        let Some(e) = e else { return false };
        let Some(parts) = p.as_object() else {
            return true;
        };
        let data = self.data;
        parts.iter().all(|(key, v)| match key.as_str() {
            "minecraft:entity_type" => match v.as_str() {
                Some(s) => match s.strip_prefix('#') {
                    Some(tag) => data.in_tag("entity_type", tag, &e.kind),
                    None => s == e.kind,
                },
                None => v
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(&e.kind))),
            },
            "minecraft:flags" => v.as_object().into_iter().flatten().all(|(flag, want)| {
                let want = want.as_bool().unwrap_or(false);
                match flag.as_str() {
                    "is_on_fire" => e.on_fire == want,
                    // EntityFlagsPredicate: only living entities have an age.
                    "is_baby" => !e.living || e.baby == want,
                    "is_fall_flying" => !e.living || !want,
                    _ => !want,
                }
            }),
            "minecraft:equipment" => {
                e.living
                    && v.as_object().into_iter().flatten().all(|(slot, item)| {
                        e.item(slot)
                            .is_some_and(|stack| self.item_matches(item, stack))
                    })
            }
            "minecraft:components" => v.as_object().into_iter().flatten().all(|(c, want)| match c
                .as_str()
            {
                "minecraft:sheep/color" => e
                    .sheep
                    .as_ref()
                    .is_some_and(|(color, _)| want.as_str() == Some(color)),
                "minecraft:frog/variant" => {
                    e.frog_variant.is_some() && want.as_str() == e.frog_variant.as_deref()
                }
                "minecraft:chicken/variant" => {
                    e.chicken_variant.is_some() && want.as_str() == e.chicken_variant.as_deref()
                }
                _ => false,
            }),
            "minecraft:type_specific/sheep" => e
                .sheep
                .as_ref()
                .is_some_and(|(_, sheared)| v["sheared"].as_bool().is_none_or(|w| w == *sheared)),
            "minecraft:type_specific/cube_mob" => {
                e.cube_size.is_some_and(|size| int_bounds(&v["size"], size))
            }
            // RaiderPredicate: no raid is running here.
            "minecraft:type_specific/raider" => e.captain.is_some_and(|captain| {
                !v["has_raid"].as_bool().unwrap_or(false)
                    && captain == v["is_captain"].as_bool().unwrap_or(false)
            }),
            "minecraft:vehicle" => self.entity_matches(v, e.vehicle.as_deref()),
            _ => false,
        })
    }
    /// A number provider. An unknown type gives 0, as in items.rs `number`
    /// (a data pack's typo yields nothing rather than a full roll).
    fn int(&mut self, v: &Value) -> i32 {
        match v {
            Value::Number(_) => int_value(v).unwrap_or(0) as i32,
            Value::Object(_) => match v["type"].as_str().unwrap_or("") {
                "minecraft:constant" => self.int(&v["value"]),
                "minecraft:uniform" => {
                    let min = self.int(&v["min"]);
                    let max = self.int(&v["max"]);
                    next_int(&mut self.random, min, max)
                }
                "minecraft:binomial" => {
                    let n = self.int(&v["n"]);
                    let p = self.float(&v["p"]);
                    (0..n).filter(|_| self.random.next_float() < p).count() as i32
                }
                _ => 0,
            },
            _ => 0,
        }
    }
    fn float(&mut self, v: &Value) -> f32 {
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.) as f32,
            Value::Object(_) => match v["type"].as_str().unwrap_or("") {
                "minecraft:constant" => self.float(&v["value"]),
                "minecraft:uniform" => {
                    let min = self.float(&v["min"]);
                    let max = self.float(&v["max"]);
                    if min >= max {
                        min
                    } else {
                        self.random.next_float() * (max - min) + min
                    }
                }
                _ => 0.,
            },
            _ => 0.,
        }
    }
    fn condition(&mut self, c: &Value) -> bool {
        let data = self.data;
        match c {
            Value::Null => true,
            Value::Array(list) => list.iter().all(|c| self.condition(c)),
            Value::String(id) => match data.predicates.get(id) {
                Some(p) => self.condition(p),
                None => false,
            },
            Value::Object(_) => match c["type"].as_str().unwrap_or("") {
                "minecraft:inverted" => !self.condition(&c["term"]),
                "minecraft:all_of" => c["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|t| self.condition(t)),
                "minecraft:any_of" => c["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|t| self.condition(t)),
                "minecraft:entity_properties" if self.entity.is_some() => {
                    let target = self.target(c["entity"].as_str().unwrap_or("this"));
                    c["predicate"].is_null() || self.entity_matches(&c["predicate"], target)
                }
                // FishingHookPredicate: the only entity property a hook has.
                "minecraft:entity_properties" => {
                    let predicate = &c["predicate"];
                    let fishing = &predicate["minecraft:type_specific/fishing_hook"];
                    let only_hook = predicate.as_object().is_some_and(|o| o.len() == 1);
                    match self.open_water {
                        Some(open) if only_hook && fishing.is_object() => fishing["in_open_water"]
                            .as_bool()
                            .is_none_or(|want| want == open),
                        _ => false,
                    }
                }
                "minecraft:killed_by_player" => self.entity.is_some_and(|p| p.player.is_some()),
                "minecraft:random_chance_with_enchanted_bonus" => {
                    let level = self.living_attacker().map_or(0, |a| {
                        data.enchantment_level(c["enchantment"].as_str().unwrap_or(""), a)
                    });
                    let chance = if level > 0 {
                        level_value(&c["enchanted_chance"], level)
                    } else {
                        c["unenchanted_chance"].as_f64().unwrap_or(0.) as f32
                    };
                    self.random.next_float() < chance
                }
                // DamageSourcePredicate (false without a damage source).
                "minecraft:damage_source_properties" => {
                    let Some(damage) = self.entity.and_then(|p| p.damage.as_ref()) else {
                        return false;
                    };
                    let p = &c["predicate"];
                    let tags_ok = p["tags"].as_array().into_iter().flatten().all(|t| {
                        let id = t["id"].as_str().unwrap_or("");
                        let has = match id.strip_prefix('#') {
                            Some(tag) => data.in_tag("damage_type", tag, &damage.kind),
                            None => id == damage.kind,
                        };
                        has == t["expected"].as_bool().unwrap_or(false)
                    });
                    tags_ok
                        && (p["direct_entity"].is_null()
                            || self.entity_matches(&p["direct_entity"], damage.direct.as_ref()))
                        && (p["source_entity"].is_null()
                            || self.entity_matches(&p["source_entity"], damage.source.as_ref()))
                        && p["is_direct"].as_bool().is_none_or(|want| {
                            let direct = match (&damage.source, &damage.direct) {
                                (Some(a), Some(b)) => a.kind == b.kind,
                                (None, None) => true,
                                _ => false,
                            };
                            direct == want
                        })
                }
                "minecraft:random_chance" => {
                    let chance = self.float(&c["chance"]);
                    self.random.next_float() < chance
                }
                "minecraft:location_check" => {
                    let Some(biome) = self.biome else {
                        return false;
                    };
                    match &c["predicate"]["biomes"] {
                        Value::Null => true,
                        Value::String(s) => match s.strip_prefix('#') {
                            // Biome tags are not loaded here.
                            Some(_) => false,
                            None => s == biome,
                        },
                        Value::Array(a) => a.iter().any(|b| b.as_str() == Some(biome)),
                        _ => false,
                    }
                }
                // A mod's own kind: the hook answers; a bare type name is
                // minecraft's own and an unknown one fails.
                k if k.contains(':') && !k.starts_with("minecraft:") => {
                    let mut holds = false;
                    native_loot_condition(k, c, &self.native_ctx(), &mut holds);
                    holds
                }
                // An unknown type fails, as in items.rs `condition`.
                _ => false,
            },
            _ => false,
        }
    }
    /// An item modifier (typed object, inline list or registry reference).
    fn modify(&mut self, m: &Value, stack: LootStack) -> LootStack {
        let data = self.data;
        match m {
            Value::Null => stack,
            Value::Array(list) => list.iter().fold(stack, |s, f| self.modify(f, s)),
            Value::String(id) => match data.modifiers.get(id) {
                Some(f) => self.modify(f, stack),
                None => stack,
            },
            Value::Object(_) => {
                if !m["condition"].is_null() && !self.condition(&m["condition"]) {
                    return stack;
                }
                self.run(m, stack)
            }
            _ => stack,
        }
    }
    /// The context a mod's own loot kinds see: drawn lazily, only when
    /// the table has a mod's kind, so the random stays untouched without.
    fn native_ctx(&mut self) -> NativeLootContext<'_> {
        let seed = self.random.next_long();
        NativeLootContext {
            entity: self.entity.map(|e| &e.this),
            damage: self.entity.and_then(|e| e.damage.as_ref()),
            player: self.entity.and_then(|e| e.player.as_ref()),
            tool: None, // The tool is not in this engine's context.
            block: None,
            luck: self.luck,
            biome: self.biome,
            seed,
            place: current_native_loot_place(),
        }
    }
    fn run(&mut self, f: &Value, mut stack: LootStack) -> LootStack {
        let data = self.data;
        match f["type"].as_str().unwrap_or("") {
            "minecraft:sequence" => {
                for function in f["functions"].as_array().into_iter().flatten() {
                    stack = self.modify(function, stack);
                }
            }
            "minecraft:set_count" => {
                let base = if f["add"].as_bool() == Some(true) {
                    stack.get_count()
                } else {
                    0
                };
                stack.count = base + self.int(&f["count"]);
            }
            "minecraft:limit_count" => {
                let limit = &f["limit"];
                let mut count = stack.get_count();
                if let Some(min) = int_value(&limit["min"]) {
                    count = count.max(min as i32);
                }
                if let Some(max) = int_value(&limit["max"]) {
                    count = count.min(max as i32);
                }
                stack.count = count;
            }
            "minecraft:set_damage" => {
                let max = data.facts(&stack.item).max_damage;
                if max > 0 {
                    let base = if f["add"].as_bool() == Some(true) {
                        1. - stack.damage as f32 / max as f32
                    } else {
                        0.
                    };
                    let pct = 1. - (self.float(&f["damage"]) + base).clamp(0., 1.);
                    stack.damage = (pct * max as f32).floor() as i32;
                }
            }
            "minecraft:enchant_randomly" => {
                let book = stack.item == "minecraft:book";
                let check = !book && f["only_compatible"].as_bool().unwrap_or(true);
                let options: Vec<usize> = match f.get("options") {
                    Some(o) => data
                        .holder_set("enchantment", o)
                        .iter()
                        .filter_map(|n| data.enchantments.iter().position(|e| &e.name == n))
                        .collect(),
                    None => (0..data.enchantments.len()).collect(),
                };
                let options: Vec<usize> = options
                    .into_iter()
                    .filter(|&i| {
                        !check || data.item_in(&stack.item, &data.enchantments[i].supported)
                    })
                    .collect();
                if !options.is_empty() {
                    let pick = self
                        .random
                        .next_int_bound(options.len() as i32)
                        .unwrap_or(0) as usize;
                    let e = &data.enchantments[options[pick]];
                    let level = next_int(&mut self.random, 1, e.max_level);
                    if book {
                        stack.item = "minecraft:enchanted_book".into();
                    }
                    enchant(&mut stack, &e.name, level);
                }
            }
            "minecraft:enchant_with_levels" => {
                let cost = self.int(&f["levels"]);
                let source: Vec<usize> = match f.get("options") {
                    Some(o) => data
                        .holder_set("enchantment", o)
                        .iter()
                        .filter_map(|n| data.enchantments.iter().position(|e| &e.name == n))
                        .collect(),
                    None => (0..data.enchantments.len()).collect(),
                };
                let chosen = self.select_enchantments(&stack.item, cost, &source);
                if stack.item == "minecraft:book" {
                    stack.item = "minecraft:enchanted_book".into();
                }
                for (i, level) in chosen {
                    enchant(&mut stack, &data.enchantments[i].name, level);
                }
            }
            "minecraft:set_enchantments" => {
                if let Some(map) = f["enchantments"].as_object() {
                    if stack.item == "minecraft:book" {
                        stack.item = "minecraft:enchanted_book".into();
                    }
                    for (name, level) in map {
                        let level = self.int(level);
                        enchant(&mut stack, name, level);
                    }
                }
            }
            "minecraft:set_potion" => stack.potion = f["id"].as_str().map(str::to_owned),
            "minecraft:set_stew_effect" => {
                let effects = f["effects"].as_array().cloned().unwrap_or_default();
                if stack.item == "minecraft:suspicious_stew" && !effects.is_empty() {
                    let pick = self
                        .random
                        .next_int_bound(effects.len() as i32)
                        .unwrap_or(0) as usize;
                    let effect = effects[pick]["type"].as_str().unwrap_or("");
                    let mut duration = self.int(&effects[pick]["duration"]);
                    // Seconds, unless the effect is instantaneous.
                    if !is_instantaneous(effect) {
                        duration *= 20;
                    }
                    // SuspiciousStewEffects.withEffectAdded
                    stack.stew_effects.push((effect.to_owned(), duration));
                }
            }
            "minecraft:set_ominous_bottle_amplifier" => {
                stack.ominous_amplifier = Some(self.int(&f["amplifier"]));
            }
            // SetInstrumentFunction: `options.getRandomElement(random)` becomes the item's
            // `instrument` component (a goat horn's tune).
            "minecraft:set_instrument" => {
                let options = data.holder_set("instrument", &f["options"]);
                if !options.is_empty() {
                    let pick = self
                        .random
                        .next_int_bound(options.len() as i32)
                        .unwrap_or(0) as usize;
                    stack
                        .components
                        .retain(|(k, _)| k != "minecraft:instrument");
                    stack.components.push((
                        "minecraft:instrument".to_owned(),
                        Value::String(options[pick].clone()),
                    ));
                }
            }
            "minecraft:set_item" => {
                if let Some(item) = f["item"].as_str() {
                    stack.item = item.to_owned();
                }
            }
            // ExplorationMapFunction needs an origin; with one, the map is
            // taken as found here and the game runs the structure search
            // (it drops the map when nothing is found, as the filtered
            // map_id check after it would).
            "minecraft:exploration_map" => {
                stack.map = self.biome.is_some();
                if stack.map {
                    stack.exploration = Some(Box::new(ExplorationMap::from_json(f)));
                }
            }
            "minecraft:filtered" => {
                let pass = self.item_matches(&f["item_filter"], &stack);
                let branch = if pass { &f["on_pass"] } else { &f["on_fail"] };
                stack = self.modify(branch, stack);
            }
            "minecraft:discard" => stack.count = 0,
            "minecraft:enchanted_count_increase" => {
                if let Some(attacker) = self.living_attacker() {
                    let level =
                        data.enchantment_level(f["enchantment"].as_str().unwrap_or(""), attacker);
                    if level != 0 {
                        let addition = level as f32 * self.float(&f["count"]);
                        // ItemStack.grow(Math.round(addition)).
                        stack.count = stack.get_count() + (addition + 0.5).floor() as i32;
                        let limit = int_value(&f["limit"]).unwrap_or(0) as i32;
                        if limit > 0 && !stack.is_empty() && stack.count > limit {
                            stack.count = limit;
                        }
                    }
                }
            }
            "minecraft:furnace_smelt" => {
                if !stack.is_empty()
                    && let Some((_, result, count)) = data
                        .smelting
                        .iter()
                        .find(|(ingredient, _, _)| data.item_in(&stack.item, ingredient))
                {
                    let input = if f["use_input_count"].as_bool().unwrap_or(true) {
                        stack.count
                    } else {
                        1
                    };
                    let mut smelted = LootStack::new(result);
                    smelted.count = (input * count).min(data.facts(result).max_stack);
                    stack = smelted;
                }
            }
            // SetNameFunction: the component the target names (custom_name by
            // default). The entity-resolved selectors and scores are not
            // modelled; no table uses `entity`.
            "minecraft:set_name" if !f["name"].is_null() => {
                let target = f["target"].as_str().unwrap_or("custom_name");
                let id = format!("minecraft:{target}");
                stack.names.retain(|(k, _)| *k != id);
                stack.names.push((id, f["name"].clone()));
            }
            // SetComponentsFunction: the patch's components, applied when the
            // stack becomes an item (trim on worn armour, see `mobs::finalize`).
            "minecraft:set_components" => {
                for (id, value) in f["components"].as_object().into_iter().flatten() {
                    stack.components.retain(|(k, _)| k != id);
                    stack.components.push((id.clone(), value.clone()));
                }
            }
            // A mod's own kind: the hook changes the stack.
            k if k.contains(':') && !k.starts_with("minecraft:") => {
                native_loot_function(k, f, &self.native_ctx(), &mut stack);
            }
            // Lore and other data without random draws.
            _ => {}
        }
        stack
    }
    /// ItemPredicate subset used by filtered modifiers and equipment.
    fn item_matches(&self, p: &Value, stack: &LootStack) -> bool {
        if !p["items"].is_null() && !self.data.item_in(&stack.item, &p["items"]) {
            return false;
        }
        if !int_bounds(&p["count"], stack.count) {
            return false;
        }
        if p["predicates"].get("minecraft:map_id").is_some() && !stack.map {
            return false;
        }
        // EnchantmentsPredicate: every entry must hold.
        if let Some(list) = p["predicates"]["minecraft:enchantments"].as_array() {
            let data = self.data;
            let ok = list.iter().all(|e| {
                let levels = &e["levels"];
                match &e["enchantments"] {
                    Value::Null => stack
                        .enchantments
                        .iter()
                        .any(|(_, l)| int_bounds(levels, *l)),
                    set => data.holder_set("enchantment", set).iter().any(|name| {
                        let level = item_level(stack, name);
                        level != 0 && int_bounds(levels, level)
                    }),
                }
            });
            if !ok {
                return false;
            }
        }
        true
    }
    /// EnchantmentHelper.selectEnchantment.
    fn select_enchantments(
        &mut self,
        item: &str,
        mut cost: i32,
        source: &[usize],
    ) -> Vec<(usize, i32)> {
        let data = self.data;
        let mut results = Vec::new();
        let Some(value) = data.facts(item).enchantable else {
            return results;
        };
        cost += 1
            + self.random.next_int_bound(value / 4 + 1).unwrap_or(0)
            + self.random.next_int_bound(value / 4 + 1).unwrap_or(0);
        let span = (self.random.next_float() + self.random.next_float() - 1.) * 0.15;
        cost = ((cost as f32 + cost as f32 * span + 0.5).floor() as i32).max(1);
        let book = item == "minecraft:book";
        let mut available: Vec<(usize, i32)> = Vec::new();
        for &i in source {
            let e = &data.enchantments[i];
            let primary = data.item_in(item, &e.supported)
                && e.primary.as_ref().is_none_or(|p| data.item_in(item, p));
            if !primary && !book {
                continue;
            }
            for level in (1..=e.max_level).rev() {
                if cost >= e.min_cost(level) && cost <= e.max_cost(level) {
                    available.push((i, level));
                    break;
                }
            }
        }
        if available.is_empty() {
            return results;
        }
        let pick = |random: &mut R, list: &[(usize, i32)]| -> Option<(usize, i32)> {
            let total: i32 = list.iter().map(|&(i, _)| data.enchantments[i].weight).sum();
            if total <= 0 {
                return None;
            }
            let mut selection = random.next_int_bound(total).unwrap_or(0);
            for &entry in list {
                selection -= data.enchantments[entry.0].weight;
                if selection < 0 {
                    return Some(entry);
                }
            }
            None
        };
        results.extend(pick(&mut self.random, &available));
        while self.random.next_int_bound(50).unwrap_or(0) <= cost {
            if let Some(&(last, _)) = results.last() {
                available.retain(|&(i, _)| self.compatible(last, i));
            }
            if available.is_empty() {
                break;
            }
            results.extend(pick(&mut self.random, &available));
            cost /= 2;
        }
        results
    }
    /// Enchantment.areCompatible.
    fn compatible(&self, a: usize, b: usize) -> bool {
        let data = self.data;
        let (ea, eb) = (&data.enchantments[a], &data.enchantments[b]);
        a != b
            && !data
                .holder_set("enchantment", &ea.exclusive)
                .contains(&eb.name)
            && !data
                .holder_set("enchantment", &eb.exclusive)
                .contains(&ea.name)
    }
    /// LootTable.getRandomItemsRaw.
    fn table_raw(&mut self, name: &str, table: &Value, sink: &mut Sink<'_, R>) {
        if self.visited.iter().any(|v| v == name) {
            return;
        }
        self.visited.push(name.to_owned());
        let table_modifier = &table["modifier"];
        for pool in table["pools"].as_array().into_iter().flatten() {
            if !pool["condition"].is_null() && !self.condition(&pool["condition"]) {
                continue;
            }
            let bonus = if pool["bonus_rolls"].is_null() {
                0.0
            } else {
                self.float(&pool["bonus_rolls"])
            };
            let rolls = self.int(&pool["rolls"]) + (bonus * self.luck).floor() as i32;
            for _ in 0..rolls {
                self.add_random_item(pool, &mut |c, stack| {
                    let stack = c.modify(&pool["modifier"], stack);
                    let stack = c.modify(table_modifier, stack);
                    sink(c, stack);
                });
            }
        }
        self.visited.pop();
    }
    /// LootPool.addRandomItem.
    fn add_random_item(&mut self, pool: &Value, sink: &mut Sink<'_, R>) {
        let mut valid: Vec<Choice<'_>> = Vec::new();
        for entry in pool["entries"].as_array().into_iter().flatten() {
            self.expand(entry, &mut valid);
        }
        // EntryBase.getWeight: max(floor(weight + quality * luck), 0).
        let luck = self.luck;
        let weight = |c: &Choice<'_>| {
            let e = c.entry;
            let w = int_value(&e["weight"]).unwrap_or(1) as f32
                + int_value(&e["quality"]).unwrap_or(0) as f32 * luck;
            w.floor().max(0.0) as i32
        };
        let total: i32 = valid.iter().map(&weight).sum();
        if total == 0 || valid.is_empty() {
            return;
        }
        let chosen = if valid.len() == 1 {
            valid.swap_remove(0)
        } else {
            let mut index = self.random.next_int_bound(total).unwrap_or(0);
            let Some(at) = valid.iter().position(|e| {
                index -= weight(e);
                index < 0
            }) else {
                return;
            };
            valid.swap_remove(at)
        };
        self.create(&chosen, sink);
    }
    /// LootPoolEntryContainer.expand: the singleton entries that can run.
    fn expand<'v>(&mut self, entry: &'v Value, out: &mut Vec<Choice<'v>>) -> bool {
        if !entry["condition"].is_null() && !self.condition(&entry["condition"]) {
            return false;
        }
        let children = || entry["children"].as_array().into_iter().flatten();
        match entry["type"].as_str().unwrap_or("") {
            "minecraft:alternatives" => children().any(|c| self.expand(c, out)),
            "minecraft:group" => {
                for c in children() {
                    self.expand(c, out);
                }
                true
            }
            "minecraft:sequence" => {
                for c in children() {
                    if !self.expand(c, out) {
                        return false;
                    }
                }
                true
            }
            "minecraft:item" | "minecraft:empty" | "minecraft:loot_table" => {
                if int_value(&entry["weight"]).unwrap_or(1) > 0 {
                    out.push(Choice { entry, item: None });
                }
                true
            }
            // TagEntry: one choice per item when expanded, else one for all.
            "minecraft:tag" => {
                if entry["expand"].as_bool().unwrap_or(false) {
                    let items = self.data.holder_set("item", &entry["items"]);
                    for item in items {
                        if int_value(&entry["weight"]).unwrap_or(1) > 0 {
                            out.push(Choice {
                                entry,
                                item: Some(item),
                            });
                        }
                    }
                    true
                } else {
                    if int_value(&entry["weight"]).unwrap_or(1) > 0 {
                        out.push(Choice { entry, item: None });
                    }
                    true
                }
            }
            _ => false,
        }
    }
    /// LootPoolEntry.createItemStack with the entry's modifier.
    fn create(&mut self, choice: &Choice<'_>, sink: &mut Sink<'_, R>) {
        let entry = choice.entry;
        let modifier = &entry["modifier"];
        let mut out = |c: &mut Context<'_, R>, stack: LootStack| {
            let stack = c.modify(modifier, stack);
            sink(c, stack);
        };
        match entry["type"].as_str().unwrap_or("") {
            "minecraft:item" => {
                let stack = LootStack::new(entry["name"].as_str().unwrap_or("minecraft:air"));
                out(self, stack);
            }
            "minecraft:tag" => match &choice.item {
                Some(item) => out(self, LootStack::new(item)),
                None => {
                    for item in self.data.holder_set("item", &entry["items"]) {
                        out(self, LootStack::new(&item));
                    }
                }
            },
            "minecraft:loot_table" => match &entry["value"] {
                Value::String(name) => {
                    if let Some(table) = self.data.tables.get(name) {
                        self.table_raw(name, table, &mut out);
                    }
                }
                inline @ Value::Object(_) => self.table_raw("<inline>", inline, &mut out),
                _ => {}
            },
            _ => {}
        }
    }
}
/// A pool entry that can be chosen (an expanded tag entry names its item).
struct Choice<'v> {
    entry: &'v Value,
    item: Option<String>,
}
fn enchant(stack: &mut LootStack, name: &str, level: i32) {
    match stack.enchantments.iter_mut().find(|(n, _)| n == name) {
        Some(slot) => slot.1 = level,
        None => stack.enchantments.push((name.to_owned(), level)),
    }
}

#[cfg(test)]
mod namespace_tests {
    use super::*;

    /// The same root and packs share one map while it is held; other packs
    /// get their own.
    #[test]
    fn loot_tables_are_shared_per_root_and_packs() {
        let root = std::env::temp_dir().join(format!("horizon-loot-share-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/loot_table/chests")).unwrap();
        fs::write(
            root.join("data/minecraft/loot_table/chests/a.json"),
            r#"{"pools": []}"#,
        )
        .unwrap();
        let first = shared_tables(&root, &PackOverlay::NONE).unwrap();
        let again = shared_tables(&root, &PackOverlay::NONE).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert!(first.contains_key("minecraft:chests/a"));
        let files = [(
            "loot_table/minecraft:chests/b".to_owned(),
            vec![br#"{"pools": []}"#.to_vec()],
        )];
        let packs = shared_tables(&root, &PackOverlay { files: &files }).unwrap();
        assert!(!Arc::ptr_eq(&first, &packs));
        assert!(packs.contains_key("minecraft:chests/b"));
        // A table changed at the same path while `first` is still held: the
        // new file is read, not the held map.
        fs::write(
            root.join("data/minecraft/loot_table/chests/a.json"),
            r#"{"pools": [], "random_sequence": "minecraft:chests/a"}"#,
        )
        .unwrap();
        let changed = shared_tables(&root, &PackOverlay::NONE).unwrap();
        assert!(!Arc::ptr_eq(&first, &changed));
        assert!(
            changed["minecraft:chests/a"]
                .get("random_sequence")
                .is_some()
        );
        assert!(first["minecraft:chests/a"].get("random_sequence").is_none());
        let _ = fs::remove_dir_all(&root);
    }

    /// A table kept as its text reads back as the file's JSON (floats, big
    /// integers, nesting, key order); a file that is not JSON still fails
    /// the load.
    #[test]
    fn loot_tables_read_back_as_loaded() {
        let root = std::env::temp_dir().join(format!("horizon-loot-text-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("data/minecraft/loot_table/blocks");
        fs::create_dir_all(&dir).unwrap();
        let table = r#"{"type": "minecraft:block", "pools": [{"rolls": 1.5,
            "bonus_rolls": 0.1, "entries": [{"type": "minecraft:item", "name": "minecraft:stone",
            "weight": 9007199254740993, "functions": [{"function": "minecraft:set_count",
            "count": {"type": "minecraft:uniform", "min": -2.25e-7, "max": 1e300}}]}]}],
            "z": null, "a": [true, false, "\u00e9\n"]}"#;
        fs::write(dir.join("stone.json"), table).unwrap();
        let tables = shared_tables(&root, &PackOverlay::NONE).unwrap();
        let expected: Value = serde_json::from_str(table).unwrap();
        assert_eq!(tables.get("minecraft:blocks/stone"), Some(&expected));
        assert_eq!(tables.len(), 1);
        assert!(tables.get("minecraft:blocks/dirt").is_none());
        drop(tables);
        fs::write(dir.join("dirt.json"), r#"{"pools": [}"#).unwrap();
        assert!(shared_tables(&root, &PackOverlay::NONE).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    /// A content mods' root: the mods' loot tables and tags load beside
    /// vanilla's, under their own namespaces.
    #[test]
    fn loads_every_namespace() {
        let root = std::env::temp_dir().join(format!("horizon-loot-ns-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let files = [
            ("items.json", r#"{"minecraft:bread": {}}"#),
            (
                "data/minecraft/loot_table/chests/village.json",
                r#"{"pools": []}"#,
            ),
            (
                "data/kaisyn/loot_table/village/forest/house.json",
                r#"{"pools": []}"#,
            ),
            (
                "data/kaisyn/tags/item/food.json",
                r#"{"values": ["minecraft:bread"]}"#,
            ),
        ];
        for (name, text) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let data = LootData::load(&root, &PackOverlay::NONE).unwrap();
        assert!(data.has_table("minecraft:chests/village"));
        assert!(data.has_table("kaisyn:village/forest/house"));
        assert!(!data.has_table("minecraft:village/forest/house"));
        assert!(data.in_tag("item", "kaisyn:food", "minecraft:bread"));
        // Rolls and weights written as floats (Towns and Towers' tables).
        let table = r#"{"pools": [{"rolls": {"type": "minecraft:uniform", "min": 3.0, "max": 3.0},
            "entries": [{"type": "minecraft:item", "name": "minecraft:bread", "weight": 2.0}]}]}"#;
        fs::write(
            root.join("data/kaisyn/loot_table/village/forest/house.json"),
            table,
        )
        .unwrap();
        let data = LootData::load(&root, &PackOverlay::NONE).unwrap();
        let filled = data.fill("kaisyn:village/forest/house", 1, 27, None);
        let count: i32 = filled.iter().flatten().map(|s| s.count).sum();
        assert_eq!(count, 3);
        let _ = fs::remove_dir_all(&root);
    }

    /// `SetInstrumentFunction`: a goat horn from a loot table gets one of the options as its instrument.
    #[test]
    fn set_instrument_picks_one_of_its_options() {
        let root = std::env::temp_dir().join(format!("horizon-loot-horn-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let table = r##"{"pools": [{"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:goat_horn",
            "modifier": [{"type": "minecraft:set_instrument", "options": ["minecraft:ponder_goat_horn", "minecraft:sing_goat_horn"]}]}]}]}"##;
        let files = [
            ("items.json", r#"{"minecraft:goat_horn": {}}"#),
            ("data/minecraft/loot_table/chests/horn.json", table),
        ];
        for (name, text) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let data = LootData::load(&root, &PackOverlay::NONE).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for seed in (0..40).map(|n| n * 7_919) {
            let filled = data.fill("minecraft:chests/horn", seed, 27, None);
            let horn = filled.iter().flatten().next().expect("a horn");
            let instrument: Vec<_> = horn
                .components
                .iter()
                .filter(|(k, _)| k == "minecraft:instrument")
                .collect();
            assert_eq!(instrument.len(), 1, "{:?}", horn.components);
            seen.insert(instrument[0].1.as_str().unwrap().to_owned());
        }
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            ["minecraft:ponder_goat_horn", "minecraft:sing_goat_horn"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    // `content_id`'s tag key shape (`tags/<ns>:<registry>/<path>`) through
    // `tag_kind`: the id loses the registry segment, keeps the namespace.
    #[test]
    fn tag_kind_yields_tag_loader_ids_for_content_id_keys() {
        let overlay = PackOverlay {
            files: &[
                (
                    "tags/minecraft:item/food".to_owned(),
                    vec![br#"{"values": ["minecraft:bread"]}"#.to_vec()],
                ),
                (
                    "tags/minecraft:enchantment/trades/desert_common".to_owned(),
                    vec![br#"{"values": ["minecraft:infinity"]}"#.to_vec()],
                ),
            ],
        };
        let item: Vec<_> = overlay.tag_kind("item").map(|(id, _)| id).collect();
        assert_eq!(item, ["minecraft:food"]);
        let enchantment: Vec<_> = overlay.tag_kind("enchantment").map(|(id, _)| id).collect();
        assert_eq!(enchantment, ["minecraft:trades/desert_common"]);
        // Another registry's files stay out.
        assert_eq!(overlay.tag_kind("instrument").count(), 0);
    }

    /// A pack's tag file joins the game's own in `TagLoader` order, and its
    /// `replace` drops what the lower sources put in.
    #[test]
    fn pack_tags_join_and_replace_the_game_own() {
        let root = std::env::temp_dir().join(format!("horizon-loot-tag-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/tags/item")).unwrap();
        fs::write(root.join("items.json"), "{}").unwrap();
        fs::write(
            root.join("data/minecraft/tags/item/food.json"),
            r#"{"values": ["minecraft:bread", "minecraft:cookie"]}"#,
        )
        .unwrap();
        let overlay = PackOverlay {
            files: &[(
                "tags/minecraft:item/food".to_owned(),
                vec![
                    br#"{"values": ["minecraft:kelp"]}"#.to_vec(),
                    br#"{"replace": true, "values": ["minecraft:apple"]}"#.to_vec(),
                ],
            )],
        };
        let data = LootData::load(&root, &overlay).unwrap();
        assert!(data.in_tag("item", "minecraft:food", "minecraft:apple"));
        assert!(!data.in_tag("item", "minecraft:food", "minecraft:bread"));
        assert!(!data.in_tag("item", "minecraft:food", "minecraft:kelp"));
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod native_kinds_tests {
    use super::*;

    fn table(name: &str, json: &str) -> LootData {
        let root = std::env::temp_dir().join(format!("loot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("data/minecraft/loot_table");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.json"), json).unwrap();
        // LootData::load reads items.json too.
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let data = LootData::load(&root, &PackOverlay::NONE).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        data
    }

    /// An unclaimed condition fails (the pool drops nothing) and an
    /// unclaimed function leaves the stack's count as it was.
    #[test]
    fn unclaimed_kinds_fail_the_condition_and_leave_the_stack() {
        let data = table(
            "cond",
            r#"{"pools":[{"rolls":1,"condition":{"type":"mymod:always_true"},
                "entries":[{"type":"minecraft:item","name":"minecraft:stone"}]}]}"#,
        );
        let mut random = horizon_core::random::LegacyRandom::new(42);
        let items = data.roll("minecraft:test", &mut random, 0.0, None, None);
        assert!(items.is_empty());

        let data = table(
            "func",
            r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stone"}],
                "modifier":[{"type":"mymod:duplicate","factor":2}]}]}"#,
        );
        let mut random = horizon_core::random::LegacyRandom::new(42);
        let items = data.roll("minecraft:test", &mut random, 0.0, None, None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count, 1);
    }

    /// A table without a mod's kind doesn't shift the random; one with a
    /// kind draws exactly one `next_long`; the same seed gives the same
    /// result.
    #[test]
    fn a_mod_kind_draws_exactly_one_long_and_a_vanilla_table_none() {
        // A vanilla table: the random is untouched.
        let vanilla = table(
            "seed_vanilla",
            r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stone"}]}]}"#,
        );
        let mut r1 = horizon_core::random::LegacyRandom::new(100);
        let mut r2 = horizon_core::random::LegacyRandom::new(100);
        let _ = vanilla.roll("minecraft:test", &mut r1, 0.0, None, None);
        let expected: Vec<_> = (0..5).map(|_| r2.next_long()).collect();
        let got: Vec<_> = (0..5).map(|_| r1.next_long()).collect();
        assert_eq!(got, expected, "vanilla table shifted the random");

        // A mod kind: exactly one next_long is drawn.
        let modded = table(
            "seed_mod",
            r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stone"}],
                "modifier":[{"type":"lootmod:x"}]}]}"#,
        );
        let mut r3 = horizon_core::random::LegacyRandom::new(100);
        let mut r4 = horizon_core::random::LegacyRandom::new(100);
        let _ = modded.roll("minecraft:test", &mut r3, 0.0, None, None);
        let _ = r4.next_long(); // the one draw
        let after_mod: Vec<_> = (0..5).map(|_| r4.next_long()).collect();
        let after_run: Vec<_> = (0..5).map(|_| r3.next_long()).collect();
        assert_eq!(after_run, after_mod, "mod table drew != 1 next_long");

        // Same seed, same result.
        let mut r5 = horizon_core::random::LegacyRandom::new(42);
        let a = modded.roll("minecraft:test", &mut r5, 0.0, None, None);
        let mut r6 = horizon_core::random::LegacyRandom::new(42);
        let b = modded.roll("minecraft:test", &mut r6, 0.0, None, None);
        assert_eq!(a, b, "same seed, different result");
    }

    fn place(x: f64) -> NativeLootPlace {
        NativeLootPlace {
            origin: Some([x, 64.0, 0.5]),
            ..NativeLootPlace::default()
        }
    }

    /// A roll sees the innermost place set around it; the outer one is back
    /// after the inner returns, and nothing after the outer, even when the
    /// roll panicked.
    #[test]
    fn a_place_holds_for_its_roll_only() {
        assert_eq!(current_native_loot_place(), None);
        with_native_loot_place(place(1.5), || {
            with_native_loot_place(place(2.5), || {
                assert_eq!(
                    current_native_loot_place().unwrap().origin,
                    Some([2.5, 64.0, 0.5])
                );
            });
            assert_eq!(
                current_native_loot_place().unwrap().origin,
                Some([1.5, 64.0, 0.5])
            );
        });
        assert_eq!(current_native_loot_place(), None);

        let panicked = std::panic::catch_unwind(|| {
            with_native_loot_place(place(3.5), || panic!("a roll failed"));
        });
        assert!(panicked.is_err());
        assert_eq!(current_native_loot_place(), None);
    }

    /// A place changes nothing a table rolls: the same items, and the random
    /// left where it was, with a mod's kind in the table or not.
    #[test]
    fn a_place_leaves_the_roll_as_it_was() {
        for (name, json) in [
            (
                "place_vanilla",
                r#"{"pools":[{"rolls":{"type":"minecraft:uniform","min":1,"max":4},
                    "entries":[{"type":"minecraft:item","name":"minecraft:stone"},
                               {"type":"minecraft:item","name":"minecraft:dirt"}]}]}"#,
            ),
            (
                "place_mod",
                r#"{"pools":[{"rolls":2,"entries":[{"type":"minecraft:item","name":"minecraft:stone"}],
                    "modifier":[{"type":"lootmod:x"}]}]}"#,
            ),
        ] {
            let data = table(name, json);
            let mut plain = horizon_core::random::LegacyRandom::new(7);
            let mut placed = horizon_core::random::LegacyRandom::new(7);
            let without = data.roll("minecraft:test", &mut plain, 0.0, None, None);
            let with = with_native_loot_place(place(0.5), || {
                data.roll("minecraft:test", &mut placed, 0.0, None, None)
            });
            assert_eq!(with, without, "{name}");
            assert_eq!(placed.next_long(), plain.next_long(), "{name}");
        }
    }

    #[test]
    fn an_unknown_minecraft_function_is_skipped() {
        let data = table(
            "vanilla",
            r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stone"}],
                "modifier":[{"type":"minecraft:future_function"},{"type":"minecraft:set_count","count":5}]}]}"#,
        );
        let mut random = horizon_core::random::LegacyRandom::new(42);
        let items = data.roll("minecraft:test", &mut random, 0.0, None, None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].count, 5);
    }
}

#[cfg(test)]
mod recipe_result_tests {
    use super::*;

    /// `ItemStackTemplate.CODEC` reads a stack's fields or a bare item id.
    #[test]
    fn a_result_is_an_object_with_an_id_or_a_bare_item_id() {
        let id = |json: &str| {
            let v: Value = serde_json::from_str(json).unwrap();
            recipe_result_id(&v).map(str::to_owned)
        };
        assert_eq!(
            id(r#"{"id": "ruby:ruby", "count": 3}"#).as_deref(),
            Some("ruby:ruby")
        );
        assert_eq!(id(r#""ruby:ruby""#).as_deref(), Some("ruby:ruby"));
        // No `id` (a transmuting recipe keeps its input's item), the `item` key of older
        // versions and anything that is neither an object nor a string name no item.
        assert_eq!(id(r#"{"count": 2}"#), None);
        assert_eq!(id(r#"{"item": "minecraft:stick"}"#), None);
        assert_eq!(id(r#"{"id": 7}"#), None);
        assert_eq!(id("null"), None);
        assert_eq!(id("[]"), None);
    }
}
