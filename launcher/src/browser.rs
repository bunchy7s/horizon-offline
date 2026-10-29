//! The mod browser's sources: search Modrinth (CurseForge support is coming soon) for what
//! Horizon loads, say how well each result runs, and download a file only after its checksum
//! matches (docs/mod-browser.md).
//!
//! Offered: Java mods (Fabric, NeoForge, Forge), whose content Horizon ports and whose jar is
//! checked before install ([`java`]), modpacks (imported and ported), resource packs
//! (`resourcepacks/`) and data packs (into a world, [`installed::install_datapack`]). Java code
//! itself doesn't run in Horizon (docs/mods.md).
pub mod curseforge;
pub mod details;
pub mod download;
pub mod horizon;
pub mod images;
pub mod installed;
pub mod java;
pub mod java_instance;
pub mod modrinth;

use serde::{Deserialize, Serialize};

/// The Minecraft version Horizon plays. A version port updates it with the other `"26.3"` sites
/// (docs/version-ports.md).
pub const GAME: &str = "26.3";

/// What a project is, as far as Horizon is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    ResourcePack,
    Modpack,
    /// A Java mod: its content is ported ([`java`]).
    Mod,
    /// One of Horizon's own shader packs (`.hshader`, docs/shaders.md), from Horizon's registry
    /// only.
    Shader,
    /// A data pack: it goes into one world's `datapacks/` (`crate::datapacks`).
    DataPack,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Mod,
        Kind::Modpack,
        Kind::ResourcePack,
        Kind::Shader,
        Kind::DataPack,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Mod => "Mods",
            Kind::ResourcePack => "Resource packs",
            Kind::Modpack => "Modpacks",
            Kind::Shader => "Shader packs",
            Kind::DataPack => "Data packs",
        }
    }

    /// The folder beside `options.txt` the file goes in; a modpack is imported, a mod ported
    /// and a data pack put in a world instead.
    pub fn folder(self) -> Option<&'static str> {
        match self {
            Kind::ResourcePack => Some("resourcepacks"),
            Kind::Shader => Some("shaderpacks"),
            Kind::Modpack | Kind::Mod | Kind::DataPack => None,
        }
    }
}

/// Where a project comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceId {
    Modrinth,
    CurseForge,
    /// Horizon's own registry of reviewed mods, modpacks and resource packs ([`horizon`]).
    Horizon,
}

impl SourceId {
    pub fn label(self) -> &'static str {
        match self {
            SourceId::Modrinth => "Modrinth",
            SourceId::CurseForge => "CurseForge",
            SourceId::Horizon => "Horizon",
        }
    }
}

/// How well a project runs in Horizon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    /// Made for the version Horizon plays, and Horizon reads everything it uses.
    Works,
    /// It loads, with the limit said.
    Partial(String),
    /// It can't run in Horizon: never offered for install.
    No(String),
}

impl Support {
    pub fn installable(&self) -> bool {
        !matches!(self, Support::No(_))
    }

    pub fn label(&self) -> &'static str {
        match self {
            Support::Works => "Works",
            Support::Partial(_) => "Partly",
            Support::No(_) => "Doesn't run",
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Support::Works => None,
            Support::Partial(r) | Support::No(r) => Some(r),
        }
    }
}

/// A search result or a looked-up project.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub source: SourceId,
    pub id: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub author: String,
    pub downloads: u64,
    /// The project's page on its source, for the player to read before installing.
    pub page: String,
    pub icon: Option<String>,
    pub license: String,
    pub kind: Kind,
    /// The Minecraft versions its files list, and the loaders (`fabric`, `neoforge`, `minecraft`...).
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
}

impl Project {
    pub fn support(&self) -> Support {
        support(self.kind, &self.game_versions, &self.loaders)
    }

    /// The project as a data pack, for a search of data packs or an install into a world;
    /// its page is Modrinth's data pack page.
    pub fn as_datapack(self) -> Project {
        Project {
            page: match self.source {
                SourceId::Modrinth => format!("https://modrinth.com/datapack/{}", self.slug),
                _ => self.page,
            },
            kind: Kind::DataPack,
            ..self
        }
    }
}

/// One downloadable file of a version, with the checksums its source publishes.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRef {
    pub url: String,
    pub name: String,
    pub size: u64,
    pub sha512: Option<String>,
    /// What Horizon's registry lists.
    pub sha256: Option<String>,
    pub sha1: Option<String>,
    pub primary: bool,
}

impl FileRef {
    /// The strongest checksum the source gives: what the download is checked against.
    pub fn checksum(&self) -> Option<(&'static str, &str)> {
        [
            ("SHA-512", &self.sha512),
            ("SHA-256", &self.sha256),
            ("SHA-1", &self.sha1),
        ]
        .into_iter()
        .find_map(|(algo, h)| Some((algo, h.as_deref()?)))
    }
}

/// A version of a project: its files and what it needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub id: String,
    pub number: String,
    pub published: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<FileRef>,
    /// Required dependencies, as project ids on the same source.
    pub required: Vec<String>,
    /// Why this version can't be downloaded from here (CurseForge authors can forbid it).
    pub blocked: Option<String>,
}

impl Version {
    /// The file to install: the primary one, else the first.
    pub fn file(&self) -> Option<&FileRef> {
        self.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| self.files.first())
    }
}

/// A search.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub text: String,
    pub kind: Kind,
    /// Off: only what runs (made for [`GAME`]). On: everything of the kind, each result marked
    /// with how well it runs.
    pub everything: bool,
    pub offset: usize,
    pub limit: usize,
    pub filters: details::Filters,
}

impl Query {
    pub fn new(text: &str, kind: Kind) -> Self {
        Self {
            text: text.trim().to_owned(),
            kind,
            everything: false,
            offset: 0,
            limit: 20,
            filters: details::Filters::default(),
        }
    }
}

/// A place projects come from.
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;
    fn search(&self, query: &Query) -> Result<Vec<Project>, String>;
    fn project(&self, id: &str) -> Result<Project, String>;
    /// Versions, newest first.
    fn versions(&self, project: &Project) -> Result<Vec<Version>, String>;
    /// The project's page: description, gallery, links, authors.
    fn details(&self, project: &Project) -> Result<details::Details, String> {
        Ok(details::Details {
            body: project.summary.clone(),
            ..details::Details::default()
        })
    }
    /// The values the search's filters can take for `kind`.
    fn tags(&self, _kind: Kind) -> Result<details::Tags, String> {
        Ok(details::Tags::default())
    }
    /// Downloads one of its files into `folder`, checked ([`download::fetch`]).
    fn fetch(
        &self,
        file: &FileRef,
        folder: &std::path::Path,
        progress: &dyn Fn(f32),
    ) -> Result<std::path::PathBuf, String> {
        download::fetch(file, folder, progress)
    }
}

/// What the launcher says wherever CurseForge would be.
pub const CURSEFORGE_SOON: &str = "CurseForge support is coming soon.";
/// What another source answers for shader packs: Horizon's own are on its registry only.
pub const SHADERS_ON_HORIZON: &str =
    "Horizon's shader packs are on Horizon's registry: pick Horizon.";

/// The sources this launcher uses: Modrinth and Horizon's registry. CurseForge support is coming
/// soon: its API needs a paid key, so no search or install goes to it; `curseforge` is kept for
/// then.
pub fn sources() -> Vec<Box<dyn Source>> {
    vec![Box::new(modrinth::Modrinth), Box::new(horizon::Registry)]
}

/// The source of that id, when this launcher can use it.
pub fn source(id: SourceId) -> Option<Box<dyn Source>> {
    sources().into_iter().find(|s| s.id() == id)
}

/// How well a project of `kind` made for `game_versions` and `loaders` runs in Horizon.
pub fn support(kind: Kind, game_versions: &[String], loaders: &[String]) -> Support {
    if loaders.iter().any(|l| l == horizon::LOADER) {
        // Made for Horizon and reviewed: it runs when it is for the version Horizon plays.
        return match newest_release(game_versions)
            .filter(|_| !game_versions.iter().any(|v| v == GAME))
        {
            Some(other) => Support::No(format!("Made for Horizon on {other}.")),
            None => Support::Works,
        };
    }
    match kind {
        Kind::ResourcePack => {
            if game_versions.iter().any(|v| v == GAME) {
                Support::Works
            } else if let Some(newest) = newest_release(game_versions) {
                Support::Partial(format!(
                    "Made for {newest}: it loads with vanilla's compatibility warning, and \
                     what changed since may look wrong."
                ))
            } else {
                Support::Partial("Lists no Minecraft version.".into())
            }
        }
        Kind::Modpack => {
            Support::Partial("Horizon runs the mods it can port, from your own downloads.".into())
        }
        Kind::Shader => Support::No("Not one of Horizon's own shader packs.".into()),
        Kind::DataPack => {
            let made_for = match newest_release(game_versions) {
                _ if game_versions.iter().any(|v| v == GAME) => String::new(),
                Some(newest) => format!("Made for {newest}: the game may refuse it. "),
                None => "Lists no Minecraft version. ".into(),
            };
            Support::Partial(format!("{made_for}{DATAPACK_SUPPORT}"))
        }
        Kind::Mod => {
            if !loaders.iter().any(|l| java::LOADERS.contains(&l.as_str())) {
                Support::No("Not a Fabric, NeoForge or Forge mod.".into())
            } else if !game_versions.iter().any(|v| portable_version(v)) {
                let newest = newest_release(game_versions).unwrap_or("an unknown version");
                Support::No(format!(
                    "Made for {newest}: Horizon ports mods made for 1.21 and later."
                ))
            } else {
                Support::Partial(
                    "Not checked yet: the launcher reads its jar to say what of it ports.".into(),
                )
            }
        }
    }
}

/// What a data pack does in Horizon today, said before install; the install says it for the
/// pack's own files (`crate::datapacks::coverage`).
pub const DATAPACK_SUPPORT: &str = "Horizon runs a data pack's functions, function tags, \
     predicates and item modifiers; not yet its recipes, loot tables, advancements, other tags \
     or worldgen.";

/// A Minecraft version whose mods port-pack carries over: 1.21 and later (the data folders
/// took their current names in 1.21).
pub fn portable_version(v: &str) -> bool {
    let parts: Vec<u32> = v.split('.').map_while(|p| p.parse().ok()).collect();
    match parts.as_slice() {
        [1, minor, ..] => *minor >= 21 && v.split('.').count() == parts.len(),
        [year, ..] => *year >= 26 && v.split('.').count() == parts.len(),
        [] => false,
    }
}

/// The newest plain release (`1.21.4`, `26.2`) in a list of Minecraft versions.
fn newest_release(versions: &[String]) -> Option<&str> {
    versions
        .iter()
        .filter(|v| {
            v.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
        .max_by_key(|v| {
            v.split('.')
                .map(|p| p.parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
        .map(String::as_str)
}

/// The version to install: the newest one that runs, or with `everything` the newest at all.
/// `versions` is newest first, as [`Source::versions`] gives them.
pub fn pick(kind: Kind, versions: &[Version], everything: bool) -> Option<&Version> {
    let mut usable = versions
        .iter()
        .filter(|v| v.blocked.is_none() && v.file().is_some());
    if versions
        .iter()
        .any(|v| v.loaders.iter().any(|l| l == horizon::LOADER))
    {
        return usable.find(|v| v.game_versions.iter().any(|g| g == GAME));
    }
    if kind == Kind::Mod {
        let java = |v: &&Version| {
            v.loaders
                .iter()
                .any(|l| java::LOADERS.contains(&l.as_str()))
        };
        let made_for = |v: &&Version| v.game_versions.iter().any(|g| g == GAME);
        let ported = |v: &&Version| v.game_versions.iter().any(|g| portable_version(g));
        return usable
            .clone()
            .filter(java)
            .find(made_for)
            .or_else(|| usable.filter(java).find(|v| everything && ported(v)));
    }
    if kind == Kind::DataPack {
        // Most data packs on Modrinth ship a mod build too: only the data pack files.
        let mut packs = usable.filter(|v| v.loaders.iter().any(|l| l == "datapack"));
        let made_for = |v: &&Version| v.game_versions.iter().any(|g| g == GAME);
        return packs
            .clone()
            .find(made_for)
            .or_else(|| packs.find(|_| everything));
    }
    let runs = |v: &&Version| match support(kind, &v.game_versions, &v.loaders) {
        Support::Works => true,
        // A modpack's version is for its own Minecraft.
        Support::Partial(_) => kind != Kind::ResourcePack,
        Support::No(_) => false,
    };
    if let Some(v) = usable.clone().find(runs) {
        return Some(v);
    }
    usable.find(|v| everything && support(kind, &v.game_versions, &v.loaders).installable())
}

/// A file name from a source, kept only when it is one plain name: no folders, no `..`, no
/// hidden file, nothing a file system would read as something else.
pub fn safe_name(name: &str) -> Option<&str> {
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.ends_with(['.', ' '])
        && name.chars().all(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    ok.then_some(name)
}

#[cfg(test)]
mod tests;
