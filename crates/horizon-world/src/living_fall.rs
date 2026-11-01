//! Living-entity fall damage calculation, impulse exemption and callback order.
use horizon_core::geometry::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImpulseContext {
    pub reset_grace_time: i32,
    pub impact_position: Option<Vec3>,
}
impl ImpulseContext {
    pub fn set_ignore_fall_damage(&mut self, ignore: bool, position: Option<Vec3>) {
        if ignore {
            self.apply_grace_time(40);
            self.impact_position = position;
        } else {
            self.reset_grace_time = 0;
        }
    }
    pub fn apply_grace_time(&mut self, ticks: i32) {
        self.reset_grace_time = self.reset_grace_time.max(ticks);
    }
    pub fn reset(&mut self) {
        self.reset_grace_time = 0;
        self.impact_position = None;
    }
    pub fn try_reset(&mut self) {
        if self.reset_grace_time == 0 {
            self.reset();
        }
    }
    /// `LivingEntity.hasLandedInLiquid`: a move into a liquid with no upward
    /// movement ends the impulse context, still deferred by the grace time.
    pub fn landed_in_liquid(&mut self, delta_y: f64, in_liquid: bool) {
        if in_liquid && delta_y < 9.9999997e-6 {
            self.try_reset();
        }
    }
    pub fn is_in_grace_time(&self) -> bool {
        self.reset_grace_time > 0
    }
    pub fn ignores_fall_damage(&self) -> bool {
        self.impact_position.is_some()
    }
}
pub trait LivingFallWorld {
    type Error;
    type Sound: Copy;
    fn impact_position(&self) -> Option<Vec3>;
    fn y(&self) -> f64;
    fn reset_impulse(&mut self) -> Result<(), Self::Error>;
    fn try_reset_impulse(&mut self) -> Result<(), Self::Error>;
    fn immune_to_fall_damage(&mut self) -> Result<bool, Self::Error>;
    fn propagate_fall_to_passengers(
        &mut self,
        distance: f64,
        multiplier: f32,
    ) -> Result<(), Self::Error>;
    fn safe_fall_distance(&mut self) -> Result<f64, Self::Error>;
    fn fall_damage_multiplier(&mut self) -> Result<f64, Self::Error>;
    /// A host may adjust a mod block's landing damage before sounds and hurt.
    fn modify_fall_damage(&mut self, _distance: f64, damage: f32) -> Result<f32, Self::Error> {
        Ok(damage)
    }
    /// Select the entity type's small/big fall sound. Big applies above four.
    fn fall_sound(&mut self, big: bool) -> Result<Self::Sound, Self::Error>;
    fn play_fall_sound(&mut self, sound: Self::Sound) -> Result<(), Self::Error>;
    fn play_block_fall_sound(&mut self) -> Result<(), Self::Error>;
    /// Dispatch native Entity.hurt semantics (server-only), preserving its source.
    fn hurt(&mut self, damage: f32) -> Result<(), Self::Error>;
}
pub fn calculate_fall_damage<W: LivingFallWorld>(
    world: &mut W,
    distance: f64,
    multiplier: f32,
) -> Result<i32, W::Error> {
    if world.immune_to_fall_damage()? {
        return Ok(0);
    }
    let power = distance + 1.0e-6 - world.safe_fall_distance()?;
    Ok((power * f64::from(multiplier) * world.fall_damage_multiplier()?).floor() as i32)
}
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        f64::from_bits(a.to_bits() | b.to_bits())
    } else {
        a.min(b)
    }
}
/// Default LivingEntity and Entity fall behavior. Entity-type overrides of the
/// calculation/parent method are a separate dispatch layer. The same damage
/// source must be retained by the provider for riders and the final hurt call.
pub fn cause_living_fall_damage<W: LivingFallWorld>(
    world: &mut W,
    distance: f64,
    multiplier: f32,
) -> Result<bool, W::Error> {
    let distance = if let Some(impact) = world.impact_position() {
        let distance = java_min(distance, impact.y - world.y());
        if distance <= 0.0 {
            world.reset_impulse()?;
        } else {
            world.try_reset_impulse()?;
        }
        distance
    } else {
        distance
    };
    // Base Entity.causeFallDamage returns false, even after notifying riders.
    if !world.immune_to_fall_damage()? {
        world.propagate_fall_to_passengers(distance, multiplier)?;
    }
    let damage = calculate_fall_damage(world, distance, multiplier)?;
    let damage = world.modify_fall_damage(distance, damage as f32)?;
    if damage > 0.0 {
        world.reset_impulse()?;
        let sound = world.fall_sound(damage > 4.0)?;
        world.play_fall_sound(sound)?;
        world.play_block_fall_sound()?;
        world.hurt(damage)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;

    struct Landing {
        replacement: Option<f32>,
        damage: f32,
        sounds: usize,
    }
    impl LivingFallWorld for Landing {
        type Error = ();
        type Sound = bool;
        fn impact_position(&self) -> Option<Vec3> {
            None
        }
        fn y(&self) -> f64 {
            0.0
        }
        fn reset_impulse(&mut self) -> Result<(), ()> {
            Ok(())
        }
        fn try_reset_impulse(&mut self) -> Result<(), ()> {
            Ok(())
        }
        fn immune_to_fall_damage(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn propagate_fall_to_passengers(&mut self, _: f64, _: f32) -> Result<(), ()> {
            Ok(())
        }
        fn safe_fall_distance(&mut self) -> Result<f64, ()> {
            Ok(3.0)
        }
        fn fall_damage_multiplier(&mut self) -> Result<f64, ()> {
            Ok(1.0)
        }
        fn modify_fall_damage(&mut self, distance: f64, damage: f32) -> Result<f32, ()> {
            assert_eq!((distance, damage), (13.0, 10.0));
            assert_eq!((self.sounds, self.damage), (0, 0.0));
            Ok(self.replacement.unwrap_or(damage))
        }
        fn fall_sound(&mut self, big: bool) -> Result<bool, ()> {
            Ok(big)
        }
        fn play_fall_sound(&mut self, _: bool) -> Result<(), ()> {
            self.sounds += 1;
            Ok(())
        }
        fn play_block_fall_sound(&mut self) -> Result<(), ()> {
            self.sounds += 1;
            Ok(())
        }
        fn hurt(&mut self, damage: f32) -> Result<(), ()> {
            self.damage = damage;
            Ok(())
        }
    }
    #[test]
    fn landing_damage_is_changed_before_sounds_and_hurt() {
        for (replacement, expected) in [(None, 10.0), (Some(2.0), 2.0), (Some(0.0), 0.0)] {
            let mut w = Landing {
                replacement,
                damage: 0.0,
                sounds: 0,
            };
            assert_eq!(
                cause_living_fall_damage(&mut w, 13.0, 1.0),
                Ok(expected > 0.0)
            );
            assert_eq!(w.damage, expected);
            assert_eq!(w.sounds, if expected > 0.0 { 2 } else { 0 });
        }
    }

    /// A wind charge's or mace's impact position with no grace time left.
    fn impact() -> ImpulseContext {
        ImpulseContext {
            reset_grace_time: 0,
            impact_position: Some(Vec3::new(0.5, 64.0, 0.5)),
        }
    }

    #[test]
    fn a_move_that_lands_in_a_liquid_ends_the_impulse_context() {
        let mut impulse = impact();
        impulse.landed_in_liquid(-0.08, true);
        assert!(impulse.impact_position.is_none());
    }

    #[test]
    fn a_move_that_lands_on_dry_ground_keeps_it() {
        let mut impulse = impact();
        impulse.landed_in_liquid(-0.08, false);
        assert_eq!(impulse.impact_position, Some(Vec3::new(0.5, 64.0, 0.5)));
    }

    #[test]
    fn a_rise_through_a_liquid_keeps_it() {
        let mut impulse = impact();
        impulse.landed_in_liquid(0.2, true);
        assert!(impulse.ignores_fall_damage());
    }

    #[test]
    fn the_grace_time_defers_the_liquid_reset() {
        let mut impulse = ImpulseContext::default();
        impulse.set_ignore_fall_damage(true, Some(Vec3::new(0.5, 64.0, 0.5)));
        impulse.landed_in_liquid(-0.08, true);
        assert!(impulse.ignores_fall_damage());
    }
}
