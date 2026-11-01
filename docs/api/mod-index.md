# The registry index and `.hpack`

Horizon's registry lists what is made for Horizon and reviewed: content mods (`.hmod`), native mods (`.hnmod`,
[native-mods.md](../native-mods.md)), modpacks (`.hpack`), resource packs and Horizon shader packs (`.hshader`,
[shaders.md](../shaders.md)). A project is listed by a reviewed pull request in
[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) that links its GitHub repository; its versions are
the repository's GitHub releases, read and checked by the registry's CI with no pull request and no review per version. The launcher and the
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
      "api": "0.1", "side": "client", "capabilities": ["registries"], "game": "26.3",
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
| `api_level` | A legacy field in old entries; the launcher ignores it. |
| `game` | The Minecraft version it is for. |
| `requires` | A modpack's mods: `{ id, version, sha256 }`, each a version in this index with that hash. |
| `file` | A plain name with the kind's extension (`.hmod`, `.hnmod`, `.hpack`, `.zip`, `.hshader`), an `https://` URL (the registry's release `https://github.com/horizon-rust/horizon-mods/releases/download/<id>-<version>/<name>`, or an asset of the project's own repository, `https://github.com/<owner>/<repo>/releases/download/<tag>/<name>`: the launcher accepts exactly these shapes), the size (at most 64 MiB), the lowercase hex SHA-256, and the `.minisig` text when the project has a key. |
| `revoked` | Files taken down: one file by `sha256`, or every version of `id` without it. The launcher refuses to install them and turns installed copies off with the reason. |

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

[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) holds one `mods/<id>.toml` per project and
`revoked.toml`. An entry is **a link and a page**: the project's GitHub repository and what the listing says about it.
Its versions are the repository's GitHub releases. What an entry can't vouch for is read from the file by the kind's own
check (`index::inspect`): a mod's API, side and capabilities, a modpack's mods, the size. Types and rules:
`horizon_mods::index::registry`.

```toml
id = "coords_hud"                # the file's name, and the manifest's id
kind = "mod"                     # mod | native-mod | modpack | resourcepack | shader
repo = "horizon-rust/coords-hud" # GitHub owner/name: its releases are its versions
owners = ["ludovic111"]          # GitHub logins responsible for the project
key = "RWT..."                   # optional: the minisign public key every version is signed with
icon = "icons/coords_hud.png"    # optional: a PNG of at most 256 KiB in the repository, or https://
# The page. Each field may instead come from horizon-registry.toml in the repo (below); these win.
name = "Coordinates HUD"
summary = "One line, at most 160 characters."
description = """Markdown."""
authors = ["Horizon"]
license = "MIT"
# source defaults to https://github.com/<repo>
```

**Versions come from the releases.** For each entry with a `repo`, the registry's CI lists its GitHub releases (drafts
and pre-releases are skipped) and, for each release it has not read yet, downloads the asset with the kind's extension
(and its `<name>.minisig` when the project has a key), runs the checks of a submission (below), and if they pass
adds the version. The version, the game and the pins are the file's own manifest (`horizon-mod.toml`,
`horizon-native.json`, `horizon-modpack.toml`, the shader manifest); the release's tag stands in where the file
names none (a leading `v` is dropped). `published` is the release's date, `changelog` its notes (cut at 6000
characters), `file.url` the asset's address in the repository, `file.sha256` and `size` are computed by the CI and, when GitHub
gives the asset a digest, must equal it. Tags and asset names use letters, digits, `.`, `-`, `_` and `+`. A release
that fails a check is skipped and reported in an issue of the registry; the same bytes are not read again. The versions that passed are kept,
in the order they were read, in `versions/<id>.json` on the registry's `versions` branch (the page last read from
the repository is kept with them), so the history is auditable and the index does not change when a repository
does: a release deleted or an asset replaced later does not change the index (the launcher refuses bytes that are not the pinned ones).

**`horizon-registry.toml`** at the root of the repository, as of the newest release's tag, may hold `name`, `summary`,
`description`, `authors` and `license` (unknown keys, and anything about owners, key, kind or icon, are refused): they fill the
page fields the entry leaves out. A listing that is incomplete without it is refused until it is.

**`[[versions]]` in an entry are optional** and kept for projects listed before releases were read, and for Horizon's own
mods built in the registry (`native/<name>/`):

```toml
[[versions]]                     # newest first
version = "0.1.0"                # letters, digits, '.', '-', '_': the release tag is <id>-<version>
published = "2026-10-24"
game = "26.3"
file = "coords_hud-0.1.0.hmod"   # the name players download, a release asset of the registry
sha256 = "e113..."               # pins the reviewed file
signature = """untrusted comment: ..."""   # when the project has a key
changelog = ""                   # optional
```

The index lists both kinds, newest first. A project listed by its repository with no release yet is not in the index.

```toml
# revoked.toml
[[revoked]]
id = "bad_mod"
sha256 = "1111..."               # optional: without it, every version
reason = "Sent chat messages it didn't declare."
date = "2026-10-24"
```

Who reviews what: **a project is reviewed when it is listed** (the pull request that adds its entry: the
repository, its source, the page, the owners), and again if its `repo`, owners or key change. **Later releases are
checked automatically, not reviewed by a person**; `revoked.toml` takes a bad one down. A signing `key` makes a stolen
repository or account useless for publishing an update.

The submission form's service (`crates/horizon-mod-registry`) lists a new project: it signs the creator in with GitHub,
reads the repository's address and the page, and opens the pull request. It no longer takes updates for a project listed
with a `repo`: "publish a GitHub release in <repo>". Projects without a `repo` can still be updated through the form
(it uploads the file to the repository's `staging` release as `<sha256>.<extension>` and opens a pull request; an
owner's update with nothing else changed is merged by `owner-update` once the checks pass). An owner may turn a `mod` into a `native-mod` once by sending an `.hnmod`
(reviewed by hand, never an owner update); no other kind change goes through. A submission that GitHub broke off after
making its `submit/<id>-<version>` branch can be sent again: the same account's retry starts that branch again from
`main` while it has no open pull request; anyone else's is refused.

Unknown keys in an entry are refused. Its CI runs `cargo-horizon-mod` (`crates/cargo-horizon-mod`):

| Command | What it does |
| --- | --- |
| `cargo horizon-mod check <file> [--json]` | A submission's checks: its kind's (above) and its signature with `--key` and `--signature`. It runs no mod's code: a native mod's source is read by the reviewers when it is listed. |
| `cargo horizon-mod registry check <base> <head> --files <dir>` | `check` on each hosted file `head` lists and `base` doesn't: a pull request's new files. |
| `cargo horizon-mod registry list <checkout>` | `<sha256> <id> <version> <file>` per hosted version: the files to download. |
| `cargo horizon-mod registry build <checkout> --files <dir> --out index.json` | Reads every entry, its hosted files (`<dir>/<sha256>`) and `versions/<id>.json`, lists every problem, and writes the index (`--release`, ending in `/`, defaults to the repository's releases, for hosted versions). |
| `cargo horizon-mod registry release <checkout> <id> <file> --tag T --published D [--notes F] [--signature F] [--game G]` | The checks of a submission, then the version a release of the project's repository is, as the index lists it (JSON). |
| `cargo horizon-mod registry page <checkout> <id> <horizon-registry.toml>` | The repository's page file, checked against its entry (JSON). |
| `cargo horizon-mod registry review <base> <head> --author <login>` | A pull request's changes: a published file never changes and is never removed (take it down in `revoked.toml`), a kind never changes; changes by a non-owner and to owners, keys or the repository are pointed out for the reviewers. |

How the repository runs (its workflows are in `.github/workflows/`):

- **A pull request** (`check.yml`) runs `registry review`, downloads each new hosted file from the `staging` prerelease,
  runs `registry check` on it and builds the index it would publish; the report is the run's summary. The tool is
  `cargo-horizon-mod` from Horizon at the commit in the repository variable `HORIZON_REV`.
- **Every hour, and on a push to `main`** (`publish.yml`): read the new releases of every entry with a `repo` as above and
  keep the result on the `versions` branch, make each new hosted version a release `<id>-<version>`, and deploy
  `index.json` and `icons/` to GitHub Pages: `https://horizon-rust.github.io/horizon-mods/index.json`, served with
  `Access-Control-Allow-Origin: *`.
- **Merging** an entry needs the `check` run and a CODEOWNER's review.
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

- 2026-10-30: an entry names its GitHub repository (`repo`) and its versions are the repository's releases, read by the registry's CI; `file.url` may be a release asset of that repository. The index's schema is unchanged.
- 2026-10-26: `projects[].downloads`, GitHub's download counts of a project's files, refreshed every hour.
- 2026-10-26: the launcher installs `shader` files into `shaderpacks/`; the sample index lists one (`glow`).
- 2026-10-25: `shader` opens, checked by `horizon_shader_api::check`; its files are `.hshader`.
- Schema 1 (2026-10-24): kinds `mod`, `modpack`, `resourcepack`, `shader`; `.hpack` format 1.
