//! Structure mods' worldgen, carried over: block and fluid states in 26.3's
//! shape, the structure, placement and pool element types of the common
//! structure libraries rewritten as vanilla's, and the loader's convention
//! biome tags (`c:`) the structures name.
//!
//! The libraries' types add rules on top of vanilla's. The start and
//! placement rules of Repurposed Structures' and Moog's jigsaws (no start in
//! liquid, biomes and terrain height in a radius, a minimum distance from
//! the world origin, sets to keep away from) are written under `horizon:`
//! keys the game reads (`horizon_world::worldgen::structure::start_checks`),
//! so those structures start where the mod would. Rules that change how the
//! pieces are laid out (piece caps, burying, clipping) are lost and counted
//! by name. Each rewrite is counted in the carry report, and every type left
//! as it is is listed, so the coverage report can say what generates and how
//! closely.
//!
//! YUNG's API's files are carried as written ([`crate::yung`]).
//! Lithostitched's jigsaws, elements, modifiers and processors are carried by
//! [`crate::lithostitched`]; the entries CTOV adds to vanilla's structure sets
//! by `legacy_structure_mods`, called from [`crate::carry`].
use crate::{lithostitched, yung};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

/// Structure types that are vanilla's jigsaw with extra start rules.
const JIGSAWS: [&str; 2] = [
    "moogs_structures:moogs_structures_generic_jigsaw_structure",
    "repurposed_structures:generic_jigsaw_structure",
];
/// The Nether variants of the piece-limited jigsaws: laid out the same,
/// then moved onto land.
const NETHER_JIGSAWS: [&str; 2] = [
    "moogs_structures:moogs_structures_generic_nether_jigsaw_structure",
    "repurposed_structures:generic_nether_jigsaw_structure",
];
/// Repurposed Structures' generic jigsaws with a start check of their own: the
/// `repurposed` key of `horizon:start_checks` names it.
const REPURPOSED_CHECKED: [(&str, &str); 3] = [
    ("repurposed_structures:city_nether_structure", "city_nether"),
    (
        "repurposed_structures:shipwreck_nether_structure",
        "shipwreck_nether",
    ),
    ("repurposed_structures:stronghold_end", "stronghold_end"),
];
/// Repurposed Structures' mineshaft: its generic jigsaw laid out under the
/// terrain (`"layout": "mineshaft"` in `horizon:piece_limited`).
const REPURPOSED_MINESHAFT: &str = "repurposed_structures:generic_mineshaft";
/// Layout rules the piece-limited assembler keeps (`horizon:piece_limited`).
const PIECE_LIMITED_RULES: [&str; 4] = [
    "y_allowance",
    "burying_type",
    "use_bounding_box_hack",
    "pools_that_ignore_boundaries",
];
/// Placements that are vanilla's random spread with extra rules.
const SPREADS: [&str; 2] = [
    "moogs_structures:advanced_random_spread",
    "repurposed_structures:advanced_random_spread",
];
/// Mo' Structures' placement (`mostructures:type`) takes its spacing and
/// separation from the mod's config, not its files: the defaults it writes
/// (MoStructuresConfig), by config key, as (spacing, separation).
const MO_SPREADS: [(&str, i64, i64); 17] = [
    ("abandoned_church", 33, 20),
    ("air_balloon", 22, 12),
    ("barn_house", 32, 16),
    ("big_pyramid", 32, 25),
    ("ice_tower", 28, 20),
    ("jungle_pyramid", 25, 22),
    ("killer_bunny_castle", 32, 22),
    ("lighthouse", 24, 14),
    ("moai", 17, 15),
    ("pillager_factory", 34, 16),
    ("pillager_mines", 40, 27),
    ("pirate_ship", 32, 16),
    ("tavern", 26, 16),
    ("the_castle_in_the_sky", 32, 22),
    ("villager_market", 32, 16),
    ("villager_tower", 26, 16),
    ("sphinx", 24, 16),
];
/// Mo' Structures keeps a start out of this many chunks around a start of
/// each set in its `structure_set_to_avoid`.
const MO_AVOID_CHUNKS: i64 = 3;
/// Moog's Structure Lib spreads its structures 1.65 times as far apart as
/// its files say (AdvancedRandomSpread's constructor).
const MOOGS_SPACING: f64 = 1.65;
/// The jigsaws' start rules and the `horizon:start_checks` key each maps to.
const START_RULES: [(&str, &str); 3] = [
    ("valid_biome_radius_check", "biome_radius"),
    ("terrain_height_radius_check", "terrain_radius"),
    ("allowed_terrain_height_range", "terrain_range"),
];
/// The jigsaws' rules on laying out pieces, which vanilla's jigsaw has no
/// place for.
const LAYOUT_RULES: [&str; 4] = [
    "y_allowance",
    "burying_type",
    "use_bounding_box_hack",
    "pools_that_ignore_boundaries",
];
/// The libraries' terrain kernel field, the game's
/// `horizon:enhanced_terrain_adaptation` (docs/mods.md).
pub(crate) const ENHANCED_TERRAIN: &str = "enhanced_terrain_adaptation";
pub(crate) const HORIZON_ENHANCED_TERRAIN: &str = "horizon:enhanced_terrain_adaptation";
/// Fields of older versions the mods' codecs no longer read: their own
/// structures ignore them, and so does carry. Repurposed's jungle fortress
/// and End stronghold still write `vertical_distance_from_start_piece`.
const DEAD_FIELDS: [&str; 1] = ["vertical_distance_from_start_piece"];
/// Pool elements that are vanilla's with extra fields: (mod type, vanilla type).
const ELEMENTS: [(&str, &str); 3] = [
    (
        "moogs_structures:versioned_single_pool_element",
        "minecraft:single_pool_element",
    ),
    (
        "repurposed_structures:legacy_ocean_bottom_single_pool_element",
        "minecraft:legacy_single_pool_element",
    ),
    ("empty_pool_element", "minecraft:empty_pool_element"),
];

/// What carry-over did to a mod's worldgen files.
#[derive(Debug, Default, serde::Serialize)]
pub struct Upgrade {
    /// Files rewritten to vanilla types.
    pub mapped: usize,
    /// Files whose block and fluid states were written in 26.3's shape.
    pub block_states: usize,
    /// Types left as they are (Horizon skips what it cannot run), by type.
    pub unsupported: BTreeMap<String, usize>,
    /// Rules of mapped types the game does not run, by field.
    pub rules_lost: BTreeMap<String, usize>,
    /// Processors written in Java, taken out of their lists, by type.
    pub processors_dropped: BTreeMap<String, usize>,
    /// Processors written in Java carried as data the game runs
    /// ([`crate::processors`]), by type.
    pub processors_carried: BTreeMap<String, usize>,
    /// Rule tests written in Java carried as the game's own
    /// ([`crate::processors::rule_tests`]), by type.
    pub rule_tests_carried: BTreeMap<String, usize>,
    /// Entries added to other structure sets and template pools
    /// (`horizon:add_to`), by the set or pool they join.
    pub added_to: BTreeMap<String, usize>,
    /// Lithostitched worldgen modifiers not carried: by type, `conditions`
    /// when they need a mod that is not ported along, `target` when they
    /// name their sets or pools by a tag.
    pub modifiers_left: BTreeMap<String, usize>,
}

/// Where a mod's additions to other sets and pools go in its carry:
/// `data/<id>/worldgen/<kind>/horizon_added/<name>.json`.
pub const ADDED: &str = "horizon_added";

/// Rewrites one `data/<ns>/worldgen/<kind>/...` file in place; whether it
/// changed.
/// `set_tags` are the mod's structure set tags, expanded
/// ([`structure_set_tags`]).
pub fn upgrade(
    kind: &str,
    v: &mut Value,
    report: &mut Upgrade,
    set_tags: &BTreeMap<String, Vec<String>>,
) -> bool {
    let states = block_states(v);
    report.block_states += usize::from(states);
    let renamed = crate::carry::renamed_ids(v);
    let dropped = crate::processors::each_list(kind, v, &mut |list| {
        drop_java_processors(list, &mut report.processors_dropped)
    });
    upgrade_types(kind, v, report, set_tags) || states || dropped || renamed
}

/// Takes the processors written in Java (any type outside `minecraft:` and
/// the game's own `horizon:`) out of a processor list: `{"processors":
/// [...]}`, a bare list, or an id (left as it is). The ones with a data form
/// were rewritten before ([`crate::processors::carry`]). The game, like vanilla, refuses a list holding a type it
/// does not know, and with it every pool that names the list, so without
/// this the structure does not generate at all. Without them it generates
/// undecorated: no pillars under it, spawners left as the template has
/// them, blocks not swapped. Whether anything was taken out.
fn drop_java_processors(list: &mut Value, dropped: &mut BTreeMap<String, usize>) -> bool {
    let Some(processors) = crate::processors::processors(list) else {
        return false;
    };
    let before = processors.len();
    processors.retain(|p| {
        let t = p["processor_type"].as_str().unwrap_or_default();
        let java = !t.starts_with("minecraft:")
            && !t.starts_with("horizon:")
            && !lithostitched::game_reads(p);
        if java {
            *dropped.entry(t.to_owned()).or_default() += 1;
        }
        !java
    });
    let mut changed = processors.len() != before;
    // The inline lists inside Lithostitched's apply_random and condition.
    for p in processors.iter_mut() {
        let inner: Vec<&mut Value> = match p["processor_type"].as_str() {
            Some(t) if t.starts_with("lithostitched:") => lithostitched::inner_lists(p),
            Some("horizon:pillar" | "horizon:bottom_pillar") => {
                p.get_mut("pillar_processor_list").into_iter().collect()
            }
            _ => Vec::new(),
        };
        for list in inner {
            changed |= drop_java_processors(list, dropped);
        }
    }
    changed
}

/// 1.21.1 writes a block or fluid state as `{"Name": ..., "Properties":
/// {...}}`; 26.3's codec reads `{"id": ..., "properties": {...}}` and
/// refuses the old shape, which fails the processor list, feature or pool
/// holding it. Whether anything changed.
fn block_states(v: &mut Value) -> bool {
    match v {
        Value::Object(o) => {
            let state = o.get("Name").is_some_and(Value::is_string)
                && o.keys().all(|k| k == "Name" || k == "Properties");
            if state {
                if let Some(name) = o.remove("Name") {
                    o.insert("id".into(), name);
                }
                if let Some(properties) = o.remove("Properties") {
                    o.insert("properties".into(), properties);
                }
                return true;
            }
            o.values_mut()
                .fold(false, |changed, x| block_states(x) | changed)
        }
        Value::Array(a) => a
            .iter_mut()
            .fold(false, |changed, x| block_states(x) | changed),
        _ => false,
    }
}

/// The library types of one file rewritten as vanilla's; whether it changed.
fn upgrade_types(
    kind: &str,
    v: &mut Value,
    report: &mut Upgrade,
    set_tags: &BTreeMap<String, Vec<String>>,
) -> bool {
    let lost = &mut report.rules_lost;
    // YUNG's and Lithostitched's types are converted apart from this code.
    if let Some(changed) =
        yung::upgrade(kind, v, lost).or_else(|| lithostitched::upgrade(kind, v, lost))
    {
        report.mapped += usize::from(changed);
        return changed;
    }
    let mut unsupported = |t: &str| *report.unsupported.entry(t.to_owned()).or_default() += 1;
    let changed = match kind {
        "structure" => match v["type"].as_str() {
            Some(t)
                if JIGSAWS.contains(&t)
                    || NETHER_JIGSAWS.contains(&t)
                    || t == REPURPOSED_MINESHAFT
                    || REPURPOSED_CHECKED.iter().any(|(r, _)| *r == t) =>
            {
                let mineshaft = t == REPURPOSED_MINESHAFT;
                let check = REPURPOSED_CHECKED.iter().find(|(r, _)| *r == t);
                let check = check.map(|(_, c)| *c);
                let library = match t.split_once(':').map(|(ns, _)| ns) {
                    Some("moogs_structures") => Some("moogs"),
                    Some("repurposed_structures") => Some("repurposed"),
                    _ => None,
                };
                let liquid = if library == Some("moogs") {
                    "top"
                } else {
                    "lowest"
                };
                let mut limited = library.map(|l| piece_limited(v, l));
                if let Some(limited) = limited.as_mut().filter(|_| mineshaft) {
                    limited["layout"] = "mineshaft".into();
                }
                start_checks(v, liquid, lost, limited.is_some());
                if library == Some("moogs") {
                    // GenericJigsawStructure's adaptation, NONE by default;
                    // its pieces may still have their own.
                    let own = v.as_object_mut().and_then(|o| o.remove(ENHANCED_TERRAIN));
                    let none = serde_json::json!({"kernel_size": 0, "kernel_distance": 0,
                        "top": "none", "bottom": "none"});
                    let mut kernel = own.and_then(|a| moogs_terrain(&a, lost)).unwrap_or(none);
                    kernel["rigid_junctions"] = false.into();
                    v[HORIZON_ENHANCED_TERRAIN] = kernel;
                } else if v
                    .as_object_mut()
                    .and_then(|o| o.remove(ENHANCED_TERRAIN))
                    .is_some()
                {
                    *lost.entry(ENHANCED_TERRAIN.into()).or_default() += 1;
                }
                if let Some(check) = check {
                    if !v["horizon:start_checks"].is_object() {
                        v["horizon:start_checks"] = Value::Object(Default::default());
                    }
                    v["horizon:start_checks"]["repurposed"] = check.into();
                }
                if let Some(limited) = limited {
                    v["horizon:piece_limited"] = limited;
                }
                v["type"] = "minecraft:jigsaw".into();
                true
            }
            // Mo' Structures' ModStructure: vanilla's jigsaw that starts only
            // where the terrain under the 5×5 chunks around varies by no
            // more than `heightRange` blocks (-1: anywhere).
            Some("mostructures:generic") => {
                let range = v
                    .as_object_mut()
                    .and_then(|o| o.remove("heightRange"))
                    .and_then(|r| r.as_i64())
                    .unwrap_or(-1);
                if range >= 0 {
                    v["horizon:start_checks"] =
                        serde_json::json!({"terrain_radius": 2, "terrain_range": range});
                }
                v["type"] = "minecraft:jigsaw".into();
                true
            }
            // Formations' SimpleStructure: its pool and placement, laid out
            // by the game (`horizon_world::worldgen::structure::formations`).
            Some("formations:simple") => {
                v["type"] = "horizon:formations".into();
                true
            }
            // Its PiecedStructure: the same start, then vanilla's jigsaw
            // (`horizon:pieced`).
            Some("formations:pieced") => {
                let Some(o) = v.as_object_mut() else {
                    return false;
                };
                let pool = o.remove("start_pool").unwrap_or_default();
                let mut pieced = serde_json::Map::new();
                for k in ["max_depth", "max_distance_from_center"] {
                    pieced.insert(k.into(), o.remove(k).unwrap_or_default());
                }
                o.insert("template".into(), pool);
                o.insert("horizon:pieced".into(), pieced.into());
                o.insert("type".into(), "horizon:formations".into());
                true
            }
            Some(t) if !t.starts_with("minecraft:") => {
                unsupported(t);
                false
            }
            _ => false,
        },
        "structure_set" => match v["placement"]["type"].as_str() {
            Some(t) if SPREADS.contains(&t) => {
                let moogs = t.starts_with("moogs_structures:");
                let p = &mut v["placement"];
                if moogs {
                    moogs_spacing(p, lost);
                }
                placement_rules(p, set_tags, lost);
                p["type"] = "minecraft:random_spread".into();
                true
            }
            Some("mostructures:type") => mo_placement(&mut v["placement"], lost),
            Some(t) if !t.starts_with("minecraft:") => {
                unsupported(t);
                false
            }
            _ => false,
        },
        "template_pool" => {
            let mut changed = false;
            let mut elements: Vec<&mut Value> = Vec::new();
            for e in v["elements"].as_array_mut().into_iter().flatten() {
                let element = &mut e["element"];
                // A list element holds elements of its own.
                if element.get("elements").is_some_and(Value::is_array) {
                    elements.extend(element["elements"].as_array_mut().into_iter().flatten());
                } else {
                    elements.push(element);
                }
            }
            for element in elements {
                if yung::upgrade_pool_element(element, lost)
                    || lithostitched::upgrade_pool_element(element, lost)
                {
                    changed = true;
                    continue;
                }
                let Some(t) = element["element_type"].as_str() else {
                    continue;
                };
                if let Some((_, to)) = ELEMENTS.iter().find(|(from, _)| *from == t) {
                    if t.starts_with("moogs_structures:") {
                        let own = element
                            .as_object_mut()
                            .and_then(|o| o.remove(ENHANCED_TERRAIN));
                        if let Some(kernel) = own.and_then(|a| moogs_terrain(&a, lost)) {
                            element[HORIZON_ENHANCED_TERRAIN] = kernel;
                        }
                    }
                    element["element_type"] = (*to).into();
                    changed = true;
                } else if t == "formations:single_pool_element" {
                    // Vanilla's element with its own ground level (1 when
                    // missing, as its codec reads it).
                    element["element_type"] = "minecraft:single_pool_element".into();
                    if let Some(o) = element.as_object_mut() {
                        let ground = o.remove("ground_level").unwrap_or(1.into());
                        o.insert("horizon:ground_level".into(), ground);
                    }
                    changed = true;
                } else if !t.starts_with("minecraft:") {
                    unsupported(t);
                }
            }
            changed
        }
        _ => false,
    };
    report.mapped += usize::from(changed);
    changed
}

/// MoogsStructureLib's terrain adaptation: already the game's shape (with
/// its band), when it has the fields the game needs (else it is lost, as
/// the library's codec would refuse it).
pub(crate) fn moogs_terrain(a: &Value, lost: &mut BTreeMap<String, usize>) -> Option<Value> {
    let ok = ["kernel_size", "kernel_distance"]
        .iter()
        .all(|k| a[k].is_u64())
        && ["top", "bottom"].iter().all(|k| a[k].is_string());
    if !ok {
        *lost.entry(ENHANCED_TERRAIN.into()).or_default() += 1;
    }
    ok.then(|| a.clone())
}

/// Moves a Repurposed Structures or Moog's jigsaw's start rules under
/// `horizon:start_checks`, and counts the layout rules it loses. `liquid` is
/// the block its no-start-in-liquid rule reads.
pub(crate) fn start_checks(
    v: &mut Value,
    liquid: &str,
    lost: &mut BTreeMap<String, usize>,
    limited: bool,
) {
    let Some(o) = v.as_object_mut() else {
        return;
    };
    let mut checks = serde_json::Map::new();
    for (from, to) in START_RULES {
        if let Some(n) = o.remove(from) {
            checks.insert(to.into(), n);
        }
    }
    if o.remove("cannot_spawn_in_liquid").and_then(|b| b.as_bool()) == Some(true) {
        checks.insert("liquid".into(), liquid.into());
    }
    // y_allowance bounds the terrain heights checked; it also clips pieces,
    // which is lost.
    for (from, to) in [("min_y_allowed", "min_y"), ("max_y_allowed", "max_y")] {
        if let Some(n) = o.get("y_allowance").and_then(|y| y.get(from)) {
            checks.insert(to.into(), n.clone());
        }
    }
    for field in DEAD_FIELDS {
        o.remove(field);
    }
    for rule in LAYOUT_RULES {
        let kept = limited && PIECE_LIMITED_RULES.contains(&rule);
        if o.remove(rule).is_some_and(|r| r != Value::Bool(false)) && !kept {
            *lost.entry(rule.into()).or_default() += 1;
        }
    }
    if !checks.is_empty() {
        o.insert("horizon:start_checks".into(), checks.into());
    }
}

/// A Repurposed Structures or Moog's jigsaw's layout rules for the game's
/// piece-limited assembler: y_allowance, burying, the pools whose fallbacks
/// ignore the bounds, liquid, and the Nether variants' land search. The
/// bounding box hack is vanilla's expansion hack. The pieces' counts are
/// added from the mod's count files ([`piece_counts`]).
fn piece_limited(v: &mut Value, library: &str) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("library".into(), library.into());
    let lower = |x: &Value| x.as_str().map(str::to_ascii_lowercase);
    for (from, to) in [("min_y_allowed", "min_y"), ("max_y_allowed", "max_y")] {
        if let Some(n) = v.get("y_allowance").and_then(|y| y.get(from)) {
            out.insert(to.into(), n.clone());
        }
    }
    if let Some(b) = v.get("burying_type").and_then(lower) {
        out.insert("burying".into(), b.into());
    }
    if v["cannot_spawn_in_liquid"].as_bool() == Some(true) {
        out.insert("no_liquid".into(), true.into());
    }
    if let Some(pools) = v
        .get("pools_that_ignore_boundaries")
        .filter(|p| p.is_array())
    {
        out.insert("ignore_bounds_pools".into(), pools.clone());
    }
    if v["use_bounding_box_hack"].as_bool() == Some(true) {
        v["use_expansion_hack"] = true.into();
    }
    if let Some(o) = v.as_object_mut() {
        if let Some(search) = o.remove("land_search_direction") {
            out.insert("land_search".into(), lower(&search).into());
        }
        if let Some(ledge) = o.remove("ledge_offset_y") {
            out.insert("ledge_offset_y".into(), ledge);
        }
    }
    out.into()
}

/// The pieces' spawn counts of the structures a mod ships, by structure id:
/// Repurposed Structures' `rs_pieces_spawn_counts` (each file names its
/// `target_structure`) and Moog's `msl_pieces_spawn_counts` (the file's id),
/// in the game's `horizon:piece_limited` shape.
pub fn piece_counts<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
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
        let (rs, path) = if let Some(p) = rest.strip_prefix("rs_pieces_spawn_counts/") {
            (true, p)
        } else if let Some(p) = rest.strip_prefix("msl_pieces_spawn_counts/") {
            (false, p)
        } else {
            continue;
        };
        let mut text = String::new();
        let Ok(v) = entry
            .read_to_string(&mut text)
            .map_err(|_| ())
            .and_then(|_| serde_json::from_str::<Value>(&text).map_err(|_| ()))
        else {
            continue;
        };
        let id = match v["target_structure"].as_str() {
            Some(target) if rs => target.to_owned(),
            _ if rs => continue,
            _ => format!("{ns}:{path}"),
        };
        let pieces: Vec<Value> = v["pieces_spawn_counts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let mut e = serde_json::Map::new();
                e.insert("name".into(), p["nbt_piece_name"].as_str()?.into());
                for (from, to) in [
                    ("always_spawn_this_many", "required"),
                    ("never_spawn_more_than_this_many", "max"),
                    ("minimum_distance_from_center_piece", "min_depth"),
                ] {
                    if let Some(n) = p.get(from).filter(|n| n.is_number()) {
                        e.insert(to.into(), n.clone());
                    }
                }
                Some(Value::Object(e))
            })
            .collect();
        out.insert(id, pieces.into());
    }
    out
}

/// Adds a structure's piece counts to its `horizon:piece_limited` key.
pub fn add_piece_counts(id: &str, v: &mut Value, counts: &BTreeMap<String, Value>) -> bool {
    match (counts.get(id), v.get_mut("horizon:piece_limited")) {
        (Some(pieces), Some(limited)) => {
            limited["pieces"] = pieces.clone();
            true
        }
        _ => false,
    }
}

/// AdvancedRandomSpread's spacing and separation, as Moog's computes them
/// with its default config: 1.65 times the file's, rounded, the separation
/// below the spacing. `when_replacing` (other numbers while a preset
/// replaces a vanilla structure) is lost.
fn moogs_spacing(p: &mut Value, lost: &mut BTreeMap<String, usize>) {
    let scaled = |k: &str| (p[k].as_f64().unwrap_or(0.) * MOOGS_SPACING).round() as i64;
    let spacing = scaled("spacing").max(1);
    let separation = scaled("separation").min(spacing - 1);
    p["spacing"] = spacing.into();
    p["separation"] = separation.into();
    if let Some(o) = p.as_object_mut() {
        if o.remove("when_replacing").is_some() {
            *lost.entry("when_replacing".into()).or_default() += 1;
        }
    }
}

/// An advanced random spread's minimum distance from the world origin and
/// its super exclusion zone (sets named by a tag) under `horizon:` keys.
pub(crate) fn placement_rules(
    p: &mut Value,
    set_tags: &BTreeMap<String, Vec<String>>,
    lost: &mut BTreeMap<String, usize>,
) {
    let Some(o) = p.as_object_mut() else {
        return;
    };
    if let Some(d) = o.remove("min_distance_from_world_origin") {
        o.insert("horizon:min_distance_from_world_origin".into(), d);
    }
    let zone = ["super_exclusion_zone", "enhanced_exclusion_zone"]
        .into_iter()
        .find_map(|k| Some((k, o.remove(k)?)));
    if let Some((key, zone)) = zone {
        let sets: Vec<String> = match &zone["other_set"] {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => set_tags.get(tag).cloned().unwrap_or_default(),
                None => vec![s.clone()],
            },
            Value::Array(a) => a
                .iter()
                .filter_map(|s| Some(s.as_str()?.to_owned()))
                .collect(),
            _ => Vec::new(),
        };
        if sets.is_empty() {
            *lost.entry(key.into()).or_default() += 1;
        } else {
            o.insert(
                "horizon:exclusion_zone".into(),
                serde_json::json!({"other_sets": sets, "chunk_count": zone["chunk_count"]}),
            );
        }
    }
}

/// Mo' Structures' placement as vanilla's random spread: the spacing and
/// separation of its config key, and its sets to avoid as
/// `horizon:exclusion_zone` tested by their spread alone
/// (ModStructurePlacement.exclusionZoneCheck). A key the config does not
/// have fails the mod's codec, and the set stays as it is.
fn mo_placement(p: &mut Value, lost: &mut BTreeMap<String, usize>) -> bool {
    let key = p["config_key"].as_str().unwrap_or_default();
    let Some(&(_, spacing, separation)) = MO_SPREADS.iter().find(|(k, ..)| *k == key) else {
        *lost.entry("config_key".into()).or_default() += 1;
        return false;
    };
    let Some(o) = p.as_object_mut() else {
        return false;
    };
    o.remove("config_key");
    o.insert("spacing".into(), spacing.into());
    o.insert("separation".into(), separation.into());
    let avoid = o.remove("structure_set_to_avoid");
    let sets: Vec<Value> = avoid
        .iter()
        .flat_map(|a| a.as_array().into_iter().flatten())
        .filter(|s| s.is_string())
        .cloned()
        .collect();
    if !sets.is_empty() {
        o.insert(
            "horizon:exclusion_zone".into(),
            serde_json::json!({"other_sets": sets, "chunk_count": MO_AVOID_CHUNKS,
                "their_zones": false}),
        );
    }
    o.insert("type".into(), "minecraft:random_spread".into());
    true
}

/// The mod's structure set tags (`data/<ns>/tags/worldgen/structure_set/`),
/// by tag id, with the tags they include expanded. Optional entries
/// (`{"id": ..., "required": false}`) count as their id.
pub fn structure_set_tags<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> BTreeMap<String, Vec<String>> {
    let mut raw: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let Some(id) = entry
            .name()
            .ok()
            .as_deref()
            .and_then(|n| n.strip_prefix("data/"))
            .and_then(|r| r.split_once("/tags/worldgen/structure_set/"))
            .and_then(|(ns, path)| Some(format!("{ns}:{}", path.strip_suffix(".json")?)))
        else {
            continue;
        };
        let mut text = String::new();
        let Ok(v) = entry.read_to_string(&mut text).map_err(drop).and_then(|_| {
            serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).map_err(drop)
        }) else {
            continue;
        };
        let values = v["values"].as_array().into_iter().flatten();
        raw.insert(
            id,
            values
                .filter_map(|e| e.as_str().or_else(|| e["id"].as_str()).map(str::to_owned))
                .collect(),
        );
    }
    let expand = |tag: &str| {
        let (mut out, mut seen, mut todo) = (Vec::new(), BTreeSet::new(), vec![tag.to_owned()]);
        while let Some(t) = todo.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            for e in raw.get(&t).into_iter().flatten() {
                match e.strip_prefix('#') {
                    Some(inner) => todo.push(inner.to_owned()),
                    None if !out.contains(e) => out.push(e.clone()),
                    None => {}
                }
            }
        }
        out
    };
    raw.keys().map(|t| (t.clone(), expand(t))).collect()
}

/// The `#c:` biome tags `v` names.
pub fn convention_tags(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => {
            if let Some(tag) = s.strip_prefix("#c:") {
                out.insert(tag.to_owned());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| convention_tags(x, out)),
        Value::Object(o) => o.values().for_each(|x| convention_tags(x, out)),
        _ => {}
    }
}

/// The loader's own convention biome tags (`data/c/tags/worldgen/biome/`),
/// by path, read from the loader jar `fetch` keeps in `<work>/loader/`.
pub fn loader_biome_tags(work: &Path) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Ok(dir) = std::fs::read_dir(work.join("loader")) else {
        return out;
    };
    for jar in dir.flatten().map(|e| e.path()) {
        let Some(mut zip) = std::fs::File::open(&jar)
            .ok()
            .and_then(|f| zip::ZipArchive::new(f).ok())
        else {
            continue;
        };
        for i in 0..zip.len() {
            let Ok(mut entry) = zip.by_index(i) else {
                continue;
            };
            let Some(path) = entry
                .name()
                .ok()
                .as_deref()
                .and_then(|n| n.strip_prefix("data/c/tags/worldgen/biome/"))
                .and_then(|p| p.strip_suffix(".json"))
                .map(str::to_owned)
            else {
                continue;
            };
            let mut text = String::new();
            if entry.read_to_string(&mut text).is_ok() {
                if let Ok(v) = serde_json::from_str(&text) {
                    out.insert(path, v);
                }
            }
        }
    }
    out
}

/// `tags` and the convention tags they include, in `conventions`.
pub fn with_included(
    tags: &BTreeSet<String>,
    conventions: &BTreeMap<String, Value>,
) -> BTreeSet<String> {
    let mut all = BTreeSet::new();
    let mut todo: Vec<String> = tags.iter().cloned().collect();
    while let Some(tag) = todo.pop() {
        if !conventions.contains_key(&tag) || !all.insert(tag.clone()) {
            continue;
        }
        let mut more = BTreeSet::new();
        convention_tags(&conventions[&tag], &mut more);
        todo.extend(more);
    }
    all
}

/// Where `fetch` keeps the pack's loader jar, and where it comes from:
/// NeoForge's own Maven repository. Only NeoForge packs need it today.
pub fn loader_jar(work: &Path, loader: &str, version: &str) -> Option<(PathBuf, String)> {
    (loader == "neoforge" && !version.is_empty()).then(|| {
        (
            work.join("loader").join(format!("neoforge-{version}-universal.jar")),
            format!(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-universal.jar"
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NO_TAGS: BTreeMap<String, Vec<String>> = BTreeMap::new();

    #[test]
    fn formations_pieced_structure_keeps_its_jigsaw() {
        let mut v = json!({"type": "formations:pieced", "start_pool": "fo:fountain/fountains",
            "max_depth": 2, "max_distance_from_center": 116, "placement": "surface"});
        let mut report = Upgrade::default();
        assert!(upgrade("structure", &mut v, &mut report, &BTreeMap::new()));
        assert_eq!(
            v,
            json!({"type": "horizon:formations", "template": "fo:fountain/fountains",
                "placement": "surface",
                "horizon:pieced": {"max_depth": 2, "max_distance_from_center": 116}})
        );
        assert!(report.unsupported.is_empty());
    }

    #[test]
    fn formations_become_the_games() {
        let mut report = Upgrade::default();
        let mut s = json!({"type": "formations:simple", "template": "f:campsites",
            "placement": "surface", "biomes": "#f:x", "step": "surface_structures",
            "terrain_adaptation": "beard_thin", "spawn_overrides": {}});
        assert!(upgrade("structure", &mut s, &mut report, &NO_TAGS));
        assert_eq!(s["type"], "horizon:formations");
        assert_eq!(s["placement"], "surface");
        let mut pool = json!({"elements": [
            {"weight": 1, "element": {"element_type": "formations:single_pool_element",
                "location": "f:a", "projection": "rigid", "ground_level": 6, "processors": []}},
            {"weight": 1, "element": {"element_type": "formations:single_pool_element",
                "location": "f:b", "projection": "rigid", "processors": []}}]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        let e = |i: usize| &pool["elements"][i]["element"];
        assert_eq!(e(0)["element_type"], "minecraft:single_pool_element");
        assert_eq!(
            (
                e(0)["horizon:ground_level"].clone(),
                e(1)["horizon:ground_level"].clone()
            ),
            (json!(6), json!(1))
        );
        assert!(e(0).get("ground_level").is_none());
        assert!(report.unsupported.is_empty());
    }

    #[test]
    fn maps_library_types_to_vanillas() {
        let mut report = Upgrade::default();
        let mut s = json!({"type": "lithostitched:jigsaw", "start_pool": "a:b"});
        assert!(upgrade("structure", &mut s, &mut report, &NO_TAGS));
        assert_eq!(s["type"], "minecraft:jigsaw");
        let mut other = json!({"type": "somelib:structure"});
        assert!(!upgrade("structure", &mut other, &mut report, &NO_TAGS));
        let mut set = json!({"placement": {"type": "moogs_structures:advanced_random_spread",
            "spacing": 10, "separation": 5, "min_distance_from_world_origin": 250}});
        assert!(upgrade("structure_set", &mut set, &mut report, &NO_TAGS));
        assert_eq!(set["placement"]["type"], "minecraft:random_spread");
        // Moog's spreads 1.65 times as far as its file says.
        assert_eq!(set["placement"]["spacing"], 17);
        assert_eq!(set["placement"]["separation"], 8);
        assert_eq!(
            set["placement"]["horizon:min_distance_from_world_origin"],
            250
        );
        assert!(
            set["placement"]
                .get("min_distance_from_world_origin")
                .is_none()
        );
        let mut pool = json!({"elements": [
            {"weight": 1, "element": {"element_type": "moogs_structures:versioned_single_pool_element", "location": "m:x",
                "enhanced_terrain_adaptation": {"kernel_size": 16, "kernel_distance": 8, "top": "carve",
                    "bottom": "carve", "band": {"bottom": 17, "top": 23}}}},
            {"weight": 1, "element": {"element_type": "minecraft:list_pool_element", "elements": [
                {"element_type": "empty_pool_element"}]}},
            {"weight": 1, "element": {"element_type": "somelib:element"}}
        ]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        assert_eq!(
            pool["elements"][0]["element"]["element_type"],
            "minecraft:single_pool_element"
        );
        // Its own terrain kernel, band and all, under the game's key.
        assert_eq!(
            pool["elements"][0]["element"]["horizon:enhanced_terrain_adaptation"]["band"],
            json!({"bottom": 17, "top": 23})
        );
        assert_eq!(
            pool["elements"][1]["element"]["elements"][0]["element_type"],
            "minecraft:empty_pool_element"
        );
        assert_eq!(report.mapped, 3);
        assert_eq!(report.block_states, 0);
        assert_eq!(
            report.unsupported,
            BTreeMap::from([
                ("somelib:element".into(), 1),
                ("somelib:structure".into(), 1),
            ])
        );
    }

    #[test]
    fn keeps_the_jigsaws_start_rules() {
        let mut report = Upgrade::default();
        let mut moogs = json!({"type": "moogs_structures:moogs_structures_generic_jigsaw_structure",
            "cannot_spawn_in_liquid": true, "terrain_height_radius_check": 2,
            "allowed_terrain_height_range": 12, "valid_biome_radius_check": 1,
            "y_allowance": {"min_y_allowed": 60}, "burying_type": "lowest_corner",
            "use_bounding_box_hack": false, "size": 7});
        assert!(upgrade("structure", &mut moogs, &mut report, &NO_TAGS));
        assert_eq!(
            moogs["horizon:start_checks"],
            json!({"liquid": "top", "terrain_radius": 2, "terrain_range": 12,
                "biome_radius": 1, "min_y": 60})
        );
        for gone in [
            "cannot_spawn_in_liquid",
            "y_allowance",
            "burying_type",
            "use_bounding_box_hack",
        ] {
            assert!(moogs.get(gone).is_none(), "{gone}");
        }
        assert_eq!(moogs["size"], 7);
        // GenericJigsawStructure's terrain kernel, NONE when it has none.
        assert_eq!(
            moogs["horizon:enhanced_terrain_adaptation"],
            json!({"kernel_size": 0, "kernel_distance": 0, "top": "none", "bottom": "none",
                "rigid_junctions": false})
        );
        let mut fortress = json!({"type": "moogs_structures:moogs_structures_generic_nether_jigsaw_structure",
            "size": 7, "enhanced_terrain_adaptation": {"kernel_size": 16, "kernel_distance": 8,
                "top": "carve", "bottom": "carve", "band": {"bottom": 17, "top": 23,
                "piece_heights": [24, 25, 27]}}});
        assert!(upgrade("structure", &mut fortress, &mut report, &NO_TAGS));
        assert_eq!(
            fortress["horizon:enhanced_terrain_adaptation"]["band"]["piece_heights"],
            json!([24, 25, 27])
        );
        assert!(fortress.get("enhanced_terrain_adaptation").is_none());
        // Repurposed Structures reads the block above the bottom. Its jungle
        // fortress's vertical distance is a field its code no longer reads.
        let mut rs = json!({"type": "repurposed_structures:generic_jigsaw_structure",
            "cannot_spawn_in_liquid": true, "vertical_distance_from_start_piece": 35});
        assert!(upgrade("structure", &mut rs, &mut report, &NO_TAGS));
        assert_eq!(rs["horizon:start_checks"], json!({"liquid": "lowest"}));
        assert!(rs.get("vertical_distance_from_start_piece").is_none());
        // Repurposed's own checks: the Nether city, Nether shipwrecks, the End stronghold.
        for (t, check) in [
            ("city_nether_structure", "city_nether"),
            ("shipwreck_nether_structure", "shipwreck_nether"),
            ("stronghold_end", "stronghold_end"),
        ] {
            let mut rs = json!({"type": format!("repurposed_structures:{t}"),
                "cannot_spawn_in_liquid": false, "size": 7});
            assert!(upgrade("structure", &mut rs, &mut report, &NO_TAGS));
            assert_eq!(rs["type"], "minecraft:jigsaw");
            assert_eq!(rs["horizon:start_checks"], json!({"repurposed": check}));
            assert_eq!(rs["horizon:piece_limited"]["library"], "repurposed");
        }
        assert!(report.unsupported.is_empty());
        // Nothing to check: no key.
        let mut plain = json!({"type": "lithostitched:jigsaw", "cannot_spawn_in_liquid": false});
        assert!(upgrade("structure", &mut plain, &mut report, &NO_TAGS));
        assert!(plain.get("horizon:start_checks").is_none());
        // The layout rules go to the piece-limited assembler.
        assert!(report.rules_lost.is_empty());
        assert_eq!(
            moogs["horizon:piece_limited"],
            json!({"library": "moogs", "min_y": 60, "burying": "lowest_corner",
                "no_liquid": true})
        );
    }

    #[test]
    fn mo_structures_become_vanillas_jigsaw_and_spread() {
        let mut report = Upgrade::default();
        let mut s = json!({"type": "mostructures:generic", "start_pool": "mostructures:a/base",
            "size": 7, "heightRange": 25, "use_expansion_hack": true,
            "start_height": {"absolute": 0}, "max_distance_from_center": 80});
        assert!(upgrade("structure", &mut s, &mut report, &BTreeMap::new()));
        assert_eq!(s["type"], "minecraft:jigsaw");
        assert!(s.get("heightRange").is_none());
        assert_eq!(
            s["horizon:start_checks"],
            json!({"terrain_radius": 2, "terrain_range": 25})
        );
        let mut anywhere = json!({"type": "mostructures:generic", "heightRange": -1});
        assert!(upgrade(
            "structure",
            &mut anywhere,
            &mut report,
            &BTreeMap::new()
        ));
        assert!(anywhere.get("horizon:start_checks").is_none());

        // The config's spacing and separation, not the file's.
        let mut set = json!({"placement": {"type": "mostructures:type", "config_key": "moai",
            "salt": 12994829, "spacing": 17, "separation": 15,
            "structure_set_to_avoid": ["minecraft:villages", "mostructures:lighthouse"]}});
        assert!(upgrade(
            "structure_set",
            &mut set,
            &mut report,
            &BTreeMap::new()
        ));
        assert_eq!(
            set["placement"],
            json!({"type": "minecraft:random_spread", "salt": 12994829, "spacing": 17,
                "separation": 15, "horizon:exclusion_zone": {"chunk_count": 3,
                "other_sets": ["minecraft:villages", "mostructures:lighthouse"],
                "their_zones": false}})
        );
        let mut set = json!({"placement": {"type": "mostructures:type",
            "config_key": "jungle_pyramid", "salt": 1, "spacing": 16, "separation": 14}});
        assert!(upgrade(
            "structure_set",
            &mut set,
            &mut report,
            &BTreeMap::new()
        ));
        assert_eq!(
            (
                &set["placement"]["spacing"],
                &set["placement"]["separation"]
            ),
            (&json!(25), &json!(22))
        );
        assert!(set["placement"].get("horizon:exclusion_zone").is_none());
        let mut unknown = json!({"placement": {"type": "mostructures:type", "config_key": "x"}});
        assert!(!upgrade(
            "structure_set",
            &mut unknown,
            &mut report,
            &BTreeMap::new()
        ));
        assert_eq!(report.rules_lost["config_key"], 1);
    }

    #[test]
    fn super_exclusion_zones_name_their_sets() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, text) in [
            (
                "data/rs/tags/worldgen/structure_set/avoid.json",
                r##"{"values": ["minecraft:villages", "#rs:more", {"id": "x:y", "required": false}]}"##,
            ),
            (
                "data/rs/tags/worldgen/structure_set/more.json",
                r##"{"values": ["minecraft:villages", "rs:cities", "#rs:avoid"]}"##,
            ),
        ] {
            zip.start_file(name, opts).unwrap();
            std::io::Write::write_all(&mut zip, text.as_bytes()).unwrap();
        }
        let mut zip = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        let tags = structure_set_tags(&mut zip);
        assert_eq!(tags["rs:avoid"], ["minecraft:villages", "x:y", "rs:cities"]);
        let mut report = Upgrade::default();
        let mut set = json!({"placement": {"type": "repurposed_structures:advanced_random_spread",
            "spacing": 30, "separation": 10,
            "super_exclusion_zone": {"chunk_count": 4, "other_set": "#rs:avoid"}}});
        assert!(upgrade("structure_set", &mut set, &mut report, &tags));
        assert_eq!(set["placement"]["spacing"], 30);
        assert_eq!(
            set["placement"]["horizon:exclusion_zone"],
            json!({"other_sets": ["minecraft:villages", "x:y", "rs:cities"], "chunk_count": 4})
        );
        let mut unknown = json!({"placement": {"type": "repurposed_structures:advanced_random_spread",
            "spacing": 30, "separation": 10,
            "super_exclusion_zone": {"chunk_count": 4, "other_set": "#rs:none"}}});
        assert!(upgrade("structure_set", &mut unknown, &mut report, &tags));
        assert_eq!(report.rules_lost["super_exclusion_zone"], 1);
    }

    #[test]
    fn drops_processors_written_in_java() {
        let mut report = Upgrade::default();
        let mut list = json!({"processors": [
            {"processor_type": "repurposed_structures:bottom_pillar_processor"},
            {"processor_type": "minecraft:gravity", "heightmap": "WORLD_SURFACE_WG", "offset": -1},
            {"processor_type": "moogs_structures:spawner_randomizing_processor"}]});
        assert!(upgrade("processor_list", &mut list, &mut report, &NO_TAGS));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "minecraft:gravity",
            "heightmap": "WORLD_SURFACE_WG", "offset": -1}])
        );
        let mut pool = json!({"elements": [
            {"weight": 1, "element": {"element_type": "minecraft:single_pool_element",
                "processors": {"processors": [{"processor_type": "lithostitched:block_swap"}]}}},
            {"weight": 1, "element": {"element_type": "minecraft:list_pool_element", "elements": [
                {"element_type": "minecraft:single_pool_element", "processors": "x:named"}]}}]});
        assert!(upgrade("template_pool", &mut pool, &mut report, &NO_TAGS));
        assert_eq!(
            pool["elements"][0]["element"]["processors"],
            json!({"processors": []})
        );
        assert_eq!(
            pool["elements"][1]["element"]["elements"][0]["processors"],
            "x:named"
        );
        let mut vanilla = json!({"processors": [{"processor_type": "minecraft:nop"}]});
        assert!(!upgrade(
            "processor_list",
            &mut vanilla,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(
            report.processors_dropped,
            BTreeMap::from([
                ("lithostitched:block_swap".into(), 1),
                ("moogs_structures:spawner_randomizing_processor".into(), 1),
                ("repurposed_structures:bottom_pillar_processor".into(), 1),
            ])
        );
    }

    #[test]
    fn piece_limited_jigsaws_keep_their_layout() {
        let mut report = Upgrade::default();
        let mut nether = json!({"type": "repurposed_structures:generic_nether_jigsaw_structure",
            "start_pool": "rs:x", "size": 8, "y_allowance": {"min_y_allowed": 40, "max_y_allowed": 90},
            "cannot_spawn_in_liquid": true, "pools_that_ignore_boundaries": ["rs:roof"],
            "use_bounding_box_hack": true, "land_search_direction": "HIGHEST_LAND",
            "ledge_offset_y": 2});
        assert!(upgrade("structure", &mut nether, &mut report, &NO_TAGS));
        assert_eq!(nether["type"], "minecraft:jigsaw");
        assert_eq!(
            nether["horizon:piece_limited"],
            json!({"library": "repurposed", "min_y": 40, "max_y": 90, "no_liquid": true,
                "ignore_bounds_pools": ["rs:roof"], "land_search": "highest_land",
                "ledge_offset_y": 2})
        );
        assert_eq!(nether["use_expansion_hack"], true);
        for gone in [
            "y_allowance",
            "use_bounding_box_hack",
            "land_search_direction",
            "ledge_offset_y",
        ] {
            assert!(nether.get(gone).is_none(), "{gone}");
        }
        let mut moogs = json!({"type": "moogs_structures:moogs_structures_generic_jigsaw_structure",
            "burying_type": "AVERAGE_LAND"});
        assert!(upgrade("structure", &mut moogs, &mut report, &NO_TAGS));
        assert_eq!(
            moogs["horizon:piece_limited"],
            json!({"library": "moogs", "burying": "average_land"})
        );
        // Repurposed's mineshaft is laid out under the terrain.
        let mut mineshaft = json!({"type": "repurposed_structures:generic_mineshaft",
            "start_pool": "rs:mineshafts/birch/start_pool", "size": 9,
            "liquid_settings": "ignore_waterlogging"});
        assert!(upgrade("structure", &mut mineshaft, &mut report, &NO_TAGS));
        assert_eq!(mineshaft["type"], "minecraft:jigsaw");
        assert_eq!(
            mineshaft["horizon:piece_limited"],
            json!({"library": "repurposed", "layout": "mineshaft"})
        );
        assert_eq!(mineshaft["liquid_settings"], "ignore_waterlogging");
        // Lithostitched's jigsaw is vanilla's: no key.
        let mut litho = json!({"type": "lithostitched:jigsaw"});
        assert!(upgrade("structure", &mut litho, &mut report, &NO_TAGS));
        assert!(litho.get("horizon:piece_limited").is_none());
        assert!(report.rules_lost.is_empty(), "{:?}", report.rules_lost);
        assert!(report.unsupported.is_empty());

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, text) in [
            (
                "data/rs/rs_pieces_spawn_counts/city.json",
                r#"{"target_structure": "rs:city_nether", "pieces_spawn_counts": [
                    {"nbt_piece_name": "rs:city/center", "always_spawn_this_many": 1,
                     "never_spawn_more_than_this_many": 1, "minimum_distance_from_center_piece": 3}]}"#,
            ),
            (
                "data/mvs/msl_pieces_spawn_counts/tower.json",
                r#"{"pieces_spawn_counts": [{"nbt_piece_name": "mvs:top", "never_spawn_more_than_this_many": 2}]}"#,
            ),
        ] {
            zip.start_file(name, opts).unwrap();
            std::io::Write::write_all(&mut zip, text.as_bytes()).unwrap();
        }
        let mut zip = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        let counts = piece_counts(&mut zip);
        assert_eq!(
            counts["rs:city_nether"],
            json!([{"name": "rs:city/center", "required": 1, "max": 1, "min_depth": 3}])
        );
        assert_eq!(counts["mvs:tower"], json!([{"name": "mvs:top", "max": 2}]));
        assert!(add_piece_counts("rs:city_nether", &mut nether, &counts));
        assert_eq!(nether["horizon:piece_limited"]["pieces"][0]["required"], 1);
        assert!(!add_piece_counts("rs:other", &mut nether, &counts));
    }

    #[test]
    fn writes_block_states_in_26_3_shape() {
        let mut report = Upgrade::default();
        let mut list = json!({"processors": [{"processor_type": "minecraft:rule", "rules": [{
            "input_predicate": {"predicate_type": "minecraft:always_true"},
            "output_state": {"Name": "minecraft:oak_stairs", "Properties": {"facing": "east"}}}]}]});
        assert!(upgrade("processor_list", &mut list, &mut report, &NO_TAGS));
        assert_eq!(
            list["processors"][0]["rules"][0]["output_state"],
            json!({"id": "minecraft:oak_stairs", "properties": {"facing": "east"}})
        );
        let mut spring = json!({"type": "minecraft:spring_feature", "config": {
            "state": {"Name": "minecraft:water"}, "valid_blocks": ["minecraft:stone"]}});
        assert!(upgrade(
            "configured_feature",
            &mut spring,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(spring["config"]["state"], json!({"id": "minecraft:water"}));
        // An object with other keys is not a state; a 26.3 state is left as it is.
        let mut other = json!({"Name": "x", "weight": 1, "state": {"id": "minecraft:stone"}});
        assert!(!upgrade(
            "configured_feature",
            &mut other,
            &mut report,
            &NO_TAGS
        ));
        assert_eq!(report.block_states, 2);
        assert_eq!(report.mapped, 0);
    }

    #[test]
    fn convention_tags_include_theirs() {
        let conventions = BTreeMap::from([
            (
                "is_hot".to_owned(),
                json!({"values": ["#c:is_desert", "#minecraft:is_badlands"]}),
            ),
            (
                "is_desert".to_owned(),
                json!({"values": ["minecraft:desert"]}),
            ),
            ("is_cold".to_owned(), json!({"values": []})),
        ]);
        let mut used = BTreeSet::new();
        convention_tags(
            &json!({"biomes": ["#c:is_hot", "#c:unknown", "#minecraft:x"]}),
            &mut used,
        );
        assert_eq!(used, BTreeSet::from(["is_hot".into(), "unknown".into()]));
        assert_eq!(
            with_included(&used, &conventions),
            BTreeSet::from(["is_hot".into(), "is_desert".into()])
        );
    }
}
