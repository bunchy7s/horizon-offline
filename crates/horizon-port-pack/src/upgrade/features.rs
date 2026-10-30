//! Configured and placed features.
use super::{Report, kind, states, vanilla};
use serde_json::{Map, Value, json};

/// Feature types 26.3 dropped for compositions of other features; a pack
/// using them needs vanilla's composition, which the upgrade does not
/// write.
const REPLACED_FEATURES: [&str; 12] = [
    "basalt_columns",
    "basalt_pillar",
    "coral_mushroom",
    "desert_well",
    "glowstone_blob",
    "kelp",
    "nether_forest_vegetation",
    "sea_pickle",
    "seagrass",
    "twisting_vines",
    "weeping_vines",
    "random_patch",
];

/// 26.2 block state provider types and their 26.3 names.
fn provider(t: &str) -> Option<&'static str> {
    Some(match t {
        "simple_state_provider" => "simple_state_provider",
        "weighted_state_provider" => "weighted",
        "rotated_block_provider" => "rotated",
        "randomized_int_state_provider" => "randomized_int",
        "rule_based_state_provider" => "rule_based",
        "noise_provider" => "noise",
        "dual_noise_provider" => "dual_noise",
        "noise_threshold_provider" => "noise_threshold",
        _ => return None,
    })
}

/// One placement modifier: `random_offset` became `offset` with one
/// provider per axis (x and z both from `xz_spread`, drawn in that order).
fn placement(m: &mut Value) {
    if kind(m) == Some("random_offset") {
        let xz = m.get("xz_spread").cloned().unwrap_or(json!(0));
        let y = m.get("y_spread").cloned().unwrap_or(json!(0));
        *m = json!({"type": "minecraft:offset", "x": xz.clone(), "y": y, "z": xz});
    }
}

/// A configured feature in 26.3's form: `config` unwrapped, block state
/// providers renamed, nested features and placements upgraded.
pub(super) fn feature(v: &Value, report: &mut Report) -> Value {
    let Value::Object(o) = v else {
        return v.clone();
    };
    let Some(t) = kind(v) else {
        return v.clone();
    };
    if REPLACED_FEATURES.contains(&t) {
        report.unknown(format!("feature {t} (written as data in 26.3)"));
    }
    let mut n = Map::new();
    n.insert("type".into(), o["type"].clone());
    if let Some(Value::Object(config)) = o.get("config") {
        for (k, x) in config {
            n.insert(k.clone(), x.clone());
        }
    }
    let mut v = Value::Object(n);
    states(&mut v);
    nested(&mut v, report);
    v
}

/// Inside a feature: block state providers, placed features and
/// configured features held inline.
fn nested(v: &mut Value, report: &mut Report) {
    match v {
        Value::Object(o) => {
            if let Some(t) = kind(&Value::Object(o.clone())) {
                if let Some(to) = provider(t) {
                    if t == "simple_state_provider" {
                        *v = o.get("state").cloned().unwrap_or_default();
                        return;
                    }
                    o.insert("type".into(), vanilla(to));
                }
            }
            // An inline placed feature: {feature, placement}.
            if o.contains_key("placement") && o.contains_key("feature") {
                let placed = placed(&Value::Object(o.clone()), report);
                *v = placed;
                return;
            }
            for x in o.values_mut() {
                if x.get("config").is_some() && x.get("type").is_some() {
                    *x = feature(x, report);
                } else {
                    nested(x, report);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| nested(x, report)),
        _ => {}
    }
}

/// A placed feature: its feature (an id or inline) and its placement
/// modifiers.
pub(super) fn placed(v: &Value, report: &mut Report) -> Value {
    let mut v = v.clone();
    states(&mut v);
    if let Some(f) = v.get("feature").cloned()
        && f.is_object()
    {
        v["feature"] = feature(&f, report);
    }
    if let Some(Value::Array(ms)) = v.get_mut("placement") {
        for m in ms.iter_mut() {
            placement(m);
        }
    }
    v
}
