//! Vanilla 26.3 block entity NBT for every block entity type.
//!
//! [`canonical`] is `loadAdditional` followed by `saveWithFullMetadata` for each of
//! the 47 `BlockEntityType`s: it reads the fields the type reads (with vanilla's
//! defaults and clamps), drops everything else and writes the fields `saveAdditional`
//! writes (including its omit-when-empty rules). Nested codec values that Horizon does
//! not simulate (spawner entity data, vibration listeners, trial spawner configuration,
//! vault data, sculk cursors, sign and banner components, profiles) are carried as
//! loaded. The common `BaseContainerBlockEntity` fields (`CustomName`, `lock`) and the
//! `RandomizableContainer` loot table fields are handled for every container type.
//!
//! [`overlay`] merges a block entity Horizon models (and re-encodes from its own,
//! partial state) with the NBT it was loaded from, so fields Horizon does not model
//! survive and only what the game changed is replaced.
use horizon_world::nbt::{Compound, Tag};

fn string(s: &str) -> Tag {
    Tag::String(s.into())
}
fn text(tag: Option<&Tag>) -> Option<String> {
    match tag {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}
/// `CompoundTag.getIntOr`: any numeric tag (floats floor).
fn int(tag: Option<&Tag>) -> Option<i32> {
    Some(match tag? {
        Tag::Byte(v) => i32::from(*v),
        Tag::Short(v) => i32::from(*v),
        Tag::Int(v) => *v,
        Tag::Long(v) => *v as i32,
        Tag::Float(b) => f32::from_bits(*b).floor() as i32,
        Tag::Double(b) => f64::from_bits(*b).floor() as i32,
        _ => return None,
    })
}
fn long(tag: Option<&Tag>) -> Option<i64> {
    Some(match tag? {
        Tag::Long(v) => *v,
        Tag::Float(b) => f32::from_bits(*b).floor() as i64,
        Tag::Double(b) => f64::from_bits(*b).floor() as i64,
        other => i64::from(int(Some(other))?),
    })
}
fn float(tag: Option<&Tag>) -> Option<f32> {
    Some(match tag? {
        Tag::Float(b) => f32::from_bits(*b),
        Tag::Double(b) => f64::from_bits(*b) as f32,
        Tag::Long(v) => *v as f32,
        other => int(Some(other))? as f32,
    })
}
fn flag(tag: Option<&Tag>) -> Option<bool> {
    Some(int(tag)? != 0)
}
fn boolean(v: bool) -> Tag {
    Tag::Byte(i8::from(v))
}
fn f32_tag(v: f32) -> Tag {
    Tag::Float(v.to_bits())
}

/// `TheEndGatewayBlockEntity.loadAdditional`: BlockPos.CODEC and Level.isInSpawnableBounds.
pub fn gateway_exit(tag: &Tag) -> Option<[i32; 3]> {
    let Tag::IntArray(v) = tag else { return None };
    let p: [i32; 3] = v.as_slice().try_into().ok()?;
    ((-30_000_000..30_000_000).contains(&p[0])
        && (-20_000_000..20_000_000).contains(&p[1])
        && (-30_000_000..30_000_000).contains(&p[2]))
    .then_some(p)
}

/// What a block entity reads and writes, in `saveAdditional` order.
struct Fields<'a> {
    input: &'a Tag,
    out: Compound,
}
impl<'a> Fields<'a> {
    fn put(&mut self, key: &str, value: Tag) {
        self.out.insert(key.into(), value);
    }
    fn int(&mut self, key: &str, default: i32) {
        let v = int(self.input.get(key)).unwrap_or(default);
        self.put(key, Tag::Int(v));
    }
    fn int_clamped(&mut self, key: &str, default: i32, min: i32, max: i32) {
        let v = int(self.input.get(key)).unwrap_or(default).clamp(min, max);
        self.put(key, Tag::Int(v));
    }
    fn short(&mut self, key: &str, default: i16) {
        let v = int(self.input.get(key)).map_or(default, |v| v as i16);
        self.put(key, Tag::Short(v));
    }
    fn long(&mut self, key: &str, default: i64) {
        let v = long(self.input.get(key)).unwrap_or(default);
        self.put(key, Tag::Long(v));
    }
    fn float(&mut self, key: &str, default: f32) {
        let v = float(self.input.get(key)).unwrap_or(default);
        self.put(key, f32_tag(v));
    }
    fn flag(&mut self, key: &str, default: bool) {
        let v = flag(self.input.get(key)).unwrap_or(default);
        self.put(key, boolean(v));
    }
    fn string(&mut self, key: &str, default: &str) {
        let v = text(self.input.get(key)).unwrap_or_else(|| default.to_owned());
        self.put(key, string(&v));
    }
    /// An optional value written as loaded, when it has the expected shape.
    fn keep(&mut self, key: &str, valid: impl Fn(&Tag) -> bool) {
        if let Some(t) = self.input.get(key).filter(|t| valid(t)) {
            self.put(key, t.clone());
        }
    }
    fn keep_compound(&mut self, key: &str) {
        self.keep(key, |t| matches!(t, Tag::Compound(_)));
    }
    /// A compound that is always written (`default` when absent).
    fn compound_or(&mut self, key: &str, default: Compound) {
        let v = match self.input.get(key) {
            Some(Tag::Compound(c)) => c.clone(),
            _ => default,
        };
        self.put(key, Tag::Compound(v));
    }
    /// An optional text component (`storeNullable("CustomName", ComponentSerialization.CODEC)`).
    fn component(&mut self, key: &str) {
        if let Some(c) = self.input.get(key).and_then(component) {
            self.put(key, c);
        }
    }
    /// `LockCode.addToTag`.
    fn lock(&mut self) {
        self.keep_compound("lock");
    }
    /// `BaseContainerBlockEntity`: the custom name and the lock.
    fn base_container(&mut self) {
        self.component("CustomName");
        self.lock();
    }
    /// `RandomizableContainer.tryLoadLootTable` / `trySaveLootTable`; true when the
    /// block entity has a loot table (and so saves no items).
    fn loot_table(&mut self) -> bool {
        let Some(table) = text(self.input.get("LootTable")) else {
            return false;
        };
        self.put("LootTable", string(&table));
        let seed = long(self.input.get("LootTableSeed")).unwrap_or(0);
        if seed != 0 {
            self.put("LootTableSeed", Tag::Long(seed));
        }
        true
    }
    /// `ContainerHelper.loadAllItems` + `saveAllItems`.
    fn items(&mut self, size: usize, also_when_empty: bool) {
        let items = items(self.input.get("Items"), size);
        if !items.is_empty() || also_when_empty {
            self.put("Items", Tag::List(items));
        }
    }
    /// A container: loot table or items.
    fn container(&mut self, size: usize, also_when_empty: bool) {
        if !self.loot_table() {
            self.items(size, also_when_empty);
        }
    }
    /// An optional `ItemStack` (`RecordItem`, `Book`, `item`).
    fn item(&mut self, key: &str) -> bool {
        match self.input.get(key).and_then(stack) {
            Some(item) => {
                self.put(key, item);
                true
            }
            None => false,
        }
    }
}

/// `ComponentSerialization.CODEC` on loaded NBT: a literal without style is a plain
/// string; anything else is kept as loaded.
pub fn component(tag: &Tag) -> Option<Tag> {
    match tag {
        Tag::String(_) | Tag::List(_) => Some(tag.clone()),
        Tag::Compound(c) => {
            if c.len() == 1
                && let Some(t @ Tag::String(_)) = c.get(&"text".into())
            {
                return Some(t.clone());
            }
            Some(tag.clone())
        }
        _ => None,
    }
}

/// `ItemStack.CODEC`: `id`, `count` (1..=99, default 1) and a non-empty `components`
/// patch kept as loaded. None for the empty stack and for invalid stacks.
pub fn stack(tag: &Tag) -> Option<Tag> {
    let id = text(tag.get("id"))?;
    if id.is_empty() || id == "minecraft:air" {
        return None;
    }
    let count = int(tag.get("count")).unwrap_or(1);
    if !(1..=99).contains(&count) {
        return None;
    }
    let mut out = vec![("id", string(&id)), ("count", Tag::Int(count))];
    if let Some(c @ Tag::Compound(map)) = tag.get("components")
        && !map.is_empty()
    {
        out.push(("components", c.clone()));
    }
    Some(Tag::compound(out))
}
/// `ItemStackTemplate.CODEC`: an item id or `{id, count?, components?}`.
fn template(tag: &Tag) -> Option<Tag> {
    match tag {
        Tag::String(_) => Some(tag.clone()),
        Tag::Compound(_) => {
            let mut out = vec![("id", string(&text(tag.get("id"))?))];
            if let Some(count) = int(tag.get("count")).filter(|c| *c != 1) {
                out.push(("count", Tag::Int(count)));
            }
            if let Some(c @ Tag::Compound(map)) = tag.get("components")
                && !map.is_empty()
            {
                out.push(("components", c.clone()));
            }
            Some(Tag::compound(out))
        }
        _ => None,
    }
}
/// `ItemStackWithSlot` entries of a container of `size` slots, by ascending slot.
fn items(tag: Option<&Tag>, size: usize) -> Vec<Tag> {
    let mut slots: Vec<Option<Tag>> = vec![None; size];
    if let Some(Tag::List(list)) = tag {
        for entry in list {
            let (Some(slot), Some(item)) = (int(entry.get("Slot")), stack(entry)) else {
                continue;
            };
            if (0..size as i32).contains(&slot) {
                slots[slot as usize] = Some(item);
            }
        }
    }
    slots
        .into_iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let Tag::Compound(mut c) = item? else {
                return None;
            };
            c.insert("Slot".into(), Tag::Byte(i as i8));
            Some(Tag::Compound(c))
        })
        .collect()
}
fn int_array(tag: Option<&Tag>) -> Option<&Vec<i32>> {
    match tag {
        Some(Tag::IntArray(v)) => Some(v),
        _ => None,
    }
}
/// A saved `BlockPos` (`[I; x, y, z]`).
fn is_pos(tag: &Tag) -> bool {
    matches!(tag, Tag::IntArray(v) if v.len() == 3)
}
/// The loaded `SignText` of one side, or the empty text.
pub fn sign_side(tag: Option<&Tag>) -> Tag {
    let empty = || string("");
    let mut messages: Vec<Tag> = match tag.and_then(|t| t.get("messages")) {
        Some(Tag::List(list)) => list.iter().filter_map(component).collect(),
        _ => Vec::new(),
    };
    messages.resize_with(4, empty);
    messages.truncate(4);
    let mut out = vec![
        ("messages", Tag::List(messages)),
        (
            "color",
            string(
                &tag.and_then(|t| text(t.get("color")))
                    .unwrap_or_else(|| "black".into()),
            ),
        ),
        (
            "has_glowing_text",
            boolean(
                tag.and_then(|t| flag(t.get("has_glowing_text")))
                    .unwrap_or(false),
            ),
        ),
    ];
    if let Some(filtered @ Tag::List(list)) = tag.and_then(|t| t.get("filtered_messages"))
        && list.len() == 4
    {
        out.push(("filtered_messages", filtered.clone()));
    }
    Tag::compound(out)
}

/// The id of a block entity compound.
pub fn id(tag: &Tag) -> Option<String> {
    text(tag.get("id"))
}

/// Every `BlockEntityType` id this module knows.
pub const TYPES: [&str; 47] = [
    "minecraft:furnace",
    "minecraft:chest",
    "minecraft:trapped_chest",
    "minecraft:ender_chest",
    "minecraft:jukebox",
    "minecraft:dispenser",
    "minecraft:dropper",
    "minecraft:sign",
    "minecraft:hanging_sign",
    "minecraft:mob_spawner",
    "minecraft:creaking_heart",
    "minecraft:piston",
    "minecraft:brewing_stand",
    "minecraft:enchanting_table",
    "minecraft:end_portal",
    "minecraft:beacon",
    "minecraft:skull",
    "minecraft:daylight_detector",
    "minecraft:hopper",
    "minecraft:comparator",
    "minecraft:banner",
    "minecraft:structure_block",
    "minecraft:end_gateway",
    "minecraft:command_block",
    "minecraft:shulker_box",
    "minecraft:conduit",
    "minecraft:barrel",
    "minecraft:smoker",
    "minecraft:blast_furnace",
    "minecraft:lectern",
    "minecraft:bell",
    "minecraft:jigsaw",
    "minecraft:campfire",
    "minecraft:beehive",
    "minecraft:sculk_sensor",
    "minecraft:calibrated_sculk_sensor",
    "minecraft:sculk_catalyst",
    "minecraft:sculk_shrieker",
    "minecraft:chiseled_bookshelf",
    "minecraft:shelf",
    "minecraft:brushable_block",
    "minecraft:decorated_pot",
    "minecraft:crafter",
    "minecraft:trial_spawner",
    "minecraft:vault",
    "minecraft:test_block",
    "minecraft:test_instance_block",
];
/// The two types added after the ones above in 26.3.
pub const NEWEST: [&str; 2] = ["minecraft:copper_golem_statue", "minecraft:potent_sulfur"];

/// The block entity as vanilla would save it after loading `tag` (a full block
/// entity compound with `id`, `x`, `y`, `z`); None for an unknown type or a tag
/// without a position.
pub fn canonical(tag: &Tag) -> Option<Tag> {
    let id = id(tag)?;
    let (x, y, z) = (int(tag.get("x"))?, int(tag.get("y"))?, int(tag.get("z"))?);
    let mut f = Fields {
        input: tag,
        out: Compound::new(),
    };
    match id.as_str() {
        "minecraft:furnace" | "minecraft:smoker" | "minecraft:blast_furnace" => {
            f.base_container();
            f.int("cooking_time_spent", 0);
            f.int("cooking_total_time", 0);
            f.int("lit_time_remaining", 0);
            f.int("lit_total_time", 0);
            f.float("speed_multiplier", 1.0);
            f.items(3, true);
            let used: Compound = match tag.get("RecipesUsed") {
                Some(Tag::Compound(c)) => c
                    .iter()
                    .filter_map(|(k, v)| Some((k.clone(), Tag::Int(int(Some(v))?))))
                    .collect(),
                _ => Compound::new(),
            };
            f.put("RecipesUsed", Tag::Compound(used));
        }
        "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:barrel" => {
            f.base_container();
            f.container(27, true);
        }
        "minecraft:dispenser" | "minecraft:dropper" => {
            f.base_container();
            f.container(9, true);
        }
        "minecraft:hopper" => {
            f.base_container();
            f.container(5, true);
            f.int("TransferCooldown", -1);
        }
        "minecraft:shulker_box" => {
            f.base_container();
            f.container(27, false);
        }
        "minecraft:crafter" => {
            f.base_container();
            f.int("crafting_ticks_remaining", 0);
            f.container(9, true);
            let disabled: Vec<i32> = int_array(tag.get("disabled_slots"))
                .map(|v| v.iter().copied().filter(|s| (0..9).contains(s)).collect())
                .unwrap_or_default();
            // `addDisabledSlots`: the slots whose container data is 1, ascending.
            let mut slots = disabled;
            slots.sort_unstable();
            slots.dedup();
            f.put("disabled_slots", Tag::IntArray(slots));
            f.put(
                "triggered",
                Tag::Int(int(tag.get("triggered")).unwrap_or(0)),
            );
        }
        "minecraft:ender_chest"
        | "minecraft:end_portal"
        | "minecraft:daylight_detector"
        | "minecraft:bell"
        | "minecraft:conduit"
        | "minecraft:copper_golem_statue" => {
            if id == "minecraft:conduit" {
                f.keep("Target", |t| matches!(t, Tag::IntArray(v) if v.len() == 4));
            }
        }
        "minecraft:end_gateway" => {
            f.long("Age", 0);
            f.keep("exit_portal", |t| gateway_exit(t).is_some());
            if flag(tag.get("ExactTeleport")) == Some(true) {
                f.put("ExactTeleport", boolean(true));
            }
        }
        "minecraft:jukebox" => {
            let has_song = f.item("RecordItem");
            if has_song
                && let Some(ticks) = tag
                    .get("ticks_since_song_started")
                    .and_then(|t| long(Some(t)))
                && let Some(Tag::Compound(c)) = f.out.get(&"RecordItem".into())
                && is_record(c)
            {
                f.put("ticks_since_song_started", Tag::Long(ticks));
            }
        }
        "minecraft:sign" | "minecraft:hanging_sign" => {
            if flag(tag.get("allow_op_features")) == Some(true) {
                f.put("allow_op_features", boolean(true));
            }
            f.put("front_text", sign_side(tag.get("front_text")));
            f.put("back_text", sign_side(tag.get("back_text")));
            f.flag("is_waxed", false);
        }
        "minecraft:mob_spawner" => {
            f.short("Delay", 20);
            f.short("MinSpawnDelay", 200);
            f.short("MaxSpawnDelay", 800);
            f.short("SpawnCount", 4);
            f.short("MaxNearbyEntities", 6);
            f.short("RequiredPlayerRange", 16);
            f.short("SpawnRange", 4);
            f.keep_compound("SpawnData");
            let potentials = match tag.get("SpawnPotentials") {
                Some(t @ Tag::List(_)) => t.clone(),
                _ => match f.out.get(&"SpawnData".into()) {
                    Some(data) => Tag::List(vec![Tag::compound([
                        ("data", data.clone()),
                        ("weight", Tag::Int(1)),
                    ])]),
                    None => Tag::List(Vec::new()),
                },
            };
            f.put("SpawnPotentials", potentials);
        }
        "minecraft:creaking_heart" => {
            f.keep(
                "creaking",
                |t| matches!(t, Tag::IntArray(v) if v.len() == 4),
            );
        }
        "minecraft:piston" => {
            f.put(
                "blockState",
                match tag.get("blockState") {
                    Some(t @ (Tag::String(_) | Tag::Compound(_))) => t.clone(),
                    _ => string("minecraft:air"),
                },
            );
            // Direction.LEGACY_ID_CODEC: the 3D data value as a byte.
            f.put(
                "facing",
                Tag::Byte(int(tag.get("facing")).unwrap_or(0) as i8),
            );
            f.float("progress", 0.0);
            f.flag("extending", false);
            f.flag("source", false);
        }
        "minecraft:brewing_stand" => {
            f.base_container();
            f.int("BrewTime", 0);
            f.int("total_brew_time", 400);
            f.items(5, true);
            f.int("Fuel", 0);
            f.int("total_fuel", 20);
            f.float("speed_multiplier", 1.0);
        }
        "minecraft:enchanting_table" => f.component("CustomName"),
        "minecraft:beacon" => {
            for key in ["primary_effect", "secondary_effect"] {
                if let Some(effect) = text(tag.get(key)) {
                    f.put(key, string(&effect));
                }
            }
            f.int("Levels", 0);
            f.component("CustomName");
            f.lock();
        }
        "minecraft:skull" => {
            f.keep("profile", |t| {
                matches!(t, Tag::Compound(_) | Tag::String(_))
            });
            if let Some(sound) = text(tag.get("note_block_sound")) {
                f.put("note_block_sound", string(&sound));
            }
            f.component("custom_name");
        }
        "minecraft:comparator" => f.int("OutputSignal", 0),
        "minecraft:banner" => {
            if let Some(patterns @ Tag::List(list)) = tag.get("patterns")
                && !list.is_empty()
            {
                f.put("patterns", patterns.clone());
            }
            f.component("CustomName");
        }
        "minecraft:structure_block" => {
            f.string("name", "");
            f.string("author", "");
            f.string("metadata", "");
            f.int_clamped("posX", 0, -48, 48);
            f.int_clamped("posY", 1, -48, 48);
            f.int_clamped("posZ", 0, -48, 48);
            f.int_clamped("sizeX", 0, 0, 48);
            f.int_clamped("sizeY", 0, 0, 48);
            f.int_clamped("sizeZ", 0, 0, 48);
            f.string("rotation", "NONE");
            f.string("mirror", "NONE");
            f.string("mode", "DATA");
            f.flag("ignoreEntities", true);
            f.flag("strict", false);
            f.flag("powered", false);
            f.flag("showair", false);
            f.flag("showboundingbox", true);
            f.float("integrity", 1.0);
            f.long("seed", 0);
        }
        "minecraft:command_block" => {
            f.string("Command", "");
            f.int("SuccessCount", 0);
            f.component("CustomName");
            let track = flag(tag.get("TrackOutput")).unwrap_or(true);
            f.put("TrackOutput", boolean(track));
            if track {
                f.component("LastOutput");
            }
            let update = flag(tag.get("UpdateLastExecution")).unwrap_or(true);
            f.put("UpdateLastExecution", boolean(update));
            if update && let Some(last) = long(tag.get("LastExecution")).filter(|l| *l != -1) {
                f.put("LastExecution", Tag::Long(last));
            }
            f.flag("powered", false);
            f.flag("conditionMet", false);
            f.flag("auto", false);
        }
        "minecraft:lectern" => {
            if f.item("Book") {
                f.int("Page", 0);
            }
        }
        "minecraft:jigsaw" => {
            f.string("name", "minecraft:empty");
            f.string("target", "minecraft:empty");
            f.string("pool", "minecraft:empty");
            f.string("final_state", "minecraft:air");
            f.string("joint", "rollable");
            f.int("placement_priority", 0);
            f.int("selection_priority", 0);
        }
        "minecraft:campfire" => {
            f.items(4, true);
            for key in ["CookingTimes", "CookingTotalTimes"] {
                let mut times = [0; 4];
                if let Some(v) = int_array(tag.get(key)) {
                    for (slot, t) in times.iter_mut().zip(v) {
                        *slot = *t;
                    }
                }
                f.put(key, Tag::IntArray(times.to_vec()));
            }
        }
        "minecraft:beehive" => {
            let bees = match tag.get("bees") {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(|bee| {
                        let data = bee
                            .get("entity_data")
                            .filter(|d| matches!(d, Tag::Compound(_)))?;
                        Some(Tag::compound([
                            ("entity_data", data.clone()),
                            ("ticks_in_hive", Tag::Int(int(bee.get("ticks_in_hive"))?)),
                            (
                                "min_ticks_in_hive",
                                Tag::Int(int(bee.get("min_ticks_in_hive"))?),
                            ),
                        ]))
                    })
                    .collect(),
                _ => Vec::new(),
            };
            f.put("bees", Tag::List(bees));
            f.keep("flower_pos", is_pos);
        }
        "minecraft:sculk_sensor" | "minecraft:calibrated_sculk_sensor" => {
            f.int("last_vibration_frequency", 0);
            f.compound_or("listener", listener());
        }
        "minecraft:sculk_shrieker" => {
            f.int("warning_level", 0);
            f.compound_or("listener", listener());
        }
        "minecraft:sculk_catalyst" => {
            let cursors = match tag.get("cursors") {
                Some(t @ Tag::List(_)) => t.clone(),
                _ => Tag::List(Vec::new()),
            };
            f.put("cursors", cursors);
        }
        "minecraft:chiseled_bookshelf" => {
            f.items(6, true);
            f.int("last_interacted_slot", -1);
        }
        "minecraft:shelf" => {
            f.items(3, true);
            f.flag("align_items_to_bottom", false);
        }
        "minecraft:brushable_block" => {
            if !f.loot_table() {
                f.item("item");
            }
        }
        "minecraft:decorated_pot" => {
            if let Some(Tag::Compound(sherds)) = tag.get("sherds") {
                let kept: Compound = ["back", "left", "right", "front"]
                    .iter()
                    .filter_map(|side| {
                        Some(((*side).into(), template(sherds.get(&(*side).into())?)?))
                    })
                    .collect();
                if !kept.is_empty() {
                    f.put("sherds", Tag::Compound(kept));
                }
            }
            if !f.loot_table() {
                f.item("item");
            }
        }
        "minecraft:trial_spawner" => {
            // `TrialSpawnerStateData.Packed` and `FullConfig`: every key is optional
            // and defaulted by vanilla; they are kept as loaded. No `spawn_data` is
            // added: an absent one loads as Optional.empty and is not stored, and
            // an empty entity would keep the spawner from ever picking a mob.
            if let Tag::Compound(c) = tag {
                for (k, v) in c {
                    let key = k.to_string_lossy();
                    if !matches!(
                        key.as_str(),
                        "id" | "x" | "y" | "z" | "keepPacked" | "components"
                    ) {
                        f.out.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        "minecraft:vault" => {
            // `VaultConfig.DEFAULT` (when the config is absent): the trial key and the
            // default reward table, which is not saved.
            f.compound_or(
                "config",
                [(
                    "key_item".into(),
                    Tag::compound([
                        ("count", Tag::Int(1)),
                        ("id", string("minecraft:trial_key")),
                    ]),
                )]
                .into_iter()
                .collect(),
            );
            f.compound_or("shared_data", Compound::new());
            f.compound_or("server_data", Compound::new());
        }
        "minecraft:test_block" => {
            f.string("mode", "fail");
            f.string("message", "");
            f.flag("powered", false);
        }
        "minecraft:test_instance_block" => {
            f.compound_or(
                "data",
                [
                    ("ignore_entities".into(), boolean(false)),
                    ("rotation".into(), string("none")),
                    ("size".into(), Tag::IntArray(vec![0, 0, 0])),
                    ("status".into(), string("cleared")),
                ]
                .into_iter()
                .collect(),
            );
            if let Some(errors @ Tag::List(list)) = tag.get("errors")
                && !list.is_empty()
            {
                f.put("errors", errors.clone());
            }
        }
        "minecraft:potent_sulfur" => f.int("countdown", -1),
        _ => return None,
    }
    let mut out = f.out;
    out.insert("id".into(), string(&id));
    out.insert("x".into(), Tag::Int(x));
    out.insert("y".into(), Tag::Int(y));
    out.insert("z".into(), Tag::Int(z));
    out.insert("keepPacked".into(), Tag::Byte(0));
    out.insert(
        "components".into(),
        match tag.get("components") {
            Some(c @ Tag::Compound(_)) => c.clone(),
            _ => Tag::Compound(Compound::new()),
        },
    );
    Some(Tag::Compound(out))
}

/// `VibrationSystem.Data` for a sensor with no pending vibration.
fn listener() -> Compound {
    [
        ("event_delay".into(), Tag::Int(0)),
        ("selector".into(), Tag::compound([("tick", Tag::Long(-1))])),
    ]
    .into_iter()
    .collect()
}
/// Whether a stack plays a song (`JukeboxSong.fromStack`): a music disc or an item
/// with the `jukebox_playable` component.
fn is_record(item: &Compound) -> bool {
    let id = match item.get(&"id".into()) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => return false,
    };
    id.starts_with("minecraft:music_disc_")
        || matches!(item.get(&"components".into()), Some(Tag::Compound(c)) if c.contains_key(&"minecraft:jukebox_playable".into()))
}

/// Three-way merge of a block entity: `original` is the NBT it was loaded from,
/// `loaded` what Horizon encodes right after decoding it, and `current` what Horizon
/// encodes now. Whatever the game left unchanged since loading keeps its original
/// form (including the detail Horizon does not model, such as styled text), what it
/// changed takes the current value, and keys Horizon never models stay.
pub fn overlay(original: &Tag, loaded: &Tag, current: &Tag) -> Tag {
    if current == loaded {
        return original.clone();
    }
    match (original, loaded, current) {
        (Tag::Compound(o), Tag::Compound(l), Tag::Compound(c)) => {
            let mut out = Compound::new();
            for (key, cv) in c {
                let value = match (o.get(key), l.get(key)) {
                    (Some(ov), Some(lv)) => overlay(ov, lv, cv),
                    _ => cv.clone(),
                };
                out.insert(key.clone(), value);
            }
            // Keys the game never models (or models only when present) survive;
            // keys it removed are gone.
            for (key, ov) in o {
                if !c.contains_key(key) && !l.contains_key(key) {
                    out.insert(key.clone(), ov.clone());
                }
            }
            Tag::Compound(out)
        }
        (Tag::List(o), Tag::List(l), Tag::List(c)) if keyed_by_slot(c) && keyed_by_slot(l) => {
            let slot = |t: &Tag| int(t.get("Slot"));
            Tag::List(
                c.iter()
                    .map(|cv| {
                        let s = slot(cv);
                        fn find(list: &[Tag], slot: Option<i32>) -> Option<&Tag> {
                            list.iter().find(|t| int(t.get("Slot")) == slot)
                        }
                        match (find(o, s), find(l, s)) {
                            (Some(ov), Some(lv)) => overlay(ov, lv, cv),
                            _ => cv.clone(),
                        }
                    })
                    .collect(),
            )
        }
        _ => current.clone(),
    }
}
fn keyed_by_slot(list: &[Tag]) -> bool {
    list.iter().all(|t| t.get("Slot").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::nbt::parse_snbt;

    fn snbt(s: &str) -> Tag {
        parse_snbt(s).expect("snbt").0
    }
    fn canon(s: &str) -> Tag {
        canonical(&snbt(s)).expect("known type")
    }

    #[test]
    fn a_trial_spawner_without_spawn_data_saves_none() {
        let fresh = canon(
            r#"{id:"minecraft:trial_spawner",x:1,y:2,z:3,normal_config:"minecraft:trial_chamber/ranged/skeleton/normal"}"#,
        );
        assert_eq!(fresh.get("spawn_data"), None);
        let picked = canon(
            r#"{id:"minecraft:trial_spawner",x:1,y:2,z:3,spawn_data:{entity:{id:"minecraft:skeleton"}}}"#,
        );
        assert_eq!(
            picked.get("spawn_data"),
            Some(&snbt(r#"{entity:{id:"minecraft:skeleton"}}"#))
        );
    }

    #[test]
    fn every_type_has_defaults() {
        for id in TYPES.iter().chain(NEWEST.iter()) {
            let tag = snbt(&format!("{{id:\"{id}\",x:1,y:2,z:3}}"));
            let out = canonical(&tag).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(out.get("id"), tag.get("id"));
            assert_eq!(out.get("keepPacked"), Some(&Tag::Byte(0)), "{id}");
            assert_eq!(
                out.get("components"),
                Some(&Tag::Compound(Compound::new())),
                "{id}"
            );
        }
        assert!(canonical(&snbt("{id:\"minecraft:nope\",x:1,y:2,z:3}")).is_none());
    }

    #[test]
    fn furnace_defaults_match_vanilla() {
        let out = canon("{id:\"minecraft:furnace\",x:1,y:2,z:3}");
        assert_eq!(out.get("cooking_total_time"), Some(&Tag::Int(0)));
        assert_eq!(out.get("speed_multiplier"), Some(&f32_tag(1.0)));
        assert_eq!(out.get("Items"), Some(&Tag::List(vec![])));
        assert_eq!(
            out.get("RecipesUsed"),
            Some(&Tag::Compound(Compound::new()))
        );
    }

    /// `PistonMovingBlockEntity.saveAdditional`: `facing` through
    /// `Direction.LEGACY_ID_CODEC` (a byte), `progress` a float.
    #[test]
    fn moving_piston_fields_match_vanilla() {
        let out = canon(
            "{id:\"minecraft:piston\",x:1,y:2,z:3,blockState:{Name:\"minecraft:stone\"},\
             facing:5,progress:0.5f,extending:1b,source:0b}",
        );
        assert_eq!(out.get("facing"), Some(&Tag::Byte(5)));
        assert_eq!(out.get("progress"), Some(&f32_tag(0.5)));
        assert_eq!(out.get("extending"), Some(&Tag::Byte(1)));
        assert_eq!(out.get("source"), Some(&Tag::Byte(0)));
        let fresh = canon("{id:\"minecraft:piston\",x:1,y:2,z:3}");
        assert_eq!(fresh.get("facing"), Some(&Tag::Byte(0)));
        assert_eq!(fresh.get("blockState"), Some(&string("minecraft:air")));
    }

    #[test]
    fn loot_table_replaces_items_and_zero_seed_is_omitted() {
        let out = canon(
            "{id:\"minecraft:chest\",x:1,y:2,z:3,LootTable:\"minecraft:chests/x\",LootTableSeed:0L,\
             Items:[{Slot:0b,id:\"minecraft:stone\",count:1}]}",
        );
        assert!(out.get("Items").is_none());
        assert!(out.get("LootTableSeed").is_none());
        assert_eq!(out.get("LootTable"), Some(&string("minecraft:chests/x")));
    }

    #[test]
    fn items_are_validated_and_sorted_by_slot() {
        let out = canon(
            "{id:\"minecraft:hopper\",x:1,y:2,z:3,Items:[\
             {Slot:4b,id:\"minecraft:stone\",count:2},{Slot:5b,id:\"minecraft:stone\",count:1},\
             {Slot:0b,id:\"minecraft:air\",count:1},{Slot:1b,id:\"minecraft:dirt\"},\
             {Slot:2b,id:\"minecraft:dirt\",count:100}]}",
        );
        let Some(Tag::List(items)) = out.get("Items") else {
            panic!("items")
        };
        let slots: Vec<_> = items.iter().map(|i| i.get("Slot").cloned()).collect();
        assert_eq!(slots, vec![Some(Tag::Byte(1)), Some(Tag::Byte(4))]);
        assert_eq!(items[0].get("count"), Some(&Tag::Int(1)));
        assert_eq!(out.get("TransferCooldown"), Some(&Tag::Int(-1)));
    }

    #[test]
    fn plain_text_components_become_strings() {
        let out =
            canon("{id:\"minecraft:banner\",x:1,y:2,z:3,CustomName:{text:\"Flag\"},patterns:[]}");
        assert_eq!(out.get("CustomName"), Some(&string("Flag")));
        assert!(out.get("patterns").is_none());
        let styled =
            canon("{id:\"minecraft:banner\",x:1,y:2,z:3,CustomName:{text:\"F\",color:\"red\"}}");
        assert!(matches!(styled.get("CustomName"), Some(Tag::Compound(_))));
    }

    #[test]
    fn shulker_box_omits_empty_items_and_structure_block_clamps() {
        let shulker = canon("{id:\"minecraft:shulker_box\",x:1,y:2,z:3}");
        assert!(shulker.get("Items").is_none());
        let structure = canon("{id:\"minecraft:structure_block\",x:1,y:2,z:3,posX:99,sizeY:-4}");
        assert_eq!(structure.get("posX"), Some(&Tag::Int(48)));
        assert_eq!(structure.get("sizeY"), Some(&Tag::Int(0)));
        assert_eq!(structure.get("posY"), Some(&Tag::Int(1)));
        assert_eq!(structure.get("showboundingbox"), Some(&Tag::Byte(1)));
    }

    #[test]
    fn canonical_is_idempotent() {
        for id in TYPES.iter().chain(NEWEST.iter()) {
            let once = canonical(&snbt(&format!("{{id:\"{id}\",x:1,y:2,z:3}}"))).expect("known");
            assert_eq!(canonical(&once).as_ref(), Some(&once), "{id}");
        }
    }

    #[test]
    fn overlay_keeps_unchanged_detail_and_takes_changes() {
        let original = snbt(
            "{id:\"minecraft:sign\",front_text:{messages:[{text:\"a\",color:\"red\"},\"\",\"\",\"\"]},\
             back_text:{messages:[\"x\",\"\",\"\",\"\"]},extra:1b}",
        );
        let loaded = snbt(
            "{id:\"minecraft:sign\",front_text:{messages:[\"a\",\"\",\"\",\"\"]},\
             back_text:{messages:[\"x\",\"\",\"\",\"\"]}}",
        );
        let current = snbt(
            "{id:\"minecraft:sign\",front_text:{messages:[\"a\",\"\",\"\",\"\"]},\
             back_text:{messages:[\"y\",\"\",\"\",\"\"]}}",
        );
        let merged = overlay(&original, &loaded, &current);
        assert_eq!(merged.get("front_text"), original.get("front_text"));
        assert_eq!(merged.get("back_text"), current.get("back_text"));
        assert_eq!(merged.get("extra"), original.get("extra"));
    }

    #[test]
    fn overlay_merges_container_slots() {
        let item = |slot: i8, count: i32, extra: &str| {
            snbt(&format!(
                "{{Slot:{slot}b,id:\"minecraft:stone\",count:{count}{extra}}}"
            ))
        };
        let original = Tag::compound([(
            "Items",
            Tag::List(vec![
                item(0, 1, ",components:{\"minecraft:lore\":[\"x\"]}"),
                item(1, 1, ""),
            ]),
        )]);
        let loaded = Tag::compound([("Items", Tag::List(vec![item(0, 1, ""), item(1, 1, "")]))]);
        let current = Tag::compound([("Items", Tag::List(vec![item(0, 1, ""), item(1, 5, "")]))]);
        let merged = overlay(&original, &loaded, &current);
        let Some(Tag::List(items)) = merged.get("Items") else {
            panic!("items")
        };
        assert!(
            items[0].get("components").is_some(),
            "slot 0 keeps its lore"
        );
        assert_eq!(items[1].get("count"), Some(&Tag::Int(5)));
    }
}
