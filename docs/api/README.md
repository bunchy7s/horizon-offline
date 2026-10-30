# Horizon APIs

Horizon has six surfaces that other programs and people build on. Each one has an **API** (the contract: what you
may call, read or write, and what Horizon promises about it) and an **SDK** (the tools that help you use it). This
section has one page per surface, plus the rules every surface follows:
[stability and versioning](stability.md).

| Surface | For | API today | Stability | Reference |
| --- | --- | --- | --- | --- |
| [Mods](../mods.md) | mod authors | content mods: manifest `api` 0.2; native mods: a horizon-mixins bundle ([native-mods.md](../native-mods.md)) | preview (content), experimental (native) | [mods.md](../mods.md) |
| [Shaders](shaders.md) | shader authors | none yet: packs were removed on 2026-10-24 | planned | none |
| [Agent control](agent-control.md) | agents, scripts, tests | `horizon-ctl`, its MCP server, the game's control socket | preview | [horizon-ctl-commands.md](../horizon-ctl-commands.md) (generated) |
| [Servers](server.md) | hosts, panels, bots | `horizon-server` commands, `server.toml`, control API `/api/v1` | stable | [server-reference.md](server-reference.md) (generated) |
| [Launcher](launcher.md) | players' agents, pack makers | `horizon-launcher` commands and its MCP server | preview | [launcher-commands.md](../launcher-commands.md) (generated) |
| [Data formats](data-formats.md) | tools that read Horizon's files | benchmark reports, telemetry, settings files, signed checksums | stable or preview, per format | on that page |

## How these APIs are designed

- **From the people who use them.** Each API starts from what a mod author, a pack author, a server host or an agent
  needs to do, written down as tasks ("a host backs up the world from a script"), and from how Horizon is built:
  Rust, wgpu and a server thread that runs every world. A function exists because a task
  needs it.
- **Our own design.** Horizon's APIs are not modelled on any other project's. Contributors may know other mod
  loaders, renderers and server platforms exist, but their source and API references are never read, copied or
  paraphrased here, and our names, shapes and docs are not taken from theirs. Vanilla Minecraft 26.3 is the reference for game
  concepts: ids, commands, `server.properties`, data packs and resource packs.
- **Safe by default.** A content mod is data and runs no code; a native mod runs with the player's rights, so it is
  off by default, behind a warning and reviewed before the registry lists it; the control
  sockets listen on this computer only, behind a token; nothing here is on by default where it would change vanilla's
  look or behaviour.
- **Free when unused.** An API nobody uses costs nothing per tick or per frame: no engine, no thread, no allocation.
- **One contract for every mode.** In-frame single player, the server thread, LAN and the dedicated server run the
  same gameplay code ([AGENTS.md](../../AGENTS.md)), so a server-side API serves all of them.

## References are generated

A reference page is written by code from the source it describes, and a test fails when the two differ:

| Page | Generated from | Update it with |
| --- | --- | --- |
| [server-reference.md](server-reference.md) | `horizon-server`'s help, `config::TEMPLATE`, `api::ROUTES` | `HORIZON_WRITE_DOCS=1 cargo test -p horizon-server reference` |
| [horizon-ctl-commands.md](../horizon-ctl-commands.md) | the command table in `crates/horizon-client/src/bin/horizon-ctl/commands.rs` | `horizon-ctl help --markdown > docs/horizon-ctl-commands.md` |
| [launcher-commands.md](../launcher-commands.md) | `launcher/src/cli.rs` | `HORIZON_WRITE_DOCS=1 cargo test --manifest-path launcher/Cargo.toml the_launcher_reference` |

So a pull request that changes an API also changes its reference, and the reviewer sees the API's diff in the
reference's diff. The guides (overview, how-to, examples) are written by hand.

## Each surface's page

Every page has the same parts: **Overview** (what you can build), **Guide** (where to start), **Reference**,
**Examples**, **Stability** (the level of each part, per [stability.md](stability.md)), **Changelog** and
**Not there yet** (what is planned, so nobody builds on a guess).

The website shows these pages at [playhorizon.dev/docs/api](https://playhorizon.dev/docs/api), built from this
folder, and a summary for players and authors at [playhorizon.dev/docs/apis](https://playhorizon.dev/docs/apis)
(in the website's repository, horizon-rust/horizon-site).
