# Horizon mods

Two kinds of mod run in Horizon:

| Kind | File | What it is | Where it runs |
|---|---|---|---|
| **Content mod** | `.hmod` | Data and assets: blocks, items, recipes, loot, tags, models, textures, structures. No code. | Read when the game starts |
| **Native mod** | `.hnmod` | A [horizon-mixins](https://github.com/horizon-rust/horizon-mixins) bundle: Rust that changes the game's own source before it is compiled. | In a game the launcher builds for the player ([native-mods.md](native-mods.md)) |

Fabric and NeoForge jars do **not** run as they are: their code calls and patches Minecraft's Java classes, and
Horizon has none. What carries over is the data and assets inside a jar ([modpacks.md](modpacks.md)).

Mods are **off by default**. A fresh install has no `horizon-mods` folder and plays exactly like vanilla 26.3.

## Playing with mods

1. Install a mod from the launcher's Mods page ([mod-browser.md](mod-browser.md)), or put its files by hand in the
   game folder (the one holding `options.txt`): a content mod's `.hmod` (or a folder holding `horizon-mod.toml`) in
   `horizon-mods/`. Native mods go in `horizon-native-mods/`, and the launcher builds them
   ([native-mods.md](native-mods.md)).
2. Options > **Mods** lists them with what each may do ("May add blocks and items to your worlds") and a switch.
   They start off. The choice is kept in `horizon-mods.txt` (`mod.<id>:true`).
3. A content mod's blocks and items load once, when the game starts: restart the game after a switch.
4. A mod that can't load shows why, in red, on its row.

Two turned-on content mods never own the same namespace: the first by file name loads, and the other stays out with
the reason in the log.

Benchmarks (`--benchmark`) never load mods.

## Making a content mod

```sh
cargo install --git https://github.com/horizon-rust/horizon cargo-horizon-mod
cargo horizon-mod new my-mod        # a content mod: manifest, one block with its model, loot and recipe
cd my-mod
cargo horizon-mod run               # build my-mod.hmod, then start the game with it turned on
cargo horizon-mod build             # only build: target/horizon-mod/my_mod.hmod
cargo horizon-mod check target/horizon-mod/my_mod.hmod   # the registry's checks; --json for JSON
```

- `new <path> [--name "Name"]` writes the template. It makes content mods only: any other `--template` is refused.
- `build` packs `horizon-mod.toml`, `assets/` and `data/` into `target/horizon-mod/<id>.hmod`, with the same checks
  the game makes when it reads the content. It refuses a folder with a `Cargo.toml` (code is native now) and a
  manifest without capability `registries`.
- `run` builds, then starts `horizon-client` (on `PATH`, or `--game PATH`, or `HORIZON_CLIENT`) with
  `--dev-mod target/horizon-mod/<id>.hmod`, always on. Arguments after `--` go to the game. The game reads content once
  at start, so rebuild and run again after an edit.
- `check <file>` runs what the registry runs: the file's size and SHA-256, its manifest and content, and a signature
  with `--key <public key> --signature <file>.minisig`. It reads `.hmod`, `.hnmod`, `.hpack`, resource pack `.zip`
  and `.hshader` files, and refuses an `.hmod` with WebAssembly code.
- `registry build|review|check|list` is the registry repository's CI ([api/mod-index.md](api/mod-index.md)).

### The manifest

```toml
id = "my_mod"                # [a-z0-9_]{1,64}: settings key, log name
name = "My mod"
version = "0.1.0"
description = "..."
authors = ["..."]
api = "0.2"                  # content mods
side = "both"
capabilities = ["registries"]
namespaces = []              # optional: more namespaces the mod's files use (below)
```

Unknown keys are refused, so a typo is an error, not a silently ignored permission.

### Content mods (API 0.2)

A content mod has capability `registries` and carries `assets/` and `data/` in the `.hmod`. It adds blocks and items
by declaring them in `data/<id>/horizon/block/*.json` and `data/<id>/horizon/item/*.json`, each copying a vanilla
template ([modpacks.md](modpacks.md#registries-02-data-driven-blocks-and-items)):

```json
{ "template": "minecraft:iron_block",
  "properties": { "hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block" },
  "item": true }
```

An item copies its template's data components, with `max_stack_size` and
`rarity` changed, and any other 26.3 component set or removed in
`components`, written as a data component patch is: `minecraft:<type>` sets
one, `!minecraft:<type>` with `{}` removes it. That is how a modded sword
gets its own damage and durability, or a tool its mining speed:

```json
{ "template": "minecraft:iron_sword",
  "components": {
    "minecraft:max_damage": 900,
    "minecraft:attribute_modifiers": [{ "type": "minecraft:attack_damage",
      "id": "minecraft:base_attack_damage", "amount": 7.0,
      "operation": "add_value", "slot": "mainhand" }],
    "!minecraft:repairable": {} } }
```

A component type no vanilla item has is refused, so a misspelt one fails
the load instead of doing nothing. Behaviour the game ties to a vanilla
item's id doesn't come along yet: a copy of the trident doesn't throw
([#3305](https://github.com/horizon-rust/horizon/issues/3305)).

The fence: files go in the mod's own namespaces (`assets/<id>/`,
`data/<id>/`, and those its manifest lists), plus additions to vanilla tags
(`data/minecraft/tags/`) and to the convention tags NeoForge and Fabric mods
share (`data/c/tags/`).
Templates and `sounds` name vanilla blocks; unknown keys are refused.

Some Java mods ship their files under other names than their mod id (Towns and Towers, mod id `t_and_t`, under
`kaisyn` and `towns_and_towers`). A content mod lists those in its manifest:

```toml
id = "t_and_t"
api = "0.2"
capabilities = ["registries"]
namespaces = ["kaisyn", "towns_and_towers"]
```

A listed namespace is the mod's as its id is: its `assets/` and `data/` files load, and the game reads its tags,
recipes, names and loot. Blocks and items are still declared under `data/<id>/horizon/` only. `minecraft`, `c` and
`horizon` cannot be listed. Two mods that are on may not own the same namespace (one's id or listed namespace being
the other's): the game keeps the first in load order and leaves the other's content out, with "mods A and B both use
the namespace N; turn one of them off" in the log. Paths use vanilla's resource characters, at most 65,536 files, 16
MiB each and 512 MiB in all. A mod whose content is refused is left out, with the reason in the log. The game reads
this content into its block and item tables (`horizon_client::mod_content`, [modpacks.md](modpacks.md)). A tag entry
naming an id nobody registers (a block a port left out, another mod's item) is dropped, so the rest of the tag still
loads; vanilla would drop the whole tag.

Tags merge as a data pack's do. Every mod's file for one tag (vanilla's `data/minecraft/tags/`, the convention
`data/c/tags/`, or another mod's) adds its values after vanilla's and the mods' before it, in load order. A file with
`"replace": true` first drops what came before it, vanilla's entries and the earlier mods', and the mods after it
still add theirs, as a higher data pack does. Since the result depends on load order, the game says so in the log:
"mod towns replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json: vanilla's entries and those
of adds are left out, ...". Tags are the only `minecraft` files a mod may hold: its other content stays in its own
namespaces.

A mod adds to structure sets and template pools that are not its own (vanilla's villages, another mod's houses) with
a set or pool file in its own namespace holding `horizon:add_to`, a key vanilla data never has. Its entries are
appended to each set or pool named, after every mod's own files, in mod and file order; the file is no set or pool
itself. A name nobody defines is skipped.

```json
{ "horizon:add_to": "minecraft:villages",
  "structures": [ { "structure": "mymod:village_beach", "weight": 10 } ] }
```

A pool addition holds `"elements"` as a pool does. `horizon-port-pack carry` writes these from the Lithostitched
modifiers `add_structure_set_entries` and `add_template_pool_elements`.

A structure (and any of its pool elements, in place of the structure's) may hold
`horizon:enhanced_terrain_adaptation`: MoogsStructureLib's terrain kernel, which carves or buries terrain around the
pieces on top of vanilla's `terrain_adaptation`. `kernel_size` and `kernel_distance` shape the kernel; `top` and
`bottom` (`carve`, `bury` or `none`) act above and below a piece's floor; `bottom_offset`, `padding` (`x`, `top`,
`bottom`, `z`) and MoogsStructureLib's `band` (`bottom`, `top`, `piece_heights`) move the box it measures from. On a
structure, `rigid_junctions` adds rigid pieces' junctions as well and `inflate` grows the start's box by the kernel
radius. `kernel_size` 0 is none.

```json
{ "horizon:enhanced_terrain_adaptation": { "kernel_size": 24, "kernel_distance": 24,
    "top": "carve", "bottom": "none", "rigid_junctions": true, "inflate": true } }
```

## Native mods

Behaviour that needs code (a new key, a HUD, a tool that changes how mining works) is a native mod. It is a bundle of
a `horizon-mixin.toml` and a few `.rs` helpers that hook into named functions of the game. Start from
[horizon-native-mod-template](https://github.com/horizon-rust/horizon-native-mod-template). How the launcher builds
one, what it pins and why it is not sandboxed: [native-mods.md](native-mods.md).

## Publish your mod

Horizon's registry lists mods, modpacks (`.hpack`) and resource packs made for Horizon, each reviewed before it is
published. Players find them at [playhorizon.dev/mods](https://playhorizon.dev/mods) and in the launcher (Mods, then
**Horizon** beside Modrinth), which checks every file before it installs it.

1. Run `cargo horizon-mod check` on the file (above): the registry runs the same checks.
2. Put the source somewhere public, under a licence that lets players run it: reviewers read it, and the file must be
   built from it. A native mod's `.hnmod` is its source, and a maintainer reads it before it is listed.
3. Open [playhorizon.dev/mods/submit](https://playhorizon.dev/mods/submit), sign in with GitHub (it reads only your
   public profile) and fill in the page: the file, its name and summary, licence, source address, and optionally a
   minisign public key with the file's `.minisig`.
4. The form checks the file again and opens a pull request in
   [horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) that adds `mods/<id>.toml`, with you as its
   owner. That repository's checks run `cargo horizon-mod check` on the file and post their report on the pull
   request; answer the reviewers there.
5. A reviewer merges it: within minutes the file is a release of horizon-mods, and the project is on the site and in
   the launcher.

Once it is published, its page and its card in the launcher show its downloads (GitHub's counts of its files,
refreshed every hour) and its rating: players signed in with GitHub on its page give it 1 to 5 stars, one rating per
account, which they can change. Its owners can't rate it, and nobody sees who rated.

An update is the same form with a new version; only the project's `owners` can publish one, signed with its key if it
has one. A published file never changes. Without the form, open the pull request by hand (the repository's README
says how). The formats: [api/mod-index.md](api/mod-index.md).

### Integrity and signing

- **Integrity.** Every download is checked against the SHA-256 the index lists before the file reaches the game
  folder.
- **Signing.** Authors sign files with an Ed25519 key (minisign format: a `.minisig` beside the file); the index pins
  each project's public key, so an update signed by another key is refused. Unsigned local files stay allowed, with an
  "unsigned" mark.
- **Takedowns.** A file the registry takes down (`revoked.toml`) is refused by the launcher and turned off where
  installed.

## Horizon's official mods

Horizon's own mods are native. They live in
[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) under `native/<name>/`:

| Mod | Folder | Replaces |
|---|---|---|
| FPS Counter | `native/fps-overlay` | Extras > FPS counter |
| Keystrokes | `native/keystrokes` | Extras > Keystrokes |
| Zoom | `native/zoom` | Extras > Zoom |
| Minimap | `native/minimap` | Extras > Minimap |
| Recipes | `native/recipes` | The Recipes WebAssembly mod (item panel and recipe pages, on hook points version 2) |
| Coordinates HUD | `native/coords-hud` | The WebAssembly SDK's example mod (position and facing, `/where`, `/coords`) |

The game keeps the parts that must be fast, such as drawing the minimap's map, and a native mod picks the options
([extras.md](extras.md)). Far View is not a mod any more: its distance and detail are Graphics > LOD Distance and Far
Detail ([graphics.md](graphics.md)).

## WebAssembly code mods (removed)

Horizon used to run mods written in Rust and compiled to sandboxed WebAssembly (`.hmod` files with a `mod.wasm`).
That runtime, its SDK, the `horizon:mods` interface and the example mods are gone.

- **A player** who has such a mod sees it in Options > Mods as "Doesn't run any more: needs a native port", and reads
  one notice in the chat. The mod's files are not deleted.
- **The launcher** hides these mods from search and browse, and can't install one alone. A modpack that requires one
  still installs, with the mod listed as not running and its code left out.
- **An author** ports the mod to a native mod: copy
  [horizon-native-mod-template](https://github.com/horizon-rust/horizon-native-mod-template), look at the official
  ones in horizon-mods, and read [native-mods.md](native-mods.md). The registry keeps the old versions for the
  modpacks that need them.
- **A content mod is not affected.** Only an `.hmod` with a capability besides `registries` needs a port.
