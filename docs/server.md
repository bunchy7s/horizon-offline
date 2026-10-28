# Hosting a Horizon server: design

Horizon's dedicated server should be the easiest and lightest Minecraft 26.3 server to host. It should be one
small native program with no Java and no Python, one command to install it, one config file, and a CLI, a web
panel and a control API to manage it. At the same view and simulation distance and with the same players, it
should beat vanilla, Paper and Fabric + Lithium on TPS/MSPT, RAM, CPU, chunk generation and start time.
Gameplay stays vanilla: there are no Paper-style rule changes unless the owner turns one on.

This page covers what the server does today (measured), the design of `horizon-server`, and what ships for the
first working version (2026-10-23). The owner's guide is docs/hosting.md, also at
https://playhorizon.dev/server.

## 1. Today

### How it runs

The dedicated server is the game itself, run headless:
`horizon-client --resources <prepared> --dedicated [--nogui] [--port N] [--world NAME] [--universe DIR]
[--initSettings]`, started in the server folder (`crates/horizon-client/src/game/dedicated.rs`).

- **Startup.** It follows vanilla's `Main.main` and `DedicatedServer.initServer`. It reads `server.properties`
  and writes it straight back with vanilla's defaults filled in (`horizon_client::server`, byte-exact with
  vanilla's `Properties.store`), and refuses to start until `eula.txt` says `eula=true`. It then loads the
  operator, whitelist and ban lists in vanilla's order and format, and opens `<universe>/<level-name>` with
  the same `Game` as single player, minus the local player.
- **Network.** Players join through the play-state hub that Open to LAN uses (`crates/horizon-client/src/lan`).
  Status ping, legacy ping, compression, login gate (bans, whitelist, IP bans), operator permission levels,
  the player limit with `bypassesPlayerLimit`, MOTD and `server-icon.png` all follow `server.properties`.
- **Main loop.** It ticks at 20 TPS on one thread, with chunk generation and lighting on the worker pool.
  Chunks stay loaded around every player (`lan_update_streaming`), or around the spawn while nobody is
  online. It supports `pause-when-empty-seconds`, `player-idle-timeout` and the autosave.
- **Console (stdin).** `stop`, `save-all`, `save-on`/`save-off`, `list [uuids]`, `op`/`deop`,
  `whitelist on|off|list|reload|add|remove`, `ban`, `ban-ip`, `pardon`, `pardon-ip`, `banlist`, `kick` and
  `say` behave as in `DedicatedServer`. Every other command goes to the game's own dispatcher as the server
  (permission level 4). The log follows vanilla's Log4j pattern, in `logs/latest.log` and on stdout.
- **Player names.** `usercache.json` in the universe folder works as vanilla's `CachedUserNameToIdResolver`.
  Players are added when they join and kept for a month, and the file keeps the 1000 most recently used,
  newest first. The console finds a player by name in it, in any case. A name it doesn't hold is asked of the
  profile service (`getByName`). If the service doesn't know the name, an offline server uses the offline
  player of the lower-case name; an online server answers "That player does not exist".
  `tools/test_user_cache_parity.py` checks this against vanilla, with a mock profile service.

### Measured (2026-10-16, Mac mini M4 Pro, main at bd8dff20, release build)

The server ran in a scratch folder with `online-mode=false`, `level-seed=12345` and view and simulation
distance 10. The vanilla server was the pinned 26.3 `server.jar` on Java 25 with `-Xmx4G`.

| | Horizon (`horizon-client --dedicated`) | Vanilla 26.3 |
|---|---|---|
| Start, new world (wall clock, to `Done`) | 11.4 s (`Done (9.762s)`) | 12.5 s (`Done (2.066s)`, includes the bundler's first unpack) |
| Start, existing world (wall clock) | **5.8 s** (resources load in 1.6 s) | 10.7 s (`Done (0.446s)`: the JVM and DataFixerUpper take the rest) |
| RAM, idle with nobody online (RSS) | 1.40–1.51 GB | **0.93–0.97 GB** |
| CPU, idle with nobody online | 1.2 % of one core | **0.6 %** |
| `stop` to exit | 0.6 s | 0.6 s |
| What has to be installed | one 46.8 MB binary + 589 MB of prepared resources | Java 25 + the 62 MB jar |
| Crates linked | 384 (wgpu, winit, cpal, gilrs, arboard, naga…) | n/a |

On Linux the full game loads `libasound.so.2` (cpal) and `libudev.so.1` (gilrs) at startup. wgpu, winit
and the Wayland/X11 libraries are loaded only when a window opens. The server build (`horizon-engine`,
section 3) is the same program built without its `audio` and `controllers` features, so it needs neither
library, and a minimal server image runs it as it is.

The two clear losses are RAM and idle CPU. The dedicated server loads everything the client needs to draw
(the HUD atlas, item renderer, block-entity models, panorama, fog and music tables, entity models), and it
builds the meshing tables at startup. None of these are used without a window. Loading them is startup work,
not tick cost, so it is in scope here (see section 9).

### Joining it

- **Horizon client:** works. A headless Horizon client (`--server 127.0.0.1:PORT`, the harness mode) logged
  in, received its 49 chunks at render distance 2, chatted, saw `/list`, and left. The server's log matched
  vanilla's lines (`logged in with entity id`, `joined the game`, `[Not Secure] <Tester> …`, `lost connection:
  Disconnected`, `left the game`).
- **Vanilla 26.3 client:** checked by `tools/test_dedicated_server_parity.py`. VanillaProtocolClient, a
  headless client built from vanilla's own codecs, pings and joins Horizon's server and vanilla's from the
  same world copy. It compares every login, configuration and play packet up to the first chunk batch, the
  chunk bytes, chat, `/list`, failing commands, the console lines and the files left behind. Result on main:
  see "Harnesses on main" below.
- **The real vanilla 26.3 client** (`tools/vanilla_client.py`, from the owner's own files, dev-only) joins
  too. On 2026-10-18 it joined the server build of the game (`horizon-engine`, section 3) on the Mac mini:
  `python3 tools/vanilla_client.py --name Vanilla --join localhost:PORT`. It logged in, drew the terrain
  around spawn and stayed connected for four minutes, with no warning or error on the server. The tool
  runs on Linux under vscreen and on macOS from the logged-in screen session, even a locked one, with vsync
  off.

### What fails or is missing

| Area | Status |
|---|---|
| `online-mode=true` | **Works** (ServerLoginPacketListenerImpl): a 1024-bit RSA key pair at start ("Generating keypair"), the encryption request, the shared secret and challenge, AES/CFB8, and the session service's `hasJoined` check (with the address when `prevent-proxy-connections` is on), with vanilla's log lines and messages. `test_online_mode_parity.py` runs the same logins against vanilla's server and Horizon's with a mock session service. |
| Secure chat | **Works** (`SignedMessageChain`, `LastSeenMessagesValidator`, `MessageSignatureCache`): the chat session update and its profile key, checked against the services' `playerCertificateKeys`; signed chat, `say`, `me` and `msg`, relayed with their signatures and last-seen lists; `enforce-secure-profile` (with `online-mode`) refuses unsigned chat; and vanilla's log lines (`[Not Secure]`, the decoder's warnings, the kicks). Offline servers and players without a chat session chat unsigned, as on vanilla. `test_secure_chat_parity.py` runs the same signed and unsigned players against vanilla's server and Horizon's, with test keys and a mock services server. |
| Nether and End | **Works**: the server holds a level per dimension, and each player is in its own (portals, `execute in … tp`, respawn). `test_server_dimension_players_parity.py` checks it against vanilla. How it works, its cost and what's left: [server-levels.md](server-levels.md). |
| Refused logins | Logged as vanilla does: `Disconnecting <name> (/address): <reason>`, then `<name> (/address) lost connection: <reason>`, and the session errors (`Username '<name>' tried to join with an invalid session`, `Couldn't verify username because servers are unavailable`). |
| Gameplay for remote players | Checked against vanilla's server with a vanilla-codec client (#484, `tools/test_dedicated_server_parity.py`): a right click with an item on a block uses it once, a dead player's commands run, players on a team without friendly fire don't hurt each other, and a player's projectiles are theirs (their ender pearl takes them). The recipe book, status effects and block sync while digging (#500) work. Left: a player's projectiles don't hit other players (no bow PvP between guests), and a mob a guest kills counts in the host's statistics and advancements, not the guest's (`mob_died` runs as the host). |
| Defaults that surprise owners | `white-list=true` is vanilla 26.3's default, so a fresh server refuses everyone until they are added. That matches vanilla, so the first-run flow (section 3) asks about the whitelist up front. |
| Management | Only stdin. There is no service, no backups, no status beyond `list`, no restart after a crash, and a server can't be managed while it runs in the background. |
| Packaging | The server is the full game client. Resources come from `horizon-prepare`, which needs a JDK and the official launcher's client install, both awkward on a headless VPS. |
| Tick times | The dedicated loop never recorded its tick times in the game clock (only the windowed loop did), so `/tick query` on a dedicated server reported 0 ms. Fixed in PR 2b, which the benchmark needs (section 9). |
| Scale | Measured against vanilla on equal work (section 9, `benchmarks/server-equal-work-2026-10-17/`): 20 TPS up to 50 bots, 9 to 11 TPS at 100, where vanilla's watchdog stops it. Horizon is ahead with few players and when exploring; with 25 to 50 players near spawn vanilla's tick is shorter. |

### Harnesses on main

On the tip of the code PRs (2a to 3, rebased on b4ad6538), `tools/parity.py dedicated_server online_mode
server_files server_player_combat server_player_damage` passed all five. That includes the vanilla protocol
client joining Horizon's server and vanilla's from the same world. The full `--changed origin/main` set
(309 harnesses, since the PRs touch the client crate) had passed 60 of 60 when it was stopped for disk
space; CI runs it on every pull request.

## 2. Architecture: a supervisor and an engine

```
 owner ──► horizon-server (CLI, TUI)          browser ──► web panel           AI ──► MCP (stdio)
                 │                                │                               │
                 └────────── control API: HTTP + JSON on 127.0.0.1, token ────────┘
                                                  │
                                   horizon-server run   (the supervisor: no GPU, no game code)
                  server.toml ──► server.properties ──► │ ◄── stdout: the vanilla log
                  backups, updates, crash restart       │ ──► stdin: console commands
                                                        │ ◄─► engine endpoint: status, TPS/MSPT, players
                                                        ▼
                                   the engine: horizon-client --dedicated   (the game, headless)
```

`horizon-server` is a new crate and binary (`crates/horizon-server`). It is the only program an owner runs.
It depends only on the light crates (`horizon-core`, `horizon-protocol`, `horizon-world`, `horizon-bench`)
plus `serde`, `toml`, `toml_edit` and `zip`, which are already in the lockfile. It doesn't link wgpu, winit,
audio or controller crates. It runs the engine as a child process.

The supervisor/engine split stays even after the engine becomes headless, for these reasons:

- **Crash restart and diagnostics.** The supervisor outlives an engine crash. It keeps the crash report,
  restarts the engine with backoff, and the panel stays up to say what happened.
- **Safe updates.** An update swaps the engine binary and restarts. If the new engine fails its start check,
  the supervisor rolls back to the previous one.
- **Parity stays cheap.** The engine keeps reading vanilla's own files (`server.properties`, the four JSON
  lists, `eula.txt`) exactly as `test_dedicated_server_parity.py` checks them today. The clean config is a
  layer on top and never changes what the engine sees.
- **No conflicts.** The supervisor touches none of the game code that the tick and integrated-server work
  (perf-server-1, perf-ticks-2) are changing.

**The engine today** is `horizon-client --dedicated`. It needs one small, additive change: a status endpoint.
With `--control-socket <dir>/.horizon/engine.json` (the existing control protocol: TCP on 127.0.0.1 plus a token, on
every OS), the dedicated loop answers `server-status` with TPS, MSPT (average, P50/P95/P99 from the tick
clock `/tick query` uses), players (name, UUID, address, ping), loaded chunks and uptime. It answers
`console <line>` with the command's output.

**The headless engine** comes after perf-ticks-2's work lands: Resources split into shared and per-game, the
game built in `Game::new`, the integrated server on its own thread. On that base:

1. The dedicated path skips everything only a window uses: the HUD atlas, item renderer, block-entity and
   entity models, panorama, fog, music, and mesh tables (perf-ticks-2 already stops server-side meshing).
   This is the RAM and start-time item.
2. The game moves into a library crate, with the window parts behind a `client` feature, so
   `horizon-engine` links no wgpu or winit either. Sound and controllers are already optional (the
   `audio` and `controllers` features, on by default): `horizon-engine` today is the game built without
   them (section 3).

## 3. Packaging

**Binaries.** The release archives hold `horizon-client`, `horizon-ctl`, `horizon-prepare` and
`horizon-server`. On Linux the release also has `horizon-server-linux-x86_64.tar.gz`: `horizon-server`,
`horizon-prepare` and `horizon-engine`, the game built with `--no-default-features` (no sound, no
controllers), which links neither `libasound.so.2` nor `libudev.so.1`. `release.yml` builds it after the
client archive (`xtask package-server`) and fails if `tools/check_imports.py --server` finds either library;
CI lints the same build. `horizon-server` runs `horizon-engine` when it is beside it or on the PATH, else
`horizon-client`. Build it yourself with `cargo build --release -p horizon-client --no-default-features`
and copy `target/release/horizon-client` as `horizon-engine`. macOS and Windows have no such libraries to
avoid, so they keep the one archive.

**Game files: the owner's own copy, as with `prepare`.** Nothing of Mojang's is redistributed and nothing is
downloaded on the owner's behalf. `horizon-server` looks for prepared resources in this order:

1. `[engine] resources` in `server.toml`;
2. `resources/` in the server folder;
3. `.local/horizon-client/26.3` beside the engine or up to four folders above it: a release folder, the
   installer's `game-files` (below), or a checkout.

If none is found, `init` says where it looked. Today the owner runs `horizon-prepare --prepare --server-jar
PATH` from the release folder (it needs a JDK and the client files, as for playing), or copies a folder
prepared on their own computer to the server (`scp -r`). Planned wrappers:

- **On this machine:** `horizon-server prepare --server-jar PATH` runs `horizon-prepare` on the owner's own
  server jar plus their launcher install.
- **Prepared elsewhere:** `horizon-server import-resources DIR` copies a folder prepared on the owner's own
  computer, for example by the launcher, to a headless machine.

A server-only `prepare` (from `server.jar` alone, without the client jar or a JDK) is the follow-up that
removes the last Java step. It needs a list of which prepared files the server really reads, and that list
comes out of the headless-engine work.

**One-command install** (`packaging/server/`):

- Linux and macOS:
  `curl -fsSL https://raw.githubusercontent.com/horizon-rust/horizon/main/packaging/server/install.sh | sh`
  The script picks the release archive for the OS and architecture (on Linux x86_64 the server archive,
  on macOS arm64 the game's), downloads it from GitHub Releases, and checks it against the SHA-256 GitHub
  publishes for each asset. It never asks for sudo. Options: `--version vX.Y.Z`, `--dir` (default
  `~/.local/share/horizon`), `--bin` (default `~/.local/bin`), and `--archive FILE` to install an archive
  already on the machine (a build of your own; no download and no checksum).
  A Linux release from before the server archive installs the full game, which links ALSA's and udev's
  libraries (`libasound.so.2`, `libudev.so.1`); minimal server images often lack them (Debian and Ubuntu:
  `sudo apt install libasound2 libudev1`, `libasound2t64` on Ubuntu 24.04 and later). Then the script lists
  any that are missing, `init` warns, and `run` stops at once with what to install, rather than restarting
  a game that can't load.
- Windows:
  `irm https://raw.githubusercontent.com/horizon-rust/horizon/main/packaging/server/install.ps1 | iex`
  The script does the same into `%LOCALAPPDATA%\Horizon` (`-Version`, `-Dir`), with junctions instead of
  links (no admin rights), and adds `bin` to the user's PATH.
- **Layout.** Each release goes to `<dir>/releases/<tag>` and `<dir>/current` points at the one in use, so an
  update or a rollback is one link. `<bin>/horizon-server` and `<bin>/horizon-prepare` are two-line launchers
  that run `current`. The prepared game files live in `<dir>/game-files/horizon-client` and each release's
  `.local/horizon-client` links there, so updates keep them. The script then prints the next steps
  (`horizon-prepare`, `horizon-server init`).

**Docker** (`packaging/server/Dockerfile`, built the same way as the relay's image): a multi-stage build that
ends on `debian:bookworm-slim` as user `horizon`, with the server build as `horizon-engine`, so the image needs
no ALSA or udev. It uses volume `/data` for the server folder, takes the owner's prepared game files as a read-only
mount on `/opt/horizon/.local/horizon-client/26.3`, exposes port `25565/tcp`, and keeps the control API (and
later the panel) on 127.0.0.1 inside the container: `docker exec <name> horizon-server status|cmd|backup`.
On the first start, `HORIZON_EULA=true` and `HORIZON_WORLD`, `HORIZON_SEED`, `HORIZON_PORT`,
`HORIZON_WHITELIST`, `HORIZON_PLAYERS` and `HORIZON_OP` answer `init`'s questions; without the EULA answer
the container exits and says why. `docker stop` sends SIGTERM, which saves and stops cleanly (give it
`--stop-timeout 180`). The image is published only with Ludovic's OK (it's hosted infrastructure).

**Services.**

- `horizon-server service install` writes and enables a systemd unit (`--user`, or a system unit with
  `--system`) on Linux, a launchd agent (`~/Library/LaunchAgents/dev.horizon.server.<name>.plist`) on
  macOS, or a Scheduled Task at logon on Windows (a real Windows service needs a wrapper and comes later).
- Each runs `horizon-server run --dir <folder>` through the installer's launcher, so an update applies at the
  next restart. The supervisor restarts the engine itself, and the service manager restarts the supervisor.
  `service uninstall` stops and removes it and leaves the server folder alone.
- `service print` shows the unit or agent it would write, for owners who'd rather install it by hand.

**First run (`horizon-server init`, interactive on a terminal):**

1. **EULA.** It shows the link to https://aka.ms/MinecraftEULA and asks "Do you agree? [y/N]". Only a yes
   writes `eula=true`.
2. **World.** It asks for the world name (`world`) and an optional seed. An existing world folder, or a
   vanilla server folder, is detected and kept: its `server.properties` is imported (section 4).
3. **Port.** 25565 by default, checked free right away.
4. **Players.** It asks whether to use a whitelist (yes by default, as vanilla) and who to add first, then
   offers to make the first player an operator.
5. **Game files.** It finds or explains the prepared resources (see above).
6. It writes `server.toml`, shows the panel address and the commands to start it in the background
   (`horizon-server start`) or as a service.

Every question has a flag (`--eula`, `--world`, `--seed`, `--port`, `--whitelist`/`--no-whitelist`,
`--player NAME`, `--op NAME`, `--yes`) and an environment variable for Docker (`HORIZON_EULA`, …; flags win).

## 4. One config file: `server.toml`

The owner edits only `server.toml`. The supervisor writes `server.properties` from it before every engine
start, so the engine stays exactly vanilla. The file is short: typed sections for what owners change, and
a `[properties]` table that passes any other vanilla key through unchanged.

```toml
# Horizon server settings (docs/server.md). The server writes them into server.properties before
# every start: edit this file, not that one.

[server]
motd = "A Minecraft Server"
port = 25565
bind = ""                    # listen on every address, or only on this one (server-ip)
max_players = 20
online_mode = true           # check players' Minecraft accounts (false: anyone joins under any name)
whitelist = true             # only whitelisted players and operators can join
enforce_whitelist = false    # kick online players taken off the whitelist
view_distance = 10
simulation_distance = 10

[world]
name = "world"               # the world folder
seed = ""                    # empty: a random seed on the first start
game_mode = "survival"       # survival, creative, adventure or spectator
difficulty = "easy"          # peaceful, easy, normal or hard
hardcore = false

# Any other server.properties key, spelled as vanilla spells it, for example:
# spawn-protection = 16
[properties]

[engine]
path = ""                    # the game program; empty: the one installed beside horizon-server
resources = ""               # the prepared game files; empty: resources/ here, else the usual places
restart_on_crash = true

[backups]
every = "6h"                 # automatic backups while the server runs: 30m, 6h, 1d...; "0" turns them off
keep = 12                    # automatic backups kept (backups made by hand are never deleted)

[updates]
keep = 1                     # other releases kept by updates, besides the one in use and the one rollback goes to
```

This is the file `init` writes today (PR 4 adds `[panel] bind = "127.0.0.1:8130"` and `allow_public =
false`, section 6). The defaults are vanilla 26.3's (a test checks each one). Unknown keys are errors, as are
a typed key set again under `[properties]` and values vanilla would refuse. Planned: `backups.before_update`
(section 7).

- **Import.** `horizon-server init` in a folder with a `server.properties`, or `horizon-server import
  server.properties`, reads it with vanilla's own parser (`ServerProperties`). Keys with typed fields go into
  their sections; any other key that differs from vanilla's default goes into `[properties]`. Keys left at
  their defaults are dropped, so the TOML stays short. The original file is kept as
  `server.properties.imported`.
- **Changes made in game.** The engine writes `server.properties` itself, for example after `/whitelist on`.
  After it stops, the supervisor compares the file with what it wrote and copies any changed keys back into
  `server.toml`, so an in-game change survives the next start. Edits go through `toml_edit`, so the owner's
  comments and order are kept.
- **Changes without opening the file.** `horizon-server config set KEY=VALUE...` (the launcher's Servers
  page uses it) writes through `toml_edit`, checks the result as `server.toml` is checked, and writes nothing
  if a value is wrong. On a running server it sends what vanilla changes live through its console:
  `difficulty`, `defaultgamemode` and `whitelist on|off`; the backup settings apply at once because the
  supervisor reads them each time. Everything else waits for the next start, and the command says which. A
  real hot reload (view distance, MOTD) needs engine support and comes later.
- `server.properties`, `ops.json`, `whitelist.json`, `banned-*.json` and `world/` keep vanilla's names and
  formats. A vanilla server folder works as is, and so does going back to vanilla.

## 5. CLI and TUI

```
horizon-server init                      first run (section 3)
horizon-server run                       supervisor + engine in the foreground; the console on stdin
horizon-server start | stop | restart    in the background (detached `run`); each waits for the game
horizon-server status [--json]           up?, uptime, TPS, MSPT, players, RAM, CPU, crashes
horizon-server cmd <command…>            one console command, with its output
horizon-server logs [-f] [-n N]
horizon-server op|deop <player>
horizon-server whitelist on|off|list|add|remove [player]
horizon-server backup [NAME]             backups             restore <backup> [--yes]
horizon-server import <server.properties>
horizon-server config [--json]           config set KEY=VALUE... [--json]
horizon-server service install|uninstall|print [--system]
horizon-server panel                     the panel's address and a one-time login link
horizon-server update [--check] [--version vX.Y.Z] [--timeout S]     rollback [--timeout S]
horizon-server update --archive FILE --version NAME     an archive you have (your own build)

Planned:
horizon-server console                   attach: the live log, and send commands (Ctrl-D detaches)
horizon-server ban|pardon <player> [reason]      ban-ip|pardon-ip <address>   (`cmd ban …` works now)
horizon-server import-resources <dir>      prepare --server-jar PATH
horizon-server mcp                       MCP server on stdio (section 8)
```

- Every command takes `--dir` (the server folder; default: the current folder).
- While the server runs, management commands go through the control API, so the engine applies them as
  vanilla's console would and its lists stay right. While it's stopped, they edit the vanilla files directly
  with the same code the engine uses (`horizon_server_files::user_lists`).
- **The TUI** is `horizon-server console`: the log, a one-line header (TPS, MSPT, players, RAM) that updates
  every second, and a command line. It uses plain ANSI with no TUI framework, and falls back to plain lines
  when stdout isn't a terminal.

## 6. Web panel

**Scope (MVP, PR 4):** status (phase, uptime, version, crashes), live TPS, MSPT, RAM and CPU graphs (the last
10 minutes, sampled once a second), players (list with ping, op, kick), the console (live log plus a command
line), and backups (list, back up now, restore with confirmation), with restart and stop. The panel lives in
the supervisor, so it is up exactly when the server is: starting a stopped server stays a command (or the
service manager's job). Whitelist and bans go through the console for now.

**Updates:** when a newer release is out, the panel says so with an Update button, which runs the same
path as `horizon-server update` (section 7) and then shows how it went.

**Later:** bans and IP bans, worlds, `server.properties` import, logs and crash reports, and modpacks (with
the modpack work).

**Tech:** one HTML page with plain JS and CSS (`crates/horizon-server/panel/`), embedded in the binary with
`include_str!`. No build step, no framework, no CDN. It uses Horizon's design system tokens (`horizon-theme`'s
colours, light and dark) and draws charts on a canvas. It polls the control API's routes once a second
(`status`, `metrics?since=`, `logs?since=`), served by the same small HTTP code on `std::net`, with no async
runtime or new dependency. Server-sent events can replace the polling later if a tab ever costs too much.

**Auth: local or tailnet only, never an open admin port.**

- **Bind.** The panel listens on `127.0.0.1:8130` by default. `panel.bind` can name a tailnet address
  (100.64.0.0/10 or Tailscale's fd7a:115c:a1e0::/48) or another loopback address. Any other address, such as
  0.0.0.0 or a public IP, is refused unless `panel.allow_public = true` is set, and even then the panel logs
  a warning at every start.
- **Who may act.** The API token in `.horizon/api.json` (file mode 0600) is what the CLI and MCP use. Browsers
  get a one-time login link from `horizon-server panel`: a code good for 10 minutes, swapped for an
  `HttpOnly; SameSite=Strict` session cookie. Anyone who can read the server folder can manage the server,
  and no one else can. Sessions end when the supervisor stops.
- **Protection.** The `Host` header must name the panel itself: its address, `localhost` on loopback, or a
  tailnet name (a MagicDNS short name or `*.ts.net`). That stops DNS rebinding. Changes are POST only and need
  the `X-Horizon: 1` header, which stops CSRF. After 10 failed logins in a minute, logins are refused for a
  while. The page is served with a strict Content-Security-Policy (nothing from elsewhere) and can't be
  framed.
- **Remote access** goes over Tailscale/SSH (`ssh -L 8130:127.0.0.1:8130 host`), with no TLS or passwords to
  manage. HTTPS comes for free with `tailscale serve`, and the doc shows how to set it up.

## 7. Backups, updates and crash restart

- **Backups.** The supervisor runs `save-off`, then `save-all flush`, then zips the world plus the vanilla
  files and `server.toml` into `backups/<UTC time>-<name>.zip`, then runs `save-on`. Players stay online. Zip
  works for everyone, Windows included. It runs on the `every` schedule and on demand (later also before an
  update). It keeps the last `keep` automatic backups and never deletes one made by hand. A backup is written
  to a temporary name and renamed when complete, so a crash never leaves a half-written backup that looks
  finished. A stopped server is backed up straight from its files.
- **Restore.** `restore NAME` takes a file name, a unique prefix, or the name given to `backup` (the newest
  with it). The server stops. The current world is moved to `backups/replaced-<time>/` (never deleted), the
  backup is unpacked into a temporary folder and checked (`level.dat` is there), then moved in, and the server
  starts again if it was running. One command, or one click with confirmation in the panel.
- **Crash restart.** If the engine exits without being asked to stop, the supervisor keeps the last 200 log
  lines and the engine's crash report (`crash-reports/`), restarts it after 5 s, and doubles the wait up to
  5 minutes. It gives up after 5 crashes in 10 minutes, so it doesn't loop on a broken world, and shows the
  reason in `status` and the panel.
- **Updates** (`crates/horizon-server/src/update.rs`), for a `horizon-server` installed by `install.sh` or
  `install.ps1`. Releases ship often (release-flow.md), so updates stay opt-in: a running server looks for a
  newer release a minute after it starts and then every 6 hours, and only says so, in its log, in `status`
  and in the panel (`HORIZON_NO_UPDATE_CHECK=1` turns the check off). `update --check` asks GitHub once.
  `update`:
  1. downloads the current Nightly's archive for this computer (the only build Horizon ships since 2026-10-28;
     GitHub's latest release while there is none; on Linux the server build; `--version` picks another: a
     `vX.Y.Z` release, or a `nightly-<date>` of the last 7 days) and checks it against its SHA256SUMS, whose Ed25519
     signature must check with a key built into `horizon-server` (docs/release-flow.md, "Signed checksums"),
     signed for the Nightly channel: an unsigned or tampered build is refused. A Nightly installs as
     `nightly-<date>-<commit>`, newer than every `vX.Y.Z`; another build of the same day is an update too;
  2. unpacks it into `<dir>/releases/<tag>`, beside the others, with its game files linked to
     `<dir>/game-files` as the installers do;
  3. stops the game, backs up the world (`backups/<time>-before-<tag>.zip`), records the release in use in
     `<dir>/previous` and points `<dir>/current` at the new one;
  4. checks that the new game loads on this system (`--help`), then starts it;
  5. waits until it has logged `Done` and answered a status request, for 300 s (`--timeout`).

  If the new game doesn't load, stops, or isn't up in time, the supervisor goes back by itself: `current`
  points at the release before, `previous` is left as it was, the world is put back from the backup (the
  one the new game left is kept in `backups/replaced-<time>/`), and the game starts again on the old
  release. `rollback` switches to `previous` by hand, through the same path (the world is backed up first
  and kept as it is). The supervisor keeps running as it was: the new `horizon-server` itself takes over
  the next time the server starts (`stop` and `start`, or the service's restart). A stopped server is
  switched and its new game checked to load; it starts on the new release next time. One install serves
  every server folder on the machine, so each one runs the new release from its next start. Once the new
  game is up (or a stopped server is switched), old releases are deleted from `<dir>/releases/`: the one in
  use, the one `previous` names (where `rollback` goes), the one the running `horizon-server` itself came
  from, and the `updates.keep` newest of the others (1 by default) stay. A release that can't be deleted (a
  program still running from it on Windows) is left for the next update.

## 8. Control API and MCP

- **Control API.** The control API is the panel's HTTP API, documented as stable (`/api/v1/...`). The
  supervisor serves it on `127.0.0.1` at a port of the system's choosing, written with the token to
  `.horizon/api.json`. It's plain HTTP/1.1 on `std::net` (`crates/horizon-server/src/http.rs`), no runtime.
  - built (PR 2b, 2c): `GET status` (phase, uptime, crashes, RSS, CPU, and the engine's TPS, MSPT
    average/P50/P95/P99, players, chunks), `GET logs?since=`, `POST command {line}` (returns the output),
    `POST stop`, `POST restart`, `GET backups`, `POST backups {name}`, `POST restore {name}`, `POST update
    {tag, timeout}` (without a tag: the newest release, downloaded first; `status` has an `update` object
    that says how it went);
  - planned: `GET metrics?since=` (the panel's graphs), `GET events` (SSE: log lines and metrics),
    `GET`/`PATCH config` (server.toml fields, validated).
- **Auth.** Every call needs `Authorization: Bearer <token from .horizon/api.json>` (file mode 0600), and the
  `Host` header must be the loopback address it listens on.
- **MCP.** `horizon-server mcp` is an MCP server on stdio, for Claude Code or any MCP client run as the same OS
  user. It reads the token itself, and its tools map one to one onto the API: `status`, `players`,
  `run_command`, `logs`, `backup`, `list_backups`, `restore` (needs `confirm: true`), `start`, `stop`,
  `restart`, `get_config` and `set_config`. It's the same idea as the game's `horizon-ctl`
  (docs/agent-control.md): an AI can run and diagnose a server, and every action it takes shows in the log.
- **Vanilla's own management protocol.** Vanilla 26.3 has a server management protocol: JSON-RPC over a
  WebSocket, set up by the `management-server-*` keys in server.properties. Horizon's engine already parses
  those keys but doesn't serve the protocol. Serving it later, on the engine or the supervisor, would let tools
  written for vanilla manage a Horizon server too. That is not part of the MVP.

## 9. Benchmark plan (bots against vanilla and Paper)

The goal is the published claim: "runs 50–100 players on hardware where vanilla struggles". It must be
reproducible by anyone, and show the same gameplay on both sides.

- **Servers:**
  - Horizon (`horizon-server`);
  - vanilla 26.3 (the owner's `server.jar`);
  - Paper (a jar the owner downloads; Paper's defaults are kept, and noted as non-vanilla);
  - optionally Fabric + Lithium (via the pinned `tools/benchmark-deps/mods.lock.json`).

  Java servers run on Java 25 with `-Xms` = `-Xmx` (the same heap for all, 4 GB by default), and the heap is
  reported.
- **Same conditions:**
  - the same seed (`12345`) and a fresh world, plus a second pass on a pre-generated copy of the same world;
  - `view-distance` 10 and `simulation-distance` 10;
  - `online-mode=false` and the same `server.properties`;
  - one server at a time on the same machine, with the bots on another machine on the LAN, or pinned to other
    cores when they share one (the report records which).
- **Bots:** `horizon-bots` (PR 5), a small Rust load client on `horizon-protocol`. It handles handshake,
  offline login and configuration, accepts teleports, answers keep-alives and pings, acknowledges chunk
  batches as vanilla's client does, and sends movement at 20 Hz. The bots play in creative and fly 24 blocks
  above their spawn point: they never touch terrain, so they need no copy of the world, and no server can
  refuse a move for walking into a block. Each behaviour is deterministic per bot from a seed:
  - `idle`: hovers above spawn;
  - `walk`: flies to random points within 48 blocks of spawn at sprint speed (5.6 blocks/s);
  - `explore`: flies straight outwards at creative flying speed (22 blocks/s), which forces chunk generation;
  - `mixed`: 60 % walk, 20 % explore, 20 % idle.

  A `build` behaviour (breaking and placing blocks) needs the bots to read terrain and comes later. Runs use
  1, 10, 25, 50 and 100 bots, joining 1 per second. Bots are needed because 100 real vanilla clients can't run
  on one machine.
- **Measured** (once a second, for 5 minutes after the last bot joins, after a 1-minute warm-up):
  - TPS and MSPT (average, P50/P95/P99), from each server's own `/tick query` (vanilla's
    `ServerTickRateManager`; Horizon implements the same command and clock), so every server measures itself
    the same way;
  - RSS and CPU (process tree, `horizon_bench::rss_bytes`/`cpu_seconds`);
  - chunk generation: chunks per second while the `explore` bots run, and time until each new bot has its
    full view loaded;
  - start time: cold and warm, to `Done`;
  - bot-side: chunk arrival, keep-alive round trip, and disconnects (a run with disconnects is reported as
    failed).
- **Output.** `tools/server_bench.py` drives it: a fresh server per run from the same `server.properties`,
  `Done`, the bots, a warm-up, then the measurement. It writes a `horizon-server-benchmark` JSON report (the
  machine, commit, Java version, settings and every run) and prints a comparison table. Results are
  published like the client benchmark, with the machine, the versions and the exact commands. TPS is derived
  from `/tick query`'s average (20 while it fits in 50 ms), since the command reports times, not the rate.
- **First run (PR 5,** on a busy shared Mac mini, so the numbers are noisy). With 1 bot Horizon ticks in
  3.5 ms against vanilla's 11.2 ms and uses 1.6 GB against 3.6 GB. At 10 and 25 `mixed` bots Horizon falls
  to about 10 TPS where vanilla keeps 20, and at 50 both land at about 15 TPS. The work isn't equal yet:
  - Vanilla pulls fast players back when it lags (`moved too quickly`), which Horizon never does, so
    vanilla's `explore` bots generated less terrain.
  - Vanilla's chunk sending yields under load.
  - Horizon sent each nearby `walk` bot 1.5–4 times as many chunks.

  Horizon sent 2–4 times the chunks per second in every multi-bot run.
- **Equal-work run (2026-10-17,** `benchmarks/server-equal-work-2026-10-17/`, on the ThinkPad with the bots
  on the Mac mini). Horizon now checks moves and paces chunks as vanilla does (#586), the bots wait for the
  chunk they stand in, an `--explore-speed` pass keeps both servers at the same rate, and vanilla runs with
  `gamerule locator_bar false` (Horizon has no locator bar yet). Results:
  - Memory: Horizon's RSS is 1.9 to 2.6 times smaller in every run (vanilla's heap is fixed at 4 GB).
  - Walking near spawn: Horizon's tick is shorter with 1 and 10 bots (5.5 against 18.3 ms, 11.5 against
    16.8), and longer with 25 and 50 (20.7 against 16.0, 31.4 against 20.5). Both keep 20 TPS up to 50.
  - Exploring at the same rate: Horizon's tick is about half of vanilla's (13.5 against 29.2 ms at 10 bots,
    20.2 against 37.0 at 25).
  - At 100 bots: vanilla's watchdog stops it in every pass; Horizon runs at 9 to 11 TPS.

  Two server costs were found and fixed on the way: every chunk section was encoded once per player (#623),
  and every packet woke its connection's writer thread (#649). Together they took the 50-bot walking tick
  from 281 ms to 31 ms. At 25 walking bots most of what remains is the engine's streaming and block entity
  ticks, both of which grow with players (the README has the profile).
- **Mixed bots, as installed (2026-10-18,** `benchmarks/server-mixed-2026-10-18/`, the first published
  run for the working version). It used the v2026.10.18.15 release as `install.sh` installs it, against
  vanilla 26.3, with 1, 10, 25 and 50 `mixed` bots and vanilla's default game rules on both (Horizon has
  the locator bar now). The servers ran on the ThinkPad and the bots on the Mac mini. Results:
  - Both kept 20 TPS in every run.
  - Horizon's RSS was 3.7 to 5.2 times smaller (1.0 GB against 4.0 GB at 50).
  - Horizon's average tick was shorter with 1, 10 and 25 bots (2.4 against 17.9 ms, 12.4 against 34.5,
    24.0 against 31.6) and longer with 50 (35.2 against 29.7). Its P99 was lower at every count.
  - Horizon also did more work: it kept up with the exploring bots, so it generated 1.9 to 2.9 times
    the new terrain a second.
- **Mixed bots, equal work (2026-10-18,** `benchmarks/server-equal-mixed-2026-10-18/`). This is the same
  run with the load made equal. The explore bots fly at 0.8 blocks a second, so every bot follows the same
  path on both servers and none waits for terrain. A 300-second warm-up lets both finish the bots' first
  views before the measurement. `server_bench.py` now prints a work table that compares each count's chunks
  a second across servers and flags a run that isn't equal. Horizon ran as `horizon-engine`, the server
  build, installed from its archive. Results, on the ThinkPad again (the desktop didn't wake):
  - The work was equal at 1 and 10 bots. At 25 the chunks sent a second were equal (158.6 against 157.6),
    and vanilla still sent 2.5 more first-time chunks a second while it caught up.
  - Tick: 1.8 against 11.3 ms at 1 bot, 4.7 against 34.3 at 10, 11.7 against 50.5 at 25 (vanilla 19.8 TPS).
    At 50 Horizon ran at 23.8 ms. Vanilla's watchdog stopped its server 30 s after the last bot joined (a
    60-second tick). This is one pass, to repeat on a bigger machine.
  - Horizon's RSS was 4.1 to 4.9 times smaller (1.03 GB at 50).
  - Served in full, vanilla's walking bots got 5400 to 6100 chunks each, against 640 to 1100 in the first
    run. That is why vanilla held 50 bots there and not here.
- **Four servers, equal work (2026-10-19,** `benchmarks/server-four-way-2026-10-19/`). The same load
  against vanilla, Paper (build 205, its defaults except the connection throttle, which refused bots that
  share an address) and vanilla on Fabric with Lithium. Horizon was release v2026.10.19, which spawns mobs
  and ticks blocks around every player as vanilla does (#950, #931), so it now does vanilla's work away
  from spawn too. Results, on the ThinkPad (the desktop didn't wake):
  - The work was equal on all four at every count (chunks a second within 4 %).
  - Horizon's RSS was 3.8 to 4.8 times smaller (0.72 to 1.11 GB). It started in 8.2 to 8.5 s, against
    14.7 to 22.4 s for the Java servers. From 10 bots up it used the least CPU.
  - Average tick: Paper was ahead of Horizon at every count (10.3 against 13.7 ms at 1 bot, 28.3 against
    30.9 at 25, 46.9 against 47.8 at 50). Fabric + Lithium was ahead at 1 and 10 bots (12.5 and 20.2
    against 13.7 and 22.9), behind at 25, and fell to 17.2 TPS at 50. Vanilla was slowest, and its
    watchdog stopped it at 50.
  - Horizon's P99 was the lowest from 10 bots up (73 ms at 50, against Paper's 110 and Fabric's 174), and
    its worst single P99 the lowest at every count.
- **Ownership.** perf-server-1 owns making the tick faster. This work only measures it, files what it finds,
  and fixes startup, RAM and packaging.

## 10. Working version for 2026-10-23

Goal: `horizon-server` installs with one command, starts from a clean config, serves Horizon and vanilla 26.3
clients, backs up and restores, has a CLI and a basic web panel, and has a first published benchmark against
vanilla.

**In:**

1. `horizon-server` crate and binary, `horizon-server-files` (the vanilla files, moved out of the client
   unchanged), `server.toml` with import, the CLI (`init`, `run`, `start`, `stop`, `status`, `cmd`, `op`,
   `whitelist`, `backup`, `backups`, `restore`, `logs`), crash restart, and the engine status endpoint.
2. `install.sh` and `install.ps1`, the Dockerfile, systemd and launchd units, `service install`, and the server
   archive in the release.
3. The web panel MVP: status, live TPS/MSPT/RAM/CPU, players, console, backups and login links.
4. `horizon-bots` and `tools/server_bench.py`, with a first run against vanilla (1/10/25/50 bots, `mixed`)
   published in `benchmarks/`: done, `benchmarks/server-mixed-2026-10-18/`, and with equal work
   `benchmarks/server-equal-mixed-2026-10-18/` (section 9).
5. Small engine fixes found on the way: logging refused logins (vanilla's lines).

**Where it stands (2026-10-16):** items 1 to 4 are built and in review (section 11). What's left for the 23rd:
run the Dockerfile and `install.ps1` once on a machine that has Docker and on Windows; publish a release
that holds `horizon-server`, so the installers have something to install; a clean benchmark run on a quiet
machine, then published in `benchmarks/`; and item 5.

**Out (next):**

- the Nether and End for remote players;
- the headless engine and its RAM win;
- a server-only prepare without a JDK;
- a Windows service, hot reload, modpacks in the panel;

Online mode works, so a public server can check accounts. An offline server should still run only on a
LAN, a tailnet, or behind a whitelist.

## 11. Pull requests

In order, each one small and shippable alone:

1. **This design** (docs only).
2. **`horizon-server`**, stacked in three parts:
   - a. The crate, the vanilla files moved out of the client unchanged (`horizon-server-files`),
     `server.toml` with import, `init`, `run` (crash restart, clean stop on Ctrl-C and SIGTERM), and `op`
     and `whitelist` for a stopped server: #551.
   - b. The control API on 127.0.0.1 with its token, `start`, `stop`, `restart`, `status`, `cmd` and
     `logs`, plus the engine status endpoint. That endpoint is about 30 additive lines in
     `game/dedicated.rs` plus a new `game/dedicated/status.rs`. perf-ticks-2's branches touch
     `dedicated.rs` elsewhere (`listen: true`), so the edits don't overlap. It also makes the dedicated loop
     record its tick times, so `/tick query` reports them: #552.
   - c. Backups, restore and automatic backups: #553.
3. **Install scripts, Docker and services** (`service install`), and `horizon-server` in the releases: #554.
4. **Web panel MVP:** #555.
5. **Bots and the first benchmark against vanilla:** #556.

Each PR is stacked on the one before. Merging them in order keeps every step shippable alone.
