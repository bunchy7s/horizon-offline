//! `SocialInteractionsScreen`: the players of the server in three tabs (All, Hidden, Blocked),
//! a search box, and a row for each with the button that hides or shows its chat. The
//! Blocked tab and each row's report button belong to Mojang's services, which Horizon does
//! not offer: the first lists no one, the second is the disabled button vanilla shows when
//! reporting is off.
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    hud_layout, tab_list,
    text::Text,
    ui,
};

pub const TITLE: &str = "gui.socialInteractions.title";
/// `SocialInteractionsScreen`'s page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    All,
    Hidden,
    Blocked,
}
/// One `PlayerEntry`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u128,
    pub name: String,
    pub skin: String,
    pub hidden: bool,
    /// Not in the level any more (`PlayerEntry.isRemoved`): a hidden player who left.
    pub removed: bool,
    pub local: bool,
}
/// What the screen shows (game/social.rs fills it).
#[derive(Clone, Debug, Default)]
pub struct SocialView {
    pub page: Page,
    pub search: String,
    /// The rows of the page, sorted and filtered as `SocialInteractionsPlayerList` does.
    pub entries: Vec<Entry>,
    /// `serverLabel`: the server's name and the players online.
    pub server_label: Text,
    pub scroll: f32,
}
/// What a click does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Page(Page),
    Hide(u128),
    Show(u128),
    Done,
}

const IMAGE_WIDTH: f32 = 238.0;
const ROW_WIDTH: f32 = 220.0;
const ITEM_HEIGHT: f32 = 36.0;
const LIST_START: f32 = 88.0;
const BG_FILL: u32 = 0xFF4A4A4A;
const BG_FILL_REMOVED: u32 = 0xFF303030;
const STATUS_COLOR: u32 = 0x8CFFFFFF;
const WHITE: u32 = u32::MAX;

fn margin_x(w: f32) -> f32 {
    ((w - IMAGE_WIDTH) / 2.0).floor()
}
/// `windowHeight`.
fn window_height(h: f32) -> f32 {
    (h - 128.0 - 16.0).max(52.0)
}
/// `listEnd`.
fn list_end(h: f32) -> f32 {
    80.0 + window_height(h) - 8.0
}
fn row_left(w: f32) -> f32 {
    ((w - ROW_WIDTH) / 2.0).floor()
}
/// `SocialInteractionsPlayerList.sortPlayerEntries`: the local player first, then names that
/// start with a letter, a digit or `_`, then the rest, each by name ignoring case.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by_key(|e| {
        let plain = e
            .name
            .chars()
            .next()
            .is_some_and(|c| c == '_' || c.is_ascii_alphanumeric());
        (!e.local, !plain, e.name.to_lowercase())
    });
}
/// The most the list scrolls: its rows less its height.
pub fn max_scroll(h: f32, rows: usize) -> f32 {
    // `AbstractSelectionList.contentHeight`: the rows and 4px.
    (rows as f32 * ITEM_HEIGHT + 4.0 - (list_end(h) - LIST_START)).max(0.0)
}
fn tabs(w: f32) -> [(f32, Page); 3] {
    let width = (ROW_WIDTH / 3.0).floor();
    let (left, right) = (row_left(w), row_left(w) + ROW_WIDTH);
    [
        (left, Page::All),
        (((left + right - width) / 2.0 + 1.0).floor(), Page::Hidden),
        (right - width + 1.0, Page::Blocked),
    ]
}
fn tab_label(page: Page, selected: bool) -> Text {
    let key = match page {
        Page::All => "gui.socialInteractions.tab_all",
        Page::Hidden => "gui.socialInteractions.tab_hidden",
        Page::Blocked => "gui.socialInteractions.tab_blocked",
    };
    let label = Text::translate(key, &[]);
    if selected { label.underlined() } else { label }
}
/// A row's content rectangle (`Entry.getContentX/Y/Width/Height`: 2px in from the row), `y` the
/// row's top.
fn row_rect(w: f32, y: f32) -> [f32; 4] {
    [
        row_left(w) + 2.0,
        y + 2.0,
        ROW_WIDTH - 4.0,
        ITEM_HEIGHT - 4.0,
    ]
}
/// `PlayerEntry.extractContent`: the hide or show button, and the report button after it.
fn row_buttons(w: f32, y: f32) -> ([f32; 2], [f32; 2]) {
    let [x, y, width, height] = row_rect(w, y);
    let hide = [
        x + (width - 20.0 - 4.0) - 20.0 - 4.0,
        y + (height - 20.0) / 2.0,
    ];
    let report = [x + (width - 20.0 - 4.0), y + (height - 20.0) / 2.0];
    (hide, report)
}
fn status(e: &Entry) -> Option<Text> {
    let key = match (e.hidden, e.removed) {
        (true, true) => "gui.socialInteractions.status_hidden_offline",
        (true, false) => "gui.socialInteractions.status_hidden",
        (false, true) => "gui.socialInteractions.status_offline",
        (false, false) => return None,
    };
    Some(Text::translate(key, &[]).italic())
}
/// The rows that show: the first one at `getY() + 2 - scroll` (`getFirstEntryY`).
fn visible(view: &SocialView, h: f32) -> impl Iterator<Item = (usize, f32)> + '_ {
    let end = list_end(h);
    view.entries
        .iter()
        .enumerate()
        .map(move |(i, _)| (i, LIST_START + 2.0 - view.scroll + i as f32 * ITEM_HEIGHT))
        .filter(move |&(_, y)| y + ITEM_HEIGHT > LIST_START + 4.0 && y < end)
}
/// The search box's rectangle.
pub fn search_box(w: f32) -> [f32; 4] {
    [margin_x(w) + 28.0, 74.0, 200.0, 15.0]
}
/// Done's rectangle (`HeaderAndFooterLayout`'s footer).
fn done(w: f32, h: f32) -> [f32; 4] {
    [(w / 2.0).floor() - 100.0, h - 27.0, 200.0, 20.0]
}
/// What is under `cursor`, if a click would do something.
pub fn click(size: (f32, f32), cursor: (f32, f32), view: &SocialView) -> Option<Action> {
    let (w, h) = size;
    for (x, page) in tabs(w) {
        if ui::hovered(cursor, x, 45.0, (ROW_WIDTH / 3.0).floor(), 20.0) {
            return Some(Action::Page(page));
        }
    }
    let [dx, dy, dw, dh] = done(w, h);
    if ui::hovered(cursor, dx, dy, dw, dh) {
        return Some(Action::Done);
    }
    if cursor.1 < LIST_START || cursor.1 >= list_end(h) {
        return None;
    }
    for (i, y) in visible(view, h) {
        let e = &view.entries[i];
        if e.local {
            continue;
        }
        let (hide, _) = row_buttons(w, y);
        if ui::hovered(cursor, hide[0], hide[1], 20.0, 20.0) {
            return Some(if e.hidden {
                Action::Show(e.id)
            } else {
                Action::Hide(e.id)
            });
        }
    }
    None
}
/// The tooltip of what the cursor is over.
fn tooltip(size: (f32, f32), cursor: (f32, f32), view: &SocialView) -> Option<Text> {
    let (w, h) = size;
    if cursor.1 < LIST_START || cursor.1 >= list_end(h) {
        return None;
    }
    for (i, y) in visible(view, h) {
        let e = &view.entries[i];
        if e.local {
            continue;
        }
        let (hide, report) = row_buttons(w, y);
        if ui::hovered(cursor, hide[0], hide[1], 20.0, 20.0) {
            let key = if e.hidden {
                "gui.socialInteractions.tooltip.show"
            } else {
                "gui.socialInteractions.tooltip.hide"
            };
            return Some(Text::translate(key, &[]));
        }
        if ui::hovered(cursor, report[0], report[1], 20.0, 20.0) {
            return Some(Text::translate(
                "gui.socialInteractions.tooltip.report.disabled",
                &[],
            ));
        }
    }
    None
}
pub fn draw(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    cursor: (f32, f32),
    view: &SocialView,
    search: &mut ui::EditBox,
    in_world: bool,
) {
    let (w, h) = (batch.width, batch.height);
    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
    let title = Text::translate(TITLE, &[]);
    let tw = atlas.font.width_text(&title);
    batch.text_styled(atlas, &title, ((w - tw) / 2.0).floor(), 15.0, WHITE, true);
    // `extractBackground`: the panel and the search icon.
    let mx = margin_x(w) + 3.0;
    ui::blit_sprite(
        batch,
        atlas,
        "gui/sprites/social_interactions/background",
        mx,
        64.0,
        236.0,
        window_height(h) + 16.0,
        WHITE,
    );
    batch.sprite_scaled(
        atlas,
        "gui/sprites/icon/search",
        mx + 10.0,
        76.0,
        12.0,
        12.0,
        WHITE,
    );
    batch.text_styled(
        atlas,
        &view.server_label,
        margin_x(w) + 8.0,
        35.0,
        WHITE,
        true,
    );
    for (x, page) in tabs(w) {
        ui::button(
            batch,
            atlas,
            x,
            45.0,
            (ROW_WIDTH / 3.0).floor(),
            20.0,
            &tab_label(page, view.page == page),
            true,
            ui::hovered(cursor, x, 45.0, (ROW_WIDTH / 3.0).floor(), 20.0),
        );
    }
    // The search box: an `EditBox` with the focus from the start (`setInitialFocus`), so its
    // hint, shown only while unfocused, never shows.
    let [sx, sy, sw, sh] = search_box(w);
    (search.x, search.y, search.width, search.height) = (sx, sy, sw, sh);
    search.max_length = 16;
    search.text_color = WHITE;
    search.focused = true;
    search.draw(batch, atlas, None);
    // The rows, clipped to the list.
    let start = batch.vertices.len();
    let mut faces = Vec::new();
    for (i, y) in visible(view, h) {
        let e = &view.entries[i];
        let [x, _, width, height] = row_rect(w, y);
        let (fill, text_y) = match status(e) {
            None => (BG_FILL, y + (height - 9.0) / 2.0),
            Some(_) => (BG_FILL_REMOVED, y + (height - 18.0) / 2.0),
        };
        batch.fill(atlas, x, y, width, height, fill);
        let (skin_x, skin_y) = (x + 4.0, y + ((height - 24.0) / 2.0).floor());
        tab_list::face(
            &mut faces,
            &e.skin,
            skin_x as i32,
            skin_y as i32,
            24,
            true,
            false,
            WHITE,
        );
        if e.removed {
            faces.push(hud_layout::fill(
                skin_x as i32,
                skin_y as i32,
                skin_x as i32 + 24,
                skin_y as i32 + 24,
                0xBE000000,
            ));
        }
        let text_x = skin_x + 24.0 + 4.0;
        batch.text(atlas, &e.name, text_x, text_y.floor(), WHITE, false);
        if let Some(s) = status(e) {
            batch.text_styled(
                atlas,
                &s,
                text_x,
                (text_y + 12.0).floor(),
                STATUS_COLOR,
                false,
            );
        }
        if !e.local {
            let (hide, report) = row_buttons(w, y);
            let sprite = |base: &str, hover: bool| {
                format!(
                    "gui/sprites/social_interactions/{base}{}",
                    if hover { "_highlighted" } else { "" }
                )
            };
            let hovered = |p: [f32; 2]| ui::hovered(cursor, p[0], p[1], 20.0, 20.0);
            let base = if e.hidden {
                "unmute_button"
            } else {
                "mute_button"
            };
            // The button's narration (`PlayerEntry`: `narration.hide` / `narration.show`).
            let key = if e.hidden {
                "gui.socialInteractions.narration.show"
            } else {
                "gui.socialInteractions.narration.hide"
            };
            crate::widgets::record(
                &Text::translate(key, &[Text::literal(e.name.clone())]).plain(),
                [hide[0], hide[1], 20.0, 20.0],
                true,
            );
            batch.sprite_scaled(
                atlas,
                &sprite(base, hovered(hide)),
                hide[0],
                hide[1],
                20.0,
                20.0,
                WHITE,
            );
            batch.sprite_scaled(
                atlas,
                "gui/sprites/social_interactions/report_button_disabled",
                report[0],
                report[1],
                20.0,
                20.0,
                WHITE,
            );
        }
    }
    hud_layout::paint(batch, atlas, &faces);
    // `enableScissor`: from 4px below the list's top.
    batch.clip_since(start, 0.0, LIST_START + 4.0, w, list_end(h));
    // `Done`.
    let [dx, dy, dw, dh] = done(w, h);
    ui::button(
        batch,
        atlas,
        dx,
        dy,
        dw,
        dh,
        &Text::translate("gui.done", &[]),
        true,
        ui::hovered(cursor, dx, dy, dw, dh),
    );
    // The empty page's note, in place of the list.
    if view.entries.is_empty() {
        let key = if !view.search.is_empty() {
            Some("gui.socialInteractions.search_empty")
        } else {
            match view.page {
                Page::Hidden => Some("gui.socialInteractions.empty_hidden"),
                Page::Blocked => Some("gui.socialInteractions.empty_blocked"),
                Page::All => None,
            }
        };
        if let Some(key) = key {
            let note = Text::translate(key, &[]);
            let nw = atlas.font.width_text(&note);
            let y = ((72.0 + list_end(h)) / 2.0).floor();
            batch.text_styled(
                atlas,
                &note,
                ((w - nw) / 2.0).floor(),
                y,
                rgba(170, 170, 170, 255),
                true,
            );
        }
    }
    if let Some(text) = tooltip((w, h), cursor, view) {
        ui::tooltip(batch, atlas, &[text], cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u128, name: &str) -> Entry {
        Entry {
            id,
            name: name.into(),
            skin: tab_list::default_skin(id).into(),
            hidden: false,
            removed: false,
            local: false,
        }
    }

    /// A click on a row's button hides a shown player and shows a hidden one, a click on
    /// one's own row does nothing, and the tabs and Done answer where vanilla puts them.
    #[test]
    fn the_buttons_answer_their_clicks() {
        let size = (427.0, 360.0);
        let mut me = entry(1, "Me");
        me.local = true;
        let mut hidden = entry(3, "Hidden");
        hidden.hidden = true;
        let view = SocialView {
            entries: vec![me, entry(2, "Other"), hidden],
            ..SocialView::default()
        };
        let rows: Vec<_> = visible(&view, size.1).collect();
        assert_eq!(rows.len(), 3);
        let at = |i: usize| {
            let (hide, _) = row_buttons(size.0, rows[i].1);
            (hide[0] + 2.0, hide[1] + 2.0)
        };
        assert_eq!(
            click(size, at(0), &view),
            None,
            "no button on one's own row"
        );
        assert_eq!(click(size, at(1), &view), Some(Action::Hide(2)));
        assert_eq!(click(size, at(2), &view), Some(Action::Show(3)));
        let tab = tabs(size.0)[1];
        assert_eq!(
            click(size, (tab.0 + 2.0, 50.0), &view),
            Some(Action::Page(Page::Hidden))
        );
        let [dx, dy, ..] = done(size.0, size.1);
        assert_eq!(click(size, (dx + 2.0, dy + 2.0), &view), Some(Action::Done));
    }

    /// The list scrolls by whole rows less its height, and a row scrolled out is not clickable.
    #[test]
    fn the_list_scrolls_and_clips() {
        let h = 240.0;
        let entries: Vec<_> = (0..10).map(|i| entry(i + 1, &format!("P{i}"))).collect();
        let mut view = SocialView {
            entries,
            ..SocialView::default()
        };
        let shown = visible(&view, h).count();
        assert!(shown < 10, "ten rows do not fit {h}");
        let max = max_scroll(h, 10);
        assert!(max > 0.0 && max < 10.0 * ITEM_HEIGHT);
        view.scroll = max;
        let last = visible(&view, h).last().unwrap().0;
        assert_eq!(last, 9, "scrolled to the end shows the last row");
        let first = visible(&view, h).next().unwrap();
        let (hide, _) = row_buttons(427.0, first.1);
        // A cursor above the list's top does not reach a row's button.
        assert_eq!(click((427.0, h), (hide[0] + 2.0, 30.0), &view), None);
    }

    #[test]
    fn rows_sort_as_the_list_does() {
        let entry = |name: &str, local: bool| Entry {
            id: 0,
            name: name.into(),
            skin: String::new(),
            hidden: false,
            removed: false,
            local,
        };
        let mut rows = vec![
            entry("zed", false),
            entry("Alpha", false),
            entry("§odd", false),
            entry("me", true),
            entry("_x", false),
        ];
        sort_entries(&mut rows);
        let names: Vec<_> = rows.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["me", "_x", "Alpha", "zed", "§odd"]);
    }
}
