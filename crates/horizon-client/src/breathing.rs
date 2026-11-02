//! Breathing for native mods: `LivingEntity.baseTick`'s air supply, the part
//! that raises or lowers it. A player (`motion/environment.rs`) and a mob
//! (`mobs/ai.rs`) work out what vanilla does this tick, `breathe` lets the
//! mods change it (`native_living_breathe`, NeoForge's `LivingBreatheEvent`),
//! and the caller applies the answer. Servers only: the callers keep it off a
//! client's prediction.

/// One tick of breathing: whether the entity can breathe, how much air it
/// loses when it cannot and how much it gains when it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Breath {
    pub can_breathe: bool,
    /// `decreaseAirSupply`: 1, or 0 by the oxygen bonus's chance, for an
    /// entity that drowns; 1 for one that would not.
    pub consume: i32,
    /// `increaseAirSupply`: 4 for one that can breathe, 0 for one that drowns
    /// or is held under by the nautilus' breath.
    pub refill: i32,
}

/// An entity's breathing this tick (`LivingBreatheEvent`, posted from
/// `LivingEntity.baseTick` every tick for a living entity with the generic
/// air logic): `entity` is its ID and `kind` its type (`minecraft:player`,
/// `minecraft:zombie`). `can_breathe`, `consume` and `refill` come in as
/// Horizon does it today (a mod-free game changes nothing); set them to
/// change this tick. With `can_breathe` the air rises by `refill`, up to its
/// maximum (300); without, it falls by `consume`, and at -20 it is set to 0
/// and the entity takes 2 drowning damage. Negative amounts count as 0.
/// Asked on servers for players and for mobs, except those with a
/// `WaterAnimal`'s or an axolotl's own air logic. API 144.
#[inline(never)]
pub fn native_living_breathe(
    entity: u32,
    kind: &str,
    can_breathe: &mut bool,
    consume: &mut i32,
    refill: &mut i32,
) {
    crate::native_client_hooks::living_breathe(entity, kind, can_breathe, consume, refill);
}

/// `vanilla`'s breathing after the mods have had their say.
pub fn breathe(entity: u32, kind: &str, vanilla: Breath) -> Breath {
    let mut now = vanilla;
    native_living_breathe(
        entity,
        kind,
        &mut now.can_breathe,
        &mut now.consume,
        &mut now.refill,
    );
    Breath {
        can_breathe: now.can_breathe,
        consume: now.consume.max(0),
        refill: now.refill.max(0),
    }
}

/// The air supply after `breath`: raised up to `max` when it can breathe,
/// lowered otherwise. The second is whether the entity now drowns
/// (`LivingEntity.shouldTakeDrowningDamage`: -20 or less), and its air is
/// then 0.
pub fn apply(air: i32, max: i32, breath: Breath) -> (i32, bool) {
    if breath.can_breathe {
        return (air.max((air + breath.refill).min(max)), false);
    }
    let air = air - breath.consume;
    if air <= -20 { (0, true) } else { (air, false) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VANILLA: Breath = Breath {
        can_breathe: true,
        consume: 1,
        refill: 4,
    };

    #[test]
    fn air_refills_up_to_the_maximum() {
        assert_eq!(apply(100, 300, VANILLA), (104, false));
        assert_eq!(apply(299, 300, VANILLA), (300, false));
        assert_eq!(apply(300, 300, VANILLA), (300, false));
        // A refill of 0 leaves it, as does one that would pass the maximum.
        let held = Breath {
            refill: 0,
            ..VANILLA
        };
        assert_eq!(apply(120, 300, held), (120, false));
    }

    #[test]
    fn air_runs_down_and_drowns_at_minus_twenty() {
        let drowning = Breath {
            can_breathe: false,
            consume: 1,
            refill: 0,
        };
        assert_eq!(apply(300, 300, drowning), (299, false));
        assert_eq!(apply(-18, 300, drowning), (-19, false));
        assert_eq!(apply(-19, 300, drowning), (0, true));
        // The oxygen bonus can spare a tick, and a mod can take more.
        let spared = Breath {
            consume: 0,
            ..drowning
        };
        assert_eq!(apply(-19, 300, spared), (-19, false));
        let gas = Breath {
            consume: 30,
            ..drowning
        };
        assert_eq!(apply(20, 300, gas), (-10, false));
        assert_eq!(apply(10, 300, gas), (0, true));
    }

    #[test]
    fn nothing_changes_without_a_mod() {
        assert_eq!(breathe(1, "minecraft:zombie", VANILLA), VANILLA);
        let negative = Breath {
            can_breathe: false,
            consume: -3,
            refill: -2,
        };
        // Mods only ever answer with their own values, so what comes back is clamped.
        assert_eq!(breathe(1, "minecraft:player", negative).consume, 0);
        assert_eq!(breathe(1, "minecraft:player", negative).refill, 0);
    }
}
