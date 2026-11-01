//! A player's advancements on a server (PlayerAdvancements): on the
//! player's first tick the `minecraft:tick` criteria are met, the
//! advancements they complete grant their recipes (ServerRecipeBook.addRecipes:
//! ClientboundRecipeBookAddPacket) and the advancements the player may see go
//! out in ClientboundUpdateAdvancementsPacket (PlayerAdvancements.flushDirty).
use super::data::{SyncData, json_to_nbt};
use crate::{
    advancements::{Progress, Tree},
    recipe_book::{Category, RecipeIndex, Settings, crafting_category, crafting_station},
};
use horizon_protocol::buf::Writer;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

pub struct ServerAdvancements {
    tree: std::sync::Arc<Tree>,
    kids: Vec<Vec<usize>>,
    layout: Vec<Option<(i32, f32)>>,
    /// Recipe files by id.
    recipes: BTreeMap<String, Value>,
    displays: RecipeIndex,
    /// RecipeManager.unpackRecipeInfo's group numbers.
    groups: HashMap<String, i32>,
}

/// What the first tick sends: ClientboundRecipeBookAddPacket's payload (when
/// recipes were unlocked) and ClientboundUpdateAdvancementsPacket's.
pub struct FirstTick {
    pub recipe_book_add: Option<Vec<u8>>,
    pub update_advancements: Vec<u8>,
}

impl ServerAdvancements {
    pub fn load(resources: &Path, overlay: &horizon_world::loot::PackOverlay) -> Self {
        let tree = Tree::shared(&resources.join("data"), overlay);
        let kids = tree.children();
        let layout = tree.layout();
        let mut recipes = BTreeMap::new();
        super::recipes::recipes(&resources.join("data/minecraft/recipe"), "", &mut recipes);
        super::recipes::overlay_recipes(overlay, &mut recipes);
        let mut groups = HashMap::new();
        for r in recipes.values() {
            if let Some(g) = r["group"].as_str().filter(|g| !g.is_empty()) {
                let next = groups.len() as i32;
                groups.entry(g.to_owned()).or_insert(next);
            }
        }
        Self {
            tree,
            kids,
            layout,
            recipes,
            displays: RecipeIndex::load(resources, overlay),
            groups,
        }
    }

    /// The first tick of a player without advancements: the criteria
    /// `minecraft:tick` meets go into `progress` and the recipes their
    /// advancements reward into `book`.
    pub fn first_tick(
        &self,
        sync: &SyncData,
        now_millis: i64,
        progress: &mut Progress,
        book: &mut Settings,
    ) -> FirstTick {
        // CriteriaTriggers.TICK: the criteria it meets, and what they complete.
        let mut obtained: Vec<Vec<&str>> = vec![Vec::new(); self.tree.list.len()];
        for (i, a) in self.tree.list.iter().enumerate() {
            for c in &a.criteria {
                if c.trigger == "minecraft:tick" && c.conditions.is_null() {
                    obtained[i].push(&c.name);
                }
            }
        }
        let done: Vec<bool> = self
            .tree
            .list
            .iter()
            .zip(&obtained)
            .map(|(a, got)| {
                !a.requirements.is_empty()
                    && a.requirements
                        .iter()
                        .all(|group| group.iter().any(|c| got.contains(&c.as_str())))
            })
            .collect();
        let date = crate::advancements::date_text(now_millis.div_euclid(1000));
        for (a, got) in self.tree.list.iter().zip(&obtained) {
            for c in got {
                progress.grant(&a.id, c, &date);
            }
        }
        // AdvancementRewards.grant: the recipes, in the advancements' order.
        let mut entries = Vec::new();
        for (i, a) in self.tree.list.iter().enumerate() {
            if !done[i] {
                continue;
            }
            book.add_recipes(&self.displays, &a.rewards.recipes);
            for id in &a.rewards.recipes {
                entries.extend(self.book_entries(id, sync, NOTIFY_AND_HIGHLIGHT));
            }
        }
        let recipe_book_add = (!entries.is_empty()).then(|| book_add(entries, false));
        // flushDirty: every advancement the player may see, and its progress.
        let visible = self.tree.visible(&self.kids, |i| done[i]);
        let shown: Vec<usize> = (0..self.tree.list.len()).filter(|i| visible[*i]).collect();
        let mut w = Writer::new();
        w.bool(true); // the first packet resets the client's advancements
        w.varint(shown.len() as i32);
        for &i in &shown {
            self.write_advancement(&mut w, i, sync);
        }
        w.varint(0); // nothing removed
        w.varint(shown.len() as i32);
        for &i in &shown {
            self.write_progress(&mut w, i, |c| {
                obtained[i].contains(&c).then_some(now_millis)
            });
        }
        w.bool(true); // show the advancements
        FirstTick {
            recipe_book_add,
            update_advancements: w.data,
        }
    }

    /// `PlayerAdvancements.reload` (`PlayerList.reloadResources`): the
    /// progress is read again against the new tree (`applyFrom` ignores an
    /// advancement that no longer exists and `AdvancementProgress.update`
    /// drops a criterion it lost), the next `flushDirty` is a first packet
    /// again, and the next `CriteriaTriggers.TICK` meets the new tree's
    /// `minecraft:tick` criteria, rewarding the recipes of the advancements
    /// that complete. Returns the recipe book packet those recipes need.
    pub fn reload(
        &self,
        sync: &SyncData,
        now_millis: i64,
        progress: &mut Progress,
        book: &mut Settings,
    ) -> Option<Vec<u8>> {
        let by_id: HashMap<&str, &crate::advancements::Advancement> =
            self.tree.list.iter().map(|a| (a.id.as_str(), a)).collect();
        progress
            .criteria
            .retain(|id, got| match by_id.get(id.as_str()) {
                Some(a) => {
                    got.retain(|name, _| a.criteria.iter().any(|c| &c.name == name));
                    true
                }
                None => false,
            });
        let date = crate::advancements::date_text(now_millis.div_euclid(1000));
        let mut entries = Vec::new();
        for a in &self.tree.list {
            let was_done = progress.done(a);
            for c in &a.criteria {
                if c.trigger == "minecraft:tick" && c.conditions.is_null() {
                    progress.grant(&a.id, &c.name, &date);
                }
            }
            if !was_done && progress.done(a) {
                book.add_recipes(&self.displays, &a.rewards.recipes);
                for id in &a.rewards.recipes {
                    entries.extend(self.book_entries(id, sync, NOTIFY_AND_HIGHLIGHT));
                }
            }
        }
        (!entries.is_empty()).then(|| book_add(entries, false))
    }

    /// `PlayerAdvancements.flushDirty`: the advancements that became visible
    /// or hidden since `visible` (which it updates; empty before the first
    /// packet, which resets the client's and carries everything visible) and
    /// the progress of the `changed` ones still visible. None when there is
    /// nothing to send.
    pub fn update(
        &self,
        sync: &SyncData,
        progress: &Progress,
        visible: &mut Vec<bool>,
        changed: &BTreeSet<usize>,
        show: bool,
    ) -> Option<Vec<u8>> {
        let n = self.tree.list.len();
        let first = visible.len() != n;
        // flushDirty does nothing unless it is the first packet or progress
        // changed: visibility follows progress alone, and every grant and
        // revoke is in `changed`. This skips walking the whole tree for
        // every player every tick.
        if !first && changed.is_empty() {
            return None;
        }
        let now = self
            .tree
            .visible(&self.kids, |i| progress.done(&self.tree.list[i]));
        let was = |i: usize| !first && visible[i];
        let added: Vec<usize> = (0..n).filter(|&i| now[i] && !was(i)).collect();
        let removed: Vec<usize> = (0..n).filter(|&i| !now[i] && was(i)).collect();
        let progressed: Vec<usize> = if first {
            (0..n).filter(|&i| now[i]).collect()
        } else {
            changed
                .iter()
                .copied()
                .filter(|&i| i < n && now[i])
                .collect()
        };
        *visible = now;
        if !first && added.is_empty() && removed.is_empty() && progressed.is_empty() {
            return None;
        }
        let mut w = Writer::new();
        w.bool(first);
        w.varint(added.len() as i32);
        for &i in &added {
            self.write_advancement(&mut w, i, sync);
        }
        w.varint(removed.len() as i32);
        for &i in &removed {
            w.string(&self.tree.list[i].id);
        }
        w.varint(progressed.len() as i32);
        for &i in &progressed {
            let id = &self.tree.list[i].id;
            self.write_progress(&mut w, i, |c| {
                progress
                    .criteria
                    .get(id)
                    .and_then(|m| m.get(c))
                    .map(|date| crate::advancements::date_millis(date).unwrap_or(0))
            });
        }
        w.bool(show);
        Some(w.data)
    }

    /// Which advancements a player with `progress` may see.
    pub fn visibility(&self, progress: &Progress) -> Vec<bool> {
        self.tree
            .visible(&self.kids, |i| progress.done(&self.tree.list[i]))
    }

    /// `ClientboundUpdateAdvancementsPacket.PositionedAdvancement`.
    fn write_advancement(&self, w: &mut Writer, i: usize, sync: &SyncData) {
        let a = &self.tree.list[i];
        w.string(&a.id);
        w.bool(a.parent.is_some());
        if let Some(p) = &a.parent {
            w.string(p);
        }
        w.bool(a.display.is_some());
        if let Some(d) = &a.display {
            w.nbt(&json_to_nbt(&d.title));
            w.nbt(&json_to_nbt(&d.description));
            w.varint(sync.id("minecraft:item", &d.icon).unwrap_or(0));
            w.varint(1);
            w.varint(0);
            w.varint(0);
            w.varint(match d.frame {
                crate::advancements::Frame::Task => 0,
                crate::advancements::Frame::Challenge => 1,
                crate::advancements::Frame::Goal => 2,
            });
            w.i32(
                i32::from(d.background.is_some())
                    | i32::from(d.show_toast) << 1
                    | i32::from(d.hidden) << 2,
            );
            if let Some(b) = &d.background {
                w.string(b);
            }
        }
        w.varint(a.requirements.len() as i32);
        for group in &a.requirements {
            w.varint(group.len() as i32);
            for c in group {
                w.string(c);
            }
        }
        w.bool(false); // sends_telemetry_event
        let (x, y) = self.layout[i].unwrap_or((0, 0.0));
        w.f32(x as f32);
        w.f32(y);
    }

    /// An `AdvancementProgress`: each criterion and when it was obtained.
    fn write_progress(&self, w: &mut Writer, i: usize, obtained: impl Fn(&str) -> Option<i64>) {
        let a = &self.tree.list[i];
        w.string(&a.id);
        w.varint(a.criteria.len() as i32);
        for c in &a.criteria {
            w.string(&c.name);
            let at = obtained(&c.name);
            w.bool(at.is_some());
            if let Some(at) = at {
                w.i64(at);
            }
        }
    }

    /// `ServerRecipeBook.addRecipes`' packet for recipes just unlocked:
    /// each display, with its notification and highlighted. None when no
    /// recipe has a display.
    pub fn recipes_added(&self, ids: &[String], sync: &SyncData) -> Option<Vec<u8>> {
        let entries: Vec<Vec<u8>> = ids
            .iter()
            .flat_map(|id| self.book_entries(id, sync, NOTIFY_AND_HIGHLIGHT))
            .collect();
        (!entries.is_empty()).then(|| book_add(entries, false))
    }

    /// `ServerRecipeBook.sendInitialRecipeBook`: every known recipe, without
    /// notifications, highlighted while not yet displayed, replacing the
    /// client's book.
    pub fn initial_recipe_book(&self, book: &Settings, sync: &SyncData) -> Vec<u8> {
        let entries: Vec<Vec<u8>> = book
            .recipes
            .iter()
            .flat_map(|id| {
                let highlight = if book.to_be_displayed.contains(id) {
                    2
                } else {
                    0
                };
                self.book_entries(id, sync, highlight)
            })
            .collect();
        book_add(entries, true)
    }

    /// The recipe's display for ClientboundPlaceGhostRecipePacket (the
    /// recipes the book lists).
    pub fn ghost_display(
        &self,
        id: &str,
        sync: &SyncData,
    ) -> Option<horizon_protocol::packets::recipes::RecipeDisplay> {
        let bytes = self.book_entries(id, sync, 0).into_iter().next()?;
        let mut r = horizon_protocol::buf::Reader::new(&bytes);
        horizon_protocol::packets::recipes::RecipeBookEntry::read(&mut r)
            .ok()
            .map(|e| e.display)
    }

    /// ClientboundRecipeBookAddPacket.Entry for each display of a recipe:
    /// highlighted, and with a notification unless the recipe turns it off.
    fn book_entries(&self, id: &str, sync: &SyncData, flags: i8) -> Vec<Vec<u8>> {
        let Some(r) = self.recipes.get(id) else {
            return Vec::new();
        };
        let ids = self.displays.display_ids(id);
        if ids.len() != 1 {
            // Special recipes have no display; transmutations aren't unlocked on joining.
            return Vec::new();
        }
        let kind = r["type"].as_str().unwrap_or("");
        let item = |n: &str| sync.id("minecraft:item", n).unwrap_or(0);
        let slot = |n: &str| sync.id("minecraft:slot_display", n).unwrap_or(0);
        let mut w = Writer::new();
        w.varint(ids.start);
        // The ingredients' TagSlotDisplays, an empty cell as an empty slot.
        let slots = |w: &mut Writer, cells: &[Option<&Value>]| {
            w.varint(cells.len() as i32);
            for cell in cells {
                match cell {
                    Some(v) => {
                        w.varint(slot("minecraft:tag"));
                        holder_set(w, v, &item);
                    }
                    None => w.varint(slot("minecraft:empty")),
                }
            }
        };
        // The result as its ItemStackSlotDisplay, the station as its item.
        let result = |w: &mut Writer| {
            w.varint(slot("minecraft:item_stack"));
            w.varint(item(
                horizon_world::loot::recipe_result_id(&r["result"]).unwrap_or("minecraft:air"),
            ));
            w.varint(r["result"]["count"].as_i64().unwrap_or(1) as i32);
            w.varint(0);
            w.varint(0);
        };
        let station = |w: &mut Writer, name: &str| {
            w.varint(slot("minecraft:item"));
            w.varint(item(name));
        };
        // The display, then the ingredients the craftable filter checks and
        // the category of the tab that lists it.
        let (category, placed): (&str, Vec<&Value>) = match kind {
            "minecraft:crafting_shaped" => {
                let pattern = shrink(r["pattern"].as_array().map_or(&[][..], |p| p));
                let (width, height) = (pattern.first().map_or(0, Vec::len), pattern.len());
                w.varint(sync.id("minecraft:recipe_display", kind).unwrap_or(0));
                w.varint(width as i32);
                w.varint(height as i32);
                let cells: Vec<Option<&Value>> = pattern
                    .iter()
                    .flatten()
                    .map(|c| (*c != ' ').then(|| &r["key"][c.to_string()]))
                    .collect();
                slots(&mut w, &cells);
                result(&mut w);
                station(&mut w, "minecraft:crafting_table");
                let placed = cells.iter().flatten().copied().collect();
                (
                    crafting_category(r["category"].as_str().unwrap_or("")).name(),
                    placed,
                )
            }
            "minecraft:crafting_shapeless" => {
                w.varint(sync.id("minecraft:recipe_display", kind).unwrap_or(0));
                let cells: Vec<Option<&Value>> = r["ingredients"]
                    .as_array()
                    .map_or(Vec::new(), |a| a.iter().map(Some).collect());
                slots(&mut w, &cells);
                result(&mut w);
                station(&mut w, "minecraft:crafting_table");
                let placed = cells.iter().flatten().copied().collect();
                (
                    crafting_category(r["category"].as_str().unwrap_or("")).name(),
                    placed,
                )
            }
            // AbstractCookingRecipe.display: the furnace, blast furnace,
            // smoker and campfire share one FurnaceRecipeDisplay; only the
            // station and the category differ.
            "minecraft:smelting"
            | "minecraft:blasting"
            | "minecraft:smoking"
            | "minecraft:campfire_cooking" => {
                w.varint(
                    sync.id("minecraft:recipe_display", "minecraft:furnace")
                        .unwrap_or(0),
                );
                w.varint(slot("minecraft:tag"));
                holder_set(&mut w, &r["ingredient"], &item);
                w.varint(slot("minecraft:any_fuel"));
                result(&mut w);
                station(&mut w, crafting_station(kind));
                w.varint(r["cookingtime"].as_i64().unwrap_or(0) as i32);
                w.f32(r["experience"].as_f64().unwrap_or(0.0) as f32);
                (cooking_category(kind, r), vec![&r["ingredient"]])
            }
            _ => return Vec::new(),
        };
        // The group, the recipe book category and the placement's ingredients.
        match r["group"].as_str().and_then(|g| self.groups.get(g)) {
            Some(g) => w.varint(g + 1),
            None => w.varint(0),
        }
        let category = format!("minecraft:{category}");
        w.varint(
            sync.id("minecraft:recipe_book_category", &category)
                .unwrap_or(0),
        );
        w.bool(true);
        w.varint(placed.len() as i32);
        for v in placed {
            holder_set(&mut w, v, &item);
        }
        // The notification only where the recipe shows one.
        let notify = r["show_notification"].as_bool().unwrap_or(true);
        w.i8(if notify { flags } else { flags & !1 });
        vec![w.data]
    }
}

/// ClientboundRecipeBookAddPacket.Entry flags: notification, highlight.
const NOTIFY_AND_HIGHLIGHT: i8 = 1 | 2;

/// ClientboundRecipeBookAddPacket: the entries, then whether they replace the book.
fn book_add(entries: Vec<Vec<u8>>, replace: bool) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(entries.len() as i32);
    for e in entries {
        w.data.extend(e);
    }
    w.bool(replace);
    w.data
}

/// `SmeltingRecipe.recipeBookCategory` and the other cooking recipes': the
/// category of the station's book, from the recipe's `CookingBookCategory`.
fn cooking_category(kind: &str, r: &Value) -> &'static str {
    let category = match kind {
        // BlastingRecipe: a blocks tab, and one misc tab for the rest.
        "minecraft:blasting" => match r["category"].as_str() {
            Some("blocks") => Category::BlastBlocks,
            _ => Category::BlastMisc,
        },
        "minecraft:smoking" => Category::SmokerFood,
        // CampfireCookingRecipe: its own category, which no book's tab lists.
        "minecraft:campfire_cooking" => return "campfire",
        // SmeltingRecipe: blocks, food and misc, the default.
        _ => match r["category"].as_str() {
            Some("food") => Category::FurnaceFood,
            Some("blocks") => Category::FurnaceBlocks,
            _ => Category::FurnaceMisc,
        },
    };
    category.name()
}

/// ShapedRecipePattern.shrink: the pattern without blank border rows and columns.
fn shrink(rows: &[Value]) -> Vec<Vec<char>> {
    let rows: Vec<Vec<char>> = rows
        .iter()
        .map(|r| r.as_str().unwrap_or("").chars().collect())
        .collect();
    let used = |r: &Vec<char>| r.iter().any(|c| *c != ' ');
    let (Some(top), Some(bottom)) = (rows.iter().position(used), rows.iter().rposition(used))
    else {
        return Vec::new();
    };
    let rows = &rows[top..=bottom];
    let left = rows
        .iter()
        .filter_map(|r| r.iter().position(|c| *c != ' '))
        .min()
        .unwrap_or(0);
    let right = rows
        .iter()
        .filter_map(|r| r.iter().rposition(|c| *c != ' '))
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|r| {
            (left..=right)
                .map(|x| r.get(x).copied().unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// ByteBufCodecs.holderSet for an ingredient: a tag by name, or the items.
fn holder_set(w: &mut Writer, v: &Value, item: &dyn Fn(&str) -> i32) {
    match v {
        Value::String(s) if s.starts_with('#') => {
            w.varint(0);
            w.string(&s[1..]);
        }
        Value::String(s) => {
            w.varint(2);
            w.varint(item(s));
        }
        Value::Array(a) => {
            w.varint(a.len() as i32 + 1);
            for s in a {
                w.varint(item(s.as_str().unwrap_or("")));
            }
        }
        _ => w.varint(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::{
        buf::Reader,
        item::Stack,
        packets::recipes::{HolderSet, RecipeBookEntry, RecipeDisplay, SlotDisplay},
    };
    use std::path::PathBuf;

    /// The registries `book_entries` looks up, in the game's order
    /// (`RecipeBookCategories` and the two display registries).
    const REGISTRIES: &str = r#"{
      "minecraft:item": {"entries": {
        "minecraft:raw_copper": {"protocol_id": 0},
        "minecraft:copper_ingot": {"protocol_id": 1},
        "minecraft:redstone_ore": {"protocol_id": 2},
        "minecraft:redstone": {"protocol_id": 3},
        "minecraft:furnace": {"protocol_id": 4},
        "minecraft:blast_furnace": {"protocol_id": 5}
      }},
      "minecraft:slot_display": {"entries": {
        "minecraft:empty": {"protocol_id": 0},
        "minecraft:any_fuel": {"protocol_id": 1},
        "minecraft:with_any_potion": {"protocol_id": 2},
        "minecraft:only_with_component": {"protocol_id": 3},
        "minecraft:item": {"protocol_id": 4},
        "minecraft:item_stack": {"protocol_id": 5},
        "minecraft:tag": {"protocol_id": 6},
        "minecraft:dyed": {"protocol_id": 7},
        "minecraft:smithing_trim": {"protocol_id": 8},
        "minecraft:with_remainder": {"protocol_id": 9},
        "minecraft:composite": {"protocol_id": 10}
      }},
      "minecraft:recipe_display": {"entries": {
        "minecraft:crafting_shapeless": {"protocol_id": 0},
        "minecraft:crafting_shaped": {"protocol_id": 1},
        "minecraft:furnace": {"protocol_id": 2},
        "minecraft:stonecutter": {"protocol_id": 3},
        "minecraft:smithing": {"protocol_id": 4}
      }},
      "minecraft:recipe_book_category": {"entries": {
        "minecraft:crafting_building_blocks": {"protocol_id": 0},
        "minecraft:crafting_redstone": {"protocol_id": 1},
        "minecraft:crafting_equipment": {"protocol_id": 2},
        "minecraft:crafting_misc": {"protocol_id": 3},
        "minecraft:furnace_food": {"protocol_id": 4},
        "minecraft:furnace_blocks": {"protocol_id": 5},
        "minecraft:furnace_misc": {"protocol_id": 6},
        "minecraft:blast_furnace_blocks": {"protocol_id": 7},
        "minecraft:blast_furnace_misc": {"protocol_id": 8},
        "minecraft:smoker_food": {"protocol_id": 9},
        "minecraft:stonecutter": {"protocol_id": 10},
        "minecraft:smithing": {"protocol_id": 11},
        "minecraft:campfire": {"protocol_id": 12}
      }}
    }"#;

    /// A resources dir with those registries and the given recipe files.
    fn resources(name: &str, recipes: &[(&str, &str)]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("horizon-recipe-book-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("data/minecraft/recipe");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(root.join("registries.json"), REGISTRIES).unwrap();
        for (id, json) in recipes {
            std::fs::write(dir.join(format!("{id}.json")), json).unwrap();
        }
        root
    }

    /// The server's sync over `root`, and the one entry of `id` read back as
    /// the client's `RecipeBookEntry::read` reads it.
    fn entry(root: &std::path::Path, id: &str) -> (SyncData, RecipeBookEntry) {
        let sync = SyncData::load(root, &horizon_world::loot::PackOverlay::NONE).unwrap();
        let entries = sync
            .advancements
            .book_entries(id, &sync, NOTIFY_AND_HIGHLIGHT);
        assert_eq!(entries.len(), 1, "{id} has one display");
        let mut r = Reader::new(&entries[0]);
        let entry = RecipeBookEntry::read(&mut r).unwrap();
        assert_eq!(r.remaining(), 0, "the entry ends with its flags");
        (sync, entry)
    }

    #[test]
    fn smelting_reaches_the_furnace_book() {
        let root = resources(
            "smelting",
            &[(
                "copper_ingot_from_smelting_raw_copper",
                r#"{"type":"minecraft:smelting","cookingtime":200,"experience":0.7,
                    "group":"copper_ingot","ingredient":"minecraft:raw_copper",
                    "result":{"id":"minecraft:copper_ingot"}}"#,
            )],
        );
        let (sync, e) = entry(&root, "minecraft:copper_ingot_from_smelting_raw_copper");
        let item = |n: &str| sync.id("minecraft:item", n).unwrap();
        // The fixture's only recipe: display ids are numbered over the sorted ids.
        assert_eq!(e.id, 0);
        assert_eq!(
            e.category,
            sync.id("minecraft:recipe_book_category", "minecraft:furnace_misc")
                .unwrap()
        );
        assert_eq!(e.group, 1, "copper_ingot is the first group");
        assert_eq!(e.flags, NOTIFY_AND_HIGHLIGHT);
        assert_eq!(
            e.requirements,
            Some(vec![HolderSet::Items(vec![item("minecraft:raw_copper")])])
        );
        let RecipeDisplay::Furnace {
            ingredient,
            fuel,
            result,
            station,
            duration,
            experience,
        } = e.display
        else {
            panic!("a smelting recipe shows a furnace display");
        };
        assert_eq!(
            ingredient,
            SlotDisplay::Tag(HolderSet::Items(vec![item("minecraft:raw_copper")]))
        );
        assert_eq!(fuel, SlotDisplay::AnyFuel);
        assert_eq!(
            result,
            SlotDisplay::ItemStack(Stack::new(item("minecraft:copper_ingot"), 1))
        );
        assert_eq!(station, SlotDisplay::Item(item("minecraft:furnace")));
        assert_eq!(duration, 200);
        assert_eq!(experience, 0.7);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn blasting_reaches_the_blast_furnace_book() {
        let root = resources(
            "blasting",
            &[(
                "redstone_from_blasting_redstone_ore",
                r#"{"type":"minecraft:blasting","category":"blocks","cookingtime":200,
                    "experience":0.7,"group":"redstone",
                    "ingredient":"minecraft:redstone_ore","result":{"id":"minecraft:redstone"}}"#,
            )],
        );
        let (sync, e) = entry(&root, "minecraft:redstone_from_blasting_redstone_ore");
        let item = |n: &str| sync.id("minecraft:item", n).unwrap();
        assert_eq!(e.id, 0);
        assert_eq!(
            e.category,
            sync.id(
                "minecraft:recipe_book_category",
                "minecraft:blast_furnace_blocks"
            )
            .unwrap()
        );
        let RecipeDisplay::Furnace {
            ingredient,
            station,
            duration,
            experience,
            ..
        } = e.display
        else {
            panic!("a blasting recipe shows a furnace display");
        };
        assert_eq!(
            ingredient,
            SlotDisplay::Tag(HolderSet::Items(vec![item("minecraft:redstone_ore")]))
        );
        assert_eq!(station, SlotDisplay::Item(item("minecraft:blast_furnace")));
        assert_eq!(duration, 200);
        assert_eq!(experience, 0.7);
        let _ = std::fs::remove_dir_all(&root);
    }
}
