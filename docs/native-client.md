# Horizon native client

`horizon-client` is the playable Rust client for Minecraft Java Edition 26.3.
It streams a world around the player on worker threads, renders every vanilla
block model with smooth lighting and biome tints, and runs the validated player
physics, health and hunger at 20 ticks per second. You can mine with vanilla
hardness/tool rules, collect drops from the block loot tables, place blocks,
craft in the 2x2 inventory grid or on a crafting table, and save and reload worlds
in vanilla's Anvil format.

**Status:** playable; players get it through [the launcher](../launcher/README.md). The [Java parity table](../README.md#project-status)
in the README scores each subsystem against vanilla and lists what is still
missing; [Rust rewrite status](rust-parity.md) has the details.

## Run from a checkout

Use current stable Rust (1.90+) and JDK 25 (Python 3.10+ only for the development
harnesses). A checkout never
downloads the game: resource preparation imports **your own copy** (the 26.3
client from your official launcher install, and the 26.3 server jar you
downloaded yourself; see [Build from source](../README.md#build-from-source)),
verifies it against the pinned checksums, runs vanilla's data generator and small
exporters against those jars, and extracts the jar's assets and data into ignored
`.local/horizon-client/26.3/`. Sounds (about 380 MB) are copied from your
launcher's asset store by `horizon-prepare sounds`, each file checked against the asset
index's SHA-1. Preparation also tries to replace the JAR's gray 1×1 title panorama
placeholders with the full textures from that asset store. If the store is missing or
incomplete, preparation logs a warning and keeps the placeholders. To import the panorama
later without rerunning the Java exporters, run `horizon-prepare panorama` (or
`cargo run --release -p horizon-prepare -- panorama` in a checkout). Pass
`--minecraft-dir PATH` when using a different Minecraft installation.
Prepared resources are private; never commit them.

Preparation is `horizon-prepare` (`crates/horizon-prepare`, Rust): it needs no Python, only
the JDK, for the small exporters that call Mojang's own code. Release folders ship it beside
`horizon-client`, and the launcher prepares with it and copies the sounds as part of preparing.

```sh
cargo run --release -p horizon-prepare -- --prepare --server-jar ~/Downloads/server.jar
cargo run --release -p horizon-prepare -- sounds
cargo run --release -p horizon-client -- --render-distance 12
```

Development builds that link quickly: `cargo build --profile fast -p horizon-client`.

| Option | Meaning |
| --- | --- |
| `--world NAME` | World folder under `saves/` (default "New World"); reopened if it exists |
| `--seed N` | Seed for a new world (numbers or text, like vanilla) |
| `--creative` | Creative mode (instant breaking, pick block) |
| `--render-distance N` | Chunks, 2..128 (default 12) |
| `--resources PATH` | Prepared resource directory |

| Control | Action |
| --- | --- |
| Mouse | Look (vanilla default sensitivity) |
| WASD / Space / Left Shift / Left Ctrl | Move, jump, sneak, sprint |
| Left / right click | Break (hold) / place or use; right click opens crafting tables |
| Middle click | Pick block (creative) |
| 1–9, scroll | Select hotbar slot |
| E | Inventory and 2x2 crafting (shift-click to quick-move or craft all) |
| Q / Ctrl+Q | Drop one / drop stack |
| F3 | Debug overlay (position, chunk, light, biome, FPS) |
| Esc | Pause menu (Save and Quit) |

Worlds autosave every 6000 ticks (5 minutes), when chunks unload and on quit.
On headless Linux use `vscreen start ./target/release/horizon-client` and
`vscreen shot`.

Sound: the game prints the output it opened (`Horizon: audio output ...`) or
why it has none (`Horizon: no sound: ...`). It opens none with
`HORIZON_NO_AUDIO` set, nor on a `vscreen`, where tests run beside someone
listening; `HORIZON_AUDIO=1` opens it there anyway, so send it to a null sink
(`PIPEWIRE_PROPS='{ target.object=<sink> node.dont-fallback=true }'`). Don't
mute a test run's stream in the mixer: PipeWire remembers the mute for every
later `horizon-client`. The launcher starts the game as "Horizon" for that
reason.

### Agent control (`horizon-ctl` and MCP)

AI agents and scripts play Horizon through `horizon-ctl`, a command line that
reads the game as JSON and acts like a player, and `horizon-ctl mcp`, an MCP
server with the same commands. `--control` makes the game accept them on a
local socket; under control, losing window focus does not pause the game or
release held keys, so a script can play while you use other windows.

```sh
cargo build --release -p horizon-client --bins
C=target/release/horizon-ctl
$C launch --world Survival --seed 5 --wait   # start the game, open or create the world, wait for it
$C state                                     # player, world, screen, crosshair target
$C find-block birch_log                      # the 16 nearest loaded logs
$C goto -0.5 65 -2.5 --near 2.5              # walk there along a path
$C break -1 65 -3                            # mine it: {"drops": {"minecraft:birch_log": 1}, ...}
$C inventory                                 # hotbar, inventory, armor, offhand, counts
$C open-inventory                            # craft planks in the 2x2 grid:
$C click-slot 0; $C click-slot 100 right; $C click-slot 0; $C click-slot 200 shift; $C close
$C place 2 65 -2 --item birch_planks         # {"changed": true, "block": {...}}
$C pause; $C ui                              # the pause menu's widgets: ids, kinds, labels, bounds
$C click options; $C set fov 0.75; $C click done   # widgets by id
$C screenshot /tmp/view.png                  # PNG of the next presented frame
$C quit                                      # save and exit
```

`horizon-ctl help` lists the commands by group and `horizon-ctl COMMAND --help`
explains one with examples; `horizon-ctl help --game` lists what the running
game accepts, test hooks included.

- **Observe:** `state` (position, rotation, velocity, health, food, saturation,
  XP, effects, game mode, dimension, time, weather, the screen and what the
  crosshair points at), `looking-at`, `entities [RADIUS] [TYPE]` (mobs with
  ids and health, dropped items, orbs), `block X Y Z`, `blocks X1 Y1 Z1 X2 Y2 Z2`, `light X1 Y1 Z1 X2 Y2 Z2`,
  `find-block BLOCK [RADIUS]`, `inventory` (stacks with their components as
  SNBT), `screen` (the open screen or title menu, its buttons and slots) and
  `messages` (chat, title, subtitle and action bar).
- **Move and act:** `move DIR SECONDS`, `jump`, `goto X Y Z` (A* over standable
  blocks: steps up one block, drops up to three, avoids lava), `look`,
  `look-at` a point, block face or entity, `attack [ID]`, `break X Y Z`,
  `place X Y Z [--item ITEM]`, `interact X Y Z` (doors, chests, crafting
  tables), `use-item [--hold S]` (eat, drink, throw), `select SLOT|ITEM`, `drop`.
- **Widgets:** `ui` lists every widget of the open screen (title menus, pause,
  options pages, inventories and menus): an id, its kind
  (`button`, `slider` with its 0..1 `value`, `field`, `row`, `tab` with
  `selected`, `slot` with its stack), the label and its bounds. Ids come from
  labels (`FOV: 70` is `fov`, `Video Settings...` is `video-settings`, a
  world row is the world's name; a repeat gets `-2`), else `row-0`, `field-0`,
  and slots are `slot-ID`. `click ID [right] [shift]` clicks one as the mouse
  would and `set ID VALUE` moves a slider to 0..1; after a click, the next
  `ui`, `click` or `set` waits for the screen to be redrawn. Design and plans:
  [agent-control.md](agent-control.md).
- **Options:** `options-get [KEY...]` gives every `options.txt` option by its
  vanilla key with its value and what it accepts (`{"type": "int", "min": 30,
  "max": 110, "value": 70}`; enums with `choices`, key bindings as
  `key.keyboard.w`); `options-set KEY VALUE` sets one, applies it at once and
  saves `options.txt` as the options screen does. A value the option refuses
  is an error saying what it accepts, not vanilla's silent reset.
- **Waits and events:** `wait-for screen NAME|widget ID|loaded|chat TEXT|event
  KIND [--timeout S] [--since SEQ]` replies once the game gets there (a timeout
  is an error with `"kind": "timeout"`), so scripts don't poll. `events [--since
  SEQ]` lists what happened, oldest first, each with a `seq` and the `tick`:
  `screen` (from, to), `chat` (text), `death`, `respawn`, `item` (item, delta),
  `dimension`, `world` (opened, closed). Take `last` from `events` before an
  action whose event you then `wait-for --since` it, so a fast one isn't missed.
- **Screens:** `open-inventory`, `click-slot SLOT [right] [shift]` (0-8 hotbar,
  9-35 inventory, 100+ crafting grid, 200 result, 300+ containers, 500-503
  armor, 504 offhand, 600+ stations), `close`, `click-button LABEL` (any
  button of any screen or title menu, by its label),
  `click-at X Y`, `type TEXT` (on the create-world screen `--field name|seed
  --replace`), `chat`, `command`, `respawn`, `pause`, `resume`.
- **World:** `tp`, `give`, `time`, `setblock`, `summon`, `wait TICKS`, and the
  benchmarking commands `bench`, `stats` and `fly` (below).

Actions block until they finish or time out and reply with the outcome: `goto`
says whether it arrived, `break` what dropped and what was picked up, `place`
the block placed. `stop` cancels the running action. Every reply is one JSON
line; errors are `{"error": "..."}` objects, and the exit status is 0 on
success, 1 when the game replied with an error, 2 for a usage error and 3 when
the game cannot be reached or started. `horizon-ctl script FILE` (or stdin)
runs one command per line with shell-like quoting and stops at the first error
(`--keep-going` doesn't); `sleep S` waits. `horizon-ctl -` is the older form
that reads stdin and keeps going.

`launch` finds the game to start: `--game PATH`, else a `horizon-client` next
to `horizon-ctl` (a build folder or a release folder), else the newest version
the launcher installed (`--version TAG` picks another), run with its prepared
resources from the game home, where the saves are. So after installing with
the launcher, `horizon-ctl launch --world X --wait` is all an agent needs.
Other arguments go to the game; its output goes to a `.log` file beside the
socket.

**Logs and crash reports.** Like vanilla, the game writes `logs/latest.log`
in the game home (`$HORIZON_HOME`, else `~/.local/share/horizon`; the
dedicated server in its own folder) with lines like
`[12:04:31] [Render thread/INFO]: message`, and at start archives the last
run's as `logs/<yyyy-mm-dd>-<n>.log.gz` (seven a day). Messages still print on
stderr as before. A panic on the main thread or a fatal error writes
`crash-reports/crash-<yyyy-mm-dd_HH.MM.SS>-client.txt` (the error, its stack,
the machine, the GPU, render settings, resource packs, the open world and the
last 50 log lines) and the game exits with a non-zero code. `horizon-ctl logs
[--lines N]` prints the end of latest.log and `horizon-ctl logs --crash` the
newest report (the `logs` MCP tool returns the same). F3 + C held for ten
seconds crashes the game on purpose, as in vanilla (the world is saved first);
development builds also crash with `HORIZON_DEBUG_CRASH=SECONDS`, that long
after the first frame.

**MCP.** `horizon-ctl mcp` serves the Model Context Protocol over stdin and
stdout (JSON-RPC 2.0, one message per line), with one tool per command and a
JSON schema for each; `screenshot` returns the frame as an image. Register it
in an MCP client as a stdio server with the command `horizon-ctl` and the
argument `mcp` (`--socket PATH` before `mcp` to pick a socket).

**Transport.** On Linux and macOS the endpoint is a Unix socket:
`$HORIZON_CONTROL`, else `$XDG_RUNTIME_DIR/horizon-control.sock`, or
`--control-socket PATH` (`horizon-ctl --socket PATH`). Windows has no Unix
sockets: there the game listens on a random `127.0.0.1` port and writes
`{"transport":"tcp","port":N,"token":"..."}` to the endpoint path
(`%TEMP%\horizon-control.json` by default), readable only by its user; the
client's first line must be `auth TOKEN`. Any endpoint path ending in `.json`
selects that transport on Linux and macOS too.

`tools/agent_control_e2e.py` checks it all on a virtual screen: it creates a
world through the title menus, walks to a tree, breaks a log, crafts planks in
the inventory screen and places one, through the CLI and through MCP.

`fly X1 Y1 Z1 X2 Y2 Z2 SECONDS [PITCH]` moves the camera along a line at
constant speed and replies with the same summary as `bench` (avg/1%-low FPS and
p50/p95/p99/max frame times), plus CPU and GPU time per frame; [the graphics
benchmark](graphics.md#benchmark) is built on it. Frame times include waits for
presentation, so `bench` reports what is on screen.

### Discord Rich Presence

While Discord runs on the same computer, the player's Discord profile shows
"Playing Horizon" and what they are doing: "In the menus", "Singleplayer",
"Multiplayer" or "Hosting a world" (with the party size, of 8), the dimension
("In the Nether") and the time since they entered the world. It never shows a
world's name or a server's address. Horizon Settings has the switch
(**Discord Rich Presence**, on by default; `discordPresence` in
`horizon-extras.txt`), and `HORIZON_NO_DISCORD=1` turns it off for one run.

`crates/horizon-client/src/discord_presence.rs` speaks Discord's local IPC
itself (the `discord-ipc-N` socket or named pipe, no SDK) from a thread of its
own: the frame only hands it a status when it changes. It retries every 15 s
while Discord isn't running and stays within Discord's 5 updates per 20 s.
Dedicated servers never start it, and automated runs (offscreen, benchmarks)
only talk to a fake Discord: `lab.game_env` sets `HORIZON_NO_DISCORD=1`, and
`tools/discord_presence_e2e.py` points games at a fake server with
`HORIZON_DISCORD_IPC_DIR` (title, world, Nether and quit on both single-player
paths; a LAN host and guest; the switch; a Discord started after the game).
`DISCORD_APP_ID` is Discord's "Horizon" application, whose `horizon` image is
the large one; there are no game-mode images yet.

### Play on another of your machines

Horizon targets Linux on x86-64 with an NVIDIA GPU (Vulkan); macOS and Windows
are out of scope. To play on another Linux machine
without Rust, JDK or prepared resources, package a release build where they exist:

```sh
cargo build --release -p horizon-client
python3 tools/package_native_client.py --out ~/builds/horizon
```

The archive holds the binary, the prepared resources and `horizon.sh`. Copy it
to the target machine (same OS and architecture), unpack it and run
`horizon/horizon.sh`; `horizon.sh --install` adds an application menu entry.
Saves and `options.txt` go to `~/.local/share/horizon` (or `$HORIZON_HOME`), so
unpacking an update keeps them. The archive contains Mojang assets: keep it
private.

## How it fits together

- `streaming.rs`: worker pool that generates or loads, lights and meshes chunk
  columns closest-first; stale results are discarded by sequence number.
- `light.rs`: vanilla sky/block light rules (sky sources down to the first
  occluding edge, `max(1, dampening)` falloff) over a 3x3 chunk region.
- `mesher.rs`: blockstate models with neighbour culling, `ModelBlockRenderer`
  smooth lighting/ambient occlusion, `BlockTintSources` tints with 5x5 biome
  blending, and `FluidRenderer` water/lava surfaces.
- `gfx.rs` + `terrain.wgsl`: camera-relative section rendering with frustum
  culling, the vanilla lightmap formula, timeline-driven sky/fog, and
  solid/cutout/translucent passes; `entity.wgsl` draws dropped items.
- `environment.rs`: the data-driven day timeline (sky colour, light factor,
  celestial angles) sampled with vanilla's keyframe/easing rules.
- `items.rs`, `crafting.rs`: item components from vanilla's reports, tool
  mining rules, loot tables and crafting recipes from the data pack.
- `save.rs`: Anvil region files and `level.dat`.

## Validation

The client loads a separate, registry-checked collision-shape export alongside
render resources. `SceneCollisions` feeds its actual `TerrainChunk` instances
into registered shape selection, block scanning, world movement resolution and
occupancy checks. The scene has no other entities, block entities or border;
entity-dependent queries report an error until an entity runtime is supplied.
Camera collision uses a position context and disables stepping. The walking
binding uses the current pose bounds, descending input and registered step-height
attribute. Sneaking invokes ledge backoff before collision resolution. The
complete authoritative player tick remains unfinished.

```sh
cargo run --release -p horizon-prepare
python3 tools/test_native_scene_collision.py
```

The [scene collision comparison](../benchmarks/rust-native-scene-collision-2026-10-07/scoped-parity.json)
checks **14,150 movement requests**, each with before/after occupancy queries,
against both pinned reference JARs. Native code independently builds the nine-
and twenty-five-chunk scenes and all sixteen gallery placements. Movement
components compare by exact double bits. Cases include partial blocks, pyramid
ledges, ceilings, initial overlaps, negative and missing chunk boundaries, tiny
and zero movement, and varied step heights. This verifies the scene adapter;
it does not establish player locomotion or complete gameplay parity.

The movement binding stores a player's position, velocity, pose bounds,
ground/support state, collision flags, movement history and live registered
attributes. `MotionWorld` composes the existing air-travel, jumping, collision,
ground-support, restitution and block-coefficient code over the same scene.
The [stored-motion comparison](../benchmarks/rust-native-scene-motion-2026-10-07/scoped-parity.json)
checks 42,257 operations across 637 evolving scenarios against actual native
`Player.travel`, jumping and movement, using real native attributes and chunks.
It compares every resulting position, velocity and bounding-box component by
exact bits, along with support, climbing and collision state and both separate
velocity/position synchronization flags.

The dry movement tick additionally binds small-velocity cleanup, float input
damping, held-jump delay and server frost-modifier removal. The expanded
[tick comparison](../benchmarks/rust-native-motion-tick-2026-10-07/scoped-parity.json)
checks 47,393 operations in 847 scenarios against both pinned JARs, including
direct calls to the actual `LivingEntity.aiStep` body. It compares lazy attribute
instance creation, stored base/cache/dirty state, modifier values and permanence,
and pending update/sync membership alongside the motion fields. Queue iteration
order depends on runtime identity hashes and is outside this scene comparison.
The test caught an omission where matching positions concealed a retained
powder-snow modifier and missing attribute notifications; dry ticks now remove
that modifier after travel, as vanilla does.

The [walking movement comparison](../benchmarks/rust-native-walking-2026-10-07/scene-motion.json)
extends this to 65,893 operations in 947 scenarios, invoking actual
`Player.aiStep` with empty inventory/equipment and no nearby entities. It checks
head yaw, jump-trigger countdown and speed refresh after movement. Prepared
input is supplied at the native virtual input boundary. Separately, the
[local input comparison](../benchmarks/rust-native-walking-2026-10-07/local-input.json)
checks 13,988 cases against actual `KeyboardInput.tick` and
`LocalPlayer.modifyInput`, covering every key combination, diagonal adjustment,
item/riding/sneaking speed gates, signed zero and nonfinite values. This verifies
input arithmetic; the sprint and crouch integration checks below extend it.
The full `LocalPlayer` lifecycle remains unfinished.

The fixed-rate driver limits work per rendered frame while retaining overdue
ticks. Clock tests cover varied frame schedules and long stalls. The development
focus pause is explicit; no vanilla pause-menu or multiplayer timing is claimed.
The [Linux walking smoke check](../benchmarks/rust-native-walking-2026-10-07/interactive-smoke.json)
enabled F5, landed at ground height 4, walked, jumped, switched modes and exited
with Ctrl+Q. Ground, moved and airborne screenshots were visually inspected;
the run recorded 207 movement ticks and finished grounded without an error.
The [renderer startup check](../benchmarks/rust-native-walking-2026-10-07/startup-smoke.json)
also passes after the movement integration.

```sh
cargo run --release -p horizon-prepare
python3 tools/test_native_scene_motion.py
python3 tools/test_local_input_parity.py
```

This binding is not yet a complete player tick or input controller. Its current
domain is the dry resident scene with standing/crouching/crawling dimensions,
zero frozen ticks, empty inventory/equipment, and no entity interpolation,
vehicles or status effects. Selected `Player.aiStep` phases are bound; the full tick and local input
lifecycle remain unfinished.
Landing now accumulates and resets the stored fall distance, clears climbing
state on ground, and invokes registered block fall/bounce reactions. The
[composed landing comparison](../benchmarks/rust-native-landing-2026-10-07/scene-motion.json)
checks 73,025 operations in 1,449 scenarios against both pinned JARs. It compares
fall state, impulse exemptions, attributes and ordered damage, statistic,
particle, sound and hit-ground requests alongside the previous motion fields.
Actual native `LivingEntity.checkFallDamage`, registered block methods and
`Player`/`LivingEntity.causeFallDamage` execute in the reference harness.
The cases include hay, honey, slime, powder snow, pointed dripstone, farmland,
turtle eggs and shelf mushrooms, with seeded world randomness. Farmland pushes
the stored player and changes the resident block to dirt; repeated egg changes
preserve hatch state. The resulting block state is checked after each operation.

The client refreshes its mesh after a resident block revision changes. This is
a complete scene rebuild for now, with no performance improvement claimed.
Damage requests are counted in runtime reports, and the health binding described
below applies them synchronously. Full death/respawn, audiovisual playback and
networking still need consumers. Block writes currently update
resident storage; neighbor updates, drops and block-entity lifecycle remain
unfinished. Block-inside effects such as cobweb slowdown and the full entity
lifecycle are also unfinished. The reference harness explicitly controls these
boundaries, which remain outside the comparison's scope.
The earlier [scene collision comparison](../benchmarks/rust-native-scene-motion-2026-10-07/collision-regression.json)
and [renderer startup](../benchmarks/rust-native-scene-motion-2026-10-07/startup-smoke.json)
also pass with this binding and the additional prepared movement resources.

The accompanying [startup check](../benchmarks/rust-native-scene-collision-2026-10-07/startup-smoke.json)
passes after sharing scene loading between the GUI and headless collision probe.
An [interactive Linux check](../benchmarks/rust-native-scene-collision-2026-10-07/interactive-smoke.json)
enables F4, holds Shift to descend, verifies that the camera stops at the terrain
surface, and exits with Ctrl+Q. The window was visually inspected on Vulkan
llvmpipe; this is not a hardware performance measurement.

The separate `horizon-assets` crate decodes the pinned 26.3 cuboid model format.
Its native comparison covers all **2,883 block-model JSON files** in the client
JAR plus **2,457 synthetic cases**: element bounds, optional geometry, nullable
children, face UVs and quarter turns, single-axis/Euler rotations, texture
references and translucency materials, light emission, display transforms and
inheritance markers. Floating-point fields compare by their exact bits. The
comparison also covers primitive coercion, integer truncation/wraparound and
selected strict-reader numeric/syntax limits. The numeric scanner also preserves
vanilla's rejection when its wrapping integer accumulator hits zero before
another digit, including in ignored fields. Twenty newly exposed acceptance
mismatches were corrected; the complete comparison now covers 5,340 models.
The [resolution](../benchmarks/rust-resource-number-scanner-2026-10-06/model-resolution.json),
[baking](../benchmarks/rust-resource-number-scanner-2026-10-06/model-baking.json) and
[animation](../benchmarks/rust-resource-number-scanner-2026-10-06/sprite-animation.json)
comparisons also pass with the shared scanner correction.

```sh
python3 tools/test_cuboid_model_parity.py
```

The [archived decoder result](../benchmarks/rust-resource-number-scanner-2026-10-06/model-decoding.json)
records source, fixture and reference hashes. The oracle uses actual client
classes; those rendering APIs are absent from the server. The decoder, parent
resolver and baker now feed the development gallery. Blockstate selection, transformed
models and UV locking have the separate checks described below. Rendered
appearance still needs its own checks.

`ModelLibrary` resolves parent geometry, ambient occlusion, GUI lighting and
each display context. It shares inherited elements rather than cloning them.
Texture aliases bind after child overrides are applied; successful and failed
alias walks are cached. Missing resources use the native missing-model cube;
parent cycles invalidate their descendants. The
[resolution comparison](../benchmarks/rust-model-resolution-2026-10-06/scoped-parity.json)
matches **5,648 requested roots across 439 graphs**, including the full bundled
block-model catalog, missing parents, 702 cyclic roots, 1,024-model parent chains,
512-slot alias chains, explicit empty geometry and identity/null transforms.
It calls native `ModelDiscovery`, `ResolvedModel` and `TextureSlots` APIs.

```sh
python3 tools/test_model_resolution_parity.py
cargo +stable test --locked -p horizon-assets
```

Cuboid baking now produces positions in block units, normalized unit-atlas UVs,
face winding and facing, material references, tint indices, shade overrides and
light-emission metadata. It supports single-axis and Euler element rotations
with the pinned rescaling rule. The
[baking comparison](../benchmarks/rust-model-baking-2026-10-06/scoped-parity.json)
matches **7,100 roots across 440 graphs**, using native `FaceBakery` with a real
sprite and the default JOML math options. All bundled block models bake without
errors. Synthetic coverage includes reversed/degenerate bounds, signed zero,
arbitrary rotations, rescaling, null geometry children and parent cycles.
Position, UV and matrix fields compare by exact float bits for these fixtures.

```sh
python3 tools/test_model_baking_parity.py
```

That comparison uses the identity blockstate transform. Atlas packing,
transparency classification, actual tint/light evaluation and GPU rendering
remain separate from the geometry result. Other operating systems still need
their native validation runs.

`BlockModelTransform` now prepares all 64 XYZ quarter-turn combinations once
and shares them across baked models. It rotates geometry around the block center,
rotates culling directions and applies UV locking in the original face basis.
The [transformed baking comparison](../benchmarks/rust-blockstate-transforms-2026-10-06/scoped-parity.json)
checks every distinct model/rotation/UV-lock tuple in bundled blockstates and
all 128 rotation/lock combinations on 81 synthetic models, plus further arbitrary
element rotations. Positions, UVs and every orientation/inverse-face matrix
compare by exact float bits: 19,704 roots and 148,439 quads match, including
7,884 bundled model/transform tuples. Quaternion basis rounding and numeric identity
rechecking are retained. This remains geometry validation; atlas packing and pixel parity are separate work.

```sh
python3 tools/test_blockstate_transform_parity.py
```

Blockstate resources now decode into ordered variant selectors and multipart
conditions. The [native blockstate comparison](../benchmarks/rust-blockstate-model-decoding-2026-10-06/scoped-parity.json)
covers all 1,288 bundled definitions and 1,351 synthetic cases (2,639 total,
468 expected rejections). It uses the same `StrictJsonParser` and
`BlockStateModelDispatcher.CODEC` as the pinned loader. The Rust representation
preserves single versus weighted variants, all three quarter-turn axes, UV-lock
flags, positive weights and total-weight limits, ordered selectors, nested
AND/OR conditions and per-term negation.

```sh
python3 tools/test_blockstate_model_parity.py
```

Property binding now selects variant roots or multipart part indices for an
ordered native state list. The [binding comparison](../benchmarks/rust-blockstate-model-binding-2026-10-06/scoped-parity.json)
covers all 1,288 bundled definitions against 35,723 registered block states plus
the four special item-frame states, followed by synthetic overlap, malformed
selector and predicate cases: 3,802 definitions and 687,466 state results match,
with 220 expected binding errors. It checks the native replacement-before-abort
behavior for overlapping variants, eager multipart validation even when variants
cover every state, integer-property aliases, nested conditions and empty part
selections. The controlled chunk renderer now uses this binder for its model
catalog.

The Java integer-property parser is shared by `horizon-core`, client binding and
world-state decoding. Its [codec regression](../benchmarks/rust-blockstate-model-binding-2026-10-06/shared-integer-codec.json)
matches 221,261 output lines against each authenticated client/server JAR.

```sh
python3 tools/test_blockstate_binding_parity.py
```

`CompiledChoices` prepares cumulative weights once per definition and shares
those choices across bound states. Selecting an ordinary single variant consumes
no random draw; a weighted singleton still does. Multipart collection draws a
seed even for an empty subset and resets the same stream before each chosen
part. The [native choice comparison](../benchmarks/rust-blockstate-model-choice-2026-10-06/scoped-parity.json)
uses actual `WeightedVariants.collectParts` and `MultiPartModel.collectParts`
with inert observable leaves. All seven distinct bundled weight patterns,
synthetic large weights and random multipart graphs produce matching selections
and subsequent random values: 6,960 fixtures, 111,360 collections, both legacy
and xoroshiro streams. The development gallery now bakes and renders selected
models using the legacy stream and the default block position seed. Specialized
block seed callbacks remain separate work.

```sh
python3 tools/test_model_choice_parity.py
```

The sprite animation module reproduces the pinned metadata codec's integer
conversion, strict primitive types, optional null fields and frame dimensions.
It preserves repeated frames, filters out-of-image frame indices, and treats
sequences with fewer than two valid entries as static physical frame zero.
Frame lookup uses cumulative durations and binary search, so skipped rendering
frames do not require replaying all elapsed animation ticks. The native oracle
also checks dirty flags, draw requirements and interpolation timing, including
transitions near the maximum Java integer duration. Interpolation pixel output
and resource-loader failure recovery are outside this comparison.

```sh
python3 tools/test_sprite_animation_parity.py
```

The [animation comparison](../benchmarks/rust-sprite-animation-2026-10-06/scoped-parity.json)
records 907 fixtures and 258,996 native tick states from the actual client codec
and `SpriteContents.AnimationState.tick`, including all 63 bundled animated
texture metadata sections and synthetic boundary cases. The associated
[startup report](../benchmarks/rust-sprite-animation-2026-10-06/startup-smoke.json)
checks the renderer using this shared implementation. Images, input
fixtures and native traces stay private under `.local/`.

```sh
cargo +stable test -p horizon-client
cargo +stable clippy --workspace --all-targets -- -D warnings
python3 tools/test_native_client.py
# Headless Linux alternative:
python3 tools/test_native_client.py --vscreen
```

The mesh tests compare expanded rectangles against independently enumerated
exposed voxel faces, including solid neighbors, cavities, material boundaries,
negative chunk origins and random surroundings. The startup runner launches a
nine-chunk scene, requires 30 presented frames and a clean exit, verifies 3,169
greedy cube rectangles and 179 other model faces across 18 sprite layers, and writes GPU,
platform, binary and source information to ignored
`.local/horizon-client/startup-smoke.json`. It has an external timeout as well as
the application's frame deadline. These checks do not establish gameplay or
pixel parity with Java. Source and resource fingerprints must remain unchanged
during the run. An animation test covers duration boundaries, wrapping and
repeated physical frames independently of render frequency.

Initial local validation uses Linux x86_64, a headless Wayland compositor and
Vulkan llvmpipe software rendering. Textured geometry and resizing were visually
inspected; movement, arrow rotation and Ctrl+Q were exercised using virtual
physical key codes. Captured relative mouse movement and Escape release were
exercised through a virtual pointer. Physical-GPU runtime tests on the target
machine remain pending. Software-renderer frame rates
are not evidence of a hardware speedup.

The model gallery was also inspected at close range on Linux/Vulkan llvmpipe.
With the camera stationary, screenshot differences were confined to the lantern
as its frames changed. The [model-client startup report](../benchmarks/rust-native-model-client-2026-10-06/startup-smoke.json)
records the bounded presentation check and the scope of this visual inspection.

The [blockstate-client report](../benchmarks/rust-native-blockstate-client-2026-10-06/startup-smoke.json)
also checks all 16 gallery placements against actual Java registry binding, root
baking and model collection using `state.getSeed(position)`. A tagged empty
geometry adapter identifies the 20 selected model parts and their transforms;
the separate baking gate covers actual quad geometry. Preparation exports typed
property domains and canonical state order, which the Rust loader checks against
its imported registry before binding. Each distinct transformed model is baked
once during gallery preparation and reused across positions. The selected state
IDs, properties, positions and models are recorded in the startup report.

[Neighbor-face visibility](face-visibility.md) now has native client/server
coverage across all registered states and suppression-rule families. Integrating
that result with resident chunk model meshing is now covered by the scene check
below.

The [resident-model report](../benchmarks/rust-native-chunk-models-2026-10-06/startup-smoke.json)
checks all 10,919 occupied cells in the bounded startup scene against a separately
constructed native scene. Actual Java state-model selection, parent geometry,
rotated culling directions and `Block.shouldRenderFace` produce the same 5,991
visible and 59,636 culled model faces. The sixteen featured model selections also
retain their direct native comparison. Expanded greedy-rectangle tests cover
rotated and mirrored UV bases, texture and shade boundaries, negative origins,
neighbor seams and cavities; cube projection tests cover all XYZ quarter turns.
These checks cover CPU mesh behavior and GPU startup, not native shader pixels
or a hardware performance comparison.

The [landing interactive smoke check](../benchmarks/rust-native-landing-2026-10-07/interactive-smoke.json)
exercises F5 walking from the initial camera position. The player lands at y=4,
resets fall distance and records one damage request of 9.0. That historical
checkpoint preceded the health integration below. The Linux window exits cleanly after visual inspection.

The [health integration comparison](../benchmarks/rust-native-health-2026-10-07/scene-motion.json)
connects actual server-player hurt to the same live attributes, synchronized
health/absorption storage and hunger state used by the resident player. It checks
90,065 operations in 1,881 scenarios against both pinned JARs. Repeated hits,
absorption, armor/toughness, health limits, damage cooldown boundaries, lethal
hits and nonfinite incoming damage retain native results and event order.
Travel reads the live movement-speed attribute through the actual native player
getter, independently of the base-living stored speed field. This removes a
controlled getter from the older movement harness and corrects the runtime
query. Dead-or-dying state disables movement input and jumping. Server-player jumps now
add the native exhaustion amount and queue a jump statistic, including when
jump strength prevents upward motion. The damage-clock fields advance at 20 Hz;
clock operations in the comparison establish elapsed-time state rather than
invoking complete native tick methods. That checkpoint preceded the hunger
integration below.

The scene loads native-decoded damage definitions, damage tags and health field
IDs from the pinned private resources. Statistics, combat records, sound,
network broadcasts and advancement criteria are queued service requests. The
death consumer marks the stored actor dead and queues a request; drops, removal,
respawn and the death screen are unfinished. F5 constructs a fresh development
actor when walking is enabled. This is not vanilla respawn behavior.

The [health interactive smoke check](../benchmarks/rust-native-health-2026-10-07/interactive-smoke.json)
enables walking and F3 diagnostics, lands from the initial position, then walks
and jumps before exiting. Health falls from 20 to 11, matching the single damage
request of 9.0; the cooldown expires to zero. The final report is grounded at y=4
with 110 movement ticks and no error. The ground view was visually inspected.
The [startup check](../benchmarks/rust-native-health-2026-10-07/startup-smoke.json)
also passes with the additional health resources. These are Linux development
checks, not release acceptance or a hardware performance comparison.

The [hunger integration comparison](../benchmarks/rust-native-hunger-2026-10-07/scene-motion.json)
checks 196,489 operations in 2,837 scenarios against both pinned references.
The resident simulation now ticks hunger after movement. It binds actual food,
saturation, exhaustion and timer fields to the same live health and attribute
storage, including regeneration and starvation through registered damage sources.
The damage callback borrows the FoodData currently being ticked, so exhaustion
writes remain visible before its timer resets. Peaceful recovery also runs in
the native Player.aiStep phase before movement. Difficulty and natural
regeneration settings live with the resident world.

The reference executes actual `FoodData.tick`, `ServerPlayer.tickRegeneration`,
health setters, armor/absorption and hurt methods. Cases cover all four
difficulties, starvation health limits, absorption during starvation, changing
regeneration rules, fast/slow healing timers, timer overflow, signed zero,
nonfinite food state and combined falling/movement/food ticks. Explicit clock
setup supplies elapsed-time fields; complete enclosing `tick`/`doTick` methods
remain outside this comparison. The default walking scene uses Normal
difficulty with natural regeneration enabled. At that checkpoint food item use,
movement-distance exhaustion, effect ticks, full death/respawn services and save integration remained
unfinished.

The [interactive hunger check](../benchmarks/rust-native-hunger-2026-10-07/interactive-smoke.json)
enabled walking and F3 diagnostics on Linux. After one landing damage request
of 9, stored health recovered from 11 to 12.5 as saturation fell from 5 to 3;
food remained 20 and exhaustion ended at 1. The player remained alive and
grounded at `[25, 4, 25]`, with an expired hurt cooldown, after 42 movement ticks.
The ground view was visually inspected and the client exited without an error.
The development client pauses movement without focus, so wall time is not
simulation time. This checks the resident simulation and window, not full
gameplay, release acceptance or hardware performance.

The [movement statistics comparison](../benchmarks/rust-native-movement-statistics-2026-10-07/scene-motion.json)
checks 215,204 operations in 5,510 scenarios against both pinned references.
The resident simulation now feeds the change in player position to the movement
statistics method after movement and before hunger ticking. Sprint distance
adds exhaustion to the same stored food state; ordinary walking adds zero.
Climbing and airborne distance also produce their native statistic requests.
Statistics persistence and scoreboard delivery remain unfinished.

Vanilla invokes `ServerPlayer.checkMovementStatistics` from its packet listener
when accepting movement, separately from `Entity.move` and travel. The resident
scene directly accepts its resolved local displacement at this boundary; its
comparison invokes the actual native statistics method. It does not yet run the
packet listener's acceptance, rejection or scheduling logic. The shared kernel
also covers swimming, underwater walking, water-surface walking, crouching and
gliding. Direct method fixtures check all 512 overlapping query-flag combinations,
invulnerable abilities, float-versus-double centimeter rounding, integer overflow,
zero and nonfinite deltas. Those query overrides are restricted to statistics
fixtures; that checkpoint used dry, standing resident physics. Combined scenes exercise
repeated movement, sprint-state changes, jumps, hunger consumption and healing.
That checkpoint preceded the local sprint controls described below.

The [Linux window check](../benchmarks/rust-native-movement-statistics-2026-10-07/interactive-smoke.json)
enabled walking, showed diagnostics and held W for 650 ms. The player moved from
`[25, 4, 25]` to `[23.12939004788537, 4, 23.12939004788537]`, remained grounded
and alive, and exited without an error after 82 movement ticks. Health recovered
to 14 after the initial fall. The ground view was visually inspected; this is a
client smoke check, while statistic amounts and exhaustion are checked by the
native comparisons above.

The [sprint input comparison](../benchmarks/rust-native-sprint-2026-10-07/sprint-input.json)
checks 502,112 evolving steps in 146,648 scenarios. It executes the actual
`LocalPlayer.aiStep` prefix, including `KeyboardInput.tick` and the private
sprint eligibility and stop methods. A sentinel ends execution at the riding
boundary before parent movement. The fixtures supply environmental, effect,
vehicle and UI queries; actual food/ability storage and item `USE_EFFECTS`
components supply the remaining conditions. The comparison covers all combinations
of 17 condition flags, keyboard states and previous input, food thresholds,
timer boundaries, sprint windows from 0 to 10, and double-tap sequences. It
checks each ordered sprint setter request, including repeated requests.

The [combined sprint scene](../benchmarks/rust-native-sprint-2026-10-07/sprint-scene.json)
then checks 13,387 operations in 419 scenarios. Its Java input actor shares the
native scene actor's food and collision state, and its sprint setters update
that actor's real attributes. Actual input preparation, movement, distance
exhaustion, hunger and healing follow. Snapshots compare the sprint timer and
previous input alongside position, velocity, modifiers and health/food fields.
Cases include collisions, releasing the sprint key while moving, double taps,
jumps, and hunger crossing the sprint threshold. The
[common scene check](../benchmarks/rust-native-sprint-2026-10-07/scene-motion.json)
also passes against both pinned references.

The walking client now samples sprint input on each simulation tick and uses
the default seven-tick double-tap window. The live food and collision fields
control eligibility; sprint changes update the existing movement-speed
attribute. The F3 title shows sprint state, and exit reports include sprint
ticks and movement-distance counters. These counters are diagnostics, not saved
statistics. Fluid movement, vehicles, effects, item use, input remapping and the
complete client/server tick remain unfinished. The input comparison tests those
eligibility queries in isolation; the sprint checkpoint used dry, standing
actors with empty equipment. The crouch integration below extends that domain.

```sh
python3 tools/test_sprint_input_parity.py
python3 tools/test_native_scene_motion.py --sprint-input
```

Linux window checks exercised both [Left Ctrl](../benchmarks/rust-native-sprint-2026-10-07/sprint-key-smoke.json)
and [double-tap W](../benchmarks/rust-native-sprint-2026-10-07/double-tap-smoke.json).
The Ctrl run recorded 71 sprint ticks and 1,948 sprint centimeters. The double-tap
run visibly showed `Sprint true` during the second press and `Sprint false`
after releasing W. Both runs ended grounded and alive without an error, and
their views were visually inspected. These are development client checks;
release acceptance on the target machine and complete gameplay remain pending.

## Crouching and dry pose transitions

Left Shift now binds crouching to the resident player. Local crouch selection
uses the previous keyboard state before the new input is sampled; that local
state controls slowdown and sprint eligibility. The current Shift input controls
ledge backoff, descending collision context, bounce suppression and ladder
sliding. After movement, pose-fit checks choose standing, crouching or crawling
and refresh the cached bounds and eye height. Releasing Shift under a ceiling
does not force the actor into a standing volume that cannot fit.

The [combined input scene](../benchmarks/rust-native-crouch-2026-10-07/input-scene.json)
checks 21,241 operations in 514 scenarios against the pinned client, including
the actual `LocalPlayer.aiStep` input prefix and `modifyInput`. The
[shared scene comparison](../benchmarks/rust-native-crouch-2026-10-07/scene-motion.json)
checks 223,645 operations in 5,605 scenarios against both pinned JARs. It invokes
actual native pose selection, dimension refresh and edge backoff. The new cases
include low ceilings, blocked pose changes, scale and sneaking-speed attributes,
diagonal ledges, step-height thresholds, jumping, release transitions and landing
on different materials. Seven output lines per operation compare motion,
attributes, events, health, hunger, input history and pose/dimension bits.
The existing [isolated sprint comparison](../benchmarks/rust-native-crouch-2026-10-07/sprint-input.json)
also passes after updating its scene controller.

The native fixture controls pose storage and invokes the actual dimension-refresh
body when the pose changes. Full synchronized pose data and network notifications
remain unfinished. That checkpoint used explicit dimension refreshes; the
pending attribute integration below adds the simulation tick phase. Crawling here means dry collision
pose selection; swimming physics, sleeping, flight and death poses are unbound.

The client camera follows the cached eye height with the inspected native float
smoothing arithmetic. Full camera behavior and pixel parity are not established.
The [Linux window check](../benchmarks/rust-native-crouch-2026-10-07/interactive-smoke.json)
visibly switched from crouching to standing after releasing Shift. It recorded
100 crouching ticks and 613 crouching centimeters, ending alive and grounded
without an error. Both views were inspected. The
[renderer startup check](../benchmarks/rust-native-crouch-2026-10-07/startup-smoke.json),
seven client tests and all-target client Clippy also passed. These checks do not
establish full vanilla gameplay or final Linux release acceptance on the target machine.


## Pending attribute updates

The resident simulation now dispatches pending attribute callbacks after movement
and before pose selection, matching their position in `LivingEntity.tick` and
`Player.tick`. Reducing maximum health or absorption clamps the stored value;
changing scale rebuilds the cached player dimensions even when the pose stays
the same. The update set clears after the callbacks finish, while attribute
synchronization stays queued. Existing modifier, cache and dirty-state behavior
is preserved.

The scene oracle invokes the actual private native refresh method and the
`ServerPlayer.onAttributeUpdated` override. Waypoint transmit/receive changes
produce ordered manager requests; waypoint management and network delivery remain
unbound. Health and absorption use the same synchronized float storage as damage
and hunger. The full enclosing player/entity tick is still incomplete.

Callback order depends on native attribute-instance identity hashes. The oracle
now exports each reference process's holder and instance identities, and the Rust
probe replays them. The check compares ordered health/waypoint events and validates
that the same number of attribute instances is allocated. It does not sort those
events to conceal ordering differences. Registry snapshots still compare map and
modifier contents independently of iteration order.

The [shared scene check](../benchmarks/rust-native-attribute-updates-2026-10-07/scene-motion.json)
passes 234,949 operations in 5,801 scenarios against both pinned JARs. The
[combined input scene](../benchmarks/rust-native-attribute-updates-2026-10-07/input-scene.json)
passes 30,025 operations in 710 scenarios against the client. New cases combine
health and absorption limits, scale, both waypoint ranges, permanent/transient
modifiers, repeated refreshes and nonfinite values. Low-tunnel histories exercise
movement, releasing Shift under a ceiling and changing scale between ticks.

The [Linux walking check](../benchmarks/rust-native-attribute-updates-2026-10-07/interactive-smoke.json)
recorded 36 sprinting ticks and 36 crouching ticks, ending grounded and alive
without an error. Its view was inspected. The
[startup check](../benchmarks/rust-native-attribute-updates-2026-10-07/startup-smoke.json),
seven client tests and all-target client Clippy passed. Full gameplay parity and
playable Linux release acceptance on the target machine remain pending.
