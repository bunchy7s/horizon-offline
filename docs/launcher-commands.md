# Launcher commands

<!-- Generated from launcher/src/cli.rs by cli_tools::markdown(); a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test --manifest-path launcher/Cargo.toml the_launcher_reference -->

Every command of `horizon-launcher`, and the tool `horizon-launcher mcp` offers for it. A tool's arguments are the command's, in snake_case. A tool runs the command with `--json` and returns what it printed. The tools that change the player's things for good need `confirm: true`. The tools that send something or reach other people say "only when the player asks". The game's own tools, `game_*`, are in [horizon-ctl-commands.md](horizon-ctl-commands.md). With No AI on, `mcp` and `agent-config` are hidden and refuse; every other command stays. See [agent-control.md](agent-control.md#no-ai).

## `horizon-launcher completions`

Write a shell completion script to stdout.

    horizon-launcher completions <SHELL>

- `SHELL` (bash, elvish, fish, powershell, zsh)

MCP: none: the player's switch.

## `horizon-launcher versions`

List Horizon releases, or the installed versions.

    horizon-launcher versions

- `--installed`: Only the versions installed here (no network)

MCP: `versions`.

## `horizon-launcher install`

Download and install a release (a tag such as v2026.10.13, or "latest").

    horizon-launcher install <TAG>

- `TAG`

MCP: `install`.

## `horizon-launcher remove`

Remove an installed version.

    horizon-launcher remove <TAG>

- `TAG`

MCP: `remove`, needs `confirm: true`.

## `horizon-launcher prepare`

Prepare a version's resources from your own Minecraft install, or from Mojang.

    horizon-launcher prepare

- `--minecraft`: The game folder of the official launcher, Prism Launcher, MultiMC, the Modrinth App, ATLauncher or GDLauncher (default: the launcher's, then every launcher's default)
- `--download`: Download Minecraft from Mojang for the signed-in account that owns it, instead of using an install

MCP: `prepare`.

## `horizon-launcher play`

Start the client (needs a signed-in Microsoft account that owns Minecraft: Java Edition).

    horizon-launcher play

- `--instance`: The instance to play (its name or id; default: the selected one). A Java instance starts its Minecraft: Java Edition, downloaded with its Java the first time
- `--world`: Open or create this world (a folder under saves) at once
- `--creative`: Creative mode for a new world
- `--join`: Join a friend's world with its invite code
- `--server`: Join this server at once: an address, or a server of the list by name or position
- `--arg`: An extra argument for the game, passed through as-is (repeatable), for example --server-thread
- `--account`: Play this launch as this signed-in account (its name or UUID), without changing the one `account select` picked: several games can play as several accounts at once

MCP: `play`, then the `game_*` tools.

## `horizon-launcher server`

Run a dedicated server here; type commands into its console.

    horizon-launcher server

- `--dir`: The server's folder (default: the one the launcher uses)
- `--accept-eula`: Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)

MCP: `server`, `server_command`, `server_log`, `server_stop`.

## `horizon-launcher native-build`

Native mods (docs/native-mods.md): compile mixin bundles on the Horizon revision they pin, else the one the instance plays, with the launcher's own Rust, as the version the instance then plays. Not sandboxed: the mods' code runs with your rights.

    horizon-launcher native-build [FOLDER]

- `FOLDER`: A mod folder (horizon-mixin.toml, or an unpacked .hnmod), or a folder of them (default: the native mods installed and turned on in the instance)
- `--server-output`: Export mode: build a standalone dedicated-server engine into this folder instead of an instance's build. Needs a mod FOLDER; no instance is read or changed, and no launcher config is written
- `--instance`: The instance whose installed native mods to build, and which then plays the build (default: the selected one)
- `--update`: Move to the newest installed release or Nightly instead of the Horizon the instance plays (a full rebuild)
- `--trust-native-code`: You read the mods' source and trust it with full access to this computer
- `--allow-version-mismatch`: Build mods made for different Horizon versions together anyway, on the newest of them
- `--profile` (fast, release): The Cargo profile: fast (the default: incremental, quick to rebuild) or release (thin LTO: slower to build, slightly faster game). Default: the native-profile setting

MCP: none: the player's switch.

## `horizon-launcher notes`

Show a release's patch notes (default: the newest).

    horizon-launcher notes [TAG]

- `TAG`

MCP: `notes`.

## `horizon-launcher links`

Show the Discord, sponsor and website links.

    horizon-launcher links

MCP: `links`.

## `horizon-launcher account list`

The signed-in accounts and which one plays.

    horizon-launcher account list

MCP: `account_list`.

## `horizon-launcher account sign-in`

Sign in with a Microsoft account in your browser (device code). Play needs one that owns Minecraft: Java Edition.

    horizon-launcher account sign-in

- `--for`: Sign this account (name or UUID) in again: another Microsoft account is refused and who plays stays the same

MCP: `account_sign_in`.

## `horizon-launcher account refresh`

Renew an account's session now, even when its access token is still fresh.

    horizon-launcher account refresh <WHO>

- `WHO`

MCP: `account_refresh`.

## `horizon-launcher account session`

Renew an account's session for a game already running: prints the read-once session file it hands over (`--json`: {"session_file"} or {"authentication"} when it can't).

    horizon-launcher account session <WHO>

- `WHO`

MCP: none: the game's own session handoff.

## `horizon-launcher account sign-out`

Sign out: the account's tokens leave the keychain (and the secrets file, where it kept them).

    horizon-launcher account sign-out <WHO>

- `WHO`: Its name or UUID

MCP: `account_sign_out`, needs `confirm: true`.

## `horizon-launcher account select`

Play as this account, or as one of your offline names.

    horizon-launcher account select <WHO>

- `WHO`

MCP: `account_select`.

## `horizon-launcher account offline`

Play offline (LAN, offline-mode servers) beside a signed-in account that owns the game: as NAME (kept to switch back to; 3 to 16 letters, digits or _), or as the name the game made up.

    horizon-launcher account offline [NAME]

- `NAME`

MCP: `account_offline`.

## `horizon-launcher account forget-offline`

Forget one of your offline names; its skin and cape go with it.

    horizon-launcher account forget-offline <NAME>

- `NAME`

MCP: `account_forget_offline`.

## `horizon-launcher account skin`

Show or set an offline name's skin and cape: a 64x64 (or 64x32) skin PNG, a 64x32 cape PNG, the model. Kept by the launcher and given to the game when that name plays.

    horizon-launcher account skin <NAME> [FILE]

- `NAME`
- `FILE`: The skin's PNG
- `--model`: classic or slim arms
- `--cape`: The cape's PNG
- `--no-skin`: Back to the default skin
- `--no-cape`: Take the cape off

MCP: `account_skin`.

## `horizon-launcher settings`

Show the launcher's settings, or set one (the Settings page): selected-version TAG, minecraft-dir PATH, server-dir PATH ("default" resets a folder), install-new-versions on|off, prune-old-versions on|off (delete old versions after an install, keeping the selected one, the ones instances use and the newest few), keep-versions N (how many of the newest to keep; "default" is 2), opening-animation on|off, proxy http://host:port ("default" removes it), while-playing stay|minimize|hide (what the window does while the game runs), language system|en|fr|ru|de|zh|es|uk|pl (the window's language; system follows the computer's), accent violet|blue|teal|green|amber|red|pink, background PATH (a picture behind Home; "default" shows Horizon's landscape), native-profile fast|release (the Cargo profile native mods build with: fast is incremental, release is slower to build and slightly faster to play).

    horizon-launcher settings [KEY] [VALUE]

- `KEY` (selected-version, minecraft-dir, server-dir, install-new-versions, prune-old-versions, keep-versions, opening-animation, proxy, while-playing, language, accent, background, native-profile)
- `VALUE`: The new value (with KEY)

MCP: `settings`.

## `horizon-launcher game-args`

Show or set the extra arguments the game starts with (for example --server-thread); quotes group a word with spaces in it.

    horizon-launcher game-args [ARGS]

- `ARGS`: Omit to show the current arguments

MCP: `game_args`.

## `horizon-launcher share-log`

Upload an instance's latest.log to mclo.gs (a paste service for Minecraft logs) and print its link, for a bug report: only when the player asks. The home folder, the user name and the account names are taken out first; the link expires after 90 days unread.

    horizon-launcher share-log

- `--instance`: The instance (default: the selected one)

MCP: `share_log`, only when the player asks.

## `horizon-launcher unshare-log`

Delete a log shared with share-log: the "<id>:<token>" it printed.

    horizon-launcher unshare-log <SHARED>

- `SHARED`

MCP: `unshare_log`, needs `confirm: true`.

## `horizon-launcher logs`

Print the end of the game's log (logs/latest.log in the game folder), or its newest crash report.

    horizon-launcher logs

- `--lines`: How many lines of latest.log
- `--crash`: The newest crash report instead

MCP: `logs`.

## `horizon-launcher update`

Look for a newer Horizon release and launcher, and install them.

    horizon-launcher update

- `--check`: Only report what is available

MCP: `update`.

## `horizon-launcher desktop-entry`

Linux: put Horizon in the applications menu (an entry and the icon, under $XDG_DATA_HOME), or take it out with --remove.

    horizon-launcher desktop-entry

- `--remove`: Remove the entry and the icon, and stop adding them back

MCP: `desktop_entry`.

## `horizon-launcher data status`

What is allowed, the install ID and how many items wait (the default).

    horizon-launcher data status

MCP: `data_status`.

## `horizon-launcher data allow`

Turn kinds on: performance, crashes, bugs, benchmarks, or all.

    horizon-launcher data allow <KINDS>...

- `KINDS`

MCP: `data_allow`, only when the player asks.

## `horizon-launcher data deny`

Turn kinds off: performance, crashes, bugs, benchmarks, or all.

    horizon-launcher data deny <KINDS>...

- `KINDS`

MCP: `data_deny`.

## `horizon-launcher data show`

Print exactly what would be sent.

    horizon-launcher data show

MCP: `data_show`.

## `horizon-launcher data delete`

Delete everything sent from here, and what waits, and take a new install ID.

    horizon-launcher data delete

MCP: `data_delete`, needs `confirm: true`.

## `horizon-launcher data send`

Send what is waiting now.

    horizon-launcher data send

MCP: `data_send`, only when the player asks.

## `horizon-launcher data bug`

Send a bug report you write; it is printed first, as it would leave.

    horizon-launcher data bug <TEXT>

- `TEXT`
- `--log`: Add the end of the game's log (scrubbed, without chat)
- `--yes`: Send without asking

MCP: `data_bug`, only when the player asks.

## `horizon-launcher data benchmark`

Upload a benchmark report (docs/benchmark-report.md); it is printed first.

    horizon-launcher data benchmark <FILE>

- `FILE`
- `--yes`

MCP: `data_benchmark`, only when the player asks.

## `horizon-launcher instance list`

The instances, each with its game folder (* = the one Play starts).

    horizon-launcher instance list

MCP: `instance_list`.

## `horizon-launcher instance create`

Make an instance: empty but for your options and name, or a copy of another one's worlds, options and packs.

    horizon-launcher instance create <NAME>

- `NAME`
- `--copy`: The instance to copy (its name or id)
- `--minecraft`: Make a Minecraft: Java Edition instance of this version (1.21.4, latest or latest-snapshot) instead of a Horizon one
- `--loader`: With this mod loader: fabric or quilt
- `--loader-version`: The loader's version (default: its newest stable one)

MCP: `instance_create`.

## `horizon-launcher instance versions`

The versions of Minecraft: Java Edition a Java instance can play, the newest first.

    horizon-launcher instance versions

- `--snapshots`: Snapshots too
- `--old`: Old betas and alphas too

MCP: `instance_versions`.

## `horizon-launcher instance java`

A Java instance's Java: its own Java home or the automatic pick, memory and JVM arguments.

    horizon-launcher instance java <INSTANCE>

- `INSTANCE`
- `--home`: A Java home to run it on
- `--auto`: Go back to the Java the version needs, picked or downloaded by the launcher
- `--memory`: The most memory the game may use, in MiB
- `--args`: Extra JVM arguments (one string; "" clears them)
- `--window`: The game window's size, as 1280x720; "default" goes back to the game's own

MCP: `instance_java`.

## `horizon-launcher instance loader`

A Java instance's mod loader: fabric or quilt, or none. --list shows its versions.

    horizon-launcher instance loader <INSTANCE> <LOADER>

- `INSTANCE`
- `LOADER`
- `--list`: List the loader's versions for the instance's Minecraft instead

MCP: `instance_loader`.

## `horizon-launcher instance rename`

Rename an instance (its folder stays).

    horizon-launcher instance rename <INSTANCE> <NAME>

- `INSTANCE`
- `NAME`

MCP: `instance_rename`.

## `horizon-launcher instance delete`

Delete an instance: its folder (worlds, options, packs) goes to the Trash, where you can get it back. Horizon's own instance stays.

    horizon-launcher instance delete <INSTANCE>

- `INSTANCE`

MCP: `instance_delete`.

## `horizon-launcher instance icon`

Set an instance's icon: a block or item name from your prepared files (grass_block), a colour (#3fa7d6), or none.

    horizon-launcher instance icon <INSTANCE> <ICON>

- `INSTANCE`
- `ICON`

MCP: `instance_icon`.

## `horizon-launcher instance group`

Put an instance in a sidebar group, or take it out with no group.

    horizon-launcher instance group <INSTANCE> [GROUP]

- `INSTANCE`
- `GROUP`

MCP: `instance_group`.

## `horizon-launcher instance select`

Make Play start this instance.

    horizon-launcher instance select <INSTANCE>

- `INSTANCE`

MCP: `instance_select`.

## `horizon-launcher instance version`

Keep an instance on one installed Horizon version, or "latest" to follow updates again.

    horizon-launcher instance version <INSTANCE> <VERSION>

- `INSTANCE`

MCP: `instance_version`.

## `horizon-launcher instance export`

Export an instance (or a port) to share: its settings, packs, Horizon mods and the worlds named, as a .mrpack (resource packs from Modrinth listed by download) or a zip. Never logs, screenshots or account data; worlds go without their per-player files.

    horizon-launcher instance export [INSTANCE]

- `INSTANCE`: The instance (default: the selected one)
- `--format` (mrpack, zip)
- `--world`: A world to put in it (its name or folder); repeat for more
- `--out`: Where to write it (default: "<name>.mrpack" in the current folder)

MCP: `instance_export`.

## `horizon-launcher instance launch`

Show or set what runs around an instance's game: a shell command before it (the game starts only if it succeeds), a wrapper it runs through (`nice -n 5`), environment variables ("NAME=value NAME2=value") and a shell command after it exits. An empty value clears one. The game and the commands also get INST_NAME, INST_ID, INST_DIR, INST_MC_DIR and, for a Java instance, INST_JAVA. On Linux, also GameMode, MangoHud and the dedicated graphics card, when the computer has them.

    horizon-launcher instance launch <INSTANCE>

- `INSTANCE`
- `--pre-launch`
- `--wrapper`
- `--post-exit`
- `--env`
- `--gamemode`: Linux: run it through GameMode (gamemoderun), on or off
- `--mangohud`: Linux: MangoHud's overlay, on or off
- `--dedicated-gpu`: Linux: the dedicated graphics card (DRI_PRIME, or NVIDIA's offload), on or off

MCP: `instance_launch`, needs `confirm: true`.

## `horizon-launcher instance shortcut`

Put a shortcut on the desktop that plays an instance (`play --instance`): a .desktop file (also in the applications menu) on Linux, an app on macOS, a .lnk on Windows.

    horizon-launcher instance shortcut [INSTANCE]

- `INSTANCE`: The instance (default: the selected one)
- `--remove`: Move the instance's shortcuts to the trash instead

MCP: `instance_shortcut`.

## `horizon-launcher instance import`

Import what a file is, as dropping it on the window does: a shared instance (a .mrpack without mod loaders, such as `instance export` writes, or a game folder zip) becomes a new instance; a Java modpack goes to the Modpacks import; a resource or shader pack into the instance (default: the selected one).

    horizon-launcher instance import <FILE>

- `FILE`
- `--instance`: The instance a pack goes into

MCP: `instance_import`.

## `horizon-launcher instance others`

List the instances other launchers keep (Prism Launcher, MultiMC, the official launcher, the CurseForge app, the Modrinth App), read without changing anything there.

    horizon-launcher instance others

- `--folder`: Look in this folder instead: a launcher's folder, its instances/ or one instance

MCP: `instance_others`.

## `horizon-launcher instance copy-from`

Copy another launcher's instance into a new Java instance: its Minecraft version and its game folder (worlds, mods, packs, options); the other launcher's copy stays as it is.

    horizon-launcher instance copy-from <NAME>

- `NAME`: Its name, as `instance others` lists it
- `--folder`: Look in this folder instead: a launcher's folder, its instances/ or one instance
- `--as`: The new instance's name (default: the same)
- `--minecraft`: The Minecraft version it plays, when its launcher doesn't say (the CurseForge app, the Modrinth App)

MCP: `instance_copy_from`.

## `horizon-launcher worlds list`

The worlds: name, mode, version, last played and size (the default).

    horizon-launcher worlds list

- `--instance`: The instance (default: the selected one)

MCP: `worlds_list`.

## `horizon-launcher worlds backup`

Zip a world into the instance's backups/ folder, as the game's Make Backup does.

    horizon-launcher worlds backup <WORLD>

- `WORLD`: Its folder or name
- `--instance`

MCP: `worlds_backup`.

## `horizon-launcher worlds copy`

Copy a world beside it, as "<name> (1)".

    horizon-launcher worlds copy <WORLD>

- `WORLD`
- `--instance`

MCP: `worlds_copy`.

## `horizon-launcher worlds delete`

Move a world to the system's trash.

    horizon-launcher worlds delete <WORLD>

- `WORLD`
- `--instance`

MCP: `worlds_delete`, needs `confirm: true`.

## `horizon-launcher worlds datapack-list`

A world's data packs, and which are on.

    horizon-launcher worlds datapack-list <WORLD>

- `WORLD`
- `--instance`

MCP: `worlds_datapack_list`.

## `horizon-launcher worlds datapack-add`

Copy a data pack zip into a world (the game turns it on when the world opens).

    horizon-launcher worlds datapack-add <WORLD> <FILE>

- `WORLD`
- `FILE`
- `--instance`

MCP: `worlds_datapack_add`.

## `horizon-launcher worlds datapack-remove`

Move a world's data pack to the system's trash.

    horizon-launcher worlds datapack-remove <WORLD> <PACK>

- `WORLD`
- `PACK`: Its file name, with or without .zip
- `--instance`

MCP: `worlds_datapack_remove`, needs `confirm: true`.

## `horizon-launcher packs list`

The resource and shader packs, and which are on (the default).

    horizon-launcher packs list

- `--kind` (resource, shader): Only this kind
- `--instance`: The instance (default: the selected one)

MCP: `packs_list`.

## `horizon-launcher packs add`

Copy a pack file in: a resource pack .zip, a shader pack .zip or a .hshader.

    horizon-launcher packs add <FILE>

- `FILE`
- `--instance`

MCP: `packs_add`.

## `horizon-launcher packs on`

Turn a pack on (a resource pack goes on top; a shader pack replaces the one that was on).

    horizon-launcher packs on <PACK>

- `PACK`: Its file name or title
- `--kind` (resource, shader)
- `--instance`

MCP: `packs_on`.

## `horizon-launcher packs off`

Turn a pack off.

    horizon-launcher packs off <PACK>

- `PACK`
- `--kind` (resource, shader)
- `--instance`

MCP: `packs_off`.

## `horizon-launcher packs remove`

Turn a pack off and move it to the system's trash.

    horizon-launcher packs remove <PACK>

- `PACK`
- `--kind` (resource, shader)
- `--instance`

MCP: `packs_remove`, needs `confirm: true`.

## `horizon-launcher java list`

The Java runtimes Horizon downloaded from Mojang and the ones installed on this computer.

    horizon-launcher java list

MCP: `java_list`.

## `horizon-launcher java install`

Download Mojang's Java runtime: a major (8, 16, 17, 21, 25) or a component name from a version's JSON (java-runtime-delta).

    horizon-launcher java install <JAVA>

- `JAVA`

MCP: `java_install`.

## `horizon-launcher screenshots list`

The screenshots, the newest first, with their files (the default).

    horizon-launcher screenshots list

- `--instance`: The instance (default: the selected one)

MCP: `screenshots_list`.

## `horizon-launcher screenshots delete`

Move a screenshot to the system's trash.

    horizon-launcher screenshots delete <SCREENSHOT>

- `SCREENSHOT`: Its file name, with or without .png
- `--instance`

MCP: `screenshots_delete`, needs `confirm: true`.

## `horizon-launcher multiplayer list`

The servers of the game's Multiplayer screen (the default).

    horizon-launcher multiplayer list

- `--instance`: The instance (default: the selected one)

MCP: `multiplayer_list`.

## `horizon-launcher multiplayer add`

Add a server to the list.

    horizon-launcher multiplayer add <ADDRESS>

- `ADDRESS`: Its address, as in the game's Add Server (host or host:port)
- `--name`: Its name in the list (default: Minecraft Server)
- `--instance`

MCP: `multiplayer_add`.

## `horizon-launcher multiplayer remove`

Remove a server from the list: its position from 1, name or address.

    horizon-launcher multiplayer remove <SERVER>

- `SERVER`
- `--instance`

MCP: `multiplayer_remove`, needs `confirm: true`.

## `horizon-launcher modpack list`

The imported packs and their coverage (the default).

    horizon-launcher modpack list

MCP: `modpack_list`.

## `horizon-launcher modpack import`

Import a .mrpack, a CurseForge zip or an instance folder, and write its coverage report.

    horizon-launcher modpack import <PATH>

- `PATH`

MCP: `modpack_import`.

## `horizon-launcher modpack port`

Get the jars of what this computer can port and carry them over.

    horizon-launcher modpack port <PACK>

- `PACK`: The pack's folder name, as list shows it
- `--from`: A folder with your own jars (an installed instance, a downloads folder)

MCP: `modpack_port`.

## `horizon-launcher modpack play`

Play a ported pack as its own profile.

    horizon-launcher modpack play <PACK>

- `PACK`

MCP: `modpack_play`.

## `horizon-launcher modpack remove`

Remove an imported pack: its jars, ports and the profile's saves.

    horizon-launcher modpack remove <PACK>

- `PACK`

MCP: `modpack_remove`, needs `confirm: true`.

## `horizon-launcher mods list`

What the browser installed in an instance (the default).

    horizon-launcher mods list

- `--instance`: The instance (default: the selected one)

MCP: `mods_list`.

## `horizon-launcher mods search`

Search for packs Horizon loads, or for what a Java instance runs (made for its Minecraft version and loader).

    horizon-launcher mods search [TEXT]...

- `TEXT`
- `--kind` (mod, resource, modpack, shader, datapack)
- `--everything`: Also what was made for other versions or runs only partly, each marked
- `--source` (modrinth, curseforge, horizon)
- `--instance`: The instance (default: the selected one)

MCP: `mods_search`.

## `horizon-launcher mods check`

Say how well a Java mod runs in Horizon: its jar is downloaded (checksum checked), carried over and read; the answer is kept.

    horizon-launcher mods check <PROJECT>

- `PROJECT`
- `--everything`: Allow a version made for another Minecraft version (1.21 and later)
- `--source` (modrinth, curseforge, horizon)

MCP: `mods_check`.

## `horizon-launcher mods install`

Install a pack and what it needs into an instance, by its id or slug (checksums checked); a Java mod is ported into horizon-mods and turned on (a Java instance gets the jar made for its version and loader in mods/); a modpack is imported as an instance of its own; a data pack goes into the world `--world` names.

    horizon-launcher mods install <PROJECT>

- `PROJECT`
- `--world`: Install the project's data pack into this world of the instance (its name or folder)
- `--instance`: The instance (default: the selected one)
- `--everything`: Allow a version made for another Minecraft version
- `--source` (modrinth, curseforge, horizon)

MCP: `mods_install`.

## `horizon-launcher mods update`

Update what the browser installed (or one pack).

    horizon-launcher mods update [PROJECT]

- `PROJECT`
- `--check`: Only say what is newer
- `--instance`: The instance (default: the selected one)

MCP: `mods_update`.

## `horizon-launcher mods add-github`

Install a native mod from a GitHub link (docs/native-mods.md, "Install from GitHub"): https://github.com/OWNER/REPO (its latest release, else its default branch), .../releases/tag/TAG, .../tree/REF or .../commit/SHA. Resolved to an exact release file or commit, checked like a registry mod. NOT reviewed by the Horizon registry.

    horizon-launcher mods add-github <URL>

- `URL`
- `--mod`: The mod to install when the link holds several
- `--all`: Install every mod the link holds
- `--instance`: The instance (default: the selected one)
- `--trust-native-code`: You read the mod's source (the link printed) and trust it with full access to this computer

MCP: none: the player's switch.

## `horizon-launcher mods update-github`

Look for newer versions of the native mods installed from GitHub (the latest release, or the default branch's head for a mod installed from a commit), and install them.

    horizon-launcher mods update-github [ID]

- `ID`: One mod (default: all of them)
- `--check`: Only say what is newer
- `--instance`: The instance (default: the selected one)
- `--trust-native-code`: You read the new source and trust it with full access to this computer

MCP: none: the player's switch.

## `horizon-launcher mods remove`

Remove a pack the browser installed.

    horizon-launcher mods remove <PROJECT>

- `PROJECT`
- `--instance`: The instance (default: the selected one)

MCP: `mods_remove`, needs `confirm: true`.

## `horizon-launcher mods enable`

Turn on a mod the browser installed (horizon-mods.txt, as the game's Options > Mods does; in a Java instance, its file loses `.disabled`).

    horizon-launcher mods enable <PROJECT>

- `PROJECT`
- `--instance`: The instance (default: the selected one)

MCP: `mods_enable`.

## `horizon-launcher mods disable`

Turn off a mod the browser installed; it stays installed (in a Java instance, as `<file>.disabled`).

    horizon-launcher mods disable <PROJECT>

- `PROJECT`
- `--instance`: The instance (default: the selected one)

MCP: `mods_disable`.

## `horizon-launcher rust status`

Whether it is installed, its version and whether this computer can link with it.

    horizon-launcher rust status

MCP: `rust_status`.

## `horizon-launcher rust install`

Download the pinned toolchain from static.rust-lang.org (once), checked by SHA-256.

    horizon-launcher rust install

MCP: `rust_install`.

## `horizon-launcher rust remove`

Delete the toolchain and the native builds' Cargo home.

    horizon-launcher rust remove

MCP: `rust_remove`.

## `horizon-launcher servers list`

The servers, their folder and whether they run.

    horizon-launcher servers list

MCP: `servers_list`.

## `horizon-launcher servers create`

Create a server; with no options it is a new world, survival, normal, 10 players, friends only.

    horizon-launcher servers create

- `--name`
- `--seed`: The new world's seed (default: random)
- `--copy-world`: Start from a copy of this world folder (one of your saves)
- `--mode` (survival, creative, adventure, spectator)
- `--difficulty` (peaceful, easy, normal, hard)
- `--max-players`
- `--open`: Let anyone in (no whitelist)
- `--offline`: Do not check players' Minecraft accounts (offline mode: anyone can use any name)
- `--player`: Who may join (repeatable); the first becomes an operator
- `--accept-eula`: Agree to the Minecraft EULA (https://aka.ms/MinecraftEULA)

MCP: `servers_create`, only when the player asks.

## `horizon-launcher servers set-up`

Copy an installed, prepared version's programs and game files for the servers (again).

    horizon-launcher servers set-up

MCP: `servers_set_up`.

## `horizon-launcher servers run`

Run a horizon-server command on a server, for example: servers run "My server" -- status.

    horizon-launcher servers run <SERVER> -- <ARGS>...

- `SERVER`: The server's name or folder
- `ARGS`

MCP: `servers_run`.

## `horizon-launcher servers forget`

Take a server off the list (its folder stays).

    horizon-launcher servers forget <SERVER>

- `SERVER`

MCP: `servers_forget`, needs `confirm: true`.

## `horizon-launcher mcp`

Serve the Model Context Protocol on stdin and stdout, for AI agents (refused while No AI is on).

    horizon-launcher mcp

MCP: none: this command is the MCP server.

## `horizon-launcher agent-config`

How to connect an AI agent (Claude Code, Codex, Cursor, any MCP client) to `mcp`: the config to paste and the command that adds it (refused while No AI is on).

    horizon-launcher agent-config

- `--client` (claude-code, codex, cursor, generic): Only this client
- `--test`: Start the MCP server as a client would and say whether it answered

MCP: none: it prints how to connect a client to `mcp`.

## `horizon-launcher no-ai`

Show or set No AI: on hides and turns off every AI feature (the companion, agent control, horizon-ctl and both MCP servers); off, the default, makes them available.

    horizon-launcher no-ai [STATE]

- `STATE` (on, off): Omit to show the current state

MCP: none: the player's switch.

## `horizon-launcher benchmark`

Measure the launcher: starts (the first is the cold one), page switching, idle CPU and memory, the release list fetch; writes a benchmark report to benchmarks/ in its data folder.

    horizon-launcher benchmark

- `--repeats`: How many times to start it

MCP: `benchmark`.
