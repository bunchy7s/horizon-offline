//! Multiplayer: the game driven by a server connection instead of the local
//! simulation. Mirrors vanilla's ClientPacketListener (chunks, entities,
//! inventory, health, time, weather, chat, respawn and dimension changes),
//! LocalPlayer.sendPosition (movement packets) and MultiPlayerGameMode
//! (block breaking with sequence numbers, use-item, attacks, container
//! clicks). The player still moves with the local physics, as the vanilla
//! client predicts its own movement; the server stays authoritative for
//! everything else.
use super::remote_chunks::{ChunkDecoder, ChunkFormat, Next};
use super::{Dim, Game, Mining, Screen};
use horizon_client::first_person::{Hand, SwingAnimation};
use horizon_client::{
    Result,
    blocks::Fluid,
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    entities::{Experience, ExperienceOrb, ItemEntity},
    hud::HudBatch,
    items::ItemStack,
    menu::Input,
    mobs::{Mob, MobType, Who},
    motion::{MotionWorld, PlayerMotion},
    net::{Event, Session},
    save::SaveDir,
    screen_effects,
    sprint::Keys,
    streaming::{ChunkGenerator, Proto, Streaming},
    tab_list::SPECTATOR,
    world::World,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_protocol::{
    entity_data::{ITEM_ENTITY_ITEM, Value},
    item::{Hashed, Stack},
    packets::{
        Empty,
        play::{self, Clientbound as C, Serverbound as S},
    },
    registry::{Builtin, DimensionType, Dynamic, SyncedNames, ceil_log2},
    rsa::RsaPublicKey,
    secure_chat::{self, Signature},
    text::{Language, plain},
};
use horizon_world::{
    level::InteractionResult,
    nbt::Tag,
    registry::BlockStateId,
    stepped_interpolation::{Path, PosRot, Start, SteppedInterpolation},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{event::MouseButton, keyboard::KeyCode};

/// `ChatType`'s two decorations: how a message shows in the chat, and what
/// the narrator says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decoration {
    Chat,
    Narration,
}

/// `ChatTypeDecoration.decorate` of a chat type's `chat` or `narration`
/// entry: its translation, with the sender, the target or the message as its
/// parameters, in its style. A chat type the registry lacks decorates as
/// `minecraft:chat` (`chat.type.text`, narrated `chat.type.text.narrate`).
fn chat_type_decorate(
    chat_type: Option<&Tag>,
    decoration: Decoration,
    bound: &play::ChatTypeBound,
    content: &serde_json::Value,
) -> serde_json::Value {
    use horizon_client::scoreboard::tag_to_json;
    let (field, default_key) = match decoration {
        Decoration::Chat => ("chat", "chat.type.text"),
        Decoration::Narration => ("narration", "chat.type.text.narrate"),
    };
    let decoration = chat_type.and_then(|t| t.get(field));
    let key = match decoration.and_then(|d| d.get("translation_key")) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => default_key.into(),
    };
    let parameter = |name: &str| match name {
        "sender" => tag_to_json(&bound.name),
        "target" => bound
            .target
            .as_ref()
            .map_or_else(|| serde_json::Value::from(""), tag_to_json),
        _ => content.clone(),
    };
    let with: Vec<serde_json::Value> = match decoration.and_then(|d| d.get("parameters")) {
        Some(Tag::List(list)) => list
            .iter()
            .map(|p| match p {
                Tag::String(s) => parameter(&s.to_string_lossy()),
                _ => serde_json::Value::from(""),
            })
            .collect(),
        _ => vec![parameter("sender"), parameter("content")],
    };
    let mut component = match decoration.and_then(|d| d.get("style")) {
        Some(style @ Tag::Compound(_)) => tag_to_json(style),
        _ => serde_json::json!({}),
    };
    component["translate"] = key.into();
    component["with"] = with.into();
    component
}

/// A text component the server sent, for the HUD.
pub(super) fn remote_text(tag: &Tag) -> horizon_client::text::Text {
    use horizon_client::scoreboard::{comp_text, tag_to_json};
    comp_text(&tag_to_json(tag))
}

/// The hurt sound of a `ClientboundDamageEventPacket`'s damage type, by its
/// synchronized registry id: `Player.getHurtSound` picks the type's effects.
fn hurt_sound_for(
    registries: &Dynamic,
    damage: &horizon_world::damage_source::Catalog,
    source_type: i32,
) -> &'static str {
    if let Some(Tag::String(effects)) = registries
        .data("minecraft:damage_type", source_type)
        .and_then(|t| t.get("effects"))
        && let Ok(effects) = effects.to_string()
    {
        return horizon_client::health::hurt_sound(&effects);
    }
    registries
        .name("minecraft:damage_type", source_type)
        .and_then(|name| damage.id(name).and_then(|id| damage.definition(id)))
        .map_or(horizon_client::health::hurt_sound("hurt"), |d| {
            horizon_client::health::hurt_sound(&d.effects)
        })
}

/// Position of the entity a `ClientboundSoundEntityPacket` names, if this level
/// has it and `EntityBoundSoundInstance.canPlaySound` (`!entity.isSilent()`)
/// holds. The local player is not a tracked entity; removed ids leave the
/// tracked table.
fn entity_position(
    id: i32,
    player: Option<(i32, [f64; 3])>,
    mobs: &[Mob],
    tracked: &HashMap<i32, Tracked>,
) -> Option<[f64; 3]> {
    if let Some((player_id, at)) = player
        && player_id == id
    {
        return Some(at);
    }
    if let Some(m) = mobs.iter().find(|m| m.id as i32 == id) {
        return (!m.silent).then_some([m.position.x, m.position.y, m.position.z]);
    }
    tracked.get(&id).map(|t| t.position)
}

/// `ClientboundOpenScreenPacket.title` as a container screen draws it: the
/// component's styles, and the `§` codes a literal holds (`StringDecomposer`).
fn menu_title(tag: &Tag) -> horizon_client::text::Text {
    remote_text(tag).resolve_legacy()
}

/// A player message Chat Delay holds back, its signature and trust level,
/// and what the narrator says of it once it shows.
type QueuedChat = (
    horizon_client::text::Text,
    horizon_client::chat::ChatSource,
    Option<Signature>,
    ChatTrust,
    String,
);

/// `ChatTrustLevel`; a modified message keeps the signed content it was
/// changed from.
pub(super) enum ChatTrust {
    Secure,
    Modified(String),
    NotSecure,
    /// `GuiMessageTag.chatError`: the line `ChatListener.handleChatMessageError` adds.
    Error,
}
impl ChatTrust {
    /// `ChatTrustLevel.createTag`.
    fn tag(&self) -> Option<horizon_client::chat::Tag> {
        match self {
            Self::Secure => None,
            Self::Modified(original) => Some(horizon_client::chat::Tag::modified(original)),
            Self::NotSecure => Some(horizon_client::chat::Tag::not_secure()),
            Self::Error => Some(horizon_client::chat::Tag::chat_error()),
        }
    }
}

/// Streaming needs a generator; a server world never generates.
struct NoTerrain;
impl ChunkGenerator for NoTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(&self, _: &mut (), _: i32, _: i32) -> Proto {
        Box::new(())
    }
    fn finish(&self, _: Proto) -> Chunk {
        Chunk::empty(0, 0)
    }
}

/// A server entity the client tracks (position as last sent).
struct Tracked {
    kind: String,
    position: [f64; 3],
}
/// Smooth an entity toward a new position over three ticks (InterpolationHandler).
#[derive(Clone, Copy)]
/// `LinearInterpolationHandler`'s target: a position and rotation (yRot,
/// xRot) reached in `steps` ticks.
struct Lerp {
    target: [f64; 3],
    rotation: Option<(f32, f32)>,
    steps: u32,
}

/// `NewMinecartBehavior.MinecartStep` (its xRot is not kept: a minecart here
/// pitches along its rails).
#[derive(Clone, Copy)]
struct CartStep {
    position: Vec3,
    movement: Vec3,
    yaw: f32,
    weight: f32,
}

/// The client's state of a minecart on `NewMinecartBehavior`: the steps the
/// server sent (`lerpSteps`), the ones being played over 3 ticks
/// (`currentLerpSteps`) and where the cart was when they began (`oldLerp`).
struct CartLerp {
    pending: Vec<CartStep>,
    current: Vec<CartStep>,
    total_weight: f64,
    delay: i32,
    old: CartStep,
}

impl CartLerp {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
            current: Vec::new(),
            total_weight: 0.0,
            delay: 0,
            old: CartStep {
                position: Vec3::ZERO,
                movement: Vec3::ZERO,
                yaw: 0.0,
                weight: 0.0,
            },
        }
    }

    /// `NewMinecartBehavior.getCartLerpPosition`, `getCartLerpMovements` and
    /// `getCartLerpYRot` at `partial`: the step the weights put this moment
    /// in, and how far along it, between it and the one before.
    fn at(&self, partial: f32) -> (Vec3, Vec3, f32) {
        let alpha = ((3 - self.delay) as f32 + partial) / 3.0;
        let mut count_up = 0.0f32;
        let mut in_step = 1.0f32;
        let mut found = None;
        for (i, s) in self.current.iter().enumerate() {
            if s.weight <= 0.0 {
                continue;
            }
            count_up += s.weight;
            if f64::from(count_up) >= self.total_weight * f64::from(alpha) {
                let before = count_up - s.weight;
                in_step = ((f64::from(alpha) * self.total_weight - f64::from(before))
                    / f64::from(s.weight)) as f32;
                found = Some(i);
                break;
            }
        }
        let index = found.unwrap_or(self.current.len() - 1);
        let current = self.current[index];
        let previous = if index > 0 {
            self.current[index - 1]
        } else {
            self.old
        };
        let t = f64::from(in_step);
        let lerp = |a: Vec3, b: Vec3| {
            Vec3::new(
                a.x + t * (b.x - a.x),
                a.y + t * (b.y - a.y),
                a.z + t * (b.z - a.z),
            )
        };
        (
            lerp(previous.position, current.position),
            lerp(previous.movement, current.movement),
            previous.yaw + in_step * wrap_degrees(current.yaw - previous.yaw),
        )
    }

    /// `NewMinecartBehavior.lerpClientPositionAndRotation`: once the steps
    /// being played are used up, the queued ones become the current ones;
    /// the cart then sits where the steps put it at the end of this tick.
    /// False when it has no steps to play.
    fn tick(&mut self, c: &mut super::minecarts::Minecart) -> bool {
        self.delay -= 1;
        if self.delay <= 0 {
            self.old = CartStep {
                position: c.position,
                movement: c.velocity,
                yaw: c.yaw,
                weight: 0.0,
            };
            self.current.clear();
            if !self.pending.is_empty() {
                self.current.append(&mut self.pending);
                self.total_weight = self.current.iter().map(|s| f64::from(s.weight)).sum();
                self.delay = if self.total_weight == 0.0 { 0 } else { 3 };
            }
        }
        if self.current.is_empty() {
            return false;
        }
        let (position, movement, yaw) = self.at(1.0);
        c.position = position;
        c.velocity = movement;
        c.yaw = yaw;
        true
    }
}

/// `SteppedInterpolationHandler.doInterpolate`'s `setPos` and `setRot` on a
/// mob: its yaw kept continuous (the same turn as vanilla's, without the
/// wrap that would spin it in the frame between ticks), its pitch clamped.
fn place_interpolated(m: &mut Mob, at: PosRot) {
    m.position = Vec3::new(at.position[0], at.position[1], at.position[2]);
    m.yaw += wrap_degrees(at.yaw - m.yaw);
    m.head_pitch = (at.pitch % 360.0).clamp(-90.0, 90.0);
}

/// The packet a living entity's move came in.
#[derive(Clone, Copy, PartialEq)]
enum MovePacket {
    /// `handleMoveEntity`: always interpolated.
    Move,
    /// `handleEntityPositionSync`.
    PositionSync,
    /// `handleTeleportEntity` (`setValuesFromPositionPacket`).
    Teleport,
}

impl MovePacket {
    /// `tooBigToInterpolate`: a move to `to` of more than 64 blocks snaps.
    /// A position sync measures it from where the entity is, a teleport from
    /// where it is headed (`PositionMoveRotation.of`'s
    /// `getClientPositionAndRotation`).
    fn snaps(self, handler: &SteppedInterpolation, entity: PosRot, to: [f64; 3]) -> bool {
        let from = match self {
            MovePacket::Move => return false,
            MovePacket::PositionSync => entity.position,
            MovePacket::Teleport => handler.heading(entity).position,
        };
        let d = [0, 1, 2].map(|i| to[i] - from[i]);
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 4096.0
    }
}

/// A move `packet` to `to` for a living entity at `entity`, through its
/// handler (`interpolateTo`): where it is put at once, if anywhere. A move
/// too big to interpolate is a `snapTo`; vanilla leaves the handler running
/// and its next step shifts what is left by the jump
/// (`adjustInterpolationTargetFromDeltas`), which this handler doesn't port,
/// so the steps are dropped instead. That is the same for an entity that
/// wasn't moving, as a teleported one usually isn't.
fn stepped_move(
    handler: &mut SteppedInterpolation,
    packet: MovePacket,
    entity: PosRot,
    to: [f64; 3],
    path: Option<Path>,
    rotation: Option<(f32, f32)>,
) -> Option<PosRot> {
    if packet.snaps(handler, entity, to) {
        handler.cancel();
        let (yaw, pitch) = rotation.unwrap_or((entity.yaw, entity.pitch));
        return Some(PosRot {
            position: to,
            yaw,
            pitch,
        });
    }
    match handler.interpolate_to(entity, path, rotation) {
        Start::Snap(at) => Some(at),
        Start::Interpolating | Start::Unchanged => None,
    }
}

/// What the network thread delivered, in order: an event, or a chunk packet
/// handed to the decoder (with the format it decodes with), whose column
/// comes back from `ChunkDecoder::next`.
enum Received {
    Event(Event),
    Chunk(Arc<ChunkFormat>),
}

/// `Minecraft.services().profileKeySignatureValidator()`: the services' profile-key keys, which a
/// player's chat key must be signed by. Asked for once per connection to a server, off the game
/// thread.
pub(super) enum ServicesKeys {
    Pending,
    Ready(Vec<RsaPublicKey>),
    /// `canValidateProfileKeys()` is false: none could be had.
    Missing,
}
/// The signed-in account's `/minecraft/profile` fetch after joining.
enum OwnProfile {
    Idle,
    Fetching(std::sync::mpsc::Receiver<Option<horizon_client::skins::Textures>>),
    Done(Option<horizon_client::skins::Textures>),
}

pub(super) struct Remote {
    pub session: Session,
    pub brand: String,
    /// The server announced it runs mods' quiet commands.
    pub quiet_commands: bool,
    /// The server announced it carries mod messages.
    pub mod_net: bool,
    pub(super) builtin: Arc<Builtin>,
    lang: Arc<Language>,
    pub(super) registries: Arc<Dynamic>,
    pub(super) joined: bool,
    /// `ServerReconfigScreen` is up since this tick.
    pub(super) reconfiguring: Option<u64>,
    /// `handleLogin` ran: vanilla's `Minecraft.player` exists from then on,
    /// while here the player is made at the first teleport.
    pub(super) logged_in: bool,
    status: String,
    pub(super) entity_id: i32,
    /// `Minecraft.cameraEntity` when the server put the view on another
    /// entity (ClientboundSetCameraPacket); None while it is the player.
    pub(super) camera: Option<i32>,
    /// The item the player's use began with (`LivingEntity.useItem`).
    pub(super) use_item: Option<Arc<str>>,
    pub(super) dimension: DimensionType,
    /// What chunk packets decode with (the dimension's height and sky, the
    /// biome registry); replaced whenever one of them changes.
    pub(super) chunk_format: Arc<ChunkFormat>,
    /// Decodes chunk packets as they arrive, off the frame.
    decoder: ChunkDecoder,
    /// Options.serverRenderDistance: the server's view distance from the
    /// login or the last chunk-cache-radius packet.
    pub(super) view_distance: i32,
    /// `ClientLevel.serverSimulationDistance`: from the login or the last
    /// simulation-distance packet.
    pub(super) simulation_distance: i32,
    sequence: i32,
    position_sender: horizon_client::player_sync::PositionSender,
    last_input: u8,
    was_sprinting: bool,
    carried: Option<usize>,
    /// The last ClientboundPlayerAbilitiesPacket. Vanilla's LocalPlayer
    /// holds its abilities from the login on; here the player is made at
    /// the first teleport, which takes them then.
    abilities: Option<play::PlayerAbilities>,
    loaded_sent: bool,
    /// `ClientPacketListener.levelLoadTracker`: the loading screen after a login
    /// or respawn, until the player's section is shown.
    pub(super) level_load: horizon_client::level_loading::LevelLoadTracker,
    /// The ClientInformation last sent (`Options.broadcastOptions` sends it
    /// again when a view distance, Chat, skin or Main Hand option changes).
    sent_info: horizon_protocol::packets::configuration::ClientInformation,
    /// BlockStatePredictionHandler: blocks changed ahead of the server (a predicted break),
    /// with the sequence of the last action that changed them and the state the server
    /// last said they have. Until the server acknowledges that sequence its block updates
    /// only replace the remembered state; then the block takes it.
    predictions: HashMap<[i32; 3], (i32, i32)>,
    /// Open container: (window id, menu name).
    pub(super) menu: Option<(i32, String)>,
    /// The open menu's title as the server named it, with its styles.
    pub(super) menu_title: Option<horizon_client::text::Text>,
    state_id: i32,
    use_target: Option<[i32; 3]>,
    entities: HashMap<i32, Tracked>,
    lerps: HashMap<i32, Lerp>,
    /// The minecarts the server moves with `ClientboundMoveMinecartPacket`
    /// (`NewMinecartBehavior`), which have no interpolation handler.
    cart_lerps: HashMap<i32, CartLerp>,
    /// Living entities' `SteppedInterpolationHandler`s.
    stepped: HashMap<i32, SteppedInterpolation>,
    /// `LivingEntity.lerpHeadTo`: the head yaw and the ticks left to reach it.
    head_lerps: HashMap<i32, (f32, u32)>,
    /// The happy ghast the player just got off, until the server's teleport
    /// puts the player on top of it (`place_ghast_under`).
    pub(super) ghast_dismount: Option<u32>,
    /// Whether the player is the first passenger of its vehicle, the one
    /// `getControllingPassenger` names.
    pub(super) first_passenger: bool,
    /// Item entities waiting for their item data: where they are and their
    /// motion.
    pending_items: HashMap<i32, ([f64; 3], Vec3)>,
    health: (f32, i32, f32),
    /// `LocalPlayer.flashOnSetHealth`: set by the first health the server
    /// sends; from then on a drop starts the hurt animation.
    flash_on_set_health: bool,
    game_mode: i32,
    pub chat: Vec<(String, Instant)>,
    /// Lines received since joining, for the control socket's events.
    pub chat_received: u64,
    /// `ChatListener.delayedMessageQueue`: player chat held back by Chat
    /// Delay, with the signature a deletion names it by and what the
    /// narrator says once it shows.
    chat_queue: VecDeque<QueuedChat>,
    /// `ClientPacketListener.messageSignatureCache`: the signatures packed
    /// ones index, as the server's copy for this connection holds them.
    pub(super) signatures: horizon_protocol::secure_chat::SignatureCache,
    /// `ChatListener.previousMessageTime`: when a message last showed, if one has.
    chat_previous: Option<Instant>,
    /// `ClientPacketListener.nextChatIndex`: the global index the next chat packet must carry.
    next_chat_index: i32,
    /// `serverEnforcesSecureChat`, from the login packet.
    enforces_secure_chat: bool,
    services_keys: Arc<std::sync::Mutex<ServicesKeys>>,
    /// The `RemoteChatSession.Data` each player's list entry last carried (INITIALIZE_CHAT).
    chat_sessions: HashMap<u128, Option<(u128, i64, Vec<u8>, Vec<u8>)>>,
    /// `PlayerInfo.messageValidator` of the players whose chat has been read.
    chat_validators: HashMap<u128, secure_chat::MessageValidator>,
    /// `ConnectScreen.lastNarration`: when "Joining the world" was last said.
    joining_narrated: Option<Instant>,
    /// The tree the chat colours, completes and parses with: the server's
    /// (ClientboundCommandsPacket) once it has sent one, else the game's own,
    /// plus the commands native mods own (`chat::chat_tree_for`). Rebuilt where
    /// that base changes, not at each keystroke.
    pub chat_tree: horizon_client::command::Dispatcher,
    /// Whether a `ClientboundCommandsPacket` has supplied the server's tree.
    /// The local fallback is for UI completion only; vanilla signs command
    /// arguments against an empty tree until the server sends one.
    pub(super) chat_tree_received: bool,
    /// The chat session, the last-seen window and the signature cache.
    pub(super) secure_chat: super::remote_chat::SecureChat,
    /// `ClientSuggestionProvider.customCompletionSuggestions`: the extra chat
    /// completions this server sent (ClientboundCustomChatCompletionsPacket).
    custom_completions: std::collections::BTreeSet<String>,
    players: HashMap<u128, String>,
    /// `ClientPacketListener.playerInfoMap` as player-list entries, and
    /// which of them are listed.
    tab_players: HashMap<u128, horizon_client::tab_list::TabPlayer>,
    /// Each listed profile's unpacked `textures` property.
    skin_textures: HashMap<u128, horizon_client::skins::Textures>,
    /// Active skin and cape from the services profile when the server sent none.
    own_profile: OwnProfile,
    /// The skin texture each profile's players show, once its download is ready.
    skins: HashMap<u128, String>,
    listed: std::collections::HashSet<u128>,
    /// `ClientboundLoginPacket.onlineMode`: the player list shows heads.
    pub(super) online_mode: bool,
    /// Player entities in the level: entity id to profile UUID.
    level_players: HashMap<i32, u128>,
    /// The profile name each player entity was created with (`RemotePlayer.getGameProfile`): it outlives the
    /// player-list entry, which servers remove to hide their NPCs from the list.
    level_player_names: HashMap<i32, String>,
    /// The `textures` each player entity found in the player list first
    /// (`AbstractClientPlayer.getPlayerInfo` keeps that `PlayerInfo`): its skin
    /// outlives the entry, which NPC plugins remove a moment after spawning.
    level_player_textures: HashMap<i32, horizon_client::skins::Textures>,
    /// Entities with shared flag 6 (`Entity.isCurrentlyGlowing` on a client); mobs keep
    /// theirs in `Mob::glowing`.
    glowing: std::collections::HashSet<i32>,
    /// The player's own shared flag 5 (`Entity.isInvisible`) as the server
    /// last set it.
    pub(super) own_invisible: bool,
    /// `LivingEntity.DATA_ARROW_COUNT_ID` and `DATA_STINGER_COUNT_ID` of the
    /// own player, as the server synced them.
    pub(super) own_arrows: i32,
    pub(super) own_stingers: i32,
    /// `Player.DATA_SHOULDER_PARROT_LEFT` and `_RIGHT` of the own player.
    pub(super) own_shoulders: [Option<i32>; 2],
    /// The local player's profile UUID (always in the level).
    profile: u128,
    /// The local player's profile name: its score holder and team member.
    pub(super) profile_name: String,
    /// `MultiPlayerGameMode.destroyBlockPos`, `.destroyProgress` and the face last sent: the block
    /// the break targets, which outlives the break itself.
    destroying: Option<([i32; 3], u8, f32)>,
    /// `MultiPlayerGameMode.destroyingItem`: the held item key the break began with.
    destroying_item: String,
    /// `MultiPlayerGameMode.isDestroying`: a break is running. `destroying` keeps the target after
    /// one ends (`destroyBlockPos` is written only when a break starts), so a dig resumed on the
    /// same block and item sends no new START and the server keeps its clock (#4124).
    is_destroying: bool,
    destroy_delay: u32,
    input: horizon_client::interaction_input::InteractionInput,
    /// Keys whose presses send a packet (swap hands, drop, a mount's
    /// inventory): like the click queue, they wait for the tick's
    /// `handleKeybinds` phase rather than going out between window events.
    pub(super) pending_keys: Vec<KeyCode>,
    /// Whether the main and off hand still wait for the server's answer to
    /// a SWAP_ITEM_WITH_OFFHAND (`remote_hand_slot_set`).
    swap_answer: [bool; 2],
    stop_mining: bool,
    placing: Option<([i32; 3], u64)>,
    /// `CommonPlayerSpawnInfo.isFlat`: the level is a superflat one.
    pub(super) flat: bool,
    death_message: Option<horizon_client::text::Text>,
    /// `LocalPlayer.showDeathScreen`: off with `immediate_respawn`.
    show_death_screen: bool,
    player_type: Option<Arc<MobType>>,
    /// Inventory, off hand and armour as last synchronised, for creative-mode
    /// slot updates (`creative_sync_stacks`).
    synced: Vec<Option<ItemStack>>,
    decode_errors: u32,
    clock: Option<i32>,
    /// `ClientClockManager.ClientClockInstance`'s rate and partial tick for
    /// the level's clock (its total is `world.day_time`), and
    /// `lastTickGameTime`.
    clock_rate: f32,
    clock_partial: f32,
    clock_game_time: i64,
    /// `ClientChunkCache.Storage`'s view centre (ClientboundSetChunkCacheCenterPacket).
    chunk_center: (i32, i32),
    /// Received events a frame left for the next one, in order, and when
    /// the oldest of them first waited (`poll_remote`).
    backlog: VecDeque<Received>,
    backlog_since: Option<Instant>,
    /// `ClientboundTransferPacket`'s target: when the connection ends the
    /// game connects here, the stored cookies carried over (`TransferState`).
    transfer: Option<String>,
    /// `ClientCommonPacketListenerImpl.serverLinks`: the links "Server
    /// Links..." in the pause menu lists (only `http`/`https` URIs kept).
    pub(super) server_links: Vec<horizon_protocol::packets::configuration::ServerLink>,
    /// `ClientPacketListener.getDebugQueryHandler`: the F3+I query waiting
    /// for the server's data.
    pub(super) debug_query: super::debug_keys::DebugQuery,
}

/// The time a frame spends on chunk packets before it leaves the rest of
/// the received events for the next frame. A batch of 25 to 60 columns
/// (0.1 ms each on an M4 Pro) all in one frame was a 3 to 8 ms frame every
/// time the player crossed a chunk border at 32 chunks; they decode on the
/// chunk decoder's thread now, and the frame only inserts them.
const CHUNK_PACKETS_PER_FRAME: Duration = Duration::from_millis(1);
/// Events that have waited this long are all handled in the next frame,
/// whatever inserting their columns costs, up to a column still decoding:
/// the packets after a slow machine's chunks don't wait on its frames.
const BACKLOG_LIMIT: Duration = Duration::from_millis(50);
/// The open menu name of a server's horse inventory, then its chest's
/// columns (the packet names no `MenuType`).
const MOUNT_MENU: &str = "horizon:mount";

fn face_direction(face: [i32; 3]) -> u8 {
    match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        [1, 0, 0] => 5,
        _ => 1,
    }
}

/// A protocol number format as the scoreboard's.
fn number_format(f: &play::NumberFormat) -> horizon_client::scoreboard::NumberFormat {
    use horizon_client::scoreboard::{NumberFormat, tag_to_json};
    match f {
        play::NumberFormat::Blank => NumberFormat::Blank,
        play::NumberFormat::Styled(t) => NumberFormat::Styled(tag_to_json(t)),
        play::NumberFormat::Fixed(t) => NumberFormat::Fixed(tag_to_json(t)),
    }
}

impl Game<'_> {
    /// A `Holder<SoundEvent>` off the wire: an inline event's location, or
    /// the name of a registered one.
    pub(super) fn remote_sound_name(
        &self,
        sound: &(i32, Option<(String, Option<f32>)>),
    ) -> Option<String> {
        match &sound.1 {
            Some((location, _)) => Some(location.clone()),
            None => self.remote.as_ref().and_then(|r| {
                r.builtin
                    .get("minecraft:sound_event")?
                    .name(sound.0)
                    .map(str::to_owned)
            }),
        }
    }
    /// Position of an entity `ClientboundSoundEntityPacket` names.
    fn remote_entity_position(&self, id: i32) -> Option<[f64; 3]> {
        let r = self.remote.as_ref()?;
        let player = self
            .player
            .as_ref()
            .map(|p| (r.entity_id, [p.position.x, p.position.y, p.position.z]));
        entity_position(id, player, &self.mobs, &r.entities)
    }
    /// The hurt sound the hurt player's own client plays from
    /// `ClientboundDamageEventPacket` (`LivingEntity.handleDamageEvent`):
    /// `Player.getHurtSound` is the damage type's effects, with the codec's
    /// `hurt` default when the type is unknown.
    pub(super) fn remote_hurt_sound(&self, source_type: i32) -> &'static str {
        match self.remote.as_ref() {
            Some(r) => hurt_sound_for(&r.registries, &self.motion.health.damage, source_type),
            None => horizon_client::health::hurt_sound("hurt"),
        }
    }
    /// Leave the title for a server (ConnectScreen): an empty world fed by the connection.
    pub(super) fn connect_server(&mut self, address: &str, name: &str) -> Result<()> {
        self.connect_server_with_cookies(address, name, None)
    }
    /// `connect_server` carrying a transfer's stored server cookies
    /// (`TransferState`); None on an ordinary join.
    pub(super) fn connect_server_with_cookies(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<horizon_client::net::Cookies>,
    ) -> Result<()> {
        self.connect_server_with_options(address, name, cookies, false)
    }
    /// `ConnectScreen.startConnecting`: keep LAN discovery's server kind through loading.
    pub(super) fn connect_server_with_options(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
    ) -> Result<()> {
        super::world_loading::preload_server_loot(&self.options.resources);
        if self.defer_world_entry(super::PendingWorldEntry::Server {
            address: address.to_owned(),
            name: name.to_owned(),
            cookies: cookies.clone(),
            lan,
        })? {
            return Ok(());
        }
        self.join_world_rest()?;
        if let Err(error) = self.join_server(address, name, None, cookies, lan) {
            log::warn!("Horizon: could not join server: {error}");
            if self.title.is_none() {
                self.title = Some(
                    horizon_client::title::Title::new(
                        std::env::current_dir()?.join("saves"),
                        &self.options.resources,
                    )
                    .with_user(|| self.options.player_name()),
                );
            }
            let skip = self.settings.file.bool("skipMultiplayerWarning");
            if let Some(title) = &mut self.title {
                title.skip_multiplayer_warning = skip;
                title.failed(error);
            }
            self.capture(false);
        }
        Ok(())
    }
    /// Join single player's integrated server through memory.
    pub(super) fn connect_local(
        &mut self,
        pipe: horizon_protocol::connection::Pipe,
        name: &str,
    ) -> Result<()> {
        self.join_server("local", name, Some(pipe), None, false)
    }
    fn join_server(
        &mut self,
        address: &str,
        name: &str,
        local: Option<horizon_protocol::connection::Pipe>,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
    ) -> Result<()> {
        self.reset_tutorial();
        self.perf.world_opening();
        // A server's own biomes come over the connection; the last local
        // world's packed table must not name its ids.
        self.world_biomes = None;
        let root = self.options.resources.clone();
        let builtin = Builtin::find(&root)
            .and_then(|p| Builtin::load(&p).ok())
            .ok_or("Resources are incomplete: registries.json is missing or invalid. Click \"Prepare again\" in the launcher (or run horizon-prepare), then try again.")?;
        let lang = Arc::new(horizon_client::net::load_language(&root));
        // Options.createInformation: what the server learns about this player,
        // sent again when one of those options changes.
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        if local.is_none() && !self.options.split_guest {
            horizon_client::crash::set_detail(
                "Server",
                horizon_client::net::shown_address(address),
            );
        }
        // A signed-in account always plays under its own name.
        let auth = horizon_client::auth::current();
        let name = auth.as_ref().map_or(name, |a| a.name.as_str());
        let user = auth.as_ref().map_or_else(
            || {
                (
                    name.to_owned(),
                    horizon_protocol::crypto::offline_uuid(name),
                )
            },
            |a| (a.name.clone(), a.uuid),
        );
        self.server_packs_connect(local.is_none().then_some(address), user);
        let session = match local {
            Some(pipe) => {
                Session::connect_local(pipe, name, info.clone(), lang.clone(), root.clone())
            }
            None => {
                self.extras_connecting(address, name, lan);
                match cookies {
                    Some(cookies) => Session::connect_transferred(
                        address,
                        name,
                        auth.clone(),
                        info.clone(),
                        lang.clone(),
                        cookies,
                        root.clone(),
                    ),
                    None => {
                        let connect = if lan {
                            Session::connect_lan
                        } else {
                            Session::connect
                        };
                        connect(
                            address,
                            name,
                            auth.clone(),
                            info.clone(),
                            lang.clone(),
                            root.clone(),
                        )
                    }
                }
            }
        };
        let player_type = super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.pack.context.clone(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.world = World::new(self.res.facts().air, 0);
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("horizon-remote-unused"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        self.inventory = vec![None; 36];
        self.armor = [None, None, None, None];
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.player = None;
        self.spawn = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.particles.clear();
        self.pickups.clear();
        self.orbs.clear();
        self.falling.clear();
        self.travelling = None;
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.clicks.miss_time = 0;
        // A new LocalPlayer has no effects: the server sends what it has
        // (ClientboundUpdateMobEffectPacket on placement).
        self.potions = Default::default();
        self.set_mode_id(0);
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        // The connected client never opens a server world (single player also
        // joins here), so load the registries its argument parsers and local
        // suggestions need before receiving the server's command tree.
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.screen = Screen::Playing;
        self.title = None;
        self.dim = Dim::Overworld;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();
        // The chat's tree until the server sends its own, or forever when it
        // sends none: the game's own, plus the commands native mods own.
        let chat_tree = self.chat_tree_for(None);
        // Minecraft.services() has the keys wherever the game is played, single player's own
        // server included: its guests' chat keys are checked against them.
        let services_keys = Arc::new(std::sync::Mutex::new(ServicesKeys::Pending));
        let slot = services_keys.clone();
        let _ = std::thread::Builder::new()
            .name("Services keys".into())
            .spawn(move || {
                use horizon_server_files::profiles;
                let keys = profiles::fetch_profile_key_keys(&profiles::discovery_url());
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = match keys {
                    Ok(keys) if !keys.is_empty() => ServicesKeys::Ready(keys),
                    Ok(_) => ServicesKeys::Missing,
                    Err(why) => {
                        log::error!("Horizon: {why}");
                        ServicesKeys::Missing
                    }
                };
            });
        self.remote = Some(Remote {
            session,
            brand: "unknown".into(),
            quiet_commands: false,
            mod_net: false,
            builtin: Arc::new(builtin),
            status: lang
                .get("connect.connecting")
                .cloned()
                .unwrap_or_else(|| "Connecting to the server...".into()),
            lang,
            registries: Arc::new(Dynamic::default()),
            joined: false,
            reconfiguring: None,
            logged_in: false,
            entity_id: 0,
            camera: None,
            use_item: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            carried: None,
            abilities: None,
            loaded_sent: false,
            level_load: horizon_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            predictions: HashMap::new(),
            menu: None,
            menu_title: None,
            state_id: 0,
            use_target: None,
            entities: HashMap::new(),
            lerps: HashMap::new(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            chat_tree_received: false,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            listed: Default::default(),
            online_mode: false,
            level_players: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: String::new(),
            is_destroying: false,
            destroy_delay: 0,
            input: Default::default(),
            pending_keys: Vec::new(),
            swap_answer: [false; 2],
            stop_mining: false,
            placing: None,
            flat: false,
            death_message: None,
            show_death_screen: true,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clock: None,
            clock_rate: 1.0,
            clock_partial: 0.0,
            clock_game_time: 0,
            chunk_center: (0, 0),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            transfer: None,
        });
        // The level's TickRateManager follows the server's ticking packets.
        self.clock = horizon_client::clock::FixedStep::client();
        // The player's advancements and statistics are the server's.
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }

    /// Disconnect and return to the multiplayer screen (DisconnectedScreen).
    pub(super) fn leave_server(&mut self, reason: Option<String>) -> Result<()> {
        if reason.is_none() && self.integrated.as_ref().is_some_and(|s| !s.stopped()) {
            return self.begin_closing_save(false);
        }
        self.finish_perf();
        if reason.is_none() {
            // Leaving on purpose: nothing for auto-reconnect to retry.
            self.extras_left();
        }
        self.clear_waypoints();
        self.hud_disconnected();
        // Vanilla's Hud also ticks on the title and connect screens, so the
        // action bar has run out by the next world; Horizon's ticks in one.
        self.titles.overlay_time = 0;
        // GameRenderer.resetData and narrator.clear (Minecraft.disconnect).
        self.item_activation.reset();
        // Kept mod block entity geometry belongs to the world leaving.
        self.kept_block_entities.get_mut().clear();
        self.clear_narrator();
        // A world is leaving either way: its icon attempt goes with it.
        self.world_icon = None;
        if let Some(mut r) = self.remote.take() {
            r.session.disconnect();
        }
        super::lan::forget_sync_data();
        self.gui.atlas.object_skins.clear();
        self.server_packs_disconnect();
        self.code_of_conduct = None;
        self.reset_tutorial();
        self.close_dialogs();
        // Gui.setScreen -> KeyMapping.releaseAll: the title or disconnect
        // screen releases every held key. Its release goes to that screen,
        // so a key kept here would still be down in the next world (#3246).
        self.keys.clear();
        self.left = false;
        self.right = false;
        // The shared world is closing: its invite code ends with it.
        self.friends_lan_closed();
        // Single player's server thread saves the world before the title shows.
        let single_player = match self.integrated.take() {
            Some(server) => {
                server.stop()?;
                true
            }
            None => false,
        };
        // A splitscreen player leaving says nothing of where P1 plays.
        if !self.options.split_guest {
            horizon_client::crash::clear_detail("Server");
        }
        self.clock = Default::default();
        self.adv = Default::default();
        self.stats = Default::default();
        self.score.board = Default::default();
        self.streaming = Streaming::new(Arc::new(NoTerrain), self.pack.context.clone(), 1);
        self.streaming.paused = true;
        self.world = World::new(self.res.facts().air, 0);
        self.player = None;
        // The vehicle went with the level: an id kept here would seat the
        // next login on an entity that is not there.
        self.riding = None;
        // Out of the server's bed: InBedChatScreen goes with the level.
        self.sleeping = None;
        self.sleep_timer = 0;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.dropped.clear();
        self.particles.clear();
        self.pickups.clear();
        self.score.board = Default::default();
        self.tab_list = Default::default();
        self.border = Default::default();
        self.screen = Screen::Playing;
        self.container = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        let saves = std::env::current_dir()?.join("saves");
        let mut title = horizon_client::title::Title::new(saves.clone(), &self.options.resources)
            .with_user(|| self.options.player_name());
        title.last_server = self.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        if single_player && reason.is_none() {
            self.title = Some(title);
            self.capture(false);
            return Ok(());
        }
        title.mode = horizon_client::title::Mode::Multiplayer(match reason {
            Some(reason) => horizon_client::multiplayer::Screen::Disconnected {
                reason,
                retry: None,
            },
            None => horizon_client::multiplayer::Screen::list(&saves),
        });
        self.title = Some(title);
        self.capture(false);
        Ok(())
    }
    /// `Hud.onDisconnected`, from `Minecraft.disconnect` and `clearClientLevel`
    /// (a proxy's reconfiguration too), but the chat: joining clears it and a
    /// reconfiguration keeps it (`ChatComponent.storeState`).
    fn hud_disconnected(&mut self) {
        self.tab_list.reset();
        self.boss_overlay.reset();
        self.adv.toasts.clear();
        self.debug_charts.reset();
        self.titles.clear();
        self.titles.reset_times();
    }

    pub(super) fn send(&self, packet: S) {
        if let Some(r) = &self.remote {
            r.session.send(packet);
        }
    }
    fn next_sequence(&mut self) -> i32 {
        let r = self.remote.as_mut().expect("remote");
        r.sequence += 1;
        r.sequence
    }
    fn item_name(&self, id: i32) -> Arc<str> {
        let r = self.remote.as_ref().expect("remote");
        Arc::from(
            r.builtin
                .get("minecraft:item")
                .and_then(|i| i.name(id))
                .unwrap_or("minecraft:air"),
        )
    }
    /// The server's network id of a mob effect (`brewing::EFFECTS` id).
    pub(super) fn remote_effect_network_id(&self, effect: &str) -> Option<i32> {
        let r = self.remote.as_ref()?;
        r.builtin
            .get("minecraft:mob_effect")?
            .id(&format!("minecraft:{effect}"))
    }
    /// A mob effect's network id as the effect (`brewing::EFFECTS` id).
    pub(super) fn remote_effect_id(&self, effect: i32) -> Option<&'static str> {
        let r = self.remote.as_ref()?;
        let name = r.builtin.get("minecraft:mob_effect")?.name(effect)?;
        horizon_client::brewing::effect(name).map(|d| d.id)
    }
    /// ClientPacketListener.handleUpdateMobEffect for the player itself
    /// (`LivingEntity.forceAddEffect`): the effect replaces any of its kind.
    /// Other entities' effects are not shown.
    fn remote_effect(&mut self, p: &play::UpdateMobEffect) {
        if self.remote.as_ref().is_none_or(|r| r.entity_id != p.id) {
            return;
        }
        let Some(id) = self.remote_effect_id(p.effect) else {
            return;
        };
        let mut e = horizon_client::effects::EffectInstance::new(id, p.duration, p.amplifier);
        e.ambient = p.flags & 1 != 0;
        e.visible = p.flags & 2 != 0;
        let list = &mut self.potions.effects.list;
        match list.iter_mut().find(|x| x.id == id) {
            // forceAddEffect's copyBlendState: an updated effect keeps its fade.
            Some(existing) => *existing = e,
            None => {
                // ClientboundUpdateMobEffectPacket.shouldBlend (flag 8).
                let blend = p.flags & 8 != 0;
                if id == "darkness" {
                    self.sculk.darkness = super::sculk::darkness_blend_added(&e, blend);
                } else if id == "nausea" {
                    let (o_nausea, nausea) = screen_effects::nausea_blend_added(&e, blend);
                    self.screen_fx.o_nausea = o_nausea;
                    self.screen_fx.nausea = nausea;
                }
                list.push(e);
            }
        }
    }
    fn item_id(&self, name: &str) -> i32 {
        let r = self.remote.as_ref().expect("remote");
        r.builtin
            .get("minecraft:item")
            .and_then(|i| i.id(name))
            .unwrap_or(0)
    }
    /// A stack from the server, decoded as the saved and command item NBT
    /// are, so tooltips and models see every field.
    pub(super) fn to_stack(&self, s: &Option<Stack>) -> Option<ItemStack> {
        let r = self.remote.as_ref()?;
        let stack = s.as_ref()?;
        if !horizon_protocol::item::custom::permitted(stack, r.registries.custom_components)
            .unwrap_or(false)
        {
            log::warn!("Dropping a stack with unnegotiated custom components");
            return None;
        }
        Some(horizon_client::item_sync::from_network(
            stack,
            &r.builtin,
            &r.registries,
        ))
    }
    /// A stack to the server (`SetCreativeModeSlot`): the same component
    /// patch a Horizon server sends its clients, with the registries' ids, so
    /// the server's echo doesn't hand back a bare item.
    fn to_network(&self, s: &Option<ItemStack>) -> Option<Stack> {
        let r = self.remote.as_ref()?;
        // A stack as the server sent it, only its count changed: its own
        // components go back, as a vanilla client keeps them.
        if let Some((stack, patch)) = s.as_ref().and_then(|s| Some((s, s.wire_patch()?))) {
            let mut out = Stack::new(self.item_id(&stack.item), stack.count as i32);
            let safe = r.registries.custom_components
                || horizon_protocol::item::custom::entries().next().is_none()
                || horizon_protocol::item::WirePatch::decode(&patch)
                    .and_then(|(added, removed)| {
                        let decoded = Stack {
                            added,
                            removed,
                            ..Stack::new(out.item, out.count)
                        };
                        horizon_protocol::item::custom::permitted(&decoded, false)
                    })
                    .unwrap_or(false);
            if safe {
                out.wire = horizon_protocol::item::WirePatch(Some(patch));
                return Some(out);
            }
            log::warn!("Cannot send cached custom components to this peer");
        }
        horizon_client::lan::guest::stack_to_network(
            |name| self.item_id(name),
            // Built-in first, as a hub's `SyncData::id` does: potions are one.
            |registry, name| {
                r.builtin
                    .get(registry)
                    .and_then(|ids| ids.id(name))
                    .or_else(|| r.registries.id(registry, name))
                    .or_else(|| {
                        (r.registries.custom_components
                            && registry == "minecraft:data_component_type")
                            .then(|| horizon_protocol::item::custom::id(name))
                            .flatten()
                    })
            },
            s,
        )
    }
    /// `HashedStack.create` with the connection's registries (the client's
    /// `decoratedHashOpsGenerator`).
    fn hashed_stack(&self, s: &Option<ItemStack>) -> Option<Hashed> {
        let r = self.remote.as_ref()?;
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
        self.to_network(s).map(|s| Hashed::create(&s, &names))
    }
    /// A line in the connection's chat, with its styles, click and hover
    /// events (ChatComponent.addMessage). The lines the Chat option hides
    /// are dropped there (its `visibleMessageFilter`).
    pub(super) fn chat_line(
        &mut self,
        content: horizon_client::text::Text,
        source: horizon_client::chat::ChatSource,
    ) -> bool {
        self.signed_chat_line(content, source, None, ChatTrust::Secure)
    }
    /// `chat_line` of a player message, keeping its signature for a
    /// deletion (`GuiMessage.signature`).
    fn signed_chat_line(
        &mut self,
        content: horizon_client::text::Text,
        source: horizon_client::chat::ChatSource,
        signature: Option<Signature>,
        trust: ChatTrust,
    ) -> bool {
        // ChatListener.showMessageToPlayer: Only Show Secure Chat drops what is not secure.
        if matches!(trust, ChatTrust::NotSecure) && self.settings.file.bool("onlyShowSecureChat") {
            return false;
        }
        if !self.ui.chat.shows(source) {
            return false;
        }
        let text = content.plain();
        let focused = self.screen == Screen::Chat;
        if source == horizon_client::chat::ChatSource::Player {
            self.ui.chat.add_signed(
                &self.gui.atlas,
                content,
                signature,
                trust.tag(),
                self.ticks,
                focused,
            );
        } else {
            // addServerSystemMessage and addClientSystemMessage both carry
            // GuiMessageTag.systemSinglePlayer: the grey bar at the line's left.
            let indicator = Some(horizon_client::chat::SYSTEM_INDICATOR);
            self.ui.chat.add(
                &self.gui.atlas,
                content,
                indicator,
                source,
                self.ticks,
                focused,
            );
        }
        if let Some(r) = self.remote.as_mut() {
            r.chat.push((text, Instant::now()));
            r.chat_received += 1;
            if r.chat.len() > 100 {
                r.chat.remove(0);
            }
        }
        true
    }

    /// `ChatListener.handleMessage` for player and disguised chat: held in the
    /// queue while Chat Delay is set, shown at once otherwise. `narration` is
    /// the chat type's narration of it (`decorateNarration`).
    pub(super) fn handle_player_chat(
        &mut self,
        line: horizon_client::text::Text,
        signature: Option<Signature>,
        trust: ChatTrust,
        narration: String,
    ) {
        let source = horizon_client::chat::ChatSource::Player;
        if self.chat_delay_ms() > 0 {
            if let Some(r) = self.remote.as_mut() {
                r.chat_queue
                    .push_back((line, source, signature, trust, narration));
            }
        } else {
            self.accept_chat(line, source, signature, trust, &narration);
        }
    }

    /// `ChatListener.handleChatMessageError`: a message that fails validation shows as the red,
    /// italic `chat.validation_error` in its chat type (queued under Chat Delay as any
    /// message). Its signature counts as processed but not shown, for the acknowledgement the
    /// client sends the server. Vanilla also
    /// skips a blocked sender and one restricted to friends (not kept here either) and
    /// narrates the line.
    fn handle_chat_message_error(
        &mut self,
        chat_type: &play::ChatTypeBound,
        signature: Option<&Signature>,
    ) {
        if let Some(signature) = signature {
            self.mark_message_processed(signature, false);
        }
        let error = serde_json::json!({
            "translate": "chat.validation_error",
            "color": "red",
            "italic": true,
        });
        let line = self.decorate_chat(chat_type, &error, Decoration::Chat);
        // saySystemChatQueued(boundChatType.decorateNarration(CHAT_VALIDATION_ERROR)).
        let narration = self
            .decorate_chat(chat_type, &error, Decoration::Narration)
            .plain();
        self.handle_player_chat(line, None, ChatTrust::Error, narration);
    }

    /// `ChatTrustLevel.evaluate` of a player message, with
    /// `ChatListener.evaluateTrustLevel`: the player's own message on their
    /// own server is secure. `decorated` is the line shown, `unsigned` the
    /// server's replacement for the signed content.
    fn chat_trust(
        &self,
        sender: u128,
        signed: bool,
        timestamp: i64,
        content: &str,
        unsigned: Option<&Tag>,
        decorated: &horizon_client::text::Text,
    ) -> ChatTrust {
        use horizon_client::scoreboard::{comp_text, tag_to_json};
        if self.integrated.is_some() && self.remote.as_ref().is_some_and(|r| r.profile == sender) {
            return ChatTrust::Secure;
        }
        // PlayerChatMessage.hasExpiredClient: MESSAGE_EXPIRES_AFTER_CLIENT is
        // the server's five minutes and two more.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let expired = now
            > timestamp.saturating_add(
                horizon_protocol::secure_chat::EXPIRES_AFTER_SERVER + 2 * 60 * 1000,
            );
        if !signed || expired {
            return ChatTrust::NotSecure;
        }
        // ChatTrustLevel.isModified: the signed content is not in the line, or
        // the replacement uses a font other than the default.
        let font_changed = unsigned.is_some_and(|tag| {
            comp_text(&tag_to_json(tag)).spans.iter().any(|span| {
                span.style
                    .font
                    .as_deref()
                    .is_some_and(|font| font != "minecraft:default")
            })
        });
        if !decorated.plain().contains(content) || font_changed {
            ChatTrust::Modified(content.to_owned())
        } else {
            ChatTrust::Secure
        }
    }

    /// `ClientPacketListener.handleDeleteChat`: the packed signature names a
    /// message; one still held back by Chat Delay never shows
    /// (`ChatListener.removeFromDelayedMessageQueue`), a shown one is
    /// replaced (`ChatComponent.deleteMessage`). A signature the cache
    /// doesn't hold is an invalid packet.
    fn handle_delete_chat(&mut self, packed: &(i32, Option<Vec<u8>>)) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let Some(signature) = r.signatures.unpack(packed) else {
            let reason =
                horizon_client::text::Text::translate("multiplayer.disconnect.invalid_packet", &[]);
            return self.leave_server(Some(reason.plain()));
        };
        r.secure_chat.ignore_pending(&signature);
        let queued = r.chat_queue.len();
        r.chat_queue
            .retain(|(_, _, s, _, _)| s.as_ref() != Some(&signature));
        if r.chat_queue.len() == queued {
            self.ui
                .chat
                .delete_message(&self.gui.atlas, &signature, self.ticks);
        }
        Ok(())
    }

    /// A message's `accept`: shown, it is narrated (`narrateChatMessage`) and
    /// starts the Chat Delay wait again.
    fn accept_chat(
        &mut self,
        line: horizon_client::text::Text,
        source: horizon_client::chat::ChatSource,
        signature: Option<Signature>,
        trust: ChatTrust,
        narration: &str,
    ) -> bool {
        let shown = self.player_chat_line(line, source, signature, trust, narration);
        if let Some(signature) = &signature {
            self.mark_message_processed(signature, shown);
        }
        if shown && let Some(r) = self.remote.as_mut() {
            r.chat_previous = Some(Instant::now());
        }
        shown
    }

    /// `showMessageToPlayer`: the line joins the chat if the Chat option and
    /// Only Show Secure Chat let it through, and the narrator says it
    /// (`sayChatQueued`).
    fn player_chat_line(
        &mut self,
        line: horizon_client::text::Text,
        source: horizon_client::chat::ChatSource,
        signature: Option<Signature>,
        trust: ChatTrust,
        narration: &str,
    ) -> bool {
        // handleChatMessageError says its line as system chat.
        let error = matches!(trust, ChatTrust::Error);
        let shown = self.signed_chat_line(line, source, signature, trust);
        if shown && error {
            self.say_system_chat_queued(&horizon_client::text::Text::literal(narration));
        } else if shown {
            self.say_chat_queued(narration);
        }
        shown
    }

    /// `ChatListener.messageDelay`: Chat Delay in milliseconds, as Java
    /// truncates it.
    fn chat_delay_ms(&self) -> i64 {
        (self.settings.chat_delay * 1000.0) as i64
    }

    /// `ChatListener.tick`: while paused the wait does not run down; with no
    /// Chat Delay the queue flushes; otherwise the next message the chat takes
    /// shows once the delay has passed since the last shown one.
    pub(super) fn tick_chat_queue(&mut self) {
        let delay = self.chat_delay_ms();
        // Minecraft.isPaused: a single-player pausing screen, not every pause menu.
        if self.pause {
            if delay > 0
                && let Some(r) = self.remote.as_mut()
            {
                r.chat_previous = r.chat_previous.map(|t| t + Duration::from_millis(50));
            }
            return;
        }
        if delay == 0 {
            if self
                .remote
                .as_ref()
                .is_some_and(|r| !r.chat_queue.is_empty())
            {
                self.flush_chat_queue();
            }
            return;
        }
        let ready = self.remote.as_ref().is_some_and(|r| {
            r.chat_previous
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(delay as u64))
        });
        if !ready {
            return;
        }
        // A message the Chat option hides is dropped and the next one is tried,
        // until one shows.
        while let Some((line, source, signature, trust, narration)) =
            self.remote.as_mut().and_then(|r| r.chat_queue.pop_front())
        {
            if self.accept_chat(line, source, signature, trust, &narration) {
                break;
            }
        }
    }

    /// `ChatListener.flushQueue`: every held-back message shows now, and the
    /// delay starts over.
    fn flush_chat_queue(&mut self) {
        let Some(queue) = self
            .remote
            .as_mut()
            .map(|r| std::mem::take(&mut r.chat_queue))
        else {
            return;
        };
        for (line, source, signature, trust, narration) in queue {
            self.accept_chat(line, source, signature, trust, &narration);
        }
        if let Some(r) = self.remote.as_mut() {
            r.chat_previous = None;
        }
    }

    /// The minecarts the server sent, for the control socket's `status`
    /// (their velocity is not tracked here).
    pub(super) fn remote_carts(&self) -> Option<Vec<serde_json::Value>> {
        let r = self.remote.as_ref()?;
        let mut carts: Vec<&Tracked> = r
            .entities
            .values()
            .filter(|t| t.kind.ends_with("minecart"))
            .collect();
        carts.sort_by(|a, b| {
            a.position
                .partial_cmp(&b.position)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Some(
            carts
                .into_iter()
                .map(|t| serde_json::json!({"kind": t.kind, "position": t.position, "velocity": [0.0, 0.0, 0.0]}))
                .collect(),
        )
    }

    /// `ClientSuggestionProvider.customCompletionSuggestions` of the server
    /// connection (none without one).
    pub(super) fn remote_custom_completions(&self) -> std::collections::BTreeSet<String> {
        self.remote
            .as_ref()
            .map(|r| r.custom_completions.clone())
            .unwrap_or_default()
    }
    /// The names in the server's player list, sorted.
    pub(super) fn remote_online_names(&self) -> Option<Vec<String>> {
        let r = self.remote.as_ref()?;
        let mut names: Vec<String> = r.players.values().cloned().collect();
        names.sort();
        Some(names)
    }
    /// The open menu's window id (0: the inventory).
    pub(super) fn remote_menu_id(&self) -> i32 {
        self.remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0)
    }
    /// The server's last `n` chat lines, newest first.
    pub(super) fn remote_chat(&self, n: usize) -> Option<Vec<String>> {
        let r = self.remote.as_ref()?;
        Some(r.chat.iter().rev().take(n).map(|l| l.0.clone()).collect())
    }

    /// Received events left for the next frame (`poll_remote`): until they
    /// are handled, the terrain the server sent is not all there.
    pub(super) fn remote_backlog(&self) -> usize {
        self.remote.as_ref().map_or(0, |r| r.backlog.len())
    }
    /// A headless command oracle has a real connection but no movement tick.
    /// Acknowledge loaded terrain without predicting or sending movement.
    pub(super) fn poll_stationary_client(&mut self) -> Result<()> {
        self.poll_remote()?;
        let loaded = self.player.as_ref().is_some_and(|p| {
            self.streaming.is_lit(
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            )
        });
        if loaded && self.remote.as_ref().is_some_and(|r| !r.loaded_sent) {
            self.send(S::PlayerLoaded(Empty));
            let r = self.remote.as_mut().unwrap();
            r.loaded_sent = true;
            r.level_load.finish();
        }
        Ok(())
    }

    /// Process what the network thread received (once per frame, like the
    /// client's packet processor), in order. Chunk packets get
    /// `CHUNK_PACKETS_PER_FRAME`: once a frame has spent it, the events from
    /// the next chunk packet on wait for the next frame, unless they have
    /// already waited `BACKLOG_LIMIT`. A chunk packet still decoding holds
    /// back what follows it.
    pub(super) fn poll_remote(&mut self) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let mut events = std::mem::take(&mut r.backlog);
        // Chunk packets start decoding now; the frame that reaches one in
        // order inserts its column.
        for event in r.session.poll() {
            events.push_back(match event {
                Event::Packet(p) if matches!(*p, C::LevelChunkWithLight(_)) => {
                    match r.decoder.submit(r.chunk_format.clone(), p) {
                        None => Received::Chunk(r.chunk_format.clone()),
                        Some(p) => Received::Event(Event::Packet(p)),
                    }
                }
                event => Received::Event(event),
            });
        }
        let overdue = r
            .backlog_since
            .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
        let mut chunks = Duration::ZERO;
        while let Some(item) = events.pop_front() {
            let chunk = matches!(item, Received::Chunk(_));
            if chunk && !overdue && chunks >= CHUNK_PACKETS_PER_FRAME {
                events.push_front(item);
                break;
            }
            let started = chunk.then(Instant::now);
            let event = match item {
                Received::Event(event) => event,
                Received::Chunk(format) => {
                    // A column still decoding waits for the next frame, with
                    // what came after it, overdue or not.
                    let Some(next) = self.remote.as_ref().map(|r| r.decoder.next()) else {
                        break;
                    };
                    let (packet, column) = match next {
                        Next::Ready(decoded) => decoded,
                        Next::Decoding => {
                            events.push_front(Received::Chunk(format));
                            break;
                        }
                        Next::Lost => {
                            log::warn!("Horizon: the chunk decoder stopped; a column is missing");
                            continue;
                        }
                    };
                    if let C::LevelChunkWithLight(p) = *packet {
                        self.remote_chunk(&p, Some((&format, column)));
                    }
                    chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
                    continue;
                }
            };
            match event {
                Event::Status(s) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.status = s;
                    }
                }
                Event::Joined {
                    registries,
                    name,
                    uuid,
                } => {
                    self.set_registries(registries);
                    self.extras_joined();
                    // handleLogin: the level loading screen replaces any dialog.
                    self.close_dialogs();
                    if let Some(r) = self.remote.as_mut() {
                        r.joined = true;
                        // handleLogin: startWaitingForNewLevel with Reason.OTHER.
                        r.profile = uuid;
                        r.profile_name = name.clone();
                        // LevelLoadingScreen's line: no player name.
                        r.status = horizon_client::lang::get_or(
                            "multiplayer.downloadingTerrain",
                            "Loading terrain...",
                        );
                    }
                }
                Event::Brand(brand) => {
                    if let Some(remote) = &mut self.remote {
                        remote.brand = brand;
                    }
                }
                Event::QuietCommands => {
                    if let Some(remote) = &mut self.remote {
                        remote.quiet_commands = true;
                    }
                }
                Event::ModNet => {
                    if let Some(remote) = &mut self.remote {
                        remote.mod_net = true;
                    }
                }
                Event::Reconfiguring => {
                    // handleConfigurationStart: the held-back chat shows, then
                    // clearClientLevel opens ServerReconfigScreen over no level.
                    self.flush_chat_queue();
                    self.send_chat_acknowledgement();
                    self.close_dialogs();
                    self.player = None;
                    self.screen = Screen::Playing;
                    self.capture(false);
                    let ticks = self.ticks;
                    if let Some(r) = self.remote.as_mut() {
                        r.reconfiguring = Some(ticks);
                        r.chat_tree_received = false;
                    }
                }
                Event::Reconfigured(registries) => {
                    // handleConfigurationStart → clearClientLevel, and the next
                    // ClientPacketListener starts with no players or scoreboard,
                    // no waypoints (its own ClientWaypointManager: a proxy's next
                    // server never untracks the last one's), and its new
                    // ClientSuggestionProvider with no custom chat completions.
                    // clearClientLevel clears the narrator.
                    self.clear_narrator();
                    self.hud_disconnected();
                    self.score.board = Default::default();
                    self.clear_waypoints();
                    if let Some(r) = self.remote.as_mut() {
                        r.players.clear();
                        r.tab_players.clear();
                        r.listed.clear();
                        r.signatures = Default::default();
                        r.custom_completions.clear();
                    }
                    self.ui.registries.custom_completions.clear();
                    self.set_registries(registries);
                }
                // handleTransfer: "Cannot transfer to server from
                // singleplayer" (no ServerData), so the world just closes.
                Event::Transfer { host, port } => {
                    if let Some(r) = self.remote.as_mut()
                        && r.session.address != "local"
                    {
                        r.transfer = Some(format!("{host}:{port}"));
                    }
                }
                Event::PackPush(p) => self.handle_pack_push(p),
                Event::PackPop(id) => self.handle_pack_pop(id),
                Event::ServerLinks(links) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.server_links = super::server_links::trusted(links);
                    }
                }
                Event::ShowDialog(dialog) => self.show_dialog(&dialog),
                Event::ClearDialog => self.clear_dialog(),
                Event::CodeOfConduct(text) => self.handle_code_of_conduct(text),
                Event::DecodeError(e) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.decode_errors += 1;
                        if r.decode_errors <= 20 {
                            log::warn!("Horizon: skipped undecodable packet {e}");
                        }
                    }
                }
                Event::Disconnected {
                    reason,
                    key,
                    from_server,
                } => {
                    log::info!("Horizon: disconnected: {reason}");
                    // ClientboundTransferPacket left a target: connect there
                    // instead of showing the disconnect, the stored cookies
                    // carried over (ClientCommonPacketListenerImpl.
                    // handleTransfer → ConnectScreen.startConnecting).
                    let transfer = self.remote.as_mut().and_then(|r| {
                        r.transfer.take().map(|address| {
                            (address, r.session.name.clone(), r.session.cookies.clone())
                        })
                    });
                    if let Some((address, name, cookies)) = transfer {
                        self.leave_server(None)?;
                        self.connect_server_with_cookies(&address, &name, Some(cookies))?;
                        return Ok(());
                    }
                    let retry = self.reconnect_after(&reason, key.as_deref(), from_server);
                    self.leave_server(Some(reason))?;
                    self.offer_reconnect(retry);
                    return Ok(());
                }
                Event::Packet(packet) => self.handle_packet(*packet)?,
            }
            chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.remote.is_none() {
                break;
            }
        }
        if let Some(r) = self.remote.as_mut() {
            r.backlog_since = if events.is_empty() {
                None
            } else {
                Some(r.backlog_since.unwrap_or_else(Instant::now))
            };
            r.backlog = events;
        }
        self.send_suggestion_request();
        Ok(())
    }
    /// ClientSuggestionProvider.customSuggestion: the chat's completion
    /// request for an `ask_server` node goes to the server.
    fn send_suggestion_request(&mut self) {
        if self.remote.is_none() {
            return;
        }
        let Some((id, command)) = self
            .ui
            .chat_screen
            .as_mut()
            .and_then(|s| s.take_request())
            .or_else(|| self.command_block_suggestion_request())
        else {
            return;
        };
        let mut w = horizon_protocol::buf::Writer::new();
        w.varint(id);
        w.string(&command);
        self.send(S::Unknown(
            horizon_protocol::ids::play::serverbound::COMMAND_SUGGESTION,
            w.data,
        ));
    }
    /// ClientboundCommandSuggestionsPacket: id, range start and length, then
    /// each suggestion's text and optional tooltip.
    fn handle_command_suggestions(&mut self, data: &[u8]) {
        let lang = self
            .remote
            .as_ref()
            .map(|r| r.lang.clone())
            .unwrap_or_default();
        let mut r = horizon_protocol::buf::Reader::new(data);
        let decoded = (|| {
            let id = r.varint()?;
            let start = r.varint()?;
            let length = r.varint()?;
            let n = r.count(usize::MAX)?;
            let mut list = Vec::with_capacity(n.min(1000));
            for _ in 0..n {
                let text = r.string(32767)?;
                let tooltip = if r.bool()? {
                    Some(plain(&r.nbt()?, &lang))
                } else {
                    None
                };
                list.push((text, tooltip));
            }
            horizon_protocol::Result::Ok((id, start, length, list))
        })();
        if let Ok((id, start, length, list)) = decoded {
            let range = (start.max(0) as usize, length.max(0) as usize);
            self.chat_server_suggestions(id, range, list.clone());
            self.command_block_server_suggestions(id, range, list);
        }
    }
    fn set_registries(&mut self, registries: Arc<Dynamic>) {
        let biomes: Vec<u16> = registries
            .registries
            .get("minecraft:worldgen/biome")
            .map(|list| {
                list.iter()
                    .map(|(name, _)| world_biomes!(self).id(name))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(r) = self.remote.as_mut() {
            let bits = ceil_log2(biomes.len());
            let column = r.chunk_format.layout();
            r.chunk_format =
                Arc::new(ChunkFormat::new(&r.dimension, biomes, bits).in_column(column));
            r.registries = registries;
        }
    }
    /// Switch to a (possibly new) dimension: a fresh client level.
    fn enter_remote_dimension(
        &mut self,
        spawn: &play::SpawnInfo,
        keep_world: bool,
        reason: horizon_client::level_loading::Reason,
    ) {
        // The replacement player starts with its inventory menu, even if the
        // server switched levels without closing the previous container.
        self.remote_close_local();
        self.screen = Screen::Playing;
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let dimension = r.registries.dimension(spawn.dimension_type);
        // The server's dimension type gives the level's shape, and its
        // chunks the shape's column.
        let shape = horizon_client::chunk::LevelShape::new(
            dimension.min_y,
            dimension.height,
            dimension.logical_height,
            dimension.has_skylight,
        )
        .unwrap_or_else(|_| super::Dim::from_id(&dimension.name).shape());
        let same = r.dimension.name == dimension.name && keep_world;
        r.chunk_format = Arc::new(
            r.chunk_format
                .with_dimension(&dimension)
                .in_column(shape.column()),
        );
        r.dimension = dimension;
        r.game_mode = spawn.game_type;
        r.flat = spawn.flat;
        r.loaded_sent = false;
        // A replacement LevelLoadingScreen starts its bar from nothing (the
        // first one carries on from the server's start).
        if self.player.is_some() {
            self.loading_progress = 0.0;
        }
        // startWaitingForNewLevel: `LevelLoadTracker.startClientLoad` and the
        // `LevelLoadingScreen`.
        r.level_load.start_client_load(client_millis(), reason);
        // A replacement LocalPlayer has a fresh sendPosition baseline.
        r.position_sender = Default::default();
        r.predictions.clear();
        self.set_mode_id(spawn.game_type);
        self.dim = Dim::from_id(&spawn.dimension);
        self.audio.set_world(&spawn.dimension);
        // ClientPacketListener.handleLogin / handleRespawn: the server's
        // Player.getLastDeathLocation, for the recovery compass.
        self.level.last_death = spawn
            .last_death
            .as_ref()
            .map(|d| (d.dimension.clone(), d.pos));
        if !same {
            // A new ClientLevel: Minecraft.setLevel stops every sound.
            self.stop_sounds_for_new_level();
            let mut streaming = Streaming::new(
                Arc::new(NoTerrain),
                self.pack.context.clone(),
                self.res.threads(),
            );
            streaming.remote = true;
            streaming.radius = self.streaming.radius;
            self.streaming = streaming;
            let (time, day, weather) = (
                self.world.game_time,
                self.world.day_time,
                self.world.weather,
            );
            let old = std::mem::take(&mut self.world.chunks);
            let count = old.len();
            horizon_client::chunk::free_columns(old, count);
            self.world = World::new(self.res.facts().air, spawn.seed);
            self.world.shape = shape;
            self.world.game_time = time;
            self.world.day_time = day;
            self.world.weather = weather;
            self.mobs.clear();
            self.clear_remote_objects();
            self.clear_remote_projectiles();
            self.dropped.clear();
            self.particles.clear();
            self.pickups.clear();
            if let Some(r) = self.remote.as_mut() {
                r.entities.clear();
                r.lerps.clear();
                r.cart_lerps.clear();
                r.stepped.clear();
                r.head_lerps.clear();
                r.pending_items.clear();
                // The entity ids are the old world's; the new dimension's
                // player entities re-map to their profiles on AddEntity.
                r.level_players.clear();
                r.level_player_names.clear();
                r.level_player_textures.clear();
                // A new ClientLevel: a new chunk storage, centred on 0, 0.
                r.chunk_center = (0, 0);
            }
            self.clear_lod();
            self.pending_uploads.clear();
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.clear_sections(self.world.layout());
            }
        }
        let has_sky = self
            .remote
            .as_ref()
            .is_none_or(|r| r.dimension.has_skylight);
        self.streaming.sky_light = has_sky;
        self.streaming.nether_shading = self.dim == Dim::Nether;
        // The seed in SpawnInfo is already the hashed one the client's BiomeManager holds.
        self.streaming.biome_zoom = horizon_core::biome::BiomeZoom::new(spawn.seed);
        self.player = None;
        // handleLogin / handleRespawn: LocalPlayer.setPortalCooldown, so a
        // player who arrives standing in a portal gets no overlay from it.
        self.portals.player.cooldown = spawn.portal_cooldown;
        self.apply_settings();
    }

    /// `ClientChunkCache.Storage.inRange`: within the storage around the
    /// view centre, `calculateStorageRange` (the server's radius, at least
    /// two, plus three) chunks each way.
    pub(super) fn remote_chunk_in_range(&self, x: i32, z: i32) -> bool {
        self.remote.as_ref().is_none_or(|r| {
            let range = r.view_distance.max(2) + 3;
            (x - r.chunk_center.0).abs() <= range && (z - r.chunk_center.1).abs() <= range
        })
    }
    /// The columns a moved or resized storage no longer holds.
    fn drop_remote_chunks_out_of_range(&mut self) {
        let gone: horizon_client::fast_hash::FxHashSet<(i32, i32)> = self
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| !self.remote_chunk_in_range(x, z))
            .collect();
        if gone.is_empty() {
            return;
        }
        let mut freed = Vec::with_capacity(gone.len());
        for &(x, z) in &gone {
            freed.extend(self.streaming.forget(&mut self.world, x, z));
        }
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.remove_columns(gone.iter().copied());
        }
        horizon_client::chunk::free_columns(freed, gone.len());
        // One pass for all of them.
        self.world
            .block_entities
            .retain(|k, _| !gone.contains(&(k.0 >> 4, k.2 >> 4)));
    }
    /// `handleBlockEntityData`: the block entity at `pos`, when it is of the
    /// packet's type, loads the tag (`loadWithComponents`); an empty tag
    /// changes nothing.
    pub(super) fn remote_block_entity_data(&mut self, pos: [i32; 3], kind: i32, tag: &Tag) {
        let key = (pos[0], pos[1], pos[2]);
        let Some(name) = self
            .remote
            .as_ref()
            .and_then(|r| r.builtin.get("minecraft:block_entity_type")?.name(kind))
        else {
            return;
        };
        // A decorated pot is no block entity here: its tag is kept with the chunk.
        if name == "minecraft:decorated_pot" {
            if let Tag::Compound(fields) = tag
                && !fields.is_empty()
            {
                let mut full = fields.clone();
                full.insert("id".into(), Tag::String(name.into()));
                for (k, v) in ["x", "y", "z"].into_iter().zip(pos) {
                    full.insert(k.into(), Tag::Int(v));
                }
                super::spill::put_saved_entity(&mut self.world, pos, Tag::Compound(full));
                // The section is drawn again with the pot's sherds.
                let state = self.world.block(pos[0], pos[1], pos[2]);
                let _ = self.world.set(pos[0], pos[1], pos[2], state);
            }
            return;
        }
        let Tag::Compound(fields) = tag else {
            return;
        };
        // A mod block entity's type is named after its block (`mod_content`):
        // its update tag merges into what the client keeps of it (`handleUpdateTag`).
        // Nothing is drawn from it yet, so its section stays as it is.
        let block = self.world.block(pos[0], pos[1], pos[2]);
        if let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) {
            if *self.res.facts().get(block).name == *name {
                m.nbt
                    .extend(fields.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            return;
        }
        let Some(entity) = self.world.block_entities.get(&key) else {
            return;
        };
        if entity.id() != name || fields.is_empty() {
            return;
        }
        let mut full = fields.clone();
        full.insert("id".into(), Tag::String(name.into()));
        for (k, v) in ["x", "y", "z"].into_iter().zip(pos) {
            full.insert(k.into(), Tag::Int(v));
        }
        if let Some((_, mut loaded)) =
            horizon_client::save::decode_block_entity(&Tag::Compound(full))
        {
            // The turn of a vault's item and a trial spawner's mob is the client's
            // own, not in the tag: an update keeps it (VaultClientData).
            match (self.world.block_entities.get(&key), &mut loaded) {
                (Some(BlockEntity::Vault(old)), BlockEntity::Vault(new)) => {
                    new.spin = old.spin;
                    new.o_spin = old.o_spin;
                }
                (Some(BlockEntity::TrialSpawner(old)), BlockEntity::TrialSpawner(new)) => {
                    new.spin = old.spin;
                    new.o_spin = old.o_spin;
                }
                _ => {}
            }
            let command_block = matches!(loaded, BlockEntity::CommandBlock(_));
            self.world.block_entities.insert(key, loaded);
            self.update_spawner_display(key);
            // The section is drawn again with what the block entity shows.
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let _ = self.world.set(pos[0], pos[1], pos[2], state);
            if command_block {
                self.command_block_data_arrived(key);
            }
        }
    }
    /// `handleAnimate`: a player wakes up, or an entity gets the critical
    /// hit (`CRIT`) or magic critical hit (`ENCHANTED_HIT`) emitter.
    fn remote_animate(&mut self, id: i32, action: u8) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let me = self.remote.as_ref().is_some_and(|r| r.entity_id == id);
        let body = if me {
            self.player.as_ref().map(|p| {
                let d = p.dimensions;
                (
                    [p.position.x, p.position.y, p.position.z],
                    f64::from(d.width),
                    f64::from(d.height),
                )
            })
        } else {
            self.mobs.iter().find(|m| m.id == id as u32).map(|m| {
                (
                    [m.position.x, m.position.y, m.position.z],
                    m.kind.width,
                    m.kind.height,
                )
            })
        };
        let Some((feet, width, height)) = body else {
            return;
        };
        let particle = match action {
            // Player.stopSleepInBed.
            0 => {
                if me {
                    self.sleeping = None;
                    self.sleep_timer = 100;
                }
                return;
            }
            1 => ParticleType::Crit,
            2 => ParticleType::EnchantedHit,
            _ => return,
        };
        self.particles
            .tracking_emitter(ParticleOptions::simple(particle), feet, width, height);
    }
    /// `handleServerData`: the server list entry of the server joined keeps
    /// the icon the server sends (`ServerList.saveSingleServer`). Single
    /// player has no entry.
    fn remote_server_data(&mut self, icon: Option<&[u8]>) {
        let (Some(r), Some(png)) = (self.remote.as_ref(), icon) else {
            return;
        };
        if self.integrated.is_some() {
            return;
        }
        if let Ok(dir) = std::env::current_dir() {
            horizon_client::multiplayer::save_server_icon(
                &dir.join("saves"),
                &r.session.address,
                png,
            );
        }
    }
    /// Keep the renderable block entities (chests, furnaces) in step with blocks.
    /// `previous`: the block's state before, when it changed.
    pub(super) fn sync_block_entity(
        &mut self,
        pos: (i32, i32, i32),
        previous: Option<BlockStateId>,
        state: BlockStateId,
    ) {
        let name = self.res.facts().get(state).name.clone();
        // LevelChunk.setBlockState: another block's block entity goes, even
        // one of the same type (a mod's blocks may share one).
        let other_block = previous.is_some_and(|p| self.res.facts().get(p).name != name);
        // A mod block's is the mod's own (`native_block_entity`), as on the server.
        match self.new_block_entity(&name) {
            Some(entity) => {
                let same = self.world.block_entities.get(&pos).is_some_and(|e| {
                    e.id() == entity.id() && !(other_block && matches!(e, BlockEntity::Mod(_)))
                });
                if !same {
                    self.world.block_entities.insert(pos, entity);
                }
            }
            None => {
                self.world.block_entities.remove(&pos);
            }
        }
    }
    /// ClientLevel.setServerVerifiedBlockState: a block the client predicted keeps its
    /// predicted state until the server acknowledges the action; the server's state is
    /// remembered for then.
    fn server_block(&mut self, pos: [i32; 3], state: i32) {
        if let Some((_, known)) = self
            .remote
            .as_mut()
            .and_then(|r| r.predictions.get_mut(&pos))
        {
            *known = state;
            return;
        }
        self.set_block(pos, state);
    }
    /// BlockStatePredictionHandler.endPredictionsUpTo (ClientboundBlockChangedAckPacket): the
    /// actions up to `sequence` are settled, so the blocks they predicted take the state the
    /// server gave them (a break the server refused comes back).
    fn end_predictions(&mut self, sequence: i32) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let settled: Vec<_> = r
            .predictions
            .iter()
            .filter(|(_, (s, _))| *s <= sequence)
            .map(|(&pos, &(_, state))| (pos, state))
            .collect();
        for (pos, state) in settled {
            if let Some(r) = self.remote.as_mut() {
                r.predictions.remove(&pos);
            }
            if self.world.block(pos[0], pos[1], pos[2]).0 as i32 != state {
                self.set_block(pos, state);
            }
        }
    }
    fn set_block(&mut self, pos: [i32; 3], state: i32) {
        let state = BlockStateId(state.max(0) as u32);
        let previous = self.world.block(pos[0], pos[1], pos[2]);
        if self.world.loaded(pos[0], pos[2])
            && self.world.set(pos[0], pos[1], pos[2], state).is_ok()
        {
            self.sync_block_entity((pos[0], pos[1], pos[2]), Some(previous), state);
            // LevelChunk.setBlockState removes the block entity of a block
            // that changes: the moving piston the client's moveBlocks made
            // stops when the server places what it moved (#3002).
            let facts = self.res.facts();
            let moving = |s: BlockStateId| &*facts.get(s).name == "minecraft:moving_piston";
            if moving(previous) && !moving(state) {
                let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
                self.block_ticks.moving_pistons.retain(|(p, _)| *p != at);
                self.sync_moving_pistons();
            }
        }
        // A block we just placed: play its place sound (the server skips the placer).
        if let Some((target, _)) = self.remote.as_ref().and_then(|r| r.placing)
            && target == pos
            && !self.res.facts().is_air(state)
        {
            let s = self.res.facts().get(state).sounds.clone();
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.audio
                .play(&s.place, Some(at), (s.volume + 1.0) / 2.0, s.pitch * 0.8);
            if let Some(r) = self.remote.as_mut() {
                r.placing = None;
            }
        }
    }
    /// `recreateFromPacket`: yRot and xRot from the packet, the body and head
    /// both at its head yaw (LivingEntity).
    fn spawn_entity(&mut self, id: i32, kind: &str, position: [f64; 3], rotation: [f32; 3]) {
        let [yaw, pitch, head] = rotation;
        let at = Vec3::new(position[0], position[1], position[2]);
        let mob_type = if kind == "minecraft:player" {
            self.remote.as_ref().and_then(|r| r.player_type.clone())
        } else {
            self.res.mob_types().get(kind).cloned()
        };
        if kind == "minecraft:ender_dragon" {
            self.spawn_remote_dragon(id, position, yaw);
        }
        if kind == "minecraft:experience_orb" {
            // Its value (ExperienceOrb.DATA_VALUE) follows in SetEntityData.
            self.orbs.retain(|o| o.id != id as u32);
            self.orbs.push(ExperienceOrb {
                id: id as u32,
                position: at,
                previous: at,
                velocity: Vec3::ZERO,
                value: 1,
                age: 0,
                on_ground: false,
                count: 1,
                ticks: 0,
                health: 5,
            });
        }
        if let Some(t) = mob_type {
            let mut mob = Mob::new(id as u32, t, at, head);
            super::remote_mob_data::client_defaults(&mut mob);
            (mob.yaw, mob.previous_yaw) = (yaw, yaw);
            (mob.head_pitch, mob.previous_head_pitch) = (pitch, pitch);
            mob.persistent = true;
            self.mobs.retain(|m| m.id != id as u32);
            self.mobs.push(mob);
        }
        if let Some(r) = self.remote.as_mut() {
            // A new entity has a new interpolation handler (a mob's or an
            // armor stand's), even under an id the server used before.
            r.stepped.remove(&id);
            if kind == "minecraft:item" {
                r.pending_items.insert(id, (position, Vec3::ZERO));
            }
            r.entities.insert(
                id,
                Tracked {
                    kind: kind.to_owned(),
                    position,
                },
            );
        }
    }
    /// A move packet for entity `id`: to the end of `path` (None: a
    /// rotation alone), turned to `rotation` if it has one. `packet` says
    /// whether a living entity more than 64 blocks away is put there at once
    /// (its `snaps`).
    fn move_entity(
        &mut self,
        id: i32,
        path: Option<Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
        packet: MovePacket,
    ) {
        // The dragon is drawn from its own state (game/dragon.rs), which
        // tick_remote_dragon eases to the server's moves through `lerps`. Its
        // copy in the mob list draws nothing: given the move, it kept the
        // dragon where it spawned (#2083).
        let dragon = self.dragon.as_ref().is_some_and(|d| d.id as i32 == id);
        let driven = self.driven_mount().is_some_and(|m| m.id as i32 == id);
        let mob = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id as u32)
            .filter(|_| !dragon);
        let tracked = self
            .remote
            .as_ref()
            .and_then(|r| r.entities.get(&id))
            .map(|t| t.position);
        let position = path
            .as_ref()
            .map(Path::end)
            .or(tracked)
            .or_else(|| {
                mob.as_ref()
                    .map(|m| [m.position.x, m.position.y, m.position.z])
            })
            .unwrap_or([0.0; 3]);
        if let Some(r) = self.remote.as_mut() {
            if let Some(t) = r.entities.get_mut(&id) {
                t.position = position;
            }
            if driven {
                // handleMoveEntity, handleEntityPositionSync: the mount this
                // client moves (isLocalInstanceAuthoritative) keeps its own
                // position, and no steps pile up for after the ride.
                r.stepped.remove(&id);
                return;
            }
            if let Some(m) = mob {
                // A living entity: LivingEntity.createInterpolationHandler's
                // SteppedInterpolationHandler, over its type's update interval.
                let entity = PosRot {
                    position: [m.position.x, m.position.y, m.position.z],
                    yaw: m.yaw,
                    pitch: m.head_pitch,
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval = horizon_client::lan::tracking::settings(&m.kind.id).1;
                    SteppedInterpolation::new(interval as i32)
                });
                if let Some(at) = stepped_move(handler, packet, entity, position, path, rotation) {
                    // snapTo: setOldPosAndRot too.
                    place_interpolated(m, at);
                    m.previous = m.position;
                    m.previous_yaw = m.yaw;
                    m.previous_head_pitch = m.head_pitch;
                }
                m.on_ground = on_ground;
                return;
            }
            // An armor stand is a living entity too: the same handler, over
            // its type's update interval, stepped in `tick_remote_entities`.
            if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id as u32) {
                let entity = PosRot {
                    position: [a.position.x, a.position.y, a.position.z],
                    yaw: a.yaw,
                    pitch: 0.0,
                };
                let handler = r.stepped.entry(id).or_insert_with(|| {
                    let interval =
                        horizon_client::lan::tracking::settings("minecraft:armor_stand").1;
                    SteppedInterpolation::new(interval as i32)
                });
                if let Some(at) = stepped_move(handler, packet, entity, position, path, rotation) {
                    a.snap(
                        Vec3::new(at.position[0], at.position[1], at.position[2]),
                        at.yaw,
                    );
                }
                a.on_ground = on_ground;
                return;
            }
            // Interaction extends Entity and keeps its NO_OP interpolation
            // handler: the server's position replaces its own at once.
            if let Some(i) = self.interactions.get_mut(&id) {
                if path.is_some() {
                    i.position = Vec3::new(position[0], position[1], position[2]);
                }
                return;
            }
            // Projectiles keep Entity's NO_OP InterpolationHandler too: the
            // server's position replaces theirs at once and their own tick
            // flies them on (`tick_remote_projectiles`). Eased towards it
            // instead, they only moved when an update came, every 10 to 20
            // ticks.
            if let Some(p) = self
                .creatures
                .projectiles
                .iter_mut()
                .find(|p| p.id == id as u32)
            {
                if path.is_some() {
                    p.position = Vec3::new(position[0], position[1], position[2]);
                }
                if let Some((yaw, pitch)) = rotation {
                    p.yaw = yaw;
                    p.pitch = pitch;
                }
                return;
            }
            if path.is_some()
                && (self.potions.move_remote_thrown(
                    id as u32,
                    Vec3::new(position[0], position[1], position[2]),
                ) || self.end.move_remote_eye(id as u32, position))
            {
                return;
            }
            // A fishing hook keeps a LinearInterpolationHandler: eased towards the
            // update over three ticks, on top of its own movement (`tick_remote_hooks`).
            if path.is_some() {
                if let Some(h) = self.hook.as_mut().filter(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
                if let Some(h) = self.remote_hooks.iter_mut().find(|h| h.id() == id as u32) {
                    h.move_to(position, on_ground);
                    return;
                }
            }
            // A thrown trident is an arrow: placed at once, flown on by its tick.
            if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id as u32) {
                if path.is_some() {
                    t.position = Vec3::new(position[0], position[1], position[2]);
                }
                if let Some((yaw, pitch)) = rotation {
                    t.yaw = yaw;
                    t.pitch = pitch;
                }
                return;
            }
            // So does a firework rocket: its own tick flies it on.
            if path.is_some()
                && let Some(f) = self.rockets.iter_mut().find(|f| f.id == id as u32)
            {
                f.position = Vec3::new(position[0], position[1], position[2]);
                return;
            }
            // A dragon fireball keeps Entity's NO_OP InterpolationHandler too:
            // the server's position replaces its own at once.
            if let Some(b) = self.dragon_fireballs.iter_mut().find(|b| b.id == id as u32) {
                if path.is_some() {
                    b.position = Vec3::new(position[0], position[1], position[2]);
                }
                return;
            }
            if let Some(p) = r.pending_items.get_mut(&id) {
                p.0 = position;
            }
            // ItemEntity, FallingBlockEntity and PrimedTnt keep Entity's
            // NO_OP InterpolationHandler: moveOrInterpolateTo puts them there
            // at once, and their own tick moves them on between the server's
            // updates (`tick_remote_entities`, `tick_remote_projectiles`).
            let at = Vec3::new(position[0], position[1], position[2]);
            if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
                d.position = at;
                d.on_ground = on_ground;
                return;
            }
            if let Some(f) = self.falling.iter_mut().find(|f| f.id == id as u32) {
                f.position = at;
                return;
            }
            if let Some(t) = self.tnts.iter_mut().find(|t| t.id == id as u32) {
                t.position = at;
                t.on_ground = on_ground;
                return;
            }
            // ExperienceOrb.createInterpolationHandler is a
            // LinearInterpolationHandler: the correction is spread over 3
            // ticks (`tick_remote_entities`), while `ExperienceOrb.tick` keeps
            // moving it.
            if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id as u32) {
                o.on_ground = on_ground;
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation: None,
                        steps: 3,
                    },
                );
                return;
            }
            if r.cart_lerps.contains_key(&id) {
                // A minecart on NewMinecartBehavior keeps Entity's NO_OP
                // InterpolationHandler: a teleport puts it there at once.
                if path.is_some()
                    && let Some(c) = self.carts.iter_mut().find(|c| c.id == id as u32)
                {
                    c.position = at;
                }
            } else {
                // AbstractInterpolationHandler.interpolateTo: without a
                // rotation, the one it was heading for.
                let rotation = rotation.or_else(|| r.lerps.get(&id).and_then(|l| l.rotation));
                r.lerps.insert(
                    id,
                    Lerp {
                        target: position,
                        rotation,
                        steps: 3,
                    },
                );
            }
        }
        self.turn_remote_object(id, rotation.map(|(yaw, _)| yaw), on_ground);
        self.move_remote_text_display(id, position, rotation);
        self.turn_remote_projectile(id, rotation);
    }
    fn remove_entity(&mut self, id: i32) {
        // Entity.setRemoved detaches every passenger, even when no empty
        // SetPassengers packet preceded the vehicle's removal. Otherwise
        // mobs and other players keep their passenger pose after it is gone.
        self.remote_set_passengers(id, &[]);
        self.mobs.retain(|m| m.id != id as u32);
        self.orbs.retain(|o| o.id != id as u32);
        self.dropped.retain(|d| d.id != id as u32);
        // Before the crystals go: the dragon's beam index follows them.
        self.remove_remote_dragon(id);
        self.remove_remote_object(id as u32);
        self.remove_remote_projectile(id as u32);
        self.persist.uuids.remove(&(id as u32));
        if let Some(r) = self.remote.as_mut() {
            r.glowing.remove(&id);
            r.entities.remove(&id);
            r.lerps.remove(&id);
            r.cart_lerps.remove(&id);
            r.stepped.remove(&id);
            r.head_lerps.remove(&id);
            r.pending_items.remove(&id);
        }
    }
    /// Map a menu slot to Horizon's inventory screen slot ids.
    fn menu_slot(menu: &str, slot: i32) -> Option<usize> {
        let s = usize::try_from(slot).ok()?;
        // A container of `n` slots, then the inventory and the hotbar.
        let slots = |n: usize| -> Option<usize> {
            match s {
                _ if s < n => Some(300 + s),
                _ if s < n + 27 => Some(9 + s - n),
                _ if s < n + 36 => Some(s - n - 27),
                _ => None,
            }
        };
        let rows = |n: usize| slots(n * 9);
        match menu {
            "inventory" => match s {
                0 => Some(200),
                1..=4 => Some(100 + s - 1),
                5..=8 => Some(500 + s - 5),
                9..=35 => Some(s),
                36..=44 => Some(s - 36),
                // InventoryMenu.SHIELD_SLOT: the off hand.
                45 => Some(504),
                _ => None,
            },
            "minecraft:crafting" => match s {
                0 => Some(200),
                1..=9 => Some(100 + s - 1),
                10..=36 => Some(9 + s - 10),
                37..=45 => Some(s - 37),
                _ => None,
            },
            "minecraft:merchant" => match s {
                0 | 1 => Some(super::stations::INPUT + s),
                2 => Some(super::stations::RESULT),
                3..=29 => Some(9 + s - 3),
                30..=38 => Some(s - 30),
                _ => None,
            },
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => match s {
                0..=2 => Some(300 + s),
                3..=29 => Some(9 + s - 3),
                30..=38 => Some(s - 30),
                _ => None,
            },
            "minecraft:generic_9x1" => rows(1),
            "minecraft:generic_9x2" => rows(2),
            "minecraft:generic_9x3" | "minecraft:shulker_box" => rows(3),
            "minecraft:generic_9x4" => rows(4),
            "minecraft:generic_9x5" => rows(5),
            "minecraft:generic_9x6" => rows(6),
            // HopperMenu; DispenserMenu (dispensers and droppers).
            "minecraft:hopper" => slots(5),
            "minecraft:generic_3x3" => slots(9),
            "minecraft:brewing_stand" => slots(5),
            // CrafterMenu: the grid, the inventory, then the result (the
            // client's own).
            "minecraft:crafter_3x3" => slots(9),
            // HorseInventoryMenu: the saddle, the body armour, the chest,
            // then the inventory.
            m if m.starts_with(MOUNT_MENU) => {
                let n = m[MOUNT_MENU.len()..].parse::<usize>().ok()? * 3;
                match s {
                    0 => Some(super::mount_menu::SADDLE),
                    1 => Some(super::mount_menu::ARMOR),
                    _ if s < 2 + n => Some(super::mount_menu::CHEST + s - 2),
                    _ if s < 2 + n + 27 => Some(9 + s - 2 - n),
                    _ if s < 2 + n + 36 => Some(s - 2 - n - 27),
                    _ => None,
                }
            }
            // A content mod's menu type: its container, then the inventory.
            // A workstation: its inputs, its result, then the inventory.
            m => {
                if let Some(t) = horizon_client::containers::mod_menu_type(m) {
                    return slots(t.slots);
                }
                let kind = Self::remote_station_kind(m)?;
                let inputs = kind.inputs();
                let own = inputs + usize::from(kind.has_result());
                match s {
                    _ if s < inputs => Some(super::stations::INPUT + s),
                    _ if s < own => Some(super::stations::RESULT),
                    _ if s < own + 27 => Some(9 + s - own),
                    _ if s < own + 36 => Some(s - own - 27),
                    _ => None,
                }
            }
        }
    }
    /// Find a game slot in the menu's complete, contiguous slot mapping.
    pub(super) fn menu_slot_index(menu: &str, internal: usize) -> Option<usize> {
        (0..)
            .map_while(|slot| Self::menu_slot(menu, slot))
            .position(|slot| slot == internal)
    }
    /// The workstation screen of a menu type a server opens.
    pub(super) fn remote_station_kind(menu: &str) -> Option<super::stations::Kind> {
        use super::stations::Kind;
        Some(match menu {
            "minecraft:loom" => Kind::Loom,
            "minecraft:stonecutter" => Kind::Stonecutter,
            "minecraft:grindstone" => Kind::Grindstone,
            "minecraft:anvil" => Kind::Anvil,
            "minecraft:smithing" => Kind::Smithing,
            "minecraft:enchantment" => Kind::Enchanting,
            "minecraft:beacon" => Kind::Beacon,
            "minecraft:cartography_table" => Kind::Cartography,
            _ => return None,
        })
    }
    fn open_menu_name(&self, container: i32) -> String {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            Some((id, name)) if *id == container && container != 0 => name.clone(),
            _ => "inventory".into(),
        }
    }
    fn set_menu_slot(&mut self, container: i32, slot: i32, item: Option<ItemStack>) {
        let menu = self.open_menu_name(container);
        if menu == "minecraft:lectern" {
            if slot == 0 {
                self.remote_lectern_book(item);
            }
            return;
        }
        if container != 0 && menu == "inventory" {
            return;
        }
        let Some(internal) = Self::menu_slot(&menu, slot) else {
            return;
        };
        if internal == 200 {
            return; // the result slot is recomputed from the grid
        }
        let slot = self.slot_mut(internal);
        let changed = !super::hand::matches_for_swap(slot.as_ref(), item.as_ref());
        *slot = item;
        if internal < 36 {
            self.remote_hand_slot_set(internal, changed);
        } else if internal == 504 {
            self.remote_hand_slot_set(super::shield::OFFHAND_SLOT, changed);
        }
        if internal >= super::stations::INPUT && self.station.is_some() {
            self.remote_station_slot_changed(internal);
        }
    }

    /// `ClientPacketListener.handleUpdateAttributes` for a mob, each value from
    /// `AttributeInstance.calculateValue`: its maximum health (the hearts drawn
    /// over its mount), and a player's or mannequin's `minecraft:scale`, which
    /// scales its model (`LivingEntityRenderState.scale`) as the local player's
    /// does: NPC plugins make their fake players bigger with it.
    fn remote_mob_attributes(&mut self, id: i32, values: &[play::AttributeSnapshot]) {
        let catalog = &self.motion.attributes;
        let value = |name: &str| {
            let attribute = catalog.id(name)?;
            let s = values
                .iter()
                .find(|s| s.attribute == attribute.registry_id())?;
            Some(
                catalog
                    .attribute(attribute)
                    .definition
                    .sanitize(snapshot_value(s)),
            )
        };
        let (max_health, scale) = (value("minecraft:max_health"), value("minecraft:scale"));
        let (speed, jump) = (
            value("minecraft:movement_speed"),
            value("minecraft:jump_strength"),
        );
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id as u32) else {
            return;
        };
        // A horse its rider's client moves goes at its own speed and jumps
        // its own height (`getSpeed`, `getCustomJump`).
        if horizon_client::mobs::equine::moved_by_rider(m.species()) {
            if let Some(speed) = speed {
                horizon_client::mobs::finalize::set_base(
                    m,
                    horizon_client::mobs::finalize::Attr::MovementSpeed,
                    speed,
                );
            }
            if let Some(jump) = jump {
                m.extra.vector[0] = jump;
            }
        }
        if let Some(max_health) = max_health {
            horizon_client::mobs::finalize::set_base(
                m,
                horizon_client::mobs::finalize::Attr::MaxHealth,
                max_health,
            );
        }
        if let Some(scale) = scale {
            m.extra.model_scale = scale as f32;
        }
    }

    fn handle_packet(&mut self, packet: C) -> Result<()> {
        match packet {
            // ClientCommonPacketListenerImpl.handlePing runs on the game
            // thread, after all preceding packets (especially teleports).
            C::Ping(p) => self.send(S::Pong(p)),
            // A native mod's server half to its client half.
            C::CustomPayload(p) if p.channel == horizon_protocol::mod_net::CHANNEL => {
                self.remote_native_net(&p.data);
            }
            // PingDebugMonitor.onPongReceived.
            C::PongResponse(p) => {
                let delay = self.debug_millis() - p.time;
                self.debug_charts.ping.log(delay);
            }
            C::Login(p) => {
                self.test_highlights.clear();
                self.level.hardcore = p.hardcore;
                // handleLogin: startWaitingForNewLevel with Reason.OTHER, which
                // replaces ServerReconfigScreen.
                if let Some(r) = self.remote.as_mut() {
                    r.reconfiguring = None;
                }
                // handleLogin makes a new MultiPlayerGameMode (carried index 0) and a
                // new LocalPlayer (slot 0, no effects, riding nothing: the server
                // pairs a vehicle again with ClientboundSetPassengersPacket).
                self.selected = 0;
                self.potions.effects.clear();
                self.riding = None;
                if let Some(r) = self.remote.as_mut() {
                    r.carried = Some(0);
                    r.own_invisible = false;
                    r.online_mode = p.online_mode;
                    r.enforces_secure_chat = p.enforces_secure_chat;
                    r.entity_id = p.player_id;
                    // handleLogin: the new LocalPlayer is the camera.
                    r.camera = None;
                    r.logged_in = true;
                    r.view_distance = p.chunk_radius;
                    r.simulation_distance = p.simulation_distance;
                    r.show_death_screen = p.show_death_screen;
                }
                self.streaming.radius =
                    self.settings.render_distance.min(p.chunk_radius + 1).max(2);
                // handleLogin: startWaitingForNewLevel(.., Reason.OTHER).
                self.enter_remote_dimension(
                    &p.spawn,
                    false,
                    horizon_client::level_loading::Reason::Other,
                );
                let info = self
                    .settings
                    .client_information(client_view_distance(self.settings.render_distance));
                if let Some(r) = self.remote.as_mut() {
                    r.sent_info = info.clone();
                }
                self.send(S::ClientInformation(info));
                self.remote_chat_login();
            }
            C::Respawn(p) => {
                // A new player, without effects (the server sends them
                // again) and with no item in use.
                self.potions.effects.clear();
                self.using = None;
                // After a death (no KEEP_ENTITY_DATA) the new LocalPlayer
                // rides nothing: the server pairs a vehicle again with its
                // passengers. A dimension change keeps the vehicle it travels in.
                if p.keep & 2 == 0 {
                    self.riding = None;
                    // KEEP_ENTITY_DATA copies the old player's synched data
                    // (its shared flags) to the new one; without it they reset.
                    if let Some(r) = self.remote.as_mut() {
                        r.own_invisible = false;
                    }
                }
                let dimension_changed = self.dim != Dim::from_id(&p.spawn.dimension);
                let died = self.screen == Screen::Dead
                    || self
                        .player
                        .as_ref()
                        .is_some_and(|m| m.health.health() <= 0.0);
                let reason = horizon_client::level_loading::reason_for_respawn(
                    died,
                    &p.spawn.dimension,
                    self.dim.id(),
                );
                self.enter_remote_dimension(&p.spawn, true, reason);
                if dimension_changed {
                    // LevelExtractor.setLevel: a new level drops the highlights.
                    self.test_highlights.clear();
                    let (music, _) = self.situational_music();
                    self.music.stop_playing(music.as_ref(), &mut self.audio);
                }
                self.screen = Screen::Playing;
                if let Some(r) = self.remote.as_mut() {
                    r.death_message = None;
                    r.health = (20.0, 20, 5.0);
                    // handleRespawn: the new LocalPlayer is the camera.
                    r.camera = None;
                }
                self.capture(true);
            }
            C::PlayerPosition(p) => self.remote_teleport(&p)?,
            C::PlayerRotation(p) => {
                self.yaw = if p.relative_yaw {
                    self.yaw + p.yaw
                } else {
                    p.yaw
                };
                self.pitch = (if p.relative_pitch {
                    self.pitch + p.pitch
                } else {
                    p.pitch
                })
                .clamp(-90.0, 90.0);
                self.snap_player_model_pitch();
            }
            C::PlayerLookAt(p) => self.remote_look_at(&p),
            // ClientCommonPacketListenerImpl.handlePostEffects.
            C::PostEffects(p) => self.post_effects = p.effects,
            C::SetCamera(p) => self.remote_set_camera(p.camera_id),
            C::LevelChunkWithLight(p) => self.remote_chunk(&p, None),
            C::LightUpdate(p) => self.remote_light_update(&p),
            C::ChunksBiomes(p) => self.remote_chunks_biomes(&p),
            C::ForgetLevelChunk(p) => {
                let [x, z] = p.pos;
                self.streaming.forget(&mut self.world, x, z);
                self.world
                    .block_entities
                    .retain(|k, _| (k.0 >> 4, k.2 >> 4) != (x, z));
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.remove_columns([(x, z)]);
                }
            }
            C::ChunkBatchFinished(_) => {
                // ChunkBatchSizeCalculator's ceiling; meshing runs on workers.
                self.send(S::ChunkBatchReceived(play::ChunkBatchReceived {
                    chunks_per_tick: 64.0,
                }));
            }
            C::BlockUpdate(p) => self.server_block(p.pos, p.state),
            C::AddTransientBlock(p) => self.add_transient_block(p.pos, p.state),
            // ClientPacketListener.handleBlockDestruction: another player's crack.
            C::BlockDestruction(p) => {
                self.destroy_block_progress(p.id, p.pos, i32::from(p.progress));
            }
            C::SectionBlocksUpdate(p) => {
                let [sx, sy, sz] = p.section;
                for change in p.changes {
                    let local = (change & 0xFFF) as i32;
                    let pos = [
                        sx * 16 + (local >> 8 & 15),
                        sy * 16 + (local & 15),
                        sz * 16 + (local >> 4 & 15),
                    ];
                    self.server_block(pos, (change >> 12) as i32);
                }
            }
            C::BlockChangedAck(p) => self.end_predictions(p.sequence),
            // Level.blockEvent on the client level: the block's triggerEvent
            // runs on the client's copy of whatever is at pos (a piston's
            // moveBlocks builds the moving block entities it animates, a note
            // block plays); what it doesn't handle reaches the client copy of
            // the block entity (chest lids, shulker boxes).
            C::BlockEvent(p) => {
                let pos = horizon_core::position::BlockPos::new(p.pos[0], p.pos[1], p.pos[2]);
                let triggered = self.with_level(|b, level, ticks| {
                    b.client_block_event(level, ticks, pos, i32::from(p.b0), i32::from(p.b1))
                });
                self.sync_moving_pistons();
                self.process_level_events()?;
                if !triggered {
                    let state = self.world.block(p.pos[0], p.pos[1], p.pos[2]);
                    let block = &self.res.facts().get(state).name;
                    let named = self.remote.as_ref().and_then(|r| {
                        r.builtin
                            .get("minecraft:block")?
                            .name(p.block)
                            .map(str::to_owned)
                    });
                    if named.as_deref() == Some(&**block) {
                        self.block_entity_event(p.pos, i32::from(p.b0), i32::from(p.b1));
                    }
                }
            }
            // ClientLevel.levelEvent: LevelEventHandler, as this client's own
            // level plays them (particles and sounds: a spawner's burst, bone
            // meal, waxing, a block broken by someone else).
            // ClientPacketListener.handleLevelEvent: a global one is heard
            // from anywhere (ClientLevel.globalLevelEvent).
            C::LevelEvent(p) if p.global => self.global_level_event_sound(p.kind, p.pos),
            C::LevelEvent(p) => self.level_event_at(p.kind, p.pos, p.data),
            C::LevelParticles(p) => self.remote_level_particles(&p),
            C::Explode(p) => self.remote_explode(&p),
            C::Sound(p) => {
                if let Some(name) = self.remote_sound_name(&p.sound) {
                    let at = [
                        f64::from(p.x) / 8.0,
                        f64::from(p.y) / 8.0,
                        f64::from(p.z) / 8.0,
                    ];
                    // ClientPacketListener.handleSoundEvent plays with packet.getSource().
                    match usize::try_from(p.source)
                        .ok()
                        .filter(|s| *s < horizon_client::settings::SOUND_SOURCES.len())
                    {
                        Some(source) => {
                            self.audio
                                .play_on(source, &name, Some(at), p.volume, p.pitch)
                        }
                        None => self.audio.play(&name, Some(at), p.volume, p.pitch),
                    }
                }
            }
            // shortcut: vanilla's `EntityBoundSoundInstance` re-reads the entity's position
            // every tick and stops with it, so the sound follows the entity; a play at the
            // packet's arrival does not. Upgrade when the sound engine has a generic
            // entity-bound instance (its tickable ones are one per mob kind, e.g. `Ticking::Bee`).
            C::SoundEntity(p) => {
                if let Some(name) = self.remote_sound_name(&p.sound)
                    && let Some(at) = self.remote_entity_position(p.id)
                {
                    match usize::try_from(p.source)
                        .ok()
                        .filter(|s| *s < horizon_client::settings::SOUND_SOURCES.len())
                    {
                        Some(source) => {
                            self.audio
                                .play_on(source, &name, Some(at), p.volume, p.pitch)
                        }
                        None => self.audio.play(&name, Some(at), p.volume, p.pitch),
                    }
                }
            }
            C::StopSound(p) => self
                .audio
                .stop_sounds(p.sound.as_deref(), p.source.map(|s| s as usize)),
            C::AddEntity(p) => {
                let kind = self
                    .remote
                    .as_ref()
                    .and_then(|r| {
                        r.builtin
                            .get("minecraft:entity_type")?
                            .name(p.kind)
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                let rotation =
                    [p.yaw, p.pitch, p.head_yaw].map(horizon_protocol::buf::unpack_degrees);
                if let Some(r) = self.remote.as_mut() {
                    // A new entity looks its PlayerInfo up afresh.
                    r.level_player_textures.remove(&p.id);
                    if kind == "minecraft:player" {
                        r.level_players.insert(p.id, p.uuid);
                        // createEntityFromPacket: the player takes the profile of its list entry.
                        match r.players.get(&p.uuid) {
                            Some(name) => r.level_player_names.insert(p.id, name.clone()),
                            None => r.level_player_names.remove(&p.id),
                        };
                    } else {
                        r.level_players.remove(&p.id);
                        r.level_player_names.remove(&p.id);
                    }
                }
                // Entity.getTeam: a mob is on a team by its UUID, a player by
                // its profile name (`Game::scoreboard_name`).
                self.persist.uuids.insert(
                    p.id as u32,
                    horizon_client::lan::playerdata::uuid_ints(p.uuid),
                );
                self.spawn_entity(p.id, &kind, p.position, rotation);
                // Entity.recreateFromPacket: setDeltaMovement(packet.getMovement()).
                self.remote_motion(p.id, p.velocity);
                if !self.spawn_remote_object(p.id, &kind, p.position, rotation[0], p.data) {
                    self.spawn_remote_projectile(
                        p.id,
                        &kind,
                        p.position,
                        (rotation[0], rotation[1]),
                        p.velocity,
                        p.data,
                    );
                }
                self.move_remote_text_display(p.id, p.position, Some((rotation[0], rotation[1])));
                self.move_remote_interaction(p.id, p.position);
            }
            C::RemoveEntities(p) => {
                for id in p.ids {
                    if let Some(r) = self.remote.as_mut() {
                        r.level_players.remove(&id);
                        r.level_player_names.remove(&id);
                        r.level_player_textures.remove(&id);
                    }
                    self.remove_entity(id);
                }
            }
            C::MoveEntityPos(play::MoveEntityPos(m))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
            | C::MoveEntityRot(play::MoveEntityRot(m)) => {
                let base = self
                    .remote
                    .as_ref()
                    .and_then(|r| r.entities.get(&m.id))
                    .map(|t| t.position);
                let Some(base) = base else { return Ok(()) };
                // VecDeltaCodec.decode: each step's position from the last.
                let path = m.delta.as_ref().map(|d| match d {
                    play::VecDelta::Linear(_) => Path::Linear(d.end(base)),
                    play::VecDelta::Stepped(steps) => {
                        let mut at = base;
                        Path::Stepped(
                            steps
                                .iter()
                                .map(|(delta, ticks)| {
                                    at = play::VecDelta::apply(at, *delta);
                                    (at, *ticks)
                                })
                                .collect(),
                        )
                    }
                });
                let rotation = m.rotation.map(|(y, x)| {
                    (
                        horizon_protocol::buf::unpack_degrees(y),
                        horizon_protocol::buf::unpack_degrees(x),
                    )
                });
                self.move_entity(m.id, path, rotation, m.on_ground, MovePacket::Move);
            }
            C::EntityPositionSync(p) => {
                let path = match p.path {
                    play::PositionPath::Linear(at) => Path::Linear(at),
                    play::PositionPath::Stepped(steps) => Path::Stepped(steps),
                };
                self.move_entity(
                    p.id,
                    Some(path),
                    Some((p.yaw, p.pitch)),
                    p.on_ground,
                    MovePacket::PositionSync,
                )
            }
            C::TeleportEntity(p) => {
                let base = self
                    .remote
                    .as_ref()
                    .and_then(|r| r.entities.get(&p.id))
                    .map_or([0.0; 3], |t| t.position);
                let rel = |bit: i32, i: usize| {
                    if p.relatives & bit != 0 {
                        base[i] + p.position[i]
                    } else {
                        p.position[i]
                    }
                };
                let position = [
                    rel(play::relative::X, 0),
                    rel(play::relative::Y, 1),
                    rel(play::relative::Z, 2),
                ];
                // setValuesFromPositionPacket: interpolated, unless too far.
                self.move_entity(
                    p.id,
                    Some(Path::Linear(position)),
                    Some((p.yaw, p.pitch)),
                    p.on_ground,
                    MovePacket::Teleport,
                );
                // handleTeleportEntity: the mount this client moves is put
                // there at once.
                if self.driven_mount().is_some_and(|m| m.id as i32 == p.id) {
                    self.remote_move_vehicle(position, p.yaw);
                }
            }
            C::SetPassengers(p) => self.remote_set_passengers(p.vehicle, &p.passengers),
            // ClientPacketListener.handleEntityLinkPacket: who holds a mob's
            // lead (this player, another player, a mob or a fence knot), for
            // the rope.
            C::SetEntityLink(p) => {
                let me = self.remote.as_ref().map(|r| r.entity_id);
                use horizon_client::mobs::leash::Holder;
                let holder = match p.dest {
                    0 => None,
                    d if Some(d) == me => Some(Holder::Player),
                    d => match self.remote_knots.get(&(d as u32)) {
                        Some(&block) => Some(Holder::Knot(block)),
                        // Not a known knot yet: kept by id until the knot
                        // arrives (game/remote_objects.rs).
                        None if self.boats.iter().any(|b| b.id == d as u32) => {
                            Some(Holder::Boat(d as u32))
                        }
                        None => Some(Holder::Mob(d as u32)),
                    },
                };
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.source as u32) {
                    m.extra.leash = holder;
                } else if let Some(b) = self.boats.iter_mut().find(|b| b.id == p.source as u32) {
                    // A leashed boat or raft (AbstractBoat is Leashable).
                    b.leash = holder;
                }
            }
            C::MoveVehicle(p) => self.remote_move_vehicle(p.position, p.yaw),
            C::MoveMinecartAlongTrack(p) => self.remote_move_minecart(p),
            // ClientPacketListener.handleRotateMob: lerpHeadTo(yHeadRot, 3).
            C::RotateHead(p) => {
                if let Some(r) = self.remote.as_mut() {
                    let yaw = horizon_protocol::buf::unpack_degrees(p.head_yaw);
                    r.head_lerps.insert(p.id, (yaw, 3));
                }
            }
            C::SetEntityData(p) => {
                // Entity.DATA_SHARED_FLAGS_ID, DATA_CUSTOM_NAME and
                // DATA_CUSTOM_NAME_VISIBLE: server NPCs and holograms rely on
                // these instead of saved NBT.
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            (0, Value::Byte(flags)) => {
                                m.invisible = flags & 0x20 != 0;
                                m.glowing = flags & 0x40 != 0;
                            }
                            (2, Value::OptText(name)) => {
                                let text = name.as_ref().map(horizon_client::entity_nbt::name_text);
                                m.custom_name =
                                    text.as_ref().map(horizon_client::text::Text::plain);
                                m.custom_name_text = text;
                            }
                            (2, Value::Text(name)) => {
                                let text = horizon_client::entity_nbt::name_text(name);
                                m.custom_name = Some(text.plain());
                                m.custom_name_text = Some(text);
                            }
                            (3, Value::Bool(visible)) => m.name_visible = *visible,
                            _ => {}
                        }
                    }
                }
                if let Some((_, _, Value::Byte(flags))) = p.values.iter().find(|v| v.0 == 0)
                    && let Some(r) = self.remote.as_mut()
                {
                    if flags & 0x40 != 0 {
                        r.glowing.insert(p.id);
                    } else {
                        r.glowing.remove(&p.id);
                    }
                }
                self.remote_mob_data(p.id, &p.values);
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                    && m.species() == horizon_client::mobs::Species::Creeper
                {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            // Creeper.DATA_SWELL_DIR, DATA_IS_POWERED, DATA_IS_IGNITED.
                            (16, Value::Int(dir)) => m.extra.counter = *dir,
                            (17, Value::Bool(powered)) => m.extra.charged = *powered,
                            (18, Value::Bool(ignited)) => m.extra.flag = *ignited,
                            _ => {}
                        }
                    }
                }
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            // Entity.DATA_TICKS_FROZEN: isFullyFrozen shakes it.
                            (7, Value::Int(ticks)) => m.ticks_frozen = *ticks,
                            // Strider.DATA_SUFFOCATING (after AgeableMob's two
                            // and DATA_BOOST_TIME): a cold strider shakes.
                            (19, Value::Bool(cold))
                                if m.species() == horizon_client::mobs::Species::Strider =>
                            {
                                m.extra.flag = *cold;
                            }
                            _ => {}
                        }
                    }
                }
                // LivingEntity.DATA_LIVING_ENTITY_FLAGS: for another player
                // too, remote_mob_data's apply reads index 8 above.
                let mannequin = self
                    .mobs
                    .iter()
                    .any(|m| m.id == p.id as u32 && &*m.kind.id == "minecraft:mannequin");
                let other = mannequin
                    || self.remote.as_ref().is_some_and(|r| {
                        r.level_players.contains_key(&p.id) && r.entity_id != p.id
                    });
                // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): another
                // player lies down in their bed (drawn lying in it), or gets up.
                if other {
                    for (index, _, value) in &p.values {
                        if let (14, Value::OptPos(bed)) = (index, value) {
                            let code = bed.map_or(0, |b| self.bed_facing(b));
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                                m.extra.ints[1] = code;
                                if let Some(head) = *bed {
                                    // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): setPosToBed.
                                    let at = Vec3::new(
                                        f64::from(head[0]) + 0.5,
                                        f64::from(head[1]) + 0.5625 + 0.125,
                                        f64::from(head[2]) + 0.5,
                                    );
                                    m.position = at;
                                    m.previous = at;
                                    m.head_pitch = 0.0;
                                    m.previous_head_pitch = 0.0;
                                }
                            }
                        }
                        // Avatar.DATA_PLAYER_MAIN_HAND (HumanoidArm: 0 left, 1
                        // right): the arm a swing of the main hand moves.
                        if let (15, Value::Id(arm)) = (index, value)
                            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                        {
                            m.extra.main_hand_left = *arm == 0;
                        }
                        // Avatar.DATA_PLAYER_MODE_CUSTOMISATION: the skin layers
                        // the player shows (PlayerModelPart bits).
                        if let (16, Value::Byte(bits)) = (index, value)
                            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                        {
                            m.extra.hidden_model_parts = !(*bits as u8) & 0x7F;
                        }
                    }
                }
                // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): this
                // player lies down in the bed, or gets up.
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    for (index, _, value) in &p.values {
                        match (index, value) {
                            (14, Value::OptPos(bed)) => {
                                if bed.is_some() && self.sleeping.is_none() {
                                    self.sleep_timer = 0;
                                }
                                self.sleeping = *bed;
                                if let Some(head) = *bed {
                                    // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): setPosToBed.
                                    let at = Vec3::new(
                                        f64::from(head[0]) + 0.5,
                                        f64::from(head[1]) + 0.5625 + 0.125,
                                        f64::from(head[2]) + 0.5,
                                    );
                                    if let Some(pl) = self.player.as_mut() {
                                        pl.position = at;
                                        pl.velocity = Vec3::ZERO;
                                        pl.pitch = 0.0;
                                    }
                                    self.previous_position = at;
                                    self.pitch = 0.0;
                                    self.snap_player_model_pitch();
                                }
                            }
                            // Entity.DATA_SHARED_FLAGS_ID's sprinting bit: the
                            // server stops the sprint after a hit's extra
                            // knockback (the flag only, as assignValues sets
                            // it: the speed modifier and wasSprinting stay).
                            // Its invisible bit (Entity.isInvisible): the
                            // Invisibility effect's, or a plugin's that hides
                            // the real body under a pose (#3788).
                            (0, Value::Byte(flags)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.sprinting = flags & 0x08 != 0;
                                }
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_invisible = flags & 0x20 != 0;
                                }
                            }
                            // Entity.DATA_AIR_SUPPLY_ID and
                            // Player.DATA_PLAYER_ABSORPTION_ID: the bubbles
                            // and golden hearts the server's player has.
                            (1, Value::Int(air)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    pl.air = *air;
                                }
                            }
                            // LivingEntity.DATA_ARROW_COUNT_ID and DATA_STINGER_COUNT_ID.
                            (12, Value::Int(count)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_arrows = *count;
                                }
                            }
                            (13, Value::Int(count)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_stingers = *count;
                                }
                            }
                            (19 | 20, Value::Id(v)) => {
                                if let Some(r) = self.remote.as_mut() {
                                    r.own_shoulders[usize::from(*index == 20)] =
                                        (*v > 0).then(|| (*v - 1).clamp(0, 4));
                                }
                            }
                            (17, Value::Float(amount)) => {
                                if let Some(pl) = self.player.as_mut() {
                                    let _ = pl.internal_set_absorption(*amount);
                                }
                            }
                            // LivingEntity.DATA_LIVING_ENTITY_FLAGS: the server
                            // starts or stops this player's use, as an axe that
                            // disables the raised shield does (Player.disableShield's
                            // stopUsingItem). LocalPlayer.onSyncedDataUpdated.
                            (8, Value::Byte(flags)) => {
                                let selected = self.selected;
                                let using = self.using;
                                self.using = synced_local_use(using, *flags, selected, |slot| {
                                    self.stack_in(slot).is_some()
                                });
                                // onSyncedDataUpdated's useItemRemaining: getUseDuration,
                                // without startUsingItem's event.
                                if using.is_none()
                                    && let Some((slot, _)) = self.using
                                    && let Some(stack) = self.stack_in(slot).cloned()
                                {
                                    self.use_duration = self.use_duration(&stack);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // Entity.DATA_SHARED_FLAGS_ID's crouching and sprinting bits:
                // another player's or a mannequin's crouch (their model and
                // Entity.isDiscrete's name tag), and the dust a runner raises.
                if (mannequin
                    || self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.level_players.contains_key(&p.id)))
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                {
                    for (index, _, value) in &p.values {
                        if let (0, Value::Byte(flags)) = (index, value) {
                            m.extra.crouching = flags & 2 != 0;
                            m.extra.sprinting = flags & 8 != 0;
                        }
                    }
                }
                for (index, _, value) in &p.values {
                    if *index == ITEM_ENTITY_ITEM
                        && let Value::Item(stack) = value
                    {
                        self.remote_item_data(p.id, stack);
                    }
                    // ExperienceOrb.DATA_VALUE (the icon's size).
                    if *index == 8
                        && let Value::Int(v) = value
                        && let Some(o) = self.orbs.iter_mut().find(|o| o.id == p.id as u32)
                    {
                        o.value = (*v).max(0) as u32;
                    }
                }
                self.remote_object_data(p.id, &p.values);
                self.remote_projectile_data(p.id, &p.values);
                self.remote_dragon_data(p.id, &p.values);
            }
            // ClientPacketListener.handleSetEquipment: what a mob or another
            // player holds and wears (EquipmentSlot ordinals).
            C::SetEquipment(p) => {
                self.remote_object_equipment(p.id, &p.slots);
                let items: Vec<(u8, Option<Arc<str>>, Option<ItemStack>)> = p
                    .slots
                    .iter()
                    .map(|(slot, stack)| {
                        (
                            *slot,
                            stack.as_ref().map(|s| self.item_name(s.item)),
                            self.to_stack(stack),
                        )
                    })
                    .collect();
                // CrossbowItem.isCharged: the main hand's `charged_projectiles`
                // (AvatarRenderer.getArmPose holds a loaded crossbow up).
                let charged = p.slots.iter().find(|(slot, _)| *slot == 0).map(|(_, s)| {
                    s.as_ref().is_some_and(|s| {
                        matches!(
                            s.component(horizon_protocol::item::component::CHARGED_PROJECTILES),
                            Some(horizon_protocol::item::Component::Stacks(list))
                                if !list.is_empty()
                        )
                    })
                });
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    if let Some(charged) = charged
                        && &*m.kind.id == "minecraft:player"
                    {
                        m.extra.charged = charged;
                    }
                    // The body slot (6: horse armour, llama carpet, harness)
                    // is kept in `head`, as Horizon's own mobs do.
                    let body = horizon_client::mobs::drops::head_is_body(m.species());
                    for (slot, item, stack) in items {
                        let e = &mut m.extra;
                        match slot {
                            0 => match stack {
                                // The components the server sent (a loaded
                                // crossbow's `charged_projectiles`, dyed
                                // leather, an enchanted book's id, ...) are
                                // what the item model draws.
                                Some(stack) => e.set_main_hand_stack(Some(stack)),
                                None => e.set_main_hand(item),
                            },
                            1 => e.off_hand = item,
                            2 => {
                                e.feet = item;
                                e.armor[3] = stack;
                            }
                            3 => {
                                e.legs = item;
                                e.armor[2] = stack;
                            }
                            4 => {
                                e.chest = item;
                                e.armor[1] = stack;
                            }
                            5 if !body => {
                                e.head = item;
                                e.armor[0] = stack;
                            }
                            6 if body => {
                                // A wolf's armor shows its cracks.
                                e.body_damage = stack.as_ref().map_or(0, |s| s.damage as i32);
                                e.head = item;
                            }
                            7 => m.set_saddle(stack),
                            _ => {}
                        }
                    }
                }
            }
            C::TakeItemEntity(p) => {
                // ClientPacketListener.handleTakeItemEntity: the pickup sound at
                // the item or arrow, whoever collects it (the server sends none).
                // An orb's sound comes from the server, at the orb; an entity
                // this client doesn't track sounds at this player.
                let orb = self.orbs.iter().any(|o| o.id == p.item as u32);
                let item = self.dropped.iter().find(|d| d.id == p.item as u32);
                let shot = self
                    .creatures
                    .projectiles
                    .iter()
                    .find(|s| s.id == p.item as u32);
                let at = match (item, shot) {
                    (Some(d), _) => Some([d.position.x, d.position.y, d.position.z]),
                    (None, Some(s)) => Some([s.position.x, s.position.y, s.position.z]),
                    (None, None)
                        if self
                            .remote
                            .as_ref()
                            .is_some_and(|r| r.entity_id == p.player) =>
                    {
                        self.eye_position()
                    }
                    (None, None) => None,
                };
                if !orb && let Some(at) = at {
                    let pitch = ((self.world.random.next_float() - self.world.random.next_float())
                        * 0.7
                        + 1.0)
                        * 2.0;
                    self.audio
                        .play("minecraft:entity.item.pickup", Some(at), 0.2, pitch);
                }
                // ItemPickupParticle toward the collector (this player, another one or a mob).
                let collector = if self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.entity_id == p.player)
                {
                    super::pickups::Collector::Player
                } else {
                    super::pickups::Collector::Mob(p.player as u32)
                };
                if let Some(item) = self.dropped.iter().find(|d| d.id == p.item as u32).cloned() {
                    self.add_pickup(super::pickups::Taken::Item(Box::new(item)), collector);
                } else if let Some(orb) = self.orbs.iter().find(|o| o.id == p.item as u32).cloned()
                {
                    self.add_pickup(super::pickups::Taken::Orb(Box::new(orb)), collector);
                } else if let Some(shot) = self
                    .creatures
                    .projectiles
                    .iter()
                    .find(|s| s.id == p.item as u32)
                    .cloned()
                {
                    // An arrow or trident flies to its collector too, then goes.
                    self.add_pickup(super::pickups::Taken::Arrow(Box::new(shot)), collector);
                }
                // An orb stays: the server may still hold merged ones in it.
                if !orb {
                    self.remove_entity(p.item);
                }
            }
            // ClientPacketListener.handleSetEntityMotion: knockback on this player.
            C::SetEntityMotion(p) if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) => {
                if let Some(pl) = self.player.as_mut() {
                    pl.velocity = Vec3::new(p.velocity[0], p.velocity[1], p.velocity[2]);
                }
            }
            // The same packet for another entity: Entity.lerpMotion.
            C::SetEntityMotion(p) => self.remote_motion(p.id, p.velocity),
            // ClientPacketListener.handleProjectilePowerPacket: the power a
            // hurting projectile accelerates by, sent bundled with its motion.
            C::ProjectilePower(p) => {
                super::remote_projectiles::remote_projectile_power(
                    &mut self.creatures.projectiles,
                    p.id,
                    p.power,
                );
            }
            // ClientPacketListener.handleDamageEvent: LivingEntity.handleDamageEvent
            // jolts the walk animation and restarts the hurt flash, and the
            // hurt player's own client plays its hurt sound (`Player.getHurtSound`:
            // the damage type's effects) at `getSoundVolume` with a pitch that
            // walks 1.0 ± 0.2 on the level random. The server's sound packet
            // reaches every other player around, never this one (#4149).
            // This player's own hurt tilt and flash come from the server too
            // (sent to the player itself): its own moves never hurt it.
            C::DamageEvent(p) => {
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    let at = self
                        .player
                        .as_ref()
                        .map(|pl| [pl.position.x, pl.position.y, pl.position.z]);
                    if let Some(pl) = self.player.as_mut() {
                        pl.health.animate_hurt();
                    }
                    if let Some(at) = at {
                        let sound = self.remote_hurt_sound(p.source_type);
                        let pitch = 1.0
                            + (self.world.random.next_float() - self.world.random.next_float())
                                * 0.2;
                        self.audio.play_on(
                            horizon_client::settings::sound_source(sound),
                            sound,
                            Some(at),
                            1.0,
                            pitch,
                        );
                    }
                } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    m.limb_speed = 1.5;
                    m.hurt_time = 10;
                } else {
                    self.remote_dragon_hurt(p.id);
                }
            }
            C::HurtAnimation(p) => {
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    // Player.animateHurt: the tilt turns around the yaw.
                    if let Some(pl) = self.player.as_mut() {
                        pl.health.animate_hurt_dir(p.yaw);
                    }
                } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    m.hurt_time = 10;
                } else {
                    self.remote_dragon_hurt(p.id);
                }
            }
            C::EntityEvent(p) => {
                if p.event == super::fireworks::FIREWORK_EXPLODE_EVENT {
                    self.remote_firework_explosion(p.id);
                }
                self.remote_fangs_event(p.id as u32, p.event);
                self.remote_boat_event(p.id, p.event);
                // Arrow.handleEntityEvent 0: a tipped arrow's burst of particles.
                if p.event == 0 {
                    self.remote_arrow_puff(p.id as u32);
                }
                // LivingEntity.handleEntityEvent 2: a spear's hit felt.
                if p.event == 2 {
                    self.remote_kinetic_hit(p.id);
                }
                // LocalPlayer.setPermissions also refreshes an open World Options screen.
                if (24..=28).contains(&p.event)
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                {
                    self.ui.registries.permission = (p.event - 24) as u8;
                    self.remote_world_options_permission(p.event >= 26);
                }
                // Ravager.handleEntityEvent 69: the roar's puff and push.
                if p.event == 69 {
                    self.remote_ravager_roar(p.id);
                }
                // ClientPacketListener.handleEntityEvent 21: a guardian's hum.
                if p.event == 21 {
                    self.start_guardian_sound(p.id as u32);
                }
                // Witch.handleEntityEvent 15: the magic puffs over its hat.
                if p.event == 15 {
                    self.witch_magic_particles(p.id as u32);
                }
                // Fox.handleEntityEvent 45: the crumbs of the food in its mouth.
                if p.event == 45 {
                    self.fox_eat_particles(p.id as u32);
                }
                if p.event == 1
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                    && m.species() == horizon_client::mobs::Species::Rabbit
                {
                    horizon_client::mobs::rabbit::hop_event(m);
                }
                if p.event == 19
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                {
                    horizon_client::mobs::aquatic::squid_cycle_event(m);
                }
                // LivingEntity.handleEntityEvent 55: another player swapped
                // its main and off hand (`swapHandItems`).
                if p.event == 55
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                {
                    std::mem::swap(&mut m.extra.main_hand, &mut m.extra.off_hand);
                }
                // LivingEntity.handleEntityEvent 47 to 52, 65 and 68: an
                // equipped item broke (onEquippedItemBroken).
                if let Some(slot) = horizon_client::mobs::item_break::equipment_break_slot(p.event)
                {
                    self.remote_equipment_break(p.id, slot);
                }
                // EntityEvent.DEATH: start the death animation.
                if p.event == 3
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32)
                {
                    m.health = 0.0;
                }
                // LivingEntity.handleEntityEvent 3 on this player: its death
                // sound, which Player.playSound plays for this client alone
                // (the server's goes to everyone else).
                if p.event == 3
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                    && let Some(pl) = self.player.as_ref()
                {
                    let at = [pl.position.x, pl.position.y, pl.position.z];
                    let sound = "minecraft:entity.player.death";
                    let pitch = 1.0
                        + (self.world.random.next_float() - self.world.random.next_float()) * 0.2;
                    self.audio.play_on(
                        horizon_client::settings::sound_source(sound),
                        sound,
                        Some(at),
                        1.0,
                        pitch,
                    );
                }
                // The events that start a mob's animation (the warden's).
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    horizon_client::mobs::sync::entity_event(m, p.event as u8);
                }
                // LivingEntity.handleEntityEvent 60: the poof as the dead body goes.
                if p.event == 60
                    && let Some(m) = self.mobs.iter().find(|m| m.id == p.id as u32)
                {
                    let (at, w, h) = (m.position, f64::from(m.width()), f64::from(m.height()));
                    self.death_poof([at.x, at.y, at.z], w, h);
                }
                if p.event == super::totem::TOTEM_EVENT {
                    self.remote_totem(p.id);
                }
                // Mob.handleEntityEvent 20: the spawn poof.
                if p.event == 20 && self.mobs.iter().any(|m| m.id == p.id as u32) {
                    self.poof_particles(p.id as u32);
                }
                // Villager 12, 13 and 14, TamableAnimal and AbstractHorse 6 and
                // 7, Animal and Allay 18: the particles about the mob.
                if matches!(p.event, 6 | 7 | 12 | 13 | 14 | 18 | 40 | 41) {
                    self.entity_event_particles(p.id as u32, p.event);
                }
                // Snowball and ThrownEgg.handleEntityEvent 3: the crumbs where
                // the projectile is (the server removes it right after).
                if p.event == 3 {
                    let crumbs = self
                        .creatures
                        .projectiles
                        .iter()
                        .find(|pr| pr.id == p.id as u32)
                        .and_then(|pr| {
                            let item = match pr.kind {
                                horizon_client::projectiles::ProjectileKind::Snowball => {
                                    "minecraft:snowball"
                                }
                                horizon_client::projectiles::ProjectileKind::Egg => "minecraft:egg",
                                horizon_client::projectiles::ProjectileKind::Thrown(t) => t.item,
                                _ => return None,
                            };
                            Some(([pr.position.x, pr.position.y, pr.position.z], item))
                        });
                    if let Some((at, item)) = crumbs {
                        self.item_burst(at, item);
                    }
                }
                if p.event == super::combat::DROWN_EVENT {
                    self.remote_drown(p.id);
                }
                // Entity event 53: the honey slide particles of a player
                // sliding down a honey block (HoneyBlock.showSlideParticles).
                if p.event == 53 {
                    let at = if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                        self.player
                            .as_ref()
                            .map(|q| [q.position.x, q.position.y, q.position.z])
                    } else {
                        self.mobs
                            .iter()
                            .find(|m| m.id == p.id as u32)
                            .map(|m| [m.position.x, m.position.y, m.position.z])
                    };
                    if let Some(at) = at {
                        self.honey_slide_particles(at);
                    }
                }
                if p.event == super::consume::USE_ITEM_COMPLETE_EVENT
                    && self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                {
                    self.remote_complete_using();
                }
            }
            C::Animate(p) => self.remote_animate(p.id, p.action),
            // ClientPacketListener.handleSwingAnimation: the entity swings
            // (LivingEntity.swing, which a swing in its first half ignores).
            C::SwingAnimation(p) => {
                let hand = if p.hand == 1 { Hand::Off } else { Hand::Main };
                let animation = horizon_client::first_person::SwingAnimation {
                    kind: p.kind.clamp(0, 255) as u8,
                    duration: p.duration,
                };
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
                    self.swing_hand(hand, animation);
                } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == p.id as u32) {
                    m.swing_hand_modified(hand == Hand::Off, animation);
                }
            }
            // The player's own instances: its maximum health (the hearts the
            // HUD draws and the health it may hold), reach, speed and so on.
            // Then the received health again, which the old maximum clamped.
            C::UpdateAttributes(p) if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) => {
                if let Some(pl) = self.player.as_mut() {
                    horizon_client::lan::attributes::apply(&mut pl.attributes, &p.values);
                }
                self.apply_remote_health();
            }
            C::UpdateAttributes(p) => self.remote_mob_attributes(p.id, &p.values),
            C::UpdateMobEffect(p) => self.remote_effect(&p),
            C::RemoveMobEffect(p) => {
                if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id)
                    && let Some(id) = self.remote_effect_id(p.effect)
                {
                    self.potions.effects.list.retain(|e| e.id != id);
                }
            }
            C::ChangeDifficulty(p) => {
                self.ui.difficulty = p.difficulty.clamp(0, 3) as u8;
                self.level.difficulty_locked = p.locked;
                self.remote_world_options_difficulty(self.ui.difficulty, p.locked);
            }
            C::GameRuleValues(p) => self.remote_world_options_rules(p.entries),
            // ClientPacketListener.handleSetHealth: LocalPlayer.hurtTo.
            C::SetHealth(p) => {
                let flash = self.remote.as_mut().is_some_and(|r| {
                    r.health = (p.health, p.food, p.saturation);
                    std::mem::replace(&mut r.flash_on_set_health, true)
                });
                if flash && let Some(pl) = self.player.as_mut() {
                    pl.health.flash_health_change(p.health);
                }
                self.apply_remote_health();
                // The death screen opens (or the respawn is asked for) with the
                // death packet when the rule hides it.
                let shown = self.remote.as_ref().is_none_or(|r| r.show_death_screen);
                if p.health <= 0.0 && shown && self.screen != Screen::Dead && self.player.is_some()
                {
                    self.close_dialogs();
                    self.screen = Screen::Dead;
                    self.ui.death_time = 0;
                    self.ui.death_confirm = None;
                    self.capture(false);
                }
            }
            C::PlayerCombatKill(p) => {
                // The component resolves in the client's current language, as
                // the chat line does, not in the language the session joined
                // with (`Remote.lang` is the vanilla server-language table).
                let message = remote_text(&p.message);
                let me = self.remote.as_ref().is_some_and(|r| r.entity_id == p.id);
                if let Some(r) = self.remote.as_mut() {
                    r.death_message = Some(message);
                }
                // ClientPacketListener.handlePlayerCombatKill: the death
                // screen, whatever health was last sent, or `respawn()` at
                // once when it is hidden.
                if me && self.screen != Screen::Dead && self.player.is_some() {
                    if self.remote.as_ref().is_none_or(|r| r.show_death_screen) {
                        self.close_dialogs();
                        self.screen = Screen::Dead;
                        self.ui.death_time = 0;
                        self.ui.death_confirm = None;
                        self.capture(false);
                    } else {
                        self.respawn_player()?;
                    }
                }
            }
            C::TrackedWaypoint(p) => self.handle_waypoint(&p),
            C::SetExperience(p) => {
                self.experience = Experience {
                    level: p.level,
                    progress: p.progress,
                    total: p.total,
                }
            }
            // ClientPacketListener.handleSetTime: the game time, then
            // ClientClockManager.handleUpdates: the clock catches up with the
            // game time and takes the server's state. Only this level's
            // default world clock is applied — a packet that only updates
            // another clock (the End's) must not overwrite day_time, or the
            // sky follows game_time and looks inverted vs the server's day.
            C::SetTime(p) => {
                self.world.game_time = p.game_time;
                self.remote_clock_tick();
                let clock = {
                    let r = self.remote.as_ref();
                    let names =
                        r.and_then(|r| r.registries.registries.get("minecraft:world_clock"));
                    let tracked = r.and_then(|r| r.clock);
                    let wanted = r
                        .and_then(|r| r.dimension.default_clock.clone())
                        .or_else(|| r.map(|r| r.dimension.name.clone()))
                        .unwrap_or_else(|| "minecraft:overworld".into());
                    let name = |id: i32| {
                        names
                            .and_then(|n| n.get(id as usize))
                            .map(|(n, _)| n.as_str())
                    };
                    p.clocks
                        .iter()
                        .find(|c| name(c.clock) == Some(wanted.as_str()))
                        .or_else(|| tracked.and_then(|id| p.clocks.iter().find(|c| c.clock == id)))
                        .cloned()
                };
                if let Some(c) = clock {
                    self.world.day_time = c.total_ticks;
                    if let Some(r) = self.remote.as_mut() {
                        r.clock = Some(c.clock);
                        r.clock_rate = c.rate;
                        r.clock_partial = c.partial_tick;
                    }
                }
            }
            C::GameEvent(p) => match p.event {
                // NO_RESPAWN_BLOCK_AVAILABLE: ClientPacketListener.handleGameEvent
                // sends the player this system message.
                0 => self.system_message(horizon_client::text::Text::translate(
                    "block.minecraft.spawn.not_valid",
                    &[],
                )),
                // LEVEL_CHUNKS_LOAD_START: the server has begun sending the level.
                13 => {
                    if let Some(r) = self.remote.as_mut() {
                        r.level_load.loading_packets_received();
                    }
                }
                // GUARDIAN_ELDER_EFFECT: the apparition, and the curse sound
                // unless the elder guardian is silent.
                10 => {
                    self.creatures.elder_apparition = super::mob_blocks::APPARITION_TICKS;
                    if p.param == 1.0
                        && let Some(at) = self.player.as_ref().map(|m| m.position)
                    {
                        self.audio.play(
                            "minecraft:entity.elder_guardian.curse",
                            Some([at.x, at.y, at.z]),
                            1.0,
                            1.0,
                        );
                    }
                }
                // WIN_GAME: the poem and credits, then PERFORM_RESPAWN.
                4 if self.screen != Screen::Win => {
                    self.open_win_screen(true, super::win::Finish::RemoteRespawn);
                }
                1 => {
                    self.world.weather.raining = true;
                    self.world.weather.rain_level = (0.0, 0.0);
                }
                2 => {
                    self.world.weather.raining = false;
                    self.world.weather.rain_level = (1.0, 1.0);
                }
                3 => {
                    let mode = p.param as i32;
                    self.set_mode_id(mode);
                    if let Some(r) = self.remote.as_mut() {
                        r.game_mode = mode;
                    }
                }
                // IMMEDIATE_RESPAWN: LocalPlayer.setShowDeathScreen.
                11 => {
                    if let Some(r) = self.remote.as_mut() {
                        r.show_death_screen = p.param == 0.0;
                    }
                }
                7 => {
                    self.world.weather.rain_level = (p.param, p.param);
                    self.world.weather.raining = p.param > 0.0;
                }
                8 => {
                    self.world.weather.thunder_level = (p.param, p.param);
                    self.world.weather.thundering = p.param > 0.0;
                }
                _ => {}
            },
            C::PlayerAbilities(p) => {
                self.creative = p.flags & 8 != 0;
                if let Some(r) = self.remote.as_mut() {
                    r.abilities = Some(p);
                }
                self.apply_remote_abilities();
            }
            // ClientPacketListener.handleSetHeldSlot: a hotbar slot only, and the
            // carried index stays, so the next tick answers with SetCarriedItem.
            C::SetHeldSlot(p) => {
                if (0..9).contains(&p.slot) {
                    self.selected = p.slot as usize;
                }
            }
            C::ContainerSetContent(p) => {
                self.remote_menu_state(p.container, p.state);
                for (slot, item) in p.items.iter().enumerate() {
                    let stack = self.to_stack(item);
                    self.set_menu_slot(p.container, slot as i32, stack);
                }
                self.carried = self.to_stack(&p.carried);
                self.remote_synced();
            }
            C::ContainerSetSlot(p) => {
                self.remote_menu_state(p.container, p.state);
                let stack = self.to_stack(&p.item);
                self.tutorial_get_item(stack.as_ref());
                // ClientPacketListener.handleContainerSetSlot: a hotbar slot that
                // gains items squeezes for 5 ticks.
                if p.container == 0
                    && (36..=45).contains(&p.slot)
                    && let Some(new) = &stack
                {
                    let (pop, old) = match p.slot {
                        45 => (9, self.offhand.as_ref()),
                        s => (s as usize - 36, self.inventory[s as usize - 36].as_ref()),
                    };
                    if old.is_none_or(|old| old.count < new.count) {
                        self.hotbar_pop[pop] = 5;
                    }
                }
                let before = self.creative_stacks();
                self.set_menu_slot(p.container, i32::from(p.slot), stack);
                self.remote_synced_since(&before);
            }
            C::SetCursorItem(p) => {
                let stack = self.to_stack(&p.item);
                self.tutorial_get_item(stack.as_ref());
                self.carried = stack;
            }
            C::SetPlayerInventory(p) => {
                let stack = self.to_stack(&p.item);
                self.tutorial_get_item(stack.as_ref());
                let before = self.creative_stacks();
                let held = match p.slot {
                    0..=35 => self.inventory[p.slot as usize].as_ref(),
                    40 => self.offhand.as_ref(),
                    _ => None,
                };
                let changed = !super::hand::matches_for_swap(held, stack.as_ref());
                if let Ok(slot) = usize::try_from(p.slot) {
                    self.remote_hand_slot_set(slot, changed);
                }
                match p.slot {
                    0..=35 => self.inventory[p.slot as usize] = stack,
                    36..=39 => self.armor[3 - (p.slot - 36) as usize] = stack,
                    // Inventory.SLOT_OFFHAND.
                    40 => self.offhand = stack,
                    _ => {}
                }
                self.remote_synced_since(&before);
            }
            C::ContainerSetData(p) if self.open_menu_name(p.container) == "minecraft:lectern" => {
                self.remote_lectern_page(i32::from(p.value));
            }
            C::ContainerSetData(p) if self.remote_station_open(p.container) => {
                self.remote_station_data(usize::try_from(p.id).unwrap_or(usize::MAX), p.value);
            }
            C::ContainerSetData(p) if self.native_screen_data(p.container, p.id, p.value) => {}
            C::ContainerSetData(p) => {
                let v = i32::from(p.value);
                match self
                    .container
                    .and_then(|key| self.world.block_entities.get_mut(&key))
                {
                    Some(BlockEntity::Furnace(f)) => match p.id {
                        0 => f.lit_remaining = v,
                        1 => f.lit_total = v,
                        2 => f.cooking_timer = v,
                        3 => f.cooking_total = v,
                        _ => {}
                    },
                    // CrafterBlockEntity.containerData.
                    Some(BlockEntity::Crafter(c)) => match usize::try_from(p.id) {
                        Ok(k @ 0..9) => c.disabled[k] = v == 1,
                        Ok(9) => c.triggered = v == 1,
                        _ => {}
                    },
                    // BrewingStandBlockEntity.dataAccess.
                    Some(BlockEntity::Brewing(b)) => match p.id {
                        0 => b.brew_time = v,
                        1 => b.fuel = v,
                        2 => b.total_brew_time = v,
                        3 => b.total_fuel = v,
                        _ => {}
                    },
                    _ => {}
                }
            }
            C::OpenScreen(p) => self.remote_open_screen(p),
            C::MountScreenOpen(p) => self.remote_open_mount(p),
            C::RecipeBookAdd(p) => self.remote_recipe_book_add(p),
            C::RecipeBookRemove(p) => self.remote_recipe_book_remove(p),
            C::RecipeBookSettings(p) => self.remote_recipe_book_settings(p),
            C::PlaceGhostRecipe(p) => self.remote_ghost_recipe(p),
            C::MerchantOffers(p) => self.remote_merchant_offers(p),
            C::ContainerClose(_) if self.remote_lectern_closed() => {}
            C::ContainerClose(_) => {
                if self.screen == Screen::Inventory {
                    self.remote_close_local();
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
            }
            C::TagQuery(p) => self.answer_tag_query(p.transaction, p.tag),
            C::Unknown(id, data)
                if id == horizon_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS =>
            {
                self.handle_command_suggestions(&data);
            }
            C::CustomChatCompletions(p) => {
                // ClientPacketListener.handleCustomChatCompletions.
                if let Some(r) = self.remote.as_mut() {
                    horizon_client::command::modify_custom_completions(
                        &mut r.custom_completions,
                        horizon_client::command::CustomCompletionAction::from_id(p.action),
                        &p.entries,
                    );
                    self.ui.registries.custom_completions = r.custom_completions.clone();
                }
            }
            C::Commands(p) => {
                let tree = horizon_client::command::network::from_packet(
                    &p,
                    horizon_client::command::vanilla::dispatcher(),
                );
                // The chat's tree follows the server's now; a mod's commands
                // go into it again (`chat_tree_for`).
                let chat_tree = self.chat_tree_for(Some(&tree));
                if let Some(r) = self.remote.as_mut() {
                    r.chat_tree = chat_tree;
                    r.chat_tree_received = true;
                }
            }
            C::Cooldown(p) => {
                // ClientPacketListener.handleItemCooldown: the server starts
                // (or confirms) a group's cooldown; 0 ends it.
                let group: Arc<str> = Arc::from(p.group.as_str());
                if p.duration == 0 {
                    self.cooldowns.remove(&group);
                } else {
                    self.cooldowns.add(group, p.duration);
                }
            }
            // ClientPacketListener.handleGameTestHighlightPos.
            C::GameTestHighlightPos(p) => self.highlight_test_pos(p.absolute, p.relative),
            // ClientPacketListener.handleTestInstanceBlockStatus.
            C::TestInstanceBlockStatus(p) => {
                self.block_editor_status(remote_text(&p.status), p.size);
            }
            C::SystemChat(p) => {
                // The component as sent, translated in the client's own
                // language. ChatListener.handleOverlay for the action bar.
                let text = remote_text(&p.content);
                if p.overlay {
                    self.say_system_queued(&text);
                    self.titles.set_overlay(text, false);
                } else if self.chat_line(text.clone(), horizon_client::chat::ChatSource::Server) {
                    // handleSystemMessage: said once the chat takes it.
                    self.say_system_chat_queued(&text);
                }
            }
            C::PlayerChat(p) => {
                use horizon_client::scoreboard::tag_to_json;
                // SignedMessageBody.Packed.unpack: the last seen it names must
                // be in the cache; then the message joins it
                // (MessageSignatureCache.push).
                let Some(r) = self.remote.as_mut() else {
                    return Ok(());
                };
                // handlePlayerChat: the global index counts every chat packet; one out of
                // order disconnects.
                let expected = r.next_chat_index;
                r.next_chat_index = r.next_chat_index.wrapping_add(1);
                if p.global_index != expected {
                    log::error!(
                        "Missing or out-of-order chat message from server, expected index {expected} but got {}",
                        p.global_index
                    );
                    let reason = horizon_client::text::Text::translate(
                        "multiplayer.disconnect.bad_chat_index",
                        &[],
                    );
                    return self.leave_server(Some(reason.plain()));
                }
                let last_seen: Option<Vec<Signature>> =
                    p.last_seen.iter().map(|s| r.signatures.unpack(s)).collect();
                let Some(last_seen) = last_seen else {
                    log::error!(
                        "Message from player with ID {:032x} referenced unrecognized signature id",
                        p.sender
                    );
                    let reason = horizon_client::text::Text::translate(
                        "multiplayer.disconnect.invalid_packet",
                        &[],
                    );
                    return self.leave_server(Some(reason.plain()));
                };
                let signature = p
                    .signature
                    .as_deref()
                    .and_then(horizon_protocol::secure_chat::signature);
                r.signatures.push(&last_seen, signature.as_ref());
                // The sender's PlayerInfo: none is an error line; else their
                // `SignedMessageValidator` decides (`updateAndValidate`), and a message it
                // takes without a signature of its own shows as unsigned.
                if !r.tab_players.contains_key(&p.sender) {
                    log::error!(
                        "Received player chat packet for unknown player with ID: {:032x}",
                        p.sender
                    );
                    self.handle_chat_message_error(&p.chat_type, signature.as_ref());
                    return Ok(());
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                let incoming = secure_chat::Incoming {
                    sender: p.sender,
                    index: p.index,
                    signature: signature.as_ref(),
                    body: secure_chat::Body {
                        content: &p.content,
                        timestamp: p.timestamp,
                        salt: p.salt,
                        last_seen: &last_seen,
                    },
                    unsigned_content: p.unsigned_content.as_ref(),
                    filter: &p.filter,
                };
                let Some(validated) = r.validate_chat(&incoming, now) else {
                    self.handle_chat_message_error(&p.chat_type, signature.as_ref());
                    return Ok(());
                };
                let signature = signature.filter(|_| validated == secure_chat::Validated::Signed);
                // ChatListener.handlePlayerChatMessage: Only Show Secure Chat
                // shows the signed content, never the server's replacement.
                // The narrator says the message's own decorated content
                // (`narrateChatMessage(boundChatType, message.decoratedContent())`).
                let decorated = p
                    .unsigned_content
                    .as_ref()
                    .map_or_else(|| serde_json::Value::from(p.content.as_str()), tag_to_json);
                let content = if self.settings.file.bool("onlyShowSecureChat") {
                    serde_json::Value::from(p.content.as_str())
                } else {
                    decorated.clone()
                };
                let narration = self
                    .decorate_chat(&p.chat_type, &decorated, Decoration::Narration)
                    .plain();
                let line = self.decorate_chat(&p.chat_type, &content, Decoration::Chat);
                let trust = self.chat_trust(
                    p.sender,
                    signature.is_some(),
                    p.timestamp,
                    &p.content,
                    p.unsigned_content.as_ref(),
                    &line,
                );
                self.handle_player_chat(line, signature, trust, narration);
            }
            C::DisguisedChat(p) => {
                let content = horizon_client::scoreboard::tag_to_json(&p.message);
                let narration = self
                    .decorate_chat(&p.chat_type, &content, Decoration::Narration)
                    .plain();
                let line = self.decorate_chat(&p.chat_type, &content, Decoration::Chat);
                self.handle_player_chat(line, None, ChatTrust::Secure, narration);
            }
            C::DeleteChat(p) => self.handle_delete_chat(&p.signature)?,
            C::PlayerInfoUpdate(p) => {
                if let Some(r) = self.remote.as_mut() {
                    // handlePlayerInfoUpdate: new entries first, then every
                    // action on every known entry.
                    // INITIALIZE_CHAT: the session (or none) each entry carries; the
                    // player's validator is made again when their chat is next read.
                    if p.actions & 2 != 0 {
                        for e in &p.entries {
                            r.chat_sessions.insert(e.uuid, e.chat_session.clone());
                            r.chat_validators.remove(&e.uuid);
                        }
                    }
                    if p.actions & 1 != 0 {
                        for e in &p.entries {
                            if let Some(name) = &e.name {
                                r.tab_players.entry(e.uuid).or_insert_with(|| {
                                    horizon_client::tab_list::TabPlayer::new(e.uuid, name.clone())
                                });
                                if let Some(t) = e.properties.iter().find(|p| p.name == "textures")
                                {
                                    // Without the services' profile property keys a
                                    // signature can't be checked; a signed property
                                    // counts as genuine.
                                    use horizon_client::skins::{self, Signature};
                                    let signature = if t.signature.is_some() {
                                        Signature::Signed
                                    } else {
                                        Signature::Unsigned
                                    };
                                    let textures =
                                        skins::unpack(&t.value, signature, &skins::TEXTURE_URIS);
                                    r.skin_textures.entry(e.uuid).or_insert(textures);
                                }
                            }
                        }
                    }
                    for e in p.entries {
                        if let Some(t) = r.tab_players.get_mut(&e.uuid) {
                            if let Some(mode) = e.game_mode {
                                t.game_mode = mode;
                            }
                            if let Some(latency) = e.latency {
                                t.latency = latency;
                            }
                            if let Some(name) = &e.display_name {
                                t.display_name =
                                    name.as_ref().map(horizon_client::scoreboard::tag_to_json);
                            }
                            if let Some(order) = e.list_order {
                                t.list_order = order;
                            }
                            if let Some(hat) = e.show_hat {
                                t.show_hat = hat;
                            }
                            match e.listed {
                                Some(true) => {
                                    r.listed.insert(e.uuid);
                                }
                                Some(false) => {
                                    r.listed.remove(&e.uuid);
                                }
                                None => {}
                            }
                        }
                        if let Some(name) = e.name {
                            r.players.insert(e.uuid, name);
                        }
                    }
                }
            }
            C::PlayerInfoRemove(p) => {
                if let Some(r) = self.remote.as_mut() {
                    for u in p.uuids {
                        r.players.remove(&u);
                        r.tab_players.remove(&u);
                        r.chat_sessions.remove(&u);
                        r.chat_validators.remove(&u);
                        r.skin_textures.remove(&u);
                        r.listed.remove(&u);
                    }
                }
            }
            C::TabList(p) => {
                // handleTabListCustomisation: empty text clears it.
                let text = |t: &Tag| {
                    let json = horizon_client::scoreboard::tag_to_json(t);
                    let text = horizon_client::scoreboard::comp_text(&json);
                    text.spans
                        .iter()
                        .any(|span| !span.text.is_empty())
                        .then_some(text)
                };
                self.tab_list.header = text(&p.header);
                self.tab_list.footer = text(&p.footer);
            }
            C::InitializeBorder(p) => {
                let b = &mut self.border;
                b.set_center([p.center_x, p.center_z]);
                if p.lerp_time > 0 {
                    b.lerp_between(p.old_size, p.new_size, p.lerp_time);
                } else {
                    b.set_size(p.new_size);
                }
                b.set_absolute_max_size(p.absolute_max_size);
                b.warning_distance = p.warning_blocks;
                b.warning_time = p.warning_time;
            }
            C::SetBorderCenter(p) => self.border.set_center([p.x, p.z]),
            C::SetBorderLerpSize(p) => {
                self.border
                    .lerp_between(p.old_size, p.new_size, p.lerp_time)
            }
            C::SetBorderSize(p) => self.border.set_size(p.size),
            C::SetBorderWarningDelay(p) => self.border.warning_time = p.delay,
            C::SetBorderWarningDistance(p) => self.border.warning_distance = p.blocks,
            C::SetObjective(p) => self.remote_objective(p),
            C::SetDisplayObjective(p) => {
                // DisplaySlot ids: list, sidebar, below_name, then the team sidebars.
                let slot = match p.slot {
                    0 => "list".to_owned(),
                    2 => "below_name".to_owned(),
                    n @ 3..=18 => format!(
                        "sidebar.team.{}",
                        horizon_client::text::COLORS[(n - 3) as usize].1
                    ),
                    _ => "sidebar".to_owned(),
                };
                let board = &mut self.score.board;
                let objective = board.objective(&p.objective).map(|o| o.name.clone());
                board.set_display_objective(&slot, objective.as_deref());
            }
            C::SetScore(p) => {
                let board = &mut self.score.board;
                if board.objective(&p.objective).is_some() {
                    let s = board.score_mut(&p.owner, &p.objective);
                    s.value = p.score;
                    s.display = p
                        .display
                        .as_ref()
                        .map(horizon_client::scoreboard::tag_to_json);
                    s.number_format = p.number_format.as_ref().map(number_format);
                }
            }
            C::ResetScore(p) => match &p.objective {
                Some(o) => self.score.board.reset_one(&p.owner, o),
                None => self.score.board.reset_all(&p.owner),
            },
            C::SetPlayerTeam(p) => self.remote_team(p),
            C::SetSimulationDistance(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.simulation_distance = p.distance;
                }
            }
            C::SetChunkCacheRadius(p) => {
                self.streaming.radius = self.settings.render_distance.min(p.distance + 1).max(2);
                if let Some(r) = self.remote.as_mut() {
                    r.view_distance = p.distance;
                }
                // ClientChunkCache.updateViewRadius: a new storage of that size.
                self.drop_remote_chunks_out_of_range();
            }
            C::SetChunkCacheCenter(p) => {
                if let Some(r) = self.remote.as_mut() {
                    r.chunk_center = (p.x, p.z);
                }
                self.drop_remote_chunks_out_of_range();
            }
            C::BlockEntityData(p) => self.remote_block_entity_data(p.pos, p.kind, &p.tag),
            C::OpenSignEditor(p) => self.remote_open_sign_editor(p),
            C::OpenBook(p) => self.remote_open_book(p.hand),
            // handleSetSpawn: ClientLevel.setRespawnData, which spawn compasses point at.
            C::SetDefaultSpawnPosition(p) => {
                let [x, y, z] = p.pos.pos;
                self.spawn = (p.pos.dimension == "minecraft:overworld")
                    .then(|| Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5));
            }
            C::ServerData(p) => self.remote_server_data(p.icon.as_deref()),
            C::MapItemData(p) => self.remote_map_item_data(&p),
            // handleTickingState / handleTickingStep: the level's TickRateManager.
            C::TickingState(p) => {
                self.clock.set_rate(p.rate);
                self.clock.set_frozen(p.frozen);
            }
            C::TickingStep(p) => self.clock.set_frozen_ticks_to_run(p.steps),
            C::UpdateAdvancements(p) => {
                let icons = p
                    .added
                    .iter()
                    .map(|a| {
                        a.display
                            .as_ref()
                            .map_or_else(String::new, |d| self.item_name(d.icon.item).to_string())
                    })
                    .collect();
                self.remote_advancements(p, icons);
            }
            // handleAwardStats: StatsCounter.setValue for each.
            C::AwardStats(p) => {
                let Some(r) = self.remote.as_ref() else {
                    return Ok(());
                };
                let mut known = Vec::new();
                for (ty, value, count) in p.stats {
                    let Some(ty) = r
                        .builtin
                        .get("minecraft:stat_type")
                        .and_then(|t| t.name(ty))
                    else {
                        continue;
                    };
                    let values = match ty {
                        "minecraft:mined" => "minecraft:block",
                        "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
                        "minecraft:custom" => "minecraft:custom_stat",
                        _ => "minecraft:item",
                    };
                    if let Some(key) = r.builtin.get(values).and_then(|v| v.name(value)) {
                        known.push((ty.to_owned(), key.to_owned(), count));
                    }
                }
                for (ty, key, count) in known {
                    self.stats.values.entry(ty).or_default().insert(key, count);
                }
            }
            // ClientPacketListener.setActionBarText, setTitleText,
            // setSubtitleText, setTitlesAnimation and handleTitlesClear.
            C::BossEvent(p) => self.apply_boss_event(&p),
            C::SetActionBarText(p) => self.titles.set_overlay(remote_text(&p.text), false),
            C::SetTitleText(p) => self.titles.set_title(remote_text(&p.text)),
            C::SetSubtitleText(p) => self.titles.set_subtitle(remote_text(&p.text)),
            C::SetTitlesAnimation(p) => self.titles.set_times(p.fade_in, p.stay, p.fade_out),
            // ClientCommonPacketListenerImpl.handleShowDialog / handleClearDialog.
            C::ShowDialog(p) => self.show_play_dialog(&p.dialog),
            C::ClearDialog(_) => self.clear_dialog(),
            // ClientPacketListener.handleLowDiskSpaceWarning.
            C::LowDiskSpaceWarning(_) => horizon_client::chunk_io::report_low_disk_space(),
            C::ClearTitles(p) => {
                self.titles.clear();
                if p.value {
                    self.titles.reset_times();
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// `handleAddObjective`: add, remove or change an objective.
    fn remote_objective(&mut self, p: play::SetObjective) {
        use horizon_client::scoreboard::{RenderType, tag_to_json};
        let board = &mut self.score.board;
        match (p.method, p.display) {
            (1, _) => board.remove_objective(&p.name),
            (method, Some((name, render, format))) => {
                let render = if render == 1 {
                    RenderType::Hearts
                } else {
                    RenderType::Integer
                };
                let format = format.as_ref().map(number_format);
                if method == 0 {
                    board.add_objective(
                        &p.name,
                        "dummy",
                        tag_to_json(&name),
                        render,
                        false,
                        format,
                    );
                } else if let Some(o) = board.objective_mut(&p.name) {
                    o.display_name = tag_to_json(&name);
                    o.render_type = render;
                    o.number_format = format;
                }
            }
            _ => {}
        }
    }
    /// `handleSetPlayerTeamPacket`.
    fn remote_team(&mut self, p: play::SetPlayerTeam) {
        use horizon_client::scoreboard::{CollisionRule, Visibility, tag_to_json};
        let board = &mut self.score.board;
        if p.method == 1 {
            board.remove_team(&p.name);
            return;
        }
        if p.method == 0 {
            board.add_team(&p.name);
        }
        if let Some(params) = &p.parameters
            && let Some(team) = board.team_mut(&p.name)
        {
            team.display_name = tag_to_json(&params.display_name);
            team.prefix = tag_to_json(&params.prefix);
            team.suffix = tag_to_json(&params.suffix);
            team.color = params
                .color
                .and_then(|c| horizon_client::text::COLORS.get(c as usize))
                .map(|c| c.1.to_owned());
            team.allow_friendly_fire = params.options & 1 != 0;
            team.see_friendly_invisibles = params.options & 2 != 0;
            team.nametag_visibility = Visibility::ALL
                .get(params.name_tag_visibility as usize)
                .copied()
                .unwrap_or(Visibility::Always);
            team.collision_rule = CollisionRule::ALL
                .get(params.collision_rule as usize)
                .copied()
                .unwrap_or(CollisionRule::Always);
        }
        match p.method {
            0 | 3 => {
                for player in &p.players {
                    board.add_to_team(player, &p.name);
                }
            }
            // Scoreboard.removePlayerFromTeam: only out of this team (vanilla
            // throws for a player in another one).
            4 => {
                for player in &p.players {
                    if board.players_team(player).is_some_and(|t| t.name == p.name) {
                        board.remove_from_team(player);
                    }
                }
            }
            _ => {}
        }
    }
    /// The player list's view of the connection: the listed players (with
    /// their downloaded skins once ready), online mode, and which players
    /// are in the level with an upside-down name.
    pub(super) fn remote_tab_players(
        &mut self,
    ) -> Option<(Vec<horizon_client::tab_list::TabPlayer>, bool, Vec<u128>)> {
        self.update_remote_skins();
        let mut players: Vec<_> = {
            let r = self.remote.as_ref()?;
            r.listed
                .iter()
                .filter_map(|u| r.tab_players.get(u).cloned())
                .collect()
        };
        for p in &mut players {
            p.skin = self
                .remote
                .as_ref()
                .and_then(|r| r.skins.get(&p.uuid).cloned())
                .unwrap_or_else(|| horizon_client::tab_list::default_skin(p.uuid).to_owned());
            self.gui
                .atlas
                .set_object_skin(format!("id:{:032x}", p.uuid), p.skin.clone());
            self.gui.atlas.set_object_skin(
                format!("name:{}", p.name.to_ascii_lowercase()),
                p.skin.clone(),
            );
        }
        let r = self.remote.as_ref()?;
        let upside_down = players
            .iter()
            .filter(|p| {
                (p.name == "Dinnerbone" || p.name == "Grumm")
                    && (p.uuid == r.profile || r.level_players.values().any(|u| *u == p.uuid))
            })
            .map(|p| p.uuid)
            .collect();
        Some((players, r.online_mode, upside_down))
    }

    /// Resolve inline `object:player` profiles through the same skin cache as
    /// player heads on items. The next frame uses the uploaded atlas texture.
    pub(super) fn resolve_inline_profiles(&mut self, profiles: &[serde_json::Value]) {
        use horizon_client::skins;
        for profile in profiles {
            let skin = skins::head_skin(
                profile,
                &mut self.skin_profiles.borrow_mut(),
                &mut self.skin_downloads.borrow_mut(),
                |location| self.gui.atlas.sprites.contains_key(location),
            );
            let id = skins::parse_profile(profile).id;
            let name = profile
                .as_str()
                .or_else(|| profile.get("name").and_then(serde_json::Value::as_str));
            // The player list's skin for the same profile is drawn too.
            let keys = [
                Some(format!("id:{id:032x}")),
                name.map(|n| format!("name:{}", n.to_ascii_lowercase())),
            ];
            for key in keys.iter().flatten() {
                if let Some(listed) = self.gui.atlas.object_skins.get(key)
                    && listed.starts_with("skins/")
                {
                    let listed = listed.clone();
                    self.again_skin(&listed);
                }
            }
            self.gui
                .atlas
                .set_object_skin(format!("id:{id:032x}"), skin.clone());
            if let Some(name) = name {
                self.gui
                    .atlas
                    .set_object_skin(format!("name:{}", name.to_ascii_lowercase()), skin);
            }
        }
    }
    fn poll_own_profile(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        match &mut r.own_profile {
            OwnProfile::Idle => {
                let Some(auth) = horizon_client::auth::current() else {
                    r.own_profile = OwnProfile::Done(None);
                    return;
                };
                let (send, recv) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .name("Own profile".into())
                    .spawn(move || {
                        let _ = send.send(horizon_client::auth::own_textures(auth.as_ref()));
                    })
                    .ok();
                r.own_profile = OwnProfile::Fetching(recv);
            }
            OwnProfile::Fetching(recv) => {
                if let Ok(own) = recv.try_recv() {
                    r.own_profile = OwnProfile::Done(own);
                }
            }
            OwnProfile::Done(_) => {}
        }
    }
    /// `PlayerInfo.getSkin` for every player entity, including NPCs that are
    /// never in the tab list: resolve each profile's textures and give the
    /// level's player models their skin.
    pub(super) fn update_remote_skins(&mut self) {
        let local = self.remote.as_ref().map(|r| r.profile);
        self.poll_own_profile();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let local = local.unwrap_or(r.profile);
        let mut entries = skin_entries(
            &r.level_players,
            &r.skin_textures,
            &mut r.level_player_textures,
        );
        let own = match &r.own_profile {
            OwnProfile::Done(Some(t)) => Some(t.clone()),
            _ => None,
        };
        // Minecraft.profileFuture: the signed-in account's active skin and cape
        // from `/minecraft/profile` when the server sent none (single player) or
        // only a skin without the equipped cape.
        if let Some(own) = own {
            if let Some((_, t)) = entries.iter_mut().find(|(u, _)| *u == local) {
                if t.skin.is_none() {
                    t.skin = own.skin.clone();
                    if own.skin.is_some() {
                        t.slim = own.slim;
                    }
                }
                if t.cape.is_none() {
                    t.cape = own.cape.clone();
                }
            } else if own.skin.is_some() || own.cape.is_some() {
                entries.retain(|(u, _)| *u != local);
                entries.push((local, own));
            }
        }
        let mut resolved: Vec<(u128, String)> = Vec::new();
        // PlayerSkin.model of each resolved skin: the textures' `slim`.
        let mut slim = std::collections::HashSet::new();
        // PlayerSkin.cape of each resolved skin.
        let mut capes: HashMap<u128, String> = HashMap::new();
        for (uuid, textures) in entries {
            // PlayerInfo.createSkinLookup: only the local player's own skin
            // may be unsigned.
            let skin = {
                let mut downloads = self.skin_downloads.borrow_mut();
                horizon_client::skins::lookup(&textures, uuid != local, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            if let Some(skin) = skin {
                if textures.slim {
                    slim.insert(uuid);
                }
                if let Some(cape) = horizon_client::skins::cape(&textures) {
                    capes.insert(uuid, cape);
                }
                resolved.push((uuid, skin));
            }
        }
        // An offline name's skin from the launcher, where neither the server
        // nor an account gives the local player one.
        if !resolved.iter().any(|(u, _)| *u == local)
            && let Some((rgba, is_slim)) = horizon_client::skins::offline_skin()
        {
            const OFFLINE: &str = "horizon/offline_skin";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.clone());
            }
            if *is_slim {
                slim.insert(local);
            }
            resolved.push((local, OFFLINE.into()));
        }
        // An offline name's cape from the launcher, where the profile has none.
        if !capes.contains_key(&local)
            && let Some(rgba) = horizon_client::skins::offline_cape()
        {
            const OFFLINE: &str = "capes/offline";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.to_vec());
            }
            capes.insert(local, OFFLINE.into());
        }
        // PlayerInfo.getSkin: the downloaded skin and its model, else the
        // profile's default skin (DefaultPlayerSkin.get), slim or wide.
        let skin_of = |skins: &HashMap<u128, String>, uuid: u128| match skins.get(&uuid) {
            Some(skin) => (skin.clone(), slim.contains(&uuid)),
            None => {
                let skin = horizon_client::tab_list::default_skin(uuid);
                (skin.to_owned(), skin.contains("/slim/"))
            }
        };
        let Some((level, skins, own)) = self.remote.as_mut().map(|r| {
            r.skins = resolved.into_iter().collect();
            (
                r.level_players
                    .iter()
                    .map(|(id, uuid)| (*id as u32, *uuid))
                    .collect::<Vec<_>>(),
                r.skins.clone(),
                r.profile,
            )
        }) else {
            return;
        };
        // Only the skins drawn hold atlas slots: the level's players, not
        // every entry of a busy server's player list.
        for (id, uuid) in level {
            let Some(i) = self
                .mobs
                .iter()
                .position(|m| m.id == id && &*m.kind.id == "minecraft:player")
            else {
                continue;
            };
            let (skin, is_slim) = self.drawn_skin(uuid, skin_of(&skins, uuid));
            let cape = capes.get(&uuid).filter(|c| self.again_skin(c)).cloned();
            let mob = &mut self.mobs[i];
            mob.skin = Some(skin);
            mob.slim = is_slim;
            mob.cape = cape;
        }
        if self.player_model.is_some() {
            let (skin, is_slim) = self.drawn_skin(own, skin_of(&skins, own));
            // Avatar.isModelPartShown(CAPE): the Skin Customization option.
            let cape = capes
                .get(&own)
                .filter(|_| self.settings.file.bool("modelPart_cape"))
                .filter(|c| self.again_skin(c))
                .cloned();
            if let Some(mob) = self.player_model.as_mut() {
                mob.skin = Some(skin);
                mob.slim = is_slim;
                mob.cape = cape;
            }
        }
        // ClientMannequin.updateSkin (PlayerSkinRenderCache.lookup): a
        // mannequin's DATA_PROFILE `textures` property resolves like a
        // player's, never required to be signed.
        let mannequins: Vec<(u32, horizon_client::skins::Textures)> = self
            .mobs
            .iter()
            .filter(|m| &*m.kind.id == "minecraft:mannequin" && m.profile.is_some())
            .map(|m| {
                let profile = m.profile.as_ref();
                let t = profile.and_then(|p| p.properties.iter().find(|p| p.name == "textures"));
                let textures = if let Some(t) = t {
                    let signature = if t.signature.is_some() {
                        horizon_client::skins::Signature::Signed
                    } else {
                        horizon_client::skins::Signature::Unsigned
                    };
                    horizon_client::skins::unpack(
                        &t.value,
                        signature,
                        &horizon_client::skins::TEXTURE_URIS,
                    )
                } else {
                    // `ResolvableProfile.resolveProfile`: no `textures` and
                    // only a name or an id resolves through the services.
                    let key = profile.and_then(|p| match (&p.name, p.uuid) {
                        (Some(name), None) => {
                            Some(horizon_client::skins::NameOrId::Name(name.clone()))
                        }
                        (None, Some(id)) => Some(horizon_client::skins::NameOrId::Id(id)),
                        _ => None,
                    });
                    key.and_then(|key| match self.skin_profiles.borrow_mut().state(&key) {
                        horizon_client::skins::Resolve::Done(Some(profile)) => {
                            profile.textures().map(|(value, signature)| {
                                horizon_client::skins::unpack(
                                    &value,
                                    signature,
                                    &horizon_client::skins::TEXTURE_URIS,
                                )
                            })
                        }
                        _ => None,
                    })
                    .unwrap_or_default()
                };
                (m.id, textures)
            })
            .collect();
        let mut resolved: Vec<(u32, Option<String>, bool)> = Vec::new();
        for (id, textures) in mannequins {
            let skin = {
                let mut downloads = self.skin_downloads.borrow_mut();
                horizon_client::skins::lookup(&textures, false, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            let slim = skin.is_some() && textures.slim;
            resolved.push((id, skin, slim));
        }
        for (id, skin, slim) in resolved {
            let skin = skin.filter(|s| !s.starts_with("skins/") || self.again_skin(s));
            if let Some(mob) = self
                .mobs
                .iter_mut()
                .find(|m| m.id == id && &*m.kind.id == "minecraft:mannequin")
            {
                mob.slim = slim && skin.is_some();
                mob.skin = skin;
            }
        }
    }
    /// Puts a downloaded skin in an atlas skin slot (`SkinSlots::place`),
    /// unless every slot holds a skin drawn in the last frame.
    pub(super) fn store_skin(&mut self, location: String, rgba: Vec<u8>) {
        let Some((slot, evicted)) = self.skin_slots.place(location.clone()) else {
            return;
        };
        if let Some(old) = evicted {
            self.gui.atlas.sprites.remove(&old);
        }
        let Some(mut rect) = self
            .gui
            .atlas
            .sprites
            .get(&format!("horizon/skin{slot}"))
            .copied()
        else {
            return;
        };
        let (w, h) = (self.gui.atlas.width as f32, self.gui.atlas.height as f32);
        self.map_patches.push(horizon_client::font::Patch {
            x: (rect.u0 * w).round() as u32,
            y: (rect.v0 * h).round() as u32,
            width: 64,
            height: 64,
            rgba,
        });
        // A cape is a 64x32 texture in the top half of the slot (skins::process_cape).
        if location.starts_with("capes/") {
            rect.v1 = (rect.v0 + rect.v1) / 2.0;
        }
        self.gui.atlas.sprites.insert(location, rect);
    }
    /// Whether a downloaded skin or cape is in the atlas to draw, marking it
    /// drawn; if not (new, or its slot taken), its image is queued for upload
    /// again and it is drawn from the next frame.
    fn again_skin(&mut self, location: &str) -> bool {
        let mut downloads = self.skin_downloads.borrow_mut();
        // A slot without its sprite (the atlas was rebuilt, or the upload
        // never happened) counts as missing, so the skin is stored again.
        if self.skin_slots.contains(location) && self.gui.atlas.sprites.contains_key(location) {
            downloads.drawn(location);
            true
        } else {
            downloads.again(location);
            false
        }
    }
    /// A player's skin and model as drawn: the downloaded skin once it is in
    /// the atlas, until then the profile's default (`DefaultPlayerSkin.get`),
    /// as `PlayerInfo.getSkin` gives before the texture has loaded.
    pub(super) fn drawn_skin(
        &mut self,
        uuid: u128,
        (skin, slim): (String, bool),
    ) -> (String, bool) {
        if (skin.starts_with("skins/") || skin == "horizon/offline_skin") && !self.again_skin(&skin)
        {
            let skin = horizon_client::tab_list::default_skin(uuid);
            return (skin.to_owned(), skin.contains("/slim/"));
        }
        (skin, slim)
    }
    /// `drawn_skin` for a face in the player list or spectator menu.
    pub(super) fn face_skin(&mut self, uuid: u128, skin: &str) -> String {
        self.drawn_skin(uuid, (skin.to_owned(), false)).0
    }
    /// ChatType.Bound.decorate (`Decoration::Chat`) and `decorateNarration`
    /// (`Decoration::Narration`): the chat type's decoration from the
    /// server's registry, applied to `content`.
    fn decorate_chat(
        &self,
        bound: &play::ChatTypeBound,
        content: &serde_json::Value,
        decoration: Decoration,
    ) -> horizon_client::text::Text {
        use horizon_client::scoreboard::comp_text;
        let Some(r) = self.remote.as_ref() else {
            return comp_text(content);
        };
        let chat_type = r.registries.data("minecraft:chat_type", bound.chat_type);
        comp_text(&chat_type_decorate(chat_type, decoration, bound, content))
    }
    fn remote_item_data(&mut self, id: i32, stack: &Option<Stack>) {
        let Some(stack) = self.to_stack(stack) else {
            return;
        };
        let position = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.remove(&id));
        if let Some(item) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
            item.stack = stack;
            return;
        }
        let Some((p, velocity)) = position else {
            return;
        };
        let at = Vec3::new(p[0], p[1], p[2]);
        let bob_offset = self.world.random.next_float() * std::f32::consts::PI * 2.0;
        self.dropped.push(ItemEntity {
            health: 5,
            fire_ticks: 0,
            thrower: None,
            target: None,
            id: id as u32,
            stack,
            position: at,
            previous: at,
            velocity,
            on_ground: false,
            age: 0,
            pickup_delay: 0,
            bob_offset,
            ticks: 0,
            needs_sync: false,
        });
    }
    /// `Entity.lerpMotion` (and `recreateFromPacket`'s motion) for the
    /// entities whose own tick moves them on a client: dropped items, experience
    /// orbs, falling blocks and primed TNT.
    fn remote_motion(&mut self, id: i32, velocity: [f64; 3]) {
        self.remote_dragon_motion(id, velocity);
        let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id as u32) {
            // Squid.aiStep derives its body pitch and roll from network motion.
            m.velocity = v;
        } else if let Some(d) = self.dropped.iter_mut().find(|d| d.id == id as u32) {
            d.velocity = v;
        } else if let Some(f) = self.falling.iter_mut().find(|f| f.id == id as u32) {
            f.velocity = v;
        } else if let Some(t) = self.tnts.iter_mut().find(|t| t.id == id as u32) {
            t.velocity = v;
        } else if let Some(o) = self.orbs.iter_mut().find(|o| o.id == id as u32) {
            o.velocity = v;
        } else if let Some(p) = self
            .creatures
            .projectiles
            .iter_mut()
            .find(|p| p.id == id as u32)
        {
            p.velocity = v;
        } else if let Some(b) = self.dragon_fireballs.iter_mut().find(|b| b.id == id as u32) {
            b.velocity = v;
        } else if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id as u32) {
            t.velocity = v;
        } else if let Some(r) = self.rockets.iter_mut().find(|r| r.id == id as u32) {
            r.velocity = v;
        } else if self.remote_hook_motion(id as u32, v) {
        } else if let Some(p) = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_items.get_mut(&id))
        {
            p.1 = v;
        } else if !self.potions.remote_thrown_motion(id as u32, v) {
            self.end.remote_eye_motion(id as u32, velocity);
        }
    }
    fn apply_remote_health(&mut self) {
        let Some((health, food, saturation)) = self.remote.as_ref().map(|r| r.health) else {
            return;
        };
        if let Some(p) = self.player.as_mut() {
            if (p.health.health() - health).abs() > f32::EPSILON {
                let _ = p.set_health(health.max(0.0), false);
            }
            p.health.food.food = food;
            p.health.food.saturation = saturation;
        }
    }
    /// `ServerPlayer.dismountTo` lands a rider at the ghast's own (x, maxY, z)
    /// (`HappyGhast.getDismountLocationForPassenger`). A vanilla client
    /// simulates the ghast it rides, so its copy is where the server has it;
    /// this window's copy trails the server's by the stepped interpolation and
    /// would rise into the rider's feet, which lets the rider fall through.
    /// Put the copy under the landing spot.
    fn place_ghast_under(&mut self, id: u32, spot: Vec3) {
        let Some(m) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id && m.species() == horizon_client::mobs::Species::HappyGhast)
        else {
            return;
        };
        let at = Vec3::new(spot.x, spot.y - f64::from(m.height()), spot.z);
        // Only a teleport that is that dismount: right above the ghast.
        if (at.x - m.position.x).powi(2) + (at.z - m.position.z).powi(2) > 25.0
            || (at.y - m.position.y).abs() > 5.0
        {
            return;
        }
        m.position = at;
        m.previous = at;
        if let Some(handler) = self
            .remote
            .as_mut()
            .and_then(|r| r.stepped.get_mut(&(id as i32)))
        {
            handler.cancel();
        }
    }
    /// `ClientPacketListener.handleLookAt`: `Entity.lookAt` from this
    /// player's feet or eyes, towards the packet's entity (its feet or eyes)
    /// when this client tracks it, else towards the packet's point.
    fn remote_look_at(&mut self, p: &play::PlayerLookAt) {
        // `EntityAnchorArgument.Anchor`: 0 feet, 1 eyes.
        const EYES: i32 = 1;
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let mut from = [player.position.x, player.position.y, player.position.z];
        if p.from == EYES {
            from[1] += f64::from(player.dimensions.eye_height);
        }
        let to = p
            .entity
            .as_ref()
            .and_then(|e| {
                let m = self.mobs.iter().find(|m| m.id == e.id as u32)?;
                let y = if e.anchor == EYES {
                    m.eye_y()
                } else {
                    m.position.y
                };
                Some([m.position.x, y, m.position.z])
            })
            .unwrap_or(p.target);
        (self.yaw, self.pitch) = look_at_rotation(from, to);
    }
    /// ClientPacketListener.handleMovePlayer.
    fn remote_teleport(&mut self, p: &play::PlayerPosition) -> Result<()> {
        use play::relative as rel;
        let current = self
            .player
            .as_ref()
            .map(|pl| pl.position)
            .unwrap_or(Vec3::ZERO);
        let velocity = self
            .player
            .as_ref()
            .map(|pl| pl.velocity)
            .unwrap_or(Vec3::ZERO);
        let pick = |bit: i32, base: f64, v: f64| if p.relatives & bit != 0 { base + v } else { v };
        let position = Vec3::new(
            pick(rel::X, current.x, p.position[0]),
            pick(rel::Y, current.y, p.position[1]),
            pick(rel::Z, current.z, p.position[2]),
        );
        let yaw = if p.relatives & rel::Y_ROT != 0 {
            self.yaw + p.yaw
        } else {
            p.yaw
        };
        let relative_pitch = p.relatives & rel::X_ROT != 0;
        let pitch = position_packet_pitch(self.pitch, p.pitch, relative_pitch);
        let delta = horizon_client::player_sync::teleport_velocity(
            velocity,
            (self.yaw, self.pitch),
            (yaw, pitch),
            p,
        );
        match self.player.as_mut() {
            Some(player) => {
                player.position = position;
                player.velocity = delta;
            }
            None => {
                let mut player = PlayerMotion::new(self.motion, position)?;
                // Player.tick derives noPhysics from the current game mode.
                player.spectator = self.spectator();
                player.velocity = delta;
                self.player = Some(player);
                self.pad.player_replaced();
                self.sway = Default::default();
                self.eye = 1.62;
                self.previous_eye = 1.62;
                self.clock.reset();
                self.apply_remote_health();
                self.apply_remote_abilities();
            }
        }
        self.previous_position = position;
        self.yaw = yaw;
        self.pitch = pitch;
        // setValuesFromPositionPacket: setOldPosAndRot gives xRotO the same
        // change, so a relative pitch keeps the model's ease and an absolute
        // one snaps it.
        if let Some(m) = self.player_model.as_mut() {
            m.previous_head_pitch =
                position_packet_pitch(m.previous_head_pitch, p.pitch, relative_pitch);
            m.head_pitch = pitch;
        }
        if let Some(id) = self.remote.as_mut().and_then(|r| r.ghast_dismount.take()) {
            self.place_ghast_under(id, position);
        }
        if let Some(r) = self.remote.as_mut() {
            // handleMovePlayer does not change LocalPlayer.sendPosition's
            // baseline: the next tick must still report the corrected state.
            r.destroying = None;
            r.is_destroying = false;
        }
        self.mining = None;
        self.send(S::AcceptTeleportation(play::AcceptTeleportation {
            id: p.teleport,
            x: position.x,
            y: position.y,
            z: position.z,
            yaw,
            pitch,
        }));
        Ok(())
    }

    /// `ClientPacketListener.tick`'s `LevelLoadTracker.tickClientLoad`: the
    /// player's section is shown once its column is lit; when the level is
    /// ready the loading screen closes and the server hears `PlayerLoaded`
    /// (`notifyPlayerLoaded`).
    fn tick_client_load(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if !self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active())
        {
            return;
        }
        // `LevelRenderer`'s playerCompiledSectionCallback: the section holding
        // the camera is there (the column is lit, as meshing needs).
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        let section_ready = self.streaming.is_lit(cx, cz);
        let camera_y = (player.position.y + f64::from(self.eye)).floor() as i32;
        let shape = self.world.shape;
        let check = horizon_client::level_loading::PlayerCheck {
            outside_build_height: camera_y < shape.min_y || camera_y >= shape.min_y + shape.height,
            spectator: player.spectator,
            alive: player.health.health() > 0.0,
        };
        let now = client_millis();
        // LevelLoadingScreen.tick.
        self.tick_load_progress();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if section_ready && r.level_load.wants_player_section() {
            r.level_load.player_section_ready();
        }
        r.level_load.tick_client_load(now, check);
        if !r.level_load.is_level_ready(now) {
            return;
        }
        r.level_load.finish();
        if !r.loaded_sent {
            r.loaded_sent = true;
            self.send(S::PlayerLoaded(Empty));
        }
        // LevelLoadingScreen.onClose.
        self.say_system_now(&horizon_client::text::Text::translate(
            "narrator.ready_to_play",
            &[],
        ));
        self.capture_loaded_world();
    }

    /// One client tick while connected (Minecraft.tick with a ClientLevel).
    pub(super) fn tick_remote(&mut self) -> Result<()> {
        self.tick_remote_level()?;
        // Client mods tick on any server, single player's own included.
        self.native_net.to_server = 0;
        self.native_tick();
        self.tell_native_now();
        self.tick_chat_session();
        // Minecraft.tick sends this even when the player cannot tick yet,
        // including while waiting for chunks. Keep it outside early returns.
        if self.remote.as_ref().is_some_and(|r| r.joined) {
            self.send(S::ClientTickEnd(Empty));
        }
        Ok(())
    }

    fn tick_remote_level(&mut self) -> Result<()> {
        self.ticks += 1;
        // Hud.tick ends with ChatComponent.tick.
        self.ui.chat.tick(&self.gui.atlas, self.ticks);
        self.tick_sign_editor();
        self.track_experience_display();
        self.tick_enchanting_book();
        // Leaving a slot clears its bundle's selection here and on the server.
        self.tick_bundle_hover();
        // Options.broadcastOptions: a changed render distance or Chat option
        // is sent again.
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        if let Some(r) = self.remote.as_mut()
            && r.joined
            && r.sent_info != info
        {
            r.sent_info = info.clone();
            self.send(S::ClientInformation(info));
        }
        // Minecraft.tick: TickRateManager.tick; ClientLevel.tickTime advances
        // the time only while the level runs normally.
        let running = self.clock.world_runs();
        self.boss_overlay.tick_darken(running);
        self.tick_item_activation();
        if running {
            self.world.game_time += 1;
            self.remote_clock_tick();
        }
        // Weather levels drift toward their targets between server updates.
        let w = &mut self.world.weather;
        w.rain_level.0 = w.rain_level.1;
        w.thunder_level.0 = w.thunder_level.1;
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        // Minecraft.handleKeybinds precedes entity ticking and sendChanges.
        // Window callbacks only queue clicks, so an interaction cannot leak
        // between CLIENT_TICK_END and a later pong.
        self.remote_handle_input();
        // Player.tick's ItemCooldowns.tick, in ClientLevel.tickEntities after
        // handleKeybinds: this path replaces `tick_world`'s for a server's
        // client (multiplayer and the default single player), so the entries
        // advance here or the slot stays shaded until the server says the
        // entry ended. `onCooldownEnded` is the plain ItemCooldowns' no-op on
        // a client. Ticking it after the keybinds keeps a held use from
        // trying again a tick before the server's entry ends.
        self.cooldowns.tick();
        // Inventory.tick's ItemStack.inventoryTick: the pop time runs down.
        for pop in &mut self.hotbar_pop {
            *pop = pop.saturating_sub(1);
        }
        // ClientLevel.tick: the border's size change.
        self.border.tick();
        self.player_border();
        self.tick_remote_entities(running);
        // Breeze.tick's block dust: drawn by each client for the breezes it sees.
        if running {
            self.tick_breeze_effects();
        }
        self.tick_remote_guardian_eyes(running);
        // ClientLevel.tickBlockEntities: chest lids, shulker boxes, bells,
        // and the moving pistons a block event's client-side moveBlocks made
        // (their progress advances between the server's block updates).
        self.tick_block_entity_anims();
        self.tick_remote_spawners();
        self.client_tick_beacons();
        self.tick_remote_potent_sulfur();
        self.with_level(|b, level, ticks| {
            b.client_tick_block_entities(level, ticks);
        });
        let pushes = std::mem::take(&mut self.block_ticks.piston_pushes);
        self.push_piston_entities(pushes);
        self.sync_moving_pistons();
        self.tick_hand();
        if running {
            self.tick_particles();
        }
        self.tick_client_ambient();
        self.tick_client_effects();
        // Player.tick's sleep counter (the darkening and the wake-up fade).
        self.sleep_timer = super::tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        // Gui.tick: the titles, the action bar and the held item's name.
        self.tick_hud();
        self.tick_subtitles();
        self.tick_client_load();
        let entity_collisions = self.player_entity_collisions();
        let pushed = self.remote_pushes();
        let sprint_window = self.settings.sprint_window;
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
        if !self.streaming.is_lit(cx, cz) {
            // LocalPlayer.tick can wait for chunks, but Minecraft.tick still
            // calls sendChanges: rotations and the 20-tick position reminder
            // must continue, as well as moves into the unloaded area.
            if let Some(r) = self.remote.as_ref()
                && r.loaded_sent
            {
                let input = r.last_input;
                self.remote_send_state(input);
            }
            return Ok(());
        }
        let stick = self.pad_move();
        if self.player.is_none() {
            return Ok(());
        }
        // The LevelLoadingScreen takes the input until the level is ready.
        let loading = self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.is_active());
        let playing = self.screen == Screen::Playing && !loading && !self.dialog_open();
        let held = |k| playing && self.keys.contains(&k);
        let mut keys = Keys {
            forward: held(KeyCode::KeyW),
            backward: held(KeyCode::KeyS),
            left: held(KeyCode::KeyA),
            right: held(KeyCode::KeyD),
            jump: held(KeyCode::Space),
            shift: held(KeyCode::ShiftLeft),
            sprint: held(KeyCode::ControlLeft),
            analog: None,
        }
        .with_stick(stick.filter(|_| playing));
        // On a server (not this game's own server thread), the stick moves as the keys it sets do
        // (`KeyboardInput.tick`: 8 directions, full speed). The server only hears those keys
        // (`ServerboundPlayerInputPacket`), and an anticheat that predicts from them pulls back
        // a player moving at the stick's own angle and speed.
        if self.integrated.is_none() && self.server_address().is_some() {
            keys.analog = None;
        }
        let input = u8::from(keys.forward)
            | u8::from(keys.backward) << 1
            | u8::from(keys.left) << 2
            | u8::from(keys.right) << 3
            | u8::from(keys.jump) << 4
            | u8::from(keys.shift) << 5
            | u8::from(keys.sprint) << 6;
        // LocalPlayer.applyInput and aiStep: watching through another
        // entity, the player does not move itself; its keys still reach the
        // server, whose sneak check lets go of the camera.
        let keys = if self.controls_camera() {
            keys
        } else {
            Keys::default()
        };
        // A passenger moves with its vehicle (Entity.rideTick), not by itself.
        let riding = self.screen != Screen::Dead && self.remote_ride(keys);
        // The server sends no passenger's position: they sit where their
        // vehicles are now.
        self.position_riders();
        // Entity.getFluidInteractionBox asks the vehicle: a boat afloat shelters
        // its passenger's eyes (AbstractBoat.modifyPassengerFluidInteractionBox).
        let boat = self
            .riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .map(|b| horizon_client::motion::RiddenBoat {
                under_water: b.under_water(),
                max_y: b.bounds().max_position().y,
            });
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        player.fluids.boat = boat;
        if !riding {
            self.previous_position = player.position;
        }
        let before = (
            player.ground.on_ground,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        let was_flying = player.flying;
        self.sync_player_body();
        // LivingEntity.tickDeath: the local player counts its deathTime once
        // per client tick while dead (the camera's death roll and zoom).
        if self.screen == Screen::Dead {
            self.ui.death_time = self.ui.death_time.saturating_add(1);
        }
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        let was_gliding = player.fall_flying;
        let dead = self.screen == Screen::Dead;
        // LivingEntity.isImmobile: a sleeper lies still in the bed.
        let sleeping = self.sleeping;
        if let Some(head) = sleeping {
            player.position = Vec3::new(
                f64::from(head[0]) + 0.5,
                // setPosToBed: the bed's top (getSleepHeight) and 0.125 above it.
                f64::from(head[1]) + 0.5625 + 0.125,
                f64::from(head[2]) + 0.5,
            );
            player.velocity = Vec3::ZERO;
            player.pitch = 0.0;
        } else if !dead && !riding {
            player.client_side = true;
            player.velocity = player.velocity.add_vector(pushed);
            MotionWorld {
                entities: &entity_collisions,
                player,
                scene: &mut self.world,
                assets: &self.pack.assets,
                collision_binding: self.collision,
                resources: self.motion,
            }
            .controlled_simulation_tick_with_sprint_window(keys, sprint_window)?;
        } else {
            // LivingEntity.baseTick counts hurtTime down for a dead or riding
            // player too (controlled_simulation_tick does it otherwise): left
            // at 10, the hurt tilt restarted every frame and shook the camera.
            player.health.advance_clock();
            if riding && !dead {
                MotionWorld {
                    entities: &entity_collisions,
                    player: &mut *player,
                    scene: &mut self.world,
                    assets: &self.pack.assets,
                    collision_binding: self.collision,
                    resources: self.motion,
                }
                .passenger_pose_tick()?;
            }
        }
        // LocalPlayer.onUpdateAbilities: flight turned on or off (a double
        // jump, a spectator, landing) is sent to the server.
        let flying = player.flying;
        // LocalPlayer.aiStep: a glide this client started is sent to the
        // server (ServerboundPlayerCommandPacket START_FALL_FLYING).
        let started_gliding = player.fall_flying && !was_gliding;
        let events = std::mem::take(&mut player.events);
        self.previous_eye = self.eye;
        self.eye += (player.dimensions.eye_height - self.eye) * 0.5;
        let feet = [player.position.x, player.position.y, player.position.z];
        for event in events {
            match event {
                horizon_client::motion::MotionEvent::Emission { state, muffled, .. } => {
                    let sounds = self.res.facts().get(state).sounds.clone();
                    // Player.playStepSound: muffled under the swim sound in water.
                    let (volume, pitch) = if muffled {
                        (sounds.volume * 0.05, sounds.pitch * 0.8)
                    } else {
                        (sounds.volume * 0.15, sounds.pitch)
                    };
                    self.audio.play(&sounds.step, Some(feet), volume, pitch);
                }
                horizon_client::motion::MotionEvent::Swim { volume, pitch } => {
                    self.audio
                        .play("minecraft:entity.player.swim", Some(feet), volume, pitch);
                }
                // Entity.doWaterSplashEffect on the client's own level: the
                // server's splash skips this player (Player.playSound), and
                // its particles are the client's own.
                horizon_client::motion::MotionEvent::Splash {
                    high_speed,
                    volume,
                    pitch,
                } => {
                    let name = if high_speed {
                        "minecraft:entity.player.splash.high_speed"
                    } else {
                        "minecraft:entity.player.splash"
                    };
                    self.audio.play(name, Some(feet), volume, pitch);
                }
                horizon_client::motion::MotionEvent::SplashParticle {
                    bubble,
                    position,
                    velocity,
                } => {
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let kind = if bubble {
                        ParticleType::Bubble
                    } else {
                        ParticleType::Splash
                    };
                    self.particles.add(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        [velocity.x, velocity.y, velocity.z],
                    );
                }
                horizon_client::motion::MotionEvent::Chime { volume, pitch } => {
                    self.audio.play(
                        "minecraft:block.amethyst_block.chime",
                        Some(feet),
                        volume,
                        pitch,
                    );
                }
                // NetherPortalBlock.entityInside on the client's own level:
                // the portal overlay and its trigger sound (the server moves
                // the player).
                horizon_client::motion::MotionEvent::InsidePortal { .. } => {
                    self.client_inside_nether_portal();
                }
                // HoneyBlock.maybeDoSlideEffects on the client's own level:
                // the slide sound, heard here (LocalPlayer.playSound); the
                // particles come from the server's event 53.
                horizon_client::motion::MotionEvent::HoneySlide { sound: true, .. } => {
                    self.audio
                        .play("minecraft:block.honey_block.slide", Some(feet), 1.0, 1.0);
                }
                // LivingEntity.causeFallDamage on the client's own level: the
                // fall sound is heard here (LocalPlayer.playSound); the
                // server's copy skips this player (Player.playSound).
                horizon_client::motion::MotionEvent::Fall(
                    horizon_client::landing::FallEvent::Sound {
                        sound,
                        volume,
                        pitch,
                        ..
                    },
                ) => {
                    if let Some(name) = sound.player_fall_sound() {
                        self.audio.play(name, Some(feet), volume, pitch);
                    }
                }
                _ => {}
            }
        }
        // A stand riding the player follows this move (tickPassenger).
        self.seat_own_stands();
        // The server owns health and hunger.
        self.apply_remote_health();
        self.tick_avatar(before, false);
        // LivingEntity.tick: a sleeper's xRot is 0.
        if sleeping.is_some() {
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // The player's own model (third person, the inventory's preview).
        self.tick_player_model();
        if !dead {
            self.remote_tick_using();
        }
        if flying != was_flying {
            self.send(S::PlayerAbilities(play::PlayerAbilitiesRequest {
                flags: if flying { 2 } else { 0 },
            }));
        }
        if started_gliding {
            let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
            self.send(S::PlayerCommand(play::PlayerCommand {
                id,
                action: 6,
                data: 0,
            }));
        }
        // LocalPlayer.sendChanges sends `input.keyPresses`, which an auto-jump's `makeJump` set to
        // jump for this tick: the server's prediction plays the jump from it. Sent as held, the jump
        // was the client's alone, and the server saw a jump its keys didn't allow.
        let auto_jumped = self.player.as_ref().is_some_and(|p| p.was_auto_jump);
        self.remote_send_state(input | u8::from(auto_jumped) << 4);
        Ok(())
    }

    /// ClientPacketListener.handlePlayerAbilities: the player flies, may fly
    /// and is invulnerable as the server says (bits 2, 4 and 1).
    fn apply_remote_abilities(&mut self) {
        let (Some(player), Some(a)) = (
            self.player.as_mut(),
            self.remote.as_ref().and_then(|r| r.abilities.as_ref()),
        ) else {
            return;
        };
        player.invulnerable = a.flags & 1 != 0;
        player.flying = a.flags & 2 != 0;
        player.may_fly = a.flags & 4 != 0;
        player.flying_speed = a.flying_speed;
    }
    /// The local player's share of every touching entity's
    /// `LivingEntity.pushEntities`, and of the boats' pushes: on a client
    /// `EntitySelector.pushableBy` lets entities push only the local player,
    /// so the server never sends these pushes. `doPush(player)` is
    /// `player.push(entity)`.
    fn remote_pushes(&self) -> Vec3 {
        let (Some(p), Some(r)) = (self.player.as_ref(), self.remote.as_ref()) else {
            return Vec3::ZERO;
        };
        // LivingEntity.isPushable: alive, no spectator, off ladders.
        let feet = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        if self.screen == Screen::Dead
            || p.spectator
            || self.block_in_tag("minecraft:climbable", &self.block_name(feet))
        {
            return Vec3::ZERO;
        }
        let bounds = p.bounds();
        let teams = self.mob_teams();
        // `Entity.getTeam` is `Scoreboard.getPlayersTeam(getScoreboardName())`:
        // a player's score holder is its profile name, every other entity's
        // its UUID text. `mob_teams` keys by UUID and names the local player
        // `PLAYER_NAME`, which misses a server's teams for players.
        let board = &self.score.board;
        let player_team = board.players_team(&r.profile_name);
        let mut push = Vec3::ZERO;
        for m in &self.mobs {
            let listed = r
                .level_players
                .get(&(m.id as i32))
                .and_then(|uuid| r.tab_players.get(uuid));
            let spectator = listed.is_some_and(|t| t.game_mode == SPECTATOR);
            let mob_team = match listed {
                Some(t) => board.players_team(&t.name),
                None => teams.of(Who::Mob(m.id)),
            };
            // Bats never push; a spectator has no physics; passengers of the
            // same vehicle never push each other.
            if !m.alive()
                || spectator
                || m.species() == horizon_client::mobs::Species::Bat
                || self
                    .riding
                    .is_some_and(|v| v == m.id || m.extra.mount == Some(v))
                || !m.bounds().intersects(bounds)
                || !horizon_client::scoreboard::pushable_by(mob_team, player_team, true)
            {
                continue;
            }
            // Entity.push(Entity), `this` the player.
            let (mut xa, mut za) = (m.position.x - p.position.x, m.position.z - p.position.z);
            let mut dd = xa.abs().max(za.abs());
            if dd < f64::from(0.01_f32) {
                continue;
            }
            dd = dd.sqrt();
            xa /= dd;
            za /= dd;
            let pow = (1.0 / dd).min(1.0);
            xa *= pow;
            za *= pow;
            xa *= f64::from(0.05_f32);
            za *= f64::from(0.05_f32);
            push = Vec3::new(push.x - xa, push.y, push.z - za);
        }
        // AbstractBoat.push(player) from the boat's own tick (its box grown
        // by 0.2 sideways) and from the player's pushEntities (its box).
        let area = Vec3::new(f64::from(0.2_f32), f64::from(-0.01_f32), f64::from(0.2_f32));
        for b in self.boats.iter().filter(|b| self.riding != Some(b.id)) {
            let bb = b.bounds();
            let times = usize::from(
                bounds.intersects(bb.inflate(area))
                    && horizon_client::scoreboard::pushable_by(None, player_team, true),
            ) + usize::from(
                bounds.intersects(bb)
                    && horizon_client::scoreboard::pushable_by(player_team, None, true),
            );
            if times > 0
                && super::boat_passengers::boat_pushes(bb, bounds, false)
                && let Some(v) = super::boat_passengers::push_impulse(b.position, p.position)
            {
                push = push.add_vector(v.scale(times as f64));
            }
        }
        push
    }
    /// ClientLevel.tickEntities; a frozen level only ticks the players
    /// (`TickRateManager.isEntityFrozen`).
    /// `GuardianRenderer.getEntityToLookAt`: a guardian's eye follows its
    /// synched attack target, else the camera.
    fn tick_remote_guardian_eyes(&mut self, running: bool) {
        use horizon_client::mobs::{Species, Who, guardian};
        if !running {
            return;
        }
        let camera = self.player.as_ref().map(|p| {
            [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ]
        });
        let looks: Vec<_> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                matches!(m.species(), Species::Guardian | Species::ElderGuardian) && m.alive()
            })
            .filter_map(|(i, m)| {
                let look = match m.ai.target.filter(|_| m.extra.flag) {
                    Some(Who::Mob(id)) => self
                        .mobs
                        .iter()
                        .find(|t| t.id == id)
                        .map(|t| [t.position.x, t.eye_y(), t.position.z]),
                    _ => camera,
                };
                look.map(|look| (i, look))
            })
            .collect();
        for (i, look) in looks {
            guardian::look_at(&mut self.mobs[i], look);
        }
    }
    fn tick_remote_entities(&mut self, running: bool) {
        if running {
            self.tick_text_displays();
            self.tick_remote_ominous_item_spawners();
        }
        // EntityFluidInteraction.update for the entities this window only displays: the
        // server never sends the fluid contact, and the renderers read it (cod, salmon and
        // tropical fish lie on their side without `in_water`, #1591).
        // A frozen level ticks only the players, as below.
        // Entity.isInRain for the wolves, which dry off by their own `Wolf.tick`.
        let (fluids, rain, thundering): (Vec<_>, Vec<_>, bool) = {
            let w = self.mob_world_base(&[]);
            let players = self.remote.as_ref().map(|r| &r.level_players);
            let fluids = self
                .mobs
                .iter()
                .map(|m| {
                    (running || players.is_some_and(|p| p.contains_key(&(m.id as i32))))
                        .then(|| w.fluids(m.bounds(), m.eye_y()))
                })
                .collect();
            let rain = self
                .mobs
                .iter()
                .map(|m| {
                    m.species() == horizon_client::mobs::Species::Wolf
                        && horizon_client::mobs::tame::in_rain(m, &w)
                })
                .collect();
            (fluids, rain, w.thundering)
        };
        for (m, fluids) in self.mobs.iter_mut().zip(fluids) {
            if let Some(fluids) = fluids {
                m.fluids = fluids;
            }
        }
        let game_time = self.world.game_time;
        // `AbstractPiglin.isConverting`: the dimension is not piglin safe.
        let nether = self.dim == super::Dim::Nether;
        // What a wither's side heads can look at, by network entity id: the
        // mobs and the player (position and eye height).
        let wither_targets: Vec<(i32, [f64; 3], f64)> = if self
            .mobs
            .iter()
            .any(|m| m.species() == horizon_client::mobs::Species::Wither)
        {
            let own = self
                .remote
                .as_ref()
                .zip(self.eye_position())
                .map(|(r, eye)| (r.entity_id, eye, eye[1]));
            self.mobs
                .iter()
                .map(|m| {
                    (
                        m.id as i32,
                        [m.position.x, m.position.y, m.position.z],
                        m.eye_y(),
                    )
                })
                .chain(own)
                .collect()
        } else {
            Vec::new()
        };
        // ExperienceOrb.followNearbyPlayer: the players an orb may go for,
        // their feet and the point `eyeHeight / 2` above them.
        let followed: Vec<([f64; 3], [f64; 3])> = {
            let own = self
                .player
                .as_ref()
                .filter(|p| !p.health.dead_or_dying() && !self.spectator())
                .map(|p| {
                    let feet = [p.position.x, p.position.y, p.position.z];
                    let eye = f64::from(p.dimensions.eye_height);
                    (feet, [feet[0], feet[1] + eye / 2.0, feet[2]])
                });
            let others = self.remote.as_ref().into_iter().flat_map(|r| {
                self.mobs
                    .iter()
                    .filter(move |m| m.alive() && r.level_players.contains_key(&(m.id as i32)))
                    .filter(move |m| {
                        let listed = r.level_players.get(&(m.id as i32));
                        !listed
                            .and_then(|uuid| r.tab_players.get(uuid))
                            .is_some_and(|t| t.game_mode == SPECTATOR)
                    })
                    .map(|m| {
                        let feet = [m.position.x, m.position.y, m.position.z];
                        (
                            feet,
                            [feet[0], feet[1] + (m.eye_y() - feet[1]) / 2.0, feet[2]],
                        )
                    })
            });
            own.into_iter().chain(others).collect()
        };
        // The mount this client moves ticks in `remote_ride` instead, as its
        // authoritative side (LivingEntity.tick with travelRidden).
        let driven = self.driven_mount().map(|m| m.id);
        // Cat.handleLieDown: the sleeping players (another's, drawn lying in
        // a bed, and this client's own) a lying cat can lie on top of.
        let sleepers: Vec<horizon_core::geometry::Aabb> = self
            .remote
            .as_ref()
            .map(|r| {
                self.mobs
                    .iter()
                    .filter(|m| {
                        r.level_players.contains_key(&(m.id as i32)) && m.extra.ints[1] != 0
                    })
                    .map(|m| {
                        let p = m.position;
                        horizon_core::geometry::Aabb::new(
                            horizon_core::geometry::Vec3::new(p.x - 0.1, p.y, p.z - 0.1),
                            horizon_core::geometry::Vec3::new(p.x + 0.1, p.y + 0.2, p.z + 0.1),
                        )
                    })
                    .chain(self.sleeping.map(|bed| {
                        let p = horizon_core::geometry::Vec3::new(
                            f64::from(bed[0]) + 0.5,
                            f64::from(bed[1]) + 0.6875,
                            f64::from(bed[2]) + 0.5,
                        );
                        horizon_core::geometry::Aabb::new(
                            horizon_core::geometry::Vec3::new(p.x - 0.1, p.y, p.z - 0.1),
                            horizon_core::geometry::Vec3::new(p.x + 0.1, p.y + 0.2, p.z + 0.1),
                        )
                    }))
                    .collect()
            })
            .unwrap_or_default();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut lerps = std::mem::take(&mut r.lerps);
        let mut cart_lerps = std::mem::take(&mut r.cart_lerps);
        let stepped = &mut r.stepped;
        let head_lerps = &mut r.head_lerps;
        let players = &r.level_players;
        let tab_players = &r.tab_players;
        let mut using = Vec::new();
        let mut mob_fx = Vec::new();
        for (m, rain) in self.mobs.iter_mut().zip(rain) {
            let player = players.contains_key(&(m.id as i32));
            if player {
                // AvatarRenderer.extractRenderState: isSpectator, from the player list.
                m.extra.spectator = players
                    .get(&(m.id as i32))
                    .and_then(|uuid| tab_players.get(uuid))
                    .is_some_and(|t| t.game_mode == SPECTATOR);
            }
            if (!running && !player) || driven == Some(m.id) {
                continue;
            }
            m.previous = m.position;
            // `Entity.deltaMovementOnPreviousTick`: the elytra's flight yaw
            // lerps from it, so another player's model does not snap when a
            // new tick's velocity arrives.
            m.extra.previous_velocity = m.velocity;
            m.previous_yaw = m.yaw;
            m.previous_body_yaw = m.body_yaw;
            m.previous_head_yaw = m.head_yaw;
            m.previous_head_pitch = m.head_pitch;
            m.previous_limb_speed = m.limb_speed;
            m.age += 1;
            if m.species() == horizon_client::mobs::Species::Rabbit {
                horizon_client::mobs::rabbit::base_tick(m);
            }
            if matches!(
                m.species(),
                horizon_client::mobs::Species::Horse
                    | horizon_client::mobs::Species::Donkey
                    | horizon_client::mobs::Species::Mule
                    | horizon_client::mobs::Species::SkeletonHorse
                    | horizon_client::mobs::Species::ZombieHorse
            ) {
                horizon_client::mobs::equine::tail_tick(m);
            }
            if m.species() == horizon_client::mobs::Species::Creeper {
                horizon_client::mobs::hostile::creeper_client_tick(m);
            }
            if matches!(
                m.species(),
                horizon_client::mobs::Species::Guardian
                    | horizon_client::mobs::Species::ElderGuardian
            ) {
                horizon_client::mobs::guardian::client_tick(m);
            }
            // PolarBear.tick: the stand animation follows the synched flag.
            if m.species() == horizon_client::mobs::Species::PolarBear {
                horizon_client::mobs::polar_bear::client_tick(m);
            }
            // The particles these mobs' own ticks draw on the client side.
            {
                use horizon_client::mobs::{
                    Species as S, cube, illusioner, nautilus, tame, wither,
                };
                match m.species() {
                    S::Wolf => tame::client_tick(m, rain, &mut mob_fx),
                    S::Slime | S::MagmaCube | S::SulfurCube => cube::client_tick(m, &mut mob_fx),
                    S::Nautilus | S::ZombieNautilus => nautilus::client_tick(m, &mut mob_fx),
                    S::Illusioner => illusioner::client_tick(m, &mut mob_fx),
                    S::Wither => wither::client_smoke(m, &mut mob_fx),
                    _ => {}
                }
            }
            if m.hurt_time > 0 {
                m.hurt_time -= 1;
            }
            if !m.alive() {
                m.death_time += 1;
            }
            // LivingEntity.updateSwingTime.
            m.anim.attack_anim_o = m.anim.attack_anim;
            horizon_client::mobs::ai::tick_swing(m);
            // Entity.tick: the SteppedInterpolationHandler's step.
            if let Some(at) = stepped
                .get_mut(&(m.id as i32))
                .and_then(|h| h.interpolate())
            {
                place_interpolated(m, at);
            }
            // LivingEntity.aiStep: the head's own interpolation.
            if let Some((yaw, steps)) = head_lerps.get_mut(&(m.id as i32))
                && *steps > 0
            {
                m.head_yaw += wrap_degrees(*yaw - m.head_yaw) / *steps as f32;
                *steps -= 1;
            }
            // Fox.tick: the head tilt and crouch follow the synched flags.
            if m.species() == horizon_client::mobs::Species::Fox {
                horizon_client::mobs::fox::client_tick(m);
            }
            // Cat.tick: the lying and relaxing amounts follow the synched flags.
            if m.species() == horizon_client::mobs::Species::Cat {
                m.anim.vo = m.anim.v;
                horizon_client::mobs::tame::cat_amounts(m);
                horizon_client::mobs::tame::cat_lying_on_sleeper(m, &sleepers);
            }
            // Chicken.aiStep: the wings follow the server's on-ground flag.
            if m.species() == horizon_client::mobs::Species::Chicken {
                horizon_client::mobs::farm::chicken_flap(m);
            }
            // Parrot.calculateFlapping: the wings follow the server's
            // on-ground flag and the shoulder it sits on (the glide is the
            // server's).
            if m.species() == horizon_client::mobs::Species::Parrot {
                horizon_client::mobs::parrot::flap(m);
            }
            // LivingEntity.updatingUsingItem: the ticks the item has been
            // used before this one. Another player's use particles; a mob's
            // drawn bow only needs the count (its pull follows it).
            if player {
                // AvatarRenderer.getArmPose: the used item's BLOCK animation
                // (an item with `blocks_attacks`) raises the arm.
                let used = if m.extra.using_off_hand {
                    &m.extra.off_hand
                } else {
                    &m.extra.main_hand
                };
                m.extra.blocking = m.extra.using_item > 0
                    && used
                        .as_ref()
                        .and_then(|i| world_items!(self).get(i))
                        .is_some_and(|i| i.blocks_attacks.is_some());
            }
            if m.extra.using_item > 0 {
                if player {
                    let item = if m.extra.using_off_hand {
                        m.extra.off_hand.clone()
                    } else {
                        m.extra.main_hand.clone()
                    };
                    if let Some(item) = item {
                        let eye = m.eye_height();
                        let at = [m.position.x, m.position.y, m.position.z];
                        using.push((
                            item,
                            m.extra.using_item as u32 - 1,
                            at,
                            eye,
                            m.head_pitch,
                            m.yaw,
                        ));
                    }
                }
                m.extra.using_item += 1;
            }
            // LivingEntity.tick's tickHeadTurn: a mob's BodyRotationControl,
            // a player's body following its walk and look.
            if player {
                remote_player_head_turn(m);
                // LivingEntity.tick: the glide count, wings and swim amount.
                m.tick_avatar_pose(m.fluids.in_water);
                // LivingEntity.tick: a sleeper's xRot is 0.
                if m.extra.ints[1] != 0 {
                    m.head_pitch = 0.0;
                    m.previous_head_pitch = 0.0;
                }
            } else if m.alive() {
                m.body_rotation_tick();
            }
            // Squid.aiStep follows LivingEntity's interpolation and head turn.
            horizon_client::mobs::aquatic::squid_client_step(m, &mut mob_fx);
            // WalkAnimationState.update from the horizontal distance moved.
            let (dx, dz) = (m.position.x - m.previous.x, m.position.z - m.previous.z);
            let (target, factor) = if horizon_client::mobs::camel::is_camel(m.species()) {
                // Camel.updateWalkAnimation.
                horizon_client::mobs::camel::walk_target(m, (dx * dx + dz * dz).sqrt() as f32)
            } else {
                ((((dx * dx + dz * dz).sqrt() * 4.0) as f32).min(1.0), 0.4)
            };
            m.limb_speed += (target - m.limb_speed) * factor;
            m.limb_position += m.limb_speed;
            if m.species() == horizon_client::mobs::Species::Rabbit {
                horizon_client::mobs::rabbit::post_tick(m);
            }
            if m.species() == horizon_client::mobs::Species::Bat {
                horizon_client::mobs::bat::setup_animation_states(m);
            }
            // The animation states a server computes in its own `ai_step`.
            horizon_client::mobs::sync::client_tick(m, game_time);
            match m.species() {
                // WitherBoss.aiStep: the side heads turn toward their targets.
                horizon_client::mobs::Species::Wither => {
                    horizon_client::mobs::wither::client_tick(m, |id| {
                        wither_targets
                            .iter()
                            .find(|t| t.0 == id)
                            .map(|t| (t.1, t.2))
                    });
                }
                // Hoglin and piglin `isConverting` outside the Nether.
                horizon_client::mobs::Species::Hoglin
                | horizon_client::mobs::Species::Piglin
                | horizon_client::mobs::Species::PiglinBrute => {
                    horizon_client::mobs::piglin::client_converting(m, nether);
                }
                horizon_client::mobs::Species::Panda => {
                    horizon_client::mobs::panda::client_tick(m, thundering);
                }
                _ => {}
            }
        }
        // ExperienceOrb.tick runs on a client too (all but the merges and the
        // push out of a block, which are the server's): gravity, buoyancy,
        // collisions and the pull of the nearest player between the server's
        // updates; the server removes it on pickup.
        let (world, facts, collision) = (&self.world, self.res.facts(), self.collision);
        let mut lava_random = None;
        let mut next_float = || {
            lava_random
                .get_or_insert_with(super::dispenser::entity_random)
                .next_float()
        };
        if running {
            self.orbs.retain_mut(|o| {
                let distance = |feet: &[f64; 3]| -> f64 {
                    (feet[0] - o.position.x).powi(2)
                        + (feet[1] - o.position.y).powi(2)
                        + (feet[2] - o.position.z).powi(2)
                };
                let target = followed
                    .iter()
                    .filter(|(feet, _)| distance(feet) < 64.0)
                    .min_by(|a, b| distance(&a.0).total_cmp(&distance(&b.0)))
                    .map(|(_, target)| *target);
                // Entity.commonTick: setOldPosAndRot, then
                // LinearInterpolationHandler.doInterpolate moves 1/remaining of
                // the way to the server's position, so the frame draws the step
                // from the old position.
                let old = o.position;
                if let Some(l) = lerps.get_mut(&(o.id as i32)).filter(|l| l.steps > 0) {
                    let alpha = 1.0 / f64::from(l.steps);
                    o.position = Vec3::new(
                        o.position.x + (l.target[0] - o.position.x) * alpha,
                        o.position.y + (l.target[1] - o.position.y) * alpha,
                        o.position.z + (l.target[2] - o.position.z) * alpha,
                    );
                    l.steps -= 1;
                }
                let before = o.position;
                let kept = o.tick(world, facts, collision, target, false, &mut next_float);
                // `tick` set `previous` to the stepped position: keep the
                // pre-step one, as `xo` is in vanilla.
                o.previous = old;
                // AbstractInterpolationHandler.applyPredictedMovement: the orb's
                // own move carries the target along (vanilla also checks the
                // moved target for collisions; that check is left out here).
                if let Some(l) = lerps.get_mut(&(o.id as i32)).filter(|l| l.steps > 0) {
                    l.target[0] += o.position.x - before.x;
                    l.target[1] += o.position.y - before.y;
                    l.target[2] += o.position.z - before.z;
                }
                kept
            });
        }
        // ItemEntity.tick runs on a client too: gravity, collisions and
        // friction move the item between the server's updates (every 20
        // ticks), and only the server removes it.
        let (world, registry, facts, collision) = (
            &self.world,
            self.pack.assets.registry,
            self.res.facts(),
            self.collision,
        );
        for d in self.dropped.iter_mut().filter(|_| running) {
            d.tick(
                world,
                registry,
                facts,
                collision,
                false,
                &self.res.motion().fluids,
            );
        }
        head_lerps.retain(|_, (_, steps)| *steps > 0);
        let _ = self.handle_mob_events(mob_fx);
        for (item, ticks, at, eye, pitch, yaw) in using {
            self.remote_player_eating(&item, ticks, at, eye, pitch, yaw);
        }
        if running {
            self.guardian_bubbles();
            // The dragon: LivingEntity's interpolation, inside its aiStep.
            let dragon_lerp = self
                .dragon
                .as_ref()
                .and_then(|d| lerps.get_mut(&(d.id as i32)))
                .filter(|l| l.steps > 0)
                .map(|l| {
                    let steps = l.steps;
                    l.steps -= 1;
                    (l.target, l.rotation.map(|r| r.0), steps)
                });
            self.tick_remote_dragon(dragon_lerp);
            // Armor stands: ClientLevel.tickNonPassenger's setOldPosAndRot,
            // then their SteppedInterpolationHandler's step, as the mobs'.
            if let Some(r) = self.remote.as_mut() {
                for a in &mut self.armor_stands {
                    let step = r
                        .stepped
                        .get_mut(&(a.id as i32))
                        .and_then(|h| h.interpolate());
                    a.interpolate(step.map(|at| {
                        let p = at.position;
                        (Vec3::new(p[0], p[1], p[2]), at.yaw)
                    }));
                }
            }
            // Boats, minecarts, projectiles and the rest ease toward the
            // server's position over three ticks (`lerps`).
            let mut step = |id: u32, at: Vec3| match lerps.get_mut(&(id as i32)) {
                Some(l) if l.steps > 0 => {
                    let n = f64::from(l.steps);
                    l.steps -= 1;
                    Vec3::new(
                        at.x + (l.target[0] - at.x) / n,
                        at.y + (l.target[1] - at.y) / n,
                        at.z + (l.target[2] - at.z) / n,
                    )
                }
                _ => at,
            };
            self.tick_remote_objects(&mut step, |c| {
                cart_lerps
                    .get_mut(&(c.id as i32))
                    .is_some_and(|l| l.tick(c))
            });
            self.tick_remote_projectiles(&mut step);
            // LightningBolt.tick on a client, and the sky flash fading.
            self.flash = self.flash.saturating_sub(1);
            self.tick_bolts();
        }
        lerps.retain(|_, l| l.steps > 0);
        if let Some(r) = self.remote.as_mut() {
            for (id, l) in lerps {
                r.lerps.entry(id).or_insert(l);
            }
            for (id, l) in cart_lerps {
                r.cart_lerps.entry(id).or_insert(l);
            }
        }
    }
    /// `ClientPacketListener.handleMinecartAlongTrack`: the steps join the ones
    /// the minecart has queued, if it runs `NewMinecartBehavior`.
    fn remote_move_minecart(&mut self, p: play::MoveMinecartAlongTrack) {
        if !self.carts.iter().any(|c| c.id == p.id as u32) {
            return;
        }
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let at = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        r.cart_lerps
            .entry(p.id)
            .or_insert_with(CartLerp::new)
            .pending
            .extend(p.steps.iter().map(|s| CartStep {
                position: at(s.position),
                movement: at(s.movement),
                yaw: horizon_protocol::buf::unpack_degrees(s.yaw),
                weight: s.weight,
            }));
    }
    /// `Consumable.onConsumeTick` for another player using a consumable: the
    /// crumbs at their mouth every 4 ticks once 21.875% of its use has passed
    /// (`shouldEmitParticlesAndSounds`; the sounds come from the server).
    fn remote_player_eating(
        &mut self,
        item: &str,
        ticks: u32,
        at: [f64; 3],
        eye: f64,
        pitch: f32,
        yaw: f32,
    ) {
        let Some(consumable) = self
            .res
            .items()
            .get(item)
            .and_then(|i| i.consumable.as_ref())
            .filter(|c| c.particles)
        else {
            return;
        };
        let duration = consumable.ticks().max(0) as u32;
        let remaining = duration - ticks.min(duration);
        if !consumable.should_emit(remaining as i32) {
            return;
        }
        let options = self.item_particle(&ItemStack::new(item, 1));
        let mut spawned = Vec::new();
        horizon_client::particles::item_use_particles(
            &mut self.world.random,
            pitch,
            yaw,
            at,
            at[1] + eye,
            5,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// LocalPlayer.sendPosition plus input, sprint and held-slot updates.
    fn remote_send_state(&mut self, input: u8) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let (pos, on_ground, collision, sprinting) = (
            [p.position.x, p.position.y, p.position.z],
            p.ground.on_ground,
            p.collision.horizontal,
            p.sprinting,
        );
        let (yaw, pitch, selected) = (self.yaw, self.pitch, self.selected);
        let riding = self.riding.is_some();
        let controls_camera = self.controls_camera();
        let vehicle = self.remote_vehicle_packets();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut out = Vec::new();
        if r.carried != Some(selected) {
            r.carried = Some(selected);
            out.push(S::SetCarriedItem(play::SetCarriedItem {
                slot: selected as i16,
            }));
        }
        if input != r.last_input {
            r.last_input = input;
            out.push(S::PlayerInput(play::PlayerInput { flags: input }));
        }
        if sprinting != r.was_sprinting {
            r.was_sprinting = sprinting;
            out.push(S::PlayerCommand(play::PlayerCommand {
                id: r.entity_id,
                action: if sprinting { 1 } else { 2 },
                data: 0,
            }));
        }
        if riding {
            // LocalPlayer.tick: a passenger sends its look, then the vehicle
            // it controls.
            out.push(S::MovePlayerRot(play::MovePlayerRot {
                yaw,
                pitch,
                flags: u8::from(on_ground) | u8::from(collision) << 1,
            }));
            out.extend(vehicle);
        } else if controls_camera
            // LocalPlayer.sendPosition: no move while the view is another
            // entity's (the server keeps the player on it).
            && let Some(packet) =
                r.position_sender
                    .update(pos, (yaw, pitch), on_ground, collision)
        {
            out.push(packet);
        }
        // Creative inventory edits are sent slot by slot (SetCreativeModeSlot).
        let mut creative_slots = Vec::new();
        if self.creative {
            // A server's open menu takes clicks, which it applies itself: what
            // this client predicts for them isn't an edit to send (it would
            // overwrite the server's stacks whenever the prediction is wrong).
            let in_menu = r.menu.is_some();
            let stacks = creative_sync_stacks(&self.inventory, &self.offhand, &self.armor);
            for (i, stack) in stacks.into_iter().enumerate() {
                if stack != r.synced[i] {
                    r.synced[i] = stack.clone();
                    if !in_menu {
                        creative_slots.push((creative_sync_slot(i), stack));
                    }
                }
            }
        }
        for packet in out {
            self.send(packet);
        }
        for (slot, stack) in creative_slots {
            let item = self.to_network(&stack);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot,
                item,
            }));
        }
    }
    fn creative_stacks(&self) -> Vec<Option<ItemStack>> {
        creative_sync_stacks(&self.inventory, &self.offhand, &self.armor)
    }
    /// `CreativeInventoryListener.slotChanged`: `handleCreativeModeItemAdd` for every slot the
    /// player's inventory menu changed. `CreativeModeInventoryScreen.slotClicked` ends in
    /// `broadcastChanges`, so a click's slots go out with the click, right behind a thrown
    /// stack, and not at the end of the tick: a slot update the server sends meanwhile (the
    /// pickup of what a held Q threw) can then never land on a change still unsent and undo it.
    pub(super) fn send_creative_slots(&mut self) {
        if !self.creative {
            return;
        }
        let stacks = self.creative_stacks();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut changed = Vec::new();
        for (i, stack) in stacks.into_iter().enumerate() {
            if stack != r.synced[i] {
                r.synced[i] = stack.clone();
                changed.push((creative_sync_slot(i), stack));
            }
        }
        for (slot, stack) in changed {
            let item = self.to_network(&stack);
            self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot,
                item,
            }));
        }
    }
    /// Inventory as the server last described it.
    fn remote_synced(&mut self) {
        let stacks = self.creative_stacks();
        if let Some(r) = self.remote.as_mut() {
            r.synced = stacks;
        }
    }
    /// The slots a server packet changed since `before` are as the server has them; the
    /// others keep what was last sent, so a creative click made since still goes out.
    fn remote_synced_since(&mut self, before: &[Option<ItemStack>]) {
        let now = self.creative_stacks();
        if let Some(r) = self.remote.as_mut() {
            mark_synced(&mut r.synced, before, now);
        }
    }

    /// `MultiPlayerGameMode.stopDestroyBlock`: aborts the break and starts the attack strength
    /// again, if one is running (`if (!isDestroying) return;`), and says whether it did. Only the
    /// progress goes: `destroyBlockPos` and `destroyingItem` keep the target, so a dig resumed on
    /// the same block sends no START and the server's clock runs on (#4124).
    pub(super) fn stop_remote_mining(&mut self) -> bool {
        let kept = self.remote.as_mut().and_then(|r| {
            if !std::mem::take(&mut r.is_destroying) {
                return None;
            }
            let (pos, face, _) = r.destroying?;
            r.destroying = Some((pos, face, 0.0));
            r.destroying
        });
        let Some((pos, _, _)) = kept else {
            return false;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.tutorial_destroy(state, -1.0);
        self.send(S::PlayerAction(play::PlayerAction {
            action: 2,
            pos,
            direction: 0,
            sequence: 0,
        }));
        self.reset_attack_strength();
        true
    }

    /// MultiPlayerGameMode.startDestroyBlock / continueDestroyBlock.
    fn remote_mining(&mut self, start: bool) {
        let debug_stick = self
            .held()
            .is_some_and(|stack| &*stack.item == "minecraft:debug_stick");
        if !self
            .res
            .items()
            .can_destroy_block(self.held(), self.creative)
        {
            self.stop_remote_mining();
            self.mining = None;
            return;
        }
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if !start && r.destroy_delay > 0 {
            r.destroy_delay -= 1;
            return;
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return;
        };
        let face = face_direction(hit.face);
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.res.facts().is_air(state) {
            return;
        }
        // startDestroyBlock refuses a restricted break before predicting it.
        if self.remote_break_restricted() {
            self.mining = None;
            return;
        }
        if self.creative {
            let seq = self.next_sequence();
            // DebugStickItem.canDestroyBlock never lets the client break the
            // block; the server only selects the property on this START.
            if !debug_stick {
                self.predict_break(hit.pos, state, seq);
            }
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.remote.as_mut() {
                r.destroy_delay = 5;
            }
            return;
        }
        let facts = self.res.facts().get(state).clone();
        // The server's own getDestroyProgress: under water, off the ground,
        // with haste, fatigue and efficiency, so the client stops digging
        // when the server is done too (#1254).
        let progress = if facts.hardness == 0.0 {
            1.0
        } else {
            self.destroy_progress(hit.pos)
        };
        // sameDestroyTarget: the same block and still the item the break began with.
        let held = self.main_hand_key();
        let same = self.remote.as_ref().is_some_and(|r| {
            r.destroying.is_some_and(|(p, _, _)| p == hit.pos) && r.destroying_item == held
        });
        // startDestroyBlock leaves only a break that is still running alone (`isDestroying &&
        // sameDestroyTarget`); a click on the block a stopped break still names starts it again,
        // so the server re-anchors its clock. A held button carries the same target on, and that
        // one keeps the clock (#4124).
        let running = self.remote.as_ref().is_some_and(|r| r.is_destroying);
        if start && same && running {
            return;
        }
        if !same || start {
            // startDestroyBlock aborts only a break that is running; a target left by a stop that
            // ended one is just replaced by the START below.
            let aborted = self.remote.as_mut().and_then(|r| {
                if !std::mem::take(&mut r.is_destroying) {
                    return None;
                }
                r.destroying.take()
            });
            if let Some((old, _, _)) = aborted {
                let old_state = self.world.block(old[0], old[1], old[2]);
                self.tutorial_destroy(old_state, -1.0);
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 2,
                    pos: old,
                    direction: face,
                    sequence: 0,
                }));
            }
            let seq = self.next_sequence();
            self.send(S::PlayerAction(play::PlayerAction {
                action: 0,
                pos: hit.pos,
                direction: face,
                sequence: seq,
            }));
            self.tutorial_destroy(state, 0.0);
            // startDestroyBlock: BlockState.attack on the client, so a redstone
            // ore's dust (RedStoneOreBlock.attack) comes before the break.
            self.redstone_ore_particles(hit.pos);
            if progress >= 1.0 {
                // startDestroyBlock: instant survival breaks have no destroy delay.
                if !debug_stick {
                    self.predict_break(hit.pos, state, seq);
                }
                return;
            }
            if let Some(r) = self.remote.as_mut() {
                r.destroying = Some((hit.pos, face, 0.0));
                r.destroying_item = held;
                r.is_destroying = true;
            }
            // startDestroyBlock plays the first hit and crack itself; the
            // server's level events 2019 and 2020 do the rest.
            self.breaking_block_effects(hit.pos, hit.face, true);
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                ticks: 0,
            });
            return;
        }
        let Some((pos, last_face, done)) = self.remote.as_ref().and_then(|r| r.destroying) else {
            return;
        };
        let done = done + progress;
        self.tutorial_destroy(state, done.clamp(0.0, 1.0));
        let ticks = self.mining.as_ref().map_or(0, |m| m.ticks) + 1;
        if done >= 1.0 {
            let seq = self.next_sequence();
            // DebugStickItem.canDestroyBlock: the server selects the property
            // and sends the block update instead; never predict a break.
            if !debug_stick {
                self.predict_break(pos, state, seq);
            }
            self.send(S::PlayerAction(play::PlayerAction {
                action: 3,
                pos,
                direction: face,
                sequence: seq,
            }));
            if let Some(r) = self.remote.as_mut() {
                // `destroyBlockPos` remains the last target after the break;
                // if that position is filled again before the next pick, a
                // held attack resumes it without sending a second START.
                r.destroying = Some((pos, face, 0.0));
                r.is_destroying = false;
                r.destroy_delay = 5;
            }
            self.mining = None;
        } else {
            if last_face != face {
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 1,
                    pos,
                    direction: face,
                    sequence: 0,
                }));
            }
            if let Some(r) = self.remote.as_mut() {
                r.destroying = Some((pos, face, done));
            }
            self.mining = Some(Mining {
                pos,
                progress: done,
                ticks,
            });
        }
    }
    /// MultiPlayerGameMode.destroyBlock: the predicted break.
    fn predict_break(&mut self, pos: [i32; 3], state: BlockStateId, sequence: i32) {
        self.tutorial_destroy(state, 1.0);
        // BlockStatePredictionHandler.retainKnownServerState.
        if let Some(r) = self.remote.as_mut() {
            r.predictions
                .entry(pos)
                .and_modify(|p| p.0 = sequence)
                .or_insert((sequence, state.0 as i32));
        }
        // Block.playerWillDestroy's destroy event (particles and the break
        // sound), which BaseFireBlock skips: the server's extinguish sound
        // is all a player hears.
        let facts = self.res.facts().get(state);
        if !facts.has_class("BaseFireBlock") {
            let s = facts.sounds.clone();
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.audio.play(
                &s.break_sound,
                Some(at),
                (s.volume + 1.0) / 2.0,
                s.pitch * 0.8,
            );
            self.destroy_particles(pos, state);
        }
        // The server's block update follows (waterlogged blocks leave water).
        let air = self.world.air;
        if self.world.set(pos[0], pos[1], pos[2], air).is_ok() {
            self.world.block_entities.remove(&(pos[0], pos[1], pos[2]));
        }
        self.mining = None;
    }
    /// `Player.blockActionRestricted` as far as this client reads it: a
    /// spectator never breaks, an adventure player only with a main-hand
    /// item the server gave `can_break`. Whether its predicate matches the
    /// block is left to the server, which sends the block back if not.
    fn remote_break_restricted(&self) -> bool {
        use horizon_protocol::item::{WirePatch, component::CAN_BREAK};
        self.spectator()
            || (!self.may_build()
                && !self
                    .held()
                    .and_then(|s| s.wire_patch())
                    .is_some_and(|p| WirePatch::adds(&p, CAN_BREAK)))
    }
    /// Minecraft.startUseItem: use one crosshair target, then the item itself.
    fn remote_use(&mut self) {
        // Minecraft.startUseItem returns before resetting rightClickDelay
        // while a survival block break is active.
        if self.remote.as_ref().is_some_and(|r| r.is_destroying) {
            return;
        }
        self.use_delay = 4;
        // Minecraft.startUseItem: nothing is used while the hands paddle a boat.
        if self.hands_busy() {
            return;
        }
        // Resolve the crosshair once. A passing entity interaction keeps
        // that target and may only fall back to using the item itself.
        let entity = self.entity_targeted();
        horizon_client::interaction_input::use_on_target(&mut RemoteUseWorld(self), entity);
    }

    /// `Minecraft.startUseItem`'s entity turn for `hand`: Interact goes to
    /// the server, and whether the click ends there is what this client's
    /// copy of the entity answers (`MultiPlayerGameMode.interact`).
    fn remote_use_entity(&mut self, hand: Hand) -> bool {
        // A menu this use opens (a chest minecart's) is no clicked block's.
        if let Some(r) = self.remote.as_mut() {
            r.use_target = None;
        }
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        // A boat, minecart, item frame, armor stand or painting nearer than
        // any mob: the server uses it (AbstractBoat.interact boards it).
        if let Some(hit) = self.remote_object_hit(false)
            && self.mob_hit().is_none_or(|(_, d)| hit.distance < d)
            && self.before_block(hit.distance)
        {
            // Interaction.interact answers SUCCESS when it responds and
            // CONSUME otherwise: both end the click (nothing behind the box
            // is used), but only SUCCESS swings; the packet carries the hand.
            let response = self.interactions.get(&hit.id).map(|i| i.response);
            // A cart whose interact passes (a TNT minecart) leaves each hand's
            // turn to its item: a crossbow aimed at it shoots.
            let passes = self
                .carts
                .iter()
                .find(|c| c.id as i32 == hit.id)
                .is_some_and(|c| {
                    let ridden = self.riding == Some(c.id)
                        || self.mobs.iter().any(|m| m.extra.mount == Some(c.id));
                    c.kind
                        .interact_passes(sneaking, ridden, self.can_use_game_master_blocks())
                });
            // Otherwise the main hand's turn takes it; there is no off hand turn.
            if response.is_none() && hand == Hand::Off && !passes {
                return false;
            }
            self.send(S::Interact(play::Interact {
                id: hit.id,
                hand: hand_id(hand),
                location: hit.location,
                secondary: sneaking,
            }));
            if passes {
                return false;
            }
            // MinecartCommandBlock.interact on the client: the editor opens.
            if self.carts.iter().any(|c| {
                c.id as i32 == hit.id && c.kind == super::minecarts::CartKind::CommandBlock
            }) {
                self.open_cart_command_block(hit.id as u32);
            }
            if response.unwrap_or(true) {
                self.swing_arm();
            }
            return true;
        }
        if let Some((i, distance)) = self.mob_hit()
            && self.before_block(distance)
        {
            let id = self.mobs[i].id as i32;
            self.send(S::Interact(play::Interact {
                id,
                hand: hand_id(hand),
                location: [0.0, 1.0, 0.0],
                secondary: sneaking,
            }));
            // Player.interactOn: another player passes, and so does a mob
            // the held item means nothing to: the item is used instead.
            let player = self
                .remote
                .as_ref()
                .is_some_and(|r| r.level_players.contains_key(&id));
            if !player && self.remote_mob_takes(i, hand) {
                self.remote_feed_particles(i, hand);
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
        }
        false
    }

    /// `Minecraft.startUseItem`'s block turn for `hand`: a use that swung drops
    /// the hand (`LocalPlayer.itemUsed`) when it spent the held item or the
    /// player has infinite materials.
    fn remote_use_block(&mut self, hand: Hand) -> bool {
        let before = self.with_hand(hand, |g| g.held().map(|s| s.count));
        let swings = self.swings_started;
        let done = self.remote_use_item_on(hand);
        if done
            && self.swings_started != swings
            && let Some(after) = self.with_hand(hand, |g| g.held().map(|s| s.count))
            && (Some(after) != before || self.creative)
        {
            self.heights.item_used(hand);
        }
        done
    }

    /// MultiPlayerGameMode.useItemOn for `hand`: whether the click ends there
    /// (the block reacts, or a block item is placed).
    fn remote_use_item_on(&mut self, hand: Hand) -> bool {
        let held = self.with_hand(hand, |g| g.held().cloned());
        let held_block = held
            .as_ref()
            .and_then(|h| self.items().get(&h.item))
            .is_some_and(|i| i.block.is_some());
        let debug_stick = held
            .as_ref()
            .is_some_and(|stack| &*stack.item == "minecraft:debug_stick");
        if let Some(hit) = self.pick() {
            // ShelfBlock.useItemOn consumes a nonempty main-hand click client-side
            // without predicting the inventory swap or swinging.
            let shelf_reacts = hand == Hand::Main
                && held.is_some()
                && !(self.keys.contains(&KeyCode::ShiftLeft)
                    && (self.stack_in(self.selected).is_some() || self.offhand.is_some()))
                && self.shelf_hit(&hit).is_some();
            let spectator = self.spectator();
            let (result, changes) = if hand == Hand::Main && !spectator {
                self.remote_block_reacts(&hit, held.as_ref())
            } else {
                // useItemOn runs for the off hand too (useWithoutItem doesn't):
                // RespawnAnchorBlock.useItemOn charges an anchor with its
                // glowstone, SUCCESS.
                let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
                let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
                let charges =
                    !(sneaking && hands_full) && self.anchor_takes(hit.pos, held.as_ref());
                let result = if charges {
                    InteractionResult::Success
                } else {
                    InteractionResult::Pass
                };
                (result, DryUse::default())
            };
            // DebugStickItem.useOn once the block passed: SUCCESS on the client
            // too (the hand swings below); only the server changes the block.
            let (result, changes) = if debug_stick && !spectator && !result.consumes_action() {
                (InteractionResult::Success, DryUse::default())
            } else {
                (result, changes)
            };
            let reacts = result.consumes_action();
            // useItemOn on this client, unless a sneak with something in hand
            // skips it: RedStoneOreBlock's dust, and the wick smoke of a lit
            // candle an empty hand snuffs (CandleBlock.useItemOn) or a lit
            // candle cake (CandleCakeBlock.useItemOn).
            if hand == Hand::Main && !spectator {
                let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
                let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
                if !(sneaking && hands_full) {
                    self.redstone_ore_particles(hit.pos);
                    let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                    let name = self.block_name(hit.pos);
                    let lit = self.pack.assets.registry.state(state).is_some_and(|s| {
                        s.properties.get("lit").map(String::as_str) == Some("true")
                    });
                    let snuffs = if name.ends_with("candle_cake") {
                        hit.point[1] - f64::from(hit.pos[1]) > 0.5
                    } else {
                        name.ends_with("candle") && self.may_build()
                    };
                    if held.is_none() && lit && snuffs {
                        self.candle_smoke(hit.pos, state);
                    }
                }
            }
            // ArmorStandItem.useOn runs in startPrediction before the packet
            // goes out (the server may spawn the stand at once).
            let stand_fits = held
                .as_ref()
                .is_some_and(|s| &*s.item == "minecraft:armor_stand")
                && held
                    .as_ref()
                    .is_some_and(|s| self.armor_stand_spawn(&hit, s).is_some());
            let seq = self.next_sequence();
            let cursor = [
                (hit.point[0] - f64::from(hit.pos[0])) as f32,
                (hit.point[1] - f64::from(hit.pos[1])) as f32,
                (hit.point[2] - f64::from(hit.pos[2])) as f32,
            ];
            self.send(S::UseItemOn(play::UseItemOn {
                hand: hand_id(hand),
                pos: hit.pos,
                face: i32::from(face_direction(hit.face)),
                cursor,
                inside: hit.face == [0; 3],
                world_border: false,
                sequence: seq,
            }));
            // MultiPlayerGameMode.performUseItemOn: a spectator's use predicts
            // nothing and ends the click (CONSUME, no swing); the server opens
            // the block's menu.
            if spectator {
                return true;
            }
            // Only a predicted placement (predict_place) marks a cell whose
            // resend plays a place sound: an empty hand or a non-block item
            // places nothing (BlockItem.place is where the sound comes from).
            if let Some(r) = self.remote.as_mut() {
                r.use_target = Some(hit.pos);
            }
            if shelf_reacts {
                return true;
            }
            if reacts {
                self.predict_use(&changes, seq);
                // LeverBlock.useWithoutItem on the client: the predicted pull puffs.
                if &*self.block_name(hit.pos) == "minecraft:lever" {
                    self.lever_puff(hit.pos);
                }
                self.open_command_block_if_editable((hit.pos[0], hit.pos[1], hit.pos[2]));
                self.open_block_editor_if_editable((hit.pos[0], hit.pos[1], hit.pos[2]));
                // MultiPlayerGameMode.useItemOn: SUCCESS SwingSource.CLIENT
                // (a chest, barrel, furnace, …). CONSUME does not swing.
                if result == InteractionResult::Success {
                    self.with_hand(hand, Game::swing_arm);
                }
            } else if self.predict_campfire_douse(hit.pos, held.as_ref(), seq) {
                // CampfireBlock.useItemOn succeeded: this hand swings and the
                // click ends, so a shield in the other hand is not raised.
                self.with_hand(hand, Game::swing_arm);
                return true;
            } else if held.as_ref().is_some_and(|s| &*s.item == "minecraft:brush") {
                // BrushItem.useOn: brushing starts here (CONSUME, no swing); the
                // server brushes and the dust is this client's own.
                self.with_hand(hand, Game::start_using_item);
                return true;
            } else if held
                .as_ref()
                .is_some_and(|s| &*s.item == "minecraft:bone_meal")
                && self.remote_bone_meal_succeeds(&hit)
            {
                // BoneMealItem.useOn succeeded on this client's level (SUCCESS;
                // the server grows the plant): this hand swings and the click
                // ends, so the other hand (a torch) is never tried.
                self.with_hand(hand, Game::swing_arm);
                return true;
            } else if stand_fits {
                // ArmorStandItem.useOn on a client level: SUCCESS once the
                // stand fits, before the server spawns it. This hand swings
                // and the click ends.
                self.with_hand(hand, Game::swing_arm);
                return true;
            } else if let Some(stack) = held.as_ref()
                && (self.predict_item_on_block(stack, &hit)
                    || self.predict_map_on_banner(stack, &hit))
            {
                // Item.useOn succeeded here (InteractionResult.SUCCESS):
                // this hand swings and the click ends, so the other hand
                // (a shield, a torch) is never tried.
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
            if held_block {
                let consumable = held
                    .as_ref()
                    .and_then(|stack| self.items().get(&stack.item).cloned())
                    .filter(|info| info.consumable.is_some());
                // ItemStack.useOn passes in adventure mode unless the item may
                // be placed on this block: useItem eats it (a block item without
                // a consumable keeps ending the click here).
                if !reacts
                    && consumable.is_some()
                    && !self.may_build()
                    && !self.with_hand(hand, |g| g.can_place_on_in_adventure(hit.pos))
                {
                    return false;
                }
                let placed = if reacts {
                    None
                } else {
                    held.as_ref()
                        .and_then(|stack| self.remote_can_place(&hit, stack))
                };
                // BlockItem.useOn: a consumable that could not be placed is used
                // (Item.use), which the server does for this same packet. CONSUME
                // does not swing, and a full player's FAIL still ends the click.
                // Adventure's can_place_on leaves the placing to the server.
                if !reacts
                    && placed.is_none()
                    && self.may_build()
                    && let Some(info) = &consumable
                {
                    self.with_hand(hand, |g| g.start_eating(info));
                }
                if let Some(placed) = &placed {
                    self.predict_place(placed, seq);
                    self.with_hand(hand, Game::consume_held);
                } else if let Some(r) = self.remote.as_mut() {
                    // BlockItem.place plays its sound only once the block is set: the
                    // server's resend of the target it refused is no placement.
                    r.placing = None;
                }
                if placed.is_some() {
                    self.with_hand(hand, Game::swing_arm);
                }
                // BlockItem.place returns FAIL when placement is rejected:
                // stop this click without trying the item or the other hand.
                return true;
            }
            return reacts;
        }
        false
    }

    /// `Item.useOn` on this client's level for the items whose effects are
    /// the clicker's own (`Level.levelEvent(player, ...)` and
    /// `playSound(player, ...)`: the server sends them to the others only):
    /// honeycomb waxing copper, and an axe, hoe or shovel transforming a
    /// block (stripping, scraping, removing wax, tilling, making a path).
    /// A cushion succeeds where it may be placed and spawns on the server.
    /// Whether the item's use succeeded, which ends the click.
    fn predict_item_on_block(&mut self, stack: &ItemStack, hit: &super::Hit) -> bool {
        let at = [
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.5,
            f64::from(hit.pos[2]) + 0.5,
        ];
        if &*stack.item == "minecraft:honeycomb" {
            // HoneycombItem.useOn.
            let registry = self.pack.assets.registry;
            let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let waxes = registry
                .state(clicked)
                .and_then(|old| self.res.transformers().waxed(registry, old))
                .is_some();
            if waxes {
                self.level_event_at(3003, hit.pos, 0);
                self.audio
                    .play("minecraft:item.honeycomb.wax_on", Some(at), 1.0, 1.0);
            }
            waxes
        } else if let Some((_, sound, _, _, particle)) =
            self.block_transform(stack, hit.pos, hit.face)
        {
            self.transform_particle(&particle, hit.pos);
            self.audio.play(&sound, Some(at), 1.0, 1.0);
            true
        } else if self
            .res
            .items()
            .get(&stack.item)
            .is_some_and(|i| i.cushion_color.is_some())
        {
            // CushionItem.useOn on a client level: SUCCESS once
            // `canBePlacedAt` holds, before the server's own checks.
            self.cushion_spawn(hit).is_some()
        } else {
            false
        }
    }

    /// `MapItem.useOn` on this client: a map used on a banner is SUCCESS
    /// whatever its data says (the server toggles the marker, or fails
    /// quietly), so this hand swings and the click ends. `ItemStack.useOn`
    /// passes in adventure mode.
    fn predict_map_on_banner(&self, stack: &ItemStack, hit: &super::Hit) -> bool {
        super::maps::map_id(stack).is_some()
            && self.may_build()
            && self.block_in_tag("minecraft:banners", &self.block_name(hit.pos))
    }

    /// `BoneMealItem.useOn` on this client's level (a copy: nothing grows
    /// here): whether it succeeds.
    fn remote_bone_meal_succeeds(&mut self, hit: &super::Hit) -> bool {
        let pos = horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        let face = horizon_core::geometry::Direction::ALL[face_direction(hit.face) as usize];
        self.with_level(|b, level, _| {
            b.bone_meal_succeeds_on_client(&mut DryLevel::new(level), pos, face)
        })
    }

    /// MultiPlayerGameMode.useItemOn inside startPrediction: the blocks the
    /// use changed on this client's level at once, each kept as predicted
    /// under the use's sequence until the server acknowledges it (a use the
    /// server refused goes back). Only the blocks a hand opens or flips: a
    /// wooden or copper door, trapdoor or fence gate, a lever or a button;
    /// whatever else the use changes waits for the server.
    fn predict_use(&mut self, (changes, sounds): &DryUse, seq: i32) {
        let facts = self.res.facts();
        let predicted = |state: BlockStateId| hand_operated(&facts.get(state).name);
        if changes.is_empty()
            || !changes
                .iter()
                .all(|(p, s)| predicted(*s) && predicted(self.world.block(p.x, p.y, p.z)))
        {
            return;
        }
        self.predict_blocks(changes, seq);
        // `level.playSound(player, ...)`: the clicker hears it from its own use; the
        // server sends it to the others only.
        for (pos, sound, volume, pitch) in sounds {
            let at = [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 0.5,
                f64::from(pos.z) + 0.5,
            ];
            let name = if sound.contains(':') {
                sound.to_string()
            } else {
                format!("minecraft:{sound}")
            };
            self.audio.play(&name, Some(at), *volume, *pitch);
        }
    }

    /// Whether the block hit takes a main-hand right click with `held`
    /// (`BlockState.useItemOn`, then `useWithoutItem`): a door, a button, a
    /// container, and the blocks it changed. Vanilla's client runs it on its
    /// own level first; this runs it on a copy, and `predict_use` keeps the
    /// changes it predicts.
    fn remote_block_reacts(
        &mut self,
        hit: &super::Hit,
        held: Option<&ItemStack>,
    ) -> (InteractionResult, DryUse) {
        // Player.isSecondaryUseActive with something in either hand.
        let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
        if self.keys.contains(&KeyCode::ShiftLeft) && hands_full {
            return (InteractionResult::Pass, DryUse::default());
        }
        if self.jukebox_takes(hit.pos, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        // RespawnAnchorBlock.useItemOn: PASS when the off hand's glowstone is to
        // charge the anchor, so useWithoutItem doesn't run and the off hand's turn
        // comes (glowstone in this hand charges it in `use_item_on`).
        if !self.anchor_takes(hit.pos, held) && self.anchor_takes(hit.pos, self.offhand.as_ref()) {
            return (InteractionResult::Pass, DryUse::default());
        }
        // BeehiveBlock.useItemOn: shears or a glass bottle on a full hive
        // (honey level 5) is SUCCESS, so the click ends after UseItemOn and
        // no UseItem follows (the server does the harvest, see `use_beehive`).
        if held.is_some_and(|s| matches!(&*s.item, "minecraft:shears" | "minecraft:glass_bottle"))
            && self
                .hive_state(hit.pos)
                .is_some_and(|(honey, _)| honey == Some(5))
        {
            return (InteractionResult::Success, DryUse::default());
        }
        // ComposterBlock.useItemOn: a compostable item on a composter below
        // level 8 is SUCCESS on the client too (only the server adds the
        // layer), so the other hand (a torch) is never tried.
        if self.composter_takes(hit.pos, held) {
            return (InteractionResult::Success, DryUse::default());
        }
        use horizon_core::geometry::Direction;
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        let player = self.use_context();
        self.with_level(|b, level, _| {
            let mut level = DryLevel::new(level);
            let result = b.use_item_on(
                &mut level,
                &mut horizon_world::level::Ticks::default(),
                horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                face,
                horizon_core::geometry::Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
                player,
                held.map(|s| &*s.item),
            );
            (result, (level.changed.into_iter().collect(), level.sounds))
        })
    }

    /// MultiPlayerGameMode.useItemOn inside startPrediction, for a block item:
    /// the blocks `BlockItem.place` set at `pos` are placed on this client's
    /// level at once, so the next click of a fast bridge aims at them rather
    /// than past them (the server would refuse it). Its place sound is the
    /// placer's own (`playSound(player, ...)`: the server sends it to the
    /// others only).
    fn predict_place(&mut self, (pos, changes): &DryPlace, seq: i32) {
        let ticks = self.ticks;
        if let Some(r) = self.remote.as_mut() {
            // set_block plays the place sound of the block at `placing`.
            r.placing = Some(([pos.x, pos.y, pos.z], ticks));
        }
        self.predict_blocks(changes, seq);
    }
    /// The blocks a use changed, set on this client's level and kept as
    /// predicted under the use's sequence until the server acknowledges it
    /// (`BlockStatePredictionHandler.retainKnownServerState`): a use the
    /// server refused goes back.
    pub(super) fn predict_blocks(&mut self, changes: &DryChanges, seq: i32) {
        for &(p, state) in changes {
            let pos = [p.x, p.y, p.z];
            let before = self.world.block(p.x, p.y, p.z);
            if let Some(r) = self.remote.as_mut() {
                r.predictions
                    .entry(pos)
                    .and_modify(|e| e.0 = seq)
                    .or_insert((seq, before.0 as i32));
            }
            self.set_block(pos, state.0 as i32);
        }
    }

    /// Run BlockItem.place on a disposable level: where it placed and the
    /// blocks it set. The real level supplies collision shapes and
    /// building-blocking entities; random draws, scheduled ticks and events
    /// stay in the preview.
    fn remote_can_place(&mut self, hit: &super::Hit, stack: &ItemStack) -> Option<DryPlace> {
        if !self.may_build() {
            return None;
        }
        let ctx = horizon_world::level::PlaceContext {
            clicked_pos: horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face: horizon_core::geometry::Direction::ALL[face_direction(hit.face) as usize],
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: hit.face == [0; 3],
            yaw: self.yaw,
            pitch: self.pitch,
            sneaking: self.keys.contains(&KeyCode::ShiftLeft),
            item: stack.item.to_string(),
            can_use_game_master_blocks: self.can_use_game_master_blocks(),
        };
        self.with_level(|b, level, _| {
            let mut level = DryLevel::new(level);
            match b.place_block(
                &mut level,
                &mut horizon_world::level::Ticks::default(),
                &ctx,
            ) {
                horizon_world::level::PlaceOutcome::Placed { pos, .. } => {
                    Some((pos, level.changed.into_iter().collect()))
                }
                horizon_world::level::PlaceOutcome::Failed => None,
            }
        })
    }

    /// `MultiPlayerGameMode.useItem`'s local `ItemStack.use` for a bucket:
    /// `BucketItem.use` picks its own target and runs `pickupBlock` or
    /// `emptyContents` on this client's level inside `startPrediction`, so
    /// its success (which decides `itemUsed`) and the blocks it changes are
    /// the client's own. A fill into a waterloggable block changes nothing
    /// here: `SimpleWaterloggedBlock.placeLiquid` tests `isClientSide` and
    /// leaves the block update to the server.
    fn remote_use_bucket(
        &mut self,
        stack: &ItemStack,
    ) -> (bool, Option<DryChanges>, Vec<DrySound>) {
        // `BucketItem.use`: `level.mayInteract` and `mayUseItemAt`.
        if !self.may_build() {
            return (false, None, Vec::new());
        }
        if &*stack.item == "minecraft:bucket" {
            let Some(hit) = self.bucket_hit() else {
                return (false, None, Vec::new());
            };
            let pos = horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let creative = self.creative;
            return self.with_level(|b, level, ticks| {
                let mut level = DryLevel::new(level);
                let picked = b.bucket_pickup(&mut level, ticks, pos, creative).is_some();
                (
                    picked,
                    Some(level.changed.into_iter().collect()),
                    std::mem::take(&mut level.sounds),
                )
            });
        }
        let Some((content, sound, _)) = horizon_client::items::bucket_contents(&stack.item) else {
            return (false, None, Vec::new());
        };
        let Some(hit) = self.pick() else {
            return (false, None, Vec::new());
        };
        let target = self.bucket_target(content, &hit);
        let pos = horizon_core::position::BlockPos::new(target[0], target[1], target[2]);
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        let creative = self.creative;
        // A water bucket on a liquid container fills it server-side only.
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let client_change =
            !(content.is_water() && self.res.behaviours().is_liquid_container(clicked));
        self.with_level(|b, level, ticks| {
            let mut level = DryLevel::new(level);
            let emptied =
                b.bucket_empty(&mut level, ticks, pos, content, sound, sneaking, creative);
            (
                emptied,
                client_change.then(|| level.changed.into_iter().collect()),
                std::mem::take(&mut level.sounds),
            )
        })
    }

    /// MultiPlayerGameMode.useItem for `hand`, if it holds something: whether
    /// the click ends there, as far as this client can tell (`use_ends_click`).
    fn remote_use_item(&mut self, hand: Hand) -> bool {
        let Some(held) = self.with_hand(hand, |g| g.held().cloned()) else {
            return false;
        };
        let seq = self.next_sequence();
        self.send(S::UseItem(play::UseItem {
            hand: hand_id(hand),
            sequence: seq,
            yaw: self.yaw,
            pitch: self.pitch,
        }));
        let Some(info) = self.items().get(&held.item).cloned() else {
            return false;
        };
        // MultiPlayerGameMode.useItem's prediction: an item cooling down
        // passes (its packet still goes).
        let group = horizon_client::cooldowns::group(&held, info.use_cooldown.as_ref());
        if self.cooldowns.is_on_cooldown(&group) {
            return false;
        }
        // a mod's item used over time starts here too, as the server starts it.
        if self.with_hand(hand, |g| g.start_mod_use(&held)) {
            return true;
        }
        // Item.use: Equippable.swapWithEquipmentSlot runs on this client too, so
        // the armor slot and the hand change at once (the server confirms).
        if info.equip_slot.is_some() && self.with_hand(hand, |g| g.equip_from_hand(&held)) {
            return true;
        }
        // `ItemStack.use` runs on this client too: a bucket's own ray and its
        // pick/empty contents decide whether the click succeeds (a failed
        // fill or pickup leaves the hand alone), and the blocks it sets are
        // this use's prediction.
        if &*held.item == "minecraft:bucket"
            || horizon_client::items::bucket_contents(&held.item).is_some()
        {
            let (succeeded, changes, sounds) = self.with_hand(hand, |g| g.remote_use_bucket(&held));
            if let Some(changes) = changes {
                self.predict_blocks(&changes, seq);
            }
            // `BucketItem.emptyContents` / evaporate: `playSound(player, …)` on
            // this client; the server leaves the actor out (#4684).
            for (pos, sound, volume, pitch) in sounds {
                let at = [
                    f64::from(pos.x) + 0.5,
                    f64::from(pos.y) + 0.5,
                    f64::from(pos.z) + 0.5,
                ];
                let name = if sound.contains(':') {
                    sound.to_string()
                } else {
                    format!("minecraft:{sound}")
                };
                self.audio.play(&name, Some(at), volume, pitch);
            }
            return succeeded;
        }
        // Food starts being eaten (Consumable.startConsuming). A food that
        // can't be eaten (full and not always-edible) fails the use
        // (InteractionResult.FAIL): startUseItem tries the other hand, and
        // no cooldown or use animation starts.
        let eating = self.with_hand(hand, |g| g.start_eating(&info));
        if info.food.is_some() && !eating {
            return false;
        }
        // A bucket, a glass bottle or a boat with nothing to fill, empty or
        // place PASSes or FAILs: startUseItem goes on to the off hand (a
        // Riptide trident there charges).
        if self.with_hand(hand, |g| g.use_finds_no_target(&held)) {
            return false;
        }
        // It runs the use here too: ItemStack.use starts the `use_cooldown`
        // of an item with no use time, and the server's
        // ClientboundCooldownPacket confirms it; one held in use starts it at
        // finishUsingItem (remote_complete_using).
        if !info.consumable.as_ref().is_some_and(|c| c.ticks() > 0) {
            self.apply_use_cooldown(&held);
        }
        if self.with_hand(hand, |g| g.remote_use_book(&held)) {
            return true;
        }
        // BowItem.use and CrossbowItem.use FAIL with nothing to shoot: no
        // itemUsed (the hand doesn't dip) and the other hand's turn comes.
        if !self.with_hand(hand, |g| g.remote_start_held_use(&held)) {
            return false;
        }
        let gliding = self.player.as_ref().is_some_and(|p| p.fall_flying);
        // A stack's own `blocks_attacks` raises it as a shield's does.
        use_ends_click(&info, gliding) || self.blocks_attacks(&held).is_some()
    }

    /// Whether `held`'s `Item.use` needs a target this client sees none of
    /// (not SUCCESS): BucketItem.use (a fluid source or powder snow to pick
    /// up through `ClipContext.Fluid.SOURCE_ONLY`, a block to empty a full
    /// one against, `mayUseItemAt`), BottleItem.use (water to fill from) and
    /// BoatItem.use (a block or fluid surface with room for the boat).
    fn use_finds_no_target(&self, held: &ItemStack) -> bool {
        let id = &*held.item;
        let facts = |pos: [i32; 3]| {
            self.res
                .facts()
                .get(self.world.block(pos[0], pos[1], pos[2]))
        };
        let source = |want: Option<Fluid>| {
            let fluid_hit = self.pick_fluid();
            // pick_fluid steps through waterlogged blocks, but their water is
            // a source the SOURCE_ONLY ray stops at (and a BucketPickup).
            if let Some(hit) = self.pick()
                && facts(hit.pos).waterlogged
            {
                let eye = self.aim().map_or(hit.point, |(eye, _)| eye);
                let d = |p: [f64; 3]| (0..3).map(|a| (p[a] - eye[a]).powi(2)).sum::<f64>();
                if fluid_hit
                    .as_ref()
                    .is_none_or(|fluid| d(fluid.point) >= d(hit.point))
                {
                    return want.is_none_or(|want| want == Fluid::Water);
                }
            }
            fluid_hit.is_some_and(|hit| {
                let f = facts(hit.pos);
                let fluid = f.fluid != Fluid::None && f.fluid_level == 8 && !f.fluid_falling;
                match want {
                    Some(want) => fluid && f.fluid == want,
                    None => fluid || self.is_powder_snow(hit.pos),
                }
            })
        };
        // SolidBucketItem only has useOn: its use is Item.use's PASS.
        if id == "minecraft:powder_snow_bucket" {
            return true;
        }
        if id == "minecraft:bucket" {
            return !self.may_build() || !source(None);
        }
        if horizon_client::items::bucket_contents(id).is_some() {
            return !self.may_build() || self.pick().is_none();
        }
        if id == "minecraft:glass_bottle" {
            return !source(Some(Fluid::Water));
        }
        if super::boats::any_boat_wood(id).is_some() {
            return self.boat_to_place(held).is_none();
        }
        false
    }
    /// `Item.use` runs on the client too (MultiPlayerGameMode.useItem): an
    /// item held in use until the button lets go starts being used here, so
    /// that letting go sends RELEASE_USE_ITEM (Minecraft.handleKeybinds) and
    /// the server ends the use: a bow or a trident is let go, a crossbow's
    /// charge ends, a shield is lowered, a spyglass stops scoping. Their
    /// sounds, loaded arrows and thrown tridents come from the server.
    /// False when the use FAILs here: a bow or an uncharged crossbow with
    /// nothing to shoot.
    fn remote_start_held_use(&mut self, held: &ItemStack) -> bool {
        if self.using.is_some() {
            return true;
        }
        let bundle = self.items().get(&held.item).is_some_and(|i| i.bundle);
        let start = match &*held.item {
            // BundleItem.use: always starts; the server drops the contents.
            _ if bundle => true,
            // BowItem.use: with an arrow, or in creative.
            "minecraft:bow" => self.creative || self.projectile_slot("minecraft:bow").is_some(),
            // CrossbowItem.use: charging when not charged and with ammunition;
            // a charged one shoots on the server.
            "minecraft:crossbow" => {
                if held.data().is_some_and(|d| !d.charged.is_empty()) {
                    return true;
                }
                self.creative || self.projectile_slot("minecraft:crossbow").is_some()
            }
            // TridentItem.use: not about to break, Riptide only in water or rain.
            "minecraft:trident" => {
                self.use_trident(held);
                return true;
            }
            // SpyglassItem.use (its sound is the server's).
            "minecraft:spyglass" => true,
            // InstrumentItem.use: the horn plays here too (`playSound(player, ...)`
            // skips the player's own client), the server's copy tells the others.
            "minecraft:goat_horn" => {
                self.remote_use_instrument(held);
                return true;
            }
            // EnderEyeItem.use: startUsingItem unless an End portal frame is
            // aimed at, with or without a stronghold to throw the eye at.
            "minecraft:ender_eye" => !self.aims_at_end_portal_frame(),
            // Item.use: a kinetic weapon starts being used.
            id if horizon_client::spear::Kinetic::of(id).is_some() => true,
            // ShieldItem.use: anything that blocks attacks, off cooldown.
            _ => {
                self.raise_shield(self.held_slot());
                return true;
            }
        };
        if start {
            self.start_using_item();
        }
        start || !matches!(&*held.item, "minecraft:bow" | "minecraft:crossbow")
    }
    /// `InstrumentItem.use` on this client: unless the horn is on cooldown,
    /// it is held up, plays its sound at the player and starts its cooldown.
    fn remote_use_instrument(&mut self, held: &ItemStack) {
        let Some(instrument) = self.res.items().instrument(held) else {
            return;
        };
        let group = horizon_client::cooldowns::group(held, None);
        if self.cooldowns.is_on_cooldown(&group) {
            return;
        }
        self.start_using_item();
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        self.audio
            .play(&instrument.sound, at, instrument.range / 16.0, 1.0);
        if instrument.use_duration > 0.0 {
            self.add_cooldown(group, instrument.use_ticks());
        }
    }
    /// Whether `stack` stays in use until the button lets go (bows,
    /// crossbows, tridents, spyglasses, shields, eyes of ender, whose
    /// `getUseDuration` 0 never counts down to the end) or the server ends
    /// it (a mod's item): its use only counts ticks here.
    fn held_until_released(&self, stack: &ItemStack) -> bool {
        if self.mod_use(stack) {
            return true;
        }
        matches!(
            &*stack.item,
            "minecraft:bow"
                | "minecraft:crossbow"
                | "minecraft:trident"
                | "minecraft:spyglass"
                | "minecraft:ender_eye"
        ) || horizon_client::spear::Kinetic::of(&stack.item).is_some()
            || self.blocks_attacks(stack).is_some()
    }

    /// The rest of Minecraft.startUseItem once the main hand's target passed:
    /// the main hand's item, then the off hand's turn (its block, its item).
    /// The server does each packet's part with that packet's hand only.
    fn remote_use_hands(&mut self) {
        // A successful Item.use always drops the hand (LocalPlayer.itemUsed).
        if self.remote_use_item(Hand::Main) {
            self.heights.item_used(Hand::Main);
            return;
        }
        // The off hand's turn: the entity again, or the block.
        let off_target = if self.entity_targeted() {
            self.remote_use_entity(Hand::Off)
        } else {
            self.remote_use_block(Hand::Off)
        };
        if off_target {
            return;
        }
        if self.remote_use_item(Hand::Off) {
            self.heights.item_used(Hand::Off);
        }
    }
    /// `LivingEntity.updatingUsingItem` on a server client: an item held in
    /// use (`held_until_released`), and a consumable's eating animation and
    /// its crumbs until the server's USE_ITEM_COMPLETE. The server sends the
    /// stack, its hunger and the sounds.
    fn remote_tick_using(&mut self) {
        let Some((slot, ticks)) = self.using else {
            return;
        };
        let stack = self.stack_in(slot).cloned();
        // The use stops when the hand no longer holds the item it began with
        // (ItemStack.isSameItem).
        let item = stack.as_ref().map(|s| Arc::clone(&s.item));
        if let Some(r) = self.remote.as_mut() {
            if ticks == 0 {
                r.use_item = item;
            } else if r.use_item != item {
                self.using = None;
                return;
            }
        }
        // Another hotbar slot picked: LivingEntity.updatingUsingItem sees a
        // different stack in the hand. The off hand keeps its item.
        let in_hand = slot == self.selected || slot == super::shield::OFFHAND_SLOT;
        // BrushItem.onUseTick: with no block aimed the use ends (the server
        // ends it too); a stroke shows its dust. Its sounds are the server's.
        if stack
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:brush")
        {
            let elapsed = ticks + 1;
            let hit = self.pick().filter(|_| in_hand);
            if let Some(hit) = &hit
                && elapsed % 10 == 5
            {
                self.brush_dust(hit, slot == super::shield::OFFHAND_SLOT);
            }
            if hit.is_none() && in_hand {
                self.send(S::PlayerAction(play::PlayerAction {
                    action: 6,
                    pos: [0; 3],
                    direction: 0,
                    sequence: 0,
                }));
            }
            self.using = hit.and(Some((slot, elapsed))).filter(|_| elapsed < 200);
            return;
        }
        // BundleItem.getUseDuration: 200 ticks, the server drops the contents.
        if stack
            .as_ref()
            .is_some_and(|s| self.items().get(&s.item).is_some_and(|i| i.bundle))
        {
            self.using = (in_hand && ticks + 1 < 200).then_some((slot, ticks + 1));
            return;
        }
        // A drawn bow, a charging crossbow, a raised shield...: held until
        // the button lets go.
        if stack.as_ref().is_some_and(|s| self.held_until_released(s)) {
            self.using = in_hand.then_some((slot, ticks + 1));
            return;
        }
        // A goat horn is held up for its instrument's use duration.
        if let Some(duration) = stack
            .as_ref()
            .and_then(|s| self.res.items().instrument(s))
            .map(|i| i.use_ticks().max(0) as u32)
        {
            self.using = (in_hand && ticks + 1 < duration).then_some((slot, ticks + 1));
            return;
        }
        let consumable = stack
            .as_ref()
            .and_then(|s| self.items().get(&s.item))
            .and_then(|i| i.consumable.clone());
        let (Some(stack), Some(consumable)) = (stack, consumable) else {
            self.using = None;
            return;
        };
        // Button release belongs to the next handleKeybinds phase, which
        // sends RELEASE_USE_ITEM. Do not silently clear a use begun by a
        // quick press/release between ticks.
        if !in_hand {
            self.using = None;
            return;
        }
        // Consumable.startConsuming eats one with no use time at once, without
        // a use (nor a USE_ITEM_COMPLETE).
        if consumable.ticks() <= 0 {
            self.using = None;
            return;
        }
        // Consumable.shouldEmitParticlesAndSounds, as in tick_used_item. The
        // countdown goes on past zero (LivingEntity.updateUsingItem completes
        // only on the server) until the server's USE_ITEM_COMPLETE.
        // getUseItemRemainingTicks: the duration this use started with.
        let remaining = self.use_duration - ticks as i32;
        if consumable.should_emit(remaining) {
            // `emit_consume` plays the eat sound and spawns the bites (when the
            // item has consume particles), as `tick_used_item` does on the
            // player's own server.
            self.emit_consume(&consumable, &stack, 5);
        }
        self.using = Some((slot, ticks + 1));
    }
    /// `Player.handleEntityEvent(9)`: `completeUsingItem` on the client. A consumable's last crumbs, and FoodProperties.onConsume,
    /// which runs on the client too (Consumable.onConsume calls its
    /// ConsumableListeners on both sides): the client's FoodData eats at once,
    /// so a held right click finds `Player.canEat` false instead of starting
    /// another bite before the server's ClientboundSetHealth arrives.
    fn remote_complete_using(&mut self) {
        let Some((slot, _)) = self.using.take() else {
            return;
        };
        let Some(stack) = self.stack_in(slot).cloned() else {
            return;
        };
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return;
        };
        // Consumable.onConsume's emitParticlesAndSounds(16): the last crumbs
        // and bite, which the server plays for the others only.
        if let Some(consumable) = &info.consumable {
            self.emit_consume(consumable, &stack, 16);
        }
        if let Some((nutrition, saturation)) = info.food
            && let Some(r) = self.remote.as_mut()
        {
            // FoodData.eat(nutrition, saturation), as finish_consuming does;
            // the next ClientboundSetHealth replaces it.
            let (_, food, sat) = &mut r.health;
            *food = (*food + nutrition as i32).clamp(0, 20);
            *sat = (*sat + saturation).clamp(0.0, *food as f32);
            self.apply_remote_health();
        }
        // ItemStack.finishUsingItem's applyAfterUseComponentSideEffects.
        self.apply_use_cooldown(&stack);
    }
    /// MouseHandler updates KeyMapping; handleKeybinds sends packets on a tick.
    pub(super) fn remote_mouse(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        use horizon_client::interaction_input::Button;
        let input = match button {
            MouseButton::Left => {
                self.toggles
                    .button(2, pressed, false, self.settings.toggle_attack);
                self.left = self.toggles.is_down(2);
                Some(Button::Attack)
            }
            MouseButton::Right => {
                self.toggles
                    .button(3, pressed, false, self.settings.toggle_use);
                self.right = self.toggles.is_down(3);
                Some(Button::Use)
            }
            MouseButton::Middle => Some(Button::Pick),
            _ => None,
        };
        if pressed
            && let Some(button) = input
            && let Some(remote) = self.remote.as_mut()
        {
            remote.input.press(button);
        }
        Ok(())
    }

    pub(super) fn request_remote_mining_stop(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.stop_mining = true;
        }
    }

    fn remote_handle_input(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut input = std::mem::take(&mut r.input);
        let stop = std::mem::take(&mut r.stop_mining);
        let ready = r.loaded_sent
            && self.screen == Screen::Playing
            && self.sleeping.is_none()
            && !self.dialog_open();
        if stop || !ready {
            self.stop_remote_mining();
        }
        if !ready {
            return;
        }
        // MultiPlayerGameMode.tick syncs the carried slot before keybinds.
        let carried_changed = self.remote.as_mut().is_some_and(|r| {
            if r.carried == Some(self.selected) {
                false
            } else {
                r.carried = Some(self.selected);
                true
            }
        });
        if carried_changed {
            self.send(S::SetCarriedItem(play::SetCarriedItem {
                slot: self.selected as i16,
            }));
        }
        let (attack, use_item) = (self.left, self.right);
        input.tick(&mut RemoteInputWorld(self), attack, use_item);
        // handleKeybinds' keybind polls: the queued presses act now, in tick
        // order, so no packet goes out between CLIENT_TICK_END and a pong.
        let pending = self
            .remote
            .as_mut()
            .map(|r| std::mem::take(&mut r.pending_keys))
            .unwrap_or_default();
        for key in pending {
            match key {
                KeyCode::KeyF if !self.spectator() => self.remote_swap_hands(),
                KeyCode::KeyQ if !self.spectator() => {
                    // Minecraft.hasControlDown: the physical keys, so a toggled
                    // sprint on Left Control does not make Q drop the stack.
                    let all = self.has_control_down();
                    self.remote_drop(all);
                }
                KeyCode::KeyE if self.riding_mount_with_menu() || self.riding_chest_boat() => {
                    let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
                    self.send(S::PlayerCommand(play::PlayerCommand {
                        id,
                        action: 5,
                        data: 0,
                    }));
                }
                _ => {}
            }
        }
        if self.clicks.miss_time > 0 {
            self.clicks.miss_time -= 1;
        }
        if let Some(r) = self.remote.as_mut() {
            r.input = input;
        }
    }

    fn remote_start_attack(&mut self) -> bool {
        if self.clicks.miss_time > 0 {
            return false;
        }
        if self.spectator() {
            // Minecraft.startAttack: a spectator's click on an entity asks
            // to watch through it (MultiPlayerGameMode.spectate); a click on
            // nothing still tells the server (spectatorNoAction).
            let target = self.spectator_click_target();
            self.send(S::SpectatorAction(play::SpectatorAction { target }));
            return true;
        }
        // A spear jabs (`MultiPlayerGameMode.piercingAttack`): the server
        // stabs, this client swings, plays the sound and starts its charge
        // again.
        if let Some(kinetic) = self
            .held()
            .and_then(|s| horizon_client::spear::Kinetic::of(&s.item))
        {
            if self.cannot_attack_with_item(self.hand_strength.ticker, 0) {
                return false;
            }
            // ServerboundPlayerActionPacket.Action.STAB.
            self.send(S::PlayerAction(play::PlayerAction {
                action: 8,
                pos: [0; 3],
                direction: 0,
                sequence: 0,
            }));
            self.hand_strength.reset_attack();
            self.swing_hand(Hand::Main, kinetic.attack_animation());
            self.play_at_feet(kinetic.attack_sound);
            return true;
        }
        let mut instant = false;
        // A ghast's fireball or a wind charge nearer than any mob: the
        // server turns it (game/deflect.rs).
        if let Some((i, distance)) = self.redirectable_hit()
            && self.mob_hit().is_none_or(|(_, d)| distance <= d)
            && self.before_block(distance)
            && self.creatures.projectiles[i].id != 0
        {
            self.send(S::Attack(play::Attack {
                id: self.creatures.projectiles[i].id as i32,
            }));
            self.reset_attack_strength();
        } else if let Some(hit) = self.remote_object_hit(true)
            && self.mob_hit().is_none_or(|(_, d)| hit.distance < d)
            && self.before_block(hit.distance)
        {
            // VehicleEntity.hurtServer, ItemFrame and ArmorStand hurtServer,
            // a painting broken or an EndCrystal hurtServer: the server decides.
            self.send(S::Attack(play::Attack { id: hit.id }));
            self.reset_attack_strength();
        } else if let Some((i, distance)) = self.mob_hit()
            && self.before_block(distance)
        {
            self.send(S::Attack(play::Attack {
                id: self.mobs[i].id as i32,
            }));
            self.remote_player_attack(i);
            self.reset_attack_strength();
        } else if !self.entity_targeted()
            && let Some(hit) = self.pick()
        {
            self.remote_mining(true);
            instant = self
                .res
                .facts()
                .is_air(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        } else {
            if !self.creative {
                self.clicks.miss_time = 10;
            }
            self.reset_attack_strength();
        }
        self.swing_arm();
        self.send(S::Punch(Empty));
        instant
    }

    /// `MultiPlayerGameMode.attack` runs `Player.attack` on this client too.
    /// For a mob `hurtOrSimulate` fails here (`Entity.hurtClient`), so the
    /// hit has no attacker side; but `RemotePlayer.hurtClient` is true, so a
    /// hit on another player with extra knockback runs the attacker's side of
    /// `causeExtraKnockback`: the 0.6 horizontal speed cut and the sprint
    /// stop. Without it the next move outruns the server's player.
    fn remote_player_attack(&mut self, mob: usize) {
        // Java dispatches this through the target's RemotePlayer type. The
        // level_players map is profile bookkeeping and can lag the entity;
        // mobs must stay out because their hurtClient returns false.
        if self.mobs[mob].kind.id != "minecraft:player" {
            return;
        }
        // `getAttackStrengthScale(0.5)`: here the ticker is `hand_strength`'s
        // (`attack_ticks` only counts in the local world), and the attack
        // speed is the player's synced attribute, which a server may change.
        let speed = self
            .own_attribute("minecraft:attack_speed")
            .unwrap_or_else(|| self.main_hand_attack_speed());
        let strength = self.hand_strength.scale(0.5, speed);
        // Player.attack's damage: a riptide spin's, or this client's own
        // ATTACK_DAMAGE, which the held item and effects don't change here.
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 {
            spin
        } else {
            self.own_attribute("minecraft:attack_damage")
                .map_or(1.0, |v| v as f32)
        };
        let knockback = self
            .own_attribute("minecraft:attack_knockback")
            .map_or(0.0, |v| v as f32);
        let sprinting = self.player.as_ref().is_some_and(|p| p.sprinting);
        if !horizon_client::attack_strength::attacker_slows_after_hit(
            damage, knockback, strength, sprinting,
        ) {
            return;
        }
        if let Some(p) = self.player.as_mut() {
            let slowdown = horizon_client::attack_strength::ATTACKER_SLOWDOWN;
            p.velocity = Vec3::new(
                p.velocity.x * slowdown,
                p.velocity.y,
                p.velocity.z * slowdown,
            );
        }
        self.stop_sprint_after_extra_knockback();
    }

    /// An attribute's value on this client's player, if its map has the attribute.
    fn own_attribute(&mut self, id: &str) -> Option<f64> {
        let p = self.player.as_mut()?;
        let key = p.attributes.key(id)?;
        p.attributes.lookup(Some(&key))?;
        p.attributes
            .value(Some(key), horizon_world::attribute_map::ValueKind::Computed)
            .ok()
    }

    fn remote_continue_attack(&mut self, down: bool) {
        if !down {
            self.clicks.miss_time = 0;
        }
        // A piercing weapon neither breaks blocks nor swings on its own.
        if self.clicks.miss_time > 0
            || self.using.is_some()
            || self
                .held()
                .is_some_and(|s| horizon_client::spear::Kinetic::of(&s.item).is_some())
        {
            return;
        }
        let entity = self.entity_targeted();
        if down
            && !entity
            && let Some(hit) = self.pick()
        {
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            if self.remote_break_restricted() {
                // continueDestroyBlock returned false: no swing.
            } else if !self.res.facts().is_air(state) {
                self.remote_mining(false);
                self.swing_arm();
                self.send(S::Punch(Empty));
            } else {
                if let Some(r) = self.remote.as_mut() {
                    r.destroying = None;
                    r.is_destroying = false;
                }
                self.mining = None;
            }
        } else {
            // `stopDestroyBlock` returns at once for a break that is not running: a pick that
            // missed once a break stopped ends nothing, so the dig's accumulated progress stays
            // and goes on growing on the jumps that follow (#4124).
            if self.stop_remote_mining() {
                self.mining = None;
            }
        }
    }
    /// Keys with a multiplayer meaning; returns true when consumed.
    pub(super) fn remote_key(&mut self, key: KeyCode) -> Result<bool> {
        if self.screen != Screen::Playing {
            if (key == KeyCode::KeyE || key == KeyCode::Escape) && self.screen == Screen::Inventory
            {
                self.remote_close_container();
                self.screen = Screen::Playing;
                self.capture(true);
                return Ok(true);
            }
            return Ok(false);
        }
        match key {
            KeyCode::KeyT | KeyCode::Slash | KeyCode::Enter => {
                // ChatScreen, over the server's command tree.
                self.open_chat_method(if key == KeyCode::Slash { "/" } else { "" });
                // `Minecraft.handleKeybinds` opens it in the call that then
                // releases an active use (Gui.handleKeybinds runs before the
                // isUsingItem branch).
                self.release_use_for_keybind_screen();
                Ok(true)
            }
            KeyCode::KeyQ => {
                if self.spectator() {
                    return Ok(true);
                }
                if let Some(r) = self.remote.as_mut() {
                    r.pending_keys.push(key);
                }
                Ok(true)
            }
            // Open the ridden vehicle's own screen (`HasCustomInventoryScreen`)
            // instead of the player's inventory: riding a horse, llama or
            // chest boat, the inventory key sends
            // ServerboundPlayerCommandPacket OPEN_INVENTORY.
            KeyCode::KeyE if self.riding_mount_with_menu() || self.riding_chest_boat() => {
                if let Some(r) = self.remote.as_mut() {
                    r.pending_keys.push(key);
                }
                Ok(true)
            }
            KeyCode::KeyE => {
                // The inventory opens without telling the server.
                if self.tutorial.open_inventory() {
                    self.persist_tutorial_step();
                }
                self.recipe_ui.reset_screen();
                if self.creative {
                    self.screen = Screen::Creative;
                    self.refresh_creative_tabs();
                    self.creative_screen.reopen();
                    self.refresh_saved_hotbars();
                } else {
                    self.screen = Screen::Inventory;
                }
                self.craft_size = 2;
                self.capture(false);
                // `Minecraft.handleKeybinds` opens the inventory in the same
                // call that then releases an active use.
                self.release_use_for_keybind_screen();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// `Minecraft.handleKeybinds`: a key that opens a screen runs
    /// `Gui.setScreen`, whose `KeyMapping.releaseAll` clears the use key, and
    /// the same invocation then releases an active use
    /// (`player.isUsingItem() && !keyUse.isDown()` → `releaseUsingItem`).
    /// Only the screens opened from those keybinds do this (inventory, chat,
    /// advancements); the pause menu, server containers, mount inventories and
    /// dialogs release on the first tick after closing instead, as vanilla
    /// does.
    pub(super) fn release_use_for_keybind_screen(&mut self) {
        // In-frame (deprecated) has no server client; `tick_used_item` releases
        // the use on its own.
        if self.remote.is_none() {
            return;
        }
        // `KeyMapping.releaseAll` clears key.use; without this the button still
        // reads held and a use would start again on the first tick back.
        self.right = false;
        horizon_client::interaction_input::release_on_screen_open(&mut RemoteInputWorld(self));
    }
    /// `MultiPlayerGameMode.releaseUsingItem`: sends `RELEASE_USE_ITEM` and
    /// runs the client half of `Item.releaseUsing` (a Riptide launch moves the
    /// player itself). Vanilla reaches it both when the use key is let go and,
    /// in the same tick, when a keybind opens a screen.
    fn release_remote_use(&mut self) {
        self.send(S::PlayerAction(play::PlayerAction {
            action: 6,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
        let hand = self.used_hand();
        if let Some((slot, ticks)) = self.using.take()
            && let Some(hand) = hand
            && self
                .stack_in(slot)
                .is_some_and(|s| &*s.item == "minecraft:trident")
        {
            self.with_hand(hand, |g| g.release_trident(ticks));
        }
    }
    /// ClientPacketListener.sendChat / sendCommand, signed with a chat
    /// session (game/remote_chat.rs). The Chat option's Hidden and System
    /// levels keep chat out and Hidden keeps the commands out (ChatAbilities).
    pub(super) fn send_chat(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() || !self.chat_abilities().allows(message) {
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            self.send_command(command);
        } else {
            self.send_signed_chat(message);
        }
    }
    /// MultiPlayerGameMode.dropItem (key.drop): predict the removal, then send
    /// the action; swing only if a nonempty stack was removed.
    pub(super) fn remote_drop(&mut self, all: bool) {
        let dropped =
            horizon_client::items::take_held_drop(&mut self.inventory[self.selected], all, false);
        self.send(S::PlayerAction(play::PlayerAction {
            action: if all { 4 } else { 5 },
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
        if dropped.is_some() {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
    }
    /// Minecraft.handleKeybinds, key.swapOffhand: the server swaps the hands
    /// (ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND at BlockPos.ZERO, facing down) and
    /// sends both slots back; nothing is predicted.
    pub(super) fn remote_swap_hands(&mut self) {
        self.send(S::PlayerAction(play::PlayerAction {
            action: 7,
            pos: [0; 3],
            direction: 0,
            sequence: 0,
        }));
        // Equal stacks swap with no slot update to wait for.
        let (main, off) = (self.stack_in(self.selected), self.offhand.as_ref());
        let moves = !super::hand::matches_for_swap(main, off);
        if let Some(r) = self.remote.as_mut() {
            r.swap_answer = [moves; 2];
        }
    }
    /// The server set inventory `slot` (`shield::OFFHAND_SLOT` for the off
    /// hand), `changed` from the stack this client held there. The stack
    /// that answers a swap (`setItemInHand`) is another object, which the
    /// hand shows after the swap animation (`hand_stack_replaced`); other
    /// updates of a hand's slot count as the held stack changing in place,
    /// as this client doesn't predict every use the way vanilla's does
    /// (a thrown snowball's stack shrinks only when the server says so).
    fn remote_hand_slot_set(&mut self, slot: usize, changed: bool) {
        let hand = if slot == super::shield::OFFHAND_SLOT {
            1
        } else if slot == self.selected {
            0
        } else {
            return;
        };
        let answer = self
            .remote
            .as_mut()
            .is_some_and(|r| std::mem::take(&mut r.swap_answer[hand]));
        if answer && changed {
            self.hand_stack_replaced(slot);
        }
    }
    /// A container click: the server answers with the authoritative slots.
    pub(super) fn remote_click(&mut self, right: bool) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size);
        let Some(internal) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let input = if self.has_shift_down() {
            Input::QuickMove
        } else {
            Input::Pickup
        };
        if self.screen == Screen::Creative {
            // CreativeModeInventoryScreen.slotClicked: the Survival Inventory tab clicks the
            // player's own InventoryMenu, and its CreativeInventoryListener sends each slot that
            // changed (SetCreativeModeSlot). No ContainerClick: a server holding the carried
            // stack would give it back after the Destroy Item slot cleared it (#3307).
            self.menu_click(Some(internal), i32::from(right), input);
            return;
        }
        self.remote_menu_ops(&[(Some(internal), i32::from(right), input)]);
    }
    /// `AbstractContainerMenu.setItem`/`initializeContents`: the state id a
    /// click answers with is the open menu's, not another window's.
    fn remote_menu_state(&mut self, container: i32, state: i32) {
        let open = self.remote_menu().0;
        if let Some(r) = self.remote.as_mut().filter(|_| container == open) {
            r.state_id = state;
        }
    }
    /// The server's open menu: its window id and menu type (0, "inventory"
    /// when none is open).
    pub(super) fn remote_menu(&self) -> (i32, String) {
        match self.remote.as_ref().and_then(|r| r.menu.clone()) {
            Some((id, name)) => (id, name),
            None => (0, "inventory".to_owned()),
        }
    }
    /// `MultiPlayerGameMode.handleContainerInput` for each click of `ops` on
    /// the server's open menu (a quick craft drag is several): the verified
    /// menu engine predicts them on this client's copy, then each goes out as
    /// a `ContainerClick` with the slots it changed and the carried stack
    /// after it; the server corrects anything it sees differently. A menu the
    /// engine doesn't model sends the clicks unpredicted. False when a click's
    /// slot isn't one of the menu's.
    pub(super) fn remote_menu_ops(&mut self, ops: &[(Option<usize>, i32, Input)]) -> bool {
        // CrafterScreen.slotClicked: a slot toggle goes out before the click.
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        let (container, menu) = self.remote_menu();
        let mut clicks = Vec::with_capacity(ops.len());
        for &(slot, button, input) in ops {
            let slot = match slot {
                None => -999,
                Some(internal) => match Self::menu_slot_index(&menu, internal) {
                    Some(s) => s as i16,
                    None => return false,
                },
            };
            clicks.push((slot, button, input));
        }
        // Unpredicted: nothing changed, the carried stack as it is.
        let mut sent: Vec<Predicted> = clicks
            .iter()
            .map(|&(slot, button, input)| (slot, button, input, Vec::new(), self.carried.clone()))
            .collect();
        let mut threw = false;
        // The engine's slot indices are the menu's own, as on the network.
        self.menu_session(|m, env, _| {
            let Some((predicted, dropped)) = predict_clicks(m, env, &clicks) else {
                // The server throws on it too: nothing is predicted.
                return Some(false);
            };
            sent = predicted;
            threw = dropped;
            Some(true)
        });
        self.send_clicks(container, sent);
        // LivingEntity.drop(.., Prediction.PREDICTED): the client swings its own
        // main hand; the server shows the swing only to the other players.
        if threw {
            self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        }
        true
    }
    /// A Shift double click on a server (`AbstractContainerScreen.mouseReleased`):
    /// a predicted QUICK_MOVE for every slot the engine moves, each sent.
    pub(super) fn remote_quick_move_all(&mut self, slot: usize, moved: &ItemStack) {
        let (container, _) = self.remote_menu();
        let mut sent: Vec<Predicted> = Vec::new();
        self.menu_session(|m, env, to_menu| {
            let clicked = to_menu(slot)?;
            let mut before = menu_items(m);
            let mut each = |m: &horizon_client::menu::Menu, t: usize| {
                let changed = changed_slots(&before, m);
                sent.push((t as i16, 0, Input::QuickMove, changed, m.carried.clone()));
                before = menu_items(m);
            };
            Some(
                m.quick_move_all_each(env, clicked, moved, &mut each)
                    .is_ok(),
            )
        });
        self.send_clicks(container, sent);
    }
    fn send_clicks(&mut self, container: i32, clicks: Vec<Predicted>) {
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        let hashed = |g: &Self, s: &Option<ItemStack>| g.hashed_stack(s);
        for (slot, button, input, changed, carried) in clicks {
            let changed = changed.iter().map(|(i, s)| (*i, hashed(self, s))).collect();
            let carried = hashed(self, &carried);
            self.send(S::ContainerClick(play::ContainerClick {
                container,
                state,
                slot,
                button: button as i8,
                input: input_ordinal(input),
                changed,
                carried,
            }));
        }
    }
    /// The server's open menu is the inventory or a chest-like container
    /// (chests, barrels, ender chests: `generic_9xN`; shulker boxes).
    pub(super) fn remote_storage_open(&self) -> bool {
        match self.remote.as_ref().and_then(|r| r.menu.as_ref()) {
            None => true,
            Some((_, name)) => {
                name == "inventory"
                    || name.starts_with("minecraft:generic_9x")
                    || name == "minecraft:shulker_box"
            }
        }
    }
    /// A click outside a server's window (slot -999): predicted like
    /// `MultiPlayerGameMode.handleContainerInput` (the carried stack after
    /// `doClick`; the server drops the stack and swings the hand).
    pub(super) fn remote_throw(&mut self, right: bool, input: horizon_client::menu::Input) {
        use horizon_client::menu::Input;
        let container = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0);
        if input == Input::Pickup
            && let Some(c) = self.carried.as_mut()
        {
            if right && c.count > 1 {
                c.count -= 1;
            } else {
                self.carried = None;
            }
            self.swing_arm();
        }
        let carried = self.hashed_stack(&self.carried);
        let state = self.remote.as_ref().map_or(0, |r| r.state_id);
        self.send(S::ContainerClick(play::ContainerClick {
            container,
            state,
            slot: -999,
            button: i8::from(right),
            input: if input == Input::Throw { 4 } else { 0 },
            changed: Vec::new(),
            carried,
        }));
    }
    /// `CreativeModeInventoryScreen.slotClicked` outside the window: the server knows nothing of
    /// a stack picked from the creative screen, so the carried stack (an item tab's right click:
    /// one of it) is dropped through `handleCreativeModeItemDrop`, a SetCreativeModeSlot to slot
    /// -1, and the hand swings as `Player.drop` predicts.
    pub(super) fn remote_creative_drop(&mut self, right: bool) {
        let one = right && !self.creative_inventory_open();
        let Some(carried) = self.carried.as_mut() else {
            return;
        };
        let thrown = if one && carried.count > 1 {
            carried.count -= 1;
            ItemStack {
                count: 1,
                ..carried.clone()
            }
        } else {
            self.carried.take().expect("carried")
        };
        self.remote_creative_throw(thrown);
    }
    /// `MultiPlayerGameMode.handleCreativeModeItemDrop`: a stack thrown out of the creative
    /// screen goes to the server as a SetCreativeModeSlot to slot -1, which drops it from the
    /// player's hand; the client only swings (`Player.drop` on a client).
    pub(super) fn remote_creative_throw(&mut self, thrown: ItemStack) {
        if !self.creative {
            return;
        }
        self.swing_arm();
        let item = self.to_network(&Some(thrown));
        self.send(S::SetCreativeModeSlot(play::SetCreativeModeSlot {
            slot: -1,
            item,
        }));
    }
    pub(super) fn remote_close_container(&mut self) {
        let id = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref().map(|m| m.0))
            .unwrap_or(0);
        self.send(S::ContainerClose(play::ContainerClose { container: id }));
        self.remote_close_local();
    }
    fn remote_close_local(&mut self) {
        self.recipe_ui.reset_screen();
        if let Some(r) = self.remote.as_mut() {
            r.menu = None;
            r.menu_title = None;
        }
        self.mount_menu_reset();
        // The client's menu has no level access: what it holds is the server's.
        self.station = None;
        // AbstractContainerMenu.removed: the server gives the grid and the
        // carried stack back to the inventory and syncs it; the copies here
        // are stale the moment the window closes.
        self.craft_grid.fill(None);
        self.carried = None;
        self.craft_size = 2;
        if let Some(key) = self.container.take() {
            // Server containers stay renderable but lose their copied slots.
            if let Some(entity) = self.world.block_entities.get_mut(&key) {
                for s in entity.slots_mut() {
                    *s = None;
                }
            }
        }
    }
    /// A server's trading window (MerchantScreen over a ClientSideMerchant).
    fn remote_open_merchant(
        &mut self,
        container: i32,
        title: String,
        text: horizon_client::text::Text,
    ) {
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((container, "minecraft:merchant".to_owned()));
            r.menu_title = Some(text);
        }
        let session = horizon_client::merchant::Session::new(Vec::new(), true);
        self.open_remote_merchant(session, (!title.is_empty()).then_some(title));
    }
    /// ClientboundMerchantOffersPacket.
    fn remote_merchant_offers(&mut self, p: play::MerchantOffers) {
        if self.open_menu_name(p.container) != "minecraft:merchant" {
            return;
        }
        let cost = |g: &Self, c: &play::NetCost| {
            g.to_stack(&Some(c.clone().into()))
                .map(|stack| horizon_client::merchant::ItemCost { stack })
        };
        let offers: Vec<_> = p
            .offers
            .iter()
            .filter_map(|o| {
                let mut offer = horizon_client::merchant::MerchantOffer {
                    base_cost_a: cost(self, &o.cost_a)?,
                    cost_b: match &o.cost_b {
                        Some(b) => Some(cost(self, b)?),
                        None => None,
                    },
                    result: self.to_stack(&Some(o.result.clone()))?,
                    uses: o.uses,
                    max_uses: o.max_uses,
                    reward_exp: true,
                    special_price_diff: o.special_price,
                    demand: o.demand,
                    price_multiplier: o.price_multiplier,
                    xp: o.xp,
                };
                // setToOutOfStock.
                if o.out_of_stock {
                    offer.uses = offer.max_uses;
                }
                Some(offer)
            })
            .collect();
        if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
            m.session.offers = offers;
            m.session.level = p.level;
            m.session.xp = p.xp;
            m.session.villager = p.show_progress;
        }
    }
    /// ClientboundOpenScreenPacket for the menus Horizon draws.
    fn remote_open_screen(&mut self, p: play::OpenScreen) {
        // A new RecipeBookComponent owns fresh ghost slots and an overlay,
        // even when the server replaces a menu without a close packet.
        self.recipe_ui.reset_screen();
        let menu = self
            .remote
            .as_ref()
            .and_then(|r| {
                r.builtin
                    .get("minecraft:menu")?
                    .name(p.menu)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let target = self
            .remote
            .as_mut()
            .and_then(|r| r.use_target.take())
            .map(|t| (t[0], t[1], t[2]));
        if menu == "minecraft:lectern" {
            // LecternScreen: the book and page come in the menu's content and data.
            if let Some(r) = self.remote.as_mut() {
                r.menu = Some((p.container, menu));
            }
            self.remote_open_lectern(target.unwrap_or_default());
            return;
        }
        if let Some(kind) = Self::remote_station_kind(&menu) {
            let title = menu_title(&p.title);
            let pos = target.map_or([0, 0, 0], |t| [t.0, t.1, t.2]);
            self.remote_open_station(p.container, menu, kind, title, pos);
            return;
        }
        if menu == "minecraft:merchant" {
            // A trading window: its offers follow in a MerchantOffers packet.
            let title = plain(
                &p.title,
                &self
                    .remote
                    .as_ref()
                    .map(|r| r.lang.clone())
                    .unwrap_or_default(),
            );
            self.remote_open_merchant(p.container, title, menu_title(&p.title));
            return;
        }
        let mod_type = horizon_client::containers::mod_menu_type(&menu);
        let entity = match menu.as_str() {
            "minecraft:crafting" => None,
            "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                let kind = match menu.as_str() {
                    "minecraft:blast_furnace" => FurnaceKind::BlastFurnace,
                    "minecraft:smoker" => FurnaceKind::Smoker,
                    _ => FurnaceKind::Furnace,
                };
                Some(BlockEntity::Furnace(Furnace {
                    kind,
                    items: [None, None, None],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 0,
                    speed: 1.0,
                    recipes_used: std::collections::HashMap::new(),
                }))
            }
            // HopperMenu, and DispenserMenu (MenuType.GENERIC_3x3: dispensers
            // and droppers).
            "minecraft:hopper" => Some(BlockEntity::Chest(vec![None; 5])),
            // BrewingStandMenu: its times come in ContainerSetData.
            "minecraft:brewing_stand" => Some(BlockEntity::Brewing(Box::new(
                horizon_client::brewing::BrewingStand::new(),
            ))),
            "minecraft:generic_3x3" => Some(BlockEntity::Chest(vec![None; 9])),
            // CrafterMenu: the disabled slots and triggered come in ContainerSetData.
            "minecraft:crafter_3x3" => Some(BlockEntity::Crafter(Default::default())),
            m if m.starts_with("minecraft:generic_9x") || m == "minecraft:shulker_box" => {
                let rows = m
                    .strip_prefix("minecraft:generic_9x")
                    .and_then(|r| r.parse::<usize>().ok())
                    .unwrap_or(3);
                Some(BlockEntity::Chest(vec![None; rows * 9]))
            }
            // A content mod's menu type: its container's slots, which its
            // mod's screen places.
            _ if mod_type.is_some() => Some(BlockEntity::Mod(Box::new(
                horizon_client::containers::ModBlockEntity {
                    items: vec![None; mod_type.map_or(0, |t| t.slots)],
                    ..Default::default()
                },
            ))),
            _ => {
                // Unsupported menu: tell the server we closed it.
                self.send(S::ContainerClose(play::ContainerClose {
                    container: p.container,
                }));
                self.chat_line(
                    horizon_client::text::Text::literal(format!(
                        "(Horizon cannot show {menu} screens yet)"
                    )),
                    horizon_client::chat::ChatSource::Client,
                );
                return;
            }
        };
        let title = menu_title(&p.title);
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((p.container, menu.clone()));
            r.menu_title = Some(title);
        }
        self.craft_size = if menu == "minecraft:crafting" { 3 } else { 2 };
        self.container = None;
        if let Some(entity) = entity {
            // The clicked block shows the menu when it holds that container;
            // an ender chest's or a double chest's slots are no one block's,
            // so they go under a key no block has.
            let holds = |k: &(i32, i32, i32)| {
                let name = &self.res.facts().get(self.world.block(k.0, k.1, k.2)).name;
                BlockEntity::for_block(name).is_some_and(|e| {
                    e.id() == entity.id() && e.slots().len() == entity.slots().len()
                })
            };
            let key = target.filter(holds).unwrap_or((i32::MIN, 0, 0));
            self.world.block_entities.insert(key, entity);
            self.container = Some(key);
            self.open_native_screen(key, target, p.container, &menu);
            // A mod menu type shows through its mod's screen only: without one
            // (`MenuScreens.create` finds no screen) the window closes again.
            if self.mod_screen.is_none() && mod_type.is_some() {
                self.world.block_entities.remove(&key);
                self.container = None;
                if let Some(r) = self.remote.as_mut() {
                    r.menu = None;
                    r.menu_title = None;
                }
                self.send(S::ContainerClose(play::ContainerClose {
                    container: p.container,
                }));
                return;
            }
        }
        self.screen = Screen::Inventory;
        self.capture(false);
    }
    /// `ClientPacketListener.handleMountScreenOpen`: a horse's or llama's
    /// inventory; for another entity the window is closed again.
    fn remote_open_mount(&mut self, p: play::MountScreenOpen) {
        let columns = usize::try_from(p.columns).unwrap_or(0).min(5);
        if !self.open_remote_mount_menu(p.entity as u32, columns) {
            self.send(S::ContainerClose(play::ContainerClose {
                container: p.container,
            }));
            return;
        }
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((p.container, format!("{MOUNT_MENU}{columns}")));
            r.menu_title = None;
        }
    }
    /// The type of the menu a server opened (`MenuType`'s id).
    pub(super) fn remote_menu_type(&self) -> Option<&str> {
        Some(self.remote.as_ref()?.menu.as_ref()?.1.as_str())
    }
    /// The title the server gave the menu it opened.
    pub(super) fn remote_menu_title(&self) -> Option<horizon_client::text::Text> {
        self.remote.as_ref()?.menu_title.clone()
    }
    /// A language change reaches the server-sent components too: their titles
    /// are resolved when the packet arrives, with the session's language.
    pub(super) fn refresh_remote_language(&mut self) {
        if let Some(lang) = horizon_client::lang::current()
            && let Some(r) = self.remote.as_mut()
        {
            r.lang = Arc::new(lang.map().clone());
        }
    }
    /// A server's chest-like menu (`MenuType.GENERIC_9xN`): its rows and title.
    pub(super) fn remote_chest_menu(&self) -> Option<(usize, horizon_client::text::Text)> {
        let r = self.remote.as_ref()?;
        let rows = r
            .menu
            .as_ref()?
            .1
            .strip_prefix("minecraft:generic_9x")?
            .parse()
            .ok()?;
        Some((
            rows,
            r.menu_title
                .clone()
                .unwrap_or_else(horizon_client::text::Text::empty),
        ))
    }
    /// ServerboundPlayerCommandPacket STOP_SLEEPING.
    pub(super) fn remote_stop_sleeping(&mut self) {
        let id = self.remote.as_ref().map_or(0, |r| r.entity_id);
        self.send(S::PlayerCommand(play::PlayerCommand {
            id,
            action: 0,
            data: 0,
        }));
    }
    pub(super) fn remote_respawn(&mut self) {
        self.send(S::ClientCommand(play::ClientCommand { action: 0 }));
    }

    /// Connecting / loading screens; true when it covered the frame.
    pub(super) fn remote_overlay(&mut self, batch: &mut HudBatch) -> bool {
        let Some(r) = &self.remote else { return false };
        let loading = r.level_load.is_active();
        if self.player.is_some() && !loading {
            return false;
        }
        let (w, h) = (batch.width, batch.height);
        if let Some(since) = r.reconfiguring {
            // ServerReconfigScreen narrates its title.
            horizon_client::widgets::begin_screen(|| {
                horizon_client::widgets::ScreenInfo::new(
                    "reconfiguring",
                    super::screen_info::tr("connect.reconfiguring", &[]),
                )
            });
            self.draw_reconfiguring(batch, since);
            return true;
        }
        if r.joined || loading {
            // LevelLoadingScreen: the world's terrain comes in. A server's
            // chunk status view and progress are the integrated server's alone
            // (`LevelLoadTracker.statusView`, `hasProgress`), and so is what
            // the narrator says of it.
            let progress = self
                .integrated
                .as_ref()
                .map(|i| i.load_view())
                .filter(|v| v.has_progress())
                .map(|v| v.server_progress());
            horizon_client::widgets::begin_screen(|| {
                super::screen_info::level_loading(progress, &super::screen_info::tr)
            });
            self.draw_level_loading_background(batch, r.level_load.reason);
            let atlas = &self.gui.atlas;
            match self.integrated.as_ref().map(|i| i.load_view()) {
                Some(view) => {
                    let status = |x: i32, z: i32| view.status(x, z);
                    let progress = view.has_progress().then_some(self.loading_progress);
                    horizon_client::level_loading::draw(batch, atlas, Some(&status), progress);
                }
                None => horizon_client::level_loading::draw(batch, atlas, None, None),
            }
            return true;
        }
        let cancel = self.connect_cancel((w, h)).is_some();
        // ConnectScreen.extractRenderState: "Joining the world" every 2 s (a
        // server's: single player's isn't joined through ConnectScreen).
        if self.integrated.is_none()
            && let Some(r) = self.remote.as_mut()
            && r.joining_narrated
                .is_none_or(|t| t.elapsed() > Duration::from_millis(2000))
        {
            r.joining_narrated = Some(Instant::now());
            self.say_system_now(&horizon_client::text::Text::translate(
                "narrator.joining",
                &[],
            ));
        }
        let Some(r) = &self.remote else { return false };
        // ConnectScreen: `GameNarrator.NO_TITLE`, then its Cancel; single
        // player's server is joined under its LevelLoadingScreen.
        let single_player = self.integrated.is_some();
        horizon_client::widgets::begin_screen(|| {
            if single_player {
                super::screen_info::level_loading(None, &super::screen_info::tr)
            } else {
                horizon_client::widgets::ScreenInfo::new("connect", "")
            }
        });
        let atlas = &self.gui.atlas;
        let status = horizon_client::text::Text::literal(r.status.as_str());
        horizon_client::level_loading::draw_connecting(batch, atlas, &status, self.cursor, cancel);
        true
    }
    /// Connected or connecting with no player yet: `ConnectScreen`, then
    /// `LevelLoadingScreen`, over the panorama with its blur.
    pub(super) fn remote_loading(&self) -> bool {
        use horizon_client::level_loading::Reason;
        self.remote.as_ref().is_some_and(|r| {
            if r.level_load.is_active() {
                // The portal screens have no panorama, so no blur.
                r.level_load.reason == Reason::Other
            } else {
                self.player.is_none()
            }
        })
    }
    /// `ServerReconfigScreen`: a vertical layout of cells padded by 10, the
    /// title (9 high) over Disconnect (150 x 20), centred. The title's and the
    /// button's top left corners.
    fn reconfigure_layout(&self, (w, h): (f32, f32)) -> ((f32, f32), (f32, f32)) {
        let title = horizon_client::text::Text::translate("connect.reconfiguring", &[]);
        let title_w = self.gui.atlas.font.width_text(&title).ceil() as i32;
        let width = (title_w + 20).max(170);
        let left = ((w as i32 - width) as f32 * 0.5) as i32;
        let top = ((h as i32 - 69) as f32 * 0.5) as i32;
        let across = |child: i32| (10.0 + ((width - child - 10) as f32 - 10.0) * 0.5) as i32;
        (
            ((left + across(title_w)) as f32, (top + 10) as f32),
            ((left + across(150)) as f32, (top + 39) as f32),
        )
    }
    /// `ServerReconfigScreen` over the panorama; Disconnect wakes up after
    /// `DISCONNECT_TIME` (600 ticks).
    fn draw_reconfiguring(&self, batch: &mut HudBatch, since: u64) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        horizon_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        let title = horizon_client::text::Text::translate("connect.reconfiguring", &[]);
        let ((tx, ty), (bx, by)) = self.reconfigure_layout((w, h));
        batch.text_styled(atlas, &title, tx, ty, u32::MAX, true);
        // CommonComponents.GUI_DISCONNECT
        let label = horizon_client::text::Text::translate("menu.disconnect", &[]);
        let active = self.ticks.saturating_sub(since) >= 600;
        let hover = horizon_client::ui::hovered(self.cursor, bx, by, 150.0, 20.0);
        horizon_client::ui::button(batch, atlas, bx, by, 150.0, 20.0, &label, active, hover);
    }
    /// `ConnectScreen`'s Cancel: a server's (single player's server isn't
    /// joined through `ConnectScreen`), until the login is done.
    fn connect_cancel(&self, (w, h): (f32, f32)) -> Option<(f32, f32)> {
        let r = self.remote.as_ref()?;
        (self.player.is_none() && !r.joined && self.integrated.is_none())
            .then(|| horizon_client::level_loading::connect_cancel(w, h))
    }
    /// A click on `ConnectScreen`: Cancel aborts the connection
    /// (`connect.aborted`) and goes back. Escape does nothing there
    /// (`shouldCloseOnEsc` is false).
    pub(super) fn connect_screen_click(&mut self) -> Result<()> {
        // ServerReconfigScreen's Disconnect, once it is active.
        if let Some(since) = self.remote.as_ref().and_then(|r| r.reconfiguring) {
            let (_, (x, y)) = self.reconfigure_layout(self.gui_size());
            if self.ticks.saturating_sub(since) >= 600
                && horizon_client::ui::hovered(self.cursor, x, y, 150.0, 20.0)
            {
                self.audio.play_button_click();
                self.leave_server(None)?;
            }
            return Ok(());
        }
        let Some((x, y)) = self.connect_cancel(self.gui_size()) else {
            return Ok(());
        };
        if horizon_client::ui::hovered(self.cursor, x, y, 200.0, 20.0) {
            self.audio.play_button_click();
            self.leave_server(None)?;
        }
        Ok(())
    }
    /// The server's death message; chat uses the shared ChatComponent.
    pub(super) fn remote_hud(&self, batch: &mut HudBatch) {
        let Some(r) = &self.remote else { return };
        let atlas = &self.gui.atlas;
        let w = batch.width;
        if self.screen == Screen::Dead
            && let Some(message) = &r.death_message
        {
            // DeathScreen draws the styled cause component.
            let tw = atlas.font.width_text(message);
            batch.text_styled(
                atlas,
                message,
                ((w - tw) / 2.0).floor(),
                85.0,
                u32::MAX,
                true,
            );
        }
    }
    /// Shared flag 6 of an entity on a server's client (`None` without a server).
    pub(super) fn remote_glowing(&self, id: u32) -> Option<bool> {
        let r = self.remote.as_ref()?;
        Some(r.glowing.contains(&(id as i32)))
    }
    /// The profile name of another player the server sent, by entity id.
    pub(super) fn remote_player_name(&self, id: u32) -> Option<&str> {
        let r = self.remote.as_ref()?;
        let uuid = r.level_players.get(&(id as i32))?;
        if *uuid == r.profile {
            return None;
        }
        r.level_player_name(id as i32, uuid)
    }
    /// Status for `horizon-ctl status`.
    /// The other players' entities in the level and their names (their
    /// profiles', which outlive the player list's entries).
    pub(super) fn remote_player_names(&self) -> Vec<(u32, String)> {
        let Some(r) = self.remote.as_ref() else {
            return Vec::new();
        };
        r.level_players
            .iter()
            .filter(|(_, uuid)| **uuid != r.profile)
            .filter_map(|(id, uuid)| Some((*id as u32, r.level_player_name(*id, uuid)?.to_owned())))
            .collect()
    }
    /// `ClientClockManager.tick`: the clock moves on by the game time that
    /// passed since its last tick, at its rate. A game time the server sets
    /// forward (its sync every second) moves the clock with it.
    fn remote_clock_tick(&mut self) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let delta = self.world.game_time - r.clock_game_time;
        r.clock_game_time = self.world.game_time;
        let partial = f64::from(r.clock_partial) + delta as f64 * f64::from(r.clock_rate);
        let full = partial.floor();
        r.clock_partial = (partial - full) as f32;
        self.world.day_time += full as i64;
    }
    pub(super) fn remote_status(&self) -> Option<serde_json::Value> {
        let r = self.remote.as_ref()?;
        Some(serde_json::json!({
            "address": horizon_client::net::shown_address(&r.session.address),
            "name": r.session.name,
            "joined": r.joined,
            // ServerboundPlayerLoadedPacket sent: the level around the
            // player is ready, and the client ticks and sends its moves.
            "loaded": r.loaded_sent,
            "status": r.status,
            "entity_id": r.entity_id,
            "dimension": r.dimension.name,
            "game_mode": r.game_mode,
            "chunks": self.world.chunks.len(),
            "entities": r.entities.values().fold(std::collections::BTreeMap::<&str, u32>::new(), |mut m, t| {
                *m.entry(t.kind.as_str()).or_default() += 1;
                m
            }),
            "players": r.players.values().collect::<Vec<_>>(),
            // Where the other players' entities are (harnesses check movement).
            // What the other players hold and wear, as this client draws them.
            "player_equipment": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)))
                .map(|m| {
                    let e = &m.extra;
                    let armor = [&e.head, &e.chest, &e.legs, &e.feet].map(|s| s.as_deref());
                    serde_json::json!({
                        "id": m.id,
                        "swinging": m.ai.swinging,
                        "using": m.extra.using_item,
                        "armor": armor,
                        "main_hand": e.main_hand.as_deref(),
                        "off_hand": e.off_hand.as_deref(),
                    })
                })
                .collect::<Vec<_>>(),
            "player_positions": r
                .entities
                .values()
                .filter(|t| t.kind == "minecraft:player")
                .map(|t| t.position)
                .collect::<Vec<_>>(),
            // The other players this client draws dead (lying down, after EntityEvent DEATH).
            "dead_players": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)) && !m.alive())
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            // The other players this client draws lying in a bed (SLEEPING_POS).
            "lying_players": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)) && m.extra.ints[1] != 0)
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            // The player's own model exists (third person and the inventory draw it).
            "player_model": self.player_model.is_some(),
            // The world view draws its body now (third person, or lying in a bed;
            // not while invisible).
            "own_body": self.player_model_shown() && self.own_body_visible(),
            "text_display_vertices": self.text_display_vertices,
            "text_displays": self.text_displays.len(),
            "health": r.health.0,
            "food": r.health.1,
            "decode_errors": r.decode_errors,
            "chat": r.chat.iter().rev().take(10).rev().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
        }))
    }
}

/// `Entity.lookAt`'s rotation from `from` towards `to`, as (yaw, pitch).
fn look_at_rotation(from: [f64; 3], to: [f64; 3]) -> (f32, f32) {
    use horizon_core::geometry::trig::atan2;
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    // `* 180.0F / (float)Math.PI` on the double.
    let degrees = |r: f64| r * 180.0 / f64::from(std::f32::consts::PI);
    let pitch = wrap_degrees((-degrees(atan2(dy, horizontal))) as f32);
    let yaw = wrap_degrees(degrees(atan2(dz, dx)) as f32 - 90.0);
    (yaw, pitch)
}

/// `PositionMoveRotation.calculateAbsolute`'s pitch: the packet's pitch,
/// added to `source` when it is relative, clamped to +-90.
fn position_packet_pitch(source: f32, change: f32, relative: bool) -> f32 {
    let offset = if relative { source } else { 0.0 };
    (offset + change).clamp(-90.0, 90.0)
}

/// `Mth.wrapDegrees`.
fn wrap_degrees(a: f32) -> f32 {
    let a = a % 360.0;
    if a >= 180.0 {
        a - 360.0
    } else if a < -180.0 {
        a + 360.0
    } else {
        a
    }
}

/// `LivingEntity.tick`'s body target and `tickHeadTurn` for another player:
/// the body turns toward where it walks (backwards when walking back), or to
/// the look while the arm swings, and stays within 50 degrees of the look.
fn remote_player_head_turn(m: &mut Mob) {
    let (dx, dz) = (m.position.x - m.previous.x, m.position.z - m.previous.z);
    let mut target = m.body_yaw;
    if (dx * dx + dz * dz) as f32 > 0.002_500_000_2 {
        let walk = (dz.atan2(dx) as f32).to_degrees() - 90.0;
        let facing = (wrap_degrees(m.yaw) - walk).abs();
        target = if 95.0 < facing && facing < 265.0 {
            walk - 180.0
        } else {
            walk
        };
    }
    if m.anim.attack_anim > 0.0 {
        target = m.yaw;
    }
    m.body_yaw += wrap_degrees(target - m.body_yaw) * 0.3;
    let head = wrap_degrees(m.yaw - m.body_yaw);
    if head.abs() > 50.0 {
        m.body_yaw += head - head.signum() * 50.0;
    }
}

/// A click sent to a server: its slot, button and input, the slots it
/// changed and the carried stack after it.
type Predicted = (
    i16,
    i32,
    Input,
    Vec<(i16, Option<ItemStack>)>,
    Option<ItemStack>,
);

/// How many stacks the creative screen keeps in step with the server (`Remote::synced`).
const CREATIVE_SYNCED: usize = 41;

/// The player's stacks in `Remote::synced` order: the 36 inventory slots, the off hand, then
/// the armour from the head down. `CreativeInventoryListener.slotChanged` sends every
/// InventoryMenu slot the creative screen changes, the armour slots too.
fn creative_sync_stacks(
    inventory: &[Option<ItemStack>],
    offhand: &Option<ItemStack>,
    armor: &[Option<ItemStack>; 4],
) -> Vec<Option<ItemStack>> {
    let mut stacks: Vec<Option<ItemStack>> = inventory.iter().take(36).cloned().collect();
    stacks.resize(36, None);
    stacks.push(offhand.clone());
    stacks.extend(armor.iter().cloned());
    stacks
}

/// Entries of `creative_sync_stacks` that went from `before` to `now` are synced as `now`.
fn mark_synced(
    synced: &mut [Option<ItemStack>],
    before: &[Option<ItemStack>],
    now: Vec<Option<ItemStack>>,
) {
    for (i, stack) in now.into_iter().enumerate() {
        if before.get(i) != Some(&stack)
            && let Some(s) = synced.get_mut(i)
        {
            *s = stack;
        }
    }
}

/// InventoryMenu's slot for entry `i` of `creative_sync_stacks`: the hotbar is 36..=44, the
/// main inventory 9..=35, the off hand 45 and the armour 5..=8 (head first).
fn creative_sync_slot(i: usize) -> i16 {
    (match i {
        0..9 => 36 + i,
        36 => 45,
        37.. => 5 + (i - 37),
        _ => i,
    }) as i16
}

/// `MultiPlayerGameMode.handleContainerInput` for each click on the engine's
/// copy of the menu: the click with the slots it changed and the carried
/// stack after it, and whether a click dropped a stack. Every drop of
/// `AbstractContainerMenu.doClick` (a THROW, a click outside the window, a
/// swap or a result move with no room left) is `Prediction.PREDICTED`.
/// None when the engine refuses a click.
fn predict_clicks(
    m: &mut horizon_client::menu::Menu,
    env: &horizon_client::menu::Env,
    clicks: &[(i16, i32, Input)],
) -> Option<(Vec<Predicted>, bool)> {
    let mut predicted = Vec::with_capacity(clicks.len());
    let mut dropped = false;
    for &(slot, button, input) in clicks {
        let before = menu_items(m);
        let thrown = m.dropped.len();
        m.click(env, i32::from(slot), button, input).ok()?;
        dropped |= m.dropped.len() > thrown;
        let changed = changed_slots(&before, m);
        predicted.push((slot, button, input, changed, m.carried.clone()));
    }
    Some((predicted, dropped))
}

fn menu_items(m: &horizon_client::menu::Menu) -> Vec<Option<ItemStack>> {
    (0..m.slot_count()).map(|i| m.slot(i).cloned()).collect()
}

/// The slots whose stack is no longer `before`'s (`ItemStack.matches`).
fn changed_slots(
    before: &[Option<ItemStack>],
    m: &horizon_client::menu::Menu,
) -> Vec<(i16, Option<ItemStack>)> {
    (0..m.slot_count())
        .filter(|&i| m.slot(i) != before.get(i).and_then(Option::as_ref))
        .map(|i| (i as i16, m.slot(i).cloned()))
        .collect()
}

/// ServerboundContainerClickPacket's `ContainerInput` ordinal.
pub(super) fn input_ordinal(input: Input) -> i32 {
    match input {
        Input::Pickup => 0,
        Input::QuickMove => 1,
        Input::Swap => 2,
        Input::Clone => 3,
        Input::Throw => 4,
        Input::QuickCraft => 5,
        Input::PickupAll => 6,
    }
}

/// The client's world for a guess at what a click does: reads go to the
/// world, writes stay here and are dropped.
struct DryLevel<'w> {
    base: &'w dyn horizon_world::level::LevelAccess,
    changed: HashMap<horizon_core::position::BlockPos, BlockStateId>,
    random: horizon_core::random::LegacyRandom,
    /// The sounds the use plays (a door's creak).
    sounds: Vec<DrySound>,
}

type DrySound = (
    horizon_core::position::BlockPos,
    std::sync::Arc<str>,
    f32,
    f32,
);
/// The blocks a use on the dry level set.
type DryChanges = Vec<(horizon_core::position::BlockPos, BlockStateId)>;
/// What a use on the dry level changed, and the sounds it played.
type DryUse = (DryChanges, Vec<DrySound>);
/// Where `BlockItem.place` on the dry level placed, and the blocks it set.
type DryPlace = (horizon_core::position::BlockPos, DryChanges);

/// The blocks a guest's hand flips that its use predicts (and hears at once): a wooden or
/// copper door, trapdoor or fence gate, a lever or a button. An iron door or trapdoor has
/// no hand use.
pub(super) fn hand_operated(name: &str) -> bool {
    let name = name.trim_start_matches("minecraft:");
    ((name.ends_with("_door") || name.ends_with("_trapdoor")) && !name.starts_with("iron_"))
        || name.ends_with("_fence_gate")
        || name.ends_with("_button")
        || name == "lever"
}

impl<'w> DryLevel<'w> {
    fn new(base: &'w dyn horizon_world::level::LevelAccess) -> Self {
        Self {
            base,
            changed: HashMap::new(),
            random: horizon_core::random::LegacyRandom::new(0),
            sounds: Vec::new(),
        }
    }
}

impl horizon_world::level::LevelAccess for DryLevel<'_> {
    fn emit(&mut self, event: horizon_world::level::LevelEvent) {
        if let horizon_world::level::LevelEvent::Sound {
            pos,
            sound,
            volume,
            pitch,
        } = event
        {
            self.sounds.push((pos, sound, volume, pitch));
        }
    }
    fn block_state(&self, pos: horizon_core::position::BlockPos) -> BlockStateId {
        self.changed
            .get(&pos)
            .copied()
            .unwrap_or_else(|| self.base.block_state(pos))
    }
    fn set_raw(
        &mut self,
        pos: horizon_core::position::BlockPos,
        state: BlockStateId,
    ) -> Option<BlockStateId> {
        let previous = self.block_state(pos);
        self.changed.insert(pos, state);
        Some(previous)
    }
    fn is_loaded(&self, pos: horizon_core::position::BlockPos) -> bool {
        self.base.is_loaded(pos)
    }
    fn min_y(&self) -> i32 {
        self.base.min_y()
    }
    fn max_y(&self) -> i32 {
        self.base.max_y()
    }
    fn block_light(&self, pos: horizon_core::position::BlockPos) -> i32 {
        self.base.block_light(pos)
    }
    fn sky_light(&self, pos: horizon_core::position::BlockPos) -> i32 {
        self.base.sky_light(pos)
    }
    fn sky_darken(&self) -> i32 {
        self.base.sky_darken()
    }
    fn is_unobstructed(&self, pos: horizon_core::position::BlockPos, state: BlockStateId) -> bool {
        self.base.is_unobstructed(pos, state)
    }
    fn random(&mut self) -> &mut horizon_core::random::LegacyRandom {
        &mut self.random
    }
    /// A mod's bone meal answer is read, never grown here: the preview asks
    /// the level it stands in for, as `isValidBonemealTarget` is read on both
    /// sides.
    fn is_valid_bonemeal_target(
        &self,
        pos: horizon_core::position::BlockPos,
        state: BlockStateId,
    ) -> Option<bool> {
        self.base.is_valid_bonemeal_target(pos, state)
    }
    fn game_time(&self) -> i64 {
        self.base.game_time()
    }
}

/// `InteractionHand` on the wire.
fn hand_id(hand: Hand) -> i32 {
    match hand {
        Hand::Main => 0,
        Hand::Off => 1,
    }
}

/// Whether `Item.use` of `info` ends `Minecraft.startUseItem`'s click, so the
/// off hand gets no turn. `FireworkRocketItem.use` answers SUCCESS while the
/// player glides (the rocket boosts them) and PASS otherwise.
fn use_ends_click(info: &horizon_client::items::ItemInfo, fall_flying: bool) -> bool {
    item_has_use(info) || (fall_flying && &*info.id == "minecraft:firework_rocket")
}

/// Whether `Item.use` usually acts on this item (eats, drinks, equips, raises,
/// throws, shoots, fills or places), which ends `Minecraft.startUseItem` at
/// its hand. Tools and materials pass, and the off hand gets its turn. The
/// server decides; this is the client's guess of whether to go on.
fn item_has_use(info: &horizon_client::items::ItemInfo) -> bool {
    let id = info.id.trim_start_matches("minecraft:");
    id.ends_with("_spear")
        || info.consumable.is_some()
        || info.equip_slot.is_some()
        || info.blocks_attacks.is_some()
        || horizon_client::items::bucket_contents(&info.id).is_some()
        || id.ends_with("_boat")
        || id.ends_with("_raft")
        || id.ends_with("_bundle")
        || matches!(
            id,
            "bow"
                | "crossbow"
                | "trident"
                | "spyglass"
                | "fishing_rod"
                | "snowball"
                | "egg"
                | "blue_egg"
                | "brown_egg"
                | "ender_pearl"
                | "ender_eye"
                | "wind_charge"
                | "splash_potion"
                | "lingering_potion"
                | "experience_bottle"
                | "bucket"
                | "glass_bottle"
                | "map"
                | "writable_book"
                | "written_book"
                | "goat_horn"
                | "bundle"
                | "brush"
                | "carrot_on_a_stick"
                | "warped_fungus_on_a_stick"
        )
}

/// The view distance a client asks for: its render distance, which Horizon
/// lets go past vanilla's 32 (a vanilla server still caps it at its own).
fn client_view_distance(render_distance: i32) -> i8 {
    render_distance.clamp(2, i32::from(i8::MAX)) as i8
}

/// `LocalPlayer.onSyncedDataUpdated(DATA_LIVING_ENTITY_FLAGS)`: the server's
/// flags (1 using, 2 the off hand) start a use the client is not running
/// (`startUsingItem`, which needs an item in that hand), and stop one it is
/// (`stopUsingItem`). `using` is the slot in use and its ticks.
fn synced_local_use(
    using: Option<(usize, u32)>,
    flags: i8,
    selected: usize,
    has_item: impl Fn(usize) -> bool,
) -> Option<(usize, u32)> {
    let server_using = flags & 1 != 0;
    match using {
        None if server_using => {
            let slot = if flags & 2 != 0 {
                super::shield::OFFHAND_SLOT
            } else {
                selected
            };
            has_item(slot).then_some((slot, 0))
        }
        Some(_) if !server_using => None,
        using => using,
    }
}

struct RemoteInputWorld<'g, 'a>(&'g mut Game<'a>);

struct RemoteUseWorld<'g, 'a>(&'g mut Game<'a>);

impl horizon_client::interaction_input::TargetUseWorld for RemoteUseWorld<'_, '_> {
    fn use_entity(&mut self) -> bool {
        self.0.remote_use_entity(Hand::Main)
    }
    fn use_block(&mut self) -> bool {
        self.0.remote_use_block(Hand::Main)
    }
    fn use_item(&mut self) {
        self.0.remote_use_hands();
    }
}

impl horizon_client::interaction_input::InteractionWorld for RemoteInputWorld<'_, '_> {
    fn pick_hit_result(&mut self) {
        self.0.tick_hit = Some(self.0.pick());
    }
    fn release_hit_result(&mut self) {
        self.0.tick_hit = None;
    }
    fn using_item(&self) -> bool {
        self.0.using.is_some()
    }
    fn release_using(&mut self) {
        self.0.release_remote_use();
    }
    fn start_attack(&mut self) -> bool {
        self.0.remote_start_attack()
    }
    fn start_use(&mut self) {
        self.0.remote_use();
    }
    fn pick(&mut self) {
        // Minecraft.pickBlockOrEntity: an entity before the block is asked
        // for (`handlePickItemFromEntity`), else the block.
        if let Some(entity) = self.0.pick_entity_hit() {
            if let Some(id) = entity {
                self.0.send(S::PickItemFromEntity(play::PickItemFromEntity {
                    id,
                    include_data: self.0.has_control_down(),
                }));
            }
        } else if let Some(hit) = self.0.pick() {
            self.0.send(S::PickItemFromBlock(play::PickItemFromBlock {
                pos: hit.pos,
                include_data: self.0.has_control_down(),
            }));
        }
    }
    fn use_delay(&self) -> u32 {
        self.0.use_delay
    }
    fn continue_attack(&mut self, down: bool) {
        self.0.remote_continue_attack(down);
    }
}

impl Remote {
    /// `PlayerInfo.getMessageValidator().updateAndValidate(message)` for `message.sender`.
    fn validate_chat(
        &mut self,
        message: &secure_chat::Incoming,
        now: i64,
    ) -> Option<secure_chat::Validated> {
        let sender = message.sender;
        if let Some(validator) = self.chat_validators.get_mut(&sender) {
            return validator.update_and_validate(message, now);
        }
        let (mut validator, settled) = self.new_chat_validator(sender);
        let validated = validator.update_and_validate(message, now);
        // While the services' keys are still on their way the validator is a stopgap, not kept.
        if settled {
            self.chat_validators.insert(sender, validator);
        }
        validated
    }
    /// `ClientPacketListener.initializeChatSession` for `sender`'s entry: a session the services'
    /// keys vouch for makes a keyed validator, anything else the fallback of
    /// `enforcesSecureChat()` (`canValidateProfileKeys() && serverEnforcesSecureChat`). `false`
    /// while the keys are not known yet.
    fn new_chat_validator(&self, sender: u128) -> (secure_chat::MessageValidator, bool) {
        let keys = self.services_keys.lock().unwrap_or_else(|e| e.into_inner());
        let (can_validate, settled) = match &*keys {
            ServicesKeys::Ready(_) => (true, true),
            ServicesKeys::Missing => (false, true),
            ServicesKeys::Pending => (false, false),
        };
        let fallback =
            secure_chat::MessageValidator::fallback(can_validate && self.enforces_secure_chat);
        let (ServicesKeys::Ready(keys), Some(Some((id, expires_at, key, signature)))) =
            (&*keys, self.chat_sessions.get(&sender))
        else {
            if matches!(&*keys, ServicesKeys::Missing)
                && self.chat_sessions.get(&sender).is_some_and(Option::is_some)
            {
                log::warn!(
                    "Ignoring chat session from {sender:032x} due to missing Services public key"
                );
            }
            return (fallback, settled);
        };
        let profile_key = secure_chat::ProfileKey {
            expires_at: *expires_at,
            key: key.clone(),
            signature: signature.clone(),
        };
        match secure_chat::ChatSession::validate(*id, profile_key, keys, sender) {
            Ok(session) => (secure_chat::MessageValidator::keyed(session), true),
            Err(why) => {
                log::error!("Failed to validate profile key for player {sender:032x}: {why}");
                (fallback, true)
            }
        }
    }
    /// A player entity's profile name: the one it was created with, else its
    /// player-list entry's (an entity the list named after it spawned).
    fn level_player_name(&self, id: i32, uuid: &u128) -> Option<&str> {
        self.level_player_names
            .get(&id)
            .or_else(|| self.players.get(uuid))
            .map(String::as_str)
    }
    /// Players in the server's player list, this one included.
    pub(super) fn player_count(&self) -> usize {
        self.players.len()
    }
    /// The local player's profile UUID.
    pub(super) fn profile(&self) -> u128 {
        self.profile
    }
    /// The entity id of a player in the level, by profile UUID.
    pub(super) fn player_entity(&self, uuid: u128) -> Option<u32> {
        self.level_players
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| *id as u32)
    }
    /// The kind and last sent position of a tracked server entity.
    pub(super) fn tracked(&self, id: i32) -> Option<(&str, [f64; 3])> {
        self.entities.get(&id).map(|t| (&*t.kind, t.position))
    }
}

/// The textures to resolve skins from: every list entry's, and for a player
/// entity whose entry is gone the profile it kept. `kept` first takes, for each
/// player entity without one, its entry's textures (`AbstractClientPlayer.getPlayerInfo`
/// looks the PlayerInfo up once and holds it, removed from the list or not).
fn skin_entries(
    level_players: &HashMap<i32, u128>,
    listed: &HashMap<u128, horizon_client::skins::Textures>,
    kept: &mut HashMap<i32, horizon_client::skins::Textures>,
) -> Vec<(u128, horizon_client::skins::Textures)> {
    for (id, uuid) in level_players {
        if !kept.contains_key(id)
            && let Some(textures) = listed.get(uuid)
        {
            kept.insert(*id, textures.clone());
        }
    }
    let mut entries: Vec<_> = listed.iter().map(|(u, t)| (*u, t.clone())).collect();
    for (id, textures) in kept.iter() {
        if let Some(uuid) = level_players.get(id)
            && !listed.contains_key(uuid)
            && !entries.iter().any(|(u, _)| u == uuid)
        {
            entries.push((*uuid, textures.clone()));
        }
    }
    entries
}

/// `AttributeInstance.calculateValue` before the attribute's range: the base,
/// `ADD_VALUE`s, then `ADD_MULTIPLIED_BASE`s of that, then each
/// `ADD_MULTIPLIED_TOTAL`.
fn snapshot_value(s: &play::AttributeSnapshot) -> f64 {
    let amounts = |op: i32| {
        s.modifiers
            .iter()
            .filter(move |m| m.operation == op)
            .map(|m| m.amount)
    };
    let base = s.base + amounts(0).sum::<f64>();
    let mut value = base + amounts(1).map(|a| base * a).sum::<f64>();
    for amount in amounts(2) {
        value *= 1.0 + amount;
    }
    value
}

/// `Util.getMillis` for the level load tracker's timeouts.
fn client_millis() -> u64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    ORIGIN
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::synced_local_use;

    #[test]
    fn the_server_stopping_the_use_lowers_the_shield() {
        // Player.disableShield's stopUsingItem: flags 0 or 2 while the
        // off-hand shield is raised (LocalPlayer.onSyncedDataUpdated →
        // stopUsingItem). stopUsingItem clears bit 0 and leaves bit 1, so a
        // main-hand stop is 0 and an off-hand stop is 2.
        let off = super::super::shield::OFFHAND_SLOT;
        assert_eq!(synced_local_use(Some((off, 12)), 0, 3, |_| true), None);
        assert_eq!(synced_local_use(Some((3, 5)), 0, 3, |_| true), None);
        assert_eq!(synced_local_use(Some((off, 12)), 2, 3, |_| true), None);
    }

    #[test]
    fn the_server_starting_a_use_starts_it_in_that_hand() {
        let off = super::super::shield::OFFHAND_SLOT;
        assert_eq!(synced_local_use(None, 1, 4, |_| true), Some((4, 0)));
        assert_eq!(synced_local_use(None, 3, 4, |_| true), Some((off, 0)));
        // LivingEntity.startUsingItem needs an item in the hand.
        assert_eq!(synced_local_use(None, 1, 4, |_| false), None);
    }

    /// tooBigToInterpolate: a position sync measures from where the entity
    /// is, a teleport from where its interpolation is headed.
    #[test]
    fn a_far_move_snaps_by_the_packets_own_measure() {
        use super::{MovePacket, Path, PosRot, SteppedInterpolation};
        let at = |x: f64| PosRot {
            position: [x, 0.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
        };
        let mut handler = SteppedInterpolation::new(3);
        // At x 0, headed for x 50.
        handler.interpolate_to(at(0.0), Some(Path::Linear([50.0, 0.0, 0.0])), None);
        let to = [100.0, 0.0, 0.0];
        assert!(MovePacket::PositionSync.snaps(&handler, at(0.0), to));
        assert!(!MovePacket::Teleport.snaps(&handler, at(0.0), to));
        assert!(!MovePacket::Move.snaps(&handler, at(0.0), to));
        // With nothing in flight, both measure from the entity.
        let idle = SteppedInterpolation::new(3);
        assert!(MovePacket::Teleport.snaps(&idle, at(0.0), to));
        assert!(!MovePacket::Teleport.snaps(&idle, at(0.0), [60.0, 0.0, 0.0]));
    }

    /// A far sync puts the entity there and drops the steps it was walking;
    /// a rotation alone keeps the position it is headed for and turns there
    /// over the update interval.
    #[test]
    fn a_living_entitys_move_snaps_or_steps() {
        use super::{MovePacket, Path, PosRot, SteppedInterpolation, stepped_move};
        let at = |x: f64, yaw: f32| PosRot {
            position: [x, 0.0, 0.0],
            yaw,
            pitch: 0.0,
        };
        let mut handler = SteppedInterpolation::new(3);
        // Walking to x 5 when the far sync comes.
        handler.interpolate_to(at(0.0, 0.0), Some(Path::Linear([5.0, 0.0, 0.0])), None);
        assert!(handler.active());
        let far = [100.0, 0.0, 0.0];
        let snap = stepped_move(
            &mut handler,
            MovePacket::PositionSync,
            at(0.0, 0.0),
            far,
            Some(Path::Linear(far)),
            Some((90.0, 0.0)),
        );
        assert_eq!(snap, Some(at(100.0, 90.0)));
        assert!(!handler.active());
        assert_eq!(handler.interpolate(), None, "no step back toward x 5");
        // A rotation-only move packet.
        let turn = stepped_move(
            &mut handler,
            MovePacket::Move,
            at(100.0, 90.0),
            far,
            None,
            Some((180.0, 0.0)),
        );
        assert_eq!(turn, None);
        let steps: Vec<_> = std::iter::from_fn(|| handler.interpolate()).collect();
        assert_eq!(
            steps.iter().map(|s| s.yaw).collect::<Vec<_>>(),
            [120.0, 150.0, 180.0]
        );
        assert!(steps.iter().all(|s| s.position == far));
    }

    #[test]
    fn matching_flags_leave_a_running_use_alone() {
        assert_eq!(synced_local_use(Some((2, 7)), 1, 2, |_| true), Some((2, 7)));
        assert_eq!(synced_local_use(None, 0, 2, |_| true), None);
    }

    /// `ChatType.Bound.decorate` and `decorateNarration`: a private message
    /// shows as "Steve whispers to you: hi" and is narrated "Steve says hi";
    /// a chat type the registry lacks is `minecraft:chat`'s.
    #[test]
    fn chat_is_narrated_with_its_chat_types_narration_decoration() {
        use super::{Decoration, chat_type_decorate, play};
        use horizon_world::nbt::{JavaString, Tag, parse_snbt};
        use serde_json::json;
        let bound = play::ChatTypeBound {
            chat_type: 0,
            name: Tag::String(JavaString::from("Steve")),
            target: None,
        };
        let content = json!("hi");
        let (incoming, _) = parse_snbt(
            r#"{chat:{translation_key:"commands.message.display.incoming",parameters:["sender","content"],style:{color:"gray",italic:1b}},narration:{translation_key:"chat.type.text.narrate",parameters:["sender","content"]}}"#,
        )
        .unwrap();
        let chat = chat_type_decorate(Some(&incoming), Decoration::Chat, &bound, &content);
        assert_eq!(chat["translate"], "commands.message.display.incoming");
        assert_eq!(chat["color"], "gray");
        assert_eq!(chat["with"], json!(["Steve", "hi"]));
        assert_eq!(
            chat_type_decorate(Some(&incoming), Decoration::Narration, &bound, &content),
            json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
        );
        assert_eq!(
            chat_type_decorate(None, Decoration::Narration, &bound, &content),
            json!({"translate": "chat.type.text.narrate", "with": ["Steve", "hi"]})
        );
        assert_eq!(
            chat_type_decorate(None, Decoration::Chat, &bound, &content)["translate"],
            "chat.type.text"
        );
    }

    /// An NPC plugin lists the NPC, spawns it, and takes the entry off the list:
    /// the entity keeps the skin it found; a player spawned before its entry
    /// arrives takes the entry once it comes.
    #[test]
    fn a_player_entity_keeps_the_skin_its_entry_gave() {
        use horizon_client::skins::Textures;
        use std::collections::HashMap;
        let npc = Textures {
            skin: Some("http://textures.minecraft.net/texture/npc".into()),
            signed: true,
            ..Textures::default()
        };
        let (mut listed, mut kept) = (HashMap::from([(7u128, npc.clone())]), HashMap::new());
        let mut players = HashMap::from([(2i32, 7u128), (3, 8)]);
        assert_eq!(
            super::skin_entries(&players, &listed, &mut kept),
            [(7, npc.clone())]
        );
        listed.remove(&7);
        assert_eq!(
            super::skin_entries(&players, &listed, &mut kept),
            [(7, npc.clone())]
        );
        // Player 8's entry comes after its entity.
        let late = Textures {
            skin: Some("http://textures.minecraft.net/texture/late".into()),
            ..npc.clone()
        };
        listed.insert(8, late.clone());
        let mut entries = super::skin_entries(&players, &listed, &mut kept);
        entries.sort_by_key(|(u, _)| *u);
        assert_eq!(entries, [(7, npc), (8, late)]);
        // A removed entity forgets its profile with it.
        players.remove(&2);
        kept.remove(&2);
        listed.clear();
        assert_eq!(super::skin_entries(&players, &listed, &mut kept).len(), 1);
    }

    /// An NPC plugin's scale: a base the plugin sets, or a modifier on 1.
    #[test]
    fn attribute_values_add_then_multiply_as_vanilla() {
        use horizon_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
        let modifier = |amount, operation| AttributeModifier {
            id: "minecraft:proof".into(),
            amount,
            operation,
        };
        let snapshot = |base, modifiers| AttributeSnapshot {
            attribute: 0,
            base,
            modifiers,
        };
        assert_eq!(super::snapshot_value(&snapshot(2.0, vec![])), 2.0);
        assert_eq!(
            super::snapshot_value(&snapshot(1.0, vec![modifier(1.0, 0)])),
            2.0
        );
        // (1 + 1) * (1 + 0.5) from the base, then * (1 + 1) of the total.
        let all = vec![modifier(1.0, 0), modifier(0.5, 1), modifier(1.0, 2)];
        assert_eq!(super::snapshot_value(&snapshot(1.0, all)), 6.0);
    }

    use super::Game;

    /// `Entity.lookAt`: yaw 0 faces +Z, -90 faces +X; pitch -90 looks up.
    #[test]
    fn look_at_turns_towards_the_target() {
        let close = |(yaw, pitch): (f32, f32), want: (f32, f32)| {
            assert!(
                (yaw - want.0).abs() < 1e-3 && (pitch - want.1).abs() < 1e-3,
                "{yaw} {pitch}, want {want:?}"
            );
        };
        let at = [10.0, 64.0, -5.0];
        let off = |d: [f64; 3]| [at[0] + d[0], at[1] + d[1], at[2] + d[2]];
        close(
            super::look_at_rotation(at, off([0.0, 0.0, 3.0])),
            (0.0, 0.0),
        );
        close(
            super::look_at_rotation(at, off([3.0, 0.0, 0.0])),
            (-90.0, 0.0),
        );
        close(
            super::look_at_rotation(at, off([-3.0, 0.0, 0.0])),
            (90.0, 0.0),
        );
        close(
            super::look_at_rotation(at, off([0.0, 0.0, -3.0])),
            (-180.0, 0.0),
        );
        close(
            super::look_at_rotation(at, off([0.0, 2.0, 2.0])),
            (0.0, -45.0),
        );
        close(
            super::look_at_rotation(at, off([0.0, -4.0, 4.0])),
            (0.0, 45.0),
        );
    }

    /// `handleMovePlayer` moves the player's pitch and the model's old pitch
    /// (`xRotO`) alike: a relative change keeps the ease a tick began, each
    /// end clamped on its own, and an absolute one snaps both (#3844).
    #[test]
    fn a_position_packet_moves_both_pitch_ends() {
        use super::position_packet_pitch as pitch;
        // The tick is laying the head down from 30 to 0; the packet turns 10.
        assert_eq!(
            (pitch(30.0, 10.0, true), pitch(0.0, 10.0, true)),
            (40.0, 10.0)
        );
        // Each end clamps on its own.
        assert_eq!(
            (pitch(80.0, 20.0, true), pitch(60.0, 20.0, true)),
            (90.0, 80.0)
        );
        // An absolute pitch ignores both ends.
        assert_eq!(
            (pitch(30.0, -45.0, false), pitch(0.0, -45.0, false)),
            (-45.0, -45.0)
        );
        assert_eq!(pitch(0.0, 120.0, false), 90.0);
    }

    /// The hurt sound of `ClientboundDamageEventPacket` follows its damage
    /// type (`Player.getHurtSound`: the type's effects), with the codec's
    /// `hurt` default for an unknown type (#4149).
    #[test]
    fn the_hurt_sound_follows_the_damage_events_type() {
        use super::hurt_sound_for;
        use horizon_protocol::registry::Dynamic;
        use horizon_world::damage_source::Catalog;
        let types = [
            ("minecraft:in_fire", "burning"),
            ("minecraft:drown", "drowning"),
            ("minecraft:sweet_berry_bush", "poking"),
            ("minecraft:freeze", "freezing"),
            ("minecraft:arrow", "hurt"),
        ];
        // The types the server synchronized, in registry order.
        let mut registries = Dynamic::default();
        registries.add(horizon_protocol::packets::configuration::RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: types
                .iter()
                .map(
                    |(id, _)| horizon_protocol::packets::configuration::RegistryEntry {
                        id: (*id).into(),
                        data: None,
                    },
                )
                .collect(),
        });
        // The damage catalog: one entry per type above, each with its effects.
        let bits = |f: f32| format!("{:08x}", f.to_bits());
        let catalog = Catalog::from_json(
            &serde_json::json!({
                "data_version": i64::from(horizon_world::DATA_VERSION),
                "types": types.iter().map(|(id, effects)| serde_json::json!({
                    "id": id,
                    "message_id": id.trim_start_matches("minecraft:"),
                    "exhaustion": bits(0.1),
                    "scaling": "when_caused_by_living_non_player",
                    "effects": effects,
                    "death_message_type": "default",
                })).collect::<Vec<_>>(),
            }),
            &serde_json::json!({}),
        )
        .unwrap();
        for (id, (name, effects)) in types.iter().enumerate() {
            assert_eq!(
                hurt_sound_for(&registries, &catalog, id as i32),
                horizon_client::health::hurt_sound(effects),
                "{name}"
            );
        }
        // An unknown id falls back to the codec's default effects.
        assert_eq!(
            hurt_sound_for(&registries, &catalog, types.len() as i32),
            "minecraft:entity.player.hurt"
        );
        // A server-defined damage type with inline NBT effects needs no entry in
        // the client's built-in catalog.
        let mut custom = horizon_world::nbt::Compound::new();
        custom.insert(
            "effects".into(),
            horizon_world::nbt::Tag::String("burning".into()),
        );
        registries.add(horizon_protocol::packets::configuration::RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: vec![horizon_protocol::packets::configuration::RegistryEntry {
                id: "custom:lava_splash".into(),
                data: Some(horizon_world::nbt::Tag::Compound(custom)),
            }],
        });
        assert_eq!(
            hurt_sound_for(&registries, &catalog, 0),
            "minecraft:entity.player.hurt_on_fire"
        );
    }
    /// `ClientboundSoundEntityPacket`: the sound is heard where the entity it
    /// names is — the local player included, which is no tracked entity — and
    /// an id this level doesn't have plays nothing at all.
    #[test]
    fn a_sound_entity_is_heard_where_its_entity_is() {
        use std::collections::HashMap;
        let mut tracked = HashMap::new();
        tracked.insert(
            7,
            super::Tracked {
                kind: "minecraft:cow".to_owned(),
                position: [8.5, 64.0, -3.5],
            },
        );
        let player = Some((2, [0.5, 70.0, 0.5]));
        assert_eq!(
            super::entity_position(2, player, &[], &tracked),
            Some([0.5, 70.0, 0.5]),
            "the local player is named by Remote.entity_id"
        );
        assert_eq!(
            super::entity_position(7, player, &[], &tracked),
            Some([8.5, 64.0, -3.5]),
            "another entity the server spawned"
        );
        assert_eq!(
            super::entity_position(9, player, &[], &tracked),
            None,
            "getEntity is null: handleSoundEntityEvent returns"
        );
        assert_eq!(
            super::entity_position(2, None, &[], &tracked),
            None,
            "a player with no position yet"
        );
    }
    /// Q over a slot of the inventory on a server (#3357): the predicted THROW
    /// drops the stack, so the player's hand swings, as `LivingEntity.drop`
    /// does for `Prediction.PREDICTED`; a plain pick up drops nothing.
    #[test]
    fn a_predicted_throw_from_a_slot_swings_the_hand() {
        use horizon_client::{
            banner::Banners,
            brewing::Brewing,
            crafting::Recipes,
            creative::Names,
            enchant::Enchantments,
            items::{ItemStack, Items},
            menu::{Env, Input, Kind, Menu},
            stations::StationRecipes,
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let none = &horizon_world::loot::PackOverlay::NONE;
        let items = Items::catalog(&root, none).unwrap();
        let recipes = Recipes::load(&root, &items, none).unwrap();
        let brewing = Brewing::load(&root, &items, none);
        let banners = Banners::load(&root);
        let station_recipes = StationRecipes::load(&root, &items, none);
        let enchantments = Enchantments::default();
        let names = Names::default();
        let env = Env {
            items: &items,
            recipes: &recipes,
            brewing: &brewing,
            banners: &banners,
            station_recipes: &station_recipes,
            enchantments: &enchantments,
            names: &names,
            trade: None,
            maps: None,
        };
        let mut menu = Menu::new(Kind::Inventory);
        let cobblestone = ItemStack {
            item: "minecraft:cobblestone".into(),
            count: 16,
            damage: 0,
            extra: None,
        };
        // The first hotbar slot of InventoryMenu.
        menu.set_slot(&env, 36, Some(cobblestone));
        let (sent, threw) =
            super::predict_clicks(&mut menu, &env, &[(36, 0, Input::Throw)]).unwrap();
        assert!(threw, "Q over a slot drops one and swings");
        assert_eq!(menu.slot(36).map(|s| s.count), Some(15));
        assert_eq!(sent.len(), 1);
        let (_, threw) = super::predict_clicks(&mut menu, &env, &[(36, 0, Input::Pickup)]).unwrap();
        assert!(!threw, "picking the stack up drops nothing");
        let (_, threw) =
            super::predict_clicks(&mut menu, &env, &[(-999, 1, Input::Pickup)]).unwrap();
        assert!(threw, "one of the carried stack outside the window");
    }

    #[test]
    fn placement_preview_checks_obstruction_without_changing_the_level() {
        use super::DryLevel;
        use horizon_core::{position::BlockPos, random::LegacyRandom};
        use horizon_world::{
            level::{
                Behaviours, LevelAccess, PlaceContext, PlaceOutcome, Ticks, data::load_registry,
            },
            registry::BlockStateId,
        };

        struct Level {
            air: BlockStateId,
            stone: BlockStateId,
            obstructed: bool,
            loaded: bool,
            ceiling: i32,
        }
        impl LevelAccess for Level {
            fn block_state(&self, pos: BlockPos) -> BlockStateId {
                if pos == BlockPos::new(0, 0, 0) {
                    self.stone
                } else {
                    self.air
                }
            }
            fn set_raw(&mut self, _: BlockPos, _: BlockStateId) -> Option<BlockStateId> {
                panic!("preview must not write to the live level");
            }
            fn is_loaded(&self, _: BlockPos) -> bool {
                self.loaded
            }
            fn max_y(&self) -> i32 {
                self.ceiling
            }
            fn is_unobstructed(&self, pos: BlockPos, _: BlockStateId) -> bool {
                assert_eq!(pos, BlockPos::new(0, 1, 0));
                !self.obstructed
            }
            fn random(&mut self) -> &mut LegacyRandom {
                panic!("preview must not advance the live level's random source");
            }
            fn game_time(&self) -> i64 {
                0
            }
        }

        // As with the LAN tests, prepared game resources are optional locally.
        let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !resources.join("block-behaviours.json").exists() {
            return;
        }
        let registry = load_registry(&resources).unwrap();
        let behaviours = Behaviours::load(&registry, &resources).unwrap();
        let mut base = Level {
            air: registry.default_state("minecraft:air").unwrap(),
            stone: registry.default_state("minecraft:stone").unwrap(),
            obstructed: false,
            loaded: true,
            ceiling: 320,
        };
        for item in [
            "minecraft:stone",
            "minecraft:oak_slab",
            "minecraft:oak_door",
            // GrowingPlantHeadBlock.getStateForPlacement draws a random age.
            "minecraft:twisting_vines",
        ] {
            let ctx = PlaceContext {
                clicked_pos: BlockPos::new(0, 0, 0),
                face: horizon_core::geometry::Direction::Up,
                hit: horizon_core::geometry::Vec3::new(0.5, 1.0, 0.5),
                inside: false,
                yaw: 0.0,
                pitch: 45.0,
                sneaking: false,
                item: item.into(),
                can_use_game_master_blocks: false,
            };
            for (obstructed, loaded, ceiling) in [
                (false, true, 320),
                (true, true, 320),
                (false, false, 320),
                (false, true, 1),
            ] {
                base.obstructed = obstructed;
                base.loaded = loaded;
                base.ceiling = ceiling;
                let valid = !obstructed && loaded && ceiling > 1;
                let mut preview = DryLevel::new(&base);
                let outcome = behaviours.place_block(&mut preview, &mut Ticks::default(), &ctx);
                assert_eq!(
                    matches!(outcome, PlaceOutcome::Placed { .. }),
                    valid,
                    "{item}, obstructed={obstructed}, loaded={loaded}, ceiling={ceiling}"
                );
                assert_eq!(preview.changed.is_empty(), !valid);
                assert_eq!(base.block_state(BlockPos::new(0, 1, 0)), base.air);
            }
        }
    }

    #[test]
    fn the_creative_sync_covers_inventory_menu_slots_5_to_45_once_each() {
        // CreativeInventoryListener sends every InventoryMenu slot but the crafting ones: the
        // armour (5..=8), the inventory (9..=44) and the off hand (45).
        let stack = |item: &str| Some(horizon_client::items::ItemStack::new(item, 1));
        let inventory: Vec<_> = (0..36)
            .map(|i| stack(&format!("minecraft:item_{i}")))
            .collect();
        let offhand = stack("minecraft:shield");
        let armor = [
            stack("minecraft:diamond_helmet"),
            stack("minecraft:diamond_chestplate"),
            stack("minecraft:diamond_leggings"),
            stack("minecraft:diamond_boots"),
        ];
        let stacks = super::creative_sync_stacks(&inventory, &offhand, &armor);
        assert_eq!(stacks.len(), super::CREATIVE_SYNCED);
        let mut slots: Vec<i16> = (0..stacks.len()).map(super::creative_sync_slot).collect();
        // Each entry goes to the menu slot that holds that stack.
        for (i, &slot) in slots.iter().enumerate() {
            let game_slot = Game::menu_slot("inventory", i32::from(slot)).unwrap();
            let expected = match game_slot {
                0..36 => &inventory[game_slot],
                504 => &offhand,
                500..504 => &armor[game_slot - 500],
                _ => panic!("entry {i} went to game slot {game_slot}"),
            };
            assert_eq!(&stacks[i], expected, "entry {i}, menu slot {slot}");
        }
        slots.sort();
        assert_eq!(slots, (5..=45).collect::<Vec<i16>>());
    }

    #[test]
    fn a_server_slot_update_leaves_an_unsent_creative_click_to_be_sent() {
        let stack = |item: &str| Some(horizon_client::items::ItemStack::new(item, 1));
        let none = || vec![None; super::CREATIVE_SYNCED];
        // Synced: nothing. A click puts a helmet on (entry 37, not sent yet), then the server
        // sets hotbar slot 0 (a pickup): only that entry counts as synced.
        let mut synced = none();
        let mut before = none();
        before[37] = stack("minecraft:diamond_helmet");
        let mut now = before.clone();
        now[0] = stack("minecraft:stick");
        super::mark_synced(&mut synced, &before, now.clone());
        assert_eq!(synced[0], now[0]);
        assert_eq!(
            synced[37], None,
            "the helmet still differs, so the next tick sends it"
        );
    }

    #[test]
    fn every_inventory_menu_slot_is_a_game_slot_and_45_is_the_off_hand() {
        // InventoryMenu: result, 2x2 grid, armour, 27 + 9 inventory slots, then the off hand.
        assert_eq!(Game::menu_slot("inventory", 45), Some(504));
        let mut seen = std::collections::HashSet::new();
        for slot in 0..46 {
            let internal = Game::menu_slot("inventory", slot);
            assert!(internal.is_some_and(|i| seen.insert(i)), "slot {slot}");
        }
        assert_eq!(Game::menu_slot("inventory", 46), None);
    }

    #[test]
    fn remote_clicks_find_slots_past_the_old_large_chest_cutoff() {
        for (menu, internal, index) in [
            ("minecraft:generic_9x6", 18, 63),
            ("minecraft:generic_9x6", 19, 64),
            ("minecraft:generic_9x6", 0, 81),
            ("minecraft:generic_9x6", 8, 89),
            ("minecraft:generic_9x6", 353, 53),
            ("minecraft:generic_9x5", 8, 80),
            ("minecraft:generic_9x4", 8, 71),
            ("minecraft:generic_9x3", 8, 62),
            ("inventory", 504, 45),
        ] {
            assert_eq!(
                Game::menu_slot_index(menu, internal),
                Some(index),
                "{menu}, game slot {internal}"
            );
        }
        assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 354), None);
        assert_eq!(Game::menu_slot_index("minecraft:generic_9x6", 504), None);
        assert_eq!(Game::menu_slot_index("unknown", 0), None);
    }

    /// `FireworkRocketItem.use` is SUCCESS while gliding: the main hand's
    /// rocket ends the click and the off hand's shield is not raised (#3424).
    /// On foot it passes, so the off hand still gets its turn.
    #[test]
    fn a_rocket_used_while_gliding_ends_the_click() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            eprintln!("skipping: no prepared resources in {}", root.display());
            return;
        }
        let registry = horizon_world::level::data::load_registry(&root).unwrap();
        let facts = horizon_client::blocks::BlockFacts::load(&root, &registry).unwrap();
        let items = horizon_client::items::Items::load(
            &root,
            &registry,
            &facts,
            &horizon_world::loot::PackOverlay::NONE,
        )
        .unwrap();
        let rocket = items.get("minecraft:firework_rocket").unwrap();
        assert!(super::use_ends_click(rocket, true), "gliding");
        assert!(!super::use_ends_click(rocket, false), "on foot");
        // Other items answer the same whether or not the player glides.
        let shield = items.get("minecraft:shield").unwrap();
        let stick = items.get("minecraft:stick").unwrap();
        for gliding in [false, true] {
            assert!(super::use_ends_click(shield, gliding));
            assert!(!super::use_ends_click(stick, gliding));
        }
    }
}
