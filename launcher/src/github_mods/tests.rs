use super::*;
use serde_json::{json, Value};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const SHA2: &str = "fedcba9876543210fedcba9876543210fedcba98";

fn link(text: &str) -> Link {
    parse(text).unwrap()
}

#[test]
fn links_name_a_repository_release_ref_or_commit() {
    let plain = link("https://github.com/acme/zoom");
    assert_eq!(
        (plain.owner.as_str(), plain.repo.as_str(), plain.target),
        ("acme", "zoom", Target::Latest)
    );
    assert_eq!(link("https://github.com/acme/zoom.git/").repo, "zoom");
    assert_eq!(
        link("https://github.com/acme/zoom/releases").target,
        Target::Latest
    );
    assert_eq!(
        link("https://github.com/acme/zoom/releases/tag/v1.2.0").target,
        Target::Tag("v1.2.0".into())
    );
    // A branch name keeps its slashes.
    assert_eq!(
        link("https://github.com/acme/zoom/tree/feature/x?tab=1").target,
        Target::Ref("feature/x".into())
    );
    assert_eq!(
        link("https://github.com/acme/zoom/commit/ABCDEF1").target,
        Target::Commit("abcdef1".into())
    );
}

#[test]
fn links_that_are_not_github_repositories_are_refused() {
    for bad in [
        "",
        "acme/zoom",
        "http://github.com/acme/zoom",
        "https://gitlab.com/acme/zoom",
        "https://github.com/acme",
        "https://github.com/acme/zoom/issues/3",
        "https://github.com/acme/zoom/commit/not-hex",
        "https://github.com/acme/zoom/tree/a..b",
        "https://github.com/../zoom",
        "https://github.com/acme/zo om",
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

/// A recorded API: the JSON each path answers, 404 for the rest.
fn recorded(answers: Vec<(&'static str, Value)>) -> impl Fn(&str) -> Result<Option<Value>, String> {
    move |path| {
        Ok(answers
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, v)| v.clone()))
    }
}

fn release(tag: &str, assets: Value) -> Value {
    json!({ "tag_name": tag, "assets": assets })
}

fn asset(name: &str, digest: Option<&str>) -> Value {
    json!({
        "name": name,
        "browser_download_url": format!("https://github.com/acme/zoom/releases/download/v1/{name}"),
        "digest": digest,
    })
}

#[test]
fn a_repository_resolves_to_its_latest_releases_hnmod_and_commit() {
    let digest = format!("sha256:{}", "AB".repeat(32));
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release(
                "v1",
                json!([
                    asset("zoom.hmod", None),
                    asset("zoom-1.hnmod", Some(&digest))
                ]),
            ),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert_eq!(
        (found.tag.as_deref(), found.commit.as_str()),
        (Some("v1"), SHA)
    );
    let file = found.asset.unwrap();
    assert_eq!(file.name, "zoom-1.hnmod");
    assert_eq!(file.sha256, Some("ab".repeat(32)));
    assert_eq!(found.content_only, None);
}

#[test]
fn a_release_with_no_hnmod_falls_back_to_its_commit_and_remembers_a_content_mod() {
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release("v1", json!([asset("zoom.hmod", None)])),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert_eq!(found.asset, None);
    assert_eq!(found.content_only.as_deref(), Some("zoom.hmod"));
    assert_eq!(found.commit, SHA);
}

#[test]
fn an_asset_from_another_host_is_ignored() {
    let evil = json!({ "name": "x.hnmod", "browser_download_url": "https://evil.example/x.hnmod" });
    let api = recorded(vec![
        (
            "repos/acme/zoom/releases/latest",
            release("v1", json!([evil])),
        ),
        ("repos/acme/zoom/commits/v1", json!({ "sha": SHA })),
    ]);
    assert_eq!(
        resolve(&link("https://github.com/acme/zoom"), &api)
            .unwrap()
            .asset,
        None
    );
}

#[test]
fn no_release_resolves_the_default_branch_to_a_commit() {
    let api = recorded(vec![
        ("repos/acme/zoom", json!({ "default_branch": "dev/main" })),
        ("repos/acme/zoom/commits/dev/main", json!({ "sha": SHA })),
    ]);
    let found = resolve(&link("https://github.com/acme/zoom"), &api).unwrap();
    assert_eq!(
        (&found.tag, &found.asset, found.commit.as_str()),
        (&None, &None, SHA)
    );
    assert_eq!(found.label(), "acme/zoom @ 0123456");
    assert_eq!(
        found.source_url(),
        format!("https://github.com/acme/zoom/tree/{SHA}")
    );
}

#[test]
fn refs_and_short_commits_become_full_commits_and_unknown_ones_say_so() {
    let api = recorded(vec![
        ("repos/acme/zoom/commits/feature/x", json!({ "sha": SHA })),
        ("repos/acme/zoom/commits/abcdef1", json!({ "sha": SHA2 })),
        ("repos/acme/zoom/releases/tags/v9", Value::Null),
    ]);
    let commit = |url: &str| resolve(&link(url), &api).map(|r| r.commit);
    assert_eq!(
        commit("https://github.com/acme/zoom/tree/feature/x"),
        Ok(SHA.into())
    );
    assert_eq!(
        commit("https://github.com/acme/zoom/commit/abcdef1"),
        Ok(SHA2.into())
    );
    let err = commit("https://github.com/acme/zoom/tree/gone").unwrap_err();
    assert!(err.contains("no commit, tag or branch gone"), "{err}");
    // A tag with no release is a commit.
    assert!(commit("https://github.com/acme/zoom/releases/tag/v9").is_err());
}

#[test]
fn a_rate_limit_is_said_plainly() {
    let limited = |_: &str| Err(RATE_LIMIT.to_string());
    let err = resolve(&link("https://github.com/acme/zoom"), &limited).unwrap_err();
    assert!(
        err.contains("rate limit") && err.contains("Try again"),
        "{err}"
    );
}

fn tree(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gh-mods-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn mixin(id: &str, pin: bool) -> String {
    let compat = match pin {
        true => format!("[compatibility]\nhorizon_revision = \"{SHA}\"\n"),
        false => String::new(),
    };
    format!("[mod]\nid = \"{id}\"\nversion = \"0.1.0\"\n{compat}")
}

#[test]
fn mod_folders_are_found_wherever_they_are_and_not_searched_inside() {
    let root = tree("find");
    write(
        &root.join("native/zoom/horizon-mixin.toml"),
        &mixin("zoom", true),
    );
    write(
        &root.join("native/zoom/src/horizon-mixin.toml"),
        &mixin("inner", true),
    );
    write(
        &root.join("native/keys/horizon-mixin.toml"),
        &mixin("keys", true),
    );
    write(
        &root.join("target/debug/horizon-mixin.toml"),
        &mixin("built", true),
    );
    write(
        &root.join(".git/x/horizon-mixin.toml"),
        &mixin("hidden", true),
    );
    let ids: Vec<String> = find_mods(&root)
        .unwrap()
        .into_iter()
        .map(|b| b.id)
        .collect();
    assert_eq!(ids, ["keys", "zoom"]);
    assert!(find_mods(&tree("empty")).unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_mod_folder_packs_into_a_bundle_the_registry_checks_accept() {
    let root = tree("pack");
    write(&root.join("horizon-mixin.toml"), &mixin("zoom", true));
    write(&root.join("src/zoom.rs"), "pub fn f() {}");
    write(&root.join("data/a.json"), "{}");
    write(&root.join("README.md"), "# Zoom");
    // A crate of the mod keeps its own data and the text files it may include.
    write(&root.join("engine/data/table.bin"), "t");
    write(&root.join("engine/LICENSE"), "MIT");
    write(&root.join("engine/logo.png"), "not shipped");
    let bundle = &find_mods(&root).unwrap()[0];
    let bytes = pack(bundle).unwrap();
    assert_eq!(bytes, pack(bundle).unwrap(), "the same files pack the same");
    let packed = checked(&bytes, &root.join("out")).unwrap();
    assert_eq!(
        (packed.id.as_str(), packed.version.as_str()),
        ("zoom", "0.1.0")
    );
    assert_eq!(packed.sha256, sha256(&bytes));
    let read = format::Bundle::read(&packed.file).unwrap();
    assert!(read.files.contains_key("mixins/zoom/src/zoom.rs"));
    assert!(read.files.contains_key("mixins/zoom/data/a.json"));
    for shipped in ["README.md", "engine/data/table.bin", "engine/LICENSE"] {
        assert!(
            read.files.contains_key(&format!("mixins/zoom/{shipped}")),
            "{shipped}"
        );
    }
    assert!(!read.files.keys().any(|n| n.contains("logo")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_mod_with_no_pinned_horizon_commit_is_refused_with_the_fix() {
    let root = tree("unpinned");
    write(&root.join("horizon-mixin.toml"), &mixin("zoom", false));
    let err = pack(&find_mods(&root).unwrap()[0]).unwrap_err();
    assert!(err.contains("horizon_revision"), "{err}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_unpacked_hnmod_keeps_its_pins_and_hook_points_and_ships_only_sources() {
    let root = tree("unpacked");
    let manifest = json!({
        "schema": 1, "id": "zoom", "version": "0.1.0", "game": "26.3",
        "horizon": SHA, "mixins": SHA2, "hooks": 3,
    });
    write(&root.join("horizon-native.json"), &manifest.to_string());
    write(
        &root.join("mixins/zoom/horizon-mixin.toml"),
        &mixin("zoom", true),
    );
    let bundle = &find_mods(&root).unwrap()[0];
    assert_eq!(bundle.mixins.as_deref(), Some(SHA2));
    let packed = checked(&pack(bundle).unwrap(), &root.join("out")).unwrap();
    let read = format::Bundle::read(&packed.file).unwrap();
    assert_eq!(
        (
            read.manifest.horizon.as_str(),
            read.manifest.mixins.as_str()
        ),
        (SHA, SHA2)
    );
    assert_eq!(read.manifest.hooks, Some(3));
    // Only sources and data ship: a binary beside them stays behind.
    write(&root.join("mixins/zoom/evil.exe"), "MZ");
    let again = checked(&pack(bundle).unwrap(), &root.join("out2")).unwrap();
    assert_eq!(again.sha256, packed.sha256);
    // A manifest and a mixin file that pin different commits are refused when found.
    write(
        &root.join("mixins/zoom/horizon-mixin.toml"),
        &format!(
            "{}[compatibility]\nhorizon_revision = \"{SHA2}\"\n",
            mixin("zoom", false)
        ),
    );
    assert!(find_mods(&root).unwrap_err().contains("pins Horizon"));
    let _ = fs::remove_dir_all(root);
}

fn packed(id: &str) -> Packed {
    Packed {
        id: id.into(),
        version: "0.1.0".into(),
        file: PathBuf::new(),
        sha256: String::new(),
    }
}

#[test]
fn several_mods_list_and_ask_which() {
    let two = [packed("keys"), packed("zoom")];
    let err = choose(&two, None, false).unwrap_err();
    assert!(err.contains("keys, zoom") && err.contains("--all"), "{err}");
    assert_eq!(choose(&two, Some("zoom"), false).unwrap()[0].id, "zoom");
    assert_eq!(choose(&two, None, true).unwrap().len(), 2);
    assert!(choose(&two, Some("map"), false).is_err());
    assert_eq!(choose(&two[..1], None, false).unwrap()[0].id, "keys");
}

#[test]
fn installing_records_the_origin_and_updates_follow_releases_or_the_head() {
    let home = tree("install");
    let source = tree("install-src");
    write(&source.join("horizon-mixin.toml"), &mixin("zoom", true));
    let bytes = pack(&find_mods(&source).unwrap()[0]).unwrap();
    let ready = checked(&bytes, &source.join("out")).unwrap();
    let found = Resolved {
        owner: "acme".into(),
        repo: "zoom".into(),
        tag: None,
        commit: SHA.into(),
        asset: None,
        content_only: None,
    };
    let kept = ready.file.clone();
    let origin = install(&home, &found, &ready).unwrap();
    assert!(!kept.exists());
    assert_eq!(origin.label(), "acme/zoom @ 0123456");
    assert_eq!(origin.sha256, sha256(&bytes));
    assert_eq!(self::origin(&home, "zoom"), Some(origin.clone()));
    assert_eq!(origins(&home), vec![origin.clone()]);
    assert!(java::is_enabled(&home, "zoom"));
    // A commit's mod follows the default branch's head.
    assert_eq!(origin.follows().target, Target::Head);
    let newer = Resolved {
        commit: SHA2.into(),
        ..found.clone()
    };
    assert!(is_update(&origin, &newer));
    assert!(!is_update(&origin, &found));
    // A release's mod follows the latest release.
    let release = Origin {
        tag: Some("v1".into()),
        asset: Some("zoom.hnmod".into()),
        ..origin.clone()
    };
    assert_eq!(release.label(), "acme/zoom @ v1");
    assert_eq!(release.follows().target, Target::Latest);
    let v2 = Resolved {
        tag: Some("v2".into()),
        ..found.clone()
    };
    assert!(is_update(&release, &v2));
    assert!(!is_update(
        &release,
        &Resolved {
            tag: Some("v1".into()),
            ..found
        }
    ));
    // A file that isn't from GitHub is never replaced.
    write(&home.join(INSTALLED).join("keys.hnmod"), "x");
    let other = Packed {
        id: "keys".into(),
        ..packed("keys")
    };
    assert!(install(&home, &newer, &other)
        .unwrap_err()
        .contains("not from GitHub"));
    assert!(remove(&home, "zoom").unwrap());
    assert!(!remove(&home, "zoom").unwrap());
    assert!(!home.join(INSTALLED).join("zoom.hnmod").exists());
    assert!(!java::is_enabled(&home, "zoom"));
    let _ = fs::remove_dir_all(home);
    let _ = fs::remove_dir_all(source);
}
