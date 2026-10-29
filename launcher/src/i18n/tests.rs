use super::*;
use std::collections::BTreeSet;
use std::path::Path;

/// Every Rust file of the launcher's sources.
fn sources(dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            // Comments left out: they quote strings too.
            let code = std::fs::read_to_string(path).unwrap();
            let lines = code
                .lines()
                .map(|l| match l.trim_start().starts_with("//") {
                    true => "",
                    false => l,
                });
            out.push(lines.collect::<Vec<_>>().join("\n"));
        }
    }
}

/// The string literal at the start of `code` (just after its opening quote), unescaped the way
/// Rust does: `\"`, `\\`, `\n`, and a backslash at a line's end that drops the next line's
/// indent.
fn literal(code: &str) -> String {
    let mut out = String::new();
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('\n') => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                }
                Some(c) => out.push(c),
                None => break,
            },
            c => out.push(c),
        }
    }
    out
}

/// The English strings the code asks a translation for: `tr("…")` and `trf("…")`, and every
/// other string literal (some reach `tr` through a widget).
fn strings() -> (BTreeSet<String>, BTreeSet<String>) {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let (mut asked, mut all) = (BTreeSet::new(), BTreeSet::new());
    for file in &files {
        for (at, _) in file.match_indices('"') {
            let before = &file[..at];
            if before.ends_with('\\') || before.ends_with('\'') {
                continue;
            }
            let s = literal(&file[at + 1..]);
            // rustfmt puts a long string on the line after the call.
            let before = before.trim_end();
            let call = |name: &str| {
                before.ends_with(&format!("{name}(")) && {
                    let head = &before[..before.len() - name.len() - 1];
                    !head.ends_with(|c: char| c.is_alphanumeric() || c == '_')
                }
            };
            if call("tr") || call("trf") {
                asked.insert(s.clone());
            }
            all.insert(s);
        }
    }
    (asked, all)
}

fn holes(s: &str) -> BTreeSet<&str> {
    s.split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect()
}

#[test]
fn every_language_has_every_string_the_code_asks_for() {
    let (asked, all) = strings();
    assert!(asked.contains("Language"), "the scan finds tr(\"…\")");
    for language in Language::ALL
        .into_iter()
        .filter(|l| *l != Language::English)
    {
        let table: HashMap<String, String> = serde_json::from_str(language.source())
            .unwrap_or_else(|e| panic!("lang/{}.json: {e}", language.code()));
        let missing: Vec<_> = asked.iter().filter(|s| !table.contains_key(*s)).collect();
        assert!(
            missing.is_empty(),
            "lang/{}.json lacks {missing:#?}",
            language.code()
        );
        let unused: Vec<_> = table.keys().filter(|s| !all.contains(*s)).collect();
        assert!(
            unused.is_empty(),
            "lang/{}.json has strings the code no longer has: {unused:#?}",
            language.code()
        );
        for (en, said) in &table {
            assert_eq!(holes(en), holes(said), "{}: {en:?}", language.code());
            assert!(
                !said.trim().is_empty(),
                "{}: {en:?} is empty",
                language.code()
            );
        }
    }
}

#[test]
fn the_system_language_is_the_first_one_the_launcher_has() {
    assert_eq!(Language::pick(["fr-CH", "en-US"]), Language::French);
    assert_eq!(Language::pick(["de-CH", "ru_RU.UTF-8"]), Language::German);
    assert_eq!(Language::pick(["de-DE", "ja"]), Language::German);
    assert_eq!(Language::pick(["es-ES", "en-US"]), Language::Spanish);
    assert_eq!(Language::pick(["ja", "ko"]), Language::English);
    assert_eq!(Language::pick(["uk-UA", "ru-RU"]), Language::Ukrainian);
    assert_eq!(Language::parse("uk_UA.UTF-8"), Some(Language::Ukrainian));
    assert_eq!(Language::pick([]), Language::English);
    assert_eq!(Language::parse("FR"), Some(Language::French));
    assert_eq!(Language::parse("DE"), Some(Language::German));
    assert_eq!(Language::parse("ES"), Some(Language::Spanish));
    assert_eq!(Language::parse("german"), None);
    assert_eq!(serde_json::to_string(&Language::German).unwrap(), "\"de\"");
    assert_eq!(serde_json::to_string(&Language::Spanish).unwrap(), "\"es\"");
}

#[test]
fn a_missing_string_shows_in_english() {
    assert_eq!(tr_in(Language::French, "Settings"), "Paramètres");
    assert_eq!(tr_in(Language::Russian, "Settings"), "Настройки");
    assert_eq!(tr_in(Language::German, "Settings"), "Einstellungen");
    assert_eq!(tr_in(Language::Spanish, "Settings"), "Ajustes");
    assert_eq!(tr_in(Language::Ukrainian, "Settings"), "Налаштування");
    assert_eq!(tr_in(Language::English, "Settings"), "Settings");
    assert_eq!(tr_in(Language::French, "No such string"), "No such string");
}
