//! Pillager patrols (`PatrolSpawner`), the raid omen, and raids (`Raids`):
//! their waves, boss bar, victory and the raiders' bookkeeping. The rules are
//! `horizon_client::patrol` and `horizon_client::raid`; this is the game's
//! side of them.
use super::{Dim, Game};
use horizon_client::mobs::villager as mobs_villager;
use horizon_client::{
    effects::{EffectInstance, OmenTick},
    mobs::{
        Mob, Species, raider,
        spawn_rules::{self, SpawnLevel},
        spawn_selection::{Heightmap, NaturalLevel},
        spawning,
    },
    patrol::{GameplayAttributes, Member, PatrolLevel, PatrolSpawner},
    raid::{self, Action, BarName, RaidLevel, RaidPlayer, RaiderInfo, RaiderType, Raids, Spawn},
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::nbt::Tag;
use std::collections::{HashMap, HashSet};

/// Patrol and raid state owned by the game.
#[derive(Default)]
pub(super) struct Raiding {
    patrol: PatrolSpawner,
    siege: Siege,
    /// The raids of the dimension the player is in, and that dimension.
    raids: Raids,
    loaded: Option<Dim>,
    /// `ServerPlayer.raidOmenPosition` (`raid_omen_position`), read from the
    /// player data on first use.
    omen_position: Option<[i32; 3]>,
    omen_loaded: bool,
    /// The last raider hurt each raid's bar has seen (`Raider.hurtServer`).
    bar_hurts: HashMap<i32, u64>,
    attributes: Option<std::sync::Arc<GameplayAttributes>>,
    /// Beds near each raid's centre, rescanned now and then (`HOME` points
    /// of interest for `RaiderMoveThroughVillageGoal`).
    homes: HashMap<i32, (i64, std::sync::Arc<Vec<[i32; 3]>>)>,
}

/// `VillageSiege`: whether tonight is a siege, whether the spot is chosen,
/// and the zombies still to spawn there.
#[derive(Default)]
struct Siege {
    tonight: bool,
    set_up: bool,
    zombies_to_spawn: i32,
    next_spawn_time: i32,
    spot: [i32; 3],
}

impl Raiding {
    /// Exchange the level-held parts (`ServerLevel.getRaids`) with another
    /// level's (game/levels.rs).
    pub(super) fn swap_level(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.raids, &mut other.raids);
        std::mem::swap(&mut self.loaded, &mut other.loaded);
        std::mem::swap(&mut self.bar_hurts, &mut other.bar_hurts);
        std::mem::swap(&mut self.homes, &mut other.homes);
    }
}

/// `SectionPos` of a block position.
pub(super) fn section(p: [i32; 3]) -> (i32, i32, i32) {
    (p[0] >> 4, p[1] >> 4, p[2] >> 4)
}
/// `PoiManager.DistanceTracker`: sections from the nearest village centre,
/// 7 when farther than 6.
pub(super) fn sections_to_village(centers: &HashSet<(i32, i32, i32)>, at: (i32, i32, i32)) -> i32 {
    centers
        .iter()
        .map(|c| {
            (c.0 - at.0)
                .abs()
                .max((c.1 - at.1).abs())
                .max((c.2 - at.2).abs())
        })
        .min()
        .map_or(7, |d| d.min(7))
}

/// The level as a raid sees it, with this tick's village data.
struct Lvl<'a, 'g> {
    game: &'a Game<'g>,
    difficulty: i32,
    centers: &'a HashSet<(i32, i32, i32)>,
}
impl RaidLevel for Lvl<'_, '_> {
    fn difficulty(&self) -> i32 {
        self.difficulty
    }
    fn has_chunk_at(&self, p: [i32; 3]) -> bool {
        self.game.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4))
    }
    fn has_chunks_at(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        (x0 >> 4..=x1 >> 4)
            .all(|cx| (z0 >> 4..=z1 >> 4).all(|cz| self.game.world.chunks.contains_key(&(cx, cz))))
    }
    fn sections_to_village(&self, p: [i32; 3]) -> i32 {
        sections_to_village(self.centers, section(p))
    }
    fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.game.surface_height(x, z)
    }
    fn is_position_entity_ticking(&self, p: [i32; 3]) -> bool {
        self.has_chunk_at(p)
    }
    fn ravager_can_spawn(&self, [x, y, z]: [i32; 3]) -> bool {
        let g = self.game;
        let w = g.mob_world_base(&[]);
        spawn_rules::is_spawn_position_ok(Species::Ravager, &w, x, y, z)
            || w.block_name(x, y - 1, z) == "minecraft:snow"
                && g.res.facts().is_air(g.world.block(x, y, z))
    }
    fn player(&self) -> Option<RaidPlayer> {
        // The first player in the level: a server has no player of its own.
        let p = self.game.level_players().into_iter().next()?;
        Some(RaidPlayer {
            position: [p.position.x, p.position.y, p.position.z],
            block: block_of(p.position),
            alive: p.alive,
        })
    }
}

/// The game level as the patrol spawner reads it.
struct PatrolCx<'a, 'g, 'h> {
    level: super::natural_spawning::GameLevel<'a, 'g>,
    centers: HashSet<(i32, i32, i32)>,
    can_spawn: &'h dyn Fn(&Game, [i32; 3]) -> bool,
}
impl SpawnLevel for PatrolCx<'_, '_, '_> {
    fn state(&self, x: i32, y: i32, z: i32) -> horizon_world::registry::BlockStateId {
        self.level.state(x, y, z)
    }
    fn facts(&self) -> &horizon_client::blocks::BlockFacts {
        self.level.facts()
    }
    fn mob_data(&self) -> &horizon_client::mobs::data::MobData {
        self.level.mob_data()
    }
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.level.block_tag(tag, block)
    }
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        self.level.biome_tag(tag, x, y, z)
    }
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.level.sky_light(x, y, z)
    }
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.level.block_light(x, y, z)
    }
    fn sky_darken(&self) -> i32 {
        SpawnLevel::sky_darken(&self.level)
    }
    fn thundering(&self) -> bool {
        self.level.thundering()
    }
    fn sea_level(&self) -> i32 {
        self.level.sea_level()
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.level.world_surface(x, z)
    }
    fn peaceful(&self) -> bool {
        self.level.peaceful()
    }
    fn dimension(&self) -> spawn_rules::Dimension {
        self.level.dimension()
    }
    fn surface_slime_chance(&self, x: i32, y: i32, z: i32) -> f32 {
        self.level.surface_slime_chance(x, y, z)
    }
    fn world_seed(&self) -> i64 {
        self.level.world_seed()
    }
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.level.player_near(x, y, z, distance)
    }
}
impl PatrolLevel for PatrolCx<'_, '_, '_> {
    fn spawn_patrols(&self) -> bool {
        self.level.game.ui.rules.bool("spawn_patrols")
    }
    fn bright_outside(&self) -> bool {
        self.level.game.dim == Dim::Overworld && self.level.game.sky_darken() < 4
    }
    /// `PatrolSpawner.tick` picks from `level.players()`: the local player
    /// and every guest in the level.
    fn patrol_players(&self) -> Vec<([i32; 3], bool)> {
        self.level
            .players
            .iter()
            .map(|p| (block_of(p.position), p.spectator))
            .collect()
    }
    fn sections_to_village(&self, pos: [i32; 3]) -> i32 {
        sections_to_village(&self.centers, section(pos))
    }
    fn has_chunk(&self, cx: i32, cz: i32) -> bool {
        self.level.game.world.chunks.contains_key(&(cx, cz))
    }
    fn can_pillager_patrol_spawn(&self, pos: [i32; 3]) -> bool {
        (self.can_spawn)(self.level.game, pos)
    }
    fn effective_difficulty(&self, pos: [i32; 3]) -> f32 {
        self.level
            .game
            .local_difficulty([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])])
            .effective
    }
    fn motion_blocking_no_leaves(&self, x: i32, z: i32) -> i32 {
        self.level
            .heightmap(Heightmap::MotionBlockingNoLeaves, x, z)
    }
}

fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}

impl Game<'_> {
    /// `Heightmap.Types.WORLD_SURFACE`: the first free block above the
    /// highest non-air one.
    pub(super) fn surface_height(&self, x: i32, z: i32) -> i32 {
        let top = if self.dim == Dim::Nether { 127 } else { 319 };
        (self.world.bottom_y()..=top)
            .rev()
            .find(|&y| !self.res.facts().is_air(self.world.block(x, y, z)))
            .map_or(self.world.bottom_y(), |y| y + 1)
    }
    /// Every claimed village point of interest (job sites, beds, bells).
    fn village_pois(&self) -> Vec<[i32; 3]> {
        let mut out: Vec<[i32; 3]> = Vec::new();
        for m in &self.mobs {
            if m.species() != Species::Villager || !m.alive() {
                continue;
            }
            for p in [m.extra.home, m.extra.bed, m.extra.meeting]
                .into_iter()
                .flatten()
            {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }
    pub(super) fn difficulty_id(&self) -> i32 {
        use horizon_world::food::Difficulty;
        match self.world.health_rules.difficulty {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        }
    }
    fn player_block(&self) -> Option<[i32; 3]> {
        self.player.as_ref().map(|p| block_of(p.position))
    }
    /// `dimensions/<dimension>/data/minecraft/raids.dat`: each dimension
    /// keeps its own raids.
    fn raids_path(&self, dim: Dim) -> std::path::PathBuf {
        horizon_client::save::dimension_dir(&self.world_root, dim.id())
            .join("data/minecraft/raids.dat")
    }
    /// The raids of the player's dimension (the previous one saved first).
    fn load_raids(&mut self) {
        if self.raiding.loaded == Some(self.dim) {
            return;
        }
        if self.raiding.loaded.is_some() {
            self.save_raids();
        }
        self.raiding.loaded = Some(self.dim);
        self.raiding.bar_hurts.clear();
        self.raiding.homes.clear();
        let seed = self.world.random.next_long();
        self.raiding.raids = match horizon_client::save::read_dat(&self.raids_path(self.dim))
            .and_then(|tag| tag.get("data").cloned())
        {
            Some(data) => Raids::from_nbt(&data, seed),
            None => Raids::default(),
        };
    }
    /// `Raids` as its dimension's `raids.dat`, when it changed.
    pub(super) fn save_raids(&mut self) {
        let Some(dim) = self.raiding.loaded else {
            return;
        };
        if !self.raiding.raids.dirty || self.remote.is_some() {
            return;
        }
        let tag = horizon_world::nbt::Tag::compound([
            ("data", self.raiding.raids.to_nbt()),
            (
                "DataVersion",
                horizon_world::nbt::Tag::Int(horizon_world::DATA_VERSION),
            ),
        ]);
        if horizon_client::save::write_dat(&self.raids_path(dim), tag).is_ok() {
            self.raiding.raids.dirty = false;
        }
        self.save_raid_omen_position();
    }
    /// `ServerPlayer.raidOmenPosition` from the player data.
    fn load_raid_omen_position(&mut self) {
        if self.raiding.omen_loaded {
            return;
        }
        self.raiding.omen_loaded = true;
        self.raiding.omen_position = match self
            .level
            .player_extra
            .as_ref()
            .and_then(|p| p.get("raid_omen_position"))
        {
            Some(horizon_world::nbt::Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
            _ => None,
        };
    }
    /// Writes `raid_omen_position` into the player data being saved.
    fn save_raid_omen_position(&mut self) {
        if !self.raiding.omen_loaded {
            return;
        }
        let extra = self
            .level
            .player_extra
            .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
        if let horizon_world::nbt::Tag::Compound(map) = extra {
            match self.raiding.omen_position {
                Some(p) => {
                    map.insert(
                        "raid_omen_position".into(),
                        horizon_world::nbt::Tag::IntArray(p.to_vec()),
                    );
                }
                None => {
                    map.remove(&"raid_omen_position".into());
                }
            }
        }
    }
    fn gameplay_attributes(&mut self) -> std::sync::Arc<GameplayAttributes> {
        if self.raiding.attributes.is_none() {
            self.raiding.attributes = Some(std::sync::Arc::new(GameplayAttributes::load(
                &self.options.resources,
            )));
        }
        self.raiding.attributes.clone().unwrap_or_default()
    }
    /// A boolean gameplay attribute at a position (dimension, biome, then
    /// the timelines at the overworld clock).
    fn gameplay_attribute(&mut self, key: &str, at: [i32; 3]) -> bool {
        let attributes = self.gameplay_attributes();
        let biome = self
            .world
            .chunks
            .get(&(at[0] >> 4, at[2] >> 4))
            .map(|c| {
                self.res
                    .biomes()
                    .get(c.biome(at[0], at[1], at[2]))
                    .name
                    .clone()
            })
            .unwrap_or_default();
        attributes.value(key, self.dim.id(), &biome, self.world.day_time)
    }

    /// `BadOmenMobEffect` and `RaidOmenMobEffect.applyEffectTick` for the
    /// player's effect tick: Bad Omen inside a village becomes Raid Omen,
    /// whose last tick starts or extends a raid where it began.
    pub(super) fn omen_tick(&mut self, e: &EffectInstance) -> OmenTick {
        if self.spectator() || self.player.is_none() {
            return OmenTick::Continue;
        }
        let Some(pos) = self.player_block() else {
            return OmenTick::Continue;
        };
        self.load_raids();
        self.load_raid_omen_position();
        match e.id {
            "bad_omen" => {
                if self.difficulty_id() != 0
                    && sections_to_village(&self.village_centers(), section(pos)) <= 1
                    && self
                        .raiding
                        .raids
                        .nearby(pos)
                        .is_none_or(|(_, r)| r.omen_level < raid::MAX_OMEN_LEVEL)
                {
                    self.raiding.omen_position = Some(pos);
                    return OmenTick::Add(EffectInstance::new("raid_omen", 600, e.amplifier));
                }
                OmenTick::Continue
            }
            "raid_omen" => match self.raiding.omen_position.take() {
                Some(at) => {
                    self.create_or_extend_raid(at, Some(e.amplifier));
                    OmenTick::Remove
                }
                None => OmenTick::Continue,
            },
            _ => OmenTick::Continue,
        }
    }
    /// The Raid Omen sound when the effect is first added.
    pub(super) fn raid_omen_added(&mut self) {
        let eye = self.eye_position().unwrap_or_default();
        self.audio
            .play("minecraft:event.mob_effect.raid_omen", Some(eye), 1.0, 1.0);
    }
    /// `Raids.createOrExtendRaid`: not for spectators, with the `raids` rule
    /// and where raids may start; the raid centres on the occupied village
    /// points of interest within 64 blocks.
    fn create_or_extend_raid(&mut self, at: [i32; 3], amplifier: Option<i32>) -> Option<i32> {
        if self.spectator()
            || !self.ui.rules.bool("raids")
            || !self.gameplay_attribute(horizon_client::patrol::CAN_START_RAID, at)
        {
            return None;
        }
        self.load_raids();
        let center = raid::raid_center(at, &self.village_pois());
        let seed = self.world.random.next_long();
        let difficulty = self.difficulty_id();
        let (id, triggered) = self
            .raiding
            .raids
            .create_or_extend(center, difficulty, amplifier, seed);
        if triggered {
            self.stats.custom("raid_trigger", 1);
            self.trigger_player("minecraft:voluntary_exile");
        }
        Some(id)
    }
    /// `ServerLevel.getRaidAt`.
    pub(super) fn raiding_nearby(&self, at: [i32; 3]) -> Option<i32> {
        self.raiding.raids.nearby(at).map(|(id, _)| id)
    }
    pub(super) fn raid_started(&self, id: i32) -> bool {
        self.raiding.raids.get(id).is_some_and(|r| r.started)
    }
    /// `/raid start`: a raid in the local village at the given omen level.
    pub(super) fn raid_command_create(&mut self, at: [i32; 3], omen_level: i32) -> bool {
        self.load_raids();
        let Some(id) = self.create_or_extend_raid(at, None) else {
            return false;
        };
        if let Some(r) = self.raiding.raids.get_mut(id) {
            r.omen_level = omen_level;
        }
        self.raiding.raids.dirty = true;
        true
    }
    pub(super) fn raid_command_stop(&mut self, id: i32) {
        if let Some(r) = self.raiding.raids.get_mut(id) {
            r.stop();
        }
    }
    pub(super) fn raid_set_omen(&mut self, id: i32, level: i32) -> i32 {
        let Some(r) = self.raiding.raids.get_mut(id) else {
            return 0;
        };
        std::mem::replace(&mut r.omen_level, level)
    }
    pub(super) fn raid_glow(&mut self, id: i32) {
        for m in &mut self.mobs {
            if raider::state(m).is_some_and(|s| s.raid_id == Some(id)) {
                m.add_effect("glowing", 1000, 1);
            }
        }
    }
    /// The `/raid check` summary line.
    pub(super) fn raid_status_text(&self, id: i32) -> String {
        let Some(r) = self.raiding.raids.get(id) else {
            return String::new();
        };
        let members: Vec<&Mob> = self.raid_members(id).collect();
        let health: f32 = members.iter().fold(0.0, |a, m| a + m.health);
        format!(
            "Num groups spawned: {} Raid omen level: {} Num mobs: {} Raid health: {} / {}",
            r.groups_spawned,
            r.omen_level,
            members.len(),
            health,
            r.total_health
        )
    }
    /// A raid's members (`groupRaiderMap`): the living raiders it counts.
    fn raid_members(&self, id: i32) -> impl Iterator<Item = &Mob> {
        self.mobs.iter().filter(move |m| {
            m.alive() && !m.removed && raider::state(m).is_some_and(|s| s.raid_id == Some(id))
        })
    }
    /// `getHealthOfLivingRaiders`, leaving out one raider.
    fn raid_living_health(&self, id: i32, except: Option<u32>) -> f32 {
        self.raid_members(id)
            .filter(|m| Some(m.id) != except)
            .fold(0.0, |a, m| a + m.health)
    }
    /// The bar update the last hurt raider of a raid asked for
    /// (`Raider.hurtServer` runs `updateBossbar` before the damage).
    fn flush_raid_bar(&mut self, id: i32) {
        let seen = self.raiding.bar_hurts.get(&id).copied().unwrap_or(0);
        let last = self
            .raid_members(id)
            .filter_map(|m| raider::state(m).map(|s| (s.hurt_seq, m.id, s.pre_hurt_health)))
            .filter(|h| h.0 > seen)
            .max_by_key(|h| h.0);
        let Some((seq, mob, before)) = last else {
            return;
        };
        let living = self.raid_living_health(id, Some(mob)) + before;
        if let Some(r) = self.raiding.raids.get_mut(id) {
            r.update_bossbar(living);
        }
        self.raiding.bar_hurts.insert(id, seq);
    }

    /// The members of every raid, for `Raids.tick`.
    fn raider_infos(&self) -> HashMap<i32, Vec<RaiderInfo>> {
        let mut out: HashMap<i32, Vec<RaiderInfo>> = HashMap::new();
        for m in &self.mobs {
            if !m.alive() || m.removed {
                continue;
            }
            let Some(s) = raider::state(m) else { continue };
            let Some(id) = s.raid_id else { continue };
            let position = block_of(m.position);
            out.entry(id).or_default().push(RaiderInfo {
                id: m.id,
                wave: s.wave,
                health: m.health,
                position,
                tick_count: m.age as i32,
                no_action_time: m.ai.no_action_time,
                ticks_outside: s.ticks_outside_raid,
                removed: false,
                other_dimension: false,
                in_level: true,
                patrol_leader: s.patrol_leader,
            });
        }
        out
    }
    /// Raiders loaded with a `RaidId` join their raid
    /// (`readAdditionalSaveData`: `addWaveMob` without counting health, and
    /// the wave's leader).
    fn register_loaded_raiders(&mut self) {
        let pending: Vec<(u32, i32, i32, f32, bool)> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && !m.removed)
            .filter_map(|m| {
                let s = raider::state(m)?;
                (!s.registered).then_some(())?;
                Some((m.id, s.raid_id?, s.wave, m.health, s.patrol_leader))
            })
            .collect();
        for (mob, id, wave, health, leader) in pending {
            let living = self.raid_living_health(id, None);
            let known = match self.raiding.raids.get_mut(id) {
                Some(r) => {
                    r.add_wave_mob(health, false, living);
                    if leader {
                        r.set_leader(wave, mob);
                    }
                    self.raiding.raids.dirty = true;
                    true
                }
                None => false,
            };
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                let s = raider::state_mut(m);
                s.registered = true;
                if !known {
                    s.raid_id = None;
                }
            }
        }
    }
    /// `ServerLevel.tick` for the raids and their raiders.
    pub(super) fn tick_raids(&mut self) {
        if self.remote.is_some() {
            return;
        }
        self.load_raids();
        self.register_loaded_raiders();
        for id in self.raiding.raids.ids() {
            self.flush_raid_bar(id);
        }
        let centers = self.village_centers();
        let infos = self.raider_infos();
        let mut raids = std::mem::take(&mut self.raiding.raids);
        let actions = {
            let lvl = Lvl {
                game: self,
                difficulty: self.difficulty_id(),
                centers: &centers,
            };
            let members = |id: i32| infos.get(&id).cloned().unwrap_or_default();
            raids.tick(&lvl, self.ui.rules.bool("raids"), &members)
        };
        self.raiding.raids = raids;
        for (id, action) in actions {
            self.apply_raid_action(id, action);
        }
        self.sync_raiders(&centers);
        let infos = self.raider_infos();
        self.sync_villager_raids(&infos);
    }
    fn apply_raid_action(&mut self, id: i32, action: Action) {
        match action {
            Action::Spawn { pos, wave, spawns } => self.spawn_wave(id, pos, wave, &spawns),
            Action::Horn { pos, .. } => self.raid_horn(id, pos),
            Action::Remove { id: mob } => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                    let s = raider::state_mut(m);
                    s.raid_id = None;
                    s.active_raid = false;
                }
            }
            Action::SetOutside { id: mob, ticks } => {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                    raider::state_mut(m).ticks_outside_raid = ticks;
                }
            }
            // Raid.tick on a victory: each hero who is still alive.
            Action::Victory { heroes, amplifier } => self.for_each_player(|g| {
                if heroes.contains(&g.acting_uuid()) && !g.spectator() {
                    let mut effect =
                        EffectInstance::new("hero_of_the_village", raid::HERO_DURATION, amplifier);
                    effect.visible = false;
                    g.potions.effects.add(effect);
                    g.stats.custom("raid_win", 1);
                    g.trigger_player("minecraft:hero_of_the_village");
                }
            }),
        }
    }
    /// `Raid.playSound`: the horn, 13 blocks away from the player toward the
    /// wave, for players within 64 blocks or in the raid.
    fn raid_horn(&mut self, id: i32, pos: [i32; 3]) {
        let Some(p) = self.level_players().first().map(|p| p.position) else {
            return;
        };
        let in_raid = self.raiding.raids.get(id).is_some_and(|r| r.bar.player);
        let (rx, rz) = (f64::from(pos[0]) + 0.5, f64::from(pos[2]) + 0.5);
        let distance = ((rx - p.x).powi(2) + (rz - p.z).powi(2)).sqrt();
        if distance <= 64.0 || in_raid {
            let k = 13.0 / distance;
            let at = [p.x + k * (rx - p.x), p.y, p.z + k * (rz - p.z)];
            self.audio
                .play("minecraft:event.raid.horn", Some(at), 64.0, 1.0);
        }
    }
    /// `Raid.spawnGroup`: a wave of raiders at `pos`, the leader wearing the
    /// banner, ravagers carrying their riders.
    fn spawn_wave(&mut self, raid_id: i32, pos: [i32; 3], wave: i32, spawns: &[Spawn]) {
        let odds = self
            .raiding
            .raids
            .get(raid_id)
            .map_or(0.0, raid::Raid::enchant_odds);
        for sp in spawns {
            let Some(id) = self.spawn_raider(sp.kind, pos, raid_id, wave, sp.leader, odds) else {
                continue;
            };
            if sp.leader
                && let Some(r) = self.raiding.raids.get_mut(raid_id)
            {
                r.set_leader(wave, id);
            }
            if let Some(kind) = sp.rider
                && let Some(rider) = self.spawn_raider(kind, pos, raid_id, wave, false, odds)
            {
                self.mount_mob(rider, id);
            }
        }
    }
    /// `Raid.joinRaid` for a new raider: placed at the block's centre one
    /// above, finalized for an event, its raid buffs applied.
    fn spawn_raider(
        &mut self,
        kind: RaiderType,
        pos: [i32; 3],
        raid_id: i32,
        wave: i32,
        leader: bool,
        odds: f32,
    ) -> Option<u32> {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ];
        let id = self.spawn_event_mob(kind.entity(), at, leader, raider::Reason::Event)?;
        let m = self.mobs.iter_mut().find(|m| m.id == id)?;
        let s = raider::state_mut(m);
        s.raid_id = Some(raid_id);
        s.wave = wave;
        s.can_join_raid = true;
        s.ticks_outside_raid = 0;
        s.active_raid = true;
        s.registered = true;
        m.on_ground = true;
        self.apply_raid_buffs(id, kind, wave, odds);
        Some(id)
    }
    /// `EntityType.create` then `finalizeSpawn` for a patrol or a raid: the
    /// mob keeps the default rotation; a leader is set before finalizing so
    /// it wears the banner.
    fn spawn_event_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        leader: bool,
        reason: raider::Reason,
    ) -> Option<u32> {
        let t = self.res.mob_types().get(kind).cloned()?;
        self.next_entity += 1;
        let mut mob = Mob::new(self.next_entity, t, Vec3::new(at[0], at[1], at[2]), 0.0);
        if leader {
            raider::set_patrol_leader(&mut mob, true);
            if reason == raider::Reason::Patrol {
                // findPatrolTarget runs before the mob is placed: the target
                // is around the world origin.
                raider::find_patrol_target_from(&mut mob, [0, 0, 0]);
            }
        }
        let facts = self.biome_facts(
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        );
        let difficulty = self.local_difficulty(at);
        spawning::finalize(&mut mob, &facts, &difficulty);
        raider::finalize(&mut mob, reason);
        let id = mob.id;
        self.mobs.push(mob);
        self.native_mobs_joined(self.mobs.len() - 1, false);
        self.enchant_spawn_equipment(id);
        self.halloween_spawn_head(id);
        Some(id)
    }

    /// Per-tick raid bookkeeping of every raider (and the ravagers they
    /// ride): the raid's state for the goals, joining and recruiting,
    /// the banner of a fallen captain, idle time.
    fn sync_raiders(&mut self, centers: &HashSet<(i32, i32, i32)>) {
        let now = self.world.game_time;
        let golems: HashSet<u32> = self
            .mobs
            .iter()
            .filter(|m| m.species() == Species::IronGolem)
            .map(|m| m.id)
            .collect();
        // Living leaders and carried riders.
        let mut leaders: HashSet<(i32, i32)> = HashSet::new();
        let mut ridden: HashSet<u32> = HashSet::new();
        let mut positions: Vec<(u32, Vec3, bool)> = Vec::new();
        for m in &self.mobs {
            let Some(s) = raider::state(m) else { continue };
            if !m.alive() {
                continue;
            }
            if let (Some(id), true) = (s.raid_id, s.patrol_leader) {
                leaders.insert((id, s.wave));
            }
            if let Some(v) = m.extra.mount {
                ridden.insert(v);
            }
            positions.push((m.id, m.position, s.active_raid));
        }
        let banners: Vec<(usize, u32, Vec3)> = self
            .dropped
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                &*e.stack.item == raider::BANNER
                    && e.stack.data().is_some_and(|d| d.ominous_banner)
                    && e.pickup_delay == 0
            })
            .map(|(i, e)| (i, e.id, e.position))
            .collect();
        let mut taken: Vec<usize> = Vec::new();
        let mut collectors: Vec<(usize, u32)> = Vec::new();
        let mut recruit: Vec<(i32, Vec3)> = Vec::new();
        let mut joins: Vec<(u32, i32, f32)> = Vec::new();
        let mut new_leaders: Vec<(i32, i32, u32)> = Vec::new();
        let homes = self.raid_homes();
        for m in &mut self.mobs {
            let species = m.species();
            if !raider::is_raider(species) {
                continue;
            }
            let alive = m.alive();
            let position = m.position;
            let block = block_of(position);
            let in_village = sections_to_village(centers, section(block)) <= 1;
            let has_rider = ridden.contains(&m.id);
            let target_golem = matches!(m.ai.target, Some(horizon_client::mobs::Who::Mob(t)) if golems.contains(&t));
            let target_player = m.ai.target == Some(horizon_client::mobs::Who::Player);
            let (health, range) = (m.health, m.attrs.follow_range);
            let raids = &mut self.raiding.raids;
            let s = m.extra.raider.get_or_insert_with(Box::default);
            s.rider = has_rider;
            s.in_village = in_village;
            // Vindicator.customServerAiStep: `ServerLevel.isRaided`.
            s.raided = species == Species::Vindicator && raids.nearby(block).is_some();
            if s.raid_id.is_some_and(|id| raids.get(id).is_none()) {
                s.raid_id = None;
                s.ticks_outside_raid = 0;
            }
            match s.raid_id.and_then(|id| raids.get(id)) {
                Some(r) => {
                    s.active_raid = r.active;
                    s.raid_over = r.is_over();
                    s.raid_loss = r.is_loss();
                    s.raid_center = r.center;
                    s.homes = s.raid_id.and_then(|id| homes.get(&id).cloned());
                }
                None => {
                    s.active_raid = false;
                    s.raid_over = false;
                    s.raid_loss = false;
                    s.homes = None;
                    s.banner_at = None;
                }
            }
            if !alive {
                continue;
            }
            // Raider.aiStep: a raider fighting a player or golem in a raid is
            // never idle. The per-tick growth (+1 serverAiStep, +2
            // Raider.updateNoActionTime) is added by the mob's own step.
            if s.raid_id.is_some() && (target_player || target_golem) {
                m.ai.no_action_time = 0;
            }
            // Raider.aiStep: join the raid the raider stands in.
            if s.raid_id.is_none()
                && s.can_join_raid
                && now % 20 == 0
                && m.ai.no_action_time <= 2400
                && let Some((id, _)) = raids.nearby(block)
            {
                joins.push((m.id, id, health));
            }
            // PathfindToRaidGoal.tick: recruitNearby, only while the goal runs.
            if std::mem::take(&mut s.recruit_due)
                && s.active_raid
                && let Some(id) = s.raid_id
            {
                recruit.push((id, position));
            }
            // ObtainRaidLeaderBannerGoal.
            s.banner_at = None;
            if let Some(id) = s.raid_id
                && s.active_raid
                && !s.raid_over
                && raider::can_be_leader(species)
                && m.extra.head.as_deref() != Some(raider::BANNER)
                && raids.get(id).is_some_and(|r| r.leader(s.wave).is_none())
                && !leaders.contains(&(id, s.wave))
            {
                let in_reach = |(i, _, p): &&(usize, u32, Vec3)| {
                    !taken.contains(i)
                        && (p.x - position.x).abs() <= range
                        && (p.z - position.z).abs() <= range
                        && (p.y - position.y).abs() <= 8.0
                };
                let by_distance = |a: &&(usize, u32, Vec3), b: &&(usize, u32, Vec3)| {
                    a.2.distance_squared(position)
                        .total_cmp(&b.2.distance_squared(position))
                };
                // The unreachable cache only picks the navigation target;
                // a banner next to the raider is still picked up.
                let near = banners
                    .iter()
                    .filter(in_reach)
                    .filter(|(_, item, _)| {
                        !s.unreachable_banners
                            .iter()
                            .any(|&(b, until)| b == *item && now < until)
                    })
                    .min_by(by_distance);
                let nearest = banners.iter().filter(in_reach).min_by(by_distance);
                let adjacent =
                    nearest.filter(|(_, _, p)| p.distance_squared(position) < 1.414 * 1.414);
                if let Some((i, item, p)) = adjacent.or(near) {
                    s.banner_at = Some([p.x, p.y, p.z]);
                    s.banner_item = *item;
                    if p.distance_squared(position) < 1.414 * 1.414 {
                        // pickUpItem: the banner becomes the wave's leader.
                        taken.push(*i);
                        collectors.push((*i, m.id));
                        leaders.insert((id, s.wave));
                        new_leaders.push((id, s.wave, m.id));
                        s.banner_at = None;
                        s.patrol_leader = true;
                        s.patrolling = true;
                        // wear_banner: head id + ominous stack for SetEquipment (#4567).
                        raider::wear_banner(m);
                    }
                }
            }
        }
        for (i, mob) in collectors {
            let item = self.dropped[i].clone();
            self.add_pickup(
                super::pickups::Taken::Item(Box::new(item)),
                super::pickups::Collector::Mob(mob),
            );
        }
        taken.sort_unstable();
        for i in taken.into_iter().rev() {
            self.remove_dropped(i);
        }
        for (id, wave, mob) in new_leaders {
            if let Some(r) = self.raiding.raids.get_mut(id) {
                r.set_leader(wave, mob);
            }
        }
        // PathfindToRaidGoal.recruitNearby: raiders nearby in no active raid
        // join this one at its current wave.
        for (id, at) in recruit {
            for (mid, p, active) in &positions {
                if !active
                    && (p.x - at.x).abs() <= 16.0
                    && (p.y - at.y).abs() <= 16.0
                    && (p.z - at.z).abs() <= 16.0
                    && let Some(m) = self.mobs.iter().find(|m| m.id == *mid)
                    && raider::state(m).is_some_and(|s| s.can_join_raid && s.raid_id != Some(id))
                    && m.alive()
                    && m.ai.no_action_time <= 2400
                {
                    joins.push((*mid, id, m.health));
                }
            }
        }
        for (mob, id, health) in joins {
            self.join_raid(mob, id, health);
        }
    }
    /// `Raid.joinRaid` for a raider already in the world: it counts toward
    /// the bar and takes the raid's current wave.
    fn join_raid(&mut self, mob: u32, id: i32, health: f32) {
        let Some(wave) = self.raiding.raids.get(id).map(|r| r.groups_spawned) else {
            return;
        };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) else {
            return;
        };
        let s = raider::state_mut(m);
        if s.raid_id == Some(id) {
            return;
        }
        s.raid_id = Some(id);
        s.wave = wave;
        s.can_join_raid = true;
        s.ticks_outside_raid = 0;
        s.registered = true;
        let living = self.raid_living_health(id, None);
        if let Some(r) = self.raiding.raids.get_mut(id) {
            r.add_wave_mob(health, true, living);
        }
        self.raiding.raids.dirty = true;
    }
    /// Beds within 64 blocks of each raid's centre (the `HOME` points of
    /// interest), rescanned every five seconds.
    fn raid_homes(&mut self) -> HashMap<i32, std::sync::Arc<Vec<[i32; 3]>>> {
        let now = self.world.game_time;
        let ids: Vec<(i32, [i32; 3])> = self
            .raiding
            .raids
            .map
            .iter()
            .filter(|(_, r)| r.active && !r.is_over() && r.started)
            .map(|(id, r)| (*id, r.center))
            .collect();
        self.raiding
            .homes
            .retain(|id, _| ids.iter().any(|(i, _)| i == id));
        for (id, c) in ids {
            if self
                .raiding
                .homes
                .get(&id)
                .is_some_and(|(at, _)| now - at < 100)
            {
                continue;
            }
            let mut beds = Vec::new();
            let mut known: HashMap<u32, bool> = HashMap::new();
            for y in c[1] - 32..=c[1] + 32 {
                for z in c[2] - 64..=c[2] + 64 {
                    for x in c[0] - 64..=c[0] + 64 {
                        let state = self.world.block(x, y, z);
                        if self.res.facts().is_air(state) {
                            continue;
                        }
                        let bed = *known.entry(state.0).or_insert_with(|| {
                            self.res.facts().get(state).name.ends_with("_bed")
                                && self.pack.assets.registry.state(state).is_some_and(|s| {
                                    s.properties.get("part").map(String::as_str) == Some("head")
                                })
                        });
                        if bed {
                            beds.push([x, y, z]);
                        }
                    }
                }
            }
            self.raiding
                .homes
                .insert(id, (now, std::sync::Arc::new(beds)));
        }
        self.raiding
            .homes
            .iter()
            .map(|(id, (_, v))| (*id, v.clone()))
            .collect()
    }
    /// `Raider.die`: the wave loses its leader, the killing player becomes
    /// a hero of the village, and the raider leaves the raid (its health
    /// stays in the bar's total).
    pub(super) fn raider_died(&mut self, m: &Mob, by_player: bool) {
        let Some(s) = raider::state(m) else {
            return;
        };
        let Some(id) = s.raid_id else {
            return;
        };
        let (leader, wave) = (s.patrol_leader, s.wave);
        self.flush_raid_bar(id);
        let living = self.raid_living_health(id, Some(m.id));
        let uuid = self.level.player_uuid;
        if let Some(r) = self.raiding.raids.get_mut(id) {
            if leader {
                r.remove_leader(wave);
            }
            if by_player && let Some(uuid) = uuid {
                r.add_hero(uuid);
            }
            r.remove_from_raid(m.health, false, living);
            self.raiding.raids.dirty = true;
        }
        if let Some(dead) = self.mobs.iter_mut().find(|x| x.id == m.id) {
            raider::state_mut(dead).raid_id = None;
        }
    }

    /// What `getRaidAt` tells each villager where it stands (its brain's
    /// `SetRaidStatus`, `ResetRaidStatus`, `ReactToBell` and raid packages
    /// read it).
    fn sync_villager_raids(&mut self, infos: &HashMap<i32, Vec<RaiderInfo>>) {
        let raids = &self.raiding.raids;
        let mut ongoing = Vec::new();
        for m in &mut self.mobs {
            if m.species() != Species::Villager {
                continue;
            }
            let at = block_of(m.position);
            let (status, wave) = match raids.nearby(at) {
                Some((id, r)) => {
                    let alive = infos.get(&id).map_or(0, Vec::len);
                    let wave = r.has_first_wave_spawned() && !r.is_between_waves(alive);
                    let status = if r.is_victory() {
                        mobs_villager::RAID_VICTORY
                    } else if r.is_loss() {
                        mobs_villager::RAID_LOSS
                    } else if wave {
                        mobs_villager::RAID
                    } else {
                        mobs_villager::PRE_RAID
                    };
                    (status, wave)
                }
                None => (mobs_villager::RAID_NONE, false),
            };
            let ext = mobs_villager::ext_mut(m);
            ext.raid = status;
            ext.raid_wave = wave;
            if !m.no_ai && matches!(status, mobs_villager::PRE_RAID | mobs_villager::RAID) {
                ongoing.push(m.id);
            }
        }
        // Villager.customServerAiStep: a 1 in 100 chance each tick, in an
        // ongoing raid, of entity event 42 (`handleEntityEvent` splashes).
        for id in ongoing {
            if self.particles.random().next_int_bound(100) == Ok(0) {
                self.mob_raid_splash(id);
            }
        }
    }
    /// Entity event 42 for villager `id`: the guests get the byte, and this
    /// game draws `AbstractVillager.addParticlesAroundSelf(SPLASH)` itself.
    fn mob_raid_splash(&mut self, id: u32) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        self.lan_entity_event(id as i32, 42, at);
        self.particles_around_mob(id, "splash", horizon_client::particles::Burst::VILLAGER);
    }

    /// The raid bar a living player at `at` is in (`Raid.getPlayers`): red,
    /// notched in tens. The name is the translate component the client draws.
    pub(super) fn raid_boss_at(&self, at: [f64; 3], alive: bool) -> Option<(i32, Tag, f32)> {
        if !alive || self.dim != Dim::Overworld {
            return None;
        }
        let block = block_of(Vec3::new(at[0], at[1], at[2]));
        let (id, raid) = self.raiding.raids.nearby(block)?;
        if !raid.bar.visible || raid.is_stopped() {
            return None;
        }
        Some((id, raid_name_tag(raid.bar.name), raid.bar.progress))
    }
    /// A firework a villager launches to celebrate (`CelebrateVillagersSurvivedRaid`).
    pub(super) fn villager_firework(&mut self, at: [f64; 3], color: usize, flight: u8) {
        let mut stack = horizon_client::items::ItemStack::new("minecraft:firework_rocket", 1);
        stack.data_mut().fireworks = Some((
            flight,
            vec![horizon_client::particles::FireworkExplosion {
                shape: horizon_client::particles::FireworkShape::Burst,
                colors: vec![horizon_client::particles::DYE_FIREWORK_COLORS[color & 15].1],
                fade: Vec::new(),
                trail: false,
                twinkle: false,
            }],
        ));
        let rocket = self.new_rocket(Vec3::new(at[0], at[1], at[2]), &stack, false);
        self.rockets.push(rocket);
    }
    /// `BehaviorUtils.throwItem`: a hero's gift tossed toward the player.
    pub(super) fn throw_gift(&mut self, table: &str, from: [f64; 3], to: [f64; 3]) {
        let drops = self
            .res
            .items()
            .loot_table(table, false, None, &mut self.world.random);
        let d = Vec3::new(to[0] - from[0], to[1] - from[1], to[2] - from[2]).normalize();
        for stack in drops {
            self.spawn_item(stack, from, [d.x * 0.3, d.y * 0.3, d.z * 0.3], 10);
        }
    }
}

impl Game<'_> {
    /// Sections holding a claimed village point of interest (a job site,
    /// bed or bell a villager has taken): `PoiManager.isVillageCenter`.
    pub(super) fn village_centers(&self) -> HashSet<(i32, i32, i32)> {
        let mut out = HashSet::new();
        for m in &self.mobs {
            if m.species() != Species::Villager || !m.alive() {
                continue;
            }
            for p in [m.extra.home, m.extra.bed, m.extra.meeting]
                .into_iter()
                .flatten()
            {
                out.insert(section(p));
            }
        }
        out
    }
    /// `PatrolSpawner.tick` (`horizon_client::patrol`) on the game level:
    /// the pillagers it accepts are created after the pass, finalized for a
    /// patrol.
    pub(super) fn tick_patrol_spawner(&mut self) {
        if self.dim != Dim::Overworld {
            return;
        }
        let attributes = self.gameplay_attributes();
        let biome_ok = |game: &Game, at: [i32; 3]| {
            let biome = game
                .world
                .chunks
                .get(&(at[0] >> 4, at[2] >> 4))
                .map(|c| {
                    game.res
                        .biomes()
                        .get(c.biome(at[0], at[1], at[2]))
                        .name
                        .clone()
                })
                .unwrap_or_default();
            attributes.value(
                horizon_client::patrol::CAN_PILLAGER_PATROL_SPAWN,
                game.dim.id(),
                &biome,
                game.world.day_time,
            )
        };
        let enemies = self.ui.rules.bool("spawn_mobs") && self.ui.rules.bool("spawn_monsters");
        let mut random = std::mem::replace(
            &mut self.world.random,
            horizon_core::random::LegacyRandom::new(0),
        );
        let mut members: Vec<Member> = Vec::new();
        let mut spawner = std::mem::take(&mut self.raiding.patrol);
        {
            let level = PatrolCx {
                level: self.natural_level(),
                centers: self.village_centers(),
                can_spawn: &biome_ok,
            };
            let mut spawn =
                |l: &PatrolCx, m: Member, _: &mut horizon_core::random::LegacyRandom| {
                    // EntityType.create makes no monsters in peaceful.
                    if l.level.game.difficulty_id() == 0 {
                        return false;
                    }
                    members.push(m);
                    true
                };
            spawner.tick(&level, enemies, &mut random, &mut spawn);
        }
        self.raiding.patrol = spawner;
        self.world.random = random;
        for m in members {
            let at = [
                f64::from(m.pos[0]),
                f64::from(m.pos[1]),
                f64::from(m.pos[2]),
            ];
            self.spawn_event_mob("minecraft:pillager", at, m.leader, raider::Reason::Patrol);
        }
    }
    /// `VillageSiege.tick`: at midnight one night in ten, 20 zombies spawn
    /// two ticks apart near a village 32 blocks from a player standing in it.
    pub(super) fn tick_village_siege(&mut self) {
        if self.dim != Dim::Overworld {
            return;
        }
        let enemies = self.ui.rules.bool("spawn_mobs") && self.ui.rules.bool("spawn_monsters");
        if self.sky_darken() < 4 || !enemies {
            self.raiding.siege.tonight = false;
            self.raiding.siege.set_up = false;
            return;
        }
        // ClockTimeMarkers.ROLL_VILLAGE_SIEGE: tick 18000 of the 24000 tick day.
        if self.world.day_time.rem_euclid(24000) == 18000 {
            self.raiding.siege.tonight = self.world.random.next_int_bound(10).unwrap_or(1) == 0;
        }
        if !self.raiding.siege.tonight {
            return;
        }
        if !self.raiding.siege.set_up {
            if !self.try_to_setup_siege() {
                return;
            }
            self.raiding.siege.set_up = true;
        }
        if self.raiding.siege.next_spawn_time > 0 {
            self.raiding.siege.next_spawn_time -= 1;
            return;
        }
        self.raiding.siege.next_spawn_time = 2;
        if self.raiding.siege.zombies_to_spawn > 0 {
            self.try_spawn_siege_zombie();
            self.raiding.siege.zombies_to_spawn -= 1;
        } else {
            self.raiding.siege.tonight = false;
        }
    }
    /// `ServerLevel.isVillage`: within one section of a village.
    fn is_village(centers: &HashSet<(i32, i32, i32)>, pos: [i32; 3]) -> bool {
        sections_to_village(centers, section(pos)) <= 1
    }
    /// `VillageSiege.tryToSetupSiege`: the first player standing in a village
    /// picks a spot 32 blocks away (ten tries); true when one stood in a
    /// village, even if no spot was found.
    fn try_to_setup_siege(&mut self) -> bool {
        let centers = self.village_centers();
        for player in self.level_players() {
            if player.spectator {
                continue;
            }
            let center = block_of(player.position);
            if !Self::is_village(&centers, center)
                || self.natural_level().biome_tag(
                    "minecraft:without_zombie_sieges",
                    center[0],
                    center[1],
                    center[2],
                )
            {
                continue;
            }
            for _ in 0..10 {
                let angle = self.world.random.next_float() * std::f32::consts::TAU;
                let spot = [
                    center[0] + (angle.cos() * 32.0).floor() as i32,
                    center[1],
                    center[2] + (angle.sin() * 32.0).floor() as i32,
                ];
                if self.find_siege_spawn_pos(&centers, spot).is_some() {
                    self.raiding.siege.spot = spot;
                    self.raiding.siege.next_spawn_time = 0;
                    self.raiding.siege.zombies_to_spawn = 20;
                    break;
                }
            }
            return true;
        }
        false
    }
    /// `VillageSiege.trySpawn`: one zombie, in a random yaw.
    fn try_spawn_siege_zombie(&mut self) {
        let centers = self.village_centers();
        let spot = self.raiding.siege.spot;
        let Some(at) = self.find_siege_spawn_pos(&centers, spot) else {
            return;
        };
        let Some(id) = self.spawn_event_mob("minecraft:zombie", at, false, raider::Reason::Event)
        else {
            return;
        };
        let yaw = self.world.random.next_float() * 360.0;
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            (m.yaw, m.previous_yaw, m.body_yaw, m.previous_body_yaw) = (yaw, yaw, yaw, yaw);
            (m.head_yaw, m.previous_head_yaw) = (yaw, yaw);
        }
        self.spawn_jockeys(id, false);
    }
    /// `VillageSiege.findRandomSpawnPos`: ten tries within 8 blocks of the
    /// spot for a village column where a zombie may spawn.
    fn find_siege_spawn_pos(
        &mut self,
        centers: &HashSet<(i32, i32, i32)>,
        spot: [i32; 3],
    ) -> Option<[f64; 3]> {
        let mut random = std::mem::replace(
            &mut self.world.random,
            horizon_core::random::LegacyRandom::new(0),
        );
        let mut found = None;
        for _ in 0..10 {
            let x = spot[0] + random.next_int_bound(16).unwrap_or(0) - 8;
            let z = spot[2] + random.next_int_bound(16).unwrap_or(0) - 8;
            if !self.world.chunks.contains_key(&(x >> 4, z >> 4)) {
                continue;
            }
            let y = self.surface_height(x, z);
            if Self::is_village(centers, [x, y, z])
                && spawn_rules::check_spawn_rules(
                    Species::Zombie,
                    &self.natural_level(),
                    spawn_rules::Reason::Event,
                    &mut random,
                    x,
                    y,
                    z,
                )
            {
                found = Some([f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]);
                break;
            }
        }
        self.world.random = random;
        found
    }
    /// A raider's ominous banner (drop chance 2.0: always dropped) and a
    /// patrol captain's ominous bottle (`entities/pillager`).
    /// `OminousBottleAmplifier.onConsume`: Bad Omen for 100 minutes at the
    /// bottle's amplifier, then the dispose sound; the bottle is used up.
    pub(super) fn drink_ominous_bottle(
        &mut self,
        slot: usize,
        stack: &horizon_client::items::ItemStack,
    ) {
        let amplifier = stack.data().and_then(|d| d.ominous_amplifier).unwrap_or(0);
        let mut effect =
            horizon_client::effects::EffectInstance::new("bad_omen", 120_000, i32::from(amplifier));
        effect.visible = false;
        let new = self.potions.effects.get("bad_omen").is_none();
        self.potions.effects.add(effect);
        let eye = self.eye_position().unwrap_or_default();
        // MobEffect.onEffectAdded: Bad Omen's sound when it is new.
        if new {
            self.audio
                .play("minecraft:event.mob_effect.bad_omen", Some(eye), 1.0, 1.0);
        }
        self.audio
            .play("minecraft:item.ominous_bottle.dispose", Some(eye), 1.0, 1.0);
        if !self.creative
            && let Some(s) = self.stack_in_mut(slot).as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                *self.stack_in_mut(slot) = None;
            }
        }
    }
}

/// `Raid.raidEvent`'s name: the raid title, the remaining-raiders line, or
/// the victory and defeat lines.
fn raid_name_tag(name: BarName) -> Tag {
    let key = |k: &str| Tag::compound([("translate", Tag::String(k.into()))]);
    match name {
        BarName::Raid => key("event.minecraft.raid"),
        BarName::Victory => key("event.minecraft.raid.victory.full"),
        BarName::Defeat => key("event.minecraft.raid.defeat.full"),
        BarName::Remaining(n) => Tag::compound([
            ("translate", Tag::String("event.minecraft.raid".into())),
            (
                "extra",
                Tag::List(vec![
                    Tag::String(" - ".into()),
                    Tag::compound([
                        (
                            "translate",
                            Tag::String("event.minecraft.raid.raiders_remaining".into()),
                        ),
                        (
                            "with",
                            Tag::List(vec![Tag::String(n.to_string().as_str().into())]),
                        ),
                    ]),
                ]),
            ),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_to_village_is_a_chebyshev_distance_capped_at_seven() {
        let mut centers = HashSet::new();
        assert_eq!(sections_to_village(&centers, (0, 0, 0)), 7);
        centers.insert(section([40, 64, 40]));
        assert_eq!(sections_to_village(&centers, (2, 4, 2)), 0);
        assert_eq!(sections_to_village(&centers, (4, 4, 2)), 2);
        assert_eq!(sections_to_village(&centers, (-10, 4, 2)), 7);
        assert_eq!(section([-1, -17, 15]), (-1, -2, 0));
    }
}
