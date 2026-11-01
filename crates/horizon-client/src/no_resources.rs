//! The screen of a game started without its prepared files: the game build
//! run by hand (the release's game zip) instead of through the launcher. It
//! says where to get the launcher, and quits. Drawn in Horizon's design
//! language with the bundled fonts: none of Minecraft's files are there.
use crate::ui_theme::{self, Face, Ui, color};

/// The launcher's download page.
pub const DOWNLOAD_URL: &str = "https://playhorizon.dev";

const TITLE: &str = "Start Horizon from the launcher";
const PARAGRAPHS: [&str; 3] = [
    "This is the game the Horizon launcher installs and starts. On its own it has none of \
     Minecraft's files, so it can't start.",
    "Horizon needs your own copy of Minecraft: Java Edition. The launcher signs you in and \
     prepares the game files from it.",
    "Download the launcher at playhorizon.dev, then press Play there.",
];
const COLUMN: f32 = 560.0;
const BUTTON_GAP: f32 = 12.0;

/// What a click on the screen does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the launcher's download page.
    Download,
    Quit,
}

pub struct NoResources {
    /// Where the prepared files were looked for, for whoever starts the game by hand on purpose.
    looked_in: String,
}

impl NoResources {
    pub fn new(looked_in: &std::path::Path) -> Self {
        let shown = std::path::absolute(looked_in).unwrap_or_else(|_| looked_in.to_path_buf());
        Self {
            looked_in: format!("No prepared game files in {}", shown.display()),
        }
    }

    /// The buttons' labels, actions and rectangles in dp, and the paragraphs' wrapped
    /// lines: one layout for drawing and clicks.
    fn layout(&self, ui: &Ui) -> Layout {
        let width = COLUMN.min(ui.w - 64.0).max(160.0);
        let x = ((ui.w - width) / 2.0).round();
        let lines: Vec<Vec<String>> = PARAGRAPHS
            .iter()
            .map(|p| wrap(ui, Face::Regular, BODY, p, width))
            .collect();
        let text_h = lines.iter().map(|l| l.len() as f32 * LINE).sum::<f32>()
            + (lines.len() - 1) as f32 * PARAGRAPH_GAP;
        let height = TITLE_LINE + 16.0 + text_h + 28.0 + CONTROL + 20.0 + SMALL_LINE;
        let y = ((ui.h - height) / 2.0).max(24.0).round();
        let buttons_y = y + TITLE_LINE + 16.0 + text_h + 28.0;
        let mut buttons = Vec::new();
        let mut bx = x;
        for (label, action) in [
            ("Download the launcher", Action::Download),
            ("Quit", Action::Quit),
        ] {
            let w = (ui.measure(Face::Medium, BUTTON_TEXT, label) + 40.0).round();
            buttons.push(Button {
                label,
                action,
                x: bx,
                y: buttons_y,
                w,
            });
            bx += w + BUTTON_GAP;
        }
        Layout {
            x,
            y,
            width,
            lines,
            buttons,
        }
    }

    pub fn draw(&self, ui: &mut Ui) {
        let layout = self.layout(ui);
        ui.rounded(0.0, 0.0, ui.w, ui.h, 0.0, color::BG);
        let mut y = layout.y;
        ui.text_line(
            Face::Medium,
            TITLE_SIZE,
            TITLE_LINE,
            layout.x,
            y,
            color::TEXT,
            TITLE,
        );
        y += TITLE_LINE + 16.0;
        for paragraph in &layout.lines {
            for line in paragraph {
                ui.text_line(Face::Regular, BODY, LINE, layout.x, y, color::TEXT, line);
                y += LINE;
            }
            y += PARAGRAPH_GAP;
        }
        for button in &layout.buttons {
            let hovered = button.contains(ui.cursor);
            let (plate, ink) = match (button.action, hovered) {
                (Action::Download, false) => (color::ACCENT, color::ON_ACCENT),
                (Action::Download, true) => {
                    (color::with_alpha(color::ACCENT, 0.85), color::ON_ACCENT)
                }
                (Action::Quit, false) => (color::RAISED, color::TEXT),
                (Action::Quit, true) => (color::HOVER, color::TEXT),
            };
            let h = CONTROL;
            ui.rounded(button.x, button.y, button.w, h, h / 2.0, plate);
            let lw = ui.measure(Face::Medium, BUTTON_TEXT, button.label);
            let lx = button.x + ((button.w - lw) / 2.0).round();
            ui.text_line(
                Face::Medium,
                BUTTON_TEXT,
                h,
                lx,
                button.y,
                ink,
                button.label,
            );
        }
        let small_y = layout.buttons[0].y + CONTROL + 20.0;
        for (i, line) in wrap(
            ui,
            Face::Regular,
            ui_theme::SMALL,
            &self.looked_in,
            layout.width,
        )
        .iter()
        .enumerate()
        {
            let line_y = small_y + i as f32 * SMALL_LINE;
            ui.text_line(
                Face::Regular,
                ui_theme::SMALL,
                SMALL_LINE,
                layout.x,
                line_y,
                color::MUTED,
                line,
            );
        }
    }

    /// The button under the cursor (`ui.cursor`, in dp), if any.
    pub fn click(&self, ui: &Ui) -> Option<Action> {
        self.layout(ui)
            .buttons
            .iter()
            .find(|b| b.contains(ui.cursor))
            .map(|b| b.action)
    }
}

const TITLE_SIZE: f32 = horizon_theme::TITLE.px;
const TITLE_LINE: f32 = 40.0;
const BODY: f32 = horizon_theme::BODY.px;
const LINE: f32 = 24.0;
const PARAGRAPH_GAP: f32 = 10.0;
const BUTTON_TEXT: f32 = horizon_theme::UI.px;
const SMALL_LINE: f32 = 18.0;
const CONTROL: f32 = horizon_theme::CONTROL_LARGE;

struct Layout {
    x: f32,
    y: f32,
    width: f32,
    lines: Vec<Vec<String>>,
    buttons: Vec<Button>,
}

struct Button {
    label: &'static str,
    action: Action,
    x: f32,
    y: f32,
    w: f32,
}

impl Button {
    fn contains(&self, (cx, cy): (f32, f32)) -> bool {
        cx >= self.x && cx < self.x + self.w && cy >= self.y && cy < self.y + CONTROL
    }
}

/// `text` broken at spaces into lines no wider than `width` dp (a longer word,
/// such as a path, gets a line of its own).
fn wrap(ui: &Ui, face: Face, size: f32, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && ui.measure(face, size, &candidate) > width {
            lines.push(std::mem::replace(&mut line, word.to_owned()));
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{HudAtlas, HudBatch};

    /// A Ui over a `size` window at GUI scale 1, the cursor at `cursor` pixels.
    fn with_ui<R>(size: [u32; 2], cursor: (f32, f32), f: impl FnOnce(&mut Ui) -> R) -> R {
        let atlas =
            HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let mut batch = HudBatch::new(size);
        let cursor = (cursor.0 / batch.scale, cursor.1 / batch.scale);
        let mut ui = Ui::new(&atlas, &mut batch, cursor);
        f(&mut ui)
    }

    #[test]
    fn the_buttons_are_where_they_are_drawn() {
        let screen = NoResources::new(std::path::Path::new(".local/horizon-client/26.3"));
        for size in [[1280, 720], [800, 600], [2560, 1440]] {
            let (download, quit, density) = with_ui(size, (0.0, 0.0), |ui| {
                let layout = screen.layout(ui);
                let center = |b: &Button| (b.x + b.w / 2.0, b.y + CONTROL / 2.0);
                (
                    center(&layout.buttons[0]),
                    center(&layout.buttons[1]),
                    ui.density,
                )
            });
            let at = |p: (f32, f32)| (p.0 * density, p.1 * density);
            assert_eq!(
                with_ui(size, at(download), |ui| screen.click(ui)),
                Some(Action::Download),
                "{size:?}"
            );
            assert_eq!(
                with_ui(size, at(quit), |ui| screen.click(ui)),
                Some(Action::Quit)
            );
            assert_eq!(with_ui(size, (1.0, 1.0), |ui| screen.click(ui)), None);
            // Everything fits in the window.
            with_ui(size, (0.0, 0.0), |ui| {
                let layout = screen.layout(ui);
                let last = layout.buttons.last().unwrap();
                assert!(
                    last.x + last.w <= ui.w && last.y + CONTROL <= ui.h,
                    "{size:?}"
                );
            });
        }
    }

    #[test]
    fn text_wraps_within_the_column_and_keeps_every_word() {
        with_ui([1280, 720], (0.0, 0.0), |ui| {
            for paragraph in PARAGRAPHS {
                let lines = wrap(ui, Face::Regular, BODY, paragraph, 300.0);
                assert!(lines.len() > 1, "{paragraph}");
                for line in &lines {
                    assert!(ui.measure(Face::Regular, BODY, line) <= 300.0, "{line}");
                }
                assert_eq!(lines.join(" "), paragraph);
            }
        });
    }
}
