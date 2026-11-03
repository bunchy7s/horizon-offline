<!-- Generated from crates/horizon-client/src/game/launch_api.rs; a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test -p horizon-client --lib launch_api -->
# Launch API reference

What `horizon-client` promises a launcher that starts it, Launch API 0.1.0. The guide: [launch.md](launch.md); stability and versions: [stability.md](stability.md).

## Options

Any other option is refused with exit code 2, so ask `--launch-api` before passing one a build may not know.

| Option | What it does |
| --- | --- |
| `--resources DIR` | The prepared resources (horizon-prepare's output). Default: `resources` beside the executable, else `.local/horizon-client/26.3`. |
| `--game-dir DIR` | The game folder: `saves/`, `options.txt`, `resourcepacks/`, `horizon-mods/`, `screenshots/`. Created when missing. Default: the working directory. `--gameDir` is the same. |
| `--home DIR` | Logs (`logs/latest.log`) and crash reports (`crash-reports/`). Default: `HORIZON_HOME`, else `horizon` in the platform data folder. |
| `--player-dir DIR` | One player's own files, for several games playing in one game folder at the same time: `options.txt` and the settings beside it, `logs/`, `crash-reports/`, `horizon-player.txt`, `servers.dat`, `hotbar.nbt`, `screenshots/`. Saves, packs, mods and their config, the chat's command history and Play with Friends' relay settings stay in the game folder. A relative path is taken from the folder the game starts in. Created when missing. Default: the game folder, and `--home` for logs. Crash telemetry still follows `--home`'s consent, and `horizon-ctl logs` and the launcher read `--home`'s logs. |
| `--width N` | The window's starting width (default 1280). |
| `--height N` | The window's starting height (default 720). |
| `--fullscreen` | Opens the window fullscreen, whatever `options.txt` says; `options.txt` is not changed. |
| `--session-file PATH` | The signed-in account, as the session JSON below. Read once, then deleted. |
| `--session-stdin` | The same JSON on standard input, so it never touches the disk. |
| `--quickPlaySingleplayer WORLD` | Opens this world of `saves/` at start, only if it exists (else the title screen). WORLD is one folder name: a path is refused. |
| `--quickPlayMultiplayer HOST[:PORT]` | Joins this server at start. |
| `--launch-api` | Prints the reply below and exits 0, before anything else runs. |

## `--launch-api`

Prints one JSON object on stdout and exits 0. `horizon` is the build's version; readers ignore fields they don't know.

```json
{
  "crash_report_line": "Horizon: crash report saved to ",
  "exit_codes": [
    0,
    1,
    2,
    101
  ],
  "horizon": "<version>",
  "launch_api": "0.1.0",
  "minecraft": "26.3",
  "options": [
    {
      "name": "--resources",
      "value": "DIR"
    },
    {
      "name": "--game-dir",
      "value": "DIR"
    },
    {
      "name": "--home",
      "value": "DIR"
    },
    {
      "name": "--player-dir",
      "value": "DIR"
    },
    {
      "name": "--width",
      "value": "N"
    },
    {
      "name": "--height",
      "value": "N"
    },
    {
      "name": "--fullscreen",
      "value": null
    },
    {
      "name": "--session-file",
      "value": "PATH"
    },
    {
      "name": "--session-stdin",
      "value": null
    },
    {
      "name": "--quickPlaySingleplayer",
      "value": "WORLD"
    },
    {
      "name": "--quickPlayMultiplayer",
      "value": "HOST[:PORT]"
    },
    {
      "name": "--launch-api",
      "value": null
    }
  ]
}
```

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The player quit, or `--help` or `--launch-api` printed. |
| 1 | The game stopped on an error. A crash report was saved. |
| 2 | The command line was refused: an unknown option, a missing or bad value. No crash report. |
| 101 | The game panicked. A crash report was saved. |

When a crash report is saved, the last line on stderr is `Horizon: crash report saved to` followed by the report's path.
