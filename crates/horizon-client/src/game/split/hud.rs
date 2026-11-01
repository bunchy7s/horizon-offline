//! What splitscreen draws over a player's view: its badge, questions answered with the pad and
//! notices.
use super::*;
use horizon_client::gamepad::Button;
use horizon_client::hud::{HudBatch, rgba};
use std::time::Duration;

/// What splitscreen draws over a player's view (`Game::split_hud`): its corner badge (who, for a
/// moment, with why), a question its controller answers, and notices nobody has to answer.
#[derive(Default)]
pub(in crate::game) struct Hud {
    pub(super) badge: Option<String>,
    pub(super) place: Option<(String, Instant)>,
    pub(super) prompt: Option<Prompt>,
    pub(super) notice: Option<Notice>,
}

/// A notice nobody answers: a strip at the top of the view in the prompts' style, its words
/// wrapped to the view (a toast is as wide as its longest line: off a narrow view's edge), gone
/// after the toasts' time (`notification_display_time`).
pub(super) struct Notice {
    title: String,
    body: String,
    /// When it was raised: it goes that long after, drawn or not (a notice raised on the way
    /// out of a world must not wait for the next one).
    raised: Instant,
}

impl Notice {
    pub(super) fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            raised: Instant::now(),
        }
    }
}

/// The prompts' and notices' box: dark, a faint edge.
fn panel(batch: &mut HudBatch, atlas: &horizon_client::hud::HudAtlas, [px, py, pw, ph]: [f32; 4]) {
    batch.fill(atlas, px, py, pw, ph, rgba(0, 0, 0, 210));
    let edge = rgba(255, 255, 255, 90);
    for [x, y, fw, fh] in [
        [px, py, pw, 1.0],
        [px, py + ph - 1.0, pw, 1.0],
        [px, py, 1.0, ph],
        [px + pw - 1.0, py, 1.0, ph],
    ] {
        batch.fill(atlas, x, y, fw, fh, edge);
    }
}

/// A question on a player's view, answered with its controller's buttons.
#[derive(Clone, PartialEq)]
pub(super) struct Prompt {
    pub(super) ask: Ask,
    pub(super) title: String,
    pub(super) choices: Vec<(Button, &'static str)>,
}

/// What a prompt asks: a controller may join (every view, a strip at the top: nobody stops
/// playing for it), or its controller disconnected (the view dimmed beneath: any controller
/// answers).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Ask {
    Join,
    Lost,
}

/// The question on a waiting player's view.
pub(super) fn lost_prompt(name: &str) -> Prompt {
    Prompt {
        ask: Ask::Lost,
        title: format!("{name}: controller disconnected"),
        choices: vec![(Button::South, "Use another"), (Button::East, "Leave")],
    }
}

impl Game<'_> {
    /// Splitscreen over this player's view, above the other toasts (`ToastManager`'s slots).
    pub(in crate::game) fn draw_split_hud(&mut self, batch: &mut HudBatch, hidden: bool) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        let width = |text: &str| HudBatch::text_width(atlas, text).ceil();
        // After a choice's words, room before the next; a glyph without words ("LB" of "LB RB
        // Move") sits right next to the next glyph.
        let gap = |what: &str| if what.is_empty() { 0.0 } else { 12.0 };
        // The badge, top left: who, and a moment where. F1 hides it with the rest of the HUD.
        if let (false, Some(badge)) = (hidden, &self.split_hud.badge) {
            let place = self.split_hud.place.as_ref();
            if let Some((place, _)) = place.filter(|(_, until)| Instant::now() < *until) {
                let text = format!("{badge} · {place}");
                batch.text(atlas, &text, 2.0, 2.0, rgba(255, 255, 255, 230), true);
            }
        }
        // A question: centred over the view dimmed, or a strip at the top (a controller joining).
        if let Some(prompt) = &self.split_hud.prompt {
            let style = self.ui.options.pad_style;
            let dim = prompt.ask != Ask::Join;
            if dim {
                batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 120));
            }
            let glyphs: Vec<_> = prompt
                .choices
                .iter()
                .map(|&(b, what)| (b.label(style), b.color(style), what))
                .collect();
            let line: f32 = glyphs
                .iter()
                .map(|(g, _, what)| width(g) + 8.0 + width(what) + gap(what))
                .sum::<f32>()
                - 12.0;
            let pw = width(&prompt.title).max(line) + 16.0;
            let ph = 34.0;
            let (px, py) = (
                ((w - pw) / 2.0).floor(),
                if dim { ((h - ph) / 2.0).floor() } else { 18.0 },
            );
            panel(batch, atlas, [px, py, pw, ph]);
            let tx = (px + (pw - width(&prompt.title)) / 2.0).floor();
            batch.text(
                atlas,
                &prompt.title,
                tx,
                py + 6.0,
                rgba(255, 255, 255, 255),
                true,
            );
            let mut x = (px + (pw - line) / 2.0).floor();
            let y = py + 20.0;
            for (glyph, [r, g, b], what) in glyphs {
                // The button as printed on the pad, in its colour, on a dark key.
                let gw = width(glyph) + 4.0;
                batch.fill(atlas, x - 1.0, y - 2.0, gw, 11.0, rgba(40, 40, 40, 255));
                batch.text(atlas, glyph, x + 1.0, y, rgba(r, g, b, 255), false);
                x += gw + 4.0;
                batch.text(atlas, what, x, y, rgba(220, 220, 220, 255), false);
                x += width(what) + gap(what);
            }
        }
        // A notice nobody answers: at the top, under a joining controller's strip, for as long
        // as a toast shows (5 s, times the setting).
        let below_strip = matches!(&self.split_hud.prompt, Some(p) if p.ask == Ask::Join);
        let shown_for =
            Duration::from_secs_f64(5.0 * f64::from(self.settings.notification_display_time));
        let Some(notice) = &mut self.split_hud.notice else {
            return;
        };
        if notice.raised.elapsed() > shown_for {
            self.split_hud.notice = None;
            return;
        }
        let max = (w - 24.0).min(260.0);
        let lines = wrap(&notice.body, max, width);
        let inner = lines
            .iter()
            .map(|l| width(l))
            .fold(width(&notice.title), f32::max);
        let (pw, ph) = (inner.min(max) + 16.0, 20.0 + 10.0 * lines.len() as f32);
        let (px, py) = (
            ((w - pw) / 2.0).floor(),
            if below_strip { 56.0 } else { 18.0 },
        );
        panel(batch, atlas, [px, py, pw, ph]);
        let white = rgba(255, 255, 255, 255);
        let tx = (px + (pw - width(&notice.title)) / 2.0).floor();
        batch.text(atlas, &notice.title, tx, py + 6.0, white, true);
        for (k, line) in lines.iter().enumerate() {
            let lx = (px + (pw - width(line)) / 2.0).floor();
            let y = py + 18.0 + 10.0 * k as f32;
            batch.text(atlas, line, lx, y, rgba(200, 200, 200, 255), false);
        }
    }
}

impl<'a> Split<'a> {
    /// The join question on every player's view in the world (a strip at the top), or gone.
    pub(super) fn prompt_join(&mut self, prompt: Option<Prompt>) {
        let views = std::iter::once(&mut self.host.split_hud)
            .chain(self.guests.iter_mut().map(|g| &mut g.game.split_hud));
        for hud in views {
            match &prompt {
                Some(p) => {
                    if hud.prompt.as_ref().is_none_or(|q| q.ask == Ask::Join) {
                        hud.prompt = Some(p.clone());
                    }
                }
                None if hud.prompt.as_ref().is_some_and(|q| q.ask == Ask::Join) => {
                    hud.prompt = None
                }
                None => {}
            }
        }
    }

    /// Player `i`'s badge says `note` for a few seconds.
    pub(super) fn tag(&mut self, i: usize, note: &str) {
        let place = Some((note.to_owned(), Instant::now() + Duration::from_secs(3)));
        if let Some(game) = self.game_mut(i) {
            game.split_hud.place = place;
        }
    }
}

/// `text` in lines at most `max` wide (by `width`), broken between words.
fn wrap(text: &str, max: f32, width: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let longer = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if width(&longer) > max && !line.is_empty() {
            lines.push(std::mem::replace(&mut line, word.to_owned()));
        } else {
            line = longer;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
