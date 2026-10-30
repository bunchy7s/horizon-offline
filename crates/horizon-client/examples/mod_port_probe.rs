//! The Horizon side of a mod port's harness (tools/mod_port_harness.py,
//! docs/modpacks.md): builds the content root for carried-over mods and
//! writes what Horizon makes of every block state, drop, recipe and tag the
//! NeoForge oracle exported.
//!
//! cargo run -p horizon-client --example mod_port_probe -- PREPARED WORK OUT.json [MOD...]
//!
//! WORK is a `horizon-port-pack` work folder: `oracle/facts.json` and
//! `mods/<id>/carry/`. The root is built in `WORK/horizon-root`.
use horizon_client::{
    Result,
    assets::Assets,
    blocks::BlockFacts,
    crafting::{Recipe, Recipes},
    items::{ItemStack, Items},
    mod_content::{self, ContentMod},
    stations::StationRecipes,
};
use horizon_core::{geometry::Axis, random::LegacyRandom};
use horizon_world::chunk::BlockState;
use horizon_world::loot::PackOverlay;
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, fs, path::Path};

fn boxes(list: impl IntoIterator<Item = [f64; 6]>) -> Value {
    let mut v: Vec<[f64; 6]> = list.into_iter().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    json!(v)
}

fn sound(s: &horizon_client::blocks::Sounds) -> Value {
    json!({"volume": s.volume, "pitch": s.pitch, "break": &*s.break_sound, "step": &*s.step,
        "place": &*s.place, "hit": &*s.hit, "fall": &*s.fall})
}

fn tool(name: &str) -> Option<ItemStack> {
    match name {
        "hand" => None,
        "silk_touch_pickaxe" => {
            let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
            pick.enchant("minecraft:silk_touch", 1);
            Some(pick)
        }
        other => Some(ItemStack::new(&format!("minecraft:{other}"), 1)),
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [base, work, out, wanted @ ..] = args.as_slice() else {
        return Err("usage: mod_port_probe PREPARED WORK OUT.json [MOD...]".into());
    };
    let (base, work) = (Path::new(base), Path::new(work));
    let oracle: Value = serde_json::from_slice(&fs::read(work.join("oracle/facts.json"))?)?;
    let mut mods = Vec::new();
    for entry in fs::read_dir(work.join("mods"))?.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        let dir = entry.path().join("carry");
        if !(wanted.is_empty() || wanted.contains(&id)) || !dir.is_dir() {
            continue;
        }
        let package = horizon_mods::Package::read(&dir)?;
        let content = horizon_mods::Content::read(&dir, &package.manifest)?;
        mods.push(ContentMod {
            id: package.manifest.id.clone(),
            content,
        });
    }
    mods.sort_by(|a, b| a.id.cmp(&b.id));
    let root = work.join("horizon-root");
    let began = std::time::Instant::now();
    let step = |what: &str| {
        let what = what.to_owned();
        move |e: Box<dyn std::error::Error>| format!("{what}: {e}")
    };
    mod_content::build(base, &mods, &root).map_err(step("content root"))?;
    let built = began.elapsed().as_secs_f64();
    let assets = Assets::load(&root).map_err(step("assets"))?;
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).map_err(step("block facts"))?;
    let items = Items::load(&root, registry, &facts, &PackOverlay::NONE).map_err(step("items"))?;
    let recipes = Recipes::load(&root, &items, &PackOverlay::NONE).map_err(step("recipes"))?;
    let stations = StationRecipes::load(&root, &items, &PackOverlay::NONE);
    let collisions =
        horizon_client::collisions::load(&root, registry).map_err(step("collisions"))?;
    let loaded = began.elapsed().as_secs_f64() - built;
    let ours = |id: &str| {
        id.split_once(':').is_some_and(|(ns, _)| {
            mods.iter()
                .any(|m| m.content.namespaces.iter().any(|n| n == ns))
        })
    };
    let tools: Vec<String> = serde_json::from_value(oracle["tools"].clone())?;

    let mut blocks = Map::new();
    for (name, block) in oracle["blocks"].as_object().ok_or("oracle: no blocks")? {
        if !ours(name) {
            continue;
        }
        let Some(default) = registry.default_state(name) else {
            blocks.insert(name.clone(), Value::Null);
            continue;
        };
        let mut states = Map::new();
        for (key, o) in block["states"].as_object().into_iter().flatten() {
            let properties: BTreeMap<String, String> = key
                .split(',')
                .filter(|p| !p.is_empty())
                .filter_map(|p| p.split_once('='))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
            let descriptor = BlockState {
                name: name.clone(),
                properties,
            };
            let Some(id) = registry.id(&descriptor) else {
                states.insert(key.clone(), Value::Null);
                continue;
            };
            let f = facts.get(id);
            let collision = collisions
                .cached_shapes()
                .cached_shape(id)
                .ok()
                .flatten()
                .map(|s| {
                    boxes(s.boxes().iter().map(|b| {
                        [
                            b.min(Axis::X),
                            b.min(Axis::Y),
                            b.min(Axis::Z),
                            b.max(Axis::X),
                            b.max(Axis::Y),
                            b.max(Axis::Z),
                        ]
                    }))
                });
            let mut row = json!({
                "hardness": f.hardness,
                "explosion_resistance": f.explosion_resistance,
                "emission": f.emission,
                "requires_tool": f.requires_tool,
                "friction": f.friction,
                "map_color_id": f.map_color,
                "can_occlude": f.can_occlude,
                "sound": sound(&f.sounds),
                "outline": boxes(f.outline.iter().copied()),
                "collision": collision,
                "classes": f.classes.iter().map(|c| &**c).collect::<Vec<_>>(),
            });
            if o.get("drops").is_some() {
                let mut drops = Map::new();
                for t in &tools {
                    let mut random = LegacyRandom::new(0);
                    let held = tool(t);
                    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
                    for s in items.block_drops(registry, id, held.as_ref(), &mut random) {
                        *counts.entry(s.item.to_string()).or_default() += s.count;
                    }
                    drops.insert(t.clone(), json!(counts));
                }
                row["drops"] = Value::Object(drops);
            }
            states.insert(key.clone(), row);
        }
        let item = facts.get(default).item.to_string();
        blocks.insert(
            name.clone(),
            json!({"states": states, "item": item, "item_known": items.items.contains_key(&*item)}),
        );
    }

    // Vanilla's sounds, by block: the harness maps the oracle's version's
    // sound events to this one's through them.
    let mut vanilla_sounds = Map::new();
    for name in oracle["blocks"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
    {
        if let Some(id) = name
            .starts_with("minecraft:")
            .then(|| registry.default_state(name))
            .flatten()
        {
            vanilla_sounds.insert(name.clone(), sound(&facts.get(id).sounds));
        }
    }

    let mut item_rows = Map::new();
    for name in oracle["items"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
    {
        item_rows.insert(
            name.clone(),
            match items.items.get(name.as_str()) {
                Some(i) => json!({"max_stack_size": i.max_stack, "block": i.block.map(|b| b.0)}),
                None => Value::Null,
            },
        );
    }

    let sorted = |list: &[String]| {
        let mut v = list.to_vec();
        v.sort();
        v.dedup();
        v
    };
    let mut recipe_rows = Map::new();
    for (name, recipe) in &recipes.crafting {
        if !ours(name) {
            continue;
        }
        let row = match recipe {
            Recipe::Shaped { cells, result, .. } => json!({
                "result": result.item.as_deref(), "count": result.count,
                "ingredients": cells.iter().flatten().map(|i| sorted(&i.1)).collect::<Vec<_>>()}),
            Recipe::Shapeless {
                ingredients,
                result,
            } => json!({
                "result": result.item.as_deref(), "count": result.count,
                "ingredients": ingredients.iter().map(|i| sorted(&i.1)).collect::<Vec<_>>()}),
            Recipe::Special(_) => json!({"special": true}),
        };
        recipe_rows.insert(name.clone(), row);
    }
    for c in recipes.cooking.iter().filter(|c| ours(&c.id)) {
        recipe_rows.insert(
            c.id.clone(),
            json!({"result": &*c.result.item, "count": c.result.count,
                "ingredients": [sorted(&c.ingredient.1)]}),
        );
    }
    for s in stations.stonecutting.iter().filter(|s| ours(&s.id)) {
        recipe_rows.insert(
            s.id.clone(),
            json!({"result": &*s.result.item, "count": s.result.count,
                "ingredients": [sorted(&s.input.1)]}),
        );
    }

    let mut block_tags = Map::new();
    for (tag, members) in &items.block_tags {
        let mut ids: Vec<&String> = members.iter().filter(|m| ours(m)).collect();
        if !ids.is_empty() {
            ids.sort();
            block_tags.insert(tag.clone(), json!(ids));
        }
    }

    let report = json!({
        "states": registry.len(),
        "build_seconds": built,
        "load_seconds": loaded,
        "blocks": blocks,
        "items": item_rows,
        "recipes": recipe_rows,
        "block_tags": block_tags,
        "vanilla_sounds": vanilla_sounds,
    });
    fs::write(out, serde_json::to_vec(&report)?)?;
    println!(
        "{out}: {} blocks, {} items, {} recipes; root built in {built:.2} s, loaded in {loaded:.2} s",
        blocks.len(),
        item_rows.len(),
        recipe_rows.len()
    );
    Ok(())
}
