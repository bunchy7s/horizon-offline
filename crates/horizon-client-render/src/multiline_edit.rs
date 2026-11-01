//! `MultiLineEditBox` over `MultilineTextField` (and `AbstractTextAreaWidget`'s
//! frame and scroll bar): a text area that wraps its text to the box, with a
//! caret and selection that move by line, a character limit, a line limit and
//! a scroll bar. Indices count characters, as `EditBox`'s do.
use crate::{
    clipboard::{Clipboard, EditShortcut, filter_text},
    font::Font,
    hud::{HudAtlas, HudBatch, rgba},
    text::Style,
    ui::{self, Scroll, java_whitespace},
};
use std::time::Instant;
use winit::keyboard::KeyCode;

/// `AbstractTextAreaWidget.INNER_PADDING`.
const PADDING: f32 = 4.0;
/// `Font.lineHeight`.
const LINE: f32 = 9.0;
/// `AbstractScrollArea.defaultSettings((int) (9.0 / 2.0))`'s scroll rate.
const SCROLL_RATE: f32 = 4.0;

/// `StringSplitter.splitLines(value, width, Style.EMPTY, false, ..)`: each
/// line's `[begin, end)`. A line ends at a `\n`, or once a character overflows
/// the width (never the first visible one) at the last space so far, or before
/// that character; the `\n` or space at the end is skipped.
pub fn split_ranges(font: &Font, chars: &[char], width: f32) -> Vec<(usize, usize)> {
    let max_width = width.max(1.0);
    let style = Style::default();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let (mut line_width, mut had_visible, mut last_space) = (0.0f32, false, None);
        let mut line_break = None;
        for (i, &c) in chars.iter().enumerate().skip(start) {
            if c == '\n' {
                line_break = Some(i);
                break;
            }
            if c == ' ' {
                last_space = Some(i);
            }
            let advance = font.advance(c, &style);
            line_width += advance;
            if !had_visible || line_width <= max_width {
                had_visible |= advance != 0.0;
            } else {
                line_break = Some(last_space.unwrap_or(i));
                break;
            }
        }
        let Some(at) = line_break else {
            out.push((start, chars.len()));
            break;
        };
        out.push((start, at));
        start = if matches!(chars[at], '\n' | ' ') {
            at + 1
        } else {
            at
        };
    }
    out
}

/// `MultiLineEditBox` with its `MultilineTextField`.
pub struct MultiLineEdit {
    chars: Vec<char>,
    cursor: usize,
    select_cursor: usize,
    selecting: bool,
    /// `characterLimit` (`MultilineTextField.NO_LIMIT` is `None`).
    character_limit: Option<usize>,
    line_limit: Option<usize>,
    /// `displayLines`.
    lines: Vec<(usize, usize)>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub focused: bool,
    focused_at: Instant,
    pub scroll: Scroll,
}

impl MultiLineEdit {
    pub fn new(font: &Font, x: f32, y: f32, width: f32, height: f32) -> Self {
        let mut edit = Self {
            chars: Vec::new(),
            cursor: 0,
            select_cursor: 0,
            selecting: false,
            character_limit: None,
            line_limit: None,
            lines: Vec::new(),
            x,
            y,
            width,
            height,
            focused: false,
            focused_at: Instant::now(),
            scroll: Scroll::default(),
        };
        edit.reflow(font);
        edit
    }

    /// `AbstractTextAreaWidget.totalInnerPadding`.
    fn text_width(&self) -> f32 {
        self.width - 2.0 * PADDING
    }
    pub fn set_character_limit(&mut self, limit: usize) {
        self.character_limit = Some(limit);
    }
    pub fn set_line_limit(&mut self, limit: usize) {
        self.line_limit = Some(limit);
    }
    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
    pub fn lines(&self) -> &[(usize, usize)] {
        &self.lines
    }
    pub fn has_selection(&self) -> bool {
        self.select_cursor != self.cursor
    }
    /// `getSelected`.
    fn selected(&self) -> (usize, usize) {
        (
            self.select_cursor.min(self.cursor),
            self.select_cursor.max(self.cursor),
        )
    }
    pub fn selected_text(&self) -> String {
        let (begin, end) = self.selected();
        self.chars[begin..end].iter().collect()
    }

    /// `setValue(value, false)`: kept as it was when the text has more lines
    /// than the limit allows.
    pub fn set_value(&mut self, font: &Font, value: &str) {
        let mut chars: Vec<char> = value.chars().collect();
        if let Some(limit) = self.character_limit {
            chars.truncate(limit);
        }
        if !self.overflows_line_limit(font, &chars) {
            self.chars = chars;
            self.cursor = self.chars.len();
            self.select_cursor = self.cursor;
            self.on_value_change(font);
        }
    }

    /// `overflowsLineLimit`.
    fn overflows_line_limit(&self, font: &Font, value: &[char]) -> bool {
        let Some(limit) = self.line_limit else {
            return false;
        };
        let lines = split_ranges(font, value, self.text_width()).len();
        // `StringUtil.endsWithNewLine`.
        lines + usize::from(value.last().is_some_and(|c| matches!(c, '\n' | '\r'))) > limit
    }

    /// `insertText`.
    pub fn insert_text(&mut self, font: &Font, input: &str) {
        if input.is_empty() && !self.has_selection() {
            return;
        }
        let mut text: Vec<char> = filter_text(input, true).chars().collect();
        if let Some(limit) = self.character_limit {
            text.truncate(limit.saturating_sub(self.chars.len()));
        }
        let (begin, end) = self.selected();
        let mut value = self.chars[..begin].to_vec();
        value.extend_from_slice(&text);
        value.extend_from_slice(&self.chars[end..]);
        if !self.overflows_line_limit(font, &value) {
            self.chars = value;
            self.cursor = begin + text.len();
            self.select_cursor = self.cursor;
            self.on_value_change(font);
        }
    }

    /// `deleteText`.
    pub fn delete_text(&mut self, font: &Font, dir: isize) {
        if !self.has_selection() {
            self.select_cursor = self.cursor.saturating_add_signed(dir).min(self.chars.len());
        }
        self.insert_text(font, "");
    }

    /// `onValueChange`: the lines again, and the cursor in view.
    fn on_value_change(&mut self, font: &Font) {
        self.reflow(font);
        self.scroll_to_cursor();
    }

    /// `reflowDisplayLines`.
    fn reflow(&mut self, font: &Font) {
        self.lines = if self.chars.is_empty() {
            vec![(0, 0)]
        } else {
            let mut lines = split_ranges(font, &self.chars, self.text_width());
            if self.chars.last() == Some(&'\n') {
                lines.push((self.chars.len(), self.chars.len()));
            }
            lines
        };
    }

    /// `getLineAtCursor`: -1 when no line holds the cursor.
    fn line_at_cursor(&self) -> isize {
        self.lines
            .iter()
            .position(|&(begin, end)| self.cursor >= begin && self.cursor <= end)
            .map_or(-1, |i| i as isize)
    }
    /// `getLineView`: the index clamped to the lines.
    fn line_view(&self, index: isize) -> (usize, usize) {
        self.lines[index.clamp(0, self.lines.len() as isize - 1) as usize]
    }
    /// `getCursorLineView(offset)`.
    fn cursor_line_view(&self, offset: isize) -> (usize, usize) {
        match self.line_at_cursor() {
            -1 => *self.lines.last().expect("a text field has a line"),
            line => self.line_view(line + offset),
        }
    }

    /// `AbstractScrollArea.contentHeight` of the text area.
    fn content_height(&self) -> f32 {
        LINE * self.lines.len() as f32 + 2.0 * PADDING
    }
    /// `setScrollAmount`.
    fn set_scroll(&mut self, amount: f32) {
        self.scroll.amount = amount;
        self.scroll.clamp(0.0, self.height, self.content_height());
    }

    /// `MultiLineEditBox.scrollToCursor`, the text field's cursor listener.
    fn scroll_to_cursor(&mut self) {
        let mut amount = self.scroll.amount;
        let first = self.line_view((amount / LINE) as isize);
        let line = self.line_at_cursor() as f32;
        if self.cursor <= first.0 {
            amount = line * LINE;
        } else {
            let last = self.line_view(((amount + self.height) / LINE) as isize - 1);
            if self.cursor > last.1 {
                amount = line * LINE - self.height + LINE + 2.0 * PADDING;
            }
        }
        self.set_scroll(amount);
    }

    /// `seekCursor` to `cursor`, then the cursor listener.
    fn seek_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.chars.len());
        self.scroll_to_cursor();
        if !self.selecting {
            self.select_cursor = self.cursor;
        }
    }

    /// `seekCursorLine`.
    fn seek_cursor_line(&mut self, font: &Font, offset: isize) {
        if offset == 0 {
            return;
        }
        let (begin, _) = self.cursor_line_view(0);
        let before: String = self.chars[begin..self.cursor].iter().collect();
        let old_left = font.width(&before) + 2.0;
        let (begin, end) = self.cursor_line_view(offset);
        let line: String = self.chars[begin..end].iter().collect();
        self.seek_cursor(begin + font.fit(&line, old_left));
    }

    /// `seekCursorToPoint`: `x` and `y` from the top left of the text.
    fn seek_cursor_to_point(&mut self, font: &Font, x: f32, y: f32) {
        let left = x.floor();
        let top = (y / LINE).floor() as isize;
        let (begin, end) = self.line_view(top);
        let line: String = self.chars[begin..end].iter().collect();
        self.seek_cursor(begin + font.fit(&line, left));
    }

    /// `getPreviousWord`: where the word starts and ends.
    fn previous_word(&self) -> (usize, usize) {
        if self.chars.is_empty() {
            return (0, 0);
        }
        let mut start = self.cursor.min(self.chars.len() - 1);
        while start > 0 && java_whitespace(self.chars[start - 1]) {
            start -= 1;
        }
        while start > 0 && !java_whitespace(self.chars[start - 1]) {
            start -= 1;
        }
        (start, self.word_end(start))
    }
    /// `getNextWord`.
    fn next_word(&self) -> (usize, usize) {
        if self.chars.is_empty() {
            return (0, 0);
        }
        let mut start = self.cursor.min(self.chars.len() - 1);
        while start < self.chars.len() && !java_whitespace(self.chars[start]) {
            start += 1;
        }
        while start < self.chars.len() && java_whitespace(self.chars[start]) {
            start += 1;
        }
        (start, self.word_end(start))
    }
    fn word_end(&self, from: usize) -> usize {
        let mut end = from;
        while end < self.chars.len() && !java_whitespace(self.chars[end]) {
            end += 1;
        }
        end
    }
    /// `selectWordAtCursor`.
    fn select_word_at_cursor(&mut self) {
        let (begin, end) = self.previous_word();
        self.seek_cursor(begin);
        self.selecting = true;
        self.seek_cursor(end);
    }

    /// `MultilineTextField.keyPressed` for the keys that are not shortcuts.
    /// True when the key was used.
    pub fn key(&mut self, font: &Font, key: KeyCode, ctrl: bool, shift: bool) -> bool {
        self.selecting = shift;
        match key {
            KeyCode::Backspace if ctrl => {
                let (begin, _) = self.previous_word();
                self.delete_text(font, begin as isize - self.cursor as isize);
            }
            KeyCode::Backspace => self.delete_text(font, -1),
            KeyCode::Enter | KeyCode::NumpadEnter => self.insert_text(font, "\n"),
            KeyCode::Delete if ctrl => {
                let (begin, _) = self.next_word();
                self.delete_text(font, begin as isize - self.cursor as isize);
            }
            KeyCode::Delete => self.delete_text(font, 1),
            KeyCode::Home if ctrl => self.seek_cursor(0),
            KeyCode::Home => self.seek_cursor(self.cursor_line_view(0).0),
            KeyCode::PageUp => self.seek_cursor(0),
            KeyCode::End if ctrl => self.seek_cursor(self.chars.len()),
            KeyCode::End => self.seek_cursor(self.cursor_line_view(0).1),
            KeyCode::PageDown => self.seek_cursor(self.chars.len()),
            KeyCode::ArrowRight if ctrl => self.seek_cursor(self.next_word().0),
            KeyCode::ArrowRight => self.seek_cursor(self.cursor + 1),
            KeyCode::ArrowLeft if ctrl => self.seek_cursor(self.previous_word().0),
            KeyCode::ArrowLeft => self.seek_cursor(self.cursor.saturating_sub(1)),
            KeyCode::ArrowDown => {
                if !ctrl {
                    self.seek_cursor_line(font, 1);
                }
            }
            KeyCode::ArrowUp => {
                if !ctrl {
                    self.seek_cursor_line(font, -1);
                }
            }
            _ => return false,
        }
        true
    }

    /// Ctrl+A/C/V/X. True when the value changed.
    pub fn shortcut(
        &mut self,
        font: &Font,
        shortcut: EditShortcut,
        clipboard: &mut dyn Clipboard,
    ) -> bool {
        let before = self.chars.len();
        match shortcut {
            EditShortcut::SelectAll => {
                self.cursor = self.chars.len();
                self.select_cursor = 0;
            }
            EditShortcut::Copy => clipboard.set(&self.selected_text()),
            EditShortcut::Paste => self.insert_text(font, &clipboard.get()),
            EditShortcut::Cut => {
                clipboard.set(&self.selected_text());
                self.insert_text(font, "");
            }
        }
        self.focused_at = Instant::now();
        self.chars.len() != before
    }

    /// `AbstractWidget.isMouseOver`: the box and its scroll bar.
    pub fn hovered(&self, (x, y): (f32, f32)) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width + 6.0 && y < self.y + self.height
    }

    /// `AbstractTextAreaWidget.mouseClicked`: a press on the scroll bar
    /// holds it, else `onClick` places the cursor (`double`: selects the word).
    pub fn click(&mut self, font: &Font, cursor: (f32, f32), shift: bool, double: bool) {
        let bar = self.x + self.width;
        if self.scroll.press(
            cursor,
            bar,
            self.y,
            self.y + self.height,
            self.content_height(),
        ) {
            return;
        }
        if double {
            self.select_word_at_cursor();
        } else {
            self.selecting = shift;
            self.seek_screen(font, cursor);
        }
    }
    /// `onDrag`: the selection follows the pointer, or the held scroll bar does.
    pub fn drag(&mut self, font: &Font, cursor: (f32, f32), shift: bool) {
        if self.scroll.held() {
            self.scroll.drag(
                cursor.1,
                self.y,
                self.y + self.height,
                self.content_height(),
            );
            return;
        }
        self.selecting = true;
        self.seek_screen(font, cursor);
        self.selecting = shift;
    }
    /// `onRelease`.
    pub fn release(&mut self) {
        self.scroll.release();
    }
    /// `mouseScrolled`.
    pub fn wheel(&mut self, delta: f32) {
        let amount = self.scroll.amount - delta * SCROLL_RATE;
        self.set_scroll(amount);
    }
    /// `seekCursorScreen`.
    fn seek_screen(&mut self, font: &Font, (x, y): (f32, f32)) {
        let mouse_x = x - self.x - PADDING;
        let mouse_y = y - self.y - PADDING + self.scroll.amount;
        self.seek_cursor_to_point(font, mouse_x, mouse_y);
    }

    /// `setFocused`: the caret starts to blink.
    pub fn set_focused(&mut self, focused: bool) {
        if focused && !self.focused {
            self.focused_at = Instant::now();
        }
        self.focused = focused;
    }

    /// `AbstractTextAreaWidget.withinContentAreaTopBottom`.
    fn within(&self, top: f32, bottom: f32) -> bool {
        bottom - self.scroll.amount >= self.y && top - self.scroll.amount <= self.y + self.height
    }

    /// `extractWidgetRenderState`: the frame, the text under a scissor, the
    /// scroll bar and the character count.
    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas) {
        crate::widgets::target_as(
            [self.x, self.y, self.width, self.height],
            crate::widgets::Kind::Field,
        );
        let sprite = if self.focused {
            "gui/sprites/widget/text_field_highlighted"
        } else {
            "gui/sprites/widget/text_field"
        };
        ui::nine_slice(
            batch,
            atlas,
            sprite,
            self.x,
            self.y,
            self.width,
            self.height,
            u32::MAX,
        );
        let start = batch.vertices.len();
        self.draw_contents(batch, atlas);
        batch.clip_since(
            start,
            self.x + 1.0,
            self.y + 1.0,
            self.x + self.width - 1.0,
            self.y + self.height - 1.0,
        );
        ui::scrollbar(
            batch,
            atlas,
            self.x + self.width,
            self.y,
            self.y + self.height,
            self.content_height(),
            self.scroll.amount,
        );
        if let Some(limit) = self.character_limit {
            let count = crate::lang::tr(
                "gui.multiLineEditBox.character_limit",
                &[&self.chars.len(), &limit],
            );
            let width = atlas.font.width(&count);
            batch.text(
                atlas,
                &count,
                self.x + self.width - width,
                self.y + self.height + 4.0,
                rgba(160, 160, 160, 255),
                true,
            );
        }
    }

    /// `MultiLineEditBox.extractContents` (the placeholder is empty).
    fn draw_contents(&self, batch: &mut HudBatch, atlas: &HudAtlas) {
        if self.chars.is_empty() && !self.focused {
            return;
        }
        let font = &atlas.font;
        let text_color = rgba(224, 224, 224, 255);
        let cursor_color = rgba(208, 208, 208, 255);
        let slice =
            |begin: usize, end: usize| -> String { self.chars[begin..end].iter().collect() };
        let show_cursor =
            self.focused && (self.focused_at.elapsed().as_millis() / 300).is_multiple_of(2);
        let needs_valid_cursor_pos = self.focused;
        let insert_cursor = self.cursor < self.chars.len();
        let (inner_left, mut draw_top) = (self.x + PADDING, self.y + PADDING - self.scroll.amount);
        let (mut cursor_x, mut cursor_y) = (0.0f32, 0.0f32);
        let mut has_drawn_cursor = false;
        // The content is translated by the scroll amount; `within` compares
        // untranslated tops, so it is given the line's top before the shift.
        let shift = self.scroll.amount;
        for &(begin, end) in &self.lines {
            let within = self.within(draw_top + shift, draw_top + shift + LINE);
            if !has_drawn_cursor
                && (needs_valid_cursor_pos || show_cursor)
                && insert_cursor
                && self.cursor >= begin
                && self.cursor <= end
            {
                let before = slice(begin, self.cursor);
                let right = inner_left + font.width(&before);
                (cursor_x, cursor_y) = (right, draw_top);
                if within {
                    let after = slice(self.cursor, end);
                    batch.text(atlas, &before, inner_left, draw_top, text_color, true);
                    batch.text(atlas, &after, right, draw_top, text_color, true);
                    if show_cursor {
                        batch.fill(atlas, cursor_x, cursor_y - 1.0, 1.0, 11.0, cursor_color);
                    }
                }
                has_drawn_cursor = true;
            } else {
                let line = slice(begin, end);
                if within {
                    batch.text(atlas, &line, inner_left, draw_top, text_color, true);
                }
                if (needs_valid_cursor_pos || show_cursor) && !insert_cursor {
                    cursor_x = inner_left + font.width(&line);
                    cursor_y = draw_top;
                }
            }
            draw_top += LINE;
        }
        if show_cursor && !insert_cursor && self.within(cursor_y + shift, cursor_y + shift + LINE) {
            batch.text(atlas, "_", cursor_x, cursor_y, cursor_color, true);
        }
        if self.focused {
            crate::preedit::report_caret(cursor_x, cursor_y, 10.0);
        }
        if self.has_selection() {
            let (sel_begin, sel_end) = self.selected();
            let draw_x = inner_left;
            let mut draw_top = self.y + PADDING - self.scroll.amount;
            for &(begin, end) in &self.lines {
                if sel_begin > end {
                    draw_top += LINE;
                    continue;
                }
                if begin > sel_end {
                    break;
                }
                if self.within(draw_top + shift, draw_top + shift + LINE) {
                    let draw_begin = font.width(&slice(begin, sel_begin.max(begin)));
                    let draw_end = if sel_end > end {
                        self.width - PADDING
                    } else {
                        font.width(&slice(begin, sel_end))
                    };
                    batch.text_highlight(
                        atlas,
                        (draw_x + draw_begin, draw_top, draw_end - draw_begin, LINE),
                        true,
                    );
                }
                draw_top += LINE;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::Rect;
    use std::{collections::HashMap, path::Path};

    fn font() -> Font {
        // The missing-glyph metrics let these tests run without game assets.
        Font::load(
            Path::new("/nonexistent-horizon-multiline-test-resources"),
            &HashMap::new(),
            &[],
            (1, 1),
            (0, 0),
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 1,
                h: 1,
            },
        )
    }

    /// A box whose text is `chars` characters wide and `lines` lines tall.
    fn edit(font: &Font, chars: usize, lines: usize) -> MultiLineEdit {
        let advance = font.advance('a', &Style::default());
        MultiLineEdit::new(
            font,
            0.0,
            0.0,
            chars as f32 * advance + 8.0,
            LINE * lines as f32 + 8.0,
        )
    }

    #[test]
    fn lines_break_at_newlines_and_at_the_last_space_that_fits() {
        let font = font();
        let split = |text: &str, chars: usize| {
            let advance = font.advance('a', &Style::default());
            let text: Vec<char> = text.chars().collect();
            split_ranges(&font, &text, chars as f32 * advance)
        };
        assert_eq!(split("abcd efgh", 4), [(0, 4), (5, 9)]);
        // No space to break at: before the character that overflows.
        assert_eq!(split("abcdefgh", 4), [(0, 4), (4, 8)]);
        assert_eq!(split("ab\ncd", 4), [(0, 2), (3, 5)]);
        // A trailing newline starts no line here; the text field adds it.
        assert_eq!(split("ab\n", 4), [(0, 2)]);
        assert_eq!(split("", 4), []);
    }

    #[test]
    fn a_trailing_newline_gets_an_empty_last_line() {
        let font = font();
        let mut edit = edit(&font, 8, 4);
        assert_eq!(edit.lines(), [(0, 0)]);
        edit.insert_text(&font, "ab\n");
        assert_eq!(edit.lines(), [(0, 2), (3, 3)]);
        assert_eq!(edit.cursor(), 3);
    }

    #[test]
    fn the_character_limit_cuts_what_is_typed_and_set() {
        let font = font();
        let mut edit = edit(&font, 8, 4);
        edit.set_character_limit(5);
        edit.set_value(&font, "abcdefgh");
        assert_eq!(edit.value(), "abcde");
        edit.key(&font, KeyCode::Backspace, false, false);
        edit.insert_text(&font, "wxyz");
        assert_eq!(edit.value(), "abcdw");
    }

    #[test]
    fn the_line_limit_refuses_an_edit_that_adds_a_line() {
        let font = font();
        let mut edit = edit(&font, 8, 4);
        edit.set_line_limit(2);
        edit.set_value(&font, "a\nb");
        assert_eq!(edit.value(), "a\nb");
        // A third line, by a newline or by wrapping, changes nothing.
        edit.key(&font, KeyCode::Enter, false, false);
        edit.insert_text(&font, "\nc");
        assert_eq!(edit.value(), "a\nb");
        edit.insert_text(&font, " abcdefghijkl");
        assert_eq!(edit.value(), "a\nb");
        // A value over the limit is not set at all.
        edit.set_value(&font, "1\n2\n3");
        assert_eq!(edit.value(), "a\nb");
        // A trailing newline counts as a line.
        edit.set_value(&font, "a\n");
        edit.insert_text(&font, "\n");
        assert_eq!(edit.value(), "a\n");
    }

    #[test]
    fn up_and_down_keep_the_column_and_home_and_end_stay_on_the_line() {
        let font = font();
        let mut edit = edit(&font, 8, 4);
        edit.set_value(&font, "abcd\nef\nghij");
        // From the end of `ghij` up to `ef`: the end of the shorter line.
        edit.key(&font, KeyCode::ArrowUp, false, false);
        assert_eq!(edit.cursor(), 7);
        edit.key(&font, KeyCode::ArrowUp, false, false);
        assert_eq!(edit.cursor(), 2);
        edit.key(&font, KeyCode::Home, false, false);
        assert_eq!(edit.cursor(), 0);
        edit.key(&font, KeyCode::End, false, false);
        assert_eq!(edit.cursor(), 4);
        edit.key(&font, KeyCode::End, true, false);
        assert_eq!(edit.cursor(), 12);
        edit.key(&font, KeyCode::PageUp, false, false);
        assert_eq!(edit.cursor(), 0);
    }

    #[test]
    fn shift_selects_and_typing_replaces_the_selection() {
        let font = font();
        let mut edit = edit(&font, 8, 4);
        edit.set_value(&font, "one two");
        edit.key(&font, KeyCode::ArrowLeft, true, true);
        assert_eq!(edit.selected_text(), "two");
        edit.insert_text(&font, "2");
        assert_eq!(edit.value(), "one 2");
        edit.set_value(&font, "one two");
        edit.key(&font, KeyCode::Backspace, true, false);
        assert_eq!(edit.value(), "one ");
    }

    #[test]
    fn the_box_scrolls_to_keep_the_cursor_in_view() {
        let font = font();
        let mut edit = edit(&font, 8, 2);
        edit.set_value(&font, "1\n2\n3\n4\n5");
        // Five lines in two lines' height: the end shows.
        assert_eq!(edit.scroll.amount, 9.0 * 5.0 + 8.0 - edit.height);
        edit.key(&font, KeyCode::PageUp, false, false);
        assert_eq!(edit.scroll.amount, 0.0);
        edit.wheel(-1.0);
        assert_eq!(edit.scroll.amount, SCROLL_RATE);
    }

    #[test]
    fn a_click_puts_the_cursor_on_the_line_and_character_under_it() {
        let font = font();
        let mut edit = edit(&font, 8, 4);
        edit.set_value(&font, "abcd\nefgh");
        let advance = font.advance('a', &Style::default());
        // The second line, after two characters.
        edit.click(
            &font,
            (4.0 + 2.0 * advance + 1.0, 4.0 + 9.0 + 1.0),
            false,
            false,
        );
        assert_eq!(edit.cursor(), 7);
        // Shift extends the selection.
        edit.click(&font, (4.0 + 1.0, 4.0 + 1.0), true, false);
        assert_eq!(edit.selected_text(), "abcd\nef");
        // A double click, after the plain click that starts it, selects the word.
        edit.click(&font, (4.0 + 1.0, 4.0 + 1.0), false, false);
        edit.click(&font, (4.0 + 1.0, 4.0 + 1.0), false, true);
        assert_eq!(edit.selected_text(), "abcd");
    }
}
