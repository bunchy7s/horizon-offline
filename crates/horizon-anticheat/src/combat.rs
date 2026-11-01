//! Combat: attacks vanilla's client can't send (docs/anticheat.md).
//!
//! Vanilla's client sends ServerboundAttackPacket from one place only, `Minecraft.startAttack`, run from
//! `handleKeybinds` once per client tick. It attacks the entity of the tick's `hitResult`, which
//! `Minecraft.tick` picks once before the keys are handled, so every attack of one client tick is on the
//! same entity. And each attack swings the arm (ServerboundPunchPacket) in the same call, so a client
//! tick never has more attacks than swings: a tick that does comes from a kill aura hiding its swing.
//! The swing is counted before or after the attack, as older clients behind a version proxy swing
//! first; a swing without an attack (a miss, a block) is fine.

/// What breaks vanilla's way of attacking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Foul {
    /// A client tick with more attacks than swings: how many more.
    NoSwing(u32),
    /// A second entity attacked in the same client tick: the first one and this one.
    TwoTargets(i32, i32),
}

/// The attacks of the client tick being heard.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Attacks {
    /// The entity this tick attacked first.
    target: Option<i32>,
    attacks: u32,
    swings: u32,
}

impl Attacks {
    /// ServerboundAttackPacket on entity `target`.
    pub fn attack(&mut self, target: i32) -> Option<Foul> {
        self.attacks = self.attacks.saturating_add(1);
        let first = *self.target.get_or_insert(target);
        (first != target).then_some(Foul::TwoTargets(first, target))
    }

    /// ServerboundPunchPacket: the arm swung.
    pub fn swing(&mut self) {
        self.swings = self.swings.saturating_add(1);
    }

    /// ServerboundClientTickEndPacket: the tick's attacks are counted, and the next tick picks its
    /// target again.
    pub fn tick_end(&mut self) -> Option<Foul> {
        let unswung = self.attacks.saturating_sub(self.swings);
        *self = Self::default();
        (unswung > 0).then_some(Foul::NoSwing(unswung))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vanilla_tick_is_clean() {
        let mut a = Attacks::default();
        // Two clicks in one tick on the same entity, each swung for.
        for _ in 0..2 {
            assert_eq!(a.attack(7), None);
            a.swing();
        }
        assert_eq!(a.tick_end(), None);
        // The next tick may attack another entity, swinging first (an older client).
        a.swing();
        assert_eq!(a.attack(8), None);
        assert_eq!(a.tick_end(), None);
        // A swing on its own (a miss, a block) is fine.
        a.swing();
        assert_eq!(a.tick_end(), None);
    }

    #[test]
    fn an_attack_not_swung_for_is_caught() {
        let mut a = Attacks::default();
        a.attack(7);
        assert_eq!(a.tick_end(), Some(Foul::NoSwing(1)));
        // A swing in the tick before doesn't count for this one.
        a.swing();
        a.tick_end();
        a.attack(7);
        a.attack(7);
        a.swing();
        assert_eq!(a.tick_end(), Some(Foul::NoSwing(1)));
    }

    #[test]
    fn two_entities_in_one_tick_are_caught() {
        let mut a = Attacks::default();
        a.attack(7);
        a.swing();
        assert_eq!(a.attack(8), Some(Foul::TwoTargets(7, 8)));
        a.swing();
        // Still against the tick's first target.
        assert_eq!(a.attack(9), Some(Foul::TwoTargets(7, 9)));
        a.swing();
        assert_eq!(a.tick_end(), None);
    }
}
