# Contributing to Horizon

Thanks for helping. Horizon is Minecraft: Java Edition 26.3 rewritten in Rust, and the one rule that shapes
everything is that **it plays exactly like vanilla**. This page is the short version of how we work; questions are
welcome on [Discord](https://discord.gg/playhorizon).

This repository holds the game and the launcher, plus the tools, tests and docs that build, check and release them.
The website (playhorizon.dev) is built in a separate repository from this one's README, `docs/api/` and roadmap. Anything else (videos, Discord and other community material) lives elsewhere.

**Where to start: [playhorizon.dev/hub](https://playhorizon.dev/hub).** It shows what is being done now and next
([ROADMAP.md](ROADMAP.md) in the repository), the work board, good first tasks, and the forms to report a bug or
suggest something. A pull request for a roadmap item carries its `roadmap:<id>` label; ideas go to the suggestion
queue (the hub, or `/suggest` on Discord) rather than to issues, which are for bugs.

## Build it

You need Rust (stable), Python 3.10+, JDK 25, and on Linux the ALSA and udev headers (`libasound2-dev libudev-dev`).
Horizon never downloads the game: it prepares its resources from your own copy of Minecraft 26.3, as described in
[Build from source](README.md#build-from-source).

```sh
cargo run --release -p horizon-prepare -- --prepare --server-jar ~/Downloads/server.jar
cargo run --release -p horizon-client
```

The launcher is its own Cargo workspace in `launcher/` (`cd launcher && cargo run`; on NixOS, `nix-shell` first).

## Vanilla first

Wherever vanilla has a feature, vanilla is the specification, down to the order of random calls. "Parity" here
means Horizon gives the same results as the pinned 26.3 client and server for the same inputs, checked by a
**harness**: a `tools/test_*_parity.py` script that runs a small Java oracle against the real game and a Rust
probe against Horizon, and compares their outputs. There are about 290 of them.

- **Read the vanilla code before writing Horizon's.** `tools/rea_vanilla.py` queries the pinned game bytecode:
  `python3 tools/rea_vanilla.py method net.minecraft.world.level.block.DispenserBlock dispenseFrom`.
  Say in your pull request which vanilla classes and methods you followed.
- **Gameplay changes come with a harness**, new or extended, so the next change can't silently break them.
  [The parity workflow](docs/parity-workflow.md) shows how harnesses run; copy a nearby one.
- **Horizon's own features** (the launcher, the new menus) follow [the design system](docs/design-system.md) and are
  off by default wherever they would change the vanilla look.
- **Everything players run is Rust.** Python and Java are fine for dev tools and harness drivers in `tools/`, never
  for player-facing code.

Never commit game files, decompiled source or prepared resources (`.local/`): `tools/check_public_tree.py` rejects
them (`git config core.hooksPath .githooks` runs it before every commit).
The pre-push hook checks the full history reachable from the branches and tags being pushed. Run
`python3 tools/check_public_tree.py --history` to audit the history of all local refs.

## Only original code, or code whose license allows it

Horizon is MIT. Its code is written from Minecraft 26.3 itself and from public documentation. Since 30 October 2026
another project's source may also be read and used, **only when that project's license allows it**:

- **Allowed:** MIT, MIT-0, BSD-2-Clause, BSD-3-Clause, 0BSD, ISC, zlib, Apache-2.0, Unlicense, and the CC0-1.0
  public-domain dedication. That code may be read, copied or adapted.
- **Everything else is off limits, even to read:** GPL, LGPL (Iris), AGPL, MPL, EPL, Polyform (Sodium), the Creative
  Commons licenses (CC BY, CC BY-SA, CC BY-NC and the others; CC0 is a public-domain dedication, not one of them),
  custom or "source available" licenses, all rights reserved (OptiFine), and any project with no license or an
  unclear one. Don't read, copy, translate or paraphrase their source, and don't send code an AI tool
  wrote from it. Being able to download a mod or a pack does not give you the right to read its files.
- **Check the license yourself.** Read the LICENSE file at the commit you use and the header of each file you take:
  some repositories mix licenses, and a file under another license is off limits. A license that changed: the one
  at your commit counts. In doubt, don't: ask in #contributors first.
- **Say it in the pull request:** the project, a link to the commit, its license, and what you read or copied. A
  pull request that used another project's source without saying so is closed.
- **Copied or adapted code** keeps its copyright line and license notice at the top of the file, and gets an entry
  in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Apache-2.0 code also keeps its NOTICE text and says what
  was changed.

User documentation and file formats are fine to read, and packs and mods are fine to run and observe, whatever
their license. Shader packs were removed on 24 October 2026 (#1561) because their code had been written from
another project's source; they come back only as a clean-room rewrite from a written description of the pack
format.

## Before asking for review

Run the self-check and paste the block it prints into your pull request:

```sh
python3 tools/pr_ready.py --base upstream/main   # or origin/main if you work on the main repository
```

It looks at what your branch changes and runs what matters: rustfmt, clippy with warnings denied, the tests of the
crates you touched (and the launcher's), generated files, `Cargo.lock`, the Python tests, and the vanilla harnesses
your change can affect. Without the vanilla data on your machine it lists those harnesses instead; CI runs them.
`--quick` skips the harnesses. It exits non-zero when something fails.

For the game workspace, Clippy checks all targets of the touched crates and their transitive workspace dependents,
including examples. The dependency scope includes optional, platform, build and development dependencies.
Tests run library, binary, integration and doc tests without building unrelated example executables. Shared Cargo
configuration and toolchain changes select the whole workspace; CI also keeps its workspace-wide checks.

## Claim before you start

Work is tracked in GitHub issues, and an issue that is assigned is taken (so is one with the `in progress` label:
claimed on Discord with Kira's `/claim`): pick another, or ask its holder in the thread. Before you start, comment
**`/claim`** on the issue; you are assigned at once, no role needed. No issue for your work yet? Open one (a line is
enough), then claim it. `/unclaim` gives it back.

A claim with no open pull request linking the issue (`Fixes #123` in the description, or a draft) is released after
5 days, with a comment: open a draft early. You can hold 8 open issues at a time, not counting the ones your
pull requests already fix. Agents claim the same way.

When your pull request changes the same functions as another open one, or closes the same issue, a bot comment names
it (edited on every push, never repeated): talk to its author in the thread before you both finish the same work.

## Pull requests

- **Small.** One change per pull request, under about 500 lines of code where you can; split bigger work into
  pieces that each make sense alone. Small pull requests get reviewed and merged the same day.
- **Fill in the template**: what and why, the vanilla reference, the pr_ready block, screenshots for anything you
  can see.
- **Tick "Allow edits by maintainers".** When something small is left (a lint, a rebase, a nit) we push the fixup
  ourselves instead of sending it back to you; only design questions come back.
- **Once we've pushed to your branch, merge main into it; don't rebase or force-push.** A rebase or force-push
  replaces our commits with yours and erases the fixes we added (it happened on #5188 and #5203). Tell your agent
  too: `git merge origin/main`, never `git rebase` or `git push --force` on a branch a maintainer has touched. If
  our fixes get erased anyway, we finish the work in our own pull request that keeps your commits and credits you.
- **Tick its kind** in the template: `parity` (Horizon now does what vanilla does), `bug` (a crash or breakage),
  `perf`, `feature` (new for players), `extra` (Horizon-only, off by default), `tooling` or `docs`. A workflow sets
  that label, and area labels (`launcher`, `mobs`, `worldgen`, `rendering`...) from the paths you change.
  `label:parity` is the list of real progress toward vanilla.
- **Release labels** (we set them if you can't): `feature` also puts it under "New" in the release notes,
  `skip-release` when players see no change (tooling, docs, tests), nothing for a fix.
  `release-now` is for crash fixes that shouldn't wait. pr_ready suggests one.

Who reviews and merges: the Horizon Team, on CI for ordinary pull requests; risky ones (saves, network, world
generation, the launcher's updates) wait for a local proof first ([merging](docs/release-flow.md#merging)).

Every merge to main is released automatically ([release flow](docs/release-flow.md)), so main always stays playable.
How agents, checks, releases and signed updates keep this safe: [security](docs/security.md).

## Parity checks on forks

The `verify` check (formatting, lints, tests) runs on every pull request. The `parity` check runs the affected
harnesses on a maintainer's machine, which holds the game files, so it never runs a fork's code on its own: a
maintainer reads your change, then adds the `run-parity` label, which runs it once on your current commit. After
new pushes, we add it again.

## Talk to us

[Discord](https://discord.gg/playhorizon) for questions and ideas, [issues](https://github.com/horizon-rust/horizon/issues)
for bugs. To take something on, `/claim` its issue ([above](#claim-before-you-start)).
