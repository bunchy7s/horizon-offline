//! Modrinth's public API (api.modrinth.com/v2): no key, files with SHA-512 and SHA-1.
use super::{details, FileRef, Kind, Project, Query, Source, SourceId, Version, GAME};
use serde_json::Value;

const API: &str = "https://api.modrinth.com/v2";
const SITE: &str = "https://modrinth.com";

pub struct Modrinth;

/// Modrinth's project type for a kind.
fn project_type(kind: Kind) -> &'static str {
    match kind {
        Kind::ResourcePack => "resourcepack",
        Kind::Modpack => "modpack",
        Kind::Mod => "mod",
        Kind::Shader => "shader",
        Kind::DataPack => "datapack",
    }
}

/// The kind of a Modrinth project; its shader packs (the shaders/ format) are not offered. A
/// data pack says it is a `mod` (most also ship a mod build): the search for data packs marks
/// its hits, and an install into a world asks for its data pack files.
fn kind_of(project_type: &str) -> Option<Kind> {
    Kind::ALL
        .into_iter()
        .filter(|k| !matches!(k, Kind::Shader | Kind::DataPack))
        .find(|k| self::project_type(*k) == project_type)
}

/// The search's facets (AND of ORs): the kind, the filters, and unless `everything` what runs.
pub(super) fn facets(query: &Query) -> String {
    let f = &query.filters;
    let mut all = vec![vec![format!("project_type:{}", project_type(query.kind))]];
    match &f.game_version {
        Some(v) => all.push(vec![format!("versions:{v}")]),
        None if !query.everything
            && matches!(query.kind, Kind::ResourcePack | Kind::Mod | Kind::DataPack) =>
        {
            all.push(vec![format!("versions:{GAME}")])
        }
        None => {}
    }
    let loaders: Vec<String> = match &f.loader {
        // Quilt loads Fabric mods: what runs on Quilt.
        Some(l) if l == "quilt" => vec!["categories:quilt".into(), "categories:fabric".into()],
        Some(l) => vec![format!("categories:{l}")],
        None => details::default_loaders(query.kind)
            .iter()
            .map(|l| format!("categories:{l}"))
            .collect(),
    };
    if !loaders.is_empty() {
        all.push(loaders);
    }
    for c in &f.categories {
        all.push(vec![format!("categories:{c}")]);
    }
    match f.environment {
        Some(details::Environment::Client) => all.push(vec!["client_side!=unsupported".into()]),
        Some(details::Environment::Server) => all.push(vec!["server_side!=unsupported".into()]),
        None => {}
    }
    serde_json::to_string(&all).unwrap_or_default()
}

pub(super) fn search_url(query: &Query) -> String {
    let mut url = format!(
        "{API}/search?facets={}&index={}&offset={}&limit={}",
        encode(&facets(query)),
        query.filters.sort(&query.text).modrinth(),
        query.offset,
        query.limit.clamp(1, 100),
    );
    if !query.text.is_empty() {
        url.push_str("&query=");
        url.push_str(&encode(&query.text));
    }
    url
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// A search hit. Its `categories` hold the loaders too; `versions` its Minecraft versions.
pub(super) fn parse_hit(hit: &Value) -> Option<Project> {
    let kind = kind_of(hit["project_type"].as_str()?)?;
    let slug = text(&hit["slug"]);
    Some(Project {
        source: SourceId::Modrinth,
        id: hit["project_id"].as_str()?.to_owned(),
        page: format!("{SITE}/{}/{slug}", project_type(kind)),
        slug,
        title: text(&hit["title"]),
        summary: text(&hit["description"]),
        author: text(&hit["author"]),
        downloads: hit["downloads"].as_u64().unwrap_or(0),
        icon: hit["icon_url"]
            .as_str()
            .filter(|u| !u.is_empty())
            .map(str::to_owned),
        license: text(&hit["license"]),
        kind,
        game_versions: strings(&hit["versions"]),
        loaders: strings(&hit["categories"]),
    })
}

/// `GET /project/{id}`: the fields differ from a search hit's.
pub(super) fn parse_project(p: &Value) -> Option<Project> {
    let kind = kind_of(p["project_type"].as_str()?)?;
    let slug = text(&p["slug"]);
    Some(Project {
        source: SourceId::Modrinth,
        id: p["id"].as_str()?.to_owned(),
        page: format!("{SITE}/{}/{slug}", project_type(kind)),
        slug,
        title: text(&p["title"]),
        summary: text(&p["description"]),
        author: String::new(),
        downloads: p["downloads"].as_u64().unwrap_or(0),
        icon: p["icon_url"]
            .as_str()
            .filter(|u| !u.is_empty())
            .map(str::to_owned),
        license: text(&p["license"]["id"]),
        kind,
        game_versions: strings(&p["game_versions"]),
        loaders: strings(&p["loaders"]),
    })
}

pub(super) fn parse_version(v: &Value) -> Option<Version> {
    let files = v["files"]
        .as_array()?
        .iter()
        .filter_map(|f| {
            Some(FileRef {
                url: f["url"].as_str()?.to_owned(),
                name: f["filename"].as_str()?.to_owned(),
                size: f["size"].as_u64().unwrap_or(0),
                sha512: f["hashes"]["sha512"].as_str().map(str::to_ascii_lowercase),
                sha256: None,
                sha1: f["hashes"]["sha1"].as_str().map(str::to_ascii_lowercase),
                primary: f["primary"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    let required = v["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["dependency_type"] == "required")
        .filter_map(|d| d["project_id"].as_str().map(str::to_owned))
        .collect();
    Some(Version {
        id: v["id"].as_str()?.to_owned(),
        number: text(&v["version_number"]),
        published: text(&v["date_published"]),
        game_versions: strings(&v["game_versions"]),
        loaders: strings(&v["loaders"]),
        files,
        required,
        blocked: None,
    })
}

/// The files of one version (`GET /version/{id}`), with their downloads and checksums.
pub fn version_files(version: &str) -> Result<Vec<FileRef>, String> {
    let v = get(&format!("{API}/version/{}", encode(version)))?;
    parse_version(&v)
        .map(|v| v.files)
        .ok_or_else(|| format!("Modrinth: no version {version}."))
}

impl Source for Modrinth {
    fn id(&self) -> SourceId {
        SourceId::Modrinth
    }

    fn search(&self, query: &Query) -> Result<Vec<Project>, String> {
        if query.kind == Kind::Shader {
            return Err(super::SHADERS_ON_HORIZON.into());
        }
        let found = get(&search_url(query))?;
        Ok(found["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(parse_hit)
            .map(|p| match query.kind {
                Kind::DataPack => p.as_datapack(),
                _ => p,
            })
            .collect())
    }

    fn project(&self, id: &str) -> Result<Project, String> {
        let p = get(&format!("{API}/project/{}", encode(id)))?;
        parse_project(&p).ok_or_else(|| format!("Modrinth: {id} is not a pack Horizon loads."))
    }

    fn versions(&self, project: &Project) -> Result<Vec<Version>, String> {
        let found = get(&format!("{API}/project/{}/version", encode(&project.id)))?;
        let mut all: Vec<Version> = found
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["status"].as_str().is_none_or(|s| s == "listed"))
            .filter_map(parse_version)
            .collect();
        // Modrinth lists newest first already; ISO dates sort as text.
        all.sort_by(|a, b| b.published.cmp(&a.published));
        Ok(all)
    }

    fn details(&self, project: &Project) -> Result<details::Details, String> {
        let id = encode(&project.id);
        let p = get(&format!("{API}/project/{id}"))?;
        let members = get(&format!("{API}/project/{id}/members")).unwrap_or_default();
        Ok(details::modrinth_details(&p, &members))
    }

    fn tags(&self, kind: Kind) -> Result<details::Tags, String> {
        static TAGS: std::sync::Mutex<Option<(Value, Value, Value)>> = std::sync::Mutex::new(None);
        let mut cached = TAGS.lock().map_err(|_| "tags".to_string())?;
        if cached.is_none() {
            *cached = Some((
                get(&format!("{API}/tag/category"))?,
                get(&format!("{API}/tag/loader"))?,
                get(&format!("{API}/tag/game_version"))?,
            ));
        }
        let (c, l, v) = cached.as_ref().expect("filled above");
        // Data packs share the mods' categories; their one loader is `datapack`.
        let mut tags = match kind {
            Kind::DataPack => details::modrinth_tags("mod", c, l, v),
            _ => details::modrinth_tags(project_type(kind), c, l, v),
        };
        if kind == Kind::DataPack {
            tags.loaders.clear();
        }
        Ok(tags)
    }
}

fn get(url: &str) -> Result<Value, String> {
    let response = super::download::agent(url)
        .get(url)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(429, _) => {
                "Modrinth asks to slow down: try again in a minute.".into()
            }
            ureq::Error::Status(code, _) => format!("Modrinth answered {code}."),
            ureq::Error::Transport(t) => format!("No connection to Modrinth: {t}"),
        })?;
    response
        .into_json()
        .map_err(|e| format!("Modrinth's answer wasn't readable: {e}"))
}

/// Percent-encoding for a query value or a path segment.
pub(super) fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
