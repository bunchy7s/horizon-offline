# Section visibility shortcuts

The helper in `crates/horizon-client-render/src/mesher.rs` returns the same 36 face-pair bits
as the original flood fill. It initializes its visited map with opaque cells,
resolving state opacity once per cell, and returns directly for fully open or
fully closed sections. During traversal, touching all six faces ends the fill;
so does accumulating all pairs across separate completed regions.

The visited map uses 4 KiB of worker stack instead of a 4 KiB heap allocation.
Mixed sections still use the original growable traversal stack. There is no
persistent cache or additional per-section storage. Every call scans all 4096
interior cells before traversal; cases that previously visited very few open
cells may benefit less, and their measurements are included below.

## Reproduction

From the repository root:

```sh
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/home/bob/Documents/horizon/target
cargo test -p horizon-client --lib --profile fast mesher::visibility_tests
cargo test -p horizon-client --lib --profile fast mesher::visibility_tests::visibility_benchmark -- --ignored --exact --nocapture --test-threads=1
```

The ignored test calls the actual production helper with `Padded` sections and
full-layout `Info` entries. Its reference is the exact helper from main commit
`73a019967e9665c6b96332988c13cb66679d8477`, with only its name changed. Both receive
identical inputs, validate equal outputs, and use `black_box` for inputs and
results. Each case warms up each implementation 200 times, then runs 11 samples
of 1000 calls each, alternating measurement order. Times are median nanoseconds
per section; padding construction and state-table construction are excluded.

No prepared section/assets fixtures were found in the bounded project/cache
search. These are deterministic synthetic, production-sized 16×16×16 sections
with a two-block padding border and four state IDs (two of each opacity).
They cover terrain, isolated cells, sealed and exposed cavities, narrow tunnels,
disconnected components, and seeded random opacity densities. Independent tests
also cover all 64 face subsets, all 256 octant masks, and 224 randomized sections.

The mesher already skips uniform all-air sections before calling this helper.
The open case describes other fully non-opaque sections and the helper itself.
These timings do not establish overall meshing, startup, or FPS improvements.

## Measurements

Measured 2026-10-15 on AMD Ryzen 5 5600X (6 cores / 12 threads), Linux
7.2.8-zen1-2-zen, x86_64, rustc 1.99.0 (b940084d7, LLVM 23.1.1). Cargo `fast`
uses opt-level 3, no LTO, 16 codegen units; incremental compilation was disabled.
The benchmark runs one test thread. Shared-target Cargo builds were serialized
with the benchmark by Cargo's build-directory lock.

| Case | Opaque cells | Before ns | After ns | Speedup |
| --- | ---: | ---: | ---: | ---: |
| Fully open | 0 | 27,019.3 | 2,691.6 | 10.04× |
| Fully closed | 4,096 | 4,823.5 | 2,788.7 | 1.73× |
| Terrain lower half | 2,048 | 15,274.3 | 14,258.1 | 1.07× |
| Dividing wall | 256 | 25,006.6 | 24,107.5 | 1.04× |
| Sealed cavity | 1,352 | 19,953.7 | 17,591.7 | 1.13× |
| Disconnected all pairs | 3,586 | 7,915.9 | 6,301.4 | 1.26× |
| Checkerboard | 2,048 | 26,846.5 | 18,697.4 | 1.44× |
| Six-face tunnel | 4,050 | 5,236.7 | 2,972.3 | 1.76× |
| Open with sealed cavity | 296 | 26,140.3 | 9,688.8 | 2.70× |
| Random 1% opaque | 38 | 27,657.9 | 9,061.1 | 3.05× |
| Random 25% opaque | 1,043 | 24,931.4 | 7,722.5 | 3.23× |
| Random 50% opaque | 2,050 | 19,811.5 | 5,754.7 | 3.44× |
| Random 75% opaque | 3,053 | 13,598.2 | 10,474.1 | 1.30× |
| Random 99% opaque | 4,064 | 5,330.7 | 4,023.7 | 1.32× |

No case regressed in this run; the 4–7% gains in wall/terrain cases are small
and more susceptible to timing noise. No distribution-weighted aggregate is
reported because real-world section frequencies were unavailable.

Validation passed: the two focused correctness tests, the opt-in benchmark
(with output equality assertions), `cargo fmt --all -- --check`, and
`cargo clippy -p horizon-client --lib --tests --profile fast -- -D warnings`.
The full parity suite and workspace-wide Clippy were not run.
