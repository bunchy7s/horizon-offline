//! Random teleport consumable: candidate draws, bounds and success effects.
use horizon_core::{geometry::Vec3, position::BlockPos, random::RandomSource};
#[derive(Clone, Copy, Debug)]
pub struct TeleportConsume {
    pub diameter: f32,
    pub directional_particles: bool,
}
impl PartialEq for TeleportConsume {
    fn eq(&self, other: &Self) -> bool {
        let bits = |v: f32| if v.is_nan() { 0x7fc00000 } else { v.to_bits() };
        bits(self.diameter) == bits(other.diameter)
            && self.directional_particles == other.directional_particles
    }
}
pub trait TeleportConsumeWorld {
    type Error;
    type Level: Copy;
    type Random: Copy;
    type Source: RandomSource;
    fn position_field(&self) -> Vec3;
    fn get_random(&mut self) -> Result<Self::Random, Self::Error>;
    fn random_source(&mut self, random: Self::Random) -> &mut Self::Source;
    fn min_y(&mut self, level: Self::Level) -> Result<i32, Self::Error>;
    /// Native execution casts the supplied level to ServerLevel here.
    fn logical_height(&mut self, level: Self::Level) -> Result<i32, Self::Error>;
    fn is_passenger(&mut self) -> Result<bool, Self::Error>;
    fn stop_riding(&mut self) -> Result<(), Self::Error>;
    fn position(&mut self) -> Result<Vec3, Self::Error>;
    /// Horizon's mods (NeoForge's `EntityTeleportEvent.ChorusFruit`): before
    /// each attempt, after the user stopped riding. They may move `target`;
    /// false stops the effect without teleporting and without more draws.
    fn teleport_allowed(&mut self, target: &mut Vec3) -> Result<bool, Self::Error> {
        let _ = target;
        Ok(true)
    }
    /// showParticles=true and forbidden tag=CONSUMABLE_DOES_NOT_TELEPORT_TO.
    fn random_teleport(&mut self, position: Vec3) -> Result<bool, Self::Error>;
    /// Level TELEPORT event, old position, context containing this actor.
    fn teleport_event(&mut self, level: Self::Level, old_position: Vec3)
    -> Result<(), Self::Error>;
    fn is_fox(&self) -> bool;
    /// Fox sound/NEUTRAL when fox=true; chorus-fruit sound/PLAYERS otherwise.
    /// Native excluded entity is null and default volume/pitch are one.
    fn teleport_sound(
        &mut self,
        level: Self::Level,
        position: Vec3,
        fox: bool,
    ) -> Result<(), Self::Error>;
    fn block_position(&mut self) -> Result<BlockPos, Self::Error>;
    /// Native level event 2017, at the origin, carrying the packed offset.
    fn directional_particles(
        &mut self,
        level: Self::Level,
        origin: BlockPos,
        offset: i32,
    ) -> Result<(), Self::Error>;
    fn reset_fall_distance(&mut self) -> Result<(), Self::Error>;
    fn reset_impulse(&mut self) -> Result<(), Self::Error>;
}
fn clamp(v: f64, low: f64, high: f64) -> f64 {
    if v < low {
        low
    } else if v.is_nan() || high.is_nan() {
        f64::NAN
    } else if v == 0.0 && high == 0.0 {
        f64::from_bits(v.to_bits() | high.to_bits())
    } else {
        v.min(high)
    }
}
fn packed_offset(a: BlockPos, b: BlockPos) -> i32 {
    let axis = |a: i32, b: i32| b.wrapping_sub(a).clamp(-127, 127) + 127;
    (axis(a.x, b.x) << 16) | (axis(a.y, b.y) << 8) | axis(a.z, b.z)
}
impl TeleportConsume {
    pub fn apply<W: TeleportConsumeWorld>(
        &self,
        world: &mut W,
        level: W::Level,
    ) -> Result<bool, W::Error> {
        for _ in 0..16 {
            let x = world.position_field().x;
            let r = world.get_random()?;
            let x = x + (world.random_source(r).next_double() - 0.5) * f64::from(self.diameter);
            let y = world.position_field().y;
            let r = world.get_random()?;
            let y = y + (world.random_source(r).next_double() - 0.5) * f64::from(self.diameter);
            let low = world.min_y(level)?;
            let min = world.min_y(level)?;
            let height = world.logical_height(level)?;
            let high = min.wrapping_add(height).wrapping_sub(1);
            let y = clamp(y, f64::from(low), f64::from(high));
            let z = world.position_field().z;
            let r = world.get_random()?;
            let z = z + (world.random_source(r).next_double() - 0.5) * f64::from(self.diameter);
            if world.is_passenger()? {
                world.stop_riding()?;
            }
            let old = world.position()?;
            let mut target = Vec3::new(x, y, z);
            if !world.teleport_allowed(&mut target)? {
                return Ok(false);
            }
            if world.random_teleport(target)? {
                world.teleport_event(level, old)?;
                let fox = world.is_fox();
                world.teleport_sound(level, world.position_field(), fox)?;
                if self.directional_particles {
                    let origin = BlockPos::new(
                        old.x.floor() as i32,
                        old.y.floor() as i32,
                        old.z.floor() as i32,
                    );
                    let destination = world.block_position()?;
                    world.directional_particles(
                        level,
                        origin,
                        packed_offset(origin, destination),
                    )?;
                }
                world.reset_fall_distance()?;
                world.reset_impulse()?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;
    use std::convert::Infallible;

    /// A user at the origin whose every landing fails unless `lands`; it
    /// records the targets tried and how the mods answer.
    struct Stub {
        random: LegacyRandom,
        allow: fn(&mut Vec3) -> bool,
        lands: bool,
        asked: usize,
        tried: Vec<Vec3>,
    }
    impl TeleportConsumeWorld for Stub {
        type Error = Infallible;
        type Level = ();
        type Random = ();
        type Source = LegacyRandom;
        fn position_field(&self) -> Vec3 {
            Vec3::ZERO
        }
        fn get_random(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
        fn random_source(&mut self, (): ()) -> &mut LegacyRandom {
            &mut self.random
        }
        fn min_y(&mut self, (): ()) -> Result<i32, Infallible> {
            Ok(-64)
        }
        fn logical_height(&mut self, (): ()) -> Result<i32, Infallible> {
            Ok(384)
        }
        fn is_passenger(&mut self) -> Result<bool, Infallible> {
            Ok(false)
        }
        fn stop_riding(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
        fn position(&mut self) -> Result<Vec3, Infallible> {
            Ok(Vec3::ZERO)
        }
        fn teleport_allowed(&mut self, target: &mut Vec3) -> Result<bool, Infallible> {
            self.asked += 1;
            Ok((self.allow)(target))
        }
        fn random_teleport(&mut self, position: Vec3) -> Result<bool, Infallible> {
            self.tried.push(position);
            Ok(self.lands)
        }
        fn teleport_event(&mut self, (): (), _: Vec3) -> Result<(), Infallible> {
            Ok(())
        }
        fn is_fox(&self) -> bool {
            false
        }
        fn teleport_sound(&mut self, (): (), _: Vec3, _: bool) -> Result<(), Infallible> {
            Ok(())
        }
        fn block_position(&mut self) -> Result<BlockPos, Infallible> {
            Ok(BlockPos::new(0, 0, 0))
        }
        fn directional_particles(&mut self, (): (), _: BlockPos, _: i32) -> Result<(), Infallible> {
            Ok(())
        }
        fn reset_fall_distance(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
        fn reset_impulse(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
    }
    fn stub(allow: fn(&mut Vec3) -> bool, lands: bool) -> Stub {
        Stub {
            random: LegacyRandom::new(7),
            allow,
            lands,
            asked: 0,
            tried: Vec::new(),
        }
    }
    const CHORUS: TeleportConsume = TeleportConsume {
        diameter: 16.0,
        directional_particles: false,
    };

    #[test]
    fn a_refused_attempt_ends_the_effect_without_teleporting() {
        let mut w = stub(|_| false, true);
        assert_eq!(CHORUS.apply(&mut w, ()), Ok(false));
        assert_eq!((w.asked, w.tried.len()), (1, 0), "asked once, never tried");
    }

    #[test]
    fn the_mods_target_is_where_the_user_tries_to_land() {
        let mut w = stub(
            |t| {
                *t = Vec3::new(100.5, 70.0, -3.5);
                true
            },
            true,
        );
        assert_eq!(CHORUS.apply(&mut w, ()), Ok(true));
        assert_eq!(w.tried, [Vec3::new(100.5, 70.0, -3.5)]);
    }

    #[test]
    fn without_mods_all_sixteen_attempts_stay_vanillas() {
        let mut w = stub(|_| true, false);
        assert_eq!(CHORUS.apply(&mut w, ()), Ok(false));
        assert_eq!((w.asked, w.tried.len()), (16, 16));
        // Each target is within half the diameter of the user.
        assert!(w.tried.iter().all(|t| t.x.abs() <= 8.0 && t.z.abs() <= 8.0));
    }
}
