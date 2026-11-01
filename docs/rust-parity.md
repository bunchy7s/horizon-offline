# Horizon Rust rewrite status

The behavior target is the repository's pinned **Minecraft Java Edition 26.3**.
One-to-one vanilla gameplay parity is the acceptance criterion. It has **not**
been reached, but the Rust game is now playable: the [native client](native-client.md)
has a title screen and world management. It generates vanilla terrain with
decoration and streams, lights and renders it. Survival play works: mining,
placing, crafting, chests and furnaces, items, mobs, combat, hunger, health,
sounds, and Anvil saves.
The Rust workspace implements core arithmetic/random streams, world data and
resident sections, scheduled tick ordering, geometry, contained fluid states,
noise/density evaluation, biome and terrain generation kernels, material rules,
and parts of feature placement and decoration. Movement/fluid interactions,
registered attributes, hunger storage/ticks, synchronized player health and
absorption, damage reduction, outer hurt gates, blocking-item damage, item
durability and item-use start/tick/release/completion have scoped native comparisons.
Shared inventory/equipment storage now binds item-use flags and lifecycle, while
item templates, remainders, checked bundle weights and cooldown expiry have
native comparisons. Item completion composes stack copies with remainder and
cooldown effects and server inventory delivery. Consumable start/completion now
composes food storage, native random streams and eating particles, with exact
sound parameters and effect ordering checked against both pinned JARs. Consume
effect bodies now cover status application/removal/clearing, sounds and random
teleport attempts. Shared status instances implement hidden-chain updates,
server/client countdown and client blending with native comparisons. Entity
effect storage now binds consumable status changes and the effect tick loop,
including expiration, periodic refresh and client particle selection. Registered
effect behavior, notification side effects and full entity/world lifecycle
bindings remain unfinished. All pinned block-survival
families now have native differential coverage. Full simulation callbacks,
complete chunk generation, entity behavior, networking, rendering and playable
integration remain unfinished. The linked subsystem documents and hashed reports
record the exact tested scope and remaining gaps.
The `horizon-assets` crate now decodes pinned cuboid models, with all 2,883 bundled
block-model JSON files and 2,457 synthetic cases compared against the actual
client parser. [Decoder scope and validation](native-client.md#validation)
also include parent/texture resolution over 5,648 roots and cuboid baking over
7,100 roots with identity blockstate transforms. Blockstate resource decoding now
matches all 1,288 bundled definitions and 1,351 synthetic cases; registry
binding now also matches native selection across registered and special
item-frame states. Weighted selection and multipart random draw consumption
also match native traces. Transformed baking now checks every bundled
model/rotation/UV-lock tuple and all XYZ quarter-turn combinations against native
geometry by exact float bits. The native renderer now draws its controlled
resident chunks from registry-bound, selected and transformed models, including binary
cutouts, multipart connections, weighted stone variants and stepped lantern
animation. All 16 scene selections match native model collection. These are not
yet simulated gameplay blocks. Resident chunk meshing now preserves weighted
variants and UV orientation when greedily merging matching cube faces, and uses
native-checked neighbor visibility. Full block-catalog material support, lighting
and chunk streaming remain unfinished.
[Neighbor face visibility](face-visibility.md) now matches both reference JARs
across 1,441,816 state pairs, including all pinned suppression-rule families.
Animation metadata, frame sequences and tick-state selection now have native
differential coverage, including all bundled animated texture metadata. GPU
interpolation and game-loop synchronization remain unfinished.

## Completion criteria and playable builds

The final deliverable is the complete Horizon Rust game with one-to-one parity
with pinned Java Edition 26.3, plus a playable **Linux** build that the user can
download and test. The target platform is a Linux x86-64 desktop with an NVIDIA
GPU (Vulkan) on Wayland. macOS and Windows are out of scope. The build must include a packaged native
executable, required runtime resources, launch instructions, artifact checksums,
and the tested OS/CPU/GPU configuration.

The build is complete only after its packaged Rust game is launched and tested on the target machine:
create/load a world, render terrain, use keyboard/mouse controls, move and interact,
break/place blocks, use inventory/crafting, save, quit and reload. These launch
checks accompany the full gameplay-parity suite; they do not replace it. Record
failures and test limits. Compiler success, library comparisons
and the existing Java packs do not establish a playable Rust build.

Optimize FPS, frame pacing, memory use and render distance using matched scenes,
worlds, settings and hardware after behavior is validated. Package and test on
the target machine before declaring the rewrite complete. Commit and push
verified development checkpoints regularly.

## Implemented foundation

`crates/horizon-core` is a dependency-free Rust library with no unsafe code.
It uses small integer position values, exclusively owned RNG state, and packed
64-bit storage. These choices avoid Java object overhead in this slice; no
whole-game memory, FPS, or render-distance gain has been measured.

| Area | Implemented and checked operations | Remaining scope |
| --- | --- | --- |
| Positions | Block, section, and chunk packing/unpacking; wrapping block offsets; block-to-section/chunk conversion; section-relative packet positions; block-state storage indices; region coordinates/local indices; bounded inclusive/directional block-box iteration | Remaining iterators, world bounds and entity/world integration |
| Simulation geometry | Selected Vec3 operations, quantized trig/rotations, normalized AABB bounds and transforms, finite block traversal/swept visitation, ray faces, immutable cube/array voxel grids, greedy boxes, collision distances, column heights, closest points, lazy floor height, box quantization and Boolean/optimized joins; movement resolution over supplied shapes, step candidates and step-up decisions/query bounds | Shape rotations/faces/edges, entity-context shape selection, full entity movement and side effects |
| Legacy random stream | Seeding/reseeding; int, bounded/inclusive/exclusive int, long, boolean, float, double; triangular float/double draws; fork; consume count; positional factories and Java UTF-16 string hashes | Gaussian sampling, threading-error behavior and remaining world-generation call sites |
| Xoroshiro random stream | 64-bit seed expansion; explicit 128-bit state including all-zero fallback; the same selected random operations as above; positional factories and MD5-derived seeds with Java UTF-8 surrogate handling | Gaussian sampling, serialization and remaining world-generation call sites |
| Noise generation | 26.3 Perlin/simplex/smeared Perlin; permutation construction and coordinate wrap; Perlin derivatives; cached/scalar density-volume accumulation; nested/flattened stacks; normal-noise octave weights and normalization; modern and legacy Nether seeding | Serialized noise definitions, density graphs, complete chunk generation, biome/feature/structure integration |
| Density evaluation | Compiled constants, clamped/repeating/mirrored gradients, unshifted and XZ/XYZ shifted noise; offset-noise variants, old blended noise, outer End islands and distance-to-point; all eleven unary operators, six binary operators, powers, rounding, constant-operand specializations, min/max, clamp, range choice, interval selection and linear interpolation; separate scalar/bulk paths, nested scratch buffers, context-bound blend alpha/offset/structure samplers, absent/empty blend-density wrappers, context-owned scalar/volume caches, coordinate slices, coarse-grid interpolation, surface searches and nested cubic splines | Codecs, structure-density producers, nonempty old-world blending and terrain integration |
| Density graph compilation | Immutable definitions for constants, gradients, arithmetic/selection operations, powers, rounding, distance-to-point, slices, caches, interpolation, surface search, nested splines, outer End islands and normal/offset/blended noise; context fields, blend-density wrappers, direct/named reference holders; range/axis metadata, constant and disjoint-range specialization, uniform-axis rewriting, cache-ID allocation/deduplication, memoized roots, spline coordinate sharing/validation, world-seed lookup and failure state | Codecs, concrete world-field producers and complete terrain integration |
| Density range analysis | Checked interval constructors, hulls, arithmetic/min/max, reciprocal, abs/square/sign, powers/logarithms, clamp, monotonic endpoint mapping, finite/infinite interpolation, predicates, equality and Java hashes; conservative spline range propagation; integration with supported graph definitions | Propagation through remaining graph definitions |
| Packed palette-index storage | Widths 1–32; allocation; imported raw words; reads; writes returning old values; unpack; fill; padding/tail preservation; invalid-write rejection | Remaining network container integration |
| World data | Named/network NBT, modified UTF-8, mixed lists; gzip/zlib/uncompressed/lz4-java codecs; region exports including external chunks; block/biome saved palettes | Complete save lifecycle, data fixing, concurrent saves and recovery |
| Resident block sections | Pinned block-state registry import, defaults, numeric IDs, uniform sections, packed palette growth, freed-slot reuse and compaction; cached collision shape lookup bound to the same registry | Block behavior, entity-context collision shapes, light and full chunk-lifecycle integration |
| Heightmaps | Six type predicates and flags, packed heights, updates/removal rescans, shared priming, raw imports and length-mismatch fallback; all pinned states and mutation fixtures compared | Block air/fluid behavior and chunk-lifecycle integration |
| Scheduled ticks | Chunk queues, level collection/drain ordering, duplicate suppression, seven priorities, saved delays, chunk eligibility, callback mutations, area removal/copying, saved block/fluid NBT | Actual block/fluid callbacks, neighbor updates, tick-time and chunk-lifecycle integration |

Packet-relative positions use X/Z/Y ordering; block-state storage indices use
Y/Z/X ordering. Negative coordinate conversion must floor rather than truncate.
The legacy RNG must retain Java's signed-int arithmetic and rejection rules;
the xoroshiro RNG has a different bounded-int algorithm and bit selection.
Positional factories preserve signed coordinate hashing and each stream's distinct
seed derivation. The hash API accepts UTF-16 units so malformed surrogate sequences
retain Java's replacement behavior. Gaussian sampling is still absent: a private
probe found bit differences between Java `Math.log` and the platform logarithm,
which must be resolved before its values and cached-draw behavior can be claimed.
Storage entries never cross a word boundary. Vanilla's fill operation also
writes padding and unused tail bits; imported words retain their original bits
when a slot is changed. These details are covered by the differential corpus.

Rust APIs return typed errors instead of Java exceptions. Storage lengths and
indices use `usize`; raw decoded values use `u32` bit patterns. Writes reject
values above the nonnegative Java-int domain, including at width 32. Test output
casts raw imported width-32 reads back to signed Java ints for comparison.
Negative lengths and the complete Java object/API surface are not supported.

## Reproduce validation

The core/world libraries require Rust 1.85 or newer; the native client's locked
dependencies require Rust 1.90 or newer. Use current stable Rust, Python 3.10 or
newer, and JDK 25 for the complete workspace. From the checkout:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 tools/test_rust_parity.py --prepare
python3 tools/test_world_parity.py
python3 tools/test_tick_parity.py
python3 tools/test_geometry_parity.py
python3 tools/test_collision_parity.py
python3 tools/test_noise_parity.py
python3 tools/test_density_parity.py
python3 tools/test_worldgen_parity.py
python3 tools/test_heightmap_parity.py
python3 tools/test_terrain_parity.py
python3 tools/test_structure_density_parity.py
python3 tools/test_material_surface_parity.py
python3 tools/test_material_condition_parity.py
python3 tools/test_material_stage_parity.py
python3 tools/test_biome_zoom_parity.py
python3 tools/test_biome_chunk_parity.py
python3 tools/test_biome_generation_parity.py
python3 tools/test_biome_source_parity.py
python3 tools/test_anchor_parity.py
python3 tools/test_biome_temperature_parity.py
python3 tools/test_float_log_parity.py
python3 tools/test_raid_parity.py
```

`--prepare` regenerates the local reference artifacts from the existing lock;
omit it after preparation to run without downloads. The runner verifies the
official manifest and both game downloads, the server's inner game JAR, and each
server dependency against pinned/bundled hashes. The original Java test adapter
calls vanilla classes directly; it does not substitute a Java reimplementation
of the Rust algorithms. The same corpus runs against client and server code.
The fixtures need no display, account, assets, or world generation.

The deterministic corpus contains coordinate extremes, all 4,096 local section
positions, 8,192 random coordinate/offset queries, boundary and random signed
seeds, mixed RNG operations, fork/reseed/consume checks, invalid bounds, and
all 32 storage widths at boundary and 4,096-entry sizes. It also imports words
containing all set bits and compares the raw padding and fill results. The
current run produces **1,634,520 matching output lines per reference JAR**.
It includes 4,400 positional-factory commands and 5,000 random-helper commands,
with overflowing integer intervals, hash padding boundaries and malformed UTF-16.
This count includes repeated checks and raw entries; it is not a count of
distinct vanilla behaviors or a percentage of game coverage.

Artifacts remain gitignored under `.local/rust-parity/`: corpus, Rust output,
client/server output, compiled oracle, and `report.json`. The report records
source/input/output hashes, reference identities, tool versions, and matched
line counts. A prior success report is removed before a new run can fail.
CI runs the same comparison in addition to offline Rust and Python checks.
The checked-in [scoped validation report](../benchmarks/rust-core-2026-10-05/scoped-parity.json)
contains only metadata and digests from the local run, not game code or artifacts.

The additional [world-data suite](world-data.md) checks NBT and region
interoperability, saved palettes, all 35,723 registered block-state IDs, and
resident sections after repeated mutation. The corresponding
[scoped report](../benchmarks/rust-world-2026-10-05/scoped-parity.json)
records client/server reference hashes and the tested source hashes.

The [scheduled-tick suite](scheduled-ticks.md) adds 25,815 commands including
2,096 tick invocations, producing 97,776 matching output lines per reference.
It also verifies block/fluid tick save codecs and a persisted region chunk.
The [geometry and cached-collision suites](collision-geometry.md) check numerical
boundaries and runtime block-state shape data, with separate
[geometry](../benchmarks/rust-geometry-2026-10-05/scoped-parity.json) and
[cached-collision](../benchmarks/rust-collision-2026-10-05/scoped-parity.json)
metadata reports. All differential runners reject changed source hashes during a run before publishing
a success report. Historic reports describe their own recorded source versions.

The [noise suite](noise-generation.md) checks base sampling, derivative and bulk
arithmetic, nested/flattened stacks, normal-noise normalization and both modern
and legacy Nether initialization. Its [scoped report](../benchmarks/rust-noise-2026-10-05/scoped-parity.json)
records the tested sources and both reference artifacts.

The [density suite](density-functions.md) checks compiled scalar and bulk samplers,
including gradients, nested shifted noise, arithmetic, caching, axis slicing and
coarse-grid interpolation, surface searches and nested splines with range analysis;
it also compares the initial graph compiler with vanilla's compiler. Its
[scoped report](../benchmarks/rust-density-2026-10-05/scoped-parity.json) records
**15,524,627 matching output lines per reference JAR**. That suite constructs graphs through its transport. Serialized registry
comparisons are covered separately below; generated terrain remains pending.

The [float-log report](../benchmarks/rust-float-log-2026-10-05/scoped-parity.json)
records exhaustive compiled scalar comparisons for all **4,294,967,296 float bit
patterns** against both JARs on the local Linux/JDK runtime, with canonical NaN
results. Double-precision logarithms and other-platform math remain separate
validation work.

The [world-generation codec suite](world-generation.md) checks every bundled
normal-noise JSON definition and all 55 bundled density definitions against
vanilla's actual registry-aware codecs. It compares decoded octave metadata,
graph ranges and axes, raw and optimized compilation, cache allocation and
seeded scalar and strided bulk sampling. Its
[scoped report](../benchmarks/rust-worldgen-2026-10-06/scoped-parity.json) records
**961,693 matching output lines per reference JAR**. All seven bundled eight-field
routers and their climate samplers are also
compared, including quart-coordinate wrapping and float/long quantization.
Climate search-tree construction and warm/fresh ties are checked across the
bounded API. Scalar and prepared-volume multi-noise biome resolvers are implemented
using private imported built-in parameter tables and compared against vanilla
scalar and bulk biome resolvers. Climate parameter codecs, vertical terrain bounds and the global fluid picker/
disabled-aquifer path are also compared. Noise-based aquifer configuration and
fluid-pressure selection are implemented, with differential checks for seeded
decisions, sampling order, caches and failure recovery. Tag codecs, stacked resource
replacement and bounded dependency resolution are compared across all 300 bundled
block tags and synthetic missing-reference/cycle fixtures. Complete terrain
integration remains pending.

The [heightmap suite](heightmaps.md) records **1,671,668 matching output lines per
reference JAR** across all 35,723 block states and 100 chunk fixtures. Rust resolves
tag membership; block air/fluid facts are private, independently verified imports.
Actual vanilla heightmaps compare predicates, mutations, packed words, priming and
selected ordered block reads. Chunk-lifecycle integration remains pending.

The [noise-fill suite](terrain-filling.md) now compares **7,950,873 output lines per
reference JAR** across 114 fixtures and 200 fills. Final density, aquifers, resident
sections, world-generation heightmaps and fluid postprocessing are integrated for
this stage. The surface stage has a separate comparison; biome, feature and
structure stages remain pending.

The [structure-density sampler](structure-density.md) matches **1,360,095 output
lines per reference JAR**, including ordered rigid-piece/junction contributions
and scalar/bulk clipping. Live placement of a start in a running level is ported
(`/place structure`, through `StructureStart.placeInChunk`); structure discovery
during chunk generation is compared by the structure-layout suite.

The [vertical-anchor suite](vertical-anchors.md) matches **181,632 output lines per
reference JAR**, including four anchor codecs, runtime overflow, nested exclusive
fields and generation bounds.

The [material-surface primitives](material-surfaces.md) match **122,988 output
lines per reference JAR** for seeded terracotta bands, band lookup and surface
noise queries. Subsequent suites cover rule evaluation and surface application.

The [numeric material-condition suite](material-conditions.md) matches **5,627,426
output lines per reference JAR** for eleven condition forms and their column/block
cache transitions, including codecs, all eight bundled condition definitions and
block/sequence/conditional/terracotta-band rule composition.
The chunk surface comparison below integrates these rules.

The [biome-temperature suite](biome-temperatures.md) matches **836,659 output lines
per reference JAR** for temperature modifiers, precipitation decisions and ordered
cache behavior, including thread isolation and climate fields from all 67 bundled
biome definitions. Full weather placement remains pending.

The [surface-extension suite](material-surfaces.md#biome-specific-column-extensions)
matches **233,100 output lines per reference JAR** for badlands pillars and
frozen-ocean icebergs, including ordered column reads/writes and both random modes.

The [bundled material-rule suite](material-rule-libraries.md) matches **678,174
output lines per reference JAR**, including 665,712 rule applications across all
42 bundled definitions and the pinned biome, noise and density registries.
The [chunk surface stage](material-surfaces.md#chunk-surface-stage) matches
**6,733,382 output lines per reference JAR** across 79 fixtures, 42 terrain fills
and 106 surface builds, followed by 158 carving-mask passes, including 21,757,952 block values, heightmaps, fluid
postprocessing and ordered biome queries. It tests all seven bundled terrain
settings through both stages with shared density contexts and vanilla fuzzy
biome lookup against 378 generated neighbor biome chunks (400,896 stored cells).
Preset builders and complete generated worlds remain pending.

The [biome-lookup suite](biome-lookup.md) matches **256,933 output lines per
reference JAR** for seed obfuscation, eight-corner fuzzy selection, direct quart
queries and source replacement. Seed hashes also match an independent SHA-256
implementation. [Chunk biome storage](biome-lookup.md#chunk-biome-storage) matches
**478,340 lines per JAR** across 160 fills, including section-atomic failure
behavior and clamped reads. [Chunk biome generation](biome-lookup.md#chunk-biome-generation)
matches **383,072 lines per JAR** across 322 fills, including both imported
multi-noise presets and fixed/checkerboard/End sources through all seven terrain
routers. Preset builders and complete world generation remain pending.

The [fixed/checkerboard/End source suite](biome-lookup.md#fixed-checkerboard-and-end-sources)
matches **1,306,096 lines per JAR**, including 1,147,984 biome queries and ordered
End erosion sampling. [Registry-bound source codecs](biome-lookup.md#registry-bound-source-codecs)
match **363,121 lines per JAR** across 40,481 inputs, including canonical encoding,
possible-biome sets and resolved queries. Chunk generation and surface integration
share the resolved-source API. Inline biome definitions and complete registry
lifecycle behavior remain pending.

The [carving-mask suite](carving.md) matches **45,742 lines per JAR** across
28 fixtures, including packed column ranges, coordinate wrapping and mutations
retained after failing traversal callbacks. Mask application is included in the
surface-stage suite. [Shared carving geometry](carving.md#shared-shape-primitives)
matches **106,699 lines per JAR**, including 690 ellipsoids and 5,536 reachability
queries. [Seeded cave/canyon paths](carving.md#seeded-cave-and-canyon-paths) match
**882,005** and **1,094,517 lines per JAR**, respectively. The combined stage suite
also covers 42 procedural carving passes, including neighboring biome selection
and ordinal carver seeds. Full-world integration remains pending.

The [feature-placement primitives](feature-placement.md) match **1,268,600 lines per
JAR** across seventeen modifier kinds, including callback failures and shared random
state. [Modifier-chain execution](feature-placement.md#modifier-chains) matches
**548,461 lines per JAR**, including feature callback failures and reused pending
buffers. [Initial block-writing features](feature-placement.md#initial-block-writing-features)
match **1,820,614 lines per JAR** for the void platform, End islands and End arrival
platform. Remaining feature algorithms and complete world integration remain pending.

The [scalar value-provider suite](value-providers.md) matches **429,920 lines per
JAR** across 504 definitions and 5,040 seeded fixtures, including exact random
stream consumption. [Anchored height providers](value-providers.md#anchored-height-providers)
match **507,660 lines per JAR**, including all four anchors and overflowing contexts.
Weighted lists include zero weights, nested choices and maximum totals. Gaussian
distributions and provider encoding remain pending.

The [block-state JSON codec suite](block-state-codecs.md) matches **221,261 output
lines per reference JAR**, covering every registered state and compact/full
encoding plus malformed-property fallback. Block gameplay remains pending.

## Work required for a playable rewrite

| Subsystem | Current Rust status | Required parity evidence |
| --- | --- | --- |
| Registries, block states, items and data components | Block-state IDs, property descriptors and defaults imported and checked; items/components absent | Registry lifecycle, remaining registries, data components and behavior |
| World/chunk storage, NBT and region files | Bounded NBT, four codecs, region export, saved and resident section palettes | Generated-save integration, entity/POI integration, upgrades, live saves and crash recovery |
| Terrain, biomes, structures and lighting | Seeded noise/density graphs, bundled registry and router loading, climate sampling/search and imported-table multi-noise biome resolution, aquifer decisions, noise-stage filling and surface-stage application implemented; complete terrain generation absent | Remaining terrain/biome codecs and world fields, seeded generated-chunk comparisons, biome/feature/structure placement, chunk lifecycle and light propagation |
| Tick scheduling, blocks, fluids and redstone | Scheduled queues and saved ticks, block survival families, fluid geometry/flow and entity immersion kernels checked; complete live block/fluid/redstone simulation absent | Actual block/fluid state transitions, neighbor updates, world integration and end-to-end ordered traces |
| Entities, AI, movement and combat | Collision, movement/fluid kernels, registered attributes, damage reduction, hurt gates and immunity predicates checked; complete live entity simulation absent | Context-dependent collision/physics and tick traces, spawning, pathfinding, damage, equipment and lifecycle |
| Survival, inventory, crafting and progression | Hunger ticks/storage and synchronized health/absorption implemented and checked; inventory/crafting/progression absent | Recipes, item transitions, hunger/health, effects, experience and advancement behavior |
| Dimensions, portals, weather and game rules | Not implemented | Cross-dimension state, transitions, time/weather and rule-dependent behavior |
| Multiplayer, protocol, authentication and commands | Not implemented | Packet/codec fixtures, vanilla interoperability, reconnects and command behavior |
| Rendering, resources, sound, UI and input | Native inspection client with selected block models, neighbor visibility, greedy meshing and a free camera; gameplay rendering/audio/UI incomplete | Resource compatibility, render/culling correctness, controls and playable interaction tests |
| Performance and platform support | Registered-attribute allocation reduction measured on Linux; whole-game performance and playable platform packages pending | Repeated matched gameplay workloads, frame/tick distributions, memory and physical GPU tests |

The next simulation milestone is integrating the checked scheduler and
registry-backed sections with actual block behavior and neighbor updates. World
generation now has noise-stage filling, aquifers and surface-stage application;
biome assignment and remaining generation stages need work,
followed by complete seeded-world comparisons. The inspection renderer needs
complete material/lighting support, chunk streaming and a simulated world.
Movement, health and combat kernels need shared live player/entity state and
world services. Scoped comparisons alone cannot establish complete gameplay parity. Performance tuning must preserve the
validated simulation and use measured workloads; changing render distance,
simulation distance or terrain detail invalidates an equal-workload speed claim.

No finite corpus proves every vanilla behavior. Future milestones need broader
version-specific differential suites and playable integration checks. The
project will report implemented/validated scope rather than label a partial
port as 100% parity.

The [pre-light chunk-stage comparison](terrain-filling.md#block-writes-across-generation-stages)
adds **8,090,090 matching lines per JAR** for stage-dependent block mutation and
heightmap selection, including insertion/removal of every pinned state. This does
not establish the complete chunk lifecycle or region integration.

The [resident feature-region comparison](feature-placement.md#resident-feature-regions)
adds **28,922,634 matching lines per JAR** for basic features, configured fill layers, replacements and scattered ore sharing
modifier chains and resident neighboring chunks, including live heightmap updates.
World-region side effects and complete feature generation remain pending.

The [inline feature-graph comparison](feature-placement.md#inline-feature-composition)
adds **1,146,057 matching lines per JAR** for nested sequences, overlays and random selectors,
including short-circuit results, shared randomness and partial mutations.

The [named feature-resource comparison](feature-placement.md#named-feature-resources)
adds **2,528,704 matching lines per JAR** for registry-aware feature/placed-feature
holders, ordered tags and subsequent nested execution. Complete registry loading,
reloads and remaining feature algorithms are still pending.

The [configured fill-layer comparison](feature-placement.md#configured-fill-layers)
adds **1,305,274 matching lines per JAR** for block-state binding and ordered
column placement, including decoding every pinned block state. Complete feature
generation and gameplay remain pending.

The [replacement-rule comparison](feature-placement.md#replacement-rules) adds
**1,890,716 matching lines per JAR** for all ten native predicate kinds, bound
state/tag identities and ordered random evaluation. Complete ore and structure
generation remain pending.

The [single-block replacement comparison](feature-placement.md#single-block-replacement)
adds **303,678 matching lines per JAR** for ordered rule-driven block mutations,
rejected writes, failures and subsequent random state.

The [scattered-ore comparison](feature-placement.md#scattered-ore) adds
**758,484 matching lines per JAR** for attempt offsets, ordered air-exposure checks,
rule evaluation and block mutations. Ordinary ore veins and complete biome
decoration remain pending.

The [bulk-section comparison](feature-placement.md#bulk-section-access) adds
**959,640 matching lines per JAR** for cached raw section access, every block state,
all-air reads and unchanged heightmaps. This supplies storage behavior needed by
ordinary ore generation; vein geometry and concurrent ownership remain pending.

The [biome feature-order comparison](feature-placement.md#biome-feature-ordering)
adds **411,751 matching lines per JAR** for identity-based ordering, cycles and
step index mappings, including every bundled biome's feature lists. This does
not yet execute complete chunk decoration.

The [world-generation random-adapter comparison](feature-placement.md#world-generation-random-streams)
adds **238,800 matching lines per JAR** for native bit sampling, counters, forks,
nested wrappers and decoration/feature seeding. Gaussian sampling and complete
chunk decoration remain pending.

The [biome-placement comparison](feature-placement.md#biome-placement-filtering)
adds **121,423 matching lines per JAR** for top-feature contexts, native generation
settings membership, query order, nested ordinary placers and partial failures.
Complete biome sampling integration and chunk decoration remain pending.

The [decoration scheduler comparison](feature-placement.md#decoration-feature-scheduling)
adds **337,398 matching lines per JAR** through the native chunk-decoration entry
point, with structures disabled and traced feature sinks. It covers resident
biome selection, global ordering, per-feature seeds and failure/progress state;
complete feature algorithms and chunk lifecycle remain pending.

The [resident decoration comparison](feature-placement.md#decoration-over-resident-chunks)
adds **36,173,951 matching lines per JAR** for native chunk decoration combining
biome storage/zoom, generation-stage guards, scheduling, implemented feature
graphs, block writes and live heightmaps. It compares complete final resident
sections; structures, lighting and the full generation lifecycle remain pending.

The [block-state provider comparison](feature-placement.md#block-state-providers)
adds **257,299,719 matching lines per JAR** for ten state-provider kinds, including
all eight bundled provider definitions. It includes every state under six
directions, property transforms, nested draw order and empty selections. Rule-based
selection includes twelve predicate kinds, ordered conditions, empty results and
query failures. Named holder references are resolved through immutable resource
snapshots. Additional predicates, complete registry loading and complete feature placement remain pending.

The [contained-fluid comparison](world-data.md#contained-fluid-states) adds
**243,331 matching lines per JAR** for fluid-state mapping, liquid-block conversion
and surface/camera height queries. It covers every pinned block state and query
failures; fluid spreading, entity interactions and rendering remain pending.

## Block entity removal, drops and copper golems

`LevelChunk.setBlockState` runs `BlockEntity.preRemoveSideEffects` when a
block with a block entity is replaced by another block (`LevelAccess::remove_block_entity`,
`level/removal.rs`): containers spill every slot through `Containers.dropItemStack`
(chests, barrels, copper chests, hoppers, dispensers, droppers, crafters,
furnaces, brewing stands, chiseled bookshelves, shelves, decorated pots and
campfires, the last four from the block entities kept as loaded), jukeboxes pop
their record, lecterns their book, and shulker boxes keep their contents for the
dropped box. Copper chests and statues keep their block entity across
weathering, and `UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS` skips the spill.
`Block.dropResources` draws its loot and pop spots when the behaviour calls it,
with ore, sculk and spawner experience, and the item entities and orbs the
behaviour makes join the world before it returns, in the same tick. Player
mining, explosions and `/setblock`/`/fill destroy` use the same path in
vanilla's order.

`tools/test_block_removal_parity.py` replaces, destroys and removes every block
entity holder with random contents (4,000 cases) against `VanillaDispenserOracle`
running vanilla's `setBlock`/`destroyBlock`/`removeBlock`, and compares block
changes, contents left, item entities and orbs, events, sounds and the level
random: all cases match. The block update oracle replays the same removal, so
the block update, random tick and redstone harnesses cover it too.

The copper golem is a species: its model, weathered textures and glowing eyes,
keyframe animations (compared by the entity model harness), sounds, weathering,
waxing and scraping, statues, saved `weather_state` and `next_weather_age`, death
loot (entity death loot harness), the poppy iron golems give it, and building it
from a pumpkin on a copper block (the copper block becomes the matching copper
chest). The dispenser harness builds, shears and equips copper golems against
vanilla. Its chest-sorting AI (`CopperGolemAi` with
`TransportItemsBetweenContainers`) still runs as goals that follow vanilla's
behaviour without its brain: porting it onto the brain machinery of
`mobs/villager_brain.rs` and comparing it in the mob AI harness, which needs
container contents in the probe's world, is still to do.

## Overlay feature composition

The Rust overlay feature executes every child placed feature in order and
accumulates success, sharing the placer and random stream across siblings.
Named holders and placed-feature tags use the same catalog resolution as other
compositions. Empty named tags produce false; empty direct lists fail decoding.

The expanded [inline graph comparison](../benchmarks/rust-feature-graph-2026-10-06/scoped-parity.json)
matches **1,146,057 output lines per reference JAR**, and the expanded
[named catalog comparison](../benchmarks/rust-feature-catalog-2026-10-06/scoped-parity.json)
matches **2,528,704 lines per reference JAR**. These include partial failures,
rejected writes, nested modifiers, repeated calls, random-stream tails and both
Java random modes. This verifies scoped composition behavior; complete chunk
generation and gameplay remain pending. Workspace tests and all-target Clippy
also pass for this implementation.

## Block-pile placement

Rust block piles preserve radius and sparse-candidate random draws, air checks,
dirt-path acceptance, state-aware FULL upper support, provider fallback reads,
and update flags 260. Previously read support states are evaluated without an
extra world read. The [native comparison](../benchmarks/rust-block-pile-2026-10-06/scoped-parity.json)
matches **11,995,254 lines per reference JAR**, covering every candidate and
supporting registry state, partial failures, rejected writes, repeated placement,
and overlay/modifier composition. Workspace tests and all-target Clippy pass.

Ordinary piles visit at most 98 cells. Rust preserves wrapped native box traversal
but limits pathological iteration to 2^20 cells; negative-count overflow fixtures
compare failure-terminated prefixes. Dynamic block entities are absent in this
feature fixture, and resident-world block updates remain pending.

## Disk replacement and postprocessing

Disk features now decode bounded radii and half-heights, traverse circular
columns in native order, apply optional providers, and mark up to two non-air
blocks above each replacement run. Failed target predicates end a run; empty
optional providers preserve it. Postprocessing requests resolve the containing
resident chunk and retain duplicate marks and prior effects after a failure.

The [disk comparison](../benchmarks/rust-disk-2026-10-06/scoped-parity.json)
matches **2,816,586 output lines per reference JAR** across **76,409 fixtures**.
Every candidate state and every possible above-state is covered, alongside
random providers, rejected writes, partial failures, wrapped coordinates and
composed placement. Chunk-resolution and native ProtoChunk mark calls are
traced; later chunk postprocessing execution is outside this comparison.
Workspace tests, all-target Clippy and all 149 Python tests pass.

## Block-blob placement

Block blobs descend until their support predicate accepts the block below, then
write three overlapping randomized shapes with flags 3. Radius calculations
retain native float precision while position distances use double precision;
even the last shape consumes its three origin-offset draws.

The [block-blob comparison](../benchmarks/rust-block-blob-2026-10-06/scoped-parity.json)
matches **2,387,914 lines per reference JAR** over **77,686 fixtures**, including
every candidate/supporting state, repeated calls, early returns, failed queries,
rejected writes, partial failures and overlay/modifier composition. Workspace
tests and all-target Clippy pass. Rust bounds pathological traversal work;
resident-world update callbacks and full gameplay remain pending.

## Fluid-state JSON codecs

Fluid states now decode and encode native object-form identifiers and properties.
Missing properties retain the pinned defaults (`falling=true`, flowing level 1);
invalid property values are ignored, while unknown fluid identifiers fail.
Integer levels share the Java-compatible decimal parser with block properties.

The expanded [fluid comparison](../benchmarks/rust-fluid-2026-10-06/scoped-parity.json)
matches **243,331 lines per reference JAR**, including all 37 fluid states,
2,432 codec fixtures, encoding, legacy block conversion, and the existing
contained-fluid/height tests. Workspace tests and all-target Clippy pass.
Fluid spreading, tick execution and entity interaction remain pending.

## Spring features and fluid tick requests

Spring features preserve above/below/origin gating, the separate west/east/north/
south/below rock and air-count passes, fluid-to-block conversion, writes with
flags 2, and immediate fluid tick requests after accepted or rejected writes.
Fluid tick execution is still separate simulation work.

The [spring comparison](../benchmarks/rust-spring-2026-10-06/scoped-parity.json)
matches **3,649,087 lines per reference JAR** across **172,141 fixtures**. It
covers every state in the gating and neighbor roles, all five-neighbor air/rock/
other combinations and count pairs, all 37 fluid states, malformed fields, tags,
wrapped positions, partial failures and nested composition. Workspace tests and
all-target Clippy pass. Full fluid spreading and world updates remain pending.

## Vine feature attachment

Vine features require an air state at the origin and choose the first acceptable
attachment in Up/North/South/West/East order. They evaluate actual support and
collision shapes and return success after the write request, including when the
world rejects that write. Feature and world random streams are unchanged.

The [vine comparison](../benchmarks/rust-vines-2026-10-06/scoped-parity.json)
matches **4,315,682 lines per reference JAR** across **224,743 fixtures**, covering
every origin state and every state on each candidate face, support combinations,
wrapped positions, partial failures and composed placement. Workspace tests and
all-target Clippy pass. Vine growth, block updates and complete gameplay remain
pending; this fixture uses absent dynamic block entities.

## Full-face occlusion

Block property bindings now cache six full-face occlusion results per state,
sharing calculations for identical imported shapes. These results use native
occlusion rules independently of collision or support geometry. Unknown state
IDs produce an error.

The expanded [block-property comparison](../benchmarks/rust-block-properties-2026-10-06/scoped-parity.json)
matches **197,180 lines per reference JAR**, including all 35,723 states and
18,096 synthetic shape/flag combinations. Workspace tests and all-target Clippy
pass. Lighting and renderer integration remain separate work.

## Underwater magma and probability decoding

Underwater magma now performs native water-column scans, including the upper
scan before the lower scan and repeated boundary reads. Each cube position draws
a probability value before testing block identity and the five required full
occlusion faces. Rejected writes still count as successful placements.

The [underwater magma comparison](../benchmarks/rust-underwater-magma-2026-10-06/scoped-parity.json)
matches **6,523,072 lines per reference JAR** over **253,252 fixtures**. Both RNG
modes cover the complete radius-64 cube, consuming **2,146,689 draws** with zero
placement probability. Every state in the origin/floor/neighbor roles, scan
limits, partial failures, wrapped radius-zero columns and composition are tested.

The native comparison exposed signed-zero codec behavior: bounded probabilities
reject `-0.0`, including negative values that round to float zero. Magma and
pillar decoding now preserve this. The expanded pillar gate passes **921,111
lines per JAR** over **41,746 fixtures**. Workspace tests and all-target Clippy
pass. World updates and magma/fluid gameplay remain pending.

## Blue ice generation

Blue ice features now preserve world sea-level gating, water and packed-ice
seeding checks, all 200 spreading attempts, signed vertical radius adjustment,
random draw order and six-direction adjacency. World sea level can differ from
the generation context's sea level. Rejected writes preserve native success and
subsequent attempts observe the actual mutable world.

The [blue-ice comparison](../benchmarks/rust-blue-ice-2026-10-06/scoped-parity.json)
matches **8,954,666 lines per reference JAR** across **79,944 fixtures**, covering
every origin/candidate state through failure-terminated prefixes, full spreading
runs, all packed-ice neighbor masks, extreme sea levels, wrapped positions,
partial failures and composition. Workspace tests, all-target Clippy and all
149 Python tests pass. Ice gameplay and complete world generation remain pending.

## Spike geometry and roots

Spike features now implement descending air search, support predicates, randomized
height and radius, rare elevated spikes, float taper calculations and independent
upper/lower replacement tests. Roots retain the native fixed Y=50 boundary,
corner-specific random lengths, gaps, and exact-state replacement exception.

The [spike comparison](../benchmarks/rust-spike-2026-10-06/scoped-parity.json)
matches **8,859,761 lines per reference JAR** over **113,127 fixtures**. Every
output/origin/candidate state is covered by failure-terminated prefixes; complete
runs include 1,024 seeds per RNG, replacement and support predicates, repeated
mutations, rejected writes, partial failures and composition. Workspace tests
and all-target Clippy pass. Complete world generation and block updates remain
pending. Pathological descent/root work is bounded to 2^20 visits.

## End exit podium construction

End podium features reproduce active/inactive construction, the native distance
cutoffs and box traversal, central bedrock pillar and ordered wall-torch facing.
Active replacement reads block identity, requests destruction with drops and no
responsible entity, then writes the replacement even if destruction was rejected.
Matching blocks skip both requests.

The [End podium comparison](../benchmarks/rust-end-podium-2026-10-06/scoped-parity.json)
matches **3,309,219 lines per reference JAR** over **38,207 fixtures**. All states
reach active replacement prefixes; full runs cover representative blocks, repeated
construction, rejected destruction/writes, partial failures, composition and
nonoverflow extreme coordinates. Workspace tests, all-target Clippy and all
149 Python tests pass. The fixture controls destruction success and air
replacement; loot, block entities, updates, portal travel and dragon gameplay
remain pending. Pathological overflow-box work is bounded to 2^20 visits.

## Replacement blobs

The `netherrack_replace_blobs` feature now clamps the search origin to world
height, descends to a matching block, samples three independent radii and visits
clipped Manhattan shells in native order. Target matching uses block identity
regardless of state properties; rejected writes still produce success.

The [replacement-blob comparison](../benchmarks/rust-replace-blobs-2026-10-06/scoped-parity.json)
matches **5,538,034 lines per reference JAR** over **109,003 fixtures**, including
every target/output state, all candidate states through failure-terminated
prefixes, every constant radius, six provider families, both RNGs, height
boundaries, wrapped X/Z coordinates, partial failures and composition. Workspace
tests and all-target Clippy pass. World updates and full generation remain
pending; pathological descent is bounded to 2^20 visits.

## Bamboo and podzol patches

Bamboo features preserve the initial air gate, real survival checks and native
attempt-success behavior when survival fails. Podzol patches use ordered surface
height requests and the native replacement tag. Stalks preserve random draws and
leaf states, including the first cap overwriting the stopping position when at
least three stalk positions were attempted.

The [bamboo comparison](../benchmarks/rust-bamboo-2026-10-06/scoped-parity.json)
matches **3,485,027 lines per reference JAR** over **144,452 fixtures**. Every
origin/support/podzol state has a failure-terminated prefix; every obstacle state
has a complete no-podzol run. Tests include both RNGs, signed-zero decoding,
obstruction heights, rejected writes, repeated mutations, wrapped coordinates
and composition. Workspace tests and all-target Clippy pass. Fixture surface
heights are controlled; dynamic height updates, bamboo growth and full gameplay
remain pending. Pathological wrapped patch loops have a 2^20 work limit.

## Nether delta patches

Delta features now preserve the double-precision rim chance, conditional rim
offset draws, independent patch radius draws and Manhattan traversal. Placement
checks block identity and ordered air/nonair neighbors, writes the rim, then
checks shifted contents against the mutated world. A zero rim component still
shifts contents even when it disables the rim write.

The [delta comparison](../benchmarks/rust-delta-2026-10-06/scoped-parity.json)
matches **13,418,994 lines per reference JAR** across **324,269 fixtures**,
covering every origin/content/rim state and all six neighbor roles, every air
mask, radius limits, six provider families, both RNGs, repeated mutations,
rejected writes, partial failures, wrapped positions and composition. Workspace
tests, all-target Clippy and all 149 Python tests pass. The fixture supplies a
flat ground/air boundary with mutable overrides; fluid behavior, world updates
and complete generation remain pending.

## Biome freezing and snow eligibility

Registry-bound weather predicates now use the native biome temperature cache,
precipitation selection, build-height and block-light gates. Freezing distinguishes
source water from flowing water, requires a liquid block and optionally checks
water neighbors in west/east/north/south order. Snow accepts air or existing snow
and evaluates the default snow state's actual survival rules.

The [biome-weather comparison](../benchmarks/rust-biome-weather-2026-10-06/scoped-parity.json)
matches **4,071,594 lines per reference JAR** across **326,057 fixtures**,
covering all states in block/fluid/snow/support/neighbor roles, neighbor masks,
float thresholds and nonfinite temperatures, both temperature modifiers, light
and height boundaries, precipitation flags, repeated queries and partial failures.
Workspace tests and all-target Clippy pass. Native fixture temperature caches
are removed after each independent case to avoid retaining hundreds of thousands
of biomes; repeated queries within a case keep their cache. Dynamic block entities
are absent. Actual weather ticks, lighting propagation and world updates remain
pending.

## Frozen top layers and snowy ground

The `freeze_top_layer` feature now visits its 16×16 footprint in native column
order, queries motion-blocking height and retains the selected biome for both
weather checks. Ice is attempted before snow eligibility. Snow attempts are
followed by snowy ground-property updates even if the snow write was rejected.

The [snow/freezing feature comparison](../benchmarks/rust-snow-freeze-2026-10-06/scoped-parity.json)
matches **7,764,082 lines per reference JAR** over **108,339 fixtures**. Every
ground/candidate/fluid state has a failure-terminated prefix; full runs cover
climate thresholds and NaN, both modifiers, precipitation, light gates, repeated
mutations, rejected writes, partial failures, extreme heights/sea levels, wrapped
coordinates and composition. Workspace tests and all-target Clippy pass. The
fixture uses controlled heights and one biome; lighting propagation, dynamic
biome selection, world updates and full generation remain pending.

## Generation-stage tick containers

Proto-chunk tick containers now keep the first type/position request in insertion
order. Newly scheduled ticks retain priority with zero delay; loaded entries
retain their saved delay. Generation tick access routes requests to the selected
container and reports no currently executing ticks or aggregate count. Snapshots
can be promoted into the existing level tick queue.

The [generation-tick comparison](../benchmarks/rust-generation-ticks-2026-10-06/scoped-parity.json)
matches **101,784 lines per reference JAR**, covering duplicates, saved entries,
snapshot independence, mixed proto/level routing, lookup failures, promotion,
priority boundaries and integer/time extremes. Workspace tests, all-target Clippy
and all 149 Python tests pass. Rust exposes packed entries as an immutable
borrowed slice; mutating Java's returned list is outside this API. Feature-world
scheduling integration and block/fluid tick execution remain pending.

## Resident generation tick queues

Resident terrain chunks now own separate block and fluid proto-tick queues.
Feature-world scheduling normalizes block states to block identity, routes to the
chunk containing the original position and advances the region's tick order even
for duplicate requests or missing chunks. Time and order use Java wrapping rules.

The [resident queue comparison](../benchmarks/rust-feature-region-ticks-2026-10-06/scoped-parity.json)
matches **28,985,883 lines per reference JAR** across **192 fixtures**, with nine
resident chunks each, **2,688 requested feature placements** and **14,784 direct
tick requests**. Both references agree on final sections, heightmaps and saved
ticks. Coverage includes spring scheduling, duplicate block identities, separate
fluid identities, missing chunks, out-of-height positions and time/order overflow.
Workspace tests and all-target Clippy pass. This connects generation-stage queues;
block/fluid tick execution, complete chunk persistence and full world-generation
effects remain pending.

## Resident block and fluid queries

Feature regions can bind an immutable snapshot of resolved block/fluid tags and
contained fluid states. The snapshot shares compiled tables with configured
features and rejects bindings from a different block registry. Feature access
forwards fluid, water-tag and block-tag queries against current resident storage.
Missing tags have no members; missing chunks remain query errors.

The [resident query comparison](../benchmarks/rust-feature-region-queries-2026-10-06/scoped-parity.json)
matches **29,312,766 lines per reference JAR**, retaining the 192 multi-chunk
feature/tick fixtures and adding insertion/removal probes for **all 35,723 block
states**. Each probe checks fluid identity, amount, falling/source/full flags,
water membership and all 300 bundled block tags plus an absent tag. Negative
coordinates, chunk edges, outside-height reads and missing chunks are included.
Workspace tests and all-target Clippy pass; a separate regression covers foreign
registries, binding lifetime and a custom water tag. These queries do not execute
fluid spreading, block behavior or lighting.

## End gateway structures and exit configuration

The `end_gateway` feature now places its air, bedrock and gateway blocks in native
box order. A configured exit triggers a block-entity lookup even when the gateway
write was rejected. Missing or unrelated entities are no-ops; a gateway receives
its exit/exact fields before its dirty notification.

The [gateway comparison](../benchmarks/rust-end-gateway-2026-10-06/scoped-parity.json)
matches **754,877 lines per reference JAR** across **5,820 fixtures**, including
every world-call failure boundary, rejected writes, repeated updates, both random
streams, placement composition, codec edge cases and bounded prefixes of wrapped
coordinate boxes. The reference invokes the actual gateway entity setter and
compares its fields after partial failures. Workspace tests, all-target Clippy and
all 149 Python tests pass. The Python checks also caught and repaired a shared
fixture dependency: decoration retains its six-feature corpus independently of
region spring/tick/query probes; regenerated inputs are byte-identical to their
validated corpora. Gateway entity creation/removal, dirty propagation, exit search
and teleportation remain pending.

## Bonus chest placement and loot assignment

Bonus chest generation now independently shuffles the chunk's X and Z columns,
searches motion-blocking-no-leaves heights and checks air or empty collision.
After a chest write it looks up a randomizable container, consuming a loot seed
only when one exists, then checks torch survival in native horizontal order.
Rejected writes retain the subsequent entity and torch operations.

The [bonus-chest comparison](../benchmarks/rust-bonus-chest-2026-10-06/scoped-parity.json)
matches **1,505,542 lines per reference JAR** across **42,396 fixtures**. All
35,723 states participate in collision prefixes; complete searches cover both
random streams, missing/unrelated/chest entities, sole viable columns, exhausted
searches, repeated mutation, rejected writes, partial failures, extreme positions
and placement composition. Actual native chest loot-table and seed fields are
compared. Workspace tests and all-target Clippy pass. Heights and entity existence
are controlled by the fixture; inventory contents, loot generation, automatic
entity lifecycle and complete spawn generation remain pending.

## Recursive chorus generation

Chorus generation now checks its air origin and tagged support, builds and
refreshes connection states, branches recursively within the native depth and
distance bounds, and places age-five terminal flowers. Clearance checks exclude
the parent branch, and rejected writes retain native subsequent reads and draws.

The [chorus comparison](../benchmarks/rust-chorus-plant-2026-10-06/scoped-parity.json)
matches **8,083,367 lines per reference JAR** across **254,296 fixtures**. Every
state participates in origin/support gates and five neighbor-connection prefixes.
Full growth covers 1,024 seeds per random generator, early failure boundaries,
rejected writes, repeated mutation, random obstacles, wrapped coordinates and
feature composition. Workspace tests and all-target Clippy pass. Neighbor-update
execution, random tick growth, drops, rendering and complete End generation remain
pending.

## Small pointed formations

The `speleothem` feature now chooses its growth direction from ceiling/floor
blocks, spreads a base patch, selects one- or two-block height, rechecks its base
and writes waterlogged tip/frustum states. Configured base and pointed states
select block identities; placement uses their defaults. Unsupported pointed-block
properties fail during placement after the preceding patch operations.

The [speleothem comparison](../benchmarks/rust-speleothem-2026-10-06/scoped-parity.json)
matches **2,752,289 lines per reference JAR** across **145,473 fixtures**, including
all states in support, taller-growth and contained-fluid roles; optional float
defaults and boundaries; both random streams; rejected writes; partial failures;
repeated mutation; wrapped positions; and composition. Workspace tests and
all-target Clippy pass. Neighbor updates, falling behavior, fluid simulation,
larger clusters and complete cave generation remain pending.

## Generation-time destruction on resident chunks

Feature access now routes destruction through its supplied write callback using
air and update flags `3`. Air is an immediate false result. This follows
`WorldGenRegion.destroyBlock`, whose generation-time path does not evaluate drops;
live-level destruction remains separate work.

The [resident destruction comparison](../benchmarks/rust-feature-region-destroy-2026-10-06/scoped-parity.json)
matches **29,502,523 lines per reference JAR** across **192 fixtures**, adding
active/inactive End podiums to the resident feature corpus. The reference invokes
the actual WorldGenRegion method through a subclass that transports reads/writes
to the existing chunk fixture. Final sections, heightmaps, ticks, tag/fluid probes,
rejected writes and partial failures still agree. Workspace tests, all-target
Clippy and all 149 Python tests pass. Write-zone policy, POIs, block entities,
postprocessing execution, lighting and live-world effects remain pending.

## Generation write radii

Feature access can now apply the generation step's horizontal write radius before
calling the mutation policy. Ordinary writes and destruction share the guard;
queries and tick scheduling retain their independent routing. The predicate uses
Java wrapping subtraction and absolute value, including integer-minimum cases.

The [write-zone comparison](../benchmarks/rust-feature-region-write-zone-2026-10-06/scoped-parity.json)
matches **29,231,123 lines per reference JAR**. It adds **20,125 direct zone cases**
for both chunk and block coordinates, then exercises unrestricted, zero-radius,
shifted-center and negative-radius policies across the 192 resident fixtures.
Vanilla's actual public/private write-zone methods provide the reference. Workspace
tests, all-target Clippy and both focused fixture tests pass. The owning world
still supplies generation-step metadata and upgrading-world vertical restrictions;
full WorldGenRegion effects remain pending.

## Proto-chunk block-entity storage

Rust now stores pending NBT and caller-owned active block-entity payloads by full
block position. Active insertion removes pending data, later pending insertion
cannot overwrite an active entry, and removal clears both maps. Saving uses the
active serializer when present and otherwise returns a pending snapshot. Changing
pending coordinate fields retains the existing map key. Coordinate resolution
matches NumericTag integer narrowing and floating-point flooring before saturating
conversion, then relocates mismatched X/Z into the owning chunk.

The [block-entity storage comparison](../benchmarks/rust-block-entity-storage-2026-10-06/scoped-parity.json)
matches **78,251 lines per reference JAR** across **28,511 commands**, using actual
native ProtoChunks and chest entities. Cases cover replacement, removal, position
unions, save selection, missing/nonnumeric coordinates, infinities/NaNs, integer
extremes and positions that alias when packed. Four focused Rust tests and
all-target Clippy pass. Collection order is normalized; Rust exposes borrowed or
owned data instead of unrestricted Java aliases. Entity construction from NBT,
complete serializers, active entity behavior and resident chunk lifecycle
integration remain pending.

## Upgrading-world write heights

Feature access can now enforce the upgrading center chunk's generation height
before invoking its write callback. Below-zero proto-chunk upgrades use Y = -64
through -1, inclusive. Ordinary generation has no vertical policy gate; storage
bounds remain separate. Reads and tick scheduling keep their existing behavior,
and generation-time destruction shares the write guard.

The [upgrade-height comparison](../benchmarks/rust-feature-region-upgrade-height-2026-10-06/scoped-parity.json)
matches **29,247,433 lines per reference JAR**, including **10,098 height cases**,
20,125 horizontal zone cases and all 192 resident fixtures. The reference calls
actual `WorldGenRegion.ensureCanWrite` with a native ProtoChunk before and after
installing below-zero upgrade data. Direct write probes cover boundary and extreme
heights, ordinary/upgrading modes and horizontal rejection. Five focused Rust
region tests, all-target Clippy and both Python fixture tests passed for that
checkpoint. Subsequent work below adds resident upgrade metadata, cleanup,
bedrock transformations and live center-chunk write policy. Automatic upgrade
activation, biome retention and the complete generation lifecycle remain pending.

## Chunk-status identities and metadata

Rust now represents all ten pinned chunk statuses, their ordering and parents,
proto/level chunk classification, structure-distance constant, and selected
heightmaps. Existing pre-light chunks use these definitions for their names and
heightmap groups. Direct name lookup retains vanilla's `empty` fallback; NBT
codec lookup rejects unknown names and writes canonical namespaced identifiers.

The [chunk-status comparison](../benchmarks/rust-chunk-status-2026-10-06/scoped-parity.json)
matches **4,232 lines per reference JAR** across all 100 ordered status pairs and
**2,065 codec inputs**. Inputs include qualified/unqualified names, empty
namespaces, obsolete and malformed names, random strings and nonstring NBT.
World-crate tests and all-target Clippy pass. Codec error text is normalized.
Status identities do not execute generation tasks: scheduling, lighting,
post-light lifecycle transitions and the complete gameplay runtime remain pending.

## Below-zero upgrade data and resident transformations

Resident chunks now hold optional below-zero upgrade data. Installation alone
keeps the data even if the current stage already exceeds its target; assigning a
persisted stage at or beyond that target clears it. Upgrading chunks expose the
-64 through -1 generation interval independently of their storage bounds.

The Rust upgrade codec validates nonempty target statuses, accepts vanilla's
lenient optional bedrock-hole mask, normalizes trailing zero words and omits an
empty mask when saving. Hole queries wrap local X/Z coordinates. Resident
transformations replace bedrock in the old Y = 0 through 4 band with default
deepslate and clear marked columns through the chunk's current generation height.

The [upgrade-data comparison](../benchmarks/rust-retrogen-data-2026-10-06/scoped-parity.json)
matches **20,626 lines per reference JAR** across **5,114 codec inputs**, including
canonical output, all 256 column bits, out-of-footprint bits, signed numeric
streams, malformed optional fields and random masks. Codec error text is normalized.
The [resident upgrade comparison](../benchmarks/rust-retrogen-chunk-2026-10-06/scoped-parity.json)
matches **3,028,057 lines per reference JAR** across **138 fixtures**. It checks all
nine nonempty targets against the six pre-light stages, repeated/backward stage
assignments, complete stored sections and heightmap words after replacement and
masking, partial vertical overlap, empty sections and changed generation bounds.
World-crate tests and all-target Clippy pass. Save-dirty notifications, post-light
transitions, biome retention, structure restrictions, automatic generation
scheduling and full saved-world/gameplay integration remain pending.

## Live upgrade permissions and retained biome status

Feature access can bind a resident center chunk and read its upgrade state before
every write. Successful callbacks that complete, remove or reinstall an upgrade
affect the next write through the same access object. The destination chunk's
upgrade state does not select this region-wide height gate. Explicit fixed-height
policies remain available for callers that need a supplied interval.

Resident chunks also expose vanilla's highest generated status: the later of
their current persisted stage and retained upgrade target. Region biome reads use
this status, allowing access to old biomes while below-zero regeneration is still
at an earlier stage. Removing upgrade metadata restores the persisted-stage gate.

The [live region comparison](../benchmarks/rust-live-upgrade-region-2026-10-06/scoped-parity.json)
matches **29,248,297 lines per reference JAR**, extending the 192 resident fixtures
with **54 lifecycle cases**. It compares native write permission, highest status
and actual ProtoChunk biome-read availability across all six pre-light stages and
nine nonempty upgrade targets. The [retained-status comparison](../benchmarks/rust-retained-upgrade-status-2026-10-06/scoped-parity.json)
also matches **3,028,057 lines per JAR** across 138 upgrade fixtures. Seven focused
region tests, all-target Clippy and the region corpus test pass. These checks do
not yet cover the biome resolver's cave-biome retention, structure restrictions,
automatic scheduling or complete saved-world integration.

## Retained biomes during below-zero generation

The [biome-retention implementation](biome-lookup.md#below-zero-biome-retention)
now preserves the pinned three cave-biome candidates and otherwise reads the old
column at quart Y = 0. The resolver captures upgrade activation at construction;
candidate failures precede fallback reads. Section fills preserve native commit
ordering and partial-failure behavior while allowing access to old biome storage.

The [retention/fill comparison](../benchmarks/rust-retrogen-biome-fill-2026-10-06/scoped-parity.json)
matches **593,152 lines per JAR** across 740 fills and 2,800 direct queries. The
[generation integration](../benchmarks/rust-retrogen-biome-generation-2026-10-06/scoped-parity.json)
matches **834,080 lines per JAR** across 396 fixtures and 610 fills, including 144
upgrading chunks alongside the ordinary-generation cases. It executes actual
native `ChunkGenerator.doCreateBiomes`, including retained-status old biome reads,
native below-zero generation bounds, repeated fills and all biome-source kinds.
This closes the resolver gap described in the preceding checkpoint. Nonempty
blending, structure restrictions, automatic generation scheduling and complete
saved-world/gameplay integration remain pending.

## Active status-effect storage and notifications

Rust now composes shared status instances with native-order map storage for
entity effect addition, forced replacement, removal, clearing and blend lookup.
It preserves eligibility checks, source identity, hidden-chain updates, blend
transfer, callback order, and mutations made while callbacks are running. Clearing
copies the map before removing live entries and reports effects in the copied
map's iteration order.

The [active-effect comparison](../benchmarks/rust-active-effects-2026-10-07/scoped-parity.json)
matches **535,063 lines per reference JAR** across **1,266 fixtures**, using actual
vanilla entity storage methods, status instances and JDK maps. Native holder hash
and identity-hash values are captured separately for each reference process.
World-library and fixture Clippy checks pass. Eligibility policy, attribute and
packet notifications, and registered effect behavior remain controlled in these
checks. Consumable integration, the entity effect tick loop and complete gameplay
remain pending; this is scoped evidence, not full-game parity.

## Consumable effects bound to entity storage

`StoredConsume` now binds concrete apply/remove/clear consume effects to the
entity's shared active-effect map. Consumable additions use a null source and
ordinary status-instance copies, then execute the entity's eligibility, update
and notification sequence. Sound and RNG services receive access to live storage
so callback mutations affect subsequent operations.

The [combined comparison](../benchmarks/rust-stored-consume-2026-10-07/scoped-parity.json)
matches **1,214,124 lines per reference JAR**, executing actual native consume
effects and inherited entity storage together. It covers both random generators,
probability boundaries, duplicate inputs, hidden chains, colliding holder keys,
map-copy removal order, callback mutations and failure prefixes. World-library
and fixture Clippy checks pass. Eligibility policy, registered effect behavior,
attribute/packet notifications and sound delivery remain controlled. Named
holder-set binding, the full player consumption lifecycle and complete gameplay
remain pending.

## Entity effect tick loop and client particles

The Rust entity effect loop now iterates shared active storage, removes expired
effects before notifying observers, reports hidden-effect restoration, and
refreshes surviving effects on vanilla's 600-tick duration boundary. It uses the
actual level class distinction. Server iteration catches concurrent-modification
failures from map iteration or callbacks; the initial level query and client
iteration preserve their propagating failure behavior. Value replacement alone
does not invalidate iteration, and callbacks continue using captured instances.

Client ticks advance each instance's duration and blend, then read synchronized
particle/ambience data, apply the invisibility-dependent random interval, and
select particles from the captured list. Position queries and the destination
level preserve native evaluation order.

The [effect-loop comparison](../benchmarks/rust-active-effect-tick-2026-10-07/scoped-parity.json)
matches **1,732,406 client-JAR lines** and **1,732,849 server-JAR lines** across
**4,036 fixtures**. Each reference exports its own holder identity hashes; those
hashes can change iteration order and subsequent callback paths between JVMs.
The checks execute inherited native entity and instance methods, native maps,
synchronized data and both RNG implementations. Library and fixture Clippy pass.
Registered effect bodies, attribute/packet notification side effects and random
position methods are still controlled. Complete entity ticking, gameplay and
playable platform builds remain pending.

## Pending-attribute set storage

Rust now implements the default fastutil object set used by vanilla attribute
update and synchronization queues. It preserves null membership, retained key
identity, native slot order, growth/shrink behavior and iterator removal across
wrapped clusters. Iterators read the live slot array and retain their original
remaining count; external mutations have native failure prefixes rather than
HashMap-style concurrent-modification checks.

The [set comparison](../benchmarks/rust-object-open-set-2026-10-07/scoped-parity.json)
matches **611,998 lines per reference classpath** against the pinned fastutil
8.5.18 library. Library and fixture Clippy pass. The gate covers default set
operations and live iteration; alternate constructors, bulk methods, and external
structural mutation followed by iterator removal remain outside its scope.
Binding this storage to attribute refresh and the complete entity lifecycle is
the next step; this collection comparison does not establish gameplay parity.

## Effect eligibility and entity notification bodies

Effect eligibility now follows vanilla's ordered immunity-tag branches. Entity
effect callbacks mark synchronization dirty, retain captured effect/map handles,
apply or remove modifiers, refresh pending attributes and construct passenger
update/removal packets in native order. New effect storage starts dirty, matching
the entity constructor. Attribute refresh reads its captured live fastutil set
and clears that same set only after every callback succeeds. Passenger lists
remain captured while later connection fields and packet data are read at their
native evaluation points.

The [notification comparison](../benchmarks/rust-effect-notifications-2026-10-07/scoped-parity.json)
matches **1,667,616 lines per reference JAR**, exercising inherited native
eligibility, notification, refresh and passenger methods together with effect
storage. The fixtures use actual status instances, pending sets, immutable
passenger lists and packet constructors. Library and fixture Clippy pass.
Modifier bodies, attribute-update consequences and packet transport are still
controlled; their full bindings and complete gameplay remain unfinished.

## Attribute-update consequences and health storage

Pending attribute refresh now dispatches native max-health, max-absorption,
scale and waypoint-range consequences. Lowering a health limit compares against
the first captured maximum, then uses the ordinary setter and its fresh limit
lookup. Health synchronization retains its selected data object through later
callbacks. Waypoint changes capture the server manager before reading the
actor's direct attribute-map field, and track only a strictly positive range.

The [combined attribute-update comparison](../benchmarks/rust-attribute-updates-2026-10-07/scoped-parity.json)
matches **3,111,429 lines per reference JAR**, including the notification corpus,
finite/nonfinite limits, signed zero, dirty health callbacks, map/manager changes
and failure prefixes. Actual native effect notifications, attribute-update
methods, health setters and synchronized Float storage execute together. Library
and fixture Clippy pass. Attribute values, dimension rebuilds and waypoint-manager
internals remain controlled. Registered modifier bodies, full player integration
and complete gameplay/platform builds are still unfinished.

## Effect-modifier map storage

The default fastutil object map now preserves native slot order, nullable keys
and values, growth/shrink behavior and live indexed entry handles. Entry
iterators invalidate their current handle on removal; older handles continue
reading the live arrays at their stored indexes. Callback traversal captures the
key array but reads values from the current array, including after a rehash.
Writes through a stale entry into an empty slot do not make that key present.

The [map comparison](../benchmarks/rust-object-open-map-2026-10-07/scoped-parity.json)
matches **1,142,630 lines per reference classpath** against pinned fastutil
8.5.18. Library and fixture Clippy pass. The gate includes collision clusters,
equal-key identity retention, entry mutation/invalidation and traversal callback
failure prefixes. Alternate constructors, custom default return values,
compute/merge/bulk APIs and external structural mutation combined with iterator
removal remain outside this scope. Effect-modifier and full gameplay integration
remain unfinished.

## Effect attribute templates and permanent modifiers

Effect modifier definitions now scale their amount with vanilla's wrapping
amplifier addition and create a fresh modifier identity on each application.
Application looks up the attribute instance, removes the current definition's
modifier, then reads the indexed definition again before creating and adding a
permanent modifier. This preserves callback changes between those operations.
Missing attribute instances skip template access, and failures retain preceding
map and attribute writes.

The [modifier comparison](../benchmarks/rust-effect-attribute-modifiers-2026-10-07/scoped-parity.json)
matches **1,421,730 lines per reference JAR** across **3,317 fixtures**. It executes
native effect modifier methods with real attribute instances, including cached
values, permanent membership, dirty observers, nonfinite arithmetic, identity,
map rehashes and callback failures. Library and fixture Clippy pass. The fixture
asserts actual holder hashes when constructing collision cases. Attribute-map
lookup and dirty callback receivers remain controlled. Registered effect
definitions, pending queue ownership, entity bindings and complete gameplay/
platform builds remain unfinished.

## Attribute-map creation and live queues

Effect modifiers now have a concrete attribute-map storage path with separate
instance lookup, update and synchronization collections. Lazy creation copies
the supplier before inserting the instance into the lookup map. Its dirty
callback adds the update entry before checking synchronization eligibility, so
a failed creation can leave an instance queued without a lookup entry. Later
lookups can create a second instance while the first remains queued. Successful
creation uses an ordinary map insertion after the callback, preserving callback
replacement, resize and alias behavior.

The [stored-attribute comparison](../benchmarks/rust-stored-effect-attributes-2026-10-07/scoped-parity.json)
matches **4,543,569 lines per reference JAR**. It combines actual native effect
modifier methods, attribute-map lookup, supplier creation, attribute-instance
replacement and permanent modifiers with real pending sets. Native instance
identity hashes are captured separately for each JVM and replayed in Rust.
Library and fixture Clippy pass. Basic-attribute default/synchronization methods
remain controlled callbacks; the fixture covers queue and lookup mutation but
not arbitrary recursive instance mutation. Registered definitions, entity
notification binding, bulk map methods and complete gameplay/platform builds
remain unfinished.

## Entity notifications bound to stored attributes

The stored-attribute adapter now connects effect modifier dispatch to a captured
attribute map, supplier creation and instance operations. Entity effect
notifications and pending refresh execute over the same concrete lookup and
update/synchronization sets, preserving queued instance identity after lookup
aliasing or removal. Refresh clears the pending update set only after every
attribute callback succeeds.

The [combined notification comparison](../benchmarks/rust-stored-effect-notifications-2026-10-07/scoped-parity.json)
matches **21,117,051 lines per reference JAR**. It includes the full previous
stored-attribute corpus plus inherited native entity notification and private
refresh methods, client guards, dirty state, optional reapplication, empty
removal and callback failure/queue mutation. Library and both affected fixture
Clippy checks pass. Native JVM instance hashes are replayed separately per
reference. Attribute-update consequences and basic-attribute default/syncability
methods remain controlled; passenger lists are empty here, with packet behavior
covered separately. Registered attribute/effect definitions, combined health
and dimension consequences, complete entity gameplay and platform builds remain
unfinished.

## Stored attribute queries and base reset

Attribute-map queries now select the existing instance or supplier prototype
without creating an instance. Computed values preserve the selected object's
cache, and modifier queries distinguish missing cached modifiers from supplier
lookup failures. Base reset checks supplier membership before looking up the
current instance, then retains that instance across the supplier base lookup.
Resetting a supported attribute with no instance leaves it uncreated.

The [query comparison](../benchmarks/rust-attribute-map-queries-2026-10-07/scoped-parity.json)
matches **34,531,715 lines per reference JAR**. It includes the full stored
notification corpus, actual native map query/reset methods, mutable supplier
prototypes, reference-sharing modifiers, alias/null lookup entries, prototype
and instance cache state, nonfinite arithmetic and dirty callback failures.
Library and fixture Clippy pass. Present supplier prototypes are non-null;
custom virtual prototype methods, registered ranged-attribute integration and
combined health/dimension consequences remain outside this gate. Complete
gameplay and playable platform builds remain unfinished.

## Live registered attribute component

Registered entity attributes now have a reusable component combining immutable
catalog defaults with lazy instances, effect modifiers and native-order lookup,
update and synchronization collections. Instance handles retain their identity
independently of pending queue membership. Queries and base resets leave absent
instances uncreated; effect application uses the same stored instance and queues
as direct edits.

The [live attribute comparison](../benchmarks/rust-live-attributes-2026-10-07/scoped-parity.json)
matches **8,350,018 lines per reference JAR**. Client and server exports agree on
all **40 attributes and 93 entity suppliers**. The gate includes the previous
entity-attribute corpus, range boundaries, null lookups, modifier identities and
caches, nonfinite values, wrapping amplifiers and independent/bulk queue
acknowledgements. It replays each JVM's actual holder and instance hashes to
compare collection order. Library and fixture Clippy pass.

This component assumes immutable registered defaults with no modifiers; the
exporter verifies that assumption. Mutable custom suppliers and their internal
caches use the separate generic storage interfaces. Combined entity health and
dimension consequences, full gameplay, save/network codecs and the playable native
Linux build remain unfinished.

The production health binding now accepts the live component through computed
attribute queries. Healing, health and base absorption limits, synchronized
health storage and hunger regeneration therefore share its stored values and
modifier engine. The [health integration comparison](../benchmarks/rust-live-attribute-health-2026-10-07/scoped-parity.json)
matches **666,082 lines per reference JAR for each storage backend**, including
all 93 entity suppliers and observer failure prefixes. All `horizon-world`
targets compile, and library/affected fixture Clippy checks pass. This borrowed
binding keeps its selected attribute map fixed; entity virtual dispatch,
attribute refresh, player absorption synchronization and starvation damage
delivery still require their complete runtime integration.

## Entity refresh over live registered attributes

The live attribute bridge now connects virtual entity queries and pending
attribute refresh to registered storage and attribute-update dispatch. Refresh
retains its selected map through callbacks and clears that map's pending set
only on success. Health and absorption consequences read actual computed limits;
waypoint updates retain their manager before reading the direct attribute-map
field. Scale changes dispatch dimension refresh.

The [combined live update comparison](../benchmarks/rust-live-attribute-updates-2026-10-07/scoped-parity.json)
matches **2,836,190 client-reference lines and 2,836,234 server-reference lines**.
It executes actual native notifications, modifier loops, registered attribute
queries, private refresh, health/absorption update bodies and synchronized Float
storage. Cases cover all registered attributes/default suppliers, selected and
captured map/data handles, callbacks changing limits or queues, null pending
members, client guards, empty removals and failures late in large refreshes.
Each reference uses its own JVM identity hashes, so callback-mutation paths can
produce different trace lengths between runs. Library and fixture Clippy pass.

Dimension rebuilding and waypoint-manager internals remain controlled, and
passenger lists are empty in this combined fixture. Full active-effect lifecycle
and particle/invisibility synchronization, player absorption storage and complete
gameplay/platform integration remain unfinished.

## Registered effect presentation synchronization

Effect synchronization now builds the visible particle list in active-map order,
updates invisibility and ambient state, and preserves the server/client glowing
branches and shared flags. It retains each selected synchronized-data object and
clears effect dirtiness only after both visibility and glowing updates succeed.
Removing the particle list leaves the ambient field unchanged, matching vanilla.

The registry exporter verifies all **40 effect factories**: **34** create fresh
colored options, while **six** return constant simple particles. Colored options
use native object identity, so equal colors in newly created lists still trigger
updates. Constant options retain identity. Synchronized storage now supports
shared list values as well as scalar fields, preserving callback and dirty-flag
ordering.

The [effect synchronization comparison](../benchmarks/rust-effect-sync-2026-10-07/scoped-parity.json)
matches **2,754,744 lines per reference JAR**, including repeated updates,
ambient/visible combinations, callback map changes, captured data objects,
reentrant writes/packing and failure prefixes. The
[byte](../benchmarks/rust-effect-sync-2026-10-07/synced-bytes-regression.json) and
[float](../benchmarks/rust-effect-sync-2026-10-07/synced-floats-regression.json)
storage regressions also pass against both JARs. Library and affected fixture
Clippy checks pass, and all world targets compile.

The registered factories have no world access; the exporter verifies their
inherited implementation and captured arguments. This permits an immutable map
traversal during particle creation. Custom impure factories remain outside this
binding. Rendering, transport, full entity lifecycle integration and playable
Linux gameplay remain unfinished. In this pinned version,
`LivingEntity.updateDataBeforeSync` invokes dirty effect synchronization; it is a
separate phase from `tickEffects`.

## Scoreboards, teams and subtitles

`tools/test_scoreboard_parity.py` runs vanilla's own `Scoreboard` (what
`ServerScoreboard` extends) and `horizon_client::scoreboard` on 500 scripted
cases, about 81,000 operations: objectives with every kind of criteria (custom,
team kill, and statistics in every spelling `ObjectiveCriteria.byName` accepts,
which resolve to their canonical names), display names, render types,
auto-updated displays and number formats; scores set, added, read, locked,
given displays and formats, reset, and changed through `forAllObjectives` as the
gameplay criteria change them; display slots; teams with every option, joined,
left, removed and emptied by `Scoreboard.entityRemoved`. Every result must
match, as must each collection in the order vanilla iterates it (objectives,
holders and teams are fastutil open hash maps, each team's members a
`HashSet`; `fastutil_order` keeps the same tables), the
`ScoreboardSaveData.Packed` codec output that `data/minecraft/scoreboard.dat`
holds (fields equal to the codec defaults are left out, components are written
as `ComponentSerialization` writes them), and the data after each side loads
the other's NBT. Only each holder's scores, which vanilla keeps in an
identity-hash map, are compared as a set. `tools/test_save_roundtrip_parity.py`
also builds a scoreboard with every saved field on a vanilla server and requires
the game to save it back unchanged.

`tools/test_team_rules_parity.py` runs vanilla's `Entity.doTeamsAllowDamage`
(friendly fire), `isAlliedTo`, `EntitySelector.pushableBy` (collision rules),
`Entity.isInvisibleTo` (seeing invisible team mates) and
`LivingEntityRenderer.shouldShowName` (name tag visibility, with a stand-in
client) on stand-in entities over 6,000 combinations of two teams' options and
the entities' teams; the game uses the same functions. Mobs on the player's team
never target the player, a creeper's blast spares its team mates when friendly
fire is off, mobs push each other only as their collision rules allow, name tags
show the team's prefix, suffix and colour and the score of the objective
displayed below names, and a team that hides death messages keeps the player's
own death message out of chat. The criteria that follow the player (health,
food, armor, xp, level, air) update as `ServerPlayer.doTick` does, and deaths,
kills, team kills and statistics update their objectives.

`tools/test_subtitle_parity.py` runs vanilla's `SubtitleOverlay` with a
stand-in client on 1,500 scripted cases of sounds reported by the sound engine
(subtitles from sounds.json, positions, ranges, looping) at controlled times,
and frames that render it for a listener transform, the subtitle options (shown
or not, notification display time, text background opacity) and a GUI size.
Every frame's fills and texts, with their pose, must match the overlay in
`hud_overlays`: which captions show and in what order, merging of repeats,
expiry, the fade by age, the direction arrows and the box width. The overlay
listens to the sound engine from the frame that renders it with the option on
until one renders with it off, as vanilla's does.

Not covered yet: the player's air supply (Horizon does not track drowning, so
the `air` criteria stays at 300); displays auto-updated with the player's name
carry the team formatting but not the click, hover and insertion of
`Player.getDisplayName`; there is no attack sweep for allies to be spared from;
scores below other players' names need the multiplayer scoreboard packets,
which the client does not handle yet.

## Loom, crafter and recipe book

The loom, the crafter and the recipe book run on code checked against
vanilla's own classes, and the game uses that code:

- **Menus.** `horizon_client::menu` runs `LoomMenu` (selectable patterns,
  `clickMenuButton`, the six-layer limit, quick moves, the result's `onTake`)
  and the server's `CrafterMenu` over the block entity (the result following
  the grid, `setSlotState`, a filled disabled slot enabling itself) besides the
  other menus. `tools/test_menu_click_parity.py` compares 8,000 random click
  sequences over 19 menus, with pattern button and slot state packets, with
  vanilla's menu classes. The game's loom and crafter screens click through
  `Game::menu_session` on the same engine.
- **Crafter block.** `tools/test_crafter_parity.py` runs `CrafterBlock.dispenseFrom`
  and `CrafterBlockEntity.serverTick` over the block update oracle's level:
  4,000 crafts of datapack recipes (remainders and special recipes included)
  with disabled slots and containers, other crafters or air in front, comparing
  block changes, container contents, the animation ticks, triggered flag,
  comparator signal, shot items, level events and the level random.
- **Recipe placement.** `horizon_client::menu::place` ports `ServerPlaceRecipe`
  with `StackedContents`' recipe picker for the inventory, crafting table and
  furnaces. Vanilla counts the available items in a hash map ordered by
  identity hash codes, an order that changes between runs and decides which
  of several fitting items fills an ingredient; `tools/test_place_recipe_parity.py`
  hands the oracle's order to the Rust side and compares 6,000 placements.
- **The book's contents.** `recipe_book::ClientBook` keeps the displays as
  `ClientRecipeBook`'s `HashMap` of display ids does (on the verified Java
  `HashMap` port), whose order is the order of the buttons;
  `tools/test_recipe_book_view_parity.py` runs the client's `ClientRecipeBook`,
  `RecipeCollection` selection and the tab and page logic over 1,200 cases.
- **The player's book.** `recipe_book::Settings` with a `RecipeIndex` is
  `ServerRecipeBook`: awards (advancements, `/recipe`, crafting) never learn
  special recipes and count displays, and loading drops unknown recipes;
  `tools/test_recipe_book_parity.py` compares 1,500 operation sequences and the
  saved compound.
- **Screens.** `screen_layout` holds `LoomScreen.extractBackground` and
  `CrafterScreen`'s decorations as draw calls the game paints;
  `tools/test_screen_layout_parity.py` runs vanilla's screens on stand-in
  menus and compares 1,600 call lists.
- **The book's drawing.** `recipe_book_screen` lays out
  `RecipeBookComponent`'s background, search `EditBox`, tab buttons, filter
  `CycleButton`, the page of `RecipeButton`s with its number and arrows, the
  `OverlayRecipeComponent`, the tooltips and `GhostSlots` as GUI calls with
  their poses (squeeze animations, the overlay's 3/8 scale items); the game
  paints them. `tools/test_recipe_book_render_parity.py` builds vanilla's
  widgets for 600 states (`VanillaRecipeBookScreenOracle`) and compares every
  call. Ingredients cycle in their `HolderSet` order (tags in `TagLoader` order).
- **The book's text search.** `recipe_book_view` also searches: the oracle
  builds `SessionSearchTrees`' `FullTextSearchTree` over the tooltip lines and
  ids of every result a display can show, and `item_tooltip` writes those
  lines (attribute modifiers, smithing template, potion, dyed, pot, rocket and
  disc lines; the names of potions, templates and banner patterns). Displays
  with several results (map copies, tipped arrows, dyed armour) keep them in
  `Entry::variants`.
- **The ghost recipe.** `Book::ghost` is the one place that fills a recipe's
  ingredients into the grid or furnace slots (`PlaceRecipeHelper`'s centring,
  the first slots for shapeless recipes, every fuel for an empty fuel slot, in
  the registry's component-lookup order); `tools/test_ghost_recipe_parity.py`
  runs `fillGhostRecipe` against the real menus for all 2,698 recipe and menu
  pairs.
- **The loom's banner preview.** `item_render::gui::draw_banner_result` draws
  `GuiBannerResultRenderer`'s flag model and layers in the 20x40 picture in
  picture, lit with `ITEMS_FLAT`; the block entity render harness compares its
  quads under the picture-in-picture pose.

Not compared yet: furnaces awarding their recipes and experience when their
output is taken, and the limited crafting game rule. Tooltips in general lack
the lines `item_tooltip` writes for recipe results (the game's hover tooltips
do not show attribute modifiers yet).

`horizon_client::hotbar` reads the creative toolbars of an older `hotbar.nbt`
through the `HOTBAR` data fixers (`EmptyItemInHotbarFix`, every item stack
fixer): `tools/test_hotbar_parity.py` loads generated files of 34 versions with
the real `HotbarManager`. The creative screen's Saved Hotbars tab shows its
rows, and the save and load keys write and read it
(`game/creative_hotbar.rs`).

## Player list and world border rendering

`horizon_client::tab_list` lays out vanilla's `PlayerTabOverlay` as draw calls:
the listed players in `PLAYER_COMPARATOR` order (higher list order first,
spectators last, team name, profile name ignoring case), at most 80 of them in
columns of 20 rows; heads with the default skin for the UUID, flipped for
Dinnerbone and Grumm in the level, only in online mode; team prefix, suffix and
colour; italic, translucent spectator names (italic at the component root);
ping bars; the `list` objective as scores in their number format (the score's,
the objective's, or yellow) or as hearts with `HealthState`'s change blink and
the `hp` text when they no longer fit; and the header and footer wrapped by
`Font::split_lines`, which now follows `StringSplitter.splitLines`, with legacy
`§` codes applied by `Text::resolve_legacy`. `Hud.extractTabList`'s rule hides
the list in single player unless a `list` objective is set.
`tools/test_tab_list_parity.py` runs vanilla's own `extractRenderState` frame
after frame (`VanillaTabListOracle`: stand-in connection, level and options,
real `Scoreboard`) against `examples/probe/tab_list_fixture.rs` and compares every fill,
styled text run, sprite and face blit (as `hud_layout::GuiDraw` calls, painted
like the rest of the HUD): **1,000 cases, 3,476 frames and 844,758
calls match**. The game draws the same layout while the player-list key is
held, from the local player and scoreboard in single player and from
`PlayerInfoUpdate`, the tab list header and footer, `onlineMode` and the
objective, score, display-slot and team packets on a server (checked live
against the 26.3 server jar). Real skins are not downloaded: every head uses
its default skin. Horizon has no "chat background only" option, so the row
background is vanilla's default.

The client `WorldBorder` wraps `horizon_core::geometry::border` (checked by
`test_world_border_parity.py`) and `horizon_client::border_render` ports
`WorldBorderRenderer`: `extract`'s fade (`(1 - distance / render distance)^4`)
and status colour (growing, shrinking, stationary), the wall buffer that is only
rebuilt when the border moves, the model offset and the texture scroll of
`prepareDynamicTransforms`, and `draw`'s walls within the render distance,
nearest first. `WorldBorder::warning_strength` feeds `hud_overlays::vignette`
the border's warning strength at the camera. `tools/test_world_border_render_parity.py` drives vanilla's border,
renderer and vignette (`VanillaWorldBorderRenderOracle`, with a stand-in GPU
device, dynamic-uniform writer and render pass) against
`examples/probe/world_border_render_fixture.rs`: **1,500 cases and 16,859 frames match
bit for bit** (8,513 with the wall drawn). The game draws the wall with
`RenderPipelines.WORLD_BORDER`'s overlay blend and the mipmapped forcefield
texture, and follows the server's border packets. One border is kept for all
dimensions.

## Resource pack screen and chat input

`horizon_client::pack_screen` is `PackSelectionScreen` as a state machine: the
header, the two `TransferableSelectionList`s (rows, header entries, scroll,
selection outline, focus, the hover overlay and its arrows, incompatible packs'
tint and text), the search box filtering both lists, the footer buttons and
their tooltip, and the `ConfirmScreen` an incompatible pack opens, all as GUI
calls (`screen_draw`, which also ports `StringWidget`, `MultiLineTextWidget`,
`Button`, `EditBox` with its scrolling, and `StringSplitter`'s line splitting
with legacy formatting codes, whose split remainder takes the style found at
the break). `tools/test_pack_screen_parity.py` drives a real
`PackSelectionScreen` (`VanillaPackScreenOracle`) through 3,000 random sessions
of clicks, typing and the mouse wheel and compares every call.

`horizon_client::chat::ChatScreen` ports `ChatScreen`, its `EditBox` and
`CommandSuggestions`: argument colouring, usage lines and parse errors, the
suggestion list (sorting, hover, Tab and Shift+Tab, clicks), history and the
input's editing keys. `tools/test_chat_screen_parity.py` runs a real
`ChatScreen` (`VanillaChatScreenOracle`) on the client command tree an operator
receives (`Commands.sendCommands`, `ClientboundCommandsPacket`), whose custom
suggestions the server's dispatcher answers as
`handleCustomCommandSuggestions` does, over 3,000 sessions, and compares the
input's state and every call. Not compared: the mouse wheel over the
suggestion list and clipboard keys.

`horizon_client::creative` is the creative inventory. Its tabs come from
`creative-inventory.json`, which `VanillaCreativeTabsExporter` writes from
vanilla's own `CreativeModeTabs.tryRebuildTabContents` over the vanilla data
pack's registries: every stack with its components, and for each stack of the
search tab the creative tooltip lines and item tags that the client's search
trees index. The tabs are built with permissions, and the export marks the
stacks only that build lists (`permissions_only`): the operator tab and its
stacks at the end of the search tab, shown while the player
`canUseGameMasterBlocks` with the Operator Items Tab option on
(`CreativeModeInventoryScreen.hasPermissions`, checked on opening and every
tick as `tryRefreshInvalidatedTabs` does). The search ports `refreshSearchResults` and `SessionSearchTrees`
(substring matches over tooltip lines; after a colon the item id's namespace,
then its path or a line; tags after `#`), and the scrollbar follows
`mouseClicked`/`mouseDragged`/`mouseScrolled`. `tools/test_creative_parity.py`
compares every shown tab's stacks, the search tab and 6,793 searches (fixed
queries, then every stack's item path and tooltip lines and every tag, whole
and halved) with the vanilla client's own search trees
(`VanillaCreativeSearchOracle`), then again with permissions (13 tabs, 2,034
search stacks, 6,845 searches). Not compared: the saved hotbars tab.

## Pillager patrols and raids

`horizon_client::patrol` ports `PatrolSpawner` and `horizon_client::raid` ports
`Raid` and `Raids`; the game runs both through them. `tools/test_raid_parity.py`
runs vanilla's own classes in `VanillaRaidOracle`, on a stand-in `ServerLevel`
built from boxes of blocks and light, a biome, village sections, village points
of interest, unloaded and non-ticking chunks, the overworld clock, local
difficulty inputs and one player, and compares every case with
`raid_fixture` line by line. The last run matched **1,900 cases at zero
differences**: 1,000 patrol, 400 raid and 500 codec cases, with 474 patrol
pillagers, 1,364 raid waves, 11,399 raiders, 149 victories and 21 losses.

- **Patrols**: the countdown, `isBrightOutside`, the one in five draw, the
  random player, spectators, the village distance, `hasChunksAt`, the
  `can_pillager_patrol_spawn` attribute (mushroom fields and the early game
  timeline), the group size from the local difficulty, each member's
  `MOTION_BLOCKING_NO_LEAVES` height, empty spawn block and spawn rules, the
  leader's banner and patrol target, and the level random's next draw.
- **Raids**: `createOrExtendRaid` (spectators, the `raids` rule,
  `can_start_raid`, the mean of the village points of interest, the omen
  absorbed, the trigger stat), the pre-raid countdown and its cached spawn
  position, `findRandomSpawnPos` with Mth's sine table, wave composition by
  difficulty and the bonus wave, leaders and riders, `addWaveMob`,
  `removeFromRaid` and `updateBossbar` (progress bits with Java's `clamp`, the
  title, visibility, the player in the bar), the horn's seed and position,
  `updateRaiders` (far, gone, removed and idle raiders), joining, loaded and
  hurt raiders, victory with Hero of the Village, loss, the celebration,
  peaceful, the `raids` rule, unloaded chunks, `/raid setomen` and `stop`, and
  the 48,000 tick timeout.
- **raids.dat**: `Raids.CODEC` with `resultOrPartial` over generated and
  mutated data: numbers of any NBT type converted as Java does, partial
  results (`Util.fixedSize` streams, lists with bad elements, a non-list
  `raids`), raids dropped on missing fields, a fresh `Raids` when `next_id` or
  `tick` is missing, the heroes in `HashSet` order and the raids in the order
  of fastutil's entry-set stream (the tick iterates the map the other way).
  Each dimension keeps its own `dimensions/<id>/data/minecraft/raids.dat`.

Scope and remaining gaps: `finalizeSpawn` runs with a fixed random in the
oracle, so its own draws are not compared, and raid buffs and patrol targets
come from unseeded entity randoms. The omens act inside the effect tick
(`BadOmenMobEffect`, `RaidOmenMobEffect`), including the
`ConcurrentModificationException` vanilla swallows when Raid Omen is first
added; this has unit tests but no oracle, since the effect map's order follows
identity hashes. Raider goals (pathfinding to the raid, celebrating, fetching
the banner, villagers' raid activities) and ravager riding are not compared:
they belong to the mob AI harness. The game's village distance comes from the
points of interest its villagers claim rather than a `PoiManager`.

