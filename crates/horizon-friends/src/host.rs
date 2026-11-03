//! The host's end. It listens on its direct port, works out how it can be
//! reached (interfaces, a router mapping, STUN) and puts that with a fresh
//! secret into the invite code: guests with the code dial it directly, no
//! server involved. When a relay is configured (the player's own, or any
//! they trust), it also registers there for a short code and serves relayed
//! sessions (with their own direct race, then the relay as fallback).
//! Every session ends at the integrated server's port. Everything runs on
//! its own threads; the game polls [`Host::poll`] and [`Host::state`].
use crate::{
    Code, PathMode,
    crypto::{self, Role},
    direct,
    invite::{Invite, RelayRef},
    reach::{self, Reach, Verdict},
    resolve_relay, session,
    wire::{self, FromRelay, ToRelay},
};
use std::{
    collections::HashMap,
    io::{self, BufReader},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Wrong short codes tried at the relay before the host retires it.
pub const MAX_WRONG: u32 = 5;
/// Sessions open at once (IntegratedServer.MAX_PLAYERS).
pub const MAX_SESSIONS: usize = 8;
/// The direct port asked for first, so a port forwarded by hand keeps
/// working from one session to the next (`HORIZON_FRIENDS_PORT`).
pub const DEFAULT_PORT: u16 = 25580;
/// Code handshakes in progress at once (scanners can't pile them up).
const MAX_HANDSHAKES: usize = 8;
/// The most an inbound stream's opening (hello, key exchange, confirmation)
/// may take, however slowly its bytes come.
const HANDSHAKE: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct HostConfig {
    /// The integrated server's port (Open to LAN's).
    pub lan: SocketAddr,
    pub protocol: i32,
    pub game: String,
    /// A relay to register with as well (short code, relay fallback).
    pub relay: Option<String>,
    pub path: PathMode,
    /// The direct port to ask for (0: any).
    pub port: u16,
    pub reach: reach::Options,
}

/// What the host's game shows.
#[derive(Clone, Debug, Default)]
pub struct HostState {
    /// The invite code; None while the host is still finding its addresses.
    pub invite: Option<String>,
    /// The relay's short code, when registered with one.
    pub short: Option<Code>,
    /// Who can join, and what to do when friends on the internet can't.
    pub summary: String,
    pub verdict: Option<Verdict>,
    /// Why the relay isn't serving, when one is configured.
    pub relay_problem: Option<String>,
    pub port: u16,
}

#[derive(Debug)]
pub enum HostEvent {
    /// A new invite code (the first, or after the relay's short code changed).
    Code(String),
    /// A guest's session is carried directly or through the relay.
    Connected {
        session: u64,
        direct: bool,
    },
    Disconnected {
        session: u64,
    },
    /// Someone tried a wrong code.
    WrongCode {
        count: u32,
    },
}

struct Shared {
    config: HostConfig,
    stop: AtomicBool,
    state: Mutex<HostState>,
    events: Mutex<Sender<HostEvent>>,
    /// The invite (endpoints and secret), rebuilt when the relay part changes.
    invite: Mutex<Option<Invite>>,
    /// The current relay control connection (shut down to stop).
    control: Mutex<Option<TcpStream>>,
    /// Inbound direct streams waiting for their relayed session.
    routes: Mutex<HashMap<u64, (Sender<TcpStream>, Instant)>>,
    sessions: AtomicUsize,
    handshakes: AtomicUsize,
    wrong: AtomicU32,
    relay_wrong: AtomicU32,
    reach: Mutex<Option<Reach>>,
}
impl Shared {
    fn emit(&self, e: HostEvent) {
        let _ = self.events.lock().unwrap().send(e);
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    /// Rebuild the code from the invite and the relay's short code.
    fn publish(&self, short: Option<Code>) {
        let text = {
            let mut invite = self.invite.lock().unwrap();
            let Some(inv) = invite.as_mut() else { return };
            inv.relay = match (&short, &self.config.relay) {
                (Some(code), Some(address)) => Some(RelayRef {
                    address: address.clone(),
                    code: code.clone(),
                }),
                _ => None,
            };
            inv.encode()
        };
        {
            let mut s = self.state.lock().unwrap();
            if s.invite.as_deref() == Some(&text) && s.short == short {
                return;
            }
            s.invite = Some(text.clone());
            s.short = short;
        }
        self.emit(HostEvent::Code(text));
    }
}

pub struct Host {
    shared: Arc<Shared>,
    events: Receiver<HostEvent>,
    threads: Vec<JoinHandle<()>>,
}

impl Host {
    /// Start sharing in the background: the code arrives as an event once
    /// the host knows its addresses (a few seconds).
    pub fn start(config: HostConfig) -> io::Result<Self> {
        let (tx, events) = channel();
        let (listeners, port) = direct::listen(config.port).or_else(|_| direct::listen(0))?;
        let shared = Arc::new(Shared {
            config,
            stop: AtomicBool::new(false),
            state: Mutex::new(HostState {
                port,
                ..HostState::default()
            }),
            events: Mutex::new(tx),
            invite: Mutex::new(None),
            control: Mutex::new(None),
            routes: Mutex::new(HashMap::new()),
            sessions: AtomicUsize::new(0),
            handshakes: AtomicUsize::new(0),
            wrong: AtomicU32::new(0),
            relay_wrong: AtomicU32::new(0),
            reach: Mutex::new(None),
        });
        let mut threads = Vec::new();
        let s = shared.clone();
        threads.push(
            std::thread::Builder::new()
                .name("horizon-friends-listen".into())
                .spawn(move || route_inbound(listeners, s))?,
        );
        let s = shared.clone();
        threads.push(
            std::thread::Builder::new()
                .name("horizon-friends-reach".into())
                .spawn(move || share(s, port))?,
        );
        Ok(Self {
            shared,
            events,
            threads,
        })
    }
    pub fn state(&self) -> HostState {
        self.shared.state.lock().unwrap().clone()
    }
    pub fn poll(&self) -> Vec<HostEvent> {
        self.events.try_iter().collect()
    }
    /// Guests whose sessions are open (connecting or playing).
    pub fn sessions(&self) -> usize {
        self.shared.sessions.load(Ordering::Relaxed)
    }
    /// End the code: close the direct port, delete the router mapping, leave
    /// the relay. Sessions already playing go on until the integrated server
    /// closes them.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(c) = self.shared.control.lock().unwrap().take() {
            let _ = c.shutdown(Shutdown::Both);
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        if let Some(r) = self.shared.reach.lock().unwrap().as_mut() {
            r.release();
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Find the addresses, publish the code, keep the router mapping alive,
/// serve the relay when there is one.
fn share(shared: Arc<Shared>, port: u16) {
    let config = &shared.config;
    let reach = if config.path == PathMode::RelayOnly {
        Reach {
            port,
            ..Reach::default()
        }
    } else {
        Reach::discover(port, &config.reach)
    };
    let endpoints = if config.path == PathMode::RelayOnly {
        Vec::new()
    } else {
        reach.endpoints()
    };
    let invite = match Invite::new(port, endpoints, None) {
        Ok(i) => i,
        Err(_) => return,
    };
    {
        let mut s = shared.state.lock().unwrap();
        s.summary = if config.path == PathMode::RelayOnly {
            "Friends join through the relay only.".into()
        } else {
            reach.summary()
        };
        s.verdict = Some(reach.verdict());
    }
    *shared.invite.lock().unwrap() = Some(invite);
    *shared.reach.lock().unwrap() = Some(reach);
    if shared.stopped() {
        return;
    }
    shared.publish(None);
    let relay = match &config.relay {
        Some(r) => {
            let s = shared.clone();
            let r = r.clone();
            std::thread::Builder::new()
                .name("horizon-friends-relay".into())
                .spawn(move || run_relay(s, r))
                .ok()
        }
        None => None,
    };
    // Renew the router mapping at half-lease until stopped.
    while !shared.stopped() {
        std::thread::sleep(Duration::from_millis(100));
        let mut reach = shared.reach.lock().unwrap();
        if let Some(m) = reach.as_mut().and_then(|r| r.mapping.as_mut())
            && m.due()
            && m.renew().is_err()
        {
            // Try again in a minute rather than hammering the router.
            m.renewed = Instant::now() - m.lifetime / 2 + Duration::from_secs(60);
        }
    }
    if let Some(t) = relay {
        let _ = t.join();
    }
}

/// Inbound streams on the direct port: a relayed session's direct race, or
/// a guest with the code.
fn route_inbound(listeners: Vec<TcpListener>, shared: Arc<Shared>) {
    while !shared.stopped() {
        let mut idle = true;
        for l in &listeners {
            let Ok((mut s, _)) = l.accept() else { continue };
            idle = false;
            if shared.handshakes.load(Ordering::Relaxed) >= MAX_HANDSHAKES {
                continue;
            }
            shared.handshakes.fetch_add(1, Ordering::Relaxed);
            let shared = shared.clone();
            let _ = std::thread::Builder::new()
                .name("horizon-friends-inbound".into())
                .spawn(move || {
                    let _ = s.set_nonblocking(false);
                    let Ok(deadline) = crate::deadline(&s, HANDSHAKE) else {
                        shared.handshakes.fetch_sub(1, Ordering::Relaxed);
                        return;
                    };
                    match direct::read_hello(&mut s) {
                        Ok(direct::Hello::Relayed(session)) => {
                            shared.handshakes.fetch_sub(1, Ordering::Relaxed);
                            let route = {
                                let mut routes = shared.routes.lock().unwrap();
                                routes.retain(|_, (_, until)| *until > Instant::now());
                                routes.get(&session).map(|(tx, _)| tx.clone())
                            };
                            if let Some(tx) = route
                                && direct::answer(&mut s, session).is_ok()
                            {
                                let _ = tx.send(s);
                            }
                        }
                        Ok(direct::Hello::Code(session)) => {
                            serve_code(&shared, s, session, deadline)
                        }
                        Err(_) => {
                            shared.handshakes.fetch_sub(1, Ordering::Relaxed);
                        }
                    }
                });
        }
        if idle {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A guest who dialled an endpoint from the code: agree on the key from
/// the code's secret, then carry the session (`deadline` bounds the
/// handshake, not the game).
fn serve_code(shared: &Arc<Shared>, mut s: TcpStream, session: u64, deadline: crate::Deadline) {
    let secret = shared
        .invite
        .lock()
        .unwrap()
        .as_ref()
        .map(Invite::secret_text);
    let secured = (|| -> io::Result<session::Secure> {
        let secret = secret.ok_or_else(|| io::Error::other("not ready"))?;
        direct::answer_code(&mut s, session)?;
        let key = crypto::exchange(&mut s, &secret, "direct", session)?;
        session::secure(s, &key, Role::Host)
    })();
    drop(deadline);
    shared.handshakes.fetch_sub(1, Ordering::Relaxed);
    match secured {
        Ok(secure) => carry(shared, session, secure, true),
        Err(e) if crypto::is_wrong_code(&e) => {
            let count = shared.wrong.fetch_add(1, Ordering::Relaxed) + 1;
            shared.emit(HostEvent::WrongCode { count });
        }
        Err(_) => {}
    }
}

/// Connect a settled session to the integrated server and carry it.
fn carry(shared: &Arc<Shared>, session: u64, (r, w): session::Secure, direct: bool) {
    if shared.sessions.fetch_add(1, Ordering::Relaxed) >= MAX_SESSIONS {
        shared.sessions.fetch_sub(1, Ordering::Relaxed);
        let _ = r.get_ref().shutdown(Shutdown::Both);
        return;
    }
    let Ok(server) = TcpStream::connect_timeout(&shared.config.lan, Duration::from_secs(5)) else {
        let _ = r.get_ref().shutdown(Shutdown::Both);
        shared.sessions.fetch_sub(1, Ordering::Relaxed);
        return;
    };
    shared.emit(HostEvent::Connected { session, direct });
    if let Ok(t) = crate::pump(server, r, w) {
        let _ = t.join();
    }
    shared.sessions.fetch_sub(1, Ordering::Relaxed);
    shared.emit(HostEvent::Disconnected { session });
}

// ----- The relay (optional) ---------------------------------------------------------------------

fn connect_relay(relay: &str, port: Option<u16>) -> io::Result<(TcpStream, SocketAddr)> {
    let wait = Duration::from_secs(5);
    let mut last = io::Error::other("no address");
    let mut timed_out = Vec::new();
    for address in resolve_relay(relay)? {
        match direct::connect_from(port, address, wait) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok((s, address));
            }
            Err(e) => {
                if port.is_some() && e.kind() == io::ErrorKind::TimedOut {
                    timed_out.push(address);
                }
                last = e;
            }
        }
    }
    // A relay on Windows ignores a new connection from the address of one it
    // closed less than its TIME_WAIT ago (the last guest's attach from the
    // direct port), where Linux takes it: those get one more try from any
    // port, once every address had its try from the direct port.
    for address in timed_out {
        match direct::connect_from(None, address, wait) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok((s, address));
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Register, serve the control connection, register again when it drops
/// (a new short code) until stopped.
fn run_relay(shared: Arc<Shared>, relay: String) {
    let mut backoff = Duration::from_secs(1);
    while !shared.stopped() {
        match register(&shared, &relay) {
            Ok(()) => {
                backoff = Duration::from_secs(1);
                shared.state.lock().unwrap().relay_problem = None;
            }
            Err(e) => {
                if shared.stopped() {
                    break;
                }
                shared.state.lock().unwrap().relay_problem =
                    Some(format!("Couldn't reach the relay {relay}: {e}"));
                shared.publish(None);
                let until = Instant::now() + backoff;
                while Instant::now() < until && !shared.stopped() {
                    std::thread::sleep(Duration::from_millis(50));
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

fn register(shared: &Arc<Shared>, relay_url: &str) -> io::Result<()> {
    let config = &shared.config;
    let port = Some(shared.state.lock().unwrap().port);
    // The control connection uses any port: the direct port stays free for
    // the attach connections, whose observed address is the one that counts.
    let (mut control, relay) = connect_relay(relay_url, None)?;
    control.set_read_timeout(Some(Duration::from_secs(70)))?;
    wire::send(
        &mut control,
        &ToRelay::Host {
            v: wire::VERSION,
            protocol: config.protocol,
            game: config.game.clone(),
        },
    )?;
    let mut reader = BufReader::new(control.try_clone()?);
    let (nameplate, token) = match wire::recv_buffered::<_, FromRelay>(&mut reader)? {
        FromRelay::Registered {
            nameplate, token, ..
        } => (nameplate, token),
        FromRelay::Error { code, .. } => {
            return Err(io::Error::other(format!("it refused ({})", code.as_str())));
        }
        other => return Err(io::Error::other(format!("unexpected answer {other:?}"))),
    };
    let code = Code::new(&nameplate)?;
    *shared.control.lock().unwrap() = Some(control.try_clone()?);
    if shared.stopped() {
        let _ = control.shutdown(Shutdown::Both);
        return Ok(());
    }
    shared.relay_wrong.store(0, Ordering::Relaxed);
    shared.publish(Some(code.clone()));
    loop {
        let message = match wire::recv_buffered::<_, FromRelay>(&mut reader) {
            Ok(m) => m,
            Err(e) => {
                shared.control.lock().unwrap().take();
                return if shared.stopped()
                    || shared.relay_wrong.load(Ordering::Relaxed) >= MAX_WRONG
                {
                    Ok(())
                } else {
                    Err(e)
                };
            }
        };
        match message {
            FromRelay::Ping => wire::send(&mut control, &ToRelay::Pong)?,
            FromRelay::Join { session } => {
                if shared.sessions.load(Ordering::Relaxed) >= MAX_SESSIONS {
                    continue;
                }
                let (shared, code, token, control, relay_url) = (
                    shared.clone(),
                    code.clone(),
                    token.clone(),
                    control.try_clone()?,
                    relay_url.to_owned(),
                );
                let _ = std::thread::Builder::new()
                    .name("horizon-friends-session".into())
                    .spawn(move || {
                        serve_relayed(
                            &shared, &relay_url, &code, &token, session, port, relay, &control,
                        )
                    });
            }
            _ => {}
        }
    }
}

/// One guest through the relay: attach, agree on the key from the short
/// code, settle on a direct or relayed stream, carry it.
#[allow(clippy::too_many_arguments)]
fn serve_relayed(
    shared: &Arc<Shared>,
    relay_url: &str,
    code: &Code,
    token: &str,
    session: u64,
    port: Option<u16>,
    relay: SocketAddr,
    control: &TcpStream,
) {
    let config = &shared.config;
    let Ok((mut stream, _)) = connect_relay(relay_url, port) else {
        return;
    };
    // What the relay sees of an attach from another port is no address of
    // this host's: nothing listens there.
    let from_port = stream.local_addr().ok().map(|a| a.port()) == port;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let attach = ToRelay::Attach {
        v: wire::VERSION,
        session,
        token: token.to_owned(),
    };
    if wire::send(&mut stream, &attach).is_err() {
        return;
    }
    let observed = match wire::recv::<_, FromRelay>(&mut stream) {
        Ok(FromRelay::Bridged { observed, .. }) => observed
            .filter(|_| from_port)
            .and_then(|o| o.parse::<SocketAddr>().ok()),
        _ => return,
    };
    let Ok(key) = crypto::exchange(&mut stream, &code.secret, &code.nameplate, session) else {
        return;
    };
    let relayed = match session::secure(stream, &key, Role::Host) {
        Ok(s) => s,
        Err(e) => {
            if crypto::is_wrong_code(&e) {
                let count = shared.relay_wrong.fetch_add(1, Ordering::Relaxed) + 1;
                shared.emit(HostEvent::WrongCode { count });
                if count >= MAX_WRONG {
                    // Retire the short code: dropping the control connection
                    // ends it at the relay, and `run_relay` registers anew.
                    let _ = control.shutdown(Shutdown::Both);
                }
            }
            return;
        }
    };
    let candidates = port.map_or_else(Vec::new, |p| direct::candidates(p, Some(relay), observed));
    let routes = shared.clone();
    let settled = session::settle(
        relayed,
        &key,
        Role::Host,
        config.path,
        session,
        port,
        candidates,
        Box::new(move |tx, deadline, _stop| {
            routes
                .routes
                .lock()
                .unwrap()
                .insert(session, (tx, deadline + Duration::from_millis(500)));
        }),
    );
    shared.routes.lock().unwrap().remove(&session);
    if let Ok(settled) = settled {
        carry(shared, session, settled.secure, settled.direct);
    }
}
