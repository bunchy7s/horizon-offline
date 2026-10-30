//! `ServerPlaceRecipe`: what the server does with a recipe book click
//! (`ServerboundPlaceRecipePacket`) in the inventory, crafting table and
//! furnace menus, with `StackedContents`' exact matching of items to
//! ingredients (`RecipePicker`). `tools/test_place_recipe_parity.py`
//! compares every placement with vanilla's.
//!
//! Vanilla keeps the available items in a `Reference2IntOpenHashMap` keyed by
//! item holders, whose iteration order follows their identity hash codes: it
//! differs between runs of the game, and decides which of several fitting
//! items fills an ingredient. `Stacked` keeps an explicit order: the order
//! items were first counted, or one given (the harness passes the oracle's).
use super::{Env, GRID, Kind, MAIN, Menu, PLAYER, RESULT, split, tidy};
use crate::{
    crafting::{Ingredient, Placement},
    items::ItemStack,
};
use std::{collections::HashMap, sync::Arc};

/// `StackedContents<Holder<Item>>`: item counts in an iteration order.
#[derive(Clone, Debug, Default)]
pub struct Stacked {
    keys: Vec<Arc<str>>,
    amounts: Vec<i32>,
    index: HashMap<Arc<str>, usize>,
}

/// `Inventory.isUsableForCrafting`: not damaged, enchanted or renamed.
pub fn usable_for_crafting(stack: &ItemStack) -> bool {
    stack.damage == 0 && stack.enchantments().is_empty() && stack.custom_name().is_none()
}

impl Stacked {
    /// `StackedContents.account`.
    pub fn account(&mut self, item: &Arc<str>, count: i32) {
        match self.index.get(item) {
            Some(&i) => self.amounts[i] += count,
            None => {
                self.index.insert(item.clone(), self.keys.len());
                self.keys.push(item.clone());
                self.amounts.push(count);
            }
        }
    }
    /// `StackedItemContents.accountStack(stack, maxCount)`.
    pub fn account_stack(&mut self, stack: &ItemStack, max: u32) {
        if stack.count > 0 {
            self.account(&stack.item, stack.count.min(max) as i32);
        }
    }
    /// `accountSimpleStack`.
    pub fn account_simple(&mut self, stack: &ItemStack, max: u32) {
        if usable_for_crafting(stack) {
            self.account_stack(stack, max);
        }
    }
    /// Reorders the keys: the given items first, in that order (vanilla's hash
    /// map order for the same items), then the rest as they were.
    pub fn set_order(&mut self, order: &[String]) {
        let mut keys: Vec<Arc<str>> = order
            .iter()
            .filter_map(|o| self.index.get_key_value(o.as_str()).map(|(k, _)| k.clone()))
            .collect();
        for k in &self.keys {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        let amounts = keys.iter().map(|k| self.amounts[self.index[k]]).collect();
        self.index = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| (k, i))
            .collect();
        self.keys = keys;
        self.amounts = amounts;
    }
    pub fn keys(&self) -> &[Arc<str>] {
        &self.keys
    }
    /// `canCraft(ingredients, null)` without touching the counts (the recipe
    /// book's craftable test; the answer does not depend on the order; no
    /// ingredients at all can always be found).
    pub fn can_craft(&self, ingredients: &[&Ingredient]) -> bool {
        self.clone().try_pick(ingredients, 1, None)
    }
    /// `getResultUpperBound`.
    fn upper_bound(&self, ingredients: &[&Ingredient]) -> i32 {
        let mut min = i32::MAX;
        'ingredients: for ing in ingredients {
            let mut max = 0;
            for (key, &count) in self.keys.iter().zip(&self.amounts) {
                if count > max {
                    if ing.0.contains(&**key) {
                        max = count;
                    }
                    if max >= min {
                        continue 'ingredients;
                    }
                }
            }
            min = max;
            if min == 0 {
                break;
            }
        }
        min
    }
    /// `tryPick` (`canCraft(recipe, amount, output)`).
    pub fn try_pick(
        &mut self,
        ingredients: &[&Ingredient],
        amount: i32,
        output: Option<&mut Vec<Arc<str>>>,
    ) -> bool {
        Picker::new(self, ingredients).try_pick(amount, output)
    }
    /// `tryPickAll` (`getBiggestCraftableStack`).
    pub fn try_pick_all(
        &mut self,
        ingredients: &[&Ingredient],
        max_size: i32,
        output: Option<&mut Vec<Arc<str>>>,
    ) -> i32 {
        Picker::new(self, ingredients).try_pick_all(max_size, output)
    }
}

/// `StackedContents.RecipePicker`: augmenting paths over item-ingredient pairs.
struct Picker<'a> {
    stacked: &'a mut Stacked,
    ingredients: Vec<&'a Ingredient>,
    /// Indices into the stacked keys of the items any ingredient accepts.
    items: Vec<usize>,
    connection: Vec<bool>,
    assigned: Vec<bool>,
    satisfied: Vec<bool>,
    visited_ingredient: Vec<bool>,
    visited_item: Vec<bool>,
    path: Vec<usize>,
}

impl<'a> Picker<'a> {
    fn new(stacked: &'a mut Stacked, ingredients: &[&'a Ingredient]) -> Self {
        // getUniqueAvailableIngredientItems
        let items: Vec<usize> = (0..stacked.keys.len())
            .filter(|&i| {
                stacked.amounts[i] > 0
                    && ingredients.iter().any(|g| g.0.contains(&*stacked.keys[i]))
            })
            .collect();
        let (ni, nt) = (ingredients.len(), items.len());
        let mut connection = vec![false; ni * nt];
        for (g, ing) in ingredients.iter().enumerate() {
            for (t, &key) in items.iter().enumerate() {
                if ing.0.contains(&*stacked.keys[key]) {
                    connection[t * ni + g] = true;
                }
            }
        }
        Self {
            stacked,
            ingredients: ingredients.to_vec(),
            items,
            connection,
            assigned: vec![false; ni * nt],
            satisfied: vec![false; ni],
            visited_ingredient: vec![false; ni],
            visited_item: vec![false; nt],
            path: Vec::new(),
        }
    }
    fn at(&self, item: usize, ingredient: usize) -> usize {
        item * self.ingredients.len() + ingredient
    }
    fn clear_visited(&mut self) {
        self.visited_ingredient.fill(false);
        self.visited_item.fill(false);
    }
    fn try_pick(&mut self, capacity: i32, mut output: Option<&mut Vec<Arc<str>>>) -> bool {
        if capacity <= 0 {
            return true;
        }
        let mut satisfied_count = 0;
        loop {
            let Some(path) = self.try_assigning_new_item(capacity) else {
                let valid = satisfied_count == self.ingredients.len();
                let has_output = valid && output.is_some();
                self.clear_visited();
                self.satisfied.fill(false);
                for g in 0..self.ingredients.len() {
                    for t in 0..self.items.len() {
                        let at = self.at(t, g);
                        if self.assigned[at] {
                            self.assigned[at] = false;
                            let key = self.items[t];
                            self.stacked.amounts[key] += capacity;
                            if has_output && let Some(out) = output.as_deref_mut() {
                                out.push(self.stacked.keys[key].clone());
                            }
                            break;
                        }
                    }
                }
                return valid;
            };
            let key = self.items[path[0]];
            self.stacked.amounts[key] -= capacity;
            self.satisfied[path[path.len() - 1]] = true;
            satisfied_count += 1;
            for i in 0..path.len() - 1 {
                if i & 1 == 0 {
                    let at = self.at(path[i], path[i + 1]);
                    self.assigned[at] = true;
                } else {
                    let at = self.at(path[i + 1], path[i]);
                    self.assigned[at] = false;
                }
            }
        }
    }
    fn try_assigning_new_item(&mut self, capacity: i32) -> Option<Vec<usize>> {
        self.clear_visited();
        for t in 0..self.items.len() {
            if self.stacked.amounts[self.items[t]] >= capacity
                && let Some(path) = self.find_path(t)
            {
                return Some(path);
            }
        }
        None
    }
    fn find_path(&mut self, start: usize) -> Option<Vec<usize>> {
        self.path.clear();
        self.visited_item[start] = true;
        self.path.push(start);
        while !self.path.is_empty() {
            let len = self.path.len();
            if (len - 1) & 1 == 0 {
                let item = self.path[len - 1];
                for g in 0..self.ingredients.len() {
                    let at = self.at(item, g);
                    if !self.visited_ingredient[g] && self.connection[at] && !self.assigned[at] {
                        self.visited_ingredient[g] = true;
                        self.path.push(g);
                        break;
                    }
                }
            } else {
                let ingredient = self.path[len - 1];
                if !self.satisfied[ingredient] {
                    return Some(self.path.clone());
                }
                for t in 0..self.items.len() {
                    if !self.visited_item[t] && self.assigned[self.at(t, ingredient)] {
                        self.visited_item[t] = true;
                        self.path.push(t);
                        break;
                    }
                }
            }
            if self.path.len() == len {
                self.path.pop();
            }
        }
        None
    }
    fn try_pick_all(&mut self, max_size: i32, output: Option<&mut Vec<Arc<str>>>) -> i32 {
        let mut min = 0i32;
        // Java int arithmetic: an empty ingredient list overflows as vanilla does.
        let mut max = max_size
            .min(self.stacked.upper_bound(&self.ingredients))
            .wrapping_add(1);
        loop {
            let mid = min.wrapping_add(max) / 2;
            if self.try_pick(mid, None) {
                if max.wrapping_sub(min) <= 1 {
                    if mid > 0 {
                        self.try_pick(mid, output);
                    }
                    return mid;
                }
                min = mid;
            } else {
                max = mid;
            }
        }
    }
}

/// `RecipeBookMenu.PostPlaceAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostPlace {
    Nothing,
    /// The client shows the recipe as a ghost (`ClientboundPlaceGhostRecipePacket`).
    Ghost,
}

/// The menu slots a placement works on.
struct Grid {
    width: usize,
    height: usize,
    /// `inputGridSlots` and `slotsToClear` as menu slot indices.
    inputs: Vec<usize>,
    clear: Vec<usize>,
}

impl Menu {
    fn place_grid(&self) -> Option<Grid> {
        match self.kind {
            Kind::Inventory => Some(Grid {
                width: 2,
                height: 2,
                inputs: (1..5).collect(),
                clear: (1..5).collect(),
            }),
            Kind::Crafting => Some(Grid {
                width: 3,
                height: 3,
                inputs: (1..10).collect(),
                clear: (1..10).collect(),
            }),
            Kind::Furnace(_) => Some(Grid {
                width: 1,
                height: 1,
                inputs: vec![0],
                clear: vec![0, 2],
            }),
            _ => None,
        }
    }

    /// `Inventory.fillStackedContents` then the menu's
    /// `fillCraftSlotsStackedContents`.
    pub fn stacked_contents(&self, env: &Env) -> Stacked {
        let mut stacked = Stacked::default();
        for stack in self.containers[PLAYER][..36].iter().flatten() {
            stacked.account_simple(stack, Self::max_stack(env, stack));
        }
        match self.kind {
            // A furnace's SimpleContainer counts all three slots, unfiltered.
            Kind::Furnace(_) => {
                for stack in self.containers[MAIN].iter().flatten() {
                    stacked.account_stack(stack, Self::max_stack(env, stack));
                }
            }
            _ => {
                for stack in self.containers[GRID].iter().flatten() {
                    stacked.account_simple(stack, Self::max_stack(env, stack));
                }
            }
        }
        stacked
    }

    /// `RecipeBookMenu.handlePlacement` for the recipe `id`
    /// (`ServerPlaceRecipe.placeRecipe`). `allow_drop` is the player being in
    /// creative; `order` the iteration order of the counted items, if known.
    pub fn place_recipe(
        &mut self,
        env: &Env,
        id: &str,
        use_max: bool,
        allow_drop: bool,
        order: Option<&[String]>,
    ) -> PostPlace {
        let Some(grid) = self.place_grid() else {
            return PostPlace::Nothing;
        };
        let Some(placement) = env.recipes.placement(id) else {
            return PostPlace::Nothing;
        };
        if placement.impossible() {
            return PostPlace::Nothing;
        }
        if !allow_drop && !self.test_clear_grid(env, &grid) {
            return PostPlace::Nothing;
        }
        let mut stacked = self.stacked_contents(env);
        if let Some(order) = order {
            stacked.set_order(order);
        }
        let ingredients: Vec<&Ingredient> = placement.ingredients.iter().collect();
        let post = if stacked.try_pick(&ingredients, 1, None) {
            self.place(env, id, &grid, &placement, &mut stacked, use_max);
            PostPlace::Nothing
        } else {
            self.clear_grid(env, &grid);
            PostPlace::Ghost
        };
        // AbstractCraftingMenu.finishPlacingRecipe: the result follows the grid.
        self.recompute(env);
        post
    }

    /// `recipeMatches`: the recipe already fills the grid.
    fn recipe_in_grid(&self, env: &Env, id: &str) -> bool {
        match self.kind {
            Kind::Furnace(_) => {
                let name = id.trim_start_matches("minecraft:");
                match (
                    env.recipes.cooking.iter().find(|c| c.id == name),
                    self.containers[MAIN][0].as_ref(),
                ) {
                    (Some(c), Some(input)) => c.ingredient.test(input),
                    _ => false,
                }
            }
            _ => env
                .recipes
                .recipe_matches(id, &self.containers[GRID], self.grid_size),
        }
    }

    fn place(
        &mut self,
        env: &Env,
        id: &str,
        grid: &Grid,
        placement: &Placement,
        stacked: &mut Stacked,
        use_max: bool,
    ) {
        let ingredients: Vec<&Ingredient> = placement.ingredients.iter().collect();
        let matches = self.recipe_in_grid(env, id);
        let biggest = stacked.try_pick_all(&ingredients, i32::MAX, None);
        if matches {
            for &s in &grid.inputs {
                if let Some(stack) = self.slot(s)
                    && (biggest.min(Self::max_stack(env, stack) as i32) as i64)
                        < i64::from(stack.count) + 1
                {
                    return;
                }
            }
        }
        // calculateAmountToCraft
        let amount = if use_max {
            biggest
        } else if matches {
            let smallest = grid
                .inputs
                .iter()
                .filter_map(|&s| self.slot(s))
                .map(|s| s.count as i32)
                .min();
            smallest.map_or(i32::MAX, |n| n + 1)
        } else {
            1
        };
        let mut used = Vec::new();
        if !stacked.try_pick(&ingredients, amount, Some(&mut used)) {
            return;
        }
        // clampToMaxStackSize
        let adjusted = used.iter().fold(amount, |n, item| {
            n.min(env.items.get(item).map_or(1, |i| i.max_stack) as i32)
        });
        if adjusted != amount {
            used.clear();
            if !stacked.try_pick(&ingredients, adjusted, Some(&mut used)) {
                return;
            }
        }
        self.clear_grid(env, grid);
        let (w, h) = placement.shaped.unwrap_or((grid.width, grid.height));
        let slots: Vec<i32> = placement.slots.clone();
        for (ingredient, index) in
            crate::recipe_book::place_cells(grid.width, grid.height, w, h, &slots)
        {
            if ingredient == -1 {
                continue;
            }
            let target = grid.inputs[index];
            let item = used[ingredient as usize].clone();
            let mut remaining = adjusted;
            while remaining > 0 {
                remaining = self.move_item_to_grid(target, &item, remaining);
                if remaining == -1 {
                    break;
                }
            }
        }
    }

    /// `moveItemToGrid`: the rest still to move, or -1 when nothing matches.
    fn move_item_to_grid(&mut self, target: usize, item: &str, count: i32) -> i32 {
        let existing = self.slot(target).cloned();
        let found = (0..36).find(|&i| {
            self.containers[PLAYER][i].as_ref().is_some_and(|s| {
                &*s.item == item
                    && usable_for_crafting(s)
                    && existing
                        .as_ref()
                        .is_none_or(|e| e.same_item_same_components(s))
            })
        });
        let Some(i) = found else {
            return -1;
        };
        let cell = &mut self.containers[PLAYER][i];
        let taken = match cell.as_mut() {
            Some(have) if (count as u32) < have.count => split(have, count as u32),
            _ => cell.take().expect("found"),
        };
        tidy(cell);
        let taken_count = taken.count as i32;
        let slot = self.slots[target];
        match self.containers[slot.container][slot.index].as_mut() {
            None => self.containers[slot.container][slot.index] = Some(taken),
            Some(have) => have.count += taken.count,
        }
        count - taken_count
    }

    /// `clearGrid`: the slots to clear go back to the inventory (or drop),
    /// then the grid empties (`clearCraftingContent`).
    fn clear_grid(&mut self, env: &Env, grid: &Grid) {
        for &s in &grid.clear {
            let slot = self.slots[s];
            if let Some(stack) = self.containers[slot.container][slot.index].clone() {
                let mut copy = stack;
                self.place_back(env, &mut copy);
            }
        }
        match self.kind {
            Kind::Furnace(_) => {
                for &s in &grid.clear {
                    let slot = self.slots[s];
                    self.containers[slot.container][slot.index] = None;
                }
            }
            _ => {
                self.containers[GRID].fill(None);
                self.containers[RESULT][0] = None;
            }
        }
    }

    /// `Inventory.placeItemBackInInventory`.
    pub(super) fn place_back(&mut self, env: &Env, stack: &mut ItemStack) {
        while stack.count > 0 {
            let slot = self
                .slot_with_space(env, stack)
                .or_else(|| (0..36).find(|&i| self.containers[PLAYER][i].is_none()));
            let Some(slot) = slot else {
                let dropped = std::mem::replace(
                    stack,
                    ItemStack {
                        count: 0,
                        ..stack.clone()
                    },
                );
                self.drop_stack(&dropped);
                break;
            };
            let have = self.containers[PLAYER][slot]
                .as_ref()
                .map_or(0, |s| s.count);
            let space = Self::max_stack(env, stack).saturating_sub(have);
            let mut part = split(stack, space);
            self.inventory_add_at(env, slot, &mut part);
        }
    }

    /// `Inventory.add(slot, stack)`.
    fn inventory_add_at(&mut self, env: &Env, slot: usize, stack: &mut ItemStack) {
        if stack.count == 0 {
            return;
        }
        if Self::damageable(env, stack) && stack.damage > 0 {
            self.containers[PLAYER][slot] = Some(stack.clone());
            stack.count = 0;
            return;
        }
        loop {
            let last = stack.count;
            // addResource(slot, stack)
            let cell = self.containers[PLAYER][slot].get_or_insert_with(|| ItemStack {
                count: 0,
                ..stack.clone()
            });
            let room = super::CONTAINER_MAX
                .min(Self::max_stack(env, cell))
                .saturating_sub(cell.count);
            let add = stack.count.min(room);
            cell.count += add;
            stack.count -= add;
            tidy(&mut self.containers[PLAYER][slot]);
            if stack.count == 0 || stack.count >= last {
                if stack.count == last && self.creative {
                    stack.count = 0;
                }
                return;
            }
        }
    }

    /// `testClearGrid`: whether the inventory can take the grid back.
    fn test_clear_grid(&self, env: &Env, grid: &Grid) -> bool {
        let mut free_slots: Vec<ItemStack> = Vec::new();
        let free_in_inventory = self.containers[PLAYER][..36]
            .iter()
            .filter(|s| s.is_none())
            .count();
        for &s in &grid.inputs {
            let Some(mut stack) = self.slot(s).cloned() else {
                continue;
            };
            let with_space = self.slot_with_space(env, &stack);
            if with_space.is_none() && free_slots.len() <= free_in_inventory {
                for listed in &mut free_slots {
                    let max = Self::max_stack(env, listed);
                    if listed.item == stack.item
                        && listed.count != max
                        && listed.count + stack.count <= max
                    {
                        listed.count += stack.count;
                        stack.count = 0;
                        break;
                    }
                }
                if stack.count > 0 {
                    if free_slots.len() >= free_in_inventory {
                        return false;
                    }
                    free_slots.push(stack);
                }
            } else if with_space.is_none() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ing(items: &[&str]) -> Ingredient {
        Ingredient::of(items.iter().map(|s| (*s).to_owned()).collect())
    }
    fn have(items: &[(&str, i32)]) -> Stacked {
        let mut s = Stacked::default();
        for (item, n) in items {
            s.account(&Arc::from(*item), *n);
        }
        s
    }
    #[test]
    fn a_single_item_can_fill_several_ingredients() {
        let planks = ing(&["minecraft:oak_planks", "minecraft:birch_planks"]);
        let reqs = [&planks, &planks, &planks, &planks];
        let mut a = have(&[("minecraft:oak_planks", 2), ("minecraft:birch_planks", 2)]);
        assert!(a.can_craft(&reqs));
        // Two sets need 8 planks, but only 4 are there.
        assert!(!a.try_pick(&reqs, 2, None));
        assert_eq!(a.try_pick_all(&reqs, i32::MAX, None), 1);
        assert_eq!(
            have(&[("minecraft:oak_planks", 64)]).try_pick_all(&reqs, i32::MAX, None),
            16
        );
        // Picking leaves the counts as they were.
        assert!(a.try_pick(&reqs, 1, None));
        assert!(a.can_craft(&reqs));
    }
    #[test]
    fn the_matching_reassigns_flexible_ingredients() {
        // The first ingredient takes either item, the second only the stick:
        // the first item tried (the stick) moves over to free it.
        let any = ing(&["minecraft:stick", "minecraft:bone"]);
        let stick = ing(&["minecraft:stick"]);
        let mut a = have(&[("minecraft:stick", 1), ("minecraft:bone", 1)]);
        let mut used = Vec::new();
        assert!(a.try_pick(&[&any, &stick], 1, Some(&mut used)));
        assert_eq!(&*used[0], "minecraft:bone");
        assert_eq!(&*used[1], "minecraft:stick");
        assert!(!have(&[("minecraft:stick", 1)]).can_craft(&[&any, &stick]));
    }
    #[test]
    fn the_order_decides_between_fitting_items() {
        let planks = ing(&["minecraft:oak_planks", "minecraft:birch_planks"]);
        let mut a = have(&[("minecraft:oak_planks", 4), ("minecraft:birch_planks", 4)]);
        let mut used = Vec::new();
        a.try_pick(&[&planks], 1, Some(&mut used));
        assert_eq!(&*used[0], "minecraft:oak_planks");
        a.set_order(&["minecraft:birch_planks".to_owned()]);
        used.clear();
        a.try_pick(&[&planks], 1, Some(&mut used));
        assert_eq!(&*used[0], "minecraft:birch_planks");
    }
    #[test]
    fn damaged_or_named_stacks_are_not_counted() {
        let mut named = ItemStack::new("minecraft:stick", 4);
        named.data_mut().custom_name = Some(horizon_world::nbt::Tag::String("x".into()));
        let mut damaged = ItemStack::new("minecraft:iron_sword", 1);
        damaged.damage = 3;
        let mut s = Stacked::default();
        for stack in [&named, &damaged, &ItemStack::new("minecraft:stick", 2)] {
            s.account_simple(stack, 64);
        }
        assert_eq!(s.keys().len(), 1);
        assert!(s.can_craft(&[&ing(&["minecraft:stick"]), &ing(&["minecraft:stick"])]));
        assert!(!s.can_craft(&[&ing(&["minecraft:stick"]); 3]));
    }
}
