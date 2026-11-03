# Nether-portal section-scan benchmark

Run with the pinned prepared client resources:

```sh
CARGO_BUILD_JOBS=6 cargo run --profile fast -p horizon-client-lib --example portal_scan_bench -- \
  .local/horizon-client/26.3
```

The example keeps the original `Game::may_hold_portal` alongside the resident
cache (`portal_sections`). Both run in the same optimized binary, alternating
measurement order. Each workload has one discarded warmup pair and seven pairs
of 300 ticks. It reports resource loading and fixture construction separately.
This measures the section scan only, not the whole `entity_portals` span, the
sweep that follows a positive answer, or the rest of the tick.

The fixture is a 17x17 chunk world (289 chunks), each with 24 sections, and 500
entity reaches spread over it (an item or mob box 0.25 wide and the small move
that produces the swept box). A reach touches 1-4 sections. Four palettes:
sparse (one non-portal state), dense (128 distinct), global (300 distinct, a
global palette whose `maybe_has` walks the whole state range) and dense_portal
(the dense palette with a portal in every fifth section). These are controlled
fixtures, not a claim about typical generated terrain.

Steady includes the initial cache fill amortized over 300 ticks. Cold recreates
the cache each tick, including allocation and destruction; reload clears it each
tick while retaining capacity. Edit workloads alternate a block value in three
or all 289 chunks before every scan. Timing includes the `Arc::make_mut` move of
the 176-byte chunk wrapper caused by weak cache handles; the baseline holds no
such handles. Both paths keep the original outer reach list.

The scan is parity-checked in the same run: the cached and per-tick answers must
agree for every reach over every sample, or the example panics.

Measured on an AMD Ryzen 5 5600X (12 logical CPUs), Linux x86-64, rustc 1.99.0
(b940084d7), using `fast` (release optimization, LTO off, 16 codegen units).
Resource loading took 705 ms; fixture construction took 66 ms sparse, 360 ms
dense, 392 ms global and 385 ms dense_portal, shared by both implementations.

Median microseconds per tick (seven paired samples):

| Palette | Workload | Original | Cached | Speedup |
| --- | --- | ---: | ---: | ---: |
| sparse | steady | 40.058 | 16.836 | 2.38x |
| sparse | cold | 40.203 | 48.619 | 0.83x |
| sparse | reload | 40.828 | 44.552 | 0.92x |
| sparse | edit_1pct | 41.282 | 52.200 | 0.79x |
| sparse | edit_all | 50.119 | 4192.396 | 0.01x |
| dense | steady | 244.610 | 21.869 | 11.19x |
| dense | cold | 292.533 | 1505.691 | 0.19x |
| dense | reload | 250.397 | 1301.608 | 0.19x |
| dense | edit_1pct | 256.522 | 69.132 | 3.71x |
| dense | edit_all | 263.829 | 5904.581 | 0.04x |
| global | steady | 6179.070 | 360.082 | 17.16x |
| global | edit_1pct | 6251.530 | 1565.249 | 3.99x |
| dense_portal | steady | 256.854 | 21.078 | 12.19x |
| dense_portal | cold | 240.498 | 1526.251 | 0.16x |
| dense_portal | reload | 347.879 | 2037.123 | 0.17x |
| dense_portal | edit_1pct | 368.684 | 118.108 | 3.12x |
| dense_portal | edit_all | 282.876 | 7194.112 | 0.04x |

The global palette's `maybe_has` walks the whole ID range, so rescanning every
chunk each tick (cold, reload, edit_all) is intractable; those rows are not
measured for it.

Keep: the steady scan improves for every palette (2.4x to 17x), which is the
case that repeats each tick in real play. Under constant edits the cache also
improves the dense and global palettes (edit_1pct 3-4x) because the original
rescans through the per-tick map. Rescanning every chunk every tick (cold,
reload, edit_all) is slower than the original because a miss scans all 24
sections of a chunk, where the original only touched the sections a reach
overlaps. No whole-game speedup is inferred from this microbenchmark; real
terrain's palette sizes and block-write frequency determine the gain. Lighting
and other chunk changes conservatively invalidate a chunk's mask.

The cache adds a weak handle and mask per visited resident chunk, retains no
unloaded block data, and is pruned when a chunk unloads (`sync_tick_chunks`) and
cleared on resource reload, like `random_tick_sections`. Editing a cached chunk
can move its 176-byte wrapper (plus Arc bookkeeping) on the first edit after it
is cached; it does not force a deep copy of the block data. The original outer
reach-list allocation remains.
