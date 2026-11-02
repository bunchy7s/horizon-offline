//! `AbstractFurnaceBlockEntity`'s cooking over the furnace data the chunk
//! holds (`containers::Furnace`): it needs the recipes, so it lives above them.
use crate::{
    containers::{Furnace, recipe_used_key},
    crafting::Recipes,
    items::{ItemStack, Items},
    providers::Providers,
};

/// The furnace methods that read the recipes.
pub trait FurnaceCooking {
    /// `AbstractFurnaceBlockEntity.setItem` on the input slot: anything but
    /// the same item with the same components restarts cooking, with the
    /// total time of the new input's recipe (200 ticks without one).
    fn set_input(&mut self, recipes: &Recipes, stack: Option<ItemStack>);
    /// `recipesUsed`, each recipe with its `experience` (0 for one this data pack does
    /// not have), in the order `getRecipesToAwardAndPopExperience` pays them.
    fn recipe_experience(&self, recipes: &Recipes) -> Vec<(u32, f32)>;
    /// A menu click wrote its slots into `items`, and `before` was the input
    /// slot until then: the menu called `setItem` on it only if it changed
    /// (`Slot.setByPlayer`; partial takes and merges keep the stack).
    fn input_clicked(&mut self, recipes: &Recipes, before: Option<ItemStack>);
    /// One server tick. Returns `Some(lit)` when the LIT block state must change.
    /// `drops` collects what the furnace lets fall at its position
    /// (`Containers.dropItemStack`): the remainder of a fuel when more of it is left.
    fn tick(
        &mut self,
        recipes: &Recipes,
        items: &Items,
        providers: &Providers,
        drops: &mut Vec<ItemStack>,
    ) -> Option<bool>;
}
impl FurnaceCooking for Furnace {
    fn set_input(&mut self, recipes: &Recipes, stack: Option<ItemStack>) {
        let same = match (&self.items[0], &stack) {
            (Some(old), Some(new)) => old.same_item_same_components(new),
            _ => false,
        };
        self.items[0] = stack;
        if !same {
            self.cooking_total = self.items[0]
                .as_ref()
                .and_then(|input| recipes.cook(self.kind.recipe_type(), input))
                .map_or(200, |recipe| total_time(recipe.time, self.speed));
            self.cooking_timer = 0;
        }
    }
    fn recipe_experience(&self, recipes: &Recipes) -> Vec<(u32, f32)> {
        self.recipes_used
            .iter()
            .map(|(id, count)| (*count, recipes.cooking_experience(id)))
            .collect()
    }
    fn input_clicked(&mut self, recipes: &Recipes, before: Option<ItemStack>) {
        if self.items[0] != before {
            let after = std::mem::replace(&mut self.items[0], before);
            self.set_input(recipes, after);
        }
    }
    fn tick(
        &mut self,
        recipes: &Recipes,
        items: &Items,
        providers: &Providers,
        drops: &mut Vec<ItemStack>,
    ) -> Option<bool> {
        let was_lit = self.lit_remaining > 0;
        let mut lit = false;
        if self.lit_remaining > 0 {
            self.lit_remaining -= 1;
            lit = self.lit_remaining > 0;
        }
        let fast = self.kind.fast();
        let has_ingredient = self.items[0].is_some();
        let has_fuel = self.items[1].is_some();
        if lit || has_fuel && has_ingredient {
            if let Some(ingredient) = self.items[0].clone() {
                if let Some(recipe) = recipes.cook(self.kind.recipe_type(), &ingredient) {
                    if self.cooking_timer == 0 {
                        self.cooking_total = total_time(recipe.time, self.speed);
                    }
                    let max = items.get(&recipe.result.item).map_or(64, |i| i.max_stack);
                    let can_burn = match &self.items[2] {
                        None => true,
                        Some(out) => {
                            out.item == recipe.result.item && out.count + recipe.result.count <= max
                        }
                    };
                    if can_burn {
                        if !lit {
                            let fuel = self.items[1].clone();
                            let time = fuel
                                .as_ref()
                                .map_or(0, |f| providers.burn_time(items, f, fast));
                            let speed = fuel
                                .as_ref()
                                .map_or(1.0, |f| providers.speed(items, f, fast));
                            self.lit_remaining = time;
                            self.lit_total = time;
                            self.speed = speed;
                            if self.cooking_total > 0 && self.cooking_timer < self.cooking_total {
                                let ratio = self.cooking_timer as f32 / self.cooking_total as f32;
                                self.cooking_total = total_time(recipe.time, self.speed);
                                self.cooking_timer =
                                    (ratio * self.cooking_total as f32).ceil() as i32;
                            }
                            if time > 0 {
                                // AbstractFurnaceBlockEntity.consumeFuel: the fuel's crafting
                                // remainder takes the slot once the stack is used up, else
                                // it is dropped.
                                if let Some(f) = self.items[1].as_mut() {
                                    let remainder = items
                                        .get(&f.item)
                                        .and_then(|i| i.crafting_remainder.as_deref())
                                        .map(|r| ItemStack::new(r, 1));
                                    f.count -= 1;
                                    if f.count == 0 {
                                        self.items[1] = remainder;
                                    } else if let Some(remainder) = remainder {
                                        drops.push(remainder);
                                    }
                                }
                                lit = true;
                            }
                        }
                        if lit {
                            self.cooking_timer += 1;
                            if self.cooking_timer >= self.cooking_total {
                                self.cooking_timer = 0;
                                self.cooking_total = total_time(recipe.time, self.speed);
                                match &mut self.items[2] {
                                    Some(out) => out.count += recipe.result.count,
                                    None => self.items[2] = Some(recipe.result.clone()),
                                }
                                *self
                                    .recipes_used
                                    .entry(recipe_used_key(&recipe.id))
                                    .or_insert(0) += 1;
                                // AbstractFurnaceBlockEntity.burn: drying a wet sponge fills
                                // the bucket in the fuel slot.
                                if self.items[0]
                                    .as_ref()
                                    .is_some_and(|i| &*i.item == "minecraft:wet_sponge")
                                    && self.items[1]
                                        .as_ref()
                                        .is_some_and(|f| &*f.item == "minecraft:bucket")
                                {
                                    self.items[1] =
                                        Some(ItemStack::new("minecraft:water_bucket", 1));
                                }
                                if let Some(i) = self.items[0].as_mut() {
                                    i.count -= 1;
                                    if i.count == 0 {
                                        self.items[0] = None;
                                    }
                                }
                            }
                        } else {
                            self.cooking_timer = 0;
                        }
                    } else {
                        self.cooking_timer = 0;
                    }
                }
            } else {
                self.cooking_timer = 0;
            }
        } else if self.cooking_timer > 0 {
            self.cooking_timer = (self.cooking_timer - 2).clamp(0, self.cooking_total);
        }
        (was_lit != lit).then_some(lit)
    }
}
fn total_time(time: u32, speed: f32) -> i32 {
    if speed > 0.0 {
        (time as f32 / speed).ceil() as i32
    } else {
        time as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{containers::FurnaceKind, items::Items};
    use horizon_world::loot::PackOverlay;
    use std::{collections::HashMap, fs};

    /// A furnace with `fuel` x `count` burning `test:ore` into `test:ingot`, over a
    /// data root holding that one recipe and an item table with two fuels: `test:coal`
    /// (no remainder) and `test:cell` (a content mod's item that leaves `test:husk`).
    fn burn(fuel: &str, count: u32) -> (Furnace, Vec<ItemStack>) {
        let root = std::env::temp_dir().join(format!(
            "horizon-furnace-remainder-{}-{}-{count}",
            std::process::id(),
            fuel.replace(':', "-")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        fs::write(
            root.join("data/minecraft/recipe/test_ingot.json"),
            br#"{"type":"minecraft:smelting","ingredient":"test:ore","result":{"id":"test:ingot"}}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        let fuel_component =
            r#"{"minecraft:cooking_fuel": {"burn_time": 200, "speed_multiplier": 1.0}}"#;
        fs::write(
            root.join("items.json"),
            format!(
                r#"{{"test:ore": {{}}, "test:ingot": {{}}, "test:coal": {fuel_component}, "test:cell": {fuel_component}, "test:husk": {{}}}}"#
            ),
        )
        .unwrap();
        fs::write(
            root.join(horizon_items::items::MOD_CRAFT_REMAINDERS),
            r#"{"test:cell": "test:husk"}"#,
        )
        .unwrap();
        let items = Items::catalog(&root, &PackOverlay::NONE).unwrap();
        let recipes = Recipes::load(&root, &items, &PackOverlay::NONE).unwrap();
        let providers = Providers::load(&root);
        let mut furnace = Furnace {
            kind: FurnaceKind::Furnace,
            items: [
                Some(ItemStack::new("test:ore", 1)),
                Some(ItemStack::new(fuel, count)),
                None,
            ],
            lit_remaining: 0,
            lit_total: 0,
            cooking_timer: 0,
            cooking_total: 0,
            speed: 1.0,
            recipes_used: HashMap::new(),
        };
        let mut drops = Vec::new();
        let lit = furnace.tick(&recipes, &items, &providers, &mut drops);
        let _ = fs::remove_dir_all(&root);
        assert_eq!(lit, Some(true), "the fuel lit the furnace");
        (furnace, drops)
    }

    fn fuel_slot(furnace: &Furnace) -> Option<(&str, u32)> {
        furnace.items[1].as_ref().map(|s| (&*s.item, s.count))
    }

    /// `AbstractFurnaceBlockEntity.consumeFuel`: the last of a fuel with a remainder
    /// leaves it in the fuel slot.
    #[test]
    fn the_last_fuel_leaves_its_remainder_in_the_slot() {
        let (furnace, drops) = burn("test:cell", 1);
        assert_eq!(fuel_slot(&furnace), Some(("test:husk", 1)));
        assert!(drops.is_empty());
    }

    /// With more of the fuel left, the remainder is dropped at the furnace.
    #[test]
    fn a_fuel_still_in_the_stack_drops_its_remainder() {
        let (furnace, drops) = burn("test:cell", 3);
        assert_eq!(fuel_slot(&furnace), Some(("test:cell", 2)));
        assert_eq!(drops, vec![ItemStack::new("test:husk", 1)]);
    }

    /// A fuel without a remainder leaves nothing.
    #[test]
    fn a_fuel_without_a_remainder_leaves_nothing() {
        let (furnace, drops) = burn("test:coal", 1);
        assert_eq!(fuel_slot(&furnace), None);
        assert!(drops.is_empty());
        let (furnace, drops) = burn("test:coal", 2);
        assert_eq!(fuel_slot(&furnace), Some(("test:coal", 1)));
        assert!(drops.is_empty());
    }
}
