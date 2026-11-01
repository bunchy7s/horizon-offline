# Opt-in data collection

Horizon collects nothing unless the player says yes. The launcher asks once, with four separate switches that are all off until the player turns one on, and the same switches stay in Settings. The game and the launcher obey the same answer. The code is in `crates/horizon-telemetry`.

The one exception is the anonymous ["I'm playing" ping](#the-im-playing-ping) behind the launcher's "N playing now": it holds nothing about the player, and the launcher says so next to the switches.

## Consent

`telemetry/consent.json` in the game folder (`HORIZON_HOME`, else `horizon` in the platform data folder):

```json
{ "version": 2, "answered": 1792060800, "choices": { "performance": true, "crashes": true, "bugs": false, "benchmarks": false }, "install_id": "3f2c…" }
```

- `version` is the consent version the player answered (`CONSENT_VERSION`). It goes up when what is collected changes. The launcher then asks again, and until the player answers nothing new is kept or sent.
- `answered` is when they answered, in seconds since 1970 (UTC).
- `install_id` is 128 random bits, made the first time the player says yes to anything. It ties their items together so they can be deleted, and nothing else. "Delete my data" asks the server to delete everything under it, then replaces it.

Nothing leaves the machine without consent:

- `outbox::push` keeps an item only if its kind is allowed.
- `upload::flush` reads the consent again before sending and deletes any waiting item whose kind the player has turned off since, so with everything off it never opens a connection.

`tests/consent_gating.rs` checks this against a local mock server that counts connections.

## What is sent

Every item has `day` (days since 1970, UTC: no finer time), `source` (`launcher` or `game`), `version` (the release), `kind` and `data`.

| Kind | Switch | `data` | Largest | A day |
|---|---|---|---|---|
| `perf` | performance | `system`, `settings` (render and simulation distance, max FPS, vsync, fullscreen, graphics mode), `seconds` played, `frames` and `ticks` summaries, `peak_ram_mb`, `to_title_ms`, `to_world_ms` | 4 KiB | 12 |
| `crash` | crash reports | `system`, `error`, `stack` (one frame a line), `thread`, the graphics and settings details, all scrubbed. No log, no world, no server | 32 KiB | 5 |
| `bug` | bug reports | `system`, the player's `text` (up to 4000 characters), and the `log` only if they add it after reading it | 80 KiB | 5 |
| `benchmark` | benchmark uploads | the benchmark report as `docs/benchmark-report.md` defines it, as it is, with its own schema field | 128 KiB | 10 |

`system` describes the machine, with what can explain its performance. A field the OS doesn't give is empty or 0:

| Field | What | Example |
|---|---|---|
| `os`, `os_version`, `arch` | the OS, its version, the CPU architecture | `linux`, `Fedora Linux 43`, `x86_64` |
| `cpu`, `threads`, `cores` | the CPU model, its threads and physical cores | `13th Gen Intel(R) Core(TM) i7-13700KF`, 24, 16 |
| `cpu_base_mhz`, `cpu_boost_mhz` | base and boost clock, where the OS gives them (not on Apple silicon) | 3400, 5400 |
| `ram_gb` | installed memory, rounded to the GiB | 32 |
| `gpu`, `gpu_vendor`, `gpu_kind` | the GPU the game draws with, its maker, `integrated` or `discrete` | `NVIDIA GeForce RTX 4080`, `nvidia`, `discrete` |
| `vram_mb` | a discrete GPU's own memory, rounded up to 256 MiB | 16384 |
| `driver`, `backend` | the driver version and the graphics API (`Vulkan`, `Metal`, `Dx12`) | `580.95.05`, `Vulkan` |
| `storage` | the drive under the game folder: `nvme`, `ssd` or `hdd` | `nvme` |
| `display` | the screen the window opened on: `width`, `height` (physical pixels), `refresh_hz`, `scale` | 2560, 1440, 165, 1.0 |
| `power`, `form` | on mains or battery (`ac`, `battery`), `laptop` or `desktop` | `ac`, `desktop` |

The launcher's own items (bug reports sent from it) have no GPU or screen: only the game knows them. The memory given to the game is not a setting in Horizon (no Java heap): `peak_ram_mb` is what it used. Benchmark reports carry the same facts in their `hardware` (`docs/benchmark-report.md`).

No serial number, host name, user name, MAC address or path is read. Settings → Data collection → "View what would be sent" and `horizon-launcher data show` print every field as it would leave this machine.

A summary (`frames`, `ticks`) is the count, the 50th, 95th and 99th percentile and the maximum in milliseconds (to the tenth), and 14 bucket counts: under 4 ms, 4–6, 6–8, 8–10, 10–12, 12–14, 14–16.7, 16.7–20, 20–25, 25–33.3, 33.3–50, 50–100, 100–250, and 250 ms and more. Raw frame times never leave: `Timings` records them in tenths of a millisecond (20 KB however long the session) and only its summary is kept. A perf item is under 1 KiB.

Never sent: account names or UUIDs, world names, server or LAN addresses, chat, file paths with the user's name, IP addresses. `scrub::Scrubber` replaces the user folder in paths (`/Users/<user>/…`, `C:\Users\<user>\…`), the user name, e-mail and IP addresses and UUIDs. It also replaces the terms the caller passes (the world's name, the server's address, account names). The server does not store the sender's IP address.

## Storage and sending

- `telemetry/outbox/` holds one JSON file per item, `<day>-<kind>-<random>.json`. It never holds more than 200 items or 2 MiB, and the oldest go first. `kept.json` counts today's items per kind against the caps above.
- The launcher sends the outbox when it starts and when it closes. Items go as one JSON batch (`{"schema":1,"install_id":…,"items":[…]}`, up to 64 items and 1 MiB), compressed with zstd: `POST /v1/batch` with `Content-Encoding: zstd`. A day's perf summaries are under 2 KB on the wire.
- After a failure the launcher waits a minute, doubling up to a day, and sends at most 8 batches a day. A batch the server refuses with a 4xx (not 429) is dropped.
- The server is `https://telemetry.playhorizon.dev` (`DEFAULT_ENDPOINT`), on Railway in the EU (`europe-west4`, Amsterdam). `HORIZON_TELEMETRY_URL` names another one (a local server); `HORIZON_TELEMETRY_URL=off` sends nothing at all, no ping either. Tests, CI and the benchmark and test harnesses set `off`.
- What is sent still depends on the switches: with every switch off nothing but the ping leaves, and the outbox stays empty.
- `SCHEMA` versions the format. New optional fields keep it. A field that changes meaning or goes bumps it, and the server keeps reading the older ones.

## The "I'm playing" ping

Ludovic decided on 2026-10-22 to count every player, with or without consent, and to tell them. The launcher's first-run question and Settings → Data collection show it as an "Always on" row next to the switches. Players who answered the question before the ping existed see the same text once on the Play page. The privacy page on playhorizon.dev says the same.

While the game runs (not a dedicated server, a benchmark or a run that quits by itself), a thread of its own (`crates/horizon-client/src/heartbeat.rs`) sends `POST /v1/live` once a minute. It also sends one at once when a world is opened to or closed from LAN or friends:

```json
{ "session": "9b1c…", "version": "v2026.10.22", "platform": "macos", "lan": false }
```

- `session` is 128 random bits made when the game starts. It is kept only in the game's memory and is not the install ID, so two sessions of one player can't be linked.
- `version` is the release, `platform` is `linux`, `macos`, `windows` or `other`, and `lan` says whether a LAN or play-with-friends world is open.
- Nothing else: no account, install ID, world, server, settings or hardware. The server checks each field (`live::Ping::check`) and refuses anything more.

The server keeps sessions in memory only, never on disk, and never with the sender's address. A session counts while its last ping is under 2 minutes old. `GET /v1/live` gives the exact count; versions past the 12 most played are summed under `other`:

```json
{ "playing": 1243, "lan": 87, "versions": { "v2026.10.22": 1102, "other": 141 }, "platforms": { "linux": 301, "macos": 238, "windows": 704 } }
```

Once a minute the server writes down only those totals (playing, LAN, per platform; no session, version or address) for the history at [playhorizon.dev/stats](https://playhorizon.dev/stats): every minute for 7 days, then per hour (average and peak) for 90 days, then per day (average and peak) for good. `GET /v1/live/history?range=24h|7d|30d|all` serves it (`&series=official`: the official server's count instead, below):

```json
{ "source": "players", "range": "24h", "step": 300, "from": 1793404800, "to": 1793491260,
  "series": [[1793404800, 41.2, 44], [1793405100, 43.0, 47]],
  "peak": { "playing": 1243, "at": 1793480400 }, "all_time": { "playing": 1243, "at": 1793480400 },
  "latest": { "playing": 1190, "at": 1793491200 },
  "platforms": { "linux": 290.5, "macos": 230.1, "windows": 690.4 } }
```

`series` holds `[start, average, peak]` per point (5 minutes for 24h, an hour for 7d, 3 hours for 30d, a day for all; UTC Unix seconds), `peak` the range's highest minute, `all_time` the highest ever, `latest` the newest minute, `platforms` the range's average per platform.

The server also asks the official server (`horizon_telemetry::server_status::OFFICIAL_STATUS_ADDRESS`, its address with the port its SRV record gives, or `HORIZON_TELEMETRY_OFFICIAL_SERVER`) for its player count every minute, with the server-list status ping every client sends to show a server in its Multiplayer list, and keeps it the same way (`series=official`, playing only; a minute it didn't answer stays empty). Its TPS and MSPT come later, once its admin API is reachable from outside the machine.

The launcher reads the count when it opens and every minute after, and shows "N playing now" on the Play page, exact, with thousands separators; clicking it opens playhorizon.dev/stats. Beside it, "M on the official SMP" is the official server's own count, from the same status ping the launcher sends once a minute (3-second timeout, nothing shown when it doesn't answer); clicking it opens the Servers page. It shows nothing when the server can't be reached. `HORIZON_TELEMETRY_URL=off` turns both off; `HORIZON_NO_HEARTBEAT=1` keeps a single automated run out of the count.

## Server

- `POST /v1/batch`
- `DELETE /v1/installs/<install_id>`: deletes everything sent under that ID.
- `GET /v1/aggregate`: the anonymous totals the website's results page reads. Its `breakdowns` split the play sessions of the period by hardware class (below), each class with its installs, sessions, hours and mean p50 and p99 frame times.
- `GET /v1/benchmarks/summary`: players' benchmark uploads grouped by GPU, CPU, render distance and game for playhorizon.dev/benchmarks. A group shows only with at least 3 different installs, and only its medians and counts: no install ID, day, OS version, driver, exact memory or screen. Uploads are never published one by one. Each group's `breakdowns` count its installs per hardware class.
- `POST /v1/live` and `GET /v1/live`: the "I'm playing" ping and the live count.
- `GET /v1/live/history?range=24h|7d|30d|all&series=players|official`: the live count, or the official server's, over time (above).

Public output never holds a raw hardware value, only these classes (`src/classes.rs`, the same in the website's `scripts/benchmarks.mjs`, horizon-rust/horizon-site), and a class is published only with at least 3 different installs:

| Breakdown | Classes |
|---|---|
| `ram` | 4 GB or less, 8 GB, 16 GB, 32 GB, 64 GB or more |
| `vram` | shared (integrated GPU), 2 GB or less, 4, 6, 8, 12, 16 GB, more than 16 GB |
| `storage` | nvme, ssd, hdd |
| `os` | linux, macos, windows |
| `gpu_kind` | integrated, discrete |

The driver, OS version, clocks, screen and power are kept raw for the 180 days of retention, to explain a slow group, and never published.

`crates/horizon-telemetry-server` is the server, at `https://telemetry.playhorizon.dev` on Railway (one SQLite file on a volume, region `europe-west4`). Its [README](../crates/horizon-telemetry-server/README.md) covers storage (about 190 bytes per play session), retention (180 days of raw rows, anonymous daily totals kept), rate limits and hosting.
