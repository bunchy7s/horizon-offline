---
name: vanilla-parity
description: Fix a Horizon bug or gap so it behaves 1:1 like vanilla 26.3, starting from the port ledger (tools/port_ledger.py) instead of grepping. Use for any issue, Discord report or PR that makes Horizon match vanilla: "X doesn't work like in Java", a missing sound, particle, animation, mob behaviour, block rule, screen or packet.
---

# Vanilla parity with the port ledger

The port ledger pairs every vanilla method with the Rust fn that ports it (or says it is missing, or needs
no port) and lists where a port isn't wired like vanilla. Ask it first; grep only for what it can't answer.
Full reference: `docs/port-ledger.md`.

## 0. Have a fresh ledger

```sh
python3 tools/port_ledger.py build          # ~20 s; again after pulling main
```

It needs the vanilla jar (the launcher's `versions/26.3/26.3.jar`, or `--jar`) and/or the lab's decompiled
sources (`python3 tools/lab.py decompile`, or `--sources DIR`). If `build` says it has no vanilla code, run
the lab first; don't guess vanilla from memory.

## 1. From the report to the code

```sh
python3 tools/port_ledger.py issue 3236                      # a GitHub issue number
python3 tools/port_ledger.py issue "mooshroom stays red after lightning"   # or the report's text
```

It ranks the vanilla classes the report names and lists, per class, its Rust home files, where each method is
ported, and the known gaps (`missing` rows from the verdict map, seam/client/drift gaps). Often the cause is
already written there ("Type is not saved"). Then drill in:

```sh
python3 tools/port_ledger.py where MushroomCow.readAdditionalSaveData   # ports, callers, callees, gaps
python3 tools/port_ledger.py class MushroomCow                          # every method: where, or missing
python3 tools/port_ledger.py what crates/horizon-client/src/mobs/farm.rs:120   # which vanilla method a fn ports
```

A ledger answer is a lead, not a verdict: about 1 in 12 map rows is wrong. Read the vanilla method (path and
line are in the output) and the Rust fn before changing anything.

## 2. Port it the vanilla way

- Translate the vanilla method, don't reinvent it: same order of operations, same constants, the same random
  draws in the same order (`horizon_core::random`), Java `Math` through `horizon_core::jmath`.
- Open the fn's doc with the vanilla method it ports: `` /// `MushroomCow.readAdditionalSaveData`: ... ``.
  That one line is what makes the next agent find it.
- Fix the path players run: single player is a client of the integrated server, so server logic lives in
  `game/lan*.rs` and `src/lan/`, and what only the client's copy shows (particles, local sounds, hand
  animation) must be reached from `game/remote*.rs`. A fix on the old in-frame path alone fixes nothing.
- Look at the neighbours the ledger lists for the same class: a `client` or `seam` gap next to your fix is
  usually the same bug on the other path.

## 3. Before the PR

```sh
python3 tools/port_ledger.py check --base origin/main
```

It lists the ports your branch touches with their open gaps, citations to add, stale citations, and (against
the base) the gaps the branch closes and opens. Close what belongs to your fix, cite what you ported, leave
no new stale citation. If you ported a method the verdict map called `missing`, change its row in
`tools/port_map/*.tsv` to `port` with the fn (`file.rs::Owner::fn`) and run
`python3 tools/port_ledger.py map-check tools/port_map/<file>.tsv`.

Then the usual: the harness for the area (`docs/architecture.md`), `cargo check`/tests per `AGENTS.md`, and
a small PR that says which vanilla method it now matches.

## Finding work

```sh
python3 tools/port_ledger.py gaps --kind seam      # ported on two paths, wired on one: the best leads
python3 tools/port_ledger.py gaps --kind client    # client-only effects the server path never reaches
python3 tools/port_ledger.py gaps --kind drift     # a port missing a sound, particle, item or constant
rg -n "player:" tools/port_map/*.tsv               # missing behaviour a player would see
```
