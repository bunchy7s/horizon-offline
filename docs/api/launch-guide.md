# Running Horizon from another launcher

From picking a build to playing, for a launcher that isn't `horizon-launcher` (Prism Launcher, MultiMC,
ATLauncher, Modrinth App, HMCL or your own). The contract it relies on is the [Launch API](launch.md); the options
are in [launch-reference.md](launch-reference.md). `horizon-launcher` does all of this and is the reference
implementation.

## 0. Only for an owner

Horizon contains none of Minecraft's code, textures or sounds: it prepares them from the player's own copy. So
every step below is for a player signed in with a Microsoft account that **owns Minecraft: Java Edition**, the
same gate `horizon-launcher` applies before every start. Horizon never downloads Mojang's files; when your
launcher downloads them (step 3), it does so only for that owner, as it would for vanilla.

## 1. Pick a build

Builds are GitHub releases of `horizon-rust/horizon`, one archive per platform:

| Platform | Archive |
| --- | --- |
| Windows x64 | `horizon-windows-x64.zip` |
| Linux x86_64 | `horizon-linux-x86_64.tar.gz` |
| macOS Apple silicon | `horizon-macos-arm64.zip` |
| macOS Intel | `horizon-macos-x64.zip` |

- The newest is the release tagged `nightly`. Each one is also published as `nightly-<date>`, kept for **seven
  days** and then deleted, so remember the tag you installed and expect it to disappear (a signed release index
  and builds kept longer are not there yet).
- Check the archive before unpacking: `SHA256SUMS` lists every archive's SHA-256, and `SHA256SUMS.sig` signs it
  with Ed25519 over `horizon release <tag>\n` followed by `SHA256SUMS`, so a signed list can't be replayed under
  another tag (a Nightly's message says `horizon release nightly`, whichever date it carries). The public keys are in `crates/horizon-release-sign/release-keys.txt`
  ([data-formats.md](data-formats.md), [release-flow.md](../release-flow.md#signed-checksums)).
- Unpack each build into a folder of its own and keep it whole: `horizon-client`, `horizon-prepare`,
  `research-lock.json` and `exporters/` work together. On macOS the build is signed ad hoc, so the first start
  needs the usual right-click > Open, or your launcher clears the quarantine flag.

## 2. Ask the build what it supports

Two calls, both quick, both before anything else runs:

```sh
horizon-client --launch-api
horizon-prepare --launch-api
```

Each prints one JSON object and exits 0. Check `launch_api`: within its line (0.1.x today) a newer build only adds.
From `horizon-prepare`'s reply take `minecraft` (the Minecraft version whose files it needs), `java_major` (the
Java it runs) and the `options` it accepts; from `horizon-client`'s, the `options` it accepts. Pass only listed
options: any other is refused with exit code 2. Readers ignore fields they don't know.

## 3. Have the owner's game files and a Java

Preparing reads, for the `minecraft` version above:

- its **version JSON**, **client jar**, **libraries** (Maven layout) and **assets** (`indexes/` and `objects/`).
  The official launcher keeps them under one game folder (`--minecraft-dir`); a launcher that keeps them
  elsewhere names each one (`--version-json`, `--client-jar`, `--libraries-dir`, `--assets-dir`);
- the **server jar**, the first time only. No launcher keeps it: download it from the version JSON's
  `downloads.server` for the signed-in owner, as `horizon-launcher` does;
- a **Java** of `java_major`. A Java your launcher manages goes in with `--java-home`.

Every file is checked against the pins in `research-lock.json`, so a wrong or damaged one is refused, not used.

## 4. Prepare the resources

Prepare into a folder of your choosing, with machine-readable progress:

```sh
horizon-prepare --prepare --server-jar SERVER_JAR --minecraft-dir GAME_DIR \
    --output PREPARED --complete --progress json
```

- `--complete` also copies the sounds and fails unless the result would pass `--check`, so the game has
  nothing missing.
- With `--progress json` every stdout line is one JSON object: `{"message"}` to show, `{"fraction"}` from 0 to 1
  for a progress bar, `{"error"}` as the last line when it fails (exit code 1). stderr is human text.
- The first preparation takes minutes; later ones skip what is already current. One preparation at a time
  writes to a folder; a second one waits for the first.
- The resources depend on the build and on the Minecraft version, not on the instance: one `PREPARED` folder
  serves every instance of that build.

## 5. Before every start: check

```sh
horizon-prepare --check --output PREPARED
```

Exit 0 means the resources are complete and current for this build. Exit 1 lists each gap on stdout with the
command that fills it: prepare again (step 4). It is fast and reads no game files, so run it before every start
and after installing a new build: a build that writes a newer layout (its `schema` or `output_version` in step 2)
makes older resources fail the check.

## 6. Start the game

One folder per instance, and the owner's session on standard input:

```sh
horizon-client --resources PREPARED --game-dir INSTANCE --home INSTANCE --width 1280 --height 720 --session-stdin
```

- Write the [session JSON](launch.md#the-session-json) (`name`, `uuid`, `access_token`, `user_type: "msa"`,
  `xuid` optional) to the game's standard input and close it. Never put a token on the command line, where other programs
  can read it. `--session-file` is the alternative: the game reads the file once and deletes it.
- `--game-dir` holds saves, `options.txt`, resource packs, mods and screenshots; `--home` holds `logs/latest.log`
  and `crash-reports/`. `--quickPlaySingleplayer WORLD` or `--quickPlayMultiplayer HOST[:PORT]` opens a world or a
  server at once.
- A session the game can't read is reported on stderr (`Horizon: not signed in: …`); the game still starts, and
  online-mode servers refuse the player.

## 7. When it ends

| Exit code | Meaning | What your launcher does |
| --- | --- | --- |
| 0 | The player quit | Nothing |
| 1 | Stopped on an error | Show the last stderr line: `Horizon: crash report saved to <path>` |
| 2 | Refused command line | Show stderr: it names the option (`Horizon: --width: …`); a bug in your launcher |
| 101 | Panicked | As 1 |

## Updating

A new build is a new folder: ask it (step 2), check the resources with it (step 5), prepare again if the check
asks (step 4), then start it (step 6). Keep the old folder until the new one has started once, so the player can
go back while its `nightly-<date>` release still exists.
