//! The launcher's glass look ("Glass" in `docs/design-system.md`), from Anthony S.'s mockup
//! (horizon-site `app/launcher/launcher.css` and `components/launcher/launcher-ui.tsx`): frosted
//! plates over the pixel sunset, white text at a few opacities, one accent. Geist sets the text,
//! Bricolage Grotesque the headings (SemiBold, at the optical size a browser picks for each).
//!
//! The mockup derives its colours with CSS `color-mix`: [`Glass::new`] does the same sums, so a
//! player's own accent comes out as it does on the website.

/// The mockup's defaults: accent, how much white the plates take, and their blur in px.
pub const ACCENT: u32 = 0x6b4bff;
pub const TINT: f32 = 0.06;
pub const BLUR: f32 = 24.;
/// Behind the scene before it is drawn.
pub const PAGE: u32 = 0x0a0919;
/// The dark text on a light accent, and on the white buttons.
pub const ON_LIGHT: u32 = 0x100a1f;
/// A running game or server, and a ready instance.
pub const LIVE: u32 = 0x4ade80;
/// Busy (pulsing) and partly-working states.
pub const WARN: u32 = 0xf2c46b;
/// Destructive menu rows (`#ff8a8a`, hovered over `#ff6b6b` at 12 %).
pub const DANGER: u32 = 0xff8a8a;
pub const DANGER_HOVER: u32 = 0xff6b6b;
/// The Discord button and its hover.
pub const DISCORD: u32 = 0x5865f2;
pub const DISCORD_HOVER: u32 = 0x4752c4;
/// The scrim under dialogs and the palette: this colour at 50 %.
pub const SCRIM: u32 = 0x07050f;

/// Text families: Geist for everything, Bricolage for page, instance and dialog names.
pub const FONT: &str = crate::FONT;
pub const DISPLAY_FONT: &str = "Bricolage Grotesque Display";
pub const HEADING_FONT: &str = "Bricolage Grotesque Heading";

/// Corners, in px. No step anywhere: everything is round.
pub mod radius {
    /// Dialogs.
    pub const MODAL: f32 = 28.;
    /// Panels (the sidebar, cards, the tabbed panel).
    pub const PANEL: f32 = 20.;
    /// Menus and popovers; choice cards in dialogs.
    pub const POPOVER: f32 = 16.;
    /// The Play group, the big icon buttons, the top bar's tab strip.
    pub const LARGE: f32 = 14.;
    /// Rows, chips, fields in dialogs.
    pub const ROW: f32 = 12.;
    /// Buttons, tabs, small fields, menu rows.
    pub const CONTROL: f32 = 10.;
    /// The sidebar rows' ⋯ button.
    pub const SMALL: f32 = 9.;
}

/// How opaque white text is in each role (the mockup's `text-white/NN`).
pub mod ink {
    pub const FULL: f32 = 1.;
    pub const STRONG: f32 = 0.8;
    pub const SOFT: f32 = 0.75;
    pub const BODY: f32 = 0.7;
    pub const MUTED: f32 = 0.65;
    pub const NOTE: f32 = 0.6;
    pub const QUIET: f32 = 0.55;
    pub const FAINT: f32 = 0.5;
    pub const DIM: f32 = 0.45;
    pub const PLACEHOLDER: f32 = 0.4;
    pub const OFF: f32 = 0.35;
}

/// How opaque white fills are (`bg-white/[0.NN]`): rows hovered, selected, buttons, fields.
pub mod fill {
    pub const HOVER: f32 = 0.05;
    pub const ROW_HOVER: f32 = 0.06;
    pub const FIELD: f32 = 0.07;
    pub const BUTTON: f32 = 0.08;
    pub const SELECTED: f32 = 0.1;
    pub const FIELD_FOCUS: f32 = 0.11;
    pub const ACTIVE: f32 = 0.12;
    pub const BUTTON_HOVER: f32 = 0.14;
}

/// The look's colours for one accent and glass setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glass {
    pub accent: u32,
    /// The accent mixed half with white: progress bars, ticks, the focus ring, links.
    pub accent_soft: u32,
    /// A dark violet from the accent: what the plates are tinted with.
    pub base: u32,
    /// Text on the accent: white, or [`ON_LIGHT`] on a light accent.
    pub on_accent: u32,
    /// A plate's fill and its opacity.
    pub plate: u32,
    pub plate_alpha: f32,
    pub tint: f32,
    pub blur: f32,
}

impl Glass {
    /// The mockup's sums: `--accent-soft: color-mix(in oklab, accent 52%, white)`,
    /// `--base: color-mix(in oklab, accent 10%, #14111d)`, and `.hzl-glass`'s background
    /// `color-mix(in srgb, white tint, color-mix(in oklab, base 22%, transparent))`.
    pub fn new(accent: u32, tint: f32, blur: f32) -> Glass {
        let base = mix_oklab(accent, 0x14111d, 0.10);
        // White at `tint` over the base at 22 %, mixed with premultiplied alpha as CSS does.
        let base_alpha = 0.22;
        let alpha = tint + (1. - tint) * base_alpha;
        let plate = channels(|s| {
            let premul = tint + (1. - tint) * base_alpha * (((base >> s) & 255) as f32 / 255.);
            premul / alpha
        });
        Glass {
            accent,
            accent_soft: mix_oklab(accent, 0xffffff, 0.52),
            base,
            on_accent: if luminance(accent) > 0.4 {
                ON_LIGHT
            } else {
                0xffffff
            },
            plate,
            plate_alpha: alpha,
            tint,
            blur,
        }
    }

    /// The mockup's defaults.
    pub fn default_look() -> Glass {
        Glass::new(ACCENT, TINT, BLUR)
    }

    /// Every colour by its CSS name (`--glass-<name>`), for `tokens.css`.
    pub fn css(&self) -> String {
        let hex = |c: u32| format!("#{c:06x}");
        [
            ("accent", hex(self.accent)),
            ("accent-soft", hex(self.accent_soft)),
            ("base", hex(self.base)),
            ("on-accent", hex(self.on_accent)),
            ("page", hex(PAGE)),
            ("live", hex(LIVE)),
            ("warn", hex(WARN)),
            ("danger", hex(DANGER)),
            (
                "plate",
                format!(
                    "rgba({}, {}, {}, {:.3})",
                    self.plate >> 16 & 255,
                    self.plate >> 8 & 255,
                    self.plate & 255,
                    self.plate_alpha
                ),
            ),
            ("blur", format!("{}px", self.blur)),
        ]
        .into_iter()
        .map(|(name, value)| format!("  --glass-{name}: {value};\n"))
        .collect()
    }
}

/// Builds a colour from a function of each channel's shift (16, 8, 0) giving 0..=1.
fn channels(f: impl Fn(u32) -> f32) -> u32 {
    [16, 8, 0].iter().fold(0, |out, &s| {
        out | ((f(s).clamp(0., 1.) * 255.).round() as u32) << s
    })
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    }
}

/// sRGB to OKLab (Björn Ottosson's matrices, as CSS Color 4 uses).
// The matrices as CSS Color 4 publishes them, digit for digit.
#[allow(clippy::excessive_precision)]
pub fn oklab(c: u32) -> [f32; 3] {
    let [r, g, b] = [16, 8, 0].map(|s| to_linear(((c >> s) & 255) as f32 / 255.));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

/// OKLab back to sRGB, clamped to the gamut.
// The matrices as CSS Color 4 publishes them, digit for digit.
#[allow(clippy::excessive_precision)]
pub fn from_oklab([l, a, b]: [f32; 3]) -> u32 {
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    let rgb = [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_,
    ];
    channels(|s| from_linear(rgb[(2 - s / 8) as usize]))
}

/// CSS `color-mix(in oklab, a p, b)`: `p` of `a`, the rest `b`.
pub fn mix_oklab(a: u32, b: u32, p: f32) -> u32 {
    let (x, y) = (oklab(a), oklab(b));
    from_oklab([0, 1, 2].map(|i| x[i] * p + y[i] * (1. - p)))
}

/// WCAG relative luminance, as the mockup's `luminance` picks the label colour on the accent.
pub fn luminance(c: u32) -> f32 {
    let [r, g, b] = [16, 8, 0].map(|s| {
        let v = ((c >> s) & 255) as f32 / 255.;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: u32, b: u32) -> bool {
        [16, 8, 0]
            .iter()
            .all(|s| (((a >> s) & 255) as i32 - ((b >> s) & 255) as i32).abs() <= 1)
    }

    /// Chrome's own results for the mockup's `color-mix` sums (getComputedStyle, then a 1 px
    /// canvas for the sRGB bytes): an independent check of the OKLab maths and the plate's mix.
    #[test]
    fn mixes_match_the_browser() {
        let g = Glass::default_look();
        let soft = oklab(g.accent_soft);
        let mixed = [0, 1, 2].map(|i| oklab(ACCENT)[i] * 0.52 + oklab(0xffffff)[i] * 0.48);
        for (i, want) in [0.772018, 0.0291158, -0.12693].into_iter().enumerate() {
            assert!(
                (mixed[i] - want).abs() < 1e-3,
                "accent-soft oklab {mixed:?}"
            );
        }
        // Chrome clips the out-of-gamut blue the same way.
        assert!(
            close(g.accent_soft, 0xa9a9ff),
            "accent-soft {:06x} {soft:?}",
            g.accent_soft
        );
        assert!(close(g.base, 0x1b1830), "base {:06x}", g.base);
        // color(srgb 0.30732 0.296469 0.370862 / 0.2668)
        assert!(close(g.plate, 0x4e4c5f), "plate {:06x}", g.plate);
        assert!((g.plate_alpha - 0.2668).abs() < 1e-4);
    }

    #[test]
    fn a_light_accent_takes_dark_labels() {
        assert_eq!(Glass::new(0xf2c46b, TINT, BLUR).on_accent, ON_LIGHT);
        assert_eq!(Glass::default_look().on_accent, 0xffffff);
    }

    #[test]
    fn oklab_round_trips() {
        for c in [0x000000, 0xffffff, 0x6b4bff, 0x14111d, 0x5865f2, 0xf89d62] {
            assert!(close(from_oklab(oklab(c)), c), "{c:06x}");
        }
    }
}
