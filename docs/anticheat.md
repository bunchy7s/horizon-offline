# The dedicated server's anticheat

Horizon's dedicated server checks what players send beyond what vanilla checks. It is **on** by default,
every check with it, but only the checks a vanilla client can't trip act on their own: `combat` drops the
attacks of a kill aura, kicks it, and bans it when it comes back and does it again; `ground` drops the
false ground claims of a no-fall or a walk on water (never sending the player back), kicks one that keeps at
it and bans it the second time. `timer` and `prediction` only write `logs/anticheat.log` (their `setback`,
`alert` and `kick` are 0) until they have shown no false positive with real players; then they act by
default too. An owner changes it in `server.toml`:

```toml
[anticheat]
enabled = true             # false: vanilla's checks only ("moved too quickly", "moved wrongly"...)
alerts = true              # tell online operators (alert_level, default 2) in their chat
alert_level = 2
log = "logs/anticheat.log" # every flag, one line each; "" writes none
strikes = "anticheat-strikes.json" # each player's kicks by check, kept across restarts

[anticheat.timer]          # each check has a table like this one; every key is optional
enabled = true             # false turns this check off
setback = 2                # the thresholds once trusted (0, log only, by default)
alert = 5
kick = 20
decay = 0.02
ban_after = 0              # the kick (counting this check's earlier ones) that bans instead; 0: never
```

| Check | Acts by default | Thresholds once trusted (`setback`, `alert`, `kick`) |
|---|---|---|
| `combat` | yes: 2, 5, 20, and `ban_after = 2` (the first kick warns, the second bans) | |
| `timer` | no, logs only | 2, 5, 20 |
| `ground` | yes: 10, 15, 30, and `ban_after = 2`. On the official server's first two hours (67 players, log only), every player not flying or walking on water peaked at level 7; those who were, at 113 to 3749 | |
| `prediction` | no, logs only | 5, 15, 60 |

It lives in the server for now, in its own crate (`horizon-anticheat`) plus one file of the hub
(`lan/anticheat.rs`), because the native mod API ([native-mods.md](native-mods.md)) has no hooks yet for a
player's moves (the position, the on-ground claim, cancelling a move) or for packet timing. It moves into a
native mod once those hooks exist.

horizon-server checks the section before every start (a wrong one turns the anticheat off, with an
error in the log); the game reads it from `server.toml` when it starts (`horizon-client --dedicated` run by hand reads the same file from its folder, if there is one).

## How a check acts

A check that fails **flags** the player: the check's violation level (VL) rises (by 1 for most flags) and
falls back by `decay` every second. From `setback` on, the move or action is cancelled and the player is
put back where the server last had them (`combat` drops the attacks instead, and `ground` only drops the
claim: `setback = 0` makes it log only); from `alert` on, the console, the log and the online operators
are told (at most once a second per player and check); from `kick` on, the player is disconnected with
"Kicked by the server's anticheat". A threshold at 0 turns that action off.

A check's kicks are counted for each player, across sessions and restarts (`strikes`,
`anticheat-strikes.json`). The kick that reaches the check's `ban_after` bans instead: the player goes
into `banned-players.json` as `/ban` puts them, with "Horizon anticheat" as the source and "Horizon
anticheat: <check>" as the reason, and is sent away with vanilla's ban message. A ban starts the count
again, so a player an operator pardons (`/pardon`) is kicked once more before another ban. Only checks
no legit client trips ban by default (`combat`); `ban_after = 0` turns a check's bans off.

A kick or a ban is always told: the log line (with what the check saw), a warning in the console, and
a line in the chat of the online operators (`alert_level`), so an operator can pardon a ban that was
wrong.

Every flag goes to `log`, with the time, the player, the check, its level, what it saw and what was done:

```
[2026-10-30 17:29:55] Steve failed timer (VL 5.9): 301 ms ahead, sent back
[2026-10-31 18:02:11] Alex failed combat (VL 20.0): attacked without swinging (1 more attacks than swings in a tick), kicked (1 of 2 before a ban)
[2026-10-31 18:09:40] Alex failed combat (VL 20.0): attacked entities 41 and 57 in one tick, banned
[2026-11-01 03:14:02] Steve failed prediction (VL 12.4): moved up to 0.173 blocks off what its keys allow (1 tick; off +0.000 +0.173 +0.000, moved 0.281, air sprinting, keys forward jump sprint, at 12.30 64.00 -5.50, velocity +0.000 +0.248 +0.000)
```

A `prediction` line ends with its worst tick as the replay played it: where the client said it went less
where the replay ended (x, y, z), how far the client said it moved, and what the replayed body was doing
(`ground` or `air`, then `water`, `gliding`, `sprinting`, `sneaking`, `swimming`, `flying`, `riding`), so a
lag, a glide or a ladder can be told from a cheat before the check acts. Then the tick's keys, where the
client said it ended (x, y, z) and the replay's velocity: enough to find the place again and play the tick
there.

## Checks

| Check | Catches | How |
|---|---|---|
| `timer` | a sped-up game (more ticks a second than the server's clock) | vanilla's client ends each tick with ServerboundClientTickEndPacket, at the server's tick rate. Each one spends one tick of a balance that wall-clock time fills. A client more than `tolerance_ms` (300) ahead is flagged; a quiet connection banks at most `max_lag_ms` (1000), so a lag spike's backlog isn't flagged and a pause can't be saved for a burst. A slow server tick adds its own time before its backlog is counted. |
| `ground` | no-fall (claiming the ground to skip fall damage) and walking on water | a move that says it ends on the ground is judged only when every block around the feet (0.6 below them, 1/16 wider than the body) is air or a fluid, nothing collides a tenth of a block down, and no boat, minecart, shulker or happy ghast is near. Anything else (powder snow, scaffolding, a waterlogged slab...) isn't judged. A false claim is dropped, so vanilla's fall damage follows (not while the check only logs); the player is never sent back, and a move whose claim was dropped never counts toward vanilla's floating kick (sent back into the air it claimed to stand in, an honest player on a block the server doesn't have claimed it again until that kick: only the anticheat's own kick, logged and told to the operators, ends a false claim); creative, spectators and gliders are skipped (no fall damage to dodge), and so is a claim standing still (no fall to skip; hovering is vanilla's floating kick). |
| `prediction` | speed, step, fly, jesus: any move vanilla's movement can't make from the player's own keys | each client tick (the packets between two ServerboundClientTickEndPackets: keys, sprinting, rotation, the move) is recorded, and every server tick ends with a ping so the server knows from which client tick on what it changed (a push, a teleport, an effect) was in effect. Each tick is then played again with the client's own movement code, from where the last one ended (it starts at a teleport the client accepted), and compared with where the client said it went; the replay then goes on from the client's position. What the server sent the player that changes its movement (a velocity, an explosion's knockback, attributes, effects, the sprinting flag) is applied on the first client tick that had seen it, so a client that ignores a push (anti-knockback) is off too. The keys and sprinting are the client's own (its sprint commands, not the server's player, whose sprint a hit stops), so they are never guessed; a sprint the server stopped that the client says goes on is played only if vanilla's client starts it again (the sprint key held, or forward pressed again); what the server can't know is tried (the ground on the first tick after a teleport, the item use on the tick the server's changed). Not played: riding, sleeping, dead, spectators. A move that lands farther than `tolerance` (0.03 blocks) from the replay flags, weighing its offset in tolerances (at most 5 a tick); the setback puts the player back where its moves last agreed. What the replay can't play yet excuses the tick, and the ticks after it until the moves agree again (2 seconds at most): another entity or player within 3 blocks (a push, a boat or shulker to stand on, a firework to fly with), an entity within 4 blocks that the server removed and the client hadn't heard of yet, a block change within 4 blocks the client hadn't seen or the server hasn't sent yet, abilities (a game mode, flight, a glider put on) the client hadn't seen, a moving piston, a chunk around the player the client doesn't have, a riptide launch, a teleport that keeps the player's motion (an End gateway's). Around a fast player (an elytra) these distances grow by two ticks of its speed, as what the server has around it is a tick or so behind. Under an excuse the replay takes the client's velocity, so it agrees again within a few ticks. It costs one ping and one replayed tick a tick per player. |
| `combat` | a kill aura: attacks a vanilla client can't send | vanilla's client attacks from one place only (`Minecraft.startAttack`, once a client tick, with no screen open): the entity under the crosshair, picked once a tick, then it swings (ServerboundPunchPacket) at once. An attack with no swing before the next attack or the tick's end (a kill aura hiding its swing), or a second entity attacked in the same client tick (a multi-target aura), flags once. From `setback` (2) on the player's attacks are dropped until the level decays below it (`decay` 0.5 a second); `kick` is 20. It costs nothing until a player attacks. |

More checks (aim, world, anti-xray) come in their own pull requests; this table lists the
ones in this version.

## Writing a check

- The rules are in `crates/horizon-anticheat` (no game state: unit-test them there); the hub runs them
  where it handles the packet (`crates/horizon-client/src/lan/`) and calls `Hub::flag`, which carries out
  setback, alert and kick (`lan/anticheat.rs`).
- A check is one entry in the `checks!` table of `horizon-anticheat/src/lib.rs`: its name in
  `server.toml`, its default thresholds and its own settings.
- Predict what vanilla allows (effects, ice, elytra, riptide, vehicles, knockback, levitation, slime,
  bubble columns, latency). A false positive is worse than a missed cheat: when unsure, don't flag.
- A clean player must cost nothing: no work per tick unless the check needs it, and none at all while the
  anticheat is off (`Hub::anticheat` is `None`).
- A new check that a vanilla client could trip ships logging only (thresholds at 0), until real players
  have shown it no false positive.
- Original code only (CONTRIBUTING.md): written from vanilla 26.3 and public descriptions of what cheats
  do. Never read, copy or paraphrase another anticheat's source (Grim, NoCheatPlus, Vulcan, Matrix,
  Spartan, Paper's anti-xray...), its decompiled jars or docs quoting its code.
- Test it both ways in `tools/anticheat_e2e.py`: a horizon-bots cheat is caught, legit bots are never
  flagged, and nothing happens with the anticheat off.

```sh
cargo build --profile fast -p horizon-client --bin horizon-client
cargo build --profile fast -p horizon-bots
python3 tools/anticheat_e2e.py --bin-dir target/fast
python3 tools/server_bench.py --servers horizon --bots 60 --behaviour walk --anticheat   # its cost
python3 tools/server_bench.py --servers horizon --bots 60 --behaviour walk --ground --anticheat   # survival, on the ground
```
