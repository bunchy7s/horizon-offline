//! Recipe filters: data files that remove recipes, or swap one of their
//! inputs or outputs, by what the recipes are rather than by id.
//!
//! A file `data/<ns>/recipe_filter/<path>.json` holds `{"filters": [...]}`.
//! Each filter has match keys, all of which must hold (none matches every
//! recipe), and exactly one action:
//! - `id`: the recipe id, `*` matching any run of characters
//!   (`minecraft:*_planks`);
//! - `type`: the recipe's `type`, with the same `*`;
//! - `input`: an item id or `#tag` among the recipe's inputs;
//! - `output`: an item id among its outputs (under `result`, `results`,
//!   `output` or `outputs`);
//! - `remove: true`, or `replace_input` / `replace_output`
//!   `{"from": ..., "to": ...}`, which swaps every input (or output) that is
//!   `from` for `to`.
//!
//! Filters run on each recipe after the data packs are merged and its load
//! conditions held, so they see the final set and reach every recipe type,
//! mods' own included: they work on the recipe's JSON. Files apply in id
//! order, each file's filters in order. A pack's file replaces a file of the
//! same id, as any data file does. A filter with an unknown key or no action
//! is skipped with a warning, so a typo never matches every recipe.
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::path::Path;

/// The keys under which a recipe lists what it makes.
const OUTPUT_KEYS: [&str; 4] = ["result", "results", "output", "outputs"];
/// Keys whose strings are never items: a type, a group, a category, the load
/// conditions.
const NOT_ITEMS: [&str; 4] = ["type", "group", "category", "neoforge:conditions"];

#[derive(Clone, Debug, PartialEq)]
enum Action {
    Remove,
    ReplaceInput(String, String),
    ReplaceOutput(String, String),
}

#[derive(Clone, Debug, PartialEq)]
struct Filter {
    id: Option<String>,
    kind: Option<String>,
    input: Option<String>,
    output: Option<String>,
    action: Action,
}

/// Every recipe filter of a load, in the order they apply.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipeFilters {
    filters: Vec<Filter>,
}

impl RecipeFilters {
    /// The filters of the content root's mods (`data/<ns>/recipe_filter/`)
    /// with the world's packs' files over them.
    pub fn load(root: &Path, overlay: &PackOverlay) -> Self {
        let mut files: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
        for ns in crate::content_namespaces::namespaces(root) {
            let dir = root.join("data").join(&ns).join("recipe_filter");
            read_dir(&dir, &format!("{ns}:"), &mut files);
        }
        for (id, versions) in overlay.kind("recipe_filter") {
            files.insert(id.to_owned(), PackOverlay::top(versions).to_vec());
        }
        Self::parse(files)
    }

    fn parse(files: std::collections::BTreeMap<String, Vec<u8>>) -> Self {
        let mut filters = Vec::new();
        for (file, bytes) in files {
            let v: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("Horizon: recipe filter {file} not loaded: {e}");
                    continue;
                }
            };
            for (i, f) in v["filters"].as_array().into_iter().flatten().enumerate() {
                match Filter::parse(f) {
                    Ok(f) => filters.push(f),
                    Err(e) => log::warn!("Horizon: recipe filter {file} #{i} skipped: {e}"),
                }
            }
        }
        Self { filters }
    }

    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// Runs the filters over recipe `id` (`ns:path`): false when one removes
    /// it, else `recipe` with every swap applied.
    pub fn apply(&self, id: &str, recipe: &mut Value) -> bool {
        for f in &self.filters {
            if !f.matches(id, recipe) {
                continue;
            }
            match &f.action {
                Action::Remove => return false,
                Action::ReplaceInput(from, to) => replace(recipe, from, to, false, false),
                Action::ReplaceOutput(from, to) => replace(recipe, from, to, false, true),
            }
        }
        true
    }
}

fn read_dir(dir: &Path, prefix: &str, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            read_dir(&path, &format!("{prefix}{name}/"), out);
        } else if let Some(stem) = name.strip_suffix(".json")
            && let Ok(bytes) = std::fs::read(&path)
        {
            out.insert(format!("{prefix}{stem}"), bytes);
        }
    }
}

impl Filter {
    fn parse(v: &Value) -> Result<Self, String> {
        let obj = v.as_object().ok_or("not an object")?;
        let text = |k: &str| -> Result<Option<String>, String> {
            match obj.get(k) {
                None => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(format!("`{k}` is not a string")),
            }
        };
        let swap = |k: &str| -> Result<Option<(String, String)>, String> {
            let Some(s) = obj.get(k) else {
                return Ok(None);
            };
            match (s["from"].as_str(), s["to"].as_str()) {
                (Some(from), Some(to)) => Ok(Some((from.to_owned(), to.to_owned()))),
                _ => Err(format!("`{k}` needs `from` and `to` strings")),
            }
        };
        for k in obj.keys() {
            if !matches!(
                k.as_str(),
                "id" | "type" | "input" | "output" | "remove" | "replace_input" | "replace_output"
            ) {
                return Err(format!("unknown key `{k}`"));
            }
        }
        let mut actions = Vec::new();
        match obj.get("remove") {
            None | Some(Value::Bool(false)) => {}
            Some(Value::Bool(true)) => actions.push(Action::Remove),
            Some(_) => return Err("`remove` is not a boolean".into()),
        }
        if let Some((from, to)) = swap("replace_input")? {
            actions.push(Action::ReplaceInput(from, to));
        }
        if let Some((from, to)) = swap("replace_output")? {
            actions.push(Action::ReplaceOutput(from, to));
        }
        let [action] = <[Action; 1]>::try_from(actions)
            .map_err(|_| "needs exactly one of `remove`, `replace_input`, `replace_output`")?;
        Ok(Self {
            id: text("id")?,
            kind: text("type")?,
            input: text("input")?,
            output: text("output")?,
            action,
        })
    }

    fn matches(&self, id: &str, recipe: &Value) -> bool {
        self.id.as_deref().is_none_or(|p| glob(p, id))
            && self
                .kind
                .as_deref()
                .is_none_or(|p| glob(p, recipe["type"].as_str().unwrap_or("")))
            && self
                .input
                .as_deref()
                .is_none_or(|i| contains(recipe, i, false, false))
            && self
                .output
                .as_deref()
                .is_none_or(|o| contains(recipe, o, false, true))
    }
}

/// `pattern` with `*` as any run of characters, against `s`.
fn glob(pattern: &str, s: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = s.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// The item a string at `key` names: a `tag` key's value is a tag without
/// its `#` (`{"tag": "c:ingots"}`).
fn named(key: Option<&str>, s: &str) -> String {
    if key == Some("tag") {
        format!("#{s}")
    } else {
        s.to_owned()
    }
}

/// Whether `item` is among `v`'s inputs (`outputs` false) or outputs
/// (`outputs` true). `inside` says the walk is already under an output key.
fn contains(v: &Value, item: &str, inside: bool, outputs: bool) -> bool {
    fn walk(v: &Value, key: Option<&str>, item: &str, inside: bool, outputs: bool) -> bool {
        match v {
            Value::String(s) => inside == outputs && named(key, s) == item,
            Value::Array(a) => a.iter().any(|v| walk(v, key, item, inside, outputs)),
            Value::Object(o) => o.iter().any(|(k, v)| {
                !NOT_ITEMS.contains(&k.as_str())
                    && walk(
                        v,
                        Some(k),
                        item,
                        inside || OUTPUT_KEYS.contains(&k.as_str()),
                        outputs,
                    )
            }),
            _ => false,
        }
    }
    walk(v, None, item, inside, outputs)
}

/// Swaps every input (`outputs` false) or output that is `from` for `to`.
fn replace(v: &mut Value, from: &str, to: &str, inside: bool, outputs: bool) {
    fn walk(v: &mut Value, key: Option<&str>, from: &str, to: &str, inside: bool, outputs: bool) {
        match v {
            Value::String(s) => {
                if inside == outputs && named(key, s) == from {
                    // A `tag` key holds the tag without its `#`.
                    *s = match key {
                        Some("tag") => to.trim_start_matches('#').to_owned(),
                        _ => to.to_owned(),
                    };
                }
            }
            Value::Array(a) => a
                .iter_mut()
                .for_each(|v| walk(v, key, from, to, inside, outputs)),
            Value::Object(o) => {
                for (k, v) in o.iter_mut() {
                    if !NOT_ITEMS.contains(&k.as_str()) {
                        let inside = inside || OUTPUT_KEYS.contains(&k.as_str());
                        walk(v, Some(k), from, to, inside, outputs);
                    }
                }
            }
            _ => {}
        }
    }
    walk(v, None, from, to, inside, outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn filters(v: Value) -> RecipeFilters {
        RecipeFilters::parse([("test:f".to_owned(), v.to_string().into_bytes())].into())
    }

    #[test]
    fn globs_match_runs() {
        assert!(glob("minecraft:*_planks", "minecraft:oak_planks"));
        assert!(glob("*", "anything"));
        assert!(glob("a*b*c", "a-x-b-y-c"));
        assert!(!glob("a*b*c", "a-x-c"));
        assert!(!glob("minecraft:oak", "minecraft:oak_planks"));
        assert!(glob("*:oak_*", "mod:oak_log"));
        assert!(!glob("ab*ba", "aba"));
    }

    #[test]
    fn remove_by_id_type_and_items() {
        let planks = json!({"type": "minecraft:crafting_shapeless",
            "ingredients": ["#minecraft:oak_logs"], "result": {"id": "minecraft:oak_planks", "count": 4}});
        let f = filters(json!({"filters": [{"id": "minecraft:*_planks", "remove": true}]}));
        assert!(!f.apply("minecraft:oak_planks", &mut planks.clone()));
        assert!(f.apply("minecraft:stick", &mut planks.clone()));
        // The input and output keys look on their own side only.
        let f = filters(json!({"filters": [{"output": "#minecraft:oak_logs", "remove": true}]}));
        assert!(f.apply("x:y", &mut planks.clone()));
        let f = filters(json!({"filters": [{"input": "#minecraft:oak_logs",
            "type": "minecraft:crafting_*", "remove": true}]}));
        assert!(!f.apply("x:y", &mut planks.clone()));
        let f = filters(
            json!({"filters": [{"output": "minecraft:oak_planks", "type": "minecraft:smelting", "remove": true}]}),
        );
        assert!(f.apply("x:y", &mut planks.clone()));
    }

    #[test]
    fn swaps_reach_mod_types_and_tag_objects() {
        let mut machine = json!({"type": "gtceu:macerator",
            "inputs": {"item": [{"tag": "c:ingots/iron"}, "minecraft:iron_ingot"]},
            "outputs": {"item": ["minecraft:iron_ingot"]}});
        let f = filters(json!({"filters": [
            {"replace_input": {"from": "#c:ingots/iron", "to": "#c:ingots/gold"}},
            {"type": "gtceu:*", "replace_output": {"from": "minecraft:iron_ingot", "to": "gtceu:iron_dust"}}
        ]}));
        assert!(f.apply("gtceu:macerator/iron", &mut machine));
        assert_eq!(machine["inputs"]["item"][0]["tag"], "c:ingots/gold");
        // Only outputs were swapped for the item: the input stays.
        assert_eq!(machine["inputs"]["item"][1], "minecraft:iron_ingot");
        assert_eq!(machine["outputs"]["item"][0], "gtceu:iron_dust");
        // `type` is never an item.
        assert_eq!(machine["type"], "gtceu:macerator");
    }

    #[test]
    fn bad_filters_are_skipped_not_widened() {
        let f = filters(json!({"filters": [
            {"Id": "minecraft:stick", "remove": true},
            {"id": "minecraft:stick"},
            {"remove": true, "replace_input": {"from": "a:b", "to": "c:d"}},
            {"replace_input": {"from": "a:b"}}
        ]}));
        assert!(f.is_empty());
        assert!(RecipeFilters::parse([("t:f".to_owned(), b"{nope".to_vec())].into()).is_empty());
    }

    #[test]
    fn packs_replace_a_mods_file_and_files_apply_in_id_order() {
        let root =
            std::env::temp_dir().join(format!("horizon-recipe-filters-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("data/mymod/recipe_filter");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            root.join(crate::content_namespaces::MARKER),
            json!(["mymod"]).to_string(),
        )
        .unwrap();
        let remove_sticks = json!({"filters": [{"id": "minecraft:stick", "remove": true}]});
        std::fs::write(dir.join("a.json"), remove_sticks.to_string()).unwrap();
        let swap = json!({"filters": [{"replace_output": {"from": "minecraft:stone", "to": "minecraft:dirt"}}]});
        std::fs::write(dir.join("b.json"), swap.to_string()).unwrap();
        let mut stone = json!({"type": "minecraft:smelting", "ingredient": "minecraft:cobblestone", "result": {"id": "minecraft:stone"}});
        let f = RecipeFilters::load(&root, &PackOverlay::NONE);
        assert!(!f.apply("minecraft:stick", &mut json!({})));
        assert!(f.apply("minecraft:stone", &mut stone));
        assert_eq!(stone["result"]["id"], "minecraft:dirt");
        // A pack's file of the same id replaces the mod's: sticks stay.
        let files = vec![(
            "recipe_filter/mymod:a".to_owned(),
            vec![json!({"filters": []}).to_string().into_bytes()],
        )];
        let f = RecipeFilters::load(&root, &PackOverlay { files: &files });
        assert!(f.apply("minecraft:stick", &mut json!({})));
        let _ = std::fs::remove_dir_all(&root);
    }
}
