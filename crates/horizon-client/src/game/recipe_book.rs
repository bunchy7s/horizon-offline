//! The recipe book of the inventory, crafting table and furnace screens
//! (`RecipeBookComponent`): the green book button, the tabs by category, the
//! search box, the craftable filter, the pages of recipe buttons with the
//! alternatives overlay, ghost recipes, and placing a recipe in the grid
//! (`ServerPlaceRecipe`). Which recipes are known comes from the advancement
//! rewards; the open and filter settings are saved with the player.
use super::{Game, Screen};
use horizon_client::{
    containers::{BlockEntity, FurnaceKind},
    hud::{HudBatch, Rect, rgba},
    items::ItemStack,
    menu::place::{PostPlace, Stacked},
    recipe_book::{Book, BookType, ClientBook, Collection, Entry, GhostTarget, Tab},
    recipe_book_screen::{
        self as screen, ButtonView, Call, Drawn, GhostView, ItemCall, OverlayRecipe, OverlayView,
        Screen as Layout, Search, TabView,
    },
    text::{Style, Text},
    ui::EditBox,
};
use std::{
    collections::BTreeSet,
    hash::{Hash, Hasher},
};

const WIDTH: f32 = 147.0;
const HEIGHT: f32 = 166.0;
/// `RecipeBookComponent.TICKS_TO_SWAP_SLOT`.
const SWAP_TICKS: f32 = 30.0;
/// Widest GUI at which the book sits next to the menu (`widthTooNarrow`).
const NARROW: f32 = 379.0;

/// A ghost recipe slot: the items it cycles through.
struct Ghost {
    slot: usize,
    items: Vec<ItemStack>,
    result: bool,
}
/// The alternatives overlay of a recipe button (`OverlayRecipeComponent`).
struct Overlay {
    x: f32,
    y: f32,
    /// (entry index, craftable)
    entries: Vec<(usize, bool)>,
}
/// One recipe button: the collection's recipes shown in the current view.
#[derive(Clone, Default)]
struct Button {
    /// (entry index, craftable)
    entries: Vec<(usize, bool)>,
}
impl Button {
    fn craftable(&self) -> bool {
        self.entries.iter().any(|e| e.1)
    }
}
#[derive(Default)]
struct View {
    /// Visible tabs.
    tabs: Vec<Tab>,
    buttons: Vec<Button>,
}

#[derive(Default)]
pub(super) struct RecipeUi {
    book: Option<Book>,
    tab: Option<Tab>,
    search: Option<EditBox>,
    page: usize,
    /// Ticks, for the ghost and button cycling (`RecipeBookComponent.time`).
    time: f32,
    ghost: Vec<Ghost>,
    last_placed: Option<String>,
    overlay: Option<Overlay>,
    /// A press landed in the search box, so a drag selects text in it.
    search_drag: bool,
    view: View,
    view_key: u64,
    /// Squeeze animations of new recipes: ticks left per button index and tab.
    button_anim: Vec<(usize, f32)>,
    tab_anim: Vec<(Tab, f32)>,
    /// The client's copy of the known recipes (`ClientRecipeBook`), its
    /// order deciding the buttons', and the recipes it was last synced with.
    client: ClientBook,
    client_known: Option<BTreeSet<String>>,
    client_version: u64,
    /// The display a click last placed (`lastRecipe`), which Enter and Space
    /// place again; slot clicks keep it.
    last_clicked: Option<i32>,
}

impl RecipeUi {
    /// A new screen starts without a ghost recipe, overlay or search focus.
    pub(super) fn reset_screen(&mut self) {
        self.ghost.clear();
        self.last_placed = None;
        self.overlay = None;
        self.tab = None;
        self.page = 0;
        if let Some(search) = self.search.as_mut() {
            search.focused = false;
            search.set_value("");
        }
        self.view_key = 0;
        self.last_clicked = None;
    }
}

fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
fn sprite(name: &str) -> String {
    format!("gui/sprites/recipe_book/{name}")
}

impl Game<'_> {
    /// The book the open screen has, if any.
    fn active_book(&self) -> Option<BookType> {
        if self.screen != Screen::Inventory || self.station.is_some() {
            return None;
        }
        // A mod's screen has no book.
        if self.mount_active() || self.native_screen_height().is_some() {
            return None;
        }
        match self.container_kind() {
            None if self.container.is_none() => Some(BookType::Crafting),
            Some(BlockEntity::Furnace(f)) => Some(match f.kind {
                FurnaceKind::Furnace => BookType::Furnace,
                FurnaceKind::BlastFurnace => BookType::BlastFurnace,
                FurnaceKind::Smoker => BookType::Smoker,
            }),
            _ => None,
        }
    }
    fn book_open(&self, book: BookType) -> bool {
        self.level.recipe_book.open[book.index()]
    }
    fn book_filtering(&self, book: BookType) -> bool {
        self.level.recipe_book.filtering[book.index()]
    }
    /// How far the open book pushes the menu to the right
    /// (`RecipeBookComponent.updateScreenPosition`).
    pub(super) fn recipe_book_shift(&self, width: f32) -> f32 {
        match self.active_book() {
            Some(book) if self.book_open(book) && width >= NARROW => 77.0,
            _ => 0.0,
        }
    }
    /// `AbstractRecipeBookScreen.isHovering` is false while the book is
    /// visible in a window narrower than `NARROW`: the book covers the menu,
    /// so no slot is hovered or clicked.
    pub(super) fn book_covers_menu(&self, width: f32) -> bool {
        width < NARROW && self.active_book().is_some_and(|b| self.book_open(b))
    }
    /// Whether typed keys go to the search box.
    pub(super) fn recipe_search_focused(&self) -> bool {
        self.active_book().is_some_and(|b| self.book_open(b))
            && self.recipe_ui.search.as_ref().is_some_and(|s| s.focused)
    }
    /// `keyChat` focuses the search box.
    pub(super) fn recipe_focus_search(&mut self) {
        if self.active_book().is_some_and(|b| self.book_open(b))
            && let Some(s) = self.recipe_ui.search.as_mut()
        {
            s.focused = true;
            s.reset_blink();
        }
    }
    /// Text typed (or one backspace) into the search box.
    pub(super) fn recipe_text(&mut self, text: Option<&str>, backspace: bool) {
        let Some(search) = self.recipe_ui.search.as_mut() else {
            return;
        };
        if backspace {
            search.delete_chars(-1);
        } else if let Some(text) = text {
            search.insert_text(text);
        }
    }
    /// Ctrl+A/C/V/X in the search box.
    pub(super) fn recipe_shortcut(&mut self, shortcut: horizon_client::clipboard::EditShortcut) {
        if let Some(search) = self.recipe_ui.search.as_mut() {
            search.shortcut(shortcut, &mut self.clipboard);
        }
    }
    /// A key press while the search box has focus.
    pub(super) fn recipe_key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::PhysicalKey;
        if let Some(shortcut) = self.edit_shortcut(event) {
            self.recipe_shortcut(shortcut);
            return;
        }
        let ctrl = self.has_control_down();
        let shift = self.has_shift_down();
        let types_text = self.types_text();
        let Some(search) = self.recipe_ui.search.as_mut() else {
            return;
        };
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
        search.reset_blink();
    }

    /// `RecipeToast.addOrUpdate` for each display of the recipes just
    /// unlocked: its crafting station and result join the toast still
    /// queued or shown, or a new one.
    pub(super) fn recipe_toast(&mut self, recipes: &[String]) {
        if recipes.is_empty() || self.adv.quiet {
            return;
        }
        self.ensure_book();
        let index = &self.stations().recipe_index;
        let book = self.recipe_ui.book.as_ref();
        // SmithingTrimDemoSlotDisplay draws from a random of its own.
        let mut random = super::dispenser::entity_random();
        let mut items = Vec::new();
        for id in recipes {
            let Some(station) = index.toast_station(id) else {
                continue;
            };
            for display in index.display_ids(id) {
                // The book's displays carry their resolved results; the
                // other recipe types name theirs, and trims are demos.
                let result = book
                    .and_then(|b| {
                        b.entry_of_display(display)
                            .map(|i| b.entries[i].result.clone())
                    })
                    .or_else(|| index.result(id).map(|r| ItemStack::new(r, 1)))
                    .or_else(|| {
                        self.stations().recipes.trim_demo(
                            &self.stations().enchantments,
                            id,
                            &mut |n| {
                                horizon_core::random::RandomSource::next_int_bound(
                                    &mut random,
                                    n as i32,
                                )
                                .unwrap_or(0) as usize
                            },
                        )
                    });
                if let Some(result) = result {
                    items.push((station.to_owned(), result));
                }
            }
        }
        if items.is_empty() {
            return;
        }
        match self.adv.toasts.iter_mut().find(|t| t.is_recipe()) {
            Some(toast) => toast.add_recipes(items),
            None => self
                .adv
                .toasts
                .push_back(super::advancements::Toast::recipe(items)),
        }
    }

    // ---- recipes and what can be crafted ----

    /// The grid size of the open crafting menu.
    fn book_grid(&self) -> (usize, usize) {
        (self.craft_size, self.craft_size)
    }
    /// The items the book counts (`RecipeBookComponent.updateStackedContents`):
    /// the inventory and the menu's crafting slots, as the menu engine counts them.
    fn book_available(&mut self, _book: BookType) -> Stacked {
        if self.remote.is_some() {
            // A server's menu: the slots it last sent, counted the same way.
            use horizon_client::menu::{Env, Kind, Menu};
            let table = self.craft_size == 3;
            let grid: Vec<_> = (0..self.craft_size * self.craft_size)
                .map(|k| self.slot_mut(100 + k).clone())
                .collect();
            let mut menu = Menu::new(if table {
                Kind::Crafting
            } else {
                Kind::Inventory
            });
            let env = Env {
                items: self.items(),
                recipes: self.recipes(),
                brewing: &self.stations().brewing,
                banners: &self.stations().banners,
                station_recipes: &self.stations().recipes,
                enchantments: &self.stations().enchantments,
                names: &self.pack.names,
                trade: None,
                maps: None,
            };
            for i in 0..36 {
                menu.set_player_item(i, self.inventory[i].clone());
            }
            for (k, stack) in grid.into_iter().enumerate() {
                menu.set_slot(&env, 1 + k, stack);
            }
            return menu.stacked_contents(&env);
        }
        let mut out = Stacked::default();
        let stock = &mut out;
        self.menu_session(|menu, env, _| {
            *stock = menu.stacked_contents(env);
            Some(false)
        });
        out
    }
    /// Sends the client book what changed in the server's: the first time a
    /// replacing batch (vanilla's server iterates an identity hash set; this
    /// one goes in id order), then each unlock and removal.
    fn sync_client_book(&mut self) {
        // On a server the book is the server's packets' (remote_recipe_book_*).
        if self.remote.is_some() {
            return;
        }
        let known = &self.level.recipe_book.recipes;
        let index = &world_stations!(self).recipe_index;
        let ui = &mut self.recipe_ui;
        match ui.client_known.as_ref() {
            Some(previous) if previous == known => return,
            Some(previous) => {
                for id in previous.difference(known) {
                    for d in index.display_ids(id) {
                        ui.client.remove(d);
                    }
                }
                for id in known.difference(previous) {
                    for d in index.display_ids(id) {
                        ui.client.add(d);
                    }
                }
            }
            None => {
                ui.client.clear();
                for id in known {
                    for d in index.display_ids(id) {
                        ui.client.add(d);
                    }
                }
            }
        }
        ui.client_known = Some(known.clone());
        ui.client_version += 1;
    }
    /// Every entry of the book, in its order (`native_recipes`).
    pub(super) fn recipe_book_entries(&mut self) -> Vec<Entry> {
        self.ensure_book();
        self.recipe_ui
            .book
            .as_ref()
            .map_or_else(Vec::new, |b| b.entries.clone())
    }
    /// The display ids the player's book holds, and a number that changes
    /// whenever they do (`native_known_recipes`).
    pub(super) fn recipe_book_displays(&mut self) -> (u64, Vec<i32>) {
        self.sync_client_book();
        (
            self.recipe_ui.client_version,
            self.recipe_ui.client.displays(),
        )
    }
    /// The book's model is made again from `Game::recipes` when next needed:
    /// the server's recipes arrived, or a new session began.
    pub(super) fn forget_book(&mut self) {
        self.recipe_ui.book = None;
    }
    fn ensure_book(&mut self) {
        if self.recipe_ui.book.is_none() {
            let mut book = Book::build(self.recipes(), self.items());
            book.set_display_ids(&self.stations().recipe_index);
            self.recipe_ui.book = Some(book);
        }
        if self.recipe_ui.search.is_none() {
            let mut search = EditBox::new(0.0, 0.0, 81.0, 14.0);
            search.message = super::screen_info::tr("itemGroup.search", &[]);
            search.max_length = 50;
            search.focused = false;
            search.text_color = u32::MAX;
            self.recipe_ui.search = Some(search);
        }
    }
    /// Rebuilds the tab and button lists when anything they depend on changed.
    fn refresh_book_view(&mut self, book_type: BookType) {
        self.ensure_book();
        self.sync_client_book();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in self.inventory.iter().flatten() {
            (&s.item, s.count, s.damage).hash(&mut h);
        }
        for slot in [
            100usize, 101, 102, 103, 104, 105, 106, 107, 108, 300, 301, 302,
        ] {
            let stack =
                if slot < 109 && self.active_book() == Some(BookType::Crafting) || slot >= 300 {
                    self.slot_mut(slot).clone()
                } else {
                    None
                };
            stack.map(|s| (s.item, s.count)).hash(&mut h);
        }
        self.craft_size.hash(&mut h);
        self.recipe_ui.client_version.hash(&mut h);
        self.book_filtering(book_type).hash(&mut h);
        self.recipe_ui.tab.map(|t| format!("{t:?}")).hash(&mut h);
        self.recipe_ui
            .search
            .as_ref()
            .map(|s| s.value.clone())
            .hash(&mut h);
        book_type.index().hash(&mut h);
        let key = h.finish();
        if key == self.recipe_ui.view_key && !self.recipe_ui.view.tabs.is_empty() {
            return;
        }
        let ui = std::mem::take(&mut self.recipe_ui);
        let view = self.build_view(&ui, book_type);
        self.recipe_ui = ui;
        self.recipe_ui.view = view;
        self.recipe_ui.view_key = key;
        let pages = self.recipe_ui.view.buttons.len().div_ceil(20);
        if self.recipe_ui.page >= pages.max(1) {
            self.recipe_ui.page = 0;
        }
        self.start_new_recipe_animations(book_type);
    }
    /// `RecipeBookTabButton.startAnimation` and `RecipeButton.init`: tabs
    /// holding a recipe still to be displayed squeeze, and the buttons of the
    /// shown page squeeze once and clear their recipes' highlight.
    fn start_new_recipe_animations(&mut self, book_type: BookType) {
        if self.level.recipe_book.to_be_displayed.is_empty() {
            return;
        }
        let Some(book) = self.recipe_ui.book.take() else {
            return;
        };
        let filtering = self.book_filtering(book_type);
        let (gw, gh) = self.book_grid();
        let available = self.book_available(book_type);
        let shown = |e: &Entry| {
            (book_type != BookType::Crafting || e.fits(gw, gh))
                && (!filtering || book.can_craft(e, &available))
        };
        let new = |level: &Self, e: &Entry| level.level.recipe_book.to_be_displayed.contains(&e.id);
        let mut tabs = Vec::new();
        for tab in self.recipe_ui.view.tabs.clone() {
            if tab == Tab::Search {
                continue;
            }
            let hit = self
                .recipe_ui
                .client
                .collections(&book, book_type, tab)
                .iter()
                .flat_map(|c| c.entries.iter())
                .any(|&i| shown(&book.entries[i]) && new(self, &book.entries[i]));
            if hit {
                tabs.push((tab, 15.0));
            }
        }
        self.recipe_ui.tab_anim = tabs;
        let first = self.recipe_ui.page * 20;
        let mut buttons = Vec::new();
        let mut cleared = Vec::new();
        for (n, button) in self
            .recipe_ui
            .view
            .buttons
            .iter()
            .skip(first)
            .take(20)
            .enumerate()
        {
            let ids: Vec<&String> = button
                .entries
                .iter()
                .map(|e| &book.entries[e.0].id)
                .filter(|id| self.level.recipe_book.to_be_displayed.contains(*id))
                .collect();
            if !ids.is_empty() {
                buttons.push((first + n, 15.0));
                cleared.extend(ids.into_iter().cloned());
            }
        }
        self.level
            .recipe_book
            .to_be_displayed
            .retain(|id| !cleared.contains(id));
        self.recipe_ui.button_anim = buttons;
        self.recipe_ui.book = Some(book);
    }
    fn build_view(&mut self, ui: &RecipeUi, book_type: BookType) -> View {
        let Some(book) = ui.book.as_ref() else {
            return View::default();
        };
        let available = self.book_available(book_type);
        let (gw, gh) = self.book_grid();
        let selected = |e: &Entry| match book_type {
            BookType::Crafting => e.fits(gw, gh),
            _ => true,
        };
        let craftable = |e: &Entry| selected(e) && book.can_craft(e, &available);
        let filtering = self.book_filtering(book_type);
        let mut tabs = Vec::new();
        for tab in book_type.tabs() {
            let visible = tab == Tab::Search
                || ui
                    .client
                    .collections(book, book_type, tab)
                    .iter()
                    .any(|c| c.entries.iter().any(|&i| selected(&book.entries[i])));
            if visible {
                tabs.push(tab);
            }
        }
        let tab = ui.tab.filter(|t| tabs.contains(t)).unwrap_or(Tab::Search);
        let query = ui
            .search
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let collections: Vec<Collection> = ui.client.collections(book, book_type, tab);
        let mut buttons = Vec::new();
        for c in collections {
            if !c.entries.iter().any(|&i| selected(&book.entries[i])) {
                continue;
            }
            if !query.is_empty()
                && !book.collection_matches(&c.entries, &query, &|stack| {
                    horizon_client::item_tooltip::lines(
                        stack,
                        self.items().get(&stack.item),
                        &|k| self.pack.names.display_get(k),
                    )
                })
            {
                continue;
            }
            let entries: Vec<(usize, bool)> = c
                .entries
                .iter()
                .filter(|&&i| selected(&book.entries[i]))
                .map(|&i| (i, craftable(&book.entries[i])))
                .collect();
            if filtering && !entries.iter().any(|e| e.1) {
                continue;
            }
            let entries = if filtering {
                entries.into_iter().filter(|e| e.1).collect()
            } else {
                entries
            };
            buttons.push(Button { entries });
        }
        View { tabs, buttons }
    }

    // ---- geometry ----

    /// `RecipeBookComponent.getXOrigin` / `getYOrigin` (`widthTooNarrow`
    /// centres the book over the menu).
    fn book_origin(&self, batch: &HudBatch) -> (f32, f32) {
        let offset = if batch.width < NARROW { 0.0 } else { 86.0 };
        (
            ((batch.width - WIDTH) / 2.0).floor() - offset,
            ((batch.height - HEIGHT) / 2.0).floor(),
        )
    }
    /// The green book button's top-left corner.
    fn book_button_pos(&self, book: BookType, ox: f32, batch: &HudBatch) -> (f32, f32) {
        let mid = (batch.height / 2.0).floor();
        match book {
            BookType::Crafting if self.craft_size == 2 => (ox + 104.0, mid - 22.0),
            BookType::Crafting => (ox + 5.0, mid - 49.0),
            _ => (ox + 20.0, mid - 49.0),
        }
    }
    fn tab_pos(&self, xo: f32, yo: f32, index: usize) -> (f32, f32) {
        (xo - 30.0, yo + 3.0 + 27.0 * index as f32)
    }
    fn button_pos(xo: f32, yo: f32, i: usize) -> (f32, f32) {
        (
            xo + 11.0 + 25.0 * (i % 5) as f32,
            yo + 31.0 + 25.0 * (i / 5) as f32,
        )
    }
    /// `RecipeBookComponent.slotSelectTime`.
    fn swap_index(&self) -> usize {
        (self.recipe_ui.time / SWAP_TICKS).floor() as usize
    }
    fn batch_for_size(&self) -> Option<HudBatch> {
        let r = self.renderer.as_ref()?;
        Some(HudBatch::new([r.config.width, r.config.height]))
    }

    // ---- clicks ----

    /// A click while a screen with a recipe book is open; true when the book
    /// used it.
    pub(super) fn recipe_book_click(&mut self, right: bool) -> bool {
        let Some(book_type) = self.active_book() else {
            return false;
        };
        let Some(batch) = self.batch_for_size() else {
            return false;
        };
        self.ensure_book();
        let cursor = self.cursor;
        let (ox, _) = self.inventory_origin(&batch);
        // Clicking a crafting slot drops the ghost recipe (`slotClicked`).
        if let Some(slot) = self.slot_at(&batch, cursor.0, cursor.1)
            && self.is_book_slot(book_type, slot)
        {
            self.recipe_ui.last_placed = None;
            self.recipe_ui.ghost.clear();
        }
        let (bx, by) = self.book_button_pos(book_type, ox, &batch);
        if inside(cursor, bx, by, 20.0, 18.0) {
            let open = !self.book_open(book_type);
            self.level.recipe_book.open[book_type.index()] = open;
            self.send_book_settings(book_type);
            if !open {
                self.recipe_ui.overlay = None;
            }
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        if !self.book_open(book_type) {
            return false;
        }
        // AbstractRecipeBookScreen.mouseClicked: in a narrow window the visible
        // book takes every click, even one it does not use itself.
        let narrow = batch.width < NARROW;
        if self.spectator() {
            return narrow;
        }
        self.refresh_book_view(book_type);
        let (xo, yo) = self.book_origin(&batch);
        // The alternatives overlay captures the click first.
        if let Some(overlay) = self.recipe_ui.overlay.take() {
            let maxrow = if overlay.entries.len() <= 16 { 4 } else { 5 };
            for (n, (entry, _)) in overlay.entries.iter().enumerate() {
                let x = overlay.x + 4.0 + 25.0 * (n % maxrow) as f32;
                let y = overlay.y + 5.0 + 25.0 * (n / maxrow) as f32;
                if !right && inside(cursor, x, y, 24.0, 24.0) {
                    let shift = self.shift_down();
                    if self.try_place(*entry, shift) {
                        self.remember_click(*entry);
                        self.close_narrow_book(book_type, &batch);
                    }
                    return true;
                }
            }
            return true;
        }
        let pages = self.recipe_ui.view.buttons.len().div_ceil(20);
        // Page arrows.
        if pages > 1 {
            if self.recipe_ui.page + 1 < pages && inside(cursor, xo + 93.0, yo + 137.0, 12.0, 17.0)
            {
                self.recipe_ui.page += 1;
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
            if self.recipe_ui.page > 0 && inside(cursor, xo + 38.0, yo + 137.0, 12.0, 17.0) {
                self.recipe_ui.page -= 1;
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
        }
        // Recipe buttons.
        let first = self.recipe_ui.page * 20;
        for n in 0..20 {
            let Some(button) = self.recipe_ui.view.buttons.get(first + n).cloned() else {
                break;
            };
            let (x, y) = Self::button_pos(xo, yo, n);
            if !inside(cursor, x, y, 25.0, 25.0) {
                continue;
            }
            if right {
                if button.entries.len() > 1 {
                    self.open_overlay(&button, x, y, xo, yo);
                }
            } else {
                let step = self.swap_index();
                let (entry, _) = button.entries[step % button.entries.len()];
                let shift = self.shift_down();
                if self.try_place(entry, shift) {
                    self.remember_click(entry);
                    self.close_narrow_book(book_type, &batch);
                }
            }
            return true;
        }
        // Search box.
        if let Some(search) = self.recipe_ui.search.as_mut() {
            search.x = xo + 25.0;
            search.y = yo + 13.0;
            // RecipeBookComponent's magnifierIconPlacement: from the book's
            // left edge plus eight to the search box's own left edge.
            let magnifier = inside(cursor, xo + 8.0, yo + 13.0, search.x - xo, 14.0);
            if magnifier || inside(cursor, search.x, search.y, search.width, search.height) {
                self.recipe_ui.search_drag = true;
                search.focused = true;
                search.reset_blink();
                if !magnifier {
                    let idx = search.index_at(&self.gui.atlas, cursor.0);
                    search.set_cursor(idx, false);
                }
                return true;
            }
            search.focused = false;
        }
        // The craftable filter.
        if inside(cursor, xo + 110.0, yo + 12.0, 26.0, 16.0) {
            let filtering = !self.book_filtering(book_type);
            self.level.recipe_book.filtering[book_type.index()] = filtering;
            self.send_book_settings(book_type);
            self.recipe_ui.page = 0;
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        // Tabs.
        for (n, tab) in self.recipe_ui.view.tabs.clone().into_iter().enumerate() {
            let (x, y) = self.tab_pos(xo, yo, n);
            if inside(cursor, x, y, 35.0, 27.0) {
                if self.recipe_ui.tab.unwrap_or(Tab::Search) != tab {
                    self.recipe_ui.tab = Some(tab);
                    self.recipe_ui.page = 0;
                }
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
        }
        narrow
    }
    /// `RecipeBookComponent.mouseClicked` after a placed recipe: it becomes
    /// `lastRecipe`.
    fn remember_click(&mut self, entry: usize) {
        self.recipe_ui.last_clicked = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry))
            .map(|e| e.display_id);
    }
    /// `RecipeBookComponent.keyPressed`: Enter or Space (`isSelection`) places
    /// the recipe clicked last again, key repeats included, as many as fit
    /// with Shift. The focused search box takes the key first.
    pub(super) fn recipe_book_key(&mut self, key: winit::keyboard::KeyCode) -> bool {
        use winit::keyboard::KeyCode;
        if !matches!(key, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
            return false;
        }
        let Some(book_type) = self.active_book() else {
            return false;
        };
        if !self.book_open(book_type) || self.spectator() || self.recipe_search_focused() {
            return false;
        }
        let Some(display) = self.recipe_ui.last_clicked else {
            return false;
        };
        // The craft just taken changed what is craftable.
        self.refresh_book_view(book_type);
        let Some(entry) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entry_of_display(display))
        else {
            return false;
        };
        self.audio
            .play("minecraft:ui.button.click", None, 0.25, 1.0);
        let shift = self.shift_down();
        self.try_place(entry, shift)
    }
    /// `RecipeBookComponent.mouseDragged`: while the search box is focused a
    /// drag moves its selection end to the pointer (`EditBox.onDrag`).
    pub(super) fn recipe_book_drag(&mut self) {
        if !self.recipe_ui.search_drag || !self.active_book().is_some_and(|b| self.book_open(b)) {
            return;
        }
        let x = self.cursor.0;
        if let Some(search) = self.recipe_ui.search.as_mut()
            && search.focused
        {
            search.drag_to(&self.gui.atlas, x);
        }
    }
    /// A mouse button let go: a drag no longer selects in the search box.
    pub(super) fn recipe_book_release(&mut self) {
        self.recipe_ui.search_drag = false;
    }
    /// `RecipeBookComponent.sendUpdateSettings`: the server owns the saved book.
    fn send_book_settings(&self, book_type: BookType) {
        self.send(
            horizon_protocol::packets::play::Serverbound::RecipeBookChangeSettings(
                horizon_protocol::packets::recipes::RecipeBookChangeSettings {
                    book_type: book_type.index() as i32,
                    open: self.book_open(book_type),
                    filtering: self.book_filtering(book_type),
                },
            ),
        );
    }
    pub(super) fn recipe_book_flags(&self) -> Option<(bool, bool)> {
        let book = self.active_book()?;
        Some((self.book_open(book), self.book_filtering(book)))
    }
    /// How many displays the client's book holds.
    pub(super) fn recipe_book_known(&self) -> usize {
        self.recipe_ui.client.displays().len()
    }
    /// The recipe book's buttons a controller can focus: the book button,
    /// recipes, page arrows, the craftable filter and the tabs (GUI pixels;
    /// the search box registers itself as a field).
    pub(super) fn recipe_book_targets(&self) -> Vec<horizon_client::widgets::Target> {
        use horizon_client::widgets::{Kind, Target};
        let (Some(book_type), Some(batch)) = (self.active_book(), self.batch_for_size()) else {
            return Vec::new();
        };
        let button = |rect| Target {
            rect,
            kind: Kind::Button,
        };
        let (ox, _) = self.inventory_origin(&batch);
        let (bx, by) = self.book_button_pos(book_type, ox, &batch);
        let mut out = vec![button([bx, by, 20.0, 18.0])];
        if !self.book_open(book_type) || self.spectator() {
            return out;
        }
        let (xo, yo) = self.book_origin(&batch);
        if let Some(overlay) = &self.recipe_ui.overlay {
            // The alternatives overlay takes every click while open.
            let maxrow = if overlay.entries.len() <= 16 { 4 } else { 5 };
            return (0..overlay.entries.len())
                .map(|n| {
                    let x = overlay.x + 4.0 + 25.0 * (n % maxrow) as f32;
                    let y = overlay.y + 5.0 + 25.0 * (n / maxrow) as f32;
                    button([x, y, 24.0, 24.0])
                })
                .collect();
        }
        let first = self.recipe_ui.page * 20;
        let shown = self
            .recipe_ui
            .view
            .buttons
            .len()
            .saturating_sub(first)
            .min(20);
        out.extend((0..shown).map(|n| {
            let (x, y) = Self::button_pos(xo, yo, n);
            button([x, y, 25.0, 25.0])
        }));
        let pages = self.recipe_ui.view.buttons.len().div_ceil(20);
        if pages > 1 && self.recipe_ui.page + 1 < pages {
            out.push(button([xo + 93.0, yo + 137.0, 12.0, 17.0]));
        }
        if pages > 1 && self.recipe_ui.page > 0 {
            out.push(button([xo + 38.0, yo + 137.0, 12.0, 17.0]));
        }
        out.push(button([xo + 110.0, yo + 12.0, 26.0, 16.0]));
        let selected = self.recipe_ui.tab.unwrap_or(Tab::Search);
        for (n, tab) in self.recipe_ui.view.tabs.iter().enumerate() {
            let (x, y) = self.tab_pos(xo, yo, n);
            out.push(Target {
                rect: [x, y, 35.0, 27.0],
                kind: Kind::Tab(*tab == selected),
            });
        }
        out
    }
    fn shift_down(&self) -> bool {
        self.has_shift_down()
    }
    /// The open screen's book for `hasClickedOutside`: (visible, selected tab
    /// hovered); None when the screen has no book.
    pub(super) fn recipe_book_area(&self, batch: &HudBatch) -> Option<(bool, bool)> {
        let book = self.active_book()?;
        if !self.book_open(book) {
            return Some((false, false));
        }
        let (xo, yo) = self.book_origin(batch);
        let tabs = &self.recipe_ui.view.tabs;
        let selected = self.recipe_ui.tab.unwrap_or(Tab::Search);
        let hovered = tabs.iter().position(|t| *t == selected).is_some_and(|n| {
            let (x, y) = self.tab_pos(xo, yo, n);
            inside(self.cursor, x, y, 35.0, 27.0)
        });
        Some((true, hovered))
    }
    /// The grid, result and furnace slots clicks on which end a ghost recipe.
    fn is_book_slot(&self, book: BookType, slot: usize) -> bool {
        match book {
            BookType::Crafting => (100..109).contains(&slot) || slot == 200,
            _ => (300..303).contains(&slot),
        }
    }
    /// `OverlayRecipeComponent.init`.
    fn open_overlay(&mut self, button: &Button, bx: f32, by: f32, xo: f32, yo: f32) {
        let mut entries: Vec<(usize, bool)> =
            button.entries.iter().copied().filter(|e| e.1).collect();
        entries.extend(button.entries.iter().copied().filter(|e| !e.1));
        let (x, y) =
            screen::overlay_origin(bx as i32, by as i32, entries.len(), xo as i32, yo as i32);
        let (x, y) = (x as f32, y as f32);
        self.recipe_ui.overlay = Some(Overlay { x, y, entries });
    }

    // ---- placing a recipe ----

    /// What the book's click handler does once a recipe was placed: a book
    /// that is not offset next to the menu (`!isOffsetNextToMainGUI`, a window
    /// narrower than `NARROW`) closes with `setVisible(false)`.
    fn close_narrow_book(&mut self, book_type: BookType, batch: &HudBatch) {
        if batch.width >= NARROW {
            return;
        }
        self.level.recipe_book.open[book_type.index()] = false;
        self.send_book_settings(book_type);
        self.recipe_ui.overlay = None;
    }
    /// `RecipeBookComponent.tryPlaceRecipe`: false when the click is refused.
    fn try_place(&mut self, entry_index: usize, use_max: bool) -> bool {
        let Some(book_type) = self.active_book() else {
            return false;
        };
        let Some(id) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .map(|e| e.id.clone())
        else {
            return false;
        };
        let button_craftable = self
            .recipe_ui
            .view
            .buttons
            .iter()
            .flat_map(|b| b.entries.iter())
            .any(|e| e.0 == entry_index && e.1);
        if !button_craftable && self.recipe_ui.last_placed.as_deref() == Some(id.as_str()) {
            return false;
        }
        self.recipe_ui.last_placed = Some(id);
        self.recipe_ui.ghost.clear();
        self.place_recipe(book_type, entry_index, use_max);
        true
    }
    /// Whether the open screen takes `native_place_recipe`: a crafting grid's
    /// book (the inventory's or a crafting table's).
    pub(super) fn mod_recipe_screen(&self) -> bool {
        self.screen == Screen::Inventory
            && self.active_book() == Some(BookType::Crafting)
            && !self.spectator()
    }
    /// `native_place_recipe`: the book's click on the recipe with this display
    /// id, refused as `handlePlaceRecipe` refuses it (no crafting screen,
    /// not in the player's book; an impossible one in `place_recipe`).
    pub(super) fn mod_place_recipe(&mut self, display_id: i32, all: bool) {
        if !self.mod_recipe_screen() || !self.recipe_ui.client.displays().contains(&display_id) {
            return;
        }
        self.ensure_book();
        let Some(index) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.iter().position(|e| e.display_id == display_id))
        else {
            return;
        };
        self.recipe_ui.ghost.clear();
        self.place_recipe(BookType::Crafting, index, all);
    }
    /// `ServerboundPlaceRecipePacket`: the server places the recipe
    /// (`ServerPlaceRecipe` on the menu engine) and answers with a ghost
    /// recipe when the items are missing.
    fn place_recipe(&mut self, book_type: BookType, entry_index: usize, use_max: bool) {
        let Some(entry) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .cloned()
        else {
            return;
        };
        if self.remote.is_some() {
            // The server places it and answers with the slots, or a ghost.
            let container = self.remote_menu_id();
            self.send(horizon_protocol::packets::play::Serverbound::PlaceRecipe(
                horizon_protocol::packets::recipes::PlaceRecipe {
                    container,
                    recipe: entry.display_id,
                    use_max,
                },
            ));
            return;
        }
        let creative = self.creative;
        let mut post = None;
        let answer = &mut post;
        self.menu_session(|menu, env, _| {
            // handlePlaceRecipe: an impossible recipe is refused outright.
            if env
                .recipes
                .placement(&entry.id)
                .is_none_or(|p| p.impossible())
            {
                return Some(false);
            }
            *answer = Some(menu.place_recipe(env, &entry.id, use_max, creative, None));
            Some(true)
        });
        if post == Some(PostPlace::Ghost) {
            self.fill_ghost(book_type, &entry);
        }
        self.recipe_book_changed();
    }
    /// The container changed under the book: comparators and saving.
    fn recipe_book_changed(&mut self) {
        if let Some(key) = self.container {
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
            self.container_changed(key);
        }
    }

    // ---- a server's recipe book ----

    /// ClientboundRecipeBookAddPacket (`ClientRecipeBook.add`): the
    /// displays join the book, highlighted when new, and the recipes whose
    /// entries ask for it show the toast.
    pub(super) fn remote_recipe_book_add(
        &mut self,
        p: horizon_protocol::packets::recipes::RecipeBookAdd,
    ) {
        let ui = &mut self.recipe_ui;
        if p.replace {
            ui.client.clear();
        }
        let mut toast = Vec::new();
        for e in &p.entries {
            ui.client.add(e.id);
            if e.flags & 2 != 0 {
                ui.client.highlight.insert(e.id);
            }
            if e.flags & 1 != 0
                && let Some(id) = world_stations!(self).recipe_index.recipe_of_display(e.id)
                && !toast.iter().any(|t| t == id)
            {
                toast.push(id.to_owned());
            }
        }
        // A local world opened later syncs its book from scratch.
        ui.client_known = None;
        ui.client_version += 1;
        self.recipe_toast(&toast);
    }
    /// ClientboundRecipeBookRemovePacket.
    pub(super) fn remote_recipe_book_remove(
        &mut self,
        p: horizon_protocol::packets::recipes::RecipeBookRemove,
    ) {
        for id in p.ids {
            self.recipe_ui.client.remove(id);
        }
        self.recipe_ui.client_version += 1;
    }
    /// ClientboundRecipeBookSettingsPacket: each book's open and filter flags.
    pub(super) fn remote_recipe_book_settings(
        &mut self,
        p: horizon_protocol::packets::recipes::RecipeBookSettings,
    ) {
        for (i, (open, filtering)) in p.books.into_iter().enumerate() {
            self.level.recipe_book.open[i] = open;
            self.level.recipe_book.filtering[i] = filtering;
        }
    }
    /// ClientboundPlaceGhostRecipePacket: the recipe last placed, faded into
    /// the open menu's grid.
    pub(super) fn remote_ghost_recipe(
        &mut self,
        p: horizon_protocol::packets::recipes::PlaceGhostRecipe,
    ) {
        let open = self.remote_menu_id();
        let Some(book_type) = self.active_book() else {
            return;
        };
        if p.container != open {
            return;
        }
        let entry = self.recipe_ui.last_placed.as_ref().and_then(|id| {
            let book = self.recipe_ui.book.as_ref()?;
            book.entry(id).cloned()
        });
        if let Some(entry) = entry {
            self.recipe_ui.ghost.clear();
            self.fill_ghost(book_type, &entry);
        }
    }

    // ---- ghost recipe ----

    /// `fillGhostRecipe`: the recipe's ingredients faded into the slots.
    fn fill_ghost(&mut self, book_type: BookType, entry: &Entry) {
        let grid = self.craft_size;
        let fuel_empty = self.slot_mut(301).is_none();
        let Some(book) = self.recipe_ui.book.as_ref() else {
            return;
        };
        let ghost = book
            .ghost(entry, book_type, grid, fuel_empty, self.items())
            .into_iter()
            .map(|g| Ghost {
                slot: match (g.target, book_type) {
                    (GhostTarget::Result, BookType::Crafting) => 200,
                    (GhostTarget::Result, _) => 302,
                    (GhostTarget::Grid(i), _) => 100 + i,
                    (GhostTarget::Input, _) => 300,
                    (GhostTarget::Fuel, _) => 301,
                },
                result: g.target == GhostTarget::Result,
                items: g.items,
            })
            .collect();
        self.recipe_ui.ghost = ghost;
    }

    // ---- drawing ----

    /// The book button, ghost recipe and the open book.
    pub(super) fn draw_recipe_book(&mut self, batch: &mut HudBatch) {
        let Some(book_type) = self.active_book() else {
            return;
        };
        self.ensure_book();
        if !self.shift_down_or_ctrl() {
            self.recipe_ui.time += self.frame_ticks;
        }
        let (ox, _) = self.inventory_origin(batch);
        let cursor = self.cursor;
        let (bx, by) = self.book_button_pos(book_type, ox, batch);
        let hover = inside(cursor, bx, by, 20.0, 18.0);
        batch.sprite(
            &self.gui.atlas,
            &sprite(if hover {
                "button_highlighted"
            } else {
                "button"
            }),
            bx,
            by,
        );
        self.draw_ghost(batch, book_type);
        if self.book_open(book_type) {
            self.refresh_book_view(book_type);
            self.draw_book_panel(batch, book_type);
        }
        self.narrate_recipe_book(batch, book_type, [bx, by, 20.0, 18.0]);
    }
    /// `AbstractRecipeBookScreen`'s narratables: the book button (an
    /// `ImageButton` without a message) and the `RecipeBookComponent`, which
    /// narrates its recipe buttons, search box, filter and tabs while open.
    fn narrate_recipe_book(&self, batch: &HudBatch, book_type: BookType, button: [f32; 4]) {
        use horizon_client::widgets::{self, Narratable, key_of};
        if !widgets::narrating() {
            return;
        }
        // The button moves with the book: it keeps the focus a click gave it.
        widgets::narratable(|| {
            Narratable::button("", button, true).key(key_of(&"recipe_book/button"))
        });
        let open = self.book_open(book_type);
        let (xo, yo) = self.book_origin(batch);
        let group = key_of(&"recipe_book");
        widgets::narratable(|| Narratable::group(open, [xo, yo, WIDTH, HEIGHT]).key(group));
        if !open {
            return;
        }
        let in_book = |n: Narratable, i: usize| n.key(key_of(&(group, i))).in_group(group);
        let ui = &self.recipe_ui;
        let first = ui.page * 20;
        let shown = ui.view.buttons.iter().skip(first).take(20);
        if let Some(book) = ui.book.as_ref() {
            for (n, b) in shown.enumerate() {
                widgets::narratable(|| {
                    // RecipeButton: the result it shows now.
                    let results: Vec<Vec<ItemStack>> = b
                        .entries
                        .iter()
                        .map(|e| vec![book.entries[e.0].result.clone()])
                        .collect();
                    let stack = screen::display_stack(&results, self.swap_index());
                    let names = |id: &str| self.pack.names.display_item(id);
                    let item = horizon_client::stations::hover_name(&names, &stack);
                    let (x, y) = Self::button_pos(xo, yo, n);
                    in_book(
                        Narratable::recipe(item, b.entries.len() > 1, [x, y, 25.0, 25.0]),
                        n,
                    )
                });
            }
        }
        let tr = |key: &str, args: &[&str]| horizon_client::screen_narrator::translate(key, args);
        if let Some(search) = &ui.search {
            widgets::narratable(|| {
                Narratable::edit_box(
                    tr("itemGroup.search", &[]),
                    search.value.clone(),
                    [xo + 25.0, yo + 13.0, 81.0, 14.0],
                )
                .focused(search.focused)
                .key(key_of(&(group, "search")))
                .in_group(group)
            });
        }
        widgets::narratable(|| {
            // A CycleButton of no name that hides its label: ": <value>",
            // the value's tooltip.
            let label = |filtering: bool| {
                tr(
                    if !filtering {
                        "gui.recipebook.toggleRecipes.all"
                    } else {
                        match book_type {
                            BookType::Crafting => "gui.recipebook.toggleRecipes.craftable",
                            BookType::Furnace => "gui.recipebook.toggleRecipes.smeltable",
                            BookType::BlastFurnace => "gui.recipebook.toggleRecipes.blastable",
                            BookType::Smoker => "gui.recipebook.toggleRecipes.smokable",
                        }
                    },
                    &[],
                )
            };
            let filtering = self.book_filtering(book_type);
            let full = |filtering| tr("options.generic_value", &["", &label(filtering)]);
            Narratable::cycle(
                full(filtering),
                full(!filtering),
                [xo + 110.0, yo + 12.0, 26.0, 16.0],
                true,
            )
            .hint(label(filtering))
            .key(key_of(&(group, "filter")))
            .in_group(group)
        });
        for n in 0..ui.view.tabs.len() {
            let (x, y) = self.tab_pos(xo, yo, n);
            widgets::narratable(|| {
                Narratable::button("", [x, y, 35.0, 27.0], true)
                    .key(key_of(&(group, "tab", n)))
                    .in_group(group)
            });
        }
    }
    fn shift_down_or_ctrl(&self) -> bool {
        self.has_control_down()
    }
    /// `GhostSlots.extractRenderState`, among the container's slots.
    fn draw_ghost(&mut self, batch: &mut HudBatch, book_type: BookType) {
        if self.recipe_ui.ghost.is_empty() {
            return;
        }
        let (ox, oy) = self.inventory_origin(batch);
        let big_result = book_type != BookType::Crafting || self.craft_size == 3;
        let ghosts: Vec<GhostView> = self
            .recipe_ui
            .ghost
            .iter()
            .map(|g| {
                let (sx, sy) = self.slot_position(g.slot);
                GhostView {
                    x: sx as i32,
                    y: sy as i32,
                    items: g.items.clone(),
                    result: g.result,
                }
            })
            .collect();
        let calls = screen::ghost(self.swap_index(), ox as i32, oy as i32, big_result, &ghosts);
        self.paint_book(batch, &calls);
    }
    /// `GhostSlots.extractTooltip`: the stack a ghost shows on menu slot
    /// `slot` now, cycling like its draw; only while the open book is visible
    /// (`RecipeBookComponent.extractTooltip`).
    pub(super) fn ghost_stack(&self, slot: usize) -> Option<ItemStack> {
        if !self.book_open(self.active_book()?) {
            return None;
        }
        let ghost = self
            .recipe_ui
            .ghost
            .iter()
            .find(|g| g.slot == slot && !g.items.is_empty())?;
        Some(ghost.items[self.swap_index() % ghost.items.len()].clone())
    }
    /// `RecipeBookComponent.extractRenderState` and `extractTooltip`, laid
    /// out by `horizon_client::recipe_book_screen`.
    fn draw_book_panel(&mut self, batch: &mut HudBatch, book_type: BookType) {
        let (xo, yo) = self.book_origin(batch);
        let cursor = self.cursor;
        let atlas = &self.gui.atlas;
        let search = match self.recipe_ui.search.as_mut() {
            Some(edit) => {
                // The box's place, for clicks and typing.
                edit.x = xo + 25.0;
                edit.y = yo + 13.0;
                let (display, blink_ms) = edit.view_state(atlas);
                Search {
                    value: edit.value.clone(),
                    cursor: edit.cursor,
                    highlight: edit.highlight,
                    display,
                    focused: edit.focused,
                    blink_ms,
                }
            }
            None => Search::default(),
        };
        let ui = &self.recipe_ui;
        let current = ui
            .tab
            .filter(|t| ui.view.tabs.contains(t))
            .unwrap_or(Tab::Search);
        let tabs: Vec<TabView> = ui
            .view
            .tabs
            .iter()
            .map(|tab| {
                let (a, b) = tab.icons();
                TabView {
                    icons: (a.to_owned(), b.map(str::to_owned)),
                    selected: *tab == current,
                    anim: ui
                        .tab_anim
                        .iter()
                        .find(|e| e.0 == *tab)
                        .map_or(0.0, |e| e.1),
                }
            })
            .collect();
        let pages = ui.view.buttons.len().div_ceil(20);
        let first = ui.page * 20;
        let (buttons, overlay) = match ui.book.as_ref() {
            Some(book) => {
                let buttons: Vec<ButtonView> = ui
                    .view
                    .buttons
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(20)
                    .map(|(n, b)| ButtonView {
                        has_craftable: b.craftable(),
                        entries: b
                            .entries
                            .iter()
                            .map(|e| vec![book.entries[e.0].result.clone()])
                            .collect(),
                        anim: ui
                            .button_anim
                            .iter()
                            .find(|e| e.0 == n)
                            .map_or(0.0, |e| e.1),
                    })
                    .collect();
                let overlay = ui.overlay.as_ref().map(|o| OverlayView {
                    x: o.x as i32,
                    y: o.y as i32,
                    recipes: o
                        .entries
                        .iter()
                        .map(|&(i, craftable)| OverlayRecipe {
                            craftable,
                            cells: screen::overlay_cells(book, &book.entries[i]),
                        })
                        .collect(),
                });
                (buttons, overlay)
            }
            None => (Vec::new(), None),
        };
        let translate = |key: &str| self.translate(key, key);
        let text_width = |s: &str| HudBatch::text_width(atlas, s) as i32;
        let layout = Layout {
            book: book_type,
            width: batch.width as i32,
            height: batch.height as i32,
            narrow: batch.width < NARROW,
            mouse: (cursor.0.floor() as i32, cursor.1.floor() as i32),
            index: self.swap_index(),
            filtering: self.book_filtering(book_type),
            search: &search,
            tabs: &tabs,
            buttons: &buttons,
            page: ui.page,
            pages,
            overlay: overlay.as_ref(),
            ghost: None,
            translate: &translate,
            text_width: &text_width,
        };
        let calls = screen::draw(&layout);
        let tooltip = self.paint_book(batch, &calls);
        // The squeeze animations run down with the frames.
        let frame = self.frame_ticks;
        for e in &mut self.recipe_ui.tab_anim {
            e.1 -= frame;
        }
        for e in &mut self.recipe_ui.button_anim {
            e.1 -= frame;
        }
        self.recipe_ui.tab_anim.retain(|e| e.1 > 0.0);
        self.recipe_ui.button_anim.retain(|e| e.1 > 0.0);
        if let Some(lines) = tooltip {
            self.draw_tooltip_lines(batch, &lines, cursor.0, cursor.1);
        }
    }
    /// Paints recipe book calls; returns the first tooltip set (vanilla keeps
    /// the first one a frame asks for).
    fn paint_book(
        &mut self,
        batch: &mut HudBatch,
        calls: &[Drawn],
    ) -> Option<Vec<(horizon_client::text::Text, u32)>> {
        let mut tooltip: Option<Vec<(horizon_client::text::Text, u32)>> = None;
        for d in calls {
            let start = batch.vertices.len();
            let atlas = &self.gui.atlas;
            match &d.call {
                Call::Blit {
                    x, y, w, h, u, v, ..
                } => {
                    if let Some(r) = atlas.sprites.get("gui/recipe_book").copied() {
                        let (w, h) = (*w as f32, *h as f32);
                        batch.rect_uv(
                            *x as f32,
                            *y as f32,
                            w,
                            h,
                            sub_rect(r, [*u, *v, w, h]),
                            u32::MAX,
                        );
                    }
                }
                Call::Sprite { id, x, y, w, h } => {
                    let name = format!("gui/sprites/{id}");
                    let native = atlas
                        .sprites
                        .get(&name)
                        .is_some_and(|r| r.w as i32 == *w && r.h as i32 == *h);
                    if native {
                        batch.sprite(atlas, &name, *x as f32, *y as f32);
                    } else {
                        horizon_client::ui::nine_slice(
                            batch,
                            atlas,
                            &name,
                            *x as f32,
                            *y as f32,
                            *w as f32,
                            *h as f32,
                            u32::MAX,
                        );
                    }
                }
                Call::Fill {
                    x0,
                    y0,
                    x1,
                    y1,
                    color,
                } => batch.fill(
                    atlas,
                    *x0 as f32,
                    *y0 as f32,
                    (x1 - x0) as f32,
                    (y1 - y0) as f32,
                    argb(*color),
                ),
                Call::Highlight { x0, y0, x1, y1 } => {
                    let (a, b) = (*x0.min(x1), *x0.max(x1));
                    batch.text_highlight(
                        atlas,
                        (a as f32, *y0 as f32, (b - a) as f32, (y1 - y0) as f32),
                        true,
                    );
                }
                Call::Text {
                    runs,
                    x,
                    y,
                    color,
                    shadow,
                } => {
                    let mut x = *x as f32;
                    for r in runs {
                        let style = Style {
                            color: (r.color >= 0).then_some(r.color as u32),
                            italic: r.italic.then_some(true),
                            ..Style::default()
                        };
                        let t = Text::styled(r.text.clone(), style);
                        batch.text_styled(atlas, &t, x, *y as f32, argb(*color), *shadow);
                        x += atlas.font.width_text(&t);
                    }
                }
                Call::Item { call, stack, x, y } => match call {
                    ItemCall::Item | ItemCall::Fake => {
                        self.draw_item_model(batch, stack, *x as f32, *y as f32)
                    }
                    ItemCall::Decorations => {
                        self.draw_item_decorations(batch, stack, *x as f32, *y as f32, None)
                    }
                },
                Call::Tooltip(lines) => {
                    if tooltip.is_none() {
                        tooltip = Some(
                            lines
                                .iter()
                                .map(|l| {
                                    (
                                        horizon_client::text::Text::literal(
                                            l.iter().map(|r| r.text.as_str()).collect::<String>(),
                                        ),
                                        u32::MAX,
                                    )
                                })
                                .collect(),
                        );
                    }
                }
                Call::ItemTooltip { stack, more } => {
                    if tooltip.is_none() {
                        let mut lines = self.stack_tooltip(stack);
                        if *more {
                            lines.push((
                                horizon_client::text::Text::literal(self.translate(
                                    "gui.recipebook.moreRecipes",
                                    "Right Click for More",
                                )),
                                u32::MAX,
                            ));
                        }
                        tooltip = Some(lines);
                    }
                }
                Call::Stratum => {}
            }
            if d.pose != screen::IDENTITY {
                let m = d.pose;
                for v in &mut batch.vertices[start..] {
                    let [x, y] = v.pos;
                    v.pos = [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
                }
            }
        }
        tooltip
    }
}

/// An ARGB colour as a HUD vertex colour.
fn argb(c: i32) -> u32 {
    let c = c as u32;
    rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8)
}

/// A sub-rectangle of a sprite given in texels.
fn sub_rect(r: Rect, [x, y, w, h]: [f32; 4]) -> Rect {
    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
    Rect {
        u0: r.u0 + x * du,
        v0: r.v0 + y * dv,
        u1: r.u0 + (x + w) * du,
        v1: r.v0 + (y + h) * dv,
        w: w as u32,
        h: h as u32,
    }
}
