//! Hunger and regeneration over the resident player's actual health, damage,
//! attribute and exhaustion storage. Starvation borrows the same FoodData that
//! is mid-tick, preserving damage callbacks and the following timer reset.
use crate::{
    Result,
    health::{Attributes, Environment, HealthEvent, HealthResources, HealthRules, State, Updates},
    motion::{MotionEvent, PlayerMotion},
};
use horizon_world::{
    food::{Difficulty, FoodData, FoodWorld},
    player_health::{PlayerAttributeHealth, PlayerFloatIds},
    server_player_combat::ServerPlayerCombat,
    synced_floats::SyncedFloats,
};

impl PlayerMotion {
    /// Player.causeFoodExhaustion: creative and spectator abilities suppress
    /// exhaustion from movement, attacks and effects.
    pub fn cause_food_exhaustion(&mut self, amount: f32) {
        if !self.invulnerable && !self.spectator {
            self.health.food.add_exhaustion(amount);
        }
    }

    /// ServerPlayer.tickRegeneration, dispatched from Player.aiStep before
    /// inventory and living movement. The separately invoked food tick follows
    /// movement in ServerPlayer.doTick.
    pub fn peaceful_regeneration(&mut self, rules: HealthRules) -> Result<()> {
        if rules.difficulty != Difficulty::Peaceful || !rules.natural_regeneration {
            return Ok(());
        }
        if self.health.tick_count % 20 == 0 {
            let health = self.health.health();
            let maximum = self.attribute("minecraft:max_health")? as f32;
            if health < maximum {
                let h = &mut self.health;
                let mut observer = Updates(&mut self.events);
                PlayerAttributeHealth::new(
                    &mut self.attributes,
                    &mut h.data,
                    h.ids,
                    h.dead,
                    &mut observer,
                )?
                .heal(1.0)?;
            }
            let saturation = self.health.food.saturation;
            if saturation < 20.0 {
                self.health.food.saturation = saturation + 1.0;
            }
        }
        if self.health.tick_count % 10 == 0 && self.health.food.needs_food() {
            self.health.food.food = self.health.food.food.wrapping_add(1);
        }
        Ok(())
    }
    pub fn food_tick(&mut self, resources: &HealthResources, rules: HealthRules) -> Result<()> {
        if self.client_side {
            return Ok(());
        }
        let h = &mut self.health;
        let mut world = TickWorld {
            entity: self.entity_id,
            resources,
            rules,
            invulnerable: self.invulnerable || self.spectator,
            client_loaded: self.client_loaded,
            attributes: &mut self.attributes,
            data: &mut h.data,
            ids: h.ids,
            hurt: &mut h.hurt,
            hurt_animation_time: &mut h.hurt_animation_time,
            dead: &mut h.dead,
            sync_velocity: &mut h.sync_velocity,
            game_time: h.game_time,
            events: &mut self.events,
        };
        h.food.tick(&mut world)
    }
}
struct TickWorld<'a> {
    entity: i32,
    resources: &'a HealthResources,
    rules: HealthRules,
    invulnerable: bool,
    client_loaded: bool,
    attributes: &'a mut Attributes,
    data: &'a mut SyncedFloats,
    ids: PlayerFloatIds,
    hurt: &'a mut State,
    hurt_animation_time: &'a mut i32,
    dead: &'a mut bool,
    sync_velocity: &'a mut bool,
    game_time: i64,
    events: &'a mut Vec<MotionEvent>,
}
impl FoodWorld for TickWorld<'_> {
    type Error = Box<dyn std::error::Error>;
    type Level = ();
    fn level(&mut self, _: &mut FoodData) -> Result<()> {
        Ok(())
    }
    fn difficulty(&mut self, _: (), _: &mut FoodData) -> Result<Difficulty> {
        Ok(self.rules.difficulty)
    }
    fn natural_regeneration(&mut self, _: (), _: &mut FoodData) -> Result<bool> {
        Ok(self.rules.natural_regeneration)
    }
    fn is_hurt(&mut self, _: &mut FoodData) -> Result<bool> {
        let mut observer = Updates(self.events);
        Ok(PlayerAttributeHealth::new(
            self.attributes,
            self.data,
            self.ids,
            *self.dead,
            &mut observer,
        )?
        .is_hurt()?)
    }
    fn heal(&mut self, _: &mut FoodData, amount: f32) -> Result<()> {
        let mut observer = Updates(self.events);
        Ok(PlayerAttributeHealth::new(
            self.attributes,
            self.data,
            self.ids,
            *self.dead,
            &mut observer,
        )?
        .heal(amount)?)
    }
    fn health(&mut self, _: &mut FoodData) -> Result<f32> {
        Ok(self
            .data
            .get(self.ids.health)
            .expect("validated health field"))
    }
    fn hurt_starvation(&mut self, _: (), food: &mut FoodData, amount: f32) -> Result<()> {
        // FoodData still ticks after a mode switch with an empty food bar, but
        // Player.hurtServer rejects starvation for invulnerable abilities.
        if self.invulnerable {
            return Ok(());
        }
        let id = self
            .resources
            .damage
            .id("minecraft:starve")
            .expect("validated starvation source");
        self.events
            .push(MotionEvent::Health(HealthEvent::StarvationDamage(amount)));
        let mut environment = Environment {
            entity: self.entity,
            causing: 0,
            invulnerable: false,
            client_loaded: self.client_loaded,
            rules: self.rules,
            source: self
                .resources
                .damage
                .source::<()>(id, None, None, None)
                .expect("validated starvation source"),
            living_cause: false,
            events: self.events,
            dead: self.dead,
            sync_velocity: self.sync_velocity,
            game_time: self.game_time,
            status: Default::default(),
            blocking: None,
            blocked: 0.0,
            fully_blocked: false,
            block_sound: false,
            knocked: false,
            totem: false,
            totem_fired: false,
            canceled: false,
        };
        if ServerPlayerCombat::new(self.attributes, self.data, food, self.ids, &mut environment)?
            .hurt(self.hurt, amount)?
        {
            *self.hurt_animation_time = self.hurt.hurt_time;
        }
        Ok(())
    }
}
