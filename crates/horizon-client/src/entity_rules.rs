//! PURPOSE: `Entity`'s gravity and fluid-push rules, decided once for every kind.
//! WHY: each kind used to check its own `NoGravity` flag and pick its own
//! number, so a kind that forgot the check ignored the flag (#6481).
//! REFS: vanilla `Entity.isNoGravity`, `getDefaultGravity`, `getGravity`,
//! `applyGravity`, `isPushedByFluid`; issue #6509, slice #6544.
//! LOGS: none. READS-BY: the kinds' ticks in this crate and in `game/`.
//! ENDS-WHEN: kinds share one stored `Entity` state and this folds into it.

/// The rules vanilla's `Entity` answers for every kind. A kind overrides one
/// only where its vanilla class does; the defaults are `Entity`'s.
pub trait EntityRules {
    /// `Entity.isNoGravity` (`NoGravity` in the save).
    fn is_no_gravity(&self) -> bool {
        false
    }
    /// `Entity.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.0
    }
    /// `Entity.getGravity`: none with `NoGravity`, else the kind's default.
    /// `applyGravity` subtracts this from the vertical velocity.
    fn get_gravity(&self) -> f64 {
        if self.is_no_gravity() {
            0.0
        } else {
            self.get_default_gravity()
        }
    }
    /// `Entity.isPushedByFluid`.
    fn is_pushed_by_fluid(&self) -> bool {
        true
    }
}
