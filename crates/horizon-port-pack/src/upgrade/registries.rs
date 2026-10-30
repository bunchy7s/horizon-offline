//! Carvers, biomes, structures and dimension types.
use super::{Report, kind, states, vanilla};
use serde_json::{Map, Value, json};

/// A configured carver in 26.3's form. 26.2's cave carvers hard-coded their
/// count, thickness and the room's vertical scale per type; 26.3 reads them.
pub(super) fn carver(v: &Value, report: &mut Report) -> Value {
    let Some(t) = kind(v) else {
        return v.clone();
    };
    let mut c = v.get("config").cloned().unwrap_or_default();
    let Some(o) = c.as_object_mut() else {
        return v.clone();
    };
    o.remove("debug_settings");
    let lava = o.remove("lava_level");
    let replaceable = o.remove("replaceable");
    if lava.is_some_and(|l| l != json!({"above_bottom": 8}) && l != json!({"above_bottom": 10})) {
        report.lossy("carver: lava_level (26.3 takes it from the aquifer)");
    }
    if replaceable.is_some_and(|r| {
        r != json!("#minecraft:overworld_carver_replaceables")
            && r != json!("#minecraft:nether_carver_replaceables")
    }) {
        report.lossy("carver: replaceable (26.3 carves any stone)");
    }
    let y_scale = o.remove("yScale");
    let mut n = Map::new();
    match t {
        "cave" | "nether_cave" => {
            let nether = t == "nether_cave";
            n.insert("type".into(), vanilla("cave"));
            n.insert(
                "count".into(),
                json!({"type": "minecraft:very_biased_to_bottom", "min_inclusive": 0,
                    "max_inclusive": if nether { 9 } else { 14 }}),
            );
            n.insert(
                "thickness".into(),
                if nether {
                    json!({"type": "minecraft:trapezoid", "min": 0.0, "max": 6.0, "plateau": 2.0})
                } else {
                    json!({"type": "minecraft:trapezoid", "min": 0.0, "max": 3.0, "plateau": 1.0})
                },
            );
            if !nether {
                n.insert("weird_thickness_bias".into(), json!(true));
            } else {
                n.insert("start_vertical_radius_multiplier".into(), json!(5.0));
            }
            if let Some(s) = y_scale {
                n.insert("room_vertical_radius_multiplier".into(), s);
            }
        }
        "canyon" => {
            n.insert("type".into(), vanilla("canyon"));
            if let (Some(s), Some(shape)) =
                (y_scale, o.get_mut("shape").and_then(Value::as_object_mut))
            {
                shape.insert("y_scale".into(), s);
            }
        }
        other => {
            report.unknown(format!("carver {other}"));
            n.insert("type".into(), v["type"].clone());
        }
    }
    for (k, x) in o.iter() {
        n.insert(k.clone(), x.clone());
    }
    let mut v = Value::Object(n);
    states(&mut v);
    v
}

/// `{type, minCount, maxCount, weight}` to `{type, count, weight}`.
fn spawner(s: &Value) -> Value {
    let mut s = s.clone();
    if let Some(o) = s.as_object_mut() {
        let min = o.remove("minCount");
        let max = o.remove("maxCount");
        if let (Some(min), Some(max)) = (min, max) {
            let count = if min == max {
                min
            } else {
                json!({"type": "minecraft:uniform", "min_inclusive": min, "max_inclusive": max})
            };
            o.insert("count".into(), count);
        }
    }
    s
}

/// A biome in 26.3's form: mob spawns become the
/// `minecraft:gameplay/natural_mob_spawns` attribute (every category kept,
/// empty ones too, so nothing comes from the dimension's defaults), and the
/// creature spawn probability its own attribute.
pub(super) fn biome(v: &Value, report: &mut Report) -> Value {
    let mut v = v.clone();
    states(&mut v);
    let Some(o) = v.as_object_mut() else {
        return v;
    };
    let spawners = o.remove("spawners");
    let costs = o.remove("spawn_costs");
    let probability = o.remove("creature_spawn_probability");
    let attributes = o
        .entry("attributes")
        .or_insert_with(|| json!({}))
        .as_object_mut();
    let Some(attributes) = attributes else {
        report.lossy("biome: attributes is not an object");
        return v;
    };
    if let Some(Value::Object(categories)) = spawners {
        let by_category: Map<String, Value> = categories
            .iter()
            .map(|(c, list)| {
                let list = list.as_array().into_iter().flatten().map(spawner).collect();
                (c.clone(), Value::Array(list))
            })
            .collect();
        attributes.insert(
            "minecraft:gameplay/natural_mob_spawns".into(),
            json!({"modifier": "overlay", "argument": {
                "spawn_costs": costs.unwrap_or(json!({})),
                "spawns_by_category": by_category}}),
        );
    }
    if let Some(p) = probability
        && p.as_f64().is_some_and(|p| (p as f32) != 0.1)
    {
        attributes.insert(
            "minecraft:gameplay/creature_world_gen_spawn_probability".into(),
            p,
        );
    }
    v
}

/// A dimension type: the bed rule's `explodes` is 26.3's `destroy_on_use`.
pub(super) fn dimension_type(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(rule) = v
        .pointer_mut("/attributes/minecraft:gameplay~1bed_rule")
        .and_then(Value::as_object_mut)
        && let Some(explodes) = rule.remove("explodes")
    {
        rule.insert("destroy_on_use".into(), explodes);
    }
    v
}

/// A structure: its spawn overrides' counts in 26.3's form.
pub(super) fn structure(v: &Value) -> Value {
    let mut v = v.clone();
    states(&mut v);
    if let Some(Value::Object(overrides)) = v.get_mut("spawn_overrides") {
        for o in overrides.values_mut() {
            if let Some(Value::Array(spawns)) = o.get_mut("spawns") {
                for s in spawns.iter_mut() {
                    *s = spawner(s);
                }
            }
        }
    }
    v
}
