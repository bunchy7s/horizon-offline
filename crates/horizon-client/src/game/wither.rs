//! The wither on the game side: building it (`WitherSkullBlock.checkSpawn`),
//! its boss bar, and its skulls hitting and exploding.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{Damage, DamageKind, Species, Who},
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::position::BlockPos;

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
    fn is_soul_base(&self, p: [i32; 3]) -> bool {
        let name = &*self
            .res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name;
        matches!(name, "minecraft:soul_sand" | "minecraft:soul_soil")
    }
    /// After a wither skeleton skull is placed: the T of soul sand (or soul
    /// soil) under three skulls, upright along X or Z, becomes a wither.
    pub(super) fn check_wither_spawn(&mut self, placed: [i32; 3]) -> horizon_client::Result<()> {
        // Nothing is built in Peaceful (`level.getDifficulty() != PEACEFUL`).
        if self.ui.difficulty == 0 || !self.is_wither_skull(placed) {
            return Ok(());
        }
        // Pattern rows top to bottom: "^^^", "###", "~#~" ('~' air).
        for (ax, az) in [(1, 0), (0, 1)] {
            for col in 0..3 {
                let left = [placed[0] - ax * col, placed[1], placed[2] - az * col];
                let at = |c: i32, row: i32| [left[0] + ax * c, left[1] - row, left[2] + az * c];
                let ok = (0..3).all(|c| self.is_wither_skull(at(c, 0)))
                    && (0..3).all(|c| self.is_soul_base(at(c, 1)))
                    && self.is_soul_base(at(1, 2))
                    && [0, 2].iter().all(|&c| {
                        let p = at(c, 2);
                        self.res.facts().is_air(self.world.block(p[0], p[1], p[2]))
                    });
                if !ok {
                    continue;
                }
                // WitherSkullBlock.checkSpawn: clear the pattern, the wither
                // appears on the middle of the bottom row, then neighbours update.
                let cells: Vec<[i32; 3]> = (0..3)
                    .flat_map(|c| (0..3).map(move |row| (c, row)))
                    .map(|(c, row)| at(c, row))
                    .collect();
                self.clear_pattern_blocks(&cells)?;
                let base = at(1, 2);
                let at = [
                    f64::from(base[0]) + 0.5,
                    f64::from(base[1]) + 0.55,
                    f64::from(base[2]) + 0.5,
                ];
                let yaw = if ax == 1 { 0.0 } else { 90.0 };
                if let Some(id) = self.create_triggered("minecraft:wither", at, yaw)
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    m.persistent = true;
                    horizon_client::mobs::wither::make_invulnerable(m);
                    let view =
                        self.mob_entity(&self.mobs.iter().find(|m| m.id == id).cloned().unwrap());
                    self.trigger_entity("minecraft:summoned_entity", "entity", &view);
                }
                self.update_pattern_blocks(&cells)?;
                return Ok(());
            }
        }
        Ok(())
    }
    /// `WitherSkull.onHitEntity` / `onHit`: 8 damage (healing the wither 5
    /// when it kills), withering on a hit (10 seconds on Normal, 40 on Hard,
    /// none on Easy), then a power 1 blast.
    pub(super) fn wither_skull_hit(
        &mut self,
        p: &Projectile,
        who: Option<Who>,
        at: [f64; 3],
    ) -> horizon_client::Result<()> {
        let owner = match p.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        let amount = if owner.is_some() { 8.0 } else { 5.0 };
        let wither_ticks = match self.difficulty_id() {
            2 => 200,
            3 => 800,
            _ => 0,
        };
        match who {
            Some(Who::Player) => {
                if !self.creative {
                    // WitherSkull.onHitEntity's source: the wither launched it.
                    self.combat.projectile_owner = owner;
                    let hit = self.mob_hits_player(
                        amount,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:wither_skull",
                    );
                    if hit && wither_ticks > 0 {
                        self.give_player_effect("minecraft:wither", wither_ticks, 1);
                    }
                }
            }
            Some(Who::Mob(id)) => {
                let source = Damage {
                    kind: DamageKind::Magic,
                    attacker: p.owner,
                    from: Some([p.previous.x, p.previous.y, p.previous.z]),
                };
                let killed = match self.mobs.iter_mut().find(|m| m.id == id) {
                    Some(m) => {
                        let hit = m.damage(amount, source);
                        if hit && m.alive() && m.species() != Species::Wither && wither_ticks > 0 {
                            m.add_effect("minecraft:wither", wither_ticks, 1);
                        }
                        hit && !m.alive()
                    }
                    None => false,
                };
                if killed
                    && let Some(o) = owner
                    && let Some(w) = self.mobs.iter_mut().find(|m| m.id == o)
                {
                    w.health = (w.health + 5.0).min(w.attrs.max_health);
                }
            }
            None => {}
        }
        if let Some(id) = owner {
            self.note_attacker("minecraft:explosion", id);
        }
        self.combat.explosion_owner = owner;
        let result = self.with_hive_emptying_blast(|g| g.explode(at, 1.0));
        self.combat.explosion_owner = None;
        result
    }
}
/// Wither skulls drawn as the skull model, blue when dangerous.
pub(super) fn skull_kind(p: &Projectile) -> Option<bool> {
    match p.kind {
        ProjectileKind::WitherSkull { dangerous } => Some(dangerous),
        _ => None,
    }
}
