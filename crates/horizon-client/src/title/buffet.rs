//! CreateBuffetWorldScreen: the single-biome preset's biome list with its
//! search box, and the Done and Cancel that hand a biome back to Create New
//! World (`WorldCreationUiState.updateDimensions`).
use super::{Button, Title};
use crate::{
    hud::{HudAtlas, HudBatch},
    lang,
    text::Text,
    ui,
};
use serde_json::Value;
use std::path::Path;

/// The search box's text field, for the caret, selection and clipboard.
pub(super) const SEARCH: crate::clipboard::FieldId = ("world.buffet-search", 0);
/// HeaderAndFooterLayout's header: the title's font line, 13 above it, the
/// 15-pixel search box 3 under it.
const HEADER: f32 = 40.0;
/// HeaderAndFooterLayout's default footer.
const FOOTER: f32 = 33.0;
/// The header's search box (`new EditBox(font, 200, 15, ...)`).
const SEARCH_WIDTH: f32 = 200.0;
const SEARCH_HEIGHT: f32 = 15.0;
/// The search box's top: 13 + the title's 9-pixel line + 3.
const SEARCH_TOP: f32 = 25.0;
/// `ObjectSelectionList`'s row: 220 wide and the list's default 20 high.
const ROW_WIDTH: f32 = 220.0;
const ROW_HEIGHT: f32 = 20.0;

/// One biome the screen offers: its registry id and the name it shows
/// (`Component.translatable(id.toLanguageKey("biome"))`, else the id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Biome {
    pub(super) id: String,
    pub(super) name: String,
}

/// CreateBuffetWorldScreen's state: the biomes the prepared worldgen data
/// holds, the search text, and the biome the overworld will be made of.
#[derive(Clone, Debug)]
pub struct State {
    /// `EditBox`'s text: the rows whose name contains it, case-folded.
    pub(super) query: String,
    pub(super) query_focused: bool,
    /// The fixed biome (`CreateBuffetWorldScreen.biome`): plains until the
    /// world says otherwise, and what `write_world_gen_settings` writes.
    pub(super) biome: String,
    pub(super) scroll: ui::Scroll,
    biomes: Vec<Biome>,
}
impl State {
    /// The screen over `biome`, the biome the selected preset fixes, with the
    /// biomes of the prepared resource root.
    pub(super) fn new(root: &Path, biome: &str) -> Self {
        Self {
            query: String::new(),
            query_focused: false,
            biome: biome.to_owned(),
            scroll: ui::Scroll::default(),
            biomes: load(root),
        }
    }
    /// The biome the world will be made of, for the control socket's state.
    pub fn biome(&self) -> &str {
        &self.biome
    }
    /// The search text, for the control socket's state.
    pub fn query(&self) -> &str {
        &self.query
    }
    /// The rows the search text leaves, in the order the list shows them.
    pub(super) fn rows(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        self.biomes
            .iter()
            .enumerate()
            .filter(|(_, biome)| query.is_empty() || biome.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect()
    }
    /// The selected row, while the search text still shows the biome.
    pub(super) fn selected(&self) -> Option<usize> {
        self.rows()
            .iter()
            .position(|index| self.biomes[*index].id == self.biome)
    }
    fn name(&self, row: usize) -> Option<&str> {
        self.biomes.get(row).map(|biome| biome.name.as_str())
    }
    fn select(&mut self, row: usize) {
        if let Some(biome) = self.biomes.get(row) {
            self.biome = biome.id.clone();
        }
    }
}
/// The biome registry of the prepared resources, by the name each is shown
/// under: `Registry.listElements` sorted with a locale collator, which for
/// these names is a case-insensitive order with the id breaking ties.
fn load(root: &Path) -> Vec<Biome> {
    let Ok(text) = std::fs::read_to_string(root.join("worldgen/biomes.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let mut biomes: Vec<Biome> = value
        .as_object()
        .map(|entries| {
            entries
                .keys()
                .map(|id| Biome {
                    id: id.clone(),
                    name: display_name(id),
                })
                .collect()
        })
        .unwrap_or_default();
    biomes.sort_by_cached_key(|biome| (biome.name.to_lowercase(), biome.id.clone()));
    biomes
}
/// `Identifier.toLanguageKey("biome")`: `biome.<namespace>.<path>` with the
/// path's separators turned into dots. An id the language has no name for is
/// shown as it is, as `CreateBuffetWorldScreen$BiomeList$Entry` does.
fn display_name(id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    let key = format!("biome.{namespace}.{}", path.replace(['/', '.'], "."));
    if lang::has(&key) {
        Text::translate(&key, &[]).plain()
    } else {
        id.to_owned()
    }
}
/// The list's area: under the header, above the footer, `ROW_WIDTH` wide in
/// the middle of the screen.
fn list_area(size: (f32, f32), rows: usize) -> (f32, f32, f32, f32) {
    let (w, h) = (size.0.ceil(), size.1.ceil());
    (
        HEADER,
        h - FOOTER,
        ((w - ROW_WIDTH) * 0.5).floor(),
        rows as f32 * ROW_HEIGHT + 4.0,
    )
}
pub(super) fn search_box(w: f32) -> [f32; 4] {
    [
        ((w - SEARCH_WIDTH) * 0.5).floor(),
        SEARCH_TOP,
        SEARCH_WIDTH,
        SEARCH_HEIGHT,
    ]
}
/// The row under the pointer, if any (`getEntryAtPosition`).
fn row_at(state: &State, cursor: (f32, f32), size: (f32, f32)) -> Option<usize> {
    let rows = state.rows();
    let (top, bottom, left, _) = list_area(size, rows.len());
    if cursor.0 < left || cursor.0 >= left + ROW_WIDTH || cursor.1 < top || cursor.1 >= bottom {
        return None;
    }
    let row = ((cursor.1 - top + state.scroll.amount.trunc()) / ROW_HEIGHT) as usize;
    rows.get(row).copied()
}
/// A press on the screen: the search box, the list, or the scrollbar.
pub(super) fn click(state: &mut State, cursor: (f32, f32), size: (f32, f32)) {
    let rows = state.rows();
    let (top, bottom, left, content) = list_area(size, rows.len());
    let field = search_box(size.0.ceil());
    state.query_focused = ui::hovered(cursor, field[0], field[1], field[2], field[3]);
    if state
        .scroll
        .press(cursor, left + ROW_WIDTH + 6.0, top, bottom, content)
    {
        return;
    }
    if let Some(row) = row_at(state, cursor, size) {
        state.select(row);
    }
}
pub(super) fn wheel(state: &mut State, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
    let rows = state.rows();
    let (top, bottom, _, content) = list_area(size, rows.len());
    if cursor.1 < top || cursor.1 >= bottom {
        return;
    }
    state.scroll.wheel(delta, ROW_HEIGHT, top, bottom, content);
}
pub(super) fn drag(state: &mut State, cursor: (f32, f32), size: (f32, f32)) {
    let rows = state.rows();
    let (top, bottom, _, content) = list_area(size, rows.len());
    state.scroll.drag(cursor.1, top, bottom, content);
}
pub(super) fn release(state: &mut State) {
    state.scroll.release();
}
/// Done and Cancel, 150 wide and 8 apart in the footer
/// (`LinearLayout.horizontal().spacing(8)`); Done needs a biome.
pub(super) fn buttons(w: f32, h: f32, state: &State) -> Vec<Button> {
    let x = ((w.ceil() - 308.0) * 0.5).trunc();
    let b = |x: f32, key: &str, fallback: &str, enabled: bool, id: u8| Button {
        x,
        y: h.ceil() - 27.0,
        w: 150.0,
        label: lang::get_or(key, fallback),
        enabled,
        id,
        icon: None,
        trailing: None,
    };
    vec![
        b(x, "gui.done", "Done", state.selected().is_some(), 80),
        b(x + 158.0, "gui.cancel", "Cancel", true, 81),
    ]
}
/// The screen: the menu background, the title, the search box and the rows.
pub(super) fn draw(
    title: &Title,
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    cursor: (f32, f32),
    state: &State,
) {
    let (w, h) = (batch.width.ceil(), batch.height.ceil());
    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
    super::centered(
        atlas,
        batch,
        &lang::get_or(
            "createWorld.customize.buffet.title",
            "Single Biome Customization",
        ),
        8.0,
        u32::MAX,
    );
    let field = search_box(w);
    let focused = state.query_focused;
    title.caret.borrow_mut().draw(
        batch,
        atlas,
        (SEARCH, &state.query, 50),
        field,
        focused,
        title.started,
    );
    crate::widgets::name_last_edit_box(|| {
        lang::get_or("createWorld.customize.buffet.search", "Search...")
    });
    if state.query.is_empty() && !focused {
        let box_ = ui::EditBox::new(field[0], field[1], field[2], field[3]);
        batch.text_styled(
            atlas,
            &Text::translate("createWorld.customize.buffet.search", &[])
                .with_color(crate::text::DARK_GRAY),
            box_.text_x(),
            box_.text_y(),
            u32::MAX,
            true,
        );
    }
    let rows = state.rows();
    let (top, bottom, left, content) = list_area((w, h), rows.len());
    ui::list_background(batch, atlas, top, bottom, false);
    crate::ui::separators(batch, atlas, top, bottom, false);
    let first = batch.vertices.len();
    let offset = state.scroll.amount.trunc();
    let selected = state.selected();
    let hovered = row_at(state, cursor, (w, h))
        .and_then(|biome| rows.iter().position(|index| *index == biome));
    // ObjectSelectionList: every row narrates "narrator.select" with its name.
    crate::widgets::narratable(|| {
        use crate::widgets::{ListKind, Narratable, Row};
        let row = |at: usize| {
            let name = rows.get(at).and_then(|index| state.name(*index));
            let narration = Text::translate(
                "narrator.select",
                &[Text::literal(name.unwrap_or_default())],
            )
            .plain();
            Row::new(at, narration)
        };
        Narratable::list(
            ListKind::Object,
            rows.len(),
            hovered.map(row),
            selected.map(row),
            [left, top, ROW_WIDTH, bottom - top],
        )
        .key(crate::widgets::key_of(&"create_world/buffet"))
    });
    for (at, index) in rows.iter().enumerate() {
        let y = top + 2.0 + at as f32 * ROW_HEIGHT - offset;
        let target = [left, y - 2.0, ROW_WIDTH, ROW_HEIGHT];
        if selected == Some(at) {
            batch.fill(
                atlas,
                target[0],
                target[1],
                target[2],
                target[3],
                crate::hud::rgba(255, 255, 255, 70),
            );
        } else if hovered == Some(at) {
            batch.fill(
                atlas,
                target[0],
                target[1],
                target[2],
                target[3],
                crate::hud::rgba(255, 255, 255, 25),
            );
        }
        // `Entry.extractContent`: the name at contentX + 5, contentY + 2.
        if let Some(biome) = state.biomes.get(*index) {
            batch.text(atlas, &biome.name, left + 5.0, y, u32::MAX, true);
        }
    }
    batch.clip_since(first, left, top, left + ROW_WIDTH, bottom);
    ui::scrollbar(
        batch,
        atlas,
        left + ROW_WIDTH + 6.0,
        top,
        bottom,
        content,
        state.scroll.amount,
    );
}
