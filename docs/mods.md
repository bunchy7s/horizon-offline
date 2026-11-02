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
2. Options > **Mods** lists them with what each may do ("May add blocks, items, mob effects and entity types to your
   worlds") and a switch. They start off. The choice is kept in `horizon-mods.txt` (`mod.<id>:true`).
3. A content mod's blocks, items, mob effects and entity types load once, when the game starts: restart the game after
   a switch.
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
  and `.hshader` files.
- `registry build|review|check|list|release|page` is the registry repository's CI ([api/mod-index.md](api/mod-index.md)).

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
template ([modpacks.md](modpacks.md#registries-02-data-driven-blocks-items-and-mob-effects)), and creative tabs, mob
effects, entity types and menu types (each below) in `creative_tab/`, `mob_effect/`, `entity_type/` and `menu/` beside
them. LAN and dedicated server players play with them when they run the same mods: a client whose mods add different
content is refused when it joins, with the reason.

A block names its template and what differs from it:

```json
{ "template": "minecraft:iron_block",
  "properties": { "hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block" },
  "item": true }
```

`random_ticks` makes a block tick randomly whatever its template does, as vanilla's
`isRandomlyTicking(state)` does for its own blocks, so a crop, sapling or soil on a template that never
ticks still grows: `"random_ticks": true` for every state (or `false`, to stop a template that ticks), or
`{ "property": "grown", "values": ["false"] }` for the states whose own state property (declared in
`state_properties`) is one of the values. The block's native `native_random_tick` hook then fires as it does
for a vanilla block that ticks, and its sections count it.

A block copies its template's states. It can also declare state properties of
its own, which its template does not have — how a pipe says which sides it
connects to, or a machine whether it is running. Its states then become its
template's states times the combinations of the ones it declares, laid out as
vanilla lays out a block's states (`StateDefinition`): every property, the
template's and the declared ones, sorted by name with the first name varying
slowest, and each declared property's default value first. The order is fixed,
so ids do not move between loads, and a declared name may sort before one of
the template's (`bites` on a carved pumpkin, whose `facing` comes after it):

```json
{ "template": "minecraft:stone",
  "state_properties": {
    "facing": { "type": "enum", "values": ["north", "south", "east", "west"] },
    "lit": { "type": "boolean" } },
  "default_state": { "lit": "true" } }
```

A `boolean` is `false` and `true`, as vanilla spells them; an `enum` is a list
of values, and a name or value is lower case letters, digits and underscores.
A declared name may not be one the template already has, and `default_state`
takes one value per declared property too, so a block's default is exactly one
state of its own. Every combination has to fit in the states a world holds
(2^20), so a declaration that would not is refused when the mods load.

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

Native mods can register component types before content loads. A content item
can set a registered type with a JSON value, such as `"sample:charge": {"charge": 42}`.
JSON values become NBT values. Values must fit the network NBT limits and
round-trip through saved NBT; negative zero is not supported. A key such as
`"!sample:charge": {}` removes the template's default. Unknown types fail when Horizon builds the content root;
`cargo horizon-mod check` cannot check native registrations.

`crafting_remainder` names what the item leaves in the crafting grid when a
recipe uses it, as a milk bucket leaves a bucket (vanilla's
`Item.Properties.craftRemainder`, not a data component): a vanilla item or
one of the mod's own, refused at load otherwise. It holds on the crafting
table, the inventory grid, the crafter and as furnace fuel (the last of the stack leaves it in
the fuel slot, a stack with more left drops it), not yet in a brewing stand. What the item leaves once eaten
or drunk is the `minecraft:use_remainder` component instead:

```json
{ "template": "minecraft:mushroom_stew", "crafting_remainder": "minecraft:bowl" }
```

A block's item takes the same `components`, over the components of its
template block's item; a block with `"item": false` can't have them. Block files
have no `max_stack_size` or `rarity` keys, so those two go in `components` here
(an integer 1 to 99, and one of `common`, `uncommon`, `rare`, `epic`):

```json
{ "template": "minecraft:iron_block",
  "components": {
    "minecraft:max_stack_size": 16,
    "minecraft:damage_resistant": { "types": "#minecraft:is_fire" } } }
```

A block that joins its neighbours in six directions (a cable, a pipe) copies
`minecraft:chorus_plant`, vanilla's `PipeBlock`, and names the block tag it
connects to besides itself:

```json
{ "template": "minecraft:chorus_plant", "connects_to": "#mymod:cable_connects" }
```

It keeps the chorus plant's six sides but none of its rules: it needs no
support, and each side is joined when the block there is the same block or in
the tag, on placement and whenever that neighbour changes. Its shapes are the
chorus plant's (10 pixels across) unless it sets `"thickness"`, 2 to 14
pixels: its outline and collision are then built as `PipeBlock` builds its
own, a core and an arm to each joined side.

A mod's creative inventory tabs are `data/<id>/horizon/creative_tab/*.json`:
a title, the item on the tab's button, and its items in order (items the game
doesn't have are left out):

```json
{ "title": "Ruby", "icon": "ruby:ruby", "items": ["ruby:ruby", "ruby:ruby_block"] }
```

They come after vanilla's tabs, ten to a page as NeoForge pages them; the
buttons above the inventory turn the pages. The title is a lang key or English
text: `"title": "itemGroup.ruby"` uses that key from the mod's lang files, and a
lang file's `itemGroup.<mod id>.<file name>` (`creative_tab/gems.json` →
`itemGroup.ruby.gems`) translates any title. A mod's items join the search tab
under their names.

A component type no vanilla item has is refused, so a misspelt one fails
the load instead of doing nothing. Behaviour the game ties to a vanilla
item's id doesn't come along yet: a copy of the trident doesn't throw
([#3305](https://github.com/horizon-rust/horizon/issues/3305)).

A mod effect is declared in `data/<id>/horizon/mob_effect/<name>.json` with what vanilla's `MobEffect` takes: its
category (`beneficial`, `harmful` or `neutral`) and its RGB colour (0 to `0xFFFFFF`), which its particles take:

```json
{ "category": "harmful", "color": 9154528 }
```

It is the effect `<id>:<name>`: `/effect give @s ruby:glow 30` gives it, saves and mobs keep it, and it reaches LAN and
dedicated server players who run the same mods (its network id follows vanilla's). Its name is the translation
`effect.<id>.<name>` from the mod's `assets/<id>/lang/`, its icon `assets/<id>/textures/mob_effect/<name>.png` (18x18,
as vanilla's; without one it draws as the missing texture). It does nothing on its own yet: no tick behaviour and no
attribute modifiers. A world opened without the mod drops the effect from its players and mobs. Clients join a server
only with the same mods' effects, as with its blocks.

A mod entity type is declared in `data/<id>/horizon/entity_type/<name>.json`. It copies a vanilla type: for now only
`minecraft:snowball`, a thrown item projectile. `item` is one of the mod's own items (one type per item), and `damage`
(0 by default) is what a hit deals to anything it hits, as `thrown` damage from the thrower (the snowball's 3 to blazes
is not copied):

```json
{ "template": "minecraft:snowball", "item": "ruby:ember", "damage": 2.0 }
```

It is the entity type `<id>:<name>`, with its network id after vanilla's. Using its item throws it as a snowball is
thrown: same speed, spread, sound and cooldown, and one item used outside creative. It flies as a snowball, breaks on
what it hits with its item's crumbs, and is drawn as its item's model (`ThrownItemRenderer`). Its name is the translation
`entity.<id>.<name>`. It works in single player and on LAN with the same mods. Dedicated servers share that code,
untested so far. Clients join a server only with the same mods' entity types, as with its blocks. It is saved with its
chunk, `/summon <id>:<name>` makes one, and it has its template's row in `entity-types.json`. A world opened without the
mod keeps the saved ones as they are, frozen in place. Not yet: a dispenser doesn't throw it, and a vanilla client on a
server with it may be disconnected when one spawns.

#### Menu types

A mod menu type (vanilla's `MenuType`) is declared in `data/<id>/horizon/menu/<name>.json`: its container's slot count
(1 to 256), the data slots its menu syncs (`ContainerData`, 0 to 256, 0 by default), and the slots players only take
from (`results`, as a furnace's result slot):

```json
{ "slots": 9, "data_slots": 2, "results": [7, 8] }
```

It is the menu type `<id>:<name>`, with its network id after vanilla's. A mod block's container opens it when its
`menu` names it with as many slots ([native-mods.md](native-mods.md), `native_block_entity_container`). The menu holds
its slots, then the player's 36. A shift-click from the inventory goes to the first slots with room, never a result
slot, and one on the container goes to the inventory, as a chest's does. `results` fence players only: hoppers follow the
container's faces and `inputs`, so leave result slots out of `inputs`. The data slots are the block entity's `data`
(16-bit on the network, as vanilla's), all sent when it opens, then each change. A Horizon client shows it through the
mod's own screen (`native_screen`); without one it closes again. Clients join a server only with the same mods' menu
types, as with its blocks, and a client that isn't Horizon's is never sent one: the block opens nothing for it. Its
clicks run on the server's vanilla menu engine (`menu::Kind::Mod`), shift-clicks routed as `ChestMenu`'s, and a native
mod can decide what a slot takes in code with `native_menu_slot_rules` ([native-mods.md](native-mods.md#menu-slot-rules)).
Not yet: declared per-slot rules beyond `results`, hooks for shift-click routing and `stillValid`, opening with extra
data, and client prediction of its clicks.

#### Namespaces and limits

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
recipes, names and loot. Blocks, items, creative tabs, mob effects, entity types and menu types are declared in
`data/<id>/horizon/` only. `minecraft`, `c` and
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

A mod adds to structure sets, template pools and processor lists that are not its own (vanilla's villages, another
mod's houses) with a set, pool or processor list file in its own namespace holding `horizon:add_to`, a key vanilla data
never has. Its entries are appended to each set, pool or list named, after every mod's own files, by
`horizon:priority` (lower first, 1000 when it has none), then in mod and file order; the file is no set, pool or list
itself. A name nobody defines is skipped.

```json
{ "horizon:add_to": "minecraft:villages",
  "structures": [ { "structure": "mymod:village_beach", "weight": 10 } ] }
```

A pool addition holds `"elements"` as a pool does, a processor list addition `"processors"`. This is what
Lithostitched's `add_structure_set_entries`, `add_template_pool_elements` and `add_processor_list_processors` modifiers
do on NeoForge and Fabric, and `horizon-port-pack carry` writes the last two from those.

A mod's own configured and placed features (`data/<ns>/worldgen/configured_feature/`, `placed_feature/`) generate in
biomes it does not own (vanilla's) through NeoForge's biome modifiers, files in `data/<ns>/neoforge/biome_modifier/`
(older Forge mods: `data/<ns>/forge/biome_modifier/`, `forge:` types). `add_features` appends its `features` to one
generation `step` of every biome in `biomes`; `remove_features` takes them out of its `steps` (every step when it lists
none); `none` does nothing. `biomes` and `features` are an id, a `#tag` or a list of ids; `biomes` may also be
NeoForge's `any`, `or`, `and` or `not`. Every addition comes before every removal, each in mod and file order, and a
feature already in the step is not added twice.

```json
{ "type": "neoforge:add_features", "biomes": "#minecraft:is_overworld",
  "features": "zinc:ore_zinc", "step": "underground_ores" }
```

The modifiers rewrite the biomes when the content root builds, and the generator orders every biome's features as
vanilla's `FeatureSorter` does, so an added feature takes its place in that order (its random seed follows from it). A
biome no modifier names keeps its text, and a world without mods keeps vanilla's biomes byte for byte. A modifier
naming a biome, biome tag, feature or feature tag the game doesn't have (another mod's, a `#c:` convention tag), or of a
type not read yet (spawns and climate: `add_spawns`, `add_carvers`, a mod's own serializer), is skipped with a warning
naming the mod and file, as NeoForge skips what it can't resolve; the mods still load. A malformed modifier of a type
that is read (an unknown step, a missing field) stops the build, and the game starts without content mods ("content
mods not loaded: ..." in the log). Feature tags are vanilla's for now (`#tag` in `features` doesn't read a mod's
`placed_feature` tags yet); a world's data pack that replaces a biome replaces the mods' changes to it too.

A pool element may be one of Lithostitched's: `lithostitched:limited` (`delegate`, `limit`, `min_depth`) places its
`delegate` at most `limit` times in one structure start, and `lithostitched:guaranteed` (`delegate`, `count`,
`min_depth`) is tried before its pool's other elements at each jigsaw that reaches that pool (as target or fallback)
until `count` are placed. Neither goes below a piece shallower than `min_depth` (the start is at depth 0).

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

A native mod's config files go in the game's shared `config/` folder, beside `options.txt` (on a dedicated server,
beside `server.properties`), as Fabric and NeoForge mods' do: the mod picks its own file names, such as
`config/<mod id>.json`. It asks for the folder with `Game::native_config_dir()` (hook points API 26), which needs no
game, so it works from any hook, and makes the folder when missing. Settings kept with `native_set_setting` stay where they were, in `horizon-mods/.horizon-mod-data/<id>/`.

## Publish your mod

Horizon's registry lists mods, modpacks (`.hpack`) and resource packs made for Horizon. **A project is reviewed
once, when it is listed; its later releases are checked by machine, not read by a person.** Players find them at
[playhorizon.dev/mods](https://playhorizon.dev/mods) and in the launcher (Mods, then **Horizon** beside Modrinth),
which checks every file before it installs it.

1. Run `cargo horizon-mod check` on the file (above): the registry runs the same checks.
2. Put the source in a public GitHub repository, under a licence that lets players run it. A native mod's `.hnmod` is
   its source.
3. Publish a **GitHub release** in it with the file as an asset (`.hmod`, `.hnmod`, `.hpack`, `.hshader`, or a resource
   pack's `.zip`), and its `.minisig` beside it if you sign. The release notes are the changelog.
4. List the project once: open [playhorizon.dev/mods/submit](https://playhorizon.dev/mods/submit), sign in with GitHub
   and give the repository and the page (name, summary, licence, optionally a minisign public key). It opens a pull
   request in [horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) adding `mods/<id>.toml`, with
   you as owner. A reviewer reads the repository and its source and merges it.

**Releasing a version is only step 3.** Publish a GitHub release (not a draft or a pre-release); within the hour the
registry reads it, runs the checks a submission runs (the file's own, the signature, the id and the version in the
file's manifest), and lists it. A release that fails is skipped and reported in an issue of the registry, never
published. Nobody opens a pull request and nobody reviews it. The version is the one in the file's manifest (the tag
when the file has none), the date and the changelog are the release's, the SHA-256 is computed by the registry, and
the launcher downloads the asset from your repository and refuses any other bytes. A published file never changes:
publish a new version. `revoked.toml` takes a bad one down.

Once it is published, its page and its card in the launcher show its rating: players signed in with GitHub on its
page give it 1 to 5 stars, one rating per account, which they can change. Its owners can't rate it, and nobody sees
who rated.

Your page can live in your repository too: a `horizon-registry.toml` at its root (on the newest release's tag) with
`name`, `summary`, `description`, `authors` and `license` fills what the registry's entry leaves out; the entry's own
words win. Its owners, key, kind and icon are the registry's, changed by a pull request. Projects listed before this
(`versions` in their entry) keep their hosted versions. The formats: [api/mod-index.md](api/mod-index.md).

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
| Recipes | `native/recipes` | The Recipes mod (item panel and recipe pages, on hook points version 2) |
| Coordinates HUD | `native/coords-hud` | A small example mod (position and facing, `/where`, `/coords`) |

The game keeps the parts that must be fast, such as drawing the minimap's map, and a native mod picks the options
([extras.md](extras.md)). Far View is not a mod any more: its distance and detail are Graphics > LOD Distance and Far
Detail ([graphics.md](graphics.md)).
