//! The Shader Options page of a `shaders/` pack: its options on the screens the pack lays out,
//! with the names its `lang` files give them, and the values the player picked.

use std::collections::BTreeMap;

use horizon_shaders::{Lang, PackOption, ScreenItem, Screens};

use crate::lang;
use crate::text::Text;

#[derive(Clone, Debug)]
pub struct ShaderOptions {
    /// The pack's file name in `shaderpacks/`.
    pub pack: String,
    pub options: BTreeMap<String, PackOption>,
    pub screens: Screens,
    pub lang: Lang,
    /// The values that differ from the defaults.
    pub chosen: BTreeMap<String, String>,
    /// The screens opened from the main one, innermost last.
    pub path: Vec<String>,
    /// Something changed since the page opened.
    pub changed: bool,
}

impl ShaderOptions {
    pub fn new(
        pack: String,
        options: BTreeMap<String, PackOption>,
        screens: Screens,
        lang: Lang,
        chosen: BTreeMap<String, String>,
    ) -> Self {
        let mut out = Self {
            pack,
            options,
            screens,
            lang,
            chosen: BTreeMap::new(),
            path: Vec::new(),
            changed: false,
        };
        for (name, value) in chosen {
            out.set(&name, value);
        }
        out.changed = false;
        out
    }

    /// The screen shown: the innermost opened, else the main one.
    pub fn screen(&self) -> &str {
        self.path.last().map_or("", String::as_str)
    }

    pub fn items(&self) -> &[ScreenItem] {
        self.screens
            .screen(self.screen())
            .map_or(&[], |(items, _)| items)
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.chosen
            .get(name)
            .or_else(|| self.options.get(name).map(|o| &o.default))
            .map(String::as_str)
    }

    fn set(&mut self, name: &str, value: String) {
        let Some(option) = self.options.get(name).filter(|o| o.values.contains(&value)) else {
            return;
        };
        if value == option.default {
            self.chosen.remove(name);
        } else {
            self.chosen.insert(name.to_owned(), value);
        }
        self.changed = true;
    }

    /// The option's next value (the previous one with `back`), wrapping around.
    pub fn cycle(&mut self, name: &str, back: bool) {
        let (Some(option), Some(value)) = (self.options.get(name), self.value(name)) else {
            return;
        };
        let n = option.values.len();
        let at = option.values.iter().position(|v| v == value);
        let next = match (at, back) {
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
            (None, _) => 0,
        };
        let value = option.values[next].clone();
        self.set(name, value);
    }

    /// Where the slider of option `name` stands (its value's place in its list, 0 to 1), when
    /// the pack shows it as one.
    pub fn slider_position(&self, name: &str) -> Option<f32> {
        let option = self.options.get(name)?;
        if option.switch || option.values.len() < 2 || !self.screens.sliders.contains(name) {
            return None;
        }
        let at = option
            .values
            .iter()
            .position(|v| Some(v.as_str()) == self.value(name))?;
        Some(at as f32 / (option.values.len() - 1) as f32)
    }

    /// Option `name` at the value nearest place `position` (0 to 1) of its list.
    pub fn set_position(&mut self, name: &str, position: f32) {
        let Some(option) = self.options.get(name) else {
            return;
        };
        let last = option.values.len().saturating_sub(1);
        let at = (position.clamp(0.0, 1.0) * last as f32).round() as usize;
        if let Some(value) = option.values.get(at).cloned()
            && Some(value.as_str()) != self.value(name)
        {
            self.set(name, value);
        }
    }

    /// Option `name` back at the pack's default.
    pub fn reset_option(&mut self, name: &str) {
        if self.chosen.remove(name).is_some() {
            self.changed = true;
        }
    }

    /// The profile after the current one (the first when none matches) with its values.
    pub fn next_profile(&mut self) {
        let profiles = &self.screens.profiles;
        if profiles.is_empty() {
            return;
        }
        let current = self.current_profile().map(str::to_owned);
        let at = profiles
            .iter()
            .position(|(n, _)| Some(n) == current.as_ref());
        let (_, values) = profiles[at.map_or(0, |i| (i + 1) % profiles.len())].clone();
        for (name, value) in values {
            self.set(&name, value);
        }
    }

    pub fn current_profile(&self) -> Option<&str> {
        self.screens
            .current_profile(|n| self.value(n).map(str::to_owned))
    }

    pub fn reset(&mut self) {
        if !self.chosen.is_empty() {
            self.chosen.clear();
            self.changed = true;
        }
    }

    fn named(&self, key: &str, or: &str) -> Text {
        Text::legacy(self.lang.get(key).unwrap_or(or))
    }

    /// The screen's title: its name in the pack's language, or its key.
    pub fn title(&self) -> Text {
        match self.screen() {
            "" => Text::literal(self.pack.clone()),
            screen => self.named(&format!("screen.{screen}"), screen),
        }
    }

    pub fn screen_label(&self, screen: &str) -> Text {
        self.named(&format!("screen.{screen}"), screen)
            .append("...")
    }

    pub fn profile_label(&self) -> Text {
        let value = match self.current_profile() {
            Some(name) => self.named(&format!("profile.{name}"), name),
            None => Text::literal(lang::get("generator.custom")),
        };
        Text::literal(format!("{}: ", lang::get("horizon.shader_options.profile"))).append(value)
    }

    /// What the pack says of an option, a screen or the profiles (`option.NAME.comment`...).
    pub fn comment(&self, key: &str) -> Option<Text> {
        self.lang
            .get(&format!("{key}.comment"))
            .map(|c| Text::legacy(&c.replace("\\n", "\n")))
    }

    /// `option_label` within `width` as `measure` sizes it: the name shortened (`Bloom Up...: 2`)
    /// when the whole doesn't fit, and whether it was.
    pub fn option_label_within(
        &self,
        name: &str,
        width: f32,
        measure: impl Fn(&Text) -> f32,
    ) -> (Text, bool) {
        let full = self.option_label(name);
        if measure(&full) <= width {
            return (full, false);
        }
        // The pack's text, cut a character at a time with the `§` codes it holds kept.
        let key = format!("option.{name}");
        let caption = self.lang.get(&key).unwrap_or(name);
        let value = self.value_text(name);
        let mut chars: Vec<char> = caption.chars().collect();
        loop {
            while chars.len() >= 2 && chars[chars.len() - 2] == '§' {
                chars.truncate(chars.len() - 2);
            }
            if chars.pop().is_none() {
                return (full, true);
            }
            // Without the spaces and codes it ends with.
            let mut end = chars.len();
            loop {
                if end >= 2 && chars[end - 2] == '§' {
                    end -= 2;
                } else if end > 0 && chars[end - 1].is_whitespace() {
                    end -= 1;
                } else {
                    break;
                }
            }
            let cut: String = chars[..end].iter().collect();
            let label = Text::legacy(&format!("{cut}..."))
                .append(": ")
                .append(value.clone());
            if measure(&label) <= width {
                return (label, true);
            }
        }
    }

    /// `Name: value`, in the pack's words, the value yellow when it is not the default.
    pub fn option_label(&self, name: &str) -> Text {
        self.named(&format!("option.{name}"), name)
            .append(": ")
            .append(self.value_text(name))
    }

    /// The value of option `name` as the pack names it, yellow when it is not the default.
    fn value_text(&self, name: &str) -> Text {
        let Some(option) = self.options.get(name) else {
            return Text::literal(String::new());
        };
        let value = self.value(name).unwrap_or_default();
        let shown = if option.switch {
            Text::literal(lang::get(if value == "true" {
                "options.on"
            } else {
                "options.off"
            }))
        } else {
            let text = self
                .lang
                .get(&format!("value.{name}.{value}"))
                .unwrap_or(value);
            let prefix = self.lang.get(&format!("prefix.{name}")).unwrap_or("");
            let suffix = self.lang.get(&format!("suffix.{name}")).unwrap_or("");
            Text::legacy(&format!("{prefix}{text}{suffix}"))
        };
        if self.chosen.contains_key(name) {
            shown.with_color(crate::text::YELLOW)
        } else {
            shown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_shaders::{Macros, Properties};

    fn options() -> ShaderOptions {
        let switch = |name: &str, default: &str| PackOption {
            name: name.into(),
            default: default.into(),
            values: vec!["false".into(), "true".into()],
            switch: true,
            tested: true,
        };
        let options = BTreeMap::from([
            ("AO".to_owned(), switch("AO", "true")),
            (
                "LEVEL".to_owned(),
                PackOption {
                    name: "LEVEL".into(),
                    default: "2".into(),
                    values: vec!["1".into(), "2".into(), "3".into()],
                    switch: false,
                    tested: true,
                },
            ),
        ]);
        let text = "screen = <profile> [more]\nscreen.more = AO LEVEL\nsliders = LEVEL AO\n\
                    profile.gone = OLD=1\n\
                    profile.low = !AO LEVEL=1\nprofile.high = AO LEVEL=3\n";
        let screens = Screens::parse(&Properties::parse(text, &Macros::standard()), &options);
        ShaderOptions::new(
            "P.zip".into(),
            options,
            screens,
            Lang::default(),
            BTreeMap::from([
                ("LEVEL".to_owned(), "2".to_owned()),
                ("AO".to_owned(), "maybe".to_owned()),
            ]),
        )
    }

    #[test]
    fn values_cycle_and_defaults_are_not_kept() {
        let mut o = options();
        assert!(o.chosen.is_empty() && !o.changed);
        o.cycle("LEVEL", false);
        o.cycle("LEVEL", false);
        assert_eq!(o.value("LEVEL"), Some("1"));
        o.cycle("LEVEL", true);
        o.cycle("AO", false);
        assert_eq!(
            o.chosen,
            BTreeMap::from([
                ("AO".to_owned(), "false".to_owned()),
                ("LEVEL".to_owned(), "3".to_owned())
            ])
        );
        assert!(o.changed);
        o.reset();
        assert!(o.chosen.is_empty());
    }

    #[test]
    fn sliders_take_the_nearest_value() {
        let mut o = options();
        assert_eq!(o.slider_position("LEVEL"), Some(0.5));
        // A switch is a button even when `sliders` names it.
        assert_eq!(o.slider_position("AO"), None);
        o.set_position("LEVEL", 0.9);
        assert_eq!(o.value("LEVEL"), Some("3"));
        o.set_position("LEVEL", -1.0);
        assert_eq!(o.value("LEVEL"), Some("1"));
        o.set_position("LEVEL", 0.4);
        assert_eq!(o.value("LEVEL"), Some("2"));
        assert!(o.chosen.is_empty());
    }

    #[test]
    fn single_resets_comments_and_fitted_labels() {
        let mut o = options();
        o.lang = Lang::parse(
            "option.LEVEL=Level of detail\noption.LEVEL.comment=How much detail.\\nMore costs more.\n",
        );
        o.cycle("LEVEL", false);
        assert_eq!(o.value("LEVEL"), Some("3"));
        o.reset_option("LEVEL");
        assert_eq!(o.value("LEVEL"), Some("2"));
        assert_eq!(
            o.comment("option.LEVEL").map(|c| c.plain()).as_deref(),
            Some("How much detail.\nMore costs more.")
        );
        assert!(o.comment("option.AO").is_none());
        // One unit a character.
        let measure = |t: &Text| t.plain().chars().count() as f32;
        assert!(!o.option_label_within("LEVEL", 40.0, measure).1);
        let (fitted, cut) = o.option_label_within("LEVEL", 12.0, measure);
        assert!(cut);
        assert_eq!(fitted.plain(), "Level...: 2");
        o.lang = Lang::parse("option.LEVEL=§aLevel §lof detail§r\n");
        let (fitted, _) = o.option_label_within("LEVEL", 12.0, measure);
        assert_eq!(fitted.plain(), "Level...: 2");
        assert_eq!(fitted.spans[0].style, Text::legacy("§aL").spans[0].style);
        let (fitted, _) = o.option_label_within("LEVEL", 14.0, measure);
        assert_eq!(fitted.plain(), "Level of...: 2");
    }

    #[test]
    fn profiles_follow_the_values() {
        let mut o = options();
        assert_eq!(o.current_profile(), None);
        o.next_profile();
        assert_eq!(o.current_profile(), Some("low"));
        o.next_profile();
        assert_eq!(o.current_profile(), Some("high"));
        assert_eq!(o.value("LEVEL"), Some("3"));
        o.next_profile();
        assert_eq!(o.current_profile(), Some("low"));
    }
}
