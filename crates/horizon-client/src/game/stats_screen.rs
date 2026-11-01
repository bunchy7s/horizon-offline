//! The Statistics screen (`StatsScreen`): General, Items and Mobs tabs over
//! the player's statistics, with the Done button.
use super::{Game, Screen};
use horizon_client::{hud::HudBatch, items::ItemStack, text::Text, ui};
use std::fmt::Write as _;

/// `Stats.CUSTOM` in registration order with its formatter.
const CUSTOM: &[(&str, Format)] = &[
    ("leave_game", Format::Default),
    ("play_time", Format::Time),
    ("total_world_time", Format::Time),
    ("time_since_death", Format::Time),
    ("time_since_rest", Format::Time),
    ("sneak_time", Format::Time),
    ("walk_one_cm", Format::Distance),
    ("crouch_one_cm", Format::Distance),
    ("sprint_one_cm", Format::Distance),
    ("walk_on_water_one_cm", Format::Distance),
    ("fall_one_cm", Format::Distance),
    ("climb_one_cm", Format::Distance),
    ("fly_one_cm", Format::Distance),
    ("walk_under_water_one_cm", Format::Distance),
    ("minecart_one_cm", Format::Distance),
    ("boat_one_cm", Format::Distance),
    ("pig_one_cm", Format::Distance),
    ("happy_ghast_one_cm", Format::Distance),
    ("horse_one_cm", Format::Distance),
    ("aviate_one_cm", Format::Distance),
    ("swim_one_cm", Format::Distance),
    ("strider_one_cm", Format::Distance),
    ("nautilus_one_cm", Format::Distance),
    ("jump", Format::Default),
    ("drop", Format::Default),
    ("damage_dealt", Format::Tenths),
    ("damage_dealt_absorbed", Format::Tenths),
    ("damage_dealt_resisted", Format::Tenths),
    ("damage_taken", Format::Tenths),
    ("damage_blocked_by_shield", Format::Tenths),
    ("damage_absorbed", Format::Tenths),
    ("damage_resisted", Format::Tenths),
    ("deaths", Format::Default),
    ("mob_kills", Format::Default),
    ("animals_bred", Format::Default),
    ("player_kills", Format::Default),
    ("fish_caught", Format::Default),
    ("talked_to_villager", Format::Default),
    ("traded_with_villager", Format::Default),
    ("eat_cake_slice", Format::Default),
    ("fill_cauldron", Format::Default),
    ("use_cauldron", Format::Default),
    ("clean_armor", Format::Default),
    ("clean_banner", Format::Default),
    ("clean_shulker_box", Format::Default),
    ("interact_with_brewingstand", Format::Default),
    ("interact_with_beacon", Format::Default),
    ("inspect_dropper", Format::Default),
    ("inspect_hopper", Format::Default),
    ("inspect_dispenser", Format::Default),
    ("play_noteblock", Format::Default),
    ("tune_noteblock", Format::Default),
    ("pot_flower", Format::Default),
    ("trigger_trapped_chest", Format::Default),
    ("open_enderchest", Format::Default),
    ("enchant_item", Format::Default),
    ("play_record", Format::Default),
    ("interact_with_furnace", Format::Default),
    ("interact_with_crafting_table", Format::Default),
    ("open_chest", Format::Default),
    ("sleep_in_bed", Format::Default),
    ("sleep_in_straw_bed", Format::Default),
    ("open_shulker_box", Format::Default),
    ("open_barrel", Format::Default),
    ("interact_with_blast_furnace", Format::Default),
    ("interact_with_smoker", Format::Default),
    ("interact_with_lectern", Format::Default),
    ("interact_with_campfire", Format::Default),
    ("interact_with_cartography_table", Format::Default),
    ("interact_with_loom", Format::Default),
    ("interact_with_stonecutter", Format::Default),
    ("bell_ring", Format::Default),
    ("raid_trigger", Format::Default),
    ("raid_win", Format::Default),
    ("interact_with_anvil", Format::Default),
    ("interact_with_grindstone", Format::Default),
    ("target_hit", Format::Default),
    ("interact_with_smithing_table", Format::Default),
];
#[derive(Clone, Copy)]
enum Format {
    Default,
    Tenths,
    Distance,
    Time,
}
fn two(v: f64) -> String {
    format!("{v:.2}")
}
/// `StatFormatter`.
fn format(f: Format, v: i32) -> String {
    match f {
        Format::Default => {
            // NumberFormat.getIntegerInstance(Locale.US): thousands separators.
            let digits = v.unsigned_abs().to_string();
            let mut out = String::new();
            for (i, c) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i).is_multiple_of(3) {
                    out.push(',');
                }
                out.push(c);
            }
            if v < 0 {
                out.insert(0, '-');
            }
            out
        }
        Format::Tenths => two(f64::from(v) * 0.1),
        Format::Distance => {
            let m = f64::from(v) / 100.0;
            let km = m / 1000.0;
            if km > 0.5 {
                format!("{} km", two(km))
            } else if m > 0.5 {
                format!("{} m", two(m))
            } else {
                format!("{v} cm")
            }
        }
        Format::Time => {
            let s = f64::from(v) / 20.0;
            let (min, h) = (s / 60.0, s / 3600.0);
            let (d, y) = (h / 24.0, h / 24.0 / 365.0);
            if y > 0.5 {
                format!("{} y", two(y))
            } else if d > 0.5 {
                format!("{} d", two(d))
            } else if h > 0.5 {
                format!("{} h", two(h))
            } else if min > 0.5 {
                format!("{} min", two(min))
            } else {
                // Java's double + " s".
                let mut t = String::new();
                let _ = write!(t, "{s:?}");
                format!("{t} s")
            }
        }
    }
}
/// The item columns: (stat type, header sprite).
const COLUMNS: [(&str, &str); 6] = [
    ("mined", "gui/sprites/statistics/block_mined"),
    ("broken", "gui/sprites/statistics/item_broken"),
    ("crafted", "gui/sprites/statistics/item_crafted"),
    ("used", "gui/sprites/statistics/item_used"),
    ("picked_up", "gui/sprites/statistics/item_picked_up"),
    ("dropped", "gui/sprites/statistics/item_dropped"),
];
#[derive(Default)]
pub(super) struct StatsUi {
    tab: usize,
    /// The list's scrolling: the wheel and the held scrollbar.
    scroll: horizon_client::ui::Scroll,
    sort: Option<(usize, i32)>,
}
/// `MenuTabBar` geometry: three tabs over at most 400 pixels.
fn tabs(w: f32) -> (f32, f32) {
    let total = w.min(400.0) - 28.0;
    let tab = ((total / 3.0) / 2.0).round() * 2.0;
    let x0 = ((w - tab * 3.0) / 2.0).floor();
    (x0, tab)
}
const TAB_H: f32 = 24.0;
const FOOTER: f32 = 33.0;
impl Game<'_> {
    pub(super) fn open_stats(&mut self) {
        self.load_stats();
        // StatsScreen.init: a server's client asks for its statistics.
        if self.remote.is_some() {
            self.send(horizon_protocol::packets::play::Serverbound::ClientCommand(
                horizon_protocol::packets::play::ClientCommand { action: 1 },
            ));
        }
        self.stats_ui = StatsUi::default();
        self.screen = Screen::Stats;
    }
    /// The list's scrollbar column (right of the 280 pixel list) and the
    /// pixels it scrolls.
    fn stats_bar(w: f32) -> f32 {
        ((w - 280.0) / 2.0).floor() + 280.0 + 6.0
    }
    fn stats_content(&self) -> f32 {
        let (row_h, rows) = self.stats_rows();
        row_h * rows as f32 + 4.0
    }
    fn stats_rows(&self) -> (f32, usize) {
        match self.stats_ui.tab {
            0 => (14.0, CUSTOM.len()),
            1 => (22.0, self.stat_items().len() + 1),
            _ => (36.0, self.stat_mobs().len()),
        }
    }
    /// Whether tab `i` can be chosen: the Items and Mobs tabs are active only
    /// while their list has rows.
    fn stats_tab_active(&self, i: usize) -> bool {
        match i {
            1 => !self.stat_items().is_empty(),
            2 => !self.stat_mobs().is_empty(),
            _ => true,
        }
    }
    /// Items (and block items) with any non-zero column.
    fn stat_items(&self) -> Vec<String> {
        let mut items: Vec<String> = Vec::new();
        for (ty, _) in COLUMNS {
            if let Some(m) = self.stats.values.get(&format!("minecraft:{ty}")) {
                for (k, v) in m {
                    if *v > 0 && k != "minecraft:air" && !items.contains(k) {
                        items.push(k.clone());
                    }
                }
            }
        }
        if let Some((col, order)) = self.stats_ui.sort {
            let ty = COLUMNS[col].0;
            items.sort_by_key(|i| self.stats.get(ty, i) * -order);
        }
        items
    }
    fn stat_mobs(&self) -> Vec<String> {
        let mut mobs: Vec<String> = Vec::new();
        for ty in ["killed", "killed_by"] {
            if let Some(m) = self.stats.values.get(&format!("minecraft:{ty}")) {
                for (k, v) in m {
                    if *v > 0 && !mobs.contains(k) {
                        mobs.push(k.clone());
                    }
                }
            }
        }
        mobs
    }
    /// `MobRow`: the mob's name, its kills and killed-by lines, and their
    /// colours (grey when none).
    fn mob_row(&self, mob: &str) -> (Text, Text, Text, (u32, u32)) {
        let name = Text::translate(&format!("entity.{}", mob.replace(':', ".")), &[]);
        let kills = self.stats.get("killed", mob);
        let by = self.stats.get("killed_by", mob);
        let k = if kills == 0 {
            Text::translate(
                "stat_type.minecraft.killed.none",
                std::slice::from_ref(&name),
            )
        } else {
            Text::translate(
                "stat_type.minecraft.killed",
                &[Text::literal(kills.to_string()), name.clone()],
            )
        };
        let b = if by == 0 {
            Text::translate(
                "stat_type.minecraft.killed_by.none",
                std::slice::from_ref(&name),
            )
        } else {
            Text::translate(
                "stat_type.minecraft.killed_by",
                &[name.clone(), Text::literal(by.to_string())],
            )
        };
        let colors = (
            if kills > 0 { 0xFFBA_BABA } else { 0xFF80_8080 },
            if by > 0 { 0xFFBA_BABA } else { 0xFF80_8080 },
        );
        (name, k, b, colors)
    }
    pub(super) fn draw_stats(&mut self, batch: &mut HudBatch) {
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new(
                "stats",
                super::screen_info::tr("gui.stats", &[]),
            )
        });
        let (w, h) = (batch.width, batch.height);
        let atlas = &self.gui.atlas;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let (top, bottom) = (TAB_H, h - FOOTER);
        ui::separators(batch, atlas, top, bottom, true);
        // The tab bar.
        let mut tab_tip = false;
        let (x0, tw) = tabs(w);
        const TABS: [&str; 3] = ["stat.generalButton", "stat.itemsButton", "stat.mobsButton"];
        // Its TabNavigationBar: the hovered tab, else the current one (a
        // GridLayoutTab's extra narration is empty).
        horizon_client::widgets::narratable(|| {
            use horizon_client::widgets::{ListKind, Narratable, Row, What};
            let row = |i: usize| {
                let mut row = Row::new(i, Text::translate(TABS[i], &[]).plain());
                row.hint = Some(String::new());
                row
            };
            let hovered = (0..TABS.len())
                .find(|&i| ui::hovered(self.cursor, x0 + tw * i as f32, 0.0, tw, TAB_H));
            Narratable::new(
                What::List {
                    kind: ListKind::Tabs,
                    total: TABS.len(),
                    hovered: hovered.map(row),
                    selected: Some(row(self.stats_ui.tab.min(TABS.len() - 1))),
                },
                [x0, 0.0, tw * TABS.len() as f32, TAB_H],
            )
            .key(horizon_client::widgets::key_of(&"stats/tabs"))
        });
        for (i, key) in TABS.into_iter().enumerate() {
            let x = x0 + tw * i as f32;
            let selected = self.stats_ui.tab == i;
            let active = self.stats_tab_active(i);
            let hover = ui::hovered(self.cursor, x, 0.0, tw, TAB_H);
            if hover && !active {
                tab_tip = true;
            }
            let sprite = match (selected, hover) {
                (true, true) => "gui/sprites/widget/tab_selected_highlighted",
                (true, false) => "gui/sprites/widget/tab_selected",
                (false, true) => "gui/sprites/widget/tab_highlighted",
                (false, false) => "gui/sprites/widget/tab",
            };
            ui::nine_slice(batch, atlas, sprite, x, 0.0, tw, TAB_H, u32::MAX);
            let label = Text::translate(key, &[]);
            let lw = atlas.font.width_text(&label);
            let color = if selected && active {
                u32::MAX
            } else {
                0xFFA0_A0A0
            };
            batch.text_styled(
                atlas,
                &label,
                (x + (tw - lw) / 2.0).floor(),
                8.0,
                color,
                true,
            );
            if selected {
                batch.fill(atlas, x + 2.0, TAB_H - 3.0, tw - 4.0, 1.0, u32::MAX);
            }
        }
        // The list.
        let (row_h, rows) = self.stats_rows();
        let content = row_h * rows as f32 + 4.0;
        self.stats_ui.scroll.clamp(top, bottom, content);
        let left = ((w - 280.0) / 2.0).floor();
        let scroll = self.stats_ui.scroll.amount;
        let row_y = |i: usize| top + 4.0 + i as f32 * row_h - scroll;
        let visible = |y: f32| y >= top && y + row_h <= bottom;
        // The list's row under the mouse (`getHovered`).
        let hovered_row = {
            let (cx, cy) = self.cursor;
            let i = ((cy - (top + 4.0) + scroll) / row_h).floor();
            let inside = cy >= top && cy < bottom && cx >= left && cx < left + 280.0;
            (inside && i >= 0.0 && (i as usize) < rows).then_some(i as usize)
        };
        let list_rect = [0.0, top, w, bottom - top];
        let list_key = horizon_client::widgets::key_of(&("stats/list", self.stats_ui.tab));
        let tr = super::screen_info::tr;
        // GeneralStatisticsList and MobsStatisticsList: ObjectSelectionLists
        // narrating the hovered row (none is selected).
        let object_list = |narration: &dyn Fn(usize) -> String| {
            horizon_client::widgets::narratable(|| {
                use horizon_client::widgets::{ListKind, Narratable, Row};
                let hovered = hovered_row.map(|i| Row::new(i, narration(i)));
                Narratable::list(ListKind::Object, rows, hovered, None, list_rect).key(list_key)
            });
        };
        // The hovered item's tooltip, drawn over the list.
        let mut tip = None;
        match self.stats_ui.tab {
            0 => {
                let mut sorted: Vec<(Text, String)> = CUSTOM
                    .iter()
                    .map(|(k, f)| {
                        let label = Text::translate(&format!("stat.minecraft.{k}"), &[]);
                        let v = self.stats.get("custom", &format!("minecraft:{k}"));
                        (label, format(*f, v))
                    })
                    .collect();
                sorted.sort_by_key(|(l, _)| l.plain());
                object_list(&|i| {
                    let (label, value) = &sorted[i];
                    tr("narrator.select", &[&format!("{} {value}", label.plain())])
                });
                for (i, (label, value)) in sorted.iter().enumerate() {
                    let y = row_y(i);
                    if !visible(y) {
                        continue;
                    }
                    let color = if i % 2 == 0 { u32::MAX } else { 0xFFBA_BABA };
                    let ty = y + (row_h - 8.0) / 2.0;
                    batch.text_styled(atlas, label, left + 2.0, ty.floor(), color, true);
                    let vw = HudBatch::text_width(atlas, value);
                    batch.text(
                        atlas,
                        value,
                        left + 280.0 - vw - 4.0,
                        ty.floor(),
                        color,
                        true,
                    );
                }
            }
            1 => {
                let items = self.stat_items();
                if items.is_empty() {
                    return self.draw_stats_footer(batch, w, h);
                }
                // ItemStatisticsList, a ContainerObjectSelectionList: the
                // header's sort buttons and each row's item.
                horizon_client::widgets::narratable(|| {
                    horizon_client::widgets::Narratable::container_list(
                        rows,
                        hovered_row,
                        list_rect,
                    )
                    .key(list_key)
                });
                // The header row of sort buttons.
                let y = row_y(0);
                if visible(y) {
                    for (c, (ty, sprite)) in COLUMNS.iter().enumerate() {
                        let x = left + 75.0 + 40.0 * c as f32 - 18.0;
                        let hover = ui::hovered(self.cursor, x, y + 1.0, 18.0, 18.0);
                        // StatSortButton: an ImageButton named by its stat
                        // type, which is its tooltip too.
                        horizon_client::widgets::narratable(|| {
                            let name = tr(&format!("stat_type.minecraft.{ty}"), &[]);
                            horizon_client::widgets::Narratable::button(
                                name.clone(),
                                [x, y + 1.0, 18.0, 18.0],
                                true,
                            )
                            .hint(name)
                            .key(horizon_client::widgets::key_of(&("stats/sort", c)))
                            .child_of(list_key, 0)
                        });
                        let bg = if hover {
                            "gui/sprites/statistics/header"
                        } else {
                            "gui/sprites/container/slot"
                        };
                        batch.sprite(atlas, bg, x, y + 1.0);
                        batch.sprite(atlas, sprite, x, y + 1.0);
                        if let Some((sc, order)) = self.stats_ui.sort
                            && sc == c
                        {
                            let s = if order < 0 {
                                "gui/sprites/statistics/sort_down"
                            } else {
                                "gui/sprites/statistics/sort_up"
                            };
                            batch.sprite(atlas, s, x, y + 1.0);
                        }
                    }
                }
                for (i, item) in items.iter().enumerate() {
                    let y = row_y(i + 1);
                    if !visible(y) {
                        continue;
                    }
                    let stack = ItemStack::new(item, 1);
                    // ItemRowWidget, an ItemDisplayWidget: "Item: <name>".
                    horizon_client::widgets::narratable(|| {
                        let names = |id: &str| self.pack.names.display_item(id);
                        let name = horizon_client::stations::hover_name(&names, &stack);
                        horizon_client::widgets::Narratable::text(
                            tr("narration.item", &[&name]),
                            [left + 1.0, y + 1.0, 18.0, 18.0],
                        )
                        .key(horizon_client::widgets::key_of(&("stats/item", item)))
                        .child_of(list_key, i + 1)
                        .without_tooltip()
                    });
                    batch.sprite(
                        &self.gui.atlas,
                        "gui/sprites/container/slot",
                        left + 1.0,
                        y + 1.0,
                    );
                    self.draw_item(batch, &stack, left + 2.0, y + 2.0);
                    // `ItemRowWidget.extractTooltip`: the item's tooltip at the slot's corner.
                    if ui::hovered(self.cursor, left + 1.0, y + 1.0, 18.0, 18.0) {
                        tip = Some((stack.clone(), left + 19.0, y + 19.0));
                    }
                    let atlas = &self.gui.atlas;
                    let color = if (i + 1) % 2 == 0 {
                        u32::MAX
                    } else {
                        0xFFBA_BABA
                    };
                    for (c, (ty, _)) in COLUMNS.iter().enumerate() {
                        let x = left + 75.0 + 40.0 * c as f32;
                        let block = c == 0
                            && self
                                .res
                                .items()
                                .get(item)
                                .is_some_and(|i| i.block.is_some());
                        let text = if c == 0 && !block {
                            Text::translate("stats.none", &[]).plain()
                        } else {
                            format(Format::Default, self.stats.get(ty, item))
                        };
                        let tw = HudBatch::text_width(atlas, &text);
                        batch.text(atlas, &text, x - tw, (y + 7.0).floor(), color, true);
                    }
                }
            }
            _ => {
                let mobs = self.stat_mobs();
                object_list(&|i| {
                    let (_, kills, killed_by, _) = self.mob_row(&mobs[i]);
                    let both = horizon_client::widgets::join_for_narration(&[
                        &kills.plain(),
                        &killed_by.plain(),
                    ]);
                    tr("narrator.select", &[&both])
                });
                for (i, mob) in mobs.iter().enumerate() {
                    let y = row_y(i);
                    if !visible(y) {
                        continue;
                    }
                    let (name, k, b, (ck, cb)) = self.mob_row(mob);
                    batch.text_styled(atlas, &name, left + 2.0, y + 1.0, u32::MAX, true);
                    batch.text_styled(atlas, &k, left + 12.0, y + 10.0, ck, true);
                    batch.text_styled(atlas, &b, left + 12.0, y + 19.0, cb, true);
                }
            }
        }
        let atlas = &self.gui.atlas;
        ui::scrollbar(
            batch,
            atlas,
            Self::stats_bar(w),
            top,
            bottom,
            content,
            self.stats_ui.scroll.amount,
        );
        self.draw_stats_footer(batch, w, h);
        if let Some((stack, x, y)) = tip {
            self.draw_stack_tooltip(batch, &stack, x, y);
        }
        // The empty tab's Tooltip.create(gui.stats.none_found).
        if tab_tip {
            let atlas = &self.gui.atlas;
            let tip = Text::translate("gui.stats.none_found", &[]);
            // Tooltip.splitTooltip: font.split(message, 170).
            ui::tooltip(batch, atlas, &atlas.font.split(&tip, 170.0), self.cursor);
        }
    }
    fn draw_stats_footer(&self, batch: &mut HudBatch, w: f32, h: f32) {
        let atlas = &self.gui.atlas;
        let (bx, by) = ((w / 2.0 - 100.0).floor(), h - FOOTER + 6.0);
        let hover = ui::hovered(self.cursor, bx, by, 200.0, 20.0);
        ui::button(
            batch,
            atlas,
            bx,
            by,
            200.0,
            20.0,
            &Text::translate("gui.done", &[]),
            true,
            hover,
        );
    }
    pub(super) fn stats_click(&mut self, size: [u32; 2]) {
        // A press on the scrollbar holds it and clicks nothing else.
        if self.stats_press_scrollbar(size) {
            return;
        }
        let b = HudBatch::new(size);
        let (w, h) = (b.width, b.height);
        let (x0, tw) = tabs(w);
        for i in 0..3 {
            if ui::hovered(self.cursor, x0 + tw * i as f32, 0.0, tw, TAB_H)
                && self.stats_ui.tab != i
                && self.stats_tab_active(i)
            {
                self.stats_ui.tab = i;
                self.stats_ui.scroll.amount = 0.0;
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                return;
            }
        }
        if ui::hovered(
            self.cursor,
            (w / 2.0 - 100.0).floor(),
            h - FOOTER + 6.0,
            200.0,
            20.0,
        ) {
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.screen = Screen::Paused;
            return;
        }
        if self.stats_ui.tab == 1 {
            let left = ((w - 280.0) / 2.0).floor();
            let y = TAB_H + 4.0 - self.stats_ui.scroll.amount;
            for c in 0..COLUMNS.len() {
                let x = left + 75.0 + 40.0 * c as f32 - 18.0;
                if ui::hovered(self.cursor, x, y + 1.0, 18.0, 18.0) {
                    // sortByColumn: descending, ascending, off.
                    self.stats_ui.sort = match self.stats_ui.sort {
                        Some((sc, -1)) if sc == c => Some((c, 1)),
                        Some((sc, _)) if sc == c => None,
                        _ => Some((c, -1)),
                    };
                    self.audio
                        .play("minecraft:ui.button.click", None, 0.25, 1.0);
                }
            }
        }
    }
    pub(super) fn stats_scroll(&mut self, amount: f32) {
        let (row_h, _) = self.stats_rows();
        let h = self.gui_size().1;
        let content = self.stats_content();
        self.stats_ui
            .scroll
            .wheel(amount, row_h / 2.0, TAB_H, h - FOOTER, content);
    }
    /// A press on the statistics list's scrollbar holds it
    /// (`AbstractSelectionList.updateScrolling`).
    pub(super) fn stats_press_scrollbar(&mut self, size: [u32; 2]) -> bool {
        let b = HudBatch::new(size);
        let (w, h) = (b.width, b.height);
        let content = self.stats_content();
        self.stats_ui
            .scroll
            .press(self.cursor, Self::stats_bar(w), TAB_H, h - FOOTER, content)
    }
    /// The pointer moved while the scrollbar is held.
    pub(super) fn stats_drag(&mut self) {
        if !self.stats_ui.scroll.held() {
            return;
        }
        let h = self.gui_size().1;
        let content = self.stats_content();
        self.stats_ui
            .scroll
            .drag(self.cursor.1, TAB_H, h - FOOTER, content);
    }
    /// The button came up: the scrollbar lets go.
    pub(super) fn stats_release(&mut self) {
        self.stats_ui.scroll.release();
    }
}
