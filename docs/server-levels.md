# A server's levels: one per dimension

Vanilla's `MinecraftServer` holds a `ServerLevel` per dimension, and each player is in one of them.
Horizon's server does the same. That covers the dedicated server and single player's server thread
(`Game::serving`, the default since #1114). So a LAN world's host and each guest can be in different
dimensions at the same time.

## How the single-level `Game` maps onto vanilla's levels

The game's tick code reads one level: the one held in `Game`'s own fields (the world, its streaming
and save, block ticks, entities, block entities and their per-level queues). `game/levels.rs` keeps
the other dimensions' levels as `Parked` values and swaps one into place (`enter_level`). The swap is
`mem::swap` of each per-level field, so the tick code runs unchanged, one level at a time.

| Vanilla | Horizon |
| --- | --- |
| `MinecraftServer.levels` | the level in place plus `Levels::parked` |
| `MinecraftServer.tickChildren`, every level | `tick_server`: each level with a player in it, in Overworld, Nether, End order. The first pass reads the network and runs the shared parts (clock, weather, functions, autosave). |
| `ServerLevel.players` | `Hub::here`: the guests whose `dimension` is the level in place |
| `ServerPlayer.teleport` into another level | `server_travel.rs`: a Respawn packet, then the teleport once the destination has loaded (portals and `execute in … tp`). An End gateway teleports within the End. |
| `PlayerList.respawn` | `guest_respawn`: into the respawn point's level, the Overworld unless the player has an anchor or a `spawnpoint` elsewhere (`Guest::respawn_elsewhere`), resolved through its block once that level has loaded |
| `ServerLevel.noPlayersEmptyTime`, chunk unloading | `close_idle_levels`: a level other than the Overworld with nobody in it for 300 ticks is saved and closed |
| `CommandSourceStack.getLevel` | `with_source` puts the source's level in place for a command (`execute in`, `execute at` a player in another level) |
| the levels' shared `LevelData` | not swapped: game time, day time, weather, game rules, scoreboard, maps, entity ids (`next_entity`, unique across levels) |

The packets of a guest are read in its own level's pass (`Hub::tick_level`), so its actions and
commands run with its level in place.

The window's own game (`--in-frame`, `--no-server-thread`, `HORIZON_SERVER_THREAD=0`, and the command
harness) keeps one level, which follows its player (`enter_dimension`). LAN guests of a world run that way
stay in the host's dimension. **That mode is deprecated: never use it, it will be removed soon.** It gets no
multi-level work and no compatibility.

## One code path for both

The dedicated server and the server thread are the same code. Each `Game` has an explicit
`ExecutionRole`: `Client`, `IntegratedServer` or `DedicatedServer`, selected before construction in
`Options.execution_role`. Startup, world loading and opening other dimensions use that role. Only
clients mesh chunks and show loading progress; a client can still run its simulation in-frame, which is deprecated and will be removed.
Connecting or disconnecting changes `remote`, independently of the execution role.

The server thread has a hub without a host player. The window joins it through memory as one
more player. It still sets `options.dedicated` for gameplay checks outside the role migration's
startup/world-loading scope. Everything above serves both servers, and `server_dimension_players`
(vanilla clients on Horizon's dedicated server) and `lan_e2e --server-thread` (Horizon clients on the
server thread) check it from the two sides.

## Cost of an extra level

Measured on the Mac mini (M4 Pro) at render distance 12, on 2026-10-22. The host and a guest each
played a fresh world on the server thread. RSS is the host's whole process, window included. Tick time
is `/tick query`'s average over 100 ticks.

| Where the players are | Average tick | P95 | Host RSS |
| --- | --- | --- | --- |
| both in the Overworld | 1.0 ms | 1.3 ms | 1194 MB |
| guest in the Nether (two levels tick) | 2.2 ms | 2.9 ms | 1238 MB |
| guest in the End (the Nether still parked) | 0.9 ms | 1.1 ms | 1324 MB |
| both back, the Nether and End closed after 300 idle ticks | 0.8 ms | 0.9 ms | 1335 MB |

- A level costs only while a player is in it: its chunks around that player, their entities, and the
  tick. With nobody outside the Overworld, `tick_server` runs one pass, as before.
- A closed level's memory goes back to the allocator, not always to the system.
- Entering a level the first time opens its region files and streaming (`open_level`). That cost
  comes once, before the player's chunks load.

## Saving

`save_game` saves the level in place, then every parked level (`save_parked_levels`). Each level has
its own region folder, as in vanilla (`DIM-1`, `DIM1`). A level closed for being idle is saved first.
Each player's saved data holds its dimension (`Dimension`). A guest who leaves in
the Nether, or whose host saves and quits, joins again in the Nether where it was.

## What's checked

`python3 tools/lan_e2e.py --server-thread --check portals --check levels`:

- `portals`: the guest goes to the Nether through a portal while the host stays. The host then goes
  to the End while the guest is in the Nether, so three levels run at once, and both come back.
- `levels`, with the host and the guest each in a level of its own:
  - `execute in the_nether run tp` moves the guest alone. From the Nether, `execute in overworld` and
    `execute at` the host change and test blocks in the host's level.
  - Mobs and armour stands fall in two levels at once, and every entity id is unique across the
    levels.
  - The guest leaves in the Nether and joins again there.
  - The guest dies in the Nether and respawns in the Overworld. With a `spawnpoint` and then a
    respawn anchor set in the Nether, it respawns there, and the anchor loses a charge.
  - The host takes a Nether portal alone.
  - The guest takes an End portal, an End gateway and then the exit portal alone.
  - The host saves, quits and opens the world again with the guest in the Nether, and the guest
    joins again there.

## Not done yet

- **End credits for server players.** The first exit from the End should show a player the credits
  (`EndPortalBlock.entityInside`, `seenCredits`). The client's WinScreen is #338.
- **Entities through portals between levels.** Mobs, items and minecarts that enter a portal on a
  server, and portal tickets (`TeleportTransition.PLACE_PORTAL_TICKET`), are not handled.
- **Unlimited selectors.** `@e` without a distance or volume reads only the source's level. Vanilla's
  reads every level.
