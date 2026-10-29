//! The registry repository (`horizon-rust/horizon-mods`, docs/api/mod-index.md):
//! one `mods/<id>.toml` per project, written by the submission form or by
//! hand and reviewed in a pull request, plus `revoked.toml`. Its CI turns
//! them into `index.json` with [`build`], reading every listed file, and
//! checks a pull request's changes with [`review`].
//!
//! What an entry can't say for itself (a mod's API, side and capabilities,
//! a modpack's mods, a file's size) is read from the file, never typed.
pub use super::open;
use super::{File, Index, Kind, Project, Revoked, SCHEMA, Version, hex64, inspect, valid_id};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The folder of entries in the registry repository.
pub const ENTRIES: &str = "mods";
/// The list of files taken down.
pub const REVOKED: &str = "revoked.toml";
/// Ids the site uses for its own pages (`/mods/submit`).
const RESERVED: [&str; 1] = ["submit"];
/// The biggest icon, a PNG in `icons/`.
const MAX_ICON: u64 = 256 << 10;

/// `mods/<id>.toml`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// GitHub logins allowed to publish updates.
    pub owners: Vec<String>,
    pub license: String,
    pub source: String,
    /// `icons/<id>.png` in the registry repository, or an `https://` address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The author's minisign public key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Newest first.
    pub versions: Vec<EntryVersion>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntryVersion {
    pub version: String,
    /// `YYYY-MM-DD`.
    pub published: String,
    /// The Minecraft version it is for.
    pub game: String,
    /// The name players download, with the kind's extension.
    pub file: String,
    /// Lowercase hex: pins the reviewed file.
    pub sha256: String,
    /// The `.minisig` text, when the project has a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub changelog: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokedFile {
    #[serde(default)]
    revoked: Vec<Revoked>,
}

/// Where [`build`] reads and what it writes into the index.
pub struct Sources<'a> {
    /// A checkout of the registry repository.
    pub registry: &'a Path,
    /// Every listed file, named by its SHA-256 (CI downloads them there).
    pub files: &'a Path,
    /// Where the files are published: `<files>/<id>-<version>/<file>`
    /// ([`super::FILE_HOST`]; `file:///` for a local test registry).
    pub release: &'a str,
    /// Where the registry's site is (its icons): `https://.../horizon-mods/`.
    pub site: &'a str,
    /// RFC 3339.
    pub generated: String,
}

/// The release tag a version's file is published under.
pub fn tag(id: &str, version: &str) -> String {
    format!("{id}-{version}")
}

/// Reads `mods/*.toml` and `revoked.toml`: every problem found, not only the
/// first, so a pull request's check lists them all.
pub fn load(registry: &Path) -> Result<(Vec<Entry>, Vec<Revoked>), Vec<String>> {
    let mut errors = Vec::new();
    let mut entries = Vec::new();
    let dir = registry.join(ENTRIES);
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(read) => read.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(e) => return Err(vec![format!("{}: {e}", dir.display())]),
    };
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Some(stem) = name.strip_suffix(".toml") else {
            errors.push(format!("{ENTRIES}/{name}: only <id>.toml files go here"));
            continue;
        };
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| toml::from_str::<Entry>(&text).map_err(|e| e.to_string()));
        match parsed {
            Ok(entry) if entry.id == stem => entries.push(entry),
            Ok(entry) => errors.push(format!(
                "{ENTRIES}/{name}: its id is {:?}; the file must be {ENTRIES}/{}.toml",
                entry.id, entry.id
            )),
            Err(e) => errors.push(format!("{ENTRIES}/{name}: {e}")),
        }
    }
    let revoked = match std::fs::read_to_string(registry.join(REVOKED)) {
        Ok(text) => match toml::from_str::<RevokedFile>(&text) {
            Ok(file) => file.revoked,
            Err(e) => {
                errors.push(format!("{REVOKED}: {e}"));
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            errors.push(format!("{REVOKED}: {e}"));
            Vec::new()
        }
    };
    if errors.is_empty() {
        Ok((entries, revoked))
    } else {
        Err(errors)
    }
}

/// A version string that is also a plain release tag and file name part.
fn plain_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.as_bytes()[0].is_ascii_alphanumeric()
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}

fn plain_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// One project's entry and its files, as the index lists them.
pub fn project(entry: &Entry, src: &Sources) -> Result<Project, Vec<String>> {
    let mut errors = Vec::new();
    let id = &entry.id;
    if !valid_id(id) {
        return Err(vec![format!("{id:?} is not [a-z0-9_]{{1,64}}")]);
    }
    if RESERVED.contains(&id.as_str()) {
        return Err(vec![format!("{id:?} is reserved: pick another id")]);
    }
    open(&entry.kind).map_err(|e| vec![format!("{id}: {e}")])?;
    if entry.owners.is_empty() {
        errors.push(format!("{id}: needs at least one owner (a GitHub login)"));
    }
    if entry.summary.chars().count() > 160 {
        errors.push(format!("{id}: the summary is over 160 characters"));
    }
    let icon = match entry.icon.as_deref() {
        None => None,
        Some(url) if url.starts_with("https://") => Some(url.to_owned()),
        Some(path) => {
            let ok = path
                .strip_prefix("icons/")
                .is_some_and(|name| super::plain_name(name) && name.ends_with(".png"));
            let size = std::fs::metadata(src.registry.join(path)).map(|m| m.len());
            match (ok, size) {
                (true, Ok(n)) if n <= MAX_ICON => Some(format!("{}{path}", src.site)),
                (true, Ok(_)) => {
                    errors.push(format!("{id}: {path} is over {} KiB", MAX_ICON >> 10));
                    None
                }
                (true, Err(e)) => {
                    errors.push(format!("{id}: {path}: {e}"));
                    None
                }
                (false, _) => {
                    errors.push(format!(
                        "{id}: icon {path:?} is neither icons/<name>.png nor https://"
                    ));
                    None
                }
            }
        }
    };
    if entry.versions.is_empty() {
        errors.push(format!("{id}: has no version"));
    }
    let mut versions = Vec::new();
    for v in &entry.versions {
        match version(entry, v, src) {
            // A mod that turned native keeps its `.hmod` versions published, but the index offers
            // only its native ones: a launcher never installs a `.hmod` as a native mod.
            Ok(_) if file_kind(entry, v) != entry.kind => {}
            Ok(v) => versions.push(v),
            Err(e) => errors.push(format!("{id} {}: {e}", v.version)),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Project {
        id: id.clone(),
        kind: entry.kind.clone(),
        name: entry.name.clone(),
        summary: entry.summary.clone(),
        description: entry.description.clone(),
        authors: entry.authors.clone(),
        owners: entry.owners.clone(),
        license: entry.license.clone(),
        source: entry.source.clone(),
        icon,
        key: entry.key.as_ref().map(|k| k.trim().to_owned()),
        versions,
        downloads: 0,
    })
}

/// The kind of a version's file: the project's, but a `.hmod` of a mod that turned native
/// (`review`) is a WebAssembly mod's.
fn file_kind(entry: &Entry, v: &EntryVersion) -> Kind {
    match entry.kind {
        Kind::NativeMod if v.file.ends_with(".hmod") => Kind::Mod,
        _ => entry.kind.clone(),
    }
}

fn version(entry: &Entry, v: &EntryVersion, src: &Sources) -> Result<Version, String> {
    if !plain_version(&v.version) {
        return Err("a version is letters, digits, '.', '-' and '_'".into());
    }
    if !plain_date(&v.published) {
        return Err(format!("published {:?} is not YYYY-MM-DD", v.published));
    }
    hex64(&v.sha256)?;
    let path = src.files.join(&v.sha256);
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("its file {}: {e}", v.sha256))?
        .len();
    if super::sha256_file(&path)? != v.sha256 {
        return Err("the file isn't the one its sha256 names".into());
    }
    match (&entry.key, &v.signature) {
        (Some(key), Some(sig)) => super::verify_signature(key, sig, &path)?,
        (Some(_), None) => return Err("the project has a key: sign the file with it".into()),
        (None, Some(_)) => return Err("signed, but the project has no key".into()),
        (None, None) => {}
    }
    let facts = inspect(&file_kind(entry, v), &path)?;
    if facts.id.as_ref().is_some_and(|id| *id != entry.id) {
        return Err(format!(
            "the file's id is {:?}, not {:?}",
            facts.id.unwrap_or_default(),
            entry.id
        ));
    }
    if facts.version.as_ref().is_some_and(|x| *x != v.version) {
        return Err(format!(
            "the file says version {:?}",
            facts.version.unwrap_or_default()
        ));
    }
    if facts.game.as_ref().is_some_and(|g| *g != v.game) {
        return Err(format!(
            "the file is for {:?}, the entry says {:?}",
            facts.game.unwrap_or_default(),
            v.game
        ));
    }
    Ok(Version {
        version: v.version.clone(),
        published: v.published.clone(),
        changelog: v.changelog.clone(),
        api: facts.api,
        side: facts.side,
        capabilities: facts.capabilities,
        api_level: facts.api_level,
        game: v.game.clone(),
        requires: facts.requires,
        file: File {
            url: format!("{}{}/{}", src.release, tag(&entry.id, &v.version), v.file),
            name: v.file.clone(),
            size,
            sha256: v.sha256.clone(),
            signature: v.signature.clone(),
        },
    })
}

impl Entry {
    /// The entry as `mods/<id>.toml` holds it (the submission form writes it).
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }
}

/// The index of the whole registry: every entry and file checked, then the
/// index's own rules ([`Index::check`]: names, a modpack's mods present).
pub fn build(src: &Sources) -> Result<Index, Vec<String>> {
    let (mut entries, revoked) = load(src.registry)?;
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    let mut errors = Vec::new();
    let mut projects = Vec::new();
    for entry in &entries {
        match project(entry, src) {
            Ok(p) => projects.push(p),
            Err(e) => errors.extend(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let index = Index {
        schema: SCHEMA,
        generated: src.generated.clone(),
        projects,
        revoked,
    };
    let local = src.release.starts_with("file:///");
    index.check_with(local).map_err(|e| vec![e])?;
    Ok(index)
}

/// What a pull request changes, for its reviewers.
#[derive(Debug, Default, PartialEq)]
pub struct Review {
    /// Changes the registry refuses whoever reviews them.
    pub errors: Vec<String>,
    /// What reviewers should look at: new projects, new owners or keys, a
    /// change by someone who isn't an owner.
    pub notes: Vec<String>,
}

/// Compares the entries of a pull request (`head`) with `main`'s (`base`):
/// a published file never changes and is never removed (take it down in
/// `revoked.toml`), a project's kind never changes but for a mod turning
/// native (its next versions are `.hnmod`s), and changes by someone who
/// isn't an owner, or to owners and keys, are pointed out.
pub fn review(base: &[Entry], head: &[Entry], author: &str) -> Review {
    let mut r = Review::default();
    for old in base {
        let Some(new) = head.iter().find(|e| e.id == old.id) else {
            r.errors.push(format!(
                "{}: removed; take its files down in {REVOKED} instead",
                old.id
            ));
            continue;
        };
        if new == old {
            continue;
        }
        if !old.owners.iter().any(|o| o.eq_ignore_ascii_case(author)) {
            r.notes.push(format!(
                "{}: changed by {author}, who isn't one of its owners ({})",
                old.id,
                old.owners.join(", ")
            ));
        }
        if (&old.kind, &new.kind) == (&Kind::Mod, &Kind::NativeMod) {
            r.notes.push(format!(
                "{}: turns native; its .hmod versions stay published, and launchers keep the installed copies",
                old.id
            ));
        } else if new.kind != old.kind {
            r.errors.push(format!("{}: its kind can't change", old.id));
        }
        for v in &old.versions {
            match new.versions.iter().find(|w| w.version == v.version) {
                None => r.errors.push(format!(
                    "{} {}: removed; take it down in {REVOKED} instead",
                    old.id, v.version
                )),
                Some(w) if w.sha256 != v.sha256 || w.file != v.file => r.errors.push(format!(
                    "{} {}: a published file never changes: publish a new version",
                    old.id, v.version
                )),
                Some(_) => {}
            }
        }
        if new.owners != old.owners {
            r.notes.push(format!(
                "{}: owners {} -> {}",
                old.id,
                old.owners.join(", "),
                new.owners.join(", ")
            ));
        }
        if new.key != old.key {
            r.notes.push(format!(
                "{}: its signing key changes; updates signed with the old one stop working",
                old.id
            ));
        }
        let added = new.versions.len().saturating_sub(old.versions.len());
        if added > 0 {
            r.notes.push(format!("{}: {added} new version(s)", old.id));
        }
    }
    for new in head {
        if !base.iter().any(|e| e.id == new.id) {
            r.notes.push(format!(
                "{}: a new {} by {}",
                new.id,
                new.kind.name(),
                new.owners.join(", ")
            ));
        }
    }
    r
}

#[cfg(test)]
mod tests;
