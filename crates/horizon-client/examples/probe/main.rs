//! Every horizon-client parity probe in one binary: `probe <name> [arguments]` runs `<name>.rs` as its own
//! example did (tools/test_*_parity.py, tools/oracle_harness.py, tools/audio_harness.py). Linking ~80 examples,
//! each with the whole client in it, was most of the parity job's probe build; one binary builds it once.
//!
//! This file only lists the probes, so that adding one reruns no harness: tools/parity.py leaves it out of
//! their dependencies. How a probe runs is in dispatch.rs.
extern crate horizon_client_lib as horizon_client;
#[macro_use]
mod dispatch;

use dispatch::{args, args_os};

mod ambient_fixture;
mod arrow_probe;
mod audio_attributes_fixture;
/// Sound-engine fakes the audio probes share.
mod audio_common;
mod auto_jump_probe;
mod block_entity_anim_fixture;
mod block_entity_render_fixture;
mod block_entity_tickers_fixture;
mod block_particles_fixture;
mod block_tint_probe;
mod block_transform_fixture;
mod breaking_fixture;
mod camera_probe;
mod cauldron_fixture;
mod chat_screen_fixture;
mod chest_pair_fixture;
mod cloud_fixture;
mod command_parse_fixture;
mod container_screen_probe;
mod container_title_fixture;
mod creative_mining_probe;
mod creative_search_fixture;
mod cushion_placement_probe;
mod emitter_particles_probe;
mod enchanting_fixture;
mod entity_anim_fixture;
mod entity_components_probe;
mod entity_death_loot_fixture;
mod entity_particles_probe;
mod environment_fixture;
mod finalize_spawn_fixture;
mod fire_starter_sound_probe;
mod firework_tooltip_fixture;
mod first_person_probe;
mod frame_limiter_probe;
mod game_loop_probe;
mod ghost_recipe_fixture;
mod held_drop_fixture;
mod held_item_stack_probe;
mod hotbar_fixture;
mod hud_fixture;
mod inline_object_fixture;
mod input_probe;
mod interaction_input_fixture;
mod lan_probe;
mod light_parity;
mod lore_tooltip_fixture;
mod marker_particles_probe;
mod menu_fixture;
mod mipmap_probe;
mod mob_ai_probe;
mod mob_attributes_fixture;
mod music_fixture;
mod natural_spawn_fixture;
mod options_probe;
mod pack_resources_fixture;
mod pack_screen_fixture;
mod pack_selection_fixture;
mod particle_providers_probe;
mod pick_block_data_fixture;
mod place_recipe_fixture;
mod player_sync_fixture;
mod raid_fixture;
mod random_tick_sections_probe;
mod recipe_book_fixture;
mod recipe_book_render_fixture;
mod recipe_book_view_fixture;
mod recipe_fixture;
mod remote_attack_probe;
mod scene_collision_probe;
mod scene_motion_probe;
mod scoreboard_fixture;
mod screen_layout_fixture;
mod section_mesh_fixture;
mod server_files_probe;
mod situational_music_fixture;
mod skin_image_fixture;
mod sound_engine_fixture;
mod sound_event_fixture;
mod spawn_rules_fixture;
mod sprint_probe;
mod squid_client_probe;
mod storage_menu_fixture;
mod subtitle_fixture;
mod tab_list_fixture;
mod team_rules_fixture;
mod text_display_fixture;
mod text_display_render_fixture;
mod toggle_keys_probe;
mod tooltip_display_fixture;
mod trading_fixture;
mod translucency_fixture;
mod trident_probe;
mod vehicle_container_fixture;
mod weather_fixture;
mod world_border_render_fixture;
mod world_options_fixture;

probes! {
    ambient_fixture,
    arrow_probe,
    audio_attributes_fixture,
    auto_jump_probe,
    block_entity_anim_fixture,
    block_entity_render_fixture,
    block_entity_tickers_fixture,
    block_particles_fixture,
    block_transform_fixture,
    breaking_fixture,
    block_tint_probe,
    camera_probe,
    squid_client_probe,
    creative_mining_probe,
    cushion_placement_probe,
    cauldron_fixture,
    chat_screen_fixture,
    chest_pair_fixture,
    cloud_fixture,
    command_parse_fixture,
    container_screen_probe,
    container_title_fixture,
    creative_search_fixture,
    emitter_particles_probe,
    enchanting_fixture,
    entity_anim_fixture,
    entity_components_probe,
    entity_death_loot_fixture,
    entity_particles_probe,
    environment_fixture,
    finalize_spawn_fixture,
    fire_starter_sound_probe,
    firework_tooltip_fixture,
    first_person_probe,
    frame_limiter_probe,
    game_loop_probe,
    ghost_recipe_fixture,
    held_drop_fixture,
    held_item_stack_probe,
    hotbar_fixture,
    hud_fixture,
    input_probe,
    inline_object_fixture,
    interaction_input_fixture,
    lan_probe,
    light_parity,
    lore_tooltip_fixture,
    marker_particles_probe,
    menu_fixture,
    mipmap_probe,
    mob_ai_probe,
    mob_attributes_fixture,
    music_fixture,
    natural_spawn_fixture,
    options_probe,
    pack_resources_fixture,
    pack_screen_fixture,
    pack_selection_fixture,
    particle_providers_probe,
    pick_block_data_fixture,
    place_recipe_fixture,
    player_sync_fixture,
    raid_fixture,
    random_tick_sections_probe,
    recipe_book_fixture,
    recipe_book_render_fixture,
    recipe_book_view_fixture,
    recipe_fixture,
    remote_attack_probe,
    scene_collision_probe,
    scene_motion_probe,
    scoreboard_fixture,
    screen_layout_fixture,
    section_mesh_fixture,
    server_files_probe,
    situational_music_fixture,
    skin_image_fixture,
    sound_engine_fixture,
    sound_event_fixture,
    spawn_rules_fixture,
    sprint_probe,
    storage_menu_fixture,
    subtitle_fixture,
    tab_list_fixture,
    team_rules_fixture,
    toggle_keys_probe,
    tooltip_display_fixture,
    text_display_fixture,
    text_display_render_fixture,
    trading_fixture,
    trident_probe,
    translucency_fixture,
    vehicle_container_fixture,
    weather_fixture,
    world_border_render_fixture,
    world_options_fixture,
}
