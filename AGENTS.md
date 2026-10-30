# Horizon: a map for agents

Minecraft: Java Edition 26.3 rewritten in Rust (wgpu). Vanilla is the specification wherever it has the
feature. Read [CONTRIBUTING.md](CONTRIBUTING.md) once; this page tells you where things are.
Feature → files → harness for the most-changed areas: [docs/architecture.md](docs/architecture.md).

## Rules that bite

- **Never commit Mojang's files**: game jars, assets, decompiled source, prepared resources (`.local/`).
  `tools/check_public_tree.py` rejects them; `git config core.hooksPath .githooks` runs it on commit.
- **Keep your runs away from real player data.** Any game or launcher you start gets fresh `mktemp -d`
  folders in `HORIZON_HOME` (the game's data), `HORIZON_LAUNCHER_HOME`, `XDG_DATA_HOME` and
  `XDG_CONFIG_HOME`, plus `HORIZON_LAUNCHER_NO_SELF_UPDATE=1` and `HORIZON_NO_AUDIO=1`, and, for the
  launcher, `HORIZON_LAUNCHER_TRASH`, `HORIZON_LAUNCHER_CLIPBOARD` and `HORIZON_LAUNCHER_DESKTOP` pointing into the scratch home. Never write to
  `~/.local/share/{applications,horizon,horizon-launcher}` or `~/.local/bin`.
- **Mark what an agent posts.** A PR body starts with a line naming the person whose agent opened it and
  the tool: `> 🤖 Opened by <login>'s AI agent (<tool, model>), not by <login> personally.`, where `<login>`
  is the GitHub account the PR is opened from (`gh api user --jq .login`). Issues say "Filed by", reviews
  "Review by". Every comment starts with `🤖 Agent:`. Write only as the person you work for: the lines on
  this page that name Ludovic are for his own agents (account `ludovic111`), never for anyone else's.
- **Attribution.** Commits end with a `Co-Authored-By:` line for the agent that wrote them, and PR bodies
  with that agent's own "Generated with" line. Example, Ludovic's agents (Claude Code on `ludovic111`):
  first line `> 🤖 Opened by Ludovic's AI agent (Claude), not by Ludovic himself.`, commits
  `Co-Authored-By: Claude <model> <noreply@anthropic.com>`, last line
  `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- Everything players run is Rust. Python/Java only for dev tools and harness drivers in `tools/`.
- Horizon-only features follow [docs/design-system.md](docs/design-system.md) and are off by default where
  they would change the vanilla look.
- Small PRs (< ~500 lines). Labels: `feature` (players get something new), `skip-release` (they see no
  change), none for a fix. Agents never add `release-now`: the maintainer decides it. Merges ship in batches of 5, or after
  6 hours ([docs/release-flow.md](docs/release-flow.md)).
- Contributors first: `gh pr list --state open --search <area>` before you start; never duplicate theirs.
- **Claim before you start**: comment `/claim` on the issue (no issue yet: open one, then claim it). An assigned
  issue is taken: pick another. `/unclaim` when you stop; a claim with no open PR linking it is released after 5 days.

## Crates

Game workspace (root `Cargo.toml`); `launcher/` and `crates/horizon-telemetry-server` are separate workspaces.

| Crate | Owns |
|---|---|
| `horizon-client` | The game: a lib (`src/lib.rs`, ~100 flat modules: gameplay, networking, title screens) and the bin (`src/main.rs` → `game.rs` + `game/`), plus `horizon-ctl` (`src/bin/horizon-ctl/`). Also the dedicated server (`--dedicated`) |
| `horizon-items` | Blocks and items below gameplay: `blocks.rs` (block facts), `items.rs`, `item_data.rs` (components), `enchant.rs`, `cooldowns.rs`, `bundle.rs`, `item_nbt.rs`. Re-exported as `horizon_client::items`... |
| `horizon-command` | Command parsing (Brigadier): the vanilla tree, argument types, selectors, suggestions, the tree's network form. Re-exported as `horizon_client::command` |
| `horizon-client-core` | The client lib's bottom layer: `text.rs`, `lang.rs`, `snbt.rs`, `nbt_path.rs`, `component.rs`, `scoreboard.rs`, `bossbar.rs`, `gamerules.rs`, `block_entity_nbt.rs`, registry/particle/sound tables. Re-exported under the old paths (`horizon_client::text`...) |
| `horizon-client-input` | Key bindings and key names, toggle keys, mouse options, controller settings and glyphs, the clipboard, window options (`keybinds.rs`, `gamepad.rs`, `mouse.rs`, `window.rs`...). Re-exported under the old paths |
| `horizon-client-render` | The client's world data, rendering and screens (chunks, meshing, light, streaming, `gfx`, HUD, menus, settings): ~80 modules that horizon-client re-exports under the same paths. Nothing in it reaches gameplay |
| `horizon-world` | Vanilla world data with no gameplay loop: NBT, regions, chunks, `datafix/`, `worldgen/`, `level/` (server-side block behaviour, ticks, redstone), entity physics, items, effects, food, damage |
| `horizon-core` | Simulation primitives: Java `Random`s, `jmath` (HotSpot-exact `Math`), positions, noise, density, paletted storage, `version.rs` (the pinned version) |
| `horizon-protocol` | Network protocol 777: codecs, framing, encryption, `packets/{handshake,status,login,configuration,play}.rs`, `entity_data.rs` |
| `horizon-assets` | Resource decoding: block-state and item models, baking, atlases |
| `horizon-prepare` | Builds `.local/horizon-client/26.3` from the player's own Minecraft install (runs the Java exporters) |
| `horizon-server` | `horizon-server`: `server.toml`, CLI, supervisor of the headless game ([docs/hosting.md](docs/hosting.md)) |
| `horizon-server-files` | `server.properties`, eula, ops/whitelist/bans, usercache |
| `horizon-friends`, `horizon-relay` | Play with Friends: invite codes, encrypted tunnel, the relay ([docs/play-with-friends.md](docs/play-with-friends.md)) |
| `horizon-mods`, `cargo-horizon-mod` | Content mods (`.hmod`: manifest, content fence, no code), the registry index and `.hpack`, and `cargo horizon-mod` (`new`, `build`, `run`, `check`, `registry`) ([docs/mods.md](docs/mods.md)). Code mods are native, built by the launcher ([docs/native-mods.md](docs/native-mods.md)) |
| `horizon-port-pack` | Porting Java modpacks ([docs/modpacks.md](docs/modpacks.md)) |
| `horizon-shader-api` | Horizon's own WGSL shader packs ([docs/api/shaders.md](docs/api/shaders.md)): `horizon-shaders.toml`, the `.hshader` package; shared by the game, the launcher and the registry |
| `horizon-shaders` | Shader packs (the `shaders/` folder format), rebuilt clean-room from a written spec of the format; no wgpu |
| `horizon-theme` | Design tokens and fonts for the launcher, the game's own menus and the website |
| `horizon-telemetry`, `horizon-telemetry-server`, `horizon-bench` | Opt-in data collection, its ingest server, the benchmark report format |
| `horizon-dualsense`, `horizon-threads`, `horizon-release-sign`, `horizon-bots`, `xtask` | DualSense output, thread priorities, signed release checksums, load-test bots, repo tasks |
| `launcher/` | GPUI launcher: `main.rs` (UI), `backend.rs` (installs, updates), `cli.rs`, `mcp.rs`, `*_page.rs` |

Other top-level folders: `tools/` (harnesses and dev scripts), `tests/` (Python unit tests: `python3 -m
unittest discover -s tests`), `mods/horizon/src/test/java/dev/horizonlab/parity/` (Java oracles),
`docs/`, `.github/workflows/`. The website (playhorizon.dev) is the private horizon-rust/horizon-site: its build
reads `README.md`, `launcher/links.json`, `docs/api/`, `roadmap/roadmap.json` and `crates/horizon-theme/tokens.css` from main.

## Build and test fast

```sh
python3 tools/pr_ready.py                  # the self-check: fmt, clippy -D warnings, touched crates' tests, harnesses; paste its block in the PR
python3 tools/pr_ready.py --quick          # same without harnesses
cargo test -p horizon-client --lib --bins --tests   # skips the 104 examples (pr_ready does this); `--doc` separately
cargo test -p horizon-client --lib mod_content::tests:: -- --ignored   # the 5 slow content-root tests, #[ignore]d; pr_ready runs them when mod content, assets, blocks or items change
cargo build --profile fast -p horizon-client        # optimized, quick to link: use it to play or run lan_e2e
python3 tools/parity.py --changed origin/main       # the harnesses your branch can affect (CI's parity job)
python3 tools/parity.py --changed-files paths.txt --list   # which harnesses cover these files, without running them
python3 tools/parity.py mob_ai sculk       # named harnesses
python3 tools/lan_e2e.py --check chest     # two real games over LAN (server-thread host)
python3 tools/agent_control_e2e.py         # horizon-ctl / MCP end to end
```

- Harnesses and the game need the vanilla data in `.local/` (`minecraft/`, `horizon-client/26.3`, `datagen/`,
  `tools/`): see [docs/parity-workflow.md](docs/parity-workflow.md). Without it, pr_ready lists the
  harnesses for CI.
- A harness = `tools/test_<name>_parity.py` (324 of them) + a Java oracle (`mods/horizon/src/test/java/dev/horizonlab/parity/Vanilla*Oracle.java`)
  + a Rust probe: `crates/<crate>/examples/<name>_{probe,fixture}.rs` (horizon-world 175, horizon-client 104),
  or the game binary with a fixture env var (`game/dispense_fixture.rs`, `game/sculk_fixture.rs`,
  `game/command_harness.rs`, `game/net_harness.rs`). Copy a nearby harness for a new one.
- Probes build into `.local/parity-target/`; harnesses find binaries via `lab.cargo_target()`, never `target/`.
- What a change reruns (of 329 harnesses, measured 2026-10-23): a file of the game binary (`game.rs`, `game/`)
  ~25, a file of the client lib ~103, of `horizon-world` ~103–109, of `horizon-core` ~119. Plan for it.
- Rust integration tests: `crates/horizon-client/tests/` (`lan_bot.rs`, `generation_saves.rs`, `streaming_meshes.rs`).
- Linux needs `libasound2-dev libudev-dev`; `--no-default-features` drops audio and controllers (the server build).
- Run and drive the game (starts the `horizon-client` beside `horizon-ctl`; other arguments go to the game):
  `target/fast/horizon-ctl --socket $PWD/ctl.sock launch --world W --creative --offscreen --wait --resources $PWD/.local/horizon-client/26.3`,
  then `horizon-ctl --socket … state|keys|turn|command|screenshot|quit`
  ([docs/horizon-ctl-commands.md](docs/horizon-ctl-commands.md)).

## Vanilla reference

- Start parity work from the port ledger, not grep: `python3 tools/port_ledger.py issue N` (or `where
  Class.method`, `what file:line`), and `check --base origin/main` before the PR. How:
  [.claude/skills/vanilla-parity/SKILL.md](.claude/skills/vanilla-parity/SKILL.md), [docs/port-ledger.md](docs/port-ledger.md).
- Decompiled sources: `.local/minecraft/26.3/sources/{client,server}/` (`python3 tools/lab.py decompile`).
- Bytecode queries (callers through lambdas, every overload):
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`
  ([docs/decompilation-status.md](docs/decompilation-status.md)).
- A harness runs the oracle against the real 26.3 jars and the Rust probe on the same inputs, then compares
  their JSON. Oracle runs are cached by `tools/parity_shim.py`; `--fresh` reruns them.
- Mirror vanilla's `Math` calls with `horizon_core::jmath`, and vanilla's RNG with `horizon_core::random`:
  random draws must happen in vanilla's order.

## horizon-client: where things live

`L:` = lib (`crates/horizon-client/src/`, or `crates/horizon-client-render/src/` for the modules that crate holds, under
the same `horizon_client::<module>` paths), `G:` = bin (`crates/horizon-client/src/game/`). The lib can't see
`Game`; bin code reaches the lib as `horizon_client::<module>`.

| Subsystem | Files |
|---|---|
| Game loop, tick, frame | `game.rs`: `fn run`, `impl ApplicationHandler for Game` (`window_event`, `about_to_wait`), `fn tick`, `fn tick_world`, `fn render`; L: `clock.rs`, `spans.rs`; `horizon-client-input/src/window.rs` |
| Rendering | L: `gfx.rs` (`Renderer::new`, `Renderer::render`), `gfx/` (cull, occlusion, translucency, upload, timing, lod, weather, stars, end_sky), `*.wgsl`, `particles.rs`, `block_entity_render.rs`, `item_render.rs`, `fog.rs`, `clouds.rs`, `camera.rs`, `first_person.rs`; G: `render_tick.rs`, `item_draw.rs` |
| Chunks, streaming, light | L: `streaming.rs` (worker pool: generate, light, mesh), `world.rs`, `chunk.rs`, `chunk_map.rs`, `light.rs`, `mesher.rs` + `mesher/`, `vanillagen.rs` (binds horizon-world's generator), `lod.rs` + `lod/`, `ticket_levels.rs`; G: `world_loading.rs`, `postprocess.rs`; game.rs `fn update_world` |
| World generation | `crates/horizon-world/src/worldgen/` (`generator*.rs`, `feature_graph.rs`, `structure/`, `carver/`) |
| Block behaviour | `crates/horizon-world/src/level/` (`mod.rs`, `tick.rs`, `hooks.rs`, `redstone.rs`, `place.rs`, `interact.rs`); game.rs `fn tick_blocks`, `fn process_level_events` |
| Mobs | L: `mobs.rs` (types), `mobs/` (`ai.rs`, `goals.rs`, `brain.rs`, `*_brain.rs`, `nav.rs`, `path.rs`, `species.rs`, `spawning.rs`, `render.rs`, `anim.rs`); G: `creatures.rs` (ticking, spawning, mob events), one file per mob family (`dragon.rs`, `wither.rs`, `villagers.rs`, `piglins.rs`, `golems.rs`...), `natural_spawning.rs` |
| Other entities | L: `entities.rs`, `projectiles.rs`, `arrows.rs`; G: `boats.rs`, `minecarts.rs`, `mounts.rs`, `armor_stands.rs`, `paintings.rs`, `frames.rs`, `fireworks.rs`, `tnt.rs`, `explosion.rs` |
| Player actions | game.rs (`fn use_hand`, `fn use_item`, `fn attack`, `fn tick_mining`, `fn break_block`); G: `hand.rs`, `combat.rs`, `consume.rs`, `potions.rs`, `shield.rs`, `crossbow.rs`, `trident.rs`, `fishing.rs`; L: `motion.rs` + `motion/` (movement), `collisions.rs`; crate `horizon-items`: `items.rs`, `item_data.rs`, `enchant.rs`, `blocks.rs` |
| Inventory and containers | L: `menu.rs` (`AbstractContainerMenu.clicked`), `container_screen.rs`, `containers.rs`, `creative.rs`, `crafting.rs`, `recipe_book*.rs`, `stations.rs`, `merchant.rs`; game.rs `fn menu_ops`, `fn click_inventory`; G: `stations.rs`, `trading.rs`, `recipe_book.rs`, `machines.rs` |
| Commands | parse: crate `horizon-command` (`args.rs`, `selector.rs`, `tree_*.rs`, `vanilla.rs`); run: G: `commands.rs` (dispatch) + `commands/*.rs` (`admin`, `execute`, `items`, `players`, `world_cmds`...); L: `scoreboard_cmd.rs`, `data_cmd.rs`; `horizon-client-core`: `nbt_path.rs`, `scoreboard.rs`; G: `functions.rs`, `command_blocks.rs` |
| UI screens | `Screen` enum in game.rs; L: `title.rs` + `title/`, `options_ui.rs`, `multiplayer.rs`, `pack_screen.rs`, `graphics_ui.rs`, `screen_draw.rs`, `ui.rs` (widgets), `hud.rs`, `hud_layout.rs`, `hud_overlays.rs`, `chat.rs`, `tab_list.rs`; G: `startup_menu.rs`, `screens.rs` (pause, death), `options.rs`, `lan_screen.rs`, `chat.rs`, `sign_edit.rs`, `books.rs` |
| Multiplayer client | L: `net.rs` (network thread); G: `remote.rs` (`fn handle_packet`, `fn tick_remote`), `remote_*.rs` (mob data, objects, projectiles, vehicles, stations) |
| LAN host / server side | L: `lan/` (`conn.rs` handshake + login threads, `hub.rs` + `hub/` play state, `chunks.rs`, `entity.rs`, `tracking.rs`, `guest.rs`, `playerdata.rs`); G: `lan.rs` (`LanHost`), `lan_actions.rs` (`fn as_guest`, `fn tick_guests`), `lan_*.rs` per feature, `lan_entities.rs` (tracked entities) |
| Integrated server | G: `integrated.rs` (the server thread), `levels.rs` (one level per dimension, `fn tick_server`), `server_travel.rs` |
| Dedicated server | G: `dedicated.rs`, `dedicated/status.rs`; crates `horizon-server-files`, `horizon-server` |
| Save and load | L: `save.rs` (level.dat, regions, playerdata), `entity_nbt.rs`, `options_file.rs`; `horizon-client-core/src/block_entity_nbt.rs`; G: `saving.rs`, `persist.rs` (entities); `crates/horizon-world/src/{region.rs,nbt.rs,datafix/}` |
| Input | game.rs `fn window_event`, `fn key_pressed`, `fn play_mouse`; crate `horizon-client-input` (`keybinds.rs`, `keymapping.rs`, `key_names.rs`, `mouse.rs`, `input.rs`, `interaction_input.rs`, `gamepad.rs`, `clipboard.rs`); L: `osk.rs`; G: `pad.rs`, `pad_menu.rs` |
| Audio | L: `audio.rs` (mixer), `sound_engine.rs`, `sound_events.rs`, `music.rs`, `ambience.rs`; G: `music.rs`, `subtitle_hud.rs` |
| Resource packs | L: `packs.rs` + `packs/`, `pack_screen.rs`, `atlas_sources.rs`, `sprites.rs`, `mipmap.rs`, `model_library.rs`, `font.rs`; `horizon-client-core/src/lang.rs`; crate `horizon-assets`; G: `packs.rs` |
| Agent control | L: `control.rs` (socket), `bin/horizon-ctl/`; G: `control.rs`, `agent.rs`, `events.rs`, `ui_control.rs` ([docs/agent-control.md](docs/agent-control.md)) |
| Mods | crate `horizon-mods`; G: `mods.rs`; L: `mod_content.rs`, `mod_list.rs` |
| Horizon extras, settings | L: `extras.rs`, `graphics.rs`, `settings.rs`, `minimap.rs`; G: `extras.rs`, `tweaks.rs`, `options.rs` |

## How `Game` is organised

- `struct Game` (game.rs, ~290 fields) holds everything. Its methods are spread over ~180 files: game.rs keeps
  the core (tick, render, input, inventory, world open/close) and each `game/<feature>.rs` adds an
  `impl Game<'_>` block for one feature, with its own state types. Fields are private to the bin.
- Find a feature's code: `grep -rn "fn tick_<thing>\|fn <verb>_<thing>" crates/horizon-client/src/game*`;
  the file is usually `game/<thing>s.rs`. Who calls it: grep the name in game.rs's `fn tick` / `fn tick_world`.
- Fields that say which mode runs: `remote` (this window is a client of a server: multiplayer, LAN guest, or
  single player on its server thread), `lan` (`LanHost`: this world is open to the network), `integrated`
  (the window's own server thread), `serving` (this `Game` *is* that server thread), `options.dedicated`.
  `fn tick` returns `tick_remote()` when `remote` is set; nothing else of the local simulation runs.
- A server's players run through the same gameplay code as the local player: `as_guest` swaps the guest's
  state (position, inventory, mode, mining, item use) into `Game` for one call (`game/lan_actions.rs`), and
  `levels.rs` swaps whole dimensions in the same way. Changes reach clients through the block log, level
  events and the entity tracker (`lan/tracking.rs`, `game/lan_entities.rs`).
- Shared read-only data: `Resources` (`res`: blocks, items, recipes, mob types, biomes) and `PackResources`
  (`pack`), both in game.rs.

## The two single-player paths

1. **Server thread (default since #1514):** `game/integrated.rs` runs a second `Game` (`serving`) on a
   "Server thread" with one level per dimension; the window joins it as a client through memory (`remote`),
   exactly as a LAN guest or a dedicated server's player does. This is what players run. Open
   `[server-thread]` issues list the gaps left.
2. **In-frame (`--in-frame`, `--no-server-thread` or `HORIZON_SERVER_THREAD=0`): DEPRECATED, never use it, it
   will be removed soon.** The window's `Game` ticks the world itself, one level that follows the player
   (`fn enter_dimension`). Compatibility does not matter: don't add features or fixes for it, don't write new
   tests against it, and move code off it when you touch it. The deterministic rule fixtures still run on it
   until they are ported.

LAN guests and dedicated servers always use the server code, so a fix there serves both. Test the server
thread when you touch player-facing gameplay:

```sh
cargo run --profile fast -p horizon-client -- --resources .local/horizon-client/26.3 --world T               # server thread
python3 tools/lan_e2e.py --check <name>                  # host on its server thread, guest is a server client
python3 tools/parity.py dedicated_server server_entities server_player_damage
```

## Docs worth knowing

[docs/parity-workflow.md](docs/parity-workflow.md) (runner, gate, shards, scopes) ·
[docs/native-client.md](docs/native-client.md) (options, controls, horizon-ctl) ·
[docs/server-levels.md](docs/server-levels.md) (levels per dimension) ·
[docs/rust-parity.md](docs/rust-parity.md) (what is ported) · [docs/known-issues.md](docs/known-issues.md) ·
[docs/release-flow.md](docs/release-flow.md) · [docs/README.md](docs/README.md) (index).
