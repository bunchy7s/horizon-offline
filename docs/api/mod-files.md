# Mod files

The rules a `.hmod` or `.hnmod` follows, in one place, for tools that make or read them. The readers are
`horizon_mods::Package` and `horizon_mods::Content` (content), and `horizon_mods::index::native::Bundle` (native
code). Level: content is preview, native code experimental ([data-formats.md](data-formats.md)).

## The zip

- A zip archive. Entries are stored or deflate-compressed, the only methods the readers support.
- A path is relative and `/`-separated: no `\`, no `:`, no empty, `.` or `..` part, and it doesn't start with `/`.
  Directory entries are skipped. A native entry must be a regular file (no symlinks or devices in the zip's Unix mode).
- Each path appears once.

## Top-level entries

What a file holds decides what it is (#5865):

| Entry | Means | Read by |
| --- | --- | --- |
| `horizon-mod.toml` | a content mod's manifest ([mods.md](../mods.md#the-manifest)) | `Package`, `Content` |
| `assets/…`, `data/…` | the content mod's resources and data | `Content` |
| `horizon-native.json` | native code: the manifest below | `Bundle` |
| `mixins/<id>/…` | the native bundle's sources ([native-mods.md](../native-mods.md#a-bundle)) | `Bundle` |
| `mod.wasm` | a legacy WebAssembly mod: it doesn't run any more | refused in a file with native code |

- **Content only:** `horizon-mod.toml` with `assets/` and `data/`. Nothing compiles.
- **Native code:** `horizon-native.json` and `mixins/<id>/`. The launcher builds it into the game.
- **Both, in one `.hmod`:** the two manifests name the same mod id. Content entries are checked as content and left
  out of the build. Any other top-level entry refuses the file.
- **`.hnmod`:** native code only, read exactly as before.

The registry still publishes native mods as `.hnmod`. It switches to `.hmod` once launchers and the game read native
`.hmod`s, so no launcher in use gets a file it can't read.

## `horizon-native.json`

A JSON object. Unknown fields refuse the file.

| Field | Type | Rule |
| --- | --- | --- |
| `schema` | number | `1` |
| `id` | string | the mod id, `[a-z0-9_]{1,64}`, the same as `mixins/<id>/` and its `horizon-mixin.toml` `[mod] id` |
| `version` | string | not empty, the same as `horizon-mixin.toml` `[mod] version` |
| `game` | string | not empty, the Minecraft version (`"26.3"`) |
| `horizon` | string | the exact Horizon commit, 40 lowercase hex characters, the same as `[compatibility] horizon_revision` |
| `mixins` | string | the exact horizon-mixins commit, 40 lowercase hex characters |
| `hooks` | number, optional | the native hook points' version it needs, at least `1` ([native-mods.md](../native-mods.md#mods-on-the-hook-points)) |

## Under `mixins/<id>/`

`mixins/<id>/horizon-mixin.toml` must be there. Besides it, only:

- `.rs` and `.toml` files;
- files under `data/`, or under the `data/` of a top-level folder (`engine/data/`);
- `README*`, `LICENSE*`, `LICENCE*`, `COPYING*`, `NOTICE*` and `.gitignore`, in the bundle's folder or a top-level
  folder.

## Content paths

`assets/` or `data/`, then at least two parts of `[a-z0-9_.-]` (vanilla's resource-location characters).

## Limits

| What | Limit |
| --- | --- |
| A registry file, as downloaded | 64 MiB |
| Native files (everything but content entries) | 512 entries, 64 MiB unpacked in total |
| Content files | 65,536 files, 16 MiB each, 512 MiB in total |
| `horizon-mod.toml` inside a native file | 1 MiB |
| `mod.wasm` (legacy) | 32 MiB |
