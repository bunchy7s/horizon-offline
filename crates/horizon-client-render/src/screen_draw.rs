//! Menu screens as the `GuiGraphicsExtractor` calls vanilla's widgets make
//! (sprites, texture blits, fills, text highlight, text, scissor and the
//! tooltip), with ports of the widgets that make them: `StringWidget`,
//! `MultiLineTextWidget`, `Button`, `EditBox` (a font-aware port, scrolling
//! included) and the label layouts. The screen layouts built on them
//! (`pack_screen`) are compared with vanilla's calls by the parity harnesses;
//! [`paint`] draws them with the HUD batch.
use crate::{
    font::Font,
    hud::{HudAtlas, HudBatch},
    text::{Style, Text},
};

/// One GUI call.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    /// `blitSprite`, the id without its namespace.
    Sprite {
        sprite: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: i32,
    },
    /// `blit` of a texture region; `texture` is the full path (`textures/…png`).
    Blit {
        texture: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        u: f32,
        v: f32,
        src: [i32; 2],
        tex: [i32; 2],
        color: i32,
    },
    /// `fill`, corners sorted.
    Fill {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: i32,
    },
    /// `textHighlight`.
    Highlight {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    },
    Text {
        text: Text,
        x: i32,
        y: i32,
        color: i32,
        shadow: bool,
    },
    /// `enableScissor`, `disableScissor`.
    Scissor {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    },
    NoScissor,
    /// `setTooltipForNextFrame`: the split lines and the mouse.
    Tooltip {
        lines: Vec<Text>,
        x: i32,
        y: i32,
    },
}

pub const WHITE: i32 = -1;
pub const GRAY: u32 = 0xAAAAAA;
pub const DARK_GRAY: u32 = 0x555555;

pub fn fill(out: &mut Vec<Draw>, x0: i32, y0: i32, x1: i32, y1: i32, color: i32) {
    out.push(Draw::Fill {
        x0: x0.min(x1),
        y0: y0.min(y1),
        x1: x0.max(x1),
        y1: y0.max(y1),
        color,
    });
}
pub fn sprite(out: &mut Vec<Draw>, sprite: &str, x: i32, y: i32, w: i32, h: i32) {
    out.push(Draw::Sprite {
        sprite: sprite.to_owned(),
        x,
        y,
        w,
        h,
        color: WHITE,
    });
}
/// `blit(pipeline, texture, x, y, u, v, w, h, texW, texH)`.
pub fn blit(
    out: &mut Vec<Draw>,
    texture: &str,
    x: i32,
    y: i32,
    uv: (f32, f32),
    w: i32,
    h: i32,
    tex: [i32; 2],
) {
    out.push(Draw::Blit {
        texture: texture.to_owned(),
        x,
        y,
        w,
        h,
        u: uv.0,
        v: uv.1,
        src: [w, h],
        tex,
        color: WHITE,
    });
}
/// `graphics.text`: dropped when fully transparent; legacy codes resolved.
pub fn text(out: &mut Vec<Draw>, text: Text, x: i32, y: i32, color: i32, shadow: bool) {
    if (color as u32) >> 24 != 0 {
        out.push(Draw::Text {
            text: text.resolve_legacy(),
            x,
            y,
            color,
            shadow,
        });
    }
}

/// `StringDecomposer.iterateFormatted`: the drawn characters of `contents`,
/// with their index in it and their style; legacy `§` codes restyle what
/// follows them, `§r` returns to `reset`.
pub fn formatted(contents: &str, current: &Style, reset: &Style) -> Vec<(usize, char, Style)> {
    let chars: Vec<char> = contents.chars().collect();
    let mut style = current.clone();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '§' {
            let Some(code) = chars.get(i + 1) else { break };
            let code = code.to_ascii_lowercase();
            if let Some(c) = crate::text::COLORS.iter().find(|c| c.0 == code) {
                style = Style {
                    color: Some(c.2),
                    bold: Some(false),
                    italic: Some(false),
                    underlined: Some(false),
                    strikethrough: Some(false),
                    obfuscated: Some(false),
                    ..style
                };
            } else {
                match code {
                    'k' => style.obfuscated = Some(true),
                    'l' => style.bold = Some(true),
                    'm' => style.strikethrough = Some(true),
                    'n' => style.underlined = Some(true),
                    'o' => style.italic = Some(true),
                    'r' => style = reset.clone(),
                    _ => {}
                }
            }
            i += 2;
            continue;
        }
        out.push((i, c, style.clone()));
        i += 1;
    }
    out
}

/// Text measurement as vanilla's `Font` and `StringSplitter` do it, on
/// text whose spans may hold legacy `§` codes (each span is a component's
/// contents and style).
pub struct Measure<'a>(pub &'a Font);
impl Measure<'_> {
    /// `Font.width`.
    pub fn width(&self, t: &Text) -> i32 {
        self.0.width_text(&t.resolve_legacy()) as i32
    }
    pub fn width_str(&self, s: &str) -> i32 {
        self.0.width(s) as i32
    }
    /// `StringSplitter.headByWidth`: the leading characters whose widths sum to at most `max`.
    pub fn head(&self, t: &Text, max: i32) -> Text {
        let mut left = max as f32;
        let mut out = Text::empty();
        for span in &t.spans {
            let mut end = None;
            let mut position = 0;
            for (i, c, style) in formatted(&span.text, &span.style, &span.style) {
                left -= self.0.advance(c, &style);
                if left >= 0.0 {
                    position = i + 1;
                } else {
                    end = Some(position);
                    break;
                }
            }
            match end {
                Some(p) => {
                    out.push(Text::styled(
                        span.text.chars().take(p).collect::<String>(),
                        span.style.clone(),
                    ));
                    return out;
                }
                None => out.push(Text::styled(span.text.clone(), span.style.clone())),
            }
        }
        t.clone()
    }
    /// `Font.plainSubstrByWidth`, from the start or (`tail`) the end, in characters.
    pub fn plain(&self, s: &str, max: i32, tail: bool) -> usize {
        if tail {
            self.0.fit_reverse(s, max as f32)
        } else {
            self.0.fit(s, max as f32)
        }
    }
    /// `StringSplitter.splitLines` (`Font.split`): breaks at a newline, or
    /// at the last space (else the character) once the width is exceeded;
    /// the rest of a broken part takes the style found at the break, legacy
    /// codes counted from the root style.
    pub fn split(&self, t: &Text, max: i32) -> Vec<Text> {
        let max = (max as f32).max(1.0);
        let mut parts: Vec<(Vec<char>, Style)> = t
            .spans
            .iter()
            .filter(|s| !s.text.is_empty())
            .map(|s| (s.text.chars().collect(), s.style.clone()))
            .collect();
        let mut lines = Vec::new();
        let mut force_new_line = false;
        let root = Style::default();
        'restart: loop {
            let (mut width, mut had_visible) = (0.0f32, false);
            let (mut last_space, mut last_space_style) = (None::<usize>, Style::default());
            let mut offset = 0;
            for (pi, (contents, style)) in parts.iter().enumerate() {
                let s: String = contents.iter().collect();
                for (i, c, st) in formatted(&s, style, &root) {
                    let at = i + offset;
                    let brk = if c == '\n' {
                        Some((at, st.clone()))
                    } else {
                        if c == ' ' {
                            last_space = Some(at);
                            last_space_style = st.clone();
                        }
                        let w = self.0.advance(c, &st);
                        width += w;
                        if !had_visible || width <= max {
                            had_visible |= w != 0.0;
                            None
                        } else {
                            Some(match last_space {
                                Some(p) => (p, last_space_style.clone()),
                                None => (at, st.clone()),
                            })
                        }
                    };
                    if let Some((at, split_style)) = brk {
                        let flat: Vec<char> =
                            parts.iter().flat_map(|p| p.0.iter().copied()).collect();
                        let first = flat.get(at).copied().unwrap_or(' ');
                        let new_line = first == '\n';
                        let skip = usize::from(new_line || first == ' ');
                        force_new_line = new_line;
                        // FlatComponents.splitAt.
                        let mut line = Text::empty();
                        let mut position = at;
                        let mut rest = Vec::new();
                        let mut in_skip = false;
                        let mut done = false;
                        for (contents, style) in parts.drain(..) {
                            if done {
                                rest.push((contents, style));
                                continue;
                            }
                            let size = contents.len();
                            if !in_skip {
                                if position > size {
                                    line.push(Text::styled(
                                        contents.iter().collect::<String>(),
                                        style,
                                    ));
                                    position -= size;
                                    continue;
                                }
                                let before: String = contents[..position].iter().collect();
                                if !before.is_empty() {
                                    line.push(Text::styled(before, style.clone()));
                                }
                                position += skip;
                                in_skip = true;
                            }
                            if position <= size {
                                if position < size {
                                    rest.push((contents[position..].to_vec(), split_style.clone()));
                                }
                                done = true;
                                continue;
                            }
                            position -= size;
                        }
                        let _ = pi;
                        parts = rest;
                        lines.push(line);
                        continue 'restart;
                    }
                }
                offset += contents.len();
            }
            break;
        }
        if !parts.is_empty() {
            let mut line = Text::empty();
            for (contents, style) in parts {
                line.push(Text::styled(contents.iter().collect::<String>(), style));
            }
            lines.push(line);
        } else if force_new_line {
            lines.push(Text::empty());
        }
        lines
    }
}

/// `TextAlignment.CENTER.calculateLeft`.
pub fn center_left(anchor: i32, width: i32) -> i32 {
    anchor - width / 2
}

/// `ComponentRenderUtils.clipText`: as much as fits before an unstyled "...".
pub fn clip(m: &Measure, t: &Text, width: i32) -> Text {
    m.head(t, width - m.width_str("..."))
        .append(Text::literal("..."))
}

/// `StringWidget` (CLAMPED) at its position: its single line, clipped when wider than `max_width`.
pub fn string_widget(
    out: &mut Vec<Draw>,
    m: &Measure,
    message: &Text,
    x: i32,
    y: i32,
    max_width: i32,
) {
    let w = m.width(message);
    let max = if max_width > 0 { max_width } else { w };
    let line = if w > max {
        clip(m, message, max)
    } else {
        message.clone()
    };
    text(out, line, x, y, WHITE, true);
}

/// `MultiLineLabel.create(font, maxWidth, maxRows, message)`: the lines and their widths;
/// the last kept line gets "..." in `ellipsis` (the message's own style) when rows were cut.
pub fn label_lines(
    m: &Measure,
    message: &Text,
    max_width: i32,
    max_rows: usize,
    ellipsis: &Style,
) -> Vec<(Text, i32)> {
    let split = m.split(message, max_width);
    let n = split.len().min(max_rows);
    let mut out = Vec::new();
    for (i, line) in split.iter().take(n).enumerate() {
        if i == n - 1 && n == max_rows && n != split.len() {
            let clipped = m.head(line, m.width(line) - m.width_str("..."));
            let with = clipped.append(Text::styled("...", ellipsis.clone()));
            let w = m.width(&with);
            out.push((with, w));
        } else {
            out.push((line.clone(), m.width(line)));
        }
    }
    out
}
/// `MultiLineTextWidget.getWidth`.
pub fn label_width(lines: &[(Text, i32)], max_width: i32) -> i32 {
    lines.iter().map(|l| l.1).max().unwrap_or(0).min(max_width)
}
/// `MultiLineTextWidget.visitLines`.
pub fn label(
    out: &mut Vec<Draw>,
    lines: &[(Text, i32)],
    x: i32,
    y: i32,
    max_width: i32,
    centered: bool,
) {
    let mid = x + label_width(lines, max_width) / 2;
    for (i, (line, w)) in lines.iter().enumerate() {
        let left = if centered { center_left(mid, *w) } else { x };
        text(out, line.clone(), left, y + 9 * i as i32, WHITE, true);
    }
}

/// `Button` (`extractDefaultSprite` and the label over its contents, which
/// must fit: scrolling labels depend on the clock).
pub fn button(
    out: &mut Vec<Draw>,
    m: &Measure,
    label: &Text,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    active: bool,
    highlighted: bool,
) {
    // Its narration: hovered is the mouse over it (`isHovered`), which the
    // narrator tests.
    crate::widgets::narratable(|| {
        crate::widgets::Narratable::button(
            label.plain(),
            [x as f32, y as f32, w as f32, h as f32],
            active,
        )
    });
    let sprite_id = if !active {
        "widget/button_disabled"
    } else if highlighted {
        "widget/button_highlighted"
    } else {
        "widget/button"
    };
    sprite(out, sprite_id, x, y, w, h);
    let (left, right) = (x + 2, x + w - 2);
    let lw = m.width(label);
    let top = (y + y + h - 9) / 2 + 1;
    let cx = ((left + right) / 2).clamp(left + lw / 2, right - lw / 2);
    text(out, label.clone(), center_left(cx, lw), top, WHITE, true);
}
/// `Tooltip.splitTooltip` of a widget's tooltip, shown under the mouse.
pub fn tooltip(out: &mut Vec<Draw>, m: &Measure, message: &Text, mouse: (i32, i32)) {
    out.push(Draw::Tooltip {
        lines: m.split(message, 170),
        x: mouse.0,
        y: mouse.1,
    });
}
/// `AbstractWidget.areCoordinatesInRectangle`.
pub fn inside(mouse: (i32, i32), x: i32, y: i32, w: i32, h: i32) -> bool {
    mouse.0 >= x && mouse.1 >= y && mouse.0 < x + w && mouse.1 < y + h
}

/// Vanilla's `EditBox`: value, cursor, selection and the first shown
/// character, which follows the cursor as `scrollTo` moves it. Positions
/// are in characters.
#[derive(Clone, Debug)]
pub struct TextField {
    pub value: String,
    pub cursor: usize,
    pub highlight: usize,
    pub display: usize,
    pub max_length: usize,
    pub bordered: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub focused: bool,
    pub text_color: i32,
    pub text_shadow: bool,
    pub hint: Option<Text>,
    pub suggestion: Option<String>,
    /// `getMessage`: what the narrator calls the box.
    pub message: String,
}
fn chars(s: &str) -> usize {
    s.chars().count()
}
fn sub(s: &str, from: usize, to: usize) -> String {
    s.chars().skip(from).take(to.saturating_sub(from)).collect()
}
/// A text field with the font that scrolls it.
struct MeasuredField<'a, 'm>(&'a mut TextField, &'a Measure<'m>);
impl crate::clipboard::Editable for MeasuredField<'_, '_> {
    fn select_all(&mut self) {
        let len = chars(&self.0.value);
        self.0.move_cursor_to(self.1, len, false);
        self.0.set_highlight(self.1, 0);
    }
    fn selected_text(&self) -> String {
        self.0.selected_text()
    }
    fn insert(&mut self, text: &str) -> bool {
        let before = self.0.value.clone();
        self.0.insert_text(self.1, text);
        self.0.value != before
    }
}
impl TextField {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            highlight: 0,
            display: 0,
            max_length: 32,
            bordered: true,
            x,
            y,
            width,
            height,
            focused: false,
            text_color: -2039584,
            text_shadow: true,
            hint: None,
            suggestion: None,
            message: String::new(),
        }
    }
    pub fn inner_width(&self) -> i32 {
        if self.bordered {
            self.width - 8
        } else {
            self.width
        }
    }
    fn text_x(&self) -> i32 {
        self.x + if self.bordered { 4 } else { 0 }
    }
    fn text_y(&self) -> i32 {
        if self.bordered {
            self.y + (self.height - 8) / 2
        } else {
            self.y
        }
    }
    /// `setValue`.
    pub fn set_value(&mut self, m: &Measure, value: &str) {
        self.value = value.chars().take(self.max_length).collect();
        self.move_cursor_to(m, chars(&self.value), false);
        self.set_highlight(m, self.cursor);
    }
    /// `insertText`: replaces the selection within the maximum length. True
    /// when the value changed (its responder runs).
    pub fn insert_text(&mut self, m: &Measure, input: &str) -> bool {
        let (start, end) = (
            self.cursor.min(self.highlight),
            self.cursor.max(self.highlight),
        );
        let room = self.max_length as isize - chars(&self.value) as isize + (end - start) as isize;
        if room <= 0 {
            return false;
        }
        let filtered: String = input
            .chars()
            .filter(|c| *c >= ' ' && *c != '\u{7f}' && *c != '§')
            .collect();
        let insert: String = filtered.chars().take(room as usize).collect();
        let n = chars(&insert);
        self.value = format!(
            "{}{}{}",
            sub(&self.value, 0, start),
            insert,
            sub(&self.value, end, chars(&self.value))
        );
        self.set_cursor(m, start + n);
        self.set_highlight(m, self.cursor);
        true
    }
    /// `deleteChars(dir)`: the selection, or `dir` characters from the cursor.
    pub fn delete_chars(&mut self, m: &Measure, dir: isize) -> bool {
        if self.value.is_empty() {
            return false;
        }
        if self.highlight != self.cursor {
            return self.insert_text(m, "");
        }
        let pos = (self.cursor as isize + dir).clamp(0, chars(&self.value) as isize) as usize;
        let (start, end) = (pos.min(self.cursor), pos.max(self.cursor));
        if start == end {
            return false;
        }
        self.value = format!(
            "{}{}",
            sub(&self.value, 0, start),
            sub(&self.value, end, chars(&self.value))
        );
        self.set_cursor(m, start);
        self.move_cursor_to(m, start, false);
        true
    }
    pub fn move_cursor_to(&mut self, m: &Measure, pos: usize, extend: bool) {
        self.set_cursor(m, pos);
        if !extend {
            self.set_highlight(m, self.cursor);
        }
    }
    fn set_cursor(&mut self, m: &Measure, pos: usize) {
        self.cursor = pos.min(chars(&self.value));
        self.scroll_to(m, self.cursor);
    }
    fn set_highlight(&mut self, m: &Measure, pos: usize) {
        self.highlight = pos.min(chars(&self.value));
        self.scroll_to(m, self.highlight);
    }
    fn scroll_to(&mut self, m: &Measure, pos: usize) {
        let len = chars(&self.value);
        self.display = self.display.min(len);
        let inner = self.inner_width();
        let rest = sub(&self.value, self.display, len);
        let last = m.plain(&rest, inner, false) + self.display;
        let mut display = self.display as isize;
        if pos == self.display {
            display -= m.plain(&self.value, inner, true) as isize;
        }
        if pos > last {
            display += (pos - last) as isize;
        } else if pos as isize <= display {
            display = pos as isize;
        }
        self.display = display.clamp(0, len as isize) as usize;
    }
    /// `getWordPosition(dir, from)`: past the next space run, or back to the
    /// start of the word.
    pub fn word_position(&self, dir: isize, from: usize) -> usize {
        crate::ui::word_position(&self.value, dir, from)
    }
    /// `keyPressed` of a focused, editable box: `Some(changed)` when handled.
    pub fn key(
        &mut self,
        m: &Measure,
        key: winit::keyboard::KeyCode,
        shift: bool,
        ctrl: bool,
    ) -> Option<bool> {
        use winit::keyboard::KeyCode as K;
        let len = chars(&self.value);
        match key {
            K::Backspace | K::Delete => {
                let dir = if key == K::Backspace { -1 } else { 1 };
                if !ctrl {
                    return Some(self.delete_chars(m, dir));
                }
                // deleteWords.
                if self.value.is_empty() {
                    return Some(false);
                }
                if self.highlight != self.cursor {
                    return Some(self.insert_text(m, ""));
                }
                let to = self.word_position(dir, self.cursor);
                return Some(self.delete_chars(m, to as isize - self.cursor as isize));
            }
            K::Home => self.move_cursor_to(m, 0, shift),
            K::End => self.move_cursor_to(m, len, shift),
            K::ArrowRight | K::ArrowLeft => {
                let dir = if key == K::ArrowRight { 1 } else { -1 };
                let to = if ctrl {
                    self.word_position(dir, self.cursor)
                } else {
                    (self.cursor as isize + dir).clamp(0, len as isize) as usize
                };
                self.move_cursor_to(m, to, shift);
            }
            K::KeyA if ctrl && !shift => {
                self.move_cursor_to(m, len, false);
                self.set_highlight(m, 0);
            }
            _ => return None,
        }
        Some(false)
    }
    /// Ctrl+A/C/V/X (`EditBox.keyPressed`). True when the value changed.
    pub fn shortcut(
        &mut self,
        m: &Measure,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        crate::clipboard::apply(&mut MeasuredField(self, m), shortcut, clipboard)
    }
    /// `getHighlighted`.
    pub fn selected_text(&self) -> String {
        sub(
            &self.value,
            self.cursor.min(self.highlight),
            self.cursor.max(self.highlight),
        )
    }
    /// `getScreenX`: where a character index is drawn, from the box's left.
    pub fn screen_x(&self, m: &Measure, index: usize) -> i32 {
        if index > chars(&self.value) {
            return self.x;
        }
        self.x + m.width_str(&sub(&self.value, 0, index))
    }
    /// `setCursorPosition` then `setHighlightPos`.
    pub fn set_cursor_and_highlight(&mut self, m: &Measure, pos: usize) {
        self.set_cursor(m, pos);
        self.set_highlight(m, pos);
    }
    /// `onClick`: the cursor where the mouse is (`findClickedPositionInText`).
    pub fn click(&mut self, m: &Measure, x: f64, extend: bool) {
        let pos = (x.floor() as i32 - self.text_x()).min(self.inner_width());
        let rest = sub(&self.value, self.display, chars(&self.value));
        let at = self.display + m.plain(&rest, pos, false);
        self.move_cursor_to(m, at, extend);
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64
            && y >= self.y as f64
            && x < (self.x + self.width) as f64
            && y < (self.y + self.height) as f64
    }
    /// `extractWidgetRenderState`; `blink_ms` is the time since the field
    /// was focused, `format` styles a substring starting at a character index.
    pub fn draw(
        &self,
        out: &mut Vec<Draw>,
        m: &Measure,
        blink_ms: u64,
        format: Option<&dyn Fn(&str, usize) -> Text>,
    ) {
        let rect = [
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
        ];
        crate::widgets::target_as(rect, crate::widgets::Kind::Field);
        crate::widgets::narratable(|| {
            crate::widgets::Narratable::edit_box(self.message.clone(), self.value.clone(), rect)
                .focused(self.focused)
        });
        if self.bordered {
            let s = if self.focused {
                "widget/text_field_highlighted"
            } else {
                "widget/text_field"
            };
            sprite(out, s, self.x, self.y, self.width, self.height);
        }
        let color = self.text_color;
        let len = chars(&self.value);
        let rest = sub(&self.value, self.display, len);
        let shown_len = m.plain(&rest, self.inner_width(), false);
        let displayed = sub(&rest, 0, shown_len);
        let rel_cursor = self.cursor as isize - self.display as isize;
        let on_screen = rel_cursor >= 0 && rel_cursor <= shown_len as isize;
        let show_cursor = self.focused && (blink_ms / 300).is_multiple_of(2) && on_screen;
        let (tx, ty) = (self.text_x(), self.text_y());
        let mut draw_x = tx;
        let rel_highlight =
            (self.highlight as isize - self.display as isize).clamp(0, shown_len as isize) as usize;
        let styled = |s: String, offset: usize| match format {
            Some(f) => f(&s, offset),
            None => Text::literal(s),
        };
        if !displayed.is_empty() {
            let half = if on_screen {
                sub(&displayed, 0, rel_cursor as usize)
            } else {
                displayed.clone()
            };
            let t = styled(half, self.display);
            let w = m.width(&t);
            text(out, t, draw_x, ty, color, self.text_shadow);
            draw_x += w + 1;
        }
        let insert = self.cursor < len || len >= self.max_length;
        let mut cursor_x = draw_x;
        if !on_screen {
            cursor_x = if rel_cursor > 0 { tx + self.width } else { tx };
        } else if insert {
            cursor_x -= 1;
            draw_x -= 1;
        }
        if !displayed.is_empty() && on_screen && (rel_cursor as usize) < shown_len {
            let t = styled(sub(&displayed, rel_cursor as usize, shown_len), self.cursor);
            text(out, t, draw_x, ty, color, self.text_shadow);
        }
        if let Some(hint) = &self.hint
            && displayed.is_empty()
            && !self.focused
        {
            text(out, hint.clone(), draw_x, ty, color, true);
        }
        if !insert && let Some(s) = &self.suggestion {
            text(
                out,
                Text::literal(s.clone()),
                cursor_x - 1,
                ty,
                -8355712,
                self.text_shadow,
            );
        }
        if rel_highlight as isize != rel_cursor {
            let hx = tx + m.width_str(&sub(&displayed, 0, rel_highlight));
            let right = self.x + self.width;
            out.push(Draw::Highlight {
                x0: cursor_x.min(right),
                y0: ty - 1,
                x1: (hx - 1).min(right),
                y1: ty + 1 + 9,
            });
        }
        if show_cursor {
            if insert {
                fill(out, cursor_x, ty - 1, cursor_x + 1, ty + 10, color);
            } else {
                text(
                    out,
                    Text::literal("_"),
                    cursor_x,
                    ty,
                    color,
                    self.text_shadow,
                );
            }
        }
    }
}

/// Draws the calls with the HUD batch: sprites nine-sliced, textures from
/// the atlas (`textures/gui/x.png` is `gui/x`; tiled textures from their
/// corner), scissor clipping, and the tooltip at the mouse.
pub fn paint(batch: &mut HudBatch, atlas: &HudAtlas, draws: &[Draw]) {
    let mut clip: Option<(usize, [i32; 4])> = None;
    let mut tooltip = None;
    for d in draws {
        match d {
            Draw::Sprite {
                sprite,
                x,
                y,
                w,
                h,
                color,
            } => crate::ui::nine_slice(
                batch,
                atlas,
                &format!("gui/sprites/{sprite}"),
                *x as f32,
                *y as f32,
                *w as f32,
                *h as f32,
                crate::hud_layout::abgr(*color as u32),
            ),
            Draw::Blit {
                texture,
                x,
                y,
                w,
                h,
                u,
                v,
                src,
                tex,
                color,
            } => {
                let key = texture
                    .trim_start_matches("textures/")
                    .trim_end_matches(".png");
                let key = if atlas.sprites.contains_key(key) {
                    key
                } else {
                    "misc/unknown_pack"
                };
                let color = crate::hud_layout::abgr(*color as u32);
                if src[0] == tex[0] && src[1] == tex[1]
                    || (*u == 0.0 && *v == 0.0 && src[0] <= tex[0] && src[1] <= tex[1])
                {
                    if let Some(mut r) = atlas.sprites.get(key).copied() {
                        let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                        r.u1 = r.u0 + du * src[0] as f32 / tex[0] as f32;
                        r.v1 = r.v0 + dv * src[1] as f32 / tex[1] as f32;
                        batch.rect_uv(*x as f32, *y as f32, *w as f32, *h as f32, r, color);
                    }
                } else {
                    crate::ui::tiled(
                        batch,
                        atlas,
                        key,
                        *x as f32,
                        *y as f32,
                        *w as f32,
                        *h as f32,
                        tex[0] as f32,
                        color,
                    );
                }
            }
            Draw::Fill {
                x0,
                y0,
                x1,
                y1,
                color,
            } => batch.fill(
                atlas,
                *x0 as f32,
                *y0 as f32,
                (x1 - x0) as f32,
                (y1 - y0) as f32,
                crate::hud_layout::abgr(*color as u32),
            ),
            Draw::Highlight { x0, y0, x1, y1 } => {
                let (a, b) = ((*x0).min(*x1), (*x0).max(*x1));
                batch.text_highlight(
                    atlas,
                    (a as f32, *y0 as f32, (b - a) as f32, (y1 - y0) as f32),
                    true,
                );
            }
            Draw::Text {
                text,
                x,
                y,
                color,
                shadow,
            } => {
                batch.text_styled(
                    atlas,
                    text,
                    *x as f32,
                    *y as f32,
                    crate::hud_layout::abgr(*color as u32),
                    *shadow,
                );
            }
            Draw::Scissor { x0, y0, x1, y1 } => {
                clip = Some((batch.vertices.len(), [*x0, *y0, *x1, *y1]))
            }
            Draw::NoScissor => {
                if let Some((start, [x0, y0, x1, y1])) = clip.take() {
                    batch.clip_since(start, x0 as f32, y0 as f32, x1 as f32, y1 as f32);
                }
            }
            Draw::Tooltip { lines, x, y } => {
                tooltip = Some((lines.clone(), (*x as f32, *y as f32)))
            }
        }
    }
    if let Some((lines, at)) = tooltip {
        crate::ui::tooltip(batch, atlas, &lines, at);
    }
}
