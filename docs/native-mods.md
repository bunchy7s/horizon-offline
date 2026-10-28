# Native mods (experimental)

Native mods are **mixin bundles** that change the game's own Rust source before it is compiled, with
[horizon-mixins](https://github.com/horizon-rust/horizon-mixins) (MIT, by levzzz5154, made for Horizon). A bundle
is a `horizon-mixin.toml` and a few `.rs` helpers that hook into named functions of the game (HEAD, BEFORE_CALL,
RETURN, MODIFY_ARG, OVERWRITE...; the tool's README lists them). A port such as Veinminer is three HEAD hooks and
one module. Start a mod from [horizon-native-mod-template](https://github.com/horizon-rust/horizon-native-mod-template).

Native mods are native code with the player's rights, not sandboxed. The launcher offers them off by default, behind
a warning, reviewed before the registry lists one, and in a game of their own, built for the instance they are installed in.

## Pinned to a Nightly

A native mod builds on one exact Horizon commit: `[compatibility] horizon_revision` in its `horizon-mixin.toml`,
and `horizon` in the `horizon-native.json` of its registry file (`.hnmod`, `horizon_mods::index::native`). Mods
pin a **Nightly's commit** (or a release's). Every Nightly ships its source and keeps its commit for good as the
tag `nightly-source/<date>-<short>` ([release-flow.md](release-flow.md#nightly-channel)), so a pinned mod still
builds after its Nightly is replaced. All the mods of one build must pin the same commit; when they don't, the
launcher says which mod pins which, and the player installs versions made for the same Nightly. A bundle with no
pin builds on the commit of the installed version the build starts from.

## What the launcher does

In the window: install a native mod from the Mods page, turn on Settings → Game → **Native mods (experimental)**,
then **Build**. The launcher names the mods, warns in red, and builds only from "I trust these mods: build"; the
row shows each step, the error if one fails, and the build the instance plays. From the command line:
`horizon-launcher native-build [FOLDER] [--instance NAME] --trust-native-code`. FOLDER is a bundle, an unpacked
`.hnmod`, or a folder of them; without it, the native mods turned on in the instance. It is not an MCP tool:
trusting native code is the player's decision, never an agent's.

1. **Rust.** The launcher's own toolchain ([`rust_toolchain`](../launcher/src/rust_toolchain.rs)): Rust 1.99.1's
   official `rustc`, `cargo` and `rust-std` (and `rust-mingw` on Windows), each pinned by SHA-256, downloaded once
   from static.rust-lang.org into `<launcher data>/rust/`, with its own Cargo home `<launcher data>/cargo/`. No
   rustup, nothing on the system; the player's own Rust is never used. Windows uses the `windows-gnu` host, which
   links without Visual Studio. The linker is the system's on macOS (the Command Line Tools: the launcher asks for
   `xcode-select --install`) and Linux (`cc`). `horizon-launcher rust status|install|remove`.
2. **horizon-mixins**, at the commit the mods name (their `.hnmod`'s `mixins`), else `native_mods::MIXINS_REV`,
   built once with that cargo (`cargo install --locked --git … --rev …`) into `native/tools/<commit>/`.
3. **The source** of the pinned commit: GitHub's archive of it, once per commit (`native/source/<commit>/`, about
   63 MB unpacked), with the commit in `.horizon-revision`. Never the player's own checkout. Links in the
   archive are left out: the tool refuses them and nothing needs them to build.
4. **Mixins.** The bundles are copied first, so the build and its key use that copy whatever happens to the
   folder meanwhile. `horizon-mixins apply` stages the modded source; the launcher copies only the files that
   changed into `native/stage/`, so Cargo, which goes by file times, recompiles only the crates the mods touch.
5. **Compile**: `cargo build --locked --profile release --features horizon-client/mods -p horizon-client`, the
   release's own command (game-build.yml), into `native/target/`. Cargo downloads the locked crates from
   crates.io (Horizon has no git dependencies). The log is `native/build.log`; a failure shows its last lines (a
   horizon-mixins diagnostic in one line).
6. **Install**: a new version, `native-<commit>-<key>`, a copy of the starting version's folder (resources
   included, cloned on APFS, Btrfs and ReFS) with the built game in place of its own, and a `native-mods.json`
   saying which commit, mods, horizon-mixins and Rust made it. The instance the mods are installed in plays it (a
   folder built from the command line, or mods of the default home, go to the **Native mods** instance).

## Cache and disk

The key hashes the commit, the horizon-mixins commit, Rust's version, the profile and features, and every file of
every bundle. The same set on the same commit is never built twice. A change gives a new build, and
`native/target/` stays, so a rebuild compiles only `horizon-client` and what the mods touched.

Measured 2026-10-28 on the Mac mini (M4 Pro, 14 cores) with Veinminer 0.3.0 on Nightly `83f15fa`,
`CARGO_BUILD_JOBS=8` while other agents' builds ran (upper bounds):

| What | Size, time |
|---|---|
| Rust 1.99.1 (`<launcher data>/rust/`) | 453 MB; 58 s to download and unpack |
| horizon-mixins (`native/tools/`) | 7 MB; built once in 19 s |
| Crates in the launcher's Cargo home (`<launcher data>/cargo/`) | 289 MB |
| Source of one commit, and the stage | 64 MB each |
| First build: release profile, the whole game | 9 min 10 s of compiling, 9 min 41 s in all; `native/target/` 1.5 GB |
| Rebuild after a helper's code changed | 2 min 46 s of compiling (only `horizon-client`), 2 min 53 s in all |
| A change horizon-mixins ignores (a comment) | nothing compiled; 31 s, mostly the version folder's copy |

Only the native builds an instance plays are kept (`native_mods::prune`); `native/target/` stays for the next one.

## Trust

- **Off by default.** The command needs `--trust-native-code`; the window shows a warning in red before every
  build: the mods run with full access to the computer, and their ids and versions. The Mods page says who made
  each one, and "Not made or endorsed by the Horizon project" for any mod not Horizon's own.
- **The registry** (horizon-rust/horizon-mods) lists them as their own kind, `native-mod`, only after a maintainer
  has read the source: the `.hnmod` *is* the source (only `.rs` and `.toml`, checked without running anything),
  pinned by SHA-256 like every registry file. An update is reviewed again; `revoked.toml` takes one down.
- **Isolation of data.** Native mods run only in the instance built for them, with its own worlds, options and logs.
- What this does not do: sandbox the compiled game. horizon-mixins' checks stop mistakes, not a malicious helper
  ([security notes](https://github.com/horizon-rust/horizon-mixins/blob/main/docs/security.md)).

## Multiplayer

- **Single player and LAN host:** the mods run in the host's game and its integrated server, so a server-side
  hook applies to every player of the world.
- **Joining a server:** only client-side hooks do anything; the protocol is unchanged.
- **Dedicated servers:** not built yet; the same pipeline can build the server (`--no-default-features`).

## Left to do

- Rebuild after the mods or the Nightly change (asking first, as it compiles for minutes).
- A prebuilt horizon-mixins from its CI instead of building it on the player's computer.
- Windows and Linux: the toolchains are pinned, but a full game build on them is unproven.
