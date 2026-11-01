//! A pack's `lang/<locale>.lang` files (spec 8.4): `key=value` lines naming options, values,
//! screens and profiles.

use std::collections::HashMap;

use crate::Pack;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lang(HashMap<String, String>);

impl Lang {
    /// English with the player's locale over it, so a key the locale lacks still has a name.
    /// File names match ignoring case: packs ship `en_US.lang` for Minecraft's `en_us`.
    /// Strings keep their `§` formatting codes for the game's text renderer.
    pub fn load(pack: &Pack, locale: &str) -> Self {
        let files = pack.list("lang");
        let mut lang = Self::default();
        for wanted in ["en_us", locale] {
            let file = files.iter().find(|f| {
                f.strip_suffix(".lang")
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(wanted))
            });
            if let Some(text) = file.and_then(|f| pack.read_text(&format!("lang/{f}"))) {
                lang.read(&text);
            }
        }
        lang
    }

    /// The names one `.lang` file's text gives.
    pub fn parse(text: &str) -> Self {
        let mut lang = Self::default();
        lang.read(text);
        lang
    }

    fn read(&mut self, text: &str) {
        for line in text.lines().map(str::trim) {
            if line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                self.0
                    .insert(key.trim().to_owned(), value.trim().to_owned());
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::zip;

    #[test]
    fn lang_falls_back_by_case_then_english() {
        let pack = Pack::from_zip_bytes(
            "P.zip",
            zip(&[
                (
                    "shaders/lang/en_US.lang",
                    "# names\noption.A=Alpha\noption.B = §6Beta§r\n",
                ),
                ("shaders/lang/fr_FR.lang", "option.A=Alpha (fr)\n"),
            ]),
        )
        .unwrap();
        let fr = Lang::load(&pack, "fr_fr");
        assert_eq!(fr.get("option.A"), Some("Alpha (fr)"));
        assert_eq!(fr.get("option.B"), Some("§6Beta§r"));
        assert_eq!(fr.get("# names"), None);
        let de = Lang::load(&pack, "de_de");
        assert_eq!(de.get("option.A"), Some("Alpha"));
        let none = Pack::from_zip_bytes("Q.zip", zip(&[("shaders/final.fsh", "")])).unwrap();
        assert_eq!(Lang::load(&none, "en_us"), Lang::default());
    }
}
