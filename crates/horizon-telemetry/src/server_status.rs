//! How many play on a Minecraft server, by the server-list status ping every client sends to show
//! a server in its Multiplayer list (vanilla's `ServerStatusPinger`): a handshake asking for the
//! status state, a status request, and the server's JSON answer. Nothing about the asker is sent.
//!
//! The launcher shows the official server's count beside "N playing now", and the telemetry
//! server writes it down every minute for playhorizon.dev/stats (`history`, the `official`
//! series). [`OFFICIAL_ADDRESS`] is the official server's built-in address for both; the launcher
//! takes the current one from `launcher/links.json`.
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Horizon's official survival server (SMP). The launcher's server list offers the same entry
/// (`launcher/src/server_list.rs`) until links.json names another; the telemetry server's
/// `HORIZON_TELEMETRY_OFFICIAL_SERVER` overrides it there. Its SRV record leads to the port.
pub const OFFICIAL_ADDRESS: &str = "play.playhorizon.dev";

/// The official server's earlier addresses: the game and the launcher move a server list's
/// entries for them to [`OFFICIAL_ADDRESS`], once. A move appends the old address here (and to
/// `official_server_previous` in `launcher/links.json`).
pub const OFFICIAL_PREVIOUS: &[&str] = &["horizon.bunchy7s.dev"];

/// [`OFFICIAL_ADDRESS`] with the port its SRV record gives, for askers that don't look SRV up
/// (the telemetry server).
pub const OFFICIAL_STATUS_ADDRESS: &str = "play.playhorizon.dev:24454";

/// The port a server address without one means.
pub const DEFAULT_PORT: u16 = 25565;

/// The protocol version the handshake names (`horizon_protocol`'s, 26.3). A server answers the
/// status of any version.
const PROTOCOL: i32 = 777;

/// The longest status answer read: room for a 64x64 icon in base64 and a long description.
const MAX_ANSWER: usize = 256 * 1024;

/// The players part of a status answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Players {
    pub online: u64,
    pub max: u64,
}

/// `host`, `host:port` or `[v6]:port` as host and port. SRV records are not looked up.
pub fn split_address(address: &str) -> Option<(String, u16)> {
    let address = address.trim();
    if address.is_empty() {
        return None;
    }
    if let Some(rest) = address.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if after.is_empty() => DEFAULT_PORT,
            None => return None,
        };
        return Some((host.to_owned(), port));
    }
    match address.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Some((host.to_owned(), port.parse().ok()?)),
        _ => Some((address.to_owned(), DEFAULT_PORT)),
    }
}

/// Asks `address` for its status within `timeout` (to connect, and again for each read and
/// write), and returns its player count.
pub fn players(address: &str, timeout: Duration) -> Result<Players, String> {
    let (host, port) = split_address(address).ok_or("not a server address")?;
    players_at(address, (&host, port), timeout)
}

/// [`players`], connecting to `target` instead: where the address's SRV record leads. The
/// handshake still names the address, as vanilla's status ping does.
pub fn players_at(
    address: &str,
    target: (&str, u16),
    timeout: Duration,
) -> Result<Players, String> {
    let (host, port) = split_address(address).ok_or("not a server address")?;
    let addr = target
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("the name has no address")?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(&request(&host, port))
        .map_err(|e| e.to_string())?;
    let json = read_answer(&mut stream)?;
    parse_players(&json)
}

/// The handshake (next state: status) and the status request, each framed with its length.
pub fn request(host: &str, port: u16) -> Vec<u8> {
    let mut handshake = Vec::new();
    write_varint(&mut handshake, 0x00);
    write_varint(&mut handshake, PROTOCOL);
    write_varint(&mut handshake, host.len() as i32);
    handshake.extend_from_slice(host.as_bytes());
    handshake.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut handshake, 1);
    let mut out = Vec::new();
    write_varint(&mut out, handshake.len() as i32);
    out.extend(handshake);
    // The status request: length 1, packet 0x00.
    out.extend([1, 0x00]);
    out
}

fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    loop {
        if v & !0x7f == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
}

fn read_varint(r: &mut impl Read) -> Result<i32, String> {
    let mut value = 0u32;
    for i in 0..5 {
        let mut byte = [0];
        r.read_exact(&mut byte).map_err(|e| e.to_string())?;
        value |= u32::from(byte[0] & 0x7f) << (7 * i);
        if byte[0] & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    Err("a varint longer than 5 bytes".into())
}

/// The status response packet's JSON.
pub fn read_answer(r: &mut impl Read) -> Result<String, String> {
    let length = read_varint(r)?;
    if length <= 0 || length as usize > MAX_ANSWER {
        return Err(format!("a status answer of {length} bytes"));
    }
    let mut packet = vec![0; length as usize];
    r.read_exact(&mut packet).map_err(|e| e.to_string())?;
    let mut body = packet.as_slice();
    if read_varint(&mut body)? != 0x00 {
        return Err("not a status answer".into());
    }
    let text = read_varint(&mut body)?;
    if text < 0 || text as usize != body.len() {
        return Err("a status answer of the wrong length".into());
    }
    String::from_utf8(body.to_vec()).map_err(|e| e.to_string())
}

/// `players.online` and `players.max` from a status answer.
pub fn parse_players(json: &str) -> Result<Players, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let players = value.get("players").ok_or("no players in the answer")?;
    let field = |k: &str| players.get(k).and_then(serde_json::Value::as_u64);
    Ok(Players {
        online: field("online").ok_or("no players.online")?,
        max: field("max").unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn answer(json: &str) -> Vec<u8> {
        let mut packet = Vec::new();
        write_varint(&mut packet, 0x00);
        write_varint(&mut packet, json.len() as i32);
        packet.extend_from_slice(json.as_bytes());
        let mut out = Vec::new();
        write_varint(&mut out, packet.len() as i32);
        out.extend(packet);
        out
    }

    #[test]
    fn addresses_split_into_host_and_port() {
        let split = |a| split_address(a);
        assert_eq!(
            split("horizon.bunchy7s.dev"),
            Some(("horizon.bunchy7s.dev".into(), 25565))
        );
        assert_eq!(
            split("example.org:25570"),
            Some(("example.org".into(), 25570))
        );
        assert_eq!(split("[::1]:25566"), Some(("::1".into(), 25566)));
        assert_eq!(split("[::1]"), Some(("::1".into(), 25565)));
        assert_eq!(split("example.org:http"), None);
        assert_eq!(split(" "), None);
    }

    #[test]
    fn varints_are_vanillas() {
        for (value, bytes) in [
            (0, vec![0x00]),
            (127, vec![0x7f]),
            (128, vec![0x80, 0x01]),
            (25565, vec![0xdd, 0xc7, 0x01]),
            (-1, vec![0xff, 0xff, 0xff, 0xff, 0x0f]),
        ] {
            let mut out = Vec::new();
            write_varint(&mut out, value);
            assert_eq!(out, bytes, "{value}");
            assert_eq!(read_varint(&mut bytes.as_slice()).unwrap(), value);
        }
    }

    #[test]
    fn a_server_answers_its_player_count() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let expected = request("127.0.0.1", port);
            let mut got = vec![0; expected.len()];
            socket.read_exact(&mut got).unwrap();
            assert_eq!(got, expected);
            let json = r#"{"version":{"name":"26.3","protocol":777},"players":{"max":100,"online":12,"sample":[]},"description":{"text":"Horizon"}}"#;
            socket.write_all(&answer(json)).unwrap();
        });
        let players = players(&format!("127.0.0.1:{port}"), Duration::from_secs(2)).unwrap();
        assert_eq!(
            players,
            Players {
                online: 12,
                max: 100
            }
        );
        server.join().unwrap();
    }

    #[test]
    fn bad_answers_are_refused() {
        assert!(read_answer(&mut [0x00].as_slice()).is_err());
        assert!(read_answer(&mut answer("{}")[..2].as_ref()).is_err());
        let mut wrong_id = answer("{}");
        wrong_id[1] = 0x01;
        assert!(read_answer(&mut wrong_id.as_slice()).is_err());
        assert!(parse_players(r#"{"players":{"max":5}}"#).is_err());
        assert!(parse_players("not json").is_err());
        assert_eq!(
            read_answer(&mut answer(r#"{"players":{"online":3}}"#).as_slice()).unwrap(),
            r#"{"players":{"online":3}}"#
        );
        assert_eq!(
            parse_players(r#"{"players":{"online":3}}"#).unwrap(),
            Players { online: 3, max: 0 }
        );
    }

    #[test]
    fn nobody_listening_fails_within_the_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(players(&format!("127.0.0.1:{port}"), Duration::from_millis(500)).is_err());
    }
}
