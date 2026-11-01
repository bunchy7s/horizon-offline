//! The player's server list, `servers.dat` in an instance's game folder: the game's Multiplayer
//! screen reads and writes the same file. Its format is vanilla's (`ServerList.load`/`save`,
//! `ServerData.read`/`write`): uncompressed NBT, `servers` a list of compounds with `name`, `ip`,
//! an optional `icon` (a PNG in base64), `acceptTextures`, `acceptedCodeOfConduct` and `hidden`.
//! The launcher only adds and removes entries: every other field of every entry is written back
//! as it was read.
use base64::Engine;
use horizon_world::nbt::{self, Compound, JavaString, Limits, NamedTag, Tag};
use std::fs;
use std::path::Path;

const FILE: &str = "servers.dat";
/// Horizon's official survival server, named and placed as the game lists it.
pub const OFFICIAL: (&str, &str) = ("Horizon Official", "horizon.bunchy7s.dev");
/// Beside servers.dat once the official server was offered, so a player who removes it never
/// gets it back. The game writes the same file.
const OFFICIAL_MARKER: &str = "horizon_official_server.txt";

#[derive(Clone, Debug, PartialEq)]
pub struct Server {
    pub name: String,
    pub ip: String,
    /// The server's picture, as the game last saw it (PNG).
    pub icon: Option<Vec<u8>>,
}

/// The entries as they are in the file, the hidden ones (servers joined with Direct
/// Connection) too.
pub struct List {
    entries: Vec<Compound>,
}

fn key(k: &str) -> JavaString {
    JavaString::from(k)
}

fn text(entry: &Compound, k: &str) -> String {
    match entry.get(&key(k)) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

fn hidden(entry: &Compound) -> bool {
    matches!(entry.get(&key("hidden")), Some(Tag::Byte(b)) if *b != 0)
}

fn entry(name: &str, ip: &str) -> Compound {
    [
        (key("name"), Tag::String(name.into())),
        (key("ip"), Tag::String(ip.into())),
        (key("hidden"), Tag::Byte(0)),
    ]
    .into_iter()
    .collect()
}

/// An address as the game connects to it: any case, 25565 when no port is given.
fn same_address(a: &str, b: &str) -> bool {
    let norm = |ip: &str| {
        let ip = ip.trim().to_ascii_lowercase();
        ip.strip_suffix(":25565").map(str::to_owned).unwrap_or(ip)
    };
    norm(a) == norm(b)
}

impl List {
    /// The list in `home`; empty when there is none or it can't be read, as the game does.
    pub fn load(home: &Path) -> List {
        Self::read(home).unwrap_or(List {
            entries: Vec::new(),
        })
    }

    /// The list in `home`, empty when there is none; none when the file is there but can't be read.
    fn read(home: &Path) -> Option<List> {
        let bytes = match fs::read(home.join(FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return None,
        };
        let entries = match bytes.is_empty() {
            true => Vec::new(),
            false => match nbt::read_named(&bytes, Limits::default())
                .ok()?
                .0
                .tag
                .get("servers")
            {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(|t| match t {
                        Tag::Compound(c) => Some(c.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
        };
        Some(List { entries })
    }

    /// The servers the Multiplayer screen shows, in its order.
    pub fn servers(&self) -> Vec<Server> {
        self.entries
            .iter()
            .filter(|e| !hidden(e))
            .map(|e| Server {
                name: text(e, "name"),
                ip: text(e, "ip"),
                icon: Some(text(e, "icon"))
                    .filter(|s| !s.is_empty())
                    .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok()),
            })
            .collect()
    }

    /// Adds a server at the end of the shown ones, as the game's Add Server does.
    pub fn add(&mut self, name: &str, ip: &str) -> Result<(), String> {
        let (name, ip) = (name.trim(), ip.trim());
        if ip.is_empty() {
            return Err("A server needs an address.".into());
        }
        let name = if name.is_empty() {
            "Minecraft Server"
        } else {
            name
        };
        let at = self
            .entries
            .iter()
            .rposition(|e| !hidden(e))
            .map_or(0, |i| i + 1);
        self.entries.insert(at, entry(name, ip));
        Ok(())
    }

    /// Removes the `index`th shown server.
    pub fn remove(&mut self, index: usize) -> Result<Server, String> {
        let server = self
            .servers()
            .into_iter()
            .nth(index)
            .ok_or_else(|| format!("There is no server {}.", index + 1))?;
        let at = (0..self.entries.len())
            .filter(|&i| !hidden(&self.entries[i]))
            .nth(index)
            .unwrap();
        self.entries.remove(at);
        Ok(server)
    }

    /// Writes the list as the game does: a new file first, the old one kept as `servers.dat_old`.
    pub fn save(&self, home: &Path) -> Result<(), String> {
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([(
                "servers",
                Tag::List(self.entries.iter().cloned().map(Tag::Compound).collect()),
            )]),
        };
        let bytes = nbt::write_named(&root, Limits::default()).map_err(|e| e.to_string())?;
        let (new, current) = (home.join("servers.dat_new"), home.join(FILE));
        let written = fs::create_dir_all(home)
            .and_then(|()| fs::write(&new, bytes))
            .and_then(|()| match current.exists() {
                true => fs::copy(&current, home.join("servers.dat_old")).map(|_| ()),
                false => Ok(()),
            })
            .and_then(|()| fs::rename(&new, &current));
        written.map_err(|e| format!("Could not save the server list: {e}"))
    }
}

/// Puts the official server first in the list in `home`, once: not again after the player removes
/// it, and not when the list already has its address. A list that can't be read is left alone.
pub fn add_official(home: &Path) -> Result<(), String> {
    let marker = home.join(OFFICIAL_MARKER);
    if marker.exists() {
        return Ok(());
    }
    let (name, address) = OFFICIAL;
    let mut list = List::read(home).ok_or("The server list can't be read.")?;
    if !list.servers().iter().any(|s| same_address(&s.ip, address)) {
        list.entries.insert(0, entry(name, address));
        list.save(home)?;
    }
    fs::write(marker, format!("{address}\n")).map_err(|e| e.to_string())
}

/// The shown server a command names: its position from 1, its name (any case) or its address.
pub fn find(list: &List, query: &str) -> Result<(usize, Server), String> {
    let servers = list.servers();
    let q = query.trim();
    q.parse::<usize>()
        .ok()
        .filter(|n| (1..=servers.len()).contains(n))
        .map(|n| n - 1)
        .or_else(|| servers.iter().position(|s| s.name.eq_ignore_ascii_case(q)))
        .or_else(|| servers.iter().position(|s| s.ip.eq_ignore_ascii_case(q)))
        .map(|i| (i, servers[i].clone()))
        .ok_or_else(|| format!("No server {q} in the list"))
}

#[cfg(test)]
#[path = "server_list/tests.rs"]
mod tests;
