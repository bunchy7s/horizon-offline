//! The SRV question as JNDI's DNS client asks it: the system's name servers in
//! turn, one UDP try of 1 s (`com.sun.jndi.dns.timeout.retries` = 1), TCP when
//! the reply is truncated, and the answer's first SRV record. By hand, so the
//! game needs no async runtime or resolver library for one question.
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, UdpSocket},
    time::Duration,
};

const TYPE_SRV: u16 = 33;
/// JNDI's `com.sun.jndi.dns.timeout.initial`, per name server.
pub(super) const SERVER_TIMEOUT: Duration = Duration::from_secs(1);

/// JNDI reads them from resolv.conf on Unix and from the network adapters on
/// Windows, where it leaves out the scoped IPv6 ones.
#[cfg(unix)]
pub(super) fn system_name_servers() -> Vec<SocketAddr> {
    std::fs::read_to_string("/etc/resolv.conf")
        .map_or_else(|_| Vec::new(), |conf| name_servers(&conf))
}

#[cfg(windows)]
pub(super) fn system_name_servers() -> Vec<SocketAddr> {
    let mut servers = Vec::new();
    for adapter in ipconfig::get_adapters().unwrap_or_default() {
        if adapter.oper_status() != ipconfig::OperStatus::IfOperStatusUp {
            continue;
        }
        for &ip in adapter.dns_servers().iter().filter(|ip| !scoped(**ip)) {
            let server = SocketAddr::new(ip, 53);
            if !servers.contains(&server) {
                servers.push(server);
            }
        }
    }
    servers
}

#[cfg(not(any(unix, windows)))]
pub(super) fn system_name_servers() -> Vec<SocketAddr> {
    Vec::new()
}

/// Link-local and site-local IPv6 addresses (such as Windows' `fec0:0:0:ffff::1`
/// defaults), which only work with a scope the adapter's list doesn't carry.
#[cfg(any(windows, test))]
fn scoped(ip: std::net::IpAddr) -> bool {
    matches!(ip, std::net::IpAddr::V6(v6) if matches!(v6.segments()[0] & 0xffc0, 0xfe80 | 0xfec0))
}

/// `nameserver` lines of resolv.conf, in order.
#[cfg(any(unix, test))]
fn name_servers(conf: &str) -> Vec<SocketAddr> {
    conf.lines()
        .filter_map(|line| line.strip_prefix("nameserver"))
        .filter_map(|rest| rest.split_whitespace().next())
        .filter_map(|ip| ip.split('%').next()?.parse::<std::net::IpAddr>().ok())
        .map(|ip| SocketAddr::new(ip, 53))
        .collect()
}

enum Reply {
    /// The server answered: the first SRV record, if any.
    Answer(Option<(String, u16)>),
    Truncated,
    /// No reply, a server failure or a malformed reply: ask the next server.
    Unusable,
}

pub(super) fn query_srv(
    name: &str,
    servers: &[SocketAddr],
    timeout: Duration,
) -> Option<(String, u16)> {
    let mut id = [0; 2];
    getrandom::fill(&mut id).ok()?;
    let id = u16::from_be_bytes(id);
    let query = encode_query(id, name)?;
    for &server in servers {
        let reply = match ask_udp(server, &query, timeout).map(|reply| parse_reply(id, &reply)) {
            Some(Reply::Truncated) => ask_tcp(server, &query, timeout)
                .map_or(Reply::Unusable, |reply| parse_reply(id, &reply)),
            reply => reply.unwrap_or(Reply::Unusable),
        };
        if let Reply::Answer(srv) = reply {
            return srv;
        }
    }
    None
}

fn ask_udp(server: SocketAddr, query: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    let local: SocketAddr = if server.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0u16; 8], 0).into()
    };
    let socket = UdpSocket::bind(local).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    socket.connect(server).ok()?;
    socket.send(query).ok()?;
    let mut buf = vec![0; 512];
    let len = socket.recv(&mut buf).ok()?;
    buf.truncate(len);
    Some(buf)
}

fn ask_tcp(server: SocketAddr, query: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&server, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    let mut framed = (query.len() as u16).to_be_bytes().to_vec();
    framed.extend_from_slice(query);
    stream.write_all(&framed).ok()?;
    let mut len = [0; 2];
    stream.read_exact(&mut len).ok()?;
    let mut buf = vec![0; u16::from_be_bytes(len) as usize];
    stream.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// A recursive SRV question (RFC 1035 §4.1) for an absolute name.
fn encode_query(id: u16, name: &str) -> Option<Vec<u8>> {
    let mut msg = Vec::with_capacity(18 + name.len());
    msg.extend_from_slice(&id.to_be_bytes());
    msg.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]); // RD; one question
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        msg.push(label.len() as u8);
        msg.extend_from_slice(label.as_bytes());
    }
    msg.push(0);
    if msg.len() - 12 > 255 {
        return None;
    }
    msg.extend_from_slice(&TYPE_SRV.to_be_bytes());
    msg.extend_from_slice(&1u16.to_be_bytes()); // IN
    Some(msg)
}

fn parse_reply(id: u16, msg: &[u8]) -> Reply {
    parse_answer(id, msg).unwrap_or(Reply::Unusable)
}

fn parse_answer(id: u16, msg: &[u8]) -> Option<Reply> {
    let u16_at = |at: usize| Some(u16::from_be_bytes([*msg.get(at)?, *msg.get(at + 1)?]));
    let flags = u16_at(2)?;
    if u16_at(0)? != id || flags & 0x8000 == 0 {
        return None;
    }
    if flags & 0x0200 != 0 {
        return Some(Reply::Truncated);
    }
    match flags & 0xf {
        0 => {}
        3 => return Some(Reply::Answer(None)), // NXDOMAIN
        _ => return None,
    }
    let mut at = 12;
    for _ in 0..u16_at(4)? {
        at = read_name(msg, at)?.1 + 4;
    }
    for _ in 0..u16_at(6)? {
        at = read_name(msg, at)?.1;
        let (kind, len) = (u16_at(at)?, u16_at(at + 8)? as usize);
        let data = at + 10;
        msg.get(data..data + len)?;
        if kind == TYPE_SRV && len >= 7 {
            let (target, _) = read_name(msg, data + 6)?;
            return Some(Reply::Answer(Some((target, u16_at(data + 4)?))));
        }
        at = data + len;
    }
    Some(Reply::Answer(None))
}

/// The name at `at`, absolute (`a.b.`), and where the record continues.
fn read_name(msg: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut end = None;
    // Pointers must go backwards; the bound ends a loop of labels and pointers.
    for _ in 0..128 {
        let len = *msg.get(at)? as usize;
        match len {
            0 => {
                if name.is_empty() {
                    name.push('.');
                }
                return Some((name, end.unwrap_or(at + 1)));
            }
            0xc0.. => {
                let target = (len & 0x3f) << 8 | *msg.get(at + 1)? as usize;
                if target >= at {
                    return None;
                }
                end.get_or_insert(at + 2);
                at = target;
            }
            1..=63 => {
                let label = msg.get(at + 1..at + 1 + len)?;
                name.push_str(std::str::from_utf8(label).ok()?);
                name.push('.');
                at += 1 + len;
            }
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "_minecraft._tcp.play.example.org.";

    /// The reply a server gives to `query`: its question, then a CNAME and the
    /// SRV records, each owner name compressed to the question's name.
    fn reply(query: &[u8], rcode: u8, records: &[(u16, &str)]) -> Vec<u8> {
        let mut msg = query.to_vec();
        msg[2] |= 0x80;
        msg[3] = 0x80 | rcode;
        let answers = records.len() as u16 + u16::from(!records.is_empty());
        msg[6..8].copy_from_slice(&answers.to_be_bytes());
        if !records.is_empty() {
            // CNAME first: the parser must step over records of other types.
            msg.extend_from_slice(&[0xc0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 2, 0xc0, 12]);
        }
        for &(port, target) in records {
            let mut rdata = vec![0, 1, 0, 1];
            rdata.extend_from_slice(&port.to_be_bytes());
            for label in target.trim_end_matches('.').split('.') {
                rdata.push(label.len() as u8);
                rdata.extend_from_slice(label.as_bytes());
            }
            rdata.push(0);
            msg.extend_from_slice(&[0xc0, 12, 0, 33, 0, 1, 0, 0, 0, 60]);
            msg.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
            msg.extend_from_slice(&rdata);
        }
        msg
    }

    fn answer(msg: &[u8]) -> Option<Option<(String, u16)>> {
        match parse_reply(7, msg) {
            Reply::Answer(srv) => Some(srv),
            _ => None,
        }
    }

    #[test]
    fn query_is_a_recursive_srv_question() {
        let query = encode_query(0x1234, NAME).unwrap();
        let mut expected = vec![0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        expected.extend_from_slice(b"\x0a_minecraft\x04_tcp\x04play\x07example\x03org\x00");
        expected.extend_from_slice(&[0, 33, 0, 1]);
        assert_eq!(query, expected);
        assert!(encode_query(1, "a..b.").is_none());
        assert!(encode_query(1, &format!("{}.org.", "a".repeat(64))).is_none());
    }

    #[test]
    fn first_srv_record_of_the_answer_wins() {
        let query = encode_query(7, NAME).unwrap();
        let msg = reply(
            &query,
            0,
            &[(25570, "a.example.org."), (25571, "b.example.org.")],
        );
        assert_eq!(answer(&msg), Some(Some(("a.example.org.".into(), 25570))));
    }

    #[test]
    fn empty_answer_and_nxdomain_mean_no_redirect() {
        let query = encode_query(7, NAME).unwrap();
        assert_eq!(answer(&reply(&query, 0, &[])), Some(None));
        assert_eq!(answer(&reply(&query, 3, &[])), Some(None));
    }

    #[test]
    fn server_failures_and_bad_replies_ask_the_next_server() {
        let query = encode_query(7, NAME).unwrap();
        let good = reply(&query, 0, &[(25570, "a.example.org.")]);
        assert!(matches!(
            parse_reply(7, &reply(&query, 2, &[])),
            Reply::Unusable
        ));
        assert!(matches!(parse_reply(8, &good), Reply::Unusable)); // another id
        assert!(matches!(parse_reply(7, &query), Reply::Unusable)); // not a response
        for cut in 0..good.len() {
            assert!(matches!(parse_reply(7, &good[..cut]), Reply::Unusable));
        }
        let mut forward = good.clone();
        let at = query.len(); // the first answer's owner pointer
        assert_eq!(forward[at..at + 2], [0xc0, 12]);
        forward[at + 1] = 0xff; // a pointer forwards
        assert!(matches!(parse_reply(7, &forward), Reply::Unusable));
        let mut truncated = good;
        truncated[2] |= 0x02;
        assert!(matches!(parse_reply(7, &truncated), Reply::Truncated));
    }

    #[test]
    fn resolv_conf_name_servers_in_order() {
        let conf = "# generated\nsearch lan\nnameserver 192.168.1.1\nnameserver fe80::1%en0\n\
                    nameserver  ::1 \nnameserver bogus\noptions ndots:1\n";
        let servers: Vec<String> = name_servers(conf).iter().map(|s| s.to_string()).collect();
        assert_eq!(servers, ["192.168.1.1:53", "[fe80::1]:53", "[::1]:53"]);
    }

    #[test]
    fn windows_skips_scoped_ipv6_name_servers() {
        let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        for s in ["fe80::1", "fec0:0:0:ffff::1", "febf::1"] {
            assert!(scoped(ip(s)), "{s}");
        }
        for s in ["1.1.1.1", "2606:4700:4700::1111", "::1", "fc00::1"] {
            assert!(!scoped(ip(s)), "{s}");
        }
    }

    /// A local name server: a silent one first, then one that answers truncated
    /// over UDP and in full over TCP, like a real server with a big answer.
    #[test]
    fn asks_each_server_in_turn_and_retries_truncated_replies_over_tcp() {
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (udp, tcp) = loop {
            let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
            if let Ok(tcp) = std::net::TcpListener::bind(udp.local_addr().unwrap()) {
                break (udp, tcp);
            }
        };
        let servers = [silent.local_addr().unwrap(), udp.local_addr().unwrap()];
        let server = std::thread::spawn(move || {
            let mut buf = [0; 512];
            let (len, from) = udp.recv_from(&mut buf).unwrap();
            let mut truncated = reply(&buf[..len], 0, &[]);
            truncated[2] |= 0x02;
            udp.send_to(&truncated, from).unwrap();
            let (mut stream, _) = tcp.accept().unwrap();
            let mut len = [0; 2];
            stream.read_exact(&mut len).unwrap();
            let mut query = vec![0; u16::from_be_bytes(len) as usize];
            stream.read_exact(&mut query).unwrap();
            let full = reply(&query, 0, &[(25570, "backend.example.org.")]);
            stream
                .write_all(&(full.len() as u16).to_be_bytes())
                .unwrap();
            stream.write_all(&full).unwrap();
        });
        let srv = query_srv(NAME, &servers, Duration::from_millis(200));
        server.join().unwrap();
        assert_eq!(srv, Some(("backend.example.org.".into(), 25570)));
    }
}
