//! Server-side vanilla block behaviour: `Level.setBlock` with neighbour and
//! shape-update propagation, scheduled block/fluid ticks, random ticks,
//! placement (`getStateForPlacement`) and interaction (`useWithoutItem`).
//!
//! The library is client-agnostic. A game owns its chunk storage and exposes
//! it through [`LevelAccess`]; it also owns one [`Ticks`] per level (scheduled
//! tick queues, the in-flight neighbour-update queue and the random-tick LCG).
//! [`Behaviours`] holds immutable tables loaded once from the pinned client
//! resources (`block-behaviours.json`, `gameplay-facts.json`,
//! `block-tags.json`, `cached-collision.nbt`; see
//! `crates/horizon-prepare`).
//!
//! # Driving a level
//!
//! Per server tick, in vanilla order (`ServerLevel.tick`):
//! 1. [`Behaviours::tick_scheduled`] runs due block ticks then fluid ticks
//!    (`blockTicks.tick(time, 65536, ...)`, then `fluidTicks`).
//! 2. For every ticking chunk, [`Behaviours::tick_chunk_random`] performs
//!    `ServerLevel.tickChunk`'s random ticks: for each non-empty section,
//!    `randomTickSpeed` (game rule, default 3) positions are drawn with
//!    `Level.getBlockRandomPos` and randomly-ticking block and fluid states are
//!    ticked. The caller says which sections are randomly ticking when each is
//!    reached (vanilla counts randomly-ticking states per section);
//!    [`Behaviours::random_tick`] ticks a single position.
//! 3. [`Behaviours::run_block_events`] runs queued block events (pistons),
//!    then (after entities) [`Behaviours::tick_block_entities`] advances
//!    moving pistons ([`Ticks::moving_pistons`]).
//! 4. Player actions call [`Behaviours::place_block`] /
//!    [`Behaviours::use_without_item`] / [`Behaviours::destroy_block`].
//!
//! Every block mutation goes through [`Behaviours::set_block`] with vanilla's
//! update flags ([`flags`]). The library writes storage only through
//! [`LevelAccess::set_raw`]; the caller updates heightmaps, light, block
//! entities and render meshes for the written position.
//!
//! Side effects the library cannot perform (sounds, particles, item drops,
//! spawning falling-block or TNT entities, growing trees) are reported as
//! [`LevelEvent`]s through [`LevelAccess::emit`], in vanilla call order.
//!
//! Randomness is drawn from [`LevelAccess::random`] (vanilla `level.random`, a
//! `LegacyRandomSource`) in the same order as vanilla.
//!
//! # Integration sketch
//!
//! ```ignore
//! use horizon_world::level::{self, Behaviours, LevelAccess, LevelEvent, PlaceContext, Ticks, flags};
//!
//! struct GameLevel { chunks: Chunks, random: LegacyRandom, time: i64, events: Vec<LevelEvent> }
//! impl LevelAccess for GameLevel {
//!     fn block_state(&self, pos: BlockPos) -> BlockStateId { self.chunks.get(pos) }
//!     fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
//!         let old = self.chunks.set(pos, state)?; // also mark light/heightmap/mesh dirty
//!         Some(old)
//!     }
//!     fn is_loaded(&self, pos: BlockPos) -> bool { self.chunks.contains(pos.chunk()) }
//!     fn random(&mut self) -> &mut LegacyRandom { &mut self.random }
//!     fn game_time(&self) -> i64 { self.time }
//!     fn block_light(&self, pos: BlockPos) -> i32 { self.chunks.block_light(pos) }
//!     fn sky_light(&self, pos: BlockPos) -> i32 { self.chunks.sky_light(pos) }
//!     fn emit(&mut self, event: LevelEvent) { self.events.push(event) }
//! }
//!
//! let registry = level::data::load_registry(&resources)?;
//! let behaviours = Behaviours::load(&registry, &resources)?;
//! let mut ticks = Ticks::new(seed as i32);
//! ticks.add_empty_chunk(chunk_pos)?;          // when a chunk loads (remove_chunk on unload)
//!
//! // Every server tick:
//! level.time += 1;
//! behaviours.tick_scheduled(&mut level, &mut ticks);
//! for chunk in ticking_chunks {
//!     behaviours.tick_chunk_random(&mut level, &mut ticks, chunk, |level, from| level.next_ticking_section(chunk, from));
//! }
//! behaviours.run_block_events(&mut level, &mut ticks);
//! /* tick entities */
//! behaviours.tick_block_entities(&mut level, &mut ticks);
//! for event in level.events.drain(..) { /* play sounds, spawn drops/falling blocks, grow trees */ }
//!
//! // Player actions:
//! behaviours.place_block(&mut level, &mut ticks, &PlaceContext { clicked_pos, face, hit, inside: false,
//!     yaw, pitch, sneaking, item: "minecraft:oak_stairs".into(), can_use_game_master_blocks: false });
//! behaviours.use_without_item(&mut level, &mut ticks, pos, face, hit, player_context);
//! behaviours.player_destroy_block(&mut level, &mut ticks, pos, creative, correct_tool);
//! // Falling-block entity landed:
//! behaviours.falling_block_landed(&mut level, &mut ticks, landing_pos, carried_state, false);
//! ```
use crate::{
    fluid::{FluidBlockBindings, FluidKind, FluidState},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    geometry::{Aabb, Direction, VoxelShape},
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use std::{collections::HashMap, io, path::Path, sync::Arc, sync::RwLock};

pub mod classes;
pub mod data;
pub mod states;
pub mod ticks;

mod bonemeal;
mod buckets;
mod creaking;
mod falling;
mod feature_world;
mod fluids;
mod growth;
mod hooks;
mod interact;
pub mod pistons;
mod place;
mod portal;
pub mod portal_travel;
mod query;
pub(crate) use query::data_2d;
mod rails;
mod redstone;
pub mod removal;
pub mod sculk;
mod shapes;
mod shelf;
mod signals;
mod speleothem;
mod survive;
mod tick;
mod timeline;
mod update;
mod weather;

#[cfg(test)]
mod growth_tests;
#[cfg(test)]
mod tests;

pub use bonemeal::TemplateSettings;
pub use buckets::BucketContent;
pub use data::{BlockData, ItemBinding, ItemKind, Named, Props, PushReaction, StateFacts, Tags};
pub use falling::Landing;
pub use interact::{InteractionResult, PlayerContext, PlayerEffect, UseBranch, hit_slot};
pub use pistons::{BlockEventData, MovingPiston};
pub use place::{PlaceContext, PlaceOutcome, PlacementContext};
pub use states::{BlockId, PropKey, StateTable};
pub use tick::{Precipitation, PrecipitationKind, RandomTickMask};
pub use ticks::Ticks;

/// `Block.UPDATE_*` flags accepted by [`Behaviours::set_block`].
pub mod flags {
    pub const UPDATE_NEIGHBORS: i32 = 1;
    pub const UPDATE_CLIENTS: i32 = 2;
    pub const UPDATE_INVISIBLE: i32 = 4;
    /// Read only by a client's level, which rebuilds the section at once.
    pub const UPDATE_IMMEDIATE: i32 = 8;
    pub const UPDATE_KNOWN_SHAPE: i32 = 16;
    pub const UPDATE_SUPPRESS_DROPS: i32 = 32;
    pub const UPDATE_MOVE_BY_PISTON: i32 = 64;
    pub const UPDATE_SKIP_SHAPE_UPDATE_ON_WIRE: i32 = 128;
    pub const UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS: i32 = 256;
    pub const UPDATE_SKIP_ON_PLACE: i32 = 512;
    pub const UPDATE_NONE: i32 = 260;
    pub const UPDATE_ALL: i32 = 3;
    pub const UPDATE_ALL_IMMEDIATE: i32 = 11;
    pub const UPDATE_SKIP_ALL_SIDEEFFECTS: i32 = 816;
    /// `Block.UPDATE_LIMIT`, the recursion budget of shape updates.
    pub const UPDATE_LIMIT: i32 = 512;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}

/// Game rules consulted by block behaviour (vanilla defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameRules {
    pub water_source_conversion: bool,
    pub lava_source_conversion: bool,
    pub random_tick_speed: i32,
    pub block_drops: bool,
    pub mob_griefing: bool,
    pub max_snow_accumulation_height: i32,
    pub spawn_mobs: bool,
    pub spawn_monsters: bool,
    /// `fire_spread_radius_around_player`: -1 spreads everywhere.
    pub fire_spread_radius_around_player: i32,
    pub spread_vines: bool,
}
impl Default for GameRules {
    fn default() -> Self {
        Self {
            water_source_conversion: true,
            lava_source_conversion: false,
            random_tick_speed: 3,
            block_drops: true,
            mob_griefing: true,
            max_snow_accumulation_height: 1,
            spawn_mobs: true,
            spawn_monsters: true,
            fire_spread_radius_around_player: 128,
            spread_vines: true,
        }
    }
}

/// Entity filters for the entity-sensing blocks (pressure plates, tripwire).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityQuery {
    /// Any entity that does not ignore block triggers (wooden plates, tripwire).
    Everything,
    /// Living entities only (stone plates).
    Mobs,
    /// Minecarts only (detector rails).
    Minecart,
    /// Arrows only (`AbstractArrow`, thrown tridents included): the entities
    /// a wooden button senses.
    Arrows,
}

/// Side effects reported to the caller, in vanilla call order.
#[derive(Clone, Debug, PartialEq)]
pub enum LevelEvent {
    /// `Level.levelEvent(type, pos, data)`, using vanilla's numeric event IDs
    /// (e.g. 2001 block destroyed with `data` = state ID, 1501 lava
    /// extinguished, 1009 fire extinguished, 1502 redstone torch burnout,
    /// 2009 dripping water/sponge, 1033/1034 ... per `LevelEvent.java`).
    LevelEvent { id: i32, pos: BlockPos, data: i32 },
    /// `Level.playSound(null, pos, sound, BLOCKS, volume, pitch)`.
    Sound {
        pos: BlockPos,
        sound: Arc<str>,
        volume: f32,
        pitch: f32,
    },
    /// `Block.dropResources(state, level, pos)`: the caller evaluates the
    /// block's loot table (no tool, no breaker) and pops the items.
    DropResources { pos: BlockPos, state: BlockStateId },
    /// `FallingBlockEntity.fall`: the block at `pos` was already replaced by its
    /// fluid's legacy block; the caller spawns and simulates a falling-block
    /// entity carrying `state` and calls [`Behaviours::falling_block_landed`]
    /// (or drops the item) when it lands.
    FallingBlock {
        pos: BlockPos,
        state: BlockStateId,
        /// Tip damage configured by `SpeleothemBlock.spawnFallingStalactite`.
        hurt_per_distance: Option<f32>,
    },
    /// A sapling reached stage 1 and tried to grow (`TreeGrower.growTree`).
    /// The caller runs the grower's configured feature and, on success,
    /// replaces the sapling (vanilla leaves the sapling on failure).
    GrowTree {
        pos: BlockPos,
        state: BlockStateId,
        grower: Arc<str>,
    },
    /// A configured feature placement request from a bonemealed or random-ticked
    /// block (huge mushrooms, fungi). The block was already removed when vanilla
    /// does so before placing; on failure the caller restores `state`.
    PlaceFeature {
        pos: BlockPos,
        state: BlockStateId,
        feature: Arc<str>,
    },
    /// `TntBlock.prime`: spawn primed TNT pushed along `rot` (radians, drawn
    /// from the level random by `new PrimedTnt`). The primed sound and game
    /// event follow as their own events; the caller removes the block.
    PrimeTnt { pos: BlockPos, rot: f64 },
    /// Block-entity or entity side effects the caller implements: `"dispense"`
    /// (dispenser/dropper/crafter scheduled tick), `"brushable"` (a suspicious
    /// block's scheduled tick: `checkReset`), `"hatch"` (frogspawn
    /// hatched into tadpoles; the block is already destroyed), `"portal_spawn"`
    /// (a nether portal won its zombified piglin roll; the caller checks
    /// `anyPlayerCloseEnoughForSpawning` and spawns below the portal),
    /// `"golem"` (a carved pumpkin was placed; the caller builds the golem
    /// its blocks complete).
    BlockEvent {
        pos: BlockPos,
        state: BlockStateId,
        kind: &'static str,
    },
    /// A queued block event that reached a block entity
    /// (`BaseEntityBlock.triggerEvent`): the client's copy runs
    /// `BlockEntity.triggerEvent(a, b)` (`ClientboundBlockEventPacket`), as
    /// for a bell's ring (event 1, the struck direction).
    BlockEntityEvent {
        pos: BlockPos,
        state: BlockStateId,
        a: i32,
        b: i32,
    },
    /// A player-side effect of [`Behaviours::use_without_item`].
    Player {
        pos: BlockPos,
        state: BlockStateId,
        effect: PlayerEffect,
    },
    /// `Level.gameEvent` (vibration system), by vanilla identifier, with the
    /// affected block state when the event carries one.
    GameEvent {
        pos: BlockPos,
        event: &'static str,
        state: Option<BlockStateId>,
    },
}

/// What spreads from one block to another (`LevelAccess::allow_spread`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    /// A fluid flows into the block (`FlowingFluid.spreadTo`).
    Fluid,
    /// Fire spreads to the block or burns it (`FireBlock.tick`,
    /// `checkBurnOut`).
    Fire,
}

/// The world as seen by block behaviour. Implemented by the game and tests.
pub trait LevelAccess {
    /// Whether `kind` may spread from `from` to `to`: a native mod may refuse
    /// (land claims), and the block at `to` stays as it is. Fire asks after
    /// its random draws, which a refusal doesn't change. The default allows.
    fn allow_spread(&self, _kind: Spread, _from: BlockPos, _to: BlockPos) -> bool {
        true
    }
    /// Whether the piston at `piston` may push the blocks `push` one step
    /// `direction` and destroy `destroy` (`PistonBaseBlock.moveBlocks`, once
    /// the structure resolved). Refused, it moves nothing, as a structure
    /// that can't move. The default allows.
    fn allow_piston_move(
        &self,
        _piston: BlockPos,
        _direction: Direction,
        _push: &[BlockPos],
        _destroy: &[BlockPos],
    ) -> bool {
        true
    }
    /// Observe a committed `Level.setBlock` state change, before its side effects.
    fn observe_block_changed(&self, _pos: BlockPos, _old: BlockStateId, _new: BlockStateId) {}
    /// Observe `BlockState.handleNeighborChanged` before the block handles it.
    /// The default does nothing; implementations must not alter the update queue.
    fn observe_neighbor_changed(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _changed: BlockId,
        _moved: bool,
    ) {
    }
    /// `BlockState.randomTick` from `ServerLevel.tickChunk`, before the block
    /// handles it. The level may change the block (`Behaviours::set_block`
    /// with `ticks`); setting `handled` skips the block's own random tick. A
    /// level that changed the block without setting `handled` must put the
    /// block's state as it is now in `state`: the block's own random tick
    /// runs on it. The default does nothing.
    fn before_random_tick(
        &mut self,
        _ticks: &mut Ticks,
        _pos: BlockPos,
        _state: &mut BlockStateId,
        _handled: &mut bool,
    ) {
    }
    /// `BlockState.tick` from `ServerLevel.tickBlock` (a due scheduled tick
    /// whose block is still there), before the block handles it. The level
    /// may change the block (`Behaviours::set_block` with `ticks`); setting
    /// `handled` skips the block's own tick. A level that changed the block
    /// without setting `handled` must put the block's state as it is now in
    /// `state`: the block's own tick runs on it. The default does nothing.
    fn before_tick_block(
        &mut self,
        _ticks: &mut Ticks,
        _pos: BlockPos,
        _state: &mut BlockStateId,
        _handled: &mut bool,
    ) {
    }
    /// `BonemealableBlock.isValidBonemealTarget` for the block `state` at
    /// `pos`, a block the port knows no `BonemealableBlock` for: a native mod
    /// answers it for its own blocks, and bone meal is then used up on one as
    /// vanilla uses it up (the caller fires the growth particles and consumes
    /// the meal). Asked on both sides, as vanilla asks it on both (a client
    /// predicts the use with it), so a mod that can only grow on a server says
    /// so with `is_client_side`. `Some(false)` refuses it, even a block whose
    /// template takes bone meal; `None` (the default) leaves the block its own
    /// `BonemealableBlock`, or none.
    fn is_valid_bonemeal_target(&self, _pos: BlockPos, _state: BlockStateId) -> Option<bool> {
        None
    }
    /// `BonemealableBlock.isBonemealSuccess` for the block `state` at `pos`,
    /// after `is_valid_bonemeal_target` said yes: whether it grows this time.
    /// Vanilla draws the level's random here, so a mod's draws through
    /// [`LevelAccess::random`] stay in vanilla's order. The default says no.
    fn is_bonemeal_success(&mut self, _pos: BlockPos, _state: BlockStateId) -> bool {
        false
    }
    /// `BonemealableBlock.performBonemeal` for the block `state` at `pos`:
    /// the growth itself, after `is_bonemeal_success` said yes. The meal is
    /// used up either way, as vanilla uses it up. `ticks` schedules the ticks
    /// the growth schedules, as `before_update_shape` hands them. The default
    /// does nothing.
    fn perform_bonemeal(&mut self, _ticks: &mut Ticks, _pos: BlockPos, _state: BlockStateId) {}
    /// `BlockState.updateShape` for `state` at `pos` after its neighbour in
    /// `direction` became `neighbour`, before the block computes it. `state`
    /// may not be in the level yet (a piston landing it). A state put in
    /// `new` is the result, and the block's own `updateShape` doesn't run;
    /// the level may schedule ticks through `ticks`, as that would. The
    /// default does nothing.
    #[allow(clippy::too_many_arguments)]
    fn before_update_shape(
        &self,
        _ticks: &mut Ticks,
        _pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
        _neighbour_pos: BlockPos,
        _neighbour: BlockStateId,
        _new: &mut Option<BlockStateId>,
    ) {
    }
    /// `Block.getStateForPlacement(BlockPlaceContext)` for the block whose
    /// default state is `state`, before the block computes the state it is
    /// placed in: a native mod chooses it for its own blocks (a block that
    /// faces the player, sits on the clicked face, or comes waterlogged).
    /// `context` is vanilla's `BlockPlaceContext`, `fluid` the fluid in the
    /// position the block would go in. `answer` starts empty: `Some(Some(s))`
    /// places `s`, a state of the block's own, `Some(None)` refuses the
    /// placement (vanilla's `null` return: the block goes nowhere and nothing
    /// is consumed), and `None` leaves the block's own `getStateForPlacement`
    /// to run. The default does nothing.
    #[allow(clippy::option_option)]
    fn before_state_for_placement(
        &self,
        _state: BlockStateId,
        _context: &PlacementContext,
        _fluid: FluidState,
        _answer: &mut Option<Option<BlockStateId>>,
    ) {
    }
    /// `BlockState.canSurvive(level, pos)` for `state` at `pos`, before the
    /// block answers it: placing it (`BlockItem.canPlace`), a plant's or an
    /// attached block's shape update, and every other rule that asks. `state`
    /// may not be in the level yet (it is being placed). An answer put in
    /// `survives` is the result, and the block's own rule doesn't run. The
    /// default does nothing.
    fn before_can_survive(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _survives: &mut Option<bool>,
    ) {
    }
    /// `VegetationBlock.mayPlaceOn(ground, level, groundPos)` for the plant
    /// `state`, before the plant answers it: the blocks a plant can stand on,
    /// which its `canSurvive` asks. An answer put in `may` is the result. The
    /// default does nothing.
    fn before_may_place_on(
        &self,
        _state: BlockStateId,
        _ground: BlockStateId,
        _ground_pos: BlockPos,
        _may: &mut Option<bool>,
    ) {
    }
    /// `Level.isClientSide`: this level is a client's copy (false on a
    /// server, a host or the test harness).
    fn is_client_side(&self) -> bool {
        false
    }
    /// `FireBlock`'s flammability table for `state`'s block, before the
    /// table answers it: `(igniteOdds, burnOdds)`, as `FireBlock.setFlammable`
    /// sets them. Fire spread, burning out and where fire can stand read it;
    /// a waterlogged state never burns and isn't asked. An answer put in
    /// `odds` is the result. The default does nothing.
    fn before_fire_odds(&self, _state: BlockStateId, _odds: &mut Option<(i32, i32)>) {}
    /// `CampfireBlock.isSmokeSource(state)` for the block under a campfire,
    /// before vanilla's rule (a hay bale) answers it: true makes the campfire
    /// a signal fire. An answer put in `smoke` is the result. The default
    /// does nothing.
    fn before_smoke_source(&self, _state: BlockStateId, _smoke: &mut Option<bool>) {}
    /// `getBlockState`. Unloaded or out-of-bounds positions read as air.
    fn block_state(&self, pos: BlockPos) -> BlockStateId;
    /// Storage write (`LevelChunkSection.setBlockState`). Returns the previous
    /// state, or None when nothing could be stored (unloaded chunk).
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId>;
    fn is_loaded(&self, pos: BlockPos) -> bool;
    /// Inclusive minimum and exclusive maximum build height.
    fn min_y(&self) -> i32 {
        -64
    }
    fn max_y(&self) -> i32 {
        320
    }
    /// `getHeight(WORLD_SURFACE, x, z)`: one above the column's highest
    /// block that isn't air. Only air lies above it, so scans for the other
    /// heightmaps start there. The default, `max_y`, scans the whole column.
    fn world_surface(&self, _x: i32, _z: i32) -> i32 {
        self.max_y()
    }
    /// Vanilla `level.random` (a `LegacyRandomSource`).
    fn random(&mut self) -> &mut LegacyRandom;
    fn game_time(&self) -> i64;
    fn block_light(&self, _pos: BlockPos) -> i32 {
        0
    }
    fn sky_light(&self, _pos: BlockPos) -> i32 {
        15
    }
    /// `Level.getSkyDarken` (0 at noon, up to 11 at night).
    fn sky_darken(&self) -> i32 {
        0
    }
    /// `EnvironmentAttributes.SUN_ANGLE` at `pos`, in degrees (0 at noon).
    fn sun_angle(&self, _pos: BlockPos) -> f32 {
        0.0
    }
    fn is_raining(&self) -> bool {
        false
    }
    fn is_thundering(&self) -> bool {
        false
    }
    fn is_raining_at(&self, _pos: BlockPos) -> bool {
        false
    }
    fn dimension(&self) -> Dimension {
        Dimension::Overworld
    }
    fn game_rules(&self) -> GameRules {
        GameRules::default()
    }
    /// `Difficulty.getId()` (peaceful 0 .. hard 3).
    fn difficulty(&self) -> i32 {
        2
    }
    /// `ServerLevel.canSpreadFireAround` (fire-spread radius around players).
    fn can_spread_fire_around(&self, _pos: BlockPos) -> bool {
        true
    }
    /// The `INCREASED_FIRE_BURNOUT` environment attribute at `pos` (a biome
    /// attribute).
    fn increased_fire_burnout(&self, _pos: BlockPos) -> bool {
        false
    }
    /// `Level.isUnobstructed(state, pos, context)`: whether the state's
    /// collision shape at `pos` would intersect an entity.
    fn is_unobstructed(&self, _pos: BlockPos, _state: BlockStateId) -> bool {
        true
    }
    /// Comparator input from a block entity (`getAnalogOutputSignal` of
    /// containers, jukeboxes, lecterns, ...). State-only sources (cake,
    /// composter, cauldrons, beehives, ...) are evaluated by the library.
    fn block_entity_analog_signal(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        0
    }
    /// `ItemFrame.getAnalogOutput` of the one item frame in the block at
    /// `pos` that faces `facing` (`ComparatorBlock.getItemFrame`), or None when
    /// there is not exactly one.
    fn item_frame_analog_output(&self, _pos: BlockPos, _facing: Direction) -> Option<i32> {
        None
    }
    /// `ContainerOpenersCounter.recheckOpenCount` for the chest or ender chest
    /// block entity at `pos`: recounts its openers and returns the previous
    /// and new counts (the caller plays open/close effects when they change),
    /// or None when there is no such block entity.
    fn recheck_container_openers(&mut self, _pos: BlockPos) -> Option<(i32, i32)> {
        None
    }
    /// `CommandBlockEntity.setPowered` for the command block entity at `pos`:
    /// stores `powered` and returns the previous flag and `isAutomatic`, or
    /// None when there is no command block entity.
    fn set_command_block_powered(
        &mut self,
        _pos: BlockPos,
        _powered: bool,
    ) -> Option<(bool, bool)> {
        None
    }
    /// `PotentSulfurBlockEntity.resetCountdown` for the potent sulfur block
    /// entity at `pos` (`PotentSulfurBlock.validBlockState`).
    fn reset_potent_sulfur_countdown(&mut self, _pos: BlockPos) {}
    /// `BlockState.isSignalSource` once the library answered `vanilla`: the
    /// answer to use. The default keeps it.
    fn adjust_signal_source(&self, _state: BlockStateId, vanilla: bool) -> bool {
        vanilla
    }
    /// `BlockState.hasAnalogOutputSignal` once the library answered `vanilla`:
    /// whether comparators read the block. The default keeps it.
    fn adjust_has_analog_output(&self, _state: BlockStateId, vanilla: bool) -> bool {
        vanilla
    }
    /// `BlockState.getAnalogOutputSignal(level, pos, direction)` once the
    /// library answered `vanilla`, `direction` pointing from the block to the
    /// comparator reading it: the output to use. The default keeps it.
    fn adjust_analog_output(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
        vanilla: i32,
    ) -> i32 {
        vanilla
    }
    /// `BlockState.getSignal` (or `getDirectSignal` when `direct`) once the
    /// library answered `vanilla`: the signal to use. The default keeps it.
    fn adjust_signal(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
        _direct: bool,
        vanilla: i32,
    ) -> i32 {
        vanilla
    }
    /// Weak/strong signal of a block-entity-driven source (trapped chest
    /// openers); other sources are evaluated by the library.
    fn block_entity_signal(
        &self,
        _pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        0
    }
    /// Biome answers for a precipitation tick at the column of `pos` (see
    /// [`Precipitation`]); None skips snow/ice formation and cauldron filling.
    fn precipitation(&self, _pos: BlockPos) -> Option<Precipitation> {
        None
    }
    /// Count of entities intersecting `area` that trigger pressure plates or
    /// tripwires under `query`.
    fn count_entities(&self, _area: Aabb, _query: EntityQuery) -> usize {
        0
    }
    /// `ServerLevel.isPositionTickingWithEntitiesLoaded` for scheduled ticks.
    fn is_ticking_chunk(&self, _chunk: ChunkPos) -> bool {
        true
    }
    fn emit(&mut self, _event: LevelEvent) {}
    /// `LevelChunk.setBlockState` replaced a block that had a block entity by
    /// another block: run `BlockEntity.preRemoveSideEffects` when
    /// `side_effects` (containers spill their contents, a jukebox pops its
    /// record, a lectern its book...) and remove the block entity. Called
    /// after the storage write, before `affectNeighborsAfterRemoval`. Returns
    /// how many times the side effects called `updateNeighborsAt(pos, block)`
    /// (a jukebox whose song changed), which the caller then runs.
    fn remove_block_entity(
        &mut self,
        _pos: BlockPos,
        _old: BlockStateId,
        _side_effects: bool,
    ) -> u8 {
        0
    }
    /// Whether a block entity a mod made lives at `pos` (`native_block_entity`),
    /// whatever its block's template: it is removed with its block and no piston
    /// moves it, as for a template that has one.
    fn has_mod_block_entity(&self, _pos: BlockPos) -> bool {
        false
    }
    /// Load saved block-entity data into the block entity of the block just
    /// placed at `pos` (`BlockEntity.loadWithComponents`): a bee nest a grown
    /// tree placed gets its bees. Levels without block entities drop it.
    fn load_block_entity(&mut self, _pos: BlockPos, _nbt: crate::nbt::Compound) {}
    /// Biome identifier at `pos` (`minecraft:plains`), for features placed by
    /// block behaviour (bone meal flowers). None when unknown.
    fn biome_name(&self, _pos: BlockPos) -> Option<&str> {
        None
    }
    /// World seed (`WorldGenLevel.getSeed`) for seed-derived feature noise.
    fn world_seed(&self) -> i64 {
        0
    }
    /// `Level.getDayTime` (the overworld clock driving the day timeline's
    /// environment attributes, e.g. eyeblossoms opening at night).
    fn day_time(&self) -> i64 {
        6000
    }
}

/// Raw brightness as `Level.getRawBrightness(pos, skyDarken)`.
pub fn raw_brightness<L: LevelAccess + ?Sized>(level: &L, pos: BlockPos, sky_darken: i32) -> i32 {
    (level.sky_light(pos) - sky_darken).max(level.block_light(pos))
}

/// Access only the gameplay fields needed by level behaviour tables.
/// Both the JSON fallback and the client's typed decoder implement this.
pub trait GameplayStateSource {
    fn flag(&self, name: &str) -> bool;
    fn unsigned(&self, name: &str) -> u64;
    fn number(&self, name: &str) -> f64;
    fn text(&self, name: &str) -> &str;
    fn sound_number(&self, name: &str) -> f64;
    fn sound_id(&self, name: &str) -> &str;
}

/// Immutable vanilla block-behaviour tables. Build once with [`Behaviours::load`].
pub struct Behaviours {
    pub table: StateTable,
    pub props: Props,
    pub named: Named,
    pub tags: Tags,
    all_tags: HashMap<String, Arc<data::BlockSet>>,
    facts: Vec<StateFacts>,
    shapes: Vec<VoxelShape>,
    shape_info: Vec<data::ShapeInfo>,
    pub fluid_tags: data::FluidTags,
    blocks: Vec<BlockData>,
    items: HashMap<String, ItemBinding>,
    potted_by_content: HashMap<BlockId, BlockId>,
    liquid_blocks: FluidBlockBindings,
    /// Uncached (position- or block-entity-dependent) collision shapes.
    dynamic: HashMap<BlockStateId, crate::dynamic_support::DynamicSupport>,
    occlusion_cache: RwLock<HashMap<(i32, i32, u8), bool>>,
    growth: growth::Growth,
}

impl Behaviours {
    /// Load from a prepared client resource directory (`.local/horizon-client/<version>`).
    pub fn load(registry: &BlockRegistry, resources_dir: &Path) -> io::Result<Self> {
        data::load(registry, resources_dir)
    }
    /// Reuse the client's typed gameplay projection while it is borrowed from the JSON bytes.
    pub fn load_with_gameplay<G: GameplayStateSource>(
        registry: &BlockRegistry,
        resources_dir: &Path,
        behaviour: &serde_json::Value,
        gameplay: &[G],
    ) -> io::Result<Self> {
        data::load_with_gameplay(registry, resources_dir, behaviour, gameplay)
    }

    pub fn load_with_behaviour_json(
        registry: &BlockRegistry,
        resources_dir: &Path,
        behaviour: &serde_json::Value,
    ) -> io::Result<Self> {
        data::load_with_behaviour_json(registry, resources_dir, behaviour)
    }

    /// Keep a snapshot of the resolved tables in `dir` (a game folder's
    /// `.horizon-cache`), so later launches of the same build restore it instead
    /// of resolving the resources again. Without it, [`Self::load`] never caches.
    pub fn set_cache_dir(dir: std::path::PathBuf) {
        data::set_cache_dir(dir);
    }
    pub fn facts(&self, state: BlockStateId) -> &StateFacts {
        &self.facts[state.0 as usize]
    }
    pub fn block_data(&self, block: BlockId) -> &BlockData {
        &self.blocks[block.0 as usize]
    }
    pub fn data(&self, state: BlockStateId) -> &BlockData {
        self.block_data(self.table.block(state))
    }
    pub fn block(&self, state: BlockStateId) -> BlockId {
        self.table.block(state)
    }
    pub fn item_binding(&self, item: &str) -> Option<&ItemBinding> {
        self.items.get(item)
    }
    /// FlowerPotBlock.POTTED_BY_CONTENT: the potted flower pot block that
    /// holds plant, if any.
    pub fn potted_block(&self, plant: BlockId) -> Option<BlockId> {
        self.potted_by_content.get(&plant).copied()
    }
    pub fn tag(&self, name: &str) -> Option<&data::BlockSet> {
        self.all_tags.get(name).map(|s| &**s)
    }
    pub fn fluid(&self, state: BlockStateId) -> FluidState {
        self.facts(state).fluid
    }
    /// `FluidState.createLegacyBlock`.
    pub fn legacy_block(&self, fluid: FluidState) -> BlockStateId {
        self.liquid_blocks.block(fluid)
    }
    /// Cached collision shape (`getCollisionShape(level, pos)` for states whose
    /// shape does not depend on the world); None for dynamic shapes.
    pub fn collision_shape(&self, state: BlockStateId) -> Option<&VoxelShape> {
        let index = self.facts(state).shape;
        (index >= 0).then(|| &self.shapes[index as usize])
    }
    /// Precomputed shape queries; dynamic shapes read as empty.
    pub fn shape_info(&self, state: BlockStateId) -> data::ShapeInfo {
        let index = self.facts(state).shape;
        if index < 0 {
            data::ShapeInfo::default()
        } else {
            self.shape_info[index as usize]
        }
    }
    pub(crate) fn has_flag(&self, state: BlockStateId, flag: u32) -> bool {
        self.facts(state).flags & flag != 0
    }
    pub fn is_air(&self, state: BlockStateId) -> bool {
        self.has_flag(state, data::state_flags::AIR)
    }
    pub fn is_solid(&self, state: BlockStateId) -> bool {
        self.has_flag(state, data::state_flags::SOLID)
    }
    pub fn is_replaceable(&self, state: BlockStateId) -> bool {
        self.has_flag(state, data::state_flags::REPLACEABLE)
    }
    pub fn is_randomly_ticking(&self, state: BlockStateId) -> bool {
        self.has_flag(state, data::state_flags::RANDOM_TICKS)
    }
    /// `isFaceSturdy(level, pos, direction, FULL)` from the empty-level cache.
    pub fn is_face_sturdy(&self, state: BlockStateId, direction: Direction) -> bool {
        self.facts(state).sturdy & (1 << direction as u32) != 0
    }
    pub fn is_face_sturdy_center(&self, state: BlockStateId, direction: Direction) -> bool {
        self.facts(state).sturdy & (1 << (6 + direction as u32)) != 0
    }
    pub fn is_face_sturdy_rigid(&self, state: BlockStateId, direction: Direction) -> bool {
        self.facts(state).sturdy & (1 << (12 + direction as u32)) != 0
    }
    /// Tick type ID of a fluid for [`Ticks::schedule_fluid`].
    pub fn fluid_kind(&self, state: BlockStateId) -> FluidKind {
        self.fluid(state).kind()
    }
}

/// Level mutation entry points (all keep vanilla update semantics).
impl Behaviours {
    /// `Level.setBlock(pos, state, flags)` with vanilla's update limit (512).
    /// Returns false when nothing was written (unloaded, out of bounds, or
    /// the state was already present).
    pub fn set_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        self.sim(level, ticks).set_block(pos, state, flags)
    }
    /// `CopperChestBlock.getFromCopperBlock(copperBlock, facing, level, pos)`.
    pub fn copper_chest_from_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        copper: BlockStateId,
        facing: Direction,
        pos: BlockPos,
    ) -> Option<BlockStateId> {
        self.sim(level, ticks)
            .copper_chest_from_block(copper, facing, pos)
    }
    /// `Level.removeBlock(pos, movedByPiston)`: replace with the fluid's block.
    pub fn remove_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        moved_by_piston: bool,
    ) -> bool {
        self.sim(level, ticks).remove_block(pos, moved_by_piston)
    }
    /// `Level.destroyBlock(pos, dropResources)`: destroy particles (2001),
    /// optional drops, and replacement by the block's fluid.
    pub fn destroy_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        drop: bool,
    ) -> bool {
        self.sim(level, ticks)
            .destroy_block(pos, drop, flags::UPDATE_LIMIT)
    }
    /// A player breaking a block (`playerWillDestroy` + `Level.destroyBlock`
    /// without drops; the caller handles the player's loot/tool). Double
    /// blocks (doors, tall plants, beds) remove their other half as vanilla.
    pub fn player_destroy_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        creative: bool,
        correct_tool: bool,
    ) -> bool {
        self.sim(level, ticks)
            .player_destroy(pos, creative, correct_tool)
    }
    /// `IceBlock.playerDestroy` after the drops: mined ice melts into water
    /// unless the tool prevents ice melting (silk touch). `destroyed` is the
    /// state that was broken.
    pub fn ice_destroyed_by_player<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        destroyed: BlockStateId,
        prevents_melting: bool,
    ) {
        self.sim(level, ticks)
            .ice_destroyed_by_player(pos, destroyed, prevents_melting);
    }
    /// `TurtleEggBlock.playerDestroy` after the drops: a mined stack of eggs
    /// loses one egg and the rest stay. `destroyed` is the state that was broken.
    pub fn turtle_egg_destroyed_by_player<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        destroyed: BlockStateId,
    ) {
        self.sim(level, ticks)
            .turtle_egg_destroyed_by_player(pos, destroyed);
    }
    /// `BaseFireBlock.getState(level, pos)`: soul fire on its base blocks,
    /// otherwise fire shaped by its neighbours.
    pub fn fire_state_for<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> BlockStateId {
        self.sim(level, ticks).fire_state_for(pos)
    }
    /// `BaseFireBlock.canBePlacedAt(level, pos, forwardDirection)`.
    pub fn fire_can_be_placed_at<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        forward: horizon_core::geometry::Direction,
    ) -> bool {
        self.sim(level, ticks).fire_can_be_placed_at(pos, forward)
    }
    /// `BlockState.canSurvive(level, pos)`.
    pub fn can_survive<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
    ) -> bool {
        self.sim(level, ticks).can_survive(state, pos)
    }
    /// `BlockState.updateShape(level, ticks, pos, direction, neighbourPos,
    /// neighbourState, random)`: the state `state` at `pos` would take after its
    /// neighbour in `direction` became `neighbour` (ticks it requests are queued).
    #[allow(clippy::too_many_arguments)]
    pub fn update_shape<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
    ) -> BlockStateId {
        self.sim(level, ticks)
            .update_shape(state, pos, direction, neighbour_pos, neighbour)
    }
    /// `Block.updateFromNeighbourShapes`: a state adjusted to its neighbours.
    pub fn update_from_neighbour_shapes<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
    ) -> BlockStateId {
        self.sim(level, ticks)
            .update_from_neighbour_shapes(state, pos)
    }
    /// `Level.updateNeighborsAt(pos, block)`: notify the six neighbours.
    pub fn update_neighbors_at<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        block: BlockId,
    ) {
        self.sim(level, ticks).update_neighbors_at(pos, block);
    }
    /// `Level.updateNeighbourForOutputSignal(pos, block)` for the block at
    /// `pos`: comparators beside it (or behind a conductor there) read their
    /// input again.
    pub fn update_neighbour_for_output_signal<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        let mut sim = self.sim(level, ticks);
        let block = self.block(sim.state(pos));
        sim.update_neighbour_for_output_signal(pos, block);
    }
    /// `Level.scheduleTick(pos, block, delay)` for the block at `pos`.
    pub fn schedule_tick<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        delay: i32,
    ) {
        let mut sim = self.sim(level, ticks);
        let block = self.block(sim.state(pos));
        sim.schedule_tick(pos, block, delay);
    }
    /// `BlockState.handleNeighborChanged(level, pos, changedBlock, orientation, movedByPiston)`
    /// for the state currently at `pos`.
    pub fn handle_neighbor_changed<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        changed: BlockId,
    ) {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        sim.neighbor_changed(state, pos, changed, false);
    }
    /// Entity collision with a block (`entityInside`) for the entity-sensing
    /// blocks: pressure plates, tripwires and buttons hit by arrows are
    /// re-evaluated through [`LevelAccess::count_entities`].
    pub fn entity_inside<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        self.sim(level, ticks).entity_inside(pos);
    }
    /// `LilyPadBlock.entityInside` for a boat whose box is inside `pos`.
    pub fn boat_inside<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        self.sim(level, ticks).boat_inside(pos);
    }
    /// `DaylightDetectorBlock.tickEntity` of the daylight detector at `pos`:
    /// its signal follows the sky every 20 game ticks. The detector's ticker
    /// exists only where the dimension has sky light.
    pub fn tick_daylight_detector<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        if level.dimension() == Dimension::Nether || level.game_time() % 20 != 0 {
            return;
        }
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        if sim.b.data(state).is(classes::Class::DaylightDetector) {
            sim.daylight_detector_update_signal(pos, state);
        }
    }
    /// `BigDripleafBlock.entityInside` for an entity whose box is inside
    /// `pos`: whether it is on the ground and its feet's y (`canEntityTilt`).
    pub fn entity_on_big_dripleaf<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        on_ground: bool,
        feet_y: f64,
    ) {
        self.sim(level, ticks)
            .big_dripleaf_entity_inside(pos, on_ground, feet_y);
    }
    /// `BellBlock.onProjectileHit` for a projectile that hit `face` of the
    /// block at `pos` at height `hit_y`. False when the block is not a bell
    /// or the hit missed the bell proper.
    pub fn bell_projectile_hit<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        face: Direction,
        hit_y: f64,
    ) -> bool {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        self.data(state).is(classes::Class::Bell)
            && sim.bell_hit(pos, state, face, hit_y - f64::from(pos.y))
    }
    /// `BellBlock.attemptToRing(entity, level, pos, null)`: a mob rings the
    /// bell from its facing. False when the block at `pos` is not a bell.
    pub fn bell_attempt_to_ring<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> bool {
        let mut sim = self.sim(level, ticks);
        let state = sim.state(pos);
        if !self.data(state).is(classes::Class::Bell) {
            return false;
        }
        let facing = sim.b.facing(state);
        sim.bell_ring(pos, state, facing);
        true
    }
    /// Redstone signal queries (`SignalGetter`).
    pub fn has_neighbor_signal<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> bool {
        self.sim(level, ticks).has_neighbor_signal(pos)
    }
    pub fn best_neighbor_signal<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> i32 {
        self.sim(level, ticks).best_neighbor_signal(pos)
    }
    /// `BlockState.onProjectileHit` of a target block (`TargetBlock`) hit by
    /// a projectile at `location` on the face with normal `face`; `arrow`
    /// for an `AbstractArrow`. Returns the output strength, or None when the
    /// block at `pos` is not a target block.
    pub fn target_projectile_hit<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        location: [f64; 3],
        face: [i32; 3],
        arrow: bool,
    ) -> Option<i32> {
        self.sim(level, ticks)
            .target_projectile_hit(pos, location, face, arrow)
    }
}

/// One mutable view used by the ported behaviour code.
pub(crate) struct Sim<'a, L: LevelAccess + ?Sized> {
    pub b: &'a Behaviours,
    pub level: &'a mut L,
    pub ticks: &'a mut Ticks,
    /// `RedstoneWireBlock.shouldSignal` is false while a wire samples its
    /// neighbours (`getBlockSignal`).
    pub wire_signal_off: std::cell::Cell<bool>,
}
impl Behaviours {
    pub(crate) fn sim<'a, L: LevelAccess + ?Sized>(
        &'a self,
        level: &'a mut L,
        ticks: &'a mut Ticks,
    ) -> Sim<'a, L> {
        Sim {
            b: self,
            level,
            ticks,
            wire_signal_off: std::cell::Cell::new(false),
        }
    }
}
