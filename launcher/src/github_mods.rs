//! Native mods installed from a GitHub link (docs/native-mods.md, "Install from GitHub"), so an
//! author can ship a version without a registry pull request. The link names a repository (its
//! latest release), a release tag, a branch or tag (`tree/`) or a commit. It is resolved through
//! GitHub's public API to an exact **release asset** (a `.hnmod`, kept by its SHA-256) or an exact
//! **commit** (the archive of that commit, whose mod folder is packed into a `.hnmod`), never to a
//! branch that moves. The result passes the checks the registry's `.hnmod`s pass
//! (`horizon_mods::index::native::Bundle::read`) before anything is installed, and the origin is
//! kept beside the file (`horizon-native-mods/<id>.origin.json`) for "check for updates".
//!
//! Nothing here runs the mod's code, and nothing here trusts it: the registry has not reviewed it.
//! Callers show [`WARNING`] and ask first (`--trust-native-code`).
use crate::backend;
use crate::browser::java;
use crate::native_mods::{self, Bundle, INSTALLED};
use horizon_mods::index::native as format;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// What a player is told before a mod from GitHub is installed.
pub const WARNING: &str = "Not reviewed by the Horizon registry. Native mods run with full access to this computer. Read the source first.";

/// The most a release asset or a mod's archive may be.
const MAX_ASSET: u64 = 64 << 20;
const MAX_ARCHIVE: u64 = 256 << 20;
/// How deep below the archive's root a mod folder is looked for.
const DEPTH: usize = 6;

/// What a link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The repository: its latest release, else its default branch.
    Latest,
    /// The head of the default branch (what a mod installed from a commit is updated to).
    Head,
    /// `releases/tag/<tag>`: that release, else the tag itself.
    Tag(String),
    /// `tree/<ref>`: a branch, tag or commit.
    Ref(String),
    /// `commit/<sha>`.
    Commit(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub owner: String,
    pub repo: String,
    pub target: Target,
}

fn plain(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A ref (branch, tag, sha) as GitHub names them: no spaces, no `..`, nothing a URL would read.
fn ref_ok(r: &str) -> bool {
    !r.is_empty()
        && !r.contains("..")
        && !r.starts_with('/')
        && !r.ends_with('/')
        && r.bytes().all(|b| {
            b.is_ascii_graphic()
                && !matches!(
                    b,
                    b'?' | b'#' | b'%' | b'\\' | b'*' | b'^' | b'~' | b':' | b'[' | b'@'
                )
        })
}

/// Parses `https://github.com/<owner>/<repo>[/releases/tag/<tag> | /tree/<ref> | /commit/<sha>]`.
pub fn parse(text: &str) -> Result<Link, String> {
    let bad = || {
        format!(
            "{:?} is not a GitHub link: use https://github.com/<owner>/<repo>, or .../releases/tag/<tag>, .../tree/<ref> or .../commit/<sha>.",
            text.trim()
        )
    };
    let text = text.trim();
    let rest = text
        .strip_prefix("https://github.com/")
        .or_else(|| text.strip_prefix("https://www.github.com/"))
        .ok_or_else(bad)?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = rest.trim_end_matches('/').splitn(3, '/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if !plain(owner) || !plain(repo) {
        return Err(bad());
    }
    let target = match parts.next() {
        None => Target::Latest,
        Some(tail) => {
            let tail = tail.trim_end_matches('/');
            let value = |prefix: &str| {
                tail.strip_prefix(prefix)
                    .filter(|v| ref_ok(v))
                    .map(str::to_owned)
            };
            if let Some(tag) = value("releases/tag/") {
                Target::Tag(tag)
            } else if let Some(r) = value("tree/") {
                Target::Ref(r)
            } else if let Some(sha) = value("commit/")
                .filter(|s| (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                Target::Commit(sha.to_ascii_lowercase())
            } else if matches!(tail, "releases" | "releases/latest") {
                Target::Latest
            } else {
                return Err(bad());
            }
        }
    };
    Ok(Link {
        owner: owner.into(),
        repo: repo.into(),
        target,
    })
}

/// A release's file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// The SHA-256 GitHub reports for it (`digest`), when it does: the download must match.
    pub sha256: Option<String>,
}

/// A link resolved to something exact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub owner: String,
    pub repo: String,
    /// The release it came from, if any.
    pub tag: Option<String>,
    /// The full commit: what the source link names, and what a commit install fetches.
    pub commit: String,
    /// The release's `.hnmod`, installed as it is when there is one.
    pub asset: Option<Asset>,
    /// A release that ships only a content mod (`.hmod`): not supported here yet.
    pub content_only: Option<String>,
}

impl Resolved {
    pub fn name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// `owner/repo @ tag` for a release, `owner/repo @ 7-digit sha` for a commit.
    pub fn label(&self) -> String {
        label(&self.name(), self.tag.as_deref(), &self.commit)
    }

    /// Where to read exactly what would be installed.
    pub fn source_url(&self) -> String {
        source_url(&self.owner, &self.repo, &self.commit)
    }
}

fn label(name: &str, tag: Option<&str>, commit: &str) -> String {
    match tag {
        Some(tag) => format!("{name} @ {tag}"),
        None => format!("{name} @ {}", &commit[..commit.len().min(7)]),
    }
}

pub fn source_url(owner: &str, repo: &str, commit: &str) -> String {
    format!("https://github.com/{owner}/{repo}/tree/{commit}")
}

/// GitHub's JSON for an API path (`repos/...`); `None` when it answers 404.
pub type Api<'a> = &'a dyn Fn(&str) -> Result<Option<serde_json::Value>, String>;

/// The real API, unauthenticated (`HORIZON_GITHUB_TOKEN` is used when set, for CI).
pub fn api(path: &str) -> Result<Option<serde_json::Value>, String> {
    let url = format!("https://api.github.com/{path}");
    match backend::get(&url) {
        Ok(response) => response.into_json().map(Some).map_err(|e| e.to_string()),
        Err(e) if e.ends_with("answered 404") => Ok(None),
        Err(e) if e.ends_with("answered 429") => Err(RATE_LIMIT.into()),
        Err(e) if e.contains("rate limit") => Err(RATE_LIMIT.into()),
        Err(e) => Err(e),
    }
}

const RATE_LIMIT: &str =
    "GitHub's rate limit for anonymous requests was reached. Try again in a few minutes.";

/// Percent-encodes a ref for a URL path, keeping its `/`.
fn encode(r: &str) -> String {
    r.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn commit_of(api: Api, link: &Link, reference: &str) -> Result<String, String> {
    let path = format!(
        "repos/{}/{}/commits/{}",
        link.owner,
        link.repo,
        encode(reference)
    );
    let found = api(&path)?.ok_or_else(|| {
        format!(
            "{}/{} has no commit, tag or branch {reference}.",
            link.owner, link.repo
        )
    })?;
    found["sha"]
        .as_str()
        .filter(|s| format::revision(s))
        .map(str::to_owned)
        .ok_or_else(|| format!("GitHub did not name the commit of {reference}."))
}

fn default_head(api: Api, link: &Link) -> Result<String, String> {
    let repo = api(&format!("repos/{}/{}", link.owner, link.repo))?
        .ok_or_else(|| format!("{}/{} doesn't exist, or is private.", link.owner, link.repo))?;
    let branch = repo["default_branch"]
        .as_str()
        .ok_or("GitHub did not name the default branch.")?;
    commit_of(api, link, branch)
}

/// The `.hnmod` of a release's JSON (and a lone `.hmod`, which is not installed here).
fn assets(release: &serde_json::Value, link: &Link) -> (Option<Asset>, Option<String>) {
    let prefix = format!(
        "https://github.com/{}/{}/releases/download/",
        link.owner, link.repo
    );
    let list = release["assets"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let named = |ext: &'static str| {
        list.iter()
            .filter(move |a| a["name"].as_str().is_some_and(|n| n.ends_with(ext)))
    };
    let hnmod = named(".hnmod").find_map(|a| {
        let url = a["browser_download_url"].as_str()?.to_owned();
        let name = a["name"].as_str()?.to_owned();
        // Only this repository's own release files.
        url.starts_with(&prefix).then(|| Asset {
            name,
            url,
            sha256: a["digest"]
                .as_str()
                .and_then(|d| d.strip_prefix("sha256:"))
                .filter(|d| d.len() == 64)
                .map(str::to_ascii_lowercase),
        })
    });
    let hmod = named(".hmod")
        .find_map(|a| a["name"].as_str().map(str::to_owned))
        .filter(|_| hnmod.is_none());
    (hnmod, hmod)
}

fn from_release(api: Api, link: &Link, release: &serde_json::Value) -> Result<Resolved, String> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or("GitHub did not name the release's tag.")?
        .to_owned();
    let (asset, content_only) = assets(release, link);
    Ok(Resolved {
        owner: link.owner.clone(),
        repo: link.repo.clone(),
        commit: commit_of(api, link, &tag)?,
        tag: Some(tag),
        asset,
        content_only,
    })
}

fn at_commit(commit: String, link: &Link) -> Resolved {
    Resolved {
        owner: link.owner.clone(),
        repo: link.repo.clone(),
        tag: None,
        commit,
        asset: None,
        content_only: None,
    }
}

/// Resolves `link` to an exact release asset or commit, through `api`.
pub fn resolve(link: &Link, api: Api) -> Result<Resolved, String> {
    let base = format!("repos/{}/{}", link.owner, link.repo);
    match &link.target {
        Target::Latest => match api(&format!("{base}/releases/latest"))? {
            Some(release) => from_release(api, link, &release),
            None => Ok(at_commit(default_head(api, link)?, link)),
        },
        Target::Head => Ok(at_commit(default_head(api, link)?, link)),
        Target::Tag(tag) => match api(&format!("{base}/releases/tags/{}", encode(tag)))? {
            Some(release) => from_release(api, link, &release),
            None => Ok(at_commit(commit_of(api, link, tag)?, link)),
        },
        Target::Ref(r) | Target::Commit(r) => Ok(at_commit(commit_of(api, link, r)?, link)),
    }
}

/// Where a mod came from, kept beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub id: String,
    pub version: String,
    pub owner: String,
    pub repo: String,
    /// The release it came from, when it is a release's asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub commit: String,
    /// The release asset's name, when it came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// The SHA-256 of the installed `.hnmod`.
    pub sha256: String,
}

impl Origin {
    /// "from GitHub: owner/repo @ tag" (or the commit's first 7 digits).
    pub fn label(&self) -> String {
        label(
            &format!("{}/{}", self.owner, self.repo),
            self.tag.as_deref(),
            &self.commit,
        )
    }

    pub fn source_url(&self) -> String {
        source_url(&self.owner, &self.repo, &self.commit)
    }

    /// What its update is read from: the latest release for a mod from a release, the default
    /// branch's head for one from a commit.
    pub fn follows(&self) -> Link {
        Link {
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            target: match self.tag {
                Some(_) => Target::Latest,
                None => Target::Head,
            },
        }
    }
}

/// Whether `latest` (what [`Origin::follows`] resolves to) is newer than what is installed.
pub fn is_update(origin: &Origin, latest: &Resolved) -> bool {
    match (&origin.tag, &latest.tag) {
        (Some(have), Some(new)) => have != new,
        // A release's mod whose repository now has no release asset to follow.
        (Some(_), None) => false,
        (None, _) => origin.commit != latest.commit,
    }
}

/// A mod ready to install: its `.hnmod` was read and checked, nothing run.
#[derive(Clone, Debug)]
pub struct Packed {
    pub id: String,
    pub version: String,
    pub file: PathBuf,
    pub sha256: String,
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    native_mods::root().join("github").join(name)
}

/// Reads `bytes` as `.hnmod`, with the registry's checks, into a file `Packed` names.
fn checked(bytes: &[u8], dir: &Path) -> Result<Packed, String> {
    let sum = sha256(bytes);
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("{}.hnmod", &sum[..16]));
    fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    let bundle = format::Bundle::read(&file).inspect_err(|_| {
        let _ = fs::remove_file(&file);
    })?;
    Ok(Packed {
        id: bundle.manifest.id,
        version: bundle.manifest.version,
        file,
        sha256: sum,
    })
}

/// Every mod folder under `root`: a `horizon-mixin.toml`, or an unpacked `.hnmod`
/// (`horizon-native.json`), the rules of `native-build <folder>`. A mod's folder is not searched
/// further.
pub fn find_mods(root: &Path) -> Result<Vec<Bundle>, String> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Bundle>) -> Result<(), String> {
        if let Some(found) = native_mods::bundle_in(dir)? {
            out.push(found);
            return Ok(());
        }
        if depth == 0 {
            return Ok(());
        }
        let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                !name.starts_with('.') && name != "target" && name != "node_modules"
            })
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for sub in dirs {
            walk(&sub, depth - 1, out)?;
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(root, DEPTH, &mut found)?;
    found.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(found)
}

fn walk_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!(
                "{} is a link: not allowed in a mod.",
                path.display()
            ));
        } else if kind.is_dir() {
            walk_files(root, &path, out)?;
        } else {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            out.push((relative.to_string_lossy().replace('\\', "/"), path));
        }
    }
    Ok(())
}

/// The `.hnmod` of a mod folder, as bytes: its source (Rust, TOML and `data/`) with a manifest made
/// from its `horizon-mixin.toml`, which must pin the Horizon commit it was made for
/// (`[compatibility] horizon_revision`). An unpacked `.hnmod` (`horizon-native.json`) keeps the
/// pins and hook points its manifest names. The zip is the same for the same files.
pub fn pack(bundle: &Bundle) -> Result<Vec<u8>, String> {
    let horizon = bundle.horizon.clone().filter(|h| format::revision(h)).ok_or_else(|| {
        format!(
            "{} has no [compatibility] horizon_revision (40 hex digits): add the Horizon commit it was made for.",
            bundle.id
        )
    })?;
    let manifest = format::Manifest {
        schema: 1,
        id: bundle.id.clone(),
        version: bundle.version.clone(),
        game: crate::browser::GAME.into(),
        horizon,
        mixins: bundle
            .mixins
            .clone()
            .unwrap_or_else(|| native_mods::MIXINS_REV.into()),
        hooks: bundle.hooks,
    };
    manifest.check()?;
    let mut put = vec![(
        format::MANIFEST.to_owned(),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )];
    let mut files = Vec::new();
    walk_files(&bundle.dir, &bundle.dir, &mut files)?;
    for (name, path) in files {
        // What the registry allows in a bundle: sources, data and crates' text files.
        if format::allowed(&name) {
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            put.push((format!("mixins/{}/{name}", bundle.id), bytes));
        }
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (name, bytes) in put {
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    backend::get(url)?
        .into_reader()
        .take(MAX_ASSET + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("Download interrupted: {e}"))?;
    if body.len() as u64 > MAX_ASSET {
        return Err("The file is bigger than 64 MiB: not a native mod bundle.".into());
    }
    Ok(body)
}

/// Downloads what `found` names and checks it: the release's `.hnmod` (its SHA-256 compared with
/// GitHub's when it reports one), else the mods of the commit's archive, each packed. A repository
/// of several mods gives several `Packed`. Nothing is installed.
pub fn fetch(found: &Resolved) -> Result<Vec<Packed>, String> {
    let dir = scratch(&found.commit);
    if let Some(asset) = &found.asset {
        let bytes = download(&asset.url)?;
        if let Some(expected) = &asset.sha256 {
            if sha256(&bytes) != *expected {
                return Err(format!(
                    "{} does not match the checksum GitHub lists for it.",
                    asset.name
                ));
            }
        }
        return Ok(vec![checked(&bytes, &dir)?]);
    }
    let url = format!(
        "https://codeload.github.com/{}/{}/tar.gz/{}",
        found.owner, found.repo, found.commit
    );
    let top = native_mods::unpack_archive(&url, &dir.join("source"), MAX_ARCHIVE)?;
    let mods = find_mods(&top)?;
    if mods.is_empty() {
        return Err(match &found.content_only {
            Some(file) => format!(
                "{} ships {file}, a content mod: installing .hmod files from GitHub is not supported yet.",
                found.label()
            ),
            None => format!(
                "{} holds no native mod (a folder with horizon-mixin.toml, or an unpacked .hnmod).",
                found.label()
            ),
        });
    }
    let packed: Result<Vec<Packed>, String> = mods
        .iter()
        .map(|m| checked(&pack(m)?, &dir).map_err(|e| format!("{}: {e}", m.id)))
        .collect();
    let _ = fs::remove_dir_all(dir.join("source"));
    packed
}

/// Deletes what [`fetch`] left for `found` (the files of mods not installed).
pub fn discard(found: &Resolved) {
    let _ = fs::remove_dir_all(scratch(&found.commit));
}

/// Which of `found` to install: the one named, all of them, or the only one. Several with no
/// choice is an error that lists them.
pub fn choose<'a>(
    found: &'a [Packed],
    id: Option<&str>,
    all: bool,
) -> Result<Vec<&'a Packed>, String> {
    if let Some(id) = id {
        return found
            .iter()
            .find(|p| p.id == id)
            .map(|p| vec![p])
            .ok_or_else(|| format!("No mod {id} there. It holds: {}.", ids(found)));
    }
    match found {
        [one] => Ok(vec![one]),
        _ if all => Ok(found.iter().collect()),
        _ => Err(format!(
            "It holds {} mods: {}. Pick one (--mod ID) or install them all (--all).",
            found.len(),
            ids(found)
        )),
    }
}

fn ids(found: &[Packed]) -> String {
    found
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn origin_path(home: &Path, id: &str) -> PathBuf {
    home.join(INSTALLED).join(format!("{id}.origin.json"))
}

/// The origin of mod `id` installed in `home`, if it came from GitHub.
pub fn origin(home: &Path, id: &str) -> Option<Origin> {
    serde_json::from_str(&fs::read_to_string(origin_path(home, id)).ok()?).ok()
}

/// Every mod of `home` installed from GitHub.
pub fn origins(home: &Path) -> Vec<Origin> {
    let mut found: Vec<Origin> = fs::read_dir(home.join(INSTALLED))
        .map(|dir| {
            dir.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".origin.json"))
                .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
                .collect()
        })
        .unwrap_or_default();
    found.sort_by(|a: &Origin, b| a.id.cmp(&b.id));
    found
}

/// Installs `packed` (from `found`) into the game folder `home`, turns it on and records where
/// it came from. A mod from the registry or the player's own is never replaced; one from GitHub
/// is (that is how it updates).
pub fn install(home: &Path, found: &Resolved, packed: &Packed) -> Result<Origin, String> {
    let target = home.join(INSTALLED).join(format!("{}.hnmod", packed.id));
    if target.exists() && origin(home, &packed.id).is_none() {
        return Err(format!(
            "{} is already installed and not from GitHub: remove it first.",
            packed.id
        ));
    }
    fs::create_dir_all(home.join(INSTALLED)).map_err(|e| e.to_string())?;
    fs::copy(&packed.file, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    let record = Origin {
        id: packed.id.clone(),
        version: packed.version.clone(),
        owner: found.owner.clone(),
        repo: found.repo.clone(),
        tag: found.asset.as_ref().and(found.tag.clone()),
        commit: found.commit.clone(),
        asset: found.asset.as_ref().map(|a| a.name.clone()),
        sha256: packed.sha256.clone(),
    };
    let text = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
    let path = origin_path(home, &packed.id);
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    java::set_enabled(home, &packed.id, true)?;
    let _ = fs::remove_file(&packed.file);
    Ok(record)
}

/// Removes mod `id` installed from GitHub; false when it wasn't.
pub fn remove(home: &Path, id: &str) -> Result<bool, String> {
    if origin(home, id).is_none() {
        return Ok(false);
    }
    for path in [
        origin_path(home, id),
        home.join(INSTALLED).join(format!("{id}.hnmod")),
    ] {
        if let Err(e) = fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(format!("{}: {e}", path.display()));
            }
        }
    }
    java::set_enabled(home, id, false)?;
    Ok(true)
}

#[cfg(test)]
mod tests;
