//! Everything that is not UI: GitHub, installs, resource preparation, the game's processes and
//! updates. The GUI, the CLI and the MCP server all go through this module. Functions here
//! block; the GUI runs them on threads and reads their messages.
use crate::{account, mojang, sources};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub const REPO: &str = "horizon-rust/horizon";
pub const README_URL: &str = "https://github.com/horizon-rust/horizon/blob/main/launcher/README.md";
/// The players' page on connecting an AI agent (the website's docs/agents.md).
pub const AGENT_DOCS_URL: &str = "https://playhorizon.dev/docs/agents";
/// This launcher's version: the release workflow sets `HORIZON_LAUNCHER_VERSION` from the
/// tag it builds (no version-bump commits); other builds say Cargo.toml's.
pub const VERSION: &str = version_or(option_env!("HORIZON_LAUNCHER_VERSION"));
/// Launcher releases are tagged `launcher@X.Y.Z` from 0.2 on (docs/versioning.md), and Horizon's
/// own `vX.Y.Z`. Before 0.2 they were `launcher-vX.Y.Z` and `vYYYY.MM.DD[.N]`: every 0.2 tag is
/// newer than every one of those ([`release_order`]).
pub const LAUNCHER_TAG_PREFIX: &str = "launcher@";
/// The launcher tags from before 0.2 (`launcher-v0.16.0`).
pub const OLD_LAUNCHER_TAG_PREFIX: &str = "launcher-v";

const fn version_or(injected: Option<&'static str>) -> &'static str {
    match injected {
        Some(v) if !v.is_empty() => v,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub body: String,
    pub date: String,
    pub asset: Option<(String, u64)>,
    /// Listed and installed by hand, never as an update: a GitHub pre-release of the old Alpha
    /// channel (v0.2.1 on), or a Nightly older than the current one.
    pub prerelease: bool,
    /// A Nightly's GitHub tag (`nightly`, or `nightly-<date>` for the last week's): its files and
    /// signature are there, while `tag` is the version it installs as (`nightly-2026-10-28-0c02067`).
    /// None for a release.
    pub nightly: Option<String>,
}

/// The release assets of each system Horizon ships for: (`std::env::consts::OS`, `ARCH`, the game,
/// the launcher). game-build.yml builds the game's, nightly-build.yml signs and publishes them, and
/// launcher.yml builds the launcher's (`platform_assets_are_published`).
pub const PLATFORMS: [(&str, &str, &str, &str); 4] = [
    (
        "linux",
        "x86_64",
        "horizon-linux-x86_64.tar.gz",
        "horizon-launcher-linux-x86_64.tar.gz",
    ),
    (
        "macos",
        "aarch64",
        "horizon-macos-arm64.zip",
        "horizon-launcher-macos-arm64.zip",
    ),
    // Intel Macs (#1161).
    (
        "macos",
        "x86_64",
        "horizon-macos-x64.zip",
        "horizon-launcher-macos-x64.zip",
    ),
    (
        "windows",
        "x86_64",
        "horizon-windows-x64.zip",
        "horizon-launcher-windows-x64.zip",
    ),
];

/// `a == b`, in a const context.
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// This system's entry of `PLATFORMS`: the game's asset and the launcher's.
const fn platform_assets(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    let mut i = 0;
    while i < PLATFORMS.len() {
        let (o, a, game, launcher) = PLATFORMS[i];
        if same(o, os) && same(a, arch) {
            return Some((game, launcher));
        }
        i += 1;
    }
    None
}

const OWN_ASSETS: Option<(&str, &str)> =
    platform_assets(std::env::consts::OS, std::env::consts::ARCH);
/// The game build this launcher installs, None where Horizon has none.
pub const ASSET: Option<&str> = match OWN_ASSETS {
    Some((game, _)) => Some(game),
    None => None,
};
/// The launcher build this launcher updates itself from.
pub const LAUNCHER_ASSET: Option<&str> = match OWN_ASSETS {
    Some((_, launcher)) => Some(launcher),
    None => None,
};

/// The launcher as an AppImage (`packaging/linux/appimage.sh`): the program and its libraries in
/// one file. A launcher running from one updates by replacing that file ([`appimage_path`]), so it
/// needs this archive rather than the platform's `.tar.gz`.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const LAUNCHER_APPIMAGE_ASSET: Option<&str> = Some("horizon-launcher-linux-x86_64.AppImage");
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub const LAUNCHER_APPIMAGE_ASSET: Option<&str> = None;

const LAUNCHER_BINARY: &str = if cfg!(windows) {
    "horizon-launcher.exe"
} else {
    "horizon-launcher"
};

pub(crate) const CLIENT: &str = if cfg!(windows) {
    "horizon-client.exe"
} else {
    "horizon-client"
};

/// The agent `url` is fetched with: the launcher's own user agent, through the player's proxy when
/// one is set (`proxy`).
pub(crate) fn agent(url: &str) -> ureq::Agent {
    crate::proxy::agent(
        ureq::AgentBuilder::new()
            .user_agent(&format!("horizon-launcher/{VERSION}"))
            // A connection that goes quiet fails instead of waiting for ever.
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(60)),
        url,
    )
    .build()
}

/// A file's body, from the network as the launcher reaches it.
pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    get(url)?
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    Ok(body)
}

pub(crate) fn get(url: &str) -> Result<ureq::Response, String> {
    let mut request = agent(url).get(url);
    // CI runners share addresses and hit GitHub's anonymous rate limit; they pass their own token.
    if let Some(token) = std::env::var("HORIZON_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    {
        if url.starts_with("https://api.github.com/") {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
    }
    request.call().map_err(|e| match e {
        ureq::Error::Status(code, r)
            if url.starts_with("https://api.github.com/")
                && crate::github::rate_limited(code, |h| r.header(h).map(str::to_owned)) =>
        {
            "GitHub's rate limit was reached. Try again in a few minutes.".into()
        }
        ureq::Error::Status(code, _) => format!("{url} answered {code}"),
        ureq::Error::Transport(t) => format!("No connection: {t}"),
    })
}

/// A Horizon game tag: `v` then a digit (`v2026.10.13`). Launcher tags never match.
pub fn is_game_tag(tag: &str) -> bool {
    tag.strip_prefix('v')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// The launcher version in a `launcher@X.Y.Z` (or `launcher-vX.Y.Z`) tag, or None for any other tag.
pub fn launcher_version(tag: &str) -> Option<&str> {
    tag.strip_prefix(LAUNCHER_TAG_PREFIX)
        .or_else(|| tag.strip_prefix(OLD_LAUNCHER_TAG_PREFIX))
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

/// The tag this launcher was released as. launcher.yml gives it (`HORIZON_LAUNCHER_TAG`); a release
/// built before it did gave only the version, of a `launcher-v` tag; a build from source is
/// Cargo.toml's version on the 0.2 line.
pub fn own_tag() -> String {
    own_tag_of(
        option_env!("HORIZON_LAUNCHER_TAG"),
        option_env!("HORIZON_LAUNCHER_VERSION"),
    )
}

fn own_tag_of(tag: Option<&str>, injected: Option<&str>) -> String {
    match (
        tag.filter(|t| !t.is_empty()),
        injected.filter(|v| !v.is_empty()),
    ) {
        (Some(tag), _) => tag.to_string(),
        (None, Some(version)) => format!("{OLD_LAUNCHER_TAG_PREFIX}{version}"),
        (None, None) => format!("{LAUNCHER_TAG_PREFIX}{}", env!("CARGO_PKG_VERSION")),
    }
}

/// Where a tag or version stands among Horizon's releases (docs/versioning.md): the tags from
/// before 0.2 (`vYYYY.MM.DD[.N]`, `launcher-vX.Y.Z`) and bare versions first, compared number by
/// number with missing parts as zero and anything after a `-` ignored, as launchers always did;
/// then the 0.2 tags (`vX.Y.Z`, `launcher@X.Y.Z`) in semver's order, a pre-release
/// (`v0.3.0-rc.1`) before its release.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReleaseOrder {
    /// Numbers without trailing zeros, so 1.2 and 1.2.0 are equal.
    Before02(Vec<u64>),
    Semver([u64; 3], Prerelease),
    /// A Nightly (`nightly-<date>-<commit>`), by its date: newer than every release, since the
    /// Nightly is the only build Horizon ships.
    Nightly(String),
}

/// (is a release, the pre-release's identifiers): a release follows every pre-release of its
/// version; identifiers compare one by one, numbers before words, a shorter list first.
pub type Prerelease = (bool, Vec<Identifier>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Identifier {
    Number(u64),
    Word(String),
}

/// Semver's majors stay below this; a date tag's first number is a year.
const FIRST_YEAR: u64 = 1000;

pub fn release_order(tag: &str) -> ReleaseOrder {
    if let Some(date) = crate::nightly::date_of(tag) {
        return ReleaseOrder::Nightly(date.to_owned());
    }
    let semver = tag
        .strip_prefix(LAUNCHER_TAG_PREFIX)
        .or_else(|| tag.strip_prefix('v'))
        .and_then(semver_order);
    semver.unwrap_or_else(|| {
        let v = tag
            .strip_prefix(OLD_LAUNCHER_TAG_PREFIX)
            .or_else(|| tag.strip_prefix(LAUNCHER_TAG_PREFIX))
            .unwrap_or(tag);
        let v = v.strip_prefix('v').unwrap_or(v);
        let mut parts: Vec<u64> = v
            .split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        while parts.last() == Some(&0) {
            parts.pop();
        }
        ReleaseOrder::Before02(parts)
    })
}

/// `MAJOR.MINOR.PATCH[-PRE]` with a major below [`FIRST_YEAR`], or None.
fn semver_order(version: &str) -> Option<ReleaseOrder> {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numbers: Vec<u64> = core
        .split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    let &[major, minor, patch] = numbers.as_slice() else {
        return None;
    };
    if major >= FIRST_YEAR {
        return None;
    }
    let pre = match pre {
        None => (true, Vec::new()),
        Some(pre) => (
            false,
            pre.split('.')
                .map(|id| {
                    (!id.is_empty()).then(|| {
                        id.parse()
                            .map_or_else(|_| Identifier::Word(id.to_string()), Identifier::Number)
                    })
                })
                .collect::<Option<_>>()?,
        ),
    };
    Some(ReleaseOrder::Semver([major, minor, patch], pre))
}

/// Compares two tags or versions by [`release_order`].
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    release_order(a).cmp(&release_order(b))
}

/// Every published release of the repository, both kinds, as GitHub's JSON (kept on disk with its
/// ETag, and read through Kira's mirror past GitHub's rate limit: [`crate::github`]).
fn release_list() -> Result<serde_json::Value, String> {
    crate::github::json(&format!(
        "https://api.github.com/repos/{REPO}/releases?per_page=100"
    ))
}

/// The releases in `list` whose tag passes `keep`, newest first, with the asset named `asset`.
pub(crate) fn parse_releases(
    list: &serde_json::Value,
    asset: Option<&str>,
    keep: fn(&str) -> bool,
) -> Vec<Release> {
    let mut out = Vec::new();
    for r in list.as_array().into_iter().flatten() {
        let tag = r["tag_name"].as_str().unwrap_or_default();
        if r["draft"].as_bool().unwrap_or(false) || !keep(tag) {
            continue;
        }
        let found = r["assets"].as_array().into_iter().flatten().find_map(|a| {
            (Some(a["name"].as_str()?) == asset).then(|| {
                Some((
                    a["browser_download_url"].as_str()?.to_string(),
                    a["size"].as_u64()?,
                ))
            })?
        });
        out.push(Release {
            tag: tag.into(),
            name: r["name"].as_str().unwrap_or_default().into(),
            body: r["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
            date: r["published_at"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(10)
                .collect(),
            asset: found,
            prerelease: r["prerelease"].as_bool().unwrap_or(false),
            nightly: None,
        });
    }
    out.sort_by(|a, b| compare_versions(&b.tag, &a.tag));
    out
}

fn is_launcher_tag(tag: &str) -> bool {
    launcher_version(tag).is_some()
}

/// Published Horizon builds, newest first: the current Nightly (the only build Horizon ships since
/// 2026-10-28, docs/release-flow.md), the last week's Nightlies, then the releases from before.
/// Drafts never reach the anonymous API, and launcher releases are left out.
pub fn releases() -> Result<Vec<Release>, String> {
    Ok(game_releases(&release_list()?))
}

/// The game's builds in `list`, newest first. A Nightly is listed under the version it installs as
/// ([`crate::nightly::version_of`]); only the current one (`nightly`) is an update.
pub(crate) fn game_releases(list: &serde_json::Value) -> Vec<Release> {
    let mut out = parse_releases(list, ASSET, |tag| {
        is_game_tag(tag) || horizon_release_sign::is_nightly(tag)
    });
    for r in &mut out {
        if horizon_release_sign::is_nightly(&r.tag) {
            let version = crate::nightly::version_of(r);
            let tag = std::mem::replace(&mut r.tag, version);
            r.prerelease = tag != horizon_release_sign::NIGHTLY;
            r.nightly = Some(tag);
        }
    }
    // Two Nightlies of one day: the current one first (the dated one is the day's earlier build).
    out.sort_by(|a, b| compare_versions(&b.tag, &a.tag).then(a.prerelease.cmp(&b.prerelease)));
    out
}

/// The launcher releases in `list` (the newest 100 releases). Game releases come out several times
/// a day, so the newest launcher release can be older than all of those: then the newest
/// `launcher@` and `launcher-v` tags are looked up one by one (a draft's tag is skipped: it has no
/// public release).
fn launcher_releases_in(list: &serde_json::Value) -> Vec<Release> {
    launcher_releases_in_with(list, |url| crate::github::json(&url))
}

fn launcher_releases_in_with(
    list: &serde_json::Value,
    json: impl Fn(String) -> Result<serde_json::Value, String>,
) -> Vec<Release> {
    let found = parse_releases(list, launcher_asset(), is_launcher_tag);
    if found.iter().any(|r| r.asset.is_some()) {
        return found;
    }
    let api = format!("https://api.github.com/repos/{REPO}");
    let mut tags = Vec::new();
    for prefix in [LAUNCHER_TAG_PREFIX, OLD_LAUNCHER_TAG_PREFIX] {
        // Every `launcher@` tag is newer than every `launcher-v` one: those only when there is none.
        if tags.is_empty() {
            if let Ok(refs) = json(format!("{api}/git/matching-refs/tags/{prefix}")) {
                tags = newest_launcher_tags(&refs);
            }
        }
    }
    for tag in tags.into_iter().take(3) {
        if let Ok(release) = json(format!("{api}/releases/tags/{tag}")) {
            let one = parse_releases(
                &serde_json::json!([release]),
                launcher_asset(),
                is_launcher_tag,
            );
            if one.iter().any(|r| r.asset.is_some()) {
                return one;
            }
        }
    }
    found
}

/// The launcher tags in GitHub's matching-refs answer, newest first.
fn newest_launcher_tags(refs: &serde_json::Value) -> Vec<String> {
    let mut tags: Vec<String> = refs
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["ref"].as_str()?.strip_prefix("refs/tags/"))
        .filter(|t| is_launcher_tag(t))
        .map(String::from)
        .collect();
    tags.sort_by(|a, b| compare_versions(b, a));
    tags
}

/// The newest Horizon release that has a build for this platform.
pub fn latest(releases: &[Release]) -> Option<&Release> {
    releases.iter().find(|r| r.asset.is_some())
}

/// The releases a launcher installs by itself: every one that is published as a release, so
/// GitHub's latest when it comes out. The pre-releases of the old Alpha channel are left out.
pub fn installable_updates(releases: &[Release]) -> Vec<Release> {
    releases.iter().filter(|r| !r.prerelease).cloned().collect()
}

/// How often a launcher left open looks for new releases: they come out several times a day.
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// The community, sponsor and website links and the official server's address, from
/// `launcher/links.json` on main, so they can change without a launcher release. An empty address
/// hides its button (an empty `official_server`: no official server offered).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Links {
    pub discord: String,
    pub sponsor: String,
    pub website: String,
    /// The official server's address, as servers.dat lists it (`server_list::add_official`).
    pub official_server: String,
    /// The addresses it had before: a list holding one moves to `official_server`. A move appends
    /// the old address here.
    pub official_server_previous: Vec<String>,
}

impl Default for Links {
    /// What the launcher uses until (or unless) it can read links.json.
    fn default() -> Self {
        Self {
            discord: "https://discord.gg/playhorizon".into(),
            sponsor: "https://github.com/sponsors/ludovic111".into(),
            website: "https://playhorizon.dev".into(),
            official_server: crate::server_list::OFFICIAL_ADDRESS.into(),
            official_server_previous: crate::server_list::OFFICIAL_PREVIOUS
                .iter()
                .map(|a| a.to_string())
                .collect(),
        }
    }
}

impl Links {
    /// Reads links.json. A key that is missing keeps its default, an empty one hides the link,
    /// and text that is not a JSON object gives the defaults.
    pub fn parse(text: &str) -> Self {
        let mut links = Self::default();
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(text) {
            for (key, slot) in [
                ("discord", &mut links.discord),
                ("sponsor", &mut links.sponsor),
                ("website", &mut links.website),
                ("official_server", &mut links.official_server),
            ] {
                if let Some(url) = map.get(key).and_then(|v| v.as_str()) {
                    *slot = url.trim().to_string();
                }
            }
            // A list of addresses; anything else keeps the built-in one.
            if let Some(serde_json::Value::Array(previous)) = map.get("official_server_previous") {
                if previous.iter().all(serde_json::Value::is_string) {
                    links.official_server_previous = previous
                        .iter()
                        .filter_map(|a| a.as_str())
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty())
                        .collect();
                }
            }
        }
        links
    }
}

/// The links from main, or the built-in ones when offline.
pub fn links() -> Links {
    let url = format!("https://raw.githubusercontent.com/{REPO}/main/launcher/links.json");
    get(&url)
        .ok()
        .and_then(|r| r.into_string().ok())
        .map_or_else(Links::default, |text| Links::parse(&text))
}

/// The launcher's data folder; `HORIZON_LAUNCHER_HOME` overrides it (for tests and scripts).
pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HORIZON_LAUNCHER_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("horizon-launcher")
}

/// A canonical path without Windows' verbatim prefix: `canonicalize` gives `\\?\C:\...` there,
/// and some tools stat `\\?\C:` — the volume itself — and fail. A drive path loses its `\\?\`;
/// any other path is left as it is.
pub(crate) fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// `path` spelled only in ASCII: what Windows' MinGW-w64 tools can open (they read paths
/// through ANSI APIs, so a Cyrillic user name in %APPDATA% breaks them, #5189).
fn ascii(path: &Path) -> bool {
    path.as_os_str().to_str().is_some_and(str::is_ascii)
}

/// Where builds happen — the Rust toolchain, its Cargo home and `native_mods::root` — as a
/// path every build tool can read: `data_dir()` resolved, so the tools are never handed a
/// link (horizon-mixins refuses one, and macOS's /tmp is one). On Windows, when the resolved
/// path isn't [`ascii`], [`windows_build_dir`] names the same folder a way MinGW-w64 can open.
pub fn build_dir() -> PathBuf {
    let data = data_dir();
    let _ = fs::create_dir_all(&data);
    let real = data.canonicalize().map(plain).unwrap_or(data);
    if ascii(&real) {
        return real;
    }
    #[cfg(windows)]
    return windows_build_dir(&real);
    #[cfg(not(windows))]
    real
}

/// [`build_dir`] when `real` isn't [`ascii`]: [`ascii_build_dir`], a real folder of
/// ProgramData's. Only when it can't be created, the data folder's short (8.3) name.
///
/// The short name came first until #6009, and wasn't enough: it is only another spelling of the
/// same folder, and rustc canonicalizes its own path to find its sysroot, which spells the long,
/// Cyrillic name again and hands it to the linker (`x86_64-w64-mingw32-gcc`: "No such file or
/// directory" on `C:/Users/\xca\xe8…`). A folder whose real name is ASCII stays ASCII however a
/// tool resolves it.
#[cfg(windows)]
fn windows_build_dir(real: &Path) -> PathBuf {
    let dir = ascii_build_dir(real, std::env::var_os("ProgramData").map(PathBuf::from));
    if fs::create_dir_all(&dir).is_ok() {
        return dir;
    }
    short_path(real).filter(|p| ascii(p)).unwrap_or(dir)
}

/// The build folder for a data folder `real` whose path isn't [`ascii`]: in `program_data`
/// (`%ProgramData%`, `C:\ProgramData` when unset, relative or not ASCII), which every user may
/// create folders in, named for `real` so each user's is their own.
#[cfg_attr(not(windows), allow(dead_code))]
fn ascii_build_dir(real: &Path, program_data: Option<PathBuf>) -> PathBuf {
    use sha2::{Digest, Sha256};
    let public = program_data
        .filter(|p| p.is_absolute() && ascii(p))
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    let key = Sha256::digest(real.as_os_str().as_encoded_bytes());
    let key: String = key[..4].iter().map(|b| format!("{b:02x}")).collect();
    public.join(format!("horizon-launcher-{key}"))
}

/// `GetShortPathNameW`: `path`'s short (8.3) name, or None on error. A folder with no short
/// name (the volume may keep none) answers with its long name again, which [`ascii`] refuses.
#[cfg(windows)]
fn short_path(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let need = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) } as usize;
    if need == 0 {
        return None;
    }
    let mut buf = vec![0u16; need];
    let got = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), need as u32) } as usize;
    (got != 0 && got < need).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buf[..got])))
}

fn versions_dir() -> PathBuf {
    data_dir().join("versions")
}

pub fn version_dir(tag: &str) -> PathBuf {
    versions_dir().join(tag)
}

const COMPLETE: &str = ".installed";

/// The mod API level of the game players run: the selected version's, else the newest installed
/// one's. None with no version installed: the launcher doesn't know yet which game will run mods.
pub fn game_mod_api_level() -> Option<u32> {
    let installed = installed();
    let tag = Settings::load()
        .selected
        .filter(|s| installed.contains(s))
        .or_else(|| installed.into_iter().next())?;
    Some(mod_api_level(&tag))
}

/// A game version's mod API level, read once from the mark its program carries
/// (`horizon_mods::api_level::MARK`). A program from before the mark serves level 0, the
/// contract of Horizon 0.2.67 and 0.2.68.
pub fn mod_api_level(tag: &str) -> u32 {
    static LEVELS: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());
    let mut levels = LEVELS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(_, level)) = levels.iter().find(|(t, _)| t == tag) {
        return level;
    }
    let level = program_mod_api_level(&version_dir(tag).join(CLIENT));
    levels.push((tag.to_owned(), level));
    level
}

/// The mod API level in the game program at `path`: its mark's, else 0.
fn program_mod_api_level(path: &Path) -> u32 {
    let program = fs::read(path).unwrap_or_default();
    horizon_mods::api_level::read_mark(&program).unwrap_or(0)
}

/// Installed versions, newest tag first.
pub fn installed() -> Vec<String> {
    installed_in(&versions_dir())
}

fn installed_in(versions: &Path) -> Vec<String> {
    let mut tags: Vec<String> = fs::read_dir(versions)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(COMPLETE).exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    tags.sort_by(|a, b| compare_versions(b, a));
    tags
}

/// A small release file (its checksums or their signature), whole; None when it has none.
pub(crate) fn fetch_small(url: &str) -> Result<Option<Vec<u8>>, String> {
    let response = match get(url) {
        Ok(response) => response,
        Err(e) if e.ends_with("answered 404") => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut body = Vec::new();
    response
        .into_reader()
        .take(1 << 20)
        .read_to_end(&mut body)
        .map_err(|e| format!("Download interrupted: {e}"))?;
    Ok(Some(body))
}

/// The SHA-256 the release's signed checksums give for `asset`, or why it must not be installed.
fn signed_digest(tag: &str, asset: &str) -> Result<String, String> {
    crate::signed::digest(
        tag,
        asset,
        &horizon_release_sign::trusted_keys(),
        fetch_small,
    )
}

/// Downloads `url` into `to`; the file is kept only if its SHA-256 is `sha256`, the signed (or
/// pinned) one. A `size` of 0 takes the server's Content-Length for the progress.
pub(crate) fn download(
    url: &str,
    size: u64,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(f32),
) -> Result<(), String> {
    let response = get(url)?;
    let size = match size {
        0 => response
            .header("Content-Length")
            .and_then(|n| n.parse().ok())
            .unwrap_or(0),
        n => n,
    };
    let mut reader = response.into_reader();
    let mut file = fs::File::create(to).map_err(|e| e.to_string())?;
    let mut hash = horizon_release_sign::Sha256::default();
    let (mut done, mut buf) = (0u64, vec![0u8; 1 << 16]);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress((done as f32 / size.max(1) as f32).min(1.0) * 0.9);
    }
    let name = url.rsplit('/').next().unwrap_or(url);
    horizon_release_sign::check(name, sha256, &hash.hex()).inspect_err(|_| {
        let _ = fs::remove_file(to);
    })
}

fn unpack(archive: &Path, url: &str, to: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    if url.ends_with(".tar.gz") {
        tar::Archive::new(flate2::read::GzDecoder::new(file))
            .unpack(to)
            .map_err(|e| e.to_string())
    } else {
        zip::ZipArchive::new(file)
            .and_then(|mut z| z.extract(to))
            .map_err(|e| e.to_string())
    }
}

/// Downloads one release, checks it against its signed checksums and unpacks it; `progress` gets
/// the fraction done.
pub fn install(release: &Release, progress: impl Fn(f32)) -> Result<(), String> {
    let tag = release.tag.as_str();
    let sha256 = match &release.nightly {
        // Signed for the Nightly channel, served under its own GitHub tag.
        Some(published) => crate::signed::nightly_digest(
            published,
            ASSET.unwrap_or_default(),
            &horizon_release_sign::trusted_keys(),
            fetch_small,
        )?,
        None if is_game_tag(tag) => signed_digest(tag, ASSET.unwrap_or_default())?,
        None => return Err(format!("{tag} is not a Horizon release.")),
    };
    install_as(release, tag, &sha256, progress)
}

/// Downloads `release`'s archive for this platform, checks it against `sha256` (its signed
/// checksum), and unpacks it as the installed version `tag`.
pub(crate) fn install_as(
    release: &Release,
    tag: &str,
    sha256: &str,
    progress: impl Fn(f32),
) -> Result<(), String> {
    let (url, size) = release
        .asset
        .clone()
        .ok_or_else(|| format!("{tag} has no build for this platform."))?;
    let root = versions_dir();
    let partial = root.join(format!(".{tag}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let archive = partial.join("download");
    download(&url, size, sha256, &archive, &progress)?;
    let unpacked = partial.join("unpacked");
    unpack(&archive, &url, &unpacked)?;
    progress(0.97);
    // The archives hold one top folder (horizon-linux-x86_64/...); the version folder is its contents.
    let entries: Vec<_> = fs::read_dir(&unpacked)
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let top = match entries.as_slice() {
        [only] if only.path().is_dir() => only.path(),
        _ => unpacked.clone(),
    };
    if !top.join(CLIENT).exists() {
        return Err(format!("The download has no {CLIENT}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(top.join(CLIENT), fs::Permissions::from_mode(0o755));
    }
    let target = version_dir(tag);
    let _ = fs::remove_dir_all(&target);
    fs::rename(&top, &target).map_err(|e| e.to_string())?;
    fs::write(target.join(COMPLETE), "").map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&partial);
    reuse_prepared(&root, tag);
    complete_assets(tag);
    progress(1.0);
    Ok(())
}

pub fn remove(tag: &str) -> Result<(), String> {
    let dir = version_dir(tag);
    // The marker goes first: a version counted as installed until its folder is gone comes back
    // into `installed` when an overlapping removal re-reads the list mid-delete.
    let _ = fs::remove_file(dir.join(COMPLETE));
    // Windows holds a file for a moment after the game, a scan or the launcher's own reader lets
    // go of it, and remove_dir_all stops at the first one. Retry for a second; a folder that is
    // already gone is removed.
    for attempt in 1.. {
        match fs::remove_dir_all(&dir) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) if attempt == 5 => return Err(e.to_string()),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
    Ok(())
}

/// The bytes an installed version takes on disk.
pub fn disk_use(tag: &str) -> u64 {
    fn size(dir: &Path) -> u64 {
        fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| match e.metadata() {
                Ok(m) if m.is_dir() => size(&e.path()),
                Ok(m) => m.len(),
                Err(_) => 0,
            })
            .sum()
    }
    size(&version_dir(tag))
}

/// The Minecraft version a Horizon build targets, from its research-lock.json.
pub fn game_version(tag: &str) -> String {
    game_version_at(&version_dir(tag))
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn game_version_at(dir: &Path) -> String {
    read_json(&dir.join("research-lock.json"))
        .and_then(|v| v["minecraft"]["version"].as_str().map(String::from))
        .unwrap_or_else(|| horizon_core::VANILLA_VERSION.into())
}

pub fn resources(tag: &str) -> PathBuf {
    resources_at(&version_dir(tag))
}

fn resources_at(dir: &Path) -> PathBuf {
    dir.join(".local/horizon-client").join(game_version_at(dir))
}

const RESOURCES_MANIFEST: &str = "horizon-resources.json";

pub fn is_prepared(tag: &str) -> bool {
    resources(tag).join(RESOURCES_MANIFEST).exists()
}

/// Whether a version's resources hold Minecraft's sounds, which `sounds` copies from the player's own
/// install beside the preparation. A version prepared while that copy failed (Mojang's republished 26.3
/// asset index, #1076), or one that reused such a version's resources, plays no sound at all (#1035).
pub fn has_sounds(tag: &str) -> bool {
    sounds_in(&resources(tag))
}

fn sounds_in(resources: &Path) -> bool {
    // What the game's sound registry reads (horizon-client sound_events.rs).
    resources.join("assets/minecraft/sounds.json").is_file()
}

/// What the Play page says of a version's resources.
pub fn resources_summary(prepared: bool, sounds: bool, version: &str) -> String {
    use crate::i18n::trf;
    match (prepared, sounds) {
        (false, _) => trf(
            "Not prepared. Horizon needs the files of your own Minecraft {version}.",
            &[("version", version)],
        ),
        (true, true) => trf("Prepared for Minecraft {version}", &[("version", version)]),
        (true, false) => trf(
            "Prepared for Minecraft {version}, without its sounds: the game will be silent. \
             Prepare again to copy them from your Minecraft install.",
            &[("version", version)],
        ),
    }
}

/// Whether the resources `from` prepared are the ones preparing `to` would make: both pin the same
/// Minecraft jars (research-lock.json) and their preparers make the same output. A preparer
/// declares what it makes with `OUTPUT_VERSION` (crates/horizon-prepare/src/lib.rs, whose sources
/// ship in the release folder), bumped by hand when the output changes and checked by CI
/// (tools/prepare_output.py), so a release that changes the preparer's code but not its output
/// keeps the resources. Preparers from before the number are known by their sources
/// (`LEGACY_OUTPUT_VERSIONS`); older ones still are compared file by file (`same_preparer`).
fn resources_compatible(from: &Path, to: &Path, manifest: &serde_json::Value) -> bool {
    let pins = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).map(|lock| {
            ["version", "client_sha1", "server_sha1"].map(|k| lock["minecraft"][k].clone())
        })
    };
    let (Some(a), Some(b)) = (pins(from), pins(to)) else {
        return false;
    };
    if a != b || a.iter().any(|v| !v.is_string()) {
        return false;
    }
    match (output_version_at(from), output_version_at(to)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => same_preparer(from, to, manifest),
        _ => false,
    }
}

/// The `OUTPUT_VERSION` of a version folder's preparer: the constant in its sources, or for a
/// preparer from before it, the number `LEGACY_OUTPUT_VERSIONS` gives its fingerprint. None for
/// an unknown older preparer.
fn output_version_at(dir: &Path) -> Option<u32> {
    output_version_in(dir, &LEGACY_OUTPUT_VERSIONS)
}

fn output_version_in(dir: &Path, legacy: &[(&str, u32)]) -> Option<u32> {
    let crate_dir = dir.join(PREPARER_CRATE);
    let lib = fs::read_to_string(crate_dir.join("src/lib.rs")).ok()?;
    let declared = lib.lines().find_map(|line| {
        line.strip_prefix("pub const OUTPUT_VERSION: u32 = ")?
            .strip_suffix(';')?
            .parse()
            .ok()
    });
    if declared.is_some() {
        return declared;
    }
    let fingerprint = legacy_fingerprint(dir)?;
    legacy
        .iter()
        .find(|(known, _)| *known == fingerprint)
        .map(|&(_, version)| version)
}

/// Preparers from before `OUTPUT_VERSION`, by `legacy_fingerprint`, whose output
/// tools/prepare_output.py measured: what they prepare is version 1's.
const LEGACY_OUTPUT_VERSIONS: [(&str, u32); 2] = [
    // v0.2.52 to v0.2.64
    (
        "54218ade1c058b1f15ea4094bf8c1a7893adb37b1ed5cc803ddae8e950c31704",
        1,
    ),
    // v0.2.49 to v0.2.51
    (
        "da9ffe7558129c38b44abf43289e81946fe9f2512e5e07272eeae0cf0b4f1254",
        1,
    ),
];

/// The vanilla exporters those preparers ran (the `*_exporters_sha256` of their manifests), in
/// mods/horizon/src/test/java/dev/horizonlab/parity.
const LEGACY_EXPORTERS: [&str; 18] = [
    "ParityData.java",
    "VanillaBlockBehaviourExporter.java",
    "VanillaBlockLandingOracle.java",
    "VanillaBlockStateBindingOracle.java",
    "VanillaCollisionOracle.java",
    "VanillaCommandParseOracle.java",
    "VanillaCreativeTabsExporter.java",
    "VanillaCuboidModelOracle.java",
    "VanillaDamageSourceOracle.java",
    "VanillaEntityAttributeOracle.java",
    "VanillaEntityModelExporter.java",
    "VanillaEntityTypesExporter.java",
    "VanillaFaceVisibilityOracle.java",
    "VanillaGameplayFactsExporter.java",
    "VanillaHeightmapOracle.java",
    "VanillaMobDataExporter.java",
    "VanillaMovementRegistryOracle.java",
    "VanillaPlayerHealthOracle.java",
];

/// A version folder's preparer as the launchers before `OUTPUT_VERSION` compared it: the digest
/// of its sources (`horizon_prepare::sources_sha256`) and of the exporters it ran, line endings
/// as checked out ignored.
fn legacy_fingerprint(dir: &Path) -> Option<String> {
    use sha2::Digest;
    let sources = horizon_prepare::sources_sha256(&dir.join(PREPARER_CRATE))?;
    let mut hash = sha2::Sha256::new();
    hash.update(sources.as_bytes());
    for name in LEGACY_EXPORTERS {
        let path = format!("mods/horizon/src/test/java/dev/horizonlab/parity/{name}");
        let data: Vec<u8> = fs::read(dir.join(&path))
            .ok()?
            .into_iter()
            .filter(|b| *b != b'\r')
            .collect();
        hash.update(format!("{path}\0{}\0", data.len()));
        hash.update(&data);
    }
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// How launchers before `OUTPUT_VERSION` compared two preparers, for versions older than the
/// ones `LEGACY_OUTPUT_VERSIONS` knows. The manifest records the hashes of the vanilla exporters
/// the resources were made with (mods/horizon/src/test/java/.../parity), and the preparer writes
/// every file and the manifest's `schema`, so both need the same preparer and the same exporter
/// sources. The preparer is crates/horizon-prepare and tools/prepare_native_client.py (the
/// Python preparer of older versions, a wrapper of the binary since); a file only one of the two
/// versions has is a different preparer. The helpers the Python preparer imported (lab.py and
/// the test_*_parity.py modules) change between releases in how they find and check the
/// player's files, which the output does not depend on; they are not compared.
fn same_preparer(from: &Path, to: &Path, manifest: &serde_json::Value) -> bool {
    let Some(fields) = manifest.as_object() else {
        return false;
    };
    let mut preparer = vec![PathBuf::from("tools/prepare_native_client.py")];
    for dir in [from, to] {
        preparer.extend(files_under(dir, Path::new(PREPARER_CRATE)));
    }
    let same_or_absent = |path: &Path| match (fs::read(from.join(path)), fs::read(to.join(path))) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => {
            a.kind() == std::io::ErrorKind::NotFound && b.kind() == std::io::ErrorKind::NotFound
        }
        _ => false,
    };
    if !preparer.iter().all(|p| same_or_absent(p)) {
        return false;
    }
    let mut sources = vec![
        "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java".to_string(),
    ];
    for (_, value) in fields
        .iter()
        .filter(|(key, _)| key.ends_with("_exporters_sha256"))
    {
        let Some(files) = value.as_object() else {
            return false;
        };
        sources.extend(files.keys().cloned());
    }
    let same = |source: &str| {
        let path = Path::new(source);
        path.is_relative()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && matches!((fs::read(from.join(path)), fs::read(to.join(path))), (Ok(a), Ok(b)) if a == b)
    };
    sources.iter().all(|s| same(s))
}

/// Where a version folder keeps the preparer's sources (the release package copies them).
const PREPARER_CRATE: &str = "crates/horizon-prepare";

/// The files under `dir/relative`, as paths relative to `dir`.
fn files_under(dir: &Path, relative: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir.join(relative)) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = relative.join(entry.file_name());
        match entry.file_type() {
            Ok(t) if t.is_dir() => files.extend(files_under(dir, &path)),
            Ok(_) => files.push(path),
            Err(_) => {}
        }
    }
    files
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    copy_dir_except(from, to, None)
}

/// Copy a folder, leaving out one directory when given.
pub(crate) fn copy_dir_except(from: &Path, to: &Path, skip: Option<&Path>) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            if Some(entry.path().as_path()) != skip {
                copy_dir_except(&entry.path(), &target, skip)?;
            }
        } else {
            horizon_prepare::clone_or_copy(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// After an install: when `tag` is not prepared but another installed version made the same
/// resources, copies them (not hard links: preparing again rewrites files in place, which would
/// change both). The player then needs neither their Minecraft folder nor a new preparation to
/// play a release that only changed Horizon. Returns the version they came from.
fn reuse_prepared(versions: &Path, tag: &str) -> Option<String> {
    let target = versions.join(tag);
    let to = resources_at(&target);
    if to.join(RESOURCES_MANIFEST).exists() {
        return None;
    }
    let version = game_version_at(&target);
    for other in installed_in(versions).into_iter().filter(|t| t != tag) {
        let source = versions.join(&other);
        if game_version_at(&source) != version {
            continue;
        }
        let from = resources_at(&source);
        let Some(manifest) = read_json(&from.join(RESOURCES_MANIFEST)) else {
            continue;
        };
        if !resources_compatible(&source, &target, &manifest) {
            continue;
        }
        // Copied beside it and renamed when complete: an interrupted copy is never "prepared".
        // The name is this call's own, as the window's start and an install can both copy.
        static COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COPIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial = to.with_file_name(format!(".{version}.partial-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&partial);
        let copied = copy_dir(&from, &partial).and_then(|_| {
            if to.join(RESOURCES_MANIFEST).exists() {
                return Ok(false); // Prepared meanwhile, by another copy or the player.
            }
            let _ = fs::remove_dir_all(&to);
            fs::rename(&partial, &to).map(|_| true)
        });
        let _ = fs::remove_dir_all(&partial);
        match copied {
            Ok(true) => return Some(other),
            Ok(false) => return None,
            Err(_) => {}
        }
    }
    None
}

/// `reuse_prepared` for every installed version that is not prepared, oldest first, so a
/// version prepared this way can serve the next. Launchers before 0.1.4 installed new versions
/// without it; this catches those up. Returns each (version, where its resources came from).
/// Prepared versions also get the run-time reports (`complete_reports`) and the built-in packs,
/// panorama and sounds (`complete_assets`) they lack.
pub fn reuse_prepared_everywhere() -> Vec<(String, String)> {
    let versions = versions_dir();
    let reused = reuse_prepared_in(&versions);
    for tag in installed_in(&versions) {
        // Here, on the start-up thread, and not in `complete_assets`, which Play also runs: a
        // download of thousands of files is not for the moment the game is waited for.
        refill_own_store(&tag);
        complete_assets(&tag);
    }
    reused
}

fn reuse_prepared_in(versions: &Path) -> Vec<(String, String)> {
    let mut tags = installed_in(versions);
    tags.reverse();
    let reused = tags
        .iter()
        .filter_map(|tag| reuse_prepared(versions, tag).map(|from| (tag.clone(), from)))
        .collect();
    for tag in &tags {
        complete_reports(versions, tag);
    }
    reused
}

/// The data generator reports the game reads at run time: the built-in registries' network ids
/// (without them no server can be joined or hosted) and the command tree.
const RUNTIME_REPORTS: [&str; 2] = ["registries.json", "commands.json"];

/// Puts the data generator reports the game reads at run time (`RUNTIME_REPORTS`) into a prepared version's
/// resources when they are missing. Preparers before they shipped there left them only in the
/// scratch output of the version that ran the data generator (`.local/datagen/out/reports`),
/// and resources copied from another version (`reuse_prepared`) came without them. They depend
/// only on the server jar, so the version's own scratch output or any installed version pinning
/// the same jar serves, and no new preparation is needed. Returns whether all are there.
fn complete_reports(versions: &Path, tag: &str) -> bool {
    let target = versions.join(tag);
    let to = resources_at(&target);
    if !to.join(RESOURCES_MANIFEST).exists() {
        return false;
    }
    let missing: Vec<_> = RUNTIME_REPORTS
        .iter()
        .filter(|name| !to.join(name).is_file())
        .collect();
    if missing.is_empty() {
        return true;
    }
    let server = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).and_then(|lock| {
            let pins = ["version", "server_sha1"].map(|k| lock["minecraft"][k].clone());
            pins.iter().all(|v| v.is_string()).then_some(pins)
        })
    };
    let Some(pins) = server(&target) else {
        return false;
    };
    let reports = |dir: &Path| dir.join(".local/datagen/out/reports");
    let mut sources = vec![reports(&target)];
    for other in installed_in(versions).into_iter().filter(|t| t != tag) {
        let dir = versions.join(&other);
        if server(&dir).as_ref() == Some(&pins) {
            sources.push(resources_at(&dir));
            sources.push(reports(&dir));
        }
    }
    let mut complete = true;
    for name in missing {
        let Some(from) = sources.iter().map(|s| s.join(name)).find(|p| p.is_file()) else {
            complete = false;
            continue;
        };
        // Copied beside it and renamed: an interrupted copy never looks complete.
        let partial = to.join(format!(".{name}.partial-{}", std::process::id()));
        let copied = fs::copy(&from, &partial).and_then(|_| fs::rename(&partial, to.join(name)));
        if copied.is_err() {
            let _ = fs::remove_file(&partial);
            complete = false;
        }
    }
    complete
}

/// What a prepared version may lack that the launcher adds without preparing again, each copied
/// from the player's own Minecraft asset store as `horizon-prepare <kind>` does: what the
/// player misses without it, its kind (`horizon_prepare::missing_extras`, the same check as
/// `horizon-prepare --check`), and the copy.
type Completion = (
    &'static str,
    &'static str,
    fn(&Path, Option<&Path>, horizon_prepare::Log) -> Result<PathBuf, String>,
);

const COMPLETIONS: [Completion; 5] = [
    // Preparers before #1538 left the built-in packs out.
    (
        "Programmer Art and High Contrast",
        "packs",
        horizon_prepare::packs,
    ),
    // A preparation that could not read the asset store keeps the client jar's 1x1 grey
    // placeholders, its English alone, and no sounds (#1867: Prism Launcher installs prepared
    // since 0.2.48).
    ("The title panorama", "panorama", horizon_prepare::panorama),
    ("The Unifont glyphs", "fonts", horizon_prepare::fonts),
    ("The languages", "languages", horizon_prepare::languages),
    ("The sounds", "sounds", horizon_prepare::sounds),
];

/// Adds what a prepared version lacks of `COMPLETIONS`: a new version reuses the resources of the
/// one before (`reuse_prepared`), which may come from a preparer that left them out. Without them
/// in any Minecraft folder, one line each in the log, once per launcher run, and the version plays
/// as before. Nothing is prepared again.
pub fn complete_assets(tag: &str) {
    complete_assets_in(&versions_dir(), tag, minecraft_folders, &|line| {
        eprintln!("{line}")
    });
}

/// Horizon's own download (an owner's) may have stopped before the sounds and the languages,
/// and the copies of `complete_assets` read them from its store: asked again, once per run, through
/// the same owner gate as any download from Mojang (`download_for_owner`), which keeps the files
/// already whole. Called where a download is expected (`reuse_prepared_everywhere`), never on Play.
fn refill_own_store(tag: &str) {
    static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let own = data_dir().join("minecraft");
    let resources = resources_at(&versions_dir().join(tag));
    if !own.join("assets/indexes").is_dir()
        || !horizon_prepare::missing_extras(&resources)
            .iter()
            .any(|(kind, _)| matches!(*kind, "languages" | "sounds"))
        || ASKED.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    let Some(pins) = read_json(&version_dir(tag).join("research-lock.json"))
        .and_then(|l| sources::Pins::read(&l))
    else {
        return;
    };
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        get(url)?
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(body)
    };
    if let Err(e) = download_for_owner(&pins, &own, &fetch, &|line| eprintln!("{line}")) {
        eprintln!("{tag}: the missing assets were not downloaded: {e}");
    }
}

/// `folders`: where to look, asked only when the version lacks something.
fn complete_assets_in(
    versions: &Path,
    tag: &str,
    folders: impl Fn() -> Vec<PathBuf>,
    log: &dyn Fn(String),
) {
    // The versions tried this run. Held while copying: the start, an install and Play can
    // complete the same version at once, and the copies' temporary names are per process.
    static TRIED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let root = versions.join(tag);
    let resources = resources_at(&root);
    let lacking = || -> Vec<&Completion> {
        if !resources.join(RESOURCES_MANIFEST).exists() {
            return Vec::new();
        }
        let missing = horizon_prepare::missing_extras(&resources);
        COMPLETIONS
            .iter()
            .filter(|(_, kind, _)| missing.iter().any(|(m, _)| m == kind))
            .collect()
    };
    if lacking().is_empty() {
        return;
    }
    let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
    let lacking = lacking();
    if tried.contains(&root) || lacking.is_empty() {
        return;
    }
    tried.push(root.clone());
    let folders = folders();
    for (what, _, copy) in lacking {
        let mut first_error = None;
        let added = folders.iter().any(|folder| {
            let copied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                copy(&root, Some(folder), &|line| log(line.to_string()))
            }))
            .unwrap_or_else(|_| Err("the copy stopped unexpectedly".into()));
            copied.map_err(|e| first_error.get_or_insert(e)).is_ok()
        });
        if !added {
            log(format!(
                "{tag}: {what} not added: {}",
                first_error.unwrap_or_else(|| "no Minecraft folder found".into())
            ));
        }
    }
}

/// The Minecraft folders whose asset store can hold what a prepared version lacks: the one
/// chosen in the launcher, the official launcher's (or HORIZON_MINECRAFT_DIR), the folder
/// other launchers' files are linked into and Horizon's own download, then every launcher's
/// default folder.
fn minecraft_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = Settings::load().minecraft_dir.into_iter().collect();
    folders.extend([
        horizon_prepare::artifacts::minecraft_dir(None),
        data_dir().join("linked-minecraft"),
        data_dir().join("minecraft"),
    ]);
    folders.extend(
        sources::Folders::current()
            .map(|f| sources::known_folders(sources::Os::current(), &f))
            .unwrap_or_default()
            .into_iter()
            .map(|(_, dir)| dir),
    );
    let mut found: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if folder.is_dir() && !found.contains(&folder) {
            found.push(folder);
        }
    }
    found
}

/// The official launcher's game directory, else the first other launcher's found
/// (sources.rs); preparing looks in all of them anyway.
pub fn default_minecraft_dir() -> Option<PathBuf> {
    let folders = sources::Folders::current()?;
    sources::known_folders(sources::Os::current(), &folders)
        .into_iter()
        .map(|(_, dir)| dir)
        .find(|dir| dir.is_dir())
}

fn found(program: &str, arg: &str) -> bool {
    crate::process::command(program)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn python() -> Option<&'static str> {
    let candidates: &[&'static str] = if cfg!(windows) {
        &["py", "python"]
    } else {
        &["python3", "python"]
    };
    candidates.iter().copied().find(|p| found(p, "--version"))
}

/// How a version's resources are prepared.
pub enum Preparer {
    /// horizon-prepare, linked into the launcher: the version ships the same preparer sources.
    InProcess(horizon_prepare::Options),
    /// The version's own preparer: its horizon-prepare binary when its sources differ from the
    /// launcher's (a newer or older release), or the Python script of versions made before it.
    Process(Command),
}

/// The version's horizon-prepare binary, when it ships one.
fn preparer_binary(dir: &Path) -> Option<PathBuf> {
    let binary = dir.join(if cfg!(windows) {
        "horizon-prepare.exe"
    } else {
        "horizon-prepare"
    });
    binary.is_file().then_some(binary)
}

/// Downloads the pinned version from Mojang into `dir` when an account in the launcher owns
/// Minecraft (asked again now), else says what signing in would do, as the last line of the
/// not-found error.
fn download_for_owner(
    pins: &sources::Pins,
    dir: &Path,
    fetch: sources::Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let mut accounts = account::Accounts::load();
    // The strict gate, never `launch_for_game`: the developer option that lets offline names
    // play without an owner must not download the game for them.
    let pass = account::launch(
        &account::Endpoints::live(),
        account::client_id().as_deref(),
        account::store(),
        &mut accounts,
    );
    if let Err(e) = accounts.save() {
        log(format!("Horizon: the accounts are not saved: {e}"));
    }
    download_for(pins, dir, pass, !accounts.accounts.is_empty(), fetch, log)
}

/// Downloads Minecraft `id` (any version on Mojang's list, its version file's sha1 from it) into
/// Horizon's own game folder, in the official launcher's layout, for an account that owns the
/// game (asked again now). Returns that folder.
pub fn download_version_for_owner(
    id: &str,
    manifest_sha1: &str,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let dir = data_dir().join("minecraft");
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        get(url)?
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(body)
    };
    let pins = sources::Pins::for_version(id, manifest_sha1);
    download_for_owner(&pins, &dir, &fetch, log)
        .map_err(|e| e.trim_start_matches("\n  • ").to_string())?;
    Ok(dir)
}

/// The command that starts a Java instance's Minecraft, for an account that owns the game: its
/// version downloaded from Mojang the first time (then it starts offline too), the Java it needs
/// (the instance's own, else one found here, else Mojang's, downloaded), and the game in the
/// instance's folder. The caller wraps it (`launch::wrap`) and never logs it: it holds the token.
/// A Java instance's game, ready to start; `server` (`host[:port]`) is joined once it has.
pub fn java_game(
    instance: &crate::instances::Instance,
    server: Option<&str>,
    log: &dyn Fn(String),
) -> Result<Command, String> {
    let game = instance.java.as_ref().ok_or_else(|| {
        format!(
            "{} plays Horizon, not Minecraft: Java Edition",
            instance.name
        )
    })?;
    let player = account::java_player()?;
    let minecraft = data_dir().join("minecraft");
    let id = mojang::relative(&game.minecraft)?;
    let natives = minecraft.join("natives").join(&id);
    // Each version and loader profile is fetched once; it then starts offline too.
    let downloaded = |id: &str| -> Result<PathBuf, String> {
        Ok(minecraft
            .join("versions")
            .join(mojang::relative(id)?)
            .join(".downloaded"))
    };
    let marker = downloaded(&game.minecraft)?;
    if !marker.is_file() {
        let listed = crate::minecraft_versions::load()?;
        let version = listed.find(&game.minecraft)?;
        download_version_for_owner(&version.id, &version.sha1, log)?;
        fs::write(&marker, "").map_err(|e| format!("{}: {e}", marker.display()))?;
    }
    let mut start = game.minecraft.clone();
    if let Some(loader) = &game.loader {
        start = loader.kind.profile_id(&loader.version, &game.minecraft);
        let marker = downloaded(&start)?;
        if !marker.is_file() {
            start = crate::loaders::install(loader, &game.minecraft, &minecraft, &get_bytes, log)?;
            fs::write(downloaded(&start)?, "").map_err(|e| e.to_string())?;
        }
    }
    let file = minecraft
        .join("versions")
        .join(mojang::relative(&start)?)
        .join(format!("{start}.json"));
    let version = crate::java_launch::resolve(crate::java_launch::read(&file)?, &minecraft)?;
    let need = crate::java::Need::of(&version);
    let found = crate::java::pick(
        &need,
        game.home.as_deref(),
        &crate::java::mojang_runtimes(&crate::java::runtimes_dir()),
        &crate::java::installed(),
    )?;
    let runtime = match found {
        Some(runtime) => runtime,
        None => {
            log(format!("Downloading Java {} from Mojang", need.major));
            crate::java::install(&need, log)?
        }
    };
    let jvm_args: Vec<String> = game.jvm_args.split_whitespace().map(String::from).collect();
    fs::create_dir_all(&instance.home).map_err(|e| format!("{}: {e}", instance.home.display()))?;
    crate::java_launch::command(
        &version,
        &crate::java_launch::Launch {
            minecraft: &minecraft,
            game_dir: &instance.home,
            natives: &natives,
            java: &runtime.java(),
            memory_mb: game
                .memory_mb
                .unwrap_or(crate::instances::DEFAULT_MEMORY_MB),
            jvm_args: &jvm_args,
            player: &player,
            os: crate::java_launch::os_name(),
            window: game.window,
            server,
        },
    )
}

/// What [`download_for_owner`] does with the owner gate's answer: Mojang's files only for its
/// `owner`, an account the services said owns the game just now.
fn download_for(
    pins: &sources::Pins,
    dir: &Path,
    pass: Result<account::Launch, account::NoOwner>,
    signed_in: bool,
    fetch: sources::Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let v = &pins.version;
    match pass {
        Ok(account::Launch {
            owner: Some((owner, _)),
            ..
        }) => {
            let name = owner.name;
            log(format!(
                "{name} owns Minecraft: downloading {v} from Mojang"
            ));
            let os = match sources::Os::current() {
                sources::Os::Linux => "linux",
                sources::Os::MacOs => "osx",
                sources::Os::Windows => "windows",
            };
            mojang::download(pins, dir, os, 8, fetch, &mojang::CANCEL, log).map_err(|e| {
                format!("\n  • Horizon tried to download {v} from Mojang for {name}, but: {e}")
            })
        }
        // An owner by its last answer, who can't be asked now.
        Ok(pass) => Err(format!(
            "\n  • Downloading {v} from Mojang needs your Microsoft account, which could not be \
             checked: {}",
            pass.offline.unwrap_or_default()
        )),
        Err(_) if signed_in => Err(
            "\n  • No signed-in account owns Minecraft: Java Edition, so Horizon can't download it."
                .into(),
        ),
        Err(_) => Err(format!(
            "\n  • Or sign in with the Microsoft account that owns Minecraft: Horizon then \
             downloads {v} from Mojang itself."
        )),
    }
}

/// The Java home for this version: a runtime for precompiled exporters, a JDK for older
/// preparers or changed sources. Forward the selection to external preparers too.
/// Else try the official launcher's Java in `games`, then download the runtime named by
/// `manifest` into Horizon's game folder (Mojang's runtimes are whole JDKs).
fn java(
    major: i64,
    compiler: bool,
    games: &[PathBuf],
    manifest: &Path,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let find = if compiler {
        horizon_prepare::java::find
    } else {
        horizon_prepare::java::find_runtime
    };
    if let Ok(jdk) = find(major, &[]) {
        return Ok(jdk.home());
    }
    let mut runtimes: Vec<PathBuf> = Vec::new();
    for home in games
        .iter()
        .flat_map(|game| horizon_prepare::java::minecraft_runtimes(&game.join("runtime")))
    {
        if !runtimes.contains(&home) {
            runtimes.push(home);
        }
    }
    let missing = match find(major, &runtimes) {
        Ok(jdk) => return Ok(jdk.home()),
        Err(e) => e,
    };
    let component = read_json(manifest)
        .and_then(|json| json["javaVersion"]["component"].as_str().map(String::from))
        .ok_or_else(|| missing.clone())?;
    // Mojang builds its Linux Java for glibc: without glibc's loader (NixOS without nix-ld,
    // musl distributions) it cannot start, so don't download it.
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) && !Path::new(GLIBC_LOADER).exists() {
        return Err(format!(
            "{missing}\n  • Minecraft's own Java from Mojang needs glibc ({GLIBC_LOADER} is missing; on NixOS, enable programs.nix-ld), so install compatible Java instead."
        ));
    }
    let fetch = |url: &str| -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        get(url)?
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(body)
    };
    let runtime = data_dir().join("minecraft").join("runtime");
    let home = mojang::java_runtime(&component, &runtime, 8, &fetch, &mojang::CANCEL, log)
        .map_err(|e| {
        format!("{missing}\n  • Horizon also tried to download Minecraft's own Java from Mojang, but: {e}")
    })?;
    find(major, &[home]).map(|jdk| jdk.home())
}

/// The dynamic loader glibc programs on x86-64 Linux start through.
pub(crate) const GLIBC_LOADER: &str = "/lib64/ld-linux-x86-64.so.2";

/// Runs `cmd` with Java at `home`: JAVA_HOME names it and its `bin` leads the PATH, as the
/// versions' own preparers look for Java.
fn with_java(cmd: &mut Command, home: &Path) {
    let mut path = vec![home.join("bin")];
    path.extend(
        std::env::var_os("PATH")
            .iter()
            .flat_map(std::env::split_paths),
    );
    cmd.env("JAVA_HOME", home);
    if let Ok(path) = std::env::join_paths(path) {
        cmd.env("PATH", path);
    }
}

/// The Minecraft folder a preparation whose game files are imported already reads: the sounds,
/// the title panorama, the fonts and the languages come from its asset store. It is the one
/// `resolve` found, linked into the official layout for another launcher's files; without one, the
/// chosen folder, and the preparation goes without them, saying so. Before, such a preparation
/// read the chosen folder as it was, which a Prism Launcher or Modrinth App install does not
/// keep its assets in: the game was silent and its title screen grey.
fn assets_folder(
    resolved: Result<sources::Resolved, String>,
    chosen: &Path,
    version: &str,
    log: &dyn Fn(String),
) -> PathBuf {
    match resolved {
        Ok(sources::Resolved::Folder(dir)) => dir,
        Ok(sources::Resolved::NotFound(_)) => {
            log(format!(
                "Minecraft {version}'s assets were not found in any launcher folder: the game \
                 will be silent and the title screen plain"
            ));
            chosen.to_path_buf()
        }
        Err(e) => {
            log(format!("Minecraft {version}'s assets: {e}"));
            chosen.to_path_buf()
        }
    }
}

/// Where the preparation takes Minecraft's files from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinecraftFrom {
    /// The player's own install (any launcher's); Mojang only when none has the version and an
    /// account that owns the game is signed in.
    Install,
    /// Mojang, for the signed-in account that owns the game ("Download and prepare").
    Mojang,
}

/// A preparation's steps, in the order they run, as Home names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrepareStep {
    /// Minecraft's files: found in an install, or downloaded from Mojang.
    Files,
    Java,
    Sounds,
    /// horizon-prepare's own work on those files.
    Prepare,
}

impl PrepareStep {
    pub const ALL: [PrepareStep; 4] = [Self::Files, Self::Java, Self::Sounds, Self::Prepare];

    pub fn name(self) -> &'static str {
        match self {
            Self::Files => "Game files",
            Self::Java => "Java",
            Self::Sounds => "Sounds",
            Self::Prepare => "Prepare",
        }
    }
}

/// The step the preparation under way is at (`PrepareStep::ALL`'s index).
static PREPARE_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn prepare_step() -> PrepareStep {
    PrepareStep::ALL[PREPARE_STEP.load(std::sync::atomic::Ordering::Relaxed)]
}

pub fn set_prepare_step(step: PrepareStep) {
    PREPARE_STEP.store(step as usize, std::sync::atomic::Ordering::Relaxed);
}

/// Checks what the preparation needs and picks how to run it. PLAY.txt describes the same
/// steps: the player's own launcher install, the server jar from that version's own json, and
/// Java (and Python, for versions before horizon-prepare). Also returns the Minecraft folder
/// to prepare from: `minecraft`, or the files another launcher keeps, linked into the official
/// launcher's layout (sources.rs).
pub fn preparer(
    tag: &str,
    minecraft: &Path,
    from: MinecraftFrom,
    log: &dyn Fn(String),
) -> Result<(Preparer, PathBuf), String> {
    set_prepare_step(PrepareStep::Files);
    let version = game_version(tag);
    let dir = version_dir(tag);
    let reused = horizon_prepare::artifacts::reuse_imported(&dir, &|line| log(line.to_string()))?;
    let chosen = minecraft.to_path_buf();
    // Resolved even when the game files are reused: the assets still come from the install.
    let minecraft = {
        let pins = read_json(&dir.join("research-lock.json"))
            .and_then(|lock| sources::Pins::read(&lock))
            .ok_or("The version's research-lock.json doesn't pin the Minecraft files")?;
        let own = data_dir().join("minecraft");
        let mut known = vec![("Horizon's own download", own.clone())];
        known.extend(
            sources::Folders::current()
                .map(|f| sources::known_folders(sources::Os::current(), &f))
                .unwrap_or_default(),
        );
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            let mut body = Vec::new();
            get(url)?
                .into_reader()
                .read_to_end(&mut body)
                .map_err(|e| e.to_string())?;
            Ok(body)
        };
        let linked = data_dir().join("linked-minecraft");
        if from == MinecraftFrom::Mojang {
            // Straight from Mojang (what is already there and whole is kept), for an owner only.
            download_for_owner(&pins, &own, &fetch, log)
                .map_err(|e| e.trim_start_matches("\n  • ").to_string())?;
            own
        } else {
            let resolved = sources::resolve(minecraft, &pins, &known, &linked, &fetch, log);
            if reused {
                assets_folder(resolved, &chosen, &version, log)
            } else {
                match resolved? {
                    sources::Resolved::Folder(dir) => dir,
                    sources::Resolved::NotFound(checked) => {
                        let not_found = sources::not_found(&version, &checked, minecraft);
                        download_for_owner(&pins, &own, &fetch, log).map_err(|e| not_found + &e)?;
                        own
                    }
                }
            }
        }
    };
    let manifest = minecraft
        .join("versions")
        .join(&version)
        .join(format!("{version}.json"));
    let binary = preparer_binary(&dir);
    let python = match binary {
        Some(_) => None,
        None => Some(python().ok_or(
            "This version prepares with Python: Python 3.10 or newer is needed. Install it, or pick a newer version.",
        )?),
    };
    // The Java the version pins, found as horizon-prepare finds it, so a missing one is
    // reported (or downloaded) before the server jar is downloaded. Versions from before
    // java_major ran Java 25.
    let major = read_json(&dir.join("research-lock.json"))
        .and_then(|lock| lock["minecraft"]["java_major"].as_i64())
        .unwrap_or(25);
    let mut games = vec![chosen, minecraft.clone(), data_dir().join("minecraft")];
    games.extend(
        sources::Folders::current()
            .map(|f| sources::known_folders(sources::Os::current(), &f))
            .unwrap_or_default()
            .into_iter()
            .map(|(_, dir)| dir),
    );
    set_prepare_step(PrepareStep::Java);
    let java_home = java(
        major,
        !horizon_prepare::exporters::available(&dir),
        &games,
        &manifest,
        log,
    )?;
    let jar = data_dir()
        .join("server-jars")
        .join(format!("{version}.jar"));
    if !reused && !jar.exists() {
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let url = json["downloads"]["server"]["url"]
            .as_str()
            .ok_or_else(|| format!("{} has no server download", manifest.display()))?;
        log(format!("Downloading the {version} server jar from Mojang"));
        fs::create_dir_all(jar.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut body = Vec::new();
        get(url)?
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        let partial = jar.with_extension("partial");
        fs::write(&partial, body)
            .and_then(|_| fs::rename(&partial, &jar))
            .map_err(|e| e.to_string())?;
    }
    if let Some(binary) = binary {
        if horizon_prepare::sources_sha256(&dir.join(PREPARER_CRATE)).as_deref()
            == Some(horizon_prepare::SOURCES_SHA256)
        {
            let options = horizon_prepare::Options {
                root: dir,
                import: !reused,
                minecraft_dir: Some(minecraft.clone()),
                server_jar: (!reused).then_some(jar),
                java_home: Some(java_home.clone()),
                inputs: Default::default(),
                output: None,
            };
            return Ok((Preparer::InProcess(options), minecraft));
        }
        let mut cmd = Command::new(binary);
        with_java(&mut cmd, &java_home);
        cmd.current_dir(&dir).arg("--root").arg(&dir);
        if !reused {
            cmd.args(["--prepare", "--server-jar"]).arg(&jar);
        }
        cmd.arg("--minecraft-dir").arg(&minecraft);
        return Ok((Preparer::Process(cmd), minecraft));
    }
    let mut cmd = Command::new(python.unwrap());
    with_java(&mut cmd, &java_home);
    cmd.current_dir(&dir)
        .args(["-u", "tools/prepare_native_client.py"]);
    if !reused {
        cmd.args(["--prepare", "--server-jar"]).arg(&jar);
    }
    cmd.arg("--minecraft-dir").arg(&minecraft);
    Ok((Preparer::Process(cmd), minecraft))
}

/// Copies the pinned version's sounds from the player's own launcher install into the version's
/// resources (`horizon-prepare sounds`), each line to `log`. Sound is optional, so a launcher
/// install without them is a line in the log, never a failed preparation. The version's own
/// preparer does it when it is not the launcher's and knows how; versions from before it read
/// the sounds where the launcher's preparer puts them.
pub fn sounds(tag: &str, minecraft: &Path, log: &dyn Fn(String)) {
    let dir = version_dir(tag);
    let own = horizon_prepare::sources_sha256(&dir.join(PREPARER_CRATE)).as_deref()
        == Some(horizon_prepare::SOURCES_SHA256);
    let binary = preparer_binary(&dir).filter(|binary| {
        !own && crate::process::command(binary)
            .arg("--help")
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("horizon-prepare sounds"))
    });
    set_prepare_step(PrepareStep::Sounds);
    log("Copying the sounds from your Minecraft install".into());
    let result = match binary {
        Some(binary) => crate::process::command(binary)
            .current_dir(&dir)
            .arg("sounds")
            .arg("--root")
            .arg(&dir)
            .arg("--minecraft-dir")
            .arg(minecraft)
            .output()
            .map_err(|e| e.to_string())
            .and_then(|o| {
                let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
                for line in text(&o.stdout).lines() {
                    log(line.to_string());
                }
                match o.status.success() {
                    true => Ok(()),
                    false => Err(text(&o.stderr).trim_start_matches("error: ").to_string()),
                }
            }),
        None => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            horizon_prepare::sounds(&dir, Some(minecraft), &|line| log(line.to_string()))
        }))
        .unwrap_or_else(|_| Err("The sound copy stopped unexpectedly".into()))
        .map(|_| ()),
    };
    if let Err(e) = result {
        log(format!("No sound (the game will be silent): {e}"));
    }
}

/// Runs `Preparer::InProcess` on this thread, each progress line to `log`. A panic is an
/// error like any other, so the window never waits on a preparation that died.
pub fn prepare_in_process(
    options: &horizon_prepare::Options,
    log: &dyn Fn(String),
) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        horizon_prepare::prepare(options, &|line| log(line.to_string()))
    }))
    .unwrap_or_else(|_| Err("The preparation stopped unexpectedly".into()))
    .map(|_| ())
}

/// Where the game keeps saves and options.txt: `HORIZON_HOME`, else `horizon` in the platform's
/// data folder (~/.local/share/horizon on Linux, as the release folders' horizon.sh uses). It is
/// never inside a version folder, so installing, updating or removing a version keeps the saves.
pub fn game_home() -> PathBuf {
    if let Some(home) = std::env::var_os("HORIZON_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("horizon")
}

/// Whether No AI is on: every AI feature hidden and off (`horizon_core::no_ai`, kept in the
/// game's `horizon-extras.txt` so the game reads the same switch).
pub fn no_ai() -> bool {
    horizon_core::no_ai::is_on(&game_home())
}

/// Turns No AI on or off for the launcher and the game.
pub fn set_no_ai(on: bool) -> Result<(), String> {
    horizon_core::no_ai::set(&game_home(), on).map_err(|e| format!("Could not save No AI: {e}"))
}

// ----- Logs and crash reports ---------------------------------------------------------------------

/// The game's logs (`latest.log` and the day archives), as it writes them in its home.
pub fn logs_dir() -> PathBuf {
    game_home().join("logs")
}

/// Where the game writes `crash-<date>_<time>-client.txt` when it crashes.
pub fn crash_reports_dir() -> PathBuf {
    game_home().join("crash-reports")
}

/// The crash reports in `dir`, oldest first (their names sort by time).
fn crash_reports_in(dir: &Path) -> Vec<PathBuf> {
    let mut reports: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    reports.sort();
    reports
}

/// The newest crash report, if any.
pub fn newest_crash_report() -> Option<PathBuf> {
    crash_reports_in(&crash_reports_dir()).pop()
}

/// The newest crash report the game playing in `home` wrote at or after `since` (one from the
/// run that started then).
pub fn crash_report_since(home: &Path, since: std::time::SystemTime) -> Option<PathBuf> {
    crash_report_in_since(&home.join("crash-reports"), since)
}

fn crash_report_in_since(dir: &Path, since: std::time::SystemTime) -> Option<PathBuf> {
    crash_reports_in(dir).into_iter().rev().find(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= since)
    })
}

/// The error a crash report describes, on one line: the lines between its description and the
/// stack (`panicked at src/game.rs:10:5: boom`).
pub fn crash_summary(report: &str) -> Option<String> {
    let mut lines = report
        .lines()
        .skip_while(|l| !l.starts_with("Description:"));
    lines.next()?;
    let error: Vec<&str> = lines
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty() && !l.starts_with('\t'))
        .map(str::trim)
        .collect();
    (!error.is_empty()).then(|| error.join(" "))
}

/// The line the game prints when it has drawn its first frame.
pub const GAME_FIRST_FRAME: &str = "Horizon: first frame";

/// Whether a game's output ends in a crash while its Vulkan renderer started: an adapter line
/// (`Horizon renderer: <GPU> (Vulkan, <type>)`) and no first frame. A native crash in a graphics
/// driver leaves no crash report, only those lines (#746).
pub fn vulkan_start_crashed<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> bool {
    let mut vulkan = false;
    for line in lines {
        let line = line.as_ref();
        if line.starts_with(GAME_FIRST_FRAME) {
            return false;
        }
        if let Some(adapter) = line.strip_prefix("Horizon renderer: ") {
            // The adapter line, not one of the start's steps: "<GPU> (<backend>, <type>)".
            if adapter.ends_with(')') && adapter.contains(" (") {
                vulkan = adapter.contains(" (Vulkan, ");
            }
        }
    }
    vulkan
}

/// The game's graphics settings file in its home (the game's `graphics.rs`).
const GRAPHICS_FILE: &str = "horizon-graphics.txt";

/// Sets `graphicsApi:dx12` in `home`'s graphics settings when it is unset or `auto`, keeping
/// every other line. True when it changed; a player's own `vulkan` or `dx12` stays.
pub fn switch_to_dx12(home: &Path) -> Result<bool, String> {
    let path = home.join(GRAPHICS_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;
    for line in text.lines() {
        match line.split_once(':') {
            Some((key, value)) if key.trim() == "graphicsApi" => {
                if value.trim() != "auto" {
                    return Ok(false);
                }
                found = true;
                lines.push("graphicsApi:dx12".to_owned());
            }
            _ => lines.push(line.to_owned()),
        }
    }
    if !found {
        lines.push("graphicsApi:dx12".to_owned());
    }
    fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    fs::write(&path, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// The last `lines` lines of `file`, or why it cannot be read.
fn tail_of(file: &Path, lines: usize) -> Result<String, String> {
    let bytes = fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..].join("\n") + "\n")
}

/// `logs`: the end of the game's latest.log, or with `crash` its newest crash report. The path
/// read comes first.
pub fn read_logs(lines: usize, crash: bool) -> Result<(PathBuf, String), String> {
    if crash {
        let path = newest_crash_report()
            .ok_or_else(|| format!("No crash report in {}.", crash_reports_dir().display()))?;
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok((path, text));
    }
    let path = logs_dir().join("latest.log");
    let text = tail_of(&path, lines.max(1))?;
    Ok((path, text))
}

/// Splits the player's extra game arguments (Settings' one line) into words, the way a shell
/// would: spaces separate them, single or double quotes group a word with spaces in it, and a
/// backslash escapes the next character (outside quotes, or inside double quotes).
pub fn split_args(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' {
                    // Inside double quotes only `"` and `\` are escaped, as a shell does.
                    match chars.next() {
                        Some(n @ ('"' | '\\')) => word.push(n),
                        Some(n) => {
                            word.push('\\');
                            word.push(n);
                        }
                        None => word.push('\\'),
                    }
                } else {
                    word.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                '\\' => {
                    if let Some(n) = chars.next() {
                        word.push(n);
                        started = true;
                    }
                }
                c if c.is_whitespace() => {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                _ => {
                    word.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// The extra game arguments the player set in Settings, split into words.
pub fn game_args() -> Vec<String> {
    split_args(&Settings::load().game_args)
}

/// Appends the player's extra game arguments, so they add to the launcher's own or override them.
pub fn append_game_args(cmd: &mut Command) {
    cmd.args(game_args());
}

pub fn client_command(tag: &str) -> Command {
    let home = game_home();
    let _ = fs::create_dir_all(&home);
    let mut cmd = Command::new(client_program(tag));
    cmd.current_dir(home).arg("--resources").arg(resources(tag));
    // How the game asks for a renewed session (`account session`) while it runs.
    if let Ok(exe) = std::env::current_exe() {
        cmd.env("HORIZON_LAUNCHER_EXE", exe);
    }
    // PipeWire remembers volume and mute per application name. The game's own name is
    // "PipeWire ALSA [horizon-client]", shared with development and test runs, where
    // muting it once left every later game silent: played from here it is "Horizon".
    if std::env::var_os("PIPEWIRE_PROPS").is_none() {
        cmd.env(
            "PIPEWIRE_PROPS",
            "{ application.name = \"Horizon\" media.role = \"Game\" }",
        );
    }
    cmd
}

/// The game's program: on macOS the one inside the version's Horizon.app, so the Dock and Cmd+Tab
/// show Horizon's name and icon (game_app.rs); the bare binary elsewhere, or if that fails.
fn client_program(tag: &str) -> PathBuf {
    let dir = version_dir(tag);
    #[cfg(target_os = "macos")]
    match crate::game_app::ensure(&dir) {
        Ok(program) => return program,
        Err(e) => eprintln!("Starting the game outside {}: {e}", crate::game_app::APP),
    }
    dir.join(CLIENT)
}

pub fn server_command(tag: &str, folder: &Path) -> Command {
    let mut cmd = Command::new(version_dir(tag).join(CLIENT));
    cmd.current_dir(folder)
        .arg("--resources")
        .arg(resources(tag))
        .args(["--dedicated", "--nogui"]);
    cmd
}

pub fn eula_agreed(folder: &Path) -> bool {
    fs::read_to_string(folder.join("eula.txt"))
        .is_ok_and(|s| s.lines().any(|l| l.trim() == "eula=true"))
}

/// Writes eula.txt as vanilla's server expects it, once the player agreed in the launcher.
pub fn agree_eula(folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let text = "#By changing the setting below to TRUE you are indicating your agreement to our EULA (https://aka.ms/MinecraftEULA).\neula=true\n";
    fs::write(folder.join("eula.txt"), text).map_err(|e| e.to_string())
}

// ----- Shared steps for the front ends -----------------------------------------------------------

/// The installed version to use: the one asked for, else the selected one, else the newest.
pub fn resolve_version(requested: Option<&str>) -> Result<String, String> {
    let installed = installed();
    if let Some(tag) = requested {
        return installed
            .iter()
            .find(|t| *t == tag)
            .cloned()
            .ok_or_else(|| format!("{tag} is not installed."));
    }
    let selected = Settings::load().selected.filter(|s| installed.contains(s));
    selected
        .or_else(|| installed.first().cloned())
        .ok_or_else(|| "No version is installed yet.".into())
}

/// Removes an installed version, unless a program runs from it, and moves the selection to the
/// newest one left if needed.
pub fn remove_version(tag: &str) -> Result<(), String> {
    if !installed().iter().any(|t| t == tag) {
        return Err(format!("{tag} is not installed."));
    }
    if in_use(tag) {
        return Err(format!("{tag} is running. Quit it first."));
    }
    remove(tag)?;
    let mut settings = Settings::load();
    if settings.selected.as_deref() == Some(tag) {
        settings.selected = installed().first().cloned();
        settings.save();
    }
    Ok(())
}

/// How many of the newest release versions stay installed when the player has not said.
const KEEP_NEWEST: usize = 2;

/// The release versions to delete: all but the `keep` newest of `installed` (newest first), and
/// never one of `protected`. Nightly builds are left out: `nightly::after_update` removes the
/// older ones.
fn stale_versions(installed: &[String], keep: usize, protected: &[String]) -> Vec<String> {
    installed
        .iter()
        .filter(|t| is_game_tag(t))
        .skip(keep)
        .filter(|t| !protected.contains(t))
        .cloned()
        .collect()
}

/// Whether a running program was started from the version's folder: the game and a server take
/// their program and `--resources` from it. Windows refuses to delete a running program's files,
/// so there the delete itself fails. With no answer from `ps` the version counts as running.
#[cfg(unix)]
fn in_use(tag: &str) -> bool {
    let dir = format!("{}/", version_dir(tag).display());
    Command::new("ps")
        .args(["-A", "-ww", "-o", "args="])
        .output()
        .ok()
        .is_none_or(|out| String::from_utf8_lossy(&out.stdout).contains(&dir))
}

#[cfg(not(unix))]
fn in_use(_tag: &str) -> bool {
    false
}

/// Deletes the release versions nothing needs after an install or update, unless the player
/// turned that off: all but the newest few (`Settings::keep_newest`), the selected version, the
/// ones instances pin and the ones a program is running from. A version that does not delete
/// is logged and the rest still go. Returns the versions deleted. They install again from GitHub.
pub fn prune_old_versions() -> Vec<String> {
    let settings = Settings::load();
    if !settings.auto_prune() {
        return Vec::new();
    }
    let mut protected: Vec<String> = settings.selected.iter().cloned().collect();
    protected.extend(
        crate::instances::list()
            .into_iter()
            .filter_map(|i| i.version),
    );
    let mut gone = Vec::new();
    for tag in stale_versions(&installed(), settings.keep_newest(), &protected) {
        if in_use(&tag) {
            eprintln!("Keeping old version {tag}: the game is running from it.");
            continue;
        }
        match remove(&tag) {
            Ok(()) => {
                eprintln!("Removed old version {tag}.");
                gone.push(tag);
            }
            Err(e) => eprintln!("Could not remove old version {tag}: {e}"),
        }
    }
    gone
}

/// The Minecraft folder to prepare from: the one given, the one chosen in the launcher, or the
/// official launcher's default. It may be absent when imported files can be reused.
pub fn minecraft_dir(requested: Option<PathBuf>) -> Result<PathBuf, String> {
    Ok(requested
        .or(Settings::load().minecraft_dir)
        .or_else(default_minecraft_dir)
        .unwrap_or_else(|| horizon_prepare::artifacts::minecraft_dir(None)))
}

/// The server folder: the one given, the one chosen in the launcher, or `server/` in the data folder.
pub fn server_dir(requested: Option<PathBuf>) -> PathBuf {
    requested
        .or(Settings::load().server_dir)
        .unwrap_or_else(|| data_dir().join("server"))
}

/// Whether `tag` is prepared, with the run-time reports it needs completed when another copy has them.
pub fn need_prepared(tag: &str) -> Result<(), String> {
    if is_prepared(tag) {
        complete_reports(&versions_dir(), tag);
        Ok(())
    } else {
        Err(format!(
            "{tag} is not prepared yet. Prepare it first: `horizon-launcher prepare --version {tag}` \
             uses your own Minecraft {v}, or downloads it from Mojang for the signed-in account \
             that owns it.",
            v = game_version(tag)
        ))
    }
}

/// The client's command, once the owner gate lets it start and its resources are prepared. Past
/// the gate, a sign-in that can't be renewed (no network, expired) still lets the game start,
/// offline.
pub fn client(tag: &str) -> Result<Command, String> {
    client_as(tag, None)
}

/// [`client`], playing as `who` for this launch only (`play --account`).
pub fn client_as(tag: &str, who: Option<&account::Player>) -> Result<Command, String> {
    let (session, offline) = match who {
        None => account::session_for_game(&game_home(), false).map_err(|e| e.to_string()),
        Some(who) => account::session_for_game_as(&game_home(), who),
    }
    .map_err(|refused| format!("{refused} Sign in with `horizon-launcher account sign-in`."))?;
    need_prepared(tag)?;
    complete_assets(tag);
    crate::browser::horizon::before_play(&game_home());
    let mut cmd = client_command(tag);
    if let Some(file) = session {
        cmd.arg("--session-file").arg(file);
    }
    if let Some(e) = offline {
        eprintln!("Playing offline: {e}");
    }
    if let Some(e) = account::take_unsaved() {
        eprintln!("{e}");
    }
    append_game_args(&mut cmd);
    Ok(cmd)
}

/// Start the game straight into a friend's world: `--server friend:CODE`.
pub fn join_friend(cmd: &mut Command, code: &str) -> Result<(), String> {
    horizon_friends::invite::parse_detailed(code).map_err(|e| e.to_string())?;
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    cmd.arg("--server")
        .arg(format!("{}{code}", horizon_friends::ADDRESS_PREFIX));
    Ok(())
}

/// The dedicated server's command in `folder`, once prepared and the EULA is agreed to.
pub fn server(tag: &str, folder: &Path, accept_eula: bool) -> Result<Command, String> {
    need_prepared(tag)?;
    if accept_eula {
        agree_eula(folder)?;
    }
    if !eula_agreed(folder) {
        return Err("Running a server needs your agreement to the Minecraft EULA (https://aka.ms/MinecraftEULA). Accept it to continue.".into());
    }
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    Ok(server_command(tag, folder))
}

/// One release's notes: the tag given, or the newest.
pub fn release_notes(tag: Option<&str>) -> Result<Release, String> {
    let releases = releases()?;
    match tag {
        Some(tag) => releases
            .into_iter()
            .find(|r| r.tag == tag)
            .ok_or_else(|| format!("No Horizon release is tagged {tag}.")),
        None => releases
            .into_iter()
            .next()
            .ok_or_else(|| "No release is published yet.".into()),
    }
}

/// A process whose output is kept: the last `LOG_KEEP` lines, and its result once it ends.
pub struct Logged {
    pub running: Running,
    pub log: Arc<Mutex<std::collections::VecDeque<String>>>,
    pub exit: Arc<Mutex<Option<Result<(), String>>>>,
}

pub const LOG_KEEP: usize = 2000;

impl Logged {
    pub fn start(job: Job, cmd: Command) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let running = spawn(job, cmd, tx)?;
        let log = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let exit = Arc::new(Mutex::new(None));
        let (l, e) = (log.clone(), exit.clone());
        std::thread::spawn(move || {
            for msg in rx {
                match msg {
                    Msg::Line(_, line) => {
                        let mut log = l.lock().unwrap();
                        log.push_back(line);
                        if log.len() > LOG_KEEP {
                            log.pop_front();
                        }
                    }
                    Msg::Exit(_, result) => *e.lock().unwrap() = Some(result),
                    _ => {}
                }
            }
        });
        Ok(Self { running, log, exit })
    }

    pub fn finished(&self) -> Option<Result<(), String>> {
        self.exit.lock().unwrap().clone()
    }

    pub fn tail(&self, lines: usize) -> Vec<String> {
        let log = self.log.lock().unwrap();
        log.iter()
            .skip(log.len().saturating_sub(lines))
            .cloned()
            .collect()
    }

    pub fn wait(&self) -> Result<(), String> {
        loop {
            if let Some(result) = self.finished() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Asks a server to stop, and kills it if it has not after `grace`.
    pub fn stop(&mut self, grace: std::time::Duration) {
        self.running.send("stop");
        let start = std::time::Instant::now();
        while self.finished().is_none() && start.elapsed() < grace {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if self.finished().is_none() {
            self.running.kill();
        }
    }
}

// ----- Updates -----------------------------------------------------------------------------------

/// What a check found: a Horizon build newer than every installed one, and a newer launcher.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Updates {
    pub horizon: Option<Release>,
    pub launcher: Option<Release>,
}

/// The newest Horizon build with a build here (the current Nightly), if it isn't installed and
/// no installed version is newer. Two Nightlies of one day compare equal: the one on GitHub is
/// the update.
pub fn horizon_update(releases: &[Release], installed: &[String]) -> Option<Release> {
    let newest = latest(releases)?;
    let newer = !installed.contains(&newest.tag)
        && installed
            .iter()
            .all(|t| compare_versions(&newest.tag, t).is_ge());
    newer.then(|| newest.clone())
}

/// The newest launcher release with an archive for this platform, if newer than this launcher
/// ([`own_tag`]: a launcher from before 0.2 takes any `launcher@` release).
pub fn launcher_update(releases: &[Release]) -> Option<Release> {
    launcher_update_from(releases, &own_tag())
}

fn launcher_update_from(releases: &[Release], own: &str) -> Option<Release> {
    releases
        .iter()
        .find(|r| r.asset.is_some())
        .filter(|r| compare_versions(&r.tag, own).is_gt())
        .cloned()
}

/// Load the game's release list and both update kinds from the same GitHub response.
/// Publish the game releases before any fallback lookups for older launcher tags, so those
/// lookups cannot hold up the release pages or opening animation. A list error reaches both.
pub fn releases_and_updates(
    on_releases: impl FnOnce(Result<Vec<Release>, String>),
) -> Result<Updates, String> {
    releases_and_updates_with(release_list, installed, launcher_releases_in, on_releases)
}

fn releases_and_updates_with(
    load: impl FnOnce() -> Result<serde_json::Value, String>,
    installed: impl FnOnce() -> Vec<String>,
    launchers: impl FnOnce(&serde_json::Value) -> Vec<Release>,
    on_releases: impl FnOnce(Result<Vec<Release>, String>),
) -> Result<Updates, String> {
    let list = match load() {
        Ok(list) => list,
        Err(e) => {
            on_releases(Err(e.clone()));
            return Err(e);
        }
    };
    let releases = game_releases(&list);
    let horizon = horizon_update(&installable_updates(&releases), &installed());
    on_releases(Ok(releases));
    Ok(Updates {
        horizon,
        launcher: launcher_update(&launchers(&list)),
    })
}

/// A fresh release-list request for both kinds of update (plus older-launcher fallback if needed).
pub fn check_updates() -> Result<Updates, String> {
    releases_and_updates(|_| {})
}

/// Whether a version installed as an update becomes the selected one: when the player had none,
/// was on a Nightly, or on the newest installed release (so a player of the release channel moves
/// to the Nightly at the first check, docs/release-flow.md). An older release picked by hand
/// stays; an instance pinned to a version keeps it either way.
pub fn select_after_update(selected: Option<&str>, installed_before: &[String]) -> bool {
    match selected {
        None => true,
        Some(s) if crate::nightly::date_of(s).is_some() => true,
        Some(s) => installed_before
            .iter()
            .find(|t| is_game_tag(t))
            .is_none_or(|newest| newest == s),
    }
}

/// Installs `tag` (or with "latest" the newest release, pre-releases left out), then updates the selection as
/// `select_after_update` says when `as_update`, or always selects it otherwise.
pub fn install_tag(tag: &str, as_update: bool, progress: impl Fn(f32)) -> Result<String, String> {
    let releases = releases()?;
    let release = if tag == "latest" {
        releases
            .iter()
            .find(|r| r.asset.is_some() && !r.prerelease)
            .ok_or("No release has a build for this platform yet.")?
    } else {
        releases
            .iter()
            .find(|r| r.tag == tag)
            .ok_or_else(|| format!("No Horizon release is tagged {tag}."))?
    };
    if release.asset.is_none() {
        return Err(format!("{} has no build for this platform.", release.tag));
    }
    let before = installed();
    install(release, progress)?;
    let mut settings = Settings::load();
    if !as_update || select_after_update(settings.selected.as_deref(), &before) {
        settings.selected = Some(release.tag.clone());
        settings.save();
    }
    if as_update && release.nightly.is_some() {
        crate::nightly::after_update(&release.tag);
    }
    prune_old_versions();
    Ok(release.tag.clone())
}

/// The launcher's own path, read once at start: after a self-update the running file is gone.
pub fn exe_path() -> &'static Path {
    static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().unwrap_or_else(|_| PathBuf::from(LAUNCHER_BINARY)))
}

/// The AppImage this launcher runs from, when it does: the runtime sets `$APPIMAGE` to the file it
/// mounted and `$APPDIR` to the mount, while [`exe_path`] then names a copy inside that read-only
/// mount, gone with it. Updating and restarting work on this file, and the menu entry, shortcuts and
/// agent configurations start it.
pub fn appimage_path() -> Option<PathBuf> {
    appimage_path_in(
        std::env::var_os("APPIMAGE").as_deref(),
        std::env::var_os("APPDIR").as_deref(),
        exe_path(),
    )
}

/// `$APPIMAGE`, if it names a file still there and this binary runs from inside `$APPDIR`. Both
/// variables reach every child of an AppImage: a launcher a different AppImage's program started
/// must never take that program's file for its own (an update would overwrite it).
fn appimage_path_in(
    appimage: Option<&std::ffi::OsStr>,
    appdir: Option<&std::ffi::OsStr>,
    exe: &Path,
) -> Option<PathBuf> {
    let appdir = Path::new(appdir?);
    if !appdir.is_absolute() || !exe.starts_with(appdir) {
        return None;
    }
    appimage
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
}

/// The file a self-update replaces and a restart starts: the AppImage when the launcher runs from
/// one, the binary this process started from otherwise.
pub fn update_target() -> PathBuf {
    appimage_path().unwrap_or_else(|| exe_path().to_path_buf())
}

/// The launcher archive this build installs from: the AppImage's when it runs from one (a `.tar.gz`
/// binary would be written inside the read-only mount), this platform's otherwise.
pub fn launcher_asset() -> Option<&'static str> {
    launcher_asset_running_from(appimage_path().is_some())
}

fn launcher_asset_running_from(appimage: bool) -> Option<&'static str> {
    if appimage {
        LAUNCHER_APPIMAGE_ASSET
    } else {
        LAUNCHER_ASSET
    }
}

/// Development builds and `HORIZON_LAUNCHER_NO_SELF_UPDATE` never replace themselves.
pub fn self_update_allowed() -> bool {
    !cfg!(debug_assertions) && std::env::var_os("HORIZON_LAUNCHER_NO_SELF_UPDATE").is_none()
}

/// The version in `horizon-launcher --version`'s output (`horizon-launcher 0.1.5`).
fn parse_version_output(out: &str) -> Option<&str> {
    out.trim()
        .strip_prefix("horizon-launcher ")
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// The version of the launcher binary now at `path`, which is not this process's once another
/// update (the command line, another window) has replaced the file.
fn installed_launcher_version(path: &Path) -> Option<String> {
    let mut cmd = crate::process::command(path);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    let out = cmd.output().ok()?;
    parse_version_output(&String::from_utf8_lossy(&out.stdout)).map(String::from)
}

/// Puts `new` at `exe`, the path this launcher started from. The running process keeps its
/// file (unlinked) and works on. Unix only: `self_replace` asks the system where the running
/// binary is, and Linux answers "<path> (deleted)" once the file has been replaced, so a second
/// update in the same run failed with "No such file or directory".
#[cfg(unix)]
fn replace_binary(new: &Path, exe: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "the launcher has no folder")
    })?;
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.update-{}", std::process::id()));
    let mode = fs::metadata(exe).map_or(0o755, |m| m.permissions().mode() | 0o111);
    let result = fs::copy(new, &temp)
        .and_then(|_| fs::set_permissions(&temp, fs::Permissions::from_mode(mode)))
        .and_then(|_| fs::rename(&temp, exe));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Windows keeps a running binary from being written or deleted but lets it be renamed: the
/// running one steps aside to `<name>.old-<pid>-<n>` in its folder, the new one takes its path,
/// and a later start removes the old file (`remove_replaced_binaries`). The `self_replace` crate
/// did this by copying the launcher under a random hidden name and starting that copy to delete
/// the original through a hidden `cmd.exe`, which Microsoft Defender quarantines as
/// Trojan:Script/Wacatac (#3175).
#[cfg(not(unix))]
fn replace_binary(new: &Path, exe: &Path) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU32, Ordering};
    // A second update in the same run moves the first one's binary aside too.
    static MOVED: AtomicU32 = AtomicU32::new(0);
    let dir = exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "the launcher has no folder")
    })?;
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    let pid = std::process::id();
    let temp = dir.join(format!("{name}.update-{pid}"));
    let old = dir.join(format!(
        "{name}.old-{pid}-{}",
        MOVED.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = fs::copy(new, &temp) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    match fs::rename(exe, &old) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        _ => {}
    }
    if let Err(e) = fs::rename(&temp, exe) {
        let _ = fs::rename(&old, exe);
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

/// Deletes the binaries earlier self-updates moved aside (`replace_binary`), and the copies of an
/// update that stopped before its new binary took its place (`<name>.update-<pid>`). The one the
/// launcher just restarted from is still exiting, so this keeps trying for a few seconds in the
/// background; one that stays in use (another launcher window) goes on a later start.
#[cfg(windows)]
pub fn remove_replaced_binaries() {
    let exe = exe_path();
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) else {
        return;
    };
    let dir = dir.to_path_buf();
    let name = name.to_string_lossy();
    let old = format!("{name}.old-");
    let update = format!("{name}.update-");
    let own = format!("{update}{}", std::process::id());
    std::thread::spawn(move || {
        for _ in 0..10 {
            let Ok(entries) = fs::read_dir(&dir) else {
                return;
            };
            let mut left = false;
            for entry in entries.flatten() {
                let file = entry.file_name();
                let file = file.to_string_lossy();
                if file.starts_with(&old) || (file.starts_with(&update) && file != own) {
                    left |= fs::remove_file(entry.path()).is_err();
                }
            }
            if !left {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}

/// The app bundle the launcher runs from (`…/Horizon.app` for `…/Horizon.app/Contents/MacOS/
/// horizon-launcher`), or None for a bare binary, as launchers before the .app shipped.
pub fn app_bundle(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then_some(app)
}

/// Finds the first `*.app` folder under `dir`.
fn find_app(dir: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.extension().is_some_and(|e| e == "app") {
                return Some(path);
            }
            if let Some(found) = find_app(&path) {
                return Some(found);
            }
        }
    }
    None
}

/// Puts a verified app bundle in place, retaining old bundles until no process uses them.
#[cfg(target_os = "macos")]
fn replace_bundle(new: &Path, app: &Path) -> Result<(), String> {
    crate::mac_update::replace_bundle(new, app)
}

/// Retired macOS launchers are removed only once their executable is no longer in use.
/// Run for every entry point (GUI, CLI and MCP), before a command can return or exec.
pub fn cleanup_launcher_updates() {
    #[cfg(target_os = "macos")]
    if let Some(app) = app_bundle(exe_path()) {
        crate::mac_update::cleanup(app);
    }
}

#[cfg(not(target_os = "macos"))]
fn replace_bundle(_new: &Path, _app: &Path) -> Result<(), String> {
    Err("App bundles are a macOS thing.".into())
}

/// Downloads a launcher release and puts it in place of this one. The running process keeps
/// working; `restart` starts the new one. When the file there already is that release or
/// a newer one (updated from the command line or another window), there is nothing to do.
pub fn self_update(release: &Release, progress: impl Fn(f32)) -> Result<(), String> {
    let (url, size) = release
        .asset
        .clone()
        .ok_or("This launcher release has no build for this platform.")?;
    let target = launcher_version(&release.tag).unwrap_or("0");
    if installed_launcher_version(&update_target())
        .is_some_and(|v| v != VERSION && compare_versions(&v, target).is_ge())
    {
        progress(1.0);
        return Ok(());
    }
    let sha256 = signed_digest(&release.tag, launcher_asset().unwrap_or_default())?;
    let work = data_dir().join(".launcher-update");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let archive = work.join("download");
    download(&url, size, &sha256, &archive, &progress)?;
    // An AppImage is the whole program: the new one takes the place of the file `$APPIMAGE` names
    // (the running mount keeps its copy until `restart` starts it). Its archive is not a `.tar.gz`
    // to unpack, so this comes before the unpacking below.
    if let Some(appimage) = appimage_path() {
        if !url.ends_with(".AppImage") {
            return Err(format!(
                "{} is not an AppImage, and this launcher runs from one. Install it from its release page instead.",
                release.tag
            ));
        }
        replace_binary(&archive, &appimage).map_err(|e| {
            format!(
                "Could not replace the AppImage at {}: {e}",
                appimage.display()
            )
        })?;
        let _ = fs::remove_dir_all(&work);
        progress(1.0);
        return Ok(());
    }
    let unpacked = work.join("unpacked");
    unpack(&archive, &url, &unpacked)?;
    // A launcher inside Horizon.app replaces the whole bundle (its Info.plist, icon and
    // signature belong with the binary); a bare binary, as older launchers are, only the binary,
    // which the macOS archive also holds (Horizon.app/Contents/MacOS/horizon-launcher).
    if let Some(app) = app_bundle(exe_path()) {
        if let Some(new) = find_app(&unpacked) {
            replace_bundle(&new, app)?;
            let _ = fs::remove_dir_all(&work);
            progress(1.0);
            return Ok(());
        }
    }
    let binary =
        find_file(&unpacked, LAUNCHER_BINARY).ok_or("The download has no launcher binary.")?;
    replace_binary(&binary, exe_path()).map_err(|e| {
        format!(
            "Could not replace the launcher at {}: {e}",
            exe_path().display()
        )
    })?;
    let _ = fs::remove_dir_all(&work);
    progress(1.0);
    Ok(())
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if entry.file_name() == name {
            return Some(path);
        }
    }
    None
}

/// Starts the launcher again (the updated binary, or the replaced AppImage) with the same
/// arguments in place of this one: on Unix the process becomes the new binary (exec), on Windows
/// the new one starts and this one exits. Returns only when the new one could not start.
pub fn restart() -> String {
    let target = update_target();
    let mut cmd = Command::new(&target);
    cmd.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    let error = {
        use std::os::unix::process::CommandExt;
        cmd.exec()
    };
    #[cfg(not(unix))]
    let error = match cmd.spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => e,
    };
    format!(
        "Could not start {}: {error}. Close the launcher and open it again.",
        target.display()
    )
}

/// Plain text from markdown, for terminals: headings on their own lines, bullets as "- ",
/// links followed by their address.
pub fn markdown_to_text(md: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut out = String::new();
    let mut links = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for event in Parser::new(md) {
        match event {
            Event::Start(Tag::Item) => {
                let depth = lists.len().saturating_sub(1);
                out.push_str(&"  ".repeat(depth));
                match lists.last_mut() {
                    Some(Some(n)) => {
                        out.push_str(&format!("{n}. "));
                        *n += 1;
                    }
                    _ => out.push_str("- "),
                }
            }
            Event::Start(Tag::List(start)) => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                lists.push(start)
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    out.push('\n');
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => links.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                if let Some(url) = links.pop() {
                    out.push_str(&format!(" ({url})"));
                }
            }
            Event::End(TagEnd::Heading(_)) => out.push_str("\n\n"),
            Event::End(TagEnd::Paragraph) => {
                out.push_str(if lists.is_empty() { "\n\n" } else { "\n" })
            }
            Event::End(TagEnd::Item) if !out.ends_with('\n') => out.push('\n'),
            Event::End(TagEnd::CodeBlock) => out.push('\n'),
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("---\n\n"),
            _ => {}
        }
    }
    out.trim_end().to_string() + "\n"
}

/// The first sentence-like line of a release's notes, for a one-line summary.
pub fn notes_summary(body: &str) -> Option<String> {
    let text = markdown_to_text(body);
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.len() > 24 && !l.starts_with("---"))?;
    let line = line.trim_start_matches("- ");
    Some(match line.char_indices().nth(140) {
        Some((i, _)) => format!("{}…", line[..i].trim_end()),
        None => line.to_string(),
    })
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Job {
    Prepare,
    Client,
    Server,
}

pub enum Msg {
    Releases(Result<Vec<Release>, String>),
    Links(Links),
    Progress(String, f32),
    Installed(String, Result<(), String>),
    /// A version's files are gone (or could not be deleted).
    Removed(String, Result<(), String>),
    /// `prune_old_versions` ended; these versions were deleted.
    Pruned(Vec<String>),
    /// What each installed version takes on disk (`disk_use`).
    DiskUse(Vec<(String, u64)>),
    Line(Job, String),
    Exit(Job, Result<(), String>),
    /// Boxed: two releases, the largest message by far.
    Updates(Result<Box<Updates>, String>),
    LauncherProgress(f32),
    LauncherUpdated(Result<(), String>),
    /// `reuse_prepared_everywhere` prepared these versions.
    Reused(Vec<(String, String)>),
    /// None when secrets are kept in the OS keychain, else what to say: they last only until
    /// the launcher closes, and why.
    Keychain(Option<String>),
    /// Whether an account in the launcher owned the game at its last check (the owner gate).
    Owner(bool),
    /// Credential-free result from a background account operation.
    AccountStatus(String, u64, crate::account::session::Status),
    AccountProgress(String, u64, crate::account::session::Phase),
    AccountHead(String, Option<String>, Option<Vec<[u8; 4]>>, bool),
    AccountRemoved(String, u64, Result<bool, String>),
    /// The owner gate's answer for the game about to start: the selected account's session
    /// file (none = offline) and why it plays offline, if it does.
    Session(Result<(Option<PathBuf>, Option<String>), crate::account::NoOwner>),
    /// A modpack step's line (`modpacks`).
    PackLine(String),
    /// A modpack step ended: the pack as it now is.
    PackDone(Result<Box<crate::modpacks::Imported>, String>),
    /// How many play Horizon now, or none when the server can't be reached (`telemetry`).
    Playing(Option<u64>),
    /// How many play on the official server now, or none when it doesn't answer its status ping
    /// (`telemetry::watch_official`).
    OfficialPlayers(Option<u64>),
    /// Where the native-mods build is (`native_mods::Step::label`).
    NativeProgress(String),
    /// The native-mods build ended: the version built, or why not.
    Native(Result<String, String>),
    /// A Java instance's game is ready to start (`java_game`, wrapped): the instance's id and
    /// game folder, and the command or why not.
    /// A Java instance's game ready to start, and its variables for the post-exit command.
    JavaGame(
        String,
        PathBuf,
        Result<(Command, crate::launch::Vars), String>,
    ),
}

/// A running process; its output arrives as `Msg::Line`, its end as `Msg::Exit`.
pub struct Running {
    pub child: Arc<Mutex<Child>>,
    pub stdin: Option<ChildStdin>,
}

impl Running {
    pub fn send(&mut self, line: &str) {
        if let Some(stdin) = &mut self.stdin {
            let _ = writeln!(stdin, "{line}");
            let _ = stdin.flush();
        }
    }
    pub fn kill(&self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

pub fn spawn(job: Job, mut cmd: Command, tx: Sender<Msg>) -> Result<Running, String> {
    crate::process::hide_console(&mut cmd);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start {:?}: {e}", cmd.get_program()))?;
    let stdin = child.stdin.take();
    let mut readers = Vec::new();
    let out = child
        .stdout
        .take()
        .map(|o| Box::new(o) as Box<dyn Read + Send>);
    let err = child
        .stderr
        .take()
        .map(|o| Box::new(o) as Box<dyn Read + Send>);
    for stream in [out, err].into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let _ = tx.send(Msg::Line(job, line));
            }
        }));
    }
    let child = Arc::new(Mutex::new(child));
    let waited = child.clone();
    std::thread::spawn(move || {
        for r in readers {
            let _ = r.join();
        }
        let status = loop {
            if let Some(s) = waited.lock().unwrap().try_wait().transpose() {
                break s;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let result = match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("Exited with {s}")),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(Msg::Exit(job, result));
    });
    Ok(Running { child, stdin })
}

/// The launcher's appearance, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    /// Follow the system's light or dark.
    System,
}

/// What the launcher's window does while the game runs, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhilePlaying {
    /// The window stays where it is.
    #[default]
    Stay,
    /// The window goes to the Dock or the taskbar, and comes back when the game closes.
    Minimize,
    /// The launcher hides (macOS only: elsewhere it minimises), and comes back when the game
    /// closes.
    Hide,
}

impl WhilePlaying {
    pub const ALL: [WhilePlaying; 3] = [Self::Stay, Self::Minimize, Self::Hide];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stay => "stay",
            Self::Minimize => "minimize",
            Self::Hide => "hide",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let name = match name.as_str() {
            "minimise" => "minimize",
            "keep" | "open" | "off" => "stay",
            n => n,
        };
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// What this computer does for it: only macOS can hide a whole app.
    pub fn here(self) -> Self {
        match self {
            Self::Hide if !cfg!(target_os = "macos") => Self::Minimize,
            w => w,
        }
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub selected: Option<String>,
    /// The instance Play starts (`instances`), by id. Unset means Horizon, the game's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub minecraft_dir: Option<PathBuf>,
    pub server_dir: Option<PathBuf>,
    /// Unset means on: only a player who turned it off has `false` here. 0.1.1 wrote its
    /// `auto_install` (off by default) for everyone, so this is a new key and that one is ignored.
    #[serde(default)]
    pub install_new_versions: Option<bool>,
    /// Unset means on: after an install the launcher deletes the release versions nothing needs
    /// (`prune_old_versions`). Only a player who turned it off has `false` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_old_versions: Option<bool>,
    /// How many of the newest release versions pruning keeps. Unset means [`KEEP_NEWEST`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_versions: Option<usize>,
    /// Off by default, so the opening animation shows unless the player turns it off.
    #[serde(default)]
    pub skip_opening: bool,
    /// Linux: whether the launcher keeps its entry in the applications menu (`desktop`). Unset
    /// means yes; removing the entry stores `false`, so it doesn't come back on the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<bool>,
    /// Extra arguments for the game's command line, as one line (see [`split_args`]). Empty by
    /// default. Appended after the launcher's own arguments, so they can add to or override them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game_args: String,
    /// The HTTP proxy every download goes through (`proxy`), such as `http://127.0.0.1:7890`.
    /// Unset means none: the environment's `HTTPS_PROXY` and `HTTP_PROXY` apply instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Dark, Light, or the system's: the launcher's look. Dark by default.
    #[serde(default)]
    pub appearance: Appearance,
    /// The player has read what the "I'm playing" ping holds (`telemetry::PING`): on the
    /// first-run question, or on the Play page once if they answered that before it existed.
    #[serde(default)]
    pub ping_notice_seen: bool,
    /// The player without an account that owns the game has read, once, why the launcher needs
    /// one now (the owner gate's screen).
    #[serde(default)]
    pub owner_notice_seen: bool,
    /// Seconds a partner banner (the sidebar's card or the Servers page's banner) has been on
    /// screen, over every start, up to `partner::SEEN_BEFORE_HIDE`: only then can the player hide it.
    #[serde(default)]
    pub partner_seen_secs: u64,
    /// The player hid a partner banner: the sidebar's card and the Servers page's banner are both gone for good.
    #[serde(default)]
    pub partner_hidden: bool,
    /// What the window does while the game runs. Stays by default.
    #[serde(default)]
    pub while_playing: WhilePlaying,
    /// The accent colour on top of the appearance (`themes::Accent`). Horizon's violet by default.
    #[serde(default)]
    pub accent: crate::themes::Accent,
    /// Home's background: a picture the player picked, kept in the data folder under this name
    /// (`themes::keep_background`). Unset shows Horizon's landscape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The window's language. Unset follows the system's (`i18n::Language::system`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::i18n::Language>,
    /// The player has read the notice before the first native mod (`native_status::NOTICE`).
    /// Native mods are always on: an older file's `native_mods` switch is ignored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_mods_noticed: bool,
    /// Native builds compile with the release profile (`native_mods::Profile`) instead of `fast`:
    /// slower to build, slightly faster game.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_release: bool,
    /// Developer options (hidden: seven clicks on the sidebar's launcher version reveal them):
    /// offline names play without an account that owns the game, in the window only
    /// (`account::Accounts::play_as_with`, `account::launch_for_game`). Never a download: the game still
    /// comes from the player's own install. Off by default, and absent from the file while off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev_offline: bool,
}

impl Settings {
    pub fn load() -> Self {
        Self::parse(&fs::read_to_string(data_dir().join("settings.json")).unwrap_or_default())
    }
    pub fn parse(text: &str) -> Self {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Self::default();
        };
        if let Some(fields) = value.as_object_mut() {
            // An appearance from a newer launcher is Dark, not a reason to lose every setting.
            if fields
                .get("appearance")
                .is_some_and(|a| serde_json::from_value::<Appearance>(a.clone()).is_err())
            {
                fields.remove("appearance");
            }
            if fields
                .get("while_playing")
                .is_some_and(|w| serde_json::from_value::<WhilePlaying>(w.clone()).is_err())
            {
                fields.remove("while_playing");
            }
            if fields.get("accent").is_some_and(|a| {
                serde_json::from_value::<crate::themes::Accent>(a.clone()).is_err()
            }) {
                fields.remove("accent");
            }
            // A language this launcher doesn't have yet follows the system's.
            if fields.get("language").is_some_and(|l| {
                serde_json::from_value::<Option<crate::i18n::Language>>(l.clone()).is_err()
            }) {
                fields.remove("language");
            }
        }
        serde_json::from_value(value).unwrap_or_default()
    }
    /// Whether the launcher keeps its entry in the applications menu (Linux).
    pub fn desktop_entry(&self) -> bool {
        self.desktop_entry.unwrap_or(true)
    }
    /// The window's language: the one picked, else the system's.
    pub fn language(&self) -> crate::i18n::Language {
        self.language.unwrap_or_else(crate::i18n::Language::system)
    }
    /// Whether new Horizon versions are installed automatically.
    pub fn auto_install(&self) -> bool {
        self.install_new_versions.unwrap_or(true)
    }
    /// Whether old versions are deleted after an install.
    pub fn auto_prune(&self) -> bool {
        self.prune_old_versions.unwrap_or(true)
    }
    /// How many of the newest release versions pruning keeps: at least the newest.
    pub fn keep_newest(&self) -> usize {
        self.keep_versions.unwrap_or(KEEP_NEWEST).max(1)
    }
    pub fn save(&self) {
        // A write that stops halfway leaves settings.json unreadable, and `load` answers
        // defaults for it: the file is replaced only once it is whole.
        let dir = data_dir();
        let path = dir.join("settings.json");
        let tmp = dir.join(format!("settings.{}.tmp", std::process::id()));
        let written = fs::create_dir_all(&dir)
            .and_then(|()| fs::write(&tmp, serde_json::to_string_pretty(self).unwrap()))
            .and_then(|()| fs::rename(&tmp, &path));
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            eprintln!("Horizon: settings.json is not saved: {e}");
        }
    }
}

#[cfg(test)]
#[path = "backend/release_loading_tests.rs"]
mod release_loading_tests;

#[cfg(test)]
#[path = "backend/java_tests.rs"]
mod java_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    /// A canonical path loses Windows' verbatim prefix on a drive (horizon-mixins' output
    /// check stats `\\?\C:`, the volume itself, which Windows refuses, os error 1), and keeps
    /// it where there is no drive form.
    #[test]
    fn a_canonical_windows_path_loses_only_a_drives_verbatim_prefix() {
        let plain = |p: &str| plain(PathBuf::from(p));
        assert_eq!(
            plain(r"\\?\C:\Users\me\AppData\Roaming\horizon-launcher\native"),
            PathBuf::from(r"C:\Users\me\AppData\Roaming\horizon-launcher\native")
        );
        // A share has no drive form to fall back to; other paths have no prefix.
        assert_eq!(
            plain(r"\\?\UNC\server\share\native"),
            PathBuf::from(r"\\?\UNC\server\share\native")
        );
        assert_eq!(
            plain("/home/me/.local/share/horizon-launcher/native"),
            PathBuf::from("/home/me/.local/share/horizon-launcher/native")
        );
    }

    /// [#5189](https://github.com/horizon-rust/horizon/issues/5189): MinGW-w64's tools can't
    /// read a non-ASCII path, so the build root is the data folder's resolved ASCII spelling —
    /// which a link's target or a short (8.3) name already is.
    #[test]
    fn the_build_dir_is_a_path_the_tools_can_read() {
        assert!(ascii(Path::new(
            r"C:\Users\me\AppData\Roaming\horizon-launcher"
        )));
        assert!(!ascii(Path::new(
            r"C:\Users\Кириченко Павло\AppData\Roaming\horizon-launcher"
        )));
        assert!(!ascii(Path::new("/høme/me/horizon-launcher")));
        let built = build_dir();
        assert!(ascii(&built) || cfg!(windows), "{}", built.display());
        // Resolved: the build never sees a link in the path (where the answer is the data
        // folder itself, so already resolved — Windows' other spellings aren't its own).
        if ascii(&built) {
            assert_eq!(
                built,
                built
                    .canonicalize()
                    .map(plain)
                    .unwrap_or_else(|_| built.clone())
            );
        }
    }

    /// [#6009](https://github.com/horizon-rust/horizon/issues/6009): with a Cyrillic user name the
    /// build folder is a real ASCII folder of ProgramData's (not a short name of the data folder,
    /// which rustc resolves back to the Cyrillic one), its own for each data folder.
    #[test]
    fn a_cyrillic_data_folder_builds_in_an_ascii_folder_of_its_own() {
        let pavlo = Path::new(r"C:\Users\Кириченко Павло\AppData\Roaming\horizon-launcher");
        let other = Path::new(r"C:\Users\Олена\AppData\Roaming\horizon-launcher");
        let public = PathBuf::from(if cfg!(windows) { r"D:\Data" } else { "/data" });
        let dir = ascii_build_dir(pavlo, Some(public.clone()));
        assert!(ascii(&dir), "{}", dir.display());
        assert_eq!(dir.parent(), Some(public.as_path()));
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("horizon-launcher-") && name.len() == 25,
            "{name}"
        );
        assert_eq!(dir, ascii_build_dir(pavlo, Some(public.clone())), "stable");
        assert_ne!(dir, ascii_build_dir(other, Some(public)), "one per user");
        // %ProgramData% unset, relative or itself not ASCII: C:\ProgramData.
        let default = PathBuf::from(r"C:\ProgramData");
        for pd in [None, Some("ProgramData".into()), Some(r"C:\Дані".into())] {
            let dir = ascii_build_dir(pavlo, pd);
            assert_eq!(dir.parent(), Some(default.as_path()));
            assert!(ascii(&dir));
        }
    }

    /// The game program's mod API level is its mark's; a program from before the mark (0.2.68
    /// and older) or none at all serves level 0.
    #[test]
    fn a_game_program_says_its_mod_api_level() {
        let dir = std::env::temp_dir().join(format!("horizon-api-level-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let marked = dir.join("marked");
        let mut program = b"\x7fELF code code".to_vec();
        program.extend_from_slice(b"horizon-mod-api-level:0007;");
        fs::write(&marked, &program).unwrap();
        let old = dir.join("old");
        fs::write(&old, b"\x7fELF code without a mark").unwrap();
        assert_eq!(program_mod_api_level(&marked), 7);
        assert_eq!(program_mod_api_level(&old), 0);
        assert_eq!(program_mod_api_level(&dir.join("missing")), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The launcher downloads Mojang's game files (client jar, libraries, assets) only for an
    /// account the services said owns the game: never without one, and never past the gate by
    /// the developer option (`Settings::dev_offline`), whose pass has no owner.
    #[test]
    fn mojang_downloads_only_for_an_owner() {
        let dir = std::env::temp_dir().join(format!("horizon-owner-only-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pins = sources::Pins::read(&serde_json::json!({ "minecraft": {
            "version": "26.3",
            "client_sha1": "00",
            "manifest_sha1": "00",
        }}))
        .unwrap();
        let asked = std::sync::Mutex::new(Vec::<String>::new());
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            asked.lock().unwrap().push(url.to_string());
            Err("no network in tests".into())
        };
        let log = |_: String| {};
        let no_owner = || Ok(account::Launch::default());
        for (pass, signed_in) in [
            (Err(account::NoOwner), false),
            (Err(account::NoOwner), true),
            (no_owner(), false),
            (no_owner(), true),
        ] {
            assert!(download_for(&pins, &dir, pass, signed_in, &fetch, &log).is_err());
        }
        assert!(asked.lock().unwrap().is_empty());
        assert!(!dir.exists());
        // An owner: Mojang is asked (and the download fails here, without network).
        let owner = account::Account {
            uuid: "069a79f444e94726a5befca90e38aaf5".into(),
            name: "Notch".into(),
            skin: None,
            cape: None,
        };
        let secrets = serde_json::from_str(
            r#"{"refresh_token":"r","minecraft_token":"t","minecraft_expires_at":1,"owns":true}"#,
        )
        .unwrap();
        let pass = Ok(account::Launch {
            owner: Some((owner, secrets)),
            ..Default::default()
        });
        assert!(download_for(&pins, &dir, pass, true, &fetch, &log).is_err());
        assert!(!asked.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);

        // A store that lacks assets is not filled for a non-owner either: nothing is asked.
        fs::create_dir_all(dir.join("assets/indexes")).unwrap();
        fs::write(
            dir.join("assets/indexes/34.json"),
            r#"{"objects":{"a":{"hash":"00","size":1}}}"#,
        )
        .unwrap();
        asked.lock().unwrap().clear();
        for (pass, signed_in) in [(Err(account::NoOwner), true), (no_owner(), true)] {
            assert!(download_for(&pins, &dir, pass, signed_in, &fetch, &log).is_err());
        }
        assert!(asked.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);

        // In the sources: one call downloads the game, in `download_for`, and the only caller
        // of that asks the strict gate. Game-file hosts appear nowhere else.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![src.clone()];
        let mut sources = Vec::new();
        while let Some(path) = files.pop() {
            if path.is_dir() {
                files.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                let in_tests = name.contains("tests") || path.parent().unwrap() != src;
                // The code alone: test modules (this one included) come after it.
                // A Windows checkout has CRLF line ends: the searches below look for "\n".
                let mut text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
                text.truncate(text.find("#[cfg(test)]").unwrap_or(text.len()));
                sources.push((name, in_tests, text));
            }
        }
        let calls: Vec<_> = sources
            .iter()
            .filter(|(_, tests, _)| !tests)
            .flat_map(|(name, _, text)| {
                text.matches("mojang::download(")
                    .map(move |_| name.as_str())
            })
            .collect();
        assert_eq!(calls, ["backend.rs"]);
        let backend = &sources.iter().find(|(n, ..)| n == "backend.rs").unwrap().2;
        let body = |f: &str| {
            let start = backend.find(&format!("\nfn {f}(")).unwrap();
            &backend[start..start + backend[start + 1..].find("\n}\n").unwrap()]
        };
        assert!(body("download_for").contains("mojang::download("));
        assert!(body("download_for_owner").contains("account::launch("));
        assert!(!body("download_for_owner").contains("launch_for_game("));
        // `download_for` is defined once and called once, by `download_for_owner`.
        assert_eq!(backend.matches("download_for(").count(), 2);
        assert!(body("download_for_owner").contains("download_for("));
        // Refilling an incomplete store asks the same gate and names no host or download itself.
        assert!(body("refill_own_store").contains("download_for_owner("));
        assert!(!body("refill_own_store").contains("mojang::"));
        // It runs at start-up, never on Play, which also runs `complete_assets`.
        let public = |f: &str| {
            let start = backend
                .find(&format!(
                    "
pub fn {f}("
                ))
                .unwrap();
            &backend[start
                ..start
                    + backend[start + 1..]
                        .find(
                            "
}
",
                        )
                        .unwrap()]
        };
        assert!(public("reuse_prepared_everywhere").contains("refill_own_store("));
        assert!(!public("complete_assets").contains("refill_own_store("));
        for host in [
            "piston-data",
            "libraries.minecraft.net",
            "resources.download.minecraft.net",
        ] {
            for (name, tests, text) in &sources {
                assert!(
                    *tests || name == "mojang.rs" || !text.contains(host),
                    "{name} names {host}"
                );
            }
        }
    }

    /// A preparation that reuses the imported game files still reads the assets from the
    /// launcher that has them: a Prism Launcher instance's are in the launcher root, linked into
    /// the official layout (jan.sonki on Discord: silent, grey title screen since 0.2.48).
    #[test]
    fn reused_imports_take_the_assets_from_the_launcher_that_has_them() {
        let dir =
            std::env::temp_dir().join(format!("horizon-reused-assets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let client = b"fake client jar";
        let manifest = br#"{"id": "26.3"}"#;
        let write = |path: &str, data: &[u8]| {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        let prism = "PrismLauncher";
        write(
            &format!("{prism}/libraries/com/mojang/minecraft/26.3/minecraft-26.3-client.jar"),
            client,
        );
        write(&format!("{prism}/assets/indexes/34.json"), b"{}");
        let instance = dir.join(prism).join("instances/26.3/minecraft");
        fs::create_dir_all(&instance).unwrap();
        let pins = sources::Pins::read(&serde_json::json!({ "minecraft": {
            "version": "26.3",
            "client_sha1": sources::sha1_hex(client),
            "manifest_sha1": sources::sha1_hex(manifest),
        }}))
        .unwrap();
        let linked = dir.join("linked");
        let fetch = |_: &str| Ok(manifest.to_vec());
        let lines = std::cell::RefCell::new(Vec::new());
        let log = |line: String| lines.borrow_mut().push(line);
        let resolved = sources::resolve(&instance, &pins, &[], &linked, &fetch, &log);
        let folder = assets_folder(resolved, &instance, "26.3", &log);
        assert_eq!(folder, linked);
        assert!(folder.join("assets/indexes/34.json").is_file());

        // Nothing found: the chosen folder, and a line saying what the game will lack.
        let empty = dir.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let resolved = sources::resolve(&empty, &pins, &[], &linked, &fetch, &log);
        assert_eq!(assets_folder(resolved, &empty, "26.3", &log), empty);
        assert!(lines.borrow().last().unwrap().contains("will be silent"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resources_without_sounds_are_reported() {
        let dir = std::env::temp_dir().join(format!("horizon-no-sounds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets/minecraft/textures")).unwrap();
        fs::write(dir.join(RESOURCES_MANIFEST), "{}").unwrap();
        assert!(!sounds_in(&dir));
        fs::write(dir.join("assets/minecraft/sounds.json"), "{}").unwrap();
        assert!(sounds_in(&dir));
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            resources_summary(true, true, "26.3"),
            "Prepared for Minecraft 26.3"
        );
        let silent = resources_summary(true, false, "26.3");
        assert!(silent.contains("without its sounds") && silent.contains("Prepare again"));
        // Not prepared says only that, sounds or not.
        assert_eq!(
            resources_summary(false, false, "26.3"),
            resources_summary(false, true, "26.3")
        );
    }

    #[test]
    fn each_mac_picks_the_build_for_its_processor() {
        assert_eq!(
            platform_assets("macos", "aarch64"),
            Some((
                "horizon-macos-arm64.zip",
                "horizon-launcher-macos-arm64.zip"
            ))
        );
        assert_eq!(
            platform_assets("macos", "x86_64"),
            Some(("horizon-macos-x64.zip", "horizon-launcher-macos-x64.zip"))
        );
        assert_eq!(
            platform_assets("windows", "x86_64").map(|(game, _)| game),
            Some("horizon-windows-x64.zip")
        );
        assert_eq!(platform_assets("linux", "aarch64"), None);
        assert_eq!(platform_assets("macos", "x86"), None);
        assert_eq!(
            ASSET,
            platform_assets(std::env::consts::OS, std::env::consts::ARCH).map(|(game, _)| game)
        );
    }

    /// Every asset a launcher looks for is one the release workflows build, sign and publish.
    #[test]
    fn platform_assets_are_published() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let read = |path: &str| fs::read_to_string(repo.join(path)).unwrap();
        let build = read(".github/workflows/game-build.yml");
        let nightly = read(".github/workflows/nightly-build.yml");
        let launcher = read(".github/workflows/launcher.yml");
        for (os, arch, game, launcher_asset) in PLATFORMS {
            assert!(
                build.contains(&format!("asset: {game}")),
                "game-build.yml builds no {game}"
            );
            // Signed (SHA256SUMS), then published.
            assert_eq!(
                nightly.matches(&format!("dist/{game}")).count(),
                2,
                "nightly-build.yml must sign and publish {game} ({os} {arch})"
            );
            assert!(
                launcher.contains(&format!("asset: {launcher_asset}")),
                "launcher.yml builds no {launcher_asset}"
            );
        }
    }

    #[test]
    fn the_game_plays_under_its_own_audio_stream_name() {
        if std::env::var_os("PIPEWIRE_PROPS").is_some() {
            return;
        }
        let cmd = client_command("v2026.10.13");
        let props = cmd
            .get_envs()
            .find(|(k, _)| *k == "PIPEWIRE_PROPS")
            .and_then(|(_, v)| v);
        assert!(props.is_some_and(|v| v
            .to_string_lossy()
            .contains("application.name = \"Horizon\"")));
    }

    #[test]
    fn a_crash_after_the_vulkan_adapter_line_and_before_the_first_frame_is_a_start_crash() {
        let start = [
            "Horizon: resources loaded in 11.3s; 35723 block states, 1658 items, 67 biomes",
            "Horizon renderer: Intel(R) HD Graphics 530 (Vulkan, IntegratedGpu)",
        ];
        assert!(vulkan_start_crashed(start));
        // A step line after the adapter keeps it a start crash.
        let step = [start[1], "Horizon renderer: creating the pipelines"];
        assert!(vulkan_start_crashed(step));
        // A crash after the first frame is the game's, not the renderer start's.
        let later = [start[1], "Horizon: first frame after 812 ms"];
        assert!(!vulkan_start_crashed(later));
        // Already on DirectX 12, or no renderer yet: nothing to switch.
        let dx12 = ["Horizon renderer: Intel(R) HD Graphics 530 (Dx12, IntegratedGpu)"];
        assert!(!vulkan_start_crashed(dx12));
        assert!(!vulkan_start_crashed(&start[..1]));
        // Vulkan failed to open and DirectX 12 took over: its line is the last adapter.
        assert!(!vulkan_start_crashed([start[1], dx12[0]]));
    }

    #[test]
    fn a_renderer_start_crash_switches_auto_to_dx12_once() {
        let home = std::env::temp_dir().join(format!("horizon-dx12-switch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        // No settings file yet: one with the API alone.
        assert_eq!(switch_to_dx12(&home), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "graphicsApi:dx12\n"
        );
        // Already DirectX 12, or the player's own Vulkan: left alone.
        assert_eq!(switch_to_dx12(&home), Ok(false));
        fs::write(home.join(GRAPHICS_FILE), "sky:false\ngraphicsApi:vulkan\n").unwrap();
        assert_eq!(switch_to_dx12(&home), Ok(false));
        // Auto becomes DirectX 12 and the other settings stay.
        fs::write(
            home.join(GRAPHICS_FILE),
            "sky:false\ngraphicsApi:auto\nfog:true\n",
        )
        .unwrap();
        assert_eq!(switch_to_dx12(&home), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "sky:false\ngraphicsApi:dx12\nfog:true\n"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn extra_game_arguments_split_like_a_shell() {
        assert!(split_args("").is_empty());
        assert!(split_args("   ").is_empty());
        assert_eq!(split_args("--server-thread"), ["--server-thread"]);
        assert_eq!(
            split_args("--render-distance 32 --server-thread"),
            ["--render-distance", "32", "--server-thread"]
        );
        // Quotes keep a word with spaces together; either kind works.
        assert_eq!(
            split_args("--name 'Two Words' --world \"My World\""),
            ["--name", "Two Words", "--world", "My World"]
        );
        // An empty quoted word is still a word.
        assert_eq!(split_args("--name \"\""), ["--name", ""]);
        // A backslash escapes a space outside quotes, and " and \ inside double quotes.
        assert_eq!(split_args("--name My\\ World"), ["--name", "My World"]);
        assert_eq!(split_args("a\\tb c"), ["atb", "c"]);
        assert_eq!(split_args("\"a\\\"b\" 'c\\d'"), ["a\"b", "c\\d"]);
    }

    #[test]
    fn the_game_arguments_setting_round_trips_and_reaches_the_command() {
        let settings = Settings::parse(r#"{"game_args":"--server-thread"}"#);
        assert_eq!(settings.game_args, "--server-thread");
        // Unset in an older settings.json: no arguments, and nothing is written for them.
        let old = Settings::parse(r#"{"installed":true}"#);
        assert_eq!(old.game_args, "");
        assert!(!serde_json::to_string(&old).unwrap().contains("game_args"));

        let mut cmd = Command::new("true");
        cmd.args(split_args("--server-thread --render-distance 40"));
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--server-thread", "--render-distance", "40"]);
    }

    #[test]
    fn tags_never_mix() {
        assert!(is_game_tag("v2026.10.13"));
        assert!(!is_game_tag("launcher-v0.2.0"));
        assert!(!is_game_tag("vnext"));
        assert_eq!(launcher_version("launcher-v0.2.0"), Some("0.2.0"));
        assert_eq!(launcher_version("launcher@0.2.0"), Some("0.2.0"));
        assert!(!is_game_tag("launcher@0.2.0"));
        assert!(is_game_tag("v0.2.0"));
        assert!(is_launcher_tag("launcher@0.2.0"));
        assert_eq!(launcher_version("v2026.10.13"), None);
        assert_eq!(launcher_version("launcher-vx"), None);
        let list = serde_json::json!([
            {"tag_name": "v2026.10.12", "assets": [{"name": "horizon-linux-x86_64.tar.gz", "browser_download_url": "u", "size": 1}]},
            {"tag_name": "launcher-v9.0.0", "assets": [{"name": "horizon-launcher-linux-x86_64.tar.gz", "browser_download_url": "u", "size": 1}]},
            {"tag_name": "v2026.10.13", "assets": []},
            {"tag_name": "launcher-v0.0.1", "draft": true, "assets": []},
        ]);
        let game: Vec<_> = parse_releases(&list, ASSET, is_game_tag)
            .into_iter()
            .map(|r| r.tag)
            .collect();
        assert_eq!(game, ["v2026.10.13", "v2026.10.12"]);
        let launcher: Vec<_> = parse_releases(&list, LAUNCHER_ASSET, is_launcher_tag)
            .into_iter()
            .map(|r| r.tag)
            .collect();
        assert_eq!(launcher, ["launcher-v9.0.0"]);
    }

    /// Rolling a release back (`gh release edit <tag> --draft`, docs/release-flow.md) makes the
    /// previous one the newest again: it is what `install latest` and the update check offer.
    #[test]
    fn withdrawn_release_is_never_offered() {
        let asset = ASSET.unwrap_or("horizon-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v2026.10.15.1", "draft": true, "assets": [{"name": asset, "browser_download_url": "bad", "size": 1}]},
            {"tag_name": "v2026.10.15", "assets": [{"name": asset, "browser_download_url": "good", "size": 1}]},
            {"tag_name": "v2026.10.14", "assets": [{"name": asset, "browser_download_url": "old", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        assert!(releases.iter().all(|r| r.tag != "v2026.10.15.1"));
        assert_eq!(
            latest(&releases).map(|r| r.tag.as_str()),
            Some("v2026.10.15")
        );
        let offered = |installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
            horizon_update(&releases, &installed).map(|r| r.tag)
        };
        assert_eq!(offered(&["v2026.10.14"]), Some("v2026.10.15".into()));
        assert_eq!(offered(&[]), Some("v2026.10.15".into()));
        // Who installed the withdrawn one is not told to "update" to an older version.
        assert_eq!(offered(&["v2026.10.15.1", "v2026.10.14"]), None);
    }

    /// Every release is published as the release, and launchers update to it. The pre-releases
    /// of the old Alpha channel (v0.2.1 on) are only installed by hand: a launcher that
    /// had one installed waits for the next release instead of going back.
    #[test]
    fn updates_follow_every_release_and_leave_the_old_alpha_ones_out() {
        let asset = ASSET.unwrap_or("horizon-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v0.2.13", "assets": [{"name": asset, "browser_download_url": "n", "size": 1}]},
            {"tag_name": "v0.2.12", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "a", "size": 1}]},
            {"tag_name": "v0.2.0", "assets": [{"name": asset, "browser_download_url": "s", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        assert_eq!(
            releases.iter().map(|r| r.prerelease).collect::<Vec<_>>(),
            [false, true, false]
        );
        let offered = |releases: &[Release], installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
            horizon_update(&installable_updates(releases), &installed).map(|r| r.tag)
        };
        assert_eq!(offered(&releases, &[]), Some("v0.2.13".into()));
        assert_eq!(offered(&releases, &["v0.2.0"]), Some("v0.2.13".into()));
        assert_eq!(offered(&releases, &["v0.2.12"]), Some("v0.2.13".into()));
        assert_eq!(offered(&releases, &["v0.2.13"]), None);
        // Before the first release of the single channel: the old Alpha one stays where it is.
        let before = &releases[1..];
        assert_eq!(offered(before, &["v0.2.0"]), None);
        assert_eq!(offered(before, &["v0.2.12", "v0.2.0"]), None);
        assert_eq!(CHECK_EVERY.as_secs(), 30 * 60);
    }

    #[test]
    fn the_window_stays_while_playing_unless_picked() {
        assert_eq!(Settings::parse("{}").while_playing, WhilePlaying::Stay);
        let hide = Settings::parse(r#"{"while_playing": "hide", "game_args": "--x"}"#);
        assert_eq!(hide.while_playing, WhilePlaying::Hide);
        let future = Settings::parse(r#"{"while_playing": "fade", "game_args": "--x"}"#);
        assert_eq!(
            (future.while_playing, future.game_args.as_str()),
            (WhilePlaying::Stay, "--x")
        );
        assert_eq!(
            WhilePlaying::parse("Minimise"),
            Some(WhilePlaying::Minimize)
        );
        assert_eq!(WhilePlaying::parse("off"), Some(WhilePlaying::Stay));
        assert_eq!(WhilePlaying::parse("close"), None);
        for w in WhilePlaying::ALL {
            let text = serde_json::to_string(&w).unwrap();
            assert_eq!(text, format!("\"{}\"", w.name()));
        }
        let here = WhilePlaying::Hide.here();
        assert_eq!(here == WhilePlaying::Hide, cfg!(target_os = "macos"));
    }

    #[test]
    fn the_language_follows_the_system_unless_picked() {
        use crate::i18n::Language;
        assert_eq!(Settings::parse("{}").language, None);
        let fr = Settings::parse(r#"{"language": "fr"}"#);
        assert_eq!(fr.language(), Language::French);
        let future = Settings::parse(r#"{"language": "eo", "game_args": "--x"}"#);
        assert_eq!((future.language, future.game_args.as_str()), (None, "--x"));
        let saved = serde_json::to_value(Settings::default()).unwrap();
        assert!(saved.get("language").is_none(), "unset is left out");
    }

    #[test]
    fn the_accent_is_violet_unless_picked_and_an_unknown_one_keeps_the_rest() {
        use crate::themes::Accent;
        assert_eq!(Settings::parse("{}").accent, Accent::VIOLET);
        let teal =
            Settings::parse(r#"{"accent": "teal", "background": "background-0123456789ab.png"}"#);
        assert_eq!(
            (teal.accent, teal.background.as_deref()),
            (Accent::TEAL, Some("background-0123456789ab.png"))
        );
        let future = Settings::parse(r#"{"accent": "plaid", "game_args": "--x"}"#);
        assert_eq!(
            (future.accent, future.game_args.as_str()),
            (Accent::VIOLET, "--x")
        );
    }

    #[test]
    fn the_appearance_is_dark_unless_picked_and_an_unknown_one_keeps_the_rest() {
        assert_eq!(Settings::parse("{}").appearance, Appearance::Dark);
        let light = Settings::parse(r#"{"appearance": "light", "game_args": "--x"}"#);
        assert_eq!(light.appearance, Appearance::Light);
        let future = Settings::parse(r#"{"appearance": "sepia", "game_args": "--x"}"#);
        assert_eq!(
            (future.appearance, future.game_args.as_str()),
            (Appearance::Dark, "--x")
        );
        let settings = Settings {
            appearance: Appearance::System,
            ..Settings::default()
        };
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains(r#""appearance":"system""#), "{text}");
    }

    #[test]
    fn the_old_update_channel_settings_are_dropped_and_the_rest_kept() {
        for old in [
            r#"{"update_channel": "alpha", "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"update_channel": "stable", "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"beta_builds": true, "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"beta_builds": false, "update_channel": "nightly", "game_args": "--x", "selected": "v0.2.0"}"#,
        ] {
            let settings = Settings::parse(old);
            assert_eq!(settings.game_args, "--x", "{old}");
            assert_eq!(settings.selected.as_deref(), Some("v0.2.0"), "{old}");
            let saved = serde_json::to_value(&settings).unwrap();
            assert!(saved.get("update_channel").is_none(), "{old}");
            assert!(saved.get("beta_builds").is_none(), "{old}");
        }
    }

    #[test]
    fn the_version_is_the_one_the_release_workflow_injects() {
        assert_eq!(version_or(Some("0.4.2")), "0.4.2");
        assert_eq!(version_or(Some("")), env!("CARGO_PKG_VERSION"));
        assert_eq!(version_or(None), env!("CARGO_PKG_VERSION"));
        assert_eq!(
            VERSION,
            option_env!("HORIZON_LAUNCHER_VERSION")
                .filter(|v| !v.is_empty())
                .unwrap_or(env!("CARGO_PKG_VERSION"))
        );
        // Self-update compares releases with that version: this launcher's own release is no update.
        let r = |tag: String| Release {
            tag,
            name: String::new(),
            body: String::new(),
            date: String::new(),
            asset: Some(("u".into(), 1)),
            prerelease: false,
            nightly: None,
        };
        assert!(launcher_update(&[r(own_tag())]).is_none());
        assert_eq!(launcher_version(&own_tag()), Some(VERSION));
        assert_eq!(
            parse_version_output(&format!("horizon-launcher {VERSION}\n")),
            Some(VERSION)
        );
    }

    #[test]
    fn a_launcher_knows_which_line_it_was_released_on() {
        // launcher.yml gives the tag; before it did, a release had only its `launcher-v` version.
        assert_eq!(
            own_tag_of(Some("launcher@0.2.3"), Some("0.2.3")),
            "launcher@0.2.3"
        );
        assert_eq!(
            own_tag_of(Some("launcher-v0.17.0"), Some("0.17.0")),
            "launcher-v0.17.0"
        );
        assert_eq!(own_tag_of(None, Some("0.16.0")), "launcher-v0.16.0");
        assert_eq!(own_tag_of(Some(""), Some("0.16.0")), "launcher-v0.16.0");
        // A build from source is on the 0.2 line.
        assert_eq!(
            own_tag_of(None, None),
            format!("launcher@{}", env!("CARGO_PKG_VERSION"))
        );
    }

    /// The update path to 0.2 (docs/versioning.md): launchers from before it take the last
    /// `launcher-v` release, which takes every `launcher@` one.
    #[test]
    fn launchers_from_before_0_2_update_to_it() {
        let r = |tag: &str| Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            asset: Some(("u".into(), 1)),
            prerelease: false,
            nightly: None,
        };
        let mut list = vec![
            r("launcher-v0.16.0"),
            r("launcher@0.2.0"),
            r("launcher-v0.17.0"),
            r("launcher@0.2.1"),
        ];
        list.sort_by(|a, b| compare_versions(&b.tag, &a.tag));
        let tags: Vec<_> = list.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(
            tags,
            [
                "launcher@0.2.1",
                "launcher@0.2.0",
                "launcher-v0.17.0",
                "launcher-v0.16.0"
            ]
        );
        let offered = |own: &str| launcher_update_from(&list, own).map(|r| r.tag);
        assert_eq!(offered("launcher-v0.16.0"), Some("launcher@0.2.1".into()));
        assert_eq!(offered("launcher-v0.17.0"), Some("launcher@0.2.1".into()));
        assert_eq!(offered("launcher@0.2.0"), Some("launcher@0.2.1".into()));
        assert_eq!(offered("launcher@0.2.1"), None);
        assert_eq!(offered("launcher@0.3.0-rc.1"), None);
        // The old launchers' own check only sees `launcher-v` tags: 0.17.0 is their way over.
        let old_scheme: Vec<_> = list
            .iter()
            .filter(|r| r.tag.starts_with(OLD_LAUNCHER_TAG_PREFIX))
            .cloned()
            .collect();
        assert_eq!(
            launcher_update_from(&old_scheme, "launcher-v0.16.0").map(|r| r.tag),
            Some("launcher-v0.17.0".into())
        );
    }

    /// A launcher in an AppImage updates with the AppImage's own file: the `.tar.gz` binary would
    /// be written inside the AppImage's read-only mount.
    #[test]
    fn an_appimage_updates_with_the_appimage() {
        assert_eq!(launcher_asset_running_from(false), LAUNCHER_ASSET);
        assert_eq!(launcher_asset_running_from(true), LAUNCHER_APPIMAGE_ASSET);
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(
            launcher_asset_running_from(true),
            Some("horizon-launcher-linux-x86_64.AppImage")
        );
    }

    /// `$APPIMAGE` names the file the runtime mounted; a path that is not there (moved or deleted
    /// since the launcher started) is no AppImage, and the launcher updates its own binary again.
    /// Nor is one inherited from another AppImage's program: the binary has to run from `$APPDIR`.
    // AppImages are Linux's, and the paths below are absolute only on Unix.
    #[cfg(unix)]
    #[test]
    fn only_an_appimage_file_counts() {
        use std::ffi::OsStr;
        let mount = OsStr::new("/tmp/.mount_Horizon123");
        let exe = Path::new("/tmp/.mount_Horizon123/usr/bin/horizon-launcher");
        assert_eq!(appimage_path_in(None, Some(mount), exe), None);
        assert_eq!(
            appimage_path_in(Some(OsStr::new("Horizon.AppImage")), Some(mount), exe),
            None,
            "a relative path is no $APPIMAGE"
        );
        assert_eq!(
            appimage_path_in(
                Some(OsStr::new("/nonexistent/Horizon.AppImage")),
                Some(mount),
                exe
            ),
            None
        );
        let file = std::env::temp_dir().join(format!("horizon-appimage-{}", std::process::id()));
        std::fs::write(&file, b"#!AppImage").unwrap();
        assert_eq!(
            appimage_path_in(Some(file.as_os_str()), Some(mount), exe),
            Some(file.clone())
        );
        // Inherited from another AppImage: this launcher is installed elsewhere.
        let elsewhere = Path::new("/home/a/.local/bin/horizon-launcher");
        assert_eq!(
            appimage_path_in(Some(file.as_os_str()), Some(mount), elsewhere),
            None
        );
        assert_eq!(appimage_path_in(Some(file.as_os_str()), None, exe), None);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn game_0_2_follows_every_date_release() {
        assert_eq!(compare_versions("v0.2.0", "v2026.10.19.15"), Greater);
        assert_eq!(compare_versions("v0.2.0", "v2026.10.21"), Greater);
        assert_eq!(compare_versions("v2099.12.31.9", "v0.2.0"), Less);
        assert_eq!(compare_versions("v0.2.1", "v0.2.0"), Greater);
        assert_eq!(compare_versions("v0.2.10", "v0.2.9"), Greater);
        assert_eq!(compare_versions("v0.3.0", "v0.2.57"), Greater);
        assert_eq!(compare_versions("v0.2.0", "v0.2.0"), Equal);
        // Pre-releases come before their release and after the one before.
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.3.0"), Less);
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.2.9"), Greater);
        assert_eq!(compare_versions("v0.3.0-beta.11", "v0.3.0-beta.2"), Greater);
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.3.0-beta.9"), Greater);
        assert_eq!(compare_versions("v0.3.0-beta.1", "v0.3.0-beta"), Greater);
        assert_eq!(compare_versions("v0.3.0-alpha", "v0.3.0-1"), Greater);
        assert_eq!(compare_versions("v0.2.1-beta.1", "v2026.10.21"), Greater);
        // Launcher and game tags of 0.2 compare alike; old launcher tags stay below.
        assert_eq!(
            compare_versions("launcher@0.2.0", "launcher-v0.16.0"),
            Greater
        );
        assert_eq!(compare_versions("launcher@0.2.0", "v0.2.0"), Equal);

        let asset = ASSET.unwrap_or("horizon-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v0.2.0", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "b", "size": 1}]},
            {"tag_name": "v2026.10.21", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "d", "size": 1}]},
            {"tag_name": "v2026.10.19.15", "assets": [{"name": asset, "browser_download_url": "s", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        let installed = ["v2026.10.19.15".to_string(), "v2026.10.12".to_string()];
        // A pre-release 0.2.0 is not an update; once published as the release, it is one.
        let offered = |releases: &[Release]| {
            horizon_update(&installable_updates(releases), &installed).map(|r| r.tag)
        };
        assert_eq!(offered(&releases), None);
        let mut released = releases.clone();
        released[0].prerelease = false;
        assert_eq!(offered(&released), Some("v0.2.0".into()));
        let mut sorted = vec![
            "v2026.10.19.15".to_string(),
            "v0.2.1".into(),
            "v0.2.0".into(),
            "v2026.10.21".into(),
        ];
        sorted.sort_by(|a, b| compare_versions(b, a));
        assert_eq!(
            sorted,
            ["v0.2.1", "v0.2.0", "v2026.10.21", "v2026.10.19.15"]
        );
    }

    #[test]
    fn the_fallback_looks_for_0_2_launchers_first() {
        // The asset the launcher reading this list installs with (the AppImage's own when it runs
        // from one), as launcher_releases_in_with looks for it.
        let asset = launcher_asset().unwrap_or("unsupported");
        let release = |tag: &str| serde_json::json!({"tag_name": tag, "assets": [{"name": asset, "browser_download_url": "u", "size": 1}]});
        let asked = std::cell::RefCell::new(Vec::new());
        let lookup = |with_new: bool| {
            asked.borrow_mut().clear();
            launcher_releases_in_with(&serde_json::json!([]), |url| {
                asked
                    .borrow_mut()
                    .push(url.rsplit('/').next().unwrap_or("").to_string());
                if url.ends_with("matching-refs/tags/launcher@") {
                    Ok(serde_json::json!(if with_new {
                        vec![serde_json::json!({"ref": "refs/tags/launcher@0.2.0"})]
                    } else {
                        Vec::new()
                    }))
                } else if url.ends_with("matching-refs/tags/launcher-v") {
                    Ok(serde_json::json!([{"ref": "refs/tags/launcher-v0.17.0"}]))
                } else {
                    Ok(release(url.rsplit('/').next().unwrap_or("")))
                }
            })
            .into_iter()
            .map(|r| r.tag)
            .collect::<Vec<_>>()
        };
        assert_eq!(lookup(true), ["launcher@0.2.0"]);
        assert_eq!(*asked.borrow(), ["launcher@", "launcher@0.2.0"]);
        // Before the first `launcher@` release, the `launcher-v` ones.
        assert_eq!(lookup(false), ["launcher-v0.17.0"]);
        assert_eq!(
            *asked.borrow(),
            ["launcher@", "launcher-v", "launcher-v0.17.0"]
        );
    }

    #[test]
    fn launcher_tags_from_matching_refs_newest_first() {
        let refs = serde_json::json!([
            {"ref": "refs/tags/launcher-v0.1.9"},
            {"ref": "refs/tags/launcher-v0.1.10"},
            {"ref": "refs/tags/launcher-vnext"},
            {"ref": "refs/tags/v2026.10.15"},
            {"ref": "refs/tags/launcher-v0.2.0"},
            {"ref": "refs/tags/launcher@0.2.0"},
        ]);
        assert_eq!(
            newest_launcher_tags(&refs),
            [
                "launcher@0.2.0",
                "launcher-v0.2.0",
                "launcher-v0.1.10",
                "launcher-v0.1.9"
            ]
        );
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare_versions("v2026.10.13", "v2026.10.12"), Greater);
        assert_eq!(compare_versions("v2026.9.30", "v2026.10.1"), Less);
        assert_eq!(compare_versions("v2026.10.13.1", "v2026.10.13"), Greater);
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("launcher-v1.2", "1.2.0"), Equal);
        assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), Equal);
    }

    #[test]
    fn updates() {
        let r = |tag: &str| Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            asset: Some(("u".into(), 1)),
            prerelease: false,
            nightly: None,
        };
        let releases = [r("v2026.10.13"), r("v2026.10.12")];
        assert_eq!(
            horizon_update(&releases, &["v2026.10.12".into()]).map(|r| r.tag),
            Some("v2026.10.13".into())
        );
        assert!(horizon_update(&releases, &["v2026.10.13".into()]).is_none());
        assert!(launcher_update(&[r("launcher-v0.0.1")]).is_none());
        assert!(launcher_update(&[r("launcher@99.0.0")]).is_some());
        assert!(launcher_update_from(&[r("launcher-v99.0.0")], "launcher-v0.16.0").is_some());
        let installed = ["v2026.10.12".to_string(), "v2026.10.1".to_string()];
        assert!(select_after_update(Some("v2026.10.12"), &installed));
        assert!(!select_after_update(Some("v2026.10.1"), &installed));
        assert!(select_after_update(None, &installed));
    }

    /// The Nightly is the only build Horizon ships (docs/release-flow.md): every launcher takes the
    /// current one as its update, over every release; the last week's are listed, never offered.
    #[test]
    fn every_launcher_updates_to_the_current_nightly() {
        let asset = ASSET.unwrap_or("horizon-linux-x86_64.tar.gz");
        let build = |url: &str| serde_json::json!([{"name": asset, "browser_download_url": url, "size": 1}]);
        let list = serde_json::json!([
            {"tag_name": "nightly", "name": "Nightly 2026-10-28 · 9b772b2", "prerelease": true,
             "published_at": "2026-10-28T09:58:01Z", "assets": build("n")},
            {"tag_name": "launcher@0.2.21", "assets": []},
            {"tag_name": "v0.2.95", "assets": build("r")},
            {"tag_name": "nightly-2026-10-28", "name": "Nightly 2026-10-28 · 83f15fa", "prerelease": true,
             "published_at": "2026-10-28T07:54:57Z", "assets": build("d")},
            {"tag_name": "nightly-2026-10-27", "name": "Nightly 2026-10-27 · 7954902", "prerelease": true,
             "published_at": "2026-10-28T00:15:40Z", "assets": build("e")},
        ]);
        let releases = game_releases(&list);
        let listed: Vec<_> = releases
            .iter()
            .map(|r| (r.tag.as_str(), r.nightly.as_deref(), r.prerelease))
            .collect();
        assert_eq!(
            listed,
            [
                ("nightly-2026-10-28-9b772b2", Some("nightly"), false),
                (
                    "nightly-2026-10-28-83f15fa",
                    Some("nightly-2026-10-28"),
                    true
                ),
                (
                    "nightly-2026-10-27-7954902",
                    Some("nightly-2026-10-27"),
                    true
                ),
                ("v0.2.95", None, false),
            ]
        );
        let offered = |installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
            horizon_update(&installable_updates(&releases), &installed).map(|r| r.tag)
        };
        let current = Some("nightly-2026-10-28-9b772b2".to_string());
        // A player of the release channel, a new player, one on an older Nightly of the same day.
        assert_eq!(offered(&["v0.2.95", "v0.2.94"]), current);
        assert_eq!(offered(&[]), current);
        assert_eq!(offered(&["nightly-2026-10-28-83f15fa", "v0.2.95"]), current);
        assert_eq!(offered(&["nightly-2026-10-28-9b772b2"]), None);
        // Nightlies come after every release in every list.
        assert!(compare_versions("nightly-2026-10-27-7954902", "v0.2.95").is_gt());
        assert!(
            compare_versions("nightly-2026-10-28-9b772b2", "nightly-2026-10-27-7954902").is_gt()
        );
    }

    /// Players move to the Nightly at their next check: whoever was on the newest release, on a
    /// Nightly, or on nothing. A release picked by hand stays picked.
    #[test]
    fn players_of_the_newest_release_move_to_the_nightly() {
        let installed = [
            "nightly-2026-10-27-7954902".to_string(),
            "v0.2.95".into(),
            "v0.2.90".into(),
        ];
        assert!(select_after_update(Some("v0.2.95"), &installed));
        assert!(select_after_update(
            Some("nightly-2026-10-27-7954902"),
            &installed
        ));
        assert!(select_after_update(None, &installed));
        assert!(!select_after_update(Some("v0.2.90"), &installed));
        assert!(select_after_update(
            Some("v0.2.95"),
            &["v0.2.95".to_string()]
        ));
    }

    #[test]
    fn new_versions_install_automatically_unless_turned_off() {
        // No settings file, an old one that never mentioned it, and 0.1.1's default `false`.
        assert!(Settings::parse("").auto_install());
        assert!(Settings::parse("{}").auto_install());
        assert!(
            Settings::parse(r#"{"selected": "v2026.10.13", "auto_install": false}"#).auto_install()
        );
        assert!(Settings::parse(r#"{"install_new_versions": true}"#).auto_install());
        assert!(!Settings::parse(r#"{"install_new_versions": false}"#).auto_install());
        // Saving after the player turned it off keeps it off.
        let off = Settings {
            install_new_versions: Some(false),
            ..Settings::default()
        };
        assert!(!Settings::parse(&serde_json::to_string(&off).unwrap()).auto_install());
        // The selection rule is unchanged: the new version is selected only from the newest.
        let installed = ["v2026.10.12".to_string(), "v2026.10.1".to_string()];
        assert!(select_after_update(Some("v2026.10.12"), &installed));
        assert!(!select_after_update(Some("v2026.10.1"), &installed));
    }

    #[test]
    fn links_fall_back_to_the_built_in_ones() {
        let defaults = Links::default();
        assert_eq!(defaults.discord, "https://discord.gg/playhorizon");
        assert_eq!(defaults.sponsor, "https://github.com/sponsors/ludovic111");
        assert_eq!(defaults.website, "https://playhorizon.dev");
        // The file on main says the same thing as the defaults.
        assert_eq!(Links::parse(include_str!("../links.json")), defaults);
        // Unreadable or missing, offline included: the defaults.
        assert_eq!(Links::parse(""), defaults);
        assert_eq!(Links::parse("<html>404</html>"), defaults);
        assert_eq!(Links::parse("[1]"), defaults);
        assert_eq!(Links::parse(r#"{"discord": 5}"#), defaults);
        // A key that is missing keeps its default; a new address replaces it; empty hides it.
        assert_eq!(
            Links::parse(r#"{"discord": "https://x.y/z"}"#),
            Links {
                discord: "https://x.y/z".into(),
                ..defaults.clone()
            }
        );
        assert_eq!(
            Links::parse(r#"{"discord": "", "sponsor": " "}"#),
            Links {
                discord: String::new(),
                sponsor: String::new(),
                ..defaults.clone()
            }
        );
        // A key this launcher does not know is ignored.
        assert_eq!(Links::parse(r#"{"mastodon": "https://x.y/z"}"#), defaults);
    }

    #[test]
    fn a_new_version_reuses_resources_prepared_by_a_compatible_one() {
        let versions =
            std::env::temp_dir().join(format!("horizon-launcher-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&versions);
        let oracle = "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java";
        let camera = "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCameraOracle.java";
        let collision =
            "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java";
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let release = |tag: &str, minecraft: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": minecraft, "client_sha1": "c", "server_sha1": "s"}});
            write(tag, "research-lock.json", &lock.to_string());
            write(tag, "tools/prepare_native_client.py", "schema 7");
            write(tag, oracle, "heightmap");
            write(tag, collision, "collision");
            write(tag, camera, tag); // Not an exporter: it may change freely.
        };
        let prepared = |tag: &str, minecraft: &str| {
            let manifest =
                serde_json::json!({"schema": 7, "collision_exporters_sha256": {collision: "x"}});
            write(
                tag,
                &format!(".local/horizon-client/{minecraft}/{RESOURCES_MANIFEST}"),
                &manifest.to_string(),
            );
            write(
                tag,
                &format!(".local/horizon-client/{minecraft}/worldgen/biomes.json"),
                "{}",
            );
        };
        let is_prepared = |tag: &str| {
            resources_at(&versions.join(tag))
                .join(RESOURCES_MANIFEST)
                .exists()
        };

        release("v2026.10.13", "26.3");
        prepared("v2026.10.13", "26.3");
        release("v2026.10.12", "26.2");
        prepared("v2026.10.12", "26.2");
        release("v2026.10.13.1", "26.3");
        assert_eq!(
            reuse_prepared(&versions, "v2026.10.13.1"),
            Some("v2026.10.13".into())
        );
        assert!(is_prepared("v2026.10.13.1"));
        assert!(resources_at(&versions.join("v2026.10.13.1"))
            .join("worldgen/biomes.json")
            .exists());
        let left: Vec<_> = fs::read_dir(versions.join("v2026.10.13.1/.local/horizon-client"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["26.3"]);
        // Already prepared: nothing to do.
        assert_eq!(reuse_prepared(&versions, "v2026.10.13.1"), None);

        // Another Minecraft version, a changed exporter or a changed prepare script: prepare again.
        release("v2026.10.14", "26.4");
        assert_eq!(reuse_prepared(&versions, "v2026.10.14"), None);
        release("v2026.10.15", "26.3");
        write("v2026.10.15", collision, "collision, fixed");
        assert_eq!(reuse_prepared(&versions, "v2026.10.15"), None);
        release("v2026.10.16", "26.3");
        write("v2026.10.16", "tools/prepare_native_client.py", "schema 8");
        assert_eq!(reuse_prepared(&versions, "v2026.10.16"), None);
        assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));

        // A version an older launcher installed without preparing it is caught up by the sweep
        // that starting the window, `versions` and `update` run; the incompatible ones stay as
        // they are, and a second sweep has nothing to do.
        release("v2026.10.17", "26.3");
        assert_eq!(
            reuse_prepared_in(&versions),
            [("v2026.10.17".to_string(), "v2026.10.13.1".to_string())]
        );
        assert!(is_prepared("v2026.10.17"));
        assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));
        assert!(reuse_prepared_in(&versions).is_empty());

        // Versions with horizon-prepare: its sources are the preparer, so the same sources
        // reuse each other's resources, and a change or a version without them prepares again.
        let crate_file = "crates/horizon-prepare/src/lib.rs";
        release("v2026.10.18", "26.3");
        write("v2026.10.18", crate_file, "rust preparer");
        assert_eq!(reuse_prepared(&versions, "v2026.10.18"), None);
        prepared("v2026.10.18", "26.3");
        release("v2026.10.19", "26.3");
        write("v2026.10.19", crate_file, "rust preparer");
        assert_eq!(
            reuse_prepared(&versions, "v2026.10.19"),
            Some("v2026.10.18".into())
        );
        release("v2026.10.20", "26.3");
        write("v2026.10.20", crate_file, "rust preparer, changed");
        assert_eq!(reuse_prepared(&versions, "v2026.10.20"), None);
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn prepared_resources_follow_the_preparers_output_version() {
        let versions = std::env::temp_dir().join(format!(
            "horizon-launcher-output-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&versions);
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let lib = "crates/horizon-prepare/src/lib.rs";
        // A preparer with its sources (as releases ship them) and the exporters it runs.
        let release = |tag: &str, code: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": "s"}});
            write(tag, "research-lock.json", &lock.to_string());
            write(tag, "crates/horizon-prepare/Cargo.toml", "[package]");
            write(tag, "crates/horizon-prepare/build.rs", "fn main() {}");
            write(tag, lib, code);
            for name in LEGACY_EXPORTERS {
                write(
                    tag,
                    &format!("mods/horizon/src/test/java/dev/horizonlab/parity/{name}"),
                    name,
                );
            }
        };
        let prepared = |tag: &str| {
            write(
                tag,
                &format!(".local/horizon-client/26.3/{RESOURCES_MANIFEST}"),
                r#"{"schema": 7}"#,
            );
            write(tag, ".local/horizon-client/26.3/facts.json", tag);
        };
        let facts = |tag: &str| {
            fs::read_to_string(resources_at(&versions.join(tag)).join("facts.json")).ok()
        };

        // The preparer's code changed, not what it makes: the resources are kept.
        release(
            "v0.3.0",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() {}",
        );
        prepared("v0.3.0");
        release(
            "v0.3.1",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() { faster() }",
        );
        assert_eq!(reuse_prepared(&versions, "v0.3.1"), Some("v0.3.0".into()));
        assert_eq!(facts("v0.3.1").as_deref(), Some("v0.3.0"));
        // Its output changed, so the number did: prepare again.
        release(
            "v0.3.2",
            "pub const OUTPUT_VERSION: u32 = 2;\nfn prepare() {}",
        );
        assert_eq!(reuse_prepared(&versions, "v0.3.2"), None);
        assert_eq!(facts("v0.3.2"), None);
        // Another Minecraft jar, same number: prepare again.
        release(
            "v0.3.3",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() {}",
        );
        let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "other", "server_sha1": "s"}});
        write("v0.3.3", "research-lock.json", &lock.to_string());
        assert_eq!(reuse_prepared(&versions, "v0.3.3"), None);

        // A preparer from before the number is known by its fingerprint.
        release("v0.2.64", "fn prepare() {}");
        let fingerprint = legacy_fingerprint(&versions.join("v0.2.64")).unwrap();
        let legacy = [(fingerprint.as_str(), 1)];
        let at = |tag: &str| output_version_in(&versions.join(tag), &legacy);
        assert_eq!(at("v0.2.64"), Some(1));
        assert_eq!(at("v0.3.2"), Some(2));
        // Line endings as checked out don't count; an exporter's or the sources' change does.
        write(
            "v0.2.64",
            "mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java",
            "ParityData.java\r",
        );
        assert_eq!(at("v0.2.64"), Some(1));
        write(
            "v0.2.64",
            "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java",
            "changed",
        );
        assert_eq!(at("v0.2.64"), None);
        release("v0.2.65", "fn prepare() { changed() }");
        assert_eq!(at("v0.2.65"), None);
        // A folder without the preparer's sources has no number.
        assert_eq!(output_version_in(&versions.join("missing"), &legacy), None);
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn prepared_versions_get_the_run_time_reports_without_preparing_again() {
        let versions = std::env::temp_dir().join(format!(
            "horizon-launcher-reports-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&versions);
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let release = |tag: &str, server: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": server}});
            write(tag, "research-lock.json", &lock.to_string());
            write(
                tag,
                &format!(".local/horizon-client/26.3/{RESOURCES_MANIFEST}"),
                "{}",
            );
        };
        let resources = |tag: &str| resources_at(&versions.join(tag));
        let datagen = |tag: &str, name: &str| {
            write(tag, &format!(".local/datagen/out/reports/{name}"), tag);
        };

        // Prepared before the reports shipped with the resources: its own scratch output.
        release("v2026.10.13", "s");
        datagen("v2026.10.13", "registries.json");
        datagen("v2026.10.13", "commands.json");
        // Resources copied from it: the reports come from the version that ran the generator.
        release("v2026.10.14", "s");
        // Another server jar: the reports of the others would be wrong, and it has none.
        release("v2026.10.15", "other");
        assert!(reuse_prepared_in(&versions).is_empty());
        for tag in ["v2026.10.13", "v2026.10.14"] {
            for name in RUNTIME_REPORTS {
                assert_eq!(
                    fs::read_to_string(resources(tag).join(name)).unwrap(),
                    "v2026.10.13"
                );
            }
        }
        assert!(!resources("v2026.10.15").join("registries.json").exists());
        assert!(!complete_reports(&versions, "v2026.10.15"));
        // Once there they stay as they are, even when the scratch output is gone.
        fs::remove_dir_all(versions.join("v2026.10.13/.local/datagen")).unwrap();
        assert!(complete_reports(&versions, "v2026.10.13"));
        let left: Vec<_> = fs::read_dir(resources("v2026.10.14"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .filter(|n| n.contains("partial"))
            .collect();
        assert!(left.is_empty());
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn prepared_versions_get_what_the_asset_store_adds_without_preparing_again() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-launcher-packs-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let (versions, store, empty) = (dir.join("versions"), dir.join("store"), dir.join("none"));
        let write = |path: PathBuf, data: &[u8]| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        // An official launcher folder whose asset store holds the two packs, a panorama (fake
        // 2x2 PNG headers), Unifont, a language and the sound list.
        let background = "minecraft/textures/gui/title/background";
        let png = |n: u8| {
            let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x02\0\0\0\x02".to_vec();
            data.push(n);
            data
        };
        let mut assets: Vec<(String, Vec<u8>)> = horizon_prepare::BUILT_IN_PACKS
            .iter()
            .map(|name| (name.to_string(), name.as_bytes().to_vec()))
            .collect();
        for face in ["0", "1", "2", "3", "4", "5", "overlay"] {
            assets.push((
                format!("{background}/panorama_{face}.png"),
                png(face.as_bytes()[0]),
            ));
        }
        for name in [
            "minecraft/font/include/unifont.json",
            "minecraft/font/unifont.zip",
            "pack.mcmeta",
            "minecraft/lang/fr_fr.json",
            "minecraft/sounds.json",
        ] {
            assets.push((name.into(), format!("{{\"{name}\": 1}}").into_bytes()));
        }
        let mut index = serde_json::Map::new();
        for (name, data) in &assets {
            let hash = sources::sha1_hex(data);
            write(
                store.join("assets/objects").join(&hash[..2]).join(&hash),
                data,
            );
            index.insert(
                name.clone(),
                serde_json::json!({"hash": hash, "size": data.len()}),
            );
        }
        let index = serde_json::json!({ "objects": index }).to_string();
        write(store.join("assets/indexes/7.json"), index.as_bytes());
        let manifest = serde_json::json!({
            "id": "9.9",
            "assetIndex": {"id": "7", "sha1": sources::sha1_hex(index.as_bytes())},
            "downloads": {"client": {"sha1": "c"}, "server": {"sha1": "s"}},
            "javaVersion": {"majorVersion": 25},
        })
        .to_string();
        write(store.join("versions/9.9/9.9.json"), manifest.as_bytes());
        fs::create_dir_all(&empty).unwrap();
        // The client jar's 1x1 placeholder.
        let placeholder = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01";
        let release = |tag: &str, prepared: bool| {
            let root = versions.join(tag);
            write(root.join(COMPLETE), b"");
            let lock = serde_json::json!({"minecraft": {"version": "9.9", "java_major": 25,
                "client_sha1": "c", "server_sha1": "s", "manifest_sha1": sources::sha1_hex(manifest.as_bytes())}});
            write(root.join("research-lock.json"), lock.to_string().as_bytes());
            if prepared {
                let resources = resources_at(&root);
                write(resources.join(RESOURCES_MANIFEST), b"{}");
                write(resources.join("assets/minecraft/lang/en_us.json"), b"{}");
                write(
                    resources.join("assets/minecraft/lang/deprecated.json"),
                    b"{}",
                );
                write(
                    resources
                        .join("assets")
                        .join(background)
                        .join("panorama_0.png"),
                    placeholder,
                );
            }
        };
        let lines = std::cell::RefCell::new(Vec::new());
        let log = |line: String| lines.borrow_mut().push(line);
        let asset = |tag: &str, name: &str| {
            fs::read(resources_at(&versions.join(tag)).join("assets").join(name)).ok()
        };

        // Resources prepared without any of them: a folder without an asset store is skipped.
        release("v0.2.1", true);
        complete_assets_in(
            &versions,
            "v0.2.1",
            || vec![empty.clone(), store.clone()],
            &log,
        );
        assert_eq!(
            *lines.borrow(),
            [
                "Built-in pack assets ready: 2 files, 0.0 MB",
                "Panorama assets ready: 7 files, 0.0 MB",
                "Font assets ready: 2 files, 0.0 MB",
                "Language assets ready: 2 files, 0.0 MB",
                "Sound assets ready: 1 files, 0.0 MB",
            ]
        );
        for (name, data) in &assets {
            assert_eq!(asset("v0.2.1", name).as_ref(), Some(data), "{name}");
        }
        // Nothing else is prepared again.
        let resources = resources_at(&versions.join("v0.2.1"));
        assert_eq!(fs::read(resources.join(RESOURCES_MANIFEST)).unwrap(), b"{}");

        // No asset store anywhere: one line each, the version stays prepared, and no second try.
        lines.borrow_mut().clear();
        release("v0.2.2", true);
        complete_assets_in(&versions, "v0.2.2", || vec![empty.clone()], &log);
        complete_assets_in(&versions, "v0.2.2", || vec![empty.clone()], &log);
        let what = lines
            .borrow()
            .iter()
            .map(|line| line.split(" not added: ").next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            what,
            [
                "v0.2.2: Programmer Art and High Contrast",
                "v0.2.2: The title panorama",
                "v0.2.2: The Unifont glyphs",
                "v0.2.2: The languages",
                "v0.2.2: The sounds",
            ]
        );
        assert!(resources_at(&versions.join("v0.2.2"))
            .join(RESOURCES_MANIFEST)
            .exists());
        assert_eq!(
            asset("v0.2.2", &format!("{background}/panorama_0.png")).as_deref(),
            Some(&placeholder[..])
        );

        // Not prepared, or complete: nothing to do, without even looking for a Minecraft folder.
        lines.borrow_mut().clear();
        release("v0.2.3", false);
        let unasked = || -> Vec<PathBuf> { unreachable!("no folder is needed") };
        complete_assets_in(&versions, "v0.2.3", unasked, &log);
        complete_assets_in(&versions, "v0.2.1", unasked, &log);
        assert!(lines.borrow().is_empty());
        assert!(!resources_at(&versions.join("v0.2.3")).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_the_app_bundle_around_the_binary() {
        let exe = Path::new("/Applications/Horizon.app/Contents/MacOS/horizon-launcher");
        assert_eq!(
            app_bundle(exe),
            Some(Path::new("/Applications/Horizon.app"))
        );
        assert_eq!(app_bundle(Path::new("/Users/me/horizon-launcher")), None);
        assert_eq!(
            app_bundle(Path::new("/x/Contents/MacOS/horizon-launcher")),
            None
        );
        let work = std::env::temp_dir().join(format!("horizon-find-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(work.join("dist/Horizon.app/Contents/MacOS")).unwrap();
        assert_eq!(find_app(&work), Some(work.join("dist/Horizon.app")));
        let _ = fs::remove_dir_all(&work);
    }

    /// The whole update of an app bundle, as the release ships it: signed, zipped with `ditto`,
    /// unpacked by the launcher, swapped in place of the installed one, still valid.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_app_bundle_updates_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let work = std::env::temp_dir().join(format!("horizon-bundle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        let make = |dir: &Path, version: &str| {
            let app = dir.join("Horizon.app");
            fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            fs::create_dir_all(app.join("Contents/Resources")).unwrap();
            let bin = app.join("Contents/MacOS/horizon-launcher");
            fs::copy("/usr/bin/true", &bin).unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(app.join("Contents/Resources/version"), version).unwrap();
            fs::write(
                app.join("Contents/Info.plist"),
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleExecutable</key><string>horizon-launcher</string>\
                 <key>CFBundleIdentifier</key><string>dev.horizon.test</string>\
                 <key>CFBundlePackageType</key><string>APPL</string></dict></plist>\n",
            )
            .unwrap();
            let signed = Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(&app)
                .status()
                .unwrap();
            assert!(signed.success());
            app
        };
        let installed = make(&work.join("Applications"), "1.0.0");
        let release = make(&work.join("release"), "2.0.0");
        let zip = work.join("release.zip");
        let zipped = Command::new("ditto")
            .args(["-c", "-k", "--norsrc", "--noextattr", "--keepParent"])
            .arg(&release)
            .arg(&zip)
            .status()
            .unwrap();
        assert!(zipped.success());
        unpack(&zip, "release.zip", &work.join("unpacked")).unwrap();
        let new = find_app(&work.join("unpacked")).unwrap();
        replace_bundle(&new, &installed).unwrap();
        let version = fs::read_to_string(installed.join("Contents/Resources/version")).unwrap();
        assert_eq!(version, "2.0.0");
        let bin = installed.join("Contents/MacOS/horizon-launcher");
        assert!(
            Command::new(&bin).status().unwrap().success(),
            "still executable"
        );
        let valid = Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(&installed)
            .status()
            .unwrap();
        assert!(valid.success(), "the updated bundle's signature holds");
        let left: Vec<_> = fs::read_dir(work.join("Applications"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            left,
            ["Horizon.app"],
            "nothing staged or moved aside is left"
        );

        // A bundle changed after it was signed is refused, and the installed one stays.
        let tampered = make(&work.join("tampered"), "3.0.0");
        fs::write(tampered.join("Contents/Resources/version"), "3.0.1").unwrap();
        assert!(replace_bundle(&tampered, &installed).is_err());
        let version = fs::read_to_string(installed.join("Contents/Resources/version")).unwrap();
        assert_eq!(version, "2.0.0");
        let left = fs::read_dir(work.join("Applications")).unwrap().count();
        assert_eq!(left, 1, "nothing staged is left");
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn launcher_versions_read_from_the_binary() {
        assert_eq!(
            parse_version_output("horizon-launcher 0.1.5\n"),
            Some("0.1.5")
        );
        assert_eq!(parse_version_output("something else"), None);
        assert_eq!(parse_version_output(""), None);
        // This test binary is not a launcher: no version, so an update goes ahead.
        assert_eq!(
            installed_launcher_version(&std::env::current_exe().unwrap()),
            None
        );
        assert_eq!(
            installed_launcher_version(Path::new("/nonexistent/horizon-launcher")),
            None
        );
    }

    /// The launcher's file was replaced while it ran (an update from the command line, or an
    /// earlier one): the next replacement still lands at the path it started from.
    #[cfg(unix)]
    #[test]
    fn a_replaced_binary_is_replaced_again_at_its_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("horizon-launcher-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("horizon-launcher");
        fs::write(&exe, "old").unwrap();
        // Another update swaps the file under the running launcher.
        fs::write(dir.join("other"), "other update").unwrap();
        fs::rename(dir.join("other"), &exe).unwrap();
        fs::write(dir.join("new"), "new").unwrap();
        replace_binary(&dir.join("new"), &exe).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        // The file there keeps its mode, made executable (the swapped-in one was 0644).
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // Gone altogether: it is put back, executable.
        fs::remove_file(&exe).unwrap();
        replace_binary(&dir.join("new"), &exe).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // No temporary file is left beside it.
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["horizon-launcher", "new"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pruning_keeps_the_newest_and_the_protected() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let installed = tags(&[
            "v0.2.91", "v0.2.90", "v0.2.88", "v0.2.85", "v0.2.80", "nightly",
        ]);
        assert_eq!(
            stale_versions(&installed, 2, &tags(&["v0.2.85"])),
            tags(&["v0.2.88", "v0.2.80"])
        );
        // A protected version among the newest still leaves the newest `keep` in place.
        assert_eq!(
            stale_versions(&installed, 2, &tags(&["v0.2.91"])),
            tags(&["v0.2.88", "v0.2.85", "v0.2.80"])
        );
        assert!(stale_versions(&installed, 9, &[]).is_empty());
    }

    #[test]
    fn crash_reports_are_found_and_summarised() {
        let dir =
            std::env::temp_dir().join(format!("horizon-launcher-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let start = std::time::SystemTime::now() - std::time::Duration::from_secs(2);
        assert!(crash_report_in_since(&dir, start).is_none());
        fs::create_dir_all(&dir).unwrap();
        let report = "---- Horizon Crash Report ----\n// Details below.\n\nTime: 2026-10-14 09:05:07\nDescription: Unexpected error\n\npanicked at src/game.rs:10:5:\nboom\n\t0: horizon_client::game::frame\n";
        fs::write(dir.join("crash-2026-10-14_09.05.07-client.txt"), report).unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        assert_eq!(
            crash_report_in_since(&dir, start).unwrap(),
            dir.join("crash-2026-10-14_09.05.07-client.txt")
        );
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        assert!(crash_report_in_since(&dir, later).is_none());
        assert_eq!(
            crash_summary(report).as_deref(),
            Some("panicked at src/game.rs:10:5: boom")
        );
        assert_eq!(crash_summary("no report"), None);
        let log = dir.join("latest.log");
        fs::write(&log, "a\nb\nc\n").unwrap();
        assert_eq!(tail_of(&log, 2).unwrap(), "b\nc\n");
        assert_eq!(tail_of(&log, 100).unwrap(), "a\nb\nc\n");
        assert!(tail_of(&dir.join("missing.log"), 5).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_notes() {
        let text = markdown_to_text(
            "# Title\n\nSome *words* and [a link](https://x.y).\n\n- one\n- two\n",
        );
        assert_eq!(
            text,
            "Title\n\nSome words and a link (https://x.y).\n\n- one\n- two\n"
        );
    }
}
