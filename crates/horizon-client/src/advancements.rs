//! Advancements: the tree loaded from `data/*/advancement`, the player's
//! progress (`PlayerAdvancements`) and the predicates criteria test
//! (`EntityPredicate`, `ItemPredicate`, `LocationPredicate`, the loot
//! conditions advancements use).
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Task,
    Goal,
    Challenge,
}
#[derive(Clone, Debug)]
pub struct Display {
    pub icon: String,
    pub title: Value,
    pub description: Value,
    pub frame: Frame,
    pub background: Option<String>,
    pub show_toast: bool,
    pub announce: bool,
    pub hidden: bool,
}
#[derive(Clone, Debug)]
pub struct Criterion {
    pub name: String,
    pub trigger: String,
    pub conditions: Value,
}
#[derive(Clone, Debug, Default)]
pub struct Rewards {
    pub recipes: Vec<String>,
    pub experience: i32,
    pub loot: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Advancement {
    pub id: String,
    pub parent: Option<String>,
    pub display: Option<Display>,
    pub criteria: Vec<Criterion>,
    pub requirements: Vec<Vec<String>>,
    pub rewards: Rewards,
}
/// Every advancement, indexed by id and by trigger.
#[derive(Default)]
pub struct Tree {
    pub list: Vec<Advancement>,
    pub by_id: HashMap<String, usize>,
    /// trigger id -> (advancement index, criterion index).
    pub by_trigger: HashMap<String, Vec<(usize, usize)>>,
    /// `recipe_unlocked` criteria by the recipe they name (`index_recipes`);
    /// those naming none in `recipes_any`.
    by_recipe: HashMap<String, Vec<(usize, usize)>>,
    recipes_any: Vec<(usize, usize)>,
}
fn frame(v: &Value) -> Frame {
    match v.as_str() {
        Some("goal") => Frame::Goal,
        Some("challenge") => Frame::Challenge,
        _ => Frame::Task,
    }
}
impl Tree {
    /// [`Tree::load`], shared while anyone holds it: a server's advancement
    /// packets (`ServerAdvancements`) and its game's criteria read the same
    /// files, and each kept its own copy (7 MB). A world whose data packs bring
    /// advancements gets its own tree ([`Tree::load_with`]).
    pub fn shared(data: &Path, overlay: &horizon_world::loot::PackOverlay) -> Arc<Tree> {
        if overlay.kind("advancement").next().is_some() {
            return Arc::new(Tree::load_with(data, overlay));
        }
        static TREES: Mutex<Vec<(PathBuf, Weak<Tree>)>> = Mutex::new(Vec::new());
        // Held while loading: a second reader waits for the first one's tree.
        let mut trees = TREES.lock().unwrap_or_else(|e| e.into_inner());
        trees.retain(|(_, tree)| tree.strong_count() > 0);
        if let Some(tree) = trees
            .iter()
            .find(|(path, _)| path == data)
            .and_then(|(_, tree)| tree.upgrade())
        {
            return tree;
        }
        let tree = Arc::new(Tree::load(data));
        trees.push((data.to_path_buf(), Arc::downgrade(&tree)));
        tree
    }
    /// Loads `<data>/<namespace>/advancement/**/*.json`.
    pub fn load(data: &Path) -> Tree {
        Self::load_with(data, &horizon_world::loot::PackOverlay::NONE)
    }
    /// The tree with a world's data packs' advancement files over the game's
    /// own: the top pack's file for each id replaces it.
    pub fn load_with(data: &Path, overlay: &horizon_world::loot::PackOverlay) -> Tree {
        let mut tree = Tree::default();
        let Ok(namespaces) = std::fs::read_dir(data) else {
            return tree;
        };
        let mut files = Vec::new();
        for ns in namespaces.flatten() {
            let root = ns.path().join("advancement");
            collect(&root, &root, &ns.file_name().to_string_lossy(), &mut files);
        }
        // One JSON per id, the packs' over the game's own.
        let mut json: BTreeMap<String, Value> = BTreeMap::new();
        files.sort();
        for (id, path) in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            json.insert(id, v);
        }
        for (id, files) in overlay.kind("advancement") {
            match serde_json::from_slice::<Value>(horizon_world::loot::PackOverlay::top(files)) {
                Ok(v) => {
                    json.insert(id.to_owned(), v);
                }
                Err(e) => log::warn!("Horizon: pack advancement {id} not loaded: {e}"),
            }
        }
        for (id, v) in json {
            if let Some(a) = parse(&id, &v) {
                let index = tree.list.len();
                for (ci, c) in a.criteria.iter().enumerate() {
                    tree.by_trigger
                        .entry(c.trigger.clone())
                        .or_default()
                        .push((index, ci));
                }
                tree.by_id.insert(id, index);
                tree.list.push(a);
            }
        }
        tree.index_recipes();
        tree
    }
    /// Index the `recipe_unlocked` criteria by recipe: a grant that unlocks
    /// twenty recipes tested every one of the ~1,300 recipe advancements'
    /// criteria for each (several ms in one tick).
    pub fn index_recipes(&mut self) {
        self.by_recipe.clear();
        self.recipes_any.clear();
        for &(ai, ci) in self
            .by_trigger
            .get("minecraft:recipe_unlocked")
            .into_iter()
            .flatten()
        {
            match self.list[ai].criteria[ci].conditions["recipes"].as_str() {
                Some(r) => self
                    .by_recipe
                    .entry(r.to_owned())
                    .or_default()
                    .push((ai, ci)),
                None => self.recipes_any.push((ai, ci)),
            }
        }
    }
    /// The `recipe_unlocked` criteria that could pass for `recipe`, in tree
    /// order (the others name another recipe).
    pub fn recipe_criteria(&self, recipe: &str) -> Vec<(usize, usize)> {
        let mut out = self.recipes_any.clone();
        out.extend(self.by_recipe.get(recipe).into_iter().flatten());
        out.sort_unstable();
        out
    }
    pub fn get(&self, id: &str) -> Option<&Advancement> {
        self.by_id.get(id).map(|&i| &self.list[i])
    }
}

/// The `inventory_changed` criteria by the items they need. A criterion whose
/// item predicates include one limited to some items (an id, a `#tag`, a
/// list) can only pass with one of those in the inventory, so an inventory
/// change tests only the criteria of the items it holds, plus those that
/// need no particular item. The same answers as testing them all: the ~1,300
/// recipe advancements each have one, and testing every one of them cost a
/// player 1–4 ms on every tick its inventory changed.
#[derive(Default)]
pub struct InventoryIndex {
    by_item: HashMap<String, Vec<(usize, usize)>>,
    any: Vec<(usize, usize)>,
}
impl InventoryIndex {
    /// `tag` gives a `#tag`'s item ids (none for a tag that doesn't exist,
    /// which matches nothing).
    pub fn build<'t>(tree: &Tree, tag: impl Fn(&str) -> Vec<&'t str>) -> Self {
        fn ids<'t>(v: &Value, tag: &impl Fn(&str) -> Vec<&'t str>, out: &mut Vec<String>) {
            match v {
                Value::String(s) => match s.strip_prefix('#') {
                    Some(t) => out.extend(tag(t).into_iter().map(str::to_owned)),
                    None => out.push(s.clone()),
                },
                Value::Array(a) => a.iter().for_each(|x| ids(x, tag, out)),
                _ => {}
            }
        }
        let mut index = Self::default();
        let Some(list) = tree.by_trigger.get("minecraft:inventory_changed") else {
            return index;
        };
        for &(ai, ci) in list {
            let items = &tree.list[ai].criteria[ci].conditions["items"];
            // `id_matches`: only a string or a list limits the items.
            let limited = items.as_array().and_then(|a| {
                a.iter()
                    .map(|p| &p["items"])
                    .find(|v| v.is_string() || v.is_array())
            });
            match limited {
                Some(v) => {
                    let mut out = Vec::new();
                    ids(v, &tag, &mut out);
                    out.sort();
                    out.dedup();
                    for id in out {
                        index.by_item.entry(id).or_default().push((ai, ci));
                    }
                }
                None => index.any.push((ai, ci)),
            }
        }
        index
    }
    /// The criteria an inventory holding these item ids could pass, in
    /// tree order.
    pub fn candidates<'a>(&self, held: impl IntoIterator<Item = &'a str>) -> Vec<(usize, usize)> {
        let mut out = self.any.clone();
        for id in held {
            if let Some(list) = self.by_item.get(id) {
                out.extend_from_slice(list);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}
fn collect(root: &Path, dir: &Path, ns: &str, out: &mut Vec<(String, std::path::PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(root, &p, ns, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            let rel = p.strip_prefix(root).unwrap_or(&p).with_extension("");
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.push((format!("{ns}:{rel}"), p));
        }
    }
}
fn parse(id: &str, v: &Value) -> Option<Advancement> {
    let criteria: Vec<Criterion> = v["criteria"]
        .as_object()?
        .iter()
        .map(|(name, c)| Criterion {
            name: name.clone(),
            trigger: c["trigger"].as_str().unwrap_or("").to_owned(),
            conditions: c["conditions"].clone(),
        })
        .collect();
    let requirements = match v["requirements"].as_array() {
        Some(r) => r
            .iter()
            .map(|group| {
                group
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .collect(),
        // AdvancementRequirements.allOf: every criterion.
        None => criteria.iter().map(|c| vec![c.name.clone()]).collect(),
    };
    let d = &v["display"];
    let display = d.is_object().then(|| Display {
        icon: d["icon"]["id"]
            .as_str()
            .unwrap_or("minecraft:air")
            .to_owned(),
        title: d["title"].clone(),
        description: d["description"].clone(),
        frame: frame(&d["frame"]),
        background: d["background"].as_str().map(str::to_owned),
        show_toast: d["show_toast"].as_bool().unwrap_or(true),
        announce: d["announce_to_chat"].as_bool().unwrap_or(true),
        hidden: d["hidden"].as_bool().unwrap_or(false),
    });
    let r = &v["rewards"];
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    Some(Advancement {
        id: id.to_owned(),
        parent: v["parent"].as_str().map(str::to_owned),
        display,
        criteria,
        requirements,
        rewards: Rewards {
            recipes: strings(&r["recipes"]),
            experience: r["experience"].as_i64().unwrap_or(0) as i32,
            loot: strings(&r["loot"]),
        },
    })
}

/// `CriterionProgress`'s date as saved, "yyyy-MM-dd HH:mm:ss Z", in UTC.
pub fn date_text(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86400);
    let rem = unix_secs.rem_euclid(86400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// A saved date back to milliseconds since the epoch (the network form of
/// `CriterionProgress`), the zone offset applied.
pub fn date_millis(text: &str) -> Option<i64> {
    let (date, rest) = text.trim().split_once(' ')?;
    let (time, zone) = rest.split_once(' ').unwrap_or((rest, "+0000"));
    let mut d = date.split('-').map(|v| v.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|v| v.parse::<i64>().ok());
    let (h, min, sec) = (t.next()??, t.next()??, t.next()??);
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let zone = zone.trim_start_matches(['+', '-']);
    let offset =
        zone.get(0..2)?.parse::<i64>().ok()? * 3600 + zone.get(2..4)?.parse::<i64>().ok()? * 60;
    // Days from civil (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86400 + h * 3600 + min * 60 + sec - sign * offset) * 1000)
}

/// The player's progress: advancement -> criterion -> obtained date.
#[derive(Default, Clone)]
pub struct Progress {
    pub criteria: BTreeMap<String, BTreeMap<String, String>>,
}
impl Progress {
    pub fn has(&self, adv: &str, criterion: &str) -> bool {
        self.criteria
            .get(adv)
            .is_some_and(|m| m.contains_key(criterion))
    }
    /// `AdvancementProgress.isDone`.
    pub fn done(&self, a: &Advancement) -> bool {
        !a.requirements.is_empty()
            && a.requirements
                .iter()
                .all(|group| group.iter().any(|c| self.has(&a.id, c)))
    }
    /// Completed requirement groups and their count (the screen's tooltip).
    pub fn counts(&self, a: &Advancement) -> (usize, usize) {
        let done = a
            .requirements
            .iter()
            .filter(|g| g.iter().any(|c| self.has(&a.id, c)))
            .count();
        (done, a.requirements.len())
    }
    pub fn grant(&mut self, adv: &str, criterion: &str, date: &str) -> bool {
        let m = self.criteria.entry(adv.to_owned()).or_default();
        if m.contains_key(criterion) {
            return false;
        }
        m.insert(criterion.to_owned(), date.to_owned());
        true
    }
    pub fn revoke(&mut self, adv: &str, criterion: &str) -> bool {
        self.criteria
            .get_mut(adv)
            .is_some_and(|m| m.remove(criterion).is_some())
    }
    pub fn from_json(v: &Value) -> Progress {
        let mut p = Progress::default();
        if let Some(o) = v.as_object() {
            for (adv, entry) in o {
                if adv == "DataVersion" {
                    continue;
                }
                if let Some(c) = entry["criteria"].as_object() {
                    let m = p.criteria.entry(adv.clone()).or_default();
                    for (k, d) in c {
                        m.insert(k.clone(), d.as_str().unwrap_or("").to_owned());
                    }
                }
            }
        }
        p
    }
    pub fn to_json(&self, tree: &Tree, data_version: i32) -> Value {
        let mut o = serde_json::Map::new();
        for (adv, m) in &self.criteria {
            if m.is_empty() {
                continue;
            }
            let done = tree.get(adv).is_some_and(|a| self.done(a));
            o.insert(
                adv.clone(),
                serde_json::json!({ "criteria": m, "done": done }),
            );
        }
        o.insert("DataVersion".into(), data_version.into());
        Value::Object(o)
    }
}

/// What predicates may ask about the world.
pub trait World {
    fn dimension(&self) -> &str;
    fn biome(&self, pos: [i32; 3]) -> String;
    fn structures(&self, pos: [i32; 3]) -> Vec<String>;
    /// Block name and properties.
    fn block(&self, pos: [i32; 3]) -> (String, BTreeMap<String, String>);
    fn fluid(&self, pos: [i32; 3]) -> Option<String>;
    fn can_see_sky(&self, pos: [i32; 3]) -> bool;
    fn light(&self, pos: [i32; 3]) -> i32;
    fn smokey(&self, pos: [i32; 3]) -> bool;
    fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool;
}
/// An item stack as predicates see it.
#[derive(Clone, Debug, Default)]
pub struct ItemView {
    pub id: String,
    pub count: u32,
    pub damage: u32,
    pub enchantments: Vec<(String, u32)>,
    pub potion: Option<String>,
    pub custom_name: bool,
}
/// An entity as predicates see it.
#[derive(Clone, Debug, Default)]
pub struct EntityView {
    pub kind: String,
    pub position: [f64; 3],
    pub on_fire: bool,
    pub sneaking: bool,
    pub sprinting: bool,
    pub swimming: bool,
    pub flying: bool,
    pub baby: bool,
    pub on_ground: bool,
    /// mainhand, offhand, head, chest, legs, feet, body.
    pub equipment: [Option<ItemView>; 7],
    pub vehicle: Option<Box<EntityView>>,
    /// Player: what it looks at within 100 blocks (`PlayerPredicate.lookingAt`).
    pub looking_at: Option<Box<EntityView>>,
    pub effects: Vec<(String, i32, i32)>,
    /// Player: game mode name.
    pub gamemode: Option<String>,
    /// Lightning: entities it struck.
    pub struck: Vec<EntityView>,
    /// Lightning: blocks it set on fire (`LightningBolt.blocksSetOnFire`).
    pub blocks_set_on_fire: i32,
    pub fishing_open_water: Option<bool>,
    pub variant: Option<String>,
    /// The data components the entity reports (`Entity.get`): id and value.
    pub components: Vec<(String, String)>,
}
fn range_f(v: &Value, x: f64) -> bool {
    match v {
        Value::Null => true,
        Value::Number(n) => n.as_f64().is_some_and(|n| (n - x).abs() < 1e-9),
        Value::Object(o) => {
            o.get("min").and_then(Value::as_f64).is_none_or(|m| x >= m)
                && o.get("max").and_then(Value::as_f64).is_none_or(|m| x <= m)
        }
        _ => false,
    }
}
pub fn range_i(v: &Value, x: i64) -> bool {
    range_f(v, x as f64)
}
/// A registry id, `#tag`, or list matched against one id.
pub fn id_matches(v: &Value, registry: &str, id: &str, w: &dyn World) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => match s.strip_prefix('#') {
            Some(tag) => w.in_tag(registry, tag, id),
            None => s == id,
        },
        Value::Array(a) => a.iter().any(|x| id_matches(x, registry, id, w)),
        _ => false,
    }
}
/// `PotionsPredicate`: the potion in a `HolderSet`, plus a `CollectionPredicate`
/// over the potion's effects (`contains`, `count`, `size`).
pub fn potions(v: &Value, potion: Option<&str>, w: &dyn World) -> bool {
    if v.is_null() {
        return true;
    }
    if !v["potions"].is_null() && !potion.is_some_and(|p| id_matches(&v["potions"], "potion", p, w))
    {
        return false;
    }
    let c = &v["effects"];
    if c.is_null() {
        return true;
    }
    let effects: Vec<(String, i32, i32)> = potion
        .and_then(crate::brewing::potion)
        .map(|d| {
            d.effects
                .iter()
                .map(|(id, dur, amp)| (format!("minecraft:{id}"), *amp, *dur))
                .collect()
        })
        .unwrap_or_default();
    // `MobEffectsPredicate.test`: the one listed effect, amplifier and duration.
    let test = |pred: &Value, e: &(String, i32, i32)| {
        pred.as_object().is_some_and(|m| {
            m.get(&e.0).is_some_and(|want| {
                range_i(&want["amplifier"], i64::from(e.1))
                    && range_i(&want["duration"], i64::from(e.2))
                    && want["ambient"].as_bool().is_none_or(|b| !b)
                    && want["visible"].as_bool().is_none_or(|b| b)
            })
        })
    };
    c["contains"]
        .as_array()
        .is_none_or(|l| l.iter().all(|p| effects.iter().any(|e| test(p, e))))
        && c["count"].as_array().is_none_or(|l| {
            l.iter().all(|entry| {
                let n = effects.iter().filter(|e| test(&entry["test"], e)).count();
                range_i(&entry["count"], n as i64)
            })
        })
        && range_i(&c["size"], effects.len() as i64)
}
/// `ItemPredicate`.
pub fn item(v: &Value, it: &ItemView, w: &dyn World) -> bool {
    if v.is_null() {
        return true;
    }
    if !id_matches(&v["items"], "item", &it.id, w) {
        return false;
    }
    if !range_i(&v["count"], i64::from(it.count)) {
        return false;
    }
    // Data components: exact matches on the few the data uses.
    let comps = &v["components"];
    if let Some(p) = comps["minecraft:potion_contents"].as_str()
        && it.potion.as_deref() != Some(p)
    {
        return false;
    }
    if let Some(d) = comps["minecraft:damage"].as_u64()
        && u64::from(it.damage) != d
    {
        return false;
    }
    let preds = &v["predicates"];
    if let Some(list) = preds["minecraft:enchantments"].as_array() {
        for e in list {
            let ok = it.enchantments.iter().any(|(id, lvl)| {
                id_matches(&e["enchantments"], "enchantment", id, w)
                    && range_i(&e["levels"], i64::from(*lvl))
            });
            if !ok {
                return false;
            }
        }
    }
    if let Some(d) = preds["minecraft:damage"].as_object() {
        let damage = Value::Object(d.clone());
        if !range_i(&damage["damage"], i64::from(it.damage)) {
            return false;
        }
    }
    true
}
/// `BlockPredicate`.
pub fn block(v: &Value, pos: [i32; 3], w: &dyn World) -> bool {
    if v.is_null() {
        return true;
    }
    let (name, props) = w.block(pos);
    if !id_matches(&v["blocks"], "block", &name, w) {
        return false;
    }
    if let Some(state) = v["state"].as_object() {
        for (k, want) in state {
            let have = props.get(k).cloned().unwrap_or_default();
            let ok = match want {
                Value::String(s) => have == *s,
                Value::Bool(b) => have == b.to_string(),
                Value::Number(n) => have == n.to_string(),
                Value::Object(_) => have.parse::<f64>().is_ok_and(|h| range_f(want, h)),
                _ => false,
            };
            if !ok {
                return false;
            }
        }
    }
    true
}
/// `LocationPredicate`.
pub fn location(v: &Value, p: [f64; 3], w: &dyn World) -> bool {
    if v.is_null() {
        return true;
    }
    let pos = [
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32,
    ];
    let posv = &v["position"];
    if !range_f(&posv["x"], p[0]) || !range_f(&posv["y"], p[1]) || !range_f(&posv["z"], p[2]) {
        return false;
    }
    if let Some(d) = v["dimension"].as_str()
        && d != w.dimension()
    {
        return false;
    }
    if !v["biomes"].is_null() && !id_matches(&v["biomes"], "worldgen/biome", &w.biome(pos), w) {
        return false;
    }
    if !v["structures"].is_null() {
        let here = w.structures(pos);
        if !here
            .iter()
            .any(|s| id_matches(&v["structures"], "worldgen/structure", s, w))
        {
            return false;
        }
    }
    if let Some(b) = v["can_see_sky"].as_bool()
        && b != w.can_see_sky(pos)
    {
        return false;
    }
    if let Some(b) = v["smokey"].as_bool()
        && b != w.smokey(pos)
    {
        return false;
    }
    if !v["light"].is_null() && !range_i(&v["light"]["light"], i64::from(w.light(pos))) {
        return false;
    }
    if !v["block"].is_null() && !block(&v["block"], pos, w) {
        return false;
    }
    if !v["fluid"].is_null() {
        let Some(f) = w.fluid(pos) else {
            return false;
        };
        if !id_matches(&v["fluid"]["fluids"], "fluid", &f, w) {
            return false;
        }
    }
    true
}
/// `DataComponentExactPredicate.test` on an entity: every expected
/// component equals what the entity reports (absent ones never match).
fn components(expected: &Value, have: &[(String, String)]) -> bool {
    let full = |id: &str| {
        if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        }
    };
    expected.as_object().is_some_and(|o| {
        o.iter().all(|(id, want)| {
            let id = full(id);
            want.as_str().is_some_and(|want| {
                have.iter()
                    .any(|(k, v)| full(k) == id && full(v) == full(want))
            })
        })
    })
}
/// `DistancePredicate.matches` between two points.
pub fn distance(v: &Value, a: [f64; 3], b: [f64; 3]) -> bool {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    range_f(&v["x"], d[0].abs())
        && range_f(&v["y"], d[1].abs())
        && range_f(&v["z"], d[2].abs())
        && range_f(&v["horizontal"], (d[0] * d[0] + d[2] * d[2]).sqrt())
        && range_f(
            &v["absolute"],
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt(),
        )
}
/// `EntityPredicate` (26.x keyed sub-predicates).
pub fn entity(v: &Value, e: &EntityView, w: &dyn World, origin: [f64; 3]) -> bool {
    if v.is_null() {
        return true;
    }
    let Some(o) = v.as_object() else {
        return false;
    };
    for (key, p) in o {
        let ok = match key.as_str() {
            "minecraft:entity_type" | "type" => id_matches(p, "entity_type", &e.kind, w),
            "minecraft:location" | "location" => location(p, e.position, w),
            "minecraft:stepping_on" => {
                location(p, [e.position[0], e.position[1] - 0.5, e.position[2]], w)
            }
            "minecraft:distance" | "distance" => distance(p, e.position, origin),
            "minecraft:flags" | "flags" => {
                let f = |k: &str, have: bool| p[k].as_bool().is_none_or(|want| want == have);
                f("is_on_fire", e.on_fire)
                    && f("is_sneaking", e.sneaking)
                    && f("is_sprinting", e.sprinting)
                    && f("is_swimming", e.swimming)
                    && f("is_flying", e.flying)
                    && f("is_baby", e.baby)
                    && f("is_on_ground", e.on_ground)
            }
            "minecraft:equipment" | "equipment" => {
                let slots = [
                    "mainhand", "offhand", "head", "chest", "legs", "feet", "body",
                ];
                slots.iter().enumerate().all(|(i, s)| {
                    p[*s].is_null()
                        || e.equipment[i]
                            .as_ref()
                            .is_some_and(|it| item(&p[*s], it, w))
                })
            }
            "minecraft:vehicle" | "vehicle" => e
                .vehicle
                .as_ref()
                .is_some_and(|veh| entity(p, veh, w, origin)),
            "minecraft:effects" | "effects" => p.as_object().is_some_and(|m| {
                m.iter().all(|(id, want)| {
                    e.effects.iter().any(|(eid, amp, dur)| {
                        eid == id
                            && range_i(&want["amplifier"], i64::from(*amp))
                            && range_i(&want["duration"], i64::from(*dur))
                    })
                })
            }),
            "minecraft:type_specific/player" => {
                p["gamemode"].as_array().is_none_or(|modes| {
                    e.gamemode
                        .as_deref()
                        .is_some_and(|g| modes.iter().any(|m| m.as_str() == Some(g)))
                }) && (p["looking_at"].is_null()
                    || e.looking_at
                        .as_ref()
                        .is_some_and(|t| entity(&p["looking_at"], t, w, origin)))
            }
            "minecraft:type_specific/lightning" => {
                range_i(&p["blocks_set_on_fire"], i64::from(e.blocks_set_on_fire))
                    && (p["entity_struck"].is_null()
                        || e.struck
                            .iter()
                            .any(|s| entity(&p["entity_struck"], s, w, origin)))
            }
            "minecraft:type_specific/fishing_hook" => p["in_open_water"]
                .as_bool()
                .is_none_or(|want| e.fishing_open_water == Some(want)),
            "minecraft:components" => components(p, &e.components),
            "minecraft:predicates" => true,
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}
/// `KilledByArrowTrigger.TriggerInstance.matches`: the weapon, each listed
/// victim condition matched by a distinct victim, and the count of entity types.
pub fn killed_by_arrow(
    c: &Value,
    me: &EntityView,
    victims: &[EntityView],
    weapon: Option<&ItemView>,
    w: &dyn World,
) -> bool {
    fn ctx<'a>(e: &'a EntityView, me: &EntityView) -> Entities<'a> {
        Entities {
            this: Some(e),
            origin: me.position,
            tool: None,
            block_pos: None,
        }
    }
    if !conditions(&c["player"], &ctx(me, me), w) {
        return false;
    }
    if !c["fired_from_weapon"].is_null()
        && !weapon.is_some_and(|it| item(&c["fired_from_weapon"], it, w))
    {
        return false;
    }
    let mut left: Vec<&EntityView> = victims.iter().collect();
    for want in c["victims"].as_array().into_iter().flatten() {
        let Some(i) = left.iter().position(|e| conditions(want, &ctx(e, me), w)) else {
            return false;
        };
        left.remove(i);
    }
    let kinds: BTreeSet<&str> = victims.iter().map(|e| e.kind.as_str()).collect();
    range_i(&c["unique_entity_types"], kinds.len() as i64)
}
/// A damage event as `DamagePredicate` sees it.
#[derive(Clone, Debug, Default)]
pub struct DamageView {
    /// The damage type id (`minecraft:arrow`).
    pub kind: String,
    /// `DamageSource.getDirectEntity` and `getEntity`.
    pub direct: Option<EntityView>,
    pub source: Option<EntityView>,
    /// `DamageSource.isDirect`: the direct entity is the causing one.
    pub is_direct: bool,
    /// The damage before and after the shield took its share.
    pub original: f32,
    pub actual: f32,
    pub blocked: bool,
}
/// `DamagePredicate`, with its `DamageSourcePredicate` as `type`.
pub fn damage(v: &Value, d: &DamageView, w: &dyn World, origin: [f64; 3]) -> bool {
    if v.is_null() {
        return true;
    }
    let by = |key: &str, e: &Option<EntityView>| {
        v[key].is_null() || e.as_ref().is_some_and(|e| entity(&v[key], e, w, origin))
    };
    if !range_f(&v["dealt"], f64::from(d.original))
        || !range_f(&v["taken"], f64::from(d.actual))
        || !by("source_entity", &d.source)
        || v["blocked"].as_bool().is_some_and(|b| b != d.blocked)
    {
        return false;
    }
    damage_source(&v["type"], d, w, origin)
}
/// `DamageSourcePredicate`: the damage type's tags and the entities behind it.
pub fn damage_source(ty: &Value, d: &DamageView, w: &dyn World, origin: [f64; 3]) -> bool {
    if ty.is_null() {
        return true;
    }
    let tags_ok = ty["tags"].as_array().into_iter().flatten().all(|t| {
        let tag = t["id"].as_str().unwrap_or("");
        let tag = tag.strip_prefix('#').unwrap_or(tag);
        w.in_tag("damage_type", tag, &d.kind) == t["expected"].as_bool().unwrap_or(true)
    });
    let direct_ok = ty["direct_entity"].is_null()
        || d.direct
            .as_ref()
            .is_some_and(|e| entity(&ty["direct_entity"], e, w, origin));
    let source_ok = ty["source_entity"].is_null()
        || d.source
            .as_ref()
            .is_some_and(|e| entity(&ty["source_entity"], e, w, origin));
    tags_ok && direct_ok && source_ok && ty["is_direct"].as_bool().is_none_or(|b| b == d.is_direct)
}
/// The entities a loot-style condition list may refer to.
pub struct Entities<'a> {
    pub this: Option<&'a EntityView>,
    pub origin: [f64; 3],
    pub tool: Option<&'a ItemView>,
    pub block_pos: Option<[i32; 3]>,
}
/// Conditions shared by item/entity interactions and thrown-item pickups.
pub fn item_entity(
    v: &Value,
    player: &EntityView,
    target: &EntityView,
    stack: &ItemView,
    w: &dyn World,
) -> bool {
    let pctx = Entities {
        this: Some(player),
        origin: player.position,
        tool: None,
        block_pos: None,
    };
    let ectx = Entities {
        this: Some(target),
        ..pctx
    };
    conditions(&v["player"], &pctx, w)
        && conditions(&v["entity"], &ectx, w)
        && item(&v["item"], stack, w)
}

/// `RecipeCraftedTrigger.TriggerInstance.matches`: each ingredient predicate
/// takes a different one of the stacks used, in order.
pub fn recipe_ingredients(v: &Value, used: &[ItemView], w: &dyn World) -> bool {
    let Some(preds) = v.as_array() else {
        return true;
    };
    let mut remaining: Vec<&ItemView> = used.iter().collect();
    preds.iter().all(|p| {
        let Some(i) = remaining.iter().position(|it| item(p, it, w)) else {
            return false;
        };
        remaining.remove(i);
        true
    })
}

/// A `ContextAwarePredicate`: a list of loot conditions (or one condition).
pub fn conditions(v: &Value, ctx: &Entities, w: &dyn World) -> bool {
    match v {
        Value::Null => true,
        Value::Array(list) => list.iter().all(|c| conditions(c, ctx, w)),
        Value::Object(o) if o.contains_key("condition") || o.contains_key("type") => {
            let ty = o
                .get("condition")
                .or_else(|| o.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match ty {
                "minecraft:entity_properties" => ctx
                    .this
                    .is_some_and(|e| entity(&v["predicate"], e, w, ctx.origin)),
                "minecraft:location_check" => {
                    let base = ctx.block_pos.map_or(ctx.origin, |b| {
                        [
                            f64::from(b[0]) + 0.5,
                            f64::from(b[1]) + 0.5,
                            f64::from(b[2]) + 0.5,
                        ]
                    });
                    let off = [
                        v["offsetX"].as_f64().unwrap_or(0.0),
                        v["offsetY"].as_f64().unwrap_or(0.0),
                        v["offsetZ"].as_f64().unwrap_or(0.0),
                    ];
                    location(
                        &v["predicate"],
                        [base[0] + off[0], base[1] + off[1], base[2] + off[2]],
                        w,
                    )
                }
                "minecraft:match_block" => ctx.block_pos.is_some_and(|b| block(v, b, w)),
                "minecraft:block_state_property" => ctx.block_pos.is_some_and(|b| {
                    let mut pred = serde_json::Map::new();
                    pred.insert("blocks".into(), v["block"].clone());
                    if !v["properties"].is_null() {
                        pred.insert("state".into(), v["properties"].clone());
                    }
                    block(&Value::Object(pred), b, w)
                }),
                "minecraft:match_tool" => ctx.tool.is_some_and(|t| item(&v["predicate"], t, w)),
                "minecraft:inverted" => !conditions(&v["term"], ctx, w),
                "minecraft:all_of" => v["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .all(|t| conditions(t, ctx, w)),
                "minecraft:any_of" => v["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|t| conditions(t, ctx, w)),
                _ => false,
            }
        }
        // An inline entity predicate (old-style).
        Value::Object(_) => ctx.this.is_some_and(|e| entity(v, e, w, ctx.origin)),
        _ => false,
    }
}

/// One node of `TreeNodePosition`, the Walker layout of a display tree.
struct Place {
    adv: usize,
    parent: Option<usize>,
    previous: Option<usize>,
    child_index: i32,
    children: Vec<usize>,
    ancestor: usize,
    thread: Option<usize>,
    x: i32,
    y: f32,
    modifier: f32,
    change: f32,
    shift: f32,
}
struct Layout<'a> {
    tree: &'a Tree,
    kids: &'a [Vec<usize>],
    nodes: Vec<Place>,
}
impl Layout<'_> {
    fn build(
        &mut self,
        adv: usize,
        parent: Option<usize>,
        previous: Option<usize>,
        child_index: i32,
        depth: i32,
    ) -> usize {
        let me = self.nodes.len();
        self.nodes.push(Place {
            adv,
            parent,
            previous,
            child_index,
            children: Vec::new(),
            ancestor: me,
            thread: None,
            x: depth,
            y: -1.0,
            modifier: 0.0,
            change: 0.0,
            shift: 0.0,
        });
        let mut prev = None;
        for &c in &self.kids[adv] {
            prev = self.add_child(me, c, prev);
        }
        me
    }
    fn add_child(&mut self, me: usize, adv: usize, mut prev: Option<usize>) -> Option<usize> {
        if self.tree.list[adv].display.is_some() {
            let index = self.nodes[me].children.len() as i32 + 1;
            let depth = self.nodes[me].x + 1;
            let c = self.build(adv, Some(me), prev, index, depth);
            self.nodes[me].children.push(c);
            Some(c)
        } else {
            for &g in &self.kids[adv] {
                prev = self.add_child(me, g, prev);
            }
            prev
        }
    }
    fn first_walk(&mut self, v: usize) {
        let children = self.nodes[v].children.clone();
        let previous = self.nodes[v].previous;
        if children.is_empty() {
            self.nodes[v].y = previous.map_or(0.0, |p| self.nodes[p].y + 1.0);
            return;
        }
        let mut default_ancestor: Option<usize> = None;
        for &c in &children {
            self.first_walk(c);
            default_ancestor = Some(self.apportion(c, default_ancestor.unwrap_or(c)));
        }
        self.execute_shifts(v);
        let mid = (self.nodes[children[0]].y + self.nodes[children[children.len() - 1]].y) / 2.0;
        if let Some(p) = previous {
            self.nodes[v].y = self.nodes[p].y + 1.0;
            self.nodes[v].modifier = self.nodes[v].y - mid;
        } else {
            self.nodes[v].y = mid;
        }
    }
    fn second_walk(&mut self, v: usize, mod_sum: f32, depth: i32, mut min: f32) -> f32 {
        self.nodes[v].y += mod_sum;
        self.nodes[v].x = depth;
        min = min.min(self.nodes[v].y);
        let m = self.nodes[v].modifier;
        for c in self.nodes[v].children.clone() {
            min = self.second_walk(c, mod_sum + m, depth + 1, min);
        }
        min
    }
    fn third_walk(&mut self, v: usize, offset: f32) {
        self.nodes[v].y += offset;
        for c in self.nodes[v].children.clone() {
            self.third_walk(c, offset);
        }
    }
    fn execute_shifts(&mut self, v: usize) {
        let (mut shift, mut change) = (0.0, 0.0);
        for c in self.nodes[v].children.clone().into_iter().rev() {
            let n = &mut self.nodes[c];
            n.y += shift;
            n.modifier += shift;
            change += n.change;
            shift += n.shift + change;
        }
    }
    fn previous_or_thread(&self, v: usize) -> Option<usize> {
        self.nodes[v]
            .thread
            .or_else(|| self.nodes[v].children.first().copied())
    }
    fn next_or_thread(&self, v: usize) -> Option<usize> {
        self.nodes[v]
            .thread
            .or_else(|| self.nodes[v].children.last().copied())
    }
    fn apportion(&mut self, v: usize, mut default_ancestor: usize) -> usize {
        let Some(previous) = self.nodes[v].previous else {
            return default_ancestor;
        };
        let parent = self.nodes[v].parent.unwrap_or(v);
        let (mut vir, mut vor, mut vil) = (v, v, previous);
        let mut vol = self.nodes[parent].children[0];
        let mut sir = self.nodes[v].modifier;
        let mut sor = sir;
        let mut sil = self.nodes[vil].modifier;
        let mut sol = self.nodes[vol].modifier;
        while let (Some(l), Some(r)) = (self.next_or_thread(vil), self.previous_or_thread(vir)) {
            vil = l;
            vir = r;
            vol = self.previous_or_thread(vol).unwrap_or(vol);
            vor = self.next_or_thread(vor).unwrap_or(vor);
            self.nodes[vor].ancestor = v;
            let shift = self.nodes[vil].y + sil - (self.nodes[vir].y + sir) + 1.0;
            if shift > 0.0 {
                let a = self.ancestor_of(vil, v, default_ancestor);
                self.move_subtree(a, v, shift);
                sir += shift;
                sor += shift;
            }
            sil += self.nodes[vil].modifier;
            sir += self.nodes[vir].modifier;
            sol += self.nodes[vol].modifier;
            sor += self.nodes[vor].modifier;
        }
        if let Some(l) = self.next_or_thread(vil)
            && self.next_or_thread(vor).is_none()
        {
            self.nodes[vor].thread = Some(l);
            self.nodes[vor].modifier += sil - sor;
        } else {
            if let Some(r) = self.previous_or_thread(vir)
                && self.previous_or_thread(vol).is_none()
            {
                self.nodes[vol].thread = Some(r);
                self.nodes[vol].modifier += sir - sol;
            }
            default_ancestor = v;
        }
        default_ancestor
    }
    fn move_subtree(&mut self, me: usize, right: usize, shift: f32) {
        let subtrees = (self.nodes[right].child_index - self.nodes[me].child_index) as f32;
        if subtrees != 0.0 {
            self.nodes[right].change -= shift / subtrees;
            self.nodes[me].change += shift / subtrees;
        }
        let r = &mut self.nodes[right];
        r.shift += shift;
        r.y += shift;
        r.modifier += shift;
    }
    fn ancestor_of(&self, me: usize, other: usize, default_ancestor: usize) -> usize {
        let a = self.nodes[me].ancestor;
        let siblings = self.nodes[other]
            .parent
            .map(|p| &self.nodes[p].children[..])
            .unwrap_or(&[]);
        if siblings.contains(&a) {
            a
        } else {
            default_ancestor
        }
    }
}
impl Tree {
    /// Each advancement's children in `AdvancementTree.addAll` order: the
    /// sorted list, a parent listed after its child inserting it a pass later.
    pub fn children(&self) -> Vec<Vec<usize>> {
        let mut kids = vec![Vec::new(); self.list.len()];
        let mut inserted = vec![false; self.list.len()];
        let mut pending: Vec<usize> = (0..self.list.len()).collect();
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|&i| {
                let parent = match &self.list[i].parent {
                    None => None,
                    Some(p) => match self.by_id.get(p) {
                        Some(&p) if inserted[p] => Some(p),
                        _ => return true,
                    },
                };
                if let Some(p) = parent {
                    kids[p].push(i);
                }
                inserted[i] = true;
                false
            });
            if pending.len() == before {
                break;
            }
        }
        kids
    }
    /// `AdvancementVisibilityEvaluator`: done advancements, their
    /// ancestors, and unhidden ones within two levels of a done one.
    pub fn visible(&self, kids: &[Vec<usize>], done: impl Fn(usize) -> bool) -> Vec<bool> {
        #[derive(Clone, Copy, PartialEq)]
        enum Rule {
            Show,
            Hide,
            Same,
        }
        fn walk(
            tree: &Tree,
            kids: &[Vec<usize>],
            done: &dyn Fn(usize) -> bool,
            i: usize,
            stack: &mut Vec<Rule>,
            out: &mut [bool],
        ) -> bool {
            let me = done(i);
            let rule = match &tree.list[i].display {
                None => Rule::Hide,
                Some(_) if me => Rule::Show,
                Some(d) if d.hidden => Rule::Hide,
                Some(_) => Rule::Same,
            };
            let mut any = me;
            stack.push(rule);
            for &c in &kids[i] {
                any |= walk(tree, kids, done, c, stack, out);
            }
            let near = stack
                .iter()
                .rev()
                .take(3)
                .find(|r| **r != Rule::Same)
                .is_some_and(|r| *r == Rule::Show);
            stack.pop();
            out[i] = any || near;
            any
        }
        let mut out = vec![false; self.list.len()];
        for (i, a) in self.list.iter().enumerate() {
            if a.parent.is_none() {
                let mut stack = vec![Rule::Same; 3];
                walk(self, kids, &done, i, &mut stack, &mut out);
            }
        }
        out
    }
    /// `AdvancementTree.repositionNodes`: (column, row) of every displayed
    /// advancement under a displayed root.
    pub fn layout(&self) -> Vec<Option<(i32, f32)>> {
        let kids = self.children();
        let mut out = vec![None; self.list.len()];
        for (i, a) in self.list.iter().enumerate() {
            if a.parent.is_some() || a.display.is_none() {
                continue;
            }
            let mut l = Layout {
                tree: self,
                kids: &kids,
                nodes: Vec::new(),
            };
            let root = l.build(i, None, None, 1, 0);
            l.first_walk(root);
            let y = l.nodes[root].y;
            let min = l.second_walk(root, 0.0, 0, y);
            if min < 0.0 {
                l.third_walk(root, -min);
            }
            for n in &l.nodes {
                out[n.adv] = Some((n.x, n.y));
            }
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The criteria an inventory change and a recipe unlock test, on vanilla's tree:
    /// `cargo test -p horizon-client --lib -- --ignored --nocapture index_on_vanilla_tree`
    /// with HORIZON_RESOURCES set to the prepared resources.
    #[test]
    #[ignore = "needs the prepared resources"]
    fn index_on_vanilla_tree() {
        let Some(root) = std::env::var_os("HORIZON_RESOURCES").map(std::path::PathBuf::from) else {
            return;
        };
        let data = root.join("data");
        let t = std::time::Instant::now();
        let tree = Tree::load(&data);
        let load = t.elapsed();
        let tags = data.join("minecraft/tags/item");
        let tag_items: HashMap<String, Vec<String>> = std::fs::read_dir(&tags)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let v: Value =
                    serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
                let name = format!("minecraft:{}", e.path().file_stem()?.to_string_lossy());
                let values = v["values"]
                    .as_array()?
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_owned));
                Some((name, values.collect()))
            })
            .collect();
        let t = std::time::Instant::now();
        let index = InventoryIndex::build(&tree, |tag| {
            tag_items
                .get(tag)
                .map(|v| v.iter().map(String::as_str).collect())
                .unwrap_or_default()
        });
        let build = t.elapsed();
        let all = tree
            .by_trigger
            .get("minecraft:inventory_changed")
            .map_or(0, Vec::len);
        let held = [
            "minecraft:iron_ingot",
            "minecraft:oak_log",
            "minecraft:cobblestone",
            "minecraft:diamond_pickaxe",
            "minecraft:bread",
            "minecraft:torch",
            "minecraft:oak_planks",
            "minecraft:stick",
            "minecraft:coal",
        ];
        let t = std::time::Instant::now();
        let mut candidates = 0;
        for _ in 0..1000 {
            candidates = index.candidates(held).len();
        }
        let per_change = t.elapsed() / 1000;
        let recipes = tree
            .by_trigger
            .get("minecraft:recipe_unlocked")
            .map_or(0, Vec::len);
        let per_recipe = tree.recipe_criteria("minecraft:iron_pickaxe").len();
        eprintln!(
            "tree {load:?}; inventory_changed: {all} criteria, index built in {build:?}, \
             a 9-stack inventory tests {candidates} (looked up in {per_change:?}); \
             recipe_unlocked: {recipes} criteria, iron_pickaxe tests {per_recipe}"
        );
        assert!(candidates < all / 10);
    }
    struct Flat;
    impl World for Flat {
        fn dimension(&self) -> &str {
            "minecraft:overworld"
        }
        fn biome(&self, _: [i32; 3]) -> String {
            "minecraft:plains".into()
        }
        fn structures(&self, _: [i32; 3]) -> Vec<String> {
            vec!["minecraft:village_plains".into()]
        }
        fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
            ("minecraft:stone".into(), BTreeMap::new())
        }
        fn fluid(&self, _: [i32; 3]) -> Option<String> {
            None
        }
        fn can_see_sky(&self, _: [i32; 3]) -> bool {
            true
        }
        fn light(&self, _: [i32; 3]) -> i32 {
            15
        }
        fn smokey(&self, _: [i32; 3]) -> bool {
            false
        }
        fn in_tag(&self, _: &str, tag: &str, id: &str) -> bool {
            (tag == "minecraft:stone_tool_materials" && id == "minecraft:cobblestone")
                || (tag == "minecraft:sulfur_cube_archetype/explosive" && id == "minecraft:tnt")
                || (tag == "minecraft:is_projectile"
                    && matches!(id, "minecraft:fireball" | "minecraft:arrow"))
        }
    }
    /// Return to Sender's `killing_blow`: only the ghast's own fireball, as a
    /// projectile, kills it for the advancement; a sword or an arrow does not.
    #[test]
    fn killing_blow_needs_the_fireball() {
        let blow = serde_json::json!({
            "direct_entity": {"minecraft:entity_type": "minecraft:fireball"},
            "tags": [{"expected": true, "id": "#minecraft:is_projectile"}]
        });
        let entity = |kind: &str| EntityView {
            kind: kind.into(),
            ..Default::default()
        };
        let player = entity("minecraft:player");
        let fireball = DamageView {
            kind: "minecraft:fireball".into(),
            direct: Some(entity("minecraft:fireball")),
            source: Some(player.clone()),
            ..Default::default()
        };
        assert!(damage_source(&blow, &fireball, &Flat, [0.0; 3]));
        let sword = DamageView {
            kind: "minecraft:player_attack".into(),
            direct: Some(player.clone()),
            source: Some(player.clone()),
            is_direct: true,
            ..Default::default()
        };
        assert!(!damage_source(&blow, &sword, &Flat, [0.0; 3]));
        let arrow = DamageView {
            kind: "minecraft:arrow".into(),
            direct: Some(entity("minecraft:arrow")),
            source: Some(player),
            ..Default::default()
        };
        assert!(!damage_source(&blow, &arrow, &Flat, [0.0; 3]));
        assert!(damage_source(&Value::Null, &sword, &Flat, [0.0; 3]));
    }
    #[test]
    fn uh_oh_accepts_either_tnt_event_only_for_adult_sulfur_cubes() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../.local/horizon-client/26.3/data/minecraft/advancement/husbandry/uh_oh.json",
        );
        if !path.exists() {
            return;
        }
        let json = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let advancement = parse("minecraft:husbandry/uh_oh", &json).unwrap();
        assert_eq!(advancement.criteria.len(), 2);
        let player = EntityView {
            kind: "minecraft:player".into(),
            ..Default::default()
        };
        let mut target = EntityView {
            kind: "minecraft:sulfur_cube".into(),
            ..Default::default()
        };
        let mut stack = ItemView {
            id: "minecraft:tnt".into(),
            count: 1,
            ..Default::default()
        };
        for criterion in &advancement.criteria {
            assert!(item_entity(
                &criterion.conditions,
                &player,
                &target,
                &stack,
                &Flat
            ));
            let mut progress = Progress::default();
            progress.grant(&advancement.id, &criterion.name, "test");
            assert!(
                progress.done(&advancement),
                "either criterion must award Uh Oh"
            );
            target.baby = true;
            assert!(!item_entity(
                &criterion.conditions,
                &player,
                &target,
                &stack,
                &Flat
            ));
            target.baby = false;
            target.kind = "minecraft:pig".into();
            assert!(!item_entity(
                &criterion.conditions,
                &player,
                &target,
                &stack,
                &Flat
            ));
            target.kind = "minecraft:sulfur_cube".into();
            stack.id = "minecraft:stone".into();
            assert!(!item_entity(
                &criterion.conditions,
                &player,
                &target,
                &stack,
                &Flat
            ));
            stack.id = "minecraft:tnt".into();
        }
    }
    /// Only the criteria the held items could pass are tested; those that
    /// need no particular item always are.
    #[test]
    fn inventory_index_keeps_the_criteria_the_items_could_pass() {
        let criteria = [
            serde_json::json!({"items": [{"items": "minecraft:diamond"}]}),
            serde_json::json!({"items": [{"items": "#minecraft:planks"}]}),
            serde_json::json!({"items": [{"count": {"min": 2}}]}),
            serde_json::json!({"items": [{"count": 1}, {"items": ["minecraft:stick", "minecraft:apple"]}]}),
            serde_json::json!({"slots": {"empty": 0}}),
            serde_json::json!({"items": [{"items": "#minecraft:no_such_tag"}]}),
        ];
        let mut tree = Tree::default();
        for (i, conditions) in criteria.into_iter().enumerate() {
            tree.list.push(Advancement {
                id: format!("test:{i}"),
                parent: None,
                display: None,
                criteria: vec![Criterion {
                    name: "c".into(),
                    trigger: "minecraft:inventory_changed".into(),
                    conditions,
                }],
                requirements: vec![vec!["c".into()]],
                rewards: Rewards::default(),
            });
            tree.by_trigger
                .entry("minecraft:inventory_changed".into())
                .or_default()
                .push((i, 0));
        }
        let index = InventoryIndex::build(&tree, |tag| match tag {
            "minecraft:planks" => vec!["minecraft:oak_planks", "minecraft:birch_planks"],
            _ => Vec::new(),
        });
        let found = |held: &[&str]| -> Vec<usize> {
            index
                .candidates(held.iter().copied())
                .into_iter()
                .map(|(ai, _)| ai)
                .collect()
        };
        assert_eq!(found(&[]), [2, 4]);
        assert_eq!(found(&["minecraft:dirt"]), [2, 4]);
        assert_eq!(
            found(&["minecraft:oak_planks", "minecraft:diamond"]),
            [0, 1, 2, 4]
        );
        assert_eq!(found(&["minecraft:apple", "minecraft:apple"]), [2, 3, 4]);
    }
    /// A recipe unlocked tests only the criteria that name it.
    #[test]
    fn recipe_index_keeps_the_criteria_of_the_recipe() {
        let mut tree = Tree::default();
        for (i, conditions) in [
            serde_json::json!({"recipes": "minecraft:oak_planks"}),
            serde_json::json!({"recipes": "minecraft:stick"}),
            serde_json::json!({}),
            serde_json::json!({"recipes": "minecraft:oak_planks"}),
        ]
        .into_iter()
        .enumerate()
        {
            tree.list.push(Advancement {
                id: format!("test:{i}"),
                parent: None,
                display: None,
                criteria: vec![Criterion {
                    name: "c".into(),
                    trigger: "minecraft:recipe_unlocked".into(),
                    conditions,
                }],
                requirements: vec![vec!["c".into()]],
                rewards: Rewards::default(),
            });
            tree.by_trigger
                .entry("minecraft:recipe_unlocked".into())
                .or_default()
                .push((i, 0));
        }
        tree.index_recipes();
        assert_eq!(
            tree.recipe_criteria("minecraft:oak_planks"),
            [(0, 0), (2, 0), (3, 0)]
        );
        assert_eq!(tree.recipe_criteria("minecraft:stick"), [(1, 0), (2, 0)]);
        assert_eq!(tree.recipe_criteria("minecraft:torch"), [(2, 0)]);
    }
    #[test]
    fn bucket_axolotl_block_advancements_require_the_placed_block() {
        let ctx = Entities {
            this: None,
            origin: [0.0; 3],
            tool: None,
            block_pos: Some([0; 3]),
        };
        let stone = serde_json::json!({"type":"minecraft:match_block","blocks":"minecraft:stone"});
        let wheat = serde_json::json!({"type":"minecraft:match_block","blocks":"minecraft:wheat"});
        assert!(conditions(&stone, &ctx, &Flat));
        assert!(!conditions(&wheat, &ctx, &Flat));
        let absent = Entities {
            block_pos: None,
            ..ctx
        };
        assert!(!conditions(&stone, &absent, &Flat));
    }
    #[test]
    fn predicates_match_tags_ranges_and_locations() {
        let w = Flat;
        let stone = ItemView {
            id: "minecraft:cobblestone".into(),
            count: 3,
            ..Default::default()
        };
        assert!(item(
            &serde_json::json!({"items": "#minecraft:stone_tool_materials"}),
            &stone,
            &w
        ));
        assert!(!item(
            &serde_json::json!({"items": "minecraft:dirt"}),
            &stone,
            &w
        ));
        assert!(item(&serde_json::json!({"count": {"min": 2}}), &stone, &w));
        let player = EntityView {
            kind: "minecraft:player".into(),
            position: [0.5, 64.0, 0.5],
            ..Default::default()
        };
        let cond = serde_json::json!({
            "type": "minecraft:entity_properties",
            "entity": "this",
            "predicate": {"minecraft:location": {"structures": "minecraft:village_plains"}}
        });
        let ctx = Entities {
            this: Some(&player),
            origin: player.position,
            tool: None,
            block_pos: None,
        };
        assert!(conditions(&cond, &ctx, &w));
        let mut p = Progress::default();
        let a = parse(
            "minecraft:t",
            &serde_json::json!({"criteria": {"a": {"trigger": "minecraft:tick"}, "b": {"trigger": "minecraft:tick"}},
                "requirements": [["a", "b"]]}),
        )
        .unwrap();
        assert!(!p.done(&a));
        p.grant("minecraft:t", "b", "now");
        assert!(p.done(&a));
    }
    #[test]
    fn layout_spreads_children_like_tree_node_position() {
        let mut tree = Tree::default();
        let shown = Display {
            icon: String::new(),
            title: Value::Null,
            description: Value::Null,
            frame: Frame::Task,
            background: None,
            show_toast: true,
            announce: true,
            hidden: false,
        };
        // Sorted ids; "b" names a parent that sorts after it.
        for (id, parent) in [
            ("a", Some("r")),
            ("b", Some("e")),
            ("c", Some("a")),
            ("d", Some("a")),
            ("e", Some("r")),
            ("r", None),
        ] {
            tree.by_id.insert(id.into(), tree.list.len());
            tree.list.push(Advancement {
                id: id.into(),
                parent: parent.map(Into::into),
                display: Some(shown.clone()),
                criteria: Vec::new(),
                requirements: Vec::new(),
                rewards: Rewards::default(),
            });
        }
        let kids = tree.children();
        assert_eq!(kids[5], vec![0, 4]);
        let l = tree.layout();
        assert_eq!(l[5], Some((0, 1.25)));
        assert_eq!(l[0], Some((1, 0.5)));
        assert_eq!(l[2], Some((2, 0.0)));
        assert_eq!(l[3], Some((2, 1.0)));
        assert_eq!(l[4], Some((1, 2.0)));
        assert_eq!(l[1], Some((2, 2.0)));
    }
    #[test]
    fn saved_dates_read_back_to_their_instant() {
        for secs in [0, 951_782_400, 1_792_216_800, 4_107_542_399] {
            assert_eq!(date_millis(&date_text(secs)), Some(secs * 1000));
        }
        assert_eq!(
            date_millis("2026-10-17 07:30:00 +0200"),
            date_millis("2026-10-17 05:30:00 +0000")
        );
        assert_eq!(date_millis("not a date"), None);
    }
}
