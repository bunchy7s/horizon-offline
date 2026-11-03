//! Play with friends' relay. Hosts register and get a nameplate (the
//! relay-visible half of an invite code); guests resolve it; the relay
//! pairs each guest's connection with a fresh one from the host and copies
//! bytes between them. Those bytes are the end-to-end encrypted session
//! (docs/play-with-friends.md): the relay can't read them, only count them.
//! State is in memory; nothing about players is logged.
pub mod limits;

use horizon_friends::{
    code,
    wire::{self, ErrorCode, FromRelay, ToRelay},
};
use limits::{Bucket, Limiter, Rates};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, channel, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Config {
    pub max_hosts: usize,
    pub max_bridges: usize,
    pub max_pending_per_host: usize,
    /// Connections open at once, of every kind (each is a thread).
    pub max_connections: usize,
    /// Bytes a second each way per relayed stream.
    pub session_rate: f64,
    /// Bytes a second for everything relayed.
    pub total_rate: f64,
    pub rates: Rates,
    /// Count limits per client address (off when every client comes from
    /// the same proxy address).
    pub per_address: bool,
    /// Unknown codes a minute across all clients, and the burst.
    pub global_unknown_per_min: f64,
    pub global_unknown_burst: f64,
    /// Read a PROXY protocol v1 line before anything else.
    pub proxy_protocol: bool,
    pub idle: Duration,
    pub ping_every: Duration,
    pub host_timeout: Duration,
    pub first_line: Duration,
    pub attach_wait: Duration,
    pub ended_kept: Duration,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            max_hosts: 1000,
            max_bridges: 200,
            max_pending_per_host: 8,
            max_connections: 4096,
            session_rate: 1024.0 * 1024.0,
            total_rate: 100.0 * 1_000_000.0 / 8.0,
            rates: Rates::default(),
            per_address: true,
            global_unknown_per_min: 120.0,
            global_unknown_burst: 30.0,
            proxy_protocol: false,
            idle: Duration::from_secs(120),
            ping_every: Duration::from_secs(20),
            host_timeout: Duration::from_secs(60),
            first_line: Duration::from_secs(10),
            attach_wait: Duration::from_secs(10),
            ended_kept: Duration::from_secs(15 * 60),
        }
    }
}
impl Config {
    /// The defaults with the `RELAY_*` environment variables applied.
    pub fn from_env() -> Self {
        fn var<T: std::str::FromStr>(name: &str) -> Option<T> {
            std::env::var(name).ok()?.trim().parse().ok()
        }
        let mut c = Self::default();
        if let Some(v) = var("RELAY_MAX_HOSTS") {
            c.max_hosts = v;
        }
        if let Some(v) = var("RELAY_MAX_BRIDGES") {
            c.max_bridges = v;
        }
        if let Some(v) = var("RELAY_MAX_PENDING_PER_HOST") {
            c.max_pending_per_host = v;
        }
        if let Some(v) = var("RELAY_MAX_CONNECTIONS") {
            c.max_connections = v;
        }
        if let Some(v) = var::<f64>("RELAY_SESSION_KBPS") {
            c.session_rate = v * 1024.0;
        }
        if let Some(v) = var::<f64>("RELAY_TOTAL_MBPS") {
            c.total_rate = v * 1_000_000.0 / 8.0;
        }
        if let Some(v) = var::<f64>("RELAY_GLOBAL_UNKNOWN_PER_MIN") {
            c.global_unknown_per_min = v;
        }
        if let Some(v) = var::<u8>("RELAY_PER_ADDRESS_LIMITS") {
            c.per_address = v != 0;
        }
        if let Some(v) = var::<u8>("RELAY_PROXY_PROTOCOL") {
            c.proxy_protocol = v != 0;
        }
        c
    }
}

/// What the relay reports (and logs): counts only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub hosts: usize,
    pub bridges: usize,
    pub registrations: u64,
    pub joins: u64,
    pub unknown: u64,
    pub rate_limited: u64,
    pub relayed_bytes: u64,
}
impl Counts {
    pub fn json(&self) -> String {
        format!(
            "{{\"ok\":true,\"hosts\":{},\"bridges\":{},\"registrations\":{},\"joins\":{},\"unknown\":{},\"rate_limited\":{},\"relayed_bytes\":{}}}",
            self.hosts,
            self.bridges,
            self.registrations,
            self.joins,
            self.unknown,
            self.rate_limited,
            self.relayed_bytes
        )
    }
}

/// What the host's control connection sends next.
enum Out {
    Line(FromRelay),
    /// The host stopped answering or went away.
    Closed,
}
struct HostEntry {
    token: String,
    protocol: i32,
    game: String,
    to_host: Sender<Out>,
    pending: usize,
}
struct Pending {
    nameplate: String,
    attach: SyncSender<(TcpStream, SocketAddr)>,
}
#[derive(Default)]
struct Inner {
    hosts: HashMap<String, HostEntry>,
    ended: HashMap<String, Instant>,
    pending: HashMap<u64, Pending>,
}

struct State {
    config: Config,
    inner: Mutex<Inner>,
    limiter: Mutex<Limiter>,
    global_unknown: Mutex<Bucket>,
    total: Mutex<Bucket>,
    bridges: AtomicUsize,
    connections: AtomicUsize,
    registrations: AtomicU64,
    joins: AtomicU64,
    unknown: AtomicU64,
    rate_limited: AtomicU64,
    relayed_bytes: AtomicU64,
    stop: AtomicBool,
}

/// The state behind `mutex`, even if a connection's thread panicked holding it: one bad connection
/// must not make every later one panic too.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone)]
pub struct Relay {
    state: Arc<State>,
}

fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("OS random generator");
    u64::from_le_bytes(b)
}
fn random_token() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS random generator");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Relay {
    pub fn new(config: Config) -> Self {
        let now = Instant::now();
        Self {
            state: Arc::new(State {
                limiter: Mutex::new(Limiter::new(config.rates.clone(), config.per_address)),
                global_unknown: Mutex::new(Bucket::new(
                    config.global_unknown_per_min / 60.0,
                    config.global_unknown_burst,
                    now,
                )),
                total: Mutex::new(Bucket::new(config.total_rate, config.total_rate / 4.0, now)),
                config,
                inner: Mutex::new(Inner::default()),
                bridges: AtomicUsize::new(0),
                connections: AtomicUsize::new(0),
                registrations: AtomicU64::new(0),
                joins: AtomicU64::new(0),
                unknown: AtomicU64::new(0),
                rate_limited: AtomicU64::new(0),
                relayed_bytes: AtomicU64::new(0),
                stop: AtomicBool::new(false),
            }),
        }
    }
    /// A relay on a free loopback port, served on a thread (tests).
    pub fn spawn_local(config: Config) -> io::Result<(Self, SocketAddr, JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let relay = Self::new(config);
        let r = relay.clone();
        let t = std::thread::spawn(move || r.serve(listener));
        Ok((relay, address, t))
    }
    pub fn counts(&self) -> Counts {
        counts(&self.state)
    }
    pub fn stop(&self) {
        self.state.stop.store(true, Ordering::Relaxed);
    }
    /// Accept connections until [`Relay::stop`].
    pub fn serve(&self, listener: TcpListener) {
        let _ = listener.set_nonblocking(true);
        while !self.state.stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, peer)) => {
                    // Beyond the cap the connection is closed unanswered.
                    if self.state.connections.fetch_add(1, Ordering::Relaxed)
                        >= self.state.config.max_connections
                    {
                        self.state.connections.fetch_sub(1, Ordering::Relaxed);
                        continue;
                    }
                    let state = self.state.clone();
                    let spawned =
                        std::thread::Builder::new()
                            .name("relay-conn".into())
                            .spawn(move || {
                                let _ = stream.set_nonblocking(false);
                                let _ = handle(&state, stream, peer);
                                state.connections.fetch_sub(1, Ordering::Relaxed);
                            });
                    if spawned.is_err() {
                        self.state.connections.fetch_sub(1, Ordering::Relaxed);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
}

fn counts(s: &State) -> Counts {
    Counts {
        hosts: locked(&s.inner).hosts.len(),
        bridges: s.bridges.load(Ordering::Relaxed),
        registrations: s.registrations.load(Ordering::Relaxed),
        joins: s.joins.load(Ordering::Relaxed),
        unknown: s.unknown.load(Ordering::Relaxed),
        rate_limited: s.rate_limited.load(Ordering::Relaxed),
        relayed_bytes: s.relayed_bytes.load(Ordering::Relaxed),
    }
}

fn refuse(stream: &mut TcpStream, code: ErrorCode) -> io::Result<()> {
    wire::send(stream, &FromRelay::Error { code, game: None })
}

/// `PROXY TCP4 <src> <dst> <sport> <dport>`: the client's address.
fn proxy_line(line: &[u8]) -> Option<SocketAddr> {
    let text = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
    let parts: Vec<&str> = text.split(' ').collect();
    match parts.as_slice() {
        ["PROXY", "TCP4" | "TCP6", src, _, sport, _] => {
            Some(SocketAddr::new(src.parse().ok()?, sport.parse().ok()?))
        }
        _ => None,
    }
}

/// IPv4 clients of a dual-stack listener show up as `::ffff:a.b.c.d`:
/// their address as IPv4, which is what the other end can dial.
fn unmap(peer: SocketAddr) -> SocketAddr {
    match peer {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(v4.into(), v6.port()),
            None => peer,
        },
        v4 => v4,
    }
}

fn handle(state: &Arc<State>, mut stream: TcpStream, peer: SocketAddr) -> io::Result<()> {
    if state.stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let mut peer = unmap(peer);
    // The first line (and an HTTP request) within `first_line` in all.
    let first_line = horizon_friends::deadline(&stream, state.config.first_line)?;
    stream.set_read_timeout(Some(state.config.first_line))?;
    let _ = stream.set_nodelay(true);
    let mut line = wire::read_line(&mut stream)?.ok_or(io::ErrorKind::UnexpectedEof)?;
    if state.config.proxy_protocol {
        peer = proxy_line(&line)
            .map(unmap)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no PROXY line"))?;
        line = wire::read_line(&mut stream)?.ok_or(io::ErrorKind::UnexpectedEof)?;
    }
    if line.starts_with(b"GET ") {
        return http(state, &mut stream, &line, peer);
    }
    drop(first_line);
    let Ok(hello) = serde_json::from_slice::<ToRelay>(&line) else {
        return refuse(&mut stream, ErrorCode::BadRequest);
    };
    match hello {
        ToRelay::Host { v, protocol, game } if v == wire::VERSION => {
            host(state, stream, peer, protocol, game)
        }
        ToRelay::Join {
            v,
            nameplate,
            protocol,
            ..
        } if v == wire::VERSION => join(state, stream, peer, &nameplate, protocol),
        ToRelay::Attach { v, session, token } if v == wire::VERSION => {
            attach(state, stream, peer, session, &token)
        }
        _ => refuse(&mut stream, ErrorCode::BadRequest),
    }
}

fn http(state: &State, stream: &mut TcpStream, line: &[u8], peer: SocketAddr) -> io::Result<()> {
    // Drain the request headers (bounded) before answering.
    let mut headers = 0;
    while let Some(h) = wire::read_line(stream)? {
        headers += 1;
        if h.is_empty() || h == b"\r" || headers > 64 {
            break;
        }
    }
    let path = std::str::from_utf8(line)
        .ok()
        .and_then(|l| l.split(' ').nth(1))
        .unwrap_or("");
    let (status, body) = match path {
        "/health" => ("200 OK", counts(state).json()),
        // The address the relay sees for the caller: checks that the hosting
        // platform's proxy passes client addresses through.
        "/whoami" => ("200 OK", format!("{{\"address\":\"{peer}\"}}")),
        _ => ("404 Not Found", "{\"ok\":false}".to_owned()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}

fn host(
    state: &Arc<State>,
    mut stream: TcpStream,
    peer: SocketAddr,
    protocol: i32,
    game: String,
) -> io::Result<()> {
    let now = Instant::now();
    if !locked(&state.limiter).allow_register(peer.ip(), now) {
        state.rate_limited.fetch_add(1, Ordering::Relaxed);
        return refuse(&mut stream, ErrorCode::RateLimited);
    }
    let (tx, rx) = channel();
    let token = random_token();
    let nameplate = {
        let mut inner = locked(&state.inner);
        let kept = state.config.ended_kept;
        inner
            .ended
            .retain(|_, t| now.saturating_duration_since(*t) < kept);
        let mut found = None;
        if inner.hosts.len() < state.config.max_hosts {
            for _ in 0..64 {
                let n = code::nameplate(random_u64() as u32);
                if !inner.hosts.contains_key(&n) && !inner.ended.contains_key(&n) {
                    found = Some(n);
                    break;
                }
            }
        }
        if let Some(n) = &found {
            inner.hosts.insert(
                n.clone(),
                HostEntry {
                    token: token.clone(),
                    protocol,
                    game: game.chars().take(32).collect(),
                    to_host: tx.clone(),
                    pending: 0,
                },
            );
        }
        found
    };
    let Some(nameplate) = nameplate else {
        return refuse(&mut stream, ErrorCode::Full);
    };
    state.registrations.fetch_add(1, Ordering::Relaxed);
    let result = serve_host(state, &mut stream, peer, &nameplate, &token, tx, rx);
    {
        let mut inner = locked(&state.inner);
        if inner
            .hosts
            .get(&nameplate)
            .is_some_and(|h| h.token == token)
        {
            inner.hosts.remove(&nameplate);
            inner.ended.insert(nameplate, Instant::now());
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    result
}

/// The host's control connection: joins and pings out, pongs in. A host
/// silent for `host_timeout` is gone.
fn serve_host(
    state: &Arc<State>,
    stream: &mut TcpStream,
    peer: SocketAddr,
    nameplate: &str,
    token: &str,
    tx: Sender<Out>,
    rx: Receiver<Out>,
) -> io::Result<()> {
    wire::send(
        stream,
        &FromRelay::Registered {
            nameplate: nameplate.to_owned(),
            token: token.to_owned(),
            observed: Some(peer.to_string()),
        },
    )?;
    let mut reader = stream.try_clone()?;
    reader.set_read_timeout(Some(state.config.host_timeout))?;
    std::thread::Builder::new()
        .name("relay-host-read".into())
        .spawn(move || {
            while let Ok(Some(_)) = wire::read_line(&mut reader) {}
            let _ = tx.send(Out::Closed);
        })?;
    while !state.stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(state.config.ping_every) {
            Ok(Out::Line(m)) => wire::send(stream, &m)?,
            Ok(Out::Closed) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => wire::send(stream, &FromRelay::Ping)?,
        }
    }
    Ok(())
}

enum Lookup {
    Found(Sender<Out>),
    Unknown,
    Ended,
    Version(String),
    Full,
}

fn join(
    state: &Arc<State>,
    mut stream: TcpStream,
    peer: SocketAddr,
    nameplate: &str,
    protocol: i32,
) -> io::Result<()> {
    let now = Instant::now();
    state.joins.fetch_add(1, Ordering::Relaxed);
    // The global budget for unknown codes is checked before the lookup, so a
    // refusal says nothing about whether the nameplate exists.
    let allowed =
        locked(&state.limiter).allow_join(peer.ip(), now) && locked(&state.global_unknown).has(now);
    if !allowed {
        state.rate_limited.fetch_add(1, Ordering::Relaxed);
        return refuse(&mut stream, ErrorCode::RateLimited);
    }
    let Some(nameplate) = code::parse_nameplate(nameplate) else {
        return refuse(&mut stream, ErrorCode::BadRequest);
    };
    let session = random_u64();
    let (atx, arx) = sync_channel(1);
    let lookup = {
        let mut inner = locked(&state.inner);
        let ended = inner
            .ended
            .get(&nameplate)
            .is_some_and(|t| now.saturating_duration_since(*t) < state.config.ended_kept);
        let busy = state.bridges.load(Ordering::Relaxed) >= state.config.max_bridges;
        let lookup = match inner.hosts.get_mut(&nameplate) {
            None if ended => Lookup::Ended,
            None => Lookup::Unknown,
            Some(h) if h.protocol != protocol => Lookup::Version(h.game.clone()),
            Some(h) if busy || h.pending >= state.config.max_pending_per_host => Lookup::Full,
            Some(h) => {
                h.pending += 1;
                Lookup::Found(h.to_host.clone())
            }
        };
        if matches!(lookup, Lookup::Found(_)) {
            inner.pending.insert(
                session,
                Pending {
                    nameplate: nameplate.clone(),
                    attach: atx,
                },
            );
        }
        lookup
    };
    let to_host = match lookup {
        Lookup::Found(tx) => tx,
        Lookup::Unknown => {
            state.unknown.fetch_add(1, Ordering::Relaxed);
            locked(&state.limiter).unknown(peer.ip(), now);
            locked(&state.global_unknown).try_take(now);
            return refuse(&mut stream, ErrorCode::UnknownCode);
        }
        Lookup::Ended => return refuse(&mut stream, ErrorCode::Ended),
        Lookup::Full => return refuse(&mut stream, ErrorCode::Full),
        Lookup::Version(game) => {
            return wire::send(
                &mut stream,
                &FromRelay::Error {
                    code: ErrorCode::VersionMismatch,
                    game: Some(game),
                },
            );
        }
    };
    let _ = to_host.send(Out::Line(FromRelay::Join { session }));
    let attached = arx.recv_timeout(state.config.attach_wait);
    {
        let mut inner = locked(&state.inner);
        inner.pending.remove(&session);
        if let Some(h) = inner.hosts.get_mut(&nameplate) {
            h.pending = h.pending.saturating_sub(1);
        }
    }
    match attached {
        Ok((host, host_peer)) => bridge(state, stream, peer, host, host_peer, session),
        Err(_) => refuse(&mut stream, ErrorCode::HostUnreachable),
    }
}

fn attach(
    state: &Arc<State>,
    mut stream: TcpStream,
    peer: SocketAddr,
    session: u64,
    token: &str,
) -> io::Result<()> {
    let target = {
        let inner = locked(&state.inner);
        inner
            .pending
            .get(&session)
            .filter(|p| {
                inner
                    .hosts
                    .get(&p.nameplate)
                    .is_some_and(|h| h.token == token)
            })
            .map(|p| p.attach.clone())
    };
    match target {
        Some(tx) => match tx.try_send((stream, peer)) {
            Ok(()) => Ok(()),
            Err(
                std::sync::mpsc::TrySendError::Full((mut stream, _))
                | std::sync::mpsc::TrySendError::Disconnected((mut stream, _)),
            ) => refuse(&mut stream, ErrorCode::BadRequest),
        },
        None => refuse(&mut stream, ErrorCode::BadRequest),
    }
}

/// Tell both ends they are bridged, then copy each way until either ends.
fn bridge(
    state: &Arc<State>,
    mut guest: TcpStream,
    guest_peer: SocketAddr,
    mut host: TcpStream,
    host_peer: SocketAddr,
    session: u64,
) -> io::Result<()> {
    state.bridges.fetch_add(1, Ordering::Relaxed);
    let result = (|| -> io::Result<()> {
        wire::send(
            &mut guest,
            &FromRelay::Bridged {
                session,
                observed: Some(guest_peer.to_string()),
            },
        )?;
        wire::send(
            &mut host,
            &FromRelay::Bridged {
                session,
                observed: Some(host_peer.to_string()),
            },
        )?;
        for s in [&guest, &host] {
            s.set_read_timeout(Some(state.config.idle))?;
            let _ = s.set_nodelay(true);
        }
        let (g, h, st) = (guest.try_clone()?, host.try_clone()?, state.clone());
        let up = std::thread::Builder::new()
            .name("relay-copy".into())
            .spawn(move || copy(&st, g, h))?;
        copy(state, host, guest);
        let _ = up.join();
        Ok(())
    })();
    state.bridges.fetch_sub(1, Ordering::Relaxed);
    result
}

fn copy(state: &State, mut from: TcpStream, mut to: TcpStream) {
    let rate = state.config.session_rate;
    let mut bucket = Bucket::new(rate, (rate / 2.0).max(16384.0), Instant::now());
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let now = Instant::now();
        let wait = bucket
            .take(n as f64, now)
            .max(locked(&state.total).take(n as f64, now));
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        if to.write_all(&buf[..n]).is_err() {
            break;
        }
        state.relayed_bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_lock_still_opens() {
        let mutex = Arc::new(Mutex::new(7));
        let held = mutex.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            panic!("a connection's thread panics holding the lock");
        })
        .join();
        assert!(mutex.is_poisoned());
        assert_eq!(*locked(&mutex), 7);
    }

    #[test]
    fn mapped_addresses_become_ipv4() {
        assert_eq!(
            unmap("[::ffff:203.0.113.7]:5000".parse().unwrap()),
            "203.0.113.7:5000".parse().unwrap()
        );
        let v6: SocketAddr = "[2001:db8::1]:5000".parse().unwrap();
        assert_eq!(unmap(v6), v6);
    }

    #[test]
    fn proxy_lines() {
        assert_eq!(
            proxy_line(b"PROXY TCP4 203.0.113.7 10.0.0.1 51000 7000\r"),
            Some("203.0.113.7:51000".parse().unwrap())
        );
        assert_eq!(
            proxy_line(b"PROXY TCP6 2001:db8::1 2001:db8::2 51000 7000"),
            Some("[2001:db8::1]:51000".parse().unwrap())
        );
        assert_eq!(proxy_line(b"PROXY UNKNOWN"), None);
        assert_eq!(proxy_line(b"{\"op\":\"pong\"}"), None);
    }
}
