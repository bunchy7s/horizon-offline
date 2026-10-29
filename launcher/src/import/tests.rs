use super::*;
use crate::export::{export, Format};
use std::io::Write;
use std::time::Duration;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "horizon-launcher-import-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn zip_file(path: &Path, files: &[(&str, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Makes instances as folders of `root`, refusing a name already taken, as the launcher does.
fn maker(root: &Path) -> impl Fn(&str) -> Result<Instance, String> + '_ {
    move |name: &str| {
        let home = root.join(crate::modpacks::slug(name));
        if home.exists() {
            return Err(format!("There is already an instance called {name}."));
        }
        fs::create_dir_all(&home).unwrap();
        Ok(Instance {
            id: crate::modpacks::slug(name),
            name: name.into(),
            kind: instances::Kind::Own,
            work: home.clone(),
            home,
            icon: None,
            group: None,
            version: None,
            played: Duration::ZERO,
            last_played: None,
            launch: Default::default(),
            java: None,
        })
    }
}

const NONE: &dyn Fn(&str, f32) = &|_, _| {};

#[test]
fn says_what_a_dropped_file_is() {
    let dir = scratch("what");
    let mrpack = |name: &str, deps: &str| {
        let path = dir.join(name);
        let index = format!(r#"{{"formatVersion":1,"name":"P","files":[],"dependencies":{deps}}}"#);
        zip_file(&path, &[("modrinth.index.json", index.as_bytes())]);
        path
    };
    let cases: Vec<(PathBuf, Result<Dropped, ()>)> = vec![
        (
            mrpack("shared.mrpack", r#"{"minecraft":"26.3"}"#),
            Ok(Dropped::Instance),
        ),
        (
            mrpack(
                "java.mrpack",
                r#"{"minecraft":"1.21.1","fabric-loader":"0.16"}"#,
            ),
            Ok(Dropped::Modpack),
        ),
        {
            let p = dir.join("cf.zip");
            zip_file(
                &p,
                &[("manifest.json", b"{}"), ("overrides/config/a.toml", b"")],
            );
            (p, Ok(Dropped::Modpack))
        },
        {
            let p = dir.join("rp.zip");
            zip_file(&p, &[("pack.mcmeta", b"{}"), ("assets/a.png", b"")]);
            (p, Ok(Dropped::Pack(packs::Kind::Resource)))
        },
        {
            let p = dir.join("dp.zip");
            zip_file(
                &p,
                &[
                    ("pack.mcmeta", b"{}"),
                    ("data/a/function/f.mcfunction", b""),
                ],
            );
            (p, Ok(Dropped::DataPack))
        },
        {
            let p = dir.join("sp.zip");
            zip_file(&p, &[("shaders/final.fsh", b"")]);
            (p, Ok(Dropped::Pack(packs::Kind::Shader)))
        },
        {
            let p = dir.join("glow.hshader");
            put(&p, b"PK");
            (p, Ok(Dropped::Pack(packs::Kind::Shader)))
        },
        {
            let p = dir.join("folder.zip");
            zip_file(
                &p,
                &[("options.txt", b"lang:fr_ch\n"), ("saves/W/level.dat", b"")],
            );
            (p, Ok(Dropped::Instance))
        },
        {
            let p = dir.join("random.zip");
            zip_file(&p, &[("readme.txt", b"")]);
            (p, Err(()))
        },
        {
            let p = dir.join("picture.png");
            put(&p, b"png");
            (p, Err(()))
        },
    ];
    for (file, want) in cases {
        assert_eq!(what(&file).map_err(|_| ()), want, "{}", file.display());
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_export_comes_back_as_an_instance_and_nothing_escapes_its_folder() {
    let dir = scratch("roundtrip");
    let home = dir.join("from");
    put(&home.join("options.txt"), b"lang:fr_ch\n");
    put(&home.join("resourcepacks/Glass.zip"), b"glass");
    put(
        &home.join("horizon-mods/coords/horizon-mod.toml"),
        b"id = \"coords\"\n",
    );
    put(&home.join("saves/Base/region/r.0.0.mca"), b"region");
    put(&home.join("saves/Base/level.dat"), b"not nbt");
    let made = dir.join("instances");
    for format in [Format::Mrpack, Format::Zip] {
        let out = dir.join(format!("Shared.{}", format.extension()));
        export(&home, "Shared", format, &["Base".into()], &[], &out).unwrap();
        let i = import_into(&out, NONE, &maker(&made)).unwrap();
        for f in [
            "options.txt",
            "resourcepacks/Glass.zip",
            "horizon-mods/coords/horizon-mod.toml",
            "saves/Base/region/r.0.0.mca",
        ] {
            assert_eq!(
                fs::read(i.home.join(f)).unwrap(),
                fs::read(home.join(f)).unwrap(),
                "{f}"
            );
        }
    }
    // The .mrpack took the pack's name; the zip came second and got "(2)".
    assert!(made.join("shared").is_dir() && made.join("shared-2").is_dir());

    // A zip that reaches out of its folder, or carries what an instance doesn't keep.
    let evil = dir.join("evil.zip");
    zip_file(
        &evil,
        &[
            ("options.txt", b"ok"),
            ("../outside.txt", b"no"),
            ("saves/../../up.txt", b"no"),
            ("logs/latest.log", b"no"),
            ("session.json", b"no"),
        ],
    );
    let i = import_into(&evil, NONE, &maker(&made)).unwrap();
    assert!(i.home.join("options.txt").is_file());
    assert!(!dir.join("outside.txt").exists() && !made.join("up.txt").exists());
    assert!(!i.home.join("logs").exists() && !i.home.join("session.json").exists());

    // A file to download from anywhere but Modrinth is refused, and the half-made folder goes.
    let index = r#"{"formatVersion":1,"name":"Far","dependencies":{"minecraft":"26.3"},
        "files":[{"path":"resourcepacks/x.zip","hashes":{"sha1":"00","sha512":"00"},
        "downloads":["https://example.com/x.zip"],"fileSize":1}]}"#;
    let far = dir.join("far.mrpack");
    zip_file(&far, &[("modrinth.index.json", index.as_bytes())]);
    let refused = import_into(&far, NONE, &maker(&made)).unwrap_err();
    assert!(refused.contains("resourcepacks/x.zip"), "{refused}");
    assert!(!made.join("far").exists());

    let java = dir.join("java.mrpack");
    zip_file(
        &java,
        &[(
            "modrinth.index.json",
            br#"{"name":"J","files":[],"dependencies":{"neoforge":"21"}}"#,
        )],
    );
    assert!(import_into(&java, NONE, &maker(&made)).is_err());
    fs::remove_dir_all(&dir).unwrap();
}
