//! The player's own look, on top of Dark, Light or System: an accent colour and a picture behind
//! Home.
//!
//! An accent is a hue. Horizon's violet is the default; another hue turns the violet tokens
//! (`horizon_theme::Blocks`' accent and the main action's face, bevel and lip) to it, keeping
//! their saturation and lightness, then darkens or lightens each just enough to read as well as
//! the violet does: white on the face at 4.5:1, the accent at 3:1 on every background.
use horizon_theme::{contrast, Blocks};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Violet,
    Blue,
    Teal,
    Green,
    Amber,
    Red,
    Pink,
}

impl Accent {
    pub const ALL: [Accent; 7] = [
        Self::Violet,
        Self::Blue,
        Self::Teal,
        Self::Green,
        Self::Amber,
        Self::Red,
        Self::Pink,
    ];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Violet => "violet",
            Self::Blue => "blue",
            Self::Teal => "teal",
            Self::Green => "green",
            Self::Amber => "amber",
            Self::Red => "red",
            Self::Pink => "pink",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let name = if name == "purple" || name == "default" {
            "violet"
        } else {
            &name
        };
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    /// Its hue in degrees; None for the violet, whose tokens stay as they are.
    fn hue(self) -> Option<f32> {
        match self {
            Self::Violet => None,
            Self::Blue => Some(212.),
            Self::Teal => Some(176.),
            Self::Green => Some(140.),
            Self::Amber => Some(36.),
            Self::Red => Some(356.),
            Self::Pink => Some(322.),
        }
    }

    /// `palette` with this accent.
    pub fn apply(self, mut p: Blocks) -> Blocks {
        let Some(hue) = self.hue() else {
            return p;
        };
        let turn = |c: u32| {
            let (_, s, l) = hsl(c);
            (hue, s, l)
        };
        // The face holds white text.
        let face = fit(turn(p.face), |c| contrast(p.on_face, c) >= 4.5, -1.);
        let (_, _, face_l) = hsl(face);
        let (h, s, l) = turn(p.face_bevel);
        p.face_bevel = from_hsl(h, s, l.max(face_l + 0.08).min(0.92));
        let (h, s, l) = turn(p.face_lip);
        p.face_lip = from_hsl(h, s, l.min(face_l - 0.12).max(0.05));
        p.face = face;
        // The accent marks things on every background: lighter on dark, darker on light.
        let grounds = [p.page, p.raised, p.button, p.panel, p.well];
        let reads = |c: u32| grounds.iter().all(|&g| contrast(c, g) >= 3.2);
        p.accent = fit(turn(p.accent), reads, if p.dark { 1. } else { -1. });
        p
    }
}

/// The colour's hue (degrees), saturation and lightness (0 to 1).
fn hsl(c: u32) -> (f32, f32, f32) {
    let [r, g, b] = [16, 8, 0].map(|s| ((c >> s) & 255) as f32 / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.;
    if max == min {
        return (0., 0., l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2. - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6. } else { 0. }
    } else if max == g {
        (b - r) / d + 2.
    } else {
        (r - g) / d + 4.
    };
    (h * 60., s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> u32 {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let x = c * (1. - ((h / 60.) % 2. - 1.).abs());
    let m = l - c / 2.;
    let (r, g, b) = match (h.rem_euclid(360.) / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let byte = |v: f32| ((v + m).clamp(0., 1.) * 255.).round() as u32;
    byte(r) << 16 | byte(g) << 8 | byte(b)
}

/// The colour, made lighter (`step` 1) or darker (-1) a little at a time until `good` holds.
fn fit((h, s, l): (f32, f32, f32), good: impl Fn(u32) -> bool, step: f32) -> u32 {
    let mut l = l;
    for _ in 0..100 {
        let c = from_hsl(h, s, l);
        if good(c) || !(0.0..=1.0).contains(&l) {
            return c;
        }
        l += step * 0.01;
    }
    from_hsl(h, s, l.clamp(0., 1.))
}

/// The pictures Home can show: what the decoders read.
const PICTURES: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
/// A picture above this is refused: a screen's worth is far less.
const MAX_BYTES: u64 = 40 << 20;

/// Keeps `file` as Home's background in `dir` (`background-<hash>.<ext>`: a new picture gets a
/// new name, so the window doesn't show the one it cached), after checking it is a picture, and
/// returns the kept file's name. An older background goes.
pub fn keep_background(dir: &Path, file: &Path) -> Result<String, String> {
    let shown = file.display();
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| PICTURES.contains(&e.as_str()))
        .ok_or_else(|| format!("{shown}: a PNG, JPEG or WebP picture, please."))?;
    let size = std::fs::metadata(file)
        .map_err(|e| format!("{shown}: {e}"))?
        .len();
    if size > MAX_BYTES {
        return Err(format!("{shown} is over 40 MB."));
    }
    let bytes = std::fs::read(file).map_err(|e| format!("{shown}: {e}"))?;
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .ok_or_else(|| format!("{shown} isn't a picture the launcher can read."))?;
    if w < 64 || h < 64 {
        return Err(format!("{shown} is {w}×{h}: too small to fill Home."));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let hash = crate::sources::sha1_hex(&bytes);
    let name = format!("background-{}.{ext}", &hash[..12]);
    std::fs::write(dir.join(&name), &bytes).map_err(|e| format!("{name}: {e}"))?;
    for old in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let other = old.file_name().to_string_lossy().into_owned();
        if other.starts_with("background-") && other != name {
            let _ = std::fs::remove_file(old.path());
        }
    }
    Ok(name)
}

/// Home's background, if the player keeps one and it is still there.
pub fn background(dir: &Path, name: Option<&str>) -> Option<PathBuf> {
    let name = name.filter(|n| n.starts_with("background-") && !n.contains(['/', '\\']))?;
    Some(dir.join(name)).filter(|p| p.is_file())
}

#[cfg(test)]
#[path = "themes/tests.rs"]
mod tests;
