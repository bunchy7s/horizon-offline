# Host a server

`horizon-server` hosts a Minecraft Java Edition 26.3 server with Horizon's own game as the engine: one native
program, no Java, one settings file, a command line, and a web panel. Vanilla 26.3 clients and Horizon clients
can join it. This page covers what works on `main` today and says plainly what doesn't yet. The design and the
plan are in [docs/server.md](https://github.com/horizon-rust/horizon/blob/main/docs/server.md).

## Before you start

- **Online mode checks accounts** (release v2026.10.17.07 and newer). With `online_mode = true`, the
  default, every player joins with their own account, as on a vanilla server: vanilla players signed in with
  the official Minecraft launcher, Horizon players with the Microsoft account they signed in with in Horizon's
  launcher ([docs/microsoft-account.md](https://github.com/horizon-rust/horizon/blob/main/docs/microsoft-account.md)).
- **Names you add are accounts** (release v2026.10.17.14 and newer). `init --player`, `horizon-server
  whitelist add` and `op`, and the console's `whitelist add`, `op` and `ban` look a name up the way vanilla
  does: players already seen (kept in `usercache.json`), then Mojang's profile service. In online mode a name
  with no account is refused. So you can whitelist friends before they first join. On an offline server,
  `horizon-server whitelist add` always writes the offline UUID, the one the player joins with, also while
  the server runs (it writes the entry and the game reloads the list): the console's own `whitelist add`
  takes the account's UUID when someone owns that name, as vanilla does, and that player is then refused.
- **An offline server** (`online_mode = false`) never checks accounts: anyone who reaches it can join under
  any name. Keep it on your LAN or a tailnet (Tailscale, ZeroTier), or share it only with friends you trust.
  A Horizon player under an offline name can join only such a server.
- **Secure chat** works as on vanilla's server: in online mode, vanilla clients and Horizon's (Nightly
  2026-11-01 and newer) sign their chat, and with `enforce-secure-profile=true` (the default) unsigned chat is
  refused. On an offline server, or for a player without a chat session, chat stays unsigned and shows as not
  secure.
- **Nether and End:** each player travels on their own through portals, and operators with `execute in
  <dimension> run tp`. The players in the level they leave stop seeing them, and the ones in the level they
  arrive in start to, as on vanilla (checked against vanilla with two players, by command and through an End
  portal: `tools/test_server_dimension_players_parity.py`). The console can't send another player to a
  dimension yet (`execute in ... run tp NAME`).
- **Movement checks and chunk pacing** (release v2026.10.17.16 and newer). The server pulls back players who
  move too fast ("moved too quickly"), as vanilla does, and sends chunks as fast as each client acknowledges
  them.
- **No Docker image is published.** You build it yourself from the repository (below). Publishing an image
  needs the project owner.
- Nothing of Mojang's is downloaded or shipped. The server reads the game files you prepare from your own copy
  of Minecraft 26.3.

## 1. Install

Linux (x86-64) and macOS (Apple Silicon; Intel from the next Nightly):

```sh
curl -fsSL https://raw.githubusercontent.com/horizon-rust/horizon/main/packaging/server/install.sh | sh
```

Windows (x64), in PowerShell:

```powershell
irm https://raw.githubusercontent.com/horizon-rust/horizon/main/packaging/server/install.ps1 | iex
```

The script downloads the current Horizon Nightly for your computer from GitHub (the only build Horizon ships;
`--version` picks another), checks it against the SHA-256 GitHub publishes, and never asks for sudo or administrator rights. It installs:

- the release in `~/.local/share/horizon/releases/<version>` (Windows: `%LOCALAPPDATA%\Horizon`), with
  `current` pointing at the one in use;
- `horizon-server` and `horizon-prepare` in `~/.local/bin` (Windows: added to your PATH);
- a `game-files` folder for the files you prepare, kept across updates.

Options: `--version vX.Y.Z`, `--dir FOLDER`, `--bin FOLDER` (Windows: `-Version`, `-Dir`). Other systems,
such as Linux on ARM, build from source: `cargo build --release -p horizon-client --bin horizon-client -p
horizon-server` and keep both programs in one folder.

The Horizon launcher also installs releases. Every release folder holds `horizon-server`, so you can run it
from there too.

## 2. Prepare the game files

Horizon reads Minecraft's data from your own copy of the game. You need Minecraft Java Edition 26.3, played
once from the official launcher on this computer, a Java 25 runtime, and the 26.3 `server.jar`: it's the
`downloads.server.url` in your launcher's `versions/26.3/26.3.json`. Then:

```sh
horizon-prepare --prepare --server-jar /path/to/server.jar
```

Add `--minecraft-dir PATH` if your launcher keeps the game elsewhere. The prepared files go to
`game-files`, which updates keep.

**On a headless machine** (a VPS, a home server), prepare on your own computer instead, with the launcher or
`horizon-prepare`, and copy the prepared `26.3` folder to the server: into
`~/.local/share/horizon/game-files/horizon-client/26.3`, or into `resources/` in the server folder, or anywhere
`[engine] resources` in `server.toml` names. A folder copied this way needs release v2026.10.17.07 or
newer: older ones don't keep `registries.json` and `commands.json` with the prepared files, and the
server then stops with "registries.json not found next to the resources". Prepare again with a newer release.

## 3. First run

```sh
mkdir my-server && cd my-server
horizon-server init
```

`init` asks five things and writes `server.toml`:

1. whether you agree to the [Minecraft EULA](https://aka.ms/MinecraftEULA): only a yes writes `eula=true`;
2. the world's name and an optional seed (an existing world or vanilla server folder is kept and its
   `server.properties` imported);
3. the port, 25565 by default;
4. whether to use a whitelist (yes by default, as in vanilla) and who to add;
5. whether to make the first player an operator.

It then tells you where it found the game and its files. Every question has a flag, so it also runs from a
script:

```sh
horizon-server init --eula --world survival --seed 12345 --port 25565 --whitelist --player Alex --op Alex --yes
```

`--no-whitelist` turns the whitelist off, and `--offline` turns online mode off (see "Before you start").
The players you add are looked up as Minecraft accounts in online mode, and get offline UUIDs on an offline
server, as vanilla does.

Start the server in the foreground, with the console on your terminal (Ctrl-C stops it cleanly):

```sh
horizon-server run
```

or in the background:

```sh
horizon-server start
horizon-server status
```

```text
Running, up 29 s
TPS 20.0, MSPT 1.0 ms average, 3.1 ms P95
0 of 20 players, 377 chunks loaded
RAM 1.42 GB, CPU 2 % of a core
```

Players join at your computer's address and the port (`192.168.1.20:25565`, or a tailnet name). Every
command takes `--dir FOLDER` to work on a server folder from elsewhere.

## 4. Settings: `server.toml`

You edit one file. The server writes vanilla's `server.properties` from it before every start, so the game
reads exactly what a vanilla server reads, and a vanilla server folder works as it is.

```toml
[server]
motd = "A Minecraft Server"
port = 25565
bind = ""                    # listen on every address, or only on this one (server-ip)
max_players = 20
online_mode = true           # check accounts with Mojang (see "Before you start")
whitelist = true
enforce_whitelist = false
view_distance = 10
simulation_distance = 10
max_view_distance = 0        # past view_distance, saved chunks only, up to this (0: off)
admin_feedback_to_ops = true # operators see each other's commands ([name: ...]); false: only the log

[world]
name = "world"
seed = ""                    # empty: a random seed on the first start
game_mode = "survival"
difficulty = "easy"
hardcore = false

# Any other server.properties key, spelled as vanilla spells it:
[properties]
# spawn-protection = 16
# resource-pack = "https://example.com/pack.zip"   # offered to players as they join; with
# resource-pack-sha1, require-resource-pack and resource-pack-prompt, as on vanilla

[engine]
path = ""                    # the game program; empty: the one beside horizon-server
resources = ""               # the prepared game files; empty: resources/ here, else the usual places
restart_on_crash = true
low_memory = false           # true: less memory, slightly slower ticks (see below)

[backups]
every = "6h"                 # 30m, 6h, 1d...; "0" turns automatic backups off
keep = 12                    # automatic backups kept; backups made by hand are never deleted

[panel]
bind = "127.0.0.1:8130"      # this computer only, or a tailnet address (100.x); "" turns it off
allow_public = false

[api]
bind = ""                    # the control API: a free port on 127.0.0.1, or a fixed one like 127.0.0.1:8131
token = ""                   # 16 characters or more; "" a new random one at every start
allow_public = false
```

The defaults are vanilla 26.3's. Unknown keys and values vanilla would refuse are errors, so a typo stops the
start instead of being ignored. When the game changes a setting itself (`/whitelist on`), the change is copied
back into `server.toml` when it stops, and your comments are kept. Changes to the file apply at the next
restart: there is no live reload yet.

### Operators' commands: `admin_feedback_to_ops`

On by default, as vanilla: when an operator runs a command that reports to admins (`/gamemode`, `/give`, `/tp`...),
the other operators read `[name: ...]` in gray, and the console logs it. Players who aren't operators never see it
either way. `false` keeps it to the console's log (`logs/latest.log`): nobody in game sees an operator's commands,
while each operator still reads their own command's answer.

### Seeing further: `max_view_distance`

Off by default (0), a Horizon setting with no vanilla equivalent. Past `view_distance`, up to
`max_view_distance` (at most 32), each player is also sent the chunks the world has already saved, as far
as their own render distance goes. Those chunks are only read and sent: the server never generates them,
keeps them loaded or ticks them, so what lies out there stands still (no mobs, no growing crops, no
redstone) until a player comes within `view_distance`. A chunk the world hasn't saved yet stays empty on
the player's screen. To fill the ring, generate the world ahead of the players with `/pregen`
(`horizon-pregen = true` under `[properties]`, then `horizon-server cmd pregen start 2500`,
[pregeneration](pregeneration.md)).

The far ring goes after everything else: its chunks wait behind the ones inside `view_distance`, are paced
by how fast each player's game takes them, and aren't read while the world's saves are behind. When the
average tick goes over 40 ms, the ring pulls in by two chunks a second, never inside `view_distance`; after
30 s of ticks under 25 ms, it grows back by one chunk every 5 s. What it sent is kept in memory for 5
minutes (96 MB at most), so a player walking back isn't sent the save again.

Already have a vanilla server? Run `horizon-server init` in its folder, or `horizon-server import
server.properties` to write `server.toml` from it. The world, `ops.json`, `whitelist.json` and the ban lists
keep vanilla's names and formats, so going back to vanilla works too.

## Native server export

A native-mods build exports a standalone engine
(`horizon-launcher native-build FOLDER --server-output DIR --version TAG --trust-native-code`;
see [Native mods](native-mods.md#standalone-server-export)). Point the server at it:

    horizon-server --dir SERVER config set engine.path=/absolute/export/horizon-engine
    horizon-server --dir SERVER restart

Use the absolute engine path printed by the build. Build on the same OS and architecture as the server
(no cross compilation), using a prepared installed launcher release. Updates do not rebuild a pinned `engine.path`: build a new
export folder, set `engine.path` to it, restart, and keep the old folder for manual rollback. The supervisor does
not auto-update or rebuild a pinned engine path, and there is no compatibility handshake yet (follow-up #6304).
Native mods run unsandboxed: only build code you have read and trust.

### Behind a Velocity proxy

A network can put one Velocity proxy in front of several servers: the proxy signs the players in with their
Microsoft accounts, and the servers behind it trust what it forwards. To run Horizon as one of them:

1. Set `online_mode = false` in `server.toml` and keep the server's port reachable only from the proxy (bind it
   to `127.0.0.1` or put it on a private network).
2. Put the proxy's forwarding secret, the one in Velocity's `forwarding.secret`, into a file `forwarding.secret`
   in the server folder, and set `player-info-forwarding-mode = "modern"` in `velocity.toml`.
3. Restart the server. The log says `Velocity player information forwarding is on`.

Players then join with the UUID, name, skin and address the proxy forwards. Anyone who connects without going
through the proxy is refused, and so is a login signed with another secret. With the file in place but
`online_mode = true` the file is ignored (with a warning), and an empty file stops the start, because an empty
secret could be signed by anyone. How the exchange works: [velocity-forwarding.md](velocity-forwarding.md).

## 5. Commands

```text
horizon-server init                      first run
horizon-server run                       in the foreground, with the console; Ctrl-C stops it
horizon-server start | stop | restart    in the background (its output goes to logs/supervisor.log)
horizon-server status [--json]           up or not, TPS, MSPT, players, chunks, RAM, CPU
horizon-server cmd COMMAND...            one console command, and what it answers
horizon-server logs [-f] [-n N]          the log; -f follows it
horizon-server console                   attach: the live log, TPS/players at the top, type commands; Ctrl-D detaches
horizon-server mcp                       an MCP server on stdio, so an AI agent can run and moderate the server
horizon-server report [SECONDS]          record a performance report (below)
horizon-server op|deop NAME
horizon-server whitelist on|off|list
horizon-server whitelist add|remove NAME
horizon-server backup [NAME]             back up now
horizon-server backups [--json]          list the backups
horizon-server restore BACKUP [--yes]
horizon-server service install|uninstall|print [--system]
horizon-server panel                     a one-time login link for the web panel
horizon-server import FILE               server.toml from a vanilla server.properties
horizon-server config [--json]           the settings in server.toml
horizon-server config set KEY=VALUE...   change settings, checked first (below)
horizon-server update [--check]          install the newest release and restart on it (below)
horizon-server rollback                  back to the release before
```

While the server runs, `op`, `whitelist` and `cmd` go to the game's console, so they act exactly as vanilla's
console commands do. While it's stopped, they edit vanilla's files directly. Any console command works through
`cmd`: `horizon-server cmd ban Griefer`, `horizon-server cmd say Restarting in 5 minutes`.
To generate the world around spawn ahead of the players, Horizon has its own `pregen` command, off until
`server.toml` has `horizon-pregen = true` under `[properties]`: `horizon-server cmd pregen start 2500`
([pregeneration](pregeneration.md)).
To let only the Horizon client join, set `horizon-clients-only = true` under `[properties]` (off by default).
Other clients are turned away while they connect with "This server is for the Horizon client:
playhorizon.dev", and the console says which client it was. The check reads the brand a client names itself
by (`minecraft:brand`), which any client can fake: it keeps out players who didn't install Horizon, not
cheaters. The server's own checks (reach, movement) are what protect it.

Horizon clients can get their far terrain (Video Settings → LOD Distance) from the server: set
`horizon-lod-distance` under `[properties]` to how far, in chunks (1024 at most), and the server samples the
tiles each player's client asks for out to there. It is off (0) by default: far tiles show the world's terrain
far past the view distance, which helps players find the seed. A player's own LOD Distance applies when it is
shorter. Each player may have 8 tiles waiting and ask for 64 a second on average, and one background thread
samples them for everyone in turn, behind the ticks, so far terrain never slows the server down; a tile past the
distance or over a player's limits is refused, and a client that keeps breaking the limits is disconnected.
Tiles are sampled again for each player who asks (there is no cache yet). Vanilla clients never hear of it.

`horizon-server report` records the running server for 60 seconds (`report 300` for five minutes, up to 600),
then prints a short summary and saves it in `reports/`: `perf-<time>.txt` and `perf-<time>.json`. They say where
the ticks' time went, phase by phase (mobs, block ticks, block entities, the network and the chunks it
sends, saving...), the slowest ticks, how many entities of each type, chunks, players, memory, CPU, and the
settings that matter (view and simulation distance, max players). They hold no names, UUIDs or addresses, so you
can attach them to an issue as they are. On the console it is `horizon-report [SECONDS]`.

`config set` changes `server.toml` without opening it: `horizon-server config set world.difficulty=hard
server.max_players=10`. Each key is a section and a name from the file (`properties.spawn-protection=0` for any
other vanilla key). The values are checked as the server checks them, and nothing is written if one is wrong.
A running server takes the difficulty, the default game mode, the whitelist and the backup settings at once;
the rest applies at its next start, and `config set` says which. `--json` prints that as `saved`, `applied`
and `restart`.

The server restarts the game by itself if it crashes: after 5 s, then waiting longer each time, up to 5
minutes. It keeps the crash report and gives up after 5 crashes in 10 minutes, and `status` says why.

**Updates** (a server installed with the install script). The server checks for a newer release a minute
after it starts and every 6 hours after, and says so in its log, in `status` and in the panel; it never
updates by itself. `horizon-server update --check` asks once. `horizon-server update`:

1. downloads the current Nightly and checks it against its signed SHA-256;
2. stops the game, backs up the world (`backups/<time>-before-<release>.zip`) and switches to the new
   release;
3. starts the game on it and waits up to 5 minutes (`--timeout SECONDS`) for it to come up.

If the new game crashes or doesn't come up in time, the server goes back to the release it was on by itself
and puts the world back as it was just before the switch (the world the new game left is kept in
`backups/`). `horizon-server rollback` goes back to the release before by hand, keeping the world as it is
(it backs it up first). `--version vX.Y.Z` installs a given release, and `--archive FILE --version NAME` an
archive you have, such as your own build. A server still on a release from before the last dated one doesn't know
that 0.2 comes after the dated releases and says it is up to date: run `horizon-server update --version v0.2.0`
once ([versioning](versioning.md)). A stopped server is switched and starts on the new release next
time. Set `HORIZON_NO_UPDATE_CHECK=1` in the server's environment to turn the check off.

Coming: `console` (attach to a server running in the background), `ban` and `pardon` as their own
commands, and `mcp` (an MCP server so an AI assistant can run the server).

## 6. Web panel

The panel runs inside `horizon-server` while the server runs. It shows live TPS, MSPT, RAM and CPU graphs for
the last 10 minutes, the players with their ping (and a kick button), the console with a command line, and the
backups (back up now, restore with a confirmation), with Restart and Stop. When a newer release is out it says
so, with an Update button that runs `horizon-server update` (section 5), and shows how it went.

```sh
horizon-server panel
```

prints a login link that works once, for 10 minutes. The panel listens on `127.0.0.1:8130`, this computer
only. To reach it from your other devices:

- put your tailnet address in `[panel] bind` (for example `bind = "100.101.102.103:8130"`), or
- forward the port over SSH: `ssh -L 8130:127.0.0.1:8130 your-server`, then open the link on your computer.

Any other address, such as `0.0.0.0` or a public IP, is refused unless you set `allow_public = true`, and
even then the panel warns at every start. It has no passwords to leak: whoever can read the server folder can
make a login link, and no one else can. Never put it on the open internet.

### A bot or a tool of your own

Everything the panel does goes through the control API ([api/server.md](api/server.md)), which a moderation bot
(a Discord `!ban`), a monitoring tool or a script can call too. It listens on a free port of `127.0.0.1` and
writes the address and a fresh token to `.horizon/api.json` at every start. For a program that runs elsewhere
or keeps its settings, fix both in `server.toml`:

```toml
[api]
bind = "127.0.0.1:8131"      # or your tailnet address, 100.x.y.z:8131
token = "a long random string only the bot knows"
```

The same rules as the panel's hold: a tailnet or loopback address, unless `allow_public = true` (the token is then
all that guards the server, and it warns at every start). The API can't change `[engine]`, `[panel]`, `[api]` or
`[discord]`, nor turn on RCON or another remote console.

### Chat in a Discord channel

`[discord]` posts the server's chat, joins, leaves and deaths to a Discord channel. In the channel's settings,
Integrations → Webhooks → New Webhook → Copy Webhook URL, then:

```toml
[discord]
webhook = "https://discord.com/api/webhooks/..."   # a secret: whoever has it can post in the channel
events = ["chat", "join", "leave", "death"]        # and "kick", "announce" (lines native mods post)
enabled = true                                     # false pauses it and keeps the webhook
```

The lines of each 2 seconds are drawn as one picture of the in-game chat (transparent, shadowed letters, colours
included), and a channel gets at most one post every 2 seconds (more when Discord asks for it: a burst beyond 20
lines is counted, not drawn). Only `chat` is forwarded, never a message the server's chat filter held back. Posts
never ping: they allow no mention, and an `@everyone` typed in game reaches Discord defused. Every post comes
from the webhook's own name and avatar (set them in Discord), never a player's; players' addresses are never sent,
and a player is always named by their account's name (a nickname only next to it). The server only makes outgoing HTTPS calls to Discord;
the API and the panel show the webhook as `(set)`. The letters are Pixelify Sans (SIL Open Font License,
`crates/horizon-theme/fonts`), not Minecraft's font.

### Tools written for vanilla's management protocol

Vanilla 26.3 has its own management protocol, JSON-RPC over a WebSocket, and Horizon's server speaks it too, so a
tool written for a vanilla server manages a Horizon one. It's off by default; turn it on in `server.toml`:

```toml
[properties]
management-server-enabled = true
management-server-port = 8132          # 0: a free port, logged at the start
# management-server-secret: 40 letters and digits; left out, the server makes one and writes it here
# management-server-tls-keystore = "management.p12"   # your certificate (PKCS12); left out, see below
# management-server-tls-keystore-password = "..."     # or MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD
```

It listens on `localhost` (`management-server-host`). A client sends the secret as `Authorization: Bearer <secret>`,
or, from a browser, in the WebSocket subprotocol (`minecraft-v1,<secret>`) from an origin listed in
`management-server-allowed-origins`. `rpc.discover` lists the methods; Horizon answers more of them with each release
([api/server.md](api/server.md#not-there-yet)).

TLS is on, as in vanilla (`wss://`). The certificate and its key come from a PKCS12 keystore, as vanilla's do:

- `keytool -genkeypair -storetype PKCS12 ...` makes one, or `openssl pkcs12 -export` turns a PEM certificate and
  key into one.
- Its password comes from the `MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD` variable, else from
  `management-server-tls-keystore-password`.

Where vanilla refuses to start without a keystore, Horizon makes one:

- a self-signed certificate for `localhost`, `127.0.0.1` and `management-server-host`, saved in
  `.horizon/management-keystore.p12`, which only the server's user can read;
- its path and password are written to `[properties]`;
- each start logs the certificate's SHA-256 fingerprint, which a client pins or adds to its trusted certificates.

Keep the protocol on `localhost` or your tailnet even so. `management-server-tls-enabled = false` turns TLS off.
As in vanilla, a wrong secret or keystore stops the server from starting.

### Tools that speak RCON

Hosting panels, Discord bridges and scripts often run commands through RCON, vanilla's remote console, and Horizon's
server serves it. It's off by default and needs a password; turn it on in `server.toml` (the dotted keys are
quoted in TOML):

```toml
[properties]
enable-rcon = true
"rcon.port" = 25575
"rcon.password" = "a long random string"
```

It listens on the address `[server] bind` names (`server-ip`; every address when it's empty), as vanilla's does.

A client logs in with the password and each command runs as one typed in the server's console, with the same
output (`horizon-server cmd` does the same from the shell). RCON isn't encrypted: the password and every command
travel in clear text, and it gives full control of the server. Keep its port closed to the internet: a firewall, a
tailnet or an SSH tunnel. As in vanilla, an empty password or a port outside 1 to 65535 leaves RCON off with a
warning in the log, and a port already in use is a warning too, not a server that doesn't start. The settings are
read when the server starts. To turn it off again, set `enable-rcon = false`: taking the lines out of
`server.toml` leaves what `server.properties` already holds, as for any other key.

Where it differs from vanilla: a connection that hasn't logged in within 30 seconds is closed, at most 16 are open
at once, and a command a client's TCP splits in two still works.

A command runs from a source named `Rcon`: operators in game see what it did as `[Rcon: ...]` (gray, italic) unless
`broadcast-rcon-to-ops` is `false`, and a `say` shows as `[Rcon] ...`.

### Tools that read the query

Server lists, status pages and monitoring tools (`mcstatus`, Prometheus exporters, hosting panels) often read the
MOTD, the version and who is online through vanilla's GS4 query, a UDP port, and Horizon's server serves it. It's off
by default; turn it on in `server.toml`:

```toml
[properties]
enable-query = true
"query.port" = 25565
```

It listens on UDP on the address `[server] bind` names (every address when it's empty), as vanilla's does: the same
number as the game's TCP port is fine, they are different ports. The query tells anyone who can reach the port the
player names and the version, and has no password: open the port only if you want that. As in vanilla, a port
outside 1 to 65535 leaves it off with a warning in the log, and a port already in use is a warning too. The
settings are read when the server starts. To turn it off again, set `enable-query = false`.

Where it differs from vanilla: while the game isn't running (starting, restarting) nothing answers, and who is
online is read from the game at most once a second.

## 7. Backups

```sh
horizon-server backup before-the-update
horizon-server backups
horizon-server restore before-the-update
```

A backup zips the world, vanilla's files and `server.toml` into `backups/`. A running server saves first and
stays up: players don't notice. Automatic backups run on the `[backups] every` schedule while the server runs,
and the oldest automatic ones beyond `keep` are deleted. Backups you make by hand are never deleted.

`restore` takes a backup's file name, a unique start of it, or the name you gave it. It moves the current
world aside to `backups/replaced-<time>/` (never deleted), puts the backup in place, and restarts the server if
it was running. The panel does the same with one click.

## 8. Start it with the computer

```sh
horizon-server service install
```

- **Linux:** a systemd user unit (`--system` for a system unit, with sudo). To keep a user unit running while
  you're logged out: `loginctl enable-linger $USER`.
- **macOS:** a launchd agent in `~/Library/LaunchAgents`.
- **Windows:** a task that starts at logon (a real Windows service comes later).

The service runs `horizon-server run` through the installed `current` release, so an update applies at the
next restart. `service print` shows what it would install, `service uninstall` removes it and leaves the server
folder alone.

## 9. Docker (build it yourself)

No image is published: build one from the repository. The image holds none of Minecraft's files; you mount the
folder you prepared (step 2) read-only.

```sh
git clone https://github.com/horizon-rust/horizon && cd horizon
docker build -f packaging/server/Dockerfile -t horizon-server .

docker run -d --name horizon -p 25565:25565 --stop-timeout 180 \
  -v horizon-data:/data \
  -v /path/to/26.3:/opt/horizon/.local/horizon-client/26.3:ro \
  -e HORIZON_EULA=true -e HORIZON_WHITELIST=true -e HORIZON_PLAYERS=Alex,Steve -e HORIZON_OP=Alex \
  horizon-server
```

- `HORIZON_EULA=true` accepts the [Minecraft EULA](https://aka.ms/MinecraftEULA) for this server. Without it
  the container stops and says why.
- The first start writes `/data/server.toml` from `HORIZON_EULA`, `HORIZON_WORLD`, `HORIZON_SEED`,
  `HORIZON_PORT`, `HORIZON_WHITELIST`, `HORIZON_PLAYERS`, `HORIZON_OP` and `HORIZON_ONLINE`. Later starts use
  the file.
- The mounted folder must come from release v2026.10.17.07 or newer (see step 2).
- Manage it with `docker exec horizon horizon-server status` (and `cmd`, `backup`, `logs`, ...).
  `docker stop` saves and stops cleanly; `--stop-timeout 180` gives it the time.
- The panel stays on 127.0.0.1 inside the container.
- The image still carries ALSA and udev libraries because the game links them, even though a server never plays
  a sound. A smaller engine without them is planned.

## 10. How fast is it

The latest bot benchmark compares Horizon with vanilla 26.3, Paper and Fabric + Lithium:
[benchmarks/server-four-way-2026-10-19](https://github.com/horizon-rust/horizon/blob/main/benchmarks/server-four-way-2026-10-19/README.md).
It ran release v2026.10.19 on a 4-thread laptop CPU (i7-7500U), with 1 to 50 players walking, exploring and
standing around spawn, the bots on another machine. All four servers did the same work. Earlier
measurements are in [docs/server.md](https://github.com/horizon-rust/horizon/blob/main/docs/server.md). In
short:

- **Memory:** 0.7 GB with one player and 1.1 GB with 50, against 3.4 to 4.5 GB for the Java servers
  (each with a 4 GB heap): 3.8 to 4.8 times less.
- **Start:** ready in 8 to 9 s, against 15 s for vanilla, 17 s for Fabric and 19 to 22 s for Paper.
- **CPU:** from 10 players up, the least of the four (1.4 cores at 50, against Paper's 2.1 and Fabric's 2.4).
- **Average tick:** Paper's is shorter at every count (10.3 against 13.7 ms with one player, 46.9 against
  47.8 with 50), and Fabric + Lithium's with 1 and 10 players. Horizon's is shorter than vanilla's (equal
  with 10), and than Fabric's from 25 players up.
- **Slow ticks:** from 10 players up, Horizon's slowest ticks (99th percentile) are the shortest of the
  four (73 ms with 50, against 110 for Paper and 174 for Fabric).
- **50 players:** Horizon and Paper keep 20 TPS, Fabric + Lithium drops to 17, and vanilla's watchdog
  stopped it.

For more than 50 active players, give it more cores.

### Less memory: `low_memory`

Off by default. On Linux, the game's allocator (mimalloc) backs its memory with transparent huge pages
(2 MB each) where the system allows them. Many of those pages are only partly used, so the server holds
more memory than it needs, but the tick runs a little faster. Under `[engine]`, `low_memory = true` starts
the game with huge pages off. Measured on a 4-core EPYC VPS with 20 bots playing (2026-11-01):

| | Default | `low_memory = true` |
|---|---|---|
| Memory, idle after start | ~300 MB | ~245 MB (−60 MB) |
| Memory, 20 players | ~1180 MB | ~980 MB (−190 MB) |
| Average tick, 20 players | 27.1 ms | 27.5 ms (+1.2 %) |
| Start | 2.1 s | 2.2 s |

Turn it on when memory is what you're short of, for example on a small VPS. It applies at the next start.
A game started without `horizon-server` (`horizon-client --dedicated`, a container of your own) gets the same
effect from the environment variable `MIMALLOC_ALLOW_THP=0`. On macOS and Windows it changes nothing.

## When something goes wrong

- **"no prepared game files"**: `init` and `run` list the folders they looked in. Prepare (step 2), or set
  `[engine] resources`.
- **"registries.json not found next to the resources"**: the prepared folder is from an older release. Prepare
  again with a newer one.
- **Nobody can join**: the whitelist is on by default (`horizon-server whitelist add NAME`), and the port must be
  open in your firewall. On a tailnet, use the server's tailnet address.
- **It keeps restarting**: `horizon-server status` shows the last crash and `horizon-server logs -n 200` the log
  before it. Crash reports are in `crash-reports/`.
- Report problems at <https://github.com/horizon-rust/horizon/issues>, with the end of the log.
