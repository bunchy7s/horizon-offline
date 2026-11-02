//! Every item and recipe the game knows, made once for the mods that read
//! them (native mods' `native_recipes`, capability `recipes` of WebAssembly
//! mods, docs/mods.md), and the recipe book's known displays; placing a
//! recipe and creative gifts. The recipes are the window's own data — the
//! content root's — with the server's recipes of the types its native mods
//! had it send (`horizon:recipe_content`) in their place, while the known
//! displays follow the server's book as the recipe book does.
use super::{
    Game,
    native_hooks::{
        NativeItem as RecipeItem, NativeRecipe as Recipe, NativeRecipeKind as RecipeKind,
        NativeRecipeStack as RecipeStack, NativeRecipes as Recipes,
    },
};
use horizon_client::{
    crafting::{Ingredient, recipe_id},
    items::ItemStack,
    recipe_book::{Category, Display},
    stations::Smithing,
};
use std::{collections::HashMap, sync::Arc};

/// The tables, made once, and the recipe book's known displays with the
/// book's version they are of.
/// The most recipes a server's recipe content may carry.
const MAX_SYNCED_RECIPES: usize = 1_000_000;

#[derive(Default)]
pub(super) struct RecipeTables {
    tables: Option<Arc<Recipes>>,
    known: (Option<u64>, Arc<[i32]>),
    /// The server's recipes of the types its native mods sent (`horizon:
    /// recipe_content`) over the window's own: what `Game::recipes` reads on
    /// a client once they came.
    synced: Option<horizon_client::crafting::Recipes>,
    /// The recipe content's parts so far, until its last.
    incoming: Vec<(String, serde_json::Value)>,
    /// The types that came before the player did: the mods hear of them
    /// once the helpers can answer (`recipes_received_now`).
    received: Option<Vec<String>>,
}

/// Item id -> its index in [`Recipes::items`].
struct Index(HashMap<Arc<str>, u32>);
impl Index {
    fn stack(&self, item: &str, count: u32, components: String) -> Option<RecipeStack> {
        Some(RecipeStack {
            item: *self.0.get(item)?,
            count,
            components,
        })
    }
    fn of(&self, s: &ItemStack) -> Option<RecipeStack> {
        self.stack(&s.item, s.count, components(s))
    }
    fn any(&self, i: &Ingredient) -> Vec<RecipeStack> {
        i.order
            .iter()
            .filter_map(|id| self.stack(id, 1, String::new()))
            .collect()
    }
}

/// A stack's components beyond its item's, as vanilla's item syntax writes
/// them (`[minecraft:potion_contents={potion:"minecraft:swiftness"}]`), or "".
pub(super) fn components(s: &ItemStack) -> String {
    use horizon_world::nbt::Tag;
    let Some(Tag::Compound(entries)) = horizon_client::item_data::encode_components(s) else {
        return String::new();
    };
    let parts: Vec<String> = entries
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                k.to_string_lossy(),
                horizon_client::snbt::to_snbt(v)
            )
        })
        .collect();
    format!("[{}]", parts.join(","))
}

fn recipe(id: String, kind: RecipeKind) -> Recipe {
    Recipe {
        id,
        kind,
        category: String::new(),
        group: String::new(),
        width: 0,
        height: 0,
        slots: Vec::new(),
        results: Vec::new(),
        cooking_time: 0,
        experience: 0.0,
        book_id: -1,
    }
}

impl Game<'_> {
    /// `native_give`: `count` of `item`, refused outside a world, unless
    /// creative and for an item the game doesn't have.
    pub(super) fn recipe_give(&mut self, item: &str, count: u32) -> Result<(), String> {
        if self.player.is_none() || self.res.rest.get().is_none() {
            return Err("not in a world".into());
        }
        if !self.creative {
            return Err("only in creative mode".into());
        }
        if self.res.items().get(item).is_none() {
            return Err(format!("no item {item}"));
        }
        self.mod_give(item, count.clamp(1, 99), "");
        Ok(())
    }
    /// `native_place_recipe`: refused as `recipes.place` refuses it.
    pub(super) fn recipe_place(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        if !self.mod_recipe_screen() {
            return Err("no screen that takes recipes is open".into());
        }
        if !self.known_displays().contains(&book_id) {
            return Err(format!(
                "recipe {book_id} is not in the player's recipe book"
            ));
        }
        self.mod_place_recipe(book_id, all);
        Ok(())
    }
    /// `recipes.give`: a creative player takes the stack as from the
    /// creative inventory; a server hears it as `SetCreativeModeSlot`
    /// through the inventory's sync, and refuses it unless creative.
    pub(super) fn mod_give(&mut self, item: &str, count: u32, components: &str) {
        let Some(max) = self.items().get(item).map(|i| i.max_stack) else {
            return;
        };
        if !self.creative {
            return;
        }
        let (damage, extra) = horizon_client::item_data::command_components(components)
            .map_or((0, None), |tag| {
                horizon_client::item_data::decode_components(Some(&tag))
            });
        let _ = self.give(ItemStack {
            item: item.into(),
            // One stack at most, as the creative inventory hands out.
            count: count.min(max),
            damage,
            extra,
        });
    }
    /// The tables, made on the first call; none while the world resources
    /// they come from are still loading behind the title on a slow start.
    pub(super) fn made_recipes(&mut self) -> Option<Arc<Recipes>> {
        self.res.rest.get()?;
        if self.recipe_tables.tables.is_none() {
            self.recipe_tables.tables = Some(Arc::new(self.make_mod_recipes()));
        }
        self.recipe_tables.tables.clone()
    }
    /// The `book_id`s the player's recipe book holds now.
    pub(super) fn known_displays(&mut self) -> Arc<[i32]> {
        if self.res.rest.get().is_none() {
            return Arc::from([]);
        }
        let (version, known) = self.recipe_book_displays();
        if self.recipe_tables.known.0 != Some(version) {
            self.recipe_tables.known = (Some(version), known.into());
        }
        self.recipe_tables.known.1.clone()
    }

    fn make_mod_recipes(&mut self) -> Recipes {
        let names = &self.pack.names;
        let items = self.items();
        let mut tags: HashMap<&str, Vec<String>> = HashMap::new();
        for (tag, members) in &items.item_tags {
            for m in members {
                tags.entry(m).or_default().push(tag.clone());
            }
        }
        let table: Vec<RecipeItem> = items
            .registry_order
            .iter()
            .map(|id| RecipeItem {
                id: id.to_string(),
                name: names.display_item(id),
                tags: tags.remove(&**id).map_or_else(Vec::new, |mut t| {
                    t.sort();
                    t
                }),
            })
            .collect();
        let index = Index(
            items
                .registry_order
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), i as u32))
                .collect(),
        );
        let mut out = Vec::new();
        let cooking: HashMap<String, (u32, f32)> = self
            .recipes()
            .cooking
            .iter()
            .map(|c| (recipe_id(&c.id), (c.time, c.experience)))
            .collect();
        for e in self.recipe_book_entries() {
            let furnace = |c: Category| match c.name().split('_').next() {
                Some("blast") => RecipeKind::Blasting,
                Some("smoker") => RecipeKind::Smoking,
                _ => RecipeKind::Smelting,
            };
            let kind = match &e.display {
                Display::Shaped { .. } => RecipeKind::CraftingShaped,
                Display::Shapeless(_) => RecipeKind::CraftingShapeless,
                Display::Furnace(_) => furnace(e.category),
            };
            let mut r = recipe(e.id.clone(), kind);
            r.category = e.category.name().to_owned();
            r.group = e.group.clone().unwrap_or_default();
            r.book_id = e.display_id;
            r.slots = match &e.display {
                Display::Shaped {
                    width,
                    height,
                    cells,
                } => {
                    (r.width, r.height) = (*width as u32, *height as u32);
                    cells
                        .iter()
                        .map(|c| c.as_ref().map_or_else(Vec::new, |i| index.any(i)))
                        .collect()
                }
                Display::Shapeless(list) => list.iter().map(|i| index.any(i)).collect(),
                Display::Furnace(i) => vec![index.any(i)],
            };
            // Requirements shown as other stacks (an arrow's potions): the
            // requirement's index counts the filled slots only.
            let filled: Vec<usize> = (0..r.slots.len())
                .filter(
                    |&k| !matches!(&e.display, Display::Shaped { cells, .. } if cells[k].is_none()),
                )
                .collect();
            for (req, stacks) in &e.stacks {
                if let Some(&k) = filled.get(*req) {
                    r.slots[k] = stacks.iter().filter_map(|s| index.of(s)).collect();
                }
            }
            r.results = e.results().iter().filter_map(|s| index.of(s)).collect();
            if let Some(&(time, xp)) = cooking.get(&e.id) {
                (r.cooking_time, r.experience) = (time, xp);
            }
            out.push(r);
        }
        for c in &self.recipes().cooking {
            if c.kind == "minecraft:campfire_cooking" {
                let mut r = recipe(recipe_id(&c.id), RecipeKind::Campfire);
                r.group = c.group.clone();
                r.slots = vec![index.any(&c.ingredient)];
                r.results = index.of(&c.result).into_iter().collect();
                (r.cooking_time, r.experience) = (c.time, c.experience);
                out.push(r);
            }
        }
        let stations = self.stations();
        for s in &stations.recipes.stonecutting {
            let mut r = recipe(s.id.clone(), RecipeKind::Stonecutting);
            r.slots = vec![index.any(&s.input)];
            r.results = index.of(&s.result).into_iter().collect();
            out.push(r);
        }
        for s in &stations.recipes.smithing {
            let (id, template, base, addition, results) = match s {
                Smithing::Transform {
                    id,
                    template,
                    base,
                    addition,
                    result,
                } => (
                    id,
                    template,
                    base,
                    addition,
                    index.stack(result, 1, String::new()).into_iter().collect(),
                ),
                Smithing::Trim {
                    id,
                    template,
                    base,
                    addition,
                    ..
                } => (id, template, base, addition, index.any(base)),
            };
            let mut r = recipe(id.clone(), RecipeKind::Smithing);
            r.slots = vec![index.any(template), index.any(base), index.any(addition)];
            r.results = results;
            out.push(r);
        }
        for b in &stations.brewing.recipes {
            let potion = |p: &horizon_client::brewing::PotionIngredient| -> Vec<RecipeStack> {
                let mut item_ids: Vec<&String> = p.items.iter().collect();
                item_ids.sort();
                let mut potions: Vec<&String> = p.potions.iter().flatten().collect();
                potions.sort();
                item_ids
                    .into_iter()
                    .flat_map(|item| {
                        let one = |c: String| index.stack(item, 1, c);
                        if potions.is_empty() {
                            one(String::new()).into_iter().collect::<Vec<_>>()
                        } else {
                            potions
                                .iter()
                                .filter_map(|p| {
                                    one(format!("[minecraft:potion_contents={{potion:\"{p}\"}}]"))
                                })
                                .collect()
                        }
                    })
                    .collect()
            };
            let mut r = recipe(b.id.clone(), RecipeKind::Brewing);
            r.slots = vec![potion(&b.input), potion(&b.reagent)];
            r.results = index.of(&b.output).into_iter().collect();
            out.push(r);
        }
        Recipes {
            items: table,
            recipes: out,
        }
    }
}

impl Game<'_> {
    /// The server's recipes a client was sent, over its own; None on a
    /// server and until a server sends any.
    pub(super) fn synced_recipes(&self) -> Option<&horizon_client::crafting::Recipes> {
        self.recipe_tables.synced.as_ref()
    }
    /// A new play session (`Login`): what the last server sent is gone.
    pub(super) fn forget_synced_recipes(&mut self) {
        self.recipe_tables.synced = None;
        self.recipe_tables.incoming.clear();
        self.recipe_tables.received = None;
        self.recipe_tables.tables = None;
        self.forget_book();
    }
    /// `native_recipes_received` for the types that came, once the window
    /// has its player: the recipes of a join come before it, and the helpers
    /// answer nothing until then. Once a frame.
    pub(super) fn recipes_received_now(&mut self) {
        if self.player.is_some()
            && let Some(types) = self.recipe_tables.received.take()
        {
            self.native_recipes_received(&types);
        }
    }
    /// `OnDatapackSyncEvent.sendRecipes`: the recipe types the native mods
    /// want their clients to have, from this load's tables, become what the
    /// server sends its Horizon clients at a join and after `/reload`. Called
    /// once a load's tables are built, before any player hears of them.
    pub(super) fn sync_recipe_content(&mut self) {
        use horizon_client::lan::recipes;
        let mut types = Vec::new();
        self.native_datapack_sync(&mut types);
        types.sort();
        types.dedup();
        let sources = if types.is_empty() {
            Vec::new()
        } else {
            horizon_client::crafting::Recipes::sources(&self.options.resources, &self.pack_files())
                .unwrap_or_else(|e| {
                    log::warn!("Horizon: the recipes for the clients were not read: {e}");
                    Vec::new()
                })
        };
        recipes::set_content(recipes::encode_content(
            &types,
            &sources,
            recipes::sent_before(),
        ));
    }
    /// A `horizon:recipe_content` part the server sent the window. On the
    /// last, the window's tables take the server's recipes of those types in
    /// place of its own, and the mods hear of it (`RecipesReceivedEvent`).
    pub(super) fn remote_recipe_content(&mut self, data: &[u8]) {
        let part: horizon_client::lan::recipes::ContentPart = match serde_json::from_slice(data) {
            Ok(part) => part,
            Err(e) => {
                log::warn!("Horizon: bad recipe content from the server: {e}");
                return;
            }
        };
        self.recipe_tables.incoming.extend(part.recipes);
        // The server is trusted to end its parts; one that never does is
        // dropped, not kept growing.
        if self.recipe_tables.incoming.len() > MAX_SYNCED_RECIPES {
            log::warn!("Horizon: the server sent more than {MAX_SYNCED_RECIPES} recipes; dropped");
            self.recipe_tables.incoming = Vec::new();
            return;
        }
        let Some(types) = part.last else {
            return;
        };
        let received = std::mem::take(&mut self.recipe_tables.incoming);
        // No type any more: back on the window's own recipes.
        if types.is_empty() {
            if self.recipe_tables.synced.take().is_some() {
                self.recipe_tables.tables = None;
                self.forget_book();
                self.recipe_tables.received = Some(types);
                self.recipes_received_now();
            }
            return;
        }
        let root = &self.options.resources;
        let overlay = horizon_world::loot::PackOverlay::NONE;
        let built = horizon_client::crafting::Recipes::sources(root, &overlay).and_then(|own| {
            let of = |v: &serde_json::Value| types.iter().any(|t| v["type"].as_str() == Some(t));
            let mut all: Vec<_> = own.into_iter().filter(|(_, v)| !of(v)).collect();
            all.extend(received);
            horizon_client::crafting::Recipes::from_sources(root, self.items(), &overlay, all)
        });
        match built {
            Ok(recipes) => {
                self.recipe_tables.synced = Some(recipes);
                // What was made of the window's own recipes is made again.
                self.recipe_tables.tables = None;
                self.forget_book();
                self.recipe_tables.received = Some(types);
                self.recipes_received_now();
            }
            Err(e) => log::warn!("Horizon: the server's recipes were not taken: {e}"),
        }
    }
}
