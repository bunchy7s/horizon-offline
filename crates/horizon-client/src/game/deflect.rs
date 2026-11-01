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

/// `Projectile.onRedirectProjectile`: another projectile hit `p`, which turns
/// with `deflect(AIM_DEFLECT, hitter's owner, hitter's owner, true, 1.0)`.
/// It flies along the owner's look (`Entity.getLookAngle`; unchanged when the
/// owner is gone), is that owner's now, and its acceleration power is back
/// to 0.1 (`AbstractHurtingProjectile.onDeflection` by an attack).
pub(super) fn aim_deflect(
    p: &mut Projectile,
    look: Option<Vec3>,
    owner: Option<Who>,
    owner_uuid: Option<[i32; 4]>,
    guest: Option<u32>,
) {
    if let Some(look) = look {
        p.velocity = look;
        p.needs_sync = true;
    }
    (p.owner, p.owner_uuid, p.guest) = (owner, owner_uuid, guest);
    p.acceleration = 0.1;
}

/// `Entity.getPickRadius`: only `AbstractHurtingProjectile` has one (1); a shulker
/// bullet's is 0.
fn pick_radius(p: &Projectile) -> f64 {
    if matches!(p.kind, ProjectileKind::ShulkerBullet { .. }) {
        0.0
    } else {
        PICK_RADIUS
    }
}

/// A projectile a player's blow reaches: a redirectable one, or a shulker bullet
/// (`isPickable` and `isAttackable`), which `ShulkerBullet.hurtServer` pops.
pub(super) fn attackable(p: &Projectile) -> bool {
    redirectable(p) || (!p.stuck && matches!(p.kind, ProjectileKind::ShulkerBullet { .. }))
}

/// Its box for the crosshair: the entity's (a fireball 1 x 1, a wind charge or a
/// shulker bullet 0.3125, from its feet up) grown by the pick radius, which only
/// `AbstractHurtingProjectile` has (`getPickRadius`: 1; a shulker bullet's is 0).
fn pick_box(p: &Projectile) -> Aabb {
    let size = match p.kind {
        ProjectileKind::LargeFireball { .. } => 1.0,
        _ => 0.3125,
    };
    let radius = pick_radius(p);
    let half = size / 2.0 + radius;
    Aabb::new(
        Vec3::new(
            p.position.x - half,
            p.position.y - radius,
            p.position.z - half,
        ),
        Vec3::new(
            p.position.x + half,
            p.position.y + size + radius,
            p.position.z + half,
        ),
    )
}

impl Game<'_> {
    /// The redirectable projectile under the crosshair within the attack
    /// reach, and how far along the view it is.
    pub(super) fn redirectable_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        self.creatures
            .projectiles
            .iter()
            .enumerate()
            .filter(|(_, p)| attackable(p))
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
        if let (Some(hit), Some(eye)) = (self.pick_unfiltered(), self.eye_position()) {
            let d = (0..3)
                .map(|a| (hit.point[a] - eye[a]).powi(2))
                .sum::<f64>()
                .sqrt();
            if d < t {
                return false;
            }
        }
        if matches!(
            self.creatures.projectiles[i].kind,
            ProjectileKind::ShulkerBullet { .. }
        ) {
            self.hit_shulker_bullet(i);
        } else {
            self.deflect(i);
        }
        true
    }

    /// `ShulkerBullet.hurtServer` from a player's blow: the hurt sound, fifteen crit
    /// particles (`sendParticles`, spread 0.2) and `destroy`. The blow is
    /// `Player.attack` on a target that is not a mob: its sounds, then the swing.
    pub(super) fn hit_shulker_bullet(&mut self, i: usize) {
        let p = self.creatures.projectiles.remove(i);
        let at = [p.position.x, p.position.y, p.position.z];
        self.audio
            .play("minecraft:entity.shulker_bullet.hurt", Some(at), 1.0, 1.0);
        if let Some(ty) = horizon_client::particles::ParticleType::from_name("crit") {
            let options = horizon_client::particles::ParticleOptions::simple(ty);
            self.particles
                .send_particles(&options, at, [0.2, 0.2, 0.2], 0.0, 15);
            self.lan_level_particles(&options, at, [0.2, 0.2, 0.2], 0.0, 15, None);
        }
        let (_, _, _, magic) = self.melee_damage("minecraft:shulker_bullet", false);
        self.vehicle_attack_sounds(p.aabb(), magic.then_some(p.id as i32));
        self.reset_attack_strength();
        self.swing_arm();
    }

    /// `Player.canInteractWithEntity(box, 3.0)` as the server checks an
    /// attack packet: the projectile's box within the reach plus 3 of the eyes.
    pub(super) fn within_interaction_range(&self, i: usize) -> bool {
        let (Some(eye), Some(p)) = (self.eye_position(), self.creatures.projectiles.get(i)) else {
            return false;
        };
        let b = pick_box(p);
        let (lo, hi) = (b.min_position(), b.max_position());
        let r = pick_radius(&self.creatures.projectiles[i]);
        let (lo, hi) = (
            [lo.x + r, lo.y + r, lo.z + r],
            [hi.x - r, hi.y - r, hi.z - r],
        );
        let d2: f64 = (0..3)
            .map(|a| (eye[a] - eye[a].clamp(lo[a], hi[a])).powi(2))
            .sum();
        let range = self.entity_reach + 3.0;
        d2 < range * range
    }

    /// `Entity.getLookAngle` of a projectile's owner: the host, the guest who
    /// threw it, or a mob; none when the owner is not here.
    pub(super) fn owner_look(&self, owner: Option<Who>, guest: Option<u32>) -> Option<Vec3> {
        let (pitch, yaw) = match (owner?, guest) {
            (Who::Player, Some(conn)) => {
                let lan = self.lan.as_ref()?;
                let g = lan.hub.guests.iter().find(|g| g.conn == conn)?;
                (g.pitch, g.yaw)
            }
            (Who::Player, None) => {
                self.player.as_ref()?;
                (self.pitch, self.yaw)
            }
            (Who::Mob(id), _) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                (m.head_pitch, m.yaw)
            }
        };
        Some(horizon_world::view_direction::view_vector(pitch, yaw))
    }

    /// `Projectile.deflect(AIM_DEFLECT, player, player, true)`.
    pub(super) fn deflect(&mut self, i: usize) {
        let dir = view_vector(self.yaw, self.pitch);
        let (guest, owner_uuid) = (self.lan_acting, Some(self.acting_player_uuid()));
        let p = &mut self.creatures.projectiles[i];
        p.velocity = Vec3::new(dir[0], dir[1], dir[2]);
        p.yaw = dir[0].atan2(dir[2]).to_degrees() as f32;
        p.pitch = dir[1].atan2(dir[0].hypot(dir[2])).to_degrees() as f32;
        p.owner = Some(Who::Player);
        // On a server, the guest who struck it (their projectile now).
        (p.guest, p.owner_uuid) = (guest, owner_uuid);
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
        // A shulker bullet takes a blow, with no pick radius round its box.
        let bullet = Projectile::new(
            ProjectileKind::ShulkerBullet {
                target: Who::Player,
                dir: None,
                steps: 0,
                delta: [0.0; 3],
                seed: 0,
            },
            at,
            None,
        );
        assert!(attackable(&bullet) && !redirectable(&bullet));
        assert!(!attackable(&small));
        let b = pick_box(&bullet);
        assert_eq!(b.max_position().y, 64.3125);
        // The pick box: a block round the fireball's 1 x 1 box.
        let b = pick_box(&large);
        assert_eq!(b.min_position().x, -1.5);
        assert_eq!(b.max_position().y, 66.0);
    }

    fn wind_charge() -> Projectile {
        let mut p = Projectile::new(
            ProjectileKind::WindCharge,
            Vec3::new(0.0, 70.0, 0.0),
            Some(Who::Mob(7)),
        );
        p.velocity = Vec3::new(0.0, 1.5, 0.0);
        p
    }

    #[test]
    fn a_redirected_wind_charge_flies_along_the_new_owners_look() {
        let mut p = wind_charge();
        let look = Vec3::new(0.0, -1.0, 0.0);
        aim_deflect(&mut p, Some(look), Some(Who::Player), Some([4; 4]), Some(3));
        assert_eq!(p.velocity, look);
        assert_eq!((p.owner, p.owner_uuid), (Some(Who::Player), Some([4; 4])));
        assert_eq!(p.guest, Some(3));
        assert_eq!(p.acceleration, 0.1);
        assert!(p.needs_sync);
    }

    #[test]
    fn with_no_owner_here_it_keeps_its_movement_but_changes_hands() {
        let mut p = wind_charge();
        p.owner_uuid = Some([7; 4]);
        aim_deflect(&mut p, None, None, None, None);
        assert_eq!(p.velocity, Vec3::new(0.0, 1.5, 0.0));
        assert_eq!((p.owner, p.owner_uuid), (None, None));
        assert_eq!(p.acceleration, 0.1);
        assert!(!p.needs_sync);
    }
}
