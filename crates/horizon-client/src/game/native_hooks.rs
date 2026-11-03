//! Hook points for native mods (docs/native-mods.md, "Mods on the hook points"): empty
//! functions the game calls at fixed places, for horizon-mixins bundles to
//! hook with HEAD. Hooks that answer take an out parameter (`claimed`,
//! `zoom`…) so several mods can hook the same point. A native mod keeps its
//! own state in a module of its bundle; the helpers below read and draw what
//! Horizon's own HUD extras do, and keep a mod's settings where the
//! WebAssembly mod of the same id kept them.
//!
//! API 359 adds a living entity jumping (`native_living_jump`).
//!
//! API 358 adds a living entity healing, with a changeable amount and cancellation (`native_living_heal`).
//!
//! API 357 adds a living entity's fall, with changeable distance and multiplier and cancellation (`native_living_fall`).
//!
//! API 356 adds a living entity drowning, with changeable drowning and damage and a read-only bubble count (`native_living_drown`).
//!
//! API 355 adds a mod's own picture in the HUD atlas (`native_mod_image`).
//!
//! API 354 adds A timed world border size change finished (`native_world_border_bounds_change_finish`).
//!
//! API 353 adds A vault ejects a loot item (`native_vault_loot`).
//!
//! API 352 adds A hopper begins an item transfer attempt (`native_prepare_hopper_transfer`).
//!
//! API 351 adds A guest player changed sneaking (`native_player_toggle_sneak`).
//!
//! API 350 adds A server player released the use-item key (`native_player_stop_using_item`).
//!
//! API 349 adds A server player's recipe book settings changed (`native_player_recipe_book_settings_change`).
//!
//! API 348 adds The server player's selected bow projectile (`native_player_ready_arrow`).
//!
//! API 347 adds A server player's experience level changed (`native_player_level_change`).
//!
//! API 346 adds A server player launched a projectile item (`native_player_launch_projectile`).
//!
//! API 345 adds A guest was disconnected by a server kick (`native_player_kicked`).
//!
//! API 344 adds A guest jumped from the ground (`native_player_jump`).
//!
//! API 343 adds A guest sent player input flags (`native_player_input`).
//!
//! API 342 adds A server player's client finished loading the world (`native_player_client_loaded_world`).
//!
//! API 341 adds A server player selected an item in a bundle (`native_player_bundle_item_select`).
//!
//! API 340 adds A guest's server handled arm swing (`native_player_arm_swing`).
//!
//! API 339 adds A server player earned an advancement criterion (`native_player_advancement_criterion_grant`).
//!
//! API 338 adds a boat or minecart about to be destroyed (`native_vehicle_destroy`).
//!
//! API 337 adds a vehicle taking damage (`native_vehicle_damage`).
//!
//! API 336 adds a boat or minecart created (`native_vehicle_create`).
//!
//! API 335 adds A dropped item entity reached its lifetime and is about to despawn (`native_item_despawn`).
//!
//! API 334 adds A hopper moves an item between containers (`native_inventory_move_item`).
//!
//! API 333 adds A hopper selects a container while searching for items (`native_hopper_inventory_search`).
//!
//! API 332 adds A server explosion is about to begin (`native_explosion_prime`).
//!
//! API 331 adds A mob's leash was removed (`native_entity_unleash`).
//!
//! API 330 adds A mob is about to drop an item (`native_entity_drop_item`).
//!
//! API 329 adds A server player stopped digging before the block broke (`native_block_damage_abort`).
//!
//! API 328 adds A player took an anvil's result (`native_anvil_craft_pre`).
//!
//! API 327 adds A player finished taking an anvil's result (`native_anvil_craft_post`).
//!
//! API 326 adds the title's own screens to the screen hooks, and the instance folder helper (`native_instance_dir`).
//!
//! API 325 adds a dropped item reaching its lifetime (`native_item_expire`).
//!
//! API 324 adds a player saved to the world's store (`native_player_save_to_file`).
//!
//! API 323 adds a player loaded from the world's save (`native_player_load_from_file`).
//!
//! API 322 adds a player's data copied to their new player (`native_player_clone`).
//!
//! API 321 adds A statistic about to be awarded to a server player, cancellable (`native_player_stat_award`).
//!
//! API 320 adds A criterion of a server player's advancement was granted or revoked (`native_player_advancement_progress`).
//!
//! API 319 adds an item frame or painting about to break, cancellable (`native_hanging_break`).
//!
//! API 318 adds A player releases a bow with adjustable charge or cancellation (`native_arrow_loose`).
//!
//! API 317 adds attachment reads that never insert the default (`AttachmentType::has`, `AttachmentType::get_existing`).
//!
//! API 316 adds A player's melee hit on a living target decides whether it is critical (`native_critical_hit`).
//!
//! API 315 adds a player's item breaking (`native_player_destroy_item`).
//!
//! API 314 adds hiding a player from the players below a permission level (`native_hide_player`).
//!
//! API 313 adds drawing a component-bearing item stack and its decorations on a screen (`native_item_stack`).
//!
//! API 312 adds a server player's ender pearl is about to teleport them (`native_ender_pearl_teleport`).
//!
//! API 311 adds a server player who ate chorus fruit is about to try a random spot (`native_chorus_fruit_teleport`).
//!
//! API 310 adds a server player's trade with a villager or wandering trader, with its offer (`native_trade_with_villager`).
//!
//! API 309 adds a shift-click in a mod container's menu, routed by the mod (`native_menu_quick_move`).
//!
//! API 308 adds the time players wake at after sleeping through the night, changeable (`native_sleep_finished_time`).
//!
//! API 307 adds a mod item holding energy (`native_item_energy_buffer`), and the item energy helpers in
//! `native_item_energy.rs`: `native_item_energy`, `native_item_energy_insert`, `native_item_energy_extract`.
//!
//! API 306 adds A world save cycle completed (`native_world_saved`).
//!
//! API 305 adds The dedicated server whitelist was enabled or disabled (`native_whitelist_state_update`).
//!
//! API 304 adds Server weather parameters changing, cancellable (`native_weather_change`).
//!
//! API 303 adds A vault block changes state (`native_vault_change_state`).
//!
//! API 302 adds Player selected a villager trade (`native_trade_select`).
//!
//! API 301 adds The server clock is about to jump, cancellable (`native_time_skip`).
//!
//! API 300 adds Server thunder state changing, cancellable (`native_thunder_change`).
//!
//! API 299 adds A projectile hits a target block (`native_target_hit`).
//!
//! API 298 adds A guest asked the server to complete a command (`native_tab_complete`).
//!
//! API 297 adds A sponge absorbs water (`native_sponge_absorb`).
//!
//! API 296 adds A sculk catalyst begins blooming (`native_sculk_bloom`).
//!
//! API 295 adds A raid is triggered or reaches a terminal state (`native_raid_state_change`).
//!
//! API 294 adds A raid wave that has spawned (`native_raid_spawn_wave`).
//!
//! API 293 adds A projectile was added to the server level (`native_projectile_launch`).
//!
//! API 292 adds Smithing result prepared (`native_prepare_smithing`).
//!
//! API 291 adds Enchanting offers calculated (`native_prepare_item_enchant`).
//!
//! API 290 adds Crafting-grid result prepared (`native_prepare_item_craft`).
//!
//! API 289 adds Grindstone result prepared (`native_prepare_grindstone`).
//!
//! API 288 adds Anvil result prepared (`native_prepare_anvil`).
//!
//! API 287 adds A thrown potion applied its hit effects (`native_potion_splash`).
//!
//! API 286 adds A Nether portal or End arrival platform is created (`native_portal_create`).
//!
//! API 285 adds The server synced changed velocity to a player (`native_player_velocity`).
//!
//! API 284 adds A server player's action removed an entity's leash (`native_player_unleash_entity`).
//!
//! API 283 adds A server player completed a merchant trade (`native_player_trade`).
//!
//! API 282 adds A server player toggled an entity's age lock (`native_player_toggle_entity_age_lock`).
//!
//! API 281 adds Player about to travel through an End gateway, cancellable (`native_player_teleport_end_gateway`).
//!
//! API 280 adds A server player swapped with an equipment slot (`native_player_swap_with_equipment_slot`).
//!
//! API 279 adds A spectator stopped watching an entity (`native_player_stop_spectating_entity`).
//!
//! API 278 adds A server player selected a stonecutter recipe (`native_player_stonecutter_recipe_select`).
//!
//! API 277 adds A server player's respawn point about to change, cancellable (`native_player_spawn_point_set`).
//!
//! API 276 adds A player's shield was disabled by a blocked hit (`native_player_shield_disable`).
//!
//! API 275 adds A player about to shear a mob, cancellable (`native_player_shear_entity`).
//!
//! API 274 adds A player launches with a riptide trident (`native_player_riptide`).
//!
//! API 273 adds A recipe about to be discovered by a server player, cancellable (`native_player_recipe_discovered`).
//!
//! API 272 adds Server player requests recipe book auto-fill (`native_player_recipe_book_click`).
//!
//! API 271 adds A server player about to travel through a portal, cancellable (`native_player_portal`).
//!
//! API 270 adds A server player picked up an arrow (`native_player_pickup_arrow`).
//!
//! API 269 adds A server player's middle-click pick, cancellable (`native_player_pick_item`).
//!
//! API 268 adds A server player opened a sign editor (`native_player_open_sign`).
//!
//! API 267 adds A server player named a mob (`native_player_name_entity`).
//!
//! API 266 adds A server player filled an empty map (`native_player_map_filled`).
//!
//! API 265 adds A server player selected a loom pattern (`native_player_loom_pattern_select`).
//!
//! API 264 adds Server player changed client language (`native_player_locale_change`).
//!
//! API 263 adds A server player's selected hotbar slot, cancellable (`native_player_item_held`).
//!
//! API 262 adds An item cooldown about to start for a server player, cancellable (`native_player_item_cooldown`).
//!
//! API 261 adds XP orb pickup cooldown changed for a server player (`native_player_exp_cooldown_change`).
//!
//! API 260 adds A server player's orb-pickup experience points before they are added (`native_player_exp_change`).
//!
//! API 259 adds A server player used a firework rocket to boost while gliding (`native_player_elytra_boost`).
//!
//! API 258 adds A server player threw an egg (`native_player_egg_throw`).
//!
//! API 257 adds A player book edit or signing packet, cancellable (`native_player_edit_book`).
//!
//! API 256 adds A server player reaches deep sleep (`native_player_deep_sleep`).
//!
//! API 255 adds A server player's client option for their dominant hand changed (`native_player_change_main_hand`).
//!
//! API 254 adds A server player changed a beacon's selected effects (`native_player_change_beacon_effect`).
//!
//! API 253 adds A server player caught a mob in a bucket (`native_player_bucket_entity`).
//!
//! API 252 adds A player left a bed, after their sleeping state changed (`native_player_bed_leave`).
//!
//! API 251 adds A player about to enter a bed, cancellable (`native_player_bed_enter`).
//!
//! API 250 adds A server player's attack strength was reset by a punch (`native_player_attack_cooldown_reset`).
//!
//! API 249 adds A player completed an advancement (`native_player_advancement_done`).
//!
//! API 248 adds Note block about to sound (`native_note_play`).
//!
//! API 247 adds A server game rule changes (`native_world_game_rule_change`).
//!
//! API 246 adds World border center change (`native_world_border_center_changed`).
//!
//! API 245 adds World border bounds change (`native_world_border_bounds_changed`).
//!
//! API 244 adds The world spawn point changes (`native_spawn_change`).
//!
//! API 243 adds A server player took a book from a lectern (`native_player_take_lectern_book`).
//!
//! API 242 adds A server player changed a lectern page (`native_player_lectern_page_change`).
//!
//! API 241 adds An orb repaired a server player's item through Mending (`native_player_item_mend`).
//!
//! API 240 adds A server player changed an item frame (`native_player_item_frame_change`).
//!
//! API 239 adds A server player inserted a book into a lectern (`native_player_insert_lectern_book`).
//!
//! API 238 adds A server player put a plant in a flower pot (`native_player_flower_pot_manipulate`).
//!
//! API 237 adds Farmland moisture changing (`native_moisture_change`).
//!
//! API 236 adds A server lightning bolt is about to be created, cancellable (`native_lightning_strike`).
//!
//! API 235 adds A leaf block decaying (`native_leaves_decay`).
//!
//! API 234 adds A dropped item entity entered the level (`native_item_spawn`).
//!
//! API 233 adds A guest is about to open a block inventory (`native_inventory_block_start`).
//!
//! API 232 adds A hopper is about to pick up a dropped item (`native_hopper_pickup_item`).
//!
//! API 231 adds A vanilla world game event fired (`native_game_event`).
//!
//! API 230 adds A furnace begins cooking an item (`native_furnace_start_smelt`).
//!
//! API 229 adds A furnace finishes smelting an item (`native_furnace_smelt`).
//!
//! API 228 adds A player takes a furnace output (`native_furnace_extract`).
//!
//! API 227 adds A furnace consumes fuel and begins burning (`native_furnace_burn`).
//!
//! API 226 adds A flowing fluid changing level (`native_fluid_level_change`).
//!
//! API 225 adds A firework rocket completed explosion effects (`native_firework_explode`).
//!
//! API 224 adds An experience bottle broke and awarded experience (`native_exp_bottle`).
//!
//! API 223 adds A mob conversion completed (`native_entity_transform`).
//!
//! API 222 adds A guest player's fall flying state changed (`native_entity_toggle_glide`).
//!
//! API 221 adds A server entity completed a same-level teleport command (`native_entity_teleport`).
//!
//! API 220 adds A mob became tamed (`native_entity_tame`).
//!
//! API 219 adds A player fired an arrow from a bow or crossbow (`native_entity_shoot_bow`).
//!
//! API 218 adds A guest player's synchronized pose changed (`native_entity_pose_change`).
//!
//! API 217 adds A server entity begins a portal transfer (`native_entity_portal`).
//!
//! API 216 adds A mob entered love mode (`native_entity_enter_love_mode`).
//!
//! API 215 adds A bee entered a beehive (`native_entity_enter_block`).
//!
//! API 214 adds A mob changed a block (`native_entity_change_block`).
//!
//! API 213 adds Two mobs completed a breeding action (`native_entity_breed`).
//!
//! API 212 adds A mob broke a door (`native_entity_break_door`).
//!
//! API 211 adds Entities left the active world as a chunk unloaded (`native_entities_unload`).
//!
//! API 210 adds Saved entity records finished loading into a chunk (`native_entities_load`).
//!
//! API 209 adds Player applied an enchantment (`native_enchant_item`).
//!
//! API 208 adds The Ender Dragon fight placing its first egg (`native_dragon_egg_form`).
//!
//! API 207 adds Lightning charged a creeper (`native_creeper_power`).
//!
//! API 206 adds A player ignited a creeper (`native_creeper_ignite`).
//!
//! API 205 adds A crafter completes a recipe (`native_crafter_craft`).
//!
//! API 204 adds A player composts an item (`native_compost_item`).
//!
//! API 203 adds Cauldron fill level about to change (`native_cauldron_level_changed`).
//!
//! API 202 adds An item starts cooking on a campfire (`native_campfire_start`).
//!
//! API 201 adds A server system broadcast line, rewritable or cancellable (`native_broadcast_message`).
//!
//! API 200 adds A brewing stand starts a brew (`native_brewing_start`).
//!
//! API 199 adds Brewing stand consumed fuel (`native_brewing_fuel`).
//!
//! API 198 adds Brewing stand finished brewing (`native_brewing_complete`).
//!
//! API 197 adds A dispenser shears an entity (`native_block_shear_entity`).
//!
//! API 196 adds A sculk listener receives a game event (`native_block_receive_game`).
//!
//! API 195 adds A guest checks a block container lock (`native_block_lock_check`).
//!
//! API 194 adds Fire about to be placed (`native_block_ignite`).
//!
//! API 193 adds Precipitation forms ice or snow (`native_block_form`).
//!
//! API 192 adds A dispenser fails to dispense an item (`native_block_failed_dispense`).
//!
//! API 191 adds A block fades (`native_block_fade`).
//!
//! API 190 adds A block is about to drop experience (`native_block_exp`).
//!
//! API 189 adds A dispenser equips armor or equipment (`native_block_dispense_armor`).
//!
//! API 188 adds A block finishes cooking an item (`native_block_cook`).
//!
//! API 187 adds Fire about to burn a block away (`native_block_burn`).
//!
//! API 186 adds A guest's block-breaking progress changes (`native_block_break_progress_update`).
//!
//! API 185 adds Bell about to ring (`native_bell_ring`).
//!
//! API 184 adds A beacon activates or deactivates (`native_beacon_state_change`).
//!
//! API 183 adds An anvil takes damage (`native_anvil_damaged`).
//!
//! API 182 adds a totem of undying saving a player (`native_player_totem`).
//!
//! API 181 adds TNT about to be lit, cancellable (`native_tnt_primed`).
//!
//! API 180 adds a server player placing an item frame, painting or leash knot, cancellable (`native_hanging_place`).
//!
//! API 179 adds a server player emptying a bucket, cancellable (`native_bucket_empty`).
//!
//! API 178 adds a boat or minecart a server player rides having moved (`native_vehicle_moved`).
//!
//! API 177 adds a creative player setting a slot from the creative screen, cancellable (`native_creative_slot`).
//!
//! API 176 adds a player opening one of the game's menus, cancellable (`native_container_opened`).
//!
//! API 175 adds a player taking an item from or putting one on an armor stand, cancellable (`native_armor_stand_manipulate`).
//!
//! API 174 adds a server player's answer to a resource pack (`native_resource_pack_status`).
//!
//! API 173 adds a server player's sprint starting or stopping (`native_player_sprint`).
//!
//! API 172 adds a joining player the mods may refuse (`native_player_login`).
//!
//! API 171 adds the root commands a player's command tree holds (`native_player_commands_sent`).
//!
//! API 170 adds a server player teleported by a command (`native_player_teleported`).
//!
//! API 169 adds a player's food level changing, changeable and cancellable (`native_food_level_changed`).
//!
//! API 168 adds a player swapping their hands, cancellable (`native_player_swap_hands`).
//!
//! API 167 adds lowering the frame rate limit, as Dynamic FPS does (`native_framerate_limit`).
//!
//! API 166 adds a recipe ingredient of a mod's own kind
//! (`native_ingredient`, which the unknown ingredient type in a namespace of
//! own reaches with the kind, its JSON and the item list to fill).
//!
//! API 165 adds the window's physical pixel size (`native_physical_window_size`).
//!
//! API 164 adds a helper that names the block a state belongs to (`native_state_block`).
//!
//! API 163 adds a server mod's line for the event stream and the Discord bridge (`native_server_event`).
//!
//! API 162 adds a player's item losing durability, changeable and cancellable (`native_item_damaged`).
//!
//! API 161 adds a player taking a crafting grid's result (`native_item_crafted`).
//!
//! API 160 adds a player's fishing rod: cast, bite, catch, reel-in, cancellable (`native_player_fish`).
//!
//! API 159 adds the server's difficulty changing (`native_difficulty_changed`).
//!
//! API 158 adds a server player's game mode about to change, cancellable (`native_player_game_mode_changed`).
//!
//! API 157 adds how fire burns a mod block (`native_fire_odds`, `FireBlock.setFlammable`), and a mod block under
//! a campfire that makes it a signal fire (`native_smoke_source`, `CampfireBlock.isSmokeSource`).
//!
//! API 156 adds where a loot roll happens, for a mod's own loot kinds
//! (`NativeLootContext.place`: the origin, the block entity and the level).
//!
//! API 155 adds a server player taking an experience orb (`native_experience_orb_pickup`, asked for through
//! `native_experience_orb_pickup_listened`: cancel it, or change the orb's value and count) and the Mending repair
//! proposed for the item it picked (`native_mending_repair`).
//!
//! API 154 adds a player changing dimension (`native_player_changed_dimension`).
//!
//! API 153 adds a player starting or stopping flight, cancellable (`native_player_toggle_flight`).
//!
//! API 152 adds a player opening and closing a menu (`native_player_container_open`, `native_player_container_close`).
//!
//! API 151 adds models no blockstate names, baked for drawing (`native_standalone_models`,
//! `native_draw_model`).
//!
//! API 150 adds a player finishing a sign, rewritable or cancellable (`native_sign_changed`).
//!
//! API 149 adds a player drawn with a mod's own model, and layers added to mobs and players (`native_player_submit`, `native_mob_layer`, told while a mod sets `native_player_render_listened`).
//!
//! API 148 adds a mob's or an armor stand's equipment slot taking another stack (`native_living_equipment_change`), and whether a mod listens to the mobs' (`native_living_equipment_listened`).
//!
//! API 147 adds a mod item as a fluid container (`native_item_fluid_tank`), and the item fluid helpers in `native_item_fluids.rs`: `native_item_fluid`, `native_item_fluid_fill`, `native_item_fluid_drain`.
//!
//! API 146 adds entities ticking, which a mod can stop (`native_entity_tick_pre`, `native_entity_tick_post`, told while a mod sets `native_entity_tick_listened`).
//!
//! API 145 adds blocks and items declared in code (`native_blocks`, `native_items`).
//!
//! API 144 adds a player's or mob's breathing each tick (`native_living_breathe`, a free function in `horizon_client::breathing`).
//!
//! API 143 adds data map types a native mod registers (`native_data_map_types`), merged from the data packs' and mods' files each time the server's tables are built, read with `native_data_map` and `native_data_map_entries` (`native_data_maps.rs`).
//!
//! API 142 adds an energy buffer on a mod block's block entity (`native_block_entity_energy`), and the energy helpers in `machines/native_energy.rs`: `native_energy`, `native_energy_insert`, `native_energy_extract`, `native_move_energy`.
//!
//! API 141 adds a player starting and stopping tracking an entity (`native_player_start_tracking`, `native_player_stop_tracking`).
//!
//! API 140 adds a mod's own icons on filled maps (`native_map_decorations`).
//!
//! API 139 adds HUD layers around vanilla's own (`native_gui_layer_pre`, `native_gui_layer_post`).
//!
//! API 138 adds a cancellable attack, use and pick-block key hook (`native_interaction_key`).
//!
//! API 137 adds an entity getting on or off a vehicle, which a mod can cancel (`native_entity_mount`).
//!
//! API 136 adds a projectile's impact a mod can cancel (`native_projectile_impact`, heard once a mod sets `native_hears_projectile_impacts`).
//!
//! API 135 adds the client's level made and dropped (`native_client_level_load`, `native_client_level_unload`).
//!
//! API 134 adds a server's levels opening and closing (`native_level_load`, `native_level_unload`).
//!
//! API 133 adds a loot table's function and condition of a mod's
//! own kind (`native_loot_function` and `native_loot_condition`, which the
//! unknown `type` in a namespace of its own reaches with the kind, its JSON,
//! the loot context and the stack or the holds).
//!
//! API 132 adds other players' inventories for server mods, online or from their file (`native_inventory`, `native_move_stack`), and maps a mod draws (`native_send_map`).
//!
//! API 131 adds an item's use tick and release (`native_living_entity_use_item_tick`,
//! `native_on_use_tick`, `native_living_entity_use_item_stop`, `native_release_using`).
//!
//! API 130 adds a content mod menu type's slot rules decided in code (`native_menu_slot_rules`, in
//! `menu.rs`: whether a slot takes a stack, `Slot.mayPlace`, and how many it holds,
//! `Slot.getMaxStackSize`), with mod menu types' clicks on the vanilla menu engine.
//!
//! API 129 adds mod block entities on any template (`native_block_entity`).
//!
//! API 128 adds experience orb merging a mod decides (`native_experience_orb_can_merge`, `native_experience_orb_merge`) and writes (`native_set_experience_orb`).
//!
//! API 127 adds column reads for server mods (`native_surface_heights`, `native_read_columns`).
//!
//! API 126 adds bulk block edits for server mods (`native_edit_blocks`).
//!
//! API 125 adds knockback a mod can change or cancel (`native_living_knockback`).
//!
//! API 124 adds the players with a mod container open (`native_mod_container_viewers`).
//!
//! API 123 adds a screen drag hook (`native_screen_mouse_dragged_pre`).
//!
//! API 122 adds a two-axis gameplay scroll hook (`native_mouse_scrolling`).
//!
//! API 121 adds a complete window keyboard event (`native_input_key`).
//!
//! API 120 adds a server's recipes sent to its Horizon clients: the server's mods name the recipe types
//! (`native_datapack_sync`, `OnDatapackSyncEvent.sendRecipes`) and the client hears they arrived
//! (`native_recipes_received`, `RecipesReceivedEvent`).
//!
//! API 119 adds music for client mods: the track and its volume (`native_music`) and looping layers over it (`native_music_layers`).
//!
//! API 118 adds a screen key release hook (`native_screen_key_released_pre`).
//!
//! API 117 adds a final world fog hook (`native_render_fog`).
//!
//! API 116 adds a client mod's view and its own items (`native_raycast`, `native_view_hit`,
//! `native_claim_items`, `native_poll`).
//!
//! API 115 adds a player respawning (`native_player_respawned`).
//!
//! API 114 adds raw mouse button pre/post hooks (`native_mouse_button_pre`, `native_mouse_button_post`).
//!
//! API 113 adds a computed fog color hook (`native_compute_fog_color`).
//!
//! API 112 adds a detached-camera distance hook (`native_camera_distance`).
//!
//! API 111 adds a native mod's own screen (`native_mod_screen_open`, `native_mod_screen_close`,
//! `native_mod_screen`) and two drawing helpers for the screen hooks (`native_quad`,
//! `native_item_model`).
//!
//! API 110 adds mobs drawn by client mods (`native_mob_submit`, with `NativeMobRender`).
//!
//! API 109 adds a world field-of-view hook (`native_compute_fov`).
//!
//! API 108 adds the server's data tables are built (`native_server_data_loaded`, `TagsUpdatedEvent` with
//! `SERVER_DATA_LOAD`): a world's first open and every `/reload`, told apart by `reload`.
//!
//! API 107 adds biome colours for client mods (`native_biome_colors`, `native_refresh_biome_colors`).
//!
//! API 106 adds data pack files for the tables a native mod contributes (`native_data_pack_files`), on a world's first open and on every `/reload`.
//!
//! API 105 adds a push for the local player and a player's flying speed (`native_push_player`,
//! `native_set_flying_speed`).
//!
//! API 104 adds the open screen hears of a key press (`native_screen_key_pressed_pre`, the
//! `ScreenEvent.KeyPressed.Pre` point, with the screen, vanilla's key name, whether it is an
//! auto-repeat and the modifiers) and of each character it types (`native_screen_char_typed_pre`,
//! `ScreenEvent.CharacterTyped.Pre`, fired after the key has acted, on the screen open then), so a
//! cancelled press goes no further and a cancelled character never reaches a text box. The helpers
//! both paths share (`screen_key_pressed`, `screen_text_typed`, `screen_chars_typed`,
//! `screen_text_hooked`) sit beside them.
//!
//! API 103 adds a mod's own particle types for `native_send_particles`
//! (`ParticleOptions::named`).
//!
//! API 102 adds lead drops to `native_loot_drops`: untying a lead or cutting it with shears, and the lead a mob a player
//! killed while leashed drops on its next tick, with the player named.
//!
//! API 101 adds experience for server mods: reading orbs (`native_experience_orbs`), giving points as an orb does (`native_give_experience`) and giving or taking levels (`native_give_levels`).
//!
//! API 100 adds player-use drops to `native_loot_drops` (`NativeLootDrops::used`): shearing, brushing, carving a
//! pumpkin, picking berries and breaking an item frame, with the player named.
//!
//! API 99 adds a mod's command typed on the dedicated server's console (`native_perform_console_command`).
//!
//! API 98 adds giving a whole stack to any player from a server hook (`native_give_stack`).
//!
//! API 97 adds reading the world's light, weather, time of day, height and item stack sizes from a native mod (`native_light`, `native_weather`, `native_day_time`, `native_world_height`, `native_max_stack_size`).
//!
//! API 96 adds reading redstone from a server mod's block (`native_neighbor_signal`).
//!
//! API 95 adds any command a player types, cancellable (`native_player_command`).
//!
//! API 94 adds keeping a mod block entity's geometry while it doesn't change
//! (`BlockEntityDraw::keep`).
//!
//! API 93 adds the open screen hears of the wheel before it scrolls (`native_screen_mouse_scrolled_pre`,
//! the `ScreenEvent.MouseScrolled.Pre` point, with the screen, the scroll in x and y as the game's
//! scroll handler scales it, and the modifiers), so a cancelled turn scrolls nothing.
//!
//! API 92 adds the world's clock on a client (`native_clock`).
//!
//! API 91 adds items and block state properties for drawing mod block entities
//! (`native_draw_item`, `native_block_state_property`).
//!
//! API 90 adds using an item over time: its duration and hand pose (`native_use_duration`,
//! `native_use_animation`), `native_can_eat`, and its start and finish
//! (`native_living_entity_use_item_start`, `native_living_entity_use_item_finish`).
//!
//! API 89 adds custom item icon decorations (`native_item_decorations`).
//!
//! API 88 adds a client resource-pack reload completion notification (`native_client_resources_reloaded`).
//!
//! API 87 adds a cancellable first-person hand render hook (`native_render_hand`).
//!
//! API 86 adds a cancellable targeted-block highlight hook (`native_block_highlight`).
//!
//! API 85 adds menus of content mods' own menu types for mod containers (the `menu` of a
//! `ModContainer` may name one).
//!
//! API 84 adds values for a mod's own shader pack (`native_shader_params`).
//!
//! API 83 adds client sounds that wait (`native_play_local_sound`).
//!
//! API 82 adds camera shake for client mods (`native_camera_shake`).
//!
//! API 81 adds falling blocks for server mods (`native_spawn_falling_block`,
//! in `native_falling.rs`).
//!
//! API 80 adds the open screen hears of a mouse button going down or up before it acts
//! (`native_screen_mouse_button_pressed_pre`, `native_screen_mouse_button_released_pre`, the
//! `ScreenEvent.MouseButtonPressed.Pre` and `.MouseButtonReleased.Pre` points, with the screen, the
//! GLFW button and the modifiers), and the helpers the input hooks share (`screen_view_now`,
//! `screen_hooks_open`).
//!
//! API 79 adds items' default components (`native_modify_default_components`, in `horizon-world`'s
//! `item_defaults.rs`): set or remove a component of any item, vanilla's included, as the game reads its defaults.
//!
//! API 78 adds `native_screen_render_post` (`ScreenEvent.Render.Post`: draw over the open
//! screen, given its id, menu type, title, container panel, slot count, mouse
//! and partial tick).
//!
//! API 77 adds a `minecraft:post_attack` enchantment effect of a mod's own kind
//! (`native_enchant_effect`, told the enchantment, the kind, the JSON, the level and
//! the mob hit).
//!
//! API 76 adds a mod block's placement state (`native_state_for_placement`) and the state it is built from
//! (`native_state_with_property`): what a block that faces the player, sits on the clicked face or comes waterlogged needs.
//!
//! API 75 adds changing vanilla's loot tables and drops: `native_loot_table_load` (in
//! `horizon-world`'s `loot.rs`: each table's JSON as it loads) and `native_loot_drops` (a broken
//! block's or a dead mob's items, with what dropped them).
//!
//! API 74 adds a consumable's effect of a mod's own kind
//! (`native_consume_effect`, which the unknown `on_consume_effects` type reaches with
//! the kind and the JSON the file wrote).
//!
//! API 73 adds mod-block entity contacts (`native_block_step_on`, `native_block_entity_inside`, `native_block_fall_on`), including mutable landing damage.
//!
//! API 72 adds bone meal on a mod block (`native_is_valid_bonemeal_target`, `native_is_bonemeal_success`,
//! `native_perform_bonemeal`), and the level's random a world hook draws from (`native_level_random`).
//!
//! API 71 adds drawing mod block entities on the client (`native_block_entity_submit`,
//! vanilla's `BlockEntityRenderer.submit`, culled by `native_block_entity_render_bounding_box`,
//! NeoForge's `getRenderBoundingBox`), with a pose stack (`BlockEntityDraw`) and the
//! helpers `native_draw_quad`, `native_draw_cube` and `native_draw_block`.
//!
//! API 70 adds a mod block's own `canSurvive` (`native_can_survive`), and the blocks a mod plant
//! can stand on (`native_may_place_on`, `VegetationBlock.mayPlaceOn`).
//!
//! API 69 adds a mod's own advancement criteria triggers (`native_trigger_criterion`,
//! in `native_criteria.rs`).
//!
//! API 68 adds stack-dependent image tooltip components, measured with `native_tooltip_component` and rendered with `native_tooltip_component_draw`.
//!
//! API 67 adds dispenser item behaviours (`native_dispense_item`) and `native_dispense_default` for a plain drop.
//!
//! API 66 adds explosions for server mods (`native_explode`, in
//! `native_explosions.rs`).
//!
//! API 65 adds native item component type registration (`native_component_types`) and opaque NBT component helpers.
//!
//! API 64 adds comparator output from a mod's block (`native_has_analog_output`,
//! `native_analog_output`, and the helper `native_update_output_neighbors`).
//!
//! API 63 adds context-free tag lookups for server hooks (`native_server_block_in_tag`, `native_server_item_in_tag`).
//!
//! API 62 adds nicknames for server mods: `native_set_nickname` shows a name in place of a player's in chat and
//! the Tab list (the real one on hover), and `native_text_allowed` asks the server's chat filter, in
//! `native_server.rs`.
//!
//! API 61 adds mods' own sound events in `native_play_sound`: another
//! namespace's event is sent by name, as vanilla sends an unregistered sound.
//!
//! API 60 adds the small bar under an item (`native_item_bar`): `GuiGraphicsExtractor.itemDecorations`
//! hands it vanilla's choice (the damage bar, or a bundle's fullness already applied), to replace or hide.
//!
//! API 59 adds entities for server mods (`native_summon`, `native_move_entity`,
//! `native_set_display`, `native_remove_entity`, in `native_entities.rs`).
//!
//! API 58 adds the renderer's hook points, in `horizon-client-render`'s
//! `gfx/native_render.rs`: `native_world_drawn` (draw with the world's colour,
//! depth, matrices and entity geometry before the hand and the HUD) and
//! `native_render_needs` (entity runs and the world's depth without a pack).
//!
//! API 57 adds chunks and players coming and going (`native_chunk_load`,
//! `native_chunk_unload`, `native_player_logged_in`, `native_player_logged_out`).
//!
//! API 56 adds what a content mod's mob effects do: their effect tick
//! (`native_should_apply_effect_tick_this_tick`, `native_apply_effect_tick`)
//! and their being given (`native_on_effect_added`,
//! `native_mob_effect_added`).
//!
//! API 55 adds moving a block's fluids to another block (`native_move_fluids`, in
//! `machines/native_fluids.rs`: what the target refuses goes back into the
//! source, whole thirds kept).
//!
//! API 54 adds fluid tanks on a mod block's block entity
//! (`native_block_entity_fluids`), and the fluid helpers in
//! `machines/native_fluids.rs`: `native_fluid_tanks`,
//! `native_fluid_insert`, `native_fluid_extract`, which also read and
//! change a cauldron.
//!
//! API 53 adds entities leaving a level (`native_entity_leave_level`), for
//! every kind the join hook tells (see API 47).
//!
//! API 52 adds `native_block_entity_sync`: a mod block entity's block and update tag
//! sent to clients again (`Level.sendBlockUpdated`).
//!
//! API 51 adds `native_menu_button_v2`: a mod container's menu button with the clicking player's cursor stack (`clickMenuButton(Player, int)`).
//!
//! API 50 adds damage a mod can change or cancel (`native_living_incoming_damage`).
//!
//! API 49 adds, in `native_server.rs`, a sidebar and Tab list lines for one
//! player (`native_sidebar`, `native_tab_list`), the server's tick times
//! (`native_tick_stats`), players left out of the sleep count
//! (`native_sleep_ignored`), and where players look (`yaw`, `pitch`).
//!
//! API 48 adds a block's items for server mods (`native_container_slots`,
//! `native_extract_item`, `native_move_items`, and `native_dimension` for a
//! machine's tick, in `machines/native_items.rs`).
//!
//! API 47 adds entities joining a level (`native_entity_join_level`); it tells
//! every kind the game keeps in its entity lists (items, orbs, mobs, boats,
//! minecarts, armor stands, TNT, falling blocks, projectiles, rockets,
//! tridents, end crystals, the dragon, cushions, paintings, item frames).
//!
//! API 46 adds bounded world-space rim and colored-light requests
//! (`native_visuals`) and opt-in texture-matched emissive material
//! requests (`native_emissive`).
//!
//! API 45 sends mod block entities to clients (`native_block_entity_update_tag`).
//!
//! API 44 adds the server starting and stopping (`native_server_about_to_start`,
//! `native_server_stopping`).
//!
//! API 43 adds the server level's random (`native_level_random`).
//!
//! API 42 adds the item entities near a block (`native_item_entities`,
//! `native_set_item_entity`).
//!
//! API 41 adds bundles' data files: `horizon_client::native_data!` compiles a file of a
//! bundle's `data/` folder in, and `native_built_data` is the folder its post-build step wrote.
//!
//! API 40 adds mod messages between a mod's server and client halves
//! (`native_net_from_client`, `native_net_from_server`, and
//! `native_net_send_to_player`, `native_net_send_to_server` in
//! `native_net.rs`), and hearing players and mobs: `native_player_died`,
//! `native_mob_spawned`, `native_mob_killed`, `native_item_dropped`,
//! `native_item_picked_up`.
//!
//! API 39 adds item behaviour: a held item's hit on a mob
//! (`native_hurt_enemy`), a carried stack's tick (`native_inventory_tick`) and
//! an equipment slot taking a different item (`native_on_equip`).
//!
//! API 38 adds redstone from a mod's block (`native_block_signal`,
//! `native_is_signal_source`, and the helper `native_update_signal_neighbors`).
//!
//! API 37 adds mobs for server mods (`native_spawn_mob`, `native_mob`,
//! `native_mobs`, `native_teleport_mob`, `native_push_mob`,
//! `native_set_mob_health`, `native_kill_mob`, `native_remove_mob`... in
//! `native_mobs.rs`).
//!
//! API 36 adds recipes of a mod's own types (`native_recipes_of_type`,
//! `native_recipe_ingredient`, `native_recipe_result`).
//!
//! API 35 adds the commands a mod owns on the client (`native_commands`), so
//! that the chat box colours, completes and lists them like the game's own.
//!
//! API 34 adds scheduled block ticks a mod can handle (`native_tick_block`).
//!
//! API 33 adds random ticks a mod can handle (`native_random_tick`), and
//! `native_set_block_property` on the level, which takes `Level.setBlock`'s
//! `flags` as `Game::native_set_block_property_with` does.
//!
//! API 32 adds shape updates a mod can answer (`native_update_shape`), and
//! `native_schedule_block_tick` and `native_schedule_fluid_tick` on the
//! level for the ticks a block's shape update schedules.
//!
//! API 31 adds a player's server tick (`native_player_tick`).
//!
//! API 30 adds left-clicking a block (`native_left_click_block`).
//!
//! API 29 adds right-clicking an entity (`native_interact_entity`).
//!
//! API 28 adds block and item tag reads for server tick hooks (`native_block_in_tag`,
//! `native_item_in_tag`).
//!
//! API 27 adds the player-health HUD rows (`native_health_hud`).
//!
//! API 26 adds the game's shared config folder (`native_config_dir`).
//!
//! API 25 adds server sound playback (`native_play_sound`).
//!
//! API 24 adds a server block setter (`native_set_block`).
//!
//! API 23 adds per-level server tick hooks (`native_level_tick_pre`,
//! `native_level_tick_post`).
//!
//! API 22 adds what a mod needs to keep the world from changing blocks
//! it guards: fluid and fire spreading (`native_block_spread`), pistons
//! (`native_piston_move`), explosions (`native_explosion_detonate`),
//! buckets (`native_fill_bucket`) and a player's attack on an entity
//! (`native_attack_entity`).
//!
//! API 21 adds right clicking a block (`native_right_click_block`), and in
//! `native_server.rs` the acting player (`native_acting_player`), a player's
//! health, the action bar (`native_action_bar`) and commands run as the
//! server (`native_run_command`).
//!
//! API 20 adds buttons, tooltips and item icons on mod screens
//! (`native_screen_click`, `native_screen_tooltip`, `native_menu_button`,
//! and `native_item`, `native_screen_title`, `native_data`, `native_block`,
//! `native_block_property`, `native_insert_item`, `native_cursor`).
//!
//! API 19 adds mod-drawn screens for mod blocks (`native_screen`,
//! `native_screen_draw`, and the `native_sprite` helpers), with the mod's own
//! GUI textures.
//!
//! API 18 adds what a machine's tick reaches: a neighbouring mod block entity
//! (`native_with_mod_block_entity`) and its own block's state
//! (`native_set_block_property`).
//!
//! API 17 adds menus for mod containers (the `menu` of a `ModContainer`: vanilla
//! slot grids and furnaces) and has `native_block_entity_tick` take the whole
//! `ModBlockEntity`, with the menu's data slots.
//!
//! API 16 adds item slots for those block entities
//! (`native_block_entity_container`), which hoppers, comparators and
//! breaking the block use as they use vanilla's containers.
//!
//! API 15 adds block entities for a mod's blocks (`native_block_entity`,
//! `native_block_entity_ticks`, `native_block_entity_tick`): made when the
//! block is placed, ticked on the server and saved with the chunk.
//!
//! API 14 adds breaking and placing a block (`native_destroy_block`,
//! `native_place_block`).
//!
//! API 13 adds using an item (`native_use_item_on`, `native_use_item`).
//!
//! API 12 adds item tooltips (`native_tooltip`).
//!
//! API 11 adds server particle emission (`native_send_particles`).
//!
//! API 10 adds server menus (`native_menu_clicked`, `native_menu_closed`,
//! and `native_open_menu`, `native_exchange_items`... in `native_server.rs`).
//!
//! API 9 adds the acting player's plain items (`native_held`,
//! `native_count_items`, `native_take_items`, `native_give_items`... in
//! `native_server.rs`).
//!
//! API 8 adds server commands and players (`native_register_commands`,
//! `native_perform_command`, and the helpers in `native_server.rs`).
//!
//! API 7 adds block-change observations (`native_block_changed`).
//!
//! API 6 adds neighbour-change observations (`native_neighbor_changed`).
//!
//! API 5 adds read-only server block queries (`native_server_block`).
//!
//! API 4 adds authoritative server tick hooks (`native_server_tick`).
//!
//! API 3 adds the world lifecycle hook (`native_world_lifecycle`).
//!
//! API 2 adds the item panel beside container screens and its recipe pages
//! (`native_panel_items`, `native_panel_recipes`, `native_panel_event`),
//! and the game's items and recipes (`native_recipes`,
//! `native_known_recipes`, `native_place_recipe`, `native_give`): what a
//! recipe viewer needs.
//!
//! Names, parameters and call sites are kept from release to release, and
//! hook points are only ever added: each addition raises [`NATIVE_HOOKS_API`].
//! A bundle that hooks only these names the API it needs (`hooks`), and the
//! launcher builds it on any Horizon whose API is at least that one. A change
//! that would break such a bundle is not made.
//! Bodies have no `cfg` and no macros: horizon-mixins refuses a target with
//! either (`tests::hook_points_stay_hookable`).
use super::native_data_maps::NativeDataMapType;
use super::native_item_energy::ItemEnergyBuffer;
use super::{
    Game, GameLevel, LevelDimension, Screen,
    execution_role::ExecutionRole,
    maps::{NativeMapDecoration, NativeMapInfo},
    native_block_entity_draw::BlockEntityDraw,
    native_mob_draw::{NativeMobRender, NativePlayerRender},
    native_panel_recipes::RecipePage,
};
use horizon_client::mod_api::{
    self, NativeCamera, NativeCommand, NativeDispensed, NativeLifecycleContext, NativeServerPlayer,
    RecipeBookCategory, ServerTickPhase, WorldPhase,
};
use horizon_client::mod_api::{AttachmentResult, Attachments, Holder, Suggestion};
pub(crate) use horizon_client::mod_api::{
    NativeBlockRim, NativeColoredLight, NativeLootDrops, NativeMinimap, NativeMod,
    NativePanelEvent, NativeScreen, NativeScreenView, NativeStack, NativeZoom,
};
use horizon_client::{
    atlas_sources::qualified,
    biome::BiomeColors,
    block_entities::Pos,
    chat::ChatSource,
    containers::{
        BlockEntity, MOD_MENUS, ModBlockEntity, ModBlockEntityFields, ModContainer,
        ModEnergyBuffer, ModFluidTanks,
    },
    crafting::{CustomRecipe, Ingredient, recipe_result},
    extras::Corner,
    extras_hud::Corners,
    fast_hash::FxHashMap,
    first_person::Hand,
    health::{HurtSource, Hurting, Knock},
    hud::{HudBatch, rgba},
    items::ItemStack,
    keybinds,
    lan::menu::Slots,
    merchant::MerchantOffer,
    mobs::{Damage, Who},
    mod_list::{self, Installed, NativePage},
    music::{Music, MusicLayer},
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    ticks::{ScheduleResult, TickPriority},
};
use horizon_world::{
    consumable::UseAnimation,
    fluid::{FluidKind, FluidState},
    level::{BlockId, LevelAccess, PlacementContext, Ticks},
    nbt::Compound,
    registry::{BlockRegistry, BlockStateId},
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use winit::{event::MouseButton, keyboard::KeyCode};

pub(super) struct ObservedScreen {
    id: &'static str,
    menu: Option<String>,
    title: Option<Text>,
    rect: [f32; 4],
    slots: usize,
    mouse: [f32; 2],
    partial_tick: f32,
}

impl ObservedScreen {
    fn view(&self) -> NativeScreenView<'_> {
        NativeScreenView {
            id: self.id,
            menu: self.menu.as_deref(),
            title: self.title.as_ref(),
            rect: self.rect,
            slots: self.slots,
            mouse: self.mouse,
            partial_tick: self.partial_tick,
        }
    }
}

fn observed_overlay_id(overlays: &[(bool, &'static str)]) -> Option<&'static str> {
    overlays.iter().find_map(|(open, id)| open.then_some(*id))
}

impl Game<'_> {
    fn observed_screen_view(&self) -> Option<NativeScreenView<'_>> {
        let book = match &self.book {
            Some(super::books::BookScreen::Edit { signing: true, .. }) => Some("book_sign"),
            Some(super::books::BookScreen::Edit { .. }) => Some("book_edit"),
            Some(super::books::BookScreen::View {
                lectern: Some(_), ..
            }) => Some("lectern"),
            Some(super::books::BookScreen::View { .. }) => Some("book_view"),
            None => None,
        };
        let overlay = observed_overlay_id(&[
            (self.command_prompt.is_some(), "command_prompt"),
            (self.link_prompt.is_some(), "link_prompt"),
            (self.code_of_conduct.is_some(), "code_of_conduct"),
            (self.server_packs.prompt.is_some(), "server_pack_prompt"),
            (self.dialog_open(), "dialog"),
            (book.is_some(), book.unwrap_or("book_view")),
            (self.sign_edit.is_some(), "sign_editor"),
        ]);
        let Some(id) = overlay else {
            return self.screen_view_now();
        };
        let renderer = self.renderer.as_ref()?;
        let batch = HudBatch::new([renderer.config.width, renderer.config.height]);
        Some(NativeScreenView {
            id,
            menu: None,
            title: None,
            rect: [0.0, 0.0, batch.width, batch.height],
            slots: 0,
            mouse: self.native_cursor(),
            partial_tick: self.clock.fraction() as f32,
        })
    }

    pub(super) fn observe_screen(&mut self) {
        // Free with no native mod: the snapshot clones the title and menu.
        if mod_api::mods().is_empty() {
            return;
        }
        let current = self.observed_screen_view().map(|view| ObservedScreen {
            id: view.id,
            menu: view.menu.map(str::to_owned),
            title: view.title.cloned(),
            rect: view.rect,
            slots: view.slots,
            mouse: view.mouse,
            partial_tick: view.partial_tick,
        });
        let changed = match (&self.observed_screen, &current) {
            (Some(old), Some(new)) => old.id != new.id || old.menu != new.menu,
            (None, None) => false,
            _ => true,
        };
        if changed {
            if let Some(old) = &self.observed_screen {
                self.native_screen_closing(&old.view());
            }
            if let Some(new) = &current {
                self.native_screen_opening(&new.view());
                self.native_screen_init_post(&new.view());
            }
        } else if let (Some(old), Some(new)) = (&self.observed_screen, &current)
            && old.rect != new.rect
        {
            self.native_screen_init_post(&new.view());
        }
        self.observed_screen = current;
    }
}

/// `Level.setBlock`'s flags, for the `native_set_block_property` helpers:
/// `flags::UPDATE_CLIENTS` as a vanilla crop grows, `flags::UPDATE_ALL` as a
/// furnace sets its `lit`. Horizon sends every write to players, so
/// `UPDATE_CLIENTS`, `UPDATE_INVISIBLE` and `UPDATE_IMMEDIATE` change nothing
/// yet.
pub(super) use horizon_world::level::flags;

/// The hook points' API version. The launcher reads this line from the
/// source it builds; keep its form.
#[allow(dead_code)]
pub(crate) const NATIVE_HOOKS_API: u32 = 359;

impl GameLevel<'_> {
    /// A hopper at `pos` is about to pick up `count` of `item`. Set `cancel` to leave the dropped stack in the world. Server-side. 232.
    #[inline(always)]
    pub(super) fn native_hopper_pickup_item(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        item: &str,
        count: u32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_hopper_pickup_item(
            self,
            mod_api::mods(),
            dimension,
            pos,
            item,
            count,
            cancel,
        );
    }
    /// A broken block is about to drop `experience` points at `pos`. The
    /// vanilla block drop rule has been checked; change the amount before its
    /// orbs are awarded. Server-side. 190.
    #[inline(always)]
    pub(super) fn native_block_exp(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        block: &str,
        experience: &mut u32,
    ) {
        mod_api::dispatch_block_exp(self, mod_api::mods(), dimension, pos, block, experience);
    }

    /// An authoritative server committed a changed state through `Level.setBlock`.
    /// The world contains `new`; block side effects and neighbour updates follow.
    #[inline(always)]
    pub(super) fn native_block_changed(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
    ) {
        mod_api::dispatch_block_changed(self, mod_api::mods(), dimension, pos, old, new);
    }

    /// A flowing fluid at `pos` is about to change level. `fluid` is `water`
    /// or `lava`; the levels are 0 through 8. Set `cancel` to keep the old
    /// level. Server-side. 226.
    #[inline(always)]
    pub(super) fn native_fluid_level_change(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        fluid: &str,
        from: i32,
        to: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_fluid_level_change(
            self,
            mod_api::mods(),
            dimension,
            pos,
            fluid,
            from,
            to,
            cancel,
        );
    }

    /// A non-persistent leaf block at distance 7 is about to decay at `pos`.
    /// Set `cancel` to keep it. Server-side. 235.
    #[inline(always)]
    pub(super) fn native_leaves_decay(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_leaves_decay(self, mod_api::mods(), dimension, pos, cancel);
    }

    /// Farmland at `pos` is about to change moisture from `from` to `to`.
    /// Set `cancel` to keep its old moisture. Server-side.
    /// 237.
    #[inline(always)]
    pub(super) fn native_moisture_change(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        from: i32,
        to: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_moisture_change(self, mod_api::mods(), dimension, pos, from, to, cancel);
    }

    /// A sponge at `pos` absorbed water and became wet. Server-side.
    /// 297.
    #[inline(always)]
    pub(super) fn native_sponge_absorb(&self, dimension: LevelDimension, pos: BlockPos) {
        mod_api::dispatch_sponge_absorb(self, mod_api::mods(), dimension, pos);
    }

    /// A block at `pos` is about to fade from `from` to `to` because of
    /// melting or coral death. Set `cancel` to keep it. Server-side.
    /// 191.
    #[inline(always)]
    pub(super) fn native_block_fade(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_block_fade(self, mod_api::mods(), dimension, pos, from, to, cancel);
    }

    /// Precipitation is about to form or add to a block at `pos`. `from` and
    /// `to` are block state IDs. Set `cancel` to keep the old state. Server-side.
    /// 193.
    #[inline(always)]
    pub(super) fn native_block_form(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_block_form(self, mod_api::mods(), dimension, pos, from, to, cancel);
    }

    /// The Ender Dragon fight is about to place its first dragon egg at `pos`.
    /// Set `cancel` to leave the block unchanged. Server-side. 208.
    #[inline(always)]
    pub(super) fn native_dragon_egg_form(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_dragon_egg_form(self, mod_api::mods(), dimension, pos, cancel);
    }

    /// An authoritative server is about to deliver `neighborChanged`.
    /// State and block IDs are valid in this level's registries for this call.
    #[inline(always)]
    pub(super) fn native_neighbor_changed(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        changed: BlockId,
        moved: bool,
    ) {
        mod_api::dispatch_neighbor_changed(
            self,
            mod_api::mods(),
            dimension,
            pos,
            state,
            changed,
            moved,
        );
    }

    /// On an authoritative server, a fluid flows or fire spreads from `from`
    /// into `to` (`kind` is `fluid` or `fire`): `FlowingFluid.spreadTo`, and
    /// `FireBlock.tick` setting fire to `to` or burning it (`checkBurnOut`).
    /// Set `canceled` to leave the block at `to` as it is. Fluids ask on
    /// every flow: answer fast. API 22.
    #[inline(always)]
    pub(super) fn native_block_spread(
        &self,
        dimension: LevelDimension,
        kind: &str,
        from: BlockPos,
        to: BlockPos,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_block_spread(self, mod_api::mods(), dimension, kind, from, to, canceled);
    }

    /// On an authoritative server, fire is about to be placed at `pos` by
    /// flint and steel, a fire charge, lava, lightning or fire spread. Set
    /// `player` is the acting server player when there is one. Set `cancel` to
    /// leave the position unchanged. 194.
    #[inline(always)]
    pub(super) fn native_block_ignite(
        &self,
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        cause: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_block_ignite(
            self,
            mod_api::mods(),
            dimension,
            player,
            pos,
            cause,
            cancel,
        );
    }

    /// On an authoritative server, fire is about to burn the block at `pos`
    /// away. Set `cancel` to keep that block. 187.
    #[inline(always)]
    pub(super) fn native_block_burn(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_block_burn(self, mod_api::mods(), dimension, pos, cancel);
    }

    /// On an authoritative server, a note block at `pos` is about to sound.
    /// `player` is the acting server player when there is one. Set `cancel` to silence it.
    /// 248.
    #[inline(always)]
    pub(super) fn native_note_play(
        &self,
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        instrument: &str,
        note: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_note_play(
            self,
            mod_api::mods(),
            dimension,
            player,
            pos,
            instrument,
            note,
            cancel,
        );
    }

    /// On an authoritative server, a bell at `pos` is about to ring. Set
    /// `cancel` to stop the ring and its raider reveal. 185.
    #[inline(always)]
    pub(super) fn native_bell_ring(
        &self,
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_bell_ring(self, mod_api::mods(), dimension, player, pos, cancel);
    }

    /// On an authoritative server, a cauldron at `pos` is about to change
    /// fill level from `from_level` to `to_level` for `reason`. Set `cancel`
    /// to leave it unchanged. 203.
    #[inline(always)]
    pub(super) fn native_cauldron_level_changed(
        &self,
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        from_level: i32,
        to_level: i32,
        reason: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_cauldron_level_changed(
            self,
            mod_api::mods(),
            dimension,
            player,
            pos,
            from_level,
            to_level,
            reason,
            cancel,
        );
    }

    /// On an authoritative server, the piston at `piston` is about to push
    /// the blocks at `push` one step `direction` (`[0, 1, 0]` is up; pulling
    /// when it retracts) and destroy those at `destroy`:
    /// `PistonBaseBlock.moveBlocks`, once its structure resolved. Set
    /// `canceled` and nothing moves, as when the structure can't; a sticky
    /// piston then retracts without its block, and clients are sent the
    /// blocks rather than the retraction to replay. API 22.
    #[inline(always)]
    pub(super) fn native_piston_move(
        &self,
        dimension: LevelDimension,
        piston: BlockPos,
        direction: [i32; 3],
        push: &[BlockPos],
        destroy: &[BlockPos],
        canceled: &mut bool,
    ) {
        mod_api::dispatch_piston_move(
            self,
            mod_api::mods(),
            dimension,
            piston,
            direction,
            push,
            destroy,
            canceled,
        );
    }
    /// The redstone signal the block `state` at `pos` gives on its `face`, the
    /// side that touches the block reading it: `BlockState.getDirectSignal`
    /// when `direct` (what passes through a block it touches, as a lever's
    /// does), else `getSignal`. `signal` holds vanilla's answer: change it when
    /// the block is yours (0 to 15; anything else is clamped). Asked very often,
    /// from inside the level's update, so answer from your own state: check the
    /// block first, and don't read or change the world here. When the answer
    /// changes, and once the block is placed, call
    /// `native_update_signal_neighbors`. Authoritative servers only. API 38.
    #[inline(always)]
    pub(super) fn native_block_signal(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        face: Direction,
        direct: bool,
        signal: &mut i32,
    ) {
        mod_api::dispatch_block_signal(
            self,
            mod_api::mods(),
            dimension,
            pos,
            state,
            face,
            direct,
            signal,
        );
    }

    /// `BlockState.isSignalSource`: set `source` for a block of yours that
    /// gives a signal, so redstone wire beside it connects to it. Authoritative
    /// servers only. API 38.
    #[inline(always)]
    pub(super) fn native_is_signal_source(&self, state: BlockStateId, source: &mut bool) {
        mod_api::dispatch_is_signal_source(self, mod_api::mods(), state, source);
    }

    /// `BlockState.hasAnalogOutputSignal`: set `output` for a block of yours
    /// that comparators read (how full or far along it is, as a composter's
    /// level), so they ask `native_analog_output`. Asked from inside the
    /// level's update: answer from `state` alone. Authoritative servers only.
    /// API 64.
    #[inline(always)]
    pub(super) fn native_has_analog_output(&self, state: BlockStateId, output: &mut bool) {
        mod_api::dispatch_has_analog_output(self, mod_api::mods(), state, output);
    }

    /// `BlockState.getAnalogOutputSignal`: the signal a comparator reads from
    /// the block `state` at `pos`, `face` the side that touches the comparator
    /// (or the conductor in front of it). `output` holds vanilla's answer:
    /// change it when the block is yours (0 to 15; anything else is clamped).
    /// Asked only for blocks `native_has_analog_output` answered for, from
    /// inside the level's update: don't change the world here. When the
    /// answer changes without the block's state changing, call
    /// `native_update_output_neighbors`; a change of state through
    /// `native_set_block` or `native_set_block_property` (neighbour updates
    /// on) tells the comparators itself, as vanilla's `Level.setBlock` does.
    /// Authoritative servers only. API 64.
    #[inline(always)]
    pub(super) fn native_analog_output(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        face: Direction,
        output: &mut i32,
    ) {
        mod_api::dispatch_analog_output(self, mod_api::mods(), dimension, pos, state, face, output);
    }

    /// An authoritative server's `ServerLevel.tickChunk` is about to
    /// random-tick the block `state` at `pos` (`BlockState.randomTick`):
    /// only blocks that tick randomly, a mod's as its vanilla template block
    /// does. Bone meal's random ticks (`StemBlock.performBonemeal`) don't
    /// fire it. Change the block with `native_set_block_property` and set
    /// `handled` to skip the block's own random tick (a mod crop growing its
    /// own way); a block changed without `handled` takes its own random tick
    /// as it is now, if it still ticks randomly. Lava's fluid random tick
    /// runs either way.
    #[inline(always)]
    pub(super) fn native_random_tick(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        handled: &mut bool,
    ) {
        mod_api::dispatch_random_tick(
            &mut super::mod_api_host::LevelTicks { level: self, ticks },
            mod_api::mods(),
            dimension,
            pos,
            state,
            handled,
        );
    }

    /// An authoritative server is about to compute the state the block
    /// `state` at `pos` takes now that its neighbour in `direction` is
    /// `neighbour` (`BlockBehaviour.updateShape`). `state` may not be in the
    /// world yet (a piston landing it): decide from `state`, not from the
    /// block at `pos`. Answer only for your own blocks: put the new state in
    /// `new` (`state` keeps the block as it is; air breaks it, with its drops,
    /// on a neighbour update) and the block's own `updateShape` doesn't run,
    /// nor the ticks it would schedule. The level is read-only, as vanilla's
    /// `LevelReader`; ticks go through `native_schedule_block_tick` and
    /// `native_schedule_fluid_tick` with `ticks` (`ScheduledTickAccess`). It
    /// runs often, at least six times per `setBlock` that updates shapes
    /// (more for redstone wire's diagonals and knock-on updates): keep it
    /// short.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn native_update_shape(
        &self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
        new: &mut Option<BlockStateId>,
    ) {
        mod_api::dispatch_update_shape(
            &mut super::mod_api_host::LevelTicks { level: self, ticks },
            mod_api::mods(),
            dimension,
            pos,
            state,
            direction,
            neighbour_pos,
            neighbour,
            new,
        );
    }

    /// On an authoritative server, `BlockState.canSurvive` for `state` at
    /// `pos` (`BlockBehaviour.canSurvive`), before the block answers it: as
    /// it is placed (`BlockItem.canPlace`), from a shape update of a block
    /// whose template checks it (a plant, a torch, a sign: false breaks the
    /// block with its drops), and wherever else vanilla asks. `state` may not
    /// be in the world yet, so decide from `state`. Answer only for your own
    /// blocks: `Some(false)` refuses the spot, `Some(true)` allows it, and
    /// the block's own rule doesn't run. The level is read-only. Asked on
    /// every shape update of such a block: keep it short. API 70.
    #[inline(always)]
    pub(super) fn native_can_survive(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        survives: &mut Option<bool>,
    ) {
        mod_api::dispatch_can_survive(self, mod_api::mods(), dimension, pos, state, survives);
    }

    /// On an authoritative server, `VegetationBlock.mayPlaceOn` for the
    /// plant `state` on `ground` at `ground_pos`, before the plant answers
    /// it: the blocks a mod plant (one whose template is a vanilla plant,
    /// such as a crop) can stand on. Its template's `canSurvive` asks it, so
    /// the plant is refused there and pops off when its ground changes to
    /// one it can't stand on. Answer only for your own blocks. The level is
    /// read-only. API 70.
    #[inline(always)]
    pub(super) fn native_may_place_on(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        may: &mut Option<bool>,
    ) {
        mod_api::dispatch_may_place_on(
            self,
            mod_api::mods(),
            dimension,
            state,
            ground,
            ground_pos,
            may,
        );
    }

    /// An authoritative server's `ServerLevel.tickBlock` is about to run a
    /// due scheduled tick of the block `state` at `pos` (`BlockState.tick`):
    /// the block is still the one the tick was scheduled for. Change the
    /// block with `native_set_block_property` and set `handled` to skip the
    /// block's own tick; a block changed without `handled` takes its own
    /// tick as it is now, if it is still the tick's block.
    #[inline(always)]
    pub(super) fn native_tick_block(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        handled: &mut bool,
    ) {
        mod_api::dispatch_tick_block(
            &mut super::mod_api_host::LevelTicks { level: self, ticks },
            mod_api::mods(),
            dimension,
            pos,
            state,
            handled,
        );
    }

    /// A block is about to take the state it is placed in
    /// (`Block.getStateForPlacement(BlockPlaceContext)`), `state` the block's
    /// own default state. Answer only for your blocks: put the state you want
    /// in `answer` as `Some(Some(state))` — a state of that block, as vanilla
    /// sets properties on its own `defaultBlockState` (another block's is
    /// ignored) — or refuse the placement with `Some(None)`: the block goes
    /// nowhere and the stack is kept, as vanilla's `null`. Left empty, the
    /// block's own placement runs. `context` is
    /// vanilla's `BlockPlaceContext` (the clicked face, the placer's rotation,
    /// the clicked position, whether the clicked block is being replaced, the
    /// item in hand) and `fluid` the fluid in the position the block would go
    /// in, which a waterlogged block reads. Build the state from `state` with
    /// `native_state_with_property`, and know whether `state` is yours with
    /// `native_state_block`. It is asked for every placer that goes
    /// through vanilla's placement (a player's `BlockItem.place`, a
    /// dispenser's `DirectionalPlaceContext`), and the answer still takes its
    /// survival and entity obstruction checks. A trait mod returns
    /// `Some(Some(state))` to choose a state, `Some(None)` to refuse, or
    /// `None` to leave the answer to the next mod or vanilla.
    /// Authoritative servers only. 76.
    #[inline(always)]
    #[allow(clippy::option_option)]
    pub(super) fn native_state_for_placement(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        context: &PlacementContext,
        fluid: FluidState,
        answer: &mut Option<Option<BlockStateId>>,
    ) {
        mod_api::dispatch_state_for_placement(
            self,
            mod_api::mods(),
            dimension,
            state,
            context,
            fluid,
            answer,
        );
    }

    /// A block nothing in Horizon knows a `BonemealableBlock` for is about to
    /// take bone meal: vanilla's `BonemealableBlock.isValidBonemealTarget`,
    /// `state` the block's own state at `pos`. Answer `valid` for your own
    /// blocks: the meal is used up on one as vanilla uses it up (the growth
    /// particles and the sound follow, whether or not the roll below grows
    /// it), from a player's `BoneMealItem.useOn`, from a dispenser and, on a
    /// crop (a villager bonemeals only a `CropBlock`), from a villager. Asked
    /// on both sides, as vanilla asks it on both, so a client predicts the
    /// use with your answer: say no where you can only grow on a server
    /// (`is_client_side`). `Some(false)` refuses the meal, even on a block
    /// whose template takes it (vanilla's `growCrop` then keeps the meal);
    /// left `None`, the block keeps the `BonemealableBlock` its template gives
    /// it, or takes no bone meal.
    /// 72.
    #[inline(always)]
    pub(super) fn native_is_valid_bonemeal_target(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        valid: &mut Option<bool>,
    ) {
        mod_api::dispatch_is_valid_bonemeal_target(
            self,
            mod_api::mods(),
            dimension,
            pos,
            state,
            valid,
        );
    }

    /// `BonemealableBlock.isBonemealSuccess` for a block that answered
    /// `native_is_valid_bonemeal_target`, after it did: set `success` to grow
    /// it this time. Vanilla draws the level's random here
    /// (`isBonemealSuccess(Level, RandomSource, BlockPos, BlockState)`), so
    /// draw from `native_level_random` to keep vanilla's order.
    /// Authoritative servers only. 72.
    #[inline(always)]
    pub(super) fn native_is_bonemeal_success(
        &mut self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        success: &mut bool,
    ) {
        mod_api::dispatch_is_bonemeal_success(
            self,
            mod_api::mods(),
            dimension,
            pos,
            state,
            success,
        );
    }

    /// `BonemealableBlock.performBonemeal` for a block that answered
    /// `native_is_valid_bonemeal_target` and whose `native_is_bonemeal_success`
    /// said yes: grow it here, `state` its state at `pos`. The meal is used up
    /// either way. Change the block with the level's
    /// `native_set_block_property`, and schedule with `native_schedule_block_tick`
    /// (`ticks` is the hook's). Authoritative servers only. 72.
    #[inline(always)]
    pub(super) fn native_perform_bonemeal(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
    ) {
        mod_api::dispatch_perform_bonemeal(
            &mut super::mod_api_host::LevelTicks { level: self, ticks },
            mod_api::mods(),
            dimension,
            pos,
            state,
        );
    }

    /// On an authoritative server, how fire treats the block `state`
    /// (`FireBlock.setFlammable(block, igniteOdds, burnOdds)`): put
    /// `(ignite, burn)` in `odds` for your own blocks, and vanilla's table
    /// isn't read. Ignite odds above zero let fire stand beside the block and
    /// spread into the air around it (planks 5, wool 30, leaves 30); burn odds
    /// are how fast fire destroys it (planks 20, wool 60, leaves 60). `(0, 0)`
    /// never burns; keep both within vanilla's 0 to 100. A waterlogged state
    /// never burns and isn't asked. Answer only for your own blocks and leave
    /// `odds` empty otherwise; the level is read-only. Asked hundreds of times
    /// per fire tick: keep it short. API 157.
    #[inline(always)]
    pub(super) fn native_fire_odds(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        odds: &mut Option<(i32, i32)>,
    ) {
        mod_api::dispatch_fire_odds(self, mod_api::mods(), dimension, state, odds);
    }

    /// On an authoritative server, `CampfireBlock.isSmokeSource` for the
    /// block `state` under a campfire, as the campfire is placed or the block
    /// under it changes: `Some(true)` for your own blocks makes the campfire a
    /// signal fire whose smoke rises higher, as over a hay bale. The level is
    /// read-only. API 157.
    #[inline(always)]
    pub(super) fn native_smoke_source(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        smoke: &mut Option<bool>,
    ) {
        mod_api::dispatch_smoke_source(self, mod_api::mods(), dimension, state, smoke);
    }
}

/// The camera shake native mods ask for (`native_camera_shake`): turns in
/// degrees, and an offset in blocks along the camera's right, up and back.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NativeShake {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub offset: [f32; 3],
}

/// The largest turn a shake gives, in degrees, on each axis.
const SHAKE_MAX_DEGREES: f32 = 10.0;
/// The largest offset a shake gives, in blocks, along each axis.
const SHAKE_MAX_OFFSET: f32 = 0.5;

/// The pose a native mod asks for the first-person hand, each out-of-range
/// or non-finite component replaced by its identity.
pub(super) fn hand_pose_matrix(values: [f32; 7]) -> glam::Mat4 {
    let valid = |value: f32, min: f32, max: f32, default: f32| {
        if value.is_finite() && (min..=max).contains(&value) {
            value
        } else {
            default
        }
    };
    let [x, y, z, pitch, yaw, roll, scale] = values;
    glam::Mat4::from_translation(glam::Vec3::new(
        valid(x, -16.0, 16.0, 0.0),
        valid(y, -16.0, 16.0, 0.0),
        valid(z, -16.0, 16.0, 0.0),
    )) * glam::Mat4::from_rotation_x(valid(pitch, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_rotation_y(valid(yaw, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_rotation_z(valid(roll, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_scale(glam::Vec3::splat(valid(scale, 0.01, 100.0, 1.0)))
}

/// `shake` scaled by `scale` (Distortion Effects) and bounded on each axis;
/// a value that isn't finite counts as none.
fn bounded_shake(shake: NativeShake, scale: f32) -> NativeShake {
    let scale = if scale.is_finite() {
        scale.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let turn = |v: f32| {
        if v.is_finite() {
            (v * scale).clamp(-SHAKE_MAX_DEGREES, SHAKE_MAX_DEGREES)
        } else {
            0.0
        }
    };
    let shift = |v: f32| {
        if v.is_finite() {
            (v * scale).clamp(-SHAKE_MAX_OFFSET, SHAKE_MAX_OFFSET)
        } else {
            0.0
        }
    };
    NativeShake {
        pitch: turn(shake.pitch),
        yaw: turn(shake.yaw),
        roll: turn(shake.roll),
        offset: shake.offset.map(shift),
    }
}

/// The pose a bounded shake multiplies into the projection, after the view
/// bobbing: the offset, then roll, pitch and yaw.
fn shake_pose(shake: NativeShake) -> glam::Mat4 {
    let [x, y, z] = shake.offset;
    glam::Mat4::from_translation(glam::Vec3::new(-x, -y, -z))
        * glam::Mat4::from_rotation_z(shake.roll.to_radians())
        * glam::Mat4::from_rotation_x(shake.pitch.to_radians())
        * glam::Mat4::from_rotation_y(shake.yaw.to_radians())
}

/// The mod screen open on this client: the block it is for, the key its
/// slots are under, the window id of its menu, and the menu's data slots as
/// the server sent them.
pub(super) struct OpenNativeScreen {
    key: Pos,
    window: i32,
    block: String,
    screen: NativeScreen,
    data: Vec<i32>,
}

impl OpenNativeScreen {
    /// Data slot `id` of window `window`'s menu: false for another window's
    /// (an old or replaced one), which this screen leaves alone.
    fn set_data(&mut self, window: i32, id: i16, value: i16) -> bool {
        if window != self.window {
            return false;
        }
        // A menu's data slot ids are small; a bad one is dropped.
        if let Ok(i @ 0..256) = usize::try_from(id) {
            if self.data.len() <= i {
                self.data.resize(i + 1, 0);
            }
            self.data[i] = i32::from(value);
        }
        true
    }
}

/// The player as a native mod reads it.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativePlayer {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
    /// Degrees; 0 faces south, 90 west (`Direction.fromYRot`).
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub(crate) dimension: &'static str,
}

/// The world's clock as a client mod reads it (`native_clock`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeClock {
    /// Ticks since the world was created (`Level.getGameTime`).
    pub game_time: i64,
    /// The time of day in ticks (`Level.getOverworldClockTime`): 0 at sunrise, 24000 a
    /// day. It keeps counting past 24000 (`/time add`, days going by), so take
    /// it modulo 24000 for the hour; `/time set` can make it negative.
    pub day_time: i64,
}

/// How the panel lays out a recipe of `native_panel_recipes`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeRecipeLayout {
    /// A crafting grid of `width` x `height` slots, an arrow and the result.
    Grid,
    /// One input over a flame, an arrow and the result (furnaces, smokers,
    /// campfires).
    Cooking,
    /// One input, an arrow and the result (the stonecutter).
    Single,
    /// Template, base and addition, an arrow and the result.
    Smithing,
    /// A potion and an ingredient, an arrow and the result.
    Brewing,
}

/// A recipe the panel shows (`native_panel_recipes`). Each slot, and the
/// result, cycles through its stacks, one a second; an empty slot shows
/// nothing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeRecipeView {
    pub(crate) layout: NativeRecipeLayout,
    /// The grid's size (`Grid` only), at most 3 x 3.
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) slots: Vec<Vec<NativeStack>>,
    pub(crate) results: Vec<NativeStack>,
    /// A short line under the recipe (a cooking time), or "".
    pub(crate) note: String,
    /// The action button beside it: none, enabled, or shown greyed.
    pub(crate) action: Option<bool>,
}

/// An item of [`NativeRecipes::items`], by its index there, with a count
/// and the components it has beyond the item's own, in vanilla's item
/// syntax (`[minecraft:potion_contents={potion:"minecraft:swiftness"}]`),
/// or "".
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeRecipeStack {
    pub(crate) item: u32,
    pub(crate) count: u32,
    pub(crate) components: String,
}

/// The station a recipe is made at.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeRecipeKind {
    CraftingShaped,
    CraftingShapeless,
    Smelting,
    Blasting,
    Smoking,
    Campfire,
    Stonecutting,
    Smithing,
    Brewing,
}

/// A recipe of [`NativeRecipes`].
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeRecipe {
    /// `minecraft:oak_planks`.
    pub(crate) id: String,
    pub(crate) kind: NativeRecipeKind,
    /// The recipe book's tab (`crafting_building_blocks`, `furnace_food`),
    /// or "".
    pub(crate) category: String,
    /// Recipes the book shows on one button share a group, or "".
    pub(crate) group: String,
    /// A shaped recipe's grid: `slots` holds its cells row by row. 0 for
    /// the other kinds.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Each slot's stacks, any one of which fits; an empty list is an empty
    /// cell. Cooking and stonecutting: the input. Smithing: the template,
    /// the base, the addition. Brewing: the bottle, the reagent.
    pub(crate) slots: Vec<Vec<NativeRecipeStack>>,
    /// What it makes; several when that varies (a trim on any armour).
    pub(crate) results: Vec<NativeRecipeStack>,
    /// Cooking: game ticks and experience; 0 for the other kinds.
    pub(crate) cooking_time: u32,
    pub(crate) experience: f32,
    /// The recipe book's id for it (one per way the book shows it), as
    /// `native_known_recipes` lists them; -1 for the kinds the book doesn't
    /// show (campfire, stonecutting, smithing, brewing).
    pub(crate) book_id: i32,
}

/// An item of [`NativeRecipes::items`].
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeItem {
    /// `minecraft:diamond`, `<mod>:<path>`.
    pub(crate) id: String,
    /// In the player's language.
    pub(crate) name: String,
    /// Its item tags, without the `#`.
    pub(crate) tags: Vec<String>,
}

/// Every item and recipe the game knows (`native_recipes`): `items` in the
/// registry's order, which each [`NativeRecipeStack::item`] indexes; the
/// recipes are the book's in its order, then campfire, stonecutting,
/// smithing and brewing.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeRecipes {
    pub(crate) items: Vec<NativeItem>,
    pub(crate) recipes: Vec<NativeRecipe>,
}

/// The mappings `native_mapping` answers for, with the key their default
/// binding is (none: a mouse button, attack or use).
#[allow(dead_code)]
const MAPPINGS: [(&str, Option<KeyCode>); 9] = [
    ("key.forward", Some(KeyCode::KeyW)),
    ("key.left", Some(KeyCode::KeyA)),
    ("key.back", Some(KeyCode::KeyS)),
    ("key.right", Some(KeyCode::KeyD)),
    ("key.jump", Some(KeyCode::Space)),
    ("key.sneak", Some(KeyCode::ShiftLeft)),
    ("key.sprint", Some(KeyCode::ControlLeft)),
    ("key.attack", None),
    ("key.use", None),
];

/// What a `ScreenEvent.CharacterTyped.Pre` leaves of `text`, given `canceled`
/// (whether a hook cancelled character `c`): the cancelled printable
/// characters are gone and every other character stays, control characters
/// included — a text box reads Backspace, Delete or Escape as keys and vanilla
/// fires no `charTyped` for them, so they are never offered to the hook.
/// `None` when nothing is left to type.
fn screen_text_hooked(text: &str, mut canceled: impl FnMut(char) -> bool) -> Option<String> {
    let mut left = String::with_capacity(text.len());
    for c in text.chars() {
        if !c.is_control() && canceled(c) {
            continue;
        }
        left.push(c);
    }
    if left.is_empty() && !text.is_empty() {
        return None;
    }
    Some(left)
}

/// What a key press's characters leave for a text box after the
/// `ScreenEvent.CharacterTyped.Pre` hook: the characters no mod cancelled,
/// control ones gone (the key's own handling takes those, and vanilla fires
/// no `charTyped` for them). `None` when nothing is left to type.
fn chars_to_type(text: &str, canceled: impl FnMut(char) -> bool) -> Option<String> {
    let left = screen_text_hooked(text, canceled)?;
    let text: String = left.chars().filter(|c| !c.is_control()).collect();
    (!text.is_empty()).then_some(text)
}

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Declare categories for custom recipe types, with registered item icons.
    /// Asked lazily once per recipe table by `native_recipe_book_category`.
    /// Invalid declarations are logged and skipped; the first valid category
    /// owns its id and type. This does not yet add tabs or book displays.
    pub(super) fn native_recipe_book_categories(&self, categories: &mut Vec<RecipeBookCategory>) {
        mod_api::dispatch_recipe_book_categories(self, mod_api::mods(), categories);
    }
    /// Once a frame, in a world or not, after Horizon's extras.
    pub(super) fn native_frame(&mut self, elapsed: Duration) {
        mod_api::dispatch_frame(self, mod_api::mods(), elapsed);
    }
    /// The frame rate this window is about to be held to: Max Framerate after the
    /// inactivity throttle and Battery Mode (260 is unlimited). `focused` is whether the
    /// window has focus, `minimized` whether it is minimised and `in_world` whether a world
    /// is open. A mod may lower `limit`, as Dynamic FPS does for a window in the background:
    /// Horizon keeps the lower of its own limit and the mod's, and ignores values below 1.
    /// API 167.
    pub(super) fn native_framerate_limit(
        &self,
        focused: bool,
        minimized: bool,
        in_world: bool,
        limit: &mut i32,
    ) {
        mod_api::dispatch_framerate_limit(
            self,
            mod_api::mods(),
            focused,
            minimized,
            in_world,
            limit,
        );
    }
    /// Render-thread notification after a resource-pack reload has applied successfully.
    /// Atlas, models, renderer and client caches already refer to the new resources.
    /// Selection/F3+T and server-pack reloads share this point; initial startup does not.
    pub(super) fn native_client_resources_reloaded(&mut self) {
        mod_api::dispatch_client_resources_reloaded(self, mod_api::mods());
    }
    /// The client made a new level for `dimension` (NeoForge's
    /// `LevelEvent.Load` from the `ClientLevel` constructor): on joining a
    /// server, single player's own included, and on changing dimension.
    /// Its chunks come after. When the dimension changes, the old level's
    /// `native_client_level_unload` follows this, as `Minecraft.setLevel`
    /// tells it after the new level is made. Client-side, on the window; the
    /// server's levels have `native_level_load`. API 135.
    pub(super) fn native_client_level_load(&mut self, dimension: &str) {
        mod_api::dispatch_client_level_load(self, mod_api::mods(), dimension);
    }
    /// The client dropped its level for `dimension` (`LevelEvent.Unload`
    /// from `Minecraft.setLevel` and `disconnect`): replaced by another
    /// dimension's, or on leaving the server. Once for each
    /// `native_client_level_load`. Not told when the game itself quits.
    /// Client-side. API 135.
    pub(super) fn native_client_level_unload(&mut self, dimension: &str) {
        mod_api::dispatch_client_level_unload(self, mod_api::mods(), dimension);
    }
    /// Once a tick, in every `Game`: the window's (in single player, on LAN
    /// and on servers), and the server's own (`native_window` is false
    /// there: the single player server thread, a dedicated server).
    pub(super) fn native_tick(&mut self) {
        mod_api::dispatch_tick(self, mod_api::mods());
    }
    /// Before an eligible targeted block's selection outline is built.
    /// Cancel skips only the default block highlight, after normal HUD and game-mode gates.
    pub(super) fn native_block_highlight(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        partial: f32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_block_highlight(self, mod_api::mods(), pos, state, partial, canceled);
    }
    /// The world frame's local visual sources. Push requests; Horizon validates
    /// and limits them before rendering. Called only while a world is visible.
    pub(super) fn native_visuals(
        &self,
        rims: &mut Vec<NativeBlockRim>,
        lights: &mut Vec<NativeColoredLight>,
    ) {
        mod_api::dispatch_visuals(self, mod_api::mods(), rims, lights);
    }
    /// At most 32 pairs of already-loaded base and transparent overlay sprites.
    /// Per-pixel alpha in the overlay marks full-bright material, without
    /// illuminating the surrounding block or changing gameplay light.
    pub(super) fn native_emissive(&self, layers: &mut Vec<(&'static str, &'static str)>) {
        mod_api::dispatch_emissive(self, mod_api::mods(), layers);
    }
    /// The world's phases, read-only; `error_text` carries the engine's
    /// own open-failure text, empty otherwise. Server-side. A server loop or final
    /// save failure fires `WorldCloseFailed` with the error text.
    pub(super) fn native_world_lifecycle(
        &mut self,
        context: &NativeLifecycleContext,
        phase: WorldPhase,
        error_text: &str,
    ) {
        mod_api::dispatch_world_lifecycle(self, mod_api::mods(), context, phase, error_text);
    }
    /// The server is about to send its data to the players (NeoForge's
    /// `OnDatapackSyncEvent`, its `sendRecipes`): push the recipe types
    /// (`minecraft:smelting`, `mymod:crusher`) whose recipes your client half
    /// needs. Horizon sends their final recipes — the world's data packs
    /// merged, load conditions held — to every Horizon client when it joins
    /// and again after each `/reload`; a vanilla client is sent nothing new.
    /// Asked once per load, when the server's data tables are built, and the
    /// same types go to every player: unlike NeoForge, which asks again for
    /// each joining player, it can't differ between players. Push nothing and
    /// nothing is sent; push nothing after a load that pushed some and the
    /// clients go back to their own recipes. Server-side (the single
    /// player's integrated server included). API 120.
    pub(super) fn native_datapack_sync(&mut self, recipe_types: &mut Vec<String>) {
        mod_api::dispatch_datapack_sync(self, mod_api::mods(), recipe_types);
    }
    /// The server's recipes of `recipe_types` arrived (NeoForge's
    /// `RecipesReceivedEvent`): from now on the recipe helpers
    /// (`native_recipes`, `native_recipes_of_type`) give the server's recipes
    /// of those types in place of the window's own, so rebuild what you made
    /// of them. It comes after a join, once the window has its player, and
    /// after every `/reload`, on a server whose mods asked for them
    /// (`native_datapack_sync`), single player included; with no types when
    /// the server stopped sending them and the window is back on its own
    /// recipes. A recipe the
    /// window's own recipe book doesn't know has `book_id` -1. Client-side.
    /// API 120.
    pub(super) fn native_recipes_received(&mut self, recipe_types: &[String]) {
        mod_api::dispatch_recipes_received(self, mod_api::mods(), recipe_types);
    }
    /// The server's data tables are built (NeoForge's `TagsUpdatedEvent`
    /// with `SERVER_DATA_LOAD`): the recipes, tags, loot and advancements of
    /// the world's data packs, and the files mods added through
    /// `native_data_pack_files`, are what the helpers read from now on.
    /// `reload` is false on a world's first open, before its chunks and
    /// players are loaded, and true after `/reload` rebuilt them, once the
    /// players were sent the new data. The place to
    /// drop caches derived from the data and to rebind what depends on it.
    /// Server-side (the single player's integrated server included); a client
    /// connected to a server never fires it. API 108.
    pub(super) fn native_server_data_loaded(&mut self, reload: bool) {
        mod_api::dispatch_server_data_loaded(self, mod_api::mods(), reload);
    }
    /// The server is assembling its data tables from the world's data packs
    /// (`ReloadableServerResources`): a world is opening, or `/reload` was
    /// run, and the recipes, tags, loot and advancements are built after
    /// this. Push `(path, bytes)` pack files and they join this load. `path`
    /// is the overlay's key, `<kind>/<namespace>:<path>` without `.json`, over
    /// the kinds the tables read: `recipe/mymod:thing`,
    /// `tags/mymod:block/thing`, `loot_table/…`, `advancement/…`,
    /// `trade_set/…`, `villager_trade/…`, `dialog/…`. A file that isn't JSON
    /// is skipped with a warning. Asked once per load on both paths, so this
    /// is where a mod recomputes them.
    /// Contributed files sit below the world's own data packs: a pack's file
    /// for the same path still overrides a mod's. Server-side (the single
    /// player's integrated server included). API 106.
    pub(super) fn native_data_pack_files(&self, files: &mut Vec<(String, Vec<u8>)>) {
        mod_api::dispatch_data_pack_files(self, mod_api::mods(), files);
    }
    /// The server is assembling its data tables, as for
    /// `native_data_pack_files` (a world's first open and every `/reload`):
    /// push the data map types your mod registers (NeoForge's
    /// `RegisterDataMapTypesEvent`). A type is an `id` (`mod_id:name`, not in
    /// `minecraft`) for a `registry`, `minecraft:item` or `minecraft:block`
    /// so far. Its values come from the world's data packs'
    /// `data/<mod_id>/data_maps/<registry path>/<name>.json`, and from files
    /// you push through `native_data_pack_files` under the key
    /// `data_maps/<mod_id>:<registry path>/<name>`: `replace`, `values`
    /// (entry ids and `#tags` to JSON values) and `remove`, a newer value
    /// overriding an older. Read them with `native_data_map` and
    /// `native_data_map_entries`, in `native_data_maps.rs`. Server-side, not
    /// synced to clients. API 143.
    pub(super) fn native_data_map_types(&self, types: &mut Vec<NativeDataMapType>) {
        mod_api::dispatch_data_map_types(self, mod_api::mods(), types);
    }
    /// The server is about to open its worlds: nothing of the world is
    /// loaded yet (NeoForge's `ServerAboutToStartEvent`, "called before the
    /// server begins loading anything"). Vanilla's `MinecraftServer.loadLevel`
    /// start. Server-side: the single player server thread once Play is
    /// pressed, a dedicated server once its settings are read. Each is
    /// followed by `native_world_lifecycle`'s `WorldOpenReady` or
    /// `WorldOpenFailed`. `dedicated` tells a dedicated server from the single
    /// player one. API 44.
    pub(super) fn native_server_about_to_start(&mut self, dedicated: bool) {
        mod_api::dispatch_server_about_to_start(self, mod_api::mods(), dedicated);
    }
    /// The server begins an orderly stop (NeoForge's `ServerStoppingEvent`):
    /// the start of vanilla's `MinecraftServer.stopServer`, with the worlds
    /// still loaded and the players still connected and not yet saved, so a
    /// mod can save its own data or tell them. `native_world_lifecycle`'s
    /// `WorldCloseFinal` follows once the worlds are saved. A server whose loop
    /// fails stops without this (`WorldCloseFailed` reports it). API 44.
    pub(super) fn native_server_stopping(&mut self, dedicated: bool) {
        mod_api::dispatch_server_stopping(self, mod_api::mods(), dedicated);
    }
    /// The authoritative server tick's phases: `ServerTickStart` before any
    /// level is ticked (NeoForge's `ServerTickEvent.Pre`), `ServerTickEnd`
    /// after the last (`.Post`). Read blocks with `native_server_block`, by
    /// dimension: which level `self.world` holds here is not defined. A paused
    /// pass fires nothing; a frozen world carries `world_running` false except
    /// during a requested step.
    pub(super) fn native_server_tick(
        &mut self,
        context: &NativeLifecycleContext,
        phase: ServerTickPhase,
    ) {
        mod_api::dispatch_server_tick(self, mod_api::mods(), context, phase);
    }
    /// Before an authoritative server level's tick pass. `dimension` is its ID.
    pub(super) fn native_level_tick_pre(&mut self, dimension: &str) {
        mod_api::dispatch_level_tick_pre(self, mod_api::mods(), dimension);
    }
    /// After a successful authoritative server level tick pass.
    pub(super) fn native_level_tick_post(&mut self, dimension: &str) {
        mod_api::dispatch_level_tick_post(self, mod_api::mods(), dimension);
    }
    /// The HUD (not drawn under F1), after the minimap: place a box with
    /// `corners.place` (boxes stack away from their edge) and draw it with
    /// `native_fill` and `native_text`. Skip a corner `native_corner_shown`
    /// says is taken.
    pub(super) fn native_hud(&mut self, batch: &mut HudBatch, corners: &mut Corners) {
        mod_api::dispatch_hud(self, mod_api::mods(), batch, corners);
    }
    /// Before one of vanilla's HUD layers is drawn (`RenderGuiLayerEvent.Pre`):
    /// draw here to put something below the layer, or set `canceled` to hide
    /// it. `layer` is the layer's id, in the order Horizon draws them:
    /// `minecraft:crosshair`, `hotbar`, `effects`, `boss_overlay`,
    /// `vehicle_health` (only riding a mount), `contextual_info_bar` (the
    /// mount's jump bar, the locator bar or the experience bar, with the level
    /// number), `player_health` (the health, armor, food and air rows, drawn in
    /// one pass), `selected_item_name`, `sleep_overlay`, `overlay_message`
    /// (the action bar), `title`, `scoreboard_sidebar`, `subtitle_overlay`,
    /// `chat` and `tab_list`. A layer F1 hides fires nothing, and the chat
    /// screen draws the focused chat itself, so `chat` is only the unfocused
    /// lines. The camera overlays and the debug screen are not layers here:
    /// they are not drawn in this pass. Draw with `native_fill`, `native_text`
    /// and the sprite helpers; `batch.width` and `batch.height` are the GUI
    /// size. Together with `native_gui_layer_post` this is NeoForge's
    /// `RegisterGuiLayersEvent`: a mod keeps its own list of what it draws
    /// above or below which layer. API 139.
    pub(super) fn native_gui_layer_pre(
        &mut self,
        layer: &str,
        batch: &mut HudBatch,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_gui_layer_pre(self, mod_api::mods(), layer, batch, canceled);
    }
    /// After one of vanilla's HUD layers (or after the layer a mod hid), with
    /// the ids of `native_gui_layer_pre`: draw here to put something above
    /// the layer (`RenderGuiLayerEvent.Post`). API 139.
    pub(super) fn native_gui_layer_post(&mut self, layer: &str, batch: &mut HudBatch) {
        mod_api::dispatch_gui_layer_post(self, mod_api::mods(), layer, batch);
    }
    /// A player right-clicks the entity `entity` (`Entity.getId`) of type
    /// `kind` (`minecraft:cow`) with `self.hand`, holding `stack` (none in an
    /// empty hand): where `Player.interactOn` starts, before the entity's own
    /// `interact` and the item's `interactLivingEntity`. Set `claimed` to take
    /// the click: the hand swings and neither runs. Asked once per hand, as
    /// vanilla tries the main hand then the off hand. Server-side, for every
    /// player of the world (the helpers read the acting one). API 29.
    pub(super) fn native_interact_entity(
        &mut self,
        entity: i32,
        kind: &str,
        stack: Option<&ItemStack>,
        claimed: &mut bool,
    ) {
        mod_api::dispatch_interact_entity(self, mod_api::mods(), entity, kind, stack, claimed);
    }
    /// A player left-clicks the block `state` at `pos`, `face` the face hit
    /// (`[0, 1, 0]` is the top): where `ServerPlayerGameMode.handleBlockBreakAction`
    /// takes a dig's start, after its reach, spawn protection and world border
    /// checks, before `Block.attack` (a note block plays) or a creative break.
    /// Set `canceled` to ignore the click: the block isn't attacked, dug or
    /// broken on the server, and the player's client gets it back (its own
    /// predicted crack still shows to the clicking player). Server-side, for
    /// every player of the world (the helpers read the acting one). API 30.
    pub(super) fn native_left_click_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_left_click_block(self, mod_api::mods(), pos, face, state, canceled);
    }
    /// A living player's server tick ends (`ServerPlayer.tick`): once a tick
    /// for every player in the world, after their digging, item use,
    /// cooldowns, statistics and advancements. The helpers read that player.
    /// Players still tick under `/tick freeze`; a dead player doesn't, nor
    /// does one killed earlier in the same tick. API 31.
    pub(super) fn native_player_tick(&mut self) {
        mod_api::dispatch_player_tick(self, mod_api::mods());
    }
    /// The player-health HUD rows, once a frame the HUD draws them
    /// (`HealthHud::layout`, not under F1): `inputs` carries the health,
    /// food, air and vehicle state the layout reads, `wi`/`hi` the screen's
    /// GUI size. A mod that sets `draws` replaces every row — armour,
    /// hearts, food, air and the heights (`left_height`/`right_height`)
    /// above the hotbar are the answer's to give; `bubble_pop_sound` too.
    /// Nothing set draws Horizon's own.
    pub(super) fn native_health_hud(
        &mut self,
        inputs: &horizon_client::hud_layout::HealthInputs,
        wi: i32,
        hi: i32,
        draws: &mut Option<Vec<horizon_client::hud_layout::GuiDraw>>,
    ) {
        mod_api::dispatch_health_hud(self, mod_api::mods(), inputs, wi, hi, draws);
    }
    /// A key or mouse button went down or up, by its vanilla name
    /// (`key.keyboard.z`, `key.mouse.left`): keys on any screen (check
    /// `native_playing`), mouse buttons while playing. Repeats are left out.
    pub(super) fn native_key(&mut self, key: &str, pressed: bool) {
        mod_api::dispatch_key(self, mod_api::mods(), key, pressed);
    }
    /// A real window key event after handling, including repeats and consumed shortcuts.
    /// Receives the original physical/logical key, text, location, state and repeat flag.
    /// Modifier bits use the screen hooks' GLFW encoding. Not cancellable.
    pub(super) fn native_input_key(&mut self, event: &winit::event::KeyEvent, modifiers: u32) {
        mod_api::dispatch_input_key(self, mod_api::mods(), event, modifiers);
    }
    /// Raw mouse button input on any screen, before dispatch; cancel skips dispatch and Post.
    /// Button/modifiers use the same GLFW encoding as screen hooks. Synthetic clicks skip it.
    pub(super) fn native_mouse_button_pre(
        &mut self,
        button: i32,
        pressed: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_mouse_button_pre(
            self,
            mod_api::mods(),
            button,
            pressed,
            modifiers,
            canceled,
        );
    }
    /// An accepted raw button event finished dispatch, including a screen handler's early return.
    pub(super) fn native_mouse_button_post(&mut self, button: i32, pressed: bool, modifiers: u32) {
        mod_api::dispatch_mouse_button_post(self, mod_api::mods(), button, pressed, modifiers);
    }
    /// Wheel steps while playing: set `claimed` to keep them from the hotbar.
    pub(super) fn native_wheel(&mut self, steps: f32, claimed: &mut bool) {
        mod_api::dispatch_wheel(self, mod_api::mods(), steps, claimed);
    }
    /// Gameplay scrolling after whole steps accumulate, before hotbar/spectator handling.
    /// `scroll` is scaled x/y, `accumulated` whole x/y steps, `buttons` left/middle/right,
    /// and `cursor` the GUI position. Cancel skips gameplay scrolling; synthetic input skips it.
    pub(super) fn native_mouse_scrolling(
        &mut self,
        scroll: [f64; 2],
        accumulated: [i32; 2],
        buttons: [bool; 3],
        cursor: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_mouse_scrolling(
            self,
            mod_api::mods(),
            scroll,
            accumulated,
            buttons,
            cursor,
            modifiers,
            canceled,
        );
    }
    /// An interaction key acts on what the crosshair points at
    /// (`InputEvent.InteractionKeyMappingTriggered`), while playing: `button` 0 is
    /// the attack key (`key_mapping` `key.attack`), 1 the use key (`key.use`, once
    /// for each hand it tries, the main hand first), 2 pick block (`key.pickItem`,
    /// only with something aimed at). The attack fires when an attack is allowed
    /// and again each tick a held attack key keeps digging a block. Set `canceled`
    /// to skip the action: a cancelled use also skips the other hand. A cancelled
    /// attack or use still swings `hand` unless the mod clears `swing_hand` (it
    /// starts true). A server's player and a single player's window only; a mod
    /// that claims an item's use (`mod_use_start`) sees the event first. API
    /// 138.
    pub(super) fn native_interaction_key(
        &mut self,
        button: i32,
        key_mapping: &str,
        hand: horizon_client::first_person::Hand,
        swing_hand: &mut bool,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_interaction_key(
            self,
            mod_api::mods(),
            button,
            key_mapping,
            hand,
            swing_hand,
            canceled,
        );
    }
    /// A command typed or clicked in chat, without its slash, before the
    /// server and Horizon's commands see it: set `ran` when the mod ran it.
    pub(super) fn native_command(&mut self, command: &str, ran: &mut bool) {
        mod_api::dispatch_command(self, mod_api::mods(), command, ran);
    }
    /// The commands `native_command` runs, by name and without their slash:
    /// push the ones that are yours. Asked when a connection starts and
    /// again when the server sends its own tree, so the chat box colours,
    /// completes and lists them as the game's own; a name the game already
    /// has keeps that command's behaviour. API 35.
    pub(super) fn native_commands(&mut self, commands: &mut Vec<&'static str>) {
        mod_api::dispatch_commands(self, mod_api::mods(), commands);
    }
    /// Player FOV scale, after vanilla effects and before zoom and easing.
    /// Invalid values or values outside 0.01..=10 revert to base. API @API:compute_fov_modifier@.
    pub(super) fn native_compute_fov_modifier(
        &self,
        player: &horizon_client::camera::FovPlayer,
        base: f32,
        modifier: &mut f32,
    ) {
        mod_api::dispatch_compute_fov_modifier(self, mod_api::mods(), player, base, modifier);
    }
    /// Local player movement input, after reading it and before applying it.
    pub(super) fn native_movement_input(
        &self,
        forward: &mut f32,
        strafe: &mut f32,
        jump: &mut bool,
        sneak: &mut bool,
    ) {
        mod_api::dispatch_movement_input(self, mod_api::mods(), forward, strafe, jump, sneak);
    }
    /// World field of view, after normal calculation and zoom and before projection/Nausea.
    /// Called for world rendering and near-plane overlays; hand/HUD FOV uses its own path.
    /// Only finite values in 1..=179 degrees replace the computed FOV.
    pub(super) fn native_compute_fov(&self, partial: f32, camera: [f64; 3], degrees: &mut f32) {
        mod_api::dispatch_compute_fov(self, mod_api::mods(), partial, camera, degrees);
    }
    /// The zoom a native mod asks for, applied while playing (the spyglass's
    /// own view wins): set `zoom` when it is none yet.
    pub(super) fn native_zoom(&self, zoom: &mut Option<NativeZoom>) {
        mod_api::dispatch_zoom(self, mod_api::mods(), zoom);
    }
    /// Computed world fog RGB, after ordinary effects and before upload. Title panoramas skip it.
    /// Only finite RGB components in 0..=1 are accepted; alpha and distances stay unchanged.
    pub(super) fn native_compute_fog_color(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: horizon_client::fog::FogType,
        rgb: &mut [f32; 3],
    ) {
        mod_api::dispatch_compute_fog_color(self, mod_api::mods(), partial, camera, fog_type, rgb);
    }
    /// Final world fog, after the computed color hook, ordinary calculations and Horizon's fog
    /// option, before upload. Title panoramas skip it. Invalid color or distance values retain the
    /// original whole fog.
    pub(super) fn native_render_fog(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: horizon_client::fog::FogType,
        fog: &mut horizon_client::fog::FogData,
    ) {
        mod_api::dispatch_render_fog(self, mod_api::mods(), partial, camera, fog_type, fog);
    }
    /// The camera shake native mods ask for in this frame, `partial` ticks
    /// past the last tick (0 to 1): add your pitch, yaw and roll in degrees
    /// and your offset in blocks, along the camera's right, up and back, to
    /// `shake`, so several mods' shakes add up. Read while playing only;
    /// Horizon scales the sum by Distortion Effects and bounds it
    /// (`native_shake_matrix`). API 82.
    pub(super) fn native_camera_shake(&self, partial: f32, shake: &mut NativeShake) {
        mod_api::dispatch_camera_shake(self, mod_api::mods(), partial, shake);
    }
    /// Name tag visibility and optional replacement text. API @API:render_name_tag@.
    pub(super) fn native_render_name_tag(
        &self,
        entity_id: u32,
        kind: &str,
        vanilla_visible: bool,
        visible: &mut bool,
        text: &mut Option<String>,
    ) {
        mod_api::dispatch_render_name_tag(
            self,
            mod_api::mods(),
            entity_id,
            kind,
            vanilla_visible,
            visible,
            text,
        );
    }
    /// Before one first-person hand and its displayed stack are drawn (RenderHandEvent).
    /// Equip progress is the inverse arm height, including the item's swap animation scale.
    /// Cancel skips that hand's default geometry, including its map and arm geometry.
    /// It only hides: the hook has no way to draw a replacement.
    pub(super) fn native_render_hand(
        &self,
        hand: horizon_client::first_person::Hand,
        stack: Option<&ItemStack>,
        partial: f32,
        swing: f32,
        equip: f32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_render_hand(
            self,
            mod_api::mods(),
            hand,
            stack,
            partial,
            swing,
            equip,
            canceled,
        );
    }

    /// Additional first-person hand and item pose, including empty arms and maps.
    /// Add offsets in blocks and pitch, yaw and roll in degrees; multiply scale.
    /// API @API:render_hand_pose@.
    pub(super) fn native_render_hand_pose(
        &self,
        hand: horizon_client::first_person::Hand,
        stack: Option<&ItemStack>,
        partial: f32,
        swing: f32,
        equip: f32,
        x: &mut f32,
        y: &mut f32,
        z: &mut f32,
        pitch: &mut f32,
        yaw: &mut f32,
        roll: &mut f32,
        scale: &mut f32,
    ) {
        mod_api::dispatch_render_hand_pose(
            self,
            mod_api::mods(),
            hand,
            stack,
            partial,
            swing,
            equip,
            x,
            y,
            z,
            pitch,
            yaw,
            roll,
            scale,
        );
    }

    /// Before a visible item frame's item or map is drawn. Cancel hides the item, not the frame.
    pub(super) fn native_render_item_in_frame(
        &self,
        entity_id: u32,
        pos: [i32; 3],
        stack: &ItemStack,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_render_item_in_frame(
            self,
            mod_api::mods(),
            entity_id,
            pos,
            stack,
            canceled,
        );
    }

    /// Final scaled third-person camera distance, before visual-shape collision clipping.
    /// `entity` includes its living scale/distance and optional mount scale/distance.
    /// Only finite requests in 0..=1024 blocks replace the vanilla distance.
    pub(super) fn native_camera_distance(
        &self,
        partial: f32,
        mirrored: bool,
        entity: &horizon_client::camera::CameraEntity,
        distance: &mut f32,
    ) {
        mod_api::dispatch_camera_distance(
            self,
            mod_api::mods(),
            partial,
            mirrored,
            entity,
            distance,
        );
    }
    /// Change the colours of `biome` (`minecraft:plains`) on this client: the
    /// grass, foliage (leaves, vines), dry foliage (leaf litter) and water
    /// tints, 0xRRGGBB, as its biome file and the colormaps set them. Asked
    /// for every biome when a mod calls `native_refresh_biome_colors`, which
    /// rebuilds the terrain with them; a season mod answers from its season
    /// and calls that when the season turns. The dark forest's modifier
    /// still applies to the grass set here; the swamp's noise colours
    /// replace it. API 107.
    pub(super) fn native_biome_colors(&self, biome: &str, colors: &mut BiomeColors) {
        mod_api::dispatch_biome_colors(self, mod_api::mods(), biome, colors);
    }
    /// Choose this client's music, once a client tick: `music` and `volume`
    /// hold what vanilla chose (`Minecraft.getSituationalMusic` and
    /// `getMusicVolume`: the biome's track, the creative or underwater one, a
    /// boss's, the menu's; None for silence). Set `music` to a track of your
    /// own, a sound event of your content mod's `sounds.json` such as
    /// `Music::new("mymod:music.spring", 1200, 6000, false)` (the delays in
    /// ticks between songs, and whether it cuts off a different song playing),
    /// None to keep vanilla's quiet, or change `volume` (0 to 1) to fade it. API
    /// 119.
    pub(super) fn native_music(&self, music: &mut Option<Music>, volume: &mut f32) {
        mod_api::dispatch_music(self, mod_api::mods(), music, volume);
    }
    /// The tracks to play over (or instead of) the music on this client,
    /// asked once a client tick: push a `MusicLayer` (a sound event, looped,
    /// and its volume, 0 to 1) for each. A layer eases to a new volume over up
    /// to two seconds, and one left out fades out and stops; layers that
    /// first appear in the same tick start together, so list every stem of a
    /// piece from the start, the silent ones at volume 0, and raise them (a
    /// fuller arrangement as a village grows). Their sound events should be
    /// `stream: true` in `sounds.json`. API 119.
    pub(super) fn native_music_layers(&self, layers: &mut Vec<MusicLayer>) {
        mod_api::dispatch_music_layers(self, mod_api::mods(), layers);
    }
    /// The values native mods hand their shader packs in this frame,
    /// `partial` ticks past the last tick (0 to 1): a pack's WGSL reads
    /// them as `frame.mods` (eight `vec4<f32>`). Write the slots your own
    /// pack reads; a mod that bundles its pack as `native-<mod id>` is the
    /// one that pack is for. Asked once a frame while a pack is on; a value
    /// that isn't finite reads as 0 (`native_shader_params_now`). API
    /// 84.
    pub(super) fn native_shader_params(&self, partial: f32, params: &mut [[f32; 4]; 8]) {
        mod_api::dispatch_shader_params(self, mod_api::mods(), partial, params);
    }
    /// The minimap a native mod shows: set `minimap` when it is none yet.
    pub(super) fn native_minimap(&self, minimap: &mut Option<NativeMinimap>) {
        mod_api::dispatch_minimap(self, mod_api::mods(), minimap);
    }
    /// Register namespaced NBT component types before content loads.
    pub(super) fn native_component_types(types: &mut Vec<String>) {
        mod_api::dispatch_component_types(mod_api::mods(), types);
    }
    /// Blocks this mod adds, declared in code rather than one file each
    /// (NeoForge's `DeferredRegister.Blocks` drained by `RegisterEvent`):
    /// push a `horizon_mods::content::Block` per block, the same declaration
    /// a content mod's `data/<id>/horizon/block/<path>.json` holds (a
    /// vanilla `template`, `properties`, `state_properties`, `shapes`...),
    /// with its id in your mod's namespace. `item: true` gives it a block
    /// item, as a file does unless told not to (`..Default::default()`
    /// leaves it false). They join the content mods' blocks: the one whose
    /// id is that namespace when it is on (your models, textures, loot and
    /// recipes are its files), else one of their own, with the template's
    /// look. Ids, saves and the check that a joining client has the same
    /// blocks are the files' (docs/mods.md). Asked once at start, on every
    /// side, before any resource loads: push the same ones every time, from
    /// nothing but your code. Declarations that don't pass the files' checks
    /// are logged and that namespace's are left out. API
    /// 145.
    pub(super) fn native_blocks(blocks: &mut Vec<horizon_mods::content::Block>) {
        mod_api::dispatch_blocks(mod_api::mods(), blocks);
    }
    /// Items this mod adds, declared in code (NeoForge's
    /// `DeferredRegister.Items`), as `native_blocks` declares blocks: a
    /// `horizon_mods::content::Item` per item, a content mod's
    /// `data/<id>/horizon/item/<path>.json` (a vanilla `template`,
    /// `max_stack_size`, `rarity`, `components`, `crafting_remainder`). A
    /// block's item comes with its block (`item: true`), not from here.
    /// Asked once at start, on every side. API 145.
    pub(super) fn native_items(items: &mut Vec<horizon_mods::content::Item>) {
        mod_api::dispatch_items(mod_api::mods(), items);
    }
    /// Models no blockstate names that this client bakes beside the block
    /// models (NeoForge's `ModelEvent.RegisterStandalone`): push their ids
    /// (`mymod:block/shaft_half`, the model at
    /// `assets/mymod/models/block/shaft_half.json`). Parts a block entity or
    /// a mob draws, with `native_draw_model`, so that no block is registered
    /// only to bake its model. Asked once at start, before the resources
    /// load; every resource load, a pack reload included, bakes them again
    /// from that pack's files. A model that doesn't read is logged and left
    /// out; an id without a namespace is dropped. Every side asks it, but
    /// only a window bakes models. API 151.
    pub(super) fn native_standalone_models(models: &mut Vec<String>) {
        mod_api::dispatch_standalone_models(mod_api::mods(), models);
    }
    /// Options > Mods lists the native mods: push yours.
    pub(super) fn native_mods(&mut self, mods: &mut Vec<NativeMod>) {
        mod_api::dispatch_mods(self, mod_api::mods(), mods);
    }
    /// The buttons of mod `mod_id`'s settings page, under its keys: push
    /// their labels when it is yours. Asked again each time the page opens
    /// and after each click.
    pub(super) fn native_settings(&mut self, mod_id: &str, buttons: &mut Vec<String>) {
        mod_api::dispatch_settings(self, mod_api::mods(), mod_id, buttons);
    }
    /// Button `button` of mod `mod_id`'s settings page was clicked.
    pub(super) fn native_settings_click(&mut self, mod_id: &str, button: usize) {
        mod_api::dispatch_settings_click(self, mod_api::mods(), mod_id, button);
    }
    /// The player did `event` on mod `mod_id`'s panel (`native_panel_items`):
    /// act on it when it is yours. API 2.
    pub(super) fn native_panel_event(&mut self, mod_id: &str, event: &NativePanelEvent) {
        mod_api::dispatch_panel_event(self, mod_api::mods(), mod_id, event);
    }
    /// Block `block` (`mymod:machine`) gets a block entity
    /// (`EntityBlock.newBlockEntity`): set `entity` to its id and saved
    /// compound when the block is yours and `entity` is none yet. It works on
    /// any template: the engine removes the block entity when its block is
    /// broken or replaced, no piston moves it, and a mod that draws it
    /// (`native_block_entity_submit`) is asked for it. API 15; before
    /// 129 the block's template had to have a
    /// block entity of its own (a chest, a furnace...), or the block got none.
    pub(super) fn native_block_entity(
        &mut self,
        block: &str,
        entity: &mut Option<(String, Compound)>,
    ) {
        mod_api::dispatch_block_entity(self, mod_api::mods(), block, entity);
    }
    /// Whether block `block`'s block entity ticks (`EntityBlock.getTicker`):
    /// set `ticks` when it is yours. Asked for each of the block's states
    /// when a world opens. API 15.
    /// Set `ticks` to true to make it tick, and never clear a value an earlier mod or mixin set.
    pub(super) fn native_block_entity_ticks(&self, block: &str, ticks: &mut bool) {
        mod_api::dispatch_block_entity_ticks(self, mod_api::mods(), block, ticks);
    }
    /// A `minecraft:post_attack` effect of a mod's own kind, on a mob a
    /// player's weapon hit: the entry says `enchanted` attacker and `affected`
    /// victim, its `requirements` passed, and its `effect` names a `type` in a
    /// namespace outside `minecraft:`. Told the enchantment id, the kind, the
    /// `effect` JSON, the level and the mob's id (it may have just died). The
    /// acting player is the one whose weapon carries the enchantment. Each
    /// such entry is told once a hit, and once for each mob a sweep lands on.
    /// A hit on a player, and an entry inside `minecraft:all_of`, aren't told
    /// yet. An unclaimed kind does nothing. API 77.
    pub(super) fn native_enchant_effect(
        &mut self,
        enchantment: &str,
        kind: &str,
        data: &serde_json::Value,
        level: u32,
        victim: u32,
    ) {
        mod_api::dispatch_enchant_effect(
            self,
            mod_api::mods(),
            enchantment,
            kind,
            data,
            level,
            victim,
        );
    }
    /// Block `block`'s block entity has item slots (a `WorldlyContainer`):
    /// set `container` when the block is yours. Asked once for each mod
    /// block when the ticking table is built; at most 256 slots, and slot
    /// numbers past the count are left out. Its `menu`, one of
    /// `containers::MOD_MENUS` or a content mod's menu type
    /// (`data/<id>/horizon/menu/`, API 85) with as many
    /// slots, opens when the block is used (its template must open a menu, as
    /// a chest or a furnace does); another is left out. A mod menu type shows
    /// on Horizon clients through the mod's `native_screen` only. API 17.
    pub(super) fn native_block_entity_container(
        &self,
        block: &str,
        container: &mut Option<ModContainer>,
    ) {
        mod_api::dispatch_block_entity_container(self, mod_api::mods(), block, container);
    }
    /// Block `block`'s block entity has fluid tanks: set `fluids` when the
    /// block is yours, in the container's shape — each tank's capacity in
    /// millibuckets, the tanks each face reaches, and the tanks a fill may
    /// reach and a drain may take. Asked once for each mod block when the
    /// ticking table is built; at most 16 tanks, capacities clamped to
    /// 1..=1 000 000, and tank numbers past the count are left out. The tanks'
    /// contents are kept in the entity's saved compound, under `fluids`, and
    /// the fluid helpers below read and write them. API 54.
    pub(super) fn native_block_entity_fluids(
        &self,
        block: &str,
        fluids: &mut Option<ModFluidTanks>,
    ) {
        mod_api::dispatch_block_entity_fluids(self, mod_api::mods(), block, fluids);
    }
    /// Block `block`'s block entity has an energy buffer
    /// (`Capabilities.Energy.BLOCK`): set `energy` when the block is yours,
    /// as `SimpleEnergyHandler` is built: its capacity, the most one insert
    /// and one extract moves, and the faces a fill may come through and a
    /// drain may leave by. Asked once for each mod block when the ticking
    /// table is built; a capacity under 1 leaves the buffer out, and the
    /// limits are clamped to 0..=capacity. The amount is kept in the entity's
    /// saved compound, as the int `energy`, and the energy helpers below read
    /// and write it. API 142.
    pub(super) fn native_block_entity_energy(
        &self,
        block: &str,
        energy: &mut Option<ModEnergyBuffer>,
    ) {
        mod_api::dispatch_block_entity_energy(self, mod_api::mods(), block, energy);
    }
    /// Item `item` (`mod_id:name`) is a fluid container
    /// (`Capabilities.FluidHandler.ITEM`): set `capacity`, in millibuckets,
    /// when the item is yours (clamped to 1..=1 000 000; vanilla's buckets
    /// are built in and cannot be changed). Asked at each call of the item
    /// fluid helpers (`native_item_fluid`, `native_item_fluid_fill`,
    /// `native_item_fluid_drain`, in `native_item_fluids.rs`). What the
    /// container holds is kept in the stack's `minecraft:custom_data`, under
    /// `horizon_fluid`. API 147.
    pub(super) fn native_item_fluid_tank(&self, item: &str, capacity: &mut Option<i32>) {
        mod_api::dispatch_item_fluid_tank(self, mod_api::mods(), item, capacity);
    }
    /// Item `item` (`mod_id:name`) holds energy (`Capabilities.Energy.ITEM`):
    /// set `buffer` when the item is yours, as `SimpleEnergyHandler` is built:
    /// its capacity and the most one insert and one extract move. A capacity
    /// under 1 leaves the buffer out, and the limits are clamped to
    /// 0..=capacity; vanilla's items have none. Asked at each call of the item
    /// energy helpers (`native_item_energy`, `native_item_energy_insert`,
    /// `native_item_energy_extract`, in `native_item_energy.rs`). The amount
    /// is kept in the stack's `minecraft:custom_data`, as the int
    /// `horizon_energy`. API 307.
    pub(super) fn native_item_energy_buffer(
        &self,
        item: &str,
        buffer: &mut Option<ItemEnergyBuffer>,
    ) {
        mod_api::dispatch_item_energy_buffer(self, mod_api::mods(), item, buffer);
    }
    /// Item `item` (`mod_id:name`) holds items (`Capabilities.Item.ITEM`): set
    /// `slots` when the item is yours, for a backpack, a storage cell or a
    /// toolbox. Clamped to 1..=256, `ItemContainerContents`' limit; 0 leaves
    /// the item without slots, and vanilla's items have none. Asked at each
    /// call of the item container helpers (`native_item_container_slots`,
    /// `native_item_container_insert`, `native_item_container_extract`, in
    /// `native_item_container.rs`). The contents are kept in the stack's
    /// `minecraft:container`, as a shulker box item keeps them.
    /// API @API:item_container@.
    pub(super) fn native_item_container(&self, item: &str, slots: &mut Option<u32>) {
        mod_api::dispatch_item_container(self, mod_api::mods(), item, slots);
    }
    /// A server tick of the block entity at `pos`, in vanilla's order among
    /// the others (`Level.tickBlockEntities`), when its `id` is yours: read
    /// and change its saved compound `nbt`, its item slots `items` (keep
    /// their count: as many as its container has, none without one) and the
    /// data slots its menu shows (`data`). It is out of the world during the
    /// call (the block entity at `pos` reads as a `Mod` with an empty id, and
    /// a save leaves it out); if the call removes or replaces it, `entity` is
    /// dropped. Clients, the window's own in single player included, get
    /// what `native_block_entity_update_tag` gives when its chunk is sent or
    /// its block changes. API 17.
    pub(super) fn native_block_entity_tick(&mut self, pos: [i32; 3], entity: &mut ModBlockEntity) {
        mod_api::dispatch_block_entity_tick(self, mod_api::mods(), pos, entity);
    }
    /// A consumable's effect of a mod's own kind: the item's `on_consume_effects`
    /// entry is of a type in a namespace of its own, and the loader kept it
    /// for this call. The kind and the JSON the file wrote are here; the acting
    /// player is the one consuming, and the game's own effects,
    /// sounds, teleports and drops are open. An unclaimed kind does
    /// nothing. API 74.
    pub(super) fn native_consume_effect(&mut self, kind: &str, data: &serde_json::Value) {
        mod_api::dispatch_consume_effect(self, mod_api::mods(), kind, data);
    }
    /// What clients get of the block entity at `pos` when its `id` is
    /// yours (`BlockEntity.getUpdateTag`): set `tag` to it, read from
    /// `entity` alone. Asked on the server whenever a chunk holding it is
    /// sent and whenever its block changes, on the server thread or, for a
    /// saved chunk sent past the view distance (`lan::far`), another one; a client's copy of the block
    /// entity (made by `native_block_entity` there) merges it into its `nbt`.
    /// Leave it unset to send nothing. API 45.
    pub(super) fn native_block_entity_update_tag(
        pos: [i32; 3],
        entity: &ModBlockEntity,
        tag: &mut Option<Compound>,
    ) {
        mod_api::dispatch_block_entity_update_tag(mod_api::mods(), pos, entity, tag);
    }
    /// Draw the mod block entity at `pos`, of block `block`
    /// (`mymod:machine`), on this client: once a frame while it is in view
    /// (vanilla's `BlockEntityRenderer.submit`: within 64 blocks, its chunk
    /// faded in, and its bounds, `native_block_entity_render_bounding_box`,
    /// in the frustum), when its `id` is yours. `state` is the block's state
    /// (`blockEntity.getBlockState()`: read its `facing` with
    /// `native_block_state_property`), `entity` holds what the server sent
    /// (`native_block_entity_update_tag`). Add geometry with
    /// `native_draw_quad`, `native_draw_cube`, `native_draw_block` and
    /// `native_draw_item` under `draw`'s pose stack, which starts at the
    /// block's lower north-west corner, in blocks; `draw` also has the partial
    /// tick, the game time and the block's light. API 71.
    pub(super) fn native_block_entity_submit(
        &self,
        pos: [i32; 3],
        block: &str,
        state: BlockStateId,
        entity: &ModBlockEntity,
        draw: &mut BlockEntityDraw,
    ) {
        mod_api::dispatch_block_entity_submit(
            self,
            mod_api::mods(),
            pos,
            block,
            state,
            entity,
            draw,
        );
    }
    /// Where the mod block entity at `pos` draws, for culling
    /// (NeoForge's `getRenderBoundingBox`): `bounds` is min x, y, z then max
    /// x, y, z, in blocks from the block's lower north-west corner. It starts
    /// as the block with a block of margin around it; widen it when your
    /// `native_block_entity_submit` draws further (a tall shaft, a beam), or
    /// set an infinite value for one never culled. Asked once a frame for each
    /// within 64 blocks whose chunk is drawn. API 71.
    pub(super) fn native_block_entity_render_bounding_box(
        &self,
        pos: [i32; 3],
        block: &str,
        state: BlockStateId,
        entity: &ModBlockEntity,
        bounds: &mut [f32; 6],
    ) {
        mod_api::dispatch_block_entity_render_bounding_box(
            self,
            mod_api::mods(),
            pos,
            block,
            state,
            entity,
            bounds,
        );
    }
    /// Draw the mob `mob` (its id, kind, custom name and the frame's body
    /// and head turns and walk cycle, `LivingEntityRenderState`) on this
    /// client with a model of your own, once a frame while it is in view
    /// (vanilla's `EntityRenderer.submit`), and set `drawn` to leave out its
    /// vanilla model, armour and held items (NeoForge's `RenderLivingEvent.Pre`,
    /// cancelled). Add geometry with `native_draw_cube`, `native_draw_quad`,
    /// `native_draw_block` and `native_draw_item` under `draw`'s pose stack,
    /// which starts at the mob's feet turned with its body: +z is the way it
    /// faces, +x its left, +y up, in blocks. `draw` also has the partial tick,
    /// the game time and the mob's light. Its name tag, shadow, flames and
    /// glowing outline draw as for any mob. Asked for every mob in view but
    /// players; a mob your server half spawned (`native_spawn_mob`) is told
    /// apart by its id, sent with mod messages, or by its custom name.
    /// A mob is drawn afresh each frame: `draw.keep()` does nothing here.
    /// API 110.
    pub(super) fn native_mob_submit(
        &self,
        mob: &NativeMobRender<'_>,
        draw: &mut BlockEntityDraw,
        drawn: &mut bool,
    ) {
        mod_api::dispatch_mob_submit(self, mod_api::mods(), mob, draw, drawn);
    }
    /// Whether a mod listens to `native_player_submit` or `native_mob_layer`: a
    /// mod that hooks either hooks this one too and sets `listened` (HEAD,
    /// `*listened = true`). Without it the game poses no player or mob for
    /// them, each frame. Client-side. API 149.
    pub(super) fn native_player_render_listened(&self, listened: &mut bool) {
        mod_api::dispatch_player_render_listened(self, mod_api::mods(), listened);
    }
    /// Draw the player `player` (`who`: its profile name, and whether it is
    /// this window's own player) with a model of your own, once a frame while
    /// it is in view (`RenderPlayerEvent.Pre`), and set `drawn` to leave out its
    /// vanilla model, armour, held items, cape and the other layers. This
    /// window's player is drawn only in third person (and F5 front view); an
    /// invisible player is not asked. `draw` is the frame of `native_mob_submit`:
    /// at the player's feet, turned with the body. The name tag, shadow,
    /// flames and glowing outline draw as before. API 149.
    pub(super) fn native_player_submit(
        &self,
        player: &NativeMobRender<'_>,
        who: &NativePlayerRender<'_>,
        draw: &mut BlockEntityDraw,
        drawn: &mut bool,
    ) {
        mod_api::dispatch_player_submit(self, mod_api::mods(), player, who, draw, drawn);
    }
    /// A layer on every mob and player in view, once a frame after its
    /// vanilla model and layers (`EntityRenderersEvent.AddLayers`, with
    /// `RenderLivingEvent.Post` and `RenderPlayerEvent.Post`): add geometry
    /// in the entity's frame, which starts at its feet turned with the body
    /// (`mob.head_yaw`, `mob.pitch` and the walk cycle place a part, the eyes
    /// being at the entity type's eye height). `who` is set for a player.
    /// Not asked for what a mod drew itself (`native_mob_submit`,
    /// `native_player_submit`) nor for a mob that isn't drawn at all.
    /// API 149.
    pub(super) fn native_mob_layer(
        &self,
        mob: &NativeMobRender<'_>,
        who: Option<&NativePlayerRender<'_>>,
        draw: &mut BlockEntityDraw,
    ) {
        mod_api::dispatch_mob_layer(self, mod_api::mods(), mob, who, draw);
    }
    /// Icons a mod puts on map `map` each time it is drawn: held, in an
    /// item frame or on the cartography table's screen (a map's pixels are
    /// its own; nothing is saved in the map). Push `NativeMapDecoration`s
    /// (an atlas sprite, a position in half map pixels, a rotation, and
    /// whether it shows in a frame and on the screen like vanilla's markers
    /// do); they are drawn after the vanilla ones with the same pose, and a
    /// sprite the atlas lacks is left out. `map.center`, `map.scale` and
    /// `map.dimension` say which blocks the map shows, and
    /// `NativeMapDecoration::x` has the formula. Vanilla's decorations are
    /// not passed in. Asked every frame a map is drawn, so keep it cheap.
    /// The client half of `MapDecorationType` and `addDecoration`: a type
    /// registered on the server and saved in the map's data is not here.
    /// API 140.
    pub(super) fn native_map_decorations(
        &self,
        map: &NativeMapInfo<'_>,
        decorations: &mut Vec<NativeMapDecoration>,
    ) {
        mod_api::dispatch_map_decorations(self, mod_api::mods(), map, decorations);
    }
    /// A menu opens on this client for block `block`, a mod's: set `screen`
    /// when the block is yours, to lay its screen out (`native_screen_draw`
    /// draws it). The server asks it too: a slot it hides is not in the menu
    /// there either, so shift-clicks never fill it. API 19.
    pub(super) fn native_screen(&self, block: &str, screen: &mut Option<NativeScreen>) {
        mod_api::dispatch_screen(self, mod_api::mods(), block, screen);
    }
    /// Draw block `block`'s screen at `origin` (its top left, GUI pixels),
    /// over a plain panel with its slots' frames and under the items and
    /// labels, from the menu's data slots `data` (the block entity's `data`
    /// on the server): data slots are 16-bit on the wire as in vanilla (a
    /// bigger value wraps), and `data` is empty until the server's first
    /// values arrive, so read it with `get`. Draw with `native_sprite`, `native_fill` and
    /// `native_text`. API 19.
    pub(super) fn native_screen_draw(
        &self,
        block: &str,
        data: &[i32],
        batch: &mut HudBatch,
        origin: [f32; 2],
    ) {
        mod_api::dispatch_screen_draw(self, mod_api::mods(), block, data, batch, origin);
    }
    /// A screen was observed opening, at most once per frame.
    pub(super) fn native_screen_opening(&self, screen: &NativeScreenView<'_>) {
        mod_api::dispatch_screen_opening(self, mod_api::mods(), screen);
    }

    /// A screen was observed opening or changing rectangle, at most once per frame.
    pub(super) fn native_screen_init_post(&self, screen: &NativeScreenView<'_>) {
        mod_api::dispatch_screen_init_post(self, mod_api::mods(), screen);
    }

    /// A screen was observed closing, at most once per frame.
    pub(super) fn native_screen_closing(&self, screen: &NativeScreenView<'_>) {
        mod_api::dispatch_screen_closing(self, mod_api::mods(), screen);
    }
    /// The open screen has drawn itself and its tooltips (`Screen.render`,
    /// the point of `ScreenEvent.Render.Post`): draw over it with
    /// `native_fill`, `native_sprite` and `native_text`, `screen.rect` being
    /// its container panel (or the whole window) and `screen.id` the screen
    /// it is. Once a frame on the window, never while playing with no screen;
    /// `&self`, so it reads and draws only. The Win and Options screens call
    /// it on their own path, after their drawing.
    ///
    /// It does not fire for the screens drawn over gameplay after it (a
    /// server dialog, the server pack, link and code-of-conduct prompts, a
    /// book, a sign editor): those keep `Screen::Playing`, which it skips (the
    /// server pack prompt hides an open screen too, and that screen does not
    /// fire under it), and draw over what it drew, while vanilla's
    /// `ScreenEvent.Render.Post` fires for them. The title's own screens
    /// (`title`, `world_select`, `world_create`, `multiplayer`…) fire it too,
    /// their `screen.id` from the title state machine and the whole window
    /// their `rect` (API 326), and no dialog or prompt over them.
    /// API 78.
    pub(super) fn native_screen_render_post(
        &self,
        batch: &mut HudBatch,
        screen: &NativeScreenView<'_>,
    ) {
        mod_api::dispatch_screen_render_post(self, mod_api::mods(), batch, screen);
    }
    /// A mouse button went down over the open screen
    /// (`ScreenEvent.MouseButtonPressed.Pre`, vanilla's
    /// `Screen.mouseClicked`): `screen` is the screen it is, `screen.mouse`
    /// where the pointer is (so there is no position of its own), `button` the
    /// button's GLFW index (0 left, 1 right, 2 middle, 3 back, 4 forward; a
    /// button past those is the platform's own number, which differs between
    /// systems) and `modifiers` GLFW's bits (shift 1, control 2,
    /// alt 4, super 8). Set `canceled` and the press goes no further: no slot,
    /// no screen button, no drag, no hotbar key, no sort or header button.
    ///
    /// It fires before anything the screen does with the press, on the window
    /// only, and while a screen of the game's or the title's is open (Options
    /// and the pack list opened from the title screen count, with no player
    /// and no `menu`; the title's own screens since API 326, their
    /// `screen.id` from the title state machine). It does not fire on the
    /// connect and loading screens, nor for a server
    /// dialog, the server pack, link and code-of-conduct prompts, a book or a
    /// sign editor drawn over gameplay, nor while a world loads or saves: the
    /// click there stays the game's own. A gamepad click or a click the
    /// keyboard's focus sends is not the mouse and fires nothing.
    /// API 80.
    pub(super) fn native_screen_mouse_button_pressed_pre(
        &self,
        screen: &NativeScreenView<'_>,
        button: i32,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_screen_mouse_button_pressed_pre(
            self,
            mod_api::mods(),
            screen,
            button,
            modifiers,
            canceled,
        );
    }
    /// A mouse button went up over the open screen
    /// (`ScreenEvent.MouseButtonReleased.Pre`, vanilla's
    /// `Screen.mouseReleased`), with the same `screen`, `button` and
    /// `modifiers` as `native_screen_mouse_button_pressed_pre` and fired at
    /// the same point. Set `canceled` and the release goes no further: a
    /// pressed slot keeps what it took and no drag, slider or scrollbar lets
    /// go.
    ///
    /// Cancel a release only when the press it ends was cancelled too: a
    /// release the screen took ends the drag or the scroll it began, as it
    /// does in vanilla, and no guard here un-cancels it. Same screens as the
    /// press; a press that opened another screen ends on that one.
    /// API 80.
    pub(super) fn native_screen_mouse_button_released_pre(
        &self,
        screen: &NativeScreenView<'_>,
        button: i32,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_screen_mouse_button_released_pre(
            self,
            mod_api::mods(),
            screen,
            button,
            modifiers,
            canceled,
        );
    }
    /// Before a real held-button movement reaches screen drag handlers.
    /// `screen.mouse` is the current GUI position; `delta` is its GUI-space movement.
    /// Cancel keeps the new cursor position but skips all screen drag handlers.
    pub(super) fn native_screen_mouse_dragged_pre(
        &self,
        screen: &NativeScreenView<'_>,
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_screen_mouse_dragged_pre(
            self,
            mod_api::mods(),
            screen,
            button,
            delta,
            modifiers,
            canceled,
        );
    }
    /// The wheel turned over the open screen (`ScreenEvent.MouseScrolled.Pre`,
    /// vanilla's `Screen.mouseScrolled`): `screen` is the screen it is,
    /// `screen.mouse` where the pointer is (so there is no position of its
    /// own), `scroll` the wheel's movement in x and y as the game's own scroll
    /// handler scales it (a line times the wheel sensitivity: with Discrete
    /// Scrolling on, a notch is ±1, its sign only; a touchpad's pixels are
    /// divided by 30), and
    /// `modifiers` GLFW's bits (shift 1, control 2, alt 4, super 8). Set
    /// `canceled` and nothing the screen would scroll moves: not the bundle
    /// under the pointer, nor a list, slider, scrollbar or map. Nothing else
    /// reads the wheel while a screen of the game's is open, so no hotbar slot
    /// changes either.
    ///
    /// It fires before the screen scrolls anything, for the window's wheel
    /// (horizon-ctl's `scroll-at` sends it through the window too), and
    /// while a screen of the game's or the title's is open (Options and the
    /// pack list opened from the title screen count, with no player and no
    /// `menu`; the title's own since API 326). It
    /// does not fire on the connect and loading
    /// screens, nor for a server dialog, the server pack, link and
    /// code-of-conduct prompts, a book or a sign editor drawn over gameplay,
    /// nor while a world loads or saves: the scroll there stays the game's
    /// own. A wheel a gamepad or the keyboard's focus sends is not the mouse
    /// and fires nothing.
    /// API 93.
    pub(super) fn native_screen_mouse_scrolled_pre(
        &self,
        screen: &NativeScreenView<'_>,
        scroll: [f64; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_screen_mouse_scrolled_pre(
            self,
            mod_api::mods(),
            screen,
            scroll,
            modifiers,
            canceled,
        );
    }
    /// A key went down over the open screen (`ScreenEvent.KeyPressed.Pre`,
    /// vanilla's `Screen.keyPressed`): `screen` is the screen it is, `key`
    /// vanilla's name for the key (`key.keyboard.e`, the name `native_key`
    /// hands over) — a key with no name fires nothing — `repeat` whether it is
    /// the keyboard's auto-repeat of a held press (Horizon's extra; NeoForge
    /// folds repeats into the event), and `modifiers` GLFW's bits (shift 1,
    /// control 2, alt 4, super 8). Set `canceled` and the press goes no
    /// further: no focus move, no Escape-close, no inventory or hotbar key, no
    /// typing by the key itself, no `native_key`.
    ///
    /// It fires for the window's presses and their repeats, and for the
    /// control socket's `key` (a release fires nothing), after the window's
    /// own shortcuts have run — the fullscreen and screenshot keys, the debug
    /// keys and the narrator — and while a screen of the game's or the title's
    /// is open (Options and the pack list opened from the title screen count,
    /// with no player and no `menu`; the title's own since API 326). It does
    /// not fire on the connect and loading screens, nor for a server dialog, the server pack,
    /// link and code-of-conduct prompts, a book or a sign editor drawn over
    /// gameplay, nor while a world loads or saves: the key there stays the
    /// game's own. A cancelled press's release still runs, so a `native_key`
    /// listener sees that release without its press, and its characters are
    /// GLFW's separate char callback and still come; see
    /// `native_screen_char_typed_pre`.
    /// API 104.
    pub(super) fn native_screen_key_pressed_pre(
        &self,
        screen: &NativeScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_screen_key_pressed_pre(
            self,
            mod_api::mods(),
            screen,
            key,
            repeat,
            modifiers,
            canceled,
        );
    }
    /// Before an open screen handles a named key's release (same screen gate as key presses).
    /// Cancel bypasses screen handling; physical keys and mapped keyboard hold/toggle state release.
    pub(super) fn native_screen_key_released_pre(
        &self,
        screen: &NativeScreenView<'_>,
        key: &str,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_screen_key_released_pre(
            self,
            mod_api::mods(),
            screen,
            key,
            modifiers,
            canceled,
        );
    }
    /// A character was typed at the open screen
    /// (`ScreenEvent.CharacterTyped.Pre`, vanilla's `Screen.charTyped`):
    /// `screen` is the screen it is, `c` the character, and `modifiers` GLFW's
    /// bits (shift 1, control 2, alt 4, super 8). Set `canceled` and that one
    /// character never reaches a text box.
    ///
    /// It fires once per printable character of a key press's text and once
    /// per character of the input method's committed text, never for a control
    /// character (Enter, Backspace, Tab, Delete, Escape, a Control letter):
    /// those are keys, and the game reads them as keys, not as text. A key's
    /// characters fire after the key has acted, as GLFW's char callback comes
    /// after its key callback (`KeyboardHandler.charTyped`): `screen` is the
    /// screen open then, so a key that closes the screen (the inventory key)
    /// leaves none to hear its character, where vanilla's closed screen hears
    /// nothing, and one that moves the focus types into the box it moved to.
    /// They fire even when `native_screen_key_pressed_pre` cancelled the press
    /// that carried the text, as NeoForge delivers `CharacterTyped.Pre` for a
    /// cancelled `KeyPressed.Pre`: the characters left reach the text box
    /// without the key's own effects. Cancelling one character drops only that
    /// one. The control socket's `type` fires it too, but not for its `\b`
    /// backspace. Same screens as the key press, and it fires with no text box
    /// focused (there is nothing to take the character then).
    /// API 104.
    pub(super) fn native_screen_char_typed_pre(
        &self,
        screen: &NativeScreenView<'_>,
        c: char,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_screen_char_typed_pre(
            self,
            mod_api::mods(),
            screen,
            c,
            modifiers,
            canceled,
        );
    }
    /// The open screen as a native mod sees it (`NativeScreenView`): its id,
    /// the open menu's type and title, its slot count, and its panel's
    /// rectangle, with the mouse and the partial tick. `screen` is the screen
    /// drawn this frame, which `NativeScreenView.id` comes from. Borrowed:
    /// nothing is copied per frame.
    pub(super) fn screen_view(&self, batch: &HudBatch, screen: Screen) -> NativeScreenView<'_> {
        NativeScreenView {
            id: screen.id(),
            menu: self.remote_menu_type().filter(|m| !m.is_empty()),
            title: self.remote.as_ref().and_then(|r| r.menu_title.as_ref()),
            rect: self.screen_rect(batch, screen),
            slots: self.slot_count(),
            mouse: self.native_cursor(),
            partial_tick: self.clock.fraction() as f32,
        }
    }
    /// The title's own screen as a native mod sees it (`NativeScreenView`):
    /// its id from the title state machine (`title`, `world_select`,
    /// `world_create`, `multiplayer`, `add_server`…), no menu and no menu
    /// title, the whole window as its rectangle and no slots (no menu is
    /// open). None while no title screen is up. API 326.
    pub(super) fn title_screen_view(&self, batch: &HudBatch) -> Option<NativeScreenView<'_>> {
        let title = self.title.as_ref()?;
        Some(NativeScreenView {
            id: title.mode.id(),
            menu: None,
            title: None,
            rect: [0.0, 0.0, batch.width, batch.height],
            slots: 0,
            mouse: self.native_cursor(),
            partial_tick: self.clock.fraction() as f32,
        })
    }
    /// The open screen's container panel in GUI pixels (`screen`'s, the one
    /// this frame draws), or the whole window for a screen with none
    /// (`NativeScreenView.rect`).
    pub(super) fn screen_rect(&self, batch: &HudBatch, screen: Screen) -> [f32; 4] {
        if screen != Screen::Inventory && screen != Screen::Creative {
            return [0.0, 0.0, batch.width, batch.height];
        }
        // The window the game itself draws (`container_image`): the merchant
        // and the beacon are wider and shifted, creative is its own size.
        let image = self.container_image(batch);
        [
            image.left as f32,
            image.top as f32,
            image.width as f32,
            image.height as f32,
        ]
    }
    pub(super) fn track_drag_button(&mut self, button: MouseButton, pressed: bool) {
        // Any release ends the drag, as vanilla's `MouseHandler.onButton` clears `activeButton`:
        // left down, right down, left up sends no drag while right is still held.
        self.drag_button = pressed.then_some(button);
    }
    pub(super) fn screen_mouse_dragged(&self, previous: (f32, f32)) -> bool {
        if self.pad_synthetic_input() || self.cursor == previous {
            return false;
        }
        let Some(button) = self
            .drag_button
            .filter(|b| self.physical_buttons.contains(b))
        else {
            return false;
        };
        let Some(screen) = self.screen_view_now() else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_mouse_dragged_pre(
            &screen,
            super::glfw_button(button),
            [self.cursor.0 - previous.0, self.cursor.1 - previous.1],
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    /// A screen the native mods' input hooks are for is open: the window has a
    /// renderer, either the title's own (`title`, `world_select`,
    /// `world_create`, `multiplayer`…) or a screen over the world
    /// (`Screen::Playing` behind it, so not a book, a sign editor, a server
    /// dialog or a prompt drawn over gameplay), and no connect or world load
    /// is in progress. The one gate they all ask before they fire.
    pub(super) fn screen_hooks_open(&self) -> bool {
        self.renderer.is_some()
            && !self.screen_over_playing()
            && !self.remote_loading()
            && (self.screen != Screen::Playing || self.title.is_some())
    }
    /// The open screen as an input hook sees it (`NativeScreenView`), or None
    /// when `screen_hooks_open` says no screen is. `screen.mouse` is the
    /// pointer the hook's event is at, so a hook takes no position, and
    /// `partial_tick` is the clock's fraction at this event, not a frame's.
    /// The `HudBatch` built here is only what `screen.rect` needs: the
    /// window's GUI scale, no allocation.
    pub(super) fn screen_view_now(&self) -> Option<NativeScreenView<'_>> {
        if !self.screen_hooks_open() {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        let batch = HudBatch::new([renderer.config.width, renderer.config.height]);
        if self.screen != Screen::Playing {
            return Some(self.screen_view(&batch, self.screen));
        }
        self.title_screen_view(&batch)
    }
    /// Whether the key press `key` over the open screen went no further
    /// (`native_screen_key_pressed_pre`): what `handle_window_event` and the
    /// control socket's `key` command both ask before a screen acts on a key.
    /// False when no screen of the hooks' is open, and for a key with no
    /// vanilla name.
    pub(super) fn screen_key_pressed(&self, key: KeyCode, repeat: bool) -> bool {
        let Some(name) = keybinds::key_name(key) else {
            return false;
        };
        let Some(screen) = self.screen_view_now() else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_key_pressed_pre(
            &screen,
            name,
            repeat,
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    pub(super) fn screen_key_released(&self, key: KeyCode) -> bool {
        let Some(name) = keybinds::key_name(key) else {
            return false;
        };
        let Some(screen) = self.screen_view_now() else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_key_released_pre(
            &screen,
            name,
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    /// Text on its way to a text box after the open screen's character hooks
    /// (`native_screen_char_typed_pre` per printable character): the cancelled
    /// characters are gone and every other one stays, control characters
    /// included (a text box reads Backspace, Delete and the like as keys).
    /// `None` when none of them is left to type.
    pub(super) fn screen_text_typed(&self, text: &str) -> Option<String> {
        let Some(screen) = self.screen_view_now() else {
            return Some(text.to_owned());
        };
        let modifiers = super::glfw_modifiers(self.modifiers);
        screen_text_hooked(text, |c| {
            let mut canceled = false;
            self.native_screen_char_typed_pre(&screen, c, modifiers, &mut canceled);
            canceled
        })
    }
    /// A key press's characters (`ScreenEvent.CharacterTyped.Pre`, vanilla's
    /// `KeyboardHandler.charTyped`), dispatched once the screen has acted on
    /// the key: GLFW's char callback comes after its key callback, so the
    /// characters go to the screen that is open now — a key that closed the
    /// screen leaves none to hear them, as vanilla's closed screen hears
    /// nothing, and one that moved the focus types into the box it moved to.
    /// Each printable one is offered to `native_screen_char_typed_pre`; the
    /// survivors go to the focused text box, control characters (keys) gone.
    pub(super) fn screen_chars_typed(&mut self, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        let Some(screen) = self.screen_view_now() else {
            return;
        };
        let modifiers = super::glfw_modifiers(self.modifiers);
        let Some(text) = chars_to_type(text, |c| {
            let mut canceled = false;
            self.native_screen_char_typed_pre(&screen, c, modifiers, &mut canceled);
            canceled
        }) else {
            return;
        };
        // A held edit modifier makes a shortcut of the key, not text.
        if !self.types_text() {
            return;
        }
        // `Err` only says no text box of the screen takes the characters.
        let _ = self.type_chars_into_screen(&text);
    }
    /// A left or right (`right`) press at `at` (GUI pixels from the screen's
    /// top left, so outside it too) on block `block`'s screen, a mod's, over
    /// no slot: set `button` to a menu button id when it is on one of yours.
    /// Horizon sends it to the server (`ContainerButtonClick`,
    /// `native_menu_button` there) and the press goes no further. API 20.
    pub(super) fn native_screen_click(
        &self,
        block: &str,
        data: &[i32],
        at: [f32; 2],
        right: bool,
        button: &mut Option<i32>,
    ) {
        mod_api::dispatch_screen_click(self, mod_api::mods(), block, data, at, right, button);
    }
    /// The tooltip at `at` (as in `native_screen_click`) on block `block`'s
    /// screen, a mod's, over no item: push its lines when it is over
    /// something of yours. Legacy colour codes (§) work. API 20.
    pub(super) fn native_screen_tooltip(
        &self,
        block: &str,
        data: &[i32],
        at: [f32; 2],
        lines: &mut Vec<String>,
    ) {
        mod_api::dispatch_screen_tooltip(self, mod_api::mods(), block, data, at, lines);
    }
    /// A player pressed menu button `id` on the menu of the mod block entity
    /// at `pos` (`AbstractContainerMenu.clickMenuButton`), on the server:
    /// change `entity` when its id is yours. It is out of the world during
    /// the call, as in `native_block_entity_tick`. API 20.
    pub(super) fn native_menu_button(
        &mut self,
        pos: [i32; 3],
        entity: &mut ModBlockEntity,
        id: i32,
    ) {
        mod_api::dispatch_menu_button(self, mod_api::mods(), pos, entity, id);
    }
    /// `AbstractContainerMenu.clickMenuButton(Player, int)`'s other half:
    /// the acting player's cursor stack, `getCarried()`/`setCarried()`,
    /// which `native_menu_button` cannot reach. Fires right after it, same
    /// moment, same `pos`/`entity`/`id`; change `carried` when its id is
    /// yours. On the server. API 51.
    pub(super) fn native_menu_button_v2(
        &mut self,
        pos: [i32; 3],
        entity: &mut ModBlockEntity,
        id: i32,
        carried: &mut Option<ItemStack>,
    ) {
        mod_api::dispatch_menu_button_v2(self, mod_api::mods(), pos, entity, id, carried);
    }
    /// A player shift-clicked slot `slot` of the menu of the mod block
    /// entity at `pos` (`AbstractContainerMenu.quickMoveStack`, the method a
    /// Java menu overrides), on the server, before the game's own routing:
    /// move the stack where your block wants it. `slot` is the menu's index:
    /// below `container_slots` it is the block's own slot (`entity.items`),
    /// from there on the player's inventory (27 main slots, then the hotbar).
    /// `stack` is a copy of what the slot holds; take what you move out of
    /// it (put it in `entity.items`, or in another block with
    /// `native_insert_item`) and set `handled`: the slot then holds what is
    /// left of `stack` (whatever you wrote to that slot of `entity.items`),
    /// the game's routing is skipped, and the player's client gets the menu
    /// again. Left unhandled, `entity.items` goes back to what it held and
    /// the game moves the stack as for any container; a later mod sees what
    /// an earlier one did to `stack`.
    ///
    /// For a mod container on a vanilla menu type and on a mod menu type,
    /// the left or right button, a slot that holds something and that the
    /// mod's screen shows; not for a spectator or a dead player, a menu no
    /// longer valid, nor during a drag.
    /// Asked once per click, where vanilla asks `quickMoveStack` again while
    /// it keeps moving the same item. The player's state is swapped in, so
    /// `native_acting_player` names them; `entity` is out of the world
    /// during the call, as in `native_menu_button`.
    /// API 309.
    pub(super) fn native_menu_quick_move(
        &mut self,
        pos: [i32; 3],
        entity: &mut ModBlockEntity,
        container_slots: usize,
        slot: usize,
        stack: &mut ItemStack,
        handled: &mut bool,
    ) {
        mod_api::dispatch_menu_quick_move(
            self,
            mod_api::mods(),
            pos,
            entity,
            container_slots,
            slot,
            stack,
            handled,
        );
    }
    /// The commands native mods add to the server's command tree: push
    /// yours. Asked once when a world opens to players (a dedicated
    /// server, a LAN host, single player's server thread); every player's
    /// chat box then completes them as vanilla's, within their level. A
    /// name vanilla has is left out. API 8.
    pub(super) fn native_register_commands(&mut self, commands: &mut Vec<NativeCommand>) {
        mod_api::dispatch_register_commands(self, mod_api::mods(), commands);
    }
    /// A player's command on the server (`Commands.performCommand`),
    /// without its slash, for a command a mod registered and the player's
    /// level allows: set `ran` when it is yours. The player's state is
    /// swapped in for the call, so `native_player` and the other helpers
    /// read that player; `native_feedback` answers them. A single player's
    /// owner console (`horizon-ctl command`) reaches it too, as the owner at
    /// level 4. API 8.
    pub(super) fn native_perform_command(
        &mut self,
        player: &NativeServerPlayer,
        command: &str,
        ran: &mut bool,
    ) {
        mod_api::dispatch_perform_command(self, mod_api::mods(), player, command, ran);
    }
    /// A player takes an item from an armor stand or puts one on it
    /// (`ArmorStand.interactAt`, after the stand's disabled-slot checks and
    /// `swapItem`'s take/put locks, before anything moves): `stand_id` is the
    /// stand's `Entity.getId` and `slot` the one the click reached
    /// (`mainhand`, `offhand`, `feet`, `legs`, `chest` or `head`). Set
    /// `cancel` and nothing changes: neither the stand nor the player's hand,
    /// and the click is spent. The player's state is swapped in for the call;
    /// `player` is the UUID the helpers take. Not told of a click that changes
    /// nothing (an empty hand on an empty slot). Not covered, other ways a
    /// stand's equipment moves: breaking or blasting the stand (its items
    /// drop), a dispenser putting armour on one, a mod's own changes.
    /// Server-side, for every player of the world.
    /// 175.
    pub(super) fn native_armor_stand_manipulate(
        &mut self,
        player: u128,
        name: &str,
        stand_id: i32,
        slot: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_armor_stand_manipulate(
            self,
            mod_api::mods(),
            player,
            name,
            stand_id,
            slot,
            cancel,
        );
    }
    /// A boat or minecart (`kind`, `minecraft:oak_boat`) moved since the last
    /// tick (`VehicleMoveEvent`): `from` and `to` are its positions, heard
    /// after the tick moved it. Once a tick for each vehicle that carries a
    /// server player, whose state is then swapped in (as for
    /// `native_perform_command`: `native_acting_player`, `native_tell` and
    /// the item helpers reach them; `player` is their UUID), and for one a
    /// server player rode and left, until it has stood still for ten
    /// seconds (`player` is none, nobody is swapped in). It only hears.
    /// Cheap when nothing moves: one position compare a tick for each watched
    /// vehicle. Not heard: a vehicle moved before anyone rode it, a mount
    /// (horse, pig...) or a vehicle carrying only a mob. Server-side, in the
    /// level the vehicle is in. 178.
    pub(super) fn native_vehicle_moved(
        &mut self,
        player: Option<u128>,
        kind: &str,
        from: [f64; 3],
        to: [f64; 3],
    ) {
        mod_api::dispatch_vehicle_moved(self, mod_api::mods(), player, kind, from, to);
    }
    /// A player opens one of the game's own menus (`ServerPlayer.openMenu`; the
    /// job of NeoForge's `PlayerContainerEvent.Open`, which comes after the
    /// menu is open and cannot stop it): `kind` is the menu type
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:crafting`,
    /// `minecraft:merchant`, `minecraft:anvil`, `minecraft:lectern`...; a
    /// horse's inventory is `minecraft:horse_inventory`) and `pos` the block
    /// it belongs to, none for a villager, a mount or a vehicle's chest.
    /// Heard after the checks that refuse the menu (a locked container, a
    /// spectator) and before the menu that was open closes. Set `cancel` and
    /// nothing opens: the open menu stays as it was and the click is spent.
    /// The player's state is swapped in for the call, as for
    /// `native_perform_command`, so `native_acting_player`, `native_tell` and
    /// the item helpers reach them. Not heard: a menu a mod opens
    /// (`native_open_menu`), the inventory screen, and the editors (sign,
    /// book, command block). Server-side, for every player of the world.
    /// 176.
    pub(super) fn native_container_opened(
        &mut self,
        player: u128,
        name: &str,
        kind: &str,
        pos: Option<[i32; 3]>,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_container_opened(self, mod_api::mods(), player, name, kind, pos, cancel);
    }
    /// A creative-mode player sets a slot of their inventory from the
    /// creative screen (`ServerGamePacketListenerImpl.handleSetCreativeModeSlot`):
    /// `slot` is the inventory menu's slot (5 to 8 armour, 9 to 35 the
    /// inventory, 36 to 44 the hotbar, 45 the off hand; below 0 is a stack
    /// thrown out of the screen, which is dropped) and `item` (`minecraft:air`
    /// for an emptied slot) and `count` what is put there. Heard once the
    /// packet passed vanilla's checks (a creative player, a real slot, a valid
    /// stack) and before the slot changes or the stack drops. Set `cancel`
    /// and the packet is ignored. The player's state is swapped in for the
    /// call, as for `native_perform_command`, so `native_acting_player`,
    /// `native_tell` and the item helpers reach them. Not heard: stacks a
    /// creative player gets other ways (middle-click pick block, `/give`).
    /// Server-side, for every player of the world.
    /// 177.
    pub(super) fn native_creative_slot(
        &mut self,
        player: u128,
        name: &str,
        slot: i32,
        item: &str,
        count: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_creative_slot(
            self,
            mod_api::mods(),
            player,
            name,
            slot,
            item,
            count,
            cancel,
        );
    }
    /// A server player's game mode is about to change (NeoForge's
    /// `PlayerChangeGameModeEvent`): from `from` to `to` (0 survival, 1
    /// creative, 2 adventure, 3 spectator, as `NativeServerPlayer`'s
    /// `game_mode`), `player` being the UUID and `name` the name. Heard for
    /// the `/gamemode` command, the F3+F4 switcher's packet and a hardcore
    /// world's respawn into spectator, once for each change to a different
    /// mode; not for the world's default mode a player joins with. Set
    /// `cancel` and the player keeps their mode (nothing is sent, the
    /// command counts it as no change). For a change the game makes inside
    /// a player's turn their state is swapped in; for the hardcore respawn
    /// it is not. 158.
    pub(super) fn native_player_game_mode_changed(
        &mut self,
        player: u128,
        name: &str,
        from: i32,
        to: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_game_mode_changed(
            self,
            mod_api::mods(),
            player,
            name,
            from,
            to,
            cancel,
        );
    }
    /// `player` (`name`) changed dimension (NeoForge's
    /// `PlayerChangedDimensionEvent`): `from` and `to` are dimension ids
    /// (`minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`).
    /// Heard once per move, when it has ended: the portal's exit is found
    /// and the player stands there (after the destination has loaded), a
    /// command's teleport has landed, or a respawn in another level than the
    /// death's is placed for good (a bed or anchor, or the world spawn's
    /// free column, after the fallback when a respawn block was gone). The
    /// level of `to` is the one in place, and the player's state is swapped
    /// in, as for `native_perform_command`: `native_acting_player`,
    /// `native_tell`, the item helpers and the rest reach them. Heard for:
    /// Nether and End portals (also riding a vehicle, and leaving the End as
    /// its winner), `/execute in ... run tp` and any other command that
    /// teleports a player into another level, and the death respawn. Not
    /// heard: a move within a level (`/tp`, an End gateway), a player who
    /// leaves before the move ends, a move that ends where it started (a
    /// respawn in the dimension of the death), and anything outside a
    /// dedicated server. 154.
    pub(super) fn native_player_changed_dimension(
        &mut self,
        player: u128,
        name: &str,
        from: &str,
        to: &str,
    ) {
        mod_api::dispatch_player_changed_dimension(self, mod_api::mods(), player, name, from, to);
    }
    /// A player's client started (`flying`) or stopped flying (vanilla's
    /// `handlePlayerAbilities`, which honours it only for a player who may
    /// fly: creative or spectator), heard once for each change, on a server,
    /// with the player's state swapped in, so `native_tell` and the other
    /// helpers reach them. The player is already flying or not as asked, as
    /// vanilla leaves them; set `cancel` to refuse it: they keep the state
    /// they had and the server sends their abilities back, as vanilla does
    /// for what it refuses, so the client lands or flies again. Only what a
    /// client asks for is heard: not the flight a game-mode change starts or
    /// ends (`/gamemode`), and not a LAN host's own player.
    /// 153.
    pub(super) fn native_player_toggle_flight(
        &mut self,
        player: u128,
        name: &str,
        flying: bool,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_toggle_flight(self, mod_api::mods(), player, name, flying, cancel);
    }
    /// A guest's client changed sneaking in its player input packet, after the hub applied the new state.
    /// The guest's state is swapped in. Server-side. 351.
    pub(super) fn native_player_toggle_sneak(&mut self, player: u128, name: &str, sneaking: bool) {
        mod_api::dispatch_player_toggle_sneak(self, mod_api::mods(), player, name, sneaking);
    }
    /// A player finished editing a sign: `ServerGamePacketListenerImpl.updateSignText`
    /// has taken their update packet and every check has passed (they were
    /// editing that sign, within reach, the sign is not waxed, the chat filter
    /// let the text through), right before the lines go on the sign. `pos` in
    /// `dimension`, the `front` or back side, and the four `lines` as written,
    /// plain text with the formatting codes still in (vanilla strips them
    /// afterwards). Rewrite `lines` (made four lines again) and that is what
    /// the sign gets, without a second look from the chat filter, or set
    /// `cancel` and the sign keeps its old text. With the player's state
    /// swapped in, so `native_tell` and the helpers that act on the player
    /// reach them. Once for each update a player sends on a server. Not told:
    /// the host's own editing of a sign on a LAN or single-player world
    /// (the editor writes into their world directly), and the sign changes
    /// that are no edit of the text (dyeing, glow ink, waxing, `/data`,
    /// `/setblock`, mods' `native_with_mod_block_entity`).
    /// 150.
    pub(super) fn native_sign_changed(
        &mut self,
        player: u128,
        name: &str,
        dimension: &str,
        pos: [i32; 3],
        front: bool,
        lines: &mut Vec<String>,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_sign_changed(
            self,
            mod_api::mods(),
            player,
            name,
            dimension,
            pos,
            front,
            lines,
            cancel,
        );
    }
    /// A command a player typed (NeoForge's `CommandEvent`, before
    /// `Commands.performCommand` runs it), without its slash: any command,
    /// vanilla's and the mods' own, also one the player may not run. Set
    /// `canceled` to stop it: nothing runs and nothing is answered but the
    /// lines pushed to `replies`, which go to the player as system chat.
    /// No player's state is swapped in and the server's player list is busy
    /// with this one, so `native_tell` and the other helpers that take a UUID
    /// reach nobody here: answer through `replies`. Once for each command a
    /// player sends, on a server, and for each line of a quiet command batch
    /// (`horizon:quiet_commands`, whose replies are dropped). A command the
    /// server refuses before reading it (unsigned chat arguments, the chat
    /// rate limit) never reaches it. 95.
    pub(super) fn native_player_command(
        &mut self,
        player: u128,
        name: &str,
        command: &str,
        canceled: &mut bool,
        replies: &mut Vec<Text>,
    ) {
        mod_api::dispatch_player_command(
            self,
            mod_api::mods(),
            player,
            name,
            command,
            replies,
            canceled,
        );
    }
    /// A command a mod registered, typed on the dedicated server's console
    /// or sent through `horizon-server console`
    /// (`DedicatedServer.handleConsoleInputs`), without its slash: set `ran`
    /// when it is yours. There is no player: the source is the console, at
    /// permission level 4, so `native_acting_player` is None and the
    /// player-only helpers give nothing; `native_feedback` answers on the
    /// console. A name no mod registered goes to vanilla's dispatcher as
    /// before. 99.
    pub(super) fn native_perform_console_command(&mut self, command: &str, ran: &mut bool) {
        mod_api::dispatch_perform_console_command(self, mod_api::mods(), command, ran);
    }
    /// A server player's ender pearl landed and is about to teleport them
    /// (NeoForge's `EntityTeleportEvent.EnderPearl`, vanilla's
    /// `ThrownEnderpearl.onHit` for a player owner): `pearl` is the pearl's
    /// entity id, `from` their feet now, `target` where the pearl started
    /// its last tick and `damage` the fall damage they then take (5). A mod
    /// may move `target` (a value that is not a number is ignored) or change
    /// `damage` (0 or less: none). Canceling
    /// keeps the player where they are: no endermite roll, no teleport, no
    /// damage and no sound; the pearl is used up either way. The player's
    /// state is swapped in (`native_acting_player`). The hit itself is
    /// heard by `native_projectile_impact`. 312.
    pub(super) fn native_ender_pearl_teleport(
        &mut self,
        player: u128,
        name: &str,
        pearl: u32,
        from: [f64; 3],
        target: &mut [f64; 3],
        damage: &mut f32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_ender_pearl_teleport(
            self,
            mod_api::mods(),
            player,
            name,
            pearl,
            from,
            target,
            damage,
            cancel,
        );
    }
    /// A server player who ate chorus fruit (or another item with the
    /// `teleport_randomly` consume effect) is about to try a random spot
    /// (NeoForge's `EntityTeleportEvent.ChorusFruit`, vanilla's
    /// `TeleportRandomlyConsumeEffect.apply`): once per attempt (up to 16),
    /// after they stopped riding. `from` is their feet, `target` the spot
    /// drawn; a mod may move it (its y kept inside the level's height, a
    /// value that is not a number ignored), and the game still looks down
    /// from it for ground it may land on. Canceling ends the effect: no teleport and no
    /// more attempts. The player's state is swapped in
    /// (`native_acting_player`). Foxes are not heard. 311.
    pub(super) fn native_chorus_fruit_teleport(
        &mut self,
        player: u128,
        name: &str,
        from: [f64; 3],
        target: &mut [f64; 3],
        cancel: &mut bool,
    ) {
        mod_api::dispatch_chorus_fruit_teleport(
            self,
            mod_api::mods(),
            player,
            name,
            from,
            target,
            cancel,
        );
    }
    /// A server player a command teleported (NeoForge's
    /// `EntityTeleportEvent`, for a player): `/tp` and `/teleport`, inside
    /// their level or into another one. `from` and `to` are their feet before
    /// and after, `dimension` the level they are in now. Heard after the move
    /// has reached the player's state and their client (an inter-level
    /// teleport: when the new level is entered, before it has loaded around
    /// them), so it cannot stop it. The player's state is swapped in, so the
    /// helpers that read the acting player and `native_tell` work. Once for
    /// each teleported player, on a server. Ender pearls and chorus fruit
    /// have their own hooks; a spectator's teleport menu is not heard yet;
    /// portals and respawns are not teleports here. 170.
    pub(super) fn native_player_teleported(
        &mut self,
        player: u128,
        name: &str,
        from: [f64; 3],
        to: [f64; 3],
        dimension: &str,
    ) {
        mod_api::dispatch_player_teleported(
            self,
            mod_api::mods(),
            player,
            name,
            from,
            to,
            dimension,
        );
    }
    /// A player whose login the server has accepted (NeoForge's
    /// `PlayerNegotiationEvent` stage): the name, whitelist, ban and
    /// server-full checks have passed and the player holds a place, but their
    /// saved state is not loaded and they are not placed in a level yet. Set
    /// `cancel` to refuse them: they are disconnected with `reason` (a plain
    /// "Disconnected" if it is left unset) and nothing else of the join (the
    /// join message, `native_player_logged_in`) happens. Once for each player
    /// who joins, on a server. The player is not placed yet and their client
    /// has no world to show a message in, so no player's state is swapped in
    /// and `native_tell` reaches nobody: the refusal `reason` is the answer.
    /// The checks that need no mod (name, whitelist, bans, a full server,
    /// the proxy's forwarding) refuse earlier, on the connection, and this
    /// hook does not hear those players. 172.
    pub(super) fn native_player_login(
        &mut self,
        player: u128,
        name: &str,
        cancel: &mut bool,
        reason: &mut Option<Text>,
    ) {
        mod_api::dispatch_player_login(self, mod_api::mods(), player, name, reason, cancel);
    }
    /// The command tree a server sends a player (NeoForge's and Paper's
    /// "commands sent to a player" stage): when they join and when their
    /// level changes (`/op`, `/deop`, the LAN settings). `names` are the
    /// root commands their client would get, vanilla's they may use and the
    /// mods'; take a name out and that client gets the tree without it
    /// (and without what it leads to), so it neither completes nor
    /// highlights it. This hides the command from the client only: the
    /// server still runs a typed one, `native_player_command` stops it.
    /// For a player who is already on, their state is swapped in and the
    /// helpers (`native_tell`...) work; for one who is joining it is not, and
    /// their client has no world to tell yet. The tree the hub sends again by
    /// itself (after a respawn or a change of level) keeps the last answer
    /// and does not ask again; the integrated server's owner is not asked
    /// when the world's cheats are switched. 171.
    pub(super) fn native_player_commands_sent(
        &mut self,
        player: u128,
        name: &str,
        names: &mut Vec<String>,
    ) {
        mod_api::dispatch_player_commands_sent(self, mod_api::mods(), player, name, names);
    }
    /// A server player's sprint started or stopped: their client sent
    /// `START_SPRINTING` or `STOP_SPRINTING` (the sprint key, a double tap
    /// forward, or letting go) and the server's player is now `sprinting`
    /// (vanilla's `setSprinting` in `handlePlayerCommand`). Heard only when it
    /// changes. It cannot be stopped, and it hears what the client asks, not
    /// what the server does: a sprint that ends by itself (a hit's knockback,
    /// hunger, a wall) is not reported. The player's state is swapped in, so
    /// the helpers and `native_tell` work. Once for each change, on a server.
    /// 173.
    pub(super) fn native_player_sprint(&mut self, player: u128, name: &str, sprinting: bool) {
        mod_api::dispatch_player_sprint(self, mod_api::mods(), player, name, sprinting);
    }
    /// A server player's client answered a resource pack in play (vanilla's
    /// `ServerboundResourcePackPacket`): `status` is `Action`'s number, 0
    /// successfully loaded, 1 declined, 2 failed download, 3 accepted, 4
    /// downloaded, 5 invalid URL, 6 failed reload, 7 discarded. Heard when the
    /// packet arrives, after the server has handled it; it cannot be stopped.
    /// The player's state is swapped in, so the helpers and `native_tell`
    /// work. The pack a dedicated server offers in the configuration
    /// (`resource-pack` in `server.properties`) is answered before the
    /// player has joined, and those answers are not heard. Once for each
    /// answer, on a server. 174.
    pub(super) fn native_resource_pack_status(&mut self, player: u128, name: &str, status: i32) {
        mod_api::dispatch_resource_pack_status(self, mod_api::mods(), player, name, status);
    }
    /// A server player places a hanging entity: an item frame
    /// (`minecraft:item_frame`, `minecraft:glow_item_frame`), a painting
    /// (`minecraft:painting`) or a leash knot (`minecraft:leash_knot`, when a
    /// lead is tied to a fence that has none yet), `kind` its entity type and
    /// `pos` the block it takes (the fence's for a knot). Where
    /// `HangingEntityItem.useOn` and `LeadItem.useOn` create it, once it is
    /// known to fit and before anything is placed or used up. Set `cancel`
    /// and nothing is placed: the item stays in the player's hand. The
    /// player's state is swapped in, so `native_tell` answers them. Only for
    /// guests of a server; `/summon`, a dispenser and the game's own player
    /// are not heard, nor is a lead tied to a mob or a boat.
    /// 180.
    pub(super) fn native_hanging_place(
        &mut self,
        player: u128,
        name: &str,
        kind: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        mod_api::dispatch_hanging_place(self, mod_api::mods(), player, name, kind, pos, cancel);
    }
    /// An item frame (`minecraft:item_frame`, `minecraft:glow_item_frame`) or
    /// a painting (`minecraft:painting`), `kind` its entity type and `pos` the
    /// block it hangs in, is about to break: a server player's hit
    /// (`player`, their state swapped in, so `native_tell` answers them), a
    /// blast, or the wall it hangs on gone (`player` None for both). Where
    /// `BlockAttachedEntity.hurtServer` and its survival check would kill
    /// it, before anything drops or sounds. Set `cancel` and it stays, as it
    /// was. A hit that only takes the item out of a frame does not break the
    /// frame and is not asked. The game's own player's hit is not asked, nor
    /// is `/kill`. 319.
    pub(super) fn native_hanging_break(
        &mut self,
        player: Option<u128>,
        kind: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        mod_api::dispatch_hanging_break(self, mod_api::mods(), player, kind, pos, cancel);
    }
    /// A server player empties a bucket at `pos` (`fluid`: `minecraft:water`,
    /// `minecraft:lava`, `minecraft:powder_snow`; a fish or axolotl bucket
    /// empties `minecraft:water`): where `BucketItem.use` has found the block
    /// the fluid goes into and the player may build there, right before it is
    /// placed. Set `cancel` and nothing is poured and the bucket stays full.
    /// The player's state is swapped in, so `native_tell` answers them. Only
    /// for guests of a server. `native_fill_bucket` hears this click too (and
    /// the filling one) without the player and the fluid. Not heard: a
    /// dispenser, a cauldron, and a fluid a block places itself.
    /// 179.
    pub(super) fn native_bucket_empty(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        fluid: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_bucket_empty(self, mod_api::mods(), player, name, pos, fluid, cancel);
    }
    /// TNT at `pos` is about to be lit (primed): a flint and steel or a fire
    /// charge on it, fire burning it, a redstone signal, an arrow, or a
    /// dispenser's flint and steel or TNT item. `player` is the server player
    /// whose click lit it, with their state swapped in (so `native_tell`
    /// answers them), None when no player is behind it (fire, redstone, an
    /// arrow, a dispenser). Set `cancel` and it is not primed: a TNT block
    /// stays where it is (one that fire or a signal had already taken out is
    /// put back, without a block update), a dispenser's TNT item stays in it.
    /// Not asked: TNT set off by a blast (`native_explosion_detonate` spares
    /// blocks), a TNT minecart and a sulfur cube. Server-side and in single
    /// player. 181.
    pub(super) fn native_tnt_primed(
        &mut self,
        player: Option<u128>,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        mod_api::dispatch_tnt_primed(self, mod_api::mods(), player, pos, cancel);
    }
    /// A server player selected a new hotbar slot. The player's state is not
    /// swapped in, and helpers that look them up through `self.lan` may find
    /// nothing while a guest packet is being handled. Set `cancel` to keep
    /// the old slot; the client is sent that slot again. 263.
    pub(super) fn native_player_item_held(
        &mut self,
        player: u128,
        name: &str,
        from_slot: i32,
        to_slot: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_item_held(
            self,
            mod_api::mods(),
            player,
            name,
            from_slot,
            to_slot,
            cancel,
        );
    }
    /// A server player picked an item from a block or entity, before it enters
    /// the hotbar. `pos` is zero for an entity; `entity_id` is zero for a
    /// block. The player's state is swapped in, though helpers that look them
    /// up through `self.lan` may find nothing while a guest packet is handled.
    /// Set `cancel` to leave the inventory and client unchanged. 269.
    pub(super) fn native_player_pick_item(
        &mut self,
        player: u128,
        name: &str,
        from_entity: bool,
        pos: [i32; 3],
        entity_id: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_pick_item(
            self,
            mod_api::mods(),
            player,
            name,
            from_entity,
            pos,
            entity_id,
            cancel,
        );
    }
    /// A server player is about to enter a bed after the sleep checks pass.
    /// Set `cancel` to keep them out. The player's state is swapped in, but
    /// helpers that look them up through `self.lan` may find nothing during a
    /// guest packet. 251.
    pub(super) fn native_player_bed_enter(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_bed_enter(self, mod_api::mods(), player, name, pos, cancel);
    }
    /// while a guest packet is handled. 252.
    pub(super) fn native_player_bed_leave(&mut self, player: u128, name: &str, pos: [i32; 3]) {
        mod_api::dispatch_player_bed_leave(self, mod_api::mods(), player, name, pos);
    }
    /// A server player's book edit or signing packet is about to be applied.
    /// `pages` is its page count; `signing` means the packet has a title. Set
    /// `cancel` to drop the edit. Helpers looking up the guest through
    /// `self.lan` may find nothing during the packet. 257.
    pub(super) fn native_player_edit_book(
        &mut self,
        player: u128,
        name: &str,
        pages: i32,
        signing: bool,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_edit_book(
            self,
            mod_api::mods(),
            player,
            name,
            pages,
            signing,
            cancel,
        );
    }
    /// A player shearing a mob is about to do so. Set `cancel` to leave the
    /// mob and shears unchanged. The guest's state is swapped in, though
    /// helpers looking them up through `self.lan` may find nothing during a
    /// guest packet. 275.
    pub(super) fn native_player_shear_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
        off_hand: bool,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_shear_entity(
            self,
            mod_api::mods(),
            player,
            name,
            entity_id,
            off_hand,
            cancel,
        );
    }
    /// A server player completed an advancement, after progress was recorded.
    /// Helpers looking up the guest through `self.lan` may find nothing during
    /// its packet. 249.
    pub(super) fn native_player_advancement_done(
        &mut self,
        player: u128,
        name: &str,
        advancement: &str,
    ) {
        mod_api::dispatch_player_advancement_done(self, mod_api::mods(), player, name, advancement);
    }
    /// A criterion of a server player's advancement was granted or revoked,
    /// after its progress changed and before any completion rewards, except
    /// the `minecraft:tick` criteria granted on a network player's first tick
    /// or a reload, which are heard after that tick's rewards and packets. A
    /// grant or revoke that changes nothing is not heard. `granted` is false for a
    /// revoke. Helpers looking up the guest through `self.lan` may find
    /// nothing during its packet. 320.
    pub(super) fn native_player_advancement_progress(
        &mut self,
        player: u128,
        name: &str,
        advancement: &str,
        criterion: &str,
        granted: bool,
    ) {
        mod_api::dispatch_player_advancement_progress(
            self,
            mod_api::mods(),
            player,
            name,
            advancement,
            criterion,
            granted,
        );
    }
    /// A statistic is about to be awarded to a server player, before its count
    /// and scoreboard criteria change. `stat_type` and `stat` are ids such as
    /// `minecraft:mined` and `minecraft:stone`, and `value` the amount. Set
    /// `cancel` to award nothing. Helpers looking up the guest through
    /// `self.lan` may find nothing during its packet. Server-side.
    /// 321.
    pub(super) fn native_player_stat_award(
        &mut self,
        player: u128,
        name: &str,
        stat_type: &str,
        stat: &str,
        value: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_stat_award(
            self,
            mod_api::mods(),
            player,
            name,
            stat_type,
            stat,
            value,
            cancel,
        );
    }
    /// A server player's experience points are about to increase from an orb pickup,
    /// after mending has taken its share. Change `amount` to change the points
    /// added. The player's state is not swapped in, so helpers that look them up
    /// through `self.lan` may find nothing while a guest packet is handled.
    /// Server-side. 260.
    pub(super) fn native_player_exp_change(&mut self, player: u128, name: &str, amount: &mut i32) {
        mod_api::dispatch_player_exp_change(self, mod_api::mods(), player, name, amount);
    }
    /// An item cooldown is about to start for a server player. `item` is the
    /// item id and `ticks` its duration. The player's state is not swapped in,
    /// so helpers that look them up through `self.lan` may find nothing while a
    /// guest packet is handled. Set `cancel` to skip starting the cooldown.
    /// Server-side. 262.
    pub(super) fn native_player_item_cooldown(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        ticks: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_item_cooldown(
            self,
            mod_api::mods(),
            player,
            name,
            item,
            ticks,
            cancel,
        );
    }
    /// A recipe is about to be unlocked for a server player. Set `cancel` to
    /// keep it locked. The player's state is not swapped in, so helpers that
    /// look them up through `self.lan` may find nothing while a guest packet
    /// is handled. Server-side. 273.
    pub(super) fn native_player_recipe_discovered(
        &mut self,
        player: u128,
        name: &str,
        recipe: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_recipe_discovered(
            self,
            mod_api::mods(),
            player,
            name,
            recipe,
            cancel,
        );
    }
    /// A server player's respawn point is about to change. `cause` is `bed`,
    /// `respawn_anchor`, `command` or `death_reset`; `pos` is its block
    /// position. The player's state is not swapped in, so helpers that look
    /// them up through `self.lan` may find nothing while a guest packet is
    /// handled. Set `cancel` to keep the old point. Server-side.
    /// 277.
    pub(super) fn native_player_spawn_point_set(
        &mut self,
        player: u128,
        name: &str,
        dimension: &str,
        pos: [i32; 3],
        cause: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_spawn_point_set(
            self,
            mod_api::mods(),
            player,
            name,
            dimension,
            pos,
            cause,
            cancel,
        );
    }
    /// A server player is about to travel through a Nether portal, End portal
    /// or End gateway. `cause` is `nether_portal`, `end_portal` or `end_gateway`.
    /// The player's state is not swapped in, so helpers that look them up
    /// through `self.lan` may find nothing while a guest packet is handled.
    /// Set `cancel` to keep them in their current place. Server-side.
    /// 271.
    pub(super) fn native_player_portal(
        &mut self,
        player: u128,
        name: &str,
        from_dimension: &str,
        to_dimension: &str,
        cause: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_portal(
            self,
            mod_api::mods(),
            player,
            name,
            from_dimension,
            to_dimension,
            cause,
            cancel,
        );
    }
    /// A server player launched with a riptide trident, after the velocity,
    /// spin attack and grounded lift have been applied. The player's state is
    /// swapped in; during a guest packet, helpers that look them up through
    /// `self.lan` may find nothing. No reply is needed. 274.
    pub(super) fn native_player_riptide(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_riptide(self, mod_api::mods(), player, name);
    }
    /// A server player's fishing rod, with `state`: 0 the cast
    /// (`FishingRodItem.use` throwing the bobber), 1 a fish biting
    /// (`FishingHook.catchingFish`), 2 the reel-in of a bite, which gives the
    /// catch, 3 the reel-in with nothing on the line, 4 the reel-in of a
    /// bobber hooked to an entity or item (like Bukkit's `PlayerFishEvent`;
    /// NeoForge's `ItemFishedEvent` is later, in `FishingHook.retrieve`). Set `cancel` on the cast and nothing
    /// is thrown; on a bite the fish gets away and the wait starts over; on
    /// a reel-in (2 to 4) nothing happens, the bobber stays out and the rod
    /// takes no wear. The bobber ending by itself (too far, the rod put
    /// away, the owner dying), nor the host of a LAN world's rod. The
    /// player's state is swapped in, so `native_acting_player`, `native_tell`
    /// and the item helpers reach them. Once for each cast, bite and reel-in,
    /// on a server. 160.
    pub(super) fn native_player_fish(
        &mut self,
        player: u128,
        name: &str,
        state: i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_fish(self, mod_api::mods(), player, name, state, cancel);
    }
    /// A sculk sensor or shrieker received a vibration for `event` at `pos`.
    /// Set `cancel` to suppress its response. Server-side.
    /// 196.
    pub(super) fn native_block_receive_game(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        block: &str,
        event: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_block_receive_game(
            self,
            mod_api::mods(),
            dimension,
            pos,
            block,
            event,
            cancel,
        );
    }
    /// A dispenser sheared `entity` of kind `kind` at the block in front of
    /// its own block at `pos`, after the shearing effects. Server-side.
    /// 197.
    pub(super) fn native_block_shear_entity(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        entity: u32,
        kind: &str,
    ) {
        mod_api::dispatch_block_shear_entity(self, mod_api::mods(), dimension, pos, entity, kind);
    }
    /// A dispenser equipped `item` on the first eligible `target` in front
    /// of its block at `pos`. `target` is `player`, `mob` or `armor_stand`.
    /// The equip, item consumption, sound and game event have completed.
    /// Server-side. 189.
    pub(super) fn native_block_dispense_armor(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        item: &str,
        target: &str,
    ) {
        mod_api::dispatch_block_dispense_armor(self, mod_api::mods(), dimension, pos, item, target);
    }
    /// A brewing stand at `pos` began brewing `ingredient`, after fuel and the timer were set. Server-side. 200.
    pub(super) fn native_brewing_start(&self, pos: [i32; 3], ingredient: &str) {
        mod_api::dispatch_brewing_start(self, mod_api::mods(), pos, ingredient);
    }
    /// A brewing stand at `pos` consumed `fuel` and gained brewing fuel. Server-side. 199.
    pub(super) fn native_brewing_fuel(&self, pos: [i32; 3], fuel: &str) {
        mod_api::dispatch_brewing_fuel(self, mod_api::mods(), pos, fuel);
    }
    /// A brewing stand finished brewing at `pos`, after its bottle contents
    /// were changed. Server-side. 198.
    pub(super) fn native_brewing_complete(&self, pos: [i32; 3]) {
        mod_api::dispatch_brewing_complete(self, mod_api::mods(), pos);
    }
    /// An item was placed on a campfire at `pos` and began cooking. 202.
    pub(super) fn native_campfire_start(&self, pos: [i32; 3], item: &str) {
        mod_api::dispatch_campfire_start(self, mod_api::mods(), pos, item);
    }
    /// A block finished cooking `item` into `result` at `pos`. 188.
    pub(super) fn native_block_cook(&self, pos: [i32; 3], item: &str, result: &str) {
        mod_api::dispatch_block_cook(self, mod_api::mods(), pos, item, result);
    }
    /// A compostable item was added to the composter at `pos`. 204.
    pub(super) fn native_compost_item(&self, pos: [i32; 3], item: &str) {
        mod_api::dispatch_compost_item(self, mod_api::mods(), pos, item);
    }
    /// A crafter at `pos` completed recipe `recipe`. 205.
    pub(super) fn native_crafter_craft(&self, pos: [i32; 3], recipe: &str) {
        mod_api::dispatch_crafter_craft(self, mod_api::mods(), pos, recipe);
    }
    /// A server player is about to open an inventory supplied by a block at
    /// `pos`. Set `cancel` to leave the inventory closed. The player's state
    /// is not swapped in. 233.
    pub(super) fn native_inventory_block_start(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        mod_api::dispatch_inventory_block_start(self, mod_api::mods(), player, name, pos, cancel);
    }

    /// A sculk catalyst at `pos` has begun blooming after taking experience.
    /// The catalyst block state has changed. Server-side.
    /// 296.
    pub(super) fn native_sculk_bloom(&mut self, dimension: &str, pos: [i32; 3]) {
        mod_api::dispatch_sculk_bloom(self, mod_api::mods(), dimension, pos);
    }

    /// A projectile hit the target block at `pos`, which now has `strength`
    /// output power. Server-side. 299.
    pub(super) fn native_target_hit(&mut self, dimension: &str, pos: [i32; 3], strength: i32) {
        mod_api::dispatch_target_hit(self, mod_api::mods(), dimension, pos, strength);
    }

    /// A vault at `pos` changed to `state`. Its block state is updated.
    /// Server-side. 303.
    pub(super) fn native_vault_change_state(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: &str,
    ) {
        mod_api::dispatch_vault_change_state(self, mod_api::mods(), dimension, pos, state);
    }

    /// A dispenser at `pos` failed to dispense `item`. Server-side.
    /// 192.
    pub(super) fn native_block_failed_dispense(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        item: &str,
    ) {
        mod_api::dispatch_block_failed_dispense(self, mod_api::mods(), dimension, pos, item);
    }

    /// A beacon at `pos` activated or deactivated after its beam was checked.
    /// `active` says whether it now has a complete pyramid. Server-side.
    /// 184.
    pub(super) fn native_beacon_state_change(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        active: bool,
    ) {
        mod_api::dispatch_beacon_state_change(self, mod_api::mods(), dimension, pos, active);
    }

    /// An anvil at `pos` took damage, changing its block from `from` to `to`.
    /// `to` is air when the anvil broke. Server-side.
    /// 183.
    pub(super) fn native_anvil_damaged(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        from: &str,
        to: &str,
    ) {
        mod_api::dispatch_anvil_damaged(self, mod_api::mods(), dimension, pos, from, to);
    }

    /// A server guest's block-breaking animation changed to `progress` at
    /// `pos` (0 through 9). 186.
    pub(super) fn native_block_break_progress_update(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        progress: i32,
    ) {
        mod_api::dispatch_block_break_progress_update(
            self,
            mod_api::mods(),
            player,
            name,
            pos,
            progress,
        );
    }

    /// A guest's block inventory lock check at `pos`. `locked` is the
    /// vanilla result and may be changed. The guest's state is not swapped in.
    /// 195.
    pub(super) fn native_block_lock_check(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        locked: &mut bool,
    ) {
        mod_api::dispatch_block_lock_check(self, mod_api::mods(), player, name, pos, locked);
    }

    /// A server player reset their attack strength by swinging (`handlePunch`).
    /// The swing and reset have already happened. The player's state is not
    /// swapped in, and helpers looking them up through `self.lan` may find
    /// nothing during a guest packet. Server-side. 250.
    pub(super) fn native_player_attack_cooldown_reset(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_attack_cooldown_reset(self, mod_api::mods(), player, name);
    }
    /// A server player caught a mob in a bucket, after it was removed and the
    /// filled bucket was given. Helpers looking up the guest through
    /// `self.lan` may find nothing during its packet. Server-side.
    /// 253.
    pub(super) fn native_player_bucket_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
        entity: &str,
    ) {
        mod_api::dispatch_player_bucket_entity(
            self,
            mod_api::mods(),
            player,
            name,
            entity_id,
            entity,
        );
    }
    /// A server player changed a beacon's selected effects, after they were
    /// stored. Empty effect ids mean no effect. Helpers looking up the guest
    /// through `self.lan` may find nothing during its packet. Server-side.
    /// 254.
    pub(super) fn native_player_change_beacon_effect(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        primary: &str,
        secondary: &str,
    ) {
        mod_api::dispatch_player_change_beacon_effect(
            self,
            mod_api::mods(),
            player,
            name,
            pos,
            primary,
            secondary,
        );
    }
    /// A server player has slept for 100 ticks in a bed and now counts as
    /// deeply asleep for the night-skip check. Helpers looking the guest up
    /// through `self.lan` may find nothing during its tick. 256.
    pub(super) fn native_player_deep_sleep(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_deep_sleep(self, mod_api::mods(), player, name);
    }
    /// A server player changed their client option for the dominant hand. `left` is true for
    /// left-handed. Their state is not swapped in, and helpers looking them up through `self.lan`
    /// may find nothing during guest packet handling. Server-side.
    /// 255.
    pub(super) fn native_player_change_main_hand(&mut self, player: u128, name: &str, left: bool) {
        mod_api::dispatch_player_change_main_hand(self, mod_api::mods(), player, name, left);
    }
    /// A server player threw an egg, after its projectile was launched and
    /// the use was applied. Helpers looking up the guest through `self.lan`
    /// may find nothing during its packet. Server-side. 258.
    pub(super) fn native_player_egg_throw(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_egg_throw(self, mod_api::mods(), player, name);
    }
    /// A server player used a firework rocket to boost while gliding, after
    /// the rocket was launched and consumed. Helpers looking up the guest
    /// through `self.lan` may find nothing during its packet. Server-side.
    /// 259.
    pub(super) fn native_player_elytra_boost(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_elytra_boost(self, mod_api::mods(), player, name);
    }
    /// A server player put a plant into a flower pot, after the potted block
    /// state was stored and the item consumed. `plant` is the item id and
    /// `pos` the pot position. Helpers looking up the guest through `self.lan`
    /// may find nothing during packet handling. Server-side. 238.
    pub(super) fn native_player_flower_pot_manipulate(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        plant: &str,
    ) {
        mod_api::dispatch_player_flower_pot_manipulate(
            self,
            mod_api::mods(),
            player,
            name,
            pos,
            plant,
        );
    }
    /// A server player inserted a book into a lectern, after the block entity
    /// and inventory changed. `book` is the item id. Helpers looking up a
    /// guest through `self.lan` may find nothing during packet handling.
    /// Server-side. 239.
    pub(super) fn native_player_insert_lectern_book(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        book: &str,
    ) {
        mod_api::dispatch_player_insert_lectern_book(
            self,
            mod_api::mods(),
            player,
            name,
            pos,
            book,
        );
    }
    /// A server player took the book from a lectern, after the menu and block
    /// entity changed. `book` is the removed item id. Helpers looking up a
    /// guest through `self.lan` may find nothing during packet handling.
    /// Server-side. 243.
    pub(super) fn native_player_take_lectern_book(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        book: &str,
    ) {
        mod_api::dispatch_player_take_lectern_book(self, mod_api::mods(), player, name, pos, book);
    }
    /// A server player changed the page of a lectern, after its page and
    /// redstone pulse changed. `page` is the zero-based new page. Helpers
    /// looking up a guest through `self.lan` may find nothing during packet
    /// handling. Server-side. 242.
    pub(super) fn native_player_lectern_page_change(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        page: i32,
    ) {
        mod_api::dispatch_player_lectern_page_change(
            self,
            mod_api::mods(),
            player,
            name,
            pos,
            page,
        );
    }
    /// A server player changed an item frame after its contents or rotation
    /// changed. `action` is `put`, `rotate`, `remove_item` or `break`; `item`
    /// is empty when no item remains, and `rotation` is the resulting step.
    /// Helpers looking up the guest through `self.lan` may find nothing during
    /// packet handling. Server-side. 240.
    pub(super) fn native_player_item_frame_change(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        action: &str,
        item: &str,
        rotation: u8,
    ) {
        mod_api::dispatch_player_item_frame_change(
            self,
            mod_api::mods(),
            player,
            name,
            entity_id,
            action,
            item,
            rotation,
        );
    }
    /// Mending repaired a server player's item with experience from an orb.
    /// `repair` is durability restored and `experience` is the orb's value.
    /// Helpers looking up the guest through `self.lan` may find nothing during
    /// packet handling. Server-side. 241.
    pub(super) fn native_player_item_mend(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        repair: u32,
        experience: u32,
    ) {
        mod_api::dispatch_player_item_mend(
            self,
            mod_api::mods(),
            player,
            name,
            item,
            repair,
            experience,
        );
    }
    /// A server player completed a villager or wandering trader trade.
    /// `entity_id` is the merchant network id. Helpers looking up the guest
    /// through `self.lan` may find nothing during packet handling. Server-side.
    /// 283.
    pub(super) fn native_player_trade(&mut self, player: u128, name: &str, entity_id: u32) {
        mod_api::dispatch_player_trade(self, mod_api::mods(), player, name, entity_id);
    }
    /// A server player traded with a villager or a wandering trader
    /// (NeoForge's `TradeWithVillagerEvent`, at vanilla's
    /// `MerchantResultSlot.onTake` once the offer is taken): `merchant` is
    /// the trader's entity id and `kind` its type (`minecraft:villager`,
    /// `minecraft:wandering_trader`), `offer` the trade as it is after this
    /// one (its `uses` counted, `result` what the player got). Once per
    /// trade, so a shift-click that trades several times is heard for each,
    /// after the trade's statistic and `villager_trade` trigger. Listen
    /// only: the trade is made. The player's state is swapped in
    /// (`native_acting_player`). API 310.
    pub(super) fn native_trade_with_villager(
        &mut self,
        merchant: u32,
        kind: &str,
        offer: &MerchantOffer,
    ) {
        mod_api::dispatch_trade_with_villager(self, mod_api::mods(), merchant, kind, offer);
    }
    /// A server player's shield was disabled by a blocked melee hit. The
    /// shield cooldown is set immediately after this call. Helpers looking up
    /// the guest through `self.lan` may find nothing during packet handling.
    /// Server-side. 276.
    pub(super) fn native_player_shield_disable(&mut self, player: u128, name: &str, ticks: u32) {
        mod_api::dispatch_player_shield_disable(self, mod_api::mods(), player, name, ticks);
    }
    /// A server player selected a loom pattern, after the loom menu accepted
    /// the button. `pattern` is the zero-based button index. Helpers looking
    /// up the guest through `self.lan` may find nothing during packet handling.
    /// Server-side. 265.
    pub(super) fn native_player_loom_pattern_select(
        &mut self,
        player: u128,
        name: &str,
        pattern: i32,
    ) {
        mod_api::dispatch_player_loom_pattern_select(self, mod_api::mods(), player, name, pattern);
    }
    /// A server player named a mob with a name tag, after its name and
    /// persistence changed and the tag was consumed. Helpers looking up the
    /// guest through `self.lan` may find nothing during packet handling.
    /// Server-side. 267.
    pub(super) fn native_player_name_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        entity_name: &str,
    ) {
        mod_api::dispatch_player_name_entity(
            self,
            mod_api::mods(),
            player,
            name,
            entity_id,
            entity_name,
        );
    }
    /// A server player opened a sign for editing, after the editor state was
    /// made. `front` selects the front or back side. Guest lookup helpers may
    /// find nothing during packet handling. Server-side.
    /// 268.
    pub(super) fn native_player_open_sign(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        front: bool,
    ) {
        mod_api::dispatch_player_open_sign(self, mod_api::mods(), player, name, pos, front);
    }
    /// A server player picked up an arrow after it was removed and its pickup
    /// was applied. `item` is the arrow's pickup item id. Guest lookup helpers
    /// may find nothing during packet handling. Server-side.
    /// 270.
    pub(super) fn native_player_pickup_arrow(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        item: &str,
    ) {
        mod_api::dispatch_player_pickup_arrow(self, mod_api::mods(), player, name, entity_id, item);
    }
    /// A server player selected a stonecutter recipe, after the menu accepted
    /// its button. `recipe` is the zero-based button index. Guest lookup
    /// helpers may find nothing during packet handling. Server-side.
    /// 278.
    pub(super) fn native_player_stonecutter_recipe_select(
        &mut self,
        player: u128,
        name: &str,
        recipe: i32,
    ) {
        mod_api::dispatch_player_stonecutter_recipe_select(
            self,
            mod_api::mods(),
            player,
            name,
            recipe,
        );
    }
    /// A server player stopped spectating `entity_id`, after their camera
    /// returned to the player. Guest lookup helpers may find nothing.
    /// 279.
    pub(super) fn native_player_stop_spectating_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: i32,
    ) {
        mod_api::dispatch_player_stop_spectating_entity(
            self,
            mod_api::mods(),
            player,
            name,
            entity_id,
        );
    }
    /// A server applied `velocity` to a player, when syncing changed motion to
    /// that player's client. Guest lookup helpers may find nothing.
    /// 285.
    pub(super) fn native_player_velocity(&mut self, player: u128, name: &str, velocity: [f64; 3]) {
        mod_api::dispatch_player_velocity(self, mod_api::mods(), player, name, velocity);
    }
    /// A server player removed an entity's leash, after it was cleared.
    /// Guest lookup helpers may find nothing during packet handling.
    /// 284.
    pub(super) fn native_player_unleash_entity(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
    ) {
        mod_api::dispatch_player_unleash_entity(self, mod_api::mods(), player, name, entity_id);
    }
    /// A server player toggled an ageable mob's age lock with a golden
    /// dandelion, after the lock and item use changed. 282.
    pub(super) fn native_player_toggle_entity_age_lock(
        &mut self,
        player: u128,
        name: &str,
        entity_id: u32,
        locked: bool,
    ) {
        mod_api::dispatch_player_toggle_entity_age_lock(
            self,
            mod_api::mods(),
            player,
            name,
            entity_id,
            locked,
        );
    }
    /// A server player swapped their main and off hands, after both inventory
    /// slots changed. Guest lookup helpers may find nothing during packet
    /// handling. Server-side. 280.
    pub(super) fn native_player_swap_with_equipment_slot(
        &mut self,
        player: u128,
        name: &str,
        slot: &str,
    ) {
        mod_api::dispatch_player_swap_with_equipment_slot(
            self,
            mod_api::mods(),
            player,
            name,
            slot,
        );
    }
    /// A server player filled an empty map, after the filled map and its saved
    /// data were created. Guest lookup helpers may find nothing during packet
    /// handling. Server-side. 266.
    pub(super) fn native_player_map_filled(&mut self, player: u128, name: &str, map_id: i32) {
        mod_api::dispatch_player_map_filled(self, mod_api::mods(), player, name, map_id);
    }
    /// A server player is about to travel through an End gateway at `gateway`.
    /// Set `cancel` to leave them at the entrance; the gateway is not consumed
    /// and its cooldown does not start. Helpers may find no guest through
    /// `self.lan` during its packet. Server-side. 281.
    pub(super) fn native_player_teleport_end_gateway(
        &mut self,
        player: u128,
        name: &str,
        gateway: [i32; 3],
        cancel: &mut bool,
    ) {
        mod_api::dispatch_player_teleport_end_gateway(
            self,
            mod_api::mods(),
            player,
            name,
            gateway,
            cancel,
        );
    }
    /// A server player's recipe book requested a known recipe be placed in
    /// their crafting grid. `recipe` is the recipe id; `all` is the shift
    /// click form. The player's state may not be available through `self.lan`
    /// while a guest packet is handled. Server-side. 272.
    pub(super) fn native_player_recipe_book_click(
        &mut self,
        player: u128,
        name: &str,
        recipe: &str,
        all: bool,
    ) {
        mod_api::dispatch_player_recipe_book_click(
            self,
            mod_api::mods(),
            player,
            name,
            recipe,
            all,
        );
    }
    /// A server player's XP orb pickup cooldown changed to `ticks`. It is
    /// reported after the value changes; the player's state may not be found
    /// through `self.lan` while a guest packet is handled. Server-side.
    /// 261.
    pub(super) fn native_player_exp_cooldown_change(
        &mut self,
        player: u128,
        name: &str,
        ticks: u32,
    ) {
        mod_api::dispatch_player_exp_cooldown_change(self, mod_api::mods(), player, name, ticks);
    }
    /// A server player changed their client language option to `locale`.
    /// The player's state may not be found through `self.lan` during a guest
    /// packet. Server-side. 264.
    pub(super) fn native_player_locale_change(&mut self, player: u128, name: &str, locale: &str) {
        mod_api::dispatch_player_locale_change(self, mod_api::mods(), player, name, locale);
    }
    /// A server entity was moved by a same-level teleport command, after its
    /// position changed. `entity` is its network id; `from` and `to` are
    /// coordinates in `dimension`. This hook reports command teleports that
    /// use Horizon's same-level target path; it does not change vanilla's
    /// movement. 221.
    pub(super) fn native_entity_teleport(
        &mut self,
        dimension: &str,
        entity: u32,
        from: [f64; 3],
        to: [f64; 3],
    ) {
        mod_api::dispatch_entity_teleport(self, mod_api::mods(), dimension, entity, from, to);
    }

    /// A server entity finished its portal wait and is beginning to leave
    /// `dimension` through `pos`. The destination follows the portal rules.
    /// 217.
    pub(super) fn native_entity_portal(&mut self, dimension: &str, entity: u32, pos: [i32; 3]) {
        mod_api::dispatch_entity_portal(self, mod_api::mods(), dimension, entity, pos);
    }

    /// A server projectile was added to the level. `player` is set only for
    /// a player launch; other sources have an empty `name`. 293.
    pub(super) fn native_projectile_launch(
        &mut self,
        player: Option<u128>,
        name: &str,
        entity: u32,
        kind: &str,
    ) {
        mod_api::dispatch_projectile_launch(self, mod_api::mods(), player, name, entity, kind);
    }

    /// A player bow or crossbow launched an arrow with `force`, after the
    /// projectile entered the level. 219.
    pub(super) fn native_entity_shoot_bow(
        &mut self,
        player: u128,
        name: &str,
        projectile: u32,
        force: f32,
    ) {
        mod_api::dispatch_entity_shoot_bow(self, mod_api::mods(), player, name, projectile, force);
    }

    /// A player is about to nock `bow` with `hand` (`BowItem.use`, NeoForge's
    /// `ArrowNockEvent`): `has_ammo` is whether `getProjectile` found ammunition
    /// or the player is in creative. Set `cancel` to fail the use.
    /// @API:arrow_nock@.
    pub(super) fn native_arrow_nock(
        &mut self,
        player: u128,
        name: &str,
        bow: &ItemStack,
        hand: Hand,
        has_ammo: bool,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_arrow_nock(
            self,
            mod_api::mods(),
            player,
            name,
            bow,
            hand,
            has_ammo,
            cancel,
        );
    }

    /// `LivingEntity.getProjectile(weapon)` (NeoForge's
    /// `LivingGetProjectileEvent`) for a player (`player` and `name`) or a
    /// mob (`entity`, `kind`): a skeleton or illusioner's shot, or the crossbow
    /// load of a pillager or piglin. `projectile` is the vanilla
    /// ammunition; replace it to choose another. A replaced stack is not taken
    /// from the inventory. @API:living_get_projectile@.
    pub(super) fn native_living_get_projectile(
        &mut self,
        player: Option<u128>,
        name: &str,
        entity: Option<u32>,
        kind: &str,
        weapon: &ItemStack,
        projectile: &mut Option<ItemStack>,
    ) {
        mod_api::dispatch_living_get_projectile(
            self,
            mod_api::mods(),
            player,
            name,
            entity,
            kind,
            weapon,
            projectile,
        );
    }

    /// A guest's fall flying state changed on the server. The change has
    /// already been applied. 222.
    pub(super) fn native_entity_toggle_glide(&mut self, player: u128, name: &str, gliding: bool) {
        mod_api::dispatch_entity_toggle_glide(self, mod_api::mods(), player, name, gliding);
    }

    /// A guest player's synchronized pose changed from `from` to `to`.
    /// 218.
    pub(super) fn native_entity_pose_change(
        &mut self,
        player: u128,
        name: &str,
        from: &str,
        to: &str,
    ) {
        mod_api::dispatch_entity_pose_change(self, mod_api::mods(), player, name, from, to);
    }

    /// A dropped item entity was successfully added to the server level.
    /// 234.
    pub(super) fn native_item_spawn(&mut self, entity: u32, item: &str, count: i32, pos: [f64; 3]) {
        mod_api::dispatch_item_spawn(self, mod_api::mods(), entity, item, count, pos);
    }

    /// A mob became tamed after its owner and persistent state were set.
    /// 220.
    pub(super) fn native_entity_tame(&mut self, entity: u32, kind: &str, owner: Option<u128>) {
        mod_api::dispatch_entity_tame(self, mod_api::mods(), entity, kind, owner);
    }

    /// Two mobs completed a breeding action. `child` is absent when vanilla
    /// lays an egg instead of creating a baby. 213.
    pub(super) fn native_entity_breed(&mut self, parent: u32, partner: u32, child: Option<u32>) {
        mod_api::dispatch_entity_breed(self, mod_api::mods(), parent, partner, child);
    }

    /// A furnace at `pos` began cooking `item` (its cooking timer advanced
    /// from zero). Server-side.
    /// 230.
    pub(super) fn native_furnace_start_smelt(&self, pos: [i32; 3], item: &str) {
        mod_api::dispatch_furnace_start_smelt(self, mod_api::mods(), pos, item);
    }
    /// A furnace at `pos` consumed fuel and began or resumed burning it.
    /// `fuel` is the item consumed from its fuel slot. Server-side.
    /// 227.
    pub(super) fn native_furnace_burn(&self, pos: [i32; 3], fuel: &str) {
        mod_api::dispatch_furnace_burn(self, mod_api::mods(), pos, fuel);
    }
    /// A furnace finished smelting `item` into `result` at `pos`. Server-side.
    /// 229.
    pub(super) fn native_furnace_smelt(&self, pos: [i32; 3], item: &str, result: &str) {
        mod_api::dispatch_furnace_smelt(self, mod_api::mods(), pos, item, result);
    }
    /// A server player took an output from a furnace at `pos`; `item` is the
    /// output item id and `count` is how many were taken. The player's state is
    /// not swapped in, so helpers that look them up through `self.lan` may find
    /// nothing during guest packet handling. 228.
    pub(super) fn native_furnace_extract(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        item: &str,
        count: u32,
    ) {
        mod_api::dispatch_furnace_extract(self, mod_api::mods(), player, name, pos, item, count);
    }
    /// A server player applied an enchantment at `pos`; `item` is the finished item and `levels` the levels charged. The player's state is not swapped in. 209.
    pub(super) fn native_enchant_item(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        item: &ItemStack,
        levels: i32,
    ) {
        mod_api::dispatch_enchant_item(self, mod_api::mods(), player, name, pos, item, levels);
    }
    /// Enchanting offers were calculated for a server player at `pos`. Change `costs` before they are sent to the menu. 291.
    pub(super) fn native_prepare_item_enchant(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        costs: &mut [i32; 3],
    ) {
        mod_api::dispatch_prepare_item_enchant(self, mod_api::mods(), player, name, pos, costs);
    }
    /// An anvil result was prepared for a server player at `pos`. Change `result` or its level `cost` before it is sent. 288.
    pub(super) fn native_prepare_anvil(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        result: &mut Option<ItemStack>,
        cost: &mut i32,
    ) {
        mod_api::dispatch_prepare_anvil(self, mod_api::mods(), player, name, pos, cost, result);
    }
    /// A player took an anvil's result at `pos` (`AnvilCraftEvent.Pre`), before the levels are charged, the inputs used up and the anvil worn. `craft` holds the inputs and the result as they were before. Set `cancel` to refuse the take: the result stays in its slot and the inputs, levels, inventory and cursor are as before the click. 328.
    pub(super) fn native_anvil_craft_pre(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        craft: &horizon_client::menu::AnvilCraft,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_anvil_craft_pre(self, mod_api::mods(), player, name, pos, craft, cancel);
    }
    /// A player took an anvil's result at `pos` and the levels, inputs and anvil wear are done (`AnvilCraftEvent.Post`); a broken anvil is already air. `craft` holds the inputs and the result as they were before. Not called when `native_anvil_craft_pre` cancelled. 327.
    pub(super) fn native_anvil_craft_post(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        craft: &horizon_client::menu::AnvilCraft,
    ) {
        mod_api::dispatch_anvil_craft_post(self, mod_api::mods(), player, name, pos, craft);
    }
    /// A grindstone result was prepared for a server player at `pos`. Change it before it is sent. 289.
    pub(super) fn native_prepare_grindstone(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        result: &mut Option<ItemStack>,
    ) {
        mod_api::dispatch_prepare_grindstone(self, mod_api::mods(), player, name, pos, result);
    }
    /// A smithing result was prepared for a server player at `pos`. Change it before it is sent. 292.
    pub(super) fn native_prepare_smithing(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        result: &mut Option<ItemStack>,
    ) {
        mod_api::dispatch_prepare_smithing(self, mod_api::mods(), player, name, pos, result);
    }
    /// A crafting-grid result was prepared for a server player; `pos` is the crafting table, or None for the inventory grid. Change `result` before it is sent. 290.
    pub(super) fn native_prepare_item_craft(
        &mut self,
        player: u128,
        name: &str,
        pos: Option<[i32; 3]>,
        result: &mut Option<ItemStack>,
    ) {
        mod_api::dispatch_prepare_item_craft(self, mod_api::mods(), player, name, pos, result);
    }
    /// A server player selected trade `index` from `villager` in the trading menu. 302.
    pub(super) fn native_trade_select(
        &mut self,
        player: u128,
        name: &str,
        villager: Option<u32>,
        index: usize,
    ) {
        mod_api::dispatch_trade_select(self, mod_api::mods(), player, name, villager, index);
    }
    /// The server changed its weather parameters. `raining` and `thundering`
    /// are the requested final states. Set `cancel` to keep the previous
    /// weather parameters. Server-side. 304.
    pub(super) fn native_weather_change(
        &mut self,
        dimension: &str,
        raining: bool,
        thundering: bool,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_weather_change(
            self,
            mod_api::mods(),
            dimension,
            raining,
            thundering,
            cancel,
        );
    }

    /// The server changed its thunder state through `/weather`. Set `cancel`
    /// to keep the previous thunder state and timer. Server-side.
    /// 300.
    pub(super) fn native_thunder_change(
        &mut self,
        dimension: &str,
        thundering: bool,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_thunder_change(self, mod_api::mods(), dimension, thundering, cancel);
    }

    /// A server lightning bolt is about to be created. `cause` is `weather`,
    /// `trident` or `command`; `player` is the responsible player when known.
    /// Set `cancel` to prevent the bolt and its effects. 236.
    pub(super) fn native_lightning_strike(
        &mut self,
        player: Option<u128>,
        pos: [i32; 3],
        cause: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_lightning_strike(self, mod_api::mods(), player, pos, cause, cancel);
    }

    /// Enough players have slept through the night in level `dimension`
    /// and the clock is about to move to the morning (NeoForge's
    /// `SleepFinishedTimeEvent`, vanilla's `ServerLevel.tick` when it moves
    /// the clock to `wake_up_from_sleep`): `new_time` comes in as the next
    /// morning's day time and may be changed to wake everyone at another
    /// time. A time before `current` (the day time now) is refused, as
    /// `setTimeAddition` refuses one, and the morning stays. Unlike
    /// NeoForge, which refuses a bad value as it is set, the check runs once
    /// after every mod, so a later mod sees the value an earlier one left.
    /// On the server, only when the `advance_time` rule is on;
    /// `native_time_skip` then hears the jump to the time chosen here and
    /// may still cancel it. The players wake and the weather resets as
    /// usual either way.
    /// API 308.
    pub(super) fn native_sleep_finished_time(
        &mut self,
        dimension: &str,
        current: i64,
        new_time: &mut i64,
    ) {
        mod_api::dispatch_sleep_finished_time(self, mod_api::mods(), dimension, current, new_time);
    }

    /// The overworld clock is about to jump, from sleep or `/time`. Set
    /// `cancel` to keep `from`. 301.
    pub(super) fn native_time_skip(
        &mut self,
        dimension: &str,
        from: i64,
        to: i64,
        cause: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_time_skip(self, mod_api::mods(), dimension, from, to, cause, cancel);
    }

    /// The world's spawn point changed to `pos` and `angle`. 244.
    pub(super) fn native_spawn_change(&mut self, pos: [i32; 3], angle: [f32; 2]) {
        mod_api::dispatch_spawn_change(self, mod_api::mods(), pos, angle);
    }

    /// A server game rule changed to its final `value`. 247.
    pub(super) fn native_world_game_rule_change(
        &mut self,
        player: u128,
        name: &str,
        rule: &str,
        value: &str,
    ) {
        mod_api::dispatch_world_game_rule_change(self, mod_api::mods(), player, name, rule, value);
    }

    /// The server world's border bounds changed, either immediately or when
    /// its timed size change was requested. 245.
    pub(super) fn native_world_border_bounds_changed(&mut self) {
        mod_api::dispatch_world_border_bounds_changed(self, mod_api::mods());
    }

    /// The server world's border center changed. 246.
    pub(super) fn native_world_border_center_changed(&mut self) {
        mod_api::dispatch_world_border_center_changed(self, mod_api::mods());
    }

    /// A Nether portal was built for a dimension travel, or an End platform
    /// was made for an arrival. `pos` is the lower corner of the Nether
    /// portal rectangle or the End platform origin. The completed blocks are
    /// already in the world. 286.
    pub(super) fn native_portal_create(&mut self, dimension: &str, pos: [i32; 3]) {
        mod_api::dispatch_portal_create(self, mod_api::mods(), dimension, pos);
    }

    /// A raid wave has been spawned at `pos`; `wave` is its one-based wave
    /// number and `raiders` counts the raiders, not the mobs they ride. 294.
    pub(super) fn native_raid_spawn_wave(
        &mut self,
        raid: i32,
        pos: [i32; 3],
        wave: i32,
        raiders: usize,
    ) {
        mod_api::dispatch_raid_spawn_wave(self, mod_api::mods(), raid, pos, wave, raiders);
    }

    /// A raid at `pos` changed to `state` (`triggered`, `victory`, `loss` or
    /// `stopped`). 295.
    pub(super) fn native_raid_state_change(&mut self, raid: i32, pos: [i32; 3], state: &str) {
        mod_api::dispatch_raid_state_change(self, mod_api::mods(), raid, pos, state);
    }

    /// The server completed a world save cycle. `wait` says chunk writes were
    /// flushed before returning. 306.
    pub(super) fn native_world_saved(&mut self, wait: bool) {
        mod_api::dispatch_world_saved(self, mod_api::mods(), wait);
    }

    /// Entities from `chunk` finished loading into the server world. `count`
    /// is the number of saved entity records read. 210.
    pub(super) fn native_entities_load(&mut self, dimension: &str, chunk: [i32; 2], count: usize) {
        mod_api::dispatch_entities_load(self, mod_api::mods(), dimension, chunk, count);
    }

    /// Entities in `chunk` left the active server world and were queued for
    /// saving. `count` is the number of entity records. 211.
    pub(super) fn native_entities_unload(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
        count: usize,
    ) {
        mod_api::dispatch_entities_unload(self, mod_api::mods(), dimension, chunk, count);
    }

    /// A generic world game event was fired at `pos`. 231.
    pub(super) fn native_game_event(&mut self, event: &str, pos: [f64; 3]) {
        mod_api::dispatch_game_event(self, mod_api::mods(), event, pos);
    }

    /// A guest asked to complete `command`; `suggestions` may be added to or
    /// filtered. `range` is the replaced byte range. 298.
    pub(super) fn native_tab_complete(
        &mut self,
        player: u128,
        name: &str,
        command: &str,
        range: [usize; 2],
        suggestions: &mut Vec<Suggestion>,
    ) {
        mod_api::dispatch_tab_complete(
            self,
            mod_api::mods(),
            player,
            name,
            command,
            range,
            suggestions,
        );
    }

    /// A server broadcast line is about to be logged and sent. Change `line`
    /// or set `cancel` to drop it. 201.
    pub(super) fn native_broadcast_message(&self, line: &mut Text, cancel: &mut bool) {
        mod_api::dispatch_broadcast_message(self, mod_api::mods(), line, cancel);
    }

    /// The dedicated server's whitelist changed to `enabled`. 305.
    pub(super) fn native_whitelist_state_update(&mut self, enabled: bool) {
        mod_api::dispatch_whitelist_state_update(self, mod_api::mods(), enabled);
    }

    /// A mob entered love mode after player interaction. `player` is the
    /// feeder when known. 216.
    pub(super) fn native_entity_enter_love_mode(
        &mut self,
        entity: u32,
        kind: &str,
        player: Option<u128>,
    ) {
        mod_api::dispatch_entity_enter_love_mode(self, mod_api::mods(), entity, kind, player);
    }

    /// A mob conversion completed; both ids and entity types are reported.
    /// 223.
    pub(super) fn native_entity_transform(
        &mut self,
        entity: u32,
        kind: &str,
        transformed: u32,
        transformed_kind: &str,
    ) {
        mod_api::dispatch_entity_transform(
            self,
            mod_api::mods(),
            entity,
            kind,
            transformed,
            transformed_kind,
        );
    }

    /// A thrown splash or lingering potion applied its hit effects.
    /// 287.
    pub(super) fn native_potion_splash(&mut self, item: &str, pos: [f64; 3]) {
        mod_api::dispatch_potion_splash(self, mod_api::mods(), item, pos);
    }

    /// A firework rocket completed its explosion effects. 225.
    pub(super) fn native_firework_explode(&mut self, entity: u32, pos: [f64; 3], bursts: usize) {
        mod_api::dispatch_firework_explode(self, mod_api::mods(), entity, pos, bursts);
    }

    /// An experience bottle broke and its experience was awarded.
    /// 224.
    pub(super) fn native_exp_bottle(&mut self, pos: [f64; 3], experience: u32) {
        mod_api::dispatch_exp_bottle(self, mod_api::mods(), pos, experience);
    }

    /// A player ignited a creeper with an item. 206.
    pub(super) fn native_creeper_ignite(&mut self, entity: u32, player: u128, item: &str) {
        mod_api::dispatch_creeper_ignite(self, mod_api::mods(), entity, player, item);
    }

    /// Lightning charged a creeper. 207.
    pub(super) fn native_creeper_power(&mut self, entity: u32, lightning: [f64; 3]) {
        mod_api::dispatch_creeper_power(self, mod_api::mods(), entity, lightning);
    }

    /// A mob broke the door at `pos`. 212.
    pub(super) fn native_entity_break_door(&mut self, entity: u32, kind: &str, pos: [i32; 3]) {
        mod_api::dispatch_entity_break_door(self, mod_api::mods(), entity, kind, pos);
    }

    /// A mob changed a block at `pos`, after the new state was applied.
    /// 214.
    pub(super) fn native_entity_change_block(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        from: &str,
        to: &str,
    ) {
        mod_api::dispatch_entity_change_block(self, mod_api::mods(), entity, kind, pos, from, to);
    }

    /// A bee entered the hive at `pos` and was stored there.
    /// 215.
    pub(super) fn native_entity_enter_block(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        block: &str,
    ) {
        mod_api::dispatch_entity_enter_block(self, mod_api::mods(), entity, kind, pos, block);
    }

    /// A server player finished loading the world around them after joining,
    /// respawning or changing dimension. The guest's state is not swapped in,
    /// so helpers that look them up through `self.lan` may find nothing while
    /// a guest packet is handled. Server-side. 342.
    pub(super) fn native_player_client_loaded_world(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_client_loaded_world(self, mod_api::mods(), player, name);
    }
    /// A server player released the use-item key before the item finished
    /// being used. The guest's state is not swapped in, so helpers that look
    /// them up through `self.lan` may find nothing during packet handling.
    /// 350.
    pub(super) fn native_player_stop_using_item(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_stop_using_item(self, mod_api::mods(), player, name);
    }
    /// A server player's recipe book open or filter setting changed. `book`
    /// is the book type's protocol id. The guest's state is not swapped in.
    /// 349.
    pub(super) fn native_player_recipe_book_settings_change(
        &mut self,
        player: u128,
        name: &str,
        book: i32,
        open: bool,
        filtering: bool,
    ) {
        mod_api::dispatch_player_recipe_book_settings_change(
            self,
            mod_api::mods(),
            player,
            name,
            book,
            open,
            filtering,
        );
    }
    /// A server player earned one criterion of an advancement, after the
    /// progress was recorded. 339.
    pub(super) fn native_player_advancement_criterion_grant(
        &mut self,
        player: u128,
        name: &str,
        advancement: &str,
        criterion: &str,
    ) {
        mod_api::dispatch_player_advancement_criterion_grant(
            self,
            mod_api::mods(),
            player,
            name,
            advancement,
            criterion,
        );
    }
    /// A server player released a drawn bow with `arrow` selected. The guest's
    /// state is not swapped in during packet handling. 348.
    pub(super) fn native_player_ready_arrow(&mut self, player: u128, name: &str, arrow: &str) {
        mod_api::dispatch_player_ready_arrow(self, mod_api::mods(), player, name, arrow);
    }
    /// A server player selected a bundle item in `slot` at `index`. The guest's
    /// state is not swapped in during packet handling. 341.
    pub(super) fn native_player_bundle_item_select(
        &mut self,
        player: u128,
        name: &str,
        slot: i32,
        index: i32,
    ) {
        mod_api::dispatch_player_bundle_item_select(
            self,
            mod_api::mods(),
            player,
            name,
            slot,
            index,
        );
    }
    /// A server player stopped digging `pos` before the block broke, after
    /// the server cleared the dig and sent the block back to the client.
    /// The player's state is not swapped in. 329.
    pub(super) fn native_block_damage_abort(&mut self, player: u128, name: &str, pos: [i32; 3]) {
        mod_api::dispatch_block_damage_abort(self, mod_api::mods(), player, name, pos);
    }
    /// A hopper at `pos` began an item transfer attempt. This runs after its
    /// cooldown and enabled checks, before pushing or pulling any items.
    /// Server-side, including the single-player server. 352.
    pub(super) fn native_prepare_hopper_transfer(&self, pos: [i32; 3]) {
        mod_api::dispatch_prepare_hopper_transfer(self, mod_api::mods(), pos);
    }
    /// A hopper is about to move one item between containers, a composter's
    /// bone meal included. Set `cancel` to leave it in its source slot. A chest
    /// boat or minecart is reported at the block it is in. 334.
    pub(super) fn native_inventory_move_item(
        &self,
        source: [i32; 3],
        destination: [i32; 3],
        item: &str,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_inventory_move_item(
            self,
            mod_api::mods(),
            source,
            destination,
            item,
            cancel,
        );
    }
    /// A hopper selected a container while searching for items to push or
    /// pull. Exactly one of `container` and `entity` identifies the target.
    /// 333.
    pub(super) fn native_hopper_inventory_search(
        &self,
        hopper: [i32; 3],
        container: Option<[i32; 3]>,
        entity: Option<u32>,
    ) {
        mod_api::dispatch_hopper_inventory_search(self, mod_api::mods(), hopper, container, entity);
    }
    /// A vault at `pos` is about to eject a loot item. 353.
    pub(super) fn native_vault_loot(&self, pos: [i32; 3], item: &str) {
        mod_api::dispatch_vault_loot(self, mod_api::mods(), pos, item);
    }
    /// A timed world-border size change finished on the server, after its
    /// final size was applied. Server-side, including the single-player
    /// server. 354.
    pub(super) fn native_world_border_bounds_change_finish(&mut self) {
        mod_api::dispatch_world_border_bounds_change_finish(self, mod_api::mods());
    }

    /// A server player launched a projectile item, after it was created.
    /// `item` is the projectile's held item. 346.
    pub(super) fn native_player_launch_projectile(&mut self, player: u128, name: &str, item: &str) {
        mod_api::dispatch_player_launch_projectile(self, mod_api::mods(), player, name, item);
    }
    /// A server explosion is about to begin at `center`, with `radius` (Bukkit's
    /// `ExplosionPrimeEvent`). This is listen-only: the explosion proceeds as
    /// usual. Server-side, including the single-player server. 332.
    pub(super) fn native_explosion_prime(&mut self, center: [f64; 3], radius: f32) {
        mod_api::dispatch_explosion_prime(self, mod_api::mods(), center, radius);
    }
    /// A dropped item entity reached its 6000-tick lifetime and is about to be
    /// removed (Bukkit's `ItemDespawnEvent`). 335.
    pub(super) fn native_item_despawn(
        &mut self,
        entity: u32,
        item: &str,
        count: i32,
        pos: [f64; 3],
    ) {
        mod_api::dispatch_item_despawn(self, mod_api::mods(), entity, item, count, pos);
    }
    /// A mob is about to drop `count` of `item` at `pos` (Bukkit's
    /// `EntityDropItemEvent`). 330.
    pub(super) fn native_entity_drop_item(
        &mut self,
        entity: u32,
        item: &str,
        count: i32,
        pos: [f64; 3],
    ) {
        mod_api::dispatch_entity_drop_item(self, mod_api::mods(), entity, item, count, pos);
    }
    /// A leashed mob's leash was removed, before any lead drop is handled
    /// (Bukkit's `EntityUnleashEvent`). 331.
    pub(super) fn native_entity_unleash(&mut self, entity: u32, kind: &str) {
        mod_api::dispatch_entity_unleash(self, mod_api::mods(), entity, kind);
    }
    /// A vehicle is about to take damage from a player's attack. `kind` is
    /// `boat` or the minecart type; mods may change `damage` or set `cancel`
    /// to leave its damage and wobble unchanged. The attack itself still
    /// completes. Server-side. 337.
    pub(super) fn native_vehicle_damage(
        &mut self,
        kind: &str,
        damage: &mut f32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_vehicle_damage(self, mod_api::mods(), kind, damage, cancel);
    }
    /// A boat or minecart was created at `pos`, after it joined the level.
    /// `kind` is the vehicle type. Server-side. 336.
    pub(super) fn native_vehicle_create(&mut self, kind: &str, pos: [f64; 3]) {
        mod_api::dispatch_vehicle_create(self, mod_api::mods(), kind, pos);
    }
    /// A boat or minecart is about to be destroyed at `pos`. TNT minecarts
    /// that prime instead of breaking are not heard. Server-side.
    /// 338.
    pub(super) fn native_vehicle_destroy(&mut self, kind: &str, pos: [f64; 3]) {
        mod_api::dispatch_vehicle_destroy(self, mod_api::mods(), kind, pos);
    }
    /// A guest sent the server's current `PlayerInput` flags, after they were applied to the hub. The guest's state is swapped in. Server-side. 343.
    pub(super) fn native_player_input(&mut self, player: u128, name: &str, flags: i32) {
        mod_api::dispatch_player_input(self, mod_api::mods(), player, name, flags);
    }
    /// A guest's arm swing was handled by the server. `hand` is `mainhand`. Server-side. 340.
    pub(super) fn native_player_arm_swing(&mut self, player: u128, name: &str, hand: &str) {
        mod_api::dispatch_player_arm_swing(self, mod_api::mods(), player, name, hand);
    }
    /// A guest's accepted movement made the server jump them from the ground. Server-side. 344.
    pub(super) fn native_player_jump(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_jump(self, mod_api::mods(), player, name);
    }
    /// A guest was disconnected by a server kick, after the hub recorded the rendered `reason`. The guest's state is
    /// swapped in unless they were already removed in the same poll. 345.
    pub(super) fn native_player_kicked(&mut self, player: u128, name: &str, reason: &str) {
        mod_api::dispatch_player_kicked(self, mod_api::mods(), player, name, reason);
    }
    /// A server player's experience level changed, after the new level was applied. 347.
    pub(super) fn native_player_level_change(
        &mut self,
        player: u128,
        name: &str,
        from: i32,
        to: i32,
    ) {
        mod_api::dispatch_player_level_change(self, mod_api::mods(), player, name, from, to);
    }
    /// A click in a menu a mod opened (`native_open_menu`), which moved
    /// nothing: the menu's `slot` (its own first, then the player's
    /// inventory, `native_menu_inventory_slot`; -999 outside), the `button`
    /// and the click's `mode` (`ClickType`'s id) of the player `uuid`.
    /// No player's state is swapped in: the menu helpers take the player's
    /// UUID. API 10.
    pub(super) fn native_menu_clicked(
        &mut self,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    ) {
        mod_api::dispatch_menu_clicked(self, mod_api::mods(), uuid, window, slot, button, mode);
    }
    /// A menu a mod opened closed: the player closed it, the server or a mod
    /// opened another, or the player left. API 10.
    pub(super) fn native_menu_closed(&mut self, uuid: u128, window: i32) {
        mod_api::dispatch_menu_closed(self, mod_api::mods(), uuid, window);
    }
    /// An item's tooltip, wherever the game shows one (slots, the creative
    /// tabs, the recipe book, chat's hover): the stack, with its components
    /// (`stack.data()`: a shulker box's contents...), and its lines, the name
    /// first, each with a colour as `rgba` packs it (`u32::MAX` keeps the
    /// text's own). Add, change or take out lines; `Text::legacy` reads `§`
    /// codes. Asked each frame the tooltip shows, never for one the stack
    /// hides. API 12.
    pub(super) fn native_tooltip(&self, stack: &ItemStack, lines: &mut Vec<(Text, u32)>) {
        mod_api::dispatch_tooltip(self, mod_api::mods(), stack, lines);
    }
    /// Change a tooltip box's background and border gradient colors, as ARGB.
    /// Stack is None for tooltips not associated with an item.
    pub(super) fn native_tooltip_color(
        &self,
        stack: Option<&ItemStack>,
        background: &mut u32,
        border_top: &mut u32,
        border_bottom: &mut u32,
    ) {
        mod_api::dispatch_tooltip_color(
            self,
            mod_api::mods(),
            stack,
            background,
            border_top,
            border_bottom,
        );
    }
    /// Client-side image component for this stack's tooltip. Answer its width/height
    /// in GUI pixels; finite positive dimensions reserve space after text and any
    /// vanilla bundle component. None keeps the vanilla tooltip unchanged.
    pub(super) fn native_tooltip_component(&self, stack: &ItemStack, size: &mut Option<[f32; 2]>) {
        mod_api::dispatch_tooltip_component(self, mod_api::mods(), stack, size);
    }
    /// Render the measured component at origin, inside the tooltip background.
    /// Stack is the component's data. Use native_sprite/native_fill/native_text;
    /// size is the measurement returned by native_tooltip_component this frame.
    pub(super) fn native_tooltip_component_draw(
        &self,
        stack: &ItemStack,
        batch: &mut HudBatch,
        origin: [f32; 2],
        size: [f32; 2],
    ) {
        mod_api::dispatch_tooltip_component_draw(self, mod_api::mods(), stack, batch, origin, size);
    }

    /// The small bar `GuiGraphicsExtractor.itemDecorations` draws under an
    /// item, wherever the game shows one: `bar` arrives holding vanilla's value
    /// (the damage bar, or a bundle's fullness already applied) and may be
    /// replaced (width `0..=13`, colour `rgb`) or set to `None` to hide it.
    /// Asked each frame a stack is drawn. Client-side. 60.
    pub(super) fn native_item_bar(&self, stack: &ItemStack, bar: &mut Option<(i32, u32)>) {
        mod_api::dispatch_item_bar(self, mod_api::mods(), stack, bar);
    }
    /// Append an item's custom decorations after its default bar, cooldown and count.
    /// Coordinates are the icon's top-left in GUI units, as the batch takes them;
    /// no item translation is active.
    /// Runs wherever the shared item-decoration path is used (hotbar, menus, tooltips,
    /// and the other screens that draw item icons: advancements, beacon, stats, trading).
    pub(super) fn native_item_decorations(
        &self,
        stack: &ItemStack,
        x: f32,
        y: f32,
        batch: &mut HudBatch,
    ) {
        mod_api::dispatch_item_decorations(self, mod_api::mods(), stack, x, y, batch);
    }
    /// A player uses `stack` (right click) with `self.hand` on the block at
    /// `pos`, `face` the face clicked (`[0, 1, 0]` is the top), once the
    /// block itself let the click through (a chest still opens): where
    /// `ItemStack.useOn` starts. Set `claimed` to take the use: the hand
    /// swings and neither the item's `useOn` nor its `use` runs.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one); an item on cooldown is never used. API 13.
    pub(super) fn native_use_item_on(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
        claimed: &mut bool,
    ) {
        mod_api::dispatch_use_item_on(self, mod_api::mods(), stack, pos, face, claimed);
    }
    /// `DispenserBlock.getDispenseMethod`: choose a behaviour for any item,
    /// including vanilla items, on the authoritative server (not droppers).
    /// Leave answer None for vanilla. Otherwise answer `(remainder, event)`:
    /// a remainder of None empties the slot, `Some(stack)` replaces it (the hook
    /// owns consumption: no extra item is removed afterwards). `event` is the
    /// level event the dispenser plays with its smoke: 1000 the click, 1001 the
    /// fail sound (`OptionalDispenseItemBehavior` without success), or 0 for
    /// nothing at all (`DispenseItemBehavior.NOOP`). To drop one normally,
    /// answer `(native_dispense_default(..), 1000)`. API 67.
    pub(super) fn native_dispense_item(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        stack: &ItemStack,
        answer: &mut Option<NativeDispensed>,
    ) {
        mod_api::dispatch_dispense_item(
            self,
            mod_api::mods(),
            dimension,
            pos,
            facing,
            stack,
            answer,
        );
    }
    /// A player uses `stack` (right click) with `self.hand`: in the air, or
    /// on a block once `native_use_item_on` let it through, where `Item.use`
    /// starts. Each click reaches it once. Set `claimed` to take the use: the
    /// hand swings and the item's own use doesn't run. Server-side, for every
    /// player of the world; an item on cooldown is never used. API 13.
    pub(super) fn native_use_item(&mut self, stack: &ItemStack, claimed: &mut bool) {
        mod_api::dispatch_use_item(self, mod_api::mods(), stack, claimed);
    }
    /// `Item.getUseDuration` of `stack`, in ticks: `duration` arrives with
    /// vanilla's (a consumable's time, 72000 for a shield, a spear, a bow,
    /// 0 for an item with no use over time). A mod's own item (outside
    /// `minecraft:`) with no `consumable`, `blocks_attacks` or kinetic
    /// weapon that answers above 0 is used over time on a right click, as an
    /// `Item.use` that calls `startUsingItem`, until it is let go or the time
    /// runs out (`native_living_entity_use_item_finish`). A consumable's time
    /// changes too, potions and the other vanilla items keep their own.
    /// Asked when a use starts, each tick a mod's item is in use, and each
    /// frame the hand draws one in use, on both sides. It has no user, as
    /// it is asked so often: answer from the stack alone. API 90.
    pub(super) fn native_use_duration(&self, stack: &ItemStack, duration: &mut i32) {
        mod_api::dispatch_use_duration(self, mod_api::mods(), stack, duration);
    }
    /// `Item.getUseAnimation` of `stack` in use, for the first person hand:
    /// `animation` arrives with its consumable's (`eat`, `drink`), else
    /// `None`. `Eat` and `Drink` bring the food to the mouth, `Bow` draws,
    /// `Trident` raises, `Brush` sweeps; the rest hold the item still.
    /// Asked each frame the hand draws an item in use, for an item with no
    /// pose of its own (bows, crossbows, tridents, brushes and spears keep
    /// theirs). Client-side. API 90.
    pub(super) fn native_use_animation(&self, stack: &ItemStack, animation: &mut UseAnimation) {
        mod_api::dispatch_use_animation(self, mod_api::mods(), stack, animation);
    }
    /// `Player.canEat` for the food `stack`, from `Consumable.canConsume`
    /// when eating starts: `can_eat` arrives with vanilla's (creative,
    /// `can_always_eat`, or hungry). Clear it to refuse the food, set it to
    /// let a full player eat. Both sides (the client starts eating as the
    /// server does), so answer from the stack and the player alone. API 90.
    pub(super) fn native_can_eat(&self, stack: &ItemStack, can_eat: &mut bool) {
        mod_api::dispatch_can_eat(self, mod_api::mods(), stack, can_eat);
    }
    /// NeoForge's `LivingEntityUseItemEvent.Start`: `entity` starts using
    /// `stack` in `hand` (`LivingEntity.startUsingItem`), for `duration`
    /// ticks (`native_use_duration`'s answer). `entity` is a mob's
    /// `Entity.getId`, None for a player (`native_acting_player` says who);
    /// only players fire it so far. Change `duration` (consumables, potions,
    /// spyglasses and mods' items count to it; below 0 refuses the use, and
    /// 0 runs it out on its first tick, as `native_living_entity_use_item_tick`
    /// running it out, unless the item's own is 0 already), or set
    /// `canceled` and the use doesn't start. Both sides, as
    /// vanilla's: on the server for every player of the world, and on a
    /// server's client for the uses it predicts. Answer the same on both,
    /// and change the world only on the server (`native_window` is false
    /// there), not on a server's client. API 90.
    pub(super) fn native_living_entity_use_item_start(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: &mut i32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_living_entity_use_item_start(
            self,
            mod_api::mods(),
            stack,
            entity,
            hand,
            duration,
            canceled,
        );
    }
    /// NeoForge's `LivingEntityUseItemEvent.Finish`: `entity`'s use of
    /// `stack` in `hand` ran its time (`LivingEntity.completeUsingItem`),
    /// `duration` the ticks left, always 0. `entity` is as in
    /// `native_living_entity_use_item_start`. `result` arrives with what
    /// `finishUsingItem` left in the hand (the bowl of a stew, none for the
    /// last apple; a mod's own item is kept) and may be replaced. Fired for
    /// consumables, potions, spyglasses and mods' items. Server-side, for every player
    /// of the world (the helpers read the acting one). API 90.
    pub(super) fn native_living_entity_use_item_finish(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: i32,
        result: &mut Option<ItemStack>,
    ) {
        mod_api::dispatch_living_entity_use_item_finish(
            self,
            mod_api::mods(),
            stack,
            entity,
            hand,
            duration,
            result,
        );
    }
    /// NeoForge's `LivingEntityUseItemEvent.Tick`: each tick `entity` uses
    /// `stack` in `hand`, where `LivingEntity.updatingUsingItem` starts,
    /// `duration` the ticks left. `entity` is as in
    /// `native_living_entity_use_item_start`. Change `duration` to move the
    /// end (consumables, potions, spyglasses and mods' items count to it,
    /// and so do a client's eating crumbs and first person pose, as the
    /// started duration of `native_living_entity_use_item_start` does). A
    /// cancel sets it to -1. Once a mod set it, the use goes on
    /// only while it is above 0: at 0 or less, by a cancel or set, the
    /// item's tick is skipped and the server finishes the use
    /// (`completeUsingItem`: `finishUsingItem`, the food eaten,
    /// `native_living_entity_use_item_finish`), while an item used on
    /// release (a crossbow) stays frozen until let go. NeoForge's javadoc
    /// says a cancel stops the use, but its code finishes it; this follows
    /// the code. On a server's client a run-out use stays in use, frozen,
    /// so letting go still tells the server. Both sides, as vanilla's
    /// `updatingUsingItem`: answer the same on both, and change the world
    /// only on the server (`native_window` is false there), not on a
    /// server's client. Runs every tick of every use:
    /// check the item first. API 131.
    pub(super) fn native_living_entity_use_item_tick(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: &mut i32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_living_entity_use_item_tick(
            self,
            mod_api::mods(),
            stack,
            entity,
            hand,
            duration,
            canceled,
        );
    }
    /// `Item.onUseTick` of `stack`, used by `entity` with `remaining` ticks
    /// left, always above 0: what a mod's own item does each tick it is used
    /// (cook, charge, drain). It comes where `ItemStack.onUseTick` calls the
    /// item, once `native_living_entity_use_item_tick` let the use go on:
    /// after a consumable's crumbs, for consumables, shields and mods' items
    /// (vanilla's other items do their own, and a kinetic weapon has none on
    /// the server). Both sides, as `native_living_entity_use_item_tick`:
    /// change the world only where this `Game` runs the world, not on a
    /// server's client. Runs every tick of every use: check the item first.
    /// API 131.
    pub(super) fn native_on_use_tick(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        remaining: i32,
    ) {
        mod_api::dispatch_on_use_tick(self, mod_api::mods(), stack, entity, remaining);
    }
    /// NeoForge's `LivingEntityUseItemEvent.Stop`: `entity` lets go of
    /// `stack` in `hand` before its use ran its time
    /// (`LivingEntity.releaseUsingItem`: the use button let go, a screen
    /// opened), `duration` the ticks left. Set `canceled` to skip the item's
    /// release (a bow doesn't shoot, a spyglass makes no sound,
    /// `native_release_using` isn't called); the use stops anyway. Not when
    /// the hand changed item, which stops the use without a release. Both
    /// sides, as `native_living_entity_use_item_tick`. API 131.
    pub(super) fn native_living_entity_use_item_stop(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: i32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_living_entity_use_item_stop(
            self,
            mod_api::mods(),
            stack,
            entity,
            hand,
            duration,
            canceled,
        );
    }
    /// NeoForge's `ArrowLooseEvent`: the player releases a bow `stack` with
    /// `has_ammo` and `charge` ticks held. Change `charge` or set `canceled`
    /// to prevent the shot; negative charge also prevents it. Unlike
    /// `native_living_entity_use_item_stop`, this does not skip the whole item
    /// release; `native_entity_shoot_bow` reports a spawned arrow afterward
    /// with read-only force. Fires on the Game running the bow release, including
    /// LAN guests; a remote client's own release path is not covered. Crossbows
    /// are not covered; `native_arrow_nock` is the bow's start. 318.
    pub(super) fn native_arrow_loose(
        &mut self,
        stack: &ItemStack,
        has_ammo: bool,
        charge: &mut i32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_arrow_loose(self, mod_api::mods(), stack, has_ammo, charge, canceled);
    }
    /// NeoForge's `CriticalHitEvent`: a player's melee hit on a living
    /// `target_kind` (`minecraft:zombie`, `minecraft:armor_stand`) that deals
    /// damage, where `Player.attack` decides whether it is critical.
    /// `vanilla_critical` is vanilla's answer; `critical` starts as it and may
    /// be forced or denied (as in vanilla a critical hit prevents the sweep, so
    /// a forced crit loses it; a denied crit loses the crit sound and
    /// particles); `damage_multiplier` starts at 1.5 for vanilla's critical and
    /// 1.0 otherwise, and scales the damage only while `critical` ends true. Not fired for a
    /// target that is no living entity (a dragon part) or when the hit does
    /// no damage. Fires on the Game that runs the attack. 316.
    pub(super) fn native_critical_hit(
        &mut self,
        target_kind: &str,
        vanilla_critical: bool,
        critical: &mut bool,
        damage_multiplier: &mut f32,
    ) {
        mod_api::dispatch_critical_hit(
            self,
            mod_api::mods(),
            target_kind,
            vanilla_critical,
            critical,
            damage_multiplier,
        );
    }
    /// `Item.releaseUsing` of `stack`: `entity` let it go with `time_left`
    /// ticks of its `native_use_duration` left, so it was held for
    /// `duration - time_left` (a bow-like item shoots by that charge). Set
    /// `used` as vanilla's return, and the stack's use cooldown starts
    /// (`ItemStack.releaseUsing`). After the vanilla item's own release, once
    /// `native_living_entity_use_item_stop` let it through. Both sides, as
    /// vanilla's: change the world only where this `Game` runs the world,
    /// not on a server's client. API 131.
    pub(super) fn native_release_using(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        time_left: i32,
        used: &mut bool,
    ) {
        mod_api::dispatch_release_using(self, mod_api::mods(), stack, entity, time_left, used);
    }
    /// A player breaks the block `state` at `pos`, once the game allowed it
    /// (the held item can break blocks, the debug stick didn't take the
    /// click): where `ServerPlayerGameMode.destroyBlock` starts. Set
    /// `canceled` to keep the block: nothing is destroyed or dropped, and a
    /// guest sees it back. Server-side, for every player of the world (the
    /// helpers read the acting one). API 14.
    pub(super) fn native_destroy_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_destroy_block(self, mod_api::mods(), pos, state, canceled);
    }
    /// A player placed `stack` as the block `state` at `pos`: where
    /// `BlockItem.place` set it, before the stack shrinks. It only hears; a
    /// mod stops a placement by claiming `native_use_item_on`, which runs
    /// before it. Server-side, for every player of the world. API 14.
    pub(super) fn native_place_block(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        state: BlockStateId,
    ) {
        mod_api::dispatch_place_block(self, mod_api::mods(), stack, pos, state);
    }
    /// A player right clicks the block at `pos` with `self.hand`, `face` the
    /// face clicked: where `ServerPlayerGameMode.useItemOn` starts, before
    /// the block's own use (a chest, a door, a lever) and before
    /// `native_use_item_on`. Set `canceled` to stop the click: neither the
    /// block nor the item acts, the hand doesn't swing, and a guest sees the
    /// block as it is. Server-side, for every player of the world
    /// (`native_acting_player` says who), once per hand. API 21.
    pub(super) fn native_right_click_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        canceled: &mut bool,
    ) {
        mod_api::dispatch_right_click_block(self, mod_api::mods(), pos, face, canceled);
    }
    /// An explosion at `center` in `dimension` (`minecraft:overworld`) is
    /// about to break `blocks`: `ServerExplosion.explode` with its `toBlow`,
    /// before any block is hit. Take out the positions to spare; they keep
    /// their block and get no fire. Entities are hurt as before. Server-side.
    /// API 22.
    pub(super) fn native_explosion_detonate(
        &mut self,
        dimension: &str,
        center: [f64; 3],
        blocks: &mut Vec<[i32; 3]>,
    ) {
        mod_api::dispatch_explosion_detonate(self, mod_api::mods(), dimension, center, blocks);
    }
    /// An explosion at `center` in `dimension`, about to break `blocks`, pushes
    /// `entity` (None for a player, whom `native_acting_player` names):
    /// `ServerExplosion.hurtEntities`, after the knockback is calculated. Change `knockback_velocity` to change
    /// what is added to the entity's velocity; a player's client is sent the
    /// same. Wind bursts too, except dragons and end crystals. End crystals
    /// are also omitted from ordinary blasts: Horizon removes them without a
    /// velocity to change. Server-side. @API:explosion_knockback@.
    pub(super) fn native_explosion_knockback(
        &mut self,
        dimension: &str,
        center: [f64; 3],
        blocks: &[[i32; 3]],
        entity: Option<u32>,
        knockback_velocity: &mut [f64; 3],
    ) {
        mod_api::dispatch_explosion_knockback(
            self,
            mod_api::mods(),
            dimension,
            center,
            blocks,
            entity,
            knockback_velocity,
        );
    }
    /// A player fills (`stack` an empty bucket) or empties a bucket at `pos`:
    /// `BucketItem.use`, where it asks `Level.mayInteract`. Set `canceled`
    /// and the bucket does nothing. Server-side, for every player of the
    /// world (`native_acting_player` says who). API 22.
    pub(super) fn native_fill_bucket(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        canceled: &mut bool,
    ) {
        mod_api::dispatch_fill_bucket(self, mod_api::mods(), stack, pos, canceled);
    }
    /// A player attacks the entity `entity` (`Entity.getId`) of type `kind`
    /// (`minecraft:item_frame`, `minecraft:cow`, `minecraft:player`) standing
    /// at `pos`: `ServerGamePacketListenerImpl.handleInteract`'s attack, once
    /// in reach, where `Player.attack` starts. Set `canceled` and the blow
    /// does nothing (no damage, no knockback, the frame keeps its item).
    /// Server-side, for every player of the world (`native_acting_player`
    /// says who). API 22.
    pub(super) fn native_attack_entity(
        &mut self,
        entity: i32,
        kind: &str,
        pos: [f64; 3],
        canceled: &mut bool,
    ) {
        mod_api::dispatch_attack_entity(self, mod_api::mods(), entity, kind, pos, canceled);
    }
    /// A message from `player`'s client on `channel` (`mod_id:name`), within
    /// the caps (`native_net.rs`): check `channel`, and treat `data` as the
    /// player's own bytes, checked by nothing. Runs in the tick after it
    /// arrives, with no player's state swapped in. Server-side. API 40.
    pub(super) fn native_net_from_client(&mut self, player: u128, channel: &str, data: &[u8]) {
        mod_api::dispatch_net_from_client(self, mod_api::mods(), player, channel, data);
    }
    /// A message from the server on `channel` (`mod_id:name`), whose `data`
    /// is the sender's own bytes. The window's game only. API 40.
    pub(super) fn native_net_from_server(&mut self, channel: &str, data: &[u8]) {
        mod_api::dispatch_net_from_server(self, mod_api::mods(), channel, data);
    }
    /// `dimension`'s level opened on the server (NeoForge's `LevelEvent.Load`,
    /// posted in `MinecraftServer.createLevels`): before any of its chunks
    /// (`native_chunk_load`), so a mod sets up what it keeps for the level
    /// here. The level the world opens in comes before `WorldOpenReady`; the
    /// others open when someone first goes there (a player, a portal, an
    /// `execute in` command), with the level in place. Once for each open:
    /// `native_level_unload` follows. Server-side. API 134.
    pub(super) fn native_level_load(&mut self, dimension: &str) {
        mod_api::dispatch_level_load(self, mod_api::mods(), dimension);
    }
    /// `dimension`'s level closed (NeoForge's `LevelEvent.Unload`, posted after
    /// `level.close()` in `MinecraftServer.stopServer`), after its last save:
    /// drop what the mod kept for it. A level other than the Overworld closes
    /// once nobody has been in it for 300 ticks, after its chunks' unloads;
    /// every level still open closes when the server stops, before
    /// `WorldCloseFinal`, and when its open or its loop fails, before
    /// `WorldOpenFailed` or `WorldCloseFailed`. Once for each
    /// `native_level_load`. Don't read its blocks. Server-side. API
    /// 134.
    pub(super) fn native_level_unload(&mut self, dimension: &str) {
        mod_api::dispatch_level_unload(self, mod_api::mods(), dimension);
    }
    /// The server's difficulty changed, from `from` to `to` (`peaceful`,
    /// `easy`, `normal`, `hard`; NeoForge's `DifficultyChangeEvent`). It is
    /// vanilla's `MinecraftServer.setDifficulty`, which `/difficulty`, a
    /// dedicated server's management and a test environment's difficulty all
    /// go through, and it is told only when the difficulty is another one. It
    /// only hears: the change has already happened. Server-side. API
    /// 159.
    pub(super) fn native_difficulty_changed(&mut self, from: &str, to: &str) {
        mod_api::dispatch_difficulty_changed(self, mod_api::mods(), from, to);
    }
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` came into the world
    /// (NeoForge's `ChunkEvent.Load`): `new` when it was just generated, false
    /// when read from the save. A flat world's chunk that leaves before it's
    /// first lit isn't saved, so it's generated, and new, again; one generated
    /// as the players left is new when it next comes, unless the server
    /// restarts first, or the level closes for being empty, which drops its
    /// memory of what never saved. Its blocks and block entities are there,
    /// but its saved entities join afterwards and its light may not be ready:
    /// change the world from a later tick. The chunks around the spawn come
    /// before `WorldOpenReady`. Not told of a chunk a client receives.
    /// Server-side. API 57.
    pub(super) fn native_chunk_load(&mut self, dimension: &str, chunk: [i32; 2], new: bool) {
        mod_api::dispatch_chunk_load(self, mod_api::mods(), dimension, chunk, new);
    }
    /// Chunk `chunk` of `dimension` left the world (NeoForge's
    /// `ChunkEvent.Unload`), with a level that closes for being empty too. Don't
    /// read its blocks. Its block entities are still there for this call, so
    /// `native_with_mod_block_entity` can change one and it's saved with the
    /// chunk. Not told when the world closes, which `native_world_lifecycle`
    /// tells. Server-side. API 57.
    pub(super) fn native_chunk_unload(&mut self, dimension: &str, chunk: [i32; 2]) {
        mod_api::dispatch_chunk_unload(self, mod_api::mods(), dimension, chunk);
    }
    /// `player` (`name`) joined the game (NeoForge's
    /// `PlayerLoggedInEvent`): placed in the world and listed in
    /// `native_server_players`, so `native_net_send_to_player` reaches them.
    /// Once a join, with no player's state swapped in. Server-side. API 57.
    pub(super) fn native_player_logged_in(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_logged_in(self, mod_api::mods(), player, name);
    }
    /// `player` (`name`) left the game (NeoForge's `PlayerLoggedOutEvent`), by
    /// any path: a lost connection, a kick, the server stopping or failing.
    /// Once for each player `native_player_logged_in` told of. They've already
    /// left `native_server_players`, except when the server stops, which tells
    /// before it saves, with them still listed and able to get a last message.
    /// Server-side. API 57.
    pub(super) fn native_player_logged_out(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_logged_out(self, mod_api::mods(), player, name);
    }
    /// `player` (`name`) respawned (NeoForge's `PlayerRespawnEvent`): at the
    /// end of `PlayerList.respawn`, once they are placed and their health,
    /// food and experience reset, with their state swapped in. Once for each
    /// death that ends in a respawn, on a server; not for the End-exit one.
    /// 115.
    pub(super) fn native_player_respawned(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_respawned(self, mod_api::mods(), player, name);
    }
    /// A player was saved to the world's store (NeoForge's
    /// `PlayerEvent.SaveToFile`, `PlayerList.save`): when they leave or the
    /// world closes, when a dedicated or integrated server saves its guests,
    /// and for an integrated server's owner when the world saves them.
    /// Once for each save. `directory`
    /// is the world's player directory (`players/data`): write your own file
    /// there, named `<uuid>.<suffix>`, never the player's `.dat`. The player
    /// may be leaving, so no player's state is swapped in. Not cancellable.
    /// Autosave snapshots are written asynchronously: this notification runs
    /// on a later server tick after a successful write, possibly after WorldSaved.
    /// Live state may have advanced beyond the saved snapshot; a process exit
    /// before completions are drained can prevent notification delivery.
    pub(super) fn native_player_save_to_file(
        &mut self,
        player: u128,
        name: &str,
        directory: &std::path::Path,
    ) {
        mod_api::dispatch_player_save_to_file(self, mod_api::mods(), player, name, directory);
    }
    /// A player was loaded from the world's save (NeoForge's
    /// `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`): their `.dat`
    /// has been read and applied, and they are not placed in a level yet.
    /// `directory` is the world's player directory (`players/data`): a mod
    /// reads its own file there, named `<uuid>.<suffix>` as
    /// `getPlayerFile(suffix)` names it. Heard once for each join, for the
    /// owner of an integrated server as for anyone else, on a server, with no
    /// player's state swapped in. Not cancellable.
    pub(super) fn native_player_load_from_file(
        &mut self,
        player: u128,
        name: &str,
        directory: &std::path::Path,
    ) {
        mod_api::dispatch_player_load_from_file(self, mod_api::mods(), player, name, directory);
    }
    /// A player's data was copied to their new player (NeoForge's
    /// `PlayerEvent.Clone`, from `ServerPlayer.restoreFrom`): `was_death` is
    /// true after a death, false when they left the End after the credits
    /// (`respawn` with `keepAllPlayerData`). The uuid stays the same, so the
    /// old and the new player share `player`. Heard before the new player is
    /// placed, on a server, with no player's state swapped in. Not
    /// cancellable.
    pub(super) fn native_player_clone(&mut self, player: u128, name: &str, was_death: bool) {
        mod_api::dispatch_player_clone(self, mod_api::mods(), player, name, was_death);
    }
    /// The number of players with the mod container at `pos` of `dimension`
    /// open went from `previous` to `viewers` (vanilla's
    /// `ContainerOpenersCounter`, as a barrel keeps it): 0 to more than 0 is
    /// `onOpen`, back to 0 is `onClose`, any change `openerCountChanged`, so a
    /// block can show itself open while `viewers` is not 0. Counted every
    /// tick from the players' open menus, so a player who leaves without
    /// closing (a lost connection, death, another dimension, out of reach, the
    /// block gone) is heard too (`recheckOpeners`). Once for each change, with
    /// no player's state swapped in. Server-side.
    /// 124.
    pub(super) fn native_mod_container_viewers(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        viewers: u32,
        previous: u32,
    ) {
        mod_api::dispatch_mod_container_viewers(
            self,
            mod_api::mods(),
            dimension,
            pos,
            viewers,
            previous,
        );
    }
    /// `player` (`name`) pressed the swap-hands key: heard before the main and
    /// off hand items swap (the `SWAP_ITEM_WITH_OFFHAND` player action that
    /// `ServerGamePacketListenerImpl.handlePlayerAction` answers; Bukkit's
    /// `PlayerSwapHandItemsEvent`), not for a spectator. Set `cancel` to keep
    /// the items where they are: the player's client, which swapped them at
    /// once, gets its slots back. The player's state is swapped in, so
    /// `native_acting_player`, `native_tell` and the item helpers reach them.
    /// Once for each press, on a server. Not heard: the same swap made in an
    /// open inventory (hovering a slot and pressing the key, a menu click) and
    /// the host of a LAN world. 168.
    pub(super) fn native_player_swap_hands(&mut self, player: u128, name: &str, cancel: &mut bool) {
        mod_api::dispatch_player_swap_hands(self, mod_api::mods(), player, name, cancel);
    }
    /// `player` (`name`) opened a menu (NeoForge's `PlayerContainerEvent.Open`,
    /// posted when `ServerPlayer.openMenu` sets the new `containerMenu`): any
    /// menu, vanilla or a mod's. `menu` is the menu type's name
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:merchant`,
    /// `minecraft:crafting`, `minecraft:lectern`, a mod's own), `pos` the
    /// block it is of in `dimension` (none for a trade, a vehicle's, a mod's
    /// native menu or a mount's inventory, which has no menu type in vanilla
    /// and is told as `horizon:mount_inventory`). The player's own inventory
    /// is not one. Heard once a tick, so a menu opened and closed again
    /// inside one tick is not; one that replaces another comes after that
    /// one's close. The player's state is swapped in. `native_player_container_close`
    /// follows. Server-side. API 152.
    pub(super) fn native_player_container_open(
        &mut self,
        player: u128,
        name: &str,
        dimension: &str,
        menu: &str,
        pos: Option<[i32; 3]>,
    ) {
        mod_api::dispatch_player_container_open(
            self,
            mod_api::mods(),
            player,
            name,
            dimension,
            menu,
            pos,
        );
    }
    /// `player` (`name`) closed the menu `native_player_container_open` told
    /// of (NeoForge's `PlayerContainerEvent.Close`, posted in
    /// `ServerPlayer.doCloseContainer` after the menu's `removed`): they
    /// closed it, walked out of reach, died, changed level, or another menu
    /// opened. A player who leaves closes it before `native_player_logged_out`
    /// (vanilla posts nothing for a disconnect; Horizon does, so each open has
    /// its close). Once for each open; not told when the server stops. The
    /// menu's items are where it left them: the block's, and the cursor stack
    /// back in the inventory. State swapped in, when the player has a body.
    /// Server-side. API 152.
    pub(super) fn native_player_container_close(
        &mut self,
        player: u128,
        name: &str,
        dimension: &str,
        menu: &str,
        pos: Option<[i32; 3]>,
    ) {
        mod_api::dispatch_player_container_close(
            self,
            mod_api::mods(),
            player,
            name,
            dimension,
            menu,
            pos,
        );
    }
    /// A client started tracking an entity (NeoForge's
    /// `PlayerEvent.StartTracking`, posted at the end of
    /// `ServerEntity.addPairing`): `player` (`name`) is now sent `entity`
    /// (its ID, `uuid` its UUID, `kind` its type, `minecraft:zombie`) and its
    /// changes. It comes into range, its client's chunks reach it, or it is
    /// put in the level; another player is an entity too, and a player never
    /// tracks their own. A mod sends the player what they need to show it
    /// (`native_net_send_to_player`). The entity is as it was last tick: it
    /// may be gone by now. Heard after the tick's players and entities have
    /// moved, in order, with no player's state swapped in. Server-side.
    /// API 141.
    pub(super) fn native_player_start_tracking(
        &mut self,
        player: u128,
        name: &str,
        entity: u32,
        uuid: u128,
        kind: &str,
    ) {
        mod_api::dispatch_player_start_tracking(
            self,
            mod_api::mods(),
            player,
            name,
            entity,
            uuid,
            kind,
        );
    }
    /// A client stopped tracking an entity (NeoForge's
    /// `PlayerEvent.StopTracking`, posted at the end of
    /// `ServerEntity.removePairing`): it went out of range, left the level or
    /// died, or `player` (`name`) left, changed level or respawned, which
    /// stops everything they tracked. `entity`, `uuid` and `kind` are as
    /// `native_player_start_tracking` told. Once for each start, a leaving
    /// player's after `native_player_logged_out`. The entity is not
    /// necessarily still there, and a player who left is gone: no player's
    /// state is swapped in. Server-side. API 141.
    pub(super) fn native_player_stop_tracking(
        &mut self,
        player: u128,
        name: &str,
        entity: u32,
        uuid: u128,
        kind: &str,
    ) {
        mod_api::dispatch_player_stop_tracking(
            self,
            mod_api::mods(),
            player,
            name,
            entity,
            uuid,
            kind,
        );
    }
    /// Whether a mod listens to the mobs' `native_living_equipment_change`: a
    /// mod that hooks it hooks this one too and sets `listened` (HEAD,
    /// `*listened = true`). Without it the game does not read every mob's
    /// equipment each tick, and mobs are not told (armor stands still are). A
    /// mob that ticks once it is set counts from empty. Asked once a tick, after
    /// the mobs' tick. Server-side. API 148.
    pub(super) fn native_living_equipment_listened(&mut self, listened: &mut bool) {
        mod_api::dispatch_living_equipment_listened(self, mod_api::mods(), listened);
    }
    /// A mob's or an armor stand's equipment slot took a different stack
    /// (NeoForge's `LivingEquipmentChangeEvent`, posted in
    /// `LivingEntity.collectEquipmentChanges`): `entity` is its ID and `kind`
    /// its type (`minecraft:zombie`), `slot` the `EquipmentSlot` (`mainhand`,
    /// `offhand`, `feet`, `legs`, `chest`, `head`, `body`, `saddle`), `from`
    /// what the slot held when this was last heard (none for empty) and `to`
    /// what it holds now. Once for each slot that changed, in that order,
    /// after the mobs' tick, so a mob that joins the level wearing something
    /// counts from empty on its first tick. It only hears: change the mob with
    /// its own helpers. Players are heard by `native_on_equip`. Server-side.
    /// API 148.
    pub(super) fn native_living_equipment_change(
        &mut self,
        entity: u32,
        kind: &str,
        slot: &str,
        from: Option<&ItemStack>,
        to: Option<&ItemStack>,
    ) {
        mod_api::dispatch_living_equipment_change(
            self,
            mod_api::mods(),
            entity,
            kind,
            slot,
            from,
            to,
        );
    }
    /// `player` died: where `ServerPlayer.die` starts, with the death
    /// message's key when the game knows it (`death.attack.lava`; none for a
    /// message it builds from the combat). It only hears. Server-side, for
    /// every player of the world, whether death messages are shown or not.
    /// API 40.
    pub(super) fn native_player_died(&mut self, player: u128, cause: Option<&str>) {
        mod_api::dispatch_player_died(self, mod_api::mods(), player, cause);
    }
    /// A new mob of `kind` joined `dimension` at `pos`: `natural` when the
    /// world's spawning made it, false when something else did (a breed, a
    /// raid, a spawner, a conversion, `/summon`, and the second mob of a
    /// jockey the world spawned). One call a mob: a `/summon` with passengers
    /// tells each. Not told of a mob loaded with its chunk, leaving a hive or
    /// changing level. It only hears; the mob is already among the game's
    /// mobs. Server-side. API 40.
    pub(super) fn native_mob_spawned(
        &mut self,
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        natural: bool,
    ) {
        mod_api::dispatch_mob_spawned(self, mod_api::mods(), dimension, kind, pos, natural);
    }
    /// An entity leaves the server level `dimension` (NeoForge's
    /// `EntityLeaveLevelEvent`): vanilla's `onTrackingEnd`, when it is removed
    /// for any reason (picked up, merged, despawned, burnt, blown up, killed,
    /// taken by a hopper or a mob, gone through a portal) or its chunk unloads,
    /// a level closing with nobody in it included. Not for the entities still
    /// loaded when the server stops, as in vanilla. `entity` is its ID and
    /// `kind` its type (`minecraft:item`); it is no longer in the level's lists.
    /// It only hears. 53 told item entities, experience orbs and mobs; it now
    /// tells every kind `native_entity_join_level` does (`kind` is the type), each
    /// once, when the game next notices it gone (the tick or action that removed
    /// it). Check `kind`. Server-side.
    pub(super) fn native_entity_leave_level(&mut self, dimension: &str, entity: u32, kind: &str) {
        mod_api::dispatch_entity_leave_level(self, mod_api::mods(), dimension, entity, kind);
    }
    /// An entity joins the server level `dimension` (NeoForge's
    /// `EntityJoinLevelEvent`): vanilla's `PersistentEntitySectionManager.addEntity`.
    /// `loaded_from_disk` is true for an entity coming back with its chunk,
    /// false for one added fresh (`addFreshEntity`: a drop, an orb, `/summon`,
    /// an arrival through a portal). `entity` is its ID and `kind` its type
    /// (`minecraft:item`). It is already in the level's lists, so helpers that
    /// take an entity ID find it. Set `canceled` to keep it out: it is taken
    /// back out as if never added, and a `/summon` of it fails. Tells every
    /// kind in the game's entity lists (items, orbs, mobs, boats, minecarts,
    /// armor stands, primed TNT, falling blocks, projectiles, rockets,
    /// tridents, end crystals, the dragon, cushions, paintings, item frames),
    /// once each, in the order they were created; check `kind`. A drop, an
    /// orb, a mob, a `/summon` and a chunk's entities are told at once, any
    /// other spawn between the tick's phases (it is checked after the blocks,
    /// the vehicles, the projectiles, the items and the mobs, and at the
    /// tick's end). A mod that cancels one of the others sees it removed
    /// before it ticks again. Not told: the entities Horizon keeps without
    /// simulating (displays, markers, interactions). Server-side.
    pub(super) fn native_entity_join_level(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
        pos: [f64; 3],
        loaded_from_disk: bool,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_entity_join_level(
            self,
            mod_api::mods(),
            dimension,
            entity,
            kind,
            pos,
            loaded_from_disk,
            canceled,
        );
    }
    /// An entity mounts or dismounts a vehicle (NeoForge's `EntityMountEvent`, posted from
    /// `Entity.startRiding` and `removeVehicle`): `entity` of type `kind` (`minecraft:player`,
    /// `minecraft:zombie`) in the server level `dimension`, getting on (`mounting`) or off the
    /// `vehicle` of type `vehicle_kind` (`minecraft:oak_boat`, `minecraft:minecart`,
    /// `minecraft:horse`, `minecraft:cushion`). Set `canceled` and it does not: a player stays
    /// where they were, a mob does not board, a `/ride` fails, and a rider whose dismount is
    /// canceled keeps riding. Told for a player boarding a boat, minecart, cushion or a mount (pig,
    /// strider, horse and its kin, llama, nautilus, happy ghast) and getting off one by their
    /// own input, `/ride`, a stab, a buck, a teleport or spectator mode; and for the mobs
    /// Horizon seats on a vehicle (jockeys, raiders on a ravager, `/ride`) and lets off one
    /// (`/ride`, a stab). It is not told for a rider whose vehicle was removed from the
    /// level (a boat or minecart destroyed, a broken cushion), nor for riders loaded with a
    /// save. The acting player is the one whose action runs, a guest on a server. Server-side.
    /// API 137.
    pub(super) fn native_entity_mount(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
        vehicle: u32,
        vehicle_kind: &str,
        mounting: bool,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_entity_mount(
            self,
            mod_api::mods(),
            dimension,
            entity,
            kind,
            vehicle,
            vehicle_kind,
            mounting,
            canceled,
        );
    }
    /// Whether a mod listens to `native_entity_tick_pre` and `native_entity_tick_post`: a mod
    /// that hooks them hooks this one too and sets `listened` (HEAD, `*listened = true`).
    /// Without it the game tells no entity's tick, and keeps no list of them. Asked once for
    /// each family (mobs, items, minecarts, boats) each tick. Server-side. API 146.
    pub(super) fn native_entity_tick_listened(&mut self, listened: &mut bool) {
        mod_api::dispatch_entity_tick_listened(self, mod_api::mods(), listened);
    }
    /// An entity of type `kind` (`minecraft:zombie`, `minecraft:item`) with ID `entity` is
    /// about to tick in the server level `dimension` (NeoForge's `EntityTickEvent.Pre`, posted
    /// around `Entity.commonTick` and `tick` in `ServerLevel.tickNonPassenger`). Set `canceled`
    /// and it does not tick this tick: a mob has no AI, physics or effects, an item no movement,
    /// ageing or merging, a minecart or boat no motion, and `native_entity_tick_post` is not told.
    /// Horizon ticks a family of entities together, so mobs and items hear every Pre before the
    /// first of them ticks; minecarts and boats hear theirs one by one. Told for the mobs, item
    /// entities, minecarts and boats that tick: those in an entity-ticking chunk, not a player's
    /// boat or cart (it ticks with the player). Later APIs add other kinds, so check `kind`. It
    /// runs for every ticking entity each tick, so keep it cheap. Server-side. API
    /// 146.
    pub(super) fn native_entity_tick_pre(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_entity_tick_pre(self, mod_api::mods(), dimension, entity, kind, canceled);
    }
    /// The entity `entity` of type `kind` ticked in the server level `dimension` (NeoForge's
    /// `EntityTickEvent.Post`): for each entity `native_entity_tick_pre` let through, after it
    /// ticked, also one that died or was removed in its tick. Mobs and items hear their Posts
    /// once the whole family has ticked; minecarts and boats right after their own tick. Server-side.
    /// API 146.
    pub(super) fn native_entity_tick_post(&mut self, dimension: &str, entity: u32, kind: &str) {
        mod_api::dispatch_entity_tick_post(self, mod_api::mods(), dimension, entity, kind);
    }
    /// Whether the experience orb `into` takes the orb `from` in (vanilla's
    /// `ExperienceOrb.canMerge`): the same value, and ids a multiple of 40 apart. Asked
    /// for each pair whose boxes meet, when an orb spawns (`tryMergeToExisting`: `from` is
    /// 0, the new orb isn't an entity yet, and only `from_value` is known) and every 20
    /// ticks as an orb scans for neighbours (`scanForMerges`). `can_merge` holds vanilla's
    /// answer; change it either way. A merged orb counts as `count` orbs of `into`'s
    /// value, so forcing orbs of different values together drops points unless the mod
    /// raises `into`'s value with `native_set_experience_orb` in `native_experience_orb_merge`.
    /// Server-side. API 128.
    pub(super) fn native_experience_orb_can_merge(
        &mut self,
        dimension: &str,
        into: u32,
        from: u32,
        into_value: u32,
        from_value: u32,
        can_merge: &mut bool,
    ) {
        mod_api::dispatch_experience_orb_can_merge(
            self,
            mod_api::mods(),
            dimension,
            into,
            from,
            into_value,
            from_value,
            can_merge,
        );
    }
    /// The experience orb `into` is about to take `from` in, after
    /// `native_experience_orb_can_merge` said yes (`from` is 0 and `from_value` the
    /// points of a new orb that is spawning). Set `canceled` to stop it: both orbs stay, and
    /// a spawning orb tries the next orb in reach, then spawns on its own. Server-side.
    /// API 128.
    pub(super) fn native_experience_orb_merge(
        &mut self,
        dimension: &str,
        into: u32,
        from: u32,
        from_value: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_experience_orb_merge(
            self,
            mod_api::mods(),
            dimension,
            into,
            from,
            from_value,
            canceled,
        );
    }
    /// A totem of undying saved the server player `player` (`name`): their
    /// hurt would have killed them and the totem held in either hand fired
    /// (`LivingEntity.checkTotemDeathProtection`; Bukkit's
    /// `EntityResurrectEvent`), heard just before it is used up and its
    /// effects apply. Listen only: whether the totem saves is decided inside
    /// the health state machine, before a mod is asked, so a mod cannot veto
    /// it from here. The player's state is swapped in, so
    /// `native_acting_player`, `native_tell` and the item helpers reach them.
    /// Once for each totem, on a server; the host of a LAN world's totem is
    /// not heard. 182.
    pub(super) fn native_player_totem(&mut self, player: u128, name: &str) {
        mod_api::dispatch_player_totem(self, mod_api::mods(), player, name);
    }
    /// The item `item` of the server player `player` (`name`) broke in `slot`
    /// (`mainhand`, `offhand`, `head`, `chest`, `legs`, `feet`, `body` or
    /// `saddle`): `LivingEntity.onEquippedItemBroken`, Bukkit's
    /// `PlayerItemBreakEvent`. Listen only; armour has a slot, not a hand.
    /// The player's state is swapped in. Once for each break, on a server; a
    /// creative player and the host of a LAN world are not heard.
    /// 315.
    pub(super) fn native_player_destroy_item(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        slot: &str,
    ) {
        mod_api::dispatch_player_destroy_item(self, mod_api::mods(), player, name, item, slot);
    }
    /// `player`'s melee blow killed the mob `entity` of `kind` (other deaths
    /// aren't told yet). It only hears, after the blow landed. Server-side.
    /// API 40.
    pub(super) fn native_mob_killed(&mut self, player: u128, entity: u32, kind: &str) {
        mod_api::dispatch_mob_killed(self, mod_api::mods(), player, entity, kind);
    }
    /// A block broke or a mob died, and `drops` are the items about to drop, in
    /// the order they drop: what the loot tables rolled (after
    /// `native_loot_table_load`'s changes), and for a mob what it wore, held or
    /// carried (a fox's mouth item, an allay's, piglin's or horse's inventory),
    /// but not an enderman's carried block or a copper golem's preserved
    /// equipment, which drop after. A charged creeper's head and a fox's mouth
    /// item come before the mob's own table. A block's stacks get its block
    /// entity's data (a shulker box's items) after this. Add, change or take out
    /// stacks; empty ones don't drop (NeoForge's global loot modifiers, Fabric's
    /// `LootTableEvents.MODIFY_DROPS`); `context` says what dropped them and
    /// how, and it is called even when `drops` is empty. A player's use drops
    /// through it too (`NativeLootDrops::used`: shears, a brush, an item
    /// frame), with that player named. It hears blocks that a
    /// player, an explosion or a mob breaks, not other breaks (leaves decaying,
    /// a torch losing its wall, a piston, `/setblock ... destroy`), nor a
    /// creative break or a tool that can't harvest the block, nor experience.
    /// Server-side, for every player. API
    /// 75.
    pub(super) fn native_loot_drops(
        &mut self,
        context: &NativeLootDrops<'_>,
        drops: &mut Vec<ItemStack>,
    ) {
        mod_api::dispatch_loot_drops(self, mod_api::mods(), context, drops);
    }
    /// The item `item` of a server player is about to lose `amount` points of
    /// durability (`ItemStack.hurtAndBreak`; Bukkit's `PlayerItemDamageEvent`):
    /// the tool that mines or hits, the armour that takes a hit, a shield, a
    /// rod, an elytra. `amount` is what is left after Unbreaking; change it
    /// (0 or less leaves the item whole) or set `cancel` for no wear at all. It
    /// is not heard for a creative player's tools (they take no wear) nor when
    /// Unbreaking already saved the item, nor for the wear of a mob's armour or
    /// of a tool a dispenser uses, nor the host of a LAN world. The player's
    /// state is swapped in, so `native_acting_player`, `native_tell` and the
    /// item helpers reach them. Once for each wear, on a server.
    /// 162.
    pub(super) fn native_item_damaged(
        &mut self,
        player: u128,
        name: &str,
        item: &str,
        amount: &mut i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_item_damaged(self, mod_api::mods(), player, name, item, amount, cancel);
    }
    /// `player` threw `count` of `item` (`Player.drop`: the drop key, a
    /// menu's throw...), not what scatters when they die. It only hears.
    /// Server-side. API 40.
    pub(super) fn native_item_dropped(&mut self, player: u128, item: &str, count: u32) {
        mod_api::dispatch_item_dropped(self, mod_api::mods(), player, item, count);
    }
    /// `player` is about to take `count` of `item` from the ground
    /// (`ItemEntity.playerTouch`). Set `canceled` to leave it there: the
    /// stack isn't offered again for 10 ticks. Server-side. API 40.
    pub(super) fn native_item_picked_up(
        &mut self,
        player: u128,
        item: &str,
        count: u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_item_picked_up(self, mod_api::mods(), player, item, count, canceled);
    }
    /// A dropped item reaches its lifetime on the server (`ItemEntity.tick`,
    /// NeoForge's `ItemExpireEvent`). Every mod may set or add to `extra_life`,
    /// in ticks, which starts at zero; a positive total keeps the item that much
    /// longer, zero or less lets it expire. Extensions are capped at 32766
    /// ticks. 325.
    pub(super) fn native_item_expire(
        &mut self,
        entity: u32,
        stack: &ItemStack,
        extra_life: &mut i32,
    ) {
        mod_api::dispatch_item_expire(self, mod_api::mods(), entity, stack, extra_life);
    }
    /// A player took a crafting grid's result (`ResultSlot.onTake`, NeoForge's
    /// `PlayerEvent.ItemCraftedEvent`): `player` is the player's uuid, `recipe`
    /// the recipe the grid matched (`minecraft:oak_planks`) and `stack` the
    /// result it took, with its components. It only hears: the take has
    /// already happened. It fires for the crafting grid of an inventory or a
    /// crafting table, on the integrated server and a dedicated server, guests
    /// included, and not for a stonecutter's, a smithing table's or a
    /// furnace's result. Server-side. API 161.
    pub(super) fn native_item_crafted(&mut self, player: u128, recipe: &str, stack: &ItemStack) {
        mod_api::dispatch_item_crafted(self, mod_api::mods(), player, recipe, stack);
    }
    /// The food level of the server player `player` (`name`) is about to change
    /// from `from` to `to` (Bukkit's `FoodLevelChangeEvent`):
    /// eating, hunger and saturation effects, the exhaustion that costs a
    /// point, starving. Change `to` (kept between 0 and 20; saturation never
    /// exceeds it) or set `cancel` to leave the level at `from`, saturation
    /// included. Heard once for the change a player's turn made as a whole, so
    /// two changes in one tick arrive as one. Not heard: the full food bar a
    /// respawn gives, the food a player joins with and the host of a LAN
    /// world. The player's state is swapped in, so `native_acting_player`,
    /// `native_tell` and the item helpers reach them. On a server.
    /// 169.
    pub(super) fn native_food_level_changed(
        &mut self,
        player: u128,
        name: &str,
        from: i32,
        to: &mut i32,
        cancel: &mut bool,
    ) {
        mod_api::dispatch_food_level_changed(self, mod_api::mods(), player, name, from, to, cancel);
    }
    /// Whether a mod hears `native_experience_orb_pickup`: set `listened`
    /// (a HEAD injection), or the server skips swapping the player in for it,
    /// and with this empty the pickup costs nothing more without mods.
    /// Server-side. 155.
    pub(super) fn native_experience_orb_pickup_listened(&self, listened: &mut bool) {
        mod_api::dispatch_experience_orb_pickup_listened(self, mod_api::mods(), listened);
    }
    /// Before `player` picks up orb `orb` in `dimension`. Mutations to `value`
    /// and `count` take effect only when not canceled; value is clamped to
    /// 1..=i32::MAX and count to at least one.
    /// Canceling leaves the orb and pickup delay unchanged. Server-side.
    /// 155.
    pub(super) fn native_experience_orb_pickup(
        &mut self,
        player: u128,
        dimension: &str,
        orb: u32,
        value: &mut u32,
        count: &mut u32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_experience_orb_pickup(
            self,
            mod_api::mods(),
            player,
            dimension,
            orb,
            value,
            count,
            canceled,
        );
    }
    /// The selected damaged Mending `stack` and the proposed durability repair
    /// for `experience` points. The amount is clamped to the stack's current damage. Zero leaves the
    /// experience unspent when it overrides a positive proposal. If the selected
    /// stack changes, the repair is skipped.
    /// Server-side, including `native_give_experience`.
    /// 155.
    pub(super) fn native_mending_repair(
        &mut self,
        player: u128,
        stack: &ItemStack,
        experience: u32,
        repair: &mut u32,
    ) {
        mod_api::dispatch_mending_repair(self, mod_api::mods(), player, stack, experience, repair);
    }
    /// A player's hit landed on the mob `entity` of type `kind`
    /// (`minecraft:zombie`), holding `stack`: where `Item.hurtEnemy` is called,
    /// before the post-attack enchantments and the weapon's wear. It only
    /// hears. Another player as the target isn't told yet. Server-side, for
    /// every player of the world (the helpers read the acting one). API 39.
    pub(super) fn native_hurt_enemy(&mut self, stack: &ItemStack, entity: u32, kind: &str) {
        mod_api::dispatch_hurt_enemy(self, mod_api::mods(), stack, entity, kind);
    }
    /// A server tick of the stack a player carries in `slot`: where
    /// `Inventory.tick` calls `Item.inventoryTick`, for every stack, each tick,
    /// `/tick freeze` included (it stops the world, not the player). `slot` is
    /// as `Inventory` saves it (0 to 35; the armour 100 feet to 103 head; -106
    /// the off hand), and `selected` says it is the held slot.
    /// `native_carried_stack` reads and changes the stack, which stays in its
    /// slot, so the item helpers see it: a count of 0 empties the slot, and the
    /// player's client gets the change. This runs often, so check the item
    /// first. Server-side, for every living player of the world (the helpers
    /// read the acting one). API 39.
    pub(super) fn native_inventory_tick(&mut self, slot: i32, selected: bool) {
        mod_api::dispatch_inventory_tick(self, mod_api::mods(), slot, selected);
    }
    /// A player's equipment slot `slot` (`head`, `chest`, `legs`, `feet`,
    /// `offhand`, and `mainhand` on a swap of the hands or `/item`) took a different
    /// item, `new` (none when it was emptied): `LivingEntity.onEquipItem`,
    /// before the equip sound. It only hears. Never for a spectator.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one). API 39.
    pub(super) fn native_on_equip(&mut self, slot: &str, new: Option<&ItemStack>) {
        mod_api::dispatch_on_equip(self, mod_api::mods(), slot, new);
    }
    /// `MobEffect.shouldApplyEffectTickThisTick` for a content mod's effect
    /// `effect` (`ruby:bleeding`), each tick a player or a mob has it, in
    /// `MobEffectInstance.tickServer` before it counts down: `tick_count` is
    /// the ticks left (the entity's tick count when infinite). Set `apply`
    /// and `native_apply_effect_tick` follows. Server-side. API 56.
    pub(super) fn native_should_apply_effect_tick_this_tick(
        &self,
        effect: &str,
        tick_count: i32,
        amplifier: i32,
        apply: &mut bool,
    ) {
        mod_api::dispatch_should_apply_effect_tick_this_tick(
            self,
            mod_api::mods(),
            effect,
            tick_count,
            amplifier,
            apply,
        );
    }
    /// `MobEffect.applyEffectTick`: the content mod's effect `effect` acts on
    /// `entity` in `dimension` (`minecraft:overworld`). Clear `keep` and the
    /// effect ends now, before it counts down. `entity` is a mob's
    /// `Entity.getId`, None for a player (`native_acting_player` says who).
    /// It runs inside the entity's tick: a mob's world borrows the game
    /// there, so the game is read-only, and it does not hold the ticking
    /// mobs, nor the player's effects. Server-side. API 56.
    pub(super) fn native_apply_effect_tick(
        &self,
        effect: &str,
        dimension: &str,
        entity: Option<i32>,
        amplifier: i32,
        keep: &mut bool,
    ) {
        mod_api::dispatch_apply_effect_tick(
            self,
            mod_api::mods(),
            effect,
            dimension,
            entity,
            amplifier,
            keep,
        );
    }
    /// `MobEffect.onEffectAdded`: `LivingEntity.addEffect` gave `entity`
    /// (as in `native_apply_effect_tick`) the content mod's effect `effect`,
    /// which it didn't have. Server-side. API 56.
    pub(super) fn native_on_effect_added(
        &mut self,
        effect: &str,
        entity: Option<i32>,
        amplifier: i32,
    ) {
        mod_api::dispatch_on_effect_added(self, mod_api::mods(), effect, entity, amplifier);
    }
    /// NeoForge's `MobEffectEvent.Added`, vanilla's `MobEffect.onEffectStarted`:
    /// every `LivingEntity.addEffect` of the content mod's effect `effect`,
    /// new or not, changed or not, after `native_on_effect_added`.
    /// Server-side. API 56.
    pub(super) fn native_mob_effect_added(
        &mut self,
        effect: &str,
        entity: Option<i32>,
        amplifier: i32,
    ) {
        mod_api::dispatch_mob_effect_added(self, mod_api::mods(), effect, entity, amplifier);
    }
    /// `LivingEntity.hurtServer`, past its guards (invulnerable, dead, fire
    /// with fire resistance), is about to hurt entity `entity` of type `kind`
    /// (`minecraft:zombie`, `minecraft:player`) by `amount` of `damage_type`
    /// (`minecraft:fall`), from the source's `causing` entity
    /// (`DamageSource.getEntity`: the shooter of an arrow) and `direct` one
    /// (`getDirectEntity`: the arrow), 0 for none or not known yet
    /// (docs/native-mods.md, API 50). Change `amount`, or set `canceled`,
    /// and nothing of `hurtServer` itself happens (no damage, hurt sound,
    /// knockback, invulnerable time, statistics or criteria); the caller acts
    /// as on a refused hit, except that a canceled hit on the ender dragon
    /// still counts as landed (`EnderDragon.hurt` returns true whatever
    /// `reallyHurt` did: the arrow is spent, the melee effects run). It runs
    /// on the thread that ticks the world (the server thread): state shared with render thread hooks needs synchronising.
    /// No `self`: the game is in the middle of the hit. API 50.
    pub(super) fn native_living_incoming_damage(
        entity: i32,
        kind: &str,
        damage_type: &str,
        causing: i32,
        direct: i32,
        amount: &mut f32,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_living_incoming_damage(
            mod_api::mods(),
            entity,
            kind,
            damage_type,
            causing,
            direct,
            amount,
            canceled,
        );
    }
    /// `LivingEntity.knockback` is about to push entity `entity` of type `kind`
    /// (`minecraft:zombie`, `minecraft:player`) `strength` away from the direction (`x`, `z`),
    /// on the raw strength, before the entity's knockback resistance takes its share and its motion
    /// changes
    /// (docs/native-mods.md, API 125). Vanilla's direction points at the
    /// source of a hit (the attacker's side), and the entity is pushed the other way. Change
    /// `strength`, `x` or `z`, or set `canceled`, and the push follows them: canceled, the
    /// entity's motion is left alone. A `strength` of 0 or less after the hook is no push, and a
    /// direction too short to tell draws a random one after it, as in vanilla. A value a mod
    /// leaves that is not a finite number is put back as it was. `damage_type`
    /// (`minecraft:player_attack`), `causing` (`DamageSource.getEntity`) and `direct`
    /// (`getDirectEntity`) are the push's `DamageSource`, named as `native_living_incoming_damage`
    /// names a hit's, with the same entity ids left at 0. It fires for the players and mobs the
    /// game knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a
    /// shield's push on its holder and a goat's ram. It does not fire for a sulfur cube with a
    /// block that a hit with a causing entity knocks back (`SulfurCube.knockback` does not call
    /// `super` there), for the pushes vanilla doesn't make with `LivingEntity.knockback` (the
    /// ender dragon's wings, Punch arrows, explosions, wind charges, a mace smash's area, a
    /// hoglin's throw), nor yet for the ones Horizon makes another way: a mob's melee push from
    /// its attack knockback or a Knockback weapon, an armor stand's, the ender dragon's. It runs
    /// on the thread that ticks the world (the server thread): state
    /// shared with render thread hooks needs synchronising. No `self`: the game is in the middle
    /// of the hit. API 125.
    pub(super) fn native_living_knockback(
        entity: i32,
        kind: &str,
        damage_type: &str,
        causing: i32,
        direct: i32,
        strength: &mut f64,
        x: &mut f64,
        z: &mut f64,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_living_knockback(
            mod_api::mods(),
            entity,
            kind,
            damage_type,
            causing,
            direct,
            strength,
            x,
            z,
            canceled,
        );
    }
    /// Whether a mod hears projectiles' impacts (`native_projectile_impact`):
    /// set `hears`. Asked once a server tick before the projectiles fly; only
    /// then does the tick keep the copy of each flying projectile that a
    /// canceled impact flies on from, so a game without such a mod pays
    /// nothing. Server-side. API 136.
    pub(super) fn native_hears_projectile_impacts(&self, hears: &mut bool) {
        mod_api::dispatch_hears_projectile_impacts(self, mod_api::mods(), hears);
    }
    /// A projectile hit something (NeoForge's `ProjectileImpactEvent`, posted
    /// where vanilla's projectile has found what it meets, before the hit
    /// acts): `projectile` (its entity id) of type `kind` (`minecraft:arrow`),
    /// shot by `owner` (a mob's or a player's entity id, 0 for none) in the level
    /// `dimension`. It hit the entity `entity` of type `entity_kind` (a player is
    /// `minecraft:player`; a dragon's part is `minecraft:ender_dragon`), or, with
    /// `entity` None, the block at `block` on its `face` (the face's normal,
    /// `[0, 1, 0]` for the top), at the point `at`. Set `canceled` and the impact
    /// does not happen: no damage, no knockback, no sound, no block reaction, the
    /// arrow does not stick, a pearl does not teleport, a potion or a fireball
    /// does not burst. The projectile flies on and is not discarded, so a mod
    /// that cancels also decides what becomes of it. It continues through its
    /// whole motion and may hit something on a later tick. Covers what
    /// `tick_projectiles` flies: arrows, tridents, snowballs,
    /// eggs, pearls, potions, fireballs, wither skulls, wind charges, llama spit and
    /// shulker bullets, from players, mobs and dispensers. It does not fire for
    /// a breeze's deflection, for a hit on a wind charge, for fireworks or for
    /// fishing bobbers. A dragon part or End crystal is told by the id of the
    /// dragon or crystal. Heard only when a mod also sets
    /// `native_hears_projectile_impacts`. Server-side. API 136.
    pub(super) fn native_projectile_impact(
        &mut self,
        dimension: &str,
        projectile: u32,
        kind: &str,
        owner: u32,
        entity: Option<u32>,
        entity_kind: &str,
        block: [i32; 3],
        face: [i32; 3],
        at: [f64; 3],
        canceled: &mut bool,
    ) {
        mod_api::dispatch_projectile_impact(
            self,
            mod_api::mods(),
            dimension,
            projectile,
            kind,
            owner,
            entity,
            entity_kind,
            block,
            face,
            at,
            canceled,
        );
    }

    /// `Block.stepOn` for a mod block, on its authoritative server level.
    /// No self: the simulation owns the entity during this callback.
    pub(super) fn native_block_step_on(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockStateId,
        position: [i32; 3],
    ) {
        mod_api::dispatch_block_step_on(mod_api::mods(), dimension, entity, kind, state, position);
    }
    /// `Block.entityInside` for each mod block touched along the entity's movement.
    /// Repeated sweep hits in one contact pass are suppressed; idle overlap still fires.
    pub(super) fn native_block_entity_inside(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockStateId,
        position: [i32; 3],
    ) {
        mod_api::dispatch_block_entity_inside(
            mod_api::mods(),
            dimension,
            entity,
            kind,
            state,
            position,
        );
    }
    /// `Block.fallOn` for a mod block. Change calculated damage before generic fall sounds
    /// and hurt handling; zero prevents both. Creative and fall-immune entities
    /// notify with zero damage and retain their immunity. Server-side only.
    pub(super) fn native_block_fall_on(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockStateId,
        position: [i32; 3],
        distance: f64,
        damage: &mut f32,
    ) {
        mod_api::dispatch_block_fall_on(
            mod_api::mods(),
            dimension,
            entity,
            kind,
            state,
            position,
            distance,
            damage,
        );
    }
}

/// `native_living_knockback` for the knockback code (`health::KNOCKBACK`): whether it
/// canceled the push. Its one caller. The pushed entity and the source are named as the
/// damage hook names them.
pub(super) fn knockback(pushed: Hurting<'_>, knock: &mut Knock) -> bool {
    let (entity, kind, source) = hurt_server_args(pushed);
    let HurtSource {
        damage_type,
        causing,
        direct,
    } = source;
    let mut canceled = false;
    Game::native_living_knockback(
        entity,
        kind,
        damage_type,
        causing,
        direct,
        &mut knock.strength,
        &mut knock.x,
        &mut knock.z,
        &mut canceled,
    );
    canceled
}

pub(super) fn block_contact(
    dimension: &str,
    event: horizon_client::block_contact::Event<'_>,
    damage: &mut f32,
) {
    use horizon_client::block_contact::Event;
    match event {
        Event::StepOn(c) => Game::native_block_step_on(
            dimension,
            c.entity,
            c.kind,
            c.state,
            [c.position.x, c.position.y, c.position.z],
        ),
        Event::Inside(c) => Game::native_block_entity_inside(
            dimension,
            c.entity,
            c.kind,
            c.state,
            [c.position.x, c.position.y, c.position.z],
        ),
        Event::FallOn(c, distance) => Game::native_block_fall_on(
            dimension,
            c.entity,
            c.kind,
            c.state,
            [c.position.x, c.position.y, c.position.z],
            distance,
            damage,
        ),
    }
}

/// `native_living_incoming_damage` for the damage code (`health::HURT_SERVER`): whether
/// it canceled the hit. Its one caller.
pub(super) fn hurt_server(hurting: Hurting<'_>, amount: &mut f32) -> bool {
    let (entity, kind, source) = hurt_server_args(hurting);
    let HurtSource {
        damage_type,
        causing,
        direct,
    } = source;
    let mut canceled = false;
    Game::native_living_incoming_damage(
        entity,
        kind,
        damage_type,
        causing,
        direct,
        amount,
        &mut canceled,
    );
    canceled
}
/// `native_living_incoming_damage`'s entity, entity type and source for a hit.
fn hurt_server_args(hurting: Hurting<'_>) -> (i32, &str, HurtSource<'_>) {
    match hurting {
        Hurting::Player(entity, source) => (entity, "minecraft:player", source),
        Hurting::Mob(mob, damage) => (mob.id as i32, mob.kind.id.as_str(), mob_source(damage)),
        Hurting::Dragon(entity, source) => (entity, "minecraft:ender_dragon", source),
    }
}
/// A mob's damage as its `DamageSource`: the causing entity is known when it
/// is a mob, and is the direct one too when it deals the damage itself.
fn mob_source(damage: &Damage) -> HurtSource<'static> {
    let causing = match damage.attacker {
        Some(Who::Mob(id)) => id as i32,
        _ => 0,
    };
    let melee = super::creatures::melee(damage.kind);
    HurtSource {
        damage_type: super::creatures::source_damage_type(*damage),
        causing,
        direct: if melee { causing } else { 0 },
    }
}

/// Whether `id` is a sound event's id, `namespace:path` as `Identifier`
/// allows: the namespace of `[a-z0-9_.-]`, the path also with `/`.
fn sound_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    let allowed = |c: char, slash: bool| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c) || (slash && c == '/')
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.chars().all(|c| allowed(c, false))
        && path.chars().all(|c| allowed(c, true))
}

/// Sets the viewer counts `known` holds for `dim` to `now` and returns the
/// changes as `(pos, previous, viewers)`, in position order. A container that
/// is no longer counted goes to 0 and leaves `known`.
pub(super) fn viewer_changes(
    known: &mut HashMap<(&'static str, (i32, i32, i32)), u32>,
    dim: &'static str,
    mut now: HashMap<(i32, i32, i32), u32>,
) -> Vec<((i32, i32, i32), u32, u32)> {
    let mut changes = Vec::new();
    known.retain(|&(d, pos), previous| {
        if d != dim {
            return true;
        }
        let viewers = now.remove(&pos).unwrap_or(0);
        if viewers != *previous {
            changes.push((pos, *previous, viewers));
            *previous = viewers;
        }
        viewers > 0
    });
    for (pos, viewers) in now {
        known.insert((dim, pos), viewers);
        changes.push((pos, 0, viewers));
    }
    changes.sort_unstable();
    changes
}

fn dimension_id(dimension: LevelDimension) -> &'static str {
    match dimension {
        LevelDimension::Overworld => "minecraft:overworld",
        LevelDimension::Nether => "minecraft:the_nether",
        LevelDimension::End => "minecraft:the_end",
    }
}

/// Server-level helpers for native mods.
#[allow(dead_code)]
impl GameLevel<'_> {
    /// Play a sound event for nearby players in this server level. The return
    /// value means queued, not heard. A `minecraft:` event must be registered:
    /// an unknown one is dropped at dispatch. Another namespace's event (a
    /// mod's, `mymod:machine_hum`) is sent by name, as vanilla sends a sound
    /// that isn't in its registry (`Holder.direct`, a variable range): players
    /// hear it if their resources define it (`assets/mymod/sounds.json`, a
    /// content mod's). API 61.
    pub(super) fn native_play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> bool {
        if !self.native_authoritative || !sound_id(sound) {
            return false;
        }
        if source >= horizon_client::settings::SOUND_SOURCES.len() {
            return false;
        }
        let position_ok = at
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= f64::from(i32::MAX) / 8.0);
        let gain_ok = volume.is_finite() && volume >= 0.0 && pitch.is_finite() && pitch >= 0.0;
        if !position_ok || !gain_ok {
            return false;
        }
        self.native_sounds.push(super::creatures::NativeSound {
            dimension: dimension_id(self.dimension),
            sound: sound.to_owned(),
            at,
            source: source as i32,
            volume,
            pitch,
        });
        true
    }

    /// Queue particles in this authoritative level. A zero count sends one
    /// directional particle. The return value means accepted, not delivered.
    /// Block particle types require their block state; item, trail and
    /// vibration particles are not supported by the current network dispatch.
    /// `ParticleOptions::named("mymod:spark")` sends a mod's own type
    /// (API 103): Horizon clients draw it from the sprite
    /// set in the mod's `assets/<namespace>/particles/<path>.json`, a client
    /// without that file draws nothing, and vanilla clients aren't sent it.
    pub(super) fn native_send_particles(
        &mut self,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        use ParticleType as T;
        // A native mod's own type: the name has to be namespaced, and a client
        // draws it only when its pack has the sprite set.
        if let Some(name) = &options.mod_particle {
            let namespaced = name
                .split_once(':')
                .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty());
            if !self.native_authoritative || !namespaced {
                return false;
            }
        }
        let supported = match options.ty {
            T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
                state.is_some_and(|id| self.registry.state(id).is_some())
            }
            T::Item | T::Trail | T::Vibration => false,
            T::Geyser | T::GeyserPlume | T::GeyserBase | T::GeyserPoof => options
                .geyser
                .is_some_and(|(_, impulse)| impulse.is_finite()),
            _ => true,
        };
        if !self.native_authoritative
            || !supported
            || !at.into_iter().all(f64::is_finite)
            || !spread
                .into_iter()
                .all(|v| v.is_finite() && v.abs() <= f32::MAX as f64)
            || !speed.is_finite()
            || speed.abs() > f32::MAX as f64
            || !options.color.into_iter().all(f32::is_finite)
            || !options.to_color.into_iter().all(f32::is_finite)
            || !options.scale.is_finite()
        {
            return false;
        }
        self.native_particles
            .push(super::lan_particles::QueuedParticle {
                dimension: dimension_id(self.dimension),
                options: options.clone(),
                state,
                at,
                spread,
                speed,
                count,
                randomization: 0,
            });
        true
    }
    /// Sets property `key` of the block at `pos` to `value` with
    /// `Level.setBlock(pos, state, flags)`, from a hook that gets the level's
    /// `ticks` (`native_random_tick`, `native_tick_block`). A crop's random
    /// tick grows it with `flags::UPDATE_CLIENTS`: neighbours' shapes update,
    /// but no neighbour gets `neighborChanged` and comparators aren't told;
    /// `flags::UPDATE_ALL` does both. Horizon sends every write to players
    /// whatever the flags: `UPDATE_CLIENTS`, `UPDATE_INVISIBLE` and
    /// `UPDATE_IMMEDIATE` change nothing yet. Any loaded position, not only
    /// the hook's; true when the block changed, false off an authoritative
    /// server, when the block has no such property or value, `pos` is not
    /// loaded, or the property already has `value` (no write, as
    /// `Level.setBlock` returns false for an unchanged state). Elsewhere,
    /// `Game::native_set_block_property_with` does the same. API 33.
    pub(super) fn native_set_block_property(
        &mut self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool {
        if !self.native_authoritative {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.block_state(at);
        let Some(id) = property_change(self.registry, state, key, value) else {
            return false;
        };
        let b = self.behaviours;
        b.set_block(self, ticks, at, id, flags)
    }
    /// Schedules a tick of `block` (a block name) at `pos` in `delay` game
    /// ticks, as a falling block's shape update schedules its fall
    /// (`ScheduledTickAccess.scheduleTick(pos, block, delay)`, normal
    /// priority); false for an unknown block or a position that isn't
    /// loaded. A tick of that block already queued there is kept and counts
    /// as scheduled. `ticks` is the hook's. API 32.
    pub(super) fn native_schedule_block_tick(
        &self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        block: &str,
        delay: i32,
    ) -> bool {
        let Some(state) = self.registry.default_state(block) else {
            return false;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let block = self.behaviours.block(state);
        let time = self.game_time();
        ticks.schedule_block(time, at, block, delay, TickPriority::Normal)
            != ScheduleResult::MissingChunk
    }
    /// Schedules a tick of `fluid` (`minecraft:water`, `minecraft:lava`,
    /// `minecraft:flowing_water`...) at `pos` in `delay` game ticks, as a
    /// waterlogged block's shape update schedules its water in 5
    /// (`ScheduledTickAccess.scheduleTick(pos, fluid, delay)`); false for
    /// an unknown fluid or a position that isn't loaded. A tick of that
    /// fluid already queued there is kept and counts as scheduled. `ticks` is
    /// the hook's. API 32.
    pub(super) fn native_schedule_fluid_tick(
        &self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        fluid: &str,
        delay: i32,
    ) -> bool {
        let Some(kind) = FluidKind::from_name(fluid) else {
            return false;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let time = self.game_time();
        ticks.schedule_fluid(time, at, kind, delay) != ScheduleResult::MissingChunk
    }
    /// The state `state` with its property `key` set to `value`, as a mod block
    /// builds the state it is placed in: `native_state_with_property(cable,
    /// "facing", "east")` (`BlockState.setValue`). A block's own declared
    /// properties (`state_properties` in its block file) are set the same way.
    /// `state` itself when the property already has that value; None when the
    /// block has no such property, or the property no such value. API
    /// 76.
    pub(super) fn native_state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        with_property(self.registry, state, key, value)
    }
    /// The block `state` is a state of (`BlockState.getBlock()`): its name,
    /// `mymod:cable`. `native_state_for_placement` is asked for every block
    /// placed and given the block's default state: compare this with your
    /// block to answer only for it. None for an unknown state. API
    /// 164.
    pub(super) fn native_state_block(&self, state: BlockStateId) -> Option<&str> {
        self.registry.state(state).map(|s| &*s.name)
    }
    /// This level's own random (vanilla's `Level.random`): the stream
    /// `native_level_random(dimension)` hands a server hook, for a world hook
    /// that draws as a vanilla block does (`isBonemealSuccess`), so the mod's
    /// draws shift vanilla's as a Java mod's would. `None` where this level
    /// isn't the server's copy. API 72.
    pub(super) fn native_level_random(
        &mut self,
    ) -> Option<&mut horizon_core::random::LegacyRandom> {
        self.native_authoritative.then_some(&mut self.world.random)
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Access a server-owned holder's attachment map. On a client it is the
    /// copy of what the server synced: `set` and `remove` change only the copy.
    pub(super) fn native_attachments(
        &mut self,
        holder: Holder,
    ) -> AttachmentResult<&mut Attachments> {
        self.attachment_map(holder)
    }
    /// Read a loaded, light-emitting block. No implicit chunk loading.
    pub(super) fn native_lit_block_name(&self, pos: [i32; 3]) -> Option<&str> {
        self.world
            .chunks
            .contains_key(&(pos[0].div_euclid(16), pos[2].div_euclid(16)))
            .then(|| {
                self.res
                    .facts()
                    .get(self.world.block(pos[0], pos[1], pos[2]))
            })
            .filter(|f| f.emission > 0)
            .map(|f| f.name.as_ref())
    }
    /// A small local vanilla particle; caller should rate-limit emission.
    pub(super) fn native_flame_particle(&mut self, at: [f64; 3], soul: bool) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        if !self.native_window() || at.iter().any(|v| !v.is_finite()) {
            return;
        }
        let Some(p) = self.native_player() else {
            return;
        };
        if (at[0] - p.x).powi(2) + (at[1] - p.y).powi(2) + (at[2] - p.z).powi(2) > 1024.0 {
            return;
        }
        let kind = if soul {
            ParticleType::SoulFireFlame
        } else {
            ParticleType::Flame
        };
        self.particles
            .add(&ParticleOptions::simple(kind), at, [0.0, 0.01, 0.0]);
    }
    /// The stack the acting player carries in `slot`, by
    /// `native_inventory_tick`'s numbers: none for an empty slot or another
    /// number. Change it as you like: a count of 0 empties the slot at the end
    /// of `native_inventory_tick`, and the player's client gets the change.
    /// API 39.
    pub(super) fn native_carried_stack(&mut self, slot: i32) -> Option<&mut ItemStack> {
        self.native_carried_slot(slot)?.as_mut()
    }
    /// Whether the player left mod `mod_id` on in Options > Mods. A mod
    /// turned off does nothing (check this first in each hook) until the
    /// launcher's next build leaves it out.
    pub(super) fn native_on(&self, mod_id: &str) -> bool {
        !self.settings.mods.disabled.contains(mod_id)
    }
    /// Sets a block in the current authoritative level through `Level.setBlock`.
    /// Returns false for another dimension, an invalid state, or no change.
    /// `flags` selects vanilla's neighbour and shape updates; Horizon sends
    /// every write to players whatever the flags. A block entity block gets
    /// its block entity. API 24.
    pub(super) fn native_set_block(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || self.pack.assets.registry.state(state).is_none()
        {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        let changed =
            self.with_level(|b, level, ticks| b.set_block(level, ticks, at, state, flags));
        let new = self.res.facts().get(state).name.clone();
        let landed = self.world.block(pos[0], pos[1], pos[2]);
        if changed
            && self.res.facts().get(previous).name != new
            && self.res.facts().get(landed).name == new
            && let Some(entity) = self.new_block_entity(&new)
        {
            self.world
                .block_entities
                .insert((pos[0], pos[1], pos[2]), entity);
        }
        let _ = self.process_level_events();
        changed
    }
    /// The strongest redstone signal reaching the block at `pos` from its six
    /// neighbours, 0 to 15 (`SignalGetter.getBestNeighborSignal`, what a
    /// dispenser or a piston reads), in the level in place, `dimension`. 0 on
    /// a client's copy of the world and for another dimension. A mod's
    /// explosive, launcher or machine reads it in its block entity's tick to
    /// go off or switch on. API 96.
    pub(super) fn native_neighbor_signal(&mut self, dimension: &str, pos: [i32; 3]) -> i32 {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return 0;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.best_neighbor_signal(level, ticks, at))
    }
    /// The light at `pos` in the world in place, as `(block, sky)`, each 0 to
    /// 15 (`Level.getBrightness` for `LightLayer.BLOCK` and `LightLayer.SKY`).
    /// A solar panel checks for full sky light above it, a lamp or a mob farm
    /// for darkness. `(0, 0)` in a chunk that isn't loaded. Like the other
    /// reads here it reads the world this game holds: the level in place on
    /// a server, the server's copy on a client. API 97.
    pub(super) fn native_light(&self, pos: [i32; 3]) -> (u8, u8) {
        if !self.world.loaded(pos[0], pos[2]) {
            return (0, 0);
        }
        let light = self.world.light(pos[0], pos[1], pos[2]);
        (light & 15, light >> 4)
    }
    /// The rain and thunder levels of the world in place, each 0.0 to 1.0
    /// (`Level.getRainLevel` and `Level.getThunderLevel`): what a solar panel
    /// or a wind turbine reads. Both 0 in the Nether and the End, which have
    /// no weather (`Level.canHaveWeather`). API 97.
    pub(super) fn native_weather(&self) -> (f32, f32) {
        if self.dim != super::Dim::Overworld {
            return (0.0, 0.0);
        }
        self.world.weather.levels(1.0)
    }
    /// The world's time of day in ticks (`Level.getOverworldClockTime`), the
    /// same as `native_clock`'s `day_time`; it keeps counting past 24000, so
    /// take it modulo 24000 for the hour. API 97.
    pub(super) fn native_day_time(&self) -> i64 {
        self.world.day_time
    }
    /// The lowest block y and the height of the world in place
    /// (`LevelHeightAccessor.getMinY` and `getHeight`): how deep a quarry or a
    /// miner may dig. API 97.
    pub(super) fn native_world_height(&self) -> (i32, i32) {
        (self.world.shape.min_y, self.world.shape.height)
    }
    /// The most of the item `item` one slot holds (`Item.getDefaultMaxStackSize`),
    /// or `None` for an unknown item: for a mod's container that fills or
    /// merges stacks itself. API 97.
    pub(super) fn native_max_stack_size(&self, item: &str) -> Option<u32> {
        self.res.items().get(item).map(|i| i.max_stack)
    }
    /// The random of the server level `dimension`: vanilla's `Level.random`
    /// (`level.getRandom()`), the one its explosions, block drops and other
    /// level draws use, so a mod's draws move vanilla's later ones as a Java
    /// mod's do. Draw with `horizon_core::random::RandomSource` (`next_int_bound`,
    /// `next_float`...). `None` on a client of a server, and for a level not
    /// open: the Nether and the End open when someone first goes there and
    /// close after 300 ticks with nobody in them (`close_idle_levels`). API 43.
    pub(super) fn native_level_random(
        &mut self,
        dimension: &str,
    ) -> Option<&mut horizon_core::random::LegacyRandom> {
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return None;
        }
        let dim = match dimension {
            "minecraft:overworld" => super::Dim::Overworld,
            "minecraft:the_nether" => super::Dim::Nether,
            "minecraft:the_end" => super::Dim::End,
            _ => return None,
        };
        if dim == self.dim {
            Some(&mut self.world.random)
        } else {
            self.levels
                .parked_world_mut(dim)
                .map(|world| &mut world.random)
        }
    }
    /// The item entities of the server level `dimension` whose box meets
    /// the box `min`..`max`, in the level's order: vanilla's
    /// `Level.getEntitiesOfClass(ItemEntity.class, box, ENTITY_STILL_ALIVE)`, as a
    /// hopper's `getItemsAtAndAbove` asks for its block and the one above
    /// (`[x, y, z]`..`[x + 1, y + 2, z + 1]`). Empty for a level not in place
    /// (as `native_set_block`) and on a client of a server. API 42.
    pub(super) fn native_item_entities(
        &self,
        dimension: &str,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Vec<NativeItemEntity> {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return Vec::new();
        }
        self.dropped
            .iter()
            .filter(|e| e.stack.count > 0 && box_meets(&e.bounds(), min, max))
            .map(|e| NativeItemEntity {
                id: e.id,
                position: [e.position.x, e.position.y, e.position.z],
                stack: e.stack.clone(),
                pickup_delay: e.pickup_delay,
            })
            .collect()
    }
    /// Sets the stack of item entity `id` in the server level `dimension`, as
    /// a hopper's `addItem` leaves what it couldn't take (`ItemEntity.setItem`);
    /// an empty stack (count 0) removes the entity (`setItem(EMPTY)`, then
    /// `discard`). Players see it on their next tracker update. False when the
    /// level in place has no such item entity, or on a client. API 42.
    pub(super) fn native_set_item_entity(
        &mut self,
        dimension: &str,
        id: u32,
        stack: ItemStack,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return false;
        }
        let Some(i) = self.dropped.iter().position(|e| e.id == id) else {
            return false;
        };
        if stack.count == 0 {
            self.remove_dropped(i);
        } else {
            self.dropped[i].stack = stack;
        }
        true
    }
    /// Sets property `key` of the block at `pos` to `value`, as a furnace's
    /// tick sets its `lit` (`Level.setBlock` with `UPDATE_ALL`: neighbours
    /// and clients are told); true when the block changed, false when the
    /// block has no such property or value, or the property already has
    /// `value` (as `Level.setBlock` for an unchanged state). The block
    /// entity stays. API 18.
    pub(super) fn native_set_block_property(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
    ) -> bool {
        self.native_set_block_property_with(pos, key, value, flags::UPDATE_ALL)
    }
    /// `native_set_block_property` with `Level.setBlock`'s `flags`, as a
    /// brewing stand's tick sets its bottles with `flags::UPDATE_CLIENTS`.
    /// Players get the write whatever the flags (`flags`); false when this
    /// window isn't a server, and in each case `native_set_block_property`
    /// returns false. API 33.
    pub(super) fn native_set_block_property_with(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool {
        // As `native_set_block`: a window that isn't a server holds a copy.
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return false;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let Some(id) = property_change(self.pack.assets.registry, state, key, value) else {
            return false;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, id, flags))
    }
    /// The block at `pos` changed its comparator output
    /// (`native_analog_output`) without changing its state: the comparators
    /// beside it, or behind a conductor there, read it again
    /// (`Level.updateNeighbourForOutputSignal`). Acts on the level in place:
    /// false (and nothing happens) when `dimension` is another, or this
    /// window isn't a server. API 64.
    pub(super) fn native_update_output_neighbors(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.update_neighbour_for_output_signal(level, ticks, at));
        true
    }
    /// The block at `pos` changed the signal it gives: its neighbours, and
    /// theirs (a block it powers passes the signal on), read it again, as
    /// around a lever (`Level.updateNeighborsAt`). Call it when the answer
    /// changes and once the block is placed. Acts on the level in place:
    /// false (and nothing happens) when `dimension` is another, or this
    /// window isn't a server. API 38.
    pub(super) fn native_update_signal_neighbors(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
        {
            return false;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| {
            let block = b.block(state);
            b.update_neighbors_at(level, ticks, at, block);
            for [x, y, z] in [
                [0, -1, 0],
                [0, 1, 0],
                [0, 0, -1],
                [0, 0, 1],
                [-1, 0, 0],
                [1, 0, 0],
            ] {
                b.update_neighbors_at(level, ticks, at.offset(x, y, z), block);
            }
        });
        true
    }
    /// Runs `change` on the mod block entity at `pos` (a generator charging
    /// the machine beside it) and returns what it returns; none for any
    /// other block entity, and for the one ticking now, which is out of the
    /// world. Afterwards, as its `setChanged` would, its chunk is saved next
    /// time and, when its items changed, comparators read it again. API 18.
    pub(super) fn native_with_mod_block_entity<R>(
        &mut self,
        pos: [i32; 3],
        change: impl FnOnce(&mut ModBlockEntity) -> R,
    ) -> Option<R> {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) else {
            return None;
        };
        if m.id.is_empty() {
            return None;
        }
        let items = m.items.clone();
        let out = change(m);
        let moved = m.items != items;
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        if moved {
            self.container_changed(key);
        }
        Some(out)
    }
    /// Sends clients the block at `pos` and its block entity's update tag
    /// (`native_block_entity_update_tag`) again, as `Level.sendBlockUpdated`
    /// does: with the level's other block changes, to the players who see its
    /// chunk, once however often it is called before then. Changes made in a
    /// block entity's tick go out at the start of the next tick, as vanilla's
    /// do (`ServerLevel.tick` broadcasts chunks before it ticks block
    /// entities). Call it when what clients show changed (a machine started),
    /// not for every tick's progress: a menu's data slots carry that. Nothing
    /// happens for an unloaded block. API 52.
    pub(super) fn native_block_entity_sync(&mut self, pos: [i32; 3]) {
        if !self.world.loaded(pos[0], pos[2]) {
            return;
        }
        if let Some(log) = self.world.block_log.as_mut() {
            log.push((pos[0], pos[1], pos[2]));
        }
    }
    /// Playing, with nothing over the world: keys and zoom apply.
    pub(super) fn native_playing(&self) -> bool {
        self.screen == Screen::Playing
            && self.title.is_none()
            && self.sign_edit.is_none()
            && self.book.is_none()
    }
    /// This `Game` is the window's, not a server's.
    pub(super) fn native_window(&self) -> bool {
        self.options.execution_role == ExecutionRole::Client
    }
    /// Plays sound event `sound` (`minecraft:entity.generic.explode`, or a
    /// mod's own from its `assets/<id>/sounds.json`) at `at` for this
    /// player only, `delay` ticks from now (0 to 1200, a minute): a client
    /// mod's sound that waits, as thunder heard after the flash or a blast
    /// heard when its shock wave arrives. `volume` past 1 carries farther,
    /// as vanilla's attenuation does (16 blocks times the volume). False, with
    /// nothing played, on a server or for a value out of range; an event the
    /// resources don't have plays nothing, as vanilla's. API
    /// 83.
    pub(super) fn native_play_local_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        delay: i32,
    ) -> bool {
        if !self.native_window() || !local_sound_ok(sound, at, volume, pitch, delay) {
            return false;
        }
        self.audio.play_after(sound, at, volume, pitch, delay);
        true
    }
    /// Draws one vanilla HUD layer between the mods' hooks: `native_gui_layer_pre`
    /// (which may hide it), the layer, then `native_gui_layer_post`. `None` when
    /// a mod hid the layer.
    pub(super) fn gui_layer<R>(
        &mut self,
        layer: &str,
        batch: &mut HudBatch,
        draw: impl FnOnce(&mut Self, &mut HudBatch) -> R,
    ) -> Option<R> {
        let mut canceled = false;
        self.native_gui_layer_pre(layer, batch, &mut canceled);
        let drawn = (!canceled).then(|| draw(self, batch));
        self.native_gui_layer_post(layer, batch);
        drawn
    }
    /// Whether a box in `corner` shows: F3's debug screen takes both top
    /// corners (its left and right columns), and a bottom corner while one
    /// of its charts is drawn there.
    pub(super) fn native_corner_shown(&self, corner: Corner) -> bool {
        let charts = self.debug_charts.corners(!self.has_singleplayer_server());
        !debug_takes(corner, self.debug, charts)
    }
    /// The width of `text` in GUI pixels.
    pub(super) fn native_text_width(&self, text: &str) -> f32 {
        HudBatch::text_width(&self.gui.atlas, text)
    }
    /// A rectangle in GUI pixels, `argb` (0x78000000 is the extras' box).
    pub(super) fn native_fill(&self, batch: &mut HudBatch, rect: [f32; 4], argb: u32) {
        let [x, y, w, h] = rect;
        let [a, r, g, b] = argb.to_be_bytes();
        batch.fill(&self.gui.atlas, x, y, w, h, rgba(r, g, b, a));
    }
    /// A mod's own RGBA image, uploaded into the HUD atlas slot
    /// [`MOD_IMAGE_SPRITE`](crate::hud::MOD_IMAGE_SPRITE) and drawn with
    /// `native_sprite`: its own source rectangle crops and stretches it. `rgba`
    /// is `width * height` pixels of four bytes, each side at most
    /// [`MOD_IMAGE_SIZE`](crate::hud::MOD_IMAGE_SIZE); false when it is not that
    /// size. The upload costs a texture patch of the image, so send one when the
    /// image changed, not every frame. API 355.
    pub(super) fn native_mod_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool {
        let Some(rect) = self
            .gui
            .atlas
            .sprites
            .get(crate::hud::MOD_IMAGE_SPRITE)
            .copied()
        else {
            return false;
        };
        if !mod_image_fits(width, height, rgba.len(), rect.w, rect.h) {
            return false;
        }
        let (w, h) = (self.gui.atlas.width as f32, self.gui.atlas.height as f32);
        self.map_patches.push(horizon_client::font::Patch {
            x: (rect.u0 * w).round() as u32,
            y: (rect.v0 * h).round() as u32,
            width,
            height,
            rgba: rgba.to_vec(),
        });
        true
    }
    /// Part `src` (`[x, y, w, h]` in the sprite's own pixels) of sprite
    /// `name` (vanilla's `gui/...`, or a content mod's GUI texture
    /// `mymod:gui/...`), drawn stretched into `dst` (`[x, y, w, h]` in GUI
    /// pixels). False when there is no such sprite. API 19.
    pub(super) fn native_sprite(
        &self,
        batch: &mut HudBatch,
        name: &str,
        src: [f32; 4],
        dst: [f32; 4],
    ) -> bool {
        let Some(mut r) = self.gui.atlas.sprites.get(name).copied() else {
            return false;
        };
        let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
        r.u0 += src[0] * du;
        r.v0 += src[1] * dv;
        r.u1 = r.u0 + src[2] * du;
        r.v1 = r.v0 + src[3] * dv;
        batch.rect_uv(dst[0], dst[1], dst[2], dst[3], r, u32::MAX);
        true
    }
    /// Item `item`'s icon (`mymod:machine`) with its top left at `at`,
    /// `size` GUI pixels square (16 in a slot); false for an unknown item.
    /// API 20.
    pub(super) fn native_item(
        &self,
        batch: &mut HudBatch,
        item: &str,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        if self.res.items().get(item).is_none() {
            return false;
        }
        let start = batch.vertices.len();
        self.draw_item_model(batch, &ItemStack::new(item, 1), at[0], at[1]);
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// Draw a stack and its slot decorations, except the count, at `at`,
    /// `size` GUI pixels square. False when nothing was drawn: an unknown item
    /// or an empty stack (air, or a count of 0, as `ItemStack.isEmpty`).
    /// API 313.
    pub(super) fn native_item_stack(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        // GuiGraphicsExtractor.item and itemDecorations draw nothing for an
        // empty stack.
        if &*stack.item == "minecraft:air" || stack.count == 0 {
            return false;
        }
        if self.res.items().get(&stack.item).is_none() {
            return false;
        }
        let start = batch.vertices.len();
        self.draw_item_model(batch, stack, at[0], at[1]);
        self.draw_item_decorations(batch, stack, at[0], at[1], Some(String::new()));
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// Item `item`'s icon drawn with model `model` (`minecraft:paper` as
    /// `mymod:wheel_icon`, an `item_model` of a content mod's), as `native_item`
    /// draws one; false for an unknown item. API 111.
    pub(super) fn native_item_model(
        &self,
        batch: &mut HudBatch,
        item: &str,
        model: &str,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        if self.res.items().get(item).is_none() {
            return false;
        }
        let mut stack = ItemStack::new(item, 1);
        let mut data = horizon_client::item_data::StackData::default();
        data.set_other(
            "minecraft:item_model",
            Some(horizon_world::nbt::Tag::String(model.into())),
        );
        stack.extra = Some(Box::new(data));
        let start = batch.vertices.len();
        self.draw_item_model(batch, &stack, at[0], at[1]);
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// A quad of corners `points` (in order round it) in GUI pixels, `argb`:
    /// a wedge, a slanted bar, what `native_fill`'s rectangles can't draw.
    /// API 111.
    pub(super) fn native_quad(&self, batch: &mut HudBatch, points: [[f32; 2]; 4], argb: u32) {
        let [a, r, g, b] = argb.to_be_bytes();
        batch.fill_quad(&self.gui.atlas, points, rgba(r, g, b, a));
    }
    /// Opens a screen of mod `mod_id`'s own over the world: the screen hooks
    /// fire over it as over any other, so the mod draws it in
    /// `native_screen_render_post` and takes its clicks, scroll, keys and
    /// typing there; the mouse is freed, and Escape closes it. Every mod's own
    /// screen has the `NativeScreenView.id` `mod`, so a mod's screen hooks
    /// check `native_mod_screen()` is its own before drawing. It doesn't pause
    /// a single-player game (as the inventory doesn't). True when it is open:
    /// false while another screen is (or another mod's), over the title
    /// screen, a book, a sign editor, a dialog or a prompt, while the world
    /// loads, and anywhere but the window's own client (the server thread, a
    /// dedicated server). A gamepad can close it (B, Start) but not click on
    /// it: its clicks don't reach the mouse hooks. API 111.
    pub(super) fn native_mod_screen_open(&mut self, mod_id: &str) -> bool {
        // What `screen_hooks_open` would refuse over the world: the mod's
        // screen would be up with no hook to draw or click it.
        let blocked = self.title.is_some()
            || self.renderer.is_none()
            || self.lan_acting.is_some()
            || self.player.is_none()
            || self.screen_over_playing()
            || self.remote_loading();
        let owner = self.native_mod_screen();
        if let Some(open) = mod_screen_opens(self.screen, blocked, owner, mod_id) {
            return open;
        }
        self.native_mod_screen = Some(mod_id.to_owned());
        self.screen = Screen::Mod;
        self.keys.clear();
        self.left = false;
        self.right = false;
        self.mining = None;
        self.capture(false);
        true
    }
    /// Closes mod `mod_id`'s own screen, if it is open. API 111.
    pub(super) fn native_mod_screen_close(&mut self, mod_id: &str) {
        if self.native_mod_screen().is_some_and(|id| id == mod_id) {
            self.close_mod_screen();
        }
    }
    /// The mod whose own screen is open, if one is (Escape, or another screen
    /// replacing it, closes it). API 111.
    pub(super) fn native_mod_screen(&self) -> Option<&str> {
        self.native_mod_screen
            .as_deref()
            .filter(|_| self.screen == Screen::Mod)
    }
    /// A mod's own screen closes (Escape, or `native_mod_screen_close`).
    pub(super) fn close_mod_screen(&mut self) {
        self.native_mod_screen = None;
        if self.screen == Screen::Mod {
            self.screen = Screen::Playing;
            self.capture(true);
        }
    }
    /// The open menu's title (the block's name, or the one it was given),
    /// as plain text. API 20.
    pub(super) fn native_screen_title(&self) -> Option<String> {
        self.remote_menu_title().map(|t| t.plain())
    }
    /// A data file of the game's resources, content mods' included
    /// (`data/mymod/machines.json`), when there is one: what a mod's content
    /// says, read at runtime. Only relative paths of plain names. API 20.
    pub(super) fn native_data(&self, path: &str) -> Option<Vec<u8>> {
        let plain = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.contains('\\');
        if !path.starts_with("data/") || !path.split('/').all(plain) {
            return None;
        }
        fs::read(self.options.resources.join(path)).ok()
    }
    /// The block at `pos` (`mymod:cable`; air where nothing is loaded).
    /// API 20.
    pub(super) fn native_block(&self, pos: [i32; 3]) -> String {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.res.facts().get(state).name.to_string()
    }
    /// The most recently extracted world frame's camera, if one exists.
    pub(super) fn native_camera(&self) -> Option<NativeCamera> {
        self.native_camera_frame.clone()
    }
    /// Collision boxes in block-local coordinates with an empty collision context.
    pub(super) fn native_block_collision(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        self.block_collision_shape(pos)
            .boxes()
            .iter()
            .map(|bounds| {
                let min = bounds.min_position();
                let max = bounds.max_position();
                [min.x, min.y, min.z, max.x, max.y, max.z]
            })
            .collect()
    }
    /// Property `key` of the block at `pos` (a machine's `facing`), if it
    /// has one. API 20.
    pub(super) fn native_block_property(&self, pos: [i32; 3], key: &str) -> Option<String> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .cloned()
    }
    /// Puts `stack` into the container at `pos` (a chest, a furnace, a mod
    /// container) through its side `face`, as a hopper would, one by one, and
    /// returns what did not fit: a machine's auto-output. A composter takes
    /// one a call, and a client's copy of the world none. API 20.
    pub(super) fn native_insert_item(
        &mut self,
        pos: [i32; 3],
        face: Direction,
        stack: ItemStack,
    ) -> Option<ItemStack> {
        self.insert_stack_through(pos, face, stack)
    }
    /// The window's width and height in physical pixels, independent of GUI
    /// scale; none on a server or when no window exists. API 165.
    pub(super) fn native_physical_window_size(&self) -> Option<[u32; 2]> {
        let size = self.window.as_ref()?.inner_size();
        Some([size.width, size.height])
    }
    /// Where the mouse is, in GUI pixels. API 20.
    pub(super) fn native_cursor(&self) -> [f32; 2] {
        [self.cursor.0, self.cursor.1]
    }
    /// The size of sprite `name` in its own pixels, if there is one. API 19.
    pub(super) fn native_sprite_size(&self, name: &str) -> Option<[f32; 2]> {
        let r = self.gui.atlas.sprites.get(name)?;
        Some([r.w as f32, r.h as f32])
    }
    /// A line of text at `(x, y)` in GUI pixels, opaque `rgb`.
    pub(super) fn native_text(
        &self,
        batch: &mut HudBatch,
        text: &str,
        at: [f32; 2],
        rgb: u32,
        shadow: bool,
    ) {
        let [x, y] = at;
        let [_, r, g, b] = rgb.to_be_bytes();
        let color = rgba(r, g, b, 0xFF);
        batch.text_styled(&self.gui.atlas, &Text::literal(text), x, y, color, shadow);
    }
    /// The player, in a world.
    pub(super) fn native_player(&self) -> Option<NativePlayer> {
        let p = self.player.as_ref()?;
        Some(NativePlayer {
            x: p.position.x,
            y: p.position.y,
            z: p.position.z,
            yaw: self.yaw,
            pitch: self.pitch,
            dimension: self.dim.id(),
        })
    }
    /// The clock of the world this window shows, for client mods: its game
    /// time and time of day as the server last sent them
    /// (`ClientboundSetTimePacket`), advanced each tick between packets; none
    /// on the title screen. API 92.
    pub(super) fn native_clock(&self) -> Option<NativeClock> {
        self.player.is_some().then_some(NativeClock {
            game_time: self.world.game_time,
            day_time: self.world.day_time,
        })
    }
    /// The frame counter: frames in the last second, the slowest frame and
    /// the 1% low of the last 5 seconds, in frames per second.
    pub(super) fn native_frame_rate(&self) -> (u32, u32, u32) {
        self.frame_reading()
    }
    /// Attack and use clicks in the last second.
    pub(super) fn native_clicks(&mut self) -> (u32, u32) {
        self.mouse_reading().0
    }
    /// A vanilla mapping (`key.forward`, `key.jump`, `key.attack`…): whether
    /// it is held, and its key's label as Controls shows it.
    pub(super) fn native_mapping(&mut self, name: &str) -> Option<(bool, String)> {
        let &(_, key) = MAPPINGS.iter().find(|m| m.0 == name)?;
        let label = keybinds::display_name(self.settings.keys.get(name)).plain();
        let down = match key {
            Some(k) => self.keys.contains(&k),
            None => self.mouse_reading().1[usize::from(name == "key.use")],
        };
        Some((down, label))
    }
    /// The key the player chose for a mod's key `name`, else `default`
    /// (`key.keyboard.z`), as the WebAssembly mods' keys were kept
    /// (`key.<mod id>.<name>` in `horizon-mods.txt`).
    pub(super) fn native_mod_key(&self, mod_id: &str, name: &str, default: &str) -> String {
        let chosen = self
            .settings
            .mods
            .keys
            .get(mod_id)
            .and_then(|k| k.get(name));
        chosen.map_or(default, String::as_str).to_owned()
    }
    /// A line in chat from the client.
    pub(super) fn native_chat(&mut self, text: &str) {
        let focused = self.screen == Screen::Chat;
        let line = Text::literal(text);
        let atlas = &self.gui.atlas;
        self.ui
            .chat
            .add(atlas, line, None, ChatSource::Client, self.ticks, focused);
    }
    /// The instance (game) folder, which its players share: `saves/`,
    /// `resourcepacks/`, `shaderpacks/` and `horizon-mods/` are in it, and the
    /// title's screens create worlds and read packs there. A world's own
    /// `datapacks/` is in that world's folder (`saves/<world>/datapacks`).
    /// The folder the game runs in (`--player-dir` moves a player's options
    /// and servers only, not the instance). Nothing is created. API 326.
    pub(super) fn native_instance_dir(&self) -> PathBuf {
        Self::game_dir()
    }
    /// A native mod's own folder: `horizon-mods/.horizon-mod-data/<id>/`,
    /// where the WebAssembly mod of the same id kept its storage, so a
    /// player's settings carry over. Not created here.
    pub(super) fn native_data_dir(&self, mod_id: &str) -> PathBuf {
        let mods = Self::game_dir().join(mod_list::DIR);
        mods.join(".horizon-mod-data").join(mod_id)
    }
    /// The folder mod `mod_id`'s post-build step wrote into when the launcher built the game
    /// (`native-data/<id>/` in the build's version folder); None when the game's own path is
    /// unknown. API 41.
    pub(super) fn native_built_data(&self, mod_id: &str) -> Option<PathBuf> {
        horizon_client::native_data::built(mod_id)
    }
    /// A mod's setting `key` (one file per key in `native_data_dir`).
    pub(super) fn native_setting(&self, mod_id: &str, key: &str) -> Option<String> {
        let path = self.native_data_dir(mod_id).join(key);
        fs::read_to_string(path).ok().map(|v| v.trim().to_owned())
    }
    /// Keeps a mod's setting `key`.
    pub(super) fn native_set_setting(&self, mod_id: &str, key: &str, value: &str) {
        let dir = self.native_data_dir(mod_id);
        let _ = fs::create_dir_all(&dir).and_then(|()| fs::write(dir.join(key), value));
    }
    /// The game's `config/` folder beside `options.txt`, shared by every mod
    /// as on Fabric and NeoForge: a mod names its own files in it
    /// (`config/<id>.json`, or a folder of its own). The same folder on the
    /// window, its server thread and a LAN host; a dedicated server's is in
    /// its own folder, beside `server.properties`. Made when missing. No
    /// `self`, so a mixin helper calls `Game::native_config_dir()` and a trait
    /// mod `Host::config_dir`. API 26.
    pub(super) fn native_config_dir() -> Result<PathBuf, String> {
        config_dir(&Self::game_dir())
    }
    /// Shows the panel beside container screens with these items (at most
    /// 8192), as the game's own slots in pages, with a search box; the
    /// player's clicks, searches and keys over items come back as
    /// `native_panel_event`. The panel is the last mod's to show one; empty
    /// items and no recipe page take yours away.
    pub(super) fn native_panel_items(&mut self, mod_id: &str, items: Vec<NativeStack>) {
        self.panel.show_items(mod_id, items);
    }
    /// Shows these recipes in the panel instead of its items, as many as
    /// fit a page, under `title` and a back button, with a row of `tabs`
    /// (each an icon; `tab` is the selected one) between them. At most 256
    /// recipes and 16 tabs; no recipes shows the items again. The same title
    /// and tab again keep the page the player turned to.
    pub(super) fn native_panel_recipes(
        &mut self,
        mod_id: &str,
        title: &str,
        tabs: Vec<NativeStack>,
        tab: u32,
        views: Vec<NativeRecipeView>,
    ) {
        let page = RecipePage::new(title, tabs, tab, views);
        self.panel.show_recipes(mod_id, page);
    }
    /// The recipes of type `kind` (`mymod:crusher`), a type outside `minecraft`,
    /// from `data/<namespace>/recipe/*.json` of the content mods (`.hmod`) and
    /// the world's data packs, in file order: each one's id and its file as
    /// JSON, for the mod to read as it likes. A data pack changes them on
    /// `/reload`, so look them up again when you need them. Empty for a type
    /// nobody uses and outside a world. These are the server's table on the
    /// server thread, a dedicated server or a LAN host; a client, the
    /// single-player window included, sees only the content mods' recipes, not
    /// the world's data packs', unless a mod had the server send it the type
    /// (`native_datapack_sync`). API 36.
    pub(super) fn native_recipes_of_type(&self, kind: &str) -> &[CustomRecipe] {
        if self.outside_world() {
            return &[];
        }
        self.recipes().custom.get(kind).map_or(&[], Vec::as_slice)
    }
    /// The validated category and icon of a custom recipe type, or None for
    /// an undeclared/invalid type, before resources load, or outside a world.
    /// Rebuilt when the recipe table changes. Does not unlock any recipes.
    pub(super) fn native_recipe_book_category(&mut self, kind: &str) -> Option<RecipeBookCategory> {
        if self.outside_world() || self.res.rest.get().is_none() {
            return None;
        }
        self.registered_recipe_book_category(kind)
    }
    /// An ingredient as a recipe's JSON names it: an item, a `#tag`, or a list
    /// of them, a bare id as `minecraft:` (as vanilla reads it); `test` says
    /// whether a stack fits. Items the game doesn't know are left out; None
    /// when nothing is left (an unknown or empty tag, for one) and outside a
    /// world. API 36.
    pub(super) fn native_recipe_ingredient(&self, json: &serde_json::Value) -> Option<Ingredient> {
        if self.outside_world() {
            return None;
        }
        let qualify = |s: &str| match s.strip_prefix('#') {
            Some(tag) => String::from("#") + &qualified(tag),
            None => qualified(s),
        };
        let json = match json {
            serde_json::Value::String(s) => serde_json::Value::String(qualify(s)),
            serde_json::Value::Array(a) => serde_json::Value::Array(
                a.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(|s| serde_json::Value::String(qualify(s)))
                    .collect(),
            ),
            _ => return None,
        };
        let items = self.items();
        let known: Vec<String> = Ingredient::parse(&json, items)?
            .order
            .into_iter()
            .filter(|item| items.get(item).is_some())
            .collect();
        (!known.is_empty()).then(|| Ingredient::of(known))
    }
    /// A stack as a recipe's `result` names it: `id` (a bare one as
    /// `minecraft:`), `count` (1 by default, up to 99) and `components`. None
    /// without an `id`, for an item the game doesn't know or a count outside
    /// that, and outside a world. API 36.
    pub(super) fn native_recipe_result(&self, json: &serde_json::Value) -> Option<ItemStack> {
        if self.outside_world() {
            return None;
        }
        let mut json = json.clone();
        if let Some(id) = json["id"].as_str() {
            json["id"] = qualified(id).into();
        }
        recipe_result(&json).filter(|s| self.items().get(&s.item).is_some())
    }
    /// No world is open: a client without a player (the title screen). A
    /// server's `Game` (the server thread, a dedicated server, a LAN host)
    /// has no player of its own, and is always in its world.
    fn outside_world(&self) -> bool {
        self.player.is_none() && self.options.execution_role.is_client()
    }
    /// Every item and recipe the game knows, made once (the same for every
    /// call); none outside a world. They are the window's own (the content
    /// mods' and vanilla's), with the server's recipes of the types a mod had
    /// it send (`native_datapack_sync`) in their place; made again when those
    /// arrive (`native_recipes_received`).
    pub(super) fn native_recipes(&mut self) -> Option<Arc<NativeRecipes>> {
        self.player.as_ref()?;
        self.made_recipes()
    }
    /// The `book_id`s of the recipes the player's recipe book holds now
    /// (the server's book on a server); empty outside a world.
    pub(super) fn native_known_recipes(&mut self) -> Arc<[i32]> {
        if self.player.is_none() {
            return Arc::from([]);
        }
        self.known_displays()
    }
    /// Places a recipe in the open screen's grid as a click in the recipe
    /// book does (`ServerboundPlaceRecipePacket`): the inventory's grid or a
    /// crafting table's. `all` places as many as the items allow, as with
    /// Shift. Refused while no such screen is open and for a recipe the
    /// player's book doesn't hold; one missing items shows as a ghost.
    pub(super) fn native_place_recipe(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        self.recipe_place(book_id, all)
    }
    /// Creative mode only: gives the player `count` of `item`
    /// (`minecraft:diamond`) as taking it from the creative inventory does
    /// (`ServerboundSetCreativeModeSlot`): the selected slot first, then the
    /// hotbar, then the inventory. The count is cut to one stack.
    pub(super) fn native_give(&mut self, item: &str, count: u32) -> Result<(), String> {
        self.recipe_give(item, count)
    }
}

/// A mob's kind as the hook points name it (`minecraft:zombie`).
pub(super) fn mob_kind(mob: &horizon_client::mobs::Mob) -> String {
    ["minecraft:", mob.kind.id.trim_start_matches("minecraft:")].concat()
}

/// Where the game calls the hook points.
impl Game<'_> {
    /// The acting player as the hook points name one: the guest whose turn
    /// it is, else the host.
    pub(super) fn native_acting_id(&mut self) -> u128 {
        horizon_client::lan::playerdata::ints_uuid(self.acting_player_uuid())
    }
    /// The mods' say on a broken block's drops (`native_loot_drops`).
    pub(super) fn native_block_drops(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        player: Option<u128>,
        tool: Option<&ItemStack>,
        explosion: Option<f32>,
        drops: &mut Vec<ItemStack>,
    ) {
        let items = match &self.world_tables {
            Some(tables) => &tables.items,
            None => self.res.items(),
        };
        let table = items.block_loot_table(self.pack.assets.registry, state);
        let context = NativeLootDrops {
            table: table.as_deref().unwrap_or_default(),
            dimension: self.dim.id(),
            origin: pos.map(|c| f64::from(c) + 0.5),
            block: Some((pos, state)),
            entity: None,
            player,
            tool,
            explosion,
            used: false,
        };
        self.native_loot_drops(&context, drops);
        drops.retain(|stack| stack.count > 0 && &*stack.item != "minecraft:air");
    }
    /// The mods' say on what the acting player's use drops
    /// (`NativeLootDrops::used`): `table` is the loot table rolled, if any.
    pub(super) fn native_use_drops(
        &mut self,
        table: &str,
        origin: [f64; 3],
        block: Option<([i32; 3], BlockStateId)>,
        entity: Option<(u32, &str)>,
        tool: Option<&ItemStack>,
        drops: &mut Vec<ItemStack>,
    ) {
        let player = Some(self.native_acting_id());
        let context = NativeLootDrops {
            table,
            dimension: self.dim.id(),
            origin,
            block,
            entity,
            player,
            tool,
            explosion: None,
            used: true,
        };
        self.native_loot_drops(&context, drops);
        drops.retain(|stack| stack.count > 0 && &*stack.item != "minecraft:air");
    }
    /// Whether a native mod keeps the acting player from taking `stack`.
    pub(super) fn native_pickup_refused(&mut self, stack: &ItemStack) -> bool {
        let player = self.native_acting_id();
        let mut canceled = false;
        self.native_item_picked_up(player, &stack.item, stack.count, &mut canceled);
        canceled
    }
    /// Tells the mods of the chunks that came and went in this level.
    pub(super) fn native_chunks_changed(
        &mut self,
        unloaded: &[std::sync::Arc<horizon_client::chunk::Chunk>],
    ) {
        let loaded = std::mem::take(&mut self.world.loaded_chunks);
        if self.remote.is_some() {
            return;
        }
        let dimension = self.dim.id();
        for (x, z, new) in loaded {
            self.native_chunk_load(dimension, [x, z], new);
        }
        for chunk in unloaded {
            self.native_chunk_unload(dimension, [chunk.x, chunk.z]);
        }
    }
    /// The players still here leave with the server (`PlayerList.removeAll`):
    /// the mods hear it once, whichever path closes it.
    pub(super) fn native_players_leaving(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if std::mem::replace(&mut host.players_told_left, true) {
            return;
        }
        let players: Vec<(u128, String)> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed)
            .map(|g| (g.uuid, g.name.clone()))
            .collect();
        for (player, name) in players {
            self.native_player_logged_out(player, &name);
        }
        // No tick follows to send what they said.
        if let Some(mut host) = self.lan.take() {
            self.flush_native_net(&mut host);
            self.lan = Some(host);
        }
    }
    /// The mobs from index `from` on are new in the world.
    pub(super) fn native_mobs_joined(&mut self, from: usize, natural: bool) {
        // A mod can keep a mob out; it is then taken out and not told of.
        self.native_joins_flush(false, 0);
        for i in from..self.mobs.len() {
            let m = &self.mobs[i];
            let (kind, pos) = (mob_kind(m), [m.position.x, m.position.y, m.position.z]);
            self.native_mob_spawned(self.dim.id(), &kind, pos, natural);
        }
    }
    /// The acting player's blow killed mob `index`.
    pub(super) fn native_kill_event(&mut self, index: usize) {
        let (entity, kind) = (self.mobs[index].id, mob_kind(&self.mobs[index]));
        let player = self.native_acting_id();
        self.native_mob_killed(player, entity, &kind);
    }
    /// The acting player's hit landed on mob `index`, holding `stack`.
    pub(super) fn native_hurt_enemy_event(&mut self, stack: &ItemStack, index: usize) {
        let mob = &self.mobs[index];
        let kind = ["minecraft:", mob.kind.id.trim_start_matches("minecraft:")].concat();
        self.native_hurt_enemy(stack, mob.id, &kind);
    }
    /// `Inventory.tick` for the acting player: each carried stack goes through
    /// `native_inventory_tick`.
    pub(super) fn native_inventory_ticks(&mut self) {
        if self.player.is_none() {
            return;
        }
        for i in 0..self.inventory.len() {
            self.native_slot_tick(i as i32, i == self.selected);
        }
        // Head first here; `Inventory` saves feet as 100 and head as 103.
        for i in 0..self.armor.len() {
            self.native_slot_tick(103 - i as i32, false);
        }
        self.native_slot_tick(-106, false);
    }
    fn native_slot_tick(&mut self, slot: i32, selected: bool) {
        if self.native_carried_slot(slot).is_none_or(|s| s.is_none()) {
            return;
        }
        self.native_inventory_tick(slot, selected);
        if let Some(stack) = self.native_carried_slot(slot)
            && stack.as_ref().is_some_and(|s| s.count == 0)
        {
            *stack = None;
        }
    }
    /// The slot a player's carried stack is in, by `native_inventory_tick`'s
    /// numbers: none for another number.
    fn native_carried_slot(&mut self, slot: i32) -> Option<&mut Option<ItemStack>> {
        match slot {
            0.. if (slot as usize) < self.inventory.len() => self.inventory.get_mut(slot as usize),
            100..=103 => self.armor.get_mut(103 - slot as usize),
            -106 => Some(&mut self.offhand),
            _ => None,
        }
    }
    /// `EntityBlock.newBlockEntity` of block `name`: Horizon's own, else a
    /// native mod's.
    pub(super) fn new_block_entity(&mut self, name: &str) -> Option<BlockEntity> {
        // `SignBlock.newBlockEntity`: a mod's sign is one by its template's class.
        if !name.starts_with("minecraft:")
            && let Some(state) = self.pack.assets.registry.default_state(name)
            && let Some(form) = self.res.facts().get(state).sign_form()
        {
            return Some(BlockEntity::Sign(horizon_client::signs::SignData {
                hanging: form.hanging,
                ..Default::default()
            }));
        }
        if let Some(entity) = BlockEntity::for_block(name) {
            return Some(entity);
        }
        let mut entity = None;
        self.native_block_entity(name, &mut entity);
        let (id, nbt) = entity?;
        Some(BlockEntity::Mod(Box::new(
            ModBlockEntityFields {
                id,
                nbt,
                ..Default::default()
            }
            .into(),
        )))
    }
    /// The containers of the mod blocks' block entities, by block state
    /// (`native_block_entity_container`), checked.
    pub(super) fn native_containers(&self) -> FxHashMap<u32, Arc<ModContainer>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModContainer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(horizon_world::registry::BlockStateId(i)) else {
                continue;
            };
            if state.name.starts_with("minecraft:") {
                continue;
            }
            let container = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut container = None;
                self.native_block_entity_container(&state.name, &mut container);
                container.map(|mut c| {
                    c.slots = c.slots.min(256);
                    let n = c.slots;
                    for list in c.faces.iter_mut().chain([&mut c.inputs, &mut c.outputs]) {
                        list.retain(|&s| s < n);
                    }
                    c.menu = c
                        .menu
                        .filter(|m| mod_menu(m).is_some_and(|(_, size)| size == n));
                    Arc::new(c)
                })
            });
            if let Some(c) = container {
                out.insert(i, Arc::clone(c));
            }
        }
        out
    }
    /// The fluid tanks of the mod blocks' block entities, by block state
    /// (`native_block_entity_fluids`), checked as the containers are.
    pub(super) fn native_fluid_tables(&self) -> FxHashMap<u32, Arc<ModFluidTanks>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModFluidTanks>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(horizon_world::registry::BlockStateId(i)) else {
                continue;
            };
            if state.name.starts_with("minecraft:") {
                continue;
            }
            let tanks = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut fluids = None;
                self.native_block_entity_fluids(&state.name, &mut fluids);
                fluids.map(|mut f| {
                    f.tanks.truncate(16);
                    // Clamped, not dropped: the face and fill lists name
                    // tanks by number, which must not shift.
                    for c in &mut f.tanks {
                        *c = (*c).clamp(1, 1_000_000);
                    }
                    let n = f.tanks.len();
                    for list in f.faces.iter_mut().chain([&mut f.inputs, &mut f.outputs]) {
                        list.retain(|&t| t < n);
                    }
                    Arc::new(f)
                })
            });
            if let Some(t) = tanks {
                out.insert(i, Arc::clone(t));
            }
        }
        out
    }
    /// The energy buffers of the mod blocks' block entities, by block state
    /// (`native_block_entity_energy`), checked as the containers are.
    pub(super) fn native_energy_tables(&self) -> FxHashMap<u32, Arc<ModEnergyBuffer>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModEnergyBuffer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(horizon_world::registry::BlockStateId(i)) else {
                continue;
            };
            if state.name.starts_with("minecraft:") {
                continue;
            }
            let buffer = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut energy = None;
                self.native_block_entity_energy(&state.name, &mut energy);
                energy.filter(|e| e.capacity >= 1).map(|mut e| {
                    e.max_insert = e.max_insert.clamp(0, e.capacity);
                    e.max_extract = e.max_extract.clamp(0, e.capacity);
                    Arc::new(e)
                })
            });
            if let Some(b) = buffer {
                out.insert(i, Arc::clone(b));
            }
        }
        out
    }
    /// Whether a native mod's block `name` has a ticking block entity.
    pub(super) fn native_ticks(&self, name: &str) -> bool {
        let mut ticks = false;
        self.native_block_entity_ticks(name, &mut ticks);
        ticks
    }
    /// A mod block's container at `key` opens its menu: its slots grown to
    /// the menu's. False for any other block.
    pub(super) fn open_mod_menu(&mut self, key: Pos) -> bool {
        let state = self.world.block(key.0, key.1, key.2).0;
        let Some(c) = self.world.block_entities.mod_container(state) else {
            return false;
        };
        let slots = c.slots;
        if c.menu.is_none() {
            return false;
        }
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) else {
            return false;
        };
        if m.items.len() < slots {
            m.items.resize(slots, None);
        }
        true
    }
    /// `ContainerOpenersCounter` for the mod containers of the level in place:
    /// everyone with one open, recounted every tick, so a change is heard
    /// whichever way an opener left (`native_mod_container_viewers`). The
    /// openers are the guests (but not a spectator, `BarrelBlockEntity.startOpen`),
    /// the host's own player, and the copper golems that opened it
    /// (`chest_openers`).
    pub(super) fn tick_mod_container_viewers(&mut self) {
        let mut now: HashMap<(i32, i32, i32), u32> = HashMap::new();
        let mut open = |game: &Self, pos: (i32, i32, i32), n: u32| {
            if n > 0 && game.mod_menu_at(pos).is_some() {
                *now.entry(pos).or_default() += n;
            }
        };
        if let Some(host) = &self.lan {
            for g in host
                .hub
                .guests
                .iter()
                .filter(|g| host.hub.here(g) && !g.spectator())
            {
                if let Some(m) = &g.menu
                    && let Slots::Block(pos) = m.slots
                {
                    open(self, pos, 1);
                }
            }
        }
        if let Some(pos) = self.container.filter(|_| !self.spectator()) {
            open(self, pos, 1);
        }
        for (&pos, &(golems, _)) in &self.world.chest_openers {
            open(self, pos, golems.max(0) as u32);
        }
        let dim = self.dim.id();
        for (pos, previous, viewers) in viewer_changes(&mut self.mod_container_viewers, dim, now) {
            self.native_mod_container_viewers(dim, [pos.0, pos.1, pos.2], viewers, previous);
        }
    }
    /// The menu type of the mod container at `key`, if it has one.
    pub(super) fn mod_menu_at(&self, key: Pos) -> Option<&'static str> {
        let state = self.world.block(key.0, key.1, key.2).0;
        let c = self.world.block_entities.mod_container(state)?;
        mod_menu(c.menu.as_deref()?).map(|(menu, _)| menu)
    }
    /// The container slots (of `size`) that the mod's screen for the block at
    /// `pos` does not show: the server's menu does not have them either, so a
    /// shift-click never moves an item where the player cannot see it.
    pub(super) fn native_hidden_slots(&self, pos: Pos, size: usize) -> Vec<usize> {
        if self.mod_menu_at(pos).is_none() {
            return Vec::new();
        }
        let block = self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name
            .to_string();
        let mut screen = None;
        self.native_screen(&block, &mut screen);
        let Some(screen) = screen else {
            return Vec::new();
        };
        (0..size)
            .filter(|&i| screen.slots.get(i).is_none_or(Option::is_none))
            .collect()
    }
    /// A menu opened on this client with its slots under `key`, on block
    /// `target`: a mod block's is laid out by its mod (`native_screen`).
    pub(super) fn open_native_screen(
        &mut self,
        key: Pos,
        target: Option<Pos>,
        window: i32,
        menu: &str,
    ) {
        self.mod_screen = None;
        let Some(t) = target else {
            return;
        };
        let block = self
            .res
            .facts()
            .get(self.world.block(t.0, t.1, t.2))
            .name
            .to_string();
        if block.starts_with("minecraft:") {
            return;
        }
        // Only when this is the menu the clicked block opens: `target` outlives the
        // use that set it, so another menu opened later (a command's, another
        // mod's) would otherwise get the last clicked machine's screen. Asked
        // here, as the window's world has no mod containers.
        let mut container = None;
        self.native_block_entity_container(&block, &mut container);
        let opens = container.and_then(|c| mod_menu(c.menu.as_deref()?).map(|(m, _)| m));
        if opens != Some(menu) {
            return;
        }
        let mut screen = None;
        self.native_screen(&block, &mut screen);
        if let Some(screen) = screen {
            self.mod_screen = Some(OpenNativeScreen {
                key,
                window,
                block,
                screen,
                data: Vec::new(),
            });
        }
    }
    /// The mod screen showing now, if any.
    fn shown_native_screen(&self) -> Option<&OpenNativeScreen> {
        self.mod_screen
            .as_ref()
            .filter(|s| self.screen == Screen::Inventory && self.container == Some(s.key))
    }
    /// A data slot of window `window`'s menu (`ContainerSetData`); false
    /// when no mod screen is showing for that window.
    pub(super) fn native_screen_data(&mut self, window: i32, id: i16, value: i16) -> bool {
        if self.shown_native_screen().is_none() {
            return false;
        }
        self.mod_screen
            .as_mut()
            .is_some_and(|s| s.set_data(window, id, value))
    }
    /// Whether a mod's screen is open for the current menu: it places its own
    /// container slots and the player's inventory, so the stock layout of the
    /// vanilla menu underneath stands back (`slot_position`).
    pub(super) fn native_screen_shown(&self) -> bool {
        self.shown_native_screen().is_some()
    }
    /// The open mod screen's height.
    pub(super) fn native_screen_height(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.height)
    }
    /// Where the open mod screen puts container slot `slot`.
    pub(super) fn native_screen_slot(&self, slot: usize) -> Option<(f32, f32)> {
        let s = self.shown_native_screen()?;
        s.screen
            .slots
            .get(slot)
            .copied()
            .flatten()
            .map(|p| (p[0], p[1]))
    }
    /// The container slots the open mod screen shows (300.. ids).
    pub(super) fn native_screen_slots(&self) -> Option<Vec<usize>> {
        let s = self.shown_native_screen()?;
        let shown = s.screen.slots.iter().enumerate();
        Some(
            shown
                .filter(|(_, p)| p.is_some())
                .map(|(i, _)| 300 + i)
                .collect(),
        )
    }
    /// How many of those slots `native_screen_slots` lists, without building
    /// it (`Game::slot_count` asks every frame).
    pub(super) fn native_screen_slot_count(&self) -> Option<usize> {
        let s = self.shown_native_screen()?;
        Some(s.screen.slots.iter().filter(|p| p.is_some()).count())
    }
    /// The first row of the player's inventory on the open mod screen.
    pub(super) fn native_screen_inventory_y(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.inventory_y)
    }
    /// The open mod screen's inventory's left column.
    pub(super) fn native_screen_inventory_x(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.inventory_x)
    }
    /// The open mod screen's width.
    pub(super) fn native_screen_width(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.screen.width)
    }
    /// The open mod screen: a plain panel with its slots' frames, the mod's
    /// drawing over it, then the title and the inventory's label. False when
    /// none is showing.
    pub(super) fn draw_native_screen(&self, batch: &mut HudBatch, ox: f32, oy: f32) -> bool {
        let Some(s) = self.shown_native_screen() else {
            return false;
        };
        let atlas = &self.gui.atlas;
        if !s.screen.own_background {
            let (w, h) = (s.screen.width, s.screen.height);
            batch.fill(atlas, ox, oy, w, h, rgba(0, 0, 0, 255));
            batch.fill(
                atlas,
                ox + 1.0,
                oy + 1.0,
                w - 2.0,
                h - 2.0,
                rgba(198, 198, 198, 255),
            );
            let container = self.native_screen_slots().unwrap_or_default();
            for i in (0..36).chain(container) {
                let (x, y) = self.slot_position(i);
                let (x, y) = (ox + x - 1.0, oy + y - 1.0);
                batch.fill(atlas, x, y, 18.0, 18.0, rgba(55, 55, 55, 255));
                batch.fill(
                    atlas,
                    x + 1.0,
                    y + 1.0,
                    17.0,
                    17.0,
                    rgba(255, 255, 255, 255),
                );
                batch.fill(
                    atlas,
                    x + 1.0,
                    y + 1.0,
                    16.0,
                    16.0,
                    rgba(139, 139, 139, 255),
                );
            }
        }
        self.native_screen_draw(&s.block, &s.data, batch, [ox, oy]);
        if s.screen.own_labels {
            return true;
        }
        let grey = rgba(64, 64, 64, 255);
        if let Some(title) = self.remote_menu_title() {
            batch.text_styled(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
        }
        let inventory = self.translate("container.inventory", "Inventory");
        let label_y = oy + s.screen.inventory_y - 11.0;
        batch.text(atlas, &inventory, ox + 8.0, label_y, grey, false);
        true
    }
    /// The tick of the mod block entity at `pos`.
    pub(super) fn tick_mod_block_entity(&mut self, pos: Pos) {
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos) else {
            return;
        };
        // An empty id marks its place while the mod has it.
        let mut taken = std::mem::take(&mut **m);
        let items = taken.items.clone();
        let state = self.world.block(pos.0, pos.1, pos.2).0;
        let slots = self
            .world
            .block_entities
            .mod_container(state)
            .map_or(0, |c| c.slots);
        // Grown only: items a container no longer has room for stay saved.
        if taken.items.len() < slots {
            taken.items.resize(slots, None);
        }
        // Without a container, an `Items` item list is the mod's own field.
        if slots == 0 && !taken.items.is_empty() {
            let items = horizon_client::save::encode_items(&std::mem::take(&mut taken.items));
            taken.nbt.insert("Items".into(), items);
        }
        self.native_block_entity_tick([pos.0, pos.1, pos.2], &mut taken);
        self.put_back_mod_block_entity(pos, taken, &items);
    }
    /// Puts back a mod block entity taken out for a call, unless the call
    /// removed or replaced it, with what its `setChanged` does.
    pub(super) fn put_back_mod_block_entity(
        &mut self,
        pos: Pos,
        mut taken: ModBlockEntity,
        items: &[Option<ItemStack>],
    ) {
        if let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos)
            && m.id.is_empty()
        {
            let moved = taken.items != items;
            **m = taken;
            // BlockEntity.setChanged: the next save writes its chunk, and
            // comparators read a container whose items changed.
            self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
            if moved {
                self.container_changed(pos);
            }
        } else if taken.items.iter().any(Option::is_some) {
            // The mod removed or replaced its own block during the call. Vanilla
            // ticks it in the world and drops a container's contents on removal.
            let lost = std::mem::take(&mut taken.items);
            let at = BlockPos {
                x: pos.0,
                y: pos.1,
                z: pos.2,
            };
            self.with_level(|_, level, _| level.drop_lost_contents(at, lost));
            self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        }
    }
    /// Menu button `id` on the mod container at `pos` (`clickMenuButton`),
    /// with the acting player's cursor stack (`carried`, read and written).
    pub(super) fn mod_menu_button(&mut self, pos: Pos, id: i32, carried: &mut Option<ItemStack>) {
        if self.mod_menu_at(pos).is_none() {
            return;
        }
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&pos) else {
            return;
        };
        if m.id.is_empty() {
            return;
        }
        let mut taken = std::mem::take(&mut **m);
        let items = taken.items.clone();
        self.native_menu_button([pos.0, pos.1, pos.2], &mut taken, id);
        self.native_menu_button_v2([pos.0, pos.1, pos.2], &mut taken, id, carried);
        self.put_back_mod_block_entity(pos, taken, &items);
    }
    /// The cursor from the open mod screen's top left, with its block and
    /// data, for its press and tooltip hooks.
    fn native_screen_at(&self) -> Option<(String, Vec<i32>, [f32; 2])> {
        let s = self.shown_native_screen()?;
        let r = self.renderer.as_ref()?;
        let batch = HudBatch::new([r.config.width, r.config.height]);
        let (ox, oy) = self.inventory_origin(&batch);
        let at = [self.cursor.0 - ox, self.cursor.1 - oy];
        Some((s.block.clone(), s.data.clone(), at))
    }
    /// A press on the open mod screen's own button (`native_screen_click`)
    /// goes to the server as a menu button click (true).
    pub(super) fn native_screen_press(&mut self, right: bool) -> bool {
        if self.inventory_target().0.is_some() {
            return false;
        }
        let Some((block, data, at)) = self.native_screen_at() else {
            return false;
        };
        let mut button = None;
        self.native_screen_click(&block, &data, at, right, &mut button);
        let Some(button) = button else {
            return false;
        };
        let window = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map(|(w, _)| *w);
        if let Some(container) = window {
            self.send(
                horizon_protocol::packets::play::Serverbound::ContainerButtonClick(
                    horizon_protocol::packets::play::ContainerButtonClick { container, button },
                ),
            );
        }
        self.audio.play_button_click();
        true
    }
    /// The open mod screen's tooltip under the cursor (`native_screen_tooltip`).
    pub(super) fn draw_native_screen_tooltip(&mut self, batch: &mut HudBatch) {
        // Over no item: an empty slot still gets the mod's hint (which fuel it takes).
        let slot = self.slot_at(batch, self.cursor.0, self.cursor.1);
        if self.carried.is_some() || slot.is_some_and(|i| self.slot_view(i).is_some()) {
            return;
        }
        let Some((block, data, at)) = self.native_screen_at() else {
            return;
        };
        let mut lines = Vec::new();
        self.native_screen_tooltip(&block, &data, at, &mut lines);
        let lines: Vec<(horizon_client::text::Text, u32)> = lines
            .into_iter()
            .map(|l| (horizon_client::text::Text::literal(l), u32::MAX))
            .collect();
        self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
    }
    /// Options > Mods' rows for the native mods, after the others.
    pub(super) fn list_native_mods(&mut self) {
        let mut mods = Vec::new();
        self.native_mods(&mut mods);
        // A WebAssembly mod of the same id: the native row stands for it,
        // and alone toggles its `mod.<id>` line.
        self.settings
            .mods
            .installed
            .retain(|row| row.native.is_none() && !mods.iter().any(|m| m.id == row.id));
        for m in mods {
            let mut buttons = Vec::new();
            self.native_settings(&m.id, &mut buttons);
            let page = NativePage {
                keys: m.keys,
                buttons,
            };
            self.settings.mods.installed.push(Installed {
                id: m.id,
                title: m.title,
                description: m.description,
                problem: None,
                dev: false,
                native: Some(page),
            });
        }
    }
    /// A click on button `button` of native mod `i`'s page (its index in
    /// Options > Mods), and the page's labels after it.
    pub(super) fn native_settings_event(&mut self, i: usize, button: usize) {
        let Some(id) = self.settings.mods.installed.get(i).map(|m| m.id.clone()) else {
            return;
        };
        self.native_settings_click(&id, button);
        self.native_settings_refresh(i);
    }
    /// Native mod `i`'s page opened: its labels asked again, so that they
    /// follow the language (and the mod's state) since the list was made.
    pub(super) fn native_settings_refresh(&mut self, i: usize) {
        let Some(id) = self.settings.mods.installed.get(i).map(|m| m.id.clone()) else {
            return;
        };
        let mut buttons = Vec::new();
        self.native_settings(&id, &mut buttons);
        let row = self.settings.mods.installed.get_mut(i);
        if let Some(page) = row.and_then(|m| m.native.as_mut()) {
            page.buttons = buttons;
        }
    }
    /// A key event for `native_key`.
    pub(super) fn native_key_event(&mut self, key: KeyCode, pressed: bool, repeat: bool) {
        if repeat {
            return;
        }
        if let Some(name) = keybinds::key_name(key) {
            if pressed {
                self.native_panel_key(name);
            }
            self.native_key(name, pressed);
        }
    }
    /// `ClientHooks.onClickInput`: tells `native_interaction_key` that `button`
    /// (0 attack, 1 use, 2 pick) is about to act with `hand`. `None` when the
    /// action goes on; when a mod cancelled it, `Some` of whether the hand swings.
    pub(super) fn native_interaction(
        &mut self,
        button: i32,
        hand: horizon_client::first_person::Hand,
    ) -> Option<bool> {
        let key_mapping = match button {
            0 => "key.attack",
            1 => "key.use",
            _ => "key.pickItem",
        };
        let (mut swing_hand, mut canceled) = (true, false);
        self.native_interaction_key(button, key_mapping, hand, &mut swing_hand, &mut canceled);
        canceled.then_some(swing_hand)
    }
    /// A mouse button while playing, for `native_key`.
    pub(super) fn native_mouse_event(&mut self, button: MouseButton, pressed: bool) {
        let name = keybinds::mouse_name(button);
        self.native_key(&name, pressed);
    }
    /// Wheel steps while playing: whether a native mod took them.
    pub(super) fn native_wheel_event(&mut self, steps: f32) -> bool {
        let mut claimed = false;
        self.native_wheel(steps, &mut claimed);
        claimed
    }
    /// A native mod's command.
    pub(super) fn client_mod_command(&mut self, command: &str) -> bool {
        let mut ran = false;
        self.native_command(command, &mut ran);
        ran
    }
    /// The zoom in effect while playing (a native mod's); the spyglass's own
    /// view wins.
    pub(super) fn native_zoom_now(&self) -> Option<NativeZoom> {
        if !self.native_playing() || self.scoping() {
            return None;
        }
        let mut zoom = None;
        self.native_zoom(&mut zoom);
        // As the WebAssembly runtime clamped a mod's zoom.
        let z = zoom?;
        let factor = finite(z.factor, 1.0, 10.0);
        (factor > 1.0).then_some(NativeZoom {
            factor,
            smooth: z.smooth,
            turn: if z.turn.is_finite() {
                z.turn.clamp(0.05, 1.0)
            } else {
                1.0
            },
        })
    }
    /// The pose of native mods' camera shake in this frame (`native_camera_shake`),
    /// none while not playing.
    pub(super) fn native_shake_matrix(&self, partial: f32) -> glam::Mat4 {
        if !self.native_playing() {
            return glam::Mat4::IDENTITY;
        }
        let mut shake = NativeShake::default();
        self.native_camera_shake(partial, &mut shake);
        if shake == NativeShake::default() {
            return glam::Mat4::IDENTITY;
        }
        shake_pose(bounded_shake(shake, self.screen_effect_scale()))
    }

    /// What native mods hand shader packs in this frame (`native_shader_params`),
    /// with values that aren't finite as 0.
    pub(super) fn native_shader_params_now(&self, partial: f32) -> [[f32; 4]; 8] {
        let mut params = [[0.0; 4]; 8];
        self.native_shader_params(partial, &mut params);
        finite_params(params)
    }
    /// The minimap a native mod shows.
    pub(super) fn native_minimap_now(&self) -> Option<NativeMinimap> {
        let mut minimap = None;
        self.native_minimap(&mut minimap);
        minimap.map(|m| NativeMinimap {
            size: finite(m.size, 16.0, 256.0),
            blocks_per_pixel: finite(m.blocks_per_pixel, 0.5, 4.0),
            ..m
        })
    }
}

/// Whether `rgba` is a whole `width` x `height` image that fits a
/// `slot_width` x `slot_height` atlas slot (a native mod's `mod_image`).
fn mod_image_fits(width: u32, height: u32, rgba: usize, slot_width: u32, slot_height: u32) -> bool {
    width > 0
        && height > 0
        && width <= slot_width
        && height <= slot_height
        && rgba == width as usize * height as usize * 4
}

/// Whether F3's debug screen, open when `debug`, covers `corner`
/// (`DebugScreenOverlay.extractRenderState`): its left and right columns
/// the top corners, and its charts the bottom corners `charts` (bottom left,
/// bottom right) says they are drawn in.
fn debug_takes(corner: Corner, debug: bool, charts: [bool; 2]) -> bool {
    debug
        && match corner {
            Corner::TopLeft | Corner::TopRight => true,
            Corner::BottomLeft => charts[0],
            Corner::BottomRight => charts[1],
        }
}

/// A menu of `MOD_MENUS` or a content mod's menu type, by its name.
fn mod_menu(name: &str) -> Option<(&'static str, usize)> {
    MOD_MENUS
        .into_iter()
        .find(|(menu, _)| *menu == name)
        .or_else(|| horizon_client::containers::mod_menu_type(name).map(|m| (m.id, m.slots)))
}

/// `state` with its property `key` set to `value`; none when the block has
/// no such property or value.
fn with_property(
    registry: &BlockRegistry,
    state: BlockStateId,
    key: &str,
    value: &str,
) -> Option<BlockStateId> {
    let mut s = registry.state(state)?.clone();
    match s.properties.get(key) {
        Some(v) if v == value => return Some(state),
        Some(_) => {}
        None => return None,
    }
    s.properties.insert(key.into(), value.into());
    registry.resolve(&s).ok()
}

/// The state a mod's property write changes `state` to; none when the block
/// has no such property or value, or already has it: `LevelChunk.setBlockState`
/// returns null for the same state, so `Level.setBlock` returns false.
fn property_change(
    registry: &BlockRegistry,
    state: BlockStateId,
    key: &str,
    value: &str,
) -> Option<BlockStateId> {
    with_property(registry, state, key, value).filter(|&id| id != state)
}

/// `config/` beside `options`, made when missing.
/// `config/` in the game folder, shared by its players.
fn config_dir(game_dir: &Path) -> Result<PathBuf, String> {
    let dir = game_dir.join("config");
    fs::create_dir_all(&dir).map_err(|e| dir.display().to_string() + ": " + &e.to_string())?;
    Ok(dir)
}

/// `params` with each value that isn't finite as 0, so a mod's mistake can't
/// put a NaN in every pixel of a pack's picture.
fn finite_params(params: [[f32; 4]; 8]) -> [[f32; 4]; 8] {
    params.map(|v| v.map(|x| if x.is_finite() { x } else { 0.0 }))
}

/// Whether `native_play_local_sound` takes these values: a sound event's id
/// (`sound_id`, as `native_play_sound` takes), a finite position, a volume
/// and pitch at or above 0, and a delay of a minute at most.
fn local_sound_ok(sound: &str, at: [f64; 3], volume: f32, pitch: f32, delay: i32) -> bool {
    sound_id(sound)
        && at.iter().all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
        && volume.is_finite()
        && volume >= 0.0
        && pitch.is_finite()
        && pitch >= 0.0
        && (0..=1200).contains(&delay)
}

/// `v` within `lo..=hi`; `lo` when it is not a number.
fn finite(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { lo }
}

/// An item entity as `native_item_entities` lists it. API 42.
/// Read by mods only.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct NativeItemEntity {
    /// The entity's ID, for `native_set_item_entity`.
    pub id: u32,
    pub position: [f64; 3],
    pub stack: ItemStack,
    /// Ticks before a player may pick it up (`ItemEntity.pickupDelay`);
    /// hoppers take it whatever this is.
    pub pickup_delay: u32,
}

/// `AABB.intersects`: the boxes overlap, touching faces excluded.
fn box_meets(b: &horizon_core::geometry::Aabb, min: [f64; 3], max: [f64; 3]) -> bool {
    let (lo, hi) = (b.min_position(), b.max_position());
    lo.x < max[0]
        && hi.x > min[0]
        && lo.y < max[1]
        && hi.y > min[1]
        && lo.z < max[2]
        && hi.z > min[2]
}

/// What `native_mod_screen_open(mod_id)` does over `screen` (`blocked`: an
/// overlay, the title screen, no window or no player; `owner`'s own screen
/// open): `None` opens it, `Some` answers without changing anything (true:
/// `mod_id`'s is already open).
fn mod_screen_opens(
    screen: Screen,
    blocked: bool,
    owner: Option<&str>,
    mod_id: &str,
) -> Option<bool> {
    if screen == Screen::Mod {
        return Some(owner == Some(mod_id));
    }
    if screen != Screen::Playing || blocked {
        return Some(false);
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn hand_pose_rejects_invalid_components() {
        let identity = super::hand_pose_matrix([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(identity, glam::Mat4::IDENTITY);
        assert_eq!(
            super::hand_pose_matrix([
                f32::NAN,
                17.0,
                f32::INFINITY,
                3601.0,
                -3601.0,
                f32::NAN,
                0.0
            ]),
            identity
        );
        let point = super::hand_pose_matrix([1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 2.0])
            .transform_point3(glam::Vec3::X);
        assert_eq!(point, glam::Vec3::new(3.0, 2.0, 3.0));
    }
    #[test]
    fn observed_overlay_prefers_first_open_screen() {
        assert_eq!(
            super::observed_overlay_id(&[(false, "sign"), (false, "book")]),
            None
        );
        assert_eq!(
            super::observed_overlay_id(&[(false, "sign"), (true, "book")]),
            Some("book")
        );
        assert_eq!(
            super::observed_overlay_id(&[(true, "sign"), (true, "book")]),
            Some("sign")
        );
    }

    /// Exercise the gameplay constructor, where name-based entities compete
    /// with a mod block's sign template.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn mod_sign_constructor_prioritizes_templates_over_suffixes() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use horizon_client::mod_content::{ContentMod, build};
        use horizon_mods::content::{Block, Content};

        let base = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES")
                .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
        );
        let root = std::env::temp_dir().join(format!(
            "horizon-sign-constructor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cases = [
            ("ruby:notice_banner", "minecraft:oak_sign", false),
            ("ruby:board_shelf", "minecraft:oak_wall_sign", false),
            ("ruby:lantern_head", "minecraft:oak_hanging_sign", true),
            (
                "ruby:bracket_skull",
                "minecraft:oak_wall_hanging_sign",
                true,
            ),
        ];
        let mut content = Content::declared("ruby");
        let blocks: Vec<_> = cases
            .iter()
            .map(|&(id, template, _)| (id, template))
            .chain([("ruby:road_sign", "minecraft:stone")])
            .map(|(id, template)| Block {
                id: id.into(),
                template: template.into(),
                item: false,
                ..Default::default()
            })
            .collect();
        content.declare(&blocks, &[]).unwrap();
        build(
            &base,
            &[ContentMod {
                id: "ruby".into(),
                content,
            }],
            &root,
        )
        .unwrap();
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();

        for (name, hanging) in cases
            .map(|(name, _, hanging)| (name, hanging))
            .into_iter()
            .chain([
                ("minecraft:oak_sign", false),
                ("minecraft:oak_hanging_sign", true),
            ])
        {
            match game.new_block_entity(name) {
                Some(super::BlockEntity::Sign(sign)) => {
                    assert_eq!(sign.hanging, hanging, "{name}")
                }
                entity => panic!("{name}: expected a sign, got {entity:?}"),
            }
        }
        assert!(game.new_block_entity("ruby:road_sign").is_none());
        assert!(matches!(
            game.new_block_entity("minecraft:white_banner"),
            Some(super::BlockEntity::Banner(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A mod's own screen opens only over the world, never over another
    /// screen, an overlay (the title screen, a book, loading, no window) or
    /// another mod's; opening it again is a no-op.
    #[test]
    fn a_mod_screen_opens_only_over_the_world() {
        use super::{Screen, mod_screen_opens};
        assert_eq!(mod_screen_opens(Screen::Playing, false, None, "a"), None);
        assert_eq!(
            mod_screen_opens(Screen::Playing, true, None, "a"),
            Some(false)
        );
        for other in [
            Screen::Paused,
            Screen::Inventory,
            Screen::Chat,
            Screen::Options,
        ] {
            assert_eq!(mod_screen_opens(other, false, None, "a"), Some(false));
        }
        assert_eq!(
            mod_screen_opens(Screen::Mod, false, Some("a"), "a"),
            Some(true)
        );
        assert_eq!(
            mod_screen_opens(Screen::Mod, false, Some("b"), "a"),
            Some(false)
        );
    }

    /// The characters a `ScreenEvent.CharacterTyped.Pre` sees are the
    /// printable ones of a text: a cancelled one leaves that text and a
    /// control character (a key, not text) is never offered a hook.
    #[test]
    fn typed_text_keeps_control_characters_and_drops_cancelled_ones() {
        let mut offered = Vec::new();
        let left = super::screen_text_hooked("a\u{7f}b\u{1b}c", |c| {
            offered.push(c);
            c == 'b'
        });
        assert_eq!(offered, ['a', 'b', 'c']);
        assert_eq!(left.as_deref(), Some("a\u{7f}\u{1b}c"));
        // Every printable character cancelled: no text is left to type.
        assert_eq!(super::screen_text_hooked("ab", |_| true), None);
        // Nothing to offer: an empty text stays empty and keys alone stay whole.
        assert_eq!(super::screen_text_hooked("", |_| true).as_deref(), Some(""));
        assert_eq!(
            super::screen_text_hooked("\u{7f}\t", |_| true).as_deref(),
            Some("\u{7f}\t")
        );
        // What no hook cancels is handed back as it came in.
        assert_eq!(
            super::screen_text_hooked("hello", |_| false).as_deref(),
            Some("hello")
        );
    }

    /// The window's key path: a key whose press a mod cancelled still types
    /// the characters no mod cancelled (vanilla's separate `charTyped`), while
    /// the key characters never reach the text box.
    #[test]
    fn a_cancelled_key_still_types_the_characters_left() {
        // `axb` with 'x' cancelled: the key went no further, `ab` still types.
        assert_eq!(
            super::chars_to_type("axb", |c| c == 'x').as_deref(),
            Some("ab")
        );
        // Backspace, Escape and the like are keys, not text.
        assert_eq!(
            super::chars_to_type("a\u{7f}\u{1b}b", |_| false).as_deref(),
            Some("ab")
        );
        // Every character cancelled: the box takes none of them.
        assert_eq!(super::chars_to_type("ab", |_| true), None);
        // Nothing typed: nothing to hand over.
        assert_eq!(super::chars_to_type("", |_| false), None);
    }

    /// A shake is scaled by Distortion Effects, bounded on each axis, and a
    /// value that isn't finite is none.
    #[test]
    fn a_shake_is_scaled_and_bounded() {
        use super::{NativeShake, bounded_shake, shake_pose};
        let shake = NativeShake {
            pitch: 4.0,
            yaw: -30.0,
            roll: f32::NAN,
            offset: [0.2, 2.0, f32::INFINITY],
        };
        assert_eq!(
            bounded_shake(shake, 1.0),
            NativeShake {
                pitch: 4.0,
                yaw: -10.0,
                roll: 0.0,
                offset: [0.2, 0.5, 0.0],
            }
        );
        assert_eq!(bounded_shake(shake, 0.5).pitch, 2.0);
        assert_eq!(bounded_shake(shake, 0.0), NativeShake::default());
        assert_eq!(shake_pose(NativeShake::default()), glam::Mat4::IDENTITY);
    }

    /// A mod's shader values that aren't finite read as 0.
    #[test]
    fn shader_params_that_arent_finite_read_as_zero() {
        let mut params = [[1.0; 4]; 8];
        params[0] = [f32::NAN, 2.0, f32::INFINITY, -3.0];
        let out = super::finite_params(params);
        assert_eq!(out[0], [0.0, 2.0, 0.0, -3.0]);
        assert_eq!(out[7], [1.0; 4]);
    }

    /// A client mod's sound needs a namespaced event, finite values and a
    /// delay of a minute at most.
    #[test]
    fn a_local_sound_is_checked() {
        use super::local_sound_ok;
        let at = [0.5, 64.0, 0.5];
        assert!(local_sound_ok("ironfall:boom_far", at, 8.0, 1.0, 140));
        assert!(local_sound_ok(
            "minecraft:entity.generic.explode",
            at,
            1.0,
            1.0,
            0
        ));
        assert!(!local_sound_ok("boom", at, 1.0, 1.0, 0));
        assert!(!local_sound_ok("ironfall:", at, 1.0, 1.0, 0));
        assert!(!local_sound_ok("Ironfall:Boom", at, 1.0, 1.0, 0));
        assert!(!local_sound_ok(
            "ironfall:boom",
            [f64::NAN, 0.0, 0.0],
            1.0,
            1.0,
            0
        ));
        assert!(!local_sound_ok("ironfall:boom", at, -1.0, 1.0, 0));
        assert!(!local_sound_ok("ironfall:boom", at, 1.0, 1.0, 1201));
        assert!(!local_sound_ok("ironfall:boom", at, 1.0, 1.0, -1));
    }

    /// `native_play_sound` takes vanilla's and mods' sound ids, nothing else.
    #[test]
    fn sound_ids_are_identifiers() {
        for id in [
            "minecraft:block.note_block.harp",
            "mymod:machine/hum",
            "my_mod.x-1:a",
        ] {
            assert!(super::sound_id(id), "{id}");
        }
        for id in [
            "",
            "hum",
            ":hum",
            "mymod:",
            "MyMod:hum",
            "mymod:Hum",
            "my/mod:hum",
            "a:b c",
        ] {
            assert!(!super::sound_id(id), "{id}");
        }
    }

    /// A mod's property write: none for a key or value the block lacks, the
    /// state itself when it already has the value, else the new state.
    #[test]
    fn with_property_finds_the_state() {
        let facts = br#"{"defaults":{"minecraft:air":0,"test:crop":1},"states":[
            ["air",true,false,"","minecraft:air"],
            ["crop0",false,false,"",{"id":"test:crop","properties":{"age":"0"}}],
            ["crop1",false,false,"",{"id":"test:crop","properties":{"age":"1"}}]
        ]}"#;
        let registry = horizon_world::registry::BlockRegistry::from_facts_json(&facts[..]).unwrap();
        use horizon_world::registry::BlockStateId;
        let (age0, age1) = (BlockStateId(1), BlockStateId(2));
        assert_eq!(
            super::with_property(&registry, age0, "age", "0"),
            Some(age0)
        );
        assert_eq!(
            super::with_property(&registry, age0, "age", "1"),
            Some(age1)
        );
        assert_eq!(super::with_property(&registry, age0, "age", "9"), None);
        assert_eq!(super::with_property(&registry, age0, "lit", "true"), None);
        // What both property setters write: nothing (false) when the
        // property already has the value, as `Level.setBlock`.
        assert_eq!(super::property_change(&registry, age0, "age", "0"), None);
        assert_eq!(
            super::property_change(&registry, age0, "age", "1"),
            Some(age1)
        );
        assert_eq!(super::property_change(&registry, age0, "age", "9"), None);
        assert_eq!(super::property_change(&registry, age0, "lit", "true"), None);
    }

    /// The state a mod block is placed in is built from its own default state
    /// and one of its properties, its declared ones (`state_properties` in its
    /// block file) included, and keeps the properties it already had.
    #[test]
    fn a_placement_state_is_built_from_a_blocks_own_properties() {
        let facts = br#"{"defaults":{"minecraft:air":0,"test:pipe":1},"states":[
            ["air",true,false,"","minecraft:air"],
            ["north_gas",false,false,"",{"id":"test:pipe","properties":{"carries":"gas","facing":"north"}}],
            ["east_gas",false,false,"",{"id":"test:pipe","properties":{"carries":"gas","facing":"east"}}],
            ["north_items",false,false,"",{"id":"test:pipe","properties":{"carries":"items","facing":"north"}}],
            ["east_items",false,false,"",{"id":"test:pipe","properties":{"carries":"items","facing":"east"}}]
        ]}"#;
        let registry = horizon_world::registry::BlockRegistry::from_facts_json(&facts[..]).unwrap();
        use horizon_world::registry::BlockStateId;
        let (north_gas, east_gas, east_items) = (BlockStateId(1), BlockStateId(2), BlockStateId(4));
        // The declared property changes, and `carries` stays as it was.
        assert_eq!(
            super::with_property(&registry, north_gas, "facing", "east"),
            Some(east_gas)
        );
        assert_eq!(
            super::with_property(&registry, east_gas, "carries", "items"),
            Some(east_items)
        );
        // A value the property does not have builds nothing.
        assert_eq!(
            super::with_property(&registry, east_gas, "facing", "up"),
            None
        );
    }
    /// Both property setters return false for a write that changes nothing:
    /// each goes through `property_change`, none returns true on its own.
    #[test]
    fn property_setters_refuse_no_change() {
        // A Windows checkout has CRLF line ends: the searches below look for "\n".
        let source = include_str!("native_hooks.rs").replace("\r\n", "\n");
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        for head in [
            "    pub(super) fn native_set_block_property(\n        &mut self,\n        ticks:",
            "    pub(super) fn native_set_block_property_with(",
        ] {
            let body = code.split(head).nth(1).expect(head);
            let body = &body[..body.find("\n    }\n").expect(head)];
            assert!(body.contains("property_change("), "{head}");
            assert!(!body.contains("return true"), "{head}");
        }
    }
    /// horizon-mixins refuses targets whose body has a `cfg` or a macro:
    /// the hook points and helpers must stay free of both. The launcher
    /// reads the API line as it is.
    /// A data packet for an old window leaves the open mod screen alone.
    #[test]
    fn mod_screen_data_is_its_windows_only() {
        let mut s = super::OpenNativeScreen {
            key: (0, 0, 0),
            window: 4,
            block: "gtmod:machine".into(),
            screen: super::NativeScreen::default(),
            data: Vec::new(),
        };
        assert!(s.set_data(4, 1, 7));
        assert!(!s.set_data(3, 1, 99));
        assert_eq!(s.data, [0, 7]);
    }

    #[test]
    fn hook_points_stay_hookable() {
        let source = include_str!("native_hooks.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        assert!(code.contains("\npub(crate) const NATIVE_HOOKS_API: u32 = "));
        let lines = code.lines().filter(|l| !l.trim_start().starts_with("//"));
        for line in lines {
            assert!(!line.contains("cfg"), "a cfg in the hook points: {line}");
            for mac in ["!(", "![", "! {", "!{"] {
                assert!(!line.contains(mac), "a macro in the hook points: {line}");
            }
        }
    }

    #[test]
    fn native_breathing_stays_hookable() {
        let source = include_str!("../breathing.rs").replace("\r\n", "\n");
        let hook = source.find("pub fn native_living_breathe(").unwrap();
        // From its doc comment's first line, so an attribute above it counts.
        let start = source[..hook].find("/// An entity's breathing").unwrap();
        let body = &source[start..hook + source[hook..].find("\n}\n").unwrap()];
        assert!(body.contains("#[inline(never)]"));
        for line in body.lines() {
            assert!(!line.contains("cfg"), "a cfg in the breathing hook: {line}");
            for mac in ["!(", "![", "! {", "!{"] {
                assert!(!line.contains(mac), "a macro in the breathing hook: {line}");
            }
        }
    }

    #[test]
    fn native_loot_kinds_stay_hookable() {
        let source = include_str!("../../../horizon-world/src/loot.rs").replace("\r\n", "\n");
        for hook in [
            "pub fn native_loot_function(",
            "pub fn native_loot_condition(",
        ] {
            let at = source
                .find(hook)
                .unwrap_or_else(|| panic!("{hook} not found"));
            let start = source[..at].rfind("///").unwrap();
            let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
            for line in body.lines() {
                assert!(!line.contains("cfg"), "a cfg in a loot hook point: {line}");
                for mac in ["!(", "![", "! {", "!{"] {
                    assert!(!line.contains(mac), "a macro in a loot hook point: {line}");
                }
            }
        }
    }

    #[test]
    fn native_ingredient_stays_hookable() {
        let source = include_str!("../crafting.rs").replace("\r\n", "\n");
        let at = source.find("pub fn native_ingredient(").unwrap();
        let start = source[..at].rfind("///").unwrap();
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        for line in body.lines() {
            assert!(
                !line.contains("cfg"),
                "a cfg in the ingredient hook: {line}"
            );
            for mac in ["!(", "![", "! {", "!{"] {
                assert!(!line.contains(mac), "a macro: {line}");
            }
        }
    }

    #[test]
    fn loot_table_load_stays_hookable() {
        let source = include_str!("../../../horizon-world/src/loot.rs").replace("\r\n", "\n");
        // From its doc comment's last line, so an attribute above it counts.
        let hook = source.find("pub fn native_loot_table_load(").unwrap();
        let start = source[..hook].rfind("///").unwrap();
        let end = source[start..]
            .find("\n}\n")
            .or_else(|| source[start..].find("\r\n}\r\n"))
            .unwrap();
        let body = &source[start..start + end];
        for line in body.lines() {
            assert!(
                !line.contains("cfg"),
                "a cfg in the loot hook point: {line}"
            );
            for mac in ["!(", "![", "! {", "!{"] {
                assert!(
                    !line.contains(mac),
                    "a macro in the loot hook point: {line}"
                );
            }
        }
    }

    #[test]
    fn lifecycle_source_stays_hookable() {
        let source = include_str!("native_lifecycle.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or_default();
        for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
            assert!(!line.contains("cfg"), "a cfg in lifecycle source: {line}");
            for mac in ["!(", "![", "! {", "!{"] {
                assert!(!line.contains(mac), "a macro in lifecycle source: {line}");
            }
        }
    }

    #[test]
    fn catalog_matches_hook_points() {
        use std::collections::BTreeSet;

        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../../../../native-hooks.json")).unwrap();
        let mut hooks = catalog["hooks"].as_array().unwrap().clone();
        let fragments = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/native-hook-fragments");
        for entry in std::fs::read_dir(fragments).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let fragment: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                hooks.extend(fragment["hooks"].as_array().unwrap().iter().cloned().map(
                    |mut hook| {
                        hook["version"] = 1.into();
                        hook
                    },
                ));
            }
        }
        let source = include_str!("native_hooks.rs");
        let section = source
            .split("/// The hook points. Each does nothing in Horizon.")
            .nth(1)
            .unwrap()
            .split("/// Server-level helpers for native mods.")
            .next()
            .unwrap();
        let level_section = source
            .split("impl GameLevel<'_> {")
            .nth(1)
            .unwrap()
            .split("\n}")
            .next()
            .unwrap();
        let level_hooks: BTreeSet<_> = level_section
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("pub(super) fn native_")?
                    .split('(')
                    .next()
            })
            .collect();
        // Hook points that are free functions in other crates.
        let free_hooks = |source: &'static str| -> BTreeSet<&'static str> {
            source
                .lines()
                .filter_map(|line| line.strip_prefix("pub fn native_")?.split('(').next())
                .collect()
        };
        // The renderer's (`horizon_client_render::gfx::native_render`).
        let render_hooks = free_hooks(include_str!(
            "../../../horizon-client-render/src/gfx/native_render.rs"
        ));
        // Items' default components (`horizon_world::item_defaults`).
        let item_hooks = free_hooks(include_str!("../../../horizon-world/src/item_defaults.rs"));
        // The loot tables' hook point (`horizon_world::loot`).
        let loot_hooks = free_hooks(include_str!("../../../horizon-world/src/loot.rs"));
        // The menu engine's (`horizon_client::menu`).
        let menu_hooks = free_hooks(include_str!("../menu.rs"));
        // The air supply's (`horizon_client::breathing`).
        let breath_hooks = free_hooks(include_str!("../breathing.rs"));
        // The crafting loaders' (`horizon_client_lib::crafting`).
        let crafting_hooks = free_hooks(include_str!("../crafting.rs"));
        let hunger_hooks = free_hooks(include_str!("../hunger.rs"));
        let landing_hooks = free_hooks(include_str!("../landing.rs"));
        let motion_hooks = free_hooks(include_str!("../motion.rs"));
        let declared: BTreeSet<_> = section
            .lines()
            .chain(level_section.lines())
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("pub(super) fn native_")?
                    .split('(')
                    .next()
            })
            .chain(render_hooks.iter().copied())
            .chain(item_hooks.iter().copied())
            .chain(loot_hooks.iter().copied())
            .chain(menu_hooks.iter().copied())
            .chain(breath_hooks.iter().copied())
            .chain(crafting_hooks.iter().copied())
            .chain(hunger_hooks.iter().copied())
            .chain(landing_hooks.iter().copied())
            .chain(motion_hooks.iter().copied())
            .collect();
        let mut names = BTreeSet::new();
        let mut contracts = BTreeSet::new();
        for hook in &hooks {
            let name = hook["name"].as_str().unwrap();
            let version = hook["version"].as_u64().unwrap();
            assert!(
                version > 0 && contracts.insert((name, version)),
                "invalid or duplicate hook: {name} v{version}"
            );
            names.insert(name);
            let target = if render_hooks.contains(name) {
                format!("horizon_client_render::gfx::native_render::native_{name}")
            } else if item_hooks.contains(name) {
                format!("horizon_world::item_defaults::native_{name}")
            } else if loot_hooks.contains(name) {
                format!("horizon_world::loot::native_{name}")
            } else if crafting_hooks.contains(name) {
                // crafting.rs compiles into horizon-client-lib (#5446's
                // split): horizon-mixins resolves the crate by its package.
                format!("horizon_client_lib::crafting::native_{name}")
            } else if hunger_hooks.contains(name) {
                format!("horizon_client_lib::hunger::native_{name}")
            } else if landing_hooks.contains(name) {
                format!("horizon_client_lib::landing::native_{name}")
            } else if motion_hooks.contains(name) {
                format!("horizon_client_lib::motion::native_{name}")
            } else if menu_hooks.contains(name) {
                // menu.rs compiles into horizon-client-lib (#5446's
                // split): horizon-mixins resolves the crate by its package.
                format!("horizon_client_lib::menu::native_{name}")
            } else if breath_hooks.contains(name) {
                // breathing.rs compiles into horizon-client-lib too.
                format!("horizon_client_lib::breathing::native_{name}")
            } else if level_hooks.contains(name) {
                format!("horizon_client::game::native_hooks::GameLevel<'_>::native_{name}")
            } else {
                format!("horizon_client::game::native_hooks::Game<'_>::native_{name}")
            };
            assert_eq!(hook["target"].as_str().unwrap(), target);
        }
        assert_eq!(names, declared);
        // Every target's crate really compiles its module: #5446 moved
        // files between horizon-client and horizon-client-lib, and a
        // target left on the old crate applies to no bundle — the string
        // comparison above shares its expectation with the catalog, so it
        // alone cannot catch a move. The first path segment after the
        // crate must be a module of that crate's lib root.
        let client_root = include_str!("../lib.rs");
        let lib_root = include_str!("../client_lib.rs");
        for hook in &hooks {
            let target = hook["target"].as_str().unwrap();
            let (root, crate_name, rest) =
                if let Some(rest) = target.strip_prefix("horizon_client::") {
                    (client_root, "horizon_client", rest)
                } else if let Some(rest) = target.strip_prefix("horizon_client_lib::") {
                    (lib_root, "horizon_client_lib", rest)
                } else {
                    // The other crates (render, world) are their own packages,
                    // never split.
                    continue;
                };
            let module = rest.split("::").next().unwrap_or(rest);
            assert!(
                root.contains(&format!("mod {module};")),
                "{crate_name} does not compile module `{module}` (target {target})"
            );
        }
    }

    /// The damage hook gets a mob's source from its damage: the causing mob,
    /// the direct one when it deals the damage itself.
    #[test]
    fn the_damage_hook_names_the_source() {
        use super::{HurtSource, Hurting, hurt_server_args, mob_source};
        use horizon_client::mobs::{Damage, DamageKind as K, Who};
        let source = |kind, attacker| {
            let s = mob_source(&Damage {
                kind,
                attacker,
                from: None,
            });
            (s.damage_type, s.causing, s.direct)
        };
        let zombie = Some(Who::Mob(3));
        assert_eq!(source(K::MobAttack, zombie), ("minecraft:mob_attack", 3, 3));
        assert_eq!(source(K::Thorns, zombie), ("minecraft:thorns", 3, 3));
        assert_eq!(source(K::Arrow, zombie), ("minecraft:arrow", 3, 0));
        let blast = source(K::Explosion, zombie);
        assert_eq!(blast, ("minecraft:player_explosion", 3, 0));
        // A player behind a mob's hit isn't known yet, nor is a fall's source.
        let hit = source(K::PlayerAttack, Some(Who::Player));
        assert_eq!(hit, ("minecraft:player_attack", 0, 0));
        assert_eq!(source(K::Fall, None), ("minecraft:fall", 0, 0));
        let arrow = HurtSource {
            damage_type: "minecraft:arrow",
            causing: 4,
            direct: 0,
        };
        let player = (2, "minecraft:player", arrow);
        assert_eq!(hurt_server_args(Hurting::Player(2, arrow)), player);
        let dragon = (5, "minecraft:ender_dragon", arrow);
        assert_eq!(hurt_server_args(Hurting::Dragon(5, arrow)), dragon);
    }

    /// A mod container opens a slot grid or a furnace's menu, by its slot
    /// count; no menu with rules of its own (a brewing stand's, a crafter's).
    #[test]
    fn mod_menus_are_slot_grids_and_furnaces() {
        use super::mod_menu;
        assert_eq!(mod_menu("minecraft:hopper"), Some(("minecraft:hopper", 5)));
        assert_eq!(mod_menu("minecraft:generic_9x3").map(|m| m.1), Some(27));
        assert_eq!(mod_menu("minecraft:furnace").map(|m| m.1), Some(3));
        assert_eq!(mod_menu("minecraft:brewing_stand"), None);
        assert_eq!(mod_menu("hopper"), None);
    }

    /// The config folder is `config/` beside `options.txt`, made when
    /// missing.
    #[test]
    fn config_dir_is_config_in_the_game_folder() {
        // Callable without a game, from any hook.
        let _: fn() -> Result<std::path::PathBuf, String> = super::Game::native_config_dir;
        use super::config_dir;
        let root = std::env::temp_dir().join(format!("horizon-config-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(!root.join("config").exists());
        let dir = config_dir(&root).unwrap();
        assert_eq!(dir, root.join("config"));
        assert!(dir.is_dir());
        assert_eq!(config_dir(&root).unwrap(), dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Vanilla's F3 draws a column in each top corner (`extractLines`, left
    /// and right aligned), and its charts along the bottom.
    #[test]
    fn the_debug_screen_takes_both_top_corners_and_its_charts_corners() {
        use super::{Corner, debug_takes};
        for corner in Corner::ALL {
            assert!(!debug_takes(corner, false, [true, true]), "{corner:?}");
        }
        assert!(debug_takes(Corner::TopLeft, true, [false, false]));
        assert!(debug_takes(Corner::TopRight, true, [false, false]));
        assert!(!debug_takes(Corner::BottomLeft, true, [false, false]));
        assert!(!debug_takes(Corner::BottomRight, true, [false, false]));
        assert!(debug_takes(Corner::BottomLeft, true, [true, false]));
        assert!(!debug_takes(Corner::BottomRight, true, [true, false]));
        assert!(debug_takes(Corner::BottomRight, true, [false, true]));
    }

    /// An image is taken whole, and only when it fits its slot.
    #[test]
    fn a_mod_image_is_taken_whole_and_only_when_it_fits() {
        use super::mod_image_fits;
        assert!(mod_image_fits(4, 2, 32, 1024, 1024));
        assert!(mod_image_fits(1024, 1024, 1024 * 1024 * 4, 1024, 1024));
        assert!(!mod_image_fits(0, 2, 0, 1024, 1024));
        assert!(!mod_image_fits(4, 2, 31, 1024, 1024));
        assert!(!mod_image_fits(4, 2, 33, 1024, 1024));
        assert!(!mod_image_fits(2048, 1, 2048 * 4, 1024, 1024));
    }
}
