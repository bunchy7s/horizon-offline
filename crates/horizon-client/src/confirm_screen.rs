//! `ConfirmScreen`: a title, a message wrapped to `width - 50` (15 lines at
//! most) and two 150-wide buttons 4 apart and 16 below it, the whole column
//! centred; Escape answers no.
use crate::{
    font::Font,
    hud::{HudAtlas, HudBatch},
    text::Text,
    ui,
};

pub struct Confirm {
    /// Each line and where it is drawn.
    pub lines: Vec<(Text, f32, f32)>,
    /// The yes and no buttons' corners.
    pub buttons: [(f32, f32); 2],
}
impl Confirm {
    pub fn new(font: &Font, size: (f32, f32), title: &Text, message: &Text) -> Self {
        let Choice { lines, buttons, .. } = Choice::new(font, size, title, message, 2, 150.0);
        Self {
            lines,
            buttons: [buttons[0], buttons[1]],
        }
    }
    /// The answer a click at `cursor` gives: true for yes, false for no.
    pub fn answer_at(&self, cursor: (f32, f32)) -> Option<bool> {
        let [yes, no] = self.buttons;
        let on = |(x, y): (f32, f32)| ui::hovered(cursor, x, y, 150.0, 20.0);
        if on(yes) {
            Some(true)
        } else if on(no) {
            Some(false)
        } else {
            None
        }
    }
    /// The lines and the buttons (yes first), over whatever is behind.
    pub fn draw(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        labels: [&Text; 2],
    ) {
        self.draw_active(batch, atlas, cursor, labels, true);
    }
    /// `draw` with the buttons greyed out while `setDelay` runs (`active` false).
    pub fn draw_active(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        labels: [&Text; 2],
        active: bool,
    ) {
        for (line, x, y) in &self.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        for ((bx, by), label) in self.buttons.into_iter().zip(labels) {
            let hover = active && ui::hovered(cursor, bx, by, 150.0, 20.0);
            ui::button(batch, atlas, bx, by, 150.0, 20.0, label, active, hover);
        }
    }
}

/// `ConfirmScreen` with a row of `count` buttons `width` wide, 4 apart: `ConfirmLinkScreen`'s
/// yes, copy and no are three of 100. `Confirm` is its two of 150.
pub struct Choice {
    /// Each line and where it is drawn.
    pub lines: Vec<(Text, f32, f32)>,
    /// The buttons' corners, left to right.
    pub buttons: Vec<(f32, f32)>,
    pub width: f32,
}
impl Choice {
    pub fn new(
        font: &Font,
        (w, h): (f32, f32),
        title: &Text,
        message: &Text,
        count: usize,
        width: f32,
    ) -> Self {
        let message: Vec<Text> = font
            .split_lines(message, w - 50.0)
            .into_iter()
            .map(|(line, _)| line)
            .take(15)
            .collect();
        let tw = font.width_text(title);
        let mw = message
            .iter()
            .map(|l| font.width_text(l))
            .fold(0.0, f32::max);
        let mh = 9.0 * message.len() as f32;
        let row = width * count as f32 + 4.0 * count.saturating_sub(1) as f32;
        let col = tw.max(mw).max(row);
        let total = 9.0 + 8.0 + mh + 8.0 + 36.0;
        let (lx, ly) = (((w - col) / 2.0).trunc(), ((h - total) / 2.0).trunc());
        let cell = |cw: f32| lx + ((col - cw) / 2.0).trunc();
        let mid = cell(mw) + (mw / 2.0).trunc();
        let mut lines = vec![(title.clone(), cell(tw), ly)];
        for (i, line) in message.into_iter().enumerate() {
            let x = mid - (font.width_text(&line) / 2.0).trunc();
            lines.push((line, x, ly + 17.0 + 9.0 * i as f32));
        }
        let (bx, by) = (cell(row), ly + 9.0 + 8.0 + mh + 8.0 + 16.0);
        Self {
            lines,
            buttons: (0..count)
                .map(|i| (bx + (width + 4.0) * i as f32, by))
                .collect(),
            width,
        }
    }
    /// The button a click at `cursor` hits, counted from the left.
    pub fn pressed_at(&self, cursor: (f32, f32)) -> Option<usize> {
        self.buttons
            .iter()
            .position(|&(x, y)| ui::hovered(cursor, x, y, self.width, 20.0))
    }
    /// The lines and the buttons, over whatever is behind.
    pub fn draw(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        labels: &[Text],
    ) {
        for (line, x, y) in &self.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        for (&(bx, by), label) in self.buttons.iter().zip(labels) {
            let hover = ui::hovered(cursor, bx, by, self.width, 20.0);
            ui::button(batch, atlas, bx, by, self.width, 20.0, label, true, hover);
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
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        };
        Font::load(
            Path::new("/nonexistent-horizon-confirm-test-resources"),
            &HashMap::new(),
            &[],
            (1, 1),
            (0, 0),
            rect,
        )
    }

    #[test]
    fn three_buttons_sit_four_apart_in_the_middle() {
        let (title, message) = (Text::literal("Open?"), Text::literal("https://x"));
        let c = Choice::new(&font(), (480.0, 270.0), &title, &message, 3, 100.0);
        let xs: Vec<f32> = c.buttons.iter().map(|b| b.0).collect();
        // 3 x 100 + 2 x 4 = 308 wide, centred on 480.
        assert_eq!(xs, vec![86.0, 190.0, 294.0]);
        assert_eq!(c.pressed_at((191.0, c.buttons[1].1 + 1.0)), Some(1));
        assert_eq!(c.pressed_at((188.0, c.buttons[1].1 + 1.0)), None);
        // Confirm keeps its two 150-wide buttons where they were.
        let two = Confirm::new(&font(), (480.0, 270.0), &title, &message);
        assert_eq!(two.buttons[0].0, 88.0);
        assert_eq!(two.buttons[1].0, 242.0);
    }
}
