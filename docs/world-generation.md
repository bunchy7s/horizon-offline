# World generation

Horizon targets the repository-pinned Minecraft Java Edition **26.3**.
`horizon_world::worldgen::decode_noise` implements its direct normal-noise JSON
codec. `DensityRegistry` decodes named density definitions and `NoiseRouter`
loads the eight router roots against the same graph. `AquiferConfig` binds the six
aquifer density roots, and `NoiseBasedAquifer` implements underground fluid and
pressure selection. Complete terrain generation remains pending.

The decoder reads numeric tokens directly from `serde_json::value::RawValue`.
This preserves decimal precision and the sign of integer `-0`, avoiding a
double-precision intermediate before float density decoding. Integer
fields retain Gson's truncation toward zero and wrapping to the low 32 bits,
including decimal and exponent forms.

Noise definitions require a base octave in `[-32,32]`. Defaults are one octave,
unit base amplitude, enabled normalization and an empty amplitude-modifier list.
Octave counts range from 1 to 32. Nonempty modifier lists must contain exactly
that many values. Amplitude bounds retain vanilla's floating-point comparator
ordering: negative zero is below the modifier lower bound of positive zero.
Normalization accepts booleans or the string `"legacy"`. Null optional fields
use their defaults; unknown keys are ignored and duplicate keys retain the last
value.

The API accepts strict JSON. Gson's additional permissive syntax is outside this
decoder's scope. Construction failures return typed errors; comparison checks
classification and values rather than Java exception messages.

After preparing the pinned artifacts, run:

```sh
python3 tools/test_worldgen_parity.py
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The original Java transport uses vanilla's actual `NormalNoise.DIRECT_CODEC`,
then its normal and legacy Nether initializers. The suite compares every bundled
noise resource, six world seeds, both random streams and both initialization
paths. Additional cases check codec defaults, normalization modes, field bounds,
invalid types, nulls, duplicate/unknown keys, decimal integer conversion, signed
zero and synthetic parameters. Comparisons include octave metadata,
normalization/range bits, scalar noise and parent random state after success or
failure. Raw registry contents and outputs remain private under `.local`.

The density loader resolves forward references into distinct immutable named
holders. Numeric and object roots retain direct-value semantics. Its graph is
loaded atomically, with typed errors for missing references and cycles. It
implements the bundled function kinds, including nested splines, shifted noise,
interval selection, rounding, surface searches and context-bound fields. Graph
nesting and interval branches are bounded at 256; inline noise holders remain
pending. These bounds are explicit limits of the Rust API.

The registry comparison decodes all **55 bundled density definitions** using
vanilla's actual registry-aware codecs and the **64 bundled noise resources**.
It compares ranges, axis dependencies, compilation failures, cache allocation,
seeded scalar values and repeated strided volumes in raw and optimized modes.
Density roots are checked through one compiler per seed and mode. Nonempty
blending and structure fields are outside these fixtures.

The local run produced **961,693 matching output lines per reference JAR**. The
[metadata report](../benchmarks/rust-worldgen-2026-10-06/scoped-parity.json) records
source hashes, reference identities and resource digests. The runner authenticates
both artifacts, verifies that their registry resources agree, removes stale success
reports and rejects source or fixture changes during validation. CI includes the suite on
its configured platforms; the recorded local run does not establish remote
results or gameplay parity.

The router loader requires all eight fields in vanilla declaration order. It
retains named holder identity and compiles every field through one shared cache
namespace. Failed router decoding leaves the registry graph intact. All seven
bundled preset routers now match the pinned JARs across six seeds,
both random algorithms and raw/optimized compilation. Single-layer surface
volumes and invalid multilayer surface requests are both checked. This API
decodes the router
object. The vertical `NoiseSettings` and optional `aquifers` members are also decoded;
complete generator settings and material configuration remain pending.

`NoiseSettings` in 26.3 contains only minimum Y and height. Its JSON fields are
bounded to `[-2032,2031]` and `[0,4064]`. The guard checks the wrapped upper limit
first, then height alignment and minimum alignment to 16. The runtime create
helper applies those guards without the JSON field bounds. Height-accessor
clamping retains wrapped arithmetic, unaligned results and negative heights for
disjoint ranges. The suite compares bundled noise members, runtime and codec
construction, integer conversion and extreme accessor bounds.

Climate sampling converts quart positions to block coordinates with wrapped
32-bit shifts, evaluates all six fields in order with the same context, and
quantizes using float multiplication followed by Java's saturating long cast.
The suite compares vanilla's actual climate sampler binding and 4,116 quantization
fixtures covering random float bits, subnormals, NaNs, infinities and saturation.
The climate parameter decoder accepts scalar intervals, two-element lists and
objects with `min`/`max` fields. Endpoints use directly parsed floats in `[-2,2]`,
with Java float comparator ordering before quantization. Parameter points require
all six coordinates and an offset in `[0,1]`; negative-zero offsets fail that
lower bound. Codec comparisons include precision-sensitive decimals, missing
fields, invalid types and duplicate keys.

The immutable climate search tree now reproduces vanilla's stable construction
order, 19-child default, wrapped interval distance and squared fitness, pruning,
and previous-result tie handling. A searcher owns its previous leaf and borrows
one index, preventing cache reuse across unrelated indices. Indexed and brute
lookup intentionally retain their respective tie behavior. The suite compares
all supported branching values 2..32, sizes around branch powers, duplicate
entries, a 65,536-entry tree and extreme signed-long ranges/targets. The entry
bound and typed failures are explicit Rust API limits.

`MultiNoiseBiomeSource` resolves biome identifiers through scalar climate sampling
and prepared quart volumes. Bulk preparation evaluates six fields in declaration
order and retains their float buffers; each query quantizes those stored values.
Scalar and bulk queries share one previous-result state, so their query order
retains vanilla's tie behavior. The bounded API rejects coordinates outside a
prepared volume.

The runner exports built-in parameter tables independently from both authenticated
JARs, requires them to agree and keeps them private. These contain 7,594 Overworld
entries covering 56 biome identifiers and five Nether entries. Rust loads the
quantized tables using a Horizon import format. This is an import of built-in
data; the preset builders and vanilla biome codecs remain pending. The reference
uses actual `MultiNoiseBiomeSource` resolvers with keyed registration holders.
These checks resolve identities without evaluating biome properties or gameplay.
Both tables are exercised against every router, including cross-preset fixtures;
this does not establish End biome-source behavior.

The report records table and fixture digests. Source or private fixture changes
during a comparison invalidate its success report.

The global fluid picker and disabled-aquifer path are now implemented. Lava
selection uses Y below `min(-54, sea_level)`; other positions use the default
fluid status. Fluid status lookup returns its state only below its level.
Disabled aquifers use the default solid block for positive density;
nonpositive or NaN density calls the picker, and this path never schedules fluid
updates. The reference comparison uses the actual global picker lambda and
`Aquifer.createDisabled`, counting picker calls and checking default state
identities. Regular-mode fixtures cover sea-level cutoffs, extreme coordinates,
subnormal density, signed zeros, NaNs and infinities. Developer fluid-disable
behavior is outside this corpus.

The noise-based aquifer implementation retains the six-field configuration's
binding order, seeded positional cell generation, nearest-cell ties, pressure
barriers, surface sampling, fluid levels and update scheduling. One context is
shared across all density roots. Cell locations and statuses belong to each
chunk aquifer; surface values are snapped to quart coordinates and cached.
Global fluid statuses use exact state identity where vanilla does, while lava,
water and air predicates classify blocks, including nondefault liquid states,
cave air and void air. Dry cells use the version's `WAY_BELOW_MIN_Y` value
`-32512`; this differs from the global developer-disable status's `-4064`.

The Rust API limits grids to 16,777,216 cells and constructor surface volumes to
2,097,152 samples, checks array bounds and returns typed errors. It preserves
wrapped grid indexing within that domain, including vanilla's final-index
aliasing. Allocation failures and invalid oversized grids are outside the
comparison corpus. Read-only cache inspection does not populate caches.

The noise-based comparison uses actual vanilla configuration decoding and aquifer
construction. It checks fluid states, scheduling flags, sampler call order and
values, global picker calls and final location/status/surface caches. Fixtures
include all bundled aquifer configurations, both seeded random factories,
strided volume extents, repeated/reversed queries, nonfinite density/sampler
values, malformed roots and injected failures followed by retries. The runner
requires every injected failure to occur. Developer fluid-disable behavior is
outside these regular-mode checks. This does not validate complete generated
chunks or the registry-backed block classification adapter independently.

`TagDefinitions` decodes tag entries and files, applies resource-stack replacement,
and resolves required and optional dependencies before building ordered, deduplicated
memberships. Missing required references omit the affected tag while unrelated tags
remain available. Lookup order and required flags retain the pinned runtime's hash
bucket and dependency traversal behavior. A valid JSON resource that fails its tag
codec can introduce an empty tag; invalid replacement leaves earlier entries intact.

The comparison checks all **300 bundled block-tag files** against **1,286 block
identifiers** exported independently from both JARs. Synthetic fixtures cover
replacement, malformed files, missing references, cycles, duplicate members and
lookup order. The API is bounded to 8,192 definitions and dependency depth 256;
Java hash-tree buckets and pathological self-recursive dependency searches remain
unsupported. These checks cover tag loading, not the gameplay behavior of members.

The separate [heightmap implementation and comparison](heightmaps.md) now checks
all six types, packed data, updates and priming against actual vanilla chunks.

The [noise-fill stage](terrain-filling.md) now integrates final density, aquifers,
registry-backed sections, world-generation heightmaps and fluid postprocessing.
Complete terrain still requires material and surface rules, nonempty world fields,
biome/feature/structure placement and complete generated-chunk validation. No rendering, frame-rate or complete gameplay claim follows from
these decoder checks.


Subsequent [biome lookup and chunk suites](biome-lookup.md) now validate fuzzy
block-to-quart selection, section-atomic biome fills and multi-noise chunk
assignment through actual `ChunkGenerator.doCreateBiomes`. The original report
above remains a record of its narrower tested source snapshot.

## Whole-chunk differential harness

`tools/test_chunk_generation_parity.py` generates chunks in actual vanilla classes (`VanillaChunkGenerationOracle`: structure starts and
references, biomes, noise, surface, carvers, then biome decoration and structure pieces through real `WorldGenRegion`s) and with
`crates/horizon-world/examples/probe/chunk_dump.rs`, and compares every block state, quart biome and block entity (canonical NBT) of each target
chunk, and the scheduled block and fluid ticks each proto chunk carries into the live world. Cases are terrain sites for several seeds
across the overworld, Nether and End, plus rectangles around every structure kind found by scanning the seed. `--phase structures`
isolates terrain plus structure pieces, `--phase terrain` stops before features. `HORIZON_ORDER` (a file of `x z` lines) makes both sides
decorate in that order instead of row-major.

### Data packs

A world's enabled data packs (`<world>/datapacks`, in `datapacks.json`'s order) change its generation as vanilla's registries do:
`worldgen::overrides` maps each pack file to the bundle it joins (noises, density functions, noise settings, material rules and
conditions, carvers, configured and placed features, structures, structure sets, template pools, processor lists, biomes, level stems), a
higher pack's file for an id replacing a lower one's, and its block, fluid and biome tag files to the tag bundles in pack order.
`GeneratorData::load_with` reads the bundles with them; the client caches the result per set of pack entries
(`vanillagen::shared_data_with`). What Horizon leaves out is logged: a pack's new biomes (the game has no id, colours or sounds for them;
their tag entries and climate points go with them) and folders the generator does not read. A pack's dimension types set each level's
height ([world-height.md](world-height.md)); noise settings taller than the level generate only inside it, as `clampToHeightAccessor`. Sapling growth and bone
meal still place vanilla's features.

`tools/test_datapack_worldgen_parity.py` runs the whole-chunk harness with `HORIZON_DATAPACKS=tools/fixtures/datapacks` on both sides
(the oracle selects every pack of the folder after vanilla through vanilla's own pack repository): its pack overrides a noise, a density
function, the overworld noise settings (sea level 70, no aquifers, a new final density with a new noise added), a placed feature and a
biome tag. Every site must match vanilla with the pack and differ from Horizon without it.

Packs must be written for 26.3: its worldgen formats differ from 26.2's (density functions take `left`/`right`, `gradient` replaces
`y_clamped_gradient`, noises take `base_octave`/`octave_count`, block states are strings, surface rules became the `material_rule` and
`material_condition` registries, configured features lost `config`). Vanilla 26.3 refuses a 26.2 worldgen pack as Horizon does.

### Decoration order

Biome decoration is order dependent: a chunk's features write into its eight neighbours, so a tree, ore blob, sculk patch or geode placed
first changes where the next chunk's features fit (and how many random draws they take), and a neighbour's geode can cut through a
stronghold room or leave it whole. Vanilla decorates on one `worldgen` executor in the order its chunk scheduler produces around a player:
loading tickets ring by ring in fastutil hash order, four in flight, FULL tasks decorating their 3x3 squares x-outer, z-inner by ticket
level. Two vanilla servers decorating around a player teleported to the same spot agree on all but about 6% of the overlapping chunk pairs
(measured from JFR `minecraft.ChunkGeneration` events) and a few hundred blocks.

`worldgen::decoration_order` replays that scheduler for a standing player and the game's streamer decorates in its order (a chunk waits
only for lower-ranked chunks within two of it, so distant chunks still decorate in parallel). Fed vanilla's recorded order, the oracle
reproduces a vanilla server's world exactly; the model itself swaps as many pairs as a second vanilla run does.

`tools/test_decorated_region_parity.py` decorates a 13x13-chunk area around each site in that order on both sides and compares everything
the whole-chunk harness compares: the two reported sites on seed 1891623539983467248 (an old-growth taiga at 204 -239, a stronghold with
sculk and a geode at 4870 401), eleven tree-heavy biomes (taigas, dark forest, birch, jungles, savanna, cherry grove, mangrove swamp, pale
garden) and the nearest mineshaft, village, ancient city and trial chambers of two seeds: 21 sites, 3,549 chunks, 0 differences.

Result on seed 12345 (overworld 42 sites / 372 chunks, Nether, End): structure pieces 0 differences on all 35 overworld structure sites; full
generation 4 differing chunks (3 blocks, 1 block entity) out of 372; Nether and End terrain sites 0 differences.

Harness normalisations (not generation differences): sections above the noise height (Nether roof, End sky) are all air in vanilla and
absent in Horizon; structure templates keep their stored NBT integer width; a vanilla block entity left as a `DUMMY` default is equal to no
Horizon entry; `LootTableSeed` without a `LootTable` and a vault's default reward table are ignored.

Remaining known differences: a handful of single blocks near structure pieces in full generation (abandoned camps, shipwreck, villages) that
need per-feature tracing, and tree features only through `tools/test_tree_feature_parity.py` (8,898 fixtures, all equal).
