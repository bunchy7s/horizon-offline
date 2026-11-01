//! CreateWorldScreen's Experiments screen (`ExperimentsScreen`): the feature
//! packs the jar ships, each with an on/off switch, under the red warning that
//! they cannot be turned off after the world is created.
//!
//! The layout follows vanilla's: `HeaderAndFooterLayout` (a 33-pixel title
//! header and footer, the contents starting 30 pixels under the header), a
//! `MultiLineTextWidget` warning 310 wide, then a `SwitchGrid` 299 wide with
//! its information underneath each switch.
use crate::{
    font::Font,
    hud::{HudAtlas, HudBatch},
    text::{Text, named_color},
    ui,
};

/// `HeaderAndFooterLayout`'s header and footer.
const HEADER: f32 = 33.0;
const FOOTER: f32 = 33.0;
/// `SwitchGrid.builder(299)`, its `DEFAULT_SWITCH_BUTTON_WIDTH` and the
/// information's maximum width (`width - paddingLeft - buttonWidth`).
const GRID: f32 = 299.0;
const BUTTON: f32 = 44.0;
const INFO: f32 = 266.0 - 11.0;
/// `ExperimentsScreen.MAIN_CONTENT_WIDTH`: the warning's maximum width.
const WARNING: f32 = 310.0;
/// `withInfoUnderneath(2, true)`: two rows of information, always that tall.
const MAX_INFO_ROWS: usize = 2;
/// `withRowSpacing(4)`.
const ROW_SPACING: f32 = 4.0;
/// One switch and its two information rows, `rowSpacing` included.
const FLAG_HEIGHT: f32 = 20.0 + ROW_SPACING + (9.0 * MAX_INFO_ROWS as f32);

/// One feature pack: `SwitchGrid.addSwitch(...).withInfo(description)`.
pub struct Flag {
    pub id: String,
    pub title: Text,
    pub description: Text,
    pub enabled: bool,
}

/// What a press asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// `gui.done`: the switches go to the selection.
    Done,
    /// `gui.cancel` or Escape.
    Cancel,
}

/// A press on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    /// The i-th switch.
    Switch(usize),
    Outcome(Outcome),
}

/// The screen.
pub struct ExperimentsScreen {
    pub flags: Vec<Flag>,
    pub in_world: bool,
}
impl ExperimentsScreen {
    pub fn new(flags: Vec<Flag>) -> Self {
        Self {
            flags,
            in_world: false,
        }
    }
    /// `HeaderAndFooterLayout.arrangeElements`: the contents start at the
    /// header's height plus 30.
    fn content_top() -> f32 {
        HEADER + 30.0
    }
    /// The warning, red (`ExperimentsScreen.INFO`).
    fn warning(&self) -> Text {
        Text::translate("selectWorld.experiments.info", &[])
            .with_color(named_color("red").unwrap_or(0xFF5555))
    }
    /// The warning's lines (`MultiLineTextWidget.setMaxWidth(310)`).
    fn warning_lines(&self, font: &Font) -> Vec<Text> {
        font.split_lines(&self.warning(), WARNING)
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }
    /// Where the `SwitchGrid` starts: under the warning and its 15-pixel
    /// bottom padding.
    fn grid_top(&self, font: &Font) -> f32 {
        Self::content_top() + 9.0 * self.warning_lines(font).len() as f32 + 15.0
    }
    fn grid_left(w: f32) -> f32 {
        ((w - GRID) / 2.0).trunc()
    }
    /// The i-th switch's button (`alignHorizontallyRight` in the 44-wide
    /// column).
    fn switch_rect(&self, font: &Font, w: f32, i: usize) -> (f32, f32) {
        let top = self.grid_top(font) + i as f32 * FLAG_HEIGHT;
        (Self::grid_left(w) + GRID - BUTTON, top)
    }
    /// A press: a switch, Done (`gui.done`) or Cancel (`gui.cancel`).
    pub fn click(&self, atlas: &HudAtlas, (x, y): (f32, f32), size: (f32, f32)) -> Option<Click> {
        for i in 0..self.flags.len() {
            let (bx, by) = self.switch_rect(&atlas.font, size.0, i);
            if ui::hovered((x, y), bx, by, BUTTON, 20.0) {
                return Some(Click::Switch(i));
            }
        }
        let [(dx, dy), (cx, cy)] = footer(size.0, size.1);
        if ui::hovered((x, y), dx, dy, 150.0, 20.0) {
            return Some(Click::Outcome(Outcome::Done));
        }
        if ui::hovered((x, y), cx, cy, 150.0, 20.0) {
            return Some(Click::Outcome(Outcome::Cancel));
        }
        None
    }
    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.in_world);
        let font = &atlas.font;
        // `addTitleHeader`: the title centred in the header's 33 pixels.
        let title = Text::translate("selectWorld.experiments", &[]);
        let tw = font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).trunc(),
            15.0,
            u32::MAX,
            true,
        );
        // The warning, centred as a 310-wide block.
        let mut y = Self::content_top();
        for line in self.warning_lines(font) {
            batch.text_styled(
                atlas,
                &line,
                ((w - WARNING) / 2.0).trunc(),
                y,
                u32::MAX,
                true,
            );
            y += 9.0;
        }
        // `SwitchGrid`: the label left, the switch right, the information
        // grey underneath.
        let left = Self::grid_left(w);
        let gray = named_color("gray").unwrap_or(0xAAAAAA);
        for (i, flag) in self.flags.iter().enumerate() {
            let (bx, by) = self.switch_rect(font, w, i);
            let label_y = by + 5.0;
            batch.text_styled(atlas, &flag.title, left, label_y, u32::MAX, true);
            let hover = ui::hovered(cursor, bx, by, BUTTON, 20.0);
            let value = Text::translate(
                if flag.enabled {
                    "options.on"
                } else {
                    "options.off"
                },
                &[],
            );
            ui::button(batch, atlas, bx, by, BUTTON, 20.0, &value, true, hover);
            let mut info_y = by + 20.0;
            for line in font.split_lines(&flag.description.clone().with_color(gray), INFO) {
                batch.text_styled(atlas, &line.0, left, info_y, u32::MAX, true);
                info_y += 9.0;
            }
        }
        let [(dx, dy), (cx, cy)] = footer(w, h);
        for (fx, fy, key) in [(dx, dy, "gui.done"), (cx, cy, "gui.cancel")] {
            let hover = ui::hovered(cursor, fx, fy, 150.0, 20.0);
            ui::button(
                batch,
                atlas,
                fx,
                fy,
                150.0,
                20.0,
                &Text::translate(key, &[]),
                true,
                hover,
            );
        }
    }
}

/// `HeaderAndFooterLayout`: two 150-wide buttons 8 apart in the 33-pixel
/// footer.
fn footer(w: f32, h: f32) -> [(f32, f32); 2] {
    let x = ((w - 308.0) / 2.0).trunc();
    let y = h - FOOTER + 6.0;
    [(x, y), (x + 158.0, y)]
}

/// `ExperimentsScreen.init`: every feature pack the jar ships under
/// `data/minecraft/datapacks`, switched on when `enabled` holds its id.
pub fn repository_flags(resources: &std::path::Path, enabled: &[String]) -> Vec<Flag> {
    crate::datapack::feature_packs(resources)
        .into_iter()
        .map(|pack| {
            let id = pack.id.clone();
            // `ExperimentsScreen.getHumanReadableTitle`.
            let name = format!("dataPack.{id}.name");
            let title = if crate::lang::has(&name) {
                Text::translate(&name, &[])
            } else {
                Text::literal(id.clone())
            };
            Flag {
                enabled: enabled.contains(&id),
                description: Text::translate(&format!("dataPack.{id}.description"), &[]),
                id,
                title,
            }
        })
        .collect()
}
