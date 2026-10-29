//! Step 1b, the jars: for the mods the player asked to port, their own
//! download of each jar into `<work>/jars/`. Only from where the pack itself
//! points (a Modrinth pack's download URLs) or the same file on Modrinth's
//! public CDN (found by its SHA-1), and only kept when the hash matches; or
//! copied from a folder the player already has (an installed instance, a
//! downloads folder). CurseForge's own downloads need an API key, so a file
//! only CurseForge has is listed as needing one. `<work>/fetch.json` says
//! what happened to each file.
use crate::{
    import::{Pack, PackFile, sha1_hex},
    inventory::Record,
    modrinth::{self, Cache},
    pipeline::{Log, read_json, write_json},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Hosts a Modrinth pack may download from (the `.mrpack` format's list).
const HOSTS: [&str; 4] = [
    "https://cdn.modrinth.com/",
    "https://github.com/",
    "https://raw.githubusercontent.com/",
    "https://gitlab.com/",
];
/// No mod jar is this large; a bigger answer is not one.
const MAX_JAR: u64 = 512 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum How {
    /// Already in `jars/` (or the imported instance).
    Present,
    /// Downloaded from the pack's URL or Modrinth's CDN, hash checked.
    Downloaded,
    /// Copied from the player's folder.
    Local,
    /// Only on CurseForge, whose downloads are coming soon (its API needs a paid key).
    NeedsKey,
    NotFound,
    Failed,
}

impl How {
    pub fn label(self) -> &'static str {
        match self {
            How::Present => "already here",
            How::Downloaded => "downloaded",
            How::Local => "copied from your folder",
            How::NeedsKey => "CurseForge downloads coming soon",
            How::NotFound => "not found",
            How::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub key: String,
    pub file: String,
    pub how: How,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Fetched {
    pub files: Vec<Outcome>,
}

impl Fetched {
    pub fn count(&self, how: How) -> usize {
        self.files.iter().filter(|o| o.how == how).count()
    }
    /// `<work>/fetch.json`, empty when the step has not run.
    pub fn load(work: &Path) -> Self {
        read_json(&work.join("fetch.json")).unwrap_or_default()
    }
}

/// A file name from a pack's index that stays inside `jars/`: one plain part, with no separator
/// of any platform, no drive and not `.` or `..`.
fn plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0'])
}

/// The jars of the records `want` picks that the work folder lacks.
pub fn fetch(
    work: &Path,
    records: &[Record],
    from: Option<&Path>,
    want: &dyn Fn(&Record) -> bool,
    log: Log,
) -> Result<Fetched, String> {
    let pack: Pack = read_json(&work.join("pack.json"))?;
    let cache = Cache::load(&work.join("modrinth.json"));
    let files: HashMap<&str, &PackFile> = pack.files.iter().map(|f| (f.path.as_str(), f)).collect();
    let jars = work.join("jars");
    fs::create_dir_all(&jars).map_err(|e| format!("{}: {e}", jars.display()))?;
    let local = from.map(Local::index);
    let wanted: Vec<&Record> = records.iter().filter(|r| want(r)).collect();
    let mut fetched = Fetched::default();
    for (i, r) in wanted.iter().enumerate() {
        let Some(file) = files.get(r.path.as_str()) else {
            continue;
        };
        let name = file.file_name();
        let outcome = |how, detail: String| Outcome {
            key: r.key.clone(),
            file: name.to_owned(),
            how,
            detail,
        };
        if !plain_file_name(name) {
            // A name like `..\..\x.dll` or `C:\x` would write outside jars/ on Windows.
            let o = outcome(How::Failed, format!("{name:?} is not a plain file name"));
            log(format!("{}: {} ({})", o.key, o.how.label(), o.detail));
            fetched.files.push(o);
            continue;
        }
        let dest = jars.join(name);
        let sha1 = file.sha1.as_deref();
        let o = if r.jar.is_some() || matches(&dest, sha1) {
            outcome(How::Present, String::new())
        } else if let Some(path) = local.as_ref().and_then(|l| l.find(file)) {
            match fs::copy(&path, &dest) {
                Ok(_) => outcome(How::Local, path.display().to_string()),
                Err(e) => outcome(How::Failed, format!("{}: {e}", path.display())),
            }
        } else if let Some(url) = url_of(file, &cache) {
            log(format!("{}/{} {name}", i + 1, wanted.len()));
            match download(&url, sha1, &dest) {
                Ok(()) => outcome(How::Downloaded, url),
                Err(e) => outcome(How::Failed, e),
            }
        } else if pack.format == "curseforge" || file.curseforge.is_some() {
            outcome(
                How::NeedsKey,
                "on CurseForge only: choose your installed instance, or put the jar in jars/"
                    .into(),
            )
        } else {
            outcome(How::NotFound, "no download link in the pack".into())
        };
        if matches!(o.how, How::Failed | How::NeedsKey | How::NotFound) {
            log(format!("{}: {} ({})", o.key, o.how.label(), o.detail));
        }
        fetched.files.push(o);
    }
    // The loader's own data (its convention tags), from its own repository.
    if let Some((dest, url)) = crate::worldgen::loader_jar(work, &pack.loader, &pack.loader_version)
        .filter(|(dest, _)| !dest.exists())
    {
        let checksum = format!("{url}.sha1");
        let sha1 = crate::net::agent(&checksum)
            .get(&checksum)
            .set("User-Agent", modrinth::AGENT)
            .call()
            .ok()
            .and_then(|r| r.into_string().ok())
            .map(|s| s.trim().to_owned());
        let made = dest.parent().map(fs::create_dir_all);
        match made
            .unwrap_or(Ok(()))
            .map_err(|e| e.to_string())
            .and_then(|()| download(&url, sha1.as_deref(), &dest))
        {
            Ok(()) => log(format!("loader: {}", dest.display())),
            Err(e) => log(format!(
                "loader: {e} (structures keep only vanilla biome tags)"
            )),
        }
    }
    write_json(&work.join("fetch.json"), &fetched)?;
    log(format!(
        "jars: {} downloaded, {} copied, {} already here, {} on CurseForge only (CurseForge downloads coming soon), {} not found, {} failed",
        fetched.count(How::Downloaded),
        fetched.count(How::Local),
        fetched.count(How::Present),
        fetched.count(How::NeedsKey),
        fetched.count(How::NotFound),
        fetched.count(How::Failed)
    ));
    Ok(fetched)
}

/// Whether `path` holds the file with this SHA-1 (any file when none is known).
fn matches(path: &Path, sha1: Option<&str>) -> bool {
    match (fs::read(path), sha1) {
        (Ok(bytes), Some(h)) => sha1_hex(&bytes).eq_ignore_ascii_case(h),
        (Ok(_), None) => true,
        (Err(_), _) => false,
    }
}

/// The pack's own download link, else the same file on Modrinth.
pub fn url_of(file: &PackFile, cache: &Cache) -> Option<String> {
    let allowed = |u: &&str| HOSTS.iter().any(|h| u.starts_with(h));
    if let Some(u) = file.downloads.iter().map(String::as_str).find(allowed) {
        return Some(u.to_owned());
    }
    let sha1 = file.sha1.as_deref()?;
    cache.versions.get(sha1)?["files"]
        .as_array()?
        .iter()
        .find(|f| f["hashes"]["sha1"].as_str() == Some(sha1))?["url"]
        .as_str()
        .filter(allowed)
        .map(str::to_owned)
}

/// Downloads `url` to `dest`, kept only if its SHA-1 is `sha1`.
fn download(url: &str, sha1: Option<&str>, dest: &Path) -> Result<(), String> {
    let response = crate::net::agent(url)
        .get(url)
        .set("User-Agent", modrinth::AGENT)
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_JAR)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{url}: {e}"))?;
    let got = sha1_hex(&bytes);
    if let Some(want) = sha1.filter(|w| !w.eq_ignore_ascii_case(&got)) {
        return Err(format!("{url}: SHA-1 {got}, the pack says {want}"));
    }
    let part = dest.with_extension("part");
    fs::write(&part, &bytes).map_err(|e| format!("{}: {e}", part.display()))?;
    fs::rename(&part, dest).map_err(|e| format!("{}: {e}", dest.display()))
}

/// The jars in a folder of the player's, by SHA-1 and by name, and what a
/// CurseForge app instance (`minecraftinstance.json`) says each
/// project's file is called.
#[derive(Default)]
struct Local {
    by_sha1: HashMap<String, PathBuf>,
    by_name: HashMap<String, PathBuf>,
    curseforge: HashMap<(u64, u64), String>,
}

impl Local {
    fn index(dir: &Path) -> Local {
        let mut local = Local::default();
        local.walk(dir, 3);
        if let Ok(text) = fs::read_to_string(dir.join("minecraftinstance.json")) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                local.curseforge = curseforge_names(&v);
            }
        }
        local
    }

    fn walk(&mut self, dir: &Path, depth: usize) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                if depth > 0 {
                    self.walk(&path, depth - 1);
                }
            } else if path.extension().is_some_and(|x| x == "jar") {
                if let Ok(bytes) = fs::read(&path) {
                    self.by_sha1.insert(sha1_hex(&bytes), path.clone());
                }
                if let Some(n) = path.file_name() {
                    self.by_name
                        .insert(n.to_string_lossy().into_owned(), path.clone());
                }
            }
        }
    }

    fn find(&self, file: &PackFile) -> Option<PathBuf> {
        if let Some(h) = &file.sha1 {
            return self.by_sha1.get(&h.to_ascii_lowercase()).cloned();
        }
        let name = file
            .curseforge
            .and_then(|ids| self.curseforge.get(&ids))
            .map_or(file.file_name(), String::as_str);
        self.by_name.get(name).cloned()
    }
}

/// `installedAddons[]`: (project, file) → the jar's file name.
pub fn curseforge_names(instance: &Value) -> HashMap<(u64, u64), String> {
    instance["installedAddons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let file = &a["installedFile"];
            Some((
                (a["addonID"].as_u64()?, file["id"].as_u64()?),
                file["fileName"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pack_index_name_must_stay_inside_jars() {
        assert!(plain_file_name("sodium-0.6.jar"));
        for name in ["", ".", "..", "..\\..\\evil.dll", "C:evil.jar", "a/b.jar"] {
            assert!(!plain_file_name(name), "{name:?}");
        }
    }

    fn file(sha1: Option<&str>, downloads: &[&str]) -> PackFile {
        PackFile {
            path: "mods/a.jar".into(),
            sha1: sha1.map(str::to_owned),
            downloads: downloads.iter().map(|d| (*d).to_owned()).collect(),
            ..PackFile::default()
        }
    }

    #[test]
    fn links_come_from_the_pack_or_modrinth_and_only_allowed_hosts() {
        let mut cache = Cache::default();
        cache.versions.insert(
            "ab".into(),
            json!({"files": [
                {"hashes": {"sha1": "cd"}, "url": "https://cdn.modrinth.com/other.jar"},
                {"hashes": {"sha1": "ab"}, "url": "https://cdn.modrinth.com/a.jar"}]}),
        );
        let pack_link = file(
            Some("ab"),
            &["https://evil.example/a.jar", "https://github.com/a.jar"],
        );
        assert_eq!(
            url_of(&pack_link, &cache).as_deref(),
            Some("https://github.com/a.jar")
        );
        let modrinth = file(Some("ab"), &["https://evil.example/a.jar"]);
        assert_eq!(
            url_of(&modrinth, &cache).as_deref(),
            Some("https://cdn.modrinth.com/a.jar")
        );
        assert_eq!(url_of(&file(Some("ef"), &[]), &cache), None);
        assert_eq!(url_of(&file(None, &[]), &cache), None);
    }

    #[test]
    fn reads_a_curseforge_instance() {
        let instance = json!({"installedAddons": [
            {"addonID": 238222, "installedFile": {"id": 5101366, "fileName": "jei-1.21.1.jar"}},
            {"addonID": 1, "installedFile": {"fileName": "no-id.jar"}}]});
        let names = curseforge_names(&instance);
        assert_eq!(names.len(), 1);
        assert_eq!(names[&(238222, 5101366)], "jei-1.21.1.jar");
    }

    #[test]
    fn copies_the_players_jars_and_lists_what_needs_a_key() {
        let root = std::env::temp_dir().join(format!("horizon-fetch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (work, mine) = (root.join("work"), root.join("instance/mods"));
        fs::create_dir_all(&mine).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(mine.join("renamed.jar"), b"jar a").unwrap();
        fs::write(mine.join("cf.jar"), b"jar b").unwrap();
        fs::write(
            root.join("instance/minecraftinstance.json"),
            json!({"installedAddons": [{"addonID": 7, "installedFile": {"id": 8, "fileName": "cf.jar"}}]})
                .to_string(),
        )
        .unwrap();
        let pack = Pack {
            format: "curseforge".into(),
            files: vec![
                PackFile {
                    path: "mods/a.jar".into(),
                    sha1: Some(sha1_hex(b"jar a")),
                    ..PackFile::default()
                },
                PackFile {
                    path: "mods/curseforge-7-8".into(),
                    curseforge: Some((7, 8)),
                    ..PackFile::default()
                },
                PackFile {
                    path: "mods/curseforge-9-9".into(),
                    curseforge: Some((9, 9)),
                    ..PackFile::default()
                },
            ],
            ..Pack::default()
        };
        write_json(&work.join("pack.json"), &pack).unwrap();
        let records: Vec<Record> = pack
            .files
            .iter()
            .map(|f| Record {
                key: f.file_name().into(),
                path: f.path.clone(),
                ..Record::default()
            })
            .collect();
        let fetched = fetch(
            &work,
            &records,
            Some(&root.join("instance")),
            &|_| true,
            &|_| {},
        )
        .unwrap();
        let hows: Vec<How> = fetched.files.iter().map(|o| o.how).collect();
        assert_eq!(hows, [How::Local, How::Local, How::NeedsKey]);
        assert_eq!(fs::read(work.join("jars/a.jar")).unwrap(), b"jar a");
        assert_eq!(
            fs::read(work.join("jars/curseforge-7-8")).unwrap(),
            b"jar b"
        );
        // A second run finds them in place.
        let again = fetch(&work, &records, None, &|_| true, &|_| {}).unwrap();
        assert_eq!(again.count(How::Present), 2);
        assert_eq!(Fetched::load(&work).count(How::Present), 2);
        fs::remove_dir_all(&root).unwrap();
    }
}
