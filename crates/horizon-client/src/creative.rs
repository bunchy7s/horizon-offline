//! Creative inventory (CreativeModeInventoryScreen): vanilla's tabs and their
//! contents as built by the game's own display generators
//! (`creative-inventory.json`), a 9x5 scrolling grid and the search tab.
use crate::{Result, items::ItemStack, ui::EditBox};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub items: Vec<ItemStack>,
    pub top: bool,
    pub column: u32,
    pub search: bool,
    pub inventory: bool,
    /// `CreativeModeTab.Type.HOTBAR`: the saved toolbars, filled when the
    /// tab is selected (`set_saved_hotbars`).
    pub hotbar: bool,
    /// The page of tabs it is on: vanilla's are 0, mods' after, as NeoForge
    /// pages them.
    pub page: u32,
}
impl Tab {
    /// The tab without its stacks.
    fn header(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            icon: self.icon.clone(),
            items: Vec::new(),
            top: self.top,
            column: self.column,
            search: self.search,
            inventory: self.inventory,
            hotbar: self.hotbar,
            page: self.page,
        }
    }
}
/// A tab as exported, built with permissions: `operator` marks the stacks a
/// build without them leaves out (the export's `permissions_only`).
#[derive(Clone, Debug)]
struct Exported {
    tab: Tab,
    /// `CreativeModeTab.Type.CATEGORY`: shown only with a stack (`shouldDisplay`).
    category: bool,
    operator: Vec<bool>,
}
/// What the client's creative search trees (SessionSearchTrees) index for one
/// stack of the search tab: its creative tooltip lines and its item tags.
#[derive(Clone, Debug, Default)]
struct SearchEntry {
    /// The item id's namespace and path, lower-cased.
    namespace: String,
    path: String,
    /// Tooltip lines, lower-cased; empty when the resources predate them.
    lines: Vec<String>,
    /// Item tags as (namespace, path), lower-cased.
    tags: Vec<(String, String)>,
}
pub struct Creative {
    /// The tabs shown (`CreativeModeTabs.tabs()`), with the stacks the current
    /// permissions put in them.
    pub tabs: Vec<Tab>,
    pub selected: usize,
    /// The page of tabs shown.
    pub page: u32,
    /// Scroll position 0..1 (scrollOffs).
    pub scroll: f32,
    /// The scrollbar is held (scrolling): dragging moves it.
    pub scrolling: bool,
    /// CreativeModeInventoryScreen's search box: an unbordered `EditBox` of 50.
    pub search: EditBox,
    /// The search tab's stacks, in its display order.
    search_items: Vec<ItemStack>,
    search_index: Vec<SearchEntry>,
    /// Every exported tab, and the search tab's stacks with their entries and
    /// whether only permissions list them: `rebuild` keeps what is shown.
    exported: Vec<Exported>,
    exported_search: Vec<(ItemStack, SearchEntry, bool)>,
    /// The `hasPermissions` the tabs were last built with
    /// (`CreativeModeTabs.tryRebuildTabContents`).
    permissions: bool,
    /// ItemPickerMenu.items: what the grid shows, filled when the tab or the
    /// search changes (selectTab, refreshSearchResults), not on every frame.
    shown: Mutex<Option<Shown>>,
}
/// The grid's stacks for one tab, search text and language.
struct Shown {
    tab: usize,
    search: String,
    /// A display name, so a language change refills the search results.
    language: String,
    items: Arc<[ItemStack]>,
}
/// Display names from the pinned en_us language file.
#[derive(Default)]
pub struct Names {
    map: HashMap<String, String>,
}
impl Names {
    pub fn load(root: &Path) -> Self {
        let read = |namespace: &str| {
            fs::read(root.join(format!("assets/{namespace}/lang/en_us.json")))
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
                .unwrap_or_default()
        };
        let mut map = read("minecraft");
        // Content mods' names (docs/modpacks.md); none in a vanilla root.
        for namespace in crate::mod_content::namespaces(root) {
            map.extend(read(&namespace));
        }
        Self { map }
    }
    pub fn item(&self, id: &str) -> String {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        self.map
            .get(&format!("item.{namespace}.{path}"))
            .or_else(|| self.map.get(&format!("block.{namespace}.{path}")))
            .cloned()
            .unwrap_or_else(|| path.replace('_', " "))
    }
    /// An item's display name in the selected language: `item.*` then
    /// `block.*`, then the pinned English map (`Names::item`).
    pub fn display_item(&self, id: &str) -> String {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        if let Some(lang) = crate::lang::current() {
            for prefix in ["item", "block"] {
                if let Some(name) = lang.get(&format!("{prefix}.{namespace}.{path}")) {
                    return name.to_owned();
                }
            }
        }
        self.item(id)
    }
    /// A translation key's text in the selected language, else the pinned
    /// English map (`HoverName` and tooltip keys such as potion names).
    pub fn display_get(&self, key: &str) -> Option<String> {
        crate::lang::current()
            .and_then(|lang| lang.get(key).map(str::to_owned))
            .or_else(|| self.get(key).map(str::to_owned))
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
}

/// `CreativeModeTabs`' key for an exported tab id (`itemGroup.*`); the
/// export's `title` is the English fallback.
pub fn item_group_key(id: &str) -> Option<&'static str> {
    Some(match id.trim_start_matches("minecraft:") {
        "building_blocks" => "itemGroup.buildingBlocks",
        "colored_blocks" => "itemGroup.coloredBlocks",
        "natural_blocks" => "itemGroup.natural",
        "functional_blocks" => "itemGroup.functional",
        "redstone_blocks" => "itemGroup.redstone",
        "hotbar" => "itemGroup.hotbar",
        "search" => "itemGroup.search",
        "tools_and_utilities" => "itemGroup.tools",
        "combat" => "itemGroup.combat",
        "food_and_drinks" => "itemGroup.foodAndDrink",
        "ingredients" => "itemGroup.ingredients",
        "spawn_eggs" => "itemGroup.spawnEggs",
        "op_blocks" => "itemGroup.op",
        "inventory" => "itemGroup.inventory",
        _ => return None,
    })
}
impl Creative {
    pub const WIDTH: f32 = 195.0;
    pub const HEIGHT: f32 = 136.0;
    pub const TRASH_POSITION: (f32, f32) = (173.0, 112.0);

    /// Visible player slots in the creative inventory tab (game slot ids).
    pub fn inventory_slots() -> impl Iterator<Item = usize> {
        (0..36).chain(500..505)
    }

    /// CreativeModeInventoryScreen.selectTab positions its InventoryMenu wrappers.
    pub fn inventory_slot_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            0..9 => Some((9.0 + slot as f32 * 18.0, 112.0)),
            9..36 => Some((
                9.0 + ((slot - 9) % 9) as f32 * 18.0,
                54.0 + ((slot - 9) / 9) as f32 * 18.0,
            )),
            500..504 => {
                let position = slot - 500;
                Some((
                    54.0 + (position / 2) as f32 * 54.0,
                    6.0 + (position % 2) as f32 * 27.0,
                ))
            }
            504 => Some((35.0, 20.0)),
            _ => None,
        }
    }
    /// Game slot id of an item tab's first display slot (ItemPickerMenu's
    /// `CustomCreativeSlot`s, 9 by 5); its hotbar row keeps ids 0..9.
    pub const DISPLAY: usize = 900;

    /// Visible slots of an item tab (game slot ids).
    pub fn tab_slots() -> impl Iterator<Item = usize> {
        (Self::DISPLAY..Self::DISPLAY + 45).chain(0..9)
    }

    /// ItemPickerMenu's slot positions.
    pub fn tab_slot_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            0..9 => Some((9.0 + slot as f32 * 18.0, 112.0)),
            _ if (Self::DISPLAY..Self::DISPLAY + 45).contains(&slot) => {
                let i = slot - Self::DISPLAY;
                Some((9.0 + (i % 9) as f32 * 18.0, 18.0 + (i / 9) as f32 * 18.0))
            }
            _ => None,
        }
    }
    /// The scrollbar track (insideScrollbar), relative to the screen origin.
    pub const SCROLL_X: f32 = 175.0;
    pub const SCROLL_Y: f32 = 18.0;
    pub const SCROLL_WIDTH: f32 = 14.0;
    pub const SCROLL_HEIGHT: f32 = 112.0;
    /// `creative-inventory.json` (VanillaCreativeTabsExporter); resources
    /// prepared before it fall back to `creative-tabs.json`'s item ids.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("creative-inventory.json");
        if path.exists() {
            Self::from_json(&serde_json::from_slice(&fs::read(path)?)?)
        } else {
            Self::from_legacy(&serde_json::from_slice(&fs::read(
                root.join("creative-tabs.json"),
            )?)?)
        }
    }
    pub fn from_json(raw: &Value) -> Result<Self> {
        let mut tabs = Vec::new();
        let mut search = Vec::new();
        for t in raw["tabs"].as_array().into_iter().flatten() {
            // The stacks only a build with permissions lists, by index.
            let only: Vec<u64> = t["permissions_only"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect();
            let mut items = Vec::new();
            let mut operator = Vec::new();
            for (i, v) in t["items"].as_array().into_iter().flatten().enumerate() {
                let snbt = v.as_str().unwrap_or("-");
                if let Some(stack) = crate::item_data::parse_stack(snbt)? {
                    items.push(stack);
                    operator.push(only.contains(&(i as u64)));
                }
            }
            let kind = t["type"].as_str().unwrap_or("");
            if kind == "SEARCH" {
                let strings = |v: &Value| -> Vec<String> {
                    v.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(java_lower))
                        .collect()
                };
                for (i, stack) in items.iter().enumerate() {
                    let (namespace, path) = split_id(&java_lower(&stack.item));
                    let entry = SearchEntry {
                        namespace,
                        path,
                        lines: strings(&t["lines"][i]),
                        tags: strings(&t["tags"][i]).iter().map(|t| split_id(t)).collect(),
                    };
                    search.push((stack.clone(), entry, operator[i]));
                }
            }
            let icon = t["icon"]
                .as_str()
                .and_then(|s| crate::item_data::parse_stack(s).ok().flatten());
            tabs.push(Exported {
                tab: Tab {
                    id: t["id"].as_str().unwrap_or("").to_owned(),
                    title: t["title"].as_str().unwrap_or("").to_owned(),
                    icon: icon.map_or_else(|| "minecraft:stone".to_owned(), |s| s.item.to_string()),
                    items,
                    top: t["row"] == "TOP",
                    column: t["column"].as_u64().unwrap_or(0) as u32,
                    search: kind == "SEARCH",
                    inventory: kind == "INVENTORY",
                    hotbar: kind == "HOTBAR",
                    page: t["page"].as_u64().unwrap_or(0) as u32,
                },
                category: kind == "CATEGORY",
                operator,
            });
        }
        Ok(Self::new(tabs, search))
    }
    /// The old export: item ids per tab, no components, tooltips or tags.
    fn from_legacy(raw: &Value) -> Result<Self> {
        let mut tabs = Vec::new();
        let mut search_items: Vec<ItemStack> = Vec::new();
        for t in raw.as_array().into_iter().flatten() {
            let items: Vec<ItemStack> = t["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(|id| ItemStack::new(id, 1)))
                .collect();
            if t["id"] == "minecraft:hotbar" || t["id"] == "minecraft:op_blocks" {
                continue;
            }
            for i in &items {
                if !search_items.iter().any(|s| s.item == i.item) {
                    search_items.push(i.clone());
                }
            }
            let kind = t["type"].as_str().unwrap_or("");
            tabs.push(Exported {
                operator: vec![false; items.len()],
                tab: Tab {
                    id: t["id"].as_str().unwrap_or("").to_owned(),
                    title: t["title"].as_str().unwrap_or("").to_owned(),
                    icon: t["icon"].as_str().unwrap_or("minecraft:stone").to_owned(),
                    items,
                    top: t["row"] == "TOP",
                    column: t["column"].as_u64().unwrap_or(0) as u32,
                    search: kind == "SEARCH",
                    inventory: kind == "INVENTORY",
                    hotbar: false,
                    page: 0,
                },
                category: false,
            });
        }
        let search = search_items
            .into_iter()
            .map(|s| {
                let (namespace, path) = split_id(&java_lower(&s.item));
                let entry = SearchEntry {
                    namespace,
                    path,
                    ..Default::default()
                };
                (s, entry, false)
            })
            .collect();
        Ok(Self::new(tabs, search))
    }
    fn new(exported: Vec<Exported>, exported_search: Vec<(ItemStack, SearchEntry, bool)>) -> Self {
        let mut creative = Self {
            tabs: Vec::new(),
            selected: 0,
            page: 0,
            scroll: 0.0,
            scrolling: false,
            search: {
                let mut search = EditBox::new(0.0, 0.0, 80.0, 9.0);
                search.bordered = false;
                search.max_length = 50;
                search.text_color = u32::MAX;
                search
            },
            search_items: Vec::new(),
            search_index: Vec::new(),
            exported,
            exported_search,
            permissions: false,
            shown: Mutex::new(None),
        };
        creative.rebuild();
        creative
    }
    /// `CreativeModeTabs.buildAllTabContents` with `permissions`: the operator
    /// tab's display items generator lists its stacks only with them
    /// (`hasPermissions`), and the search tab with it; a category tab is
    /// shown with a stack (`CreativeModeTab.shouldDisplay`).
    fn rebuild(&mut self) {
        let permissions = self.permissions;
        let keep = |operator: bool| permissions || !operator;
        self.tabs = self
            .exported
            .iter()
            .filter_map(|e| {
                let items: Vec<ItemStack> = (e.tab.items.iter().zip(&e.operator))
                    .filter(|(_, o)| keep(**o))
                    .map(|(s, _)| s.clone())
                    .collect();
                (!e.category || !items.is_empty()).then(|| Tab {
                    items,
                    ..e.tab.header()
                })
            })
            .collect();
        (self.search_items, self.search_index) = self
            .exported_search
            .iter()
            .filter(|(_, _, o)| keep(*o))
            .map(|(s, e, _)| (s.clone(), e.clone()))
            .unzip();
        self.forget_items();
    }
    /// `CreativeModeInventoryScreen.tryRefreshInvalidatedTabs`, which its
    /// constructor and `containerTick` run with `hasPermissions`
    /// (`Player.canUseGameMasterBlocks` and the Operator Items Tab option):
    /// when that changed, the tabs are built again. A selected tab left
    /// without stacks (the operator tab) gives way to the default tab; any
    /// other keeps its first row (`refreshCurrentTabContents`). True when the
    /// tabs were rebuilt.
    pub fn set_permissions(&mut self, permissions: bool, names: &Names) -> bool {
        if permissions == self.permissions {
            return false;
        }
        let selected = self.tab().id.clone();
        let row = self.first_row(names);
        self.permissions = permissions;
        self.rebuild();
        match self.tabs.iter().position(|t| t.id == selected) {
            Some(tab) => {
                self.selected = tab;
                // ItemPickerMenu.getScrollForRowIndex.
                let hidden = self.hidden_rows(names);
                self.scroll = if hidden > 0 {
                    (row as f32 / hidden as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
            }
            None => {
                // CreativeModeTabs.getDefaultTab.
                let default = self
                    .tabs
                    .iter()
                    .position(|t| t.id == "minecraft:building_blocks");
                self.select(default.unwrap_or(0));
            }
        }
        true
    }
    /// Where a stack comes in the creative tabs (the search tab lists them
    /// all, in tab order): the same stack's place, else its item's first.
    /// None for items the tabs don't show.
    pub fn order_of(&self, stack: &ItemStack) -> Option<usize> {
        let items = &self.search_items;
        items
            .iter()
            .position(|s| s.same_item_same_components(stack))
            .or_else(|| items.iter().position(|s| s.item == stack.item))
    }
    /// The tabs without their stacks: what the screen draws of them each frame.
    pub fn tab_headers(&self) -> Vec<Tab> {
        self.tabs.iter().map(Tab::header).collect()
    }
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.selected.min(self.tabs.len() - 1)]
    }
    /// selectTab: a different tab empties the search box; the scrollbar goes to the top.
    /// A tab on another page shows that page.
    pub fn select(&mut self, tab: usize) {
        if tab != self.selected {
            self.search.set_value("");
        }
        self.selected = tab;
        if let Some(t) = self.tabs.get(tab)
            && !Self::on_every_page(t)
        {
            self.page = t.page;
        }
        self.scroll = 0.0;
        if self.tab().search {
            self.search.focused = true;
            self.search.reset_blink();
        }
    }
    /// How many pages of tabs there are (1 without mods' tabs).
    pub fn pages(&self) -> u32 {
        self.tabs.iter().map(|t| t.page + 1).max().unwrap_or(1)
    }
    /// The tabs every page shows (NeoForge's): the right-hand ones, saved
    /// hotbars, search, operator blocks and the inventory.
    fn on_every_page(t: &Tab) -> bool {
        t.column >= 5
    }
    /// Whether tab `t` shows on the page shown.
    pub fn on_page(&self, t: &Tab) -> bool {
        t.page == self.page || Self::on_every_page(t)
    }
    /// Shows page `page` (NeoForge's page buttons) and selects its first tab,
    /// unless the selected tab is one every page shows.
    pub fn set_page(&mut self, page: u32) {
        let page = page.min(self.pages() - 1);
        let kept = self
            .tabs
            .get(self.selected)
            .is_some_and(Self::on_every_page);
        if !kept
            && let Some(first) = self
                .tabs
                .iter()
                .position(|t| t.page == page && !Self::on_every_page(t))
        {
            self.select(first);
        }
        self.page = page;
    }
    /// The search tab's stacks.
    pub fn search_items(&self) -> &[ItemStack] {
        &self.search_items
    }
    /// Opening the screen: vanilla rebuilds CreativeModeInventoryScreen, whose
    /// init() makes a fresh search box and a zeroed scrollbar; only the tab
    /// persists (its static selectedTab). selectTab on the reopened screen
    /// takes the oldTab != tab branch, so it empties and refocuses the box.
    pub fn reopen(&mut self) {
        self.search.set_value("");
        self.release();
        self.scroll = 0.0;
        self.search.focused = self.tab().search;
        if self.search.focused {
            self.search.reset_blink();
        }
    }
    /// Indices into `search_items` that CreativeModeInventoryScreen.refreshSearchResults
    /// shows for `term`: the whole tab when empty, the tag tree after `#`, else
    /// the name tree, both over vanilla's suffix arrays (substring matches,
    /// results in the tab's order).
    pub fn search_results(&self, term: &str, names: &Names) -> Vec<usize> {
        let all = 0..self.search_index.len();
        if term.is_empty() {
            return all.collect();
        }
        if let Some(tag) = term.strip_prefix('#') {
            // IdSearchTree over the stacks' tags.
            let text = java_lower(tag);
            return all
                .filter(|&i| {
                    let tags = &self.search_index[i].tags;
                    match text.split_once(':') {
                        None => any(tags.iter().map(|t| t.1.as_str()), &text),
                        Some((ns, path)) => {
                            let (ns, path) = (java_trim(ns), java_trim(path));
                            any(tags.iter().map(|t| t.0.as_str()), ns)
                                && any(tags.iter().map(|t| t.1.as_str()), path)
                        }
                    }
                })
                .collect();
        }
        // FullTextSearchTree: tooltip lines, and the item id after a colon.
        // The export's lines are English (they come from the vanilla jars);
        // the hover name in the selected language is matched too, so a
        // localized search finds the item. It is the whole name (a tipped
        // arrow's potion, the ominous banner), as the first line is.
        let text = java_lower(term);
        let localized = |i: usize| {
            java_lower(&crate::item_tooltip::name_line(
                &self.search_items[i],
                &|k| names.display_get(k),
            ))
        };
        all.filter(|&i| {
            let e = &self.search_index[i];
            let fallback;
            let lines: &[String] = if e.lines.is_empty() {
                fallback = [java_lower(&names.item(&self.search_items[i].item))];
                &fallback
            } else {
                &e.lines
            };
            match text.split_once(':') {
                None => {
                    any(lines.iter().map(String::as_str), &text) || localized(i).contains(&text)
                }
                Some((ns, path)) => {
                    let (ns, path) = (java_trim(ns), java_trim(path));
                    e.namespace.contains(ns)
                        && (e.path.contains(path)
                            || any(lines.iter().map(String::as_str), path)
                            || localized(i).contains(path))
                }
            }
        })
        .collect()
    }
    /// Items shown by the selected tab (the search tab filters by `search`),
    /// worked out again only when the tab, the search or the language changed.
    pub fn items(&self, names: &Names) -> Arc<[ItemStack]> {
        let tab = self.selected.min(self.tabs.len() - 1);
        let search = if self.tab().search {
            self.search.value.as_str()
        } else {
            ""
        };
        let language = names.display_get("itemGroup.search").unwrap_or_default();
        let mut shown = self.shown.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = shown.as_ref()
            && s.tab == tab
            && s.search == search
            && s.language == language
        {
            return s.items.clone();
        }
        let items: Arc<[ItemStack]> = if self.tab().search {
            self.search_results(search, names)
                .into_iter()
                .map(|i| self.search_items[i].clone())
                .collect()
        } else {
            self.tab().items.as_slice().into()
        };
        *shown = Some(Shown {
            tab,
            search: search.to_owned(),
            language,
            items: items.clone(),
        });
        items
    }
    /// The stack in grid cell `index` (row-major from the tab's top), None
    /// for an empty cell: a saved toolbar's empty slot, or past the end.
    pub fn shown(&self, names: &Names, index: usize) -> Option<ItemStack> {
        self.items(names)
            .get(index)
            .filter(|s| s.count > 0)
            .cloned()
    }
    /// `CreativeModeInventoryScreen.selectTab` for the HOTBAR tab: the nine
    /// saved rows of `HotbarManager` as the tab's items, an empty stack (count
    /// 0) for an empty slot.
    pub fn set_saved_hotbars(&mut self, items: Vec<ItemStack>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.hotbar) {
            tab.items = items;
        }
        self.forget_items();
    }
    /// Drop the shown items, so the next `items` works them out again: a
    /// reload can rename items without changing the Search label.
    pub fn forget_items(&self) {
        *self.shown.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    pub fn rows(&self, names: &Names) -> usize {
        self.items(names).len().div_ceil(9)
    }
    /// ItemPickerMenu.calculateRowCount: rows beyond the five shown.
    fn hidden_rows(&self, names: &Names) -> i32 {
        self.rows(names) as i32 - 5
    }
    /// First visible grid row for the scroll position (getRowIndexForScroll).
    pub fn first_row(&self, names: &Names) -> usize {
        ((self.scroll * self.hidden_rows(names) as f32 + 0.5) as i32).max(0) as usize
    }
    /// CreativeModeInventoryScreen.canScroll: more than five rows, on a tab with a scrollbar.
    pub fn can_scroll(&self, names: &Names) -> bool {
        !self.tab().inventory && self.items(names).len() > 45
    }
    /// insideScrollbar, for a point relative to the screen origin.
    pub fn inside_scrollbar(&self, x: f32, y: f32) -> bool {
        x >= Self::SCROLL_X
            && y >= Self::SCROLL_Y
            && x < Self::SCROLL_X + Self::SCROLL_WIDTH
            && y < Self::SCROLL_Y + Self::SCROLL_HEIGHT
            && !self.tab().inventory
    }
    /// mouseClicked on the scrollbar: it is held only if the tab can scroll
    /// (the thumb doesn't move until the mouse does).
    pub fn press_scrollbar(&mut self, x: f32, y: f32, names: &Names) -> bool {
        if !self.inside_scrollbar(x, y) {
            return false;
        }
        self.scrolling = self.can_scroll(names);
        true
    }
    /// mouseDragged while the scrollbar is held, at a cursor height relative to
    /// the screen origin, wherever the cursor is (even off the track or the window).
    pub fn drag_scrollbar(&mut self, y: f32) -> bool {
        if !self.scrolling {
            return false;
        }
        let (top, bottom) = (Self::SCROLL_Y, Self::SCROLL_Y + Self::SCROLL_HEIGHT);
        self.scroll = ((y - top - 7.5) / (bottom - top - 15.0)).clamp(0.0, 1.0);
        true
    }
    /// mouseReleased.
    pub fn release(&mut self) {
        self.scrolling = false;
    }
    /// mouseScrolled: ItemPickerMenu.subtractInputFromScroll.
    pub fn wheel(&mut self, amount: f64, names: &Names) {
        if !self.can_scroll(names) {
            return;
        }
        let rows = self.hidden_rows(names) as f64;
        self.scroll = (self.scroll - (amount / rows) as f32).clamp(0.0, 1.0);
    }
    /// The scroller sprite's offset from the top of the track.
    pub fn scroller_offset(&self) -> f32 {
        ((Self::SCROLL_HEIGHT - 17.0) * self.scroll) as i32 as f32
    }
    /// The tabs whose `CreativeModeTab.contains(stack)` holds, in
    /// `CreativeModeTabs.tabs()` order, the search tab left out: the stack is
    /// among the tab's search-tab items, by item and components. The export
    /// lists each tab's own items; the search tab's stacks that no tab shows
    /// (the enchanted books below their top level, `SEARCH_TAB_ONLY`) belong
    /// to the tabs that show their item.
    pub fn tabs_containing(&self, stack: &ItemStack) -> Vec<usize> {
        let category = |t: &Tab| !t.search && !t.inventory;
        let tabs_where = |shows: &dyn Fn(&ItemStack) -> bool| -> Vec<usize> {
            (0..self.tabs.len())
                .filter(|&i| category(&self.tabs[i]) && self.tabs[i].items.iter().any(shows))
                .collect()
        };
        let found = tabs_where(&|s| same_components(s, stack));
        if found.is_empty() && self.search_items.iter().any(|s| same_components(s, stack)) {
            return tabs_where(&|s| s.item == stack.item);
        }
        found
    }
    /// The `#namespace:path` lines `getTooltipFromContainerItem` adds for a
    /// search tab stack while the search is a `#` query: the item tags
    /// `updateVisibleTags` keeps for the text after `#` (the path contains it;
    /// with a colon, the namespace and the path each contain their part) that
    /// the stack has.
    pub fn search_tag_lines(&self, stack: &ItemStack) -> Vec<String> {
        let Some(text) = self.search.value.strip_prefix('#') else {
            return Vec::new();
        };
        let Some(entry) = self
            .search_items
            .iter()
            .position(|s| same_components(s, stack))
            .and_then(|i| self.search_index.get(i))
        else {
            return Vec::new();
        };
        let visible = |(ns, path): &(String, String)| match text.split_once(':') {
            None => path.contains(text),
            Some((n, p)) => ns.contains(java_trim(n)) && path.contains(java_trim(p)),
        };
        entry
            .tags
            .iter()
            .filter(|t| visible(t))
            .map(|(ns, path)| format!("#{ns}:{path}"))
            .collect()
    }
    /// Tab button position relative to the screen origin (CreativeModeInventoryScreen.getTabX/Y).
    pub fn tab_position(tab: &Tab) -> (f32, f32) {
        let mut x = tab.column as f32 * 27.0;
        if tab.column >= 5 {
            // Right-aligned tabs (search, operator, inventory).
            x = Self::WIDTH - 27.0 * (7 - tab.column) as f32 + 1.0;
        }
        let y = if tab.top { -28.0 } else { Self::HEIGHT - 4.0 };
        (x, y)
    }
    /// `CreativeModeInventoryScreen.extractTabButton`'s cursor: an unselected
    /// tab asks for the pointing hand when the whole-pixel mouse, relative to
    /// the screen origin, is strictly inside its 26x32 button.
    pub fn tab_wants_hand(tab: &Tab, selected: bool, mouse: (f32, f32)) -> bool {
        let (x, y) = Self::tab_position(tab);
        let (mx, my) = (mouse.0.floor(), mouse.1.floor());
        !selected && mx > x && my > y && mx < x + 26.0 && my < y + 32.0
    }
}
/// Whether a suffix array of `strings` has a suffix starting with `text`:
/// some string contains it (none for no strings, even when `text` is empty).
fn any<'a>(strings: impl IntoIterator<Item = &'a str>, text: &str) -> bool {
    strings.into_iter().any(|s| s.contains(text))
}
/// `ItemStack.isSameItemSameComponents`, the count left out, as the
/// tabs' `ItemStackLinkedSet` compares: no components and empty ones match.
fn same_components(a: &ItemStack, b: &ItemStack) -> bool {
    let none = crate::item_data::StackData::default();
    a.item == b.item
        && a.damage == b.damage
        && a.data().unwrap_or(&none) == b.data().unwrap_or(&none)
}
/// `CreativeModeInventoryScreen.getTooltipFromContainerItem`: the plain
/// tooltip only for a category tab's own grid slots (`CustomCreativeSlot`);
/// everywhere else in the screen the tab names (and, on the search tab's
/// grid, the tag lines) follow the item's name.
pub fn tooltip_lists_tabs(category_tab: bool, grid_slot: bool) -> bool {
    !(category_tab && grid_slot)
}
/// String.toLowerCase(Locale.ROOT).
fn java_lower(s: &str) -> String {
    s.to_lowercase()
}
/// String.trim: strips code points up to U+0020.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}
/// Identifier.parse's namespace and path (default namespace `minecraft`).
fn split_id(id: &str) -> (String, String) {
    match id.split_once(':') {
        Some((ns, path)) => (ns.to_owned(), path.to_owned()),
        None => ("minecraft".to_owned(), id.to_owned()),
    }
}
/// The slot under the cursor, for the search tab's keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hover {
    Nothing,
    /// A display slot (`isCreativeSlot`), showing an item or past the end.
    Display {
        filled: bool,
    },
    /// Any other slot: the hotbar row, or the inventory tab's slots.
    Slot,
}

/// `CreativeModeInventoryScreen.keyPressed` on the search tab: whether a
/// key is a hotbar swap on the hovered slot instead of text for the search
/// box. `numeric` is `getNumericKeyValue().isPresent()` (the digit and keypad
/// keys); `hotbar_key` is a `keyHotbarSlots` key. Over an empty display slot
/// the key types (`doQuickSwap` is false), and `checkHotbarKeyPressed` needs
/// a hovered slot and an empty cursor.
pub fn search_key_swaps(numeric: bool, hotbar_key: bool, hover: Hover, carrying: bool) -> bool {
    let quick_swap = hover != Hover::Display { filled: false };
    quick_swap && numeric && hotbar_key && !carrying && hover != Hover::Nothing
}

/// What a click on an item tab's display slot does besides changing the
/// carried stack.
#[derive(Clone, PartialEq, Debug)]
pub enum DisplayClick {
    Nothing,
    /// SWAP: the hotbar slot gets a full stack of the item.
    Hotbar(usize, ItemStack),
    /// SWAP with button 40 (Key F): the off-hand gets a full stack of the item.
    Offhand(ItemStack),
    /// THROW: a copy of the item is dropped.
    Drop(ItemStack),
}

/// Whether a stack has `minecraft:creative_slot_lock`.
pub fn slot_locked(stack: &ItemStack) -> bool {
    stack
        .data()
        .is_some_and(|d| d.other("minecraft:creative_slot_lock").is_some())
}

/// `CreativeModeInventoryScreen.slotClicked` on a display slot
/// (`slot.container == CONTAINER`): `shown` is the slot's item (None past
/// the tab's end) and `max` gives an item's max stack size. The tab's items
/// are never changed: clicks make copies.
pub fn click_display(
    carried: &mut Option<ItemStack>,
    shown: Option<&ItemStack>,
    max: impl Fn(&ItemStack) -> u32,
    button: i32,
    input: crate::menu::Input,
) -> DisplayClick {
    use crate::menu::Input;
    // CustomCreativeSlot.mayPickup: a `creative_slot_lock` stack (the saved
    // toolbars' hint) can't be clicked at all.
    if shown.is_some_and(slot_locked) {
        return DisplayClick::Nothing;
    }
    let copy = |s: &ItemStack, count: u32| ItemStack { count, ..s.clone() };
    let quick_key = input == Input::QuickMove;
    match input {
        Input::Swap => {
            return match shown {
                Some(s) if button == 40 => DisplayClick::Offhand(copy(s, max(s))),
                Some(s) if (0..9).contains(&button) => {
                    DisplayClick::Hotbar(button as usize, copy(s, max(s)))
                }
                Some(_) | None => DisplayClick::Nothing,
            };
        }
        Input::Clone => {
            if carried.is_none()
                && let Some(s) = shown
            {
                *carried = Some(copy(s, max(s)));
            }
            return DisplayClick::Nothing;
        }
        Input::Throw => {
            return match shown {
                Some(s) => DisplayClick::Drop(copy(s, if button == 0 { 1 } else { max(s) })),
                None => DisplayClick::Nothing,
            };
        }
        _ => {}
    }
    match (carried.as_mut(), shown) {
        (Some(c), Some(s)) if c.item == s.item && c.extra == s.extra && c.damage == s.damage => {
            // isSameItemSameComponents: grow, fill or shrink what is carried.
            if button == 0 {
                if quick_key {
                    c.count = max(c);
                } else if c.count < max(c) {
                    c.count += 1;
                }
            } else {
                c.count -= 1;
            }
        }
        (None, Some(s)) => {
            *carried = Some(copy(s, if quick_key { max(s) } else { s.count }));
        }
        (Some(c), _) => {
            if button == 0 {
                *carried = None;
            } else {
                c.count -= 1;
            }
        }
        (None, None) => {}
    }
    if carried.as_ref().is_some_and(|c| c.count == 0) {
        *carried = None;
    }
    DisplayClick::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::nbt::Tag;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    /// One click on a display slot showing one stone (max 64; pearls 16).
    fn display(
        carried: Option<ItemStack>,
        button: i32,
        input: crate::menu::Input,
    ) -> (Option<ItemStack>, DisplayClick) {
        let mut carried = carried;
        let max = |s: &ItemStack| if s.item.ends_with("pearl") { 16 } else { 64 };
        let effect = click_display(&mut carried, Some(&stack("stone", 1)), max, button, input);
        (carried, effect)
    }

    /// Tabs as the export lists them: two block tabs sharing the oak log,
    /// ingredients with the top-level book only, the search tab with every
    /// stack (a level 1 book only there) and its tags, the inventory.
    fn tooltip_tabs() -> Creative {
        let book = |level: u8| {
            format!(
                r#"{{components:{{"minecraft:stored_enchantments":{{"minecraft:sharpness":{level}}}}},count:1,id:"minecraft:enchanted_book"}}"#
            )
        };
        let log = r#"{count:1,id:"minecraft:oak_log"}"#;
        let dirt = r#"{count:1,id:"minecraft:dirt"}"#;
        let tab = |id: &str, kind: &str, items: Vec<String>| serde_json::json!({"id": id, "type": kind, "title": id, "icon": log, "items": items});
        let mut search = tab(
            "minecraft:search",
            "SEARCH",
            vec![log.into(), dirt.into(), book(1), book(5)],
        );
        search["lines"] = serde_json::json!([[], [], [], []]);
        search["tags"] = serde_json::json!([
            [
                "minecraft:logs",
                "minecraft:oak_logs",
                "minecraft:logs_that_burn"
            ],
            ["minecraft:dirt"],
            [],
            []
        ]);
        Creative::from_json(&serde_json::json!({"tabs": [
            tab("minecraft:building_blocks", "CATEGORY", vec![log.into()]),
            tab("minecraft:natural_blocks", "CATEGORY", vec![log.into(), dirt.into()]),
            tab("minecraft:hotbar", "HOTBAR", vec![]),
            search,
            tab("minecraft:ingredients", "CATEGORY", vec![book(5)]),
            tab("minecraft:inventory", "INVENTORY", vec![]),
        ]}))
        .unwrap()
    }

    #[test]
    fn tooltip_lists_the_tabs_that_contain_the_stack() {
        let c = tooltip_tabs();
        let ids = |s: &ItemStack| -> Vec<&str> {
            c.tabs_containing(s)
                .into_iter()
                .map(|i| c.tabs[i].id.as_str())
                .collect()
        };
        // In tab order, the search and inventory tabs left out; the count
        // doesn't matter.
        assert_eq!(
            ids(&stack("oak_log", 37)),
            ["minecraft:building_blocks", "minecraft:natural_blocks"]
        );
        assert_eq!(ids(&stack("dirt", 1)), ["minecraft:natural_blocks"]);
        // A stack no tab holds: other components, or no creative item.
        let mut damaged = stack("oak_log", 1);
        damaged.damage = 3;
        assert!(ids(&damaged).is_empty());
        assert!(ids(&stack("barrier", 1)).is_empty());
        // A level 1 book is a search-tab-only stack of the ingredients tab.
        let books: Vec<&ItemStack> = c
            .search_items()
            .iter()
            .filter(|s| &*s.item == "minecraft:enchanted_book")
            .collect();
        assert_eq!(ids(books[0]), ["minecraft:ingredients"]);
        assert_eq!(ids(books[1]), ["minecraft:ingredients"]);
    }

    #[test]
    fn tooltip_tab_names_skip_only_a_category_tab_grid() {
        assert!(!tooltip_lists_tabs(true, true));
        // The hotbar row of a category tab, the search tab's grid and hotbar,
        // the inventory tab.
        assert!(tooltip_lists_tabs(true, false));
        assert!(tooltip_lists_tabs(false, true));
        assert!(tooltip_lists_tabs(false, false));
    }

    #[test]
    fn tag_search_lists_the_matching_tags_of_the_stack() {
        let mut c = tooltip_tabs();
        let log = stack("oak_log", 1);
        assert!(c.search_tag_lines(&log).is_empty());
        c.search.set_value("logs");
        assert!(c.search_tag_lines(&log).is_empty());
        c.search.set_value("#logs");
        assert_eq!(
            c.search_tag_lines(&log),
            [
                "#minecraft:logs",
                "#minecraft:oak_logs",
                "#minecraft:logs_that_burn"
            ]
        );
        c.search.set_value("#oak");
        assert_eq!(c.search_tag_lines(&log), ["#minecraft:oak_logs"]);
        c.search.set_value("#mine : burn");
        assert_eq!(c.search_tag_lines(&log), ["#minecraft:logs_that_burn"]);
        assert!(c.search_tag_lines(&stack("dirt", 1)).is_empty());
    }

    #[test]
    fn search_tab_number_keys_swap_over_a_slot_before_typing() {
        let filled = Hover::Display { filled: true };
        let empty = Hover::Display { filled: false };
        // Over an item, the hotbar row or another slot: the hotbar swap.
        assert!(search_key_swaps(true, true, filled, false));
        assert!(search_key_swaps(true, true, Hover::Slot, false));
        // Over an empty display slot, nothing or with a carried stack: text.
        assert!(!search_key_swaps(true, true, empty, false));
        assert!(!search_key_swaps(true, true, Hover::Nothing, false));
        assert!(!search_key_swaps(true, true, filled, true));
        // Letters always type; so do the digits no hotbar key is bound to.
        assert!(!search_key_swaps(false, false, filled, false));
        assert!(!search_key_swaps(true, false, filled, false));
    }

    #[test]
    fn display_slot_clicks_follow_slot_clicked() {
        use crate::menu::Input::*;
        // An empty hand takes the shown stack, or a full one with Shift.
        assert_eq!(display(None, 0, Pickup).0, Some(stack("stone", 1)));
        assert_eq!(display(None, 1, Pickup).0, Some(stack("stone", 1)));
        assert_eq!(display(None, 0, QuickMove).0, Some(stack("stone", 64)));
        // The same item grows by one (left), fills (Shift) or shrinks (right).
        assert_eq!(
            display(Some(stack("stone", 5)), 0, Pickup).0,
            Some(stack("stone", 6))
        );
        assert_eq!(
            display(Some(stack("stone", 64)), 0, Pickup).0,
            Some(stack("stone", 64))
        );
        assert_eq!(
            display(Some(stack("stone", 5)), 0, QuickMove).0,
            Some(stack("stone", 64))
        );
        assert_eq!(
            display(Some(stack("stone", 5)), 1, Pickup).0,
            Some(stack("stone", 4))
        );
        assert_eq!(display(Some(stack("stone", 1)), 1, QuickMove).0, None);
        // Another item is deleted (left) or loses one (right).
        assert_eq!(display(Some(stack("ender_pearl", 9)), 0, Pickup).0, None);
        assert_eq!(display(Some(stack("ender_pearl", 9)), 0, QuickMove).0, None);
        assert_eq!(
            display(Some(stack("ender_pearl", 9)), 1, Pickup).0,
            Some(stack("ender_pearl", 8))
        );
        // Number keys fill a hotbar slot, F fills off-hand, Q drops copies, pick block clones.
        assert_eq!(
            display(None, 3, Swap).1,
            DisplayClick::Hotbar(3, stack("stone", 64))
        );
        assert_eq!(
            display(None, 40, Swap).1,
            DisplayClick::Offhand(stack("stone", 64))
        );
        assert_eq!(
            display(None, 0, Throw).1,
            DisplayClick::Drop(stack("stone", 1))
        );
        assert_eq!(
            display(None, 1, Throw).1,
            DisplayClick::Drop(stack("stone", 64))
        );
        assert_eq!(display(None, 2, Clone).0, Some(stack("stone", 64)));
        assert_eq!(
            display(Some(stack("dirt", 2)), 2, Clone).0,
            Some(stack("dirt", 2))
        );
    }

    #[test]
    fn an_empty_display_slot_deletes_or_shrinks_the_carried_stack() {
        use crate::menu::Input::Pickup;
        let mut carried = Some(stack("dirt", 3));
        click_display(&mut carried, None, |_| 64, 1, Pickup);
        assert_eq!(carried, Some(stack("dirt", 2)));
        click_display(&mut carried, None, |_| 64, 0, Pickup);
        assert_eq!(carried, None);
    }

    fn creative(items: usize) -> Creative {
        let mut stacks: Vec<String> = ["anvil", "crafting_table", "wooden_hoe"]
            .iter()
            .map(|i| format!("{{count:1,id:\"minecraft:{i}\"}}"))
            .collect();
        stacks.extend((0..items).map(|i| format!("{{count:1,id:\"minecraft:item_{i}\"}}")));
        let mut lines = vec![
            vec!["Anvil"],
            vec!["Crafting Table"],
            vec!["Wooden Hoe", "When in Main Hand:", "-3 Attack Speed"],
        ];
        lines.extend(vec![vec!["Thing"]; items]);
        let mut tags = vec![
            vec!["minecraft:anvil"],
            vec![],
            vec!["minecraft:hoes", "minecraft:enchantable/mining"],
        ];
        tags.extend(vec![vec![]; items]);
        let raw = serde_json::json!({"schema": 2, "tabs": [
            {"id": "minecraft:functional_blocks", "type": "CATEGORY", "row": "TOP", "column": 3, "title": "Functional Blocks",
             "icon": "{count:1,id:\"minecraft:oak_sign\"}", "visible": true,
             "items": ["{count:1,id:\"minecraft:anvil\"}", "{count:1,id:\"minecraft:crafting_table\"}"]},
            {"id": "minecraft:op_blocks", "type": "CATEGORY", "row": "BOTTOM", "column": 5, "title": "Operator Utilities",
             "icon": "{count:1,id:\"minecraft:command_block\"}", "visible": false, "items": []},
            {"id": "minecraft:search", "type": "SEARCH", "row": "TOP", "column": 6, "title": "Search Items",
             "icon": "{count:1,id:\"minecraft:compass\"}", "visible": true,
             "items": stacks, "lines": lines, "tags": tags},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": "{count:1,id:\"minecraft:chest\"}", "visible": true, "items": []}
        ]});
        Creative::from_json(&raw).unwrap()
    }

    #[test]
    fn inventory_tab_slot_layout_matches_vanilla() {
        assert_eq!(Creative::inventory_slots().count(), 41);
        for (slot, expected) in [
            (0, (9.0, 112.0)),
            (8, (153.0, 112.0)),
            (9, (9.0, 54.0)),
            (35, (153.0, 90.0)),
            (500, (54.0, 6.0)),
            (501, (54.0, 33.0)),
            (502, (108.0, 6.0)),
            (503, (108.0, 33.0)),
            (504, (35.0, 20.0)),
        ] {
            assert_eq!(Creative::inventory_slot_position(slot), Some(expected));
        }
        assert_eq!(Creative::inventory_slot_position(100), None);
        assert_eq!(Creative::TRASH_POSITION, (173.0, 112.0));
        let mut c = creative(90);
        c.select(2);
        assert!(c.tab().inventory);
        assert!(!c.can_scroll(&Names {
            map: HashMap::new()
        }));
    }

    #[test]
    fn inventory_tab_can_return_to_category_and_search() {
        let mut c = creative(90);
        for (selected, inventory, search) in [
            (2, true, false),
            (0, false, false),
            (1, false, true),
            (2, true, false),
        ] {
            c.search.set_value("anvil");
            c.scroll = 0.5;
            c.select(selected);
            assert_eq!(c.selected, selected);
            assert_eq!(c.tab().inventory, inventory);
            assert_eq!(c.tab().search, search);
            assert!(c.search.value.is_empty());
            assert_eq!(c.scroll, 0.0);
        }
    }

    #[test]
    fn hidden_tabs_are_skipped() {
        let c = creative(0);
        let ids: Vec<&str> = c.tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "minecraft:functional_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        assert_eq!(c.tabs[0].items[0].item.as_ref(), "minecraft:anvil");
        let mut c = c;
        c.select(1);
        c.search.set_value("anvil");
        c.scroll = 0.5;
        c.select(1);
        assert_eq!((c.search.value.as_str(), c.scroll), ("anvil", 0.0));
        c.select(0);
        assert_eq!(c.search.value, "");
    }

    /// An export built with permissions: the operator tab's stacks, and their
    /// place at the end of the search tab, only with them.
    fn with_operator_tab() -> Creative {
        let stack = |id: &str| format!("{{count:1,id:\"minecraft:{id}\"}}");
        let raw = serde_json::json!({"schema": 3, "tabs": [
            {"id": "minecraft:building_blocks", "type": "CATEGORY", "row": "TOP", "column": 0, "title": "Building Blocks",
             "icon": stack("bricks"), "visible": true, "items": [stack("stone")]},
            {"id": "minecraft:op_blocks", "type": "CATEGORY", "row": "BOTTOM", "column": 5, "title": "Operator Utilities",
             "icon": stack("command_block"), "visible": false,
             "items": [stack("command_block"), stack("barrier")], "permissions_only": [0, 1]},
            {"id": "minecraft:search", "type": "SEARCH", "row": "TOP", "column": 6, "title": "Search Items",
             "icon": stack("compass"), "visible": true,
             "items": [stack("stone"), stack("command_block"), stack("barrier")],
             "lines": [["Stone"], ["Command Block"], ["Barrier"]], "tags": [[], [], []],
             "permissions_only": [1, 2]},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": stack("chest"), "visible": true, "items": []}
        ]});
        Creative::from_json(&raw).unwrap()
    }

    fn tab_ids(c: &Creative) -> Vec<&str> {
        c.tabs.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn operator_stacks_stay_out_without_permissions() {
        let c = with_operator_tab();
        let names = Names::default();
        assert_eq!(
            tab_ids(&c),
            [
                "minecraft:building_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        assert_eq!(c.search_items().len(), 1);
        assert!(c.search_results("command", &names).is_empty());
        assert_eq!(c.search_results("", &names), [0]);
    }

    #[test]
    fn operator_tab_comes_and_goes_with_permissions() {
        let mut c = with_operator_tab();
        let names = Names::default();
        assert!(!c.set_permissions(false, &names));
        assert!(c.set_permissions(true, &names));
        assert!(!c.set_permissions(true, &names));
        assert_eq!(
            tab_ids(&c),
            [
                "minecraft:building_blocks",
                "minecraft:op_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        let op: Vec<&str> = c.tabs[1].items.iter().map(|s| s.item.as_ref()).collect();
        assert_eq!(op, ["minecraft:command_block", "minecraft:barrier"]);
        assert_eq!(c.search_results("command", &names), [1]);
        assert_eq!(c.search_results("", &names), [0, 1, 2]);
        // The search tab keeps its place across the rebuild.
        c.select(2);
        c.search.set_value("block");
        assert!(c.set_permissions(false, &names));
        assert!(c.tab().search);
        assert_eq!(c.search.value, "block");
        assert!(c.items(&names).is_empty());
        // The selected operator tab is left empty: the default tab instead.
        c.set_permissions(true, &names);
        c.select(1);
        assert_eq!(c.items(&names).len(), 2);
        c.set_permissions(false, &names);
        assert_eq!(c.tab().id, "minecraft:building_blocks");
        assert_eq!(c.items(&names)[0].item.as_ref(), "minecraft:stone");
    }

    #[test]
    fn saved_hotbars_tab_shows_its_rows_and_locks_the_hint() {
        let mut c = Creative::from_json(&serde_json::json!({"schema": 2, "tabs": [
            {"id": "minecraft:hotbar", "type": "HOTBAR", "row": "TOP", "column": 5, "title": "Saved Hotbars",
             "icon": "{count:1,id:\"minecraft:bookshelf\"}", "visible": true, "items": []},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": "{count:1,id:\"minecraft:chest\"}", "visible": true, "items": []}
        ]}))
        .unwrap();
        assert!(c.tab().hotbar);
        assert!(!c.tab_headers()[1].hotbar);
        let mut hint = stack("paper", 1);
        hint.data_mut().set_other(
            "minecraft:creative_slot_lock",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        // A saved sword in slot 0, an empty slot, then the hint.
        c.set_saved_hotbars(vec![stack("diamond_sword", 1), stack("air", 0), hint]);
        let names = Names {
            map: HashMap::new(),
        };
        assert_eq!(c.shown(&names, 0), Some(stack("diamond_sword", 1)));
        assert_eq!(c.shown(&names, 1), None);
        assert_eq!(c.shown(&names, 3), None);
        let hint = c.shown(&names, 2).unwrap();
        assert!(slot_locked(&hint));
        assert!(!slot_locked(&stack("diamond_sword", 1)));
        // CustomCreativeSlot.mayPickup: the hint is never taken, and a carried
        // stack stays as it is.
        use crate::menu::Input::*;
        for (carried, button, input) in [
            (None, 0, Pickup),
            (None, 0, QuickMove),
            (None, 2, Clone),
            (None, 3, Swap),
            (None, 0, Throw),
            (Some(stack("dirt", 2)), 0, Pickup),
            (Some(stack("dirt", 2)), 1, Pickup),
        ] {
            let mut carried = carried;
            let before = carried.clone();
            let effect = click_display(&mut carried, Some(&hint), |_| 64, button, input);
            assert_eq!(effect, DisplayClick::Nothing);
            assert_eq!(carried, before);
        }
    }

    #[test]
    fn search_matches_vanilla_trees() {
        let c = creative(0);
        let names = Names {
            map: HashMap::new(),
        };
        let q = |s: &str| c.search_results(s, &names);
        assert_eq!(q(""), [0, 1, 2]);
        assert_eq!(q("ANVIL"), [0]);
        assert_eq!(q("table"), [1]);
        assert_eq!(q("a"), [0, 1, 2]);
        // Tooltip lines, not just names; no trimming without a colon.
        assert_eq!(q("attack speed"), [2]);
        assert_eq!(q(" anvil"), Vec::<usize>::new());
        // A colon searches the id: namespace, then the path or a tooltip line.
        assert_eq!(q("minecraft:crafting_t"), [1]);
        assert_eq!(q(" minecraft : hoe "), [2]);
        assert_eq!(q("mine:anvil"), [0]);
        assert_eq!(q("other:anvil"), Vec::<usize>::new());
        assert_eq!(q(":"), [0, 1, 2]);
        // Tags after "#": only stacks with tags.
        assert_eq!(q("#"), [0, 2]);
        assert_eq!(q("#hoe"), [2]);
        assert_eq!(q("#minecraft:enchantable/"), [2]);
        assert_eq!(q("#craft"), Vec::<usize>::new());
    }

    #[test]
    fn search_matches_the_display_name_in_the_selected_language() {
        let c = creative(0);
        let names = Names {
            map: HashMap::from([
                ("item.minecraft.anvil".to_owned(), "铁砧".to_owned()),
                ("item.minecraft.wooden_hoe".to_owned(), "木锄".to_owned()),
            ]),
        };
        let q = |s: &str| c.search_results(s, &names);
        assert_eq!(q("铁砧"), [0]);
        assert_eq!(q("木锄"), [2]);
        // The English tooltip lines the export indexed still match.
        assert_eq!(q("anvil"), [0]);
        // A colon's path searches the display name too.
        assert_eq!(q("minecraft:铁砧"), [0]);
    }

    #[test]
    fn scrollbar_drag_follows_the_mouse_while_held() {
        let mut c = creative(90);
        let names = Names {
            map: HashMap::new(),
        };
        c.selected = 1;
        // Pressing the track holds the scrollbar without moving it.
        assert!(c.press_scrollbar(180.0, 60.0, &names));
        assert!(c.scrolling);
        assert_eq!(c.scroll, 0.0);
        // Dragging maps the cursor height onto the track (minus half the thumb).
        assert!(c.drag_scrollbar(18.0 + 7.5 + 48.5));
        assert!((c.scroll - 0.5).abs() < 1e-6);
        // The cursor may leave the track, the panel or the window: it keeps scrolling.
        c.drag_scrollbar(400.0);
        assert_eq!(c.scroll, 1.0);
        c.drag_scrollbar(-50.0);
        assert_eq!(c.scroll, 0.0);
        c.drag_scrollbar(18.0 + 7.5 + 97.0 * 0.25);
        assert!((c.scroll - 0.25).abs() < 1e-6);
        let rows = (93usize.div_ceil(9) - 5) as f32;
        assert_eq!(c.first_row(&names), (0.25 * rows + 0.5) as usize);
        // Releasing stops it.
        c.release();
        assert!(!c.drag_scrollbar(400.0));
        assert!((c.scroll - 0.25).abs() < 1e-6);
        // Off the track, or with too few items, it isn't held.
        assert!(!c.press_scrollbar(170.0, 60.0, &names));
        assert!(!c.press_scrollbar(180.0, 130.0, &names));
        c.selected = 0;
        assert!(c.press_scrollbar(180.0, 60.0, &names));
        assert!(!c.scrolling);
    }

    #[test]
    fn creative_tab_titles_have_vanilla_keys() {
        assert_eq!(
            item_group_key("minecraft:building_blocks"),
            Some("itemGroup.buildingBlocks")
        );
        assert_eq!(
            item_group_key("minecraft:food_and_drinks"),
            Some("itemGroup.foodAndDrink")
        );
        assert_eq!(item_group_key("minecraft:hotbar"), Some("itemGroup.hotbar"));
        assert_eq!(item_group_key("minecraft:nonsense"), None);
    }

    #[test]
    fn wheel_moves_one_row_per_notch() {
        let mut c = creative(90);
        let names = Names {
            map: HashMap::new(),
        };
        c.selected = 1;
        let rows = (93usize.div_ceil(9) - 5) as f32;
        c.wheel(-1.0, &names);
        assert!((c.scroll - 1.0 / rows).abs() < 1e-6);
        assert_eq!(c.first_row(&names), 1);
        c.wheel(5.0, &names);
        assert_eq!(c.scroll, 0.0);
        assert_eq!(c.scroller_offset(), 0.0);
        c.scroll = 1.0;
        assert_eq!(c.scroller_offset(), 95.0);
    }

    #[test]
    fn search_edits_at_its_cursor() {
        use winit::keyboard::KeyCode;
        let mut c = creative(0);
        c.select(1);
        c.search.insert_text("hello world");
        for _ in 0..5 {
            assert!(c.search.key(KeyCode::ArrowLeft, false, false));
        }
        c.search.insert_text("X");
        assert_eq!(c.search.value, "hello Xworld");
        c.search.key(KeyCode::Home, false, true);
        c.search.insert_text("Y");
        assert_eq!(c.search.value, "Yworld");
        // Ctrl+Left jumps a word; one step right and Backspace deletes before the cursor.
        c.search.key(KeyCode::ArrowLeft, true, false);
        c.search.key(KeyCode::ArrowRight, false, false);
        c.search.key(KeyCode::Backspace, false, false);
        assert_eq!(c.search.value, "world");
    }

    #[test]
    fn unselected_tabs_want_the_hand_strictly_inside() {
        let tab = |top, column| Tab {
            id: String::new(),
            title: String::new(),
            icon: String::new(),
            items: Vec::new(),
            top,
            column,
            search: false,
            inventory: false,
            hotbar: false,
            page: 0,
        };
        // Top row, second column: the button spans x 27..53, y -28..4.
        let t = tab(true, 1);
        assert!(Creative::tab_wants_hand(&t, false, (40.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, true, (40.0, -10.0)));
        // Its edges ask for nothing; the mouse counts in whole pixels.
        assert!(!Creative::tab_wants_hand(&t, false, (27.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (27.9, -10.0)));
        assert!(Creative::tab_wants_hand(&t, false, (28.0, -10.0)));
        assert!(Creative::tab_wants_hand(&t, false, (52.9, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (53.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (40.0, -28.0)));
        assert!(Creative::tab_wants_hand(&t, false, (40.0, -27.0)));
        assert!(Creative::tab_wants_hand(&t, false, (40.0, 3.9)));
        assert!(!Creative::tab_wants_hand(&t, false, (40.0, 4.0)));
        // Bottom row, right-aligned inventory column: x 169..195, y 132..164.
        let b = tab(false, 6);
        assert!(Creative::tab_wants_hand(&b, false, (180.0, 140.0)));
        assert!(!Creative::tab_wants_hand(&b, false, (169.0, 140.0)));
        assert!(Creative::tab_wants_hand(&b, false, (170.0, 140.0)));
        assert!(!Creative::tab_wants_hand(&b, false, (180.0, 132.0)));
    }
}
