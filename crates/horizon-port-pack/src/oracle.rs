//! The NeoForge oracle's facts (`tools/neoforge_oracle.py export`, in
//! `<work>/oracle/facts.json`): every block, item, recipe and tag the pack's
//! mods register, read from the running game, and the vanilla blocks beside
//! them. Carry-over uses them in place of guesses: a block's template is the
//! vanilla block of the same class and states whose shapes and sounds match
//! best, and the declared properties are the mod's own numbers.
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Deserialize)]
pub struct Facts {
    pub blocks: BTreeMap<String, Block>,
    pub items: BTreeMap<String, Item>,
    #[serde(default)]
    pub tags: Tags,
    /// The game's items' default components (`items.json`), so item ports leave out what their template has.
    #[serde(skip)]
    pub vanilla_items: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Tags {
    #[serde(default)]
    pub block: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub item: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct Block {
    /// The class and its superclasses, most derived first.
    pub class: Vec<String>,
    pub properties: BTreeMap<String, Vec<String>>,
    pub default: String,
    pub item: String,
    pub explosion_resistance: f32,
    pub friction: f32,
    pub speed_factor: f32,
    pub jump_factor: f32,
    #[serde(default)]
    pub block_set_type: Option<SetType>,
    #[serde(default)]
    pub wood_type: Option<String>,
    pub states: BTreeMap<String, State>,
}

#[derive(Debug, Deserialize)]
pub struct SetType {
    pub name: String,
    pub can_open_by_hand: bool,
}

#[derive(Debug, Deserialize)]
pub struct State {
    pub hardness: f32,
    pub emission: u8,
    pub requires_tool: bool,
    pub map_color_id: u8,
    pub sound: Value,
    pub outline: Value,
    pub collision: Value,
    /// What breaking the state drops, per tool (`hand`, `diamond_pickaxe`,
    /// `shears`, `silk_touch_pickaxe`...): item id → count.
    #[serde(default)]
    pub drops: BTreeMap<String, BTreeMap<String, u32>>,
}

#[derive(Debug, Deserialize)]
pub struct Item {
    pub class: String,
    pub max_stack_size: u8,
    pub rarity: String,
    #[serde(default)]
    pub block: Option<String>,
    #[serde(default)]
    pub food: Option<Value>,
    /// The default data components in 1.21.1's form; empty in facts from before they were exported.
    #[serde(default)]
    pub components: Map<String, Value>,
    #[serde(default)]
    pub attribute_modifiers: Option<Value>,
    #[serde(default)]
    pub enchantment_value: Option<u32>,
    #[serde(default)]
    pub repair_items: Option<Vec<String>>,
    #[serde(default)]
    pub armor: Option<Armor>,
}

#[derive(Debug, Deserialize)]
pub struct Armor {
    /// `head`, `chest`, `legs` or `feet`.
    pub slot: String,
    /// The armour material's id: the 26.3 equipment asset it becomes.
    #[serde(default)]
    pub material: String,
    #[serde(default)]
    pub equip_sound: Option<String>,
    #[serde(default)]
    pub layers: Vec<ArmorLayer>,
}

/// A 1.21.1 armour layer's textures: `<ns>:textures/models/armor/<name>_layer_1.png` and `_layer_2`.
#[derive(Debug, Deserialize)]
pub struct ArmorLayer {
    pub texture: String,
    pub texture_legs: String,
    #[serde(default)]
    pub dyeable: bool,
}

impl Armor {
    /// Whether carry writes the material's equipment asset: in the mod's own namespaces, from layers it can place.
    pub fn asset_carried(&self, owned: &[&str]) -> bool {
        self.material
            .split_once(':')
            .is_some_and(|(ns, _)| owned.contains(&ns))
            && self.equipment().is_some()
    }

    /// The 26.3 equipment asset of the material (`layers` by kind), and the 1.21.1 textures to copy, as (from, to) paths in the mod.
    pub fn equipment(&self) -> Option<(Value, Vec<(String, String)>)> {
        self.material.split_once(':')?;
        let mut copies = Vec::new();
        let mut kinds = Map::new();
        for (kind, legs) in [("humanoid", false), ("humanoid_leggings", true)] {
            let mut list = Vec::new();
            for layer in &self.layers {
                let texture = if legs {
                    &layer.texture_legs
                } else {
                    &layer.texture
                };
                let (ns, path) = texture.split_once(':')?;
                let file = path.strip_prefix("textures/models/armor/")?;
                let name = file.strip_suffix(if legs { "_layer_2.png" } else { "_layer_1.png" })?;
                copies.push((
                    format!("assets/{ns}/{path}"),
                    format!("assets/{ns}/textures/entity/equipment/{kind}/{name}.png"),
                ));
                let mut entry = json!({"texture": format!("{ns}:{name}")});
                if layer.dyeable {
                    entry["dyeable"] = json!({});
                }
                list.push(entry);
            }
            kinds.insert(kind.into(), Value::Array(list));
        }
        Some((json!({"layers": kinds}), copies))
    }
}

/// The vanilla item standing in for an item of a vanilla class, for the behaviour the class gives.
fn item_template(item: &Item) -> &'static str {
    let class = item.class.strip_prefix("net.minecraft.world.item.");
    match (class, item.armor.as_ref().map(|a| a.slot.as_str())) {
        (Some("SwordItem"), _) => "minecraft:iron_sword",
        (Some("PickaxeItem"), _) => "minecraft:iron_pickaxe",
        (Some("AxeItem"), _) => "minecraft:iron_axe",
        (Some("ShovelItem"), _) => "minecraft:iron_shovel",
        (Some("HoeItem"), _) => "minecraft:iron_hoe",
        (Some("ArmorItem"), Some("head")) => "minecraft:iron_helmet",
        (Some("ArmorItem"), Some("chest")) => "minecraft:iron_chestplate",
        (Some("ArmorItem"), Some("legs")) => "minecraft:iron_leggings",
        (Some("ArmorItem"), Some("feet")) => "minecraft:iron_boots",
        _ => "minecraft:stick",
    }
}

/// 1.21.1's `{"modifiers": [...]}` as 26.3's list, with attribute ids' `generic.` and `player.` dropped.
fn modifiers_26_3(v: &Value) -> Value {
    let list = v["modifiers"].as_array().cloned().unwrap_or_default();
    Value::Array(
        list.into_iter()
            .map(|mut m| {
                if let Some(t) = m["type"].as_str() {
                    let t = t.replacen(":generic.", ":", 1).replacen(":player.", ":", 1);
                    m["type"] = json!(t);
                }
                m
            })
            .collect(),
    )
}

const VANILLA_PACKAGE: &str = "net.minecraft.";

/// A state key with its properties sorted, so keys compare across blocks.
/// Which states of a block drop the block itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfDrop {
    /// Every state: `dropSelf`.
    Always,
    /// Only the states where this property has this value.
    When(String, String),
}

/// A state key's `property=value` pairs.
fn pairs(key: &str) -> BTreeSet<(String, String)> {
    key.split(',')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

pub fn state_key(key: &str) -> String {
    let mut parts: Vec<&str> = key.split(',').filter(|p| !p.is_empty()).collect();
    parts.sort_unstable();
    parts.join(",")
}

impl Block {
    /// The first vanilla class in the chain.
    pub fn vanilla_class(&self) -> &str {
        self.class
            .iter()
            .find(|c| c.starts_with(VANILLA_PACKAGE))
            .map_or("", String::as_str)
    }
    /// Its own class is a mod class: behaviour code the template may not have.
    pub fn custom_class(&self) -> bool {
        self.class
            .first()
            .is_some_and(|c| !c.starts_with(VANILLA_PACKAGE))
    }
    /// Every state drops one `item` (the block's own) with every tool, except
    /// where the state needs a tool and gets none: vanilla's `dropSelf`. Or
    /// only the states where one property has one value do, and the others
    /// drop nothing with any tool: a door's lower half (`createDoorTable`).
    /// Mods often register these tables in code, so their jar has no file.
    pub fn drops_itself(&self, item: &str) -> Option<SelfDrop> {
        let one = BTreeMap::from([(item.to_owned(), 1)]);
        let (mut dropping, mut nothing) = (Vec::new(), Vec::new());
        for (key, state) in &self.states {
            if state.drops.is_empty() {
                continue;
            }
            let mut drops = false;
            let mut short = false;
            for got in state.drops.values() {
                if *got == one {
                    drops = true;
                } else if got.is_empty() {
                    short = true;
                } else {
                    return None;
                }
            }
            match (drops, short) {
                (true, true) if !state.requires_tool => return None,
                (true, _) => dropping.push(pairs(key)),
                (false, _) => nothing.push(pairs(key)),
            }
        }
        let (first, rest) = dropping.split_first()?;
        if nothing.is_empty() {
            return Some(SelfDrop::Always);
        }
        first
            .iter()
            .filter(|pair| rest.iter().all(|s| s.contains(*pair)))
            .find(|pair| nothing.iter().all(|s| !s.contains(*pair)))
            .map(|(p, v)| SelfDrop::When(p.clone(), v.clone()))
    }
    fn default_state(&self) -> Option<&State> {
        self.states
            .get(&self.default)
            .or_else(|| self.states.values().next())
    }
    fn same_states(&self, other: &Block) -> bool {
        self.properties.len() == other.properties.len()
            && self.properties.iter().all(|(k, v)| {
                other.properties.get(k).is_some_and(|w| {
                    v.iter().collect::<BTreeSet<_>>() == w.iter().collect::<BTreeSet<_>>()
                })
            })
    }
    fn by_key(&self) -> BTreeMap<String, &State> {
        self.states.iter().map(|(k, s)| (state_key(k), s)).collect()
    }
}

/// The template carry-over gives a mod block.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub template: String,
    /// `data/<mod>/horizon/block/<name>.json`'s `properties`.
    pub properties: Map<String, Value>,
    /// What the template does not reproduce, for the report.
    pub differences: Vec<String>,
}

/// Equal JSON, numbers compared as f64 (the oracle writes `6.0` where the game's table has `6`).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

impl Facts {
    pub fn read(path: &Path) -> Result<Facts, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The vanilla block that best stands in for `id`: the same vanilla
    /// class and states, then the most states with the same outline and
    /// collision, then the same sounds, set type, wood type and hardness.
    /// `None` when no vanilla block has its class and states.
    pub fn template(&self, id: &str, allowed: &dyn Fn(&str) -> bool) -> Option<Choice> {
        let block = self.blocks.get(id)?;
        let class = block.vanilla_class();
        let own = block.by_key();
        let ours = block.default_state()?;
        let mut best: Option<(i64, &str, &Block)> = None;
        for (name, v) in &self.blocks {
            if !name.starts_with("minecraft:")
                || v.vanilla_class() != class
                || v.custom_class()
                || !block.same_states(v)
                || !allowed(name)
            {
                continue;
            }
            let theirs = v.by_key();
            let shapes = own
                .iter()
                .filter(|(k, s)| {
                    theirs
                        .get(*k)
                        .is_some_and(|t| t.outline == s.outline && t.collision == s.collision)
                })
                .count() as i64;
            let Some(td) = v.default_state() else {
                continue;
            };
            let same_set = match (&block.block_set_type, &v.block_set_type) {
                (Some(a), Some(b)) => {
                    (a.name == b.name) as i64 * 2
                        + (a.can_open_by_hand == b.can_open_by_hand) as i64 * 8
                }
                _ => 0,
            };
            let score = shapes * 1000
                + (td.sound == ours.sound) as i64 * 100
                + same_set * 10
                + (block.wood_type.is_some() && block.wood_type == v.wood_type) as i64 * 20
                + (td.hardness == ours.hardness) as i64 * 5
                + (td.map_color_id == ours.map_color_id) as i64;
            if best.is_none_or(|(s, n, _)| score > s || (score == s && name.as_str() < n)) {
                best = Some((score, name, v));
            }
        }
        let (_, template, t) = best?;
        Some(self.choice(block, template, t, allowed))
    }

    fn choice(
        &self,
        block: &Block,
        template: &str,
        t: &Block,
        allowed: &dyn Fn(&str) -> bool,
    ) -> Choice {
        let (ours, theirs) = (block.default_state(), t.default_state());
        let mut properties = Map::new();
        let mut differences = Vec::new();
        if let Some(s) = ours {
            properties.insert("hardness".into(), json!(s.hardness));
            properties.insert(
                "explosion_resistance".into(),
                json!(block.explosion_resistance),
            );
            properties.insert("light".into(), json!(s.emission));
            properties.insert("requires_tool".into(), json!(s.requires_tool));
            properties.insert("friction".into(), json!(block.friction));
            properties.insert("map_color".into(), json!(s.map_color_id));
            if theirs.is_none_or(|t| t.sound != s.sound) {
                match self.sound_block(&s.sound, allowed) {
                    Some(b) => {
                        properties.insert("sounds".into(), json!(b));
                    }
                    None => differences.push("sounds: no vanilla block has them".into()),
                }
            }
        }
        if (block.speed_factor, block.jump_factor) != (t.speed_factor, t.jump_factor) {
            differences.push(format!(
                "speed and jump factors {} {} (template {} {})",
                block.speed_factor, block.jump_factor, t.speed_factor, t.jump_factor
            ));
        }
        let theirs = t.by_key();
        let mut shapes = 0;
        let mut varying = BTreeSet::new();
        let first = ours.map(|s| (s.hardness, s.emission, s.requires_tool, s.map_color_id));
        for (k, s) in block.by_key() {
            let Some(v) = theirs.get(&k) else {
                continue;
            };
            if v.outline != s.outline || v.collision != s.collision {
                shapes += 1;
            }
            if Some((s.hardness, s.emission, s.requires_tool, s.map_color_id)) != first {
                varying.insert("per-state hardness, light or map colour");
            }
        }
        if shapes > 0 {
            differences.push(format!(
                "shapes differ from {template} in {shapes} of {} states",
                block.states.len()
            ));
        }
        differences.extend(varying.into_iter().map(str::to_owned));
        if block.custom_class() {
            differences.push(format!(
                "class {} (behaviour beyond {})",
                block.class[0],
                block.vanilla_class()
            ));
        }
        Choice {
            template: template.to_owned(),
            properties,
            differences,
        }
    }

    /// A vanilla block whose default state has these sounds.
    fn sound_block(&self, sound: &Value, allowed: &dyn Fn(&str) -> bool) -> Option<&str> {
        self.blocks
            .iter()
            .filter(|(n, _)| n.starts_with("minecraft:") && allowed(n))
            .find(|(_, b)| b.default_state().is_some_and(|s| &s.sound == sound))
            .map(|(n, _)| n.as_str())
    }

    /// An item declaration for a mod item that is not a declared block's:
    /// the vanilla item of its class with its stack size, rarity and numbers,
    /// and what it does not reproduce.
    pub fn item(&self, id: &str, owned: &[&str]) -> Result<(Value, Vec<String>), String> {
        let item = self.items.get(id).ok_or("not in the oracle")?;
        if item.block.is_some() {
            return Err("a block item without a block".into());
        }
        let mut differences = Vec::new();
        if !item.class.starts_with(VANILLA_PACKAGE) {
            differences.push(format!("class {} (behaviour beyond Item)", item.class));
        }
        if item.food.is_some() {
            return Err("food (no food template yet)".into());
        }
        let template = item_template(item);
        let mut decl = json!({"template": template});
        if item.max_stack_size != 64 {
            decl["max_stack_size"] = json!(item.max_stack_size);
        }
        if item.rarity != "common" {
            decl["rarity"] = json!(item.rarity);
        }
        if let Some(armor) = item.armor.as_ref().filter(|a| !a.material.is_empty())
            && !self.armor_asset_exists(armor, owned)
        {
            differences.push(format!(
                "armour material {} has no equipment asset: it draws the template's",
                armor.material
            ));
        }
        let components = self.item_components(item, template, owned);
        if !components.is_empty() {
            decl["components"] = Value::Object(components);
        }
        Ok((decl, differences))
    }

    /// The item's numbers as a 26.3 component patch over `template`'s, leaving out what already matches.
    fn armor_asset_exists(&self, armor: &Armor, owned: &[&str]) -> bool {
        armor.asset_carried(owned)
            || self.vanilla_items.values().any(|i| {
                i["minecraft:equippable"]["asset_id"].as_str() == Some(armor.material.as_str())
            })
    }

    fn item_components(&self, item: &Item, template: &str, owned: &[&str]) -> Map<String, Value> {
        if item.components.is_empty() {
            return Map::new();
        }
        let base = self.vanilla_items.get(template);
        let mut want = Map::new();
        want.insert(
            "minecraft:max_damage".into(),
            item.components
                .get("minecraft:max_damage")
                .cloned()
                .unwrap_or(Value::Null),
        );
        if let Some(m) = &item.attribute_modifiers {
            want.insert("minecraft:attribute_modifiers".into(), modifiers_26_3(m));
        }
        // The mod's rules as they are; what 1.21.1 kept in the class (can_destroy_blocks_in_creative) stays the template's.
        let tool = item.components.get("minecraft:tool").map(|t| {
            let mut tool = base
                .and_then(|b| b.get("minecraft:tool"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            for key in ["rules", "default_mining_speed", "damage_per_block"] {
                match t.get(key) {
                    Some(v) => tool[key] = v.clone(),
                    None => _ = tool.as_object_mut().map(|o| o.remove(key)),
                }
            }
            tool
        });
        want.insert("minecraft:tool".into(), tool.unwrap_or(Value::Null));
        if let Some(value) = item.enchantment_value {
            let enchantable = (value > 0).then(|| json!({"value": value}));
            want.insert(
                "minecraft:enchantable".into(),
                enchantable.unwrap_or(Value::Null),
            );
        }
        if let Some(repair) = &item.repair_items {
            let items = match repair.as_slice() {
                [] => Value::Null,
                [one] => json!({"items": one}),
                many => json!({"items": many}),
            };
            want.insert("minecraft:repairable".into(), items);
        }
        if let Some(armor) = item.armor.as_ref().filter(|a| !a.material.is_empty()) {
            let mut equippable = base.and_then(|b| b.get("minecraft:equippable")).cloned();
            if self.armor_asset_exists(armor, owned) {
                equippable.get_or_insert_with(|| json!({"slot": armor.slot}))["asset_id"] =
                    json!(armor.material);
            }
            if let Some(mut equippable) = equippable {
                if let Some(sound) = &armor.equip_sound {
                    equippable["equip_sound"] = json!(sound);
                }
                want.insert("minecraft:equippable".into(), equippable);
            }
        }
        let mut patch = Map::new();
        for (key, value) in want {
            let had = base.and_then(|b| b.get(&key));
            match (value, had) {
                (Value::Null, None) => {}
                (Value::Null, Some(_)) => _ = patch.insert(format!("!{key}"), json!({})),
                (v, Some(h)) if same(&v, h) => {}
                (v, _) => _ = patch.insert(key, v),
            }
        }
        patch
    }

    /// A tag's members the content mod may name: vanilla's and its own.
    pub fn tag_members(&self, kind: &str, tag: &str, own: &str) -> Option<Vec<String>> {
        let map = if kind == "block" {
            &self.tags.block
        } else {
            &self.tags.item
        };
        let members = map.get(tag)?;
        Some(
            members
                .iter()
                .filter(|m| {
                    m.split_once(':')
                        .is_some_and(|(ns, _)| ns == "minecraft" || ns == own)
                })
                .cloned()
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(hardness: f32, sound: &str, outline: Value) -> Value {
        json!({"hardness": hardness, "emission": 0, "requires_tool": false, "map_color_id": 13,
            "sound": {"break": sound}, "outline": outline, "collision": outline})
    }
    fn block(class: &[&str], props: Value, states: Value) -> Value {
        let default = states
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .to_owned();
        json!({"class": class, "properties": props, "default": default, "item": "x",
            "explosion_resistance": 3.0, "friction": 0.6, "speed_factor": 1.0,
            "jump_factor": 1.0, "states": states})
    }

    #[test]
    fn templates_follow_class_shapes_and_sounds() {
        let cube = json!([[0, 0, 0, 1, 1, 1]]);
        let low = json!([[0, 0, 0, 1, 0.9375, 1]]);
        let block_class = ["net.minecraft.world.level.block.Block"];
        let path = ["net.minecraft.world.level.block.DirtPathBlock"];
        let facts: Facts = serde_json::from_value(json!({
            "blocks": {
                "minecraft:stone": block(&block_class, json!({}), json!({"": state(1.5, "stone", cube.clone())})),
                "minecraft:oak_planks": block(&block_class, json!({}), json!({"": state(2.0, "wood", cube.clone())})),
                "minecraft:dirt_path": block(&path, json!({}), json!({"": state(0.65, "grass", low.clone())})),
                "m:wood_tile": block(&["m.Tile", block_class[0]], json!({}), json!({"": state(2.5, "wood", cube.clone())})),
                "m:path": block(&["m.Path", block_class[0]], json!({}), json!({"": state(1.0, "stone", low.clone())})),
                "m:gadget": block(&["m.Gadget", block_class[0]], json!({"mode": ["a", "b"]}),
                    json!({"mode=a": state(1.0, "stone", cube.clone()), "mode=b": state(1.0, "stone", cube)})),
            },
            "items": {}
        }))
        .unwrap();
        let any = |_: &str| true;
        let tile = facts.template("m:wood_tile", &any).unwrap();
        assert_eq!(tile.template, "minecraft:oak_planks");
        assert_eq!(tile.properties["hardness"], json!(2.5));
        assert!(!tile.properties.contains_key("sounds"));
        assert_eq!(
            tile.differences,
            ["class m.Tile (behaviour beyond net.minecraft.world.level.block.Block)"]
        );
        // A plain Block is never templated on a DirtPathBlock: shapes differ, and say so.
        let path = facts.template("m:path", &any).unwrap();
        assert_eq!(path.template, "minecraft:stone");
        assert!(
            path.differences[0].starts_with("shapes differ"),
            "{:?}",
            path.differences
        );
        assert_eq!(facts.template("m:gadget", &any), None);
        let tile = facts
            .template("m:wood_tile", &|n: &str| n != "minecraft:oak_planks")
            .unwrap();
        assert_eq!(tile.template, "minecraft:stone");
        assert_eq!(tile.differences[0], "sounds: no vanilla block has them");
    }

    #[test]
    fn drops_itself_is_vanillas_drop_self() {
        let cube = json!([[0, 0, 0, 1, 1, 1]]);
        let with = |requires_tool: bool, drops: Value| {
            let mut s = state(1.5, "stone", cube.clone());
            s["requires_tool"] = json!(requires_tool);
            s["drops"] = drops;
            let b: Block = serde_json::from_value(block(&[], json!({}), json!({"": s}))).unwrap();
            b.drops_itself("m:tile")
        };
        let me = json!({"m:tile": 1});
        let always = Some(SelfDrop::Always);
        assert_eq!(with(false, json!({"hand": me, "shears": me})), always);
        // A state that needs a tool drops nothing by hand, as dropSelf does.
        assert_eq!(
            with(true, json!({"hand": {}, "diamond_pickaxe": me})),
            always
        );
        assert_eq!(
            with(false, json!({"hand": {}, "diamond_pickaxe": me})),
            None
        );
        // Two of itself (a double slab), or something else, is not dropSelf.
        assert_eq!(with(false, json!({"hand": {"m:tile": 2}})), None);
        assert_eq!(with(false, json!({"hand": {"m:other": 1}})), None);
        assert_eq!(with(false, json!({})), None);
    }

    #[test]
    fn a_door_drops_from_its_lower_half() {
        let cube = json!([[0, 0, 0, 1, 1, 1]]);
        let me = json!({"hand": {"m:door": 1}, "shears": {"m:door": 1}});
        let none = json!({"hand": {}, "shears": {}});
        let door = |upper: &Value| {
            let mut states = serde_json::Map::new();
            for (key, drops) in [
                ("half=lower,open=false", &me),
                ("half=lower,open=true", &me),
                ("half=upper,open=false", upper),
                ("half=upper,open=true", upper),
            ] {
                let mut s = state(3.0, "wood", cube.clone());
                s["drops"] = drops.clone();
                states.insert(key.to_owned(), s);
            }
            let props = json!({"half": ["upper", "lower"], "open": ["true", "false"]});
            let b: Block =
                serde_json::from_value(block(&[], props, Value::Object(states))).unwrap();
            b.drops_itself("m:door")
        };
        assert_eq!(
            door(&none),
            Some(SelfDrop::When("half".to_owned(), "lower".to_owned()))
        );
        assert_eq!(door(&me), Some(SelfDrop::Always));
        // Something else from the upper half: not a door table.
        assert_eq!(door(&json!({"hand": {"m:other": 1}})), None);
    }

    #[test]
    fn items_of_vanilla_classes_take_their_template_and_numbers() {
        let mut facts: Facts = serde_json::from_value(json!({
            "blocks": {},
            "items": {
                "m:sword": {"class": "net.minecraft.world.item.SwordItem", "max_stack_size": 1, "rarity": "common",
                    "components": {"minecraft:max_damage": 905, "minecraft:tool": {"rules": [
                        {"blocks": "minecraft:cobweb", "speed": 15.0, "correct_for_drops": true}], "damage_per_block": 2}},
                    "attribute_modifiers": {"modifiers": [{"type": "minecraft:generic.attack_damage",
                        "id": "minecraft:base_attack_damage", "amount": 5.5, "operation": "add_value", "slot": "mainhand"}]},
                    "enchantment_value": 14, "repair_items": ["minecraft:emerald"]},
                "m:boots": {"class": "net.minecraft.world.item.ArmorItem", "max_stack_size": 1, "rarity": "common",
                    "components": {"minecraft:max_damage": 1}, "enchantment_value": 0, "repair_items": [],
                    "armor": {"slot": "feet"}},
                "m:gem": {"class": "net.minecraft.world.item.Item", "max_stack_size": 64, "rarity": "common"},
            }
        }))
        .unwrap();
        facts.vanilla_items = serde_json::from_value(json!({
            "minecraft:iron_sword": {"minecraft:max_damage": 250, "minecraft:enchantable": {"value": 14},
                "minecraft:repairable": {"items": "#minecraft:iron_tool_materials"},
                "minecraft:tool": {"can_destroy_blocks_in_creative": false, "damage_per_block": 2,
                    "rules": [{"blocks": "#minecraft:sword_efficient", "speed": 1.5}]}},
            "minecraft:iron_boots": {"minecraft:max_damage": 195, "minecraft:enchantable": {"value": 9},
                "minecraft:repairable": {"items": "#minecraft:repairs_iron_armor"}},
        }))
        .unwrap();
        let (sword, _) = facts.item("m:sword", &["m"]).unwrap();
        assert_eq!(sword["template"], "minecraft:iron_sword");
        let c = &sword["components"];
        assert_eq!(c["minecraft:max_damage"], 905);
        assert_eq!(
            c["minecraft:attribute_modifiers"][0]["type"],
            "minecraft:attack_damage"
        );
        assert_eq!(
            c["minecraft:repairable"],
            json!({"items": "minecraft:emerald"})
        );
        // The mod's rules replace the template's; the creative flag the class gave stays.
        assert_eq!(
            c["minecraft:tool"],
            json!({"can_destroy_blocks_in_creative": false, "damage_per_block": 2,
                "rules": [{"blocks": "minecraft:cobweb", "speed": 15.0, "correct_for_drops": true}]})
        );
        // Equal to the template's: left out.
        assert!(c.get("minecraft:enchantable").is_none());
        let (boots, _) = facts.item("m:boots", &["m"]).unwrap();
        assert_eq!(boots["template"], "minecraft:iron_boots");
        assert_eq!(boots["components"]["!minecraft:enchantable"], json!({}));
        assert_eq!(boots["components"]["!minecraft:repairable"], json!({}));
        // Facts without components (exported before they were) keep the plain template.
        let (gem, _) = facts.item("m:gem", &["m"]).unwrap();
        assert_eq!(gem, json!({"template": "minecraft:stick"}));
    }

    #[test]
    fn armour_materials_become_equipment_assets() {
        let armor: Armor = serde_json::from_value(json!({"slot": "legs", "material": "m:ruby",
            "equip_sound": "minecraft:item.armor.equip_diamond", "layers": [
                {"texture": "m:textures/models/armor/ruby_layer_1.png",
                 "texture_legs": "m:textures/models/armor/ruby_layer_2.png", "dyeable": false},
                {"texture": "m:textures/models/armor/ruby_overlay_layer_1.png",
                 "texture_legs": "m:textures/models/armor/ruby_overlay_layer_2.png", "dyeable": true}]}))
        .unwrap();
        let (asset, copies) = armor.equipment().unwrap();
        assert_eq!(
            asset["layers"]["humanoid_leggings"],
            json!([{"texture": "m:ruby"}, {"texture": "m:ruby_overlay", "dyeable": {}}])
        );
        assert_eq!(
            copies[2],
            (
                "assets/m/textures/models/armor/ruby_layer_2.png".to_owned(),
                "assets/m/textures/entity/equipment/humanoid_leggings/ruby.png".to_owned()
            )
        );
        let mut facts: Facts =
            serde_json::from_value(json!({"blocks": {}, "items": {"m:ruby_leggings": {
            "class": "net.minecraft.world.item.ArmorItem", "max_stack_size": 1, "rarity": "common",
            "components": {"minecraft:max_damage": 300}, "armor": armor_json()}}}))
            .unwrap();
        facts.vanilla_items = serde_json::from_value(json!({"minecraft:iron_leggings": {
            "minecraft:equippable": {"asset_id": "minecraft:iron", "equip_sound": "minecraft:item.armor.equip_iron", "slot": "legs"}}}))
        .unwrap();
        let (decl, _) = facts.item("m:ruby_leggings", &["m"]).unwrap();
        assert_eq!(
            decl["components"]["minecraft:equippable"],
            json!({"asset_id": "m:ruby", "equip_sound": "minecraft:item.armor.equip_diamond", "slot": "legs"})
        );
        // Layers outside models/armor/ have no 26.3 place to go.
        let odd: Armor =
            serde_json::from_value(json!({"slot": "head", "material": "m:odd", "layers": [
            {"texture": "m:textures/odd.png", "texture_legs": "m:textures/odd.png"}]}))
            .unwrap();
        assert!(odd.equipment().is_none());
    }

    #[test]
    fn armour_only_names_an_equipment_asset_that_exists() {
        let leggings = |material: &str, layers: Value| {
            json!({"class": "net.minecraft.world.item.ArmorItem", "max_stack_size": 1, "rarity": "common",
                "components": {"minecraft:max_damage": 300},
                "armor": {"slot": "legs", "material": material,
                    "equip_sound": "minecraft:item.armor.equip_diamond", "layers": layers}})
        };
        let odd = json!([{"texture": "m:textures/odd.png", "texture_legs": "m:textures/odd.png"}]);
        let mut facts: Facts = serde_json::from_value(json!({"blocks": {}, "items": {
            "m:ruby": leggings("m:ruby", json!([])),
            "m:odd": leggings("m:odd", odd),
            "m:other": leggings("other:steel", json!([])),
            "m:unknown": leggings("minecraft:steel", json!([])),
            "m:diamond": leggings("minecraft:diamond", json!([]))}}))
        .unwrap();
        facts.vanilla_items = serde_json::from_value(json!({
            "minecraft:iron_leggings": {"minecraft:equippable": {"asset_id": "minecraft:iron", "slot": "legs"}},
            "minecraft:diamond_leggings": {"minecraft:equippable": {"asset_id": "minecraft:diamond", "slot": "legs"}}}))
        .unwrap();
        let asset = |id: &str| {
            let (decl, differences) = facts.item(id, &["m"]).unwrap();
            let asset = decl["components"]["minecraft:equippable"]["asset_id"].clone();
            (asset, !differences.is_empty())
        };
        assert_eq!(asset("m:ruby"), (json!("m:ruby"), false));
        assert_eq!(asset("m:diamond"), (json!("minecraft:diamond"), false));
        for id in ["m:odd", "m:other", "m:unknown"] {
            assert_eq!(asset(id), (json!("minecraft:iron"), true), "{id}");
        }
        // Without the game's tables the template's own component stays whole.
        facts.vanilla_items.clear();
        let (decl, _) = facts.item("m:odd", &["m"]).unwrap();
        assert!(decl["components"].get("minecraft:equippable").is_none());
    }

    fn armor_json() -> Value {
        json!({"slot": "legs", "material": "m:ruby", "equip_sound": "minecraft:item.armor.equip_diamond"})
    }

    #[test]
    fn state_keys_sort() {
        assert_eq!(
            state_key("half=lower,facing=north"),
            "facing=north,half=lower"
        );
        assert_eq!(state_key(""), "");
    }
}
