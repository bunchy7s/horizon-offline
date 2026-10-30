//! The launcher's night look ("Night" in `docs/design-system.md`): a pixel night sky that turns to
//! a sunset at the bottom, plates over it only where text needs them, one violet for the main
//! action, focus and progress, and Bricolage Grotesque (SIL OFL 1.1, `fonts/OFL-BricolageGrotesque.txt`).
//! Light players get the same look at dawn ([`DAWN`]). The game's menus don't use it.
use crate::{step, Blocks, Step, Web};

/// The UI family (optical size 14: Regular, Medium, SemiBold, Bold) and the display family
/// (Bold at optical size 72, for page and instance names). Mono stays [`crate::MONO`].
pub const FONT: &str = "Bricolage Grotesque";
pub const DISPLAY_FONT: &str = "Bricolage Grotesque Display";

/// Instance names and page titles, in [`DISPLAY_FONT`].
pub const DISPLAY: Step = step(
    "night-display",
    56.,
    Web::Fluid(40., 72.),
    1.,
    -0.035,
    700,
    false,
);
/// Section and card heads.
pub const TITLE: Step = step("night-title", 20., Web::Fixed, 1.25, -0.01, 700, false);
/// Text, rows, buttons and navigation.
pub const UI: Step = step("night-ui", 14., Web::Fixed, 1.45, -0.005, 500, false);
/// One-line states and details under a name ("Ready · 26.3").
pub const CAPTION: Step = step("night-caption", 12., Web::Fixed, 1.4, 0., 400, false);
/// A screen uses two or three of these, plus [`crate::MICRO`] for paths and counts.
pub const TYPE: [Step; 4] = [DISPLAY, TITLE, UI, CAPTION];

/// Corners. A plate cuts each corner in two [`PIXEL_CORNER`] steps, like the sky's art, and its
/// 1 px stroke follows the steps. The main action (an accent fill: Play, Create, Install) cuts one
/// step. Other controls (buttons, fields, focus rings) round at [`RADIUS_CONTROL`]: at 1x a step
/// vanishes on a pale fill and reads as notches on a thin ring. Pills are [`crate::RADIUS_PILL`],
/// for chips only.
pub const PIXEL_CORNER: f32 = 4.;
pub const RADIUS_CONTROL: f32 = 8.;
/// The main action (Play) is taller than the other controls.
pub const PLAY: f32 = 56.;

/// How many of the sky's stops, from the top, can sit behind text without a plate: the night's
/// lower stops (the sunset) are too bright for light text, so text there always has a plate.
pub const UNCARDED_STOPS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Night {
    /// The sky from the top of the window to the horizon, drawn in bands.
    pub sky: [u32; 6],
    /// Behind everything before the sky is drawn, and the sidebar.
    pub page: u32,
    pub side: u32,
    /// Plates over the sky (rows, cards, dialogs), with their opacity, and a raised or selected
    /// one.
    pub card: u32,
    pub card_alpha: f32,
    pub card_hi: u32,
    pub card_hi_alpha: f32,
    /// The plates' 1 px inner stroke and the rules between rows: this colour at these opacities.
    pub stroke: u32,
    pub stroke_alpha: f32,
    pub rule_alpha: f32,
    pub ink: u32,
    pub ink_soft: u32,
    pub muted: u32,
    /// The one main action's fill and progress; its label is `on_accent`.
    pub accent: u32,
    /// The accent as text and as the focus ring, readable on every plate.
    pub accent_hi: u32,
    pub on_accent: u32,
    pub success: u32,
    pub warn: u32,
    pub danger: u32,
    pub dark: bool,
}

pub const NIGHT: Night = Night {
    sky: [0x07061a, 0x140f38, 0x2a1656, 0x5b2a78, 0xb0477d, 0xf08a5d],
    page: 0x0e0b26,
    side: 0x0a0820,
    card: 0x15112e,
    card_alpha: 0.84,
    card_hi: 0x231c4a,
    card_hi_alpha: 0.90,
    stroke: 0xb9acff,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0xf4f2ff,
    ink_soft: 0xcfc9ea,
    muted: 0xa59fc6,
    accent: 0x7656fa,
    accent_hi: 0xa08cff,
    on_accent: 0xffffff,
    success: 0x5fd39a,
    warn: 0xf5b45a,
    danger: 0xff8a78,
    dark: true,
};

/// The light appearance: the same sky at dawn, pale blue to peach, with the night's violet.
pub const DAWN: Night = Night {
    sky: [0xc9d3f2, 0xdcd8f3, 0xead5ec, 0xf5d3d8, 0xf9d6bd, 0xfbe4c6],
    page: 0xe9e6f6,
    side: 0xe1ddf1,
    card: 0xfdfcff,
    card_alpha: 0.86,
    card_hi: 0xffffff,
    card_hi_alpha: 0.94,
    stroke: 0x3b2f78,
    stroke_alpha: 0.12,
    rule_alpha: 0.10,
    ink: 0x1b1530,
    ink_soft: 0x3b3354,
    muted: 0x585071,
    accent: 0x5b3fd6,
    accent_hi: 0x5333c9,
    on_accent: 0xffffff,
    success: 0x136336,
    warn: 0x7a4f00,
    danger: 0xa3301d,
    dark: false,
};

/// `top` at `alpha` over `under`, as the GPU blends them.
pub fn over(top: u32, alpha: f32, under: u32) -> u32 {
    [16, 8, 0].iter().fold(0, |out, s| {
        let (a, b) = (((top >> s) & 255) as f32, ((under >> s) & 255) as f32);
        out | ((a * alpha + b * (1. - alpha)).round() as u32) << s
    })
}

impl Night {
    /// These colours in the parts of the Blocks look the launcher still draws until each screen is
    /// rebuilt: plates come out opaque over the page, with soft strokes instead of dark outlines.
    pub fn blocks(&self) -> Blocks {
        let shade = |a: f32, under: u32| over(if self.dark { 0 } else { self.stroke }, a, under);
        let panel = over(self.card, self.card_alpha, self.page);
        let raised = over(self.card_hi, self.card_hi_alpha, self.page);
        Blocks {
            page: self.page,
            side: self.side,
            panel,
            raised,
            button: over(self.stroke, 0.10, raised),
            button_bevel: over(self.stroke, 0.24, raised),
            well: if self.dark {
                shade(0.3, self.side)
            } else {
                over(self.card, 0.7, self.side)
            },
            bevel: over(self.stroke, self.stroke_alpha * 1.5, panel),
            lip: shade(if self.dark { 0.45 } else { 0.18 }, self.page),
            outline: shade(if self.dark { 0.6 } else { 0.28 }, self.page),
            rule: over(self.stroke, self.rule_alpha, panel),
            ink: self.ink,
            ink_soft: self.ink_soft,
            muted: self.muted,
            accent: self.accent_hi,
            face: self.accent,
            face_bevel: over(0xffffff, 0.22, self.accent),
            face_lip: over(0, 0.35, self.accent),
            on_face: self.on_accent,
            danger: self.danger,
            success: self.success,
            warn: self.warn,
            dark: self.dark,
        }
    }

    /// Every colour by its CSS name (`--night-<name>`); plates carry their opacity.
    pub fn css(&self) -> String {
        let rgba = |c: u32, a: f32| {
            format!(
                "rgba({}, {}, {}, {})",
                c >> 16 & 255,
                c >> 8 & 255,
                c & 255,
                (a * 100.).round() / 100.
            )
        };
        let mut lines: Vec<(String, String)> = self
            .sky
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("sky-{i}"), format!("#{c:06x}")))
            .collect();
        for (name, c) in [
            ("page", self.page),
            ("side", self.side),
            ("ink", self.ink),
            ("ink-soft", self.ink_soft),
            ("muted", self.muted),
            ("accent", self.accent),
            ("accent-hi", self.accent_hi),
            ("on-accent", self.on_accent),
            ("success", self.success),
            ("warn", self.warn),
            ("danger", self.danger),
        ] {
            lines.push((name.into(), format!("#{c:06x}")));
        }
        lines.push(("card".into(), rgba(self.card, self.card_alpha)));
        lines.push(("card-hi".into(), rgba(self.card_hi, self.card_hi_alpha)));
        lines.push(("stroke".into(), rgba(self.stroke, self.stroke_alpha)));
        lines.push(("rule".into(), rgba(self.stroke, self.rule_alpha)));
        lines
            .into_iter()
            .map(|(name, value)| format!("  --night-{name}: {value};\n"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contrast;

    /// Every text colour reaches WCAG AA (4.5:1) on both plates over every stop of the sky, and
    /// the sky's upper stops, the page and the sidebar hold the text drawn there without a plate.
    /// The focus ring is `accent_hi`, so it is held to text contrast too.
    #[test]
    fn night_text_meets_wcag_aa_over_the_sky() {
        for p in [NIGHT, DAWN] {
            let look = if p.dark { "night" } else { "dawn" };
            let texts = [
                ("ink", p.ink),
                ("ink_soft", p.ink_soft),
                ("muted", p.muted),
                ("accent_hi", p.accent_hi),
                ("success", p.success),
                ("warn", p.warn),
                ("danger", p.danger),
            ];
            for (i, &sky) in p.sky.iter().enumerate() {
                for (plate, top, alpha) in [
                    ("card", p.card, p.card_alpha),
                    ("card_hi", p.card_hi, p.card_hi_alpha),
                ] {
                    let bg = over(top, alpha, sky);
                    for (tn, fg) in texts {
                        let ratio = contrast(fg, bg);
                        assert!(
                            ratio >= 4.5,
                            "{look} {tn} on {plate} over sky {i}: {ratio:.2}"
                        );
                    }
                }
            }
            let bare = p.sky[..UNCARDED_STOPS]
                .iter()
                .enumerate()
                .map(|(i, &c)| (format!("sky {i}"), c))
                .chain([("page".into(), p.page), ("side".into(), p.side)]);
            for (bn, bg) in bare {
                for (tn, fg) in &texts[..4] {
                    let ratio = contrast(*fg, bg);
                    assert!(ratio >= 4.5, "{look} {tn} on {bn}: {ratio:.2}");
                }
            }
            let label = contrast(p.on_accent, p.accent);
            assert!(label >= 4.5, "{look} the main action's label: {label:.2}");
        }
    }

    #[test]
    fn over_blends_like_the_gpu() {
        assert_eq!(over(0xffffff, 1., 0x000000), 0xffffff);
        assert_eq!(over(0xffffff, 0.5, 0x000000), 0x808080);
        assert_eq!(over(0x123456, 0., 0xabcdef), 0xabcdef);
    }
}
