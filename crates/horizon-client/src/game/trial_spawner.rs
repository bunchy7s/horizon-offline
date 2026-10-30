//! Trial spawners in the game: `TrialSpawner.tickServer` for every trial
//! spawner (`mobs::trial_spawner`), the block's state as it changes, the mobs
//! it adds and the rewards it ejects.
use super::{Game, natural_spawning::GameLevel, spawner::Planned};
use horizon_client::{
    Result,
    containers::BlockEntity,
    items::ItemStack,
    mobs::{
        Who,
        spawner::SpawnerSpawn,
        trial_spawner::{
            TRIAL_OMEN_TICKS_PER_LEVEL, TrialEvent, TrialPlayer, TrialSpawner, TrialSpawnerExt,
            TrialSpawnerLevel, TrialState, Uuid,
        },
    },
};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::nbt::Tag;
use std::collections::HashMap;

type Pos = (i32, i32, i32);

impl TrialSpawnerLevel for GameLevel<'_, '_> {
    fn game_time(&self) -> i64 {
        self.game.world.game_time
    }
    fn mob_spawning(&self) -> bool {
        self.game.ui.rules.bool("spawn_mobs")
    }
    fn players(&self) -> Vec<TrialPlayer> {
        let guests = self.game.lan_present().into_iter().map(|g| TrialPlayer {
            uuid: g.uuid,
            block: g.position.map(|v| v.floor() as i32),
            eye: g.eye,
            creative: g.creative,
            spectator: g.spectator,
            position: g.position,
            height: g.height,
            alive: g.alive,
            omen: g.omen,
        });
        let (Some(p), Some(view)) = (self.game.player.as_ref(), self.game.player_view()) else {
            return guests.collect();
        };
        let host = TrialPlayer {
            uuid: self.game.acting_uuid(),
            block: [p.position.x, p.position.y, p.position.z].map(|v| v.floor() as i32),
            eye: view.eye,
            creative: self.game.creative,
            spectator: self.game.spectator(),
            position: [p.position.x, p.position.y, p.position.z],
            height: p.dimensions.height,
            alive: self.game.level.health > 0.0,
            omen: super::lan_mobs::omen_of(&self.game.potions.effects),
        };
        std::iter::once(host).chain(guests).collect()
    }
    fn visual_hit(&self, from: [f64; 3], to: [f64; 3]) -> Option<[i32; 3]> {
        let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
        self.w.visual_hit(v(from), v(to))
    }
    fn entity_block(&self, uuid: Uuid) -> Option<[i32; 3]> {
        let (id, _) = self.game.persist.uuids.iter().find(|(_, u)| **u == uuid)?;
        let m = self.game.mobs.iter().find(|m| m.id == *id)?;
        (!m.removed && m.alive())
            .then(|| [m.position.x, m.position.y, m.position.z].map(|v| v.floor() as i32))
    }
    fn entity_box(&self, uuid: Uuid) -> Option<([f64; 3], f32)> {
        let (id, _) = self.game.persist.uuids.iter().find(|(_, u)| **u == uuid)?;
        let m = self.game.mobs.iter().find(|m| m.id == *id)?;
        let b = m.bounds();
        (!m.removed && m.alive()).then(|| {
            (
                [m.position.x, m.position.y, m.position.z],
                (b.max_position().y - b.min_position().y) as f32,
            )
        })
    }
    fn no_block_collision(&self, block: [i32; 3]) -> bool {
        self.w
            .block_collision_boxes(block[0], block[1], block[2])
            .is_empty()
    }
    fn loot_items(&self, table: &str, seed: i64) -> Vec<ItemStack> {
        let Some(loot) = horizon_client::structure_blocks::loot_data(
            &self.game.options.resources,
            &self.game.pack_files(),
        ) else {
            return Vec::new();
        };
        loot.roll_split(table, &mut LegacyRandom::new(seed))
            .iter()
            .map(horizon_client::structure_blocks::loot_item_with_components)
            .collect()
    }
    fn seed(&self) -> i64 {
        self.game.level.seed
    }
}

impl Game<'_> {
    /// The server tick of the trial spawner at `key`. Its client tick comes
    /// after every block entity's.
    pub(super) fn tick_trial_spawner(&mut self, key: Pos) -> Result<()> {
        let Some((current, ominous)) = self.trial_spawner_block(key) else {
            return Ok(());
        };
        let Some(BlockEntity::TrialSpawner(mut spawner)) = self.world.block_entities.take(&key)
        else {
            return Ok(());
        };
        let result = self.server_tick_trial_spawner(key, &mut spawner, current, ominous);
        self.world
            .block_entities
            .put(key, BlockEntity::TrialSpawner(spawner));
        result
    }
    /// The trial spawner block at `key`: its state and ominous flag.
    pub(super) fn trial_spawner_block(&self, key: Pos) -> Option<(TrialState, bool)> {
        let block = self
            .pack
            .assets
            .registry
            .state(self.world.block(key.0, key.1, key.2))?;
        if block.name != "minecraft:trial_spawner" {
            return None;
        }
        let state = block
            .properties
            .get("trial_spawner_state")
            .and_then(|s| TrialState::from_name(s))
            .unwrap_or_default();
        Some((
            state,
            block.properties.get("ominous").is_some_and(|v| v == "true"),
        ))
    }
    fn server_tick_trial_spawner(
        &mut self,
        key: Pos,
        spawner: &mut TrialSpawner,
        current: TrialState,
        ominous: bool,
    ) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let mut random = std::mem::replace(&mut self.world.random, LegacyRandom::new(0));
        let mut sequences = std::mem::take(&mut self.rand_sequences);
        let mut events = Vec::new();
        let mut planned: Vec<Planned> = Vec::new();
        let mut uuids: Vec<(u32, Uuid)> = Vec::new();
        let mut ejected: Vec<(ItemStack, [f64; 3])> = Vec::new();
        let next = {
            let level = self.natural_level();
            let loot = horizon_client::structure_blocks::loot_data(
                &self.options.resources,
                &self.pack_files(),
            );
            let seed = self.level.seed;
            let mut spawn = |l: &GameLevel, r: &SpawnerSpawn| {
                let added = l.game.plan_spawner_mob(l, r, &mut planned)?;
                let uuid = match planned.last_mut() {
                    Some(Planned::Loaded(Tag::Compound(tag), _)) => {
                        let tag_uuid =
                            horizon_client::entity_nbt::uuid(&Tag::Compound(tag.clone()));
                        tag_uuid.unwrap_or_else(|| {
                            let uuid = horizon_client::save::random_uuid();
                            tag.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
                            uuid
                        })
                    }
                    _ => horizon_client::save::random_uuid(),
                };
                uuids.push((added.id, uuid));
                Some(uuid)
            };
            // TrialSpawner.ejectReward: the table's items, each thrown up out
            // of the spawner (DefaultDispenseItemBehavior.spawnItem, accuracy 2).
            let mut eject = |table: &str, random: &mut LegacyRandom| {
                let Some(loot) = &loot else {
                    return false;
                };
                let drops = match loot.random_sequence(table) {
                    Some(s) => loot.roll_split(table, sequences.get(s, seed)),
                    None => loot.roll_split(table, random),
                };
                for stack in &drops {
                    // The power, which only scales the direction's x and z (0 going up).
                    random.next_double();
                    let spread = 0.017_227_5 * 2.0;
                    let v = [
                        random.triangle(0.0, spread),
                        random.triangle(0.2, spread),
                        random.triangle(0.0, spread),
                    ];
                    ejected.push((
                        horizon_client::structure_blocks::loot_item_with_components(stack),
                        v,
                    ));
                }
                !drops.is_empty()
            };
            spawner.server_tick(
                &level,
                &mut random,
                pos,
                current,
                ominous,
                &mut spawn,
                &mut eject,
                &mut events,
            )
        };
        self.world.random = random;
        self.rand_sequences = sequences;
        let ids = self.add_planned(planned);
        let mut by_uuid = HashMap::new();
        for (provisional, uuid) in uuids {
            if let Some(id) = ids.get(&provisional) {
                self.persist.uuids.insert(*id, uuid);
                by_uuid.insert(uuid, *id);
            }
        }
        let center = pos.map(|v| f64::from(v) + 0.5);
        // Vec3.atBottomCenterOf(pos).relative(UP, 1.2), lowered by spawnItem.
        let above = [center[0], center[1] + 0.7 - 0.125, center[2]];
        for (stack, v) in ejected {
            self.spawn_item(stack, above, v, 0);
        }
        if next != current || ominous != spawner.is_ominous {
            self.set_trial_spawner_state(key, next, spawner.is_ominous)?;
        }
        for event in events {
            match event {
                TrialEvent::BlockUpdated => self.mark_block_entity_updated(key),
                TrialEvent::LevelEvent(id, at, data) => self.level_event_at(id, at, data),
                TrialEvent::Sound(name) => {
                    self.audio
                        .play(&format!("minecraft:{name}"), Some(center), 1.0, 1.0)
                }
                // Written with the state above.
                TrialEvent::OminousSet(_) => {}
                TrialEvent::EntityPlace(uuid, at) => {
                    let who = by_uuid.get(&uuid).map(|id| Who::Mob(*id));
                    self.post_block_game_event("minecraft:entity_place", at, who, None);
                }
                TrialEvent::TrialOmen(uuid) => self.bad_omen_to_trial_omen(uuid),
                TrialEvent::Discard(uuid) => {
                    let id = self.persist.uuids.iter().find(|(_, u)| **u == uuid);
                    if let Some((id, _)) = id
                        && let Some(m) = self.mobs.iter_mut().find(|m| m.id == *id)
                    {
                        m.removed = true;
                    }
                }
                TrialEvent::SoundAt(name, at, pitch) => self.audio.play(
                    &format!("minecraft:{name}"),
                    Some(at.map(|v| f64::from(v) + 0.5)),
                    1.0,
                    pitch,
                ),
                TrialEvent::ItemSpawner {
                    item,
                    position,
                    delay,
                } => self.add_ominous_item_spawner(item, position, delay),
            }
        }
        Ok(())
    }
    /// `setState` (and `removeOminous`): the block's state and ominous flag.
    pub(super) fn set_trial_spawner_state(
        &mut self,
        key: Pos,
        state: TrialState,
        ominous: bool,
    ) -> Result<()> {
        let current = self.world.block(key.0, key.1, key.2);
        let Some(mut block) = self.pack.assets.registry.state(current).cloned() else {
            return Ok(());
        };
        block
            .properties
            .insert("trial_spawner_state".into(), state.name().into());
        block
            .properties
            .insert("ominous".into(), ominous.to_string());
        let id = self.pack.assets.registry.resolve(&block)?;
        self.level_set([key.0, key.1, key.2], id)
    }
    /// `transformBadOmenIntoTrialOmen`: Bad Omen becomes Trial Omen, 15
    /// minutes per level, with Trial Omen's sound when it is new. `uuid` is
    /// the player found: the one in the game's frame, else a guest.
    fn bad_omen_to_trial_omen(&mut self, uuid: Uuid) {
        let eye = if self.acting_uuid() == uuid {
            transform_bad_omen(&mut self.potions.effects)
                .then(|| self.eye_position().unwrap_or_default())
        } else {
            let acting = self.lan_acting;
            let Some(guest) = self.lan.as_mut().and_then(|h| {
                h.hub.guests.iter_mut().find(|g| {
                    horizon_client::lan::playerdata::uuid_ints(g.uuid) == uuid
                        && Some(g.conn) != acting
                })
            }) else {
                return;
            };
            let [x, y, z] = guest.position;
            let eye = [x, y + guest.eye_height(), z];
            transform_bad_omen(&mut guest.effects).then_some(eye)
        };
        if let Some(eye) = eye {
            self.audio
                .play("minecraft:event.mob_effect.trial_omen", Some(eye), 1.0, 1.0);
        }
    }
    /// `sendBlockUpdated` of a block entity: the clients load its new update tag.
    pub(super) fn mark_block_entity_updated(&mut self, key: Pos) {
        if let Some(log) = self.world.block_log.as_mut() {
            log.push(key);
        }
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
    }
}

/// Swap Bad Omen for Trial Omen on `effects`; true when Trial Omen is new.
fn transform_bad_omen(effects: &mut horizon_client::effects::PlayerEffects) -> bool {
    let Some(amplifier) = effects.amplifier("bad_omen") else {
        return false;
    };
    effects.list.retain(|e| e.id != "bad_omen");
    let new = effects.get("trial_omen").is_none();
    effects.add(horizon_client::effects::EffectInstance::new(
        "trial_omen",
        TRIAL_OMEN_TICKS_PER_LEVEL * (amplifier + 1),
        0,
    ));
    new
}

#[cfg(test)]
mod omen_tests {
    use super::*;
    use horizon_client::{
        effects::{EffectInstance, PlayerEffects},
        mobs::trial_spawner::Omen,
    };

    #[test]
    fn bad_omen_becomes_trial_omen_per_level() {
        let omen_of = super::super::lan_mobs::omen_of;
        let mut e = PlayerEffects::default();
        assert_eq!(omen_of(&e), None);
        e.add(EffectInstance::new("bad_omen", 100, 1));
        assert_eq!(omen_of(&e), Some(Omen::Bad));
        assert!(transform_bad_omen(&mut e));
        assert!(e.get("bad_omen").is_none());
        assert_eq!(
            e.get("trial_omen").unwrap().duration,
            TRIAL_OMEN_TICKS_PER_LEVEL * 2
        );
        assert_eq!(omen_of(&e), Some(Omen::Trial));
        assert!(!transform_bad_omen(&mut e));
    }
}
