//! Hosting a world on the local network. The network side lives here: the
//! listener, the per-connection handshake/login/configuration threads and the
//! writer threads (`conn`), plus the data the configuration phase sends
//! (`data`). The play state is driven by the game thread (`hub`), which only
//! ever exchanges decoded packets with these threads through channels, so a
//! world that is not open to the network pays nothing.
mod advancements;
pub mod attributes;
pub mod border;
pub mod chunks;
mod command_order;
pub mod command_suggestions;
pub mod commands;
pub mod conn;
pub mod containers;
pub mod data;
pub mod discovery;
pub mod entity;
pub mod guest;
pub mod hub;
pub mod menu;
pub mod playerdata;
mod recipes;
pub mod scoreboard;
pub mod tracking;
pub mod waypoints;

pub use conn::{Holding, PlayerSender, flush_all};
use horizon_protocol::packets::play;
use horizon_protocol::{
    GAME_VERSION, PROTOCOL_VERSION, crypto::uuid_string, packets::configuration::ClientInformation,
    text::translatable,
};
use std::{
    collections::HashSet,
    io,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::JoinHandle,
    time::Duration,
};

/// IntegratedServer.MAX_PLAYERS.
pub const MAX_PLAYERS: usize = 8;

/// Messages from the connection threads to the game thread.
pub enum NetEvent {
    /// A client finished configuration and enters the play state.
    Joined(Box<Joined>),
    Packet(u32, Box<play::Serverbound>),
    /// The connection ended (reason for the log).
    Left(u32, String),
    /// PlayerList.disconnectAllPlayersWithProfile: a new login of this
    /// profile replaces the player already in the world.
    Duplicate(u128),
}
pub struct Joined {
    pub conn: u32,
    pub uuid: u128,
    pub name: String,
    /// The profile's properties (skin textures, in online mode).
    pub properties: Vec<horizon_protocol::packets::login::Property>,
    pub info: ClientInformation,
    pub brand: String,
    pub sender: PlayerSender,
    /// Smoothed keep-alive round trip in milliseconds.
    pub latency: Arc<AtomicI32>,
    pub address: SocketAddr,
}
/// Queued for a connection's writer thread.
pub enum Out {
    /// A packet body: VarInt id and payload.
    Packet(Vec<u8>),
    /// Packet bodies held during a tick, in order.
    Packets(Vec<Vec<u8>>),
    /// Send a play disconnect with this component, then close.
    Disconnect(horizon_world::nbt::Tag),
    Close,
}

/// PlayerList.canPlayerLogin for a dedicated server: the disconnect reason
/// (bans, whitelist) for a profile connecting from an address, if refused.
pub type LoginGate =
    Arc<dyn Fn(SocketAddr, u128, &str) -> Option<horizon_world::nbt::Tag> + Send + Sync>;

/// online-mode=true: the key pair the server made at startup
/// (MinecraftServer.initializeKeyPair) and how players are verified.
pub struct OnlineMode {
    pub key: horizon_protocol::rsa::RsaPrivateKey,
    /// The public key as X.509 (PublicKey.getEncoded), sent in the hello.
    pub public_der: Vec<u8>,
    /// prevent-proxy-connections: the session service also checks the address.
    pub prevent_proxy: bool,
}

impl OnlineMode {
    /// MinecraftServer.initializeKeyPair.
    pub fn generate(prevent_proxy: bool) -> Result<Self, String> {
        let key = horizon_protocol::rsa::RsaPrivateKey::generate(1024)
            .map_err(|e| format!("Failed to generate key pair: {e}"))?;
        Ok(Self {
            public_der: key.public_der(),
            key,
            prevent_proxy,
        })
    }
    /// IntegratedServer.initServer (`setUsesAuthentication(true)`): a LAN or
    /// friends world authenticates its guests when the host is signed in;
    /// an offline name hosts in offline mode.
    pub fn lan() -> Option<Arc<Self>> {
        crate::auth::current()?;
        Self::generate(false)
            .map_err(|e| log::warn!("LAN stays in offline mode: {e}"))
            .ok()
            .map(Arc::new)
    }
}

/// The dedicated server's log for the login phase: (thread, level, line)
/// written as vanilla's Log4j would, and the language that turns a
/// disconnect reason into its text (Component.getString).
#[derive(Clone)]
pub struct NetLog {
    pub write: Arc<dyn Fn(&str, &str, &str) + Send + Sync>,
    pub lang: Arc<horizon_protocol::text::Language>,
    /// log-ips: addresses in the log, or "<ip address withheld>".
    pub log_ips: bool,
}

/// How the server side of the network behaves: an integrated server's LAN
/// uses the defaults, a dedicated server takes them from server.properties.
#[derive(Clone)]
pub struct NetConfig {
    /// server-ip (all interfaces when absent).
    pub bind: Option<std::net::IpAddr>,
    /// 0 picks a free port.
    pub port: u16,
    pub motd: String,
    pub max_players: usize,
    /// network-compression-threshold; negative disables compression.
    pub compression_threshold: i32,
    /// rate-limit: packets a second a player may average before being kicked
    /// (RateKickingConnection); 0 is off.
    pub rate_limit: i32,
    /// enable-status: answer status pings.
    pub enable_status: bool,
    pub hide_online_players: bool,
    /// server-icon.png (64x64 PNG bytes).
    pub favicon: Option<Vec<u8>>,
    /// Secure chat (a dedicated server; none on a LAN world).
    pub secure_chat: Option<Arc<SecureChat>>,
    pub accepts_transfers: bool,
    /// LanServerPinger: announce the world on the local network.
    pub announce: bool,
    /// Accept connections from the network. Without it only local ones
    /// (`Lan::connect_local`): an integrated server not open to the LAN.
    pub listen: bool,
    /// Ops bypass a full server (ServerOpList.canBypassPlayerLimit).
    pub bypasses_limit: Option<Arc<dyn Fn(u128) -> bool + Send + Sync>>,
    pub gate: Option<LoginGate>,
    /// Online mode: players are authenticated (a LAN world's when its host is signed in).
    pub online: Option<Arc<OnlineMode>>,
    pub log: Option<NetLog>,
    /// server.properties `resource-pack`: offered to every player during
    /// configuration (a dedicated server's; MinecraftServer.getServerResourcePack).
    pub resource_pack: Option<crate::server::properties::ResourcePackInfo>,
    /// MinecraftServer.serverLinks (a dedicated server's `bug-report-link`):
    /// sent in configuration when non-empty
    /// (ServerConfigurationPacketListenerImpl.startConfiguration).
    pub server_links: Vec<horizon_protocol::packets::configuration::ServerLink>,
}
impl NetConfig {
    /// MinecraftServer.enforceSecureProfile.
    pub fn enforces_secure_chat(&self) -> bool {
        self.secure_chat.as_ref().is_some_and(|s| s.enforced())
    }
    /// IntegratedServer.publishServer.
    pub fn lan(port: u16, motd: String, max_players: usize) -> Self {
        Self {
            bind: None,
            port,
            motd,
            max_players,
            compression_threshold: 256,
            rate_limit: 0,
            enable_status: true,
            hide_online_players: false,
            favicon: None,
            secure_chat: None,
            accepts_transfers: false,
            announce: true,
            listen: true,
            bypasses_limit: None,
            gate: None,
            online: None,
            log: None,
            resource_pack: None,
            server_links: Vec::new(),
        }
    }
}

/// A dedicated server's secure chat (DedicatedServer.enforceSecureProfile,
/// Services.profileKeySignatureValidator): the services' profile-key keys,
/// fetched in the background from startup and waited for by their first
/// use, as authlib's RetryableFetch does.
pub struct SecureChat {
    /// enforce-secure-profile, in online mode.
    pub enforce: bool,
    keys: std::sync::OnceLock<Vec<horizon_protocol::rsa::RsaPublicKey>>,
    fetch: Mutex<Option<JoinHandle<Vec<horizon_protocol::rsa::RsaPublicKey>>>>,
}

impl SecureChat {
    pub fn new(
        enforce: bool,
        fetch: impl FnOnce() -> Vec<horizon_protocol::rsa::RsaPublicKey> + Send + 'static,
    ) -> Self {
        let handle = std::thread::Builder::new()
            .name("Services keys".into())
            .spawn(fetch)
            .ok();
        Self {
            enforce,
            keys: std::sync::OnceLock::new(),
            fetch: Mutex::new(handle),
        }
    }
    /// The keys players' chat keys must be signed by (none when the
    /// services couldn't be reached).
    pub fn keys(&self) -> &[horizon_protocol::rsa::RsaPublicKey] {
        self.keys.get_or_init(|| {
            let handle = self.fetch.lock().ok().and_then(|mut h| h.take());
            handle.and_then(|h| h.join().ok()).unwrap_or_default()
        })
    }
    /// enforceSecureProfile: enforced, and able to check keys.
    pub fn enforced(&self) -> bool {
        self.enforce && !self.keys().is_empty()
    }
}

/// State shared with the connection threads.
pub struct Shared {
    pub sync: Arc<data::SyncData>,
    pub events: Sender<NetEvent>,
    pub config: NetConfig,
    motd: Mutex<String>,
    max_players: AtomicUsize,
    /// PlayerList.players in join order: profile id and name.
    online: Mutex<Vec<(u128, String)>>,
    /// Profiles held by the host (the LAN host's own name).
    reserved: Mutex<HashSet<u128>>,
    next: AtomicU32,
    stop: AtomicBool,
    random: Mutex<u64>,
}
impl Shared {
    pub fn next_id(&self) -> u32 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
    /// PlayerList.canPlayerLogin then disconnectAllPlayersWithProfile: the
    /// refusal reason, or None once the profile holds a place. A profile
    /// already playing is kicked and its place awaited.
    pub fn try_admit(
        &self,
        address: SocketAddr,
        uuid: u128,
        name: &str,
    ) -> Option<horizon_world::nbt::Tag> {
        if let Some(gate) = &self.config.gate
            && let Some(reason) = gate(address, uuid, name)
        {
            return Some(reason);
        }
        if self.reserved.lock().unwrap().contains(&uuid) {
            return Some(translatable("multiplayer.disconnect.duplicate_login", &[]));
        }
        {
            let online = self.online.lock().unwrap();
            let bypass = self.config.bypasses_limit.as_ref().is_some_and(|b| b(uuid));
            let taken = online.len() + self.reserved.lock().unwrap().len();
            if taken >= self.max_players() && !bypass {
                return Some(translatable("multiplayer.disconnect.server_full", &[]));
            }
        }
        if self.is_online(uuid) {
            let _ = self.events.send(NetEvent::Duplicate(uuid));
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while self.is_online(uuid) {
                if std::time::Instant::now() > deadline {
                    return Some(translatable("multiplayer.disconnect.slow_login", &[]));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.online.lock().unwrap().push((uuid, name.to_owned()));
        None
    }
    fn is_online(&self, uuid: u128) -> bool {
        self.online.lock().unwrap().iter().any(|(u, _)| *u == uuid)
    }
    pub fn motd(&self) -> String {
        self.motd.lock().unwrap().clone()
    }
    pub fn max_players(&self) -> usize {
        self.max_players.load(Ordering::Relaxed)
    }
    pub fn release(&self, uuid: u128) {
        self.online.lock().unwrap().retain(|(u, _)| *u != uuid);
    }
    pub fn online(&self) -> usize {
        self.online.lock().unwrap().len() + self.reserved.lock().unwrap().len()
    }
    fn next_random(&self, bound: usize) -> usize {
        let mut r = self.random.lock().unwrap();
        *r = r
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*r >> 33) as usize) % bound.max(1)
    }
    /// The status response (MinecraftServer.buildServerStatus encoded with
    /// ServerStatus.CODEC): description, players with up to 12 shuffled
    /// names, version, favicon and enforcesSecureChat when true.
    pub fn status_json(&self) -> String {
        use crate::server::json::Json;
        let motd = self.motd.lock().unwrap().clone();
        let online: Vec<(u128, String)> = self.online.lock().unwrap().clone();
        let count = self.online();
        let mut players = vec![
            (
                "max".to_owned(),
                Json::Number(self.max_players().to_string()),
            ),
            ("online".to_owned(), Json::Number(count.to_string())),
        ];
        if !self.config.hide_online_players && !online.is_empty() {
            let size = online.len().min(12);
            let offset = self.next_random(online.len() - size + 1);
            let mut sample: Vec<(u128, String)> = online[offset..offset + size].to_vec();
            for i in (1..sample.len()).rev() {
                let j = self.next_random(i + 1);
                sample.swap(i, j);
            }
            players.push((
                "sample".to_owned(),
                Json::Array(
                    sample
                        .into_iter()
                        .map(|(u, n)| {
                            Json::Object(vec![
                                ("id".to_owned(), Json::String(uuid_string(u))),
                                ("name".to_owned(), Json::String(n)),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        let mut status = vec![
            ("description".to_owned(), Json::String(motd)),
            ("players".to_owned(), Json::Object(players)),
            (
                "version".to_owned(),
                Json::Object(vec![
                    ("name".to_owned(), Json::String(GAME_VERSION.to_owned())),
                    (
                        "protocol".to_owned(),
                        Json::Number(PROTOCOL_VERSION.to_string()),
                    ),
                ]),
            ),
        ];
        if let Some(icon) = &self.config.favicon {
            status.push((
                "favicon".to_owned(),
                Json::String(format!("data:image/png;base64,{}", base64(icon))),
            ));
        }
        if self.config.enforces_secure_chat() {
            status.push(("enforcesSecureChat".to_owned(), Json::Bool(true)));
        }
        Json::Object(status).compact()
    }
}

/// java.util.Base64.getEncoder (padded, no line breaks).
pub fn base64(data: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(char::from(A[(n >> 18) as usize & 63]));
        out.push(char::from(A[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(A[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(A[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// A running LAN server.
pub struct Lan {
    pub shared: Arc<Shared>,
    events: Receiver<NetEvent>,
    pub port: u16,
    listener: Option<JoinHandle<()>>,
    /// Stops the accept loop alone: local connections stay.
    listener_stop: Arc<AtomicBool>,
    /// The multicast announcement (LanServerPinger); absent when the socket failed.
    _pinger: Option<discovery::Pinger>,
}
impl Lan {
    /// Listen on `port` (0 picks a free one).
    pub fn start(
        sync: Arc<data::SyncData>,
        port: u16,
        motd: String,
        max_players: usize,
    ) -> io::Result<Self> {
        Self::start_with(sync, NetConfig::lan(port, motd, max_players))
    }
    /// ServerConnectionListener.startTcpServerListener with a configuration.
    pub fn start_with(sync: Arc<data::SyncData>, config: NetConfig) -> io::Result<Self> {
        let (tx, events) = channel();
        let (listen, port, announce) = (config.listen, config.port, config.announce);
        let shared = Arc::new(Shared {
            sync,
            events: tx,
            motd: Mutex::new(config.motd.clone()),
            max_players: AtomicUsize::new(config.max_players),
            online: Mutex::new(Vec::new()),
            reserved: Mutex::new(HashSet::new()),
            next: AtomicU32::new(1),
            stop: AtomicBool::new(false),
            random: Mutex::new(crate::server::java_properties::now_millis() as u64 | 1),
            config,
        });
        let mut lan = Self {
            shared,
            events,
            port: 0,
            listener: None,
            listener_stop: Arc::new(AtomicBool::new(false)),
            _pinger: None,
        };
        if listen {
            lan.listen(port, announce)?;
        }
        Ok(lan)
    }
    /// Start accepting connections on `port` (0 picks a free one), and
    /// announce the world on the local network when `announce`
    /// (`IntegratedServer.publishServer`'s LanServerPinger). Returns the port.
    pub fn listen(&mut self, port: u16, announce: bool) -> io::Result<u16> {
        if self.listener.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "already listening",
            ));
        }
        let listener = Self::bind(self.shared.config.bind, port)?;
        self.listen_on(listener, announce)
    }
    /// The socket [`Lan::listen`] accepts on, bound ahead so that a failure
    /// can be reported before the hub is at hand.
    pub fn bind(address: Option<std::net::IpAddr>, port: u16) -> io::Result<TcpListener> {
        let bind = address.unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        let listener = TcpListener::bind((bind, port))?;
        listener.set_nonblocking(true)?;
        Ok(listener)
    }
    /// [`Lan::listen`] on a socket already bound.
    pub fn listen_on(&mut self, listener: TcpListener, announce: bool) -> io::Result<u16> {
        if self.listener.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "already listening",
            ));
        }
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let (accept_shared, accept_stop) = (self.shared.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("horizon-lan-accept".into())
            .spawn(move || accept_loop(listener, accept_shared, &accept_stop))?;
        self._pinger = if announce {
            discovery::Pinger::start(self.shared.motd(), port).ok()
        } else {
            None
        };
        self.listener = Some(thread);
        self.listener_stop = stop;
        self.port = port;
        Ok(port)
    }
    /// Stop accepting connections and announcing; connections already in
    /// (the local one included) stay.
    pub fn stop_listening(&mut self) {
        self.listener_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.listener.take() {
            let _ = t.join();
        }
        self._pinger = None;
        self.port = 0;
    }
    /// Whether connections from the network are accepted.
    pub fn listening(&self) -> bool {
        self.listener.is_some()
    }
    /// `Connection.connectToLocalServer`: a connection through memory, its
    /// server end served on its own thread like an accepted socket.
    pub fn connect_local(&self) -> io::Result<horizon_protocol::connection::Pipe> {
        let (client, server) = horizon_protocol::connection::Pipe::pair();
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name("horizon-local-conn".into())
            .spawn(move || {
                if let Err(e) = conn::serve_local(server, shared)
                    && std::env::var_os("HORIZON_NET_DEBUG").is_some()
                {
                    log::warn!("[lan] local: {e}");
                }
            })?;
        Ok(client)
    }
    /// Everything the connection threads reported since the last call.
    pub fn poll(&self) -> Vec<NetEvent> {
        self.events.try_iter().collect()
    }
    /// Reserve a uuid for the host so a guest cannot take the name.
    pub fn reserve(&self, uuid: u128) {
        self.shared.reserved.lock().unwrap().insert(uuid);
    }
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.stop_listening();
    }
}
impl Drop for Lan {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, stop: &AtomicBool) {
    while !shared.stop.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, addr)) => {
                let shared = shared.clone();
                let _ = std::thread::Builder::new()
                    .name("horizon-lan-conn".into())
                    .spawn(move || handle(stream, addr, shared));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
fn handle(stream: TcpStream, addr: SocketAddr, shared: Arc<Shared>) {
    let _ = stream.set_nonblocking(false);
    if let Err(e) = conn::serve(stream, addr, shared)
        && std::env::var_os("HORIZON_NET_DEBUG").is_some()
    {
        log::warn!("[lan] {addr}: {e}");
    }
}

/// HttpUtil.getAvailablePort: a free local port.
pub fn available_port() -> u16 {
    TcpListener::bind(("0.0.0.0", 0))
        .and_then(|l| l.local_addr())
        .map_or(25565, |a| a.port())
}
