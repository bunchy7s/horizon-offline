# Survival multiplayer (SMP): the official server's mod

The official Horizon server (play.playhorizon.dev) wants an organised survival game: a currency players earn in
game, a server market, safe player-to-player trades, small games of chance with that currency, and spawn, homes,
teleport requests, warps, land claims and rules.

**None of it is in Horizon.** The server people download stays a plain server, like vanilla's server jar: no SMP
command, no SMP setting in `server.toml`, nothing in the release build. The SMP is a **native mod**, `smp`, that
only the official server builds into its engine. What Horizon gains are general **server hook points** any native
mod can use (commands, players, messages, inventories, the world folder, then interactions, damage, menus): the
mod API grows, and a server owner who wants their own SMP, shop or claims mod can write it on the same points.

Status: design (this page); built in phases below.

## Players install nothing

Everything works with a stock Horizon client: no mod, no resource pack, nothing on the player's side. The SMP speaks
only what vanilla's protocol already carries and the Horizon client already renders:

- chat commands with completions (the server's command tree), and chat lines;
- menus the server opens with vanilla menu types (a 6-row chest, `generic_9x6`) and fills with ordinary items as
  buttons and displays; the server rules every click, the client only shows them;
- vanilla sounds, titles, the action bar, boss bars and the scoreboard sidebar for what a player should notice.

Only the Horizon client may join the official server; that gate is anticheat-1's work, not the SMP's. The SMP may rely
on what the stock client renders, never on a client mod.

## Architecture

| Part | Where | What |
|---|---|---|
| Server hook points | this repository, `crates/horizon-client/src/game/native_hooks.rs` ([native-mods.md](native-mods.md)) | stable, versioned (`NATIVE_HOOKS_API`), empty when no mod hooks them |
| The `smp` mod | [horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods), `native/smp/` | a horizon-mixins bundle: `smp_store.rs`, `smp_economy.rs`, `smp_market.rs`... hold the rules and data (also the mod's own crate, with its tests, no game types), `glue.rs` and small helpers hook the points |
| The official server's engine | built from a Horizon release's commit with the bundle applied | `horizon-server`'s `[engine] path` points at it; the rest of the server is unchanged |

Why a native mod: it is the mod system Horizon has now, it runs on the server
that simulates the world, and it keeps the SMP out of everyone else's game. Why hook points rather than reaching into
the game's private code: a mod on the hook points builds on any later Horizon (`"hooks": N`), so the official server
updates with each release without the mod being ported again.

Why not a datapack: no command arguments, no inventory transactions, no trade window, no way to refuse an action in a
claim.

### Hook points the SMP needs

Each is added in its own pull request, raising `NATIVE_HOOKS_API`, documented in [native-mods.md](native-mods.md),
and free when no mod hooks it (an empty function). Where a contributor's pull request already adds one (block
breaking and placing, item use: #4483, #4302), the SMP uses theirs.

| Phase | API | Hook points and helpers |
|---|---|---|
| 1. Economy | 8 | `native_register_commands` (commands every player's chat box completes), `native_perform_command` (a player's command on the server, with that player's state as the acting player); helpers: the online players and their game mode, a line to one player or to all, the world's folder |
| 1. Market | 9 | the acting player's items: held, count, carried, take, room, give (plain stacks only, nothing dropped) |
| 2. Trading | 10 | a vanilla chest menu the server opens and fills for a player, display only: its clicks and its closing go to the mod; an atomic exchange of two players' items; a player left or died |
| 3. Gambling | nothing new |
| 4. Organisation | 21 | a right click on a block that a mod can refuse (beside #4302's item use and #4483's block breaking), the acting player, a command run as the server (teleports across levels with `execute in … run tp`), a player's health (a teleport's warmup stops when it drops) |
| 4. Claims | 22 | the world kept out of claims: fluid and fire spreading, pistons, explosions' blocks, buckets, a player's blow on an entity (frames, armor stands, animals); right-clicking an entity is #4648's |
| 5. Public server polish | 49 | a sidebar and Tab list lines for one player (the richest players, the server's TPS), the server's tick times, AFK players out of the sleep count, where players look |

## Mojang's rules

The currency is earned in game only (the market, trades, playing). Rules that no setting changes:

- **in-game currency only**: games of chance take and pay only the SMP's currency, within limits the owner sets;
- **never bought with real money**: no store, rank or perk sells the currency, a stake or a ticket;
- **never cashed out**: nothing turns the currency or winnings into money or anything outside the game;
- **no paid random items**: nothing random (crates, keys, loot boxes) is sold, for money or for the currency bought
  with money.

Games of chance are off in the mod by default (`gambling.enabled: false`): a server that builds the mod turns them
on itself. The official server has them on (Ludovic's decision, 2026-10-30). Operators' corrections (`/eco`) are
logged like every other transaction.

## Commands

Player commands work for every player; operator commands need the permission level shown.

| Command | Does |
|---|---|
| `/balance [player]`, `/bal` | your balance, or another player's |
| `/pay <player> <amount>` | gives money to a player who has an account |
| `/baltop [page]` | the richest players |
| `/eco give\|take\|set <player> <amount>` (level 3) | operator corrections, logged |
| `/sell hand [count]\|all\|<item> [count]`, `/buy <item> [count]`, `/worth [item]` | the server market, at the configured prices; plain items only, in survival and adventure |
| `/trade <player>`, `/trade accept\|deny\|cancel`, `/trade money <amount>` | a two-sided trade window |
| `/coinflip <amount> [player]`, `/coinflip accept <player>`, `/coinflip cancel\|list` | a 50/50 bet between two players, both stakes held until it is decided |
| `/lottery [buy <tickets>]` | tickets for the next draw; the pot goes to one ticket drawn at random |
| `/spawn`, `/setspawn` (level 2) | the server's spawn |
| `/sethome [name]`, `/home [name]`, `/delhome <name>`, `/homes` | up to `homes.max` homes |
| `/tpa <player>`, `/tpahere <player>`, `/tpaccept`, `/tpdeny` | teleport requests |
| `/warp [name]`, `/setwarp <name>`, `/delwarp <name>` (level 2) | warps set by operators |
| `/claim [radius]`, `/unclaim`, `/trust <player>`, `/untrust <player>`, `/claims` | land claims |
| `/rules` | the server's rules |

Teleports wait `teleport.warmup_seconds` and are cancelled when the player moves more than half a block or takes
damage.

**Trades.** `/trade Alex` asks Alex; once accepted, both see a 6-row chest menu: their own offer on the left, the
other's on the right, the money each offers, and confirm and cancel buttons (ordinary items such as lime and red
stained glass panes). The menu is display only: a player adds a stack to their offer by clicking it in their own
inventory, below the menu, and takes it back by clicking it in the offer. **Nothing moves until the swap**: the
offered stacks stay in their owners' inventories, the menu shows copies. Any change to either offer clears both
confirmations; when both have confirmed, the server checks that each player still holds exactly what they offer and
has room for what they get, then swaps the items and the money in one step of its tick, or swaps nothing. Closing the
menu, `/trade cancel`, disconnecting, dying or the other player leaving ends the trade with nothing moved, so there is
no item to return and nothing to duplicate. Tests cover each way out.

**Claims.** A rectangle of columns in one dimension, owned by a player who trusts others by name. In a claim, players
who are neither the owner, trusted nor an operator can't break or place blocks, open containers, or use doors,
buttons and levers. Claims start empty: existing builds are not claimed for anyone, their owners claim them. Each
player may claim `claims.initial_blocks` plus `claims.blocks_per_hour` for each hour played (vanilla's `play_time`
statistic, so past hours count), up to `claims.max_blocks`. Spawn protection is a claim of `spawn.protection_radius`
around `/setspawn` that only operators change. Not covered at first: pistons, fluids, explosions and fire entering a
claim from outside.

## Configuration and data

Everything is in the world's folder, `world/horizon-smp/`, so a backup of the world carries it, and a world without
that folder is untouched: the mod creates it the first time it runs.

| File | Holds |
|---|---|
| `config.json` | the settings below, written with their defaults on first run; a section added by a later version is added to the file |
| `economy.json` | balances by player UUID, and names last seen |
| `homes.json`, `warps.json`, `spawn.json` | positions with their dimension |
| `claims.json` | claims: id, owner, dimension, corners, trusted UUIDs |
| `lottery.json` | the current draw's tickets |
| `audit.log` | one JSON line per transaction: time, kind, from, to, amount, items, balances after |

Each `.json` file is written whole to a temporary file, flushed and renamed over the old one, at the end of a server
tick that changed it (at most once a tick) and when the world closes: a crash leaves the old file or the new one,
never half of one. `audit.log` is appended before the balances change.

```json
{
  "currency": "coins",
  "starting_balance": 0,
  "market": {
    "enabled": true,
    "items": { "minecraft:diamond": { "buy": 400, "sell": 100 }, "minecraft:wheat": { "buy": 0, "sell": 1 } }
  }
}
```

In the market, `sell` is what the server pays for one item and `buy` what it asks; 0 closes that side. The later
phases add their sections (`teleport`, `homes`, `claims`, `spawn`, `rules`). `gambling` holds `enabled` (false),
`min_bet` (10), `max_bet` (1000), `cooldown_seconds` (30), `offer_seconds` (60) and `fee_percent` (0). A coinflip's
stakes are held in `economy.json` itself (`held`), so one atomic write covers balances and stakes; an offer that
ends, expires or whose player leaves gives its stake back, and so does the next start for one the server stopped
with. The coin is a fresh random key from the system, never the world's random.

## Building the official server's engine

A native mod is built into the game it runs in ([native-mods.md](native-mods.md)). For the server: the source of the
release the server runs, `horizon-mixins apply` with the `smp` bundle, then the server build
(`cargo build --release --no-default-features -p horizon-client`), and `horizon-server`'s `[engine] path` set to
the result. When to deploy it on the official server is the owners' decision; the mod's README holds the exact
commands.

## Cost

With no mod hooking them, the hook points are empty functions. With `smp`, a server tick writes the files that
changed (none most ticks) and looks at the teleports waiting; a claim check is a hash lookup of the chunk's claims,
made only when a player acts on a block. Each phase measures a server tick with 55 bots (`tools/server_bench.py`).

## Phases

Each is pull requests under ~500 lines, each tested: the hook points here (unit tests, a game test that hooks
them), the mod in horizon-mods (its crate's tests, and a proof: a dedicated server built with the bundle and real
clients driven by `horizon-ctl`, in fresh folders).

1. **Economy**: command and player hook points; the mod's ledger, data files and audit log, `/balance`, `/pay`,
   `/baltop`, `/eco`; then the items helpers and the market (`/sell`, `/buy`, `/worth`).
2. **Trading**: the server menu hook points; `/trade`, with a test for each way out (close, cancel, disconnect,
   death, a full inventory, the other player leaving).
3. **Gambling**: `/coinflip` with escrow, then the lottery; limits, cooldown, logged.
4. **Organisation**: teleport and damage hook points, `/spawn`, homes, `/rules`, `/tpa`, warps; the interaction
   hook points, claims, spawn protection and limits that grow with time played.
