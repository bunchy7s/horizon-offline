//! `AbstractFurnaceBlockEntity`'s cooking over the furnace data the chunk
//! holds (`containers::Furnace`): it needs the recipes, so it lives above them.
use crate::{
    containers::{Furnace, recipe_used_key},
    crafting::Recipes,
    items::{ItemStack, Items},
    providers::Providers,
};
use std::sync::Arc;

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
    fn tick(&mut self, recipes: &Recipes, items: &Items, providers: &Providers) -> Option<bool>;
}
fn remainder(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:lava_bucket" | "minecraft:water_bucket" | "minecraft:milk_bucket" => {
            Some("minecraft:bucket")
        }
        _ => None,
    }
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
    fn tick(&mut self, recipes: &Recipes, items: &Items, providers: &Providers) -> Option<bool> {
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
                                if let Some(f) = self.items[1].as_mut() {
                                    let item = f.item.clone();
                                    f.count -= 1;
                                    if f.count == 0 {
                                        self.items[1] = remainder(&item).map(|r| ItemStack {
                                            item: Arc::from(r),
                                            count: 1,
                                            damage: 0,
                                            extra: None,
                                        });
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
