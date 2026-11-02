//! The on-screen keyboard a controller types with in focus mode (Horizon's
//! own; docs/controllers.md, "Text fields"): a QWERTY grid and a symbols
//! grid in Horizon's design language, moved over with the D-pad or stick.
//! A types the key, X deletes, Y is a space, Start is Enter, B closes it.
//! It only produces [`Output`]s; the game feeds them to the focused field
//! the way typed keys reach it.
use crate::gamepad::{Button, GlyphStyle};
use crate::ui_theme::{self, Face, Ui, color};

/// A label of the keyboard: the language table's, or the English text.
fn tr(key: &str, english: &str) -> String {
    crate::lang::get_or(key, english)
}

/// What a key press does to the focused field.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    Text(String),
    Backspace,
    Enter,
    /// The keyboard closes; the field keeps its text.
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Key {
    Char(char),
    Shift,
    Layer,
    Space,
    Backspace,
    Done,
}

/// A row of keys and their widths in key units (every row is 10 wide).
type Row = &'static [(Key, f32)];

const fn c(ch: char) -> (Key, f32) {
    (Key::Char(ch), 1.0)
}
const BOTTOM: Row = &[
    (Key::Shift, 1.5),
    (Key::Layer, 1.5),
    (Key::Space, 4.0),
    (Key::Backspace, 1.5),
    (Key::Done, 1.5),
];
const LETTERS: [Row; 5] = [
    &[
        c('1'),
        c('2'),
        c('3'),
        c('4'),
        c('5'),
        c('6'),
        c('7'),
        c('8'),
        c('9'),
        c('0'),
    ],
    &[
        c('q'),
        c('w'),
        c('e'),
        c('r'),
        c('t'),
        c('y'),
        c('u'),
        c('i'),
        c('o'),
        c('p'),
    ],
    &[
        c('a'),
        c('s'),
        c('d'),
        c('f'),
        c('g'),
        c('h'),
        c('j'),
        c('k'),
        c('l'),
        c('\''),
    ],
    &[
        c('z'),
        c('x'),
        c('c'),
        c('v'),
        c('b'),
        c('n'),
        c('m'),
        c(','),
        c('.'),
        c('-'),
    ],
    BOTTOM,
];
const SYMBOLS: [Row; 5] = [
    &[
        c('!'),
        c('@'),
        c('#'),
        c('$'),
        c('%'),
        c('^'),
        c('&'),
        c('*'),
        c('('),
        c(')'),
    ],
    &[
        c('/'),
        c('\\'),
        c('_'),
        c('='),
        c('+'),
        c('['),
        c(']'),
        c('{'),
        c('}'),
        c('|'),
    ],
    &[
        c(';'),
        c(':'),
        c('"'),
        c('<'),
        c('>'),
        c('?'),
        c('~'),
        c('`'),
        c('§'),
        c('°'),
    ],
    &[
        c('é'),
        c('è'),
        c('à'),
        c('ç'),
        c('ù'),
        c('ü'),
        c('ö'),
        c('ä'),
        c('ß'),
        c('ñ'),
    ],
    BOTTOM,
];

/// The keyboard's state while open.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyboard {
    row: usize,
    col: usize,
    symbols: bool,
    /// The next letter is upper case (Shift), or every letter (double Shift).
    shift: bool,
    caps: bool,
}
impl Default for Keyboard {
    fn default() -> Self {
        // Focus starts on `q`.
        Self {
            row: 1,
            col: 0,
            symbols: false,
            shift: false,
            caps: false,
        }
    }
}

impl Keyboard {
    fn rows(&self) -> &'static [Row; 5] {
        if self.symbols { &SYMBOLS } else { &LETTERS }
    }
    /// A key's left edge and width in key units.
    fn span(row: Row, col: usize) -> (f32, f32) {
        let left: f32 = row[..col].iter().map(|k| k.1).sum();
        (left, row[col].1)
    }
    fn key(&self) -> Key {
        self.rows()[self.row][self.col].0
    }
    /// A D-pad step: across a row with wrap-around; up and down to the key
    /// under the focused key's centre (wrapping top and bottom).
    pub fn step(&mut self, button: Button) {
        let rows = self.rows();
        let row = rows[self.row];
        match button {
            Button::DPadLeft => self.col = (self.col + row.len() - 1) % row.len(),
            Button::DPadRight => self.col = (self.col + 1) % row.len(),
            Button::DPadUp | Button::DPadDown => {
                let (left, width) = Self::span(row, self.col);
                let centre = left + width / 2.0;
                self.row = if button == Button::DPadUp {
                    (self.row + rows.len() - 1) % rows.len()
                } else {
                    (self.row + 1) % rows.len()
                };
                let next = rows[self.row];
                self.col = (0..next.len())
                    .find(|&i| {
                        let (l, w) = Self::span(next, i);
                        centre < l + w
                    })
                    .unwrap_or(next.len() - 1);
            }
            _ => {}
        }
    }
    fn letter(&self, ch: char) -> String {
        if self.shift || self.caps {
            ch.to_uppercase().collect()
        } else {
            ch.to_string()
        }
    }
    /// A pressed: the focused key's output (Shift and the symbols switch only
    /// change the keyboard).
    pub fn press(&mut self) -> Option<Output> {
        match self.key() {
            Key::Char(ch) => {
                let text = self.letter(ch);
                self.shift = false;
                Some(Output::Text(text))
            }
            Key::Shift => {
                // Shift, then Shift again locks capitals; a third press lets go.
                (self.shift, self.caps) = match (self.shift, self.caps) {
                    (false, false) => (true, false),
                    (true, false) => (false, true),
                    _ => (false, false),
                };
                None
            }
            Key::Layer => {
                self.symbols = !self.symbols;
                self.col = self.col.min(self.rows()[self.row].len() - 1);
                None
            }
            Key::Space => Some(Output::Text(" ".into())),
            Key::Backspace => Some(Output::Backspace),
            Key::Done => Some(Output::Enter),
        }
    }
    /// A pad button while the keyboard is open; `None` when it does nothing
    /// to the field (moving, Shift).
    pub fn button(&mut self, button: Button) -> Option<Output> {
        match button {
            Button::South => self.press(),
            Button::West => Some(Output::Backspace),
            Button::North => Some(Output::Text(" ".into())),
            Button::Start => Some(Output::Enter),
            Button::East => Some(Output::Close),
            Button::LeftStick | Button::LeftTrigger => {
                self.shift = !self.shift;
                None
            }
            Button::RightBumper | Button::LeftBumper => {
                self.symbols = !self.symbols;
                self.col = self.col.min(self.rows()[self.row].len() - 1);
                None
            }
            Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight => {
                self.step(button);
                None
            }
            _ => None,
        }
    }

    /// Draw the keyboard across the bottom of the screen, or the top when
    /// the field it types into is in the lower half.
    pub fn draw(&self, ui: &mut Ui, at_top: bool, glyphs: GlyphStyle) {
        const UNIT: f32 = 50.0;
        const GAP: f32 = 6.0;
        const KEY_H: f32 = 38.0;
        let width = 10.0 * UNIT + 9.0 * GAP;
        let height = 5.0 * KEY_H + 4.0 * GAP + 24.0 + 30.0;
        let x0 = ((ui.w - width) / 2.0).round();
        let y0 = if at_top { 16.0 } else { ui.h - height - 16.0 };
        ui.rounded(
            x0 - 12.0,
            y0 - 12.0,
            width + 24.0,
            height,
            ui_theme::RADIUS_PLATE,
            color::with_alpha(color::BG, 0.94),
        );
        ui.border(
            x0 - 12.0,
            y0 - 12.0,
            width + 24.0,
            height,
            ui_theme::RADIUS_PLATE,
            color::BORDER,
        );
        for (r, row) in self.rows().iter().enumerate() {
            let y = y0 + r as f32 * (KEY_H + GAP);
            for (i, (key, units)) in row.iter().enumerate() {
                let (left, _) = Self::span(row, i);
                let x = x0 + left * (UNIT + GAP);
                let w = units * UNIT + (units - 1.0) * GAP;
                let focused = r == self.row && i == self.col;
                let lit = matches!(key, Key::Shift) && (self.shift || self.caps)
                    || matches!(key, Key::Layer) && self.symbols;
                let (plate, ink) = if focused {
                    (color::ACCENT, color::ON_ACCENT)
                } else if lit {
                    (color::HOVER, color::TEXT)
                } else {
                    (color::RAISED, color::TEXT)
                };
                ui.rounded(x, y, w, KEY_H, ui_theme::RADIUS, plate);
                let label = match key {
                    Key::Char(ch) => self.letter(*ch),
                    Key::Shift if self.caps => "CAPS".into(),
                    Key::Shift => tr("horizon.osk.shift", "Shift"),
                    Key::Layer if self.symbols => "abc".into(),
                    Key::Layer => "&123".into(),
                    Key::Space => tr("horizon.osk.space", "Space"),
                    Key::Backspace => tr("horizon.osk.delete", "Delete"),
                    Key::Done => tr("gui.done", "Done"),
                };
                let lw = ui.measure(Face::Medium, ui_theme::BODY, &label);
                ui.text_line(
                    Face::Medium,
                    ui_theme::BODY,
                    KEY_H,
                    x + ((w - lw) / 2.0).round(),
                    y,
                    ink,
                    &label,
                );
            }
        }
        let hints = [
            (Button::South, tr("horizon.osk.type", "Type")),
            (Button::West, tr("horizon.osk.delete", "Delete")),
            (Button::North, tr("horizon.osk.space", "Space")),
            (Button::LeftTrigger, tr("horizon.osk.shift", "Shift")),
            (Button::RightBumper, tr("horizon.osk.symbols", "Symbols")),
            (Button::Start, tr("gui.done", "Done")),
            (Button::East, tr("horizon.osk.close", "Close")),
        ];
        let line: Vec<String> = hints
            .iter()
            .map(|(b, what)| format!("{} {what}", b.label(glyphs)))
            .collect();
        ui.text_centered(
            Face::Regular,
            ui_theme::SMALL,
            x0 + width / 2.0,
            y0 + 5.0 * (KEY_H + GAP) + 2.0,
            color::MUTED,
            &line.join("   "),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_ten_units_wide() {
        for rows in [&LETTERS, &SYMBOLS] {
            for row in rows {
                let units: f32 = row.iter().map(|k| k.1).sum();
                assert_eq!(units, 10.0);
            }
        }
    }

    #[test]
    fn types_shifts_and_moves() {
        let mut k = Keyboard::default();
        assert_eq!(k.press(), Some(Output::Text("q".into())));
        k.step(Button::DPadRight);
        k.button(Button::LeftTrigger);
        assert_eq!(k.press(), Some(Output::Text("W".into())));
        assert_eq!(k.press(), Some(Output::Text("w".into())), "one-shot shift");
        // Left wraps to the end of the row.
        k.col = 0;
        k.step(Button::DPadLeft);
        assert_eq!(k.press(), Some(Output::Text("p".into())));
        // Down from `p` to the row of `'`, then the bottom row: `Done` sits
        // under the right edge; up from the wide Space bar lands mid-row.
        k.step(Button::DPadDown);
        k.step(Button::DPadDown);
        k.step(Button::DPadDown);
        assert_eq!(k.press(), Some(Output::Enter));
        k.col = 2;
        assert_eq!(k.press(), Some(Output::Text(" ".into())));
        k.step(Button::DPadUp);
        assert_eq!(k.press(), Some(Output::Text("n".into())));
        // Up from the top row wraps to the bottom row.
        k.row = 0;
        k.col = 0;
        k.step(Button::DPadUp);
        assert_eq!(k.key(), Key::Shift);
        // Shift twice locks capitals; the symbols grid has no case.
        k.press();
        k.press();
        k.row = 1;
        k.col = 0;
        assert_eq!(k.press(), Some(Output::Text("Q".into())));
        assert_eq!(k.press(), Some(Output::Text("Q".into())));
        k.button(Button::RightBumper);
        assert_eq!(k.press(), Some(Output::Text("/".into())));
        assert_eq!(k.button(Button::West), Some(Output::Backspace));
        assert_eq!(k.button(Button::East), Some(Output::Close));
        assert_eq!(k.button(Button::Start), Some(Output::Enter));
    }
}
