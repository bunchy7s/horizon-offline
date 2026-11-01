//! Filesystem-only regression tests; process queries and signature decisions are injected.
//! Real executable mappings and Keychain writes belong to the opt-in Python harness.
use super::*;
use std::os::unix::{fs::symlink, process::ExitStatusExt};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "horizon-mac-update-{}-{:x?}",
            std::process::id(),
            nonce
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn app(&self) -> PathBuf {
        self.0.join("Horizon.app")
    }
    fn retired(&self, generation: u32) -> PathBuf {
        self.0.join(format!(".Horizon.app.old-1-{generation:032x}"))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bundle(app: &Path, version: &str) {
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    fs::write(app.join("Contents/MacOS/horizon-launcher"), b"fixture").unwrap();
    fs::write(app.join("Contents/version"), version).unwrap();
}

fn version(app: &Path) -> String {
    fs::read_to_string(app.join("Contents/version")).unwrap()
}

fn retired(work: &Scratch) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(&work.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| is_retired(p.file_name().unwrap().to_str().unwrap(), "Horizon.app"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn only_an_unambiguous_empty_lsof_exit_one_allows_deletion() {
    for (code, stdout, stderr, unused) in [
        (1, "", "", true),
        (0, "123\n456\n", "", false),
        (0, "", "", false),
        (1, "123\n", "", false),
        (1, "", "warning", false),
        (1, "\n", "", false),
        (1, "", "\n", false),
        (2, "", "", false),
        (127, "", "", false),
    ] {
        assert_eq!(
            definitely_unused(Ok(Output {
                status: std::process::ExitStatus::from_raw(code << 8),
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
            })),
            unused,
            "exit={code}, stdout={stdout:?}, stderr={stderr:?}"
        );
    }
    assert!(!definitely_unused(Ok(Output {
        status: std::process::ExitStatus::from_raw(15),
        stdout: vec![],
        stderr: vec![],
    })));
    assert!(!definitely_unused(Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no lsof"
    ))));
}

#[test]
fn cleanup_names_are_exactly_our_app_positive_pid_and_lowercase_nonce() {
    let hex = "0123456789abcdef0123456789abcdef";
    for pid in ["1", "4294967295"] {
        assert!(is_retired(
            &format!(".Horizon.app.old-{pid}-{hex}"),
            "Horizon.app"
        ));
    }
    for suffix in [
        format!("0-{hex}"),
        format!("4294967296-{hex}"),
        format!("+1-{hex}"),
        format!("01-{hex}"),
        format!("-1-{hex}"),
        format!("1-{}", hex.to_uppercase()),
        "1-123".into(),
        format!("1-{hex}0"),
        format!("1-{hex}.app"),
        format!("1-{}g", &hex[..31]),
        "123".into(),
    ] {
        assert!(
            !is_retired(&format!(".Horizon.app.old-{suffix}"), "Horizon.app"),
            "{suffix}"
        );
    }
    assert!(!is_retired(
        &format!(".Other.app.old-1-{hex}"),
        "Horizon.app"
    ));
    assert!(!is_retired(
        &format!(".Horizon.app.update-1-{hex}"),
        "Horizon.app"
    ));
}

#[test]
fn cleanup_preserves_unknown_names_incomplete_bundles_and_every_symlink_component() {
    let work = Scratch::new();
    let outside = work.0.join("unrelated");
    bundle(&outside, "keep");
    let safe = work.retired(1);
    bundle(&safe, "unused");
    symlink(&outside, work.retired(2)).unwrap();
    for (n, component, target) in [
        (3, "Contents", outside.join("Contents")),
        (4, "Contents/MacOS", outside.join("Contents/MacOS")),
        (
            5,
            "Contents/MacOS/horizon-launcher",
            outside.join("Contents/MacOS/horizon-launcher"),
        ),
    ] {
        let root = work.retired(n);
        let link = root.join(component);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(target, link).unwrap();
    }
    let missing = work.retired(6);
    fs::create_dir_all(missing.join("Contents/MacOS")).unwrap();
    let directory = work.retired(7);
    fs::create_dir_all(directory.join("Contents/MacOS/horizon-launcher")).unwrap();
    for name in [
        ".Horizon.app.old-123",
        ".Horizon.app.old-1-not-a-nonce",
        ".Other.app.old-1-00000000000000000000000000000000",
    ] {
        bundle(&work.0.join(name), "keep");
    }
    fs::write(work.retired(8), b"not a bundle").unwrap();
    let before = fs::read_dir(&work.0).unwrap().count();
    let mut queried = vec![];
    cleanup_with(&work.app(), |exe| {
        queried.push(exe.to_path_buf());
        true
    });
    assert_eq!(queried, [safe.join("Contents/MacOS/horizon-launcher")]);
    assert_eq!(fs::read_dir(&work.0).unwrap().count(), before - 1);
    assert!(!safe.exists());
    assert_eq!(version(&outside), "keep");
}

#[test]
fn multiple_users_and_ambiguous_queries_keep_the_bundle_until_the_last_user_leaves() {
    let work = Scratch::new();
    let old = work.retired(1);
    bundle(&old, "v1");
    for users in [2, 1] {
        cleanup_with(&work.app(), |_| users == 0);
        assert_eq!(version(&old), "v1");
    }
    cleanup_with(&work.app(), |_| {
        definitely_unused(Err(io::Error::other("query failed")))
    });
    assert!(old.exists());
    cleanup_with(&work.app(), |_| true);
    assert!(!old.exists());
    cleanup_with(&work.app(), |_| panic!("already cleaned"));
}

#[test]
fn repeated_updates_retain_distinct_generations_and_query_their_actual_executables() {
    let work = Scratch::new();
    let app = work.app();
    let new = work.0.join("release.app");
    bundle(&app, "v1");
    bundle(&new, "v2");
    replace_bundle_with(&new, &app, |_| true, |_| false).unwrap();
    let first = retired(&work);
    assert_eq!(first.len(), 1);
    assert_eq!(version(&first[0]), "v1");
    fs::write(new.join("Contents/version"), "v3").unwrap();
    replace_bundle_with(&new, &app, |_| true, |_| false).unwrap();
    let both = retired(&work);
    assert_eq!(
        both.len(),
        2,
        "same updater PID must not overwrite the previous generation"
    );
    assert_eq!(
        fs::read_dir(&work.0).unwrap().count(),
        4,
        "no staging or failed residue"
    );
    assert_eq!(version(&app), "v3");
    let second = both.iter().find(|p| **p != first[0]).unwrap();
    assert_eq!(version(second), "v2");
    cleanup_with(&app, |exe| {
        assert_eq!(exe.file_name().unwrap(), "horizon-launcher");
        assert_eq!(exe.parent().unwrap().file_name().unwrap(), "MacOS");
        exe == second.join("Contents/MacOS/horizon-launcher")
    });
    assert_eq!(retired(&work), first);
    assert_eq!(version(&first[0]), "v1");
    cleanup_with(&app, |_| true);
    assert!(retired(&work).is_empty());
    assert_eq!(fs::read_dir(&work.0).unwrap().count(), 2);
}

#[test]
fn failed_signature_before_or_after_swap_keeps_the_original_and_cleans_only_its_own_staging() {
    for fail_check in [1, 2] {
        let work = Scratch::new();
        let app = work.app();
        let new = work.0.join("release.app");
        bundle(&app, "original");
        bundle(&new, "replacement");
        let unrelated = work.0.join(".Horizon.app.update-someone-elses-work");
        bundle(&unrelated, "preserve");
        let previous = work.retired(1);
        bundle(&previous, "previous generation still in use");
        let mut checks = vec![];
        let error = replace_bundle_with(
            &new,
            &app,
            |path| {
                checks.push(path.to_path_buf());
                checks.len() != fail_check
            },
            |_| panic!("a failed update must not clean unrelated generations"),
        )
        .unwrap_err();
        assert!(error.contains("signature"));
        assert_eq!(checks.len(), fail_check);
        if fail_check == 2 {
            assert_eq!(checks[1], app);
        }
        assert_eq!(version(&app), "original");
        assert_eq!(version(&new), "replacement");
        assert_eq!(version(&unrelated), "preserve");
        assert_eq!(version(&previous), "previous generation still in use");
        assert_eq!(
            fs::read_dir(&work.0).unwrap().count(),
            4,
            "no new staging/failed/old residue"
        );
    }
}

#[test]
fn a_concurrent_cleanup_during_final_signature_cannot_destroy_the_rollback() {
    for final_signature_holds in [false, true] {
        let work = Scratch::new();
        let app = work.app();
        let new = work.0.join("release.app");
        // An older updater is replacing v2, which has no executable users at all.
        bundle(&app, "v2");
        bundle(&new, "v3");
        let foreign = work
            .0
            .join(".Other.app.old-1-00000000000000000000000000000000");
        let quarantine = work
            .0
            .join(".Horizon.app.rollback-1-00000000000000000000000000000000");
        let other_staging = work.0.join(".Horizon.app.update-another-transaction");
        for path in [&foreign, &quarantine, &other_staging] {
            bundle(path, "unrelated");
        }
        let mut checks = 0;
        let mut backup_survived_collection = false;
        let result = replace_bundle_with(
            &new,
            &app,
            |path| {
                checks += 1;
                assert_eq!(version(path), "v3");
                if checks == 1 {
                    return true;
                }
                assert_eq!(path, app.as_path());
                // A startup collector runs after installation, before the final verdict.
                // Join fixes the interleaving; no timing assumptions or false lsof answer.
                std::thread::scope(|scope| {
                    scope.spawn(|| cleanup_with(&app, |_| true)).join().unwrap();
                });
                // Identify the backup by its data, not by the fix's chosen namespace.
                backup_survived_collection = fs::read_dir(&work.0).unwrap().any(|entry| {
                    fs::read_to_string(entry.unwrap().path().join("Contents/version"))
                        .is_ok_and(|v| v == "v2")
                });
                final_signature_holds
            },
            |_| false,
        );
        assert_eq!(checks, 2);
        if !final_signature_holds {
            assert!(result.unwrap_err().contains("signature"));
            assert_eq!(
                fs::read_to_string(app.join("Contents/version"))
                    .ok()
                    .as_deref(),
                Some("v2"),
                "a concurrent collector must not destroy the only rollback source"
            );
            assert!(retired(&work).is_empty());
        } else {
            result.unwrap();
            assert_eq!(version(&app), "v3");
            let published = retired(&work);
            assert_eq!(
                published.len(),
                1,
                "publish a collectable generation only after success"
            );
            assert_eq!(version(&published[0]), "v2");
            cleanup_with(&app, |_| true);
            assert!(
                !published[0].exists(),
                "the committed, unused generation is collectable"
            );
        }
        assert!(
            backup_survived_collection,
            "the transaction still needs its backup before the verdict"
        );
        assert!(!is_retired(
            quarantine.file_name().unwrap().to_str().unwrap(),
            "Horizon.app"
        ));
        assert_eq!(version(&new), "v3");
        for path in [&foreign, &quarantine, &other_staging] {
            assert_eq!(version(path), "unrelated");
        }
        assert_eq!(
            fs::read_dir(&work.0).unwrap().count(),
            5,
            "no new transaction residue"
        );
    }
}
