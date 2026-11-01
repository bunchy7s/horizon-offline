use super::*;
use chrono::TimeZone;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "horizon-launcher-worlds-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A world as the game saves it: a gzip level.dat with `data`'s fields under `Data`.
fn world(home: &Path, folder: &str, data: Vec<(&str, Tag)>) -> PathBuf {
    let dir = home.join("saves").join(folder);
    fs::create_dir_all(dir.join("region")).unwrap();
    let root = Tag::compound([("Data", Tag::compound(data))]);
    let bytes = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: root,
        },
        Limits::default(),
    )
    .unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&bytes).unwrap();
    fs::write(dir.join("level.dat"), gz.finish().unwrap()).unwrap();
    fs::write(dir.join("region/r.0.0.mca"), [7u8; 1000]).unwrap();
    fs::write(dir.join(LOCK), "lock").unwrap();
    dir
}

fn name(n: &str) -> Tag {
    Tag::String(n.into())
}

#[test]
fn lists_what_each_world_is() {
    let home = scratch("list");
    let survival = world(
        &home,
        "Survival",
        vec![
            ("LevelName", name("My survival")),
            ("GameType", Tag::Int(0)),
            ("LastPlayed", Tag::Long(1_700_000_000_000)),
            ("Version", Tag::compound([("Name", name("26.3"))])),
        ],
    );
    fs::write(survival.join("icon.png"), "png").unwrap();
    world(
        &home,
        "Hard",
        vec![
            ("LevelName", name("Hard")),
            ("GameType", Tag::Int(0)),
            ("LastPlayed", Tag::Long(1_800_000_000_000)),
            (
                "difficulty_settings",
                Tag::compound([("hardcore", Tag::Byte(1))]),
            ),
        ],
    );
    world(
        &home,
        "Old",
        vec![
            ("GameType", Tag::Int(1)),
            ("hardcore", Tag::Byte(0)),
            ("LastPlayed", Tag::Long(1_600_000_000_000)),
        ],
    );
    fs::create_dir_all(home.join("saves/not a world")).unwrap();

    let all = list(&home);
    let names: Vec<&str> = all.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Hard", "My survival", "Old"]);
    assert_eq!(all[0].mode, "Hardcore");
    assert_eq!(all[1].mode, "Survival");
    assert_eq!(all[1].folder, "Survival");
    assert_eq!(all[1].version.as_deref(), Some("26.3"));
    assert_eq!(all[1].icon, Some(survival.join("icon.png")));
    assert_eq!(
        all[1].last_played,
        UNIX_EPOCH + Duration::from_millis(1_700_000_000_000)
    );
    assert!(all[1].size >= 1000);
    // No LevelName: the folder's name.
    assert_eq!(all[2].mode, "Creative");
    assert_eq!(find(&home, "my SURVIVAL").unwrap().folder, "Survival");
    assert!(find(&home, "nope").is_err());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_backup_is_named_and_laid_out_as_vanillas() {
    let home = scratch("backup");
    world(&home, "World", vec![("LevelName", name("World"))]);
    let w = find(&home, "World").unwrap();
    let at = chrono::Local
        .with_ymd_and_hms(2026, 10, 27, 1, 2, 3)
        .unwrap();
    let first = backup_at(&home, &w, at).unwrap();
    assert_eq!(first, home.join("backups/2026-10-27_01-02-03_World.zip"));
    // The same second again: the next free name.
    let second = backup_at(&home, &w, at).unwrap();
    assert_eq!(
        second,
        home.join("backups/2026-10-27_01-02-03_World (1).zip")
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(&first).unwrap()).unwrap();
    let mut entries: Vec<String> = zip
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, ["World/level.dat", "World/region/r.0.0.mca"]);
    let mut region = Vec::new();
    zip.by_name("World/region/r.0.0.mca")
        .unwrap()
        .read_to_end(&mut region)
        .unwrap();
    assert_eq!(region, [7u8; 1000]);
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_copy_gets_the_next_free_name_in_its_folder_and_level_dat() {
    let home = scratch("copy");
    world(
        &home,
        "Base",
        vec![("LevelName", name("Base")), ("GameType", Tag::Int(1))],
    );
    let base = find(&home, "Base").unwrap();
    let copy1 = copy(&base).unwrap();
    assert_eq!(copy1.folder, "Base (1)");
    assert_eq!(copy1.name, "Base (1)");
    assert_eq!(copy1.mode, "Creative");
    assert!(!copy1.path.join(LOCK).exists());
    assert_eq!(
        fs::read(copy1.path.join("region/r.0.0.mca")).unwrap(),
        [7u8; 1000]
    );
    assert_eq!(copy(&base).unwrap().folder, "Base (2)");
    // The original is untouched.
    assert_eq!(find(&home, "Base").unwrap(), base);
    // A name that can't be a folder name is made safe.
    assert_eq!(free_name(&home, "a/b: c?", "", 0), "a_b_ c_");
    // A world whose folder isn't its name: the copy still gets a new name.
    world(&home, "f", vec![("LevelName", name("Named"))]);
    let named = copy(&find(&home, "f").unwrap()).unwrap();
    assert_eq!(
        (named.folder.as_str(), named.name.as_str()),
        ("Named (1)", "Named (1)")
    );
    fs::remove_dir_all(&home).unwrap();
}
