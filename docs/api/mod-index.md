# The registry index and `.hpack`

Horizon's registry lists what is made for Horizon and reviewed: content mods (`.hmod`), native mods (`.hnmod`,
[native-mods.md](../native-mods.md)), modpacks (`.hpack`), resource packs and Horizon shader packs (`.hshader`,
[shaders.md](../shaders.md)). Submissions are pull requests in
[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods); merging one publishes it. The launcher and the
website read one file, `index.json`, which that repository's CI builds. The Rust types and every check below are
`horizon_mods::index` and `horizon_mods::modpack` (feature `index`); the sample is
`crates/horizon-mods/tests/index/index-v1.json`, read by the tests with the files it lists.

Level: preview ([stability.md](stability.md)). Schema 1.

## `index.json`

```json
{
  "schema": 1,
  "generated": "2026-10-24T18:00:00Z",
  "projects": [{
    "id": "coords_hud", "kind": "mod", "name": "Coordinates HUD", "summary": "One line.",
    "description": "Markdown.", "authors": ["Horizon"], "owners": ["ludovic111"],
    "license": "MIT", "source": "https://github.com/...", "icon": "https://...png", "key": "RWT...", "downloads": 1234,
    "versions": [{
      "version": "0.1.0", "published": "2026-10-24", "changelog": "",
      "api": "0.1", "side": "client", "capabilities": ["game-read", "hud", "chat"], "api_level": 0, "game": "26.3",
      "file": { "name": "coords_hud-0.1.0.hmod", "url": "https://github.com/horizon-rust/horizon-mods/releases/download/...",
                "size": 438, "sha256": "e113...", "signature": "untrusted comment: ...\n...\ntrusted comment: ...\n...\n" }
    }]
  }],
  "revoked": [{ "id": "bad_mod", "sha256": "1111...", "reason": "Sent chat messages it didn't declare.", "date": "2026-10-24" }]
}
```

| Field | Meaning |
| --- | --- |
| `schema` | 1. A reader refuses a higher number with "update Horizon" rather than misread it. |
| `projects[].id` | `[a-z0-9_]{1,64}`, unique; a mod's is its manifest's `id`. |
| `kind` | `mod` (content), `native-mod`, `modpack`, `resourcepack`, `shader` (Horizon's own shader packs, not Iris or OptiFine packs). Other kinds (data packs, worlds, themes) may come without a new schema: a reader keeps a kind it doesn't know and doesn't offer it. |
| `summary`, `description` | One line for lists; Markdown for the project's page. |
| `owners` | GitHub logins allowed to publish updates. |
| `license`, `source` | An SPDX expression and an `https://` address of the source code: both required. |
| `icon` | `https://`, optional. |
| `key` | The author's [minisign](https://jedisct1.github.io/minisign/) public key. When set, every version must be signed with it; an update signed with another key is refused. Absent: the project is unsigned and the launcher says so. |
| `versions` | Newest first, at least one. |
| `downloads` | How many times its files were downloaded, every version together: GitHub's count of each release asset, summed by the publish job, which runs every hour. Absent until a file is downloaded. Read-only: never in an entry. |
| `api` | A mod's API line (its manifest's `api`); a native mod's pinned revisions (`native-1/<horizon>/<mixins>`); a modpack's manifest format (`1`). |
| `side`, `capabilities` | A mod's, read from its manifest by CI, never typed by hand. |
| `api_level` | For a WebAssembly mod published before they were removed: the mod API level its code needs, read from the file by CI. Absent for content mods and native mods. The launcher doesn't install such a mod alone (it "needs a native port"); it stays in the index for the modpacks that require it. |
| `game` | The Minecraft version it is for. |
| `requires` | A modpack's mods: `{ id, version, sha256 }`, each a version in this index with that hash. |
| `file` | A plain name with the kind's extension (`.hmod`, `.hnmod`, `.hpack`, `.zip`, `.hshader`), an `https://` URL, the size (at most 64 MiB), the lowercase hex SHA-256, and the `.minisig` text when the project has a key. |
| `revoked` | Files taken down: one file by `sha256`, or every version of `id` without it. The launcher refuses to install them and turns installed copies off with the reason. |

The sample's `coords_hud` is a WebAssembly mod from before they were removed, which is why it has an `api_level`.

Readers ignore fields they don't know, so an optional field can be added to schema 1. Renaming or removing a field, or
changing what one means, makes schema 2.

## Before a file is installed

`index::check_file` runs on the downloaded file, before it reaches the game folder:

1. Its size and SHA-256 are the ones the entry lists, and it isn't revoked.
2. With a project key: its signature verifies with that key (minisign's prehashed and legacy signatures both).
3. What the file says matches the entry: a mod's `horizon-mod.toml` (id, version, API, side, capabilities), its API
   level when the entry lists one, and its
   content's fence ([mods.md](../mods.md#content-mods-api-02)); a native mod's `horizon-native.json` and
   `horizon-mixin.toml` (id, version, the pinned Horizon and horizon-mixins revisions); a modpack's manifest (id, version, format, game,
   mods); a resource pack's `pack.mcmeta`; a shader pack's `horizon_shader_api::check` and manifest (id, version,
   shader API).
4. A kind this build can't install is refused with "this Horizon can't install a … yet".

## `.hpack`

A zip holding `horizon-modpack.toml` and, optionally, `overrides/`. The mods are not inside: the launcher downloads
each from the index and checks it as above.

```toml
format = 1
id = "cozy"                 # [a-z0-9_]{1,64}: the profile's folder
name = "Cozy"
version = "1.0.0"
description = "Coordinates and a calm render distance."
authors = ["Horizon"]
game = "26.3"

[[mods]]
id = "coords_hud"
version = "0.1.0"
sha256 = "e1135345692f6dd8fd4c51920151944b82333e17049c9bb7d756f27b8deefc53"
on = true                   # optional, default true: turned on in the profile's horizon-mods.txt
```

Unknown keys are refused. `overrides/` may hold only `options.txt`, `horizon-graphics.txt`, `horizon-extras.txt`,
`resourcepacks/<name>.zip` and `horizon-mods/.horizon-mod-data/<a listed mod>/...` (a mod's own saved settings,
a native mod's saved settings), each at most 64 MiB and 256 MiB in all; anything else refuses the pack. The launcher copies
them into the profile's game folder.

## The registry repository

[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) holds one `mods/<id>.toml` per project, written by
the submission form or by hand, and `revoked.toml`. What an entry can't vouch for is read from its file by the kind's
own check (`index::inspect`): a mod's API, side and capabilities, a modpack's mods, the size. Types and rules:
`horizon_mods::index::registry`.

```toml
id = "coords_hud"                # the file's name, and the manifest's id
kind = "mod"                     # mod | modpack | resourcepack | shader
name = "Coordinates HUD"
summary = "One line, at most 160 characters."
description = """Markdown."""    # optional
authors = ["Horizon"]
owners = ["ludovic111"]          # GitHub logins who may publish updates
license = "MIT"
source = "https://github.com/..."
icon = "icons/coords_hud.png"    # optional: a PNG of at most 256 KiB in the repository, or https://
key = "RWT..."                   # optional: the minisign public key every version is signed with

[[versions]]                     # newest first
version = "0.1.0"                # letters, digits, '.', '-', '_': the release tag is <id>-<version>
published = "2026-10-24"
game = "26.3"
file = "coords_hud-0.1.0.hmod"   # the name players download
sha256 = "e113..."               # pins the reviewed file
signature = """untrusted comment: ..."""   # when the project has a key
changelog = ""                   # optional
```

```toml
# revoked.toml
[[revoked]]
id = "bad_mod"
sha256 = "1111..."               # optional: without it, every version
reason = "Sent chat messages it didn't declare."
date = "2026-10-24"
```

The submission form's service (`crates/horizon-mod-registry`) writes these entries: it signs the creator in with GitHub, runs the file's checks, uploads the file to the repository's `staging` release as `<sha256>.<extension>` and opens the pull request. Only an entry's `owners` publish updates through it.

Unknown keys are refused. Its CI runs `cargo-horizon-mod` (`crates/cargo-horizon-mod`):

| Command | What it does |
| --- | --- |
| `cargo horizon-mod check <file> [--json]` | A submission's checks: its kind's (above) and its signature with `--key` and `--signature`. It refuses an `.hmod` with WebAssembly code, and runs no mod's code: a native mod's source is read by the reviewers. |
| `cargo horizon-mod registry check <base> <head> --files <dir>` | `check` on each file `head` lists and `base` doesn't: a pull request's new files. |
| `cargo horizon-mod registry list <checkout>` | `<sha256> <id> <version> <file>` per version: the files to download. |
| `cargo horizon-mod registry build <checkout> --files <dir> --out index.json` | Reads every entry and its file (`<dir>/<sha256>`), lists every problem, and writes the index with URLs `<release><id>-<version>/<file>` (`--release`, ending in `/`, defaults to the repository's releases). |
| `cargo horizon-mod registry review <base> <head> --author <login>` | A pull request's changes: a published file never changes and is never removed (take it down in `revoked.toml`), a kind never changes; changes by a non-owner and to owners or keys are pointed out for the reviewers. |

How the repository runs (its workflows are in `.github/workflows/`):

- **A pull request** (`check.yml`) runs `registry review`, downloads each new file from the `staging` prerelease, runs
  `registry check` on it and builds the index it would publish; the report is the run's summary. The tool is
  `cargo-horizon-mod` from Horizon at the commit in the repository variable `HORIZON_REV`. The form's pull requests
  are opened with the token of the account in the variable `REGISTRY_BOT`; for those, the author is the login in the
  body's `<!-- submitted-by: … -->` line.
- **Merging** needs the `check` run and a CODEOWNER's review. `publish.yml` then makes each new version a release
  `<id>-<version>` and deploys `index.json` and `icons/` to GitHub Pages:
  `https://horizon-rust.github.io/horizon-mods/index.json`, served with `Access-Control-Allow-Origin: *`.
- **Weekly**, `cleanup.yml` deletes staged copies of published files after 7 days and of files no open pull request
  lists after 30.
- **Taking a file down** is a pull request adding it to `revoked.toml`. Removing an entry outright (a test, a project
  nobody installed) fails `registry review` on purpose and needs an administrator's merge.

Kinds open to submissions: `mod` (content only), `native-mod`, `modpack`, `resourcepack`, `shader`. A shader pack (`.hshader`) passes
`horizon_shader_api::check`, the check the game runs before it loads one (`horizon-shader check`, [shaders.md](../shaders.md)):
every stage validated by naga and translated for Vulkan, Metal and DirectX 12; its manifest's id, version and shader
API line are its facts. Iris and OptiFine packs are for the Modrinth browser. A new kind gets its own check before it
opens.

## Changelog

- 2026-10-26: `projects[].downloads`, GitHub's download counts of a project's files, refreshed every hour.
- 2026-10-26: the launcher installs `shader` files into `shaderpacks/`; the sample index lists one (`glow`).
- 2026-10-25: `shader` opens, checked by `horizon_shader_api::check`; its files are `.hshader`.
- Schema 1 (2026-10-24): kinds `mod`, `modpack`, `resourcepack`, `shader`; `.hpack` format 1.
