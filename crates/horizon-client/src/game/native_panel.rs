//! The item panel beside container screens: the items a recipe viewer mod
//! lists (a native mod's `native_panel_items`, or WIT `interface browser`
//! of a WebAssembly mod, docs/mods.md), drawn as the game's own item slots
//! in pages right of the screen's window, with a search box under them. The
//! mod says what the panel holds; the game pages it and tells the mod what
//! the player clicked, typed, and which keys went down over an item
//! (`native_panel_event`). With no mod showing a panel, each hook here
//! returns after one check.
use super::{
    Game, Screen,
    native_hooks::{NativePanelEvent as BrowserEvent, NativeStack as BrowserStack},
    native_panel_recipes::RecipePage,
};
use horizon_client::{
    hud::{HudBatch, rgba},
    items::ItemStack,
    ui::EditBox,
};

/// One slot of the panel, in GUI pixels.
const SLOT: f32 = 18.0;
/// The most columns the panel takes, and the fewest it shows with.
const COLUMNS: (usize, usize) = (12, 2);
/// The gap between the window and the panel, and the screen's edges.
const MARGIN: f32 = 8.0;

/// Items a panel may hold.
const ITEMS: usize = 8192;

/// Whose panel it is: a native mod's id.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Owner {
    Native(String),
}
impl Owner {
    fn native_id(&self) -> &str {
        let Owner::Native(id) = self;
        id
    }
}

/// What a mod shows on the panel: its items, and the recipes shown in
/// their place.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Shown {
    pub(super) owner: Owner,
    /// Which change the panel is from, so the game converts it once.
    pub(super) version: u64,
    pub(super) items: Vec<BrowserStack>,
    pub(super) recipes: Option<RecipePage>,
}

/// The panel's state in the game.
#[derive(Default)]
pub(super) struct Panel {
    /// A native mod's panel: the last one to show something.
    native: Option<Shown>,
    /// The last native change.
    versions: u64,
    /// The items' owner and version they were made from.
    of: Option<(Owner, u64)>,
    /// The mod's items as stacks; `None` for an item the game doesn't have.
    stacks: Vec<Option<ItemStack>>,
    page: usize,
    search: Option<EditBox>,
    /// The search text the mod last heard.
    sent: String,
    /// The recipes shown instead of the items.
    pub(super) recipes: Option<super::native_panel_recipes::Pages>,
}

impl Panel {
    /// What the panel shows: a native mod's, one `on`.
    pub(super) fn shown(&self, on: impl Fn(&str) -> bool) -> Option<&Shown> {
        self.native.as_ref().filter(|s| on(s.owner.native_id()))
    }
    /// `native_panel_items`.
    pub(super) fn show_items(&mut self, mod_id: &str, mut items: Vec<BrowserStack>) {
        items.truncate(ITEMS);
        if let Some(shown) = self.native_of(mod_id, !items.is_empty()) {
            shown.items = items;
            self.native_changed();
        }
    }
    /// `native_panel_recipes`.
    pub(super) fn show_recipes(&mut self, mod_id: &str, page: Option<RecipePage>) {
        if let Some(shown) = self.native_of(mod_id, page.is_some()) {
            shown.recipes = page;
            self.native_changed();
        }
    }
    /// Mod `mod_id`'s panel, made when it shows something on another's;
    /// none when it clears a panel that isn't its own.
    fn native_of(&mut self, mod_id: &str, showing: bool) -> Option<&mut Shown> {
        let own = self
            .native
            .as_ref()
            .is_some_and(|s| s.owner.native_id() == mod_id);
        if !own {
            if !showing {
                return None;
            }
            self.native = Some(Shown {
                owner: Owner::Native(mod_id.to_owned()),
                version: 0,
                items: Vec::new(),
                recipes: None,
            });
        }
        self.native.as_mut()
    }
    /// A native panel changed: a new version, and no panel once it shows
    /// nothing.
    fn native_changed(&mut self) {
        self.versions += 1;
        if let Some(shown) = self.native.as_mut() {
            shown.version = self.versions;
            if shown.items.is_empty() && shown.recipes.is_none() {
                self.native = None;
            }
        }
    }
}

/// Where the panel's parts are this frame.
pub(super) struct Layout {
    /// The panel's whole area, for clicks that hit none of its parts.
    pub(super) area: [f32; 4],
    /// The grid's top left corner, its columns and rows.
    pub(super) grid: (f32, f32),
    pub(super) columns: usize,
    pub(super) rows: usize,
    /// The page arrows' row.
    pub(super) arrows_y: f32,
    pub(super) search: [f32; 4],
}
impl Layout {
    fn per_page(&self) -> usize {
        self.columns * self.rows
    }
    pub(super) fn width(&self) -> f32 {
        self.columns as f32 * SLOT
    }
    fn slot(&self, n: usize) -> (f32, f32) {
        (
            self.grid.0 + (n % self.columns) as f32 * SLOT,
            self.grid.1 + (n / self.columns) as f32 * SLOT,
        )
    }
    /// The page arrows: backward, then forward.
    pub(super) fn arrows(&self) -> [(f32, f32); 2] {
        [
            (self.grid.0, self.arrows_y),
            (self.grid.0 + self.width() - 12.0, self.arrows_y),
        ]
    }
}

pub(super) fn inside(p: (f32, f32), [x, y, w, h]: [f32; 4]) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}

impl Game<'_> {
    fn browser_batch(&self) -> Option<HudBatch> {
        let r = self.renderer.as_ref()?;
        Some(HudBatch::new([r.config.width, r.config.height]))
    }
    /// The mod showing a panel, with its items made into stacks once per
    /// change; none while no mod shows one or no container screen is up.
    fn browser_sync(&mut self) -> Option<Owner> {
        if self.screen != Screen::Inventory || self.title.is_some() {
            return None;
        }
        let disabled = &self.settings.mods.disabled;
        let Some(shown) = self.panel.shown(|id| !disabled.contains(id)) else {
            self.panel.of = None;
            self.panel.stacks.clear();
            return None;
        };
        let of = (shown.owner.clone(), shown.version);
        if self.panel.of.as_ref() != Some(&of) {
            let items = world_items!(self);
            let stacks = shown
                .items
                .iter()
                .map(|s| browser_stack(items, s))
                .collect();
            let shown = shown.recipes.clone();
            self.panel.stacks = stacks;
            // The same title again (the mod updating its buttons) keeps the page.
            let old = self.panel.recipes.take();
            self.panel.recipes = shown
                .as_ref()
                .map(|shown| super::native_panel_recipes::Pages::new(items, shown, old));
            self.panel.of = Some(of.clone());
        }
        Some(of.0)
    }
    /// What the panel shows now.
    fn panel_shown(&self) -> Option<&Shown> {
        let disabled = &self.settings.mods.disabled;
        self.panel.shown(|id| !disabled.contains(id))
    }
    /// The player did `event` on `owner`'s panel.
    pub(super) fn panel_event(&mut self, owner: &Owner, event: BrowserEvent) {
        let Owner::Native(id) = owner;
        self.native_panel_event(id, &event);
    }
    /// The panel's parts: right of the window, from the top of the screen to
    /// the search box at its foot. None when the screen has no room for it.
    fn browser_layout(&self, batch: &HudBatch) -> Option<Layout> {
        let image = self.container_image(batch);
        let left = (image.left + image.width) as f32 + MARGIN;
        let right = batch.width - MARGIN;
        let columns = (((right - left) / SLOT).floor().max(0.0) as usize).min(COLUMNS.0);
        if columns < COLUMNS.1 {
            return None;
        }
        let x = (left + (right - left - columns as f32 * SLOT) / 2.0).floor();
        let arrows_y = MARGIN;
        let top = arrows_y + 17.0 + 4.0;
        let search = [x, batch.height - MARGIN - 16.0, columns as f32 * SLOT, 16.0];
        let rows = ((search[1] - 4.0 - top) / SLOT).floor().max(0.0) as usize;
        (rows > 0).then_some(Layout {
            area: [left, 0.0, right - left, batch.height],
            grid: (x, top),
            columns,
            rows,
            arrows_y,
            search,
        })
    }
    fn browser_pages(&self, layout: &Layout) -> usize {
        self.panel.stacks.len().div_ceil(layout.per_page()).max(1)
    }
    /// The stack under the pointer on the panel, an item's or a recipe's,
    /// as the mod sent it and as a stack.
    fn browser_pointed_stack(&self, layout: &Layout) -> Option<(BrowserStack, ItemStack)> {
        if self.panel.recipes.is_some() {
            if let Some((_, sent, stack)) = self.recipe_tab_hovered(layout) {
                return Some((sent, stack));
            }
            let (_, _, sent, stack) = self.recipe_slot_hovered(layout)?;
            return Some((sent, stack));
        }
        let (index, stack) = self.browser_hovered(layout)?;
        let items = &self.panel_shown()?.items;
        Some((items.get(index)?.clone(), stack))
    }
    /// The panel's item under the pointer: its index and stack.
    fn browser_hovered(&self, layout: &Layout) -> Option<(usize, ItemStack)> {
        if self.panel.recipes.is_some() {
            return None;
        }
        let first = self.panel.page * layout.per_page();
        (0..layout.per_page()).find_map(|n| {
            let (x, y) = layout.slot(n);
            if !inside(self.cursor, [x, y, SLOT, SLOT]) {
                return None;
            }
            let stack = self.panel.stacks.get(first + n)?.clone()?;
            Some((first + n, stack))
        })
    }
    fn browser_search(&mut self, layout: &Layout) -> &mut EditBox {
        let search = self.panel.search.get_or_insert_with(|| {
            let mut edit = EditBox::new(0.0, 0.0, 0.0, 0.0);
            edit.max_length = 50;
            edit.focused = false;
            edit
        });
        [search.x, search.y, search.width, search.height] = layout.search;
        search
    }

    /// The items' page arrows and grid.
    fn draw_browser_items(&mut self, batch: &mut HudBatch, layout: &Layout) {
        let pages = self.browser_pages(layout);
        self.panel.page = self.panel.page.min(pages - 1);
        let page = self.panel.page;
        let cursor = self.cursor;
        if pages > 1 {
            let labels = [
                self.translate("gui.recipebook.previous_page", "Previous Page"),
                self.translate("gui.recipebook.next_page", "Next Page"),
            ];
            let shown = [page > 0, page + 1 < pages];
            let atlas = &self.gui.atlas;
            for (((x, y), name), (label, shown)) in layout
                .arrows()
                .into_iter()
                .zip(["page_backward", "page_forward"])
                .zip(labels.iter().zip(shown))
            {
                if !shown {
                    continue;
                }
                let hover = inside(cursor, [x, y, 12.0, 17.0]);
                let suffix = if hover { "_highlighted" } else { "" };
                batch.sprite(
                    atlas,
                    &format!("gui/sprites/recipe_book/{name}{suffix}"),
                    x,
                    y,
                );
                horizon_client::widgets::record(label, [x, y, 12.0, 17.0], true);
            }
            let text = format!("{}/{pages}", page + 1);
            let w = HudBatch::text_width(atlas, &text);
            let x = (layout.grid.0 + (layout.width() - w) / 2.0).floor();
            batch.text(atlas, &text, x, layout.arrows_y + 5.0, u32::MAX, true);
        }
        let first = page * layout.per_page();
        for n in 0..layout.per_page() {
            let Some(stack) = self.panel.stacks.get(first + n) else {
                break;
            };
            let Some(stack) = stack.clone() else {
                continue;
            };
            let (x, y) = layout.slot(n);
            self.draw_item(batch, &stack, x + 1.0, y + 1.0);
            let rect = [x + 1.0, y + 1.0, 16.0, 16.0];
            if inside(cursor, [x, y, SLOT, SLOT]) {
                batch.fill(
                    &self.gui.atlas,
                    x + 1.0,
                    y + 1.0,
                    16.0,
                    16.0,
                    rgba(255, 255, 255, 128),
                );
            }
            let name = self.pack.names.display_item(&stack.item);
            horizon_client::widgets::record_as(
                &name,
                rect,
                true,
                horizon_client::widgets::Kind::Button,
            );
        }
    }
    /// Draws the panel over the open container screen, then the carried
    /// stack or the tooltip again so they stay on top of it.
    pub(super) fn draw_mod_browser(&mut self, batch: &mut HudBatch) {
        if self.browser_sync().is_none() {
            return;
        }
        let Some(layout) = self.browser_layout(batch) else {
            return;
        };
        let cursor = self.cursor;
        if self.panel.recipes.is_some() {
            self.draw_browser_recipes(batch, &layout);
        } else {
            self.draw_browser_items(batch, &layout);
        }
        let hint = self.translate("gui.recipebook.search_hint", "Search...");
        let search = self.browser_search(&layout);
        // The hint shows in an empty box without focus, as the recipe book's.
        search.suggestion = (search.value.is_empty() && !search.focused).then_some(hint);
        let mut search = search.clone();
        search.draw(batch, &self.gui.atlas, None);
        self.panel.search = Some(search);
        if self.carried.is_some() {
            self.draw_carried(batch);
        } else if let Some(stack) = self.browser_pointed_stack(&layout) {
            let lines = self.stack_tooltip(&stack.1);
            self.draw_tooltip_lines(batch, &lines, cursor.0, cursor.1);
        } else {
            self.draw_slot_tooltip(batch);
        }
    }

    /// A mouse press on a container screen: true when the panel took it
    /// (its search box, page arrows, items or empty space; a click there
    /// never throws the carried stack).
    pub(super) fn mod_browser_press(&mut self, right: bool) -> bool {
        let Some(owner) = self.browser_sync() else {
            return false;
        };
        let Some(batch) = self.browser_batch() else {
            return false;
        };
        let Some(layout) = self.browser_layout(&batch) else {
            return false;
        };
        let cursor = self.cursor;
        let shift = self.has_shift_down();
        let atlas = &self.gui.atlas;
        let search = self.panel.search.get_or_insert_with(|| {
            let mut edit = EditBox::new(0.0, 0.0, 0.0, 0.0);
            edit.max_length = 50;
            edit
        });
        [search.x, search.y, search.width, search.height] = layout.search;
        if inside(cursor, layout.search) {
            search.focused = true;
            search.click(atlas, cursor.0, shift, false);
            search.reset_blink();
            return true;
        }
        search.focused = false;
        if self.panel.recipes.is_some() {
            return self.browser_recipes_press(&layout, &owner, right)
                || inside(cursor, layout.area);
        }
        let pages = self.browser_pages(&layout);
        let page = self.panel.page;
        let [back, forward] = layout.arrows();
        let step = if pages > 1 && page > 0 && inside(cursor, [back.0, back.1, 12.0, 17.0]) {
            Some(page - 1)
        } else if pages > 1
            && page + 1 < pages
            && inside(cursor, [forward.0, forward.1, 12.0, 17.0])
        {
            Some(page + 1)
        } else {
            None
        };
        if let Some(to) = step {
            self.panel.page = to;
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        if let Some((index, _)) = self.browser_hovered(&layout) {
            let event = BrowserEvent::Clicked {
                index: index as u32,
                right,
            };
            self.panel_event(&owner, event);
            return true;
        }
        inside(cursor, layout.area)
    }

    /// Whether keys go to the panel's search box.
    pub(super) fn mod_browser_typing(&self) -> bool {
        self.screen == Screen::Inventory
            && self.panel.of.is_some()
            && self.panel.search.as_ref().is_some_and(|s| s.focused)
    }
    /// A key press while the search box has focus.
    pub(super) fn mod_browser_key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::PhysicalKey;
        let shortcut = self.edit_shortcut(event);
        let ctrl = self.has_control_down();
        let shift = self.has_shift_down();
        let types_text = self.types_text();
        let Some(search) = self.panel.search.as_mut() else {
            return;
        };
        if let Some(shortcut) = shortcut {
            search.shortcut(shortcut, &mut self.clipboard);
        } else {
            let handled = match event.physical_key {
                PhysicalKey::Code(code) => search.key(code, ctrl, shift),
                _ => false,
            };
            if !handled
                && types_text
                && let Some(text) = &event.text
            {
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                search.insert_text(&text);
            }
        }
        search.reset_blink();
        self.browser_searched();
    }
    /// Text typed (or one backspace) into the search box (`horizon-ctl type`).
    pub(super) fn mod_browser_text(&mut self, text: Option<&str>, backspace: bool) {
        let Some(search) = self.panel.search.as_mut() else {
            return;
        };
        if backspace {
            search.delete_chars(-1);
        } else if let Some(text) = text {
            search.insert_text(text);
        }
        self.browser_searched();
    }
    /// The mod hears the search text when it changed.
    fn browser_searched(&mut self) {
        let (Some((owner, _)), Some(search)) = (&self.panel.of, &self.panel.search) else {
            return;
        };
        if search.value != self.panel.sent {
            let (owner, value) = (owner.clone(), search.value.clone());
            self.panel.sent.clone_from(&value);
            self.panel.page = 0;
            self.panel_event(&owner, BrowserEvent::Search(value));
        }
    }
    /// The container screen closed: the search box lets go of the keys.
    pub(super) fn mod_browser_closed(&mut self) {
        if let Some(search) = self.panel.search.as_mut() {
            search.focused = false;
        }
    }
    /// The item under the pointer on a container screen, for the mods' keys
    /// (the `Key` event): the panel's, else a slot's.
    pub(super) fn mod_browser_pointed(&mut self) -> Option<BrowserStack> {
        let batch = self.browser_batch()?;
        let stack = match self
            .browser_sync()
            .and_then(|_| self.browser_layout(&batch))
        {
            // The panel's stacks as the mod sent them.
            Some(layout) if inside(self.cursor, layout.area) => {
                return Some(self.browser_pointed_stack(&layout)?.0);
            }
            _ => {
                let slot = self.slot_at(&batch, self.cursor.0, self.cursor.1)?;
                self.slot_view(slot)?
            }
        };
        Some(BrowserStack {
            item: stack.item.to_string(),
            count: stack.count,
            components: super::native_recipes::components(&stack),
        })
    }
    /// A key `key` (`key.keyboard.r`) went down: over an item on a container
    /// screen, a native mod's panel hears it as a `Key` event.
    pub(super) fn native_panel_key(&mut self, key: &str) {
        if self.screen != Screen::Inventory || self.title.is_some() || self.mod_browser_typing() {
            return;
        }
        let Some(owner) = self.browser_sync() else {
            return;
        };
        if let Some(stack) = self.mod_browser_pointed() {
            let key = key.to_owned();
            self.panel_event(&owner, BrowserEvent::Key { key, stack });
        }
    }
}

/// A mod's item as a stack: an item the game has, its components read as
/// `/give` reads them (ones it can't read are left out).
pub(super) fn browser_stack(
    items: &horizon_client::items::Items,
    s: &BrowserStack,
) -> Option<ItemStack> {
    items.get(&s.item)?;
    let (damage, extra) = if s.components.is_empty() {
        (0, None)
    } else {
        horizon_client::item_data::command_components(&s.components).map_or((0, None), |tag| {
            horizon_client::item_data::decode_components(Some(&tag))
        })
    };
    Some(ItemStack {
        item: s.item.as_str().into(),
        count: s.count.clamp(1, 99),
        damage,
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::{BrowserStack, Owner, Panel, RecipePage};
    use crate::game::native_hooks::{NativeRecipeLayout, NativeRecipeView};

    fn stack(item: &str) -> BrowserStack {
        BrowserStack {
            item: item.into(),
            count: 1,
            components: String::new(),
        }
    }
    fn view() -> NativeRecipeView {
        NativeRecipeView {
            layout: NativeRecipeLayout::Grid,
            width: 5,
            height: 1,
            slots: vec![vec![stack("minecraft:oak_log")]; 12],
            results: vec![stack("minecraft:oak_planks")],
            note: String::new(),
            action: Some(true),
        }
    }

    /// A native mod's items and recipe page are what the panel holds, and
    /// its owner (who hears the panel's events) is that mod.
    #[test]
    fn native_owner_holds_the_panel() {
        let mut panel = Panel::default();
        let items = vec![stack("minecraft:stone"), stack("minecraft:dirt")];
        panel.show_items("recipes", items.clone());
        let shown = panel.shown(|_| true).expect("a panel");
        assert_eq!(shown.owner, Owner::Native("recipes".into()));
        assert_eq!(shown.items, items);
        assert_eq!(shown.recipes, None);
        let first = shown.version;

        let tabs = vec![stack("minecraft:crafting_table")];
        let page = RecipePage::new("Oak Planks", tabs.clone(), 0, vec![view()]);
        panel.show_recipes("recipes", page);
        let shown = panel.shown(|_| true).expect("a panel");
        assert_ne!(shown.version, first, "the game converts each change");
        assert_eq!(shown.items, items);
        let page = shown.recipes.as_ref().expect("a recipe page");
        assert_eq!(
            (page.title.as_str(), &page.tabs, page.tab),
            ("Oak Planks", &tabs, 0)
        );
        // Cut as `set-recipes` cuts it: a 3 x 3 grid at most, 9 slots.
        let r = &page.recipes[0];
        assert_eq!((r.width, r.height, r.slots.len()), (3, 1, 9));
        assert_eq!(r.results, [stack("minecraft:oak_planks")]);

        // Another mod clearing its own (empty) panel leaves this one.
        panel.show_items("other", Vec::new());
        let owner = panel.shown(|_| true).map(|s| s.owner.clone());
        assert_eq!(owner, Some(Owner::Native("recipes".into())));
        // A mod the player turned off shows nothing.
        assert!(panel.shown(|id| id != "recipes").is_none());

        // No recipes: the items again; then no items: no panel.
        panel.show_recipes("recipes", RecipePage::new("", Vec::new(), 0, Vec::new()));
        let shown = panel.shown(|_| true).expect("a panel");
        assert_eq!((shown.items.len(), shown.recipes.is_none()), (2, true));
        panel.show_items("recipes", Vec::new());
        assert!(panel.shown(|_| true).is_none());
    }
}
