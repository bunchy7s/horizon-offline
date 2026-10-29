# Horizon documentation

To play, use [the launcher](../launcher/README.md); the project site is [playhorizon.dev](https://playhorizon.dev). These pages are for developers. What is being done now and next, the work board and how to help: [the hub](https://playhorizon.dev/hub) ([ROADMAP.md](../ROADMAP.md), [roadmap/README.md](../roadmap/README.md) for keeping it current).

## Build and run

- [AGENTS.md](../AGENTS.md) and [Architecture](architecture.md): where each part of the code lives, how `Game` is organised, which harness checks which area.
- [Build from source](../README.md#build-from-source): your own copy of the game, resource preparation, `cargo run`.
- [Native client](native-client.md): options, controls, the dedicated server, `horizon-ctl` scripting, packaging, architecture.
- [Host a server](hosting.md): install `horizon-server`, prepare the game files, first run, `server.toml`, the commands, the web panel, backups, services and Docker; what works today and what doesn't yet.
- [Hosting a server: design](server.md): `horizon-server`, the design and the plan: install, `server.toml`, the CLI, the web panel, backups, services, Docker, and the bot benchmark against vanilla.
- [Agent control](agent-control.md): what AI agents can drive in the launcher and the game, and the design of the control socket, `horizon-ctl` and MCP.
- [Graphics](graphics.md): the Simple and Advanced graphics settings, the graphics benchmark and what it measured.
- [Benchmark](benchmark.md): Horizon against vanilla 26.3 and vanilla with performance mods, on the same machine, world, path and settings: what is measured, how it is kept fair, how to reproduce it.
- [Extras](extras.md): optional zoom, minimap, FPS and keystroke overlays and auto-reconnect, all off by default.
- [Shader packs](shaders.md): Horizon's own WGSL shader packs, how to make and check one (`horizon-shader`).
- [Mods](mods.md): Horizon's own mod system (Rust, sandboxed WebAssembly), how to make and install mods.
- [Modpacks](modpacks.md): porting whole packs (ATM10 and smaller): import, inventory, carry-over, an agent per mod with a harness against the Java original, coverage, and how "10x" is measured.
- [Launcher](../launcher/README.md): installs, updates and starts builds; its CLI and MCP server; releasing it.
- [Versioning](versioning.md): how releases are numbered from 0.2 (`v0.2.x`, `launcher@0.2.x`), how installs from before 0.2 get there, and the steps for the day 0.2 ships.
- [Resource packs](resource-packs.md): everything a 26.3 resource pack can change, whether Horizon honours it, and the proof.
- [Opt-in data collection](telemetry.md): the consent, exactly what is sent, how it is stored and sent.
- [Benchmark report format](benchmark-report.md): the versioned JSON every benchmark writes (game, launcher, `tools/perf_bench.py`), the local log and, with consent, the upload.
- [Horizon APIs](api/README.md): every surface other programs build on (mods, shaders, agent control, servers, the launcher, data formats), each with its guide, generated reference, stability and changelog, and the [stability and versioning policy](api/stability.md).
- Menus: every screen is vanilla's (title, world list, create world, multiplayer, options, pause). Horizon's own settings hang off vanilla's Options the way mods add theirs: **Horizon Settings...** and **Mods...** in the last two slots of the Options grid (Horizon Settings opens Graphics, Advanced Graphics, Controller Settings, Extras and Mods, each a vanilla options list), with Horizon Graphics and Advanced Graphics also at the top of Video Settings and Controller Settings at the bottom of Controls. The title screen's and pause menu's Friends buttons open Join a Friend and Play with Friends ([play-with-friends.md](play-with-friends.md)).

## Parity

- [Rust rewrite status](rust-parity.md): completion criteria, how to reproduce the validation, and per-area scope.
- [Parity workflow](parity-workflow.md): `tools/parity.py`, the gate, other machines, merging.
- Area notes: [block behaviours and ticking](parity/blocks.md), [section meshing](parity/meshing.md).
- [Port ledger](port-ledger.md): `tools/port_ledger.py`, every vanilla method next to the Rust functions that port it, both ways, with the gaps where a port isn't wired like vanilla.
- [Version ports](version-ports.md): how a new Minecraft release is detected, diffed against the pinned one and ported, and what stays manual.
- [Decompilation and research lab](research-lab.md) and [decompilation status](decompilation-status.md): the local game inspection workspace the harnesses build on.

## Architecture: world generation

- [World generation](world-generation.md): the pipeline from noise to features.
- [Pregeneration](pregeneration.md): `/pregen`, generating, lighting and saving an area ahead of the players.
- [Noise generation](noise-generation.md), [compiled density functions](density-functions.md), [structure density](structure-density.md), [terrain filling](terrain-filling.md).
- Biomes: [lookup](biome-lookup.md), [temperatures](biome-temperatures.md).
- Surfaces: [material surfaces](material-surfaces.md), [material conditions](material-conditions.md), [rule libraries](material-rule-libraries.md), [vertical anchors](vertical-anchors.md), [value providers](value-providers.md).
- [Carving](carving.md), [heightmaps](heightmaps.md), [feature placement](feature-placement.md).

## Architecture: world data and simulation

- [World data](world-data.md): chunk storage, palettes, region files.
- [Block-state codecs](block-state-codecs.md), [scheduled ticks](scheduled-ticks.md).
- [Collision geometry](collision-geometry.md): shapes, collision, movement and fluids.
- [Face visibility](face-visibility.md): which model faces the mesher emits.

## Java comparison profiling

- [JFR analysis for Java comparisons](profiling.md).
