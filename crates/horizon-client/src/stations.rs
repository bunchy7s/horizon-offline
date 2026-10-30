//! Result rules of the workstation menus, ported from vanilla's
//! `EnchantmentMenu`, `AnvilMenu`, `GrindstoneMenu`, `SmithingMenu` (with
//! `SmithingTransformRecipe` / `SmithingTrimRecipe`) and `StonecutterMenu`.
//! Rendering and clicks live in the game; this module is pure item logic.
use crate::{
    crafting::Ingredient,
    enchant::Enchantments,
    item_data::{self, Levels},
    items::{ItemStack, Items},
};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::loot::PackOverlay;
use serde_json::Value;
use std::{fs, path::Path, sync::Arc};

/// Everything the workstation menus read, loaded once with the resources.
pub struct StationData {
    /// The same registry `Items` carries (`Items::enchantments`).
    pub enchantments: std::sync::Arc<Enchantments>,
    pub recipes: StationRecipes,
    pub brewing: crate::brewing::Brewing,
    pub trades: crate::merchant::Trades,
    pub banners: crate::banner::Banners,
    /// Every recipe's learnability and display count (the server recipe book).
    pub recipe_index: crate::recipe_book::RecipeIndex,
}
impl StationData {
    pub fn load(root: &Path, items: &Items, overlay: &PackOverlay) -> Self {
        Self {
            enchantments: std::sync::Arc::clone(&items.enchantments),
            recipes: StationRecipes::load(root, items, overlay),
            brewing: crate::brewing::Brewing::load(root, items, overlay),
            trades: crate::merchant::Trades::load(root),
            banners: crate::banner::Banners::load(root),
            recipe_index: crate::recipe_book::RecipeIndex::load(root, overlay),
        }
    }
}

// ---------------------------------------------------------------- enchanting

/// `EnchantingTableBlock.BOOKSHELF_OFFSETS`: the 5x5 ring two blocks out, two high.
pub fn bookshelf_offsets() -> Vec<[i32; 3]> {
    let mut v = Vec::new();
    for x in -2i32..=2 {
        for z in -2i32..=2 {
            if x.abs() == 2 || z.abs() == 2 {
                for y in 0..=1 {
                    v.push([x, y, z]);
                }
            }
        }
    }
    v
}
/// The three offers of `EnchantmentMenu.slotsChanged`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnchantOffers {
    pub costs: [i32; 3],
    /// Shown clue: enchantment and level (`enchantClue` / `levelClue`).
    pub clues: [Option<(Arc<str>, u32)>; 3],
}
/// `EnchantmentMenu.getEnchantmentList`.
pub fn enchantment_list(
    e: &Enchantments,
    items: &Items,
    seed: i32,
    slot: usize,
    cost: i32,
    item: &str,
) -> Levels {
    let mut random = LegacyRandom::new(i64::from(seed.wrapping_add(slot as i32)));
    let pool = e.tag("minecraft:in_enchanting_table").to_vec();
    let mut list = e.select(items, &mut random, item, cost, &pool);
    if item == "minecraft:book" && list.len() > 1 {
        let i = random.next_int_bound(list.len() as i32).unwrap_or(0) as usize;
        list.remove(i);
    }
    list
}
/// `EnchantmentMenu.slotsChanged` for an enchantable item.
pub fn enchant_offers(
    e: &Enchantments,
    items: &Items,
    seed: i32,
    bookcases: i32,
    stack: Option<&ItemStack>,
) -> EnchantOffers {
    let mut offers = EnchantOffers::default();
    let Some(stack) = stack else { return offers };
    // ItemStack.isEnchantable: enchantable component and no enchantments yet.
    if e.enchantable(&stack.item) == 0 || !stack.enchantments().is_empty() {
        return offers;
    }
    let mut random = LegacyRandom::new(i64::from(seed));
    for i in 0..3 {
        offers.costs[i] = e.enchantment_cost(&mut random, i, bookcases, &stack.item);
        if offers.costs[i] < i as i32 + 1 {
            offers.costs[i] = 0;
        }
    }
    for i in 0..3 {
        if offers.costs[i] > 0 {
            // getEnchantmentList reseeds the menu random; the clue draws continue from it.
            let mut list_random = LegacyRandom::new(i64::from(seed.wrapping_add(i as i32)));
            let pool = e.tag("minecraft:in_enchanting_table").to_vec();
            let mut list = e.select(items, &mut list_random, &stack.item, offers.costs[i], &pool);
            if &*stack.item == "minecraft:book" && list.len() > 1 {
                let r = list_random.next_int_bound(list.len() as i32).unwrap_or(0) as usize;
                list.remove(r);
            }
            if !list.is_empty() {
                let pick = list_random.next_int_bound(list.len() as i32).unwrap_or(0) as usize;
                offers.clues[i] = Some(list[pick].clone());
            }
        }
    }
    offers
}
/// `EnchantmentMenu.clickMenuButton` result: the enchanted item, or None.
pub fn apply_enchantment(
    e: &Enchantments,
    items: &Items,
    seed: i32,
    slot: usize,
    cost: i32,
    stack: &ItemStack,
) -> Option<ItemStack> {
    let list = enchantment_list(e, items, seed, slot, cost, &stack.item);
    if list.is_empty() {
        return None;
    }
    let mut out = if &*stack.item == "minecraft:book" {
        stack.transmute("minecraft:enchanted_book", stack.count)
    } else {
        stack.clone()
    };
    for (id, level) in list {
        out.enchant(&id, level);
    }
    Some(out)
}

// ---------------------------------------------------------------- anvil

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnvilResult {
    pub result: Option<ItemStack>,
    /// Level cost (the `cost` data slot).
    pub cost: i32,
    /// Material items consumed from the second slot on take.
    pub repair_item_count_cost: u32,
    pub only_renaming: bool,
}
/// `ItemStack.isDamageableItem`: durability, and not `minecraft:unbreakable`.
pub fn is_damageable(items: &Items, stack: &ItemStack) -> bool {
    items.get(&stack.item).is_some_and(|i| i.max_damage > 0)
        && stack
            .data()
            .is_none_or(|d| d.other("minecraft:unbreakable").is_none())
}
fn max_damage(items: &Items, stack: &ItemStack) -> i32 {
    items.get(&stack.item).map_or(0, |i| i.max_damage as i32)
}
/// `ItemStack.isValidRepairItem`.
pub fn is_valid_repair_item(
    e: &Enchantments,
    items: &Items,
    stack: &ItemStack,
    material: &ItemStack,
) -> bool {
    e.props(&stack.item)
        .is_some_and(|p| Enchantments::item_in(items, &p.repairable, &material.item))
}
/// `AnvilMenu.calculateIncreasedRepairCost`.
pub fn increased_repair_cost(cost: u32) -> u32 {
    (u64::from(cost) * 2 + 1).min(i32::MAX as u64) as u32
}
/// The display name the rename box starts with (`getHoverName`).
pub fn hover_name(names: &dyn Fn(&str) -> String, stack: &ItemStack) -> String {
    stack
        .custom_name()
        .or_else(|| stack.item_name())
        .unwrap_or_else(|| names(&stack.item))
}
/// `AnvilMenu.createResult`. `name` is the rename box text (None = untouched).
pub fn anvil_result(
    e: &Enchantments,
    items: &Items,
    input: Option<&ItemStack>,
    addition: Option<&ItemStack>,
    name: Option<&str>,
    hover_name: &str,
    creative: bool,
) -> AnvilResult {
    let fail = AnvilResult::default();
    let Some(input) = input else { return fail };
    // EnchantmentHelper.canStoreEnchantments: enchantable or enchanted book... every
    // item with an enchantments component, which is all of them in 26.3.
    let mut result = input.clone();
    let mut enchantments: Levels = input.crafting_enchantments().to_vec();
    let mut price: i64 = 0;
    let tax: i64 =
        i64::from(input.repair_cost()) + i64::from(addition.map_or(0, ItemStack::repair_cost));
    let mut repair_item_count_cost = 0;
    let mut naming_cost = 0;
    if let Some(addition) = addition {
        // addition.has(STORED_ENCHANTMENTS): an enchanted book (its default
        // component), or any item given stored enchantments.
        let using_book = &*addition.item == "minecraft:enchanted_book"
            || !addition.stored_enchantments().is_empty();
        if is_damageable(items, &result) && is_valid_repair_item(e, items, input, addition) {
            let max = max_damage(items, &result);
            let mut repair = (result.damage as i32).min(max / 4);
            if repair <= 0 {
                return fail;
            }
            let mut count = 0;
            while repair > 0 && count < addition.count {
                result.damage = (result.damage as i32 - repair) as u32;
                price += 1;
                repair = (result.damage as i32).min(max / 4);
                count += 1;
            }
            repair_item_count_cost = count;
        } else {
            if !using_book && (result.item != addition.item || !is_damageable(items, &result)) {
                return fail;
            }
            if is_damageable(items, &result) && !using_book {
                let max = max_damage(items, &result);
                let remaining1 = max - input.damage as i32;
                let remaining2 = max_damage(items, addition) - addition.damage as i32;
                let additional = remaining2 + max * 12 / 100;
                let remaining = remaining1 + additional;
                let damage = (max - remaining).max(0);
                if damage < result.damage as i32 {
                    result.damage = damage as u32;
                    price += 2;
                }
            }
            let mut any_compatible = false;
            let mut any_incompatible = false;
            for (id, level) in addition.crafting_enchantments() {
                let current = enchantments
                    .iter()
                    .find(|(x, _)| x == id)
                    .map_or(0, |(_, l)| *l);
                let mut level = if current == *level {
                    level + 1
                } else {
                    (*level).max(current)
                };
                let Some(def) = e.get(id) else { continue };
                let mut compatible = e.can_enchant(items, id, &input.item);
                if creative || &*input.item == "minecraft:enchanted_book" {
                    compatible = true;
                }
                for (other, _) in &enchantments {
                    if other != id && !e.compatible(id, other) {
                        compatible = false;
                        price += 1;
                    }
                }
                if !compatible {
                    any_incompatible = true;
                } else {
                    any_compatible = true;
                    level = level.min(def.max_level);
                    item_data::set_level(&mut enchantments, id, level);
                    let mut fee = def.anvil_cost as i64;
                    if using_book {
                        fee = (fee / 2).max(1);
                    }
                    price += fee * i64::from(level);
                    if input.count > 1 {
                        price = 40;
                    }
                }
            }
            if any_incompatible && !any_compatible {
                return fail;
            }
        }
    }
    match name {
        Some(n) if !n.trim().is_empty() => {
            if n != hover_name {
                naming_cost = 1;
                price += 1;
                result.data_mut().custom_name = Some(horizon_world::nbt::Tag::String(n.into()));
            }
        }
        _ => {
            if input.custom_name().is_some() {
                naming_cost = 1;
                price += 1;
                result.data_mut().custom_name = None;
            }
        }
    }
    let mut cost = if price <= 0 {
        0
    } else {
        (tax + price).clamp(0, i64::from(i32::MAX)) as i32
    };
    let mut out = (price > 0).then_some(result);
    let mut only_renaming = false;
    if naming_cost == price && naming_cost > 0 {
        if cost >= 40 {
            cost = 39;
        }
        only_renaming = true;
    }
    if cost >= 40 && !creative {
        out = None;
    }
    if let Some(r) = out.as_mut() {
        let mut base = r
            .repair_cost()
            .max(addition.map_or(0, ItemStack::repair_cost));
        if naming_cost != price || naming_cost == 0 {
            base = increased_repair_cost(base);
        }
        r.data_mut().repair_cost = base;
        r.set_crafting_enchantments(enchantments);
        r.tidy();
    }
    AnvilResult {
        result: out,
        cost,
        repair_item_count_cost,
        only_renaming,
    }
}

// ---------------------------------------------------------------- grindstone

/// `GrindstoneMenu` slot filter: damageable or enchanted.
pub fn grindstone_accepts(items: &Items, stack: &ItemStack) -> bool {
    is_damageable(items, stack) || stack.has_any_enchantments()
}
fn remove_non_curses(e: &Enchantments, mut item: ItemStack) -> ItemStack {
    let kept: Levels = item
        .crafting_enchantments()
        .iter()
        .filter(|(id, _)| e.is_curse(id))
        .cloned()
        .collect();
    let n = kept.len();
    item.set_crafting_enchantments(kept);
    if &*item.item == "minecraft:enchanted_book" && item.stored_enchantments().is_empty() {
        item = item.transmute("minecraft:book", item.count);
    }
    let mut cost = 0;
    for _ in 0..n {
        cost = increased_repair_cost(cost);
    }
    item.data_mut().repair_cost = cost;
    item.tidy();
    item
}
/// `GrindstoneMenu.computeResult`.
pub fn grindstone_result(
    e: &Enchantments,
    items: &Items,
    input: Option<&ItemStack>,
    additional: Option<&ItemStack>,
) -> Option<ItemStack> {
    if input.is_none() && additional.is_none() {
        return None;
    }
    if input.is_some_and(|s| s.count > 1) || additional.is_some_and(|s| s.count > 1) {
        return None;
    }
    match (input, additional) {
        (Some(a), Some(b)) => {
            if a.item != b.item {
                return None;
            }
            let durability = max_damage(items, a).max(max_damage(items, b));
            let remaining = (max_damage(items, a) - a.damage as i32)
                + (max_damage(items, b) - b.damage as i32)
                + durability * 5 / 100;
            let mut count = 1;
            if !is_damageable(items, a) {
                let stack_max = items.get(&a.item).map_or(64, |i| i.max_stack);
                if stack_max < 2 || a != b {
                    return None;
                }
                count = 2;
            }
            let mut out = ItemStack { count, ..a.clone() };
            if is_damageable(items, &out) {
                out.damage = (durability - remaining).max(0) as u32;
            }
            // mergeEnchantsFrom
            let mut levels = out.crafting_enchantments().to_vec();
            for (id, l) in b.crafting_enchantments() {
                if !e.is_curse(id) || !levels.iter().any(|(x, _)| x == id) {
                    item_data::upgrade(&mut levels, id, *l);
                }
            }
            out.set_crafting_enchantments(levels);
            Some(remove_non_curses(e, out))
        }
        (Some(s), None) | (None, Some(s)) => s
            .has_any_enchantments()
            .then(|| remove_non_curses(e, s.clone())),
        (None, None) => None,
    }
}
/// The grindstone result slot's `getExperienceAmount` before its roll: the
/// minimum costs of the inputs' non-curse enchantments.
pub fn grindstone_xp_base(
    e: &Enchantments,
    input: Option<&ItemStack>,
    additional: Option<&ItemStack>,
) -> i32 {
    let from = |s: Option<&ItemStack>| -> i32 {
        s.map_or(0, |s| {
            s.crafting_enchantments()
                .iter()
                .filter(|(id, _)| !e.is_curse(id))
                .map(|(id, l)| e.get(id).map_or(0, |d| d.min_cost.calculate(*l)))
                .sum()
        })
    };
    from(input) + from(additional)
}
/// `getExperienceAmount`'s roll of the sum: `half + nextInt(half)`.
pub fn grindstone_xp_roll(amount: i32, random: &mut LegacyRandom) -> u32 {
    if amount > 0 {
        let half = (f64::from(amount) / 2.0).ceil() as i32;
        (half + random.next_int_bound(half).unwrap_or(0)) as u32
    } else {
        0
    }
}
/// Experience from the grindstone result slot's `getExperienceAmount`.
pub fn grindstone_experience(
    e: &Enchantments,
    random: &mut LegacyRandom,
    input: Option<&ItemStack>,
    additional: Option<&ItemStack>,
) -> u32 {
    grindstone_xp_roll(grindstone_xp_base(e, input, additional), random)
}

// ---------------------------------------------------------------- smithing & stonecutting

#[derive(Clone, Debug)]
pub enum Smithing {
    Transform {
        id: String,
        template: Ingredient,
        base: Ingredient,
        addition: Ingredient,
        result: String,
    },
    Trim {
        id: String,
        template: Ingredient,
        base: Ingredient,
        addition: Ingredient,
        pattern: String,
    },
}
#[derive(Clone, Debug)]
pub struct Stonecutting {
    pub id: String,
    pub input: Ingredient,
    pub result: ItemStack,
}
#[derive(Default)]
pub struct StationRecipes {
    pub smithing: Vec<Smithing>,
    /// Sorted by recipe id, the registry order vanilla lists them in.
    pub stonecutting: Vec<Stonecutting>,
    /// `trim_material` registry: id -> (asset name, description).
    pub trim_materials: Vec<(String, String)>,
}
fn ingredient(v: &Value, items: &Items) -> Ingredient {
    Ingredient::parse(v, items).unwrap_or_else(|| Ingredient::of(Vec::new()))
}
impl StationRecipes {
    pub fn load(root: &Path, items: &Items, overlay: &PackOverlay) -> Self {
        // Vanilla's recipes, then the content mods' (docs/modpacks.md).
        let mut files = Vec::new();
        let namespaces =
            std::iter::once("minecraft".to_owned()).chain(crate::mod_content::namespaces(root));
        for namespace in namespaces {
            let mut more: Vec<_> = fs::read_dir(root.join(format!("data/{namespace}/recipe")))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect();
            more.sort();
            files.extend(more.into_iter().map(|p| (namespace.clone(), p)));
        }
        let mut sources: Vec<(String, Value)> = files
            .iter()
            .filter_map(|(namespace, p)| {
                let bytes = fs::read(p).ok()?;
                let v = serde_json::from_slice::<Value>(&bytes).ok()?;
                let id = format!(
                    "{namespace}:{}",
                    p.file_stem().and_then(|s| s.to_str()).unwrap_or("")
                );
                Some((id, v))
            })
            .collect();
        // The world's packs' recipes over the game's own: the top pack's
        // file for each id replaces it (`SimpleJsonResourceReloadListener`).
        for (id, files) in overlay.kind("recipe") {
            let Ok(v) = serde_json::from_slice::<Value>(PackOverlay::top(files)) else {
                continue;
            };
            match sources.iter_mut().find(|(n, _)| n == id) {
                Some(slot) => slot.1 = v,
                None => sources.push((id.to_owned(), v)),
            }
        }
        let mut smithing = Vec::new();
        let mut stonecutting = Vec::new();
        for (id, v) in sources {
            match v["type"].as_str().unwrap_or("") {
                "minecraft:smithing_transform" => smithing.push(Smithing::Transform {
                    id,
                    template: ingredient(&v["template"], items),
                    base: ingredient(&v["base"], items),
                    addition: ingredient(&v["addition"], items),
                    result: v["result"]["id"]
                        .as_str()
                        .unwrap_or("minecraft:air")
                        .to_owned(),
                }),
                "minecraft:smithing_trim" => smithing.push(Smithing::Trim {
                    id,
                    template: ingredient(&v["template"], items),
                    base: ingredient(&v["base"], items),
                    addition: ingredient(&v["addition"], items),
                    pattern: v["pattern"].as_str().unwrap_or("").to_owned(),
                }),
                "minecraft:stonecutting" => {
                    // A result count outside 1..=99 fails vanilla's codec: no recipe.
                    if let Some(count) = crate::crafting::template_count(&v["result"]) {
                        stonecutting.push(Stonecutting {
                            id,
                            input: ingredient(&v["ingredient"], items),
                            result: ItemStack::new(
                                v["result"]["id"].as_str().unwrap_or("minecraft:air"),
                                count,
                            ),
                        });
                    }
                }
                _ => {}
            }
        }
        let mut trim_materials = Vec::new();
        let mut tm: Vec<_> = fs::read_dir(root.join("data/minecraft/trim_material"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        tm.sort();
        for p in tm {
            if let Ok(bytes) = fs::read(&p)
                && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
            {
                let id = format!(
                    "minecraft:{}",
                    p.file_stem().and_then(|s| s.to_str()).unwrap_or("")
                );
                trim_materials.push((
                    id,
                    v["description"]["translate"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                ));
            }
        }
        Self {
            smithing,
            stonecutting,
            trim_materials,
        }
    }
    /// Recipes whose input accepts `stack`, in display order.
    pub fn stonecutter(&self, stack: &ItemStack) -> Vec<&Stonecutting> {
        self.stonecutting
            .iter()
            .filter(|r| r.input.test(stack))
            .collect()
    }
    fn slot_ok(&self, slot: usize, stack: &ItemStack) -> bool {
        self.smithing.iter().any(|r| {
            let (t, b, a) = match r {
                Smithing::Transform {
                    template,
                    base,
                    addition,
                    ..
                }
                | Smithing::Trim {
                    template,
                    base,
                    addition,
                    ..
                } => (template, base, addition),
            };
            [t, b, a][slot].test(stack)
        })
    }
    /// `RecipePropertySet` SMITHING_TEMPLATE / BASE / ADDITION.
    pub fn smithing_accepts(&self, slot: usize, stack: &ItemStack) -> bool {
        slot < 3 && self.slot_ok(slot, stack)
    }
    /// `SmithingMenu.createResult`.
    pub fn smithing_result(
        &self,
        e: &Enchantments,
        template: Option<&ItemStack>,
        base: Option<&ItemStack>,
        addition: Option<&ItemStack>,
    ) -> Option<ItemStack> {
        self.smithing_recipe(e, template, base, addition)
            .map(|(_, result)| result)
    }
    /// `SmithingMenu.createResult` with the id of the recipe that made the result.
    pub fn smithing_recipe(
        &self,
        e: &Enchantments,
        template: Option<&ItemStack>,
        base: Option<&ItemStack>,
        addition: Option<&ItemStack>,
    ) -> Option<(&str, ItemStack)> {
        let (template, base, addition) = (template?, base?, addition?);
        for r in &self.smithing {
            match r {
                Smithing::Transform {
                    id,
                    template: t,
                    base: b,
                    addition: a,
                    result,
                } => {
                    if t.test(template) && b.test(base) && a.test(addition) {
                        // SmithingTransformRecipe: transmute the base keeping its components.
                        return Some((id.as_str(), base.transmute(result, 1)));
                    }
                }
                Smithing::Trim {
                    id,
                    template: t,
                    base: b,
                    addition: a,
                    pattern,
                    ..
                } => {
                    if t.test(template) && b.test(base) && a.test(addition) {
                        return apply_trim(e, base, addition, pattern).map(|r| (id.as_str(), r));
                    }
                }
            }
        }
        None
    }
    /// `SmithingTrimDemoSlotDisplay.resolveForFirstStack` of a trim recipe:
    /// a random base trimmed with a random material (`pick(n)` draws below n).
    pub fn trim_demo(
        &self,
        e: &Enchantments,
        id: &str,
        pick: &mut dyn FnMut(usize) -> usize,
    ) -> Option<ItemStack> {
        let (base, addition, pattern) = self.smithing.iter().find_map(|r| match r {
            Smithing::Trim {
                id: i,
                base,
                addition,
                pattern,
                ..
            } if i == id => Some((&base.1, &addition.1, pattern)),
            _ => None,
        })?;
        if base.is_empty() || addition.is_empty() {
            return None;
        }
        // applyDemoTransformation: up to 256 draws, the first that isn't empty.
        (0..256).find_map(|_| {
            let base = ItemStack::new(&base[pick(base.len())], 1);
            let material = ItemStack::new(&addition[pick(addition.len())], 1);
            apply_trim(e, &base, &material, pattern)
        })
    }
}
/// `SmithingTrimRecipe.applyTrim`: none when the material provides no trim
/// or the base already has this one.
fn apply_trim(
    e: &Enchantments,
    base: &ItemStack,
    material: &ItemStack,
    pattern: &str,
) -> Option<ItemStack> {
    let material = e.props(&material.item)?.trim_material.clone()?;
    let trim = (Arc::from(material.as_str()), Arc::from(pattern));
    if base.data().and_then(|d| d.trim.as_ref()) == Some(&trim) {
        return None;
    }
    let mut out = ItemStack {
        count: 1,
        ..base.clone()
    };
    out.data_mut().trim = Some(trim);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enchant::tests::{sample, sample_items};

    fn sword(ench: &[(&str, u32)]) -> ItemStack {
        let mut s = ItemStack::new("minecraft:diamond_sword", 1);
        for (id, l) in ench {
            s.enchant(id, *l);
        }
        s
    }
    fn book(ench: &[(&str, u32)]) -> ItemStack {
        let mut s = ItemStack::new("minecraft:enchanted_book", 1);
        for (id, l) in ench {
            s.enchant(id, *l);
        }
        s
    }

    #[test]
    fn bookshelf_ring_has_32_positions() {
        let o = bookshelf_offsets();
        assert_eq!(o.len(), 32);
        assert!(o.contains(&[-2, 0, -2]) && o.contains(&[2, 1, 0]) && !o.contains(&[1, 0, 1]));
    }

    #[test]
    fn enchant_offers_are_deterministic_per_seed() {
        let e = sample();
        let items = sample_items();
        let s = ItemStack::new("minecraft:diamond_sword", 1);
        let a = enchant_offers(&e, &items, 1234, 15, Some(&s));
        let b = enchant_offers(&e, &items, 1234, 15, Some(&s));
        assert_eq!(a, b);
        assert_eq!(a.costs[2], 30, "{a:?}");
        assert!(a.costs[0] >= 1 && a.costs[0] <= a.costs[2]);
        // The clue is the first offer's random pick from the same list.
        for slot in 0..3 {
            let list = enchantment_list(
                &e,
                &items,
                1234,
                slot,
                a.costs[slot],
                "minecraft:diamond_sword",
            );
            let clue = a.clues[slot].clone().unwrap();
            assert!(list.contains(&clue));
            let applied = apply_enchantment(&e, &items, 1234, slot, a.costs[slot], &s).unwrap();
            assert_eq!(applied.enchantments(), &list[..]);
        }
        // Already enchanted items are not enchantable.
        assert_eq!(
            enchant_offers(
                &e,
                &items,
                1234,
                15,
                Some(&sword(&[("minecraft:sharpness", 1)]))
            )
            .costs,
            [0, 0, 0]
        );
        let books =
            apply_enchantment(&e, &items, 99, 2, 30, &ItemStack::new("minecraft:book", 1)).unwrap();
        assert_eq!(&*books.item, "minecraft:enchanted_book");
        assert!(!books.stored_enchantments().is_empty());
    }

    #[test]
    fn anvil_combines_and_charges_like_vanilla() {
        let e = sample();
        let items = sample_items();
        // Sharpness II + Sharpness II book = Sharpness III; book fee 1/2 -> max(1, 0) = 1 per level.
        let r = anvil_result(
            &e,
            &items,
            Some(&sword(&[("minecraft:sharpness", 2)])),
            Some(&book(&[("minecraft:sharpness", 2)])),
            None,
            "Diamond Sword",
            false,
        );
        let out = r.result.unwrap();
        assert_eq!(out.enchantment_level("minecraft:sharpness"), 3);
        assert_eq!(r.cost, 3);
        assert_eq!(out.repair_cost(), 1);
        // Incompatible only: no result.
        let r = anvil_result(
            &e,
            &items,
            Some(&sword(&[("minecraft:sharpness", 1)])),
            Some(&book(&[("minecraft:smite", 1)])),
            None,
            "Diamond Sword",
            false,
        );
        assert!(r.result.is_none());
        // Max level clamps: V + V stays V.
        let r = anvil_result(
            &e,
            &items,
            Some(&sword(&[("minecraft:sharpness", 5)])),
            Some(&book(&[("minecraft:sharpness", 5)])),
            None,
            "x",
            false,
        );
        assert_eq!(
            r.result.unwrap().enchantment_level("minecraft:sharpness"),
            5
        );
        assert_eq!(r.cost, 5);
    }

    #[test]
    fn anvil_rename_and_prior_work() {
        let e = sample();
        let items = sample_items();
        let mut s = sword(&[]);
        s.data_mut().repair_cost = 7;
        let r = anvil_result(
            &e,
            &items,
            Some(&s),
            None,
            Some("Excalibur"),
            "Diamond Sword",
            false,
        );
        assert_eq!(r.cost, 8);
        assert!(r.only_renaming);
        let out = r.result.unwrap();
        assert_eq!(out.custom_name().as_deref(), Some("Excalibur"));
        // Renaming alone does not raise the penalty.
        assert_eq!(out.repair_cost(), 7);
        // Same name: nothing to do.
        assert!(
            anvil_result(
                &e,
                &items,
                Some(&s),
                None,
                Some("Diamond Sword"),
                "Diamond Sword",
                false
            )
            .result
            .is_none()
        );
        // Too expensive at 40+ unless creative; rename-only caps at 39.
        s.data_mut().repair_cost = 63;
        let r = anvil_result(
            &e,
            &items,
            Some(&s),
            None,
            Some("A"),
            "Diamond Sword",
            false,
        );
        assert_eq!(r.cost, 39);
        assert!(r.result.is_some());
        let r = anvil_result(
            &e,
            &items,
            Some(&s),
            Some(&book(&[("minecraft:unbreaking", 1)])),
            None,
            "Diamond Sword",
            false,
        );
        assert!(r.cost >= 40 && r.result.is_none());
        assert!(
            anvil_result(
                &e,
                &items,
                Some(&s),
                Some(&book(&[("minecraft:unbreaking", 1)])),
                None,
                "Diamond Sword",
                true
            )
            .result
            .is_some()
        );
        assert_eq!(increased_repair_cost(0), 1);
        assert_eq!(increased_repair_cost(3), 7);
    }

    /// Checks against the prepared vanilla resources when they are present.
    #[test]
    fn prepared_vanilla_data() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let items = Items::tags_only(&root, &PackOverlay::NONE).unwrap();
        let data = StationData::load(&root, &items, &PackOverlay::NONE);
        let e = &data.enchantments;
        assert_eq!(e.list.len(), 43);
        assert_eq!(
            &*e.tag("minecraft:in_enchanting_table")[0],
            "minecraft:protection"
        );
        assert!(!e.compatible("minecraft:fortune", "minecraft:silk_touch"));
        assert!(e.is_curse("minecraft:binding_curse"));
        assert_eq!(e.full_name("minecraft:sharpness", 5).0, "Sharpness V");
        assert_eq!(e.enchantable("minecraft:golden_sword"), 22);
        // Brewing: water + nether wart = awkward.
        assert_eq!(data.brewing.recipes.len(), 279);
        let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
        let r = data
            .brewing
            .find(&water, &ItemStack::new("minecraft:nether_wart", 1))
            .unwrap();
        assert_eq!(r.output.potion(), Some("minecraft:awkward"));
        assert!(
            data.brewing
                .is_reagent(&ItemStack::new("minecraft:blaze_powder", 1))
        );
        // Smithing: netherite upgrade keeps enchantments.
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.enchant("minecraft:sharpness", 2);
        let out = data
            .recipes
            .smithing_result(
                e,
                Some(&ItemStack::new(
                    "minecraft:netherite_upgrade_smithing_template",
                    1,
                )),
                Some(&sword),
                Some(&ItemStack::new("minecraft:netherite_ingot", 1)),
            )
            .unwrap();
        assert_eq!(
            (&*out.item, out.enchantment_level("minecraft:sharpness")),
            ("minecraft:netherite_sword", 2)
        );
        assert!(
            data.recipes
                .stonecutter(&ItemStack::new("minecraft:stone", 1))
                .len()
                >= 10
        );
        // Trades: every profession level set yields offers.
        let ctx = crate::merchant::TradeContext {
            items: &items,
            enchantments: e,
            variant: "minecraft:plains",
            origin: [0.0; 3],
            maps: None,
        };
        for p in crate::merchant::PROFESSIONS {
            let set = crate::merchant::Trades::set_for(p, 1);
            let mut random = crate::merchant::sequence_random(
                12345,
                &format!(
                    "minecraft:trade_set/{}",
                    set.trim_start_matches("minecraft:")
                ),
            );
            assert!(
                !data
                    .trades
                    .offers_from_set(&ctx, &set, &mut random)
                    .is_empty(),
                "{p}"
            );
        }
    }

    #[test]
    fn grindstone_strips_non_curses() {
        let e = sample();
        let items = sample_items();
        let s = sword(&[("minecraft:sharpness", 3)]);
        let out = grindstone_result(&e, &items, Some(&s), None).unwrap();
        assert!(out.enchantments().is_empty());
        assert_eq!(out.repair_cost(), 0);
        assert!(out.extra.is_none());
        let b =
            grindstone_result(&e, &items, None, Some(&book(&[("minecraft:mending", 1)]))).unwrap();
        assert_eq!(&*b.item, "minecraft:book");
        assert!(grindstone_result(&e, &items, Some(&sword(&[])), None).is_none());
        let mut r = LegacyRandom::new(5);
        // min cost of Sharpness III = 23; ceil(23/2)=12 -> 12..23.
        let xp = grindstone_experience(&e, &mut r, Some(&s), None);
        assert!((12..24).contains(&xp));
    }
    #[test]
    fn smithing_ingredient_parses_holders() {
        let items = sample_items();
        let a = ingredient(&serde_json::json!("minecraft:diamond_sword"), &items);
        assert_eq!(a.1, vec!["minecraft:diamond_sword"]);
        let b = ingredient(
            &serde_json::json!(["minecraft:diamond_sword", "minecraft:diamond_axe"]),
            &items,
        );
        assert_eq!(
            b.1,
            vec!["minecraft:diamond_sword", "minecraft:diamond_axe"]
        );
    }
}
