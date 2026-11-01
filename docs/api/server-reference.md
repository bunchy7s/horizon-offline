<!-- Generated from crates/horizon-server/src (the help text, config::TEMPLATE, api::ROUTES) by reference.rs; a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test -p horizon-server reference -->
# Server reference

Everything `horizon-server` offers a host or a tool: its commands, every `server.toml` key and the control API. How to host: [hosting.md](../hosting.md); the design: [server.md](../server.md); stability and versions: [stability.md](stability.md).

## Commands

`horizon-server help` prints:

```text
horizon-server: host a Horizon (Minecraft Java Edition 26.3) server

Usage: horizon-server [--dir DIR] <command>

  init        Set up a server here: the EULA, the world, the port, who may join (asks, or takes flags:
              --eula --world NAME --seed SEED --port N --whitelist/--no-whitelist --player NAME... --op NAME
              --offline --yes)
  run         Run the server in the foreground; type console commands, Ctrl-C stops it cleanly. It
              restarts the game if it crashes.
  start       Run the server in the background (its output goes to logs/supervisor.log)
  stop, restart
  status [--json]
              Up or not, TPS, MSPT, players, RAM and CPU
  cmd COMMAND...
              Run a console command on the running server and print what it answers
  report [SECONDS]
              Record a performance report for SECONDS (default 60): where the ticks' time goes,
              entities, chunks, network, memory; saved in reports/ (JSON and text, no player data)
  logs [-f] [-n N]
              The last N lines of the log (default 50); -f follows it
  console     Attach to the running server: its log as it happens, TPS, MSPT, players and RAM at the
              top, and a command line for console commands; Ctrl-D detaches
  mcp         An MCP server on stdin and stdout, for an AI agent to run and moderate the server
              (players, kick, ban, whitelist, logs, events, backups, settings, restart...)
  backup [NAME]
              Zip the world and the settings into backups/ (a running server saves first and stays up)
  backups [--json]
              List the backups
  restore BACKUP [--yes]
              Put a backup's world in place (the current one is kept in backups/); a running
              server restarts with it
  service install|uninstall|print [--system]
              Start the server at boot or login: a systemd unit on Linux (--system: for every user,
              with sudo), a launchd agent on macOS, a task at logon on Windows
  panel       A one-time login link for the web panel ([panel] in server.toml; 127.0.0.1:8130 by
              default)
  update [--check [--json]] [--version TAG] [--timeout SECONDS]
              Install the newest release (checked against GitHub's SHA-256), back up the world and
              restart on it; goes back by itself if the game isn't up within the timeout (300 s).
              --archive FILE --version NAME installs an archive you have
  rollback    Back to the release used before the update
  config [--json]
              The settings in server.toml
  config set KEY=VALUE... [--json]
              Change settings (for example world.difficulty=hard server.max_players=10), checked first;
              a running server takes the difficulty, default game mode and whitelist at once, the rest
              at its next start
  import FILE Write server.toml from a vanilla server.properties
  op NAME, deop NAME
  whitelist on|off|list, whitelist add|remove NAME
              Who may join (on the running server, or in the files when it is stopped)
  players [--json]
              Who is online, the operators, the whitelist and the bans
  kick NAME [REASON], ban NAME [REASON], pardon NAME, ban-ip ADDRESS|NAME [REASON], pardon-ip ADDRESS
              Moderate the running server (vanilla's commands, as from its console)

  --dir DIR   The server folder (default: the current folder)

Settings live in server.toml; the server writes server.properties from it before every start.
```

## `server.toml`

Horizon server settings (docs/server.md). The server writes them into server.properties before every start: edit this file, not that one. Unknown keys are refused. `horizon-server init` writes this file with these defaults; `horizon-server config set SECTION.KEY=VALUE` changes it.

### `[server]`

| Key | Default | In `server.properties` | What it does |
| --- | --- | --- | --- |
| `motd` | `"A Minecraft Server"` | `motd` | the line under the server's name in the server list |
| `port` | `25565` | `server-port` | the port players connect to |
| `bind` | `""` | `server-ip` | listen on every address, or only on this one (server-ip) |
| `max_players` | `20` | `max-players` | how many players can be online at once |
| `online_mode` | `true` | `online-mode` | check players' Minecraft accounts (false: anyone joins under any name) |
| `whitelist` | `true` | `white-list` | only whitelisted players and operators can join |
| `enforce_whitelist` | `false` | `enforce-whitelist` | kick online players taken off the whitelist |
| `view_distance` | `10` | `view-distance` | chunks sent around each player |
| `simulation_distance` | `10` | `simulation-distance` | chunks around each player where mobs, crops and redstone tick |
| `max_view_distance` | `0` | `horizon-max-view-distance` | past view_distance, saved chunks only, up to this (never generated; 0: off) |
| `admin_feedback_to_ops` | `true` | `horizon-admin-feedback-to-ops` | operators see each other's commands ([name: ...], vanilla); false: only the log |

### `[world]`

| Key | Default | In `server.properties` | What it does |
| --- | --- | --- | --- |
| `name` | `"world"` | `level-name` | the world folder |
| `seed` | `""` | `level-seed` | empty: a random seed on the first start |
| `game_mode` | `"survival"` | `gamemode` | survival, creative, adventure or spectator |
| `difficulty` | `"easy"` | `difficulty` | peaceful, easy, normal or hard |
| `hardcore` | `false` | `hardcore` | one life: a player who dies becomes a spectator |

### `[properties]`

Any other server.properties key, spelled as vanilla spells it, for example: spawn-protection = 16

### `[engine]`

| Key | Default | In `server.properties` | What it does |
| --- | --- | --- | --- |
| `path` | `""` |  | the game program; empty: the one installed beside horizon-server |
| `resources` | `""` |  | the prepared game files; empty: resources/ here, else the usual places |
| `restart_on_crash` | `true` |  | start the game again when it crashes |
| `low_memory` | `false` |  | true: no transparent huge pages, ~60 MB less idle for ~1 % more tick time |

### `[backups]`

| Key | Default | In `server.properties` | What it does |
| --- | --- | --- | --- |
| `every` | `"6h"` |  | automatic backups while the server runs: 30m, 6h, 1d...; "0" turns them off |
| `keep` | `12` |  | automatic backups kept (backups made by hand are never deleted) |

### `[updates]`

| Key | Default | In `server.properties` | What it does |
| --- | --- | --- | --- |
| `keep` | `1` |  | other releases kept by updates, besides the one in use and the one rollback goes to |

### `[panel]`

| Key | Default | In `server.properties` | What it does |
| --- | --- | --- | --- |
| `bind` | `"127.0.0.1:8130"` |  | the web panel: this computer only, or a tailnet address (100.x); "" turns it off |
| `allow_public` | `false` |  | true lets bind be any address; never put the panel on the open internet |

### `[api]`

| Key | Default | In `server.properties` | What it does |
| --- | --- | --- | --- |
| `bind` | `""` |  | the control API: "" a free port on 127.0.0.1; or a fixed one, like 127.0.0.1:8131 or a tailnet address |
| `token` | `""` |  | its token (16 characters or more); "" a new random one at every start |
| `allow_public` | `false` |  | true lets bind be any address; the token is then all that guards the server |

### `[discord]`

Post the chat, joins, leaves and deaths to a Discord channel, drawn like the in-game chat (docs/hosting.md)

| Key | Default | In `server.properties` | What it does |
| --- | --- | --- | --- |
| `webhook` | `""` |  | the channel's webhook URL (a secret: anyone with it can post there); "" off |
| `events` | `["chat", "join", "leave", "death"]` |  | what is posted; "kick" too if you like |
| `enabled` | `true` |  | false pauses it and keeps the webhook |

## Control API (`/api/v1`)

HTTP and JSON on 127.0.0.1. A running server writes its address and a random token to `.horizon/api.json`, readable only by its own user; every request sends `Authorization: Bearer <token>` and a `Host` that is that address. Errors answer `{"error": "..."}` with 400, 401, 403, 404, 405 or 409. The web panel uses the same routes with its session.

| Method | Path | Takes | Answers | What it does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/status` | nothing | `phase`, `version`, `pid`, `uptime_seconds`, `engine_pid`, `engine_uptime_seconds`, `crashes`, `last_crash`, `sample` (TPS, MSPT, players, RAM, CPU), `update` | How the server does |
| `GET` | `/api/v1/metrics` | `?since=T` (seconds since 1970) | `{"samples": [...]}`: one a second, the last 10 minutes | The samples taken after T, for graphs |
| `GET` | `/api/v1/logs` | `?since=N` | `{"next": N, "lines": [...]}` | The log lines after the Nth (the last 2000 are kept) |
| `GET` | `/api/v1/events` | `?since=N` (or `Last-Event-ID`), `?types=join,leave,kick,chat,chat_filtered,death,log,phase` (all by default) | with `Accept: text/event-stream`, server-sent events as they happen (`id`, `event`: the type, `data`: JSON); otherwise `{"next": N, "events": [...]}` | Log lines, the supervisor's phase, and the game's joins, leaves, kicks, chat, messages the chat filter refused (`chat_filtered`, without their text) and deaths (the last 2000 kept) |
| `POST` | `/api/v1/command` | `{"line": "<one command>"}` | the lines the command logged | Run a console command on the running game |
| `POST` | `/api/v1/stop` | nothing | `{"ok": true}` | Stop the game and the supervisor |
| `POST` | `/api/v1/restart` | nothing | `{"ok": true}` | Restart the game |
| `GET` | `/api/v1/backups` | nothing | `{"backups": [{"name", "bytes"}]}` | List the backups |
| `POST` | `/api/v1/backups` | `{"name": "..."}` (optional) | `{"name": "<file>"}` | Back up now (a running game saves first and stays up) |
| `DELETE` | `/api/v1/backups` | `?name=<file>` (the full name `backups` lists) | `{"name": "<file>"}` | Delete a backup |
| `GET` | `/api/v1/backups/download` | `?name=<file>` | the zip | Download a backup |
| `POST` | `/api/v1/restore` | `{"name": "<backup>"}` | `{"name": "<backup>"}` | Restart the game with that backup's world (the current one is kept in backups/) |
| `POST` | `/api/v1/update` | `{"tag": "...", "timeout": seconds}` (both optional) | `{"ok": true}`; `status` says how it went (`update`) | Restart on that installed release, or download the newest first; goes back if the game isn't up within the timeout |
| `GET` | `/api/v1/metrics/prometheus` | nothing | the latest sample as Prometheus text: `horizon_up`, `horizon_tps`, `horizon_mspt{stat}`, `horizon_players_online`, `horizon_memory_rss_bytes`... | Metrics for a monitoring tool to scrape |
| `GET` | `/api/v1/players` | nothing | `online` (name, uuid, address, ping_ms, op_level), `max_players`, `ops`, `whitelist` (`enabled`, `players`), `bans`, `ip_bans` (with reason, source, created, expires) | Who is online, and vanilla's lists |
| `POST` | `/api/v1/players/kick` | `{"name", "reason"}` (reason optional) | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Kick an online player |
| `POST` | `/api/v1/players/ban` | `{"name", "reason"}` (reason optional) | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Ban a player (kicked if online) |
| `POST` | `/api/v1/players/pardon` | `{"name"}` | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Lift a player's ban |
| `POST` | `/api/v1/players/ban-ip` | `{"ip", "reason"}`, or `{"name"}` of an online player | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Ban an address (its players are kicked) |
| `POST` | `/api/v1/players/pardon-ip` | `{"ip"}` | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Lift an address's ban |
| `POST` | `/api/v1/players/op` | `{"name"}` | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Make a player an operator (op-permission-level) |
| `POST` | `/api/v1/players/deop` | `{"name"}` | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Make a player no longer an operator |
| `POST` | `/api/v1/whitelist` | `{"enabled": bool}` | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Turn the whitelist on or off |
| `POST` | `/api/v1/whitelist/add` | `{"name"}` | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Whitelist a player |
| `POST` | `/api/v1/whitelist/remove` | `{"name"}` | `{"changed": bool, "output": [...]}`: whether the list (or who is online) changed, and the console's lines | Take a player off the whitelist (kicked if enforce-whitelist) |
| `GET` | `/api/v1/config` | nothing | `{"config": {...}}`: server.toml's settings by section (`api.token` hidden) | The settings |
| `PATCH` | `/api/v1/config` | `{"changes": {"world.difficulty": "hard", ...}}` | `saved`, `applied` (taken by the running game at once), `restart` (from the next start), `running` | Change settings as `config set` does; not `engine.*`, `panel.*`, `api.*`, nor RCON, query or management keys |
| `POST` | `/api/v1/panel/login` | nothing | `{"url": "..."}` | A one-time login link for the web panel (with the token only, not from the panel) |
