//! The mod registry's index, `index.json` (docs/api/mod-index.md): every
//! reviewed mod and modpack, its versions, the file of each with its size,
//! SHA-256 and minisign signature, and the files taken down. The launcher
//! reads it to browse and install, the website to show it, and the
//! registry's CI writes it.
//!
//! [`check_file`] is what a file must pass before it is installed: its size
//! and hash are the ones the index lists, its signature verifies against the
//! project's pinned key, and what its own manifest says matches the entry.
use crate::{
    Capability, Package, Side,
    modpack::{self, Modpack},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

/// The index format this build reads and writes. A reader refuses a newer
/// one rather than misread it.
pub const SCHEMA: u32 = 1;
/// The biggest file the registry takes (a `.hmod` with content, a `.hpack`).
pub const MAX_FILE: u64 = 64 << 20;
/// Where files may be downloaded from: the registry's release assets.
pub const FILE_HOST: &str = "https://github.com/horizon-rust/horizon-mods/releases/download/";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    /// When CI built it, RFC 3339.
    #[serde(default)]
    pub generated: String,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub revoked: Vec<Revoked>,
}

/// What a project is. New kinds (data packs, worlds, themes) can be added
/// without a new schema: a reader keeps a kind it doesn't know as
/// [`Kind::Other`] and doesn't offer it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Kind {
    /// A `.hmod` of content only.
    Mod,
    /// A source-only native mixin bundle, compiled into an isolated game build. Published as an
    /// `.hnmod` until launchers and the game read native `.hmod`s (#5865, docs/api/mod-files.md).
    NativeMod,
    /// A `.hpack`: registry mods played together as a profile.
    Modpack,
    /// A resource pack (`.zip` with `pack.mcmeta`) made or tested for Horizon.
    ResourcePack,
    /// A Horizon shader pack (`.hshader`, the native WGSL format of
    /// docs/shaders.md; not the shaders/-folder packs, which stay on Modrinth).
    Shader,
    Other(String),
}

impl Kind {
    pub fn name(&self) -> &str {
        match self {
            Kind::Mod => "mod",
            Kind::NativeMod => "native-mod",
            Kind::Modpack => "modpack",
            Kind::ResourcePack => "resourcepack",
            Kind::Shader => "shader",
            Kind::Other(name) => name,
        }
    }

    /// The extension of its files; None for a kind this build doesn't know.
    pub fn extension(&self) -> Option<&'static str> {
        match self {
            Kind::Mod => Some("hmod"),
            Kind::NativeMod => Some("hnmod"),
            Kind::Modpack => Some("hpack"),
            Kind::ResourcePack => Some("zip"),
            Kind::Shader => Some(horizon_shader_api::EXTENSION),
            Kind::Other(_) => None,
        }
    }
}

impl From<String> for Kind {
    fn from(name: String) -> Kind {
        match name.as_str() {
            "mod" => Kind::Mod,
            "native-mod" => Kind::NativeMod,
            "modpack" => Kind::Modpack,
            "resourcepack" => Kind::ResourcePack,
            "shader" => Kind::Shader,
            _ => Kind::Other(name),
        }
    }
}

impl From<Kind> for String {
    fn from(kind: Kind) -> String {
        kind.name().to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// The manifest's id, `[a-z0-9_]{1,64}`: unique across the index.
    pub id: String,
    pub kind: Kind,
    pub name: String,
    /// One line for lists.
    pub summary: String,
    /// Markdown, for the project's page.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The GitHub logins allowed to publish its updates.
    #[serde(default)]
    pub owners: Vec<String>,
    /// An SPDX expression.
    pub license: String,
    /// Where its source code is, `https://`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The author's minisign public key: every version must be signed with
    /// it. None: unsigned (the SHA-256 still pins the reviewed file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Newest first.
    pub versions: Vec<Version>,
    /// Its files' downloads from their releases, all versions together:
    /// GitHub's counts, added by the registry's publish job (hourly).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub downloads: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
    /// The day it was published, `YYYY-MM-DD`.
    #[serde(default)]
    pub published: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub changelog: String,
    /// A mod's API line (`0.1`), from its manifest; a modpack's format (`1`).
    pub api: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Capability>,
    /// The mod API level a mod's code needs ([`crate::api_level`]), read
    /// from its file; none for content and other kinds, and in indexes
    /// built before levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
    /// The Minecraft version it is for.
    pub game: String,
    /// A modpack's mods, as its manifest lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
    pub file: File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
    /// The `.minisig` file's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// A file taken down: by hash, or every version of `id` when `sha256` is
/// None. The launcher refuses to install it and turns installed copies off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revoked {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub reason: String,
    #[serde(default)]
    pub date: String,
}

/// How far a checked file is trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// Signed with the project's pinned key.
    Verified,
    /// The project has no key: only the hash pins it.
    Unsigned,
}

impl Index {
    /// Parses and checks an index (see [`Index::check`]).
    pub fn parse(text: &str) -> Result<Index, String> {
        Self::parse_with(text, false)
    }

    /// [`Index::parse`] for a local test registry, whose files may be
    /// `file://` paths (trying a registry out before it is published).
    pub fn parse_local(text: &str) -> Result<Index, String> {
        Self::parse_with(text, true)
    }

    fn parse_with(text: &str, local_files: bool) -> Result<Index, String> {
        #[derive(Deserialize)]
        struct Head {
            schema: u32,
        }
        let head: Head = serde_json::from_str(text).map_err(|e| format!("not a mod index: {e}"))?;
        if head.schema > SCHEMA {
            return Err(format!(
                "the mod index is schema {}; this build reads {SCHEMA}: update Horizon",
                head.schema
            ));
        }
        let index: Index = serde_json::from_str(text).map_err(|e| format!("mod index: {e}"))?;
        index.check_with(local_files)?;
        Ok(index)
    }

    /// Every rule a reader relies on: unique valid ids, at least one
    /// version each, plain file names with the kind's extension, `https`
    /// URLs, hex hashes, keys and signatures that decode, and each
    /// modpack's mods present in the index with the same hash.
    pub fn check(&self) -> Result<(), String> {
        self.check_with(false)
    }

    fn check_with(&self, local_files: bool) -> Result<(), String> {
        for (i, p) in self.projects.iter().enumerate() {
            let at = |e: String| format!("{}: {e}", p.id);
            if !valid_id(&p.id) {
                return Err(format!("project id {:?} is not [a-z0-9_]{{1,64}}", p.id));
            }
            if self.projects[..i].iter().any(|q| q.id == p.id) {
                return Err(at("listed twice".into()));
            }
            if p.name.trim().is_empty() || p.license.trim().is_empty() {
                return Err(at("needs a name and a licence".into()));
            }
            https(&p.source).map_err(at)?;
            if let Some(icon) = &p.icon {
                https(icon).map_err(at)?;
            }
            if let Some(key) = &p.key {
                minisign_verify::PublicKey::from_base64(key.trim())
                    .map_err(|e| at(format!("key: {e}")))?;
            }
            if p.versions.is_empty() {
                return Err(at("has no version".into()));
            }
            for (j, v) in p.versions.iter().enumerate() {
                let at = |e: String| format!("{} {}: {e}", p.id, v.version);
                if v.version.trim().is_empty()
                    || p.versions[..j].iter().any(|w| w.version == v.version)
                {
                    return Err(at("empty or repeated version".into()));
                }
                let f = &v.file;
                let extension_ok = p
                    .kind
                    .extension()
                    .is_none_or(|e| f.name.ends_with(&format!(".{e}")));
                if !plain_name(&f.name) || !extension_ok {
                    return Err(at(format!(
                        "{:?} is not a plain file name for a {}",
                        f.name,
                        p.kind.name()
                    )));
                }
                if !(local_files && f.url.starts_with("file:///")) {
                    https(&f.url).map_err(at)?;
                }
                hex64(&f.sha256).map_err(at)?;
                if f.size == 0 || f.size > MAX_FILE {
                    return Err(at(format!(
                        "size {} is not 1 byte to {} MiB",
                        f.size,
                        MAX_FILE >> 20
                    )));
                }
                match (&p.key, &f.signature) {
                    (Some(_), None) => {
                        return Err(at("unsigned, but the project has a key".into()));
                    }
                    (None, Some(_)) => return Err(at("signed, but the project has no key".into())),
                    (Some(_), Some(sig)) => {
                        minisign_verify::Signature::decode(sig)
                            .map_err(|e| at(format!("signature: {e}")))?;
                    }
                    (None, None) => {}
                }
                for r in &v.requires {
                    let found = self
                        .project(&r.id)
                        .and_then(|q| q.versions.iter().find(|w| w.version == r.version));
                    match found {
                        Some(w) if w.file.sha256 == r.sha256 => {}
                        Some(_) => {
                            return Err(at(format!(
                                "{} {}: another file than the index's",
                                r.id, r.version
                            )));
                        }
                        None => {
                            return Err(at(format!(
                                "needs {} {}, which is not in the index",
                                r.id, r.version
                            )));
                        }
                    }
                }
            }
        }
        for r in &self.revoked {
            if let Some(h) = &r.sha256 {
                hex64(h).map_err(|e| format!("revoked {}: {e}", r.id))?;
            }
        }
        Ok(())
    }

    /// A project's newest version for Minecraft `game` that wasn't taken
    /// down and that a Horizon serving mod API `level` can run (an unknown
    /// level counts as runnable: the game still checks when it starts it).
    pub fn installable<'a>(
        &self,
        project: &'a Project,
        game: &str,
        level: u32,
    ) -> Option<&'a Version> {
        project.versions.iter().find(|v| {
            v.game == game
                && v.api_level.is_none_or(|l| l <= level)
                && self.revoked(&project.id, &v.file.sha256).is_none()
        })
    }
    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    /// Why the file of `id` with this hash was taken down, if it was.
    pub fn revoked(&self, id: &str, sha256: &str) -> Option<&Revoked> {
        self.revoked.iter().find(|r| {
            r.id == id
                && r.sha256
                    .as_ref()
                    .is_none_or(|h| h.eq_ignore_ascii_case(sha256))
        })
    }
}

/// `[a-z0-9_]{1,64}`, the manifests' id rule.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn https(url: &str) -> Result<(), String> {
    match url.strip_prefix("https://") {
        Some(rest) if !rest.is_empty() && !url.chars().any(char::is_whitespace) => Ok(()),
        _ => Err(format!("{url:?} is not an https:// address")),
    }
}

fn hex64(h: &str) -> Result<(), String> {
    if h.len() == 64
        && h.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(format!("{h:?} is not a lowercase hex SHA-256"))
    }
}

/// One plain file name: no folder, no hidden file, nothing a file system
/// reads as something else.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

/// The SHA-256 of a file, lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks a minisign signature (`.minisig` text) of `path` against `key`.
pub fn verify_signature(key: &str, signature: &str, path: &Path) -> Result<(), String> {
    let key =
        minisign_verify::PublicKey::from_base64(key.trim()).map_err(|e| format!("key: {e}"))?;
    let sig =
        minisign_verify::Signature::decode(signature).map_err(|e| format!("signature: {e}"))?;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut verifier = key
        .verify_stream(&sig)
        .map_err(|e| format!("signature: {e}"))?;
    let mut reader = std::io::BufReader::new(file);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        verifier.update(&buf[..n]);
    }
    verifier
        .finalize()
        .map_err(|_| "the signature doesn't match the project's key".to_string())
}

/// Checks the file at `path` against its entry before it is installed:
/// size, SHA-256, revocation, signature, and that its manifest says what the
/// index says (id, version, API, side, capabilities; a modpack's mods).
pub fn check_file(
    index: &Index,
    project: &Project,
    version: &Version,
    path: &Path,
) -> Result<Trust, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size != version.file.size {
        return Err(format!(
            "{} bytes, the index lists {}",
            size, version.file.size
        ));
    }
    let hash = sha256_file(path)?;
    if hash != version.file.sha256 {
        return Err("its SHA-256 isn't the one the index lists: nothing was installed".into());
    }
    if let Some(r) = index.revoked(&project.id, &hash) {
        return Err(format!("{} was taken down: {}", project.name, r.reason));
    }
    let trust = match (&project.key, &version.file.signature) {
        (Some(key), Some(sig)) => {
            verify_signature(key, sig, path)?;
            Trust::Verified
        }
        (None, None) => Trust::Unsigned,
        _ => return Err("its signature and the project's key don't go together".into()),
    };
    if matches!(project.kind, Kind::Other(_)) {
        return Err(format!(
            "this Horizon can't install a {} yet",
            project.kind.name()
        ));
    }
    inspect(&project.kind, path)?.matches(project, version)?;
    Ok(trust)
}

/// What a file says about itself, read by its kind's own check
/// ([`inspect`]). The registry's CI writes these into the index; the
/// launcher compares them with the index before it installs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// A mod's or a modpack's manifest id; None for a resource pack.
    pub id: Option<String>,
    pub version: Option<String>,
    /// A mod's API line, a modpack's format, a resource pack's pack format.
    pub api: String,
    pub side: Option<Side>,
    /// In the manifest's order.
    pub capabilities: Vec<Capability>,
    /// The mod API level a mod's code needs, from the functions it imports.
    pub api_level: Option<u32>,
    /// The Minecraft version, when the file names one (a modpack does).
    pub game: Option<String>,
    pub requires: Vec<Requirement>,
}

impl Facts {
    /// Whether the file is the one `version` of `project` describes.
    pub fn matches(&self, project: &Project, version: &Version) -> Result<(), String> {
        let (mut caps, mut listed) = (self.capabilities.clone(), version.capabilities.clone());
        caps.sort();
        listed.sort();
        let differs = self.id.as_ref().is_some_and(|id| *id != project.id)
            || self.version.as_ref().is_some_and(|v| *v != version.version)
            || self.api != version.api
            || self.side != version.side
            || caps != listed
            || version.api_level.is_some() && self.api_level != version.api_level
            || self.game.as_ref().is_some_and(|g| *g != version.game)
            || self.requires != version.requires;
        if !differs {
            return Ok(());
        }
        Err(match project.kind {
            Kind::Mod => format!(
                "its horizon-mod.toml ({} {}, API {}, {:?}, {}) isn't what the index lists",
                self.id.as_deref().unwrap_or("?"),
                self.version.as_deref().unwrap_or("?"),
                self.api,
                self.side,
                names(&self.capabilities)
            ),
            Kind::Modpack => format!("its {} isn't what the index lists", modpack::MANIFEST),
            Kind::Shader => format!(
                "its {} ({} {}, shader API {}) isn't what the index lists",
                horizon_shader_api::MANIFEST,
                self.id.as_deref().unwrap_or("?"),
                self.version.as_deref().unwrap_or("?"),
                self.api
            ),
            _ => format!("its pack format {} isn't what the index lists", self.api),
        })
    }
}

/// Reads and checks a file as its kind: a mod's manifest and content, a
/// modpack's manifest and overrides, a resource pack's `pack.mcmeta` and
/// paths, a shader pack's `horizon_shader_api::check` (the game's own). Kinds
/// this build doesn't know are refused: their checks come with their formats.
pub fn inspect(kind: &Kind, path: &Path) -> Result<Facts, String> {
    match kind {
        Kind::NativeMod => {
            let facts = native::Bundle::read(path)?.manifest.facts();
            // A `.hmod` with content too: checked as the game reads it.
            if Package::has_manifest(path) {
                let m = Package::read(path)?.manifest;
                crate::Content::read(path, &m)?;
            }
            Ok(facts)
        }
        Kind::Mod => {
            if native::holds(path) {
                return Err(format!(
                    "it has native code ({}): it is a native-mod",
                    native::MANIFEST
                ));
            }
            let Package { manifest: m, wasm } = Package::read(path)?;
            crate::Content::read(path, &m)?;
            let api_level = match wasm {
                Some(wasm) => {
                    let functions = crate::api_level::imports(&wasm)?;
                    Some(crate::api_level::needed(functions.iter().map(String::as_str)).0)
                }
                None => None,
            };
            Ok(Facts {
                api_level,
                capabilities: m.capabilities,
                id: Some(m.id),
                version: Some(m.version),
                api: m.api,
                side: Some(m.side),
                ..Facts::default()
            })
        }
        Kind::Modpack => {
            let m = Modpack::read(path)?.manifest;
            Ok(Facts {
                requires: m.mods.iter().map(modpack::PackMod::requirement).collect(),
                id: Some(m.id),
                version: Some(m.version),
                api: m.format.to_string(),
                game: Some(m.game),
                ..Facts::default()
            })
        }
        Kind::ResourcePack => Ok(Facts {
            api: check_resource_pack(path)?,
            ..Facts::default()
        }),
        Kind::Shader => {
            let pack = horizon_shader_api::Pack::read(path)?;
            if let Err(problems) = horizon_shader_api::check(&pack) {
                let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
                return Err(format!(
                    "the shader pack fails its check:\n{}",
                    lines.join("\n")
                ));
            }
            let m = pack.manifest;
            Ok(Facts {
                id: Some(m.id),
                version: Some(m.version),
                api: m.api,
                ..Facts::default()
            })
        }
        Kind::Other(name) => Err(format!("this Horizon can't read a {name} yet")),
    }
}

/// A resource pack: a zip with `pack.mcmeta` at its root holding a `pack`
/// object, and only plain paths. Returns its pack format (`pack_format`, or
/// `min_format` in newer packs), empty when it names none.
fn check_resource_pack(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let name = zip
            .by_index_raw(i)
            .map_err(|e| e.to_string())?
            .name()
            .map_err(|e| e.to_string())?
            .into_owned();
        if name.starts_with('/') || name.split('/').any(|part| part == "..") || name.contains('\\')
        {
            return Err(format!("{name:?} is not a plain path"));
        }
    }
    let mut meta = String::new();
    zip.by_name("pack.mcmeta")
        .map_err(|_| "pack.mcmeta: missing".to_string())?
        .take(1 << 20)
        .read_to_string(&mut meta)
        .map_err(|e| format!("pack.mcmeta: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&meta).map_err(|e| format!("pack.mcmeta: {e}"))?;
    let pack = &json["pack"];
    if !pack.is_object() {
        return Err("pack.mcmeta: no \"pack\" object".into());
    }
    let format = [&pack["pack_format"], &pack["min_format"]]
        .into_iter()
        .find(|f| !f.is_null())
        .map(|f| match f {
            serde_json::Value::Array(parts) => parts
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("."),
            other => other.to_string(),
        })
        .unwrap_or_default();
    Ok(format)
}

/// The kinds the registry takes submissions for; other kinds wait for their
/// formats.
pub fn open(kind: &Kind) -> Result<(), String> {
    match kind {
        Kind::Mod | Kind::NativeMod | Kind::Modpack | Kind::ResourcePack | Kind::Shader => Ok(()),
        Kind::Other(name) => Err(format!("the registry doesn't take {name} yet")),
    }
}

fn names(caps: &[Capability]) -> String {
    let names: Vec<String> = caps.iter().map(ToString::to_string).collect();
    format!("[{}]", names.join(", "))
}

pub mod native;
pub mod registry;
#[cfg(test)]
mod tests;
