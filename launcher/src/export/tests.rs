use super::*;
use std::collections::BTreeMap;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "horizon-launcher-export-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// A gzipped level.dat whose player has a UUID.
fn level_dat() -> Vec<u8> {
    let mut player = BTreeMap::new();
    player.insert("UUID".into(), Tag::IntArray(vec![1, 2, 3, 4]));
    player.insert("XpLevel".into(), Tag::Int(7));
    let mut data = BTreeMap::new();
    data.insert("LevelName".into(), Tag::String("Base".into()));
    data.insert("Player".into(), Tag::Compound(player));
    let mut root = BTreeMap::new();
    root.insert("Data".into(), Tag::Compound(data));
    let raw = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: Tag::Compound(root),
        },
        Limits::default(),
    )
    .unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

fn sha512(bytes: &[u8]) -> String {
    sha2::Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A game folder with settings, packs, a mod, two worlds, and what never goes out.
fn home(name: &str) -> PathBuf {
    let home = scratch(name);
    put(&home.join("options.txt"), b"lang:fr_ch\n");
    put(&home.join("horizon-graphics.txt"), b"preset:high\n");
    put(&home.join("servers.dat"), b"\x0a\x00\x00\x00");
    put(&home.join("resourcepacks/Glass.zip"), b"glass pack");
    put(&home.join("resourcepacks/Own.zip"), b"own pack");
    put(
        &home.join("horizon-mods/coords/horizon-mod.toml"),
        b"id = \"coords\"\n",
    );
    put(&home.join("saves/Base/level.dat"), &level_dat());
    put(&home.join("saves/Base/region/r.0.0.mca"), b"region");
    put(&home.join("saves/Base/playerdata/0001.dat"), b"me");
    put(&home.join("saves/Base/stats/0001.json"), b"{}");
    put(&home.join("saves/Base/session.lock"), b"");
    put(&home.join("saves/Other/level.dat"), &level_dat());
    for private in [
        "logs/latest.log",
        "screenshots/a.png",
        "backups/b.zip",
        "mod-browser/checks-1/x/check.json",
        "horizon-player.txt",
        "session.json",
        "usercache.json",
        "resourcepacks/.DS_Store",
    ] {
        put(&home.join(private), b"private");
    }
    let entry = serde_json::json!({ "entries": [{
        "source": "modrinth", "project": "P1", "slug": "glass", "title": "Glass",
        "kind": "resource-pack", "version": "V1", "number": "1.0",
        "file": "resourcepacks/Glass.zip",
        "checksum": format!("SHA-512:{}", sha512(b"glass pack")),
        "page": "https://modrinth.com/resourcepack/glass",
    }]});
    put(
        &home.join("mod-browser.json"),
        &serde_json::to_vec(&entry).unwrap(),
    );
    home
}

fn names(zip: &Path) -> Vec<String> {
    let zip = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    let mut names: Vec<String> = zip
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    names.sort();
    names
}

fn read(zip: &Path, name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    let mut out = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut out).unwrap();
    out
}

fn served(e: &Entry) -> Option<FileRef> {
    (e.version == "V1").then(|| FileRef {
        url: "https://cdn.modrinth.com/data/P1/versions/V1/Glass.zip".into(),
        name: "Glass.zip".into(),
        size: 10,
        sha512: Some(sha512(b"glass pack")),
        sha256: None,
        sha1: Some("ab".repeat(20)),
        primary: true,
    })
}

#[test]
fn a_zip_has_the_instance_and_the_picked_world_and_nothing_private() {
    let home = home("zip");
    let out = home.join("out/Hardcore.zip");
    let done = export(&home, "Hardcore", Format::Zip, &["Base".into()], &[], &out).unwrap();
    assert_eq!((done.copied, done.linked), (8, 0));
    // Settings, packs, the mod and the picked world, without its per-player files; no logs,
    // screenshots, backups, caches, account files or the other world.
    assert_eq!(
        names(&out),
        [
            "horizon-graphics.txt",
            "horizon-mods/coords/horizon-mod.toml",
            "options.txt",
            "resourcepacks/Glass.zip",
            "resourcepacks/Own.zip",
            "saves/Base/level.dat",
            "saves/Base/region/r.0.0.mca",
            "servers.dat",
        ]
    );
    // The world's level.dat keeps the player, without the UUID.
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&read(&out, "saves/Base/level.dat")[..])
        .read_to_end(&mut raw)
        .unwrap();
    let (root, _) = nbt::read_named(&raw, Limits::default()).unwrap();
    let Some(Tag::Compound(data)) = root.tag.get("Data") else {
        panic!("no Data")
    };
    let Some(Tag::Compound(player)) = data.get(&"Player".into()) else {
        panic!("no Player")
    };
    assert!(player.get(&"UUID".into()).is_none());
    assert_eq!(player.get(&"XpLevel".into()), Some(&Tag::Int(7)));
    assert!(!out.with_extension("part").exists());

    assert!(export(&home, "Hardcore", Format::Zip, &["../x".into()], &[], &out).is_err());
    let empty = scratch("empty");
    let refused = export(&empty, "New", Format::Zip, &[], &[], &empty.join("New.zip"));
    assert_eq!(
        refused.unwrap_err(),
        "New has nothing to share yet: no settings, packs, mods or worlds."
    );
    assert!(!empty.join("New.zip").exists());
    fs::remove_dir_all(&empty).unwrap();
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn an_mrpack_lists_what_modrinth_serves_and_carries_the_rest() {
    let home = home("mrpack");
    let linked = linkable(&home, &served);
    assert_eq!(linked.len(), 1);
    let out = free_path(&home, "Hard: core", Format::Mrpack);
    assert_eq!(out.file_name().unwrap(), "Hard- core.mrpack");
    let done = export(&home, "Hardcore", Format::Mrpack, &[], &linked, &out).unwrap();
    assert_eq!((done.copied, done.linked), (5, 1));
    assert_eq!(
        names(&out),
        [
            "modrinth.index.json",
            "overrides/horizon-graphics.txt",
            "overrides/horizon-mods/coords/horizon-mod.toml",
            "overrides/options.txt",
            "overrides/resourcepacks/Own.zip",
            "overrides/servers.dat",
        ]
    );
    let index: serde_json::Value =
        serde_json::from_slice(&read(&out, "modrinth.index.json")).unwrap();
    assert_eq!(index["formatVersion"], 1);
    assert_eq!(index["game"], "minecraft");
    assert_eq!(index["dependencies"]["minecraft"], GAME);
    let file = &index["files"][0];
    assert_eq!(file["path"], "resourcepacks/Glass.zip");
    assert_eq!(
        file["downloads"][0],
        "https://cdn.modrinth.com/data/P1/versions/V1/Glass.zip"
    );
    assert_eq!(file["hashes"]["sha512"], sha512(b"glass pack"));
    assert_eq!(
        free_path(&home, "Hard: core", Format::Mrpack)
            .file_name()
            .unwrap(),
        "Hard- core (2).mrpack"
    );

    // A pack changed since its install is carried, not linked.
    put(&home.join("resourcepacks/Glass.zip"), b"edited");
    assert!(linkable(&home, &served).is_empty());
    fs::remove_dir_all(&home).unwrap();
}
