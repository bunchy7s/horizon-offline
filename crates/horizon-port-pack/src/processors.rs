//! Structure processors the structure libraries write in Java, carried over
//! as data the game runs: the ones that give spawners their mob.
//!
//! - Moog's Structure Lib's and Repurposed Structures' spawner randomizing
//!   processors pick a mob from a weighted list for each spawner. They become
//!   `horizon:spawner_mobs` (`horizon_world::worldgen::structure::processor`),
//!   with Repurposed's `rs_spawners` list written in.
//! - Moog's vanilla loot swap only acts when its mod's "replace vanilla"
//!   preset is on; one that ships off is a no-op and is taken out.
//! - Repurposed Structures' mineshaft sky view becomes the game's
//!   `horizon:mineshaft_sky_view`.
//!
//! Lithostitched's processors and Choice Theorem's Overhauled Village's
//! compat hook are carried by `crate::lithostitched`, called from [`carry`].
//!
//! What is not here is taken out of its list later and counted
//! (`worldgen::upgrade`). The entity NBT is written in 26.3's shape (an
//! `equipment` map, not 1.21.1's `HandItems` and `ArmorItems`).
use crate::lithostitched;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek},
};

/// What the processors read from the mod's own data files.
#[derive(Debug, Default)]
pub struct ProcessorData {
    /// Repurposed Structures' `rs_spawners/<path>.json` by id: each mob and
    /// its weight, without the weight-0 entries its loader removes.
    pub spawners: BTreeMap<String, Vec<(String, f64)>>,
    /// Moog's "replace vanilla" presets that are on by default:
    /// `(mod id, vanilla_key)`.
    pub replace_vanilla: BTreeSet<(String, String)>,
    /// The mod's processor lists by id, for Lithostitched's references.
    pub lists: BTreeMap<String, Value>,
    /// The mods named by its `ctov:modular_compat` processors that are
    /// ported along ([`Self::mark_loaded`]).
    pub compat_loaded: BTreeSet<String>,
}

impl ProcessorData {
    /// Notes which mods its compat processors name are `loaded`.
    pub fn mark_loaded(&mut self, loaded: &dyn Fn(&str) -> bool) {
        lithostitched::mark_loaded(self, loaded);
    }

    pub fn read<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Self {
        let mut out = Self::default();
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else {
                continue;
            };
            let Ok(name) = entry.name().map(|n| n.into_owned()) else {
                continue;
            };
            let Some((ns, rest)) = name
                .strip_prefix("data/")
                .and_then(|r| r.strip_suffix(".json"))
                .and_then(|r| r.split_once('/'))
            else {
                continue;
            };
            let spawners = rest.strip_prefix("rs_spawners/");
            let list = rest.strip_prefix("worldgen/processor_list/");
            if spawners.is_none() && list.is_none() && rest != "moogs_structures/replace_vanilla" {
                continue;
            }
            let mut text = String::new();
            let Ok(v) = entry
                .read_to_string(&mut text)
                .map_err(|_| ())
                .and_then(|_| serde_json::from_str::<Value>(&text).map_err(|_| ()))
            else {
                continue;
            };
            if let Some(path) = list {
                out.lists.insert(format!("{ns}:{path}"), v);
                continue;
            }
            if let Some(path) = spawners {
                let mobs = v["mobs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| Some((m["name"].as_str()?.to_owned(), m["weight"].as_f64()?)))
                    .filter(|(_, w)| *w != 0.)
                    .collect();
                out.spawners.insert(format!("{ns}:{path}"), mobs);
                continue;
            }
            for preset in v["presets"].as_array().into_iter().flatten() {
                if preset["default_enabled"].as_bool() != Some(true) {
                    continue;
                }
                for r in preset["replacements"].as_array().into_iter().flatten() {
                    if let Some(key) = r["vanilla_key"].as_str() {
                        out.replace_vanilla.insert((ns.to_owned(), key.to_owned()));
                    }
                }
            }
        }
        out
    }
}

/// Calls `f` on each processor list in a worldgen file: a `processor_list`
/// file, and the inline lists of a template pool's elements (list elements
/// included). Whether any call changed something.
pub fn each_list(kind: &str, v: &mut Value, f: &mut dyn FnMut(&mut Value) -> bool) -> bool {
    match kind {
        "processor_list" => f(v),
        "template_pool" => {
            let mut changed = false;
            for e in v["elements"].as_array_mut().into_iter().flatten() {
                let element = &mut e["element"];
                if let Some(list) = element.get_mut("elements").and_then(Value::as_array_mut) {
                    for inner in list {
                        if let Some(p) = inner.get_mut("processors") {
                            changed |= f(p);
                        }
                    }
                } else if let Some(p) = element.get_mut("processors") {
                    changed |= f(p);
                }
            }
            changed
        }
        _ => false,
    }
}

/// The processors of a list: `{"processors": [...]}` or a bare list (an id
/// names another file and is left alone).
pub fn processors(list: &mut Value) -> Option<&mut Vec<Value>> {
    match list {
        Value::Array(a) => Some(a),
        Value::Object(o) => o.get_mut("processors").and_then(Value::as_array_mut),
        _ => None,
    }
}

/// Rewrites the processors of a worldgen file that have a data form; each is
/// counted by type in `carried`. Whether anything changed.
pub fn carry(
    kind: &str,
    v: &mut Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> bool {
    each_list(kind, v, &mut |list| {
        let Some(list) = processors(list) else {
            return false;
        };
        let (out, changed) = carry_all(std::mem::take(list), data, carried);
        *list = out;
        changed
    })
}

/// The processors of one list carried: references written in, then each
/// one with a data form replaced by it. Whether anything changed.
pub(crate) fn carry_all(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    carry_at(list, data, carried, 0)
}

/// [`carry_all`] for a list `depth` pillars deep (see [`pillar`]).
fn carry_at(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
    depth: usize,
) -> (Vec<Value>, bool) {
    let (expanded, mut changed) = lithostitched::expand_references(list, data, carried);
    let mut out = Vec::with_capacity(expanded.len());
    for p in expanded {
        let kind = p["processor_type"].as_str().unwrap_or_default().to_owned();
        // Lithostitched's processors stay theirs, with what they hold carried.
        let replacement = lithostitched::processor(&kind, &p, data, carried).or_else(|| {
            match pillar_type(&kind) {
                Some(to) => Some(vec![pillar(&p, to, data, carried, depth)]),
                None => as_data(&kind, &p, data),
            }
        });
        match replacement {
            Some(replacement) => {
                *carried.entry(kind).or_default() += 1;
                out.extend(replacement);
                changed = true;
            }
            None => out.push(p),
        }
    }
    (out, changed)
}

/// The game's type for the mods' processors that place blocks past their
/// template, which keep their fields (`horizon-world`'s `pillars.rs`).
fn pillar_type(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "repurposed_structures:pillar_processor" | "moogs_structures:pillar_processor" => {
            "horizon:pillar"
        }
        "repurposed_structures:bottom_pillar_processor" => "horizon:bottom_pillar",
        "repurposed_structures:close_off_fluid_sources_processor"
        | "moogs_structures:close_off_fluid_sources_processor" => "horizon:close_off_fluid_sources",
        "repurposed_structures:close_off_air_sources_processor" => "horizon:close_off_air_sources",
        _ => return None,
    })
}

/// A pillar processor as the game's `to`. Its `pillar_processor_list`
/// (which runs on each block of the column; Repurposed's often names the
/// list it is in) is written in, carried; the pillars inside that copy lose
/// theirs, as a column's blocks are never the mods' triggers.
fn pillar(
    p: &Value,
    to: &str,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
    depth: usize,
) -> Value {
    let mut out = p.clone();
    out["processor_type"] = to.into();
    let Some(o) = out.as_object_mut() else {
        return out;
    };
    let Some(list) = o.remove("pillar_processor_list") else {
        return out;
    };
    let inner = match &list {
        Value::String(id) if id == "minecraft:empty" => None,
        _ if depth > 0 => None,
        other => referenced(other, data),
    };
    if let Some(inner) = inner {
        let (inner, _) = carry_at(inner, data, carried, depth + 1);
        o.insert("pillar_processor_list".into(), json!({"processors": inner}));
    }
    out
}

/// Repurposed Structures' rule tests and the game's types that run them
/// (`horizon_world::worldgen`'s `rule_test` and `structure::processor`).
const RULE_TESTS: [(&str, &str); 3] = [
    (
        "repurposed_structures:matter_phase_rule_test",
        "horizon:matter_phase",
    ),
    (
        "repurposed_structures:piece_origin_axis_aligned_linear_pos_rule_test",
        "horizon:piece_origin_axis_aligned_linear_pos",
    ),
    (
        "repurposed_structures:y_value_pos_rule_test",
        "horizon:y_value",
    ),
];

/// Renames the rule tests written in Java that the game runs under its own
/// type ([`RULE_TESTS`]) in a worldgen file's processor lists; each is
/// counted by type in `carried`. Their fields are the mod's. Whether
/// anything changed. Without this the game, like vanilla, refuses the list
/// and every pool naming it: Repurposed's mineshafts and Nether monument.
pub fn rule_tests(kind: &str, v: &mut Value, carried: &mut BTreeMap<String, usize>) -> bool {
    fn rename(v: &mut Value, carried: &mut BTreeMap<String, usize>) -> bool {
        match v {
            Value::Object(o) => {
                let mut changed = false;
                if let Some(Value::String(t)) = o.get_mut("predicate_type")
                    && let Some((old, new)) = RULE_TESTS.iter().find(|(old, _)| t == old)
                {
                    *carried.entry((*old).to_owned()).or_default() += 1;
                    *t = (*new).to_owned();
                    changed = true;
                }
                o.values_mut().fold(changed, |c, x| rename(x, carried) | c)
            }
            Value::Array(a) => a.iter_mut().fold(false, |c, x| rename(x, carried) | c),
            _ => false,
        }
    }
    each_list(kind, v, &mut |list| rename(list, carried))
}

/// The processors of the lists a reference names: an id, an inline list, or
/// a list of either.
pub(crate) fn referenced(lists: &Value, data: &ProcessorData) -> Option<Vec<Value>> {
    let one = |l: &Value| -> Option<Vec<Value>> {
        let mut list = match l {
            Value::String(id) if !id.starts_with('#') => {
                let id = if id.contains(':') {
                    id.clone()
                } else {
                    format!("minecraft:{id}")
                };
                data.lists.get(&id)?.clone()
            }
            Value::Object(_) | Value::Array(_) => l.clone(),
            _ => return None,
        };
        processors(&mut list).map(std::mem::take)
    };
    match lists {
        // A bare list of processors is one inline list; else a list of lists.
        Value::Array(a) if a.iter().any(|x| x.get("processor_type").is_some()) => Some(a.clone()),
        Value::Array(a) => Some(a.iter().map(one).collect::<Option<Vec<_>>>()?.concat()),
        other => one(other),
    }
}

/// The data form of a Java processor: the processors to put in its place
/// (none for a no-op), or `None` when it has none.
fn as_data(kind: &str, p: &Value, data: &ProcessorData) -> Option<Vec<Value>> {
    kind.split_once(':')?;
    Some(match kind {
        // Its structure voids keep the world's block, as vanilla's
        // block_ignore does (the templates hold them; vanilla places them).
        "repurposed_structures:structure_void_processor" => vec![json!({
            "processor_type": "minecraft:block_ignore",
            "blocks": ["minecraft:structure_void"]})],
        "moogs_structures:spawner_randomizing_processor" => {
            let mobs = p["weighted_entities"]
                .as_array()?
                .iter()
                .map(|e| {
                    // The entry's NBT, then its id over it.
                    let mut entity = match &e["nbt"] {
                        Value::Object(o) => o.clone(),
                        _ => serde_json::Map::new(),
                    };
                    entity.insert("id".into(), e["entity"].as_str()?.into());
                    Some(json!({"entity": entity, "weight": e["weight"].as_f64()?}))
                })
                .collect::<Option<Vec<_>>>()?;
            vec![spawner_mobs(p, mobs, None)]
        }
        "repurposed_structures:spawner_randomizing_processor" => {
            // A list the mod does not ship falls back to a vanilla dungeon
            // mob in the mod; left out here.
            let list = data
                .spawners
                .get(p["rs_spawner_resourcelocation"].as_str()?)?;
            let mobs = list
                .iter()
                .map(|(id, weight)| json!({"entity": {"id": id}, "weight": weight}))
                .collect();
            vec![spawner_mobs(p, mobs, repurposed_light(p)?)]
        }
        // Wet where the world holds water, dry where it does not.
        "repurposed_structures:waterlogging_when_replacing_water_processor" => {
            vec![json!({"processor_type": "horizon:waterlog_in_world_water"})]
        }
        // Its mineshafts: nothing above the ocean floor but grounded rails.
        "repurposed_structures:mineshaft_sky_view_processor" => {
            vec![json!({"processor_type": "horizon:mineshaft_sky_view"})]
        }
        "moogs_structures:vanilla_loot_swap_processor" => {
            let key = (
                p["modid"].as_str()?.to_owned(),
                p["vanilla_key"].as_str()?.to_owned(),
            );
            if data.replace_vanilla.contains(&key) {
                return None;
            }
            Vec::new()
        }
        "moogs_structures:trial_spawner_randomizing_processor" => {
            // The configs by id: 26.3 reads them from `trial_spawner/`, the
            // folder Moog's loads them from.
            let mut nbt = json!({"normal_config": p["normal_config"].as_str()?});
            if let Some(ominous) = p["ominous_config"].as_str() {
                nbt["ominous_config"] = ominous.into();
            }
            // The state the template was saved with goes.
            let saved = [
                "server_data",
                "shared_data",
                "spawn_data",
                "cooldown_end_at_tick",
                "next_mob_spawns_at",
            ];
            vec![same_state_rule(&TRIAL_SPAWNER_STATES, &saved, |_| {
                nbt.clone()
            })]
        }
        "moogs_structures:vault_randomizing_processor" => {
            let normal = p["loot_table"].as_str()?;
            let config = |ominous: bool| {
                let (table, key) = match ominous {
                    true => (
                        p["ominous_loot_table"].as_str().unwrap_or(normal),
                        p["ominous_key_item"]
                            .as_str()
                            .unwrap_or("minecraft:ominous_trial_key"),
                    ),
                    false => (
                        normal,
                        p["key_item"].as_str().unwrap_or("minecraft:trial_key"),
                    ),
                };
                // Merged into the vault's config, as the mod puts both in.
                json!({"config": {"loot_table": table, "key_item": {"id": key, "count": 1}}})
            };
            vec![same_state_rule(
                &VAULT_STATES,
                &["server_data", "shared_data"],
                |s| config(s.get("ominous") == Some(&"true")),
            )]
        }
        _ => return None,
    })
}

/// A block's states, as (block, [(property, values)]).
type States = (
    &'static str,
    &'static [(&'static str, &'static [&'static str])],
);
const TRIAL_SPAWNER_STATES: States = (
    "minecraft:trial_spawner",
    &[
        (
            "trial_spawner_state",
            &[
                "inactive",
                "waiting_for_players",
                "active",
                "waiting_for_reward_ejection",
                "ejecting_reward",
                "cooldown",
            ],
        ),
        ("ominous", &["false", "true"]),
    ],
);
const VAULT_STATES: States = (
    "minecraft:vault",
    &[
        ("facing", &["north", "south", "west", "east"]),
        (
            "vault_state",
            &["inactive", "active", "unlocking", "ejecting"],
        ),
        ("ominous", &["false", "true"]),
    ],
);

/// Moog's trial spawner and vault processors keep the block, take keys out
/// of its block entity and set others. As vanilla's `rule`: one rule per
/// state of the block, matching it and writing it back, with the game's
/// `horizon:edit` modifier taking out `remove` and merging `nbt(state)` in.
fn same_state_rule(
    states: &States,
    remove: &[&str],
    nbt: impl Fn(&BTreeMap<&str, &str>) -> Value,
) -> Value {
    let (block, properties) = states;
    let mut all = vec![BTreeMap::new()];
    for (key, values) in properties.iter() {
        all = all
            .into_iter()
            .flat_map(|s| {
                values.iter().map(move |v| {
                    let mut s = s.clone();
                    s.insert(*key, *v);
                    s
                })
            })
            .collect();
    }
    let rules: Vec<Value> = all
        .iter()
        .map(|s| {
            let state = json!({"id": block, "properties": s});
            json!({
                "input_predicate": {"predicate_type": "minecraft:blockstate_match", "block_state": state},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": state,
                "block_entity_modifier": {"type": "horizon:edit", "remove": remove, "data": nbt(s)},
            })
        })
        .collect();
    json!({"processor_type": "minecraft:rule", "rules": rules})
}

/// An entity's NBT in 26.3's shape, in place: 1.21.1's `ArmorItems` (feet
/// to head), `HandItems` (main hand, off hand) and their drop chances become
/// `equipment` and `drop_chances` by slot; empty items and the default drop
/// chance (0.085) are left out, as 26.3 saves them. Whether it changed.
pub fn upgrade_entity(entity: &mut Value) -> bool {
    let Some(o) = entity.as_object_mut() else {
        return false;
    };
    const ARMOR: [&str; 4] = ["feet", "legs", "chest", "head"];
    const HANDS: [&str; 2] = ["mainhand", "offhand"];
    let mut changed = false;
    for (items, chances, slots) in [
        ("ArmorItems", "ArmorDropChances", &ARMOR[..]),
        ("HandItems", "HandDropChances", &HANDS[..]),
    ] {
        if let Some(Value::Array(list)) = o.remove(items) {
            changed = true;
            for (slot, item) in slots.iter().zip(list) {
                if item.get("id").is_some() {
                    let equipment = o.entry("equipment").or_insert_with(|| json!({}));
                    equipment[*slot] = item;
                }
            }
        }
        if let Some(Value::Array(list)) = o.remove(chances) {
            changed = true;
            for (slot, chance) in slots.iter().zip(list) {
                if chance.as_f64().is_some_and(|c| (c - 0.085).abs() > 1e-6) {
                    let drop = o.entry("drop_chances").or_insert_with(|| json!({}));
                    drop[*slot] = chance;
                }
            }
        }
    }
    changed
}

/// A 1.21.1 trial spawner config (`trial_spawner/`) in 26.3's shape, in
/// place: its mobs' NBT ([`upgrade_entity`]). Whether it changed.
pub fn upgrade_trial_spawner(config: &mut Value) -> bool {
    let mut changed = false;
    for p in config["spawn_potentials"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        changed |= upgrade_entity(&mut p["data"]["entity"]);
    }
    changed
}

/// [`as_data`]'s `horizon:spawner_mobs` for the mods' spawner randomizing
/// processors, whose fields have the same names and defaults.
fn spawner_mobs(p: &Value, mobs: Vec<Value>, rules: Option<Value>) -> Value {
    let mut out = json!({"processor_type": "horizon:spawner_mobs", "mobs": mobs});
    for key in [
        "delay",
        "min_spawn_delay",
        "max_spawn_delay",
        "spawn_count",
        "max_nearby_entities",
        "required_player_range",
        "spawn_range",
    ] {
        if let Some(n) = p.get(key).filter(|n| n.is_i64()) {
            out[key] = n.clone();
        }
    }
    if let Some(r) = rules {
        out["custom_spawn_rules"] = r;
    }
    if let Some(b) = p.get("spawner_replacement_block") {
        out["replacement"] = b.clone();
    }
    out
}

/// Repurposed Structures' `custom_spawn_rules` from its light level ranges:
/// `Some(None)` without either, `None` when one does not read. The sky
/// light's upper bound is written as `max_exclusive`, as the mod does.
fn repurposed_light(p: &Value) -> Option<Option<Value>> {
    let range = |key: &str| -> Option<Option<(i64, i64)>> {
        match p.get(key) {
            None => Some(None),
            Some(Value::Number(n)) => n.as_i64().map(|n| Some((n, n))),
            Some(v) => Some(Some((
                v["min_inclusive"].as_i64()?,
                v["max_inclusive"].as_i64()?,
            ))),
        }
    };
    let (block, sky) = (
        range("valid_block_light_level")?,
        range("valid_sky_light_level")?,
    );
    if block.is_none() && sky.is_none() {
        return Some(None);
    }
    let mut rules = json!({});
    if let Some((min, max)) = block {
        rules["block_light_limit"] = json!({"min_inclusive": min, "max_inclusive": max});
    }
    if let Some((min, max)) = sky {
        rules["sky_light_limit"] = json!({"min_inclusive": min, "max_exclusive": max});
    }
    Some(Some(rules))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> ProcessorData {
        let mut d = ProcessorData::default();
        d.spawners.insert(
            "repurposed_structures:dungeons/badlands".into(),
            vec![
                ("minecraft:husk".into(), 10.),
                ("minecraft:cave_spider".into(), 3.),
            ],
        );
        d.replace_vanilla.insert(("mvs".into(), "village".into()));
        d
    }

    #[test]
    fn spawner_processors_become_data() {
        let mut list = json!({"processors": [
            {"processor_type": "moogs_structures:spawner_randomizing_processor",
             "weighted_entities": [{"entity": "minecraft:blaze", "weight": 2, "nbt": {"Health": 5}}],
             "spawn_count": 2, "spawner_replacement_block": {"Name": "minecraft:stone"}},
            {"processor_type": "repurposed_structures:spawner_randomizing_processor",
             "rs_spawner_resourcelocation": "repurposed_structures:dungeons/badlands",
             "valid_block_light_level": {"min_inclusive": 0, "max_inclusive": 7}},
            {"processor_type": "repurposed_structures:spawner_randomizing_processor",
             "rs_spawner_resourcelocation": "repurposed_structures:missing"},
            {"processor_type": "minecraft:gravity", "heightmap": "WORLD_SURFACE_WG", "offset": 0},
            {"processor_type": "moogs_structures:vanilla_loot_swap_processor",
             "modid": "mns", "vanilla_key": "fortress", "loot_table_mapping": {}},
            {"processor_type": "moogs_structures:vanilla_loot_swap_processor",
             "modid": "mvs", "vanilla_key": "village", "loot_table_mapping": {}}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        assert_eq!(p.len(), 5, "{p:#?}");
        assert_eq!(
            p[0],
            json!({"processor_type": "horizon:spawner_mobs",
                   "mobs": [{"entity": {"Health": 5, "id": "minecraft:blaze"}, "weight": 2.0}],
                   "spawn_count": 2, "replacement": {"Name": "minecraft:stone"}})
        );
        assert_eq!(
            p[1],
            json!({"processor_type": "horizon:spawner_mobs",
                   "mobs": [{"entity": {"id": "minecraft:husk"}, "weight": 10.0},
                            {"entity": {"id": "minecraft:cave_spider"}, "weight": 3.0}],
                   "custom_spawn_rules": {"block_light_limit": {"min_inclusive": 0, "max_inclusive": 7}}})
        );
        // Unknown list, vanilla processor, and a swap whose preset is on: kept
        // (the Java ones are dropped later); the swap that is off is gone.
        assert_eq!(
            p[2]["rs_spawner_resourcelocation"],
            "repurposed_structures:missing"
        );
        assert_eq!(p[3]["processor_type"], "minecraft:gravity");
        assert_eq!(p[4]["modid"], "mvs");
        assert_eq!(
            carried,
            BTreeMap::from([
                ("moogs_structures:spawner_randomizing_processor".into(), 1),
                ("moogs_structures:vanilla_loot_swap_processor".into(), 1),
                (
                    "repurposed_structures:spawner_randomizing_processor".into(),
                    1
                ),
            ])
        );
    }

    #[test]
    fn trial_spawners_and_vaults_drop_their_saved_state() {
        let mut list = json!({"processors": [
            {"processor_type": "moogs_structures:trial_spawner_randomizing_processor",
             "normal_config": "mns:arena/normal", "ominous_config": "mns:arena/ominous"},
            {"processor_type": "moogs_structures:vault_randomizing_processor",
             "loot_table": "mns:vaults/main", "ominous_loot_table": "mns:vaults/ominous"}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        let rules = |i: usize| p[i]["rules"].as_array().unwrap().clone();
        // Every state of the block: 6 x 2 trial spawners, 4 x 4 x 2 vaults.
        assert_eq!((rules(0).len(), rules(1).len()), (12, 32));
        let spawner = &rules(0)[0];
        assert_eq!(
            spawner["input_predicate"]["block_state"],
            spawner["output_state"]
        );
        assert_eq!(
            spawner["block_entity_modifier"],
            json!({"type": "horizon:edit",
                   "remove": ["server_data", "shared_data", "spawn_data",
                              "cooldown_end_at_tick", "next_mob_spawns_at"],
                   "data": {"normal_config": "mns:arena/normal",
                            "ominous_config": "mns:arena/ominous"}})
        );
        for rule in rules(1) {
            let ominous = rule["output_state"]["properties"]["ominous"] == "true";
            let (table, key) = match ominous {
                true => ("mns:vaults/ominous", "minecraft:ominous_trial_key"),
                false => ("mns:vaults/main", "minecraft:trial_key"),
            };
            assert_eq!(
                rule["block_entity_modifier"],
                json!({"type": "horizon:edit", "remove": ["server_data", "shared_data"],
                       "data": {"config": {"loot_table": table,
                                           "key_item": {"id": key, "count": 1}}}})
            );
        }
        assert_eq!(carried.len(), 2);
    }

    #[test]
    fn trial_spawner_mobs_take_26_3s_equipment() {
        let mut config = json!({"spawn_potentials": [{"weight": 1, "data": {"entity": {
            "id": "minecraft:skeleton",
            "ArmorItems": [{}, {}, {}, {"id": "minecraft:iron_helmet", "count": 1}],
            "ArmorDropChances": [0.085, 0.085, 0.085, 0.0],
            "HandItems": [{"id": "minecraft:bow", "count": 1}, {}],
            "HandDropChances": [0.085, 0.085]}}}]});
        assert!(upgrade_trial_spawner(&mut config));
        assert_eq!(
            config["spawn_potentials"][0]["data"]["entity"],
            json!({"id": "minecraft:skeleton",
                   "equipment": {"head": {"id": "minecraft:iron_helmet", "count": 1},
                                 "mainhand": {"id": "minecraft:bow", "count": 1}},
                   "drop_chances": {"head": 0.0}})
        );
        assert!(!upgrade_trial_spawner(&mut config));
    }

    #[test]
    fn repurposed_waterlogging_follows_the_world() {
        let mut list = json!({"processors": [{"processor_type":
            "repurposed_structures:waterlogging_when_replacing_water_processor"},
            {"processor_type": "repurposed_structures:mineshaft_sky_view_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry(
            "processor_list",
            &mut list,
            &ProcessorData::default(),
            &mut carried
        ));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "horizon:waterlog_in_world_water"},
                {"processor_type": "horizon:mineshaft_sky_view"}])
        );
    }

    #[test]
    fn repurposed_rule_tests_become_the_games() {
        let rule = |input: Value, position: Value| {
            json!({"input_predicate": input, "location_predicate": {"predicate_type":
                "repurposed_structures:matter_phase_rule_test", "phase_to_test_for": "AIR"},
                "position_predicate": position, "output_state": {"id": "minecraft:lava"}})
        };
        let mut pool = json!({"elements": [{"weight": 1, "element": {
            "element_type": "minecraft:single_pool_element", "processors": {"processors": [
                {"processor_type": "minecraft:rule", "rules": [rule(
                    json!({"predicate_type": "minecraft:always_true"}),
                    json!({"predicate_type": "repurposed_structures:y_value_pos_rule_test",
                        "min_y_value": 0, "max_y_value": 31}))]},
                {"processor_type": "minecraft:capped", "limit": 2, "delegate": {
                    "processor_type": "minecraft:rule", "rules": [rule(
                        json!({"predicate_type": "minecraft:block_match", "block": "minecraft:rail"}),
                        json!({"predicate_type":
                            "repurposed_structures:piece_origin_axis_aligned_linear_pos_rule_test",
                            "axis": "y", "min_dist": 0, "max_dist": 1}))]}}]}}}]});
        let mut carried = BTreeMap::new();
        assert!(rule_tests("template_pool", &mut pool, &mut carried));
        let list = &pool["elements"][0]["element"]["processors"]["processors"];
        let first = &list[0]["rules"][0];
        assert_eq!(
            first["location_predicate"]["predicate_type"],
            "horizon:matter_phase"
        );
        assert_eq!(first["location_predicate"]["phase_to_test_for"], "AIR");
        assert_eq!(
            first["position_predicate"]["predicate_type"],
            "horizon:y_value"
        );
        assert_eq!(first["position_predicate"]["max_y_value"], 31);
        let delegated = &list[1]["delegate"]["rules"][0];
        assert_eq!(
            delegated["position_predicate"]["predicate_type"],
            "horizon:piece_origin_axis_aligned_linear_pos"
        );
        assert_eq!(
            delegated["input_predicate"]["predicate_type"],
            "minecraft:block_match"
        );
        assert_eq!(carried.values().sum::<usize>(), 4);
        assert_eq!(carried["repurposed_structures:matter_phase_rule_test"], 2);
        // A file holding none is left as it is.
        assert!(!rule_tests("template_pool", &mut pool, &mut carried));
        assert!(!rule_tests(
            "structure",
            &mut json!({"predicate_type":
            "repurposed_structures:y_value_pos_rule_test"}),
            &mut carried
        ));
    }

    #[test]
    fn repurposed_structure_voids_keep_the_world() {
        let mut list = json!({"processors": [
            {"processor_type": "repurposed_structures:structure_void_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry(
            "processor_list",
            &mut list,
            &ProcessorData::default(),
            &mut carried
        ));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "minecraft:block_ignore",
                "blocks": ["minecraft:structure_void"]}])
        );
        assert_eq!(carried["repurposed_structures:structure_void_processor"], 1);
    }

    #[test]
    fn pillars_become_the_games() {
        let mut d = ProcessorData::default();
        // Repurposed's city list names itself as its pillar's list.
        let city = json!({"processors": [
            {"processor_type": "repurposed_structures:pillar_processor", "direction": "down",
             "pillar_length": 15, "pillar_processor_list": "repurposed_structures:cities/x",
             "pillar_trigger_and_replacements": [{"trigger": {"Name": "minecraft:red_stained_glass"},
                 "replacement": {"Name": "minecraft:cobblestone"}}]},
            {"processor_type": "minecraft:block_rot", "integrity": 0.5}]});
        d.lists
            .insert("repurposed_structures:cities/x".into(), city.clone());
        let mut list = city;
        list["processors"].as_array_mut().unwrap().extend([
            json!({"processor_type": "repurposed_structures:bottom_pillar_processor",
                   "pillar_processor_list": "minecraft:empty"}),
            json!({"processor_type": "moogs_structures:close_off_fluid_sources_processor",
                   "weighted_list_of_replacement_blocks": [{"block": "minecraft:stone", "weight": 1}]}),
            json!({"processor_type": "repurposed_structures:close_off_air_sources_processor",
                   "weighted_list_of_replacement_blocks": [{"block": "minecraft:prismarine", "weight": 1}]}),
            json!({"processor_type": "betterfortresses:pillar_processor",
                   "target_block": {"Name": "minecraft:orange_terracotta"},
                   "target_block_output": {"defaultBlockState": {"Name": "minecraft:red_nether_bricks"}, "entries": []},
                   "pillar_states": {"defaultBlockState": {"Name": "minecraft:nether_bricks"}, "entries": []}}),
            json!({"processor_type": "betterfortresses:stair_pillar_processor"}),
        ]);
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &d, &mut carried));
        let p = list["processors"].as_array().unwrap();
        let types: Vec<&str> = p
            .iter()
            .map(|p| p["processor_type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            [
                "horizon:pillar",
                "minecraft:block_rot",
                "horizon:bottom_pillar",
                "horizon:close_off_fluid_sources",
                "horizon:close_off_air_sources",
                // YUNG's processors stay Java (dropped later) until the
                // specification covers them.
                "betterfortresses:pillar_processor",
                "betterfortresses:stair_pillar_processor"
            ]
        );
        // The named list is written in once; its own pillar loses its list.
        let inner = &p[0]["pillar_processor_list"]["processors"];
        assert_eq!(inner[0]["processor_type"], "horizon:pillar");
        assert!(inner[0].get("pillar_processor_list").is_none());
        assert_eq!(
            inner[0]["pillar_trigger_and_replacements"],
            p[0]["pillar_trigger_and_replacements"]
        );
        assert_eq!(inner[1]["processor_type"], "minecraft:block_rot");
        assert!(p[2].get("pillar_processor_list").is_none());
        assert_eq!(
            p[4]["weighted_list_of_replacement_blocks"],
            json!([{"block": "minecraft:prismarine", "weight": 1}])
        );
        assert_eq!(carried["repurposed_structures:pillar_processor"], 2);
        assert!(!carried.contains_key("betterfortresses:stair_pillar_processor"));
    }
}
