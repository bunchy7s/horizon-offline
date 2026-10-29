//! A player's blow at a ghast's fireball or a wind charge
//! (`#minecraft:redirectable_projectile`): `Player.attack` turns it with
//! `ProjectileDeflection.AIM_DEFLECT` instead of hurting anything. It flies on
//! along the player's view and is the player's now (`Projectile.deflect`
//! sets the owner), so a ghast it meets dies of it (`Ghast.isReflectedFireball`).
use super::{Game, ray_box, view_vector};
use horizon_client::{
    mobs::Who,
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::geometry::{Aabb, Vec3};

/// `AbstractHurtingProjectile.getPickRadius`: the crosshair finds it a block
/// round its box.
const PICK_RADIUS: f64 = 1.0;

/// `#minecraft:redirectable_projectile`: a ghast's fireball and both wind charges.
pub(super) fn redirectable(p: &Projectile) -> bool {
    !p.stuck
        && matches!(
            p.kind,
            ProjectileKind::LargeFireball { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        )
}

/// Its box for the crosshair: the entity's (a fireball 1 x 1, a wind charge
/// 0.3125, from its feet up) grown by the pick radius.
fn pick_box(p: &Projectile) -> Aabb {
    let size = match p.kind {
        ProjectileKind::LargeFireball { .. } => 1.0,
        _ => 0.3125,
    };
    let half = size / 2.0 + PICK_RADIUS;
    Aabb::new(
        Vec3::new(
            p.position.x - half,
            p.position.y - PICK_RADIUS,
            p.position.z - half,
        ),
        Vec3::new(
            p.position.x + half,
            p.position.y + size + PICK_RADIUS,
            p.position.z + half,
        ),
    )
}

impl Game<'_> {
    /// The redirectable projectile under the crosshair within the attack
    /// reach, and how far along the view it is.
    pub(super) fn redirectable_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = if self.creative { 5.0 } else { 3.0 };
        self.creatures
            .projectiles
            .iter()
            .enumerate()
            .filter(|(_, p)| redirectable(p))
            .filter_map(|(i, p)| ray_box(eye, dir, reach, pick_box(p)).map(|t| (i, t)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// `Player.attack` on a redirectable projectile nearer than any mob or
    /// block in view: `AIM_DEFLECT` (its movement the unit view vector), the
    /// player its owner, its acceleration power back to 0.1
    /// (`onDeflection` by an attack), and the no-damage sound. True when a
    /// projectile was turned.
    pub(super) fn attack_redirectable(&mut self) -> bool {
        let Some((i, t)) = self.redirectable_hit() else {
            return false;
        };
        if self.mob_hit().is_some_and(|(_, d)| d < t) {
            return false;
        }
        if let (Some(hit), Some(eye)) = (self.pick(), self.eye_position()) {
            let d = (0..3)
                .map(|a| (hit.point[a] - eye[a]).powi(2))
                .sum::<f64>()
                .sqrt();
            if d < t {
                return false;
            }
        }
        self.deflect(i);
        true
    }

    /// `Player.canInteractWithEntity(box, 3.0)` as the server checks an
    /// attack packet: the projectile's box within the reach plus 3 of the eyes.
    pub(super) fn within_interaction_range(&self, i: usize) -> bool {
        let (Some(eye), Some(p)) = (self.eye_position(), self.creatures.projectiles.get(i)) else {
            return false;
        };
        let b = pick_box(p);
        let (lo, hi) = (b.min_position(), b.max_position());
        let (lo, hi) = (
            [lo.x + PICK_RADIUS, lo.y + PICK_RADIUS, lo.z + PICK_RADIUS],
            [hi.x - PICK_RADIUS, hi.y - PICK_RADIUS, hi.z - PICK_RADIUS],
        );
        let d2: f64 = (0..3)
            .map(|a| (eye[a] - eye[a].clamp(lo[a], hi[a])).powi(2))
            .sum();
        let range = if self.creative { 5.0 } else { 3.0 } + 3.0;
        d2 < range * range
    }

    /// `Projectile.deflect(AIM_DEFLECT, player, player, true)`.
    pub(super) fn deflect(&mut self, i: usize) {
        let dir = view_vector(self.yaw, self.pitch);
        let guest = self.lan_acting;
        let p = &mut self.creatures.projectiles[i];
        p.velocity = Vec3::new(dir[0], dir[1], dir[2]);
        p.yaw = dir[0].atan2(dir[2]).to_degrees() as f32;
        p.pitch = dir[1].atan2(dir[0].hypot(dir[2])).to_degrees() as f32;
        p.owner = Some(Who::Player);
        // On a server, the guest who struck it (their projectile now).
        p.guest = guest;
        // Not into the one who turned it on its way out.
        p.left_owner = false;
        // `AbstractHurtingProjectile.onDeflection` by an attack, and the
        // `needsSync` that `AIM_DEFLECT` sets: a server's players see it turn
        // at once, not at the fireball's next update ten ticks later.
        p.acceleration = 0.1;
        p.needs_sync = true;
        self.reset_attack_strength();
        self.swing_arm();
        let from = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        self.audio
            .play("minecraft:entity.player.attack.nodamage", from, 1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ghast_fireballs_and_wind_charges_are_redirectable() {
        let at = Vec3::new(0.0, 64.0, 0.0);
        let large = Projectile::fireball(
            ProjectileKind::LargeFireball { power: 1 },
            at,
            None,
            [1.0, 0.0, 0.0],
        );
        let small = Projectile::fireball(ProjectileKind::SmallFireball, at, None, [1.0, 0.0, 0.0]);
        assert!(redirectable(&large));
        assert!(!redirectable(&small));
        // The pick box: a block round the fireball's 1 x 1 box.
        let b = pick_box(&large);
        assert_eq!(b.min_position().x, -1.5);
        assert_eq!(b.max_position().y, 66.0);
    }
}
