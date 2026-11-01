use super::*;
use std::collections::BTreeSet;
use std::path::Path;

/// Every Rust file of the launcher's sources.
fn sources(dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(std::fs::read_to_string(path).unwrap());
        }
    }
}

/// Read Rust tokens rather than quote positions: whitespace, comments, escaped and raw strings
/// must not hide a call or turn a quoted example into one. Recurse into macro bodies too.
fn strings_in(
    tokens: proc_macro2::TokenStream,
    asked: &mut BTreeSet<String>,
    all: &mut BTreeSet<String>,
) {
    use proc_macro2::{Delimiter, TokenTree};
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Ident(name) if name == "tr" || name == "trf" => {
                if let Some(TokenTree::Group(args)) = tokens.peek() {
                    if args.delimiter() == Delimiter::Parenthesis {
                        if let Some(TokenTree::Literal(value)) = args.stream().into_iter().next() {
                            if let Ok(value) = syn::parse_str::<syn::LitStr>(&value.to_string()) {
                                asked.insert(value.value());
                            }
                        }
                    }
                }
            }
            TokenTree::Group(group) => strings_in(group.stream(), asked, all),
            TokenTree::Literal(value) => {
                if let Ok(value) = syn::parse_str::<syn::LitStr>(&value.to_string()) {
                    all.insert(value.value());
                }
            }
            _ => {}
        }
    }
}

/// The English strings requested directly, plus every literal (some reach `tr` via a widget or
/// constant). The tables' shared key set below covers those indirect translations.
fn strings() -> (BTreeSet<String>, BTreeSet<String>) {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let (mut asked, mut all) = (BTreeSet::new(), BTreeSet::new());
    for file in &files {
        strings_in(
            file.parse().expect("valid Rust tokens"),
            &mut asked,
            &mut all,
        );
    }
    (asked, all)
}

#[test]
fn the_scan_finds_multiline_calls_without_reading_comments_or_quoted_code() {
    let code = r###"
        tr
        (
            "multiline"
        );
        trf(/* before the string */ "hello {name}", &[("name", "player")]);
        some_macro!(crate::i18n::tr(r#"raw "quotes""#));
        tr("escaped \"quotes\" and \u{4e2d}\nnext");
        tr("continued \
            line");
        // tr("line comment");
        /* tr("block comment"); */
        let example = r#"tr("quoted code")"#;
        other_tr("not a translation");
        trf_other("not a translation either");
        let indirect = "widget label";
        tr(indirect);
    "###;
    let (mut asked, mut all) = (BTreeSet::new(), BTreeSet::new());
    strings_in(code.parse().unwrap(), &mut asked, &mut all);
    assert_eq!(
        asked,
        [
            "multiline",
            "hello {name}",
            "raw \"quotes\"",
            "escaped \"quotes\" and 中\nnext",
            "continued line",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
    assert!(all.contains("widget label"));
    assert!(asked.is_subset(&all));
    assert!(!all.contains("line comment"));
    assert!(!all.contains("block comment"));
}

fn holes(s: &str) -> BTreeSet<&str> {
    s.split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect()
}

#[test]
fn every_language_has_every_string_the_code_asks_for() {
    let (mut asked, all) = strings();
    assert!(asked.contains("Language"), "the scan finds tr(\"…\")");
    let tables: Vec<_> = Language::ALL
        .into_iter()
        .filter(|language| *language != Language::English)
        .map(|language| {
            let table: HashMap<String, String> = serde_json::from_str(language.source())
                .unwrap_or_else(|e| panic!("lang/{}.json: {e}", language.code()));
            (language, table)
        })
        .collect();
    // A label passed through a widget or a constant is not a literal argument to `tr`. Once
    // translated in any language it must exist in all of them, not silently fall back to English.
    for (_, table) in &tables {
        asked.extend(table.keys().cloned());
    }
    for (language, table) in &tables {
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
        for (en, said) in table {
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
    assert_eq!(Language::pick(["pl-PL", "en-US"]), Language::Polish);
    assert_eq!(Language::pick(["ja-JP", "en-US"]), Language::Japanese);
    assert_eq!(Language::parse("ja_JP.UTF-8"), Some(Language::Japanese));
    assert_eq!(Language::pick(["tr-TR", "en-US"]), Language::Turkish);
    assert_eq!(Language::parse("tr_TR.UTF-8"), Some(Language::Turkish));
    assert_eq!(Language::parse("pl_PL.UTF-8"), Some(Language::Polish));
    assert_eq!(Language::pick(["fr-CH", "en-US"]), Language::French);
    assert_eq!(Language::pick(["it-IT", "ru_RU.UTF-8"]), Language::Russian);
    assert_eq!(Language::pick(["it-IT", "zh-Hans-CN"]), Language::Chinese);
    assert_eq!(Language::pick(["it-IT", "ja"]), Language::Japanese);
    assert_eq!(Language::pick(["de-CH", "ru_RU.UTF-8"]), Language::German);
    assert_eq!(Language::pick(["de-DE", "ja"]), Language::German);
    assert_eq!(Language::pick(["de-DE", "zh-Hans-CN"]), Language::German);
    assert_eq!(Language::pick(["es-ES", "en-US"]), Language::Spanish);
    assert_eq!(
        Language::pick(["pt-BR", "en-US"]),
        Language::PortugueseBrazil
    );
    assert_eq!(Language::pick(["ko", "it-IT"]), Language::English);
    assert_eq!(Language::pick(["uk-UA", "ru-RU"]), Language::Ukrainian);
    assert_eq!(Language::parse("uk_UA.UTF-8"), Some(Language::Ukrainian));
    assert_eq!(
        Language::parse("pt_BR.UTF-8"),
        Some(Language::PortugueseBrazil)
    );
    assert_eq!(Language::parse("pt-BR"), Some(Language::PortugueseBrazil));
    assert_eq!(Language::parse("pt-PT"), None); // Explicit check: pt-PT won't alias pt-BR
    assert_eq!(Language::pick([]), Language::English);
    assert_eq!(Language::parse("FR"), Some(Language::French));
    assert_eq!(Language::parse("french"), None);
    assert_eq!(Language::parse("DE"), Some(Language::German));
    assert_eq!(Language::parse("ES"), Some(Language::Spanish));
    assert_eq!(Language::parse("german"), None);
    assert_eq!(Language::parse("ZH"), Some(Language::Chinese));
    assert_eq!(serde_json::to_string(&Language::German).unwrap(), "\"de\"");
    assert_eq!(serde_json::to_string(&Language::Chinese).unwrap(), "\"zh\"");
    assert_eq!(serde_json::to_string(&Language::Russian).unwrap(), "\"ru\"");
    assert_eq!(serde_json::to_string(&Language::Spanish).unwrap(), "\"es\"");
    assert_eq!(
        serde_json::to_string(&Language::PortugueseBrazil).unwrap(),
        "\"pt-br\""
    );
}

#[test]
fn a_missing_string_shows_in_english() {
    assert_eq!(tr_in(Language::Polish, "Settings"), "Ustawienia");
    assert_eq!(tr_in(Language::Japanese, "Settings"), "設定");
    assert_eq!(tr_in(Language::Turkish, "Settings"), "Ayarlar");
    assert_eq!(tr_in(Language::French, "Settings"), "Paramètres");
    assert_eq!(tr_in(Language::Russian, "Settings"), "Настройки");
    assert_eq!(tr_in(Language::German, "Settings"), "Einstellungen");
    assert_eq!(tr_in(Language::Chinese, "Settings"), "设置");
    assert_eq!(tr_in(Language::Spanish, "Settings"), "Ajustes");
    assert_eq!(tr_in(Language::Ukrainian, "Settings"), "Налаштування");
    assert_eq!(tr_in(Language::English, "Settings"), "Settings");
    assert_eq!(tr_in(Language::French, "No such string"), "No such string");
    assert_eq!(
        tr_in(Language::PortugueseBrazil, "Settings"),
        "Configurações"
    );
}

#[test]
fn polish_entries_are_valid_and_keep_placeholders() {
    let (_, all) = strings();
    let table: HashMap<String, String> = serde_json::from_str(Language::Polish.source()).unwrap();
    for (en, said) in &table {
        assert!(all.contains(en), "unused Polish string: {en:?}");
        assert!(!said.trim().is_empty(), "empty Polish string: {en:?}");
        assert_eq!(holes(en), holes(said), "Polish: {en:?}");
    }
}
