# Host a server

`horizon-server` hosts a Minecraft Java Edition 26.3 server with Horizon's own game as the engine: one native
program, no Java, one settings file, a command line, and a web panel. Vanilla 26.3 clients and Horizon clients
can join it. This page covers what works on `main` today and says plainly what doesn't yet. The design and the
plan are in [docs/server.md](https://github.com/horizon-rust/horizon/blob/main/docs/server.md).

## Before you start

- **Online mode checks accounts** (release v2026.10.17.07 and newer). With `online_mode = true`, the
  default, players who sign in with the official Minecraft launcher join with their own accounts, as on a
  vanilla server.
- **Names you add are accounts** (release v2026.10.17.14 and newer). `init --player`, `horizon-server
  whitelist add` and `op`, and the console's `whitelist add`, `op` and `ban` look a name up the way vanilla
  does: players already seen (kept in `usercache.json`), then Mojang's profile service. In online mode a name
  with no account is refused. So you can whitelist friends before they first join. On an offline server,
  `horizon-server whitelist add` always writes the offline UUID, the one the player joins with, also while
  the server runs (it writes the entry and the game reloads the list): the console's own `whitelist add`
  takes the account's UUID when someone owns that name, as vanilla does, and that player is then refused.
- **Horizon's own client can't sign in to a Microsoft account yet.** That needs an Azure app registration and
  Mojang's approval, which the project doesn't have
  ([docs/microsoft-account.md](https://github.com/horizon-rust/horizon/blob/main/docs/microsoft-account.md)).
  Until then Horizon players can join offline servers only (`online_mode = false`). An offline server never
  checks accounts: anyone who reaches it can join under any name. Keep it on your LAN or a tailnet
  (Tailscale, ZeroTier), or share it only with friends you trust.
- **Secure chat** works as on vanilla's server: in online mode, vanilla clients sign their chat, and with
  `enforce-secure-profile=true` (the default) unsigned chat is refused. On an offline server, or for a player
  without a chat session (Horizon's own client today), chat stays unsigned and shows as not secure.
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

Linux (x86-64) and macOS (Apple Silicon):

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

[backups]
every = "6h"                 # 30m, 6h, 1d...; "0" turns automatic backups off
keep = 12                    # automatic backups kept; backups made by hand are never deleted

[panel]
bind = "127.0.0.1:8130"      # this computer only, or a tailnet address (100.x); "" turns it off
allow_public = false
```

The defaults are vanilla 26.3's. Unknown keys and values vanilla would refuse are errors, so a typo stops the
start instead of being ignored. When the game changes a setting itself (`/whitelist on`), the change is copied
back into `server.toml` when it stops, and your comments are kept. Changes to the file apply at the next
restart: there is no live reload yet.

Already have a vanilla server? Run `horizon-server init` in its folder, or `horizon-server import
server.properties` to write `server.toml` from it. The world, `ops.json`, `whitelist.json` and the ban lists
keep vanilla's names and formats, so going back to vanilla works too.

## 5. Commands

```text
horizon-server init                      first run
horizon-server run                       in the foreground, with the console; Ctrl-C stops it
horizon-server start | stop | restart    in the background (its output goes to logs/supervisor.log)
horizon-server status [--json]           up or not, TPS, MSPT, players, chunks, RAM, CPU
horizon-server cmd COMMAND...            one console command, and what it answers
horizon-server logs [-f] [-n N]          the log; -f follows it
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
