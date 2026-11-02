//! ClientboundUpdateRecipesPacket: what RecipeManager.finalizeRecipeLoading
//! synchronizes, the recipe property sets (the items each furnace, smithing
//! table slot and brewing stand slot accepts) and the stonecutter's recipes,
//! read from the data pack's recipe files.
use super::data::SyncData;
use horizon_protocol::buf::Writer;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
    sync::{Arc, Mutex},
};

/// RecipeManager.RECIPE_PROPERTY_SETS: each set and the ingredients it takes
/// from a recipe of the given type.
const PROPERTY_SETS: &[(&str, &str, &[&str])] = &[
    ("smithing_addition", "smithing", &["addition"]),
    ("smithing_base", "smithing", &["base"]),
    ("smithing_template", "smithing", &["template"]),
    ("furnace_input", "minecraft:smelting", &["ingredient"]),
    ("blast_furnace_input", "minecraft:blasting", &["ingredient"]),
    ("smoker_input", "minecraft:smoking", &["ingredient"]),
    (
        "campfire_input",
        "minecraft:campfire_cooking",
        &["ingredient"],
    ),
    ("brewing_input", "minecraft:brewing", &["input"]),
    ("brewing_reagent", "minecraft:brewing", &["reagent"]),
];

/// An ingredient as its HolderSet: a tag, or the items listed.
enum Ingredient {
    Tag(String),
    Items(Vec<String>),
}

fn ingredient(v: &Value) -> Option<Ingredient> {
    match v {
        Value::String(s) => Some(match s.strip_prefix('#') {
            Some(tag) => Ingredient::Tag(tag.to_owned()),
            None => Ingredient::Items(vec![s.clone()]),
        }),
        Value::Array(a) => Some(Ingredient::Items(
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
        )),
        // A brewing slot: the item predicate's `item` ingredient.
        Value::Object(o) => ingredient(o.get("item")?),
        _ => None,
    }
}

/// Every recipe file, by recipe id (the registry's order): read through
/// `json_folder`, so a load holding the folder shares one read of it. A file
/// that can't be read or doesn't parse is left out.
pub(super) fn recipes(dir: &Path, out: &mut BTreeMap<String, Value>) {
    let Ok(folder) = horizon_world::json_folder::read(dir) else {
        return;
    };
    for file in &folder.files {
        if let Ok(v) = serde_json::from_slice(file.bytes().unwrap_or_default()) {
            out.insert(format!("minecraft:{}", file.name), v);
        }
    }
}

/// The packs' recipe files over the game's own: the top pack's file for
/// each id (`SimpleJsonResourceReloadListener`).
pub(super) fn overlay_recipes(
    overlay: &horizon_world::loot::PackOverlay,
    out: &mut BTreeMap<String, Value>,
) {
    for (id, files) in overlay.kind("recipe") {
        if let Ok(v) = serde_json::from_slice::<Value>(horizon_world::loot::PackOverlay::top(files))
        {
            out.insert(id.to_owned(), v);
        }
    }
}

/// The network copy's recipes: every recipe file and overlay recipe, each
/// ingredient object — NeoForge's shapes, a brewing slot's item predicate —
/// replaced by the flat item list the crafting copy resolves it to. The
/// crafting copy resolves these shapes where it loads them; this copy reads
/// the files again, and `holder_set` writes any object it meets as an empty
/// list. A recipe that keeps an object the loader cannot parse (a mod's own
/// ingredient kind) is left out: the server does not load it, and an empty
/// ingredient in its place misleads the client.
pub(super) fn network_recipes(
    resources: &Path,
    overlay: &horizon_world::loot::PackOverlay,
) -> (BTreeMap<String, Box<str>>, HashMap<String, i32>) {
    let mut all = BTreeMap::new();
    recipes(&resources.join("data/minecraft/recipe"), &mut all);
    overlay_recipes(overlay, &mut all);
    // The data's recipe filters, as the server's own tables apply them
    // (#5910), once, before the ingredient normalization, so a swapped
    // name resolves and every copy — the book's displays, the property
    // sets — agrees on the swapped recipe.
    let filters = horizon_items::recipe_filters::RecipeFilters::load(resources, overlay);
    all.retain(|id, r| filters.apply(id, r));
    match horizon_client::items::Items::catalog(resources, overlay) {
        Ok(items) => all.retain(|_, r| normalize_ingredients(r, &items)),
        Err(e) => log::warn!("recipes: NeoForge's ingredient shapes stay unresolved: {e}"),
    }
    // RecipeManager.unpackRecipeInfo's group numbers, while the files are
    // still parsed.
    let mut groups = HashMap::new();
    for r in all.values() {
        if let Some(g) = r["group"].as_str().filter(|g| !g.is_empty()) {
            let next = groups.len() as i32;
            groups.entry(g.to_owned()).or_insert(next);
        }
    }
    // Each file as its compact JSON text: a book entry parses the one it
    // needs, and the parsed files would keep about 5 MB alive for the
    // server's whole run.
    let all = all
        .into_iter()
        .map(|(id, r)| (id, r.to_string().into_boxed_str()))
        .collect();
    (all, groups)
}

/// An ingredient object's flat item list: the shape NeoForge's standard
/// kinds resolve to, or a brewing predicate's `item`. `None`: the object is
/// a kind the loader does not know, a mod's own.
fn flat(ing: &Value, items: &horizon_client::items::Items) -> Option<Value> {
    if !ing.is_object() {
        return None;
    }
    let parsed = horizon_client::crafting::Ingredient::parse(ing, items).or_else(|| {
        // A brewing slot's item predicate: its `ingredient`, as the
        // property sets have always read it.
        ing.get("item")
            .and_then(|v| horizon_client::crafting::Ingredient::parse(v, items))
    })?;
    Some(Value::Array(
        parsed.1.iter().map(|i| Value::String(i.clone())).collect(),
    ))
}

/// Replace each of a recipe's ingredient objects with its flat item list,
/// so `holder_set` writes the items instead of an empty list. `false`: the
/// recipe keeps an object the loader cannot parse.
fn normalize_ingredients(r: &mut Value, items: &horizon_client::items::Items) -> bool {
    let known = r["type"]
        .as_str()
        .is_some_and(|t| t.starts_with("minecraft:"));
    if !known {
        return true;
    }
    let mut parsed = true;
    // The shaped recipe's key: a map of symbols to ingredients.
    if let Some(map) = r.get_mut("key").and_then(|k| k.as_object_mut()) {
        for ing in map.values_mut() {
            if let Some(replacement) = flat(ing, items) {
                *ing = replacement;
            } else if ing.is_object() {
                parsed = false;
            }
        }
    }
    // The list and single-ingredient keys, the smithing trio, the brewing
    // pair.
    for key in [
        "ingredients",
        "ingredient",
        "base",
        "addition",
        "template",
        "input",
        "reagent",
    ] {
        let Some(ing) = r.get_mut(key) else { continue };
        if let Some(replacement) = flat(ing, items) {
            *ing = replacement;
            continue;
        }
        if let Value::Array(a) = ing {
            for entry in a.iter_mut() {
                if let Some(replacement) = flat(entry, items) {
                    *entry = replacement;
                } else if entry.is_object() {
                    parsed = false;
                }
            }
        } else if ing.is_object() {
            parsed = false;
        }
    }
    parsed
}

/// The packet's payload for the network copy's recipes (`network_recipes`).
pub fn update_recipes(all: &BTreeMap<String, Box<str>>, sync: &SyncData) -> Vec<u8> {
    // The copy is already filtered and swapped (`network_recipes`); the
    // filters never run twice, so one swap cannot feed a later one again.
    // Parsing every recipe takes about a millisecond.
    let chosen: Vec<Value> = all
        .values()
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .collect();
    let tags: BTreeMap<&str, &[i32]> = sync
        .tags
        .registries
        .iter()
        .filter(|r| r.registry == "minecraft:item")
        .flat_map(|r| r.tags.iter())
        .map(|t| (t.name.trim_start_matches("minecraft:"), t.ids.as_slice()))
        .collect();
    // An item the registries do not know is left out, not sent as id 0.
    let item = |name: &str| sync.id("minecraft:item", name);
    let items = |i: &Ingredient| -> Vec<i32> {
        match i {
            Ingredient::Tag(t) => tags
                .get(t.trim_start_matches("minecraft:"))
                .map(|ids| ids.to_vec())
                .unwrap_or_default(),
            Ingredient::Items(list) => list.iter().filter_map(|n| item(n)).collect(),
        }
    };
    let mut w = Writer::new();
    w.varint(PROPERTY_SETS.len() as i32);
    for (key, kind, fields) in PROPERTY_SETS {
        let mut set = BTreeSet::new();
        for recipe in &chosen {
            let t = recipe["type"].as_str().unwrap_or("");
            let matches = if *kind == "smithing" {
                t.starts_with("minecraft:smithing_")
            } else {
                t == *kind
            };
            if matches {
                for field in *fields {
                    if let Some(i) = ingredient(&recipe[*field]) {
                        set.extend(items(&i));
                    }
                }
            }
        }
        w.string(&format!("minecraft:{key}"));
        w.varint(set.len() as i32);
        for id in set {
            w.varint(id);
        }
    }
    // SelectableRecipe.SingleInputSet: each stonecutting recipe's input and
    // its result as an ItemStackSlotDisplay.
    let stonecutting: Vec<&Value> = chosen
        .iter()
        .filter(|r| r["type"] == "minecraft:stonecutting")
        .collect();
    let item_stack = sync
        .id("minecraft:slot_display", "minecraft:item_stack")
        .unwrap_or(0);
    w.varint(stonecutting.len() as i32);
    for r in stonecutting {
        match ingredient(&r["ingredient"]) {
            Some(Ingredient::Tag(t)) => {
                w.varint(0);
                w.string(&t);
            }
            Some(i) => {
                let ids = items(&i);
                w.varint(ids.len() as i32 + 1);
                for id in ids {
                    w.varint(id);
                }
            }
            None => w.varint(1),
        }
        w.varint(item_stack);
        w.varint(
            item(horizon_world::loot::recipe_result_id(&r["result"]).unwrap_or("minecraft:air"))
                .unwrap_or(0),
        );
        w.varint(r["result"]["count"].as_i64().unwrap_or(1) as i32);
        w.varint(0); // no components added
        w.varint(0); // nor removed
    }
    w.data
}

/// The Horizon channel a server sends its clients the recipes its native mods
/// asked for (`native_datapack_sync`), NeoForge's
/// `OnDatapackSyncEvent.sendRecipes`. Horizon clients only.
pub const CONTENT_CHANNEL: &str = "horizon:recipe_content";
/// `ClientboundCustomPayloadPacket`'s cap on a payload, less room for the
/// envelope.
const CONTENT_MAX: usize = 1_048_576 - 1024;

/// One payload of the recipe content: some recipes as `(id, json)`, and on the
/// last one the recipe types they replace on the client (a type the server has
/// no recipe of still empties the client's).
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContentPart {
    pub recipes: Vec<(String, Value)>,
    pub last: Option<Vec<String>>,
}

/// The payloads for `types`' recipes among `sources` (`Recipes::sources`,
/// vanilla's ids bare), ids made whole (`minecraft:<name>`). With no type
/// asked for: nothing, or with `reset` (this server sent recipes before) one
/// last part with no types, which puts its clients back on their own. A
/// recipe too big for a payload of its own is left out.
pub fn encode_content(types: &[String], sources: &[(String, Value)], reset: bool) -> Vec<Vec<u8>> {
    // The last part carries the types: they count against its cap too.
    let types_len = serde_json::to_string(types).map_or(0, |t| t.len()) + 32;
    if types_len > CONTENT_MAX {
        log::warn!("Horizon: too many recipe types to send to clients");
        return Vec::new();
    }
    if types.is_empty() && !reset {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut part: Vec<(String, Value)> = Vec::new();
    let mut size = 0;
    for (name, v) in sources {
        if !types.iter().any(|t| v["type"].as_str() == Some(t)) {
            continue;
        }
        let id = if name.contains(':') {
            name.clone()
        } else {
            format!("minecraft:{name}")
        };
        let len = id.len() + v.to_string().len() + 8;
        if len > CONTENT_MAX {
            log::warn!("Horizon: recipe {id} is too big to send to clients");
            continue;
        }
        if size + len > CONTENT_MAX {
            let recipes = std::mem::take(&mut part);
            out.push(
                serde_json::to_vec(&ContentPart {
                    recipes,
                    last: None,
                })
                .unwrap_or_default(),
            );
            size = 0;
        }
        size += len;
        part.push((id, v.clone()));
    }
    if size + types_len > CONTENT_MAX {
        let recipes = std::mem::take(&mut part);
        out.push(
            serde_json::to_vec(&ContentPart {
                recipes,
                last: None,
            })
            .unwrap_or_default(),
        );
    }
    let last = ContentPart {
        recipes: part,
        last: Some(types.to_vec()),
    };
    out.push(serde_json::to_vec(&last).unwrap_or_default());
    out
}

/// What the server sends its Horizon clients now (`set_content`).
static CONTENT: Mutex<Option<Arc<Vec<Vec<u8>>>>> = Mutex::new(None);

/// The recipe content the server sends from now on, at a join and after a
/// `/reload` (one server per process, as `lan::sync_data`).
pub fn set_content(payloads: Vec<Vec<u8>>) {
    if !payloads.is_empty() {
        SENT.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    *CONTENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(payloads));
}

/// Whether this server sent its clients recipes before: a later load that
/// asks for none must put them back on their own (`encode_content`'s reset).
static SENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn sent_before() -> bool {
    SENT.load(std::sync::atomic::Ordering::Relaxed)
}

/// The payloads to send a Horizon client; none when no mod asked.
pub fn content() -> Arc<Vec<Vec<u8>>> {
    CONTENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

#[cfg(test)]
mod content_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_keeps_the_asked_types_and_ends_with_them() {
        let sources = vec![
            ("furnace".to_owned(), json!({"type": "minecraft:smelting"})),
            (
                "stick".to_owned(),
                json!({"type": "minecraft:crafting_shaped"}),
            ),
            ("mymod:crush".to_owned(), json!({"type": "mymod:crusher"})),
        ];
        assert!(encode_content(&[], &sources, false).is_empty());
        // After a sync that sent recipes, asking for none sends an empty last part.
        let reset = encode_content(&[], &sources, true);
        assert_eq!(reset.len(), 1);
        let part: ContentPart = serde_json::from_slice(&reset[0]).unwrap();
        assert!(part.recipes.is_empty());
        assert_eq!(part.last.as_deref(), Some(&[][..]));
        let types = vec![
            "minecraft:smelting".to_owned(),
            "mymod:crusher".to_owned(),
            "x:none".to_owned(),
        ];
        let parts = encode_content(&types, &sources, false);
        assert_eq!(parts.len(), 1);
        let part: ContentPart = serde_json::from_slice(&parts[0]).unwrap();
        let ids: Vec<&str> = part.recipes.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["minecraft:furnace", "mymod:crush"]);
        assert_eq!(part.last.as_deref(), Some(types.as_slice()));
    }

    #[test]
    fn content_splits_under_the_payload_cap() {
        let big = "x".repeat(300_000);
        let sources: Vec<_> = (0..10)
            .map(|i| (format!("m:r{i}"), json!({"type": "m:t", "pad": big})))
            .collect();
        let parts = encode_content(&["m:t".to_owned()], &sources, false);
        assert!(parts.len() >= 3);
        let mut n = 0;
        for (i, p) in parts.iter().enumerate() {
            assert!(p.len() <= 1_048_576);
            let part: ContentPart = serde_json::from_slice(p).unwrap();
            assert_eq!(part.last.is_some(), i + 1 == parts.len());
            n += part.recipes.len();
        }
        assert_eq!(n, 10);
    }

    #[test]
    fn a_long_type_list_keeps_the_last_part_under_the_cap() {
        let types: Vec<String> = (0..5000)
            .map(|i| format!("mod{i}:type_with_a_long_name"))
            .collect();
        let big = "x".repeat(1_000_000);
        let sources = vec![(
            "m:r".to_owned(),
            json!({"type": "mod0:type_with_a_long_name", "pad": big}),
        )];
        let parts = encode_content(&types, &sources, false);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.len() <= 1_048_576));
        let last: ContentPart = serde_json::from_slice(&parts[1]).unwrap();
        assert!(last.recipes.is_empty());
        assert_eq!(last.last.map(|t| t.len()), Some(5000));
    }
}
