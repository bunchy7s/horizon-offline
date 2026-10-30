//! Density functions.
use super::{PackFile, Report, contains_type, full_id, kind, noise::noise_holder, vanilla};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Vanilla 26.2 density functions whose value does not depend on y, which
/// a pack can name without defining them.
const VANILLA_FLAT: [&str; 13] = [
    "minecraft:zero",
    "minecraft:shift_x",
    "minecraft:shift_z",
    "minecraft:overworld/continents",
    "minecraft:overworld/erosion",
    "minecraft:overworld/ridges",
    "minecraft:overworld/ridges_folded",
    "minecraft:overworld/offset",
    "minecraft:overworld/factor",
    "minecraft:overworld/jaggedness",
    "minecraft:overworld_large_biomes/continents",
    "minecraft:overworld_large_biomes/erosion",
    "minecraft:end/sloped_cheese",
];

pub(super) struct Context<'a> {
    /// The pack's own density functions by id, in 26.2's form.
    functions: BTreeMap<String, &'a Value>,
    /// The interpolation cell (`size_horizontal * 4`, `size_vertical * 4`)
    /// of the noise settings being upgraded, which 26.3 writes on each
    /// `interpolated`.
    pub(super) cell: (i64, i64),
    /// The cells of the pack's noise settings that reach each of its named
    /// density functions through their routers.
    cells: BTreeMap<String, BTreeSet<(i64, i64)>>,
    /// The cell of every noise settings in the pack.
    all_cells: BTreeSet<(i64, i64)>,
    /// Named density functions the upgrade adds, by id.
    pub(super) generated: BTreeMap<String, Value>,
    /// Material rules the upgrade adds (one per noise settings), by id.
    pub(super) rules: BTreeMap<String, Value>,
}

impl<'a> Context<'a> {
    pub(super) fn new(files: &'a BTreeMap<String, Value>) -> Self {
        let mut functions = BTreeMap::new();
        let mut settings = Vec::new();
        for (path, v) in files {
            let Some(f) = PackFile::parse(path) else {
                continue;
            };
            match f.registry.as_str() {
                "worldgen/density_function" => {
                    functions.insert(f.id(), v);
                }
                "worldgen/noise_settings" => settings.push(v),
                _ => {}
            }
        }
        let mut ctx = Self {
            functions,
            cell: (4, 8),
            cells: BTreeMap::new(),
            all_cells: BTreeSet::new(),
            generated: BTreeMap::new(),
            rules: BTreeMap::new(),
        };
        for s in settings {
            let cell = cell_of(s.get("noise").unwrap_or(&Value::Null));
            ctx.all_cells.insert(cell);
            let mut reached = BTreeSet::new();
            ctx.reach(s.get("noise_router").unwrap_or(&Value::Null), &mut reached);
            for id in reached {
                ctx.cells.entry(id).or_default().insert(cell);
            }
        }
        ctx
    }

    /// Every pack density function `v` names, directly or through others.
    fn reach(&self, v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::String(id) => {
                let id = full_id(id);
                if let Some(f) = self.functions.get(&id)
                    && out.insert(id)
                {
                    self.reach(f, out);
                }
            }
            Value::Object(o) => o.values().for_each(|x| self.reach(x, out)),
            Value::Array(a) => a.iter().for_each(|x| self.reach(x, out)),
            _ => {}
        }
    }

    /// A named density function file in 26.3's form, with the cell of the
    /// noise settings that use it: vanilla's overworld cell when none of the
    /// pack's do, the pack's own when it has one.
    pub(super) fn named_function(&mut self, id: &str, v: &Value, report: &mut Report) -> Value {
        let cells = self.cells.get(id).cloned().unwrap_or_default();
        self.cell = match (cells.len(), self.all_cells.len()) {
            (1, _) => *cells.iter().next().unwrap_or(&(4, 8)),
            (0, 1) => *self.all_cells.iter().next().unwrap_or(&(4, 8)),
            (0, _) => (4, 8),
            _ => {
                if contains_type(v, "interpolated") {
                    report.lossy("density: an interpolated function shared by noise settings with different cells");
                }
                *cells.iter().next().unwrap_or(&(4, 8))
            }
        };
        self.density(v, report)
    }

    /// One density function (a number, an id, or an object) in 26.3's form.
    pub(super) fn density(&mut self, v: &Value, report: &mut Report) -> Value {
        let Value::Object(o) = v else {
            return v.clone();
        };
        let Some(t) = kind(v) else {
            return v.clone();
        };
        let arg = |name: &str| o.get(name).cloned().unwrap_or(Value::Null);
        let unary = |this: &mut Self, report: &mut Report, to: &str| {
            let input = this.density(&arg("argument"), report);
            json!({"type": format!("minecraft:{to}"), "input": input})
        };
        match t {
            "abs" | "square" | "cube" | "half_negative" | "quarter_negative" | "squeeze"
            | "blend_density" => unary(self, report, t),
            "invert" => unary(self, report, "reciprocal"),
            "add" | "mul" | "min" | "max" => {
                let left = self.density(&arg("argument1"), report);
                let right = self.density(&arg("argument2"), report);
                json!({"type": format!("minecraft:{t}"), "left": left, "right": right})
            }
            "interpolated" => {
                let input = self.density(&arg("argument"), report);
                json!({"type": "minecraft:interpolated", "cell_size_xz": self.cell.0,
                    "cell_size_y": self.cell.1, "input": input})
            }
            // 26.2's flat cache holds the value at y = 0 of each column
            // inside the chunk; 26.3's cache only memoises.
            "flat_cache" => {
                let raw = arg("argument");
                let input = self.density(&raw, report);
                let input = if self.depends_on_y(&raw, &mut BTreeSet::new()) {
                    json!({"type": "minecraft:slice", "axis": "y", "coordinate": 0, "input": input})
                } else {
                    input
                };
                json!({"type": "minecraft:cache", "input": input})
            }
            "cache_2d" | "cache_once" | "cache_all_in_cell" => {
                let raw = arg("argument");
                if t == "cache_2d" && self.depends_on_y(&raw, &mut BTreeSet::new()) {
                    report.lossy("density: cache_2d around a function of y");
                }
                let input = self.density(&raw, report);
                json!({"type": "minecraft:cache", "input": input})
            }
            "y_clamped_gradient" => {
                let (from, to) = (arg("from_y"), arg("to_y"));
                if from == to {
                    // A step at that y: clampedMap's NaN at the step itself
                    // has no 26.3 form.
                    report.lossy("density: y_clamped_gradient with from_y == to_y");
                    let y = from.as_f64().unwrap_or_default();
                    return json!({"type": "minecraft:range_choice", "input": "minecraft:y",
                        "min_inclusive": -1_000_000.0, "max_exclusive": y,
                        "when_in_range": arg("from_value"), "when_out_of_range": arg("to_value")});
                }
                json!({"type": "minecraft:gradient", "axis": "y", "from_coordinate": from,
                    "to_coordinate": to, "from_value": arg("from_value"),
                    "to_value": arg("to_value")})
            }
            "noise" => {
                let mut n = o.clone();
                n.insert("noise".into(), noise_holder(&arg("noise"), report));
                Value::Object(n)
            }
            "shifted_noise" => {
                let mut n = Map::new();
                n.insert("type".into(), vanilla("noise"));
                n.insert("noise".into(), noise_holder(&arg("noise"), report));
                n.insert("xz_scale".into(), arg("xz_scale"));
                n.insert("y_scale".into(), arg("y_scale"));
                n.insert("shift_x".into(), self.density(&arg("shift_x"), report));
                let shift_y = arg("shift_y");
                if !shift_y.as_f64().is_some_and(|y| y == 0.0) {
                    n.insert("shift_y".into(), self.density(&shift_y, report));
                }
                n.insert("shift_z".into(), self.density(&arg("shift_z"), report));
                Value::Object(n)
            }
            "shift_a" | "shift_b" | "shift" => {
                json!({"type": format!("minecraft:{t}"), "noise": noise_holder(&arg("argument"), report)})
            }
            "end_islands" => json!({"type": "minecraft:end_outer_islands"}),
            "constant" => arg("argument"),
            "clamp" | "range_choice" | "find_top_surface" | "interval_select" => {
                let mut n = o.clone();
                for key in [
                    "input",
                    "when_in_range",
                    "when_out_of_range",
                    "density",
                    "upper_bound",
                ] {
                    if let Some(x) = o.get(key) {
                        n.insert(key.into(), self.density(x, report));
                    }
                }
                if let Some(Value::Array(fs)) = o.get("functions") {
                    let fs = fs.iter().map(|f| self.density(f, report)).collect();
                    n.insert("functions".into(), Value::Array(fs));
                }
                Value::Object(n)
            }
            "spline" => {
                let spline = self.spline(&arg("spline"), report);
                json!({"type": "minecraft:spline", "spline": spline})
            }
            "old_blended_noise" | "blend_alpha" | "blend_offset" | "beardifier" => v.clone(),
            other => {
                report.unknown(format!("density function {other}"));
                v.clone()
            }
        }
    }

    /// A cubic spline: a number, or a coordinate and points whose values are
    /// splines themselves.
    fn spline(&mut self, v: &Value, report: &mut Report) -> Value {
        let Value::Object(o) = v else {
            return v.clone();
        };
        let mut n = o.clone();
        if let Some(c) = o.get("coordinate") {
            n.insert("coordinate".into(), self.density(c, report));
        }
        if let Some(Value::Array(points)) = o.get("points") {
            let points = points
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    if let Some(value) = p.get("value").cloned() {
                        p["value"] = self.spline(&value, report);
                    }
                    p
                })
                .collect();
            n.insert("points".into(), Value::Array(points));
        }
        Value::Object(n)
    }

    /// Whether a 26.2 density function's value can change with y, through
    /// the pack's own functions; unknown ids count as depending on y.
    fn depends_on_y(&self, v: &Value, visiting: &mut BTreeSet<String>) -> bool {
        match v {
            Value::Number(_) => false,
            Value::String(id) => {
                let id = if id.contains(':') {
                    id.clone()
                } else {
                    format!("minecraft:{id}")
                };
                if id == "minecraft:y" {
                    return true;
                }
                if let Some(f) = self.functions.get(&id) {
                    return visiting.insert(id.clone()) && {
                        let r = self.depends_on_y(f, visiting);
                        visiting.remove(&id);
                        r
                    };
                }
                !VANILLA_FLAT.contains(&id.as_str())
            }
            Value::Object(o) => match kind(v) {
                Some("y_clamped_gradient" | "old_blended_noise" | "beardifier" | "shift") => true,
                Some("flat_cache" | "end_islands" | "find_top_surface" | "constant") => false,
                Some("blend_alpha" | "blend_offset" | "shift_a" | "shift_b") => false,
                Some("noise") => o.get("y_scale").and_then(Value::as_f64) != Some(0.0),
                Some("shifted_noise") => {
                    o.get("y_scale").and_then(Value::as_f64) != Some(0.0)
                        || ["shift_x", "shift_y", "shift_z"]
                            .iter()
                            .any(|k| o.get(*k).is_some_and(|x| self.depends_on_y(x, visiting)))
                }
                Some("spline") => self.spline_depends_on_y(&o["spline"], visiting),
                _ => o
                    .iter()
                    .filter(|(k, _)| *k != "type")
                    .any(|(_, x)| match x {
                        Value::Array(a) => a.iter().any(|x| self.depends_on_y(x, visiting)),
                        x => self.depends_on_y(x, visiting),
                    }),
            },
            _ => false,
        }
    }

    fn spline_depends_on_y(&self, v: &Value, visiting: &mut BTreeSet<String>) -> bool {
        let Value::Object(o) = v else {
            return false;
        };
        o.get("coordinate")
            .is_some_and(|c| self.depends_on_y(c, visiting))
            || o.get("points").and_then(Value::as_array).is_some_and(|ps| {
                ps.iter().any(|p| {
                    p.get("value")
                        .is_some_and(|x| self.spline_depends_on_y(x, visiting))
                })
            })
    }
}

/// The interpolation cell of 26.2 `noise` settings.
pub(super) fn cell_of(noise: &Value) -> (i64, i64) {
    let size = |k: &str, d: i64| noise.get(k).and_then(Value::as_i64).unwrap_or(d) * 4;
    (size("size_horizontal", 1), size("size_vertical", 2))
}
