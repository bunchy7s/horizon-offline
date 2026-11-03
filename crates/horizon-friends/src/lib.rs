//! Play with friends: a host shares the world it opened to LAN behind an
//! invite code that holds its addresses and a secret; guests dial it
//! directly (the host opens its router port with PCP / NAT-PMP / UPnP and
//! learns its public address with STUN). A relay anyone can run
//! (`horizon-relay`) adds short codes and a fallback for networks that
//! can't be reached. Both ends carry the game's TCP stream
//! unchanged between loopback ports (the integrated server on the host, a
//! local port the guest's client connects to), end-to-end encrypted with a
//! key only the holders of the code can derive. The design, the protocol and
//! what the relay sees are in docs/play-with-friends.md.
pub mod code;
pub mod crypto;
pub mod direct;
pub mod guest;
pub mod host;
pub mod invite;
pub mod portmap;
pub mod reach;
mod session;
pub mod stun;
pub mod wire;

pub use code::Code;
pub use guest::{JoinError, Tunnel};
pub use host::{Host, HostConfig, HostEvent, HostState};

use std::{
    io,
    net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs},
    sync::mpsc::{RecvTimeoutError, Sender, channel},
    thread::JoinHandle,
    time::Duration,
};

/// The game's server address for a friend's world: `friend:K7QM-3XPD`
/// (`horizon-client --server friend:…`, what the launcher's Join a friend
/// starts).
pub const ADDRESS_PREFIX: &str = "friend:";

/// Fill `buf` from the OS random generator.
pub(crate) fn fill_random(buf: &mut [u8]) -> io::Result<()> {
    getrandom::fill(buf).map_err(|e| io::Error::other(e.to_string()))
}

/// Ends a stream's opening once its time is up; dropping it first lets the
/// stream go on. A read timeout alone restarts with every byte, so a peer
/// sending one byte at a time could hold a handshake (and its slot) for
/// hours.
pub struct Deadline {
    _cancel: Sender<()>,
}

/// Shut `stream` down after `limit` unless the returned [`Deadline`] is
/// dropped before.
pub fn deadline(stream: &TcpStream, limit: Duration) -> io::Result<Deadline> {
    let stream = stream.try_clone()?;
    let (tx, rx) = channel::<()>();
    std::thread::Builder::new()
        .name("horizon-friends-deadline".into())
        .spawn(move || {
            if rx.recv_timeout(limit) == Err(RecvTimeoutError::Timeout) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        })?;
    Ok(Deadline { _cancel: tx })
}

/// Which paths a session may take (`HORIZON_FRIENDS_PATH`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathMode {
    /// Direct when the ends reach each other, else the relay.
    Auto,
    /// Never connect directly: the other end doesn't learn this one's address.
    RelayOnly,
    /// Never fall back to the relay (tests).
    DirectOnly,
}
impl PathMode {
    pub fn from_env() -> Self {
        match std::env::var("HORIZON_FRIENDS_PATH").as_deref() {
            Ok("relay") => Self::RelayOnly,
            Ok("direct") => Self::DirectOnly,
            _ => Self::Auto,
        }
    }
}

/// A relay from `HORIZON_RELAY_URL` (tests, or players who set it in the
/// environment); the game adds its own setting. None by default: friends
/// join from the code alone.
pub fn relay_url() -> Option<String> {
    std::env::var("HORIZON_RELAY_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// `host:port`, optionally written `tcp://host:port`.
pub fn resolve_relay(url: &str) -> io::Result<Vec<SocketAddr>> {
    let address = url
        .trim()
        .trim_start_matches("tcp://")
        .trim_end_matches('/');
    let found: Vec<SocketAddr> = address.to_socket_addrs()?.collect();
    if found.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the relay's name didn't resolve",
        ));
    }
    Ok(found)
}

/// Carry one connection both ways between a plain stream (the game's) and a
/// protected one, until either side ends; then both are closed.
pub fn pump(
    plain: TcpStream,
    mut reader: crypto::SecureReader<TcpStream>,
    mut writer: crypto::SecureWriter<TcpStream>,
) -> io::Result<JoinHandle<()>> {
    use io::{Read, Write};
    for s in [&plain, reader.get_ref(), writer.get_ref()] {
        s.set_read_timeout(None)?;
        let _ = s.set_nodelay(true);
    }
    let mut plain_in = plain.try_clone()?;
    let mut plain_out = plain;
    let outbound = std::thread::Builder::new()
        .name("horizon-friends-out".into())
        .spawn(move || {
            let mut buf = vec![0u8; crypto::MAX_PAYLOAD];
            loop {
                match plain_in.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if writer.write_data(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = writer.get_ref().shutdown(std::net::Shutdown::Both);
            let _ = plain_in.shutdown(std::net::Shutdown::Both);
        })?;
    std::thread::Builder::new()
        .name("horizon-friends-in".into())
        .spawn(move || {
            while let Ok(Some(frame)) = reader.read_frame() {
                if let crypto::Frame::Data(d) = frame
                    && plain_out.write_all(&d).is_err()
                {
                    break;
                }
            }
            let _ = plain_out.shutdown(std::net::Shutdown::Both);
            let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
            let _ = outbound.join();
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        net::TcpListener,
        time::{Duration, Instant},
    };

    #[test]
    fn a_deadline_ends_the_stream_unless_dropped() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let _kept = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (mut kept, _) = l.accept().unwrap();
        let _ended = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (mut ended, _) = l.accept().unwrap();
        drop(deadline(&kept, Duration::from_millis(100)).unwrap());
        let _running = deadline(&ended, Duration::from_millis(100)).unwrap();
        // Read once the limit has passed: a read already blocked when it
        // passes is not woken on Windows (shutdown doesn't cancel it there).
        std::thread::sleep(Duration::from_millis(300));
        ended
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let started = Instant::now();
        match ended.read(&mut [0u8; 1]) {
            Ok(n) => assert_eq!(n, 0),
            Err(e) => assert!(!matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            )),
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        kept.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let e = kept.read(&mut [0u8; 1]).unwrap_err();
        assert!(matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
    }

    #[test]
    fn a_deadline_ends_a_peer_that_sends_one_byte_at_a_time() {
        use std::io::Write;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (mut s, _) = l.accept().unwrap();
        std::thread::spawn(move || {
            for _ in 0..100 {
                if peer.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        // What the callers do: a read timeout that restarts with every byte.
        s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let _running = deadline(&s, Duration::from_millis(300)).unwrap();
        let started = Instant::now();
        while let Ok(n) = s.read(&mut [0u8; 1])
            && n > 0
        {}
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }
}
