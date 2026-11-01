//! Input method composition text (`PreeditEvent`, `IMEPreeditOverlay`,
//! `TextInputManager`): the text an input method is still composing, shown
//! under the focused text field's caret, and the area the system's candidate
//! window is placed at.
//!
//! Vanilla hands each `PreeditEvent` to the focused widget, which keeps an
//! overlay and reports its caret every frame. Horizon draws its fields from
//! many places, so the event is kept here and every focused field calls
//! [`report_caret`] where it draws its caret; the game draws the overlay last
//! with [`draw_overlay`] and applies [`take_text_input_area`] to the window.
use crate::{
    hud::{HudAtlas, HudBatch},
    text::{Style, Text},
    ui::nine_slice,
};
use std::{sync::Mutex, time::Instant};

/// `PreeditEvent`: the composition, its caret and the blocks of text the
/// input method set apart (the one being converted is `focused_block`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreeditEvent {
    pub full_text: String,
    /// Byte offset of the caret in `full_text` (vanilla's UTF-16 offset).
    pub caret_position: usize,
    pub blocks: Vec<String>,
    pub focused_block: usize,
}

impl PreeditEvent {
    /// `PreeditEvent.fromSdlTextEditing`: `selection_start` and
    /// `selection_length` are in code points; a negative start puts the caret
    /// at the end. Empty text is no composition.
    pub fn from_sdl_text_editing(
        text: Option<&str>,
        selection_start: i32,
        selection_length: i32,
    ) -> Option<Self> {
        let text = text.filter(|t| !t.is_empty())?;
        let total = text.chars().count();
        let caret_codepoint = if selection_start < 0 {
            total
        } else {
            (selection_start as usize).min(total)
        };
        let caret_char = offset_by_codepoints(text, caret_codepoint);
        let single = |caret| Self {
            full_text: text.to_owned(),
            caret_position: caret,
            blocks: vec![text.to_owned()],
            focused_block: 0,
        };
        if selection_length <= 0 {
            return Some(single(caret_char));
        }
        let end_codepoint = (caret_codepoint + selection_length as usize).min(total);
        let end_char = offset_by_codepoints(text, end_codepoint);
        if end_char <= caret_char {
            return Some(single(caret_char));
        }
        let mut blocks = Vec::new();
        let mut focused_block = 0;
        if caret_char > 0 {
            blocks.push(text[..caret_char].to_owned());
            focused_block = 1;
        }
        blocks.push(text[caret_char..end_char].to_owned());
        if end_char < text.len() {
            blocks.push(text[end_char..].to_owned());
        }
        Some(Self {
            full_text: text.to_owned(),
            caret_position: caret_char,
            blocks,
            focused_block,
        })
    }

    /// winit's `Ime::Preedit`: `cursor` is the byte range of the part being
    /// converted, `None` when the input method hides its cursor. SDL's
    /// `TEXTEDITING` says the same in code points.
    pub fn from_winit(text: &str, cursor: Option<(usize, usize)>) -> Option<Self> {
        let (start, length) = match cursor {
            Some((from, to)) => {
                let from = from.min(text.len());
                let to = to.clamp(from, text.len());
                let start = text.get(..from).map_or(-1, |s| s.chars().count() as i32);
                let length = text.get(from..to).map_or(0, |s| s.chars().count() as i32);
                (start, length)
            }
            None => (-1, 0),
        };
        Self::from_sdl_text_editing(Some(text), start, length)
    }

    /// `PreeditEvent.toFormattedText`.
    pub fn to_formatted_text(&self, focused_style: &Style) -> Text {
        if self.blocks.len() == 1 {
            return Text::literal(self.blocks[0].clone()).with_style(focused_style);
        }
        let mut result = Text::empty();
        for (i, block) in self.blocks.iter().enumerate() {
            let mut part = Text::literal(block.clone());
            if i == self.focused_block {
                part = part.with_style(focused_style);
            }
            result.push(part);
        }
        result
    }
}

/// `String.offsetByCodePoints(0, n)`: the byte offset after `n` characters.
fn offset_by_codepoints(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map_or(text.len(), |(i, _)| i)
}

/// The caret a field reported this frame, in GUI pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Caret {
    x: f32,
    y: f32,
    input_height: f32,
}

#[derive(Default)]
struct State {
    /// `KeyboardHandler.lastPreeditEvent`, with when the overlay was created.
    preedit: Option<(PreeditEvent, Instant)>,
    caret: Option<Caret>,
    area: Option<[f32; 4]>,
}

static STATE: Mutex<State> = Mutex::new(State {
    preedit: None,
    caret: None,
    area: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// `KeyboardHandler.textEditing`: the input method's composition changed
/// (`None`: it ended). A new `IMEPreeditOverlay` restarts the caret blink.
pub fn set_preedit(event: Option<PreeditEvent>) {
    let mut s = state();
    s.preedit = event.map(|e| (e, Instant::now()));
}

/// Whether text is being composed.
pub fn composing() -> bool {
    state().preedit.is_some()
}

/// What the focused field's `extractRenderState` ends with: its caret is at
/// (`x`, `y`) in GUI pixels and `input_height` tall (a line plus one).
/// Without a composition that is the area for the candidate window; with one
/// it is where the overlay goes ([`draw_overlay`]).
pub fn report_caret(x: f32, y: f32, input_height: f32) {
    state().caret = Some(Caret { x, y, input_height });
}

/// `IMEPreeditOverlay.extractRenderState` for the caret reported this frame,
/// drawn on top of everything in `batch`. Without a composition the caret is
/// the text input area (`TextInputManager.setTextInputArea`).
pub fn draw_overlay(batch: &mut HudBatch, atlas: &HudAtlas) {
    let mut s = state();
    let Some(caret) = s.caret.take() else {
        return;
    };
    let Some((event, init)) = s.preedit.as_ref() else {
        s.area = Some([
            caret.x,
            caret.y,
            caret.x + 1.0,
            caret.y + caret.input_height,
        ]);
        return;
    };
    let font = &atlas.font;
    let focused = Style {
        underlined: Some(true),
        ..Style::default()
    };
    let black = 0xFF00_0000;
    let text = event.to_formatted_text(&focused).with_color(black);
    let width = font.width_text(&text);
    let caret_pos = font.width(&event.full_text[..event.caret_position]);
    let blink_visible = (init.elapsed().as_millis() / 300).is_multiple_of(2);
    let line = 9.0;
    let (gui_w, gui_h) = (batch.width, batch.height);
    let mut left = caret.x;
    let mut right = left + width;
    if right > gui_w {
        left = gui_w - width;
        right = left + width;
    }
    let input_bottom = caret.y + caret.input_height;
    let mut bottom = input_bottom + 4.0 + line;
    if bottom > gui_h {
        bottom = caret.y - 4.0 - line;
    }
    let top = bottom - line;
    s.area = Some([left, top, right, bottom]);
    drop(s);
    nine_slice(
        batch,
        atlas,
        "gui/sprites/widget/preedit",
        left - 5.0,
        top - 5.0,
        right - left + 10.0,
        bottom - top + 10.0,
        u32::MAX,
    );
    batch.text_styled(atlas, &text, left, top, black, false);
    if blink_visible {
        batch.fill(atlas, left + caret_pos, top - 1.0, 1.0, 11.0, black);
    }
}

/// The text input area (`x0, y0, x1, y1` in GUI pixels) set since the last
/// call, for `Window.set_ime_cursor_area`.
pub fn take_text_input_area() -> Option<[f32; 4]> {
    state().area.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_no_composition() {
        assert_eq!(PreeditEvent::from_sdl_text_editing(None, 0, 0), None);
        assert_eq!(PreeditEvent::from_sdl_text_editing(Some(""), 0, 0), None);
    }

    #[test]
    fn a_negative_start_puts_the_caret_at_the_end() {
        let e = PreeditEvent::from_sdl_text_editing(Some("abc"), -1, 0).unwrap();
        assert_eq!(
            (e.caret_position, e.blocks, e.focused_block),
            (3, vec!["abc".into()], 0)
        );
    }

    #[test]
    fn the_caret_counts_code_points() {
        // Two code points before the caret, the second outside the BMP.
        let e = PreeditEvent::from_sdl_text_editing(Some("a\u{1F600}b"), 2, 0).unwrap();
        assert_eq!(e.caret_position, "a\u{1F600}".len());
        let past = PreeditEvent::from_sdl_text_editing(Some("ab"), 9, 0).unwrap();
        assert_eq!(past.caret_position, 2);
    }

    #[test]
    fn a_selection_splits_the_text_into_blocks() {
        let e = PreeditEvent::from_sdl_text_editing(Some("abcdef"), 2, 2).unwrap();
        assert_eq!(e.blocks, ["ab", "cd", "ef"]);
        assert_eq!((e.focused_block, e.caret_position), (1, 2));
        let first = PreeditEvent::from_sdl_text_editing(Some("abcdef"), 0, 3).unwrap();
        assert_eq!(first.blocks, ["abc", "def"]);
        assert_eq!(first.focused_block, 0);
        let last = PreeditEvent::from_sdl_text_editing(Some("abcdef"), 3, 99).unwrap();
        assert_eq!(last.blocks, ["abc", "def"]);
        assert_eq!(last.focused_block, 1);
    }

    #[test]
    fn winit_byte_ranges_become_code_points() {
        let text = "\u{3042}\u{3044}\u{3046}";
        // Converting the middle character (bytes 3..6).
        let e = PreeditEvent::from_winit(text, Some((3, 6))).unwrap();
        assert_eq!(e.blocks, ["\u{3042}", "\u{3044}", "\u{3046}"]);
        assert_eq!(e.focused_block, 1);
        // A hidden cursor sits at the end.
        let hidden = PreeditEvent::from_winit(text, None).unwrap();
        assert_eq!(hidden.caret_position, text.len());
        assert_eq!(hidden.blocks.len(), 1);
    }

    #[test]
    fn only_the_focused_block_is_underlined() {
        let focused = Style {
            underlined: Some(true),
            ..Style::default()
        };
        let e = PreeditEvent::from_sdl_text_editing(Some("abcdef"), 2, 2).unwrap();
        let styled: Vec<(char, bool)> = e
            .to_formatted_text(&focused)
            .chars()
            .map(|(c, s)| (c, s.underlined == Some(true)))
            .collect();
        let underlined: String = styled.iter().filter(|x| x.1).map(|x| x.0).collect();
        assert_eq!(underlined, "cd");
        assert_eq!(styled.len(), 6);
        let one = PreeditEvent::from_sdl_text_editing(Some("ab"), 0, 0).unwrap();
        assert!(
            one.to_formatted_text(&focused)
                .chars()
                .all(|(_, s)| s.underlined == Some(true))
        );
    }
}
