//! The trial spawner (`TrialSpawner` as `TrialSpawnerBlockEntity` runs it):
//! its configs, saved state (`TrialSpawnerStateData`) and the server tick that
//! walks `TrialSpawnerState`: waiting for players, spawning its waves while
//! they fight, ejecting a reward per player, then cooling down.
//!
//! The state lives in the block (`trial_spawner_state`); the tick reads it and
//! reports the next one. The level's random drives the tick in vanilla's draw
//! order; what the spawner asks of the level goes through
//! [`TrialSpawnerLevel`], so the game and a differential test world answer the
//! same questions.
use super::{
    spawn_rules::{self, Reason},
    spawn_selection,
    spawner::{self, SpawnData, SpawnDataExt, SpawnerLevel, SpawnerSpawn},
};
pub use crate::chunk::block_entity::{ConfigSlot, TrialConfig, TrialSpawner};
use horizon_core::{position::BlockPos, random::RandomSource};
use horizon_world::nbt::{Compound, JavaString, Tag};
use std::{collections::HashMap, path::Path, sync::OnceLock};

/// An entity's UUID as its four ints (`UUIDUtil.CODEC`).
pub type Uuid = [i32; 4];

/// `MAX_MOB_TRACKING_DISTANCE`: a spawned mob further than this stops
/// counting as one of the spawner's.
const MAX_MOB_TRACKING_DISTANCE: i64 = 47;
/// `DELAY_BEFORE_EJECT_AFTER_KILLING_LAST_MOB` and `TIME_BETWEEN_EACH_EJECTION`.
const DELAY_BEFORE_EJECT: f32 = 40.0;
const TIME_BETWEEN_EJECTIONS: f32 = 30.0;
/// `TrialSpawnerConfig.ticksBetweenItemSpawners`.
const TICKS_BETWEEN_ITEM_SPAWNERS: i64 = 160;
/// `transformBadOmenIntoTrialOmen`: Trial Omen lasts this long per level of
/// Bad Omen.
pub const TRIAL_OMEN_TICKS_PER_LEVEL: i32 = 18000;

/// `TrialSpawnerConfig`'s codec and targets, over the data in `chunk::block_entity`.
pub trait TrialConfigExt: Sized {
    /// `TrialSpawnerConfig.DIRECT_CODEC`; None when the tag is not a
    /// compound. Read as `TagValueInput` does, keeping the partial result:
    /// a field that fails to decode takes its default, and the list entries
    /// that fail are dropped.
    fn decode(tag: &Tag) -> Option<Self>;
    /// `TrialSpawnerConfig.DIRECT_CODEC`'s encoding: the fields that differ
    /// from their defaults (`optionalFieldOf` with a default).
    fn encode(&self) -> Tag;
    /// `calculateTargetTotalMobs`.
    fn target_total_mobs(&self, additional_players: i32) -> i32;
    /// `calculateTargetSimultaneousMobs`.
    fn target_simultaneous_mobs(&self, additional_players: i32) -> i32;
}
impl TrialConfigExt for TrialConfig {
    fn decode(tag: &Tag) -> Option<Self> {
        let Tag::Compound(_) = tag else {
            return None;
        };
        let d = Self::default();
        let float = |k: &str, default: f32| {
            tag.get(k)
                .and_then(float)
                .filter(|v| *v >= 0.0)
                .unwrap_or(default)
        };
        let int = |k: &str, default: i32, lo: i32, hi: i32| {
            spawner::number(tag.get(k))
                .filter(|v| (i64::from(lo)..=i64::from(hi)).contains(v))
                .map_or(default, |v| v as i32)
        };
        let list = |k: &str| match tag.get(k) {
            Some(Tag::List(list)) => Some(list.iter()),
            _ => None,
        };
        let spawn_potentials = list("spawn_potentials").map_or_else(Vec::new, |l| {
            l.filter_map(|e| Some((SpawnData::decode(e.get("data")?)?, weight(e.get("weight"))?)))
                .collect()
        });
        let loot_tables_to_eject =
            list("loot_tables_to_eject").map_or(d.loot_tables_to_eject, |l| {
                l.filter_map(|e| Some((identifier(e.get("data"))?, weight(e.get("weight"))?)))
                    .collect()
            });
        Some(Self {
            spawn_range: int("spawn_range", d.spawn_range, 1, 128),
            total_mobs: float("total_mobs", d.total_mobs),
            simultaneous_mobs: float("simultaneous_mobs", d.simultaneous_mobs),
            total_mobs_added_per_player: float(
                "total_mobs_added_per_player",
                d.total_mobs_added_per_player,
            ),
            simultaneous_mobs_added_per_player: float(
                "simultaneous_mobs_added_per_player",
                d.simultaneous_mobs_added_per_player,
            ),
            ticks_between_spawn: int("ticks_between_spawn", d.ticks_between_spawn, 0, i32::MAX),
            spawn_potentials,
            loot_tables_to_eject,
            items_to_drop_when_ominous: identifier(tag.get("items_to_drop_when_ominous"))
                .unwrap_or(d.items_to_drop_when_ominous),
        })
    }
    fn encode(&self) -> Tag {
        let d = Self::default();
        let mut out = Vec::new();
        if self.spawn_range != d.spawn_range {
            out.push(("spawn_range", Tag::Int(self.spawn_range)));
        }
        for (key, value, default) in [
            ("total_mobs", self.total_mobs, d.total_mobs),
            (
                "simultaneous_mobs",
                self.simultaneous_mobs,
                d.simultaneous_mobs,
            ),
            (
                "total_mobs_added_per_player",
                self.total_mobs_added_per_player,
                d.total_mobs_added_per_player,
            ),
            (
                "simultaneous_mobs_added_per_player",
                self.simultaneous_mobs_added_per_player,
                d.simultaneous_mobs_added_per_player,
            ),
        ] {
            if value != default {
                out.push((key, Tag::Float(value.to_bits())));
            }
        }
        if self.ticks_between_spawn != d.ticks_between_spawn {
            out.push(("ticks_between_spawn", Tag::Int(self.ticks_between_spawn)));
        }
        let weighted = |data: Tag, weight: i32| {
            Tag::compound(vec![("data", data), ("weight", Tag::Int(weight))])
        };
        if !self.spawn_potentials.is_empty() {
            let list = self.spawn_potentials.iter();
            out.push((
                "spawn_potentials",
                Tag::List(list.map(|(data, w)| weighted(data.encode(), *w)).collect()),
            ));
        }
        if self.loot_tables_to_eject != d.loot_tables_to_eject {
            let list = self.loot_tables_to_eject.iter();
            out.push((
                "loot_tables_to_eject",
                Tag::List(
                    list.map(|(id, w)| weighted(Tag::String(id.as_str().into()), *w))
                        .collect(),
                ),
            ));
        }
        if self.items_to_drop_when_ominous != d.items_to_drop_when_ominous {
            out.push((
                "items_to_drop_when_ominous",
                Tag::String(self.items_to_drop_when_ominous.as_str().into()),
            ));
        }
        Tag::compound(out)
    }
    fn target_total_mobs(&self, additional_players: i32) -> i32 {
        (self.total_mobs + self.total_mobs_added_per_player * additional_players as f32).floor()
            as i32
    }
    fn target_simultaneous_mobs(&self, additional_players: i32) -> i32 {
        (self.simultaneous_mobs
            + self.simultaneous_mobs_added_per_player * additional_players as f32)
            .floor() as i32
    }
}

static REGISTRY: OnceLock<HashMap<String, TrialConfig>> = OnceLock::new();

/// `Registries.TRIAL_SPAWNER_CONFIG`: the configs under the resources'
/// `data/<namespace>/trial_spawner`, read once per process.
pub fn load_registry(root: &Path) {
    REGISTRY.get_or_init(|| {
        let mut out = HashMap::new();
        let Ok(namespaces) = std::fs::read_dir(root.join("data")) else {
            return out;
        };
        for ns in namespaces.flatten() {
            let base = ns.path().join("trial_spawner");
            let mut stack = vec![base.clone()];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    let (Ok(rel), Some(ns)) = (
                        path.with_extension("")
                            .strip_prefix(&base)
                            .map(Path::to_path_buf),
                        ns.file_name().to_str().map(str::to_owned),
                    ) else {
                        continue;
                    };
                    let config = std::fs::read(&path)
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .and_then(|v| TrialConfig::decode(&crate::scoreboard::json_to_tag(&v)));
                    match config {
                        Some(c) => {
                            let id = rel.to_string_lossy().replace('\\', "/");
                            out.insert(format!("{ns}:{id}"), c);
                        }
                        None => log::warn!("Horizon: bad trial spawner config {}", path.display()),
                    }
                }
            }
        }
        out
    });
}
/// A config of the registry by id.
pub fn registry_config(id: &str) -> Option<TrialConfig> {
    REGISTRY.get()?.get(id).cloned()
}

fn float(t: &Tag) -> Option<f32> {
    Some(match t {
        Tag::Float(b) => f32::from_bits(*b),
        Tag::Double(b) => f64::from_bits(*b) as f32,
        t => spawner::number(Some(t))? as f32,
    })
}
/// `Weighted.CODEC`'s weight: a non-negative int.
fn weight(t: Option<&Tag>) -> Option<i32> {
    spawner::number(t)
        .filter(|w| (0..=i64::from(i32::MAX)).contains(w))
        .map(|w| w as i32)
}
/// An `Identifier` (`minecraft` when the namespace is left out).
fn identifier(t: Option<&Tag>) -> Option<String> {
    match t? {
        Tag::String(s) => spawner::normalize_id(&s.to_string_lossy()),
        _ => None,
    }
}

/// `TrialSpawnerState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrialState {
    #[default]
    Inactive,
    WaitingForPlayers,
    Active,
    WaitingForRewardEjection,
    EjectingReward,
    Cooldown,
}
impl TrialState {
    pub const ALL: [Self; 6] = [
        Self::Inactive,
        Self::WaitingForPlayers,
        Self::Active,
        Self::WaitingForRewardEjection,
        Self::EjectingReward,
        Self::Cooldown,
    ];
    /// The `trial_spawner_state` block property.
    pub fn name(self) -> &'static str {
        match self {
            Self::Inactive => "inactive",
            Self::WaitingForPlayers => "waiting_for_players",
            Self::Active => "active",
            Self::WaitingForRewardEjection => "waiting_for_reward_ejection",
            Self::EjectingReward => "ejecting_reward",
            Self::Cooldown => "cooldown",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
    /// The spin speed of the mob shown inside, None when it shows none.
    pub fn spinning_mob_speed(self) -> Option<f64> {
        match self {
            Self::WaitingForPlayers => Some(200.0),
            Self::Active => Some(1000.0),
            _ => None,
        }
    }
    pub fn capable_of_spawning(self) -> bool {
        matches!(self, Self::WaitingForPlayers | Self::Active)
    }
}

/// A player the spawner can detect (`PlayerDetector.NO_CREATIVE_PLAYERS`).
#[derive(Clone, Copy, Debug)]
pub struct TrialPlayer {
    pub uuid: Uuid,
    pub block: [i32; 3],
    pub eye: [f64; 3],
    pub creative: bool,
    pub spectator: bool,
    /// `position()` and the box's height.
    pub position: [f64; 3],
    pub height: f32,
    pub alive: bool,
    pub omen: Option<Omen>,
}

/// The effect that turns a trial spawner ominous when its player is seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Omen {
    /// `MobEffects.TRIAL_OMEN`.
    Trial,
    /// `MobEffects.BAD_OMEN`, which becomes Trial Omen.
    Bad,
}

/// What a trial spawner did that the level shows or sends.
#[derive(Clone, Debug, PartialEq)]
pub enum TrialEvent {
    /// `markUpdated`: `sendBlockUpdated` with the new update tag.
    BlockUpdated,
    /// `levelEvent(id, pos, data)`.
    LevelEvent(i32, [i32; 3], i32),
    /// `playSound(null, spawner, sound, BLOCKS)` at volume and pitch 1.
    Sound(&'static str),
    /// `gameEvent(entity, ENTITY_PLACE, block)` for the mob just added.
    EntityPlace(Uuid, [i32; 3]),
    /// `setBlockAndUpdate` of the `ominous` property alone, even when it
    /// does not change (`removeOminous`).
    OminousSet(bool),
    /// `transformBadOmenIntoTrialOmen`: the player's Bad Omen becomes Trial
    /// Omen for [`TRIAL_OMEN_TICKS_PER_LEVEL`] per level.
    TrialOmen(Uuid),
    /// `resetAfterBecomingOminous`: the mob is discarded (after its level
    /// event 3012).
    Discard(Uuid),
    /// `playSound(null, block, sound, BLOCKS, 1, pitch)`.
    SoundAt(&'static str, [i32; 3], f32),
    /// `OminousItemSpawner.create(level, item)` added at `position`, spawning
    /// its item after `delay` ticks.
    ItemSpawner {
        item: crate::items::ItemStack,
        position: [f64; 3],
        delay: i32,
    },
}

/// What a trial spawner reads from the level besides the spawn rules.
pub trait TrialSpawnerLevel: SpawnerLevel {
    fn game_time(&self) -> i64;
    /// `canSpawnInLevel`'s `spawn_mobs` game rule.
    fn mob_spawning(&self) -> bool;
    /// `level.getPlayers()`, in the server's order.
    fn players(&self) -> Vec<TrialPlayer>;
    /// The block a VISUAL clip from `from` to `to` (no fluids) first hits.
    fn visual_hit(&self, from: [f64; 3], to: [f64; 3]) -> Option<[i32; 3]>;
    /// The block position of a living entity of this dimension, if any.
    fn entity_block(&self, uuid: Uuid) -> Option<[i32; 3]>;
    /// A living entity's position and box height, if it is in this dimension.
    fn entity_box(&self, uuid: Uuid) -> Option<([f64; 3], f32)>;
    /// The block's collision shape is empty.
    fn no_block_collision(&self, block: [i32; 3]) -> bool;
    /// `LootTable.getRandomItems` of `table` seeded with `seed` (an empty
    /// context, stacks split).
    fn loot_items(&self, table: &str, seed: i64) -> Vec<crate::items::ItemStack>;
    /// `getSeed`.
    fn seed(&self) -> i64;
}

fn uuids(tag: Option<&Tag>) -> Vec<Uuid> {
    let Some(Tag::List(list)) = tag else {
        return Vec::new();
    };
    let mut out: Vec<Uuid> = Vec::new();
    for t in list {
        if let Tag::IntArray(v) = t
            && let Ok(u) = <Uuid>::try_from(v.as_slice())
            && !out.contains(&u)
        {
            out.push(u);
        }
    }
    out
}
fn encode_uuids(list: &[Uuid]) -> Tag {
    Tag::List(list.iter().map(|u| Tag::IntArray(u.to_vec())).collect())
}

/// `TrialSpawner`'s behaviour, over the data in `chunk::block_entity`.
pub trait TrialSpawnerExt: Sized {
    /// `TrialSpawner.load`, with the configs of [`load_registry`].
    fn load(tag: &Tag) -> Self;
    /// `TrialSpawner.load`. `registry` resolves a config id.
    fn load_with(tag: &Tag, registry: &dyn Fn(&str) -> Option<TrialConfig>) -> Self;
    /// `TrialSpawner.store`: the state data's fields that differ from their
    /// defaults, then the configs.
    fn save(&self) -> Vec<(&'static str, Tag)>;
    /// `TrialSpawnerStateData.getUpdateTag`.
    fn update_tag(&self, state: TrialState) -> Tag;
    /// `activeConfig`.
    fn active_config(&self) -> &TrialConfig;
    /// `overrideEntityToSpawn` (a spawn egg's `setEntityId`): the data resets
    /// and both configs spawn only `id` (`withSpawning`), saved inline as
    /// `Holder.direct`. The caller then sets the state to `inactive`.
    fn override_entity(&mut self, id: &str);
    /// `resetStatistics`.
    fn reset_statistics(&mut self);
    /// `reset`.
    fn reset(&mut self);
    /// `getOrCreateNextSpawnData`: a potential at random, else empty data.
    fn next_or_create(
        &mut self,
        random: &mut impl RandomSource,
        events: &mut Vec<TrialEvent>,
    ) -> &SpawnData;
    /// `hasMobToSpawn`.
    fn has_mob_to_spawn(
        &mut self,
        random: &mut impl RandomSource,
        events: &mut Vec<TrialEvent>,
    ) -> bool;
    /// `canSpawnInLevel`.
    fn can_spawn_in_level<L: TrialSpawnerLevel>(l: &L) -> bool;
    /// `PlayerDetector.NO_CREATIVE_PLAYERS.detect`.
    fn detect<L: TrialSpawnerLevel>(&self, l: &L, pos: [i32; 3], line_of_sight: bool) -> Vec<Uuid>;
    /// `tryDetectPlayers`, every 20 ticks: a player seen with Trial Omen or
    /// Bad Omen turns the spawner ominous.
    fn try_detect_players<L: TrialSpawnerLevel>(
        &mut self,
        l: &L,
        pos: [i32; 3],
        state: TrialState,
        events: &mut Vec<TrialEvent>,
    );
    /// `applyOminous`, then `resetAfterBecomingOminous`: the spawner's mobs
    /// vanish and the ominous config starts over.
    fn apply_ominous<L: TrialSpawnerLevel>(
        &mut self,
        l: &L,
        pos: [i32; 3],
        events: &mut Vec<TrialEvent>,
    );
    /// `TrialSpawnerState.spawnOminousOminousItemSpawner`: every tick an item
    /// is drawn; once the item spawner cooldown is over, an ominous item
    /// spawner appears above a player or one of the spawner's mobs.
    fn spawn_ominous_item_spawner<L: TrialSpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        events: &mut Vec<TrialEvent>,
    );
    /// `calculatePositionToSpawnSpawner`: above a player in range or one of
    /// the spawner's mobs (a coin decides which), under the first block in
    /// the way, unless that spot is solid.
    fn item_spawner_position<L: TrialSpawnerLevel, R: RandomSource>(
        &self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
    ) -> Option<[f64; 3]>;
    /// `TrialSpawner.tickServer`: the next state of the block.
    #[allow(clippy::too_many_arguments)]
    fn server_tick<L: TrialSpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        state: TrialState,
        ominous: bool,
        spawn: &mut dyn FnMut(&L, &SpawnerSpawn) -> Option<Uuid>,
        eject: &mut dyn FnMut(&str, &mut R) -> bool,
        events: &mut Vec<TrialEvent>,
    ) -> TrialState;
    /// `TrialSpawnerState.tickAndGetNext`.
    #[allow(clippy::too_many_arguments)]
    fn tick_and_get_next<L: TrialSpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        state: TrialState,
        spawn: &mut dyn FnMut(&L, &SpawnerSpawn) -> Option<Uuid>,
        eject: &mut dyn FnMut(&str, &mut R) -> bool,
        events: &mut Vec<TrialEvent>,
    ) -> TrialState;
    /// `TrialSpawner.spawnMob`: the UUID of the mob added, if one was.
    fn spawn_mob<L: TrialSpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        spawn: &mut dyn FnMut(&L, &SpawnerSpawn) -> Option<Uuid>,
        events: &mut Vec<TrialEvent>,
    ) -> Option<Uuid>;
}
impl TrialSpawnerExt for TrialSpawner {
    fn load(tag: &Tag) -> Self {
        Self::load_with(tag, &registry_config)
    }
    fn load_with(tag: &Tag, registry: &dyn Fn(&str) -> Option<TrialConfig>) -> Self {
        let long = |k: &str| match tag.get(k) {
            Some(Tag::Long(v)) => *v,
            t => spawner::number(t).unwrap_or(0),
        };
        let mut s = Self {
            detected_players: uuids(tag.get("registered_players")),
            current_mobs: uuids(tag.get("current_mobs")),
            cooldown_ends_at: long("cooldown_ends_at"),
            next_mob_spawns_at: long("next_mob_spawns_at"),
            total_mobs_spawned: spawner::number(tag.get("total_mobs_spawned"))
                .filter(|v| (0..=i64::from(i32::MAX)).contains(v))
                .map_or(0, |v| v as i32),
            next_spawn_data: tag.get("spawn_data").and_then(SpawnData::decode),
            ejecting_loot_table: identifier(tag.get("ejecting_loot_table")),
            ..Self::default()
        };
        // FullConfig.MAP_CODEC, partial result kept: a field that fails to
        // decode takes its default.
        let slot = |k: &str| -> ConfigSlot {
            let config = match tag.get(k) {
                Some(Tag::String(id)) => {
                    spawner::normalize_id(&id.to_string_lossy()).and_then(|id| registry(&id))
                }
                Some(t) => TrialConfig::decode(t),
                None => None,
            };
            config.map_or_else(ConfigSlot::default, |config| ConfigSlot {
                saved: tag.get(k).cloned(),
                config,
            })
        };
        let int = |k: &str, default: i32, lo: i32, hi: i32| {
            spawner::number(tag.get(k))
                .filter(|v| (i64::from(lo)..=i64::from(hi)).contains(v))
                .map_or(default, |v| v as i32)
        };
        s.normal = slot("normal_config");
        s.ominous = slot("ominous_config");
        s.target_cooldown_length = int("target_cooldown_length", 36000, 0, i32::MAX);
        s.required_player_range = int("required_player_range", 14, 1, 128);
        s
    }
    fn save(&self) -> Vec<(&'static str, Tag)> {
        let mut out = Vec::new();
        if !self.detected_players.is_empty() {
            out.push(("registered_players", encode_uuids(&self.detected_players)));
        }
        if !self.current_mobs.is_empty() {
            out.push(("current_mobs", encode_uuids(&self.current_mobs)));
        }
        if self.cooldown_ends_at != 0 {
            out.push(("cooldown_ends_at", Tag::Long(self.cooldown_ends_at)));
        }
        if self.next_mob_spawns_at != 0 {
            out.push(("next_mob_spawns_at", Tag::Long(self.next_mob_spawns_at)));
        }
        if self.total_mobs_spawned != 0 {
            out.push(("total_mobs_spawned", Tag::Int(self.total_mobs_spawned)));
        }
        if let Some(d) = &self.next_spawn_data {
            out.push(("spawn_data", d.encode()));
        }
        if let Some(t) = &self.ejecting_loot_table {
            out.push(("ejecting_loot_table", Tag::String(t.as_str().into())));
        }
        for (key, slot) in [
            ("normal_config", &self.normal),
            ("ominous_config", &self.ominous),
        ] {
            if let Some(t) = &slot.saved {
                out.push((key, t.clone()));
            }
        }
        if self.target_cooldown_length != 36000 {
            out.push((
                "target_cooldown_length",
                Tag::Int(self.target_cooldown_length),
            ));
        }
        if self.required_player_range != 14 {
            out.push((
                "required_player_range",
                Tag::Int(self.required_player_range),
            ));
        }
        out
    }
    fn update_tag(&self, state: TrialState) -> Tag {
        let mut out = Compound::new();
        if state == TrialState::Active {
            out.insert(
                "next_mob_spawns_at".into(),
                Tag::Long(self.next_mob_spawns_at),
            );
        }
        if let Some(d) = &self.next_spawn_data {
            out.insert("spawn_data".into(), d.encode());
        }
        Tag::Compound(out)
    }
    fn active_config(&self) -> &TrialConfig {
        if self.is_ominous {
            &self.ominous.config
        } else {
            &self.normal.config
        }
    }
    fn reset_statistics(&mut self) {
        self.detected_players.clear();
        self.total_mobs_spawned = 0;
        self.next_mob_spawns_at = 0;
        self.cooldown_ends_at = 0;
    }
    fn reset(&mut self) {
        self.current_mobs.clear();
        self.next_spawn_data = None;
        self.reset_statistics();
    }
    fn override_entity(&mut self, id: &str) {
        self.reset();
        for slot in [&mut self.normal, &mut self.ominous] {
            let mut entity = Compound::new();
            entity.insert("id".into(), Tag::String(id.into()));
            slot.config.spawn_potentials = vec![(SpawnData::new(entity, None, None), 1)];
            slot.saved = Some(slot.config.encode());
        }
    }
    fn next_or_create(
        &mut self,
        random: &mut impl RandomSource,
        events: &mut Vec<TrialEvent>,
    ) -> &SpawnData {
        if self.next_spawn_data.is_none() {
            let potentials = &self.active_config().spawn_potentials;
            let picked = spawner::weighted_index(potentials.iter().map(|(_, w)| *w), random)
                .map(|i| potentials[i].0.clone());
            self.next_spawn_data = Some(picked.unwrap_or_default());
            events.push(TrialEvent::BlockUpdated);
        }
        self.next_spawn_data.get_or_insert_default()
    }
    fn has_mob_to_spawn(
        &mut self,
        random: &mut impl RandomSource,
        events: &mut Vec<TrialEvent>,
    ) -> bool {
        self.next_or_create(random, events).id().is_some()
            // WeightedList.isEmpty: a total weight of 0.
            || self.active_config().spawn_potentials.iter().any(|(_, w)| *w > 0)
    }
    fn can_spawn_in_level<L: TrialSpawnerLevel>(l: &L) -> bool {
        l.spawner_blocks_work() && !l.peaceful() && l.mob_spawning()
    }
    fn detect<L: TrialSpawnerLevel>(&self, l: &L, pos: [i32; 3], line_of_sight: bool) -> Vec<Uuid> {
        let range = f64::from(self.required_player_range);
        l.players()
            .into_iter()
            .filter(|p| {
                let d = [0, 1, 2].map(|i| f64::from(p.block[i] - pos[i]));
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < range * range
                    && !p.creative
                    && !p.spectator
            })
            .filter(|p| !line_of_sight || in_line_of_sight(l, center(pos), p.eye))
            .map(|p| p.uuid)
            .collect()
    }
    fn try_detect_players<L: TrialSpawnerLevel>(
        &mut self,
        l: &L,
        pos: [i32; 3],
        state: TrialState,
        events: &mut Vec<TrialEvent>,
    ) {
        let packed = BlockPos::new(pos[0], pos[1], pos[2]).pack();
        if packed.wrapping_add(l.game_time()) % 20 != 0 {
            return;
        }
        let cooldown = state == TrialState::Cooldown;
        if cooldown && self.is_ominous {
            return;
        }
        let in_sight = self.detect(l, pos, true);
        let mut became_ominous = false;
        if !self.is_ominous && !in_sight.is_empty() {
            // findPlayerWithOminousEffect: the first with Trial Omen, else
            // the last with Bad Omen.
            let players = l.players();
            let mut found = None;
            for uuid in &in_sight {
                let Some(p) = players.iter().find(|p| p.uuid == *uuid) else {
                    continue;
                };
                match p.omen {
                    Some(Omen::Trial) => {
                        found = Some((*p, Omen::Trial));
                        break;
                    }
                    Some(Omen::Bad) => found = Some((*p, Omen::Bad)),
                    None => {}
                }
            }
            if let Some((player, omen)) = found {
                if omen == Omen::Bad {
                    events.push(TrialEvent::TrialOmen(player.uuid));
                }
                let eye = player.eye.map(|v| v.floor() as i32);
                events.push(TrialEvent::LevelEvent(3020, eye, 0));
                self.apply_ominous(l, pos, events);
                became_ominous = true;
            }
        }
        if cooldown && !became_ominous {
            return;
        }
        let found = if self.detected_players.is_empty() {
            in_sight
        } else {
            self.detect(l, pos, false)
        };
        let mut added = false;
        for uuid in found {
            if !self.detected_players.contains(&uuid) {
                self.detected_players.push(uuid);
                added = true;
            }
        }
        if added {
            self.next_mob_spawns_at = (l.game_time() + 40).max(self.next_mob_spawns_at);
            if !became_ominous {
                let id = if self.is_ominous { 3019 } else { 3013 };
                events.push(TrialEvent::LevelEvent(
                    id,
                    pos,
                    self.detected_players.len() as i32,
                ));
            }
        }
    }
    fn apply_ominous<L: TrialSpawnerLevel>(
        &mut self,
        l: &L,
        pos: [i32; 3],
        events: &mut Vec<TrialEvent>,
    ) {
        events.push(TrialEvent::OminousSet(true));
        events.push(TrialEvent::LevelEvent(3020, pos, 1));
        self.is_ominous = true;
        for uuid in hash_order(&self.current_mobs) {
            if let Some(block) = l.entity_block(uuid) {
                // FlameParticle.NORMAL.
                events.push(TrialEvent::LevelEvent(3012, block, 0));
                events.push(TrialEvent::Discard(uuid));
            }
        }
        // WeightedList.isEmpty: a total weight of 0.
        if self
            .ominous
            .config
            .spawn_potentials
            .iter()
            .any(|(_, w)| *w > 0)
        {
            self.next_spawn_data = None;
        }
        self.total_mobs_spawned = 0;
        self.current_mobs.clear();
        let now = l.game_time();
        self.next_mob_spawns_at = now + i64::from(self.ominous.config.ticks_between_spawn);
        events.push(TrialEvent::BlockUpdated);
        self.cooldown_ends_at = now + TICKS_BETWEEN_ITEM_SPAWNERS;
    }
    fn spawn_ominous_item_spawner<L: TrialSpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        events: &mut Vec<TrialEvent>,
    ) {
        // getDispensingItems: rolled once with a seed from the level seed and
        // the spawner's 30x20x30 cell; an empty roll is not kept.
        if self.dispensing.is_none() {
            let cell = BlockPos::new(
                (pos[0] as f32 / 30.0).floor() as i32,
                (pos[1] as f32 / 20.0).floor() as i32,
                (pos[2] as f32 / 30.0).floor() as i32,
            );
            let table = &self.active_config().items_to_drop_when_ominous;
            let drops = l.loot_items(table, l.seed().wrapping_add(cell.pack()));
            if !drops.is_empty() {
                self.dispensing = Some(drops);
            }
        }
        let items = self.dispensing.as_deref().unwrap_or_default();
        // WeightedList.getRandom: each stack weighs its count; one item spawns.
        let Some(index) = spawner::weighted_index(items.iter().map(|s| s.count as i32), random)
        else {
            return;
        };
        let item = crate::items::ItemStack {
            count: 1,
            ..items[index].clone()
        };
        if l.game_time() < self.cooldown_ends_at {
            return;
        }
        let Some(position) = self.item_spawner_position(l, random, pos) else {
            return;
        };
        let delay = random.next_int_between_inclusive(60, 120).unwrap_or(60);
        events.push(TrialEvent::ItemSpawner {
            item,
            position,
            delay,
        });
        let pitch = (random.next_float() - random.next_float()) * 0.2 + 1.0;
        events.push(TrialEvent::SoundAt(
            "block.trial_spawner.spawn_item_begin",
            position.map(|v| v.floor() as i32),
            pitch,
        ));
        self.cooldown_ends_at = l.game_time() + TICKS_BETWEEN_ITEM_SPAWNERS;
    }
    fn item_spawner_position<L: TrialSpawnerLevel, R: RandomSource>(
        &self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
    ) -> Option<[f64; 3]> {
        let c = center(pos);
        let range = f64::from(self.required_player_range * self.required_player_range);
        let near = |p: [f64; 3]| {
            (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2) <= range
        };
        let all = l.players();
        let players: Vec<([f64; 3], f32)> = hash_order(&self.detected_players)
            .into_iter()
            .filter_map(|uuid| all.iter().find(|p| p.uuid == uuid))
            .filter(|p| !p.creative && !p.spectator && p.alive && near(p.position))
            .map(|p| (p.position, p.height))
            .collect();
        if players.is_empty() {
            return None;
        }
        let mobs: Vec<([f64; 3], f32)> = hash_order(&self.current_mobs)
            .into_iter()
            .filter_map(|uuid| l.entity_box(uuid))
            .filter(|(p, _)| near(*p))
            .collect();
        let eligible = if random.next_boolean() { mobs } else { players };
        let (at, height) = match eligible.len() {
            0 => return None,
            1 => eligible[0],
            n => eligible[random.next_int_bound(n as i32).unwrap_or(0) as usize],
        };
        // calculatePositionAbove: a VISUAL clip up to 2 to 5 blocks over the
        // entity's head; a miss ends in the block holding the end point.
        let up = height + 2.0 + random.next_int_bound(4).unwrap_or(0) as f32;
        let to = [at[0], at[1] + f64::from(up), at[2]];
        let hit = l
            .visual_hit(at, to)
            .unwrap_or_else(|| to.map(|v| v.floor() as i32));
        let down = [
            f64::from(hit[0]) + 0.5,
            f64::from(hit[1]) + 0.5 - 1.0,
            f64::from(hit[2]) + 0.5,
        ];
        l.no_block_collision(down.map(|v| v.floor() as i32))
            .then_some(down)
    }
    #[allow(clippy::too_many_arguments)]
    fn server_tick<L: TrialSpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        state: TrialState,
        ominous: bool,
        spawn: &mut dyn FnMut(&L, &SpawnerSpawn) -> Option<Uuid>,
        eject: &mut dyn FnMut(&str, &mut R) -> bool,
        events: &mut Vec<TrialEvent>,
    ) -> TrialState {
        self.is_ominous = ominous;
        let before = self.current_mobs.len();
        let max = MAX_MOB_TRACKING_DISTANCE * MAX_MOB_TRACKING_DISTANCE;
        self.current_mobs.retain(|id| {
            l.entity_block(*id).is_some_and(|b| {
                let d = [0, 1, 2].map(|i| i64::from(b[i] - pos[i]));
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= max
            })
        });
        if self.current_mobs.len() != before {
            self.next_mob_spawns_at =
                l.game_time() + i64::from(self.active_config().ticks_between_spawn);
        }
        self.tick_and_get_next(l, random, pos, state, spawn, eject, events)
    }
    #[allow(clippy::too_many_arguments)]
    fn tick_and_get_next<L: TrialSpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        state: TrialState,
        spawn: &mut dyn FnMut(&L, &SpawnerSpawn) -> Option<Uuid>,
        eject: &mut dyn FnMut(&str, &mut R) -> bool,
        events: &mut Vec<TrialEvent>,
    ) -> TrialState {
        use TrialState as S;
        let now = l.game_time();
        match state {
            S::Inactive => {
                // getOrCreateDisplayEntity: made once, from the next data's id.
                if !self.has_display {
                    let id = self.next_or_create(random, events).id();
                    self.has_display = id.is_some_and(|id| l.entity_exists(&id));
                }
                if self.has_display {
                    S::WaitingForPlayers
                } else {
                    state
                }
            }
            S::WaitingForPlayers => {
                if !Self::can_spawn_in_level(l) {
                    self.reset_statistics();
                    state
                } else if !self.has_mob_to_spawn(random, events) {
                    S::Inactive
                } else {
                    self.try_detect_players(l, pos, state, events);
                    if self.detected_players.is_empty() {
                        state
                    } else {
                        S::Active
                    }
                }
            }
            S::Active => {
                if !Self::can_spawn_in_level(l) {
                    self.reset_statistics();
                    return S::WaitingForPlayers;
                }
                if !self.has_mob_to_spawn(random, events) {
                    return S::Inactive;
                }
                let config = self.active_config().clone();
                let additional = (self.detected_players.len() as i32 - 1).max(0);
                self.try_detect_players(l, pos, state, events);
                if self.is_ominous {
                    self.spawn_ominous_item_spawner(l, random, pos, events);
                }
                if self.total_mobs_spawned >= config.target_total_mobs(additional) {
                    if self.current_mobs.is_empty() {
                        self.cooldown_ends_at = now + i64::from(self.target_cooldown_length);
                        self.total_mobs_spawned = 0;
                        self.next_mob_spawns_at = 0;
                        return S::WaitingForRewardEjection;
                    }
                } else if now >= self.next_mob_spawns_at
                    && (self.current_mobs.len() as i32)
                        < config.target_simultaneous_mobs(additional)
                    && let Some(uuid) = self.spawn_mob(l, random, pos, spawn, events)
                {
                    self.current_mobs.push(uuid);
                    self.total_mobs_spawned += 1;
                    self.next_mob_spawns_at = now + i64::from(config.ticks_between_spawn);
                    let potentials = &config.spawn_potentials;
                    if let Some(i) =
                        spawner::weighted_index(potentials.iter().map(|(_, w)| *w), random)
                    {
                        self.next_spawn_data = Some(potentials[i].0.clone());
                        events.push(TrialEvent::BlockUpdated);
                    }
                }
                state
            }
            S::WaitingForRewardEjection => {
                let started = self.cooldown_ends_at - i64::from(self.target_cooldown_length);
                if now as f32 >= started as f32 + DELAY_BEFORE_EJECT {
                    events.push(TrialEvent::Sound("block.trial_spawner.open_shutter"));
                    S::EjectingReward
                } else {
                    state
                }
            }
            S::EjectingReward => {
                let started = self.cooldown_ends_at - i64::from(self.target_cooldown_length);
                if (now - started) as f32 % TIME_BETWEEN_EJECTIONS != 0.0 {
                    return state;
                }
                if self.detected_players.is_empty() {
                    events.push(TrialEvent::Sound("block.trial_spawner.close_shutter"));
                    self.ejecting_loot_table = None;
                    return S::Cooldown;
                }
                if self.ejecting_loot_table.is_none() {
                    let tables = &self.active_config().loot_tables_to_eject;
                    self.ejecting_loot_table =
                        spawner::weighted_index(tables.iter().map(|(_, w)| *w), random)
                            .map(|i| tables[i].0.clone());
                }
                if let Some(table) = self.ejecting_loot_table.clone()
                    && eject(&table, random)
                {
                    events.push(TrialEvent::LevelEvent(3014, pos, 0));
                }
                // detectedPlayers.remove(iterator().next()).
                if let Some(first) = hash_order(&self.detected_players).first() {
                    self.detected_players.retain(|u| u != first);
                }
                state
            }
            S::Cooldown => {
                self.try_detect_players(l, pos, state, events);
                if !self.detected_players.is_empty() {
                    self.total_mobs_spawned = 0;
                    self.next_mob_spawns_at = 0;
                    S::Active
                } else if now >= self.cooldown_ends_at {
                    self.is_ominous = false;
                    events.push(TrialEvent::OminousSet(false));
                    self.reset();
                    S::WaitingForPlayers
                } else {
                    state
                }
            }
        }
    }
    fn spawn_mob<L: TrialSpawnerLevel, R: RandomSource>(
        &mut self,
        l: &L,
        random: &mut R,
        pos: [i32; 3],
        spawn: &mut dyn FnMut(&L, &SpawnerSpawn) -> Option<Uuid>,
        events: &mut Vec<TrialEvent>,
    ) -> Option<Uuid> {
        let data = self.next_or_create(random, events).clone();
        // EntityType.by: an unknown type spawns nothing.
        let id = data.id().filter(|id| l.entity_exists(id))?;
        let at = match spawner::read_pos(data.entity.get(&JavaString::from("Pos"))) {
            Some(p) => p,
            None => {
                let range = f64::from(self.active_config().spawn_range);
                let x =
                    f64::from(pos[0]) + (random.next_double() - random.next_double()) * range + 0.5;
                let y = f64::from(pos[1] + random.next_int_bound(3).unwrap_or(0) - 1);
                let z =
                    f64::from(pos[2]) + (random.next_double() - random.next_double()) * range + 0.5;
                [x, y, z]
            }
        };
        // Entity types Horizon cannot create spawn nothing.
        let (species, width, height, _) = l.mob_type(&id)?;
        let spawn_box = spawn_selection::spawn_aabb(species, at[0], at[1], at[2], width, height);
        if !l.no_collision(spawn_box) || !in_line_of_sight(l, center(pos), at) {
            return None;
        }
        let block = at.map(|v| v.floor() as i32);
        if !spawn_rules::check_spawn_rules(
            species,
            l,
            Reason::TrialSpawner,
            random,
            block[0],
            block[1],
            block[2],
        ) {
            return None;
        }
        if let Some(rules) = data.custom_spawn_rules
            && !SpawnData::valid_position(rules, l, block[0], block[1], block[2])
        {
            return None;
        }
        let yaw = random.next_float() * 360.0;
        let placed = spawner::entity_position(species, at);
        let own = placed.map(|v| v.floor() as i32);
        let scale = spawner::cube_scale(&id, &data.entity);
        let bounds = spawn_selection::spawn_box(
            placed[0],
            placed[1],
            placed[2],
            width * scale,
            height * scale,
        );
        if !spawn_selection::spawn_obstruction(species, l, bounds, &[], own[0], own[1], own[2]) {
            return None;
        }
        let request = SpawnerSpawn {
            entity: &data.entity,
            id: &id,
            position: placed,
            yaw,
            finalize: data.unconfigured(),
            equipment: data.equipment.as_ref(),
            trial: true,
        };
        let uuid = spawn(l, &request)?;
        let flame = i32::from(self.is_ominous);
        events.push(TrialEvent::LevelEvent(3011, pos, flame));
        events.push(TrialEvent::LevelEvent(3012, block, flame));
        events.push(TrialEvent::EntityPlace(uuid, block));
        Some(uuid)
    }
}

/// `Vec3.atCenterOf`.
/// The iteration order of a `HashSet<UUID>` holding `ids` (inserted in
/// order): `HashMap` buckets by the spread `UUID.hashCode`, in a table of 16
/// slots grown to keep it at most three quarters full; within a bucket,
/// insertion order.
fn hash_order(ids: &[Uuid]) -> Vec<Uuid> {
    let mut table = 16usize;
    while ids.len() > table * 3 / 4 {
        table *= 2;
    }
    let bucket = |u: &Uuid| {
        let h = (u[0] ^ u[1] ^ u[2] ^ u[3]) as u32;
        ((h ^ (h >> 16)) as usize) & (table - 1)
    };
    let mut out = ids.to_vec();
    out.sort_by_key(bucket);
    out
}
fn center(pos: [i32; 3]) -> [f64; 3] {
    pos.map(|v| f64::from(v) + 0.5)
}
/// `TrialSpawner.inLineOfSight`: a VISUAL clip from `to` back to the spawner
/// hits nothing, or the spawner's own block.
fn in_line_of_sight<L: TrialSpawnerLevel>(l: &L, origin: [f64; 3], to: [f64; 3]) -> bool {
    match l.visual_hit(to, origin) {
        None => true,
        Some(b) => b == origin.map(|v| v.floor() as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(snbt: &str) -> Tag {
        horizon_world::nbt::parse_snbt(snbt).expect("snbt").0
    }

    #[test]
    fn defaults_load_and_save_to_nothing() {
        let s = TrialSpawner::load_with(&tag("{}"), &|_| None);
        assert_eq!(s, TrialSpawner::default());
        assert!(s.save().is_empty());
    }

    #[test]
    fn a_spawn_egg_overrides_both_configs_inline() {
        let breeze = TrialConfig {
            total_mobs: 2.0,
            ..TrialConfig::default()
        };
        let registry = |_: &str| Some(breeze.clone());
        let mut s = TrialSpawner::load_with(
            &tag(
                r#"{normal_config:"minecraft:trial_chamber/breeze/normal",spawn_data:{entity:{}},total_mobs_spawned:3}"#,
            ),
            &registry,
        );
        s.override_entity("minecraft:zombie");
        assert_eq!(s.next_spawn_data, None);
        assert_eq!(s.total_mobs_spawned, 0);
        for slot in [&s.normal, &s.ominous] {
            assert_eq!(slot.config.spawn_potentials.len(), 1);
            assert_eq!(
                slot.config.spawn_potentials[0].0.id().as_deref(),
                Some("minecraft:zombie")
            );
            let saved = slot.saved.as_ref().expect("saved inline");
            assert_eq!(TrialConfig::decode(saved).as_ref(), Some(&slot.config));
        }
        assert_eq!(
            s.normal.saved.as_ref().and_then(|t| t.get("total_mobs")),
            Some(&Tag::Float(2.0f32.to_bits()))
        );
    }

    #[test]
    fn registry_configs_resolve_and_are_saved_by_id() {
        let breeze = TrialConfig {
            total_mobs: 2.0,
            ..TrialConfig::default()
        };
        let registry =
            |id: &str| (id == "minecraft:trial_chamber/breeze/normal").then(|| breeze.clone());
        let s = TrialSpawner::load_with(
            &tag(
                r#"{normal_config:"minecraft:trial_chamber/breeze/normal",required_player_range:10}"#,
            ),
            &registry,
        );
        assert_eq!(s.normal.config.total_mobs, 2.0);
        assert_eq!(s.required_player_range, 10);
        let saved: Vec<_> = s.save().into_iter().map(|(k, _)| k).collect();
        assert_eq!(saved, ["normal_config", "required_player_range"]);
    }

    #[test]
    fn an_invalid_field_takes_its_default_alone() {
        let s = TrialSpawner::load_with(
            &tag(
                r#"{normal_config:"minecraft:nope",required_player_range:200,target_cooldown_length:5}"#,
            ),
            &|_| None,
        );
        assert_eq!(s.normal, ConfigSlot::default());
        assert_eq!(s.required_player_range, 14);
        assert_eq!(s.target_cooldown_length, 5);
        let c = TrialConfig::decode(&tag(
            r#"{spawn_range:0,total_mobs:3.0f,spawn_potentials:[{data:{entity:{id:"minecraft:zombie"}},weight:-1},{data:{entity:{id:"minecraft:husk"}},weight:2}]}"#,
        ))
        .expect("config");
        assert_eq!(c.spawn_range, 4);
        assert_eq!(c.total_mobs, 3.0);
        assert_eq!(c.spawn_potentials.len(), 1);
    }

    #[test]
    fn targets_floor_the_per_player_additions() {
        let c = TrialConfig {
            simultaneous_mobs: 1.0,
            simultaneous_mobs_added_per_player: 0.5,
            ..TrialConfig::default()
        };
        assert_eq!(c.target_simultaneous_mobs(0), 1);
        assert_eq!(c.target_simultaneous_mobs(1), 1);
        assert_eq!(c.target_simultaneous_mobs(2), 2);
        assert_eq!(c.target_total_mobs(3), 12);
    }
}
