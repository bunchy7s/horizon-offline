# Modpacks: porting any pack to Horizon

Goal (Ludovic, 2026-10-16): **support modpacks like All the Mods 10 far better than Minecraft
does, and be able to port any of them.** "Better" is measured: pack load time, RAM, FPS, TPS
and world-generation speed against the same pack on NeoForge or Fabric. "Any of them" means a
pipeline, not a list of hand ports. It takes a pack as players download it and produces native
Horizon mods. The pipeline is AI agents in dependency order, each port checked against the
original mod running in a dev-only Java instance.

This page is the design. It builds on [mods.md](mods.md) (the mod API, WASM in wasmtime, and
why Java jars cannot run in Horizon, so we port instead). The ATM10 inventory behind the numbers below is
[modpacks/atm10-mods.csv](modpacks/atm10-mods.csv).

## In one picture

```
 pack (.mrpack / CurseForge zip or instance / Prism instance)
   │  import: normalise to pack.json (MC version, loader, files, hashes, overrides)
   ▼
 inventory: every jar's metadata (mods.toml / fabric.mod.json, licence, deps, file census)
   │  classify: content · client-ui · library · performance · server-only · integration · other
   │  tier:     data · logic · machines · deep · client · none
   ▼
 dependency order (libraries first, then the mods that need them)
   │
   ├─ carry-over (mechanical, no agent): assets/ + data/ upgraded to 26.3 formats,
   │    block and item registrations from the oracle's facts export  →  <mod>.hmod
   ├─ port (one agent per mod, tiers logic and up): Rust → WASM against the Horizon
   │    mod API, iterating until its harness passes                  →  <mod>.hmod
   ├─ harness per mod: the Java mod in a dev-only NeoForge/Fabric oracle vs the port
   ▼
 coverage report: ported · partial · not yet · not needed, per mod and per class
```

Everything runs on the player's (or server owner's) machine, on jars they downloaded. The
results stay there. See [Licences](#licences).

## All the Mods 10, measured

**Source.** ATM10 is on CurseForge. Its GitHub repository (AllTheMods/ATM-10) carries the pack's
own mod list, `config/crash_assistant/modlist.json`, written by the Crash Assistant mod in the
pack. We used the version committed on 2026-10-15. It lists jar names, mod ids, versions,
CurseForge fingerprints and Modrinth SHA-1s. ATM10 8.x runs **Minecraft 1.21.1 on NeoForge
21.1.259**. The list has **495 jars** (494 mod ids: CC: Tweaked ships two jars) plus one
datapack (Universal Sawmill's).

**Metadata only.** Modrinth's `version_files` endpoint resolved 321 mods by hash, with their
categories, client/server sides, licence and required dependencies. A Modrinth name search
found 70 more CurseForge-only mods with a listing. The remaining 104 are CurseForge-only, so
CurseForge's API (which needs a key) would be the only source. They were classified from their
names and what they are known to do. 20 of those calls are marked `guess` in the CSV. No jar
was downloaded.

### Classes

| Class | What it is | Port? | ATM10 |
|---|---|---|---:|
| **content** | adds or changes gameplay: blocks, items, machines, mobs, worldgen, magic | yes: carry-over or agent | **287** |
| **library** | APIs and code shared by other mods (Architectury, Curios, GeckoLib, Kotlin for Forge, Balm...) | no: maps to the Horizon API or the SDK | **80** |
| **integration** | glue between two other mods (JEI plugins, AE2 ↔ Mekanism, Create add-on wiring) | after both sides are ported | **44** |
| **client-ui** | client-only screens, HUDs, tooltips, maps, recipe viewers | yes: client API, or built in | **32** |
| **other** | pack tooling and engine features: KubeJS, Almost Unified, shader mods, CTM, crash/version tools | no: folded into the pipeline or the engine | **25** |
| **performance** | Sodium, FerriteCore, ModernFix, ImmediatelyFast, spark, AI and recipe speed-ups | no: Horizon is the replacement | **16** |
| **server-only** | admin and server features: ranks, essentials, backups, claims, spawn control | yes: server API | **11** |
| | | | **495** |

### Tiers (the API a port needs)

| Tier | Needs | ATM10 | of which content |
|---|---|---:|---:|
| **data** | registries (blocks, items) + carried-over data and assets; no code | 68 | 56 |
| **logic** | + behaviour callbacks (use, place, break, ticks), server events, config | 72 | 55 |
| **machines** | + block entities, menus and screens, capabilities (energy, items, fluids), mod networking | 101 | 89 |
| **deep** | + entities and AI, custom renderers and animated models, dimensions, new vanilla hooks (what Mixins do) | 91 | 87 |
| **client** | the client interfaces (`keys`, `target`, `inventory-read`, `ui`, `render`) | 38 | 0 |
| **none** | nothing to port | 125 | 0 |

Class by tier:

| | data | logic | machines | deep | client | none |
|---|---:|---:|---:|---:|---:|---:|
| content | 56 | 55 | 89 | 87 | | |
| library | | | | | | 80 |
| integration | 11 | 7 | 12 | 3 | 8 | 3 |
| client-ui | | | | | 30 | 2 |
| other | 1 | | | | | 24 |
| performance | | | | | | 16 |
| server-only | | 10 | | 1 | | |

What this says:

- **A quarter of ATM10 needs no port at all** (125: libraries, performance mods, pack tooling).
  Horizon's renderer and simulation replace the performance mods. Libraries become API
  features: Curios becomes accessory slots, GeckoLib an animated-model format, Patchouli,
  Modonomicon and GuideME a guide-book reader, the config libraries a config schema.
- **68 mods are data tier.** They are the structure packs (YUNG's, Moog's, Formations, CTOV,
  Towns and Towers, When Dungeons Arise), the decoration blocks (Macaw's doors, fences, paths,
  roofs, stairs; Chipped; XTones; Factory Blocks) and the ore and compression sets
  (AllTheOres, AllTheCompressed). They come over **without an agent** once registries and
  worldgen carry-over exist.
- **The big systems are deep tier.** Create, AE2, Mekanism, Ars Nouveau, Twilight Forest,
  MineColonies and Applied/Refined Storage are a long campaign of agent ports. They reach the
  most dependents: AE2 has 9 required dependents in the pack, Create 7, Ars Nouveau 6 and
  Mekanism 5.
- **Dependency depth is shallow.** Of the 321 mods with Modrinth data, 188 need nothing else
  in the pack, 103 sit one layer up, 29 two layers up and one (Apotheosis) three. The most
  required libraries are YUNG's API (10 dependents), GeckoLib, Resourceful Lib, SuperMartijn's
  core (8, 8, 7), then Curios, Bookshelf, Prickle and McJtyLib (6 each).

### Licences in ATM10

| Class | permissive (MIT, Apache, BSD, CC0, CC-BY) | copyleft (GPL, LGPL, MPL, AGPL) | All Rights Reserved | custom | non-commercial / no-derivatives | unknown (CurseForge only) |
|---|---:|---:|---:|---:|---:|---:|
| content | 93 | 46 | 62 | 16 | 9 | 61 |
| library | 27 | 20 | 9 | 6 | 4 | 14 |
| integration | 16 | 8 | 9 | 0 | 0 | 11 |
| client-ui | 13 | 5 | 3 | 5 | 1 | 5 |
| other | 6 | 8 | 4 | 1 | 0 | 6 |
| performance | 6 | 5 | 1 | 1 | 0 | 3 |
| server-only | 6 | 0 | 1 | 0 | 0 | 4 |
| **all** | **167** | **92** | **89** | **29** | **14** | **104** |

The "unknown" column is a gap in our metadata, not in the mods: NeoForge's `mods.toml` has a
mandatory `license` field, and the inventory reads it from each jar at import.

## Licences

Mods have their own licences: many are All Rights Reserved, others MIT, LGPL or MPL. A port is a
derivative work. These rules are part of the pipeline, not advice to users:

1. **Local by default.** The pipeline runs on the player's or server owner's machine, on jars
   they downloaded themselves. The ports it makes stay there. This works like `prepare` with
   Minecraft's own files: nothing is redistributed.
2. **The licence is recorded per mod** in the inventory (`license` from `mods.toml` or
   `fabric.mod.json`, plus Modrinth's SPDX id where one exists) and travels with the port into
   its `horizon-mod.toml` (`ported-from = { id, version, license, url }`).
3. **Publishing needs permission.** A port can go into a mod index, the launcher browser or a
   server's download list only if its licence allows redistributing derivatives (permissive,
   or copyleft with the port under the same licence and its source published) or the author
   has given written permission, recorded in the index entry. The tooling refuses to package
   anything else. No-derivatives and non-commercial licences (14 in ATM10) are never published.
   For them, even a local port gets a warning that the licence may not allow it.
4. **Mod code never enters this repository, and nobody reads it.** Mod jars, Java oracle builds
   with mod code, and ports of mods we have no permission to publish all live outside the
   repo: in the user's pipeline work directory, or for our own development in
   `~/code/horizon-parity/modports/` (dev-only, never pushed). A mod's code is never
   decompiled, read, quoted or paraphrased, by people or agents. Ports are written only from
   the mod's observed behaviour (the oracle and the harness), its own data files and its
   public documentation. The repository holds the pipeline, the API, metadata (ids, names, versions, licences, counts)
   and test fixtures we write ourselves.
5. **Contacting authors or publishing any port is Ludovic's call.** Agents ask first.

This is engineering policy, not legal advice. Before anything is published, Ludovic decides,
with advice if needed.

## The mod API: what packs need, against API 0.1

API 0.1 is client-side only: `log`, `game` (player and clock), `chat`, `hud`, plus tick,
frame, chat and command hooks. Server mods do not run yet. What a content pack needs:

| Feature | API 0.1 | Needed by (ATM10 content+integration, by tier) | Planned in | Shape |
|---|---|---|---|---|
| Block registry: ids, states, properties, shapes, sounds, light | — | data and up (all 331) | **0.2** | declared in `data/<mod>/horizon/block/*.json`, each from a vanilla *template* block (below) |
| Item registry, block items | — | data and up | **0.2** | `data/<mod>/horizon/item/*.json`, from a vanilla template item |
| Recipes, loot tables, tags, advancements, lang, blockstates, models, textures, sounds in the mod's namespace | vanilla namespace only | data and up | **0.2** | the 26.3 data-pack and resource-pack formats, as they are |
| Worldgen JSON: features, structures, template pools, processors, structure NBT, biome additions | vanilla bundle only | about 30 data-tier structure and biome mods | 0.2.x | carried into the `worldgen/` bundle |
| Block and item behaviour callbacks: use, place, break, attack, random tick, scheduled tick, neighbour change, entity inside | — | logic and up (253) | **0.3** | `server` interface; hooks per registered id, budgeted |
| Server events, typed commands, game rules | — | logic and up, server-only (11) | **0.3** | `server` interface |
| Config: typed schema in the manifest, per world and per server, a settings page | — | most mods with code | **0.3** | `config` + the Options > Mods page |
| Block entities: per-block NBT state, ticking, client sync | — | machines and up (191) | **0.4** | `block-entity` interface, ticks batched per mod |
| Menus and screens: slots, progress bars, buttons, fluid and energy gauges | — | machines and up | **0.4** | a declared screen layout the game draws (`ui`), with server-side menu logic in the mod |
| Capabilities: energy (FE), item, fluid handlers on blocks and items | — | machines and up | **0.4** | `capabilities` interface; FE as the common unit, others as mod-defined resources |
| Data attachments on entities, chunks, levels | — | machines and up | **0.4** | `storage` per entity/chunk/level |
| Accessory slots (what Curios does) | — | Curios users (6 require it, more use it optionally) | **0.4** | built-in slots API |
| Mod networking between its client and server halves | — | machines and up | **0.4** | `net` interface (already planned in mods.md) |
| Custom recipe types: machine recipes with their own JSON | — | machines and up | **0.4** | recipe-type registry: the mod declares its JSON schema and a display layout for the recipe viewer |
| Entities: types, attributes, AI goals and brains, spawning | — | deep (87) | 0.5 | `entity` interface, AI in the mod with vanilla goals reusable |
| Custom renderers and animated models (block-entity, entity, item; GeckoLib's format) | — | deep | 0.5 | data models (Bedrock geometry + animations), plus a budgeted draw-list interface for the rest |
| Dimensions: dimension types, levels, chunk generators | — | about 10 (Twilight Forest, Aether, Ad Astra, Undergarden, ...) | 0.5 | JSON part with worldgen; custom generators as mod code |
| New hooks into vanilla behaviour (what Mixins do) | — | deep | as asked | each becomes a named hook, added for everyone |
| Client interfaces: `keys`, `target`, `inventory-read`, `ui`, `render`, `world-read` | planned | client tier (38) | 0.2–0.4 | as in [mods.md](mods.md#planned-interfaces) |

Two rules from mods.md stay: **the sandbox** (no files, sockets or processes; memory cap; fuel
per call; a crashing mod stops alone) and **free when unused** (no mods means vanilla code
paths and the vanilla resource root, so the parity harnesses test the same game).

### Registries 0.2: data-driven blocks and items

A `.hmod` may now carry `assets/` and `data/` beside (or instead of) `mod.wasm`. A content-only
mod needs no code:

```
ruby.hmod
├── horizon-mod.toml          id = "ruby", api = "0.2", side = "both", capabilities = ["registries"]
├── data/ruby/horizon/block/ruby_block.json
├── data/ruby/horizon/item/ruby.json
├── data/ruby/recipe/ruby_block.json            (26.3 data-pack formats)
├── data/ruby/loot_table/blocks/ruby_block.json
├── data/minecraft/tags/block/mineable/pickaxe.json   (adds to a vanilla tag)
├── assets/ruby/blockstates/ruby_block.json     (26.3 resource-pack formats)
├── assets/ruby/models/block/ruby_block.json
├── assets/ruby/items/ruby.json
├── assets/ruby/textures/block/ruby_block.png
└── assets/ruby/lang/en_us.json
```

A block declaration names a **template**, a vanilla block whose behaviour class, state
properties and shapes the new block takes, and overrides what differs:

```json
{
  "template": "minecraft:iron_block",
  "properties": { "hardness": 5.0, "explosion_resistance": 6.0, "sound": "minecraft:metal",
                  "map_color": 33, "light": 0, "requires_tool": true },
  "item": true
}
```

Most decoration and resource blocks are vanilla classes with other numbers: Macaw's doors are
`DoorBlock`s, XTones are full cubes, ore sets are `DropExperienceBlock`s. So templates cover the
data tier without code. Templates also give mod blocks every behaviour Horizon already
implements and the harnesses already check. Items work the same way (`"template":
"minecraft:diamond"`, overriding `max_stack_size`, `rarity` and any data component in
`components`).

How the game loads them, without touching vanilla's path:

- Horizon reads everything from one resource root. Resource packs already work as an
  **overlay root** (`packs::startup_root`) with vanilla's files linked and the overridden ones
  rewritten. Content mods add one more layer: the enabled mods' `assets/` and `data/`, and the
  prepared per-state tables (`facts.json`, `gameplay-facts.json`, `block-behaviours.json`, the
  collision, occlusion and movement tables, `model-states.json`, `render-definitions.json`,
  `items.json`, ...). Each table gets the mod blocks' rows appended, copied from the template's
  rows with the declared overrides. The loaders then read one tree and need no mod-specific
  code.
- **Vanilla state ids do not move.** Mod states are appended after vanilla's, mods in id order,
  so the same mod set gives the same ids. Saves store block names, so ids never reach the disk.
- Loaders that read only `data/minecraft/` (recipes, loot, tags) read every namespace, as
  vanilla does. With no mods there is only `minecraft`, so nothing changes.
- **Namespaces.** A mod's files go under its id, and under the namespaces its manifest lists
  when its Java original ships them so (`namespaces = ["kaisyn", "towns_and_towers"]` for Towns
  and Towers, mod id `t_and_t`; docs/mods.md). No namespace has two owners: a mod whose id or
  listed namespace another mod that is on already owns is left out, with both named in the log.
  `horizon-port-pack` avoids that inside a pack: a namespace two carried mods both claim (YUNG's
  structure mods all ship files under `yungsapi`, the namespace of YUNG's API, whose own jar isn't
  carried) is left to none of them, and its files stay outside. The launcher applies the game's
  rule before it starts a pack (`pipeline::loadable`) and lists what it left off.
- The overlay is built on the first start with a set of mods and kept for later starts. A stamp
  (`horizon-mod-content.stamp`) hashes the game executable, the base tables and every mod file's
  size and mtime. Adding, removing or editing a mod, or a new build of Horizon, rebuilds it. For
  ATM10's 40 data-tier mods the build takes about a minute of the first start on a busy machine.
- **Worldgen.** Mods' structures, structure sets, template pools, processor lists, configured and
  placed features join the `worldgen/*.json` bundles. Their structure templates go under
  `worldgen/structure/<ns>/`. Their biome tags are merged into `worldgen/biome_tags.json`, minus
  the biomes and tags Horizon does not have. Mod biomes, noise and density functions are not
  carried yet.
- **Limits.** Chunk sections size their global palette to the registry: 16 bits for vanilla's
  35,723 states (the same chunks, saves and packets as before), one more bit each time content
  mods double it, up to 2^20 states (1,048,576). Past that the content root is refused with the
  states per mod. ATM10's 42 carried mods take the registry to 193,762 states (18 bits). On the
  network the width is the registry's: unchanged for vanilla clients as long as no content mod
  is on, wider only between Horizons that share the same mods; a Horizon with content mods still
  reads a vanilla server's 16-bit chunks. Block sprites fill texture arrays of 2048 layers, eight
  at most (16,384; vanilla takes 1,250, identical textures share a layer; ATM10's 42 take 9,790).
- **Broken mod files load as vanilla loads them.** A blockstate definition that does not bind
  (a multipart value the block does not have), a block model that is missing or does not parse,
  and a recipe that does not parse are logged and skipped: the block gets no model or the
  missing model, the recipe is absent, everything else loads. Vanilla's own files never hit
  these paths.
- Not in 0.2: multiplayer registry sync (single player and the integrated server first), worlds
  opened without a mod they used (vanilla turns unknown blocks into air; we will ask first),
  vanilla ids redefined by a mod (refused).

## The pipeline: `horizon-port-pack`

A Rust tool in this repo (`crates/horizon-port-pack`, a CLI the launcher also calls), modelled
on the version-port pipeline (`tools/version_port.py`: detect, diff, regen, harnesses, report,
publish). Each step reads and writes files in a work directory, so it can be rerun, resumed and
inspected:

```
<work>/                       ~/.horizon/ports/<pack-slug>/ for players; ~/code/horizon-parity/modports/ for us
├── pack.json                 normalised import
├── inventory.json            per mod: metadata, licence, census, class, tier, deps, layer
├── jars/                     the user's own jars (copied or linked from their instance), never uploaded
├── mods/<id>/
│   ├── carry/                assets/ + data/ upgraded to 26.3
│   ├── oracle/               facts, registries and traces exported from the Java oracle
│   ├── port/                 the agent's Rust crate (tiers logic and up)
│   ├── harness/              per-mod harness results and logs
│   └── <id>.hmod
└── coverage.json, coverage.md
```

**Status.** `import`, `inventory`, `fetch`, `carry` and `report` run today, and `port` runs
them all for what a player's machine can port (content mods of the data tier), as the
launcher's "Import modpack" does:

```
cargo run -p horizon-port-pack -- import ~/Downloads/pack.mrpack ~/.horizon/ports/pack
cargo run -p horizon-port-pack -- inventory ~/.horizon/ports/pack     # --offline reuses modrinth.json
cargo run -p horizon-port-pack -- fetch ~/.horizon/ports/pack         # the portable mods' jars, or name them
#   --from <folder>: copy the player's own jars (an installed instance, a downloads folder)
cargo run -p horizon-port-pack -- carry ~/.horizon/ports/pack         # content mods with a jar, or name them
#   --resources <prepared root>: only templates Horizon's tables have
cargo run -p horizon-port-pack -- report ~/.horizon/ports/pack        # coverage.json and coverage.md
cargo run -p horizon-port-pack -- port ~/Downloads/pack.zip ~/.horizon/ports/pack [--from <folder>] [--resources <root>]
```

`check <jar> <out>` takes one mod jar on its own, as the launcher's mod browser installs it
([mod-browser.md](mod-browser.md)): it carries the jar into `<out>/carry/` and reads its class
files (constant pool, superclass, declared methods; no decompiling) for what the carry can't
take, then writes the verdict to `<out>/check.json`. **Works**: content only, and the code just
registers it. **Partly**: the content ports, and the code features listed don't (block entities,
menus, entities, blocks and items with behaviour of their own, recipe types, Mixins, game events).
**Doesn't run**: nothing to port, or a jar made for 1.20 or older. Each mod class is followed up to
the first game class it extends; a block subclass that only overrides how it looks or is placed
(`getStateForPlacement`, `rotate`...) is registration. Game classes are read by Mojang's names
(26.1 and later, and NeoForge since 1.20.5); older Fabric jars name them in intermediary, so a
class extending one counts as code that can't be read. On ten 26.3 jars (2026-10-22): Macaw's
Fences, Windows and Furniture, Ecologics, Compressed Blocks, More Decorative Blocks and Moog's
Nether Structures are Partly; Sodium and GeckoLib don't run (Mixins, renderers, nothing to port);
Fabric API doesn't run (its id has a dash, which content mods can't use). A check takes 0.2 s to
3 s, and 24 s for Compressed Blocks (18 MB, 9,360 recipes, writing the carry dominates).

`fetch` gets each jar the player's way, never from us: copied from a folder they have (matched
by SHA-1, or through a CurseForge app instance's `minecraftinstance.json`), else downloaded from
the link the pack itself gives (a `.mrpack`'s, on the format's hosts only) or, for a CurseForge
pack, the same file on Modrinth's public CDN, found by its SHA-1. A jar is kept only when its
hash matches. A file only CurseForge has is listed as "CurseForge downloads coming soon" (its API
needs a paid key; Ludovic, 2026-10-22), with the player's installed instance as the way round it.
`<work>/fetch.json` says what happened to each file. On All the Mods 10 8.3's CurseForge zip
(2026-10-17): of the 54 data-tier content mods, 42 come from Modrinth's CDN (84 MB) and 12 need
the key or the instance; `port` then carries the 42 over (0 ported, 42 partial: the harness is a
dev step).

`inventory` reads metadata only: Modrinth answers by file hash, and the jars only if the user's
instance has them (an imported instance folder, or jars put in `<work>/jars/`). Without jars,
classes come from the reviewed table, Modrinth categories and sides; with jars, from their
`mods.toml` / `fabric.mod.json` and file census too. On Create+ 6.0.0 alpha f (155 files, metadata
only): content 54, library 33, client-ui 26, performance 25, other 14, unknown 2, server-only 1;
40 rows matched the reviewed ATM10 table; three dependency layers (93 / 57 / 5).

`carry` copies a jar's `assets/<id>/`, `data/<id>/`, `data/minecraft/tags/` and `data/c/tags/`
into `mods/<id>/carry/`, a folder content mod (API 0.2) the game loads with `--dev-mod`, and
keeps what a content mod may not hold (other namespaces, vanilla data) in `outside/`. Files of
the mod's own content that the game's content loader would refuse whole also go to `outside/`,
listed with the reason in the report's `unreadable`: paths vanilla does not read (upper case),
and JSON that does not parse. Vanilla skips them one by one. A vanilla or convention tag
with `"replace": true` is carried as it is: the game honours it as a data pack's, with a warning
(docs/mods.md); Towns and Towers' seven `minecraft:has_structure/*` biome tags are such.
A byte-order mark is dropped. For ATM10's 42 mods that is 6 files in 4 mods, and the 42 load as
carried. No class file is copied. It upgrades from 1.21.1 to 26.3: vanilla-type recipes (ingredients), loot tables
(`conditions` lists become one `condition`, `functions` become `modifier`, kinds move to `type`,
tool matches on silk touch or shears become `minecraft:tool/can_silk_touch` and
`minecraft:tool/can_shear`, and number providers written as a bare `{"min", "max"}` get
`"type": "minecraft:uniform"`), and the `items/` definitions 1.21.4 introduced. 1.21.1 read such
a bare range as uniform and 26.3 refuses it: 304 of ATM10's carried tables have them (rolls,
counts, enchantment levels), and before the carry typed them their chests came out empty or short.

Blocks and items come from the NeoForge oracle when `<work>/oracle/facts.json` exists (step 6):
every block the mod registers gets the vanilla template of the same vanilla class and states
whose shapes, sounds, set type and wood type match best, with the mod's own hardness, blast
resistance, light, tool need, friction, map colour and sounds as overrides. `--resources
<prepared>` limits templates to blocks Horizon's tables have. `carry-report.json` lists, per
block, what the template does not reproduce (other shapes, sounds no vanilla block has, a class
of its own), and the blocks left out: those whose vanilla class has no vanilla block with the
same states (Macaw's garage doors, multi-part gates). Recipes get the convention tags they use
(`c:dyes/red`), filled from the oracle with vanilla's and the mod's own members. Without the
oracle, templates are guessed from the blockstate's properties: the template whose properties
include them with the fewest left over (models often ignore `powered` or `waterlogged`), the
block's name breaking ties (`_pane`, `_gate`, `_trapdoor`). Walls and vines have the same
property names; sides tested as `true` / `false` make a vine (Chipped's 11 vines, which the game
refused as walls: their blockstates did not load).

### 1. Import

| Format | Read | Jars come from |
|---|---|---|
| Modrinth `.mrpack` | `modrinth.index.json`: `dependencies` (minecraft, neoforge / fabric-loader / forge / quilt-loader), `files[]` with path, sha1/sha512, `env` (client/server), download URLs; `overrides/`, `client-overrides/`, `server-overrides/` | downloaded from the listed Modrinth URLs, hash-checked |
| CurseForge zip | `manifest.json`: `minecraft.version`, `modLoaders[]`, `files[]` (`projectID`, `fileID`, `required`), `overrides`. When the overrides hold Crash Assistant's `config/crash_assistant/modlist.json` (All the Mods 10 does), its jar names, mod ids and SHA-1s replace the bare ids | the user's installed instance (CurseForge app `minecraftinstance.json` + `mods/`), or the same file on Modrinth by SHA-1. CurseForge's API needs a key, so we take what the user already downloaded |
| Prism / MultiMC instance | `mmc-pack.json` components (`net.minecraft`, `net.neoforged`, `net.fabricmc.fabric-loader`), `instance.cfg`, `.minecraft/mods/*.jar`, packwiz `.pw.toml` files if present | the instance folder |
| packwiz | `pack.toml`, `index.toml`, `mods/*.pw.toml` | as listed |
| Crash Assistant `modlist.json` | jar name → mod id, name, version, Modrinth SHA-1; the loader entry (NeoForge `21.1.x` is Minecraft 1.21.1). Packs such as ATM10 publish it in their repository | metadata only; jars put in `<work>/jars/` |

`pack.json` holds the Minecraft version, the loader and its version, each file with its hashes
and side, and the overrides (configs, KubeJS scripts, datapacks).

### 2. Inventory and classification

Per jar, without decompiling:

- **Metadata:** `META-INF/neoforge.mods.toml` / `META-INF/mods.toml` / `fabric.mod.json`: id,
  version, `license`, authors, URLs, dependencies with ordering and side, mixin configs, access
  transformers.
- **Census of files:** blockstates (≈ blocks), item models (≈ items), recipes, loot tables,
  tags, worldgen files, structure NBTs, languages, sounds, textures; the number of mixins and
  the vanilla classes they target; whether the jar has code beyond its data.
- **Census of classes:** the superclass names in the class files (constant pool only): how
  many block entities, menus, screens, entities, renderers, recipe serializers, payload types.
- **Known mods:** a reviewed table in the repo (id → class, tier, Horizon equivalent: "sodium:
  performance, built in", "jei: client-ui, recipe viewer built in") wins over heuristics. It
  starts with ATM10's 495 rows.

Class rules, in order: the known-mods table; client-only side → client-ui; server-only side →
server-only; mixins only, into rendering or ticking, no content → performance; no content and
depended on by others → library; depends on two or more content mods and adds little →
integration; otherwise content. Tier rules: content with no code beyond registration →
data; behaviour callbacks only → logic; block entities or menus → machines; entities,
renderers, dimensions or mixins into gameplay → deep.

### 3. Dependency order

A topological order over required dependencies, libraries first. Each mod gets a layer (0:
needs nothing in the pack). A mod is ported only after its dependencies are ported or mapped
to the API. A library mapped to the API ("Curios → accessory slots") counts as done when that
API feature exists. Cycles (rare: optional mutual integrations) are broken at optional edges.

### 4. Carry-over: data and assets

Mechanical, the same for every mod, and no agent:

1. Extract `assets/` and `data/` from the jar into `mods/<id>/carry/`.
2. **Upgrade to 26.3.** ATM10 is 1.21.1 and Horizon is 26.3. Item models moved to `items/`
   (1.21.4), recipe ingredients and results changed shape (1.21.2), some definitions moved to
   files of their own, and some registries and tags were renamed. The upgrader is a table of these changes generated from
   vanilla's own datafixers and pack-format notes, applied per file. A file it cannot upgrade
   is listed, not guessed.
3. **Registrations.** Block and item ids registered in Java code are read from the oracle
   (step 6): its facts exporter writes each mod block state's hardness, sounds, shapes, light,
   map colour and class chain. These are the same exporters that produce Horizon's vanilla
   `gameplay-facts.json`, run on the modded game. The class chain picks the template (a
   `DoorBlock` → a vanilla door). Without an oracle, the blockstate's property set gives the
   template (`facing, half, hinge, open, powered` is a door), with the template's numbers.
4. **The pack's own data.** KubeJS scripts, Almost Unified and config-driven recipes change
   recipes, tags and loot after load. We do not translate scripts. The oracle exports the
   **final** recipe, tag and loot registries of the whole pack once, and the pack layer
   carries those over.
5. Write `<id>.hmod`. For a data-tier mod, it is now ported.

Two cases the oracle fills in:

- **Loot registered in code.** Chipped ships 7 loot tables for about 7,000 blocks; the rest
  drop themselves through Java code. When a declared block has no table in the jar and the
  oracle shows vanilla's drop-self, carry writes that table in 26.3's shape. When only the
  states with one property value drop it (a door's lower half), the entry gets vanilla's
  `match_block` condition. Chipped: 6,843 tables added.
- **Worldgen carry-over.** Structure libraries give vanilla's types other names. Carry maps
  these to vanilla's:
  - `lithostitched:jigsaw`, MoogsStructureLib's and Repurposed Structures' generic jigsaws
    become `minecraft:jigsaw`;
  - their `advanced_random_spread` becomes `minecraft:random_spread`;
  - their versioned, legacy and empty pool elements become vanilla's.

  Three more rewrites make the mapped structures decode and start where their mod would:
  - **Block states.** 1.21.1 writes a block or fluid state as `{"Name", "Properties"}`, 26.3
    as `{"id", "properties"}` and refuses the old shape, which failed the whole processor
    list, feature or pool. Carry rewrites them (#816; 1,005 files in 17 of ATM10's mods).
  - **Start and placement rules** (#820 in the game, #821 in carry). Repurposed Structures'
    and Moog's jigsaws refuse to start in liquid and check biomes and terrain height in a
    radius; their spreads keep a minimum distance from the world origin and away from other
    sets (named by a structure set tag). Carry writes these under `horizon:start_checks`,
    `horizon:min_distance_from_world_origin` and `horizon:exclusion_zone`; the game runs them
    (`horizon-world`'s `structure/start_checks.rs`). Vanilla data has none of the keys. Moog's
    also spreads its structures 1.65 times as far apart as its files say, which carry now
    applies. MVS went from 829 starts to 90 in a 128×128-chunk square (seed 1234): 294 from the
    spacing, the rest from the start checks.
  - **Mo' Structures** (LGPL-3.0; 38 files). Its `mostructures:generic` is vanilla's jigsaw
    that starts only where the terrain under the 5×5 chunks around it varies by no more than
    its `heightRange`: carry writes `minecraft:jigsaw` with `horizon:start_checks`
    (`terrain_radius` 2, `terrain_range`). Its placement `mostructures:type` takes spacing and
    separation from the mod's config, not its files; carry writes the config's defaults
    (keyed by `config_key`) and the sets it avoids within 3 chunks as `horizon:exclusion_zone`
    with `"their_zones": false`: like the mod, the game tests those sets by their spread and
    frequency alone, not by their own exclusion zones. Its two block-ageing processors
    (`jungle_rot_processor`, `simple_cobblestone`, 3 uses) are still dropped, as is its
    mixin that keeps lakes out of its structures. Over chunks -64..63 that adds 13 starts at
    seed 1234 (pirate ships, churches, a tavern, the castle in the sky...) and 16 at seed 2,
    and every other start stays as it was.
  - **Processors written in Java** are taken out of their lists (#822). The game, like
    vanilla, refuses a list holding a type it does not know, and with it every pool naming the
    list. The structures now generate undecorated (no pillars under them, spawners as the
    template has them); 1,427 processors of 146 types in ATM10's mods, the most common
    `lithostitched:block_swap`, Formations' `biome_replacements` and Repurposed Structures'
    pillar, fluid and void processors.
  - **Spawner processors** carried as data (`horizon-port-pack/src/processors.rs`). Most
    chests in the mods' templates already name their loot table; what the Java processors add
    is the spawners' mobs. Moog's and Repurposed Structures' spawner randomizing processors
    become `horizon:spawner_mobs` (a weighted mob per spawner, picked with the placement's
    random as theirs do, and the spawner NBT they write; Repurposed's `rs_spawners` lists are
    written in). YUNG's Better Dungeons and Better Desert Temples turn a spawner or a
    placeholder block (gravel, bone block, yellow wool...) into a spawner of a fixed mob: that
    is vanilla's `rule` processor with an `append_static` block entity. Moog's vanilla loot
    swap acts only when a "replace vanilla" preset is on, and they ship off. Moog's trial
    spawner and vault processors keep the block and change its block entity: one `rule` per
    block state, with the game's `horizon:edit` modifier, which takes out the state the
    template was saved with (a vault saved with `state_updating_resumes_at` far in the future
    never opens; a spawner's saved `spawn_data` blocks its own mobs) and merges in the
    spawner's normal and ominous configs (by id: the carry upgrades `trial_spawner/` files'
    mobs to 26.3's `equipment`) or the vault's loot table and key. Better Desert Temples'
    placeholder blocks (orange, red and white glass, blue and yellow concrete, red wool,
    polished diorite, blank pots) become what its processors make of them: the fixed swaps as
    vanilla's `rule`, the random ones (suspicious sand, sand and sandstone, upside-down stairs,
    cobwebs, sherds on the pots) as the game's `horizon:pick`, on the same `nextFloat` as the
    mod. Better Dungeons' small dungeons keep one chest and each other one with a fifth's
    chance, two at most (the game's `horizon:count_cap`, counted per placement as the mod's
    DungeonContext is): 12 chests in five small dungeons at seed 1234 become 6.
    Lithostitched's processors (all in ChoiceTheorem's Overhauled Village in ATM10) are the
    game's own types, read as the mod writes them. `block_swap` keeps the properties the new
    block can have: a farm's ripe wheat becomes ripe carrots, but beetroot, whose age stops at
    3, keeps its default; a target block the game does not have (most are Nature's Spirit's and
    Croptopia's) leaves the block as it is. `reference` runs the lists it names, `condition`
    tests the template's block or the world's, `apply_random` and `set_block` draw per block or
    per piece as the data says. Lithostitched's own random is not known yet, so the shares of a
    random choice are the Java pack's but the blocks chosen differ. Lists named by tag and state
    providers other than simple and weighted ones are dropped and counted (ATM10 has none).
    CTOV's compat hooks for other mods (its `modular_compat`, 30 of them in 17 lists) become a
    `reference` when the mod they name is ported along, and are taken out (a no-op, as in the
    mod) when it is not. In ATM10 they all name Nature's Spirit, which the pack does not have.
    In Java, Lithostitched also lays out every jigsaw structure its own way (villages,
    outposts, trial chambers; CTOV's villages are `lithostitched:jigsaw`, carried as vanilla's
    jigsaw). Horizon keeps vanilla's layouts, so with Lithostitched in a pack, structure layouts
    differ from the Java pack's for the same seed (their positions do not).
    The processors that build outside their template are carried too: Repurposed's and
    Moog's pillar processors become the game's `horizon:pillar` (a column from a placeholder
    block down to the ground, with the processor list they run on each of its blocks written
    in), Repurposed's bottom pillars `horizon:bottom_pillar`, their fluid fix (a wall of the
    structure's block over water and lava sources next to its open blocks)
    `horizon:close_off_fluid_sources`, and YUNG's pillars (Better Fortresses, Better Jungle
    Temples) and Better Fortresses' stair supports `horizon:yung_pillar` and
    `horizon:stair_pillar`. They write only in the chunk being generated, as the mods do.
    Dropped processors went from 570 to 456 in Repurposed Structures (its pillars, bottom
    pillars and fluid fixes: 242 lists), 14 to 1 in Moog's, 10 to 5 in Better Fortresses and
    52 to 50 in Better Jungle Temples; the structure starts are the same (382 starts, 37,451
    pieces in chunks -64..63, seed 1234). Around spawn at seed 1234, 1,515 blocks change: the
    382 red glass placeholders under Repurposed's ocean outpost and pyramids become prismarine
    columns, and their gravel and prismarine pillars reach the sea floor. In the Nether, the
    Better Fortresses fortress at chunk (-23, 9) stands on its pillars: its terracotta
    placeholders become nether bricks and 41,487 blocks of air (and 2,163 of lava) under it
    become brick and magma columns down to the netherrack, 64,568 blocks changed in its 180
    chunks (the huge fungi there grow around the columns).
    Better Ocean Monuments' ten processors are carried as well, so its monuments are no longer
    dry: its templates are mostly air, which its air processor turns to water below the sea
    level (the game's `horizon:flood_air`); its waterlog processor (`horizon:waterlog`)
    waterlogs what can be and marks the water on the chunk's edges for postprocessing, its
    structure void keeps the world's block (`horizon:keep_world_block`), its placeholder
    blocks become slabs, sponges, seagrass, gravel and aged copper (vanilla's `rule`, and
    `horizon:pick` with a new `nextFloat` for each chance, as the mod draws), and its legs and
    Better Witch Huts' log and fence legs are `horizon:yung_pillar`. In the monument at chunk
    (2, 8) (seed 1234), 318,512 blocks of air become water and none is left between y 30 and
    60 (227,034 before); its 5,142 placeholder blocks are gone.
    Repurposed's opposite fix, which walls off the air and other liquids beside its ocean
    mineshafts' water, is `horizon:close_off_air_sources` (its 4 lists). It runs in the mod's
    order, so the walls it builds keep the later blocks of the template from the mod's "where
    the world is air, keep it" rules: around the ocean mineshaft at chunk (-18, -54), seed
    1234, 650 blocks of air and cave air become prismarine and 614 more template water blocks
    stay water (1,303 blocks change in its 81 chunks).
    Its mineshafts' sky view processor (71 lists) is `horizon:mineshaft_sky_view`: a block
    of the template at or above the world's `OCEAN_FLOOR_WG` height in its column is not
    placed, so a mineshaft cut by a valley or a ravine is not built in the open, except
    plain rails over a block that can occlude, which stay (waterlogged where the world holds
    water). Repurposed's dropped processors go from 253 to 182.
    Still taken out: Formations' biome replacements, blocks aged or propped up.
    In the 128×128-chunk square (seed 1234), every spawner of a mod structure now has its mob:
    in the Overworld 83 Better Dungeons spawners held the template's placeholder pig and 17 of
    Moog's were empty (763 spawners with a mob, up from 628); in the Nether the 52 empty ones
    in Moog's Nether Structures are filled (162, up from 110). Chests are unchanged: 2,777
    and 1,405 with a loot table. The game read loot tables from `data/minecraft/` only, so every one of
    those chests naming a mod's table opened empty; it now reads every namespace, and an
    integer field written as a float (`"rolls": {"min": 3.0, ...}`, as Towns and Towers
    writes them) counts as vanilla's int codecs count it. Of the 675 chest and other
    non-block tables in ATM10's carried mods, 534 now fill a chest (seed 1; none before).
    Repurposed's structure voids keep the world's block, as vanilla's `minecraft:block_ignore`
    does (its 82 lists: mineshafts, the ocean and swamp villages, the ocean and taiga temples,
    the jungle pyramid's hidden room). Before, the game placed them like any template block:
    around the ocean mineshaft at chunk (-18, -54), seed 1234, 974 structure voids took the
    place of 464 prismarine blocks of its walls, 225 of water (dry holes in the sea), 279 of
    air and cave air and 6 rails and walls.
  - **Rule tests** written in Java. Repurposed Structures' mineshafts and Nether monument test
    blocks with three of the mod's own rule tests inside vanilla `rule` processors: what a
    block is made of (solid, liquid, air, with rails and chains), a chance over the distance
    from the template's origin along one axis, and a world height range. The game runs them
    as `horizon:matter_phase`, `horizon:piece_origin_axis_aligned_linear_pos` and
    `horizon:y_value` with the mod's fields, and carry renames them (213 in 75 lists).
  - **Waterlogging.** Repurposed's `waterlogging_when_replacing_water_processor` (the bottom
    pillars of its villages and outposts, 16 lists) is `horizon:waterlog_in_world_water`: a
    block that can be waterlogged is where the world holds water, dry where it does not, and
    one placed in water gets a block tick. Its dropped processors go from 370 to 253 (with
    the structure voids carried); around the ocean village at chunk (12, 10), seed 1234, 6
    spruce fences of its pillars under the sea are now waterlogged.
  - **Namespaces besides the mod id.** Towns and Towers (mod id `t_and_t`) ships all 60 of its
    structures under `kaisyn` and `towns_and_towers`, which carry used to leave in `outside/`.
    Carry now claims a namespace for the mod when the mod's own files (its id's, its tag
    additions and the namespaces already claimed) name ids in it, and no mod of the pack has
    it as its id; it lists them in the manifest's `namespaces` (docs/mods.md). Files for mods
    it supports (CTOV's `wares` and `monobank`, YUNG's `morevillagers` tags) and configs of
    pack mods (`cristellib`) stay outside: of ATM10's 42 jars only T&T claims any. A tag of a
    mod's own namespace that `replace`s is carried as an addition (no pack before it defines
    it). In the 128×128-chunk square T&T has 7 starts at seed 1234 (its ocean wreckages, an
    ocean village and outpost) and 7 at seed 2 (forest and snowy taiga villages, snowy
    outposts), from none.
  - **Renamed ids.** 26.3 calls 1.21.1's chain the iron chain (its datafixers rename it in
    worlds). Carry renames it in recipes, loot tables and worldgen files (112 files in 5 mods:
    Repurposed's mineshafts, Better Dungeons' processors, Regions Unexplored's hanging sign
    recipes...), which otherwise fail to load in 26.3. With both, the 5 Nether mineshaft pools load: 47 of
    Repurposed's Nether mineshafts in the 128×128-chunk square (seed 1234), under the lava sea.
  - **Advancements.** An `enter_block` criterion names its block as `blocks` in 26.3; 26.3
    skips 1.21.1's `block`, so the criterion matched any block the player entered. Carry renames
    it (20 advancements in Regions Unexplored: "Downer", "From the Tree Tops" and "Light as a
    Frog" were granted on the first dive into the sea, and its boat recipes' "in water"
    criteria).
  - **Big templates** (#838). Vanilla reads structure templates with no limit on their tag
    count; Horizon now does too (`nbt::Limits::templates`). Seven of ATM10's 8,962 carried
    templates are past the old 1,000,000-tag cap: YUNG's monument start, four When Dungeons
    Arise main pieces and two Nullscape pieces. They used to load as empty templates, so their
    structures never started.
  - **Additions to other sets and pools** (#832 in the game, #833 in carry). A set or pool file
    holding `horizon:add_to` appends its entries to the sets or pools it names (docs/mods.md).
    Carry writes these from lithostitched's `add_structure_set_entries` and
    `add_template_pool_elements` modifiers whose load conditions hold for the mods ported
    along, and from CTOV's Java registration: its 21 village types and 11 outposts join
    vanilla's `villages` and `pillager_outposts` with the mod's default weights.
  - **YUNG's API** (#839, #846 in the game; #840, #847 in carry). `yungsapi:yung_jigsaw` and its
    elements become vanilla's jigsaw and single element, with their rules under
    `horizon:yungsapi`. The game assembles them as YUNG's `JigsawManager` does: priority
    pieces, caps by name, depth ranges, `ignore_bounds` and the conditions tree, with
    `piece_in_range`. Dead-end pools swap a piece that got no child for one of its dead-end
    pool, and once the layout is done each piece's modifiers can turn it into another template
    (`transform`) or place it last (`delay_generation`). Better Fortresses' bridges need all
    three: over seed 1234's Nether chunks -64..63 it goes from 3,765 to 4,106 pieces. YUNG's
    placements become `horizon:` keys on a random spread: `horizon:exclusion_zone` for the
    enhanced exclusion zone, `horizon:avoid_biomes` for the temples' ocean and river search,
    `horizon:ring_sections` for Better Strongholds' rings. `horizon:disables` on a YUNG
    structure turns off the vanilla type its mod's mixin turns off. It is written only when
    the game can start the replacement, so vanilla mineshafts stay while Better Mineshafts (a
    Java type) can't.
  - **Repurposed Structures' and Moog's own assembler** (#850 in the game, #851 in carry). Both
    lay out their jigsaws with `PieceLimitedJigsawManager`, not vanilla's. Their structures, the
    Nether variants too, get `horizon:piece_limited`: `y_allowance`, burying, the pools whose
    fallbacks ignore the bounds, liquid, the Nether land search and ledge, and each
    structure's piece caps and required pieces from the mod's `*_pieces_spawn_counts` files.
    The game assembles them the mods' way and runs their post-layout steps (centring,
    burying, the Nether land search).
  - **Enhanced terrain adaptation.** YUNG's API and MoogsStructureLib add a kernel of their
    own to the Beardifier: its own size and falloff, carving or burying above and below each
    piece's floor. Carry writes it as `horizon:enhanced_terrain_adaptation` on the structure
    and on pool elements with their own (docs/mods.md), and the game sums it after vanilla's
    terrain density. In seed 1234's terrain before structures are placed, Better Fortresses'
    pieces held 39 to 41 % netherrack and now 3 to 7 %, the mod's open bridges; Moog's mega
    fortress band 45 % and now 30 %; Better Strongholds' pieces, buried, go from 92 to 97 %
    solid to 99 %, closing the caves that cut through them.


  Each mod's `carry-report.json` lists what was mapped, the rules and processors lost by name,
  and the types left as they are. NeoForge's convention biome tags (`#c:is_forest`, ...) come
  from the loader jar (`<work>/loader/neoforge-<version>-universal.jar`, fetched dev-only and
  SHA-1 checked) and are written into the carry. The game skips the structures it cannot
  decode.

  On ATM10's 42 data-tier mods with a jar (seed 1234, chunks -64..63, `chunkgen_bench
  --starts` over the carried worldgen): 286 starts with 15,097 pieces, 150 of them vanilla's
  (unchanged), and no pool that fails to decode (65 before #816 in the 64×64 centre).
  - 1,061 files were mapped (MVS 413, MNS 210, MSS 157, MES 110, Repurposed Structures 104,
    ChoiceTheorem's Overhauled Village 66), and 103 convention biome tags written.
  - When Dungeons Arise, Structory and Towns and Towers use vanilla's types already (T&T's
    namespaces are carried since the namespaces rule above).
  - Still lost, written in Java:
    - Repurposed's mineshafts, mansions and monuments, Better Mineshafts and two Better
      Dungeons types: Java structure types;
    - the block processors written in Java
      (YUNG's banners and decorations, Better Strongholds' legs, Lithostitched's block swaps);
  - **Formations** (Overworld and Nether: 50 structures, 128 pool elements) places one
    template per start at a height found in the noise columns under its corners and centre: on
    the surface, on water or lava, or in a cave (on its floor, under its ceiling, buried in
    stone, on a lava lake). Carry writes its `formations:simple` as the game's
    `horizon:formations` (`horizon-world`'s `structure/formations.rs`, which reads the blocks
    of ChunkGenerator.getBaseColumn through the structure context) and its pool element as
    vanilla's with its `ground_level` under `horizon:ground_level`. In chunks -64..63 that adds
    39 Overworld and 672 Nether starts at seed 1234 (51 and 643 at seed 2), and leaves every
    other start as it was. Its processors (biome replacements, aged blocks, bed colours,
    plants) are still dropped, so its structures keep their templates' blocks.
    Its one PiecedStructure, the fountain, starts the same way and then lays out the element's
    jigsaw children with vanilla's jigsaw: carry writes it as `horizon:formations` with
    `horizon:pieced` (`max_depth`, `max_distance_from_center`), and the game's assembler takes
    the placed start piece (`jigsaw::add_children`), the children kept within that distance of
    the start's centre, one block above the height found. It shares Formations' "uncommon"
    structure set, so where the set now picks it, it takes the place of a statue or a house:
    1 fountain at seed 1234 and 3 at seed 2 in chunks -64..63, one with a child piece.

  With every structure PR above, the same square (seed 1234, chunks -64..63) holds 362
  overworld starts with 22,026 pieces. CTOV has 7, Better Dungeons 72 and Better Ocean
  Monuments 3, and vanilla's monuments are gone as YUNG's mod has them. In the Nether there
  are 512 starts, up from 322: Moog's Nether Structures 147, Repurposed's Nether structures
  40, Better Fortresses 7, with no vanilla fortress. Over chunks -256..255: Better Strongholds 10
  (vanilla 0), Better Jungle Temples 5, Better Witch Huts 11. Vanilla's own worlds are
  unchanged: the structure-start oracle still matches 35 of 35.

  Repurposed's Nether city, Nether shipwrecks and End stronghold are its generic jigsaw with
  one more start check each, carried as `minecraft:jigsaw` with the check named in
  `horizon:start_checks.repurposed`: open air from sea level + 20 to + 45 around the city's
  chunk, no solid block in a 33×30×33 grid over a shipwreck, and land 45 blocks up at nine
  points 35 and 70 blocks apart for the stronghold. Over chunks -320..319 in the Nether there
  are 2 cities and 45 shipwrecks (30 and 865 without the checks); the End stronghold keeps
  8,000 blocks from the centre, and chunks 500..1139 × -320..319 hold 2 (43 without).

  In a world with the 40 mods, seed 1234, `/locate structure mvs:azelea_house` finds one 27
  blocks from spawn and it stands there as the template builds it; When Dungeons Arise's
  bandit towers and Structory's chapel locate too.

### 5. Code: one agent per mod

For tiers logic, machines and deep, in dependency order, as many agents at once as the AI
budget allows:

- **Input:** the inventory record, the carried-over data, the oracle's exports and traces, the
  harness, the Horizon mod API docs, the ports of its dependencies, and the mod's public
  documentation. Never its code: the agent doesn't decompile, read or quote it.
- **Output:** a Rust crate in `mods/<id>/port/` built with `cargo horizon-mod build`, and its
  `.hmod`.
- **Loop:** write → build → run the harness → read the differences → fix, until the harness
  passes or the agent reports what blocks it. Usually that is an API gap, which becomes an
  issue against the mod API ("needs a scheduled-tick hook"), the way version ports turn
  differences into work items.
- **Rules:** the agent writes the port from observed behaviour, the harness and the mod's
  own data files only; ports follow the mod's licence ([Licences](#licences)).
  Players without an AI agent get the carry-over tiers. A port the licence lets us publish can
  be downloaded instead of regenerated.

### 6. The Java oracle and per-mod harnesses

The oracle is **dev-only**, like Horizon's vanilla oracles in `tools/`: the pack's own
Minecraft version (1.21.1 for ATM10) with NeoForge or Fabric, the mod and its dependencies,
plus a small exporter mod (written by us, never shipped to players). It runs headless and is
driven over a socket, as the vanilla oracles are.

A **harness per mod**, generated from the tier:

| Tier | What is compared, oracle vs Horizon with the port |
|---|---|
| data | every registered block state's facts (hardness, sounds, shapes, light, map colour, drops via loot), every item's facts, every recipe's result for its inputs, loot table outputs over fixed seeds, tags; structure placement over fixed seeds and chunks |
| logic | scripted interactions: place, use, break, tick N times; the world diff and the inventories after each step |
| machines | a fixture world with the machine, its inputs and power: the output, energy and progress per tick over N ticks; the menu's slots and data values; NBT after save and load |
| deep | entity scenarios (spawn, tick N, positions and states, drops), and render captures where the look is the point, compared as images with a tolerance |

The comparison skips vanilla 1.21.1-versus-26.3 differences: it normalises ids and components
through the step-4 upgrader and compares only the mod's own ids and effects.

**Status: the data-tier harness runs.**

```
python3 tools/neoforge_oracle.py export --jars <mods> --deps <their libraries> --out <work>/oracle/facts.json
cargo run -p horizon-port-pack -- carry <work> --resources .local/horizon-client/26.3
python3 tools/mod_port_harness.py --work <work>      # or --jars/--deps to export first
```

- `tools/neoforge_oracle.py` installs a NeoForge server (21.1.259, ATM10's) under
  `.local/neoforge-oracle/`, builds the exporter mod in `mods/horizon/src/neoforge-oracle/` (our code, dev
  only), starts the server headless with the jars and writes, for every block of the mods'
  namespaces and every vanilla block: the class chain, states, per-state hardness, light, tool
  need, map colour, sounds, outline and collision shapes; drops for hand, diamond tools, shears
  and silk touch; placement on a floor and a wall; BlockSetType and WoodType. Then items (class,
  stack size, rarity, food), recipes (type, result, count, resolved ingredients) and tags. One run
  with the seven mods below takes about 30 s.
- `crates/horizon-client/examples/mod_port_probe.rs` builds the content root from the carried
  mods and dumps what Horizon's loaders make of the same blocks, drops, recipes and tags.
- `tools/mod_port_harness.py` compares them per mod (blocks, properties, sounds, shapes, drops,
  placement against the template's in the oracle, vanilla tags, recipes, items) and writes
  `<work>/mods/<id>/harness/result.json`. It maps 1.21.1 sound events to 26.3's through the
  vanilla blocks both versions have (`block.metal.*` became `block.iron.*`) and ignores vanilla
  items 1.21.1 did not have (pale oak in `#minecraft:logs`).

The oracle needs the mods' jars, so it is not a `tools/parity.py` harness: it runs where the
user's jars are, and its facts and results stay in the work folder. The first real mods
(ATM10's own jars from Modrinth, all MIT; checks passed / compared):

| mod | blocks | properties | sounds | shapes | drops | placement | tags | recipes | items |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Macaw's Trapdoors 1.1.5 | 179/179 | 11456/11456 | 11456/11456 | 11456/11456 | 10024/10024 | 358/358 | 534/534 | 191/191 | 191/191 |
| Macaw's Doors 1.1.5 | 239/245 | 15296/15296 | 15296/15296 | 12352/15296 | 13384/13384 | 478/478 | 941/941 | 257/257 | 251/257 |
| Macaw's Fences and Walls 1.2.1 | 150/180 | 4800/4800 | 4800/4800 | 4032/4800 | 33600/33600 | 300/300 | 432/432 | 222/222 | 150/180 |
| Macaw's Paths and Pavings 1.1.1 | 234/315 | 4602/4602 | 4602/4602 | 4472/4602 | 6734/6734 | 468/468 | 416/416 | 315/315 | 234/315 |
| Glassential-renewed 3.4.7 | 105/114 | 5177/5177 | 5177/5177 | 4727/5177 | 6447/6447 | 210/210 | 105/105 | 143/143 | 106/115 |
| Factory Blocks 1.4.0 | 42/49 | 42/42 | 0/42 | 42/42 | 294/294 | 84/84 | 42/42 | 1/1 | 42/49 |
| Cloud Glass 1.1.0 | 16/16 | 16/16 | 16/16 | 0/16 | 112/112 | 32/32 | – | 31/31 | 16/16 |
| Chipped 4.0.2 | 6857/6981 | 46212/46443 | 46443/46443 | 46396/46443 | 149093/149093 | 13548/13714 | 11984/12378 | 6/13 | 6849/6973 |

What fails is what API 0.2 cannot express yet: shapes of their own (Japanese doors, hedges,
paths a sixteenth high, glass without collision), sound events of the mod's own
(`factory_blocks:metal_sound`), and blocks whose vanilla class has no vanilla block with the
same states (garage doors, double gates, engraved and facing paths, fans). Every drop, recipe
and tag of the declared blocks matches. Chipped's 124 missing blocks are its own classes (workbenches and
`DirectionBlock`), and its workbench recipes need those blocks. Its drops match since carry writes
the self-drop tables it registers in code (step 4).

### 7. Coverage report

`coverage.json` and `coverage.md`, per mod and totalled per class and tier:

- **ported**: `.hmod` built, harness green;
- **partial**: `.hmod` built, harness passes the listed parts (e.g. blocks and recipes yes,
  machine processing not yet), with the failing checks and the API gaps blocking them;
- **not yet**: waiting on an API tier or a dependency, which is named;
- **not needed**: performance, library mapped to the API, pack tooling folded into the
  pipeline, client-ui built in.

Plus the licence summary (how many ports could be published) and, for a pack, "playable?":
whether every required content mod is ported or partial with its blocks and items present.

`horizon-port-pack report <work>` writes the per-mod rows and the class, tier and licence totals from the inventory, each mod's `carry-report.json`
and its harness result. A data-tier mod is **ported** when its harness is green and nothing was
left out (blocks with a class of their own are counted and named, since facts cannot show
their extra code); **partial** with the failing checks, the blocks left out and what the
templates miss; **not yet** with the API tier it waits for, or the dependency; **not needed**
for libraries, performance mods, pack tooling and what Horizon has built in. Unclassified mods
(the 20 CurseForge-only rows the reviewed table only guesses) count as not yet.

**ATM10, measured** (`import` of ATM10's `modlist.json` from its repository on 2026-10-16, 496
files; `inventory`; `carry` and the harness on the eight mods we downloaded; `report`):

| class | ported | partial | not yet | not needed | all |
|---|---:|---:|---:|---:|---:|
| content | 1 | 7 | 268 | 0 | 276 |
| library | 0 | 0 | 0 | 77 | 77 |
| integration | 0 | 0 | 40 | 3 | 43 |
| client-ui | 0 | 0 | 27 | 2 | 29 |
| other | 0 | 0 | 1 | 23 | 24 |
| unknown | 0 | 0 | 20 | 0 | 20 |
| performance | 0 | 0 | 0 | 16 | 16 |
| server-only | 0 | 0 | 11 | 0 | 11 |
| **all** | **1** | **7** | **367** | **121** | **496** |

By tier, the 367 not yet: data 57 (carry-over and the harness, no agent: their jars were not
downloaded), logic 66 (API 0.3), machines 97 (0.4), deep 92 (0.5), client 35, unclassified 20.
Ported: Macaw's Trapdoors. Partial: Macaw's Doors, Fences and Walls, Paths and Pavings,
Glassential, Factory Blocks, Cloud Glass, Chipped (see the harness table) and MVS, whose
structures now generate (see step 4). Of the 9, 7 are MIT, one LGPL and Chipped is under the
Terrarium License; nothing is published.

## Measuring "10x"

`tools/perf_bench.py` already measures Horizon against vanilla and vanilla with performance
mods on the same machine, world and camera path ([benchmark.md](benchmark.md)). With
`--game neoforge --mod-jars <dir> --deps <dir>` it runs the mods on their own loader (NeoForge
21.1.259 for ATM10, installed under `.local/neoforge-oracle`, `-Xmx2G` like the launcher),
driven by the same steps through a dev-only NeoForge port of the bench mod
(`mods/horizon/src/neoforge-bench`). `--game horizon --port-work <work>` loads a work folder's
carried-over mods into Horizon. A whole pack (`--pack`, its recommended JVM arguments) is next.

| Metric | How | Horizon side |
|---|---|---|
| **Load time** | start → title screen; start → in a fresh world, every chunk around the spawn drawn | the same with the ported pack's `.hmod`s enabled |
| **RAM** | peak and steady RSS of the process, sampled 4 times a second from outside (the whole JVM for Java) | the whole process |
| **FPS** | the four flight scenes: mean, 1 % and 0.1 % lows, p99 frame time | same |
| **TPS / MSPT** | a fixed server-load world: N loaded chunks with the pack's machines and mobs placed by the harness fixtures; mean and p99 milliseconds per tick over 60 s (NeoForge: `/neoforge tps` and spark; Horizon: the integrated server's tick times, which today feed only the debug chart, exposed through `horizon-ctl`) | same fixture, the ported content |
| **World generation** | chunks per second pregenerating a fixed square (Chunky on NeoForge; Horizon gets a pregeneration command for this) with the pack's worldgen | same |
| **Disk and download** | install size of the pack against its `.hmod`s | |

Two comparisons, both published:

1. **Same content.** NeoForge with only the mods Horizon has ported (plus their dependencies)
   against Horizon with those ports. Content matches, so this is the honest ratio.
2. **What players run today.** The full pack on NeoForge as a reference row, labelled as
   such, beside Horizon's partial pack. This is not a like-for-like ratio.

"10x" is claimed per metric only where the same-content ratio reaches it over the repeats'
confidence interval. Other ratios are reported as they come out. Caveat (in every report):
the pack runs Minecraft 1.21.1 and Horizon runs 26.3, so the vanilla parts differ by a few
versions.

### Measured: six ported mods against the same mods on NeoForge

Cloud Glass, Factory Blocks, Glassential, Macaw's Fences, Paths and Trapdoors (Doors left out:
with Trapdoors it passes the block states chunks hold, see Limits), carried over and loaded as
dev mods in Horizon, against their jars plus Architectury and Fusion on NeoForge 21.1.259.
Mac mini M4 Pro, 1280x720, seed 1234, the four flight scenes, 2 games per row, 2026-10-17.
Means; FPS is the mean of the four scenes. "Title" is start to title screen, "world" start to
a fresh world drawn around the spawn, "scene load" a teleport to every chunk drawn.

| RD | game | title s | world s | scene load s | FPS | RSS steady / peak MB | load avg |
|---|---|---:|---:|---:|---:|---:|---:|
| 8 | Horizon + 6 mods | 9.8 | 16.2 | 4.0 | 122 | 2,644 / 2,771 | 32 |
| 8 | NeoForge + 6 mods | 19.5 | 50.0 | 30.2 | 72 | 2,325 / 2,592 | 56 |
| 8 | Horizon | 5.0 | 11.0 | 4.0 | 124 | 1,559 / 1,629 | 51 |
| 8 | NeoForge | 12.8 | 36.1 | 21.9 | 104 | 2,334 / 2,558 | 30 |
| 16 | Horizon + 6 mods | 10.8 | 27.0 | 11.3 | 90 | 2,925 / 3,032 | 40 |
| 16 | NeoForge + 6 mods | 26.4 | 82.5 | 64.3 | 25 | 3,016 / 3,519 | 45 |
| 16 | Horizon | 3.5 | 10.0 | 7.4 | 119 | 1,732 / 1,871 | 56 |
| 16 | NeoForge | 16.9 | 73.7 | 43.2 | 36 | 2,748 / 3,425 | 48 |

Same content, Horizon against NeoForge: the world is ready 3.1x sooner at 8 chunks and 3.1x at
16, the title screen 2.0x and 2.4x sooner, and the flights run 1.7x and 3.6x the frames. RAM is
not lower: Horizon's steady RSS grows 1.1 GB with the mods (61,816 block states against
vanilla's 35,723; the per-state tables are the next thing to measure), while the JVM stays
inside its 2 GB heap. Caveats, so none of this is a "10x" claim yet: other sessions and CI
runners kept the machine's load average between 30 and 56 on 14 threads, so the confidence
intervals are as wide as the means (`perf_bench.py --table`); Horizon's frames are paced near
the display's 144 Hz at 8 chunks; Horizon's title time includes building the content overlay
(about 5 s for six mods; it was rebuilt on every start then, and is kept for later starts now);
and the pack runs 1.21.1 against Horizon's 26.3.
A quiet rerun with more games is the next measurement.

**Where the RAM goes (2026-10-17).** RSS overstates it on macOS. `ps` counts pages the allocator
freed and marked reusable, and the content tables free a lot at load. `perf_bench.py` now also
samples the physical footprint (`footprint`, what the OS charges the process). The machine was
shared and very busy (load average 90-138 on 14 threads). RD 8, the same six mods, one or two
games per row:

| game | RSS steady MB | footprint steady MB |
|---|---:|---:|
| Horizon | 1,598-1,636 | 1,277-1,295 |
| Horizon + 6 mods | 2,996-3,052 | 1,736-1,869 |

So the six mods cost about 0.5 GB of footprint, not 1.1-1.4 GB. ATM10's 42 mods (193,762
states) take the game to 7.7 GB RSS but 3.9 GB footprint. The rest is startup garbage the
allocator keeps.

`crates/horizon-client/examples/resource_memory.rs` breaks it down per loader: the live heap
(a counting allocator), each step's peak, RSS and the footprint. Live heap after every loader,
vanilla against ATM10's 42 mods:

| loader | vanilla MB | ATM10 42 mods MB |
|---|---:|---:|
| block registry | 24 | 125 |
| assets (registry, models, sprites: 153 MB of pixels for 9,790 layers) | 94 | 657 |
| block facts and behaviours | +29 | +75 |
| items, recipes | +14 | +82 |
| light, mesh, collision, motion | +24 | +53 |
| worldgen | +79 | +182 |
| **live** / **footprint** | **240 / 551** | **1,049 / 2,724** |

The block registry kept a second copy of every state as the key of its id map; it now
indexes ids by a hash checked against the states: -22 MB live for vanilla, -124 MB for
ATM10's, -30 MB and -140 MB of footprint. The startup peaks are what is left. Each step parses
its per-state tables whole: as JSON trees, or as NBT trees of 300-600 MB each for ATM10
(`block-properties.nbt`, the collision, support, occlusion and movement tables). The peak
reaches 3.9 GB during worldgen loading, and the footprint stays near the peak.

The NBT tables now stream: the readers check each state's identity as it is read, and no
`states` tree is built (#753). That tree was about 240 MB for each table. With the carried
ATM10 mods (153,511 states), the peak of each step falls by about 210 MB:

| step | before MB | after MB |
|---|---:|---:|
| light | 911 | 699 |
| collision | 945 | 733 |
| motion | 1,427 | 1,215 |
| worldgen, uncached | 3,000 | 2,788 |

The fluid tables no longer decode the 211 MB `gameplay-facts.json` a second time (#754). The
content root is kept between starts (#756). What is left at startup:

- `facts.json` went through a JSON tree, then an NBT tree, in both the registry and the assets
  step. It now streams (#825): at ATM10's size (214,338 states) each step's peak fell from
  816 MB to 153 MB (registry) and 235 MB (assets), and the RSS after the assets step from
  1,344 MB to 442 MB;
- on a first start, the content-root build and `Behaviours`' decode of `gameplay-facts.json`
  (cached afterwards in the behaviour snapshot).

**Against NeoForge, same 40 mods (2026-10-17).** The 40 data-tier mods carried from ATM10 (the
42 without Regions Unexplored and MO Structures, whose libraries are only on CurseForge),
against their jars on NeoForge 21.1.259 with `-Xmx2G`. `perf_bench.py --port-work`, RD 8, one
game each, load average 80-125 on 14 threads; `--warm` measures a later start:

| game | title / world s | RSS steady / peak MB | footprint steady / peak MB |
|---|---|---|---|
| Horizon before #756 (every start) | 81.1 / 102.8 | 4,008 / 4,164 | 2,368 / 2,554 |
| Horizon, first start | 105.2 / 128.8 | 3,795 / 3,993 | 2,306 / 2,560 |
| **Horizon, later starts** | **21.6 / 33.7** | **2,629 / 2,935** | **2,445 / 2,731** |
| NeoForge + 40 mods | 53.2 / 119.9 | 3,099 / 3,203 | 3,304 / 3,381 |

On later starts, Horizon uses 15 % less RSS and 26 % less footprint than NeoForge with the
same mods. It reaches the title 2.5x sooner and the world 3.6x sooner, and its flights ran at
93-233 FPS against NeoForge's 27-62. The first start is slower than NeoForge's to the title,
because it builds the content root once. One game per row on a loaded machine is not a
benchmark; a quiet rerun with repeats is still to do.

## The working version for 2026-10-23

In scope (each is a PR or a small stack):

1. **This design merged.**
2. **Registries 0.2** in the game: content-only `.hmod`s with blocks (full cubes and the
   vanilla templates the first pack needs: doors, trapdoors, fences, walls, slabs, stairs,
   panes, glass), items, block items, recipes, loot tables, tags, blockstates, models, textures
   and lang, in single player. Proven by a harness: a test mod's block placed, rendered, broken
   with the right time and tool, dropping its loot, crafted from its recipe; every per-state
   fact equal to its template's with the overrides applied. Parity unchanged with no mods.
3. **`horizon-port-pack`**: import (`.mrpack`, CurseForge zip and instance, Prism), inventory,
   classify, licence record, dependency order, data carry-over for data-tier mods and their
   `.hmod`s, coverage report. Run on ATM10's real list (coverage report published in the PR,
   metadata only) and on a small real pack end to end.
4. **First pack end to end, locally:** 5–10 data-tier mods with permissive licences from ATM10
   (candidates: Macaw's Doors, Fences and Walls, Trapdoors and Paths (MIT), Factory Blocks
   (CC0), XTones Reworked (MIT), Cloud Glass (MIT), Glassential (MIT), Crystalix (LGPL)), each
   with its harness against the NeoForge 1.21.1 oracle. The ports stay local even though these
   licences would allow publishing: we ask first.
5. **Launcher:** "Import modpack" (`.mrpack` / CurseForge zip / instance) showing the
   coverage report (ATM10's first), and "Port and play" for what is portable.
6. **Measured:** load time, RAM and FPS of that pack on NeoForge 1.21.1 against Horizon, with
   the full ATM10 on NeoForge as the reference row.

Out of scope for that date, and next in order: the rest of worldgen carry-over (mods'
structures, sets, pools and features generate now, see step 4; mod biomes, noise and density
functions, and the structure types written in Java such as YUNG's and Formations', do not yet),
behaviour callbacks and server events (0.3), block entities, menus, capabilities and net
(0.4), the first agent-ported logic-tier mod as the pilot of step 5, then machines. Porting all
of ATM10 is a campaign. Each step publishes its coverage number; we make no claim beyond what
the report shows.

### Where ATM10's data mods stand (2026-10-22)

Measured with #1175, #1176, #1177 and #1191 on main, over the 42 data-tier mods carried from
ATM10 (the launcher's `--dev-mod` carries), seeds 1234 and 2.

- **Structures.** The carries define 687 structures in 25 mods. 419 of them start somewhere in
  chunks -128..127 of the Overworld, the Nether or the End at one of the two seeds, and 264 in
  chunks -64..63 (249 on 2026-10-19). Most of the rest are biome-bound and rare: no desert lies
  in either square, and ChoiceTheorem's Overhauled Village (25 of 78) and Towns and Towers (15
  of 60) have a variant per biome. 27 never start because their type is written in Java:
  Better Mineshafts 13, Repurposed Structures' mansions 8, monuments 4 and End mineshaft 1, and
  one Better Dungeons type.
- **Processors.** 1,560 carried and 605 dropped (1,347 and 818 on 2026-10-19). The dropped ones
  by mod: Formations 227 (Overworld 182, Nether 45), Repurposed Structures 182, Better Dungeons
  69, Better Jungle Temples 50, Better Strongholds 41, Better Desert Temples 14, Villages and
  Pillages 10, Better Fortresses 5, Better Witch Huts 3, Mo' Structures 3, Moog's Nether
  Structures 1.
- **Content.** 190,550 block states and 12,931 items load. 5 block models are missing (4 of
  Macaw's Doors' cherry waffle door, Regions Unexplored's blue crystalite): those blocks draw
  the missing model.
- **Load time** (Mac mini M4 Pro, offscreen, render distance 12). The first start builds the
  content root: about 25 to 35 s, and again after every game update, since the root's stamp
  includes the executable. Later starts: 6.6 to 8.2 s to "resources loaded", against 1.3 to
  2.5 s for vanilla.
- **Play.** No crash at either seed: spawn, Mo' Structures' pirate ship, castle in the sky,
  barn house and villager tower, CTOV villages, Formations' fountains (one with its chamber),
  the Nether.

What is missing, ranked by how much a player notices:

1. **Code mods.** 333 of ATM10's 496 mods (machines, magic, mobs, tools, storage): the pack's
   gameplay. They need behaviour APIs (0.3 and later) and a port per mod.
2. **The 12 mods only on CurseForge**, which the carry has not fetched.
3. **Plainer structures.** The 605 dropped processors are what makes structures look aged,
   overgrown or fitted to their biome in Java: Formations' biome swaps and block ageing (its
   tables are All Rights Reserved code), Repurposed Structures' random and noise replacements
   and floating-block removal, and YUNG's decorations (Better Dungeons, Jungle Temples,
   Strongholds).
4. **Structures that never generate:** the 27 Java types above. Repurposed Structures' mansions
   and monuments are about 3,000 lines of LGPL layout code; whether to port them into this MIT
   repository is a licence question.
5. **The first start after installing or updating:** 25 to 35 s. Later starts are about four
   times vanilla's.
6. **Smaller gaps:** Mo' Structures' mixin that keeps lakes out of its structures and its two
   block-ageing processors, and the 5 missing block models.

### Three packs through the mod browser, measured

2026-10-22, Mac mini M4 Pro (offscreen, a debug launcher, release-profile `horizon-port-pack` and game), while
other sessions kept the CPU at 85 to 93 %, so times are upper bounds. The two Modrinth packs were imported with
`horizon-launcher mods install <slug>` (the browser's Import), ATM10 from its CurseForge zip with `modpack import`
(CurseForge support is coming soon), then each ported with `--resources` for 26.3 and opened as a survival
world with every ported mod on (horizon-ctl `launch --dev-mod ...`, from a scratch script).

| Pack | Files | Import | Port | Ported mods | Coverage | Declared | World load (resources) | Crashes |
|---|---|---|---|---|---|---|---|---|
| Vanilla Perfected 26.3 (Fabric, small) | 108 | 30.4 s | 1.0 s | 2 | 2 partial, 79 not yet, 27 not needed | 0 blocks (a trail effect, a torch-flower texture) | 5.2 s (2.1 s) | none |
| Slimes Adventure 1.13.4 (Fabric 1.21.1, medium) | 105 | 18.8 s | 22.8 s | 20 | 20 partial, 46 not yet, 39 not needed | 8,208 blocks, 41 items, 1,376 recipes | 23.0 s (15.0 s) | none |
| All the Mods 10 8.3 (NeoForge 1.21.1, CurseForge zip) | 496 | 3.1 s | 29.4 s | 42 | 42 partial, 333 not yet, 121 not needed | 10,000 blocks, 1,273 items, 4,178 recipes | 38.3 s (28.7 s) | none |

The same world with no mods loads in 5.4 s (resources 1.1 s). Import is the pack download and Modrinth's answers by
file hash; Vanilla Perfected's 30 s is mostly its 108 lookups and its download. Port is the jars (downloaded for the
two Modrinth packs; ATM10's 42 copied from a folder, 12 more are on CurseForge only: CurseForge downloads coming soon) and the carry,
which writes tens of thousands of small files. World load grows with the blocks the pack declares: ATM10 adds
about 155,000 block states, and loading their models and textures is most of its 28.7 s.

Every world loaded and stayed up, with no panic and no error in the log. Before the namespace fix above, Slimes
Adventure lost three of YUNG's mods at start ("both use the namespace yungsapi"); after it all 20 load. As a share of
the mods that need porting (not counting performance mods and libraries), 2 of 81 ported partly, 20 of 66 and 42 of
375. Every ported mod is Partial: their Java code (block entities, entities, Mixins) doesn't come along.

## Open questions for Ludovic

- Contacting authors of permissively licensed mods to publish ports (none contacted).
- Whether players should run code-port agents themselves (their own Claude Code or Codex
  login, as Zenith does) or only download published ports.
- CurseForge: dropped for now (its API needs a paid key; Ludovic, 2026-10-22), shown as coming
  soon. CurseForge packs import from their zip or an installed instance.
