//! Hosting the open world on the local network ("Open to LAN"): starts the
//! server threads and the play-state hub, feeds it the world's entities each
//! tick and turns what the guests do into gameplay. A world that is not open
//! costs one `Option` check per tick.
use super::{Game, shield::OFFHAND_SLOT, shoulders::shoulder_variants};
use horizon_client::{
    Result,
    items::ItemStack,
    lan::{
        self, Lan, MAX_PLAYERS, NetConfig,
        chunks::ChunkEncoder,
        data::SyncData,
        entity::{EntityState, States},
        hub::{BodyFlags, HostEvent, HostState, Hub, HubConfig, TickCtx},
        playerdata,
    },
    mobs::{Mob, Species, Who, leash::Holder},
    text::Text,
    world::World,
};
use horizon_core::geometry::Vec3;
use horizon_protocol::{
    crypto::offline_uuid,
    entity_data::{Value, serializer},
    packets::play::{self, Clientbound as C},
};
use horizon_world::block_collision::BlockCollisionRegistry;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// What `sync_data` loaded: whether it was with data packs over the game's
/// data, and the registries and tags.
type Synced = (bool, std::result::Result<Arc<SyncData>, String>);

/// The registries and tags, loaded once per world (a world with packs open
/// or a `/reload` forgets them, as `forget_sync_data` does).
static SYNC: Mutex<Option<Synced>> = Mutex::new(None);

/// The registries and tags a server sends, loaded from `root` the first
/// time, with the world's data packs' recipes, advancements and dialogs over
/// them. What was read with packs is read again for a world without them
/// (their dialogs would stay); a world with packs forgets what was read
/// before it at open (`finish_open_world`).
pub(super) fn sync_data(
    root: &std::path::Path,
    overlay: &horizon_world::loot::PackOverlay,
) -> std::result::Result<Arc<SyncData>, String> {
    let packs = !overlay.files.is_empty();
    let mut sync = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    if sync
        .as_ref()
        .is_some_and(|(with_packs, _)| *with_packs != packs)
    {
        *sync = None;
    }
    sync.get_or_insert_with(|| (packs, SyncData::load(root, overlay).map(Arc::new)))
        .1
        .clone()
}

/// Let go of what `sync_data` loaded: a preload no world used
/// (game/world_loading.rs), what was read before a world with packs opened,
/// or before a `/reload` without a hub.
pub(super) fn forget_sync_data() {
    *SYNC.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// `/reload` (MinecraftServer.reloadResources) under a running hub: the
/// tags, recipes and advancements read again with the world's packs, the
/// synchronized registries `kept` (the hub's) as they were, and that becomes
/// what `sync_data` gives.
pub(super) fn reload_sync_data(
    root: &std::path::Path,
    overlay: &horizon_world::loot::PackOverlay,
    kept: &SyncData,
) -> std::result::Result<Arc<SyncData>, String> {
    // Kept pack dialogs count as packs: a world opened without packs after
    // a `/reload` that dropped them must read its own registries again.
    let packs = !overlay.files.is_empty() || kept.has_pack_entries();
    let sync = SyncData::load(root, overlay).map(|mut sync| {
        sync.keep_registries(root, kept);
        Arc::new(sync)
    });
    *SYNC.lock().unwrap_or_else(|e| e.into_inner()) = Some((packs, sync.clone()));
    sync
}

impl Game<'_> {
    /// The commands' `minecraft:dialog` ids as the clients get them in the
    /// configuration phase (`sync`, the world's packs' dialogs among them):
    /// `/dialog show` parses an id against these and sends its index.
    pub(super) fn sync_dialog_ids(&mut self, sync: &SyncData) {
        let Some(names) = sync.dynamic.registries.get("minecraft:dialog") else {
            return;
        };
        let ids = names.iter().map(|(name, _)| name.clone()).collect();
        self.ui
            .registries
            .by_registry
            .entry("minecraft:dialog".to_owned())
            .or_default()
            .ids = ids;
    }
}

/// The registries and tags a server sends without data packs over them, read
/// on a helper thread while a server's world prepares: most worlds have no
/// packs, so it is what `sync_data` would load once the world is open.
pub(super) struct SyncPreload(std::thread::JoinHandle<std::result::Result<SyncData, String>>);

/// Start reading the server's data beside the world's preparation; `None`
/// when it is read already, or being read (`preload_world_data`).
pub(super) fn preload_sync_data(root: &std::path::Path) -> Option<SyncPreload> {
    if !SYNC.try_lock().is_ok_and(|sync| sync.is_none()) {
        return None;
    }
    let root = root.to_owned();
    std::thread::Builder::new()
        .name("Server data preload".into())
        .spawn(move || SyncData::load(&root, &horizon_world::loot::PackOverlay::NONE))
        .map(SyncPreload)
        .ok()
}

impl SyncPreload {
    /// The world is open: without packs of its own (`overlay`), what was read
    /// becomes `sync_data`'s; with them, `sync_data` loads them as before.
    pub(super) fn adopt(self, overlay: &horizon_world::loot::PackOverlay) {
        if !overlay.files.is_empty() {
            return;
        }
        if let Ok(read) = self.0.join() {
            let mut sync = SYNC.lock().unwrap_or_else(|e| e.into_inner());
            if sync.as_ref().is_none_or(|(with_packs, _)| *with_packs) {
                *sync = Some((false, read.map(Arc::new)));
            }
        }
    }
}

/// The settings of the open world (WorldOptionsScreen's multiplayer section).
#[derive(Clone, Copy, Debug)]
pub struct LanSettings {
    pub port: u16,
    pub guest_commands: bool,
    pub force_game_mode: bool,
}

pub(super) struct LanHost {
    pub hub: Hub,
    /// The mods heard that everyone still here leaves with the server.
    pub players_told_left: bool,
    pub settings: LanSettings,
    pub host_entity: i32,
    /// The host's swings the guests were told of (`Game::swings_started`).
    pub host_swings: u32,
    /// The host's crack the guests were told of: the block and its stage.
    pub host_crack: Option<([i32; 3], i32)>,
    pub host_uuid: u128,
    /// The host's profile name (the account's, or the offline name).
    pub host_name: String,
    /// The guests' models in the host's world.
    pub models: HashMap<u32, Mob>,
    host_mode: i32,
    pub play: HashMap<u32, super::lan_actions::GuestPlay>,
    /// Blocks broken this tick (their sound is a level event for the guests).
    pub broken: Vec<[f64; 3]>,
    /// Hurt time, death and swing time (while swinging) of the mobs guests
    /// see; the players' hurt time by entity id.
    mob_state: HashMap<super::Dim, HashMap<u32, (i32, bool, Option<i32>)>>,
    /// The guest each mob sees as its player, when not the host (game/lan_mobs.rs).
    /// Looked up per mob each tick: a fast hasher (never iterated).
    pub mob_players: horizon_client::fast_hash::FxHashMap<u32, u32>,
    /// An integrated server's owner (game/integrated.rs).
    pub owner: Option<Owner>,
    /// The players' locator bars, one waypoint manager per level by
    /// dimension id (game/lan_waypoints.rs), and the guests who are dead,
    /// whose respawn replaces their waypoint.
    pub waypoints: std::collections::HashMap<String, lan::waypoints::Waypoints>,
    pub waypoint_dead: std::collections::HashSet<u128>,
    /// The world border as the guests were last told of it (lan/border.rs).
    pub border_sent: Option<lan::border::Sent>,
    /// Guests' commands waiting for the hub to be back in `self.lan`, where
    /// a command can reach every player (game/lan_actions.rs).
    pub commands: Vec<super::lan_actions::GuestCommand>,
    /// Mods' quiet commands waiting to run a few a tick (game/lan_quiet.rs).
    pub quiet: std::collections::VecDeque<super::lan_quiet::QuietBatch>,
    /// Far terrain sampled for Horizon clients (game/lan_far.rs); None when
    /// `horizon-lod-distance` is 0.
    pub far: Option<horizon_client::lod_net::Service>,
    /// Native mods' commands and the level each needs, once asked
    /// (game/native_server.rs).
    pub native_commands: Option<HashMap<String, u8>>,
    /// The players a native mod leaves out of the sleep count when they
    /// aren't in bed (`native_sleep_ignored`).
    pub sleep_ignored: std::collections::HashSet<u128>,
    /// The entities the guests track, described again every tick
    /// (`lan_entities`) over last tick's states.
    pub(super) states: lan::entity::States,
}

impl LanHost {
    /// `PlayerList.remove`: `player.awardStat(Stats.LEAVE_GAME)` before the
    /// player is saved. Once per guest, however often they are saved.
    pub(super) fn award_leave_game(&mut self, conn: u32) {
        if let Some(play) = self.play.get_mut(&conn)
            && !play.left_game
        {
            play.left_game = true;
            play.stats.custom("leave_game", 1);
        }
    }

    /// `PlayerList.removeAll`: every player who joined leaves with the world.
    pub(super) fn award_leave_game_to_all(&mut self) {
        let conns: Vec<u32> = self
            .hub
            .guests
            .iter()
            .filter(|g| g.placed)
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            self.award_leave_game(conn);
        }
    }
}

/// The player who owns an integrated server's world: a guest whose state is
/// the world's own player (`players/data/<singleplayer_uuid>.dat`), as in
/// single player.
pub(super) struct Owner {
    pub uuid: u128,
    /// The world's player as loaded, until the owner joins.
    pub saved: Option<Box<horizon_client::save::Level>>,
}

/// A stable UUID for an entity that has none.
pub(super) fn entity_uuid(id: u32) -> u128 {
    0x4856_5a4e_0000_0000_0000_0000_0000_0000u128 | u128::from(id)
}

/// The SynchedEntityData index of the baby flag: AgeableMob.DATA_BABY_ID and
/// Zombie.DATA_BABY_ID (Zoglin's too) are the first entry after Mob's (16);
/// Piglin's comes after AbstractPiglin's zombification flag (17). Every
/// AgeableMob has one, the happy ghast and the nautilus included (#6133).
pub(super) fn baby_index(species: Species) -> Option<u8> {
    use Species as S;
    match species {
        S::Piglin => Some(17),
        s if horizon_client::mobs::finalize::ageable(s)
            || horizon_client::mobs::doors::is_zombie(s)
            || s == S::Zoglin =>
        {
            Some(16)
        }
        _ => None,
    }
}

/// The variant and look entries of `mob_meta`'s mobs (registry holders, the
/// fish's, the bee's, the copper golem's weathering...), in a match of their
/// own: `mob_meta`'s arms hold these mobs' other entries.
fn mob_variant_meta(
    m: &Mob,
    hub: &Hub,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    let (kind, e) = (m.kind.id.as_str(), &m.extra);
    let id = |registry: &str, name: &str| hub.registry_id(registry, &format!("minecraft:{name}"));
    match kind {
        // Axolotl.DATA_VARIANT (after AgeableMob's two).
        "minecraft:axolotl" => {
            if e.variant != 0 {
                meta.push((18, serializer::INT, Value::Int(i32::from(e.variant))));
            }
        }
        // Frog.DATA_VARIANT_ID, Pig's, Cow's, Chicken's and ZombieNautilus's
        // (after the entries of their parents): registry holders.
        "minecraft:frog"
        | "minecraft:pig"
        | "minecraft:cow"
        | "minecraft:chicken"
        | "minecraft:zombie_nautilus" => {
            let (index, serializer, registry) = match kind {
                "minecraft:frog" => (18, serializer::FROG_VARIANT, "minecraft:frog_variant"),
                "minecraft:pig" => (19, serializer::PIG_VARIANT, "minecraft:pig_variant"),
                "minecraft:cow" => (18, serializer::COW_VARIANT, "minecraft:cow_variant"),
                "minecraft:chicken" => {
                    (18, serializer::CHICKEN_VARIANT, "minecraft:chicken_variant")
                }
                _ => (
                    21,
                    serializer::ZOMBIE_NAUTILUS_VARIANT,
                    "minecraft:zombie_nautilus_variant",
                ),
            };
            // Each registry's default (`define(DATA_VARIANT_ID, ... TEMPERATE)`)
            // stays out of the pairing data, as other default values do.
            let name = horizon_client::mobs::finalize::variant_name(m.species(), e.variant);
            let name = name.trim_start_matches("minecraft:");
            if let Some(v) = id(registry, name)
                && name != "temperate"
            {
                meta.push((index, serializer, Value::Id(v)));
            }
            if let Some(v) = id(registry, "temperate") {
                defaults.push((index, serializer, Value::Id(v)));
            }
        }
        // Rabbit.DATA_TYPE_ID (after AgeableMob's two) and
        // Parrot.DATA_VARIANT_ID (after TamableAnimal's two and its flags and
        // owner). The fox's type goes with its flags (#2538).
        "minecraft:rabbit" | "minecraft:parrot" => {
            let index = if kind == "minecraft:parrot" { 20 } else { 18 };
            if e.variant != 0 {
                meta.push((index, serializer::INT, Value::Int(i32::from(e.variant))));
            }
        }
        // Bee.DATA_FLAGS_ID (rolling 2, stung 4, nectar 8) and
        // DATA_ANGER_END_TIME: the texture shows the nectar and the anger.
        "minecraft:bee" => {
            let bee = horizon_client::mobs::bee::get(m);
            let flags = bee.map_or(0, |b| b.flags);
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags as i8)));
            }
            defaults.push((18, serializer::BYTE, Value::Byte(0)));
            let anger_end = bee.map_or(-1, |b| b.anger_end);
            if anger_end != -1 {
                meta.push((19, serializer::LONG, Value::Long(anger_end)));
            }
            defaults.push((19, serializer::LONG, Value::Long(-1)));
        }
        // AbstractFish.FROM_BUCKET takes 16: Salmon.DATA_TYPE (small 0,
        // medium 1, large 2; medium is the data's default, so always sent),
        // Pufferfish.PUFF_STATE and TropicalFish.DATA_ID_TYPE_VARIANT: the
        // pattern's packed id (size, then index << 8), base colour << 16 and
        // pattern colour << 24.
        "minecraft:salmon" => {
            meta.push((17, serializer::INT, Value::Int(i32::from(e.variant.min(2)))));
        }
        "minecraft:pufferfish" => {
            if e.size > 0 {
                meta.push((17, serializer::INT, Value::Int(e.size)));
            }
            defaults.push((17, serializer::INT, Value::Int(0)));
        }
        "minecraft:tropical_fish" => {
            let pattern = e.variant.min(11);
            let packed = i32::from(pattern >= 6)
                | i32::from(pattern % 6) << 8
                | i32::from(e.variant2 & 15) << 16
                | i32::from(e.variant3 & 15) << 24;
            if packed != 0 {
                meta.push((17, serializer::INT, Value::Int(packed)));
            }
        }
        // Tadpole.AGE_LOCKED (after AbstractFish's FROM_BUCKET).
        "minecraft:tadpole" => {
            if e.age_locked {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
        }
        // CopperGolem.DATA_WEATHER_STATE: unaffected 0 to oxidized 3, and it
        // goes back down when the golem is scraped.
        "minecraft:copper_golem" => {
            if e.variant != 0 {
                meta.push((
                    16,
                    serializer::WEATHERING_COPPER_STATE,
                    Value::Id(i32::from(e.variant.min(3))),
                ));
            }
            defaults.push((16, serializer::WEATHERING_COPPER_STATE, Value::Id(0)));
        }
        // Ghast.DATA_IS_CHARGING: the shooting texture.
        "minecraft:ghast" => {
            if e.flag {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
        }
        // WitherBoss.DATA_ID_INV (after the three head targets): the
        // invulnerable ticks, the blinking texture and the growing scale.
        "minecraft:wither" => {
            if e.counter > 0 {
                meta.push((19, serializer::INT, Value::Int(e.counter)));
            }
            defaults.push((19, serializer::INT, Value::Int(0)));
        }
        _ => {}
    }
}

/// A mob's synched data as ServerEntity sends it (only the values that differ
/// from the defaults the client constructs), with vanilla's entry indices:
/// Entity 0..=7, LivingEntity 8..=14, Mob 15, then the subclasses.
/// `defaults` gets the defaults of the entries that can go back to theirs.
fn mob_meta(
    m: &Mob,
    hub: &Hub,
    player_entity: Option<i32>,
    game_time: i64,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    let kind = m.kind.id.as_str();
    // Entity.DATA_SHARED_FLAGS_ID: on fire 0x01, sprinting 0x08 (a dashing
    // cat or ocelot), swimming 0x10 (a drowned under water,
    // `Drowned.setSwimming`), invisible 0x20 (the effect's flag) and glowing
    // 0x40 (LivingEntity.updateGlowingStatus).
    let shared = i8::from(m.is_on_fire())
        | i8::from(m.extra.sprinting) << 3
        | i8::from(m.species() == Species::Drowned && m.extra.flag) << 4
        | i8::from(m.invisible || horizon_client::mobs::trader::is_invisible(m)) << 5
        | i8::from(m.is_currently_glowing()) << 6;
    if shared != 0 {
        meta.push((0, serializer::BYTE, Value::Byte(shared)));
    }
    // Entity.DATA_POSE: the pose the species keeps in its own fields (a
    // pose back at standing goes out by the entity's own default).
    let pose = horizon_client::mobs::sync::pose_id(m);
    if pose != 0 {
        meta.push((6, serializer::POSE, Value::Id(i32::from(pose))));
    }
    // LivingEntity.DATA_SLEEPING_POS_ID: the bed a sleeping villager lies in.
    if let Some(bed) = horizon_client::mobs::villager::sleeping_pos(m) {
        meta.push((14, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(Some(bed))));
    }
    if let Some(name) = &m.custom_name {
        meta.push((
            2,
            serializer::OPTIONAL_COMPONENT,
            Value::OptText(Some(horizon_protocol::text::literal(name))),
        ));
        if m.name_visible {
            meta.push((3, serializer::BOOLEAN, Value::Bool(true)));
        }
    }
    // LivingEntity.DATA_LIVING_ENTITY_FLAGS: using an item 1, in the off
    // hand 2 (a skeleton draws its bow, a witch raises her potion).
    let using = i8::from(m.extra.using_item > 0) | i8::from(m.extra.using_off_hand) << 1;
    if using != 0 {
        meta.push((8, serializer::BYTE, Value::Byte(using)));
    }
    defaults.push((8, serializer::BYTE, Value::Byte(0)));
    // LivingEntity.DATA_HEALTH_ID: defined as 1, then set to the max health.
    let health = m.health.max(0.0);
    if health != 1.0 {
        meta.push((9, serializer::FLOAT, Value::Float(health)));
    }
    defaults.push((9, serializer::FLOAT, Value::Float(1.0)));
    // LivingEntity.DATA_EFFECT_PARTICLES and DATA_EFFECT_AMBIENCE_ID: the
    // swirls of its effects (the ones a mob has are visible, not ambient).
    meta.extend(
        hub.effect_meta(
            (m.extra.effects.iter())
                .filter(|e| !m.extra.hidden_effects.contains(&e.0))
                .map(|e| (e.0, m.extra.ambient_effects.contains(&e.0))),
        ),
    );
    defaults.push((10, serializer::PARTICLES, Value::Particles(Vec::new())));
    defaults.push((11, serializer::BOOLEAN, Value::Bool(false)));
    // Mob.DATA_MOB_FLAGS_ID: no AI 1, left-handed 2, aggressive 4 (what
    // draws a vindicator's axe, a skeleton's drawn bow, a piglin's raised
    // weapon).
    let flags =
        i8::from(m.no_ai) | i8::from(m.extra.left_handed) << 1 | i8::from(m.ai.aggressive) << 2;
    if flags != 0 {
        meta.push((15, serializer::BYTE, Value::Byte(flags)));
    }
    defaults.push((15, serializer::BYTE, Value::Byte(0)));
    // AgeableMob.DATA_BABY_ID (and Zombie's, Zoglin's, Piglin's).
    if let Some(index) = baby_index(m.species()) {
        if m.is_baby() {
            meta.push((index, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((index, serializer::BOOLEAN, Value::Bool(false)));
        // AgeableMob.AGE_LOCKED, the entry after it (not Zombie's or Piglin's).
        if index == 16 && horizon_client::mobs::finalize::ageable(m.species()) {
            if m.extra.age_locked {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
        }
    }
    let e = &m.extra;
    let id = |registry: &str, name: &str| hub.registry_id(registry, &format!("minecraft:{name}"));
    mob_variant_meta(m, hub, meta, defaults);
    match kind {
        "minecraft:guardian" | "minecraft:elder_guardian" => {
            // Guardian.DATA_ID_MOVING and DATA_ID_ATTACK_TARGET.
            if e.flag2 {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            let target = if e.flag {
                m.ai.target.and_then(|who| match who {
                    Who::Player => player_entity,
                    Who::Mob(id) => Some(id as i32),
                })
            } else {
                None
            };
            if let Some(target) = target {
                meta.push((17, serializer::INT, Value::Int(target)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
            defaults.push((17, serializer::INT, Value::Int(0)));
        }
        // Bat.DATA_ID_FLAGS: resting in bit 0, flying by default.
        "minecraft:bat" => {
            if e.flag {
                meta.push((16, serializer::BYTE, Value::Byte(1)));
            }
            defaults.push((16, serializer::BYTE, Value::Byte(0)));
        }
        // SnowGolem.DATA_PUMPKIN_ID: 16 while the pumpkin is on (the default).
        "minecraft:snow_golem" => {
            if e.sheared {
                meta.push((16, serializer::BYTE, Value::Byte(0)));
            }
            defaults.push((16, serializer::BYTE, Value::Byte(16)));
        }
        // MushroomCow.DATA_TYPE (after AgeableMob's two): 0 red, 1 brown.
        "minecraft:mooshroom" => {
            if e.variant == 1 {
                meta.push((18, serializer::INT, Value::Int(1)));
            }
            defaults.push((18, serializer::INT, Value::Int(0)));
        }
        // AbstractPiglin.DATA_IMMUNE_TO_ZOMBIFICATION (16: with the dimension it
        // makes it shake) and Piglin.DATA_IS_DANCING (after the baby flag and
        // the crossbow charge).
        "minecraft:piglin" => {
            if e.flag2 {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
            if e.scratch[0] > 0 {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Sheep.DATA_WOOL_ID: the colour in the low nibble, 16 when sheared.
        "minecraft:sheep" => {
            let value = (e.color & 15) | if e.sheared { 16 } else { 0 };
            if value != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(value as i8)));
            }
            defaults.push((18, serializer::BYTE, Value::Byte(0)));
        }
        // Fox.DATA_TYPE_ID and DATA_FLAGS_ID (after AgeableMob's two; sleeping 32, ...).
        "minecraft:fox" => {
            if e.variant != 0 {
                meta.push((18, serializer::INT, Value::Int(i32::from(e.variant))));
            }
            defaults.push((18, serializer::INT, Value::Int(0)));
            let flags = (e.ints[0] & 0xff) as u8 as i8;
            if flags != 0 {
                meta.push((19, serializer::BYTE, Value::Byte(flags)));
            }
            defaults.push((19, serializer::BYTE, Value::Byte(0)));
        }
        "minecraft:parrot" => {
            // TamableAnimal.DATA_FLAGS_ID: sitting 1, tame 4.
            let flags = i8::from(e.sitting) | if e.tame { 4 } else { 0 };
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags)));
            }
        }
        "minecraft:cat" | "minecraft:wolf" => {
            // TamableAnimal.DATA_FLAGS_ID: sitting 1, tame 4.
            let flags = i8::from(e.sitting) | if e.tame { 4 } else { 0 };
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags)));
            }
            let (index, registry, name) = if kind == "minecraft:cat" {
                let v = horizon_client::mobs::tame::CAT_VARIANTS[(e.variant as usize).min(10)];
                (20, "minecraft:cat_variant", v)
            } else {
                let v = horizon_client::mobs::tame::WOLF_VARIANTS[(e.variant as usize).min(8)].0;
                (23, "minecraft:wolf_variant", v)
            };
            let serializer = if index == 20 {
                serializer::CAT_VARIANT
            } else {
                serializer::WOLF_VARIANT
            };
            if let Some(v) = id(registry, name) {
                meta.push((index, serializer, Value::Id(v)));
            }
            // Cat.IS_LYING and RELAX_STATE_ONE.
            if kind == "minecraft:cat" {
                for (index, on) in [(21, e.flag), (22, e.relax_one)] {
                    if on {
                        meta.push((index, serializer::BOOLEAN, Value::Bool(true)));
                    }
                    defaults.push((index, serializer::BOOLEAN, Value::Bool(false)));
                }
            }
            // Wolf.DATA_COLLAR_COLOR and Cat's, red by default.
            let collar = if kind == "minecraft:cat" { 23 } else { 21 };
            let red = i32::from(horizon_client::mobs::tame::RED);
            if i32::from(e.color & 15) != red {
                meta.push((collar, serializer::INT, Value::Int(i32::from(e.color & 15))));
            }
            defaults.push((collar, serializer::INT, Value::Int(red)));
            // Wolf.DATA_ANGER_END_TIME: the game time the anger ends, -1 when calm.
            if kind == "minecraft:wolf" {
                // Wolf.DATA_INTERESTED_ID: BegGoal's flag, the head tilt.
                if e.flag {
                    meta.push((20, serializer::BOOLEAN, Value::Bool(true)));
                }
                defaults.push((20, serializer::BOOLEAN, Value::Bool(false)));
                if e.anger > 0 {
                    let end = game_time + i64::from(e.anger);
                    meta.push((22, serializer::LONG, Value::Long(end)));
                }
                defaults.push((22, serializer::LONG, Value::Long(-1)));
            }
        }
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:llama"
        | "minecraft:trader_llama"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:camel" => {
            // AbstractHorse.DATA_ID_FLAGS: tame 2, eating 16 (grazing), standing
            // 32 (rearing).
            let grazes = horizon_client::mobs::equine::grazes(m.species());
            let flags = if e.tame { 2 } else { 0 }
                | if grazes && e.flag { 16 } else { 0 }
                | if grazes && e.counter > 0 { 32 } else { 0 };
            if flags != 0 {
                meta.push((18, serializer::BYTE, Value::Byte(flags)));
            }
            defaults.push((18, serializer::BYTE, Value::Byte(0)));
            if kind == "minecraft:horse" {
                let v = i32::from(e.variant) | i32::from(e.variant2) << 8;
                if v != 0 {
                    meta.push((19, serializer::INT, Value::Int(v)));
                }
            } else if kind.ends_with("llama") {
                if e.chested {
                    meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
                }
                meta.push((
                    20,
                    serializer::INT,
                    Value::Int(i32::from(e.variant2.max(1))),
                ));
                if e.variant != 0 {
                    meta.push((21, serializer::INT, Value::Int(i32::from(e.variant))));
                }
            } else if (kind == "minecraft:donkey" || kind == "minecraft:mule") && e.chested {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        // HappyGhast.STAYS_STILL (after AgeableMob's two and IS_LEASH_HOLDER),
        // set while `serverStillTimeout > 0` (`syncStayStillFlag`).
        "minecraft:happy_ghast" => {
            // HappyGhast.IS_LEASH_HOLDER (`setLeashHolder` in `tick`).
            if horizon_client::mobs::happy_ghast::is_leash_holder(m) {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
            if horizon_client::mobs::happy_ghast::on_still_timeout(m) {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Villager.DATA_VILLAGER_DATA and ZombieVillager's (after Zombie's three).
        "minecraft:villager" | "minecraft:zombie_villager" => {
            use horizon_client::mobs::render::{PROFESSIONS, VILLAGER_TYPES};
            if kind == "minecraft:villager" {
                unhappy_counter(m, meta, defaults);
            } else {
                // ZombieVillager.DATA_CONVERTING_ID: the cure's shaking.
                if horizon_client::mobs::zombie_villager::is_converting(m) {
                    meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
                }
                defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
            }
            let villager_type = VILLAGER_TYPES[e.variant as usize % VILLAGER_TYPES.len()];
            let profession = PROFESSIONS[e.variant2 as usize % PROFESSIONS.len()];
            if let (Some(t), Some(p)) = (
                id("minecraft:villager_type", villager_type),
                id("minecraft:villager_profession", profession),
            ) {
                let index = if kind == "minecraft:villager" { 19 } else { 20 };
                let level = i32::from(e.variant3.max(1));
                meta.push((
                    index,
                    serializer::VILLAGER_DATA,
                    Value::Ids(vec![t, p, level]),
                ));
            }
        }
        // Phantom.ID_SIZE (after Mob's flags).
        "minecraft:phantom" => {
            if e.size > 0 {
                meta.push((16, serializer::INT, Value::Int(e.size)));
            }
            defaults.push((16, serializer::INT, Value::Int(0)));
        }
        // AbstractCubeMob.ID_SIZE; SulfurCube also syncs FROM_BUCKET and
        // MAX_FUSE (clients tick the fuse for the swell / white flash).
        "minecraft:slime" | "minecraft:magma_cube" | "minecraft:sulfur_cube" => {
            if e.size > 1 {
                meta.push((18, serializer::INT, Value::Int(e.size)));
            }
            defaults.push((18, serializer::INT, Value::Int(1)));
            if kind == "minecraft:sulfur_cube" {
                let from_bucket = e.scratch[3] != 0;
                if from_bucket {
                    meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
                }
                defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
                // SulfurCube.MAX_FUSE: set when primed; 0 means unprimed.
                let fuse = horizon_client::mobs::sulfur_cube::fuse(m);
                if fuse >= 0 {
                    meta.push((20, serializer::INT, Value::Int(fuse)));
                }
                defaults.push((20, serializer::INT, Value::Int(0)));
            }
        }
        // Creeper.DATA_SWELL_DIR, DATA_IS_POWERED and DATA_IS_IGNITED: the
        // guest swells the creeper from them. The direction always goes in,
        // as only changed entries are sent and it drops back to -1.
        "minecraft:creeper" => {
            meta.push((16, serializer::INT, Value::Int(e.counter)));
            if e.charged {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            if e.flag {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        // Pillager.IS_CHARGING_CROSSBOW: the charge pose and the held
        // item's loading animation.
        "minecraft:pillager" => {
            if e.flag {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Panda: UNHAPPY_COUNTER, SNEEZE_COUNTER and EAT_COUNTER (after
        // AgeableMob's two), MAIN_GENE_ID, HIDDEN_GENE_ID and DATA_ID_FLAGS;
        // the client animates the states from them.
        "minecraft:panda" => {
            let (flags, counters) = horizon_client::mobs::panda::get(m)
                .map_or((0, [0; 3]), |s| (s.flags, [s.unhappy, s.sneeze, s.eat]));
            for (index, value) in (18..).zip(counters) {
                if value != 0 {
                    meta.push((index, serializer::INT, Value::Int(value)));
                }
                defaults.push((index, serializer::INT, Value::Int(0)));
            }
            for (index, gene) in [(21, e.variant), (22, e.variant2)] {
                if gene != 0 {
                    meta.push((index, serializer::BYTE, Value::Byte(gene as i8)));
                }
                defaults.push((index, serializer::BYTE, Value::Byte(0)));
            }
            if flags != 0 {
                meta.push((23, serializer::BYTE, Value::Byte(flags as i8)));
            }
            defaults.push((23, serializer::BYTE, Value::Byte(0)));
        }
        // PolarBear.DATA_STANDING_ID (after AgeableMob's two): the bear
        // rears up; the client animates it.
        "minecraft:polar_bear" => {
            if e.flag {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
        }
        // SpellcasterIllager.DATA_SPELL_CASTING_ID: the casting pose.
        "minecraft:evoker" | "minecraft:illusioner" => {
            if e.variant2 != 0 {
                meta.push((17, serializer::BYTE, Value::Byte(e.variant2 as i8)));
            }
            defaults.push((17, serializer::BYTE, Value::Byte(0)));
        }
        // Warden.CLIENT_ANGER_LEVEL: the heartbeat's pace.
        "minecraft:warden" => {
            let anger = e.warden.as_ref().map_or(0, |w| w.client_anger);
            if anger != 0 {
                meta.push((16, serializer::INT, Value::Int(anger)));
            }
            defaults.push((16, serializer::INT, Value::Int(0)));
        }
        // Sniffer.DATA_STATE (after AgeableMob's two).
        "minecraft:sniffer" => {
            if e.variant != 0 {
                meta.push((
                    18,
                    serializer::SNIFFER_STATE,
                    Value::Id(i32::from(e.variant)),
                ));
            }
            defaults.push((18, serializer::SNIFFER_STATE, Value::Id(0)));
        }
        // Shulker.DATA_ATTACH_FACE_ID, DATA_PEEK_ID and DATA_COLOR_ID (16
        // for none).
        "minecraft:shulker" => {
            if e.variant3 != 0 {
                meta.push((16, serializer::DIRECTION, Value::Id(i32::from(e.variant3))));
            }
            defaults.push((16, serializer::DIRECTION, Value::Id(0)));
            if e.counter != 0 {
                meta.push((17, serializer::BYTE, Value::Byte(e.counter as i8)));
            }
            defaults.push((17, serializer::BYTE, Value::Byte(0)));
            if e.color != 16 {
                meta.push((18, serializer::BYTE, Value::Byte(e.color as i8)));
            }
            defaults.push((18, serializer::BYTE, Value::Byte(16)));
        }
        // Vex.DATA_FLAGS_ID: charging in bit 0 (the arms and the glowing
        // texture).
        "minecraft:vex" => {
            if e.flag {
                meta.push((16, serializer::BYTE, Value::Byte(1)));
            }
            defaults.push((16, serializer::BYTE, Value::Byte(0)));
        }
        // GlowSquid.DATA_DARK_TICKS_REMAINING (after AgeableMob's two): the
        // dimming a hit gave, counted down by the server and the client alike.
        "minecraft:glow_squid" => {
            let ticks = horizon_client::mobs::aquatic::dark_ticks(m);
            if ticks != 0 {
                meta.push((18, serializer::INT, Value::Int(ticks)));
            }
            defaults.push((18, serializer::INT, Value::Int(0)));
        }
        // Ocelot.DATA_TRUSTING (after AgeableMob's two).
        "minecraft:ocelot" => {
            if e.trusting {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Mannequin.DATA_PROFILE, DATA_IMMOVABLE and DATA_DESCRIPTION (after Avatar's two):
        // `updateDescription` keeps the last as a component, or none when hidden.
        "minecraft:mannequin" => {
            if let Some(profile) = &m.profile {
                meta.push((
                    17,
                    serializer::RESOLVABLE_PROFILE,
                    Value::Profile(profile.clone()),
                ));
            }
            if e.immovable {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
            let default = horizon_client::mobs::MannequinDescription::Default.component();
            let shown = e.description.component();
            if shown != default {
                meta.push((19, serializer::OPTIONAL_COMPONENT, Value::OptText(shown)));
            }
            defaults.push((19, serializer::OPTIONAL_COMPONENT, Value::OptText(default)));
        }
        // Creaking.CAN_MOVE, IS_ACTIVE and IS_TEARING_DOWN.
        "minecraft:creaking" => {
            if e.flag2 {
                meta.push((16, serializer::BOOLEAN, Value::Bool(false)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(true)));
            if e.flag {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
            if horizon_client::mobs::creaking::tearing_down(m) {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Allay.DATA_DANCING and DATA_CAN_DUPLICATE (the held item is its
        // main hand equipment).
        "minecraft:allay" => {
            if e.flag {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
            if e.egg_time > 0 {
                meta.push((17, serializer::BOOLEAN, Value::Bool(false)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(true)));
        }
        // Axolotl.DATA_PLAYING_DEAD (after DATA_VARIANT).
        "minecraft:axolotl" => {
            if horizon_client::mobs::axolotl::is_playing_dead(m) {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        }
        // CopperGolem.COPPER_GOLEM_STATE (after DATA_WEATHER_STATE).
        "minecraft:copper_golem" => {
            if e.variant3 != 0 {
                meta.push((
                    17,
                    serializer::COPPER_GOLEM_STATE,
                    Value::Id(i32::from(e.variant3)),
                ));
            }
            defaults.push((17, serializer::COPPER_GOLEM_STATE, Value::Id(0)));
        }
        // Armadillo.ARMADILLO_STATE (after AgeableMob's two).
        "minecraft:armadillo" => {
            let state = horizon_client::mobs::armadillo::state(m);
            if state != 0 {
                meta.push((18, serializer::ARMADILLO_STATE, Value::Id(i32::from(state))));
            }
            defaults.push((18, serializer::ARMADILLO_STATE, Value::Id(0)));
        }
        // Frog.DATA_TONGUE_TARGET_ID (after the variant): the entity id plus
        // one, 0 for none.
        "minecraft:frog" => {
            if let Some(target) = e.frog.tongue_target {
                let id = i32::try_from(target).map_or(0, |t| t + 1);
                meta.push((19, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(id)));
            }
            defaults.push((19, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(0)));
        }
        // Goat.DATA_HAS_LEFT_HORN and DATA_HAS_RIGHT_HORN (after
        // DATA_IS_SCREAMING_GOAT): both true by default.
        "minecraft:goat" => {
            for (index, lost) in [(19, e.lost_horns[0]), (20, e.lost_horns[1])] {
                if lost {
                    meta.push((index, serializer::BOOLEAN, Value::Bool(false)));
                }
                defaults.push((index, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        // Enderman.DATA_CARRY_STATE and DATA_CREEPY (angry: the open mouth).
        "minecraft:enderman" => {
            if let Some(state) = e.carried {
                meta.push((
                    16,
                    serializer::OPTIONAL_BLOCK_STATE,
                    Value::Id(state.0 as i32),
                ));
            }
            if e.flag {
                meta.push((17, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((17, serializer::BOOLEAN, Value::Bool(false)));
            defaults.push((16, serializer::OPTIONAL_BLOCK_STATE, Value::Id(0)));
        }
        // Zombie.DATA_DROWNED_CONVERSION_ID (after DATA_BABY_ID and
        // DATA_SPECIAL_TYPE_ID): the shaking of the conversion under water.
        "minecraft:zombie" | "minecraft:husk" => {
            if e.counter2 > 0 {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Skeleton.DATA_STRAY_CONVERSION_ID (the first entry after Mob's):
        // the shaking of the freezing conversion.
        "minecraft:skeleton" => {
            if e.counter2 > 0 {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Bogged.DATA_SHEARED (the first entry after Mob's): the mushrooms
        // on its head are gone.
        "minecraft:bogged" => {
            if e.sheared {
                meta.push((16, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((16, serializer::BOOLEAN, Value::Bool(false)));
        }
        // AbstractVillager.DATA_UNHAPPY_COUNTER (after AgeableMob's two).
        "minecraft:wandering_trader" => unhappy_counter(m, meta, defaults),
        // Hoglin.DATA_IMMUNE_TO_ZOMBIFICATION (after AgeableMob's two) and
        // AbstractPiglin's, the first entry of its subclasses (the piglin's is
        // in its own arm): with the dimension it makes them shake.
        "minecraft:hoglin" | "minecraft:piglin_brute" => {
            let index = if kind == "minecraft:hoglin" { 18 } else { 16 };
            if e.flag2 {
                meta.push((index, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((index, serializer::BOOLEAN, Value::Bool(false)));
        }
        // Blaze.DATA_FLAGS_ID: charged 1, which makes it burn.
        "minecraft:blaze" => {
            if e.flag {
                meta.push((16, serializer::BYTE, Value::Byte(1)));
            }
            defaults.push((16, serializer::BYTE, Value::Byte(0)));
        }
        // Turtle.HAS_EGG and LAYING_EGG (after AgeableMob's two).
        "minecraft:turtle" => {
            if e.flag {
                meta.push((18, serializer::BOOLEAN, Value::Bool(true)));
            }
            if e.flag2 {
                meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
            }
            defaults.push((18, serializer::BOOLEAN, Value::Bool(false)));
            defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        }
        // WitherBoss.DATA_TARGET_B and DATA_TARGET_C (after DATA_TARGET_A, the
        // main target): the entities the side heads look at, by id.
        "minecraft:wither" => {
            for (k, index) in [(0, 17), (1, 18)] {
                let target = match e.ints[k] {
                    0 => None,
                    -1 => player_entity,
                    id => Some(id),
                };
                if let Some(target) = target {
                    meta.push((index, serializer::INT, Value::Int(target)));
                }
                defaults.push((index, serializer::INT, Value::Int(0)));
            }
        }
        _ => {}
    }
    // Camel.DASH and LAST_POSE_CHANGE_TICK (after AbstractHorse's flags).
    if horizon_client::mobs::camel::is_camel(m.species()) {
        if e.flag2 {
            meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
        let tick = horizon_client::mobs::camel::synced_pose_tick(m, game_time);
        if tick != 0 {
            meta.push((20, serializer::LONG, Value::Long(tick)));
        }
        defaults.push((20, serializer::LONG, Value::Long(0)));
    }
}

/// AbstractVillager.DATA_UNHAPPY_COUNTER (after AgeableMob's two): the head
/// shake of a villager saying no counts it down from 40.
fn unhappy_counter(
    m: &Mob,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    if m.extra.ints[0] > 0 {
        meta.push((18, serializer::INT, Value::Int(m.extra.ints[0])));
    }
    defaults.push((18, serializer::INT, Value::Int(0)));
}

/// What a client shakes a mob by: Entity.DATA_TICKS_FROZEN (7), whose full
/// count is `isFullyFrozen`, and Strider.DATA_SUFFOCATING (19, after
/// AgeableMob's two and DATA_BOOST_TIME).
fn mob_freeze_meta(
    m: &Mob,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) {
    if m.ticks_frozen > 0 {
        meta.push((7, serializer::INT, Value::Int(m.ticks_frozen)));
    }
    defaults.push((7, serializer::INT, Value::Int(0)));
    if m.kind.id == "minecraft:strider" {
        if m.extra.flag {
            meta.push((19, serializer::BOOLEAN, Value::Bool(true)));
        }
        defaults.push((19, serializer::BOOLEAN, Value::Bool(false)));
    }
}

/// The equipment a mob shows (ClientboundSetEquipmentPacket slots: main hand,
/// off hand, feet, legs, chest, head, body, saddle).
fn mob_equipment(
    m: &Mob,
    hub: &mut Hub,
    out: &mut Vec<(u8, Option<horizon_protocol::item::Stack>)>,
) {
    for (slot, stack) in m.equipment_stacks().into_iter().enumerate() {
        if stack.is_some() {
            out.push((slot as u8, hub.stack(&stack)));
        }
    }
}

pub const HELP: &[&str] = &[
    "lan open [PORT] [commands] [force]  open the world to the network (PORT 0 picks one)",
    "lan close                      stop hosting",
    "lan status                     the hosting state and the guests",
];

/// With the game mode forced, the guests online take the default mode
/// (setting a mode does nothing when it is already theirs). A world open to
/// LAN is an integrated server, which leaves out the players who have game
/// master permission; a dedicated server does not.
fn enforce_guest_game_mode(
    host: &mut LanHost,
    world: &World,
    collision: &BlockCollisionRegistry<'_>,
) {
    if !host.hub.config.force_game_mode {
        return;
    }
    let mode = host.hub.config.default_game_mode;
    let spares_operators = !host.hub.config.dedicated || host.hub.config.integrated;
    let stale: Vec<u32> = host
        .hub
        .guests
        .iter()
        .filter(|g| g.game_mode != mode && !(spares_operators && g.permission >= 2))
        .map(|g| g.conn)
        .collect();
    for conn in stale {
        host.hub.set_game_mode(conn, mode, world, Some(collision));
    }
}

impl Game<'_> {
    /// `horizon-ctl lan ...`.
    pub(super) fn lan_control(&mut self, words: &[&str]) -> Result<Option<serde_json::Value>> {
        use serde_json::json;
        if words[0] != "lan" {
            return Ok(None);
        }
        if let Some(server) = &self.integrated
            && words.get(1) == Some(&"status")
        {
            // The hub is the server thread's.
            let status = server
                .call(|g| g.lan_status())
                .ok_or("the integrated server stopped")?;
            return Ok(Some(status));
        }
        match words.get(1).copied() {
            Some("open") => {
                if self.title.is_some() {
                    return Err("open a world first".into());
                }
                let port = words.get(2).map_or(Ok(0), |p| p.parse::<u16>())?;
                self.lan_open(LanSettings {
                    port: if port == 0 {
                        lan::available_port()
                    } else {
                        port
                    },
                    guest_commands: words.contains(&"commands"),
                    force_game_mode: words.contains(&"force"),
                })?;
                Ok(Some(json!({ "ok": true, "port": self.lan_port() })))
            }
            Some("close") => {
                self.lan_close(true);
                Ok(Some(json!({ "ok": true })))
            }
            Some("status") => Ok(Some(self.lan_status())),
            _ => Err("lan open|close|status".into()),
        }
    }

    /// `horizon-ctl lan status`.
    fn lan_status(&self) -> serde_json::Value {
        use serde_json::json;
        match &self.lan {
            Some(h) if !self.serving() || self.published_port().is_some() => json!({
                "open": true,
                "host": h.host_name,
                "port": h.settings.port,
                // The server's own world: its finished chunks and how many it
                // has generated or loaded so far.
                "world": {
                    "chunks": self.world.chunks.len(),
                    "generated": self.streaming.generated_total,
                },
                "guests": h.hub.guests.iter().map(|g| json!({
                    "name": g.name,
                    "position": g.position,
                    "placed": g.placed,
                    "loaded": g.loaded,
                    "game_mode": g.game_mode,
                    // ItemCooldowns: the ticks the shield an axe disabled stays down.
                    "shield_cooldown": h.play.get(&g.conn).map_or(0, |p| p.shield_cooldown),
                    // The item the server has this guest using: its slot and ticks.
                    "using": h.play.get(&g.conn).and_then(|p| p.using())
                        .map(|(slot, ticks)| json!({ "slot": slot, "ticks": ticks })),
                    "sneaking": g.sneaking,
                    // The server's Elytra glide (shared flag 7).
                    "fall_flying": g.fall_flying,
                    "sleeping": g.bed,
                    "health": g.health,
                    // The server's body: its air supply and golden hearts.
                    "air": g.motion.as_ref().map(|m| m.air),
                    "absorption": g.motion.as_ref().map(|m| m.health.absorption()),
                    // FoodData: food level, saturation and exhaustion.
                    "food": [
                        f64::from(g.food),
                        f64::from(g.saturation),
                        g.motion.as_ref().map_or(0.0, |m| f64::from(m.health.food.exhaustion)),
                    ],
                    "chunks": g.sent_chunks.len(),
                    "tracked": g.tracker.len(),
                    // Whether the host draws this guest lying in a bed.
                    "model_lying": h.models.get(&g.conn).map(|m| m.extra.ints[1] != 0),
                    // The armour the host draws on this guest, head to feet.
                    "model_armor": h.models.get(&g.conn).map(|m| {
                        [&m.extra.head, &m.extra.chest, &m.extra.legs, &m.extra.feet]
                            .map(|s| s.as_deref())
                    }),
                    // The host draws this guest's arm swinging.
                    "model_swinging": h.models.get(&g.conn).map(|m| m.ai.swinging),
                    // For how many ticks the host draws this guest using an item.
                    "model_using": h.models.get(&g.conn).map(|m| m.extra.using_item),
                })).collect::<Vec<_>>(),
            }),
            _ => json!({ "open": false }),
        }
    }

    pub(super) fn lan_open(&mut self, settings: LanSettings) -> Result<()> {
        let port = if let Some(server) = &self.integrated {
            // ShareToLanScreen: IntegratedServer.publishServer.
            server
                .call(move |g| g.publish(settings))
                .ok_or("the integrated server stopped")??
        } else if self.serving() {
            return self.publish(settings).map(drop).map_err(Into::into);
        } else {
            self.lan_start(settings)?;
            self.lan.as_ref().map_or(0, |l| l.settings.port)
        };
        self.system_message(Text::translate(
            "menu.multiplayerOptions.publish.started.lan",
            &[Text::literal(port.to_string())],
        ));
        Ok(())
    }
    /// IntegratedServer.publishServer without the screen's announcement.
    pub(super) fn lan_start(&mut self, settings: LanSettings) -> Result<()> {
        if self.lan.is_some() || self.remote.is_some() || self.title.is_some() {
            return Err("already open".into());
        }
        // IntegratedServer.publishServer: the MOTD names the host's profile.
        let name = self.options.player_name();
        let motd = format!("{name} - {}", self.level.name);
        let host_uuid = self.level.player_uuid.map_or_else(
            || offline_uuid(&name),
            |u| {
                u128::from(u[0] as u32) << 96
                    | u128::from(u[1] as u32) << 64
                    | u128::from(u[2] as u32) << 32
                    | u128::from(u[3] as u32)
            },
        );
        let config = HubConfig {
            allow_flight: true,
            default_game_mode: self.level.mode(),
            guest_commands: settings.guest_commands,
            force_game_mode: settings.force_game_mode,
            hardcore: self.level.hardcore,
            view_distance: self.settings.render_distance.clamp(2, 32),
            simulation_distance: self.settings.render_distance.clamp(2, 32),
            dimension: self.dim.id().to_owned(),
            levels: self.level_shapes,
            seed: self.level.seed,
            max_players: MAX_PLAYERS,
            dedicated: false,
            integrated: false,
        };
        let net = NetConfig {
            levels: self.level_shapes,
            online: horizon_client::lan::OnlineMode::lan(),
            ..NetConfig::lan(settings.port, motd, MAX_PLAYERS)
        };
        self.lan_host(settings, net, config, Some(host_uuid))
    }
    /// Start the network side and the play-state hub: for the LAN (with the
    /// host's own profile reserved) or for a dedicated server (no host).
    pub(super) fn lan_host(
        &mut self,
        settings: LanSettings,
        net: NetConfig,
        config: HubConfig,
        host: Option<u128>,
    ) -> Result<()> {
        let sync = sync_data(&self.options.resources, &self.pack_files())?;
        self.sync_dialog_ids(&sync);
        // The table the world plays with (`world_biomes!`): a world with
        // biome data packs sends their ids past the built-in list, as the
        // encoder's `SectionCoder` expects.
        let biome_names: Vec<String> = world_biomes!(self)
            .list
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let registry: Vec<String> = sync
            .dynamic
            .registries
            .get("minecraft:worldgen/biome")
            .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        let mut encoder = ChunkEncoder::new(
            self.res
                .facts()
                .states
                .iter()
                .map(|s| (s.name.clone(), s.fluid_level > 0 || s.waterlogged)),
            &sync.builtin,
            &biome_names,
            &registry,
        );
        encoder.tables = Some(
            self.save
                .tables
                .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&self.pack.assets)))
                .clone(),
        );
        encoder.mod_update_tag = Some(|pos, entity| {
            let mut tag = None;
            Game::native_block_entity_update_tag(pos, entity, &mut tag);
            tag
        });
        let far = (net.lod_distance > 0)
            .then(|| horizon_client::lod_net::Service::new(net.lod_distance, biome_names.clone()));
        let lan = Lan::start_with(sync.clone(), net)?;
        if let Some(uuid) = host {
            lan.reserve(uuid);
        }
        let host_uuid = host.unwrap_or(0);
        self.next_entity += 1;
        let host_entity = self.next_entity as i32;
        let mut hub = Hub::new(lan, sync, encoder, config);
        hub.flat_world = self.world_flat;
        hub.debug_world = self.world_debug;
        let port = hub.lan.port;
        self.world.block_log = Some(Vec::new());
        self.world.light_log = Some(Default::default());
        self.lan = Some(LanHost {
            players_told_left: false,
            hub,
            settings: LanSettings { port, ..settings },
            host_entity,
            host_swings: self.swings_started,
            host_crack: None,
            host_uuid,
            host_name: self.options.player_name(),
            models: HashMap::new(),
            host_mode: self.mode_id(),
            play: HashMap::new(),
            broken: Vec::new(),
            mob_state: HashMap::new(),
            mob_players: Default::default(),
            owner: None,
            waypoints: Default::default(),
            waypoint_dead: Default::default(),
            border_sent: None,
            commands: Vec::new(),
            quiet: std::collections::VecDeque::new(),
            far,
            native_commands: None,
            sleep_ignored: Default::default(),
            states: Default::default(),
        });
        self.audio.capture = Some(Vec::new());
        Ok(())
    }

    /// IntegratedServer.unpublishServer: the guests are sent away.
    pub(super) fn lan_close(&mut self, announce: bool) {
        if self.integrated.is_some() || self.serving() {
            let closed = match &self.integrated {
                Some(server) => server.call(|g| g.unpublish()).unwrap_or(false),
                None => self.unpublish(),
            };
            if closed {
                self.friends_lan_closed();
            }
            if closed && announce && self.integrated.is_some() {
                self.system_message(Text::translate(
                    "menu.multiplayerOptions.publish.stopped",
                    &[],
                ));
            }
            return;
        }
        self.friends_lan_closed();
        self.native_players_leaving();
        let Some(mut host) = self.lan.take() else {
            return;
        };
        host.award_leave_game_to_all();
        self.save_leaving_guests(&mut host);
        host.hub.disconnect_all();
        // Give the writer threads a moment to flush the disconnect packets.
        std::thread::sleep(std::time::Duration::from_millis(100));
        self.world.block_log = None;
        self.world.light_log = None;
        self.audio.capture = None;
        self.lan_events.clear();
        self.lan_songs.clear();
        self.lan_critical_hits.clear();
        self.lan_cushion_breaks.clear();
        self.streaming.extra.clear();
        if announce {
            self.system_message(Text::translate(
                "menu.multiplayerOptions.publish.stopped",
                &[],
            ));
        }
    }

    /// `PlayerList.removeAll`: every guest leaves in turn, the vehicle it
    /// alone rides with it (`PlayerList.remove`), and is saved.
    pub(super) fn save_leaving_guests(&mut self, host: &mut LanHost) {
        // Every open menu closes first, in its guest's level
        // (`ServerPlayer.disconnect` -> `doCloseContainer`), so the cursor,
        // the trade and the inputs are back in the file that is saved.
        let back = self.dim;
        let open: Vec<(u32, super::Dim)> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed)
            .map(|g| (g.conn, super::Dim::from_id(&g.dimension)))
            .collect();
        for (conn, dim) in open {
            if self.enter_level(Some(host), dim).is_ok() {
                self.guest_close_all(host, conn);
            }
        }
        if let Err(e) = self.enter_level(Some(host), back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
        for i in 0..host.hub.guests.len() {
            let mut g = host.hub.guests.remove(i);
            self.take_parent_vehicle(host, &mut g);
            self.save_guest(host, &mut g);
            host.hub.guests.insert(i, g);
        }
    }
    /// Save a guest when they leave or the world closes (after
    /// `take_parent_vehicle`, `PlayerList.save`): an integrated server's owner
    /// into the level (`/data`) and both its files (`playerdata::save_owner`),
    /// anyone else into their own file.
    pub(super) fn save_guest(
        &mut self,
        host: &LanHost,
        guest: &mut horizon_client::lan::guest::Guest,
    ) {
        self.guest_attachments_into(guest);
        if guest.placed && host.owner.as_ref().is_some_and(|o| o.uuid == guest.uuid) {
            playerdata::save_owner(guest, &mut self.level, &self.world_root);
        } else if guest.placed {
            playerdata::save(guest, &self.world_root);
        }
        self.save_guest_records(host, guest);
    }
    /// The vehicles the other players in level `dim` ride, all but the
    /// guest `but` (`Entity.hasExactlyOnePlayerPassenger`); `host` adds the
    /// host's own player, in the level in place.
    pub(super) fn ridden_by_others(
        &self,
        guests: &[horizon_client::lan::guest::Guest],
        dim: &str,
        but: Option<u128>,
        host: bool,
    ) -> Vec<u32> {
        let mut others: Vec<u32> = guests
            .iter()
            .filter(|g| g.placed && g.dimension == dim && Some(g.uuid) != but)
            .filter_map(|g| g.vehicle)
            .collect();
        if host && super::Dim::from_id(dim) == self.dim {
            others.extend(self.riding);
        }
        others
    }
    /// `ServerPlayer.saveParentVehicle` of a player riding `vehicle` in level
    /// `dim`, put in place for it if need be; with `remove`, the vehicle tree
    /// leaves the world with the player (`PlayerList.remove`).
    fn parent_vehicle_in(
        &mut self,
        mut host: Option<&mut LanHost>,
        dim: &str,
        vehicle: Option<u32>,
        others: &[u32],
        remove: bool,
    ) -> Option<(horizon_world::nbt::Tag, [i32; 4])> {
        vehicle?;
        let back = self.dim;
        let theirs = super::Dim::from_id(dim);
        if theirs != back
            && let Err(e) = self.enter_level(host.as_deref_mut(), theirs)
        {
            log::error!("could not open {}: {e}", theirs.id());
            return None;
        }
        let found = if self.dim == theirs {
            self.parent_vehicle_tag(vehicle, others)
        } else {
            None
        };
        if remove && let Some((_, root, _)) = &found {
            self.remove_vehicle_tree(*root);
        }
        if self.dim != back
            && let Err(e) = self.enter_level(host, back)
        {
            log::error!("could not reopen {}: {e}", back.id());
        }
        found.map(|(tag, _, attach)| (tag, attach))
    }
    /// `PlayerList.remove` of a guest out of the hub: its file takes the
    /// vehicle it alone rides (`saveParentVehicle`'s RootVehicle, kept in
    /// `saved` as a file not yet ridden again), the vehicle tree leaves the
    /// world, and the guest stops riding (`unRide`).
    pub(super) fn take_parent_vehicle(
        &mut self,
        host: &mut LanHost,
        guest: &mut horizon_client::lan::guest::Guest,
    ) {
        let Some(vehicle) = guest.vehicle.take() else {
            return;
        };
        if !guest.placed {
            return;
        }
        let others = self.ridden_by_others(&host.hub.guests, &guest.dimension, None, true);
        let Some((tag, attach)) =
            self.parent_vehicle_in(Some(host), &guest.dimension, Some(vehicle), &others, true)
        else {
            return;
        };
        let saved = guest
            .saved
            .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
        if let horizon_world::nbt::Tag::Compound(p) = saved {
            p.insert("RootVehicle".into(), tag);
        }
        guest.saved_vehicle = Some(attach);
    }
    /// `ServerPlayer.loadAndSpawnParentVehicle`: a guest whose file names the
    /// vehicle it rode rides it again, once the vehicle is in the world.
    fn restore_guest_vehicles(&mut self, host: &mut LanHost) {
        let waiting: Vec<(usize, [i32; 4])> = host
            .hub
            .guests
            .iter()
            .enumerate()
            .filter_map(|(i, g)| Some((i, g.saved_vehicle?)))
            .filter(|(i, _)| {
                let g = &host.hub.guests[*i];
                let chunk = (
                    (g.position[0].floor() as i32).div_euclid(16),
                    (g.position[2].floor() as i32).div_euclid(16),
                );
                g.placed
                    && g.vehicle.is_none()
                    && g.dimension == self.dim.id()
                    && g.sent_chunks.contains(&chunk)
            })
            .collect();
        for (i, uuid) in waiting {
            let raw = host.hub.guests[i].saved.clone();
            let Some(id) = self.restore_saved_vehicle(uuid, raw.as_ref()) else {
                continue;
            };
            host.hub.seat_order += 1;
            let order = host.hub.seat_order;
            let g = &mut host.hub.guests[i];
            if let Some(horizon_world::nbt::Tag::Compound(p)) = &mut g.saved {
                p.remove(&"RootVehicle".into());
            }
            g.vehicle = Some(id);
            g.vehicle_order = order;
            g.saved_vehicle = None;
            // Pair the stationary restored entities before their passenger packet.
            g.moved = true;
        }
    }
    /// The file name of a player's advancements and statistics: an
    /// integrated server's owner keeps single player's.
    fn records_uuid(&self, host: &LanHost, uuid: u128) -> Option<[i32; 4]> {
        if host.owner.as_ref().is_some_and(|o| o.uuid == uuid) {
            self.level.player_uuid
        } else {
            Some(playerdata::uuid_ints(uuid))
        }
    }
    /// `PlayerAdvancements.save` and `ServerStatsCounter.save` of a network player.
    pub(super) fn save_guest_records(
        &self,
        host: &LanHost,
        guest: &horizon_client::lan::guest::Guest,
    ) {
        if !guest.placed || self.adv.tree.list.is_empty() {
            return;
        }
        let Some(uuid) = self.records_uuid(host, guest.uuid) else {
            return;
        };
        let path = super::stats::player_json(&self.world_root, "advancements", uuid);
        let json = guest
            .advancements
            .to_json(&self.adv.tree, horizon_world::DATA_VERSION);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(path, text);
        }
        if let Some(play) = host.play.get(&guest.conn) {
            play.stats
                .write(&super::stats::player_json(&self.world_root, "stats", uuid));
        }
    }
    /// `PlayerAdvancements.load` and `ServerStatsCounter.parseLocal` of a
    /// network player who joined; every statistic goes out on the first
    /// request (`PlayerList.placeNewPlayer`'s `markAllDirty`).
    fn load_guest_records(&mut self, host: &mut LanHost, conn: u32) {
        let Some(uuid) = host
            .hub
            .guest(conn)
            .and_then(|g| self.records_uuid(host, g.uuid))
        else {
            return;
        };
        let tree = self.advancement_tree();
        let path = super::stats::player_json(&self.world_root, "advancements", uuid);
        let saved = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        if let Some(g) = host.hub.guest_mut(conn)
            && let Some(json) = saved
        {
            let mut progress = horizon_client::advancements::Progress::from_json(&json);
            // PlayerAdvancements.applyFrom: advancements this server lacks are dropped.
            progress
                .criteria
                .retain(|id, _| tree.by_id.contains_key(id));
            g.advancements = progress;
            g.advancements_loaded = true;
        }
        let mut stats =
            super::stats::Stats::read(&super::stats::player_json(&self.world_root, "stats", uuid));
        stats.mark_all_dirty();
        host.play.entry(conn).or_default().stats = stats;
    }
    /// The integrated server's owner into the world's player, for `Level::write`.
    pub(super) fn store_owner(&mut self) {
        let Some(uuid) = self
            .lan
            .as_ref()
            .and_then(|h| h.owner.as_ref())
            .map(|o| o.uuid)
        else {
            return;
        };
        let Some((riding, dim, others)) = self.lan.as_ref().and_then(|h| {
            let g = h.hub.guests.iter().find(|g| g.uuid == uuid && g.placed)?;
            let others = self.ridden_by_others(&h.hub.guests, &g.dimension, Some(uuid), true);
            Some((g.vehicle, g.dimension.clone(), others))
        }) else {
            return;
        };
        // The vehicle the owner rides (`ServerPlayer.saveParentVehicle`).
        let vehicle = self
            .parent_vehicle_in(None, &dim, riding, &others, false)
            .map(|(tag, _)| tag);
        let Some(host) = &self.lan else {
            return;
        };
        if let Some(g) = host.hub.guests.iter().find(|g| g.uuid == uuid && g.placed) {
            playerdata::store(g, &mut self.level, vehicle);
            self.save_guest_records(host, g);
        } else if let Some(saved) = host.owner.as_ref().and_then(|o| o.saved.as_ref()) {
            self.level.position = saved.position;
        }
    }

    /// The world's default game mode (`defaultgamemode`).
    pub(super) fn set_default_game_mode(&mut self, mode: i32) {
        self.level.game_mode = mode;
        self.level.creative = mode == 1;
        if let Some(host) = self.lan.as_mut() {
            host.hub.config.default_game_mode = mode;
            // `setWorldGameType` and `DefaultGameModeCommands.setMode`: a
            // world that forces its mode puts the guests online in it.
            enforce_guest_game_mode(host, &self.world, self.collision);
        }
    }

    /// New command access and game mode forcing settings of the open world.
    pub(super) fn lan_apply_settings(&mut self, guest_commands: bool, force: bool) {
        if let Some(server) = &self.integrated {
            server.call(move |g| g.lan_apply_settings(guest_commands, force));
            return;
        }
        // An integrated server gives guests commands only where the world
        // allows them (IntegratedServer.getCustomPermissionLevel).
        let effective = guest_commands && (!self.serving() || self.allow_commands());
        if let Some(host) = self.lan.as_mut() {
            host.settings.guest_commands = guest_commands;
            host.settings.force_game_mode = force;
            host.hub.config.guest_commands = effective;
            host.hub.config.force_game_mode = force;
            enforce_guest_game_mode(host, &self.world, self.collision);
            host.hub.resend_commands();
        }
    }

    pub(super) fn lan_port(&self) -> Option<u16> {
        if let Some(server) = &self.integrated {
            return server.published();
        }
        if self.serving() {
            return self.published_port();
        }
        self.lan.as_ref().map(|h| h.settings.port)
    }

    /// The host as the guests see them.
    fn lan_host_state(&self, host: &LanHost) -> Option<HostState> {
        let p = self.player.as_ref()?;
        let mut equipment: [Option<ItemStack>; 6] = Default::default();
        equipment[0] = self.inventory.get(self.selected).cloned().flatten();
        equipment[1] = self.offhand.clone();
        // Feet, legs, chest, head are slots 2..=5; armor is stored head first.
        equipment[2] = self.armor[3].clone();
        equipment[3] = self.armor[2].clone();
        equipment[4] = self.armor[1].clone();
        equipment[5] = self.armor[0].clone();
        Some(HostState {
            uuid: host.host_uuid,
            name: host.host_name.clone(),
            entity_id: host.host_entity,
            position: [p.position.x, p.position.y, p.position.z],
            yaw: self.yaw,
            pitch: self.pitch,
            head_yaw: self.yaw,
            on_ground: p.ground.on_ground,
            game_mode: self.mode_id(),
            sneaking: p.pose == horizon_client::pose::Pose::Crouching,
            sprinting: p.sprinting,
            equipment,
            health: p.health.health(),
            attributes: horizon_client::lan::attributes::syncable(&p.attributes),
            // Bit 4: Riptide's spin (LivingEntity.isAutoSpinAttack).
            living_flags: living_flags(self.using)
                | if p.auto_spin_attack_ticks > 0 { 4 } else { 0 },
            stuck: (self.combat.stuck.arrows, self.combat.stuck.stingers),
            shoulders: shoulder_variants(self.level.player_extra.as_ref()),
            bed: self.sleeping,
            body: BodyFlags {
                gliding: p.fall_flying,
                swimming: p.swimming,
                crawling: p.pose == horizon_client::pose::Pose::Swimming && !p.swimming,
                crouching: p.pose == horizon_client::pose::Pose::Crouching,
                spin_attack: p.auto_spin_attack_ticks > 0,
                on_fire: p.fire_ticks > 0,
                // ServerPlayer.updateInvisibilityStatus: a spectator is always invisible.
                invisible: self.potions.effects.get("invisibility").is_some() || self.spectator(),
                glowing: self.potions.effects.get("glowing").is_some(),
            },
        })
    }

    /// The entities guests track: mobs, items and the rest (game/lan_entities.rs).
    /// `mob_players` is the guest each mob sees as its player (game/lan_mobs.rs).
    fn lan_entities(
        &self,
        host_entity: Option<i32>,
        mob_players: &horizon_client::fast_hash::FxHashMap<u32, u32>,
        hub: &mut Hub,
        out: &mut States,
    ) {
        out.clear();
        for m in self.mobs.iter().filter(|m| !m.removed) {
            // The lead's holder: the player the mob sees (a guest, else the
            // host), another mob or a fence knot (game/lan_entities.rs).
            let player_entity = mob_players
                .get(&m.id)
                .and_then(|conn| hub.guest(*conn))
                .map(|g| g.entity_id)
                .or(host_entity);
            let leash = m.extra.leash.and_then(|h| match h {
                Holder::Player => player_entity,
                Holder::Mob(id) | Holder::Boat(id) => Some(id as i32),
                Holder::Knot(p) => self.knot_ids.get(&p).map(|&id| id as i32),
            });
            let e = out.add(m.id as i32, entity_uuid(m.id), m.kind.id.as_str());
            e.position = [m.position.x, m.position.y, m.position.z];
            (e.yaw, e.pitch, e.head_yaw) = (m.yaw, m.head_pitch, m.head_yaw);
            e.on_ground = m.on_ground;
            e.velocity = [m.velocity.x, m.velocity.y, m.velocity.z];
            e.needs_sync = m.needs_sync;
            mob_meta(
                m,
                hub,
                player_entity,
                self.world.game_time,
                &mut e.meta,
                &mut e.defaults,
            );
            // HappyGhast.aiStep: requiresPrecisePosition while on a still timeout.
            e.precise = m.species() == horizon_client::mobs::Species::HappyGhast
                && horizon_client::mobs::happy_ghast::on_still_timeout(m);
            mob_freeze_meta(m, &mut e.meta, &mut e.defaults);
            mob_equipment(m, hub, &mut e.equipment);
            self.mob_attributes(m, &mut e.attributes);
            e.leash = leash;
        }
        for d in &self.dropped {
            let stack = hub.stack(&Some(d.stack.clone()));
            let e = out.add(d.id as i32, entity_uuid(d.id), "minecraft:item");
            e.position = [d.position.x, d.position.y, d.position.z];
            e.on_ground = d.on_ground;
            e.velocity = [d.velocity.x, d.velocity.y, d.velocity.z];
            e.precise = d.needs_sync;
            e.needs_sync = d.needs_sync;
            // Entity.DATA_SHARED_FLAGS_ID: glowing 0x40 (the Glowing tag).
            if self.entity_glowing(d.id) {
                e.meta.push((0, serializer::BYTE, Value::Byte(0x40)));
            }
            e.meta.push((8, serializer::ITEM_STACK, Value::Item(stack)));
        }
        for o in &self.orbs {
            let e = out.add(o.id as i32, entity_uuid(o.id), "minecraft:experience_orb");
            e.position = [o.position.x, o.position.y, o.position.z];
            e.on_ground = o.on_ground;
            e.velocity = [o.velocity.x, o.velocity.y, o.velocity.z];
            // ExperienceOrb.DATA_VALUE.
            e.meta
                .push((8, serializer::INT, Value::Int(o.value as i32)));
        }
        self.lan_object_states(host_entity, hub, out);
    }

    /// Apply gameplay events produced while the hub handles guest packets.
    ///
    /// Both the pre-block packet phase and the normal LAN tick receive the
    /// same event stream. Keep their side effects in one place so joins,
    /// departures, chat, and queued guest commands cannot drift apart. None when a native hook's
    /// command closed the hub.
    fn lan_process_events(
        &mut self,
        mut host: LanHost,
        events: impl IntoIterator<Item = HostEvent>,
        state: Option<&HostState>,
        entities: &[EntityState],
        spawn: [f64; 3],
    ) -> Option<LanHost> {
        for event in events {
            match event {
                event @ (HostEvent::Chat { .. } | HostEvent::Warn(_) | HostEvent::Error(_)) => {
                    self.lan_log_event(event);
                }
                HostEvent::Ban { uuid, name, reason } => {
                    self.anticheat_ban(&mut host.hub, uuid, &name, &reason);
                }
                HostEvent::ChatFiltered {
                    sender,
                    name,
                    place,
                    text,
                    count,
                    action,
                } => self.chat_filtered(&mut host, sender, &name, place, &text, count, action),
                HostEvent::PlayerChat { sender, text } => {
                    if let Some(g) = host.hub.guests.iter().find(|g| g.uuid == sender) {
                        if self.options.dedicated.is_some() {
                            super::dedicated::events::record(
                                "chat",
                                &g.name,
                                sender,
                                serde_json::json!({ "message": text }),
                            );
                        }
                        let conn = g.conn;
                        self.detect_rate_spam(&mut host, conn, false);
                    }
                }
                HostEvent::Joined(conn) => {
                    self.lan_guest_joined(&mut host, conn, state, entities, spawn);
                    if self.options.dedicated.is_some()
                        && let Some(g) = host.hub.guest(conn)
                    {
                        // PlayerList.placeNewPlayer, then the broadcast join message.
                        super::dedicated::player_joined(g.uuid, &g.name);
                        super::dedicated::events::record(
                            "join",
                            &g.name,
                            g.uuid,
                            serde_json::json!({ "address": g.address.ip().to_string() }),
                        );
                        let [x, y, z] = g.position;
                        super::dedicated::log(
                            "INFO",
                            &format!(
                                "{}[/{}] logged in with entity id {} at ({}, {}, {})",
                                g.name,
                                g.address,
                                g.entity_id,
                                super::dedicated::java_double(x),
                                super::dedicated::java_double(y),
                                super::dedicated::java_double(z)
                            ),
                        );
                        super::dedicated::log(
                            "INFO",
                            &format!("System chat: {} joined the game", g.name),
                        );
                    } else if self.player.is_some()
                        && let Some(g) = host.hub.guest(conn)
                        && g.placed
                    {
                        // The host's chat hears of the newcomer as the guests do.
                        let who = self.player_text(&g.name, g.uuid);
                        self.system_message(
                            Text::translate("multiplayer.player.joined", &[who])
                                .with_color(horizon_client::text::YELLOW),
                        );
                    }
                    if let Some((uuid, name)) = host
                        .hub
                        .guest(conn)
                        .filter(|g| g.placed)
                        .map(|g| (g.uuid, g.name.clone()))
                    {
                        self.lan = Some(host);
                        self.native_player_logged_in(uuid, &name);
                        // A hook's command can close the hub.
                        host = self.lan.take()?;
                    }
                }
                HostEvent::Left(mut guest) => {
                    // The menu they had open closes, in the level they were in
                    // (`ServerPlayer.disconnect` -> `doCloseContainer`).
                    if guest.placed {
                        let back = self.dim;
                        let theirs = super::Dim::from_id(&guest.dimension);
                        if self.enter_level(Some(&mut host), theirs).is_ok() {
                            let conn = guest.conn;
                            let at = host.hub.guests.len();
                            host.hub.guests.push(*guest);
                            self.guest_close_all(&mut host, conn);
                            *guest = host.hub.guests.remove(at);
                        }
                        if let Err(e) = self.enter_level(Some(&mut host), back) {
                            log::error!("could not reopen {}: {e}", back.id());
                        }
                    }
                    self.lan_crafting_left(&mut host, &mut guest);
                    if guest.placed {
                        host.award_leave_game(guest.conn);
                    }
                    // PlayerList.remove: the vehicle it alone rides goes too.
                    self.take_parent_vehicle(&mut host, &mut guest);
                    self.save_guest(&host, &mut guest);
                    self.guest_attachments.remove(&guest.uuid);
                    host.models.remove(&guest.conn);
                    host.play.remove(&guest.conn);
                    self.guest_combat.remove(&guest.conn);
                    if self.options.dedicated.is_some() && guest.placed {
                        super::dedicated::events::record(
                            "leave",
                            &guest.name,
                            guest.uuid,
                            serde_json::json!({ "reason": guest.left_reason }),
                        );
                        super::dedicated::log(
                            "INFO",
                            &format!("{} lost connection: {}", guest.name, guest.left_reason),
                        );
                        super::dedicated::log(
                            "INFO",
                            &format!("System chat: {} left the game", guest.name),
                        );
                    } else if self.player.is_some() && guest.placed {
                        // ServerGamePacketListenerImpl.removePlayerFromWorld.
                        let who = self.player_text(&guest.name, guest.uuid);
                        self.system_message(
                            Text::translate("multiplayer.player.left", &[who])
                                .with_color(horizon_client::text::YELLOW),
                        );
                    }
                    if guest.placed && !host.players_told_left {
                        self.lan = Some(host);
                        self.native_player_logged_out(guest.uuid, &guest.name);
                        host = self.lan.take()?;
                    }
                }
                HostEvent::Flight { conn, flying } => {
                    self.lan_flight_toggled(&mut host, conn, flying);
                }
                HostEvent::Packet(conn, packet) => {
                    self.lan_guest_packet(&mut host, conn, *packet);
                    if !host.commands.is_empty() {
                        self.lan = Some(host);
                        self.run_guest_commands();
                        host = self.lan.take().expect("the hub is back after the commands");
                    }
                }
            }
        }
        Some(host)
    }

    /// Handle this level's player packets before scheduled block ticks
    /// (`ServerGamePacketListenerImpl` ahead of `ServerLevel`'s block phase).
    /// A sand place in the same tick as another sand's fall schedule still
    /// sees the support (#4510). `tick_lan` then syncs the level; a paused
    /// tick that never calls this still drains inside `tick_lan`.
    pub(super) fn tick_lan_packets(&mut self) {
        if self.level_pass.packets_drained {
            return;
        }
        let Some(mut host) = self.lan.take() else {
            return;
        };
        let holding = lan::Holding::start();
        let state = self.lan_host_state(&host);
        if state.is_none() && !host.hub.config.dedicated {
            self.lan = Some(host);
            self.level_pass.packets_drained = true;
            return;
        }
        // Last tick's tracked entities: enough for place collision and joins
        // before this tick moves anyone.
        let entities = std::mem::take(&mut host.states);
        let spawn = self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]);
        let events = {
            let clock_rate = self.network_clock_rate();
            let mut ctx = TickCtx {
                world: &mut self.world,
                host: state.as_ref(),
                entities: entities.as_slice(),
                next_entity: &mut self.next_entity,
                spawn,
                difficulty: i32::from(self.ui.difficulty),
                difficulty_locked: self.level.difficulty_locked,
                clock_rate,
                tick_rate: self.clock.rate(),
                tick_frozen: self.clock.frozen(),
                tick_steps: self.clock.step_state(),
                collision: Some(self.collision),
                border: &self.border,
                streaming: Some(&self.streaming),
            };
            host.hub.movement_checks = (
                self.ui.rules.bool("player_movement_check"),
                self.ui.rules.bool("elytra_movement_check"),
            );
            let pass = self.level_pass;
            let mut events = if pass.first {
                host.hub.poll(&mut ctx)
            } else {
                Vec::new()
            };
            events.extend(host.hub.drain_packets(&mut ctx));
            events
        };
        let Some(back) =
            self.lan_process_events(host, events, state.as_ref(), entities.as_slice(), spawn)
        else {
            return;
        };
        host = back;
        if !host.quiet.is_empty() {
            self.lan = Some(host);
            self.run_quiet_commands();
            host = self
                .lan
                .take()
                .expect("the hub is back after the quiet commands");
        }
        if !host.hub.native_menu_events.is_empty() {
            self.lan = Some(host);
            self.run_native_menu_events();
            host = self
                .lan
                .take()
                .expect("the hub is back after the menu events");
        }
        host.states = entities;
        self.level_pass.packets_drained = true;
        drop(holding);
        self.lan = Some(host);
    }

    /// Run the hub for this tick and apply what the guests did. While the
    /// single player game is `paused` only the connections run
    /// (`IntegratedServer.tickPaused`): `ServerGamePacketListenerImpl.tick`
    /// skips `tickPlayer`, so no player's air, hunger or damage moves on.
    pub(super) fn tick_lan(&mut self, paused: bool) {
        // A frozen level does not reach tick_lan_packets through tick_world.
        // Apply its packets before taking the snapshot too.
        if !paused {
            self.tick_lan_packets();
        }
        let Some(mut host) = self.lan.take() else {
            if self.level_pass.running.is_none() {
                self.level_pass.packets_drained = false;
            }
            return;
        };
        // What the tick sends a guest goes to its connection at the end.
        let holding = lan::Holding::start();
        self.register_native_commands(&mut host);
        // The host's own commands' (and command blocks') players moving
        // to another level, and their broadcastToAdmins lines.
        if !self.level_teleports.is_empty() {
            self.run_level_teleports(&mut host);
        }
        if !self.admin_feedback.is_empty() {
            self.send_admin_feedback(&mut host, None);
        }
        let state = self.lan_host_state(&host);
        // An animal whose rider left the game (or the level) stops taking
        // its input.
        for m in &mut self.mobs {
            if m.rider.is_some()
                && self.riding != Some(m.id)
                && !host.hub.guests.iter().any(|g| g.vehicle == Some(m.id))
            {
                m.rider = None;
            }
        }
        // A guest whose file names the vehicle it rode rides it again.
        self.restore_guest_vehicles(&mut host);
        // Finish the players' mining, item use and pickups before describing
        // this tick to their clients. Otherwise a newly broken block and its
        // drops remain absent from the snapshot until the following tick.
        if !paused {
            self.tick_guests(&mut host);
        }
        // A dedicated server runs the hub without a host player.
        if state.is_some() || host.hub.config.dedicated {
            let has_guests = !host.hub.guests.is_empty();
            let mut states = std::mem::take(&mut host.states);
            if has_guests {
                let host_entity = state.as_ref().map(|s| s.entity_id);
                self.lan_assign_ids();
                let guests: Vec<(u32, i32)> = host
                    .hub
                    .guests
                    .iter()
                    .filter_map(|g| Some((g.vehicle?, g.entity_id)))
                    .collect();
                self.lan_riding(host_entity, &guests, &mut host.hub.riding);
                self.lan_entities(host_entity, &host.mob_players, &mut host.hub, &mut states);
                self.lan_hook_states(host_entity, &host.play, &host.hub, &mut states);
                self.lan_entity_uuids(&mut states);
            } else {
                states.clear();
            }
            // ServerEntity.sendChanges clears needsSync once it has looked.
            for m in &mut self.mobs {
                m.needs_sync = false;
            }
            for p in &mut self.creatures.projectiles {
                p.needs_sync = false;
            }
            for d in &mut self.dropped {
                d.needs_sync = false;
            }
            // The step command reaches clients before the events its tick
            // generated, as ServerTickRateManager.updateStepTicks does.
            host.hub.sync_ticking(
                self.clock.rate(),
                self.clock.frozen(),
                self.clock.step_state(),
            );
            // `runBlockEvents`' broadcast precedes the tick's block changes
            // (`ChunkHolder.broadcastChanges` in `tick_level` below): the
            // guest re-runs a piston's `triggerEvent` while its level still
            // has the blocks as they were before the move.
            for (pos, b0, b1, state) in std::mem::take(&mut self.lan_block_events) {
                host.hub.block_event(pos, b0, b1, state);
            }
            let entities = states.as_slice();
            // An entity that left the level since the last tick (a snowball
            // that hit) is dropped by the tracker below: its event goes out
            // first, as `Level.broadcastEntityEvent` precedes the discard.
            let (gone, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.lan_entity_events)
                .into_iter()
                .partition(|(id, ..)| !entities.iter().any(|e| e.id == *id));
            self.lan_entity_events = later;
            for (id, event, at) in gone {
                host.hub.entity_event(id, event, at);
            }
            host.hub.far_level(&self.save);
            let spawn = self.spawn.map_or([0.5, 80.0, 0.5], |s| [s.x, s.y, s.z]);
            let events = {
                let clock_rate = self.network_clock_rate();
                let mut ctx = TickCtx {
                    world: &mut self.world,
                    host: state.as_ref(),
                    entities,
                    next_entity: &mut self.next_entity,
                    spawn,
                    difficulty: i32::from(self.ui.difficulty),
                    difficulty_locked: self.level.difficulty_locked,
                    clock_rate,
                    tick_rate: self.clock.rate(),
                    tick_frozen: self.clock.frozen(),
                    tick_steps: self.clock.step_state(),
                    collision: Some(self.collision),
                    border: &self.border,
                    streaming: Some(&self.streaming),
                };
                host.hub.movement_checks = (
                    self.ui.rules.bool("player_movement_check"),
                    self.ui.rules.bool("elytra_movement_check"),
                );
                // IntegratedServer.getScaledTrackingDistance: the Entity Distance
                // option scales what an integrated server or a LAN host tracks (a
                // dedicated one keeps its entity-broadcast-range-percentage).
                if host.hub.config.integrated || !host.hub.config.dedicated {
                    host.hub.entity_range_percent =
                        (self.settings.entity_distance * 100.0).round() as i32;
                }
                host.hub
                    .set_immediate_respawn(self.ui.rules.bool("immediate_respawn"));
                // A server with several levels runs a pass for each
                // (`tick_server`): the network is read once, before the first.
                // Player packets may already have been drained before
                // scheduled block ticks (`tick_lan_packets`).
                let pass = self.level_pass;
                let mut events = if pass.first && !pass.packets_drained {
                    host.hub.poll(&mut ctx)
                } else {
                    Vec::new()
                };
                if !pass.packets_drained {
                    events.extend(host.hub.drain_packets(&mut ctx));
                }
                events.extend(host.hub.tick_level(&mut ctx));
                if pass.last {
                    host.hub.end_tick();
                }
                events
            };
            if let Some(state) = &state
                && host.host_mode != state.game_mode
            {
                host.host_mode = state.game_mode;
                host.hub.set_host_game_mode(state.uuid, state.game_mode);
            }
            self.wake_woken_guests(&mut host);
            let Some(back) = self.lan_process_events(host, events, state.as_ref(), entities, spawn)
            else {
                return;
            };
            host = back;
            host.states = states;
        }
        if !host.quiet.is_empty() {
            self.lan = Some(host);
            self.run_quiet_commands();
            host = self
                .lan
                .take()
                .expect("the hub is back after the quiet commands");
        }
        if self.native_net_waiting() {
            self.lan = Some(host);
            self.run_native_net();
            host = self
                .lan
                .take()
                .expect("the hub is back after the mod messages");
        }
        if !host.hub.native_menu_events.is_empty() {
            self.lan = Some(host);
            self.run_native_menu_events();
            host = self
                .lan
                .take()
                .expect("the hub is back after the menu events");
        }
        host.hub.collect_tracking();
        if !host.hub.native_tracking.is_empty() {
            self.lan = Some(host);
            self.run_native_tracking();
            host = self
                .lan
                .take()
                .expect("the hub is back after the tracking events");
        }
        self.lan = Some(host);
        // A blast from a guest's attack, and a mace's wind burst.
        self.flush_lan_explosions();
        let _ = self.flush_wind_bursts();
        // A sweep attack's reach to the other players from a guest's attack.
        self.flush_sweeps();
        // A reeled-in hook pulling a player.
        self.flush_hook_pulls();
        // Particles a guest's turn sent (sweep attack, damage indicator, a hook's splashes).
        self.flush_lan_particles();
        let mut host = self
            .lan
            .take()
            .expect("the hub is back after the guest flushes");
        self.lan_flush_effects(&mut host);
        self.lan_living_flags(&mut host);
        self.lan_stuck(&mut host, paused);
        self.tick_server_travel(&mut host);
        self.lan_swings(&mut host);
        self.lan_update_streaming(&host);
        self.lan_update_models(&mut host);
        self.lan_update_waypoints(&mut host, state.as_ref());
        self.lan_sync_scoreboard(&mut host);
        self.lan_sync_border(&mut host);
        host.hub.anticheat_ping();
        drop(holding);
        self.flush_native_net(&mut host);
        self.flush_far_terrain(&mut host);
        lan::flush_all(host.hub.guests.iter().map(|g| &g.sender));
        self.lan = Some(host);
        self.flush_native_sounds();
        // `tick_server` resets the pass after all dimensions. The deprecated
        // in-frame path has no outer pass, so make its packet gate per-tick.
        if self.level_pass.running.is_none() {
            self.level_pass.packets_drained = false;
        }
    }

    fn lan_guest_joined(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        state: Option<&HostState>,
        entities: &[EntityState],
        spawn: [f64; 3],
    ) {
        let default_mode = host.hub.config.default_game_mode;
        let force = host.hub.config.force_game_mode;
        let root = self.world_root.clone();
        let in_place = host.hub.config.dimension.clone();
        let owner = host.owner.as_mut();
        let Some(g) = host.hub.guest_mut(conn) else {
            return;
        };
        // PlayerList.loadPlayerData: the owner gets the world's player file,
        // everyone else their own.
        let known = match owner.filter(|o| o.uuid == g.uuid) {
            Some(o) => playerdata::load_owner(g, &root, o.saved.take().as_deref()),
            None => playerdata::load(g, &root),
        };
        self.guest_attachments
            .insert(g.uuid, std::mem::take(&mut g.attachments));
        if !known || force {
            g.game_mode = default_mode;
        }
        if !known {
            // ServerPlayer.adjustSpawnLocation: a player new to the world
            // stands where PlayerSpawnFinder puts them, not all on one spot
            // (it reads the loaded level, which holds the spawn when it is
            // the overworld).
            let feet = if self.dim == super::Dim::Overworld {
                self.server_player_spawn(spawn)
            } else {
                spawn
            };
            g.position = feet;
            g.previous_position = feet;
            g.health = 20.0;
            g.food = 20;
            g.dimension = super::Dim::Overworld.id().to_owned();
        }
        // A server places each player in their own level
        // (`PlayerList.placeNewPlayer`); the in-frame game has one.
        let back = self.dim;
        let level = super::Dim::from_id(&g.dimension);
        let elsewhere = self.options.dedicated.is_some() && level != back;
        if !elsewhere {
            g.dimension = in_place;
        }
        self.load_guest_records(host, conn);
        let mut theirs = States::default();
        let entities = if elsewhere {
            if let Err(e) = self.enter_level(Some(host), level) {
                log::error!("could not open {}: {e}", level.id());
                return;
            }
            // A dedicated server has no host player.
            self.lan_entities(None, &host.mob_players, &mut host.hub, &mut theirs);
            self.lan_hook_states(None, &host.play, &host.hub, &mut theirs);
            self.lan_entity_uuids(&mut theirs);
            theirs.as_slice()
        } else {
            entities
        };
        let clock_rate = self.network_clock_rate();
        let mut ctx = TickCtx {
            world: &mut self.world,
            host: state,
            entities,
            next_entity: &mut self.next_entity,
            spawn,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
            streaming: Some(&self.streaming),
        };
        host.hub.place_new_player(conn, &mut ctx);
        if elsewhere && let Err(e) = self.enter_level(Some(host), back) {
            log::error!("could not reopen {}: {e}", back.id());
        }
    }

    /// Level events the game queued, for the guests (`process_level_events`).
    pub(super) fn lan_record_events(&mut self, events: &[horizon_world::level::LevelEvent]) {
        if self.audio.capture.is_none() {
            return;
        }
        // The acting guest's own break: `Block.playerWillDestroy` excludes the
        // player from the destroy events up to the broken block's. What a
        // break sets off after that (a torch popping off, a stalk falling) is
        // `destroyBlock` with no player, which the breaker hears and sees too.
        let mut own_break = self.lan_break_pos.is_some();
        for e in events {
            match e {
                horizon_world::level::LevelEvent::LevelEvent { id, pos, data } => {
                    let own = *id == 2001 && own_break;
                    if own && self.lan_break_pos == Some([pos.x, pos.y, pos.z]) {
                        own_break = false;
                    }
                    // The acting guest does not predict the egg's teleport
                    // particles, bone meal's growth, anvil result sounds, or a
                    // finished brushing or an eye inserted in a portal frame (these calls name no
                    // excluded player), nor the digger's own crack and hit
                    // sound (2019, 2020).
                    let except = if matches!(
                        *id,
                        2015 | 1503 | 1505 | 1029 | 1030 | 1045 | 1052 | 2019 | 2020 | 3000 | 3008
                    ) || (*id == 2001 && self.lan_break_pos.is_some() && !own)
                    {
                        None
                    } else {
                        self.lan_acting
                    };
                    self.lan_events
                        .push((*id, [pos.x, pos.y, pos.z], *data, except));
                }
                // `ServerLevel.runBlockEvents` broadcasts a handled block
                // event (`ClientboundBlockEventPacket`): the guest re-runs
                // the block's `triggerEvent` — a piston's `moveBlocks` is
                // what animates the move there. `state` is the block the
                // event named, read before it ran (the packet's `block`).
                horizon_world::level::LevelEvent::BlockEntityEvent { pos, state, a, b } => {
                    self.lan_block_events
                        .push(([pos.x, pos.y, pos.z], *a, *b, state.0));
                }
                _ => {}
            }
        }
    }

    /// `Level.blockEvent` for the guests (`ClientboundBlockEventPacket`).
    pub(super) fn lan_block_event(&mut self, pos: [i32; 3], b0: i32, b1: i32) {
        if self.audio.capture.is_some() {
            let state = self.world.block(pos[0], pos[1], pos[2]).0;
            self.lan_block_events.push((pos, b0, b1, state));
        }
    }
    /// `Level.broadcastEntityEvent` for the guests.
    pub(super) fn lan_entity_event(&mut self, id: i32, event: i8, at: [f64; 3]) {
        if self.audio.capture.is_some() {
            self.lan_entity_events.push((id, event, at));
        }
        // Event 21: a window that simulates the world itself is its own
        // client, and hears the guardian's hum too.
        if event == 21 && self.options.execution_role.is_client() {
            self.start_guardian_sound(id as u32);
        }
        // Event 53: a mob sliding down a honey block sheds honey particles
        // (HoneyBlock.showSlideParticles), drawn by the client that runs the world.
        if event == 53 && self.options.execution_role.is_client() {
            self.honey_slide_particles(at);
        }
        // Event 2: a mob's spear hit felt; the same window hears its own mob's hit.
        if event == 2 && self.options.execution_role.is_client() {
            self.remote_kinetic_hit(id);
        }
        // Fox.handleEntityEvent 45: the same window draws the eating crumbs.
        if event == 45 && self.options.execution_role.is_client() {
            self.fox_eat_particles(id as u32);
        }
        // Entity event 35: a mob's death protection fired
        // (`LivingEntity.checkTotemDeathProtection`); the clients draw the
        // totem particles and play its sound.
        if event == 35 && self.options.execution_role.is_client() {
            self.remote_totem(id);
        }
    }
    /// `LivingEntity.onEquippedItemBroken` for the acting player (the guest whose turn it
    /// is, else the host): `level.broadcastEntityEvent` with `entityEventForEquipmentBreak`
    /// of the slot, which each client answers with `breakItem` (the sound and the crumbs,
    /// `remote_equipment_break`). Without a hub to tell, or in a window that runs the world
    /// itself, the sound and the crumbs are played here.
    pub(super) fn player_equipment_broke(
        &mut self,
        slot: horizon_world::player_inventory::EquipmentSlot,
        stack: &horizon_client::items::ItemStack,
    ) {
        use horizon_world::player_inventory::EquipmentSlot;
        let event = match slot {
            EquipmentSlot::MainHand => 47,
            EquipmentSlot::OffHand => 48,
            EquipmentSlot::Head => 49,
            EquipmentSlot::Chest => 50,
            EquipmentSlot::Legs => 51,
            EquipmentSlot::Feet => 52,
            EquipmentSlot::Body => 65,
            EquipmentSlot::Saddle => 68,
        };
        let Some(at) = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
        else {
            return;
        };
        let told = if self.lan_acting.is_some() {
            self.guest_equipment_breaks.push((event, at));
            true
        } else if let Some(host) = &self.lan {
            host.hub.entity_event(host.host_entity, event, at);
            true
        } else {
            false
        };
        if told && !self.options.execution_role.is_client() {
            return;
        }
        // LivingEntity.breakItem: the stack's break sound where it stands, then the crumbs.
        self.break_sound_at(stack, at);
        self.eat_particles(stack, 5);
    }
    /// `ServerPlayer.crit` (or `magicCrit` when `magic`): this window's
    /// emitter, and an animation for the attacker's client and observers once
    /// the hub is back in place.
    pub(super) fn critical_hit(
        &mut self,
        attacker: Option<u32>,
        id: i32,
        feet: [f64; 3],
        width: f64,
        height: f64,
        magic: bool,
    ) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let particle = if magic {
            ParticleType::EnchantedHit
        } else {
            ParticleType::Crit
        };
        self.particles
            .tracking_emitter(ParticleOptions::simple(particle), feet, width, height);
        if self.audio.capture.is_some() {
            self.lan_critical_hits.push((attacker, id, magic));
        }
    }
    /// Send the tick's level events, sounds and mob animations.
    fn lan_flush_effects(&mut self, host: &mut LanHost) {
        let critical_hits = std::mem::take(&mut self.lan_critical_hits);
        let kinetic_hits = std::mem::take(&mut self.lan_kinetic_hits);
        for (state, at) in std::mem::take(&mut self.lan_cushion_breaks) {
            if let Some(id) = host
                .hub
                .registry_id("minecraft:particle_type", "minecraft:block")
            {
                host.hub.broadcast_near(
                    at,
                    32.0,
                    &C::LevelParticles(play::LevelParticles {
                        particle: horizon_protocol::entity_data::Particle {
                            id,
                            data: Some(horizon_protocol::entity_data::ParticleData::State(
                                state.0 as i32,
                            )),
                        },
                        override_limiter: false,
                        always_show: false,
                        pos: at,
                        dist: [0.25, 0.0625, 0.25],
                        max_speed: [0.05; 3],
                        count: 10,
                        randomization: 0,
                    }),
                    None,
                );
            }
        }
        for (kind, pos, data, acting) in std::mem::take(&mut self.lan_events) {
            if kind == 2001 {
                host.broken
                    .push([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]);
            }
            host.hub.level_event(kind, pos, data, acting);
        }
        // Level events 1010 and 1011: the guests' parrots dance or stop.
        for (pos, song) in std::mem::take(&mut self.lan_songs) {
            match song {
                Some(name) => {
                    let id = host.hub.registry_id("minecraft:jukebox_song", &name);
                    host.hub.level_event(1010, pos, id.unwrap_or(0), None);
                }
                None => host.hub.level_event(1011, pos, 0, None),
            }
        }
        for (pos, b0, b1, state) in std::mem::take(&mut self.lan_block_events) {
            host.hub.block_event(pos, b0, b1, state);
        }
        for (id, event, at) in std::mem::take(&mut self.lan_entity_events) {
            host.hub.entity_event(id, event, at);
        }
        for (kind, pos, data) in std::mem::take(&mut self.lan_global_events) {
            host.hub.global_level_event(kind, pos, data);
        }
        self.lan_broadcast_effects(host);
        if host.hub.guests.is_empty() {
            host.mob_state.clear();
            return;
        }
        // Hurt and death animations of mobs.
        let mut seen = HashMap::new();
        for m in &self.mobs {
            let dead = !m.alive();
            let old = host
                .mob_state
                .get(&self.dim)
                .and_then(|s| s.get(&m.id))
                .copied();
            let at = [m.position.x, m.position.y, m.position.z];
            // LivingEntity.hurtServer: a hurt that restarted the hurt time.
            if m.hurt_time > old.map_or(0, |o| o.0)
                && !dead
                && let Some((source, _)) = m.last_damage
            {
                use horizon_client::mobs::{DamageKind as K, Who};
                let kind = super::creatures::source_damage_type(source);
                // The player a mob sees is the guest acting, else the host.
                let cause = source.attacker.map(|who| match who {
                    Who::Player => host
                        .mob_players
                        .get(&m.id)
                        .and_then(|conn| host.hub.guest(*conn))
                        .map_or(host.host_entity, |g| g.entity_id),
                    Who::Mob(id) => id as i32,
                });
                // A projectile is the direct entity of its hit: not tracked here.
                let melee = matches!(
                    source.kind,
                    K::PlayerAttack
                        | K::MaceSmash
                        | K::Spear
                        | K::MobAttack
                        | K::Sting
                        | K::SonicBoom
                );
                let direct = cause.filter(|_| melee);
                host.hub.damage_event(m.id as i32, kind, cause, direct, at);
            }
            if dead && !old.is_some_and(|o| o.1) {
                host.hub.entity_event(m.id as i32, 3, at);
            }
            // LivingEntity.swing on a server (a melee hit, a thrown potion):
            // ClientboundSwingAnimationPacket for the players around. A swing
            // started since the last flush, or restarted (its time went back).
            let swing = m.ai.swinging.then_some(m.ai.swing_time);
            if let Some(time) = swing
                && !dead
                && old.and_then(|o| o.2).is_none_or(|before| time < before)
            {
                host.hub.swing(m.id as i32, 0, at, None);
            }
            seen.insert(m.id, (m.hurt_time, dead, swing));
        }
        if let Some(d) = self.dragon.as_ref().filter(|d| d.id != 0) {
            let id = d.id;
            let dead = !d.alive();
            let old = host
                .mob_state
                .get(&self.dim)
                .and_then(|s| s.get(&id))
                .copied();
            let at = [d.position.x, d.position.y, d.position.z];
            if d.hurt_time > old.map_or(0, |o| o.0) && !dead {
                let (kind, cause, direct) =
                    d.last_source.unwrap_or(("minecraft:generic", None, None));
                host.hub.damage_event(id as i32, kind, cause, direct, at);
            }
            seen.insert(id, (d.hurt_time, dead, None));
        }
        // LivingEntity.hurtServer for the in-frame LAN host's player (guests
        // broadcast directly from HealthEvent::DamageBroadcast): a hit that
        // restarted the hurt time and was not blocked by a shield broadcasts
        // the damage event, so the clients around flash it red.
        if let Some(p) = self.player.as_ref() {
            let id = host.host_entity;
            let at = [p.position.x, p.position.y, p.position.z];
            let hurt = p.health.hurt_animation_time;
            let dead = p.health.dead_or_dying();
            let old = host
                .mob_state
                .get(&self.dim)
                .and_then(|s| s.get(&(id as u32)));
            if hurt > old.map_or(0, |o| o.0) && !dead {
                let kind = p
                    .health
                    .hurt
                    .last_source
                    .and_then(|s| self.motion.health.damage.definition(s))
                    .map_or("minecraft:generic", |d| d.name.as_str());
                host.hub.damage_event(id, kind, None, None, at);
            }
            seen.insert(id as u32, (hurt, dead, None));
        }
        host.mob_state.insert(self.dim, seen);
        // Level.broadcastEntityEvent(player, 2) of a spear's stab.
        for attacker in kinetic_hits {
            let hit = match attacker {
                None => self
                    .player
                    .as_ref()
                    .map(|p| (host.host_entity, [p.position.x, p.position.y, p.position.z])),
                Some(conn) => host.hub.guest(conn).map(|g| (g.entity_id, g.position)),
            };
            if let Some((id, at)) = hit {
                host.hub.entity_event(id, 2, at);
            }
        }
        for (attacker, target, magic) in critical_hits {
            let Some(attacker) = attacker.map_or(Some(host.host_entity), |conn| {
                host.hub.guest(conn).map(|g| g.entity_id)
            }) else {
                continue;
            };
            host.hub.critical_hit(attacker, target, magic);
        }
    }

    /// Keep the chunks around every guest loaded.
    fn lan_update_streaming(&mut self, host: &LanHost) {
        // DistanceManager.PlayerTicketTracker: every chunk within the server's
        // view distance (Chebyshev) of a player holds a PLAYER_LOADING ticket
        // at the entity-ticking level (31), whatever the player asked for;
        // levels rise by one a chunk, so FULL (33) reaches two chunks past
        // it. An extra area finishes its chunks out to its radius + 1. That
        // covers what the player tracks (`ChunkTrackingView`, within its view
        // distance + 1) and those chunks' lit neighbours.
        let radius = host.hub.config.view_distance + 1;
        let rule = self.ui.rules.bool("spectators_generate_chunks");
        self.streaming.extra = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && super::game_mode::loads_chunks(rule, g.spectator()))
            .map(|g| (g.chunk(), radius))
            .chain(
                self.arrival_chunks(self.dim)
                    .into_iter()
                    // An arrival keeps only its landing spot lit enough to
                    // resolve (the 3x3 `resolve_arrivals` checks), where it
                    // then ticks without a player.
                    .map(|chunk| (chunk, 1)),
            )
            // A portal ticket keeps the area around the exit loaded and
            // ticking (its entity-ticking chunks and their neighbours).
            .chain(
                self.portal_ticket_chunks(self.dim)
                    .into_iter()
                    .map(|chunk| (chunk, 1)),
            )
            .collect();
    }

    /// The guests' models, for the host to see them.
    fn lan_update_models(&mut self, host: &mut LanHost) {
        let Some(kind) = self.res.player_type().clone() else {
            return;
        };
        host.models
            .retain(|conn, _| host.hub.guest(*conn).is_some_and(|g| g.placed));
        for g in host.hub.guests.iter().filter(|g| g.placed) {
            let at = Vec3::new(
                g.position[0],
                g.position[1] - if g.sneaking { 0.125 } else { 0.0 },
                g.position[2],
            );
            let m = host
                .models
                .entry(g.conn)
                .or_insert_with(|| Mob::new(g.entity_id as u32, kind.clone(), at, g.yaw));
            let (dx, dy, dz) = (
                at.x - m.position.x,
                at.y - m.position.y,
                at.z - m.position.z,
            );
            m.previous = m.position;
            m.position = at;
            if dx * dx + dy * dy + dz * dz > 100.0 {
                m.previous = at;
            }
            m.previous_yaw = m.yaw;
            m.previous_body_yaw = m.body_yaw;
            m.previous_head_yaw = m.head_yaw;
            m.previous_head_pitch = m.head_pitch;
            m.yaw = g.yaw;
            m.head_yaw = g.head_yaw;
            m.head_pitch = g.pitch;
            let mut target = m.body_yaw;
            let wrap = |a: f32| (a + 180.0).rem_euclid(360.0) - 180.0;
            if dx * dx + dz * dz > 0.002_500_000_2 {
                // LivingEntity.tick: walking backwards the body faces away
                // from the walk direction, not along it.
                let walk = (dz.atan2(dx) as f32).to_degrees() - 90.0;
                let facing = (wrap(g.yaw) - walk).abs();
                target = if 95.0 < facing && facing < 265.0 {
                    walk - 180.0
                } else {
                    walk
                };
            }
            m.body_yaw += wrap(target - m.body_yaw) * 0.3;
            let diff = wrap(g.yaw - m.body_yaw);
            if diff.abs() > 50.0 {
                m.body_yaw += diff - diff.signum() * 50.0;
            }
            let dist = (dx * dx + dz * dz).sqrt() as f32;
            m.previous_limb_speed = m.limb_speed;
            m.limb_speed += ((dist * 4.0).min(1.0) - m.limb_speed) * 0.4;
            m.limb_position += m.limb_speed;
            m.age = m.age.wrapping_add(1);
            // The hurt red flash (LivingEntity.hurtTime).
            m.hurt_time = g
                .motion
                .as_ref()
                .map_or(0, |p| p.health.hurt_animation_time);
            m.extra
                .set_main_hand_stack(g.inventory.get(g.selected).cloned().flatten());
            m.extra.off_hand = g.offhand.as_ref().map(|s| s.item.clone());
            [m.extra.head, m.extra.chest, m.extra.legs, m.extra.feet] = g
                .armor
                .each_ref()
                .map(|s| s.as_ref().map(|s| s.item.clone()));
            m.extra.armor = g.armor.clone();
            m.extra.crouching = g.sneaking;
            m.extra.avatar_pose = g.pose();
            m.extra.sprinting = g.sprinting;
            // Entity.DATA_POSE and the shared and living flags the others
            // get (hub::guest_body): gliding, swimming, Riptide's spin.
            let body = lan::hub::guest_body(g);
            m.extra.fall_flying = body.gliding;
            m.extra.synced_pose = if body.swimming || body.crawling { 3 } else { 0 };
            m.extra.auto_spin_attack = body.spin_attack;
            m.extra.previous_velocity = m.velocity;
            (m.extra.arrow_count, m.extra.stinger_count) = g.stuck;
            m.extra.shoulder_parrots = g.shoulders;
            m.velocity = g
                .motion
                .as_ref()
                .map_or(Vec3::new(dx, dy, dz), |motion| motion.velocity);
            m.tick_avatar_pose(
                g.motion
                    .as_ref()
                    .is_some_and(|motion| motion.fluids.was_touching_water),
            );
            // CrossbowItem.isCharged: the held crossbow's projectiles.
            m.extra.charged = g
                .inventory
                .get(g.selected)
                .and_then(|s| s.as_ref())
                .is_some_and(|s| s.data().is_some_and(|d| !d.charged.is_empty()));
            // ClientInformation.mainHand (HumanoidArm: 0 left, 1 right).
            m.extra.main_hand_left = g.info.main_hand == 0;
            m.extra.hidden_model_parts = !g.info.model_customisation & 0x7F;
            m.extra.spectator = g.game_mode == horizon_client::lan::guest::SPECTATOR;
            // LivingEntity.DATA_EFFECT_PARTICLES: the swirls of its visible effects.
            let visible = || g.effects.list.iter().filter(|e| e.visible);
            m.extra.effect_particles = visible()
                .filter_map(|e| super::client_ambient::effect_particle(e.id, e.ambient))
                .collect();
            m.extra.effect_ambient = visible().all(|e| e.ambient);
            // Entity.isInvisible: the invisibility effect hides the body (the
            // model pass, the name tag rules and `trader::is_invisible` read it).
            m.extra.effects.retain(|e| e.0 != "minecraft:invisibility");
            if let Some(e) = g.effects.get("invisibility") {
                m.extra
                    .effects
                    .push(("minecraft:invisibility", e.duration, e.amplifier));
            }
            // Lying in its bed (LivingEntity.getBedOrientation).
            m.extra.ints[1] = g.bed.map_or(0, |b| self.bed_facing(b));
            // LivingEntity.updateSwingTime: the arm swing lan_swings started.
            m.anim.attack_anim_o = m.anim.attack_anim;
            horizon_client::mobs::ai::tick_swing(m);
            // LivingEntity.onSyncedDataUpdated(DATA_LIVING_ENTITY_FLAGS): the
            // item in use and its hand (the arm poses of a bow, a shield...).
            let using = host.play.get(&g.conn).and_then(|p| p.using());
            m.extra.using_item = using.map_or(0, |(_, ticks)| ticks as i32 + 1);
            m.extra.using_off_hand = using.is_some_and(|(slot, _)| slot == OFFHAND_SLOT);
            // AvatarRenderer.getArmPose: the used item's BLOCK animation (an
            // item with `blocks_attacks`) raises the arm.
            m.extra.blocking = using
                .and_then(|(slot, _)| {
                    if slot == OFFHAND_SLOT {
                        m.extra.off_hand.as_ref()
                    } else {
                        m.extra.main_hand.as_ref()
                    }
                })
                .and_then(|i| self.items().get(i))
                .is_some_and(|i| i.blocks_attacks.is_some());
        }
    }

    /// The bed at `head` as a sleeping model lies in it (`ints[1]`,
    /// `villager::sleep_pose`): its `facing`.
    pub(super) fn bed_facing(&self, head: [i32; 3]) -> i32 {
        let state = self.world.block(head[0], head[1], head[2]);
        let facing = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing"))
            .map(String::as_str);
        horizon_client::mobs::villager::bed_facing_code(facing)
    }

    /// The guests' models for the entity pass.
    pub(super) fn guest_model_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        let Some(host) = &self.lan else {
            return;
        };
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        for (conn, m) in &host.models {
            let entity_start = out.len();
            let probe = super::perspective::entity_light_block(
                m.position,
                m.extra.crouching,
                m.eye_height(),
            );
            let light = self.world.light(probe[0], probe[1], probe[2]);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            horizon_client::mobs::render::render_with(
                m,
                alpha as f32,
                camera,
                &lookup,
                packed,
                self.res.mob_data(),
                &host.hub.guest(*conn).map_or_else(
                    || self.worn_armor_layers(m),
                    |g| self.stack_armor_layers(&g.armor),
                ),
                out,
            );
            self.player_elytra_geometry(
                m,
                m.extra.chest.as_deref(),
                camera,
                alpha as f32,
                packed,
                out,
            );
            if let Some(g) = host.hub.guest(*conn) {
                self.player_ears_geometry(m, &g.name, camera, alpha as f32, packed, out);
            }
            self.spin_attack_geometry(m, camera, alpha as f32, packed, out);
            if let Some(g) = host.hub.guest(*conn) {
                self.stuck_in_body_geometry(m, g.entity_id, camera, alpha as f32, packed, out);
            }
            self.held_item_geometry(m, camera, alpha, packed, true, out);
            self.player_pumpkin_geometry(
                m,
                host.hub.guest(*conn).and_then(|g| g.armor[0].as_ref()),
                camera,
                alpha,
                packed,
                out,
            );
            // LivingEntity.isCurrentlyGlowing: the guest's Glowing effect.
            let guest = host.hub.guest(*conn);
            if m.glowing || guest.is_some_and(|g| g.effects.get("glowing").is_some()) {
                let team = guest.and_then(|g| self.score.board.players_team(&g.name));
                self.mark_outline(&out[entity_start..], super::glowing::team_color(team));
            } else {
                self.cull_entity_vertices(out, entity_start);
            }
        }
    }

    /// A chat line the host typed, for the guests.
    pub(super) fn lan_host_chat(&mut self, message: &str) {
        if let Some(host) = &self.lan {
            host.hub.chat(&host.host_name, message);
        }
    }
}

impl Game<'_> {
    /// A hub's chat and log lines: a dedicated server logs them
    /// (MinecraftServer.logChatMessage, `[Not Secure]` for unsigned or
    /// expired messages); a LAN host shows the chat.
    pub(super) fn lan_log_event(&mut self, event: HostEvent) {
        let dedicated = self.options.dedicated.is_some();
        match event {
            HostEvent::Chat { line, secure } if dedicated => {
                let tag = if secure { "" } else { "[Not Secure] " };
                super::dedicated::log("INFO", &format!("{tag}{line}"));
            }
            // Player chat (and `/say`'s or `/me`'s, which are chat messages
            // too): the host's Chat option decides, as `ChatListener` does
            // for the guests' packets. Command feedback is a system message
            // and takes the system-message permission instead.
            HostEvent::Chat { line, .. } => self.player_message(Text::literal(line)),
            HostEvent::Warn(line) if dedicated => super::dedicated::log("WARN", &line),
            HostEvent::Warn(line) => log::warn!("{line}"),
            HostEvent::Error(line) if dedicated => super::dedicated::log("ERROR", &line),
            HostEvent::Error(line) => log::error!("{line}"),
            _ => {}
        }
    }
}

/// `LivingEntity.DATA_LIVING_ENTITY_FLAGS` for an item in use: 1, and 2 when
/// it is the off hand's. A stop is 0 here; vanilla leaves bit 1 set, so its
/// stop of an off-hand use is 2. Both mean "not using" (bit 0 is clear), and
/// nothing reads bit 1 without bit 0.
pub(super) fn living_flags(using: Option<(usize, u32)>) -> i8 {
    using.map_or(0, |(slot, _)| if slot == OFFHAND_SLOT { 3 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::{OFFHAND_SLOT, living_flags};

    /// `LivingEntity.startUsingItem` sets flag 1 for the use and flag 2 for
    /// the off hand, so an off-hand use is 3, not 2. A reader takes bit 0 as
    /// `isUsingItem` and bit 1 as `getUsedItemHand`.
    #[test]
    fn an_off_hand_use_flags_both_the_use_and_the_hand() {
        assert_eq!(living_flags(None), 0);
        assert_eq!(living_flags(Some((0, 12))), 1);
        assert_eq!(living_flags(Some((OFFHAND_SLOT, 12))), 3);
    }
}
