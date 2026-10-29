//! The multiplayer server list, stored like vanilla's ServerList in an
//! uncompressed `servers.dat`: `{servers: [{name, ip, icon?, acceptTextures?,
//! acceptedCodeOfConduct?}]}`.
use horizon_world::nbt::{self, JavaString, NamedTag, Tag};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct ServerEntry {
    pub name: String,
    pub ip: String,
    /// Base64 PNG favicon from the last status ping.
    pub icon: Option<String>,
    /// ServerPackStatus: None = prompt, Some(true) enabled, Some(false) disabled.
    pub accept_textures: Option<bool>,
    /// `String.hashCode` of the Code of Conduct accepted for good (ServerData's
    /// `acceptedCodeOfConduct`); 0 for none.
    pub accepted_code_of_conduct: i32,
}
impl ServerEntry {
    pub fn new(name: &str, ip: &str) -> Self {
        Self {
            name: name.to_owned(),
            ip: ip.to_owned(),
            icon: None,
            accept_textures: None,
            accepted_code_of_conduct: 0,
        }
    }
    fn to_tag(&self) -> Tag {
        let mut entries = vec![
            ("name", Tag::String(JavaString::from(self.name.as_str()))),
            ("ip", Tag::String(JavaString::from(self.ip.as_str()))),
        ];
        if let Some(icon) = &self.icon {
            entries.push(("icon", Tag::String(JavaString::from(icon.as_str()))));
        }
        if let Some(accept) = self.accept_textures {
            entries.push(("acceptTextures", Tag::Byte(i8::from(accept))));
        }
        if self.accepted_code_of_conduct != 0 {
            entries.push((
                "acceptedCodeOfConduct",
                Tag::Int(self.accepted_code_of_conduct),
            ));
        }
        Tag::compound(entries)
    }
    fn from_tag(tag: &Tag) -> Option<Self> {
        let string = |k: &str| match tag.get(k) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        Some(Self {
            name: string("name").unwrap_or_default(),
            ip: string("ip")?,
            icon: string("icon"),
            accept_textures: match tag.get("acceptTextures") {
                Some(Tag::Byte(b)) => Some(*b != 0),
                _ => None,
            },
            accepted_code_of_conduct: match tag.get("acceptedCodeOfConduct") {
                Some(Tag::Int(hash)) => *hash,
                _ => 0,
            },
        })
    }
}

/// ServerList.load: missing or unreadable files give an empty list.
pub fn load(path: &Path) -> Vec<ServerEntry> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok((root, _)) = nbt::read_named(&bytes, nbt::Limits::default()) else {
        return Vec::new();
    };
    // ServerList skips entries flagged "hidden" (the LAN/quick-play ones).
    match root.tag.get("servers") {
        Some(Tag::List(list)) => list
            .iter()
            .filter(|t| !matches!(t.get("hidden"), Some(Tag::Byte(1))))
            .filter_map(ServerEntry::from_tag)
            .collect(),
        _ => Vec::new(),
    }
}
/// ServerList.save: write to a temporary file, then replace.
pub fn save(path: &Path, servers: &[ServerEntry]) -> std::io::Result<()> {
    let root = NamedTag {
        name: JavaString::from(""),
        tag: Tag::compound([(
            "servers",
            Tag::List(servers.iter().map(ServerEntry::to_tag).collect()),
        )]),
    };
    write(path, &root)
}
/// Puts `entry` first, keeping every other entry and field as the file has them (the hidden
/// ones too). A file that is there but can't be read is left as it is.
pub fn prepend(path: &Path, entry: &ServerEntry) -> std::io::Result<()> {
    let mut root = match std::fs::read(path) {
        Ok(bytes) => {
            nbt::read_named(&bytes, nbt::Limits::default())
                .map_err(std::io::Error::other)?
                .0
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => NamedTag {
            name: JavaString::from(""),
            tag: Tag::Compound(Default::default()),
        },
        Err(e) => return Err(e),
    };
    let Tag::Compound(top) = &mut root.tag else {
        return Err(std::io::Error::other("servers.dat is not a compound"));
    };
    match top
        .entry(JavaString::from("servers"))
        .or_insert_with(|| Tag::List(Vec::new()))
    {
        Tag::List(list) => list.insert(0, entry.to_tag()),
        _ => return Err(std::io::Error::other("servers.dat has no server list")),
    }
    write(path, &root)
}
fn write(path: &Path, root: &NamedTag) -> std::io::Result<()> {
    let bytes = nbt::write_named(root, nbt::Limits::default()).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("dat_tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// ServerAddress.parseString: `host`, `host:port` or `[v6]:port`; default 25565.
pub fn parse_address(text: &str) -> Option<(String, u16)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if tail.is_empty() => 25565,
            None => return None,
        };
        return Some((host.to_owned(), port));
    }
    match text.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Some((host.to_owned(), port.parse().ok()?)),
        Some(_) => Some((text.to_owned(), 25565)),
        None => Some((text.to_owned(), 25565)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_dat_round_trips() {
        let dir = std::env::temp_dir().join(format!("horizon-servers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.dat");
        let list = vec![
            ServerEntry::new("Minecraft Server", "localhost:25565"),
            ServerEntry {
                accept_textures: Some(false),
                ..ServerEntry::new("Other", "example.org")
            },
        ];
        save(&path, &list).unwrap();
        assert_eq!(load(&path), list);
        // Vanilla layout: an unnamed root compound with a "servers" list.
        let (root, _) =
            nbt::read_named(&std::fs::read(&path).unwrap(), nbt::Limits::default()).unwrap();
        assert_eq!(root.name, JavaString::from(""));
        assert!(matches!(root.tag.get("servers"), Some(Tag::List(l)) if l.len() == 2));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepend_keeps_hidden_entries_and_unknown_fields() {
        let dir = std::env::temp_dir().join(format!("horizon-prepend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.dat");
        prepend(&path, &ServerEntry::new("First", "a.example")).unwrap();
        assert_eq!(load(&path), [ServerEntry::new("First", "a.example")]);
        let hidden = Tag::compound([
            ("name", Tag::String(JavaString::from("Direct"))),
            ("ip", Tag::String(JavaString::from("b.example"))),
            ("hidden", Tag::Byte(1)),
            ("acceptedCodeOfConduct", Tag::String(JavaString::from("x"))),
        ]);
        let root = NamedTag {
            name: JavaString::from(""),
            tag: Tag::compound([("servers", Tag::List(vec![hidden.clone()]))]),
        };
        write(&path, &root).unwrap();
        prepend(&path, &ServerEntry::new("Top", "c.example")).unwrap();
        let (root, _) =
            nbt::read_named(&std::fs::read(&path).unwrap(), nbt::Limits::default()).unwrap();
        let Some(Tag::List(list)) = root.tag.get("servers") else {
            panic!("a server list");
        };
        assert_eq!(
            list,
            &[ServerEntry::new("Top", "c.example").to_tag(), hidden]
        );
        // A file that can't be read stays as it is.
        std::fs::write(&path, b"not nbt").unwrap();
        assert!(prepend(&path, &ServerEntry::new("Top", "c.example")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not nbt");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_addresses_like_server_address() {
        assert_eq!(
            parse_address("localhost"),
            Some(("localhost".into(), 25565))
        );
        assert_eq!(
            parse_address("127.0.0.1:25570"),
            Some(("127.0.0.1".into(), 25570))
        );
        assert_eq!(parse_address("[::1]:1234"), Some(("::1".into(), 1234)));
        assert_eq!(parse_address("host:notaport"), None);
        assert_eq!(parse_address(""), None);
    }
}
