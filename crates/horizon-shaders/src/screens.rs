//! How a pack lays out its options for the player (8.2, 8.3): `screen` and `screen.NAME` list
//! what each screen shows, `sliders` the options shown as sliders, `profile.NAME` sets of values
//! picked together.

use std::collections::{BTreeMap, BTreeSet};

use crate::{PackOption, Properties};

/// One place on a screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Option(String),
    /// A button opening `screen.NAME`.
    Screen(String),
    /// The button cycling through the profiles.
    Profile,
    Empty,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screens {
    /// By name, `""` for the main screen: its items and its number of columns.
    screens: BTreeMap<String, (Vec<Item>, u32)>,
    pub sliders: BTreeSet<String>,
    /// In the pack's order: the name and the values it sets.
    pub profiles: Vec<(String, Vec<(String, String)>)>,
}

impl Screens {
    pub fn parse(properties: &Properties, options: &BTreeMap<String, PackOption>) -> Self {
        let mut lists: BTreeMap<String, String> = BTreeMap::new();
        let mut columns: BTreeMap<String, u32> = BTreeMap::new();
        let mut profiles: Vec<(String, String)> = Vec::new();
        let mut sliders = BTreeSet::new();
        for entry in &properties.entries {
            let key = entry.key.as_str();
            if key == "sliders" {
                sliders.extend(entry.value.split_whitespace().map(str::to_owned));
            } else if let Some(name) = key.strip_prefix("profile.") {
                match profiles.iter_mut().find(|(n, _)| n == name) {
                    Some(p) => p.1 = entry.value.clone(),
                    None => profiles.push((name.to_owned(), entry.value.clone())),
                }
            } else if key == "screen" {
                lists.insert(String::new(), entry.value.clone());
            } else if key == "screen.columns" {
                columns.insert(String::new(), entry.value.trim().parse().unwrap_or(2));
            } else if let Some(rest) = key.strip_prefix("screen.") {
                match rest.strip_suffix(".columns") {
                    Some(name) => {
                        columns.insert(name.to_owned(), entry.value.trim().parse().unwrap_or(2));
                    }
                    None => _ = lists.insert(rest.to_owned(), entry.value.clone()),
                }
            }
        }
        let item = |word: &str| match word {
            "<empty>" => Item::Empty,
            "<profile>" => Item::Profile,
            w if w.starts_with('[') && w.ends_with(']') => {
                Item::Screen(w[1..w.len() - 1].to_owned())
            }
            w => Item::Option(w.to_owned()),
        };
        let placed: BTreeSet<&str> = lists.values().flat_map(|l| l.split_whitespace()).collect();
        // `*`: every option no screen names.
        let rest: Vec<Item> = options
            .values()
            .filter(|o| o.tested && !placed.contains(o.name.as_str()))
            .map(|o| Item::Option(o.name.clone()))
            .collect();
        let mut screens: BTreeMap<String, (Vec<Item>, u32)> = lists
            .iter()
            .map(|(name, list)| {
                let items = list
                    .split_whitespace()
                    .flat_map(|w| match w {
                        "*" => rest.clone(),
                        w => vec![item(w)],
                    })
                    .filter(|i| !matches!(i, Item::Option(o) if !options.contains_key(o)))
                    .collect();
                (
                    name.clone(),
                    (items, columns.get(name).copied().unwrap_or(2)),
                )
            })
            .collect();
        if !screens.contains_key("") {
            let all = options
                .values()
                .filter(|o| o.tested)
                .map(|o| Item::Option(o.name.clone()))
                .collect();
            screens.insert(String::new(), (all, columns.get("").copied().unwrap_or(2)));
        }
        let texts: BTreeMap<&str, &str> = profiles
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        // A profile naming none of the pack's options sets nothing: it would match any values.
        let profiles = profiles
            .iter()
            .map(|(name, _)| (name.clone(), profile_values(name, &texts, options)))
            .filter(|(_, values)| !values.is_empty())
            .collect();
        Self {
            screens,
            sliders,
            profiles,
        }
    }

    /// The items of screen `name` (`""`: the main screen) and its columns.
    pub fn screen(&self, name: &str) -> Option<(&[Item], u32)> {
        self.screens.get(name).map(|(i, c)| (i.as_slice(), *c))
    }

    /// The profile whose values are all those of `value`.
    pub fn current_profile(&self, value: impl Fn(&str) -> Option<String>) -> Option<&str> {
        self.profiles
            .iter()
            .find(|(_, values)| values.iter().all(|(n, v)| value(n).as_deref() == Some(v)))
            .map(|(name, _)| name.as_str())
    }
}

/// The values profile `name` sets: `NAME=VALUE` or `NAME:VALUE`, `NAME` and `!NAME` for a
/// switch, `profile.OTHER` for another profile's values.
fn profile_values(
    name: &str,
    texts: &BTreeMap<&str, &str>,
    options: &BTreeMap<String, PackOption>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut pending = vec![name];
    while let Some(profile) = pending.pop() {
        if !seen.insert(profile) {
            continue;
        }
        for word in texts.get(profile).copied().unwrap_or("").split_whitespace() {
            if let Some(other) = word.strip_prefix("profile.") {
                pending.push(other);
                continue;
            }
            let (option, value) = match word.split_once(['=', ':']) {
                Some((n, v)) => (n, v.to_owned()),
                None => match word.strip_prefix('!') {
                    Some(n) => (n, "false".to_owned()),
                    None => (word, "true".to_owned()),
                },
            };
            if !options.contains_key(option) || out.iter().any(|(n, _)| n == option) {
                continue;
            }
            out.push((option.to_owned(), value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Macros;

    fn option(name: &str, default: &str, switch: bool) -> (String, PackOption) {
        let values = if switch {
            vec!["false".into(), "true".into()]
        } else {
            vec!["1".into(), "2".into()]
        };
        let o = PackOption {
            name: name.into(),
            default: default.into(),
            values,
            switch,
            tested: true,
        };
        (name.into(), o)
    }

    #[test]
    fn screens_profiles_and_the_rest() {
        let options = BTreeMap::from([
            option("AO", "true", true),
            option("BLOOM", "false", true),
            option("LEVEL", "2", false),
            option("SPARE", "1", false),
        ]);
        let text = "screen = <profile> <empty> [post] LEVEL GONE\nscreen.post = BLOOM *\n\
                    screen.post.columns = 1\nsliders = LEVEL\n\
                    profile.empty = GONE\nprofile.low = !AO LEVEL=1\nprofile.high = profile.low AO BLOOM LEVEL:2\n";
        let screens = Screens::parse(&Properties::parse(text, &Macros::standard()), &options);
        let (main, columns) = screens.screen("").unwrap();
        assert_eq!(columns, 2);
        assert_eq!(
            main,
            [
                Item::Profile,
                Item::Empty,
                Item::Screen("post".into()),
                Item::Option("LEVEL".into())
            ]
        );
        let (post, columns) = screens.screen("post").unwrap();
        assert_eq!(columns, 1);
        assert_eq!(
            post,
            [
                Item::Option("BLOOM".into()),
                Item::Option("AO".into()),
                Item::Option("SPARE".into())
            ]
        );
        assert!(screens.sliders.contains("LEVEL"));
        assert_eq!(
            screens.profiles[1].1,
            [
                ("AO".to_owned(), "true".to_owned()),
                ("BLOOM".to_owned(), "true".to_owned()),
                ("LEVEL".to_owned(), "2".to_owned())
            ]
        );
        let values = BTreeMap::from([("AO", "false"), ("LEVEL", "1"), ("BLOOM", "true")]);
        assert_eq!(
            screens.current_profile(|n| values.get(n).map(|v| (*v).to_owned())),
            Some("low")
        );
    }
}
