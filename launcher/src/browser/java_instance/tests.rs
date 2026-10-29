use super::*;
use crate::browser::{download, modrinth, FileRef, SourceId};
use sha2::Digest;
use std::collections::HashMap;
use std::path::PathBuf;

/// A source in memory: projects, their versions (newest first), and the bytes each URL serves.
#[derive(Default)]
struct Fake {
    projects: HashMap<String, Project>,
    versions: HashMap<String, Vec<Version>>,
    bytes: HashMap<String, Vec<u8>>,
}

impl Source for Fake {
    fn id(&self) -> SourceId {
        SourceId::Modrinth
    }
    fn search(&self, _: &Query) -> Result<Vec<Project>, String> {
        Ok(self.projects.values().cloned().collect())
    }
    fn project(&self, id: &str) -> Result<Project, String> {
        self.projects
            .get(id)
            .cloned()
            .ok_or_else(|| "not found".into())
    }
    fn versions(&self, p: &Project) -> Result<Vec<Version>, String> {
        Ok(self.versions.get(&p.id).cloned().unwrap_or_default())
    }
    fn fetch(
        &self,
        file: &FileRef,
        folder: &Path,
        progress: &dyn Fn(f32),
    ) -> Result<PathBuf, String> {
        download::save(file, &self.bytes[&file.url][..], folder, progress)
    }
}

impl Fake {
    /// A new newest version of `id` (a project added on first use).
    fn release(
        &mut self,
        id: &str,
        kind: Kind,
        number: &str,
        games: &[&str],
        loaders: &[&str],
        needs: &[&str],
    ) {
        self.projects.entry(id.into()).or_insert_with(|| Project {
            source: SourceId::Modrinth,
            id: id.into(),
            slug: format!("{id}-slug"),
            title: id.into(),
            summary: String::new(),
            author: String::new(),
            downloads: 0,
            page: String::new(),
            icon: None,
            license: String::new(),
            kind,
            game_versions: Vec::new(),
            loaders: Vec::new(),
        });
        let body = format!("{id} {number}").into_bytes();
        let url = format!("https://cdn.modrinth.com/data/{id}/versions/{number}/{id}-{number}.jar");
        let file = FileRef {
            url: url.clone(),
            name: format!("{id}-{number}.jar"),
            size: body.len() as u64,
            sha512: Some(
                sha2::Sha512::digest(&body)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            ),
            sha256: None,
            sha1: None,
            primary: true,
        };
        let strings = |l: &[&str]| l.iter().map(|s| s.to_string()).collect();
        self.bytes.insert(url, body);
        self.versions.entry(id.into()).or_default().insert(
            0,
            Version {
                id: format!("{id}@{number}"),
                number: number.into(),
                published: String::new(),
                game_versions: strings(games),
                loaders: strings(loaders),
                files: vec![file],
                required: strings(needs),
                blocked: None,
            },
        );
    }
}

fn target(minecraft: &str, loader: Option<&str>) -> Target {
    Target {
        minecraft: minecraft.into(),
        loader: loader.map(Into::into),
    }
}

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-java-instance-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

const NONE: &dyn Fn(&str, f32) = &|_, _| {};

#[test]
fn picks_the_newest_file_for_the_version_and_loader() {
    let mut fake = Fake::default();
    fake.release("sodium", Kind::Mod, "1", &["1.21.1"], &["fabric"], &[]);
    fake.release("sodium", Kind::Mod, "2", &["1.21.1"], &["quilt"], &[]);
    fake.release("sodium", Kind::Mod, "3", &["1.21.1"], &["fabric"], &[]);
    fake.release(
        "sodium",
        Kind::Mod,
        "4",
        &["1.21.4"],
        &["fabric", "quilt"],
        &[],
    );
    let versions = &fake.versions["sodium"];
    let pick = |t: Target| t.pick(Kind::Mod, versions).map(|v| v.number.clone());

    assert_eq!(pick(target("1.21.1", Some("fabric"))).as_deref(), Some("3"));
    // Quilt's own file first, else a Fabric one.
    assert_eq!(pick(target("1.21.1", Some("quilt"))).as_deref(), Some("2"));
    fake.versions
        .get_mut("sodium")
        .unwrap()
        .retain(|v| v.number != "2");
    let versions = &fake.versions["sodium"];
    assert_eq!(
        target("1.21.1", Some("quilt"))
            .pick(Kind::Mod, versions)
            .unwrap()
            .number,
        "3"
    );
    assert!(target("1.21.1", Some("neoforge"))
        .pick(Kind::Mod, versions)
        .is_none());
    assert!(target("1.20.1", Some("fabric"))
        .pick(Kind::Mod, versions)
        .is_none());
    assert!(target("1.21.1", None).pick(Kind::Mod, versions).is_none());

    let mut packs = Fake::default();
    packs.release(
        "faithful",
        Kind::ResourcePack,
        "1",
        &["1.21.1"],
        &["minecraft"],
        &[],
    );
    let versions = &packs.versions["faithful"];
    assert!(target("1.21.1", None)
        .pick(Kind::ResourcePack, versions)
        .is_some());
    assert!(target("1.21.4", None)
        .pick(Kind::ResourcePack, versions)
        .is_none());
}

#[test]
fn searches_for_what_runs_in_the_instance() {
    let t = target("1.20.1", Some("quilt"));
    assert_eq!(
        modrinth::facets(&t.query("map", Kind::Mod)),
        r#"[["project_type:mod"],["versions:1.20.1"],["categories:quilt","categories:fabric"]]"#
    );
    assert_eq!(
        modrinth::facets(&t.query("", Kind::ResourcePack)),
        r#"[["project_type:resourcepack"],["versions:1.20.1"]]"#
    );
    assert_eq!(
        modrinth::facets(&target("1.21.1", Some("neoforge")).query("", Kind::Mod)),
        r#"[["project_type:mod"],["versions:1.21.1"],["categories:neoforge"]]"#
    );
}

#[test]
fn installs_jars_with_dependencies_then_updates_and_removes() {
    let home = home("cycle");
    let t = target("1.21.1", Some("fabric"));
    let mut fake = Fake::default();
    fake.release("fabric-api", Kind::Mod, "1", &["1.21.1"], &["fabric"], &[]);
    fake.release(
        "lib",
        Kind::Mod,
        "1",
        &["1.21.1"],
        &["fabric"],
        &["fabric-api"],
    );
    fake.release(
        "map",
        Kind::Mod,
        "1",
        &["1.21.1"],
        &["fabric"],
        &["lib", "fabric-api"],
    );
    let map = fake.project("map").unwrap();

    let steps = plan(&fake, &t, &map).unwrap();
    let names: Vec<_> = steps
        .iter()
        .map(|s| (s.project.id.as_str(), s.dependency))
        .collect();
    assert_eq!(names, [("fabric-api", true), ("lib", true), ("map", false)]);

    let done = install(&fake, &t, &map, &home, NONE).unwrap();
    assert_eq!(done.len(), 3);
    assert_eq!(fs::read(home.join("mods/map-1.jar")).unwrap(), b"map 1");
    assert!(home.join("mods/fabric-api-1.jar").is_file());
    assert_eq!(Installed::load(&home).entries.len(), 3);

    fake.release("map", Kind::Mod, "2", &["1.21.1"], &["fabric"], &["lib"]);
    let entry = Installed::load(&home).lookup("map").cloned().unwrap();
    assert_eq!(update_for(&fake, &t, &entry).unwrap().unwrap().number, "2");
    // Turned off, the mod is kept as `.jar.disabled`, which the loaders skip.
    set_on(&home, &entry, false).unwrap();
    assert!(!is_on(&home, &entry));
    assert!(
        home.join("mods/map-1.jar.disabled").is_file() && !home.join("mods/map-1.jar").exists()
    );
    set_on(&home, &entry, false).unwrap();
    // It stays off through an update, and the old file goes.
    install(&fake, &t, &map, &home, NONE).unwrap();
    assert!(
        !home.join("mods/map-1.jar").exists() && !home.join("mods/map-1.jar.disabled").exists()
    );
    assert!(
        home.join("mods/map-2.jar.disabled").is_file() && !home.join("mods/map-2.jar").exists()
    );
    let entry = Installed::load(&home).lookup("map").cloned().unwrap();
    assert!(!is_on(&home, &entry));
    assert!(update_for(&fake, &t, &entry).unwrap().is_none());
    install(&fake, &t, &map, &home, NONE).unwrap();
    assert!(!is_on(&home, &entry));
    set_on(&home, &entry, true).unwrap();
    assert!(is_on(&home, &entry) && home.join("mods/map-2.jar").is_file());
    set_on(&home, &entry, false).unwrap();

    remove(&home, "map-slug").unwrap();
    assert!(
        !home.join("mods/map-2.jar").exists() && !home.join("mods/map-2.jar.disabled").exists()
    );
    assert!(Installed::load(&home).lookup("map").is_none());
    assert!(home.join("mods/lib-1.jar").is_file());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn refuses_what_the_instance_cannot_run() {
    let home = home("refuse");
    let mut fake = Fake::default();
    fake.release("forge-lib", Kind::Mod, "1", &["1.21.1"], &["forge"], &[]);
    fake.release(
        "map",
        Kind::Mod,
        "1",
        &["1.21.1"],
        &["fabric"],
        &["forge-lib"],
    );
    let map = fake.project("map").unwrap();

    let err = install(&fake, &target("1.21.1", Some("fabric")), &map, &home, NONE).unwrap_err();
    assert_eq!(
        err,
        "forge-lib has no version for Minecraft 1.21.1 with fabric."
    );
    // Nothing is downloaded when one dependency can't be installed.
    assert!(!home.join("mods").exists());

    let err = plan(&fake, &target("1.21.1", None), &map).unwrap_err();
    assert!(err.contains("vanilla Minecraft 1.21.1"), "{err}");
    let err = plan(&fake, &target("1.20.1", Some("fabric")), &map).unwrap_err();
    assert_eq!(err, "map has no version for Minecraft 1.20.1 with fabric.");
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_page_search_keeps_the_players_filters_but_not_their_version() {
    let t = target("1.21.1", Some("fabric"));
    let picked = details::Filters {
        categories: vec!["optimization".into()],
        loader: Some("neoforge".into()),
        game_version: Some("26.3".into()),
        environment: Some(details::Environment::Client),
        sort: Some(details::Sort::Downloads),
    };
    let q = t.search("fps", Kind::Mod, &picked);
    assert_eq!(q.text, "fps");
    assert_eq!(
        q.filters,
        details::Filters {
            game_version: Some("1.21.1".into()),
            loader: Some("fabric".into()),
            ..picked
        }
    );
    assert!(t.takes(Kind::Mod) && t.takes(Kind::ResourcePack));
    assert!(!t.takes(Kind::Modpack) && !t.takes(Kind::Shader) && !t.takes(Kind::DataPack));
    assert_eq!(t.describe(Kind::Mod), "Minecraft 1.21.1 with fabric");
    assert_eq!(t.describe(Kind::ResourcePack), "Minecraft 1.21.1");
}
