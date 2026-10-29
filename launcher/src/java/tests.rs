//! Fake Java homes in a scratch folder: a `release` file and an empty `bin/java`.
use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-java-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn fake_home(home: &Path, version: &str) -> PathBuf {
    // A Windows home has both: the runtime search looks for java.exe, the game starts javaw.exe.
    let programs: &[&str] = if cfg!(windows) {
        &["java.exe", "javaw.exe"]
    } else {
        &["java"]
    };
    fs::create_dir_all(home.join("bin")).unwrap();
    for program in programs {
        fs::write(home.join("bin").join(program), b"").unwrap();
    }
    fs::write(
        home.join("release"),
        format!("IMPLEMENTOR=\"Fake\"\nJAVA_VERSION=\"{version}\"\nOS_NAME=\"Darwin\"\n"),
    )
    .unwrap();
    home.to_path_buf()
}

fn need(major: u32) -> Need {
    Need {
        major,
        component: format!("c{major}"),
    }
}

#[test]
fn a_version_names_its_java_and_old_ones_ran_java_8() {
    let new =
        serde_json::json!({"javaVersion": {"component": "java-runtime-delta", "majorVersion": 21}});
    assert_eq!(
        Need::of(&new),
        Need {
            major: 21,
            component: "java-runtime-delta".into()
        }
    );
    let old = serde_json::json!({"id": "1.12.2"});
    assert_eq!(
        Need::of(&old),
        Need {
            major: 8,
            component: "jre-legacy".into()
        }
    );
}

#[test]
fn the_release_file_gives_the_version_java_8_included() {
    let dir = scratch("release");
    for (version, major) in [
        ("21.0.5", 21),
        ("1.8.0_432", 8),
        ("25", 25),
        ("17.0.12+7", 17),
    ] {
        let home = fake_home(&dir.join(version), version);
        assert_eq!(release_version(&home), Some((version.to_string(), major)));
    }
    assert_eq!(release_version(&dir.join("none")), None);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn mojangs_runtimes_are_found_in_their_layout() {
    let dir = scratch("mojang");
    let platform = mojang::java_platform().unwrap_or("linux");
    let delta = fake_home(
        &dir.join(format!("java-runtime-delta/{platform}/java-runtime-delta")),
        "21.0.5",
    );
    // macOS runtimes keep their home inside a bundle.
    let legacy = fake_home(
        &dir.join(format!(
            "jre-legacy/{platform}/jre-legacy/jre.bundle/Contents/Home"
        )),
        "1.8.0_432",
    );
    let found = mojang_runtimes(&dir);
    let homes: Vec<_> = found.iter().map(|r| (r.home.clone(), r.major)).collect();
    assert_eq!(homes, [(delta, 21), (legacy, 8)]);
    assert!(found.iter().all(|r| r.source == Source::Mojang));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_pick_is_the_exact_major_mojangs_first_never_a_newer_one() {
    let dir = scratch("pick");
    let rt = |name: &str, version: &str, source| {
        runtime(fake_home(&dir.join(name), version), source).unwrap()
    };
    let mojang = [rt("m21", "21.0.5", Source::Mojang)];
    let installed = [
        rt("i25", "25.0.1", Source::Installed),
        rt("i21", "21.0.3", Source::Installed),
        rt("i17", "17.0.12", Source::Installed),
    ];
    let picked = |n| pick(&need(n), None, &mojang, &installed).unwrap();
    assert_eq!(picked(21).unwrap().home, dir.join("m21"));
    assert_eq!(picked(17).unwrap().home, dir.join("i17"));
    // Java 8 is needed and only newer ones are here: Mojang's is to be downloaded.
    assert_eq!(picked(8), None);
    assert_eq!(pick(&need(8), None, &[], &[]).unwrap(), None);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_instances_own_java_wins_and_a_wrong_folder_says_so() {
    let dir = scratch("own");
    let own = fake_home(&dir.join("own"), "17.0.12");
    let picked = pick(&need(21), Some(&own), &[], &[]).unwrap().unwrap();
    assert_eq!((picked.home, picked.major), (own, 17));
    assert!(pick(&need(21), Some(&dir.join("nothing")), &[], &[])
        .unwrap_err()
        .contains("is not a Java home"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_typed_java_is_a_major_or_a_component() {
    assert_eq!(Need::named("21").unwrap().component, "java-runtime-delta");
    assert_eq!(Need::named(" jre-legacy ").unwrap().major, 8);
    assert!(Need::named("11")
        .unwrap_err()
        .contains("Known: 8 (jre-legacy)"));
}
