# Velocity player information forwarding

How `horizon-server` joins a network behind a Velocity proxy (`crates/horizon-client/src/lan/forwarding.rs`,
the owner's steps in [hosting.md](hosting.md#behind-a-velocity-proxy)). Horizon-only: vanilla has no such
thing, so there is no vanilla reference and no parity harness.

## Where this comes from

Velocity's documentation covers only the configuration of forwarding. The exchange below was worked out by
watching a real Velocity 4.3.1 (offline mode, modern forwarding) in front of a stand-in backend that asks for
the forwarding and records the proxy's answer (`tools/velocity_forwarding_probe.py`). No source of Velocity,
Paper or any other project was read for it. (A web search made while planning returned pages of such source
and a summary that mentioned the general shape, a signature followed by the player's data; none of those pages
was opened, and every detail here was checked against the recording.)

## The exchange

1. The proxy connects like a client (handshake, then Login Start with the player's name and UUID).
2. The server sends a login plugin request (clientbound login packet 4) on the channel
   `velocity:player_info` with one data byte: the highest forwarding version it reads.
3. The proxy answers with a login plugin response (serverbound login packet 2) whose payload is
   `signature` (32 bytes) followed by `data`.
4. `signature` is the HMAC-SHA256 of `data` with the shared secret (checked on every recording).
5. `data`: `version` (VarInt), `address` (string), `uuid` (16 bytes), `name` (string), then the profile
   `properties` (VarInt count, each a name, a value, a bool, and the signature string when the bool is set).

Horizon asks for version 1 and accepts only version 1.

## What was observed

| Request data | Answer's version | Payload |
|---|---|---|
| `01` | 1 | 67 bytes for the name `Tester` and no properties |
| `04` | 4 | 67 bytes, the same fields |
| `03`, `02`, none | 1 | 67 bytes |

For a player without a chat key the proxy falls back to version 1 when 2 or 3 is asked for, and a request with
no data is answered with version 1. The recording used for the unit test is a version 1 answer.

**Not observed:** non-empty `properties`. An offline proxy forwards none. Their encoding follows the profile
properties the login already sends (`login::Property`: name, value, optional signature) and is covered by a
unit test built from that layout only; checking it against a proxy that forwards a skin is still to do.

## Refusals

A login that doesn't answer (a client that connects directly), has a wrong signature, another version, an
address that isn't an IP, or trailing bytes is refused with "This server is behind a proxy: join through the
proxy."; the reason goes to the log as a warning.
