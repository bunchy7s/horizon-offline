# Resource preparation and filesystem work

The reported preparation timings put sounds at 20–29 seconds with little CPU use, versus
0.9–1.4 seconds for fresh client asset extraction. Prefer fewer file opens, reads and writes
first. The exporter JVMs dominate a fresh preparation now; they and the asset copy run side by
side, and extraction and hashing are spread over threads that share one parsed jar
([launcher-preparation.md](launcher-preparation.md)).

Preparation now shares one parsed asset index between panorama, font, pack and language imports.
Its destination verification results live only for that preparation. A subsequent invocation
checks bytes again, so it can repair corrupt resources. Existing valid destination files do not
require the original asset objects to remain installed. A saved asset index also lets preparation
preserve full imports when only cached Minecraft JARs remain.

New imports are staged beside their destination. On Linux, FICLONE is attempted first; other
filesystems fall back to copying while hashing. Cloned copies are hashed before publication.
macOS attempts clonefile with the same checked streaming fallback. A whole asset group is staged and
verified before any replacement is published; corrupt late panorama faces leave placeholders
alone. Temporary files are cleaned up on errors. This is not a transaction for rename failures
or process termination during publication: a later run repairs incomplete groups.

Unchanged JAR resources are checked by the ZIP member's size and CRC, avoiding repeated
decompression. Valid indexed replacements take precedence over JAR placeholders. Reusing checked
Minecraft imports also passes the verified artifact/library results directly to the exporters.

## An immutable store across releases

The official launcher's SHA-1 object store already provides an immutable source for indexed
assets. Independent filesystem clones let installed Horizon releases share physical blocks
where supported, without sharing writable inodes. Ordinary copies remain the fallback.

A separate Horizon store could additionally share complete preparations on filesystems without
cloning. Its key would need the pinned client/server hashes, the preparer's output version (or
the legacy source/exporter fingerprint), and the actual asset-index digest: Mojang republishes
translations without changing the Minecraft release. Two preparations with different optional
asset availability must not claim the same complete snapshot.

Before adopting that store, separate immutable runtime resources from mutable exporter scratch,
classes and logs. Publish snapshots through a temporary directory and atomic rename, give each
release an explicit reference, and define locking, corruption repair, migration and garbage
collection. Exporters must never receive a shared writable directory or hard-linked files.
The existing launcher compatibility check should choose reuse; the store should only supply its
bytes. Keep this separate from the open work on output-version compatibility (#1946/#1947).

For now, retain independent release directories and clone their resource files. This reduces
copying where the filesystem supports it without introducing a second ownership/cleanup scheme.
