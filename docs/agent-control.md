# Agent control: driving the launcher and the game

A player's own AI agent (Claude Code, Codex, Cursor, any MCP client) or a script can do
everything a player does with the mouse and keyboard, in the launcher and in the game. This page
covers developers: the surfaces, the full inventory, and the rules every new command follows.
Players read [Control Horizon with your AI agent](https://playhorizon.dev/docs/agents)
(playhorizon.dev/docs/agents).

The command lists are generated, so they can't drift:

- [launcher-commands.md](launcher-commands.md): every `horizon-launcher` command, its arguments
  and its MCP tool. It comes from the CLI's clap definition (`cli_tools::markdown`), and the
  launcher test `the_launcher_reference_is_up_to_date` fails when it is stale or a tool is missing.
- [horizon-ctl-commands.md](horizon-ctl-commands.md): every `horizon-ctl` command, its MCP tool
  and its arguments. It comes from the command table (`horizon-ctl help --markdown`), and the
  test `the_generated_reference_is_up_to_date` fails when it is stale.

## Surfaces

| Surface | Where | Transport |
| --- | --- | --- |
| Game control socket | `horizon-client --control` / `--control-socket PATH`, or Agent Control on in Horizon Settings > AI Agent (`crates/horizon-client/src/control.rs`; commands in `src/game/control.rs`, `agent.rs`, `ui_control.rs`, `saves_control.rs` and friends) | text line in, one JSON line out; Unix socket, or localhost TCP and a token where there is none |
| `horizon-ctl` | `crates/horizon-client/src/bin/horizon-ctl/` | CLI over the socket; one table of commands (`commands.rs`) gives the help, the argument checks, the MCP tools and the reference |
| `horizon-ctl mcp` | same | MCP over stdio, one tool per command of the table |
| Launcher CLI | `launcher/src/cli.rs` | `horizon-launcher <command> [--json]` |
| Launcher MCP | `launcher/src/mcp.rs`, `cli_tools.rs`, `game_tools.rs` | `horizon-launcher mcp` (stdio): every CLI command as a tool, the session tools, and the game's tools as `game_*` |
| Connecting | `launcher/src/agents.rs`, `agents_page.rs`; `src/game/agent_settings.rs` | `horizon-launcher agent-config`; Settings > AI in the launcher; Horizon Settings > AI Agent in the game |
| No AI | `crates/horizon-core/src/no_ai.rs` | `noAi:true` in `<game home>/horizon-extras.txt`, read by every surface |

## One MCP server for everything

An agent adds one server, `horizon-launcher mcp`. It holds:

- **The launcher's CLI, generated.** `cli_tools.rs` walks the clap definition. Each command is a
  tool named after its words (`mods search` is `mods_search`). Its arguments are the command's,
  in snake_case, with clap's help, choices and defaults. A call runs that command with `--json`
  as a child process and returns what it printed. A new CLI command is a new tool with no extra
  code.
- **Session tools**, which keep something running between calls:
  - `play` (the game, under control: `world`, `creative`, `join`, `offscreen`, `arg`);
  - `server`, `server_command`, `server_log`, `server_stop`;
  - `account_sign_in`, `account_list`.
- **The game's tools.** It runs the selected version's `horizon-ctl mcp` (shipped beside the
  game) as a child. It lists that server's tools as `game_<name>`, all but `launch`, and passes
  calls and results through, images included. `play` starts the game with a control socket in
  the launcher's data folder. While no game answers, `game_*` calls say to call `play`.

Why one server rather than two: the game's tools stay defined once, in horizon-ctl's table. The
agent's client needs one entry and one approval. The version that plays is always the one the
launcher selected, so the game's tools always match the game. `horizon-ctl mcp` stays available
on its own, for a game started by hand or by a script.

## Inventory, before and after

Before is `main` at 2026-10-23, before #1342, #1343, #1344, #1345 and #1353. After is with
them. CLI is `horizon-launcher` for the launcher and `horizon-ctl` for the game. MCP is
`horizon-launcher mcp` (the game's tools as `game_*`) and `horizon-ctl mcp`. **part** means only
through a generic command (`ui` and `click` by widget, or a raw socket line that horizon-ctl
passes on without checking it).

### Launcher

| Capability | Window | CLI before | MCP before | CLI after | MCP after | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| Releases and installed versions | Play, version picker | yes | yes | yes | yes | `versions [--installed]` |
| Install, remove a version | Play | yes | yes | yes | yes | `remove` needs `confirm` in MCP |
| Pick the version that plays | Play | no | no | yes | yes | `settings selected-version TAG` |
| Prepare resources | Play | yes | yes | yes | yes | |
| Start the game | Play | yes | yes | yes | yes | `play --world --creative --join --arg --account`; MCP `play` adds `offscreen` |
| Join a friend by invite code | Play | yes | no | yes | yes | `play --join CODE` |
| Dedicated server here: start, console, log, stop | Servers | yes | yes | yes | yes | `server`; MCP `server`, `server_command`, `server_log`, `server_stop` |
| Hosted servers: list, create, set up, run a command, forget | Servers | yes | no | yes | yes | `servers ...`; `servers_forget` needs `confirm` |
| Accounts: list, sign in, sign out, select, offline | Settings | yes | part | yes | yes | names and UUIDs only; sign-in is a device code; `account_offline` was missing |
| Game folder, server folder, install new versions, opening animation | Settings | no | no | yes | yes | `settings [KEY VALUE]` |
| Extra game arguments | Settings | yes | no | yes | yes | `game-args` |
| Applications menu entry (Linux) | Settings | yes | no | yes | yes | `desktop-entry` |
| Updates: check and install | Settings | yes | part | yes | yes | MCP could only check |
| Patch notes, links | Patch notes, footer | yes | yes | yes | yes | |
| Game log, crash reports | Settings | yes | yes | yes | yes | `logs [--crash]` |
| Data collection: status, allow, deny, show, delete, send, bug, benchmark upload | Settings > Data | yes | part | yes | yes | sending was missing in MCP; `data_delete` needs `confirm` |
| Mod browser: search, check, install, update, remove, list (mods, resource packs, modpacks) | Mods | yes | no | yes | yes | `mods ...`; `mods_remove` needs `confirm` |
| Modpacks: import, port, play, remove, list | Modpacks | yes | no | yes | yes | `modpack ...`; `modpack_remove` needs `confirm` |
| Launcher benchmark | (none) | yes | no | yes | yes | |
| The game it started | (the game) | (horizon-ctl) | yes | (horizon-ctl) | yes | `game_*` |
| Connect an AI agent | Settings > AI | no | (none) | yes | (none) | `agent-config [--client] [--test]`; the agent is already connected, so there is no tool |
| No AI | Settings > AI | no | (none) | yes | (none) | `no-ai [on\|off]`; the player's switch, never an agent's |

### Game

| Capability | Screen | CLI before | MCP before | CLI after | MCP after | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| Start, wait until ready, save, quit, log | (all) | yes | yes | yes | yes | `launch`, `wait-ready`, `save`, `quit`, `logs` |
| Every widget of every screen, by id; click, set, type | (all) | yes | yes | yes | yes | `ui`, `click ID`, `set ID VALUE`, `type`; `click-at`, `drag-at`, `scroll-at` by pixel |
| Title menu, pause menu | Title, Pause | yes | yes | yes | yes | `ui`/`click`, `pause`, `resume` |
| Worlds: list | Select World | part | part | yes | yes | `worlds` |
| Worlds: create, open | Create New World | yes | yes | yes | yes | `open WORLD [SEED] [creative]` |
| Worlds: rename, back up | Edit World | part | part | yes | yes | `world-rename`, `world-backup` |
| Worlds: delete | Select World | part | part | yes | yes | `world-delete FOLDER --confirm`; it was possible through `click` without any confirm |
| Servers list | Multiplayer | part | part | yes | yes | `servers` (names and addresses; reads servers.dat) |
| Join a server or an invite code, leave | Multiplayer | part | no | yes | yes | `connect`, `disconnect` |
| Open to LAN | Pause > Open to LAN | part | no | yes | yes | `lan open\|close\|status` |
| Play with Friends: host, stop, status | Pause | part | no | yes | yes | `friends host\|stop\|status` |
| Vanilla options, every page | Options | yes | part | yes | yes | `options-get`, `options-set` (all of options.txt, typed); `options PAGE` was socket-only |
| Horizon Settings: Graphics, Controller, Extras, Mods, AI Agent | Horizon Settings | part | part | yes | yes | `options horizon\|graphics\|extras\|agent...`, then `ui`/`click`/`set` |
| Agent control on or off, status, connection details | Horizon Settings > AI Agent | no | no | yes | yes | new page (#1353) |
| Key binds | Controls > Key Binds | yes | yes | yes | yes | `options-set key_key.jump key.keyboard.j` |
| Resource packs | Options > Resource Packs | part | part | yes | yes | `options packs`, then `ui` rows and `click`; `options-set resourcePacks` |
| Chat and commands | Chat | yes | yes | yes | yes | `chat`, `command`, `messages`, `chat-log` |
| Inventory, containers, stations | Inventory, Chest... | yes | part | yes | yes | `inventory`, `screen`, `click-slot`; `container X Y Z` was socket-only |
| Crafting, recipe book | Crafting | part | part | yes | yes | the grid by `click-slot`; the recipe book by `ui`/`click` |
| Trading, riding, shearing (right-click an entity) | Merchant, Horse | part | no | yes | yes | `interact-entity ID`; `mouse` was CLI-only |
| Horse, llama inventory | Horse | no | no | yes | yes | `mount-menu` |
| Books, lecterns, signs | Book, Sign | part | part | yes | yes | `book [next\|back\|take\|done]`, `signedit X Y Z` |
| Move, look, find a path | (playing) | yes | yes | yes | yes | `move`, `goto`, `look`, `look-at`, `turn`, `jump` |
| Mine, place, use, attack, hotbar, drop | (playing) | yes | yes | yes | yes | `break`, `place`, `interact`, `use-item`, `attack`, `select`, `drop` |
| Raw keys and mouse buttons | (all) | yes | part | yes | yes | `key`; `mouse`, `release` were CLI-only |
| Controllers | (all) | yes | no | yes | yes | `pad` |
| Respawn | Death | yes | yes | yes | yes | `respawn` |
| Screenshots | (all) | yes | yes | yes | yes | PNG of the next frame, `--offscreen` too |
| F3 data | F3 | yes | part | yes | yes | `key f3`; `status` (renderer, chunks, memory) was CLI-only |
| Waits and events | (all) | yes | yes | yes | yes | `wait`, `wait-for`, `events` |
| Effects, experience, enchantments, potions, fill | (cheats) | part | no | yes | yes | `effect`, `effects`, `xp`, `enchant`, `potion`, `fill` |
| Benchmarks and frame stats | (all) | yes | part | yes | yes | `bench`, `stats` were CLI-only; `fly` |

## No AI

One switch hides and turns off every AI feature: `noAi:true` in `<game home>/horizon-extras.txt`.
The game home is `HORIZON_HOME`, else the data dir's `horizon`. The game reads its extras from
its working directory, which is that folder. `horizon_core::no_ai` reads and writes the switch:

- `is_on(home)` and `check(home)` read it;
- `set(home, on)` keeps the file's other lines, and writes no file to turn off a switch that was
  never on;
- `REFUSAL` is the one line every surface prints.

The switch is in three places: Settings > AI in the launcher, Horizon Settings in the game
(`Item::NoAi`), and `horizon-launcher no-ai on|off`. When it is on:

| Surface | What happens |
| --- | --- |
| Game | opens no control socket, even with `--control` (it logs the refusal); Horizon Settings hides AI Agent...; turning it on in a running game closes the socket at once |
| `horizon-ctl` | every command refuses: exit status 4, `{"error": REFUSAL, "kind": "denied"}` |
| `horizon-ctl mcp` | refuses to start (exit 4, REFUSAL on stderr); a session already running refuses each call |
| `horizon-launcher mcp` | refuses to start; a running session answers each `tools/call` with error -32000 and REFUSAL |
| Launcher CLI | `mcp` and `agent-config` are hidden from the help and refuse |
| Launcher window | Settings > AI shows only the No AI switch, without Connect an AI agent |
| Companion (#1068) | uses the same switch (see horizon-parity companion-notes.md) |

Every other launcher command stays usable: versions, install, remove, prepare, play, server,
notes, links, account, settings, game-args, logs, update, desktop-entry, data,
modpack, mods, servers, no-ai and benchmark. None of them drives the game for an agent. The
launcher itself, packagers, the docs and players' scripts need them, and `no-ai` has to stay to
turn the switch off.

## Connecting

- **Launcher, Settings > AI > Connect an AI agent:**
  - a client picker (Claude Code, Codex, Cursor, any MCP client) with its config and a Copy button;
  - for clients with a CLI, the add command, shown and run only on **Add to ...**:
    `claude mcp add --scope user horizon -- <exe> mcp`, `codex mcp add horizon -- <exe> mcp`;
  - **Test the connection** (`agents::self_test`) starts `<exe> mcp`, lists its tools and says
    how many answered.
  - `horizon-launcher agent-config [--client ID] [--json] [--test]` prints the same thing.
- **Game, Horizon Settings > AI Agent:**
  - **Agent Control** (`agentControl` in horizon-extras.txt, off by default) opens the default
    endpoint (`$HORIZON_CONTROL`, else `horizon-control.sock` in the runtime directory) in a game
    the player started. Turning it off closes the endpoint and stops its listener.
  - The page shows whether a client is connected (`Server::clients`), when the last command came,
    and the socket's name.
  - **Copy Connection Details** copies the socket path, a `horizon-ctl --socket ... state` line
    and an `mcpServers` config for this game's `horizon-ctl mcp`.
- **The default:** a game an agent starts (`play` in MCP, `horizon-ctl launch`, `--control`) is
  under control. A game the player starts from the window is not, until they turn Agent Control
  on. An open socket lets any program of that user drive the game, so it opens only when an agent
  asked for the game, or when the player chose it.
- **Headless:** `horizon-ctl launch --headless` (`horizon-client --headless --control`) starts a
  game with no display, no window and no GPU: it runs where no display server is (an SSH session,
  a server) and runs its own loop instead of winit's (`game/headless.rs`), one frame a tick. It
  plays through these commands only; commands that need the screen's pixels (`screenshot`, clicks
  at GUI points) answer with an error ([#4951](https://github.com/horizon-rust/horizon/issues/4951)).

## Design rules for a new command

### One vocabulary

- **A command has one name**, kebab-case, the same on the socket, in `horizon-ctl` and in the
  launcher CLI. Its MCP tool is the name with `_` for `-` (`click-slot` is `click_slot`). In the
  launcher, a subcommand's words are joined (`mods search` is `mods_search`). The launcher's MCP
  offers every game tool as `game_<name>`, so none collides with its own (`logs` is on both
  sides). A launcher tool whose name starts with `game_` (`game_args`) is matched first.
- **Arguments:** positional words, then `--flags`.
  - The game: the `horizon-ctl` table (`Param`) is the source of truth. It generates the MCP
    `inputSchema` and the reference. A game command is not done until it is in that table.
  - The launcher: the clap definition is the source of truth. Give every argument a help line;
    it becomes the tool's description.
- **Replies are one JSON object.** A query returns its data; an action returns
  `{"ok": true, ...}` plus what changed (`clicked`, `screen`, `drops`...).
- **Errors** are `{"error": "<sentence an agent can act on>", "kind": "<reason>"}`. The kinds:
  - `usage`: a bad or missing argument;
  - `unknown`: no such command, widget, option or entity;
  - `state`: the game is not in a state that allows it (no world, wrong screen, disabled widget);
  - `busy`: another action or a benchmark is running;
  - `timeout`;
  - `connection`: the game can't be reached;
  - `denied`: No AI is on (exit status 4).

  The message names what to do next ("no world open; use: open WORLD").
- **Shapes are shared:**
  - a position is `[x, y, z]`;
  - a block is `{"pos", "block", "state", "loaded"}`;
  - an item stack is `{"item", "count", "damage"?, "components"?}`;
  - a slot is a stack plus `"slot": ID` (the slot ids of `click-slot`);
  - bounds are `[x, y, w, h]` in GUI pixels, the coordinates of `click-at`.

### Safety

- **Local only.** The socket is a Unix socket in the user's runtime directory, or a TCP port on
  127.0.0.1 behind a 128-bit token in a file only the user can read. It is never `0.0.0.0` and
  never a network listener. Both MCP servers use stdio.
- **Confirm what can't be undone.** A command that deletes the player's things requires
  `confirm` and carries the MCP `destructiveHint`. In the game these are `world-delete`, `fill`
  and `setblock` (the table's `DESTRUCTIVE`); in the launcher, `remove`, `modpack remove`,
  `mods remove`, `servers forget`, `account sign-out` and `data delete` (`cli_tools::DESTRUCTIVE`).
  Without `confirm: true` the call is refused and the message says to ask the player. In the game
  CLI the flag is `--confirm`. The launcher CLI asks nothing: a person typing `remove` meant it.
- **Ask before reaching out.** A command that sends data or reaches other people carries
  `openWorldHint`, and its description says "only when the player asks". In the game these are
  `connect`, `lan`, `friends`, `chat` and `command`; in the launcher, `data allow`, `data send`,
  `data bug`, `data benchmark` and `servers create`. `play`'s `join` argument says the same.
  Telemetry stays opt-in, and only `data allow` turns it on.
- **No secrets.** No surface returns tokens, session files, keychain entries or invite secrets
  the player didn't ask to share:
  - accounts are names and UUIDs;
  - sign-in is a device code the player enters;
  - `--session-file` is read once and deleted;
  - `servers` reads names and addresses only.
- **Vanilla behaviour untouched.** Control code reads state and synthesizes the input events a
  player produces; registering widgets for the tree draws nothing. A change to game files runs
  `tools/parity.py --changed origin/main`.

### The widget tree

`ui` returns every interactive element the last frame drew, on every screen:

```json
{"screen": "options", "gui_size": [427, 240], "frame": 3061,
 "widgets": [
   {"id": "fov", "kind": "slider", "label": "FOV: 70", "bounds": [40, 50, 150, 20], "enabled": true, "value": 0.5},
   {"id": "done", "kind": "button", "label": "Done", "bounds": [...], "enabled": true}]}
```

- **Ids are stable and readable.** An id is the label up to its `:`, as a slug (`FOV: 70` is
  `fov`, `AI Agent...` is `ai-agent`). A repeated slug gets `-2`, `-3`; unlabelled elements are
  `<kind>-<n>`. A slider's id doesn't change with its value. Slots are `slot-<ID>` with their
  stack.
- **Kinds:** `button`, `slider` (`value` 0..1), `field`, `row` (a world, server, pack or
  language), `tab` (`selected`), `slot`.
- **Actions:**
  - `click ID [right|middle] [shift]` clicks where the mouse would.
  - `set ID VALUE` moves a slider.
  - `type` types into the focused field.

  After a click, the next `ui`, `click` or `set` waits for a redrawn frame.
- **No new code paths in screens.** The tree is built from what the draw functions already
  register for controller navigation (`widgets.rs`).

### Waits and events

- `wait-for screen NAME|widget ID|loaded|chat TEXT|event KIND [--timeout S] [--since SEQ]`
  replies when the condition holds, or with `kind: timeout`.
- `events [--since SEQ]` returns what happened since SEQ: screens, chat, deaths, respawns, items,
  dimensions and worlds (the newest 256). Take `last` before an action, and wait with `--since`,
  so the action's event can't slip in before the wait.

## Tests

- **Unit tests:**
  - horizon-ctl: the table, the schemas, the reference, the No AI refusal, the destructive and
    network annotations;
  - the launcher: generated tools, `confirm`, the reference, the agent configs;
  - `horizon_core::no_ai`;
  - `options_ui`: the AI Agent page, and that No AI hides it.
- **`tools/agent_control_e2e.py`** drives a real game, offscreen on macOS and on a virtual screen
  on Linux, through the CLI and MCP:
  - `survival`, `ui`, `options`, `events`, `consume`, `pick`, `suggestions`;
  - `manage`: worlds, servers, effects, books, LAN, the options pages and `confirm`;
  - `no-ai`: every surface refuses;
  - `setting`: Agent Control on opens the endpoint, the AI Agent page, Copy, and off closes it.
- **`tools/launcher_mcp_e2e.py`** drives `horizon-launcher mcp`: the generated tools, `play` and
  the `game_*` tools, `confirm`, `agent-config` and `--test`, and No AI.

## Later

These are tracked in FEATURES.md (LATER):

- Horizon's own option files by key (`horizon-graphics.txt`, controller, extras) next to
  `options-get` and `options-set`.
- Adding and removing saved servers by command.
- `subscribe`: events as a stream on one connection.
- Launcher window screenshots in this page's PRs.
- No AI changed in the launcher while a game is open: the game rewrites its extras when it saves.
- A confirmation question in the launcher CLI for destructive commands.
