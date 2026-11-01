# Launcher resource preparation

Release folders include `exporters/server`, `exporters/client` and an integrity manifest.
These contain only Horizon's original Java exporter classes. Minecraft jars, libraries,
assets and generated resources remain local and are verified against `research-lock.json`.
The same platform-independent class bundle is used in game and dedicated-server releases.

The release runner compiles the server and client source sets separately with
`javac --release <pinned major> -proc:none -encoding UTF-8`, resolving shared helpers once
per classpath. It checks every class and its signatures with `-Xverify:all` against the
pinned jars. The release job also compares two fresh isolated preparations with a fresh
batched preparation using a runtime-only Java home:

```sh
cargo build --release -p horizon-prepare
# Requires already imported Minecraft files in .local/minecraft/<version>.
target/release/horizon-prepare compile-exporters
# Copy server/, client/ and manifest.json from the printed folder into exporters/.
python3 tools/check_prepare_exporters.py --binary target/release/horizon-prepare
```

Development checkouts and changed sources compile on demand, into
`.local/exporters/<sha256>`. The key covers Java sources (including implicit helpers),
pinned dependency checksums and compiler settings. Class checksums detect missing or damaged
outputs. Compilation happens in a private staging folder before publishing a complete cache
entry. Sources and paths are normalized so one release bundle works on all three platforms.

Compatible exporters run through one server JVM (two with 6 processors or more, below) and one
client JVM, each worker's exporters in a fixed order, the workers side by side. They keep
Minecraft's initialized built-in registries. Every task restores the original registry tags
before replying; failures stop preparation. Collision exports share the first generated
registry and collision files, encoded block states and shape grids within the server worker.
Ordinary parity oracle processes keep their existing behavior.

Command-registry and creative-tab exporters remain separate processes: they install data
components as well as tags. The data generator also remains separate. Inspection of pinned
`net.minecraft.data.Main.main` showed that it calls `Util.shutdownExecutors` after running
its providers. Calling it inside a worker would shut down shared executors; incorporating
its providers directly needs a separate comparison of registry and executor lifetimes.
Fresh release preparation therefore starts five exporter/data-generator JVMs, down from 18
(six with 6 processors or more, below), and invokes no compiler. `HORIZON_PREPARE_ISOLATED=1`
retains separate exporter JVMs for output comparisons and diagnosis.

Those JVMs run on four lanes at once, so at most three run at a time (five lanes and four JVMs
with the split below): the server worker's steps in their fixed order; the client worker's,
then the command registries'; the data generator, then the creative tabs'; and the Rust-only
copying (client assets and data, the world generator's bundle), spread over several threads.
The world generator's tables, which need the block facts and the reports, and the manifest
come after every lane. A worker stops when its lane is done. The two sides compile at once
too. Imports and checks hash the pinned files on several threads; every file is still checked
in full.

`HORIZON_PREPARE_JOBS` is how many threads each copying, hashing or compiling step spreads
over: the logical processors by default, at most 8. `1` also runs the lanes one after another
(for a machine short on memory); any other number runs them all at once. From 6, the world
generator's block behaviour and structure blocks, a quarter of the server lane, run on a fifth
lane in a second server JVM: it costs a second Bootstrap and ~400 MB, and only pays off when
the other lanes would leave processors idle. On a 6-core laptop they finished ~20 s before the
server lane; the split itself is not yet timed on such a machine (on 3 cores it is slower,
44.9 s against 40.5 s, so it stays off there). The release check prepares both ways.

Every JVM preparing starts (javac too, through `-J`) runs with `-XX:TieredStopAtLevel=1
-XX:+UseSerialGC`: short batch jobs side by side lose more CPU to the C2 compiler than it wins
back, and the serial collector keeps their heaps small (fresh: 105 s before, 63–66 s after,
same output). Inside the server worker the collision exporter skips its 62 MB verification
inputs, which only the parity harness reads, and the gameplay facts exporter streams its 56 MB
file: the worker peaks at about 480 MB instead of 1.25 GB, and a fresh preparation at about
1.06 GB for all its processes instead of 1.47 GB (about 400 MB more with the second server
JVM). Threads reading one jar share its parsed member list.

A repeat run starts no JVM when nothing changed: each exporter is skipped by the hashes of its
sources, its jar and its outputs. Most of those are in `horizon-resources.json`; the block
behaviour and structure-block exporters keep a `stamp.json` beside their working files in
`.local/` (the exporter bundle's key, the server jar, this preparer's sources, and the
outputs' sha256), so the prepared files stay the same.

`tools/check_prepare_exporters.py` requires identical resource file sets and identical bytes
for every file stable across the two isolated runs. For the three known noisy JSON files,
it also requires semantic equality after normalizing object key order, opaque JVM lambda
addresses and creative-tab tag set order. Other variation fails the release check.
It compares two batched runs with them, with 6 jobs and with 2 (six JVMs and five), then
repeats the second: that run must start no JVM and leave every file as it was.

On Linux with Java 25, the output comparisons used 23,204 resource files per run.
All runs used precompiled exporters:

| Comparison | Isolated A | Isolated B | Batched |
|---|---:|---:|---:|
| Initial | 178.95 s | 104.25 s | 45.05 s |
| Repeat | 100.03 s | 176.06 s | 120.09 s |
| Final, immutable collision cache | 133.80 s | 123.79 s | 56.19 s |

These use fresh output directories with a warm OS file cache, on a machine with other
workloads. The first two rows include output hashing; the final row measures preparation
before comparison. The variability prevents treating these as a controlled speedup estimate.
The compiler saving is separate: release preparation removes the previously measured
15 compiler invocations (29–52 s).
