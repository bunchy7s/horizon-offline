//! What the player wants to see of other players' cosmetics: two lines of the game's
//! `horizon-extras.txt` (in the game's home, beside `options.txt`), so the launcher's Wardrobe and
//! the game's settings read and write the same switches. "Show my cosmetics" is not here: it is
//! the account's, kept by the cosmetics service.
use std::{fs, io, path::Path};

/// The game's extras file.
pub const FILE: &str = "horizon-extras.txt";
/// Other players' cosmetics: `shown`, `name_tags` or `hidden` (`true` and `false` from before).
pub const OTHERS: &str = "cosmeticsOthers";
/// Other players' auras and trails: `true` or `false` (the default).
pub const AURAS: &str = "cosmeticsAuras";

/// How much of other players' cosmetics this player sees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Others {
    #[default]
    Shown,
    /// Only their name tag styles: no models on their bodies.
    NameTags,
    /// None: others look like plain players, and the game never asks what they wear.
    Hidden,
}

impl Others {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "shown" | "true" => Some(Self::Shown),
            "name_tags" => Some(Self::NameTags),
            "hidden" | "false" => Some(Self::Hidden),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::NameTags => "name_tags",
            Self::Hidden => "hidden",
        }
    }

    /// The next one in the game's button: shown, name tags only, hidden.
    pub fn next(self) -> Self {
        match self {
            Self::Shown => Self::NameTags,
            Self::NameTags => Self::Hidden,
            Self::Hidden => Self::Shown,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    pub others: Others,
    pub auras: bool,
}

impl Prefs {
    /// The switches in an extras file's `text`; a missing or unknown line keeps its default.
    pub fn parse(text: &str) -> Self {
        let mut prefs = Self::default();
        for (key, value) in text.lines().filter_map(|l| l.split_once(':')) {
            match key.trim() {
                OTHERS => prefs.others = Others::parse(value).unwrap_or(prefs.others),
                AURAS => prefs.auras = value.trim() == "true",
                _ => {}
            }
        }
        prefs
    }

    /// The switches of the game whose home is `home`.
    pub fn load(home: &Path) -> Self {
        fs::read_to_string(home.join(FILE)).map_or_else(|_| Self::default(), |t| Self::parse(&t))
    }

    /// `text` with these two lines set, every other line kept as it was.
    pub fn write_into(&self, text: &str) -> String {
        let mut out: String = text
            .lines()
            .filter(|l| {
                l.split_once(':')
                    .is_none_or(|(k, _)| ![OTHERS, AURAS].contains(&k.trim()))
            })
            .flat_map(|l| [l, "\n"])
            .collect();
        out.push_str(&format!(
            "{OTHERS}:{}\n{AURAS}:{}\n",
            self.others.name(),
            self.auras
        ));
        out
    }

    /// Saves these switches for the game in `home`, keeping the file's other settings.
    pub fn save(&self, home: &Path) -> io::Result<()> {
        let path = home.join(FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        fs::create_dir_all(home)?;
        fs::write(path, self.write_into(&text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_old_switch_and_keeps_the_other_settings() {
        assert_eq!(Prefs::parse(""), Prefs::default());
        assert!(!Prefs::default().auras);
        // The game's earlier on/off line still means what it meant.
        assert_eq!(Prefs::parse("cosmeticsOthers:false").others, Others::Hidden);
        assert_eq!(Prefs::parse("cosmeticsOthers:true").others, Others::Shown);
        let text = "zoom:true\ncosmeticsOthers:false\nnoAi:true\n";
        let prefs = Prefs {
            others: Others::NameTags,
            auras: true,
        };
        let written = prefs.write_into(text);
        assert_eq!(
            written,
            "zoom:true\nnoAi:true\ncosmeticsOthers:name_tags\ncosmeticsAuras:true\n"
        );
        assert_eq!(Prefs::parse(&written), prefs);
    }

    #[test]
    fn the_game_button_cycles_through_the_three() {
        let mut o = Others::Shown;
        let seen: Vec<Others> = (0..3)
            .map(|_| {
                o = o.next();
                o
            })
            .collect();
        assert_eq!(seen, [Others::NameTags, Others::Hidden, Others::Shown]);
    }
}
