//! The launcher's languages: English (the strings in the code), French, Russian, German and
//! Spanish, the languages Horizon's own game menus have, Simplified Chinese, Ukrainian,
//! Polish, Brazilian Portuguese, Japanese and Turkish. A table per language (`launcher/lang/<code>.json`) maps each English string to its
//! translation; a string the table lacks shows in English.
//!
//! The language follows the system's unless the player picks one in Settings
//! (`Settings::language`). Translations are Horizon's own, written for this launcher.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "zh")]
    Chinese,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "uk")]
    Ukrainian,
    #[serde(rename = "pl")]
    Polish,
    #[serde(rename = "pt-br")]
    PortugueseBrazil,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "tr")]
    Turkish,
}

impl Language {
    pub const ALL: [Language; 11] = [
        Self::English,
        Self::French,
        Self::Russian,
        Self::German,
        Self::Chinese,
        Self::Spanish,
        Self::Ukrainian,
        Self::Polish,
        Self::PortugueseBrazil,
        Self::Japanese,
        Self::Turkish,
    ];

    /// Its code in `settings.json` and on the command line.
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::French => "fr",
            Self::Russian => "ru",
            Self::German => "de",
            Self::Chinese => "zh",
            Self::Spanish => "es",
            Self::Ukrainian => "uk",
            Self::Polish => "pl",
            Self::PortugueseBrazil => "pt-br",
            Self::Japanese => "ja",
            Self::Turkish => "tr",
        }
    }

    /// Its name in itself: what a player looks for in a list of languages.
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::French => "Français",
            Self::Russian => "Русский",
            Self::German => "Deutsch",
            Self::Chinese => "简体中文",
            Self::Spanish => "Español",
            Self::Ukrainian => "Українська",
            Self::Polish => "Polski",
            Self::PortugueseBrazil => "Português (Brasil)",
            Self::Japanese => "日本語",
            Self::Turkish => "Türkçe",
        }
    }

    /// Parses a language code (e.g. `fr`, `fr-CH`, `pt-BR`, or `pt_BR.UTF-8`)
    /// if the launcher supports it.
    ///
    /// Underscores are normalized to hyphens to support locale formats commonly
    /// returned by operating systems. Brazilian Portuguese (`pt-BR`) is matched
    /// explicitly, while other Portuguese variants (such as `pt-PT`) are not
    /// mapped to it.
    pub fn parse(code: &str) -> Option<Self> {
        let lower = code.trim().to_ascii_lowercase();
        let clean = lower.split(['.', '@']).next().unwrap_or("");
        let normalized = clean.replace('_', "-");

        if normalized == "pt-br" {
            return Some(Self::PortugueseBrazil);
        }

        let base = normalized.split('-').next().unwrap_or("");

        Self::ALL
            .into_iter()
            .find(|language| language.code() == base)
    }

    /// The first of the system's preferred languages the launcher has, else English.
    pub fn pick<'a>(preferred: impl IntoIterator<Item = &'a str>) -> Self {
        preferred
            .into_iter()
            .find_map(Self::parse)
            .unwrap_or_default()
    }

    /// The system's language, as far as the launcher has it.
    pub fn system() -> Self {
        let locales: Vec<String> = sys_locale::get_locales().collect();
        Self::pick(locales.iter().map(String::as_str))
    }

    /// The table, as shipped: English text to this language's.
    fn source(self) -> &'static str {
        match self {
            Self::English => "{}",
            Self::French => include_str!("../lang/fr.json"),
            Self::Russian => include_str!("../lang/ru.json"),
            Self::German => include_str!("../lang/de.json"),
            Self::Chinese => include_str!("../lang/zh.json"),
            Self::Spanish => include_str!("../lang/es.json"),
            Self::Ukrainian => include_str!("../lang/uk.json"),
            Self::Polish => include_str!("../lang/pl.json"),
            Self::PortugueseBrazil => include_str!("../lang/pt-br.json"),
            Self::Japanese => include_str!("../lang/ja.json"),
            Self::Turkish => include_str!("../lang/tr.json"),
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Shows the launcher in `language` from now on.
pub fn set(language: Language) {
    let i = Language::ALL
        .iter()
        .position(|l| *l == language)
        .unwrap_or(0);
    CURRENT.store(i as u8, Ordering::Relaxed);
}

pub fn current() -> Language {
    Language::ALL[CURRENT.load(Ordering::Relaxed) as usize % Language::ALL.len()]
}

type Table = HashMap<String, &'static str>;

fn table(language: Language) -> &'static Table {
    static TABLES: OnceLock<Vec<Table>> = OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        Language::ALL
            .iter()
            .map(|l| {
                serde_json::from_str::<HashMap<String, String>>(l.source())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(en, said)| (en, &*Box::leak(said.into_boxed_str())))
                    .collect()
            })
            .collect()
    });
    let i = Language::ALL
        .iter()
        .position(|l| *l == language)
        .unwrap_or(0);
    &tables[i]
}

/// `en` in `language`, or `en` itself where the table lacks it.
pub fn tr_in(language: Language, en: &str) -> &str {
    table(language).get(en).copied().unwrap_or(en)
}

/// `en` in the launcher's language.
pub fn tr(en: &str) -> &str {
    tr_in(current(), en)
}

/// `en` in the launcher's language with its `{name}` holes filled.
pub fn trf(en: &str, args: &[(&str, &str)]) -> String {
    let mut out = tr(en).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
#[path = "i18n/tests.rs"]
mod tests;
