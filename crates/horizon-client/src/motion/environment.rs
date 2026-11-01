//! Player liquid travel and environmental damage over the shared fluid state.
use super::*;
use horizon_core::random::RandomSource;
use horizon_world::{
    fluid::FluidKind,
    living_fluid::{LivingFluidWorld, TravelFluidTag},
};
impl MotionWorld<'_, '_, '_> {
    pub(super) fn environment_hurt(&mut self, source: &str, damage: f32) -> Result<()> {
        if self.player.invulnerable || self.player.spectator {
            return Ok(());
        }
        // Frost walker's `damage_immunity`: #burn_from_stepping (magma, campfire).
        if self.player.burn_from_stepping_immune
            && matches!(source, "minecraft:hot_floor" | "minecraft:campfire")
        {
            return Ok(());
        }
        let id = self
            .resources
            .health
            .damage
            .id(source)
            .ok_or("missing environment damage type")?;
        self.player
            .hurt(&self.resources.health, self.scene.health_rules, id, damage)?;
        Ok(())
    }
    pub(super) fn tick_remaining_fire(&mut self) -> Result<()> {
        if self.player.spectator {
            self.player.fire_ticks = 0;
        }
        if self.player.invulnerable && self.player.fire_ticks > 1 {
            self.player.fire_ticks = 1;
        }
        if self.player.fire_ticks > 0 {
            if self.player.fire_ticks % 20 == 0 && !self.player.is_in_lava() {
                self.environment_hurt("minecraft:on_fire", 1.0)?;
            }
            self.player.fire_ticks -= 1;
        }
        if self.player.fluids.was_touching_water {
            self.player.fire_ticks = self.player.fire_ticks.min(0);
        }
        Ok(())
    }
    /// `LivingEntity.baseTick`'s air supply and drowning, then lava.
    pub fn tick_environment_damage(&mut self) -> Result<()> {
        let eye = BlockPos::containing(self.player.position.add_vector(Vec3::new(
            0.0,
            f64::from(self.player.dimensions.eye_height),
            0.0,
        )));
        let bubble_column = self
            .assets
            .registry
            .state(self.block(eye))
            .is_some_and(|state| state.name == "minecraft:bubble_column");
        let submerged = self.player.is_eye_in_fluid(WATER) && !bubble_column;
        if submerged
            && !self.player.status.water_breathing
            && !self.player.status.breath_of_the_nautilus
            && !self.player.invulnerable
            && !self.player.spectator
        {
            // decreaseAirSupply's getAttribute: the instance exists from now
            // on, even unmodified.
            let key = self.player.attributes.key("minecraft:oxygen_bonus");
            self.player
                .attributes
                .get_instance(key)
                .map_err(|e| format!("oxygen bonus: {e:?}"))?;
            let oxygen = self
                .player
                .attribute("minecraft:oxygen_bonus")?
                .max(f64::from(self.player.respiration));
            if oxygen <= 0.0 || self.scene.random.next_double() < 1.0 / (oxygen + 1.0) {
                self.player.air -= 1;
            }
            if self.player.air <= -20 {
                self.player.air = 0;
                self.player.events.push(MotionEvent::Drown);
                self.player.events.push(MotionEvent::Health(
                    crate::health::HealthEvent::DrownDamage(2.0),
                ));
                self.environment_hurt("minecraft:drown", 2.0)?;
            }
        } else if !submerged
            || !self.player.status.breath_of_the_nautilus
            || self.player.status.water_breathing
        {
            self.player.air = (self.player.air + 4).min(300);
        }
        if self.player.is_in_lava() {
            self.player.ignite_for_ticks(300)?;
            self.environment_hurt("minecraft:lava", 4.0)?;
        }
        Ok(())
    }
    /// What a server's `LivingEntity.baseTick` adds for a player it hosts:
    /// the void (`Entity.checkBelowWorld`, 4 a tick, which creative players
    /// take too) and suffocation (`isInWall`, which the caller decides from
    /// the blocks at the eyes).
    pub fn server_hazards_tick(&mut self, in_wall: bool) -> Result<()> {
        if self.player.position.y < f64::from(self.scene.bottom_y() - 64) {
            let id = self
                .resources
                .health
                .damage
                .id("minecraft:out_of_world")
                .ok_or("missing environment damage type")?;
            self.player
                .hurt(&self.resources.health, self.scene.health_rules, id, 4.0)?;
        }
        if in_wall {
            self.environment_hurt("minecraft:in_wall", 1.0)?;
        }
        Ok(())
    }
    pub fn apply_fire_contact(&mut self) -> Result<()> {
        if self.player.spectator {
            self.player.fire_ticks = 0;
            return Ok(());
        }
        let damage = fire_contact(self.scene, self.assets.registry, self.player.bounds());
        if damage > 0.0 {
            self.player.ticks_frozen = 0;
            if !self.player.fluids.was_touching_water {
                let multiplier = self.player.attribute("minecraft:burning_time")?;
                let ticks = ignite_contact(
                    self.player.fire_ticks,
                    true,
                    &mut self.scene.random,
                    multiplier,
                );
                self.player.set_remaining_fire_ticks(ticks);
            }
            self.environment_hurt("minecraft:in_fire", damage)?;
        } else if self.player.fire_ticks <= 0 {
            self.player.fire_ticks = -20;
        }
        Ok(())
    }
}
pub(crate) fn fire_contact(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    bounds: Aabb,
) -> f32 {
    let b = bounds.deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    let mut damage = 0.0f32;
    for x in b.min_position().x.floor() as i32..=b.max_position().x.floor() as i32 {
        for y in b.min_position().y.floor() as i32..=b.max_position().y.floor() as i32 {
            for z in b.min_position().z.floor() as i32..=b.max_position().z.floor() as i32 {
                damage = damage.max(
                    match registry
                        .state(world.block(x, y, z))
                        .map(|s| s.name.as_str())
                    {
                        Some("minecraft:fire") => 1.0,
                        Some("minecraft:soul_fire") => 2.0,
                        _ => 0.0,
                    },
                );
            }
        }
    }
    damage
}
/// `CactusBlock.entityInside`: the box (deflated by `1.0E-5`, as in
/// `Entity.checkInsideBlocks`) touches a cactus; its inside shape is the
/// whole block.
pub(crate) fn cactus_contact(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    bounds: Aabb,
) -> bool {
    let b = bounds.deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    for x in b.min_position().x.floor() as i32..=b.max_position().x.floor() as i32 {
        for y in b.min_position().y.floor() as i32..=b.max_position().y.floor() as i32 {
            for z in b.min_position().z.floor() as i32..=b.max_position().z.floor() as i32 {
                if registry
                    .state(world.block(x, y, z))
                    .is_some_and(|s| s.name == "minecraft:cactus")
                {
                    return true;
                }
            }
        }
    }
    false
}
impl LivingFluidWorld for MotionWorld<'_, '_, '_> {
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.fluids.was_touching_water)
    }
    fn sprinting(&mut self) -> Result<bool> {
        Ok(self.player.sprinting)
    }
    fn water_slow_down(&mut self) -> Result<f32> {
        Ok(0.8)
    }
    fn water_movement_efficiency(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:water_movement_efficiency")
    }
    fn dolphins_grace(&mut self) -> Result<bool> {
        Ok(self.player.status.dolphins_grace)
    }
    fn fluid_height(&mut self, tag: TravelFluidTag) -> Result<f64> {
        Ok(self.player.fluid_height(if tag == TravelFluidTag::Lava {
            LAVA
        } else {
            WATER
        }))
    }
    fn fluid_jump_threshold(&mut self) -> Result<f64> {
        Ok(self.player.fluid_jump_threshold())
    }
    fn is_free(&mut self, v: Vec3) -> Result<bool> {
        let b = self.player.bounds().move_by(v);
        Ok(self.no_player_collision(b)? && !self.has_liquid(b)?)
    }
    fn is_vehicle(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn can_float_while_ridden(&mut self) -> Result<bool> {
        Ok(false)
    }
}
impl PlayerMotion {
    /// `Entity.getFluidJumpThreshold`: 0.4, or 0 under eyes lower than that.
    pub fn fluid_jump_threshold(&self) -> f64 {
        horizon_world::swimming::fluid_jump_threshold(self.dimensions.eye_height)
    }
}
impl MotionWorld<'_, '_, '_> {
    /// `LivingEntity.aiStep`'s jump phase: rise in water or lava, else jump
    /// from the ground (or from shallow fluid); nothing while flying.
    pub(super) fn jump_phase(&mut self) -> Result<()> {
        if !self.player.jumping || self.player.flying {
            self.player.no_jump_delay = 0;
            return Ok(());
        }
        let in_lava = self.player.is_in_lava();
        let height = self.player.fluid_height(if in_lava { LAVA } else { WATER });
        let in_water = self.player.fluids.was_touching_water && height > 0.0;
        let threshold = self.player.fluid_jump_threshold();
        let on_ground = self.player.ground.on_ground;
        if in_water && !(on_ground && height <= threshold) {
            // jumpInLiquid(WATER)
            self.player.velocity.y += f64::from(0.04f32);
        } else if in_lava && !(on_ground && self.player.fluid_height(LAVA) <= threshold) {
            // jumpInLiquid(LAVA)
            self.player.velocity.y += f64::from(0.04f32);
        } else if (on_ground || in_water && height <= threshold) && self.player.no_jump_delay == 0 {
            self.jump_from_ground()?;
            self.player.no_jump_delay = 10;
        }
        Ok(())
    }
    /// `BubbleColumnBlock.entityInside` with `Player.onAboveBubbleColumn` /
    /// `onInsideBubbleColumn`: pushed up (or dragged down), faster at the
    /// surface, never while flying.
    pub(super) fn bubble_column_inside(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        precise: bool,
    ) -> Result<()> {
        if !precise {
            return Ok(());
        }
        let drag = self
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("drag"))
            .is_some_and(|d| d == "true");
        let above = self.block(p.offset(0, 1, 0));
        // stateAbove.getCollisionShape(level, pos): the shape is asked at the
        // column's own position.
        let binding = self.collision_binding;
        let context = CollisionContext::empty(false);
        let empty = binding
            .collision_shape_with_context(&mut self.collisions(), above, p, &context)?
            .is_empty()
            && self.resources.fluids.fluid(above).kind() == FluidKind::Empty;
        if self.player.flying {
            return Ok(());
        }
        let v = self.player.velocity;
        let y = match (empty, drag) {
            (true, true) => (-0.9_f64).max(v.y - 0.03),
            (true, false) => 1.8_f64.min(v.y + 0.1),
            (false, true) => (-0.3_f64).max(v.y - 0.03),
            (false, false) => 0.7_f64.min(v.y + 0.06),
        };
        self.player.velocity = Vec3::new(v.x, y, v.z);
        if empty {
            // Entity.sendBubbleColumnParticles: two splashes and two bubbles
            // over the column's top, from the level random.
            let random = &mut self.scene.random;
            for bubble in [false, true, false, true] {
                let x = f64::from(p.x) + random.next_double();
                let z = f64::from(p.z) + random.next_double();
                self.player.events.push(MotionEvent::BubbleColumnParticle {
                    bubble,
                    position: Vec3::new(x, f64::from(p.y + 1), z),
                });
            }
        } else {
            self.player.fall_distance = 0.0;
        }
        Ok(())
    }
    fn has_liquid(&self, b: Aabb) -> Result<bool> {
        for x in b.min_position().x.floor() as i32..=b.max_position().x.floor() as i32 {
            for y in b.min_position().y.floor() as i32..=b.max_position().y.floor() as i32 {
                for z in b.min_position().z.floor() as i32..=b.max_position().z.floor() as i32 {
                    if self.fluid_at(BlockPos::new(x, y, z)).kind() != FluidKind::Empty {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

pub(crate) fn ignite_contact(
    ticks: i32,
    player: bool,
    random: &mut impl RandomSource,
    multiplier: f64,
) -> i32 {
    let ticks = if ticks < 0 {
        ticks + 1
    } else if player {
        ticks + 1 + random.next_int_bound(2).unwrap_or(0)
    } else {
        ticks
    };
    if ticks >= 0 {
        ticks.max((160.0 * multiplier).ceil() as i32)
    } else {
        ticks
    }
}

#[cfg(test)]
mod fire_ignition_tests {
    use super::*;
    #[test]
    fn fire_hurts_before_twenty_tick_ignition_and_scales_duration() {
        let mut r = horizon_core::random::LegacyRandom::new(3);
        let mut ticks = -20;
        for _ in 0..19 {
            ticks = ignite_contact(ticks, true, &mut r, 0.4);
            assert!(ticks < 0);
        }
        assert_eq!(ignite_contact(ticks, true, &mut r, 0.4), 64);
    }
}
