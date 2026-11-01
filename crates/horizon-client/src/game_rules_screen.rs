//! AbstractGameRulesScreen: the rules by category under a search box, a
//! switch or a number box each. InWorldGameRulesScreen, opened from World
//! Options, sends the changed ones on Done (ServerboundSetGameRulePacket) and
//! asks before leaving with changes (ConfirmScreen); WorldCreationGameRulesScreen,
//! opened from Create New World, hands them back and leaves without asking.
use crate::{
    confirm_screen::Confirm,
    font::Font,
    gamerules::{self, GameRules, RULES, Rule, Value},
    hud::{HudAtlas, HudBatch},
    lang,
    text::{Style, Text, named_color},
    ui::{self, EditBox},
};

/// HeaderAndFooterLayout(12 + 9 + 15, 33): the title over the search box.
const HEADER: f32 = 36.0;
const FOOTER: f32 = 33.0;
/// RuleList: 24 pixel entries in AbstractSelectionList's 220 pixel rows.
const ROW: f32 = 24.0;
const ROW_WIDTH: f32 = 220.0;
const SEARCH_WIDTH: f32 = 200.0;
/// EditBox text colours (`hud::rgba` order, ABGR): IntegerRuleEntry's valid
/// (-2039584) and invalid (-65536).
const VALID: u32 = 0xFFE0_E0E0;
const INVALID: u32 = 0xFF00_00FF;

#[derive(Clone, Copy)]
enum Row {
    Category(&'static str),
    Rule(&'static Rule),
}

/// What a click or Escape asks of World Options.
#[derive(Debug, PartialEq)]
pub enum Exit {
    /// Done: the rules that changed.
    Apply(Vec<(&'static str, Value)>),
    /// Cancel, or discarding the changes.
    Discard,
}

pub struct RulesScreen {
    search: EditBox,
    values: GameRules,
    initial: GameRules,
    /// IntegerRuleEntry's boxes, by rule.
    numbers: Vec<(&'static str, EditBox)>,
    /// RuleList's scrolling: the wheel and the held scrollbar.
    scroll: ui::Scroll,
    /// ConfirmScreen "Game Rule Changes" in its place.
    confirm: bool,
    /// InWorldGameRulesScreen rather than WorldCreationGameRulesScreen.
    in_world: bool,
}

fn name_key(r: &Rule) -> String {
    format!("gamerule.minecraft.{}", r.name)
}
fn category_key(category: &str) -> String {
    format!("gamerule.category.minecraft.{category}")
}
/// GameRule.deserialize of an integer rule: Brigadier's readInt (digits,
/// '.' and '-' only, nothing after), then the rule's range.
fn parse(r: &Rule, s: &str) -> Option<i32> {
    let number = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.');
    number
        .then(|| s.parse::<i32>().ok())
        .flatten()
        .filter(|v| (r.min..=r.max).contains(v))
}
fn lines(font: &Font, t: &Text, width: f32) -> Vec<Text> {
    font.split_lines(t, width)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}
fn row_left(w: f32) -> f32 {
    (w / 2.0).trunc() - ROW_WIDTH / 2.0
}
/// A rule's switch or box: 44 by 20, 45 from the entry content's right.
fn control_x(w: f32) -> f32 {
    row_left(w) + 2.0 + (ROW_WIDTH - 4.0) - 45.0
}
/// The header's LinearLayout (title, 4, search box) centred in its 36 pixels.
fn search_at(w: f32) -> (f32, f32) {
    (((w - SEARCH_WIDTH) / 2.0).trunc(), 4.0 + 9.0 + 4.0)
}
/// The footer's Done and Cancel, 150 wide and 8 apart.
fn footer(w: f32, h: f32) -> [(f32, f32); 2] {
    let x = ((w - 308.0) / 2.0).trunc();
    [(x, h - FOOTER + 6.0), (x + 158.0, h - FOOTER + 6.0)]
}
fn yellow() -> u32 {
    named_color("yellow").unwrap_or(0xFFFF55)
}
/// RuleList.addEntry's `narration`: the rule's description, when it has
/// one, over its default value.
fn narration(r: &Rule) -> String {
    let default = Text::translate(
        "editGamerule.default",
        &[Text::literal(r.default.to_string())],
    )
    .plain();
    let description = format!("{}.description", name_key(r));
    if lang::has(&description) {
        format!("{}\n{default}", lang::get(&description))
    } else {
        default
    }
}

/// BooleanRuleEntry's CycleButton (`displayOnlyValue`, its own narration):
/// "<name>: <value> button", the rule's description and default, and the
/// value a click moves to.
fn bool_narration(r: &Rule, value: bool, rect: [f32; 4]) -> crate::widgets::Narratable {
    let tr = crate::screen_narrator::translate;
    let on_off = |on: bool| tr(if on { "options.on" } else { "options.off" }, &[]);
    let full = tr(
        "options.generic_value",
        &[&lang::get(&name_key(r)), &on_off(value)],
    );
    let title = format!("{}\n{}", tr("gui.narrate.button", &[&full]), narration(r));
    crate::widgets::Narratable::new(
        crate::widgets::What::NamedCycle {
            title,
            next: on_off(!value),
            active: true,
        },
        rect,
    )
    .without_tooltip()
}

/// ConfirmScreen "Game Rule Changes".
fn confirm(font: &Font, size: (f32, f32)) -> Confirm {
    let title = Text::translate("editGamerule.inGame.discardChanges.title", &[]);
    let message = Text::translate("editGamerule.inGame.discardChanges.message", &[]);
    Confirm::new(font, size, &title, &message)
}

impl RulesScreen {
    /// InWorldGameRulesScreen over the world's rules.
    pub fn new(rules: &GameRules) -> Self {
        Self::build(rules, true)
    }
    /// WorldCreationGameRulesScreen over the new world's rules.
    pub fn for_creation(rules: &GameRules) -> Self {
        Self::build(rules, false)
    }
    fn build(rules: &GameRules, in_world: bool) -> Self {
        // setInitialFocus: the search box.
        let search = EditBox::new(0.0, 0.0, SEARCH_WIDTH, 15.0);
        let numbers = RULES
            .iter()
            .filter(|r| matches!(r.default, Value::Int(_)))
            .map(|r| {
                let mut b = EditBox::new(0.0, 0.0, 44.0, 20.0);
                b.focused = false;
                b.set_value(&rules.int(r.name).to_string());
                b.text_color = VALID;
                (r.name, b)
            })
            .collect();
        Self {
            search,
            values: rules.clone(),
            initial: rules.clone(),
            numbers,
            scroll: ui::Scroll::default(),
            confirm: false,
            in_world,
        }
    }

    /// RuleList.populateChildren: the rules matching the search (id, name,
    /// category or description), by category, each sorted by description id.
    fn rows(&self) -> Vec<Row> {
        let filter = self.search.value.to_lowercase();
        let has = |s: &str| s.to_lowercase().contains(&filter);
        let mut categories: Vec<(&'static str, Vec<&'static Rule>)> = Vec::new();
        for r in RULES {
            let key = name_key(r);
            let description = format!("{key}.description");
            let matches = has(r.name)
                || has(&lang::get(&key))
                || has(&lang::get(&category_key(r.category)))
                || (lang::has(&description) && has(&lang::get(&description)));
            if !matches {
                continue;
            }
            match categories.iter_mut().find(|(c, _)| *c == r.category) {
                Some((_, rules)) => rules.push(r),
                None => categories.push((r.category, vec![r])),
            }
        }
        // By the categories' ids, then the rules' description ids.
        categories.sort_by_key(|(c, _)| *c);
        categories
            .into_iter()
            .flat_map(|(c, mut rules)| {
                rules.sort_by_key(|r| name_key(r));
                std::iter::once(Row::Category(c)).chain(rules.into_iter().map(Row::Rule))
            })
            .collect()
    }
    /// Entry `i`'s top: 2 under the list's top, less the scroll.
    fn row_top(&self, i: usize) -> f32 {
        HEADER + 2.0 - self.scroll.amount.trunc() + ROW * i as f32
    }
    /// The list's scrollbar column (AbstractSelectionList: 6 pixels right of
    /// the entries) and the pixels it scrolls.
    fn bar(w: f32) -> f32 {
        row_left(w) + ROW_WIDTH + 6.0 + 2.0
    }
    fn content(&self) -> f32 {
        ROW * self.rows().len() as f32 + 4.0
    }
    /// getEntryAtPosition: the row under the pointer, inside the list.
    fn hovered(&self, rows: usize, (x, y): (f32, f32), (w, h): (f32, f32)) -> Option<usize> {
        let left = row_left(w);
        let inside = y >= HEADER && y < h - FOOTER && x >= left && x < left + ROW_WIDTH;
        let i = ((y - self.row_top(0)) / ROW).floor();
        (inside && i >= 0.0 && (i as usize) < rows).then_some(i as usize)
    }
    fn number(&mut self, name: &str) -> Option<&mut EditBox> {
        self.numbers
            .iter_mut()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| b)
    }
    /// An invalid number disables Done (markInvalid).
    fn invalid(&self) -> bool {
        self.numbers
            .iter()
            .any(|(name, b)| gamerules::rule(name).is_none_or(|r| parse(r, &b.value).is_none()))
    }
    fn changes(&self) -> Vec<(&'static str, Value)> {
        RULES
            .iter()
            .filter_map(|r| {
                let value = self.values.get(r.name)?;
                (Some(value) != self.initial.get(r.name)).then_some((r.name, value))
            })
            .collect()
    }
    /// InWorldGameRulesScreen.onClose: pending changes ask first;
    /// WorldCreationGameRulesScreen.onClose discards them.
    fn close(&mut self) -> Option<Exit> {
        if !self.in_world || self.changes().is_empty() {
            Some(Exit::Discard)
        } else {
            self.confirm = true;
            None
        }
    }
    /// Escape: ConfirmScreen answers no; the rules close.
    pub fn escape(&mut self) -> Option<Exit> {
        if self.confirm {
            self.confirm = false;
            None
        } else {
            self.close()
        }
    }
    /// The mouse wheel (AbstractSelectionList: half an entry a step).
    pub fn wheel(&mut self, delta: f32, h: f32) {
        if !self.confirm {
            let content = self.content();
            self.scroll
                .wheel(delta, ROW / 2.0, HEADER, h - FOOTER, content);
        }
    }
    /// A press on the scrollbar holds it (AbstractSelectionList.updateScrolling).
    pub fn press_scrollbar(&mut self, cursor: (f32, f32), (w, h): (f32, f32)) -> bool {
        let content = self.content();
        self.scroll
            .press(cursor, Self::bar(w), HEADER, h - FOOTER, content)
    }
    /// The pointer moved while the scrollbar is held.
    pub fn drag(&mut self, y: f32, h: f32) {
        if self.scroll.held() {
            let content = self.content();
            self.scroll.drag(y, HEADER, h - FOOTER, content);
        }
    }
    /// The button came up: the scrollbar lets go.
    pub fn release(&mut self) {
        self.scroll.release();
    }
    fn focused(&mut self) -> Option<(Option<&'static str>, &mut EditBox)> {
        if self.search.focused {
            return Some((None, &mut self.search));
        }
        self.numbers
            .iter_mut()
            .find(|(_, b)| b.focused)
            .map(|(n, b)| (Some(*n), b))
    }
    /// The focused box changed: the search filters again from the top; a
    /// number sets its rule when valid (IntegerRuleEntry's responder).
    fn edited(&mut self, rule: Option<&'static str>) {
        let Some(name) = rule else {
            self.scroll.amount = 0.0;
            return;
        };
        let Some(r) = gamerules::rule(name) else {
            return;
        };
        let value = self.number(name).map(|b| parse(r, &b.value));
        if let Some(Some(v)) = value {
            self.values.set(name, Value::Int(v));
        }
        if let Some(b) = self.number(name) {
            b.text_color = if matches!(value, Some(Some(_))) {
                VALID
            } else {
                INVALID
            };
        }
    }
    /// Run an edit on the focused box, then its responder.
    pub fn edit(&mut self, f: impl FnOnce(&mut EditBox)) {
        let Some((rule, b)) = self.focused() else {
            return;
        };
        let before = b.value.clone();
        f(b);
        if b.value != before {
            self.edited(rule);
        }
    }
    pub fn typing(&self) -> bool {
        !self.confirm && (self.search.focused || self.numbers.iter().any(|(_, b)| b.focused))
    }

    /// A left click; Some when the screen closes.
    pub fn click(
        &mut self,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> Option<Exit> {
        let (w, h) = size;
        let on = |x: f32, y: f32, bw: f32, bh: f32| ui::hovered(cursor, x, y, bw, bh);
        if self.confirm {
            match confirm(&atlas.font, size).answer_at(cursor) {
                Some(true) => return Some(Exit::Discard),
                Some(false) => self.confirm = false,
                None => {}
            }
            return None;
        }
        let [(dx, dy), (cx, cy)] = footer(w, h);
        if on(dx, dy, 150.0, 20.0) && !self.invalid() {
            return Some(Exit::Apply(self.changes()));
        }
        if on(cx, cy, 150.0, 20.0) {
            return self.close();
        }
        // A press on the scrollbar holds it and clicks nothing else.
        if self.press_scrollbar(cursor, size) {
            self.unfocus();
            return None;
        }
        // A click moves the focus to the widget under it.
        let (sx, sy) = search_at(w);
        if on(sx, sy, SEARCH_WIDTH, 15.0) {
            self.unfocus();
            self.search.focused = true;
            self.search.reset_blink();
            self.search.click(atlas, cursor.0, false, false);
            return None;
        }
        let rows = self.rows();
        let i = self.hovered(rows.len(), cursor, size)?;
        let Row::Rule(r) = rows[i] else {
            return None;
        };
        if !on(control_x(w), self.row_top(i) + 2.0, 44.0, 20.0) {
            return None;
        }
        self.unfocus();
        match r.default {
            Value::Bool(_) => {
                let on = self.values.bool(r.name);
                self.values.set(r.name, Value::Bool(!on));
            }
            Value::Int(_) => {
                if let Some(b) = self.number(r.name) {
                    b.focused = true;
                    b.reset_blink();
                    b.click(atlas, cursor.0, false, false);
                }
            }
        }
        None
    }
    fn unfocus(&mut self) {
        self.search.focused = false;
        for (_, b) in &mut self.numbers {
            b.focused = false;
        }
    }
    /// Whether a click lands on a button (for its sound).
    pub fn clicks_button(&self, atlas: &HudAtlas, cursor: (f32, f32), size: (f32, f32)) -> bool {
        let (w, h) = size;
        let on = |(x, y): (f32, f32)| ui::hovered(cursor, x, y, 150.0, 20.0);
        if self.confirm {
            return confirm(&atlas.font, size).answer_at(cursor).is_some();
        }
        let [done, cancel] = footer(w, h);
        if (on(done) && !self.invalid()) || on(cancel) {
            return true;
        }
        let rows = self.rows();
        self.hovered(rows.len(), cursor, size).is_some_and(|i| {
            matches!(rows[i], Row::Rule(r) if matches!(r.default, Value::Bool(_)))
                && ui::hovered(cursor, control_x(w), self.row_top(i) + 2.0, 44.0, 20.0)
        })
    }

    pub fn draw(&mut self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
        // AbstractGameRulesScreen: its title; InWorldGameRulesScreen's
        // discard question is a ConfirmScreen (title and message).
        crate::widgets::begin_screen(|| {
            let tr = |key: &str| Text::translate(key, &[]).plain();
            if self.confirm {
                crate::widgets::ScreenInfo::new(
                    "game_rules/discard",
                    crate::widgets::join_for_narration(&[
                        &tr("editGamerule.inGame.discardChanges.title"),
                        &tr("editGamerule.inGame.discardChanges.message"),
                    ]),
                )
            } else {
                crate::widgets::ScreenInfo::new("game_rules", tr("editGamerule.title"))
            }
        });
        if self.confirm {
            let labels = [
                Text::translate("gui.yes", &[]),
                Text::translate("gui.no", &[]),
            ];
            confirm(&atlas.font, (w, h)).draw(batch, atlas, cursor, [&labels[0], &labels[1]]);
            return;
        }
        let rows = self.rows();
        let content = ROW * rows.len() as f32 + 4.0;
        self.scroll.clamp(HEADER, h - FOOTER, content);
        // RuleList: the in-world list background, its entries clipped to it,
        // then the separators and scrollbar.
        ui::list_background(batch, atlas, HEADER, h - FOOTER, self.in_world);
        let first = batch.vertices.len();
        let hovered = self.hovered(rows.len(), cursor, (w, h));
        let (left, x) = (row_left(w), control_x(w));
        // RuleList, a ContainerObjectSelectionList: its rows' widgets are
        // narrated with their row's place.
        let list = crate::widgets::key_of(&"game_rules/list");
        crate::widgets::narratable(|| {
            crate::widgets::Narratable::container_list(
                rows.len(),
                hovered,
                [0.0, HEADER, w, h - FOOTER - HEADER],
            )
            .key(list)
        });
        for (i, row) in rows.iter().enumerate() {
            let top = self.row_top(i);
            if top + ROW < HEADER || top > h - FOOTER {
                continue;
            }
            let (cx, cy) = (left + 2.0, top + 2.0);
            match *row {
                Row::Category(c) => {
                    // A FocusableTextWidget padded by 4, centred in the entry.
                    let label = Text::translate(&category_key(c), &[])
                        .bold()
                        .underlined()
                        .with_color(yellow());
                    let lw = atlas.font.width_text(&label);
                    let mid = cx + (ROW_WIDTH - 4.0) / 2.0;
                    let lx = mid - (lw / 2.0).trunc();
                    batch.text_styled(atlas, &label, lx, cy + 10.0 - 8.0 + 4.0, u32::MAX, true);
                    // CategoryRuleEntry's FocusableTextWidget: its label.
                    crate::widgets::narratable(|| {
                        crate::widgets::Narratable::text(
                            label.plain(),
                            [lx - 4.0, cy, lw + 8.0, 17.0],
                        )
                        .key(crate::widgets::key_of(&("game_rules/category", c)))
                        .child_of(list, i)
                        .without_tooltip()
                    });
                }
                Row::Rule(r) => {
                    let name = lines(&atlas.font, &Text::translate(&name_key(r), &[]), 170.0);
                    match name.as_slice() {
                        [one] => {
                            batch.text_styled(atlas, one, cx, cy + 5.0, u32::MAX, true);
                        }
                        [one, two, ..] => {
                            batch.text_styled(atlas, one, cx, cy, u32::MAX, true);
                            batch.text_styled(atlas, two, cx, cy + 10.0, u32::MAX, true);
                        }
                        [] => {}
                    }
                    let hover = hovered == Some(i) && ui::hovered(cursor, x, cy, 44.0, 20.0);
                    match r.default {
                        Value::Bool(_) => {
                            let key = if self.values.bool(r.name) {
                                "options.on"
                            } else {
                                "options.off"
                            };
                            let label = Text::translate(key, &[]);
                            let value = self.values.bool(r.name);
                            ui::button_as(
                                batch,
                                atlas,
                                x,
                                cy,
                                44.0,
                                20.0,
                                &label,
                                true,
                                hover,
                                |rect| {
                                    bool_narration(r, value, rect)
                                        .key(crate::widgets::key_of(&("game_rules", r.name)))
                                        .child_of(list, i)
                                },
                            );
                        }
                        Value::Int(_) => {
                            if let Some(b) = self.number(r.name) {
                                b.x = x;
                                b.y = cy;
                                // IntegerRuleEntry's box: the rule's name, its
                                // description and default, a line each.
                                if crate::widgets::narrating() {
                                    b.message =
                                        format!("{}\n{}\n", lang::get(&name_key(r)), narration(r));
                                }
                                b.draw(batch, atlas, None);
                                crate::widgets::last_narratable(|n| {
                                    *n = n
                                        .clone()
                                        .key(crate::widgets::key_of(&("game_rules", r.name)))
                                        .child_of(list, i)
                                        .without_tooltip();
                                });
                            }
                        }
                    }
                }
            }
        }
        batch.clip_since(first, 0.0, HEADER, w, h - FOOTER);
        ui::separators(batch, atlas, HEADER, h - FOOTER, self.in_world);
        ui::scrollbar(
            batch,
            atlas,
            Self::bar(w),
            HEADER,
            h - FOOTER,
            content,
            self.scroll.amount.trunc(),
        );
        // The header: the title centred over the search box.
        let title = Text::translate("editGamerule.title", &[]);
        let tw = atlas.font.width_text(&title);
        let (sx, sy) = search_at(w);
        let lw = tw.max(SEARCH_WIDTH);
        let lx = ((w - lw) / 2.0).trunc();
        batch.text_styled(
            atlas,
            &title,
            lx + ((lw - tw) / 2.0).trunc(),
            4.0,
            u32::MAX,
            true,
        );
        self.search.x = sx;
        self.search.y = sy;
        self.search.draw(batch, atlas, None);
        // The header's search box is added before the list.
        crate::widgets::last_narratable_at(0, |_| {});
        if self.search.value.is_empty() && !self.search.focused {
            // EditBox.SEARCH_HINT_STYLE: gray and italic.
            let hint = Text::translate("gui.game_rule.search", &[]).with_style(&Style {
                color: named_color("gray"),
                italic: Some(true),
                ..Style::default()
            });
            let (hx, hy) = (self.search.text_x(), self.search.text_y());
            batch.text_styled(atlas, &hint, hx, hy, u32::MAX, true);
        }
        let invalid = self.invalid();
        for ((bx, by), (key, active)) in footer(w, h)
            .into_iter()
            .zip([("gui.done", !invalid), ("gui.cancel", true)])
        {
            let hover = active && ui::hovered(cursor, bx, by, 150.0, 20.0);
            let label = Text::translate(key, &[]);
            ui::button(batch, atlas, bx, by, 150.0, 20.0, &label, active, hover);
        }
        // The hovered rule's tooltip: its id, description and default.
        if let Some(Row::Rule(r)) = hovered.map(|i| rows[i]) {
            let mut tip = vec![Text::literal(r.name).with_color(yellow())];
            let description = format!("{}.description", name_key(r));
            if lang::has(&description) {
                tip.extend(lines(
                    &atlas.font,
                    &Text::translate(&description, &[]),
                    150.0,
                ));
            }
            let gray = named_color("gray").unwrap_or(0xAAAAAA);
            tip.push(
                Text::translate(
                    "editGamerule.default",
                    &[Text::literal(r.default.to_string())],
                )
                .with_color(gray),
            );
            ui::tooltip(batch, atlas, &tip, cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_narrates_its_name_value_description_and_next_value() {
        use crate::screen_narrator::translate as tr;
        use crate::widgets::What;
        let r = RULES
            .iter()
            .find(|r| matches!(r.default, Value::Bool(_)))
            .unwrap();
        let n = bool_narration(r, true, [0.0; 4]);
        let What::NamedCycle {
            title,
            next,
            active,
        } = &n.what
        else {
            panic!("a CycleButton: {:?}", n.what);
        };
        let full = tr(
            "options.generic_value",
            &[&lang::get(&name_key(r)), &tr("options.on", &[])],
        );
        assert_eq!(
            *title,
            format!("{}\n{}", tr("gui.narrate.button", &[&full]), narration(r))
        );
        assert_eq!(*next, tr("options.off", &[]));
        assert!(*active);
        // The list draws the rule's tooltip, not the button.
        assert_eq!(n.hint.as_deref(), Some(""));
    }

    #[test]
    fn box_colours_are_vanillas() {
        assert_eq!(VALID, crate::hud::rgba(224, 224, 224, 255));
        assert_eq!(INVALID, crate::hud::rgba(255, 0, 0, 255));
    }

    #[test]
    fn numbers_parse_like_brigadier_within_the_rule() {
        let r = gamerules::rule("max_snow_accumulation_height").unwrap();
        assert_eq!(parse(r, "8"), Some(8));
        assert_eq!(parse(r, "9"), None);
        assert_eq!(parse(r, "-1"), None);
        for s in ["", "+3", " 3", "3 ", "3.0", "x"] {
            assert_eq!(parse(r, s), None, "{s:?}");
        }
        let r = gamerules::rule("fire_spread_radius_around_player").unwrap();
        assert_eq!(parse(r, "-1"), Some(-1));
    }

    #[test]
    fn rows_group_categories_and_rules_by_description_id() {
        let s = RulesScreen::new(&GameRules::default());
        let rows = s.rows();
        assert_eq!(rows.len(), RULES.len() + 7);
        assert!(matches!(rows[0], Row::Category("chat")));
        let categories: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Category(c) => Some(*c),
                Row::Rule(_) => None,
            })
            .collect();
        assert_eq!(
            categories,
            [
                "chat", "drops", "misc", "mobs", "player", "spawning", "updates"
            ]
        );
    }

    /// AbstractScrollArea: a press on the bar holds it, a drag moves the
    /// list (to the ends past them), a release lets go.
    #[test]
    fn the_scroll_bar_can_be_held_dragged_and_let_go() {
        let mut s = RulesScreen::new(&GameRules::default());
        let (w, h) = (427.0, 240.0);
        let (top, bottom, content) = (HEADER, h - FOOTER, s.content());
        assert!(
            content > bottom - top,
            "the rules overflow a 240-high screen"
        );
        let bar = RulesScreen::bar(w);
        assert!(!s.press_scrollbar((bar - 4.0, top + 5.0), (w, h)));
        assert!(s.press_scrollbar((bar + 3.0, top + 5.0), (w, h)));
        s.drag(top + 40.0, h);
        assert!(s.scroll.amount > 0.0);
        s.drag(bottom + 10.0, h);
        assert_eq!(s.scroll.amount, content - (bottom - top));
        s.release();
        s.drag(top, h);
        assert_eq!(s.scroll.amount, content - (bottom - top));
    }

    #[test]
    fn edits_change_rules_and_bad_numbers_hold_done() {
        let mut s = RulesScreen::new(&GameRules::default());
        assert!(s.changes().is_empty());
        s.unfocus();
        s.number("random_tick_speed").unwrap().focused = true;
        s.edit(|b| b.set_value("10"));
        assert_eq!(s.changes(), vec![("random_tick_speed", Value::Int(10))]);
        s.edit(|b| b.set_value("ten"));
        assert!(s.invalid());
        // The last valid value stays, as the responder left it.
        assert_eq!(s.changes(), vec![("random_tick_speed", Value::Int(10))]);
        // Leaving with changes asks; No returns, Yes discards.
        assert_eq!(s.escape(), None);
        assert!(s.confirm);
        assert_eq!(s.escape(), None);
        assert!(!s.confirm);
    }
}
