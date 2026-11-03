# Random-tick eligibility benchmark

Run with the pinned prepared client resources:

```sh
flock /tmp/horizon-optimizations-20261015-heavy.lock env CARGO_BUILD_JOBS=2 \
  CARGO_TARGET_DIR=/home/bob/Documents/horizon/target \
  cargo run --profile fast -p horizon-client-lib --example random_tick_eligibility_bench -- \
  .local/horizon-client/26.3
```

The example keeps the original selection algorithm alongside the cache. Both run
in the same optimized binary, alternating measurement order. Each workload has
one discarded warmup pair and seven pairs of 300 ticks. It reports every sample,
resource loading and fixture construction separately. This measures eligibility
selection and writes, not whole-game tick latency or rendering.

There are 289 lit chunks (simulation distance eight), each with 24 sections.
Sparse sections have one non-ticking state; dense sections have 128 distinct
non-ticking states selected from the real prepared facts. Every third section
also contains grass at its final position. These are controlled fixtures, not a
claim about typical generated terrain.

Steady includes initial cache fill amortized over 300 ticks. Cold recreates the
cache each tick, including allocation and destruction; reload clears it each
tick while retaining capacity. Edit workloads alternate a block value in three
or all 289 chunks before every selection. Timing includes the additional
`Arc::make_mut` allocation caused by weak cache handles; the baseline drops those
handles before its turn. Both paths retain the original outer-vector allocation.

The initial scan-only baseline was measured before implementing the cache with
the same fast profile: sparse steady median 49.277 us and dense steady median
1487.781 us. Paired results below are the comparison used for the keep decision.

Measured on an AMD Ryzen 5 5600X (12 logical CPUs), Linux x86-64, rustc
1.99.0 (b940084d7), using `fast` (release optimization, LTO off, 16 codegen units).
All build/measurement commands held the shared heavy-work lock with two build
jobs. Resource loading took 1086 ms; fixture construction took 85 ms sparse and
375 ms dense, shared by both implementations.

Median microseconds per tick (seven paired samples):

| Palette | Workload | Original | Cached | Speedup |
| --- | --- | ---: | ---: | ---: |
| sparse | steady | 48.855 | 2.588 | 18.88x |
| sparse | cold | 50.319 | 39.110 | 1.29x |
| sparse | reload | 49.051 | 36.167 | 1.36x |
| sparse | edit_1pct | 49.110 | 2.850 | 17.23x |
| sparse | edit_all | 59.425 | 50.759 | 1.17x |
| dense | steady | 1311.924 | 4.397 | 298.37x |
| dense | cold | 1305.237 | 553.209 | 2.36x |
| dense | reload | 1292.879 | 542.848 | 2.38x |
| dense | edit_1pct | 1290.162 | 9.102 | 141.74x |
| dense | edit_all | 1303.115 | 561.606 | 2.32x |

Keep: steady selection improves repeatedly, and cold/refill/edit-heavy selection
also improves in this fixture. No whole-game speedup is inferred from this
microbenchmark. Real terrain's palette sizes and block-write frequency determine
the gain. Lighting and other chunk changes conservatively invalidate eligibility.

The cache adds a weak handle and mask per cached resident chunk: about 13 KiB of
hash-table storage for 289 entries on this platform, including capacity slack.
It retains no unloaded block data. A first edit after caching can allocate/move
the 176-byte chunk wrapper (plus Arc bookkeeping); it does not force a deep
copy. Its old Arc allocation can remain until the next query or removal (about
54 KiB for 289 edited chunks, including Arc bookkeeping). The edit rows include
the allocation cost. Unloads remove entries; resource reload clears the cache.
The original outer tick-vector allocation remains.
