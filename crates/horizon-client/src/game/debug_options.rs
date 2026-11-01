//! `DebugOptionsScreen` (F3+F6): every debug entry with Off / In Overlay /
//! Always buttons, a search box over them, the two built-in profiles and the
//! debug overlay's GUI scale. The statuses live in `debug-profile.json`
//! (`horizon_client::debug_entries`).
//! https://mcsrc.dev/2/26.3/net/minecraft/client/gui/screens/debug/DebugOptionsScreen
use super::{Game, Screen};
use horizon_client::{
    debug_entries::{Category, Entry, Profile, Status, search},
    hud::HudBatch,
    options_file::V,
    text::Text,
    ui::{self, EditBox, Scroll},
};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

/// `HeaderAndFooterLayout(this, 61, 33)`.
const HEADER: f32 = 61.0;
const FOOTER: f32 = 33.0;
/// `OptionList.getRowWidth` and `ITEM_HEIGHT`.
const ROW_WIDTH: f32 = 350.0;
const ITEM: f32 = 20.0;
/// `OptionEntry.BUTTON_WIDTH`, at the CycleButtons' 16 pixels high.
const BUTTON: f32 = 60.0;
/// The footer: two profile buttons, the GUI scale and Done, 4 pixels apart.
const FOOTER_BUTTONS: [f32; 4] = [120.0, 120.0, 110.0, 60.0];
/// `debug.options.warning`'s colour (-2142128) and the buttons' text colours.
const WARNING: u32 = 0xDF_5050;
const UNSELECTED: u32 = 0xBA_BABA;
const OVERLAY: u32 = 0xFF_FF55;
const NOT_ALLOWED: u32 = 0x80_8080;
/// `EditBox.SEARCH_HINT_STYLE`: `ChatFormatting.GRAY`.
const HINT: u32 = 0xAA_AAAA;

/// The open screen: its search box and the list's scroll.
pub(super) struct DebugOptionsUi {
    search: EditBox,
    scroll: Scroll,
}

impl DebugOptionsUi {
    pub(super) fn search_focused(&self) -> bool {
        self.search.focused
    }
}

/// One list row (`AbstractOptionEntry`).
enum Row {
    Category(Category),
    Option(&'static Entry),
}

/// `OptionList.updateSearch`: a category row before each category's entries.
fn rows(value: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut current = None;
    for entry in search(value) {
        if current != Some(entry.category) {
            rows.push(Row::Category(entry.category));
            current = Some(entry.category);
        }
        rows.push(Row::Option(entry));
    }
    rows
}

/// `AbstractSelectionList.contentHeight`.
fn content_height(n: usize) -> f32 {
    n as f32 * ITEM + 4.0
}
/// `getRowLeft`: the list spans the screen, its rows centred.
fn row_left(w: f32) -> f32 {
    (w / 2.0).floor() - ROW_WIDTH / 2.0
}
/// The three buttons' left edges (Off, In Overlay, Always) for a row whose
/// content starts at `x` (`extractContent`).
fn button_xs(content_x: f32) -> [f32; 3] {
    let start = content_x + ROW_WIDTH - 4.0 - 3.0 * BUTTON;
    [start, start + BUTTON, start + 2.0 * BUTTON]
}
/// The footer buttons' left edges and their row's top.
fn footer(w: f32, h: f32) -> ([f32; 4], f32) {
    let total: f32 = FOOTER_BUTTONS.iter().sum::<f32>() + 3.0 * 4.0;
    let mut x = ((w - total) / 2.0).floor();
    let mut xs = [0.0; 4];
    for (slot, width) in xs.iter_mut().zip(FOOTER_BUTTONS) {
        *slot = x;
        x += width + 4.0;
    }
    (xs, h - FOOTER + ((FOOTER - 20.0) / 2.0).floor())
}
/// The three buttons' statuses and texts, in their order on the row.
const BUTTONS: [(Status, &str, u32); 3] = [
    (Status::Never, "options.off", 0xFF_FFFF),
    (Status::InOverlay, "debug.entry.overlay", OVERLAY),
    (Status::AlwaysOn, "debug.entry.always", WARNING),
];

impl Game<'_> {
    /// `KeyboardHandler`'s F3+F6: closes the screen if it is open, else opens
    /// it over the game or the pause menu (`canInterruptScreen`).
    pub(super) fn toggle_debug_options(&mut self) {
        match self.screen {
            Screen::DebugOptions => self.close_debug_options(),
            Screen::Playing | Screen::Paused if !self.screen_over_playing() => {
                let mut search = EditBox::new(0.0, 0.0, (ROW_WIDTH / 3.0).floor(), 20.0);
                search.max_length = 32;
                self.debug_options = Some(DebugOptionsUi {
                    search,
                    scroll: Scroll::default(),
                });
                self.screen = Screen::DebugOptions;
                self.pause_menu = true;
                self.capture(false);
                self.mining = None;
            }
            _ => {}
        }
    }
    /// `Screen.onClose`: back to the game.
    pub(super) fn close_debug_options(&mut self) {
        self.debug_options = None;
        self.screen = Screen::Playing;
        self.capture(true);
    }
    /// The header's title row and warning (`init`'s vertical layout,
    /// centred in the header): the title's x, the row's top, the warning's
    /// lines with their y.
    fn debug_options_header(&self, w: f32) -> (f32, f32, Vec<(Text, f32)>) {
        let font = &self.gui.atlas.font;
        let title = Text::translate("debug.options.title", &[]);
        let warning = Text::translate("debug.options.warning", &[]).with_color(WARNING);
        let lines = font.split(&warning, ROW_WIDTH);
        let height = 20.0 + 8.0 + lines.len() as f32 * 9.0;
        let top = ((HEADER - height) / 2.0).floor();
        let third = (ROW_WIDTH / 3.0).floor();
        let tw = font.width_text(&title);
        let row = third + 8.0 + tw + 8.0 + third;
        let title_x = ((w - row) / 2.0).floor() + third + 8.0;
        let lines = lines
            .into_iter()
            .enumerate()
            .map(|(i, line)| (line, top + 28.0 + i as f32 * 9.0))
            .collect();
        (title_x, top, lines)
    }
    /// `debugGuiScale`'s button: `GUI Scale: Unchanged`, `Auto` or the scale.
    fn debug_gui_scale_label(&self) -> Text {
        let value = match self.settings.file.int("debugGuiScale") {
            -1 => Text::translate("options.debugGuiScale.unchanged", &[]),
            0 => Text::translate("options.guiScale.auto", &[]),
            n => Text::literal(n.to_string()),
        };
        Text::translate(
            "options.generic_value",
            &[Text::translate("options.guiScale", &[]), value],
        )
    }
    /// `ClampingLazyMaxIntRange(-1, window.calculateScale(0, unicode))` as a
    /// CycleButton: the next value, or the previous one with Shift.
    fn cycle_debug_gui_scale(&mut self, back: bool) {
        let size = self.renderer.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        let max = horizon_client::window::calculate_scale(size, 0, self.settings.force_unicode);
        let current = self.settings.file.int("debugGuiScale").clamp(-1, max);
        let next = match (back, current) {
            (false, c) if c >= max => -1,
            (false, c) => c + 1,
            (true, -1) => max,
            (true, c) => c - 1,
        };
        self.settings.file.set("debugGuiScale", V::Int(next));
        self.settings.save(&Self::options_path());
    }
    pub(super) fn draw_debug_options(&mut self, batch: &mut HudBatch) {
        let Some(mut s) = self.debug_options.take() else {
            return;
        };
        let (w, h) = (batch.width, batch.height);
        // extractBlurredBackground: the debug overlay goes under the screen.
        let size = self
            .renderer
            .as_ref()
            .map_or([w as u32, h as u32], |r| [r.config.width, r.config.height]);
        self.draw_debug_overlay_under(batch, size);
        let atlas = &self.gui.atlas;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let reduced = self.reduced_debug_info();
        let rows = rows(&s.search.value);
        let (top, bottom) = (HEADER, h - FOOTER);
        let content = content_height(rows.len());
        s.scroll.clamp(top, bottom, content);
        // The list: background, rows clipped to it, separators, scrollbar.
        ui::list_background(batch, atlas, top, bottom, true);
        let first = batch.vertices.len();
        let left = row_left(w);
        let inside = self.cursor.1 >= top && self.cursor.1 < bottom;
        let mut tooltip = None;
        for (i, row) in rows.iter().enumerate() {
            let y = top + 2.0 - s.scroll.amount.trunc() + i as f32 * ITEM;
            if y + ITEM < top || y > bottom {
                continue;
            }
            let (cx, cy) = (left + 2.0, y + 2.0);
            match row {
                Row::Category(category) => {
                    let label = Text::translate(category.translation_key(), &[])
                        .bold()
                        .underlined();
                    let lw = atlas.font.width_text(&label);
                    let lx = (cx + (ROW_WIDTH - 4.0) / 2.0 - (lw + 8.0) / 2.0).floor() + 4.0;
                    batch.text_styled(atlas, &label, lx, cy + 4.0, u32::MAX, true);
                }
                Row::Option(entry) => {
                    let allowed = entry.reduced || !reduced;
                    let name = if allowed {
                        Text::literal(entry.path)
                    } else {
                        Text::literal(entry.path).italic().with_color(NOT_ALLOWED)
                    };
                    batch.text_styled(atlas, &name, cx, cy + 5.0, u32::MAX, true);
                    let xs = button_xs(cx);
                    let status = self.debug_entries.status(entry.path);
                    for ((status_of, key, color), x) in BUTTONS.into_iter().zip(xs) {
                        let selected = status == status_of;
                        let label = Text::translate(key, &[]).with_color(if selected {
                            color
                        } else {
                            UNSELECTED
                        });
                        let hover = inside && ui::hovered(self.cursor, x, cy, BUTTON, 16.0);
                        ui::button(batch, atlas, x, cy, BUTTON, 16.0, &label, !selected, hover);
                    }
                    if !allowed && inside && ui::hovered(self.cursor, cx, y, xs[0] - cx, ITEM) {
                        tooltip = Some(Text::translate("debug.options.notAllowed.tooltip", &[]));
                    }
                }
            }
        }
        batch.clip_since(first, 0.0, top, w, bottom);
        ui::separators(batch, atlas, top, bottom, true);
        ui::scrollbar(
            batch,
            atlas,
            left + ROW_WIDTH + 8.0,
            top,
            bottom,
            content,
            s.scroll.amount.trunc(),
        );
        // The header: title, search box, warning.
        let (title_x, header_top, lines) = self.debug_options_header(w);
        let title = Text::translate("debug.options.title", &[]);
        batch.text_styled(atlas, &title, title_x, header_top + 5.0, u32::MAX, true);
        s.search.x = title_x + atlas.font.width_text(&title) + 8.0;
        s.search.y = header_top;
        s.search.draw(batch, atlas, None);
        if s.search.value.is_empty() {
            let hint = Text::translate("debug.options.search", &[])
                .with_color(HINT)
                .italic();
            batch.text_styled(
                atlas,
                &hint,
                s.search.text_x(),
                s.search.text_y(),
                u32::MAX,
                true,
            );
        }
        for (line, y) in &lines {
            let lw = atlas.font.width_text(line);
            batch.text_styled(atlas, line, ((w - lw) / 2.0).floor(), *y, u32::MAX, true);
        }
        // The footer.
        let (xs, fy) = footer(w, h);
        let labels = [
            Text::translate(Profile::Default.translation_key(), &[]),
            Text::translate(Profile::Performance.translation_key(), &[]),
            self.debug_gui_scale_label(),
            Text::translate("gui.done", &[]),
        ];
        let active = [
            !self.debug_entries.is_using_profile(Profile::Default),
            !self.debug_entries.is_using_profile(Profile::Performance),
            true,
            true,
        ];
        for (i, label) in labels.iter().enumerate() {
            let hover = ui::hovered(self.cursor, xs[i], fy, FOOTER_BUTTONS[i], 20.0);
            ui::button(
                batch,
                atlas,
                xs[i],
                fy,
                FOOTER_BUTTONS[i],
                20.0,
                label,
                active[i],
                hover,
            );
            if i == 2 && hover {
                tooltip = Some(Text::translate("options.debugGuiScale.tooltip", &[]));
            }
        }
        if let Some(tooltip) = tooltip {
            let lines = atlas.font.split(&tooltip, 200.0);
            ui::tooltip(batch, atlas, &lines, self.cursor);
        }
        self.debug_options = Some(s);
    }
    /// A left click: a status button, the search box, a footer button.
    pub(super) fn debug_options_click(&mut self) {
        let Some(mut s) = self.debug_options.take() else {
            return;
        };
        let (w, h) = self.gui_size();
        let (top, bottom) = (HEADER, h - FOOTER);
        let rows = rows(&s.search.value);
        let left = row_left(w);
        // A press on the scrollbar holds it and clicks nothing else.
        if s.scroll.press(
            self.cursor,
            left + ROW_WIDTH + 8.0,
            top,
            bottom,
            content_height(rows.len()),
        ) {
            self.debug_options = Some(s);
            return;
        }
        let shift =
            self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        s.search.focused = ui::hovered(
            self.cursor,
            s.search.x,
            s.search.y,
            s.search.width,
            s.search.height,
        );
        if s.search.focused {
            s.search.click(&self.gui.atlas, self.cursor.0, shift, false);
        }
        let inside = self.cursor.1 >= top && self.cursor.1 < bottom;
        let mut clicked = None;
        if inside {
            for (i, row) in rows.iter().enumerate() {
                let Row::Option(entry) = row else { continue };
                let cy = top + 2.0 - s.scroll.amount.trunc() + i as f32 * ITEM + 2.0;
                let status = self.debug_entries.status(entry.path);
                for ((status_of, _, _), x) in BUTTONS.into_iter().zip(button_xs(left + 2.0)) {
                    if status != status_of && ui::hovered(self.cursor, x, cy, BUTTON, 16.0) {
                        clicked = Some((entry.path, status_of));
                    }
                }
            }
        }
        self.debug_options = Some(s);
        if let Some((path, status)) = clicked {
            // OptionEntry.setValue.
            self.audio.play_button_click();
            self.debug_entries.set_status(path, status);
            return;
        }
        let (xs, fy) = footer(w, h);
        let Some(i) =
            (0..4).find(|&i| ui::hovered(self.cursor, xs[i], fy, FOOTER_BUTTONS[i], 20.0))
        else {
            return;
        };
        match i {
            0 | 1 => {
                let profile = if i == 0 {
                    Profile::Default
                } else {
                    Profile::Performance
                };
                if self.debug_entries.is_using_profile(profile) {
                    return;
                }
                self.audio.play_button_click();
                self.debug_entries.load_profile(profile);
            }
            2 => {
                self.audio.play_button_click();
                self.cycle_debug_gui_scale(shift);
            }
            _ => {
                self.audio.play_button_click();
                self.close_debug_options();
            }
        }
    }
    /// The mouse wheel over the list (`scrollRate`: half a row).
    pub(super) fn debug_options_scroll(&mut self, delta: f32) {
        let (_, h) = self.gui_size();
        if let Some(s) = &mut self.debug_options {
            let content = content_height(rows(&s.search.value).len());
            s.scroll
                .wheel(delta, ITEM / 2.0, HEADER, h - FOOTER, content);
        }
    }
    /// The pointer moved while the list's scrollbar is held.
    pub(super) fn debug_options_drag(&mut self) {
        let (_, h) = self.gui_size();
        let y = self.cursor.1;
        if let Some(s) = &mut self.debug_options
            && s.scroll.held()
        {
            let content = content_height(rows(&s.search.value).len());
            s.scroll.drag(y, HEADER, h - FOOTER, content);
        }
    }
    /// The button came up: the scrollbar lets go.
    pub(super) fn debug_options_release(&mut self) {
        if let Some(s) = &mut self.debug_options {
            s.scroll.release();
        }
    }
    /// A key the debug keys left: Escape closes, the rest edits the search.
    pub(super) fn debug_options_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        if key == KeyCode::Escape {
            self.close_debug_options();
            return;
        }
        let shortcut = self.edit_shortcut(event);
        let ctrl = self.keys.contains(&KeyCode::ControlLeft);
        let shift = self.keys.contains(&KeyCode::ShiftLeft);
        let types_text = self.types_text();
        let Some(s) = &mut self.debug_options else {
            return;
        };
        if !s.search.focused {
            return;
        }
        if let Some(shortcut) = shortcut {
            s.search.shortcut(shortcut, &mut self.clipboard);
        } else if !s.search.key(key, ctrl, shift)
            && types_text
            && let Some(text) = event.text.as_deref()
        {
            self.debug_options_type(text);
        }
    }
    /// Characters typed into the search box (`EditBox.charTyped`).
    pub(super) fn debug_options_type(&mut self, typed: &str) {
        let allowed: String = typed
            .chars()
            .filter(|&c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}')
            .collect();
        if let Some(s) = &mut self.debug_options
            && s.search.focused
            && !allowed.is_empty()
        {
            s.search.insert_text(&allowed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_put_a_category_before_each_section() {
        let all = rows("");
        assert_eq!(all.len(), 48);
        assert!(matches!(all[0], Row::Category(Category::Text)));
        let renderer = all
            .iter()
            .position(|r| matches!(r, Row::Category(Category::Renderer)))
            .unwrap();
        assert!(matches!(all[renderer + 1], Row::Option(e) if e.path == "3d_crosshair"));
        // A search inside one section shows only its heading.
        let hitboxes = rows("hitbox");
        assert_eq!(hitboxes.len(), 2);
        assert!(matches!(hitboxes[0], Row::Category(Category::Renderer)));
    }

    #[test]
    fn the_layout_matches_vanilla_at_854_by_480() {
        // 854x480 at GUI scale 2 is 427x240.
        assert_eq!(row_left(427.0), 38.0);
        assert_eq!(button_xs(40.0), [206.0, 266.0, 326.0]);
        assert_eq!(footer(427.0, 240.0), ([2.0, 126.0, 250.0, 364.0], 213.0));
    }
}
