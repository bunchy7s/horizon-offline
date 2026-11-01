# The next server: design

This page is the design of Horizon's next server: the fastest server we can build for Horizon players, with
vanilla Java clients welcome and vanilla's rules kept exactly. Two bars:

1. **Better than Pumpkin on every number we can measure**: players at 20 TPS, MSPT p99, memory, bandwidth and
   join time, with Pumpkin's released binary run as a black box against the same bots.
2. **Vanilla-exact where Pumpkin isn't.** Gameplay is Horizon's engine, checked by its 324 vanilla parity
   harnesses (each one runs the real 26.3 jars and Horizon on the same inputs and compares the results).

All of it lives in this repo. Single player's integrated server, LAN, `--dedicated` and the
`horizon-server` supervisor run the same server code (section 3). horizon-rust/horizon-server is only a
download home, for people who want a standalone server.

Contents: the measured baseline (section 1), the targets (2), where the work goes (3), the design of each part
with the measured cost it removes (4 to 8), the plan (9), and what each open PR on the theme brings
(appendix A).

What [server.md](server.md) and [hosting.md](hosting.md) already decide and mostly build is not repeated here:
the supervisor, `server.toml`, the panel, backups and the control API.

**Originality.** Nothing here comes from Pumpkin's, Paper's, Folia's or any other server's code; none of it
was read. About other servers we use only what a user can see: their documentation, their generated config
file, and black-box runs of their released binaries. Vanilla's behaviour comes from the 26.3 jars, as for the
rest of Horizon.

## 1. Baseline (measured 2026-10-29, Mac mini M4 Pro, 14 cores, 64 GB)

**Servers.**
- Horizon: `horizon-client --dedicated` at horizon `668419b92`, `fast` profile. This is main *before* the three
  tick fixes still in review (#3815, #3834, #3847).
- Vanilla: 26.3 `server.jar` on Temurin 25, `-Xms4G -Xmx4G`.
- Pumpkin: release `0.2.1+26.3-26.51`, `pumpkin-ARM64-macOS`, SHA-256 checked against the release's
  `checksums.sha256`.

**Same settings for all three.** Seed 12345, a fresh world, view and simulation distance 10, offline mode, creative
with forced game mode, max players 200, difficulty easy, bound to 127.0.0.1. Pumpkin also ran with its
Bedrock listeners, LAN broadcast, plugins and telemetry turned off, through its own `pumpkin.toml`.

**Load.** `horizon-bots --behaviour mixed` (60 % walk within 48 blocks of spawn, 20 % explore outwards at
22 blocks/s, 20 % idle). One bot joins every 0.5 s, then a 30 s warm-up, then a 60 s window. The bots don't fight:
`horizon-bots` has no combat yet (issue S-1, section 9.3).

**How each number is taken.** The server measures itself, the same way on all three:
- MSPT: the console's `tick query` every 5 s, which reports the average, P50, P95 and P99 over the last
  100 ticks.
- TPS: `time query gametime` at the start and end of the window, as game ticks per wall-clock second.
- Memory: RSS, sampled every second.
- CPU: the server process's CPU seconds divided by the window.
- Bandwidth: the bots' loopback bytes, from macOS `nettop`, per player.
- Join time: from the bot's connect to its spawn position.

The driver, `sn_run.py`, comes into `tools/server_bench.py` with issue S-2 (#3972).

**Caveat.** The Mac mini is shared. Its load average was 57 to 132 on 14 cores during the runs (other agents' builds), and
the bots ran on the same machine. So a tick can include waiting for a core, and the numbers carry more noise
than a dedicated machine's would. All three servers ran interleaved under the same load (10, then 50, then
100 bots, each in the order Horizon, vanilla, Pumpkin). Treat differences under about 20 % as noise.

| Bots | Server | TPS | MSPT avg | p50 | p99 (median / worst 5 s) | CPU cores | RSS MB | KB/s per player | KB per chunk | Chunks sent | Join ms p50 / max | Pulled back |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 10 | Horizon | **20.0** | **11.3** | 10.9 | **17.7 / 23.6** | 1.01 | 1165 | 87.9 | **3.55** | **15 272** | 74 / **103** | 1 469 |
| 10 | vanilla | 16.7 | 41.1 | 7.8 | 243 / 352 | 0.79 | 3334 | 61.4 | 6.75 | 5 646 | 3 730 / 5 550 | 1 366 |
| 10 | Pumpkin | 20.0 | 23.0 | 16.6 | 87 / 143 | 1.20 | **538** | 46.3 | 4.50 | 6 397 | **55** / 1 980 | 0 |
| 50 | Horizon | **20.0** | **28.7** | 27.4 | **38.5 / 51.7** | 2.11 | 1535 | 76.5 | **3.64** | **65 431** | 93 / **249** | 12 141 |
| 50 | vanilla | 5.3 | 58.8 | 7.8 | 416 / 537 | 0.70 | 3466 | 38.4 | 7.43 | 16 115 | 4 791 / 15 082 | 2 577 |
| 50 | Pumpkin | 20.0 | 45.8 | 40.2 | 151 / 283 | 1.08 | **869** | 34.5 | 3.77 | 28 427 | **54** / 4 075 | 0 |
| 100 | Horizon | **14.3** | 79.2 | 83.8 | **108 / 138** | 4.42 | 2054 | 95.4 | 3.70 | **160 042** | 83 / **244** | 14 638 |
| 100 | vanilla | 2.2 | 108.3 | 63.5 | 539 / 556 | 1.37 | 3699 | 45.0 | 5.18 | 53 786 | 933 / 10 524 | 14 790 |
| 100 | Pumpkin | 13.6 | **78.6** | 73.3 | 231 / 348 | 2.54 | **1421** | 51.6 | **3.61** | 88 625 | **43** / 494 | 0 |

Idle RSS after start (fresh world, nobody on): Horizon 760–780 MB, vanilla 1 550–1 630 MB, **Pumpkin 32–35 MB**.
Start to ready, fresh world: Horizon 9.2–14.1 s, vanilla 25–34 s, **Pumpkin 0.1–0.2 s** (it generates no spawn
area before it accepts players). The raw JSON comes with S-2.

**What it says.**
- **Up to 50 players**, Horizon and Pumpkin both hold 20 TPS. Horizon's tick is about 40 % shorter (28.7 against
  45.8 ms at 50) and its p99 is 4 to 5 times lower (51.7 against 283 ms at worst). Over the same window it also
  sent 2.3 times the chunks. Vanilla falls to 5.3 TPS at 50.
- **At 100 players nobody holds 20 TPS.** Horizon runs at 14.3, Pumpkin at 13.6, vanilla at 2.2. Horizon still has the
  lowest p99 (138 against 348 and 556 ms) and sends 1.8 times Pumpkin's chunks. Two of the Horizon fixes in review
  (#3815, #3834) took a 100-bot run from 14.5 to 17.8 TPS (different mob counts, so read it as a direction).
- **Memory is Pumpkin's clear lead.** It idles at 32 MB against Horizon's 760 MB, and uses 1.4 GB against 2.05 GB
  at 100. Vanilla's heap alone is 4 GB.
- **Bandwidth per chunk is the same** for Horizon and Pumpkin (3.6–3.7 KB), and 5–7 KB for vanilla. Horizon's
  higher KB/s per player is the extra chunks. So bandwidth can't drop much further on vanilla's format. The Horizon
  fast path (section 8) is where it can.
- **Join.** Pumpkin's median is lowest (43–55 ms), but its worst reached 4.1 s at 50 bots. Horizon's stays at
  74–93 ms with a worst of 249 ms. Vanilla takes 0.9 to 4.8 s.
- **The work isn't fully equal.** Pumpkin never pulled a bot back (0 against ~14 600 for Horizon and vanilla at
  100): it doesn't run vanilla's "moved too quickly" check, so its explore bots never wait for it. The mob counts
  of vanilla and Pumpkin can't be seen from outside; Horizon had 1 834 mobs at the end of the 100-bot window.
  Vanilla's self-reported MSPT leaves out the chunk work it runs between ticks, which is why its TPS (from game
  time) is the number to read, not its MSPT.

**Where Horizon's tick goes at 100 bots** (macOS `sample` of the tick thread, from the profiling behind #3815, #3834 and #3847, same machine):

| Cost | Share of the tick thread | Status |
|---|---|---|
| Mobs: AI and movement | 12–19 % | flat profile (collision scan, AABBs, chunk reads) |
| Natural spawning (`spawn_for_chunk`) | 10–12 % | already tight per position |
| Hub (`tick_lan`): tracking, chunk send, play state | 15–22 % | section encoding is already on helper threads |
| Light views of proto chunks on the tick thread | 18.7 % → 0.3 % | **fixed** in #3815 |
| Scheduled ticks: one `GameLevel` per due tick | tick_blocks 15 → 10 ms at 50 bots | **fixed** in #3834 |
| Spawners: a level built with nobody in range | 1.6 % → 0.5 % | **fixed** in #3847 |
| Random ticks | 7–10 % | vanilla's own block reads |
| Fluids (`slope_distance`, `Vec` allocations) | ~4.5 % | open |

Everything above runs on **one thread**. The worker pool (generation, light, section encoding) has cores to
spare: the whole server used 1.0 to 4.4 of the 14 cores. Pumpkin leads on memory and start
time today; section 7 is the plan to pass it there too. On scale the lesson is simple: the next 10× can't come from shaving one
thread. It comes from running the tick on more cores without changing what vanilla would do.

## 2. Targets

Each one is measured with the bench from section 1, on a dedicated 8-core machine (the bots on another
machine), with `mixed` bots, view distance 10, and a pre-generated world plus a fresh-world pass.

| Number | Pumpkin today | Target | Why it is reachable |
|---|---|---|---|
| Players at 20 TPS (walking, one level) | 50 (13.6 TPS at 100) | **200** on 8 cores | parallel phases (4.2) + the hub off the tick thread (5) |
| Players at 20 TPS spread over 3 dimensions | — | **300** | levels in parallel (4.1) |
| MSPT p99 at 100 players | 231 ms (worst 5 s: 348) | **< 35 ms** (p50 < 15) | the same, plus saving off the tick thread (5.5) |
| RSS idle, fresh world | 32 MB | **< 32 MB** | nothing loaded while empty, lazy data (7) |
| RSS at 100 players | 1.42 GB | **< 1.0 GB** | per-player state measured and capped (7) |
| Bandwidth, walking vanilla client | 3.6 KB per chunk | **≤ vanilla's bytes per chunk**, compression level picked by measurement | 5.3 |
| Bandwidth, walking Horizon client | — | **−50 %** against vanilla clients on the same server | the fast path (8) |
| Join to spawn, p99 | 43 ms p50, 494 ms max (4.1 s at 50) | **< 300 ms** on loopback, **< 2 s** for a full view | 5.2 |
| Start to ready (status ping answered) | 0.1–0.2 s | **< 0.1 s**, fresh or existing world | nothing heavy before the listener opens (7) |
| Vanilla parity | not checked | **every harness that covers server code passes** on every server PR | 4.3 |

The 200-player target fits within one tick: at 100 bots, the tick thread's CPU time per tick was 31 ms
(with #3815 and #3834). That leaves 8 cores × 50 ms = 400 core-ms per tick for 200 players,
if most of the work moves to parallel phases.

## 3. Where the work goes

### 3.1 One server, in this repo

Single player runs on the server thread (`game/integrated.rs`, since #1514), so the integrated server, LAN
hosts and `--dedicated` share every line of server code: the tick, the levels, the hub, the entity tracker,
saving. A separate server repo would fork that code or pin it across repos, and either way two copies would
drift. Gameplay is about 420 000 lines (`game/` 133 k, `mobs/` 62 k, `lan/` 15 k, the flat lib 90 k, and
`horizon-world` 124 k), guarded by the 324 harnesses. So everything here lands in horizon:

| Part | Where | Shared by |
|---|---|---|
| Parallel tick (section 4) | `game/levels.rs`, `game/integrated.rs`, `game/dedicated.rs`, and the scheduler as a new module beside them | integrated server, LAN, dedicated |
| Network front (5) | the hub, `lan/` (`conn.rs`, `hub.rs`, `chunks.rs`, `tracking.rs`); framing in `horizon-protocol` | LAN, Play with Friends, dedicated, the integrated server's memory connection where it applies |
| Saving and light off the tick thread (5.5) | `game/saving.rs`, `save.rs`, `horizon-world` | everyone |
| Fast path (8) | `horizon-protocol` (the `horizon:hello` payloads), `lan/conn.rs` (server), `net.rs` and `game/remote*.rs` (client) | every Horizon client against every Horizon server |
| Operations (9.2) | `crates/horizon-server` (supervisor, panel, control API, MCP), `crates/horizon-server-files` | dedicated |
| Plugin API (W9) | `crates/horizon-mods` and the server hooks from #3706 and #3878 | integrated server, LAN, dedicated |
| Bench (W0) | `tools/server_bench.py`, `crates/horizon-bots`, `benchmarks/` | — |

What changes for single player: it gets the exact parallel phases (4.1, 4.2 `tick = "vanilla"`), the cheaper
hub and off-thread saving, at no cost in behaviour. `tick = "regions"` is a dedicated-server option and
single player never uses it.

The crate split carries on as planned. When `Game`'s server half moves from the binary into a library,
that move serves the game's build times and architecture. The server doesn't need it, because it is
already the same binary.

### 3.2 `--dedicated`, the supervisor and the download repo

- **`horizon-client --dedicated`** (the `horizon-engine` build without audio and controllers,
  [server.md](server.md) §3) stays the server engine and the harnesses' server. With `HORIZON_SERIAL_TICK` it is also the serial
  reference every parallel phase is compared against (4.3).
- **`horizon-server`** (the supervisor) stays the program owners run.
- **horizon-rust/horizon-server** holds no code. Its releases repack the engine and the supervisor, built from
  this repo's tags, for people who only want a standalone server (W8).

## 4. Ticking on more cores

Vanilla ticks one level at a time (`MinecraftServer.tickChildren`: each `ServerLevel.tick` in order, then the
connections). Inside a level it ticks in a fixed order:
1. time and weather;
2. scheduled block and fluid ticks, in their queue's order;
3. raids;
4. chunks: random ticks and natural spawning, in a shuffled chunk order that draws on the level's random;
5. block events;
6. entities, in list order;
7. block entities, in list order.

Horizon follows the same order (`game/levels.rs` `tick_server`, which swaps each level into the one `Game`).

### 4.1 Levels in parallel (exact)

Each level ticks on its own thread. Three things stay serial, in vanilla's level order, at a barrier after the
parallel phase:
- anything that crosses levels: entities and players changing dimension, `execute in`, ender pearls, portal
  search into the other level's chunks;
- shared server state: scoreboard, advancements and statistics, game rules, boss bars, the player list,
  command functions and `/schedule`, the world border's sync across levels;
- the connections.

A level's tick writes those effects to an ordered log, which the barrier applies in vanilla's order.

The one place where vanilla's order is visible across levels is an entity that leaves a level that ticks
earlier for one that ticks later: in vanilla it can tick in both levels during one server tick. The barrier
replays that case serially (the arriving entity's tick runs in the destination's serial slot). A harness that
throws entities through portals every tick checks it against the serial path, tick by tick.
- **Removes:** the levels' sum becomes their maximum. It helps only when players are spread over dimensions;
  with everyone in the overworld it gains nothing. That's why 4.2 exists.
- **Engine work (horizon):** levels already have separate state (`levels.rs` parks the inactive ones). The
  work is making `Game`'s per-level part a value a thread can own, with the shared part behind the barrier log.

### 4.2 One level on more cores (exact by default)

Vanilla's order inside a level can't be split into regions freely. Random ticks and natural spawning draw from
one level random in chunk order. Scheduled ticks run in one global order. Entity IDs come from one counter.
Mobs see the moves of the mobs that ticked before them. Running regions in parallel changes these draw
sequences. No player can see the difference, but the parity harnesses do, and so would anyone comparing seeds
with vanilla. Two modes:

- **`tick = "vanilla"` (default): exact.** Vanilla's order of effects is kept. Only work that doesn't depend
  on that order runs in parallel:
  1. *Per-entity pure work, run ahead in parallel, committed in order.* Pathfinding (`PathFinder` over a
     read-only snapshot of the level), sensor scans (nearest player, nearest item, line of sight), collision
     shape gathering for `Entity.move`, and brain sensors run on all cores at the start of the entity phase.
     None of this draws a random number, and an entity's own draws use its own random (`Entity.random`),
     so no level draw moves. The serial
     entity loop then uses each result if nothing it read changed since. It checks the blocks and entities in
     the result's read set against a per-section change counter, and recomputes in place when they did. Mobs
     that don't move or path (most of them: 1 400+ mobs against 100 players in section 1) commit
     without any recompute. **Removes:** the AI and movement share (12–19 %), minus the serial commit.
  2. *Hub work after the tick.* Entity tracking, chunk sending and packet encoding read the level once it has
     ticked, so they run in parallel per player while the next tick's serial phases start (section 5).
     **Removes:** the hub's 15–22 %.
  3. *Random ticks: block reads batched per chunk.* The draws stay serial and in order, but each chunk's
     sections are pinned once instead of a map lookup per read. **Removes:** part of the 7–10 %.
  4. *Natural spawning: a per-tick grid of "players near this chunk" answers*, built in parallel. The draws
     stay serial. **Removes:** ~2 % (the per-chunk map entry and `Vec` measured while profiling #3847).
- **`tick = "regions"` (opt-in, off by default): parallel regions, vanilla rules, not vanilla's random
  sequences.**
  - Groups of loaded chunks more than 2 chunks apart, with no entity, ticket or redstone link between them,
    tick on separate threads. Each group gets its own random, split from the level's, plus its own entity ID
    range and scheduled-tick queue. Groups merge and split at the barrier as players move.
  - Every rule stays vanilla's (the harnesses that compare outcomes still pass). What changes is only which
    random number a given chunk draws. The config says it in those words.
  - Off by default, like every Horizon change to vanilla behaviour ([design-system.md](design-system.md)).
- **What stays serial in both modes:** cross-level effects (4.1), commands and functions, the player list, chat
  order, advancements, the scoreboard, raids (one per village, but their spawning draws the level random),
  block events (vanilla runs them to a fixed point in one queue), and the order of packets to one player.

### 4.3 How exact is checked

Every parallel phase has a switch that runs it serially (`HORIZON_SERIAL_TICK=entities,hub,...`). A harness runs
the same fixture both ways and compares a digest of the level every tick: block states, entity positions and
NBT, the random's state. On top of that, the harnesses that cover server code run with the default mode. A phase that can't pass that comparison doesn't ship in `tick = "vanilla"`.

## 5. Network and I/O off the tick thread

**What already exists in the hub:**
- section encoding once per chunk, on helper threads (#623, `lan/chunks.rs`);
- chunk frames compressed once for every player (`SharedFrames`, `lan/conn.rs`);
- vanilla's chunk batch pacing (`ChunkBatchReceived`) and its move checks (#586);
- a reader and a writer thread per connection (#649 made the writer wake once per tick).

#3036 (hectoreccles) moves batch bodies to helper threads. W1 builds on it rather than redoing it.

### 5.1 I/O threads instead of two threads per player

200 players means 400 threads today. The hub moves to a fixed pool of I/O threads (one per two cores) on
`kqueue`/`epoll` through `mio`, with one bounded queue per player. The tick thread only pushes packet bodies.
Framing, compression, encryption and the socket writes happen on the pool. A player whose queue passes its
budget gets its chunk sending throttled first (as vanilla's batch pacing does). Vanilla's timeouts and
disconnects stay unchanged. **Removes:** thread switches and lock traffic per packet. Bench: CPU per player at
200 idle bots.

### 5.2 Join

Login and configuration (registries, tags, known packs) are the same for every player. They are encoded once
per server start and sent as prepared frames. The first chunks around the spawn come from the chunk cache
(5.3), not the generator. **Removes:** Horizon already joins in 74–93 ms p50 on loopback, worst 249 ms. Pumpkin's median is lower (43–55 ms) but its worst reached 4.1 s at 50 bots. Prepared frames bring the median under Pumpkin's and keep the worst flat.

### 5.3 Chunk cache and compression level

- A chunk's encoded and compressed packet is kept until a block in it changes. Block changes go to the players
  holding it as vanilla's section updates, and drop the cached frame. Walking players who re-enter chunks then
  cost a memcpy, not an encode.
- Compression uses vanilla's threshold (256) and the level that gives the best bytes per core-ms on the bench.
  zlib 6 is what Horizon uses today; Pumpkin's config defaults to 4. Both are vanilla's wire format.
- **Removes:** the hub's encode cost for re-sent chunks. In section 1, Horizon's walking bots received
  each chunk several times, and those re-sends are where most of the bandwidth went.

### 5.4 Entity tracking as deltas

- Today the tracker rebuilds each tracked entity's state per tick (`lan/entity.rs` `Entities::tick`,
  `game/lan.rs` `mob_meta` ~3 % of the tick thread).
- Instead, the engine writes a change log as it changes an entity (position, rotation, metadata index, equipment,
  passengers). The tracker turns each changed entity into packets once, and sends them to every player who
  sees it.
- A grid of players per chunk section finds who sees it.
- Vanilla's update cadence is kept exactly, because clients interpolate on it: `ServerEntity.sendChanges`,
  with its update interval, its relative moves, its 60-tick position check, and its rotation and head
  thresholds.
- **Removes:** the per-viewer encode (×players) and the full state rebuild for entities that didn't change.

### 5.5 Saving and light

- Light already runs on the worker pool. Since #3815 the tick thread no longer builds proto-chunk views
  (18.7 % → 0.3 %).
- Saving: at autosave (every 6 000 ticks) and on unload, a chunk is snapshotted on the tick thread (a copy of
  its sections and block entities' NBT, which is cheap). The workers serialize, compress and write the region
  files, through vanilla's `RegionFile` format (`horizon-world`).
- The bench gets an autosave window so that the spike is measured, not guessed.

## 6. Who can join

`server.toml`:

```toml
online_mode = true   # the default: every player's account is checked
[clients]
allow = "any"        # "any" (vanilla Java and Horizon) or "horizon" (Horizon clients only)
```

- **Online mode is on by default**, as on every server, vanilla's included. With it on, each player's
  account is checked by the session service, and the whitelist and bans apply on top.
  - `online_mode` stays the owner's setting. hosting.md's warnings about offline mode stay as they are: an
    offline server checks no accounts, so keep it on a LAN or a tailnet.
  - No other auth providers. Horizon's own sign-in: [microsoft-account.md](microsoft-account.md).
- With `allow = "horizon"`, the server refuses a client that didn't send brand `horizon` and the fast-path
  hello (section 8) in configuration. The refusal message names the Horizon launcher.
  - **Be honest about what this is:** a convenience filter, not security. Horizon's client is open source, so
    a modified client can claim to be Horizon. It keeps vanilla players off a server they can't play fully,
    nothing more. Accounts (online mode), the whitelist and bans are the real gate.

## 7. Memory and start: below Pumpkin's 32 MB and 0.1 s

Today Horizon idles at 760 MB, and a fresh world is ready in 9–14 s. Pumpkin idles at 32 MB and is ready in
0.1–0.2 s. The targets are **idle RSS < 32 MB** and **ready < 0.1 s**, on a fresh world and on an existing one.

Each step below is one or more PRs. Each one is measured with the bench on three numbers:
- RSS 10 s after ready, with nobody on;
- the time from exec to the first answered status ping, timed the same way for Pumpkin;
- a per-component memory table: `vmmap` summary, plus the engine's own counters for chunks, registries and
  pools.

1. **Nothing loaded while nobody is on** (#2608, FusselTV): no chunks, light, mobs or tickets around the spawn
   on an empty server.
   - On a fresh world, the initial spawn search (`MinecraftServer.setInitialSpawn`) runs on the workers after
     the listener opens. Status pings are answered at once. A login waits until the spawn is found.
   - The chosen spawn and every rule stay vanilla's. Only the moment changes: the first player's join pays for
     it, not the start.
2. **Headless engine** ([server.md](server.md) §2): the dedicated path skips everything only a window uses: the
   HUD and block atlases, item and block-entity models, entity models, the panorama, fog and music tables, the
   meshing tables, and client language files beyond the server's own messages.
3. **One server data file, mapped, parsed on first use.** `horizon-prepare` writes a compact, versioned binary
   `server.pack` from the owner's own files: block states, registries, tags, worldgen settings, loot tables,
   recipes, advancements, structure templates.
   - The engine maps it (`mmap`) and decodes an entry the first time it's needed: a loot table at the first
     drop, a structure template when it's first placed, a recipe at the first craft.
   - Pages nobody reads never count in RSS. Decoding goes through the same types as today, so parity doesn't
     move.
4. **World generation built per dimension, on first use.** The noise router, the density-function graph, the
   biome source and the structure sets are built when a dimension's first chunk is generated. The Nether and the
   End, already parked when empty (`levels.rs`), build nothing until a player goes there.
5. **No idle threads or pools.** The worker pool and the hub's I/O threads start with the first player and
   shrink back after the last one leaves. Their stacks are sized to what they use.
6. **Give memory back.** After the last player leaves, unloaded chunks and caches are freed, and the allocator
   returns pages to the OS. The bench checks that RSS drops back under the target 60 s after the last leave.
7. **Start < 0.1 s.** With steps 1 to 4, nothing heavy runs before the listener opens: read `server.properties`
   and the lists, map `server.pack`, read `level.dat`, bind. A startup span log times each step, and the bench
   fails a PR that makes any of them slower.

With players on, the target is **< 1 GB at 100 players** (Pumpkin: 1.42 GB). Every per-player and per-chunk
structure gets a measured size in the bench report: bytes per loaded chunk, per tracked entity, per player.
Chunk lighting and heightmaps are kept packed, on #3253's storage primitives (hectoreccles), and #3489 shares
uniform sections.

## 8. The Horizon fast path

Vanilla's protocol stays the default and is never changed for a vanilla client. Horizon clients ask for more,
in configuration:

1. The client sends brand `horizon` (it does today, `net.rs`), then a `horizon:hello` custom payload:
   protocol revision plus a capability bit set.
2. The server answers `horizon:hello` with the subset it accepts. Horizon's server already answers a Horizon
   brand with `horizon:quiet_commands` (`lan/conn.rs`), and this is the same mechanism.
3. A client that sends no hello, or a capability the server doesn't know, gets vanilla. The fast path is
   always negotiated, never assumed.

| Capability | What it changes | Measured cost it removes |
|---|---|---|
| `chunk-zstd` | chunk packets in Horizon's own section encoding with a shared zstd dictionary, as one `horizon:chunk` payload | bytes per chunk (section 1: 3.6–3.7 KB for Horizon and Pumpkin alike with vanilla's format) |
| `far-terrain` | the server sends LOD tiles beyond view distance from the saved world, at low priority under the send budget. The client's `lod.rs` draws them today from its own generator | the client generating far terrain itself. Players see the server's real world, builds included |
| `entity-batch` | one packet per tick with every tracked entity's change, quantized as vanilla's deltas are | per-packet overhead: dozens of small moves per tick per player |
| `server-stats` | TPS, MSPT and the player's own ping, for the client's debug overlay | none (a feature) |

Each capability has its own harness. A Horizon client with the capability, and one without, must end up with
identical client state (`horizon-ctl state` digests). The client half lands in horizon behind the same
negotiation, so a Horizon client still plays on every vanilla server.

## 9. Plan

### 9.1 Workstreams (in order; each is a few PRs under ~500 lines, with its target and bench)

| # | Workstream | Target | Proof |
|---|---|---|---|
| W0 | **Bench**: Pumpkin and vanilla runners in `tools/server_bench.py`, a pre-generated world, bots on another machine, the report | ±5 % between two runs of the same binary | the report, published in `benchmarks/` |
| W1 | **Network front**: I/O pool, prepared login frames, chunk cache, compression level by measurement (with #3036) | join p99 < 300 ms, CPU per idle player halved | bench, `lan_e2e`, `dedicated_server` |
| W2 | **Tracker deltas**: the engine's change log, encode once per entity | `tick_lan` share ÷ 3 at 100 bots | bench, `server_entities` |
| W3 | **Parallel levels**, then the exact entity phase (4.2.1), then `tick = "regions"` | 200 players at 20 TPS on 8 cores | bench, the serial-against-parallel digest harness (4.3) |
| W4 | **Memory and start** (section 7, steps 1–7, with #2608) | idle RSS < 32 MB, ready < 0.1 s, RSS back under 32 MB 60 s after the last player leaves | bench: idle RSS, time to first status ping, per-component table |
| W5 | **Saving off the tick thread** | no autosave spike in p99 | bench with an autosave window |
| W6 | **Fast path**: hello, `chunk-zstd`, `entity-batch`, `far-terrain` | −50 % bytes for Horizon clients | per-capability harness |
| W7 | **Operations**: metrics endpoint (Prometheus text, loopback/tailnet only), scheduled restarts | the official server's checklist (9.2) | an hour-long soak with 100 bots |
| W8 | **Downloads**: a workflow in horizon-rust/horizon-server repacks the engine and supervisor from this repo's release tags | one archive per OS, the same checksums as horizon's release | the release |
| W9 | **Plugin API**: server-side mods on `horizon-mods`, with events, commands, permissions and a scheduler, on the server hooks (#3706, #3878). Led by levzzz5154, who leads modding; roadmap item `server-mods` | a plugin can do what bunchy7s asks for, measured per-tick cost under 0.1 ms with no plugin loaded | mod harness on the integrated server and `--dedicated` |

W0 comes first: without a bench that repeats within 5 %, no tick PR can show its gain. W1, W2 and W4 can run
side by side. W3 waits for W0. W9 follows levzzz5154's plan.

### 9.2 The official server

horizon.bunchy7s.dev already runs Horizon's own server, hosted by bunchy7s: `horizon-client --dedicated` from the
current release, in online mode, without the `horizon-server` supervisor. It runs no plugins, and
kira-pumpkin was never used on it (his answers, 2026-10-29). So each improvement above reaches it with a
release, and its baseline is the Horizon row of section 1. What he asked for is a plugin API, which is W9.

Still to settle with him: his backups, his restarts, how he deploys (systemd, Docker), and why he runs without
the supervisor (a missing feature, or no need), so that the supervisor covers what he does by hand.

### 9.3 Issues anyone can take

- **S-1 (#3971) bots: a `fight` behaviour.** Bots in survival that attack the nearest mob and each other.
  Today's bench has no combat.
- **S-2 (#3972) bench: Pumpkin and vanilla runs in `tools/server_bench.py`**, with `time query gametime` for
  TPS, a `--pre-generated` world copy, and a Markdown table from the JSON.
- **S-3 (#3973) bench: bots on another machine**, with the report recording where they ran.
- **S-4 (#3974) the download repo's release workflow** (W8), now that no code lives there.
- **S-5 (#3975) docs: an owner's page** comparing `server.properties` to `server.toml`: done, [owners.md](owners.md).
- **S-6 (#3976) bots: report bytes and join time**, so the bench doesn't need `nettop`.

## Appendix A. Open horizon PRs on the server's theme (2026-10-29)

A snapshot of that day, not kept up to date: a PR's state now is on GitHub.

Each one is **use** (what the server takes from it), **covered** (by what), or **not for the server**. ★ marks
the ones that should land in horizon first: the server builds on them, so triage should prove them first.

| PR | Author | Verdict |
|---|---|---|
| ★ #3815, ★ #3847 | ludovic111 | **use**: tick-thread costs (section 1). In the merger; #3834 merged |
| ★ #3036 | hectoreccles | **use**: chunk batch bodies on helper threads, the base of W1 (5.3) |
| ★ #2608 | FusselTV | **use**: nothing kept around spawn while nobody is on. It cuts idle CPU, idle RAM and start time, two of the three numbers where Pumpkin leads. It conflicts since `streaming.rs` moved to `horizon-client-render`; triage carries it if needed, as #2698 did |
| ★ #3756 | ludovic111 | **use**: the server names itself `horizon`, which the fast path (8) builds on |
| ★ #3441 | xKumorio | **use**: guests' known movement as vanilla's; server players' gameplay |
| ★ #3873 | DUzzL | **use**: breeding statistics and advancements credited on the server thread |
| ★ #3300, ★ #3802 | masa-masa-0314, ThatCakeIsAPie | **use**: a world's data packs reach recipes, loot, advancements, tags and block state providers. Servers run packs |
| ★ #2698 (carries #2544, #2586) | FusselTV | **use**: structure mobs and generation animals as vanilla's |
| ★ #2872 | levzzz5154 | **use**: forced chunks get vanilla's rings (`/forceload` farms) |
| #3480–#3490 | ThatCakeIsAPie | **use**: each level's height from its dimension type (tall-world packs), local and over the network. #3489 (uniform sections shared) also feeds the memory target (7). It's a long stack, so it lands at its own pace |
| #3491–#3494 | ThatCakeIsAPie | **use**: data-pack biomes, including the biome registry a server sends (#3493) |
| #3366 | ThatCakeIsAPie | **use**: faster surface rules, i.e. more chunks generated a second for exploring players |
| #3706, #3878 | arismoko | **use** (W9): read-only native world and server-tick hooks, the base of the plugin API |
| #3827 | abit-foggy | **use if proven on the server bench**: incremental tick-chunk membership and random-tick bounds touch `tick_blocks` on the server too. It needs the bench and parity, not only the client's frame times |
| #3868 | spidercraft | **covered** for the server: the dedicated server already runs online mode with the session check. The PR is for LAN and Play with Friends |
| #3750, #3851 | leifiyoo, DUzzL | **use**: block entities replaced as vanilla does; the axolotl test fix (main's failing `pr_ready` test) |
| #3369–#3374 | ThatCakeIsAPie | not for the server core: upgrading 26.2 data packs is an owner's tool, which the supervisor can call later |
| #3201 | damnurmum | not for the server: packet translation for client mods |
| #3276 | dan4ikx | not for the server now: client audio, UDP and mod payloads. Its client-to-server mod channels are a starting point for W9's plugin channels, and must not collide with `horizon:hello` (8) |
| #3869, #3724, #3023 | spidercraft, aangeraakt, Web-mc1 | not for the server: Play with Friends codes, client skins, the launcher's ViaProxy |
| #3669, #3841, #3865, #3881 | various | not for the server: client-side fixes |

**Server parity fixes from contributors** land on their own merits: they are already the server's code: #3716, #3645, #3686, #2990, #2987, #2577, #3401, #3621, #3675, #3748, #3875, #3908.

**Closed without merging in the last two weeks**, with a server idea: #628 (packets a second and the server
line on F3) is the client half of the fast path's `server-stats` capability. The other 16 have no server idea, or are against
Mojang's rules (#3537, #3681: offline play, other account providers).
