//! Horizon's own registry (docs/api/mod-index.md): mods, modpacks and resource packs made for
//! Horizon, reviewed as pull requests in horizon-rust/horizon-mods. One file lists them all,
//! `index.json`; every download is checked against it before it reaches the game folder
//! (`horizon_mods::index::check_file`: size, SHA-256, the author's signature, the file's own
//! manifest), and files the index says were taken down are refused and turned off.
use super::{
    details::{Details, Link},
    download, installed, java, FileRef, Kind, Project, Query, Source, SourceId, Version, GAME,
};
use horizon_mods::index::{self, Index, Trust};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where the registry publishes its index (GitHub Pages of horizon-rust/horizon-mods).
pub const INDEX_URL: &str = "https://horizon-rust.github.io/horizon-mods/index.json";
/// The registry's pages on the website, `<SITE>/<id>`.
const SITE: &str = "https://playhorizon.dev/mods";
/// The report form of a project.
const REPORT: &str = "https://github.com/horizon-rust/horizon-mods/issues/new?template=report.yml";
/// The loader name the browser gives registry versions, so `support` and `pick` know them.
pub const LOADER: &str = "horizon";
/// How long a fetched index is used before it is fetched again.
const FRESH: Duration = Duration::from_secs(600);
/// Larger than any index should be.
const MAX_INDEX: u64 = 16 << 20;
/// The last index fetched, kept in the game folder for when the registry can't be reached.
const CACHE: &str = "mod-browser/horizon-index.json";
/// Players' ratings of every project (the registry service, crates/horizon-mod-registry).
const RATINGS_URL: &str = "https://playhorizon.dev/api/mods/ratings";
/// Where a player rates a project: the site's GitHub sign-in, which comes back to its page.
/// The launcher has no GitHub account of its own.
const RATE: &str = "https://playhorizon.dev/api/mods/auth/login?back=/mods/";

/// Where the index comes from: `HORIZON_MOD_INDEX` (a file or an `https://` address, for a
/// test registry), else [`INDEX_URL`].
fn location() -> String {
    std::env::var("HORIZON_MOD_INDEX")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| INDEX_URL.to_owned())
}

/// True when the index is a local file: its files may then be local too (`file://`), for
/// trying a registry out before it is published.
pub fn local() -> bool {
    !location().starts_with("https://")
}

static LOADED: Mutex<Option<(Instant, Arc<Index>)>> = Mutex::new(None);

/// The index read last in this run, without asking the network: for what must not wait.
pub fn known_index() -> Option<Arc<Index>> {
    LOADED
        .lock()
        .unwrap()
        .as_ref()
        .map(|(_, index)| index.clone())
}

/// Whether the registry's project `id` is a native mod, by the index already read (never the
/// network); false when none was read.
pub fn is_native_mod(id: &str) -> bool {
    known_index()
        .and_then(|index| Some(index.project(id)?.kind == index::Kind::NativeMod))
        .unwrap_or(false)
}

/// The index: the one fetched in the last ten minutes, else fetched again (and kept in the game
/// folder), else the copy kept last time.
pub fn index() -> Result<Arc<Index>, String> {
    if let Some((at, index)) = LOADED.lock().unwrap().as_ref() {
        if at.elapsed() < FRESH {
            return Ok(index.clone());
        }
    }
    let cache = crate::backend::game_home().join(CACHE);
    let index = match fetch_index() {
        Ok((index, text)) => {
            if let Some(dir) = cache.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(&cache, text);
            index
        }
        Err(e) => {
            let kept = fs::read_to_string(&cache).map_err(|_| e.clone())?;
            match local() {
                true => Index::parse_local(&kept),
                false => Index::parse(&kept),
            }
            .map_err(|_| e)?
        }
    };
    let index = Arc::new(index);
    *LOADED.lock().unwrap() = Some((Instant::now(), index.clone()));
    Ok(index)
}

fn fetch_index() -> Result<(Index, String), String> {
    use std::io::Read;
    let at = location();
    let text = if at.starts_with("https://") {
        let response = download::agent(&at).get(&at).call().map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("The mod registry answered {code}."),
            ureq::Error::Transport(t) => format!("The mod registry can't be reached: {t}"),
        })?;
        let mut text = String::new();
        response
            .into_reader()
            .take(MAX_INDEX)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        text
    } else {
        fs::read_to_string(&at).map_err(|e| format!("{at}: {e}"))?
    };
    let index = match local() {
        true => Index::parse_local(&text)?,
        false => Index::parse(&text)?,
    };
    Ok((index, text))
}

/// A project's ratings: 1 to 5 stars, their mean and how many.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rating {
    pub average: f32,
    pub count: u64,
}

/// When the ratings were fetched last, and every rated project's [`Rating`] (None: that failed).
type Ratings = Option<Arc<HashMap<String, Rating>>>;
static RATINGS: Mutex<Option<(Instant, Ratings)>> = Mutex::new(None);

/// A project's rating as fetched last, without asking the network: None when the ratings
/// aren't known (not fetched, or the service didn't answer), `Some(None)` when it has none.
pub fn rating(id: &str) -> Option<Option<Rating>> {
    let ratings = RATINGS.lock().unwrap();
    let known = ratings.as_ref()?.1.as_ref()?;
    Some(known.get(id).copied())
}

/// The page that signs the player in with GitHub on the site and lets them rate `id`.
pub fn rate_url(id: &str) -> String {
    format!("{RATE}{id}")
}

/// Fetches the ratings again when the last try is ten minutes old: from `HORIZON_MOD_RATINGS`
/// (a file or an `https://` address, for a test registry), else [`RATINGS_URL`]; a local test
/// registry has none unless given. After a failure the browser shows downloads only.
fn refresh_ratings() {
    if RATINGS
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|(at, _)| at.elapsed() < FRESH)
    {
        return;
    }
    let given = std::env::var("HORIZON_MOD_RATINGS")
        .ok()
        .filter(|v| !v.is_empty());
    if given.is_none() && local() {
        return;
    }
    let at = given.unwrap_or_else(|| RATINGS_URL.to_owned());
    let text = if at.starts_with("https://") {
        download::agent(&at).get(&at).call().ok().and_then(|r| {
            use std::io::Read;
            let mut text = String::new();
            r.into_reader()
                .take(MAX_INDEX)
                .read_to_string(&mut text)
                .ok()
                .map(|_| text)
        })
    } else {
        fs::read_to_string(&at).ok()
    };
    let fetched = text.map(|t| Arc::new(parse_ratings(&t)));
    *RATINGS.lock().unwrap() = Some((Instant::now(), fetched));
}

/// `{"projects": {"<id>": {"average", "count"}}}`, keeping only well-formed entries.
fn parse_ratings(text: &str) -> HashMap<String, Rating> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return HashMap::new();
    };
    let Some(projects) = json["projects"].as_object() else {
        return HashMap::new();
    };
    projects
        .iter()
        .filter_map(|(id, r)| {
            let average = r["average"].as_f64().filter(|a| (1.0..=5.0).contains(a))?;
            let count = r["count"].as_u64().filter(|&c| c > 0)?;
            index::valid_id(id).then(|| {
                (
                    id.clone(),
                    Rating {
                        average: average as f32,
                        count,
                    },
                )
            })
        })
        .collect()
}

/// The browser's kind for a registry kind; None for one the launcher doesn't install yet
/// (kinds newer than this launcher).
fn kind_of(kind: &index::Kind) -> Option<Kind> {
    match kind {
        index::Kind::Mod => Some(Kind::Mod),
        index::Kind::ResourcePack => Some(Kind::ResourcePack),
        index::Kind::Modpack => Some(Kind::Modpack),
        index::Kind::Shader => Some(Kind::Shader),
        // Mixin mods: installed beside the others, compiled into the game by `native_mods`.
        index::Kind::NativeMod => Some(Kind::Mod),
        index::Kind::Other(_) => None,
    }
}

/// Where the registry's mod policy is: who owns and answers for a project, takedowns.
pub const POLICY: &str = "https://playhorizon.dev/mods/policy";

/// Whether the Horizon project made `p` (the mod policy's "Official": author Horizon, source in
/// the horizon-rust organisation).
pub fn official(p: &index::Project) -> bool {
    p.authors == ["Horizon"] && p.source.starts_with("https://github.com/horizon-rust/")
}

/// What the player reads about a registry project before installing it: who made it, whether its
/// files are signed, and for a mod what it may do, in the game's words (Options > Mods). A native
/// mod says it runs with the player's rights.
pub fn trust_and_capabilities(id: &str) -> Vec<String> {
    let Ok(index) = index() else {
        return Vec::new();
    };
    let Some(p) = index.project(id) else {
        return Vec::new();
    };
    let mut lines = vec![if official(p) {
        "Made by Horizon".to_owned()
    } else {
        format!(
            "Made by {}. Not made or endorsed by the Horizon project ({POLICY})",
            p.authors.join(", ")
        )
    }];
    lines.push(match &p.key {
        Some(_) => "Signed by its author: the launcher checks the signature".to_owned(),
        None => "Unsigned: the launcher checks its checksum only".to_owned(),
    });
    if p.kind == index::Kind::NativeMod {
        lines.push(
            "Native code: it is compiled into the game and runs with full access to your computer. Its source was reviewed before it was listed".into(),
        );
        if let Some(v) = p.versions.first() {
            let pin = v.api.split('/').nth(1).unwrap_or_default();
            lines.push(format!(
                "Built by the launcher on Horizon {}",
                &pin[..pin.len().min(7)]
            ));
        }
    }
    if let Some(v) = p.versions.first().filter(|_| p.kind == index::Kind::Mod) {
        lines.extend(
            v.capabilities
                .iter()
                .map(|c| format!("May {}", c.describe())),
        );
        if v.capabilities.is_empty() {
            lines.push("Asks for nothing beyond its own log".into());
        }
    }
    lines
}

fn project_of(p: &index::Project) -> Option<Project> {
    let mut game_versions: Vec<String> = p.versions.iter().map(|v| v.game.clone()).collect();
    game_versions.dedup();
    Some(Project {
        source: SourceId::Horizon,
        id: p.id.clone(),
        slug: p.id.clone(),
        title: p.name.clone(),
        summary: p.summary.clone(),
        author: p.authors.join(", "),
        downloads: p.downloads,
        page: format!("{SITE}/{}", p.id),
        icon: p.icon.clone(),
        license: p.license.clone(),
        kind: kind_of(&p.kind)?,
        game_versions,
        loaders: vec![LOADER.into()],
    })
}

/// Version `v` of `p` as the browser shows it; blocked when taken down, or when it needs a newer
/// mod API level than `level`, the game's.
fn version_of(
    index: &Index,
    p: &index::Project,
    v: &index::Version,
    level: Option<u32>,
) -> Version {
    Version {
        id: v.version.clone(),
        number: v.version.clone(),
        published: v.published.clone(),
        game_versions: vec![v.game.clone()],
        loaders: vec![LOADER.into()],
        files: vec![FileRef {
            url: v.file.url.clone(),
            name: v.file.name.clone(),
            size: v.file.size,
            sha512: None,
            sha256: Some(v.file.sha256.clone()),
            sha1: None,
            primary: true,
        }],
        required: v.requires.iter().map(|r| r.id.clone()).collect(),
        blocked: index
            .revoked(&p.id, &v.file.sha256)
            .map(|r| format!("taken down: {}", r.reason))
            .or_else(|| needs_newer(v, level).then(|| NEEDS_NEWER.to_string())),
    }
}

/// A GitHub login as GitHub allows it (letters, digits, single dashes, at most 39), so an index
/// entry can't turn a link into something else.
fn github_login(login: &str) -> bool {
    (1..=39).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--")
}

/// The words of a query found in a project's id, name, summary or authors.
fn matches(p: &index::Project, text: &str) -> bool {
    let hay = format!("{} {} {} {}", p.id, p.name, p.summary, p.authors.join(" ")).to_lowercase();
    text.split_whitespace()
        .all(|word| hay.contains(&word.to_lowercase()))
}

/// The projects of the query's kind whose words match, for this game unless `everything`: the
/// most downloaded first.
fn search(index: &Index, query: &Query) -> Vec<Project> {
    let mut found: Vec<Project> = index
        .projects
        .iter()
        .filter(|p| kind_of(&p.kind) == Some(query.kind) && matches(p, &query.text))
        .filter(|p| query.everything || p.versions.iter().any(|v| v.game == GAME))
        .filter_map(project_of)
        .collect();
    found.sort_by_key(|p| std::cmp::Reverse(p.downloads));
    found
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect()
}

/// The registry as a browser source.
pub struct Registry;

impl Source for Registry {
    fn id(&self) -> SourceId {
        SourceId::Horizon
    }

    fn search(&self, query: &Query) -> Result<Vec<Project>, String> {
        let found = search(&*index()?, query);
        refresh_ratings();
        Ok(found)
    }

    fn project(&self, id: &str) -> Result<Project, String> {
        let index = index()?;
        let p = index
            .project(id)
            .ok_or_else(|| format!("{id} isn't in Horizon's registry."))?;
        project_of(p).ok_or_else(|| {
            format!(
                "{} is a {} in Horizon's registry: this launcher can't install those yet.",
                p.name,
                p.kind.name()
            )
        })
    }

    fn versions(&self, project: &Project) -> Result<Vec<Version>, String> {
        let index = index()?;
        let p = index
            .project(&project.id)
            .ok_or_else(|| format!("{} isn't in Horizon's registry.", project.title))?;
        let level = crate::backend::game_mod_api_level();
        Ok(p.versions
            .iter()
            .map(|v| version_of(&index, p, v, level))
            .collect())
    }

    fn details(&self, project: &Project) -> Result<Details, String> {
        refresh_ratings();
        let index = index()?;
        let p = index
            .project(&project.id)
            .ok_or_else(|| format!("{} isn't in Horizon's registry.", project.title))?;
        let newest = p.versions.first();
        let mut body = p.description.clone();
        if let Some(v) = newest.filter(|v| !v.capabilities.is_empty()) {
            body.push_str("\n\n## What it may do\n\n");
            for c in &v.capabilities {
                body.push_str(&format!("- May {}\n", c.describe()));
            }
        }
        if let Some(v) = newest.filter(|v| !v.requires.is_empty()) {
            body.push_str(
                "\n\n## Its mods\n\nInstalled with it into its own profile, each checked:\n\n",
            );
            for r in &v.requires {
                let found = index
                    .project(&r.id)
                    .and_then(|m| Some((m, m.versions.iter().find(|w| w.version == r.version)?)));
                let Some((m, w)) = found else { continue };
                let may: Vec<&str> = w.capabilities.iter().map(|c| c.describe()).collect();
                let may = match may.is_empty() {
                    true => String::new(),
                    false => format!(": may {}", may.join(", ")),
                };
                body.push_str(&format!("- {} {}{may}\n", m.name, w.version));
            }
        }
        // Who publishes it: the GitHub accounts the registry checked at submission.
        let owners = p.owners.iter().filter(|o| github_login(o)).map(|o| Link {
            label: format!("@{o}"),
            url: format!("https://github.com/{o}"),
        });
        Ok(Details {
            body,
            links: [
                Link {
                    label: "Source".into(),
                    url: p.source.clone(),
                },
                Link {
                    label: "Page".into(),
                    url: project.page.clone(),
                },
                Link {
                    label: "Report".into(),
                    url: format!("{REPORT}&title={}", p.id),
                },
            ]
            .into_iter()
            .chain(owners)
            .collect(),
            authors: p.authors.clone(),
            published: p
                .versions
                .last()
                .map(|v| v.published.clone())
                .unwrap_or_default(),
            updated: newest.map(|v| v.published.clone()).unwrap_or_default(),
            ..Details::default()
        })
    }

    fn fetch(
        &self,
        file: &FileRef,
        folder: &Path,
        progress: &dyn Fn(f32),
    ) -> Result<PathBuf, String> {
        fetch(file, folder, local(), progress)
    }
}

/// Downloads a registry file, checked; `file://` files only from a local index.
fn fetch(
    file: &FileRef,
    folder: &Path,
    local: bool,
    progress: &dyn Fn(f32),
) -> Result<PathBuf, String> {
    match file.url.strip_prefix("file://").filter(|_| local) {
        Some(path) => {
            let path = local_path(path);
            let reader = fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
            download::save_from_anywhere(file, reader, folder, progress)
        }
        None => download::fetch(file, folder, progress),
    }
}

/// The path of a `file:///` URL: on Windows `/C:/x` is `C:/x`.
fn local_path(path: &str) -> &str {
    match path.strip_prefix('/') {
        Some(rest) if cfg!(windows) && rest.get(1..2) == Some(":") => rest,
        _ => path,
    }
}

/// What the registry adds to an installed entry.
fn trust_label(trust: Trust) -> &'static str {
    match trust {
        Trust::Verified => "Verified",
        Trust::Unsigned => "Unsigned",
    }
}

/// The game folder path a registry project's file goes to.
fn relative(kind: Kind, id: &str, file_name: &str) -> Result<String, String> {
    match kind {
        Kind::Mod => Ok(format!("{}/{id}.hmod", java::MODS_DIR)),
        Kind::ResourcePack => Ok(format!("resourcepacks/{file_name}")),
        Kind::Shader => Ok(format!("shaderpacks/{file_name}")),
        Kind::Modpack => Err("A modpack installs as its own profile (pack::install).".into()),
        Kind::DataPack => Err("Horizon's registry has no data packs.".into()),
    }
}

/// Installs a registry project into the game folder `home`: its newest version for this game that
/// the game the player runs can load, downloaded into `mod-browser/downloads/`, checked against
/// the index, then moved into place. A mod is `horizon-mods/<id>.hmod`, turned on in
/// `horizon-mods.txt`.
pub fn install(
    project: &Project,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<installed::Entry>, String> {
    let level = crate::backend::game_mod_api_level();
    install_from(&*index()?, local(), level, project, home, progress)
}

/// [`install`] from `index`, for a game serving mod API `level`.
fn install_from(
    index: &Index,
    local: bool,
    level: Option<u32>,
    project: &Project,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<installed::Entry>, String> {
    let p = index
        .project(&project.id)
        .ok_or_else(|| format!("{} isn't in Horizon's registry.", project.title))?;
    let v = newest(index, p, level)?;
    install_version(index, local, p, v, home, progress).map(|e| vec![e])
}

/// A project's newest version for this game that wasn't taken down and that a game serving mod
/// API `level` runs (any, when the level isn't known).
fn newest<'a>(
    index: &Index,
    p: &'a index::Project,
    level: Option<u32>,
) -> Result<&'a index::Version, String> {
    let (mut taken_down, mut too_new) = (None, false);
    for v in p.versions.iter().filter(|v| v.game == GAME) {
        if let Some(r) = index.revoked(&p.id, &v.file.sha256) {
            taken_down.get_or_insert(r);
        } else if needs_newer(v, level) {
            too_new = true;
        } else {
            return Ok(v);
        }
    }
    Err(match taken_down {
        _ if too_new => format!("{} {NEEDS_NEWER}: update it first.", p.name),
        Some(r) => format!("{} was taken down: {}", p.name, r.reason),
        None => format!("{} has no version for {GAME} to install.", p.name),
    })
}

const NEEDS_NEWER: &str = "needs a newer Horizon than the game you play";

/// Whether version `v` calls mod API functions a game serving `level` doesn't have.
fn needs_newer(v: &index::Version, level: Option<u32>) -> bool {
    matches!((v.api_level, level), (Some(needed), Some(level)) if needed > level)
}

/// Installs version `v` of the registry's mod, resource pack or shader pack `p` into the game
/// folder `home`.
fn install_version(
    index: &Index,
    local: bool,
    p: &index::Project,
    v: &index::Version,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<installed::Entry, String> {
    let project = project_of(p).ok_or_else(|| format!("{} can't be installed here.", p.name))?;
    if let Some(r) = index.revoked(&p.id, &v.file.sha256) {
        return Err(format!("{} was taken down: {}", p.name, r.reason));
    }
    let native = p.kind == index::Kind::NativeMod;
    let relative = match native {
        true => format!("{}/{}.hnmod", crate::native_mods::INSTALLED, p.id),
        false => relative(project.kind, &p.id, &v.file.name)?,
    };
    let mut list = installed::Installed::load(home);
    let ours = |e: &installed::Entry| e.source == SourceId::Horizon && e.project == p.id;
    let old = list.entries.iter().find(|e| ours(e)).cloned();
    if old.as_ref().is_some_and(|o| o.version == v.version) {
        return Ok(old.expect("checked"));
    }
    let target = home.join(&relative);
    let taken_by = list.entries.iter().find(|e| !ours(e) && e.file == relative);
    let folder_mod = home.join(java::MODS_DIR).join(&p.id);
    if let Some(other) = taken_by {
        return Err(format!(
            "{relative} belongs to {}: remove it first.",
            other.title
        ));
    }
    let clash = target.exists() && old.as_ref().is_none_or(|o| o.file != relative);
    if project.kind == Kind::Mod && (clash || (!native && folder_mod.exists())) {
        return Err(format!(
            "A mod with the id {} is already in {}: move it away first.",
            p.id,
            java::MODS_DIR
        ));
    }
    if clash {
        return Err(format!("{relative} is already there: move it away first."));
    }
    let line = format!("{} {}", p.name, v.version);
    let file = &version_of(index, p, v, None).files[0];
    let staging = home.join("mod-browser/downloads");
    let path = fetch(file, &staging, local, &|f| progress(&line, f))?;
    let checked = index::check_file(index, p, v, &path).and_then(|trust| {
        // Only Rust and TOML sources, for the mod the index names: nothing in it runs here.
        match native {
            true => index::native::Bundle::read(&path).and_then(|b| match b.manifest.id == p.id {
                true => Ok(trust),
                false => Err(format!("it is {}, not {}", b.manifest.id, p.id)),
            }),
            false => Ok(trust),
        }
    });
    let trust = checked.inspect_err(|_| {
        let _ = fs::remove_file(&path);
    })?;
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::rename(&path, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    if let Some(old) = old
        .as_ref()
        .filter(|o| o.file != relative && !stays(native, &o.file))
    {
        let _ = fs::remove_file(home.join(&old.file));
    }
    if project.kind == Kind::Mod {
        java::set_enabled(home, &p.id, true)?;
    }
    let entry = installed::Entry {
        source: SourceId::Horizon,
        project: p.id.clone(),
        slug: p.id.clone(),
        title: p.name.clone(),
        kind: project.kind,
        version: v.version.clone(),
        number: v.version.clone(),
        file: relative,
        checksum: format!("SHA-256:{}", v.file.sha256),
        page: project.page.clone(),
        dependency: false,
        verdict: Some(trust_label(trust).into()),
        skipped: Vec::new(),
        namespaces: Vec::new(),
    };
    list.entries.retain(|e| !ours(e));
    list.entries.push(entry.clone());
    list.save(home)?;
    Ok(entry)
}

/// Whether an update to a native version keeps the old file `old`: a mod that turned native
/// keeps its `.hmod` until the player builds the game with the new version. The game runs it
/// meanwhile and skips it once the native one is built in; removing the mod removes both.
fn stays(native: bool, old: &str) -> bool {
    native && old.ends_with(".hmod")
}

/// The mod id of an installed mod (`horizon-mods/<id>`, `horizon-mods/<id>.hmod` or
/// `horizon-native-mods/<id>.hnmod`).
pub fn mod_id(entry: &installed::Entry) -> Option<&str> {
    if let Some(rest) = entry.file.strip_prefix(crate::native_mods::INSTALLED) {
        return rest.strip_prefix('/')?.strip_suffix(".hnmod");
    }
    let rest = entry.file.strip_prefix(java::MODS_DIR)?.strip_prefix('/')?;
    Some(rest.strip_suffix(".hmod").unwrap_or(rest))
}

/// Turns the registry's files that were taken down off in the game folder `home`, and says
/// which. Their files stay, so the player sees them and can remove them.
pub fn turn_off_revoked(index: &Index, home: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in installed::Installed::load(home).entries {
        let hash = e.checksum.strip_prefix("SHA-256:").unwrap_or_default();
        let Some(r) = (e.source == SourceId::Horizon)
            .then(|| index.revoked(&e.project, hash))
            .flatten()
        else {
            continue;
        };
        if let (Kind::Mod, Some(id)) = (e.kind, mod_id(&e)) {
            if java::is_enabled(home, id) && java::set_enabled(home, id, false).is_ok() {
                out.push(format!(
                    "{} was taken down and is off: {}",
                    e.title, r.reason
                ));
            }
        } else if matches!(e.kind, Kind::ResourcePack | Kind::Shader) {
            out.push(format!(
                "{} was taken down: {} Remove it.",
                e.title, r.reason
            ));
        }
    }
    out
}

/// Before the game starts: registry files taken down are turned off, by the index kept last
/// time (no waiting on the network), and the index is fetched again in the background for the
/// next start. Nothing happens when nothing came from the registry.
pub fn before_play(home: &Path) {
    let from_registry = installed::Installed::load(home)
        .entries
        .iter()
        .any(|e| e.source == SourceId::Horizon);
    if !from_registry {
        return;
    }
    let kept = fs::read_to_string(crate::backend::game_home().join(CACHE)).ok();
    let parse = |t: String| match local() {
        true => Index::parse_local(&t).ok(),
        false => Index::parse(&t).ok(),
    };
    if let Some(index) = kept.and_then(parse) {
        for line in turn_off_revoked(&index, home) {
            eprintln!("{line}");
        }
    }
    let home = home.to_owned();
    std::thread::spawn(move || {
        if let Ok(index) = index() {
            turn_off_revoked(&index, &home);
        }
    });
}

pub mod pack;
#[cfg(test)]
mod tests;
