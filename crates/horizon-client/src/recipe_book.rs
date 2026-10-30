//! The recipe book: what the book shows for each menu (`RecipeBookType`
//! tabs, `RecipeBookCategories`, groups), which recipes the player can craft
//! with the items at hand (`StackedItemContents.canCraft` and the biggest
//! craftable stack), where a recipe goes in a grid (`PlaceRecipeHelper`) and
//! the book's saved settings.
use crate::{
    crafting::{Ingredient, Placement, Recipe, Recipes},
    items::{ItemStack, Items},
};
use horizon_world::loot::PackOverlay;
use horizon_world::nbt::Tag;
use std::collections::{BTreeSet, HashMap};

/// `RecipeBookType`: the menus with a recipe book and their own settings.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BookType {
    Crafting,
    Furnace,
    BlastFurnace,
    Smoker,
}
impl BookType {
    pub const ALL: [Self; 4] = [
        Self::Crafting,
        Self::Furnace,
        Self::BlastFurnace,
        Self::Smoker,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
    /// `RecipeBookSettings` keys in the player's `recipeBook` compound.
    fn keys(self) -> (&'static str, &'static str) {
        match self {
            Self::Crafting => ("isGuiOpen", "isFilteringCraftable"),
            Self::Furnace => ("isFurnaceGuiOpen", "isFurnaceFilteringCraftable"),
            Self::BlastFurnace => (
                "isBlastingFurnaceGuiOpen",
                "isBlastingFurnaceFilteringCraftable",
            ),
            Self::Smoker => ("isSmokerGuiOpen", "isSmokerFilteringCraftable"),
        }
    }
    /// The tab buttons of the book's screen, search first.
    pub fn tabs(self) -> Vec<Tab> {
        use Category::*;
        let categories: &[Category] = match self {
            Self::Crafting => &[
                CraftingEquipment,
                CraftingBuilding,
                CraftingMisc,
                CraftingRedstone,
            ],
            Self::Furnace => &[FurnaceFood, FurnaceBlocks, FurnaceMisc],
            Self::BlastFurnace => &[BlastBlocks, BlastMisc],
            Self::Smoker => &[SmokerFood],
        };
        std::iter::once(Tab::Search)
            .chain(categories.iter().map(|c| Tab::Category(*c)))
            .collect()
    }
    /// The categories the search tab lists, in `SearchRecipeBookCategory` order.
    fn search_categories(self) -> &'static [Category] {
        use Category::*;
        match self {
            Self::Crafting => &[
                CraftingEquipment,
                CraftingBuilding,
                CraftingMisc,
                CraftingRedstone,
            ],
            Self::Furnace => &[FurnaceFood, FurnaceBlocks, FurnaceMisc],
            Self::BlastFurnace => &[BlastBlocks, BlastMisc],
            Self::Smoker => &[SmokerFood],
        }
    }
}

/// `RecipeBookCategories` of the four menus' recipes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Category {
    CraftingBuilding,
    CraftingRedstone,
    CraftingEquipment,
    CraftingMisc,
    FurnaceFood,
    FurnaceBlocks,
    FurnaceMisc,
    BlastBlocks,
    BlastMisc,
    SmokerFood,
}
impl Category {
    /// The `RecipeBookCategories` registry path.
    pub fn name(self) -> &'static str {
        match self {
            Self::CraftingBuilding => "crafting_building_blocks",
            Self::CraftingRedstone => "crafting_redstone",
            Self::CraftingEquipment => "crafting_equipment",
            Self::CraftingMisc => "crafting_misc",
            Self::FurnaceFood => "furnace_food",
            Self::FurnaceBlocks => "furnace_blocks",
            Self::FurnaceMisc => "furnace_misc",
            Self::BlastBlocks => "blast_furnace_blocks",
            Self::BlastMisc => "blast_furnace_misc",
            Self::SmokerFood => "smoker_food",
        }
    }
}
/// A tab button: the search tab or one category.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tab {
    Search,
    Category(Category),
}
impl Tab {
    /// The tab's icon items (`TabInfo`).
    pub fn icons(self) -> (&'static str, Option<&'static str>) {
        use Category::*;
        match self {
            Self::Search => ("minecraft:compass", None),
            Self::Category(c) => match c {
                CraftingEquipment => ("minecraft:iron_axe", Some("minecraft:golden_sword")),
                CraftingBuilding => ("minecraft:bricks", None),
                CraftingMisc => ("minecraft:lava_bucket", Some("minecraft:apple")),
                CraftingRedstone => ("minecraft:redstone", None),
                FurnaceFood | SmokerFood => ("minecraft:porkchop", None),
                FurnaceBlocks => ("minecraft:stone", None),
                FurnaceMisc => ("minecraft:lava_bucket", Some("minecraft:emerald")),
                BlastBlocks => ("minecraft:redstone_ore", None),
                BlastMisc => ("minecraft:iron_shovel", Some("minecraft:golden_leggings")),
            },
        }
    }
}

/// `RecipeDisplay`: how a recipe is laid out.
#[derive(Clone, Debug)]
pub enum Display {
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<Ingredient>>,
    },
    Shapeless(Vec<Ingredient>),
    /// A smelting-type recipe: its input; the fuel is any fuel.
    Furnace(Ingredient),
}
/// One recipe of the book (`RecipeDisplayEntry`).
#[derive(Clone, Debug)]
pub struct Entry {
    /// The recipe id, `minecraft:name`.
    pub id: String,
    /// `RecipeDisplayId`: the display's index among every recipe's displays
    /// (`Book::set_display_ids`; -1 until then).
    pub display_id: i32,
    pub category: Category,
    pub group: Option<String>,
    pub display: Display,
    pub result: ItemStack,
    /// Every stack its display can show as the result when that is more than
    /// `result` (a map copy of any map, an arrow of any potion): the search
    /// tree reads them all, the button shows the first.
    pub variants: Vec<ItemStack>,
    /// Ingredients shown as other stacks than their items (by index in
    /// `requirements`): the potions an imbued arrow's lingering potion is.
    pub stacks: Vec<(usize, Vec<ItemStack>)>,
}
impl Entry {
    /// Every stack the result display cycles through.
    pub fn results(&self) -> &[ItemStack] {
        if self.variants.is_empty() {
            std::slice::from_ref(&self.result)
        } else {
            &self.variants
        }
    }
    /// The ingredients that must be matched to craft it, in order.
    pub fn requirements(&self) -> Vec<&Ingredient> {
        match &self.display {
            Display::Shaped { cells, .. } => cells.iter().flatten().collect(),
            Display::Shapeless(list) => list.iter().collect(),
            Display::Furnace(i) => vec![i],
        }
    }
    /// `CraftingRecipeBookComponent.canDisplay` for a grid.
    pub fn fits(&self, grid_width: usize, grid_height: usize) -> bool {
        match &self.display {
            Display::Shaped { width, height, .. } => grid_width >= *width && grid_height >= *height,
            Display::Shapeless(list) => grid_width * grid_height >= list.len(),
            Display::Furnace(_) => true,
        }
    }
    /// The size the recipe takes in the grid (`PlaceRecipeHelper`).
    pub fn size_in(&self, grid_width: usize, grid_height: usize) -> (usize, usize) {
        match &self.display {
            Display::Shaped { width, height, .. } => (*width, *height),
            _ => (grid_width, grid_height),
        }
    }
    /// The ingredient of each grid cell the recipe fills, with its grid index
    /// (the ghost recipe and the placement use the same layout).
    pub fn grid_cells(&self, grid_width: usize, grid_height: usize) -> Vec<(Option<usize>, usize)> {
        let (w, h) = self.size_in(grid_width, grid_height);
        // One entry per recipe cell: the index of its ingredient, if any.
        let entries: Vec<Option<usize>> = match &self.display {
            Display::Shaped { cells, .. } => {
                let mut next = 0;
                cells
                    .iter()
                    .map(|c| {
                        c.as_ref().map(|_| {
                            next += 1;
                            next - 1
                        })
                    })
                    .collect()
            }
            Display::Shapeless(list) => (0..list.len()).map(Some).collect(),
            Display::Furnace(_) => vec![Some(0)],
        };
        place_cells(grid_width, grid_height, w, h, &entries)
    }
}

/// `PlaceRecipeHelper.placeRecipe`: the grid index each entry goes to; a
/// recipe smaller than half the grid is centred.
pub fn place_cells<T: Clone>(
    grid_width: usize,
    grid_height: usize,
    recipe_width: usize,
    recipe_height: usize,
    entries: &[T],
) -> Vec<(T, usize)> {
    let mut out = Vec::new();
    let mut iter = entries.iter().peekable();
    let mut index = 0usize;
    let mut y = 0;
    while y < grid_height {
        let center_rows = (recipe_height as f32) < grid_height as f32 / 2.0;
        let start_row = (grid_height as f32 / 2.0 - recipe_height as f32 / 2.0).floor() as usize;
        if center_rows && start_row > y {
            index += grid_width;
            y += 1;
        }
        for x in 0..grid_width {
            if iter.peek().is_none() {
                return out;
            }
            let center_cols = (recipe_width as f32) < grid_width as f32 / 2.0;
            let start_col = (grid_width as f32 / 2.0 - recipe_width as f32 / 2.0).floor() as usize;
            let mut total = recipe_width;
            let mut add = x < recipe_width;
            if center_cols {
                total = start_col + recipe_width;
                add = start_col <= x && x < start_col + recipe_width;
            }
            if add {
                if let Some(item) = iter.next() {
                    out.push((item.clone(), index));
                }
            } else if total == x {
                index += grid_width - x;
                break;
            }
            index += 1;
        }
        y += 1;
    }
    out
}

/// A group of recipes shown as one button (`RecipeCollection`).
#[derive(Clone, Debug, Default)]
pub struct Collection {
    /// Indices into `Book::entries`.
    pub entries: Vec<usize>,
}

/// Every recipe the menus' books can show.
#[derive(Default)]
pub struct Book {
    pub entries: Vec<Entry>,
    /// `Recipe.placementInfo` by recipe id: what the book's craftable test
    /// and the server's placement look for.
    pub placements: HashMap<String, Placement>,
    /// Item id -> registry position, to order ingredient choices.
    pub order: HashMap<String, usize>,
}
fn crafting_category(name: &str) -> Category {
    match name {
        "building" => Category::CraftingBuilding,
        "redstone" => Category::CraftingRedstone,
        "equipment" => Category::CraftingEquipment,
        _ => Category::CraftingMisc,
    }
}
impl Book {
    pub fn build(recipes: &Recipes, items: &Items) -> Self {
        let mut entries = Vec::new();
        let group = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        for (name, recipe) in &recipes.crafting {
            let (category, grp) = recipes.meta.get(name).cloned().unwrap_or_default();
            let (display, result) = match recipe {
                Recipe::Shaped {
                    width,
                    height,
                    cells,
                    result,
                } => (
                    Display::Shaped {
                        width: *width,
                        height: *height,
                        cells: cells.clone(),
                    },
                    result,
                ),
                Recipe::Shapeless {
                    ingredients,
                    result,
                } => (Display::Shapeless(ingredients.clone()), result),
                // Custom recipes are never learnable; the others show one
                // display per material count (TransmuteRecipe.display).
                Recipe::Special(special) => {
                    for shown in special.book_displays() {
                        entries.push(Entry {
                            id: format!("minecraft:{name}"),
                            display_id: -1,
                            category: crafting_category(&category),
                            group: group(&grp),
                            display: shown.display,
                            result: shown.result,
                            variants: shown.variants,
                            stacks: shown.stacks,
                        });
                    }
                    continue;
                }
            };
            entries.push(Entry {
                id: format!("minecraft:{name}"),
                display_id: -1,
                category: crafting_category(&category),
                group: group(&grp),
                display,
                result: result.create(),
                variants: Vec::new(),
                stacks: Vec::new(),
            });
        }
        for c in &recipes.cooking {
            let category = match (c.kind.as_str(), c.category.as_str()) {
                ("minecraft:smelting", "food") => Category::FurnaceFood,
                ("minecraft:smelting", "blocks") => Category::FurnaceBlocks,
                ("minecraft:smelting", _) => Category::FurnaceMisc,
                ("minecraft:blasting", "blocks") => Category::BlastBlocks,
                ("minecraft:blasting", _) => Category::BlastMisc,
                ("minecraft:smoking", _) => Category::SmokerFood,
                _ => continue,
            };
            entries.push(Entry {
                id: format!("minecraft:{}", c.id),
                display_id: -1,
                category,
                group: group(&c.group),
                display: Display::Furnace(c.ingredient.clone()),
                result: c.result.clone(),
                variants: Vec::new(),
                stacks: Vec::new(),
            });
        }
        let order = items
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.to_string(), i))
            .collect();
        let placements = entries
            .iter()
            .filter_map(|e| Some((e.id.clone(), recipes.placement(&e.id)?)))
            .collect();
        Self {
            entries,
            placements,
            order,
        }
    }
    /// `RecipeDisplayEntry.canCraft`: the recipe's placement ingredients
    /// (`craftingRequirements`) can all be found in `stock`.
    pub fn can_craft(&self, entry: &Entry, stock: &crate::menu::place::Stacked) -> bool {
        self.placements
            .get(&entry.id)
            .is_some_and(|p| stock.can_craft(&p.ingredients.iter().collect::<Vec<_>>()))
    }
    /// Numbers the entries as `RecipeManager.unpackRecipeInfo` numbers the
    /// displays: every recipe in id order, each with its displays.
    pub fn set_display_ids(&mut self, index: &RecipeIndex) {
        let mut previous: Option<(String, i32)> = None;
        for e in &mut self.entries {
            let next = match &previous {
                Some((id, last)) if *id == e.id => last + 1,
                _ => index.first_display(&e.id).unwrap_or(-1),
            };
            e.display_id = next;
            previous = Some((e.id.clone(), next));
        }
    }
    /// The entry of a display id.
    pub fn entry_of_display(&self, display: i32) -> Option<usize> {
        self.entries.iter().position(|e| e.display_id == display)
    }
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
    /// `ingredient`'s items as its display resolves them (the cycling order
    /// of ghost slots and the overlay).
    pub fn items_of(&self, ingredient: &Ingredient) -> Vec<String> {
        ingredient.1.clone()
    }
    /// `RecipeBookComponent.fillGhostRecipe` of a book type: the result and the
    /// ingredients faded into the menu's slots, in the order the component
    /// sets them. `grid` is the side of the crafting grid, `fuel_empty`
    /// whether the furnace's fuel slot is empty.
    pub fn ghost(
        &self,
        entry: &Entry,
        book_type: BookType,
        grid: usize,
        fuel_empty: bool,
        items: &crate::items::Items,
    ) -> Vec<GhostSlot> {
        let stacks = |ingredient: &Ingredient| -> Vec<ItemStack> {
            self.items_of(ingredient)
                .iter()
                .map(|i| ItemStack::new(i, 1))
                .collect()
        };
        let mut out = Vec::new();
        match book_type {
            BookType::Crafting => {
                out.push(GhostSlot {
                    target: GhostTarget::Result,
                    items: entry.results().to_vec(),
                });
                let requirements = entry.requirements();
                for (ingredient, index) in entry.grid_cells(grid, grid) {
                    let Some(i) = ingredient else { continue };
                    let list = match entry.stacks.iter().find(|s| s.0 == i) {
                        Some(shown) => shown.1.clone(),
                        None => stacks(requirements[i]),
                    };
                    if !list.is_empty() {
                        out.push(GhostSlot {
                            target: GhostTarget::Grid(index),
                            items: list,
                        });
                    }
                }
            }
            _ => {
                out.push(GhostSlot {
                    target: GhostTarget::Result,
                    items: entry.results().to_vec(),
                });
                if let Display::Furnace(ingredient) = &entry.display {
                    out.push(GhostSlot {
                        target: GhostTarget::Input,
                        items: stacks(ingredient),
                    });
                    if fuel_empty {
                        // `SlotDisplay.AnyFuel`: every item with a cooking fuel
                        // component, the registry's component lookup listing the
                        // items of one component value together, the values in
                        // the order of their first item.
                        let mut groups: Vec<(String, Vec<ItemStack>)> = Vec::new();
                        for item in &items.registry_order {
                            let Some(fuel) = items.get(item).and_then(|i| i.fuel.as_ref()) else {
                                continue;
                            };
                            let key = format!("{fuel:?}");
                            match groups.iter_mut().find(|g| g.0 == key) {
                                Some(group) => group.1.push(ItemStack::new(item, 1)),
                                None => groups.push((key, vec![ItemStack::new(item, 1)])),
                            }
                        }
                        let fuels = groups.into_iter().flat_map(|g| g.1).collect();
                        out.push(GhostSlot {
                            target: GhostTarget::Fuel,
                            items: fuels,
                        });
                    }
                }
            }
        }
        // `GhostSlots.setSlot` keeps only slots with something to show.
        out.retain(|g| !g.items.is_empty());
        out
    }
    /// The recipe-book search tree (`SessionSearchTrees.updateRecipes`, a
    /// `FullTextSearchTree`): without a colon the text must occur in a
    /// tooltip line of a result item of the collection; `namespace:path`
    /// needs the namespace in a result id and the path in an id or a line.
    pub fn collection_matches(
        &self,
        entries: &[usize],
        query: &str,
        tooltip: &dyn Fn(&ItemStack) -> Vec<String>,
    ) -> bool {
        let query = query.to_lowercase();
        let results: Vec<&ItemStack> = entries
            .iter()
            .flat_map(|&i| self.entries[i].results())
            .collect();
        let in_lines = |text: &str| {
            results
                .iter()
                .any(|r| tooltip(r).iter().any(|l| l.to_lowercase().contains(text)))
        };
        let Some((namespace, path)) = query.split_once(':') else {
            return in_lines(&query);
        };
        let (namespace, path) = (namespace.trim(), path.trim());
        let id = |r: &&ItemStack| {
            let (ns, p) = r.item.split_once(':').unwrap_or(("minecraft", &r.item));
            (ns.to_lowercase(), p.to_lowercase())
        };
        results.iter().any(|r| id(r).0.contains(namespace))
            && (results.iter().any(|r| id(r).1.contains(path)) || in_lines(path))
    }
}

/// Where a ghost recipe's stacks are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhostTarget {
    Result,
    /// A crafting grid slot, row by row.
    Grid(usize),
    /// A furnace's input and fuel slots.
    Input,
    Fuel,
}
/// One slot of a ghost recipe and the stacks it cycles through.
#[derive(Clone, Debug)]
pub struct GhostSlot {
    pub target: GhostTarget,
    pub items: Vec<ItemStack>,
}

/// A `RecipeDisplayId` as a `HashMap` key: a record of one int hashes as it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DisplayKey(i32);
impl horizon_world::java_hash_map::JavaKey for DisplayKey {
    fn java_hash(&self) -> i32 {
        self.0
    }
    fn tie_break(&self, other: &Self) -> std::cmp::Ordering {
        if self.0 < other.0 {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    }
}

/// `ClientRecipeBook`: the displays the client was sent, in a `HashMap` whose
/// iteration order (the display ids' buckets, then arrival order) decides the
/// order of the book's buttons.
#[derive(Default)]
pub struct ClientBook {
    known: horizon_world::java_hash_map::JavaHashMap<DisplayKey, ()>,
    pub highlight: std::collections::HashSet<i32>,
}
impl ClientBook {
    pub fn add(&mut self, display: i32) {
        self.known.put(DisplayKey(display), ());
    }
    pub fn remove(&mut self, display: i32) {
        self.known.remove(&DisplayKey(display));
        self.highlight.remove(&display);
    }
    /// `clear` (a replacing `ClientboundRecipeBookAddPacket`): the table keeps its size.
    pub fn clear(&mut self) {
        self.known.clear();
        self.highlight.clear();
    }
    /// The known display ids in iteration order.
    pub fn displays(&self) -> Vec<i32> {
        self.known.entries().map(|(k, _)| k.0).collect()
    }
    /// `rebuildCollections` then `getCollection(tab)`: per category, a
    /// collection per entry without a group or per group, in the order the
    /// known displays iterate; the search tab lists its categories in turn.
    pub fn collections(&self, book: &Book, book_type: BookType, tab: Tab) -> Vec<Collection> {
        let by_display: HashMap<i32, usize> = book
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.display_id, i))
            .collect();
        let mut by_category: HashMap<Category, Vec<Collection>> = HashMap::new();
        let mut groups: HashMap<(Category, String), (Category, usize)> = HashMap::new();
        for display in self.displays() {
            let Some(&i) = by_display.get(&display) else {
                continue;
            };
            let e = &book.entries[i];
            let list = by_category.entry(e.category).or_default();
            match &e.group {
                None => list.push(Collection { entries: vec![i] }),
                Some(g) => match groups.get(&(e.category, g.clone())) {
                    Some(&(_, at)) => list[at].entries.push(i),
                    None => {
                        groups.insert((e.category, g.clone()), (e.category, list.len()));
                        list.push(Collection { entries: vec![i] });
                    }
                },
            }
        }
        let categories: Vec<Category> = match tab {
            Tab::Search => book_type.search_categories().to_vec(),
            Tab::Category(c) => vec![c],
        };
        categories
            .into_iter()
            .flat_map(|c| by_category.remove(&c).unwrap_or_default())
            .collect()
    }
}

/// What the server knows of each recipe of the datapack: whether it is
/// special (`Recipe.isSpecial`: never learnable) and how many recipe book
/// displays it has (`RecipeManager.listDisplaysForRecipe`).
#[derive(Clone, Debug, Default)]
pub struct RecipeIndex {
    recipes: std::collections::BTreeMap<String, Known>,
}
#[derive(Clone, Debug)]
struct Known {
    special: bool,
    displays: usize,
    first_display: i32,
    /// `RecipeDisplay.craftingStation`: the workstation item of its displays.
    station: &'static str,
    /// `Recipe.showNotification`: unlocking it shows the `RecipeToast`.
    notify: bool,
    /// `result.id`, for the recipes the book doesn't list.
    result: Option<String>,
}
/// `RecipeDisplay.craftingStation` of a recipe type.
fn crafting_station(kind: &str) -> &'static str {
    match kind {
        "minecraft:smelting" => "minecraft:furnace",
        "minecraft:blasting" => "minecraft:blast_furnace",
        "minecraft:smoking" => "minecraft:smoker",
        "minecraft:campfire_cooking" => "minecraft:campfire",
        "minecraft:stonecutting" => "minecraft:stonecutter",
        "minecraft:smithing_transform" | "minecraft:smithing_trim" => "minecraft:smithing_table",
        _ => "minecraft:crafting_table",
    }
}
impl RecipeIndex {
    /// Reads `data/minecraft/recipe/*.json` under `root`, with the world's
    /// packs' recipe files over it (`overlay`).
    pub fn load(root: &std::path::Path, overlay: &PackOverlay) -> Self {
        let mut recipes = std::collections::BTreeMap::new();
        let mut insert = |id: String, v: &serde_json::Value| {
            let kind = v["type"].as_str().unwrap_or_default();
            let special = kind.starts_with("minecraft:crafting_special_")
                || kind == "minecraft:crafting_decorated_pot";
            let displays = if special {
                0
            } else if kind == "minecraft:crafting_transmute" {
                // TransmuteRecipe.display: one per material count.
                match &v["material_count"] {
                    serde_json::Value::Object(o) => {
                        let min = o.get("min").and_then(|m| m.as_u64()).unwrap_or(1);
                        let max = o.get("max").and_then(|m| m.as_u64()).unwrap_or(8);
                        (max.saturating_sub(min) + 1) as usize
                    }
                    _ => 1,
                }
            } else {
                1
            };
            recipes.insert(
                id,
                Known {
                    special,
                    displays,
                    first_display: 0,
                    station: crafting_station(kind),
                    notify: v["show_notification"].as_bool().unwrap_or(true),
                    result: v["result"]["id"].as_str().map(str::to_owned),
                },
            );
        };
        let dir = root.join("data/minecraft/recipe");
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let (Some(stem), Some(v)) = (
                path.file_stem().and_then(|s| s.to_str()),
                std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()),
            ) else {
                continue;
            };
            insert(format!("minecraft:{stem}"), &v);
        }
        // The world's packs' recipes over the game's own: the top pack's
        // file for each id replaces it (`SimpleJsonResourceReloadListener`).
        for (id, files) in overlay.kind("recipe") {
            match serde_json::from_slice::<serde_json::Value>(PackOverlay::top(files)) {
                Ok(v) => insert(id.to_owned(), &v),
                Err(e) => log::warn!("Horizon: pack recipe {id} not shown in the book: {e}"),
            }
        }
        // RecipeManager keeps its recipes sorted by id and numbers the displays in that order.
        let mut next = 0;
        for r in recipes.values_mut() {
            r.first_display = next;
            next += r.displays as i32;
        }
        Self { recipes }
    }
    pub fn contains(&self, id: &str) -> bool {
        self.recipes.contains_key(id)
    }
    pub fn special(&self, id: &str) -> bool {
        self.recipes.get(id).is_some_and(|r| r.special)
    }
    pub fn displays(&self, id: &str) -> usize {
        self.recipes.get(id).map_or(0, |r| r.displays)
    }
    /// The crafting station shown on its displays' toast, when unlocking it
    /// shows one (`Recipe.showNotification`).
    pub fn toast_station(&self, id: &str) -> Option<&'static str> {
        self.recipes.get(id).filter(|r| r.notify).map(|r| r.station)
    }
    /// The result item id its file names (none for trims).
    pub fn result(&self, id: &str) -> Option<&str> {
        self.recipes.get(id).and_then(|r| r.result.as_deref())
    }
    /// Every recipe id, in id order.
    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.recipes.keys()
    }
    /// The display ids of a recipe (none for a special one).
    pub fn display_ids(&self, id: &str) -> std::ops::Range<i32> {
        self.recipes.get(id).map_or(0..0, |r| {
            r.first_display..r.first_display + r.displays as i32
        })
    }
    /// The recipe a display id belongs to (`RecipeManager.getRecipeFromDisplay`).
    pub fn recipe_of_display(&self, display: i32) -> Option<&str> {
        self.recipes
            .iter()
            .find(|(_, r)| {
                (r.first_display..r.first_display + r.displays as i32).contains(&display)
            })
            .map(|(id, _)| id.as_str())
    }
    pub fn first_display(&self, id: &str) -> Option<i32> {
        self.recipes
            .get(id)
            .filter(|r| r.displays > 0)
            .map(|r| r.first_display)
    }
}

/// `ServerRecipeBook`: the player's `recipeBook` compound, the known recipes
/// and the ones still to highlight (`toBeDisplayed`) with the open and
/// filter settings of each `RecipeBookType`.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// `isGuiOpen`-style flags per `BookType`.
    pub open: [bool; 4],
    pub filtering: [bool; 4],
    /// Known recipe ids (`recipes`).
    pub recipes: BTreeSet<String>,
    /// Recipes not yet shown in the book (`toBeDisplayed`): they animate once.
    pub to_be_displayed: BTreeSet<String>,
}
impl Settings {
    /// `ServerRecipeBook.addRecipes`: the displays added (the count
    /// `/recipe give` reports) and the recipes now known, in order.
    pub fn add_recipes(&mut self, index: &RecipeIndex, ids: &[String]) -> (usize, Vec<String>) {
        let mut displays = 0;
        let mut added = Vec::new();
        for id in ids {
            if !self.recipes.contains(id) && !index.special(id) {
                self.recipes.insert(id.clone());
                self.to_be_displayed.insert(id.clone());
                displays += index.displays(id);
                added.push(id.clone());
            }
        }
        (displays, added)
    }
    /// `ServerRecipeBook.removeRecipes`: the display ids removed, which its
    /// `ClientboundRecipeBookRemovePacket` lists (a transmute recipe has one
    /// per material count).
    pub fn remove_recipes(&mut self, index: &RecipeIndex, ids: &[String]) -> Vec<i32> {
        let mut displays = Vec::new();
        for id in ids {
            if self.recipes.remove(id) {
                self.to_be_displayed.remove(id);
                displays.extend(index.display_ids(id));
            }
        }
        displays
    }
    /// `removeHighlight` (`ServerboundRecipeBookSeenRecipePacket`).
    pub fn remove_highlight(&mut self, id: &str) {
        self.to_be_displayed.remove(id);
    }
    pub fn knows(&self, id: &str) -> bool {
        self.recipes.contains(id)
    }
    /// The compound as saved, every recipe kept.
    pub fn read(tag: Option<&Tag>) -> Self {
        Self::read_valid(tag, &|_| true)
    }
    /// `ServerRecipeBook.loadUntrusted`: unknown recipes are dropped. Without
    /// both lists the compound does not decode and nothing is read.
    pub fn read_valid(tag: Option<&Tag>, valid: &dyn Fn(&str) -> bool) -> Self {
        let mut s = Self::default();
        let Some(tag) = tag else { return s };
        let ids = |key: &str| match tag.get(key) {
            Some(Tag::List(list)) => Some(
                list.iter()
                    .filter_map(|t| match t {
                        Tag::String(id) => Some(id.to_string_lossy()),
                        _ => None,
                    })
                    .map(|id| {
                        if id.contains(':') {
                            id
                        } else {
                            format!("minecraft:{id}")
                        }
                    })
                    .filter(|id| valid(id))
                    .collect::<BTreeSet<String>>(),
            ),
            _ => None,
        };
        let (Some(recipes), Some(to_be_displayed)) = (ids("recipes"), ids("toBeDisplayed")) else {
            return s;
        };
        let flag = |k: &str| matches!(tag.get(k), Some(Tag::Byte(b)) if *b != 0);
        for b in BookType::ALL {
            let (open, filter) = b.keys();
            s.open[b.index()] = flag(open);
            s.filtering[b.index()] = flag(filter);
        }
        s.recipes = recipes;
        s.to_be_displayed = to_be_displayed;
        s
    }
    /// The compound written to the player file (vanilla's lists follow an
    /// identity hash order; these are sorted).
    pub fn write(&self) -> Tag {
        let mut entries: Vec<(&str, Tag)> = Vec::new();
        for b in BookType::ALL {
            let (open, filter) = b.keys();
            // The codec omits settings at their default (false).
            if self.open[b.index()] {
                entries.push((open, Tag::Byte(1)));
            }
            if self.filtering[b.index()] {
                entries.push((filter, Tag::Byte(1)));
            }
        }
        let list = |ids: &BTreeSet<String>| {
            Tag::List(ids.iter().map(|i| Tag::String(i.as_str().into())).collect())
        };
        entries.push(("recipes", list(&self.recipes)));
        entries.push(("toBeDisplayed", list(&self.to_be_displayed)));
        Tag::compound(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ing(items: &[&str]) -> Ingredient {
        Ingredient::of(items.iter().map(|s| (*s).to_owned()).collect())
    }
    #[test]
    fn the_index_knows_each_recipes_toast_station_and_result() {
        let root = std::env::temp_dir().join(format!("horizon-toast-index-{}", std::process::id()));
        let dir = root.join("data/minecraft/recipe");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, json) in [
            (
                "stick",
                r#"{"type":"minecraft:crafting_shaped","result":{"id":"minecraft:stick"}}"#,
            ),
            (
                "crafting_table",
                r#"{"type":"minecraft:crafting_shaped","show_notification":false,"result":{"id":"minecraft:crafting_table"}}"#,
            ),
            (
                "cooked_beef_from_campfire_cooking",
                r#"{"type":"minecraft:campfire_cooking","result":{"id":"minecraft:cooked_beef"}}"#,
            ),
            (
                "stone_slab_from_stonecutting",
                r#"{"type":"minecraft:stonecutting","result":{"id":"minecraft:stone_slab"}}"#,
            ),
            (
                "coast_trim",
                r#"{"type":"minecraft:smithing_trim","pattern":"minecraft:coast"}"#,
            ),
        ] {
            std::fs::write(dir.join(format!("{name}.json")), json).unwrap();
        }
        let index = RecipeIndex::load(&root, &PackOverlay::NONE);
        std::fs::remove_dir_all(&root).unwrap();
        let toast = |id: &str| (index.toast_station(id), index.result(id));
        assert_eq!(
            toast("minecraft:stick"),
            (Some("minecraft:crafting_table"), Some("minecraft:stick"))
        );
        // show_notification: false unlocks silently.
        assert_eq!(toast("minecraft:crafting_table").0, None);
        assert_eq!(
            toast("minecraft:cooked_beef_from_campfire_cooking"),
            (Some("minecraft:campfire"), Some("minecraft:cooked_beef"))
        );
        assert_eq!(
            toast("minecraft:stone_slab_from_stonecutting").0,
            Some("minecraft:stonecutter")
        );
        assert_eq!(
            toast("minecraft:coast_trim"),
            (Some("minecraft:smithing_table"), None)
        );
    }
    #[test]
    fn small_recipes_are_centred_in_the_grid() {
        // One ingredient in a 3x3 grid goes to the middle cell.
        assert_eq!(place_cells(3, 3, 1, 1, &['a']), vec![('a', 4)]);
        // A 2x2 recipe fills the top-left corner.
        let cells: Vec<usize> = place_cells(3, 3, 2, 2, &['a', 'b', 'c', 'd'])
            .into_iter()
            .map(|(_, i)| i)
            .collect();
        assert_eq!(cells, vec![0, 1, 3, 4]);
        // A one-row recipe sits on the middle row.
        let row: Vec<usize> = place_cells(3, 3, 3, 1, &['a', 'b', 'c'])
            .into_iter()
            .map(|(_, i)| i)
            .collect();
        assert_eq!(row, vec![3, 4, 5]);
        // The 2x2 grid is never centred.
        let small: Vec<usize> = place_cells(2, 2, 1, 1, &['a'])
            .into_iter()
            .map(|(_, i)| i)
            .collect();
        assert_eq!(small, vec![0]);
    }
    #[test]
    fn collections_group_recipes_of_a_category() {
        let entry = |id: &str, display_id, group: Option<&str>, category| Entry {
            id: id.into(),
            display_id,
            category,
            group: group.map(str::to_owned),
            display: Display::Shapeless(vec![ing(&["minecraft:stick"])]),
            result: ItemStack::new("minecraft:stick", 1),
            variants: Vec::new(),
            stacks: Vec::new(),
        };
        let book = Book {
            entries: vec![
                entry("minecraft:a", 0, Some("planks"), Category::CraftingBuilding),
                entry("minecraft:b", 1, Some("planks"), Category::CraftingBuilding),
                entry("minecraft:c", 2, None, Category::CraftingBuilding),
                entry("minecraft:d", 3, Some("planks"), Category::CraftingMisc),
            ],
            placements: HashMap::new(),
            order: HashMap::new(),
        };
        let mut client = ClientBook::default();
        for d in 0..4 {
            client.add(d);
        }
        let tab = Tab::Category(Category::CraftingBuilding);
        let c = client.collections(&book, BookType::Crafting, tab);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].entries, vec![0, 1]);
        // The search tab lists every category; the same group in another category is separate.
        let all = client.collections(&book, BookType::Crafting, Tab::Search);
        assert_eq!(all.len(), 3);
        // Displays the client was not sent are not listed.
        client.clear();
        client.add(2);
        assert_eq!(client.collections(&book, BookType::Crafting, tab).len(), 1);
    }
    #[test]
    fn display_ids_collide_in_a_small_table() {
        // A 16-bucket table: 17 and 1 share a bucket and keep arrival order.
        let mut client = ClientBook::default();
        for d in [17, 3, 1] {
            client.add(d);
        }
        assert_eq!(client.displays(), vec![17, 1, 3]);
    }
    #[test]
    fn search_reads_tooltip_lines_and_ids() {
        let book = Book {
            entries: vec![Entry {
                id: "minecraft:stick".into(),
                display_id: 0,
                category: Category::CraftingMisc,
                group: None,
                display: Display::Shapeless(Vec::new()),
                result: ItemStack::new("minecraft:stick", 4),
                variants: Vec::new(),
                stacks: Vec::new(),
            }],
            placements: HashMap::new(),
            order: HashMap::new(),
        };
        let lines = |s: &ItemStack| vec![format!("Item {}", s.item), "Stick".to_owned()];
        assert!(book.collection_matches(&[0], "STI", &lines));
        // Plain text searches tooltips only, not the registry path.
        assert!(!book.collection_matches(&[0], "stick:", &lines));
        assert!(book.collection_matches(&[0], "minecraft:stic", &lines));
        assert!(!book.collection_matches(&[0], "other:stick", &lines));
    }
    #[test]
    fn settings_round_trip() {
        let mut s = Settings::default();
        s.open[BookType::Furnace.index()] = true;
        s.filtering[BookType::Crafting.index()] = true;
        s.recipes = ["minecraft:stick".to_owned()].into();
        let back = Settings::read(Some(&s.write()));
        assert_eq!(back.open, s.open);
        assert_eq!(back.filtering, s.filtering);
        assert_eq!(back.recipes, s.recipes);
    }
}
