# Native mods

Native mods are **mixin bundles** that change the game's own Rust source before it is compiled, with
[horizon-mixins](https://github.com/horizon-rust/horizon-mixins) (MIT, by levzzz5154, made for Horizon). A bundle
is a `horizon-mixin.toml` and a few `.rs` helpers that hook into named functions of the game (HEAD, BEFORE_CALL,
RETURN, MODIFY_ARG, OVERWRITE...; the tool's README lists them). A port such as Veinminer is three HEAD hooks and
one module. Start a mod from [horizon-native-mod-template](https://github.com/horizon-rust/horizon-native-mod-template).

Native mods are native code with the player's rights, not sandboxed. They are always on in the launcher: there is no
switch. The first install shows a trust notice, and every build waits behind a red warning; the registry lists one only
after a review (its later releases are checked by machine); and they play in a game of their own, built for the
instance they are installed in.

## Pinned to a Nightly

A native mod builds on one exact Horizon commit: `[compatibility] horizon_revision` in its `horizon-mixin.toml`,
and `horizon` in the `horizon-native.json` of its registry file (`.hnmod`, `horizon_mods::index::native`). Mods
pin a **Nightly's commit** (or a release's). Every Nightly ships its source and keeps its commit for good as the
tag `nightly-source/<date>-<short>` ([release-flow.md](release-flow.md#nightly-channel)), so a pinned mod still
builds after its Nightly is replaced. When the mods of one build pin different commits, the launcher says which
mod pins which and the commit the build would use, the newest of them, and builds only after the player chooses
**Build anyway** (`native-build --allow-version-mismatch`): such mods often still apply and compile. A failed build
then says the mods were made for other versions. Their horizon-mixins pins may differ too: the build uses the
newest of them and the launcher's own, never one older than a mod was made with. A bundle with no pin builds on
the commit of the installed version the build starts from.

### Mods on the hook points

A mod that hooks only Horizon's native hook points (`crates/horizon-client/src/game/native_hooks.rs`, with a few
free functions in other crates: [render hook points](#render-hook-points), [items' default
components](#items-default-components), `horizon_world::loot::native_loot_table_load`, [menu slot
rules](#menu-slot-rules); empty functions the game
calls at fixed places, plus helpers, kept stable across releases) can name the version it
needs instead of pinning: `"hooks": 1` in its `.hnmod`'s `horizon-native.json`, the `NATIVE_HOOKS_API` of that
file when it was made. Hook points are only ever added: each addition raises `NATIVE_HOOKS_API`, and a mod builds
on any Horizon whose version is the one it needs or a later one. Its `horizon` is then only the commit it was tested on. The launcher builds it on the commit the other mods
pin, else on the installed version's commit (the game the player plays), after checking that commit's
`NATIVE_HOOKS_API` (read from GitHub) is at least the mod's; when it isn't and nothing pins, it builds on the commit the
mod was tested on. In its own copy of the mod it sets `horizon_revision` to the commit it builds on. Horizon's
own native mods (Zoom, FPS counter, Keystrokes, Minimap; horizon-rust/horizon-mods, `native/<name>/`) are made this way, so any of them build together
with no re-release for each Nightly.

Hook and helper authors use [API fragments](../tools/native-hook-fragments/README.md) to request a number.

API 2 adds the item panel beside container screens and its recipe pages (`native_panel_items`,
`native_panel_recipes`, and the hook `native_panel_event` for the player's clicks, searches and keys over items), and
the game's items and recipes (`native_recipes`, `native_known_recipes`, `native_place_recipe`, `native_give`): what a
recipe viewer needs.

API 11 adds `GameLevel::native_send_particles` on authoritative servers. It sends supported
vanilla particles to nearby players. Block particles require a valid block state. Item, trail and
vibration particles are not supported by the current dispatch. Count zero sends one directional
particle. The return value says whether the call was accepted, not whether any player received it. Bundles that use it need `"hooks": 11`.

API 3 adds authoritative world lifecycle events through `native_world_lifecycle`: open ready, open failed,
and final close. Bundles that use these events need `"hooks": 3`.

API 4 adds authoritative server tick events through `native_server_tick`: start (before any level ticks, like NeoForge's `ServerTickEvent.Pre`) and end of each non-paused
server tick, with `world_running`. A frozen world reports false except during a requested step. Bundles that
use these events need `"hooks": 4`.

API 8 adds server commands and players: `native_register_commands` (the commands a mod adds to every player's
command tree) and `native_perform_command` (a player's command on the server), with helpers to answer
([Server commands and players](#server-commands-and-players)). Bundles that use them need `"hooks": 8`.

API 9 adds the acting player's plain items for server mods (count, take, give; [Server commands and
players](#server-commands-and-players)). Bundles that use them need `"hooks": 9`.

API 10 adds server menus: a vanilla chest menu a server mod opens and fills, whose clicks and closing reach the mod
(`native_menu_clicked`, `native_menu_closed`), and `native_exchange_items`, two players' stacks swapped in one checked
step ([Server menus](#server-menus)). Bundles that use them need `"hooks": 10`.

API 12 adds item tooltips through `native_tooltip`: the hovered stack, with its components, and its tooltip's
lines, which a mod can add to, change or take out of, wherever the game shows an item's tooltip. Bundles that use it
need `"hooks": 12`.

API 13 adds using an item, on the server for every player: `native_use_item_on` hears a right click with an item on a
block once the block let it through (`ItemStack.useOn`), and `native_use_item` hears the item's own use, in the air or
after `native_use_item_on` let it through (`Item.use`). Each hears a click once. A mod that claims one swings the hand
instead of the item's use. Bundles that use them need `"hooks": 13`.

API 14 adds breaking and placing a block, on the server for every player: `native_destroy_block` hears a player break a
block once the game allowed it (`ServerPlayerGameMode.destroyBlock`) and may cancel it, which keeps the block and its
drops; `native_place_block` hears a block placed from an item (`BlockItem.place`). To stop a placement, a mod claims
`native_use_item_on`, which runs before it. Bundles that use them need `"hooks": 14`.

API 15 adds block entities for a mod's blocks (`native_block_entity`, `native_block_entity_ticks`,
`native_block_entity_tick`): made when the block is placed, ticked on the server and saved with the chunk. Bundles that use them need `"hooks": 15`.

API 16 adds item slots for those block entities (`native_block_entity_container`): hoppers, comparators and breaking the
block use them as they use vanilla's containers, and `native_block_entity_tick` also gets the slots. Bundles that use them need `"hooks": 16`.

API 17 adds menus for mod containers: the container names one of vanilla's slot-grid or furnace menus, which opens when the block is used,
and `native_block_entity_tick` takes the whole block entity, with the data slots the menu shows. Bundles that use them need `"hooks": 17`.

API 18 adds what a machine's tick reaches: `native_with_mod_block_entity` (change a neighbouring mod block entity, which marks its chunk
and tells comparators when its items changed) and `native_set_block_property` (a property of the block's own state, through the level's
normal update path). Bundles that use them need `"hooks": 18`.

API 19 adds screens a mod draws for its blocks' menus: `native_screen` lays one out (its height, where each container slot is,
where the inventory starts; a slot it leaves out is not in the menu, so shift-clicks skip it) and `native_screen_draw` draws it with the mod's
own GUI textures (`assets/<namespace>/textures/gui`, drawn with `native_sprite`). Bundles that use them need `"hooks": 19`.

API 20 adds buttons, tooltips and item icons on mod screens: `native_screen_click` and `native_screen_tooltip` (the client), `native_menu_button`
(a button press on the server, on the mod block entity) and the helpers `native_item`, `native_screen_title`, `native_data`, `native_block`,
`native_block_property`, `native_insert_item` and `native_cursor`. Bundles that use them need `"hooks": 20`.

API 21 adds what a server mod needs to guard land and move players. `native_right_click_block` hears a player's right
click on a block before the block's own use (a chest, a door, a lever) and the item's, and may cancel it: nothing
happens and the player sees the block as it is. `native_acting_player` says who acts inside a player's hook (UUID,
name, level, position, game mode, health); `native_run_command` runs a command as the server's console (level 4, no
output), so `execute in <dimension> run tp <player> <x> <y> <z>` moves a player across levels; `native_action_bar`
tells a player why, even inside their own hook; and the players
`native_server_players` lists carry their health. Bundles that use them need `"hooks": 21`.

API 22 adds what a server mod needs to keep the world itself out of guarded land. `native_block_spread` hears a
fluid flow or fire spread into a block, `native_piston_move` a piston's push or pull, `native_explosion_detonate` the
blocks an explosion is about to break (a mod takes out those it spares), `native_fill_bucket` a bucket filled or
emptied, and `native_attack_entity` a player's blow on an entity (an item frame, an armor stand, an animal, a
player). Each may cancel, and nothing happens. Bundles that use them need `"hooks": 22`.

API 23 adds `native_level_tick_pre` and `native_level_tick_post`, server-side observations around each
level tick pass. Each receives the dimension ID; neither can cancel the tick. Paused and held passes fire
neither hook, and an error skips the post hook. Bundles that use them need `"hooks": 23`.

API 24 adds `native_set_block`, a server-side `Game` helper that replaces a state in the current dimension through
`Level.setBlock`. It accepts vanilla's neighbour and shape flags and returns whether a block changed; a block with a
block entity gets one. Horizon sends every write to players whatever the flags: `UPDATE_CLIENTS`, `UPDATE_INVISIBLE`
and `UPDATE_IMMEDIATE` change nothing yet. Hooks holding only `GameLevel` cannot call it yet. Bundles that use it need
`"hooks": 24`.

API 25 adds `GameLevel::native_play_sound` for registered vanilla sound events on an authoritative server.
It queues the sound for nearby players in the level's dimension, including during a guest's turn.
Pass a `SOUND_SOURCES` index, position, volume and pitch. The return value says whether the call
was queued, not whether a player heard it. The server discards unknown sound events when it dispatches
the queue. Bundles that use it need `"hooks": 25`.

API 26 adds `Game::native_config_dir()`: the game's shared `config/` folder beside `options.txt` (beside
`server.properties` on a dedicated server), where a mod names its own files, made when missing. It takes no `self`, so
it works from any hook ([mods.md](mods.md#native-mods)). Bundles that use it need `"hooks": 26`.

API 27 adds the player-health HUD rows through `native_health_hud`: every frame the HUD draws them, the
hook hears the layout's inputs (`HealthInputs`) and the screen's GUI size, and a mod that answers with
draws replaces all the rows — armour, hearts, food, air and the heights above the hotbar — for that
frame. Nothing answered draws Horizon's own. Bundles that use it need `"hooks": 27`.

API 28 adds `native_block_in_tag` and `native_item_in_tag` for server tick hooks. They read the world's merged block and item tags, including data packs and content mods. An unknown tag returns false. An empty item stack tests the `minecraft:air` item, as vanilla does. Calls with an inactive or stale tick context return an error. Bundles that use them need `"hooks": 28`.

API 29 adds right-clicking an entity, on the server for every player: `native_interact_entity` hears a right click on
an entity (its id and type, and the stack in the hand) where `Player.interactOn` starts, once per hand. A mod that claims
it swings the hand instead of the entity's and the item's own interaction. Bundles that use it need `"hooks": 29`.

API 30 adds left-clicking a block, on the server for every player: `native_left_click_block` hears the start of a dig
(the block, the face hit) once the game allowed it, before the block is attacked (a note block plays) or broken in
creative. A mod that cancels it keeps the block: on the server it isn't attacked, dug or broken, and the player's client
gets it back. The clicking player's own client still predicts the crack until then.
Bundles that use it need `"hooks": 30`.

API 31 adds a player's server tick: `native_player_tick` runs once a tick for every living player in the world, at the
end of their `ServerPlayer.tick`, with the helpers reading that player. Players still tick while the game is frozen
(`/tick freeze`), as in vanilla. A dead player isn't called, and neither is one killed earlier in the same tick; it
runs again once they respawn. Bundles that use it need `"hooks": 31`.

API 32 adds shape updates a mod can answer through `native_update_shape`: on an authoritative server, each time a
block's state is worked out from a neighbour (`BlockBehaviour.updateShape`): neighbour shape updates after a
`setBlock`, a block fitted to all its neighbours as a piston lands it or a chunk is post-processed, and the blocks at
a feature's edge as it is placed in the live world, such as a tree grown from a sapling
(`StructureTemplate.updateShapeAtEdge`). It gets the dimension, position, state, the neighbour's direction, position
and state, before the block computes it. `state` may not be in the world yet (a piston landing it), so decide from
`state`, not from the block at `pos`. Answer only for your own blocks and leave `new` empty otherwise: a state put
there is the block's new state, applied as vanilla applies the result (air breaks the block, with its drops, on a
neighbour update; post-processing and a feature's edge write air without drops), and the block's own `updateShape`
doesn't run, nor the ticks it would schedule. The level is read-only, as vanilla's `LevelReader` is there; the hook
schedules ticks with its `ticks` through the level's `native_schedule_block_tick(ticks, pos, block, delay)` and
`native_schedule_fluid_tick(ticks, pos, fluid, delay)` (vanilla's `ScheduledTickAccess.scheduleTick`; false for an
unknown name or a position that isn't loaded). It runs often, at least six times per `setBlock` that updates shapes
(more for redstone wire's diagonals and knock-on updates): keep it short. A LAN guest shows vanilla's shape for a
moment, until the server's update arrives. The deprecated in-frame path and chunk generation fire none. Bundles that
use it need `"hooks": 32`.

API 33 adds random ticks a mod can handle through `native_random_tick`: on an authoritative server, each block
`ServerLevel.tickChunk` random-ticks (`BlockState.randomTick`), with its dimension, position and state, before the
block handles it. Only blocks that tick randomly fire it (a mod block as its vanilla template block does), once per
draw that lands on one: it runs often, so keep it short. Bone meal's random ticks (`StemBlock.performBonemeal` calls
`randomTick` itself) don't fire it. The hook may change the block with the level's
`native_set_block_property(ticks, pos, key, value, flags)` and set `handled` to skip the block's own random tick, so a
mod crop grows its own way. Without `handled`, the block then takes its own random tick as it is now (if it still
ticks randomly), and lava's fluid random tick runs either way. It needs the level mutably, which only `native_random_tick` and API 34's `native_tick_block` give. It is API 18's `Game::native_set_block_property` with `Level.setBlock`'s flags (`native_hooks::flags`): a
vanilla crop grows with `flags::UPDATE_CLIENTS` (neighbours' shapes update, so an observer fires, but no neighbour
gets `neighborChanged` and comparators aren't told), a furnace sets its `lit` with `flags::UPDATE_ALL` (both). Horizon
sends every write to players whatever the flags: `UPDATE_CLIENTS` (2), `UPDATE_INVISIBLE` (4) and `UPDATE_IMMEDIATE`
(8) change nothing yet. It writes at any loaded position, returns false off an authoritative server (the `Game` one too: a window that isn't a server), for a property or
value the block doesn't have, a position that isn't loaded, or a property that already has the value (nothing is written:
vanilla's `Level.setBlock` returns false for an unchanged state, as `native_set_block` does), and true when the block changed. Elsewhere, `Game::native_set_block_property_with(pos, key, value, flags)` does the same, as a brewing
stand's tick sets its bottles with `flags::UPDATE_CLIENTS`. The deprecated in-frame path fires none. Bundles that use
them need `"hooks": 33`.

API 34 adds scheduled block ticks a mod can handle through `native_tick_block`: on an authoritative server, each due
scheduled tick of a block that is still there (`ServerLevel.tickBlock` → `BlockState.tick`), with its dimension,
position and state, before the block handles it. The hook may change the block with the level's
`native_set_block_property` and the flags the block's own tick would write with (as in API 33: an observer's tick
uses `flags::UPDATE_CLIENTS`) and set `handled` to skip the block's own tick. Without `handled`, the block then takes
its own tick as it is now, if it is still the tick's block. The deprecated in-frame path fires none. Bundles that use
it need `"hooks": 34`.

API 35 adds the commands a mod owns on the client: `native_commands` is asked for their names, so the chat box
colours, completes and lists them like the game's own, and `native_command` runs them. A name the game already has
keeps that command. Bundles that use it need `"hooks": 35`.

API 36 adds recipes of a mod's own types, for a machine's crusher or altar. A recipe file whose `type` is outside `minecraft`
(`mymod:crusher`) in `data/<namespace>/recipe/`, in a content mod (`.hmod`) or a world's data pack (not in a native mod's own bundle), stays loaded as it is: `native_recipes_of_type`
gives each one's id and JSON, for the mod to read as it likes, and `native_recipe_ingredient` and `native_recipe_result` read the
usual fields, tags included (a bare id is read as `minecraft:`; items the game doesn't know are left out, and the helper gives nothing if none is left). The recipes are the server's table: on the server thread, a dedicated server or a LAN host; a client, the single-player window included, sees only the content mods' recipes, not the world's data packs'. All three give nothing outside a world. The game doesn't craft them or show them in the recipe book; `/reload` reads a data pack's again.
Bundles that use them need `"hooks": 36`.

API 37 adds [mobs](#mobs) for server mods: spawn one, read the mobs of a level in place, and move, hurt or remove one by its id.
Bundles that use them need `"hooks": 37`.

API 38 adds redstone from a mod's block, on authoritative servers. `native_block_signal` may change the signal a block gives on
a face, weak or direct, and `native_is_signal_source` lets redstone wire connect to it. Both are asked very often, so check the
block first and answer from the mod's own state. When the output changes, and once the block is placed (a direct signal past a conductor isn't read before), `native_update_signal_neighbors(dimension, pos)` makes the blocks around
it read it again; it acts on the level in place and is refused for another dimension. Bundles that use them need `"hooks": 38`.

API 39 adds item behaviour, on the server for every player: `native_hurt_enemy` hears a player's hit land on a mob with the
held stack, `native_inventory_tick` runs each tick for every stack a player carries (armour and off hand included), and
`native_carried_stack` reads and changes that stack, and `native_on_equip` hears an equipment slot take a different item. The helpers read the acting player. Bundles
that use them need `"hooks": 39`.

API 40 adds [mod messages](#mod-messages) between a mod's server and client halves, and hearing players and mobs on the
server for every player: `native_player_died`, `native_mob_spawned`, `native_mob_killed` (a player's melee blow, a sweep or a spear thrust too),
`native_item_dropped` (a throw; not the items scattered when a player dies) and `native_item_picked_up` (`canceled` leaves the stack on the ground). They name a player by UUID. Bundles that use them
need `"hooks": 40`.

API 41 adds bundles' data files (`native_data!`, `native_built_data`; "Data files" below). Bundles that use them need
`"hooks": 41`.

API 42 adds the item entities near a block, for machines that pick up items as a hopper does:
`native_item_entities(dimension, min, max)` lists those whose box meets `min`..`max` (vanilla's
`Level.getEntitiesOfClass(ItemEntity.class, box)`), each with its entity ID, position, stack and pickup delay, and
`native_set_item_entity(dimension, id, stack)` changes one's stack, or removes it when the stack is empty, as a hopper
does with what it takes. Both act on the level in place, as `native_set_block` does. Bundles that use them need
`"hooks": 42`.

API 43 adds `native_level_random(dimension)`, the server level's own random (vanilla's `Level.random`, NeoForge mods'
`level.getRandom()`): a mod draws from the stream vanilla's explosions and block drops use, so its draws shift theirs
as a Java mod's would. It is `None` on a client of a server and for a level that isn't open: the Nether and the End
open when someone first goes there and close after 300 ticks with nobody in them. Bundles
that use it need `"hooks": 43`.

API 44 adds the server starting and stopping, on the single player server thread and on a dedicated server:
`native_server_about_to_start(dedicated)` before the worlds load (NeoForge's `ServerAboutToStartEvent`), and
`native_server_stopping(dedicated)` when an orderly stop begins, with the players still connected and not yet saved
(`ServerStoppingEvent`). Each start is followed by `native_world_lifecycle`'s `WorldOpenReady` or `WorldOpenFailed`,
and each stop by `WorldCloseFinal`; a server whose loop fails reports `WorldCloseFailed` instead of stopping. Bundles
that use them need `"hooks": 44`.

API 45 sends mod block entities to clients. Each mod block has a block entity type of its own, named after it, after
vanilla's; a client makes its copy of the block entity with `native_block_entity`, as the server does.
`native_block_entity_update_tag` gives what clients get of one (`BlockEntity.getUpdateTag`, read from the block entity
alone): it is asked when a chunk holding it is sent and when its block changes, and the client merges it into the
copy's `nbt`. A saved chunk sent past the view distance asks it on its own thread, so read only `entity`. Keep it to what clients show (a machine running, its facing, a tank's level), not a menu's progress:
menus have their own data slots. Bundles that use it need `"hooks": 45`.

API 46 adds `native_visuals`: a world-frame hook that appends block-rim and
colored-light requests. Rims trace the client's existing block outline edges,
depth-tested against terrain. Lights add camera-local RGB illumination to the
normal terrain/entity shaders, not gameplay light. Only the first eight rims
and eight lights are examined; positions must be within 32 blocks of the player,
values must be finite and bounded, and source chunks must be loaded. At most
256 rim edges and 16 colored lights are rendered. Requests are frame-local,
so disabling a native mod or leaving the world clears them without a retained
state. Helpers `native_lit_block_name` and `native_flame_particle` read loaded
blocks and emit a nearby vanilla flame particle; native mods must rate-limit
emission. It also adds `native_emissive`: a world-frame hook for at most 32 pairs of
already-loaded terrain sprites. Transparent pixels in an overlay leave its
base unchanged; opaque pixels are drawn full-bright without changing gameplay
light or the surrounding rock. The source pack must supply the overlay texture
under `assets/<namespace>/textures/emissive/`. When no mod requests overlays,
the terrain shader skips the lookup. Bundles using these hooks need `"hooks": 46`.
When a native mod bundles a Horizon shader pack, naming its folder
`native-<mod-id>` makes the Mods On/Off control pause that pack too (without
changing the player's saved shader selection); other packs are unaffected.

## A bundle

A bundle is a folder with a `horizon-mixin.toml`. Besides its helpers and modules (horizon-mixins' README) it can
have its own crate, data files and a post-build step (horizon-mixins 0.6.0). The test bundle
[`launcher/tests/fixtures/native-crate-demo`](../launcher/tests/fixtures/native-crate-demo) uses all of them:

```
native-crate-demo/
  horizon-mixin.toml
  frame.rs                # helper, compiled into the game
  data/greeting.txt       # the bundle's data: native_data!("crate_demo", "greeting.txt")
  engine/                 # the mod's own crate
    Cargo.toml            # [dependencies] crc32fast = "1"
    data/salt.txt         # the crate's data: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/salt.txt"))
    src/lib.rs
    src/bin/bake.rs       # the post-build step
```

```toml
[mod]
id = "crate_demo"
version = "0.1.0"

[[injection]]
id = "frame"
hook = "frame"            # a name from native-hooks.json, instead of `target`
hook_version = 1
kind = "HEAD"
helper = "frame.rs"
bindings = [{ name = "game", ty = "&mut crate::game::Game<'_>", value = "&mut *self" }]

[[crate]]
id = "engine"
path = "engine"           # holds Cargo.toml
# dependent = "horizon-client"   (the default: the workspace package that depends on it)

[[post_build]]
crate = "engine"
bin = "bake"
args = []
```

### Named hook points

`native-hooks.json` at the root of the source names each hook point and the version of its contract (signature,
when it is called, what it does): `{"name": "hud", "version": 1, "target": "horizon_client::game::native_hooks::Game<'_>::native_hud"}`.
An injection names `hook` and `hook_version` instead of `target`, and horizon-mixins resolves it from the catalog
of the source it applies to, so a hook point can move in the source without breaking the mods that use it. A name
the catalog lacks is `E_HOOK_UNKNOWN` (the error lists the names it has), a version it lacks `E_HOOK_VERSION`. A
contract is never changed, its `mod_api` entry (the trait method's contract) included: an incompatible change adds
a version. Helpers (`native_text`, `native_player`…) have no hook entry; the catalog's `helpers` list names the ones
the generated `Host` offers, and they are kept stable as the hook points are. A test (`catalog_matches_hook_points`) keeps the
catalog and the hook points' sources (`native_hooks.rs`, `gfx/native_render.rs`, `horizon-world`'s `loot.rs`) in step.

### The mod's crate

A `[[crate]]` is a Cargo package of the bundle's, staged as `native-crates/<mod>/<crate>/` and added to the
workspace's members and to the `[dependencies]` of its `dependent`, so helpers call it as
`crate_demo_engine::checksum(…)`. Cargo compiles it once and keeps it: editing a helper, or switching another mod
on or off, recompiles `horizon-client` but not the mod's crate, so the heavy code of a big mod belongs there and
only the glue in helpers. A crate can't depend on `horizon-client` (Cargo forbids the cycle) or on any path or git
crate, and has no `Cargo.lock`, `[workspace]` or build of its own. Horizon's own crates (`horizon-core`…) are path
crates too, so they are refused the same way, on purpose for now: a mod names game types through its helpers, which
are compiled into `horizon-client`; a stable API crate for mods' own crates is #3897.

**Dependencies come from Horizon's `Cargo.lock`.** A crate names registry crates by version requirement
(`crc32fast = "1"`, `{ version, features, default-features, optional }`); each must already be in Horizon's lock
file in a version that matches, and builds in exactly that version. The lock file is the allow list: horizon-mixins
refuses a crate the lock lacks (`E_CRATE_DEPENDENCY`) and writes the mod's crates into the staged lock without
changing any other byte, so the game still builds with `--locked`. The launcher checks it again (every downloaded
package of the staged lock as in Horizon's: name, version, source, checksum), and a feature that turns on a
dependency the lock lacks fails at `cargo fetch --locked` with a message saying so. The build stops with:

```
horizon-mixins failed (exit status: 1). The end of its output, the whole of it in …/native/build.log:
E_CRATE_DEPENDENCY: crate_lock needs rand ^0.9, which isn't in Horizon's Cargo.lock (…/horizon-mixin.toml)
```

### Conditional data

A data file that carries NeoForge's `neoforge:conditions` loads only when every condition holds, and is
silently absent otherwise — as on NeoForge, where packs gate cross-mod recipes on the other mod being
there. The recipe loaders (crafting, the stations, brewing, the recipe book, and the packs' overlays
over them) all answer the gate; `mod_loaded` asks which mods' data is in the root: `minecraft`, always,
and the content mods that are on. `neoforge:true`, `not`, `and` and `or` combine; a condition the game
does not know leaves the file out, so a future shape fails closed. Tags, loot tables and advancements
do not answer the gate yet.

### Recipe filters

A content mod or a data pack can remove recipes, or swap one of their inputs or outputs, without knowing
their ids: a file `data/<ns>/recipe_filter/<name>.json` holds `{"filters": [...]}`. Each filter has match
keys, all of which must hold (a filter with none matches every recipe), and exactly one action:

| Key | Meaning |
|---|---|
| `id` | the recipe id; `*` matches any run of characters (`minecraft:*_planks`) |
| `type` | the recipe's `type`, with the same `*` (`mymod:*`) |
| `input` | an item id or `#tag` among the recipe's inputs |
| `output` | an item id among its outputs (anything under `result`, `results`, `output` or `outputs`) |
| `remove` | `true`: the recipe is not loaded |
| `replace_input` | `{"from": ..., "to": ...}`: every input that is `from` becomes `to` |
| `replace_output` | the same, for the outputs |

```json
{"filters": [
  {"id": "minecraft:*_planks", "remove": true},
  {"type": "mymod:*", "replace_input": {"from": "minecraft:iron_ingot", "to": "#c:ingots/iron"}}
]}
```

Filters run on each recipe after the packs are merged and its load conditions held, so they see the final
set and reach every recipe type, mods' own included: they work on the recipe's JSON, every string outside
`type`, `group`, `category` and the conditions (a `{"tag": ...}` object's tag reads as `#tag`). Every recipe
loader applies them: crafting, the stations, brewing, the recipe book and what players are sent. Files apply
in id order and each file's filters in order; a pack's file replaces a file of the same id, as any data file
does. A filter with an unknown key or not exactly one action is skipped with a warning, so a typo never
matches every recipe.

### A mod's own enchantment effect kinds

A `minecraft:post_attack` entry with `enchanted` attacker and `affected` victim whose `effect.type` is in a
namespace outside `minecraft:` — where the game used to skip it silently — reaches
`native_enchant_effect(enchantment, kind, data, level, victim)` when a player's weapon hits a mob and the entry's
`requirements` pass. `victim` is the mob's id, and the acting player is the one holding the weapon. A bare type
name is minecraft's own. Other components and pairs, a hit on a player, and an entry inside `minecraft:all_of`
aren't told yet. An unclaimed kind does nothing.

### A mod's own consume effects

A consumable's `on_consume_effects` entry of a type in a namespace of its own — where the old loader
dropped it silently — reaches `native_consume_effect(kind, data)` when the item is consumed, on the server
thread, for the acting player. The kind and the JSON the file wrote are there; the game's own effects,
sounds, teleports and drops are open to the hook. An unclaimed kind does nothing.

### A mod's own loot functions and conditions

A loot table's function or condition that names a `type` in a namespace outside `minecraft:` — where the
loot roll used to silently skip it — reaches `native_loot_function(kind, data, ctx, stack)` or
`native_loot_condition(kind, data, ctx, holds)` when the table rolls. The function changes the stack; the
condition says whether it holds. The context brings the entity, the damage, the player who killed it, the
tool, the block, the luck, the biome and a seed from the roll's random. An unclaimed function leaves the
stack, an unclaimed condition fails, as unknown kinds do; a bare type name is minecraft's own.

The tables the items engine rolls (block drops, entity drops, `loot_table`, `interact_loot` and the
built-in tables) answer both hooks, with the tool and the block in the context. The function hook there
gets the stack as the main engine's (`LootStack`): its item, count, damage, enchantments (a book's stored
ones), potion, ominous bottle amplifier, stew effects and the components the engine keeps as saved, as
their JSON; what it changes comes back to the engine's stack, and a component it adds arrives with the
drop. A function that asks for a map's structure search (`map`/`exploration`) changes nothing there: the
items engine has no level to search. In the items engine's context the `entity`, `damage`, `player` and
`biome` fields are `None` and `luck` is 0; in the main loot engine's context the `tool` and `block` fields
are `None`.

Both engines' contexts carry `place`, where the roll happens: `origin` (a broken block's centre, a dead mob's
position), `block_entity` (the broken block's entity as it was before the break, saved with its id, position and
`Items`) and `level` (the dimension, the game time, the time of day, the rain and the thunder, for conditions like
vanilla's `time_check` and `weather_check`). The game sets it for a block a player or an explosion breaks and for a
mob's death loot; on other rolls `place` is `None`.

### Data files

A bundle's `data/` folder (any files) is staged as `native-data/<mod>/` at the root of the workspace. A helper or
module compiles a file in with `horizon_client::native_data!("<mod id>", "path/in/data")` (`&'static [u8; N]`,
`crate::native_data!` inside the lib), since hook points API 41. A mod's crate keeps its own data in its folder and
reads it with `include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/…"))`. The registry's `.hnmod` may carry
both (`mixins/<id>/data/…`, `mixins/<id>/<crate>/data/…`).

Both are compiled in: neither folder is beside the game when it runs. A file the game reads at run time is one a
post-build step writes into `HORIZON_NATIVE_DATA_DIR` (below), copying it from `HORIZON_NATIVE_BUNDLE_DATA_DIR` or
making it; the game finds it with `native_built_data("<mod>")` (None only when the game can't tell its own folder).

**What a `.hnmod` holds.** Only files under `mixins/<id>/`: `.rs` and `.toml` files, the data folders above, and the
text files a crate folder carries (`README*`, `LICENSE*`, `COPYING*`, `NOTICE*`, `.gitignore`, in the bundle's
folder or a crate's), so a crate that compiles its README in builds the same from the registry as from a folder. At
most 512 files and 64 MiB unpacked; anything else refuses the bundle. A local folder build has no such limits.

**One `.hmod` for content and code.** A mod's content and its native code ship as one `.hmod`, and what is inside
decides what happens: only `horizon-mod.toml` with `assets/` and `data/` is a content mod, nothing compiles; with
`horizon-native.json` and `mixins/<id>/` (a bundle as above) it is a native mod, built into the game. Both may be in
one file: `horizon-mod.toml`, `assets/` and `data/` are its content, checked as a content mod's
(`horizon_mods::Content`) and left out of the build, and the two manifests name the same mod id. A `.hmod` with
native code is never a WebAssembly mod: one carrying `mod.wasm` is refused with
`horizon_mods::index::native::WASM_GONE`. An `.hnmod` (code only) keeps loading as before. The registry keeps
publishing native mods as `.hnmod` until launchers and the game read native `.hmod`s. Every rule of the file is in
[api/mod-files.md](api/mod-files.md).

### Post-build steps

A `[[post_build]]` names a bin of one of the mod's crates. After the game compiles, the launcher builds it with the
same profile into the same `native/target/`, then runs it (at low priority, in mod order) in the new build's version
folder with:

| Variable | What |
|---|---|
| `HORIZON_NATIVE_MOD` | the mod's id |
| `HORIZON_NATIVE_VERSION_DIR` | the new build's version folder (also the working directory) |
| `HORIZON_NATIVE_DATA_DIR` | `<version folder>/native-data/<mod>/`, created: what the game reads at run time with `native_built_data("<mod>")` (or `horizon_client::native_data::built`) |
| `HORIZON_NATIVE_CACHE_DIR` | `<launcher data>/native/cache/<mod>/`, created, kept from one build to the next |
| `HORIZON_NATIVE_BUNDLE_DATA_DIR` | the bundle's own `data/` as staged for the build (`native/stage/native-data/<mod>/`); read it, don't write it |

Its output goes to `native/build.log`. A `native-post-build.json` that can't be read, or names a mod id that isn't
one, stops the build instead of skipping the steps. When a step fails, so does the build ("crate_demo's post-build
step failed …") and the version folder is removed. It runs on the player's computer like the rest of the mod's code: only the
launcher runs it, never horizon-mixins.

API 47 adds `native_entity_join_level(dimension, entity, kind, pos, loaded_from_disk, canceled)`, an entity joining a
server level (NeoForge's `EntityJoinLevelEvent`, vanilla's `PersistentEntitySectionManager.addEntity`):
`loaded_from_disk` is true when it comes back with its chunk, false for a fresh add (a drop, an orb, `/summon`, an
arrival through a portal into another level). The entity is already in the level's lists when the hook runs; `canceled` takes it back
out, and a `/summon` of it fails with vanilla's "duplicate UUIDs" message. It tells item entities and experience
orbs so far; later APIs add other kinds, so check `kind`. Bundles that use it need `"hooks": 47`.

API 48 adds [a block's items](#a-blocks-items) for server mods, what a pipe or a machine needs to pull from other blocks: `native_container_slots`, `native_extract_item`
and `native_move_items`, and `native_dimension` for a machine's tick to name its level. `native_insert_item` now goes one item at a time, so it no longer overfills a slot,
leaves a client's copy of the world alone, and can't reach a machine's own slots during its tick, where what went in was lost. Bundles that use the new helpers need `"hooks": 48`.

API 49 adds what a server mod needs for a public server's screens: `native_sidebar(uuid, sidebar)` shows one
player a sidebar of up to 15 lines (a text, and one where the score would be) that only their client holds (the
server's scoreboard never sees it; a mod sends it again when they rejoin), or takes it away with `None`;
`native_tab_list(uuid, header, footer)` sets the lines over and under their Tab list; `native_tick_stats()` reads the
server's tick rate and average tick time in ms (`/tick query`'s); and `native_sleep_ignored(uuid, ignored)` leaves a
player out of `players_sleeping_percentage`'s count until they are in bed (players away from the keyboard), outside a
player's hook only. The players `native_server_players` lists carry where they look (`yaw`, `pitch`). Bundles that
use them need `"hooks": 49`.

API 50 adds damage a mod can change or cancel through `native_living_incoming_damage` (NeoForge's
`LivingIncomingDamageEvent`): where `LivingEntity.hurtServer` starts, once its guards passed (invulnerable, dead or
dying, fire with fire resistance; Horizon's mobs don't check the fire resistance effect there yet), with the hurt
entity's id and type (`minecraft:player` for a player), the damage type (`minecraft:fall`), the source's causing
entity (`causing`, vanilla's `DamageSource.getEntity`: a skeleton for its arrow) and direct entity (`direct`,
`getDirectEntity`: the arrow; the attacker itself in melee), and the amount. The entity ids are 0 for none, and for
the sources listed below that Horizon doesn't track yet. A mod may change the amount, or set `canceled`: nothing of
`hurtServer` itself happens then (no damage, hurt sound, knockback, invulnerable time, statistics or criteria), and
the caller acts as on a refused hit, so a blast still pushes a mob whose hit was canceled, as vanilla pushes one that
refused it. A canceled hit on the ender dragon still counts as landed, as vanilla's `EnderDragon.hurt` returns true
whatever `reallyHurt` did: the arrow is spent and the melee effects run. A ghast's canceled hit by a fireball it was
sent back counts as landed (`Ghast.hurtServer`). A negative amount hurts by 0 and a NaN or infinite one by
`Float.MAX_VALUE`, as vanilla clamps them. It fires for the players, mobs and the ender dragon the game hurts. It has
no `self`: the game is in the middle of the hit. It runs on the thread that ticks the world (the server thread; the
window's on the deprecated in-frame path), so state a mod shares with its render thread hooks needs synchronising. On
the in-frame path the window's player has id 0, or the host's id once the world is open to LAN. `/kill` and
`native_kill_mob` don't reach it yet: they set the health to 0 instead of hurting with `minecraft:generic_kill` as
vanilla does.

A player's damage type is the source's own. A mob's and the dragon's come from the kind of damage Horizon keeps for
the hit, which doesn't tell every vanilla type apart yet:

- A trident's hit on a mob is `minecraft:arrow`, not `minecraft:trident`
- A wither skull's hit on a mob is `minecraft:magic`, not `minecraft:wither_skull`
- Evoker fangs on a mob are `minecraft:magic`, not `minecraft:indirect_magic`
- A fireball nobody shot is `minecraft:fireball`, not `minecraft:unattributed_fireball`
- A shulker bullet's hit has no `minecraft:mob_projectile`
- A bed or respawn anchor blast on a mob is `minecraft:explosion`, not `minecraft:bad_respawn_point`
- A blast from TNT a player lit is `minecraft:explosion` on a mob, not `minecraft:player_explosion`
- A bed or respawn anchor blast on the dragon is `minecraft:explosion`, not `minecraft:bad_respawn_point`
- A blast a mob caused (an end crystal a skeleton shot, a creeper's) is `minecraft:explosion` on the dragon, not
  `minecraft:player_explosion`

The causing and direct entities are 0 for now in these cases:

- A player's direct entity, always
- A player's causing entity, except for a mob that shot the projectile, the mob `/damage` names, an evoker that summoned
  the fangs, a slime or magma cube that touched the player, and a mob that exploded (a sulfur cube's is guessed until
  its blast names its source)
- A mob's causing and direct entities when a player is behind the hit (melee, arrows, tridents, blasts, TNT a player
  lit)
- A mob's direct entity when it isn't the causing one (a projectile, a blast, a potion)
- A falling block on a mob, whose causing and direct entity is the block in vanilla
- The dragon's direct entity for a projectile or a blast
- The dragon's causing player when it isn't the acting one on a LAN guest's turn
- `/damage` on a mob `by` an entity with a damage type that isn't melee: the direct entity is 0, not that entity

And `/damage` on a mob `by` one entity `from` another with a melee damage type gives the `from` entity as the direct
one, where vanilla gives the `by` one.

Bundles that use it need `"hooks": 50`.

API 51 adds `native_menu_button_v2`: a mod container's menu button (`AbstractContainerMenu.clickMenuButton(Player, int)`) with the clicking player's cursor stack, read and written, so a widget outside the slots can take or give what the player carries (a tank taking a bucket). It fires right after `native_menu_button`, on the server, for single player and LAN guests alike. Bundles need "hooks": 51.

API 52 adds `native_block_entity_sync(pos)`, which sends the block and its update tag to clients again
(`Level.sendBlockUpdated`): with the level's block changes, to the players who see its chunk, once per position
however often it is called in a tick. A change made in a block entity's tick reaches clients at the start of the
next tick, as in vanilla. Call it when what clients show changes, not for every tick's progress: a menu's data slots
carry that. Bundles that use it need `"hooks": 52`.

API 53 adds `native_entity_leave_level(dimension, entity, kind)`, an entity leaving a server level (NeoForge's
`EntityLeaveLevelEvent`, vanilla's `onTrackingEnd`): removed for any reason or its chunk unloading, not the entities
still loaded when the server stops. It only hears. It tells item entities and experience orbs so far; check `kind`.
Bundles that use it need `"hooks": 53`.

API 54 adds fluid tanks on a mod block's block entity, and the fluid helpers that read and change them, in millibuckets (a
bucket is a thousand). `native_block_entity_fluids` declares the tanks, as `native_block_entity_container` declares the slots: each tank's capacity, the
tanks each face reaches, and which a fill may reach and a drain may take; the tanks' contents live in the entity's saved compound under
`fluids`, so they survive restarts, and the block's own tick reads them there. `native_fluid_tanks` reads them through a side, `native_fluid_insert`
fills (first into tanks holding the same fluid, then into empty ones) and answers what did not fit, `native_fluid_extract` drains and answers what it
took; both also work a cauldron, which gives and takes whole thirds of a bucket and never more than was asked (less than its top third gives nothing), a lava one only whole, and powder snow none, as it is no fluid. Water and lava
(`minecraft:water`, `minecraft:lava`) are the fluids; a tank holds one at a time. Bundles that use them need `"hooks": 54`.

API 55 adds moving a block's fluids to another block. `native_move_fluids(dimension, from, from_face, to, to_face, max)`
moves up to `max` millibuckets between two blocks, one fluid, and what the target won't take goes back into the
source, past the fill sides' leave: the source's tanks just gave the fluid up, so they have the room, and a
cauldron levels back up by the thirds it gave, which a drain hands out whole. None where `native_fluid_tanks`
is none at the source, where nothing drainable is, where the target holds no fluids, and within one block.
Bundles that use it need "hooks": 55.

API 56 adds what the mob effects of a content mod ([mods.md](mods.md)) do, named after vanilla's `MobEffect` methods,
or NeoForge's event where it has one. On the server, each tick a player or a mob has one,
`native_should_apply_effect_tick_this_tick` says whether it acts (vanilla's base answer is no) and
`native_apply_effect_tick` acts, and may end it, before it counts down, as in `MobEffectInstance.tickServer`. These
two take `&self`: during a mob's tick the mob world borrows the game, so `&mut self` would alias that borrow, and the
game doesn't hold the mobs being ticked, nor the player's effects. `native_on_effect_added` hears it given to an
entity that didn't have it, and `native_mob_effect_added` (NeoForge's `MobEffectEvent.Added`) every time it is given.
A mob is its `Entity.getId`; a player is None, with their state swapped in (`native_acting_player`). Vanilla's effects
never reach them. Bundles that use them need `"hooks": 56`.

API 57 adds chunks and players coming and going, on the server: `native_chunk_load` (with whether the chunk is new), `native_chunk_unload`, and
`native_player_logged_in` and `native_player_logged_out`, which come in pairs however a player leaves, the server stopping included. Bundles that use them need `"hooks": 57`.

API 58 adds [the renderer's hook points](#render-hook-points): `native_world_drawn`, called once the world is drawn with its colour, depth, matrices and entity geometry, and
`native_render_needs`, where a mod asks for the entity runs and the world's depth without a shader pack. Bundles that use them need `"hooks": 58`.

API 59 adds [entities](#entities) for server mods: `native_summon(dimension, kind, pos, nbt)` spawns any entity from its compound
as `/summon` does, and `native_move_entity`, `native_set_display` (a display's transformation, eased over ticks) and
`native_remove_entity` move, pose and remove displays, markers and interactions: models that move, from a mod's own
item models. Bundles that use them need `"hooks": 59`.

API 60 adds `native_item_bar(stack, bar)`: the small bar `GuiGraphicsExtractor.itemDecorations` draws under an item,
wherever the game shows one. `bar` arrives holding
vanilla's choice (the damage bar, or a bundle's fullness, already applied); set it to a width `0..=13` and an
`rgb` colour, or to `None` to hide it. Asked each frame a stack is drawn. Client-side (NeoForge has no event for
this; vanilla's `Item.isBarVisible`/`getBarWidth`/`getBarColor`). Bundles that use it need `"hooks": 60`.

API 61 adds mods' own sound events in `native_play_sound`: an event of another namespace than `minecraft` (`mymod:machine_hum`) is sent to players by
name, as vanilla sends a sound that isn't in its registry, and they hear it when their resources define it (a content mod's `assets/mymod/sounds.json`
and its `.ogg` files). `minecraft:` events must still be registered. Bundles that play their own sounds need `"hooks": 61`.

API 62 adds nicknames for server mods: `native_set_nickname(uuid, Option<&str>)` shows a name in place of a player's in chat (their messages, `/me`, `/msg`) and the Tab list, the real name on hover and everywhere else; `native_text_allowed(text)` asks the server's chat filter. Both outside a player's hook. Bundles need "hooks": 62.

API 63 adds context-free block and item tag lookups from server hooks (`native_server_block_in_tag`, `native_server_item_in_tag`). They return `NotServer` on a client and `TagsUnavailable` before the world loads its tags. An unknown tag returns false; an empty stack tests `minecraft:air`. Bundles need "hooks": 63.

API 64 adds a mod block's comparator output, as vanilla's `BlockBehaviour.hasAnalogOutputSignal` and `getAnalogOutputSignal`: `native_has_analog_output` says a block of yours has one,
and `native_analog_output` gives it (0 to 15) for its state and position. A state change with neighbour updates tells the comparators itself; when the output
changes otherwise, `native_update_output_neighbors(dimension, pos)` makes them read it again (`Level.updateNeighbourForOutputSignal`). Bundles need `"hooks": 64`.

API 65 adds `native_component_types(types)` runs once before content loads. Append namespaced component IDs; duplicates, invalid IDs and the `minecraft` namespace stop startup. `native_component`, `native_set_component` and `native_remove_component` read, set and remove registered components, including item defaults. Values use NBT and must fit the network reader limits. Bundles that use them need `"hooks": 65`.

API 66 adds [explosions](#explosions) for server mods: `native_explode(dimension, center, radius, fire, blast)` sets one off as
`Level.explode` does, with `NativeBlast::Keep`, `Block` or `Tnt` for what it does to blocks. Bundles that use it need
`"hooks": 66`.

API 67 adds dispenser item behaviours (`native_dispense_item`): a server hook receives the dimension, dispenser position, facing and selected stack, before any vanilla behaviour, for mod or vanilla items. Leave its answer `None` for vanilla, or answer `Some((remainder, event))`: a `None` remainder empties the slot, `Some(stack)` writes that remainder without consuming again, and `event` is the level event played with the smoke (1000 click, 1001 fail, 0 nothing, as `NOOP`). Answer `Some((native_dispense_default(dimension, pos, facing, stack.clone()), 1000))` to drop one item normally, including items with a special vanilla behaviour. Droppers are unaffected. Bundles need "hooks": 67.

API 68 adds image tooltip components: `native_tooltip_component(stack, size)` answers finite positive width/height in GUI pixels, and `native_tooltip_component_draw(stack, batch, origin, size)` draws that component with `native_sprite`, `native_fill` and `native_text`. The stack supplies its data. Space and background are reserved after the text and any vanilla bundle component; the component width participates in tooltip positioning. Neither hook changes ordinary tooltips without an answer. Client-side. Bundles need "hooks": 68.

API 69 adds [advancement triggers](#advancement-triggers) of a mod's own: `native_trigger_criterion(uuid, trigger, test)` fires a trigger
an advancement of a content mod or a data pack names (`mymod:pipe_connected`) for one player, as a NeoForge mod's
`SimpleCriterionTrigger.trigger`. Bundles that use it need `"hooks": 69`.

API 70 adds where a mod block can stay, on an authoritative server. `native_can_survive(dimension, pos, state, survives)` answers
`BlockState.canSurvive` before the block does: as it is placed (`BlockItem.canPlace`: `Some(false)` refuses the spot),
from the shape update of a block whose template checks it (a plant, a torch, a sign: `Some(false)` breaks it with its
drops when a neighbour changes), and wherever else vanilla asks. `state` may not be in the world yet, so decide from
`state`. `native_may_place_on(dimension, state, ground, ground_pos, may)` answers `VegetationBlock.mayPlaceOn` for a
mod plant (a block whose template is a vanilla plant, such as a crop): the blocks it can stand on, which its
template's `canSurvive` asks, so it is refused elsewhere and pops off when its ground changes. Answer only for your
own blocks and leave the answer empty otherwise; the level is read-only. Both run on every shape update of such a
block: keep them short. The deprecated in-frame path fires neither. Bundles that use them need
`"hooks": 70`.

API 71 adds drawing mod block entities on the client. `native_block_entity_submit(pos, block, state, entity, draw)` runs once a frame
for each mod block entity in view, as vanilla's `BlockEntityRenderer.submit` does (within 64 blocks, in the frustum,
its chunk faded in), with the copy the server sent (`native_block_entity_update_tag`). It draws into the entity batch
under `draw`'s pose stack (`push`, `pop`, `translate`, `rotate_about`, `scale`, in blocks from the block's corner, as
vanilla's `PoseStack`): `native_draw_quad` (four corners and a sprite's UVs), `native_draw_cube` (a model element's box,
its faces baked as a model's are; `CubeFace` is in `super::native_block_entity_draw`) and `native_draw_block` (a block
state's model, as `submitBlockModel`: a moving part is often a block of the mod's own). Sprites are the atlas's:
`<ns>:block/...` and `block/...`, and a content mod's `assets/<ns>/textures/entity/` as `<ns>:entity/...`. Culling uses the bounds
`native_block_entity_render_bounding_box` gives (NeoForge's `getRenderBoundingBox`): the block with a block of margin
unless the mod widens them, or never culled when a bound is infinite. Both run every frame for every one in view: keep
them cheap. Bundles that use them need `"hooks": 71`.

API 72 adds bone meal on a mod block, the `BonemealableBlock` three questions a block nothing in Horizon knows one for
answers itself: `native_is_valid_bonemeal_target(state, valid)` says whether the block takes bone meal (`Some(false)` refuses it even where its template takes it, `None` leaves the
template's; asked on both sides, as vanilla asks it, so a client predicts the use); `native_is_bonemeal_success(state, success)` is
`isBonemealSuccess`, drawn from `native_level_random()` to keep vanilla's order; and `native_perform_bonemeal(state)`
is the growth itself, with `native_set_block`/`native_set_block_property`. The meal is used up and the growth
particles and sound follow as vanilla's do, whether or not the roll grows the block, from a player's bone meal, a
dispenser's and, on a crop, a villager's. A block that answers nothing keeps the `BonemealableBlock` its template gives it.
Authoritative servers only, except the first question. Bundles that use it need `"hooks": 72`.

API 73 adds mod-block entity contacts: `native_block_step_on`, `native_block_entity_inside` and `native_block_fall_on`. Each receives the server dimension, entity id/type, block state and position. The landing hook also receives fall distance and mutable calculated damage; zero prevents fall hurt and its generic fall sounds. Creative and fall-immune entities notify with zero and retain immunity. Invalid non-finite damage changes are ignored; negative damage is clamped to zero. Hooks run synchronously without `self`, like `native_living_incoming_damage`. Bundles need "hooks": 73.

API 74 adds a consumable's effect of a mod's own kind. The item's `on_consume_effects` entry of a type in a namespace of its own — where the old loader dropped it silently — reaches `native_consume_effect(kind, data)` when the
item is consumed, on the server thread, for the acting player. The kind and the JSON the file wrote are there;
the game's own effects, sounds, teleports and drops are open to the hook. An unclaimed kind does nothing.
The hook sees the item's own consumable component, not a patch on the stack; a bare vanilla name and an unknown `minecraft:` kind are the loader's, not the hook's. Bundles that use it need "hooks": 74.

API 75 adds [loot table hooks](#loot-tables), for changing loot a mod didn't make. `native_loot_table_load(id, table)`, in `horizon-world`'s `loot.rs`, gets each loot table's JSON as it loads, after the data packs' (NeoForge's `LootTableLoadEvent`, Fabric's `LootTableEvents.MODIFY`): add pools or entries, change or replace it, and chests, block and mob drops, fishing and the rest then roll the changed table. It runs wherever tables load, a client too. `native_loot_drops(context, drops)` gets the items a broken block or a dead mob is about to drop, to add, change or take out (global loot modifiers, `MODIFY_DROPS`), with `NativeLootDrops`: the table rolled, the dimension, the origin, the block or the mob, the player who broke or killed it, the tool and an explosion's radius. It hears blocks a player, an explosion or a mob breaks, not other breaks such as leaves decaying. Bundles need `"hooks": 75`.

API 76 adds `native_state_for_placement(state, context, fluid, answer)`: the state a block is about to be placed in
(`Block.getStateForPlacement(BlockPlaceContext)`), `state` the block's own default state. Answer only for your own
blocks: `answer` set to `Some(Some(state))` places that state (a state of that block, as vanilla sets properties on its own
`defaultBlockState`; another block's is ignored), `Some(None)` refuses the placement (vanilla's `null`:
the block goes nowhere and the stack is kept), and left empty the block's own placement runs. `context` is vanilla's
`BlockPlaceContext`: the clicked face, the placer's rotation, the clicked position, whether the clicked block is
being replaced and the item in hand. `fluid` is the fluid in the position the block would go in, which a
waterlogged block reads. It is asked for every placer that goes through vanilla's placement (a player's
`BlockItem.place`, a dispenser's `DirectionalPlaceContext`), and the answer still takes its survival and entity
obstruction checks. `native_state_with_property(state, key, value)` builds the state from `state` and one of its
properties, its own declared ones included. Authoritative servers only. Bundles that use it need `"hooks": 76`.

API 77 adds a `minecraft:post_attack` enchantment effect of a mod's own kind, on a mob a player's weapon hit: `native_enchant_effect` (see
[A mod's own enchantment effect kinds](#a-mods-own-enchantment-effect-kinds)). Bundles need "hooks": 77.

API 78 adds `native_screen_render_post`, the point of `ScreenEvent.Render.Post`: a native mod draws over the open screen after it and its tooltips, given its id, the server-opened menu's vanilla type (`menu`) and title, its container panel, its slot count, the mouse in GUI pixels and the partial tick (`NativeScreenView`). Every container, station and mod window draws on the inventory screen and so reports `inventory`: tell them apart by `menu` and `title` (None on a window the client opened) and by `slots`. It does not fire on the title screen, nor for the prompts, books and sign editors drawn over gameplay after it. Bundles that use it need "hooks": 78.

API 79 adds `native_modify_default_components(item, components)`, in `horizon-world`'s `item_defaults.rs`: an item's default
components in `items.json`'s form, for any item, vanilla's included (NeoForge's `ModifyDefaultComponentsEvent`). Insert
or replace a component to set it (`"minecraft:max_stack_size": 16`), remove it to take it away. The item table,
loot, enchanting and item models all read the changed defaults, and stacks are saved and sent against them as
against vanilla's. Asked for every item each time the game reads the defaults: answer the same each time. Client and
server each ask their own mods, so both need the mod. Bundles that use it need `"hooks": 79`.

API 80 adds the open screen hears of a mouse button going down (`native_screen_mouse_button_pressed_pre`) or up (`native_screen_mouse_button_released_pre`) before it acts, as vanilla's `ScreenEvent.MouseButtonPressed.Pre` and `.MouseButtonReleased.Pre`: the screen, the GLFW button (0 left, 1 right, 2 middle, 3 back, 4 forward; a button past those is the platform's own number, which differs between systems) and GLFW's modifier bits (shift 1, control 2, alt 4, super 8), with the pointer in `screen.mouse`. A cancelled press goes no further (no slot, screen button, drag or hotbar key); cancel a release only when its press was cancelled, or a drag it ended stays held. They fire on the window only, never on the title screen itself (Options and the pack list opened from it do fire), the connect and loading screens, the prompts, books and sign editors drawn over gameplay, or while a world loads or saves, and never for a gamepad or keyboard-focus click. Bundles need "hooks": 80.

API 81 adds [falling blocks](#falling-blocks) for server mods: `native_spawn_falling_block(dimension, state, pos, velocity, falling)`
throws a block that flies, lands and places itself as sand does, for debris and collapsing structures. Bundles that
use it need `"hooks": 81`.

API 82 adds camera shake for client mods: `native_camera_shake(partial, shake)` asks each frame for turns in degrees and an
offset in blocks, which add up across mods and multiply into the view after its bobbing. Horizon scales the sum by
Distortion Effects and bounds it to 10 degrees and half a block on each axis. Bundles that use it need
`"hooks": 82`.

API 83 adds client sounds that wait: `native_play_local_sound(sound, at, volume, pitch, delay)` plays a sound event, vanilla's or
the mod's own from its `assets/<id>/sounds.json`, for this player only and up to a minute later, so a client mod
can time a blast's sound to its shock wave. Bundles that use it need `"hooks": 83`.

API 84 adds values for a mod's own shader pack: `native_shader_params(partial, params)` asks each frame for eight `vec4`s that
every stage of a Horizon shader pack reads as `frame.mods`, so a mod can tell the pack it bundles where and when
something happened (an explosion's centre and age for a cloud drawn in a post pass). Bundles that use it need
`"hooks": 84`.

API 85 adds menus of content mods' own menu types (vanilla's `MenuType`): a mod container's `menu` may name a menu type its content mod declares in `data/<id>/horizon/menu/` with as many slots ([mods.md](mods.md#menu-types)). It opens when the block is used, its data slots are the block entity's `data`, and a Horizon client shows it through the mod's `native_screen`. Bundles that use it need `"hooks": 85`.

API 86 adds `native_block_highlight(pos, state, partial, canceled)` runs before building the eligible targeted block's selection outline, after hidden-HUD and game-mode restrictions. Set `canceled` to suppress only that outline; default behavior is unchanged otherwise. Position is block coordinates and state is the current block state ID. No entity outlines are affected. Bundles need "hooks": 86.

API 87 adds `native_render_hand(hand, stack, partial, swing, equip, canceled)` runs before each visible first-person hand: `Hand::Main` or `Hand::Off`, the displayed stack (None for an empty arm), partial tick, swing progress and inverse equip height including swap animation scale. Set `canceled` to skip the default geometry for that hand (arms and maps included). It runs after the existing spectator, hidden-HUD, sleep and camera gates. It only hides: the hook has no way to draw a replacement. Bundles need "hooks": 87.

API 88 adds `native_client_resources_reloaded()` fires on the render thread after successful client resource-pack application: the atlas, models, renderer and client caches already use the new resources. It covers selection/F3+T and server-pack reloads. Failed staging or application and initial startup do not fire it. This is a completion notification, not a reload-listener registration or staging API. Bundles need "hooks": 88.

API 89 adds `native_item_decorations(stack, x, y, batch)` appends custom GUI drawing after the default bar, cooldown shade and count have been painted for an item icon. The stack is the displayed stack, x/y are its icon's top-left in GUI units, the units the batch takes (no item-local translation remains active). Existing GUI helpers can draw badges, text and overlays. The shared path covers the hotbar, carried stacks, menu slots, recipe book and item tooltip icons, and the item icons of the advancement, beacon, stats and trading screens. `native_item_bar` still controls the bar separately; this hook does not suppress vanilla decorations. Bundles need "hooks": 89.

API 90 adds using an item over time, as vanilla's `Item.getUseDuration`, `getUseAnimation` and `finishUsingItem` and NeoForge's `LivingEntityUseItemEvent.Start` and `.Finish`. `native_use_duration` answers a stack's use time: a mod's own item with no use of vanilla's that answers above 0 is used over time on a right click, until it is let go or the time runs out, on single player, LAN and in-frame alike. `native_use_animation` picks the first person pose of a use (eat, drink, bow, trident, brush), and `native_can_eat` decides whether a food can be eaten now. `native_living_entity_use_item_start` may change a use's duration or refuse it, on the server and where the window's client predicts the use, and on the server `native_living_entity_use_item_finish` may replace what the hand holds once the time ran out (consumables, potions and mods' items). Both take the using entity, None for a player, so mobs can fire them later, and the hand in use. Change the world only where this game runs the world, not on a server's client. Bundles that use them need `"hooks": 90`.

API 91 adds items and block state properties for drawing mod block entities: in `native_block_entity_submit`, `native_draw_item(draw,
stack, display, owner_yaw, seed)` draws an item stack under `draw`'s pose as a campfire's or a shelf's
(`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`, in the model's `display` transform; with `owner_yaw`, the block
entity is the item's owner, as a shelf is, so a compass on it points from there). The client's copy of a block entity has
no item slots: send the stacks you draw in `native_block_entity_update_tag` and read them from `entity.nbt`.
`native_block_state_property(state, key)` reads the block's
`facing` from the hook's `state` without a world lookup. Bundles that use them need `"hooks": 91`.

API 92 adds the world's clock for client mods: `native_clock()` is the shown world's game time and time of day in ticks (`i64`; the day time counts on past 24000), none on the title screen. Bundles need `"hooks": 92`.

API 93 adds the open screen hears of the wheel (`native_screen_mouse_scrolled_pre`) before it scrolls, as vanilla's `ScreenEvent.MouseScrolled.Pre`: the screen, the scroll's x and y as the game's scroll handler scales them (a line times the wheel sensitivity: with Discrete Scrolling on, a notch is ±1, its sign only; a touchpad's pixels are divided by 30), GLFW's modifier bits (shift 1, control 2, alt 4, super 8) and the pointer in `screen.mouse`. A cancelled turn scrolls nothing: no bundle under the pointer, no list, slider or scrollbar, and no hotbar slot. It fires for the window's wheel (horizon-ctl's `scroll-at` included), never on the title screen itself (Options and the pack list opened from it do fire), the connect and loading screens, the prompts, books and sign editors drawn over gameplay, or while a world loads or saves, and never for a wheel a gamepad or the keyboard's focus sends. Bundles need "hooks": 93.

API 94 adds keeping a mod block entity's geometry: `draw.keep()` in `native_block_entity_submit` keeps what that call drew, and the
block is drawn the same again without calling `native_block_entity_submit` or `native_block_entity_render_bounding_box`
until its state, its block entity (what the server sent), its light or the resource packs change, so idle machines cost
no call into the mod each frame. Keep only geometry that reads none of `partial`, `game_time`, the blocks around it or other state of yours:
kept geometry stands still (an enchanted item's glint, a clock or compass drawn with `native_draw_item`).
Bundles that use it need `"hooks": 94`.

API 95 adds any command a player types: `native_player_command(player, name, command, cancel, replies)` is heard before the command is looked at (NeoForge's `CommandEvent`), without its slash, for vanilla's commands, the mods' own and commands the player may not run, once for each command a player sends, on a server. Also each line of a quiet command batch. A mod sets `canceled` to stop it: nothing runs and nothing is answered but the lines it pushes to `replies`, which reach the player as system chat. Bundles that use it need `"hooks": 95`.

API 96 adds `native_neighbor_signal(dimension, pos)`, the strongest redstone signal reaching a block from its neighbours (`SignalGetter.getBestNeighborSignal`, 0 to 15), as a dispenser or a piston reads it: what a mod's explosive, launcher or machine needs to go off or switch on from its block entity's tick. 0 on a client and for another dimension. Bundles that use it need "hooks": 96.

API 97 adds `native_light(pos)` (block and sky light, 0 to 15, both 0 where the chunk isn't loaded), `native_weather()` (rain and thunder levels, 0 in the Nether and the End), `native_day_time()`, `native_world_height()` (min y and height) and `native_max_stack_size(item)`: what a mod's solar panel, wind turbine, quarry or own container needs without reaching into Horizon's internals. Bundles that use them need "hooks": 97.

API 98 adds `native_give_stack(uuid, stack)`: gives a whole stack (damage, name, enchantments kept) to any player from a server hook, as `Inventory.add` places it, and returns what did not fit. Bundles need "hooks": 98.

API 99 adds a mod's command typed on the dedicated server's console: `native_perform_console_command(command, ran)` hears a command a mod registered with `native_register_commands`, without its slash, from the console (permission level 4, no player: `native_acting_player` is None), and a mod sets `ran`. `native_feedback` answers on the console. A name no mod registered goes to vanilla's dispatcher as before; the single player owner's console is not covered. Bundles that use it need `"hooks": 99`.

API 100 adds player-use drops to `native_loot_drops`: shears on a sheep, a mooshroom, a snow golem or a bogged, a brush on an
armadillo or a suspicious block, carving a pumpkin, picking sweet berries or cave vines, and a player taking an item frame or
its item now pass through it too, with `NativeLootDrops::used` set and the player named. Bundles that read `used` need
`"hooks": 100`.

API 101 adds `native_experience_orbs(dimension, min, max)`, the experience orbs in a box; `native_give_experience(uuid, points)`, points given to any player as an orb they touch gives them (Mending first); and `native_give_levels(uuid, levels)`, levels given or, negative, taken. Bundles need "hooks": 101.

API 102 adds lead drops to `native_loot_drops`: untying a lead or cutting it with shears (`used`), and the lead a mob a
player killed while leashed drops on its next tick, naming the killer. Bundles that rely on them need
`"hooks": 102`.

API 103 adds native mod particle types to `native_send_particles`: `ParticleOptions::named("mymod:spark")` sends a particle of the mod's own type, drawn by Horizon clients from the sprite set in the mod's `assets/<namespace>/particles/<path>.json` (`"textures": ["mymod:spark/1", ...]`, resolved as `particle/<texture>` in the atlas). The name must be namespaced; a client without that file draws nothing for it, vanilla clients (whose brand isn't Horizon's) aren't sent it, and the call still returns true. The type behaves as vanilla's `SimpleParticleType` with a sprite set: a plain billboard, no tint, 20 to 40 ticks, its motion the game's default spread (the speed sets its direction, not how fast it goes). Bundles that use it need `"hooks": 103`.

API 104 adds the open screen hears of a key press (`native_screen_key_pressed_pre`) and of each character it types (`native_screen_char_typed_pre`), as vanilla's `ScreenEvent.KeyPressed.Pre` and `.CharacterTyped.Pre`: the screen, vanilla's key name (`key.keyboard.e`, as `native_key` hands it over; a key with no name fires nothing), whether the press is an auto-repeat, the character, GLFW's modifier bits (shift 1, control 2, alt 4, super 8), and `screen.mouse` for the pointer. A cancelled press goes no further — no focus move, no Escape-close, no inventory or hotbar key, no typing by the key itself, no `native_key` — while its release still arrives, so a `native_key` listener sees that release without its press, and the characters no mod cancelled still reach the text box, as vanilla's separate `charTyped` comes for a cancelled `keyPressed` too. A cancelled character drops only that character and never reaches a text box; control characters (Enter, Backspace, Tab, Delete, Escape, a Control letter) are keys and fire nothing. A key's characters fire after the key has acted, as GLFW's char callback comes after its key callback, so a key that closes the screen (the inventory key) reports no character (the closed screen hears none) and one that moves the focus types into the box it moved to. The input method's committed text fires the character hook too. They fire for the window and the control socket's `key` and `type` (not its `\b` backspace), after the fullscreen, screenshot, debug and narrator keys, never on the title screen itself (Options and the pack list opened from it do fire), the connect and loading screens, the prompts, books and sign editors drawn over gameplay, or while a world loads or saves. Bundles need "hooks": 104.

API 105 adds `native_push_player([x, y, z])` on a client, added to the local player's velocity as an explosion's knockback is, and `native_set_flying_speed(player, speed)` for server mods, a player's creative flying speed, kept across respawns and game mode changes and saved with the player as vanilla's `abilities.flySpeed` is. Bundles need `"hooks": 105`.

API 106 adds `native_data_pack_files(files)`, a native mod's own pack files in the data the server loads: while the tables are assembled (a world's first open, and again on every `/reload`), push `(path, bytes)` and they join that load. `path` is the overlay's key, `<kind>/<namespace>:<path>` without `.json`, over the kinds the tables read: `recipe/mymod:thing`, `tags/mymod:block/thing`, `loot_table/…`, `advancement/…`, `trade_set/…`, `villager_trade/…`, `dialog/…`; a file that isn't JSON is skipped with a warning. Asked once per load on both paths, so it is where a mod recomputes them from its config or the world. Contributed files sit below the world's own data packs: a pack's file for the same path still overrides a mod's, and a later contribution overrides an earlier one. Bundles that use it need "hooks": 106.

API 107 adds biome colours for client mods, for seasons: `native_biome_colors(biome, colors)` changes a biome's grass, foliage, dry
foliage and water tints, asked for every biome when a mod calls `native_refresh_biome_colors()`, which rebuilds the
loaded terrain with them. Bundles that use it need `"hooks": 107`.

API 108 adds `native_server_data_loaded(reload)` fires once the server's data tables are built (NeoForge's `TagsUpdatedEvent` with `SERVER_DATA_LOAD`): the recipes, tags, loot and advancements of the world's data packs and of `native_data_pack_files` are what the helpers read from then on. `reload` is false on a world's first open, before its chunks and players are loaded, and true after `/reload`, once the players were sent the new data. It is where a mod drops caches derived from the data and rebinds what depends on it. Server-side, the single player's integrated server included; a client connected to a server never fires it. Bundles need "hooks": 108.

API 109 adds `native_compute_fov(partial, camera, degrees)` receives the computed world FOV after settings, ordinary FOV effects and zoom, before projection/Nausea. `camera` is its world position and `partial` the interpolation fraction. Mods can narrow or widen the angle; only finite values in 1..=179 degrees are accepted, otherwise the original computed angle is used. It runs for world rendering and near-plane overlays, not just once per frame. Hand/HUD FOV and the title panorama keep their existing paths. Bundles need "hooks": 109.

API 110 adds mobs drawn by client mods: `native_mob_submit(mob, draw, drawn)` is asked each frame for every mob in view but players,
with its id, kind, custom name and the frame's body and head turns and walk cycle; a mod draws a model of its own with the
block entity drawing helpers under `draw`'s pose (at the feet, turned with the body) and sets `drawn` to leave out the vanilla
model. Bundles that use it need `"hooks": 110`.

API 111 adds a screen of a client mod's own: `native_mod_screen_open(mod_id)` opens one over the world (every mod's own screen has the `NativeScreenView.id` `mod`: check `native_mod_screen()` is yours; it doesn't pause a single-player game; it opens only on the window's own client, over the world with no overlay up; a gamepad can close it but not click on it), the screen hooks fire over it as over any other, so the mod draws it in `native_screen_render_post` and takes its clicks, scroll, keys and typing from them, and Escape closes it (`native_mod_screen()` says whose is open, `native_mod_screen_close(mod_id)` closes it); and two drawing helpers beside `native_fill`: `native_quad(batch, points, argb)` and `native_item_model(batch, item, model, at, size)`, an icon drawn with another `item_model`. Bundles need `"hooks": 111`.

API 112 adds `native_camera_distance(partial, mirrored, entity, distance)` receives the final scaled maximum distance for a third-person camera, before the existing eight visual-shape collision rays shorten it. `entity` is the camera entity view, including optional living scale/distance and mount scale/distance; this covers player and spectator-mob cameras. `mirrored` identifies front-facing third person. Only finite values in 0..=1024 blocks are accepted, keeping mod requests within the collision-ray budget; other values retain the original distance. First-person views and title panoramas do not fire it. Bundles need "hooks": 112.

API 113 adds `native_compute_fog_color(partial, camera, fog_type, rgb)` adjusts the final world fog RGB after normal biome, weather and effect calculation and before GPU upload. Camera is its world position; fog_type distinguishes atmosphere, water, lava and powder snow. Only finite components in 0..=1 are accepted; invalid requests retain the original RGB. Alpha and all distance fields are unchanged. Title panoramas skip this hook. Bundles need "hooks": 113.

API 114 adds `native_mouse_button_pre(button, pressed, modifiers, canceled)` sees real button events on any screen, before normal dispatch. Button and modifiers use the screen hooks' GLFW encoding. Physical button bookkeeping already ran. Cancellation skips normal dispatch and Post. Accepted events call `native_mouse_button_post(button, pressed, modifiers)` after dispatch, including screen handlers' early returns. Synthetic pad/focus clicks skip both hooks. Existing screen-specific hooks and native_key notifications remain. Bundles need "hooks": 114.

API 115 adds a player respawning: `native_player_respawned(player, name)` is heard once at the end of `PlayerList.respawn`, with the player placed and their health, food and experience reset (NeoForge's `PlayerRespawnEvent`), on a server, with their state swapped in so the helpers reach them. Bundles that use it need `"hooks": 115`.

API 116 adds a client mod's view and items: `native_raycast(distance)` and `native_view_hit()` (the first block along the view, up to 1024 blocks, across the distant terrain past the loaded chunks), and `native_claim_items(mod_id, ids)`: a right-click with one of those items is the mod's alone, the server never hears of it, and `native_poll(mod_id)` gives `UseStart` and `UseStop` with the stack (its custom name and data as SNBT) and the ticks it was held. Bundles need `"hooks": 116`.

API 117 adds `native_render_fog(partial, camera, fog_type, fog)` adjusts final FogData after normal environment/effect/render-distance calculation and Horizon's fog option, before GPU upload. Title panoramas skip it. The whole request is accepted only when color components are finite in 0..=1, all distances are finite, environmental and render-distance ends exceed their starts, and sky/cloud ends are nonnegative. Negative near planes remain valid (water/lava). Invalid requests retain the original whole fog. Bundles need "hooks": 117.

API 118 adds `native_screen_key_released_pre(screen, key, modifiers, canceled)` runs before a named key's release reaches an open screen, with the same screen exclusions and GLFW modifier encoding as the existing key-press hook. It does not fire for presses/repeats or unnamed keys. Cancel bypasses screen handling and its legacy native_key notification, but physical key removal and mapped keyboard/toggle releases still happen to prevent stuck inputs. Mouse mappings retain their existing gameplay-only gate. Bundles need "hooks": 118.

API 119 adds music for client mods: `native_music(music, volume)` changes the track the music manager plays and its volume each
tick, and `native_music_layers(layers)` lists looping tracks played over it, each easing to its own volume (stems that
grow fuller). Bundles that use them need `"hooks": 119`.

API 120 adds a server's recipes reach its Horizon clients, as NeoForge's `OnDatapackSyncEvent.sendRecipes` and `RecipesReceivedEvent`: on the server, `native_datapack_sync(recipe_types)` takes the recipe types (`minecraft:smelting`, `mymod:crusher`) a mod's client half needs, asked once per load, the same types for every player (NeoForge asks per joining player); Horizon sends their final recipes (data packs merged, load conditions held) to every Horizon client when it joins and after each `/reload`, in payloads within the clientbound cap, and nothing to a vanilla client. On the client the recipe helpers (`native_recipes`, `native_recipes_of_type`) then give the server's recipes of those types in place of the window's own, single player included, and `native_recipes_received(recipe_types)` says they came. No type asked: nothing is sent and nothing changes; none asked after a load that asked some: the clients go back to their own recipes and `native_recipes_received` comes with no types. Bundles need "hooks": 120.

API 121 adds `native_input_key(event, modifiers)` runs after a real window keyboard event finishes handling, including shortcuts, screen consumption and repeats. event is the original winit::event::KeyEvent with physical/logical key, location, text, state and repeat; modifiers use the screen hooks' GLFW bits, captured before handling. Not cancellable. Foreign-window and OS synthetic focus events skip it. Window callback only: pad/focus/control-socket input retains its existing native_key route. The legacy native_key hook is unchanged. Bundles need "hooks": 121.

API 122 adds `native_mouse_scrolling(scroll, accumulated, buttons, cursor, modifiers, canceled)` fires with a player in gameplay and no open screen/prompt/sleeping UI, once at least one whole scroll step accumulates and before hotbar/spectator handling. scroll is x/y after discrete/sensitivity scaling (touchpad pixels normalized by 30), accumulated is the whole x/y pair, buttons are physical left/middle/right, cursor is the GUI position, and modifiers use GLFW bits. Cancel skips gameplay scrolling; accumulated whole steps remain consumed. Synthetic input skips the new hook. The legacy vertical native_wheel still runs first and can consume the event; screen-scroll hooks are unchanged. Bundles need "hooks": 122.

API 123 adds `native_screen_mouse_dragged_pre(screen, button, delta, modifiers, canceled)` runs before screen drag handlers for real nonzero pointer movements with a held button. The most recently pressed button is active until it releases; focus loss and splitscreen handoff clear it. Same screen exclusions and GLFW button/modifier encoding as the other screen input hooks. screen.mouse is the new GUI position; delta is movement in GUI pixels. Cancel skips screen drag handlers while retaining the new cursor position. Synthetic input and hover-only moves skip it. Bundles need "hooks": 123.

API 124 adds `native_mod_container_viewers(dimension, pos, viewers, previous)` is heard once each time the number of players with the mod container at `pos` open changes (vanilla's `ContainerOpenersCounter`, as a barrel keeps it): `previous` 0 is `onOpen`, `viewers` 0 is `onClose`, so a block can show itself open while `viewers` is not 0. It is counted every tick from the players' open menus, so a player who leaves without closing (a lost connection, death, another dimension, out of reach, the block gone) is heard too. On a server, with no player's state swapped in. Bundles that use it need `"hooks": 124`.

API 125 adds knockback a mod can change or cancel through `native_living_knockback` (NeoForge's
`LivingKnockBackEvent`): at the start of `LivingEntity.knockback`, on the raw strength, before the entity's knockback resistance takes its
share and its motion changes, with the pushed entity's id and type (`minecraft:player` for a player), the `strength`, the direction
(`x`, `z`; vanilla points it at the source of a hit, the attacker's side, and the entity is pushed the other way) and a
`canceled` flag. A mod may change the three numbers or set `canceled`, and the push follows: canceled, the entity's motion is
left alone. A `strength` of 0 or less after the hook is no push, and a direction too short to tell draws a random one
after it, as in vanilla; the resistance scales the strength the hook leaves, so a full knockback resistance still stops the push. A value a mod
leaves that is not a finite number is put back as it was. `damage_type` (`minecraft:player_attack`), `causing`
(`DamageSource.getEntity`) and `direct` (`getDirectEntity`) are the push's damage source, named as
`native_living_incoming_damage` names a hit's, with the same entity ids left at 0 (a goat's ram pushes with
`minecraft:mob_attack` and hurts with `minecraft:mob_attack_no_aggro`, as in vanilla). It fires for the players and mobs the
game knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a shield's push on its holder and a
goat's ram. It does not fire for a sulfur cube with a block that a hit with a causing entity knocks back
(`SulfurCube.knockback` does not call `super` there), for the pushes vanilla doesn't make with `LivingEntity.knockback` (the
ender dragon's wings, Punch arrows, explosions, wind charges, a mace smash's area, a hoglin's throw), nor yet for the ones
Horizon makes another way: a mob's melee push from its attack knockback or a Knockback weapon, an armor stand's and the
ender dragon's. It has no `self`: the game is in the middle of the hit. It runs on the thread that ticks the world (the
server thread; the window's on the deprecated in-frame path), so state a mod shares with its render thread hooks needs
synchronising.

Bundles that use it need `"hooks": 125`.

API 126 adds `native_edit_blocks(dimension, edit)` for server mods: runs up columns, from a height or a heightmap's, of block states and filters written as commands write them, made at once straight into the chunks of the level being ticked with what `fill ... strict` does for block entities; the chunks changed go to the players whole. Bundles need `"hooks": 126`.

API 127 adds column reads for server mods, in the level being ticked: `native_surface_heights(dimension, columns, heightmap)` and `native_read_columns(dimension, columns, heightmap, (below, above))`, the blocks round each column's height as a palette and indexes with the biome's grass, foliage and water tints. Bundles need `"hooks": 127`.

API 128 adds `native_experience_orb_can_merge(dimension, into, from, into_value, from_value, can_merge)` is asked for each pair of orbs whose boxes meet, when an orb spawns (`ExperienceOrb.tryMergeToExisting`; `from` is 0, the new orb is no entity yet) and every 20 ticks as an orb scans (`scanForMerges`); `can_merge` holds vanilla's `canMerge` answer, which a mod may flip either way. `native_experience_orb_merge(dimension, into, from, from_value, canceled)` follows a yes and may cancel it: both orbs stay, and a spawning orb tries the next orb in reach, then spawns on its own. A merged orb counts as `count` orbs of the keeper's value. `native_set_experience_orb(dimension, id, value, count, age)` writes an orb's `value`, `count` and `age` (each optional; false for a `value` or `count` of 0 or an orb that isn't there). Bundles need "hooks": 128.

API 129 adds `native_block_entity` gives a block a block entity whatever its template: a mod's stone casing or thin part no longer needs a chest's or furnace's template. The engine removes it when its block is broken or replaced, no piston moves it, and a mod that draws it (`native_block_entity_submit`) is asked for it. Bundles need "hooks": 129.

API 130 adds slot rules a mod decides in code for its content menu types: `native_menu_slot_rules(menu, slot, stack, may_place, max_stack)`, in `horizon-client`'s `menu.rs`, is given the type's own answer (false on one of its `results`, [mods.md](mods.md#menu-types), true elsewhere, and 99) and may change both, as a NeoForge mod overrides `Slot.mayPlace` and `Slot.getMaxStackSize`. A mod menu type's clicks run on the vanilla menu engine (`menu::Kind::Mod`). Bundles need `"hooks": 130`.

API 131 adds an item's use tick and release, as vanilla's `Item.onUseTick` and `releaseUsing` and NeoForge's `LivingEntityUseItemEvent.Tick` and `.Stop`. Each tick of a use, `native_living_entity_use_item_tick` may change the ticks left: at 0 or less, or on a cancel, the server finishes the use, as NeoForge's code does, and an item used on release stays frozen until let go. `native_on_use_tick` is what a mod's own item does then (cook, charge). When the player lets go early, `native_living_entity_use_item_stop` may skip the release, and `native_release_using` acts with the ticks left (a bow-like item shoots by its charge); setting `used` starts the item's cooldown. All four run on both sides, as vanilla's do, and take the using entity, None for a player; the tick and stop take the hand too. Change the world only where this game runs the world, not on a server's client. Bundles that use them need `"hooks": 131`.

API 132 adds `native_inventory(uuid, kind)`: a player's inventory (`Player`: 41 slots, main, armour, off hand; `Ender`: 27), live when they are on the server, else from their file, never while they log in or are dead; `native_move_stack((from, kind, slot), expected, (to, kind))`: one stack moved, checked against `expected`, where it fits whole, or nothing moves (at most one side a file, written first); `native_send_map(uuid, map_id, colors)`: a 128 x 128 map picture sent to a player, for a filled map with that `map_id`. What an operator's `/invsee` and a price chart on a map need. Bundles need "hooks": 132.

API 133 adds a loot table's function and condition of a mod's own kind. The entry that names a `type` in a namespace outside `minecraft:` — where the loot roll used to silently skip it — reaches `native_loot_function(kind, data, ctx, stack)` or `native_loot_condition(kind, data, ctx, holds)` when the table rolls. The context brings the entity, the damage, the player, the tool, the block, the luck, the biome and a seed from the roll's random. Both hooks answer in the tables the items engine rolls (block drops, entity drops, loot_table, interact_loot), where entity, damage, player and biome are None; the function hook there gets the stack as the main engine's (its item, count, damage, enchantments, potion and saved components) and takes back what it changes, but a function that asks for a map's structure search changes nothing there. An unclaimed function leaves the stack, an unclaimed condition fails, as unknown kinds do. Bundles that use them need "hooks": 133.

API 134 adds a server's levels opening and closing, on the single player server thread and on a dedicated server: `native_level_load(dimension)` when a level opens, before any of its chunks (NeoForge's `LevelEvent.Load`), and `native_level_unload(dimension)` once it has closed, after its last save (`LevelEvent.Unload`). The level the world opens in comes before `WorldOpenReady`; the Nether and the End open when someone first goes there and close after 300 ticks with nobody in them. Every level still open closes when the server stops, before `WorldCloseFinal`, or when its open or its loop fails, so each load has one unload. Bundles that use them need "hooks": 134.

API 135 adds the client's level made and dropped, on the window: `native_client_level_load(dimension)` when the client makes a level, on joining a server (single player's own included) and on changing dimension (NeoForge's `LevelEvent.Load` from the `ClientLevel` constructor), and `native_client_level_unload(dimension)` when it drops one, replaced by another dimension's (told after the new one's load, as `Minecraft.setLevel` does) or on leaving the server (`LevelEvent.Unload`). Each load has one unload, except when the game itself quits. Bundles that use them need "hooks": 135.

API 136 adds a projectile's impact, which a mod can cancel, through `native_projectile_impact` (NeoForge's `ProjectileImpactEvent`): it fires when a projectile has found what it meets, before the hit acts, with the projectile's id, type (`minecraft:arrow`) and owner, the level, and what it hit: an entity (its id and type) or a block (its position and the face's normal) and the point. Set `canceled` and the impact does not happen: no damage, knockback, sound or block reaction, an arrow does not stick, a pearl does not teleport, a potion or a fireball does not burst. The projectile flies on and is not discarded, so a mod that cancels decides what becomes of it. It covers arrows, tridents, thrown items, fireballs, wither skulls, wind charges, llama spit and shulker bullets, on the single player server thread and on a dedicated server. It does not fire for a breeze's deflection, a hit on a wind charge, fireworks or fishing bobbers. A mod hooks `native_hears_projectile_impacts(hears)` too and sets `hears`: without it the server keeps no copy of its flying projectiles and the impact hook is not called. Bundles that use it need "hooks": 136.

API 137 adds an entity getting on or off a vehicle, which a mod can cancel, through `native_entity_mount` (NeoForge's `EntityMountEvent`): the rider's id and type (`minecraft:player`, `minecraft:zombie`), the vehicle's id and type (`minecraft:oak_boat`, `minecraft:horse`), whether it is mounting, and a `canceled` flag. Set `canceled` and the rider does not board, or keeps riding; a `/ride` that is canceled fails. It is told for a player boarding a boat, minecart, cushion or mount (pig, strider, horse and its kin, llama, nautilus, happy ghast) and getting off by their own input, `/ride`, a stab, a buck, a teleport or spectator mode, and for the mobs Horizon seats on a vehicle (jockeys, raiders on a ravager, `/ride`) and lets off one with `/ride` or a stab. It is not told for a rider whose vehicle is gone or dies, nor for riders loaded with a save. On the single player server thread and on a dedicated server. Bundles that use it need "hooks": 137.

API 138 adds `native_interaction_key(button, key_mapping, hand, swing_hand, canceled)` fires while playing when an interaction key is about to act on the crosshair: button 0 the attack key (`key.attack`; when an attack is allowed, and each tick a held key keeps digging a block), 1 the use key (`key.use`; once for each hand tried, main first, before the hand's target and item), 2 pick block (`key.pickItem`; only with a block or entity aimed at). Set `canceled` to skip the action; a cancelled use ends the click, so the other hand gets no turn. A cancelled attack or use still swings `hand` unless the mod clears `swing_hand` (true on entry). A single player's window and a server's player (any window that plays on a server); the deprecated in-frame path is not covered. Bundles need "hooks": 138.

API 139 adds `native_gui_layer_pre(layer, batch, canceled)` and `native_gui_layer_post(layer, batch)` fire around each vanilla HUD layer Horizon draws, in its paint order, so a mod places its own drawing below a layer (in `pre`) or above it (in `post`), NeoForge's `RegisterGuiLayersEvent` registerBelow/registerAbove, and hides a layer by setting `canceled` in `pre` (`RenderGuiLayerEvent.Pre`). Layer ids: `minecraft:crosshair`, `hotbar`, `effects`, `boss_overlay`, `vehicle_health` (riding a mount), `contextual_info_bar` (the jump bar, locator bar or experience bar with the level number), `player_health` (health, armor, food and air rows, one pass), `selected_item_name`, `sleep_overlay`, `overlay_message` (action bar), `title`, `scoreboard_sidebar`, `subtitle_overlay`, `chat` (the unfocused lines) and `tab_list`. F1 hides the layers it hides in vanilla, and then nothing fires for them. The camera overlays (drawn by the screen-effects pass) and the debug screen are not layers. A hidden layer still runs its game logic (the health rows' blink, the bubble pop sound) and `post` still fires. Bundles need "hooks": 139.

API 140 adds `native_map_decorations(map, decorations)` is asked each time a map is drawn (held, in an item frame, or on the cartography table's screen) with `NativeMapInfo` (id, centre block, scale, dimension, locked, whether it is in a frame); push `NativeMapDecoration { sprite, x, y, rot, shown_on_frame }` (an atlas sprite, a position in half map pixels from -128 to 127, a rotation 0 to 15, and whether it shows in frames and on the screen like vanilla's markers) and it is drawn after the vanilla decorations with the same pose; a sprite the atlas lacks is left out. Nothing is saved in the map and vanilla's decorations are not passed in. This is the client half of NeoForge's `MapDecorationType`; types registered on the server and saved in map data need code-side registration. Bundles need "hooks": 140.

API 141 adds a player's client starting and stopping tracking an entity, on the single player server thread and on a dedicated server: `native_player_start_tracking(player, name, entity, uuid, kind)` when the entity is sent to the player (NeoForge's `PlayerEvent.StartTracking`, vanilla's `ServerEntity.addPairing`) and `native_player_stop_tracking(...)` when it stops (`PlayerEvent.StopTracking`, `removePairing`): out of range, gone from the level, or the player left, changed level or respawned. They name the entity by ID, UUID and type and run with no player's state swapped in, so a mod sends what the player needs with `native_net_send_to_player`. Each start has one stop. Bundles that use them need "hooks": 141.

API 142 adds an energy buffer on a mod block's block entity (`Capabilities.Energy.BLOCK`), and the helpers that read and change it. `native_block_entity_energy` declares the buffer as `SimpleEnergyHandler` is built: its capacity, the most one insert and one extract moves, and the faces a fill may come through and a drain may leave by. The amount lives in the entity's saved compound as the int `energy`, so it survives restarts and the block's own tick reads it there. `native_energy(dimension, pos, face)` reads it through a side; `native_energy_insert(pos, face, amount, simulate)` and `native_energy_extract(dimension, pos, face, amount, simulate)` answer the amount moved (`simulate` changes nothing); `native_move_energy(dimension, from, from_face, to, to_face, max)` is `EnergyHandlerUtil.move` between two blocks and answers the amount moved. Bundles need "hooks": 142.

API 143 adds data maps (`RegisterDataMapTypesEvent`). `native_data_map_types(types)` pushes the types a mod registers: an `id` (`mod_id:name`, not in `minecraft`) for a `registry`, `minecraft:item` or `minecraft:block` so far; asked once per load of the server's data (a world's first open and every `/reload`). The values come from `data/<mod_id>/data_maps/<registry path>/<name>.json` in the world's data packs and from files a mod pushes through `native_data_pack_files` under the key `data_maps/<mod_id>:<registry path>/<name>`: `{"replace": false, "values": {"minecraft:coal": ..., "#minecraft:logs": ...}, "remove": [...]}`, an entry's value being the data itself or `{"value": ..., "replace": false}`; `replace` clears what earlier files set, a tag gives every entry in it the value, `remove` applies after the file's own values, and a newer value overrides an older one. `native_data_map(registry, type, entry)` reads an entry's value (the JSON the file held) and `native_data_map_entries(registry, type)` lists them. Server side only and not synced to clients; mergers, conditions, other registries and mod datapack registries are not here yet. Bundles need "hooks": 143.

API 144 adds a player's or mob's breathing, on the single player server thread and on a dedicated server: `native_living_breathe(entity, kind, can_breathe, consume, refill)` every tick for a player and for a mob with the generic air logic (NeoForge's `LivingBreatheEvent`, vanilla's `LivingEntity.baseTick`), with Horizon's answers as the defaults: set `can_breathe` and how much air `consume` takes or `refill` gives (up to 300); at -20 the entity is set to 0 air and takes 2 drowning damage. It is a free function in `horizon_client::breathing`, so it answers and cannot call helpers. Axolotls and `WaterAnimal`s keep their own air logic. Bundles that use it need "hooks": 144.

API 145 adds blocks and items a native mod declares in code, as NeoForge's `DeferredRegister.Blocks` and `.Items` drained by `RegisterEvent`, instead of one content file each: `native_blocks(blocks)` and `native_items(items)`, asked once at start on every side, take `horizon_mods::content::Block` and `Item`, the declarations a content mod's `data/<id>/horizon/block/` and `item/` files hold (a vanilla template and what differs; `item: true` gives a block its item). Ids are in the mod's namespace. They join the content mods' blocks and items, inside the content mod whose id is that namespace when it is on (its models, textures, loot and recipes), else on their own with the template's look, so their network ids, saves and the joining client's content check are the files' (docs/mods.md). They pass the files' checks; a namespace whose declarations don't is logged and left out. Bundles that use them need `"hooks": 145`.

API 146 adds entities ticking, through `native_entity_tick_pre` (NeoForge's `EntityTickEvent.Pre`) and `native_entity_tick_post` (`EntityTickEvent.Post`), with the entity's id and type (`minecraft:zombie`, `minecraft:item`) and the level. Set `canceled` in the Pre and the entity doesn't tick this tick (and its Post isn't told). Told for the mobs, item entities, minecarts and boats that tick in an entity-ticking chunk, on the single player server thread and on a dedicated server; not for a player's own boat or minecart, projectiles, orbs, primed TNT, falling blocks or armor stands yet (check `kind`: later APIs add kinds). Mobs and items hear every Pre before the family's first tick and every Post after its last; minecarts and boats hear theirs around each one's tick. A mod that hooks them also hooks `native_entity_tick_listened(listened)` and sets `*listened = true`: otherwise no entity's tick is told. It runs each tick for every ticking entity, so keep it cheap. Bundles that use them need "hooks": 146.

API 147 adds a fluid handler on items (`Capabilities.FluidHandler.ITEM`). `native_item_fluid_tank(item, capacity)` makes a mod item a container with a capacity in millibuckets; what it holds lives in the stack's `minecraft:custom_data` under `horizon_fluid` (`Fluid`, `Amount`), so it is saved, sent to Horizon clients and stacks only with equal contents. `native_item_fluid(stack)` reads one item of a stack (vanilla's buckets are built in: empty, water and lava, a whole bucket or nothing); `native_item_fluid_fill(stack, fluid, amount)` and `native_item_fluid_drain(stack, max)` answer how much one item takes or gives and the single item it becomes, as `IFluidHandler.fill`/`drain` through an `ItemAccess`; the stack given is not changed, so the caller shrinks it by one and hands out the result. They work on any side. Bundles need "hooks": 147.

API 148 adds a mob's or an armor stand's equipment changing, on the single player server thread and on a dedicated server: `native_living_equipment_change(entity, kind, slot, from, to)` once for each slot (`mainhand`, `offhand`, `feet`, `legs`, `chest`, `head`, `body`, `saddle`) that holds another stack than the mods last heard (NeoForge's `LivingEquipmentChangeEvent`, vanilla's `LivingEntity.collectEquipmentChanges`). A mob new to the level counts from empty on its first tick. A mod that hooks it also hooks `native_living_equipment_listened(listened)` and sets `*listened = true`: otherwise the game does not read every mob's equipment each tick, and only armor stands are told. Players keep `native_on_equip`. It only hears. Bundles that use it need "hooks": 148.

API 149 adds `native_player_submit(player, who, draw, drawn)` fires once a frame for each other player in view and for this window's player in third person (not while invisible), with the `native_mob_submit` frame (`NativeMobRender`, a `BlockEntityDraw` at the feet turned with the body) and `who` (profile name, whether it is the local player); set `drawn` to leave out the vanilla model, armour, held items, cape and other layers (`RenderPlayerEvent.Pre`). `native_mob_layer(mob, who, draw)` fires once a frame after the vanilla model and layers of every mob and player in view that a mod did not draw itself, and what the mod adds in that frame is drawn with it (`EntityRenderersEvent.AddLayers`, `RenderLivingEvent.Post`); `who` is set for a player. Geometry attaches to the entity's frame, not to the model's bones: head yaw, pitch and the walk cycle are in `NativeMobRender`. A mod that hooks either also hooks `native_player_render_listened(listened)` and sets `*listened = true`: otherwise neither is asked. Bundles need "hooks": 149.

API 150 adds a player finishing a sign: `native_sign_changed(player, name, dimension, pos, front, lines, cancel)` is heard when a player sends the text they wrote on a sign they were editing, right before it goes on the sign, once the checks have passed, on a server, with the player's state swapped in (`native_tell` reaches them). A mod may rewrite `lines` (four lines of plain text) or set `cancel` so the sign keeps its old text; a waxed sign is not told. Bundles that use it need `"hooks": 150.

API 151 adds models no blockstate names, as NeoForge's `ModelEvent.RegisterStandalone`: `native_standalone_models(models)`, asked once at start, takes the ids of models a block entity or a mob draws as parts (`mymod:block/shaft_half`, from `assets/mymod/models/block/shaft_half.json`), and every resource load, a pack reload included, bakes them beside the block models. `native_draw_model(draw, model, tint)` draws one under `draw`'s pose in `native_block_entity_submit` or `native_mob_submit`, as `native_draw_block` draws a block's model, with `tint` (0xRRGGBB) on its tinted faces. A model that doesn't read is logged and left out, and drawing it gives false; no block has to be registered just to bake a part's model. Bundles that use them need `"hooks": 151`.

API 152 adds a player opening and closing any menu on a server, on the single player server thread and on a dedicated server: `native_player_container_open(player, name, dimension, menu, pos)` and `native_player_container_close(...)` (NeoForge's `PlayerContainerEvent.Open` and `.Close`), with the menu type's name and, for a block's menu, its position. They run with the player's state swapped in. Unlike the viewer count of a mod container (`native_mod_container_viewers`) they name the player and cover every menu: chests, furnaces, crafting tables, trades, a mod's own. A menu is heard once a tick, so one opened and closed inside a tick is not; each open has one close, also when the player leaves. Bundles that use them need "hooks": 152.

API 153 adds a player starting or stopping flight: `native_player_toggle_flight(player, name, flying, cancel)` is heard once for each change a player's client makes to its flying (vanilla honours it only for a player who may fly), on a server, with the player's state swapped in. A mod sets `cancel` to refuse it: the player keeps the state they had and the server sends their abilities back, so the client lands or flies again. Only what a client asks for is heard, not the flight a game-mode change starts or ends, and not a LAN host's own player. Bundles that use it need `"hooks": 153`.

API @API:physical_window_size@ adds `native_physical_window_size()`, the window's width and height in physical pixels, independent of GUI scale. It returns no size on a server or when no window exists. Bundles that use it need `"hooks": @API:physical_window_size@`.

## What the launcher does

In the window: install a native mod from the Mods page, into the instance picked on the left ("Installs into
{instance}", shown even with one instance). The first one shows a trust notice once (native mods aren't sandboxed; the
first build downloads Rust and can take 20 to 30 minutes; later builds are faster); `mods install` prints it too. Each
installed native mod's row says "Installed in {instance}" and has **Open folder** (`<instance>/horizon-native-mods/`).
The installed list has a **Native build of {instance}** row for the instance picked on the left, never another's:

- a badge: **Not built** (the version it plays isn't a `native-*` build), **Needs rebuild** (the `mods` in its
  `native-mods.json` differ from those turned on, by id and version) or **Up to date**, with **Build** for the first
  two;
- for a native build, "Plays native-…, built on Horizon {version}" (the `base` of its `native-mods.json`);
- **Rebuild on Horizon {version}** when a newer release or Nightly is installed (`native_mods::update_for`). Build
  stays on the Horizon the instance plays; only that button moves it, and its warning says the build starts from scratch
  and takes longer.

The instance list on the left shows such an instance as "Native build · Nightly 10-28" (its base). The launcher names
the mods, warns in red, and builds only from "I trust these mods: build"; the row shows each step and the error if
one fails, on the instance being built only; one build runs at a time. From the command line:
`horizon-launcher native-build [FOLDER] [--instance NAME] [--update] --trust-native-code`. FOLDER is a bundle, an
unpacked `.hnmod`, or a folder of them; without it, the native mods turned on in the instance. It is not an MCP tool:
trusting native code is the player's decision, never an agent's.

1. **Rust.** The launcher's own toolchain ([`rust_toolchain`](../launcher/src/rust_toolchain.rs)): Rust 1.99.1's
   official `rustc`, `cargo` and `rust-std` (and `rust-mingw` on Windows), each pinned by SHA-256, downloaded once
   from static.rust-lang.org into `<launcher data>/rust/`, with its own Cargo home `<launcher data>/cargo/`. No
   rustup, nothing on the system; the player's own Rust is never used. It ships for macOS on Apple Silicon, Linux
   x86-64 and Windows x64; an Intel Mac has none yet, so native builds there stop with "the launcher has none for
   macos x86_64". Windows uses the `windows-gnu` host, which links without Visual Studio. `rust-mingw` only links, so on Windows the toolchain also brings MinGW-w64
   (`rust_toolchain::MINGW`: WinLibs' GCC 16.2.0 on MinGW-w64 14.0.0, MSVCRT like `rust-mingw`, one zip pinned by
   SHA-256, its `mingw64/` without the documentation) on the build's `PATH`: `ring` and `hidapi` compile C,
   and `windows-sys`' `raw-dylib` imports need `dlltool` and the assembler it runs. The linker is the system's on
   macOS (the Command Line Tools: the launcher asks for `xcode-select --install`) and Linux (`cc`). On Linux the
   game also builds against the system's ALSA (`alsa-sys`) and libudev (`libudev-sys`, `hidapi`), found with
   `pkg-config`, whose development files desktop distributions don't install. Before downloading anything the
   launcher checks for `cc`, the C library's headers (`cc` compiles an include of `stdint.h`), `pkg-config`,
   `alsa` and `libudev` and names what is missing, with the install command for the distribution in
   `/etc/os-release` (Debian, Ubuntu and Mint:
   `sudo apt install build-essential pkg-config libasound2-dev libudev-dev`). SteamOS has GCC without the C
   library's headers, and a read-only system: native builds don't work there yet.
   `horizon-launcher rust status|install|remove`.
2. **horizon-mixins**, at the commit the mods name (their `.hnmod`'s `mixins`), else `native_mods::MIXINS_REV`,
   built once with that cargo (`cargo install --locked --git … --rev …`) into `native/tools/<commit>/`.
3. **The source** of the pinned commit: GitHub's archive of it, once per commit (`native/source/<commit>/`, about
   63 MB unpacked), with the commit in `.horizon-revision`. Never the player's own checkout. Links in the
   archive are left out: the tool refuses them and nothing needs them to build.
4. **Mixins.** The bundles are copied first, so the build and its key use that copy whatever happens to the
   folder meanwhile. `horizon-mixins apply` stages the modded source; the launcher copies only the files that
   changed into `native/stage/`, so Cargo, which goes by file times, recompiles only the crates the mods touch.
   With mods' crates, their dependencies are checked against Horizon's `Cargo.lock` ("The mod's crate").
5. **Compile**: `cargo build --locked --profile fast [--features horizon-client/mods] -p horizon-client`, into
   `native/target/`. `fast` is the workspace's own profile: the release's optimizations without its link-time
   ones (`lto = false`, `codegen-units = 16`, incremental), so a rebuild recompiles and links only what changed.
   A revision whose `Cargo.toml` has no `fast` builds with `release`, the release's own command (game-build.yml);
   `native-mods.json` says which (`profile`). The Mods page's "Native build" row → "Release build: slower to build,
   slightly faster game", `horizon-launcher settings native-profile release` or `native-build --profile release`
   compile the release's own profile instead (thin LTO). The profile is in the build's key and each has its own
   folder (`native/target/fast/`, `native/target/release/`), so the two never mix and may both stay. The mods feature
   is passed while the revision still has it. Cargo downloads the locked crates from crates.io (Horizon has no git
   dependencies). The compiler and the horizon-mixins build run at the lowest priority (`rust_toolchain::low_priority`:
   nice 19 and the idle I/O class on Linux, nice 19 on macOS, below-normal priority on Windows), so the desktop and a
   game in play stay smooth; on an otherwise idle computer they still use every core. The log is `native/build.log`;
   a failure shows its last lines (a horizon-mixins diagnostic in one line).
6. **Post-build**: each mod's post-build program is built; it runs in the new version folder once that is copied
   (next step), before the version counts as installed ("Post-build steps").
7. **Install**: a new version, `native-<commit>-<key>`, a copy of the starting version's folder (resources
   included, cloned on APFS, Btrfs and ReFS) with the built game in place of its own, and a `native-mods.json`
   saying which release or Nightly it started from (`base`) and which commit, mods, horizon-mixins and Rust made
   it. The instance the mods are installed in plays it, the default **Horizon** instance included: the build
   pins its version, as picking a version does. Updates of the launcher's version leave a pinned instance where
   it is, and their clean-up never removes a native build. An instance built before this (the **Native mods**
   instance the default home's mods used to go to) keeps its worlds and its build.

## Rebuilds stay on the instance's Horizon

A build starts from the version the instance plays (`native_mods::base_for`). When that is a native build, the
next one compiles the same commit (its `native-mods.json`'s `horizon`) from the same game files, unless a mod pins
another, so a rebuild after a mod changes or is switched off compiles only what changed. A newer Horizon is taken
only when the player asks: `native-build --update` starts from the newest installed release or Nightly (a full
rebuild); `native_mods::update_for` says which one, if any, the window can offer.

The compiler runs as the release's build does, whatever the player's own Rust setup: the launcher's toolchain
drops the variables that change compiler flags or the target (`RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`,
`RUSTDOCFLAGS`, `CARGO_INCREMENTAL`, `CARGO_PROFILE_*`, `CARGO_BUILD_*` but `CARGO_BUILD_JOBS`, `CARGO_TARGET_*`,
rustup's and the wrappers'), and starts Cargo from the disk's root with `--manifest-path`. Cargo reads `.cargo/config.toml` from
the folder it starts in and every one above it, whatever `CARGO_HOME` says, so from the launcher's data folder it
would read the player's `~/.cargo/config.toml`, whose `[profile]`, `[build]` and `[target]` settings beat the
workspace and the environment; no switch turns that search off. The source's own `.cargo/config.toml`, if it
has one, is passed with `--config`.

Every path the build's tools see — the toolchain, the Cargo home, `native/` and the tools' temp dir (TMPDIR,
TMP and TEMP, where gcc, the assembler, `ld` and `cargo install` put their scratch files) — comes from
`backend::build_dir`: the launcher's data folder, resolved (never through a link, which horizon-mixins
refuses). On Windows, when that path is not spelled in ASCII — a Cyrillic user name in `%APPDATA%`, which
MinGW-w64's tools can't open — it is a real folder under `ProgramData` named for it (`horizon-launcher-<id>`),
and only when that can't be created the data folder's short (8.3) name. A short name alone is not enough
(#6009): rustc canonicalizes its own path to find its sysroot, which spells the long name again for the
linker. A data folder that is itself a junction to a drive with an ASCII path resolves to that and needs
neither.

## Install from GitHub

An author can ship a native mod without a registry pull request per version: the player installs it from a
GitHub link, on the Mods page ("Add from GitHub") or with `horizon-launcher mods add-github <url>
[--mod ID | --all] [--instance NAME] --trust-native-code`. The link is `https://github.com/<owner>/<repo>`
(its latest release), `.../releases/tag/<tag>`, `.../tree/<ref>` or `.../commit/<sha>`.

- **Resolved to something exact.** GitHub's public API (no sign-in; a rate limit is said plainly, and
  `HORIZON_GITHUB_TOKEN` raises it) turns the link into a release's `.hnmod` file or a **commit SHA**, never a
  branch that moves. A release with a `.hnmod` asset installs that file (its SHA-256 is compared with the one GitHub
  lists). With no release or no such asset, the launcher fetches
  `codeload.github.com/<owner>/<repo>/tar.gz/<sha>` and looks in it for mods by the rules of
  `native-build <folder>`: a folder with `horizon-mixin.toml`, or an unpacked `.hnmod` (`horizon-native.json`).
  Several mods: they are listed, and the player picks one or installs all. A source folder must pin the Horizon commit
  (`[compatibility] horizon_revision`); the launcher packs its Rust, TOML and `data/` files into a `.hnmod`.
  A release that ships only a content mod (`.hmod`) says "not supported yet": installing `.hmod` files from a link
  is not built.
- **Checked like the registry's files.** The `.hnmod` is read as `horizon_mods::index::native::Bundle::read` reads a
  registry file (only Rust, TOML and data under `mixins/<id>/`, size limits, nothing run) before it is installed.
- **Said first.** Before anything is downloaded: "not reviewed by the Horizon registry; native mods run with full
  access to this computer; read the source", with a link to the exact commit. The command needs
  `--trust-native-code`, as `native-build` does. It is not an MCP tool: trusting native code is the player's
  decision, never an agent's.
- **Origin kept.** `horizon-native-mods/<id>.origin.json` beside the `.hnmod`: owner/repo, release tag (for a
  release), commit, asset name and the SHA-256 of the installed file. `mods list` shows "from GitHub: owner/repo @
  tag" (or the commit's first 7 digits).
- **Updates.** `horizon-launcher mods update-github [ID] [--check]` compares a mod from a release with the
  repository's latest release, and a mod from a commit with the head of the default branch; a newer one is
  installed with the same warning and `--trust-native-code`. `mods remove <id>` removes it. A mod already
  installed from the registry (or by hand) is never replaced.

Installing turns Native mods on, as any native mod does; then build as usual.

## Cache and disk

The key hashes the commit, the horizon-mixins commit, Rust's version, the profile and features, and every file of
every bundle. The same set on the same commit is never built twice. A change gives a new build, and
`native/target/` stays, so a rebuild compiles only `horizon-client` and what the mods touched.

**Incremental rebuilds.** With `fast`, Cargo recompiles only the changed parts of `horizon-client`, not the whole
crate with LTO. Patched source is cached separately by its mixin inputs and pins (the three most
recent snapshots are kept), so editing only a trait crate skips the mixins pass. This holds because
the stage (`native/stage/`) is one folder for every build and the launcher writes
only the files whose content changed (Cargo goes by file times), switching mods on the same commit reuses the same
`native/target/`, and the launcher removes `CARGO_INCREMENTAL` and `CARGO_BUILD_INCREMENTAL` from the build's
environment. `release` has no incremental compilation (the release profile's own).

**Build status.** Before it starts, the build says which case it is, with the profile: "Up to date (no build needed)",
"Incremental rebuild: mods X, Y" (or "Horizon commit a to b", "horizon-mixins"; compared with the last build of that
profile, kept in `native/built.json`) or "First build: the whole game (several minutes)". While compiling it shows the
crate, crates done of the total (Cargo's own progress), and the time taken; afterwards "Built in 42 s (fast,
incremental)". The window shows it in the Native mods row, `native-build` prints each step with its time on standard
error; `native/build.log` keeps Cargo's output as before (without the progress bar).

Measured 2026-10-30 on Linux (x86_64 desktop), Zoom 2.0.0 and FPS Counter 2.0.0 on Nightly `6f8c7a0`, the `fast`
profile, the launcher at nice 19 with `CARGO_BUILD_JOBS=10` (an empty launcher home: Rust, the crates and the
source downloaded first):

| What | Time |
|---|---|
| First build: Rust, horizon-mixins, the whole game (384 crates) | 4 min 29 s in all; 3 min 6 s of compiling the game; `native/target/` about 2 GB |
| One mod switched off, rebuild | 56 s in all, 51 s of compiling (only `horizon-client`) |
| That mod switched on again | under 1 s: the kept build plays again |
| Nothing changed, rebuild | 1 s, nothing compiled |

The test bundle `native-crate-demo` (its own crate on `crc32fast`, data files, the `frame` hook by name, a
post-build step), measured the same way on 2026-10-30 (Rust already downloaded):

| What | Time | Compiled |
|---|---|---|
| First build (horizon-mixins, the crates, the whole game) | 3 min 27 s; 2 min 24 s compiling the game | 343 crates |
| A helper's code changed | 48 s (43 s compiling) | `horizon-client` only: the mod's crate is kept |
| The mod's crate changed | 11 s (6 s compiling) | `crate-demo-engine`, then `horizon-client` |
| Nothing changed | under 1 s | nothing |

The same set with the `release` profile took 7 min 6 s for the first build and 2 min 34 s per rebuild (2 min 28 s of
compiling `horizon-client`, mostly its link-time optimization). In play, a `fast` game drew the same frames per
second as the Nightly's own `release` build (vsync-bound, about 112 FPS at 120 Hz), with about 0.1 ms more CPU per
frame (0.57 to 0.68 ms against 0.51 to 0.54 ms).

Measured before builds used `fast`, with the `release` profile, on 2026-10-28 on the Mac mini (M4 Pro, 14 cores) with Veinminer 0.3.0 on Nightly `83f15fa`,
`CARGO_BUILD_JOBS=8` while other agents' builds ran (upper bounds):

| What | Size, time |
|---|---|
| Rust 1.99.1 (`<launcher data>/rust/`) | 453 MB; 58 s to download and unpack |
| horizon-mixins (`native/tools/`) | 7 MB; built once in 19 s |
| Crates in the launcher's Cargo home (`<launcher data>/cargo/`) | 289 MB |
| Source of one commit, and the stage | 64 MB each |
| First build: the whole game | 9 min 10 s of compiling, 9 min 41 s in all; `native/target/` 1.5 GB |
| Rebuild after a helper's code changed | 2 min 46 s of compiling (only `horizon-client`), 2 min 53 s in all |
| A change horizon-mixins ignores (a comment) | nothing compiled; 31 s, mostly the version folder's copy |

Measured 2026-10-29 on Windows 11 (Ryzen 7 9800X3D, 16 threads), the same mod and Nightly, an empty `native/target/`
(horizon-mixins, the source and the crates were already downloaded):

| What | Size, time |
|---|---|
| Rust 1.99.1 and MinGW-w64 (`<launcher data>/rust/`) | 1.7 GB, MinGW-w64 875 MB of it |
| Crates in the launcher's Cargo home | 657 MB |
| First build: the whole game | 6 min 57 s of compiling, 8 min 3 s in all; `native/target/` 2.0 GB |

Measured 2026-10-28 on Linux (an x86_64 server with no Rust of its own), Zoom on Horizon `1149c92`, 3 jobs while CI
ran: `rust install` in 11 s (592 MB), the first build in 12 min 9 s. The built game ran Zoom's proof (under `xvfb-run`:
the game needs a display even with `--offscreen`).

The native builds an instance plays are kept, and the three newest no instance plays (`native_mods::prune`, by when
each was built or last used): switching a mod off and on again plays the kept build again without compiling.
`native/target/` stays for the next one.

## Trust

- **A notice, then a warning.** The first native install shows a trust notice once. The command needs
  `--trust-native-code`; the window
  shows a warning in red before every build: the mods run with full access to the computer, and their ids and
  versions. The Mods page says who made each one, and "Not made or endorsed by the Horizon project" for any mod not
  Horizon's own.
- **The registry** (horizon-rust/horizon-mods) lists them as their own kind, `native-mod`, only after a maintainer
  has read the source and the repository they come from. Its entry names the GitHub repository (`repo`); each
  release of it with a `.hnmod` asset is a version once the bundle passes the registry's checks, with no further
  review (nothing is built or run by the registry; the player's launcher builds it behind its warning). The `.hnmod` contains `.rs` and `.toml` files and may contain data files under
  `mixins/<id>/data/` or a crate's `data/`. The launcher checks the bundle without running anything. Native code is trusted.
  The file is pinned by SHA-256 like every registry file, and the version, game and pins are the bundle's own
  `horizon-native.json`. `revoked.toml` takes a bad release down.
  A content mod's entry may turn `native-mod` once (never back): its `.hmod` versions stay published and
  are checked as `.hmod`s, the index lists only its versions with native code, and the launcher's update keeps the
  player's `.hmod` (the game runs it until a native build has the mod, then skips it); removing the mod removes both.
- **Isolation of data.** Native mods run only in the instance they are installed in, with its worlds, options and
  logs (the default Horizon instance's, when they are installed there); other instances play their own versions.
- What this does not do: sandbox the compiled game. horizon-mixins' checks stop mistakes, not a malicious helper
  ([security notes](https://github.com/horizon-rust/horizon-mixins/blob/main/docs/security.md)).

## Multiplayer

- **Single player and LAN host:** the mods run in the host's game and its integrated server, so a server-side
  hook applies to every player of the world.
- **Joining a server:** only client-side hooks do anything; the protocol is unchanged.
- **Dedicated server:** native mods run in a standalone engine export built with `--server-output`, not in an
  instance build (`--server-output` conflicts with `--instance` and needs an explicit mod `FOLDER`). See
  [Standalone server export](#standalone-server-export).

### Standalone server export

`horizon-launcher native-build FOLDER --server-output DIR --version TAG --trust-native-code [--profile fast|release] [--allow-version-mismatch]`

Builds a standalone dedicated-server engine into `DIR`, which must not exist yet. `FOLDER` (a mod folder or a
folder of them) is required; no instance is read, selected, edited or played, and no launcher config is written.
It reuses the pinned toolchain, source and horizon-mixins and compiles `--no-default-features` (no audio or
controllers), keeping the mods feature when the revision has it. `--version` pins the Horizon release/Nightly the
export reuses (explicit for reproducibility). The export holds the built `horizon-engine` executable, prepared resources,
`native-data/` and `native-mods.json`, and prints its absolute path. It runs on the same OS/architecture as the
host only (no cross compilation), needs a prepared installed launcher base release, and survives cache pruning
because it is copied outside the managed folders. On Windows use the printed `horizon-engine.exe` path.

Point a server at it:

    horizon-server --dir SERVER config set engine.path=/absolute/export/horizon-engine
    horizon-server --dir SERVER restart

Updates do not rebuild a pinned `engine.path`: build a new export folder, set `engine.path` to it, restart, and keep
the old folder for manual rollback. The supervisor does not auto-update or rebuild a pinned engine path, and there
is no compatibility handshake yet (shared server build crate / native config / updates / handshake are follow-up
#6304, not implemented). Native mods run unsandboxed: only build code you have read and trust.
`--trust-native-code` records the operator's decision; an agent must not supply that consent.

### Developer acceptance workflow

Use a disposable server folder and fresh homes when developing:

```sh
SCRATCH=$(mktemp -d)
export HORIZON_HOME="$SCRATCH/game" HORIZON_LAUNCHER_HOME="$SCRATCH/launcher"
export XDG_DATA_HOME="$SCRATCH/data" XDG_CONFIG_HOME="$SCRATCH/config"
export HORIZON_NO_AUDIO=1 HORIZON_LAUNCHER_NO_SELF_UPDATE=1
export HORIZON_LAUNCHER_TRASH="$SCRATCH/trash"
export HORIZON_LAUNCHER_CLIPBOARD="$SCRATCH/clipboard"
export HORIZON_LAUNCHER_DESKTOP="$SCRATCH/desktop"
```

Install and prepare the base release in that scratch launcher home first, then test your own licensed bundle:

1. `horizon-launcher native-build bundle/ --server-output $ENGINE --version TAG --trust-native-code` (scratch homes,
   `HORIZON_NO_AUDIO=1`, `HORIZON_LAUNCHER_NO_SELF_UPDATE=1`).
2. `horizon-server --dir SERVER config set engine.path=$ENGINE/horizon-engine`; `horizon-server --dir SERVER run`.
3. Join with the same mod's client half where the hook needs one; test lifecycle, tick, command and net hooks.
4. Headless compatibility: the mod's server code must not require an audio device; run the server audio-free (`HORIZON_NO_AUDIO=1`).

## Left to do

- A list of every native build, to delete the ones no instance needs.
- A prebuilt horizon-mixins from its CI instead of building it on the player's computer.

## Named hook catalog (draft)

`native-hooks.json` maps stable hook names and contract versions to the current
source targets. A contract includes the signature, call timing and behavior.
Keep an existing contract or add a new version when it changes incompatibly.
The catalog does not cover helper methods or arbitrary private game code.
Injections name them with `hook` and `hook_version` ("Named hook points" above); raw
targets and the `hooks` metadata continue to work as before.

### Reading server blocks (draft)

A native server tick hook can call `native_server_block` with its context, a dimension id and a block position. It returns the block name and state properties from a loaded chunk. It does not load a chunk or open a dimension. Reads outside that hook, from a stale context, from unavailable dimensions or outside the world bounds return an error.

### Neighbour observations (draft)

`native_neighbor_changed` observes each authoritative server `neighborChanged` before the block handles it. It supplies the dimension, position, current state ID, changed block ID and moved flag. Resolve IDs with the level's registries during the hook; the hook does not load chunks or change the update queue.

### Server block changes (draft)

`native_block_changed` observes a committed authoritative server `Level.setBlock` change before block side effects and neighbour updates. It supplies the dimension, position and old and new state IDs. It does not observe chunk loading or direct storage writes that bypass `Level.setBlock`.

### Server commands and players

For mods that run where the world is simulated: a dedicated server, a LAN host, single player's server thread
(`crates/horizon-client/src/game/native_server.rs`). The SMP mod of the official server is built on them
([smp.md](smp.md)).

- **`native_register_commands(commands)`**: push a `NativeCommand` per usage, asked once when a world opens to
  players. A usage is words, then arguments `<name:kind>`, optional ones last in brackets:
  `NativeCommand::new("pay <player:player> <amount:integer>")`, `NativeCommand::op("eco set <player:player>
  <amount:integer>", 3)` for operators of level 3. Kinds: `word`, `text` (the rest of the line, last), `integer`,
  `number`, `boolean`, `player`, `item`, `block_pos`. The usages of one command merge into one tree, which every
  player receives after vanilla's (filtered by their level, as vanilla's is), so their chat box completes and colours
  it. A name vanilla has, or a usage that doesn't parse, is left out with a warning in the log.
- **`native_player_game_mode_changed(player, name, from, to, cancel)`**: a server player's game mode is about to change
  (0 survival, 1 creative, 2 adventure, 3 spectator, as `NativeServerPlayer.game_mode`): the `/gamemode` command, the
  F3+F4 switcher's packet and a hardcore world's respawn into spectator, once per change to a different mode. The mode a
  player joins with (`defaultgamemode`) is not heard. Set `cancel` and the player keeps their mode. Inside the command and
  the switcher the player's state is swapped in; for the hardcore respawn it is not (use the UUID with the helpers that
  take one).
- **`native_player_changed_dimension(player, name, from, to)`**: a player changed dimension, once per move
  (`from` and `to` are ids such as `minecraft:overworld` and `minecraft:the_nether`), heard when the move has ended:
  the portal's exit is found and the player stands there, a command's teleport has landed, or a respawn in another
  level than the death's is placed for good. `to` is the level in place and the player's state is swapped in (as in
  `native_perform_command`), so `native_acting_player` and `native_tell` reach them. Heard for Nether and End portals
  (also riding, and leaving the End as its winner), `/execute in ... run tp` and any command teleporting into another
  level, and the death respawn. Not heard: a move within a level (`/tp`, an End gateway), a player who leaves before
  the move ends, a respawn in the dimension of the death, and anything outside a dedicated server.
- **`native_player_toggle_flight(player, name, flying, cancel)`**: a player's client started (`flying`) or stopped
  flying, once per change, for a player who may fly (creative or spectator), on a server. The player's state is swapped
  in, so `native_tell` and the other helpers reach them. Set `cancel` to refuse it: the player keeps the state they
  had and gets their abilities back, so the client lands or flies again. Only what a client asks for is heard: not the
  flight a game-mode change starts or ends, and not a LAN host's own player.
- **`native_sign_changed(player, name, dimension, pos, front, lines, cancel)`**: a player finished a sign they were
  editing, heard once every check has passed (editing that sign, in reach, not waxed, the chat filter) and right before
  the text goes on it. `lines` holds the four lines as written, formatting codes still in; rewrite them and that is
  what the sign gets (the chat filter does not look at it again), or set `cancel` and it keeps its old text. The
  player's state is swapped in, so `native_tell` reaches them. Not told: the host's own editing on a LAN or
  single-player world, and changes that are no text edit (dye, glow ink, wax, `/data`, `/setblock`).
- **`native_player_command(player, name, command, canceled, replies)`**: any command a player typed (without the
  slash), vanilla's too and commands the player may not run, heard before anything looks at it, and each line of a
  quiet command batch. Set `canceled` and nothing runs; push lines to `replies` and the player gets them as system chat
  (a quiet batch drops them). The player's state is not swapped in, and `native_tell` and the other helpers that take a
  UUID reach nobody inside this hook: answer through `replies`. Commands may carry secrets (`/msg`, a login plugin's
  password), so don't log them carelessly.
- **`native_perform_console_command(command, ran)`**: a registered command typed on the dedicated server's console
  (without its slash), run as the server at level 4: no player, so `native_acting_player()` is None and the player-only
  helpers give nothing. The mod parses the line and sets `ran`; `native_feedback` answers on the console. A name no mod
  registered goes to vanilla's dispatcher as before. The single player owner's console (`horizon-ctl command`, the MCP
  command tool) is a player's: it reaches `native_perform_command` with the owner, at level 4.
- **`native_player_respawned(player, name)`**: a dead player came back, once per respawn, on a server: after
  `PlayerList.respawn` reset their health, food and experience and placed them at their bed, anchor or the world spawn
  (NeoForge's `PlayerRespawnEvent`). The player's state is swapped in for the call, as for their other actions, so
  `native_player`, `native_give_items` and the other helpers reach them. A respawn at an anchor in another dimension, or
  at the world spawn's free column, may still move them on a later tick, once that level has loaded round them; the
  End-exit respawn (after the credits) does not fire it.
- **`native_player_fish(player, name, state, cancel)`**: a player's fishing rod, on a server: `state` 0 the cast, 1 a fish
  biting, 2 the reel-in of a bite (the catch), 3 the reel-in with nothing on the line, 4 the reel-in of a bobber hooked
  to an entity or item. Set `cancel` and nothing is thrown (0), the fish gets away and the wait starts over (1), or the
  bobber stays out with no wear on the rod (2 to 4). The bobber ending by itself (too far, rod put away, owner dead) is
  not heard, nor is the host of a LAN world's rod. The player's state is swapped in, so `native_acting_player`,
  `native_tell` and the item helpers reach them.
- **`native_perform_command(player, command, ran)`**: a player typed a registered command (without its slash) and
  their level allows it, whether the world allows commands or not. The mod parses the line and sets `ran`. The
  player's state is swapped in for the call, as for their other actions, so `native_player` reads them; a
  registered command no mod takes answers as vanilla's unknown command.
- Helpers: `native_server_players()` (uuid, name, level, dimension, position and game mode of each player on);
  `native_tell(uuid, text)`, a system line to one player, also the acting player inside their hook;
  `native_broadcast(text)`, to every player and the server's
  log; `native_server_event(name, uuid, message)`, an `announce` event on a dedicated server's event stream, which the Discord
  bridge relays when `[discord] events` lists `announce`; `native_feedback(text)`, the command's answer to the player whose command it is; `native_world_dir()`, the
  open world's folder, where a server mod keeps its data in a folder of its own so the world's backups carry it.
- **`native_item_damaged(player, name, item, amount, cancel)`**: an item of a player is about to lose durability, on a
  server (`ItemStack.hurtAndBreak`: tools, armour, shield, rod, elytra), after Unbreaking has had its say. Change
  `amount` (0 leaves the item whole) or set `cancel` for no wear. Not heard for a creative player or when Unbreaking
  saved the item, nor for a mob's armour, a dispenser's tool or the host of a LAN world. The player's state is swapped
  in, so `native_acting_player`, `native_tell` and the item helpers reach them.
- Items (API 9), of the acting player (inside `native_perform_command`, whose player is swapped in): `native_held()`
  (the main hand's item, count, and whether it is plain), `native_count_items(item)`, `native_carried_items()`,
  `native_take_items(item, count)` (the main hand first, then as `Inventory.add` orders slots),
  `native_room_for(item)` and `native_give_items(item, count)` (only where it fits: nothing is dropped). They look at
  the 36 inventory slots and the off hand, never the armour, and only at **plain** stacks: no damage and no
  component (name, enchantment...), so a shop never takes a worn or enchanted item for a new one. Each returns how
  many it counted, took or gave; the player's screen follows as for any change to their inventory.
- `native_give_stack(uuid, stack)`, a whole stack (damage, name, enchantments... kept) to any player from any server
  hook, as `Inventory.add` places it (onto equal stacks, then empty inventory slots); what did not fit comes back, all
  of it for a player not here, dead, or swapped out by another player's hook. Nothing is dropped: an auto-pickup
  leaves the rest on the ground, a machine keeps it.
- Experience: `native_experience_orbs(dimension, min, max)`, the orbs in a box (id, position, `value`, and `count`
  for merged orbs), read before a mod cancels an orb in `native_entity_join_level`; and `native_give_experience(uuid,
  points)`, points given to any player as touching an orb gives them: Mending repairs their damaged items first, the
  rest goes to their experience, with the level-up sound; and `native_give_levels(uuid, levels)`, levels added or,
  negative, taken off (`Player.giveExperienceLevels`, as the anvil and the enchanting table charge; below 0 the level
  and points are 0; no level-up sound). Both return false for a player not here, dead, or swapped out by another
  player's hook, and where the server's player list is busy (some world ticks hold it): give first, and cancel the orb
  only when the give returned true. A merged orb is worth `value` × `count`.
- API 21: `native_acting_player()`, the player whose state is swapped in (inside `native_perform_command`,
  `native_right_click_block`, `native_use_item_on`, `native_destroy_block`...; None elsewhere);
  `native_run_command(line)`, a command run as the server's console (level 4, no output), only where no player is
  swapped in (`native_tick`, a menu hook): a teleport to another level (`execute in minecraft:the_nether run tp Alex 0
  70 0`) lands at the next tick's start. `native_server_players()` and the acting player carry `health` (0 once dead).
  `native_action_bar(uuid, text)`, a line over a player's hotbar, which also reaches the acting player inside their
  hook.
- **`native_right_click_block(pos, face, canceled)`** (API 21): a player's right click on a block, before the block's
  own use and the item's (`ServerPlayerGameMode.useItemOn`), once per hand. A mod that sets `canceled` stops both,
  without a swing; the player's client is sent the block back. With `native_destroy_block` it keeps players out of
  claimed land.
- **`native_block_spread(dimension, kind, from, to, canceled)`** and **`native_piston_move(dimension, piston,
  direction, push, destroy, canceled)`** (API 22, on `GameLevel`, like the block observers): a fluid flowing or fire
  spreading (`kind` `fluid` or `fire`) from `from` into `to`, and a piston moving blocks once its structure resolved.
  Canceled, the block at `to` stays as it is, and the piston moves nothing. Fluids ask on every flow: answer from
  memory, fast.
- **`native_random_tick(ticks, dimension, pos, state, handled)`** (API 33, on `GameLevel`): a block `tickChunk`
  random-ticks, before its own `randomTick`. Change the block with the level's `native_set_block_property(ticks, pos,
  key, value, flags)`; set `handled` to skip the block's own random tick.
- **`native_tick_block(ticks, dimension, pos, state, handled)`** (API 34, on `GameLevel`): a due scheduled tick of a block
  that is still there (`ServerLevel.tickBlock`), before its own `tick`; the same contract as `native_random_tick`.
- **`native_explosion_detonate(dimension, center, blocks)`** (API 22): the positions an explosion is about to break,
  before any is hit; take out the ones to spare. Entities are hurt as before.
- **`native_fill_bucket(stack, pos, canceled)`** and **`native_attack_entity(entity, kind, pos, canceled)`** (API 22):
  a player's bucket at a block, and their blow on an entity, with the acting player swapped in. Canceled, the bucket
  does nothing and the blow does no harm.

### Mobs

API 37, `crates/horizon-client/src/game/native_mobs.rs`: helpers for server mods, for the level in place, on a server only (on a
client they do nothing, and the readers give nothing). Each takes the `dimension` (`minecraft:overworld`) of the level it means, as
`native_set_block` does, and is refused when another level is in place. The per-level tick hooks (API 23) run with a defined level in
place; `native_server_tick` does not define one, so name the dimension and expect a refusal there.

- `native_spawn_mob(dimension, kind, pos, &NativeSpawn)` spawns a mob and returns its id. `NativeSpawn` holds an optional name and health, and the
  `no_ai`, `invisible`, `silent`, `invulnerable` and `no_gravity` flags; its default is a plain mob. The mob is finalized as a natural spawn
  is, and kept when its chunk unloads. Nothing is spawned outside `/summon`'s bounds, in a chunk that isn't loaded, or a monster on peaceful.
- `native_mob(dimension, id)` and `native_mobs(dimension)` read the living mobs: id, dimension, kind, position, yaw, health and maximum health.
- `native_teleport_mob(dimension, id, pos)` (as `/tp`: it dismounts and drops its path), `native_push_mob(dimension, id, velocity)` and
  `native_set_mob_health(dimension, id, health)` change one, and answer false for no such mob or a value that isn't finite. Health must be
  above 0.
- `native_kill_mob(dimension, id)` kills as `/kill` does, with its loot; `native_remove_mob(dimension, id)` takes the mob away with no death
  and no loot (it is not saved either).
- While the game ticks its mobs they are out of the level: from a hook inside that tick, a spawned mob joins after the tick, and the others
  read as gone.

### Loot tables

Changing loot a mod didn't make, as NeoForge's `LootTableLoadEvent` and global loot modifiers and Fabric's
`LootTableEvents.MODIFY` and `MODIFY_DROPS` do. A mod's own loot function and condition kinds are #5451.

- **`horizon_world::loot::native_loot_table_load(id, table)`**: each table's JSON as it loads, after the data packs'. Push a pool
  onto `table["pools"]` or an entry onto a pool's `entries`; chests, block and mob drops, fishing and the rest then roll the
  changed table. Keep a replaced table's `random_sequence`: a mob's table without one drops nothing. It runs wherever tables load,
  a client too, so make the same change each time; a `/reload` may keep the tables already loaded. It is a free function in
  `crates/horizon-world/src/loot.rs`, so a helper injected there compiles inside `horizon-world`, without `crate::game`.
- **`native_loot_drops(context, drops)`**: the items a broken block or a dead mob is about to drop, after its tables rolled (a
  mob's worn, held and carried items and inventory too, not an enderman's carried block or a copper golem's preserved equipment).
  Add, change or take out stacks; empty ones don't drop. `NativeLootDrops` says what dropped them: its own loot table (empty when
  none rolled), the dimension, the origin, the block's position and state or the mob's id and type, the player who broke or killed
  it (a mob names one only while it remembers the player's hit, 100 ticks), the tool and an explosion's radius. It hears blocks
  that a player, an explosion or a mob breaks, not other breaks (leaves decaying, a torch losing its wall, a piston,
  `/setblock ... destroy`), nor a creative break or a tool that can't harvest the block. A block's stacks get its block entity's data (a
  shulker box's items) after the hook, and a container's contents spill on their own, not through it. A player's use drops
  through it too, with `used` set and that player named: shears on a sheep, a mooshroom, a snow golem or a bogged (its shearing
  table), a brush on an armadillo (`minecraft:brush/armadillo`) or a suspicious block (its item, no table), shears carving a
  pumpkin, picking sweet berries or cave vines (their harvest table), and a player's hit taking an item frame or its item (the
  frame's entity, no table). A dispenser's shears and an explosion's frames aren't told. Leads: untying one or cutting it with
  shears is a use (the leashed mob, no table), and a mob a player killed while leashed drops its lead on its next tick as a call
  of its own, naming the killer (`used` false, no table). A lead that snaps or loses its holder names no player and isn't told.

### A block's items

Helpers for a server mod, by the rules hoppers follow: a furnace's sides, a double chest's two halves, a mod's `ModContainer` with its `faces`, `inputs` and `outputs`. `face` is the side of the block being reached, so a pipe above a chest uses `Direction::Up`. Those that read or take name the level in place by its `dimension`, and another one is refused. A client's copy of the world is refused.

- **`native_container_slots(dimension, pos, face)`:** the slots that side reaches, with each one's number, stack and `can_take`. None where no container is: another dimension, a chunk that isn't loaded, a block without slots, a composter, or the mod block entity ticking now, which has its items in hand.
- **`native_extract_item(dimension, pos, face, slot, count)`:** takes up to `count` from a slot through that side. The items are the mod's to place then, so to hand them on use `native_move_items`.
- **`native_move_items(dimension, from, from_face, to, to_face, item, max)`:** moves up to `max` items, one by one, and returns how many. An item the target refuses stays in its slot, so none is lost. `item` (`minecraft:coal`) takes only that item. It answers 0 within one container.
- **`native_insert_item(pos, face, stack)`** (API 20): puts a stack in the level in place, and returns what didn't fit.
- **`native_dimension()`:** the level in place (`minecraft:overworld`), for a hook that isn't told it, such as `native_block_entity_tick`.

During a machine's own tick its block entity is out of the world, so these answer as if no container were there: it has its slots in hand, in the `items` it's given. A composter takes one item a call, and these don't empty it: a hopper does. Container entities (minecarts, chest boats) aren't reached yet.

### Entities

`crates/horizon-client/src/game/native_entities.rs`: helpers for server mods, for the level in place, on a server only. Each
names the level it means by its `dimension` (`minecraft:overworld`), as `native_set_block` does.

- `native_summon(dimension, kind, pos, nbt)` spawns an entity of `kind` from its compound, as `/summon <kind> <pos> <nbt>`
  does (passengers included), and returns its id. None on a client, for another dimension, a position outside `/summon`'s
  bounds, a chunk that isn't loaded, a kind Horizon can't load, or one a mod kept out with `native_entity_join_level`.
- `native_move_entity(dimension, id, pos, yaw, pitch)` moves an entity Horizon keeps without simulating it (a display, a
  marker, an interaction); a display eases there over its teleport duration. Mobs move with `native_teleport_mob`.
- `native_set_display(dimension, id, &NativeDisplay)` sets a `block_display`, `item_display` or `text_display`'s
  transformation (translation, left rotation, scale, right rotation; quaternions `[x, y, z, w]`), the ticks players see it
  ease over (`interpolation`) and its teleport duration (`teleport`, 0 to 59). Each call restarts the ease on clients.
- `native_remove_entity(dimension, id)` removes such an entity as `/kill` does. Mobs go with `native_remove_mob`.

A model that moves (a turning gun barrel, a thrown projectile) is an `item_display` showing an item whose model comes from
the mod's `.hmod` assets: summon it once, then move and pose it from a tick hook. Easing between ticks happens on each
client, so a call a tick looks smooth.

### Mobs drawn by a mod

`crates/horizon-client/src/game/native_mob_draw.rs`: `native_mob_submit(mob, draw, drawn)` (client) is asked once a frame for
every mob in view but players (vanilla's `EntityRenderer.submit`). Set `drawn` and the mob's vanilla model, armour and held
items are left out (NeoForge's `RenderLivingEvent.Pre`, cancelled); its name tag, shadow, flames and glowing outline still draw.

- `mob` (`NativeMobRender`) is the frame's `LivingEntityRenderState`: the id, kind, custom name and `baby`, the partial tick,
  `age` (ticks lived plus the partial tick), `body_yaw`, `head_yaw` (from the body) and `pitch` in degrees, the walk cycle
  (`walk_position`, `walk_speed`: a vanilla quadruped's legs swing `cos(walk_position * 0.6662) * 1.4 * walk_speed` radians), `hurt_time`,
  `death_time` and `on_ground`.
- `draw` is a block entity's frame (`native_block_entity_submit`): draw with `native_draw_cube`, `native_draw_quad`,
  `native_draw_block` and `native_draw_item` under its pose stack, which starts at the mob's feet turned with its body: +z
  is the way it faces, +x its left, +y up, in blocks. Turn a head or swing a leg with `rotate_about` and a pivot. It holds
  the mob's light.
- A mod's own creature is an ordinary mob (`native_spawn_mob`, API 37) drawn this way: a quiet villager, an allay or a bat
  gives it movement, sounds and saving, and the mod gives it a model and animations of its own. Tell yours apart by the
  ids the server half sends with mod messages (API 40), or by its custom name.
- Players have their own hook, `native_player_submit(player, who, draw, drawn)` (`RenderPlayerEvent.Pre`): once a frame for
  each other player in view and for this window's player in third person, with the same frame and `who` (the profile name,
  and `local`). `drawn` leaves out the vanilla model, armour, held items, cape and other layers.
- `native_mob_layer(mob, who, draw)` (`EntityRenderersEvent.AddLayers`, `RenderLivingEvent.Post`) adds to every mob and player
  after its vanilla model and layers, in the same frame (not for one a mod drew itself); `who` is set for a player. The
  geometry hangs from the entity's frame, not from a model bone: place a hat from `head_yaw`, `pitch` and the entity type's
  eye height.

### Advancement triggers

`crates/horizon-client/src/game/native_criteria.rs`: a mod's own criteria triggers, as a NeoForge mod registers a
`CriterionTrigger` and calls its `SimpleCriterionTrigger.trigger`. The advancement is an ordinary file of a content mod
or a world's data pack (`data/mymod/advancement/connect_a_pipe.json`) whose criterion names the mod's trigger and
whatever conditions the mod reads:

```json
{
  "criteria": {
    "copper_pipe": {
      "trigger": "mymod:pipe_connected",
      "conditions": {"pipe": {"items": "#c:ingots/copper"}}
    }
  }
}
```

- `native_trigger_criterion(uuid, trigger, test)` fires it for one player, on a server only. Each criterion with that
  trigger the player hasn't been granted, of an advancement not done yet, goes to `test`, then to its `player`
  predicate, as vanilla tests one. Those that pass are granted; an advancement they complete is awarded as by the game's
  own triggers: the toast, the chat line, the rewards and the player's saved progress.
- `test` gets a `NativeCriterion`: its `conditions` (`Null` when it has none), and `item(key, stack)`, which tests
  `stack` against the item predicate under `key` as the game's own triggers read one: `items` (an id, a list or a
  `#tag`), `count`, the `potion_contents` and `damage` components, and the `enchantments` and `damage` predicates.
  Other components and predicates aren't checked. A missing key passes.
- It reaches the acting player inside their hook, and any player on the server, in their own level, only where no
  player is swapped in (`native_tick`, a menu hook), as `native_run_command`. True means the player was reached, not
  that anything was granted. It returns false otherwise, when the player isn't on, and for a `minecraft:` trigger or a
  name without a namespace: the game fires vanilla's triggers from its own events.

### Explosions

`crates/horizon-client/src/game/native_explosions.rs`: a helper for server mods, for the level in place, on a server only. It
names the level it means by its `dimension` (`minecraft:overworld`), as `native_set_block` does, and is refused when another
level is in place.

- `native_explode(dimension, center, radius, fire, blast)` sets off an explosion as `Level.explode` with no source entity
  does: blocks break (`native_explosion_detonate` sees them first and can spare some), entities are hurt and pushed, `fire`
  lights a third of the air above solid blocks, and nearby players hear and see it. `blast` is `NativeBlast::Keep` (no
  block breaks), `Block` (drops by `block_explosion_drop_decay`) or `Tnt` (by `tnt_explosion_drop_decay`). It answers false,
  doing nothing, on a client, for another dimension, a chunk that isn't loaded, or a radius outside `0 < radius <= 32`
  (TNT is 4, vanilla's largest is 7).
- A blast set off inside `native_explosion_detonate` runs that hook again, so a mod that chains blasts keeps its own guard.

### Camera shake

`native_camera_shake(partial, shake)` (client) asks every frame for the camera shake a mod wants: `pitch`, `yaw` and `roll`
in degrees and an `offset` in blocks along the camera's right, up and back. Add to `shake` rather than set it, so two
mods' shakes add up. Horizon reads it only while playing, scales the sum by Distortion Effects (a player who turns
that to 0% sees no shake), bounds it to 10 degrees and half a block on each axis, and multiplies it into the view
after the hurt tilt and view bobbing. A mod keeps its own shake's time and decay, from `partial` and its tick hooks;
it moves only what the player sees, not where they are or aim.

### A player's motion

`crates/horizon-client/src/game/native_motion.rs`: a push on a client, a flying speed on a server.

- `native_push_player([x, y, z])` (client) adds a push to the player's own velocity, in blocks a tick, as an
  explosion's knockback does: the player's own game moves them. The server didn't send it, so on a LAN or dedicated
  server the anticheat's movement check sees a move it can't explain (logged only unless the server sets it to send
  players back); for a push the server should stand behind, have the server send it. Each component is
  cut to 4 either way; an upward push lifts the player off the ground. A spectator, no player, or a value that isn't
  a number is left alone. A server mod that wants a client pushed sends its client half a message (API 40).
- `native_set_flying_speed(player, speed)` (server) sets `player`'s creative flying speed: vanilla's is 0.05, `None`
  puts it back, and it is cut to 0 to 50 (1000 times vanilla's). The player's client is told at once, a respawn and
  a game mode change keep it, and it is saved with the player as vanilla's `abilities.flySpeed` is. Past about 0.45
  (sprinting) a player flies more than 10 blocks a tick: anyone but the single-player owner then meets the server's
  moved-too-quickly check, which can send them back, as vanilla's does. It answers false for a player not on this server,
  on a client, and inside a player's hook (call it from a tick hook).

### Values for a mod's shader pack

`native_shader_params(partial, params)` (client) asks every frame, while a Horizon shader pack is on, for eight
`[f32; 4]` that every stage of the pack reads as `frame.mods` (`array<vec4<f32>, 8>`, docs/api/shaders-reference.md). What
they mean is the mod's own: a mod that bundles its pack as `native-<mod id>` writes the slots that pack reads, such as an
explosion's centre (camera-relative, from `frame.camera_position`) and its age in seconds, for a cloud or a shock wave
ray marched in a post pass against `scene_depth`. A value that isn't finite reads as 0, and all are 0 when no mod sets
them. The mod's server half can send what happened to each client half with mod messages (API 40).

### Biome colours

`crates/horizon-client/src/game/native_biome_colors.rs`: the grass, leaf and water colours a client mod changes while the
game runs, for seasons (a golden autumn, a pale winter) or a world that greens as it heals.

- `native_biome_colors(biome, colors)` (client) changes a biome's `BiomeColors`: `grass`, `foliage` (leaves, vines),
  `dry_foliage` (leaf litter) and `water`, 0xRRGGBB, starting from the biome file and the colormaps. The dark forest's
  modifier still darkens the grass set here; the swamp's noise colours replace it.
- It is asked for every biome when a mod calls `native_refresh_biome_colors()`. When the colours differ from those in use,
  every loaded section and the far terrain are rebuilt with them, as changing Biome Blend does. Call it when your colours
  change and once when the player joins, not every tick: a rebuild takes the workers a few seconds. A slow fade is a
  call a day, not a frame.
- Block-breaking and falling-leaf particles keep the biome's own colours for now.
- A season's mood in a shader pack (warmer grading, a lower sun, more haze) comes from the same mod through
  `native_shader_params` (`frame.mods`); its server half can send the season to each client with mod messages.

### Sounds a client mod plays

`native_play_local_sound(sound, at, volume, pitch, delay)` (client) plays sound event `sound` at `at` for this player only,
`delay` ticks from now (0 to 1200). The event is vanilla's (`minecraft:entity.generic.explode`) or the mod's own: a content
mod's `assets/<id>/sounds.json` and `assets/<id>/sounds/*.ogg` load with the rest of its assets. A volume past 1 carries
farther, as vanilla's attenuation does. A client half that hears from its server half (mod messages, API 40) where
something happened can time the sound to the distance, such as a blast heard when its shock wave reaches the player:
`delay = distance / speed`. It answers false on a server and for a value out of range; an event the resources lack
plays nothing.

### Falling blocks

`crates/horizon-client/src/game/native_falling.rs`: a helper for server mods, for the level in place, on a server only. It
names the level it means by its `dimension` (`minecraft:overworld`), as `native_set_block` does.

- `native_spawn_falling_block(dimension, state, pos, velocity, falling)` throws a falling block of `state` from `pos` (its
  bottom centre) at `velocity` in blocks a tick, each axis cut to 3.9. It flies, lands and places itself as sand does, or
  drops as an item where it can't be placed. `NativeFalling` holds `drop_item` (default true) and `hurts` (hurts what it
  lands on as an anvil does, 2 a block fallen and 40 at most; default false).
- The block at `pos` stays: take it away first with `native_set_block` to make a block fall out of the world, as an
  explosion's debris or a collapsing tower would.
- It answers false, throwing nothing, on a client, for another dimension, air or an unknown state, a chunk that isn't
  loaded, a value that isn't finite, or with 1024 falling blocks in the level already.

### Bulk block edits

`crates/horizon-client/src/game/native_world_edit.rs`: a helper for server mods, for the level in place, on a server only.
It names the level it means by its `dimension` (`minecraft:overworld`), as `native_set_block` does.

- `native_edit_blocks(dimension, edit)` makes many blocks at once: `NativeBlockEdit` holds a `palette` of block states and
  a list of `filters` (block predicates, `#minecraft:logs`), both written as commands write them, and `runs`, each
  `height` blocks up column (`x`, `z`) from `y`, set to palette entry `block`, only where filter `only` passes the block
  there. With `surface` (`NativeHeightmap`: `WorldSurface`, `MotionBlocking`, `MotionBlockingNoLeaves` or `OceanFloor`),
  `y` counts from that height of the column, read once a column before anything changes. Runs are made in order, each
  seeing the changes of those before it.
- The blocks go straight into the chunks, a section packed once however many of its blocks change, as `fill ... strict`
  does: no neighbour is told, nothing drops, and no block updates or ticks; light is worked out again for the chunks
  changed, and they go to the players whole. A block replaced by another loses its block entity, and a new block gets
  the one `native_set_block` would give it (a mod block's own, API 15, included).
- It answers the number of blocks changed. Runs in chunks that aren't loaded, and the parts of runs outside the level's
  height, are left out. It answers `Err`, changing nothing, on a client, for another dimension, a palette entry or filter
  that doesn't read or carries NBT (`chest{...}`: refused, set states only), a run naming a palette entry or filter past
  the end, or more than 65536 filters.

### Column reads

`crates/horizon-client/src/game/native_world_columns.rs`: helpers for server mods, for the level in place, on a server
only, that read many columns in one call. They name the level they mean by its `dimension` (`minecraft:overworld`), as
`native_set_block` does, and answer `Err` with the reason, reading nothing, on a client, for another dimension or for
a read too big, so a mod can tell "ask for less" from "wrong level". They run on the server tick: read a big area in
pieces.

- `native_surface_heights(dimension, columns, heightmap)` answers each column's height on `heightmap` (`NativeHeightmap`,
  as `native_edit_blocks` reads it: one above the highest block that counts), `None` for a column whose chunk isn't
  loaded; at most 65,536 columns a call.
- `native_read_columns(dimension, columns, heightmap, (below, above))` reads, for each column, the blocks from `below`
  under its height to `above` - 1 over it, bottom first: `NativeColumnBlocks` holds the heights (`bases`), the block
  names seen (`palette`), the blocks as indexes into it, a column after another, and three biome tints a column (grass,
  foliage and water, at the column's top). Blocks outside the level's height, and every block of a column not loaded,
  read as air (its tints as 0). `below + above` must be 1 to the level's height, and the read at most 1,048,576 blocks.
  The tints are the biome's own at the column's top, not blended over the biome blend radius as the game draws them, so
  near a biome border they can differ from what players see by a few blocks.

### Mod messages

API 40, `crates/horizon-client/src/game/native_net.rs`: bytes between a mod's server half and its client half, on the
one custom payload channel `horizon:mod_net`. A vanilla client never gets them, and a vanilla server isn't sent any.

- `native_net_send_to_player(player, channel, data)` on the server and `native_net_send_to_server(channel, data)` on the
  client send one. The channel is `mod_id:name`: the id `[a-z0-9_]`, the name `[a-z0-9_./-]`, each 1 to 64 characters;
  `minecraft` and `horizon` are the game's. The data is at most 30 000 bytes, and 32 messages a tick go each way per
  player. Ok means accepted: a player who left doesn't get it. Sending to the server is an error until the player has
  joined.
- `native_net_from_client(player, channel, data)` and `native_net_from_server(channel, data)` hear them, all mods on every
  message: check the channel. What a client sent is checked by nothing, so treat it as a player's own input.
- A player's message reaches the server's hook in the tick after it arrives, when the helpers can reach every player;
  a message to a player leaves at the end of the tick, from any server hook.

### Server menus

API 10, `crates/horizon-client/src/game/native_menus.rs`: menus a server mod builds from ordinary items, which a
stock client shows as a chest (`generic_9x1` to `generic_9x6`); nothing is installed on the player's side.

- `native_open_menu(uuid, rows, title, items)` opens one in place of the player's open menu (its window id);
  `native_set_menu_items(uuid, items)` changes what it shows; `native_close_menu(uuid)` closes it.
- **No click moves anything.** Each reaches `native_menu_clicked(uuid, window, slot, button, mode)`, and the client
  is sent the menu, its inventory and its cursor as they were. `slot` counts the menu's own slots first, then the
  player's inventory below it: `native_menu_inventory_slot(size, slot)` gives the inventory slot (0..9 the hotbar).
- `native_menu_closed(uuid, window)`: the player closed it, another menu opened, or the player left.
- `native_player_stacks(uuid)`: the player's 36 inventory slots, to show copies or offer them.
- `native_exchange_items(a, a_offer, b, b_offer)`: each offer is inventory slots, each with the exact stack it must
  still hold. Both offers leave their slots and go to the other player where they fit whole (onto matching stacks,
  then empty slots), or nothing moves and the error says why (an offer that changed, no room, a player gone or
  dead). A trade on these menus leaves the offered stacks in their owners' inventories until this one step, so
  closing the menu, leaving or dying moves nothing.

The menu hooks run with no player's state swapped in; `native_player_stacks` and `native_exchange_items` refuse a
player whose state is (inside `native_perform_command`): call them from a menu hook or `native_server_tick`.

### Render hook points

`crates/horizon-client-render/src/gfx/native_render.rs`: the renderer's own hook points, free functions rather than
`Game` methods, which a bundle targets as `horizon_client_render::gfx::native_render::native_<name>`. They run on the
render thread with the frame's wgpu objects; what a mod draws there shows in that frame.

- `native_world_drawn(world)`: the world is drawn (sky, terrain, entities, particles, weather and the world post pass),
  the first-person hand, the HUD and screens not yet. `WorldDrawn` holds the device, queue and encoder, the frame's
  colour target (copy it out before sampling it), the world's reversed depth and its size, the camera's position and
  its matrices with and without TAA's jitter, the frame's camera-relative entity quads with their runs by entity, the
  entity atlas and the shared quad index buffer. Not called on the title screen's panorama.
- `native_render_needs(needs)`: set `entity_spans` to have the entity geometry split into runs by entity without a
  shader pack (each run's id is stable from frame to frame: a hash of the entity type's name, a block entity's state),
  and `world_depth` to keep the world pass's depth when nothing else needs it. Set what you need and leave the rest:
  several mods can ask. It's asked as each frame is built, so keep it cheap.

### Menu slot rules

`crates/horizon-client/src/menu.rs`: `native_menu_slot_rules(menu, slot, stack, may_place, max_stack)`, a free function
a bundle targets as `horizon_client::menu::native_menu_slot_rules`. A content mod menu type's container slot `slot`
(from 0) is asked whether it takes `stack` (`Slot.mayPlace`) and how many it holds (`Slot.getMaxStackSize`), as a
NeoForge mod's own `Slot` overrides them. `may_place` and `max_stack` come in with the type's own answer (false on one
of its `results` ([mods.md](mods.md#menu-types)), true elsewhere, and 99): change them for your own menu types.
`max_stack` is kept to 1 to 99, and the item's own stack size still caps it. It runs inside a click, a drag or a
shift-click (several times for one), with no level or player at hand: answer from the stack, and fast. It is asked
only for a server's players (single player's server thread, LAN guests, a dedicated server), never in-frame
(`--in-frame`), and vanilla's menus never ask it. As in vanilla (`AbstractContainerMenu.moveItemStackTo`), a
shift-click tops up a slot (not a `results` one) that already holds the same item without asking `may_place`, only `max_stack`: to keep a
slot from filling that way, lower `max_stack`.

### Items' default components

`crates/horizon-world/src/item_defaults.rs`: `native_modify_default_components(item, components)`, a free function a
bundle targets as `horizon_world::item_defaults::native_modify_default_components` (NeoForge's
`ModifyDefaultComponentsEvent`). `components` is the item's entry in `items.json`, vanilla's default-component report:
insert or replace a key to set a component (`"minecraft:max_stack_size": 16`), remove it to take the component away.
Any item can be changed, vanilla's included. Write values as vanilla's codec writes them: nothing decodes them with it.
Each table reads the fields it understands and takes a default for a field it can't read, but keeps the component, so a
malformed `minecraft:food` still makes the item edible, with no nutrition. Only the stack size is kept to vanilla's
1 to 99.

- Every table that reads items' defaults reads them through it: the item table, loot, enchanting, item models,
  banners, pot sherds, item rarity in chat. Stacks are saved and sent as their difference from the changed defaults, as from vanilla's.
- It's asked for every item each time the defaults are read, from any thread, several times as resources load
  and again when a world's data packs load. Answer the same each time, from the item alone.
- Client and server each apply their own mods, so both need the mod. A client without it (a vanilla Java client, or
  Horizon without the mod) fills in vanilla's defaults for the same stacks: a stone stack the server caps at 16 still
  looks like it stacks to 64 there.
- The creative search tab matches vanilla's exported tooltip text, so a changed `item_name` is not searchable yet.

## Trait mods (draft)

A bundle declares its trait mod package by its path inside the bundle:

```toml
[api]
path = "engine"
```

The package may use only crates and versions in Horizon's `Cargo.lock`, like mixin crates.
The folder contains a Rust package with a `horizon-client` dependency. Its library exports
`pub fn hooks() -> Box<dyn horizon_client::mod_api::Hooks>`. The launcher links this package
into a generated game executable and calls `hooks()` in dependency order (a mod after the mods it
depends on, else in bundle order). The package is not a
`[[crate]]`: it depends on the game instead of the game depending on it. A bundle with no
injections, state, bridges, modules, mixin crates or post-build steps skips horizon-mixins.

### Dependencies between trait mods

A mod can publish functions and statics from its `[api]` package for another mod to call:

```toml
[dependencies]
base_mod = "^1"
extra_mod = { version = "^2", optional = true }
```

Requirements use Cargo semver syntax and are checked against each installed bundle's `[mod] version`,
not its Cargo package version. The dependent and every installed dependency need an `[api]` package.
Missing required dependencies, incompatible versions and cycles stop the build with the mods named.
An absent optional dependency is permitted; an installed optional dependency must match its requirement.

The launcher adds a path dependency using the mod ID as the Cargo alias and the target's actual Cargo
package name. In Rust, replace hyphens in the alias with underscores. Optional dependencies also get a
Cargo feature named exactly as their mod ID, enabled only when that mod is installed. Guard calls with
`#[cfg(feature = "extra_mod")]`. Do not define these dependencies or features yourself in Cargo.toml.
Public statics must use appropriate synchronization for shared and reentrant hooks; private state in
another mod's hook object is not exposed.

Trait dispatch follows dependency order, with ties kept in bundle order. Mixin injection order remains
horizon-mixins' priority, mod ID, then injection ID; dependency order does not change injection order.
A bundle with an `[api]` package may declare its own HEAD injection in the trait:

```toml
[[injection]]
id = "before-tick"
target = "horizon_client::game::example"
kind = "HEAD"
trait_method = "before_tick"
bindings = [{ name = "amount", ty = "&mut i32", value = "&mut *amount" }]
```

The generated trait method is `inject_<mod_id>_<trait_method>` (hyphens in the mod id become
underscores). Its event, as a catalog hook's, is that name in CamelCase plus `Event`
(`InjectExampleBeforeTickEvent`), with the bindings as fields under the same rules. Implement it on the object
returned by that bundle's `hooks()`; it is called only for that bundle's object. Types must be
public and available from both the target and `horizon_client::mod_api`. Only HEAD listen is
supported, or HEAD with `cancellable = true` for a unit-returning target: that method returns
`Flow`, and `Flow::Cancel` returns early from the target. Do not specify `helper` on these entries.
Other injection kinds are rejected. Trait injections can target only functions in `horizon-client`;
ordinary mixin helpers in other crates continue to work as before. The generated helper has the same priority and ordering as an
ordinary helper of that mod: priority descending, mod id then injection id. It is not automatically
ordered after other mixins. Catalog hooks retain the mixin-before-trait order below.

A linked trait mod implements `horizon_client::mod_api::Hooks`. Mixin injections run first.
An injection that returns early skips trait mods. A claimed cancellation also skips trait mods.
Otherwise, trait mods run in their supplied order. The first `Cancel` stops dispatch.
For replacement hooks, the first `Some` wins. Collection hooks call every mod. Receiverless hooks have no `Host`.

Each hook method takes its host, where it has one, and one event:

```rust
fn living_incoming_damage(&self, event: &mut LivingIncomingDamageEvent<'_>) -> Flow {
    if event.kind == "minecraft:pig" {
        event.amount *= 0.5;
    }
    Flow::Continue
}
```

The event is a generated `#[non_exhaustive]` struct named after the catalog hook, in CamelCase plus `Event`, with
the hook point's arguments as public fields: values as values and borrowed ones as references (`event.kind`). A
number or `bool` a mod may change is a plain field, written back once every mod has run, even after a `Cancel`;
anything else a mod may change, a game struct or a list, is a `&mut` field. A cancellation or replacement is still
the return value; a collection hook's list is a field (`event.lines`). One event goes through every mod, so each
sees the changes before it. Because no mod can build or match the whole
struct, a later version can add a field without breaking any mod, as NeoForge's event objects do; the cost is that
a mod's own tests can't build an event yet.

Hook methods take `&self` and require `Send + Sync`. The process-wide mod list is installed once, like mixin module statics. One mod instance serves both
the window's game and the server-thread game. A hook can also fire again inside
the same mod: for example, `set_block` fires `block_changed`. Mods keep mutable
state with interior mutability so these calls can share the instance safely. Data-load
hooks can run on any loading thread; item default changes must be identical on every side.

The trait has every hook carrying a `mod_api` entry in `native-hooks.json`.
Level hooks use access-specific hosts rather than `Host`. `LevelHost` provides
`block_state` through `LevelAccess::block_state`, client-side detection and
`state_with_property`. `LevelRandomHost` provides vanilla's `Level.random` so
mod draws come in vanilla's order. `LevelTicksHost` adds scheduling through
`native_schedule_block_tick` and `native_schedule_fluid_tick`, while
`LevelWriteHost` adds random access and `native_set_block_property`. Whole-block writes wait for a
later slice that creates block entities, as vanilla's `setBlock` does. The game owns
the tick queue; mods do not pass it back to these helpers.
For frame hooks, implement `RenderHooks` and return it from `Hooks::render`. The ordered
renderer list is installed once alongside the mod list. `render_needs` collects requests;
`world_drawn` observes the world pass and can encode drawing before the hand and HUD. Both
run on the render thread while the frame is built, before any deferred GPU submission.
The renderer module re-exports the game's wgpu types.
### Server attachments

A mod can declare an `AttachmentType<T>` once and call its `get`, `set` and `remove`
methods with the hook's host and a `Holder`. `T` is `Clone + Send + Sync + 'static`;
`get` returns a clone and inserts the default on first access. `set` runs the optional
codec before changing the value, returning an `AttachmentError` if it cannot encode.
Keys are the pair `(namespace, name)`; a key already decoded as another Rust type is
an error. A missing or non-server holder is an error, not a new phantom holder.
`AttachmentError` distinguishes `MissingHolder`, `TypeMismatch`, `Decode`, `Encode`,
`InvalidNamespace` (a malformed saved compound) and `InvalidCopyPolicy` (death-copy
without a codec).

```rust
use horizon_client::mod_api::{AttachmentCodec, AttachmentError, AttachmentType, Compound, Holder, Tag};

static COUNTER: AttachmentType<i32> = AttachmentType {
    namespace: "example", name: "counter", default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("count".into(), Tag::Int(*n))].into()),
        decode: |data: &Compound| match data.get(&"count".into()) {
            Some(Tag::Int(n)) => Ok(*n), _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
};
// Inside a server hook with `host: &mut dyn Host`:
let old = COUNTER.get(host, Holder::Chunk([0, 0]))?;
COUNTER.set(host, Holder::Chunk([0, 0]), old + 1)?;
Ok::<(), AttachmentError>(())
```

Use `Holder::Level`, `Holder::Chunk([x, z])`, `Holder::BlockEntity([x, y, z])`,
`Holder::Entity(id)`, or `Holder::Player(uuid)`. Entity IDs cover mobs, item
entities, boats, minecarts, armor stands, cushions, item frames, paintings and the
server's `OtherEntity` records; projectiles, orbs and other ID-less save paths are
not covered. Player UUIDs cover online guests and the acting player. Mutable level
hosts can access their level, loaded chunks and block entities; read-only level hosts
cannot access attachments. These maps live on the server only: there is **no client
synchronization**. Unserialized values disappear on unload or restart. A saved value
is stored under `horizon:attachments`, then the mod namespace and attachment name,
as `{value: <compound>, copy_on_death: 1b}` (the flag is omitted when false).
Unknown mods' saved entries survive load and save without being decoded. Level
attachments use the dimension's `data/minecraft/horizon_attachments.dat` SavedData
file, not `level.dat`.

`copy_on_death` requires a codec. Player respawn copies opted-in entries via their
encoded representation, including raw entries of an absent mod; other entries are
removed. Living-entity conversion is not supported. This follows the default supplier,
optional server-side persistence and player-respawn option of NeoForge 26.3's public
`AttachmentType.Builder` contract, but not its holder-aware supplier, custom copy
handler, conditional serialization or sync APIs.

`Host` exposes the documented `Game` helpers listed under `helpers` in
`native-hooks.json`, without their `native_` prefix. Each keeps the helper's
receiver and documentation. Level services stay on the level hosts above.
The following documented `Game` helpers are not in the trait yet:

- `spawn_mob`, `mob`, `mobs`, `block_in_tag`, `item_in_tag`,
  `server_block_in_tag`, `server_item_in_tag`, `server_block`,
  `set_display`, `sidebar`, `experience_orbs`,
  `item_entities`, `player`, `panel_recipes`, `recipes`, `component`,
  `set_component`, `remove_component`, `spawn_falling_block`, `explode`,
  `fluid_tanks` and `container_slots` use gameplay-private types.
- `with_mod_block_entity` and `trigger_criterion` are generic or take
  `impl Trait`, so their signatures cannot be called through `dyn Host`.

The following catalog hooks are not in the trait yet:

- Newly added upstream hooks `player_command`, `perform_console_command`, `item_decorations`,
  `client_resources_reloaded`, `camera_shake`, `screen_key_pressed_pre`, `screen_char_typed_pre`,
  `screen_mouse_scrolled_pre`, `use_duration`, `use_animation`, `can_eat`,
  `living_entity_use_item_start`, `living_entity_use_item_finish`, `render_hand`, `shader_params`
  and `block_highlight` will join a later slice.
- Newer upstream hooks `biome_colors`, `camera_distance`, `compute_fog_color`, `compute_fov`,
  `data_pack_files`, `mob_submit`, `mouse_button_pre`, `mouse_button_post`, `player_respawned`
  and `server_data_loaded` will join a later slice.
