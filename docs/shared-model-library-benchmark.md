# Shared model library benchmark

`model_library_bench` measures `Assets::load` followed by item preparation using
that asset snapshot. It includes filesystem traversal, JSON decoding, blockstate
binding, block model resolution/baking, sprite preparation, item definitions,
entity parts and item registry setup. Timing stops before the optional parity
probe. The Python driver's `wait4` resource accounting reports process peak RSS
on Linux. For latency runs, allocation tracking is compiled out. For separate
retained-memory runs, compile with `HORIZON_BENCH_COUNT_ALLOCATIONS=1`; the counting
system allocator reports live requested bytes after both loads. A zero
`retained_bytes` with tracking disabled is not a heap measurement.

Use real prepared resources; no game files belong in Git. Build the baseline
at `89b58991bbc02eb79b6965fc4ca56357fe2297d9` with the benchmark example copied in
and `ItemRenderer::load(root)` in its item stage. Save that binary, then build
this branch with `ItemRenderer::load_with_library(root, assets.model_library.clone())`.
Both builds use `cargo build --locked --profile fast -p horizon-client-lib --example
model_library_bench` with `HORIZON_BENCH_COUNT_ALLOCATIONS` unset (`opt-level=3`, LTO disabled, 16 codegen units). Set
`CARGO_BUILD_JOBS=2`. On the optimization machine, hold
`/tmp/horizon-optimizations-20261015-heavy.lock` for each entire build or paired
measurement session. Reuse a shared Cargo target only under that lock.

```sh
flock /tmp/horizon-optimizations-20261015-heavy.lock \
  python3 tools/model_library_bench.py BASELINE_BINARY CANDIDATE_BINARY RESOURCE_ROOT
BASELINE_BINARY RESOURCE_ROOT --parity
CANDIDATE_BINARY RESOURCE_ROOT --parity
```

Run the parity probes under the same lock. The driver runs two warmups for each
binary, followed by ten samples per binary in alternating order. Every sample
uses a fresh process, including setup and allocation costs. Filesystem caches
are warm; these measurements do not represent cold disk, GPU initialization or
entire time to title. The library is built during asset preparation, so compare
the combined time rather than either stage alone. Block-only callers of
`Assets::load` now prepare and retain item models too; standalone
`ItemRenderer::load` remains available for callers without an asset snapshot.

The optional parity probe hashes every bound registry state's selected variants
and baked block faces at two fixed positions, then every resource model's baked
item geometry, lighting, transforms and particle material. It complements the
synthetic test for malformed/missing parents, cycles, child overrides, built-in
models, shared geometry and reload isolation. It is a baseline comparison, not
a replacement for vanilla differential harnesses.

## Results

Prepared 26.3 resources contain 4,192 model JSON files. Both binaries use the
optimized `fast` profile and the same resource root on this Linux machine
(12 logical CPUs, 31 GiB RAM; rustc 1.99.0, b940084d7). Tracking is compiled out
for these ten samples.

| Median | Baseline | Shared |
| --- | ---: | ---: |
| Assets (ms) | 450.1310 | 477.4105 |
| Items (ms) | 83.1180 | 36.2980 |
| Combined (ms) | 532.6055 | 514.1075 |
| Peak RSS (KiB) | 275440.0000 | 277018.0000 |

Combined startup preparation improves by 18.4980 ms (3.47%). The
candidate is faster in all ten pairs. Stage medians do not necessarily add to
the median combined time. Peak RSS increases by 1578 KiB (0.57%).

Combined sample times in milliseconds, in paired order:

```text
baseline: 531.090, 528.995, 523.807, 534.564, 538.876, 529.148, 531.804, 533.407, 541.323, 539.934
candidate: 514.767, 513.659, 514.896, 506.912, 516.291, 514.556, 510.443, 513.191, 510.017, 519.038
```

Separate builds with allocation counting enabled report the same retained
requested heap in every sample: 129,921,272 versus 129,921,320 bytes (+48 bytes)
after both loaders. After assets alone, the values are
115,873,670 versus 122,249,411 bytes (+6.08 MiB). The shared models
are prepared earlier, so block-only asset consumers pay this retained-memory
and loading-time cost even when no item renderer follows.

The baseline and current candidate resource probes both produce
`86a447cfd3f535c2` over all 4,192 item models and the block selections/faces at
two fixed positions. The focused test additionally preserves item-only error
timing, so invalid item identifiers do not change resource-pack rollback.

## Integration check against updated upstream

After merging upstream `c6eb50f10b04a6fa01fffe1ca94a8420cc148342`, including its
model-state schema reuse, the same protocol compares that upstream commit
against this implementation on top. Build and paired measurement use separate
heavy-lock acquisitions. The allocation tracker remains compiled out for latency.

| Median | Updated upstream | Shared |
| --- | ---: | ---: |
| Assets (ms) | 410.753 | 434.221 |
| Items (ms) | 83.7835 | 36.578 |
| Combined (ms) | 495.526 | 471.568 |
| Peak RSS (KiB) | 238280 | 239944 |

Combined preparation improves by 23.958 ms (4.83%), faster in all ten pairs.
Peak RSS increases by 1664 KiB (0.70%). Separate counting builds again retain
129,921,272 versus 129,921,320 bytes (+48 bytes) after both loaders, and
115,873,670 versus 122,249,411 bytes after assets alone. The baseline and candidate
probes still match `86a447cfd3f535c2` over 4,192 models.

```text
baseline: 498.655, 498.148, 494.521, 493.105, 492.193, 501.778, 493.373, 494.278, 496.531, 497.549
candidate: 470.865, 469.526, 481.467, 475.666, 471.591, 472.186, 479.168, 469.798, 471.545, 469.152
```
