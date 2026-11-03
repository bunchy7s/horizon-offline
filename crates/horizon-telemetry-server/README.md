# horizon-telemetry-server

This server receives what players choose to send (performance summaries, crash and bug reports, benchmark uploads: [docs/telemetry.md](../../docs/telemetry.md)) and stores it in one SQLite file. It serves anonymous totals to the website. It also keeps the live player count from the games' anonymous "I'm playing" pings, in memory only, and takes what players send from the hub (playhorizon.dev/hub): votes and bug reports. It is one small Rust process, about 7 MB of memory when idle.

    cargo run --release                     # in this folder: its own workspace, like the launcher's
    PORT=8080 HORIZON_TELEMETRY_DB=/data/telemetry.sqlite3 horizon-telemetry-server

It runs at **https://telemetry.playhorizon.dev** (`DEFAULT_ENDPOINT` in `crates/horizon-telemetry`). The game and the launcher use it unless `HORIZON_TELEMETRY_URL` names another server, or is `off` (tests and automated runs send nothing).

## Endpoints

| | |
|---|---|
| `POST /v1/batch` | A zstd batch (`Content-Encoding: zstd`, at most 256 KiB, 1 MiB inflated, 64 items). Returns 204. Returns 400 for a bad batch, 413 for one too large, 415 if it is not zstd, 429 when over a limit. |
| `DELETE /v1/installs/<id>` | Deletes everything that install sent. Its share of the daily totals is rebuilt at once. |
| `GET /v1/aggregate?days=30` | The totals the website reads. Cached 10 minutes, with `Access-Control-Allow-Origin` set to the website. |
| `GET /v1/benchmarks/summary` | Players' benchmark uploads grouped by GPU, CPU, render distance and game, for the website's Benchmarks page (below). Cached 10 minutes, same `Access-Control-Allow-Origin`. |
| `GET /v1/export/benchmarks?after=<id>&limit=100` | Owner-only: the raw benchmark reports, oldest first, `{"reports": [{"id", "day", "version", "report"}], "next": <id>\|null}` (pass `next` as `after`). No install ID. Needs `Authorization: Bearer <TELEMETRY_EXPORT_TOKEN>`: 401 without it, 404 while the server has no token, gzip if the client accepts it, never cached. |
| `GET /v1/export/hardware?days=30&offset=0&limit=200` | Owner-only, same token: the play sessions of the last `days` per machine (CPU, threads, physical cores, RAM, GPU, drive, OS…), busiest first, with installs, sessions, hours, mean p50/p99 and the frame buckets. No minimum number of installs; no install ID. `next_offset` pages. |
| `POST /v1/live` | A game's "I'm playing" ping (JSON, at most 512 bytes). Returns 204. Returns 400 for a bad ping, 413 for one too large, 429 over the address's limit, 503 when it holds `LIVE_MAX_SESSIONS` already. |
| `GET /v1/live` | How many play now, exactly: `playing`, `lan`, `versions` (the 12 most played, the rest under `other`), `platforms`. Made again at most every 15 seconds, with the same `Access-Control-Allow-Origin`. |
| `GET /v1/live/history?range=24h\|7d\|30d\|all&series=players\|official` | The live count (`players`, the default) or the official server's player count (`official`) over time for playhorizon.dev/stats: `series` of `[start, average, peak]`, the range's `peak` and the `all_time` one with their minute, the `latest` minute, the average per platform ([docs/telemetry.md](../../docs/telemetry.md)). Made again at most every minute (`Cache-Control: max-age=60`), on the live count's rate limit. 400 for another range or series. |
| `POST /hub/vote` | `{"item": "<roadmap id>", "voter": "<the browser's random id>", "up": true}`: a 👍 on a roadmap item, one per voter; `"up": false` takes it back. Returns `{"item", "votes"}`. 400 for a bad id, 429 over the address's limit. |
| `GET /hub/votes` | `{"votes": {"<item>": n}}`. Made again at most every minute. |
| `POST /hub/report` | A bug report from the hub's form: `{"title", "did", "happened", "expected", "version"?, "system", "name"?}`. Opens a GitHub issue at once (labels `bug` and `from-website`, the `#horizon-bugs` template, "Reported on playhorizon.dev") and returns 201 `{"id", "issue": "<url>"}`. If GitHub fails: 202 `{"id", "issue": null}`, and the upkeep tries again for a week. The same report twice in a day returns the first one's issue. 400 for an empty or too long field, 429 over the address's limit, 503 past the day's cap or without `GITHUB_TOKEN`. |
| `GET /healthz` | `ok` |

The hub's posts take JSON from the website (`OPTIONS` preflights answer with its origin). A form's hidden `website` field filled in (a bot) gets 202 and is dropped.

The totals are grouped per version, OS, GPU vendor and render distance. Each group has its sessions, install-days, hours played, the 14 frame-time buckets summed, and the mean p50 and p99 frame times. Groups with fewer than 5 install-days are left out. The totals also list crash counts per version. Benchmark uploads are published only through the summary below, never one by one.

## Benchmarks summary

`src/summary.rs` makes it with the same rules as the website's `scripts/benchmarks.mjs` (horizon-rust/horizon-site), which bakes the page's fallback copy; both are tested against the same fixtures (`tests/fixtures/benchmarks/` here, copied in the site's repository).

- Only reports that compare: a version 1 `horizon-benchmark-report` of the standard flight over the four scenes, VSync off, no frame cap, on screen, from Horizon or plain vanilla, naming its GPU and CPU.
- A group (GPU, CPU, render distance, game) is published only with **at least 3 different installs**. Each install counts once: the median of its runs, then the median across installs.
- Out go the names, the counts, the newest three versions and the medians. Never an install ID, a day, an OS version, a driver, a memory size or a screen.

```json
{
  "format": "horizon-benchmark-summary", "version": 1, "generated": "2026-10-22T20:00:00Z", "min_installs": 3,
  "groups": [{
    "gpu": "NVIDIA GeForce RTX 4080", "cpu": "13th Gen Intel Core i7-13700KF", "render_distance": 12,
    "game": "horizon", "source": "players", "installs": 3, "reports": 4, "versions": ["v0.2.1", "v0.2.0"],
    "medians": { "avg_fps": 515, "low_1pct_fps": 120, "p95_ms": 3.5, "world_ms": 4500,
                 "chunk_load_ms": 1400, "rss_peak_mb": 1450, "rss_steady_mb": 1250 }
  }]
}
```

Frame rates are whole numbers, milliseconds have one decimal under 10, megabytes are whole. Groups come most installs first. A deleted install leaves its groups at once.

## Storage

- `systems`: each distinct machine description once. Sessions point at it.
- `perf`: one row per session, in columns, its frame and tick summaries as 80-byte blobs.
- `reports`: crash and bug reports and benchmarks, their data compressed with zstd.
- `perf_daily`: the totals per day, rebuilt from `perf` when a day changes (new sessions, a deletion).

Measured: **about 190 bytes per play session** on disk, indexes included (`tests/server.rs`, 10 000 sessions). 1 000 players playing twice a day for 180 days is about 70 MB.

The live count's history (`src/history.rs`) is three small tables of totals, never sessions: `live_minute` (one row a minute, kept `LIVE_MINUTE_DAYS`, 7), `live_hour` (sum, sample count, peak and its minute, kept `LIVE_HOUR_DAYS`, 90) and `live_day` (the same per UTC day, kept for good). The server writes one sample a minute, starting a whole window (2 minutes) after it starts so a restart doesn't record a dip. About 1 MB a year. The `official_` tables hold the official server's player count the same way, from a server-list status ping every minute (`HORIZON_TELEMETRY_OFFICIAL_SERVER`, default `horizon_telemetry::server_status::OFFICIAL_STATUS_ADDRESS`, the SRV target of the official address since this server doesn't look SRV up; `off` for none); a minute it didn't answer has no row.

The live count is a table in memory: a session ID, when it last pinged, its version, platform and LAN flag, about 100 bytes a session. A session counts while its last ping is under 2 minutes old and is forgotten after that. Nothing of it reaches the disk, so a restart empties it; it is whole again a minute later.

The day stored is the day the server received an item. The sender's IP address is never written down. The per-address rate limit keeps addresses in memory for an hour at most.

The hub keeps votes as an item ID and the browser's random ID, and bug reports as their text, for `HORIZON_TELEMETRY_HUB_KEEP_DAYS` (90). Its rate limits key their buckets by a hash of the address with a key drawn at each start, in memory.

Raw rows are deleted after `HORIZON_TELEMETRY_RETENTION_DAYS` (180). The daily totals stay; they hold no install ID.

## Settings

All are environment variables:

- `PORT`
- `HORIZON_TELEMETRY_DB`
- `HORIZON_TELEMETRY_RETENTION_DAYS` (180)
- `HORIZON_TELEMETRY_MIN_INSTALL_DAYS` (5)
- `HORIZON_TELEMETRY_REQUESTS_PER_HOUR` per address (60, bursts of 6)
- `HORIZON_TELEMETRY_ITEMS_PER_DAY` per install (100)
- `HORIZON_TELEMETRY_TRUST_PROXY=1` behind a proxy (the client's address is then the proxy's `X-Real-IP`, else the last `X-Forwarded-For` entry; it is used only for the rate limits, in memory, and never stored or logged)
- `HORIZON_TELEMETRY_ALLOW_ORIGIN` (`https://playhorizon.dev`)
- `HORIZON_TELEMETRY_LIVE_PER_HOUR` per address, pings and reads (600, bursts of 60: ten players behind one address)
- `HORIZON_TELEMETRY_LIVE_WINDOW_SECONDS` (120)
- `HORIZON_TELEMETRY_LIVE_MAX_SESSIONS` (200 000)
- `HORIZON_TELEMETRY_OFFICIAL_SERVER` (`play.playhorizon.dev:24454`; `off` asks no server): the server whose player count is kept as the `official` series
- `HORIZON_TELEMETRY_LIVE_MINUTE_DAYS` (7) and `…_LIVE_HOUR_DAYS` (90, at least 31): how long the history keeps minute and hourly samples; daily ones stay
- `HORIZON_TELEMETRY_HUB_PER_HOUR` votes and vote reads per address (600, bursts of 60: a page load is one read, and several players can share an address)
- `HORIZON_TELEMETRY_HUB_POSTS_PER_HOUR` bug reports per address (20, bursts of 2)
- `HORIZON_TELEMETRY_HUB_REPORTS_PER_DAY` from everyone (30): each opens an issue
- `HORIZON_TELEMETRY_HUB_KEEP_DAYS` (90)
- `HORIZON_TELEMETRY_GITHUB_TOKEN`: a fine-grained token with Issues read and write on the repository only (unset: `POST /hub/report` answers 503 and the form points to GitHub and Discord)
- `HORIZON_TELEMETRY_GITHUB_REPO` (`horizon-rust/horizon`)
- `TELEMETRY_EXPORT_TOKEN` (or `HORIZON_TELEMETRY_EXPORT_TOKEN`): the bearer token of `/v1/export/*` (unset: those answer 404). A long random secret; requests with a wrong one count against the address's rate limit.

## Hosting

`Dockerfile` builds it from the repository root (`docker build -f crates/horizon-telemetry-server/Dockerfile .`). The database goes on a volume mounted at `/data`.

On Railway it is a service of the same project as the website:

- Source: this repository, root directory `/` (the Dockerfile needs the shared crate), config file `/crates/horizon-telemetry-server/railway.json` (Dockerfile build, health check `/healthz`, one replica in `europe-west4`, Amsterdam: one SQLite file can't be shared).
- A volume mounted at `/data`. The Dockerfile already sets `HORIZON_TELEMETRY_DB=/data/telemetry.sqlite3` and `HORIZON_TELEMETRY_TRUST_PROXY=1`; Railway sets `PORT`.
- The custom domain `telemetry.playhorizon.dev`: a `CNAME` at Porkbun to the target Railway gives.

Check it with `curl https://telemetry.playhorizon.dev/healthz`.

At Railway's prices (RAM $10/GB-month, CPU $20/vCPU-month, volumes $0.15/GB-month, egress $0.05/GB), one more service with a 1 GB volume uses about $0.35 a month:

- RAM about 15 MB: $0.15
- CPU nearly idle: under $0.05
- the volume: $0.15
- egress, the cached totals: cents

That fits inside the Hobby plan's included $5 of usage. Railway Postgres would cost a few dollars a month for its memory alone and buys nothing at this size. A bucket isn't needed: crash reports are a few KB compressed and live in the same file.
