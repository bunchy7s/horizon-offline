# Server API

What a host, a panel or a bot can do with a Horizon server: the `horizon-server` commands, `server.toml` and the
control API. How to host: [hosting.md](../hosting.md); the design: [server.md](../server.md).

## Overview

`horizon-server` sets up a server folder, runs the game as a dedicated server under a supervisor (restarts on a crash,
backups on a schedule, updates with a rollback), and lets other programs manage it through a local HTTP API with a
token. The web panel uses that API, and so can a moderation bot: players, kick, ban and pardon (by name or
address), op and deop, the whitelist, and metrics as Prometheus text.

| | Count |
| --- | --- |
| Commands | 30 (`init`, `run`, `start`, `stop`, `restart`, `status`, `cmd`, `logs`, `console`, `backup`, `backups`, `restore`, `service`, `panel`, `update`, `rollback`, `config`, `config set`, `import`, `mcp`, `op`, `deop`, `whitelist`, `players`, `kick`, `ban`, `pardon`, `ban-ip`, `pardon-ip`, `version`) |
| `server.toml` keys | 25 in 7 sections, plus `[properties]` for any other vanilla key |
| Control API endpoints | 28 under `/api/v1` |

## Guide

1. `horizon-server init` in an empty folder, then `horizon-server start` ([hosting.md](../hosting.md)).
2. From a script: `horizon-server status --json`, `horizon-server cmd say hi`, `horizon-server backup`. By hand:
   `horizon-server console` attaches to the running server (live log, status header, command line).
   From an AI agent: `horizon-server --dir /srv/minecraft mcp` is an MCP server on stdio with a tool per endpoint
   (`players`, `ban`, `events`, `set_config`...); add it to the agent's MCP servers, run as the server's user.
3. From a program: read `.horizon/api.json` (address and token), then `GET /api/v1/status` with
   `Authorization: Bearer <token>`. A program that runs elsewhere or keeps its settings (a Discord bot) uses a
   fixed address and token: `[api] bind` and `[api] token` in `server.toml` ([hosting.md](../hosting.md#a-bot-or-a-tool-of-your-own)).
4. Moderate: `horizon-server players`, `horizon-server ban NAME [REASON]`, or `POST /api/v1/players/ban`. An
   action runs vanilla's console command on the running game, so it kicks, logs and saves `banned-players.json`
   exactly as an operator's `/ban` does; the reply says whether anything `changed` and has the console's lines.
5. Monitor: point Prometheus (or any scraper that sends a bearer token) at `GET /api/v1/metrics/prometheus`.
6. Watch: `GET /api/v1/events` with `Accept: text/event-stream` streams what happens as server-sent events: every
   log line (`log`), the supervisor's `phase`, and the game's typed `join`, `leave`, `kick`, `chat` and `death`
   events with the player's name and UUID (`?types=chat,death` picks some). `Last-Event-ID` (or `?since=N`)
   resumes after a reconnection; without `Accept: text/event-stream` it answers the events after `since` as JSON.
7. A tool written for vanilla: turn on vanilla's management protocol (`management-server-enabled = true` in
   `[properties]`, [hosting.md](../hosting.md#tools-written-for-vanillas-management-protocol)); it answers as vanilla
   26.3's does, and `rpc.discover` describes it.
8. A tool that speaks RCON (a hosting panel, a Discord bridge): turn on `enable-rcon` with a password in
   `[properties]` ([hosting.md](../hosting.md#tools-that-speak-rcon)); a command runs as one typed in the console.

## Reference

[server-reference.md](server-reference.md), generated from `horizon-server`'s source: the help text, every
`server.toml` key with its default and the `server.properties` key it writes, and every endpoint.

## Examples

```sh
token=$(jq -r .token .horizon/api.json); address=$(jq -r .address .horizon/api.json)
curl -s -H "Authorization: Bearer $token" "http://$address/api/v1/status"
curl -s -H "Authorization: Bearer $token" -d '{"line": "list"}' "http://$address/api/v1/command"
curl -s -H "Authorization: Bearer $token" "http://$address/api/v1/players"
curl -s -H "Authorization: Bearer $token" -d '{"name": "Steve", "reason": "griefing"}' "http://$address/api/v1/players/ban"
# {"changed":true,"output":["Banned Steve: griefing"]}
curl -s -H "Authorization: Bearer $token" "http://$address/api/v1/metrics/prometheus"
curl -sN -H "Authorization: Bearer $token" -H "Accept: text/event-stream" "http://$address/api/v1/events?types=join,chat,death"
# id: 41
# event: chat
# data: {"id":41,"type":"chat","name":"Steve","uuid":"...","message":"hi","time":1793451234567}
curl -s -H "Authorization: Bearer $token" -X PATCH -d '{"changes": {"world.difficulty": "hard"}}' "http://$address/api/v1/config"
# {"applied":["world.difficulty"],"restart":[],"running":true,"saved":["world.difficulty"]}
curl -s -H "Authorization: Bearer $token" -o world.zip "http://$address/api/v1/backups/download?name=2026-10-31T12-00-00Z-manual.zip"
```

## Stability

Stable ([stability.md](stability.md)): the commands, `server.toml`'s keys and `/api/v1`. A new key or endpoint is an
addition; a change to one starts `/api/v2`, served beside `/api/v1`.

## Not there yet

- **Players**: positions in `players`; editing the lists while the game is stopped through the API (the CLI's
  `op`, `deop` and `whitelist` already do).
- **Metrics**: entities and per-dimension counts in the Prometheus text.
- **Backups**: restore a single dimension.
- **Mods on a dedicated server**: list, turn on and off, and their settings, once native mods can be built for the
  server ([mods.md](../mods.md), [native-mods.md](../native-mods.md#multiplayer)).
- **Vanilla's management protocol**: `serversettings/*`, `gamerules`, the notifications and the status
  heartbeat. They answer an internal error saying so; a kick's translatable message is sent as its key.
- **Compatibility tests**: golden replies per endpoint, and a `server.toml` from each release line.

## Changelog

- `api::ROUTES` lists the endpoints; the reference is generated from it.
- `/api/v1`: status, metrics, logs, command, stop, restart, backups, restore, update, panel login.
- Commands: `players`, `kick`, `ban`, `pardon`, `ban-ip`, `pardon-ip`.
- `/api/v1` additions: `players`, `players/{kick,ban,pardon,ban-ip,pardon-ip,op,deop}`, `whitelist`,
  `whitelist/{add,remove}`, `metrics/prometheus`, `GET`/`PATCH config`, `DELETE backups`, `GET backups/download`;
  `server.toml` gets `[api]` (`bind`, `token`, `allow_public`); `GET events` (server-sent events, or JSON).
- RCON (`enable-rcon`, `rcon.port`, `rcon.password`): vanilla's remote console, served by the supervisor.
- Vanilla's management protocol (`management-server-*`): the WebSocket, its authentication, `rpc.discover`,
  `players`, `players/kick`, `server/{status,save,stop,system_message}`, and the four lists' getters.
- The management protocol's TLS (on by default, as in vanilla): a PKCS12 keystore, or a self-signed one generated
  in `.horizon/` when none is set.
- The management protocol's `allowlist/{add,remove,set,clear}` and `bans/{add,remove,set,clear}`.
- The management protocol's `ip_bans/{add,remove,set,clear}` and `operators/{add,remove,set,clear}`.
