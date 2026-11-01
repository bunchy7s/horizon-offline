//! The resource pack screen (`PackSelectionScreen` with its two
//! `TransferableSelectionList`s, search box and buttons) and the
//! `ConfirmScreen` an incompatible pack opens, as vanilla's state machine:
//! clicks, the mouse wheel and typing change the selection model, the lists'
//! entries, scroll, selection and focus as the vanilla widgets do, and
//! [`PackScreen::draw`] gives the screen's GUI calls. The game shows it;
//! tools/test_pack_screen_parity.py compares it with vanilla.
use crate::{
    font::Patch,
    hud::{HudAtlas, PACK_ICON_SIZE, PACK_ICON_SLOTS},
    packs::{Position, Selectable, SelectionModel},
    screen_draw::{self as d, Draw, Measure, TextField},
    text::{Style, Text},
};
use std::{collections::HashMap, sync::Arc};

/// `PackCompatibility`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compat {
    Compatible,
    TooOld,
    TooNew,
    Unknown,
}

impl Compat {
    fn key(self) -> &'static str {
        match self {
            Compat::TooOld => "old",
            Compat::TooNew => "new",
            Compat::Unknown => "unknown",
            Compat::Compatible => "compatible",
        }
    }
}
/// `PackSource`: how the description is decorated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Default,
    BuiltIn,
    Feature,
    World,
    Server,
}

/// A pack as the screen shows it.
#[derive(Clone, Debug)]
pub struct PackInfo {
    pub id: String,
    pub title: Text,
    pub description: Text,
    /// The description component's own (root) style.
    pub description_style: Style,
    pub source: Source,
    pub compat: Compat,
    /// The pack's `pack.png` ([`load_icon`]); none draws `unknown_pack`.
    pub icon: Option<Arc<[u8]>>,
    pub required: bool,
    pub position: Position,
    pub fixed: bool,
}
impl PackInfo {
    pub fn selectable(&self) -> Selectable {
        Selectable {
            id: self.id.clone(),
            required: self.required,
            position: self.position,
            fixed: self.fixed,
            compatible: self.compat == Compat::Compatible,
        }
    }
    /// `PackSource.decorate` merged with the entry's grey
    /// (`getExtendedDescription`, `ComponentUtils.mergeStyles`), and the style of
    /// its root, which the ellipsis of a cut description takes.
    fn extended_description(&self) -> (Text, Style) {
        let gray = Style::color(0x808080);
        let key = match self.source {
            Source::Default => {
                let mut root = self.description_style.clone();
                root.inherit(&gray);
                return (self.description.clone().with_style(&gray), root);
            }
            Source::BuiltIn => "pack.source.builtin",
            Source::Feature => "pack.source.feature",
            Source::World => "pack.source.world",
            Source::Server => "pack.source.server",
        };
        let t = Text::translate(
            "pack.nameAndSource",
            &[self.description.clone(), Text::translate(key, &[])],
        )
        .with_color(d::GRAY);
        (t, Style::color(d::GRAY))
    }
}
/// A `pack.png` as the pack screen draws it: RGBA, stretched to a
/// `PACK_ICON_SIZE` square by nearest texels, as vanilla's icon texture is
/// sampled into its 32x32 square. None when it does not decode, where
/// `loadPackIcon` falls back to `unknown_pack`.
pub fn load_icon(png: &[u8]) -> Option<Arc<[u8]>> {
    let (w, h, rgba) = crate::hud::decode_bytes(png).ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    let size = u64::from(PACK_ICON_SIZE);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let sy = ((2 * y + 1) * u64::from(h) / (2 * size)) as usize;
        for x in 0..size {
            let sx = ((2 * x + 1) * u64::from(w) / (2 * size)) as usize;
            let at = (sy * w as usize + sx) * 4;
            out.extend_from_slice(&rgba[at..at + 4]);
        }
    }
    Some(out.into())
}
/// `PackSelectionScreen.loadPackIcon`'s texture for a pack with an icon.
pub fn icon_texture(id: &str) -> String {
    let sanitized: String = id
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let bytes: Vec<u8> = id.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let hash: String = crate::skins::sha1(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("pack/{sanitized}/{hash}/icon")
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Header,
    Pack(String),
}
#[derive(Clone, Debug)]
struct Entry {
    row: Row,
    y: i32,
    height: i32,
}
/// A `TransferableSelectionList`.
#[derive(Clone, Debug)]
struct List {
    selected_side: bool,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scroll: f64,
    entries: Vec<Entry>,
    /// `getSelected`, an index into `entries`.
    selected: Option<usize>,
    /// The list has a focused child (possibly an entry it no longer holds).
    focused: bool,
    /// The pointer's last y while the scrollbar is held (`scrolling`).
    held: Option<f64>,
}
impl List {
    fn new(selected_side: bool, height: i32) -> Self {
        Self {
            selected_side,
            x: 0,
            y: 33,
            width: 200,
            height,
            scroll: 0.0,
            entries: Vec::new(),
            selected: None,
            focused: false,
            held: None,
        }
    }
    fn bottom(&self) -> i32 {
        self.y + self.height
    }
    fn row_left(&self) -> i32 {
        self.x + self.width / 2 - self.row_width() / 2
    }
    fn row_width(&self) -> i32 {
        self.width - 4
    }
    fn content_height(&self) -> i32 {
        self.entries.iter().map(|e| e.height).sum::<i32>() + 4
    }
    fn max_scroll(&self) -> i32 {
        (self.content_height() - self.height).max(0)
    }
    fn scrollable(&self) -> bool {
        self.max_scroll() > 0
    }
    /// `TransferableSelectionList.Entry.getWidth`.
    fn entry_width(&self) -> i32 {
        self.row_width() - if self.scrollable() { 6 } else { 0 }
    }
    fn next_y(&self) -> i32 {
        self.y + 2 - self.scroll as i32 + self.entries.iter().map(|e| e.height).sum::<i32>()
    }
    fn add(&mut self, row: Row, height: i32) {
        let y = self.next_y();
        self.entries.push(Entry { row, y, height });
    }
    fn reposition(&mut self) {
        let mut y = self.y + 2 - self.scroll as i32;
        for e in &mut self.entries {
            e.y = y;
            y += e.height;
        }
    }
    fn set_scroll(&mut self, v: f64) {
        self.scroll = v.clamp(0.0, self.max_scroll() as f64);
        self.reposition();
    }
    fn scroll_to(&mut self, i: usize) {
        let e = &self.entries[i];
        let top = e.y - self.y - 2;
        if top < 0 {
            self.set_scroll(self.scroll + top as f64);
        }
        let e = &self.entries[i];
        let bottom = self.bottom() - e.y - e.height - 2;
        if bottom < 0 {
            self.set_scroll(self.scroll - bottom as f64);
        }
    }
    fn set_selected(&mut self, i: Option<usize>) {
        self.selected = i;
        if let Some(i) = i {
            let e = &self.entries[i];
            let top = e.y + 2 < self.y;
            let bottom = e.y + 2 + e.height - 4 > self.bottom();
            if top || bottom {
                self.scroll_to(i);
            }
        }
    }
    fn update_size(&mut self, width: i32, height: i32, x: i32, y: i32) {
        self.width = width;
        self.height = height;
        self.x = x;
        self.y = y;
        self.reposition();
        if let Some(i) = self.selected {
            self.scroll_to(i);
        }
        self.set_scroll(self.scroll);
    }
    fn is_over(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64
            && y >= self.y as f64
            && x < (self.x + self.width) as f64
            && y < self.bottom() as f64
    }
    /// `getEntryAtPosition`.
    fn entry_at(&self, x: i32, y: i32) -> Option<usize> {
        let (left, w) = (self.row_left(), self.entry_width());
        self.entries
            .iter()
            .position(|e| x >= left && x < left + w && y >= e.y && y < e.y + e.height)
    }
    fn scroll_bar_x(&self) -> i32 {
        self.x + self.width - 6
    }
    fn scroller_height(&self) -> i32 {
        ((self.height * self.height) as f32 / self.content_height() as f32) as i32
    }
    /// `AbstractScrollArea.mouseDragged` while the scrollbar is held.
    fn drag(&mut self, y: f64) {
        let Some(last) = self.held else { return };
        if y < self.y as f64 {
            self.set_scroll(0.0);
        } else if y > self.bottom() as f64 {
            self.set_scroll(self.max_scroll() as f64);
        } else {
            let max = self.max_scroll().max(1) as f64;
            let bar = self.scroller_height().clamp(32, self.height - 8);
            let scale = (max / (self.height - bar) as f64).max(1.0);
            self.set_scroll(self.scroll + (y - last) * scale);
        }
        self.held = Some(y);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    None,
    Search,
    List(usize),
    /// Open Pack Folder (0) or Done (1).
    Button(usize),
}

/// The open `ConfirmScreen` for an incompatible pack.
#[derive(Clone, Debug)]
struct Confirm {
    id: String,
    title: Text,
    message: Text,
}

/// What a click on the screen asks the game to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The Open Pack Folder button.
    OpenFolder,
    /// The Done button (`onClose`: commit).
    Done,
}

/// The pack screen and its state.
#[derive(Clone, Debug)]
pub struct PackScreen {
    /// `resourcePack.title`, or `dataPack.title` from Create World.
    pub title: Text,
    pub width: i32,
    pub height: i32,
    pub in_world: bool,
    pub packs: Vec<PackInfo>,
    pub model: SelectionModel,
    search: TextField,
    focus: Focus,
    /// 0: available, 1: selected.
    lists: [List; 2],
    confirm: Option<Confirm>,
    /// The selection changed since the screen opened.
    pub changed: bool,
    /// The packs' icons by their [`icon_texture`].
    icons: HashMap<String, Arc<[u8]>>,
}

fn tr(key: &str) -> Text {
    Text::translate(key, &[])
}

impl PackScreen {
    /// `PackSelectionScreen.init`: the lists filled, the search box focused.
    pub fn new(
        m: &Measure,
        title: Text,
        width: i32,
        height: i32,
        in_world: bool,
        packs: Vec<PackInfo>,
        selected: &[String],
    ) -> Self {
        let model = SelectionModel::new(packs.iter().map(PackInfo::selectable).collect(), selected);
        let mut search = TextField::new(0, 0, 200, 15);
        search.hint = Some(tr("gui.packSelection.search").with_style(&Style {
            color: Some(d::GRAY),
            italic: Some(true),
            ..Style::default()
        }));
        let icons = packs
            .iter()
            .filter_map(|p| Some((icon_texture(&p.id), p.icon.clone()?)))
            .collect();
        let mut s = Self {
            title,
            width,
            height,
            in_world,
            packs,
            icons,
            model,
            search,
            focus: Focus::None,
            lists: [List::new(false, height - 66), List::new(true, height - 66)],
            confirm: None,
            changed: false,
        };
        s.reposition(m);
        s.populate(None);
        s.set_focus(Focus::Search);
        s
    }
    fn info(&self, id: &str) -> Option<&PackInfo> {
        self.packs.iter().find(|p| p.id == id)
    }
    /// The header layout: title, drop info and search box, centred.
    fn header(&self, m: &Measure) -> [(i32, i32); 3] {
        let tw = m.width(&self.title);
        let dw = m.width(&tr("pack.dropInfo"));
        let col = tw.max(dw).max(200);
        let frame = self.width.max(col);
        let lx = (0.5 * (frame - col) as f32) as i32;
        let cell = |w: i32| lx + (0.5 * (col - w) as f32) as i32;
        [(cell(tw), 4), (cell(dw), 17), (cell(200), 30)]
    }
    /// The footer's two buttons.
    fn footer(&self) -> [(i32, i32); 2] {
        let frame = self.width.max(308);
        let x = (0.5 * (frame - 308) as f32) as i32;
        let y = self.height - 33 + (0.5f32 * 13.0).round() as i32;
        [(x, y), (x + 158, y)]
    }
    /// `repositionElements`.
    fn reposition(&mut self, m: &Measure) {
        let (sx, sy) = self.header(m)[2];
        self.search.x = sx;
        self.search.y = sy;
        let content = self.height - 49 - 33;
        let w = self.width;
        self.lists[0].update_size(200, content, w / 2 - 15 - 200, 49);
        self.lists[1].update_size(200, content, w / 2 + 15, 49);
    }
    /// `AbstractContainerEventHandler.setFocused` on the screen.
    fn set_focus(&mut self, f: Focus) {
        if self.focus == f {
            return;
        }
        match self.focus {
            Focus::Search => self.search.focused = false,
            Focus::List(i) => self.lists[i].focused = false,
            Focus::None | Focus::Button(_) => {}
        }
        if f == Focus::Search {
            self.search.focused = true;
        }
        self.focus = f;
    }
    fn shown(&self, side: usize) -> Vec<String> {
        let value = self.search.value.to_lowercase();
        let blank = self.search.value.trim().is_empty()
            && self.search.value.chars().all(char::is_whitespace);
        let list = if side == 1 {
            &self.model.selected
        } else {
            &self.model.unselected
        };
        list.iter()
            .filter(|p| {
                blank
                    || p.id.to_lowercase().contains(&value)
                    || self.info(&p.id).is_some_and(|i| {
                        i.title.plain().to_lowercase().contains(&value)
                            || i.description.plain().to_lowercase().contains(&value)
                    })
            })
            .map(|p| p.id.clone())
            .collect()
    }
    /// `TransferableSelectionList.updateList`.
    fn update_list(&mut self, side: usize, ids: Vec<String>, transferred: Option<&str>) {
        let l = &mut self.lists[side];
        l.entries.clear();
        l.selected = None;
        l.add(Row::Header, 13);
        l.set_selected(None);
        for id in ids {
            let hit = transferred == Some(id.as_str());
            self.lists[side].add(Row::Pack(id), 36);
            if hit {
                self.set_focus(Focus::List(side));
                let l = &mut self.lists[side];
                l.focused = true;
                let i = l.entries.len() - 1;
                l.set_selected(Some(i));
            }
        }
        let l = &mut self.lists[side];
        l.set_scroll(l.scroll);
    }
    /// `populateLists`: both lists, then again through the search filter.
    fn populate(&mut self, transferred: Option<&str>) {
        let all = |m: &SelectionModel, side: bool| -> Vec<String> {
            (if side { &m.selected } else { &m.unselected })
                .iter()
                .map(|p| p.id.clone())
                .collect()
        };
        self.update_list(1, all(&self.model, true), transferred);
        self.update_list(0, all(&self.model, false), transferred);
        self.filter(transferred);
    }
    /// `updateFilteredEntries`.
    fn filter(&mut self, transferred: Option<&str>) {
        let (sel, avail) = (self.shown(1), self.shown(0));
        self.update_list(1, sel, transferred);
        self.update_list(0, avail, transferred);
    }
    /// The pack's index in its model list.
    fn model_index(&self, side: usize, id: &str) -> Option<usize> {
        let list = if side == 1 {
            &self.model.selected
        } else {
            &self.model.unselected
        };
        list.iter().position(|p| p.id == id)
    }
    fn flags(&self, side: usize, id: &str) -> (bool, bool, bool, bool) {
        let Some(i) = self.model_index(side, id) else {
            return (false, false, false, false);
        };
        let s = side == 1;
        (
            self.model.can_select(s, i),
            self.model.can_unselect(s, i),
            self.model.can_move_up(s, i),
            self.model.can_move_down(s, i),
        )
    }
    fn overlay(&self, id: &str) -> bool {
        self.info(id).is_none_or(|p| !p.fixed || !p.required)
    }

    /// Whether a left click activates a footer or confirmation button.
    /// Pack entries and the search field do not play widget click sounds.
    pub fn button_at(&self, m: &Measure, x: f64, y: f64) -> bool {
        let buttons = if let Some(c) = &self.confirm {
            self.confirm_layout(m, c).2
        } else {
            self.footer()
        };
        buttons.into_iter().any(|(bx, by)| {
            x >= bx as f64 && y >= by as f64 && x < (bx + 150) as f64 && y < (by + 20) as f64
        })
    }
    /// A left click (`Screen.mouseClicked`).
    pub fn click(&mut self, m: &Measure, x: f64, y: f64) -> Action {
        if self.confirm.is_some() {
            return self.confirm_click(m, x, y);
        }
        if self.search.contains(x, y) {
            self.search.click(m, x, false);
            self.set_focus(Focus::Search);
            return Action::None;
        }
        for side in 0..2 {
            if !self.lists[side].is_over(x, y) {
                continue;
            }
            let l = &self.lists[side];
            let scrolling = l.scrollable()
                && x >= l.scroll_bar_x() as f64
                && x <= (l.scroll_bar_x() + 6) as f64
                && y >= l.y as f64
                && y < l.bottom() as f64;
            // updateScrolling: a press on the scroller holds it.
            if scrolling {
                self.lists[side].held = Some(y);
                self.set_focus(Focus::List(side));
                return Action::None;
            }
            let l = &self.lists[side];
            let hit = l.entry_at(x as i32, y as i32);
            let mut took = false;
            if let Some(i) = hit {
                took = true;
                match l.entries[i].row.clone() {
                    Row::Header => {
                        let l = &mut self.lists[side];
                        l.focused = true;
                        l.set_selected(Some(i));
                    }
                    Row::Pack(id) => {
                        let current = self.entry_click(m, side, i, &id, x as i32, y as i32);
                        if self.lists[side]
                            .entries
                            .iter()
                            .any(|e| e.row == Row::Pack(id.clone()))
                        {
                            let l = &mut self.lists[side];
                            l.focused = true;
                            if current {
                                l.set_selected(Some(i));
                            }
                        }
                    }
                }
            }
            if took {
                self.set_focus(Focus::List(side));
            }
            return Action::None;
        }
        for (i, (bx, by)) in self.footer().into_iter().enumerate() {
            if x >= bx as f64 && y >= by as f64 && x < (bx + 150) as f64 && y < (by + 20) as f64 {
                self.set_focus(Focus::Button(i));
                return if i == 0 {
                    Action::OpenFolder
                } else {
                    Action::Done
                };
            }
        }
        Action::None
    }
    /// `PackEntry.mouseClicked`; true when the entry clicked is still the
    /// list's own (nothing was repopulated).
    fn entry_click(
        &mut self,
        m: &Measure,
        side: usize,
        i: usize,
        id: &str,
        x: i32,
        y: i32,
    ) -> bool {
        if !self.overlay(id) {
            return true;
        }
        let e = &self.lists[side].entries[i];
        let (cx, cy) = (self.lists[side].row_left() + 2, e.y + 2);
        let (rx, ry) = (x - cx, y - cy);
        let (sel, unsel, up, down) = self.flags(side, id);
        let icon = (0..32).contains(&rx) && (0..32).contains(&ry);
        let left = (0..16).contains(&rx) && (0..32).contains(&ry);
        let top_right = (16..32).contains(&rx) && (0..16).contains(&ry);
        let bottom_right = (16..32).contains(&rx) && (16..32).contains(&ry);
        let index = self.model_index(side, id).unwrap_or(0);
        let s = side == 1;
        if sel && icon {
            if self
                .info(id)
                .is_some_and(|p| p.compat != Compat::Compatible)
            {
                self.open_confirm(id);
                return true;
            }
            self.model.activate(s, index, true);
        } else if unsel && left {
            self.model.activate(s, index, false);
        } else if up && top_right {
            self.model.move_by(s, index, true);
        } else if down && bottom_right {
            self.model.move_by(s, index, false);
        } else {
            return true;
        }
        self.changed = true;
        self.populate(Some(id));
        let _ = m;
        false
    }
    /// `handlePackSelection` of an incompatible pack: the ConfirmScreen
    /// (the pack screen loses its focus as `Gui.setScreen` clears it).
    fn open_confirm(&mut self, id: &str) {
        let compat = self.info(id).map_or(Compat::Unknown, |p| p.compat);
        self.set_focus(Focus::None);
        for l in &mut self.lists {
            l.focused = false;
        }
        self.confirm = Some(Confirm {
            id: id.to_owned(),
            title: tr("pack.incompatible.confirm.title"),
            message: tr(&format!("pack.incompatible.confirm.{}", compat.key())),
        });
    }
    /// The ConfirmScreen's layout: title, message lines and the two buttons.
    fn confirm_layout(
        &self,
        m: &Measure,
        c: &Confirm,
    ) -> ((i32, i32), (Vec<(Text, i32)>, i32, i32), [(i32, i32); 2]) {
        let max = self.width - 50;
        let lines = d::label_lines(m, &c.message, max, 15, &Style::default());
        let mw = d::label_width(&lines, max);
        let mh = lines.len() as i32 * 9;
        let tw = m.width(&c.title);
        let col = tw.max(mw).max(304);
        let total = 9 + 8 + mh + 8 + 36;
        let lx = (0.5 * (self.width - col) as f32) as i32;
        let ly = (0.5 * (self.height - total) as f32) as i32;
        let cell = |w: i32| lx + (0.5 * (col - w) as f32) as i32;
        let by = ly + 9 + 8 + mh + 8 + 16;
        let bx = cell(304);
        (
            (cell(tw), ly),
            (lines, cell(mw), ly + 17),
            [(bx, by), (bx + 154, by)],
        )
    }
    fn confirm_click(&mut self, m: &Measure, x: f64, y: f64) -> Action {
        let Some(c) = self.confirm.clone() else {
            return Action::None;
        };
        let (_, _, buttons) = self.confirm_layout(m, &c);
        for (i, (bx, by)) in buttons.into_iter().enumerate() {
            if x >= bx as f64 && y >= by as f64 && x < (bx + 150) as f64 && y < (by + 20) as f64 {
                // The callback: back to the pack screen (repositioned), then select.
                self.confirm = None;
                self.reposition(m);
                if i == 0
                    && let Some(index) = self.model_index(0, &c.id)
                {
                    self.model.activate(false, index, true);
                    self.changed = true;
                    self.populate(Some(&c.id));
                }
                return Action::None;
            }
        }
        Action::None
    }
    /// The focus, the lists' rows, scroll and selection (for debugging).
    pub fn debug(&self) -> String {
        format!(
            "{:?} {:?}",
            self.focus,
            self.lists
                .iter()
                .map(|l| (
                    l.scroll,
                    l.selected,
                    l.focused,
                    l.entries.iter().map(|e| e.y).collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>()
        )
    }
    /// Whether the confirmation screen is shown.
    pub fn confirming(&self) -> bool {
        self.confirm.is_some()
    }
    /// Escape on the confirmation screen answers no.
    pub fn cancel_confirm(&mut self, m: &Measure) -> bool {
        if self.confirm.take().is_some() {
            self.reposition(m);
            return true;
        }
        false
    }
    /// The mouse wheel (`AbstractScrollArea.mouseScrolled`).
    /// The pointer moved with the button held: a held scrollbar follows it.
    pub fn drag(&mut self, y: f64) {
        for l in &mut self.lists {
            l.drag(y);
        }
    }
    /// `onRelease`.
    pub fn release(&mut self) {
        for l in &mut self.lists {
            l.held = None;
        }
    }
    pub fn scroll(&mut self, x: f64, y: f64, amount: f64) {
        if self.confirm.is_some() || self.search.contains(x, y) {
            return;
        }
        for l in &mut self.lists {
            if l.is_over(x, y) {
                l.set_scroll(l.scroll - amount * 18.0);
                return;
            }
        }
    }
    /// Typed text, when the search box has the focus.
    pub fn type_text(&mut self, m: &Measure, s: &str) {
        if self.confirm.is_none() && self.focus == Focus::Search && self.search.insert_text(m, s) {
            self.filter(None);
        }
    }
    /// Ctrl+A/C/V/X in the search box; false when it does not have the focus.
    pub fn shortcut(
        &mut self,
        m: &Measure,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        if !self.search_focused() {
            return false;
        }
        if self.search.shortcut(m, shortcut, clipboard) {
            self.filter(None);
        }
        true
    }
    /// Backspace in the search box.
    pub fn erase(&mut self, m: &Measure, count: isize) {
        if self.confirm.is_none()
            && self.focus == Focus::Search
            && self.search.delete_chars(m, -count)
        {
            self.filter(None);
        }
    }
    pub fn search_focused(&self) -> bool {
        self.confirm.is_none() && self.focus == Focus::Search
    }
    /// Resize (`Screen.init` on a new size: `repositionElements`).
    pub fn resize(&mut self, m: &Measure, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.reposition(m);
    }
    /// `PackSelectionModel.commit`: the selection as options.txt stores it.
    pub fn commit(&self) -> Vec<String> {
        self.model.commit()
    }
    /// Points the icon blits of `draws` at the atlas's pack icon slots and
    /// writes the icons into them, one slot per icon drawn (vanilla registers a
    /// texture per pack). Icons past the last slot keep `unknown_pack`.
    pub fn bind_icons(&self, draws: &mut [Draw], atlas: &HudAtlas, patches: &mut Vec<Patch>) {
        let mut slots = 0..PACK_ICON_SLOTS;
        for draw in draws {
            let Draw::Blit { texture, .. } = draw else {
                continue;
            };
            let Some(icon) = self.icons.get(texture.as_str()) else {
                continue;
            };
            let Some(slot) = slots.next() else {
                return;
            };
            let sprite = format!("horizon/pack-icon{slot}");
            let Some(rect) = atlas.sprites.get(&sprite) else {
                return;
            };
            patches.push(Patch {
                x: (rect.u0 * atlas.width as f32).round() as u32,
                y: (rect.v0 * atlas.height as f32).round() as u32,
                width: PACK_ICON_SIZE,
                height: PACK_ICON_SIZE,
                rgba: icon.to_vec(),
            });
            *texture = sprite;
        }
    }

    /// The shown screen's calls (`Screen.extractRenderState`); `blink_ms`
    /// is the time since the search box took the focus.
    pub fn draw(&self, m: &Measure, mouse: (i32, i32), blink_ms: u64) -> Vec<Draw> {
        let mut out = Vec::new();
        // PackSelectionScreen, or the ConfirmScreen over it (its title and
        // its message).
        crate::widgets::begin_screen(|| match &self.confirm {
            Some(c) => crate::widgets::ScreenInfo::new(
                "packs/confirm",
                crate::widgets::join_for_narration(&[&c.title.plain(), &c.message.plain()]),
            ),
            None => crate::widgets::ScreenInfo::new("packs", self.title.plain()),
        });
        if let Some(c) = &self.confirm {
            let ((tx, ty), (lines, mx, my), buttons) = self.confirm_layout(m, c);
            d::string_widget(&mut out, m, &c.title, tx, ty, 0);
            d::label(&mut out, &lines, mx, my, self.width - 50, true);
            for ((bx, by), key) in buttons.into_iter().zip(["gui.yes", "gui.no"]) {
                let hover = d::inside(mouse, bx, by, 150, 20);
                crate::widgets::widget_cursor(hover, true);
                d::button(&mut out, m, &tr(key), bx, by, 150, 20, true, hover);
            }
            return out;
        }
        let [(tx, ty), (dx, dy), _] = self.header(m);
        d::string_widget(&mut out, m, &self.title, tx, ty, 0);
        d::string_widget(
            &mut out,
            m,
            &tr("pack.dropInfo").with_color(d::GRAY),
            dx,
            dy,
            0,
        );
        self.search.draw(&mut out, m, blink_ms, None);
        for side in 0..2 {
            self.draw_list(&mut out, m, side, mouse);
        }
        for (i, (bx, by)) in self.footer().into_iter().enumerate() {
            let hover = d::inside(mouse, bx, by, 150, 20);
            let label = tr(if i == 0 {
                "pack.openFolder"
            } else {
                "gui.done"
            });
            // The cursor follows the mouse; the highlight also keyboard focus.
            crate::widgets::widget_cursor(hover, true);
            d::button(
                &mut out,
                m,
                &label,
                bx,
                by,
                150,
                20,
                true,
                hover || self.focus == Focus::Button(i),
            );
            if i == 0 {
                crate::widgets::last_narratable(|n| {
                    n.hint = Some(tr("pack.folderInfo").plain());
                });
            }
            if i == 0 && hover {
                d::tooltip(&mut out, m, &tr("pack.folderInfo"), mouse);
            }
        }
        out
    }
    fn draw_list(&self, out: &mut Vec<Draw>, m: &Measure, side: usize, mouse: (i32, i32)) {
        let l = &self.lists[side];
        let hovered = if l.is_over(mouse.0 as f64, mouse.1 as f64) {
            l.entry_at(mouse.0, mouse.1)
        } else {
            None
        };
        let header = if l.selected_side {
            "pack.selected.title"
        } else {
            "pack.available.title"
        };
        // TransferableSelectionList: the row under the mouse, else the
        // selected one; the header narrates its title, a pack "Selected:
        // <title>".
        crate::widgets::narratable(|| {
            use crate::widgets::{ListKind, Narratable, Row as Narrated};
            let narrated = |i: usize| {
                let text = match &l.entries[i].row {
                    Row::Header => tr(header).plain(),
                    Row::Pack(id) => {
                        let title = self
                            .info(id)
                            .map_or_else(|| Text::literal(id.clone()), |p| p.title.clone());
                        Text::translate("narrator.select", &[title]).plain()
                    }
                };
                Narrated::new(i, text)
            };
            Narratable::list(
                ListKind::Object,
                l.entries.len(),
                hovered.map(narrated),
                l.selected.filter(|i| *i < l.entries.len()).map(narrated),
                [l.x as f32, l.y as f32, l.width as f32, l.height as f32],
            )
            .key(crate::widgets::key_of(&("packs", side)))
            .focused(l.focused)
        });
        let prefix = if self.in_world { "inworld_" } else { "" };
        let right = l.x + l.width;
        d::blit(
            out,
            &format!("textures/gui/{prefix}menu_list_background.png"),
            l.x,
            l.y,
            (right as f32, (l.bottom() + l.scroll as i32) as f32),
            l.width,
            l.height,
            [32, 32],
        );
        out.push(Draw::Scissor {
            x0: l.x.clamp(0, self.width),
            y0: l.y.clamp(0, self.height),
            x1: right.clamp(0, self.width),
            y1: l.bottom().clamp(0, self.height),
        });
        let (left, w) = (l.row_left(), l.entry_width());
        let narrow = if l.scrollable() { 6 } else { 0 };
        for (i, e) in l.entries.iter().enumerate() {
            if e.y + e.height < l.y || e.y > l.bottom() {
                continue;
            }
            if l.selected == Some(i) {
                let color = if l.focused { -1 } else { -8355712 };
                d::fill(out, left, e.y, left + w, e.y + e.height, color);
                d::fill(
                    out,
                    left + 1,
                    e.y + 1,
                    left + w - 1,
                    e.y + e.height - 1,
                    -16777216,
                );
            }
            let (cx, cy) = (left + 2, e.y + 2);
            match &e.row {
                Row::Header => {
                    let t = tr(header).with_style(&Style {
                        underlined: Some(true),
                        bold: Some(true),
                        ..Style::default()
                    });
                    let tw = m.width(&t);
                    d::text(
                        out,
                        t,
                        left + w / 2 - tw / 2,
                        cy + (e.height - 4) / 2 - 4,
                        -1,
                        true,
                    );
                }
                Row::Pack(id) => {
                    let Some(p) = self.info(id) else { continue };
                    if p.compat != Compat::Compatible {
                        d::fill(
                            out,
                            cx - 1,
                            cy - 1,
                            cx + w - 4 + 1,
                            cy + e.height - 4 + 1,
                            -8978432,
                        );
                    }
                    let icon = if p.icon.is_some() {
                        icon_texture(id)
                    } else {
                        "textures/misc/unknown_pack.png".into()
                    };
                    d::blit(out, &icon, cx, cy, (0.0, 0.0), 32, 32, [32, 32]);
                    let (mut name, (mut desc, mut desc_style)) =
                        (p.title.clone(), p.extended_description());
                    let focus_hover = l.selected == Some(i) && l.focused;
                    if self.overlay(id) && (hovered == Some(i) || focus_hover) {
                        d::fill(out, cx, cy, cx + 32, cy + 32, -1601138544);
                        let (rx, ry) = (mouse.0 - cx, mouse.1 - cy);
                        if p.compat != Compat::Compatible {
                            name = tr("pack.incompatible");
                            desc = tr(&format!("pack.incompatible.{}", p.compat.key()))
                                .with_color(d::GRAY);
                            desc_style = Style::color(d::GRAY);
                        }
                        let (sel, unsel, up, down) = self.flags(side, id);
                        let quarter = |x0: i32, x1: i32, y0: i32, y1: i32| {
                            (x0..x1).contains(&rx) && (y0..y1).contains(&ry)
                        };
                        let mut arrow = |name: &str, on: bool| {
                            let s = if on {
                                format!("transferable_list/{name}_highlighted")
                            } else {
                                format!("transferable_list/{name}")
                            };
                            d::sprite(out, &s, cx, cy, 32, 32);
                        };
                        if sel {
                            arrow("select", quarter(0, 32, 0, 32));
                        } else {
                            if unsel {
                                arrow("unselect", quarter(0, 16, 0, 32));
                            }
                            if up {
                                arrow("move_up", quarter(16, 32, 0, 16));
                            }
                            if down {
                                arrow("move_down", quarter(16, 32, 16, 32));
                            }
                        }
                    }
                    let max = 157 - narrow;
                    d::string_widget(out, m, &name, cx + 34, cy + 1, max);
                    let lines = d::label_lines(m, &desc, max, 2, &desc_style);
                    d::label(out, &lines, cx + 34, cy + 12, max, false);
                }
            }
        }
        out.push(Draw::NoScissor);
        d::blit(
            out,
            &format!("textures/gui/{prefix}header_separator.png"),
            l.x,
            l.y - 2,
            (0.0, 0.0),
            l.width,
            2,
            [32, 2],
        );
        d::blit(
            out,
            &format!("textures/gui/{prefix}footer_separator.png"),
            l.x,
            l.bottom(),
            (0.0, 0.0),
            l.width,
            2,
            [32, 2],
        );
        if l.scrollable() {
            let sh = l.scroller_height().clamp(32, l.height - 8);
            let sy = (l.scroll as i32 * (l.height - sh) / l.max_scroll() + l.y).max(l.y);
            d::sprite(
                out,
                "widget/scroller_background",
                l.scroll_bar_x(),
                l.y,
                6,
                l.height,
            );
            d::sprite(out, "widget/scroller", l.scroll_bar_x(), sy, 6, sh);
        }
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;

    fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(rgba)
            .unwrap();
        out
    }
    fn pack(id: &str, icon: Option<Arc<[u8]>>) -> PackInfo {
        PackInfo {
            id: id.into(),
            title: Text::literal(id),
            description: Text::literal(""),
            description_style: Style::default(),
            source: Source::BuiltIn,
            compat: Compat::Compatible,
            icon,
            required: false,
            position: Position::Top,
            fixed: false,
        }
    }

    /// Any size stretches over the whole square, nearest texels, as the
    /// icon texture is sampled into its 32x32 blit.
    #[test]
    fn icons_stretch_to_the_slot_by_nearest_texels() {
        let (red, blue) = ([255, 0, 0, 255], [0, 0, 255, 128]);
        let icon = load_icon(&png(2, 1, &[red, blue].concat())).unwrap();
        let side = PACK_ICON_SIZE as usize;
        assert_eq!(icon.len(), side * side * 4);
        let at = |x: usize, y: usize| &icon[(y * side + x) * 4..][..4];
        assert_eq!((at(0, 0), at(side / 2 - 1, side - 1)), (&red[..], &red[..]));
        assert_eq!(
            (at(side / 2, 0), at(side - 1, side - 1)),
            (&blue[..], &blue[..])
        );
        assert!(load_icon(b"not a png").is_none());
    }

    /// Programmer Art and High Contrast draw their own `pack.png`; a pack
    /// without one keeps `unknown_pack`.
    #[test]
    fn drawn_icons_take_atlas_slots() {
        let atlas =
            crate::hud::HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets"))
                .unwrap();
        let m = Measure(&atlas.font);
        let dirt = load_icon(&png(1, 1, &[134, 96, 67, 255])).unwrap();
        let contrast = load_icon(&png(1, 1, &[0, 0, 0, 255])).unwrap();
        let packs = vec![
            pack("programmer_art", Some(dirt.clone())),
            pack("high_contrast", Some(contrast.clone())),
            pack("file/Iconless", None),
        ];
        let screen = PackScreen::new(&m, tr("resourcePack.title"), 480, 320, false, packs, &[]);
        let mut draws = screen.draw(&m, (0, 0), 0);
        let mut patches = Vec::new();
        screen.bind_icons(&mut draws, &atlas, &mut patches);
        let icons: Vec<&str> = draws
            .iter()
            .filter_map(|d| match d {
                Draw::Blit { texture, .. } if texture.contains("pack") => Some(texture.as_str()),
                _ => None,
            })
            .collect();
        // The list shows them sorted by id.
        assert_eq!(
            icons,
            [
                "textures/misc/unknown_pack.png",
                "horizon/pack-icon0",
                "horizon/pack-icon1"
            ]
        );
        assert_eq!(patches.len(), 2);
        for (i, (patch, icon)) in patches.iter().zip([&contrast, &dirt]).enumerate() {
            let rect = atlas.sprites[&format!("horizon/pack-icon{i}")];
            assert_eq!(
                (patch.x, patch.y, patch.width, patch.height),
                (
                    (rect.u0 * atlas.width as f32).round() as u32,
                    (rect.v0 * atlas.height as f32).round() as u32,
                    PACK_ICON_SIZE,
                    PACK_ICON_SIZE
                )
            );
            assert_eq!(&patch.rgba[..], &icon[..]);
        }
    }
}

#[cfg(test)]
mod click_sound_tests {
    use super::*;

    #[test]
    fn only_footer_and_confirmation_buttons_request_a_click_sound() {
        let atlas =
            crate::hud::HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets"))
                .unwrap();
        let m = Measure(&atlas.font);
        let title = tr("resourcePack.title");
        let mut screen = PackScreen::new(&m, title, 480, 320, false, Vec::new(), &[]);
        assert!(!screen.button_at(&m, 0.0, 0.0));
        assert!(!screen.button_at(
            &m,
            screen.search.x as f64 + 1.0,
            screen.search.y as f64 + 1.0
        ));
        let buttons = screen.footer();
        for (i, (x, y)) in buttons.into_iter().enumerate() {
            assert!(screen.button_at(&m, x as f64 + 1.0, y as f64 + 1.0));
            assert!(!screen.button_at(&m, (x + 150) as f64, (y + 20) as f64));
            assert_eq!(
                screen.click(&m, x as f64 + 1.0, y as f64 + 1.0),
                if i == 0 {
                    Action::OpenFolder
                } else {
                    Action::Done
                }
            );
        }
        screen.confirm = Some(Confirm {
            id: "incompatible".into(),
            title: Text::literal("Confirm"),
            message: Text::literal("Use this pack?"),
        });
        let buttons = screen
            .confirm_layout(&m, screen.confirm.as_ref().unwrap())
            .2;
        for (x, y) in buttons {
            assert!(screen.button_at(&m, x as f64 + 1.0, y as f64 + 1.0));
        }
        assert!(!screen.button_at(
            &m,
            screen.footer()[0].0 as f64 + 1.0,
            screen.footer()[0].1 as f64 + 1.0
        ));
        let (x, y) = buttons[1];
        assert_eq!(
            screen.click(&m, x as f64 + 1.0, y as f64 + 1.0),
            Action::None
        );
        assert!(!screen.confirming());
    }
}
