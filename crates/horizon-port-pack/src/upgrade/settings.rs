//! Noise settings, dimensions and world presets.
use super::{
    Report,
    density::{Context, cell_of},
    kind, states, vanilla,
};
use serde_json::{Map, Value, json};

impl Context<'_> {
    /// A density function the settings use in several places, as an id: its
    /// own id when it is one, otherwise a new named function under
    /// `<settings>/upgraded/<name>`.
    pub(super) fn named(&mut self, settings: &str, name: &str, v: Value) -> Value {
        if v.is_string() {
            return v;
        }
        let (ns, path) = settings.split_once(':').unwrap_or(("minecraft", settings));
        let id = format!("{ns}:{path}/upgraded/{name}");
        self.generated.insert(id.clone(), v);
        Value::String(id)
    }

    /// 26.2 noise settings in 26.3's form.
    pub(super) fn settings(&mut self, id: &str, v: &Value, report: &mut Report) -> Value {
        let Value::Object(o) = v else {
            return v.clone();
        };
        if o.contains_key("material_rule") {
            return v.clone();
        }
        let router = o.get("noise_router").cloned().unwrap_or_default();
        self.cell = cell_of(o.get("noise").unwrap_or(&Value::Null));
        let conv = |this: &mut Self, report: &mut Report, name: &str| {
            this.density(&router.get(name).cloned().unwrap_or(json!(0.0)), report)
        };
        // The climate functions are named: the spawn target and the aquifer
        // exclusion refer to them by id.
        let climate = |this: &mut Self, report: &mut Report, name: &str| {
            let f = conv(this, report, name);
            this.named(id, name, f)
        };
        let temperature = climate(self, report, "temperature");
        let vegetation = climate(self, report, "vegetation");
        let continents = climate(self, report, "continents");
        let erosion = climate(self, report, "erosion");
        let depth = climate(self, report, "depth");
        let ridges = climate(self, report, "ridges");
        let final_density = conv(self, report, "final_density");
        let surface = conv(self, report, "preliminary_surface_level");
        let surface = self.named(id, "preliminary_surface_level", surface);
        let mut n = Map::new();
        let mut noise = o.get("noise").cloned().unwrap_or_default();
        if let Some(noise) = noise.as_object_mut() {
            noise.remove("size_horizontal");
            noise.remove("size_vertical");
        }
        n.insert("noise".into(), noise);
        for key in ["default_block", "default_fluid"] {
            let mut s = o.get(key).cloned().unwrap_or_default();
            states(&mut s);
            n.insert(key.into(), s);
        }
        let aquifers = o
            .get("aquifers_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if aquifers {
            let barrier = conv(self, report, "barrier");
            let floodedness = conv(self, report, "fluid_level_floodedness");
            let spread = conv(self, report, "fluid_level_spread");
            let lava = conv(self, report, "lava");
            // 26.2's OverworldBiomeBuilder.isDeepDarkRegion: no aquifer
            // where erosion < -0.225 and depth > 0.9 (positive excludes).
            let exclusion = json!({"type": "minecraft:min",
                "left": {"type": "minecraft:sub", "left": -0.225, "right": erosion.clone()},
                "right": {"type": "minecraft:max",
                    "left": {"type": "minecraft:sub", "left": depth.clone(), "right": 0.9},
                    "right": 0.0}});
            n.insert(
                "aquifers".into(),
                json!({"barrier": barrier, "exclusion": exclusion,
                    "fluid_level_floodedness": floodedness, "fluid_level_spread": spread,
                    "lava": lava, "surface_level": surface.clone()}),
            );
        }
        let mut rule = o.get("surface_rule").cloned().unwrap_or(json!({
            "type": "minecraft:sequence", "sequence": []}));
        states(&mut rule);
        let veins = o
            .get("ore_veins_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let rule = if veins {
            let toggle = conv(self, report, "vein_toggle");
            let toggle = self.named(
                id,
                "vein_toggle",
                json!({"type": "minecraft:cache", "input": toggle}),
            );
            let ridged = conv(self, report, "vein_ridged");
            let ridged = self.named(id, "vein_ridged", ridged);
            let gap = conv(self, report, "vein_gap");
            let gap = self.named(id, "vein_gap", gap);
            // Ore veins come first: 26.2 placed them while filling the
            // noise, and its surface pass only replaced the default block.
            let mut sequence = ore_veins(&toggle, &ridged, &gap);
            sequence.push(rule);
            json!({"type": "minecraft:sequence", "sequence": sequence})
        } else {
            rule
        };
        n.insert(
            "noise_router".into(),
            json!({"temperature": temperature, "vegetation": vegetation,
                "continents": continents, "erosion": erosion,
                "depth": depth, "ridges": ridges,
                "chunk_surface_level": {"type": "minecraft:interpolated", "cell_size_xz": 16,
                    "cell_size_y": 1, "input": surface},
                "final_density": {"type": "minecraft:add", "left": final_density,
                    "right": {"type": "minecraft:beardifier"}}}),
        );
        // Named, as vanilla names its own (and as Horizon reads them), under
        // the settings' id so other vanilla settings keep their rule.
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let rule_id = format!("{ns}:{path}/upgraded/material_rule");
        self.rules.insert(rule_id.clone(), rule);
        n.insert("material_rule".into(), Value::String(rule_id));
        let climate = [
            ("temperature", n["noise_router"]["temperature"].clone()),
            ("humidity", n["noise_router"]["vegetation"].clone()),
            ("continentalness", n["noise_router"]["continents"].clone()),
            ("erosion", n["noise_router"]["erosion"].clone()),
            ("weirdness", n["noise_router"]["ridges"].clone()),
        ];
        let spawn: Vec<Value> = o
            .get("spawn_target")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|p| {
                for key in ["depth", "offset"] {
                    if p.get(key).is_some_and(|d| d.as_f64() != Some(0.0)) {
                        report.lossy(format!("noise settings: spawn target {key} other than 0"));
                    }
                }
                let mut point = Map::new();
                for (param, f) in &climate {
                    if let (Some(range), Some(f)) = (p.get(*param), f.as_str()) {
                        point.insert(f.into(), range.clone());
                    }
                }
                Value::Object(point)
            })
            .collect();
        n.insert("spawn_target".into(), Value::Array(spawn));
        for key in [
            "sea_level",
            "disable_mob_generation",
            "legacy_random_source",
        ] {
            if let Some(x) = o.get(key) {
                n.insert(key.into(), x.clone());
            }
        }
        Value::Object(n)
    }

    /// A dimension (`dimension/<id>.json` or a world preset's entry): an
    /// inline noise settings object in its generator is upgraded.
    pub(super) fn dimension(&mut self, v: &Value, report: &mut Report) -> Value {
        let mut v = v.clone();
        let noise = v.get("generator").and_then(kind) == Some("noise");
        if let Some(settings) = v.pointer("/generator/settings").cloned()
            && settings.is_object()
            && noise
        {
            v["generator"]["settings"] = self.settings("minecraft:inline", &settings, report);
        }
        v
    }

    pub(super) fn world_preset(&mut self, v: &Value, report: &mut Report) -> Value {
        let mut v = v.clone();
        if let Some(dims) = v.get_mut("dimensions").and_then(Value::as_object_mut) {
            for d in dims.values_mut() {
                *d = self.dimension(d, report);
            }
        }
        v
    }
}

/// 26.2's `OreVeinifier` as 26.3 material rules, one per vein type, from
/// the pack's `vein_toggle`, `vein_ridged` and `vein_gap`: a block between
/// the type's heights where the toggle (copper above zero, iron below)
/// clears 0.4 less the edge roundoff, and the ridged noise is negative, is
/// the filler with chance 0.7, and ore where the richness draw and the gap
/// noise allow.
fn ore_veins(toggle: &Value, ridged: &Value, gap: &Value) -> Vec<Value> {
    let types = [
        ("copper_ore", "raw_copper_block", "granite", 0, 50, false),
        (
            "deepslate_iron_ore",
            "raw_iron_block",
            "tuff",
            -60,
            -8,
            true,
        ),
    ];
    types
        .iter()
        .map(|(ore, raw, filler, min_y, max_y, negate)| {
            let veininess = if *negate {
                json!({"type": "minecraft:negate", "input": toggle})
            } else {
                toggle.clone()
            };
            // clampedMap(min(maxY - y, y - minY), 0, 20, -0.2, 0)
            let edge = json!({"type": "minecraft:add",
                "left": {"type": "minecraft:mul",
                    "left": {"type": "minecraft:clamp",
                        "input": {"type": "minecraft:min",
                            "left": {"type": "minecraft:sub", "left": f64::from(*max_y), "right": "minecraft:y"},
                            "right": {"type": "minecraft:sub", "left": "minecraft:y", "right": f64::from(*min_y)}},
                        "min": 0.0, "max": 20.0},
                    "right": 0.01},
                "right": -0.2});
            let density = json!({"type": "minecraft:range_choice", "input": "minecraft:y",
                "min_inclusive": f64::from(*min_y), "max_exclusive": f64::from(max_y + 1),
                "when_in_range": {"type": "minecraft:range_choice", "input": ridged,
                    "min_inclusive": -1_000_000.0, "max_exclusive": 0.0,
                    "when_in_range": {"type": "minecraft:range_choice",
                        "input": {"type": "minecraft:add",
                            "left": {"type": "minecraft:sub", "left": veininess, "right": 0.4},
                            "right": edge},
                        "min_inclusive": 0.0, "max_exclusive": 1_000_000.0,
                        "when_in_range": 0.7, "when_out_of_range": -1.0},
                    "when_out_of_range": -1.0},
                "when_out_of_range": -1.0});
            // clampedMap(|toggle|, 0.4, 0.6, 0.1, 0.3)
            let richness = json!({"type": "minecraft:add",
                "left": {"type": "minecraft:clamp", "input": {"type": "minecraft:abs", "input": toggle},
                    "min": 0.4, "max": 0.6},
                "right": -0.3});
            json!({"type": "minecraft:ore_vein", "ore_block": vanilla(ore),
                "raw_ore_block": vanilla(raw), "filler_block": vanilla(filler),
                "raw_ore_chance": 0.02, "density": density, "richness": richness,
                "filler_gap": {"type": "minecraft:sub", "left": -0.3, "right": gap}})
        })
        .collect()
}
