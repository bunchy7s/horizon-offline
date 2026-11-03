//! Forge's and NeoForge's lists recorded from their repositories (trimmed, in
//! `launcher/tests/fixtures/loaders/`).
use super::*;
use crate::loaders::pick;

const METADATA: &str = include_str!("../../tests/fixtures/loaders/forge-maven-metadata.xml");
const PROMOTIONS: &str = include_str!("../../tests/fixtures/loaders/forge-promotions_slim.json");
const NEOFORGE: &str = include_str!("../../tests/fixtures/loaders/neoforge-versions.json");

fn versions_of(listed: &[Listed]) -> Vec<(&str, bool)> {
    listed
        .iter()
        .map(|l| (l.version.as_str(), l.stable))
        .collect()
}

#[test]
fn forge_lists_a_minecraft_versions_builds_stable_up_to_the_recommended_one() {
    // 26.3's recommended build is 66.1.0: 66.1.3 is newer than Forge vouches for.
    let listed = parse_forge(METADATA, PROMOTIONS, "26.3");
    assert_eq!(
        versions_of(&listed),
        [("66.1.3", false), ("66.1.0", true), ("66.0.13", true)]
    );
    assert_eq!(pick(Kind::Forge, &listed, None).unwrap(), "66.1.0");
    // 1.20 is not 1.20.1; an old build keeps the game's version it carries after its own.
    assert_eq!(
        versions_of(&parse_forge(METADATA, PROMOTIONS, "1.20")),
        [("46.0.14", true)]
    );
    assert_eq!(
        parse_forge(METADATA, PROMOTIONS, "1.7.10")[0].version,
        "10.13.4.1614-1.7.10"
    );
    // Without promotions every build installs, none held back.
    let listed = parse_forge(METADATA, "", "1.20.1");
    assert_eq!(listed[0].version, "47.4.0");
    assert!(listed.iter().all(|l| l.stable));
    assert!(parse_forge(METADATA, PROMOTIONS, "1.21.4").is_empty());
}

#[test]
fn neoforge_lists_by_the_game_version_its_numbers_carry() {
    let of = |mc| -> Vec<String> {
        let listed = parse_neoforge(NEOFORGE, mc).unwrap();
        listed.into_iter().map(|l| l.version).collect()
    };
    assert_eq!(of("1.21.1"), ["21.1.228", "21.1.77"]);
    assert_eq!(of("1.21"), ["21.0.167"]);
    assert_eq!(of("1.20.2"), ["20.2.86"]);
    assert_eq!(of("26.3"), ["26.3.0.101", "26.3.0.76", "26.3.0.75-beta"]);
    assert!(of("25w14craftmine").is_empty());
    let listed = parse_neoforge(NEOFORGE, "26.3").unwrap();
    assert!(!listed[2].stable);
    assert_eq!(pick(Kind::NeoForge, &listed, None).unwrap(), "26.3.0.101");
    // 1.20.1's builds are its `forge` artifact's, the game's version in front.
    let old = r#"{"versions": ["1.20.1-47.1.5", "1.20.1-47.1.106", "1.20.1-47.1.79"]}"#;
    let listed = parse_neoforge(old, "1.20.1").unwrap();
    assert_eq!(versions_of(&listed)[0], ("47.1.106", true));
}
