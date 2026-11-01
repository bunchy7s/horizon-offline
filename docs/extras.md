# Horizon extras

Optional conveniences built into the client: fullbright, dynamic lights, your own name tag, auto-reconnect,
Mouse Tweaks-style inventory controls and inventory sorting. **Every one is off by default**: a fresh
install looks and behaves exactly like vanilla 26.3, and turning an extra off
again restores vanilla's behaviour completely.

## Where they live

| Part | Code |
|---|---|
| Settings, parsing and saving | `crates/horizon-client-render/src/extras.rs` |
| FPS and click counters (for native mods) | `extras.rs` (pure logic, unit tested) |
| Dynamic lights: item glow, nearest sources | `crates/horizon-client-render/src/dynamic_light.rs` (unit tested), `game/dynamic_lights.rs`, `terrain.wgsl` and `entity.wgsl` (`dynamic_block_light`) |
| HUD drawing (a native mod's minimap, corner stacking) | `crates/horizon-client/src/extras_hud.rs` |
| Minimap colours, tiles and image (for the Minimap native mod) | `crates/horizon-client-render/src/minimap.rs` |
| Reconnect policy and backoff | `crates/horizon-client/src/reconnect.rs` |
| Game glue (input hooks, per-frame update) | `crates/horizon-client/src/game/extras.rs` |
| Mouse Tweaks-style controls, sort buttons | `crates/horizon-client/src/game/tweaks.rs` |
| Sort order and its clicks | `crates/horizon-client/src/inv_sort.rs` (pure, unit tested) |
| The settings page | `options_ui.rs` (`Page::Extras`): **Options → Horizon Settings... → Extras...** |

The settings are grouped by extra, under a header each, two buttons to a row as
vanilla's options lists are, and each says what it does in its tooltip. An
extra's own settings (its key, corner, size...) are greyed out while its switch
is off (`Item::parent`).

`game.rs` only gets one-line hooks (key, wheel, mouse, frame, HUD), so the
extras stay out of the way of other input and options work.

## Options storage

`horizon-extras.txt` beside `options.txt`, in the same `key:value` format as
`horizon-graphics.txt`. `options.txt` is untouched, so a vanilla client can share
the folder. Unknown or broken lines fall back to the defaults.

Extras' keys (the sort key) are Horizon settings stored in that file and bound on
the Extras page, not in vanilla's Key Binds screen: Horizon has no Horizon category
in Controls, and adding one would put a non-vanilla mapping into `options.txt`.

## The extras

### Fullbright (`fullbright`, default off)
- Displays blocks and entities with a fully bright lightmap, including in dark caves.
- Turning it off restores the normal lightmap using the current brightness,
  time of day and effects. The brightness slider keeps its normal range.

### Dynamic lights (`dynamicLights`, default off)
- A light source held by a player or mob (main or off hand), dropped as an item, or an entity on
  fire lights the blocks, entities and hand around it, as dynamic-light mods do. Not
  in vanilla; the idea is ported, not a mod's code (issue #814).
- An item glows like its block's default state (`getLightEmission`): torches 14, lanterns,
  glowstone, sea lanterns and jack o'lanterns 15, soul torches 10, a redstone torch 7; a lava bucket
  15. An entity on fire gives 15.
- Client-only, at render time: the light engine, meshes, saves and other players never see it.
  Each frame the game picks the 16 sources nearest the camera (within 96 blocks,
  `dynamic_light::select`); terrain.wgsl and entity.wgsl raise each vertex's block light to the
  source's level minus its distance in blocks when that is brighter, then light it through the
  lightmap as usual. The light doesn't stop at walls.
- Off: nothing is gathered and the shaders' loop runs zero times (the count is 0), so it costs
  nothing measurable (numbers in the pull request, #814).

### Moved to native mods

The **FPS counter**, **Keystrokes**, **Zoom** and the **Minimap** are official native mods now (in
horizon-rust/horizon-mods under `native/<name>/`, [mods.md](mods.md#horizons-official-mods)), with the same look and
options. Players install them from the launcher's Mods page, and the launcher builds them into the game
([native-mods.md](native-mods.md)). The game keeps
the hook points they use (`crates/horizon-client/src/game/native_hooks.rs`), and `horizon-extras.txt` keeps the old
keys and `movedToMods`.

The game still draws the minimap, for the native mod (`native_minimap` in `native_hooks.rs`, [mods.md](mods.md)):
- a corner map (top right by default), square or round, small/medium/large, zoom 0.5 to 4 blocks per pixel, rotating
  with the player or north up, with the player arrow (vanilla's map decoration sprite);
- vanilla's `MapColor` per block, as map items use, with the map item's height shading and water-depth dithering;
- underground, in the Nether and under any roof, the floor around the player's height instead of the top block;
- one cached 16x16 tile per loaded chunk (packed map colour and height), filled nearest first and refreshed when
  blocks change within 0.3 ms a frame; the 128x128 image is recomposed only when the view moved or a tile changed, and
  uploaded into a dedicated atlas slot. With no mod showing a map, nothing runs.

### Own name tag (`ownNameTag`, default off)
- In third person (F5, either view), the player's own name shows above its head,
  as other players see it: the profile name in its team's prefix, suffix and
  colour, the below-name score under it within 10 blocks, faded and without the
  see-through copy while crouching. Vanilla never names the camera entity
  (`LivingEntityRenderer.shouldShowName`).
- Hidden in first person, while sleeping, in spectator mode, while invisible, and
  whenever the team's name tag visibility would hide it, exactly as the other
  players' tags (`game/names.rs`, `own_name_tag`). Client-only: nothing is sent.

### Auto-reconnect (`autoReconnect`, default off)
- On the "Connection lost" screen after a multiplayer connection drops, a
  countdown "Reconnecting in N s" with a Cancel button, then a new connection
  to the same address.
- Backoff 5, 10, 20, 40, 60 s; at most 5 attempts, then it stops. The count
  only resets after staying connected for a minute, so a server that drops
  players at once never causes an endless loop.
- Never after a kick, a ban, a whitelist or version refusal, a duplicate login,
  an online-mode refusal (no account signed in), or a session refusal from the
  account's login (invalid session, banned from playing online, multiplayer
  disabled, the server failing to verify the username). Unreachable session
  servers, on either side, are retried with the same backoff.
- Online-mode servers: the retry joins with the account the game signed in
  with, held in memory for the whole run (`auth::current()`); nothing is
  written to disk.

### Mouse Tweaks controls (`mouseTweaks`, default off; `mouseTweaksWheel`, default on)
In the player inventory, chest-like containers (chests, barrels, ender chests,
shulker boxes, hoppers, dispensers and droppers), furnaces (also blast furnaces
and smokers) and crafting tables, in singleplayer and on servers. Anvils,
enchanting tables, merchants, beacons, brewing stands, crafters and the other
workstations keep vanilla's controls:
- Right-drag holding a stack: one item goes into each slot entered, at once
  (slots holding another item, or full, are skipped).
- Left-drag holding a stack: each slot entered that holds the same item is
  gathered into the hand, when it all fits. This takes over only while
  vanilla's drag covers one slot, so a left-drag across empty slots still
  splits the stack as vanilla does.
- Shift + left-drag: every slot entered is quick-moved (with a stack in hand,
  only the slots of its item).
- The wheel over a slot, with an empty hand (`mouseTweaksWheel`): down moves
  one item where a Shift click would send it (vanilla's `quickMoveStack`: fuel
  to the fuel slot, a smeltable to the input, into the crafting grid, across a
  container, between hotbar and main inventory, an armour piece onto the
  player), up brings one of the same item from the other side.
- Nothing goes into a result slot. The wheel down over one takes one craft, or a
  furnace's whole output, to the inventory when it all fits (a result can't be
  split or put back); the wheel up over one does nothing, and it isn't a source
  for the wheel up elsewhere. A right-drag skips it, and a right press on it is
  vanilla's.

Every action is ordinary slot clicks (`ContainerClick` on a server), so a
vanilla server sees nothing unusual. With the extra off, the hooks return at
once and the screens click exactly as vanilla's (`tools/test_menu_click_parity.py`
and the container screen tests are unchanged).

### Sorting (`sort`, default off; `sortKey`, unbound)
- A small button right of the container's title (chests, barrels, ender
  chests, shulker boxes) and one right of the "Inventory" line (the main
  inventory; the hotbar is left alone). The key, when bound on the Extras
  page, sorts the part under the cursor (the container when over no slot).
- Partial stacks of one item merge, then the stacks go in creative tab order
  (the search tab's), then by name; full stacks before the partial one,
  empty slots last. It needs an empty hand.
- Done with ordinary left clicks planned by `inv_sort::plan`, so a vanilla
  server accepts it; a unit test replays the planned clicks through the menu
  engine on random chests.

## Shared rules

- All extras are hidden with F1 and drawn in the HUD layer: under screens, over
  the world, like vanilla's HUD. Extras in the top left corner give way to the
  debug screen (F3), which shows the FPS anyway.
- Corners stack: extras placed in the same corner stack downward (top corners)
  or upward (bottom corners) in the order a mod's minimap, then mods' panels. The top
  right corner starts below the status effect icons.

## Perf budget

Off: a flag check per frame per extra (no allocations, no world access).
A mod's minimap stays under 0.3 ms of CPU
per frame (tile budget) plus a 64 KiB texture upload when its image changes.
