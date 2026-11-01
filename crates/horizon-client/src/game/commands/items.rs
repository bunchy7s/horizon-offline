//! `/item` (ItemCommands) and `if items`/`if slots`: addressing the item
//! slots of the player, mobs, armor stands, item frames and containers.
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{
    command::{CommandError, args::ItemInput},
    items::ItemStack,
    scoreboard_cmd::{Holder, Tracker},
    text::Text,
};
use horizon_world::nbt::Tag;

/// One addressable slot.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) enum Slot {
    /// `inventory[0..36]`: hotbar then the main rows.
    Inv(usize),
    /// The selected hotbar slot as `weapon.mainhand` names it: the equipment's
    /// `setItemSlot`, where `hotbar.N` is the inventory's `setItem`.
    MainHand,
    /// `armor[0..4]` (head, chest, legs, feet).
    Armor(usize),
    Offhand,
    Ender(usize),
    /// A mob's equipment by name (`mainhand`, `offhand`, `head`, ...).
    Mob(u32, &'static str),
    /// An armor stand's equipment (`ArmorStand::SLOTS` index).
    Stand(u32, usize),
    Frame(u32),
    /// A chest or hopper minecart's slot (`ContainerEntity.getChestVehicleSlot`).
    Cart(u32, usize),
    Block([i32; 3], usize),
    /// A slot of another player (`for_player`): reached with their state in place.
    Guest(u32, Box<Slot>),
}
/// Whose slots a command addresses.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) enum Owner {
    Entity(Target),
    Block([i32; 3]),
}

/// The player's slot names and the inventory index each addresses.
fn player_slot_names() -> Vec<(String, usize)> {
    let mut v = Vec::new();
    for i in 0..36 {
        v.push((format!("container.{i}"), i));
    }
    for i in 0..9 {
        v.push((format!("hotbar.{i}"), i));
    }
    for i in 0..27 {
        v.push((format!("inventory.{i}"), 9 + i));
    }
    v
}
/// `name` against a group: `container.*` matches every `container.N`.
fn matches_name(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix(".*") {
        Some(prefix) => name.starts_with(&format!("{prefix}.")),
        None => pattern == name,
    }
}

impl Game<'_> {
    fn owner_of(&mut self, ctx: &Context, prefix: &str, many: bool) -> R<Vec<Owner>> {
        let kind = ctx
            .literals
            .iter()
            .filter(|l| matches!(l.as_str(), "entity" | "block"))
            .nth(usize::from(prefix == "source" && ctx.has("from")))
            .cloned()
            .unwrap_or_default();
        if kind == "block" {
            let p = self.block_pos_arg(ctx, &format!("{prefix}Pos"))?;
            return Ok(vec![Owner::Block(p)]);
        }
        let targets = self.entities(ctx, prefix, false)?;
        let _ = many;
        Ok(targets.into_iter().map(Owner::Entity).collect())
    }
    /// The slots a name (or wildcard group) addresses on an owner.
    pub(in crate::game) fn slots_named(&mut self, owner: &Owner, name: &str) -> Vec<Slot> {
        if let Owner::Entity(Target::Guest(conn)) = *owner {
            let slots = self.for_player(Target::Guest(conn), |g| {
                g.slots_named(&Owner::Entity(Target::Player), name)
            });
            return slots
                .unwrap_or_default()
                .into_iter()
                .map(|s| Slot::Guest(conn, Box::new(s)))
                .collect();
        }
        let mut out = Vec::new();
        let mut add = |s: Slot| out.push(s);
        match owner {
            Owner::Block(p) => {
                let size = match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                    Some(horizon_client::containers::BlockEntity::Chest(items)) => items.len(),
                    _ => 0,
                };
                for i in 0..size {
                    if matches_name(name, &format!("container.{i}")) {
                        add(Slot::Block(*p, i));
                    }
                }
            }
            Owner::Entity(Target::Player) => {
                for (n, i) in player_slot_names() {
                    if matches_name(name, &n) {
                        add(Slot::Inv(i));
                    }
                }
                for (i, n) in ["armor.head", "armor.chest", "armor.legs", "armor.feet"]
                    .into_iter()
                    .enumerate()
                {
                    if matches_name(name, n) {
                        add(Slot::Armor(i));
                    }
                }
                if matches_name(name, "weapon") || matches_name(name, "weapon.mainhand") {
                    add(Slot::MainHand);
                }
                if matches_name(name, "weapon.offhand") {
                    add(Slot::Offhand);
                }
                for i in 0..27 {
                    if matches_name(name, &format!("enderchest.{i}")) {
                        add(Slot::Ender(i));
                    }
                }
            }
            Owner::Entity(Target::Mob(id)) => {
                for (n, key) in [
                    ("weapon", "mainhand"),
                    ("weapon.mainhand", "mainhand"),
                    ("weapon.offhand", "offhand"),
                    ("armor.head", "head"),
                    ("armor.chest", "chest"),
                    ("armor.legs", "legs"),
                    ("armor.feet", "feet"),
                    ("armor.body", "body"),
                    ("saddle", "saddle"),
                ] {
                    if matches_name(name, n) {
                        add(Slot::Mob(*id, key));
                    }
                }
            }
            Owner::Entity(Target::Stand(id)) => {
                for (i, key) in horizon_client::entities::ArmorStand::SLOTS
                    .iter()
                    .enumerate()
                {
                    let aliases: &[&str] = match *key {
                        "mainhand" => &["weapon", "weapon.mainhand"],
                        "offhand" => &["weapon.offhand"],
                        "feet" => &["armor.feet"],
                        "legs" => &["armor.legs"],
                        "chest" => &["armor.chest"],
                        _ => &["armor.head"],
                    };
                    if aliases.iter().any(|a| matches_name(name, a)) {
                        add(Slot::Stand(*id, i));
                    }
                }
            }
            Owner::Entity(Target::Frame(id)) => {
                if matches_name(name, "container.0") || matches_name(name, "contents") {
                    add(Slot::Frame(*id));
                }
            }
            // `getChestVehicleSlot` takes any index below `getContainerSize`.
            Owner::Entity(Target::Cart(id)) => {
                let size = self
                    .carts
                    .iter()
                    .find(|c| c.id == *id)
                    .map_or(0, |c| c.kind.slots());
                for i in 0..size {
                    if matches_name(name, &format!("container.{i}")) {
                        add(Slot::Cart(*id, i));
                    }
                }
            }
            Owner::Entity(_) => {}
        }
        out
    }
    pub(in crate::game) fn slot_get(&mut self, s: &Slot) -> Option<ItemStack> {
        match s {
            Slot::Guest(conn, slot) => self
                .for_player(Target::Guest(*conn), |g| g.slot_get(slot))
                .flatten(),
            Slot::Inv(i) => self.inventory.get(*i).cloned().flatten(),
            Slot::MainHand => self.inventory.get(self.selected).cloned().flatten(),
            Slot::Armor(i) => self.armor.get(*i).cloned().flatten(),
            Slot::Offhand => self.offhand.clone(),
            Slot::Ender(i) => self.ender_items.get(*i).cloned().flatten(),
            Slot::Mob(id, key) => {
                let m = self.mobs.iter().find(|m| m.id == *id)?;
                let item = match *key {
                    "mainhand" => &m.extra.main_hand,
                    "offhand" => &m.extra.off_hand,
                    "head" | "body" => &m.extra.head,
                    "chest" => &m.extra.chest,
                    "legs" => &m.extra.legs,
                    "feet" => &m.extra.feet,
                    _ => return None,
                };
                item.as_ref().map(|i| ItemStack {
                    item: i.clone(),
                    count: 1,
                    damage: 0,
                    extra: None,
                })
            }
            Slot::Stand(id, i) => self
                .armor_stands
                .iter()
                .find(|a| a.id == *id)?
                .equipment
                .get(*i)
                .cloned()
                .flatten(),
            Slot::Frame(id) => self.item_frames.iter().find(|f| f.id == *id)?.item.clone(),
            Slot::Cart(id, i) => self.cart_slots(*id)?.get(*i).cloned().flatten(),
            Slot::Block(p, i) => match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                Some(horizon_client::containers::BlockEntity::Chest(items)) => {
                    items.get(*i).cloned().flatten()
                }
                _ => None,
            },
        }
    }
    /// Put a stack (or nothing) in a slot; false when the slot cannot take it.
    pub(in crate::game) fn slot_set(&mut self, s: &Slot, stack: Option<ItemStack>) -> bool {
        match s {
            Slot::Guest(conn, slot) => self
                .for_player(Target::Guest(*conn), |g| g.slot_set(slot, stack))
                .unwrap_or(false),
            Slot::Inv(i) => match self.inventory.get_mut(*i) {
                Some(slot) => {
                    *slot = stack;
                    true
                }
                None => false,
            },
            // LivingEntity's equipment SlotAccess: setItemSlot, so onEquipItem.
            Slot::MainHand => match self.inventory.get_mut(self.selected) {
                Some(slot) => {
                    let old = std::mem::replace(slot, stack.clone());
                    self.player_on_equip("mainhand", old.as_ref(), stack.as_ref());
                    true
                }
                None => false,
            },
            Slot::Armor(i) => match self.armor.get_mut(*i) {
                Some(slot) => {
                    let old = std::mem::replace(slot, stack.clone());
                    let name = ["head", "chest", "legs", "feet"][*i];
                    self.player_on_equip(name, old.as_ref(), stack.as_ref());
                    self.apply_equipment();
                    true
                }
                None => false,
            },
            Slot::Offhand => {
                let old = std::mem::replace(&mut self.offhand, stack.clone());
                self.player_on_equip("offhand", old.as_ref(), stack.as_ref());
                true
            }
            Slot::Ender(i) => match self.ender_items.get_mut(*i) {
                Some(slot) => {
                    *slot = stack;
                    true
                }
                None => false,
            },
            Slot::Mob(id, key) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == *id) else {
                    return false;
                };
                let item = stack.as_ref().map(|s| s.item.clone());
                match *key {
                    "mainhand" => m.extra.set_main_hand_stack(stack),
                    "offhand" => m.extra.off_hand = item,
                    "head" | "body" => m.extra.head = item,
                    "chest" => m.extra.chest = item,
                    "legs" => m.extra.legs = item,
                    "feet" => m.extra.feet = item,
                    _ => return false,
                }
                true
            }
            Slot::Stand(id, i) => match self
                .armor_stands
                .iter_mut()
                .find(|a| a.id == *id)
                .and_then(|a| a.equipment.get_mut(*i))
            {
                Some(slot) => {
                    *slot = stack;
                    true
                }
                None => false,
            },
            Slot::Frame(id) => match self.item_frames.iter_mut().find(|f| f.id == *id) {
                Some(f) => {
                    f.item = stack;
                    true
                }
                None => false,
            },
            Slot::Cart(id, i) => match self.cart_slots(*id).and_then(|c| c.get_mut(*i)) {
                Some(slot) => {
                    *slot = stack;
                    true
                }
                None => false,
            },
            Slot::Block(p, i) => match self.world.block_entities.get_mut(&(p[0], p[1], p[2])) {
                Some(horizon_client::containers::BlockEntity::Chest(items)) => {
                    match items.get_mut(*i) {
                        Some(slot) => {
                            *slot = stack;
                            true
                        }
                        None => false,
                    }
                }
                _ => false,
            },
        }
    }
    fn owner_holder(&self, o: &Owner) -> Holder {
        match o {
            Owner::Entity(t) => Holder {
                name: String::new(),
                text: self.target_name(*t),
                display: None,
            },
            Owner::Block(p) => Holder {
                name: String::new(),
                text: Text::literal(format!("{}, {}, {}", p[0], p[1], p[2])),
                display: None,
            },
        }
    }
    fn stack_name(&self, s: &ItemStack) -> Text {
        self.stack_display_name(s)
    }
    fn item_from_input(&self, ctx: &Context) -> R<Option<ItemStack>> {
        let Some(Arg::Item(ItemInput { id, components, .. })) = ctx.get("item").cloned() else {
            return Ok(None);
        };
        let (damage, extra) = match components
            .as_deref()
            .map(horizon_client::item_data::command_components)
        {
            Some(Ok(tag)) => horizon_client::item_data::decode_components(Some(&tag)),
            Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
            None => (0, None),
        };
        let count = Self::int(ctx, "count").unwrap_or(1).max(1) as u32;
        let stack = ItemStack {
            item: id.as_str().into(),
            count,
            damage,
            extra,
        };
        // ItemInput.createItemStack: a count above the stack's max (its
        // max_stack_size component, else the item's) is refused.
        let max_stack = self.stack_max_size(&stack);
        if count > max_stack {
            return Err(err("arguments.item.overstacked", &[&id, &max_stack]));
        }
        Ok(Some(stack))
    }
    /// One item modifier (a typed object, a list or a datapack reference)
    /// applied to a stack; `None` when the stack ends up empty.
    fn apply_item_modifier(
        &self,
        m: &serde_json::Value,
        stack: Option<ItemStack>,
        rng: &mut horizon_core::random::LegacyRandom,
        this: Option<&horizon_client::advancements::EntityView>,
        origin: [f64; 3],
        depth: u32,
    ) -> Option<ItemStack> {
        use horizon_core::random::RandomSource;
        use serde_json::Value;
        if depth > 16 {
            return stack;
        }
        let int = |v: &Value, rng: &mut horizon_core::random::LegacyRandom| -> i32 {
            if let Some(n) = v.as_f64() {
                return n as i32;
            }
            let min = v["min"].as_f64().unwrap_or(0.) as i32;
            let max = v["max"].as_f64().unwrap_or(0.) as i32;
            match v["type"].as_str().unwrap_or("minecraft:constant") {
                "minecraft:uniform" | "uniform" => rng
                    .next_int_between_inclusive(min, max.max(min))
                    .unwrap_or(min),
                _ => v["value"].as_f64().unwrap_or(0.) as i32,
            }
        };
        match m {
            Value::Array(list) => list.iter().fold(stack, |s, f| {
                self.apply_item_modifier(f, s, rng, this, origin, depth + 1)
            }),
            Value::String(id) => match self.fns.store.item_modifiers.get(id) {
                Some(f) => self.apply_item_modifier(f, stack, rng, this, origin, depth + 1),
                None => stack,
            },
            Value::Object(_) => {
                let mut stack = stack?;
                // 26.3 item modifiers: a single `condition`, dispatched on `type`.
                if !m["condition"].is_null() && !self.conditions_hold(&m["condition"], this, origin)
                {
                    return Some(stack);
                }
                match m["type"].as_str().unwrap_or("") {
                    "minecraft:sequence" => {
                        #[allow(clippy::manual_try_fold)]
                        return m["functions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .fold(Some(stack), |s, f| {
                                self.apply_item_modifier(f, s, rng, this, origin, depth + 1)
                            });
                    }
                    "minecraft:set_count" => {
                        let n = int(&m["count"], rng);
                        let base = if m["add"].as_bool() == Some(true) {
                            stack.count as i32
                        } else {
                            0
                        };
                        let max = self
                            .res
                            .items()
                            .get(&stack.item)
                            .map_or(64, |i| i.max_stack);
                        stack.count = (base + n).clamp(0, max as i32) as u32;
                    }
                    "minecraft:limit_count" => {
                        let min = m["limit"]["min"].as_i64().or(m["limit"].as_i64());
                        let max = m["limit"]["max"].as_i64().or(m["limit"].as_i64());
                        if let Some(min) = min {
                            stack.count = stack.count.max(min as u32);
                        }
                        if let Some(max) = max {
                            stack.count = stack.count.min(max as u32);
                        }
                    }
                    "minecraft:set_damage" => {
                        let max = self
                            .res
                            .items()
                            .get(&stack.item)
                            .map_or(0, |i| i.max_damage);
                        if max > 0 {
                            let f = match m["damage"].as_f64() {
                                Some(f) => f as f32,
                                None => {
                                    let lo = m["damage"]["min"].as_f64().unwrap_or(0.) as f32;
                                    let hi = m["damage"]["max"].as_f64().unwrap_or(0.) as f32;
                                    lo + rng.next_float() * (hi - lo)
                                }
                            };
                            let base = if m["add"].as_bool() == Some(true) {
                                1. - stack.damage as f32 / max as f32
                            } else {
                                0.
                            };
                            let frac = (base + f).clamp(0., 1.);
                            stack.damage = ((1. - frac) * max as f32).floor() as u32;
                        }
                    }
                    "minecraft:set_name" => {
                        if m["name"].is_null() {
                            return Some(stack);
                        }
                        if m["target"].as_str() == Some("item_name") {
                            let name = horizon_client::scoreboard::json_to_tag(&m["name"]);
                            edit_components(&mut stack, |c| {
                                c.insert("minecraft:item_name".into(), name);
                            });
                        } else {
                            // SetNameFunction.run preserves the component when no entity resolver is requested.
                            stack
                                .data_mut()
                                .set_loot_name("minecraft:custom_name", &m["name"]);
                        }
                    }
                    "minecraft:set_enchantments" => {
                        let add = m["add"].as_bool() == Some(true);
                        let d = stack.extra.get_or_insert_with(Default::default);
                        let target = if &*stack.item == "minecraft:enchanted_book" {
                            &mut d.stored_enchantments
                        } else {
                            &mut d.enchantments
                        };
                        for (id, level) in m["enchantments"].as_object().into_iter().flatten() {
                            let n = int(level, rng).clamp(0, 255) as u32;
                            let id: std::sync::Arc<str> = if id.contains(':') {
                                id.as_str().into()
                            } else {
                                format!("minecraft:{id}").into()
                            };
                            match target.iter_mut().find(|(e, _)| *e == id) {
                                Some(e) => e.1 = if add { e.1 + n } else { n },
                                None => target.push((id, n)),
                            }
                            target.retain(|(_, l)| *l > 0);
                        }
                    }
                    "minecraft:set_potion" => {
                        if let Some(p) = m["id"].as_str() {
                            stack.extra.get_or_insert_with(Default::default).potion =
                                Some(p.into());
                        }
                    }
                    "minecraft:set_lore" => {
                        let lines: Vec<Tag> = m["lore"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(horizon_client::scoreboard::json_to_tag)
                            .collect();
                        edit_components(&mut stack, |c| {
                            let old: Vec<Tag> = match c.get(&"minecraft:lore".into()) {
                                Some(Tag::List(l)) => l.clone(),
                                _ => Vec::new(),
                            };
                            let new = list_operation(m, old, lines);
                            c.insert("minecraft:lore".into(), Tag::List(new));
                        });
                    }
                    "minecraft:set_custom_data" => {
                        let tag = match &m["tag"] {
                            Value::String(snbt) => horizon_world::nbt::parse_snbt(snbt)
                                .map(|(t, _)| t)
                                .unwrap_or(Tag::Compound(Default::default())),
                            v => horizon_client::scoreboard::json_to_tag(v),
                        };
                        edit_components(&mut stack, |c| {
                            let mut data = match c.get(&"minecraft:custom_data".into()) {
                                Some(t @ Tag::Compound(_)) => t.clone(),
                                _ => Tag::Compound(Default::default()),
                            };
                            merge_compound(&mut data, &tag);
                            c.insert("minecraft:custom_data".into(), data);
                        });
                    }
                    "minecraft:set_components" => {
                        let patch = m["components"].clone();
                        edit_components(&mut stack, |c| {
                            for (k, v) in patch.as_object().into_iter().flatten() {
                                let (remove, key) = match k.strip_prefix('!') {
                                    Some(rest) => (true, rest),
                                    None => (false, k.as_str()),
                                };
                                let key = if key.contains(':') {
                                    key.to_owned()
                                } else {
                                    format!("minecraft:{key}")
                                };
                                if remove {
                                    c.remove(&key.as_str().into());
                                } else {
                                    c.insert(
                                        key.as_str().into(),
                                        horizon_client::scoreboard::json_to_tag(v),
                                    );
                                }
                            }
                        });
                    }
                    "minecraft:toggle_tooltips" => {
                        let toggles = m["toggles"].clone();
                        edit_components(&mut stack, |c| {
                            let mut hidden: Vec<String> = c
                                .get(&"minecraft:tooltip_display".into())
                                .and_then(|d| d.get("hidden_components"))
                                .and_then(|l| match l {
                                    Tag::List(l) => Some(
                                        l.iter()
                                            .filter_map(|t| match t {
                                                Tag::String(s) => Some(s.to_string_lossy()),
                                                _ => None,
                                            })
                                            .collect(),
                                    ),
                                    _ => None,
                                })
                                .unwrap_or_default();
                            let hide_tooltip = c
                                .get(&"minecraft:tooltip_display".into())
                                .and_then(|d| d.get("hide_tooltip"))
                                .cloned();
                            for (k, shown) in toggles.as_object().into_iter().flatten() {
                                let key = if k.contains(':') {
                                    k.clone()
                                } else {
                                    format!("minecraft:{k}")
                                };
                                hidden.retain(|h| *h != key);
                                if shown.as_bool() == Some(false) {
                                    hidden.push(key);
                                }
                            }
                            let mut display: Vec<(&str, Tag)> = Vec::new();
                            if let Some(h) = hide_tooltip {
                                display.push(("hide_tooltip", h));
                            }
                            if !hidden.is_empty() {
                                display.push((
                                    "hidden_components",
                                    Tag::List(
                                        hidden
                                            .iter()
                                            .map(|h| Tag::String(h.as_str().into()))
                                            .collect(),
                                    ),
                                ));
                            }
                            c.insert("minecraft:tooltip_display".into(), Tag::compound(display));
                        });
                    }
                    "minecraft:filtered" => {
                        let pass = self.loot_item_filter(&m["item_filter"], &stack);
                        let branch = if pass { &m["on_pass"] } else { &m["on_fail"] };
                        if branch.is_null() {
                            return Some(stack);
                        }
                        return self.apply_item_modifier(
                            branch,
                            Some(stack),
                            rng,
                            this,
                            origin,
                            depth + 1,
                        );
                    }
                    "minecraft:discard" => stack.count = 0,
                    _ => {}
                }
                (stack.count > 0).then_some(stack)
            }
            _ => stack,
        }
    }
    /// `ItemStack.getMaxStackSize`: the max_stack_size component, else the
    /// item's.
    pub(in crate::game) fn stack_max_size(&self, s: &ItemStack) -> u32 {
        if let Some(Tag::Int(n)) = s.data().and_then(|d| d.other("minecraft:max_stack_size")) {
            return (*n).clamp(1, 99) as u32;
        }
        self.items().get(&s.item).map_or(64, |i| i.max_stack)
    }
    /// `ResourceOrIdArgument.getItemModifier`: an inline modifier (SNBT) or
    /// a registered one.
    pub(super) fn item_modifier_arg(&mut self, ctx: &Context, name: &str) -> R<serde_json::Value> {
        let raw = match ctx.get(name) {
            Some(Arg::Resource(s) | Arg::Str(s)) => s.clone(),
            _ => return Err(err("command.failed", &[])),
        };
        if raw.starts_with('{') || raw.starts_with('[') {
            let (tag, _) = horizon_world::nbt::parse_snbt(&raw)
                .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
            return Ok(horizon_client::scoreboard::tag_to_json(&tag));
        }
        let id = if raw.contains(':') {
            raw
        } else {
            format!("minecraft:{raw}")
        };
        self.fns
            .store
            .item_modifiers
            .get(&id)
            .cloned()
            .ok_or_else(|| err("argument.item_modifier.unknown", &[&id]))
    }
    /// `/item modify (entity|block) <target> <slot> <modifier>`.
    pub(super) fn cmd_item_modify(&mut self, ctx: &Context) -> R<i32> {
        self.ensure_datapacks();
        let targets = self.owner_of(ctx, "target", true)?;
        let Some(Arg::Str(slot_name)) = ctx.get("slot").cloned() else {
            return Ok(0);
        };
        let modifier = self.item_modifier_arg(ctx, "modifier")?;
        let source = self.command_source.unwrap_or_else(|| self.player_source());
        let view = source.entity.map(|t| self.entity_view_of(t));
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        let mut rng = horizon_core::random::LegacyRandom::new(seed);
        let mut tracker = Tracker::default();
        let mut selected = 0;
        for o in &targets {
            let slots = self.slots_named(o, &slot_name);
            if slots.is_empty() {
                if let Owner::Block(p) = o
                    && !self.world.block_entities.contains_key(&(p[0], p[1], p[2]))
                {
                    return Err(err(
                        "commands.item.target.not_a_container",
                        &[&p[0], &p[1], &p[2]],
                    ));
                }
                continue;
            }
            let mut changed = 0;
            for slot in &slots {
                // SlotSelector.NON_EMPTY_SLOTS
                let Some(current) = self.slot_get(slot) else {
                    continue;
                };
                selected += 1;
                let new = self.apply_item_modifier(
                    &modifier,
                    Some(current.clone()),
                    &mut rng,
                    view.as_ref(),
                    source.pos,
                    0,
                );
                // CountingModifier: every slot set counts, changed or not;
                // ItemStack.limitSize to the (new) max stack size.
                let new = new.map(|mut s| {
                    let max = self.stack_max_size(&s);
                    s.count = s.count.min(max);
                    s
                });
                if self.slot_set(slot, new) {
                    changed += 1;
                }
            }
            let h = self.owner_holder(o);
            tracker.track(&h, changed);
        }
        if selected == 0 {
            return Err(err("commands.item.target.no_such_slot", &[&slot_name]));
        }
        let pos = match targets.first() {
            Some(Owner::Block(p)) => Some(*p),
            _ => None,
        };
        let (message, total) = tracker.message(
            false,
            Some(err("commands.item.target.failed", &[])),
            |h, count| {
                let n = Text::literal(count.to_string());
                match pos {
                    Some(p) => Text::translate(
                        "commands.item.block.modify.success",
                        &[
                            n,
                            Text::literal(p[0].to_string()),
                            Text::literal(p[1].to_string()),
                            Text::literal(p[2].to_string()),
                        ],
                    ),
                    None => Text::translate(
                        "commands.item.entity.modify.success.single",
                        &[n, h.text.clone()],
                    ),
                }
            },
            |c, _| {
                Text::translate(
                    "commands.item.entity.modify.success.multiple",
                    &[Text::literal(c.to_string())],
                )
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    /// `/item replace|fill|override <target> <slots> (with <item> [count] | from <source> <slots>)`.
    pub(super) fn cmd_item(&mut self, ctx: &Context) -> R<i32> {
        let mode = ctx
            .literals
            .iter()
            .find(|l| matches!(l.as_str(), "replace" | "fill" | "override"))
            .cloned()
            .unwrap_or_default();
        let targets = self.owner_of(ctx, "target", true)?;
        let Some(Arg::Str(slot_name)) = ctx.get("slots").cloned() else {
            return Ok(0);
        };
        // The items to place.
        let (items, known): (Vec<Option<ItemStack>>, Option<ItemStack>) = if ctx.has("with") {
            let stack = self.item_from_input(ctx)?;
            (vec![stack.clone()], stack)
        } else {
            let sources = self.owner_of(ctx, "source", true)?;
            let Some(Arg::Str(source_slots)) = ctx.get("sourceSlots").cloned() else {
                return Ok(0);
            };
            let mut found = Vec::new();
            for o in &sources {
                for s in self.slots_named(o, &source_slots) {
                    found.push(self.slot_get(&s));
                }
            }
            if found.is_empty() {
                return Err(match &sources[..] {
                    [Owner::Block(p)] => err(
                        "commands.item.source.not_a_container",
                        &[&p[0], &p[1], &p[2]],
                    ),
                    _ => err("commands.item.source.no_such_slot", &[&source_slots]),
                });
            }
            // ItemCommands.applyModifier on each source stack.
            if ctx.get("modifier").is_some() {
                let modifier = self.item_modifier_arg(ctx, "modifier")?;
                let source = self.command_source.unwrap_or_else(|| self.player_source());
                let view = source.entity.map(|t| self.entity_view_of(t));
                let mut rng = horizon_core::random::LegacyRandom::new(0);
                found = found
                    .into_iter()
                    .map(|s| {
                        s.and_then(|s| {
                            self.apply_item_modifier(
                                &modifier,
                                Some(s),
                                &mut rng,
                                view.as_ref(),
                                source.pos,
                                0,
                            )
                        })
                    })
                    .collect();
            }
            // A source with nothing in the slot contributes air.
            (found, None)
        };
        let mut tracker = Tracker::default();
        let mut selected = 0;
        for o in &targets {
            let slots = self.slots_named(o, &slot_name);
            if slots.is_empty() {
                if let Owner::Block(p) = o
                    && !self.world.block_entities.contains_key(&(p[0], p[1], p[2]))
                {
                    return Err(err(
                        "commands.item.target.not_a_container",
                        &[&p[0], &p[1], &p[2]],
                    ));
                }
                continue;
            }
            selected += slots.len();
            let mut changed = 0;
            for (i, slot) in slots.iter().enumerate() {
                // `fill` cycles through the items; `replace` stops when they
                // run out; `override` empties the rest.
                let item = match mode.as_str() {
                    "fill" => items.get(i % items.len()).cloned().flatten(),
                    "override" => items.get(i).cloned().flatten(),
                    _ => match items.get(i) {
                        Some(x) => x.clone(),
                        None => break,
                    },
                };
                // ItemStack.isEmpty: air (or a count of 0) is an empty slot.
                // SlotAccess.set counts every slot it sets, the same item too.
                let item = item.filter(|s| &*s.item != "minecraft:air" && s.count > 0);
                if self.slot_set(slot, item) {
                    changed += 1;
                }
            }
            let h = self.owner_holder(o);
            tracker.track(&h, changed);
        }
        if selected == 0 {
            return Err(err("commands.item.target.no_such_slot", &[&slot_name]));
        }
        let block = matches!(targets.first(), Some(Owner::Block(_)));
        let pos = match targets.first() {
            Some(Owner::Block(p)) => *p,
            _ => [0; 3],
        };
        let zero = match &known {
            Some(k) => err(
                "commands.item.target.failed.known_item",
                &[&k.item.to_string()],
            ),
            None => err("commands.item.target.failed", &[]),
        };
        let name = known.as_ref().map(|k| self.stack_name(k));
        let (message, total) = tracker.message(
            false,
            Some(zero),
            |h, count| {
                let n = Text::literal(count.to_string());
                match (&name, block) {
                    (Some(item), true) => Text::translate(
                        "commands.item.block.replace.success.known_item",
                        &[
                            n,
                            Text::literal(pos[0].to_string()),
                            Text::literal(pos[1].to_string()),
                            Text::literal(pos[2].to_string()),
                            item.clone(),
                        ],
                    ),
                    (None, true) => Text::translate(
                        "commands.item.block.replace.success",
                        &[
                            n,
                            Text::literal(pos[0].to_string()),
                            Text::literal(pos[1].to_string()),
                            Text::literal(pos[2].to_string()),
                        ],
                    ),
                    (Some(item), false) => Text::translate(
                        "commands.item.entity.replace.success.single.known_item",
                        &[n, h.text.clone(), item.clone()],
                    ),
                    (None, false) => Text::translate(
                        "commands.item.entity.replace.success.single",
                        &[n, h.text.clone()],
                    ),
                }
            },
            |c, _| match &name {
                Some(item) => Text::translate(
                    "commands.item.entity.replace.success.multiple.known_item",
                    &[Text::literal(c.to_string()), item.clone()],
                ),
                None => Text::translate(
                    "commands.item.entity.replace.success.multiple",
                    &[Text::literal(c.to_string())],
                ),
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    /// `if items` and `if slots`: how many items match / how many slots.
    pub(super) fn condition_items(&mut self, c: &Context) -> R<(bool, i32)> {
        let owners = self.owner_of(c, "source", true)?;
        let Some(Arg::Str(slot_name)) = c.get("slots").cloned() else {
            return Ok((false, 0));
        };
        let predicate = match c.get("item_predicate") {
            Some(Arg::Item(p)) => Some(p.clone()),
            _ => None,
        };
        let mut count = 0;
        for o in owners {
            let slots = self.slots_named(&o, &slot_name);
            if predicate.is_none() {
                count += slots.len() as i32;
                continue;
            }
            for s in slots {
                if let Some(stack) = self.slot_get(&s)
                    && predicate
                        .as_ref()
                        .is_some_and(|p| self.stack_matches(p, &stack))
                {
                    count += stack.count as i32;
                }
            }
        }
        Ok((count > 0, count))
    }
}

/// Edit a stack's components as their saved compound.
fn edit_components(stack: &mut ItemStack, f: impl FnOnce(&mut horizon_world::nbt::Compound)) {
    let mut c = match horizon_client::item_data::encode_components(stack) {
        Some(Tag::Compound(c)) => c,
        _ => horizon_world::nbt::Compound::new(),
    };
    f(&mut c);
    let tag = Tag::Compound(c);
    let (damage, extra) = horizon_client::item_data::decode_components(Some(&tag));
    stack.damage = damage;
    stack.extra = extra;
    stack.tidy();
}
/// `CompoundTag.merge`: nested compounds merge, other values replace.
fn merge_compound(into: &mut Tag, from: &Tag) {
    let (Tag::Compound(a), Tag::Compound(b)) = (into, from) else {
        return;
    };
    for (k, v) in b {
        match (a.get_mut(k), v) {
            (Some(existing @ Tag::Compound(_)), Tag::Compound(_)) => merge_compound(existing, v),
            _ => {
                a.insert(k.clone(), v.clone());
            }
        }
    }
}
/// `ListOperation.apply` (`mode`: replace_all, replace_section, insert,
/// append) with a 256-entry limit.
fn list_operation(m: &serde_json::Value, old: Vec<Tag>, new: Vec<Tag>) -> Vec<Tag> {
    let offset = m["offset"].as_u64().unwrap_or(0) as usize;
    let mut out = match m["mode"].as_str().unwrap_or("") {
        "replace_all" => new,
        "insert" => {
            let at = offset.min(old.len());
            let mut out = old[..at].to_vec();
            out.extend(new);
            out.extend_from_slice(&old[at..]);
            out
        }
        "replace_section" => {
            let at = offset.min(old.len());
            let size = m["size"].as_u64().map_or(new.len(), |s| s as usize);
            let mut out = old[..at].to_vec();
            out.extend(new);
            let end = (at + size).min(old.len());
            out.extend_from_slice(&old[end..]);
            out
        }
        _ => {
            let mut out = old;
            out.extend(new);
            out
        }
    };
    out.truncate(256);
    out
}
