//! The one-line text field every launcher input shares: the state (text, caret, selection,
//! undo), the keyboard (`handle`) and what it draws (`content`). A page keeps its own box and
//! its Enter/Escape; this is what is inside it. Positions count characters, not bytes.

use super::*;
use gpui::{
    black, canvas, Bounds, Font, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point,
    TextRun,
};
use std::{cell::RefCell, rc::Rc};

/// What a key did to the field.
#[derive(Debug, PartialEq)]
pub(super) enum Key {
    Enter,
    Escape,
    /// Not the field's: Tab (focus), shortcuts.
    Pass,
    /// The caret or the selection moved; the text is the same.
    Moved,
    /// The text changed.
    Edited,
    /// Ctrl/Cmd-V: `handle` inserts the clipboard's first line.
    Paste,
    /// Ctrl/Cmd-C: `handle` puts this on the clipboard.
    Copy(String),
    /// Ctrl/Cmd-X: the text changed; `handle` puts this on the clipboard.
    Cut(String),
}

/// What the last edit was, so a run of typing undoes as one step.
#[derive(Clone, Copy, PartialEq)]
enum Run {
    None,
    Typing,
    Deleting,
    Other,
}

/// Where the text was drawn last frame, and how: what turns a mouse position into a character.
struct Area {
    bounds: Bounds<Pixels>,
    font: Font,
    size: Pixels,
    mask: bool,
}

struct Snap {
    text: String,
    caret: usize,
}

pub(super) struct LineEdit {
    text: String,
    caret: usize,
    /// The other end of the selection; equal to `caret` when nothing is selected.
    anchor: usize,
    scroll: usize,
    max: usize,
    /// How many characters the box shows.
    visible: usize,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    run: Run,
    area: Rc<RefCell<Option<Area>>>,
    /// A drag that started in this field is on.
    dragging: bool,
}

impl Default for LineEdit {
    fn default() -> Self {
        LineEdit::new(String::new(), 256, 60)
    }
}

const HISTORY: usize = 100;

impl LineEdit {
    pub(super) fn new(text: String, max: usize, visible: usize) -> Self {
        let caret = text.chars().count();
        let mut e = LineEdit {
            text,
            caret,
            anchor: caret,
            scroll: 0,
            max,
            visible,
            undo: Vec::new(),
            redo: Vec::new(),
            run: Run::None,
            area: Rc::default(),
            dragging: false,
        };
        e.reveal();
        e
    }

    pub(super) fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text from outside (not an edit: no undo), the caret at its end.
    pub(super) fn set(&mut self, text: String) {
        *self = LineEdit::new(text, self.max, self.visible);
    }

    /// The text, leaving the field empty.
    pub(super) fn take(&mut self) -> String {
        let text = self.text.clone();
        self.set(String::new());
        text
    }

    /// How many characters the box shows.
    pub(super) fn show(&mut self, visible: usize) {
        self.visible = visible;
        self.reveal();
    }

    pub(super) fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.len();
    }

    /// The character under `pos`, from where the text was drawn last frame.
    fn index_at(&self, window: &Window, pos: Point<Pixels>) -> Option<usize> {
        let area = self.area.borrow();
        let a = area.as_ref()?;
        let shown: String = self
            .text
            .chars()
            .skip(self.scroll)
            .take(self.visible)
            .collect();
        let shown = if a.mask {
            "•".repeat(shown.chars().count())
        } else {
            shown
        };
        let run = TextRun {
            len: shown.len(),
            font: a.font.clone(),
            color: black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(shown.clone().into(), a.size, &[run], None);
        let at = line
            .closest_index_for_x(pos.x - a.bounds.origin.x)
            .min(shown.len());
        Some(self.scroll + shown[..at].chars().count())
    }

    fn select_word(&mut self, at: usize) {
        self.anchor = self.word(at, false);
        self.caret = self.word(at, true).max(self.anchor);
        self.run = Run::None;
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, at: usize) -> usize {
        self.text
            .char_indices()
            .nth(at)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn range(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    fn selected(&self) -> String {
        let (lo, hi) = self.range();
        self.text.chars().skip(lo).take(hi - lo).collect()
    }

    fn snap(&self) -> Snap {
        Snap {
            text: self.text.clone(),
            caret: self.caret,
        }
    }

    /// Runs an edit and remembers the text it replaced for undo, unless it changed nothing
    /// (typing a "t" over a selected "t") or continues a run. A run of typing is one step per
    /// word: `split` starts a new one.
    fn edit(&mut self, run: Run, split: bool, f: impl FnOnce(&mut Self)) {
        let prev = self.snap();
        let prev_run = self.run;
        f(self);
        if self.text == prev.text {
            self.run = prev_run;
            return;
        }
        if run == Run::Other || run != prev_run || split {
            if self.undo.len() == HISTORY {
                self.undo.remove(0);
            }
            self.undo.push(prev);
        }
        self.redo.clear();
        self.run = run;
    }

    fn restore(&mut self, s: Snap) {
        self.text = s.text;
        self.caret = s.caret;
        self.anchor = s.caret;
        self.run = Run::None;
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snap());
            self.restore(s);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snap());
            self.restore(s);
        }
    }

    fn delete_selection(&mut self) -> bool {
        let (lo, hi) = self.range();
        if lo == hi {
            return false;
        }
        self.text.replace_range(self.byte(lo)..self.byte(hi), "");
        self.caret = lo;
        self.anchor = lo;
        true
    }

    fn put(&mut self, s: &str) {
        self.delete_selection();
        let room = self.max.saturating_sub(self.len());
        let s: String = s.chars().filter(|c| !c.is_control()).take(room).collect();
        self.text.insert_str(self.byte(self.caret), &s);
        self.caret += s.chars().count();
        self.anchor = self.caret;
    }

    /// Replaces the selection with `s` (control characters dropped), up to the length limit.
    pub(super) fn insert(&mut self, s: &str) {
        self.edit(Run::Other, false, |e| e.put(s));
        self.reveal();
    }

    fn move_to(&mut self, at: usize, extend: bool) {
        self.caret = at.min(self.len());
        if !extend {
            self.anchor = self.caret;
        }
        self.run = Run::None;
    }

    /// The start of the word before `from`, or the end of the one after it.
    fn word(&self, from: usize, forward: bool) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = from;
        if forward {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
        } else {
            while i > 0 && chars[i - 1].is_whitespace() {
                i -= 1;
            }
            while i > 0 && !chars[i - 1].is_whitespace() {
                i -= 1;
            }
        }
        i
    }

    /// Applies a key press. `command` is Ctrl, or Cmd on macOS.
    pub(super) fn key(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        command: bool,
        shift: bool,
    ) -> Key {
        let (lo, hi) = self.range();
        let out = match key {
            "enter" => Key::Enter,
            "escape" => Key::Escape,
            "left" if !shift && lo != hi && !command => {
                self.move_to(lo, false);
                Key::Moved
            }
            "right" if !shift && lo != hi && !command => {
                self.move_to(hi, false);
                Key::Moved
            }
            "left" => {
                let to = if command {
                    self.word(self.caret, false)
                } else {
                    self.caret.saturating_sub(1)
                };
                self.move_to(to, shift);
                Key::Moved
            }
            "right" => {
                let to = if command {
                    self.word(self.caret, true)
                } else {
                    self.caret + 1
                };
                self.move_to(to, shift);
                Key::Moved
            }
            "home" => {
                self.move_to(0, shift);
                Key::Moved
            }
            "end" => {
                self.move_to(self.len(), shift);
                Key::Moved
            }
            "backspace" | "delete" => {
                let run = if lo == hi { Run::Deleting } else { Run::Other };
                self.edit(run, false, |e| {
                    if !e.delete_selection() {
                        let (from, to) = match (key, command) {
                            ("backspace", false) => (e.caret.saturating_sub(1), e.caret),
                            ("backspace", true) => (e.word(e.caret, false), e.caret),
                            (_, false) => (e.caret, (e.caret + 1).min(e.len())),
                            (_, true) => (e.caret, e.word(e.caret, true)),
                        };
                        e.anchor = from;
                        e.caret = to;
                        e.delete_selection();
                    }
                });
                Key::Edited
            }
            "a" if command => {
                self.select_all();
                self.run = Run::None;
                Key::Moved
            }
            "z" if command && shift => {
                self.redo();
                Key::Edited
            }
            "z" if command => {
                self.undo();
                Key::Edited
            }
            "y" if command => {
                self.redo();
                Key::Edited
            }
            "c" if command => Key::Copy(self.selected()),
            "x" if command => {
                let cut = self.selected();
                self.edit(Run::Other, false, |e| {
                    e.delete_selection();
                });
                Key::Cut(cut)
            }
            "v" if command => Key::Paste,
            _ if command => Key::Pass,
            // Not every platform gives Space a character.
            "space" => {
                let run = if lo == hi { Run::Typing } else { Run::Other };
                self.edit(run, false, |e| e.put(" "));
                Key::Edited
            }
            _ => match key_char {
                Some(ch) if !ch.is_empty() && !ch.chars().any(char::is_control) => {
                    let starts_word = !ch.starts_with(char::is_whitespace)
                        && self
                            .text
                            .chars()
                            .nth(self.caret.wrapping_sub(1))
                            .is_some_and(char::is_whitespace);
                    let run = if lo == hi { Run::Typing } else { Run::Other };
                    self.edit(run, starts_word, |e| e.put(ch));
                    Key::Edited
                }
                _ => Key::Pass,
            },
        };
        self.reveal();
        out
    }

    /// Scrolls the field so the caret shows.
    fn reveal(&mut self) {
        if self.caret < self.scroll {
            self.scroll = self.caret;
        } else if self.caret > self.scroll + self.visible {
            self.scroll = self.caret - self.visible;
        }
    }

    /// What shows: the text before the selection, the selection, the text after, and whether the
    /// caret sits before the selection (otherwise after it). `whole` shows all of it selected.
    fn view(&self, whole: bool) -> (String, String, String, bool) {
        let (lo, hi) = if whole { (0, self.len()) } else { self.range() };
        let end = self.scroll + self.visible;
        let part = |a: usize, b: usize| -> String {
            let (a, b) = (a.max(self.scroll), b.min(end));
            self.text
                .chars()
                .skip(a)
                .take(b.saturating_sub(a))
                .collect()
        };
        (
            part(0, lo),
            part(lo, hi),
            part(hi, usize::MAX),
            self.caret == lo,
        )
    }
}

/// What a paste puts in a one-line field: the clipboard's first line, spaces at its ends kept
/// (` --world Test` after an argument is a new argument).
fn pasted(clipboard: &str) -> &str {
    clipboard.lines().next().unwrap_or_default()
}

/// The keyboard in a field: moves the caret, edits, copies and pastes. Never returns Paste, Copy
/// or Cut (they are done); Enter, Escape and Pass are the page's.
pub(super) fn handle(edit: &mut LineEdit, ev: &KeyDownEvent, cx: &mut App) -> Key {
    let k = &ev.keystroke;
    let command = k.modifiers.control || k.modifiers.platform;
    match edit.key(&k.key, k.key_char.as_deref(), command, k.modifiers.shift) {
        Key::Paste => {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                edit.insert(pasted(&text));
            }
            Key::Edited
        }
        Key::Copy(s) => {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(s));
            Key::Moved
        }
        Key::Cut(s) => {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(s));
            Key::Edited
        }
        other => other,
    }
}

/// Puts mouse handling on a field's box: a click puts the caret in (Shift extends the selection),
/// a drag selects, a double click selects a word and a triple click the line.
pub(super) fn mouse<T: 'static, E: InteractiveElement>(
    el: E,
    cx: &mut Context<T>,
    edit: impl Fn(&mut T) -> &mut LineEdit + Clone + 'static,
) -> E {
    let get = edit.clone();
    el.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
            let e = get(this);
            if let Some(at) = e.index_at(window, ev.position) {
                match ev.click_count {
                    1 => e.move_to(at, ev.modifiers.shift),
                    2 => e.select_word(at),
                    _ => e.select_all(),
                }
                e.dragging = true;
                cx.notify();
            }
        }),
    )
    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, window, cx| {
        let e = edit(this);
        if !ev.dragging() {
            e.dragging = false;
        } else if e.dragging {
            if let Some(at) = e.index_at(window, ev.position) {
                e.move_to(at, true);
                cx.notify();
            }
        }
    }))
}

/// What a field draws inside its box: the placeholder while empty and unfocused, else the text
/// with its selection and caret. `mask` shows dots (a token); `whole` shows all selected.
pub(super) fn content(
    edit: &LineEdit,
    t: &theme::Theme,
    focused: bool,
    whole: bool,
    mask: bool,
    placeholder: &'static str,
) -> Div {
    if edit.text.is_empty() && !focused {
        return div().text_color(t.muted).child(placeholder);
    }
    let (before, selected, after, caret_first) = edit.view(whole);
    let text = |s: String| {
        let s = if mask {
            "•".repeat(s.chars().count())
        } else {
            s
        };
        div().flex_none().whitespace_nowrap().child(s)
    };
    let caret = || div().flex_none().w(px(1.)).h(px(14.)).bg(t.ink);
    let area = edit.area.clone();
    div()
        .relative()
        .flex()
        .items_center()
        .text_color(t.ink)
        .child(text(before))
        .when(focused && caret_first, |d| d.child(caret()))
        .when(!selected.is_empty(), |d| {
            d.child(
                text(selected)
                    .rounded(px(2.))
                    .bg(t.accent.opacity(theme::SELECTION)),
            )
        })
        .when(focused && !caret_first, |d| d.child(caret()))
        .child(text(after))
        .child(
            // Records where the text sits and in what font, for the mouse.
            canvas(
                move |bounds, window, _| {
                    let style = window.text_style();
                    *area.borrow_mut() = Some(Area {
                        bounds,
                        font: style.font(),
                        size: style.font_size.to_pixels(window.rem_size()),
                        mask,
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(text: &str) -> LineEdit {
        LineEdit::new(text.into(), 1024, 38)
    }

    fn typed(e: &mut LineEdit, s: &str) {
        for c in s.chars() {
            let key = if c == ' ' { "space" } else { "x" };
            let ch = c.to_string();
            e.key(key, (c != ' ').then_some(ch.as_str()), false, false);
        }
    }

    #[test]
    fn spaces_type_and_the_caret_moves_and_edits_in_the_middle() {
        let mut e = edit("");
        typed(&mut e, "--name Steve");
        assert_eq!(e.text(), "--name Steve");
        for _ in 0..5 {
            e.key("left", None, false, false);
        }
        typed(&mut e, "X");
        assert_eq!(e.text(), "--name XSteve");
        e.key("backspace", None, false, false);
        e.key("delete", None, false, false);
        assert_eq!(e.text(), "--name teve");
        e.key("home", None, false, false);
        e.key("delete", None, false, false);
        assert_eq!(e.text(), "-name teve");
    }

    #[test]
    fn undo_steps_back_a_run_at_a_time_and_redo_returns() {
        let mut e = edit("");
        typed(&mut e, "--name Steve");
        e.key("home", None, false, false);
        e.key("delete", None, false, false);
        e.key("delete", None, false, false);
        assert_eq!(e.text(), "name Steve");
        e.key("z", None, true, false);
        assert_eq!(e.text(), "--name Steve");
        // Typing undoes a word at a time.
        e.key("z", None, true, false);
        assert_eq!(e.text(), "--name ");
        e.key("z", None, true, false);
        assert_eq!(e.text(), "");
        e.key("z", None, true, false);
        assert_eq!(e.text(), "");
        e.key("y", None, true, false);
        assert_eq!(e.text(), "--name ");
        e.key("y", None, true, false);
        assert_eq!(e.text(), "--name Steve");
        e.key("z", None, true, false);
        e.key("z", None, true, true);
        assert_eq!(e.text(), "--name Steve");
        // A paste is a step of its own, and typing after an undo drops the redo.
        e.key("end", None, false, false);
        e.insert("!");
        e.key("z", None, true, false);
        assert_eq!(e.text(), "--name Steve");
        typed(&mut e, "?");
        e.key("y", None, true, false);
        assert_eq!(e.text(), "--name Steve?");
    }

    #[test]
    fn selection_is_replaced_copied_and_cut() {
        let mut e = edit("--name Steve");
        e.key("a", None, true, false);
        assert_eq!(
            e.key("c", None, true, false),
            Key::Copy("--name Steve".into())
        );
        e.key("home", None, false, false);
        for _ in 0..2 {
            e.key("right", None, false, true);
        }
        assert_eq!(e.key("x", None, true, false), Key::Cut("--".into()));
        assert_eq!(e.text(), "name Steve");
        e.key("a", None, true, false);
        e.key("y", Some("y"), false, false);
        assert_eq!(e.text(), "y");
        e.insert("a\nb c");
        assert_eq!(e.text(), "yab c");
    }

    #[test]
    fn words_and_the_length_limit() {
        let mut e = edit("one two three");
        e.key("backspace", None, true, false);
        assert_eq!(e.text(), "one two ");
        e.key("left", None, true, false);
        e.key("delete", None, true, false);
        assert_eq!(e.text(), "one  ");
        let mut e = LineEdit::new("abc".into(), 4, 38);
        e.insert("defg");
        assert_eq!(e.text(), "abcd");
        assert_eq!(e.key("tab", None, false, false), Key::Pass);
        assert_eq!(e.key("q", None, true, false), Key::Pass);
        assert_eq!(e.key("enter", Some("\n"), false, false), Key::Enter);
        assert_eq!(e.key("escape", None, false, false), Key::Escape);
    }

    #[test]
    fn an_edit_that_changes_nothing_is_not_an_undo_step() {
        let mut e = edit("test");
        e.key("left", None, false, false);
        e.key("end", None, false, false);
        e.key("left", None, false, true);
        e.key("t", Some("t"), false, false);
        assert_eq!(e.text(), "test");
        e.key("delete", None, false, false);
        e.key("home", None, false, false);
        e.key("backspace", None, false, false);
        e.key("z", None, true, false);
        assert_eq!(e.text(), "test");
        e.insert("");
        e.key("x", Some("x"), false, false);
        assert_eq!(e.text(), "xtest");
        e.key("z", None, true, false);
        assert_eq!(e.text(), "test");
    }

    #[test]
    fn set_resets_the_history_and_a_selected_suggestion_is_replaced() {
        let mut e = edit("My server");
        e.select_all();
        e.key("f", Some("F"), false, false);
        assert_eq!(e.text(), "F");
        e.set("Other".into());
        e.key("z", None, true, false);
        assert_eq!(e.text(), "Other");
        assert_eq!(e.take(), "Other");
        assert_eq!(e.text(), "");
    }

    #[test]
    fn the_view_follows_the_caret() {
        let mut e = edit(&"é".repeat(100));
        let (before, sel, after, _) = e.view(false);
        assert_eq!((before.chars().count(), sel.len(), after.len()), (38, 0, 0));
        e.key("home", None, false, false);
        assert_eq!(e.view(false).2.chars().count(), 38);
        assert_eq!(e.view(true).1.chars().count(), 38);
    }
}
