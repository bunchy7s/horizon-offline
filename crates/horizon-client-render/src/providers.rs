//! Vanilla's `context_int_provider` and `context_float_provider` values (furnace
//! fuel times, brewing times), evaluated from the data pack's JSON.
use crate::items::{ItemStack, Items};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

/// Evaluator for `context_int_provider` / `context_float_provider` values.
pub struct Providers {
    ints: HashMap<String, Value>,
    floats: HashMap<String, Value>,
}
fn load_dir(dir: &Path, prefix: &str, out: &mut HashMap<String, Value>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        if p.is_dir() {
            load_dir(&p, &format!("{prefix}{name}/"), out);
        } else if let Ok(bytes) = fs::read(&p)
            && let Ok(v) = serde_json::from_slice(&bytes)
        {
            out.insert(format!("minecraft:{prefix}{name}"), v);
        }
    }
}
impl Providers {
    pub fn load(root: &Path) -> Self {
        let mut ints = HashMap::new();
        let mut floats = HashMap::new();
        load_dir(
            &root.join("data/minecraft/context_int_provider"),
            "",
            &mut ints,
        );
        load_dir(
            &root.join("data/minecraft/context_float_provider"),
            "",
            &mut floats,
        );
        Self { ints, floats }
    }
    fn eval(v: &Value, fast: bool, table: &HashMap<String, Value>, depth: u32) -> f64 {
        if depth > 16 {
            return 0.0;
        }
        match v {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            Value::String(name) => table
                .get(name)
                .map_or(0.0, |v| Self::eval(v, fast, table, depth + 1)),
            Value::Object(o) => {
                let f =
                    |k: &str| Self::eval(o.get(k).unwrap_or(&Value::Null), fast, table, depth + 1);
                match o.get("type").and_then(Value::as_str).unwrap_or("") {
                    "minecraft:div" => {
                        let r = f("right");
                        if r == 0.0 {
                            0.0
                        } else {
                            (f("left") / r).trunc()
                        }
                    }
                    "minecraft:mul" => f("left") * f("right"),
                    "minecraft:add" | "minecraft:sum" => f("left") + f("right"),
                    "minecraft:sub" => f("left") - f("right"),
                    "minecraft:constant" => f("value"),
                    "minecraft:conditional" => {
                        // The only condition in the pinned data: block/fast_cooking.
                        let condition = o.get("condition").and_then(Value::as_str)
                            == Some("minecraft:block/fast_cooking");
                        if condition && fast {
                            f("on_true")
                        } else {
                            f("on_false")
                        }
                    }
                    _ => 0.0,
                }
            }
            _ => 0.0,
        }
    }
    /// A plain `context_int_provider` value or reference (no context conditions).
    pub fn int_value(&self, v: &Value) -> i32 {
        Self::eval(v, false, &self.ints, 0) as i32
    }
    /// A plain `context_float_provider` value or reference; `default` when absent.
    pub fn float_value(&self, v: &Value, default: f32) -> f32 {
        if v.is_null() {
            default
        } else {
            Self::eval(v, false, &self.floats, 0) as f32
        }
    }
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            ints: HashMap::new(),
            floats: HashMap::new(),
        }
    }
    pub fn burn_time(&self, items: &Items, stack: &ItemStack, fast: bool) -> i32 {
        items
            .get(&stack.item)
            .and_then(|i| i.fuel.as_ref())
            .map_or(0, |(time, _)| Self::eval(time, fast, &self.ints, 0) as i32)
    }
    pub fn speed(&self, items: &Items, stack: &ItemStack, fast: bool) -> f32 {
        items
            .get(&stack.item)
            .and_then(|i| i.fuel.as_ref())
            .map_or(1.0, |(_, speed)| {
                Self::eval(speed, fast, &self.floats, 0) as f32
            })
    }
}
