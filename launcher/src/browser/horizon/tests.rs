//! The registry source against the sample index of `horizon_mods::index`
//! (`crates/horizon-mods/tests/index/`), its files served from disk.
use super::*;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/horizon-mods/tests/index")
}

/// The sample index with its files on disk, as a local test registry lists them.
fn sample() -> Index {
    let text = fs::read_to_string(fixtures().join("index-v1.json")).unwrap();
    let mut index = Index::parse(&text).unwrap();
    for p in &mut index.projects {
        for v in &mut p.versions {
            let path = fixtures().join(&v.file.name);
            v.file.url = format!("file://{}", path.canonicalize().unwrap_or(path).display());
        }
    }
    index
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-launcher-registry-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn hud(index: &Index) -> Project {
    project_of(index.project("coords_hud").unwrap()).unwrap()
}

#[test]
fn native_registry_entries_are_mods_that_say_they_run_native_code() {
    assert_eq!(kind_of(&index::Kind::NativeMod), Some(Kind::Mod));
    let mut index = sample();
    index.projects[0].kind = index::Kind::NativeMod;
    let found = search(&index, &Query::new("", Kind::Mod));
    assert!(found.iter().any(|p| p.id == index.projects[0].id));
    let entry = |file: &str| {
        serde_json::from_value::<installed::Entry>(serde_json::json!({
            "source": "horizon", "project": "p", "slug": "p", "title": "P", "kind": "mod",
            "version": "1", "number": "1", "file": file, "checksum": "", "page": ""
        }))
        .unwrap()
    };
    assert_eq!(
        mod_id(&entry("horizon-native-mods/veinminer.hnmod")),
        Some("veinminer")
    );
    assert_eq!(mod_id(&entry("horizon-mods/zoom.hmod")), Some("zoom"));
}

#[test]
fn only_horizons_own_projects_go_without_the_not_endorsed_line() {
    let mut p = sample().projects[0].clone();
    p.authors = vec!["Horizon".into()];
    p.source = "https://github.com/horizon-rust/horizon-mod-zoom".into();
    assert!(official(&p));
    p.source = "https://github.com/someone/horizon-mod-zoom".into();
    assert!(!official(&p), "the name alone doesn't make it official");
    p.authors = vec!["levzzz5154".into()];
    assert!(!official(&p));
}

#[test]
fn searches_by_kind_and_words() {
    let index = sample();
    let found = |text: &str, kind| -> Vec<String> {
        search(&index, &Query::new(text, kind))
            .into_iter()
            .map(|p| p.id)
            .collect()
    };
    assert_eq!(found("", Kind::Mod), ["coords_hud"]);
    assert_eq!(found("coordinates HUD", Kind::Mod), ["coords_hud"]);
    assert!(found("zoom", Kind::Mod).is_empty());
    // Modpacks under Modpacks; a kind this launcher can't install yet (a world) never.
    assert_eq!(found("", Kind::Modpack), ["cozy"]);
    assert!(found("skyblock", Kind::ResourcePack).is_empty());
    assert_eq!(found("", Kind::Shader), ["glow"]);
    // The index's download counts, most downloaded first.
    let mut counted = index.clone();
    counted.projects.push(index::Project {
        id: "zoom".into(),
        name: "Zoom".into(),
        downloads: 12_345,
        ..index.project("coords_hud").unwrap().clone()
    });
    counted.projects[0].downloads = 80;
    let first = search(&counted, &Query::new("", Kind::Mod));
    let order: Vec<(&str, u64)> = first.iter().map(|p| (p.id.as_str(), p.downloads)).collect();
    assert_eq!(order, [("zoom", 12_345), ("coords_hud", 80)]);
    let p = hud(&index);
    assert_eq!(p.support(), super::super::Support::Works);
    assert_eq!(p.page, "https://playhorizon.dev/mods/coords_hud");
}

#[test]
fn installs_checked_turns_on_and_removes() {
    let index = sample();
    let home = scratch("install");
    let p = hud(&index);
    let done = install_from(&index, true, None, &p, &home, &|_, _| {}).unwrap();
    assert_eq!(done[0].file, "horizon-mods/coords_hud.hmod");
    assert_eq!(done[0].verdict.as_deref(), Some("Verified"));
    assert!(done[0].checksum.starts_with("SHA-256:e1135345"));
    assert!(home.join("horizon-mods/coords_hud.hmod").is_file());
    let list = fs::read_to_string(home.join("horizon-mods.txt")).unwrap();
    assert_eq!(list, "mod.coords_hud:true\n");
    assert_eq!(
        fs::read_dir(home.join("mod-browser/downloads"))
            .unwrap()
            .count(),
        0
    );
    // Installed already: nothing to do.
    let again = install_from(&index, true, None, &p, &home, &|_, _| {}).unwrap();
    assert_eq!(again, done);

    let mut revoked = index.clone();
    revoked.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let said = turn_off_revoked(&revoked, &home);
    assert_eq!(said, ["Coordinates HUD was taken down and is off: Test."]);
    assert_eq!(
        fs::read_to_string(home.join("horizon-mods.txt")).unwrap(),
        ""
    );

    installed::remove(&home, "coords_hud").unwrap();
    assert!(!home.join("horizon-mods/coords_hud.hmod").exists());
    assert!(installed::Installed::load(&home).entries.is_empty());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_mod_that_turned_native_keeps_its_hmod_until_removed() {
    assert!(stays(true, "horizon-mods/coords_hud.hmod"));
    assert!(!stays(false, "horizon-mods/coords_hud.hmod"));
    assert!(!stays(true, "horizon-native-mods/coords_hud.hnmod"));
    let index = sample();
    let home = scratch("turned-native");
    install_from(&index, true, None, &hud(&index), &home, &|_, _| {}).unwrap();
    // As install_version leaves it after the native update.
    let mut list = installed::Installed::load(&home);
    list.entries[0].file = "horizon-native-mods/coords_hud.hnmod".into();
    list.save(&home).unwrap();
    fs::create_dir_all(home.join("horizon-native-mods")).unwrap();
    fs::write(home.join("horizon-native-mods/coords_hud.hnmod"), "").unwrap();
    installed::remove(&home, "coords_hud").unwrap();
    assert!(!home.join("horizon-native-mods/coords_hud.hnmod").exists());
    assert!(!home.join("horizon-mods/coords_hud.hmod").exists());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn installs_a_shader_pack_into_shaderpacks() {
    let index = sample();
    let home = scratch("shader");
    let p = project_of(index.project("glow").unwrap()).unwrap();
    assert_eq!(p.support(), super::super::Support::Works);
    let done = install_from(&index, true, None, &p, &home, &|_, _| {}).unwrap();
    assert_eq!(done[0].file, "shaderpacks/glow-1.0.0.hshader");
    assert_eq!(done[0].kind, Kind::Shader);
    assert!(home.join("shaderpacks/glow-1.0.0.hshader").is_file());
    // Not a mod: nothing turns on in horizon-mods.txt; the game's Shader Packs page picks it.
    assert!(!home.join("horizon-mods.txt").exists());

    let mut revoked = index.clone();
    revoked.revoked.push(index::Revoked {
        id: "glow".into(),
        sha256: None,
        reason: "Test.".into(),
        date: String::new(),
    });
    let said = turn_off_revoked(&revoked, &home);
    assert_eq!(said, ["Soft Glow was taken down: Test. Remove it."]);

    installed::remove(&home, "glow").unwrap();
    assert!(!home.join("shaderpacks/glow-1.0.0.hshader").exists());
    // A file of the same name the registry didn't put there is left alone.
    fs::write(home.join("shaderpacks/glow-1.0.0.hshader"), b"mine").unwrap();
    let e = install_from(&index, true, None, &p, &home, &|_, _| {}).unwrap_err();
    assert_eq!(
        e,
        "shaderpacks/glow-1.0.0.hshader is already there: move it away first."
    );
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn refuses_a_changed_file_a_taken_id_and_local_files_from_the_real_registry() {
    let index = sample();
    let p = hud(&index);

    let home = scratch("tampered");
    let mut tampered = index.clone();
    let cozy = tampered.projects[1].versions[0].file.url.clone();
    tampered.projects[0].versions[0].file.url = cozy;
    let e = install_from(&tampered, true, None, &p, &home, &|_, _| {}).unwrap_err();
    assert!(e.contains("larger than its source says"), "{e}");
    assert!(!home.join("horizon-mods").exists());

    let e = install_from(&index, false, None, &p, &home, &|_, _| {}).unwrap_err();
    assert!(e.contains("file servers"), "{e}");

    fs::create_dir_all(home.join("horizon-mods/coords_hud")).unwrap();
    let e = install_from(&index, true, None, &p, &home, &|_, _| {}).unwrap_err();
    assert!(e.contains("already in horizon-mods"), "{e}");

    let mut taken_down = index.clone();
    taken_down.revoked.push(index::Revoked {
        id: "coords_hud".into(),
        sha256: Some(index.projects[0].versions[0].file.sha256.clone()),
        reason: "Test.".into(),
        date: String::new(),
    });
    let e = install_from(&taken_down, true, None, &p, &home, &|_, _| {}).unwrap_err();
    assert_eq!(e, "Coordinates HUD was taken down: Test.");
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn registry_versions_run_when_made_for_this_game() {
    use super::super::{pick, support, Support};
    let v = |game: &str| vec![game.to_owned()];
    let horizon = vec![LOADER.to_owned()];
    assert_eq!(support(Kind::Mod, &v(GAME), &horizon), Support::Works);
    assert!(!support(Kind::Mod, &v("26.2"), &horizon).installable());
    let index = sample();
    let p = index.project("coords_hud").unwrap();
    let versions: Vec<Version> = p
        .versions
        .iter()
        .map(|x| version_of(&index, p, x, None))
        .collect();
    assert_eq!(pick(Kind::Mod, &versions, false).unwrap().number, "0.1.0");
}

#[test]
fn owner_links_only_take_github_logins() {
    for ok in ["Stink-O", "ludovic111", "a"] {
        assert!(github_login(ok), "{ok}");
    }
    for bad in [
        "",
        "-x",
        "x-",
        "a--b",
        "bad login",
        "a/b",
        "x?y=1",
        &"a".repeat(40),
    ] {
        assert!(!github_login(bad), "{bad}");
    }
}

/// A version that calls mod API functions the game doesn't have is marked and skipped.
#[test]
fn versions_for_a_newer_horizon_are_not_installed() {
    let mut index = sample();
    index.projects[0].versions[0].api_level = Some(1);
    let p = index.project("coords_hud").unwrap();
    let e = newest(&index, p, Some(0)).unwrap_err();
    assert_eq!(
        e,
        "Coordinates HUD needs a newer Horizon than the game you play: update it first."
    );
    assert!(newest(&index, p, Some(1)).is_ok());
    assert!(newest(&index, p, None).is_ok());
    let v = version_of(&index, p, &p.versions[0], Some(0));
    assert_eq!(v.blocked.as_deref(), Some(NEEDS_NEWER));
    assert_eq!(version_of(&index, p, &p.versions[0], Some(1)).blocked, None);
}

#[test]
fn ratings_keep_only_what_is_well_formed() {
    let text = r#"{"projects": {
        "zoom": {"average": 4.25, "count": 12},
        "minimap": {"average": 9, "count": 3},
        "keystrokes": {"average": 3, "count": 0},
        "Bad Id": {"average": 4, "count": 1},
        "fps_overlay": {"average": "5", "count": 1}
    }}"#;
    let ratings = parse_ratings(text);
    assert_eq!(
        ratings.into_iter().collect::<Vec<_>>(),
        [(
            "zoom".to_owned(),
            Rating {
                average: 4.25,
                count: 12
            }
        )]
    );
    assert!(parse_ratings("not json").is_empty());
    assert!(parse_ratings(r#"{"projects": []}"#).is_empty());
    assert_eq!(
        rate_url("zoom"),
        "https://playhorizon.dev/api/mods/auth/login?back=/mods/zoom"
    );
}
