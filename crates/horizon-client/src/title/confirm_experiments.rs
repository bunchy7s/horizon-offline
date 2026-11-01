//! CreateWorldScreen's `ConfirmExperimentalFeaturesScreen`: the warning that
//! the picked data packs ask for experimental features, with the packs' own
//! flags on a second screen (`DetailsScreen`).
//!
//! Vanilla centres a grid (columns 10 apart, rows 20 apart): the title, the
//! message wrapped to 310, a 100-wide Details button, then Proceed and Back.

use crate::{
    font::Font,
    hud::{HudAtlas, HudBatch},
    text::{Style, Text, named_color},
    ui,
};

/// `ConfirmExperimentalFeaturesScreen.COLUMN_SPACING`.
const COLUMN_SPACING: f32 = 10.0;
/// The footer buttons' width (`Button.defaults`).
const BUTTON: f32 = 150.0;
/// `DETAILS_BUTTON_WIDTH`.
const DETAILS: f32 = 100.0;
/// `MultiLineTextWidget.setMaxWidth(310)`.
const WARNING: f32 = 310.0;
/// One button row plus the grid's 20-pixel row spacing, twice.
const ROW: f32 = 40.0;

/// A press on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    /// The Details button: the packs' flags.
    Details,
    /// `gui.proceed`: apply the pending selection.
    Proceed,
    /// `gui.back` or Escape: leave the selection as it was.
    Back,
}

/// One enabled feature pack, as `DetailsScreen`'s list shows it.
pub struct Entry {
    /// The pack's name, bold.
    pub title: Text,
    /// `selectWorld.experimental.details.entry`'s argument: the flags.
    pub flags: String,
}

/// The screen; `details` shows `DetailsScreen` instead.
pub struct ConfirmExperimentsScreen {
    pub entries: Vec<Entry>,
    /// The selection to apply on Proceed.
    pub pending: Vec<String>,
    pub details: bool,
    in_world: bool,
}
impl ConfirmExperimentsScreen {
    /// `ConfirmExperimentalFeaturesScreen::new`: the picked selection's
    /// feature packs, each with the flags it asks for.
    pub fn new(resources: &std::path::Path, selected: &[String]) -> Self {
        let entries = crate::datapack::feature_packs(resources)
            .into_iter()
            .filter(|p| selected.contains(&p.id))
            .filter(|p| !p.features().is_empty())
            .map(|p| {
                let name = format!("dataPack.{}.name", p.id);
                let title = (if crate::lang::has(&name) {
                    Text::translate(&name, &[])
                } else {
                    Text::literal(p.id.clone())
                })
                .with_style(&Style {
                    bold: Some(true),
                    ..Style::default()
                });
                let flags = p
                    .features()
                    .iter()
                    .map(|f| f.trim_start_matches("minecraft:").to_owned())
                    .collect::<Vec<_>>()
                    .join(", ");
                Entry { title, flags }
            })
            .collect();
        Self {
            entries,
            pending: selected.to_vec(),
            details: false,
            in_world: false,
        }
    }
    /// The message's lines (`MultiLineTextWidget` at 310; the text's own
    /// line breaks kept).
    fn message_lines(font: &Font) -> Vec<Text> {
        crate::lang::get_or(
            "selectWorld.experimental.message",
            "Be careful!\nThis configuration requires features that are still under development. \
             Your world might crash, break, or not work with future updates.",
        )
        .split('\n')
        .flat_map(|line| {
            font.split_lines(&Text::literal(line.to_owned()), WARNING)
                .into_iter()
                .map(|(l, _)| l)
        })
        .collect()
    }
    /// The grid's top: the whole block centred vertically
    /// (`FrameLayout.alignInRectangle`).
    fn block_top(message: usize, h: f32) -> f32 {
        let rows = 9.0 + message as f32 * 9.0 + ROW + ROW;
        ((h - rows) / 2.0).max(0.0)
    }
    /// The Details button's rect (centred, under the message).
    fn details_rect(message: usize, w: f32, h: f32) -> (f32, f32) {
        (
            ((w - DETAILS) / 2.0).trunc(),
            Self::block_top(message, h) + 9.0 + ROW,
        )
    }
    /// A press: Details, Proceed (`gui.proceed`) or Back (`gui.back`).
    pub fn click(&self, atlas: &HudAtlas, (x, y): (f32, f32), size: (f32, f32)) -> Option<Click> {
        let (w, h) = size;
        if self.details {
            let [(bx, by)] = footer_single(w, h);
            return ui::hovered((x, y), bx, by, BUTTON, 20.0).then_some(Click::Back);
        }
        let message = Self::message_lines(&atlas.font).len();
        let (dx, dy) = Self::details_rect(message, w, h);
        if ui::hovered((x, y), dx, dy, DETAILS, 20.0) {
            return Some(Click::Details);
        }
        let [(px, py), (bx, by)] = footer_pair(w, h);
        if ui::hovered((x, y), px, py, BUTTON, 20.0) {
            return Some(Click::Proceed);
        }
        ui::hovered((x, y), bx, by, BUTTON, 20.0).then_some(Click::Back)
    }
    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
        let font = &atlas.font;
        if self.details {
            // `DetailsScreen`: the header's title, the packs, Back in the
            // footer.
            let title = Text::translate("selectWorld.experimental.details.title", &[]);
            let tw = font.width_text(&title);
            batch.text_styled(
                atlas,
                &title,
                ((w - tw) / 2.0).trunc(),
                15.0,
                u32::MAX,
                true,
            );
            let gray = named_color("gray").unwrap_or(0xAAAAAA);
            let mut y = 63.0 + 30.0;
            for entry in &self.entries {
                let x = (w / 2.0 - 155.0).trunc();
                batch.text_styled(atlas, &entry.title, x, y, u32::MAX, true);
                y += 12.0;
                let line = Text::translate("selectWorld.experimental.details.entry", &[])
                    .append(Text::literal(entry.flags.clone()).with_color(gray));
                for (l, _) in font.split_lines(&line, WARNING) {
                    batch.text_styled(atlas, &l, x, y, u32::MAX, true);
                    y += 9.0;
                }
                y += 10.0;
            }
            let [(bx, by)] = footer_single(w, h);
            ui::button(
                batch,
                atlas,
                bx,
                by,
                BUTTON,
                20.0,
                &Text::translate("gui.back", &[]),
                true,
                ui::hovered(cursor, bx, by, BUTTON, 20.0),
            );
            return;
        }
        let message = Self::message_lines(font);
        let top = Self::block_top(message.len(), h);
        // The title, the message, Details, then the two buttons: the grid's
        // rows 20 apart, the whole block centred.
        let title = Text::translate("selectWorld.experimental.title", &[]);
        let tw = font.width_text(&title);
        batch.text_styled(atlas, &title, ((w - tw) / 2.0).trunc(), top, u32::MAX, true);
        let (dx, dy) = Self::details_rect(message.len(), w, h);
        let mut y = top + 9.0;
        for line in &message {
            batch.text_styled(
                atlas,
                line,
                ((w - WARNING) / 2.0).trunc(),
                y,
                u32::MAX,
                true,
            );
            y += 9.0;
        }
        ui::button(
            batch,
            atlas,
            dx,
            dy,
            DETAILS,
            20.0,
            &Text::translate("selectWorld.experimental.details", &[]),
            true,
            ui::hovered(cursor, dx, dy, DETAILS, 20.0),
        );
        let [(px, py), (bx, by)] = footer_pair(w, h);
        for (fx, fy, key) in [(px, py, "gui.proceed"), (bx, by, "gui.back")] {
            ui::button(
                batch,
                atlas,
                fx,
                fy,
                BUTTON,
                20.0,
                &Text::translate(key, &[]),
                true,
                ui::hovered(cursor, fx, fy, BUTTON, 20.0),
            );
        }
    }
}

/// Two 150-wide buttons `COLUMN_SPACING` apart, centred.
fn footer_pair(w: f32, h: f32) -> [(f32, f32); 2] {
    let x = ((w - (BUTTON * 2.0 + COLUMN_SPACING)) / 2.0).trunc();
    let y = h - 45.0;
    [(x, y), (x + BUTTON + COLUMN_SPACING, y)]
}
/// One centred footer button (`DetailsScreen`'s Back).
fn footer_single(w: f32, h: f32) -> [(f32, f32); 1] {
    [(((w - BUTTON) / 2.0).trunc(), h - 45.0)]
}
