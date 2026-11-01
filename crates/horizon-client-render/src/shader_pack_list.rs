//! The shader packs in the `shaderpacks` folder beside `options.txt`, as Options > Horizon
//! Settings > Shader Packs lists them: Horizon's own (docs/shaders.md) and those in the
//! `shaders/` format, side by side.

use std::path::{Path, PathBuf};

use horizon_shader_api::{OptionKind, OptionPage, PackOption};

pub const DIR: &str = "shaderpacks";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A folder with `horizon-shaders.toml`, or a `.hshader`.
    Horizon,
    /// A folder with a `shaders` folder, or a `.zip`.
    Classic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The folder's or file's name in `shaderpacks`: what the pick saves.
    pub file: String,
    /// The pack's own name for Horizon's packs, else `file`.
    pub title: String,
    pub kind: Kind,
    /// A Horizon pack's `[[options]]`, and the value of each the player set (`values_path`).
    pub options: Vec<PackOption>,
    pub values: Vec<f64>,
    /// Its options page's `[[pages]]`; empty for one page of every option.
    pub pages: Vec<OptionPage>,
}

impl Entry {
    /// Saves the player's option values, for the pack in the folder `dir`.
    pub fn save_values(&self, dir: &Path) {
        let path = dir.join(&self.file);
        if let Err(e) = save_values(&path, &self.options, &self.values) {
            log::warn!("shader pack options of {}: {e}", self.file);
        }
    }

    /// The option `i` one value on (`back`: one before), or at its default (`reset`). Whether
    /// the pack has that option.
    pub fn step_option(&mut self, i: usize, back: bool, reset: bool) -> bool {
        let (Some(o), Some(v)) = (self.options.get(i), self.values.get_mut(i)) else {
            return false;
        };
        *v = if reset {
            o.kind.default_constant()
        } else {
            step_value(&o.kind, *v, back)
        };
        true
    }

    /// Every option back to its default.
    pub fn reset_options(&mut self) {
        let defaults = self.options.iter().map(|o| o.kind.default_constant());
        self.values = defaults.collect();
    }
}

/// The packs in `dir`, Horizon's first, each kind by title. A missing folder has none.
pub fn scan(dir: &Path) -> Vec<Entry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut packs: Vec<Entry> = entries.flatten().filter_map(|e| entry(&e.path())).collect();
    packs.sort_by(|a, b| {
        (a.kind == Kind::Classic, a.title.to_lowercase())
            .cmp(&(b.kind == Kind::Classic, b.title.to_lowercase()))
    });
    packs
}

fn entry(path: &Path) -> Option<Entry> {
    let file = path.file_name()?.to_str()?.to_owned();
    if file.starts_with('.') {
        return None;
    }
    if horizon_shader_api::package::is_pack(path) {
        // Its manifest's name; a pack that can't be read is still listed, under its file
        // name: picking it says why in the chat.
        let manifest = if path.is_dir() {
            std::fs::read_to_string(path.join(horizon_shader_api::MANIFEST))
                .ok()
                .and_then(|t| horizon_shader_api::Manifest::parse(&t).ok())
        } else {
            horizon_shader_api::Pack::read(path)
                .ok()
                .map(|p| p.manifest)
        };
        let (title, options, pages) = manifest.map_or_else(
            || (file.clone(), Vec::new(), Vec::new()),
            |m| (m.name, m.options, m.pages),
        );
        let values = load_values(path, &options);
        return Some(Entry {
            file,
            title,
            kind: Kind::Horizon,
            options,
            values,
            pages,
        });
    }
    let zip = path.is_file()
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    (zip || path.join("shaders").is_dir()).then(|| Entry {
        title: file.clone(),
        file,
        kind: Kind::Classic,
        options: Vec::new(),
        values: Vec::new(),
        pages: Vec::new(),
    })
}

/// Where the option values of the Horizon pack at `pack` are kept: `<pack>.txt` beside it,
/// one `id=value` a line.
pub fn values_path(pack: &Path) -> PathBuf {
    let mut path = pack.as_os_str().to_owned();
    path.push(".txt");
    PathBuf::from(path)
}

/// The value of each of `options` saved for the pack at `pack`, else its default; a value
/// out of the option's range is brought into it.
pub fn load_values(pack: &Path, options: &[PackOption]) -> Vec<f64> {
    let text = std::fs::read_to_string(values_path(pack)).unwrap_or_default();
    let saved: std::collections::HashMap<&str, f64> = text
        .lines()
        .filter_map(|l| l.split_once('='))
        .filter_map(|(k, v)| Some((k.trim(), v.trim().parse().ok()?)))
        .collect();
    options
        .iter()
        .map(|o| {
            saved
                .get(o.id.as_str())
                .map_or_else(|| o.kind.default_constant(), |&v| clamp(&o.kind, v))
        })
        .collect()
}

/// Saves `values` of `options`; values at their defaults are left out, and with none left
/// the file goes.
fn save_values(pack: &Path, options: &[PackOption], values: &[f64]) -> std::io::Result<()> {
    let text: String = options
        .iter()
        .zip(values)
        .filter(|(o, v)| o.kind.default_constant() != **v)
        .map(|(o, v)| format!("{}={v}\n", o.id))
        .collect();
    let path = values_path(pack);
    if text.is_empty() {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    } else {
        std::fs::write(path, text)
    }
}

fn clamp(kind: &OptionKind, v: f64) -> f64 {
    match kind {
        OptionKind::Bool { .. } => f64::from(u8::from(v != 0.0)),
        OptionKind::Float { min, max, .. } => v.clamp(f64::from(*min), f64::from(*max)),
        OptionKind::Int { min, max, .. } => v.round().clamp(f64::from(*min), f64::from(*max)),
        OptionKind::Choice { values, .. } => v.round().clamp(0.0, values.len() as f64 - 1.0),
    }
}

/// The option's next value (`back`: the one before), wrapping around at the ends.
fn step_value(kind: &OptionKind, v: f64, back: bool) -> f64 {
    let dir = if back { -1.0 } else { 1.0 };
    match kind {
        OptionKind::Bool { .. } => f64::from(u8::from(v == 0.0)),
        OptionKind::Float { min, max, step, .. } => {
            let (min, max, step) = (f64::from(*min), f64::from(*max), f64::from(*step));
            if !back && v >= max - step * 0.01 {
                min
            } else if back && v <= min + step * 0.01 {
                max
            } else {
                // On the grid of steps from the minimum, so float error never piles up.
                let n = ((v - min) / step).round() + dir;
                (min + n * step).clamp(min, max)
            }
        }
        OptionKind::Int { min, max, .. } => {
            let (min, max) = (f64::from(*min), f64::from(*max));
            let n = v.round() + dir;
            if n > max {
                min
            } else if n < min {
                max
            } else {
                n
            }
        }
        OptionKind::Choice { values, .. } => {
            let n = values.len() as f64;
            (v.round() + dir).rem_euclid(n)
        }
    }
}

/// The value as the settings page shows it.
pub fn show_value(kind: &OptionKind, v: f64) -> String {
    match kind {
        OptionKind::Bool { .. } => crate::lang::get(if v != 0.0 {
            "options.on"
        } else {
            "options.off"
        }),
        OptionKind::Float { step, .. } => {
            let digits = if *step >= 1.0 {
                0
            } else if *step >= 0.1 {
                1
            } else {
                2
            };
            format!("{v:.digits$}")
        }
        OptionKind::Int { .. } => format!("{v:.0}"),
        OptionKind::Choice { values, .. } => {
            values.get(v.round() as usize).cloned().unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_both_kinds_horizon_first_and_skips_the_rest() {
        let dir = std::env::temp_dir().join(format!("horizon-shader-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.as_path();
        let manifest = "schema = 1\nid = \"warm\"\nname = \"Warm Light\"\nversion = \"1\"\n\
            api = \"0.1\"\n[[post]]\nfile = \"p.wgsl\"\n";
        std::fs::create_dir(d.join("warm")).unwrap();
        std::fs::write(d.join("warm").join(horizon_shader_api::MANIFEST), manifest).unwrap();
        std::fs::create_dir_all(d.join("Classic Folder/shaders")).unwrap();
        std::fs::write(d.join("archive.zip"), b"").unwrap();
        std::fs::write(d.join("notes.txt"), b"").unwrap();
        std::fs::create_dir(d.join("empty")).unwrap();
        std::fs::create_dir_all(d.join(".hidden/shaders")).unwrap();
        let packs = scan(d);
        let listed: Vec<(&str, &str, Kind)> = packs
            .iter()
            .map(|e| (e.file.as_str(), e.title.as_str(), e.kind))
            .collect();
        assert_eq!(
            listed,
            [
                ("warm", "Warm Light", Kind::Horizon),
                ("archive.zip", "archive.zip", Kind::Classic),
                ("Classic Folder", "Classic Folder", Kind::Classic),
            ]
        );
        assert!(scan(&d.join("missing")).is_empty());
        std::fs::remove_dir_all(d).unwrap();
    }

    fn option(id: &str, kind: OptionKind) -> PackOption {
        PackOption {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            kind,
        }
    }

    #[test]
    fn values_step_wrap_and_round_trip() {
        let float = OptionKind::Float {
            default: 1.0,
            min: 0.0,
            max: 2.0,
            step: 0.5,
        };
        assert_eq!(step_value(&float, 1.0, false), 1.5);
        assert_eq!(step_value(&float, 2.0, false), 0.0);
        assert_eq!(step_value(&float, 0.0, true), 2.0);
        assert_eq!(show_value(&float, 1.5), "1.5");
        let choice = OptionKind::Choice {
            values: vec!["a".into(), "b".into(), "c".into()],
            default: 0,
        };
        assert_eq!(step_value(&choice, 2.0, false), 0.0);
        assert_eq!(step_value(&choice, 0.0, true), 2.0);
        assert_eq!(show_value(&choice, 1.0), "b");
        let int = OptionKind::Int {
            default: 0,
            min: -1,
            max: 1,
        };
        assert_eq!(step_value(&int, 1.0, false), -1.0);
        let on = OptionKind::Bool { default: true };
        assert_eq!(step_value(&on, 1.0, false), 0.0);

        let dir =
            std::env::temp_dir().join(format!("horizon-shader-values-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pack = dir.join("p.hshader");
        let options = [option("a", float), option("b", choice)];
        // Only what differs from the defaults is written; out of range comes back clamped.
        save_values(&pack, &options, &[1.0, 2.0]).unwrap();
        let text = std::fs::read_to_string(values_path(&pack)).unwrap();
        assert_eq!(text, "b=2\n");
        std::fs::write(values_path(&pack), "a=9\nb=1\nzzz=4\n").unwrap();
        assert_eq!(load_values(&pack, &options), [2.0, 1.0]);
        save_values(&pack, &options, &[1.0, 0.0]).unwrap();
        assert!(!values_path(&pack).exists());
        assert_eq!(load_values(&pack, &options), [1.0, 0.0]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
