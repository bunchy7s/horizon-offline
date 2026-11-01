//! Lithostitched's worldgen, carried as the game reads it (clean room:
//! `cleanroom/structures/spec-lithostitched.md`; CTOV's processor:
//! `spec-ctov.md` §4).
//!
//! - `lithostitched:jigsaw` structures become vanilla's jigsaw. Lithostitched
//!   lays jigsaws out its own way (OPEN-L5): Horizon keeps vanilla's layouts.
//! - Its processors (`block_swap`, `reference`, `apply_random`, `condition`,
//!   `set_block`, `discard_input`) are the game's own types
//!   (`horizon_world`'s `structure::processor`): they are carried as written,
//!   the processors nested in them carried in turn. CTOV's
//!   `ctov:modular_compat` (a reference that runs only when its mod is loaded)
//!   becomes a `reference` when the mod is ported along, else nothing.
//! - `add_template_pool_elements` and `add_processor_list_processors`
//!   modifiers become `horizon:add_to` pool and processor list files
//!   (`docs/mods.md`), with their `priority`, when their load conditions pass.
//! - Its `limited` and `guaranteed` pool elements are the game's own
//!   (`horizon_world`'s `structure::jigsaw`); `delegating` places its
//!   `delegate`, and what it adds is counted as lost.
use crate::{processors::ProcessorData, worldgen::Upgrade};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const JIGSAW: &str = "lithostitched:jigsaw";
/// Vanilla's JigsawStructure.MAX_DEPTH.
const MAX_SIZE: i64 = 20;
const MODULAR_COMPAT: &str = "ctov:modular_compat";
/// The processor types the game reads.
const PROCESSORS: [&str; 6] = [
    "lithostitched:block_swap",
    "lithostitched:reference",
    "lithostitched:apply_random",
    "lithostitched:condition",
    "lithostitched:set_block",
    "lithostitched:discard_input",
];

fn count(map: &mut BTreeMap<String, usize>, name: &str) {
    *map.entry(name.to_owned()).or_default() += 1;
}

/// A `lithostitched:jigsaw` structure as vanilla's jigsaw: `Some(true)`, else
/// `None`.
pub(crate) fn upgrade(
    kind: &str,
    v: &mut Value,
    lost: &mut BTreeMap<String, usize>,
) -> Option<bool> {
    if kind != "structure" || v["type"] != JIGSAW {
        return None;
    }
    v["type"] = "minecraft:jigsaw".into();
    // `size` may be an int provider (sampled per start): its largest value.
    let size = match &v["size"] {
        Value::Null => None,
        Value::Number(n) => n.as_i64(),
        provider => {
            count(lost, "lithostitched:jigsaw size provider");
            ["max_inclusive", "value"]
                .iter()
                .find_map(|k| provider[k].as_i64())
        }
    };
    if let Some(size) = size {
        if size > MAX_SIZE {
            count(lost, "lithostitched:jigsaw size over 20");
        }
        v["size"] = size.clamp(0, MAX_SIZE).into();
    }
    let o = v.as_object_mut()?;
    match o.remove("start_projection") {
        Some(Value::String(heightmap)) if heightmap != "floor" && heightmap != "ceiling" => {
            o.entry("project_start_to_heightmap")
                .or_insert(heightmap.into());
        }
        Some(_) => count(lost, "lithostitched:jigsaw start_projection"),
        None => {}
    }
    if o.remove("fixed_rotation").is_some_and(|f| f == true) {
        count(lost, "lithostitched:jigsaw fixed_rotation");
    }
    Some(true)
}

/// Lithostitched's pool elements as the element they wrap; whether `element`
/// was one.
pub(crate) fn upgrade_pool_element(
    element: &mut Value,
    lost: &mut BTreeMap<String, usize>,
) -> bool {
    let Some(kind) = element["element_type"].as_str() else {
        return false;
    };
    let unwrap = match kind {
        "lithostitched:guaranteed" => false,
        // A `limit` is what it adds (spec-lithostitched §3).
        "lithostitched:limited" => element["limit"].as_i64().is_none(),
        "lithostitched:delegating" => true,
        _ => return false,
    };
    if unwrap {
        count(lost, kind);
        let delegate = element["delegate"].take();
        *element = delegate;
        // A delegate wrapping another (only `delegating` allows it).
        upgrade_pool_element(element, lost);
    }
    true
}

/// The ids a modifier targets: an id or a list of ids (`None` for tags and
/// Lithostitched's object forms).
fn ids(v: &Value) -> Option<Vec<String>> {
    let id = |v: &Value| {
        v.as_str()
            .filter(|s| !s.starts_with('#'))
            .map(str::to_owned)
    };
    match v {
        Value::Array(a) => a.iter().map(id).collect(),
        other => Some(vec![id(other)?]),
    }
}

/// NeoForge's load conditions (`neoforge:conditions`): all must pass.
fn conditions_pass(v: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
    fn pass(c: &Value, loaded: &dyn Fn(&str) -> bool) -> bool {
        let all = |key: &str| {
            c[key]
                .as_array()
                .into_iter()
                .flatten()
                .all(|c| pass(c, loaded))
        };
        match c["type"].as_str() {
            Some("neoforge:mod_loaded") => c["modid"].as_str().is_some_and(loaded),
            Some("neoforge:true") => true,
            Some("neoforge:not") => !pass(&c["value"], loaded),
            Some("neoforge:and") => all("values"),
            Some("neoforge:or") => {
                (c["values"].as_array().into_iter().flatten()).any(|c| pass(c, loaded))
            }
            _ => false,
        }
    }
    (v["neoforge:conditions"].as_array().into_iter().flatten()).all(|c| pass(c, loaded))
}

/// A worldgen modifier as a file the game merges: `("template_pool", file)`
/// for `add_template_pool_elements`, `("processor_list", file)` for
/// `add_processor_list_processors` (spec-lithostitched §2.1, §2.2); the
/// others are counted in `report.modifiers_left`.
pub(crate) fn modifier(
    v: &Value,
    loaded: &dyn Fn(&str) -> bool,
    report: &mut Upgrade,
) -> Option<(&'static str, Value)> {
    let kind = v["type"].as_str().unwrap_or("no type");
    if !conditions_pass(v, loaded) {
        count(&mut report.modifiers_left, "conditions");
        return None;
    }
    // 1.8.1 reads `processor_lists`, not the wiki's `processor_list`.
    let (to, targets, list) = match kind {
        "lithostitched:add_template_pool_elements" => {
            ("template_pool", "template_pools", "elements")
        }
        "lithostitched:add_processor_list_processors" => {
            ("processor_list", "processor_lists", "processors")
        }
        _ => {
            count(&mut report.modifiers_left, kind);
            return None;
        }
    };
    let (Some(targets), Some(entries)) = (ids(&v[targets]), v[list].as_array()) else {
        count(&mut report.modifiers_left, "target");
        return None;
    };
    for target in &targets {
        *report.added_to.entry(target.clone()).or_default() += entries.len();
    }
    let mut file = json!({"horizon:add_to": targets, list: entries});
    if let Some(priority) = v["priority"].as_i64() {
        file["horizon:priority"] = priority.into();
    }
    Some((to, file))
}

/// Whether the game reads a Lithostitched processor as it is: one of its
/// types, with no processor list named by tag (the game has no processor
/// list tags) and a state provider it runs.
pub(crate) fn game_reads(p: &Value) -> bool {
    let Some(kind) = p["processor_type"]
        .as_str()
        .filter(|t| PROCESSORS.contains(t))
    else {
        return false;
    };
    let tag = |v: &Value| v.as_str().is_some_and(|s| s.starts_with('#'));
    match kind {
        "lithostitched:reference" => match &p["processor_lists"] {
            Value::Array(a) => !a.iter().any(tag),
            other => other.is_string() && !tag(other),
        },
        "lithostitched:apply_random" => p["processor_lists"]
            .as_array()
            .is_some_and(|a| a.iter().all(|e| !tag(e) && !tag(&e["data"]))),
        "lithostitched:set_block" => matches!(
            p["state_provider"]["type"].as_str(),
            Some("minecraft:simple_state_provider" | "minecraft:weighted_state_provider")
        ),
        "lithostitched:block_swap" => p["blocks"].is_object(),
        "lithostitched:condition" => !p["if_true"].is_null(),
        _ => true,
    }
}

/// The processor lists inside a Lithostitched processor, for the general
/// code that walks lists (`then`, `else`, `apply_random`'s inline data).
pub(crate) fn inner_lists(p: &mut Value) -> Vec<&mut Value> {
    match p["processor_type"].as_str() {
        Some("lithostitched:apply_random") => (p["processor_lists"].as_array_mut().into_iter())
            .flatten()
            .filter_map(|e| e.get_mut("data").filter(|d| !d.is_string()))
            .collect(),
        Some("lithostitched:condition") => (p.as_object_mut().into_iter())
            .flat_map(|o| o.iter_mut())
            .filter(|(k, _)| *k == "then" || *k == "else")
            .map(|(_, v)| v)
            .collect(),
        _ => Vec::new(),
    }
}

/// A Lithostitched processor with the processors nested in it carried
/// (`Some`, when one of them changed); `None` for any other processor.
pub(crate) fn processor(
    kind: &str,
    p: &Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> Option<Vec<Value>> {
    if !PROCESSORS.contains(&kind) {
        return None;
    }
    let mut out = p.clone();
    let mut changed = false;
    for list in inner_lists(&mut out) {
        // One processor alone is a list of one.
        if list.get("processor_type").is_some() {
            *list = Value::Array(vec![list.take()]);
            changed = true;
        }
        if let Some(processors) = crate::processors::processors(list) {
            let (inner, inner_changed) =
                crate::processors::carry_all(std::mem::take(processors), data, carried);
            *processors = inner;
            changed |= inner_changed;
        }
    }
    changed.then(|| vec![out])
}

/// The list with CTOV's modular compat processors written as the game's
/// `reference` (its mod is ported along) or taken out; whether any was.
pub(crate) fn expand_references(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    let mut changed = false;
    let mut out = Vec::with_capacity(list.len());
    for p in list {
        if p["processor_type"] != MODULAR_COMPAT {
            out.push(p);
            continue;
        }
        changed = true;
        count(carried, MODULAR_COMPAT);
        let loaded = p["modid"]
            .as_str()
            .is_some_and(|m| data.compat_loaded.contains(m));
        if loaded {
            out.push(json!({"processor_type": "lithostitched:reference",
                "processor_lists": p["reference"].clone()}));
        }
    }
    (out, changed)
}

/// Notes which mods CTOV's modular compat processors name are `loaded`.
pub(crate) fn mark_loaded(data: &mut ProcessorData, loaded: &dyn Fn(&str) -> bool) {
    fn visit(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                if o.get("processor_type").and_then(Value::as_str) == Some(MODULAR_COMPAT)
                    && let Some(m) = o.get("modid").and_then(Value::as_str)
                {
                    out.push(m.to_owned());
                }
                o.values().for_each(|v| visit(v, out));
            }
            Value::Array(a) => a.iter().for_each(|v| visit(v, out)),
            _ => {}
        }
    }
    let mut named = Vec::new();
    data.lists.values().for_each(|l| visit(l, &mut named));
    data.compat_loaded
        .extend(named.into_iter().filter(|m| loaded(m)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::upgrade as upgrade_file;

    #[test]
    fn jigsaws_become_vanillas() {
        let mut lost = BTreeMap::new();
        let mut s = json!({"type": JIGSAW, "size": {"type": "uniform", "min_inclusive": 3,
            "max_inclusive": 40}, "start_projection": "WORLD_SURFACE_WG", "fixed_rotation": true});
        assert_eq!(upgrade("structure", &mut s, &mut lost), Some(true));
        assert_eq!(
            s,
            json!({"type": "minecraft:jigsaw", "size": 20,
                "project_start_to_heightmap": "WORLD_SURFACE_WG"})
        );
        assert_eq!(lost.len(), 3);
        assert_eq!(
            upgrade(
                "structure",
                &mut json!({"type": "minecraft:jigsaw"}),
                &mut lost
            ),
            None
        );
    }

    #[test]
    fn limited_and_guaranteed_elements_stay_theirs() {
        let mut lost = BTreeMap::new();
        let delegate = json!({"element_type": "minecraft:single_pool_element", "location": "a:b",
            "projection": "rigid", "processors": "minecraft:empty"});
        for kept in [
            json!({"element_type": "lithostitched:limited", "limit": 1, "delegate": delegate}),
            json!({"element_type": "lithostitched:guaranteed", "count": 1, "min_depth": 3,
                "delegate": delegate}),
        ] {
            let mut e = kept.clone();
            assert!(upgrade_pool_element(&mut e, &mut lost));
            assert_eq!(e, kept);
        }
        assert!(lost.is_empty());
        // Without a limit, and `delegating`: the delegate, the rest counted.
        for kind in ["lithostitched:limited", "lithostitched:delegating"] {
            let mut e = json!({"element_type": kind, "delegate": delegate});
            assert!(upgrade_pool_element(&mut e, &mut lost));
            assert_eq!(e, delegate);
            assert_eq!(lost[kind], 1);
        }
        assert!(!upgrade_pool_element(&mut delegate.clone(), &mut lost));
    }

    #[test]
    fn modifiers_add_to_pools_when_their_mods_are_loaded() {
        let mut report = Upgrade::default();
        let loaded = |m: &str| m == "waystones";
        let add = |modid: &str, pools: Value| {
            json!({"type": "lithostitched:add_template_pool_elements", "template_pools": pools,
                "elements": [{"weight": 2, "element": {"element_type": "minecraft:empty_pool_element"}}],
                "neoforge:conditions": [{"type": "neoforge:mod_loaded", "modid": modid}]})
        };
        let (kind, file) = modifier(
            &add("waystones", json!("ctov:village/plains/house")),
            &loaded,
            &mut report,
        )
        .unwrap();
        assert_eq!(kind, "template_pool");
        assert_eq!(file["horizon:add_to"], json!(["ctov:village/plains/house"]));
        assert_eq!(file["elements"][0]["weight"], 2);
        assert_eq!(report.added_to["ctov:village/plains/house"], 1);
        assert!(modifier(&add("sawmill", json!("a:b")), &loaded, &mut report).is_none());
        assert!(modifier(&add("waystones", json!("#a:pools")), &loaded, &mut report).is_none());
        let aliases = json!({"type": "lithostitched:set_pool_aliases"});
        assert!(modifier(&aliases, &loaded, &mut report).is_none());
        // Processors appended to lists, with the modifier's priority; 1.8.1
        // reads `processor_lists` (spec-lithostitched §2.2).
        let nop = json!([{"processor_type": "minecraft:nop"}]);
        let processors = json!({"type": "lithostitched:add_processor_list_processors",
            "priority": 10, "processor_lists": ["minecraft:mossify_10_percent"], "processors": nop});
        let (kind, file) = modifier(&processors, &loaded, &mut report).unwrap();
        assert_eq!(kind, "processor_list");
        assert_eq!(
            file,
            json!({"horizon:add_to": ["minecraft:mossify_10_percent"], "processors": nop,
                "horizon:priority": 10})
        );
        let wiki = json!({"type": "lithostitched:add_processor_list_processors",
            "processor_list": "minecraft:mossify_10_percent", "processors": nop});
        assert!(modifier(&wiki, &loaded, &mut report).is_none());
        assert_eq!(
            report.modifiers_left,
            BTreeMap::from([
                ("conditions".into(), 1),
                ("lithostitched:set_pool_aliases".into(), 1),
                ("target".into(), 2),
            ])
        );
    }

    /// A CTOV list (spec-ctov §4): references, a condition with a single
    /// processor and a nested condition, a random list holding the compat
    /// hook, and processors the game cannot read.
    #[test]
    fn processors_stay_lithostitcheds_with_their_insides_carried() {
        let hook = json!({"processor_type": MODULAR_COMPAT, "modid": "natures_spirit",
            "reference": "ctov:village/mesa/house_wisteria"});
        let list = json!({"processors": [
            {"processor_type": "lithostitched:reference", "processor_lists": ["ctov:village/mesa/house"]},
            {"processor_type": "lithostitched:condition", "random_mode": "per_block",
                "if_true": {"type": "lithostitched:matching_blocks", "blocks": "minecraft:water",
                    "match_type": "location"},
                "then": {"processor_type": "lithostitched:set_block", "state_provider": {
                    "type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:spruce_planks"}}},
                "else": [{"processor_type": "somemod:java_processor"}]},
            {"processor_type": "lithostitched:apply_random", "mode": "per_piece", "processor_lists": [
                {"weight": 1, "data": "minecraft:empty"},
                {"weight": 1, "data": {"processors": [hook]}}]},
            {"processor_type": "lithostitched:reference", "processor_lists": "#ctov:palettes"},
            {"processor_type": "lithostitched:set_block", "state_provider": {
                "type": "minecraft:noise_provider"}}]});
        for (loaded, wisteria) in [
            (false, json!([])),
            (
                true,
                json!([{"processor_type":
            "lithostitched:reference", "processor_lists": "ctov:village/mesa/house_wisteria"}]),
            ),
        ] {
            let mut data = ProcessorData::default();
            data.lists
                .insert("ctov:village/mesa/house".into(), list.clone());
            mark_loaded(&mut data, &|m| loaded && m == "natures_spirit");
            let mut v = list.clone();
            let mut report = Upgrade::default();
            crate::processors::carry(
                "processor_list",
                &mut v,
                &data,
                &mut report.processors_carried,
            );
            upgrade_file("processor_list", &mut v, &mut report, &BTreeMap::new());
            let p = &v["processors"];
            assert_eq!(p.as_array().unwrap().len(), 3, "{p}");
            assert_eq!(p[0], list["processors"][0]);
            // One processor is a list of one; the Java one is taken out.
            assert_eq!(
                p[1]["then"][0]["state_provider"]["state"]["id"],
                "minecraft:spruce_planks"
            );
            assert_eq!(p[1]["else"], json!([]));
            assert_eq!(p[2]["processor_lists"][0]["data"], "minecraft:empty");
            assert_eq!(p[2]["processor_lists"][1]["data"]["processors"], wisteria);
            assert_eq!(report.processors_carried[MODULAR_COMPAT], 1);
            assert_eq!(
                report.processors_dropped,
                BTreeMap::from([
                    ("lithostitched:reference".into(), 1),
                    ("lithostitched:set_block".into(), 1),
                    ("somemod:java_processor".into(), 1),
                ])
            );
        }
    }
}
