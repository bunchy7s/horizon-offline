//! The wither on the game side: building it (`WitherSkullBlock.checkSpawn`),
//! its boss bar, and its skulls hitting and exploding.
use super::Game;
use horizon_client::block_pattern::{BlockPattern, PatternMatch, WITHER_FULL};
use horizon_client::{
    items::ItemStack,
    mobs::{Damage, DamageKind, Species, Who},
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::{geometry::Direction, position::BlockPos};

impl Game<'_> {
    /// `LivingEntity.createWitherRose`: a mob a wither killed leaves a wither
    /// rose where it stood (with mobGriefing, on air that can hold it), else
    /// drops one.
    pub(super) fn create_wither_rose(&mut self, m: &horizon_client::mobs::Mob) {
        // getKillCredit: the player who last hurt it, else the last mob (kept
        // for 100 ticks).
        let by_wither = m.ai.last_hurt_by_player <= 0
            && m.age as i32 - m.ai.last_hurt_by_time <= 100
            && matches!(m.ai.last_hurt_by, Some(Who::Mob(id))
                if self.mobs.iter().any(|o| o.id == id && o.alive() && o.species() == Species::Wither));
        if !by_wither {
            return;
        }
        let pos = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        let registry = self.pack.assets.registry;
        let rose = registry.default_state("minecraft:wither_rose");
        if self.ui.rules.bool("mob_griefing")
            && let Some(rose) = rose
            && self
                .res
                .facts()
                .is_air(self.world.block(pos[0], pos[1], pos[2]))
            && self.with_level(|b, level, ticks| {
                b.can_survive(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]), rose)
            })
        {
            let _ = self.level_set(pos, rose);
            return;
        }
        self.spawn_item(
            ItemStack::new("minecraft:wither_rose", 1),
            [m.position.x, m.position.y, m.position.z],
            super::dispenser::new_item_motion(),
            0,
        );
    }
    fn is_wither_skull(&self, p: [i32; 3]) -> bool {
        let name = &*self
            .res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name;
        matches!(
            name,
            "minecraft:wither_skeleton_skull" | "minecraft:wither_skeleton_wall_skull"
        )
    }
    /// `#minecraft:wither_summon_base_blocks` (soul sand and soul soil).
    pub(super) fn is_soul_base(&self, p: [i32; 3]) -> bool {
        let name = &*self
            .res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name;
        self.res
            .items()
            .block_tags
            .get("minecraft:wither_summon_base_blocks")
            .is_some_and(|t| t.contains(name))
    }
    /// `pattern.find(level, pos)` for the wither patterns: '^' a wither
    /// skeleton skull, '#' a summon base block, '~' air.
    pub(super) fn find_wither_pattern(
        &self,
        pattern: BlockPattern,
        pos: [i32; 3],
    ) -> Option<PatternMatch> {
        pattern.find(pos, |key, p| match key {
            '^' => self.is_wither_skull(p),
            '#' => self.is_soul_base(p),
            _ => self.res.facts().is_air(self.world.block(p[0], p[1], p[2])),
        })
    }
    /// `WitherSkullBlock.checkSpawn`, after a wither skeleton skull is
    /// placed: the T of three skulls on four soul sand or soul soil, built in
    /// any orientation, becomes a wither.
    pub(super) fn check_wither_spawn(&mut self, placed: [i32; 3]) -> horizon_client::Result<()> {
        // Nothing is built in Peaceful (`level.getDifficulty() != PEACEFUL`).
        if self.ui.difficulty == 0 || !self.is_wither_skull(placed) {
            return Ok(());
        }
        let Some(found) = self.find_wither_pattern(WITHER_FULL, placed) else {
            return Ok(());
        };
        // Clear the pattern, the wither appears on the middle of the bottom
        // row facing along the pattern's forwards axis, then neighbours update.
        let cells = found.front_cells();
        self.clear_pattern_blocks(&cells)?;
        let base = found.block(1, 2, 0);
        let at = [
            f64::from(base[0]) + 0.5,
            f64::from(base[1]) + 0.55,
            f64::from(base[2]) + 0.5,
        ];
        let yaw = if matches!(found.forwards, Direction::West | Direction::East) {
            0.0
        } else {
            90.0
        };
        let id = self.create_triggered("minecraft:wither", at, yaw);
        if let Some(m) = id.and_then(|id| self.mobs.iter_mut().find(|m| m.id == id)) {
            m.persistent = true;
            horizon_client::mobs::wither::make_invulnerable(m);
        }
        // SUMMONED_ENTITY for each player within 50 blocks of the wither's box.
        self.trigger_summoned(id, 50.0);
        self.update_pattern_blocks(&cells)
    }
    /// `WitherSkull.onHitEntity` / `onHit`: 8 `wither_skull` damage from a
    /// skull with an owner (healing a wither owner 5 when it kills), else 5
    /// `magic`, withering on a living target that took it (10 seconds on
    /// Normal, 40 on Hard, none on Easy), then a power 1 blast.
    pub(super) fn wither_skull_hit(
        &mut self,
        p: &Projectile,
        who: Option<Who>,
        at: [f64; 3],
    ) -> horizon_client::Result<()> {
        // Projectile.getOwner: none once the owner is gone.
        let live_owner = match p.owner {
            Some(Who::Mob(id)) if self.mobs.iter().any(|m| m.id == id && !m.removed) => {
                Some(Who::Mob(id))
            }
            Some(Who::Mob(_)) => None,
            other => other,
        };
        let owner = match live_owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        let from = [p.previous.x, p.previous.y, p.previous.z];
        let skull = skull_source(live_owner, from);
        let (amount, kind) = (skull.amount, skull.kind);
        let wither_ticks = match self.difficulty_id() {
            2 => 200,
            3 => 800,
            _ => 0,
        };
        match who {
            Some(Who::Player) => {
                if !self.creative {
                    // WitherSkull.onHitEntity's source: the wither launched
                    // it, and `magic` names no entity and no position.
                    self.combat.projectile_owner = owner;
                    // EnchantmentHelper.doPostAttackEffects runs only on a
                    // target that lived.
                    self.combat.post_attack_if_alive = true;
                    if skull.from.is_none() {
                        self.combat.projectile_direct = None;
                    }
                    let hit = self.hurt_player_from(
                        amount,
                        skull.from,
                        super::creatures::damage_type(kind, false),
                        Some(owner),
                    );
                    if hit && wither_ticks > 0 {
                        self.give_player_effect("minecraft:wither", wither_ticks, 1);
                    }
                    // A totem that fired saves the player inside hurtServer:
                    // still alive, so no heal.
                    let killed = self.player.as_ref().is_some_and(|pl| {
                        killed(hit, pl.health.dead_or_dying(), pl.health.totem_used)
                    });
                    if killed {
                        self.heal_skull_owner(p, live_owner);
                    }
                }
            }
            // EnderDragonPart.hurtServer: EnderDragon.hurt routes it by the
            // part, a skull is no arrow a perched dragon shrugs off, and only
            // a player's skull takes health. A part is no LivingEntity
            // (no withering) and stays alive (no heal).
            Some(Who::Mob(id)) if id >= super::dragon::SPECIAL => {
                let by_player = live_owner == Some(Who::Player);
                self.hit_special(id, amount, by_player, kind, Some(&p.kind));
            }
            Some(Who::Mob(id)) => {
                let source = Damage {
                    kind,
                    attacker: live_owner,
                    from: skull.from,
                };
                let owner_species = owner
                    .and_then(|o| self.mobs.iter().find(|m| m.id == o))
                    .map(|m| m.species());
                let killed = match self.mobs.iter_mut().find(|m| m.id == id) {
                    // `WitherBoss.hurtServer`: nothing from a wither friend.
                    Some(m)
                        if owner_species.is_some_and(|o| {
                            horizon_client::mobs::wither::shielded_from(m.species(), o)
                        }) =>
                    {
                        false
                    }
                    Some(m) => {
                        let hit = m.damage(amount, source);
                        if hit && m.alive() && m.species() != Species::Wither && wither_ticks > 0 {
                            m.add_effect("minecraft:wither", wither_ticks, 1);
                        }
                        hit && !m.alive()
                    }
                    None => false,
                };
                if killed {
                    self.heal_skull_owner(p, live_owner);
                }
            }
            None => {}
        }
        self.combat.explosion_owner = owner;
        self.combat.explosion_in_water = p.touching_water;
        // The blast's source names a player owner, for the dragon's kill credit.
        self.set_blast_by_player(live_owner == Some(Who::Player));
        let result = self.with_hive_emptying_blast(|g| g.explode(at, 1.0));
        self.set_blast_by_player(false);
        self.combat.explosion_owner = None;
        self.combat.explosion_in_water = false;
        result
    }
    /// `livingOwner.heal(5.0F)` after the skull killed: `LivingEntity.heal`
    /// does nothing to an owner at no health.
    fn heal_skull_owner(&mut self, p: &Projectile, owner: Option<Who>) {
        match owner {
            Some(Who::Mob(id)) => {
                if let Some(w) = self.mobs.iter_mut().find(|m| m.id == id && m.alive()) {
                    w.health = (w.health + 5.0).min(w.attrs.max_health);
                }
            }
            // The player (or the guest acting now) who owns the skull.
            Some(Who::Player) if p.guest.is_none() || p.guest == self.lan_acting => {
                self.apply_effect_action(horizon_client::effects::EffectAction::Heal(5.0));
            }
            _ => {}
        }
    }
}
/// `!target.isAlive()` after a hit that landed: a totem that fired keeps the
/// player alive.
pub(super) fn killed(hurt: bool, dead_or_dying: bool, totem_used: bool) -> bool {
    hurt && dead_or_dying && !totem_used
}
/// `WitherSkull.onHitEntity`'s damage source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SkullSource {
    pub(super) amount: f32,
    pub(super) kind: DamageKind,
    /// The source's position: the skull's for `witherSkull(skull, owner)`,
    /// none for `magic()`, which has no entity.
    pub(super) from: Option<[f64; 3]>,
}
/// 8 `wither_skull` from a skull with a living owner, else 5 `magic`.
/// `skull` is where the skull is.
pub(super) fn skull_source(owner: Option<Who>, skull: [f64; 3]) -> SkullSource {
    match owner {
        Some(_) => SkullSource {
            amount: 8.0,
            kind: DamageKind::WitherSkull,
            from: Some(skull),
        },
        None => SkullSource {
            amount: 5.0,
            kind: DamageKind::Magic,
            from: None,
        },
    }
}
/// Wither skulls drawn as the skull model, blue when dangerous.
pub(super) fn skull_kind(p: &Projectile) -> Option<bool> {
    match p.kind {
        ProjectileKind::WitherSkull { dangerous } => Some(dangerous),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unowned_skull_hurts_as_magic_from_nowhere() {
        // DamageSources.magic(): no entity and no position, so nothing to
        // blame or push away from.
        let at = [1.0, 64.0, 2.0];
        let unowned = skull_source(None, at);
        assert_eq!((unowned.amount, unowned.kind), (5.0, DamageKind::Magic));
        assert_eq!(unowned.from, None);
        // witherSkull(skull, owner): from where the skull is.
        let owned = skull_source(Some(Who::Mob(3)), at);
        assert_eq!((owned.amount, owned.kind), (8.0, DamageKind::WitherSkull));
        assert_eq!(owned.from, Some(at));
        assert_eq!(skull_source(Some(Who::Player), at), owned);
    }

    #[test]
    fn a_totem_that_fires_is_no_kill() {
        assert!(killed(true, true, false));
        // The totem restores health next tick: alive inside hurtServer.
        assert!(!killed(true, true, true));
        assert!(!killed(true, false, false));
        assert!(!killed(false, true, false));
    }
}
