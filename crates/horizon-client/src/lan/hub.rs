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
use horizon_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape};
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
};
use std::sync::{Arc, atomic::Ordering};

/// GameEvent types (ClientboundGameEventPacket).
pub mod chat;

pub mod game_event {
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
    pub seed: i64,
    pub max_players: usize,
    /// A dedicated server: operators from `ops`, the dedicated-only commands.
    pub dedicated: bool,
    /// An integrated server (`dedicated` too, since it has no host player):
    /// the owner's level from `ops`, the others' from Allow Commands and
    /// the guests' command access, and the integrated-only commands.
    pub integrated: bool,
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
    /// `/tick step`: a count that changes with each step started or
    /// stopped, and the ticks the frozen game still runs (`FixedStep::step_state`).
    pub tick_steps: (u32, i32),
    /// Block collisions, for ServerGamePacketListenerImpl's moved-wrongly
    /// check (none: moves are not checked against blocks).
    pub collision: Option<&'a BlockCollisionRegistry<'a>>,
    /// The level's world border (PlayerList.sendLevelInfo sends it).
    pub border: &'a crate::worldborder::WorldBorder,
}

/// What the gameplay code has to do for a guest.
pub enum HostEvent {
    /// A guest entered the world; load their saved state.
    Joined(u32),
    /// A guest left (state to save, entities to drop).
    Left(Box<Guest>),
    Packet(u32, Box<S>),
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
    /// A server log warning (ServerGamePacketListenerImpl's LOGGER.warn).
    Warn(String),
    /// A server log error (LOGGER.error).
    Error(String),
}

pub struct Hub {
    pub lan: Lan,
    pub config: HubConfig,
    pub encoder: ChunkEncoder,
    pub guests: Vec<Guest>,
    pub(super) sync: Arc<SyncData>,
    pub ticks: u64,
    /// The `player_movement_check` and `elytra_movement_check` game rules,
    /// set by the game before each tick.
    pub movement_checks: (bool, bool),
    /// The `immediate_respawn` game rule, as the players were last told.
    immediate_respawn: bool,
    /// An integrated server's owner (`MinecraftServer.isSingleplayerOwner`),
    /// whose moves are never checked.
    pub singleplayer_owner: Option<u128>,
    /// The world is superflat (`ServerLevel.isFlat`, a world-wide flag, sent as
    /// `CommonPlayerSpawnInfo.isFlat` in every dimension).
    pub flat_world: bool,
    /// Who rides what, set by the game before each tick.
    pub riding: super::entity::Riding,
    /// The scoreboard's teams, set by the game each tick, for players'
    /// display names.
    pub teams: Vec<crate::scoreboard::Team>,
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
    chat_type: i32,
    dimension_type: i32,
    clock_ids: Vec<i32>,
    item_ids: std::collections::HashMap<Arc<str>, i32>,
    entity_types: std::collections::HashMap<String, i32>,
    /// Each level's tracked entities, by dimension.
    entities: std::collections::HashMap<String, Entities>,
    /// What one tick of entity tracking sends each guest, kept between ticks.
    entity_out: Vec<Vec<Vec<u8>>>,
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

impl Hub {
    /// Player.getDisplayName: the name formatted for the player's team
    /// (PlayerTeam.formatNameForTeam), suggesting `/tell NAME `, showing the
    /// player on hover and inserting the name.
    pub fn player_display_name(&self, uuid: u128, name: &str) -> Tag {
        let Some(team) = self.teams.iter().find(|t| t.players.contains(name)) else {
            return display_name(uuid, name);
        };
        let mut c = team.formatted_name_comp(&crate::scoreboard::literal(name));
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
    (cx, cz): (i32, i32),
    r: i32,
) {
    queue.retain(|&(x, z)| tracked(cx, cz, r, x, z));
    // A walking player changes chunk every few ticks with hundreds of chunks
    // queued: a set, not a scan of the queue for each of the view's chunks.
    let mut queued: crate::fast_hash::FxHashSet<(i32, i32)> = queue.iter().copied().collect();
    for dz in -r - 2..=r + 2 {
        for dx in -r - 2..=r + 2 {
            let key = (cx + dx, cz + dz);
            if tracked(cx, cz, r, key.0, key.1) && !sent.contains(&key) && queued.insert(key) {
                queue.push(key);
            }
        }
    }
    queue.sort_by_key(|&(x, z)| i64::from(x - cx).pow(2) + i64::from(z - cz).pow(2));
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
        let d = horizon_protocol::registry::DimensionType::vanilla(&config.dimension);
        encoder.set_dimension(d.min_y, d.height, d.has_skylight);
        Self {
            lan,
            config,
            encoder,
            guests: Vec::new(),
            sync,
            ticks: 0,
            immediate_respawn: false,
            movement_checks: (true, true),
            singleplayer_owner: None,
            flat_world: false,
            riding: Default::default(),
            last_clock: i64::MIN,
            last_rate: 0.0,
            ticking: (20.0, false),
            stepping: 0,
            ops: Default::default(),
            entity_range_percent: 100,
            spawn_protection: 0,
            weather: (false, 0.0, 0.0),
            level_sync: Default::default(),
            chat_type,
            dimension_type,
            clock_ids,
            item_ids: Default::default(),
            entity_types: Default::default(),
            entities: Default::default(),
            entity_out: Vec::new(),
            mod_commands: (serde_json::Value::Null, None),
            teams: Vec::new(),
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
        self.broadcast(&C::SetChunkCacheRadius(play::Distance { distance }));
        for g in &mut self.guests {
            g.view_distance = i32::from(g.info.view_distance).clamp(2, distance);
            g.center = None;
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
            properties: guest.map(|g| g.properties.clone()).unwrap_or_default(),
            chat_session: guest.and_then(|g| g.chain.session()).map(|s| s.data()),
            game_mode: Some(mode),
            listed: Some(true),
            latency: Some(latency),
            display_name: Some(None),
            list_order: Some(0),
            show_hat: Some(true),
        }
    }
    /// ClientboundPlayerInfoUpdatePacket.createPlayerInitializing: every action.
    const INFO_ACTIONS: u8 = 0xFF;

    // ---- joining -------------------------------------------------------

    fn spawn_info(&self, i: usize) -> play::SpawnInfo {
        let g = &self.guests[i];
        play::SpawnInfo {
            dimension_type: self.dimension_type,
            dimension: self.config.dimension.clone(),
            seed: horizon_core::biome::obfuscate_seed(self.config.seed),
            game_type: g.game_mode,
            previous_game_type: None,
            debug: false,
            flat: self.flat_world,
            // Player.getLastDeathLocation.
            last_death: g
                .last_death
                .as_ref()
                .map(|(dimension, pos)| play::GlobalPos {
                    dimension: dimension.clone(),
                    pos: *pos,
                }),
            portal_cooldown: 0,
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
            chunk_radius: self.config.view_distance,
            simulation_distance: self.config.simulation_distance,
            reduced_debug_info: false,
            show_death_screen: !self.immediate_respawn,
            limited_crafting: false,
            spawn: self.spawn_info(i),
            // MinecraftServer.usesAuthentication, enforceSecureProfile.
            online_mode: self.lan.shared.config.online.is_some(),
            enforces_secure_chat: self.enforces_secure_chat(),
        }));
        g.send(&C::ChangeDifficulty(play::ChangeDifficulty {
            difficulty: ctx.difficulty,
            locked: ctx.difficulty_locked,
        }));
        g.send(&C::PlayerAbilities(play::PlayerAbilities {
            flags: g.ability_flags(),
            flying_speed: 0.05,
            walking_speed: 0.1,
        }));
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        // PlayerList.placeNewPlayer: ClientboundUpdateRecipesPacket.
        g.send(&raw(ids::UPDATE_RECIPES, self.sync.recipes.clone()));
        // PlayerList.sendPlayerPermissionLevel, then the recipe book.
        self.send_permission_level(i);
        // ServerRecipeBook.sendInitialRecipeBook: each book's open and
        // filtering flags, then every known recipe.
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
        // The others hear of the newcomer before it joins the list.
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
        self.teleport(i, None);
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
        self.send_level_info(i, ctx);
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
        self.guests[i].send(&raw(ids::POST_EFFECTS, vec![0]));
        self.send_inventory(i);
    }

    /// ClientboundUpdateMobEffectPacket for one of the guest's effects.
    fn send_effect(&mut self, i: usize, e: &EffectInstance, blend: bool) -> Option<SentEffect> {
        let effect = self
            .sync
            .id("minecraft:mob_effect", &format!("minecraft:{}", e.id))?;
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
                .id("minecraft:mob_effect", &format!("minecraft:{id}"))
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
        self.teleport_relative(i, position, 0);
    }

    /// A teleport to `position` that keeps the player's motion and look
    /// (`Relative.DELTA` and `Relative.ROTATION`: an End gateway's).
    pub fn teleport_keeping_motion(&mut self, i: usize, position: [f64; 3]) {
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

    /// PlayerList.sendLevelInfo.
    fn send_level_info(&mut self, i: usize, ctx: &TickCtx) {
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
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::LEVEL_CHUNKS_LOAD_START,
            param: 0.0,
        }));
        // TickRateManager.updateJoiningPlayer.
        g.send(&ticking_state(ctx.tick_rate, ctx.tick_frozen));
        g.send(&C::TickingStep(play::TickingStep {
            steps: ctx.tick_steps.1,
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
        events.extend(self.tick_level(ctx));
        self.end_tick();
        events
    }

    /// The network since the last tick, once per server tick before any
    /// level ticks: joins (into the level in place), leaves, and each guest's
    /// packets queued for its own level's pass.
    pub fn poll(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        self.ticks += 1;
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
                NetEvent::Packet(conn, packet) => {
                    if let Some(g) = self.guest_mut(conn) {
                        g.inbox.push_back(*packet);
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
        events
    }

    /// The level in place's part of the tick (`ServerLevel.tick`'s chunk
    /// map and the connections of the players in it): their packets, then
    /// the chunks, block changes, containers and entities they see.
    pub fn tick_level(&mut self, ctx: &mut TickCtx) -> Vec<HostEvent> {
        let mut events = Vec::new();
        for i in 0..self.guests.len() {
            if self.here(&self.guests[i]) {
                self.process_inbox(i, ctx, &mut events);
            }
        }
        if !self.guests.iter().any(|g| self.here(g)) {
            // ServerTickRateManager broadcasts a change to the players there
            // are; one who joins later is sent the state as it is then
            // (`updateJoiningPlayer`).
            if !self.guests.iter().any(|g| g.placed) {
                self.ticking = (ctx.tick_rate, ctx.tick_frozen);
                self.stepping = ctx.tick_steps.0;
            }
            if !self
                .guests
                .iter()
                .any(|g| g.dimension == self.config.dimension)
            {
                let _ = ctx.world.block_log.take();
                let _ = ctx.world.light_log.take();
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

    fn remove_guest(&mut self, i: usize, reason: &str) -> Guest {
        let mut g = self.guests.remove(i);
        if g.left_reason.is_empty() {
            g.left_reason = reason.to_owned();
        }
        if std::env::var_os("HORIZON_NET_DEBUG").is_some() {
            log::info!("[lan] {} left: {reason}", g.name);
        }
        if !g.placed {
            return g;
        }
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
        if (ctx.tick_rate, ctx.tick_frozen) != self.ticking {
            self.ticking = (ctx.tick_rate, ctx.tick_frozen);
            self.broadcast(&ticking_state(ctx.tick_rate, ctx.tick_frozen));
        }
        // ServerTickRateManager.updateStepTicks: a step started or stopped.
        if ctx.tick_steps.0 != self.stepping {
            self.stepping = ctx.tick_steps.0;
            self.broadcast(&C::TickingStep(play::TickingStep {
                steps: ctx.tick_steps.1,
            }));
        }
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
    /// level in between, and each player's packets keep their order.
    fn update_chunks(&mut self, ctx: &TickCtx) {
        let world = &*ctx.world;
        let mut batches = Vec::new();
        for i in 0..self.guests.len() {
            if self.here(&self.guests[i]) {
                self.update_view(i);
                if let Some(batch) = self.next_batch(i, world) {
                    batches.push((i, batch));
                }
            }
        }
        if batches.is_empty() {
            return;
        }
        // Each column once, however many players it goes to, with the
        // number of batches that take it.
        let (mut columns, mut uses): (Vec<&Chunk>, Vec<usize>) = Default::default();
        let mut index = crate::fast_hash::FxHashMap::default();
        let batches: Vec<(usize, Vec<usize>, Vec<usize>)> = batches
            .into_iter()
            .map(|(i, batch)| {
                let (taken, bodies) = batch
                    .into_iter()
                    .map(|(idx, chunk)| {
                        let j = *index.entry((chunk.x, chunk.z)).or_insert_with(|| {
                            columns.push(chunk);
                            uses.push(0);
                            columns.len() - 1
                        });
                        uses[j] += 1;
                        (idx, j)
                    })
                    .unzip();
                (i, taken, bodies)
            })
            .collect();
        self.encoder.warm(&columns);
        let mut bodies: Vec<Vec<u8>> = columns
            .iter()
            .map(|c| self.encoder.body_in(c, world))
            .collect();
        for (i, taken, batch) in batches {
            let g = &mut self.guests[i];
            let size = batch.len();
            g.batch_quota -= size as f32;
            g.send(&C::ChunkBatchStart(horizon_protocol::packets::Empty));
            g.sender.send_many(batch.into_iter().map(|j| {
                uses[j] -= 1;
                if uses[j] == 0 {
                    std::mem::take(&mut bodies[j])
                } else {
                    bodies[j].clone()
                }
            }));
            g.send(&C::ChunkBatchFinished(play::ChunkBatchFinished {
                size: size as i32,
            }));
            g.unacknowledged += 1;
            for idx in taken.into_iter().rev() {
                let key = g.chunk_queue.remove(idx);
                g.sent_chunks.insert(key);
            }
        }
    }

    /// ChunkMap.move's view change: the new centre, the chunks that left
    /// the view forgotten and the ones that entered it queued, closest first.
    fn update_view(&mut self, i: usize) {
        let (cx, cz) = self.guests[i].chunk();
        let r = self.guests[i].view_distance;
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
            .filter(|&(x, z)| !tracked(cx, cz, r, x, z))
            .collect();
        for (x, z) in forget {
            g.sent_chunks.remove(&(x, z));
            g.send(&C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }));
        }
        requeue(&mut g.chunk_queue, &g.sent_chunks, (cx, cz), r);
    }

    /// PlayerChunkSender.sendNextChunks' choice: ready chunks, closest first,
    /// as many as the batch quota the client's acknowledgements pace allows,
    /// with their places in the queue.
    fn next_batch<'w>(&mut self, i: usize, world: &'w World) -> Option<Vec<(usize, &'w Chunk)>> {
        let g = &mut self.guests[i];
        if self.ticks <= g.placed_at + 1 || g.unacknowledged >= g.max_unacknowledged {
            return None;
        }
        g.batch_quota = (g.batch_quota + g.chunks_per_tick).min(g.chunks_per_tick.max(1.0));
        if g.batch_quota < 1.0 || g.chunk_queue.is_empty() {
            return None;
        }
        let budget = g.batch_quota.floor() as usize;
        let batch: Vec<_> = g
            .chunk_queue
            .iter()
            .enumerate()
            .filter_map(|(idx, key)| Some((idx, &**world.chunks.get(key).filter(|c| c.lit)?)))
            .take(budget)
            .collect();
        (!batch.is_empty()).then_some(batch)
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
        let mut changes: Vec<((i32, i32), u32)> = log.drain().collect();
        changes.sort_unstable();
        for ((x, z), sections) in changes {
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
        let Some(log) = world.block_log.as_mut() else {
            return;
        };
        if log.is_empty() {
            return;
        }
        let mut changes = std::mem::take(log);
        changes.sort_unstable();
        changes.dedup();
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
            // Block entities with data the client shows.
            for &(x, y, z) in &positions {
                // A decorated pot is no block entity: its tag is the chunk's saved one.
                let tag = match world.block_entities.get(&(x, y, z)) {
                    Some(entity) => super::chunks::entity_tag(entity),
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
            all.push(self.player_state(
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
            ));
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
                self.player_state(
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
                )
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
        let mut viewers: Vec<Viewer> = self
            .guests
            .iter_mut()
            .filter(|g| here(g))
            .zip(outs.iter_mut())
            .map(|(g, out)| Viewer {
                entity_id: g.entity_id,
                moved: std::mem::take(&mut g.moved),
                position: g.position,
                view_distance: g.view_distance,
                chunks: &g.sent_chunks,
                tracker: &mut g.tracker,
                out,
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
        self.entity_out = outs;
        self.entities.insert(self.config.dimension.clone(), level);
    }

    /// ServerPlayer.stopSleepInBed: the wake-up animation goes to the player
    /// and those around, the pose is standing again and the sleep counter
    /// starts the wake-up fade (100).
    pub fn wake(&mut self, i: usize) {
        let g = &mut self.guests[i];
        if g.bed.take().is_none() && !g.sleeping {
            return;
        }
        g.sleeping = false;
        g.sleep_timer = 100;
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
        if self.guests[i].sent_health.is_none() {
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
        // track_entities).
        let g = &mut self.guests[i];
        if let Some(m) = g.motion.as_mut() {
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

    /// `PlayerAdvancements.flushDirty` and the recipes the player unlocked
    /// since (`ServerRecipeBook.addRecipes`), once the player's first tick
    /// has sent their advancements.
    pub fn flush_progress(&mut self, conn: u32) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn)
            && self.guests[i].sent_health.is_some()
        {
            self.flush_advancements(i, true);
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

    fn process_inbox(&mut self, i: usize, ctx: &mut TickCtx, events: &mut Vec<HostEvent>) {
        // ServerGamePacketListenerImpl.tickPlayer ran between the last
        // tick's packets and these: resetPosition, and the moves it counted.
        let g = &mut self.guests[i];
        g.first_good = g.position;
        g.known_moves = g.received_moves;
        let mut budget = 400;
        while budget > 0 {
            let Some(packet) = self.guests[i].inbox.pop_front() else {
                break;
            };
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
                | S::KeepAlive(_) => false,
                S::MovePlayerPos(p) => self.moved_far(i, [p.x, p.y, p.z]),
                S::MovePlayerPosRot(p) => self.moved_far(i, [p.x, p.y, p.z]),
                S::PlayerInput(_) => self.guests[i].loaded,
                _ => true,
            };
            if acted {
                self.guests[i].last_action_tick = self.ticks;
            }
            match packet {
                // handleAcceptTeleportPacket.
                S::AcceptTeleportation(p) => {
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
                        let rot = (p.yaw, p.pitch);
                        self.position_change(i, [p.x, p.y, p.z], rot, 0, ctx, events);
                    }
                }
                // handlePlayerAbilities: the client flies only if it may.
                S::PlayerAbilities(p) => {
                    let g = &mut self.guests[i];
                    g.flying = p.flags & 2 != 0 && (g.creative() || g.spectator());
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
                S::PlayerLoaded(_) => self.guests[i].loaded = true,
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
                S::ClientTickEnd(_) => self.guests[i].position_this_tick = false,
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
                    g.view_distance =
                        i32::from(info.view_distance).clamp(2, self.config.view_distance);
                    g.info = info;
                    g.center = None;
                }
                S::PlayerInput(p) => {
                    self.guests[i].input = p.flags;
                    self.guests[i].sneaking = p.flags & 32 != 0;
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
                    1 => self.guests[i].sprinting = true,
                    2 => self.guests[i].sprinting = false,
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
                    if self.guests[i]
                        .inbox
                        .iter()
                        .any(|p| matches!(p, S::MovePlayerPos(_) | S::MovePlayerPosRot(_)))
                    {
                        break;
                    }
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
        let g = &mut self.guests[i];
        if pos.is_some() {
            if g.position_this_tick {
                self.kick(conn, invalid_movement());
                return;
            }
            g.position_this_tick = true;
        }
        if !g.loaded && g.ticks == 0 && pos.is_none() {
            return;
        }
        // updateAwaitingTeleport: moves wait for the client to accept the
        // teleport, which is sent again after 20 ticks without an answer.
        if let Some((_, at)) = g.awaiting_teleport {
            if self.ticks - g.teleport_tick > 20 {
                self.teleport(i, Some(at));
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
                self.teleport(i, None);
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
                    self.teleport(i, None);
                    return;
                }
            }
        }
        let start = g.position;
        let (wrongly, refused) = self.collide(i, to, ctx);
        if wrongly {
            let line = format!("{} moved wrongly!", self.guests[i].name);
            events.push(HostEvent::Warn(line));
        }
        if refused {
            let g = &mut self.guests[i];
            (g.yaw, g.pitch) = (yaw, pitch);
            self.teleport(i, Some(start));
            return;
        }
        let g = &mut self.guests[i];
        g.previous_position = g.position;
        g.position = to;
        // ServerChunkCache.move: its pairs are checked again.
        g.moved = true;
        let moved = [0, 1, 2].map(|k| g.position[k] - g.previous_position[k]);
        if g.moves.is_empty() {
            g.moves_from_ground = g.on_ground;
        }
        g.moves.push((moved, flags & 1 != 0));
        g.yaw = yaw;
        g.pitch = pitch.clamp(-90.0, 90.0);
        g.head_yaw = g.yaw;
        g.on_ground = flags & 1 != 0;
        g.ticks = g.ticks.max(1);
    }

    /// The collision part of handlePlayerPositionChange: whether the move
    /// counts as moved wrongly (logged), and whether it is refused (the
    /// player is sent back where they were).
    fn collide(&self, i: usize, to: [f64; 3], ctx: &TickCtx) -> (bool, bool) {
        let g = &self.guests[i];
        // noPhysics (spectators) and sleepers move freely.
        let Some(binding) = ctx.collision else {
            return (false, false);
        };
        if g.spectator() || g.sleeping {
            return (false, false);
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
        let fail = !g.creative()
            && scene
                .resolve(old, movement, 0.6, g.on_ground)
                .is_ok_and(|done| {
                    let (dx, dz) = (movement.x - done.x, movement.z - done.z);
                    dx * dx + dz * dz > 0.0625
                });
        if fail && scene.unobstructed(old).unwrap_or(true) {
            return (true, true);
        }
        // isEntityCollidingWithAnythingNew: a block the old box did not
        // already touch.
        let shrink = f64::from(1.0E-5_f32);
        let shrink = Vec3::new(shrink, shrink, shrink);
        let Ok(old_shape) = VoxelShape::from_box(old.deflate(shrink)) else {
            return (fail, false);
        };
        let context = CollisionContext::position(g.position[1]);
        let Ok(scan) =
            BlockCollisionScan::new(&mut scene, context, new.deflate(shrink), false, 1 << 20)
        else {
            return (fail, false);
        };
        for hit in scan {
            let Ok(hit) = hit else { break };
            if !hit
                .shape
                .join_is_not_empty(&old_shape, ShapeOp::AND)
                .unwrap_or(true)
            {
                return (fail, true);
            }
        }
        (fail, false)
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
    pub fn set_game_mode(&mut self, conn: u32, mode: i32) {
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
        let uuid = g.uuid;
        g.send(&C::GameEvent(play::GameEvent {
            event: game_event::CHANGE_GAME_MODE,
            param: mode as f32,
        }));
        g.send(&C::PlayerAbilities(play::PlayerAbilities {
            flags: g.ability_flags(),
            flying_speed: 0.05,
            walking_speed: 0.1,
        }));
        self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 4,
            entries: vec![play::PlayerInfo {
                uuid,
                game_mode: Some(mode),
                ..Default::default()
            }],
        }));
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

    /// PlayerList.respawn: back to `at` with a fresh start. The vanilla client waits on its
    /// level loading screen from the Respawn packet until the level info's "level chunks load
    /// start" event and the chunk under the player, so the packets follow vanilla's order.
    pub fn respawn(&mut self, i: usize, at: [f64; 3], ctx: &TickCtx) {
        let info = self.spawn_info(i);
        self.close_container_for(i);
        let g = &mut self.guests[i];
        // PlayerList.respawn: into the level in place (the respawn level).
        g.dimension = self.config.dimension.clone();
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
        let id = g.entity_id;
        self.respawned(id);
        let g = &mut self.guests[i];
        // ServerPlayer.restoreFrom: the game mode again (onUpdateAbilities).
        g.send(&C::PlayerAbilities(play::PlayerAbilities {
            flags: g.ability_flags(),
            flying_speed: 0.05,
            walking_speed: 0.1,
        }));
        // The new player faces the level's respawn direction (snapTo).
        g.yaw = 0.0;
        g.pitch = 0.0;
        g.position = at;
        g.previous_position = at;
        g.send(&C::Respawn(play::Respawn {
            spawn: info,
            keep: 0,
        }));
        // The respawned player is a new one, without effects; so is the
        // client's (ClientPacketListener.handleRespawn).
        g.effects = Default::default();
        g.sent_effects.clear();
        g.center = None;
        g.sent_chunks.clear();
        g.chunk_queue.clear();
        g.tracker.clear();
        self.teleport(i, None);
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
        g.send(&raw(ids::POST_EFFECTS, vec![0]));
        self.send_level_info(i, ctx);
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
        let d = horizon_protocol::registry::DimensionType::vanilla(dimension);
        self.encoder
            .set_dimension(d.min_y, d.height, d.has_skylight);
    }

    /// ServerPlayer.teleport(TeleportTransition) into the level the hub now
    /// names (`set_dimension`): ClientboundRespawnPacket keeping all data,
    /// then vanilla's packet order. Nothing tracked in the old level is
    /// removed one by one: the client drops its whole level.
    pub fn change_level(
        &mut self,
        i: usize,
        at: [f64; 3],
        yaw: f32,
        pitch: Option<f32>,
        portal_sound: bool,
        ctx: &TickCtx,
    ) {
        let info = self.spawn_info(i);
        self.close_container_for(i);
        let g = &mut self.guests[i];
        g.dimension = self.config.dimension.clone();
        g.tracker.clear();
        g.sent_chunks.clear();
        g.chunk_queue.clear();
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
        self.teleport(i, None);
        let g = &mut self.guests[i];
        g.send(&C::PlayerAbilities(play::PlayerAbilities {
            flags: g.ability_flags(),
            flying_speed: 0.05,
            walking_speed: 0.1,
        }));
        self.send_level_info(i, ctx);
        // PlayerList.sendAllPlayerInfo: the inventory and the held slot.
        self.send_inventory(i);
        let g = &mut self.guests[i];
        g.send(&C::SetHeldSlot(play::SetHeldSlot {
            slot: g.selected as i32,
        }));
        self.send_active_effects(i);
        let g = &mut self.guests[i];
        // ServerPlayer.sendPostEffects.
        g.send(&raw(ids::POST_EFFECTS, vec![0]));
        // TeleportTransition.PLAY_PORTAL_SOUND.
        if portal_sound {
            g.send(&C::LevelEvent(play::LevelEvent {
                kind: 1032,
                pos: [0, 0, 0],
                data: 0,
                global: false,
            }));
        }
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

fn invalid_movement() -> Tag {
    translatable("multiplayer.disconnect.invalid_player_movement", &[])
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
        invisible: g.effects.get("invisibility").is_some(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fast_hash::FxHashSet;

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
