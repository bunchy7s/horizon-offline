//! Hook points for native mods (docs/native-mods.md, "Mods on the hook points"): empty
//! functions the game calls at fixed places, for horizon-mixins bundles to
//! hook with HEAD. Hooks that answer take an out parameter (`claimed`,
//! `zoom`…) so several mods can hook the same point. A native mod keeps its
//! own state in a module of its bundle; the helpers below read and draw what
//! Horizon's own HUD extras do, and keep a mod's settings where the
//! WebAssembly mod of the same id kept them.
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
//! item entities and experience orbs so far.
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
//! API 47 adds entities joining a level (`native_entity_join_level`), for
//! item entities and experience orbs so far.
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
use super::native_lifecycle::{NativeLifecycleContext, ServerTickPhase, WorldPhase};
use super::{
    Game, GameLevel, LevelDimension, Screen,
    execution_role::ExecutionRole,
    native_block_entity_draw::BlockEntityDraw,
    native_panel_recipes::RecipePage,
    native_server::{NativeCommand, NativeServerPlayer},
};
use horizon_client::{
    atlas_sources::qualified,
    block_entities::Pos,
    chat::ChatSource,
    containers::{BlockEntity, MOD_MENUS, ModBlockEntity, ModContainer, ModFluidTanks},
    crafting::{CustomRecipe, Ingredient, recipe_result},
    extras::Corner,
    extras_hud::Corners,
    fast_hash::FxHashMap,
    first_person::Hand,
    health::{HurtSource, Hurting},
    hud::{HudBatch, rgba},
    items::ItemStack,
    keybinds,
    mobs::{Damage, Who},
    mod_list::{self, Installed, NativePage},
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
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use winit::{event::MouseButton, keyboard::KeyCode};

/// `Level.setBlock`'s flags, for the `native_set_block_property` helpers:
/// `flags::UPDATE_CLIENTS` as a vanilla crop grows, `flags::UPDATE_ALL` as a
/// furnace sets its `lit`. Horizon sends every write to players, so
/// `UPDATE_CLIENTS`, `UPDATE_INVISIBLE` and `UPDATE_IMMEDIATE` change nothing
/// yet.
pub(super) use horizon_world::level::flags;

/// The hook points' API version. The launcher reads this line from the
/// source it builds; keep its form.
#[allow(dead_code)]
pub(crate) const NATIVE_HOOKS_API: u32 = 80;

impl GameLevel<'_> {
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
        let _ = (dimension, pos, old, new);
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
        let _ = (dimension, pos, state, changed, moved);
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
        let _ = (dimension, kind, from, to, canceled);
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
        let _ = (dimension, piston, direction, push, destroy, canceled);
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
        let _ = (dimension, pos, state, face, direct, signal);
    }

    /// `BlockState.isSignalSource`: set `source` for a block of yours that
    /// gives a signal, so redstone wire beside it connects to it. Authoritative
    /// servers only. API 38.
    #[inline(always)]
    pub(super) fn native_is_signal_source(&self, state: BlockStateId, source: &mut bool) {
        let _ = (state, source);
    }

    /// `BlockState.hasAnalogOutputSignal`: set `output` for a block of yours
    /// that comparators read (how full or far along it is, as a composter's
    /// level), so they ask `native_analog_output`. Asked from inside the
    /// level's update: answer from `state` alone. Authoritative servers only.
    /// API 64.
    #[inline(always)]
    pub(super) fn native_has_analog_output(&self, state: BlockStateId, output: &mut bool) {
        let _ = (state, output);
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
        let _ = (dimension, pos, state, face, output);
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
        let _ = (ticks, dimension, pos, state, handled);
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
        let _ = (
            ticks,
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
        let _ = (dimension, pos, state, survives);
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
        let _ = (dimension, state, ground, ground_pos, may);
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
        let _ = (ticks, dimension, pos, state, handled);
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
    /// `native_state_with_property`. It is asked for every placer that goes
    /// through vanilla's placement (a player's `BlockItem.place`, a
    /// dispenser's `DirectionalPlaceContext`), and the answer still takes its
    /// survival and entity obstruction checks. Authoritative servers only.
    /// 76.
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
        let _ = (dimension, state, context, fluid, answer);
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
        let _ = (dimension, pos, state, valid);
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
        let _ = (dimension, pos, state, success);
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
        let _ = (ticks, dimension, pos, state);
    }
}

/// What dropped the items [`Game::native_loot_drops`] hears.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeLootDrops<'a> {
    /// The block's or mob's own loot table (`minecraft:blocks/stone`, `minecraft:entities/zombie`);
    /// empty when none rolled: the block or mob has none, the `mob_drops` rule is off, a baby
    /// animal. A mob's drops may hold more: a charged creeper's head, a fox's mouth item.
    pub(crate) table: &'a str,
    /// The level they drop in (`minecraft:overworld`).
    pub(crate) dimension: &'a str,
    /// `LootContextParams.ORIGIN`: a broken block's centre, a mob's position.
    pub(crate) origin: [f64; 3],
    /// The block broken: its position and state.
    pub(crate) block: Option<([i32; 3], BlockStateId)>,
    /// The mob that died, or the entity a player's use dropped them from (the
    /// sheep sheared, the item frame broken): its entity ID and type
    /// (`minecraft:zombie`).
    pub(crate) entity: Option<(u32, &'a str)>,
    /// The player who broke the block or killed the mob, when known. A mob
    /// names one only while it remembers the player's hit (100 ticks), so one
    /// that dies to lava long after names none.
    pub(crate) player: Option<u128>,
    /// What broke the block (`LootContextParams.TOOL`).
    pub(crate) tool: Option<&'a ItemStack>,
    /// The radius of the explosion that broke the block.
    pub(crate) explosion: Option<f32>,
    /// A player's use dropped them rather than a break or a death: shears on a
    /// sheep, a mooshroom, a snow golem or a bogged, a brush on an armadillo or
    /// a suspicious block, shears carving a pumpkin, picking sweet berries or
    /// cave vines, a hit taking an item frame or its item, or untying a lead
    /// or cutting it with shears. `player` names them; `block` or `entity` says
    /// what dropped them. API @API:use_drops@. A mob that a player killed while
    /// leashed drops its lead on its next tick as a call of its own (`entity`
    /// the mob, `player` its killer, `table` empty, `used` false). API
    /// @API:lead_drops@.
    pub(crate) used: bool,
}

/// A depth-tested outline of the client's block selection shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeBlockRim {
    pub(crate) pos: [i32; 3],
    pub(crate) argb: u32,
    pub(crate) width: f32,
}

/// A camera-local visual light, with no gameplay light or shadow writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeColoredLight {
    pub(crate) at: [f64; 3],
    pub(crate) rgb: u32,
    pub(crate) radius: f32,
    pub(crate) intensity: f32,
}

/// The zoom a native mod asks for (`native_zoom`): the factor (1 is none),
/// eased through the field of view like the spyglass when `smooth`, and the
/// scale of mouse turning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeZoom {
    pub(crate) factor: f32,
    pub(crate) smooth: bool,
    pub(crate) turn: f32,
}

/// The camera shake native mods ask for (`native_camera_shake`): turns in
/// degrees, and an offset in blocks along the camera's right, up and back.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NativeShake {
    pub(crate) pitch: f32,
    pub(crate) yaw: f32,
    pub(crate) roll: f32,
    pub(crate) offset: [f32; 3],
}

/// The largest turn a shake gives, in degrees, on each axis.
const SHAKE_MAX_DEGREES: f32 = 10.0;
/// The largest offset a shake gives, in blocks, along each axis.
const SHAKE_MAX_OFFSET: f32 = 0.5;

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

/// The minimap a native mod shows (`native_minimap`), drawn by Horizon's
/// minimap: its corner, its side in GUI pixels, blocks per map pixel,
/// turning with the player or north up, round or square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeMinimap {
    pub(crate) corner: Corner,
    pub(crate) size: f32,
    pub(crate) blocks_per_pixel: f32,
    pub(crate) rotate: bool,
    pub(crate) round: bool,
}

/// A mod block's screen over its menu (`native_screen`): its size, where
/// each of the container's slots is (`Slot.x`, `Slot.y`; none hides the
/// slot, and so do slots past the list; the server's menu does not have a
/// hidden slot either, so clicks and shift-clicks skip it), and the player inventory's first
/// row (vanilla's is 84; the hotbar is 58 below it) and left column, from
/// its top left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeScreen {
    /// Vanilla's container screens are 176 wide.
    pub width: f32,
    pub height: f32,
    pub slots: Vec<Option<[f32; 2]>>,
    pub inventory_y: f32,
    /// The inventory's left column (vanilla's is 8).
    pub inventory_x: f32,
    /// The mod draws the title and the "Inventory" label itself
    /// (`native_screen_draw`); Horizon leaves them out.
    pub own_labels: bool,
}

/// The open screen, for the screen hooks (`native_screen_render_post`, the
/// mouse ones `native_screen_mouse_button_pressed_pre`, `..._released_pre` and
/// `native_screen_mouse_scrolled_pre`, and the key ones
/// `native_screen_key_pressed_pre` and `native_screen_char_typed_pre`): what
/// it is, its menu and title, where its container panel is, how many slots its
/// menu has, and where the mouse and the frame's partial tick are.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct NativeScreenView<'a> {
    /// One id per `Screen` (`Screen::id`): `inventory`, `creative`, `paused`,
    /// `stats`, `options`, `win`… Stable across releases, so a mod can match
    /// on it. Every container, station and mod window draws on the inventory
    /// screen and so reports `inventory`: tell them apart by `menu` and
    /// `title` (both None on a window the client opened itself) and by
    /// `slots`.
    pub id: &'static str,
    /// The type id of the menu the server opened for this screen
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, a mod's own id; a mount's
    /// inventory is `horizon:mount<n>`). None on a screen without one (the
    /// client's own inventory, creative, the pause menu, stats…) and for a
    /// menu type the client does not know.
    pub menu: Option<&'a str>,
    /// The title the server gave the open menu, if it has one: the component
    /// itself, not a copy (`.plain()` it to draw, `chars()` for the styles).
    /// None when the server gave none, on a screen the client opened, and
    /// while playing.
    pub title: Option<&'a Text>,
    /// The screen's container panel in GUI pixels, `[x, y, width, height]`;
    /// the whole window for a screen with none. The panel is where the game
    /// draws the window: 176 wide for most, wider for the merchant and the
    /// beacon, a mod screen's own size for a mod's.
    pub rect: [f32; 4],
    /// The slots the open menu has, as `Game::slots` counts them: the player
    /// inventory's 36, then the container's, the station's, the mod screen's
    /// or the crafting grid's (a screen with no menu of its own still counts
    /// the inventory menu's). Counted, not listed.
    pub slots: usize,
    /// The pointer in GUI pixels (`Screen.mouseX`, `mouseY`, from the
    /// window's top left), as `native_cursor` gives it: for hover highlights.
    pub mouse: [f32; 2],
    /// This frame's partial tick (`getGameTimeDeltaPartialTick(false)`, 0 to
    /// 1): what the screen draws animation against.
    pub partial_tick: f32,
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
pub(crate) struct NativeClock {
    /// Ticks since the world was created (`Level.getGameTime`).
    pub(crate) game_time: i64,
    /// The time of day in ticks (`Level.getOverworldClockTime`): 0 at sunrise, 24000 a
    /// day. It keeps counting past 24000 (`/time add`, days going by), so take
    /// it modulo 24000 for the hour; `/time set` can make it negative.
    pub(crate) day_time: i64,
}

/// A native mod as Options > Mods lists it (`native_mods`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeMod {
    /// Its id: its settings folder and its keys' lines.
    pub(crate) id: String,
    /// `Name 1.0`.
    pub(crate) title: String,
    pub(crate) description: String,
    /// Its keys: name, label and default key (`key.keyboard.z`). The page
    /// lists them first; `native_mod_key` reads the player's choice.
    pub(crate) keys: Vec<[String; 3]>,
}

/// An item the panel shows (`native_panel_items`): its id
/// (`minecraft:diamond`), a count (1 draws none) and its components beyond
/// the item's own, in vanilla's item syntax, or "".
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeStack {
    pub(crate) item: String,
    pub(crate) count: u32,
    pub(crate) components: String,
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

/// What the player did on a native mod's panel (`native_panel_event`).
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NativePanelEvent {
    /// A click on an item of the panel, by its index in
    /// `native_panel_items`' list.
    Clicked { index: u32, right: bool },
    /// The search box's text, each time the player changes it.
    Search(String),
    /// A key (`key.keyboard.r`; compare it with `native_mod_key`) went down
    /// while the pointer was on an item: a slot of the screen's, or the
    /// panel's.
    Key { key: String, stack: NativeStack },
    /// A click on a slot of a recipe the panel shows: the recipe's index in
    /// `native_panel_recipes`' list and the slot's in it (the results come
    /// after the slots), and the stack it showed then.
    SlotClicked {
        recipe: u32,
        slot: u32,
        stack: NativeStack,
        right: bool,
    },
    /// The action button of the recipe at this index.
    Action(u32),
    /// The player turned the recipes' page: the index of the first recipe
    /// now shown.
    Page(u32),
    /// The recipe page's back button.
    Back,
    /// The player picked the tab at this index.
    Tab(u32),
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
    /// Once a frame, in a world or not, after Horizon's extras.
    pub(super) fn native_frame(&mut self, elapsed: Duration) {
        let _ = elapsed;
    }
    /// Render-thread notification after a resource-pack reload has applied successfully.
    /// Atlas, models, renderer and client caches already refer to the new resources.
    /// Selection/F3+T and server-pack reloads share this point; initial startup does not.
    pub(super) fn native_client_resources_reloaded(&mut self) {}
    /// Once a tick, in every `Game`: the window's (in single player, on LAN
    /// and on servers), and the server's own (`native_window` is false
    /// there: the single player server thread, a dedicated server).
    pub(super) fn native_tick(&mut self) {}
    /// Before an eligible targeted block's selection outline is built.
    /// Cancel skips only the default block highlight, after normal HUD and game-mode gates.
    pub(super) fn native_block_highlight(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        partial: f32,
        canceled: &mut bool,
    ) {
        let _ = (pos, state, partial, canceled);
    }
    /// The world frame's local visual sources. Push requests; Horizon validates
    /// and limits them before rendering. Called only while a world is visible.
    pub(super) fn native_visuals(
        &self,
        rims: &mut Vec<NativeBlockRim>,
        lights: &mut Vec<NativeColoredLight>,
    ) {
        let _ = (rims, lights);
    }
    /// At most 32 pairs of already-loaded base and transparent overlay sprites.
    /// Per-pixel alpha in the overlay marks full-bright material, without
    /// illuminating the surrounding block or changing gameplay light.
    pub(super) fn native_emissive(&self, layers: &mut Vec<(&'static str, &'static str)>) {
        let _ = layers;
    }
    /// The world's phases, read-only; `error_text` carries the engine's
    /// own open-failure text, empty otherwise. Server-side; the deprecated
    /// in-frame path delivers no lifecycle events. A server loop or final
    /// save failure fires `WorldCloseFailed` with the error text.
    pub(super) fn native_world_lifecycle(
        &mut self,
        context: &NativeLifecycleContext,
        phase: WorldPhase,
        error_text: &str,
    ) {
        let _ = (context, phase, error_text);
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
        let _ = dedicated;
    }
    /// The server begins an orderly stop (NeoForge's `ServerStoppingEvent`):
    /// the start of vanilla's `MinecraftServer.stopServer`, with the worlds
    /// still loaded and the players still connected and not yet saved, so a
    /// mod can save its own data or tell them. `native_world_lifecycle`'s
    /// `WorldCloseFinal` follows once the worlds are saved. A server whose loop
    /// fails stops without this (`WorldCloseFailed` reports it). API 44.
    pub(super) fn native_server_stopping(&mut self, dedicated: bool) {
        let _ = dedicated;
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
        let _ = (context, phase);
    }
    /// Before an authoritative server level's tick pass. `dimension` is its ID.
    pub(super) fn native_level_tick_pre(&mut self, dimension: &str) {
        let _ = dimension;
    }
    /// After a successful authoritative server level tick pass.
    pub(super) fn native_level_tick_post(&mut self, dimension: &str) {
        let _ = dimension;
    }
    /// The HUD (not drawn under F1), after the minimap: place a box with
    /// `corners.place` (boxes stack away from their edge) and draw it with
    /// `native_fill` and `native_text`. Skip a corner `native_corner_shown`
    /// says is taken.
    pub(super) fn native_hud(&mut self, batch: &mut HudBatch, corners: &mut Corners) {
        let _ = (batch, corners);
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
        let _ = (entity, kind, stack, claimed);
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
        let _ = (pos, face, state, canceled);
    }
    /// A living player's server tick ends (`ServerPlayer.tick`): once a tick
    /// for every player in the world, after their digging, item use,
    /// cooldowns, statistics and advancements. The helpers read that player.
    /// Players still tick under `/tick freeze`; a dead player doesn't, nor
    /// does one killed earlier in the same tick. API 31.
    pub(super) fn native_player_tick(&mut self) {}
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
        let _ = (inputs, wi, hi, draws);
    }
    /// A key or mouse button went down or up, by its vanilla name
    /// (`key.keyboard.z`, `key.mouse.left`): keys on any screen (check
    /// `native_playing`), mouse buttons while playing. Repeats are left out.
    pub(super) fn native_key(&mut self, key: &str, pressed: bool) {
        let _ = (key, pressed);
    }
    /// Wheel steps while playing: set `claimed` to keep them from the hotbar.
    pub(super) fn native_wheel(&mut self, steps: f32, claimed: &mut bool) {
        let _ = (steps, claimed);
    }
    /// A command typed or clicked in chat, without its slash, before the
    /// server and Horizon's commands see it: set `ran` when the mod ran it.
    pub(super) fn native_command(&mut self, command: &str, ran: &mut bool) {
        let _ = (command, ran);
    }
    /// The commands `native_command` runs, by name and without their slash:
    /// push the ones that are yours. Asked when a connection starts and
    /// again when the server sends its own tree, so the chat box colours,
    /// completes and lists them as the game's own; a name the game already
    /// has keeps that command's behaviour. API 35.
    pub(super) fn native_commands(&mut self, commands: &mut Vec<&'static str>) {
        let _ = commands;
    }
    /// The zoom a native mod asks for, applied while playing (the spyglass's
    /// own view wins): set `zoom` when it is none yet.
    pub(super) fn native_zoom(&self, zoom: &mut Option<NativeZoom>) {
        let _ = zoom;
    }
    /// The camera shake native mods ask for in this frame, `partial` ticks
    /// past the last tick (0 to 1): add your pitch, yaw and roll in degrees
    /// and your offset in blocks, along the camera's right, up and back, to
    /// `shake`, so several mods' shakes add up. Read while playing only;
    /// Horizon scales the sum by Distortion Effects and bounds it
    /// (`native_shake_matrix`). API @API:native_camera_shake@.
    pub(super) fn native_camera_shake(&self, partial: f32, shake: &mut NativeShake) {
        let _ = (partial, shake);
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
        let _ = (hand, stack, partial, swing, equip, canceled);
    }

    /// The values native mods hand their shader packs in this frame,
    /// `partial` ticks past the last tick (0 to 1): a pack's WGSL reads
    /// them as `frame.mods` (eight `vec4<f32>`). Write the slots your own
    /// pack reads; a mod that bundles its pack as `native-<mod id>` is the
    /// one that pack is for. Asked once a frame while a pack is on; a value
    /// that isn't finite reads as 0 (`native_shader_params_now`). API
    /// @API:native_shader_params@.
    pub(super) fn native_shader_params(&self, partial: f32, params: &mut [[f32; 4]; 8]) {
        let _ = (partial, params);
    }
    /// The minimap a native mod shows: set `minimap` when it is none yet.
    pub(super) fn native_minimap(&self, minimap: &mut Option<NativeMinimap>) {
        let _ = minimap;
    }
    /// Register namespaced NBT component types before content loads.
    pub(super) fn native_component_types(types: &mut Vec<String>) {
        let _ = types;
    }
    /// Options > Mods lists the native mods: push yours.
    pub(super) fn native_mods(&mut self, mods: &mut Vec<NativeMod>) {
        let _ = mods;
    }
    /// The buttons of mod `mod_id`'s settings page, under its keys: push
    /// their labels when it is yours. Asked again each time the page opens
    /// and after each click.
    pub(super) fn native_settings(&mut self, mod_id: &str, buttons: &mut Vec<String>) {
        let _ = (mod_id, buttons);
    }
    /// Button `button` of mod `mod_id`'s settings page was clicked.
    pub(super) fn native_settings_click(&mut self, mod_id: &str, button: usize) {
        let _ = (mod_id, button);
    }
    /// The player did `event` on mod `mod_id`'s panel (`native_panel_items`):
    /// act on it when it is yours. API 2.
    pub(super) fn native_panel_event(&mut self, mod_id: &str, event: &NativePanelEvent) {
        let _ = (mod_id, event);
    }
    /// Block `block` (`mymod:machine`) gets a block entity
    /// (`EntityBlock.newBlockEntity`): set `entity` to its id and saved
    /// compound when the block is yours and `entity` is none yet. The block's
    /// template must have a block entity (a chest, a furnace...), so that
    /// breaking the block removes it; on another template it gets none. API 15.
    pub(super) fn native_block_entity(
        &mut self,
        block: &str,
        entity: &mut Option<(String, Compound)>,
    ) {
        let _ = (block, entity);
    }
    /// Whether block `block`'s block entity ticks (`EntityBlock.getTicker`):
    /// set `ticks` when it is yours. Asked for each of the block's states
    /// when a world opens. API 15.
    pub(super) fn native_block_entity_ticks(&self, block: &str, ticks: &mut bool) {
        let _ = (block, ticks);
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
        let _ = (enchantment, kind, data, level, victim);
    }
    /// Block `block`'s block entity has item slots (a `WorldlyContainer`):
    /// set `container` when the block is yours. Asked once for each mod
    /// block when the ticking table is built; at most 256 slots, and slot
    /// numbers past the count are left out. Its `menu`, one of
    /// `containers::MOD_MENUS` or a content mod's menu type
    /// (`data/<id>/horizon/menu/`, API @API:mod_menu_types@) with as many
    /// slots, opens when the block is used (its template must open a menu, as
    /// a chest or a furnace does); another is left out. A mod menu type shows
    /// on Horizon clients through the mod's `native_screen` only. API 17.
    pub(super) fn native_block_entity_container(
        &self,
        block: &str,
        container: &mut Option<ModContainer>,
    ) {
        let _ = (block, container);
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
        let _ = (block, fluids);
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
        let _ = (pos, entity);
    }
    /// A consumable's effect of a mod's own kind: the item's `on_consume_effects`
    /// entry is of a type in a namespace of its own, and the loader kept it
    /// for this call. The kind and the JSON the file wrote are here; the acting
    /// player is the one consuming, and the game's own effects,
    /// sounds, teleports and drops are open. An unclaimed kind does
    /// nothing. API 74.
    pub(super) fn native_consume_effect(&mut self, kind: &str, data: &serde_json::Value) {
        let _ = (kind, data);
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
        let _ = (pos, entity, tag);
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
        let _ = (pos, block, state, entity, draw);
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
        let _ = (pos, block, state, entity, bounds);
    }
    /// A menu opens on this client for block `block`, a mod's: set `screen`
    /// when the block is yours, to lay its screen out (`native_screen_draw`
    /// draws it). The server asks it too: a slot it hides is not in the menu
    /// there either, so shift-clicks never fill it. API 19.
    pub(super) fn native_screen(&self, block: &str, screen: &mut Option<NativeScreen>) {
        let _ = (block, screen);
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
        let _ = (block, data, batch, origin);
    }
    /// The open screen has drawn itself and its tooltips (`Screen.render`,
    /// the point of `ScreenEvent.Render.Post`): draw over it with
    /// `native_fill`, `native_sprite` and `native_text`, `screen.rect` being
    /// its container panel (or the whole window) and `screen.id` the screen
    /// it is. Once a frame on the window, never while playing with no screen;
    /// `&self`, so it reads and draws only. The Win and Options screens call
    /// it on their own path, after their drawing.
    ///
    /// It does not fire on the title screen, which returns before its point,
    /// nor for the screens drawn over gameplay after it (a server dialog, the
    /// server pack, link and code-of-conduct prompts, a book, a sign editor):
    /// those keep `Screen::Playing`, which it skips, and draw over what it
    /// drew, while vanilla's `ScreenEvent.Render.Post` fires for them.
    /// API 78.
    pub(super) fn native_screen_render_post(
        &self,
        batch: &mut HudBatch,
        screen: &NativeScreenView<'_>,
    ) {
        let _ = (batch, screen);
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
    /// only, and only while a screen of the game's is open (Options and the
    /// pack list opened from the title screen count, with no player and no
    /// `menu`). It does not fire on the title screen itself, the connect and
    /// loading screens, nor for a server
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
        let _ = (screen, button, modifiers, canceled);
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
        let _ = (screen, button, modifiers, canceled);
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
    /// only while a screen of the game's is open (Options and the pack list
    /// opened from the title screen count, with no player and no `menu`). It
    /// does not fire on the title screen itself, the connect and loading
    /// screens, nor for a server dialog, the server pack, link and
    /// code-of-conduct prompts, a book or a sign editor drawn over gameplay,
    /// nor while a world loads or saves: the scroll there stays the game's
    /// own. A wheel a gamepad or the keyboard's focus sends is not the mouse
    /// and fires nothing.
    /// API @API:screen_mouse_scrolled@.
    pub(super) fn native_screen_mouse_scrolled_pre(
        &self,
        screen: &NativeScreenView<'_>,
        scroll: [f64; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        let _ = (screen, scroll, modifiers, canceled);
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
    /// keys and the narrator — and only while a screen of the game's is open
    /// (Options and the pack list opened from the title screen count, with no
    /// player and no `menu`). It does not fire on the title screen itself, the
    /// connect and loading screens, nor for a server dialog, the server pack,
    /// link and code-of-conduct prompts, a book or a sign editor drawn over
    /// gameplay, nor while a world loads or saves: the key there stays the
    /// game's own. A cancelled press's release still runs, so a `native_key`
    /// listener sees that release without its press, and its characters are
    /// GLFW's separate char callback and still come; see
    /// `native_screen_char_typed_pre`.
    /// API @API:screen_key_input@.
    pub(super) fn native_screen_key_pressed_pre(
        &self,
        screen: &NativeScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        let _ = (screen, key, repeat, modifiers, canceled);
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
    /// API @API:screen_key_input@.
    pub(super) fn native_screen_char_typed_pre(
        &self,
        screen: &NativeScreenView<'_>,
        c: char,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        let _ = (screen, c, modifiers, canceled);
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
    /// A screen the native mods' input hooks are for is open: the window has a
    /// renderer, `Screen::Playing` is behind it (so not the title screen, nor a
    /// book, a sign editor, a server dialog or a prompt drawn over gameplay)
    /// and no connect or world load is in progress. The one gate they all ask
    /// before they fire.
    pub(super) fn screen_hooks_open(&self) -> bool {
        self.renderer.is_some()
            && self.screen != Screen::Playing
            && !self.screen_over_playing()
            && !self.remote_loading()
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
        Some(self.screen_view(&batch, self.screen))
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
        let _ = (block, data, at, right, button);
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
        let _ = (block, data, at, lines);
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
        let _ = (pos, entity, id);
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
        let _ = (pos, entity, id, carried);
    }
    /// The commands native mods add to the server's command tree: push
    /// yours. Asked once when a world opens to players (a dedicated
    /// server, a LAN host, single player's server thread); every player's
    /// chat box then completes them as vanilla's, within their level. A
    /// name vanilla has is left out. API 8.
    pub(super) fn native_register_commands(&mut self, commands: &mut Vec<NativeCommand>) {
        let _ = commands;
    }
    /// A player's command on the server (`Commands.performCommand`),
    /// without its slash, for a command a mod registered and the player's
    /// level allows: set `ran` when it is yours. The player's state is
    /// swapped in for the call, so `native_player` and the other helpers
    /// read that player; `native_feedback` answers them. API 8.
    pub(super) fn native_perform_command(
        &mut self,
        player: &NativeServerPlayer,
        command: &str,
        ran: &mut bool,
    ) {
        let _ = (player, command, ran);
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
    /// rate limit) never reaches it. @API:native_player_command@.
    pub(super) fn native_player_command(
        &mut self,
        player: u128,
        name: &str,
        command: &str,
        canceled: &mut bool,
        replies: &mut Vec<Text>,
    ) {
        let _ = (player, name, command, canceled, replies);
    }
    /// A command a mod registered, typed on the dedicated server's console
    /// or sent through `horizon-server console`
    /// (`DedicatedServer.handleConsoleInputs`), without its slash: set `ran`
    /// when it is yours. There is no player: the source is the console, at
    /// permission level 4, so `native_acting_player` is None and the
    /// player-only helpers give nothing; `native_feedback` answers on the
    /// console. A name no mod registered goes to vanilla's dispatcher as
    /// before. @API:native_perform_console_command@.
    pub(super) fn native_perform_console_command(&mut self, command: &str, ran: &mut bool) {
        let _ = (command, ran);
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
        let _ = (uuid, window, slot, button, mode);
    }
    /// A menu a mod opened closed: the player closed it, the server or a mod
    /// opened another, or the player left. API 10.
    pub(super) fn native_menu_closed(&mut self, uuid: u128, window: i32) {
        let _ = (uuid, window);
    }
    /// An item's tooltip, wherever the game shows one (slots, the creative
    /// tabs, the recipe book, chat's hover): the stack, with its components
    /// (`stack.data()`: a shulker box's contents...), and its lines, the name
    /// first, each with a colour as `rgba` packs it (`u32::MAX` keeps the
    /// text's own). Add, change or take out lines; `Text::legacy` reads `§`
    /// codes. Asked each frame the tooltip shows, never for one the stack
    /// hides. API 12.
    pub(super) fn native_tooltip(&self, stack: &ItemStack, lines: &mut Vec<(Text, u32)>) {
        let _ = (stack, lines);
    }
    /// Client-side image component for this stack's tooltip. Answer its width/height
    /// in GUI pixels; finite positive dimensions reserve space after text and any
    /// vanilla bundle component. None keeps the vanilla tooltip unchanged.
    pub(super) fn native_tooltip_component(&self, stack: &ItemStack, size: &mut Option<[f32; 2]>) {
        let _ = (stack, size);
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
        let _ = (stack, batch, origin, size);
    }

    /// The small bar `GuiGraphicsExtractor.itemDecorations` draws under an
    /// item, wherever the game shows one: `bar` arrives holding vanilla's value
    /// (the damage bar, or a bundle's fullness already applied) and may be
    /// replaced (width `0..=13`, colour `rgb`) or set to `None` to hide it.
    /// Asked each frame a stack is drawn. Client-side. 60.
    pub(super) fn native_item_bar(&self, stack: &ItemStack, bar: &mut Option<(i32, u32)>) {
        let _ = (stack, bar);
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
        let _ = (stack, x, y, batch);
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
        let _ = (stack, pos, face, claimed);
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
        facing: horizon_core::geometry::Direction,
        stack: &ItemStack,
        answer: &mut Option<super::dispenser::NativeDispensed>,
    ) {
        let _ = (dimension, pos, facing, stack, answer);
    }
    /// A player uses `stack` (right click) with `self.hand`: in the air, or
    /// on a block once `native_use_item_on` let it through, where `Item.use`
    /// starts. Each click reaches it once. Set `claimed` to take the use: the
    /// hand swings and the item's own use doesn't run. Server-side, for every
    /// player of the world; an item on cooldown is never used. API 13.
    pub(super) fn native_use_item(&mut self, stack: &ItemStack, claimed: &mut bool) {
        let _ = (stack, claimed);
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
    /// it is asked so often: answer from the stack alone. API @API:item_use_over_time@.
    pub(super) fn native_use_duration(&self, stack: &ItemStack, duration: &mut i32) {
        let _ = (stack, duration);
    }
    /// `Item.getUseAnimation` of `stack` in use, for the first person hand:
    /// `animation` arrives with its consumable's (`eat`, `drink`), else
    /// `None`. `Eat` and `Drink` bring the food to the mouth, `Bow` draws,
    /// `Trident` raises, `Brush` sweeps; the rest hold the item still.
    /// Asked each frame the hand draws an item in use, for an item with no
    /// pose of its own (bows, crossbows, tridents, brushes and spears keep
    /// theirs). Client-side. API @API:item_use_over_time@.
    pub(super) fn native_use_animation(&self, stack: &ItemStack, animation: &mut UseAnimation) {
        let _ = (stack, animation);
    }
    /// `Player.canEat` for the food `stack`, from `Consumable.canConsume`
    /// when eating starts: `can_eat` arrives with vanilla's (creative,
    /// `can_always_eat`, or hungry). Clear it to refuse the food, set it to
    /// let a full player eat. Both sides (the client starts eating as the
    /// server does), so answer from the stack and the player alone. API @API:item_use_over_time@.
    pub(super) fn native_can_eat(&self, stack: &ItemStack, can_eat: &mut bool) {
        let _ = (stack, can_eat);
    }
    /// NeoForge's `LivingEntityUseItemEvent.Start`: `entity` starts using
    /// `stack` in `hand` (`LivingEntity.startUsingItem`), for `duration`
    /// ticks (`native_use_duration`'s answer). `entity` is a mob's
    /// `Entity.getId`, None for a player (`native_acting_player` says who);
    /// only players fire it so far. Change `duration` (consumables other
    /// than potions, and mods' items, count to it; 0 or less lasts until let
    /// go), or set `canceled` and the use doesn't start. Both sides, as
    /// vanilla's: wherever the world runs (the server, single player
    /// in-frame, a LAN host) for every player of the world, and on a
    /// server's client for the uses it predicts. Answer the same on both,
    /// and change the world only where this `Game` runs the world, not on a
    /// server's client: no helper tells the two apart yet (`native_window`
    /// is also true in-frame and on an in-frame LAN host). API @API:item_use_over_time@.
    pub(super) fn native_living_entity_use_item_start(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: &mut i32,
        canceled: &mut bool,
    ) {
        let _ = (stack, entity, hand, duration, canceled);
    }
    /// NeoForge's `LivingEntityUseItemEvent.Finish`: `entity`'s use of
    /// `stack` in `hand` ran its time (`LivingEntity.completeUsingItem`),
    /// `duration` the ticks left, always 0. `entity` is as in
    /// `native_living_entity_use_item_start`. `result` arrives with what
    /// `finishUsingItem` left in the hand (the bowl of a stew, none for the
    /// last apple; a mod's own item is kept) and may be replaced. Fired for
    /// consumables, potions and mods' items. Server-side, for every player
    /// of the world (the helpers read the acting one). API @API:item_use_over_time@.
    pub(super) fn native_living_entity_use_item_finish(
        &mut self,
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: i32,
        result: &mut Option<ItemStack>,
    ) {
        let _ = (stack, entity, hand, duration, result);
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
        let _ = (pos, state, canceled);
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
        let _ = (stack, pos, state);
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
        let _ = (pos, face, canceled);
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
        let _ = (dimension, center, blocks);
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
        let _ = (stack, pos, canceled);
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
        let _ = (entity, kind, pos, canceled);
    }
    /// A message from `player`'s client on `channel` (`mod_id:name`), within
    /// the caps (`native_net.rs`): check `channel`, and treat `data` as the
    /// player's own bytes, checked by nothing. Runs in the tick after it
    /// arrives, with no player's state swapped in. Server-side. API 40.
    pub(super) fn native_net_from_client(&mut self, player: u128, channel: &str, data: &[u8]) {
        let _ = (player, channel, data);
    }
    /// A message from the server on `channel` (`mod_id:name`), whose `data`
    /// is the sender's own bytes. The window's game only. API 40.
    pub(super) fn native_net_from_server(&mut self, channel: &str, data: &[u8]) {
        let _ = (channel, data);
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
        let _ = (dimension, chunk, new);
    }
    /// Chunk `chunk` of `dimension` left the world (NeoForge's
    /// `ChunkEvent.Unload`), with a level that closes for being empty too. Don't
    /// read its blocks. Its block entities are still there for this call, so
    /// `native_with_mod_block_entity` can change one and it's saved with the
    /// chunk. Not told when the world closes, which `native_world_lifecycle`
    /// tells. Server-side. API 57.
    pub(super) fn native_chunk_unload(&mut self, dimension: &str, chunk: [i32; 2]) {
        let _ = (dimension, chunk);
    }
    /// `player` (`name`) joined the game (NeoForge's
    /// `PlayerLoggedInEvent`): placed in the world and listed in
    /// `native_server_players`, so `native_net_send_to_player` reaches them.
    /// Once a join, with no player's state swapped in. Server-side. API 57.
    pub(super) fn native_player_logged_in(&mut self, player: u128, name: &str) {
        let _ = (player, name);
    }
    /// `player` (`name`) left the game (NeoForge's `PlayerLoggedOutEvent`), by
    /// any path: a lost connection, a kick, the server stopping or failing.
    /// Once for each player `native_player_logged_in` told of. They've already
    /// left `native_server_players`, except when the server stops, which tells
    /// before it saves, with them still listed and able to get a last message.
    /// Server-side. API 57.
    pub(super) fn native_player_logged_out(&mut self, player: u128, name: &str) {
        let _ = (player, name);
    }
    /// `player` died: where `ServerPlayer.die` starts, with the death
    /// message's key when the game knows it (`death.attack.lava`; none for a
    /// message it builds from the combat). It only hears. Server-side, for
    /// every player of the world, whether death messages are shown or not.
    /// API 40.
    pub(super) fn native_player_died(&mut self, player: u128, cause: Option<&str>) {
        let _ = (player, cause);
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
        let _ = (dimension, kind, pos, natural);
    }
    /// An entity leaves the server level `dimension` (NeoForge's
    /// `EntityLeaveLevelEvent`): vanilla's `onTrackingEnd`, when it is removed
    /// for any reason (picked up, merged, despawned, burnt, blown up, killed,
    /// taken by a hopper or a mob, gone through a portal) or its chunk unloads,
    /// a level closing with nobody in it included. Not for the entities still
    /// loaded when the server stops, as in vanilla. `entity` is its ID and
    /// `kind` its type (`minecraft:item`); it is no longer in the level's lists.
    /// It only hears. 53 tells item entities and experience orbs; later
    /// APIs add other kinds, so check `kind`. Server-side.
    pub(super) fn native_entity_leave_level(&mut self, dimension: &str, entity: u32, kind: &str) {
        let _ = (dimension, entity, kind);
    }
    /// An entity joins the server level `dimension` (NeoForge's
    /// `EntityJoinLevelEvent`): vanilla's `PersistentEntitySectionManager.addEntity`.
    /// `loaded_from_disk` is true for an entity coming back with its chunk,
    /// false for one added fresh (`addFreshEntity`: a drop, an orb, `/summon`,
    /// an arrival through a portal). `entity` is its ID and `kind` its type
    /// (`minecraft:item`). It is already in the level's lists, so helpers that
    /// take an entity ID find it. Set `canceled` to keep it out: it is taken
    /// back out as if never added, and a `/summon` of it fails. 47 tells
    /// item entities and experience orbs; later APIs add other kinds, so check
    /// `kind`. Server-side.
    pub(super) fn native_entity_join_level(
        &mut self,
        dimension: &str,
        entity: u32,
        kind: &str,
        pos: [f64; 3],
        loaded_from_disk: bool,
        canceled: &mut bool,
    ) {
        let _ = (dimension, entity, kind, pos, loaded_from_disk, canceled);
    }
    /// `player`'s melee blow killed the mob `entity` of `kind` (other deaths
    /// aren't told yet). It only hears, after the blow landed. Server-side.
    /// API 40.
    pub(super) fn native_mob_killed(&mut self, player: u128, entity: u32, kind: &str) {
        let _ = (player, entity, kind);
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
        let _ = (context, drops);
    }
    /// `player` threw `count` of `item` (`Player.drop`: the drop key, a
    /// menu's throw...), not what scatters when they die. It only hears.
    /// Server-side. API 40.
    pub(super) fn native_item_dropped(&mut self, player: u128, item: &str, count: u32) {
        let _ = (player, item, count);
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
        let _ = (player, item, count, canceled);
    }
    /// A player's hit landed on the mob `entity` of type `kind`
    /// (`minecraft:zombie`), holding `stack`: where `Item.hurtEnemy` is called,
    /// before the post-attack enchantments and the weapon's wear. It only
    /// hears. Another player as the target isn't told yet. Server-side, for
    /// every player of the world (the helpers read the acting one). API 39.
    pub(super) fn native_hurt_enemy(&mut self, stack: &ItemStack, entity: u32, kind: &str) {
        let _ = (stack, entity, kind);
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
        let _ = (slot, selected);
    }
    /// A player's equipment slot `slot` (`head`, `chest`, `legs`, `feet`,
    /// `offhand`, and `mainhand` on a swap of the hands or `/item`) took a different
    /// item, `new` (none when it was emptied): `LivingEntity.onEquipItem`,
    /// before the equip sound. It only hears. Never for a spectator.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one). API 39.
    pub(super) fn native_on_equip(&mut self, slot: &str, new: Option<&ItemStack>) {
        let _ = (slot, new);
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
        let _ = (effect, tick_count, amplifier, apply);
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
        let _ = (effect, dimension, entity, amplifier, keep);
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
        let _ = (effect, entity, amplifier);
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
        let _ = (effect, entity, amplifier);
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
    /// on the thread that ticks the world (the server thread; the window's
    /// in-frame): state shared with render thread hooks needs synchronising.
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
        let _ = (entity, kind, damage_type, causing, direct, amount, canceled);
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
        let _ = (dimension, entity, kind, state, position);
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
        let _ = (dimension, entity, kind, state, position);
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
        let _ = (dimension, entity, kind, state, position, distance, damage);
    }
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
    /// (API @API:mod_particle_types@): Horizon clients draw it from the sprite
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
    /// go off or switch on. API @API:native_neighbor_signal@.
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
    /// a server, the server's copy on a client. API @API:native_world_state@.
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
    /// no weather (`Level.canHaveWeather`). API @API:native_world_state@.
    pub(super) fn native_weather(&self) -> (f32, f32) {
        if self.dim != super::Dim::Overworld {
            return (0.0, 0.0);
        }
        self.world.weather.levels(1.0)
    }
    /// The world's time of day in ticks (`Level.getOverworldClockTime`), the
    /// same as `native_clock`'s `day_time`; it keeps counting past 24000, so
    /// take it modulo 24000 for the hour. API @API:native_world_state@.
    pub(super) fn native_day_time(&self) -> i64 {
        self.world.day_time
    }
    /// The lowest block y and the height of the world in place
    /// (`LevelHeightAccessor.getMinY` and `getHeight`): how deep a quarry or a
    /// miner may dig. API @API:native_world_state@.
    pub(super) fn native_world_height(&self) -> (i32, i32) {
        (self.world.shape.min_y, self.world.shape.height)
    }
    /// The most of the item `item` one slot holds (`Item.getDefaultMaxStackSize`),
    /// or `None` for an unknown item: for a mod's container that fills or
    /// merges stacks itself. API @API:native_world_state@.
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
    /// @API:native_local_sound@.
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
    /// on the title screen. API @API:native_clock@.
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
    /// A native mod's own folder: `horizon-mods/.horizon-mod-data/<id>/`,
    /// where the WebAssembly mod of the same id kept its storage, so a
    /// player's settings carry over. Not created here.
    pub(super) fn native_data_dir(&self, mod_id: &str) -> PathBuf {
        let mods = Self::options_path().with_file_name(mod_list::DIR);
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
    /// `self`, so it works from any hook (`Game::native_config_dir()`), the
    /// level's too. API 26.
    pub(super) fn native_config_dir() -> Result<PathBuf, String> {
        config_dir(&Self::options_path())
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
    /// the world's data packs'. API 36.
    pub(super) fn native_recipes_of_type(&self, kind: &str) -> &[CustomRecipe] {
        if self.outside_world() {
            return &[];
        }
        self.recipes().custom.get(kind).map_or(&[], Vec::as_slice)
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
            .1
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
    /// call); none outside a world. They are the window's own: the world's
    /// in single player, vanilla's on a server, which sends a client no
    /// others.
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
        for i in from..self.mobs.len() {
            let m = &self.mobs[i];
            let (kind, pos) = (mob_kind(m), [m.position.x, m.position.y, m.position.z]);
            self.native_mob_spawned(self.dim.id(), &kind, pos, natural);
        }
    }
    /// `native_entity_leave_level` for each of `entities`, of `kind`, gone
    /// from the level in place. A client of a server tells nothing.
    pub(super) fn native_entities_left(&mut self, kind: &str, entities: &[u32]) {
        if self.remote.is_some() {
            return;
        }
        let dimension = self.dim.id();
        for &entity in entities {
            self.native_entity_leave_level(dimension, entity, kind);
        }
    }
    /// The IDs of the level's item entities, for `native_items_left_since`
    /// (none on a client of a server, which tells nothing).
    pub(super) fn dropped_ids(&self) -> Vec<u32> {
        if self.remote.is_some() {
            return Vec::new();
        }
        self.dropped.iter().map(|e| e.id).collect()
    }
    /// The IDs of the level's experience orbs, for `native_orbs_left_since`.
    pub(super) fn orb_ids(&self) -> Vec<u32> {
        if self.remote.is_some() {
            return Vec::new();
        }
        self.orbs.iter().map(|o| o.id).collect()
    }
    /// The item entities of `before` gone since have left the level.
    pub(super) fn native_items_left_since(&mut self, before: &[u32]) {
        let now: std::collections::HashSet<u32> = self.dropped.iter().map(|e| e.id).collect();
        let gone: Vec<u32> = before
            .iter()
            .copied()
            .filter(|id| !now.contains(id))
            .collect();
        self.native_entities_left("minecraft:item", &gone);
    }
    /// The experience orbs of `before` gone since have left the level.
    pub(super) fn native_orbs_left_since(&mut self, before: &[u32]) {
        let now: std::collections::HashSet<u32> = self.orbs.iter().map(|o| o.id).collect();
        let gone: Vec<u32> = before
            .iter()
            .copied()
            .filter(|id| !now.contains(id))
            .collect();
        self.native_entities_left("minecraft:experience_orb", &gone);
    }
    /// `native_entity_join_level` for `entity`, just put at the end of its
    /// list: false when a mod canceled, and the caller takes it back out.
    /// A client of a server tells nothing.
    pub(super) fn native_entity_joins(
        &mut self,
        entity: u32,
        kind: &str,
        pos: [f64; 3],
        loaded_from_disk: bool,
    ) -> bool {
        if self.remote.is_some() {
            return true;
        }
        let mut canceled = false;
        let dimension = self.dim.id();
        self.native_entity_join_level(
            dimension,
            entity,
            kind,
            pos,
            loaded_from_disk,
            &mut canceled,
        );
        if canceled {
            self.persist.joins_canceled += 1;
        }
        !canceled
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
        if let Some(entity) = BlockEntity::for_block(name) {
            return Some(entity);
        }
        let mut entity = None;
        self.native_block_entity(name, &mut entity);
        let (id, nbt) = entity?;
        // A template with no block entity (stone) never removes one, so it would stay in the save.
        let state = self.pack.assets.registry.default_state(name)?;
        if !self.res.facts().get(state).block_entity {
            super::mods::warn_no_block_entity(name, &id);
            return None;
        }
        Some(BlockEntity::Mod(Box::new(ModBlockEntity {
            id,
            nbt,
            ..ModBlockEntity::default()
        })))
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
    fn put_back_mod_block_entity(
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
fn config_dir(options: &Path) -> Result<PathBuf, String> {
    let dir = options.with_file_name("config");
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

#[cfg(test)]
mod tests {

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
    fn loot_table_load_stays_hookable() {
        let source = include_str!("../../../horizon-world/src/loot.rs");
        // From its doc comment's last line, so an attribute above it counts.
        let hook = source.find("pub fn native_loot_table_load(").unwrap();
        let start = source[..hook].rfind("///").unwrap();
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
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
            } else if level_hooks.contains(name) {
                format!("horizon_client::game::native_hooks::GameLevel<'_>::native_{name}")
            } else {
                format!("horizon_client::game::native_hooks::Game<'_>::native_{name}")
            };
            assert_eq!(hook["target"].as_str().unwrap(), target);
        }
        assert_eq!(names, declared);
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
    fn config_dir_is_config_beside_options() {
        // Callable without a game, from any hook.
        let _: fn() -> Result<std::path::PathBuf, String> = super::Game::native_config_dir;
        use super::config_dir;
        let root = std::env::temp_dir().join(format!("horizon-config-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let options = root.join("options.txt");
        assert!(!root.join("config").exists());
        let dir = config_dir(&options).unwrap();
        assert_eq!(dir, root.join("config"));
        assert!(dir.is_dir());
        assert_eq!(config_dir(&options).unwrap(), dir);
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
}
