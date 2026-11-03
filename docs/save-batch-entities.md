# Block entity save batches

`Game::with_entities` prepares each dimension-change, full-save and dirty-unload
batch through `save::chunks_with_block_entities`. For multiple chunks, it visits
the live block-entity and custom-name maps once, groups borrowed entries only for
requested coordinates, then clones their contents one chunk at a time. The
single-chunk path keeps the original filters; an empty batch visits neither map.

Chunk input order, map iteration order within each chunk, duplicate inputs and
arithmetic `>> 4` mapping for negative coordinates are preserved. World maps and
original `Arc<Chunk>` values remain untouched. Attaching tick/extras data, draining
modified flags, unload cleanup, synchronous save errors and asynchronous save
queue errors retain their existing order and handling.

## Reproduce

From this worktree, with no other benchmark running:

```sh
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/home/bob/Documents/horizon/target
cargo build --profile fast -p horizon-client-lib --example save_batch_bench
taskset -c 2 "$CARGO_TARGET_DIR/fast/examples/save_batch_bench"
cargo test --profile fast -p horizon-client --lib save::tests
cargo clippy --profile fast -p horizon-client --all-targets -- -D warnings
cargo fmt --all -- --check
```

Change the target directory and CPU affinity for other machines. The example
constructs all fixtures itself and removes its temporary biome/colormap inputs.
It compares a copy of the previous per-chunk scans against the actual production
helper on the same maps and chunks, validates exact full chunk NBT bytes (not
just entity counts), warms both paths five times, and reports medians of 21
alternating samples. Each sample repeats enough batches for approximately 10 ms
of the slower path, capped at 10,000 repetitions; large batches run once.
Inputs and outputs pass through `black_box`. Destruction of outputs is timed.

## Scope and memory

The fixtures cover a 16 by 16 loaded area spanning negative and positive chunks:
3,072 block entities / 768 names for a village and 12,288 / 3,072 for storage.
Each chunk has 12 or 48 mixed chests, furnaces and signs, with populated
inventories, custom item components, names, saved NBT overlays, saved entity
order, and an unmodelled end gateway. Batch sizes are 0, 1, 16, 64 and 256 chunks.
Terrain uses eight uniform stone sections plus sections containing the entities;
the minimal registry has five states and one biome. Prepared production assets
were not found in the relevant project/Horizon cache/data locations, so these
are synthetic workloads rather than captured saves.

`snapshot` measures the production grouping/clone helper. `snapshot+nbt` also
is optional (`--nbt`) and calls the production `save::encode` and
`nbt::write_named` paths, with no
precomputed save tables. Both exclude `Game::attach_save_data`, writer queueing,
compression, region I/O, rendering and startup. They do not establish an overall
save-latency, startup or FPS improvement. Serialization and populated inventory
clones can dominate this CPU stage.

The multi-chunk path adds an O(chunk-count) index and O(selected-entity +
selected-name) borrowed pairs (16 bytes per pair on this 64-bit machine, plus
vector spare capacity). It also temporarily stores input chunk references and
cloned `Chunk` values before the game wraps them in `Arc`. Unsaved entity/name
contents are not cloned, and the owned snapshots remain independent of unload
cleanup. No full-world cache or persistent index is added.

## Recorded results

Recorded on 2026-10-15: AMD Ryzen 5 5600X, Linux 7.2.8-zen1-2-zen,
`rustc 1.99.0 (b940084d7 2026-09-28)`, `fast` profile (optimized, LTO off,
16 codegen units), `CARGO_INCREMENTAL=0`, pinned to logical CPU 2. The retained
run followed completion of other Rust builds; a contended trial was discarded.
Full raw medians, repetitions, entity counts and NBT byte counts are in
[save-batch-entities.csv](save-batch-entities.csv).

| Fixture | Saved chunks | Before (ms) | After (ms) | Speedup |
| --- | ---: | ---: | ---: | ---: |
| village | 1 | 0.043732 | 0.043813 | 0.998x |
| village | 16 | 0.695763 | 0.578064 | 1.204x |
| village | 64 | 8.185510 | 7.467929 | 1.096x |
| village | 256 | 38.434320 | 36.447195 | 1.055x |
| storage | 1 | 0.172842 | 0.170303 | 1.015x |
| storage | 16 | 3.688055 | 3.242919 | 1.137x |
| storage | 64 | 35.799641 | 33.446834 | 1.070x |
| storage | 256 | 155.262657 | 147.501719 | 1.053x |

Multi-chunk snapshot speedups were 1.053–1.204x (5.0–16.9% less time).
Single-chunk village time changed by +0.19%, and storage by -1.47%; the original
scan strategy remains in place for both. Empty batches skip the maps, and their
nanosecond-scale ratios in the CSV should not be treated as a throughput claim.
All ten cases passed exact NBT byte equality, including names and block entities.
Optional combined serialization timings were not retained in this result set.
