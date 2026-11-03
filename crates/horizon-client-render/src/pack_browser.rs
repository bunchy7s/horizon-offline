//! Browsing Modrinth for resource packs from the Select Resource Packs
//! screen, so a pack can be added without leaving the game.
//!
//! The launcher has its own browser (`launcher/src/browser/`), but the
//! launcher is a separate workspace: nothing there is reachable from the
//! game, so this is the small client the screen needs. Search and download
//! both run on a worker thread and report back over a channel; the screen
//! only ever polls.
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, channel},
};

/// Modrinth asks every client to name itself.
const AGENT: &str = concat!(
    "horizon-rust/horizon/",
    env!("CARGO_PKG_VERSION"),
    " (in-game resource pack browser)"
);
const API: &str = "https://api.modrinth.com/v2";
/// Hits one search asks for. The list shows them all; Modrinth's own cap is
/// 100.
pub const LIMIT: usize = 20;

/// One search hit, as much of it as the list draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub author: String,
    pub description: String,
    pub downloads: u64,
    /// Modrinth's icon for the project, empty when it has none.
    pub icon_url: String,
}

/// What the worker sends back.
#[derive(Debug)]
pub enum Message {
    Results(Vec<Hit>),
    /// A project's icon, as the PNG bytes Modrinth served.
    Icon(String, Vec<u8>),
    /// A pack landed in the packs folder under this file name.
    Downloaded(String),
    /// A technical reason in English: the screen shows it inside
    /// `horizon.pack.addFailed`.
    Failed(String),
}

/// The filters the sidebar offers: the label's translation key, and the
/// Modrinth facet it adds. Facets of one group are OR-ed, groups AND-ed, so
/// each group is its own inner list.
pub const FILTERS: [(&str, &str, &str); 6] = [
    ("horizon.pack.filter.realistic", "categories", "realistic"),
    ("horizon.pack.filter.simplistic", "categories", "simplistic"),
    (
        "horizon.pack.filter.vanillaLike",
        "categories",
        "vanilla-like",
    ),
    ("horizon.pack.filter.x16", "resolutions", "16x"),
    ("horizon.pack.filter.x32", "resolutions", "32x"),
    ("horizon.pack.filter.x64", "resolutions", "64x"),
];

/// The search URL for `query` with the chosen filters (indices into
/// [`FILTERS`]), restricted to resource packs. An empty query is Modrinth's
/// own browse (its default ordering).
pub fn search_url(query: &str, chosen: &std::collections::BTreeSet<usize>) -> String {
    // One inner list per group: within a group Modrinth ORs, between groups
    // it ANDs, so picking 16x and 32x widens rather than excluding both.
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for i in chosen.iter().filter(|i| **i < FILTERS.len()) {
        let (_, group, value) = FILTERS[*i];
        let term = format!("\"{group}:{value}\"");
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, terms)) => terms.push(term),
            None => groups.push((group, vec![term])),
        }
    }
    let mut facets = String::from("[[\"project_type:resourcepack\"]]");
    if !groups.is_empty() {
        let extra: Vec<String> = groups
            .iter()
            .map(|(_, terms)| format!("[{}]", terms.join(",")))
            .collect();
        facets = format!("[[\"project_type:resourcepack\"],{}]", extra.join(","));
    }
    format!(
        "{API}/search?limit={LIMIT}&index=relevance&facets={}&query={}",
        urlencode(&facets),
        urlencode(query.trim())
    )
}

/// The versions of `project` that name this game version, newest first as
/// Modrinth returns them.
pub fn versions_url(project: &str, game_version: &str) -> String {
    format!(
        "{API}/project/{}/version?game_versions={}",
        urlencode(project),
        urlencode(&format!("[\"{game_version}\"]"))
    )
}

/// Percent-encoding for a query value: everything but the unreserved set.
fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The hits of a `/search` reply. A hit missing the fields the list draws is
/// left out rather than drawn blank.
pub fn parse_search(body: &str) -> Result<Vec<Hit>, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let hits = value
        .get("hits")
        .and_then(|h| h.as_array())
        .ok_or("the reply has no hits")?;
    Ok(hits
        .iter()
        .filter_map(|hit| {
            Some(Hit {
                id: hit.get("project_id")?.as_str()?.to_owned(),
                title: hit.get("title")?.as_str()?.to_owned(),
                author: hit
                    .get("author")
                    .and_then(|a| a.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                description: hit
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                downloads: hit.get("downloads").and_then(|d| d.as_u64()).unwrap_or(0),
                icon_url: hit
                    .get("icon_url")
                    .and_then(|u| u.as_str())
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect())
}

/// The primary file of the newest version: its URL, its file name and the
/// SHA-1 the index states for it.
pub fn parse_version_file(body: &str) -> Result<(String, String, Option<String>), String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let versions = value.as_array().ok_or("the reply is not a version list")?;
    for version in versions {
        let Some(files) = version.get("files").and_then(|f| f.as_array()) else {
            continue;
        };
        // `primary` marks the pack itself; some versions ship extras beside it.
        let file = files
            .iter()
            .find(|f| f.get("primary").and_then(|p| p.as_bool()) == Some(true))
            .or_else(|| files.first());
        let Some(file) = file else { continue };
        let (Some(url), Some(name)) = (
            file.get("url").and_then(|u| u.as_str()),
            file.get("filename").and_then(|n| n.as_str()),
        ) else {
            continue;
        };
        let sha1 = file
            .get("hashes")
            .and_then(|h| h.get("sha1"))
            .and_then(|h| h.as_str())
            .map(str::to_owned);
        return Ok((url.to_owned(), name.to_owned(), sha1));
    }
    Err("no version of this pack names this game version".into())
}

/// A file name safe to write into the packs folder: Modrinth's own name with
/// every separator and parent reference taken out, and a `.zip` ending.
/// Without this a reply could name a path outside the folder.
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut cleaned: String = base
        .chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | ' ' | '(' | ')' | '[' | ']' => c,
            _ => '_',
        })
        .collect();
    cleaned = cleaned.trim_matches(['.', ' ']).to_owned();
    if cleaned.is_empty() {
        cleaned = "pack".into();
    }
    // Windows' device names cannot be files, whatever their extension.
    let stem = cleaned.split('.').next().unwrap_or_default().trim_end();
    let device = matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
    ) || (stem.len() == 4
        && ["COM", "LPT"]
            .iter()
            .any(|p| stem[..3].eq_ignore_ascii_case(p))
        && stem.as_bytes()[3].is_ascii_digit());
    if device {
        cleaned.insert(0, '_');
    }
    if !cleaned.to_ascii_lowercase().ends_with(".zip") {
        cleaned.push_str(".zip");
    }
    cleaned
}

/// `name` in `dir`, numbered when something of that name is already there, so
/// a download never replaces a pack the player already has. `None` once every
/// number up to 999 is taken.
pub fn free_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let path = dir.join(name);
    if !path.exists() {
        return Some(path);
    }
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, "zip"));
    (2..1000)
        .map(|n| dir.join(format!("{stem} ({n}).{extension}")))
        .find(|candidate| !candidate.exists())
}

/// The browser's state, owned by the pack screen.
pub struct Browser {
    pub query: String,
    pub hits: Vec<Hit>,
    pub selected: Option<usize>,
    pub scroll: f64,
    /// Set while a search or a download is in flight.
    pub busy: bool,
    /// Already-translated text, shown in place of the list.
    pub status: Option<String>,
    /// The hit an install is running for.
    pub installing: Option<usize>,
    /// Hits already installed this visit, by index.
    pub installed: std::collections::BTreeSet<usize>,
    /// The sidebar filters that are on, as indices into [`FILTERS`].
    pub filters: std::collections::BTreeSet<usize>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("query", &self.query)
            .field("hits", &self.hits.len())
            .field("busy", &self.busy)
            .finish()
    }
}

impl Clone for Browser {
    /// The screen is cloned for drawing; a clone carries the state, not the
    /// worker's channel, and starts its own.
    fn clone(&self) -> Self {
        let (sender, receiver) = channel();
        Self {
            query: self.query.clone(),
            hits: self.hits.clone(),
            selected: self.selected,
            scroll: self.scroll,
            busy: self.busy,
            status: self.status.clone(),
            installing: self.installing,
            installed: self.installed.clone(),
            filters: self.filters.clone(),
            sender,
            receiver,
        }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl Browser {
    pub fn new() -> Self {
        let (sender, receiver) = channel();
        Self {
            query: String::new(),
            hits: Vec::new(),
            selected: None,
            scroll: 0.0,
            busy: false,
            status: None,
            installing: None,
            installed: Default::default(),
            filters: Default::default(),
            sender,
            receiver,
        }
    }

    /// Anything the worker has finished since the last call. The caller
    /// refreshes its pack list when a [`Message::Downloaded`] comes back.
    pub fn poll(&mut self) -> Option<Message> {
        let message = self.receiver.try_recv().ok()?;
        let was_busy = self.busy;
        self.busy = false;
        match &message {
            Message::Results(hits) => {
                self.hits = hits.clone();
                self.selected = None;
                self.scroll = 0.0;
                self.status = None;
            }
            Message::Downloaded(_) => {
                self.status = None;
                if let Some(i) = self.installing.take() {
                    self.installed.insert(i);
                }
            }
            // An icon is the screen's to keep; it does not end a search.
            Message::Icon(..) => self.busy = was_busy,
            Message::Failed(reason) => {
                self.status = Some(reason.clone());
                self.installing = None;
            }
        }
        Some(message)
    }

    /// Searches for the current query on a worker thread.
    pub fn search(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = None;
        let url = search_url(&self.query, &self.filters);
        let sender = self.sender.clone();
        spawn("pack-browser-search", move || {
            let message = match get_text(&url) {
                Ok(body) => match parse_search(&body) {
                    Ok(hits) => Message::Results(hits),
                    Err(reason) => Message::Failed(reason),
                },
                Err(reason) => Message::Failed(reason),
            };
            let _ = sender.send(message);
        });
    }

    /// Fetches each hit's icon on one worker, so the list fills in as they
    /// arrive. Icons are small; a hit without one is skipped.
    pub fn fetch_icons(&self) {
        let wanted: Vec<(String, String)> = self
            .hits
            .iter()
            .filter(|h| !h.icon_url.is_empty())
            .map(|h| (h.id.clone(), h.icon_url.clone()))
            .collect();
        if wanted.is_empty() {
            return;
        }
        let sender = self.sender.clone();
        spawn("pack-browser-icons", move || {
            for (id, url) in wanted {
                match get_bytes(&url, MAX_ICON_BYTES) {
                    Ok(bytes) => {
                        if sender.send(Message::Icon(id, bytes)).is_err() {
                            return;
                        }
                    }
                    // A missing icon only leaves that row without one.
                    Err(e) => log::debug!("Horizon: pack icon {id} did not load: {e}"),
                }
            }
        });
    }

    /// Downloads the selected pack into `dir` on a worker thread.
    pub fn download(&mut self, index: usize, dir: &Path, game_version: &str) {
        if self.busy {
            return;
        }
        let Some(hit) = self.hits.get(index) else {
            return;
        };
        self.installing = Some(index);
        self.busy = true;
        self.status = None;
        let url = versions_url(&hit.id, game_version);
        let dir = dir.to_path_buf();
        let sender = self.sender.clone();
        spawn("pack-browser-download", move || {
            let _ = sender.send(download_pack(&url, &dir));
        });
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(work) {
        log::warn!("Horizon: {name} did not start: {e}");
    }
}

/// Fetches the version list, then the pack, and writes it into `dir`.
fn download_pack(versions: &str, dir: &Path) -> Message {
    let (url, name, sha1) = match get_text(versions).and_then(|body| parse_version_file(&body)) {
        Ok(found) => found,
        Err(reason) => return Message::Failed(reason),
    };
    let body = match get_bytes(&url, MAX_PACK_BYTES) {
        Ok(body) => body,
        Err(reason) => return Message::Failed(reason),
    };
    if let Some(expected) = &sha1 {
        let found = sha1_hex(&body);
        if !found.eq_ignore_ascii_case(expected) {
            return Message::Failed(format!(
                "the download did not match its checksum ({found} for {expected})"
            ));
        }
    }
    let name = safe_file_name(&name);
    if let Err(e) = std::fs::create_dir_all(dir) {
        return Message::Failed(e.to_string());
    }
    let Some(path) = free_path(dir, &name) else {
        return Message::Failed(format!("{name}: every numbered name is taken"));
    };
    // Written beside the pack and renamed, so a pack list never sees a
    // half-written zip.
    let partial = path.with_extension("partial");
    match std::fs::write(&partial, &body).and_then(|()| std::fs::rename(&partial, &path)) {
        Ok(()) => Message::Downloaded(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(name),
        ),
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Message::Failed(e.to_string())
        }
    }
}

/// A pack Modrinth serves is capped: a reply past this is refused rather than
/// read into memory.
const MAX_PACK_BYTES: usize = 512 * 1024 * 1024;
/// A project icon is a small image; one past this is not read.
const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;

/// Only HTTPS: a reply names the URLs fetched next (icons, files), and a plain
/// `http://` one could point anywhere unencrypted, the player's own network
/// included.
fn https_only(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err(format!("{url} is not an https address"))
    }
}

fn get_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .set("User-Agent", AGENT)
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())
}

fn get_bytes(url: &str, max: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    https_only(url)?;
    let response = ureq::get(url)
        .set("User-Agent", AGENT)
        .call()
        .map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(max as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    if body.len() > max {
        return Err(format!("{url} is larger than {max} bytes"));
    }
    Ok(body)
}

/// SHA-1 of `bytes` as lowercase hex, for the checksum Modrinth states beside
/// each file (the `sha1` crate the skins already use).
fn sha1_hex(bytes: &[u8]) -> String {
    crate::skins::sha1(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_the_published_vectors() {
        // FIPS 180-4's own examples, independent of this implementation.
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // Past one block, so the padding and the length word are exercised.
        assert_eq!(
            sha1_hex(&b"a".repeat(1000)),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
    }

    #[test]
    fn search_asks_only_for_resource_packs() {
        let none = std::collections::BTreeSet::new();
        let url = search_url("blue", &none);
        assert!(url.starts_with("https://api.modrinth.com/v2/search?"));
        assert!(url.contains("project_type%3Aresourcepack"));
        assert!(url.ends_with("query=blue"));
        // A query with its own separators cannot add parameters.
        assert!(search_url("a&b=c", &none).ends_with("query=a%26b%3Dc"));
    }

    #[test]
    fn filters_of_one_group_widen_and_groups_narrow() {
        let chosen = |v: &[usize]| v.iter().copied().collect();
        // Two resolutions are one group: Modrinth ORs inside a list.
        let both: std::collections::BTreeSet<usize> = chosen(&[3, 4]);
        let url = search_url("", &both);
        assert!(url.contains(&urlencode(
            r#"[["project_type:resourcepack"],["resolutions:16x","resolutions:32x"]]"#
        )));
        // A category and a resolution are separate lists, so they AND.
        let mixed: std::collections::BTreeSet<usize> = chosen(&[0, 3]);
        let url = search_url("", &mixed);
        assert!(url.contains(&urlencode(
            r#"[["project_type:resourcepack"],["categories:realistic"],["resolutions:16x"]]"#
        )));
        // Out-of-range indices are ignored rather than panicking.
        let silly: std::collections::BTreeSet<usize> = chosen(&[99]);
        assert!(search_url("", &silly).contains(&urlencode(r#"[["project_type:resourcepack"]]"#)));
    }

    #[test]
    fn versions_are_asked_for_this_game_version() {
        let url = versions_url("abc", "26.3");
        assert_eq!(
            url,
            "https://api.modrinth.com/v2/project/abc/version?game_versions=%5B%2226.3%22%5D"
        );
    }

    #[test]
    fn search_hits_are_read_and_incomplete_ones_left_out() {
        let body = r#"{"hits":[
            {"project_id":"aa","title":"Blue","author":"someone","description":"d","downloads":12},
            {"title":"no id"},
            {"project_id":"bb","title":"Bare"}
        ]}"#;
        let hits = parse_search(body).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "aa");
        assert_eq!(hits[0].downloads, 12);
        // The fields the list can do without default rather than drop the hit.
        assert_eq!(hits[1].id, "bb");
        assert_eq!(hits[1].author, "");
        assert_eq!(hits[1].downloads, 0);
        assert!(parse_search("not json").is_err());
        assert!(parse_search(r#"{"total":0}"#).is_err());
    }

    #[test]
    fn the_primary_file_of_the_newest_version_wins() {
        let body = r#"[
            {"files":[
                {"url":"https://e/x.txt","filename":"x.txt","primary":false},
                {"url":"https://e/p.zip","filename":"p.zip","primary":true,
                 "hashes":{"sha1":"abc"}}
            ]},
            {"files":[{"url":"https://e/old.zip","filename":"old.zip","primary":true}]}
        ]"#;
        let (url, name, sha1) = parse_version_file(body).unwrap();
        assert_eq!((url.as_str(), name.as_str()), ("https://e/p.zip", "p.zip"));
        assert_eq!(sha1.as_deref(), Some("abc"));
        // No file marked primary: the first one stands in.
        let single = r#"[{"files":[{"url":"https://e/a.zip","filename":"a.zip"}]}]"#;
        assert_eq!(parse_version_file(single).unwrap().1, "a.zip");
        assert!(parse_version_file("[]").is_err());
    }

    #[test]
    fn a_served_name_cannot_leave_the_packs_folder() {
        assert_eq!(safe_file_name("../../evil.zip"), "evil.zip");
        assert_eq!(safe_file_name("a/b/c.zip"), "c.zip");
        assert_eq!(safe_file_name(r"a\b\c.zip"), "c.zip");
        assert_eq!(safe_file_name("pack"), "pack.zip");
        assert_eq!(safe_file_name(""), "pack.zip");
        assert_eq!(safe_file_name("..."), "pack.zip");
        assert_eq!(safe_file_name("Nice Pack (v2).zip"), "Nice Pack (v2).zip");
        // Keeps its ending whatever its case, rather than gaining a second one.
        assert_eq!(safe_file_name("Pack.ZIP"), "Pack.ZIP");
        // Windows' device names get a prefix; a longer stem keeps its name.
        assert_eq!(safe_file_name("CON"), "_CON.zip");
        assert_eq!(safe_file_name("aux.zip"), "_aux.zip");
        assert_eq!(safe_file_name("com1.tar.zip"), "_com1.tar.zip");
        assert_eq!(safe_file_name("console.zip"), "console.zip");
        assert_eq!(safe_file_name("COM10.zip"), "COM10.zip");
    }

    #[test]
    fn a_download_never_replaces_a_pack_already_there() {
        let dir = std::env::temp_dir().join(format!("horizon-browser-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("p.zip"));
        let _ = std::fs::remove_file(dir.join("p (2).zip"));
        assert_eq!(free_path(&dir, "p.zip"), Some(dir.join("p.zip")));
        std::fs::write(dir.join("p.zip"), b"x").unwrap();
        assert_eq!(free_path(&dir, "p.zip"), Some(dir.join("p (2).zip")));
        std::fs::write(dir.join("p (2).zip"), b"x").unwrap();
        assert_eq!(free_path(&dir, "p.zip"), Some(dir.join("p (3).zip")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
