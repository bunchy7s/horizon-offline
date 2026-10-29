# Architecture: feature → files → checks

The 40 areas changed most often in the 30 days to 2026-10-23 (commits on main touching the area; `game.rs` is
counted only for the game loop because nearly every gameplay change touches it). The crate map, how `Game` is
organised and how to build and test are in [AGENTS.md](../AGENTS.md).

Paths: `L/` = `crates/horizon-client/src/`, `G/` = `crates/horizon-client/src/game/`, `W/` =
`crates/horizon-world/src/`, `K/` = `crates/horizon-client-core/src/`, `C/` = `crates/horizon-command/src/`,
`I/` = `crates/horizon-items/src/`, `R/` = `crates/horizon-client-render/src/`, `N/` = `crates/horizon-client-input/src/`.
Checks: a bare name is a harness (`python3 tools/parity.py <name>`, script `tools/test_<name>_parity.py`);
`lan_e2e <check>` is `python3 tools/lan_e2e.py --check <check>`.

For the full set of harnesses a file can affect, ask the runner:
`printf '%s\n' <paths> > /tmp/p.txt && python3 tools/parity.py --changed-files /tmp/p.txt --list`.
Any file of the game binary (`game.rs`, `game/`) selects the ~25 harnesses that run the binary; a client lib
file selects ~103.

| # | Area (commits) | Files | Checks |
|---|---|---|---|
| 1 | Game loop, tick order, frame pacing (554) | `L/game.rs` (`fn tick`, `fn tick_world`, `fn render`, `about_to_wait`), `L/clock.rs`, `N/window.rs`, `R/spans.rs` | game_loop, frame_limiter, interaction_input, tick |
| 2 | Mobs in the game loop, spawning (166) | `G/creatures.rs`, `G/natural_spawning.rs`, `L/mobs/spawning.rs`, `L/mobs/spawn_rules.rs`, `L/mobs/spawn_selection.rs`, `L/mobs/finalize.rs`, `G/spawner.rs`, `G/trial_spawner.rs` | natural_spawn, spawn_rules, finalize_spawn, spawner, trial_spawner, mob_ai |
| 3 | World save and load (163) | `L/save.rs`, `G/saving.rs`, `G/persist.rs` (entities), `L/entity_nbt.rs`, `K/block_entity_nbt.rs`, `W/region.rs`, `W/nbt.rs` | save_roundtrip, world_reopen, world, datafix |
| 4 | World generation (153) | `W/worldgen.rs`, `W/worldgen/` (`generator*.rs`, `feature_graph.rs`, `structure/`, `placement/`), `L/vanillagen.rs` | chunk_generation, worldgen, server_world_gen, world_spawn, feature_graph, feature_placement, decoration and the other worldgen harnesses |
| 5 | Multiplayer client, packets in (145) | `G/remote.rs` (`fn handle_packet`, `fn tick_remote`), `G/remote_*.rs`, `L/net.rs` | dedicated_server, server_entities, lan_e2e (all) |
| 6 | Agent control, horizon-ctl (145) | `L/control.rs`, `L/bin/horizon-ctl/`, `G/control.rs`, `G/agent.rs`, `G/events.rs`, `G/ui_control.rs` | `python3 tools/agent_control_e2e.py`; `cargo test -p horizon-client --bin horizon-ctl` |
| 7 | CI and release (134) | `.github/workflows/` (`ci.yml`, `parity.yml`, `release.yml`, `launcher.yml`), `tools/ci/`, `tools/parity.py`, `tools/pr_ready.py` | `python3 -m unittest discover -s tests`; actionlint (`.github/actionlint.yaml`) |
| 8 | LAN host: connections, play state, tracking (133) | `L/lan/` (`conn.rs`, `hub.rs` + `hub/`, `chunks.rs`, `entity.rs`, `tracking.rs`, `guest.rs`, `playerdata.rs`), `G/lan.rs` | lan, lan_e2e (all), `cargo test -p horizon-client --test lan_bot` |
| 9 | Renderer core (132) | `R/gfx.rs` (`Renderer::new`, `Renderer::render`), `R/gfx/` (cull, occlusion, translucency, upload, timing, lod) | client_graphics, culling_rays, translucency; `horizon-client --benchmark standard` |
| 10 | Launcher (124) | `launcher/src/` (`main.rs`, `backend.rs`, `cli.rs`, `mcp.rs`, `servers_page.rs`) | `cd launcher && cargo test`; `python3 tools/launcher_mcp_e2e.py` |
| 11 | Mob types and species (117) | `L/mobs.rs`, `L/mobs/species.rs`, `L/mobs/data.rs`, `L/mobs/living.rs`, `L/mobs/drops.rs`, `L/mobs/gear.rs` | mob_attributes, entity_attribute, entity_death_loot, mob_ai |
| 12 | Guests' actions on a server (98) | `G/lan_actions.rs` (`fn as_guest`, `fn tick_guests`), `G/lan_{attributes,pvp,chat,crafting,stations,trading,mount_menu,waypoints}.rs` | server_player_combat, server_player_damage, server_player_targets, lan_e2e (all) |
| 13 | Modpack porting, mod content (97) | `crates/horizon-port-pack/`, `L/mod_content.rs`, `tools/port_*.py` | `cargo test -p horizon-port-pack`; `python3 tools/mod_port_harness.py` |
| 14 | Mob AI (96) | `L/mobs/ai.rs`, `goals.rs`, `brain.rs`, `*_brain.rs`, `nav.rs`, `path.rs`, `control.rs` | mob_ai (probe `examples/mob_ai_probe.rs`; known failure on main: drowned_*, #290) |
| 15 | Items and item use (96) | `I/items.rs`, `I/item_data.rs`, `G/hand.rs`, `G/consume.rs`, `G/potions.rs`, `R/effects.rs`; game.rs `fn use_hand`, `fn use_item` | use_item_on, item_use, item_use_lifecycle, consumable, consume_effect, status_effect |
| 16 | Options and settings screens (93) | `R/options_ui.rs`, `G/options.rs`, `R/settings.rs`, `R/options_file.rs`, `R/graphics_ui.rs`, `R/graphics.rs` | options, toggle_keys |
| 17 | HUD, chat, tab list (93) | `R/hud.rs`, `R/hud_layout.rs`, `R/hud_overlays.rs`, `R/chat.rs`, `G/chat.rs`, `R/tab_list.rs`, `L/locator_bar.rs`, `G/locator_bar.rs` | hud, tab_list, locator_bar, chat_screen, secure_chat |
| 18 | Command execution (88) | `G/commands.rs`, `G/commands/*.rs`, `L/scoreboard_cmd.rs`, `L/data_cmd.rs`, `K/scoreboard.rs`, `G/functions.rs` | command_exec (corpus `tools/command_exec_corpus.py`), scoreboard, server_ops_feedback, lan_e2e commands |
| 19 | Chunk streaming, light, tickets (85) | `R/streaming.rs`, `R/world.rs`, `R/chunk.rs`, `R/chunk_map.rs`, `R/light.rs`, `L/ticket_levels.rs`, `L/tick_chunk_membership.rs`, `G/world_loading.rs` | light, chunk_status, random_tick_sections; `cargo test -p horizon-client --test streaming_meshes` |
| 20 | Mob models and animation (78) | `L/mobs/render.rs`, `L/mobs/anim.rs`, `L/mobs/body_anim.rs`, `L/mobs/keyframes.rs`, `R/entity.wgsl` | entity_model |
| 21 | Block behaviour, server side (78) | `W/level/` (`mod.rs`, `tick.rs`, `hooks.rs`, `data.rs`, `redstone.rs`, `place.rs`), `G/block_entity_ticks.rs`, `G/moving_pistons.rs` | block_update, redstone, random_tick, simple_block, piston_movement, block_entity_tickers |
| 22 | Player movement and collision (75) | `L/motion.rs` + `L/motion/`, `R/collisions.rs`, `W/entity_move.rs`, `W/player_travel.rs`, `W/collision.rs` | collision, entity_move, player_travel, living_motion, climbing, swimming; `python3 tools/test_native_scene_motion.py` |
| 23 | Resource packs and models (65) | `R/packs.rs` + `R/packs/`, `R/pack_screen.rs`, `R/atlas_sources.rs`, `R/sprites.rs`, `R/model_library.rs`, `G/packs.rs`, `crates/horizon-assets/` | resource_pack, pack_selection, pack_screen, model_resolution, item_model, mipmap |
| 24 | Container menus and inventory (61) | `L/menu.rs`, `L/container_screen.rs`, `R/containers.rs`, `L/creative.rs`; game.rs `fn menu_ops`, `fn click_inventory` | menu_click, container_screen, creative, lan_e2e containers clicks |
| 25 | Workstations (60) | `L/stations.rs`, `G/stations.rs`, `I/enchant.rs`, `R/brewing.rs` | enchanting, screen_layout, lan_e2e brewing loom stonecutter grindstone anvil smithing |
| 26 | Crafting and the recipe book (59) | `L/crafting.rs`, `L/recipe_book.rs`, `L/recipe_book_screen.rs`, `G/recipe_book.rs`, `G/machines.rs`, `G/machines/crafter.rs` | recipe, recipe_book, recipe_book_view, place_recipe, ghost_recipe, crafter |
| 27 | Vehicles and mounts (54) | `G/boats.rs`, `G/boat_passengers.rs`, `G/minecarts.rs`, `G/mounts.rs`, `G/mount_menu.rs`, `G/llamas.rs`, `G/passengers.rs` | server_entities, lan_e2e boats minecarts riding mounts |
| 28 | Villagers, trading, raids (50) | `L/mobs/villager*.rs`, `L/merchant.rs`, `G/trading.rs`, `G/villagers.rs`, `L/raid.rs`, `G/raids.rs`, `L/gossip.rs` | trading, raid, mob_ai |
| 29 | Title, worlds, startup (49) | `L/title.rs` + `L/title/`, `G/startup_menu.rs`, `R/title_edition.rs` | world_options; `cargo test -p horizon-client --lib title` |
| 30 | Dedicated server, horizon-server (48) | `G/dedicated.rs`, `G/dedicated/status.rs`, `crates/horizon-server/`, `crates/horizon-server-files/` | dedicated_server, server_files, user_cache, online_mode, server_ops_feedback |
| 31 | Particles and client effects (46) | `R/particles.rs`, `G/render_tick.rs`, `G/level_particles.rs`, `L/animate_tick.rs` | particle_providers, block_particles, entity_particles, emitter_particles, marker_particles |
| 32 | Bosses and the End (45) | `G/dragon.rs`, `G/wither.rs`, `G/end.rs` | server_dragon, forced_end, end_podium, end_gateway |
| 33 | Protocol and packets (44) | `crates/horizon-protocol/src/` (`packets/play.rs`, `entity_data.rs`, `connection.rs`) | dedicated_server, online_mode, secure_chat; `cargo test -p horizon-protocol` |
| 34 | Multiplayer and LAN screens, friends (44) | `L/multiplayer.rs`, `G/lan_screen.rs`, `G/friends.rs`, `L/friends_screen.rs`, `crates/horizon-friends/`, `crates/horizon-relay/` | lan, world_options; `python3 tools/test_friends.py` |
| 35 | Dispensers (43) | `G/dispenser.rs`, `G/dispense_fixture.rs` (the harness side) | dispenser |
| 36 | Audio and music (38) | `R/audio.rs`, `R/sound_engine.rs`, `R/sound_events.rs`, `L/music.rs`, `G/music.rs`, `L/ambience.rs`, `G/subtitle_hud.rs` | sound_engine, sound_event, music, situational_music, ambient_sound, subtitle |
| 37 | Integrated server and levels (34) | `G/integrated.rs`, `G/levels.rs`, `G/server_travel.rs` | server_dimension_players, `lan_e2e` (server-thread by default; portals, levels, distance, lava) |
| 38 | Command parsing (34) | `C/` (`args.rs`, `selector.rs`, `tree_*.rs`, `vanilla.rs`); `cargo test -p horizon-command` | command_parse (corpus `tools/command_parse_corpus.py`) |
| 39 | Benchmarks and telemetry (34) | `L/benchmark.rs`, `tools/perf_bench.py`, `R/telemetry.rs`, `crates/horizon-telemetry*/`, `crates/horizon-bench/` | `python3 -m unittest tests.test_perf_bench`; [benchmark.md](benchmark.md) |
| 40 | Explosions and TNT (31) | `G/explosion.rs`, `G/tnt.rs` | lan_e2e blast; no dedicated harness yet |
