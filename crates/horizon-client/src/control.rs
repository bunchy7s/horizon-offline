//! Local control socket: lets a script or `horizon-ctl` drive a running game.
//!
//! Each connection sends commands as text lines and receives one JSON line per
//! command. Commands are executed on the game thread between frames; replies
//! that depend on a later frame (screenshots, benchmarks) arrive when ready.
//!
//! Two transports carry the same lines. On Linux and macOS the endpoint is a
//! Unix domain socket. Where those don't exist (Windows), or when the endpoint
//! path ends in `.json`, the game listens on a random localhost TCP port and
//! writes `{"transport":"tcp","port":N,"token":"..."}` to the path, readable
//! only by its user; a client's first line must then be `auth TOKEN`.
use crate::block_entity_render::StaticCacheFrameStats;
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};

/// A connection to the control endpoint, over either transport.
pub enum Stream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Stream {
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(match self {
            #[cfg(unix)]
            Self::Unix(s) => Self::Unix(s.try_clone()?),
            Self::Tcp(s) => Self::Tcp(s.try_clone()?),
        })
    }
    /// A read timeout (None waits forever).
    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.set_read_timeout(timeout),
            Self::Tcp(s) => s.set_read_timeout(timeout),
        }
    }
}
impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.read(buf),
            Self::Tcp(s) => s.read(buf),
        }
    }
}
impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.write(buf),
            Self::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}

/// Whether the endpoint at `path` is a TCP port described by a JSON file.
pub fn uses_tcp(path: &Path) -> bool {
    !cfg!(unix) || path.extension().is_some_and(|e| e == "json")
}

/// The TCP endpoint file: (port, token).
fn read_endpoint(path: &Path) -> io::Result<(u16, String)> {
    let text = std::fs::read_to_string(path)?;
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "not a control endpoint file");
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| bad())?;
    let port = v["port"]
        .as_u64()
        .and_then(|p| u16::try_from(p).ok())
        .ok_or_else(bad)?;
    let token = v["token"].as_str().ok_or_else(bad)?.to_owned();
    Ok((port, token))
}

/// Connect to a running game's control endpoint (and authenticate over TCP).
pub fn connect(path: &Path) -> io::Result<Stream> {
    #[cfg(unix)]
    if !uses_tcp(path) {
        return UnixStream::connect(path).map(Stream::Unix);
    }
    let (port, token) = read_endpoint(path)?;
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    stream.set_nodelay(true)?;
    writeln!(stream, "auth {token}")?;
    let mut reply = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut reply)?;
    if !reply.contains("\"ok\"") {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("the game refused the token: {}", reply.trim()),
        ));
    }
    Ok(Stream::Tcp(stream))
}

/// 128 random bits as hex, from the standard library's OS-seeded hasher keys.
fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..2)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            h.write_u32(std::process::id());
            format!("{:016x}", h.finish())
        })
        .collect()
}

/// Write the endpoint file so that only this user can read it.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_bytes())
}

/// Constant-time comparison of the presented token.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// One command line from a client, with the channel for its JSON reply.
pub struct Request {
    pub line: String,
    pub reply: mpsc::Sender<String>,
}

pub struct Server {
    path: PathBuf,
    requests: mpsc::Receiver<Request>,
    sender: mpsc::Sender<Request>,
    /// Clients connected now (an MCP server keeps one open; horizon-ctl one per command).
    clients: Arc<AtomicUsize>,
    /// Set when the server is dropped: the listener stops accepting and exits.
    closed: Arc<AtomicBool>,
    /// The TCP port, to wake the listener when closing.
    port: Option<u16>,
}

/// `$HORIZON_CONTROL`, else `horizon-control.sock` in the user's runtime
/// directory (`horizon-control.json` in the temporary directory on Windows).
pub fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("HORIZON_CONTROL") {
        return path.into();
    }
    if !cfg!(unix) {
        return std::env::temp_dir().join("horizon-control.json");
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    dir.join("horizon-control.sock")
}

impl Server {
    pub fn start(path: &Path) -> io::Result<Self> {
        // An endpoint left by a crashed game refuses connections; replace it.
        if path.exists() && connect(path).is_err() {
            std::fs::remove_file(path)?;
        }
        let (sender, requests) = mpsc::channel();
        let local = sender.clone();
        let clients = Arc::new(AtomicUsize::new(0));
        let counted = clients.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let mut tcp_port = None;
        if uses_tcp(path) {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
            let token = random_token();
            let port = listener.local_addr()?.port();
            tcp_port = Some(port);
            write_private(
                path,
                &serde_json::json!({ "transport": "tcp", "port": port, "token": token })
                    .to_string(),
            )?;
            spawn_listener(
                move || listener.accept().map(|(s, _)| s),
                sender,
                counted,
                closed.clone(),
                move |stream: TcpStream| {
                    let _ = stream.set_nodelay(true);
                    let mut stream = Stream::Tcp(stream);
                    authenticate(&mut stream, &token).then_some(stream)
                },
            )?;
        } else {
            #[cfg(unix)]
            {
                let listener = UnixListener::bind(path)?;
                spawn_listener(
                    move || listener.accept().map(|(s, _)| s),
                    sender,
                    counted,
                    closed.clone(),
                    |s| Some(Stream::Unix(s)),
                )?;
            }
        }
        Ok(Self {
            path: path.to_owned(),
            requests,
            sender: local,
            clients,
            closed,
            port: tcp_port,
        })
    }
    /// Commands from inside the process only (`--benchmark` without `--control`).
    pub fn local() -> Self {
        let (sender, requests) = mpsc::channel();
        Self {
            path: PathBuf::new(),
            requests,
            sender,
            clients: Arc::default(),
            closed: Arc::default(),
            port: None,
        }
    }
    /// How many clients are connected.
    pub fn clients(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }
    /// Sends commands as a socket client would, from another thread.
    pub fn sender(&self) -> mpsc::Sender<Request> {
        self.sender.clone()
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn try_recv(&self) -> Option<Request> {
        self.requests.try_recv().ok()
    }
}

/// Accept connections on a thread, one more thread per client.
fn spawn_listener<S: Send + 'static>(
    mut accept_next: impl FnMut() -> io::Result<S> + Send + 'static,
    sender: mpsc::Sender<Request>,
    clients: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
    accept: impl Fn(S) -> Option<Stream> + Send + Sync + 'static,
) -> io::Result<()> {
    let accept = std::sync::Arc::new(accept);
    thread::Builder::new()
        .name("horizon-control".into())
        .spawn(move || {
            loop {
                let next = accept_next();
                if closed.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = next else {
                    thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                };
                let sender = sender.clone();
                let accept = accept.clone();
                let clients = clients.clone();
                let _ = thread::Builder::new()
                    .name("horizon-control-client".into())
                    .spawn(move || {
                        if let Some(stream) = accept(stream) {
                            clients.fetch_add(1, Ordering::Relaxed);
                            serve(stream, &sender);
                            clients.fetch_sub(1, Ordering::Relaxed);
                        }
                    });
            }
        })?;
    Ok(())
}

/// The first line of a TCP client must be `auth TOKEN`.
fn authenticate(stream: &mut Stream, token: &str) -> bool {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    // Read the line byte by byte so nothing after it is buffered away.
    while line.len() < 256 {
        match stream.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => break,
            Ok(1) => line.push(byte[0]),
            _ => return false,
        }
    }
    let line = String::from_utf8_lossy(&line);
    let ok = line
        .trim()
        .strip_prefix("auth ")
        .is_some_and(|t| same_token(t.trim(), token));
    let reply = if ok {
        r#"{"ok":true}"#
    } else {
        r#"{"error":"bad or missing token: the first line must be auth TOKEN"}"#
    };
    let _ = writeln!(stream, "{reply}");
    ok
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        // Wake the listener so it sees `closed` and exits (agent control turned off in-game).
        self.closed.store(true, Ordering::Relaxed);
        match self.port {
            Some(port) => drop(TcpStream::connect((Ipv4Addr::LOCALHOST, port))),
            #[cfg(unix)]
            None => drop(UnixStream::connect(&self.path)),
            #[cfg(not(unix))]
            None => {}
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve(stream: Stream, sender: &mpsc::Sender<Request>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let (reply, response) = mpsc::channel();
        if sender.send(Request { line, reply }).is_err() {
            return;
        }
        // The game dropping the reply sender means it exited mid-command.
        let text = response
            .recv()
            .unwrap_or_else(|_| r#"{"error":"game exited"}"#.to_owned());
        if writeln!(writer, "{text}").is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_endpoint_round_trip() {
        let path =
            std::env::temp_dir().join(format!("horizon-ctl-test-{}.json", std::process::id()));
        let server = Server::start(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"tcp\""));
        let mut client = connect(&path).unwrap();
        writeln!(client, "status").unwrap();
        let request = loop {
            if let Some(r) = server.try_recv() {
                break r;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(request.line, "status");
        request.reply.send(r#"{"ok":true}"#.into()).unwrap();
        let mut reply = String::new();
        BufReader::new(client).read_line(&mut reply).unwrap();
        assert_eq!(reply.trim(), r#"{"ok":true}"#);
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn tcp_refuses_a_wrong_token() {
        let path =
            std::env::temp_dir().join(format!("horizon-ctl-bad-{}.json", std::process::id()));
        let _server = Server::start(&path).unwrap();
        let (port, _) = read_endpoint(&path).unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        writeln!(stream, "auth nope").unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert!(reply.contains("error"));
    }

    #[test]
    fn tokens_differ() {
        assert_ne!(random_token(), random_token());
        assert_eq!(random_token().len(), 32);
        assert!(same_token("abc", "abc") && !same_token("abc", "abd") && !same_token("a", "ab"));
    }

    #[test]
    fn frame_summary_reports_block_entity_cache_work() {
        let mut stats = FrameStats::default();
        stats.record(10.0);
        stats.record_cpu(5.0);
        stats.record_block_entities(StaticCacheFrameStats {
            snapshot_rebuilds: 2,
            snapshot_bytes_built: 128,
            static_bytes_uploaded: 96,
            static_entities_drawn: 4,
            dynamic_entities_drawn: 3,
            ..Default::default()
        });

        let summary = stats.summary();
        assert_eq!(summary["static_snapshot_rebuilds"], 2);
        assert_eq!(summary["static_snapshot_bytes_built"], 128);
        assert_eq!(summary["static_bytes_uploaded"], 96);
        assert_eq!(summary["static_entities_drawn"], 4);
        assert_eq!(summary["dynamic_entities_drawn"], 3);
        assert_eq!(summary["static_entities_drawn_avg_per_frame"], 4.0);
        assert_eq!(summary["dynamic_entities_drawn_avg_per_frame"], 3.0);
    }
}

/// Frame durations since the last reset, summarized like the Java recorder,
/// with the CPU part of each frame and GPU times from timestamp queries.
#[derive(Default)]
pub struct FrameStats {
    millis: Vec<f32>,
    cpu: Vec<f32>,
    gpu: Vec<f32>,
    /// Milliseconds per frame phase (`PHASES`), one entry per rendered frame.
    phases: Vec<[f32; PHASES.len()]>,
    static_snapshot_rebuilds: u64,
    static_snapshot_bytes_built: u64,
    static_bytes_uploaded: u64,
    static_entities_drawn: u64,
    dynamic_entities_drawn: u64,
}

/// The parts of a frame's CPU time: world streaming and uploads, game ticks,
/// building the frame's geometry and HUD, and the renderer's own work.
pub const PHASES: [&str; 4] = ["update", "tick", "prepare", "render"];

fn percentiles(values: &[f32]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let n = sorted.len();
    if n == 0 {
        return serde_json::Value::Null;
    }
    let total: f64 = sorted.iter().map(|&m| f64::from(m)).sum();
    let percentile = |p: f64| sorted[((n as f64 * p).ceil() as usize).clamp(1, n) - 1];
    serde_json::json!({
        "avg_ms": total / n as f64,
        "p50_ms": percentile(0.5),
        "p95_ms": percentile(0.95),
        "p99_ms": percentile(0.99),
        "max_ms": sorted[n - 1],
    })
}

impl FrameStats {
    const LIMIT: usize = 1 << 20;
    pub fn record(&mut self, millis: f32) {
        if self.millis.len() < Self::LIMIT {
            self.millis.push(millis);
        }
    }
    /// The frame's own work: its duration minus waits for the GPU, the
    /// display and the frame limiter.
    pub fn record_cpu(&mut self, millis: f32) {
        if self.cpu.len() < Self::LIMIT {
            self.cpu.push(millis);
        }
    }
    pub fn record_phases(&mut self, millis: [f32; PHASES.len()]) {
        if self.phases.len() < Self::LIMIT {
            self.phases.push(millis);
        }
    }
    /// Average milliseconds per phase over the frames whose CPU time is at
    /// least `min_cpu` (all frames for 0): where the slow frames go.
    fn phase_average(&self, min_cpu: f32) -> serde_json::Value {
        let mut sums = [0.0f64; PHASES.len()];
        let mut n = 0;
        for (i, phases) in self.phases.iter().enumerate() {
            if self.cpu.get(i).is_some_and(|&c| c >= min_cpu) {
                for (sum, m) in sums.iter_mut().zip(phases) {
                    *sum += f64::from(*m);
                }
                n += 1;
            }
        }
        PHASES
            .iter()
            .zip(sums)
            .map(|(name, sum)| (name.to_string(), (sum / f64::from(n.max(1))).into()))
            .collect::<serde_json::Map<_, _>>()
            .into()
    }
    pub fn record_gpu(&mut self, millis: &[f32]) {
        if self.gpu.len() < Self::LIMIT {
            self.gpu.extend_from_slice(millis);
        }
    }
    /// Block-entity cache counters for the frame just rendered.
    pub fn record_block_entities(&mut self, stats: StaticCacheFrameStats) {
        self.static_snapshot_rebuilds += stats.snapshot_rebuilds;
        self.static_snapshot_bytes_built += stats.snapshot_bytes_built;
        self.static_bytes_uploaded += stats.static_bytes_uploaded;
        self.static_entities_drawn += stats.static_entities_drawn;
        self.dynamic_entities_drawn += stats.dynamic_entities_drawn;
    }
    /// Every recorded frame time, in milliseconds, in order.
    pub fn frame_millis(&self) -> &[f32] {
        &self.millis
    }
    /// CPU time of every recorded frame, in milliseconds.
    pub fn cpu_millis(&self) -> &[f32] {
        &self.cpu
    }
    /// GPU time of every frame the GPU timed, in milliseconds.
    pub fn gpu_millis(&self) -> &[f32] {
        &self.gpu
    }
    pub fn reset(&mut self) {
        self.millis.clear();
        self.cpu.clear();
        self.gpu.clear();
        self.phases.clear();
        self.static_snapshot_rebuilds = 0;
        self.static_snapshot_bytes_built = 0;
        self.static_bytes_uploaded = 0;
        self.static_entities_drawn = 0;
        self.dynamic_entities_drawn = 0;
    }
    pub fn summary(&self) -> serde_json::Value {
        let mut sorted = self.millis.clone();
        sorted.sort_by(f32::total_cmp);
        let n = sorted.len();
        if n == 0 {
            return serde_json::json!({ "frames": 0 });
        }
        let total: f64 = sorted.iter().map(|&m| f64::from(m)).sum();
        let percentile = |p: f64| sorted[((n as f64 * p).ceil() as usize).clamp(1, n) - 1];
        // Average frame time of the slowest 1% (0.1%) of frames, as frames per second.
        let low = |per: usize| {
            let slowest = n.div_ceil(per);
            let slow: f64 = sorted[n - slowest..]
                .iter()
                .map(|&m| f64::from(m))
                .sum::<f64>()
                / slowest as f64;
            1000.0 / slow
        };
        // Where the slowest 1% of frames by CPU time go.
        let slow_phases = {
            let mut cpu = self.cpu.clone();
            cpu.sort_by(f32::total_cmp);
            let p99 = cpu
                .get(cpu.len().saturating_sub(cpu.len().div_ceil(100)))
                .copied();
            p99.map_or(serde_json::Value::Null, |p| self.phase_average(p))
        };
        serde_json::json!({
            "frames": n,
            "seconds": total / 1000.0,
            "avg_fps": n as f64 * 1000.0 / total,
            "low_1pct_fps": low(100),
            "low_01pct_fps": low(1000),
            "p50_ms": percentile(0.5),
            "p95_ms": percentile(0.95),
            "p99_ms": percentile(0.99),
            "max_ms": sorted[n - 1],
            "cpu": percentiles(&self.cpu),
            "gpu": percentiles(&self.gpu),
            "phases_avg_ms": self.phase_average(0.0),
            "slow_phases_avg_ms": slow_phases,
            "static_snapshot_rebuilds": self.static_snapshot_rebuilds,
            "static_snapshot_bytes_built": self.static_snapshot_bytes_built,
            "static_bytes_uploaded": self.static_bytes_uploaded,
            "static_entities_drawn": self.static_entities_drawn,
            "dynamic_entities_drawn": self.dynamic_entities_drawn,
            "static_entities_drawn_avg_per_frame": self.static_entities_drawn as f64 / n as f64,
            "dynamic_entities_drawn_avg_per_frame": self.dynamic_entities_drawn as f64 / n as f64,
        })
    }
}
