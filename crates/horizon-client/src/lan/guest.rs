//! A remote player: connection, position, chunk and entity views, and the
//! player state (inventory, health, game mode) that the host's gameplay code
//! operates on when it acts for them.
use super::{Joined, PlayerSender, entity::Tracker};
use crate::{
    items::ItemStack,
    motion::PlayerMotion,
    pose::{Dimensions, Pose},
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_protocol::{
    item::Stack,
    packets::{configuration::ClientInformation, play},
};
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::AtomicI32},
};

/// GameType ids.
pub const SURVIVAL: i32 = 0;
pub const CREATIVE: i32 = 1;
pub const SPECTATOR: i32 = 3;

/// The body state a player's file keeps that lives on `PlayerMotion`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SavedBody {
    pub fire_ticks: i32,
    pub air: i32,
    pub fall_distance: f64,
}

impl SavedBody {
    /// The body state of `motion`, to save.
    pub fn of(motion: &PlayerMotion) -> Self {
        Self {
            fire_ticks: motion.fire_ticks,
            air: motion.air,
            fall_distance: motion.fall_distance,
        }
    }
    /// Put it on a new body.
    pub fn apply(self, motion: &mut PlayerMotion) {
        motion.fire_ticks = self.fire_ticks;
        motion.air = self.air;
        motion.fall_distance = self.fall_distance;
    }
}

/// An effect as last sent in ClientboundUpdateMobEffectPacket.
#[derive(Clone, Debug, PartialEq)]
pub struct SentEffect {
    pub id: &'static str,
    pub amplifier: i32,
    pub flags: i8,
    pub duration: i32,
    pub tick_count: i32,
}

pub struct Guest {
    pub conn: u32,
    /// The client's address (IP bans, `list`).
    pub address: std::net::SocketAddr,
    /// The hub tick of the player's last action (ServerPlayer.lastActionTime),
    /// for player-idle-timeout.
    pub last_action_tick: u64,
    /// The next ClientboundPlayerChatPacket.globalIndex for this connection.
    pub chat_index: i32,
    /// Secure chat (ServerGamePacketListenerImpl): the player's message
    /// chain, keyed once it sends a chat session; the messages its
    /// acknowledgements cover; the signatures its client caches.
    pub chain: horizon_protocol::secure_chat::Chain,
    pub last_seen: horizon_protocol::secure_chat::LastSeenValidator,
    pub signatures: horizon_protocol::secure_chat::SignatureCache,
    /// The hub tick the player was placed on: chunks go out from the tick
    /// after next, behind the first doTick's packets, as vanilla's do.
    pub placed_at: u64,
    /// The chunk cache centre placeNewPlayer already sent.
    pub center_sent: Option<(i32, i32)>,
    /// Why the connection closed (the console's "lost connection").
    pub left_reason: String,
    pub uuid: u128,
    pub name: String,
    /// The profile's properties, for the player list (skins).
    pub properties: Vec<horizon_protocol::packets::login::Property>,
    pub entity_id: i32,
    pub sender: PlayerSender,
    pub latency: Arc<AtomicI32>,
    pub info: ClientInformation,
    pub brand: String,
    /// The chunk radius this client is sent.
    pub view_distance: i32,
    pub position: [f64; 3],
    pub previous_position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub on_ground: bool,
    pub game_mode: i32,
    /// The permission level (0 none, 2 game masters, 4 owners).
    pub permission: u8,
    pub sneaking: bool,
    /// The last `ServerboundPlayerInputPacket` flags (forward 1, backward 2,
    /// left 4, right 8, jump 16, shift 32, sprint 64).
    pub input: u8,
    /// The boat or minecart the player rides (`Entity.getVehicle`).
    pub vehicle: Option<u32>,
    /// `ServerGamePacketListenerImpl.lastGoodX/Y/Z` of the vehicle it controls.
    pub vehicle_good: Option<[f64; 3]>,
    pub sprinting: bool,
    pub fall_flying: bool,
    pub flying: bool,
    pub sleeping: bool,
    /// `LivingEntity.DATA_LIVING_ENTITY_FLAGS`: 1 while using an item, 2 if
    /// it is in the off hand.
    pub living_flags: i8,
    /// The bed (its head) the player sleeps in (`LivingEntity.getSleepingPos`),
    /// `Player.sleepCounter`, and the bed last told to the player itself.
    pub bed: Option<[i32; 3]>,
    pub sleep_timer: u32,
    pub sent_bed: Option<[i32; 3]>,
    /// Where the player respawns (a bed's spot; `ServerPlayer.respawnConfig`),
    /// None for the world spawn.
    pub spawn: Option<[f64; 3]>,
    /// A respawn point outside the Overworld (a respawn anchor, or one
    /// `spawnpoint` set there): its dimension, block, yaw and whether it is
    /// forced, as the player's save keeps it (`respawn`).
    pub respawn_elsewhere: Option<(String, [i32; 3], f32, bool)>,
    /// Pending ClientboundPlayerPositionPacket id; movement is ignored until
    /// the client accepts it (ServerGamePacketListenerImpl.awaitingPositionFromClient).
    pub awaiting_teleport: Option<(i32, [f64; 3])>,
    /// Through a portal, until `LanHub::land` gives the client its position:
    /// the moves it sent from the old level are ignored, as they would be
    /// while it had a teleport to accept.
    pub landing: bool,
    pub teleport_counter: i32,
    /// The hub tick the pending teleport was (re)sent on (awaitingTeleportTime).
    pub teleport_tick: u64,
    /// Where the player was when this tick's packets began (firstGoodX/Y/Z),
    /// for the moved-too-quickly check.
    pub first_good: [f64; 3],
    /// Position moves received in all and by the start of this tick
    /// (receivedMovePacketCount, knownMovePacketCount).
    pub received_moves: i32,
    pub known_moves: i32,
    /// A position move arrived since the client's last ClientTickEnd
    /// (receivedPositionThisTick): a second one is a protocol error.
    pub position_this_tick: bool,
    /// It accepted a move since the last entity range checks
    /// (ServerChunkCache.move), which then check all of its pairs again.
    pub moved: bool,
    /// The client finished loading terrain (ServerboundPlayerLoadedPacket).
    pub loaded: bool,
    // Chunks.
    pub center: Option<(i32, i32)>,
    pub sent_chunks: crate::fast_hash::FxHashSet<(i32, i32)>,
    pub chunk_queue: Vec<(i32, i32)>,
    /// PlayerChunkSender: batches sent but not yet acknowledged, how many
    /// may be (1 until the client's first acknowledgement, then 10), the
    /// rate the client asks for and the chunks the next batch may hold.
    pub unacknowledged: u32,
    pub max_unacknowledged: u32,
    pub chunks_per_tick: f32,
    pub batch_quota: f32,
    // Entities.
    pub tracker: Tracker,
    // Player state the gameplay code reads and writes.
    pub motion: Option<Box<PlayerMotion>>,
    /// What the player's file says of the body (`Entity.load`: the fire
    /// and air ticks and the fall distance), for the first body it gets.
    pub loaded_body: Option<SavedBody>,
    /// The hurt and attack logs of the body it had before it respawned,
    /// until the new body takes them over (the control socket's `hurtlog`
    /// reads across a death).
    pub logs_kept: Option<(
        Vec<crate::health::HurtRecord>,
        Vec<crate::health::AttackRecord>,
    )>,
    /// The 36 inventory slots (hotbar first), armor (head..feet) and off hand.
    pub inventory: Vec<Option<ItemStack>>,
    pub armor: [Option<ItemStack>; 4],
    pub offhand: Option<ItemStack>,
    pub selected: usize,
    pub carried: Option<ItemStack>,
    /// The inventory menu's crafting grid (2x2) and its result
    /// (`InventoryMenu.craftSlots`, `resultSlots`), which the game computes.
    pub craft_grid: [Option<ItemStack>; 4],
    pub craft_result: Option<ItemStack>,
    /// The ender chest inventory (`PlayerEnderChestContainer`, 27 slots).
    pub ender_items: Vec<Option<ItemStack>>,
    pub health: f32,
    /// The saved `AbsorptionAmount`, until the player's first body on the
    /// server takes it (the body holds the live amount).
    pub saved_absorption: Option<f32>,
    pub food: i32,
    pub saturation: f32,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub xp_total: i32,
    /// `Player.enchantmentSeed` (the enchanting table's offers).
    pub xp_seed: i32,
    /// Active status effects (`LivingEntity.activeEffects`).
    pub effects: crate::effects::PlayerEffects,
    /// What the client was told of each effect: id, amplifier, flags, and
    /// the duration and `effects.tick_count` at the time, so a duration that
    /// only counted down is not sent again (`ServerPlayer.onEffectUpdated`).
    pub sent_effects: Vec<SentEffect>,
    /// What the client believes the inventory menu holds (for change detection).
    pub menu_sent: Vec<Option<ItemStack>>,
    pub menu_state: i32,
    /// `lastSentHealth`, `lastSentFood` and `lastFoodSaturationZero`.
    pub sent_health: Option<(f32, i32, bool)>,
    pub sent_xp: Option<(f32, i32, i32)>,
    /// The air supply and absorption the player's client was last sent
    /// (Entity.DATA_AIR_SUPPLY_ID, Player.DATA_PLAYER_ABSORPTION_ID); their
    /// defaults until either changes.
    pub sent_air: (i32, f32),
    /// The shared flags (Entity.DATA_SHARED_FLAGS_ID) the player's client was
    /// last sent about itself.
    pub sent_shared: i8,
    /// Where the player last died (Player.lastDeathLocation), sent with the respawn.
    pub last_death: Option<(String, [i32; 3])>,
    /// The last action sequence to acknowledge at the next tick (-1: none).
    pub ack_up_to: std::cell::Cell<i32>,
    pub sent_selected: usize,
    pub inbox: VecDeque<play::Serverbound>,
    pub joined_tick: u64,
    pub ticks: u64,
    /// The moves accepted since the game last looked (movement, on ground),
    /// for ServerPlayer.doCheckFallDamage.
    pub moves: Vec<([f64; 3], bool)>,
    /// Whether the guest was on the ground before the first of `moves`.
    pub moves_from_ground: bool,
    /// `ServerPlayer.lastKnownClientMovement`: the last move accepted from
    /// their client, zero after a client tick without one.
    pub known_movement: [f64; 3],
    pub dead: bool,
    /// `LivingEntity.deathTime`: ticks since the guest died.
    pub death_time: u32,
    /// The join packets were sent (PlayerList.placeNewPlayer ran).
    pub placed: bool,
    /// The level the player is in (`ServerPlayer.level().dimension()`): the
    /// hub's level passes only reach the players in theirs.
    pub dimension: String,
    /// The open block container, if any.
    pub menu: Option<super::menu::Menu>,
    pub next_window: i32,
    /// An unfinished drag in the inventory menu.
    pub drag: Option<(u8, Vec<usize>)>,
    /// The compound this player was loaded from, for fields Horizon does not model.
    pub saved: Option<horizon_world::nbt::Tag>,
    /// `ServerPlayer.seenCredits`: the end poem has been shown.
    pub seen_credits: bool,
    /// `ServerPlayer.recipeBook`.
    pub recipe_book: crate::recipe_book::Settings,
    /// `PlayerAdvancements`: the criteria obtained, whether they came from
    /// the player's file, the advancements the client was last told it may
    /// see (empty before the first packet), and those whose progress
    /// changed since (`progressChanged`).
    pub advancements: crate::advancements::Progress,
    pub advancements_loaded: bool,
    pub advancements_visible: Vec<bool>,
    pub advancements_changed: std::collections::BTreeSet<usize>,
    /// The server's scoreboard as this player's client has it (lan/scoreboard.rs).
    pub scoreboard: super::scoreboard::View,
    /// Recipes unlocked since the client was last told
    /// (`ServerRecipeBook.addRecipes` sends them).
    pub recipes_unlocked: Vec<String>,
}

impl Guest {
    pub fn new(j: Joined, entity_id: i32, position: [f64; 3], view_distance: i32) -> Self {
        Self {
            conn: j.conn,
            address: j.address,
            last_action_tick: 0,
            chat_index: 0,
            chain: Default::default(),
            last_seen: Default::default(),
            signatures: Default::default(),
            placed_at: 0,
            center_sent: None,
            left_reason: String::new(),
            uuid: j.uuid,
            name: j.name,
            properties: j.properties,
            entity_id,
            sender: j.sender,
            latency: j.latency,
            view_distance: (i32::from(j.info.view_distance)).clamp(2, view_distance),
            info: j.info,
            brand: j.brand,
            position,
            previous_position: position,
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            on_ground: false,
            game_mode: SURVIVAL,
            permission: 0,
            sneaking: false,
            input: 0,
            vehicle: None,
            vehicle_good: None,
            sprinting: false,
            fall_flying: false,
            flying: false,
            sleeping: false,
            living_flags: 0,
            bed: None,
            sleep_timer: 0,
            sent_bed: None,
            spawn: None,
            respawn_elsewhere: None,
            awaiting_teleport: None,
            landing: false,
            teleport_counter: 0,
            teleport_tick: 0,
            first_good: position,
            received_moves: 0,
            known_moves: 0,
            position_this_tick: false,
            moved: false,
            loaded: false,
            center: None,
            sent_chunks: Default::default(),
            chunk_queue: Vec::new(),
            unacknowledged: 0,
            max_unacknowledged: 1,
            chunks_per_tick: 9.0,
            batch_quota: 0.0,
            tracker: Tracker::default(),
            motion: None,
            loaded_body: None,
            logs_kept: None,
            inventory: vec![None; 36],
            armor: [None, None, None, None],
            offhand: None,
            selected: 0,
            carried: None,
            craft_grid: Default::default(),
            craft_result: None,
            ender_items: vec![None; 27],
            health: 20.0,
            saved_absorption: None,
            food: 20,
            saturation: 5.0,
            xp_level: 0,
            xp_progress: 0.0,
            xp_total: 0,
            xp_seed: 0,
            effects: Default::default(),
            sent_effects: Vec::new(),
            menu_sent: vec![None; 46],
            menu_state: 0,
            sent_health: None,
            sent_xp: None,
            sent_air: (300, 0.0),
            sent_shared: 0,
            last_death: None,
            ack_up_to: std::cell::Cell::new(-1),
            sent_selected: 0,
            inbox: VecDeque::new(),
            joined_tick: 0,
            ticks: 0,
            moves: Vec::new(),
            moves_from_ground: false,
            known_movement: [0.0; 3],
            dead: false,
            death_time: 0,
            placed: false,
            dimension: "minecraft:overworld".into(),
            menu: None,
            next_window: 0,
            drag: None,
            saved: None,
            seen_credits: false,
            recipe_book: Default::default(),
            advancements: Default::default(),
            advancements_loaded: false,
            advancements_visible: Vec::new(),
            advancements_changed: Default::default(),
            scoreboard: Default::default(),
            recipes_unlocked: Vec::new(),
        }
    }
    pub fn send(&self, packet: &play::Clientbound) {
        self.sender.send(packet.encode());
    }
    /// `Entity.stopRiding`: out of the vehicle it rode.
    pub fn stop_riding(&mut self) {
        (self.vehicle, self.vehicle_good) = (None, None);
    }
    /// `LivingEntity.tickDeath` for a dead player: true once its body is
    /// removed, 20 ticks after it died (`remove(KILLED)`, whose
    /// `Entity.setRemoved` stops its riding). The body sits in its vehicle
    /// until then.
    pub fn tick_death(&mut self) -> bool {
        self.death_time += 1;
        let removed = self.death_time >= 20;
        if removed {
            self.stop_riding();
        }
        removed
    }
    pub fn send_all(&self, packets: &[play::Clientbound]) {
        for p in packets {
            self.send(p);
        }
    }
    pub fn chunk(&self) -> (i32, i32) {
        (
            (self.position[0].floor() as i32) >> 4,
            (self.position[2].floor() as i32) >> 4,
        )
    }
    pub fn creative(&self) -> bool {
        self.game_mode == CREATIVE
    }
    pub fn spectator(&self) -> bool {
        self.game_mode == SPECTATOR
    }
    /// PlayerAbilities flags (invulnerable, flying, may fly, instabuild).
    pub fn ability_flags(&self) -> i8 {
        let mut f = 0;
        if self.creative() || self.spectator() {
            f |= 1;
        }
        if self.flying || self.spectator() {
            f |= 2;
        }
        if self.creative() || self.spectator() {
            f |= 4;
        }
        if self.creative() {
            f |= 8;
        }
        f
    }
    /// `Player.getDesiredPose`: sleeping, gliding, then the body's own pose
    /// (swimming, crawling, crouching) once its server tick has run.
    pub fn pose(&self) -> Pose {
        if self.sleeping {
            Pose::Sleeping
        } else if self.fall_flying {
            Pose::FallFlying
        } else if let Some(m) = &self.motion {
            m.pose
        } else if self.sneaking && !self.flying {
            Pose::Crouching
        } else {
            Pose::Standing
        }
    }
    /// `Avatar.POSES` for the guest's pose, with its scale attribute.
    pub fn dimensions(&self) -> Dimensions {
        let pose = self.pose();
        match &self.motion {
            Some(m) if m.pose == pose => m.dimensions,
            _ => pose.dimensions(1.0),
        }
    }
    /// `Entity.getBoundingBox`.
    pub fn bounds(&self) -> Aabb {
        let [x, y, z] = self.position;
        let d = self.dimensions();
        let half = f64::from(d.width) / 2.0;
        Aabb::new(
            Vec3::new(x - half, y, z - half),
            Vec3::new(x + half, y + f64::from(d.height), z + half),
        )
    }
    pub fn eye_height(&self) -> f64 {
        f64::from(self.dimensions().eye_height)
    }
    /// `Player.isWithinBlockInteractionRange(pos, buffer)` for the guest:
    /// its block interaction range is 4.5, 5 in creative.
    pub fn within_block_range(&self, pos: [i32; 3], buffer: f64) -> bool {
        let eye = [
            self.position[0],
            self.position[1] + self.eye_height(),
            self.position[2],
        ];
        let range = if self.creative() { 5.0 } else { 4.5 };
        within_block_range(eye, range + buffer, pos)
    }
    /// `Player.isWithinBlockInteractionRange(pos, 1.0)`, which the server
    /// checks a block action against (ServerPlayerGameMode.handleBlockBreakAction,
    /// ServerGamePacketListenerImpl.handleUseItemOn): from the eyes of the
    /// guest's body in its pose (crouching, swimming, gliding).
    pub fn may_reach_block(&self, pos: [i32; 3]) -> bool {
        let eye = self
            .motion
            .as_ref()
            .map_or_else(|| self.eye_height(), |m| f64::from(m.dimensions.eye_height));
        let range = if self.creative() { 5.0 } else { 4.5 };
        within_block_range(
            [self.position[0], self.position[1] + eye, self.position[2]],
            range + 1.0,
            pos,
        )
    }
    /// The inventory menu's 46 slots (ServerPlayer.inventoryMenu): 0 crafting
    /// result, 1..=4 crafting grid, 5..=8 armor, 9..=35 main, 36..=44 hotbar,
    /// 45 off hand.
    pub fn menu_slot(&self, slot: usize) -> Option<ItemStack> {
        match slot {
            0 => self.craft_result.clone(),
            1..=4 => self.craft_grid[slot - 1].clone(),
            5..=8 => self.armor[slot - 5].clone(),
            9..=35 => self.inventory.get(slot).cloned().flatten(),
            36..=44 => self.inventory.get(slot - 36).cloned().flatten(),
            45 => self.offhand.clone(),
            _ => None,
        }
    }
    pub fn set_menu_slot(&mut self, slot: usize, item: Option<ItemStack>) {
        match slot {
            0 => self.craft_result = item,
            1..=4 => self.craft_grid[slot - 1] = item,
            5..=8 => self.armor[slot - 5] = item,
            9..=35 => self.inventory[slot] = item,
            36..=44 => self.inventory[slot - 36] = item,
            45 => self.offhand = item,
            _ => {}
        }
    }
}

/// `FireworkExplosion.STREAM_CODEC`'s fields (`Shape` by its BY_ID order).
fn network_explosion(
    e: &crate::particles::FireworkExplosion,
) -> horizon_protocol::item::FireworkExplosion {
    horizon_protocol::item::FireworkExplosion {
        shape: e.shape as i32,
        colors: e.colors.iter().map(|c| *c as i32).collect(),
        fade: e.fade.iter().map(|c| *c as i32).collect(),
        trail: e.trail,
        twinkle: e.twinkle,
    }
}

/// A rocket's `minecraft:fireworks` and a star's `minecraft:firework_explosion`.
pub fn firework_components(
    d: &crate::item_data::StackData,
) -> Vec<(i32, horizon_protocol::item::Component)> {
    use horizon_protocol::item::{Component, component};
    let mut out = Vec::new();
    if let Some((flight, explosions)) = &d.fireworks {
        out.push((
            component::FIREWORKS,
            Component::Fireworks {
                flight: i32::from(*flight),
                explosions: explosions.iter().map(network_explosion).collect(),
            },
        ));
    }
    if let Some(e) = &d.firework_explosion {
        out.push((
            component::FIREWORK_EXPLOSION,
            Component::FireworkExplosion(network_explosion(e)),
        ));
    }
    out
}

/// `DyeColor.getId` of a dye's name.
fn dye_id(name: &str) -> Option<i32> {
    crate::block_entity_render::DYES
        .iter()
        .position(|d| d.0 == name)
        .map(|i| i as i32)
}

/// The off hand slot is conventionally 40 in saves and 45 in the menu.
/// `synced_id` names an entry of a synced registry (enchantments, trim
/// materials and patterns) by its id on this connection.
pub fn stack_to_network(
    item_id: impl Fn(&str) -> i32,
    synced_id: impl Fn(&str, &str) -> Option<i32>,
    s: &Option<ItemStack>,
) -> Option<Stack> {
    network_stack(&item_id, &synced_id, s)
}
fn network_stack(
    item_id: &dyn Fn(&str) -> i32,
    synced_id: &dyn Fn(&str, &str) -> Option<i32>,
    s: &Option<ItemStack>,
) -> Option<Stack> {
    let enchantment_id = |name: &str| synced_id("minecraft:enchantment", name);
    let s = s.as_ref()?;
    let mut stack = Stack::new(item_id(&s.item), s.count as i32);
    if s.damage > 0 {
        stack.added.push((
            horizon_protocol::item::component::DAMAGE,
            horizon_protocol::item::Component::Int(s.damage as i32),
        ));
    }
    let enchantments: Vec<(i32, i32)> = s
        .enchantments()
        .iter()
        .filter_map(|(e, level)| Some((enchantment_id(e)?, *level as i32)))
        .collect();
    if !enchantments.is_empty() {
        stack.added.push((
            horizon_protocol::item::component::ENCHANTMENTS,
            horizon_protocol::item::Component::Enchantments(enchantments),
        ));
    }
    let stored: Vec<(i32, i32)> = s
        .stored_enchantments()
        .iter()
        .filter_map(|(e, level)| Some((enchantment_id(e)?, *level as i32)))
        .collect();
    if !stored.is_empty() {
        stack.added.push((
            horizon_protocol::item::component::STORED_ENCHANTMENTS,
            horizon_protocol::item::Component::Enchantments(stored),
        ));
    }
    if let Some(d) = s.data() {
        if let Some(rgb) = d.dyed_color {
            stack.added.push((
                horizon_protocol::item::component::DYED_COLOR,
                horizon_protocol::item::Component::Int(rgb),
            ));
        }
        // ArmorTrim: both holders as references into the synced registries.
        if let Some((material, pattern)) = &d.trim {
            let id = |registry: &str, name: &str| {
                if name.contains(':') {
                    synced_id(registry, name)
                } else {
                    synced_id(registry, &format!("minecraft:{name}"))
                }
            };
            if let (Some(material), Some(pattern)) = (
                id("minecraft:trim_material", material),
                id("minecraft:trim_pattern", pattern),
            ) {
                stack.added.push((
                    horizon_protocol::item::component::TRIM,
                    horizon_protocol::item::Component::Trim { material, pattern },
                ));
            }
        }
        // BannerPatternLayers: each pattern a reference into the synced registry;
        // a layer that has none leaves the stack without the component.
        let layers: Option<Vec<(i32, i32)>> = d
            .banner_patterns
            .iter()
            .map(|layer| {
                let name = if layer.pattern.contains(':') {
                    layer.pattern.to_string()
                } else {
                    format!("minecraft:{}", layer.pattern)
                };
                Some((
                    synced_id("minecraft:banner_pattern", &name)?,
                    dye_id(&layer.color)?,
                ))
            })
            .collect();
        if let Some(layers) = layers.filter(|layers| !layers.is_empty()) {
            stack.added.push((
                horizon_protocol::item::component::BANNER_PATTERNS,
                horizon_protocol::item::Component::BannerPatterns(layers),
            ));
        }
        // `painting/variant`: a reference into the synced registry.
        if let Some(id) = crate::save::text(d.other("minecraft:painting/variant")).and_then(|v| {
            synced_id(
                "minecraft:painting_variant",
                &horizon_world::datafix::dynamic::namespaced(&v),
            )
        }) {
            stack.added.push((
                horizon_protocol::item::component::PAINTING_VARIANT,
                horizon_protocol::item::Component::PaintingVariant(id),
            ));
        }
        // A shield's `base_color`.
        if let Some(id) = d.base_color.as_deref().and_then(dye_id) {
            stack.added.push((
                horizon_protocol::item::component::BASE_COLOR,
                horizon_protocol::item::Component::Int(id),
            ));
        }
        if let Some(id) = d.map_id {
            stack.added.push((
                horizon_protocol::item::component::MAP_ID,
                horizon_protocol::item::Component::Int(id),
            ));
        }
        if d.repair_cost > 0 {
            stack.added.push((
                horizon_protocol::item::component::REPAIR_COST,
                horizon_protocol::item::Component::Int(d.repair_cost as i32),
            ));
        }
        if let Some(mode) = d.map_post_processing {
            stack.added.push((
                horizon_protocol::item::component::MAP_POST_PROCESSING,
                horizon_protocol::item::Component::Int(i32::from(mode)),
            ));
        }
        if let Some(amplifier) = d.ominous_amplifier {
            stack.added.push((
                horizon_protocol::item::component::OMINOUS_BOTTLE_AMPLIFIER,
                horizon_protocol::item::Component::Int(i32::from(amplifier)),
            ));
        }
        stack.added.extend(firework_components(d));
        // A goat horn's instrument, by its `minecraft:instrument` id.
        if let Some(id) = d
            .other("minecraft:instrument")
            .and_then(|t| crate::save::text(Some(t)))
            .and_then(|name| {
                synced_id(
                    "minecraft:instrument",
                    &horizon_world::datafix::dynamic::namespaced(&name),
                )
            })
        {
            stack.added.push((
                horizon_protocol::item::component::INSTRUMENT,
                horizon_protocol::item::Component::Instrument(id),
            ));
        }
        // The compass's lodestone target.
        if let Some(t) = d.other("minecraft:lodestone_tracker") {
            let target = t.get("target").and_then(|g| {
                let Some(horizon_world::nbt::Tag::IntArray(pos)) = g.get("pos") else {
                    return None;
                };
                let dimension = crate::save::text(g.get("dimension"))?;
                Some((dimension, [*pos.first()?, *pos.get(1)?, *pos.get(2)?]))
            });
            let tracked = crate::save::number(t.get("tracked")).is_none_or(|v| v != 0);
            stack.added.push((
                horizon_protocol::item::component::LODESTONE_TRACKER,
                horizon_protocol::item::Component::LodestoneTracker { target, tracked },
            ));
        }
        // PotionContents: the potion by its `minecraft:potion` id.
        if let Some(id) = (d.potion.as_deref()).and_then(|potion| {
            synced_id(
                "minecraft:potion",
                &horizon_world::datafix::dynamic::namespaced(potion),
            )
        }) {
            stack.added.push((
                horizon_protocol::item::component::POTION_CONTENTS,
                horizon_protocol::item::Component::Potion {
                    potion: Some(id),
                    color: None,
                    rest: Vec::new(),
                },
            ));
        }
        // SuspiciousStewEffects: each effect by its `minecraft:mob_effect` id.
        if !d.stew_effects.is_empty() {
            let effects = d
                .stew_effects
                .iter()
                .filter_map(|(effect, duration)| {
                    let name = horizon_world::datafix::dynamic::namespaced(effect);
                    Some((synced_id("minecraft:mob_effect", &name)?, *duration))
                })
                .collect();
            stack.added.push((
                horizon_protocol::item::component::SUSPICIOUS_STEW_EFFECTS,
                horizon_protocol::item::Component::StewEffects(effects),
            ));
        }
        // A loaded crossbow's ammunition.
        if !d.charged.is_empty() {
            let charged = d
                .charged
                .iter()
                .filter_map(|c| network_stack(item_id, synced_id, &Some(c.clone())))
                .collect();
            stack.added.push((
                horizon_protocol::item::component::CHARGED_PROJECTILES,
                horizon_protocol::item::Component::Stacks(charged),
            ));
        }
        // What a bundle holds, for its tooltip (ClientBundleTooltip).
        if !d.bundle_contents.is_empty() {
            let contents = d
                .bundle_contents
                .iter()
                .filter_map(|c| network_stack(item_id, synced_id, &Some(c.clone())))
                .collect();
            stack.added.push((
                horizon_protocol::item::component::BUNDLE_CONTENTS,
                horizon_protocol::item::Component::Stacks(contents),
            ));
        }
        // The books a lectern shows and a guest reads.
        if !d.writable_pages.is_empty() {
            stack.added.push((
                horizon_protocol::item::component::WRITABLE_BOOK_CONTENT,
                horizon_protocol::item::Component::WritableBook(d.writable_pages.clone()),
            ));
        }
        if let Some(w) = &d.written {
            stack.added.push((
                horizon_protocol::item::component::WRITTEN_BOOK_CONTENT,
                horizon_protocol::item::Component::WrittenBook(
                    horizon_protocol::item::WrittenBook {
                        title: w.title.clone(),
                        author: w.author.clone(),
                        generation: i32::from(w.generation),
                        pages: if w.page_tags.len() == w.pages.len() {
                            w.page_tags.clone()
                        } else {
                            w.pages
                                .iter()
                                .map(|p| horizon_protocol::text::literal(p))
                                .collect()
                        },
                        resolved: true,
                    },
                ),
            ));
        }
    }
    // Keep the component's styles across integrated-server and LAN inventory sync.
    if let Some(name) = s.data().and_then(|d| d.other("minecraft:custom_name")) {
        stack.added.push((
            horizon_protocol::item::component::CUSTOM_NAME,
            horizon_protocol::item::Component::Text(name.clone()),
        ));
    }
    // The component as loaded, so a translatable name stays one.
    if let Some(name) = s.data().and_then(|d| d.other("minecraft:item_name")) {
        stack.added.push((
            horizon_protocol::item::component::ITEM_NAME,
            horizon_protocol::item::Component::Text(name.clone()),
        ));
    }
    Some(stack)
}

/// `Player.isWithinBlockInteractionRange`: `eye` closer than `range` to the
/// box of the block at `pos` (`AABB.distanceToSqr`).
pub fn within_block_range(eye: [f64; 3], range: f64, pos: [i32; 3]) -> bool {
    (0..3)
        .map(|k| {
            let lo = f64::from(pos[k]);
            (lo - eye[k]).max(eye[k] - lo - 1.0).max(0.0).powi(2)
        })
        .sum::<f64>()
        < range * range
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::buf::{Codec, Reader, Writer};
    use horizon_protocol::item::{Component, component};
    use horizon_world::nbt::Tag;

    #[test]
    fn styled_custom_name_survives_inventory_sync() {
        let name = Tag::compound([
            ("text", Tag::String("Red".into())),
            ("color", Tag::String("#ff0000".into())),
            ("italic", Tag::Byte(0)),
        ]);
        let mut item = ItemStack::new("minecraft:stick", 1);
        item.data_mut().custom_name = Some(name.clone());
        let sent = stack_to_network(|_| 7, |_, _| None, &Some(item));
        let mut writer = Writer::new();
        horizon_protocol::item::Slot::write(&sent, &mut writer);
        let mut reader = Reader::new(&writer.data);
        let received = horizon_protocol::item::Slot::read(&mut reader)
            .unwrap()
            .unwrap();
        assert_eq!(reader.remaining(), 0);
        assert_eq!(
            received.component(component::CUSTOM_NAME),
            Some(&Component::Text(name))
        );
    }

    #[test]
    fn potions_go_as_potion_registry_ids() {
        let synced_id = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:potion", "minecraft:swiftness") => Some(14),
            _ => None,
        };
        let potion = ItemStack::new("minecraft:splash_potion", 1).with_potion("swiftness");
        let stack = stack_to_network(|_| 7, synced_id, &Some(potion)).unwrap();
        assert_eq!(
            stack.component(component::POTION_CONTENTS),
            Some(&Component::Potion {
                potion: Some(14),
                color: None,
                rest: Vec::new(),
            })
        );
    }

    #[test]
    fn trims_go_as_synced_registry_references() {
        let mut chestplate = ItemStack::new("minecraft:diamond_chestplate", 1);
        chestplate.data_mut().trim = Some(("minecraft:gold".into(), "coast".into()));
        let synced_id = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:trim_material", "minecraft:gold") => Some(4),
            ("minecraft:trim_pattern", "minecraft:coast") => Some(1),
            _ => None,
        };
        let stack = stack_to_network(|_| 7, synced_id, &Some(chestplate.clone())).unwrap();
        assert_eq!(
            stack.component(component::TRIM),
            Some(&Component::Trim {
                material: 4,
                pattern: 1
            })
        );
        // A name the connection's registry lacks sends no trim.
        chestplate.data_mut().trim = Some(("minecraft:x".into(), "coast".into()));
        let stack = stack_to_network(|_| 7, synced_id, &Some(chestplate)).unwrap();
        assert_eq!(stack.component(component::TRIM), None);
    }

    #[test]
    fn goat_horns_send_their_instrument_by_registry_id() {
        let mut horn = ItemStack::new("minecraft:goat_horn", 1);
        horn.data_mut().other.push((
            "minecraft:instrument".into(),
            horizon_world::nbt::Tag::String("minecraft:sing_goat_horn".into()),
        ));
        let synced_id = |registry: &str, name: &str| match (registry, name) {
            ("minecraft:instrument", "minecraft:sing_goat_horn") => Some(1),
            _ => None,
        };
        let stack = stack_to_network(|_| 7, synced_id, &Some(horn)).unwrap();
        assert_eq!(
            stack.component(component::INSTRUMENT),
            Some(&Component::Instrument(1))
        );
    }

    fn guest_at(position: [f64; 3]) -> Guest {
        let (tx, _rx) = std::sync::mpsc::channel();
        Guest::new(
            Joined {
                conn: 0,
                uuid: 0,
                name: "guest".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(std::sync::atomic::AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
            },
            1,
            position,
            10,
        )
    }

    #[test]
    fn a_dead_player_leaves_its_vehicle_when_its_body_is_removed() {
        let mut g = guest_at([0.5, -120.0, 0.5]);
        g.vehicle = Some(7);
        g.vehicle_good = Some([0.5, -120.0, 0.5]);
        g.dead = true;
        // LivingEntity.tickDeath: the body sits in the boat for 19 ticks...
        for tick in 1..20 {
            assert!(!g.tick_death(), "tick {tick}");
            assert_eq!(g.vehicle, Some(7), "tick {tick}");
        }
        // ...then remove(KILLED)'s setRemoved stops its riding, so the
        // respawned player is not put back into the boat.
        assert!(g.tick_death());
        assert_eq!((g.vehicle, g.vehicle_good), (None, None));
    }

    #[test]
    fn a_block_action_must_be_within_reach() {
        let mut g = guest_at([0.5, 64.0, 0.5]);
        // Survival (4.5 + 1 from the eyes): five blocks off horizontally,
        // not one further.
        assert!(g.may_reach_block([5, 64, 0]));
        assert!(!g.may_reach_block([6, 64, 0]));
        // Creative's 5.0 reaches one block further.
        g.game_mode = CREATIVE;
        assert!(g.may_reach_block([6, 64, 0]));
        assert!(!g.may_reach_block([7, 64, 0]));
        // With no body yet the sneak flag gives the eye height: a block at
        // the edge above is gone once crouched.
        g.game_mode = SURVIVAL;
        assert!(g.may_reach_block([0, 71, 0]));
        g.sneaking = true;
        assert!(!g.may_reach_block([0, 71, 0]));
    }
}
