# Native hook points, modelled on NeoForge 26.3 (proposal)

Status: proposal, for the authors of hook PRs to comment on. Nothing here is built yet. Checked against `native-hooks.json` on main
(41 hooks, `NATIVE_HOOKS_API` 24) on 31 October 2026.

Native mods hook empty functions in `crates/horizon-client/src/game/native_hooks.rs`, listed by name in
`native-hooks.json` ([native-mods.md](native-mods.md)). So far each hook was designed on its own, and open PRs
number their API versions over each other. This proposes one
reference for every new hook point: the NeoForge 26.3 event that does the same thing.

Why NeoForge:

- Its events already answer the questions each of our hooks asks again: where exactly it fires, on which side,
  what it can cancel, what it is handed.
- Mod authors know them. A Java mod's `@SubscribeEvent` methods map to hook names, which helps porting by
  hand. [horizon-port-pack](modpacks.md) may suggest mappings for a human to review, not translate listeners
  automatically.

What we take is event names and their documented contracts, from NeoForge's public user documentation
(<https://docs.neoforged.net/>) only, each claim below citing the page it comes from, as [CONTRIBUTING.md](../CONTRIBUTING.md#only-original-code) allows. Nobody
reads, copies, translates or paraphrases NeoForge's source, its patches included: where a hook fires is decided
from vanilla 26.3's own methods, and the hooks stay Horizon's own Rust.

## Rules

1. **Name.** A hook with a NeoForge counterpart takes the event's name in snake case, its nested class
   included: `ServerTickEvent.Pre` → `server_tick_pre`, function `native_server_tick_pre`;
   `PlayerInteractEvent.RightClickBlock` → `right_click_block`. A hook with no counterpart (the minimap, the
   mods list, the item panel) keeps a Horizon name.
2. **Firing point.** It fires in Horizon's port of the vanilla method where the documented event happens
   (the tick starts, a block is broken), at the point vanilla's own steps make that true. The catalog names the vanilla method, so a
   reviewer can check it with `tools/rea_vanilla.py`.
3. **Side.** NeoForge documents that "some events are only fired on one side"
   ([Events](https://docs.neoforged.net/docs/concepts/events/), "Sided Events"); the rest fire on whichever
   side runs the method. Horizon has one binary with several
   `Game`s (the window, the server thread, a dedicated server), so each hook says its side:
   - `server`: the server's `Game` (single player's server thread, LAN host, dedicated) and the in-frame path.
   - `client`: the window's `Game`.
   - `both`: both, with the `Game`'s role available (`native_window`). Interactions are `both`: the client
     predicts, the server acts, and a mod that claims a use on one side only gets the two out of step.
4. **Cancel and results.** Hooks never return; several mods hook the same point. A cancellable event takes
   `canceled: &mut bool`; one with a result (an `InteractionResult`) takes `result: &mut Option<...>`. Every
   mod is called; a mod checks `*canceled` itself when it should stand back. NeoForge's handlers likewise
   "can opt to explicitly receive cancelled events" and some events implement `ICancellableEvent`
   ([Events](https://docs.neoforged.net/docs/concepts/events/), "Cancellable Events").
5. **Payload.** The event's getters become parameters, by value or `&`. A hook gets `&mut self` only when the
   NeoForge listener may change the world at that point (server-side events); otherwise `&self`, and the
   doc does not call a `&mut` hook read-only.
6. **Frozen once merged.** Name, parameters and call site never change after a merge (as today). A changed
   contract is a new hook (`..._v2`) or a new catalog `version`. A stack of PRs that refines one hook lands
   in one release batch, under one `NATIVE_HOOKS_API` bump.
7. **API numbers are given at merge**, not when a PR is opened: a PR says "API next" and takes `main + 1`
   when it merges, so open PRs stop colliding.

## Catalog fields

Each `native-hooks.json` entry gains three optional fields:

```json
{
  "name": "server_tick_pre",
  "version": 1,
  "target": "horizon_client::game::native_hooks::Game<'_>::native_server_tick_pre",
  "neoforge": "ServerTickEvent.Pre",
  "side": "server",
  "cancellable": false
}
```

`neoforge` is absent for Horizon-only hooks. horizon-mixins ignores fields it doesn't know, so this needs no
tool change; the launcher and the docs can list them.

## The hooks we have

Main's catalog (`native-hooks.json`) is the list; this page does not copy it. Each hook would gain `side`,
`cancellable` and, where one exists, `neoforge`, filled in by the PR that adopts this page. Hooks main already
has under a NeoForge-like name: `server_tick`, `level_tick_pre`, `level_tick_post`, `right_click_block`,
`block_entity_tick`, `destroy_block`, `place_block`, `attack_entity`. Per rule 6 none of them is renamed:
a rename is a new hook with a new name, if ever wanted.

## Next hooks

Hooks main does not have yet, with the vanilla method each would sit in (rule 2). The NeoForge names are
proposals, to be confirmed against the Javadoc linked from the documentation when a hook is built.

| Hook | NeoForge event | Side | Cancellable | Vanilla method |
|---|---|---|---|---|
| `left_click_block` | `PlayerInteractEvent.LeftClickBlock` | both | yes | `ServerPlayerGameMode.handleBlockBreakAction` |
| `right_click_item` | `PlayerInteractEvent.RightClickItem` | both | yes, with a result | `ServerPlayerGameMode.useItem` |
| `entity_interact` | `PlayerInteractEvent.EntityInteract` | both | yes | `Player.interactOn` |
| `player_tick_pre` / `_post` | `PlayerTickEvent.Pre` / `.Post` | both | no | `Player.tick` |
| `entity_join_level` | `EntityJoinLevelEvent` | server | yes | `ServerLevel.addEntity` |
| `living_incoming_damage` | `LivingIncomingDamageEvent` | server | yes | `LivingEntity.hurtServer` |
| `living_death` | `LivingDeathEvent` | server | yes | `LivingEntity.die` |
| `container_open` / `_close` | `PlayerContainerEvent.Open` / `.Close` | server | no | `ServerPlayer.openMenu`, `doCloseContainer` |
| `client_tick_pre` / `_post` | `ClientTickEvent.Pre` / `.Post` | client | no | the window's half of `tick` |

## Capabilities

Hoppers, comparators and other mods should not read a fixed `slots` field. NeoForge's documentation lists
`Capabilities.Item.BLOCK` ("automation-accessible inventory of a block") with `ResourceHandler<ItemResource>`,
and `Capabilities.Energy.BLOCK` with `EnergyHandler`
([Capabilities](https://docs.neoforged.net/docs/inventories/capabilities/)). A Horizon
`block_capability(pos, kind, side, out)` hook could fill an out parameter with a handler object, so new kinds
(fluids, energy) are new kinds, not new fields on a struct, and older mods keep compiling. Menus and screens
are not modelled here: main already has `menu_clicked`, `menu_closed`, `screen*` and `block_entity_container`,
and any change to them gets its own proposal with its own citations.

## Rollout

1. This document, and the catalog fields filled in for the hooks main has.
2. The table above, one small PR per hook or family, each with a test that it fires at the vanilla method
   named here.

## Open questions

- Snake-case NeoForge names without a `neoforge_` prefix (agreed in the comments). Both-side hooks pass the
  role as a parameter (agreed).
- Rule 7 renumbers every open PR on each merge. Every catalog entry already has a name and `version`, so a
  bundle could list the hooks it uses (`random_tick` v1...) and the launcher check each against the catalog,
  with `"hooks": N` kept for old bundles. That is a separate change to `native-mods.md`; if agreed, rule 7
  goes away.
