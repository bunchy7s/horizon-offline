# Launch API

What another launcher (Prism Launcher, MultiMC, ATLauncher, Modrinth App, HMCL or your own) needs to start Horizon:
the options `horizon-client` takes, how it hands over an account, and how it ends. `horizon-launcher` is the
reference implementation. Proposed in horizon-rust/horizon#3545.

## Overview

| | |
| --- | --- |
| Version | 0.1.0, printed by `horizon-client --launch-api` |
| Level | experimental ([stability.md](stability.md)): preview once preparing and the guide are covered too |
| Covers | starting `horizon-client`: its options, the session JSON, the exit codes, the crash report's path; preparing with `horizon-prepare`: `--output`, `--check`, `--complete`, `--progress json` |
| Not yet | finding a build, asking the preparer what it supports (see Not there yet) |

## Guide

1. Start Horizon only for a player signed in with a Microsoft account that owns Minecraft: Java Edition, and
   hand over that account's session (`--session-stdin`, or `--session-file`), never on the command line. This
   is the same owner gate `horizon-launcher` applies before every start. Horizon never downloads Mojang's
   files; a launcher that does downloads them only for that owner.
2. Ask the build what it takes: `horizon-client --launch-api` prints JSON with `launch_api` (the version) and
   `options`. Pass only options it lists: any other is refused with exit code 2.
3. Start it with a folder per instance:

   ```sh
   horizon-client --resources PREPARED --game-dir INSTANCE --home INSTANCE --width 1280 --height 720 --session-stdin
   ```

4. Watch the exit code. On 2, stderr names the refused option (`Horizon: --width: …`). On 1 or 101, the last
   line on stderr names the crash report.

## Reference

[launch-reference.md](launch-reference.md), generated from `crates/horizon-client/src/game/launch_api.rs`: every
option, the `--launch-api` reply and the exit codes.

### The session JSON

The account the game joins online-mode servers with ([microsoft-account.md](../microsoft-account.md)):

```json
{"name": "Alex", "uuid": "069a79f444e94726a5befca90e38aaf5", "access_token": "…", "user_type": "msa", "xuid": "…"}
```

- `name`, `uuid` (with or without dashes) and `access_token` are required; `user_type`, when present, is `msa`;
  `xuid` is optional.
- `--session-stdin`: write the JSON to the game's standard input and close it.
- `--session-file PATH`: a file the game reads once and deletes at once. On Unix its mode must be 0600; a file
  older than two minutes is refused (and deleted).
- A session the game can't read is reported on stderr (`Horizon: not signed in: …`), and online-mode servers
  will refuse the player.

## Preparing

`horizon-prepare` turns the player's own Minecraft files into the resources `horizon-client --resources` reads. It
downloads nothing: it reads the files of a Minecraft the player owns (Guide, step 1) and checks each against the
pins in `research-lock.json`.

```sh
horizon-prepare --prepare --output PREPARED --complete --progress json
horizon-prepare --check --output PREPARED
```

- `--output DIR`: where the resources go; the preparer's scratch files stay in its own `.local/`. One
  preparation at a time writes to a folder; a second one waits.
- `--check`: exit 0 when the resources are complete and current for this preparer. Otherwise exit 1, with each
  gap on stdout and the command that fills it, or an `error:` line on stderr when it can't check. It tests which
  files are there and the versions they were made for, not their contents: a launcher that suspects damage
  prepares again. A folder being prepared, or whose preparation stopped, is not current.
  `--check` refuses `--progress` and `--complete` (exit 2): they apply to preparing.
- `--complete`: after preparing, copies the sounds and fails unless `--check` would pass.
- `--version-json`, `--client-jar`, `--libraries-dir` (Maven layout), `--assets-dir` (`indexes/` and `objects/`):
  for a launcher whose files aren't laid out like the official launcher's, each names what `--minecraft-dir`
  would give. Every file is still checked against the pins. When an earlier import beside the preparer is still
  verified, it is reused and these files aren't read.
- `--server-jar PATH`: the first `--prepare` needs the server jar, which no launcher keeps. Horizon never
  downloads it; a launcher downloads it (the version JSON's `downloads.server`) only for a signed-in account
  that owns Java Edition, as `horizon-launcher` does.
- `--java-home PATH`: a Java of the pinned version that the launcher manages, tried after `JAVA_HOME` and the
  `PATH`, before the official launcher's runtimes.
- Exit codes: 0 done, 1 failed or gaps found, 2 a refused command line.

### `--progress json`

- Every line on stdout is one JSON object; stderr is human text.
- `{"message": string}`: a log line, for the player to read.
- `{"fraction": number}`: how far the work is, from 0 to 1, never decreasing.
- `{"error": string}`: why it failed. It is the last line, and the exit code is 1.
- Readers ignore keys they don't know, and lines with no key they know. A new key comes with a patch of the
  Launch API; a key whose meaning changes starts the next line ([stability.md](stability.md)).

## Examples

```sh
horizon-client --launch-api
printf '%s' "$SESSION_JSON" | horizon-client --resources ~/horizon/prepared --game-dir ~/instances/a --session-stdin --fullscreen
```

## Stability

Experimental: the options in the reference, the session JSON, the exit codes and the crash report line may still
change before preview. From preview on, they change only as [stability.md](stability.md) says, and the reference's
test keeps the contract and the code together.

## Not there yet

- **Asking the preparer what it supports:** `horizon-prepare --launch-api`, with its output version, schema and
  Java major.
- **Finding a build:** a signed release index and builds kept longer than seven days.
- **A guide**, "Running Horizon from another launcher", from picking a build to playing.

## Changelog

- 0.1.0 (not released yet, so still one version): `horizon-client`'s `--game-dir`, `--home`, `--fullscreen`,
  `--session-stdin`, `--launch-api`, exit code 2 for a refused command line; `horizon-prepare`'s `--output`,
  `--check`, `--complete`, `--progress json`, `--version-json`, `--client-jar`, `--libraries-dir`, `--assets-dir`
  and `--java-home`.
