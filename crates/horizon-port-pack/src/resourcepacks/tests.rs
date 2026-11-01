use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "horizon-resourcepacks-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn work(&self) -> PathBuf {
        self.0.join("work")
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}
fn imported(source: &Path, work: &Path) -> Result<Report, String> {
    pipeline::import(source, work, &|_| {})?;
    Ok(Report::load(work))
}

#[test]
fn mrpack_client_override_wins_and_server_and_unrelated_overrides_stay_out() {
    let t = Temp::new();
    let src = t.0.join("pack.mrpack");
    archive(
        &src,
        &[
            (
                "modrinth.index.json",
                br#"{"name":"Small","files":[{"path":"shaderpacks/later.zip"}]}"#,
            ),
            ("overrides/resourcepacks/look.zip", b"common"),
            ("client-overrides/resourcepacks/look.zip", b"client"),
            ("server-overrides/resourcepacks/server.zip", b"server"),
            ("overrides/options.txt", b"do not copy"),
            ("overrides/resourcepacks/README.txt", b"not a pack"),
        ],
    );
    let report = imported(&src, &t.work()).unwrap();
    assert_eq!(
        fs::read(t.work().join("resourcepacks/look.zip")).unwrap(),
        b"client"
    );
    assert!(!t.work().join("resourcepacks/server.zip").exists());
    assert!(!t.work().join("options.txt").exists());
    assert!(!t.work().join("resourcepacks/README.txt").exists());
    assert_eq!(
        report.files["resourcepacks/look.zip"].state,
        State::Available
    );
    assert_eq!(
        report.files["shaderpacks/later.zip"].state,
        State::Unsupported
    );
}

#[test]
fn curseforge_custom_root_and_directory_pack() {
    let t = Temp::new();
    let src = t.0.join("pack.zip");
    archive(
        &src,
        &[
            (
                "manifest.json",
                br#"{"name":"Custom","overrides":"custom/root","files":[]}"#,
            ),
            ("custom/root/resourcepacks/Pretty/pack.mcmeta", b"{}"),
            (
                "custom/root/resourcepacks/Pretty/assets/test/a.txt",
                b"texture",
            ),
        ],
    );
    imported(&src, &t.work()).unwrap();
    assert_eq!(
        fs::read(t.work().join("resourcepacks/Pretty/assets/test/a.txt")).unwrap(),
        b"texture"
    );
    assert_eq!(available(&t.work()).len(), 1);
}

#[test]
fn resource_only_instance_and_repeated_import_keep_edits() {
    let t = Temp::new();
    let src = t.0.join("instance");
    put(&src.join("mmc-pack.json"), b"{}");
    put(
        &src.join(".minecraft/resourcepacks/Look/pack.mcmeta"),
        b"{}",
    );
    put(&src.join(".minecraft/resourcepacks/look.zip"), b"zip");
    imported(&src, &t.work()).unwrap();
    put(&t.work().join("resourcepacks/look.zip"), b"local edit");
    let report = imported(&src, &t.work()).unwrap();
    assert_eq!(
        report.files["resourcepacks/look.zip"].state,
        State::Conflict
    );
    assert_eq!(
        fs::read(t.work().join("resourcepacks/look.zip")).unwrap(),
        b"local edit"
    );
    assert_eq!(available(&t.work()).len(), 2);
}

#[test]
fn hostile_archive_paths_and_cross_layer_collisions_are_rejected_before_staging() {
    for name in [
        "../escape.zip",
        "/absolute.zip",
        "C:drive.zip",
        "a\\..\\escape.zip",
        "CON.zip",
        "space.zip ",
    ] {
        let t = Temp::new();
        let src = t.0.join("bad.mrpack");
        archive(
            &src,
            &[
                ("modrinth.index.json", b"{}"),
                (&format!("overrides/resourcepacks/{name}"), b"bad"),
            ],
        );
        // A bad entry leaves the packs out; it no longer fails the import.
        assert!(imported(&src, &t.work()).is_ok(), "{name}");
        assert!(available(&t.work()).is_empty());
        assert!(!t.work().join("escape.zip").exists());
    }
    let t = Temp::new();
    let src = t.0.join("bad.mrpack");
    archive(
        &src,
        &[
            ("modrinth.index.json", b"{}"),
            ("overrides/resourcepacks/a.zip", b"first"),
            (
                "client-overrides/resourcepacks/a.zip/pack.mcmeta",
                b"second",
            ),
        ],
    );
    assert!(imported(&src, &t.work()).is_ok());
    assert!(available(&t.work()).is_empty());
}

#[test]
fn duplicate_names_case_aliases_and_byte_limits() {
    let mut files = Files::new();
    add(&mut files, "A.zip", &b"one"[..], &mut 0).unwrap();
    assert!(add(&mut files, "A.zip", &b"two"[..], &mut 0).is_err());
    add(&mut files, "a.zip", &b"two"[..], &mut 0).unwrap();
    assert!(validate(&files).is_err());
    assert!(read_limited(&b"12345"[..], 4).is_err());
    assert_eq!(read_limited(&b"1234"[..], 4).unwrap(), b"1234");
}

#[test]
fn profile_copy_is_idempotent_and_preserves_whole_directory_edits() {
    let t = Temp::new();
    put(&t.work().join("resourcepacks/Look/pack.mcmeta"), b"{}");
    put(&t.work().join("resourcepacks/Look/assets/a"), b"original");
    put(&t.work().join("resourcepacks/look.zip"), b"zip");
    let home = t.0.join("home");
    let first = install(&t.work(), &home).unwrap();
    assert_eq!(first.files.len(), 2);
    put(&home.join("resourcepacks/Look/assets/a"), b"edit");
    let second = install(&t.work(), &home).unwrap();
    assert_eq!(second.files["resourcepacks/Look"].state, State::Conflict);
    assert_eq!(
        second.files["resourcepacks/look.zip"].state,
        State::Available
    );
    assert_eq!(
        fs::read(home.join("resourcepacks/Look/assets/a")).unwrap(),
        b"edit"
    );
    assert!(!home.join("options.txt").exists());
}

#[cfg(unix)]
#[test]
fn symlinks_at_source_and_destination_are_rejected() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let src = t.0.join("instance");
    put(&src.join("resourcepacks/good.zip"), b"good");
    put(&t.0.join("outside.zip"), b"outside");
    symlink(t.0.join("outside.zip"), src.join("resourcepacks/link.zip")).unwrap();
    // A bad entry leaves the packs out; it no longer fails the import.
    assert!(imported(&src, &t.work()).is_ok());
    fs::remove_file(src.join("resourcepacks/link.zip")).unwrap();
    imported(&src, &t.work()).unwrap();
    let home = t.0.join("home");
    fs::create_dir(&home).unwrap();
    symlink(&src, home.join("resourcepacks")).unwrap();
    assert!(install(&t.work(), &home).is_err());
    assert_eq!(
        fs::read(src.join("resourcepacks/good.zip")).unwrap(),
        b"good"
    );
}

#[cfg(unix)]
#[test]
fn a_home_behind_a_link_takes_its_packs() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let src = t.0.join("instance");
    put(&src.join("resourcepacks/good.zip"), b"good");
    // Silverblue's /home -> /var/home: the link is above the work folder and the home.
    fs::create_dir(t.0.join("var-home")).unwrap();
    symlink(t.0.join("var-home"), t.0.join("home")).unwrap();
    let work = t.0.join("home/work");
    imported(&src, &work).unwrap();
    let home = t.0.join("home/profile");
    let report = install(&work, &home).unwrap();
    assert_eq!(
        report.files["resourcepacks/good.zip"].state,
        State::Available
    );
    assert_eq!(
        fs::read(t.0.join("var-home/profile/resourcepacks/good.zip")).unwrap(),
        b"good"
    );
}

#[cfg(unix)]
#[test]
fn a_link_below_the_base_is_refused() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let base = t.0.join("home");
    fs::create_dir(&base).unwrap();
    fs::create_dir(t.0.join("outside")).unwrap();
    symlink(t.0.join("outside"), base.join("resourcepacks")).unwrap();
    assert!(directory(&base, &base.join("resourcepacks/Look")).is_err());
    assert!(!t.0.join("outside/Look").exists());
    // The same layout one level up is the player's own: accepted.
    assert!(
        directory(
            &base.join("resourcepacks"),
            &base.join("resourcepacks/Look")
        )
        .is_ok()
    );
    assert!(t.0.join("outside/Look").is_dir());
}

#[test]
fn indexed_downloads_are_hash_checked_and_failures_do_not_discard_other_packs() {
    use import::PackFile;
    let t = Temp::new();
    fs::create_dir(t.work()).unwrap();
    let file = |name: &str| PackFile {
        path: format!("resourcepacks/{name}.zip"),
        sha1: Some(import::sha1_hex(b"pack")),
        downloads: vec![format!("https://cdn.modrinth.com/{name}.zip")],
        ..PackFile::default()
    };
    let mut excluded = file("server");
    excluded.client = "unsupported".into();
    let mut missing = file("missing");
    missing.downloads = vec!["https://untrusted.example/pack.zip".into()];
    let mut no_hash = file("nohash");
    no_hash.sha1 = None;
    let pack = import::Pack {
        files: vec![file("good"), file("bad"), excluded, missing, no_hash],
        ..import::Pack::default()
    };
    pipeline::write_json(&t.work().join("pack.json"), &pack).unwrap();
    let report = fetch_with(&t.work(), &|_| {}, &|url, hash, dest| {
        let bytes: &[u8] = if url.ends_with("bad.zip") {
            b"corrupt"
        } else {
            b"pack"
        };
        crate::fetch::store_download(bytes, Some(hash), dest, false)
    })
    .unwrap();
    assert_eq!(
        report.files["resourcepacks/good.zip"].state,
        State::Available
    );
    assert_eq!(report.files["resourcepacks/bad.zip"].state, State::Failed);
    assert_eq!(
        report.files["resourcepacks/server.zip"].state,
        State::Unsupported
    );
    assert_eq!(
        report.files["resourcepacks/missing.zip"].state,
        State::Missing
    );
    assert_eq!(
        report.files["resourcepacks/nohash.zip"].state,
        State::Failed
    );
    assert!(!t.work().join("resourcepacks/bad.zip").exists());
    let again = fetch_with(&t.work(), &|_| {}, &|_, _, _| Err("offline".into())).unwrap();
    assert_eq!(
        again.files["resourcepacks/good.zip"].state,
        State::Available
    );
    put(&t.work().join("resourcepacks/good.zip"), b"user edit");
    let again = fetch_with(&t.work(), &|_| {}, &|url, _, _| {
        assert!(
            !url.ends_with("good.zip"),
            "must not download over an existing file"
        );
        Err("offline".into())
    })
    .unwrap();
    assert_eq!(again.files["resourcepacks/good.zip"].state, State::Conflict);
    assert_eq!(
        fs::read(t.work().join("resourcepacks/good.zip")).unwrap(),
        b"user edit"
    );
}

#[test]
fn interrupted_downloads_publish_nothing_and_keep_existing_files() {
    struct Interrupted(bool);
    impl Read for Interrupted {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if !self.0 {
                self.0 = true;
                out[0] = b'x';
                return Ok(1);
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "interrupted",
            ))
        }
    }
    let t = Temp::new();
    let dest = t.0.join("pack.zip");
    assert!(crate::fetch::store_download(Interrupted(false), None, &dest, false).is_err());
    assert!(!dest.exists());
    put(&dest, b"existing");
    assert!(
        crate::fetch::store_download(&b"bad"[..], Some(&import::sha1_hex(b"good")), &dest, false)
            .is_err()
    );
    assert_eq!(fs::read(&dest).unwrap(), b"existing");
    assert!(
        crate::fetch::store_download(&b"new"[..], Some(&import::sha1_hex(b"new")), &dest, false)
            .is_err()
    );
    assert_eq!(fs::read(&dest).unwrap(), b"existing");
    assert_eq!(fs::read_dir(&t.0).unwrap().count(), 1);
}

#[test]
fn reports_include_embedded_packs_and_never_count_shaders_as_working() {
    let t = Temp::new();
    let src = t.0.join("report.mrpack");
    archive(&src, &[
        ("modrinth.index.json", br#"{"files":[{"path":"resourcepacks/missing.zip"},{"path":"shaderpacks/later.zip"}]}"#),
        ("overrides/resourcepacks/look.zip", b"original"),
    ]);
    imported(&src, &t.work()).unwrap();
    pipeline::inventory(&t.work(), true, &|_| {}).unwrap();
    let rows = pipeline::report(&t.work(), &|_| {}).unwrap();
    assert_eq!(rows.len(), 3);
    let row = |path: &str| rows.iter().find(|r| r.key == path).unwrap();
    assert_eq!(
        row("resourcepacks/look.zip").status,
        crate::report::Status::Partial
    );
    assert!(
        row("resourcepacks/look.zip")
            .reason
            .contains("rendering not checked")
    );
    assert_eq!(
        row("shaderpacks/later.zip").status,
        crate::report::Status::NotYet
    );
    assert_eq!(
        row("resourcepacks/missing.zip").status,
        crate::report::Status::NotYet
    );
    // An old work directory without resourcepacks.json remains readable.
    fs::remove_file(t.work().join("resourcepacks.json")).unwrap();
    assert!(pipeline::report(&t.work(), &|_| {}).is_ok());
}

#[test]
fn duplicate_archive_entries_are_rejected() {
    let t = Temp::new();
    let src = t.0.join("duplicate.mrpack");
    archive(
        &src,
        &[
            ("modrinth.index.json", b"{}"),
            ("overrides/resourcepacks/a.zip", b"first"),
            ("overrides/resourcepacks/b.zip", b"second"),
        ],
    );
    // Rename both the local and central ZIP records; ZipWriter otherwise refuses duplicates.
    let mut bytes = fs::read(&src).unwrap();
    for i in 0..bytes.len().saturating_sub(4) {
        if &bytes[i..i + 5] == b"b.zip" {
            bytes[i] = b'a';
        }
    }
    fs::write(&src, bytes).unwrap();
    // A bad entry leaves the packs out; it no longer fails the import.
    assert!(imported(&src, &t.work()).is_ok());
    assert!(available(&t.work()).is_empty());
}

#[test]
fn archive_symlinks_are_refused_and_file_counts_are_bounded() {
    let t = Temp::new();
    let src = t.0.join("link.mrpack");
    let mut zip = zip::ZipWriter::new(fs::File::create(&src).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("modrinth.index.json", options).unwrap();
    zip.write_all(b"{}").unwrap();
    zip.add_symlink(
        "overrides/resourcepacks/link.zip",
        "../../outside.zip",
        options,
    )
    .unwrap();
    zip.finish().unwrap();
    // A bad entry leaves the packs out; it no longer fails the import.
    assert!(imported(&src, &t.work()).is_ok());
    assert!(available(&t.work()).is_empty());
    let mut files: Files = (0..MAX_FILES)
        .map(|n| (format!("pack/{n}"), Vec::new()))
        .collect();
    assert!(add(&mut files, "pack/one-more", &b""[..], &mut 0).is_err());
}
