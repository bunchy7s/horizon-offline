// The test texts spell the slurs the filter must refuse: that is what they test.
use super::*;

fn filter() -> Filter {
    Filter::default_list()
}

#[test]
fn the_list_compiles() {
    let f = filter();
    assert!(f.len() > 30, "{} entries", f.len());
    assert_eq!(f.allowed.len(), 2);
}

#[test]
fn normalising_catches_the_usual_disguises() {
    let f = filter();
    for text in [
        "nigger",
        "NIGGER",
        "shut up nigga",
        "Niggers",
        "n1gg3r",
        "ni99a",
        "N!GGA",
        "nigg@",
        "n.i.g.g.e.r",
        "n_i_g_g_a",
        "n i g g e r",
        "n i gg a",
        "nig ger",
        "nig  gaaa",
        "niiiiggggggerrrrr",
        "ñíggér",
        "ｎｉｇｇｅｒ",
        "Niggeг",         // Cyrillic ghe for r
        "nіgger",         // Cyrillic i
        "nigg\u{0435}r",  // Cyrillic e
        "n\u{200B}igger", // a zero-width space
        "n\u{0301}igger", // a combining accent
        "𝐧𝐢𝐠𝐠𝐞𝐫",
        "ⓝⓘⓖⓖⓐ",
        "ɴɪɢɢᴀ",
        "hi nig",
        "f a g",
        "fag!",
        "you $pic",
        "FAGGOTINO-CHAN",
        "sandnigger",
        "lol kike",
    ] {
        assert!(f.matches(text), "{text:?} should be refused");
    }
}

#[test]
fn ordinary_words_pass() {
    let f = filter();
    for text in [
        "hello",
        "Niger",
        "I live in Niger",
        "Nigeria and the Niger river",
        "nigerian food",
        "proud2benigerian",
        "don't snigger",
        "sniggering at that",
        "a niggardly offer",
        "knight",
        "qKnights",
        "its still night",
        "anyone able to boot the latest nightly version?",
        "i hate nighbours",
        "can someune give mi night vision",
        "suspicious stew",
        "spicy food",
        "a raccoon in a cocoon",
        "Scunthorpe",
        "bigger trigger digger",
        "gg",
        "gg ez",
        "mini gg all",
        "kniga",
        "shenanigans",
        "dignity and signing",
        "Pakistan",
        "chinking glasses",
        "the trannys? no: transmission",
        "1nicerOli",
        "ask me evernig",
        "冰淇淋",
        "привет",
        "",
        "!!!",
    ] {
        assert!(!f.matches(text), "{text:?} should pass");
    }
}

#[test]
fn a_custom_list_has_the_same_syntax() {
    let f = Filter::new("# comment\n=bad\n~worse\nmeh\n!mehish*\n");
    assert!(f.matches("so bad"));
    assert!(!f.matches("badly"));
    assert!(f.matches("wor se"));
    assert!(f.matches("xxmehxx"));
    assert!(!f.matches("mehishness"));
    assert!(!f.matches("me h"));
}

fn guard(config: Config) -> Guard {
    Guard::new(config, filter(), None)
}

const NOW: i64 = 1_793_469_939_000;
const MIN: i64 = 60_000;

#[test]
fn the_second_hit_mutes_and_the_third_bans() {
    let mut g = guard(Config::default());
    let id = 7;
    assert_eq!(g.strike(id, "Ann", NOW), (1, Action::Refused));
    assert_eq!(g.muted_until(id, NOW), None);
    let until = NOW + 3 * MIN + 10 * MIN;
    assert_eq!(
        g.strike(id, "Ann", NOW + 3 * MIN),
        (2, Action::Muted { until })
    );
    assert_eq!(g.muted_until(id, NOW + 4 * MIN), Some(until));
    assert_eq!(g.muted_until(id, until), None);
    // The time spent muted doesn't count: 8 minutes after the mute ended is still within the window.
    assert_eq!(
        g.strike(id, "Ann", until + 8 * MIN),
        (
            3,
            Action::Banned {
                until: until + 8 * MIN + 24 * 60 * MIN
            }
        )
    );
    // Then it starts again.
    assert_eq!(g.strike(id, "Ann", until + 9 * MIN), (1, Action::Refused));
}

#[test]
fn hits_far_apart_dont_add_up() {
    let mut g = guard(Config::default());
    assert_eq!(g.strike(1, "Bo", NOW), (1, Action::Refused));
    assert_eq!(g.strike(1, "Bo", NOW + 11 * MIN), (1, Action::Refused));
    // Players are counted apart.
    assert_eq!(g.strike(2, "Cy", NOW + 12 * MIN), (1, Action::Refused));
}

#[test]
fn thresholds_are_configurable() {
    let mut g = guard(Config {
        mute_after: 0,
        tempban_after: 2,
        tempban_hours: 1,
        ..Config::default()
    });
    assert_eq!(g.strike(1, "Bo", NOW), (1, Action::Refused));
    assert_eq!(
        g.strike(1, "Bo", NOW + MIN),
        (
            2,
            Action::Banned {
                until: NOW + MIN + 60 * MIN
            }
        )
    );
    let mut off = guard(Config {
        mute_after: 0,
        tempban_after: 0,
        ..Config::default()
    });
    for k in 1..5 {
        assert_eq!(off.strike(1, "Bo", NOW + k), (k as u32, Action::Refused));
    }
}

#[test]
fn mutes_survive_a_restart_and_unmute_lifts_them() {
    let dir = std::env::temp_dir().join(format!("chat-filter-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let now = now_millis();
    let (mut g, problem) = Guard::load(Config::default(), &dir);
    assert_eq!(problem, None);
    g.strike(5, "Dee", now);
    let (_, action) = g.strike(5, "Dee", now);
    assert!(matches!(action, Action::Muted { .. }));
    let (mut again, _) = Guard::load(Config::default(), &dir);
    assert!(again.muted_until(5, now).is_some());
    assert_eq!(again.unmute("dee").as_deref(), Some("Dee"));
    assert_eq!(again.unmute("dee"), None);
    assert_eq!(again.muted_until(5, now), None);
    let (reloaded, _) = Guard::load(Config::default(), &dir);
    assert_eq!(reloaded.muted_until(5, now), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_words_file_falls_back_to_the_built_in_list() {
    let config = Config {
        words_file: "nope.txt".into(),
        ..Config::default()
    };
    let (g, problem) = Guard::load(config, Path::new("/nonexistent-horizon"));
    assert!(problem.unwrap().contains("nope.txt"));
    assert!(g.filter.matches("nigger"));
}

#[test]
fn the_section_reads_from_server_toml() {
    let c = Config::from_server_toml("[server]\nport = 25565\n").unwrap();
    assert_eq!(c, Config::default());
    assert!(c.enabled);
    let c = Config::from_server_toml(
        "[chat_filter]\nenabled = true\nmute_after = 3\ntempban_after = 5\nwords_file = \"slurs.txt\"\n",
    )
    .unwrap();
    assert_eq!((c.mute_after, c.tempban_after), (3, 5));
    assert_eq!(c.words_file, "slurs.txt");
    assert!(
        !Config::from_server_toml("[chat_filter]\nenabled = false\n")
            .unwrap()
            .enabled
    );
    assert!(Config::from_server_toml("[chat_filter]\nmute = 2\n").is_err());
    assert!(Config::from_server_toml("[chat_filter]\nwindow_minutes = 0\n").is_err());
}

#[test]
fn log_lines_have_the_time_the_player_and_the_text() {
    assert_eq!(utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(utc(951_782_400_000), "2000-02-29T00:00:00Z");
    assert_eq!(
        log_line(NOW, "Ann", 1, "chat", 2, Action::Muted { until: 0 }, "a\nb"),
        "2026-10-31T18:05:39Z Ann 00000000-0000-0000-0000-000000000001 chat hit 2 muted: a b"
    );
}

#[test]
fn a_message_costs_little() {
    // A long message through the whole list stays far below a tick (debug builds included).
    let f = filter();
    let text = "the quick brown fox jumps over the lazy dog ".repeat(6);
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        assert!(!f.matches(&text));
    }
    assert!(start.elapsed().as_millis() < 2000, "{:?}", start.elapsed());
}
