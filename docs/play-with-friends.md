# Play with friends

A player opens their single-player world to friends with one click and gets an **invite code**.
Friends paste the code (in game: the title screen's Friends button opens Join a Friend, and Direct
Connection takes a full code too; or on the launcher's Play page) and join.
There is no Horizon server in the middle and no account: the code itself says where the host is and
holds the key that encrypts the game between them.

It is built on Open to LAN: the integrated server (`crates/horizon-client/src/lan`) listens on a local
TCP port and speaks the vanilla protocol. Play with friends puts an encrypted tunnel in front of that
port. Neither the integrated server nor the guest's client changes: the tunnel carries exactly the
bytes vanilla's protocol would send, between a loopback port on each side.

```
 guest's game                                                              host's game
 ┌──────────────┐ 127.0.0.1:A ┌────────────┐   TCP, end-to-end encrypted   ┌────────────┐ 127.0.0.1:LAN ┌──────────────┐
 │ client       │────────────▶│ guest      │══════════════════════════════▶│ host       │──────────────▶│ integrated   │
 │ (unchanged)  │             │ tunnel     │  to an address from the code  │ tunnel     │               │ server (LAN) │
 └──────────────┘             └────────────┘                               └────────────┘               └──────────────┘
                                     ╲            optional: a relay anyone can run         ╱
                                      ╲──────────▶ horizon-relay (short codes, fallback) ◀╱
```

## How it works

**Hosting.** The pause menu's Friends button (vanilla's icon, beside Player Reporting) opens **Play
with Friends** (`friends_screen.rs`); its **Start Sharing** opens the world to LAN if it isn't (Open to
LAN's defaults), then the host's game, on its own:

1. listens on its **direct port**, TCP 25580 by default (the same port every time, so a port forwarded
   by hand keeps working; another one if it is taken; `HORIZON_FRIENDS_PORT` picks it);
2. collects this computer's addresses: the LAN address, VPN addresses (Tailscale's `100.x`, ZeroTier
   and others), IPv6. Container and VM bridges are left out;
3. asks the home router to open the direct port for an hour, renewed every half hour and deleted when
   sharing stops: **PCP** (RFC 6887), then **NAT-PMP** (RFC 6886) on the default gateway, then **UPnP
   IGD** (SSDP discovery, SOAP `AddPortMapping`). A router that only takes permanent UPnP mappings gets
   one, deleted on stop the same way (if the game crashes instead, a timed mapping expires within the
   hour; a permanent one stays until it is removed in the router's settings);
4. asks public **STUN** servers for its public IPv4 address (only the address: STUN sees UDP, the game
   is TCP). Defaults: `stun.cloudflare.com:3478` and `stun.l.google.com:19302`, two independent
   operators of free STUN at a global scale, so one outage doesn't break discovery. `HORIZON_STUN_SERVERS`
   (a comma list, or `none`) changes them;
5. builds the **invite code**: the endpoints (the router mapping, the public address on the direct
   port, the interface addresses) and a fresh 80-bit secret, in base32 with a checksum.

**Joining.** The guest's game dials every endpoint in the code at once. The first that answers as a
Horizon host gets the handshake, the others are dropped. On the same LAN, over a VPN, with IPv6, with a
router that opened the port, or with a forwarded port, that's it: no server took part.

**Codes.** Crockford base32 (`0-9 A-Z` without `I L O U`; input ignores case, spaces and dashes and reads
`O` as `0`, `I`/`L` as `1`) in groups of five, with a 16-bit checksum so a typo is caught before
anything is dialled:

- a home with a router mapping, a LAN address and a tailnet address: 52 symbols
  (`2A7KQ-M3XPD-…`, 11 groups); with IPv6 too, 79;
- byte layout: version, secret (10 bytes), direct port, endpoints (tag, address, own port when it
  differs), an optional relay entry, the first two bytes of SHA-256 of all that.

A code lives as long as the host shares: stopping, closing the world or quitting ends it (the direct
port closes, the router mapping is deleted). Sharing again makes a new secret.

## When friends can't reach the host

Play with Friends says what to expect, from what the game found; when friends on the internet can't
reach the host, its **Troubleshooting...** button (beside Done) opens this section in the browser:

| What the game found | What the host reads |
| --- | --- |
| The router opened the port | "Friends anywhere can join: your router opened the port with UPnP (203.0.113.5:25580)." |
| This computer has a public address | "Friends anywhere can join … (TCP port 25580 must not be blocked by a firewall)." |
| No router answered | "Friends on your network or VPN can join. For friends on the internet: turn on UPnP on your router, or forward TCP port 25580 to 192.168.1.20, or use a relay, or a VPN such as Tailscale or ZeroTier (the code already holds your VPN address)." |
| The router is behind another NAT (the router's public side is a private or `100.64/10` address, or STUN sees another address) | "… your router is itself behind another NAT (your provider's carrier-grade NAT, CGNAT, or a second router), so opening its port isn't enough. Use a relay or a VPN such as Tailscale or ZeroTier." |

The code always holds the public address on the direct port when STUN found it, so **forwarding the
port by hand** works without a new code. A guest that can't reach any endpoint is told: "Couldn't reach
your friend's game. Ask them to read the note under their code: they may need UPnP on their router, a
forwarded port, a relay or a VPN."

## Encryption

The code's secret never leaves the code. Every session runs a password-authenticated key exchange
(SPAKE2 over Ed25519, RustCrypto's `spake2`) keyed by it and bound to a session number, then derives
per-direction ChaCha20-Poly1305 keys with HKDF-SHA256 from the exchange and two fresh randoms, with a
key confirmation first. A wrong code, a changed byte, a replayed or reordered frame ends the stream.
The same exchange runs over relayed sessions, where it matters most: the relay gets nothing to test
guesses against offline. Online guesses on the direct port face the 80-bit secret; at most 8 openings
run at once there, and each must finish within 10 seconds however slowly its bytes come.

## Privacy: who sees what

- **Friends with the code** see the host's addresses in it (LAN, VPN, public), as with any server
  address. A guest's address is seen by the host, as on any server.
- **STUN servers** see the host's public IP address and a 20-byte request, like any website it visits.
  Turn them off with `HORIZON_STUN_SERVERS=none` (the code then lacks the public address unless the
  router reported it).
- **The router** gets a port mapping named "Horizon (play with friends)".
- **A relay** (only if one is configured) sees the IP addresses of the host and the guests, the short
  code's first half, when sessions start and end and the size of encrypted frames. It can't see the
  secret, the key or the game (names, chat, the world). It logs counts every five minutes, nothing else.
- **Horizon** runs no service for it. The game's anonymous "I'm playing" ping says only whether a
  LAN or friends world is open, never its address, code or name ([telemetry.md](telemetry.md)).

## Identity

Like LAN, and like vanilla's `IntegratedServer`: a host signed in with a Microsoft account runs the world
in online mode, so each guest signs in through Mojang's session servers and arrives with their own
UUID, skin and cape. A host playing under an offline name runs it in offline mode, where a guest joins
with the name their game uses. Either way the host's own profile is reserved. Every tunnelled guest
reaches the integrated server from `127.0.0.1`.

## Relays (optional)

For networks nothing can reach (CGNAT on both sides, no VPN), a **relay** carries the session: one small
binary, `horizon-relay`, that anyone can run on their own PC, a VPS, a Raspberry Pi or a free tier.
There is no default relay. With one configured:

- the host also registers there and gets a **short code** (`K7QM-3XPD`: a 4-symbol nameplate the relay
  allocates, a 4-symbol secret it never sees), and the full code names the relay and carries the short
  code, so guests with the full code fall back to the relay without any setting;
- guests with only the short code need the relay's address (on the Join a friend page);
- a relayed session first races direct connections both ways (candidates exchanged encrypted, TCP
  simultaneous open from the direct port), and stays on the relay only if none connects.

Configuring it: the **Relay** field on the Join a friend page (saved to `horizon.txt` as
`friendsRelay:host:port`, used for hosting too), or `HORIZON_RELAY_URL`.

### Running your own relay

The relay needs one TCP port reachable by your friends and almost nothing else (a few MB of memory, no
disk). Relayed players cost what the game sends: a join is a few MB, play is about 10–50 KB/s, flying
over new terrain up to a few hundred KB/s (capped per session).

**From a release or source** (any OS with Rust):

```
cargo build --release -p horizon-relay
PORT=7000 ./target/release/horizon-relay
```

**With Docker** (build context: the repository root):

```
docker build -f crates/horizon-relay/Dockerfile -t horizon-relay .
docker run -d --restart unless-stopped -p 7000:7000 horizon-relay
```

**As a service** (Linux, including a Raspberry Pi): copy the binary to `/usr/local/bin/horizon-relay`,
`crates/horizon-relay/horizon-relay.service` to `/etc/systemd/system/`, then
`systemctl enable --now horizon-relay`.

Then forward TCP port 7000 on your router to that machine (or run it on a VPS with a public address),
check `curl http://<address>:7000/health` from outside, and give friends `<address>:7000`. `GET /whoami`
shows the address the relay sees for you: behind a proxy that hides client addresses (some hosting
platforms' TCP proxies), set `RELAY_PER_ADDRESS_LIMITS=0`. Run **one** instance per address: codes live
in its memory.

Settings, all optional:

| Variable | Default | |
| --- | --- | --- |
| `PORT` | 7000 | the TCP port (also answers `GET /health` and `GET /whoami`) |
| `RELAY_BIND` | all addresses | listen on one address only (e.g. a tailnet address) |
| `RELAY_MAX_HOSTS` | 1000 | short codes at once |
| `RELAY_MAX_BRIDGES` | 200 | relayed streams at once |
| `RELAY_MAX_PENDING_PER_HOST` | 8 | guests joining one host at the same time |
| `RELAY_MAX_CONNECTIONS` | 4096 | connections open at once, of every kind (more are closed unanswered) |
| `RELAY_SESSION_KBPS` | 1024 | KiB/s each way per relayed stream |
| `RELAY_TOTAL_MBPS` | 100 | Mbit/s for everything relayed |
| `RELAY_GLOBAL_UNKNOWN_PER_MIN` | 120 | unknown short codes a minute, all clients together |
| `RELAY_PER_ADDRESS_LIMITS` | 1 | per-address rate limits (see below) |
| `RELAY_PROXY_PROTOCOL` | 0 | read the client address from a PROXY protocol v1 line |

Fixed limits: per client address (IPv6 by /64), 6 host registrations a minute (burst 3), 12 joins a
minute (burst 6), 5 unknown short codes in 10 minutes block joins for 10 minutes; a relayed stream idle
for 120 s is closed (the game's keep-alive sends every 15 s); the first line must be complete within 10 s;
hosts answer pings within 60 s; five wrong secrets on a short code and the host retires it for a new one.

### The relay protocol

One TCP port. The first line of each connection is a JSON object (at most 4 KiB, within 10 seconds); a
connection starting with `GET ` gets an HTTP answer instead.

| From | Line | Meaning |
| --- | --- | --- |
| host | `{"op":"host","v":1,"protocol":P,"game":"26.3"}` | register; answer `{"op":"registered","nameplate":"K7QM","token":T,"observed":"ip:port"}` |
| guest | `{"op":"join","v":1,"nameplate":"K7QM","protocol":P,"game":"26.3"}` | resolve; the relay announces the session to the host and waits for its attach, then answers `{"op":"bridged","session":S,"observed":…}` or `{"op":"error","code":…}` |
| relay → host | `{"op":"join","session":S}` | open a new connection and attach |
| host | `{"op":"attach","v":1,"session":S,"token":T}` | paired with the guest; both get `bridged`, then raw bytes |
| relay ↔ host | `{"op":"ping"}` / `{"op":"pong"}` | every 20 s |

Errors: `unknown_code`, `ended` (kept 15 minutes after a host leaves), `version_mismatch` (with the
host's version), `host_unreachable`, `full`, `rate_limited`, `bad_request`.

## Troubleshooting

- **"Friends on your network or VPN can join"**: turn on UPnP (sometimes "UPnP IGD" or "NAT-PMP") in
  the router's settings and share again; or forward TCP port 25580 (the panel shows the port and your
  LAN address) to this computer; or install Tailscale or ZeroTier on both computers (the code already
  holds the VPN address); or run a relay.
- **CGNAT / a second router**: port forwarding on your own router can't help. Use a VPN or a relay (on a
  machine with a public address, e.g. a VPS).
- **"Couldn't reach your friend's game"** (guest): the host's note under the code says why. A firewall
  on the host computer can also block the direct port: allow `horizon-client` on private and public
  networks (Windows asks the first time; on macOS answer "Allow" when asked whether Horizon may accept
  incoming network connections).
- **"The code has a typo"**: copy it again; codes are only letters and digits in groups of five.
- **"This short code works through your friend's relay"**: enter the relay's address under Join a
  friend, or ask for the full code.
- **"Your friend plays Minecraft X; you have Y"**: both need the same game version.

## Testing

- Unit tests (`horizon-friends`): code format (round trips, every single-symbol typo caught, versions),
  STUN against a mock server, PCP / NAT-PMP against a mock router (map, renew, delete; NAT-PMP when PCP
  is unsupported), UPnP against a mock IGD (description, chunked replies, `AddPortMapping`, permanent-only
  leases (725), taken ports (718), `DeletePortMapping`), the verdicts and endpoint order, gateway
  parsing for Linux, macOS and Windows, the key exchange and frames (wrong secret, tampering), the direct
  race.
- `crates/horizon-relay/tests/tunnel.rs`: the code alone with no server (1 MiB, two guests at once); a
  self-hosted relay with the short code (relayed, then its direct race); dead endpoints falling back to
  the relay named in the code; wrong secrets (full and short codes, the short code retired after five);
  unknown and ended codes, version mismatch, rate limits, bandwidth caps, `/health`.
- `tools/test_friends.py`: real games: a host Horizon sharing a world and a guest Horizon joining over
  plain LAN (`--baseline`), with the **code alone** (no relay running), and through a **locally run
  relay** with the short code; chat, movement and blocks both ways, then the ended invite.
- Two machines over a tailnet with the code alone (`--role host` on one, `--role guest --code …` on the
  other).
