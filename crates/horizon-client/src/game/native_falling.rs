//! Falling blocks for native server mods (docs/native-mods.md, "Falling
//! blocks"): a mod throws a block as a `FallingBlockEntity` that flies, lands
//! and places itself (or drops as an item) as sand does. It acts where the
//! world is simulated, in the level in place, which it names by its
//! `dimension` (`minecraft:overworld`) as `native_set_block` does.
use super::{FallingBlock, Game};
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::registry::BlockStateId;

/// The most falling blocks a level holds before `native_spawn_falling_block`
/// refuses more: a mod's debris shouldn't take the server's tick with it.
pub(super) const NATIVE_FALLING_MAX: usize = 1024;
/// The largest speed a thrown block starts with, in blocks a tick on each
/// axis (`Entity.setDeltaMovement` takes any; a packet's speed is cut to 3.9).
const NATIVE_FALLING_MAX_SPEED: f64 = 3.9;

/// What a mod's falling block does when it lands and when it hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativeFalling {
    /// `FallingBlockEntity.dropItem`: whether it drops as an item where it
    /// can't be placed.
    pub drop_item: bool,
    /// `FallingBlockEntity.hurtEntities`, at 2 a block fallen and at most 40,
    /// an anvil's.
    pub hurts: bool,
}

impl Default for NativeFalling {
    fn default() -> Self {
        Self {
            drop_item: true,
            hurts: false,
        }
    }
}

/// `velocity` with each axis cut to the largest start speed; none for a
/// value that isn't finite.
fn start_velocity(velocity: [f64; 3]) -> Option<Vec3> {
    velocity.iter().all(|v| v.is_finite()).then(|| {
        let [x, y, z] =
            velocity.map(|v| v.clamp(-NATIVE_FALLING_MAX_SPEED, NATIVE_FALLING_MAX_SPEED));
        Vec3::new(x, y, z)
    })
}

#[allow(dead_code)]
impl Game<'_> {
    /// Throws a falling block of `state` from `pos` (its bottom centre) at
    /// `velocity` (blocks a tick) in the level in place, `dimension`: it
    /// falls, lands and places itself as sand does, or drops as an item
    /// where it can't. The block at `pos` is left as it is: take it away first
    /// with `native_set_block` to make a block fall out of the world. False,
    /// with nothing thrown, on a client's copy of the world, for another
    /// dimension, an air or unknown state, a chunk that isn't loaded, a value
    /// that isn't finite, or with 1024 falling blocks in the level already.
    /// Each axis of `velocity` is cut to 3.9. API @API:native_falling_block@.
    pub(super) fn native_spawn_falling_block(
        &mut self,
        dimension: &str,
        state: BlockStateId,
        pos: [f64; 3],
        velocity: [f64; 3],
        falling: NativeFalling,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || self.pack.assets.registry.state(state).is_none()
            || self.res.facts().is_air(state)
            || self.falling.len() >= NATIVE_FALLING_MAX
            || !pos.iter().all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
        {
            return false;
        }
        let Some(velocity) = start_velocity(velocity) else {
            return false;
        };
        let cell = BlockPos::new(
            pos[0].floor() as i32,
            pos[1].floor() as i32,
            pos[2].floor() as i32,
        );
        if !self.world.chunks.contains_key(&(cell.x >> 4, cell.z >> 4)) {
            return false;
        }
        let mut damage =
            horizon_client::falling_block::Damage::falling(&self.res.facts().get(state).name);
        if falling.hurts {
            damage.hurt_entities = true;
            damage.per_distance = 2.0;
            damage.max = 40;
        }
        let at = Vec3::new(pos[0], pos[1], pos[2]);
        self.falling.push(FallingBlock {
            id: 0,
            state,
            start_pos: cell,
            position: at,
            previous: at,
            velocity,
            age: 0,
            no_gravity: false,
            drop_item: falling.drop_item,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            damage,
            saved: None,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_throw_is_cut_to_the_largest_speed_and_refuses_what_isnt_finite() {
        assert_eq!(
            start_velocity([0.5, 9.0, -9.0]),
            Some(Vec3::new(
                0.5,
                NATIVE_FALLING_MAX_SPEED,
                -NATIVE_FALLING_MAX_SPEED
            ))
        );
        assert_eq!(start_velocity([0.0, f64::NAN, 0.0]), None);
        assert_eq!(start_velocity([f64::INFINITY, 0.0, 0.0]), None);
    }
}
