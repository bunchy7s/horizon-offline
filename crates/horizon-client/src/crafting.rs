//! Crafting and cooking recipes from the pinned data pack: shaped patterns
//! (`ShapedRecipePattern.matches`), shapeless recipes (a bipartite match of
//! ingredients to stacks, as `StackedContents.canCraft`), the special and
//! transmuting crafting recipes (`CustomRecipe` and friends), and
//! furnace-type cooking lookups.
use crate::{
    Result,
    item_data::StackData,
    items::{ItemStack, Items},
    particles::{DYE_FIREWORK_COLORS, FireworkExplosion, FireworkShape},
};
use horizon_world::loot::PackOverlay;
use horizon_world::nbt::Tag;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// A set of accepted items (expanded from an item, list or tag), and the
/// items in its `HolderSet` order (`Ingredient.display`'s `TagSlotDisplay`).
#[derive(Clone, Debug)]
pub struct Ingredient(pub HashSet<String>, pub Vec<String>);
impl Ingredient {
    /// The items of a list, in order.
    pub fn of(items: Vec<String>) -> Self {
        Self(items.iter().cloned().collect(), items)
    }
    pub(crate) fn parse(v: &Value, items: &Items) -> Option<Self> {
        let mut list: Vec<String> = Vec::new();
        let mut add = |s: &str| {
            let more: Vec<String> = match s.strip_prefix('#') {
                Some(tag) => items.item_tag_order.get(tag).cloned().unwrap_or_default(),
                None => vec![s.to_owned()],
            };
            for i in more {
                if !list.contains(&i) {
                    list.push(i);
                }
            }
        };
        match v {
            Value::String(s) => add(s),
            Value::Array(a) => a.iter().filter_map(Value::as_str).for_each(&mut add),
            _ => return None,
        }
        (!list.is_empty()).then(|| Self::of(list))
    }
    pub fn test(&self, stack: &ItemStack) -> bool {
        self.0.contains(&*stack.item)
    }
}
/// `PlacementInfo`: the ingredients to find and, per recipe slot, the index
/// of its ingredient (`slotsToIngredientIndex`, -1 for an empty slot). Empty
/// when the recipe is impossible to place.
#[derive(Clone, Debug, Default)]
pub struct Placement {
    pub ingredients: Vec<Ingredient>,
    pub slots: Vec<i32>,
    /// A `ShapedRecipe`'s width and height (the others fill the grid).
    pub shaped: Option<(usize, usize)>,
}
impl Placement {
    /// `PlacementInfo.createFromOptionals` (and `create` with no empty slots).
    pub fn of(cells: Vec<Option<Ingredient>>, shaped: Option<(usize, usize)>) -> Self {
        let mut out = Self {
            shaped,
            ..Self::default()
        };
        for cell in cells {
            match cell {
                Some(i) if i.0.is_empty() => return Self::default(),
                Some(i) => {
                    out.slots.push(out.ingredients.len() as i32);
                    out.ingredients.push(i);
                }
                None => out.slots.push(-1),
            }
        }
        out
    }
    pub fn impossible(&self) -> bool {
        self.slots.is_empty()
    }
}
/// A recipe result: `ItemStackTemplate` (`id`, `count`, `components`), with the
/// id optional for the transmuting recipes that keep the input's item.
#[derive(Clone, Debug)]
pub struct Template {
    pub item: Option<Arc<str>>,
    pub count: u32,
    pub damage: u32,
    pub extra: Option<Box<StackData>>,
}
/// `ItemStackTemplate.CODEC` count: absent means 1, present must be an
/// integer in `1..=99` (`ExtraCodecs.intRange(1, 99)`), else the recipe is
/// not loaded.
pub(crate) fn template_count(v: &Value) -> Option<u32> {
    match v.get("count") {
        None => Some(1),
        Some(c) => c
            .as_u64()
            .filter(|n| (1..=99).contains(n))
            .map(|n| n as u32),
    }
}
impl Template {
    fn parse(v: &Value) -> Option<Self> {
        let (damage, extra) = match v.get("components") {
            Some(c) => crate::item_data::decode_components(Some(&json_tag(c))),
            None => (0, None),
        };
        Some(Self {
            item: v["id"].as_str().map(Arc::from),
            count: template_count(v)?,
            damage,
            extra,
        })
    }
    /// `ItemStackTemplate.create`.
    pub fn create(&self) -> ItemStack {
        ItemStack {
            item: self
                .item
                .clone()
                .unwrap_or_else(|| Arc::from("minecraft:air")),
            count: self.count,
            damage: self.damage,
            extra: self.extra.clone(),
        }
    }
    /// `TransmuteRecipe.createWithOriginalComponents`: the input's component
    /// patch on the result item, then the template's own components over it.
    fn over(&self, input: &ItemStack, extra_count: u32) -> ItemStack {
        let mut out = ItemStack {
            item: self.item.clone().unwrap_or_else(|| input.item.clone()),
            count: self.count + extra_count,
            damage: input.damage,
            extra: input.extra.clone(),
        };
        if self.damage > 0 {
            out.damage = self.damage;
        }
        if let Some(over) = &self.extra {
            overlay(out.data_mut(), over);
        }
        out.tidy();
        out
    }
}
/// `DataComponentPatch` application: every component the overlay sets wins.
fn overlay(dst: &mut StackData, src: &StackData) {
    let none = StackData::default();
    macro_rules! field {
        ($($f:ident),*) => {$(
            if src.$f != none.$f {
                dst.$f = src.$f.clone();
            }
        )*};
    }
    field!(
        enchantments,
        stored_enchantments,
        repair_cost,
        custom_name,
        potion,
        trim,
        container,
        charged,
        fireworks,
        map_id,
        writable_pages,
        written,
        banner_patterns,
        base_color,
        ominous_banner,
        ominous_amplifier,
        dyed_color,
        firework_explosion,
        pot_decorations,
        bundle_contents,
        stew_effects,
        map_post_processing
    );
}
/// A JSON recipe value as the NBT a component codec reads.
fn json_tag(v: &Value) -> Tag {
    match v {
        Value::Null => Tag::End,
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i32::try_from(i).map_or(Tag::Long(i), Tag::Int),
            None => Tag::Double(n.as_f64().unwrap_or(0.0).to_bits()),
        },
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(a) => Tag::List(a.iter().map(json_tag).collect()),
        Value::Object(o) => Tag::compound(o.iter().map(|(k, v)| (k.as_str(), json_tag(v)))),
    }
}
#[derive(Clone, Debug)]
pub enum Recipe {
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<Ingredient>>,
        result: Template,
    },
    Shapeless {
        ingredients: Vec<Ingredient>,
        result: Template,
    },
    /// The recipes with code of their own.
    Special(Special),
}
/// The custom and transmuting crafting recipes.
#[derive(Clone, Debug)]
pub enum Special {
    /// A patterned banner and a blank one of the same colour copy the patterns.
    BannerDuplicate { banner: Ingredient, result: String },
    /// A banner decorates an undecorated shield.
    ShieldDecoration {
        banner: Ingredient,
        target: Ingredient,
        result: String,
    },
    /// `TransmuteRecipe`: one input, 1..8 materials; shulker box and bundle
    /// dyeing, map cloning.
    Transmute {
        input: Ingredient,
        material: Ingredient,
        min: usize,
        max: usize,
        add_count: bool,
        result: Template,
    },
    /// `DyeRecipe`: dyed armour.
    Dye {
        target: Ingredient,
        dye: Ingredient,
        result: Template,
    },
    /// `ImbueRecipe`: tipped arrows.
    Imbue {
        source: Ingredient,
        material: Ingredient,
        result: Template,
    },
    /// `BookCloningRecipe`.
    BookCloning {
        source: Ingredient,
        material: Ingredient,
        min: i32,
        max: i32,
        result: Template,
    },
    /// `FireworkRocketRecipe`.
    FireworkRocket {
        shell: Ingredient,
        fuel: Ingredient,
        star: Ingredient,
        result: Template,
    },
    /// `FireworkStarRecipe`.
    FireworkStar {
        dye: Ingredient,
        fuel: Ingredient,
        trail: Ingredient,
        twinkle: Ingredient,
        shapes: Vec<(FireworkShape, Ingredient)>,
        result: Template,
    },
    /// `FireworkStarFadeRecipe`.
    FireworkStarFade {
        dye: Ingredient,
        target: Ingredient,
        result: Template,
    },
    /// `MapExtendingRecipe`: a map ringed by paper, when its scale is under 4.
    MapExtending {
        map: Ingredient,
        material: Ingredient,
        result: Template,
    },
    /// `DecoratedPotRecipe`: four sherds or bricks.
    DecoratedPot {
        back: Ingredient,
        left: Ingredient,
        right: Ingredient,
        front: Ingredient,
        result: Template,
    },
    /// `RepairItemRecipe`: two damaged items of one kind.
    RepairItem,
}
impl Special {
    /// The recipe-book displays of a learnable (non-custom) crafting recipe:
    /// its layout, its result as the button shows it, and every stack the
    /// result display cycles through when it is more than that one
    /// (`SlotDisplay.resolveForStacks`).
    pub fn book_displays(&self) -> Vec<BookDisplay> {
        use crate::recipe_book::Display;
        let first_of =
            |all: &Vec<ItemStack>, single: ItemStack| all.first().cloned().unwrap_or(single);
        let shown = |display, result, variants| BookDisplay {
            display,
            result,
            variants,
            stacks: Vec::new(),
        };
        match self {
            Self::Transmute {
                input,
                material,
                min,
                max,
                add_count,
                result,
            } => (*min..=*max)
                .map(|n| {
                    let mut ingredients = vec![input.clone()];
                    ingredients.extend(std::iter::repeat_n(material.clone(), n));
                    let mut made = result.create();
                    if *add_count {
                        made.count += n as u32;
                    }
                    // The result keeps the input's item: any item of the ingredient.
                    let all: Vec<ItemStack> = if result.item.is_none() {
                        input
                            .1
                            .iter()
                            .map(|item| {
                                let mut stack = made.clone();
                                stack.item = Arc::from(item.as_str());
                                stack
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    shown(Display::Shapeless(ingredients), first_of(&all, made), all)
                })
                .collect(),
            Self::Dye {
                target,
                dye,
                result,
            } => {
                // DyedSlotDemo: the targets cycle fastest, 16 stacks at most.
                let (targets, dyes) = (&target.1, &dye.1);
                let all: Vec<ItemStack> = (0..targets.len() * dyes.len())
                    .map(|i| {
                        let mut made = result.create();
                        made.item = Arc::from(targets[i % targets.len()].as_str());
                        let name = dyes[i / targets.len()].trim_start_matches("minecraft:");
                        let color = crate::block_entity_render::DYES
                            .iter()
                            .find(|(c, _)| name.strip_suffix("_dye") == Some(c))
                            .map_or(crate::block_entity_render::DYES[0].1, |d| d.1);
                        made.data_mut().dyed_color = Some(color as i32);
                        made
                    })
                    .take(16)
                    .collect();
                let first = result.create();
                vec![shown(
                    Display::Shapeless(vec![target.clone(), dye.clone()]),
                    first_of(&all, first),
                    all,
                )]
            }
            Self::Imbue {
                source,
                material,
                result,
            } => {
                let cells = (0..9)
                    .map(|i| Some(if i == 4 { source } else { material }.clone()))
                    .collect();
                // Any potion's tipped arrow.
                let potions = |item: ItemStack| -> Vec<ItemStack> {
                    crate::brewing::POTIONS
                        .iter()
                        .map(|p| item.clone().with_potion(&format!("minecraft:{}", p.id)))
                        .collect()
                };
                let all = potions(result.create());
                // The source ingredient is shown as a lingering potion of each kind.
                let lingering = source
                    .1
                    .iter()
                    .flat_map(|item| potions(ItemStack::new(item, 1)))
                    .collect();
                vec![BookDisplay {
                    stacks: vec![(4, lingering)],
                    ..shown(
                        Display::Shaped {
                            width: 3,
                            height: 3,
                            cells,
                        },
                        first_of(&all, result.create()),
                        all,
                    )
                }]
            }
            _ => Vec::new(),
        }
    }
}
/// One recipe-book display of a special recipe.
pub struct BookDisplay {
    pub display: crate::recipe_book::Display,
    /// The result as the button shows it (the first of `variants`).
    pub result: ItemStack,
    /// Every stack the result display cycles through; empty for one stack.
    pub variants: Vec<ItemStack>,
    /// Ingredients shown as other stacks than their items, by index in
    /// `Entry::requirements`.
    pub stacks: Vec<(usize, Vec<ItemStack>)>,
}
/// `CraftingInput`: the occupied bounding box of the grid.
pub struct Input<'a> {
    pub width: usize,
    pub height: usize,
    /// Position of the box in the grid.
    pub left: usize,
    pub top: usize,
    cells: Vec<Option<&'a ItemStack>>,
}
impl<'a> Input<'a> {
    /// `CraftingInput.ofPositioned`; None for an empty grid.
    pub fn of(grid: &'a [Option<ItemStack>], size: usize) -> Option<Self> {
        let occupied: Vec<(usize, usize)> = (0..size * size)
            .filter(|&i| grid.get(i).is_some_and(|c| c.is_some()))
            .map(|i| (i % size, i / size))
            .collect();
        let (x0, x1) = (
            occupied.iter().map(|p| p.0).min()?,
            occupied.iter().map(|p| p.0).max()?,
        );
        let (y0, y1) = (
            occupied.iter().map(|p| p.1).min()?,
            occupied.iter().map(|p| p.1).max()?,
        );
        let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
        let cells = (0..width * height)
            .map(|i| grid[(y0 + i / width) * size + x0 + i % width].as_ref())
            .collect();
        Some(Self {
            width,
            height,
            left: x0,
            top: y0,
            cells,
        })
    }
    pub fn get(&self, x: usize, y: usize) -> Option<&'a ItemStack> {
        self.cells.get(y * self.width + x).copied().flatten()
    }
    /// The non-empty stacks in slot order.
    pub fn stacks(&self) -> Vec<&'a ItemStack> {
        self.cells.iter().flatten().copied().collect()
    }
    pub fn count(&self) -> usize {
        self.cells.iter().flatten().count()
    }
}
/// What recipes need to know about an item.
#[derive(Clone, Debug, Default)]
struct Facts {
    /// Index into the dye colours when the item has the `dye` component.
    dye: Option<usize>,
    max_damage: u32,
    max_stack: u32,
    remainder: Option<Arc<str>>,
}
/// The tables recipes consult.
pub struct Context<'a> {
    recipes: &'a Recipes,
    /// A filled map's scale, when its data is known.
    pub map_scale: &'a dyn Fn(i32) -> Option<u8>,
}
impl Context<'_> {
    fn facts(&self, stack: &ItemStack) -> Option<&Facts> {
        self.recipes.facts.get(&stack.item)
    }
    /// `itemStack.has(DataComponents.DYE)`.
    fn dye(&self, stack: &ItemStack) -> Option<usize> {
        self.facts(stack).and_then(|f| f.dye)
    }
    fn max_stack(&self, item: &str) -> u32 {
        self.recipes.facts.get(item).map_or(64, |f| f.max_stack)
    }
}
impl Special {
    /// `Recipe.matches`.
    fn matches(&self, ctx: &Context, input: &Input) -> bool {
        use crate::banner;
        let stacks = input.stacks();
        match self {
            Self::BannerDuplicate { banner: ing, .. } => {
                if stacks.len() != 2 {
                    return false;
                }
                let mut color = None;
                let (mut target, mut source) = (false, false);
                for s in &stacks {
                    let Some(this) = banner::banner_color(&s.item).filter(|_| ing.test(s)) else {
                        return false;
                    };
                    match color {
                        None => color = Some(this),
                        Some(c) if c != this => return false,
                        _ => {}
                    }
                    let layers = banner::layers(s).len();
                    if layers > banner::MAX_LAYERS {
                        return false;
                    }
                    let seen = if layers > 0 { &mut source } else { &mut target };
                    if *seen {
                        return false;
                    }
                    *seen = true;
                }
                source && target
            }
            Self::ShieldDecoration {
                banner: ing,
                target: tgt,
                ..
            } => {
                if stacks.len() != 2 {
                    return false;
                }
                let (mut patterned, mut clear) = (false, false);
                for s in &stacks {
                    if ing.test(s) && banner::banner_color(&s.item).is_some() {
                        if patterned {
                            return false;
                        }
                        patterned = true;
                    } else {
                        if !tgt.test(s) || clear || !banner::layers(s).is_empty() {
                            return false;
                        }
                        clear = true;
                    }
                }
                patterned && clear
            }
            Self::Transmute {
                input: base,
                material,
                min,
                max,
                add_count,
                result,
            } => {
                let n = stacks.len();
                if n < min + 1 || n > max + 1 {
                    return false;
                }
                let mut found: Option<&ItemStack> = None;
                let mut materials = 0;
                for s in &stacks {
                    if base.test(s) {
                        if found.is_some() {
                            return false;
                        }
                        found = Some(s);
                    } else {
                        if !material.test(s) {
                            return false;
                        }
                        materials += 1;
                        if materials > *max {
                            return false;
                        }
                    }
                }
                let Some(found) = found else {
                    return false;
                };
                if materials < *min || materials > *max {
                    return false;
                }
                let count = if *add_count {
                    materials as u32 + result.count
                } else {
                    result.count
                };
                if count != 1 {
                    return true;
                }
                let made = result.over(found, 0);
                made.count <= ctx.max_stack(&made.item) && !made.same_item_same_components(found)
            }
            Self::Dye { target, dye, .. } => {
                if stacks.len() < 2 {
                    return false;
                }
                let (mut has_target, mut has_dyes) = (false, false);
                for s in &stacks {
                    if target.test(s) {
                        if has_target {
                            return false;
                        }
                        has_target = true;
                    } else {
                        if !dye.test(s) || ctx.dye(s).is_none() {
                            return false;
                        }
                        has_dyes = true;
                    }
                }
                has_dyes && has_target
            }
            Self::Imbue {
                source, material, ..
            } => {
                if input.width != 3 || input.height != 3 || stacks.len() != 9 {
                    return false;
                }
                (0..3).all(|y| {
                    (0..3).all(|x| {
                        let ing = if x == 1 && y == 1 { source } else { material };
                        input.get(x, y).is_some_and(|s| ing.test(s))
                    })
                })
            }
            Self::BookCloning {
                source,
                material,
                min,
                max,
                ..
            } => {
                if stacks.len() < 2 {
                    return false;
                }
                let (mut has_material, mut has_source) = (false, false);
                for s in &stacks {
                    if source.test(s) {
                        let Some(book) = s.data().and_then(|d| d.written.as_ref()) else {
                            return false;
                        };
                        let generation = i32::from(book.generation);
                        if generation < *min || generation > *max || has_source {
                            return false;
                        }
                        has_source = true;
                    } else {
                        if !material.test(s) {
                            return false;
                        }
                        has_material = true;
                    }
                }
                has_source && has_material
            }
            Self::FireworkRocket {
                shell, fuel, star, ..
            } => {
                if stacks.len() < 2 {
                    return false;
                }
                let mut has_shell = false;
                let mut fuels = 0;
                for s in &stacks {
                    if shell.test(s) {
                        if has_shell {
                            return false;
                        }
                        has_shell = true;
                    } else if fuel.test(s) {
                        fuels += 1;
                        if fuels > 3 {
                            return false;
                        }
                    } else if !star.test(s) {
                        return false;
                    }
                }
                has_shell && fuels >= 1
            }
            Self::FireworkStar {
                dye,
                fuel,
                trail,
                twinkle,
                shapes,
                ..
            } => {
                if stacks.len() < 2 {
                    return false;
                }
                let (mut has_fuel, mut has_dye, mut has_shape) = (false, false, false);
                let (mut has_trail, mut has_twinkle) = (false, false);
                for s in &stacks {
                    if twinkle.test(s) {
                        if has_twinkle {
                            return false;
                        }
                        has_twinkle = true;
                    } else if trail.test(s) {
                        if has_trail {
                            return false;
                        }
                        has_trail = true;
                    } else if fuel.test(s) {
                        if has_fuel {
                            return false;
                        }
                        has_fuel = true;
                    } else if dye.test(s) && ctx.dye(s).is_some() {
                        has_dye = true;
                    } else {
                        if !shapes.iter().any(|(_, ing)| ing.test(s)) || has_shape {
                            return false;
                        }
                        has_shape = true;
                    }
                }
                has_fuel && has_dye
            }
            Self::FireworkStarFade { dye, target, .. } => {
                if stacks.len() < 2 {
                    return false;
                }
                let (mut has_dye, mut has_target) = (false, false);
                for s in &stacks {
                    if dye.test(s) && ctx.dye(s).is_some() {
                        has_dye = true;
                    } else {
                        if !target.test(s) || has_target {
                            return false;
                        }
                        has_target = true;
                    }
                }
                has_target && has_dye
            }
            Self::MapExtending { map, material, .. } => {
                if input.width != 3 || input.height != 3 || stacks.len() != 9 {
                    return false;
                }
                let ring = (0..3).all(|y| {
                    (0..3).all(|x| {
                        let ing = if x == 1 && y == 1 { map } else { material };
                        input.get(x, y).is_some_and(|s| ing.test(s))
                    })
                });
                if !ring {
                    return false;
                }
                stacks
                    .iter()
                    .find_map(|s| s.data().and_then(|d| d.map_id))
                    .and_then(|id| (ctx.map_scale)(id))
                    .is_some_and(|scale| scale < 4)
            }
            Self::DecoratedPot {
                back,
                left,
                right,
                front,
                ..
            } => {
                input.width == 3
                    && input.height == 3
                    && stacks.len() == 4
                    && input.get(1, 0).is_some_and(|s| back.test(s))
                    && input.get(0, 1).is_some_and(|s| left.test(s))
                    && input.get(2, 1).is_some_and(|s| right.test(s))
                    && input.get(1, 2).is_some_and(|s| front.test(s))
            }
            Self::RepairItem => Self::repair_pair(ctx, &stacks).is_some(),
        }
    }
    /// `RepairItemRecipe.getItemsToCombine`.
    fn repair_pair<'a>(
        ctx: &Context,
        stacks: &[&'a ItemStack],
    ) -> Option<(&'a ItemStack, &'a ItemStack)> {
        let [first, second] = stacks else {
            return None;
        };
        let durable = |s: &ItemStack| ctx.facts(s).is_some_and(|f| f.max_damage > 0);
        (second.item == first.item
            && first.count == 1
            && second.count == 1
            && durable(first)
            && durable(second))
        .then_some((*first, *second))
    }
    /// `Recipe.assemble`; None is the empty stack.
    fn assemble(&self, ctx: &Context, input: &Input) -> Option<ItemStack> {
        use crate::banner;
        let stacks = input.stacks();
        let made = match self {
            Self::BannerDuplicate { result, .. } => {
                let source = stacks
                    .iter()
                    .find(|s| (1..=banner::MAX_LAYERS).contains(&banner::layers(s).len()))?;
                // TransmuteRecipe.createWithOriginalComponents: the source's components.
                ItemStack {
                    item: Arc::from(result.as_str()),
                    count: 1,
                    ..(**source).clone()
                }
            }
            Self::ShieldDecoration {
                banner: ing,
                target,
                result,
            } => {
                let (mut banner_stack, mut shield) = (None, None);
                for s in &stacks {
                    if ing.test(s) && banner::banner_color(&s.item).is_some() {
                        banner_stack = Some(*s);
                    } else if target.test(s) {
                        shield = Some(*s);
                    }
                }
                let (banner_stack, shield) = (banner_stack?, shield?);
                let mut out = ItemStack {
                    item: Arc::from(result.as_str()),
                    count: 1,
                    ..shield.clone()
                };
                let data = out.data_mut();
                data.banner_patterns = banner::layers(banner_stack).to_vec();
                data.base_color = banner::banner_color(&banner_stack.item).map(Arc::from);
                out
            }
            Self::Transmute {
                input: base,
                material,
                add_count,
                result,
                ..
            } => {
                if *add_count {
                    let mut materials = 0;
                    let mut found = None;
                    for s in &stacks {
                        if base.test(s) {
                            found = Some(*s);
                        } else if material.test(s) {
                            materials += 1;
                        }
                    }
                    result.over(found?, materials)
                } else {
                    let s = stacks.iter().find(|s| base.test(s))?;
                    result.over(s, 0)
                }
            }
            Self::Dye {
                target,
                dye,
                result,
            } => {
                let mut dyes = Vec::new();
                let mut target_stack: Option<&ItemStack> = None;
                for s in &stacks {
                    if target.test(s) {
                        if target_stack.is_some() {
                            return None;
                        }
                        target_stack = Some(s);
                    } else {
                        if !dye.test(s) {
                            return None;
                        }
                        // getOrDefault(DYE, WHITE)
                        dyes.push(ctx.dye(s).unwrap_or(0));
                    }
                }
                let target_stack = target_stack?;
                if dyes.is_empty() {
                    return None;
                }
                let current = target_stack.data().and_then(|d| d.dyed_color);
                let mut out = result.over(target_stack, 0);
                out.data_mut().dyed_color = Some(apply_dyes(current, &dyes));
                out
            }
            Self::Imbue { result, .. } => {
                let source = input.get(1, 1)?;
                let mut out = result.create();
                out.data_mut().potion = source.data().and_then(|d| d.potion.clone());
                out
            }
            Self::BookCloning {
                source,
                material,
                result,
                ..
            } => {
                let mut copies = 0u32;
                let mut book: Option<&ItemStack> = None;
                for s in &stacks {
                    if source.test(s) && s.data().is_some_and(|d| d.written.is_some()) {
                        if book.is_some() {
                            return None;
                        }
                        book = Some(s);
                    } else {
                        if !material.test(s) {
                            return None;
                        }
                        copies += 1;
                    }
                }
                let book = book?;
                // count - 1 can be -1 when no material was seen; matches() ensures one.
                let mut out = result.over(book, copies.saturating_sub(1));
                if let Some(w) = out.data_mut().written.as_mut() {
                    w.generation += 1;
                }
                out
            }
            Self::FireworkRocket {
                fuel, star, result, ..
            } => {
                let mut explosions = Vec::new();
                let mut fuels = 0u8;
                for s in &stacks {
                    if fuel.test(s) {
                        fuels += 1;
                    } else if star.test(s)
                        && let Some(e) = s.data().and_then(|d| d.firework_explosion.clone())
                    {
                        explosions.push(e);
                    }
                }
                let mut out = result.create();
                // A rocket's prototype is one flight and no explosions.
                if fuels != 1 || !explosions.is_empty() {
                    out.data_mut().fireworks = Some((fuels, explosions));
                }
                out
            }
            Self::FireworkStar {
                dye,
                trail,
                twinkle,
                shapes,
                result,
                ..
            } => {
                let mut shape = FireworkShape::SmallBall;
                let (mut has_twinkle, mut has_trail) = (false, false);
                let mut colors = Vec::new();
                for s in &stacks {
                    if let Some((found, _)) = shapes.iter().find(|(_, ing)| ing.test(s)) {
                        shape = *found;
                    } else if twinkle.test(s) {
                        has_twinkle = true;
                    } else if trail.test(s) {
                        has_trail = true;
                    } else if dye.test(s) {
                        colors.push(DYE_FIREWORK_COLORS[ctx.dye(s).unwrap_or(0)].1);
                    }
                }
                let mut out = result.create();
                out.data_mut().firework_explosion = Some(FireworkExplosion {
                    shape,
                    colors,
                    fade: Vec::new(),
                    trail: has_trail,
                    twinkle: has_twinkle,
                });
                out
            }
            Self::FireworkStarFade {
                dye,
                target,
                result,
            } => {
                let mut colors = Vec::new();
                let mut target_stack = None;
                for s in &stacks {
                    if dye.test(s) {
                        colors.push(DYE_FIREWORK_COLORS[ctx.dye(s).unwrap_or(0)].1);
                    } else if target.test(s) {
                        target_stack = Some(*s);
                    }
                }
                let target_stack = target_stack?;
                if colors.is_empty() {
                    return None;
                }
                let mut out = result.over(target_stack, 0);
                let explosion = out
                    .data_mut()
                    .firework_explosion
                    .get_or_insert_with(FireworkExplosion::default);
                explosion.fade = colors;
                out
            }
            Self::MapExtending { result, .. } => {
                let map = stacks
                    .iter()
                    .find(|s| s.data().is_some_and(|d| d.map_id.is_some()))?;
                let mut out = result.over(map, 0);
                out.data_mut().map_post_processing = Some(1);
                out
            }
            Self::DecoratedPot { result, .. } => {
                let side = |x, y| {
                    input.get(x, y).map(|s| ItemStack {
                        count: 1,
                        ..s.clone()
                    })
                };
                let mut out = result.create();
                let sides = [side(1, 0), side(0, 1), side(2, 1), side(1, 2)];
                if !crate::item_data::all_bricks(&sides) {
                    out.data_mut().pot_decorations = Some(Box::new(sides));
                }
                out
            }
            Self::RepairItem => {
                let (first, second) = Self::repair_pair(ctx, &stacks)?;
                let max = |s: &ItemStack| ctx.facts(s).map_or(0, |f| f.max_damage) as i64;
                let durability = max(first).max(max(second));
                let remaining = (max(first) - i64::from(first.damage))
                    + (max(second) - i64::from(second.damage))
                    + durability * 5 / 100;
                let mut out = ItemStack::new(&first.item, 1);
                out.damage = (durability - remaining).max(0) as u32;
                // Only the curses of both survive, at the higher level.
                let (a, b) = (
                    first.crafting_enchantments(),
                    second.crafting_enchantments(),
                );
                let mut curses: Vec<(Arc<str>, u32)> = Vec::new();
                for (id, level) in a.iter().chain(b.iter()) {
                    if !ctx.recipes.curses.contains(&**id) {
                        continue;
                    }
                    match curses.iter_mut().find(|(c, _)| c == id) {
                        Some((_, l)) => *l = (*l).max(*level),
                        None => curses.push((id.clone(), *level)),
                    }
                }
                out.set_crafting_enchantments(curses);
                out
            }
        };
        let mut made = made;
        made.tidy();
        // ItemStackTemplate.validate: oversized stacks become empty.
        (made.count > 0 && made.count <= ctx.max_stack(&made.item)).then_some(made)
    }
}
/// `DyedItemColor.applyDyes` over dye indexes.
pub(crate) fn apply_dyes(current: Option<i32>, dyes: &[usize]) -> i32 {
    let (mut r, mut g, mut b, mut intensity, mut n) = (0i32, 0i32, 0i32, 0i32, 0i32);
    let mut add = |rgb: u32| {
        let (red, green, blue) = (
            (rgb >> 16 & 255) as i32,
            (rgb >> 8 & 255) as i32,
            (rgb & 255) as i32,
        );
        intensity += red.max(green.max(blue));
        r += red;
        g += green;
        b += blue;
        n += 1;
    };
    if let Some(c) = current {
        add(c as u32);
    }
    for d in dyes {
        add(crate::block_entity_render::DYES[*d].1);
    }
    let (red, green, blue) = (r / n, g / n, b / n);
    let average = intensity as f32 / n as f32;
    let result = red.max(green.max(blue)) as f32;
    let scale = |c: i32| (c as f32 * average / result) as i32;
    scale(red) << 16 | scale(green) << 8 | scale(blue)
}
#[derive(Clone, Debug)]
pub struct Cooking {
    /// The recipe's file name (its id without namespace).
    pub id: String,
    /// `category` (`food`, `blocks`, `misc`) and `group` of the recipe book.
    pub category: String,
    pub group: String,
    pub kind: String,
    pub ingredient: Ingredient,
    pub result: ItemStack,
    pub experience: f32,
    pub time: u32,
}
pub struct Recipes {
    pub crafting: Vec<(String, Recipe)>,
    pub cooking: Vec<Cooking>,
    /// `category` and `group` of each crafting recipe, by file name.
    pub meta: HashMap<String, (String, String)>,
    facts: HashMap<Arc<str>, Facts>,
    /// The `#minecraft:curse` enchantments.
    curses: HashSet<String>,
    /// The first cooking recipe of each kind taking each item, built at the
    /// first lookup (furnaces look theirs up every tick).
    cook_index: std::sync::OnceLock<CookIndex>,
}
type CookIndex = HashMap<String, crate::fast_hash::FxHashMap<String, usize>>;
/// `MinMaxBounds.Ints` as (min, max) with the given defaults.
fn bounds(v: Option<&Value>, default: (i64, i64), open: (i64, i64)) -> (i64, i64) {
    match v {
        Some(Value::Number(n)) => {
            let n = n.as_i64().unwrap_or(default.0);
            (n, n)
        }
        Some(o @ Value::Object(_)) => (
            o["min"].as_i64().unwrap_or(open.0),
            o["max"].as_i64().unwrap_or(open.1),
        ),
        _ => default,
    }
}
/// `Util.isSymmetrical` is not needed: both orientations are tried.
/// `ShapedRecipePattern.shrink`: the pattern without empty border rows and columns.
fn shrink(pattern: &[Vec<char>]) -> Vec<Vec<char>> {
    let rows: Vec<&Vec<char>> = pattern
        .iter()
        .skip_while(|r| r.iter().all(|c| *c == ' '))
        .collect();
    let mut rows: Vec<Vec<char>> = rows.into_iter().cloned().collect();
    while rows.last().is_some_and(|r| r.iter().all(|c| *c == ' ')) {
        rows.pop();
    }
    let left = rows
        .iter()
        .filter_map(|r| r.iter().position(|c| *c != ' '))
        .min();
    let right = rows
        .iter()
        .filter_map(|r| r.iter().rposition(|c| *c != ' '))
        .max();
    match (left, right) {
        (Some(l), Some(r)) => rows.iter().map(|row| row[l..=r].to_vec()).collect(),
        _ => Vec::new(),
    }
}
/// A perfect matching of the ingredients to distinct stacks (`StackedContents.canCraft`).
fn matches_all(ingredients: &[Ingredient], stacks: &[&ItemStack]) -> bool {
    fn augment(
        i: usize,
        ingredients: &[Ingredient],
        stacks: &[&ItemStack],
        owner: &mut [Option<usize>],
        seen: &mut [bool],
    ) -> bool {
        for s in 0..stacks.len() {
            if seen[s] || !ingredients[i].test(stacks[s]) {
                continue;
            }
            seen[s] = true;
            if owner[s].is_none_or(|o| augment(o, ingredients, stacks, owner, seen)) {
                owner[s] = Some(i);
                return true;
            }
        }
        false
    }
    if ingredients.len() != stacks.len() {
        return false;
    }
    let mut owner = vec![None; stacks.len()];
    (0..ingredients.len()).all(|i| {
        let mut seen = vec![false; stacks.len()];
        augment(i, ingredients, stacks, &mut owner, &mut seen)
    })
}
impl Recipes {
    pub fn load(root: &Path, items: &Items, overlay: &PackOverlay) -> Result<Self> {
        let mut crafting = Vec::new();
        let mut cooking = Vec::new();
        let mut meta = HashMap::new();
        let json_files = |dir: &Path| -> Result<Vec<PathBuf>> {
            let mut files: Vec<_> = fs::read_dir(dir)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect();
            files.sort();
            Ok(files)
        };
        let stem = |p: &Path| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned()
        };
        // Vanilla's recipes by bare name, then content mods' as `<ns>:<name>`.
        let mut sources: Vec<(String, Vec<u8>)> = Vec::new();
        for p in json_files(&root.join("data/minecraft/recipe"))? {
            sources.push((stem(&p), fs::read(&p)?));
        }
        for namespace in crate::mod_content::namespaces(root) {
            let dir = root.join(format!("data/{namespace}/recipe"));
            if dir.is_dir() {
                for p in json_files(&dir)? {
                    sources.push((format!("{namespace}:{}", stem(&p)), fs::read(&p)?));
                }
            }
        }
        // The world's packs' recipes over the game's own: the top pack's
        // file for each id replaces it (`SimpleJsonResourceReloadListener`
        // sees one resource per id, the resource stack's top).
        for (id, files) in overlay.kind("recipe") {
            let Some((ns, path)) = id.split_once(':') else {
                continue;
            };
            let name = if ns == "minecraft" {
                path.to_owned()
            } else {
                id.to_owned()
            };
            let bytes = PackOverlay::top(files).to_vec();
            match sources.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = bytes,
                None => sources.push((name, bytes)),
            }
        }
        for (name, bytes) in sources {
            // Vanilla's RecipeManager logs a file that does not parse and
            // goes on ("Couldn't parse data file").
            let v: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("Horizon: recipe {name} not loaded: {e}");
                    continue;
                }
            };
            let text = |k: &str| v[k].as_str().unwrap_or("").to_owned();
            meta.insert(name.clone(), (text("category"), text("group")));
            let ing = |k: &str| Ingredient::parse(&v[k], items);
            let result = || Template::parse(&v["result"]);
            let special = |recipe: Option<Special>| recipe.map(Recipe::Special);
            let parsed: Option<Recipe> = match v["type"].as_str().unwrap_or("") {
                "minecraft:crafting_shaped" => {
                    let pattern: Vec<Vec<char>> = v["pattern"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|r| r.as_str().map(|s| s.chars().collect()))
                        .collect();
                    let pattern = shrink(&pattern);
                    let height = pattern.len();
                    let width = pattern.first().map_or(0, Vec::len);
                    let mut cells = Vec::with_capacity(width * height);
                    let mut ok = height > 0;
                    for row in &pattern {
                        for c in row {
                            if *c == ' ' {
                                cells.push(None);
                            } else {
                                match Ingredient::parse(&v["key"][c.to_string()], items) {
                                    Some(i) => cells.push(Some(i)),
                                    None => ok = false,
                                }
                            }
                        }
                    }
                    match (ok, result()) {
                        (true, Some(result)) => Some(Recipe::Shaped {
                            width,
                            height,
                            cells,
                            result,
                        }),
                        _ => None,
                    }
                }
                "minecraft:crafting_shapeless" => {
                    let ingredients: Option<Vec<_>> = v["ingredients"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|i| Ingredient::parse(i, items))
                        .collect();
                    match (ingredients, result()) {
                        (Some(ingredients), Some(result)) => Some(Recipe::Shapeless {
                            ingredients,
                            result,
                        }),
                        _ => None,
                    }
                }
                "minecraft:crafting_special_bannerduplicate" => special(
                    ing("banner")
                        .zip(v["result"]["id"].as_str())
                        .map(|(banner, id)| Special::BannerDuplicate {
                            banner,
                            result: id.to_owned(),
                        }),
                ),
                "minecraft:crafting_special_shielddecoration" => special(
                    ing("banner")
                        .zip(ing("target"))
                        .zip(v["result"]["id"].as_str())
                        .map(|((banner, target), id)| Special::ShieldDecoration {
                            banner,
                            target,
                            result: id.to_owned(),
                        }),
                ),
                "minecraft:crafting_transmute" => special((|| {
                    let (min, max) = bounds(v.get("material_count"), (1, 1), (1, 8));
                    Some(Special::Transmute {
                        input: ing("input")?,
                        material: ing("material")?,
                        min: min.clamp(1, 8) as usize,
                        max: max.clamp(1, 8) as usize,
                        add_count: v["add_material_count_to_result"].as_bool().unwrap_or(false),
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_dye" => special((|| {
                    Some(Special::Dye {
                        target: ing("target")?,
                        dye: ing("dye")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_imbue" => special((|| {
                    Some(Special::Imbue {
                        source: ing("source")?,
                        material: ing("material")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_bookcloning" => special((|| {
                    let (min, max) = bounds(v.get("allowed_generations"), (0, 1), (0, 2));
                    Some(Special::BookCloning {
                        source: ing("source")?,
                        material: ing("material")?,
                        min: min as i32,
                        max: max as i32,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_firework_rocket" => special((|| {
                    Some(Special::FireworkRocket {
                        shell: ing("shell")?,
                        fuel: ing("fuel")?,
                        star: ing("star")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_firework_star" => special((|| {
                    let mut shapes = Vec::new();
                    for (shape, value) in v["shapes"].as_object()? {
                        shapes.push((
                            FireworkShape::from_name(shape),
                            Ingredient::parse(value, items)?,
                        ));
                    }
                    Some(Special::FireworkStar {
                        dye: ing("dye")?,
                        fuel: ing("fuel")?,
                        trail: ing("trail")?,
                        twinkle: ing("twinkle")?,
                        shapes,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_firework_star_fade" => special((|| {
                    Some(Special::FireworkStarFade {
                        dye: ing("dye")?,
                        target: ing("target")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_mapextending" => special((|| {
                    Some(Special::MapExtending {
                        map: ing("map")?,
                        material: ing("material")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_decorated_pot" => special((|| {
                    Some(Special::DecoratedPot {
                        back: ing("back")?,
                        left: ing("left")?,
                        right: ing("right")?,
                        front: ing("front")?,
                        result: result()?,
                    })
                })()),
                "minecraft:crafting_special_repairitem" => special(Some(Special::RepairItem)),
                kind @ ("minecraft:smelting"
                | "minecraft:blasting"
                | "minecraft:smoking"
                | "minecraft:campfire_cooking") => {
                    if let (Some(ingredient), Some(result)) = (ing("ingredient"), result()) {
                        cooking.push(Cooking {
                            id: name.clone(),
                            category: text("category"),
                            group: text("group"),
                            kind: kind.to_owned(),
                            ingredient,
                            result: result.create(),
                            experience: v["experience"].as_f64().unwrap_or(0.0) as f32,
                            time: v["cookingtime"].as_u64().unwrap_or(200) as u32,
                        });
                    }
                    None
                }
                _ => None,
            };
            if let Some(recipe) = parsed {
                crafting.push((name, recipe));
            }
        }
        let facts = items
            .items
            .iter()
            .map(|(id, info)| {
                (
                    id.clone(),
                    Facts {
                        dye: info.dye.as_deref().and_then(|d| {
                            crate::block_entity_render::DYES
                                .iter()
                                .position(|(name, _)| *name == d)
                        }),
                        max_damage: info.max_damage,
                        max_stack: info.max_stack,
                        remainder: info.crafting_remainder.as_deref().map(Arc::from),
                    },
                )
            })
            .collect();
        let curses = crate::items::load_tags(root, "enchantment", overlay)?
            .remove("minecraft:curse")
            .unwrap_or_default();
        Ok(Self {
            crafting,
            cooking,
            meta,
            facts,
            curses,
            cook_index: Default::default(),
        })
    }
    /// Match a `size`x`size` grid (row-major) against the crafting recipes.
    pub fn craft(&self, grid: &[Option<ItemStack>], size: usize) -> Option<ItemStack> {
        self.find(grid, size).map(|(_, result)| result)
    }
    /// `craft` with the matched recipe's id (the file name without namespace).
    pub fn find(&self, grid: &[Option<ItemStack>], size: usize) -> Option<(&str, ItemStack)> {
        self.find_with(grid, size, &|_| None)
    }
    /// `RecipeManager.getRecipeFor(CRAFTING, ...)` with a lookup for the scale
    /// of a filled map (map extending reads the map's saved data).
    pub fn find_with(
        &self,
        grid: &[Option<ItemStack>],
        size: usize,
        map_scale: &dyn Fn(i32) -> Option<u8>,
    ) -> Option<(&str, ItemStack)> {
        let input = Input::of(grid, size)?;
        let ctx = Context {
            recipes: self,
            map_scale,
        };
        for (name, recipe) in &self.crafting {
            if let Some(result) = Self::assemble_if_matches(&ctx, &input, recipe) {
                return result.map(|r| (name.as_str(), r));
            }
        }
        None
    }
    /// `Recipe.matches` then `assemble` for one recipe: None when it does
    /// not match, Some(None) when it matches with nothing to make.
    fn assemble_if_matches(
        ctx: &Context,
        input: &Input,
        recipe: &Recipe,
    ) -> Option<Option<ItemStack>> {
        match recipe {
            Recipe::Special(special) => special
                .matches(ctx, input)
                .then(|| special.assemble(ctx, input)),
            Recipe::Shaped {
                width,
                height,
                cells,
                result,
            } => {
                if *width != input.width
                    || *height != input.height
                    || cells.iter().flatten().count() != input.count()
                {
                    return None;
                }
                let matches = |mirror: bool| {
                    (0..*height).all(|y| {
                        (0..*width).all(|x| {
                            let rx = if mirror { width - 1 - x } else { x };
                            match (&cells[y * width + rx], input.get(x, y)) {
                                (None, None) => true,
                                (Some(i), Some(s)) => i.test(s),
                                _ => false,
                            }
                        })
                    })
                };
                (matches(false) || matches(true)).then(|| Some(result.create()))
            }
            Recipe::Shapeless {
                ingredients,
                result,
            } => matches_all(ingredients, &input.stacks()).then(|| Some(result.create())),
        }
    }
    /// `recipe.matches(craftSlots.asCraftInput())` for one recipe by id
    /// (`minecraft:name`).
    pub fn recipe_matches(&self, id: &str, grid: &[Option<ItemStack>], size: usize) -> bool {
        let name = id.trim_start_matches("minecraft:");
        let Some((_, recipe)) = self.crafting.iter().find(|(n, _)| n == name) else {
            return false;
        };
        let Some(input) = Input::of(grid, size) else {
            return false;
        };
        let ctx = Context {
            recipes: self,
            map_scale: &|_| None,
        };
        Self::assemble_if_matches(&ctx, &input, recipe).is_some()
    }
    /// `Recipe.placementInfo` of a crafting or cooking recipe by id; None for
    /// an unknown id, an empty placement when it is impossible to place.
    pub fn placement(&self, id: &str) -> Option<Placement> {
        let name = id.trim_start_matches("minecraft:");
        if let Some(c) = self.cooking.iter().find(|c| c.id == name) {
            return Some(Placement::of(vec![Some(c.ingredient.clone())], None));
        }
        let (_, recipe) = self.crafting.iter().find(|(n, _)| n == name)?;
        Some(match recipe {
            Recipe::Shaped {
                width,
                height,
                cells,
                ..
            } => Placement::of(cells.clone(), Some((*width, *height))),
            Recipe::Shapeless { ingredients, .. } => {
                Placement::of(ingredients.iter().cloned().map(Some).collect(), None)
            }
            Recipe::Special(Special::Transmute {
                input,
                material,
                max,
                ..
            }) => {
                let mut list = vec![Some(input.clone())];
                list.extend(std::iter::repeat_n(Some(material.clone()), *max));
                Placement::of(list, None)
            }
            Recipe::Special(Special::Dye { target, dye, .. }) => {
                Placement::of(vec![Some(target.clone()), Some(dye.clone())], None)
            }
            Recipe::Special(Special::Imbue {
                source, material, ..
            }) => Placement::of(
                (0..9)
                    .map(|i| Some(if i == 4 { source } else { material }.clone()))
                    .collect(),
                None,
            ),
            // CustomRecipe.placementInfo: NOT_PLACEABLE.
            Recipe::Special(_) => Placement::default(),
        })
    }
    /// `Recipe.getRemainingItems` placed on the `size`x`size` grid: an item's
    /// crafting remainder (buckets, bottles), the patterned banner a duplicate
    /// keeps and the written book a clone keeps.
    pub fn remainders(
        &self,
        recipe: &str,
        grid: &[Option<ItemStack>],
        size: usize,
    ) -> Vec<Option<ItemStack>> {
        let mut out = vec![None; size * size];
        let Some(input) = Input::of(grid, size) else {
            return out;
        };
        let kind = self
            .crafting
            .iter()
            .find(|(n, _)| n == recipe)
            .map(|(_, r)| r);
        for (i, cell) in input.cells.iter().enumerate() {
            let Some(s) = cell else { continue };
            let at = (input.top + i / input.width) * size + input.left + i % input.width;
            let rest = self
                .facts
                .get(&s.item)
                .and_then(|f| f.remainder.as_deref())
                .map(|r| ItemStack::new(r, 1));
            if rest.is_some() {
                out[at] = rest;
                continue;
            }
            let copy = ItemStack {
                count: 1,
                ..(*s).clone()
            };
            match kind {
                Some(Recipe::Special(Special::BannerDuplicate { .. }))
                    if !crate::banner::layers(s).is_empty() =>
                {
                    out[at] = Some(copy);
                }
                // BookCloningRecipe stops at the first copied book.
                Some(Recipe::Special(Special::BookCloning { .. }))
                    if s.data().is_some_and(|d| d.written.is_some()) =>
                {
                    out[at] = Some(copy);
                    break;
                }
                _ => {}
            }
        }
        out
    }
    /// The `experience` of the cooking recipe with this file name, and 0 for one this data
    /// pack does not have (a save from another pack can name it).
    /// `id` with or without the `minecraft:` namespace (a furnace keeps the full id).
    pub fn cooking_experience(&self, id: &str) -> f32 {
        let id = id.strip_prefix("minecraft:").unwrap_or(id);
        self.cooking
            .iter()
            .find(|c| c.id == id)
            .map_or(0.0, |c| c.experience)
    }
    pub fn cook(&self, kind: &str, input: &ItemStack) -> Option<&Cooking> {
        let index = self.cook_index.get_or_init(|| {
            let mut index = CookIndex::new();
            for (i, c) in self.cooking.iter().enumerate() {
                let of_kind = index.entry(c.kind.clone()).or_default();
                for item in &c.ingredient.0 {
                    of_kind.entry(item.clone()).or_insert(i);
                }
            }
            index
        });
        let i = *index.get(kind)?.get(&*input.item)?;
        self.cooking.get(i)
    }
    /// `cook` by scanning every recipe, as it was before the index.
    #[cfg(test)]
    fn cook_scan(&self, kind: &str, input: &ItemStack) -> Option<&Cooking> {
        self.cooking
            .iter()
            .find(|c| c.kind == kind && c.ingredient.test(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::banner::BannerLayer;

    #[test]
    fn result_count_is_bounded_like_vanilla() {
        let parse = |s: &str| Template::parse(&serde_json::from_str(s).unwrap());
        assert_eq!(parse(r#"{"id":"minecraft:stone"}"#).unwrap().count, 1);
        let top = parse(r#"{"id":"minecraft:stone","count":99}"#);
        assert_eq!(top.unwrap().count, 99);
        for bad in [
            "0",
            "100",
            "2147483648",
            "4294967295",
            "4294967296",
            "-1",
            "1.5",
            "\"2\"",
        ] {
            let json = format!(r#"{{"id":"minecraft:stone","count":{bad}}}"#);
            assert!(parse(&json).is_none(), "count {bad} must be rejected");
        }
    }

    fn ingredient(items: &[&str]) -> Ingredient {
        Ingredient::of(items.iter().map(|s| (*s).to_owned()).collect())
    }
    fn patterned(item: &str) -> ItemStack {
        let mut s = ItemStack::new(item, 1);
        s.data_mut().banner_patterns = vec![BannerLayer::new("minecraft:cross", "red")];
        s
    }
    /// About vanilla's count of cooking recipes: four kinds, ingredients of
    /// one to twelve items, overlapping (several recipes take some items).
    fn cooking_book() -> Recipes {
        let kinds = ["smelting", "blasting", "smoking", "campfire_cooking"];
        let cooking = (0..320usize)
            .map(|i| Cooking {
                id: format!("r{i}"),
                category: "misc".into(),
                group: String::new(),
                kind: kinds[i % 4].into(),
                ingredient: ingredient(
                    &(0..1 + i % 12)
                        .map(|k| format!("minecraft:item_{}", (i * 7 + k * 13) % 400))
                        .collect::<Vec<_>>()
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                ),
                result: ItemStack::new("minecraft:stone", 1),
                experience: 0.1,
                time: 200,
            })
            .collect();
        Recipes {
            crafting: Vec::new(),
            cooking,
            meta: HashMap::new(),
            facts: HashMap::new(),
            curses: HashSet::new(),
            cook_index: Default::default(),
        }
    }
    #[test]
    fn cook_finds_the_first_recipe_the_scan_finds() {
        let recipes = cooking_book();
        for kind in [
            "smelting",
            "blasting",
            "smoking",
            "campfire_cooking",
            "other",
        ] {
            for n in 0..420 {
                let stack = ItemStack::new(&format!("minecraft:item_{n}"), 1);
                let a = recipes.cook(kind, &stack).map(|c| &c.id);
                let b = recipes.cook_scan(kind, &stack).map(|c| &c.id);
                assert_eq!(a, b, "{kind} item_{n}");
            }
        }
    }
    /// A furnace's recipe lookup, once per lit furnace per tick.
    #[test]
    #[ignore]
    fn cook_cost() {
        let recipes = cooking_book();
        let stacks: Vec<ItemStack> = (0..1000)
            .map(|n| ItemStack::new(&format!("minecraft:item_{}", n % 400), 1))
            .collect();
        let t = std::time::Instant::now();
        let a = stacks
            .iter()
            .filter(|s| recipes.cook_scan("smelting", s).cloned().is_some())
            .count();
        let before = t.elapsed().as_secs_f64() * 1e3;
        recipes.cook("smelting", &stacks[0]);
        let t = std::time::Instant::now();
        let b = stacks
            .iter()
            .filter(|s| recipes.cook("smelting", s).is_some())
            .count();
        let after = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(a, b);
        println!("1000 furnace lookups (scan and clone -> index): {before:.3} ms -> {after:.3} ms");
    }
    fn run(special: &Special, cells: &[ItemStack]) -> Option<ItemStack> {
        let recipes = Recipes {
            crafting: Vec::new(),
            cooking: Vec::new(),
            meta: HashMap::new(),
            facts: HashMap::new(),
            curses: HashSet::new(),
            cook_index: Default::default(),
        };
        let ctx = Context {
            recipes: &recipes,
            map_scale: &|_| None,
        };
        let mut grid: Vec<Option<ItemStack>> = cells.iter().cloned().map(Some).collect();
        grid.resize(9, None);
        let input = Input::of(&grid, 3)?;
        special
            .matches(&ctx, &input)
            .then(|| special.assemble(&ctx, &input))
            .flatten()
    }
    #[test]
    fn banner_duplicate_copies_the_patterns_of_the_same_colour() {
        let recipe = Special::BannerDuplicate {
            banner: ingredient(&["minecraft:white_banner"]),
            result: "minecraft:white_banner".into(),
        };
        let source = patterned("minecraft:white_banner");
        let blank = ItemStack::new("minecraft:white_banner", 1);
        let out = run(&recipe, &[source.clone(), blank.clone()]).expect("copy");
        assert_eq!(out.data().unwrap().banner_patterns.len(), 1);
        // Two blanks, or two patterned banners, do not match.
        assert!(run(&recipe, &[blank.clone(), blank.clone()]).is_none());
        assert!(run(&recipe, &[source.clone(), source.clone()]).is_none());
        // Another colour is not in the recipe's ingredient.
        let other = ItemStack::new("minecraft:red_banner", 1);
        assert!(run(&recipe, &[source, other]).is_none());
    }
    #[test]
    fn shield_decoration_needs_a_bare_shield() {
        let recipe = Special::ShieldDecoration {
            banner: ingredient(&["minecraft:red_banner"]),
            target: ingredient(&["minecraft:shield"]),
            result: "minecraft:shield".into(),
        };
        let banner = patterned("minecraft:red_banner");
        let shield = ItemStack::new("minecraft:shield", 1);
        let out = run(&recipe, &[banner.clone(), shield]).expect("decorated");
        let data = out.data().unwrap();
        assert_eq!(data.base_color.as_deref(), Some("red"));
        assert_eq!(data.banner_patterns.len(), 1);
        // A shield that already has patterns cannot be decorated again.
        assert!(run(&recipe, &[banner, out]).is_none());
    }
    #[test]
    fn shapeless_matching_assigns_each_stack_one_ingredient() {
        let any = ingredient(&["a", "b"]);
        let only_a = ingredient(&["a"]);
        let (a, b) = (ItemStack::new("a", 1), ItemStack::new("b", 1));
        // Greedy matching would give `a` to the first ingredient and fail.
        assert!(matches_all(&[any.clone(), only_a.clone()], &[&a, &b]));
        assert!(!matches_all(&[only_a.clone(), only_a], &[&a, &b]));
        assert!(!matches_all(&[any], &[&a, &b]));
    }
    #[test]
    fn dyes_blend_like_dyed_item_color() {
        // One red dye on an undyed item gives the dye's own colour.
        assert_eq!(apply_dyes(None, &[14]), 0xB02E26);
    }
}
