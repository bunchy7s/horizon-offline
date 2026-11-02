//! The mod list: which content mods a Horizon server and a Horizon client
//! have turned on, compared when a client joins (docs/mods.md, "Playing on a
//! server with mods").
//!
//! In the configuration phase the server sends its list on [`CHANNEL`] and
//! the client answers with its own on the same channel. The server then
//! refuses a client whose mods differ, naming each one, which the registry
//! hash of `horizon:block_registry` cannot do (it only sees ids): a mod
//! whose loot, recipes or textures were edited keeps its ids, not its file's
//! SHA-256. A vanilla client never hears the channel.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};
use std::sync::OnceLock;

/// The custom payload channel, both ways.
pub const CHANNEL: &str = "horizon:mod_list";
/// Mods in a list at most; a list with more is refused whole.
pub const MAX_ENTRIES: usize = 128;
/// Characters in a mod's id or version at most.
pub const MAX_TEXT: usize = 64;
/// Mods named in a refusal's reason per kind of difference; the rest is counted.
const SHOWN: usize = 6;

/// Where a mod runs (a manifest's `side`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Both halves must have it: it changes what the world holds.
    Both,
    /// Only the server needs it.
    Server,
    /// Only the player's game: a HUD, keys, a minimap. Any server accepts it.
    Client,
}

impl Side {
    fn code(self) -> u8 {
        match self {
            Side::Both => 0,
            Side::Server => 1,
            Side::Client => 2,
        }
    }

    fn from_code(code: u8) -> Result<Self> {
        match code {
            0 => Ok(Side::Both),
            1 => Ok(Side::Server),
            2 => Ok(Side::Client),
            _ => Err(Error::new(format!("mod list: side {code}"))),
        }
    }
}

/// A mod that is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub version: String,
    /// SHA-256 of the mod's file; none for a mod run from a folder.
    pub sha256: Option<[u8; 32]>,
    pub side: Side,
}

/// The mods this game has on, set once when they load (`game/mods.rs`), like
/// the registries they fill, and read when a connection is set up, whatever
/// side of it this game is.
static LOCAL: OnceLock<Vec<Entry>> = OnceLock::new();

/// Sets the mods this game has on; false when they were set already (a pack
/// reload builds another root, the registries keep the first).
pub fn set_local(mut entries: Vec<Entry>) -> bool {
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries.truncate(MAX_ENTRIES);
    LOCAL.set(entries).is_ok()
}

/// Whether `set_local` has run: a pack reload then skips reading and hashing
/// the mods' files again, since the first list stays.
pub fn is_set() -> bool {
    LOCAL.get().is_some()
}

/// The mods this game has on, by id.
pub fn local() -> Vec<Entry> {
    LOCAL.get().cloned().unwrap_or_default()
}

/// The entries the other side compares (`side = "both"`): what a join sends. Client-only
/// and server-only mods stay private, since any server can ask for the list.
pub fn shared(entries: &[Entry]) -> Vec<Entry> {
    entries
        .iter()
        .filter(|e| e.side == Side::Both)
        .cloned()
        .collect()
}

/// A list as a payload.
pub fn encode(entries: &[Entry]) -> Vec<u8> {
    let cut = |s: &str| -> String { s.chars().take(MAX_TEXT).collect() };
    let entries = &entries[..entries.len().min(MAX_ENTRIES)];
    let mut w = Writer::new();
    w.varint(entries.len() as i32);
    for e in entries {
        w.string(&cut(&e.id));
        w.string(&cut(&e.version));
        w.u8(e.side.code());
        match &e.sha256 {
            Some(hash) => {
                w.bool(true);
                w.bytes(hash);
            }
            None => w.bool(false),
        }
    }
    w.data
}

/// A payload's list; an error for one that breaks a cap.
pub fn decode(data: &[u8]) -> Result<Vec<Entry>> {
    let mut r = Reader::new(data);
    let count = r.varint()?;
    if !(0..=MAX_ENTRIES as i32).contains(&count) {
        return Err(Error::new(format!("mod list: {count} mods")));
    }
    let entries = (0..count)
        .map(|_| {
            let id = r.string(MAX_TEXT)?;
            let version = r.string(MAX_TEXT)?;
            let side = Side::from_code(r.u8()?)?;
            let sha256 = match r.bool()? {
                true => Some(r.bytes(32)?.try_into().expect("32 bytes")),
                false => None,
            };
            Ok(Entry {
                id,
                version,
                sha256,
                side,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if r.remaining() > 0 {
        return Err(Error::new("mod list: trailing bytes"));
    }
    Ok(entries)
}

/// How a client's mods differ from a server's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Difference {
    /// The server has it on and the client does not: `id version`.
    pub missing: Vec<String>,
    /// The client has it on, the server does not, and it changes the world.
    pub extra: Vec<String>,
    /// Both have it, not the same: `id (server version, yours version)`, or
    /// `id version (your file was changed)` for the same version.
    pub changed: Vec<String>,
}

impl Difference {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && self.changed.is_empty()
    }

    /// What the refused player reads.
    pub fn reason(&self) -> String {
        let mut parts = Vec::new();
        for (label, names) in [
            ("Missing", &self.missing),
            ("Not on this server", &self.extra),
            ("Different version or file", &self.changed),
        ] {
            if names.is_empty() {
                continue;
            }
            let mut text = names[..names.len().min(SHOWN)].join(", ");
            if names.len() > SHOWN {
                text.push_str(&format!(" and {} more", names.len() - SHOWN));
            }
            parts.push(format!("{label}: {text}"));
        }
        format!("Your mods don't match this server's. {}", parts.join(". "))
    }
}

/// What a client must change to join a server. Only `Both` mods count, on
/// either side: a `Server` mod is the server's business, a `Client` one the
/// player's.
pub fn compare(server: &[Entry], client: &[Entry]) -> Difference {
    let mut difference = Difference::default();
    for s in server.iter().filter(|s| s.side == Side::Both) {
        match client.iter().find(|c| c.id == s.id) {
            None => difference.missing.push(format!("{} {}", s.id, s.version)),
            Some(c) => {
                let other_file = matches!((s.sha256, c.sha256), (Some(a), Some(b)) if a != b);
                if s.version != c.version {
                    difference.changed.push(format!(
                        "{} (server {}, yours {})",
                        s.id, s.version, c.version
                    ));
                } else if other_file {
                    difference
                        .changed
                        .push(format!("{} {} (your file was changed)", s.id, s.version));
                }
            }
        }
    }
    for c in client.iter().filter(|c| c.side == Side::Both) {
        if !server.iter().any(|s| s.id == c.id) {
            difference.extra.push(format!("{} {}", c.id, c.version));
        }
    }
    difference
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, version: &str, hash: u8, side: Side) -> Entry {
        Entry {
            id: id.into(),
            version: version.into(),
            sha256: Some([hash; 32]),
            side,
        }
    }

    #[test]
    fn only_shared_mods_are_sent() {
        let list = [
            entry("ruby", "1.0", 1, Side::Both),
            entry("minimap", "2.0", 2, Side::Client),
            entry("admin", "3.0", 3, Side::Server),
        ];
        let sent = shared(&list);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].id, "ruby");
        assert!(compare(&list, &sent).is_empty());
    }

    #[test]
    fn a_list_round_trips() {
        let list = vec![
            entry("ruby", "1.2.0", 7, Side::Both),
            Entry {
                sha256: None,
                ..entry("dev", "0.1", 0, Side::Client)
            },
        ];
        assert_eq!(decode(&encode(&list)).unwrap(), list);
        assert_eq!(decode(&encode(&[])).unwrap(), []);
    }

    #[test]
    fn lists_over_the_caps_are_refused() {
        let mut w = Writer::new();
        w.varint(MAX_ENTRIES as i32 + 1);
        assert!(decode(&w.data).is_err());
        let mut w = Writer::new();
        w.varint(1);
        w.string(&"x".repeat(MAX_TEXT + 1));
        assert!(decode(&w.data).is_err());
        let mut w = Writer::new();
        w.varint(1);
        w.string("a");
        w.string("1");
        w.u8(9);
        w.bool(false);
        assert!(decode(&w.data).is_err());
        let mut trailing = encode(&[]);
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        assert!(decode(&[0xFF]).is_err());
    }

    #[test]
    fn the_same_mods_match_whatever_the_order() {
        let a = entry("a", "1", 1, Side::Both);
        let b = entry("b", "2", 2, Side::Both);
        assert!(compare(&[a.clone(), b.clone()], &[b, a]).is_empty());
    }

    #[test]
    fn each_kind_of_difference_is_named() {
        let server = [
            entry("ruby", "1.0", 1, Side::Both),
            entry("pipes", "2.0", 2, Side::Both),
            entry("glass", "1.0", 3, Side::Both),
        ];
        let client = [
            entry("ruby", "1.0", 1, Side::Both),
            entry("pipes", "2.0", 99, Side::Both),
            entry("glass", "0.9", 3, Side::Both),
            entry("xray_blocks", "1.0", 4, Side::Both),
        ];
        let d = compare(&server, &client);
        assert_eq!(d.missing, Vec::<String>::new());
        assert_eq!(d.extra, ["xray_blocks 1.0"]);
        assert_eq!(
            d.changed,
            [
                "pipes 2.0 (your file was changed)",
                "glass (server 1.0, yours 0.9)"
            ]
        );
        let d = compare(&server, &client[..1]);
        assert_eq!(d.missing, ["pipes 2.0", "glass 1.0"]);
        let reason = d.reason();
        assert!(reason.contains("Missing: pipes 2.0, glass 1.0"), "{reason}");
    }

    #[test]
    fn client_only_and_server_only_mods_never_block_a_join() {
        let server = [
            entry("anti_grief", "1", 1, Side::Server),
            entry("ruby", "1", 2, Side::Both),
        ];
        let client = [
            entry("minimap", "3", 3, Side::Client),
            entry("ruby", "1", 2, Side::Both),
        ];
        assert!(compare(&server, &client).is_empty());
        assert!(compare(&[], &[entry("minimap", "3", 3, Side::Client)]).is_empty());
    }

    #[test]
    fn a_mod_run_from_a_folder_matches_by_version() {
        let mut folder = entry("ruby", "1", 0, Side::Both);
        folder.sha256 = None;
        assert!(compare(&[entry("ruby", "1", 5, Side::Both)], &[folder.clone()]).is_empty());
        assert!(!compare(&[entry("ruby", "2", 5, Side::Both)], &[folder]).is_empty());
    }

    #[test]
    fn a_long_refusal_counts_the_rest() {
        let server: Vec<_> = (0..9)
            .map(|i| entry(&format!("m{i}"), "1", i, Side::Both))
            .collect();
        let reason = compare(&server, &[]).reason();
        assert!(reason.contains("m5 1 and 3 more"), "{reason}");
        assert!(!reason.contains("m6"), "{reason}");
    }
}
