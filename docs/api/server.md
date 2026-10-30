# Server API

What a host, a panel or a bot can do with a Horizon server: the `horizon-server` commands, `server.toml` and the
control API. How to host: [hosting.md](../hosting.md); the design: [server.md](../server.md).

## Overview

`horizon-server` sets up a server folder, runs the game as a dedicated server under a supervisor (restarts on a crash,
backups on a schedule, updates with a rollback), and lets other programs manage it through a local HTTP API with a
token. The web panel uses that API.

| | Count |
| --- | --- |
| Commands | 22 (`init`, `run`, `start`, `stop`, `restart`, `status`, `cmd`, `logs`, `backup`, `backups`, `restore`, `service`, `panel`, `update`, `rollback`, `config`, `config set`, `import`, `op`, `deop`, `whitelist`, `version`) |
| `server.toml` keys | 22 in 6 sections, plus `[properties]` for any other vanilla key |
| Control API endpoints | 11 under `/api/v1` |

## Guide

1. `horizon-server init` in an empty folder, then `horizon-server start` ([hosting.md](../hosting.md)).
2. From a script: `horizon-server status --json`, `horizon-server cmd say hi`, `horizon-server backup`.
3. From a program: read `.horizon/api.json` (address and token), then `GET /api/v1/status` with
   `Authorization: Bearer <token>`.

## Reference

[server-reference.md](server-reference.md), generated from `horizon-server`'s source: the help text, every
`server.toml` key with its default and the `server.properties` key it writes, and every endpoint.

## Examples

```sh
token=$(jq -r .token .horizon/api.json); address=$(jq -r .address .horizon/api.json)
curl -s -H "Authorization: Bearer $token" "http://$address/api/v1/status"
curl -s -H "Authorization: Bearer $token" -d '{"line": "list"}' "http://$address/api/v1/command"
```

## Stability

Stable ([stability.md](stability.md)): the commands, `server.toml`'s keys and `/api/v1`. A new key or endpoint is an
addition; a change to one starts `/api/v2`, served beside `/api/v1`.

## Not there yet

- **Players**: list with ping and position, kick, ban and pardon, op and deop, the whitelist, as endpoints (today only
  through `cmd` and the files).
- **Console stream**: the log as a stream (server-sent events) rather than `logs?since=N` polling; chat, joins, leaves,
  deaths and crashes as typed events.
- **Metrics**: a plain-text metrics endpoint any monitoring tool can scrape (TPS, MSPT percentiles, players, chunks,
  entities, RAM, CPU), from the samples `status` already takes.
- **Backups**: delete and download one; restore a single dimension.
- **Mods on a dedicated server**: list, turn on and off, and their settings, once native mods can be built for the
  server ([mods.md](../mods.md), [native-mods.md](../native-mods.md#multiplayer)).
- **An MCP server** for the same endpoints, so a host's agent manages the server as it manages the game.
- **Compatibility tests**: golden replies per endpoint, and a `server.toml` from each release line.

## Changelog

- `api::ROUTES` lists the endpoints; the reference is generated from it.
- `/api/v1`: status, metrics, logs, command, stop, restart, backups, restore, update, panel login.
