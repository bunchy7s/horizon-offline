//! `AccessibilityOnboardingScreen`: the screen shown once before the title
//! on a first launch (`Gui.addInitialScreens` while `onboardAccessibility`
//! is set), with the Narrator toggle, the Sound, Accessibility and Language
//! pages and Continue. It fades in over two seconds and out over one.
use super::{Button, Mode, Title, translate};
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    lang,
};
use std::{cell::Cell, time::Instant};

/// `AccessibilityOnboardingScreen.TEXT_WIDGET_WIDTH`.
const TEXT_WIDTH: f32 = 374.0;
/// `HeaderAndFooterLayout(this, initTitleYPos(), 33)`.
const HEADER: f32 = 90.0;
const FOOTER: f32 = 33.0;
/// `fadeInStart` runs for 2000 ms, `FADE_OUT_TIME` is 1000 ms.
const FADE_IN_SECONDS: f32 = 2.0;
const FADE_OUT_SECONDS: f32 = 1.0;

/// The ids of the screen's buttons.
pub(super) const NARRATOR: u8 = 80;
pub(super) const SOUNDS: u8 = 81;
pub(super) const ACCESSIBILITY: u8 = 82;
pub(super) const LANGUAGE: u8 = 83;
pub(super) const CONTINUE: u8 = 84;

/// The fades of an open screen (`fadeInStart`, `fadingIn`, `fadeOutStart`).
#[derive(Clone, Debug)]
pub struct Onboarding {
    fade_in_start: Cell<Option<Instant>>,
    fading_in: Cell<bool>,
    fade_out_start: Cell<Option<Instant>>,
    /// The widget alpha of the frame being drawn.
    alpha: Cell<f32>,
}

impl Onboarding {
    pub fn new() -> Self {
        Self {
            fade_in_start: Cell::new(None),
            fading_in: Cell::new(true),
            fade_out_start: Cell::new(None),
            alpha: Cell::new(1.0),
        }
    }

    /// `Mth.clampedMap(fade, 0.5, 1, 0, 1)`.
    fn widget_alpha(fade: f32) -> f32 {
        ((fade.clamp(0.0, 1.0) - 0.5) / 0.5).clamp(0.0, 1.0)
    }

    /// `extractRenderState`'s fades at `now`: the alpha every widget gets.
    fn fade(&self, now: Instant) -> f32 {
        if self.fade_in_start.get().is_none() && self.fading_in.get() {
            self.fade_in_start.set(Some(now));
        }
        let mut alpha = 1.0;
        if let Some(start) = self.fade_in_start.get() {
            let fade = now.saturating_duration_since(start).as_secs_f32() / FADE_IN_SECONDS;
            if fade >= 1.0 {
                self.fading_in.set(false);
                self.fade_in_start.set(None);
            } else {
                alpha = Self::widget_alpha(fade);
            }
        }
        if let Some(start) = self.fade_out_start.get() {
            let fade = 1.0 - now.saturating_duration_since(start).as_secs_f32() / FADE_OUT_SECONDS;
            alpha = if fade <= 0.0 {
                0.0
            } else {
                Self::widget_alpha(fade)
            };
        }
        alpha
    }

    /// `onClose`: starts the fade out once.
    fn close(&self, now: Instant) {
        if self.fade_out_start.get().is_none() {
            self.fade_out_start.set(Some(now));
        }
    }

    /// Whether the fade out has run out: `close(true, onClose)`.
    fn closed(&self, now: Instant) -> bool {
        self.fade_out_start.get().is_some_and(|start| {
            now.saturating_duration_since(start).as_secs_f32() >= FADE_OUT_SECONDS
        })
    }
}

impl Default for Onboarding {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the screen's parts go.
pub(super) struct Layout {
    /// The title box: x, y, width, height.
    pub text: [f32; 4],
    /// The four grid buttons, row by row.
    pub grid: [(f32, f32); 4],
    pub continue_at: (f32, f32),
}

/// `HeaderAndFooterLayout.arrangeElements` for the contents (the title in
/// an 8 px cell over the grid of four 150-wide buttons in 4 px cells, both
/// centred) with the title wrapped to `lines`.
pub(super) fn layout(w: f32, h: f32, lines: usize) -> Layout {
    let text_h = 9.0 * lines as f32 + 8.0;
    // The 8 px cell around the box, then the grid's 4 px cell around the
    // two rows of 4 px cells.
    let content_w = TEXT_WIDTH + 16.0;
    let content_h = text_h + 16.0 + 64.0;
    let y = (HEADER + 30.0).min(h - FOOTER - content_h);
    let x = ((w.max(content_w) - content_w) / 2.0).trunc();
    // The grid: 316 wide in the 382 left by the cell's padding.
    let grid_x = x + 4.0 + ((content_w - 8.0 - 316.0) / 2.0).trunc() + 4.0;
    let grid_y = y + text_h + 16.0 + 4.0 + 4.0;
    Layout {
        text: [x + 8.0, y + 8.0, TEXT_WIDTH, text_h],
        grid: [
            (grid_x, grid_y),
            (grid_x + 158.0, grid_y),
            (grid_x, grid_y + 28.0),
            (grid_x + 158.0, grid_y + 28.0),
        ],
        continue_at: (
            ((w - 150.0) / 2.0).trunc(),
            h - FOOTER + ((FOOTER - 20.0) / 2.0).trunc(),
        ),
    }
}

/// The wrapped lines of the title (`accessibility.onboarding.screen.title`).
fn title_lines(atlas: &HudAtlas) -> Vec<String> {
    let title = lang::get_or(
        "accessibility.onboarding.screen.title",
        "Welcome to Minecraft!\n\nWould you like to enable the Narrator or visit the Accessibility Settings?",
    );
    let mut lines = Vec::new();
    for paragraph in title.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
        } else {
            lines.extend(
                atlas
                    .font
                    .split(&crate::text::Text::literal(paragraph), TEXT_WIDTH - 8.0)
                    .iter()
                    .map(|line| line.plain()),
            );
        }
    }
    lines
}

impl Title {
    /// The first screen of a first launch (`addInitialScreens`).
    pub fn start_onboarding(&mut self) {
        self.mode = Mode::Onboarding(Onboarding::new());
    }

    /// `onClose`: Escape and Continue fade the screen out.
    pub(super) fn onboarding_close(&self) {
        if let Mode::Onboarding(screen) = &self.mode {
            screen.close(Instant::now());
        }
    }

    /// Once the fade out is over the title follows and the owner stores that
    /// onboarding is finished (`Options.onboardingAccessibilityFinished`).
    pub(super) fn poll_onboarding(&mut self, now: Instant) {
        if let Mode::Onboarding(screen) = &self.mode
            && screen.closed(now)
        {
            self.mode = Mode::Main;
            self.onboarding_finished = true;
        }
    }

    pub(super) fn onboarding_buttons(&self, w: f32, h: f32) -> Vec<Button> {
        let layout = layout(w, h, self.message_lines.get());
        let button =
            |(x, y): (f32, f32), label: String, enabled: bool, id: u8, icon: Option<_>| Button {
                x,
                y,
                w: 150.0,
                label,
                enabled,
                id,
                icon: None,
                trailing: icon,
            };
        // The Narrator button is inactive without a narrator, and shows
        // `options.narrator.notavailable`.
        let narrator = translate(
            "options.generic_value",
            "%s: %s",
            &[
                &lang::get_or("options.narrator", "Narrator"),
                &lang::get_or("options.narrator.notavailable", "Not Available"),
            ],
        );
        vec![
            button(layout.grid[0], narrator, false, NARRATOR, None),
            button(
                layout.grid[1],
                lang::get_or("options.sounds", "Music & Sounds..."),
                true,
                SOUNDS,
                Some(("icon/music_notes", 16.0)),
            ),
            button(
                layout.grid[2],
                lang::get_or(
                    "accessibility.onboarding.accessibility.button",
                    "Accessibility Settings...",
                ),
                true,
                ACCESSIBILITY,
                Some(("icon/accessibility", 15.0)),
            ),
            button(
                layout.grid[3],
                lang::get_or("options.language", "Language..."),
                true,
                LANGUAGE,
                Some(("icon/language", 15.0)),
            ),
            button(
                layout.continue_at,
                lang::get_or("gui.continue", "Continue"),
                true,
                CONTINUE,
                None,
            ),
        ]
    }

    /// The title box over the background; the index of its first vertex,
    /// where the widgets' fade begins.
    pub(super) fn draw_onboarding(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        w: f32,
        h: f32,
    ) -> usize {
        let Mode::Onboarding(screen) = &self.mode else {
            return batch.vertices.len();
        };
        screen.alpha.set(screen.fade(Instant::now()));
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        let lines = title_lines(atlas);
        self.message_lines.set(lines.len());
        let first = batch.vertices.len();
        let [x, y, bw, bh] = layout(w, h, lines.len()).text;
        // FocusableTextWidget: a black fill from x + 1 and the grey outline.
        batch.fill(atlas, x + 1.0, y, bw - 1.0, bh, rgba(0, 0, 0, 255));
        let border = rgba(160, 160, 160, 255);
        batch.fill(atlas, x, y, bw, 1.0, border);
        batch.fill(atlas, x, y + bh - 1.0, bw, 1.0, border);
        batch.fill(atlas, x, y, 1.0, bh, border);
        batch.fill(atlas, x + bw - 1.0, y, 1.0, bh, border);
        // Centred on the widget, the text 4 px inside.
        let middle = x + (bw / 2.0).trunc();
        for (i, line) in lines.iter().enumerate() {
            let width = HudBatch::text_width(atlas, line);
            batch.text(
                atlas,
                line,
                middle - (width.ceil() / 2.0).floor(),
                y + 4.0 + 9.0 * i as f32,
                u32::MAX,
                true,
            );
        }
        first
    }

    /// After the buttons: every widget from `from` takes the fade's alpha,
    /// then the logo (`LogoRenderer(true)` keeps it through the fade).
    pub(super) fn draw_onboarding_logo(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        w: f32,
        from: usize,
    ) {
        let Mode::Onboarding(screen) = &self.mode else {
            return;
        };
        let alpha = screen.alpha.get();
        if alpha < 1.0 {
            for vertex in &mut batch.vertices[from..] {
                let a = (f32::from((vertex.color >> 24) as u8) * alpha).round() as u32;
                vertex.color = vertex.color & 0x00FF_FFFF | a << 24;
            }
        }
        super::draw_logo(atlas, batch, w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_widgets_sit_where_the_layouts_put_them() {
        // 854 x 480 GUI pixels, the title wrapped to 4 lines: the contents
        // frame starts at min(90 + 30, 480 - 33 - 124).
        let l = layout(854.0, 480.0, 4);
        assert_eq!(l.text, [240.0, 128.0, 374.0, 44.0]);
        assert_eq!(l.grid[0], (273.0, 188.0));
        assert_eq!(l.grid[1], (431.0, 188.0));
        assert_eq!(l.grid[2], (273.0, 216.0));
        assert_eq!(l.continue_at, (352.0, 453.0));
        // A short window moves the contents up to clear the footer.
        let l = layout(854.0, 240.0, 4);
        assert_eq!(l.text[1], 240.0 - 33.0 - 124.0 + 8.0);
    }

    #[test]
    fn the_fades_run_two_seconds_in_and_one_out() {
        let screen = Onboarding::new();
        let t0 = Instant::now();
        assert_eq!(screen.fade(t0), 0.0);
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        // Invisible for the first second, then up to full at two seconds.
        assert_eq!(screen.fade(at(1.0)), 0.0);
        assert!((screen.fade(at(1.5)) - 0.5).abs() < 1e-3);
        assert_eq!(screen.fade(at(2.0)), 1.0);
        assert!(!screen.fading_in.get());
        // Close: full at once, gone after a second, the title follows.
        screen.close(at(3.0));
        assert!(!screen.closed(at(3.5)));
        assert!((screen.fade(at(3.25)) - 0.5).abs() < 1e-3);
        assert_eq!(screen.fade(at(4.0)), 0.0);
        assert!(screen.closed(at(4.0)));
    }

    #[test]
    fn escape_continues_and_the_title_follows_the_fade() {
        let mut title = Title::new(
            std::path::PathBuf::new(),
            std::path::Path::new("/nonexistent/horizon-test-resources"),
        );
        title.start_onboarding();
        title.escape();
        assert!(matches!(title.mode, Mode::Onboarding(_)));
        title.poll_onboarding(Instant::now());
        assert!(!title.onboarding_finished);
        title.poll_onboarding(Instant::now() + Duration::from_secs(2));
        assert!(matches!(title.mode, Mode::Main));
        assert!(title.onboarding_finished);
    }
}
