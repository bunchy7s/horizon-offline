//! Potent sulfur geysers: the tickers `PotentSulfurBlock.getTicker` gives
//! `PotentSulfurBlockEntity` by the block's state. The server's make living
//! entities near the gas nauseous, count a dormant or erupting geyser down to
//! its next change of state and push the entities an eruption throws up. The
//! client's draw the gas clouds and the eruption's plume, play its sound and
//! push the local player, who moves on its own client and not on the server.
use super::{
    Game,
    potions::{mob_can_be_affected, mob_effect_id},
};
use horizon_client::{
    Result,
    block_entities::Pos,
    containers::BlockEntity,
    effects::EffectInstance,
    mobs::Mob,
    particles::{ParticleLevel, ParticleOptions, ParticleType as T},
    potent_sulfur::{
        can_be_reached_by_noxious_gas, find_noxious_gas_source_block, unobstructed_block_count,
    },
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::{PositionalRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{level::flags, registry::BlockStateId};

/// `PotentSulfurBlockEntity.GEYSER_SALT`.
const GEYSER_SALT: i64 = -904_011_478;
/// `PotentSulfurBlockEntity.EFFECT_APPLICATION_FREQUENCY_TICKS`.
const EFFECT_APPLICATION_FREQUENCY_TICKS: i64 = 10;
/// `PotentSulfurBlockEntity.EFFECT_DURATION_IN_TICKS`.
const EFFECT_DURATION_IN_TICKS: i32 = 80;
/// `PotentSulfurBlockEntity.PARTICLE_FREQUENCY_TICKS`.
const PARTICLE_FREQUENCY_TICKS: i64 = 20;
/// `PotentSulfurBlockEntity.SOUND_FREQUENCY_TICKS`.
const SOUND_FREQUENCY_TICKS: i64 = 40;
/// `SoundEvents.GEYSER_ERUPTION_ACTIVE`.
const GEYSER_ERUPTION_ACTIVE: &str = "minecraft:block.potent_sulfur.geyser_eruption_active";
/// `SoundEvents.GEYSER_CONTINUOUS_ACTIVE`.
const GEYSER_CONTINUOUS_ACTIVE: &str =
    "minecraft:block.potent_sulfur.geyser_continuous_eruption_active";

/// `PotentSulfurState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Geyser {
    Dry,
    Wet,
    Dormant,
    Erupting,
    Continuous,
}

/// `PotentSulfurBlockEntity.geyserPositional`: the random of the sulfur block
/// at `pos`, forked from the level seed and the geyser salt.
fn geyser_positional(seed: i64, pos: Pos) -> XoroshiroRandom {
    let mut base = XoroshiroRandom::new(seed ^ GEYSER_SALT);
    base.fork_positional()
        .at(BlockPos::new(pos.0, pos.1, pos.2))
}

/// `PotentSulfurBlockEntity.getNearbyLivingEntities`: the box of the source
/// block grown 2.5 blocks sideways.
fn nearby_living_area(source: [i32; 3]) -> Aabb {
    let [x, y, z] = source.map(f64::from);
    Aabb::new(
        Vec3::new(x - 2.5, y, z - 2.5),
        Vec3::new(x + 1.0 + 2.5, y + 1.0, z + 1.0 + 2.5),
    )
}

/// The box `PotentSulfurBlockEntity.LAUNCH_ENTITY_TICKER` pushes entities in:
/// the block above the sulfur, `expandTowards` the unobstructed height.
fn launch_area(pos: Pos, height: i32) -> Aabb {
    let (x, y, z) = (f64::from(pos.0), f64::from(pos.1 + 1), f64::from(pos.2));
    let up = f64::from(height - 1);
    let (min_y, max_y) = if up < 0.0 {
        (y + up, y + 1.0)
    } else {
        (y, y + 1.0 + up)
    };
    Aabb::new(Vec3::new(x, min_y, z), Vec3::new(x + 1.0, max_y, z + 1.0))
}

/// `PotentSulfurBlockEntity.applyNauseaEffect` on a mob, if it can be
/// affected.
fn apply_nausea_effect_to_mob(
    mob: &mut Mob,
    tags: &std::collections::HashMap<String, std::collections::HashSet<String>>,
) {
    if mob_can_be_affected(tags, &mob.kind.id, "nausea")
        && let Some(nausea) = mob_effect_id("nausea")
    {
        mob.add_effect(nausea, EFFECT_DURATION_IN_TICKS, 0);
    }
}

impl Game<'_> {
    /// The `potent_sulfur_state` of the block at `pos`, with its block state.
    fn potent_sulfur_state(&self, pos: Pos) -> Option<(BlockStateId, Geyser)> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let value = self
            .pack
            .assets
            .registry
            .state(state)?
            .properties
            .get("potent_sulfur_state")?;
        let geyser = match value.as_str() {
            "dry" => Geyser::Dry,
            "wet" => Geyser::Wet,
            "dormant" => Geyser::Dormant,
            "erupting" => Geyser::Erupting,
            "continuous" => Geyser::Continuous,
            _ => return None,
        };
        Some((state, geyser))
    }

    /// `PotentSulfurBlockEntity.findNoxiousGasSourceBlock` of the sulfur at
    /// `pos`.
    fn potent_sulfur_source(&self, pos: Pos) -> Option<[i32; 3]> {
        let level = ParticleLevel {
            world: &self.world,
            facts: self.res.facts(),
            collision: self.collision,
            player: None,
        };
        find_noxious_gas_source_block(&level, [pos.0, pos.1, pos.2])
    }

    /// `PotentSulfurBlock.getTicker` on the server, for the block entity at
    /// `pos`: a wet block gives nausea; a dormant one counts down, then gives
    /// nausea; an erupting one pushes entities, then counts down; a continuous
    /// one pushes entities. A dry block has no ticker.
    pub(super) fn tick_potent_sulfur(&mut self, pos: Pos) -> Result<()> {
        let Some((state, geyser)) = self.potent_sulfur_state(pos) else {
            return Ok(());
        };
        match geyser {
            Geyser::Dry => {}
            Geyser::Wet => self.server_nausea_effect_ticker(pos),
            Geyser::Dormant => {
                self.server_waiting_countdown_ticker(pos, state, geyser)?;
                self.server_nausea_effect_ticker(pos);
            }
            Geyser::Erupting => {
                self.launch_entity_ticker(pos);
                self.server_waiting_countdown_ticker(pos, state, geyser)?;
            }
            Geyser::Continuous => self.launch_entity_ticker(pos),
        }
        Ok(())
    }

    /// `PotentSulfurBlockEntity.SERVER_NAUSEA_EFFECT_TICKER`: every 10 ticks,
    /// the living entities near the source block that its gas reaches (at
    /// their eyes) get nausea for 4 seconds.
    fn server_nausea_effect_ticker(&mut self, pos: Pos) {
        if self.world.game_time % EFFECT_APPLICATION_FREQUENCY_TICKS != 0 {
            return;
        }
        let Some(source) = self.potent_sulfur_source(pos) else {
            return;
        };
        let area = nearby_living_area(source);
        for player in self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive && p.bounds.intersects(area))
        {
            self.as_fight_player(player.who, |g| g.apply_nausea_effect_to_player(source));
        }
        let level = ParticleLevel {
            world: &self.world,
            facts: self.res.facts(),
            collision: self.collision,
            player: None,
        };
        let tags = &self.res.stations().enchantments.entity_tags;
        for mob in self
            .mobs
            .iter_mut()
            .filter(|m| m.alive() && m.bounds().intersects(area))
        {
            let eye = [mob.position.x, mob.eye_y(), mob.position.z];
            if can_be_reached_by_noxious_gas(&level, source, eye) {
                apply_nausea_effect_to_mob(mob, tags);
            }
        }
    }

    /// `PotentSulfurBlockEntity.applyNauseaEffect` on the player acting (a
    /// guest is swapped in as the player) when the gas reaches their eyes;
    /// spectators are not affected.
    fn apply_nausea_effect_to_player(&mut self, source: [i32; 3]) {
        if self.spectator() {
            return;
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let eye = [
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ];
        let reached = {
            let level = ParticleLevel {
                world: &self.world,
                facts: self.res.facts(),
                collision: self.collision,
                player: None,
            };
            can_be_reached_by_noxious_gas(&level, source, eye)
        };
        if reached {
            // MobEffectInstance(NAUSEA, 80, 0, true, true).
            let mut nausea = EffectInstance::new("nausea", EFFECT_DURATION_IN_TICKS, 0);
            nausea.ambient = true;
            self.add_player_effect(nausea);
        }
    }

    /// `PotentSulfurBlockEntity.SERVER_WAITING_COUNTDOWN_TICKER`: every second
    /// the countdown runs down; a dormant geyser waits `10 * (water - 1)` plus
    /// 15 to 30, an erupting one `water - 1` plus 1 or 2, and at zero the
    /// block turns erupting (dormant) or dormant (erupting). `state` is the
    /// block state the tick began with.
    fn server_waiting_countdown_ticker(
        &mut self,
        pos: Pos,
        state: BlockStateId,
        geyser: Geyser,
    ) -> Result<()> {
        if self.world.game_time % 20 != 0 {
            return Ok(());
        }
        let Some(source) = self.potent_sulfur_source(pos) else {
            return Ok(());
        };
        let Some(BlockEntity::PotentSulfur(sulfur)) = self.world.block_entities.get_mut(&pos)
        else {
            return Ok(());
        };
        if sulfur.waiting_countdown <= 0 {
            let water_blocks = source[1] - pos.1 - 1;
            let mut positional = geyser_positional(self.level.seed, pos);
            sulfur.waiting_countdown = if geyser == Geyser::Dormant {
                10 * (water_blocks - 1)
                    + positional.next_int_between_inclusive(15, 30).unwrap_or(15)
            } else {
                positional.next_int();
                water_blocks - 1 + positional.next_int_between_inclusive(1, 2).unwrap_or(1)
            };
        }
        if sulfur.waiting_countdown > 0 {
            sulfur.waiting_countdown -= 1;
        }
        if sulfur.waiting_countdown != 0 {
            return Ok(());
        }
        let to_set = if geyser == Geyser::Dormant {
            "erupting"
        } else {
            "dormant"
        };
        let registry = self.pack.assets.registry;
        let Some(mut next) = registry.state(state).cloned() else {
            return Ok(());
        };
        next.properties
            .insert("potent_sulfur_state".into(), to_set.into());
        let Some(next) = registry.id(&next) else {
            return Ok(());
        };
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, next, flags::UPDATE_ALL));
        self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        if to_set == "dormant" {
            self.post_block_game_event(
                "minecraft:block_deactivate",
                [pos.0, pos.1, pos.2],
                None,
                Some(state),
            );
        }
        self.process_level_events()
    }

    /// `PotentSulfurBlockEntity.LAUNCH_ENTITY_TICKER` on the server: mobs and
    /// items and primed TNT in the geyser's column with less than
    /// `0.3 + 0.1 * water` upward speed get 0.2 more, unless they ride
    /// something or are of a type geysers do not move. Players are moved by
    /// their own client.
    fn launch_entity_ticker(&mut self, pos: Pos) {
        let Some(source) = self.potent_sulfur_source(pos) else {
            return;
        };
        let water_blocks = source[1] - pos.1 - 1;
        let level = ParticleLevel {
            world: &self.world,
            facts: self.res.facts(),
            collision: self.collision,
            player: None,
        };
        let height = unobstructed_block_count(&level, [pos.0, pos.1 + 1, pos.2], water_blocks);
        let area = launch_area(pos, height);
        let limit = f64::from(0.3f32) + f64::from(water_blocks) * 0.1;
        let push = f64::from(0.2f32);
        let tags = &self.res.stations().enchantments.entity_tags;
        let immune = tags.get("minecraft:not_affected_by_geysers");
        for mob in self
            .mobs
            .iter_mut()
            .filter(|m| m.alive() && m.bounds().intersects(area))
        {
            // Entity.checkFallDistanceAccumulation.
            if mob.velocity.y > -0.5 && mob.fall_distance > 1.0 {
                mob.fall_distance = 1.0;
            }
            if !mob.extra.passenger
                && !immune.is_some_and(|t| t.contains(&mob.kind.id))
                && mob.velocity.y < limit
            {
                mob.velocity.y += push;
            }
        }
        for item in self
            .dropped
            .iter_mut()
            .filter(|i| i.bounds().intersects(area))
        {
            if item.velocity.y < limit {
                item.velocity.y += push;
            }
        }
        for tnt in self.tnts.iter_mut().filter(|t| t.bounds().intersects(area)) {
            if tnt.velocity.y < limit {
                tnt.velocity.y += push;
            }
        }
    }

    /// `PotentSulfurBlockEntity.LAUNCH_ENTITY_TICKER` on the client: the local
    /// player in the column, unless spectating, flying or riding.
    fn launch_local_player_ticker(&mut self, pos: Pos) {
        let Some(source) = self.potent_sulfur_source(pos) else {
            return;
        };
        let water_blocks = source[1] - pos.1 - 1;
        let height = {
            let level = ParticleLevel {
                world: &self.world,
                facts: self.res.facts(),
                collision: self.collision,
                player: None,
            };
            unobstructed_block_count(&level, [pos.0, pos.1 + 1, pos.2], water_blocks)
        };
        let area = launch_area(pos, height);
        let (spectator, riding) = (self.spectator(), self.riding.is_some());
        let Some(p) = self.player.as_mut() else {
            return;
        };
        if spectator || p.health.dead_or_dying() || !p.bounds().intersects(area) {
            return;
        }
        // Entity.checkFallDistanceAccumulation.
        if p.velocity.y > -0.5 && p.fall_distance > 1.0 {
            p.fall_distance = 1.0;
        }
        if !p.flying && !riding && p.velocity.y < f64::from(0.3f32) + f64::from(water_blocks) * 0.1
        {
            p.velocity.y += f64::from(0.2f32);
        }
    }

    /// `PotentSulfurBlockEntity.spawnGeyserParticle`: the eruption particle at
    /// the bottom of the source block, as high as the water column is deep.
    fn spawn_geyser_particle(&mut self, sulfur: Pos, source: [i32; 3]) {
        let water_blocks = source[1] - sulfur.1 - 1;
        self.particles.add(
            &ParticleOptions::geyser(T::Geyser, water_blocks),
            [
                f64::from(source[0]) + 0.5,
                f64::from(source[1]),
                f64::from(source[2]) + 0.5,
            ],
            [0.0; 3],
        );
    }

    /// `PotentSulfurBlockEntity.spawnNoxiousGasCloudParticle` at the centre of
    /// the source block.
    fn spawn_noxious_gas_cloud_particle(&mut self, source: [i32; 3]) {
        self.particles.add(
            &ParticleOptions::simple(T::NoxiousGasCloud),
            source.map(|c| f64::from(c) + 0.5),
            [0.0; 3],
        );
    }

    /// `PotentSulfurBlock.getTicker` on the client, for the block entity at
    /// `pos`: a wet or dormant geyser releases gas clouds every second; an
    /// erupting or continuous one shows its plume every second (from its last
    /// eruption event), plays its sound every two, and pushes the local
    /// player.
    pub(super) fn client_tick_potent_sulfur(&mut self, pos: Pos) {
        let Some((_, geyser)) = self.potent_sulfur_state(pos) else {
            return;
        };
        let time = self.world.game_time;
        // PotentSulfurBlockEntity.setLevel.
        let eruption_tick = match self.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::PotentSulfur(sulfur)) => {
                if sulfur.eruption_tick == -1 {
                    sulfur.eruption_tick = time;
                }
                sulfur.eruption_tick
            }
            _ => return,
        };
        match geyser {
            Geyser::Dry => {}
            // CLIENT_NOXIOUS_GAS_TICKER.
            Geyser::Wet | Geyser::Dormant => {
                if time % PARTICLE_FREQUENCY_TICKS == 0
                    && let Some(source) = self.potent_sulfur_source(pos)
                {
                    self.spawn_noxious_gas_cloud_particle(source);
                }
            }
            // CLIENT_GEYSER_PLUME_TICKER, then LAUNCH_ENTITY_TICKER.
            Geyser::Erupting | Geyser::Continuous => {
                if let Some(source) = self.potent_sulfur_source(pos) {
                    let eruption_time = time - eruption_tick;
                    if eruption_time % PARTICLE_FREQUENCY_TICKS == 0 {
                        self.spawn_geyser_particle(pos, source);
                    }
                    if eruption_time % SOUND_FREQUENCY_TICKS == 0 {
                        let sound = if geyser == Geyser::Erupting {
                            GEYSER_ERUPTION_ACTIVE
                        } else {
                            GEYSER_CONTINUOUS_ACTIVE
                        };
                        let at = source.map(|c| f64::from(c) + 0.5);
                        // playLocalSound: guests play their own.
                        self.audio.play_unshared(sound, Some(at), 1.0, 1.0);
                    }
                }
                self.launch_local_player_ticker(pos);
            }
        }
    }

    /// `ClientLevel.tickBlockEntities` for the potent sulfur of a server's
    /// level: every one the client knows ticks.
    pub(super) fn tick_remote_potent_sulfur(&mut self) {
        let keys: Vec<Pos> = self
            .world
            .block_entities
            .iter()
            .filter_map(|(k, e)| matches!(e, BlockEntity::PotentSulfur(_)).then_some(*k))
            .collect();
        for key in keys {
            self.client_tick_potent_sulfur(key);
        }
    }

    /// `PotentSulfurBlock.triggerEvent`: the eruption block event stamps the
    /// game time the plume counts from; false when `pos` holds no potent
    /// sulfur block entity.
    pub(super) fn potent_sulfur_eruption_event(&mut self, pos: Pos) -> bool {
        let time = self.world.game_time;
        match self.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::PotentSulfur(sulfur)) => {
                sulfur.eruption_tick = time;
                true
            }
            _ => false,
        }
    }
}
