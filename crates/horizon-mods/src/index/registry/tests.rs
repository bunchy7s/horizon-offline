//! A registry checkout holding the sample index's mod, modpack and shader pack
//! builds the sample index (`tests/index/index-v1.json`) exactly.
use super::*;

const HMOD: &str = "e1135345692f6dd8fd4c51920151944b82333e17049c9bb7d756f27b8deefc53";
const HPACK: &str = "241ac840f9c5ea3f6252d849da10c90d36cb1da4d6cd8d1d268c2760a353cd65";
const HSHADER: &str = "331cfe29a666a0de804e9a9ff3d19d246eebcfbfa70b4f49bba8482d30ad9f02";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/index")
}

fn sample() -> Index {
    Index::parse(&std::fs::read_to_string(fixtures().join("index-v1.json")).unwrap()).unwrap()
}

fn hud_entry() -> String {
    let p = sample();
    let sig = p.project("coords_hud").unwrap().versions[0]
        .file
        .signature
        .clone()
        .unwrap();
    format!(
        r#"id = "coords_hud"
kind = "mod"
name = "Coordinates HUD"
summary = "Your coordinates, facing and the time of day in a corner of the screen."
description = "`/where` puts them in chat, `/coords` hides or shows the panel."
authors = ["Horizon"]
owners = ["ludovic111"]
license = "MIT"
source = "https://github.com/horizon-rust/horizon/tree/main/crates/horizon-mod-sdk/examples/coords-hud"
icon = "icons/coords_hud.png"
key = "RWTVtuGb/GRXcBf3Re5hzM07EzoStr2sLTJjghJia0xX15ULoZRFFZDb"

[[versions]]
version = "0.1.0"
published = "2026-10-24"
game = "26.3"
file = "coords_hud-0.1.0.hmod"
sha256 = "{HMOD}"
signature = """{sig}"""
"#
    )
}

const COZY: &str = r#"id = "cozy"
kind = "modpack"
name = "Cozy"
summary = "Coordinates and a calm render distance."
authors = ["Horizon"]
owners = ["ludovic111"]
license = "MIT"
source = "https://github.com/horizon-rust/horizon-mods"

[[versions]]
version = "1.0.0"
published = "2026-10-24"
game = "26.3"
file = "cozy-1.0.0.hpack"
sha256 = "241ac840f9c5ea3f6252d849da10c90d36cb1da4d6cd8d1d268c2760a353cd65"
"#;

const GLOW: &str = r#"id = "glow"
kind = "shader"
name = "Soft Glow"
summary = "A post-processing pass over the world."
authors = ["Horizon"]
owners = ["ludovic111"]
license = "MIT"
source = "https://github.com/horizon-rust/horizon-mods"

[[versions]]
version = "1.0.0"
published = "2026-10-26"
game = "26.3"
file = "glow-1.0.0.hshader"
sha256 = "331cfe29a666a0de804e9a9ff3d19d246eebcfbfa70b4f49bba8482d30ad9f02"
"#;

const REVOKED_TOML: &str = r#"[[revoked]]
id = "bad_mod"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
reason = "Sent chat messages it didn't declare."
date = "2026-10-24"
"#;

/// A registry checkout and its downloaded files, in a scratch folder.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("horizon-registry-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["repo/mods", "repo/icons", "files"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let s = Scratch(dir);
        s.write("mods/coords_hud.toml", &hud_entry());
        s.write("mods/cozy.toml", COZY);
        s.write("mods/glow.toml", GLOW);
        s.write("revoked.toml", REVOKED_TOML);
        s.write("icons/coords_hud.png", "png");
        for (name, hash) in [
            ("coords_hud-0.1.0.hmod", HMOD),
            ("cozy-1.0.0.hpack", HPACK),
            ("glow-1.0.0.hshader", HSHADER),
        ] {
            std::fs::copy(fixtures().join(name), s.0.join("files").join(hash)).unwrap();
        }
        s
    }

    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join("repo").join(path), text).unwrap();
    }

    fn build(&self) -> Result<Index, Vec<String>> {
        let (repo, files) = (self.0.join("repo"), self.0.join("files"));
        build(&Sources {
            registry: &repo,
            files: &files,
            release: super::super::FILE_HOST,
            site: "https://horizon-rust.github.io/horizon-mods/",
            generated: "2026-10-24T18:00:00Z".into(),
        })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn builds_the_sample_index() {
    let s = Scratch::new("sample");
    let built = s.build().unwrap();
    let sample = sample();
    assert_eq!(built.projects, sample.projects[..3]);
    assert_eq!(built.revoked, sample.revoked);
    assert_eq!(built.generated, sample.generated);
    // What it writes reads back.
    Index::parse(&serde_json::to_string_pretty(&built).unwrap()).unwrap();
}

#[test]
fn lists_every_problem_of_a_registry() {
    let s = Scratch::new("bad");
    // A file that isn't the one its hash names, an unsigned version of a
    // signed project, a shader pack, and a mod whose file is another mod.
    std::fs::write(s.0.join("files").join(HPACK), b"other bytes").unwrap();
    s.write(
        "mods/coords_hud.toml",
        &hud_entry().replace("signature = \"\"\"", "changelog = \"\"\""),
    );
    s.write(
        "mods/glow.toml",
        &COZY
            .replace("\"cozy\"", "\"glow\"")
            .replace("modpack", "shader"),
    );
    s.write(
        "mods/other.toml",
        &hud_entry()
            .replace("\"coords_hud\"", "\"other\"")
            .replace("key = ", "# key = ")
            .replace("signature = \"\"\"", "changelog = \"\"\""),
    );
    let errors = s.build().unwrap_err();
    let has = |part: &str| errors.iter().any(|e| e.contains(part));
    assert!(
        has("cozy 1.0.0: the file isn't the one its sha256 names"),
        "{errors:?}"
    );
    assert!(has("coords_hud 0.1.0: the project has a key"), "{errors:?}");
    // A shader entry is taken now, and checked like the others.
    assert!(
        has("glow 1.0.0: the file isn't the one its sha256 names"),
        "{errors:?}"
    );
    assert!(
        has("other 0.1.0: the file's id is \"coords_hud\""),
        "{errors:?}"
    );
    assert_eq!(errors.len(), 4, "{errors:?}");
}

#[test]
fn an_entry_is_named_by_its_id_and_has_only_known_fields() {
    let s = Scratch::new("names");
    s.write("mods/hud.toml", &hud_entry());
    s.write("mods/notes.txt", "");
    s.write("mods/cozy.toml", &format!("{COZY}homepage = \"x\"\n"));
    let errors = load(&s.0.join("repo")).unwrap_err();
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors[0].contains("cozy.toml") && errors[0].contains("homepage"));
    assert!(errors[1].contains("must be mods/coords_hud.toml"));
    assert!(errors[2].contains("only <id>.toml"));
}

#[test]
fn an_entry_written_reads_back() {
    let entry: Entry = toml::from_str(&hud_entry()).unwrap();
    let text = entry.to_toml().unwrap();
    assert_eq!(toml::from_str::<Entry>(&text).unwrap(), entry);
    assert!(text.contains("[[versions]]"), "{text}");
}

#[test]
fn a_review_protects_published_files() {
    let base: Entry = toml::from_str(&hud_entry()).unwrap();
    let cozy: Entry = toml::from_str(COZY).unwrap();
    let one = std::slice::from_ref(&base);
    let same = review(one, one, "someone");
    assert_eq!(same, Review::default());

    let mut update = base.clone();
    let mut next = update.versions[0].clone();
    next.version = "0.2.0".into();
    update.versions.insert(0, next);
    let r = review(one, std::slice::from_ref(&update), "ludovic111");
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(r.notes, ["coords_hud: 1 new version(s)"]);
    let r = review(one, &[update], "stranger");
    assert!(r.notes[0].contains("isn't one of its owners"), "{r:?}");

    let mut swapped = base.clone();
    swapped.versions[0].sha256 = HPACK.into();
    swapped.kind = Kind::ResourcePack;
    swapped.key = None;
    let r = review(&[base.clone(), cozy.clone()], &[swapped], "ludovic111");
    assert_eq!(r.errors.len(), 3, "{r:?}");
    assert!(r.errors.iter().any(|e| e.contains("cozy: removed")));
    assert!(r.errors.iter().any(|e| e.contains("never changes")));
    assert!(r.errors.iter().any(|e| e.contains("kind can't change")));
    assert!(r.notes.iter().any(|n| n.contains("signing key")));

    let r = review(&[], &[cozy], "newcomer");
    assert_eq!(r.notes, ["cozy: a new modpack by ludovic111"]);
}

#[test]
fn a_mod_may_turn_native_once() {
    let base: Entry = toml::from_str(&hud_entry()).unwrap();
    let mut native = base.clone();
    native.kind = Kind::NativeMod;
    let mut next = native.versions[0].clone();
    next.version = "2.0.0".into();
    next.file = "coords_hud-2.0.0.hnmod".into();
    native.versions.insert(0, next);
    let r = review(
        std::slice::from_ref(&base),
        std::slice::from_ref(&native),
        "ludovic111",
    );
    assert!(r.errors.is_empty(), "{r:?}");
    assert!(r.notes.iter().any(|n| n.contains("turns native")), "{r:?}");
    // Its content-only .hmod versions are from before; the new ones native, and a .hmod
    // holding native code is native too.
    let none = Path::new("no-such-file");
    assert_eq!(file_kind(&native, &native.versions[1], none), Kind::Mod);
    assert_eq!(
        file_kind(&native, &native.versions[0], none),
        Kind::NativeMod
    );
    let unified = std::env::temp_dir().join(format!("unified-{}.hmod", std::process::id()));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&unified).unwrap());
    zip.start_file(
        super::super::native::MANIFEST,
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.finish().unwrap();
    assert_eq!(
        file_kind(&native, &native.versions[1], &unified),
        Kind::NativeMod
    );
    std::fs::remove_file(unified).unwrap();
    // And never back.
    let r = review(&[native], &[base], "ludovic111");
    assert!(
        r.errors.iter().any(|e| e.contains("kind can't change")),
        "{r:?}"
    );
}

#[test]
fn only_open_kinds_are_taken() {
    assert!(open(&Kind::Mod).is_ok() && open(&Kind::ResourcePack).is_ok());
    assert!(open(&Kind::Shader).is_ok());
    assert!(open(&Kind::Other("world".into())).is_err());
    let mut submit: Entry = toml::from_str(COZY).unwrap();
    submit.id = "submit".into();
    let dir = std::env::temp_dir();
    let src = Sources {
        registry: &dir,
        files: &dir,
        release: super::super::FILE_HOST,
        site: "https://x.test/",
        generated: String::new(),
    };
    assert!(project(&submit, &src).unwrap_err()[0].contains("reserved"));
    assert!(plain_version("1.0.0-beta.2") && !plain_version("../1") && !plain_version("1 0"));
    assert!(plain_date("2026-10-24") && !plain_date("24/10/2026"));
}

const COZY_REPO: &str = r#"id = "cozy"
kind = "modpack"
repo = "acme/cozy-pack"
owners = ["acme"]
"#;

#[test]
fn a_project_listed_by_its_repository_takes_its_versions_from_releases() {
    let s = Scratch::new("repo");
    s.write("mods/coords_hud.toml", &hud_entry());
    s.write("mods/cozy.toml", COZY_REPO);
    let hpack = fixtures().join("cozy-1.0.0.hpack");
    let entry: Entry = toml::from_str(COZY_REPO).unwrap();
    // Listed, nothing released yet: not in the index, and not an error.
    let built = s.build().unwrap();
    assert!(built.project("cozy").is_none());
    // The page file fills what the entry leaves out; the entry's own words win.
    let page = page(
        &entry,
        "name = \"Cozy\"\nsummary = \"Calm.\"\nlicense = \"MIT\"\nauthors = [\"Acme\"]\n",
    )
    .unwrap();
    assert!(
        self::page(&entry, "name = \"Cozy\"")
            .unwrap_err()
            .contains("summary")
    );
    assert!(self::page(&entry, "owners = [\"me\"]").is_err());
    let release = |tag, asset, notes| Release {
        tag,
        published: "2026-10-30",
        notes,
        asset,
        game: "26.3",
        signature: None,
    };
    let v = release_version(
        &entry,
        &release("v1.0.0", "cozy-1.0.0.hpack", "Notes."),
        &hpack,
    )
    .unwrap();
    assert_eq!(v.version, "1.0.0");
    assert_eq!(v.file.sha256, HPACK);
    assert_eq!(
        v.file.url,
        "https://github.com/acme/cozy-pack/releases/download/v1.0.0/cozy-1.0.0.hpack"
    );
    assert_eq!(v.changelog, "Notes.");
    // A name the launcher can't download, a wrong extension, a project with a key but no signature.
    assert!(release_version(&entry, &release("v1/x", "cozy-1.0.0.hpack", ""), &hpack).is_err());
    assert!(release_version(&entry, &release("v1", "cozy.zip", ""), &hpack).is_err());
    let mut keyed = entry.clone();
    keyed.key = Some("RWTVtuGb/GRXcBf3Re5hzM07EzoStr2sLTJjghJia0xX15ULoZRFFZDb".into());
    assert!(
        release_version(&keyed, &release("v1", "cozy-1.0.0.hpack", ""), &hpack)
            .unwrap_err()
            .contains("sign")
    );
    let cache = Cache {
        page,
        versions: vec![v.clone()],
    };
    std::fs::create_dir_all(s.0.join("repo").join(CACHE)).unwrap();
    s.write(
        "versions/cozy.json",
        &serde_json::to_string(&cache).unwrap(),
    );
    let built = s.build().unwrap();
    let p = built.project("cozy").unwrap();
    assert_eq!(
        (p.name.as_str(), p.authors.clone()),
        ("Cozy", vec!["Acme".to_string()])
    );
    assert_eq!(p.source, "https://github.com/acme/cozy-pack");
    assert_eq!(p.versions, vec![v]);
    // The registry's own words win over the repository's.
    s.write(
        "mods/cozy.toml",
        &format!("{COZY_REPO}name = \"Cozy Pack\"\n"),
    );
    assert_eq!(
        s.build().unwrap().project("cozy").unwrap().name,
        "Cozy Pack"
    );
    // A repository that is not owner/name is refused.
    s.write(
        "mods/cozy.toml",
        &COZY_REPO.replace("acme/cozy-pack", "acme"),
    );
    assert!(
        s.build()
            .unwrap_err()
            .iter()
            .any(|e| e.contains("owner/name"))
    );
}
