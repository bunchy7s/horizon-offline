//! Data-driven enchantments (`data/minecraft/enchantment/*.json`): the
//! registry, ordered enchantment tags, `EnchantmentHelper` selection and cost
//! rules used by the enchanting table, display names, and evaluation of the
//! value/attribute effect components (`LevelBasedValue`, `EnchantmentValueEffect`
//! and the loot-condition subset that enchantment requirements use).
use crate::items::{ItemStack, Items};
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};

/// `Enchantment.Cost`: base + per_level_above_first * (level - 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    pub base: i32,
    pub per_level: i32,
}
impl Cost {
    fn parse(v: &Value) -> Self {
        Self {
            base: v["base"].as_i64().unwrap_or(1) as i32,
            per_level: v["per_level_above_first"].as_i64().unwrap_or(0) as i32,
        }
    }
    pub fn calculate(self, level: u32) -> i32 {
        self.base + self.per_level * (level as i32 - 1)
    }
}
#[derive(Clone, Debug)]
pub struct Enchantment {
    pub id: Arc<str>,
    /// The name the pinned English map gave at load: the fallback for a
    /// language that has no entry for the key below.
    pub description: String,
    /// `description`'s translation key (`enchantment.minecraft.<name>`).
    pub description_key: String,
    pub max_level: u32,
    pub weight: u32,
    pub anvil_cost: u32,
    pub min_cost: Cost,
    pub max_cost: Cost,
    /// `supported_items` / `primary_items`: item ids or `#tags`.
    pub supported: Vec<String>,
    pub primary: Option<Vec<String>>,
    pub exclusive: HashSet<Arc<str>>,
    pub slots: Vec<String>,
    pub effects: Value,
}
/// Item components the enchanting, anvil and smithing rules read.
#[derive(Clone, Debug, Default)]
pub struct ItemProps {
    /// `minecraft:enchantable.value`.
    pub enchantable: u32,
    /// `minecraft:repairable.items` (ids or a `#tag`).
    pub repairable: Vec<String>,
    pub glint: bool,
    pub rarity: String,
    /// `minecraft:provides_trim_material`.
    pub trim_material: Option<String>,
    /// `minecraft:brewing_fuel` present.
    pub brewing_fuel: bool,
}
#[derive(Default)]
pub struct Enchantments {
    pub list: Vec<Enchantment>,
    index: HashMap<Arc<str>, usize>,
    /// Enchantment tags in vanilla `HolderSet` order (nested tags expanded in place).
    pub tags: HashMap<String, Vec<Arc<str>>>,
    pub entity_tags: HashMap<String, HashSet<String>>,
    pub damage_tags: HashMap<String, HashSet<String>>,
    pub props: HashMap<Arc<str>, ItemProps>,
    lang: HashMap<String, String>,
}

fn string_list(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(|e| e.as_str().map(str::to_owned))
            .collect(),
        _ => vec![],
    }
}
fn read_tag_dir(dir: &Path) -> HashMap<String, Vec<String>> {
    fn walk(dir: &Path, base: &Path, out: &mut HashMap<String, Vec<String>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if p.extension().is_some_and(|x| x == "json")
                && let Ok(rel) = p.strip_prefix(base)
                && let Ok(bytes) = fs::read(&p)
                && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
            {
                let name = format!(
                    "minecraft:{}",
                    rel.with_extension("").to_string_lossy().replace('\\', "/")
                );
                let values = v["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str().or_else(|| e["id"].as_str()).map(str::to_owned))
                    .collect();
                out.insert(name, values);
            }
        }
    }
    let mut out = HashMap::new();
    walk(dir, dir, &mut out);
    out
}
/// Expand a tag keeping first-seen order (`TagLoader` builds a linked set).
fn expand_ordered(
    name: &str,
    raw: &HashMap<String, Vec<String>>,
    out: &mut Vec<String>,
    depth: u32,
) {
    if depth > 16 {
        return;
    }
    for v in raw.get(name).into_iter().flatten() {
        match v.strip_prefix('#') {
            Some(t) => expand_ordered(t, raw, out, depth + 1),
            None if !out.contains(v) => out.push(v.clone()),
            None => {}
        }
    }
}
/// Roman numerals as in `enchantment.level.N` (falls back to the number).
pub fn level_name(level: u32) -> String {
    const NAMES: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    NAMES
        .get(level.wrapping_sub(1) as usize)
        .map_or_else(|| level.to_string(), |s| (*s).to_owned())
}

impl Enchantments {
    pub fn load(root: &Path) -> Self {
        Self::load_with(root, &horizon_world::loot::PackOverlay::NONE)
    }
    /// Load from a resource root; `overlay` (a world's data packs' content
    /// files) joins the game's own enchantment tags in pack order, a
    /// `replace` file dropping the lower sources' (`TagLoader.load`).
    pub fn load_with(root: &Path, overlay: &horizon_world::loot::PackOverlay) -> Self {
        let data = root.join("data/minecraft");
        let mut raw_tags = read_tag_dir(&data.join("tags/enchantment"));
        for (id, files) in overlay.tag_kind("enchantment") {
            let slot = raw_tags.entry(id).or_default();
            for bytes in files {
                let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
                    continue;
                };
                if v["replace"].as_bool() == Some(true) {
                    slot.clear();
                }
                if let Some(values) = v["values"].as_array() {
                    slot.extend(
                        values
                            .iter()
                            .filter_map(|e| e.as_str().or_else(|| e["id"].as_str()))
                            .map(str::to_owned),
                    );
                }
            }
        }
        let tags: HashMap<String, Vec<Arc<str>>> = raw_tags
            .keys()
            .map(|k| {
                let mut v = Vec::new();
                expand_ordered(k, &raw_tags, &mut v, 0);
                (
                    k.clone(),
                    v.into_iter().map(|s| Arc::from(s.as_str())).collect(),
                )
            })
            .collect();
        let unordered = |raw: HashMap<String, Vec<String>>| -> HashMap<String, HashSet<String>> {
            raw.keys()
                .map(|k| {
                    let mut v = Vec::new();
                    expand_ordered(k, &raw, &mut v, 0);
                    (k.clone(), v.into_iter().collect())
                })
                .collect()
        };
        let entity_tags = unordered(read_tag_dir(&data.join("tags/entity_type")));
        let damage_tags = unordered(read_tag_dir(&data.join("tags/damage_type")));
        // The registry lists entries sorted by id, like the data-pack loader.
        let mut files: BTreeMap<String, Value> = BTreeMap::new();
        if let Ok(entries) = fs::read_dir(data.join("enchantment")) {
            for e in entries.flatten() {
                let p = e.path();
                if let (Some(stem), Ok(bytes)) =
                    (p.file_stem().and_then(|s| s.to_str()), fs::read(&p))
                    && let Ok(v) = serde_json::from_slice(&bytes)
                {
                    files.insert(format!("minecraft:{stem}"), v);
                }
            }
        }
        let lang: HashMap<String, String> = fs::read(root.join("assets/minecraft/lang/en_us.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(k, _)| {
                k.starts_with("enchantment.")
                    || k.starts_with("attribute.")
                    || k.starts_with("item.modifiers.")
                    || k.starts_with("effect.")
                    || k.starts_with("item.minecraft.potion")
                    || k.starts_with("item.minecraft.splash_potion")
                    || k.starts_with("item.minecraft.lingering_potion")
                    || k.starts_with("item.minecraft.tipped_arrow")
                    || k.starts_with("item.minecraft.firework_rocket.")
                    || k.starts_with("item.minecraft.firework_star.")
                    || k.starts_with("trim_")
                    || k.starts_with("potion.")
                    || k.starts_with("container.")
                    || k.starts_with("merchant.")
                    || k.starts_with("entity.minecraft.")
                    || k.starts_with("gui.recipebook.")
                    || k.starts_with("gui.togglable_slot")
                    || k.starts_with("block.minecraft.banner.")
                    || k.starts_with("recipe.toast.")
            })
            .collect();
        let mut list = Vec::new();
        for (id, v) in &files {
            let exclusive = match &v["exclusive_set"] {
                Value::String(s) if s.starts_with('#') => {
                    tags.get(&s[1..]).cloned().unwrap_or_default()
                }
                other => string_list(other)
                    .into_iter()
                    .map(|s| Arc::from(s.as_str()))
                    .collect(),
            };
            let key = v["description"]["translate"].as_str().unwrap_or("");
            list.push(Enchantment {
                id: Arc::from(id.as_str()),
                description: lang
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| id.trim_start_matches("minecraft:").replace('_', " ")),
                description_key: key.to_owned(),
                max_level: v["max_level"].as_u64().unwrap_or(1) as u32,
                weight: v["weight"].as_u64().unwrap_or(1) as u32,
                anvil_cost: v["anvil_cost"].as_u64().unwrap_or(0) as u32,
                min_cost: Cost::parse(&v["min_cost"]),
                max_cost: Cost::parse(&v["max_cost"]),
                supported: string_list(&v["supported_items"]),
                primary: v.get("primary_items").map(string_list),
                exclusive: exclusive.into_iter().collect(),
                slots: string_list(&v["slots"]),
                effects: v["effects"].clone(),
            });
        }
        let index = list
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let mut props = HashMap::new();
        if let Ok(raw) = horizon_world::item_defaults::read(root) {
            for (id, c) in raw {
                props.insert(
                    Arc::from(id.as_str()),
                    ItemProps {
                        enchantable: c["minecraft:enchantable"]["value"].as_u64().unwrap_or(0)
                            as u32,
                        repairable: string_list(&c["minecraft:repairable"]["items"]),
                        glint: c["minecraft:enchantment_glint_override"]
                            .as_bool()
                            .unwrap_or(false),
                        rarity: c["minecraft:rarity"]
                            .as_str()
                            .unwrap_or("common")
                            .to_owned(),
                        trim_material: c["minecraft:provides_trim_material"]
                            .as_str()
                            .map(str::to_owned),
                        brewing_fuel: c.get("minecraft:brewing_fuel").is_some(),
                    },
                );
            }
        }
        Self {
            list,
            index,
            tags,
            entity_tags,
            damage_tags,
            props,
            lang,
        }
    }
    pub fn get(&self, id: &str) -> Option<&Enchantment> {
        self.index.get(id).map(|&i| &self.list[i])
    }
    pub fn tag(&self, name: &str) -> &[Arc<str>] {
        self.tags
            .get(name.trim_start_matches('#'))
            .map_or(&[], Vec::as_slice)
    }
    pub fn in_tag(&self, name: &str, id: &str) -> bool {
        self.tag(name).iter().any(|e| &**e == id)
    }
    pub fn is_curse(&self, id: &str) -> bool {
        self.in_tag("minecraft:curse", id)
    }
    pub fn translate(&self, key: &str) -> Option<&str> {
        self.lang.get(key).map(String::as_str)
    }
    pub fn props(&self, item: &str) -> Option<&ItemProps> {
        self.props.get(item)
    }
    pub fn enchantable(&self, item: &str) -> u32 {
        self.props(item).map_or(0, |p| p.enchantable)
    }
    /// Whether `item` matches an id / `#tag` list.
    pub fn item_in(items: &Items, specs: &[String], item: &str) -> bool {
        specs.iter().any(|s| match s.strip_prefix('#') {
            Some(tag) => items.item_tags.get(tag).is_some_and(|t| t.contains(item)),
            None => s == item,
        })
    }
    /// `Enchantment.canEnchant` / `isSupportedItem`.
    pub fn can_enchant(&self, items: &Items, id: &str, item: &str) -> bool {
        self.get(id)
            .is_some_and(|e| Self::item_in(items, &e.supported, item))
    }
    /// `Enchantment.isPrimaryItem`.
    pub fn is_primary(&self, items: &Items, e: &Enchantment, item: &str) -> bool {
        Self::item_in(items, &e.supported, item)
            && e.primary
                .as_ref()
                .is_none_or(|p| Self::item_in(items, p, item))
    }
    /// `Enchantment.areCompatible`.
    pub fn compatible(&self, a: &str, b: &str) -> bool {
        if a == b {
            return false;
        }
        let excl = |x: &str, y: &str| self.get(x).is_some_and(|e| e.exclusive.contains(y));
        !excl(a, b) && !excl(b, a)
    }
    /// `Enchantment.getFullname` as plain text and whether it is a curse (red).
    pub fn full_name(&self, id: &str, level: u32) -> (String, bool) {
        let language = horizon_client_core::lang::current();
        self.full_name_in(language.as_deref(), id, level)
    }
    /// `full_name` against `language`; `None` (no language loaded: a
    /// dedicated server, the probes) keeps the English names loaded above.
    pub fn full_name_in(
        &self,
        language: Option<&horizon_client_core::lang::Language>,
        id: &str,
        level: u32,
    ) -> (String, bool) {
        let tr = |key: &str| language.and_then(|l| l.get(key).map(str::to_owned));
        let Some(e) = self.get(id) else {
            return (format!("{id} {}", level_name(level)), false);
        };
        let mut name = tr(&e.description_key).unwrap_or_else(|| e.description.clone());
        if level != 1 || e.max_level != 1 {
            name.push(' ');
            name.push_str(
                &tr(&format!("enchantment.level.{level}")).unwrap_or_else(|| level_name(level)),
            );
        }
        (name, self.is_curse(id))
    }
    /// Tooltip lines for a level list, in `#tooltip_order` then remaining order.
    pub fn tooltip(&self, levels: &[(Arc<str>, u32)]) -> Vec<(String, bool)> {
        let order = self.tag("minecraft:tooltip_order");
        let mut out = Vec::new();
        for id in order {
            if let Some((_, l)) = levels.iter().find(|(e, _)| e == id) {
                out.push(self.full_name(id, *l));
            }
        }
        for (id, l) in levels {
            if !order.contains(id) {
                out.push(self.full_name(id, *l));
            }
        }
        out
    }

    // ---- Enchanting table (EnchantmentHelper) ----

    /// `EnchantmentHelper.getEnchantmentCost`.
    pub fn enchantment_cost<R: RandomSource>(
        &self,
        random: &mut R,
        slot: usize,
        bookcases: i32,
        item: &str,
    ) -> i32 {
        if self.enchantable(item) == 0 {
            return 0;
        }
        let bookcases = bookcases.min(15);
        let selected = next_int(random, 8) + 1 + (bookcases >> 1) + next_int(random, bookcases + 1);
        match slot {
            0 => (selected / 3).max(1),
            1 => selected * 2 / 3 + 1,
            _ => selected.max(bookcases * 2),
        }
    }
    /// `EnchantmentHelper.getAvailableEnchantmentResults`.
    pub fn available_results(
        &self,
        items: &Items,
        value: i32,
        item: &str,
        pool: &[Arc<str>],
    ) -> Vec<(Arc<str>, u32)> {
        let book = item == "minecraft:book";
        let mut out = Vec::new();
        for id in pool {
            let Some(e) = self.get(id) else { continue };
            if !(book || self.is_primary(items, e, item)) {
                continue;
            }
            for level in (1..=e.max_level).rev() {
                if value >= e.min_cost.calculate(level) && value <= e.max_cost.calculate(level) {
                    out.push((e.id.clone(), level));
                    break;
                }
            }
        }
        out
    }
    fn weighted<R: RandomSource>(
        &self,
        random: &mut R,
        list: &[(Arc<str>, u32)],
    ) -> Option<(Arc<str>, u32)> {
        let total: i32 = list
            .iter()
            .map(|(id, _)| self.get(id).map_or(0, |e| e.weight as i32))
            .sum();
        if total <= 0 {
            return None;
        }
        let mut index = next_int(random, total);
        for (id, l) in list {
            index -= self.get(id).map_or(0, |e| e.weight as i32);
            if index < 0 {
                return Some((id.clone(), *l));
            }
        }
        None
    }
    /// `EnchantmentHelper.selectEnchantment`.
    pub fn select<R: RandomSource>(
        &self,
        items: &Items,
        random: &mut R,
        item: &str,
        cost: i32,
        pool: &[Arc<str>],
    ) -> Vec<(Arc<str>, u32)> {
        let mut results = Vec::new();
        let value = self.enchantable(item) as i32;
        if value == 0 {
            return results;
        }
        let mut cost = cost + 1 + next_int(random, value / 4 + 1) + next_int(random, value / 4 + 1);
        let span = (random.next_float() + random.next_float() - 1.0) * 0.15;
        cost = ((cost as f32 + cost as f32 * span).round() as i32).max(1);
        let mut available = self.available_results(items, cost, item, pool);
        if !available.is_empty() {
            if let Some(first) = self.weighted(random, &available) {
                results.push(first);
            }
            while next_int(random, 50) <= cost {
                if let Some((last, _)) = results.last() {
                    let last = last.clone();
                    available.retain(|(e, _)| self.compatible(&last, e));
                }
                if available.is_empty() {
                    break;
                }
                if let Some(next) = self.weighted(random, &available) {
                    results.push(next);
                }
                cost /= 2;
            }
        }
        results
    }

    // ---- Effect components ----

    /// Run `component` value effects of every enchantment on `stack` over `value`.
    pub fn modify_value(
        &self,
        stack: Option<&ItemStack>,
        component: &str,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
        value: f32,
    ) -> f32 {
        let Some(stack) = stack else { return value };
        let mut value = value;
        for (id, level) in stack.enchantments() {
            value = self.modify_value_with(id, *level, component, ctx, random, value);
        }
        value
    }
    pub fn modify_value_with(
        &self,
        id: &str,
        level: u32,
        component: &str,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
        mut value: f32,
    ) -> f32 {
        let Some(e) = self.get(id) else { return value };
        for effect in e.effects[component].as_array().into_iter().flatten() {
            if let Some(req) = effect.get("requirements")
                && !self.condition(req, level, ctx, random)
            {
                continue;
            }
            value = apply_value_effect(&effect["effect"], level, random, value);
        }
        value
    }
    /// Whether `stack` has any enchantment providing `component`.
    pub fn has_component(&self, stack: &ItemStack, component: &str) -> bool {
        stack.enchantments().iter().any(|(id, _)| {
            self.get(id)
                .is_some_and(|e| e.effects.get(component).is_some())
        })
    }
    /// `minecraft:attributes` contributions of `stack`: (attribute, id, amount, operation).
    pub fn attributes(&self, stack: &ItemStack) -> Vec<(String, String, f64, String)> {
        let mut out = Vec::new();
        for (id, level) in stack.enchantments() {
            let Some(e) = self.get(id) else { continue };
            for a in e.effects["minecraft:attributes"]
                .as_array()
                .into_iter()
                .flatten()
            {
                out.push((
                    a["attribute"].as_str().unwrap_or("").to_owned(),
                    a["id"].as_str().unwrap_or("").to_owned(),
                    f64::from(level_value(&a["amount"], *level)),
                    a["operation"].as_str().unwrap_or("add_value").to_owned(),
                ));
            }
        }
        out
    }
    /// `EnchantmentHelper.forEachModifier` as an item's tooltip lists them: the
    /// `minecraft:attributes` effects of `stack`'s enchantments as rows of
    /// (attribute, id, amount, operation, slot group), one per slot group the
    /// enchantment lists.
    pub fn modifier_rows(&self, stack: &ItemStack) -> Vec<(String, String, f64, String, String)> {
        let mut rows = Vec::new();
        for (id, level) in stack.enchantments() {
            let Some(e) = self.get(id) else { continue };
            for a in e.effects["minecraft:attributes"]
                .as_array()
                .into_iter()
                .flatten()
            {
                for slot in &e.slots {
                    rows.push((
                        a["attribute"].as_str().unwrap_or("").to_owned(),
                        a["id"].as_str().unwrap_or("").to_owned(),
                        f64::from(level_value(&a["amount"], *level)),
                        a["operation"].as_str().unwrap_or("add_value").to_owned(),
                        slot.clone(),
                    ));
                }
            }
        }
        rows
    }
    /// Sum of `add_value` amounts of one attribute from enchantments on `stack`.
    pub fn attribute_add(&self, stack: Option<&ItemStack>, attribute: &str) -> f64 {
        stack.map_or(0.0, |s| {
            self.attributes(s)
                .iter()
                .filter(|a| a.0 == attribute && a.3 == "add_value")
                .map(|a| a.2)
                .sum()
        })
    }
    fn entity_matches(&self, spec: &Value, entity: Option<&str>) -> bool {
        let Some(entity) = entity else { return false };
        match spec.as_str() {
            Some(s) => match s.strip_prefix('#') {
                Some(tag) => self
                    .entity_tags
                    .get(tag)
                    .is_some_and(|t| t.contains(entity)),
                None => s == entity,
            },
            None => string_list(spec).iter().any(|s| s == entity),
        }
    }
    /// The loot-condition subset enchantment `requirements` use.
    pub fn condition(
        &self,
        c: &Value,
        level: u32,
        ctx: &EffectContext<'_>,
        random: &mut LegacyRandom,
    ) -> bool {
        match c["type"].as_str().or(c["condition"].as_str()).unwrap_or("") {
            "minecraft:inverted" => !self.condition(&c["term"], level, ctx, random),
            "minecraft:all_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|t| self.condition(t, level, ctx, random)),
            "minecraft:any_of" => c["terms"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| self.condition(t, level, ctx, random)),
            "minecraft:match_tool" => {
                let Some(tool) = ctx.tool else { return false };
                let items = &c["predicate"]["items"];
                if items.is_null() {
                    return true;
                }
                string_list(items)
                    .iter()
                    .any(|s| match s.strip_prefix('#') {
                        Some(tag) => ctx.items.is_some_and(|i| {
                            i.item_tags.get(tag).is_some_and(|t| t.contains(tool))
                        }),
                        None => s == tool,
                    })
            }
            "minecraft:damage_source_properties" => {
                let p = &c["predicate"];
                if let Some(direct) = p["is_direct"].as_bool()
                    && direct != ctx.is_direct
                {
                    return false;
                }
                p["tags"].as_array().into_iter().flatten().all(|t| {
                    let tag = t["id"].as_str().unwrap_or("").trim_start_matches('#');
                    let has = ctx
                        .damage_type
                        .is_some_and(|d| self.damage_tags.get(tag).is_some_and(|s| s.contains(d)));
                    has == t["expected"].as_bool().unwrap_or(true)
                })
            }
            "minecraft:entity_properties" => {
                let entity = match c["entity"].as_str().unwrap_or("this") {
                    "this" => ctx.this,
                    "attacker" => ctx.attacker,
                    "direct_attacker" => ctx.direct_attacker,
                    _ => None,
                };
                let spec = &c["predicate"]["minecraft:entity_type"];
                let spec = if spec.is_null() {
                    &c["predicate"]["type"]
                } else {
                    spec
                };
                (spec.is_null() && entity.is_some() || self.entity_matches(spec, entity))
                    && movement_matches(&c["predicate"], role_state(c, ctx))
            }
            "minecraft:random_chance" => {
                let chance = match &c["chance"] {
                    Value::Object(o)
                        if o.get("type").and_then(Value::as_str)
                            == Some("minecraft:enchantment_level") =>
                    {
                        level_value(&c["chance"]["amount"], level)
                    }
                    v => v
                        .as_f64()
                        .map_or_else(|| level_value(v, level), |f| f as f32),
                };
                random.next_float() < chance
            }
            _ => false,
        }
    }
}
/// What enchantment requirement predicates can see.
#[derive(Clone, Copy, Default)]
pub struct EffectContext<'a> {
    pub items: Option<&'a Items>,
    /// The tool for `match_tool` (the enchanted item, or the ammo for ammo_use).
    pub tool: Option<&'a str>,
    /// Damage type id (e.g. `minecraft:fall`).
    pub damage_type: Option<&'a str>,
    pub is_direct: bool,
    /// Entity type ids: the victim/user ("this"), attacker and direct attacker.
    pub this: Option<&'a str>,
    pub attacker: Option<&'a str>,
    pub direct_attacker: Option<&'a str>,
    /// The direct attacker's fall distance and whether it flies (elytra or
    /// abilities), for `movement` and `flags` predicates.
    pub direct_motion: Option<(f64, bool)>,
}
/// The fall distance and flying flag known for an `entity_properties` role.
fn role_state(c: &Value, ctx: &EffectContext<'_>) -> Option<(f64, bool)> {
    match c["entity"].as_str().unwrap_or("this") {
        "direct_attacker" => ctx.direct_motion,
        _ => None,
    }
}
/// `EntityPredicate`'s `minecraft:movement` fall distance and
/// `minecraft:flags` is_flying (the parts enchantment requirements use); an
/// entity whose state is unknown matches neither.
fn movement_matches(predicate: &Value, state: Option<(f64, bool)>) -> bool {
    let fall = &predicate["minecraft:movement"]["fall_distance"];
    let flying = predicate["minecraft:flags"]["is_flying"].as_bool();
    if fall.is_null() && flying.is_none() {
        return true;
    }
    let Some((distance, is_flying)) = state else {
        return false;
    };
    // MinMaxBounds.Doubles: a number is an exact bound.
    let bound = |key: &str| {
        fall.as_f64()
            .or_else(|| fall.get(key).and_then(Value::as_f64))
    };
    // MinMaxBounds.Doubles.matches
    bound("min").is_none_or(|min| min <= distance)
        && bound("max").is_none_or(|max| distance <= max)
        && flying.is_none_or(|f| f == is_flying)
}
fn next_int<R: RandomSource>(random: &mut R, bound: i32) -> i32 {
    random.next_int_bound(bound.max(1)).unwrap_or(0)
}
/// `LevelBasedValue.calculate`.
pub fn level_value(v: &Value, level: u32) -> f32 {
    let l = level as f32;
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0) as f32,
        Value::Object(o) => {
            let f = |k: &str| o.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            match o.get("type").and_then(Value::as_str).unwrap_or("") {
                "minecraft:linear" => f("base") + f("per_level_above_first") * (l - 1.0),
                "minecraft:levels_squared" => l * l + f("added"),
                "minecraft:clamped" => level_value(&o["value"], level).clamp(f("min"), f("max")),
                "minecraft:fraction" => {
                    let d = level_value(&o["denominator"], level);
                    if d == 0.0 {
                        0.0
                    } else {
                        level_value(&o["numerator"], level) / d
                    }
                }
                "minecraft:exponent" => {
                    level_value(&o["base"], level).powf(level_value(&o["power"], level))
                }
                "minecraft:lookup" => {
                    let values = o.get("values").and_then(Value::as_array);
                    match values.and_then(|a| a.get(level as usize - 1)) {
                        Some(v) => v.as_f64().unwrap_or(0.0) as f32,
                        None => level_value(&o["fallback"], level),
                    }
                }
                _ => f("value"),
            }
        }
        _ => 0.0,
    }
}
/// `EnchantmentValueEffect.process`.
pub fn apply_value_effect(
    effect: &Value,
    level: u32,
    random: &mut LegacyRandom,
    value: f32,
) -> f32 {
    match effect["type"].as_str().unwrap_or("") {
        "minecraft:add" => value + level_value(&effect["value"], level),
        "minecraft:multiply" => value * level_value(&effect["factor"], level),
        "minecraft:set" => level_value(&effect["value"], level),
        "minecraft:remove_binomial" => {
            let p = level_value(&effect["chance"], level);
            let n = value;
            // Normal approximation for large counts (written as vanilla's negated tests).
            let large = n > 128.0 && n * p >= 20.0 && n * (1.0 - p) >= 20.0;
            let removed = if large {
                let miu = (f64::from(n) * f64::from(p)).floor();
                let sigma = (f64::from(n) * f64::from(p) * f64::from(1.0 - p)).sqrt();
                ((miu + random.next_gaussian() * sigma).round() as i32).clamp(0, n as i32)
            } else {
                let mut d = 0;
                for _ in 0..n as i32 {
                    if random.next_float() < p {
                        d += 1;
                    }
                }
                d
            };
            n - removed as f32
        }
        "minecraft:all_of" => effect["effects"]
            .as_array()
            .into_iter()
            .flatten()
            .fold(value, |v, e| apply_value_effect(e, level, random, v)),
        _ => value,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use serde_json::json;

    /// A small registry built from literal JSON matching the vanilla files.
    pub fn sample() -> Enchantments {
        let defs = [
            (
                "minecraft:sharpness",
                json!({"anvil_cost":1,"description":{"translate":"enchantment.minecraft.sharpness"},"exclusive_set":"#minecraft:exclusive_set/damage","max_cost":{"base":21,"per_level_above_first":11},"max_level":5,"min_cost":{"base":1,"per_level_above_first":11},"primary_items":"#minecraft:enchantable/melee_weapon","supported_items":"#minecraft:enchantable/sharp_weapon","weight":10,"slots":["mainhand"],"effects":{"minecraft:damage":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":1.0,"per_level_above_first":0.5}}}]}}),
            ),
            (
                "minecraft:smite",
                json!({"anvil_cost":2,"exclusive_set":"#minecraft:exclusive_set/damage","max_cost":{"base":25,"per_level_above_first":8},"max_level":5,"min_cost":{"base":5,"per_level_above_first":8},"primary_items":"#minecraft:enchantable/melee_weapon","supported_items":"#minecraft:enchantable/weapon","weight":5,"effects":{"minecraft:damage":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":2.5,"per_level_above_first":2.5}},"requirements":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"#minecraft:sensitive_to_smite"}}}]}}),
            ),
            (
                "minecraft:unbreaking",
                json!({"anvil_cost":2,"max_cost":{"base":55,"per_level_above_first":8},"max_level":3,"min_cost":{"base":5,"per_level_above_first":8},"supported_items":"#minecraft:enchantable/durability","weight":5,"effects":{"minecraft:item_damage":[{"effect":{"type":"minecraft:remove_binomial","chance":{"type":"minecraft:fraction","denominator":{"type":"minecraft:linear","base":2.0,"per_level_above_first":1.0},"numerator":{"type":"minecraft:linear","base":1.0,"per_level_above_first":1.0}}}}]}}),
            ),
            (
                "minecraft:mending",
                json!({"anvil_cost":4,"max_cost":{"base":75,"per_level_above_first":25},"max_level":1,"min_cost":{"base":25,"per_level_above_first":25},"supported_items":"#minecraft:enchantable/durability","weight":2}),
            ),
            (
                "minecraft:feather_falling",
                json!({"anvil_cost":2,"max_cost":{"base":11,"per_level_above_first":6},"max_level":4,"min_cost":{"base":5,"per_level_above_first":6},"supported_items":"#minecraft:enchantable/foot_armor","weight":5,"effects":{"minecraft:damage_protection":[{"effect":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":3.0,"per_level_above_first":3.0}},"requirements":{"type":"minecraft:damage_source_properties","predicate":{"tags":[{"expected":true,"id":"#minecraft:is_fall"},{"expected":false,"id":"#minecraft:bypasses_invulnerability"}]}}}]}}),
            ),
        ];
        let list: Vec<Enchantment> = defs
            .iter()
            .map(|(id, v)| Enchantment {
                id: Arc::from(*id),
                description: id.trim_start_matches("minecraft:").into(),
                description_key: v["description"]["translate"].as_str().unwrap_or("").into(),
                max_level: v["max_level"].as_u64().unwrap() as u32,
                weight: v["weight"].as_u64().unwrap() as u32,
                anvil_cost: v["anvil_cost"].as_u64().unwrap() as u32,
                min_cost: Cost::parse(&v["min_cost"]),
                max_cost: Cost::parse(&v["max_cost"]),
                supported: string_list(&v["supported_items"]),
                primary: v.get("primary_items").map(string_list),
                exclusive: if v["exclusive_set"].is_null() {
                    HashSet::new()
                } else {
                    [
                        Arc::from("minecraft:sharpness"),
                        Arc::from("minecraft:smite"),
                    ]
                    .into_iter()
                    .collect()
                },
                slots: vec![],
                effects: v["effects"].clone(),
            })
            .collect();
        let index = list
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let pool: Vec<Arc<str>> = [
            "minecraft:feather_falling",
            "minecraft:sharpness",
            "minecraft:smite",
            "minecraft:unbreaking",
            "minecraft:mending",
        ]
        .iter()
        .map(|s| Arc::from(*s))
        .collect();
        let mut tags = HashMap::new();
        tags.insert(
            "minecraft:in_enchanting_table".to_owned(),
            pool[..4].to_vec(),
        );
        tags.insert(
            "minecraft:tooltip_order".to_owned(),
            vec![
                Arc::from("minecraft:sharpness"),
                Arc::from("minecraft:unbreaking"),
            ],
        );
        let mut entity_tags = HashMap::new();
        entity_tags.insert(
            "minecraft:sensitive_to_smite".to_owned(),
            ["minecraft:zombie".to_owned()].into_iter().collect(),
        );
        let mut damage_tags = HashMap::new();
        damage_tags.insert(
            "minecraft:is_fall".to_owned(),
            ["minecraft:fall".to_owned()].into_iter().collect(),
        );
        let mut props = HashMap::new();
        for (item, value) in [
            ("minecraft:diamond_sword", 10),
            ("minecraft:iron_sword", 14),
            ("minecraft:book", 1),
            ("minecraft:iron_boots", 9),
        ] {
            props.insert(
                Arc::from(item),
                ItemProps {
                    enchantable: value,
                    ..Default::default()
                },
            );
        }
        Enchantments {
            list,
            index,
            tags,
            entity_tags,
            damage_tags,
            props,
            lang: HashMap::new(),
        }
    }
    pub fn sample_items() -> Items {
        let mut item_tags: HashMap<String, HashSet<String>> = HashMap::new();
        let swords = ["minecraft:diamond_sword", "minecraft:iron_sword"];
        for tag in [
            "enchantable/melee_weapon",
            "enchantable/sharp_weapon",
            "enchantable/weapon",
            "enchantable/durability",
        ] {
            item_tags.insert(
                format!("minecraft:{tag}"),
                swords.iter().map(|s| (*s).to_owned()).collect(),
            );
        }
        item_tags
            .get_mut("minecraft:enchantable/durability")
            .unwrap()
            .insert("minecraft:iron_boots".into());
        item_tags.insert(
            "minecraft:enchantable/foot_armor".into(),
            ["minecraft:iron_boots".to_owned()].into_iter().collect(),
        );
        Items::for_tests(item_tags)
    }

    #[test]
    fn level_based_values() {
        assert_eq!(
            level_value(
                &json!({"type":"minecraft:linear","base":1.0,"per_level_above_first":0.5}),
                5
            ),
            3.0
        );
        assert_eq!(
            level_value(&json!({"type":"minecraft:levels_squared","added":1.0}), 3),
            10.0
        );
        assert_eq!(
            level_value(
                &json!({"type":"minecraft:fraction","numerator":{"type":"minecraft:linear","base":1.0,"per_level_above_first":1.0},"denominator":{"type":"minecraft:linear","base":2.0,"per_level_above_first":1.0}}),
                3
            ),
            0.75
        );
        assert_eq!(level_name(4), "IV");
        assert_eq!(
            Cost {
                base: 1,
                per_level: 11
            }
            .calculate(5),
            45
        );
    }

    #[test]
    fn enchanting_cost_matches_vanilla_formula() {
        let e = sample();
        // Replays EnchantmentHelper.getEnchantmentCost draws by hand.
        for seed in [0i64, 1, 42, -7, 123456] {
            for bookcases in [0, 5, 15, 20] {
                let mut a = LegacyRandom::new(seed);
                let mut b = LegacyRandom::new(seed);
                let costs: Vec<i32> = (0..3)
                    .map(|slot| {
                        e.enchantment_cost(&mut a, slot, bookcases, "minecraft:diamond_sword")
                    })
                    .collect();
                let bc = bookcases.min(15);
                let expect: Vec<i32> = (0..3)
                    .map(|slot| {
                        let s = b.next_int_bound(8).unwrap()
                            + 1
                            + (bc >> 1)
                            + b.next_int_bound(bc + 1).unwrap();
                        match slot {
                            0 => (s / 3).max(1),
                            1 => s * 2 / 3 + 1,
                            _ => s.max(bc * 2),
                        }
                    })
                    .collect();
                assert_eq!(costs, expect);
                assert!(costs[2] <= 30);
            }
        }
        assert_eq!(
            e.enchantment_cost(&mut LegacyRandom::new(0), 0, 15, "minecraft:stick"),
            0
        );
    }

    #[test]
    fn selection_respects_costs_and_exclusivity() {
        let e = sample();
        let items = sample_items();
        let pool = e.tag("minecraft:in_enchanting_table").to_vec();
        for seed in 0..200 {
            let mut r = LegacyRandom::new(seed);
            let picked = e.select(&items, &mut r, "minecraft:diamond_sword", 30, &pool);
            assert!(!picked.is_empty());
            let has = |id: &str| picked.iter().any(|(p, _)| &**p == id);
            assert!(
                !(has("minecraft:sharpness") && has("minecraft:smite")),
                "{picked:?}"
            );
            assert!(!has("minecraft:feather_falling"));
            for (id, l) in &picked {
                assert!(*l >= 1 && *l <= e.get(id).unwrap().max_level);
            }
        }
        // Books accept every enchantment in the pool.
        let mut r = LegacyRandom::new(3);
        let available = e.available_results(&items, 20, "minecraft:book", &pool);
        assert!(
            available
                .iter()
                .any(|(id, _)| &**id == "minecraft:feather_falling")
        );
        assert!(
            e.select(&items, &mut r, "minecraft:stick", 30, &pool)
                .is_empty()
        );
    }

    #[test]
    fn value_effects_and_requirements() {
        let e = sample();
        let items = sample_items();
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.enchant("minecraft:sharpness", 5);
        let mut r = LegacyRandom::new(1);
        let ctx = EffectContext {
            items: Some(&items),
            this: Some("minecraft:zombie"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(Some(&sword), "minecraft:damage", &ctx, &mut r, 0.0),
            3.0
        );
        let mut smite = ItemStack::new("minecraft:diamond_sword", 1);
        smite.enchant("minecraft:smite", 2);
        assert_eq!(
            e.modify_value(Some(&smite), "minecraft:damage", &ctx, &mut r, 0.0),
            5.0
        );
        let pig = EffectContext {
            this: Some("minecraft:pig"),
            ..ctx
        };
        assert_eq!(
            e.modify_value(Some(&smite), "minecraft:damage", &pig, &mut r, 0.0),
            0.0
        );
        let mut boots = ItemStack::new("minecraft:iron_boots", 1);
        boots.enchant("minecraft:feather_falling", 4);
        let fall = EffectContext {
            damage_type: Some("minecraft:fall"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(
                Some(&boots),
                "minecraft:damage_protection",
                &fall,
                &mut r,
                0.0
            ),
            12.0
        );
        let lava = EffectContext {
            damage_type: Some("minecraft:lava"),
            ..Default::default()
        };
        assert_eq!(
            e.modify_value(
                Some(&boots),
                "minecraft:damage_protection",
                &lava,
                &mut r,
                0.0
            ),
            0.0
        );
        // Unbreaking III keeps about 1/4 of durability hits.
        let mut tool = ItemStack::new("minecraft:diamond_sword", 1);
        tool.enchant("minecraft:unbreaking", 3);
        let kept: f32 = (0..4000)
            .map(|_| e.modify_value(Some(&tool), "minecraft:item_damage", &ctx, &mut r, 1.0))
            .sum();
        assert!((800.0..1200.0).contains(&kept), "{kept}");
    }

    #[test]
    fn names_and_tooltip_order() {
        let e = sample();
        assert_eq!(e.full_name("minecraft:mending", 1).0, "mending");
        assert_eq!(e.full_name("minecraft:sharpness", 1).0, "sharpness I");
        let levels = vec![
            (Arc::from("minecraft:unbreaking"), 3),
            (Arc::from("minecraft:mending"), 1),
            (Arc::from("minecraft:sharpness"), 2),
        ];
        let lines: Vec<String> = e.tooltip(&levels).into_iter().map(|l| l.0).collect();
        assert_eq!(lines, ["sharpness II", "unbreaking III", "mending"]);
        assert!(!e.compatible("minecraft:sharpness", "minecraft:smite"));
        assert!(e.compatible("minecraft:sharpness", "minecraft:unbreaking"));
    }

    /// The enchantment's name and its level come from the selected language,
    /// with the English name loaded above as the fallback.
    #[test]
    fn names_follow_the_language() {
        struct Mem(HashMap<String, Vec<u8>>);
        impl horizon_client_core::lang::ResourceSource for Mem {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                self.0.get(path).cloned()
            }
        }
        let ru = Mem(HashMap::from([(
            "assets/minecraft/lang/ru_ru.json".to_owned(),
            r#"{"enchantment.minecraft.sharpness":"Острота","enchantment.level.5":"V"}"#
                .as_bytes()
                .to_vec(),
        )]));
        let ru = horizon_client_core::lang::Language::load(&[&ru], "ru_ru");
        let e = sample();
        assert_eq!(
            e.full_name_in(Some(&ru), "minecraft:sharpness", 5).0,
            "Острота V"
        );
        // mending carries no key in the fixture: the English name stays.
        assert_eq!(
            e.full_name_in(Some(&ru), "minecraft:mending", 1).0,
            "mending"
        );
    }
}
