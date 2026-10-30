//! Monster spawners in the game: `SpawnerBlockEntity.serverTick` and
//! `clientTick` for every spawner (`mobs::spawner`), the mobs they add, a
//! spawn egg used on one (`SpawnEggItem.useOn`), their events and the mob
//! spinning inside (`SpawnerRenderer`).
use super::Game;
use horizon_client::{
    Result,
    containers::BlockEntity,
    mobs::{
        Mob, Who,
        spawner::{SpawnDataExt, Spawner, SpawnerAdded, SpawnerEvent, SpawnerExt, SpawnerSpawn},
        spawning,
        trial_spawner::{TrialSpawnerExt, TrialState},
    },
    particles::{ParticleOptions, ParticleType},
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::nbt::Tag;
use std::collections::{HashMap, HashSet};

type Pos = (i32, i32, i32);

/// What a spawner tick decided to add, created once the level is free.
pub(super) enum Planned {
    /// `finalizeSpawn` ran on a fresh mob; the spawn data's `equipment`.
    Finalized(Mob, Option<Tag>),
    /// A mob loaded from the spawn data (with its passengers) and its
    /// `equipment`.
    Loaded(Tag, Option<Tag>),
}

/// What the spawner code keeps between ticks.
#[derive(Default)]
pub(super) struct SpawnerState {
    /// Every entity type id (`EntityType.byString`), read once.
    entity_types: Option<HashSet<String>>,
    /// The mob shown in each spawner and the id it was made for
    /// (`getOrCreateDisplayEntity`).
    displays: HashMap<Pos, (String, Mob)>,
}

impl Game<'_> {
    /// `EntityType.byString`: the type is in the registry.
    pub(super) fn entity_type_exists(&self, id: &str) -> bool {
        self.spawners
            .entity_types
            .as_ref()
            .is_some_and(|t| t.contains(id))
    }
    pub(super) fn load_entity_types(&mut self) {
        if self.spawners.entity_types.is_some() {
            return;
        }
        let path = self.options.resources.join("entity-types.json");
        let types = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| {
                v.as_object().map(|o| {
                    o.keys()
                        .map(|k| horizon_client::command::args::normalize(k))
                        .collect()
                })
            })
            .unwrap_or_default();
        self.spawners.entity_types = Some(types);
    }
    /// The server tick of the spawner at `key`. Its client tick comes after
    /// every block entity's (`client_tick_spawners`).
    ///
    /// The caller only reaches this for a `BlockEntity::Spawner`, and changing
    /// the block removes or replaces that entity, so the block is not read
    /// again here.
    pub(super) fn tick_spawner(&mut self, key: Pos) -> Result<()> {
        let Some(BlockEntity::Spawner(mut spawner)) = self.world.block_entities.take(&key) else {
            return Ok(());
        };
        let result = self.server_tick_spawner(key, &mut spawner);
        self.world
            .block_entities
            .put(key, BlockEntity::Spawner(spawner));
        result
    }
    /// The client ticks of these spawners, then the shown mobs of spawners
    /// that are gone go.
    pub(super) fn client_tick_spawners(&mut self, keys: &[Pos]) {
        for &key in keys {
            self.client_tick_spawner(key);
        }
        let entities = &self.world.block_entities;
        self.spawners.displays.retain(|k, _| {
            matches!(
                entities.get(k),
                Some(BlockEntity::Spawner(_) | BlockEntity::TrialSpawner(_))
            )
        });
    }
    fn server_tick_spawner(&mut self, key: Pos, spawner: &mut Spawner) -> Result<()> {
        // `BaseSpawner.serverTick` does nothing without an alive player in
        // range (`isNearPlayer`) or with the rule off: no level to build for it.
        let at = [key.0, key.1, key.2].map(|c| f64::from(c) + 0.5);
        let range = f64::from(spawner.required_player_range);
        if !self.alive_player_near(at[0], at[1], at[2], range)
            || !self.ui.rules.bool("spawner_blocks_work")
        {
            return Ok(());
        }
        let pos = [key.0, key.1, key.2];
        let mut random = std::mem::replace(
            &mut self.world.random,
            horizon_core::random::LegacyRandom::new(0),
        );
        let mut events = Vec::new();
        let mut planned: Vec<Planned> = Vec::new();
        {
            let level = self.natural_level();
            let mut spawn = |l: &super::natural_spawning::GameLevel, r: &SpawnerSpawn| {
                l.game.plan_spawner_mob(l, r, &mut planned)
            };
            spawner.server_tick(&level, &mut random, pos, &mut spawn, &mut events);
        }
        self.world.random = random;
        let ids = self.add_planned(planned);
        self.apply_spawner_events(key, events, &ids)
    }
    /// Creates the mobs a spawner tick planned; their provisional ids
    /// (counting down from u32::MAX) map to the mobs' ids.
    pub(super) fn add_planned(&mut self, planned: Vec<Planned>) -> HashMap<u32, u32> {
        let mut ids = HashMap::new();
        for (i, plan) in planned.into_iter().enumerate() {
            let provisional = u32::MAX - i as u32;
            let id = match plan {
                Planned::Finalized(mut mob, equipment) => {
                    self.next_entity += 1;
                    mob.id = self.next_entity;
                    let id = mob.id;
                    self.mobs.push(mob);
                    self.enchant_spawn_equipment(id);
                    self.spawn_jockeys(id, true);
                    Some((id, equipment))
                }
                Planned::Loaded(tag, equipment) => {
                    self.load_mob(tag, None).map(|id| (id, equipment))
                }
            };
            if let Some((id, equipment)) = id {
                // `nextSpawnData.getEquipment().ifPresent(mob::equip)`.
                if let Some(table) = equipment {
                    self.equip_from_table(id, &table);
                }
                ids.insert(provisional, id);
            }
        }
        ids
    }
    /// The spawner's `loadEntityRecursive` and `tryAddFreshEntityWithPassengers`:
    /// the mob it will add (created after the tick) and its box.
    pub(super) fn plan_spawner_mob(
        &self,
        l: &super::natural_spawning::GameLevel,
        r: &SpawnerSpawn,
        planned: &mut Vec<Planned>,
    ) -> Option<SpawnerAdded> {
        let kind = self.res.mob_types().get(r.id)?.clone();
        let [x, y, z] = r.position;
        let at = Vec3::new(x, y, z);
        let bounds = if r.finalize {
            let mut mob = Mob::new(0, kind, at, r.yaw);
            let facts = self.biome_facts(x.floor() as i32, y.floor() as i32, z.floor() as i32);
            let difficulty = self.local_difficulty(r.position);
            spawning::finalize(&mut mob, &facts, &difficulty);
            // TrialSpawner.spawnMob: setPersistenceRequired.
            mob.persistent = r.trial;
            horizon_client::mobs::raider::finalize(
                &mut mob,
                horizon_client::mobs::raider::Reason::Other,
            );
            // Shulker.finalizeSpawn faces it south.
            if mob.species() == horizon_client::mobs::Species::Shulker {
                mob.yaw = 0.0;
                mob.previous_yaw = 0.0;
                mob.body_yaw = 0.0;
                mob.previous_body_yaw = 0.0;
                mob.head_yaw = 0.0;
                mob.previous_head_yaw = 0.0;
            }
            let bounds = mob.bounds();
            planned.push(Planned::Finalized(mob, r.equipment.cloned()));
            bounds
        } else {
            // A UUID already in the level is refused.
            let mut tag = r.entity.clone();
            if let Some(uuid) = horizon_client::entity_nbt::uuid(&Tag::Compound(tag.clone()))
                && self.persist.uuids.values().any(|u| *u == uuid)
            {
                return None;
            }
            let double = |v: f64| Tag::Double(v.to_bits());
            tag.insert(
                "Pos".into(),
                Tag::List(vec![double(x), double(y), double(z)]),
            );
            tag.insert(
                "Rotation".into(),
                Tag::List(vec![
                    Tag::Float(r.yaw.to_bits()),
                    Tag::Float(0f32.to_bits()),
                ]),
            );
            if r.trial {
                tag.insert("PersistenceRequired".into(), Tag::Byte(1));
            }
            let tag = Tag::Compound(tag);
            let mob = horizon_client::entity_nbt::mob_from_nbt(&tag, self.res.mob_types(), 0)?;
            let bounds = mob.bounds();
            planned.push(Planned::Loaded(tag, r.equipment.cloned()));
            bounds
        };
        l.pending.borrow_mut().push(bounds);
        l.pending_ids.borrow_mut().push((r.id.to_owned(), bounds));
        Some(SpawnerAdded {
            id: u32::MAX - (planned.len() as u32 - 1),
            bounds,
            mob: true,
        })
    }
    /// What a spawner's tick or a spawn egg did, shown and sent.
    fn apply_spawner_events(
        &mut self,
        key: Pos,
        events: Vec<SpawnerEvent>,
        ids: &HashMap<u32, u32>,
    ) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        for event in events {
            match event {
                SpawnerEvent::BlockUpdated => self.spawner_block_updated(key),
                SpawnerEvent::BlockEvent(id) => {
                    self.lan_block_event(pos, id, 0);
                    // The client copy's onEventTriggered.
                    if let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get_mut(&key) {
                        s.client_event(id);
                    }
                }
                SpawnerEvent::LevelEvent(id) => self.level_event_at(id, pos, 0),
                SpawnerEvent::EntityPlace(mob, at) => {
                    let who = ids.get(&mob).map(|id| Who::Mob(*id));
                    self.post_block_game_event("minecraft:entity_place", at, who, None);
                }
                SpawnerEvent::SpawnAnim(mob) => {
                    if let Some(id) = ids.get(&mob) {
                        self.mob_poof(*id);
                    }
                }
            }
        }
        Ok(())
    }
    /// `sendBlockUpdated`: the clients load the new update tag (the client
    /// copy's delay follows it, its display mob is made again).
    fn spawner_block_updated(&mut self, key: Pos) {
        if let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get_mut(&key) {
            s.client_loaded();
        }
        self.spawners.displays.remove(&key);
        if let Some(log) = self.world.block_log.as_mut() {
            log.push(key);
        }
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
    }
    /// `LivingEntity.makePoofParticles` (entity event 20) for a mob.
    pub(super) fn mob_poof(&mut self, id: u32) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = m.position;
        self.lan_entity_event(id as i32, 20, [at.x, at.y, at.z]);
        self.poof_particles(id);
    }
    /// `Mob.handleEntityEvent` 20 (`spawnAnim` on a client): the poof about
    /// mob `id`.
    pub(super) fn poof_particles(&mut self, id: u32) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let (at, w, h) = (m.position, f64::from(m.width()), f64::from(m.height()));
        let options = ParticleOptions::simple(ParticleType::Poof);
        for _ in 0..20 {
            let r = self.particles.level_random();
            let xa = r.next_gaussian() * 0.02;
            let ya = r.next_gaussian() * 0.02;
            let za = r.next_gaussian() * 0.02;
            let x = at.x + w * (2.0 * r.next_double() - 1.0) - xa * 10.0;
            let y = at.y + h * r.next_double() - ya * 10.0;
            let z = at.z + w * (2.0 * r.next_double() - 1.0) - za * 10.0;
            self.particles.add(&options, [x, y, z], [xa, ya, za]);
        }
    }
    /// `LevelEventHandler` 2004: twenty smoke and flame pairs around the spawner.
    pub(super) fn spawner_burst(&mut self, pos: [i32; 3]) {
        let (smoke, flame) = (
            ParticleOptions::simple(ParticleType::Smoke),
            ParticleOptions::simple(ParticleType::Flame),
        );
        for _ in 0..20 {
            let r = self.particles.level_random();
            let x = f64::from(pos[0]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            let y = f64::from(pos[1]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            let z = f64::from(pos[2]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            self.particles.add(&smoke, [x, y, z], [0.0; 3]);
            self.particles.add(&flame, [x, y, z], [0.0; 3]);
        }
    }
    /// The client copy's tick: particles and the spin while a player is near.
    fn client_tick_spawner(&mut self, key: Pos) {
        let near = {
            let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get(&key) else {
                return;
            };
            let range = f64::from(s.required_player_range);
            let center = [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ];
            self.player.as_ref().is_some_and(|p| {
                let d = (p.position.x - center[0]).powi(2)
                    + (p.position.y - center[1]).powi(2)
                    + (p.position.z - center[2]).powi(2);
                !p.health.dead_or_dying() && !self.spectator() && (range < 0.0 || d < range * range)
            })
        };
        let display = self.update_spawner_display(key);
        let pos = [key.0, key.1, key.2];
        let random = self.particles.level_random();
        let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get_mut(&key) else {
            return;
        };
        if let Some(p) = s.client_tick(near, display, random, pos) {
            self.particles
                .add(&ParticleOptions::simple(ParticleType::Smoke), p, [0.0; 3]);
            self.particles
                .add(&ParticleOptions::simple(ParticleType::Flame), p, [0.0; 3]);
        }
    }
    /// `getOrCreateDisplayEntity`: the mob shown in the spawner, made again
    /// when its type changes. False when there is none.
    pub(super) fn update_spawner_display(&mut self, key: Pos) -> bool {
        let Some(kind) = self.spawner_display_kind(key) else {
            self.spawners.displays.remove(&key);
            return false;
        };
        if self
            .spawners
            .displays
            .get(&key)
            .is_none_or(|(k, _)| *k != kind)
        {
            let Some(t) = self.res.mob_types().get(&kind).cloned() else {
                return false;
            };
            let mob = Mob::new(u32::MAX, t, Vec3::ZERO, 0.0);
            self.spawners.displays.insert(key, (kind, mob));
        }
        true
    }
    /// The type of the mob a spawner shows: its next data's id, when the game
    /// knows the mob.
    /// A trial spawner shows one only in the states with a spinning mob.
    fn spawner_display_kind(&self, key: Pos) -> Option<String> {
        let id = match self.world.block_entities.get(&key)? {
            BlockEntity::Spawner(s) => s.next_data()?.id()?,
            BlockEntity::TrialSpawner(s) => {
                self.trial_spawner_block(key)?.0.spinning_mob_speed()?;
                s.next_spawn_data.as_ref()?.id()?
            }
            _ => return None,
        };
        self.res.mob_types().contains_key(&id).then_some(id)
    }
    /// `SpawnEggItem.useOn` on a spawner: the egg sets its mob, and an op's
    /// egg its entity `data` too (`block_entity_item::spawn_egg_on_spawner`).
    /// None when the clicked block has no spawner.
    pub(super) fn use_spawn_egg_on_spawner(
        &mut self,
        pos: [i32; 3],
        kind: &str,
        data: Option<&horizon_world::nbt::Compound>,
    ) -> Option<bool> {
        let key = (pos[0], pos[1], pos[2]);
        let trial = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Spawner(_)) => false,
            Some(BlockEntity::TrialSpawner(_)) => true,
            _ => return None,
        };
        if !self.ui.rules.bool("spawner_blocks_work") {
            self.system_message(horizon_client::text::Text::translate(
                "advMode.notEnabled.spawner",
                &[],
            ));
            return Some(false);
        }
        if trial {
            // TrialSpawnerBlockEntity.setEntityId: overrideEntityToSpawn, which
            // ends in setState(INACTIVE).
            if let Some(BlockEntity::TrialSpawner(t)) = self.world.block_entities.get_mut(&key) {
                t.override_entity(kind);
            }
            let ominous = self.trial_spawner_block(key).is_some_and(|(_, o)| o);
            let _ = self.set_trial_spawner_state(key, TrialState::Inactive, ominous);
        } else {
            let mut events = Vec::new();
            let mut random = std::mem::replace(
                &mut self.world.random,
                horizon_core::random::LegacyRandom::new(0),
            );
            if let Some(BlockEntity::Spawner(s)) = self.world.block_entities.get_mut(&key) {
                match data {
                    Some(data) => s.set_entity_data(kind, data, &mut random, &mut events),
                    None => s.set_entity_id(kind, &mut random, &mut events),
                }
            }
            self.world.random = random;
            let _ = self.apply_spawner_events(key, events, &HashMap::new());
        }
        // sendBlockUpdated(3) and the player's block_change game event.
        self.spawner_block_updated(key);
        self.update_spawner_display(key);
        self.post_block_game_event("minecraft:block_change", pos, Some(Who::Player), None);
        Some(true)
    }
    /// `PlayerList.isOp` for the player acting now: a server's guest by their
    /// own level, else the window's player.
    pub(super) fn acting_player_is_op(&self) -> bool {
        self.lan_acting_permission
            .unwrap_or(self.ui.registries.permission)
            > 0
    }
    /// Ticks spawners and trial spawners near the player on a remote client (`tick_remote`).
    pub(super) fn tick_remote_spawners(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let center = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        let mut keys = Vec::new();
        let mut trial_keys = Vec::new();
        for (&pos, be) in &self.world.block_entities {
            let d = (pos.0 - center[0])
                .abs()
                .max((pos.1 - center[1]).abs())
                .max((pos.2 - center[2]).abs());
            if d <= 64 {
                match be {
                    BlockEntity::Spawner(_) => keys.push(pos),
                    BlockEntity::TrialSpawner(_) => trial_keys.push(pos),
                    _ => {}
                }
            }
        }
        self.client_tick_spawners(&keys);
        for key in trial_keys {
            self.client_tick_trial_spawner(key);
        }
    }
    /// `SpawnerRenderer` and `TrialSpawnerRenderer`: the next mob of every
    /// spawner near the camera, small and turning.
    pub(super) fn spawner_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        for (key, (_, mob)) in &self.spawners.displays {
            let at = glam::DVec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
            if at.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let (spin, scale) = match self.world.block_entities.get(key) {
                Some(BlockEntity::Spawner(s)) => {
                    s.display_pose(alpha as f32, mob.width(), mob.height())
                }
                Some(BlockEntity::TrialSpawner(s)) => horizon_client::mobs::spawner::display_pose(
                    s.o_spin,
                    s.spin,
                    alpha as f32,
                    mob.width(),
                    mob.height(),
                ),
                _ => continue,
            };
            let origin = glam::Mat4::from_translation(
                (at - camera).as_vec3() + glam::Vec3::new(0.5, 0.4, 0.5),
            ) * glam::Mat4::from_rotation_y(spin.to_radians())
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.2, 0.0))
                * glam::Mat4::from_rotation_x((-30.0_f32).to_radians())
                * glam::Mat4::from_scale(glam::Vec3::splat(scale));
            let light = self.world.light(key.0, key.1, key.2);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            if mob.species() == horizon_client::mobs::Species::SulfurCube {
                self.sulfur_cube_body_geometry(
                    mob,
                    alpha as f32,
                    horizon_client::mobs::render::base_transform_at(mob, alpha as f32, origin),
                    packed,
                    out,
                );
            }
            horizon_client::mobs::render::render_posed(
                mob,
                alpha as f32,
                origin,
                &lookup,
                packed,
                self.res.mob_data(),
                out,
            );
        }
    }
}
