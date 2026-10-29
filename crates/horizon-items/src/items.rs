//! Items from vanilla's default-component reports, block<->item mapping,
//! tool mining rules and block loot tables (`LootTable`/`LootPool` semantics
//! for the condition, entry and modifier types used by block drops).
use crate::{Result, blocks::BlockFacts};
use horizon_core::random::RandomSource;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};

/// A fire starter's use sound: `SoundEvents.FIRECHARGE_USE` or
/// `FLINTANDSTEEL_USE`.
pub fn fire_starter_sound(fire_charge: bool) -> &'static str {
    if fire_charge {
        "minecraft:item.firecharge.use"
    } else {
        "minecraft:item.flintandsteel.use"
    }
}

/// The pitch of a fire starter's use sound, from the level's random:
/// `FlintAndSteelItem.useOn`'s `nextFloat() * 0.4 + 0.8`, or
/// `FireChargeItem.playSound`'s `(nextFloat() - nextFloat()) * 0.2 + 1.0`.
pub fn fire_starter_pitch(fire_charge: bool, random: &mut impl RandomSource) -> f32 {
    if fire_charge {
        (random.next_float() - random.next_float()) * 0.2 + 1.0
    } else {
        random.next_float() * 0.4 + 0.8
    }
}

#[derive(Clone, Debug)]
pub struct ToolRule {
    pub blocks: Vec<String>,
    pub speed: Option<f32>,
    pub correct: Option<bool>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ConsumeEffectSpec {
    pub id: String,
    pub duration: i32,
    pub amplifier: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConsumeEffect {
    ApplyEffects {
        effects: Vec<ConsumeEffectSpec>,
        probability: f32,
    },
    ClearAllEffects,
    RemoveEffects(Vec<String>),
    /// `TeleportRandomlyConsumeEffect`.
    TeleportRandomly(horizon_world::teleport_consume::TeleportConsume),
    PlaySound(String),
}

#[derive(Clone, Debug)]
pub struct ItemInfo {
    pub fire_resistant: bool,
    pub id: Arc<str>,
    pub max_stack: u32,
    pub max_damage: u32,
    pub block: Option<BlockStateId>,
    pub tool: Vec<ToolRule>,
    pub tool_default_speed: f32,
    /// The `tool` component's `damage_per_block`, when the item has the
    /// component (a mace or trident has one without rules).
    pub tool_damage_per_block: Option<u32>,
    /// `tool.can_destroy_blocks_in_creative` (defaults to true).
    pub can_destroy_blocks_in_creative: bool,
    pub food: Option<(u32, f32)>,
    pub can_always_eat: bool,
    /// `consumable`: its timing, sound, particles and `on_consume_effects`.
    pub consumable: Option<horizon_world::consumable::Consumable<String, ConsumeEffect>>,
    /// `use_remainder`: what a finished use leaves (`UseRemainder`).
    pub use_remainder: Option<String>,
    /// `use_cooldown`: the cooldown a use starts (`UseCooldown`).
    pub use_cooldown: Option<crate::cooldowns::UseCooldown>,
    pub model: String,
    /// Main-hand `attack_damage` / `attack_speed` modifier amounts.
    pub attack_damage: f32,
    pub attack_speed: f32,
    /// `cooking_fuel`: burn-time and speed-multiplier provider references.
    pub fuel: Option<(Value, Value)>,
    /// `brewing_fuel`: uses and speed-multiplier provider references.
    pub brewing_fuel: Option<(Value, Value)>,
    /// `compostable.layers`: the layers provider (`ComposterBlock.addLayer`).
    pub compostable: Option<Value>,
    /// Item left behind after crafting/fuel use (e.g. bucket).
    pub remainder: Option<String>,
    /// `equippable.slot` (head, chest, legs, feet, ...).
    pub equip_slot: Option<String>,
    /// `equippable.asset_id`: the equipment asset drawn when worn.
    pub equip_asset: Option<String>,
    /// All `attribute_modifiers`: (attribute, id, amount, operation, slot group).
    pub modifiers: Vec<(String, String, f64, String, String)>,
    /// `blocks_attacks` (shields).
    pub blocks_attacks: Option<BlocksAttacks>,
    /// `weapon.disable_blocking_for_seconds` (axes).
    pub disable_blocking_seconds: f32,
    /// `weapon.item_damage_per_attack`, when the item is a weapon.
    pub weapon_damage: Option<u32>,
    /// `use_effects.speed_multiplier` (UseEffects.DEFAULT: 0.2).
    pub use_speed: f32,
    /// `glider` (elytra).
    pub glider: bool,
    /// `equippable.swappable` and `equip_sound`.
    pub swappable: bool,
    /// `equippable.dispensable` (dispensers may put it on an entity).
    pub equip_dispensable: bool,
    /// `equippable.damage_on_hurt`: the item wears when its wearer is hurt
    /// (false for the elytra).
    pub equip_damage_on_hurt: bool,
    /// `equippable.allowed_entities` resolved to entity type ids; None allows all.
    pub equip_allowed: Option<HashSet<String>>,
    pub equip_sound: Option<String>,
    /// `jukebox_playable`: the jukebox song it plays.
    pub jukebox_song: Option<String>,
    /// `crafting_remainder`: what stays in the grid when the item is used in a recipe.
    pub crafting_remainder: Option<String>,
    /// `dye`: the dye colour name (dyes, and anything else with the component).
    pub dye: Option<String>,
    /// `cushion/color`: the DyeColor name of a cushion item.
    pub cushion_color: Option<String>,
    /// `block_transformer`: the registry entry an axe, hoe or shovel uses on blocks.
    pub block_transformer: Option<String>,
    /// Has a default `bundle_contents` (the bundles, `BundleItem`).
    pub bundle: bool,
    /// The ids of the item's default components (its prototype's keys).
    pub components: Vec<String>,
}
/// `Item.Properties.craftRemainder` (not a data component, so not in the
/// default-component report): what an item leaves in the grid when crafted with.
pub const CRAFT_REMAINDERS: [(&str, &str); 5] = [
    ("minecraft:water_bucket", "minecraft:bucket"),
    ("minecraft:lava_bucket", "minecraft:bucket"),
    ("minecraft:milk_bucket", "minecraft:bucket"),
    ("minecraft:dragon_breath", "minecraft:glass_bottle"),
    ("minecraft:honey_bottle", "minecraft:glass_bottle"),
];
/// A reduction of `blocks_attacks.damage_reductions`.
#[derive(Clone, Debug)]
pub struct BlockReduction {
    pub horizontal_angle: f32,
    /// Damage type ids or `#tags`; None blocks every type.
    pub types: Option<Vec<String>>,
    pub base: f32,
    pub factor: f32,
}
/// The `blocks_attacks` component.
#[derive(Clone, Debug)]
pub struct BlocksAttacks {
    pub delay_seconds: f32,
    pub disable_scale: f32,
    pub reductions: Vec<BlockReduction>,
    /// item_damage: (threshold, base, factor)
    pub item_damage: (f32, f32, f32),
    pub bypassed_by: Option<String>,
    pub block_sound: Option<String>,
    pub disabled_sound: Option<String>,
}
impl BlocksAttacks {
    fn parse(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let f = |v: &Value, d: f64| v.as_f64().unwrap_or(d) as f32;
        let text = |v: &Value| v.as_str().map(str::to_owned);
        let reductions = match v["damage_reductions"].as_array() {
            Some(list) => list
                .iter()
                .map(|r| BlockReduction {
                    horizontal_angle: f(&r["horizontal_blocking_angle"], 90.0),
                    types: match &r["type"] {
                        Value::String(s) => Some(vec![s.clone()]),
                        Value::Array(a) => Some(
                            a.iter()
                                .filter_map(|t| t.as_str().map(str::to_owned))
                                .collect(),
                        ),
                        _ => None,
                    },
                    base: f(&r["base"], 0.0),
                    factor: f(&r["factor"], 0.0),
                })
                .collect(),
            // BlocksAttacks.CODEC default: block everything within 90 degrees.
            None => vec![BlockReduction {
                horizontal_angle: 90.0,
                types: None,
                base: 0.0,
                factor: 1.0,
            }],
        };
        let d = &v["item_damage"];
        Some(Self {
            delay_seconds: f(&v["block_delay_seconds"], 0.0),
            disable_scale: f(&v["disable_cooldown_scale"], 1.0),
            reductions,
            item_damage: if d.is_object() {
                (
                    f(&d["threshold"], 1.0),
                    f(&d["base"], 0.0),
                    f(&d["factor"], 1.0),
                )
            } else {
                (1.0, 0.0, 1.0)
            },
            bypassed_by: text(&v["bypassed_by"]),
            block_sound: text(&v["block_sound"]),
            disabled_sound: text(&v["disabled_sound"]),
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub item: Arc<str>,
    pub count: u32,
    pub damage: u32,
    /// Components beyond count and damage (see `item_data`); None when default.
    pub extra: Option<Box<crate::item_data::StackData>>,
}
/// `order`'s items in the registry's order (`command-registries.json`'s
/// `items`, exported from the game); `order` itself when that is missing.
fn registry_order(root: &Path, order: &[Arc<str>]) -> Vec<Arc<str>> {
    let listed: Vec<String> = fs::read(root.join("command-registries.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| serde_json::from_value(v["items"].clone()).ok())
        .unwrap_or_default();
    if listed.is_empty() {
        return order.to_vec();
    }
    let known: HashSet<&str> = order.iter().map(|i| &**i).collect();
    listed
        .iter()
        .filter(|i| known.contains(i.as_str()))
        .map(|i| Arc::from(i.as_str()))
        .collect()
}
/// Item ids standing for a loot table's `minecraft:dynamic` entries.
const DYNAMIC: &str = "#dynamic:";
pub struct Items {
    pub items: HashMap<Arc<str>, ItemInfo>,
    pub order: Vec<Arc<str>>,
    /// The same items in item registry (network id) order, as vanilla lists
    /// them; sorted by name when the registry export is missing.
    pub registry_order: Vec<Arc<str>>,
    /// Block tag name (without '#') -> member block names.
    pub block_tags: HashMap<String, HashSet<String>>,
    pub item_tags: HashMap<String, HashSet<String>>,
    /// Item tags in vanilla's order (`TagLoader`: file order, nested tags in
    /// place, first occurrence kept), the order `TagSlotDisplay` resolves.
    pub item_tag_order: HashMap<String, Vec<String>>,
    loot: HashMap<String, Value>,
    /// The `jukebox_song` registry.
    pub songs: HashMap<String, Song>,
    /// The enchantment registry: loot functions enchant the items they roll
    /// (`EnchantWithLevelsFunction`, `EnchantRandomlyFunction`), and the
    /// station menus share this copy.
    pub enchantments: Arc<crate::enchant::Enchantments>,
}
/// A `JukeboxSong`.
#[derive(Clone, Debug)]
pub struct Song {
    pub sound: String,
    pub length_ticks: i64,
    pub comparator: i32,
    pub description: Value,
}
impl Song {
    /// `hasFinished`: the length plus one second.
    pub fn finished(&self, ticks: i64) -> bool {
        ticks >= self.length_ticks + 20
    }
}
fn load_songs(root: &Path) -> HashMap<String, Song> {
    let mut songs = HashMap::new();
    let Ok(dir) = fs::read_dir(root.join("data/minecraft/jukebox_song")) else {
        return songs;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        let (Some(stem), Ok(bytes)) = (path.file_stem().and_then(|s| s.to_str()), fs::read(&path))
        else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let sound = match &v["sound_event"] {
            Value::String(s) => s.clone(),
            other => other["sound_id"].as_str().unwrap_or("").to_owned(),
        };
        songs.insert(
            format!("minecraft:{stem}"),
            Song {
                sound,
                length_ticks: (v["length_in_seconds"].as_f64().unwrap_or(0.0) as f32 * 20.0).ceil()
                    as i64,
                comparator: v["comparator_output"].as_i64().unwrap_or(0) as i32,
                description: v["description"].clone(),
            },
        );
    }
    songs
}
pub fn load_tags(root: &Path, kind: &str) -> Result<HashMap<String, HashSet<String>>> {
    Ok(load_tag_lists(root, kind)?
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect())
}
/// Every tag's contents in `TagLoader.tryBuildTag` order (a `LinkedHashSet`).
pub(crate) fn load_tag_lists(root: &Path, kind: &str) -> Result<HashMap<String, Vec<String>>> {
    let mut raw: HashMap<String, Vec<String>> = HashMap::new();
    fn walk(
        dir: &Path,
        base: &Path,
        namespace: &str,
        raw: &mut HashMap<String, Vec<String>>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, base, namespace, raw)?;
            } else if p.extension().is_some_and(|e| e == "json") {
                let name = format!(
                    "{namespace}:{}",
                    p.strip_prefix(base)?
                        .with_extension("")
                        .to_string_lossy()
                        .replace('\\', "/")
                );
                let v: Value = serde_json::from_slice(&fs::read(&p)?)?;
                let values = v["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str().or_else(|| e["id"].as_str()).map(str::to_owned))
                    .collect();
                raw.insert(name, values);
            }
        }
        Ok(())
    }
    // Vanilla's tags; with content mods also the convention (`c`) tags and the mods' own.
    for namespace in crate::content_namespaces::tag_namespaces(root) {
        let dir = root.join(format!("data/{namespace}/tags/{kind}"));
        if dir.exists() {
            walk(&dir, &dir, &namespace, &mut raw)?;
        }
    }
    fn resolve(name: &str, raw: &HashMap<String, Vec<String>>, out: &mut Vec<String>, depth: u32) {
        if depth > 32 {
            return;
        }
        for v in raw.get(name).into_iter().flatten() {
            if let Some(tag) = v.strip_prefix('#') {
                resolve(tag, raw, out, depth + 1);
            } else if !out.contains(v) {
                out.push(v.clone());
            }
        }
    }
    Ok(raw
        .keys()
        .map(|k| {
            let mut list = Vec::new();
            resolve(k, &raw, &mut list, 0);
            (k.clone(), list)
        })
        .collect())
}
/// An entity type id or `#tag` as the set of entity type ids it names.
fn entity_spec(tags: &HashMap<String, HashSet<String>>, spec: &str) -> HashSet<String> {
    match spec.strip_prefix('#') {
        Some(tag) => tags.get(tag).cloned().unwrap_or_default(),
        None => HashSet::from([spec.to_owned()]),
    }
}
impl Items {
    pub fn resists_fire(&self, stack: &ItemStack, source: &str) -> bool {
        if let Some(data) = stack.data()
            && let Some((_, v)) = data
                .other
                .iter()
                .find(|(key, _)| key == "minecraft:damage_resistant")
        {
            return match v.get("types") {
                Some(horizon_world::nbt::Tag::String(tag)) => {
                    tag.to_string().is_ok_and(|id|id=="#minecraft:is_fire" || id==source)
                }
                Some(horizon_world::nbt::Tag::List(tags)) => tags.iter().any(
                    |t| matches!(t,horizon_world::nbt::Tag::String(id) if id.to_string().is_ok_and(|id|id==source)),
                ),
                _ => false,
            };
        }
        self.get(&stack.item).is_some_and(|i| i.fire_resistant)
    }

    /// `ItemStack.canBeHurtBy` for the cactus damage type: a `damage_resistant`
    /// component naming `minecraft:cactus`, or a damage type tag that holds it.
    /// (`#minecraft:is_fire`, as on netherite gear, does not.)
    pub fn resists_cactus(&self, stack: &ItemStack) -> bool {
        const CACTUS_TAGS: [&str; 5] = [
            "#minecraft:bypasses_shield",
            "#minecraft:no_knockback",
            "#minecraft:panic_environmental_causes",
            "#minecraft:panic_causes",
            "#minecraft:sulfur_cube_with_block_immune_to",
        ];
        let Some((_, v)) = stack.data().and_then(|data| {
            data.other
                .iter()
                .find(|(key, _)| key == "minecraft:damage_resistant")
        }) else {
            return false;
        };
        match v.get("types") {
            Some(horizon_world::nbt::Tag::String(id)) => id
                .to_string()
                .is_ok_and(|id| id == "minecraft:cactus" || CACTUS_TAGS.contains(&id.as_str())),
            Some(horizon_world::nbt::Tag::List(ids)) => ids.iter().any(|t| {
                matches!(t, horizon_world::nbt::Tag::String(id)
                    if id.to_string().is_ok_and(|id| id == "minecraft:cactus"))
            }),
            _ => false,
        }
    }

    pub fn load(root: &Path, registry: &BlockRegistry, facts: &BlockFacts) -> Result<Self> {
        let mut block_for_item: HashMap<&str, BlockStateId> = HashMap::new();
        for (i, s) in facts.states.iter().enumerate() {
            let default = registry.default_state(&s.name);
            if default == Some(BlockStateId(i as u32)) && &*s.item != "minecraft:air" {
                block_for_item
                    .entry(&s.item)
                    .or_insert(BlockStateId(i as u32));
            }
        }
        Self::load_with(root, &block_for_item)
    }
    /// Every item's default components and the tags, without the block <->
    /// item mapping (for headless tools that never place blocks).
    pub fn catalog(root: &Path) -> Result<Self> {
        Self::load_with(root, &HashMap::new())
    }
    fn load_with(root: &Path, block_for_item: &HashMap<&str, BlockStateId>) -> Result<Self> {
        let raw: BTreeMap<String, Value> =
            serde_json::from_slice(&fs::read(root.join("items.json"))?)?;
        let entity_tags = load_tags(root, "entity_type")?;
        let mut items = HashMap::new();
        let mut order = Vec::new();
        for (name, c) in &raw {
            let id: Arc<str> = Arc::from(name.as_str());
            let tool = c["minecraft:tool"]["rules"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| ToolRule {
                    blocks: match &r["blocks"] {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(a) => a
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect(),
                        _ => vec![],
                    },
                    speed: r["speed"].as_f64().map(|v| v as f32),
                    correct: r["correct_for_drops"].as_bool(),
                })
                .collect();
            let modifier = |kind: &str| -> f32 {
                c["minecraft:attribute_modifiers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|m| {
                        m["type"].as_str() == Some(kind)
                            && m["operation"].as_str() == Some("add_value")
                    })
                    .filter_map(|m| m["amount"].as_f64())
                    .sum::<f64>() as f32
            };
            let food = c.get("minecraft:food").map(|f| {
                (
                    f["nutrition"].as_u64().unwrap_or(0) as u32,
                    f["saturation"].as_f64().unwrap_or(0.0) as f32,
                )
            });
            let can_always_eat = c
                .get("minecraft:food")
                .and_then(|f| f.get("can_always_eat"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut consume_effects = Vec::new();
            if let Some(effects) = c
                .get("minecraft:consumable")
                .and_then(|cons| cons.get("on_consume_effects"))
                .and_then(|e| e.as_array())
            {
                for effect in effects {
                    match effect.get("type").and_then(|t| t.as_str()) {
                        Some("minecraft:apply_effects") => {
                            let probability = effect
                                .get("probability")
                                .and_then(|p| p.as_f64())
                                .map_or(1.0, |p| p as f32);
                            let mut specs = Vec::new();
                            if let Some(list) = effect.get("effects").and_then(|l| l.as_array()) {
                                for e in list {
                                    if let Some(id) = e.get("id").and_then(|i| i.as_str()) {
                                        let duration =
                                            e.get("duration").and_then(|d| d.as_i64()).unwrap_or(0)
                                                as i32;
                                        let amplifier = e
                                            .get("amplifier")
                                            .and_then(|a| a.as_i64())
                                            .unwrap_or(0)
                                            as i32;
                                        specs.push(ConsumeEffectSpec {
                                            id: id.to_string(),
                                            duration,
                                            amplifier,
                                        });
                                    }
                                }
                            }
                            consume_effects.push(ConsumeEffect::ApplyEffects {
                                effects: specs,
                                probability,
                            });
                        }
                        Some("minecraft:clear_all_effects") => {
                            consume_effects.push(ConsumeEffect::ClearAllEffects);
                        }
                        Some("minecraft:remove_effects") => {
                            let mut targets = Vec::new();
                            if let Some(eff) = effect.get("effects") {
                                if let Some(s) = eff.as_str() {
                                    targets.push(s.to_string());
                                } else if let Some(arr) = eff.as_array() {
                                    for item in arr {
                                        if let Some(s) = item.as_str() {
                                            targets.push(s.to_string());
                                        }
                                    }
                                }
                            }
                            consume_effects.push(ConsumeEffect::RemoveEffects(targets));
                        }
                        Some("minecraft:teleport_randomly") => {
                            consume_effects.push(ConsumeEffect::TeleportRandomly(
                                horizon_world::teleport_consume::TeleportConsume {
                                    diameter: effect
                                        .get("diameter")
                                        .and_then(Value::as_f64)
                                        .map_or(16.0, |d| d as f32),
                                    directional_particles: effect
                                        .get("directional_particles")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(true),
                                },
                            ));
                        }
                        Some("minecraft:play_sound") => {
                            if let Some(sound) = effect.get("sound").and_then(|s| s.as_str()) {
                                consume_effects.push(ConsumeEffect::PlaySound(sound.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            items.insert(
                id.clone(),
                ItemInfo {
                    fire_resistant: c["minecraft:damage_resistant"]["types"].as_str()
                        == Some("#minecraft:is_fire"),
                    id: id.clone(),
                    max_stack: c["minecraft:max_stack_size"].as_u64().unwrap_or(64) as u32,
                    max_damage: c["minecraft:max_damage"].as_u64().unwrap_or(0) as u32,
                    block: block_for_item.get(name.as_str()).copied(),
                    tool,
                    tool_default_speed: c["minecraft:tool"]["default_mining_speed"]
                        .as_f64()
                        .unwrap_or(1.0) as f32,
                    tool_damage_per_block: c
                        .get("minecraft:tool")
                        .map(|t| t["damage_per_block"].as_u64().unwrap_or(1) as u32),
                    can_destroy_blocks_in_creative:
                        c["minecraft:tool"]["can_destroy_blocks_in_creative"]
                            .as_bool()
                            .unwrap_or(true),
                    food,
                    can_always_eat,
                    // Consumable.CODEC's defaults.
                    consumable: c.get("minecraft:consumable").map(|cons| {
                        horizon_world::consumable::Consumable {
                            seconds: cons["consume_seconds"].as_f64().map_or(1.6, |s| s as f32),
                            animation: crate::use_animation::consumable_animation(cons),
                            sound: cons["sound"]
                                .as_str()
                                .unwrap_or("minecraft:entity.generic.eat")
                                .to_owned(),
                            particles: cons["has_consume_particles"].as_bool().unwrap_or(true),
                            effects: std::mem::take(&mut consume_effects),
                        }
                    }),
                    use_remainder: c["minecraft:use_remainder"]["id"]
                        .as_str()
                        .map(str::to_owned),
                    use_cooldown: c
                        .get("minecraft:use_cooldown")
                        .and_then(crate::cooldowns::UseCooldown::from_json),
                    model: c["minecraft:item_model"]
                        .as_str()
                        .unwrap_or(name)
                        .to_owned(),
                    glider: c["minecraft:glider"].is_object(),
                    swappable: c["minecraft:equippable"]["swappable"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_dispensable: c["minecraft:equippable"]["dispensable"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_damage_on_hurt: c["minecraft:equippable"]["damage_on_hurt"]
                        .as_bool()
                        .unwrap_or(true),
                    equip_allowed: match &c["minecraft:equippable"]["allowed_entities"] {
                        Value::String(spec) => Some(entity_spec(&entity_tags, spec)),
                        Value::Array(list) => Some(
                            list.iter()
                                .filter_map(Value::as_str)
                                .flat_map(|spec| entity_spec(&entity_tags, spec))
                                .collect(),
                        ),
                        _ => None,
                    },
                    equip_sound: c["minecraft:equippable"]["equip_sound"]
                        .as_str()
                        .map(str::to_owned),
                    jukebox_song: c["minecraft:jukebox_playable"].as_str().map(str::to_owned),
                    dye: c["minecraft:dye"].as_str().map(str::to_owned),
                    cushion_color: c["minecraft:cushion/color"].as_str().map(str::to_owned),
                    block_transformer: c["minecraft:block_transformer"].as_str().map(str::to_owned),
                    bundle: c.get("minecraft:bundle_contents").is_some(),
                    components: c
                        .as_object()
                        .map(|c| c.keys().cloned().collect())
                        .unwrap_or_default(),
                    crafting_remainder: CRAFT_REMAINDERS
                        .iter()
                        .find(|(item, _)| *item == name)
                        .map(|(_, rest)| (*rest).to_owned()),
                    use_speed: c["minecraft:use_effects"]["speed_multiplier"]
                        .as_f64()
                        .unwrap_or(0.2) as f32,
                    blocks_attacks: BlocksAttacks::parse(&c["minecraft:blocks_attacks"]),
                    disable_blocking_seconds: c["minecraft:weapon"]["disable_blocking_for_seconds"]
                        .as_f64()
                        .unwrap_or(0.0) as f32,
                    weapon_damage: c["minecraft:weapon"].is_object().then(|| {
                        c["minecraft:weapon"]["item_damage_per_attack"]
                            .as_u64()
                            .unwrap_or(1) as u32
                    }),
                    equip_asset: c["minecraft:equippable"]["asset_id"]
                        .as_str()
                        .map(str::to_owned),
                    equip_slot: c["minecraft:equippable"]["slot"]
                        .as_str()
                        .map(str::to_owned),
                    modifiers: c["minecraft:attribute_modifiers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| {
                            Some((
                                m["type"].as_str()?.to_owned(),
                                m["id"].as_str()?.to_owned(),
                                m["amount"].as_f64()?,
                                m["operation"].as_str().unwrap_or("add_value").to_owned(),
                                m["slot"].as_str().unwrap_or("any").to_owned(),
                            ))
                        })
                        .collect(),
                    fuel: c
                        .get("minecraft:cooking_fuel")
                        .map(|f| (f["burn_time"].clone(), f["speed_multiplier"].clone())),
                    brewing_fuel: c
                        .get("minecraft:brewing_fuel")
                        .map(|f| (f["uses"].clone(), f["speed_multiplier"].clone())),
                    compostable: c.get("minecraft:compostable").map(|f| f["layers"].clone()),
                    remainder: c["minecraft:use_remainder"]["id"]
                        .as_str()
                        .or(c["minecraft:crafting_remainder"]["id"].as_str())
                        .or(c["minecraft:crafting_remainder"].as_str())
                        .map(str::to_owned),
                    attack_damage: modifier("minecraft:attack_damage"),
                    attack_speed: modifier("minecraft:attack_speed"),
                },
            );
            order.push(id);
        }
        let mut loot = HashMap::new();
        // Every loot table, keyed by its id (blocks/stone, entities/sheep/white,
        // gameplay/chicken_lay, shearing/sheep/red...).
        fn walk_loot(
            dir: &Path,
            base: &Path,
            namespace: &str,
            loot: &mut HashMap<String, Value>,
        ) -> Result<()> {
            for entry in fs::read_dir(dir)?.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk_loot(&p, base, namespace, loot)?;
                } else if p.extension().is_some_and(|e| e == "json") {
                    let id = p
                        .strip_prefix(base)?
                        .with_extension("")
                        .to_string_lossy()
                        .replace('\\', "/");
                    loot.insert(
                        format!("{namespace}:{id}"),
                        serde_json::from_slice(&fs::read(&p)?)?,
                    );
                }
            }
            Ok(())
        }
        let loot_root = root.join("data/minecraft/loot_table");
        walk_loot(&loot_root, &loot_root, "minecraft", &mut loot)?;
        // Content mods' tables (docs/modpacks.md): `<ns>:blocks/...`.
        for namespace in crate::content_namespaces::namespaces(root) {
            let dir = root.join(format!("data/{namespace}/loot_table"));
            if dir.is_dir() {
                walk_loot(&dir, &dir, &namespace, &mut loot)?;
            }
        }
        let registry_order = registry_order(root, &order);
        Ok(Self {
            items,
            order,
            registry_order,
            block_tags: load_tags(root, "block")?,
            item_tags: load_tags(root, "item")?,
            item_tag_order: load_tag_lists(root, "item")?,
            loot,
            songs: load_songs(root),
            enchantments: Arc::new(crate::enchant::Enchantments::load(root)),
        })
    }
    pub fn get(&self, id: &str) -> Option<&ItemInfo> {
        self.items.get(id)
    }
    /// Only the block and item tags (and the enchantment registry, which the
    /// station rules and loot functions are built from), for tools that need
    /// tag membership.
    pub fn tags_only(root: &Path) -> Result<Self> {
        Ok(Self {
            items: HashMap::new(),
            order: Vec::new(),
            registry_order: Vec::new(),
            block_tags: load_tags(root, "block")?,
            item_tags: load_tags(root, "item")?,
            item_tag_order: load_tag_lists(root, "item")?,
            loot: HashMap::new(),
            songs: HashMap::new(),
            enchantments: Arc::new(crate::enchant::Enchantments::load(root)),
        })
    }
    /// An item table with only tags, for unit tests of item rules.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(item_tags: HashMap<String, HashSet<String>>) -> Self {
        Self {
            items: HashMap::new(),
            order: Vec::new(),
            registry_order: Vec::new(),
            block_tags: HashMap::new(),
            item_tag_order: item_tags
                .iter()
                .map(|(k, v)| (k.clone(), v.iter().cloned().collect()))
                .collect(),
            item_tags,
            loot: HashMap::new(),
            songs: HashMap::new(),
            enchantments: Arc::new(crate::enchant::Enchantments::default()),
        }
    }
    /// Whether `block` is in the block tag `tag` (`minecraft:fire`, no `#`).
    pub fn block_in_tag(&self, tag: &str, block: &str) -> bool {
        self.block_tags.get(tag).is_some_and(|s| s.contains(block))
    }
    fn block_matches(&self, spec: &str, block: &str) -> bool {
        match spec.strip_prefix('#') {
            Some(tag) => self.block_tags.get(tag).is_some_and(|s| s.contains(block)),
            None => spec == block,
        }
    }
    /// `Item.canDestroyBlock`: a tool may forbid block destruction with instabuild.
    pub fn can_destroy_block(&self, held: Option<&ItemStack>, creative: bool) -> bool {
        if !creative {
            return true;
        }
        let Some(held) = held.filter(|s| s.count > 0 && &*s.item != "minecraft:air") else {
            return true;
        };
        if let Some(tool) = held.data().and_then(|d| d.other("minecraft:tool")) {
            return tool
                .get("can_destroy_blocks_in_creative")
                .and_then(|v| match v {
                    horizon_world::nbt::Tag::Byte(v) => Some(*v),
                    _ => None,
                })
                .is_none_or(|v| v != 0);
        }
        self.get(&held.item)
            .is_none_or(|info| info.can_destroy_blocks_in_creative)
    }
    /// `Tool.getMiningSpeed` for the held item (1.0 without a tool component).
    pub fn mining_speed(&self, held: Option<&ItemStack>, block: &str) -> f32 {
        let Some(info) = held.and_then(|s| self.get(&s.item)) else {
            return 1.0;
        };
        for rule in &info.tool {
            if let Some(speed) = rule.speed
                && rule.blocks.iter().any(|b| self.block_matches(b, block))
            {
                return speed;
            }
        }
        if info.tool.is_empty() {
            1.0
        } else {
            info.tool_default_speed
        }
    }
    /// `Tool.isCorrectForDrops`.
    pub fn correct_for_drops(&self, held: Option<&ItemStack>, block: &str) -> bool {
        let Some(info) = held.and_then(|s| self.get(&s.item)) else {
            return false;
        };
        for rule in &info.tool {
            if let Some(correct) = rule.correct
                && rule.blocks.iter().any(|b| self.block_matches(b, block))
            {
                return correct;
            }
        }
        false
    }
    /// Whether a loot table (`minecraft:blocks/stone`, ...) exists.
    pub fn has_loot_table(&self, id: &str) -> bool {
        self.loot.contains_key(id)
    }
    /// `LootTable.getRandomItems` of an inline table.
    pub fn inline_loot_table<R: RandomSource>(
        &self,
        table: &Value,
        random: &mut R,
    ) -> Vec<ItemStack> {
        let empty = BTreeMap::new();
        let ctx = LootContext {
            items: self,
            block: "",
            properties: &empty,
            held: None,
            killed_by_player: false,
            explosion_radius: None,
            this_entity: false,
        };
        ctx.evaluate(table, random)
    }
    /// The server random sequence the block's loot table draws from.
    pub fn block_loot_sequence(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
    ) -> Option<String> {
        let block = registry.state(state)?;
        let (namespace, name) = block
            .name
            .split_once(':')
            .unwrap_or(("minecraft", &block.name));
        let standing = name.replacen("_wall_", "_", 1);
        let standing = standing.strip_prefix("wall_").unwrap_or(&standing);
        let standing = standing.replacen("wall_", "", 1);
        self.loot_sequence(&format!("{namespace}:blocks/{name}"))
            .or_else(|| self.loot_sequence(&format!("{namespace}:blocks/{standing}")))
    }
    /// Evaluate the block's loot table for a player break without explosions.
    pub fn block_drops<R: RandomSource>(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        self.block_drops_in(registry, state, held, random, None)
    }
    /// `block_drops` drawing from the table's random sequence when it has one
    /// (`sequences`: the world's sequences and seed), as BlockState.getDrops.
    pub fn block_drops_in<R: RandomSource>(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        held: Option<&ItemStack>,
        random: &mut R,
        sequences: Option<(&mut crate::random_sequences::RandomSequences, i64)>,
    ) -> Vec<ItemStack> {
        self.block_drops_full(registry, state, held, &[], random, sequences)
    }
    /// `block_drops` with the block entity's dynamic drops (`LootParams.withDynamicDrop`:
    /// a decorated pot's sherds) in place of the `minecraft:dynamic` entries.
    pub fn block_drops_with(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        held: Option<&ItemStack>,
        dynamic: &[ItemStack],
        random: &mut impl RandomSource,
    ) -> Vec<ItemStack> {
        self.block_drops_full(registry, state, held, dynamic, random, None)
    }
    /// `block_drops_in` with dynamic drops.
    pub fn block_drops_full(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        held: Option<&ItemStack>,
        dynamic: &[ItemStack],
        random: &mut impl RandomSource,
        sequences: Option<(&mut crate::random_sequences::RandomSequences, i64)>,
    ) -> Vec<ItemStack> {
        self.block_drops_exploded(
            registry, state, held, dynamic, random, sequences, None, false,
        )
    }
    /// `block_drops_with` for a block a player breaks (`Block.playerDestroy`):
    /// the player is the loot context's `this` entity, so snow and chorus
    /// flower tables see an entity.
    pub fn block_drops_by_player(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        held: Option<&ItemStack>,
        dynamic: &[ItemStack],
        random: &mut impl RandomSource,
    ) -> Vec<ItemStack> {
        self.block_drops_exploded(registry, state, held, dynamic, random, None, None, true)
    }
    /// `block_drops_full` for a block an explosion destroys: `explosion`
    /// is its radius when the explosion decays drops (`DESTROY_WITH_DECAY`);
    /// `breaker` is whether an entity (the player) broke the block.
    #[allow(clippy::too_many_arguments)]
    pub fn block_drops_exploded(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        held: Option<&ItemStack>,
        dynamic: &[ItemStack],
        random: &mut impl RandomSource,
        sequences: Option<(&mut crate::random_sequences::RandomSequences, i64)>,
        explosion: Option<f32>,
        breaker: bool,
    ) -> Vec<ItemStack> {
        let drops =
            self.block_drops_raw(registry, state, held, random, sequences, explosion, breaker);
        if !drops.iter().any(|d| d.item.starts_with(DYNAMIC)) {
            return drops;
        }
        drops
            .into_iter()
            .flat_map(|d| {
                if d.item.starts_with(DYNAMIC) {
                    dynamic.to_vec()
                } else {
                    vec![d]
                }
            })
            .collect()
    }
    #[allow(clippy::too_many_arguments)]
    fn block_drops_raw(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        held: Option<&ItemStack>,
        random: &mut impl RandomSource,
        sequences: Option<(&mut crate::random_sequences::RandomSequences, i64)>,
        explosion_radius: Option<f32>,
        breaker: bool,
    ) -> Vec<ItemStack> {
        let Some(block) = registry.state(state) else {
            return vec![];
        };
        let (namespace, name) = block
            .name
            .split_once(':')
            .unwrap_or(("minecraft", &block.name));
        // `Blocks.wallVariant`: wall signs, banners, heads, fans and torches
        // reuse the standing block's loot table.
        let standing = name.replacen("_wall_", "_", 1);
        let standing = standing.strip_prefix("wall_").unwrap_or(&standing);
        let standing = standing.replacen("wall_", "", 1);
        let table = self
            .loot
            .get(&format!("{namespace}:blocks/{name}"))
            .or_else(|| self.loot.get(&format!("{namespace}:blocks/{standing}")));
        let Some(table) = table else {
            return vec![];
        };
        // LootTable.getRandomItems draws from the table's random sequence.
        if let (Some(seq), Some(sequences)) = (
            table.get("random_sequence").and_then(Value::as_str),
            sequences,
        ) {
            let random = sequences.0.get(seq, sequences.1);
            let ctx = LootContext {
                items: self,
                block: &block.name,
                properties: &block.properties,
                held,
                killed_by_player: false,
                explosion_radius,
                this_entity: breaker,
            };
            return ctx.evaluate(table, random);
        }
        let ctx = LootContext {
            items: self,
            block: &block.name,
            properties: &block.properties,
            held,
            killed_by_player: false,
            explosion_radius,
            this_entity: breaker,
        };
        ctx.evaluate(table, random)
    }
    /// Evaluate an entity's loot table (`minecraft:entities/<name>`).
    /// Evaluate any loot table by id (e.g. `minecraft:gameplay/chicken_lay`).
    pub fn loot_table<R: RandomSource>(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        let Some(table) = self.loot.get(id) else {
            return vec![];
        };
        let empty = BTreeMap::new();
        let ctx = LootContext {
            items: self,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity: false,
        };
        ctx.evaluate(table, random)
    }
    /// The server random sequence a loot table draws from (`random_sequence`):
    /// vanilla's `LootContext.Builder.create` uses it instead of the level random.
    pub fn loot_sequence(&self, id: &str) -> Option<String> {
        self.loot.get(id)?["random_sequence"]
            .as_str()
            .map(str::to_owned)
    }
    /// `Block.dropFromBlockInteractLootTable`: the `block_interact` table `id`
    /// for a right click on `state` (its `match_block` conditions see the
    /// state), from the table's random sequence when it has one.
    pub fn interact_loot(
        &self,
        id: &str,
        registry: &BlockRegistry,
        state: BlockStateId,
        random: &mut impl RandomSource,
        sequences: &mut crate::random_sequences::RandomSequences,
        seed: i64,
    ) -> Vec<ItemStack> {
        let (Some(table), Some(block)) = (self.loot.get(id), registry.state(state)) else {
            return vec![];
        };
        let ctx = LootContext {
            items: self,
            block: &block.name,
            properties: &block.properties,
            held: None,
            killed_by_player: false,
            explosion_radius: None,
            this_entity: false,
        };
        match table.get("random_sequence").and_then(Value::as_str) {
            Some(sequence) => ctx.evaluate(table, sequences.get(sequence, seed)),
            None => ctx.evaluate(table, random),
        }
    }
    /// `loot_table` drawing from any random (a random sequence).
    pub fn loot_table_from(
        &self,
        id: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut impl RandomSource,
    ) -> Vec<ItemStack> {
        let Some(table) = self.loot.get(id) else {
            return vec![];
        };
        let empty = BTreeMap::new();
        let ctx = LootContext {
            items: self,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity: false,
        };
        ctx.evaluate(table, random)
    }
    pub fn entity_drops<R: RandomSource>(
        &self,
        entity: &str,
        killed_by_player: bool,
        held: Option<&ItemStack>,
        random: &mut R,
    ) -> Vec<ItemStack> {
        let name = entity.trim_start_matches("minecraft:");
        let Some(table) = self.loot.get(&format!("minecraft:entities/{name}")) else {
            return vec![];
        };
        let empty = BTreeMap::new();
        let ctx = LootContext {
            items: self,
            block: "",
            properties: &empty,
            held,
            killed_by_player,
            explosion_radius: None,
            this_entity: false,
        };
        ctx.evaluate(table, random)
    }
}
/// `DataComponents.BLOCK_STATE`.
const BLOCK_STATE: &str = "minecraft:block_state";
impl LootContext<'_> {
    fn evaluate<R: RandomSource>(&self, table: &Value, random: &mut R) -> Vec<ItemStack> {
        let ctx = self;
        let mut out = Vec::new();
        for pool in table["pools"].as_array().into_iter().flatten() {
            if !ctx.conditions(pool, random) {
                continue;
            }
            // LootPool: rolls.getInt (Mth.randomBetweenInclusive for uniform rolls).
            let rolls = LootContext::int_number(&pool["rolls"], random).max(0) as u32;
            for _ in 0..rolls {
                let mut produced = Vec::new();
                ctx.choose(
                    pool["entries"].as_array().map_or(&[][..], Vec::as_slice),
                    random,
                    &mut produced,
                );
                for mut stack in produced {
                    ctx.modifiers(&pool["modifier"], &mut stack, random);
                    ctx.modifiers(&table["modifier"], &mut stack, random);
                    if stack.count > 0 {
                        out.push(stack);
                    }
                }
            }
        }
        out
    }
}
struct LootContext<'a> {
    items: &'a Items,
    block: &'a str,
    properties: &'a BTreeMap<String, String>,
    held: Option<&'a ItemStack>,
    killed_by_player: bool,
    /// `LootContextParams.EXPLOSION_RADIUS`: an explosion that decays drops.
    explosion_radius: Option<f32>,
    /// `LootContextParams.THIS_ENTITY` is set (a player broke the block).
    this_entity: bool,
}
impl LootContext<'_> {
    /// Level of an enchantment on the tool / killer's weapon.
    fn enchantment_level(&self, id: &str) -> u32 {
        self.held.map_or(0, |h| h.enchantment_level(id))
    }
    fn number<R: RandomSource>(v: &Value, random: &mut R) -> f32 {
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
            Value::Object(o) => match o
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("minecraft:uniform")
            {
                "minecraft:constant" => {
                    o.get("value").and_then(Value::as_f64).unwrap_or(0.0) as f32
                }
                "minecraft:uniform" => {
                    let min = Self::number(o.get("min").unwrap_or(&Value::Null), random);
                    let max = Self::number(o.get("max").unwrap_or(&Value::Null), random);
                    // UniformGenerator.getFloat / getInt (Mth.nextInt) differ; use int for counts.
                    if max <= min {
                        min
                    } else {
                        min + random.next_float() * (max - min)
                    }
                }
                "minecraft:binomial" => {
                    let n = Self::number(o.get("n").unwrap_or(&Value::Null), random) as u32;
                    let p = Self::number(o.get("p").unwrap_or(&Value::Null), random);
                    (0..n).filter(|_| random.next_float() < p).count() as f32
                }
                // An unknown type gives 0, as in loot.rs `int` and `float`.
                _ => 0.0,
            },
            _ => 0.0,
        }
    }
    fn int_number<R: RandomSource>(v: &Value, random: &mut R) -> i32 {
        if let Value::Object(o) = v
            && o.get("type")
                .and_then(Value::as_str)
                .unwrap_or("minecraft:uniform")
                == "minecraft:uniform"
        {
            let min = Self::int_number(o.get("min").unwrap_or(&Value::Null), random);
            let max = Self::int_number(o.get("max").unwrap_or(&Value::Null), random);
            return if max <= min {
                min
            } else {
                min + random.next_int_bound(max - min + 1).unwrap_or(0)
            };
        }
        Self::number(v, random).round() as i32
    }
    fn condition<R: RandomSource>(&self, c: &Value, random: &mut R) -> bool {
        let (kind, c) = match c {
            Value::Null => return true,
            Value::Array(list) => return list.iter().all(|c| self.condition(c, random)),
            Value::String(s) => (s.as_str(), &Value::Null),
            Value::Object(o) => (o.get("type").and_then(Value::as_str).unwrap_or(""), c),
            _ => return false,
        };
        match kind {
            // ExplosionCondition: 1 / radius under a decaying explosion.
            "minecraft:survives_explosion" => self
                .explosion_radius
                .is_none_or(|r| random.next_float() <= 1.0 / r),
            "minecraft:tool/can_silk_touch" => self.enchantment_level("minecraft:silk_touch") > 0,
            "minecraft:tool/can_shear" => self.held.is_some_and(|h| &*h.item == "minecraft:shears"),
            "minecraft:inverted" => !self.condition(&c["term"], random),
            "minecraft:any_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| self.condition(t, random)),
            "minecraft:all_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|t| self.condition(t, random)),
            "minecraft:random_chance" => random.next_float() < Self::number(&c["chance"], random),
            "minecraft:table_bonus" => {
                // BonusLevelTableCondition: chances[min(level, len - 1)].
                let level =
                    self.enchantment_level(c["enchantment"].as_str().unwrap_or("")) as usize;
                let chances = c["chances"].as_array().map_or(&[][..], Vec::as_slice);
                let chance = chances
                    .get(level.min(chances.len().saturating_sub(1)))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32;
                random.next_float() < chance
            }
            "minecraft:match_block" | "minecraft:block_state_property" => {
                let blocks_ok = match &c["blocks"].as_str().or(c["block"].as_str()) {
                    Some(spec) => self.items.block_matches(spec, self.block),
                    None => true,
                };
                blocks_ok
                    && c["state"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .chain(c["properties"].as_object().into_iter().flatten())
                        .all(|(k, v)| {
                            let have = self.properties.get(k).map(String::as_str);
                            match v {
                                Value::String(s) => have == Some(s.as_str()),
                                Value::Object(r) => {
                                    let have: i64 = have.and_then(|h| h.parse().ok()).unwrap_or(0);
                                    r.get("min")
                                        .and_then(Value::as_i64)
                                        .is_none_or(|m| have >= m)
                                        && r.get("max")
                                            .and_then(Value::as_i64)
                                            .is_none_or(|m| have <= m)
                                }
                                _ => true,
                            }
                        })
            }
            "minecraft:match_tool" => {
                let items = &c["predicate"]["items"];
                let items_ok = match (self.held, items) {
                    (_, Value::Null) => self.held.is_some(),
                    (Some(h), Value::String(s)) => match s.strip_prefix('#') {
                        Some(tag) => self
                            .items
                            .item_tags
                            .get(tag)
                            .is_some_and(|t| t.contains(&*h.item)),
                        None => **s == *h.item,
                    },
                    _ => false,
                };
                // ItemPredicate "minecraft:enchantments": each entry needs a level in range.
                let enchants_ok = c["predicate"]["predicates"]["minecraft:enchantments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|e| {
                        let level =
                            self.enchantment_level(e["enchantments"].as_str().unwrap_or(""));
                        let min = e["levels"]["min"].as_u64().unwrap_or(1) as u32;
                        let max = e["levels"]["max"].as_u64().map_or(u32::MAX, |m| m as u32);
                        level >= min.max(1) && level <= max
                    });
                items_ok && enchants_ok
            }
            "minecraft:killed_by_player" => self.killed_by_player,
            "minecraft:random_chance_with_enchanted_bonus" => {
                let level = if self.killed_by_player && self.block.is_empty() {
                    self.enchantment_level(c["enchantment"].as_str().unwrap_or(""))
                } else {
                    0
                };
                let chance = if level > 0 {
                    crate::enchant::level_value(&c["enchanted_chance"], level)
                } else {
                    c["unenchanted_chance"].as_f64().unwrap_or(0.0) as f32
                };
                random.next_float() < chance
            }
            // `this` with no checks (snow, chorus flower) holds when an entity
            // broke the block (LootItemEntityPropertyCondition needs the entity).
            "minecraft:entity_properties"
                if self.this_entity
                    && c["entity"].as_str() == Some("this")
                    && c["predicate"].as_object().is_none_or(|p| p.is_empty()) =>
            {
                true
            }
            // Entity flags, equipment and damage-source predicates: none hold
            // for an unenchanted player kill without fire or lightning.
            "minecraft:entity_properties" | "minecraft:damage_source_properties" => false,
            // LocationCheck reads the world at its offset, which this engine
            // does not have: held true (a double plant's other half is taken
            // as there). Unknown types fail, as in loot.rs.
            "minecraft:location_check" => true,
            _ => false,
        }
    }
    fn conditions<R: RandomSource>(&self, o: &Value, random: &mut R) -> bool {
        if let Some(c) = o.get("condition")
            && !self.condition(c, random)
        {
            return false;
        }
        o.get("conditions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .all(|c| self.condition(c, random))
    }
    /// Expand entries; returns whether an entry succeeded (for alternatives).
    fn expand<R: RandomSource>(
        &self,
        e: &Value,
        random: &mut R,
        out: &mut Vec<(i32, ItemStack, Value)>,
    ) -> bool {
        if !self.conditions(e, random) {
            return false;
        }
        match e["type"].as_str().unwrap_or("") {
            "minecraft:item" => {
                out.push((
                    e["weight"].as_i64().unwrap_or(1) as i32,
                    ItemStack {
                        item: Arc::from(e["name"].as_str().unwrap_or("minecraft:air")),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    e["modifier"].clone(),
                ));
                true
            }
            "minecraft:alternatives" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    if self.expand(child, random, out) {
                        return true;
                    }
                }
                false
            }
            "minecraft:group" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    self.expand(child, random, out);
                }
                true
            }
            "minecraft:sequence" => {
                for child in e["children"].as_array().into_iter().flatten() {
                    if !self.expand(child, random, out) {
                        break;
                    }
                }
                true
            }
            // DynamicLoot: what the block entity adds (a cracked decorated
            // pot's sherds), filled in by `block_drops_with`.
            "minecraft:dynamic" => {
                out.push((
                    e["weight"].as_i64().unwrap_or(1) as i32,
                    ItemStack {
                        item: Arc::from(format!("{DYNAMIC}{}", e["name"].as_str().unwrap_or(""))),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    Value::Null,
                ));
                true
            }
            "minecraft:empty" => {
                out.push((
                    e["weight"].as_i64().unwrap_or(1) as i32,
                    ItemStack {
                        item: Arc::from("minecraft:air"),
                        count: 0,
                        damage: 0,
                        extra: None,
                    },
                    Value::Null,
                ));
                true
            }
            _ => false,
        }
    }
    fn choose<R: RandomSource>(
        &self,
        entries: &[Value],
        random: &mut R,
        produced: &mut Vec<ItemStack>,
    ) {
        let mut candidates = Vec::new();
        for e in entries {
            self.expand(e, random, &mut candidates);
        }
        if candidates.is_empty() {
            return;
        }
        let pick = if candidates.len() == 1 {
            0
        } else {
            let total: i32 = candidates.iter().map(|c| c.0.max(0)).sum();
            let mut r = random.next_int_bound(total.max(1)).unwrap_or(0);
            let mut chosen = 0;
            for (i, c) in candidates.iter().enumerate() {
                r -= c.0.max(0);
                if r < 0 {
                    chosen = i;
                    break;
                }
            }
            chosen
        };
        let (_, mut stack, modifiers) = candidates.swap_remove(pick);
        self.modifiers(&modifiers, &mut stack, random);
        produced.push(stack);
    }
    /// An enchantment function's `options` (`HolderSet`): a tag, a list or
    /// one id; every enchantment when absent (`registryAccess().listElements`).
    fn enchant_pool(e: &crate::enchant::Enchantments, options: &Value) -> Vec<Arc<str>> {
        match options {
            Value::String(s) if s.starts_with('#') => e.tag(s).to_vec(),
            Value::String(s) => vec![Arc::from(s.as_str())],
            Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(Arc::from)).collect(),
            _ => e.list.iter().map(|x| Arc::clone(&x.id)).collect(),
        }
    }
    fn modifiers<R: RandomSource>(&self, m: &Value, stack: &mut ItemStack, random: &mut R) {
        let list: Vec<&Value> = match m {
            Value::Array(a) => a.iter().collect(),
            Value::Object(_) => vec![m],
            _ => return,
        };
        for f in list {
            if !self.conditions(f, random) {
                continue;
            }
            match f["type"].as_str().or(f["function"].as_str()).unwrap_or("") {
                "minecraft:set_count" => {
                    let n = Self::int_number(&f["count"], random);
                    let base = if f["add"].as_bool() == Some(true) {
                        stack.count as i32
                    } else {
                        0
                    };
                    stack.count = (base + n).max(0) as u32;
                }
                "minecraft:apply_bonus" => {
                    let level =
                        self.enchantment_level(f["enchantment"].as_str().unwrap_or("")) as i32;
                    let count = stack.count as i32;
                    let params = &f["parameters"];
                    stack.count = match f["formula"].as_str().unwrap_or("") {
                        "minecraft:ore_drops" if level > 0 => {
                            let bonus = (random.next_int_bound(level + 2).unwrap_or(0) - 1).max(0);
                            count * (bonus + 1)
                        }
                        "minecraft:uniform_bonus_count" => {
                            let multiplier = params["bonusMultiplier"].as_i64().unwrap_or(1) as i32;
                            count + random.next_int_bound(multiplier * level + 1).unwrap_or(0)
                        }
                        "minecraft:binomial_with_bonus_count" => {
                            let (extra, p) = (
                                params["extra"].as_i64().unwrap_or(0) as i32,
                                params["probability"].as_f64().unwrap_or(0.0) as f32,
                            );
                            let mut n = count;
                            for _ in 0..level + extra {
                                if random.next_float() < p {
                                    n += 1;
                                }
                            }
                            n
                        }
                        _ => count,
                    }
                    .max(0) as u32;
                }
                "minecraft:enchanted_count_increase" => {
                    // Attacker's level x count, rounded (looting).
                    let level = self.enchantment_level(f["enchantment"].as_str().unwrap_or(""));
                    if level > 0 && self.killed_by_player && self.block.is_empty() {
                        let add = level as f32 * Self::number(&f["count"], random);
                        stack.count = (stack.count as i32 + add.round() as i32).max(0) as u32;
                        if let Some(limit) = f["limit"].as_i64().filter(|l| *l > 0) {
                            stack.count = stack.count.min(limit as u32);
                        }
                    }
                }
                // CopyBlockState: the listed properties of the broken block
                // become the item's `block_state` component.
                "minecraft:copy_state" => {
                    use horizon_world::nbt::Tag;
                    let mut copied = match stack.data().and_then(|d| d.other(BLOCK_STATE)) {
                        Some(Tag::Compound(c)) => c.clone(),
                        _ => Default::default(),
                    };
                    for name in f["properties"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        if let Some(value) = self.properties.get(name) {
                            copied.insert(name.into(), Tag::String(value.as_str().into()));
                        }
                    }
                    if !copied.is_empty() {
                        stack
                            .data_mut()
                            .set_other(BLOCK_STATE, Some(Tag::Compound(copied)));
                    }
                }
                // ApplyExplosionDecay: each item survives with 1 / radius.
                "minecraft:explosion_decay" => {
                    if let Some(radius) = self.explosion_radius {
                        let probability = 1.0 / radius;
                        stack.count = (0..stack.count)
                            .filter(|_| random.next_float() <= probability)
                            .count() as u32;
                    }
                }
                // SetPotionFunction (the barter's fire resistance potions).
                "minecraft:set_potion" => {
                    if let Some(id) = f["id"].as_str() {
                        *stack = std::mem::replace(stack, ItemStack::new("minecraft:air", 0))
                            .with_potion(id);
                    }
                }
                // EnchantWithLevelsFunction: EnchantmentHelper.enchantItem on
                // the context's random (the End cities' gear).
                "minecraft:enchant_with_levels" => {
                    let cost = Self::int_number(&f["levels"], random);
                    let e = &self.items.enchantments;
                    let pool = Self::enchant_pool(e, &f["options"]);
                    let chosen = e.select(self.items, random, &stack.item, cost, &pool);
                    // EnchantmentHelper.enchantItem: a book becomes one
                    // enchanted book (the fresh stack's count).
                    if &*stack.item == "minecraft:book" {
                        *stack = ItemStack::new("minecraft:enchanted_book", 1);
                    }
                    for (id, level) in chosen {
                        stack.enchant(&id, level);
                    }
                }
                // EnchantRandomlyFunction: one enchantment the item accepts,
                // at a level between its min and max.
                "minecraft:enchant_randomly" => {
                    let e = &self.items.enchantments;
                    let book = &*stack.item == "minecraft:book";
                    let only_compatible = f["only_compatible"].as_bool().unwrap_or(true);
                    let options: Vec<Arc<str>> = Self::enchant_pool(e, &f["options"])
                        .into_iter()
                        .filter(|id| {
                            book || !only_compatible || e.can_enchant(self.items, id, &stack.item)
                        })
                        .collect();
                    if options.is_empty() {
                        continue;
                    }
                    let id = options
                        [random.next_int_bound(options.len() as i32).unwrap_or(0) as usize]
                        .clone();
                    let max = e.get(&id).map_or(1, |d| d.max_level).max(1);
                    // `Mth.nextInt(random, 1, max)`: a single-level
                    // enchantment (Mending) returns 1 without drawing, so the
                    // rolls after this one keep vanilla's sequence.
                    let level = if max <= 1 {
                        1
                    } else {
                        1 + random.next_int_bound(max as i32).unwrap_or(0) as u32
                    };
                    if book {
                        *stack = ItemStack::new("minecraft:enchanted_book", 1);
                    }
                    stack.enchant(&id, level);
                }
                "minecraft:limit_count" => {
                    let limit = &f["limit"];
                    if let Some(min) = limit["min"].as_i64() {
                        stack.count = stack.count.max(min as u32);
                    }
                    if let Some(max) = limit["max"].as_i64() {
                        stack.count = stack.count.min(max as u32);
                    }
                }
                _ => {}
            }
        }
    }
}

/// `DispensibleContainerItem`: shared contents for hand use and dispensers.
pub fn bucket_contents(
    item: &str,
) -> Option<(horizon_world::level::BucketContent, &str, Option<&str>)> {
    use horizon_world::{fluid::FluidKind, level::BucketContent as C};
    Some(match item {
        "minecraft:water_bucket" => (
            C::Fluid(FluidKind::Water),
            "minecraft:item.bucket.empty",
            None,
        ),
        "minecraft:lava_bucket" => (
            C::Fluid(FluidKind::Lava),
            "minecraft:item.bucket.empty_lava",
            None,
        ),
        "minecraft:powder_snow_bucket" => {
            (C::Powder, "minecraft:item.bucket.empty_powder_snow", None)
        }
        "minecraft:cod_bucket" => (C::MobWater, "minecraft:item.bucket.empty_fish", Some("cod")),
        "minecraft:salmon_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("salmon"),
        ),
        "minecraft:pufferfish_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("pufferfish"),
        ),
        "minecraft:tropical_fish_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_fish",
            Some("tropical_fish"),
        ),
        "minecraft:axolotl_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_axolotl",
            Some("axolotl"),
        ),
        "minecraft:tadpole_bucket" => (
            C::MobWater,
            "minecraft:item.bucket.empty_tadpole",
            Some("tadpole"),
        ),
        "minecraft:sulfur_cube_bucket" => (
            C::Nothing,
            "minecraft:item.bucket.empty_sulfur_cube",
            Some("sulfur_cube"),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod bucket_use_tests {
    use super::*;
    #[test]
    fn every_filled_bucket_has_the_native_content_and_sound() {
        use horizon_world::level::BucketContent as C;
        assert_eq!(
            bucket_contents("minecraft:powder_snow_bucket"),
            Some((C::Powder, "minecraft:item.bucket.empty_powder_snow", None))
        );
        assert_eq!(
            bucket_contents("minecraft:tadpole_bucket"),
            Some((
                C::MobWater,
                "minecraft:item.bucket.empty_tadpole",
                Some("tadpole")
            ))
        );
        assert_eq!(
            bucket_contents("minecraft:sulfur_cube_bucket"),
            Some((
                C::Nothing,
                "minecraft:item.bucket.empty_sulfur_cube",
                Some("sulfur_cube")
            ))
        );
        for id in ["cod", "salmon", "pufferfish", "tropical_fish", "axolotl"] {
            assert!(bucket_contents(&format!("minecraft:{id}_bucket")).is_some());
        }
        assert!(bucket_contents("minecraft:bucket").is_none());
        assert!(bucket_contents("minecraft:milk_bucket").is_none());
    }
}

/// The items of the prepared resources, when they are on this machine, for
/// tests: the block registry and facts as the client's `Assets::load` reads them.
#[cfg(test)]
pub(crate) fn prepared_items() -> Option<Items> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client/26.3");
    if !root.join("items.json").exists() {
        return None;
    }
    let (registry, _) =
        BlockRegistry::from_facts_json_with_flags(fs::File::open(root.join("facts.json")).ok()?)
            .ok()?;
    let facts = BlockFacts::load(&root, &registry).ok()?;
    Items::load(&root, &registry, &facts).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;
    use serde_json::json;

    fn ctx<'a>(
        items: &'a Items,
        held: Option<&'a ItemStack>,
        props: &'a BTreeMap<String, String>,
        block: &'a str,
    ) -> LootContext<'a> {
        LootContext {
            items,
            block,
            properties: props,
            held,
            killed_by_player: block.is_empty(),
            explosion_radius: None,
            this_entity: false,
        }
    }

    #[test]
    fn fortune_silk_touch_and_looting() {
        let items = Items::for_tests(HashMap::new());
        let props = BTreeMap::new();
        let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
        pick.enchant("minecraft:fortune", 3);
        let ore = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:alternatives","children":[
            {"type":"minecraft:item","condition":"minecraft:tool/can_silk_touch","name":"minecraft:diamond_ore"},
            {"type":"minecraft:item","name":"minecraft:diamond","modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune","formula":"minecraft:ore_drops"}]}]}]}]});
        let mut random = LegacyRandom::new(7);
        let n = 20000;
        let total: u32 = (0..n)
            .map(|_| {
                ctx(&items, Some(&pick), &props, "minecraft:diamond_ore")
                    .evaluate(&ore, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        // Fortune III ore drops average (1 + 1 + 2 + 3 + 4) / 5 = 2.2 diamonds.
        let mean = f64::from(total) / f64::from(n);
        assert!((mean - 2.2).abs() < 0.05, "{mean}");
        let mut silk = ItemStack::new("minecraft:diamond_pickaxe", 1);
        silk.enchant("minecraft:silk_touch", 1);
        let drops =
            ctx(&items, Some(&silk), &props, "minecraft:diamond_ore").evaluate(&ore, &mut random);
        assert_eq!(&*drops[0].item, "minecraft:diamond_ore");
        let plain = ItemStack::new("minecraft:diamond_pickaxe", 1);
        let drops =
            ctx(&items, Some(&plain), &props, "minecraft:diamond_ore").evaluate(&ore, &mut random);
        assert_eq!((&*drops[0].item, drops[0].count), ("minecraft:diamond", 1));
        // Looting III adds level x uniform(0, 1) on player kills.
        let flesh = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:rotten_flesh","modifier":[
            {"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":0.0,"max":2.0}},
            {"type":"minecraft:enchanted_count_increase","count":{"type":"minecraft:uniform","min":0.0,"max":1.0},"enchantment":"minecraft:looting"}]}]}]});
        let mut sword = ItemStack::new("minecraft:iron_sword", 1);
        sword.enchant("minecraft:looting", 3);
        let looted: u32 = (0..n)
            .map(|_| {
                ctx(&items, Some(&sword), &props, "")
                    .evaluate(&flesh, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        let base: u32 = (0..n)
            .map(|_| {
                ctx(&items, None, &props, "")
                    .evaluate(&flesh, &mut random)
                    .iter()
                    .map(|s| s.count)
                    .sum::<u32>()
            })
            .sum();
        assert!(f64::from(looted) / f64::from(n) > f64::from(base) / f64::from(n) + 1.0);
        // Table bonus uses the chance for the tool's level.
        let leaves = json!({"pools":[{"rolls":1,"conditions":[{"type":"minecraft:table_bonus","enchantment":"minecraft:fortune","chances":[0.0,0.0,0.0,1.0]}],"entries":[{"type":"minecraft:item","name":"minecraft:apple"}]}]});
        assert_eq!(
            ctx(&items, Some(&pick), &props, "minecraft:oak_leaves")
                .evaluate(&leaves, &mut random)
                .len(),
            1
        );
        assert!(
            ctx(&items, Some(&plain), &props, "minecraft:oak_leaves")
                .evaluate(&leaves, &mut random)
                .is_empty()
        );
    }

    #[test]
    fn set_potion_gives_the_potion_its_contents() {
        let items = Items::for_tests(HashMap::new());
        let props = BTreeMap::new();
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:splash_potion",
            "modifier":{"type":"minecraft:set_potion","id":"minecraft:fire_resistance"}}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(1));
        assert_eq!(drops.len(), 1);
        assert_eq!(&*drops[0].item, "minecraft:splash_potion");
        assert_eq!(drops[0].potion(), Some("minecraft:fire_resistance"));
    }

    /// `/loot give` and every other table this dispatcher rolls: the two
    /// enchantment functions (`EnchantWithLevelsFunction`,
    /// `EnchantRandomlyFunction`) put their enchantments on the stack.
    #[test]
    fn loot_enchant_functions_enchant_the_rolled_item() {
        let Some(items) = super::prepared_items() else {
            return;
        };
        let props = BTreeMap::new();
        let mut random = LegacyRandom::new(11);
        // A sword at levels 20..39 comes out enchanted, as in an End city.
        let levels = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:diamond_sword","modifier":[
            {"type":"minecraft:enchant_with_levels","levels":{"type":"minecraft:uniform","min":20.0,"max":39.0}}]}]}]});
        let mut enchanted = 0;
        for _ in 0..40 {
            let drops = ctx(&items, None, &props, "").evaluate(&levels, &mut random);
            if drops[0].enchantments().is_empty() {
                continue;
            }
            enchanted += 1;
            assert_eq!(&*drops[0].item, "minecraft:diamond_sword");
            // Levels stay within the enchantment's range.
            for (id, level) in drops[0].enchantments() {
                let max = items
                    .enchantments
                    .get(id)
                    .expect("a known enchantment")
                    .max_level;
                assert!((1..=max).contains(level), "{id} at {level}");
            }
        }
        assert!(
            enchanted > 30,
            "only {enchanted} of 40 rolls were enchanted"
        );
        // `enchant_randomly` on a book makes one enchanted book, its levels
        // stored; the pool is the enchantment tag it names.
        let book = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:book","modifier":[
            {"type":"minecraft:enchant_randomly","options":"#minecraft:on_random_loot"}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&book, &mut random);
        assert_eq!(&*drops[0].item, "minecraft:enchanted_book");
        assert_eq!(drops[0].count, 1);
        assert!(drops[0].enchantments().is_empty());
        let stored = drops[0].stored_enchantments();
        assert_eq!(stored.len(), 1, "{stored:?}");
        assert!(
            items
                .enchantments
                .in_tag("minecraft:on_random_loot", &stored[0].0)
        );
        // An item no enchantment accepts keeps none of them.
        let dirt = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:dirt","modifier":[
            {"type":"minecraft:enchant_with_levels","levels":{"type":"minecraft:constant","value":30.0}}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&dirt, &mut random);
        assert!(drops[0].enchantments().is_empty());
    }

    /// `Mth.nextInt(random, 1, max)` returns `1` without drawing when the
    /// enchantment has one level (`EnchantRandomlyFunction.enchantItem`), so
    /// the rolls after it must not shift. Vanilla with `LegacyRandom(11)`: a
    /// Mending book, then `set_count` uniform 1..64, is 46.
    #[test]
    fn a_single_level_enchantment_draws_no_level() {
        let Some(items) = super::prepared_items() else {
            return;
        };
        let props = BTreeMap::new();
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
            "name":"minecraft:book","modifier":[
            {"type":"minecraft:enchant_randomly","options":"minecraft:mending"},
            {"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1.0,"max":64.0}}]}]}]});
        let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(11));
        assert_eq!(drops.len(), 1);
        assert_eq!(&*drops[0].item, "minecraft:enchanted_book");
        assert_eq!(
            drops[0].stored_enchantments(),
            [("minecraft:mending".to_owned().into(), 1)],
            "the book keeps its single-level enchantment"
        );
        // 46 is vanilla's count for this seed; an extra level draw gives 28.
        assert_eq!(drops[0].count, 46);
    }
}

/// Minecraft.handleKeybinds: spectators cannot drop from the selected slot.
pub fn take_held_drop(
    slot: &mut Option<ItemStack>,
    all: bool,
    spectator: bool,
) -> Option<ItemStack> {
    if spectator {
        return None;
    }
    let stack = slot.as_mut()?;
    if stack.count == 0 {
        return None;
    }
    let count = if all { stack.count } else { 1 };
    let thrown = ItemStack {
        count,
        ..stack.clone()
    };
    stack.count -= count;
    if stack.count == 0 {
        *slot = None;
    }
    Some(thrown)
}
#[cfg(test)]
mod held_drop_tests {
    use super::*;
    #[test]
    fn one_all_and_empty_drop() {
        let mut slot = Some(ItemStack::new("minecraft:stone", 4));
        assert_eq!(take_held_drop(&mut slot, false, false).unwrap().count, 1);
        assert_eq!(slot.as_ref().unwrap().count, 3);
        assert_eq!(take_held_drop(&mut slot, true, false).unwrap().count, 3);
        assert!(slot.is_none());
        assert!(take_held_drop(&mut slot, false, false).is_none());
    }
    #[test]
    fn spectator_keeps_selected_stack_for_single_and_whole_stack_drops() {
        for all in [false, true] {
            let mut slot = Some(ItemStack::new("minecraft:stone", 4));
            assert!(take_held_drop(&mut slot, all, true).is_none());
            assert_eq!(slot.as_ref().unwrap().count, 4);
            // Leaving spectator allows dropping the same selected stack again.
            assert_eq!(
                take_held_drop(&mut slot, all, false).unwrap().count,
                if all { 4 } else { 1 }
            );
        }
    }
}
