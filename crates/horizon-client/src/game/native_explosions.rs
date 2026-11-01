//! Explosions for native server mods (docs/native-mods.md, "Explosions"): a mod
//! sets one off as `Level.explode` does, in the level in place, which it names
//! by its `dimension` (`minecraft:overworld`) as `native_set_block` does.
use super::{Game, explosion::BlockInteraction};

/// The largest radius `native_explode` takes. A blast's cost grows with its
/// radius (its 1352 rays walk `radius * 1.3` blocks at most); vanilla's
/// largest is the wither's charged spawn, 7.
pub(super) const NATIVE_EXPLOSION_MAX_RADIUS: f32 = 32.0;

/// What a mod's blast does to blocks (`Level.ExplosionInteraction`). Built by
/// mods only.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum NativeBlast {
    /// Blocks stay; entities are hurt and pushed (`ExplosionInteraction.NONE`).
    Keep,
    /// Blocks break and drop by the `block_explosion_drop_decay` rule
    /// (`ExplosionInteraction.BLOCK`).
    #[default]
    Block,
    /// Blocks break and drop by the `tnt_explosion_drop_decay` rule
    /// (`ExplosionInteraction.TNT`).
    Tnt,
}

/// Whether `native_explode` takes this centre and radius.
fn blast_ok(center: [f64; 3], radius: f32) -> bool {
    center
        .into_iter()
        .all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
        && radius.is_finite()
        && radius > 0.0
        && radius <= NATIVE_EXPLOSION_MAX_RADIUS
}

#[allow(dead_code)]
impl Game<'_> {
    /// Explodes at `center` in the level in place, `dimension`, as
    /// `Level.explode(null, x, y, z, radius, fire, interaction)` does: the
    /// blocks it reaches break (`native_explosion_detonate` sees them first),
    /// entities are hurt and pushed, `fire` lights a third of the air above
    /// solid blocks, and everyone near hears and sees it. False, with nothing
    /// done, on a client's copy of the world, for another dimension, a chunk
    /// that isn't loaded, or a radius that isn't in `0 < radius <= 32`. Call
    /// it from a server hook; a blast inside `native_explosion_detonate`
    /// starts that hook again. API 66.
    pub(super) fn native_explode(
        &mut self,
        dimension: &str,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        blast: NativeBlast,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none())
            || dimension != self.dim.id()
            || !blast_ok(center, radius)
        {
            return false;
        }
        let chunk = (
            (center[0].floor() as i32) >> 4,
            (center[2].floor() as i32) >> 4,
        );
        if !self.world.chunks.contains_key(&chunk) {
            return false;
        }
        let interaction = match blast {
            NativeBlast::Keep => BlockInteraction::Keep,
            NativeBlast::Block => self.destroy_type("block_explosion_drop_decay"),
            NativeBlast::Tnt => self.destroy_type("tnt_explosion_drop_decay"),
        };
        // `Level.explode(null, ...)`: no source. Called from inside another
        // blast (`native_explosion_detonate`), it must not take that blast's
        // source, owner or flags, and that blast gets them back after.
        let outer = self.take_blast_state();
        let done = self
            .server_explode(center, radius, interaction, fire)
            .is_ok();
        self.restore_blast_state(outer);
        done
    }

    /// The blast going off now, cleared to a sourceless one.
    fn take_blast_state(&mut self) -> BlastState {
        let c = &mut self.combat;
        BlastState {
            source: c.explosion_source.take(),
            cause: c.explosion_cause.take(),
            source_named: std::mem::take(&mut c.explosion_source_named),
            in_water: std::mem::take(&mut c.explosion_in_water),
            hive_emptying: std::mem::take(&mut c.hive_emptying_blast),
            rail_proof: std::mem::take(&mut c.rail_proof_blast),
            owner: c.explosion_owner.take(),
        }
    }

    /// Puts back what `take_blast_state` cleared.
    fn restore_blast_state(&mut self, s: BlastState) {
        let c = &mut self.combat;
        c.explosion_source = s.source;
        c.explosion_cause = s.cause;
        c.explosion_source_named = s.source_named;
        c.explosion_in_water = s.in_water;
        c.hive_emptying_blast = s.hive_emptying;
        c.rail_proof_blast = s.rail_proof;
        c.explosion_owner = s.owner;
    }
}

/// The fields of `CombatTracker` that describe the blast going off now.
struct BlastState {
    source: Option<u32>,
    cause: Option<horizon_client::advancements::EntityView>,
    source_named: bool,
    in_water: bool,
    hive_emptying: bool,
    rail_proof: bool,
    owner: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blast_needs_a_finite_centre_and_a_radius_in_range() {
        assert!(blast_ok([0.5, 64.0, 0.5], 4.0));
        assert!(blast_ok([0.5, 64.0, 0.5], NATIVE_EXPLOSION_MAX_RADIUS));
        assert!(!blast_ok([0.5, 64.0, 0.5], 0.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], -1.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], 33.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], f32::NAN));
        assert!(!blast_ok([f64::INFINITY, 64.0, 0.5], 4.0));
        assert!(!blast_ok([0.5, 64.0, 4.0e7], 4.0));
    }
}
