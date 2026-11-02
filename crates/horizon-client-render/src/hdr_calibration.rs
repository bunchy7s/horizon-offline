//! HDR calibration (Graphics: HDR, Calibrate...): full screen, one step at a time, the way
//! display calibration tools do it. HDR output draws each step's test picture straight in
//! nits above a band of controls (`hdr.wgsl`, `Hdr::calibration`), past everything that
//! shapes the game's picture, so what the player sees is the display alone:
//!
//! - Black Level: Horizon's logo at the slider's brightness on 0 nits; lowered until it
//!   disappears, it sits on the darkest the display shows.
//! - Peak Brightness: a patch of a tenth of the screen at 10 000 nits (the display clips
//!   it at its brightest), the logo inside at the slider; raised until it disappears, it
//!   sits on that brightest. Small, so the display doesn't dim a bright full screen; the
//!   HDR metadata declares PQ's 10 000 nits meanwhile, so a compositor that clips at the
//!   declared peak (KWin) lets the patch through.
//! - Paper White: a white card at the slider, matched to a sheet of white paper held
//!   beside the screen in the light the player plays in: white is where the eyes have
//!   adapted to, lower in a dark room than by day (as ITU-R BT.2408's reference white).
//!
//! Nothing changes until Save, which writes the three into Horizon's settings; Cancel
//! or Escape leaves them as they were.
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    lang,
    settings::Settings,
    text::Text,
    ui,
};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Black,
    Peak,
    PaperWhite,
}
const STEPS: [Step; 3] = [Step::Black, Step::Peak, Step::PaperWhite];

/// What the options screen does after an input here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Click,
    Changed,
    Save,
    Cancel,
}

pub struct Calibration {
    step: usize,
    /// Nits, in the order of `STEPS`.
    values: [f32; 3],
    /// Which values were Auto (peak, paper white): saved as Auto again unless the
    /// player moved them.
    auto: [Option<f32>; 3],
    dragging: bool,
}

/// Height of the band of controls at the bottom, in GUI pixels.
const BAND: f32 = 84.0;

impl Step {
    /// Range and step of the slider, in nits: the settings' own.
    fn range(self) -> (f32, f32, f32) {
        match self {
            Step::Black => (0.0, 1.0, 0.01),
            Step::Peak => (200.0, 2000.0, 50.0),
            Step::PaperWhite => (10.0, 480.0, 10.0),
        }
    }
    /// The English name, which is also its key in the language table.
    fn name_key(self) -> &'static str {
        match self {
            Step::Black => "Black Level",
            Step::Peak => "Peak Brightness",
            Step::PaperWhite => "Paper White",
        }
    }
    fn name(self) -> String {
        lang::get(self.name_key())
    }
    /// The English hint, which is also its key in the language table.
    fn hint_key(self) -> &'static str {
        match self {
            Step::Black => "Lower it until the logo just disappears into the black.",
            Step::Peak => "Raise it until the logo just disappears into the bright square.",
            Step::PaperWhite => {
                "Make the card a white that's comfortable to look at for a long time, in the light you play in."
            }
        }
    }
    /// `hdr::Calibration::step`'s number.
    pub fn number(self) -> u32 {
        match self {
            Step::Black => 1,
            Step::Peak => 2,
            Step::PaperWhite => 3,
        }
    }
}

struct Layout {
    top: f32,
    slider: (f32, f32, f32),
    cancel: (f32, f32, f32),
    back: (f32, f32, f32),
    next: (f32, f32, f32),
}
fn layout(w: f32, h: f32) -> Layout {
    let top = (h - BAND).floor();
    let cx = (w / 2.0).floor();
    let by = top + 58.0;
    Layout {
        top,
        slider: (cx - 100.0, top + 32.0, 200.0),
        cancel: (cx - 152.0, by, 98.0),
        back: (cx - 49.0, by, 98.0),
        next: (cx + 54.0, by, 98.0),
    }
}
fn over(cursor: (f32, f32), (x, y, w): (f32, f32, f32)) -> bool {
    ui::hovered(cursor, x, y, w, 20.0)
}

impl Calibration {
    pub fn new(s: &Settings) -> Self {
        let g = &s.horizon;
        Self {
            step: 0,
            values: [
                g.black_level_nits(),
                g.peak_nits() as f32,
                g.paper_white_nits() as f32,
            ],
            auto: [
                (g.hdr_black_level == 0).then_some(g.black_level_nits()),
                (g.hdr_peak == 0).then_some(g.peak_nits() as f32),
                (g.hdr_paper_white == 0).then_some(g.paper_white_nits() as f32),
            ],
            dragging: false,
        }
    }
    fn current(&self) -> Step {
        STEPS[self.step]
    }
    fn set(&mut self, v: f32) {
        let (min, max, step) = self.current().range();
        self.values[self.step] = (min + ((v - min) / step).round() * step).clamp(min, max);
    }
    fn label(&self) -> Text {
        let v = self.values[self.step];
        let step = self.current();
        let value = match step {
            Step::Black => format!("{v:.2}"),
            _ => format!("{v:.0}"),
        };
        Text::literal(format!("{}: {}", step.name(), value))
    }
    /// The step, its nits and where the controls start (a share of the height), for
    /// HDR output's test picture.
    pub fn picture(&self, h: f32) -> (Step, f32, f32) {
        (
            self.current(),
            self.values[self.step],
            layout(0.0, h).top / h.max(1.0),
        )
    }
    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32)) {
        let (w, h) = (batch.width, batch.height);
        let l = layout(w, h);
        // Above the band HDR output draws the test picture over whatever is here.
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 255));
        batch.fill(atlas, 0.0, l.top, w, h - l.top, rgba(24, 24, 24, 255));
        let step = self.current();
        let title = Text::literal(lang::tr(
            "HDR Calibration %s/%s: %s",
            &[&(self.step + 1), &STEPS.len(), &step.name()],
        ));
        ui::centered_text(batch, atlas, &title, 0.0, l.top + 6.0, w, u32::MAX);
        let hint = Text::literal(lang::get(step.hint_key()));
        ui::centered_text(
            batch,
            atlas,
            &hint,
            0.0,
            l.top + 18.0,
            w,
            rgba(160, 160, 160, 255),
        );
        let (min, max, _) = step.range();
        let (x, y, sw) = l.slider;
        ui::slider(
            batch,
            atlas,
            x,
            y,
            sw,
            &self.label(),
            (self.values[self.step] - min) / (max - min),
            over(cursor, l.slider) || self.dragging,
            true,
            self.dragging,
        );
        let last = self.step + 1 == STEPS.len();
        let save = lang::get_or("selectWorld.edit.save", "Save");
        let next = lang::get("Next");
        for (rect, label, active) in [
            (l.cancel, lang::get_or("gui.cancel", "Cancel"), true),
            (l.back, lang::get_or("gui.back", "Back"), self.step > 0),
            (l.next, if last { save } else { next }, true),
        ] {
            let hover = over(cursor, rect) && active;
            ui::button(
                batch,
                atlas,
                rect.0,
                rect.1,
                rect.2,
                20.0,
                &Text::literal(label),
                active,
                hover,
            );
        }
    }
    /// A press, or a drag with `drag`, at `cursor`.
    pub fn click(&mut self, cursor: (f32, f32), (w, h): (f32, f32), drag: bool) -> Outcome {
        let l = layout(w, h);
        let (x, _, sw) = l.slider;
        if drag {
            if !self.dragging {
                return Outcome::Nothing;
            }
            let (min, max, _) = self.current().range();
            self.set(min + ui::slider_value(x, sw, cursor.0) * (max - min));
            return Outcome::Changed;
        }
        if over(cursor, l.slider) {
            self.dragging = true;
            let (min, max, _) = self.current().range();
            self.set(min + ui::slider_value(x, sw, cursor.0) * (max - min));
            return Outcome::Changed;
        }
        if over(cursor, l.cancel) {
            return Outcome::Cancel;
        }
        if over(cursor, l.back) && self.step > 0 {
            self.step -= 1;
            return Outcome::Click;
        }
        if over(cursor, l.next) {
            return self.next();
        }
        Outcome::Nothing
    }
    fn next(&mut self) -> Outcome {
        if self.step + 1 == STEPS.len() {
            return Outcome::Save;
        }
        self.step += 1;
        Outcome::Click
    }
    /// The slider let go: true if it was held (it clicks on release, as sliders do).
    pub fn release(&mut self) -> bool {
        std::mem::take(&mut self.dragging)
    }
    /// Left and Right step the slider, Enter goes on, Escape cancels.
    pub fn key(&mut self, key: KeyCode) -> Outcome {
        let (_, _, step) = self.current().range();
        match key {
            KeyCode::ArrowLeft | KeyCode::ArrowDown => {
                self.set(self.values[self.step] - step);
                Outcome::Changed
            }
            KeyCode::ArrowRight | KeyCode::ArrowUp => {
                self.set(self.values[self.step] + step);
                Outcome::Changed
            }
            KeyCode::Enter | KeyCode::NumpadEnter => self.next(),
            KeyCode::Escape => Outcome::Cancel,
            _ => Outcome::Nothing,
        }
    }
    /// Writes what the player landed on into Horizon's settings.
    pub fn save(&self, s: &mut Settings) {
        let g = &mut s.horizon;
        let scale = [100.0, 1.0, 1.0];
        let [black, peak, paper_white] = [0, 1, 2].map(|i| match self.auto[i] {
            Some(auto) if auto == self.values[i] => 0,
            _ => (self.values[i] * scale[i]).round() as u32,
        });
        g.hdr_black_level = black;
        g.hdr_peak = peak;
        g.hdr_paper_white = paper_white;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_saves_only_on_save() {
        let mut s = Settings::default();
        s.horizon.hdr_black_level = 5;
        let before = s.horizon.clone();
        let mut c = Calibration::new(&s);
        assert_eq!(c.key(KeyCode::ArrowLeft), Outcome::Changed);
        assert_eq!(c.key(KeyCode::ArrowLeft), Outcome::Changed);
        assert!((c.picture(100.0).1 - 0.03).abs() < 1e-6);
        assert_eq!(c.key(KeyCode::Escape), Outcome::Cancel);
        assert_eq!(s.horizon, before);
        // Black, then Peak (one step up), then Paper White, then Save.
        assert_eq!(c.key(KeyCode::Enter), Outcome::Click);
        assert_eq!(c.key(KeyCode::ArrowRight), Outcome::Changed);
        assert_eq!(c.key(KeyCode::Enter), Outcome::Click);
        assert_eq!(c.picture(100.0).0, Step::PaperWhite);
        assert_eq!(c.key(KeyCode::Enter), Outcome::Save);
        c.save(&mut s);
        assert_eq!(s.horizon.hdr_black_level, 3);
        assert_eq!(s.horizon.hdr_peak, before.peak_nits() + 50);
        assert_eq!(s.horizon.hdr_paper_white, before.hdr_paper_white);
    }
}
