# How Horizon is built

Who writes Horizon, how changes are checked, and why it is safe to run. You can check every claim on this page in the repository, and anything not done yet is marked as not done.

This page is also on the website: [playhorizon.dev/docs/how-horizon-is-built](https://playhorizon.dev/docs/how-horizon-is-built). The longer version, with what is still missing, is [Security](security.md).

## Who builds it

Horizon is Ludovic Marie's project: he decides what it does and what ships. **Most of the code is written by AI agents**: Claude sessions that Ludovic runs, and contributors' own AI tools. Some contributors write their code by hand.

Agents say what they are. A pull request opened by one of Ludovic's agents starts with "Opened by Ludovic's AI agent (Claude), not by Ludovic himself.", its comments start with "🤖 Agent:", and its commits name the agent in a `Co-Authored-By` line.

## How changes are checked

Not every line is read by a person. Instead, every change is checked against the real game, automatically:

- **Harnesses.** A harness is a test that gives the same inputs to Horizon and to the real Minecraft 26.3, then compares the results. There are 326 of them (25 October 2026), from world generation to mobs, redstone and commands. The [status table](https://playhorizon.dev/#parity) shows what they cover. Passing them all does not prove that nothing else differs.
- **Checks on every pull request.** `verify` runs on GitHub's machines for every pull request and every change to main: formatting, lints with warnings treated as errors, the tests, and the check that no Minecraft files are committed. Main takes no change without it. `parity` runs the harnesses a change can affect, on a machine that holds a copy of Minecraft. For a pull request from a fork, it runs once a maintainer has read the change.
- **Outside contributions are checked again.** Before a contributor's pull request merges, Ludovic's agents compare what it claims with vanilla's own code, review it line by line, rewrite the weak parts, and test it in single player, on a LAN and on a dedicated server where those apply.
- **Releases are tested first.** A release is published only after its smoke test passes (the game starts and draws a world) and while no issue is labelled `release-blocker`.

## Why it's safe to run

- **Open source.** All of the code is public on [GitHub](https://github.com/horizon-rust/horizon), under the MIT licence.
- **Built from the public code.** Releases are built by GitHub Actions, the build service of GitHub, from the public source. Linux and Windows are built on GitHub's machines, and macOS on Ludovic's Mac mini, which runs GitHub Actions jobs. Every build log is [public](https://github.com/horizon-rust/horizon/actions/workflows/release.yml).
- **Signed checksums.** Every release has a `SHA256SUMS` file, a fingerprint of each download, and `SHA256SUMS.sig`, a signature of that list made with Horizon's release key, which only release builds from main can use. The launcher and `horizon-server update` check both before they install anything, and refuse a file that does not match. The very first server install with `install.sh` checks the fingerprint but not the signature.
- **Not yet signed by Apple or Microsoft.** The macOS app is not notarized by Apple, because it is not signed with a paid Apple developer account, so macOS asks you to confirm the first time ([Install](https://playhorizon.dev/docs/install#download-the-launcher) shows how). The Windows programs are not code-signed either, so Windows may warn you too.
- **Mods are off by default, and reviewed.** A content mod is data and runs no code. A native mod is Rust code with your rights, not sandboxed: the launcher builds it only after a warning and your OK, in a game of its own, and the registry lists one only after a maintainer has read its source. See [Mods](mods.md) and [Native mods](native-mods.md).
- **Nothing collected unless you say yes.** Data collection is off until you turn it on, with four separate switches. The one exception is an anonymous "I'm playing" ping for the launcher's player count, which holds nothing about you. See [Opt-in data collection](telemetry.md).

## What it doesn't contain

- **No Minecraft files.** Horizon contains no Minecraft code, assets or data, and never downloads them. The launcher builds the game files on your computer from your own copy of Minecraft: Java Edition 26.3. A check that runs on every pull request (`tools/check_public_tree.py`) keeps game files and decompiled code out of the repository and its history.
- **No code from other mods.** Since 24 October 2026, Horizon's code may only be written from Minecraft 26.3 itself and public documentation, never from another mod, loader or renderer ([the rule](../CONTRIBUTING.md#only-original-code)). That day shader packs were removed ([#1561](https://github.com/horizon-rust/horizon/pull/1561)), after a developer of Iris showed that our shader pack code had been written from Iris's source. They are being written again from a description of the pack format only.

## Check it yourself

- **Read the code and its checks.** [The source](https://github.com/horizon-rust/horizon), [the checks on every pull request](https://github.com/horizon-rust/horizon/actions/workflows/ci.yml) and [the release builds](https://github.com/horizon-rust/horizon/actions/workflows/release.yml).
- **Compare the fingerprints.** Put a release's files and its `SHA256SUMS` in one folder and run `sha256sum -c SHA256SUMS --ignore-missing` (on macOS `shasum -a 256 -c SHA256SUMS --ignore-missing`, on Windows `Get-FileHash <file>` in PowerShell). Each file should say `OK`.
- **Check the signature.** Download a whole release into a folder, then from a clone of the repository run `cargo run -p horizon-release-sign -- verify --tag <tag> <folder>`. The public key is in `crates/horizon-release-sign/release-keys.txt`.
- **Build it yourself.** The README explains how to [build from source](../README.md#build-from-source).
