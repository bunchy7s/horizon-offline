# Versioning

How Horizon's releases are numbered from 0.2 on, how that maps onto the release workflows, and how every install
made before 0.2 gets there. The workflows themselves are in [release-flow.md](release-flow.md).

Until 0.2 the game was tagged by date (`v2026.10.19.15`) and the launcher counted on its own (`launcher-v0.16.0`).
From 0.2 both follow [semantic versioning](https://semver.org) and share one release line: **Horizon 0.2.x** is the
game and the launcher of the 0.2 line.

**Since 2026-10-28 the game ships only Nightlies** ([release-flow.md](release-flow.md#nightly-only)): no new `vX.Y.Z`
game tag is made, and a game build is named `nightly-<date>-<commit>` (its release title, "Nightly <date> ·
<commit>"), which launchers and servers rank after every `vX.Y.Z`. The launcher keeps its `launcher@X.Y.Z` tags on
the release line. What follows describes the `vX.Y.Z` game tags as they were.

## Tags

| | Game | Launcher |
| --- | --- | --- |
| From 0.2 | `v0.2.0`, `v0.2.1`, … | `launcher@0.2.0`, `launcher@0.2.1`, … |
| Before 0.2 | `v2026.10.19`, `v2026.10.19.15`, … | `launcher-v0.1.1` … `launcher-v0.16.0` (and the bridge, `launcher-v0.17.0`) |
| Release title | `Horizon 0.2.3: <what changed>` | `Horizon launcher 0.2.1` |
| The program says | `horizon-server 0.2.3`, `v0.2.3` in crash reports and telemetry | `horizon-launcher 0.2.1` |

- **The line** (`MAJOR.MINOR`) is in `.github/release-line.txt`, one line such as `0.2`. Ludovic decides when it
  moves; moving it is a one-line pull request (`0.3`), and the new line's first releases (`v0.3.0`,
  `launcher@0.3.0`) go out as soon as it merges. Without the file, the release train keeps the tags from before
  0.2. A `feature` label no longer bumps anything: it only lists the change under **New**.
- **Patch releases** are every release the train makes: the patch after the newest release tagged on the line, game
  and launcher counted separately (the game releases more often, so `v0.2.14` can go with `launcher@0.2.3`).
- **One channel** (since 2026-10-23): every game release is published under its final tag as the release and
  GitHub's latest, once its smoke test passed and no `release-blocker` is open
  ([One channel](release-flow.md#one-channel)). The game pre-releases from before (v0.2.1 on, the old Alpha
  channel) keep their tags and stay pre-releases.
- **Pre-release tags** (`v0.3.0-rc.1`, `v0.3.0-beta.2`) are semver's and are reserved for a hand-made preview of a
  new line; the train never makes them. Every reader orders them before their release and after the release
  before (`v0.2.9` < `v0.3.0-beta.1` < `v0.3.0-rc.1` < `v0.3.0`).
- **`launcher@`, not `launcher-v`**: `launcher-v0.2.0` and `launcher-v0.2.1` were taken long before 0.2, and a
  launcher from before 0.2 must not mistake a new 0.2.x for an older release (see below).

### The order every reader uses

Every 0.2 tag is newer than every tag from before 0.2. Within each:

- before 0.2: number by number, missing numbers as zero (`v2026.10.19` = `v2026.10.19.0`);
- from 0.2: semver: major, minor, patch as numbers, then a release after its pre-releases, whose identifiers
  compare one by one (numbers before words, numbers as numbers, a shorter list first).

A tag is a 0.2 tag when it is `v` or `launcher@` followed by exactly three numbers (and an optional `-PRE`) with a
major below 1000; a date's first number is a year, so `v2026.10.21` never is. The rule is written once per
language, with the same tests: `crates/horizon-server/src/update.rs` (`newer`), `launcher/src/backend.rs`
(`release_order`), `tools/release_train.py` (`tag_key`) and
the website's `src/versions.js` (horizon-rust/horizon-site).

## The workflows

- **`release.yml`**: `tools/release_train.py decide --kind game` picks the tag (`next_game_tag`); the build gets the
  version without its `v` in `HORIZON_VERSION`, so `horizon_core::release::VERSION` (and `--version`, the user
  agents, crash reports and telemetry) say `0.2.3`. No version-bump commits: `Cargo.toml` versions are what builds
  from source say. The build checks that `horizon-server --version` and `horizon-prepare --version` say it.
- **`launcher.yml`**: `decide --kind launcher` picks `launcher@0.2.N`; the build gets `HORIZON_LAUNCHER_VERSION`
  (`0.2.N`) and `HORIZON_LAUNCHER_TAG` (`launcher@0.2.N`). The tag tells the launcher which line it is on: a release
  built with only the version (every launcher before 0.2) is a `launcher-v` one.
- **Notes**: a line's first release (`X.Y.0`) opens with a paragraph saying so; `v0.2.0`'s and `launcher@0.2.0`'s say
  that releases are numbered from now on, and tell server owners to run `horizon-server update` (or, if their server
  says it is up to date, `horizon-server update --version v0.2.0`).
- **A line's first release** goes out as soon as the release train runs after the line changes, without waiting for
  a pull request, but never while `RELEASES_PAUSED` is set.

## Existing installs: the way to 0.2

Installs made before 0.2 run old code that compares tags number by number, so to them `v0.2.0` is older than
`v2026.10.19.15` and launcher 0.2.0 older than 0.16.0. What each one does, and what moves it:

| Install | What its old code does with 0.2 | How it gets there |
| --- | --- | --- |
| Launcher 0.16.0 and older, updating itself | Looks only at `launcher-v` tags and takes one above its own version. Never sees `launcher@`. | **The bridge**: one last `launcher-v0.17.0`, built with the new code. Old launchers take it as usual; it knows it is a `launcher-v` release and takes `launcher@0.2.0` next. Two updates, both by themselves. |
| Launcher 0.16.0 and older, installing games | Sorts `v0.2.0` below every dated release and below the versions it has installed, so it never offers it. | Once the launcher has updated itself (above), it offers `v0.2.0` like any update. Until then a player can install `v0.2.0` by hand on the Versions page (it is listed last). |
| Launcher that never updates itself (`HORIZON_LAUNCHER_NO_SELF_UPDATE`, a package) | As above, without the bridge. | Update the package, or install `v0.2.0` by hand. |
| `horizon-server` from before the bridge | Reads GitHub's *latest* release and installs it only if its tag compares higher: `v0.2.0` never does. `--version <tag>` skips the comparison. A server never updates itself: its supervisor only logs a notice every 6 hours, and the running process keeps its old code until the service restarts. | **The game bridge**: the last dated release, built with the new code, made stable (*latest*) before `v0.2.0` is published. `v0.2.0` then spends at least 6 hours as a beta, so *latest* stays the bridge for that long (the longer it stays a beta, the more servers are caught). An owner who runs `horizon-server update` in that time gets the bridge, and the command runs the new code at once, so the next `horizon-server update` takes `v0.2.0` once it is stable. The log notice only changes after the service restarts. An owner on old code after that time is told the server is up to date, and runs `horizon-server update --version v0.2.0` once. 0.2.0's notes say so. |
| Servers the launcher hosts (Servers page) | Set up from the launcher's installed versions. | Follow the launcher. |
| `install.sh`, `install.ps1` | Install *latest*. | Nothing to do. |
| Crash reports and telemetry of releases before 0.2 | Report the folder name (`v2026.10.19.15`). | Unchanged; 0.2 builds report `v0.2.x` from the built-in version. |

No change on the feed side alone could do it: the dated releases can't be hidden (old launchers also compare with the
versions they have installed, and *latest* is a single release), and GitHub's release JSON can't be rewritten. So the
bridges are required, and must never be withdrawn or deleted: `launcher-v0.17.0` stays the step every old launcher
takes, for as long as any is around.

Outside this repository, whatever reads tags must accept `vX.Y.Z` and `launcher@X.Y.Z` too: Kira's release posts,
the fleet's `release-kick` and the orchestrator's heartbeat prompts.

## Irreversible

**None of these can be undone.**

- Once 0.2 is published, dated tags can never be used again: every reader ranks them below `v0.2.0`.
- After `launcher-v0.17.0`, no other `launcher-v` release may ever go out: old launchers would take it.
- `.github/release-line.txt` must never be removed.
- The bridges (`launcher-v0.17.0` and the last dated game release) must never be withdrawn or deleted.

## The day Ludovic says go

Done on 2026-10-22; kept as the record of how 0.2 went out. `promote.yml` and `tools/release_channels.py`, which
it uses, are gone since the single channel (2026-10-23, [One channel](release-flow.md#one-channel)).

Run by the orchestrator, in this order. Until step 3, nothing is published.

1. Merge, in order: this page, the `horizon-server update` order, the launcher order, the release line (train,
   workflows), the version sources and the website pull requests. Not the switch pull request yet. Wait for main's
   `checks` to pass.
2. Turn the freeze off:
   ```sh
   gh workflow enable release && gh workflow enable launcher && gh workflow enable promote
   gh variable delete RELEASES_PAUSED
   U=$(id -u); launchctl enable gui/$U/dev.fleet.release-kick
   launchctl bootstrap gui/$U ~/Library/LaunchAgents/dev.fleet.release-kick.plist   # on the Mac mini
   ```
   **Decision for Ludovic first:** the pre-freeze beta `v2026.10.21` would be promoted. A dry run says
   `Would promote: v2026.10.21` (`python3 tools/release_channels.py promote --dry-run`). The first promote run after
   this step (at :13 or :43) makes it stable and *latest*, hours before the bridge is due. Either he accepts that build as
   stable, or it is held: leave `gh workflow enable promote` out of this step and run it once the bridge has been out
   6 hours (a run promotes only the newest due beta, which is then the bridge). An open `release-blocker` issue
   would hold the bridge too, so it is no way to hold just this one. Do not run this step before he has said which.
3. The bridges go out by themselves within about 25 minutes (the pull requests merged during the freeze count): a
   dated game beta (`v2026.10.DD`) and `launcher-v0.17.0`. Check:
   ```sh
   gh release list --limit 5
   gh run list --workflow launcher --limit 1   # its build log shows horizon-launcher 0.17.0
   ```
4. Let the game bridge become stable on its normal schedule: about 6 hours after it was published, the promote run
   makes it *latest*. Do not force it (`promote` with `force=true`): it carries about 43 player-facing pull requests
   from the freeze and needs its beta like any release. Check it is *latest*:
   `gh api repos/horizon-rust/horizon/releases/latest --jq .tag_name`.
5. Only then merge the update channels pull request (the launcher's Stable and Alpha, the daily promotion), then
   mark the switch pull request (#1132) ready and merge it (`.github/release-line.txt` = `0.2`, and the
   docs that talk about 0.2). On the next run, about 10 minutes, the train publishes `v0.2.0` as a beta and
   `launcher@0.2.0`, both "the first release of 0.2", built from the same code as the bridges plus that pull request.
6. Check the way over from an old launcher, in a scratch folder:
   ```sh
   export HORIZON_LAUNCHER_HOME=$(mktemp -d)
   # with launcher-v0.16.0's binary:  update --check --json  ->  "launcher": "0.17.0"
   # with launcher-v0.17.0's binary:  update --check --json  ->  "launcher": "0.2.0", "horizon": "v0.2.0" (with Beta builds on)
   ```
   and `gh release view v0.2.0 --json assets`, `horizon-server --version` in the Linux server archive.
7. `v0.2.0` becomes stable in the first daily promotion (20:00 UTC) it has been out 6 hours for (or
   `gh workflow run promote -f tag=v0.2.0`). While it is a beta, *latest*
   stays the bridge, so keeping it in beta for longer catches more servers (Ludovic's call). From then on everything
   is automatic again: `v0.2.1`, `launcher@0.2.1`, ….
8. Point Kira, `release-kick` and the heartbeat prompts at the new tags (outside this repository).
