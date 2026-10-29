//! Step 4, carry-over: a jar's `assets/` and `data/` into a folder content
//! mod (API 0.2) for local use. Files are copied, never code; recipes are
//! upgraded from 1.21.1 to 26.3, item models get their `items/` client
//! definitions, and each blockstate gets a vanilla template guessed from its
//! properties until the oracle's facts replace the guess.
use crate::{jar::JarInfo, oracle::Facts};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub copied: usize,
    /// Namespaces besides the mod id its content owns ([`extra_namespaces`]),
    /// listed in the manifest's `namespaces`.
    pub namespaces: Vec<String>,
    /// Files outside what a content mod may hold (other namespaces, vanilla
    /// data other than tags), kept in `outside/` for review.
    pub outside: Vec<String>,
    /// Files of the mod's own content the game would refuse, kept in
    /// `outside/` too, with why: a path vanilla does not read (upper case),
    /// JSON that does not parse.
    pub unreadable: BTreeMap<String, String>,
    pub recipes_upgraded: usize,
    pub loot_tables_upgraded: usize,
    /// Advancements whose criteria were rewritten ([`upgrade_advancement`]).
    pub advancements_upgraded: usize,
    /// Blocks whose jar has no loot table but drop themselves in the oracle:
    /// the mod registers that table in code, and carry-over writes it.
    pub loot_tables_added: usize,
    /// Worldgen and structure files (`data/<id>/worldgen/`, `structure/`).
    pub worldgen: usize,
    /// Recipes left as they were: custom types, NeoForge conditions.
    pub recipes_kept: Vec<String>,
    /// Recipes whose result is one of the mod's own items that isn't declared (its block was
    /// left out), moved to `outside/`: vanilla refuses a recipe for an unknown item, and the
    /// game would craft air.
    pub recipes_unregistered: Vec<String>,
    pub item_definitions_added: usize,
    /// Templates and properties came from the NeoForge oracle's facts, not
    /// from guesses.
    pub oracle: bool,
    /// Declared blocks and their templates.
    pub blocks: BTreeMap<String, String>,
    /// What a declared block's template does not reproduce (oracle only).
    pub differences: BTreeMap<String, Vec<String>>,
    /// Blocks left out, and why.
    pub blocks_unknown: BTreeMap<String, String>,
    pub items: usize,
    /// Items left out, and why (oracle only).
    pub items_unknown: BTreeMap<String, String>,
    /// Convention (`c:`) tags the recipes use, filled in from the oracle.
    pub convention_tags: usize,
    /// Structure, placement and pool element types rewritten as vanilla's,
    /// and those left as they are.
    pub worldgen_types: crate::worldgen::Upgrade,
    /// The loader's convention biome tags the structures name, carried.
    pub convention_biome_tags: usize,
}

/// The pack around a mod being carried.
pub struct Pack<'a> {
    /// The mods ported along, for the conditions of worldgen modifiers.
    pub loaded: &'a dyn Fn(&str) -> bool,
    /// Every mod of the pack by id, ported or not: their namespaces are theirs.
    pub mods: &'a dyn Fn(&str) -> bool,
}

/// Vanilla blocks by their sorted property names.
const TEMPLATES: &[(&str, &str)] = &[
    ("", "minecraft:stone"),
    ("axis", "minecraft:oak_log"),
    ("type,waterlogged", "minecraft:oak_slab"),
    ("facing,half,shape,waterlogged", "minecraft:oak_stairs"),
    ("facing,half,hinge,open,powered", "minecraft:oak_door"),
    (
        "facing,half,open,powered,waterlogged",
        "minecraft:oak_trapdoor",
    ),
    ("east,north,south,waterlogged,west", "minecraft:oak_fence"),
    ("east,north,south,waterlogged,west", "minecraft:glass_pane"),
    (
        "east,north,south,up,waterlogged,west",
        "minecraft:cobblestone_wall",
    ),
    ("facing,in_wall,open,powered", "minecraft:oak_fence_gate"),
    ("face,facing,powered", "minecraft:oak_button"),
    ("powered", "minecraft:oak_pressure_plate"),
    ("facing", "minecraft:carved_pumpkin"),
    ("facing,lit", "minecraft:furnace"),
    ("lit", "minecraft:redstone_lamp"),
    ("waterlogged", "minecraft:chain"),
    ("age", "minecraft:wheat"),
    ("distance,persistent,waterlogged", "minecraft:oak_leaves"),
    ("stage", "minecraft:oak_sapling"),
];

/// A guess without the oracle: the template whose properties include the
/// blockstate's with the fewest left over (a blockstate may leave out what
/// its models ignore, such as `powered` or `waterlogged`), the block's name
/// breaking ties (`_pane`, `_gate`, `_trapdoor`).
pub fn template(name: &str, properties: &BTreeSet<String>) -> Option<&'static str> {
    let fits = TEMPLATES.iter().filter_map(|(k, t)| {
        let have: BTreeSet<&str> = k.split(',').filter(|p| !p.is_empty()).collect();
        properties
            .iter()
            .all(|p| have.contains(p.as_str()))
            .then(|| (have.len() - properties.len(), *t))
    });
    let hint = |t: &str| {
        let kind = t.rsplit('_').next().unwrap_or(t);
        !name.ends_with(&format!("_{kind}"))
    };
    fits.min_by_key(|&(extra, t)| (extra, hint(t)))
        .map(|(_, t)| t)
}

/// [`template`] for a blockstate file. A wall's sides are `none`, `low` or
/// `tall`; sides and `up` that are `true` or `false` are a vine's (Chipped's
/// vines), which the names alone take for a wall.
pub fn block_template(name: &str, blockstate: &Value) -> Option<&'static str> {
    let t = template(name, &properties(blockstate))?;
    let boolean = ["north", "east", "south", "west"]
        .iter()
        .flat_map(|side| values(blockstate, side))
        .any(|v| v == "true" || v == "false");
    Some(if t == "minecraft:cobblestone_wall" && boolean {
        "minecraft:vine"
    } else {
        t
    })
}

/// The values a blockstate file tests a property for.
fn values(blockstate: &Value, property: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for key in blockstate["variants"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
    {
        for pair in key.split(',') {
            if let Some((k, v)) = pair.split_once('=')
                && k == property
            {
                out.insert(v.to_owned());
            }
        }
    }
    for part in blockstate["multipart"].as_array().into_iter().flatten() {
        let when = &part["when"];
        let mut conditions: Vec<&Value> = vec![when];
        for or in ["OR", "AND"] {
            conditions.extend(when[or].as_array().into_iter().flatten());
        }
        for v in conditions.iter().filter_map(|c| c[property].as_str()) {
            out.extend(v.trim_start_matches('!').split('|').map(str::to_owned));
        }
    }
    out
}

/// The property names a blockstate file uses.
pub fn properties(blockstate: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |pairs: &str| {
        for pair in pairs.split(',').filter(|p| !p.is_empty()) {
            out.insert(pair.split('=').next().unwrap_or(pair).to_owned());
        }
    };
    for key in blockstate["variants"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
    {
        add(key);
    }
    for part in blockstate["multipart"].as_array().into_iter().flatten() {
        let when = &part["when"];
        let mut conditions: Vec<&Value> = vec![when];
        for or in ["OR", "AND"] {
            conditions.extend(when[or].as_array().into_iter().flatten());
        }
        for c in conditions {
            for (k, _) in c.as_object().into_iter().flatten() {
                if k != "OR" && k != "AND" {
                    out.insert(k.clone());
                }
            }
        }
    }
    out
}

/// A 1.21.1 ingredient (`{"item": id}`, `{"tag": t}` or a list of those) as
/// 26.3's (`"id"`, `"#t"`, `["id", ...]`); `None` for what has no 26.3 form
/// (custom ingredient types, tags inside lists).
pub fn ingredient(v: &Value) -> Option<Value> {
    match v {
        Value::String(_) => Some(v.clone()),
        Value::Object(o) if o.len() == 1 => {
            if let Some(Value::String(id)) = o.get("item") {
                Some(json!(id))
            } else if let Some(Value::String(tag)) = o.get("tag") {
                Some(json!(format!("#{tag}")))
            } else {
                None
            }
        }
        Value::Array(list) => {
            let ids: Option<Vec<Value>> = list
                .iter()
                .map(|x| {
                    x.get("item")
                        .filter(|_| x.as_object().is_some_and(|o| o.len() == 1))
                        .cloned()
                })
                .collect();
            match ids? {
                ids if ids.len() == 1 => ids.into_iter().next(),
                ids => Some(Value::Array(ids)),
            }
        }
        _ => None,
    }
}

/// A vanilla recipe upgraded in place; `false` (unchanged) when it is not
/// a vanilla type or one of its ingredients has no 26.3 form.
pub fn upgrade_recipe(recipe: &mut Value) -> bool {
    let kind = recipe["type"].as_str().unwrap_or("");
    if !kind.starts_with("minecraft:") || recipe.get("neoforge:conditions").is_some() {
        return false;
    }
    let mut out = recipe.clone();
    if let Some(key) = out.get_mut("key").and_then(Value::as_object_mut) {
        for v in key.values_mut() {
            let Some(new) = ingredient(v) else {
                return false;
            };
            *v = new;
        }
    }
    if let Some(list) = out.get_mut("ingredients").and_then(Value::as_array_mut) {
        for v in list.iter_mut() {
            let Some(new) = ingredient(v) else {
                return false;
            };
            *v = new;
        }
    }
    for field in ["ingredient", "base", "addition", "template"] {
        if let Some(v) = out.get_mut(field) {
            let Some(new) = ingredient(v) else {
                return false;
            };
            *v = new;
        }
    }
    *recipe = out;
    true
}

/// Block and item ids vanilla renamed after 1.21.1, as its datafixers
/// rename them in worlds (DataVersion 4541: the chain became the iron chain
/// when copper chains came in). A data file naming the old id fails to load
/// in 26.3: a recipe, a loot entry, a processor's block match.
const RENAMED_IDS: [(&str, &str); 1] = [("minecraft:chain", "minecraft:iron_chain")];

/// Every string of a data file that is a renamed id ([`RENAMED_IDS`]) set to
/// the new one, in place; whether any was.
pub fn renamed_ids(v: &mut Value) -> bool {
    match v {
        Value::String(s) => match RENAMED_IDS.iter().find(|(old, _)| s == old) {
            Some((_, new)) => {
                *s = (*new).to_owned();
                true
            }
            None => false,
        },
        Value::Array(a) => a.iter_mut().fold(false, |c, x| renamed_ids(x) | c),
        Value::Object(o) => o.values_mut().fold(false, |c, x| renamed_ids(x) | c),
        _ => false,
    }
}

/// A 1.21.1 advancement in 26.3's shape, in place: an `enter_block`
/// criterion names its block as `blocks`, which 26.3 reads as a block list.
/// 26.3 skips the old `block` field, so the criterion matched any block the
/// player entered (Regions Unexplored's "Downer" on the first dive).
pub fn upgrade_advancement(v: &mut Value) -> bool {
    let mut changed = false;
    let criteria = v.get_mut("criteria").and_then(Value::as_object_mut);
    for c in criteria.into_iter().flat_map(|o| o.values_mut()) {
        let trigger = c.get("trigger").and_then(Value::as_str);
        if !matches!(trigger, Some("minecraft:enter_block" | "enter_block")) {
            continue;
        }
        if let Some(conditions) = c.get_mut("conditions").and_then(Value::as_object_mut)
            && !conditions.contains_key("blocks")
            && let Some(block) = conditions.remove("block")
        {
            conditions.insert("blocks".into(), block);
            changed = true;
        }
    }
    changed
}

/// A 1.21.1 loot table in 26.3's shape, in place: `conditions` lists become
/// one `condition` (`all_of` for several), `functions` become `modifier`,
/// and conditions and functions name their kind in `type`. A tool match on
/// silk touch alone, or on shears alone, becomes 26.3's
/// `minecraft:tool/can_silk_touch` or `minecraft:tool/can_shear`. Number
/// providers written as a bare `{min, max}` get their `type`
/// ([`type_number`]).
pub fn upgrade_loot(v: &mut Value) {
    match v {
        Value::Array(a) => a.iter_mut().for_each(upgrade_loot),
        Value::Object(o) => {
            if let Some(Value::Array(pools)) = o.get_mut("pools") {
                for pool in pools {
                    for key in ["rolls", "bonus_rolls"] {
                        if let Some(n) = pool.get_mut(key) {
                            type_number(n);
                        }
                    }
                }
            }
            if let Some(Value::Array(mut list)) = o.remove("conditions") {
                list.iter_mut().for_each(upgrade_condition);
                let one = match <[Value; 1]>::try_from(list) {
                    Ok([c]) => c,
                    Err(list) => json!({"type": "minecraft:all_of", "terms": list}),
                };
                o.insert("condition".to_owned(), one);
            }
            if let Some(Value::Array(mut list)) = o.remove("functions") {
                for f in &mut list {
                    rename_kind(f, "function");
                    type_function_numbers(f);
                    upgrade_loot(f);
                }
                o.insert("modifier".to_owned(), Value::Array(list));
            }
            for (k, x) in o.iter_mut() {
                if k != "condition" && k != "modifier" {
                    upgrade_loot(x);
                }
            }
        }
        _ => {}
    }
}

/// A number provider in 26.3's shape, in place. 1.21.1 read a `{min, max}`
/// with no `type` as a uniform one (NumberProviders.CODEC's fallback to
/// UniformGenerator); 26.3 dispatches on `type` alone and refuses the table.
/// Bounds nested in a uniform or binomial one get theirs too.
fn type_number(v: &mut Value) {
    let Some(o) = v.as_object_mut() else {
        return;
    };
    if !o.contains_key("type") && o.contains_key("min") && o.contains_key("max") {
        o.insert("type".to_owned(), json!("minecraft:uniform"));
    }
    let kind = o.get("type").and_then(Value::as_str).unwrap_or_default();
    let bounds: &[&str] = match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "uniform" => &["min", "max"],
        "binomial" => &["n", "p"],
        _ => &[],
    };
    for key in bounds {
        if let Some(b) = o.get_mut(*key) {
            type_number(b);
        }
    }
}

/// The number providers of one loot function ([`type_number`]): the fields
/// its 1.21.1 codec reads with NumberProviders.CODEC. Range bounds such as
/// `limit_count`'s are providers too, the range itself is not.
fn type_function_numbers(f: &mut Value) {
    let kind = f["type"].as_str().unwrap_or_default().to_owned();
    let Some(f) = f.as_object_mut() else {
        return;
    };
    let mut each = |list: &str, key: &str| {
        for x in f
            .get_mut(list)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(n) = x.get_mut(key) {
                type_number(n);
            }
        }
    };
    match kind.strip_prefix("minecraft:").unwrap_or(&kind) {
        "set_stew_effect" => each("effects", "duration"),
        "set_attributes" => each("modifiers", "amount"),
        "set_enchantments" => {
            for level in f
                .get_mut("enchantments")
                .and_then(Value::as_object_mut)
                .into_iter()
                .flatten()
            {
                type_number(level.1);
            }
        }
        "limit_count" => {
            if let Some(limit) = f.get_mut("limit") {
                for key in ["min", "max"] {
                    if let Some(n) = limit.get_mut(key) {
                        type_number(n);
                    }
                }
            }
        }
        k => {
            let key = match k {
                "set_count" | "enchanted_count_increase" => "count",
                "set_damage" => "damage",
                "enchant_with_levels" => "levels",
                "set_ominous_bottle_amplifier" => "amplifier",
                _ => return,
            };
            if let Some(n) = f.get_mut(key) {
                type_number(n);
            }
        }
    }
}

/// `{"<key>": kind, ...}` as `{"type": kind, ...}`.
fn rename_kind(v: &mut Value, key: &str) {
    if let Some(o) = v.as_object_mut()
        && !o.contains_key("type")
        && let Some(kind) = o.remove(key)
    {
        o.insert("type".to_owned(), kind);
    }
}

/// One 1.21.1 loot condition in 26.3's shape.
fn upgrade_condition(c: &mut Value) {
    rename_kind(c, "condition");
    if let Some(term) = c.get_mut("term") {
        upgrade_condition(term);
    }
    if let Some(Value::Array(terms)) = c.get_mut("terms") {
        terms.iter_mut().for_each(upgrade_condition);
    }
    if c["type"] == "minecraft:match_tool"
        && let Some(named) = named_tool_condition(&c["predicate"])
    {
        *c = json!(named);
    }
}

/// 26.3's named tool conditions for the 1.21.1 predicates they replace.
fn named_tool_condition(predicate: &Value) -> Option<&'static str> {
    let keys: Vec<&str> = predicate.as_object()?.keys().map(String::as_str).collect();
    match keys.as_slice() {
        ["predicates"] => {
            let list = predicate["predicates"]["minecraft:enchantments"].as_array()?;
            let [e] = list.as_slice() else {
                return None;
            };
            let silk = e["enchantments"] == "minecraft:silk_touch"
                && e["levels"]["min"].as_u64().unwrap_or(1) <= 1
                && e["levels"].get("max").is_none()
                && predicate["predicates"].as_object()?.len() == 1;
            silk.then_some("minecraft:tool/can_silk_touch")
        }
        ["items"] => {
            let items = &predicate["items"];
            let shears = items == "minecraft:shears" || *items == json!(["minecraft:shears"]);
            shears.then_some("minecraft:tool/can_shear")
        }
        _ => None,
    }
}

/// A jar entry's root (`assets` or `data`), namespace and the rest, for
/// the files a content mod may carry.
fn split(name: &str) -> Option<(&str, &str, &str)> {
    let mut parts = name.splitn(3, '/');
    let (root, ns, rest) = (parts.next()?, parts.next()?, parts.next()?);
    let carried = (root == "assets" || root == "data") && !rest.is_empty() && !name.ends_with('/');
    carried.then_some((root, ns, rest))
}

/// Additions to vanilla and convention tags, which every mod may hold.
fn shared_tag(root: &str, ns: &str, rest: &str) -> bool {
    root == "data" && (ns == "minecraft" || ns == "c") && rest.starts_with("tags/")
}

/// Where a jar entry goes: `Some(true)` inside the content mod (its own
/// namespaces, additions to vanilla and convention tags), `Some(false)`
/// to `outside/`, `None` not carried (code, metadata).
fn place(name: &str, owned: &[&str]) -> Option<bool> {
    let (root, ns, rest) = split(name)?;
    Some(owned.contains(&ns) || shared_tag(root, ns, rest))
}

/// Namespaces no carried mod claims: vanilla's, the convention tags', the
/// game's own and the loaders'.
const NOT_CLAIMED: [&str; 6] = ["minecraft", "c", "horizon", "forge", "neoforge", "fabric"];

/// The namespaces besides `id` a mod's content owns. Some mods ship their
/// files under other names than their id (Towns and Towers, `t_and_t`,
/// under `kaisyn` and `towns_and_towers`); others ship files for mods they
/// support (`wares`, `morevillagers`) or configure (`cristellib`). A
/// namespace is the mod's when its own files (its id's, its tag additions
/// and the namespaces already claimed) name ids in it, and no mod of the
/// pack (`pack_mod`) has it as its id.
fn extra_namespaces(
    zip: &mut zip::ZipArchive<fs::File>,
    id: &str,
    pack_mod: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    for name in zip.file_names() {
        if let Some((_, ns, _)) = split(name)
            && ns != id
            && !NOT_CLAIMED.contains(&ns)
            && !pack_mod(ns)
        {
            candidates.insert(ns.to_owned());
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    // The JSON files that could be the mod's, by namespace ("" for tag additions).
    let mut texts: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let Some((root, ns, rest)) = split(entry.name()) else {
            continue;
        };
        let ns = match () {
            _ if shared_tag(root, ns, rest) => String::new(),
            _ if ns == id || candidates.contains(ns) => ns.to_owned(),
            _ => continue,
        };
        let mut bytes = Vec::new();
        if entry.name().ends_with(".json") && entry.read_to_end(&mut bytes).is_ok() {
            texts.push((ns, bytes));
        }
    }
    let mut owned: BTreeSet<String> = [String::new(), id.to_owned()].into();
    let mut claimed = Vec::new();
    loop {
        let named = candidates.iter().find(|ns| {
            let (id, tag) = (format!("\"{ns}:"), format!("\"#{ns}:"));
            texts.iter().any(|(owner, text)| {
                owned.contains(owner)
                    && (contains(text, id.as_bytes()) || contains(text, tag.as_bytes()))
            })
        });
        let Some(ns) = named.cloned() else {
            break;
        };
        candidates.remove(&ns);
        owned.insert(ns.clone());
        claimed.push(ns);
    }
    claimed.sort();
    claimed
}

/// The namespaces besides `id` the mod in `jar` would own ([`extra_namespaces`]), for the
/// pack to find the ones several mods claim.
pub fn namespaces_claimed(
    jar: &Path,
    id: &str,
    pack_mod: &dyn Fn(&str) -> bool,
) -> Result<Vec<String>, String> {
    let file = fs::File::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", jar.display()))?;
    Ok(extra_namespaces(&mut zip, id, pack_mod))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Why the game would refuse a file of the mod's own content, if it would.
/// Vanilla skips such files one by one (an invalid resource path, a data
/// file that does not parse); Horizon's content loader refuses the whole
/// set, so they stay out of the carry.
fn unreadable(name: &str, bytes: &mut Vec<u8>) -> Option<String> {
    if let Err(e) = horizon_mods::content::check_path(name) {
        return Some(e);
    }
    if !(name.ends_with(".json") || name.ends_with(".mcmeta")) {
        return None;
    }
    // A byte-order mark: Horizon's JSON readers take the file without it.
    if bytes.starts_with(b"\xEF\xBB\xBF") {
        bytes.drain(..3);
    }
    let mut value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        // Minecraft reads data files with Gson's lenient reader, which skips
        // comments: YUNG's pools have them. The carry is plain JSON.
        Err(e) => match serde_json::from_slice::<Value>(&without_comments(bytes)) {
            Ok(v) => {
                *bytes = serde_json::to_vec_pretty(&v).expect("a parsed value serializes");
                v
            }
            Err(_) => return Some(format!("does not parse as JSON: {e}")),
        },
    };
    // A vanilla or convention tag that replaces is kept: the game honours it
    // as a data pack's, with a warning (docs/mods.md). A tag of the mod's own
    // that replaces: no pack before it defines the tag, so it is the same tag
    // added to (Towns and Towers' biome tags).
    let shared_tag = name.starts_with("data/minecraft/tags/") || name.starts_with("data/c/tags/");
    let own_tag = !shared_tag && name.starts_with("data/") && name.contains("/tags/");
    if own_tag && let Some(o) = value.as_object_mut().filter(|o| o.contains_key("replace")) {
        o.remove("replace");
        *bytes = serde_json::to_vec_pretty(&value).expect("a parsed value serializes");
    }
    None
}

/// JSON text without the comments Gson's lenient reader skips: `//` and
/// `#` to the end of the line, and `/* ... */`, outside strings.
fn without_comments(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if string {
            out.push(b);
            if b == b'\\' {
                out.extend(bytes.get(i + 1));
                i += 1;
            } else if b == b'"' {
                string = false;
            }
            i += 1;
            continue;
        }
        match (b, bytes.get(i + 1)) {
            (b'"', _) => {
                string = true;
                out.push(b);
                i += 1;
            }
            (b'/', Some(b'/')) | (b'#', _) => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            (b'/', Some(b'*')) => {
                i += 2;
                while i < bytes.len() && !bytes[i..].starts_with(b"*/") {
                    i += 1;
                }
                i += 2;
                out.push(b' ');
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Carries `jar` over into `out/carry/`. With the oracle's `facts`, blocks
/// and items are the ones the mod registers, with the templates and numbers
/// the facts give; `allowed` says which vanilla blocks the game has;
/// `pack` which mods the pack holds and ports along.
pub fn carry(
    jar: &Path,
    info: &JarInfo,
    out: &Path,
    facts: Option<&Facts>,
    allowed: &dyn Fn(&str) -> bool,
    pack: &Pack,
    conventions: &BTreeMap<String, Value>,
) -> Result<Report, String> {
    let loaded = pack.loaded;
    let id = &info.mod_id;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(format!(
            "mod id {id:?} is not [a-z0-9_]{{1,64}}: rename it before carrying over"
        ));
    }
    let file = fs::File::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", jar.display()))?;
    let (inside, outside) = (out.join("carry"), out.join("outside"));
    let _ = fs::remove_dir_all(&inside);
    let _ = fs::remove_dir_all(&outside);
    let mut report = Report::default();
    let mut blockstates: BTreeMap<String, Value> = BTreeMap::new();
    let mut item_models = BTreeSet::new();
    let mut convention = BTreeSet::new();
    let mut block_loot = BTreeSet::new();
    let mut biome_conventions = BTreeSet::new();
    let set_tags = crate::worldgen::structure_set_tags(&mut zip);
    let piece_counts = crate::worldgen::piece_counts(&mut zip);
    let mut processor_data = crate::processors::ProcessorData::read(&mut zip);
    processor_data.mark_loaded(loaded);
    report.namespaces = extra_namespaces(&mut zip, id, pack.mods);
    let owned: Vec<&str> = std::iter::once(id.as_str())
        .chain(report.namespaces.iter().map(String::as_str))
        .collect();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        let Some(keep) = place(&name, &owned) else {
            continue;
        };
        // `..` or an empty part escapes on every platform; `\` or `:` (a drive or
        // a separator) escapes on Windows.
        if name.split('/').any(|p| p == ".." || p.is_empty()) || name.contains(['\\', ':']) {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{name}: {e}"))?;
        let unreadable = if keep {
            unreadable(&name, &mut bytes)
        } else {
            None
        };
        let keep = keep && unreadable.is_none();
        if let Some(why) = unreadable {
            report.unreadable.insert(name.clone(), why);
        }
        let dest = if keep {
            report.copied += 1;
            inside.join(&name)
        } else {
            report.outside.push(name.clone());
            outside.join(&name)
        };
        if keep {
            // The path in one of the mod's own namespaces; "" for tag additions.
            let (ns, rel) = match split(&name) {
                Some((_, ns, rest)) if owned.contains(&ns) => (ns, rest),
                _ => ("", ""),
            };
            // Blocks and items are declared under the mod id only.
            let own = ns == id;
            if name.starts_with("data/")
                && (rel.starts_with("worldgen/") || rel.starts_with("structure/"))
            {
                report.worldgen += 1;
            }
            if let Some(file) = rel
                .strip_prefix("recipe/")
                .filter(|_| name.starts_with("data/"))
            {
                if let Ok(mut recipe) = serde_json::from_slice::<Value>(&bytes) {
                    tags_used(&recipe, &mut convention);
                    let renamed = renamed_ids(&mut recipe);
                    let upgraded = upgrade_recipe(&mut recipe);
                    if upgraded {
                        report.recipes_upgraded += 1;
                    } else {
                        report.recipes_kept.push(file.to_owned());
                    }
                    if upgraded || renamed {
                        bytes = serde_json::to_vec_pretty(&recipe).unwrap_or(bytes);
                    }
                }
            } else if let Some(kind) = rel
                .strip_prefix("worldgen/")
                .and_then(|r| r.split_once('/'))
                .map(|(kind, _)| kind)
                .filter(|_| name.starts_with("data/") && name.ends_with(".json"))
            {
                if let Ok(mut v) = serde_json::from_slice::<Value>(&bytes) {
                    if kind == "structure" {
                        crate::worldgen::convention_tags(&v["biomes"], &mut biome_conventions);
                    }
                    let types = &mut report.worldgen_types;
                    let carried = crate::processors::carry(
                        kind,
                        &mut v,
                        &processor_data,
                        &mut types.processors_carried,
                    ) | crate::processors::rule_tests(
                        kind,
                        &mut v,
                        &mut types.rule_tests_carried,
                    );
                    let mut changed =
                        crate::worldgen::upgrade(kind, &mut v, types, &set_tags) || carried;
                    if kind == "structure" {
                        let path = rel.trim_start_matches("worldgen/structure/");
                        let structure = format!("{ns}:{}", path.trim_end_matches(".json"));
                        changed |=
                            crate::worldgen::add_piece_counts(&structure, &mut v, &piece_counts);
                    }
                    if changed {
                        bytes = serde_json::to_vec_pretty(&v).unwrap_or(bytes);
                    }
                }
            } else if let Some(path) = rel
                .strip_prefix("lithostitched/worldgen_modifier/")
                .filter(|_| name.starts_with("data/") && name.ends_with(".json"))
            {
                // Additions to other sets and pools, as files the game merges.
                let modifier = serde_json::from_slice::<Value>(&bytes).ok();
                let types = &mut report.worldgen_types;
                if let Some((kind, mut added)) =
                    modifier.and_then(|v| crate::lithostitched::modifier(&v, loaded, types))
                {
                    crate::processors::carry(
                        kind,
                        &mut added,
                        &processor_data,
                        &mut types.processors_carried,
                    );
                    crate::processors::rule_tests(kind, &mut added, &mut types.rule_tests_carried);
                    crate::worldgen::upgrade(kind, &mut added, types, &set_tags);
                    let to = format!(
                        "data/{id}/worldgen/{kind}/{}/{path}",
                        crate::worldgen::ADDED
                    );
                    write(
                        &inside.join(to),
                        &serde_json::to_vec_pretty(&added).unwrap_or_default(),
                    )?;
                }
            } else if rel.starts_with("trial_spawner/")
                && name.starts_with("data/")
                && name.ends_with(".json")
            {
                // Trial spawner configs (Moog's arenas): their mobs' NBT.
                if let Ok(mut config) = serde_json::from_slice::<Value>(&bytes)
                    && crate::processors::upgrade_trial_spawner(&mut config)
                {
                    bytes = serde_json::to_vec_pretty(&config).unwrap_or(bytes);
                }
            } else if rel.starts_with("tags/worldgen/biome/") && name.starts_with("data/") {
                if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                    crate::worldgen::convention_tags(&v, &mut biome_conventions);
                }
            } else if rel.starts_with("advancement/")
                && name.starts_with("data/")
                && name.ends_with(".json")
            {
                if let Ok(mut advancement) = serde_json::from_slice::<Value>(&bytes)
                    && upgrade_advancement(&mut advancement)
                {
                    report.advancements_upgraded += 1;
                    bytes = serde_json::to_vec_pretty(&advancement).unwrap_or(bytes);
                }
            } else if rel.starts_with("loot_table/") && name.starts_with("data/") {
                if let Some(b) = rel
                    .strip_prefix("loot_table/blocks/")
                    .and_then(|b| b.strip_suffix(".json"))
                    .filter(|_| own)
                {
                    block_loot.insert(b.to_owned());
                }
                if let Ok(mut table) = serde_json::from_slice::<Value>(&bytes) {
                    let before = table.clone();
                    upgrade_loot(&mut table);
                    renamed_ids(&mut table);
                    if table != before {
                        report.loot_tables_upgraded += 1;
                        bytes = serde_json::to_vec_pretty(&table).unwrap_or(bytes);
                    }
                }
            } else if let Some(b) = rel
                .strip_prefix("blockstates/")
                .and_then(|b| b.strip_suffix(".json"))
                .filter(|_| own)
            {
                if let Ok(v) = serde_json::from_slice(&bytes) {
                    blockstates.insert(b.to_owned(), v);
                }
            } else if let Some(m) = rel
                .strip_prefix("models/item/")
                .and_then(|m| m.strip_suffix(".json"))
                .filter(|_| own)
            {
                item_models.insert(m.to_owned());
            }
        }
        write(&dest, &bytes)?;
    }
    // Entries the mod adds to other structure sets in Java code, for the
    // structures it has.
    for (file, set, entries) in crate::legacy_structure_mods::ctov::code_set_entries(id) {
        let entries: Vec<Value> = entries
            .into_iter()
            .filter(|(s, _)| {
                s.split_once(':').is_some_and(|(ns, path)| {
                    inside
                        .join(format!("data/{ns}/worldgen/structure/{path}.json"))
                        .exists()
                })
            })
            .map(|(structure, weight)| json!({"structure": structure, "weight": weight}))
            .collect();
        if entries.is_empty() {
            continue;
        }
        let added = &mut report.worldgen_types.added_to;
        *added.entry(set.to_owned()).or_default() += entries.len();
        let to = format!(
            "data/{id}/worldgen/structure_set/{}/{file}.json",
            crate::worldgen::ADDED
        );
        let v = json!({"horizon:add_to": set, "structures": entries});
        write(
            &inside.join(to),
            &serde_json::to_vec_pretty(&v).unwrap_or_default(),
        )?;
    }
    // 1.21.4 moved item models behind `items/<name>.json` definitions.
    for m in &item_models {
        let def = inside.join(format!("assets/{id}/items/{m}.json"));
        if !def.exists() {
            let model =
                json!({"model": {"type": "minecraft:model", "model": format!("{id}:item/{m}")}});
            write(&def, &serde_json::to_vec_pretty(&model).unwrap_or_default())?;
            report.item_definitions_added += 1;
        }
    }
    // The loader's convention biome tags the structures and their tags name
    // (NeoForge's `c:is_plains`...): the game has no loader, so they come along.
    for tag in crate::worldgen::with_included(&biome_conventions, conventions) {
        let file = inside.join(format!("data/c/tags/worldgen/biome/{tag}.json"));
        if !file.exists() {
            write(
                &file,
                &serde_json::to_vec_pretty(&conventions[&tag]).unwrap_or_default(),
            )?;
            report.convention_biome_tags += 1;
        }
    }
    let decl = inside.join(format!("data/{id}/horizon"));
    let write_decl = |kind: &str, name: &str, v: &Value| {
        let text = serde_json::to_vec_pretty(v).unwrap_or_default();
        write(&decl.join(format!("{kind}/{name}.json")), &text)
    };
    if let Some(facts) = facts {
        report.oracle = true;
        let prefix = format!("{id}:");
        let mut declared = BTreeSet::new();
        for (full, block) in facts.blocks.range(prefix.clone()..) {
            let Some(b) = full.strip_prefix(&prefix) else {
                break;
            };
            let Some(choice) = facts.template(full, allowed) else {
                let props: Vec<&str> = block.properties.keys().map(String::as_str).collect();
                let why = format!(
                    "no vanilla {} with properties [{}]",
                    block.vanilla_class().rsplit('.').next().unwrap_or(""),
                    props.join(", ")
                );
                report.blocks_unknown.insert(b.to_owned(), why);
                continue;
            };
            let mut v = json!({"template": choice.template, "properties": choice.properties});
            if block.item == "minecraft:air" {
                v["item"] = json!(false);
            } else if block.item != *full {
                v["item"] = json!(false);
                report
                    .differences
                    .entry(b.to_owned())
                    .or_default()
                    .push(format!("its item is {}", block.item));
            } else {
                declared.insert(full.clone());
                let drop = (!block_loot.contains(b))
                    .then(|| block.drops_itself(full))
                    .flatten();
                if let Some(drop) = drop {
                    let mut entry = json!({"type": "minecraft:item", "name": full});
                    if let crate::oracle::SelfDrop::When(property, value) = drop {
                        entry["condition"] = json!({
                            "type": "minecraft:match_block",
                            "blocks": full,
                            "state": {property: value}
                        });
                    }
                    let table = json!({
                        "type": "minecraft:block",
                        "pools": [{
                            "condition": {"type": "minecraft:survives_explosion"},
                            "entries": [entry],
                            "rolls": 1
                        }],
                        "random_sequence": format!("{id}:blocks/{b}")
                    });
                    let file = inside.join(format!("data/{id}/loot_table/blocks/{b}.json"));
                    write(
                        &file,
                        &serde_json::to_vec_pretty(&table).unwrap_or_default(),
                    )?;
                    report.loot_tables_added += 1;
                }
            }
            write_decl("block", b, &v)?;
            report.blocks.insert(b.to_owned(), choice.template);
            if !choice.differences.is_empty() {
                report
                    .differences
                    .entry(b.to_owned())
                    .or_default()
                    .extend(choice.differences);
            }
        }
        for (full, item) in facts.items.range(prefix.clone()..) {
            let Some(name) = full.strip_prefix(&prefix) else {
                break;
            };
            if declared.contains(full) {
                continue;
            }
            if item
                .block
                .as_ref()
                .is_some_and(|b| facts.blocks.contains_key(b))
                && report.blocks_unknown.contains_key(name)
            {
                report
                    .items_unknown
                    .insert(name.to_owned(), "its block is left out".into());
                continue;
            }
            match facts.item(full) {
                Ok((v, differences)) => {
                    write_decl("item", name, &v)?;
                    report.items += 1;
                    if !differences.is_empty() {
                        report.differences.insert(name.to_owned(), differences);
                    }
                }
                Err(e) => {
                    report.items_unknown.insert(name.to_owned(), e);
                }
            }
        }
        // Convention tags are NeoForge's and the pack's: the recipes get the
        // members the oracle saw, those of vanilla and of this mod.
        for tag in &convention {
            let Some(path) = tag.strip_prefix("c:") else {
                continue;
            };
            let file = inside.join(format!("data/c/tags/item/{path}.json"));
            if file.exists() {
                continue;
            }
            if let Some(members) = facts.tag_members("item", tag, id) {
                let v = json!({"values": members});
                write(&file, &serde_json::to_vec_pretty(&v).unwrap_or_default())?;
                report.convention_tags += 1;
            }
        }
    } else {
        for (b, state) in &blockstates {
            match block_template(b, state) {
                Some(t) => {
                    write_decl("block", b, &json!({"template": t}))?;
                    report.blocks.insert(b.clone(), t.to_owned());
                }
                None => {
                    let props: Vec<String> = properties(state).into_iter().collect();
                    let why = format!("no template for properties [{}]", props.join(", "));
                    report.blocks_unknown.insert(b.clone(), why);
                }
            }
        }
        for m in item_models.iter().filter(|m| !blockstates.contains_key(*m)) {
            write_decl("item", m, &json!({"template": "minecraft:stick"}))?;
            report.items += 1;
        }
    }
    let owned: Vec<String> = owned.iter().map(|n| n.to_string()).collect();
    let owned: Vec<&str> = owned.iter().map(String::as_str).collect();
    unregistered_recipes(&inside, &outside, id, &owned, &mut report)?;
    let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let name = if info.name.is_empty() { id } else { &info.name };
    let mut manifest = format!(
        "# Carried over from {jar} by horizon-port-pack, for local use only.\n# Licence: {license}\nid = {}\nname = {}\nversion = {}\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        q(id),
        q(name),
        q(&info.version),
        jar = jar.file_name().unwrap_or_default().to_string_lossy(),
        license = if info.license.is_empty() {
            "not stated"
        } else {
            &info.license
        },
    );
    if !report.namespaces.is_empty() {
        let list: Vec<String> = report.namespaces.iter().map(|n| q(n)).collect();
        manifest.push_str(&format!("namespaces = [{}]\n", list.join(", ")));
    }
    write(&inside.join("horizon-mod.toml"), manifest.as_bytes())?;
    write(
        &out.join("carry-report.json"),
        &serde_json::to_vec_pretty(&report).unwrap_or_default(),
    )?;
    Ok(report)
}

/// The item a recipe makes: `result.id` (1.21 and later), `result.item` or a bare id.
fn recipe_result(recipe: &Value) -> Option<&str> {
    let r = &recipe["result"];
    r.as_str()
        .or_else(|| r["id"].as_str())
        .or_else(|| r["item"].as_str())
}

/// Moves the recipes whose result is an undeclared item of the mod's own namespaces from
/// `inside` to `outside` ([`Report::recipes_unregistered`]). The declared items are the blocks
/// with their item and the items under `data/<id>/horizon/`.
fn unregistered_recipes(
    inside: &Path,
    outside: &Path,
    id: &str,
    owned: &[&str],
    report: &mut Report,
) -> Result<(), String> {
    let decl = inside.join(format!("data/{id}/horizon"));
    let mut declared = BTreeSet::new();
    for kind in ["block", "item"] {
        for f in fs::read_dir(decl.join(kind))
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = f.path();
            let Some(name) = path.file_stem().and_then(|n| n.to_str()) else {
                continue;
            };
            let has_item = kind == "item"
                || fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .is_none_or(|v| v["item"] != json!(false));
            if has_item {
                declared.insert(format!("{id}:{name}"));
            }
        }
    }
    for ns in owned {
        let dir = inside.join(format!("data/{ns}/recipe"));
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            for f in fs::read_dir(&d).into_iter().flatten().flatten() {
                let path = f.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(recipe) = fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                else {
                    continue;
                };
                let Some(result) = recipe_result(&recipe) else {
                    continue;
                };
                let unknown = result
                    .split_once(':')
                    .is_some_and(|(n, _)| owned.contains(&n) && !declared.contains(result));
                if !unknown {
                    continue;
                }
                let rel = path.strip_prefix(inside).map_err(|e| e.to_string())?;
                let to = outside.join(rel);
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                fs::rename(&path, &to).map_err(|e| format!("{}: {e}", to.display()))?;
                report.copied = report.copied.saturating_sub(1);
                report
                    .recipes_unregistered
                    .push(rel.to_string_lossy().into_owned());
            }
        }
    }
    report.recipes_unregistered.sort();
    Ok(())
}

/// The tags a recipe's ingredients name (`#ns:path` or `{"tag": ...}`).
fn tags_used(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => {
            if let Some(t) = s.strip_prefix('#') {
                out.insert(t.to_owned());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| tags_used(x, out)),
        Value::Object(o) => {
            for (k, x) in o {
                match (k.as_str(), x) {
                    ("tag", Value::String(t)) => {
                        out.insert(t.clone());
                    }
                    ("result", _) => {}
                    _ => tags_used(x, out),
                }
            }
        }
        _ => {}
    }
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// YUNG's pools carry comments, which Minecraft's lenient reader skips;
    /// the carry holds them as plain JSON. A comment marker inside a string
    /// stays, and text that is not JSON without its comments is refused.
    #[test]
    fn reads_json_with_comments_as_minecraft_does() {
        let name = "data/m/worldgen/template_pool/bridge.json";
        let mut bytes = br#"{
  // Pillar w/ chance of unlit beacon
  "elements": [ /* one */ {"weight": 20, "location": "m:a//b#c"} ], # done
  "fallback": "minecraft:empty"
}"#
        .to_vec();
        assert_eq!(unreadable(name, &mut bytes), None);
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            v,
            json!({"elements": [{"weight": 20, "location": "m:a//b#c"}],
                "fallback": "minecraft:empty"})
        );
        let mut plain = br#"{"a": "b\"//"}"#.to_vec();
        assert_eq!(unreadable(name, &mut plain), None);
        assert_eq!(plain, br#"{"a": "b\"//"}"#.to_vec());
        let mut broken = b"{\"a\": 1} trailing".to_vec();
        assert!(
            unreadable(name, &mut broken)
                .unwrap()
                .starts_with("does not parse")
        );
    }

    #[test]
    fn enter_block_criteria_name_their_blocks() {
        let mut a = json!({"criteria": {
            "dive": {"trigger": "minecraft:enter_block",
                "conditions": {"block": "m:giant_lily_pad"}},
            "swim": {"trigger": "minecraft:enter_block",
                "conditions": {"blocks": ["minecraft:water"]}},
            "has": {"trigger": "minecraft:recipe_unlocked",
                "conditions": {"recipe": "m:boat"}}}});
        assert!(upgrade_advancement(&mut a));
        assert_eq!(
            a["criteria"]["dive"]["conditions"],
            json!({"blocks": "m:giant_lily_pad"})
        );
        assert_eq!(
            a["criteria"]["swim"]["conditions"],
            json!({"blocks": ["minecraft:water"]})
        );
        assert_eq!(
            a["criteria"]["has"]["conditions"],
            json!({"recipe": "m:boat"})
        );
        // Already 26.3's: unchanged.
        assert!(!upgrade_advancement(&mut a));
        assert!(!upgrade_advancement(&mut json!({"parent": "m:root"})));
    }

    #[test]
    fn upgrades_ingredients() {
        let mut r = json!({"type": "minecraft:crafting_shaped", "pattern": ["##"],
            "key": {"#": {"item": "gem:gem"}, "s": {"tag": "c:rods/wooden"}},
            "result": {"id": "gem:gem_block", "count": 1}});
        assert!(upgrade_recipe(&mut r));
        assert_eq!(r["key"], json!({"#": "gem:gem", "s": "#c:rods/wooden"}));
        let mut custom = json!({"type": "gem:infuse", "ingredient": {"item": "gem:gem"}});
        assert!(!upgrade_recipe(&mut custom));
        let mut tag_list = json!({"type": "minecraft:smelting",
            "ingredient": [{"tag": "c:ores"}, {"item": "gem:gem"}]});
        assert!(!upgrade_recipe(&mut tag_list));
    }

    #[test]
    fn upgrades_loot_tables() {
        let mut slab = json!({"type": "minecraft:block", "pools": [{"rolls": 1.0, "entries": [{
            "type": "minecraft:item", "name": "m:slab",
            "functions": [{"function": "minecraft:set_count", "count": 2.0, "add": false,
                "conditions": [{"condition": "minecraft:block_state_property", "block": "m:slab",
                    "properties": {"type": "double"}}]},
                {"function": "minecraft:explosion_decay"}]}]}]});
        upgrade_loot(&mut slab);
        assert_eq!(
            slab["pools"][0]["entries"][0]["modifier"][0],
            json!({"type": "minecraft:set_count", "count": 2.0, "add": false,
                "condition": {"type": "minecraft:block_state_property", "block": "m:slab",
                    "properties": {"type": "double"}}})
        );
        let silk = json!({"condition": "minecraft:match_tool", "predicate": {"predicates": {
            "minecraft:enchantments": [{"enchantments": "minecraft:silk_touch", "levels": {"min": 1}}]}}});
        let shears = json!({"condition": "minecraft:match_tool", "predicate": {"items": "minecraft:shears"}});
        let mut glass = json!({"pools": [{"conditions": [silk.clone()], "entries": []},
            {"conditions": [{"condition": "minecraft:any_of", "terms": [silk, shears]},
                {"condition": "minecraft:survives_explosion"}], "entries": []}]});
        upgrade_loot(&mut glass);
        assert_eq!(
            glass["pools"][0]["condition"],
            "minecraft:tool/can_silk_touch"
        );
        assert_eq!(
            glass["pools"][1]["condition"],
            json!({"type": "minecraft:all_of", "terms": [
                {"type": "minecraft:any_of", "terms": ["minecraft:tool/can_silk_touch", "minecraft:tool/can_shear"]},
                {"type": "minecraft:survives_explosion"}]})
        );
        // A loot table already in 26.3's shape stays as it is.
        let mut new = glass.clone();
        upgrade_loot(&mut new);
        assert_eq!(new, glass);
    }

    #[test]
    fn types_bare_number_ranges() {
        // When Dungeons Arise's and Structory's chests: rolls and counts as
        // bare ranges, nested in each other or in a binomial.
        let mut chest = json!({"pools": [{"rolls": {"min": 1, "max": 2}, "bonus_rolls": 0,
            "entries": [{"type": "minecraft:item", "name": "minecraft:cod", "functions": [
                {"function": "minecraft:set_count", "count": {"min": 3, "max": {"min": 5, "max": 7}}},
                {"function": "minecraft:enchant_with_levels", "levels": {"type": "minecraft:binomial",
                    "n": {"min": 1, "max": 3}, "p": 0.5}},
                {"function": "minecraft:limit_count", "limit": {"min": {"min": 0, "max": 1}, "max": 4}},
                {"function": "minecraft:set_count", "count": 2,
                    "conditions": [{"condition": "minecraft:match_tool",
                        "predicate": {"count": {"min": 1, "max": 3}}}]}]}]}]});
        upgrade_loot(&mut chest);
        let uniform =
            |min: Value, max: Value| json!({"type": "minecraft:uniform", "min": min, "max": max});
        let pool = &chest["pools"][0];
        assert_eq!(pool["rolls"], uniform(json!(1), json!(2)));
        assert_eq!(pool["bonus_rolls"], 0);
        let modifiers = &pool["entries"][0]["modifier"];
        assert_eq!(
            modifiers[0]["count"],
            uniform(json!(3), uniform(json!(5), json!(7)))
        );
        assert_eq!(modifiers[1]["levels"]["n"], uniform(json!(1), json!(3)));
        // A range stays a range; its bounds are providers.
        assert_eq!(
            modifiers[2]["limit"],
            json!({"min": uniform(json!(0), json!(1)), "max": 4})
        );
        // An item predicate's count is a range too.
        assert_eq!(
            modifiers[3]["condition"]["predicate"]["count"],
            json!({"min": 1, "max": 3})
        );
        // Already typed, it stays as it is.
        let mut again = chest.clone();
        upgrade_loot(&mut again);
        assert_eq!(again, chest);
    }

    #[test]
    fn renames_ids_vanilla_renamed() {
        let mut recipe = json!({"type": "minecraft:crafting_shaped", "pattern": ["c"],
            "key": {"c": {"item": "minecraft:chain"}}, "result": {"id": "m:sign", "count": 1}});
        assert!(renamed_ids(&mut recipe));
        assert_eq!(recipe["key"]["c"]["item"], "minecraft:iron_chain");
        let mut rule = json!({"input_predicate": {"predicate_type": "minecraft:block_match",
            "block": "minecraft:chain"}, "output_state": {"id": "minecraft:chain",
            "properties": {"axis": "y"}}});
        assert!(renamed_ids(&mut rule));
        assert_eq!(rule["input_predicate"]["block"], "minecraft:iron_chain");
        assert_eq!(rule["output_state"]["id"], "minecraft:iron_chain");
        // Only whole ids: other blocks and mods' own chains stay.
        let mut other = json!([
            "minecraft:chain_command_block",
            "m:chain",
            "minecraft:iron_chain"
        ]);
        assert!(!renamed_ids(&mut other));
    }

    #[test]
    fn templates_from_blockstate_properties() {
        let door =
            json!({"variants": {"facing=east,half=lower,hinge=left,open=false,powered=false": {}}});
        assert_eq!(
            template("gem_door", &properties(&door)),
            Some("minecraft:oak_door")
        );
        // Models that ignore `powered`: still a door.
        let door = json!({"variants": {"facing=east,half=lower,hinge=left,open=false": {}}});
        assert_eq!(
            template("gem_door", &properties(&door)),
            Some("minecraft:oak_door")
        );
        let trapdoor = json!({"variants": {"facing=east,half=top,open=false": {}}});
        assert_eq!(
            template("gem_trapdoor", &properties(&trapdoor)),
            Some("minecraft:oak_trapdoor")
        );
        let sides = json!({"multipart": [{"when": {"north": "true"}}, {"when": {"east": "true"}},
            {"when": {"south": "true"}}, {"when": {"west": "true"}}]});
        assert_eq!(
            template("gem_pane", &properties(&sides)),
            Some("minecraft:glass_pane")
        );
        assert_eq!(
            template("gem_fence", &properties(&sides)),
            Some("minecraft:oak_fence")
        );
        let fence = json!({"multipart": [{"apply": {}}, {"when": {"north": "true"}, "apply": {}},
            {"when": {"east": "true"}}, {"when": {"south": "true"}}, {"when": {"west": "true"}},
            {"when": {"OR": [{"waterlogged": "true"}]}}]});
        assert_eq!(
            template("gem_fence", &properties(&fence)),
            Some("minecraft:oak_fence")
        );
        assert_eq!(
            template("gem_block", &properties(&json!({"variants": {"": {}}}))),
            Some("minecraft:stone")
        );
        // Walls and vines have the same sides and up; their values tell them apart.
        let wall = json!({"multipart": [{"when": {"up": "true"}},
            {"when": {"north": "low"}}, {"when": {"east": "tall"}},
            {"when": {"south": "low"}}, {"when": {"west": "low"}}]});
        assert_eq!(
            block_template("gem_wall", &wall),
            Some("minecraft:cobblestone_wall")
        );
        let vine = json!({"multipart": [{"when": {"up": "true"}},
            {"when": {"up": "false", "north": "false", "east": "false", "south": "false",
                "west": "false"}}, {"when": {"north": "true"}}]});
        assert_eq!(block_template("gem_vine", &vine), Some("minecraft:vine"));
        let odd = json!({"variants": {"charge=1,mode=fast": {}}});
        assert_eq!(template("gem_core", &properties(&odd)), None);
    }

    /// A small NeoForge jar carried over is a content mod Horizon loads.
    #[test]
    fn carried_jar_is_a_content_mod() {
        let dir = std::env::temp_dir().join(format!("horizon-port-pack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("gem-1.0.jar");
        let mut zip = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        let files: &[(&str, &str)] = &[
            (
                "META-INF/neoforge.mods.toml",
                "license = \"MIT\"\n[[mods]]\nmodId = \"gem\"\nversion = \"1.0\"\ndisplayName = \"Gem\"\n",
            ),
            ("com/example/gem/Gem.class", "not real bytecode"),
            (
                "assets/gem/blockstates/gem_block.json",
                r#"{"variants": {"": {"model": "gem:block/gem_block"}}}"#,
            ),
            (
                "assets/gem/blockstates/gem_door.json",
                r#"{"variants": {"facing=east,half=lower,hinge=left,open=false,powered=false": {"model": "gem:block/gem_door"}}}"#,
            ),
            (
                "assets/gem/blockstates/gem_core.json",
                r#"{"variants": {"charge=1": {"model": "gem:block/gem_core"}}}"#,
            ),
            (
                "assets/gem/models/item/gem.json",
                r#"{"parent": "minecraft:item/generated", "textures": {"layer0": "gem:item/gem"}}"#,
            ),
            (
                "assets/gem/models/item/gem_block.json",
                r#"{"parent": "gem:block/gem_block"}"#,
            ),
            ("assets/gem/textures/item/gem.png", "png"),
            ("assets/gem/lang/en_us.json", r#"{"item.gem.gem": "Gem"}"#),
            (
                "data/gem/recipe/gem_block.json",
                r####"{"type": "minecraft:crafting_shaped", "pattern": ["###", "###", "###"],
                    "key": {"#": {"item": "gem:gem"}}, "result": {"id": "gem:gem_block", "count": 1}}"####,
            ),
            (
                "data/gem/recipe/infuse.json",
                r#"{"type": "gem:infuse", "ingredient": {"item": "gem:gem"}}"#,
            ),
            (
                "data/gem/loot_table/blocks/gem_block.json",
                r#"{"type": "minecraft:block", "pools": []}"#,
            ),
            (
                "data/minecraft/tags/block/mineable/pickaxe.json",
                r#"{"values": ["gem:gem_block"]}"#,
            ),
            ("data/c/tags/item/gems.json", r#"{"values": ["gem:gem"]}"#),
            (
                "data/gem/lithostitched/worldgen_modifier/houses.json",
                r#"{"type": "lithostitched:add_template_pool_elements",
                    "template_pools": "minecraft:village/plains/houses",
                    "elements": [{"weight": 2, "element": {"element_type": "minecraft:single_pool_element",
                    "location": "gem:hut", "projection": "rigid",
                    "processors": {"processors": [{"processor_type": "lithostitched:block_swap"}]}}}]}"#,
            ),
            (
                "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json",
                r#"{"replace": true, "values": []}"#,
            ),
            // What the game would refuse: vanilla skips these files alone.
            ("assets/gem/lang/ry_UA.json", r#"{"item.gem.gem": "Gem"}"#),
            ("data/gem/tags/item/empty.json", ""),
            (
                "assets/gem/lang/de_de.json",
                "\u{feff}{\"item.gem.gem\": \"Juwel\"}",
            ),
        ];
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        let info = crate::jar::read(&jar).unwrap();
        assert_eq!(
            (info.mod_id.as_str(), info.license.as_str()),
            ("gem", "MIT")
        );
        assert_eq!(info.census.classes, 1);
        let out = dir.join("work/mods/gem");
        let report = carry(
            &jar,
            &info,
            &out,
            None,
            &|_| true,
            &Pack {
                loaded: &|_| true,
                mods: &|_| false,
            },
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(report.copied, 15);
        let unreadable: Vec<&str> = report.unreadable.keys().map(String::as_str).collect();
        assert_eq!(
            unreadable,
            [
                "assets/gem/lang/ry_UA.json",
                "data/gem/tags/item/empty.json"
            ]
        );
        assert_eq!(report.outside.len(), 2);
        // A vanilla tag that replaces is kept as it is: the game honours it.
        let replacing: Value = serde_json::from_slice(
            &fs::read(out.join(
                "carry/data/minecraft/tags/worldgen/biome/has_structure/village_plains.json",
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(replacing, json!({"replace": true, "values": []}));
        assert!(!out.join("carry/data/gem/tags/item/empty.json").exists());
        // A byte-order mark is dropped, the file kept.
        assert_eq!(
            fs::read(out.join("carry/assets/gem/lang/de_de.json")).unwrap()[0],
            b'{'
        );
        assert_eq!((report.recipes_upgraded, report.recipes_kept.len()), (1, 1));
        assert_eq!(report.item_definitions_added, 2);
        assert_eq!(report.blocks["gem_door"], "minecraft:oak_door");
        assert_eq!(report.blocks["gem_block"], "minecraft:stone");
        assert_eq!(
            report.blocks_unknown.keys().collect::<Vec<_>>(),
            ["gem_core"]
        );
        assert_eq!(report.items, 1);
        let carried = out.join("carry");
        assert!(!carried.join("com").exists());
        let recipe: Value = serde_json::from_slice(
            &fs::read(carried.join("data/gem/recipe/gem_block.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(recipe["key"]["#"], "gem:gem");
        // A lithostitched pool addition, as a file the game merges, upgraded
        // as the mod's own pools are.
        let added: Value = serde_json::from_slice(
            &fs::read(carried.join("data/gem/worldgen/template_pool/horizon_added/houses.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            added["horizon:add_to"],
            json!(["minecraft:village/plains/houses"])
        );
        assert_eq!(
            added["elements"][0]["element"]["processors"],
            json!({"processors": []})
        );
        assert_eq!(
            report.worldgen_types.added_to["minecraft:village/plains/houses"],
            1
        );

        // Horizon's own reader accepts it: blocks and items declared, files fenced.
        let package = horizon_mods::Package::read(&carried).unwrap();
        let content = horizon_mods::Content::read(&carried, &package.manifest).unwrap();
        let blocks: Vec<&str> = content.blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(blocks, ["gem:gem_block", "gem:gem_door"]);
        assert_eq!(content.items.len(), 1);
        assert_eq!(content.items[0].id, "gem:gem");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A mod shipping its files under other names than its id: the
    /// namespaces its own files name are its, compat files for other mods
    /// and a pack mod's namespace stay outside.
    #[test]
    fn claims_the_namespaces_its_files_name() {
        let dir = std::env::temp_dir().join(format!("horizon-port-pack-ns-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("towers-1.0.jar");
        let mut zip = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        let files: &[(&str, &str)] = &[
            (
                "META-INF/neoforge.mods.toml",
                "license = \"MIT\"\n[[mods]]\nmodId = \"towers\"\nversion = \"1.0\"\n",
            ),
            // Its tag addition names its structure; the structure names its pool.
            (
                "data/minecraft/tags/worldgen/structure/village.json",
                r#"{"values": ["towers_wg:tower"]}"#,
            ),
            (
                "data/towers_wg/worldgen/structure/tower.json",
                r#"{"type": "minecraft:jigsaw", "start_pool": "kaisyn:tower/start"}"#,
            ),
            ("data/kaisyn/structure/tower.nbt", "nbt"),
            (
                "data/towers_wg/tags/worldgen/biome/has_tower.json",
                r#"{"replace": true, "values": ["minecraft:plains"]}"#,
            ),
            (
                "assets/kaisyn/blockstates/brick.json",
                r#"{"variants": {}}"#,
            ),
            // Compat for a mod outside the pack, named by nothing of its own.
            ("data/wares/loot_table/crate.json", r#"{"pools": []}"#),
            // A pack mod's config naming the mod's sets, and a loader's file.
            (
                "data/cristel/config/towers.json",
                r#"{"sets": ["towers_wg:towns"]}"#,
            ),
            (
                "data/neoforge/loot_modifiers/global.json",
                r#"{"entries": ["towers_wg:x"]}"#,
            ),
        ];
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let info = crate::jar::read(&jar).unwrap();
        let out = dir.join("work/mods/towers");
        let pack = Pack {
            loaded: &|_| true,
            mods: &|m| m == "cristel",
        };
        let report = carry(&jar, &info, &out, None, &|_| true, &pack, &BTreeMap::new()).unwrap();
        assert_eq!(report.namespaces, ["kaisyn", "towers_wg"]);
        assert_eq!(report.worldgen, 2);
        assert_eq!(
            report.outside,
            [
                "data/wares/loot_table/crate.json",
                "data/cristel/config/towers.json",
                "data/neoforge/loot_modifiers/global.json",
            ]
        );
        // No block is declared from another namespace's blockstate.
        assert!(report.blocks.is_empty());
        let carried = out.join("carry");
        let package = horizon_mods::Package::read(&carried).unwrap();
        assert_eq!(package.manifest.namespaces, ["kaisyn", "towers_wg"]);
        let content = horizon_mods::Content::read(&carried, &package.manifest).unwrap();
        assert!(
            content
                .files
                .contains(&"data/kaisyn/structure/tower.nbt".to_owned())
        );
        assert!(content.blocks.is_empty());
        // Its own tag that replaces is the same tag added to.
        let tag: Value = serde_json::from_slice(
            &fs::read(carried.join("data/towers_wg/tags/worldgen/biome/has_tower.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(tag, json!({"values": ["minecraft:plains"]}));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A jar entry whose name escapes through a Windows separator writes
    /// nothing: not into `carry`, not into `outside`, nowhere.
    #[test]
    fn entry_names_cannot_escape_the_output() {
        let dir =
            std::env::temp_dir().join(format!("horizon-port-pack-esc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("evil-1.0.jar");
        let mut zip = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
        for (name, text) in [
            (
                "META-INF/neoforge.mods.toml",
                "license = \"MIT\"\n[[mods]]\nmodId = \"evil\"\nversion = \"1.0\"\n",
            ),
            ("data/evil/recipe/..\\..\\..\\evil.json", "{}"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let info = crate::jar::read(&jar).unwrap();
        let out = dir.join("work/mods/evil");
        let pack = Pack {
            loaded: &|_| true,
            mods: &|_| false,
        };
        let report = carry(&jar, &info, &out, None, &|_| true, &pack, &BTreeMap::new()).unwrap();
        assert_eq!(report.copied, 0);
        assert!(report.outside.is_empty());
        assert!(!dir.join("evil.json").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
