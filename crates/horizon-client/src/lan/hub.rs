//! The play state of a hosted world (PlayerList, ServerGamePacketListenerImpl,
//! ChunkMap, PlayerChunkSender): admits the players the connection threads
//! hand over, sends them the level (chunks, time, weather, entities, player
//! list), keeps their views up to date as blocks and entities change, and
//! turns what they send into events for the gameplay code. It runs on the
//! game thread inside the tick, only when the world is open to the network.
use super::{
    Lan, NetEvent,
    chunks::ChunkEncoder,
    data::SyncData,
    entity::{Entities, EntityState, Viewer},
    guest::{Guest, SentEffect, stack_to_network},
};
use crate::{
    chunk::Chunk,
    collisions::SceneCollisions,
    effects::{EffectInstance, PlayerEffects},
    items::ItemStack,
    world::World,
};
use horizon_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    random::LegacyRandom,
};
use horizon_protocol::ids::play::clientbound as ids;
use horizon_protocol::{
    entity_data::{Particle, ParticleData, Value, serializer},
    packets::play::{self, Clientbound as C, Serverbound as S},
    text::{literal, translatable},
};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    block_collision_scan::BlockCollisionScan,
    collision_context::CollisionContext,
    nbt::{JavaString, Tag},
    world_movement::in_range_of_ground,
};
use std::sync::{Arc, Mutex, atomic::Ordering};

/// GameEvent types (ClientboundGameEventPacket).
pub mod chat;
pub mod chat_filter;

pub mod game_event {
    pub const NO_RESPAWN_BLOCK_AVAILABLE: u8 = 0;
    pub const START_RAINING: u8 = 1;
    pub const STOP_RAINING: u8 = 2;
    pub const CHANGE_GAME_MODE: u8 = 3;
    pub const RAIN_LEVEL: u8 = 7;
    pub const THUNDER_LEVEL: u8 = 8;
    pub const GUARDIAN_ELDER_EFFECT: u8 = 10;
    pub const IMMEDIATE_RESPAWN: u8 = 11;
    pub const LEVEL_CHUNKS_LOAD_START: u8 = 13;
}

#[derive(Clone, Debug)]
pub struct HubConfig {
    pub default_game_mode: i32,
    pub guest_commands: bool,
    pub force_game_mode: bool,
    pub hardcore: bool,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub dimension: String,
    /// `MinecraftServer.allowFlight`: true except on a dedicated server whose
    /// `allow-flight` is off.
    pub allow_flight: bool,
    pub seed: i64,
    pub max_players: usize,
    /// A dedicated server: operators from `ops`, the dedicated-only commands.
    pub dedicated: bool,
    /// An integrated server (`dedicated` too, since it has no host player):
    /// the owner's level from `ops`, the others' from Allow Commands and
    /// the guests' command access, and the integrated-only commands.
    pub integrated: bool,
    /// The shapes of the world's levels (their dimension types).
    pub levels: crate::chunk::LevelShapes,
}

/// The host player as the other players see them.
#[derive(Clone, Debug)]
pub struct HostState {
    pub uuid: u128,
    pub name: String,
    pub entity_id: i32,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub on_ground: bool,
    pub game_mode: i32,
    pub sneaking: bool,
    pub sprinting: bool,
    pub equipment: [Option<ItemStack>; 6],
    pub health: f32,
    /// The host player's client-syncable attribute instances.
    pub attributes: Vec<play::AttributeSnapshot>,
    /// `LivingEntity.DATA_LIVING_ENTITY_FLAGS` (the item in use).
    pub living_flags: i8,
    /// `LivingEntity.DATA_ARROW_COUNT_ID` and `DATA_STINGER_COUNT_ID`.
    pub stuck: (i32, i32),
    /// `Player.DATA_SHOULDER_PARROT_LEFT` and `_RIGHT`: the parrots' variants.
    pub shoulders: [Option<i32>; 2],
    /// The bed the host sleeps in (`LivingEntity.SLEEPING_POS`).
    pub bed: Option<[i32; 3]>,
    pub body: BodyFlags,
}

/// What the other players see of a player's body besides sneaking and
/// sprinting: the pose and the rest of `Entity.DATA_SHARED_FLAGS_ID`.
#[derive(Clone, Copy, Debug, Default)]
pub struct BodyFlags {
    /// `Entity.isFallFlying`: gliding with elytra.
    pub gliding: bool,
    /// `Entity.isSwimming` (the swimming flag).
    pub swimming: bool,
    /// The swimming pose of a player crawling under a block.
    pub crawling: bool,
    /// The crouching pose, held without sneaking under a low ceiling.
    pub crouching: bool,
    /// `LivingEntity.isAutoSpinAttack`: Riptide's spin.
    pub spin_attack: bool,
    /// `Entity.isOnFire`.
    pub on_fire: bool,
    /// `Entity.isInvisible`: the invisibility effect.
    pub invisible: bool,
    /// The glowing effect.
    pub glowing: bool,
}

/// Everything the hub needs from the game for one tick.
pub struct TickCtx<'a> {
    pub world: &'a mut World,
    /// The player playing on the host (none on a dedicated server).
    pub host: Option<&'a HostState>,
    pub entities: &'a [EntityState],
    pub next_entity: &'a mut u32,
    /// Where new players appear.
    pub spawn: [f64; 3],
    pub difficulty: i32,
    pub difficulty_locked: bool,
    /// The overworld clock's speed (0 when the day does not advance).
    pub clock_rate: f32,
    /// TickRateManager: the tick rate and whether the game is frozen.
    pub tick_rate: f32,
    pub tick_frozen: bool,
    /// The last step command's broadcast and the remaining ticks for joins.
    pub tick_steps: crate::clock::StepState,
    /// Block collisions, for ServerGamePacketListenerImpl's moved-wrongly
    /// check (none: moves are not checked against blocks).
    pub collision: Option<&'a BlockCollisionRegistry<'a>>,
    /// The level's world border (PlayerList.sendLevelInfo sends it).
    pub border: &'a crate::worldborder::WorldBorder,
    /// The level's columns still being generated: a column is sent once
    /// each neighbour is loaded or a decorated proto chunk (#4294). None:
    /// every column there is is loaded.
    pub streaming: Option<&'a crate::streaming::Streaming>,
}

/// What the gameplay code has to do for a guest.
pub enum HostEvent {
    /// A guest entered the world; load their saved state.
    Joined(u32),
    /// A guest left (state to save, entities to drop).
    Left(Box<Guest>),
    Packet(u32, Box<S>),
    /// A guest's client started (`flying`) or stopped flying (their abilities packet), applied to
    /// the guest already: the game tells the mods, which may refuse it.
    Flight {
        conn: u32,
        flying: bool,
    },
    /// A chat line to show on the host (already broadcast to the guests);
    /// a dedicated server logs it, `[Not Secure]` unless signed and fresh.
    Chat {
        line: String,
        secure: bool,
    },
    /// A chat message a player sent (after it was broadcast), for server mods.
    PlayerChat {
        sender: u128,
        text: String,
    },
    /// A player's text the chat filter refused (docs/chat-filter.md): relayed to nobody, for the
    /// game to log and to carry out `action`.
    ChatFiltered {
        sender: u128,
        name: String,
        place: chat_filter::Place,
        text: String,
        /// The hit's number in the filter's window.
        count: u32,
        action: horizon_chat_filter::Action,
    },
    /// A server log warning (ServerGamePacketListenerImpl's LOGGER.warn).
    Warn(String),
    /// A server log error (LOGGER.error).
    Error(String),
    /// The anticheat bans this player (`banned-players.json`, then the ban's kick).
    Ban {
        uuid: u128,
        name: String,
        reason: String,
    },
}

/// What one player started and stopped tracking, in order, for the game to
/// tell the mods.
pub struct TrackingBatch {
    pub player: u128,
    pub name: String,
    pub events: Vec<super::entity::TrackEvent>,
}

/// Moves a guest's tracking events into `out`, one batch.
fn take_tracking(g: &mut Guest, out: &mut Vec<TrackingBatch>) {
    let events = g.tracker.take_events();
    if !events.is_empty() {
        out.push(TrackingBatch {
            player: g.uuid,
            name: g.name.clone(),
            events,
        });
    }
}

pub struct Hub {
    /// Clicks on and closings of native mods' menus, for the game.
    pub native_menu_events: Vec<super::menu::NativeMenuEvent>,
    /// The entities players started and stopped tracking, collected by
    /// `collect_tracking`, for the game to tell the mods.
    pub native_tracking: Vec<TrackingBatch>,
    pub lan: Lan,
    pub config: HubConfig,
    pub encoder: ChunkEncoder,
    pub guests: Vec<Guest>,
    pub(super) sync: Arc<SyncData>,
    pub ticks: u64,
    /// The next `Guest.vehicle_order`: a rider that boards later sits behind.
    pub seat_order: u64,
    /// `Level.soundSeedGenerator`: a fresh `nextLong` per `playSound`.
    pub(super) sound_seeds: Mutex<LegacyRandom>,
    /// The `player_movement_check` and `elytra_movement_check` game rules,
    /// set by the game before each tick.
    pub movement_checks: (bool, bool),
    /// The anticheat, when `server.toml` turns it on (dedicated servers).
    pub anticheat: Option<Box<super::anticheat::Anticheat>>,
    /// The chat filter, when `server.toml` turns it on (dedicated servers).
    pub chat_filter: Option<Box<horizon_chat_filter::Guard>>,
    /// The far ring, when `server.toml`'s `max_view_distance` turns it on (dedicated servers).
    pub far: Option<super::far::Far>,
    /// The `immediate_respawn` game rule, as the players were last told.
    immediate_respawn: bool,
    /// An integrated server's owner (`MinecraftServer.isSingleplayerOwner`),
    /// whose moves are never checked.
    pub singleplayer_owner: Option<u128>,
    /// The world is superflat (`ServerLevel.isFlat`, a world-wide flag, sent as
    /// `CommonPlayerSpawnInfo.isFlat` in every dimension).
    pub flat_world: bool,
    /// The world is the Debug preset (`CommonPlayerSpawnInfo.isDebug`).
    pub debug_world: bool,
    /// Who rides what, set by the game before each tick.
    pub riding: super::entity::Riding,
    /// The scoreboard's teams, set by the game each tick, for players'
    /// display names.
    pub teams: Vec<crate::scoreboard::Team>,
    /// Names a server mod shows for players in chat and the Tab list
    /// (`native_set_nickname`), by UUID. Their real name stays everywhere
    /// else: the log, commands, the hover of the shown name.
    pub nicknames: std::collections::HashMap<u128, String>,
    last_clock: i64,
    last_rate: f32,
    ticking: (f32, bool),
    /// The `tick_steps` count last broadcast.
    stepping: u32,
    weather: (bool, f32, f32),
    /// The clock and weather last sent in the levels not in place, by
    /// dimension (`set_dimension` swaps them with the fields above).
    level_sync: std::collections::HashMap<String, LevelSync>,
    /// Operators and their permission levels (dedicated servers).
    pub ops: std::collections::HashMap<u128, u8>,
    /// `entity-broadcast-range-percentage` (dedicated servers).
    pub entity_range_percent: i32,
    /// `spawn-protection` (dedicated servers): the radius around the world
    /// spawn that only operators may change.
    pub spawn_protection: i32,
    /// `chat-spam-threshold-seconds` as a TickThrottler threshold (20 a second):
    /// over it, a guest who is not an operator is kicked. 0 never kicks.
    pub chat_spam_threshold: i32,
    /// `command-spam-threshold-seconds`, as `chat_spam_threshold`.
    pub command_spam_threshold: i32,
    /// `server.toml`'s `admin_feedback_to_ops` (dedicated servers): whether
    /// the other operators get a command's `[name: ...]` line (vanilla's
    /// broadcastToAdmins). False keeps it to the console's log.
    pub admin_feedback_to_ops: bool,
    chat_type: i32,
    dimension_type: i32,
    clock_ids: Vec<i32>,
    item_ids: std::collections::HashMap<Arc<str>, i32>,
    entity_types: std::collections::HashMap<String, i32>,
    /// Each level's tracked entities, by dimension.
    entities: std::collections::HashMap<String, Entities>,
    /// What one tick of entity tracking sends each guest, kept between ticks.
    entity_out: Vec<Vec<Vec<u8>>>,
    /// Each placed guest's entities gone this tick, where they were last sent (only while the
    /// prediction records).
    entity_gone: Vec<Vec<[f64; 3]>>,
    /// Server mods' commands, a root in the commands report's format, sent
    /// after vanilla's; the key the game last set them for.
    mod_commands: (serde_json::Value, Option<u64>),
}

fn string(s: &str) -> Tag {
    Tag::String(JavaString::from(s))
}

/// Player.getDisplayName for a player in no team: the name, suggesting
/// `/tell NAME `, showing the player on hover and inserting the name.
pub fn display_name(uuid: u128, name: &str) -> Tag {
    let words = [
        (uuid >> 96) as i32,
        (uuid >> 64) as i32,
        (uuid >> 32) as i32,
        uuid as i32,
    ];
    Tag::compound([
        ("text", string(name)),
        (
            "click_event",
            Tag::compound([
                ("action", string("suggest_command")),
                ("command", string(&format!("/tell {name} "))),
            ]),
        ),
        (
            "hover_event",
            Tag::compound([
                ("action", string("show_entity")),
                ("id", string("minecraft:player")),
                ("uuid", Tag::IntArray(words.to_vec())),
                ("name", string(name)),
            ]),
        ),
        ("insertion", string(name)),
    ])
}

/// `LivingEntity.DATA_ARROW_COUNT_ID` (12) and `DATA_STINGER_COUNT_ID` (13):
/// the counts of what is stuck in a player, 0 by default.
fn stuck_meta(state: &mut EntityState, (arrows, stingers): (i32, i32)) {
    for (index, count) in [(12, arrows), (13, stingers)] {
        if count != 0 {
            state.meta.push((index, serializer::INT, Value::Int(count)));
        }
        state.defaults.push((index, serializer::INT, Value::Int(0)));
    }
}

/// `Player.DATA_SHOULDER_PARROT_LEFT` (19) and `_RIGHT` (20), an
/// `OptionalInt`: 0 for none, else the parrot variant's id plus one.
fn shoulder_meta(state: &mut EntityState, shoulders: [Option<i32>; 2]) {
    for (index, variant) in [(19, shoulders[0]), (20, shoulders[1])] {
        if let Some(v) = variant {
            state
                .meta
                .push((index, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(v + 1)));
        }
        state
            .defaults
            .push((index, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(0)));
    }
}

impl Hub {
    /// Player.getDisplayName: the name formatted for the player's team
    /// (PlayerTeam.formatNameForTeam), suggesting `/tell NAME `, showing the
    /// player on hover and inserting the name.
    /// A nickname (`nicknames`) takes the name's place in the text; the
    /// hover, the suggested `/tell` and the insertion keep the real name.
    pub fn player_display_name(&self, uuid: u128, name: &str) -> Tag {
        let shown = self.nicknames.get(&uuid).map_or(name, String::as_str);
        let team = self.teams.iter().find(|t| t.players.contains(name));
        if team.is_none() && shown == name {
            return display_name(uuid, name);
        }
        let mut c = match team {
            Some(team) => team.formatted_name_comp(&crate::scoreboard::literal(shown)),
            None => serde_json::json!({ "text": shown }),
        };
        c["click_event"] =
            serde_json::json!({"action": "suggest_command", "command": format!("/tell {name} ")});
        c["hover_event"] = serde_json::json!({"action": "show_entity", "id": "minecraft:player",
            "uuid": horizon_protocol::crypto::uuid_string(uuid), "name": name});
        c["insertion"] = name.into();
        crate::scoreboard::component_tag(&c)
    }
}

/// A yellow translation of player names (`multiplayer.player.joined`).
fn yellow_names(key: &str, names: Vec<Tag>) -> Tag {
    Tag::compound([
        ("translate", string(key)),
        ("with", Tag::List(names)),
        ("color", string("yellow")),
    ])
}

/// An entity type's registry id, remembered once looked up.
fn entity_type(
    types: &mut std::collections::HashMap<String, i32>,
    sync: &SyncData,
    kind: &str,
) -> Option<i32> {
    if let Some(id) = types.get(kind) {
        return Some(*id);
    }
    let id = sync.builtin.get("minecraft:entity_type")?.id(kind)?;
    types.insert(kind.to_owned(), id);
    Some(id)
}

/// ChunkTrackingView.contains: the chunks a player at (cx, cz) with view
/// distance `r` is sent (isWithinDistance with the neighbours' buffer).
fn tracked(cx: i32, cz: i32, r: i32, x: i32, z: i32) -> bool {
    let dx = i64::from(((x - cx).abs() - 2).max(0));
    let dz = i64::from(((z - cz).abs() - 2).max(0));
    dx * dx + dz * dz < i64::from(r) * i64::from(r)
}

/// The view's queue around a new centre: the queued chunks still tracked, then
/// the tracked ones neither sent nor queued, closest first (a stable sort, so
/// equally close chunks keep their order).
fn requeue(
    queue: &mut Vec<(i32, i32)>,
    sent: &crate::fast_hash::FxHashSet<(i32, i32)>,
    center: (i32, i32),
    r: i32,
) {
    requeue_ring(queue, sent, center, 0, r);
}

/// `requeue` for the chunks tracked at `r` and not at `inner` (0: none): the far ring's.
fn requeue_ring(
    queue: &mut Vec<(i32, i32)>,
    sent: &crate::fast_hash::FxHashSet<(i32, i32)>,
    (cx, cz): (i32, i32),
    inner: i32,
    r: i32,
) {
    let tracked = |x, z| tracked(cx, cz, r, x, z) && !tracked(cx, cz, inner, x, z);
    queue.retain(|&(x, z)| tracked(x, z));
    // A walking player changes chunk every few ticks with hundreds of chunks
    // queued: a set, not a scan of the queue for each of the view's chunks.
    let mut queued: crate::fast_hash::FxHashSet<(i32, i32)> = queue.iter().copied().collect();
    for dz in -r - 2..=r + 2 {
        for dx in -r - 2..=r + 2 {
            let key = (cx + dx, cz + dz);
            if tracked(key.0, key.1) && !sent.contains(&key) && queued.insert(key) {
                queue.push(key);
            }
        }
    }
    queue.sort_by_key(|&(x, z)| i64::from(x - cx).pow(2) + i64::from(z - cz).pow(2));
}

/// A player's next chunks (`Hub::next_batch`), with their places in its queues: the view's, then
/// the far ring's (none: the save has none to send).
struct Batch<'w> {
    near: Vec<(usize, &'w Chunk)>,
    far: Vec<(usize, Option<Column<'w>>)>,
}

/// A far chunk to send: the level's own while it holds it, else what the save's readers made.
enum Column<'w> {
    Level(&'w Chunk),
    Saved(Arc<[u8]>),
}

/// A body in a batch: a column of `Columns`, or one read from the save.
enum Body {
    Level(usize),
    Saved(Arc<[u8]>),
}

/// The level's columns a tick's batches send, each once, with the number of batches taking it.
#[derive(Default)]
struct Columns<'w> {
    chunks: Vec<&'w Chunk>,
    uses: Vec<usize>,
    index: crate::fast_hash::FxHashMap<(i32, i32), usize>,
}

impl<'w> Columns<'w> {
    fn add(&mut self, chunk: &'w Chunk) -> usize {
        let j = *self.index.entry((chunk.x, chunk.z)).or_insert_with(|| {
            self.chunks.push(chunk);
            self.uses.push(0);
            self.chunks.len() - 1
        });
        self.uses[j] += 1;
        j
    }
}

/// What a level's players were last told of its clock and weather.
#[derive(Clone, Copy)]
struct LevelSync {
    last_clock: i64,
    last_rate: f32,
    weather: (bool, f32, f32),
}

impl Default for LevelSync {
    fn default() -> Self {
        Self {
            last_clock: i64::MIN,
            last_rate: 0.0,
            weather: (false, 0.0, 0.0),
        }
    }
}

impl Hub {
    pub fn new(lan: Lan, sync: Arc<SyncData>, encoder: ChunkEncoder, config: HubConfig) -> Self {
        let chat_type = sync
            .id("minecraft:chat_type", "minecraft:chat")
            .unwrap_or(0);
        let dimension_type = sync
            .id("minecraft:dimension_type", &config.dimension)
            .unwrap_or(0);
        let clock_ids = (0..sync.dynamic.len("minecraft:world_clock") as i32).collect();
        let mut encoder = encoder;
        encoder.set_dimension(config.levels.get(&config.dimension));
        Self {
            lan,
            config,
            encoder,
            guests: Vec::new(),
            sync,
            ticks: 0,
            seat_order: 0,
            sound_seeds: Mutex::new(LegacyRandom::new(crate::sound_engine::unique_seed())),
            immediate_respawn: false,
            movement_checks: (true, true),
            anticheat: None,
            chat_filter: None,
            far: None,
            singleplayer_owner: None,
            flat_world: false,
            debug_world: false,
            riding: Default::default(),
            last_clock: i64::MIN,
            last_rate: 0.0,
            ticking: (20.0, false),
            stepping: 0,
            ops: Default::default(),
            entity_range_percent: 100,
            spawn_protection: 0,
            chat_spam_threshold: 0,
            command_spam_threshold: 0,
            admin_feedback_to_ops: true,
            weather: (false, 0.0, 0.0),
            level_sync: Default::default(),
            chat_type,
            dimension_type,
            clock_ids,
            item_ids: Default::default(),
            entity_types: Default::default(),
            entities: Default::default(),
            entity_out: Vec::new(),
            entity_gone: Vec::new(),
            mod_commands: (serde_json::Value::Null, None),
            native_menu_events: Vec::new(),
            native_tracking: Vec::new(),
            teams: Vec::new(),
            nicknames: std::collections::HashMap::new(),
        }
    }

    pub fn guest(&self, conn: u32) -> Option<&Guest> {
        self.guests.iter().find(|g| g.conn == conn)
    }
    pub fn guest_mut(&mut self, conn: u32) -> Option<&mut Guest> {
        self.guests.iter_mut().find(|g| g.conn == conn)
    }

    /// The network id of a name in a built-in or synchronized registry.
    pub fn registry_id(&self, registry: &str, name: &str) -> Option<i32> {
        self.sync.id(registry, name)
    }

    /// The name of a network id in a built-in registry.
    pub fn registry_name(&self, registry: &str, id: i32) -> Option<&str> {
        self.sync.builtin.get(registry)?.name(id)
    }

    pub fn item_id(&mut self, item: &Arc<str>) -> i32 {
        if let Some(id) = self.item_ids.get(item) {
            return *id;
        }
        let id = self
            .sync
            .builtin
            .get("minecraft:item")
            .and_then(|i| i.id(item))
            .unwrap_or(0);
        self.item_ids.insert(item.clone(), id);
        id
    }
    pub fn stack(&mut self, s: &Option<ItemStack>) -> Option<horizon_protocol::item::Stack> {
        let item = s.as_ref()?.item.clone();
        let id = self.item_id(&item);
        let sync = self.sync.clone();
        // The stacks inside it (a bundle's contents, a crossbow's projectiles)
        // have ids of their own.
        let item_id = |name: &str| {
            if name == &*item {
                return id;
            }
            sync.builtin
                .get("minecraft:item")
                .and_then(|i| i.id(name))
                .unwrap_or(0)
        };
        stack_to_network(item_id, |registry, name| sync.id(registry, name), s)
    }

    /// `PlayerList.broadcastAll`: send a packet to every placed player.
    pub fn broadcast(&self, packet: &C) {
        let body = packet.encode();
        for g in self.guests.iter().filter(|g| g.placed) {
            g.sender.send(body.clone());
        }
    }
    /// MinecraftServer.onGameRuleChanged: every player is told when
    /// `immediate_respawn` changes; a joining player's Login carries it.
    pub fn set_immediate_respawn(&mut self, on: bool) {
        if self.immediate_respawn != on {
            self.immediate_respawn = on;
            self.broadcast(&C::GameEvent(play::GameEvent {
                event: game_event::IMMEDIATE_RESPAWN,
                param: if on { 1.0 } else { 0.0 },
            }));
        }
    }
    /// PlayerList.setViewDistance: every player is told the new radius,
    /// and each one's chunks follow it, up to what they asked for
    /// (ChunkMap.getPlayerViewDistance).
    pub fn set_view_distance(&mut self, distance: i32) {
        self.config.view_distance = distance;
        let distance = self.chunk_radius();
        self.broadcast(&C::SetChunkCacheRadius(play::Distance { distance }));
        for g in &mut self.guests {
            g.view_distance = i32::from(g.info.view_distance).clamp(2, self.config.view_distance);
            g.center = None;
        }
    }
    /// The radius the players are told (Login, SetChunkCacheRadius): with the far ring on, its
    /// `max_view_distance`, so that a client keeps and draws what it is sent past the view.
    fn chunk_radius(&self) -> i32 {
        let view = self.config.view_distance;
        self.far.as_ref().map_or(view, |far| far.max.max(view))
    }
    /// How far a player is sent chunks: its view, and with the far ring on, as far as it asked for
    /// up to `max_view_distance`.
    fn reach(&self, g: &Guest) -> i32 {
        self.far.as_ref().map_or(g.view_distance, |far| {
            i32::from(g.info.view_distance)
                .min(far.max)
                .max(g.view_distance)
        })
    }
    /// Where the far ring reads the level in place from: its saves, before each of its ticks.
    pub fn far_level(&mut self, save: &Arc<crate::save::SaveDir>) {
        if let Some(far) = self.far.as_mut() {
            far.set_level(&self.config.dimension, save, &self.encoder);
        }
    }
    /// PlayerList.setSimulationDistance.
    pub fn set_simulation_distance(&mut self, distance: i32) {
        self.config.simulation_distance = distance;
        self.broadcast(&C::SetSimulationDistance(play::Distance { distance }));
    }
    /// Whether a guest is placed in the level in place (`config.dimension`).
    pub fn here(&self, g: &Guest) -> bool {
        g.placed && g.dimension == self.config.dimension
    }
    /// `PlayerList.broadcastAll(packet, dimension)`: the players in the
    /// level in place.
    pub fn broadcast_level(&self, packet: &C) {
        let body = packet.encode();
        for g in self.guests.iter().filter(|g| self.here(g)) {
            g.sender.send(body.clone());
        }
    }
    /// `DedicatedServer.sendLowDiskSpaceWarning`: the players with
    /// `PermissionLevel.ADMINS` (3) or more get `ClientboundLowDiskSpaceWarningPacket`.
    pub fn send_low_disk_space_warning(&self) {
        let body = C::LowDiskSpaceWarning(horizon_protocol::packets::Empty).encode();
        for g in self.guests.iter().filter(|g| g.placed && g.permission >= 3) {
            g.sender.send(body.clone());
        }
    }
    pub fn broadcast_except(&self, conn: u32, packet: &C) {
        let body = packet.encode();
        for g in self.guests.iter().filter(|g| g.placed && g.conn != conn) {
            g.sender.send(body.clone());
        }
    }
    /// PlayerList.broadcastSystemMessage.
    pub fn system_message(&self, content: Tag) {
        self.broadcast(&C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        }));
    }
    /// A system message for the guests `hears` picks by name
    /// (`PlayerList.broadcastSystemToTeam` and its kin).
    pub fn system_message_to(&self, content: Tag, hears: impl Fn(&str) -> bool) {
        let body = C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        })
        .encode();
        for g in self.guests.iter().filter(|g| g.placed && hears(&g.name)) {
            g.sender.send(body.clone());
        }
    }
    /// `ServerPlayer.sendSystemMessage` to the guest with `uuid`, in any
    /// dimension.
    pub fn system_message_to_player(&self, uuid: u128, content: Tag) {
        let body = C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        })
        .encode();
        for g in self.guests.iter().filter(|g| g.placed && g.uuid == uuid) {
            g.sender.send(body.clone());
        }
    }
    /// `ServerPlayer.sendOverlayMessage` to every guest (the action bar).
    pub fn overlay_message(&self, content: Tag) {
        self.broadcast(&C::SystemChat(play::SystemChat {
            content,
            overlay: true,
        }));
    }
    /// A chat message from the host (a disguised chat: the host has no
    /// connection to sign with).
    pub fn chat(&self, sender: &str, text: &str) {
        self.broadcast(&C::DisguisedChat(play::DisguisedChat {
            message: literal(text),
            chat_type: play::ChatTypeBound {
                chat_type: self.chat_type,
                name: literal(sender),
                target: None,
            },
        }));
    }
    /// ListPlayersCommand's reply: the players in join order (the host of a
    /// LAN world first), as display names or names with ids, joined by
    /// gray commas (ComponentUtils.formatList).
    pub fn list_reply(&self, host: Option<(u128, &str)>, uuids: bool) -> Tag {
        let players: Vec<(u128, &str)> = host
            .into_iter()
            .chain(
                self.guests
                    .iter()
                    .filter(|g| g.placed)
                    .map(|g| (g.uuid, g.name.as_str())),
            )
            .collect();
        let format = |(uuid, name): (u128, &str)| {
            if uuids {
                Tag::compound([
                    ("translate", string("commands.list.nameAndId")),
                    (
                        "with",
                        Tag::List(vec![
                            string(name),
                            string(&horizon_protocol::crypto::uuid_string(uuid)),
                        ]),
                    ),
                ])
            } else {
                display_name(uuid, name)
            }
        };
        let names = match players.len() {
            0 => string(""),
            1 => format(players[0]),
            _ => {
                let mut extra = Vec::new();
                for (k, p) in players.iter().enumerate() {
                    if k > 0 {
                        extra.push(Tag::compound([
                            ("text", string(", ")),
                            ("color", string("gray")),
                        ]));
                    }
                    extra.push(format(*p));
                }
                Tag::compound([("text", string("")), ("extra", Tag::List(extra))])
            }
        };
        Tag::compound([
            ("translate", string("commands.list.players")),
            (
                "with",
                Tag::List(vec![
                    Tag::Int(players.len() as i32),
                    Tag::Int(self.lan.shared.max_players() as i32),
                    names,
                ]),
            ),
        ])
    }

    /// SayCommand from a source without a player: a disguised chat of type
    /// say_command named after the source.
    pub fn say(&self, source: &str, text: &str) {
        let chat_type = self
            .sync
            .id("minecraft:chat_type", "minecraft:say_command")
            .unwrap_or(self.chat_type);
        self.broadcast(&C::DisguisedChat(play::DisguisedChat {
            message: literal(text),
            chat_type: play::ChatTypeBound {
                chat_type,
                name: literal(source),
                target: None,
            },
        }));
    }
    /// Disconnect a guest with a reason (ServerGamePacketListenerImpl.disconnect).
    pub fn kick(&mut self, conn: u32, reason: Tag) {
        // The log's "lost connection" line gives the reason, as vanilla's.
        let text = self.english(&reason);
        if let Some(g) = self.guest_mut(conn) {
            g.left_reason = text;
            g.sender.disconnect(reason);
        }
    }

    fn player_info(&self, uuid: u128, name: &str, mode: i32, latency: i32) -> play::PlayerInfo {
        let guest = self.guests.iter().find(|g| g.uuid == uuid);
        play::PlayerInfo {
            uuid,
            name: Some(name.to_owned()),
            properties: guest
                .map(|g| {
                    listed_properties(
                        &g.properties,
                        self.singleplayer_owner == Some(g.uuid),
                        self.lan.shared.config.owner_profile.as_deref(),
                    )
                })
                .unwrap_or_default(),
            chat_session: guest.and_then(|g| g.chain.session()).map(|s| s.data()),
            game_mode: Some(mode),
            listed: Some(true),
            latency: Some(latency),
            display_name: Some(self.nicknames.get(&uuid).map(|n| literal(n))),
            list_order: Some(0),
            show_hat: Some(guest.is_none_or(|g| g.info.model_customisation & 0x40 != 0)),
        }
    }
    /// ClientboundPlayerInfoUpdatePacket.createPlayerInitializing: every action.
    const INFO_ACTIONS: u8 = 0xFF;

    // ---- joining -------------------------------------------------------

    /// `CommonPlayerSpawnInfo`, with the player's `portalCooldown`.
    fn spawn_info(&self, i: usize, portal_cooldown: i32) -> play::SpawnInfo {
        let g = &self.guests[i];
        play::SpawnInfo {
            dimension_type: self.dimension_type,
            dimension: self.config.dimension.clone(),
            seed: horizon_core::biome::obfuscate_seed(self.config.seed),
            game_type: g.game_mode,
            previous_game_type: None,
            debug: self.debug_world,
            flat: self.flat_world,
            // Player.getLastDeathLocation.
            last_death: g
                .last_death
                .as_ref()
                .map(|(dimension, pos)| play::GlobalPos {
                    dimension: dimension.clone(),
                    pos: *pos,
                }),
            portal_cooldown,
            sea_level: 63,
        }
    }

    fn admit(&mut self, j: super::Joined, ctx: &mut TickCtx) -> u32 {
        // The game's ids are taken by incrementing first: the current value
        // is the last id in use (the host's own, right after opening to LAN).
        *ctx.next_entity += 1;
        let entity_id = *ctx.next_entity as i32;
        let conn = j.conn;
        let mut guest = Guest::new(j, entity_id, ctx.spawn, self.config.view_distance);
        guest.last_action_tick = self.ticks;
        guest.game_mode = self.config.default_game_mode;
        guest.permission = self.default_permission(guest.uuid);
        guest.joined_tick = self.ticks;
        guest.position = ctx.spawn;
        guest.previous_position = ctx.spawn;
        guest.dimension = self.config.dimension.clone();
        self.guests.push(guest);
        conn
    }

    /// The permission level a player gets: an integrated server gives every
    /// guest the game masters' level when commands are allowed
    /// (IntegratedServer.setGuestCommandAccess); a dedicated server its
    /// operators' level (`ops`).
    fn default_permission(&self, uuid: u128) -> u8 {
        if self.config.integrated {
            // IntegratedServer.getProfilePermissions / getCustomPermissionLevel.
            let guests = if self.config.guest_commands { 2 } else { 0 };
            self.ops.get(&uuid).copied().unwrap_or(guests)
        } else if self.config.dedicated {
            self.ops.get(&uuid).copied().unwrap_or(0)
        } else if self.config.guest_commands {
            2
        } else {
            0
        }
    }

    /// The levels of a server, in MinecraftServer.levelKeys order.
    fn level_keys() -> Vec<String> {
        ["overworld", "the_end", "the_nether"]
            .map(|l| format!("minecraft:{l}"))
            .to_vec()
    }

    /// PlayerList.placeNewPlayer for a guest whose saved state is loaded.
    pub fn place_new_player(&mut self, conn: u32, ctx: &mut TickCtx) {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let (name, uuid, entity_id, mode) = {
            let g = &self.guests[i];
            (g.name.clone(), g.uuid, g.entity_id, g.game_mode)
        };
        let g = &self.guests[i];
        g.send(&C::Login(play::Login {
            player_id: entity_id,
            hardcore: self.config.hardcore,
            levels: Self::level_keys(),
            max_players: self.config.max_players as i32,
            chunk_radius: self.chunk_radius(),
            simulation_distance: self.config.simulation_distance,
            reduced_debug_info: false,
            show_death_screen: !self.immediate_respawn,
            limited_crafting: false,
            spawn: self.spawn_info(i, 0),
            // MinecraftServer.usesAuthentication, enforceSecureProfile.
            online_mode: self.lan.shared.config.online.is_some(),
            enforces_secure_chat: self.enforces_secure_chat(),
        }));
        g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
            difficulty: ctx.difficulty,
            locked: ctx.difficulty_locked,
        }));
        g.send(&g.abilities());
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        // PlayerList.placeNewPlayer: ClientboundUpdateRecipesPacket.
        g.send(&raw(ids::UPDATE_RECIPES, self.sync.recipes.clone()));
        // OnDatapackSyncEvent.sendRecipes: the recipes the native mods asked
        // for, to a Horizon client only.
        if g.brand == "horizon" {
            send_recipe_content(g);
        }
        // PlayerList.sendPlayerPermissionLevel, then the recipe book.
        self.send_permission_level(i);
        self.send_initial_recipe_book(i);
        // PlayerList.placeNewPlayer broadcasts the join line before
        // players.add: the others read it, the newcomer doesn't.
        self.broadcast_except(
            conn,
            &C::SystemChat(play::SystemChat {
                content: yellow_names(
                    "multiplayer.player.joined",
                    vec![display_name(self.guests[i].uuid, &name)],
                ),
                overlay: false,
            }),
        );
        self.guests[i].placed = true;
        self.resend_teleport(i, None);
        let g = &self.guests[i];
        g.send(&C::ServerData(play::ServerData {
            motd: literal(&self.lan.shared.motd()),
            icon: self.lan.shared.config.favicon.clone(),
        }));
        // The player list: everyone already here, then the newcomer to all.
        let mut entries = Vec::new();
        if let Some(host) = ctx.host {
            entries.push(self.player_info(host.uuid, &host.name, host.game_mode, 0));
        }
        for o in self.guests.iter().filter(|o| o.conn != conn && o.placed) {
            entries.push(self.player_info(
                o.uuid,
                &o.name,
                o.game_mode,
                o.latency.load(Ordering::Relaxed),
            ));
        }
        self.guests[i].send(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: Self::INFO_ACTIONS,
            entries,
        }));
        let newcomer = play::PlayerInfoUpdate {
            actions: Self::INFO_ACTIONS,
            entries: vec![self.player_info(uuid, &name, mode, 0)],
        };
        self.broadcast(&C::PlayerInfoUpdate(newcomer));
        self.send_level_info(i, ctx, true);
        // ServerLevel.addNewPlayer: the chunk tracking starts at the player.
        let g = &mut self.guests[i];
        let (cx, cz) = g.chunk();
        g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
            x: cx,
            z: cz,
        }));
        g.center_sent = Some((cx, cz));
        g.placed_at = self.ticks;
        self.send_active_effects(i);
        // ServerPlayer.sendPostEffects and initInventoryMenu.
        self.send_post_effects(i);
        self.send_inventory(i);
    }

    /// `ServerPlayer.sendPostEffects`: the player's `/posteffect` chains.
    pub fn send_post_effects(&mut self, i: usize) {
        let g = &mut self.guests[i];
        g.post_effects_dirty = false;
        let effects = g.post_effects.clone();
        g.send(&C::PostEffects(play::PostEffects { effects }));
    }

    /// ClientboundUpdateMobEffectPacket for one of the guest's effects.
    fn send_effect(&mut self, i: usize, e: &EffectInstance, blend: bool) -> Option<SentEffect> {
        let effect = self
            .sync
            .id("minecraft:mob_effect", crate::brewing::namespaced(e.id))?;
        let flags = PlayerEffects::flags(e);
        let g = &self.guests[i];
        g.send(&C::UpdateMobEffect(play::UpdateMobEffect {
            id: g.entity_id,
            effect,
            amplifier: e.amplifier,
            duration: e.duration,
            flags: flags | if blend { 8 } else { 0 },
        }));
        Some(SentEffect {
            id: e.id,
            amplifier: e.amplifier,
            flags,
            duration: e.duration,
            tick_count: g.effects.tick_count,
        })
    }

    /// PlayerList.sendActivePlayerEffects: every effect, unblended, when the
    /// player is placed (or respawns).
    pub fn send_active_effects(&mut self, i: usize) {
        let effects = self.guests[i].effects.list.clone();
        let sent = effects
            .iter()
            .filter_map(|e| self.send_effect(i, e, false))
            .collect();
        self.guests[i].sent_effects = sent;
    }

    /// ServerPlayer.onEffectAdded (blended), onEffectUpdated and
    /// onEffectsRemoved, from what changed since the client was told: an
    /// effect that only counted down is not sent again, except at every
    /// 600 ticks left (`LivingEntity.tickEffects`).
    /// `sync_effects` for every player at once: what a command changed
    /// leaves with its feedback (ServerPlayer.onEffectAdded sends at once),
    /// not after the tick's countdown.
    pub fn sync_all_effects(&mut self) {
        for i in 0..self.guests.len() {
            if self.guests[i].placed {
                self.sync_effects(i);
            }
        }
    }
    fn sync_effects(&mut self, i: usize) {
        let g = &self.guests[i];
        if g.effects.list.is_empty() && g.sent_effects.is_empty() {
            return;
        }
        let now = g.effects.tick_count;
        let mut sent = Vec::with_capacity(g.effects.list.len());
        let mut changes = Vec::new();
        for e in &g.effects.list {
            match g.sent_effects.iter().find(|s| s.id == e.id) {
                None => changes.push((e.clone(), true)),
                Some(s) => {
                    let ticked = now.wrapping_sub(s.tick_count);
                    let expected = if s.duration == -1 {
                        -1
                    } else {
                        s.duration - ticked
                    };
                    let refresh = ticked > 0 && e.duration > 0 && e.duration % 600 == 0;
                    if s.amplifier != e.amplifier
                        || s.flags != PlayerEffects::flags(e)
                        || e.duration != expected
                        || refresh
                    {
                        changes.push((e.clone(), false));
                    } else {
                        sent.push(s.clone());
                    }
                }
            }
        }
        let removed: Vec<&'static str> = g
            .sent_effects
            .iter()
            .filter(|s| g.effects.get(s.id).is_none())
            .map(|s| s.id)
            .collect();
        for id in removed {
            let Some(effect) = self
                .sync
                .id("minecraft:mob_effect", crate::brewing::namespaced(id))
            else {
                continue;
            };
            let g = &self.guests[i];
            g.send(&C::RemoveMobEffect(play::RemoveMobEffect {
                id: g.entity_id,
                effect,
            }));
        }
        for (e, added) in changes {
            sent.extend(self.send_effect(i, &e, added));
        }
        self.guests[i].sent_effects = sent;
    }

    /// PlayerList.sendPlayerPermissionLevel: the op level as an entity event
    /// (24 + level), then the commands that level may use.
    fn send_permission_level(&self, i: usize) {
        let g = &self.guests[i];
        g.send(&C::EntityEvent(play::EntityEvent {
            id: g.entity_id,
            event: 24 + g.permission.min(4) as i8,
        }));
        self.send_commands(i);
    }

    /// The permissions changed: tell everyone their level and commands again.
    pub fn resend_commands(&mut self) {
        for i in 0..self.guests.len() {
            self.guests[i].permission = self.default_permission(self.guests[i].uuid);
            if self.guests[i].placed {
                self.send_permission_level(i);
            }
        }
    }

    /// Whether vanilla's command tree has a command of this name.
    pub fn vanilla_command(&self, name: &str) -> bool {
        self.sync.commands.as_ref().is_some_and(|t| t.has(name))
    }

    /// The key [`Hub::set_mod_commands`] last had: the game compares it
    /// with its server mods' each tick.
    pub fn mod_commands_key(&self) -> Option<u64> {
        self.mod_commands.1
    }
    /// Server mods' commands changed: every player gets their tree again.
    pub fn set_mod_commands(&mut self, key: u64, commands: serde_json::Value) {
        let resend = self.mod_commands.0 != commands;
        self.mod_commands = (commands, Some(key));
        if resend {
            for i in 0..self.guests.len() {
                if self.guests[i].placed {
                    self.send_commands(i);
                }
            }
        }
    }

    /// Commands.sendCommands: the command tree for this player.
    fn send_commands(&self, i: usize) {
        let g = &self.guests[i];
        let packet = match &self.sync.commands {
            Some(tree) => {
                let dedicated = self.config.dedicated && !self.config.integrated;
                tree.packet_with(
                    g.permission,
                    dedicated,
                    &self.sync.builtin,
                    &self.mod_commands.0,
                )
            }
            None => play::Commands {
                nodes: vec![play::CommandNode {
                    kind: 0,
                    executable: false,
                    restricted: false,
                    children: Vec::new(),
                    redirect: None,
                    name: String::new(),
                    parser: None,
                    suggestions: None,
                }],
                root: 0,
            },
        };
        g.send(&C::Commands(packet));
    }

    /// ServerGamePacketListenerImpl.teleport: move the client and wait for its confirmation.
    pub fn teleport(&mut self, i: usize, position: Option<[f64; 3]>) {
        // ServerPlayer.teleportTo: resetFlyingTicks.
        self.guests[i].above_ground_ticks = 0;
        self.teleport_relative(i, position, 0);
    }

    /// `ServerGamePacketListenerImpl.teleport` alone (a refused or too fast
    /// move, a teleport sent again): the flying ticks go on.
    pub(super) fn resend_teleport(&mut self, i: usize, position: Option<[f64; 3]>) {
        self.teleport_relative(i, position, 0);
    }

    /// A teleport to `position` that keeps the player's motion and look
    /// (`Relative.DELTA` and `Relative.ROTATION`: an End gateway's).
    pub fn teleport_keeping_motion(&mut self, i: usize, position: [f64; 3]) {
        self.guests[i].above_ground_ticks = 0;
        use play::relative::{DELTA_X, DELTA_Y, DELTA_Z, ROTATE_DELTA, X_ROT, Y_ROT};
        let relatives = Y_ROT | X_ROT | DELTA_X | DELTA_Y | DELTA_Z | ROTATE_DELTA;
        self.teleport_relative(i, Some(position), relatives);
    }

    /// ServerGamePacketListenerImpl.teleport with `relatives`: the rotation
    /// and motion they name are sent as zero changes.
    fn teleport_relative(&mut self, i: usize, position: Option<[f64; 3]>, relatives: i32) {
        let ticks = self.ticks;
        let g = &mut self.guests[i];
        g.teleport_tick = ticks;
        if let Some(p) = position {
            g.position = p;
            g.previous_position = p;
        }
        g.teleport_counter = (g.teleport_counter + 1) & 0x7FFF_FFFF;
        let id = g.teleport_counter;
        g.awaiting_teleport = Some((id, g.position));
        use play::relative::{DELTA_X, DELTA_Y, DELTA_Z};
        g.awaiting_keeps_motion = relatives & (DELTA_X | DELTA_Y | DELTA_Z) != 0;
        let turn = relatives & (play::relative::Y_ROT | play::relative::X_ROT) != 0;
        g.send(&C::PlayerPosition(play::PlayerPosition {
            teleport: id,
            position: g.position,
            delta: [0.0; 3],
            yaw: if turn { 0.0 } else { g.yaw },
            pitch: if turn { 0.0 } else { g.pitch },
            relatives,
        }));
    }

    /// ClientboundSetDefaultSpawnPositionPacket for the level's respawn data
    /// (the world spawn, in the Overworld wherever the player is).
    fn default_spawn(&self, spawn: [f64; 3]) -> C {
        C::SetDefaultSpawnPosition(play::SetDefaultSpawnPosition {
            pos: play::GlobalPos {
                dimension: "minecraft:overworld".into(),
                pos: [
                    spawn[0].floor() as i32,
                    spawn[1].floor() as i32,
                    spawn[2].floor() as i32,
                ],
            },
            yaw: 0.0,
            pitch: 0.0,
        })
    }

    /// PlayerList.sendLevelInfo; `load_start` false holds back
    /// LEVEL_CHUNKS_LOAD_START for a player who has no position yet
    /// ([`Self::land`] sends it).
    fn send_level_info(&mut self, i: usize, ctx: &TickCtx, load_start: bool) {
        let spawn = ctx.spawn;
        let g = &self.guests[i];
        g.send(&C::InitializeBorder(super::border::initialize(ctx.border)));
        g.send(&self.time_packet(ctx.world, ctx.clock_rate));
        g.send(&self.default_spawn(spawn));
        let w = &ctx.world.weather;
        if w.raining {
            g.send(&C::GameEvent(play::GameEvent {
                event: game_event::START_RAINING,
                param: 0.0,
            }));
            g.send(&C::GameEvent(play::GameEvent {
                event: game_event::RAIN_LEVEL,
                param: w.rain_level.1,
            }));
            g.send(&C::GameEvent(play::GameEvent {
                event: game_event::THUNDER_LEVEL,
                param: w.thunder_level.1,
            }));
        }
        if load_start {
            g.send(&C::GameEvent(play::GameEvent {
                event: game_event::LEVEL_CHUNKS_LOAD_START,
                param: 0.0,
            }));
        }
        // TickRateManager.updateJoiningPlayer.
        g.send(&ticking_state(ctx.tick_rate, ctx.tick_frozen));
        g.send(&C::TickingStep(play::TickingStep {
            steps: ctx.tick_steps.remaining,
        }));
    }

    /// ServerClockManager.createFullSyncPacket: every clock. Horizon keeps the
    /// overworld clock (the day time); the End's clock runs with the game
    /// time. Clocks do not advance without the advance_time rule.
    fn time_packet(&self, world: &World, rate: f32) -> C {
        let overworld = self
            .sync
            .id("minecraft:world_clock", "minecraft:overworld")
            .unwrap_or(0);
        C::SetTime(play::SetTime {
            game_time: world.game_time,
            clocks: self
                .clock_ids
                .iter()
                .map(|&id| {
                    if id == overworld {
                        play::ClockState {
                            clock: id,
                            total_ticks: world.day_time,
                            partial_tick: 0.0,
                            rate,
                        }
                    } else {
                        play::ClockState {
                            clock: id,
                            total_ticks: world.game_time,
                            partial_tick: 0.0,
                            rate: if rate > 0.0 { 1.0 } else { 0.0 },
                        }
                    }
                })
                .collect(),
        })
    }

    /// ServerPlayer.initInventoryMenu / sendAllDataToRemote for the inventory menu.
    /// ClientboundPlaceGhostRecipePacket: the recipe `id` faded into the
    /// guest's menu `container`, whose items were missing.
    pub fn send_ghost_recipe(&mut self, conn: u32, container: i32, id: &str) {
        let Some(display) = self.sync.advancements.ghost_display(id, &self.sync) else {
            return;
        };
        self.send_to(
            conn,
            &C::PlaceGhostRecipe(horizon_protocol::packets::recipes::PlaceGhostRecipe {
                container,
                display,
            }),
        );
    }
    pub fn send_inventory(&mut self, i: usize) {
        let items: Vec<Option<ItemStack>> = (0..46).map(|s| self.guests[i].menu_slot(s)).collect();
        let net: Vec<_> = items.iter().map(|s| self.stack(s)).collect();
        let carried = self.guests[i].carried.clone();
        let carried = self.stack(&carried);
        let g = &mut self.guests[i];
        g.menu_state = (g.menu_state + 1) & 0x7FFF;
        g.send(&C::ContainerSetContent(play::ContainerSetContent {
            container: 0,
            state: g.menu_state,
            items: net,
            carried,
        }));
        g.menu_sent = items;
        g.sent_selected = g.selected;
    }

    // ---- per tick ------------------------------------------------------

    /// A whole tick with one level: the network, the level, then the
    /// players' latencies.
    pub fn tick(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let mut events = self.poll(ctx);
        events.extend(self.drain_packets(ctx));
        events.extend(self.tick_level(ctx));
        self.end_tick();
        events
    }

    /// The network since the last tick, once per server tick before any
    /// level ticks: joins (into the level in place), leaves, and each guest's
    /// packets queued for its own level's pass.
    pub fn poll(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        self.ticks += 1;
        self.anticheat_tick(ctx.tick_rate);
        let mut events = Vec::new();
        for event in self.lan.poll() {
            match event {
                NetEvent::Joined(j) => {
                    let conn = self.admit(*j, ctx);
                    events.push(HostEvent::Joined(conn));
                }
                NetEvent::Left(conn, reason) => {
                    if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
                        let g = self.remove_guest(i, &reason);
                        events.push(HostEvent::Left(Box::new(g)));
                    }
                }
                NetEvent::Packet(conn, packet, bytes) => {
                    let Some(g) = self.guest_mut(conn) else {
                        continue;
                    };
                    // A kicked player's packets wait for nothing.
                    if !g.left_reason.is_empty() {
                        continue;
                    }
                    g.inbox.push_back((*packet, bytes));
                    g.inbox_bytes += bytes;
                    // A client sending faster than its budget is handled
                    // would grow the inbox until the server runs out of memory.
                    if g.inbox.len() > INBOX_LIMIT || g.inbox_bytes > INBOX_BYTES {
                        events.push(HostEvent::Warn(format!(
                            "{} sent more packets than the server handles ({} waiting, {} kB)",
                            g.name,
                            g.inbox.len(),
                            g.inbox_bytes >> 10
                        )));
                        g.inbox = Default::default();
                        g.inbox_bytes = 0;
                        self.kick(conn, translatable("disconnect.exceeded_packet_rate", &[]));
                    }
                }
                NetEvent::Duplicate(uuid) => {
                    let conns: Vec<u32> = self
                        .guests
                        .iter()
                        .filter(|g| g.uuid == uuid)
                        .map(|g| g.conn)
                        .collect();
                    for conn in conns {
                        self.kick(
                            conn,
                            translatable("multiplayer.disconnect.duplicate_login", &[]),
                        );
                    }
                }
            }
        }
        // keepConnectionAlive's timeout, checked once a second here, not on
        // the writer thread: a client that stops reading blocks that thread
        // and would hold its slot forever. Kicked, it gets the reason; still
        // here a second later, its connection is closed under the writer.
        if self.ticks.is_multiple_of(20) {
            for g in &self.guests {
                if !g.left_reason.is_empty() && g.sender.timed_out() {
                    g.sender.close();
                }
            }
            let silent: Vec<u32> = self
                .guests
                .iter()
                .filter(|g| g.left_reason.is_empty() && g.sender.timed_out())
                .map(|g| g.conn)
                .collect();
            for conn in silent {
                self.kick(conn, translatable("disconnect.timeout", &[]));
            }
        }
        events
    }

    /// Handle the players in this level's packets (`ServerGamePacketListenerImpl`
    /// for each), before the level's scheduled block ticks so a place landed in
    /// the same tick as a falling block's schedule still sees the support
    /// (#4510). `tick_level` then syncs what those packets (and the block ticks)
    /// changed.
    pub fn drain_packets(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let mut events = Vec::new();
        for i in 0..self.guests.len() {
            if self.here(&self.guests[i]) {
                self.process_inbox(i, ctx, &mut events);
                self.tick_floating(i, &mut events);
            }
        }
        events
    }

    /// The level in place's part of the tick (`ServerLevel.tick`'s chunk
    /// map and the connections of the players in it): the chunks, block
    /// changes, containers and entities they see. Player packets are drained
    /// earlier (`drain_packets`) so scheduled ticks see what they placed.
    pub fn tick_level(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let events = Vec::new();
        if !self.guests.iter().any(|g| self.here(g)) {
            // ServerTickRateManager broadcasts a change to the players there
            // are; one who joins later is sent the state as it is then
            // (`updateJoiningPlayer`).
            if !self.guests.iter().any(|g| g.placed) {
                self.ticking = (ctx.tick_rate, ctx.tick_frozen);
                self.stepping = ctx.tick_steps.revision;
            }
            if !self
                .guests
                .iter()
                .any(|g| g.dimension == self.config.dimension)
            {
                let _ = ctx.world.block_log.take();
                let _ = ctx.world.light_log.take();
                if let Some(far) = self.far.as_mut() {
                    far.forget_level();
                }
                ctx.world.bulk_log.clear();
            }
            return events;
        }
        if ctx.world.block_log.is_none() {
            ctx.world.block_log = Some(Vec::new());
        }
        if ctx.world.light_log.is_none() {
            ctx.world.light_log = Some(Default::default());
        }
        self.sync_world(ctx);
        crate::spans::time("lan_chunks", || self.update_chunks(ctx));
        self.broadcast_light_changes(ctx.world);
        self.broadcast_block_changes(ctx.world);
        // ServerGamePacketListenerImpl.tick: the actions handled since the last tick are
        // acknowledged after the block changes they made were sent, so a client's predicted
        // blocks settle on the server's states.
        for g in self.guests.iter().filter(|g| self.here(g)) {
            let sequence = g.ack_up_to.replace(-1);
            if sequence >= 0 {
                g.send(&C::BlockChangedAck(play::BlockChangedAck { sequence }));
            }
        }
        self.sync_containers(ctx.world);
        self.track_entities(ctx);
        for i in 0..self.guests.len() {
            if self.here(&self.guests[i]) {
                self.sync_player_state(i);
                self.sync_effects(i);
            }
        }
        events
    }

    /// After every level: `PlayerList.tick`.
    pub fn end_tick(&mut self) {
        let latencies: Vec<_> = self
            .guests
            .iter()
            .map(|g| (g.uuid, g.latency.load(Ordering::Relaxed)))
            .collect();
        // PlayerList.tick: UPDATE_LATENCY once `++sendAllPlayerInfoIn > 600`,
        // so on the 601st tick, then every 601.
        if self.ticks.is_multiple_of(601) && !latencies.is_empty() {
            let entries = latencies
                .iter()
                .map(|(uuid, latency)| play::PlayerInfo {
                    uuid: *uuid,
                    latency: Some(*latency),
                    ..Default::default()
                })
                .collect();
            self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 16,
                entries,
            }));
        }
    }

    /// The starts and stops of tracking the guests had since the last call
    /// are kept in `native_tracking`.
    pub fn collect_tracking(&mut self) {
        for g in &mut self.guests {
            take_tracking(g, &mut self.native_tracking);
        }
    }

    fn remove_guest(&mut self, i: usize, reason: &str) -> Guest {
        let mut g = self.guests.remove(i);
        if let Some(m) = g
            .menu
            .as_ref()
            .filter(|m| m.slots == super::menu::Slots::Native)
        {
            self.native_menu_events
                .push(super::menu::NativeMenuEvent::Closed {
                    uuid: g.uuid,
                    window: m.window,
                });
        }
        if g.left_reason.is_empty() {
            g.left_reason = reason.to_owned();
        }
        if std::env::var_os("HORIZON_NET_DEBUG").is_some() {
            log::info!("[lan] {} left: {reason}", g.name);
        }
        if !g.placed {
            return g;
        }
        // `PlayerList.remove`: the entities it tracked stop (`removePairing`).
        g.tracker.clear();
        take_tracking(&mut g, &mut self.native_tracking);
        self.broadcast(&C::PlayerInfoRemove(play::PlayerInfoRemove {
            uuids: vec![g.uuid],
        }));
        self.broadcast(&C::RemoveEntities(play::RemoveEntities {
            ids: vec![g.entity_id],
        }));
        self.system_message(yellow_names(
            "multiplayer.player.left",
            vec![display_name(g.uuid, &g.name)],
        ));
        g
    }

    /// `ServerTickRateManager.updateStepTicks` / `updateStateToClients`:
    /// send clock changes before the block events caused by their ticks.
    pub fn sync_ticking(&mut self, rate: f32, frozen: bool, steps: crate::clock::StepState) {
        if (rate, frozen) != self.ticking {
            self.ticking = (rate, frozen);
            self.broadcast(&ticking_state(rate, frozen));
        }
        // ServerTickRateManager.updateStepTicks: a step started or stopped.
        if steps.revision != self.stepping {
            self.stepping = steps.revision;
            self.broadcast(&C::TickingStep(play::TickingStep {
                steps: steps.requested,
            }));
        }
    }

    /// Time and weather (ServerLevel.tickTime / advanceWeatherCycle broadcasts).
    fn sync_world(&mut self, ctx: &TickCtx) {
        let world = &*ctx.world;
        // MinecraftServer.tickServer: the game time every 20 ticks
        // (forceGameTimeSynchronization, no clocks).
        if self.ticks.is_multiple_of(20) {
            self.broadcast_level(&C::SetTime(play::SetTime {
                game_time: world.game_time,
                clocks: Vec::new(),
            }));
        }
        // ServerClockManager.modifyClock: a day that jumped (`time set`, `time
        // add`) or a new rate updates the overworld clock.
        let (day, rate) = (world.day_time, ctx.clock_rate);
        let step = i64::from(rate.ceil() as i32).max(1);
        if self.last_clock != i64::MIN
            && ((day - self.last_clock).abs() > step || rate != self.last_rate)
        {
            let overworld = self
                .sync
                .id("minecraft:world_clock", "minecraft:overworld")
                .unwrap_or(0);
            self.broadcast_level(&C::SetTime(play::SetTime {
                game_time: world.game_time,
                clocks: vec![play::ClockState {
                    clock: overworld,
                    total_ticks: day,
                    partial_tick: 0.0,
                    rate,
                }],
            }));
        }
        self.last_clock = day;
        self.last_rate = rate;
        self.sync_ticking(ctx.tick_rate, ctx.tick_frozen, ctx.tick_steps);
        let w = &world.weather;
        let (raining, rain, thunder) = self.weather;
        if w.raining != raining {
            self.broadcast_level(&C::GameEvent(play::GameEvent {
                event: if w.raining {
                    game_event::START_RAINING
                } else {
                    game_event::STOP_RAINING
                },
                param: 0.0,
            }));
        }
        if (w.rain_level.1 - rain).abs() > f32::EPSILON {
            self.broadcast_level(&C::GameEvent(play::GameEvent {
                event: game_event::RAIN_LEVEL,
                param: w.rain_level.1,
            }));
        }
        if (w.thunder_level.1 - thunder).abs() > f32::EPSILON {
            self.broadcast_level(&C::GameEvent(play::GameEvent {
                event: game_event::THUNDER_LEVEL,
                param: w.thunder_level.1,
            }));
        }
        self.weather = (w.raining, w.rain_level.1, w.thunder_level.1);
    }

    // ---- chunks --------------------------------------------------------

    /// Every player's chunks here: their views, then the next batch of each
    /// (PlayerChunkSender.sendNextChunks). The batches are chosen first, the
    /// columns in them encoded once each, their new sections on the warm-up
    /// threads too (`ChunkEncoder::warm`), then sent: nothing changes the
    /// level in between, and each player's packets keep their order. The far
    /// ring's chunks the level doesn't hold go as the save's readers made them.
    fn update_chunks(&mut self, ctx: &TickCtx) {
        let world = &*ctx.world;
        if let Some(far) = self.far.as_mut() {
            far.poll();
        }
        let mut batches = Vec::new();
        for i in 0..self.guests.len() {
            if self.here(&self.guests[i]) {
                self.update_view(i);
                if let Some(batch) = self.next_batch(i, world, ctx.streaming) {
                    batches.push((i, batch));
                }
            }
        }
        if batches.is_empty() {
            return;
        }
        // Each column once, however many players it goes to, with the
        // number of batches that take it.
        let mut columns = Columns::default();
        let batches: Vec<_> = batches
            .into_iter()
            .map(|(i, batch)| {
                let (taken, mut bodies): (Vec<_>, Vec<_>) = batch
                    .near
                    .into_iter()
                    .map(|(idx, chunk)| (idx, Body::Level(columns.add(chunk))))
                    .unzip();
                let mut far = Vec::new();
                for (idx, column) in batch.far {
                    far.push((idx, column.is_some()));
                    match column {
                        Some(Column::Level(chunk)) => bodies.push(Body::Level(columns.add(chunk))),
                        Some(Column::Saved(body)) => bodies.push(Body::Saved(body)),
                        None => {}
                    }
                }
                (i, taken, far, bodies)
            })
            .collect();
        self.encoder.warm(&columns.chunks);
        let mut encoded: Vec<Vec<u8>> = columns
            .chunks
            .iter()
            .map(|c| {
                self.encoder.body_with(c, world, &|x, z| {
                    ctx.streaming.and_then(|s| s.proto_sections(x, z))
                })
            })
            .collect();
        let uses = &mut columns.uses;
        for (i, taken, far, batch) in batches {
            let g = &mut self.guests[i];
            let size = batch.len();
            if size > 0 {
                g.batch_quota -= size as f32;
                g.send(&C::ChunkBatchStart(horizon_protocol::packets::Empty));
                g.sender.send_many(batch.into_iter().map(|body| match body {
                    Body::Level(j) => {
                        uses[j] -= 1;
                        if uses[j] == 0 {
                            std::mem::take(&mut encoded[j])
                        } else {
                            encoded[j].clone()
                        }
                    }
                    Body::Saved(body) => body.to_vec(),
                }));
                g.send(&C::ChunkBatchFinished(play::ChunkBatchFinished {
                    size: size as i32,
                }));
                g.unacknowledged += 1;
            }
            for idx in taken.into_iter().rev() {
                let key = g.chunk_queue.remove(idx);
                g.sent_chunks.insert(key);
            }
            // A far chunk the save doesn't have leaves the queue unsent.
            for (idx, sent) in far.into_iter().rev() {
                let key = g.far_queue.remove(idx);
                if sent {
                    g.sent_chunks.insert(key);
                }
            }
        }
    }

    /// ChunkMap.move's view change: the new centre, the chunks that left
    /// the view forgotten and the ones that entered it queued, closest first;
    /// past the view, the far ring's.
    fn update_view(&mut self, i: usize) {
        let (cx, cz) = self.guests[i].chunk();
        let r = self.guests[i].view_distance;
        let reach = self.reach(&self.guests[i]);
        let g = &mut self.guests[i];
        if g.center == Some((cx, cz)) {
            return;
        }
        g.center = Some((cx, cz));
        if g.center_sent.take() != Some((cx, cz)) {
            g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
                x: cx,
                z: cz,
            }));
        }
        // Chunks that left the view.
        let forget: Vec<_> = g
            .sent_chunks
            .iter()
            .copied()
            .filter(|&(x, z)| !tracked(cx, cz, reach, x, z))
            .collect();
        for (x, z) in forget {
            g.sent_chunks.remove(&(x, z));
            g.send(&C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }));
        }
        requeue(&mut g.chunk_queue, &g.sent_chunks, (cx, cz), r);
        if reach > r {
            requeue_ring(&mut g.far_queue, &g.sent_chunks, (cx, cz), r, reach);
        } else {
            g.far_queue.clear();
        }
    }

    /// PlayerChunkSender.sendNextChunks' choice: ready chunks, closest first,
    /// as many as the batch quota the client's acknowledgements pace allows,
    /// with their places in the queue. What the view leaves of the quota goes
    /// to the far ring, as far as its brake lets it reach.
    fn next_batch<'w>(
        &mut self,
        i: usize,
        world: &'w World,
        streaming: Option<&crate::streaming::Streaming>,
    ) -> Option<Batch<'w>> {
        let g = &mut self.guests[i];
        if self.ticks <= g.placed_at + 1 || g.unacknowledged >= g.max_unacknowledged {
            return None;
        }
        g.batch_quota = (g.batch_quota + g.chunks_per_tick).min(g.chunks_per_tick.max(1.0));
        if g.batch_quota < 1.0 || (g.chunk_queue.is_empty() && g.far_queue.is_empty()) {
            return None;
        }
        let budget = g.batch_quota.floor() as usize;
        // ChunkStatus.LIGHT needs its neighbours at INITIALIZE_LIGHT: a
        // column whose light layers would leave one out waits for it.
        let ready = |c: &Chunk| {
            c.lit
                && streaming.is_none_or(|streaming| {
                    (-1..=1).all(|dz| {
                        (-1..=1).all(|dx| {
                            let (x, z) = (c.x + dx, c.z + dz);
                            world.chunks.contains_key(&(x, z))
                                || streaming.proto_sections(x, z).is_some()
                        })
                    })
                })
        };
        let near: Vec<_> = g
            .chunk_queue
            .iter()
            .enumerate()
            .filter_map(|(idx, key)| Some((idx, &**world.chunks.get(key)?)))
            .filter(|(_, c)| ready(c))
            .take(budget)
            .collect();
        let mut far = Vec::new();
        if let (Some(ring), Some((cx, cz))) = (self.far.as_mut(), g.center) {
            let mut left = budget - near.len();
            for (idx, &(x, z)) in g.far_queue.iter().enumerate().take(super::far::LOOKS) {
                if left == 0 {
                    break;
                }
                if !tracked(cx, cz, ring.limit(), x, z) {
                    continue;
                }
                let column = match world.chunks.get(&(x, z)) {
                    Some(c) if ready(c) => Some(Column::Level(c)),
                    Some(_) => continue,
                    None => match ring.saved(x, z) {
                        super::far::Saved::Body(body) => Some(Column::Saved(body)),
                        super::far::Saved::Missing => None,
                        super::far::Saved::Waiting => continue,
                    },
                };
                left -= usize::from(column.is_some());
                far.push((idx, column));
            }
        }
        (!near.is_empty() || !far.is_empty()).then_some(Batch { near, far })
    }

    /// `ChunkMap.resendBiomesForChunks`: each player in the level is sent,
    /// in one packet, the biomes of those of `columns` it has been sent
    /// (`getPlayers(pos, false)`), in the order given.
    pub fn resend_biomes(&mut self, world: &World, columns: &[(i32, i32)]) {
        if let Some(far) = self.far.as_mut() {
            for &(x, z) in columns {
                far.changed(x, z);
            }
        }
        let data: Vec<_> = columns
            .iter()
            .filter_map(|key| Some((*key, self.encoder.biome_data(world.chunks.get(key)?))))
            .collect();
        for g in self.guests.iter().filter(|g| self.here(g)) {
            let chunks: Vec<_> = data
                .iter()
                .filter(|(key, _)| g.sent_chunks.contains(key))
                .map(|(_, d)| d.clone())
                .collect();
            if !chunks.is_empty() {
                g.send(&C::ChunksBiomes(play::ChunksBiomes { chunks }));
            }
        }
    }

    /// `ChunkHolder.broadcastChanges`' light part: the light sections that
    /// changed in a column go to the players for whom it is on the border of
    /// what they track (`ChunkMap.isChunkOnTrackedBorder`); the others light
    /// the blocks they are sent themselves.
    fn broadcast_light_changes(&mut self, world: &mut World) {
        let Some(log) = world.light_log.as_mut() else {
            return;
        };
        if log.is_empty() {
            return;
        }
        let mut changes: Vec<_> = log.drain().collect();
        // One entry per column, so the order is the columns'.
        changes.sort_unstable_by_key(|&(column, _)| column);
        for ((x, z), sections) in changes {
            if let Some(far) = self.far.as_mut() {
                far.changed(x, z);
            }
            let tracked = |g: &Guest, x: i32, z: i32| g.sent_chunks.contains(&(x, z));
            let viewers: Vec<usize> = (0..self.guests.len())
                .filter(|&i| {
                    let g = &self.guests[i];
                    self.here(g)
                        && tracked(g, x, z)
                        && (-1..=1).any(|dz| (-1..=1).any(|dx| !tracked(g, x + dx, z + dz)))
                })
                .collect();
            if viewers.is_empty() {
                continue;
            }
            let Some(chunk) = world.chunks.get(&(x, z)) else {
                continue;
            };
            let body = self.encoder.light_update(chunk, sections);
            for &i in &viewers {
                self.guests[i].sender.send(body.clone());
            }
        }
    }

    fn broadcast_block_changes(&mut self, world: &mut World) {
        // Chunks changed in bulk (`World::edit_runs`) go whole to those who see them: one packet each, where a
        // change of each block would be thousands (the client lights and meshes them once, on its workers).
        let bulk: Vec<(i32, i32)> = if world.bulk_log.is_empty() {
            Vec::new()
        } else {
            let mut keys: Vec<_> = world.bulk_log.drain().collect();
            keys.sort_unstable();
            keys
        };
        if !bulk.is_empty() {
            let recording = self.recording();
            let columns: Vec<&Chunk> = bulk
                .iter()
                .filter_map(|k| world.chunks.get(k).map(|c| &**c))
                .collect();
            self.encoder.warm(&columns);
            for chunk in columns {
                let key = (chunk.x, chunk.z);
                if let Some(far) = self.far.as_mut() {
                    far.changed(key.0, key.1);
                }
                let viewers: Vec<usize> = (0..self.guests.len())
                    .filter(|&i| {
                        self.here(&self.guests[i]) && self.guests[i].sent_chunks.contains(&key)
                    })
                    .collect();
                if viewers.is_empty() {
                    continue;
                }
                let body = self.encoder.body_in(chunk, world);
                for &i in &viewers {
                    self.guests[i].sender.send(body.clone());
                }
                if recording {
                    // The prediction's replay excuses a client near the column: any of its blocks may have
                    // changed, so the one nearest the player stands for them.
                    for &i in &viewers {
                        let g = &mut self.guests[i];
                        let Some(record) = g.anticheat.record.as_deref_mut() else {
                            continue;
                        };
                        let slack = super::prediction::slack(std::array::from_fn(|i| {
                            g.position[i] - g.previous_position[i]
                        }));
                        let column =
                            |c: f64, at: i32| (c.floor() as i32).clamp(at * 16, at * 16 + 15);
                        let at = [
                            column(g.position[0], key.0),
                            g.position[1].floor() as i32,
                            column(g.position[2], key.1),
                        ];
                        if super::prediction::near(g.position, at, slack) {
                            record.changed_near(self.ticks, at);
                        }
                    }
                }
            }
        }
        let Some(log) = world.block_log.as_mut() else {
            return;
        };
        if !bulk.is_empty() {
            // (what changed in them one at a time went with them)
            log.retain(|p| bulk.binary_search(&(p.0 >> 4, p.2 >> 4)).is_err());
        }
        if log.is_empty() {
            return;
        }
        let mut changes = std::mem::take(log);
        changes.sort_unstable();
        changes.dedup();
        // The prediction's replay excuses a client that hadn't seen a change near it.
        let recording = self.recording();
        // Group by section.
        let mut sections: std::collections::BTreeMap<(i32, i32, i32), Vec<(i32, i32, i32)>> =
            Default::default();
        for p in changes {
            sections
                .entry((p.0 >> 4, p.1 >> 4, p.2 >> 4))
                .or_default()
                .push(p);
        }
        for ((sx, sy, sz), positions) in sections {
            if let Some(far) = self.far.as_mut() {
                far.changed(sx, sz);
            }
            let viewers: Vec<usize> = (0..self.guests.len())
                .filter(|&i| {
                    self.here(&self.guests[i]) && self.guests[i].sent_chunks.contains(&(sx, sz))
                })
                .collect();
            if viewers.is_empty() {
                continue;
            }
            let packet = if positions.len() == 1 {
                let (x, y, z) = positions[0];
                C::BlockUpdate(play::BlockUpdate {
                    pos: [x, y, z],
                    state: world.block(x, y, z).0 as i32,
                })
            } else {
                C::SectionBlocksUpdate(play::SectionBlocksUpdate {
                    section: [sx, sy, sz],
                    changes: positions
                        .iter()
                        .map(|&(x, y, z)| {
                            let state = i64::from(world.block(x, y, z).0);
                            state << 12
                                | i64::from(x & 15) << 8
                                | i64::from(z & 15) << 4
                                | i64::from(y & 15)
                        })
                        .collect(),
                })
            };
            let body = packet.encode();
            for &i in &viewers {
                self.guests[i].sender.send(body.clone());
            }
            if recording {
                for &i in &viewers {
                    let g = &mut self.guests[i];
                    let Some(record) = g.anticheat.record.as_deref_mut() else {
                        continue;
                    };
                    let slack = super::prediction::slack(std::array::from_fn(|i| {
                        g.position[i] - g.previous_position[i]
                    }));
                    for &(x, y, z) in &positions {
                        if super::prediction::near(g.position, [x, y, z], slack) {
                            record.changed_near(self.ticks, [x, y, z]);
                        }
                    }
                }
            }
            // Block entities with data the client shows.
            for &(x, y, z) in &positions {
                // A decorated pot is no block entity: its tag is the chunk's saved one.
                let tag = match world.block_entities.get(&(x, y, z)) {
                    Some(entity) => self.encoder.update_tag((x, y, z), entity),
                    None => world
                        .chunks
                        .get(&(x >> 4, z >> 4))
                        .and_then(|c| super::chunks::pot_tag(c, (x, y, z))),
                };
                let Some(tag) = tag else {
                    continue;
                };
                let state = world.block(x, y, z).0;
                let Some(kind) = self.encoder.entity_type(state) else {
                    continue;
                };
                let body = C::BlockEntityData(play::BlockEntityData {
                    pos: [x, y, z],
                    kind,
                    tag,
                })
                .encode();
                for &i in &viewers {
                    self.guests[i].sender.send(body.clone());
                }
            }
        }
    }

    // ---- entities ------------------------------------------------------

    /// `LivingEntity.updateSynchronizedMobEffectParticles`: DATA_EFFECT_PARTICLES
    /// (10) and DATA_EFFECT_AMBIENCE_ID (11) for the visible `effects`, as
    /// (effect, ambient). The entries are left out when there is nothing to
    /// show, which is their default.
    pub fn effect_meta<'a>(
        &self,
        effects: impl IntoIterator<Item = (&'a str, bool)>,
    ) -> Vec<(u8, i32, Value)> {
        let mut particles = Vec::new();
        let mut all_ambient = true;
        for (effect, ambient) in effects {
            let effect = effect.trim_start_matches("minecraft:");
            // MobEffect.particleFactory: the omens have their own particles,
            // the others an entity effect in their colour (faint if ambient).
            let (name, data) = match effect {
                "trial_omen" | "raid_omen" => (effect, None),
                _ => {
                    let Some(def) = crate::brewing::effect(effect) else {
                        continue;
                    };
                    let alpha = if ambient { 38 } else { 255 };
                    let argb = (alpha << 24) | def.color as i32;
                    ("entity_effect", Some(ParticleData::Color(argb)))
                }
            };
            let Some(id) =
                self.registry_id("minecraft:particle_type", &format!("minecraft:{name}"))
            else {
                continue;
            };
            particles.push(Particle { id, data });
            all_ambient &= ambient;
        }
        if particles.is_empty() {
            return Vec::new();
        }
        let mut meta = vec![(10, serializer::PARTICLES, Value::Particles(particles))];
        if all_ambient {
            meta.push((11, serializer::BOOLEAN, Value::Bool(true)));
        }
        meta
    }

    fn player_state(
        &self,
        id: i32,
        uuid: u128,
        position: [f64; 3],
        rot: (f32, f32, f32),
        on_ground: bool,
        flags: (bool, bool, bool),
        body: BodyFlags,
        living_flags: i8,
        bed: Option<[i32; 3]>,
        skin: u8,
        health: f32,
        equipment: Vec<(u8, Option<horizon_protocol::item::Stack>)>,
        attributes: Vec<play::AttributeSnapshot>,
        effects: &[EffectInstance],
    ) -> EntityState {
        let (sneaking, sprinting, sleeping) = flags;
        let shared = shared_flags(sneaking, sprinting, &body);
        // Player.getDesiredPose (and updatePlayerPose's crawl): sleeping,
        // swimming, gliding, Riptide's spin, crouching.
        let pose = if sleeping {
            2
        } else if body.swimming || body.crawling {
            3
        } else if body.gliding {
            1
        } else if body.spin_attack {
            4
        } else if sneaking || body.crouching {
            5
        } else {
            0
        };
        let mut meta = Vec::new();
        if shared != 0 {
            meta.push((0, serializer::BYTE, Value::Byte(shared)));
        }
        if pose != 0 {
            meta.push((6, serializer::POSE, Value::Id(pose)));
        }
        // Always there: the trackers send the entries that changed, and an
        // item no longer in use is a change back to 0.
        meta.push((8, serializer::BYTE, Value::Byte(living_flags)));
        // LivingEntity.SLEEPING_POS: the others draw the player lying in it,
        // a player paired later too.
        if let Some(bed) = bed.filter(|_| sleeping) {
            meta.push((14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(Some(bed))));
        }
        if (health - 1.0).abs() > f32::EPSILON && health != 20.0 {
            meta.push((9, serializer::FLOAT, Value::Float(health)));
        }
        if skin != 0 {
            meta.push((16, serializer::BYTE, Value::Byte(skin as i8)));
        }
        meta.extend(
            self.effect_meta(
                effects
                    .iter()
                    .filter(|e| e.visible)
                    .map(|e| (e.id, e.ambient)),
            ),
        );
        EntityState {
            id,
            uuid,
            kind: "minecraft:player".into(),
            position,
            yaw: rot.0,
            pitch: rot.1,
            head_yaw: rot.2,
            on_ground,
            velocity: [0.0; 3],
            data: 0,
            meta,
            // The effect swirls go back to none.
            defaults: vec![
                (10, serializer::PARTICLES, Value::Particles(Vec::new())),
                (11, serializer::BOOLEAN, Value::Bool(false)),
            ],
            equipment,
            attributes,
            precise: false,
            needs_sync: false,
            leash: None,
            spectator: false,
            watching: false,
        }
    }

    fn equipment_of(
        &mut self,
        main: &Option<ItemStack>,
        off: &Option<ItemStack>,
        armor: [&Option<ItemStack>; 4],
    ) -> Vec<(u8, Option<horizon_protocol::item::Stack>)> {
        // Armor is stored head..feet; the wire slots are feet(2) legs(3) chest(4) head(5).
        let mut out = Vec::new();
        let slots = [
            (0u8, main),
            (1, off),
            (5, armor[0]),
            (4, armor[1]),
            (3, armor[2]),
            (2, armor[3]),
        ];
        for (slot, item) in slots {
            if item.is_some() {
                out.push((slot, self.stack(item)));
            }
        }
        out
    }

    /// All entities as one client sees them: the world's, the host, the other guests.
    fn track_entities(&mut self, ctx: &mut TickCtx) {
        let mut all: Vec<EntityState> =
            Vec::with_capacity(ctx.entities.len() + self.guests.len() + 1);
        if let Some(h) = ctx.host {
            let equipment = {
                let e = &h.equipment;
                self.equipment_of(&e[0], &e[1], [&e[5], &e[4], &e[3], &e[2]])
            };
            let mut state = self.player_state(
                h.entity_id,
                h.uuid,
                h.position,
                (h.yaw, h.pitch, h.head_yaw),
                h.on_ground,
                (h.sneaking, h.sprinting, h.bed.is_some()),
                h.body,
                h.living_flags,
                h.bed,
                0x7F,
                h.health,
                equipment,
                h.attributes.clone(),
                &[],
            );
            state.spectator = h.game_mode == 3;
            stuck_meta(&mut state, h.stuck);
            shoulder_meta(&mut state, h.shoulders);
            all.push(state);
        }
        let placed: Vec<usize> = (0..self.guests.len())
            .filter(|&i| self.here(&self.guests[i]))
            .collect();
        let guest_states: Vec<_> = placed
            .into_iter()
            .map(|i| {
                let (main, off, armor) = {
                    let g = &self.guests[i];
                    (
                        g.inventory.get(g.selected).cloned().flatten(),
                        g.offhand.clone(),
                        g.armor.clone(),
                    )
                };
                let equipment =
                    self.equipment_of(&main, &off, [&armor[0], &armor[1], &armor[2], &armor[3]]);
                let g = &self.guests[i];
                let mut state = self.player_state(
                    g.entity_id,
                    g.uuid,
                    g.position,
                    (g.yaw, g.pitch, g.head_yaw),
                    g.on_ground,
                    (g.sneaking, g.sprinting, g.sleeping),
                    guest_body(g),
                    g.living_flags,
                    g.bed,
                    g.info.model_customisation,
                    g.health,
                    equipment,
                    g.motion
                        .as_ref()
                        .map(|m| super::attributes::syncable(&m.attributes))
                        .unwrap_or_default(),
                    &g.effects.list,
                );
                state.spectator = g.spectator();
                state.watching = g.watching;
                stuck_meta(&mut state, g.stuck);
                shoulder_meta(&mut state, g.shoulders);
                state
            })
            .collect();
        all.extend(guest_states);
        let entities: Vec<&EntityState> = all.iter().chain(ctx.entities).collect();
        // ChunkMap.tick: the level's entities, once for all its players.
        let mut level = self
            .entities
            .remove(&self.config.dimension)
            .unwrap_or_default();
        let mut outs = std::mem::take(&mut self.entity_out);
        let dimension = &self.config.dimension;
        let here = |g: &Guest| g.placed && g.dimension == *dimension;
        outs.resize_with(self.guests.iter().filter(|g| here(g)).count(), Vec::new);
        // The prediction's replay excuses a client that may still have an entity that left it.
        let recording = self.recording();
        let mut gone = std::mem::take(&mut self.entity_gone);
        gone.resize_with(outs.len(), Vec::new);
        let mut viewers: Vec<Viewer> = self
            .guests
            .iter_mut()
            .filter(|g| here(g))
            .zip(outs.iter_mut().zip(gone.iter_mut()))
            .map(|(g, (out, gone))| Viewer {
                entity_id: g.entity_id,
                spectator: g.spectator(),
                moved: std::mem::take(&mut g.moved),
                position: g.position,
                view_distance: g.view_distance,
                chunks: &g.sent_chunks,
                tracker: &mut g.tracker,
                out,
                gone: recording.then_some(gone),
            })
            .collect();
        level.range_percent = Some(self.entity_range_percent);
        let (types, sync) = (&mut self.entity_types, &self.sync);
        level.tick(&entities, &self.riding, &mut viewers, |kind| {
            entity_type(types, sync, kind)
        });
        drop(viewers);
        for (g, out) in self.guests.iter().filter(|g| here(g)).zip(outs.iter_mut()) {
            g.sender.send_many(out.drain(..));
        }
        let ticks = self.ticks;
        for (g, gone) in self
            .guests
            .iter_mut()
            .filter(|g| here(g))
            .zip(gone.iter_mut())
        {
            let slack = super::prediction::slack(std::array::from_fn(|i| {
                g.position[i] - g.previous_position[i]
            }));
            for at in gone.drain(..) {
                let block = at.map(|v| v.floor() as i32);
                if let Some(record) = g.anticheat.record.as_deref_mut()
                    && super::prediction::near(g.position, block, slack)
                {
                    record.entity_gone(ticks);
                }
            }
        }
        self.entity_gone = gone;
        self.entity_out = outs;
        self.entities.insert(self.config.dimension.clone(), level);
    }

    /// ServerPlayer.stopSleepInBed: the wake-up animation goes to the player
    /// and those around, the pose is standing again and the sleep counter
    /// starts the wake-up fade (100).
    pub fn wake(&mut self, i: usize) {
        let g = &mut self.guests[i];
        let head = g.bed.take();
        if head.is_none() && !g.sleeping {
            return;
        }
        if let Some(head) = head {
            g.woke_from = Some(head);
        }
        g.sleeping = false;
        g.sleep_timer = 100;
        if g.sent_bed.take().is_some() {
            g.send(&C::SetEntityData(play::SetEntityData {
                id: g.entity_id,
                values: vec![
                    (6, serializer::POSE, Value::Id(0)),
                    (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None)),
                ],
            }));
        }
        let (id, at) = (g.entity_id, g.position);
        // ClientboundAnimatePacket.WAKE_UP.
        self.broadcast_near(
            at,
            128.0,
            &C::Animate(play::Animate { id, action: 0 }),
            None,
        );
    }

    // ---- per-player state ------------------------------------------------

    /// ServerPlayer.doTick: health, food, experience and the inventory menu.
    fn sync_player_state(&mut self, i: usize) {
        // LivingEntity.startSleeping / stopSleeping: the pose and the bed go
        // to the player too (its client lies down in the bed from them).
        let g = &mut self.guests[i];
        if g.bed != g.sent_bed {
            g.sent_bed = g.bed;
            let pose = if g.bed.is_some() { 2 } else { 0 };
            g.send(&C::SetEntityData(play::SetEntityData {
                id: g.entity_id,
                values: vec![
                    (6, serializer::POSE, Value::Id(pose)),
                    (14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(g.bed)),
                ],
            }));
        }
        // ServerEntity.sendDirtyEntityData on the player's first tick: the
        // health its creation set is still dirty, and goes to the player too.
        let first_tick = self.guests[i].sent_health.is_none();
        if first_tick {
            let g = &self.guests[i];
            g.send(&C::SetEntityData(play::SetEntityData {
                id: g.entity_id,
                values: vec![(9, serializer::FLOAT, Value::Float(g.health))],
            }));
            // ServerPlayer.tick's CriteriaTriggers.TICK, then flushDirty. A
            // player who has been here has had the tick's criteria already.
            if !g.advancements_loaded {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                let sync = self.sync.clone();
                let g = &mut self.guests[i];
                let first = sync.advancements.first_tick(
                    &sync,
                    now,
                    &mut g.advancements,
                    &mut g.recipe_book,
                );
                if let Some(add) = first.recipe_book_add {
                    g.send(&raw(ids::RECIPE_BOOK_ADD, add));
                }
                g.send(&raw(ids::UPDATE_ADVANCEMENTS, first.update_advancements));
                g.advancements_visible = sync.advancements.visibility(&g.advancements);
                g.advancements_changed.clear();
            } else {
                self.flush_advancements(i, true);
            }
        }
        // ServerEntity.sendDirtyEntityData: the player's changed attribute
        // instances go to the player too (its trackers get them from
        // track_entities). On the first tick vanilla's ran before
        // ServerPlayer.tick, whose updatePlayerAttributes (and doTick's
        // removeFrost) created the instances the guest's tick just made: they
        // go out on the next tick, after the first chunk batch (#5560). This
        // server's first batch waits for the player's chunks to be ready, so
        // they wait for it too: a slow load must not put them before it.
        let g = &mut self.guests[i];
        let after_first_batch = g.unacknowledged > 0 || g.max_unacknowledged > 1;
        if let Some(m) = g
            .motion
            .as_mut()
            .filter(|_| !first_tick && after_first_batch)
        {
            let values = super::attributes::take_dirty(&mut m.attributes);
            if !values.is_empty() {
                g.send(&C::UpdateAttributes(play::UpdateAttributes {
                    id: g.entity_id,
                    values,
                }));
            }
        }
        // ServerEntity.sendDirtyEntityData: the air supply and the absorption
        // amount the server's body changed go to the player, whose HUD draws
        // them (bubbles, golden hearts).
        let g = &mut self.guests[i];
        if let Some(m) = g.motion.as_ref() {
            let now = (m.air, m.health.absorption());
            let mut values = Vec::new();
            if now.0 != g.sent_air.0 {
                values.push((1, serializer::INT, Value::Int(now.0)));
            }
            if now.1.to_bits() != g.sent_air.1.to_bits() {
                values.push((17, serializer::FLOAT, Value::Float(now.1)));
            }
            if !values.is_empty() {
                g.sent_air = now;
                g.send(&C::SetEntityData(play::SetEntityData {
                    id: g.entity_id,
                    values,
                }));
            }
        }
        // ServerEntity.sendDirtyEntityData: the shared flags go to the player
        // too, so the sprint a hit stops (Player.causeExtraKnockback) stops
        // on its client.
        let g = &mut self.guests[i];
        let shared = shared_flags(g.sneaking, g.sprinting, &guest_body(g));
        if shared != g.sent_shared {
            g.sent_shared = shared;
            g.send(&C::SetEntityData(play::SetEntityData {
                id: g.entity_id,
                values: vec![(0, serializer::BYTE, Value::Byte(shared))],
            }));
        }
        // Sent when the health or the food level changes, or the saturation
        // reaches or leaves zero; a dead player's health of 0 too.
        let g = &mut self.guests[i];
        let health = (g.health, g.food, g.saturation == 0.0);
        if g.sent_health != Some(health) {
            g.send(&C::SetHealth(play::SetHealth {
                health: g.health,
                food: g.food,
                saturation: g.saturation,
            }));
            g.sent_health = Some(health);
        }
        let xp = {
            let g = &self.guests[i];
            (g.xp_progress, g.xp_level, g.xp_total)
        };
        if self.guests[i].sent_xp != Some(xp) {
            let g = &mut self.guests[i];
            g.send(&C::SetExperience(play::SetExperience {
                progress: xp.0,
                level: xp.1,
                total: xp.2,
            }));
            g.sent_xp = Some(xp);
        }
        // AbstractContainerMenu.broadcastChanges for the inventory menu.
        let mut changes = Vec::new();
        for slot in 0..46 {
            let now = self.guests[i].menu_slot(slot);
            if now != self.guests[i].menu_sent[slot] {
                changes.push((slot, now));
            }
        }
        for (slot, now) in changes {
            let net = self.stack(&now);
            let g = &mut self.guests[i];
            g.menu_state = (g.menu_state + 1) & 0x7FFF;
            g.send(&C::ContainerSetSlot(play::ContainerSetSlot {
                container: 0,
                state: g.menu_state,
                slot: slot as i16,
                item: net,
            }));
            g.menu_sent[slot] = now;
        }
    }

    /// `ServerRecipeBook.sendInitialRecipeBook`: each book's open and
    /// filtering flags, then every known recipe.
    fn send_initial_recipe_book(&self, i: usize) {
        let g = &self.guests[i];
        let book = &g.recipe_book;
        let settings = (0..4)
            .flat_map(|b| [u8::from(book.open[b]), u8::from(book.filtering[b])])
            .collect();
        g.send(&raw(ids::RECIPE_BOOK_SETTINGS, settings));
        g.send(&raw(
            ids::RECIPE_BOOK_ADD,
            self.sync.advancements.initial_recipe_book(book, &self.sync),
        ));
    }

    /// `PlayerList.reloadResources` (`/reload`): every player's advancements
    /// reload against the new data, then everyone gets the tags, and each
    /// player the recipes and their recipe book again.
    pub fn reload_resources(&mut self, sync: Arc<SyncData>) {
        self.lan.shared.set_sync(sync.clone());
        self.sync = sync.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        for g in &mut self.guests {
            // PlayerAdvancements.reload: isFirstPacket again, the progress
            // read against the new tree; a player whose advancements were
            // never loaded does that on their first tick.
            if !g.advancements_loaded {
                continue;
            }
            g.advancements_visible.clear();
            g.advancements_changed.clear();
            if let Some(add) =
                sync.advancements
                    .reload(&sync, now, &mut g.advancements, &mut g.recipe_book)
            {
                g.send(&raw(ids::RECIPE_BOOK_ADD, add));
            }
        }
        // broadcastAll(ClientboundUpdateTagsPacket).
        self.broadcast(&C::UpdateTags(sync.tags.clone()));
        // For each player: ClientboundUpdateRecipesPacket, then
        // sendInitialRecipeBook.
        for i in 0..self.guests.len() {
            if !self.guests[i].placed {
                continue;
            }
            self.guests[i].send(&raw(ids::UPDATE_RECIPES, sync.recipes.clone()));
            if self.guests[i].brand == "horizon" {
                send_recipe_content(&self.guests[i]);
            }
            self.send_initial_recipe_book(i);
        }
    }

    /// `PlayerAdvancements.flushDirty` and the recipes the player unlocked
    /// since (`ServerRecipeBook.addRecipes`), once the player's first tick
    /// has sent their advancements.
    pub fn flush_progress(&mut self, conn: u32) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn)
            && self.guests[i].sent_health.is_some()
        {
            self.flush_advancements(i, true);
            // ServerPlayer.doTick: sendPostEffects when postEffectsDirty.
            if self.guests[i].post_effects_dirty {
                self.send_post_effects(i);
            }
        }
    }
    fn flush_advancements(&mut self, i: usize, show: bool) {
        let sync = self.sync.clone();
        let g = &mut self.guests[i];
        let mut unlocked = std::mem::take(&mut g.recipes_unlocked);
        // A recipe given and taken before this flush must not reappear.
        unlocked.retain(|id| g.recipe_book.knows(id));
        if let Some(add) = sync.advancements.recipes_added(&unlocked, &sync) {
            g.send(&raw(ids::RECIPE_BOOK_ADD, add));
        }
        let changed = std::mem::take(&mut g.advancements_changed);
        if let Some(update) = sync.advancements.update(
            &sync,
            &g.advancements,
            &mut g.advancements_visible,
            &changed,
            show,
        ) {
            g.send(&raw(ids::UPDATE_ADVANCEMENTS, update));
        }
    }

    /// ClientboundAwardStatsPacket (`ServerStatsCounter.sendStats`): each
    /// statistic as its type and value by network id (`Stat.STREAM_CODEC`),
    /// then its count. Statistics the registries don't know are left out.
    pub fn send_stats(&self, conn: u32, stats: &[(String, String, i32)]) {
        let known: Vec<(i32, i32, i32)> = stats
            .iter()
            .filter_map(|(ty, key, count)| {
                let values = match ty.as_str() {
                    "minecraft:mined" => "minecraft:block",
                    "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
                    "minecraft:custom" => "minecraft:custom_stat",
                    _ => "minecraft:item",
                };
                Some((
                    self.sync.id("minecraft:stat_type", ty)?,
                    self.sync.id(values, key)?,
                    *count,
                ))
            })
            .collect();
        let mut w = horizon_protocol::buf::Writer::new();
        w.varint(known.len() as i32);
        for (ty, value, count) in known {
            w.varint(ty);
            w.varint(value);
            w.varint(count);
        }
        self.send_to(conn, &raw(ids::AWARD_STATS, w.data));
    }

    // ---- incoming ------------------------------------------------------

    /// The squared movement past 1.0E-5F (handlePlayerKnownMovement).
    fn moved_far(&self, i: usize, to: [f64; 3]) -> bool {
        let from = self.guests[i].position;
        let d: f64 = (0..3).map(|k| (to[k] - from[k]).powi(2)).sum();
        d > f64::from(1.0E-5_f32)
    }

    /// A move still queued for guest `i`, which a packet the game handles
    /// after this pass must not overtake.
    fn move_queued(&self, i: usize) -> bool {
        self.guests[i].inbox.iter().any(|(p, _)| {
            matches!(
                p,
                S::MovePlayerPos(_)
                    | S::MovePlayerPosRot(_)
                    | S::MovePlayerRot(_)
                    | S::MovePlayerStatus(_)
            )
        })
    }

    /// handlePlayerCommand's START_SPRINTING and STOP_SPRINTING: `setSprinting`, whose movement
    /// speed modifier the attributes then sync to the client (which takes the modifiers as sent:
    /// one without it would slow a sprinting client to a walk).
    fn set_sprinting(&mut self, i: usize, sprinting: bool) {
        let g = &mut self.guests[i];
        g.sprinting = sprinting;
        g.anticheat.sprinting = sprinting;
        if let Some(m) = g.motion.as_mut() {
            let _ = m.set_sprint_modifier(sprinting);
        }
    }

    fn process_inbox(&mut self, i: usize, ctx: &mut TickCtx, events: &mut Vec<HostEvent>) {
        // ServerGamePacketListenerImpl.tickPlayer ran between the last
        // tick's packets and these: resetPosition, and the moves it counted.
        let g = &mut self.guests[i];
        g.first_good = g.position;
        g.known_moves = g.received_moves;
        // ServerPlayer.tick: tickClientLoadTimeout.
        g.load_timeout = g.load_timeout.saturating_sub(1);
        let mut budget = PACKETS_PER_TICK;
        while budget > 0 {
            let Some((packet, bytes)) = self.guests[i].inbox.pop_front() else {
                break;
            };
            self.guests[i].inbox_bytes -= bytes;
            budget -= 1;
            let conn = self.guests[i].conn;
            // ServerPlayer.resetLastActionTime: every handler of a player's
            // own action resets it, movement only when it moved
            // (handlePlayerKnownMovement), PlayerInput once loaded.
            let acted = match &packet {
                S::AcceptTeleportation(_)
                | S::PlayerAbilities(_)
                | S::MovePlayerStatus(_)
                | S::MovePlayerRot(_)
                | S::PlayerLoaded(_)
                | S::ChunkBatchReceived(_)
                | S::ClientInformation(_)
                | S::RecipeBookChangeSettings(_)
                | S::ClientTickEnd(_)
                | S::KeepAlive(_)
                | S::Pong(_) => false,
                S::MovePlayerPos(p) => self.moved_far(i, [p.x, p.y, p.z]),
                S::MovePlayerPosRot(p) => self.moved_far(i, [p.x, p.y, p.z]),
                S::PlayerInput(_) => self.guests[i].client_loaded(),
                _ => true,
            };
            if acted {
                self.guests[i].last_action_tick = self.ticks;
            }
            match packet {
                // hasClientLoaded: these handlers ignore the player's actions until
                // the client says the level around it is loaded (or 60 ticks pass),
                // and while it is dead.
                S::PlayerAction(_)
                | S::UseItemOn(_)
                | S::UseItem(_)
                | S::Attack(_)
                | S::Interact(_)
                | S::SpectatorAction(_)
                | S::PlayerCommand(_)
                    if !self.guests[i].client_loaded() => {}
                // handleAcceptTeleportPacket.
                S::AcceptTeleportation(p) => {
                    let recording = self.recording();
                    let g = &mut self.guests[i];
                    if let Some((id, at)) = g.awaiting_teleport
                        && id == p.id
                    {
                        if p.x.is_nan()
                            || p.y.is_nan()
                            || p.z.is_nan()
                            || !p.yaw.is_finite()
                            || !p.pitch.is_finite()
                        {
                            self.kick(conn, invalid_movement());
                            continue;
                        }
                        g.position = at;
                        g.awaiting_teleport = None;
                        // The prediction's replay starts again where the client now is.
                        if recording {
                            let kept = g.awaiting_keeps_motion;
                            g.anticheat
                                .record
                                .get_or_insert_default()
                                .teleported(at, kept);
                        }
                        let rot = (p.yaw, p.pitch);
                        self.position_change(i, [p.x, p.y, p.z], rot, 0, ctx, events);
                    }
                }
                // handlePlayerAbilities: the client flies only if it may.
                S::PlayerAbilities(p) => {
                    let g = &mut self.guests[i];
                    let flying = p.flags & 2 != 0 && (g.creative() || g.spectator());
                    // The change is the game's to announce (the mods may refuse it).
                    if flying != g.flying {
                        g.flying = flying;
                        events.push(HostEvent::Flight { conn, flying });
                    }
                }
                S::MovePlayerPos(p) => {
                    self.move_player(i, Some([p.x, p.y, p.z]), None, p.flags, ctx, events);
                }
                S::MovePlayerPosRot(p) => {
                    let (pos, rot) = ([p.x, p.y, p.z], (p.yaw, p.pitch));
                    self.move_player(i, Some(pos), Some(rot), p.flags, ctx, events);
                }
                S::MovePlayerRot(p) => {
                    self.move_player(i, None, Some((p.yaw, p.pitch)), p.flags, ctx, events);
                }
                S::MovePlayerStatus(p) => {
                    self.move_player(i, None, None, p.flags, ctx, events);
                }
                // handleAcceptPlayerLoad: markClientLoaded.
                S::PlayerLoaded(_) => {
                    let g = &mut self.guests[i];
                    g.loaded = true;
                    g.loading = false;
                    g.load_timeout = 0;
                }
                // PlayerChunkSender.onChunkBatchReceivedByClient.
                S::ChunkBatchReceived(p) => {
                    let g = &mut self.guests[i];
                    g.unacknowledged = g.unacknowledged.saturating_sub(1);
                    g.chunks_per_tick = if p.chunks_per_tick.is_nan() {
                        0.01
                    } else {
                        p.chunks_per_tick.clamp(0.01, 64.0)
                    };
                    if g.unacknowledged == 0 {
                        g.batch_quota = 1.0;
                    }
                    g.max_unacknowledged = 10;
                }
                S::Pong(p) => self.anticheat_pong(i, p.id),
                // handleAttack is the game's too: a move queued behind an attack would move
                // the player (or land them on the ground, losing a falling attack's critical
                // hit) before the game checks their reach and state.
                S::Attack(p) => {
                    if !self.anticheat_attack(i, p.id, events) {
                        events.push(HostEvent::Packet(conn, Box::new(S::Attack(p))));
                        if self.move_queued(i) {
                            break;
                        }
                    }
                }
                S::Punch(p) => {
                    self.anticheat_swing(i);
                    events.push(HostEvent::Packet(conn, Box::new(S::Punch(p))));
                }
                S::ClientTickEnd(_) => {
                    // handleClientTickEnd: no move this tick, no known movement.
                    let g = &mut self.guests[i];
                    if !g.position_this_tick {
                        g.known_movement = [0.0; 3];
                    }
                    g.position_this_tick = false;
                    self.anticheat_tick_end(i, events);
                }
                // ServerGamePacketListenerImpl.handleRecipeBookChangeSettingsPacket.
                S::RecipeBookChangeSettings(p) => {
                    if let Some(book) = crate::recipe_book::BookType::ALL.get(p.book_type as usize)
                    {
                        let settings = &mut self.guests[i].recipe_book;
                        settings.open[book.index()] = p.open;
                        settings.filtering[book.index()] = p.filtering;
                    }
                }
                S::ClientInformation(info) => {
                    let g = &mut self.guests[i];
                    // PlayerModelPart.HAT (bit 6): the tab list shows the hat layer.
                    let was_hat = g.info.model_customisation & 0x40 != 0;
                    g.view_distance =
                        i32::from(info.view_distance).clamp(2, self.config.view_distance);
                    g.info = info;
                    g.center = None;
                    let hat = g.info.model_customisation & 0x40 != 0;
                    let uuid = g.uuid;
                    // ServerGamePacketListenerImpl.handleClientInformation: UPDATE_HAT to everyone.
                    if hat != was_hat {
                        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                            actions: 128,
                            entries: vec![play::PlayerInfo {
                                uuid,
                                show_hat: Some(hat),
                                ..Default::default()
                            }],
                        }));
                    }
                }
                S::PlayerInput(p) => {
                    let g = &mut self.guests[i];
                    g.input = p.flags;
                    if g.client_loaded() {
                        g.sneaking = p.flags & 32 != 0;
                    }
                    // handlePlayerInput only records keys and updates crouching.
                    // Sprinting persists after the sprint key is released (or
                    // starts with a double tap); PlayerCommand changes it.
                }
                // OPEN_INVENTORY (the ridden mount's inventory) and
                // START_FALL_FLYING (checked against the guest's chest slot
                // and effects) are the game's.
                S::PlayerCommand(p) if matches!(p.action, 5 | 6) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::PlayerCommand(p))));
                }
                S::PlayerCommand(p) => match p.action {
                    1 | 2 => self.set_sprinting(i, p.action == 1),
                    // STOP_SLEEPING.
                    0 => self.wake(i),
                    _ => {}
                },
                S::Chat(c) => self.handle_chat(i, c, events),
                S::ChatAck(p) => self.chat_ack(i, p.offset, events),
                S::ChatSessionUpdate(p) => self.chat_session_update(i, p, events),
                // handlePlayerAction (a dig starting, stopping or finishing) is the
                // game's, run after this pass. A move queued behind it is checked
                // against the block the player just broke and falls through, so it
                // would be sent back; it waits for the next tick, when the block is
                // gone, as vanilla handles the packets in order. So does everything
                // queued before that move: the arm swing (Punch) comes between the dig
                // and the move.
                S::PlayerAction(p) if p.action <= 3 => {
                    events.push(HostEvent::Packet(conn, Box::new(S::PlayerAction(p))));
                    if self.move_queued(i) {
                        break;
                    }
                }
                // handleUseItem uses the player's position and known movement now:
                // a later move must not overtake a throw before the game runs it.
                S::UseItem(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::UseItem(p))));
                    if self.move_queued(i) {
                        break;
                    }
                }
                // handleUseItemOn is the game's too, and BlockItem.canPlace checks
                // the player's box where they clicked from (Level.isUnobstructed), as
                // their client's prediction did. After the move behind it, a player
                // jumping up a pillar was already clear of the cell their client had
                // refused: the block went down without the place sound that only the
                // placer's client plays (BlockItem.place's playSound(player, ...)).
                S::UseItemOn(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::UseItemOn(p))));
                    if self.move_queued(i) {
                        break;
                    }
                }
                // handleInteract is the game's too, like handleAttack above: a move queued
                // behind it waits for the next tick.
                S::Interact(p) => {
                    events.push(HostEvent::Packet(conn, Box::new(S::Interact(p))));
                    if self.move_queued(i) {
                        break;
                    }
                }
                // ContainerInput's codec (ByIdMap, OutOfBoundsStrategy.ZERO) reads an
                // unknown click type as PICKUP.
                S::ContainerClick(mut p) => {
                    if !(0..=6).contains(&p.input) {
                        p.input = 0;
                    }
                    events.push(HostEvent::Packet(conn, Box::new(S::ContainerClick(p))));
                }
                // ServerboundSelectBundleItemPacket's constructor: no index below -1.
                S::SelectBundleItem(p) if p.index < -1 => {
                    let reason = format!("Invalid selectedItemIndex: {}", p.index);
                    self.kick(conn, literal(&reason));
                }
                other => events.push(HostEvent::Packet(conn, Box::new(other))),
            }
        }
    }

    /// ServerGamePacketListenerImpl.handleMovePlayer.
    fn move_player(
        &mut self,
        i: usize,
        pos: Option<[f64; 3]>,
        rot: Option<(f32, f32)>,
        flags: u8,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) {
        let conn = self.guests[i].conn;
        // containsInvalidValues, on the packet's values (0 where it has none).
        let (yaw, pitch) = rot.unwrap_or_default();
        if pos.is_some_and(|p| p.iter().any(|v| v.is_nan()))
            || !yaw.is_finite()
            || !pitch.is_finite()
        {
            self.kick(conn, invalid_movement());
            return;
        }
        if self.anticheat.is_some() {
            self.anticheat_moved(i, pos, rot, flags);
        }
        let g = &mut self.guests[i];
        if pos.is_some() {
            if g.position_this_tick {
                self.kick(conn, invalid_movement());
                return;
            }
            g.position_this_tick = true;
        }
        // handleMovePlayer: nothing moves before the client has loaded.
        if g.landing || !g.client_loaded() {
            return;
        }
        // updateAwaitingTeleport: moves wait for the client to accept the
        // teleport, which is sent again after 20 ticks without an answer.
        if let Some((_, at)) = g.awaiting_teleport {
            if self.ticks - g.teleport_tick > 20 {
                self.resend_teleport(i, Some(at));
            }
            return;
        }
        let to = pos.unwrap_or(g.position);
        let rot = rot.unwrap_or((g.yaw, g.pitch));
        self.position_change(i, to, rot, flags, ctx, events);
    }

    /// ServerGamePacketListenerImpl.handlePlayerPositionChange.
    fn position_change(
        &mut self,
        i: usize,
        to: [f64; 3],
        (yaw, pitch): (f32, f32),
        flags: u8,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) {
        let (frozen, (check, elytra_check)) = (self.ticking.1, self.movement_checks);
        let g = &mut self.guests[i];
        let owner = self.singleplayer_owner == Some(g.uuid);
        let to = [
            to[0].clamp(-3.0E7, 3.0E7),
            to[1].clamp(-2.0E7, 2.0E7),
            to[2].clamp(-3.0E7, 3.0E7),
        ];
        let (yaw, pitch) = (wrap_degrees(yaw), wrap_degrees(pitch));
        // A passenger only turns: its vehicle carries it (absSnapTo where it is).
        if g.vehicle.is_some() {
            (g.yaw, g.pitch) = (yaw, pitch);
            return;
        }
        let d = [0, 1, 2].map(|k| to[k] - g.first_good[k]);
        let moved = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if g.sleeping {
            if moved > 1.0 {
                (g.yaw, g.pitch) = (yaw, pitch);
                self.resend_teleport(i, None);
            }
            return;
        }
        // TickRateManager.runsNormally: not frozen (a frozen game's steps
        // are not counted either; Horizon's hub does not see them).
        if !frozen {
            g.received_moves += 1;
            let mut packets = g.received_moves - g.known_moves;
            if packets > 5 {
                packets = 1;
            }
            // shouldCheckPlayerMovement (never the single-player owner).
            if !owner && check && (!g.fall_flying || elytra_check) {
                let limit = if g.fall_flying { 300.0 } else { 100.0 };
                // `movedDist - expectedDist` (getDeltaMovement): the velocity
                // the server last simulated for the guest is expected
                // movement, not a violation — a landing glide keeps its speed.
                let expected = g.motion.as_ref().map_or(0.0, |m| {
                    let v = m.velocity;
                    v.x * v.x + v.y * v.y + v.z * v.z
                });
                if moved - expected > limit * f64::from(packets) {
                    events.push(HostEvent::Warn(format!(
                        "{} moved too quickly! {},{},{}",
                        g.name,
                        crate::java_text::double_to_string(d[0]),
                        crate::java_text::double_to_string(d[1]),
                        crate::java_text::double_to_string(d[2])
                    )));
                    self.resend_teleport(i, None);
                    return;
                }
            }
        }
        let start = g.position;
        let (wrongly, refused, stands) = self.collide(i, to, ctx);
        if wrongly {
            let line = format!("{} moved wrongly!", self.guests[i].name);
            events.push(HostEvent::Warn(line));
        }
        if refused {
            let g = &mut self.guests[i];
            (g.yaw, g.pitch) = (yaw, pitch);
            self.resend_teleport(i, Some(start));
            return;
        }
        // The anticheat's `ground`: a claim of the ground with nothing to
        // stand on is dropped (no-fall, walking on water).
        let (mut flags, mut corrected) = (flags, false);
        if flags & 1 != 0 && self.anticheat.is_some() {
            match self.anticheat_ground(i, to, stands, ctx, events) {
                Some(true) => {
                    let g = &mut self.guests[i];
                    (g.yaw, g.pitch) = (yaw, pitch);
                    return;
                }
                Some(false) => (flags, corrected) = (flags & !1, true),
                None => {}
            }
        }
        // clientIsFloating, from the move that is accepted: no upward-or-level
        // step onto something, nothing but air around the body, and nothing
        // that lets the player hover (the server's allow-flight, mayfly,
        // levitation, gliding, riptide). A move whose ground claim the anticheat dropped never counts:
        // a false claim ends in the anticheat's own kick, logged and told to the operators, or not at all.
        let floating = !corrected && ctx.collision.is_some() && {
            let g = &self.guests[i];
            to[1] - g.position[1] >= -0.03125
                && !stands
                && !g.spectator()
                && !self.config.allow_flight
                && !g.creative()
                && g.effects.get("levitation").is_none()
                && !g.fall_flying
                && !g
                    .motion
                    .as_ref()
                    .is_some_and(|m| m.auto_spin_attack_ticks > 0)
                && self.no_blocks_around(i, to, ctx.world)
        };
        let g = &mut self.guests[i];
        g.client_is_floating = floating;
        g.previous_position = g.position;
        g.position = to;
        // ServerChunkCache.move: its pairs are checked again.
        g.moved = true;
        let moved = [0, 1, 2].map(|k| g.position[k] - g.previous_position[k]);
        if g.moves.is_empty() {
            g.moves_from_ground = g.on_ground;
        }
        g.moves.push((moved, flags & 1 != 0));
        g.known_movement = moved;
        g.yaw = yaw;
        g.pitch = pitch.clamp(-90.0, 90.0);
        g.head_yaw = g.yaw;
        g.on_ground = flags & 1 != 0;
        g.ticks = g.ticks.max(1);
    }

    /// `ServerGamePacketListenerImpl.noBlocksAround`: every block of the
    /// player's box grown by 0.0625 and stretched 0.55 down is air. Vanilla
    /// loads a chunk it reads; a chunk not loaded here yet (a player's first
    /// move after joining, before the spawn chunks are generated) is not
    /// known to be air, so it never makes the player float.
    fn no_blocks_around(&self, i: usize, at: [f64; 3], world: &World) -> bool {
        let d = self.guests[i].dimensions();
        let half = f64::from(d.width) / 2.0 + 0.0625;
        let lo = [at[0] - half, at[1] - 0.0625 - 0.55, at[2] - half];
        let hi = [
            at[0] + half,
            at[1] + f64::from(d.height) + 0.0625,
            at[2] + half,
        ];
        no_blocks_in(world, lo, hi)
    }

    /// `ServerGamePacketListenerImpl.tickPlayer`'s flying check: a player the
    /// last move left hovering for more than `getMaximumFlyingTicks` ticks is
    /// kicked.
    fn tick_floating(&mut self, i: usize, events: &mut Vec<HostEvent>) {
        let g = &mut self.guests[i];
        if g.client_is_floating && !g.sleeping && g.vehicle.is_none() && !g.dead {
            g.above_ground_ticks += 1;
            if g.above_ground_ticks > maximum_flying_ticks(PLAYER_GRAVITY) {
                let line = format!("{} was kicked for floating too long!", g.name);
                let conn = g.conn;
                events.push(HostEvent::Warn(line));
                self.kick(conn, translatable("multiplayer.disconnect.flying", &[]));
            }
        } else {
            g.client_is_floating = false;
            g.above_ground_ticks = 0;
        }
        // The vehicle the player drives, by its own gravity: a boat may hover 160 ticks, a horse 80.
        let g = &mut self.guests[i];
        if g.vehicle_floating && g.vehicle.is_some() {
            g.above_ground_vehicle_ticks += 1;
            if g.above_ground_vehicle_ticks > maximum_flying_ticks(g.vehicle_gravity) {
                let line = format!("{} was kicked for floating a vehicle too long!", g.name);
                let conn = g.conn;
                events.push(HostEvent::Warn(line));
                self.kick(conn, translatable("multiplayer.disconnect.flying", &[]));
            }
        } else {
            g.vehicle_floating = false;
            g.above_ground_vehicle_ticks = 0;
        }
    }

    /// The collision part of handlePlayerPositionChange: whether the move
    /// counts as moved wrongly (logged), and whether it is refused (the
    /// player is sent back where they were).
    fn collide(&self, i: usize, to: [f64; 3], ctx: &TickCtx) -> (bool, bool, bool) {
        let g = &self.guests[i];
        // noPhysics (spectators) and sleepers move freely.
        let Some(binding) = ctx.collision else {
            return (false, false, false);
        };
        if g.spectator() || g.sleeping {
            return (false, false, false);
        }
        // getBoundingBox: the box of the body's pose (swimming and crawling
        // are 0.6 tall), so a swimmer under a ceiling isn't sent back.
        let d = g.dimensions();
        let (half, height) = (f64::from(d.width) / 2.0, f64::from(d.height));
        let at = |p: [f64; 3]| {
            Aabb::new(
                Vec3::new(p[0] - half, p[1], p[2] - half),
                Vec3::new(p[0] + half, p[1] + height, p[2] + half),
            )
        };
        let (old, new) = (at(g.position), at(to));
        let mut scene = SceneCollisions {
            world: ctx.world,
            binding,
            subject: None,
        };
        // player.move, then how far from the target it ended (vanilla drops
        // the vertical part: `yDist > -0.5 || yDist < 0.5` always holds).
        let movement = Vec3::new(
            to[0] - g.position[0],
            to[1] - g.position[1],
            to[2] - g.position[2],
        );
        let done = scene
            .resolve_with_context(
                old,
                movement,
                0.6,
                g.on_ground,
                // The mover's own sneaking, as CollisionContext.of(player):
                // a sneaking player descends through scaffolding.
                CollisionContext::position(g.position[1]).with_descending(g.sneaking),
            )
            .ok();
        let fail = !g.creative()
            && done.is_some_and(|done| {
                let (dx, dz) = (movement.x - done.x, movement.z - done.z);
                dx * dx + dz * dz > 0.0625
            });
        // Entity.move: verticalCollisionBelow, the player "stands on something".
        let stands = done.is_some_and(|done| movement.y < 0.0 && done.y != movement.y);
        if fail && scene.unobstructed(old).unwrap_or(true) {
            return (true, true, stands);
        }
        // isEntityCollidingWithAnythingNew: a block the old box did not
        // already touch.
        let shrink = f64::from(1.0E-5_f32);
        let shrink = Vec3::new(shrink, shrink, shrink);
        let Ok(old_shape) = VoxelShape::from_box(old.deflate(shrink)) else {
            return (fail, false, stands);
        };
        let context = CollisionContext::position(g.position[1]).with_descending(g.sneaking);
        let Ok(scan) =
            BlockCollisionScan::new(&mut scene, context, new.deflate(shrink), false, 1 << 20)
        else {
            return (fail, false, stands);
        };
        for hit in scan {
            let Ok(hit) = hit else { break };
            if !hit
                .shape
                .join_is_not_empty(&old_shape, ShapeOp::AND)
                .unwrap_or(true)
            {
                return (fail, true, stands);
            }
        }
        (fail, false, stands)
    }

    /// Acknowledge a block interaction (ClientboundBlockChangedAckPacket).
    /// ServerGamePacketListenerImpl.ackBlockChangesUpTo: sent with the next tick's block
    /// changes (`tick`).
    pub fn ack(&self, conn: u32, sequence: i32) {
        if sequence >= 0
            && let Some(g) = self.guest(conn)
        {
            g.ack_up_to.set(g.ack_up_to.get().max(sequence));
        }
    }

    /// Set a guest's game mode and tell everyone (ServerPlayer.setGameMode).
    pub fn set_game_mode(
        &mut self,
        conn: u32,
        mode: i32,
        world: &World,
        collision: Option<&BlockCollisionRegistry<'_>>,
    ) {
        let Some(g) = self.guest_mut(conn) else {
            return;
        };
        g.game_mode = mode;
        // GameType.updatePlayerAbilities: a spectator flies, survival and
        // adventure don't, creative keeps what it had.
        if g.spectator() {
            g.flying = true;
        } else if !g.creative() {
            g.flying = false;
        }
        // ServerPlayerGameMode.changeGameModeForPlayer: a player left flying
        // by the change lands when the ground is within a block.
        if !g.spectator()
            && g.flying
            && let Some(binding) = collision
        {
            let mut scene = SceneCollisions {
                world,
                binding,
                subject: None,
            };
            if in_range_of_ground(&mut scene, g.bounds()) {
                g.flying = false;
            }
        }
        let uuid = g.uuid;
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::CHANGE_GAME_MODE,
            param: mode as f32,
        }));
        g.send(&g.abilities());
        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 4,
            entries: vec![play::PlayerInfo {
                uuid,
                game_mode: Some(mode),
                ..Default::default()
            }],
        }));
    }

    /// A player's nickname changed (None: their name again): kept for chat,
    /// and the Tab list shows it (PlayerInfo UPDATE_DISPLAY_NAME for
    /// everyone).
    pub fn set_nickname(&mut self, uuid: u128, nickname: Option<String>) {
        match nickname {
            Some(n) => self.nicknames.insert(uuid, n),
            None => self.nicknames.remove(&uuid),
        };
        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 0x20,
            entries: vec![play::PlayerInfo {
                uuid,
                display_name: Some(self.nicknames.get(&uuid).map(|n| literal(n))),
                ..Default::default()
            }],
        }));
    }

    /// Set a guest's flying speed and tell it (`Abilities.setFlyingSpeed`, then
    /// `onUpdateAbilities`). The abilities packets after carry it too, so a
    /// respawn or a game mode change keeps it.
    pub fn set_flying_speed(&mut self, conn: u32, speed: f32) {
        let Some(g) = self.guest_mut(conn) else {
            return;
        };
        g.flying_speed = speed;
        // The server's copy of the body, which the movement replay reads.
        if let Some(motion) = g.motion.as_mut() {
            motion.flying_speed = speed;
        }
        g.send(&g.abilities());
    }

    /// The host's game mode changed (PlayerInfo UPDATE_GAME_MODE for everyone).
    pub fn set_host_game_mode(&self, uuid: u128, mode: i32) {
        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 4,
            entries: vec![play::PlayerInfo {
                uuid,
                game_mode: Some(mode),
                ..Default::default()
            }],
        }));
    }

    pub fn disconnect_all(&mut self) {
        let reason = translatable("multiplayer.disconnect.server_shutdown", &[]);
        for g in &self.guests {
            g.sender.disconnect(reason.clone());
        }
    }
}

impl Hub {
    pub fn item_name(&self, id: i32) -> Option<std::sync::Arc<str>> {
        self.sync
            .builtin
            .get("minecraft:item")?
            .name(id)
            .map(std::sync::Arc::from)
    }
    pub fn enchantment_name(&self, id: i32) -> Option<String> {
        self.sync
            .dynamic
            .name("minecraft:enchantment", id)
            .map(str::to_owned)
    }
    /// A stack a client sent (`SetCreativeModeSlot`), read with this
    /// server's registries.
    pub fn stack_from_network(&self, s: &horizon_protocol::item::Stack) -> crate::items::ItemStack {
        crate::item_sync::from_network(s, &self.sync.builtin, &self.sync.dynamic)
    }

    /// ServerLevel.removePlayerImmediately, then addRespawnedPlayer: the other players
    /// lose the dead player's entity (ServerEntity.removePairing) and are sent the
    /// respawned one as a new entity at the next tick. Their clients would otherwise keep
    /// the dead body, lying down, wherever the player goes.
    pub fn respawned(&mut self, id: i32) {
        for g in &mut self.guests {
            if g.entity_id != id && g.tracker.forget(id) {
                g.send(&C::RemoveEntities(play::RemoveEntities { ids: vec![id] }));
            }
        }
    }

    /// A respawn point whose block is gone or blocked: the respawning player
    /// is sent NO_RESPAWN_BLOCK_AVAILABLE and says
    /// `block.minecraft.spawn.not_valid` in its chat.
    pub fn missing_respawn_block(&self, conn: u32) {
        self.send_to(
            conn,
            &C::GameEvent(play::GameEvent {
                event: game_event::NO_RESPAWN_BLOCK_AVAILABLE,
                param: 0.0,
            }),
        );
    }

    /// PlayerList.respawn: back to `at` with a fresh start. The vanilla client waits on its
    /// level loading screen from the Respawn packet until the level info's "level chunks load
    /// start" event and the chunk under the player, so the packets follow vanilla's order.
    pub fn respawn(&mut self, i: usize, at: [f64; 3], ctx: &TickCtx) {
        let info = self.spawn_info(i, 0);
        self.close_container_for(i);
        let g = &mut self.guests[i];
        // PlayerList.respawn: into the level in place (the respawn level).
        g.dimension = self.config.dimension.clone();
        // restartClientLoadTimerAfterRespawn: the client loads a whole new level before it
        // reports it loaded.
        g.load_timeout = super::guest::CLIENT_LOADED_TIMEOUT;
        g.loading = true;
        // ServerLevel.removePlayerImmediately: the dead player's
        // Entity.setRemoved (KILLED) stops its riding; the new player rides
        // nothing.
        g.stop_riding();
        // ServerLevel.removePlayerImmediately: every entity paired with the dead player is
        // removed (ServerEntity.removePairing, one packet each). Its chunks are dropped
        // without ForgetLevelChunk (PlayerChunkSender.dropChunk skips dead players): the
        // client keeps them and they are all sent again.
        for id in g.tracker.ids() {
            g.send(&C::RemoveEntities(play::RemoveEntities { ids: vec![id] }));
        }
        g.tracker.clear();
        g.sent_chunks.clear();
        g.chunk_queue.clear();
        g.far_queue.clear();
        let id = g.entity_id;
        self.respawned(id);
        let g = &mut self.guests[i];
        // ServerPlayer.restoreFrom: the game mode again (onUpdateAbilities).
        g.send(&g.abilities());
        // The new player faces the level's respawn direction (snapTo).
        g.yaw = 0.0;
        g.pitch = 0.0;
        g.position = at;
        g.previous_position = at;
        g.send(&C::Respawn(play::Respawn {
            spawn: info,
            keep: 0,
        }));
        // ClientPacketListener.handleRespawn without KEEP_ENTITY_DATA: the client's new player
        // starts with wasSprinting false. (A level change keeps it: the new player sprints on, or
        // says STOP.)
        g.anticheat.sprinting = false;
        // The respawned player is a new one, without effects; so is the
        // client's (ClientPacketListener.handleRespawn).
        g.effects = Default::default();
        g.sent_effects.clear();
        g.center = None;
        g.sent_chunks.clear();
        g.chunk_queue.clear();
        g.far_queue.clear();
        g.tracker.clear();
        self.resend_teleport(i, None);
        let spawn = self.default_spawn(ctx.spawn);
        let g = &mut self.guests[i];
        g.send(&spawn);
        g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
            difficulty: ctx.difficulty,
            locked: ctx.difficulty_locked,
        }));
        let xp = (g.xp_progress, g.xp_level, g.xp_total);
        g.send(&C::SetExperience(play::SetExperience {
            progress: xp.0,
            level: xp.1,
            total: xp.2,
        }));
        g.sent_xp = Some(xp);
        // ServerPlayer.sendPostEffects.
        let list = g.post_effects.clone();
        g.post_effects_dirty = false;
        g.send(&C::PostEffects(play::PostEffects { effects: list }));
        self.send_level_info(i, ctx, true);
        self.send_permission_level(i);
        // ServerLevel.addRespawnedPlayer: the chunk tracking starts again at the player.
        let g = &mut self.guests[i];
        let (cx, cz) = g.chunk();
        g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
            x: cx,
            z: cz,
        }));
        g.center = None;
        g.center_sent = Some((cx, cz));
        // ServerPlayer.initInventoryMenu.
        self.send_inventory(i);
    }

    /// The level the players are in: the server holds one at a time, and the
    /// hub's packets name it.
    pub fn set_dimension(&mut self, dimension: &str) {
        if self.config.dimension == dimension {
            return;
        }
        // What this level's players were told stays with it.
        let leaving = LevelSync {
            last_clock: self.last_clock,
            last_rate: self.last_rate,
            weather: self.weather,
        };
        let entering = self.level_sync.remove(dimension).unwrap_or_default();
        self.level_sync
            .insert(std::mem::take(&mut self.config.dimension), leaving);
        (self.last_clock, self.last_rate, self.weather) =
            (entering.last_clock, entering.last_rate, entering.weather);
        self.config.dimension = dimension.to_owned();
        self.dimension_type = self
            .sync
            .id("minecraft:dimension_type", dimension)
            .unwrap_or(0);
        self.encoder
            .set_dimension(self.config.levels.get(dimension));
    }

    /// ServerPlayer.teleport(TeleportTransition) into the level the hub now
    /// names (`set_dimension`): ClientboundRespawnPacket keeping all data,
    /// then vanilla's packet order. Nothing tracked in the old level is
    /// removed one by one: the client drops its whole level.
    ///
    /// Through a `portal`, vanilla finds the exit (an End platform, a Nether
    /// portal, a spawn column) before the teleport; here it is found once
    /// the destination has loaded around `at`. Until [`Self::land`] the
    /// client gets no position, so it stays on its level loading screen
    /// instead of showing the player in the air at `at`.
    pub fn change_level(
        &mut self,
        i: usize,
        at: [f64; 3],
        yaw: f32,
        pitch: Option<f32>,
        portal: bool,
        portal_cooldown: i32,
        ctx: &TickCtx,
    ) {
        let info = self.spawn_info(i, portal_cooldown);
        self.close_container_for(i);
        let g = &mut self.guests[i];
        g.dimension = self.config.dimension.clone();
        // The load timer is not restarted: vanilla resets the client's loaded state on death and
        // respawn only, and covers a level change with its separate `isChangingDimension` term.
        g.tracker.clear();
        g.sent_chunks.clear();
        g.chunk_queue.clear();
        g.far_queue.clear();
        g.center = None;
        g.yaw = yaw;
        if let Some(pitch) = pitch {
            g.pitch = pitch;
        }
        g.position = at;
        g.previous_position = at;
        g.send(&C::Respawn(play::Respawn {
            spawn: info,
            // ClientboundRespawnPacket.KEEP_ALL_DATA.
            keep: 3,
        }));
        g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
            difficulty: ctx.difficulty,
            locked: ctx.difficulty_locked,
        }));
        self.send_permission_level(i);
        if portal {
            // A teleport still pending from the old level is void: its late
            // acceptance must not move the player back there.
            let g = &mut self.guests[i];
            g.landing = true;
            g.awaiting_teleport = None;
        } else {
            self.resend_teleport(i, None);
        }
        let g = &mut self.guests[i];
        g.send(&g.abilities());
        self.send_level_info(i, ctx, !portal);
        // PlayerList.sendAllPlayerInfo: the inventory and the held slot.
        self.send_inventory(i);
        let g = &mut self.guests[i];
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        self.send_active_effects(i);
        let g = &mut self.guests[i];
        // ServerPlayer.sendPostEffects.
        let list = g.post_effects.clone();
        g.post_effects_dirty = false;
        g.send(&C::PostEffects(play::PostEffects { effects: list }));
        // lastSentExp, lastSentHealth and lastSentFood; the client's new
        // player starts with the default air and no absorption.
        g.sent_health = None;
        g.sent_xp = None;
        g.sent_air = (300, 0.0);
        g.sent_shared = 0;
        let (cx, cz) = g.chunk();
        g.send(&C::SetChunkCacheCenter(play::SetChunkCacheCenter {
            x: cx,
            z: cz,
        }));
        g.center_sent = Some((cx, cz));
    }

    /// The rest of a portal's [`Self::change_level`] once its exit is found:
    /// the position, LEVEL_CHUNKS_LOAD_START (the client's loading screen
    /// closes once its chunk is in) and TeleportTransition.PLAY_PORTAL_SOUND.
    pub fn land(&mut self, i: usize, at: [f64; 3]) {
        self.guests[i].landing = false;
        self.teleport(i, Some(at));
        let g = &self.guests[i];
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::LEVEL_CHUNKS_LOAD_START,
            param: 0.0,
        }));
        g.send(&C::LevelEvent(play::LevelEvent {
            kind: 1032,
            pos: [0, 0, 0],
            data: 0,
            global: false,
        }));
    }
}

/// A packet Horizon sends as raw bytes (no fields worth a type).
/// multiplayer.disconnect.invalid_player_movement.
/// Mth.wrapDegrees.
fn wrap_degrees(degrees: f32) -> f32 {
    let mut wrapped = degrees % 360.0;
    if wrapped >= 180.0 {
        wrapped -= 360.0;
    }
    if wrapped < -180.0 {
        wrapped += 360.0;
    }
    wrapped
}

/// The packets of one player handled in a tick; the rest wait for the next.
const PACKETS_PER_TICK: usize = 400;
/// The packets a player may have waiting (a second of its budget) before it is
/// disconnected, as vanilla's rate-limit does: no real client gets near it.
const INBOX_LIMIT: usize = 20 * PACKETS_PER_TICK;
/// The same in bytes on the wire: 8000 full books would hold gigabytes. A
/// real client's largest packets (a 100-page book) are a few hundred kB.
const INBOX_BYTES: usize = 16 << 20;

/// The `minecraft:gravity` attribute of a player.
const PLAYER_GRAVITY: f64 = 0.08;

/// `ServerGamePacketListenerImpl.getMaximumFlyingTicks`: 80 ticks, more where
/// gravity is weaker, none at all (never kicked) without gravity.
/// `ServerGamePacketListenerImpl.noBlocksAround` over a box: every block from `lo` to `hi` is air, in
/// loaded chunks (a chunk not loaded yet is not known to be air: nothing floats there).
pub fn no_blocks_in(world: &World, lo: [f64; 3], hi: [f64; 3]) -> bool {
    let cell = |v: f64| v.floor() as i32;
    (cell(lo[0])..=cell(hi[0])).all(|x| {
        (cell(lo[2])..=cell(hi[2])).all(|z| {
            world.loaded(x, z)
                && (cell(lo[1])..=cell(hi[1])).all(|y| world.is_air(world.block(x, y, z)))
        })
    })
}

fn maximum_flying_ticks(gravity: f64) -> i32 {
    if gravity < f64::from(1.0E-5_f32) {
        return i32::MAX;
    }
    let modifier = 0.08 / gravity;
    // Mth.ceil is (int) with a floor correction; a double past int range
    // saturates like the Java cast.
    (80.0 * modifier.max(1.0)).ceil() as i32
}

fn invalid_movement() -> Tag {
    translatable("multiplayer.disconnect.invalid_player_movement", &[])
}

/// The recipe content the native mods asked for (`lan::recipes::content`),
/// to one Horizon client: none when no mod asked.
fn send_recipe_content(g: &Guest) {
    for data in super::recipes::content().iter() {
        g.send(&C::CustomPayload(
            horizon_protocol::packets::configuration::CustomPayload {
                channel: super::recipes::CONTENT_CHANNEL.into(),
                data: data.clone(),
            },
        ));
    }
}

fn raw(id: i32, payload: Vec<u8>) -> C {
    C::Unknown(id, payload)
}

/// ClientboundTickingStatePacket: the tick rate and whether the game is frozen.
fn ticking_state(rate: f32, frozen: bool) -> C {
    let mut w = horizon_protocol::buf::Writer::new();
    w.f32(rate);
    w.bool(frozen);
    raw(ids::TICKING_STATE, w.data)
}

/// What the others see of a guest's body, from its server-side state.
pub fn guest_body(g: &Guest) -> BodyFlags {
    let m = g.motion.as_deref();
    BodyFlags {
        gliding: g.fall_flying,
        swimming: m.is_some_and(|m| m.swimming),
        crawling: m.is_some_and(|m| m.pose == crate::pose::Pose::Swimming && !m.swimming),
        crouching: m.is_some_and(|m| m.pose == crate::pose::Pose::Crouching),
        spin_attack: m.is_some_and(|m| m.auto_spin_attack_ticks > 0),
        on_fire: m.is_some_and(|m| m.fire_ticks > 0),
        // ServerPlayer.updateInvisibilityStatus: a spectator is always invisible.
        invisible: g.effects.get("invisibility").is_some() || g.spectator(),
        glowing: g.effects.get("glowing").is_some(),
    }
}

/// Entity.DATA_SHARED_FLAGS_ID of a player: on fire 0x01, crouching 0x02,
/// sprinting 0x08, swimming 0x10, invisible 0x20, glowing 0x40, gliding 0x80.
fn shared_flags(sneaking: bool, sprinting: bool, body: &BodyFlags) -> i8 {
    let mut shared = 0i8;
    if body.on_fire {
        shared |= 0x01;
    }
    if sneaking {
        shared |= 0x02;
    }
    if sprinting {
        shared |= 0x08;
    }
    if body.swimming {
        shared |= 0x10;
    }
    if body.invisible {
        shared |= 0x20;
    }
    if body.glowing {
        shared |= 0x40;
    }
    if body.gliding {
        shared |= -128;
    }
    shared
}

/// The profile properties the player list gives a player: those it logged
/// in with, or for the integrated server's owner (who logs in with none) the
/// signed-in account's, once fetched (`IntegratedServer.getSingleplayerProfile`).
fn listed_properties(
    logged_in: &[horizon_protocol::packets::login::Property],
    owner: bool,
    profile: Option<&super::OwnerProfile>,
) -> Vec<horizon_protocol::packets::login::Property> {
    match profile {
        Some(profile) if owner && logged_in.is_empty() => profile.properties().to_vec(),
        _ => logged_in.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fast_hash::FxHashSet;

    /// The counts of what is stuck in a player are sent when set, and a
    /// count back at zero is sent as the default.
    #[test]
    fn stuck_counts_are_synched_as_entity_data() {
        let mut state = EntityState::default();
        stuck_meta(&mut state, (3, 0));
        assert_eq!(state.meta, vec![(12, serializer::INT, Value::Int(3))]);
        assert_eq!(
            state.defaults,
            vec![
                (12, serializer::INT, Value::Int(0)),
                (13, serializer::INT, Value::Int(0))
            ]
        );
    }

    /// `getMaximumFlyingTicks`: 80 ticks at normal gravity, ceil(80 * 0.08 /
    /// gravity) where it is weaker, and never a kick without gravity.
    #[test]
    fn maximum_flying_ticks_follow_gravity() {
        assert_eq!(maximum_flying_ticks(0.08), 80);
        assert_eq!(maximum_flying_ticks(0.16), 80);
        assert_eq!(maximum_flying_ticks(0.04), 160);
        assert_eq!(maximum_flying_ticks(0.03), 214);
        assert_eq!(maximum_flying_ticks(0.0), i32::MAX);
        assert_eq!(maximum_flying_ticks(1.0E-6), i32::MAX);
    }

    fn textures(value: &str) -> horizon_protocol::packets::login::Property {
        horizon_protocol::packets::login::Property {
            name: "textures".into(),
            value: value.into(),
            signature: Some("c2lnbmVk".into()),
        }
    }

    /// The owner is listed with the account's skin once the fetch answered,
    /// with none before (no wait) or when it failed; other players keep the
    /// properties they logged in with, and none of the owner's.
    #[test]
    fn the_owner_is_listed_with_the_accounts_skin_once_fetched() {
        let (answer, fetched) = std::sync::mpsc::channel();
        let profile = super::super::OwnerProfile::new(move || fetched.recv().unwrap_or_default());
        let start = std::time::Instant::now();
        assert!(listed_properties(&[], true, Some(&profile)).is_empty());
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
        answer.send(vec![textures("owner")]).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while listed_properties(&[], true, Some(&profile)).is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "the fetch never landed"
            );
            std::thread::yield_now();
        }
        assert_eq!(
            listed_properties(&[], true, Some(&profile)),
            vec![textures("owner")]
        );
        // A guest: its own properties (online mode), or none (offline mode).
        assert_eq!(
            listed_properties(&[textures("guest")], false, Some(&profile)),
            vec![textures("guest")]
        );
        assert!(listed_properties(&[], false, Some(&profile)).is_empty());
        // No signed-in account (no fetch): as before.
        assert!(listed_properties(&[], true, None).is_empty());
        let failed = super::super::OwnerProfile::new(Vec::new);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while failed.properties.get().is_none() {
            let _ = failed.properties();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(listed_properties(&[], true, Some(&failed)).is_empty());
    }

    /// `requeue` as it was, scanning the queue for each chunk of the view.
    fn requeue_scan(
        queue: &mut Vec<(i32, i32)>,
        sent: &FxHashSet<(i32, i32)>,
        (cx, cz): (i32, i32),
        r: i32,
    ) {
        queue.retain(|&(x, z)| tracked(cx, cz, r, x, z));
        for dz in -r - 2..=r + 2 {
            for dx in -r - 2..=r + 2 {
                let key = (cx + dx, cz + dz);
                if tracked(cx, cz, r, key.0, key.1) && !sent.contains(&key) && !queue.contains(&key)
                {
                    queue.push(key);
                }
            }
        }
        queue.sort_by_key(|&(x, z)| i64::from(x - cx).pow(2) + i64::from(z - cz).pow(2));
    }

    /// A player walking a loop, sending a few chunks a tick: the queue
    /// after every move is the one the scan built.
    #[test]
    fn requeue_matches_the_scan() {
        for r in [2, 5, 10, 12, 32] {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            let mut sent = FxHashSet::default();
            for step in 0..60i32 {
                let center = ((step % 17) - 8, (step * 3 % 11) - 5);
                requeue(&mut a, &sent, center, r);
                requeue_scan(&mut b, &sent, center, r);
                assert_eq!(a, b, "radius {r}, step {step}");
                for key in a.drain(..a.len().min(7)) {
                    sent.insert(key);
                }
                b.drain(..b.len().min(7));
                sent.retain(|&(x, z)| tracked(center.0, center.1, r, x, z));
            }
        }
    }

    /// The far ring's queue is what the view's leaves of the chunks tracked at the reach, closest
    /// first, with the sent ones out of both.
    #[test]
    fn the_far_ring_queues_what_the_view_leaves() {
        let mut sent = FxHashSet::default();
        let (mut near, mut far) = (Vec::new(), Vec::new());
        for step in 0..12 {
            let center = (step, -step / 2);
            requeue(&mut near, &sent, center, 10);
            requeue_ring(&mut far, &sent, center, 10, 24);
            let mut all = Vec::new();
            requeue(&mut all, &sent, center, 24);
            assert!(far.iter().all(|key| !near.contains(key)));
            let mut both = [near.clone(), far.clone()].concat();
            both.sort_unstable();
            all.sort_unstable();
            assert_eq!(both, all);
            let distance = |&(x, z): &(i32, i32)| {
                i64::from(x - center.0).pow(2) + i64::from(z - center.1).pow(2)
            };
            assert!(far.windows(2).all(|w| distance(&w[0]) <= distance(&w[1])));
            // A few of each sent.
            for key in near
                .drain(..near.len().min(40))
                .chain(far.drain(..far.len().min(90)))
            {
                sent.insert(key);
            }
        }
        // Without a ring, nothing.
        requeue_ring(&mut far, &sent, (0, 0), 10, 10);
        assert!(far.is_empty());
    }

    /// cargo test --release -p horizon-client --lib lan::hub::tests::requeue_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn requeue_cost() {
        for r in [10, 16, 32] {
            for (name, f) in [
                ("scan", requeue_scan as fn(&mut _, &_, _, _)),
                ("set", requeue as fn(&mut _, &_, _, _)),
            ] {
                let sent = FxHashSet::default();
                let mut queue = Vec::new();
                let start = std::time::Instant::now();
                let moves = 200;
                for step in 0..moves {
                    // Nothing sent yet (an explorer waiting on generation).
                    f(&mut queue, &sent, (step, 0), r);
                }
                let per = start.elapsed().as_secs_f64() * 1e6 / f64::from(moves);
                println!(
                    "view {r}: {name} {per:.1} us per chunk crossed, {} queued",
                    queue.len()
                );
            }
        }
    }
}
