//! Vanilla's GS4 query (docs/server.md, section 8): `enable-query` and `query.port` in server.properties
//! (`[properties]` in server.toml) turn on a UDP listener that answers as vanilla 26.3's
//! `net.minecraft.server.rcon.thread.QueryThreadGs4` does: a handshake (a request of type 9 answers with a
//! challenge token), then the basic stat (MOTD, game type, world, players, port, address) or the full stat
//! (key/value pairs and the player names) to a request that carries the token. The supervisor serves it, and
//! asks the game for who is online as the control API does.
//!
//! Where it differs from vanilla, on purpose: while the game isn't running nothing answers (vanilla's query
//! thread dies with its server), who is online is read at most once a second, and a session id with bytes
//! outside ASCII is echoed as UTF-8 text does (Java's `new String(bytes, UTF_8)` replaces each bad byte, Rust's
//! lossy decoder each bad run; real clients mask the id to ASCII).

use crate::{
    api::{Phase, Shared},
    engine,
    supervisor::note,
};
use horizon_server_files::properties::{ServerProperties, SystemEntropy};
use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, UdpSocket},
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

/// `QueryThreadGs4.GAME_TYPE` and `GAME_ID`.
const GAME_TYPE: &str = "SMP";
const GAME_ID: &str = "MINECRAFT";
/// `QueryThreadGs4.buffer`: the most a request takes.
const MAX_PACKET: usize = 1460;
/// `CHALLENGE_CHECK_INTERVAL`: tokens are dropped this often (every one made before the check goes).
const PRUNE_EVERY: Duration = Duration::from_secs(30);
/// `RESPONSE_CACHE_TIME`: the full stat is kept this long.
const RULES_CACHE: Duration = Duration::from_secs(5);
/// How long what the game said about its players is believed.
const INFO_CACHE: Duration = Duration::from_secs(1);
/// `socket.setSoTimeout`: how often the listener wakes up to drop old tokens.
const WAKE: Duration = Duration::from_millis(500);
/// The request types: the handshake and a stat.
const HANDSHAKE: u8 = 9;
const STAT: u8 = 0;
/// A full stat request is a basic one plus four bytes of padding: magic (2), type (1), session id (4), token
/// (4), padding (4).
const FULL_STAT_LENGTH: usize = 15;

/// What a reply says about the server.
#[derive(Clone, Debug, PartialEq)]
struct Info {
    motd: String,
    world: String,
    version: String,
    players: Vec<String>,
    max_players: i32,
    port: u16,
    host_ip: String,
}

/// A handshake's token for one address (`QueryThreadGs4.RequestChallenge`).
struct Challenge {
    /// The session id the handshake carried, which the stats echo.
    ident: [u8; 4],
    token: i32,
}

/// The listener's state: the tokens handed out, when they are dropped next, and the last full stat.
struct Query {
    challenges: HashMap<SocketAddr, Challenge>,
    last_prune: Instant,
    /// When the full stat was built and its bytes (the session id at 1..5 is put in again for each reply).
    rules: Option<(Instant, Vec<u8>)>,
}

impl Query {
    fn new(now: Instant) -> Self {
        Self {
            challenges: HashMap::new(),
            last_prune: now,
            rules: None,
        }
    }

    /// `pruneChallenges`: every 30 seconds each token made before now goes.
    fn prune(&mut self, now: Instant) {
        if now.duration_since(self.last_prune) >= PRUNE_EVERY {
            self.last_prune = now;
            self.challenges.clear();
        }
    }

    /// `processPacket`: the reply to `packet` from `from`, or none. `token` makes a new challenge (vanilla's
    /// `nextInt(16777216)`), `info` is asked only when a stat is built.
    fn answer(
        &mut self,
        packet: &[u8],
        from: SocketAddr,
        now: Instant,
        token: impl FnOnce() -> i32,
        info: impl FnOnce() -> Option<Info>,
    ) -> Option<Vec<u8>> {
        self.prune(now);
        if packet.len() < 3 || packet[0] != 0xFE || packet[1] != 0xFD {
            return None;
        }
        match packet[2] {
            HANDSHAKE => {
                let mut ident = [0; 4];
                for (at, byte) in ident.iter_mut().zip(packet.iter().skip(3)) {
                    *at = *byte;
                }
                let token = token();
                self.challenges.insert(from, Challenge { ident, token });
                let mut reply = vec![HANDSHAKE];
                reply.extend(String::from_utf8_lossy(&ident).as_bytes());
                reply.extend(token.to_string().as_bytes());
                reply.push(0);
                Some(reply)
            }
            STAT => {
                let challenge = self.challenges.get(&from)?;
                // `PktUtils.intFromNetworkByteArray(data, 7, length)`: big-endian, 0 when the packet is short.
                let given = packet
                    .get(7..11)
                    .map_or(0, |b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]));
                if given != challenge.token {
                    return None;
                }
                let ident = challenge.ident;
                if packet.len() == FULL_STAT_LENGTH {
                    let stale = self
                        .rules
                        .as_ref()
                        .is_none_or(|(at, _)| now.duration_since(*at) >= RULES_CACHE);
                    if stale {
                        self.rules = Some((now, full_stat(&info()?)));
                    }
                    let mut reply = self.rules.as_ref()?.1.clone();
                    reply[1..5].copy_from_slice(&ident);
                    Some(reply)
                } else {
                    Some(basic_stat(&info()?, ident))
                }
            }
            _ => None,
        }
    }
}

/// `NetworkDataOutputStream.writeString`: UTF-8 and a NUL.
fn string(out: &mut Vec<u8>, text: &str) {
    out.extend(text.as_bytes());
    out.push(0);
}

/// The basic stat: the type (0), the session id, then MOTD, game type, world, players online, max players,
/// the port (a little-endian short) and the host address.
fn basic_stat(info: &Info, ident: [u8; 4]) -> Vec<u8> {
    let mut out = vec![STAT];
    out.extend(ident);
    string(&mut out, &info.motd);
    string(&mut out, GAME_TYPE);
    string(&mut out, &info.world);
    string(&mut out, &info.players.len().to_string());
    string(&mut out, &info.max_players.to_string());
    out.extend(info.port.to_le_bytes());
    string(&mut out, &info.host_ip);
    out
}

/// The full stat (`buildRuleResponse`) with a placeholder session id: the type, the id, `splitnum` and its two
/// bytes, the key/value pairs, then the `player_` section with each name and a closing NUL.
fn full_stat(info: &Info) -> Vec<u8> {
    let mut out = vec![STAT, 0, 0, 0, 0];
    string(&mut out, "splitnum");
    out.extend([0x80, 0]);
    for (key, value) in [
        ("hostname", info.motd.as_str()),
        ("gametype", GAME_TYPE),
        ("game_id", GAME_ID),
        ("version", info.version.as_str()),
        // `ServerInterface.getPluginNames` of a vanilla dedicated server.
        ("plugins", ""),
        ("map", info.world.as_str()),
        ("numplayers", &info.players.len().to_string()),
        ("maxplayers", &info.max_players.to_string()),
        ("hostport", &info.port.to_string()),
        ("hostip", info.host_ip.as_str()),
    ] {
        string(&mut out, key);
        string(&mut out, value);
    }
    out.extend([0, 1]);
    string(&mut out, "player_");
    out.push(0);
    for name in &info.players {
        string(&mut out, name);
    }
    out.push(0);
    out
}

/// What server.properties makes of the query, as `QueryThreadGs4.create` reads it.
enum Setup {
    /// `enable-query=false`.
    Off,
    /// On, but vanilla leaves it off with this warning.
    Refused(String),
    On {
        host: String,
        port: u16,
    },
}

fn setup(dir: &Path) -> Result<Setup, String> {
    let file = crate::config::read_properties(&dir.join(crate::config::PROPERTIES))?;
    let p = ServerProperties::new(file, &mut SystemEntropy::default())?;
    if !p.enable_query {
        return Ok(Setup::Off);
    }
    let Some(port) = u16::try_from(p.query_port).ok().filter(|port| *port > 0) else {
        return Ok(Setup::Refused(format!(
            "Invalid query port {} found in server.properties (queries disabled)",
            p.query_port
        )));
    };
    Ok(Setup::On {
        host: if p.server_ip.is_empty() {
            "0.0.0.0".into()
        } else {
            p.server_ip
        },
        port,
    })
}

/// Starts the query if server.properties turns it on. As in vanilla, what keeps it off is a warning in the
/// log, not a server that doesn't start.
pub fn start(shared: Arc<Shared>) -> Result<Option<SocketAddr>, String> {
    // The file the game reads, as server.toml makes it.
    crate::config::Config::load(&shared.dir)?
        .write_properties(&shared.dir)
        .map_err(|e| format!("server.properties: {e}"))?;
    let (host, port) = match setup(&shared.dir)? {
        Setup::Off => return Ok(None),
        Setup::Refused(why) => {
            note(&shared, "WARN", &why);
            return Ok(None);
        }
        Setup::On { host, port } => (host, port),
    };
    let socket = match UdpSocket::bind((host.as_str(), port)) {
        Ok(socket) => socket,
        Err(e) => {
            note(
                &shared,
                "WARN",
                &format!("Unable to initialise query system on {host}:{port}: {e}"),
            );
            return Ok(None);
        }
    };
    let address = socket.local_addr().map_err(|e| e.to_string())?;
    let game = shared.clone();
    let mut provider = Provider::default();
    serve(socket, move || provider.info(&game)).map_err(|e| format!("query: {e}"))?;
    note(&shared, "INFO", &format!("Query running on {host}:{port}"));
    Ok(Some(address))
}

/// Who is online, asked of the game at most once a second.
#[derive(Default)]
struct Provider {
    last: Option<(Instant, Info)>,
}

impl Provider {
    fn info(&mut self, shared: &Shared) -> Option<Info> {
        if let Some((at, info)) = &self.last
            && at.elapsed() < INFO_CACHE
        {
            return Some(info.clone());
        }
        let info = read_info(shared)?;
        self.last = Some((Instant::now(), info.clone()));
        Some(info)
    }
}

/// The server as the query tells it: server.properties and the game's `server-status`. None while the game
/// isn't running or doesn't answer.
fn read_info(shared: &Shared) -> Option<Info> {
    if shared.state.lock().ok()?.phase != Phase::Running {
        return None;
    }
    let file = crate::config::read_properties(&shared.dir.join(crate::config::PROPERTIES)).ok()?;
    let p = ServerProperties::new(file, &mut SystemEntropy::default()).ok()?;
    let status = engine::Control::connect(&shared.dir)
        .and_then(|mut game| game.ask("server-status"))
        .ok()?;
    let players = status["players"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect();
    Some(Info {
        motd: p.motd,
        world: p.level_name,
        version: horizon_core::version::VERSION_ID.to_owned(),
        players,
        max_players: status["max_players"]
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .unwrap_or(p.max_players),
        port: u16::try_from(p.server_port).unwrap_or(25565),
        host_ip: if p.server_ip.is_empty() {
            local_ip()
        } else {
            p.server_ip
        },
    })
}

/// The host address vanilla gives from the machine's own name when no `server-ip` is set: here the address the
/// machine reaches the internet from (a UDP socket is connected, nothing is sent), else `0.0.0.0` as vanilla
/// leaves it when it can't tell.
fn local_ip() -> String {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|()| s))
        .and_then(|s| s.local_addr())
        .map_or_else(|_| "0.0.0.0".into(), |a| a.ip().to_string())
}

/// A token as `nextInt(16777216)` makes one: 24 random bits, from the OS's random hasher seeds and the time.
fn random_token() -> i32 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    (hasher.finish() & 0xFF_FFFF) as i32
}

/// Serves `socket` until the process ends. `info` says how the server is; None means nothing answers.
fn serve(
    socket: UdpSocket,
    mut info: impl FnMut() -> Option<Info> + Send + 'static,
) -> io::Result<()> {
    socket.set_read_timeout(Some(WAKE))?;
    thread::Builder::new().name("query".into()).spawn(move || {
        let mut query = Query::new(Instant::now());
        let mut buffer = [0u8; MAX_PACKET];
        loop {
            match socket.recv_from(&mut buffer) {
                Ok((length, from)) => {
                    let reply = query.answer(
                        &buffer[..length],
                        from,
                        Instant::now(),
                        random_token,
                        &mut info,
                    );
                    if let Some(reply) = reply {
                        let _ = socket.send_to(&reply, from);
                    }
                }
                // The timeout is the wake-up that drops old tokens; a refused reply (Windows reports the
                // peer's ICMP error on the next read) is the peer's business.
                Err(_) => query.prune(Instant::now()),
            }
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn info() -> Info {
        Info {
            motd: "A Horizon Server".into(),
            world: "world".into(),
            version: "26.3".into(),
            players: vec!["Kim0".into(), "Kim1".into()],
            max_players: 20,
            port: 25565,
            host_ip: "192.168.1.10".into(),
        }
    }

    /// A handshake request with session id `session`.
    fn handshake(session: [u8; 4]) -> Vec<u8> {
        let mut p = vec![0xFE, 0xFD, HANDSHAKE];
        p.extend(session);
        p
    }

    /// A stat request: the session id, the token and `padding` bytes after it (4 asks for the full stat).
    fn stat(session: [u8; 4], token: i32, padding: usize) -> Vec<u8> {
        let mut p = vec![0xFE, 0xFD, STAT];
        p.extend(session);
        p.extend(token.to_be_bytes());
        p.extend(vec![0; padding]);
        p
    }

    /// A listener that has handed out `token` to `from` for `session`.
    fn after_handshake(from: SocketAddr, session: [u8; 4], token: i32) -> (Query, Instant) {
        let now = Instant::now();
        let mut query = Query::new(now);
        query
            .answer(&handshake(session), from, now, || token, || None)
            .unwrap();
        (query, now)
    }

    #[test]
    fn the_handshake_answers_with_the_session_and_the_token_as_text() {
        let now = Instant::now();
        let mut query = Query::new(now);
        let reply = query
            .answer(
                &handshake([1, 2, 3, 4]),
                address(1),
                now,
                || 9_513_671,
                || None,
            )
            .unwrap();
        assert_eq!(reply, b"\x09\x01\x02\x03\x049513671\0");
    }

    #[test]
    fn a_session_id_with_bytes_outside_ascii_is_echoed_as_text() {
        let now = Instant::now();
        let mut query = Query::new(now);
        let reply = query
            .answer(
                &handshake([0x80, b'a', 0, 7]),
                address(1),
                now,
                || 5,
                || None,
            )
            .unwrap();
        assert_eq!(reply, "\u{9}\u{FFFD}a\0\u{7}5\0".as_bytes());
    }

    #[test]
    fn packets_that_are_not_requests_get_no_answer() {
        let now = Instant::now();
        let mut query = Query::new(now);
        for packet in [
            &[][..],
            &[0xFE, 0xFD][..],
            &[0xFD, 0xFE, HANDSHAKE, 1, 2, 3, 4][..],
            &[0xFE, 0xFC, HANDSHAKE, 1, 2, 3, 4][..],
            // A type nobody answers.
            &[0xFE, 0xFD, 7, 1, 2, 3, 4][..],
        ] {
            assert_eq!(
                query.answer(packet, address(1), now, || 1, || Some(info())),
                None
            );
        }
    }

    #[test]
    fn a_stat_needs_the_token_the_handshake_gave_that_address() {
        let (mut query, now) = after_handshake(address(1), [1, 2, 3, 4], 777);
        // No handshake from this address.
        assert_eq!(
            query.answer(
                &stat([1, 2, 3, 4], 777, 0),
                address(2),
                now,
                || 0,
                || Some(info())
            ),
            None
        );
        // The wrong token.
        assert_eq!(
            query.answer(
                &stat([1, 2, 3, 4], 778, 0),
                address(1),
                now,
                || 0,
                || Some(info())
            ),
            None
        );
        // A request too short to hold one reads as token 0.
        assert_eq!(
            query.answer(
                &[0xFE, 0xFD, STAT, 1, 2, 3, 4],
                address(1),
                now,
                || 0,
                || Some(info())
            ),
            None
        );
        assert!(
            query
                .answer(
                    &stat([1, 2, 3, 4], 777, 0),
                    address(1),
                    now,
                    || 0,
                    || Some(info())
                )
                .is_some()
        );
    }

    #[test]
    fn the_basic_stat_is_the_motd_world_players_port_and_address() {
        let (mut query, now) = after_handshake(address(1), [1, 2, 3, 4], 42);
        let reply = query
            .answer(
                &stat([9, 9, 9, 9], 42, 0),
                address(1),
                now,
                || 0,
                || Some(info()),
            )
            .unwrap();
        // The session id is the handshake's, whatever the stat request carries.
        let mut expected = vec![0, 1, 2, 3, 4];
        expected.extend(b"A Horizon Server\0SMP\0world\x002\x0020\0");
        expected.extend(25565u16.to_le_bytes());
        expected.extend(b"192.168.1.10\0");
        assert_eq!(reply, expected);
    }

    #[test]
    fn the_full_stat_lists_the_pairs_then_the_players() {
        let (mut query, now) = after_handshake(address(1), [1, 2, 3, 4], 42);
        let reply = query
            .answer(
                &stat([1, 2, 3, 4], 42, 4),
                address(1),
                now,
                || 0,
                || Some(info()),
            )
            .unwrap();
        let mut expected = vec![0, 1, 2, 3, 4];
        expected.extend(b"splitnum\0\x80\0");
        expected.extend(
            b"hostname\0A Horizon Server\0gametype\0SMP\0game_id\0MINECRAFT\0version\x0026.3\0plugins\0\0\
              map\0world\0numplayers\x002\0maxplayers\x0020\0hostport\x0025565\0hostip\x00192.168.1.10\0",
        );
        expected.extend(b"\0\x01player_\0\0Kim0\0Kim1\0\0");
        assert_eq!(reply, expected);
    }

    #[test]
    fn any_other_length_than_fifteen_is_a_basic_stat() {
        let (mut query, now) = after_handshake(address(1), [1, 2, 3, 4], 42);
        for padding in [0, 1, 3, 5, 40] {
            let reply = query
                .answer(
                    &stat([1, 2, 3, 4], 42, padding),
                    address(1),
                    now,
                    || 0,
                    || Some(info()),
                )
                .unwrap();
            assert!(
                reply.starts_with(&[0, 1, 2, 3, 4, b'A']),
                "padding {padding}"
            );
        }
    }

    #[test]
    fn the_full_stat_is_kept_five_seconds_with_each_askers_session_id() {
        let now = Instant::now();
        let mut query = Query::new(now);
        for (port, session) in [(1, [1, 1, 1, 1]), (2, [2, 2, 2, 2])] {
            query
                .answer(&handshake(session), address(port), now, || 5, || None)
                .unwrap();
        }
        let first = query
            .answer(
                &stat([1, 1, 1, 1], 5, 4),
                address(1),
                now,
                || 0,
                || Some(info()),
            )
            .unwrap();
        let mut changed = info();
        changed.players = vec!["Someone".into()];
        // Inside the cache the old stat is sent, with the other session id and without asking for the info.
        let later = now + Duration::from_secs(4);
        let second = query
            .answer(
                &stat([2, 2, 2, 2], 5, 4),
                address(2),
                later,
                || 0,
                || panic!("not asked"),
            )
            .unwrap();
        assert_eq!(&second[1..5], [2, 2, 2, 2]);
        assert_eq!(second[5..], first[5..]);
        // After it, a new one.
        let later = now + Duration::from_secs(5);
        let third = query
            .answer(
                &stat([1, 1, 1, 1], 5, 4),
                address(1),
                later,
                || 0,
                || Some(changed),
            )
            .unwrap();
        assert!(third.windows(7).any(|w| w == b"Someone"));
        assert_ne!(third[5..], first[5..]);
    }

    #[test]
    fn a_token_lasts_until_the_next_check_which_comes_every_thirty_seconds() {
        let (mut query, now) = after_handshake(address(1), [1, 2, 3, 4], 42);
        let early = now + Duration::from_secs(29);
        assert!(
            query
                .answer(
                    &stat([1, 2, 3, 4], 42, 0),
                    address(1),
                    early,
                    || 0,
                    || Some(info())
                )
                .is_some()
        );
        // The check drops every token made before it, this one included.
        let check = now + Duration::from_secs(30);
        assert_eq!(
            query.answer(
                &stat([1, 2, 3, 4], 42, 0),
                address(1),
                check,
                || 0,
                || Some(info())
            ),
            None
        );
    }

    #[test]
    fn nothing_answers_a_stat_while_the_server_is_unknown() {
        let (mut query, now) = after_handshake(address(1), [1, 2, 3, 4], 42);
        assert_eq!(
            query.answer(&stat([1, 2, 3, 4], 42, 0), address(1), now, || 0, || None),
            None
        );
        assert_eq!(
            query.answer(&stat([1, 2, 3, 4], 42, 4), address(1), now, || 0, || None),
            None
        );
    }

    #[test]
    fn the_tokens_are_24_bits() {
        for _ in 0..1000 {
            assert!((0..1 << 24).contains(&random_token()));
        }
    }

    #[test]
    fn a_listener_on_a_real_socket_answers_the_handshake_and_both_stats() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let server = socket.local_addr().unwrap();
        serve(socket, || Some(info())).unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0u8; 2048];
        let mut ask = |packet: &[u8]| {
            client.send_to(packet, server).unwrap();
            let (n, _) = client.recv_from(&mut buffer).unwrap();
            buffer[..n].to_vec()
        };
        let reply = ask(&handshake([0, 1, 0, 1]));
        assert_eq!(reply[0], HANDSHAKE);
        assert_eq!(&reply[1..5], [0, 1, 0, 1]);
        assert_eq!(reply.last(), Some(&0));
        let token: i32 = std::str::from_utf8(&reply[5..reply.len() - 1])
            .unwrap()
            .parse()
            .unwrap();
        let basic = ask(&stat([0, 1, 0, 1], token, 0));
        assert_eq!(basic[0], STAT);
        assert!(basic[5..].starts_with(b"A Horizon Server\0"));
        let full = ask(&stat([0, 1, 0, 1], token, 4));
        assert!(full.windows(8).any(|w| w == b"numplayers"[..8].to_vec()));
        assert!(full.ends_with(b"player_\0\0Kim0\0Kim1\0\0"));
    }

    #[test]
    fn an_invalid_port_leaves_the_query_off_with_vanillas_warning() {
        let dir = std::env::temp_dir().join(format!("horizon-query-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |text: &str| std::fs::write(dir.join(crate::config::PROPERTIES), text).unwrap();
        write("enable-query=false\nquery.port=0\n");
        assert!(matches!(setup(&dir), Ok(Setup::Off)));
        for port in ["0", "65536", "-3"] {
            write(&format!("enable-query=true\nquery.port={port}\n"));
            match setup(&dir) {
                Ok(Setup::Refused(why)) => assert_eq!(
                    why,
                    format!(
                        "Invalid query port {port} found in server.properties (queries disabled)"
                    )
                ),
                _ => panic!("port {port} should be refused"),
            }
        }
        write("enable-query=true\nquery.port=25570\nserver-ip=\n");
        assert!(matches!(
            setup(&dir),
            Ok(Setup::On { ref host, port: 25570 }) if host == "0.0.0.0"
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
