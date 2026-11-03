//! Opening the host's direct port on the home router, for a limited time:
//! PCP (RFC 6887), then NAT-PMP (RFC 6886) on the gateway's port 5351, then
//! UPnP IGD (SSDP discovery, then SOAP `AddPortMapping`). The mapping is
//! renewed while sharing and deleted when it stops; a crashed game's mapping
//! expires with its lease.
use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    time::{Duration, Instant},
};

/// How long a mapping is asked for; it is renewed at half-time.
pub const LEASE: Duration = Duration::from_secs(3600);
const PMP_PORT: u16 = 5351;
const DESCRIPTION: &str = "Horizon (play with friends)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Pcp,
    NatPmp,
    Upnp,
}
impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pcp => "PCP",
            Self::NatPmp => "NAT-PMP",
            Self::Upnp => "UPnP",
        }
    }
}

#[derive(Clone, Debug)]
enum Handle {
    Pmp {
        gateway: SocketAddr,
        client: Ipv4Addr,
        nonce: [u8; 12],
    },
    NatPmp {
        gateway: SocketAddr,
    },
    Upnp {
        igd: Igd,
        client: Ipv4Addr,
        permanent: bool,
    },
}

/// A port opened on the router.
#[derive(Clone, Debug)]
pub struct Mapping {
    pub method: Method,
    pub internal: u16,
    /// The router's public side: its WAN address (unspecified when the
    /// router didn't say) and the external port.
    pub external: SocketAddr,
    pub lifetime: Duration,
    pub renewed: Instant,
    handle: Handle,
}

/// Where to ask: gateways speaking PCP/NAT-PMP, UPnP description URLs.
#[derive(Clone, Debug, Default)]
pub struct Targets {
    pub pmp: Vec<SocketAddr>,
    pub upnp: Vec<String>,
}
impl Targets {
    /// The default gateway's port 5351, and the IGDs answering SSDP.
    pub fn discover(timeout: Duration) -> Self {
        let mut pmp: Vec<SocketAddr> = default_gateway()
            .into_iter()
            .map(|g| SocketAddr::new(IpAddr::V4(g), PMP_PORT))
            .collect();
        let upnp = ssdp_discover(timeout);
        // An IGD's own address is a gateway too.
        for l in &upnp {
            if let Some((host, _, _)) = split_url(l)
                && let Ok(ip) = host.parse::<Ipv4Addr>()
            {
                let a = SocketAddr::new(IpAddr::V4(ip), PMP_PORT);
                if !pmp.contains(&a) {
                    pmp.push(a);
                }
            }
        }
        Self { pmp, upnp }
    }
}

/// Open `internal` on the router for `client` (this machine's LAN address),
/// trying each method in turn. The error lists why each failed.
pub fn map(
    targets: &Targets,
    client: Ipv4Addr,
    internal: u16,
    lease: Duration,
) -> Result<Mapping, String> {
    let mut why = Vec::new();
    for &gw in &targets.pmp {
        let mut nonce = [0u8; 12];
        let _ = crate::fill_random(&mut nonce);
        match pcp_map(gw, client, internal, internal, lease, &nonce) {
            Ok((external, life)) => {
                return Ok(Mapping {
                    method: Method::Pcp,
                    internal,
                    external,
                    lifetime: life,
                    renewed: Instant::now(),
                    handle: Handle::Pmp {
                        gateway: gw,
                        client,
                        nonce,
                    },
                });
            }
            Err(e) => why.push(format!("PCP: {e}")),
        }
        match natpmp_map(gw, internal, internal, lease) {
            Ok((port, life)) => {
                let ip = natpmp_external(gw).map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), IpAddr::V4);
                return Ok(Mapping {
                    method: Method::NatPmp,
                    internal,
                    external: SocketAddr::new(ip, port),
                    lifetime: life,
                    renewed: Instant::now(),
                    handle: Handle::NatPmp { gateway: gw },
                });
            }
            Err(e) => why.push(format!("NAT-PMP: {e}")),
        }
    }
    for location in &targets.upnp {
        match Igd::from_location(location).and_then(|igd| upnp_map(igd, client, internal, lease)) {
            Ok(m) => return Ok(m),
            Err(e) => why.push(format!("UPnP: {e}")),
        }
    }
    if why.is_empty() {
        why.push("no router answered".into());
    }
    Err(why.join("; "))
}

fn upnp_map(igd: Igd, client: Ipv4Addr, internal: u16, lease: Duration) -> io::Result<Mapping> {
    let mut permanent = false;
    let mut last = io::Error::other("no port");
    // The same port outside first (easy to forward by hand later), then others.
    let mut ports = vec![internal];
    let mut r = [0u8; 2];
    for _ in 0..3 {
        let _ = crate::fill_random(&mut r);
        ports.push(20000 + u16::from_le_bytes(r) % 40000);
    }
    for port in ports {
        let secs = if permanent { 0 } else { lease.as_secs() as u32 };
        match igd.add(port, internal, client, secs) {
            Ok(()) => {
                let ip = igd
                    .external_ip()
                    .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
                return Ok(Mapping {
                    method: Method::Upnp,
                    internal,
                    external: SocketAddr::new(ip, port),
                    lifetime: if permanent { LEASE } else { lease },
                    renewed: Instant::now(),
                    handle: Handle::Upnp {
                        igd,
                        client,
                        permanent,
                    },
                });
            }
            // OnlyPermanentLeasesSupported: ask without a lease (deleted on stop).
            Err(UpnpError::Code(725)) if !permanent => {
                permanent = true;
                match igd.add(port, internal, client, 0) {
                    Ok(()) => {
                        let ip = igd
                            .external_ip()
                            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
                        return Ok(Mapping {
                            method: Method::Upnp,
                            internal,
                            external: SocketAddr::new(ip, port),
                            lifetime: LEASE,
                            renewed: Instant::now(),
                            handle: Handle::Upnp {
                                igd,
                                client,
                                permanent,
                            },
                        });
                    }
                    Err(e) => last = e.into(),
                }
            }
            // ConflictInMappingEntry and friends: another port.
            Err(UpnpError::Code(c)) => last = io::Error::other(format!("error {c}")),
            Err(UpnpError::Io(e)) => return Err(e),
        }
    }
    Err(last)
}

impl Mapping {
    /// A mapping as a router would have granted it (tests elsewhere).
    #[cfg(test)]
    pub(crate) fn granted(method: Method, external: SocketAddr) -> Self {
        Self {
            method,
            internal: 25580,
            external,
            lifetime: LEASE,
            renewed: Instant::now(),
            handle: Handle::NatPmp {
                gateway: SocketAddr::from(([127, 0, 0, 1], 9)),
            },
        }
    }
    /// Whether the lease is past half-time.
    pub fn due(&self) -> bool {
        self.renewed.elapsed() >= self.lifetime / 2
    }
    pub fn renew(&mut self) -> io::Result<()> {
        let internal = self.internal;
        let lease = LEASE;
        match &self.handle {
            Handle::Pmp {
                gateway,
                client,
                nonce,
            } => {
                let (external, life) = pcp_map(
                    *gateway,
                    *client,
                    internal,
                    self.external.port(),
                    lease,
                    nonce,
                )?;
                self.external = external;
                self.lifetime = life;
            }
            Handle::NatPmp { gateway } => {
                let (port, life) = natpmp_map(*gateway, internal, self.external.port(), lease)?;
                self.external.set_port(port);
                self.lifetime = life;
            }
            Handle::Upnp {
                igd,
                client,
                permanent,
            } => {
                let secs = if *permanent {
                    0
                } else {
                    lease.as_secs() as u32
                };
                igd.add(self.external.port(), internal, *client, secs)?;
            }
        }
        self.renewed = Instant::now();
        Ok(())
    }
    /// Delete the mapping (best effort).
    pub fn remove(&self) {
        match &self.handle {
            Handle::Pmp {
                gateway,
                client,
                nonce,
            } => {
                let _ = pcp_map(*gateway, *client, self.internal, 0, Duration::ZERO, nonce);
            }
            Handle::NatPmp { gateway } => {
                let _ = natpmp_map(*gateway, self.internal, 0, Duration::ZERO);
            }
            Handle::Upnp { igd, .. } => {
                let _ = igd.delete(self.external.port());
            }
        }
    }
}

// ----- PCP and NAT-PMP --------------------------------------------------------------------------

/// Send `req` and wait for a datagram from the gateway, with RFC 6886's
/// retransmissions (250 ms, doubling), three tries.
fn exchange(gateway: SocketAddr, req: &[u8]) -> io::Result<Vec<u8>> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect(gateway)?;
    let mut wait = Duration::from_millis(250);
    let mut buf = [0u8; 1100];
    for _ in 0..3 {
        socket.send(req)?;
        socket.set_read_timeout(Some(wait))?;
        match socket.recv(&mut buf) {
            Ok(n) => return Ok(buf[..n].to_vec()),
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => return Err(e),
            Err(_) => wait *= 2,
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "no answer"))
}

fn mapped_v6(ip: Ipv4Addr) -> [u8; 16] {
    ip.to_ipv6_mapped().octets()
}

/// PCP MAP for TCP: the external address and the granted lifetime. A
/// zero lifetime deletes.
pub fn pcp_map(
    gateway: SocketAddr,
    client: Ipv4Addr,
    internal: u16,
    external: u16,
    lease: Duration,
    nonce: &[u8; 12],
) -> io::Result<(SocketAddr, Duration)> {
    let mut req = vec![0u8; 60];
    req[0] = 2;
    req[1] = 1;
    req[4..8].copy_from_slice(&(lease.as_secs() as u32).to_be_bytes());
    req[8..24].copy_from_slice(&mapped_v6(client));
    req[24..36].copy_from_slice(nonce);
    req[36] = 6;
    req[40..42].copy_from_slice(&internal.to_be_bytes());
    req[42..44].copy_from_slice(&external.to_be_bytes());
    req[44..60].copy_from_slice(&mapped_v6(Ipv4Addr::UNSPECIFIED));
    let r = exchange(gateway, &req)?;
    if r.len() < 2 || r[0] != 2 {
        return Err(io::Error::other("not a PCP router"));
    }
    if r.len() < 60 || r[1] != 0x81 || &r[24..36] != nonce {
        return Err(io::Error::other("bad answer"));
    }
    if r[3] != 0 {
        return Err(io::Error::other(format!("result {}", r[3])));
    }
    let life = Duration::from_secs(u64::from(u32::from_be_bytes(r[4..8].try_into().unwrap())));
    let port = u16::from_be_bytes([r[42], r[43]]);
    let o: [u8; 16] = r[44..60].try_into().unwrap();
    let v6 = Ipv6Addr::from(o);
    let ip = v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4);
    Ok((SocketAddr::new(ip, port), life))
}

/// NAT-PMP's external address request.
pub fn natpmp_external(gateway: SocketAddr) -> io::Result<Ipv4Addr> {
    let r = exchange(gateway, &[0, 0])?;
    if r.len() < 12 || r[0] != 0 || r[1] != 128 {
        return Err(io::Error::other("bad answer"));
    }
    if r[2..4] != [0, 0] {
        return Err(io::Error::other(format!(
            "result {}",
            u16::from_be_bytes([r[2], r[3]])
        )));
    }
    Ok(Ipv4Addr::new(r[8], r[9], r[10], r[11]))
}

/// NAT-PMP TCP mapping: the external port and lifetime. Zero deletes.
pub fn natpmp_map(
    gateway: SocketAddr,
    internal: u16,
    external: u16,
    lease: Duration,
) -> io::Result<(u16, Duration)> {
    let mut req = [0u8; 12];
    req[1] = 2;
    req[4..6].copy_from_slice(&internal.to_be_bytes());
    req[6..8].copy_from_slice(&external.to_be_bytes());
    req[8..12].copy_from_slice(&(lease.as_secs() as u32).to_be_bytes());
    let r = exchange(gateway, &req)?;
    if r.len() < 16 || r[0] != 0 || r[1] != 130 {
        return Err(io::Error::other("not a NAT-PMP router"));
    }
    if r[2..4] != [0, 0] {
        return Err(io::Error::other(format!(
            "result {}",
            u16::from_be_bytes([r[2], r[3]])
        )));
    }
    Ok((
        u16::from_be_bytes([r[10], r[11]]),
        Duration::from_secs(u64::from(u32::from_be_bytes(r[12..16].try_into().unwrap()))),
    ))
}

// ----- UPnP IGD ---------------------------------------------------------------------------------

/// `http://host:port/path` → host, port, path.
fn split_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    // `[v6]` or `[v6]:port`: the colons inside the brackets are not a port's.
    let (host, port) = match hostport.rfind(']') {
        Some(i) => match &hostport[i + 1..] {
            "" => (&hostport[..=i], 80),
            p => (&hostport[..=i], p.strip_prefix(':')?.parse().ok()?),
        },
        None => match hostport.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().ok()?),
            None => (hostport, 80),
        },
    };
    Some((
        host.trim_matches(|c| c == '[' || c == ']').to_owned(),
        port,
        path.to_owned(),
    ))
}

/// A tiny HTTP/1.1 client (Connection: close; Content-Length, chunked or
/// to the end).
fn http(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> io::Result<(u16, String)> {
    let (host, port, path) = split_url(url).ok_or_else(|| io::Error::other("bad URL"))?;
    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::other("no address"))?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    s.take(1 << 20).read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| io::Error::other("bad response"))?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| io::Error::other("bad status"))?;
    let chunked = head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let body = if chunked {
        dechunk(rest)
    } else {
        rest.to_owned()
    };
    Ok((status, body))
}
fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16) else {
            break;
        };
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        s = rest[n..].trim_start_matches("\r\n");
    }
    out
}

/// The text of the first `<tag>` (any namespace prefix) in `xml`.
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(i) = xml[from..].find('<') {
        let start = from + i + 1;
        let end = start + xml[start..].find('>')?;
        let open = &xml[start..end];
        let local = open.split_whitespace().next().unwrap_or("");
        let local = local.rsplit(':').next().unwrap_or(local);
        if local == name && !open.starts_with('/') {
            let body = &xml[end + 1..];
            let close = body.find("</")?;
            return Some(body[..close].trim());
        }
        from = end + 1;
    }
    None
}

/// The Location headers of SSDP answers (devices answering an M-SEARCH for
/// internet gateways).
pub fn ssdp_discover(timeout: Duration) -> Vec<String> {
    let Ok(s) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
    for st in [
        "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
        "urn:schemas-upnp-org:service:WANIPConnection:1",
    ] {
        let m = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nST: {st}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\n\r\n"
        );
        let _ = s.send_to(m.as_bytes(), "239.255.255.250:1900");
    }
    let deadline = Instant::now() + timeout;
    let mut found = Vec::new();
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        if let Ok((n, _)) = s.recv_from(&mut buf)
            && let Some(l) = ssdp_location(&String::from_utf8_lossy(&buf[..n]))
            && !found.contains(&l)
        {
            found.push(l);
        }
    }
    found
}
pub fn ssdp_location(response: &str) -> Option<String> {
    response.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case("location")
            .then(|| v.trim().to_owned())
    })
}

enum UpnpError {
    Code(u32),
    Io(io::Error),
}
impl From<io::Error> for UpnpError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<UpnpError> for io::Error {
    fn from(e: UpnpError) -> Self {
        match e {
            UpnpError::Code(c) => io::Error::other(format!("error {c}")),
            UpnpError::Io(e) => e,
        }
    }
}

/// An internet gateway's connection service.
#[derive(Clone, Debug)]
pub struct Igd {
    control: String,
    service: String,
}
impl Igd {
    /// Read the device description for a WANIPConnection (or WANPPPConnection) service.
    pub fn from_location(location: &str) -> io::Result<Self> {
        let (status, xml) = http("GET", location, &[], "")?;
        if status != 200 {
            return Err(io::Error::other(format!("description: HTTP {status}")));
        }
        let (host, port, _) = split_url(location).ok_or_else(|| io::Error::other("bad URL"))?;
        let base = tag(&xml, "URLBase")
            .map(|b| b.trim_end_matches('/').to_owned())
            .unwrap_or_else(|| format!("http://{host}:{port}"));
        for block in xml.split("<service>").skip(1) {
            let Some(service) = tag(block, "serviceType") else {
                continue;
            };
            if !(service.contains(":WANIPConnection:") || service.contains(":WANPPPConnection:")) {
                continue;
            }
            let Some(control) = tag(block, "controlURL") else {
                continue;
            };
            let control = if control.starts_with("http://") {
                control.to_owned()
            } else {
                format!("{base}/{}", control.trim_start_matches('/'))
            };
            return Ok(Self {
                control,
                service: service.to_owned(),
            });
        }
        Err(io::Error::other("no WAN connection service"))
    }
    fn soap(&self, action: &str, args: &[(&str, String)]) -> Result<String, UpnpError> {
        let mut body = format!(
            "<?xml version=\"1.0\"?>\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">",
            self.service
        );
        for (k, v) in args {
            body.push_str(&format!("<{k}>{v}</{k}>"));
        }
        body.push_str(&format!("</u:{action}></s:Body></s:Envelope>"));
        let soap_action = format!("\"{}#{action}\"", self.service);
        let (status, reply) = http(
            "POST",
            &self.control,
            &[
                ("Content-Type", "text/xml; charset=\"utf-8\""),
                ("SOAPAction", &soap_action),
            ],
            &body,
        )?;
        if status == 200 {
            return Ok(reply);
        }
        match tag(&reply, "errorCode").and_then(|c| c.parse().ok()) {
            Some(code) => Err(UpnpError::Code(code)),
            None => Err(UpnpError::Io(io::Error::other(format!("HTTP {status}")))),
        }
    }
    fn add(
        &self,
        external: u16,
        internal: u16,
        client: Ipv4Addr,
        lease: u32,
    ) -> Result<(), UpnpError> {
        self.soap(
            "AddPortMapping",
            &[
                ("NewRemoteHost", String::new()),
                ("NewExternalPort", external.to_string()),
                ("NewProtocol", "TCP".into()),
                ("NewInternalPort", internal.to_string()),
                ("NewInternalClient", client.to_string()),
                ("NewEnabled", "1".into()),
                ("NewPortMappingDescription", DESCRIPTION.into()),
                ("NewLeaseDuration", lease.to_string()),
            ],
        )
        .map(|_| ())
    }
    fn delete(&self, external: u16) -> Result<(), UpnpError> {
        self.soap(
            "DeletePortMapping",
            &[
                ("NewRemoteHost", String::new()),
                ("NewExternalPort", external.to_string()),
                ("NewProtocol", "TCP".into()),
            ],
        )
        .map(|_| ())
    }
    fn external_ip(&self) -> io::Result<IpAddr> {
        let r = self.soap("GetExternalIPAddress", &[])?;
        tag(&r, "NewExternalIPAddress")
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| io::Error::other("no external address"))
    }
}

// ----- The default gateway ----------------------------------------------------------------------

/// The IPv4 default gateway from the OS routing table.
pub fn default_gateway() -> Option<Ipv4Addr> {
    #[cfg(target_os = "linux")]
    {
        linux_gateway(&std::fs::read_to_string("/proc/net/route").ok()?)
    }
    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd"))]
    {
        let out = std::process::Command::new("route")
            .args(["-n", "get", "default"])
            .output()
            .ok()?;
        bsd_gateway(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("route")
            .args(["print", "-4", "0.0.0.0"])
            .output()
            .ok()?;
        windows_gateway(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        windows
    )))]
    {
        None
    }
}
pub fn linux_gateway(table: &str) -> Option<Ipv4Addr> {
    table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() > 2 && f[1] == "00000000" {
            let g = u32::from_str_radix(f[2], 16).ok()?;
            (g != 0).then(|| Ipv4Addr::from(g.swap_bytes()))
        } else {
            None
        }
    })
}
pub fn bsd_gateway(text: &str) -> Option<Ipv4Addr> {
    text.lines().find_map(|l| {
        l.trim()
            .strip_prefix("gateway:")
            .and_then(|g| g.trim().parse().ok())
    })
}
pub fn windows_gateway(text: &str) -> Option<Ipv4Addr> {
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() >= 3 && f[0] == "0.0.0.0" && f[1] == "0.0.0.0")
            .then(|| f[2].parse().ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    type Table = Arc<Mutex<HashMap<u16, (u16, u32)>>>;

    /// A router on loopback speaking PCP (unless `pmp_only`) and NAT-PMP;
    /// external port = internal + 1000, WAN address 198.51.100.7.
    fn mock_pmp(pmp_only: bool) -> (SocketAddr, Table) {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = s.local_addr().unwrap();
        let table: Table = Arc::default();
        let t = table.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1100];
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                let r = &buf[..n];
                let reply = match (r[0], r.get(1)) {
                    (2, _) if pmp_only => vec![0, 0x81, 0, 1],
                    (2, Some(1)) => {
                        let internal = u16::from_be_bytes([r[40], r[41]]);
                        let life = u32::from_be_bytes(r[4..8].try_into().unwrap());
                        let mut out = r.to_vec();
                        out[1] = 0x81;
                        out[2] = 0;
                        out[3] = 0;
                        let ext = if life == 0 {
                            t.lock().unwrap().remove(&internal);
                            0
                        } else {
                            t.lock().unwrap().insert(internal, (internal + 1000, life));
                            internal + 1000
                        };
                        out[42..44].copy_from_slice(&ext.to_be_bytes());
                        out[44..60].copy_from_slice(
                            &Ipv4Addr::new(198, 51, 100, 7).to_ipv6_mapped().octets(),
                        );
                        out
                    }
                    (0, Some(0)) => vec![0, 128, 0, 0, 0, 0, 0, 1, 198, 51, 100, 7],
                    (0, Some(2)) => {
                        let internal = u16::from_be_bytes([r[4], r[5]]);
                        let life = u32::from_be_bytes(r[8..12].try_into().unwrap());
                        let ext = if life == 0 {
                            t.lock().unwrap().remove(&internal);
                            0
                        } else {
                            t.lock().unwrap().insert(internal, (internal + 1000, life));
                            internal + 1000
                        };
                        let mut out = vec![0, 130, 0, 0, 0, 0, 0, 1];
                        out.extend_from_slice(&internal.to_be_bytes());
                        out.extend_from_slice(&ext.to_be_bytes());
                        out.extend_from_slice(&life.to_be_bytes());
                        out
                    }
                    _ => continue,
                };
                let _ = s.send_to(&reply, from);
            }
        });
        (addr, table)
    }

    #[test]
    fn pcp_maps_renews_and_deletes() {
        let (gw, table) = mock_pmp(false);
        let targets = Targets {
            pmp: vec![gw],
            upnp: vec![],
        };
        let mut m = map(&targets, Ipv4Addr::new(192, 168, 1, 20), 25580, LEASE).unwrap();
        assert_eq!(m.method, Method::Pcp);
        assert_eq!(m.external, "198.51.100.7:26580".parse().unwrap());
        assert_eq!(table.lock().unwrap()[&25580], (26580, 3600));
        assert!(!m.due());
        m.renew().unwrap();
        m.remove();
        assert!(table.lock().unwrap().is_empty());
    }

    #[test]
    fn nat_pmp_when_pcp_is_unsupported() {
        let (gw, table) = mock_pmp(true);
        let targets = Targets {
            pmp: vec![gw],
            upnp: vec![],
        };
        let m = map(&targets, Ipv4Addr::new(192, 168, 1, 20), 25580, LEASE).unwrap();
        assert_eq!(m.method, Method::NatPmp);
        assert_eq!(m.external, "198.51.100.7:26580".parse().unwrap());
        m.remove();
        assert!(table.lock().unwrap().is_empty());
    }

    /// An IGD on loopback: description, then SOAP. `permanent_only` answers
    /// 725 to leases; `taken` ports answer 718.
    fn mock_igd(
        permanent_only: bool,
        taken: u16,
    ) -> (String, Arc<Mutex<HashMap<u16, (u16, String, u32)>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let table: Arc<Mutex<HashMap<u16, (u16, String, u32)>>> = Arc::default();
        let t = table.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                // Read the headers, then Content-Length bytes.
                loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&req);
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if body.len() >= len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&req).into_owned();
                let (status, body) = if text.starts_with("GET /desc.xml") {
                    (200, "<root><device><serviceList><service><serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType><controlURL>/wrong</controlURL></service><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl</controlURL></service></serviceList></device></root>".to_owned())
                } else if text.contains("#GetExternalIPAddress") {
                    (200, "<s:Envelope><s:Body><u:GetExternalIPAddressResponse><NewExternalIPAddress>198.51.100.8</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>".to_owned())
                } else if text.contains("#AddPortMapping") {
                    let ext: u16 = tag(&text, "NewExternalPort").unwrap().parse().unwrap();
                    let int: u16 = tag(&text, "NewInternalPort").unwrap().parse().unwrap();
                    let client = tag(&text, "NewInternalClient").unwrap().to_owned();
                    let lease: u32 = tag(&text, "NewLeaseDuration").unwrap().parse().unwrap();
                    if permanent_only && lease != 0 {
                        (500, "<s:Fault><detail><UPnPError><errorCode>725</errorCode></UPnPError></detail></s:Fault>".to_owned())
                    } else if ext == taken {
                        (500, "<s:Fault><detail><UPnPError><errorCode>718</errorCode></UPnPError></detail></s:Fault>".to_owned())
                    } else {
                        t.lock().unwrap().insert(ext, (int, client, lease));
                        (200, "<ok/>".to_owned())
                    }
                } else if text.contains("#DeletePortMapping") {
                    let ext: u16 = tag(&text, "NewExternalPort").unwrap().parse().unwrap();
                    t.lock().unwrap().remove(&ext);
                    (200, "<ok/>".to_owned())
                } else {
                    (404, String::new())
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 {status} X\r\nContent-Type: text/xml\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
                    body.len()
                );
            }
        });
        (format!("http://127.0.0.1:{port}/desc.xml"), table)
    }

    #[test]
    fn upnp_maps_and_deletes() {
        let (location, table) = mock_igd(false, 0);
        let targets = Targets {
            pmp: vec![],
            upnp: vec![location],
        };
        let client = Ipv4Addr::new(192, 168, 1, 20);
        let mut m = map(&targets, client, 25580, LEASE).unwrap();
        assert_eq!(m.method, Method::Upnp);
        assert_eq!(m.external, "198.51.100.8:25580".parse().unwrap());
        assert_eq!(
            table.lock().unwrap()[&25580],
            (25580, "192.168.1.20".into(), 3600)
        );
        m.renew().unwrap();
        m.remove();
        assert!(table.lock().unwrap().is_empty());
    }

    #[test]
    fn upnp_permanent_leases_and_taken_ports() {
        let (location, table) = mock_igd(true, 25580);
        let targets = Targets {
            pmp: vec![],
            upnp: vec![location],
        };
        let m = map(&targets, Ipv4Addr::new(10, 0, 0, 2), 25580, LEASE).unwrap();
        // 725 → no lease; 718 on 25580 → another port.
        assert_ne!(m.external.port(), 25580);
        let entry = table.lock().unwrap()[&m.external.port()].clone();
        assert_eq!(entry, (25580, "10.0.0.2".into(), 0));
        m.remove();
        assert!(table.lock().unwrap().is_empty());
    }

    #[test]
    fn nothing_answering_says_why() {
        let quiet = UdpSocket::bind("127.0.0.1:0").unwrap();
        let targets = Targets {
            pmp: vec![quiet.local_addr().unwrap()],
            upnp: vec![],
        };
        let e = map(&targets, Ipv4Addr::new(10, 0, 0, 2), 1, LEASE).unwrap_err();
        assert!(e.contains("PCP") && e.contains("NAT-PMP"), "{e}");
        assert_eq!(
            map(&Targets::default(), Ipv4Addr::LOCALHOST, 1, LEASE).unwrap_err(),
            "no router answered"
        );
    }

    #[test]
    fn parsers() {
        assert_eq!(
            ssdp_location(
                "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nLOCATION: http://192.168.1.1:5000/rootDesc.xml\r\nST: x\r\n\r\n"
            ),
            Some("http://192.168.1.1:5000/rootDesc.xml".into())
        );
        assert_eq!(
            split_url("http://192.168.1.1:5000/a/b"),
            Some(("192.168.1.1".into(), 5000, "/a/b".into()))
        );
        assert_eq!(
            split_url("http://[fe80::1]/rootDesc.xml"),
            Some(("fe80::1".into(), 80, "/rootDesc.xml".into()))
        );
        assert_eq!(
            split_url("http://[fe80::1]:5000/x"),
            Some(("fe80::1".into(), 5000, "/x".into()))
        );
        assert_eq!(
            split_url("http://router/"),
            Some(("router".into(), 80, "/".into()))
        );
        assert_eq!(
            linux_gateway(
                "Iface\tDestination\tGateway \tFlags\neth0\t0001A8C0\t00000000\t0001\neth0\t00000000\t0101A8C0\t0003\n"
            ),
            Some(Ipv4Addr::new(192, 168, 1, 1))
        );
        assert_eq!(
            bsd_gateway("   route to: default\ndestination: default\n    gateway: 192.168.1.254\n"),
            Some(Ipv4Addr::new(192, 168, 1, 254))
        );
        assert_eq!(
            windows_gateway(
                "Network Destination        Netmask          Gateway       Interface  Metric\n          0.0.0.0          0.0.0.0      10.0.0.138      10.0.0.12     25\n"
            ),
            Some(Ipv4Addr::new(10, 0, 0, 138))
        );
        assert_eq!(
            tag("<a:Body><x:Name attr=\"1\"> v </x:Name></a:Body>", "Name"),
            Some("v")
        );
    }
}
